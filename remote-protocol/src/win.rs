//! Windows tokens and security descriptors for the terminal server, shared
//! by the server (`brindle-remote`) and its clients: who this process and
//! the process at the other end of a pipe run as, and the protections pipes
//! and the server directory are created with. The rules themselves are in
//! [`crate::access`], re-exported here.

use std::ffi::{OsString, c_void};
use std::fs::File;
use std::io;
use std::os::windows::ffi::OsStringExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use windows::Win32::Foundation::{ERROR_SUCCESS, HANDLE, HLOCAL, LocalFree, WIN32_ERROR};
use windows::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
    GetNamedSecurityInfoW, GetSecurityInfo, SDDL_REVISION_1, SE_FILE_OBJECT, SE_KERNEL_OBJECT,
    SetNamedSecurityInfoW,
};
use windows::Win32::Security::{
    LABEL_SECURITY_INFORMATION,
    OWNER_SECURITY_INFORMATION, SYSTEM_MANDATORY_LABEL_ACE, SetKernelObjectSecurity,
};
use windows::Win32::Security::{
    ACCESS_ALLOWED_ACE, ACL, DACL_SECURITY_INFORMATION, GetAce, GetSecurityDescriptorControl,
    GetSecurityDescriptorDacl, GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation,
    PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SE_DACL_PROTECTED,
    SECURITY_ATTRIBUTES, TOKEN_INFORMATION_CLASS, TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TOKEN_USER,
    TokenIntegrityLevel, TokenUser,
};
use windows::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CreateDirectoryW, CreateFileW, FILE_ATTRIBUTE_DIRECTORY,
    FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT,
    FILE_LIST_DIRECTORY, FILE_READ_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE,
    FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ, FILE_GENERIC_WRITE, FILE_SHARE_DELETE,
    FILE_WRITE_DATA, GETFINALPATHNAMEBYHANDLE_FLAGS, GetFileInformationByHandle, GetFinalPathNameByHandleW,
    GetLongPathNameW, MOVE_FILE_FLAGS, MoveFileExW, OPEN_ALWAYS, OPEN_EXISTING, READ_CONTROL, WRITE_DAC,
    WRITE_OWNER,
};
use std::os::windows::fs::MetadataExt;
use windows::Win32::System::Pipes::GetNamedPipeServerProcessId;
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows::core::{BOOL, HSTRING, PWSTR};

pub use crate::access::{
    Ace, Contents, Dacl, Integrity, Principal, Refusal, SYSTEM_SID, Side, dir_sddl, peer_allowed,
    pipe_sddl,
};
use crate::access::{self, DirSecurity, Found, Label, Step};
use crate::server_dir::{self, EntryKind, ImageRemoval, LostRename, PruneDir};

/// `ACCESS_ALLOWED_ACE_TYPE`: the only entry type [`Dacl`] reads a SID from.
const ACCESS_ALLOWED: u8 = 0;

fn context(what: impl std::fmt::Display, error: io::Error) -> io::Error {
    io::Error::new(error.kind(), format!("{what}: {error}"))
}

/// The `windows` crate's error as an OS error, so its `kind()` is the usual
/// one: the crate's own conversion keeps the HRESULT, which std doesn't know.
fn os(error: windows::core::Error) -> io::Error {
    match crate::hresult::win32_code(error.code().0 as u32) {
        Some(code) => io::Error::from_raw_os_error(code as i32),
        None => io::Error::other(error),
    }
}

fn win32(code: WIN32_ERROR) -> io::Result<()> {
    if code == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(io::Error::from_raw_os_error(code.0 as i32))
    }
}

/// Memory the system allocated with `LocalAlloc`, freed on drop.
struct Local(*mut c_void);

impl Drop for Local {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: the pointer came from a function that documents
            // `LocalFree` as its release, and is freed once.
            unsafe { LocalFree(Some(HLOCAL(self.0))) };
        }
    }
}

/// The string form of a SID (`S-1-5-…`).
///
/// # Safety
/// `sid` must point to a valid SID.
unsafe fn sid_string(sid: PSID) -> io::Result<String> {
    let mut text = PWSTR::null();
    // SAFETY: the caller's SID; `text` receives a LocalAlloc'd string.
    unsafe { ConvertSidToStringSidW(sid, &mut text) }.map_err(os)?;
    let _free = Local(text.0.cast());
    // SAFETY: a NUL-terminated string from the call above.
    unsafe { text.to_string() }.map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

/// One class of token information, in a buffer aligned for its struct.
fn token_information(token: HANDLE, class: TOKEN_INFORMATION_CLASS) -> io::Result<Vec<u64>> {
    let mut size = 0u32;
    // SAFETY: a size query; it fails with the size needed.
    let _ = unsafe { GetTokenInformation(token, class, None, 0, &mut size) };
    if size == 0 {
        return Err(io::Error::last_os_error());
    }
    let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
    // SAFETY: the buffer holds at least `size` bytes.
    unsafe {
        GetTokenInformation(
            token,
            class,
            Some(buffer.as_mut_ptr().cast()),
            size,
            &mut size,
        )
    }
    .map_err(os)?;
    Ok(buffer)
}

/// The user and integrity level of the token of `process`, a handle with
/// `PROCESS_QUERY_LIMITED_INFORMATION`.
fn principal_of_process(process: HANDLE) -> io::Result<Principal> {
    let mut token = HANDLE::default();
    // SAFETY: a live process handle; the token handle is owned below.
    unsafe { OpenProcessToken(process, TOKEN_QUERY, &mut token) }
        .map_err(|error| context("can't open its token", os(error)))?;
    // SAFETY: a fresh handle nothing else owns.
    let token = unsafe { OwnedHandle::from_raw_handle(token.0) };
    let handle = HANDLE(token.as_raw_handle());

    let user = token_information(handle, TokenUser)
        .map_err(|error| context("can't read its user", error))?;
    // SAFETY: `TokenUser` fills a `TOKEN_USER`, whose SID points into the
    // same buffer, which is aligned and outlives the call.
    let sid = unsafe { sid_string((*user.as_ptr().cast::<TOKEN_USER>()).User.Sid) }?;

    let label = token_information(handle, TokenIntegrityLevel)
        .map_err(|error| context("can't read its integrity level", error))?;
    // SAFETY: `TokenIntegrityLevel` fills a `TOKEN_MANDATORY_LABEL` whose
    // SID is in the buffer; the level is that SID's last sub-authority, and
    // a label SID has at least one.
    let rid = unsafe {
        let sid = (*label.as_ptr().cast::<TOKEN_MANDATORY_LABEL>()).Label.Sid;
        let count = *GetSidSubAuthorityCount(sid);
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "its integrity label is empty",
            ));
        }
        *GetSidSubAuthority(sid, u32::from(count) - 1)
    };
    Ok(Principal {
        sid,
        integrity: Integrity(rid),
    })
}

/// Who this process runs as.
pub fn own_principal() -> io::Result<Principal> {
    // SAFETY: the pseudo-handle for this process, which needs no closing.
    principal_of_process(unsafe { GetCurrentProcess() })
        .map_err(|error| context("this process", error))
}

/// This process's token user, as a SID string.
pub fn own_sid() -> io::Result<String> {
    Ok(own_principal()?.sid)
}

/// This process's integrity level.
pub fn own_integrity() -> io::Result<Integrity> {
    Ok(own_principal()?.integrity)
}

/// Whether this process is elevated (high integrity or above).
pub fn is_elevated() -> io::Result<bool> {
    Ok(own_integrity()?.is_elevated())
}

/// Who process `pid` runs as. An error when the process or its token can't
/// be opened or read, which callers treat as a refusal.
pub fn principal_of(pid: u32) -> io::Result<Principal> {
    let what = || format!("process {pid}");
    // SAFETY: plain arguments; the handle is owned below.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
        .map_err(|error| context(what(), context("can't open it", os(error))))?;
    // SAFETY: a fresh handle nothing else owns.
    let process = unsafe { OwnedHandle::from_raw_handle(process.0) };
    principal_of_process(HANDLE(process.as_raw_handle())).map_err(|error| context(what(), error))
}

/// The pid of the process that created the pipe `pipe` is a client end of.
pub fn pipe_server_pid(pipe: &impl AsRawHandle) -> io::Result<u32> {
    let mut pid = 0u32;
    // SAFETY: the caller's open pipe handle.
    unsafe { GetNamedPipeServerProcessId(HANDLE(pipe.as_raw_handle()), &mut pid) }.map_err(os)?;
    Ok(pid)
}

/// The client's check of a pipe it has opened, before it sends anything:
/// the server must run as this user and at this process's integrity level.
/// A server whose token can't be read is refused too.
pub fn check_server(pipe: &impl AsRawHandle) -> io::Result<()> {
    let refuse = |error: io::Error| {
        io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("refusing the server on this pipe: {error}"),
        )
    };
    let pid =
        pipe_server_pid(pipe).map_err(|error| refuse(context("can't find its process", error)))?;
    let server = principal_of(pid).map_err(refuse)?;
    let own = own_principal()?;
    peer_allowed(&own, &server, Side::Client).map_err(|refusal| refuse(refusal.into()))
}

/// Owned `SECURITY_ATTRIBUTES` (handles not inherited) with its security
/// descriptor, for creating a pipe or a directory.
pub struct Security {
    attributes: SECURITY_ATTRIBUTES,
    _descriptor: Local,
}

impl Security {
    /// Builds the descriptor from an SDDL string.
    pub fn from_sddl(sddl: &str) -> io::Result<Self> {
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        // SAFETY: `descriptor` receives a LocalAlloc'd self-relative
        // descriptor, owned by the result.
        unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                &HSTRING::from(sddl),
                SDDL_REVISION_1,
                &mut descriptor,
                None,
            )
        }
        .map_err(|error| {
            context(
                format_args!("can't build the security descriptor {sddl}"),
                os(error),
            )
        })?;
        Ok(Self {
            attributes: SECURITY_ATTRIBUTES {
                nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
                lpSecurityDescriptor: descriptor.0,
                bInheritHandle: false.into(),
            },
            _descriptor: Local(descriptor.0),
        })
    }

    /// For a server pipe: this user and SYSTEM, labelled no-write-up at this
    /// process's level (high when elevated, else medium).
    pub fn for_pipe() -> io::Result<Self> {
        let own = own_principal()?;
        Self::from_sddl(&pipe_sddl(&own.sid, own.is_elevated()))
    }

    /// For the server directory: this user and SYSTEM, inherited by its
    /// files.
    pub fn for_dir() -> io::Result<Self> {
        Self::from_sddl(&dir_sddl(&own_sid()?))
    }

    /// What `CreateNamedPipeW`, `CreateDirectoryW` and the like take. Valid
    /// while `self` lives.
    pub fn attributes(&self) -> *const SECURITY_ATTRIBUTES {
        &self.attributes
    }

    fn descriptor(&self) -> PSECURITY_DESCRIPTOR {
        PSECURITY_DESCRIPTOR(self.attributes.lpSecurityDescriptor)
    }
}

/// Creates the one directory `path` for this user and SYSTEM only; its
/// parent must exist. `AlreadyExists` when something is there.
pub fn create_private_dir(path: &Path) -> io::Result<()> {
    let security = Security::for_dir()?;
    // SAFETY: the attributes live until the call returns.
    unsafe {
        CreateDirectoryW(
            &HSTRING::from(path.as_os_str()),
            Some(security.attributes()),
        )
    }
    .map_err(os)
}

/// Replaces the DACL of the existing directory `path` with the one
/// [`create_private_dir`] gives, dropping inherited entries; its files
/// inherit the new one.
pub fn protect_dir(path: &Path) -> io::Result<()> {
    let security = Security::for_dir()?;
    let (mut present, mut defaulted) = (BOOL::default(), BOOL::default());
    let mut dacl: *mut ACL = std::ptr::null_mut();
    // SAFETY: a valid descriptor; `dacl` points into it, and it outlives
    // the calls below.
    unsafe {
        GetSecurityDescriptorDacl(
            security.descriptor(),
            &mut present,
            &mut dacl,
            &mut defaulted,
        )
    }
    .map_err(os)?;
    // SAFETY: the path is NUL-terminated by `HSTRING`, the DACL is valid.
    win32(unsafe {
        SetNamedSecurityInfoW(
            &HSTRING::from(path.as_os_str()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(dacl.cast_const()),
            None,
        )
    })
}

/// Reading a [`Dacl`] back from an object; what it must hold is
/// [`Dacl::allows_only`]'s, in [`crate::access`].
impl Dacl {
    /// Reads the descriptor `GetSecurityInfo` or its named form returned,
    /// and frees it.
    fn read(descriptor: PSECURITY_DESCRIPTOR, dacl: *const ACL) -> io::Result<Self> {
        let _free = Local(descriptor.0);
        Self::parse(descriptor, dacl)
    }

    /// Reads the DACL `dacl` of `descriptor`, which the caller frees.
    fn parse(descriptor: PSECURITY_DESCRIPTOR, dacl: *const ACL) -> io::Result<Self> {
        let (mut control, mut revision) = (0u16, 0u32);
        // SAFETY: the descriptor the system just returned.
        unsafe { GetSecurityDescriptorControl(descriptor, &mut control, &mut revision) }
            .map_err(os)?;
        let mut this = Self {
            protected: control & SE_DACL_PROTECTED.0 != 0,
            allowed: Vec::new(),
            others: 0,
        };
        if dacl.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "the object has no DACL: everyone is allowed",
            ));
        }
        // SAFETY: `dacl` points into the descriptor, which lives until this
        // function returns; each entry `GetAce` returns starts with a header,
        // and an access-allowed entry holds its SID at `SidStart`.
        unsafe {
            for index in 0..u32::from((*dacl).AceCount) {
                let mut entry: *mut c_void = std::ptr::null_mut();
                GetAce(dacl, index, &mut entry).map_err(os)?;
                let ace = entry.cast::<ACCESS_ALLOWED_ACE>();
                if (*ace).Header.AceType != ACCESS_ALLOWED {
                    this.others += 1;
                    continue;
                }
                this.allowed.push(Ace {
                    sid: sid_string(PSID((&raw mut (*ace).SidStart).cast()))?,
                    mask: (*ace).Mask,
                    flags: (*ace).Header.AceFlags,
                });
            }
        }
        Ok(this)
    }

    /// The DACL of a kernel object, such as a pipe, through a handle with
    /// `READ_CONTROL`.
    pub fn of_handle(handle: RawHandle) -> io::Result<Self> {
        let mut dacl: *mut ACL = std::ptr::null_mut();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        // SAFETY: the caller's open handle; the descriptor is freed by `read`.
        win32(unsafe {
            GetSecurityInfo(
                HANDLE(handle),
                SE_KERNEL_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(&mut dacl),
                None,
                Some(&mut descriptor),
            )
        })?;
        Self::read(descriptor, dacl)
    }

    /// The DACL of a file or directory.
    pub fn of_path(path: &Path) -> io::Result<Self> {
        let mut dacl: *mut ACL = std::ptr::null_mut();
        let mut descriptor = PSECURITY_DESCRIPTOR::default();
        // SAFETY: the path is NUL-terminated by `HSTRING`; the descriptor is
        // freed by `read`.
        win32(unsafe {
            GetNamedSecurityInfoW(
                &HSTRING::from(path.as_os_str()),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                None,
                None,
                Some(&mut dacl),
                None,
                &mut descriptor,
            )
        })?;
        Self::read(descriptor, dacl)
    }
}

/// What tells one file from another whatever its name: the volume's serial
/// number and the file's index on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FileIdentity {
    volume: u32,
    index: u64,
}

/// The identity of an open file (`GetFileInformationByHandle`). A file
/// created under a deleted file's name has another one.
pub fn file_identity(file: &File) -> io::Result<FileIdentity> {
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: the handle is open for the call and `info` is writable.
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle()), &mut info) }.map_err(os)?;
    Ok(FileIdentity {
        volume: info.dwVolumeSerialNumber,
        index: u64::from(info.nFileIndexHigh) << 32 | u64::from(info.nFileIndexLow),
    })
}

/// This process's image path (`GetModuleFileNameW`, through std). A server
/// role runs from its per-build copy, so this is what it starts roles from.
pub fn own_image_path() -> io::Result<PathBuf> {
    std::env::current_exe()
}

/// The image path of the running process `pid`
/// (`QueryFullProcessImageNameW`), where Linux reads `/proc/<pid>/exe`:
/// the upgrade check reads the running core's binary through it.
pub fn image_path_of(pid: u32) -> io::Result<PathBuf> {
    let what = || format!("process {pid}");
    // SAFETY: plain arguments; the handle is owned below.
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
        .map_err(|error| context(what(), context("can't open it", os(error))))?;
    // SAFETY: a fresh handle nothing else owns.
    let process = unsafe { OwnedHandle::from_raw_handle(process.0) };
    // Long paths can exceed MAX_PATH; 32767 units is the most a path has.
    let mut buffer = vec![0u16; 32 * 1024];
    let mut len = buffer.len() as u32;
    // SAFETY: the buffer has `len` units, and `len` is set to what is used.
    unsafe {
        QueryFullProcessImageNameW(
            HANDLE(process.as_raw_handle()),
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut len,
        )
    }
    .map_err(|error| context(what(), os(error)))?;
    Ok(PathBuf::from(OsString::from_wide(&buffer[..len as usize])))
}

/// The build id of the file at `path`, or `None` when it can't be read.
fn build_of(path: &Path) -> Option<String> {
    std::fs::read(path)
        .ok()
        .map(|bytes| crate::build_id::of_binary(&bytes))
}

/// Makes sure the per-build copy of the helper at `installed` exists under
/// `copies` (see [`crate::server_dir::helper_copies`]) and returns its path.
/// Server roles run from the copy, never from the installed helper, whose
/// image a role running for days would keep locked.
///
/// The helper is copied to a temporary name beside the copy, keeping its
/// last-write time, and renamed into place. When another process's rename
/// got there first, its file is used once its build id checks out.
pub fn ensure_helper_copy(installed: &Path, copies: &Path) -> io::Result<PathBuf> {
    static NEXT: AtomicU64 = AtomicU64::new(0);

    let build = std::fs::read(installed)
        .map(|bytes| crate::build_id::of_binary(&bytes))
        .map_err(|error| context(installed.display(), error))?;
    let copy = server_dir::helper_copy(copies, &build);
    if server_dir::is_current(build_of(&copy).as_deref(), &build) {
        return Ok(copy);
    }
    // `helper_copy` puts the copy in its build's directory, so this can't miss.
    let dir = copy.parent().ok_or_else(|| {
        io::Error::other(format!("{} has no directory to be made in", copy.display()))
    })?;
    std::fs::create_dir_all(dir).map_err(|error| context(dir.display(), error))?;
    let temp = dir.join(server_dir::copy_temp_name(
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
    ));
    let placed = place_copy(installed, &temp, &copy, &build);
    if placed.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    placed.map(|()| copy)
}

/// Copies `installed` to `temp` and renames that to `copy`. On an error
/// `temp` may be left behind.
fn place_copy(installed: &Path, temp: &Path, copy: &Path, build: &str) -> io::Result<()> {
    // `CopyFileExW`, which keeps the last-write time as `CopyFileW` does.
    std::fs::copy(installed, temp).map_err(|error| context(temp.display(), error))?;
    if !server_dir::is_current(build_of(temp).as_deref(), build) {
        return Err(io::Error::other(format!(
            "{} changed while it was copied",
            installed.display()
        )));
    }
    // Without `MOVEFILE_REPLACE_EXISTING`, so a copy that is already there
    // (and may be running) is never replaced by an equal one.
    // SAFETY: both names are NUL-terminated strings that outlive the call.
    let renamed = unsafe {
        MoveFileExW(
            &HSTRING::from(temp),
            &HSTRING::from(copy),
            MOVE_FILE_FLAGS(0),
        )
    };
    let Err(error) = renamed else {
        return Ok(());
    };
    match server_dir::after_lost_rename(build_of(copy).as_deref(), build, copy.exists()) {
        LostRename::UseWinner => std::fs::remove_file(temp),
        LostRename::Replace => {
            std::fs::rename(temp, copy).map_err(|error| context(copy.display(), error))
        }
        LostRename::Fail => Err(context(copy.display(), os(error))),
    }
}

/// What [`prune_helper_copies`] did.
#[derive(Debug, Default)]
pub struct Pruned {
    /// How many builds' directories it removed.
    pub removed: usize,
    /// What it kept, and why: a build's directory whose image is in use or
    /// that couldn't be removed, or the copies' directory itself when it
    /// couldn't be read.
    pub kept: Vec<(PathBuf, String)>,
}

/// Removes every build's directory under `copies` except `keep`'s, where it
/// can: Windows refuses to delete a running image, so a build that still
/// has a role running keeps its directory. What can't be removed stays for
/// a later call, and is in [`Pruned::kept`] with its cause; no `copies` at
/// all is nothing to prune.
pub fn prune_helper_copies(copies: &Path, keep: &str) -> Pruned {
    let mut pruned = Pruned::default();
    let entries = match std::fs::read_dir(copies) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return pruned,
        Err(error) => {
            pruned
                .kept
                .push((copies.to_path_buf(), format!("can't read it: {error}")));
            return pruned;
        }
    };
    let entries = entries
        .flatten()
        .map(|entry| {
            // A reparse point is never a build's copy, whatever it points to,
            // and is never descended into.
            let kind = match entry.metadata() {
                Ok(metadata) if metadata.is_dir() && !is_reparse_point(metadata.file_attributes()) => {
                    EntryKind::Dir
                }
                Ok(_) | Err(_) => EntryKind::Other,
            };
            (entry.file_name(), kind)
        })
        .collect();
    for name in server_dir::pruned_builds(entries, keep) {
        let dir = copies.join(name);
        // The image first: while it runs, the directory is left whole.
        let removal = std::fs::remove_file(dir.join("brindle-remote.exe"));
        let image = match &removal {
            Ok(()) => ImageRemoval::Removed,
            Err(error) if error.kind() == io::ErrorKind::NotFound => ImageRemoval::Missing,
            Err(_) => ImageRemoval::Failed,
        };
        match server_dir::after_image_removal(image) {
            PruneDir::Remove => match std::fs::remove_dir_all(&dir) {
                Ok(()) => pruned.removed += 1,
                Err(error) => pruned.kept.push((dir, format!("can't remove it: {error}"))),
            },
            PruneDir::Keep => {
                let why = removal.err().map_or_else(
                    || "its image stays".to_string(),
                    |error| format!("its image can't be removed, so a role may run from it: {error}"),
                );
                pruned.kept.push((dir, why));
            }
        }
    }
    pruned
}

/// `SYSTEM_MANDATORY_LABEL_ACE_TYPE`: the entry of a SACL that is an
/// object's integrity label.
const MANDATORY_LABEL: u8 = 17;

/// Whether an entry with these attributes is a reparse point: a junction,
/// a symbolic link or anything else the system would follow or hand to a
/// filter instead of opening the entry itself.
fn is_reparse_point(attributes: u32) -> bool {
    attributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0
}

/// The directory an elevated process keeps what it runs, loads or locks
/// in, made or checked by [`protected_dir`] and held open while the
/// process uses it.
///
/// It holds the directory and its parent open, sharing reading and writing
/// but not deleting, so while it lives neither can be renamed or deleted:
/// the path that was checked stays the one that is used.
#[derive(Debug)]
pub struct ProtectedDir {
    path: PathBuf,
    _handle: OwnedHandle,
    _parent: OwnedHandle,
}

impl ProtectedDir {
    /// The directory, as it was asked for.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Makes or checks the directory `path` of an elevated process (its server
/// directory, its helper copies, its ConPTY files) and holds it open.
///
/// The directory must have exactly [`access::elevated_dir_sddl`]'s
/// descriptor: owned by Administrators, for Administrators and SYSTEM
/// only, with a high no-write-up label, so the user's unelevated processes
/// can't add, remove or rename anything in it. It is opened without
/// following a link, and it and its parent must be where they were asked
/// for: a parent that is, or sits under, a link is refused
/// ([`server_dir::same_final_path`]), since what it points to can change
/// once the check is done. What is found otherwise is set aside, made
/// fresh or protected again, as [`access::protected_step`] says for its
/// `contents`; its parent is made first when missing.
pub fn protected_dir(path: &Path, contents: Contents) -> io::Result<ProtectedDir> {
    protected_dir_as(
        path,
        contents,
        access::elevated_dir_sddl(),
        &access::matches_elevated,
    )
}

/// Checks the directory `path` of an elevated process without changing
/// it, and holds it: `None` when there is nothing there, an error when
/// what is there isn't the directory [`protected_dir`] makes. A server
/// role checks the directory it runs from this way.
pub fn hold_protected_dir(path: &Path) -> io::Result<Option<ProtectedDir>> {
    hold_protected_dir_as(path, &access::matches_elevated)
}

/// [`hold_protected_dir`], with the descriptor checked by `matches`.
fn hold_protected_dir_as(
    path: &Path,
    matches: &dyn Fn(&DirSecurity) -> bool,
) -> io::Result<Option<ProtectedDir>> {
    let parent = hold_parent(path)?;
    match look_held(path, matches)? {
        (Found::Missing, _) => Ok(None),
        (Found::Matches, Some(handle)) => Ok(Some(ProtectedDir {
            path: path.to_path_buf(),
            _handle: handle,
            _parent: parent,
        })),
        (found, _) => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!(
                "{} isn't a directory only an elevated process can write ({}): refusing it",
                path.display(),
                found_text(found)
            ),
        )),
    }
}

/// What was found, for a message.
fn found_text(found: Found) -> &'static str {
    match found {
        Found::Missing => "nothing is there",
        Found::Link => "it is a link",
        Found::NotDir => "it is a file",
        Found::Matches => "it is the protected directory",
        Found::Other => "its owner, access or label is another",
    }
}

/// [`protected_dir`], with the descriptor `sddl` that a directory is made
/// with and `matches`, which says whether one read back has it. The tests
/// make directories an unelevated process can make this way.
fn protected_dir_as(
    path: &Path,
    contents: Contents,
    sddl: &str,
    matches: &dyn Fn(&DirSecurity) -> bool,
) -> io::Result<ProtectedDir> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let failed = |what: String, error: io::Error| context(format_args!("{what} {}", path.display()), error);
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent).map_err(|error| failed("can't make the parent of".into(), error))?;
    }
    // Held from here on: what is made, set aside or protected below is in
    // the parent that was checked.
    let parent = hold_parent(path)?;
    let mut created = false;
    // Each round changes what is there, so a few always settle it: set
    // aside, made, used; a race with another process making it costs one.
    for _ in 0..4 {
        let (found, held) = look_held(path, matches)?;
        match (access::protected_step(found, contents, created), held) {
            (Step::Use, Some(handle)) => {
                return Ok(ProtectedDir {
                    path: path.to_path_buf(),
                    _handle: handle,
                    _parent: parent,
                });
            }
            (Step::Create, _) => match create_dir_as(path, sddl) {
                Ok(()) => created = true,
                // Another process made it first: look again.
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
                Err(error) => return Err(failed("can't make".into(), error)),
            },
            (Step::SetAside, held) => {
                // This process's own handle would stop the rename too.
                drop(held);
                let name = path.file_name().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default();
                let aside = path.with_file_name(server_dir::untrusted_name(
                    &name,
                    std::process::id(),
                    NEXT.fetch_add(1, Ordering::Relaxed),
                ));
                // Without `MOVEFILE_COPY_ALLOWED`: a link is renamed, never
                // followed, and nothing is copied.
                // SAFETY: both names are NUL-terminated and outlive the call.
                unsafe { MoveFileExW(&HSTRING::from(path), &HSTRING::from(aside.as_path()), MOVE_FILE_FLAGS(0)) }
                    .map_err(|error| failed(format!("can't set aside ({})", found_text(found)), os(error)))?;
            }
            (Step::Reprotect, Some(handle)) => {
                reprotect(&handle, path, sddl, matches).map_err(|error| failed("can't protect".into(), error))?;
                return Ok(ProtectedDir {
                    path: path.to_path_buf(),
                    _handle: handle,
                    _parent: parent,
                });
            }
            // A directory this process can't open isn't one it can protect.
            (Step::Use | Step::Reprotect, None) => {
                return Err(failed(
                    "can't open".into(),
                    io::Error::new(io::ErrorKind::PermissionDenied, found_text(found)),
                ));
            }
            (Step::Fail, _) => {
                return Err(failed(
                    "made, but it doesn't read back as made:".into(),
                    io::Error::new(io::ErrorKind::PermissionDenied, found_text(found)),
                ));
            }
        }
    }
    Err(failed(
        "can't settle".into(),
        io::Error::other("it kept changing while it was made"),
    ))
}

/// Opens `path` without following a link, sharing reading and writing but
/// not deleting, with `access` (which must include `FILE_LIST_DIRECTORY`
/// or other data access for the open to keep it from being renamed).
fn open_entry(path: &Path, access: u32, follow: bool) -> io::Result<OwnedHandle> {
    let flags = if follow {
        FILE_FLAG_BACKUP_SEMANTICS
    } else {
        FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT
    };
    // SAFETY: the name is NUL-terminated and outlives the call; the handle
    // is owned below.
    let handle = unsafe {
        CreateFileW(
            &HSTRING::from(path),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            None,
            OPEN_EXISTING,
            flags,
            None,
        )
    }
    .map_err(os)?;
    // SAFETY: a fresh handle nothing else owns.
    Ok(unsafe { OwnedHandle::from_raw_handle(handle.0) })
}

/// Refuses `handle`, opened by `path`, when the system has it somewhere
/// else: a directory on the way is a link.
fn check_final_path(path: &Path, handle: &OwnedHandle) -> io::Result<()> {
    let held = final_path(HANDLE(handle.as_raw_handle())).map_err(|error| context(path.display(), error))?;
    if server_dir::same_final_path(&long_path(path), &held) {
        return Ok(());
    }
    Err(io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!(
            "{} resolves to {held}, so a directory on the way is a link: refusing it",
            path.display()
        ),
    ))
}

/// Opens and holds the parent of `path`, following links, and refuses it
/// when it isn't where it was asked for.
fn hold_parent(path: &Path) -> io::Result<OwnedHandle> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("{} has no parent", path.display())))?;
    let handle = open_entry(parent, FILE_LIST_DIRECTORY.0 | FILE_READ_ATTRIBUTES.0, true)
        .map_err(|error| context(parent.display(), error))?;
    check_final_path(parent, &handle)?;
    Ok(handle)
}

/// Opens what is at `path` without following a link, sharing reading and
/// writing but not deleting, and says what it is. What was opened comes
/// back held; what can't be opened at all is [`Found::Other`] with no
/// handle. Something whose final path isn't `path` (a directory above it
/// is a link) is an error.
fn look_held(
    path: &Path,
    matches: &dyn Fn(&DirSecurity) -> bool,
) -> io::Result<(Found, Option<OwnedHandle>)> {
    // `FILE_LIST_DIRECTORY` is data access, which is what takes part in
    // sharing: a handle with only `READ_CONTROL` would block no rename.
    let handle = match open_entry(path, FILE_LIST_DIRECTORY.0 | READ_CONTROL.0 | FILE_READ_ATTRIBUTES.0, false) {
        Ok(handle) => handle,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok((Found::Missing, None)),
        Err(error) if error.kind() == io::ErrorKind::PermissionDenied => return Ok((Found::Other, None)),
        Err(error) => return Err(context(path.display(), error)),
    };
    // Before anything is read from it or done to it: what is set aside or
    // protected must be the entry asked for, not one a link leads to.
    check_final_path(path, &handle)?;
    let raw = HANDLE(handle.as_raw_handle());
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: the handle is open for the call and `info` is writable.
    unsafe { GetFileInformationByHandle(raw, &mut info) }.map_err(|error| context(path.display(), os(error)))?;
    if is_reparse_point(info.dwFileAttributes) {
        return Ok((Found::Link, Some(handle)));
    }
    if info.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 == 0 {
        return Ok((Found::NotDir, Some(handle)));
    }
    let found = match read_dir_security(raw) {
        Ok(security) if matches(&security) => Found::Matches,
        Ok(_) | Err(_) => Found::Other,
    };
    Ok((found, Some(handle)))
}

/// The path the system has for an open file (`GetFinalPathNameByHandleW`),
/// with its `\\?\` prefix.
fn final_path(handle: HANDLE) -> io::Result<String> {
    let mut buffer = vec![0u16; 1024];
    loop {
        // SAFETY: the handle is open and the buffer writable.
        let len = unsafe { GetFinalPathNameByHandleW(handle, &mut buffer, GETFINALPATHNAMEBYHANDLE_FLAGS(0)) }
            as usize;
        if len == 0 {
            return Err(io::Error::last_os_error());
        }
        if len < buffer.len() {
            return Ok(String::from_utf16_lossy(&buffer[..len]));
        }
        buffer.resize(len + 1, 0);
    }
}

/// `path` with its short (8.3) names long (`GetLongPathNameW`); as it is
/// when that fails.
fn long_path(path: &Path) -> String {
    let name = HSTRING::from(path);
    let mut buffer = vec![0u16; 1024];
    loop {
        // SAFETY: the name is NUL-terminated and the buffer writable.
        let len = unsafe { GetLongPathNameW(&name, Some(&mut buffer)) } as usize;
        if len == 0 {
            return path.to_string_lossy().into_owned();
        }
        if len < buffer.len() {
            return String::from_utf16_lossy(&buffer[..len]);
        }
        buffer.resize(len + 1, 0);
    }
}

/// The owner, DACL and label of the object `handle` is open on (with
/// `READ_CONTROL`).
fn read_dir_security(handle: HANDLE) -> io::Result<DirSecurity> {
    let mut owner = PSID::default();
    let mut dacl: *mut ACL = std::ptr::null_mut();
    let mut sacl: *mut ACL = std::ptr::null_mut();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    // SAFETY: the caller's open handle; every out pointer points into the
    // descriptor, which is freed below once they have been read.
    win32(unsafe {
        GetSecurityInfo(
            handle,
            SE_FILE_OBJECT,
            OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION | LABEL_SECURITY_INFORMATION,
            Some(&mut owner),
            None,
            Some(&mut dacl),
            Some(&mut sacl),
            Some(&mut descriptor),
        )
    })?;
    let _free = Local(descriptor.0);
    if owner.is_invalid() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "the object has no owner"));
    }
    // SAFETY: the owner SID points into the descriptor, which lives here.
    let owner = unsafe { sid_string(owner) }?;
    let dacl = Dacl::parse(descriptor, dacl)?;
    // SAFETY: as above, for the SACL.
    let label = unsafe { label_of(sacl) }?;
    Ok(DirSecurity { owner, dacl, label })
}

/// The first integrity label in `sacl`, if any.
///
/// # Safety
/// `sacl` is null or points to a valid ACL.
unsafe fn label_of(sacl: *const ACL) -> io::Result<Option<Label>> {
    if sacl.is_null() {
        return Ok(None);
    }
    // SAFETY: the caller's ACL; each entry `GetAce` returns starts with a
    // header, and a label entry holds its SID at `SidStart`, which has at
    // least one sub-authority (its level).
    unsafe {
        for index in 0..u32::from((*sacl).AceCount) {
            let mut entry: *mut c_void = std::ptr::null_mut();
            GetAce(sacl, index, &mut entry).map_err(os)?;
            let ace = entry.cast::<SYSTEM_MANDATORY_LABEL_ACE>();
            if (*ace).Header.AceType != MANDATORY_LABEL {
                continue;
            }
            let sid = PSID((&raw mut (*ace).SidStart).cast());
            let count = *GetSidSubAuthorityCount(sid);
            if count == 0 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "an empty label"));
            }
            return Ok(Some(Label {
                integrity: Integrity(*GetSidSubAuthority(sid, u32::from(count) - 1)),
                mask: (*ace).Mask,
                flags: (*ace).Header.AceFlags,
            }));
        }
    }
    Ok(None)
}

/// Makes the one directory `path` with the descriptor `sddl`.
fn create_dir_as(path: &Path, sddl: &str) -> io::Result<()> {
    let security = Security::from_sddl(sddl)?;
    // SAFETY: the attributes live until the call returns.
    unsafe { CreateDirectoryW(&HSTRING::from(path), Some(security.attributes())) }.map_err(os)
}

/// Puts `security`'s DACL, owner and label on the entry at `path` itself,
/// never one a link leads to, and on nothing inside it: the system
/// propagates nothing for these calls.
///
/// The DACL goes first, through the right to change it that an owner
/// always has, and it gives this process the right to change the owner,
/// which the owner and label then take.
fn set_security(path: &Path, security: &Security) -> io::Result<()> {
    let steps = [
        (WRITE_DAC.0, DACL_SECURITY_INFORMATION),
        (WRITE_OWNER.0, OWNER_SECURITY_INFORMATION | LABEL_SECURITY_INFORMATION),
    ];
    for (access, information) in steps {
        let handle =
            open_entry(path, access | READ_CONTROL.0, false).map_err(|error| context(path.display(), error))?;
        // SAFETY: an open handle with the access the call needs, and a
        // valid descriptor that outlives it.
        unsafe { SetKernelObjectSecurity(HANDLE(handle.as_raw_handle()), information, security.descriptor()) }
            .map_err(|error| context(path.display(), os(error)))?;
    }
    Ok(())
}

/// Protects the held directory `path` where it is: first the directory
/// itself, so nobody else can add, remove or rename anything in it, which
/// is checked; then what is inside, which it holds flat. A link inside is
/// deleted, never followed, and so is a directory inside, which a server
/// directory never holds and which could hide links; each file takes the
/// directory's descriptor without inheritance.
fn reprotect(
    held: &OwnedHandle,
    path: &Path,
    sddl: &str,
    matches: &dyn Fn(&DirSecurity) -> bool,
) -> io::Result<()> {
    set_security(path, &Security::from_sddl(sddl)?)?;
    let security = read_dir_security(HANDLE(held.as_raw_handle()))?;
    if !matches(&security) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("the protection didn't hold: {security:?}"),
        ));
    }
    let file = Security::from_sddl(&access::contained_file_sddl(sddl))?;
    for entry in std::fs::read_dir(path)? {
        let entry = entry?.path();
        let metadata = std::fs::symlink_metadata(&entry).map_err(|error| context(entry.display(), error))?;
        let attributes = metadata.file_attributes();
        let removed = if is_reparse_point(attributes) {
            if attributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0 {
                std::fs::remove_dir(&entry)
            } else {
                std::fs::remove_file(&entry)
            }
        } else if metadata.is_dir() {
            // Deletes links inside, never what they point to.
            std::fs::remove_dir_all(&entry)
        } else {
            set_security(&entry, &file)
        };
        removed.map_err(|error| context(entry.display(), error))?;
    }
    Ok(())
}

/// How a role opens a lock or log in an elevated server directory
/// ([`open_elevated_file`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileOpen {
    /// A lock: read and written, and made when `create` and missing.
    Lock { create: bool },
    /// A log: appended to, and made when missing.
    Log,
}

/// Opens a lock or log of an elevated role in its server directory without
/// following a link. A file it makes is owned by Administrators, for them
/// and SYSTEM only; one it finds must be a plain file of one name owned by
/// Administrators ([`access::check_file`]), or it is refused, naming the
/// file and why. Protecting the directory stops what an unelevated process
/// adds to it later, but not what one adds through a handle it opened
/// before: this check stops that, file by file.
pub fn open_elevated_file(path: &Path, open: FileOpen) -> io::Result<File> {
    open_owned_file(
        path,
        open,
        &access::contained_file_sddl(access::elevated_dir_sddl()),
        access::ADMINISTRATORS_SID,
    )
}

/// [`open_elevated_file`], making a file with the descriptor `sddl` and
/// requiring the owner `owner`.
fn open_owned_file(path: &Path, open: FileOpen, sddl: &str, owner: &str) -> io::Result<File> {
    let refused = |why: String| io::Error::new(io::ErrorKind::PermissionDenied, format!("refusing {}: {why}", path.display()));
    // Applied only to a file this call makes.
    let security = Security::from_sddl(sddl)?;
    let (access, disposition) = match open {
        FileOpen::Lock { create } => (
            FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
            if create { OPEN_ALWAYS } else { OPEN_EXISTING },
        ),
        // Appending only, as std's `append`, and reading its attributes.
        FileOpen::Log => ((FILE_GENERIC_WRITE.0 & !FILE_WRITE_DATA.0) | FILE_READ_ATTRIBUTES.0, OPEN_ALWAYS),
    };
    // SAFETY: the name is NUL-terminated and the attributes outlive the
    // call; the handle is owned below.
    let handle = unsafe {
        CreateFileW(
            &HSTRING::from(path),
            access,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            Some(security.attributes()),
            disposition,
            FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
            None,
        )
    }
    .map_err(|error| context(path.display(), os(error)))?;
    // SAFETY: a fresh handle nothing else owns.
    let file = unsafe { File::from_raw_handle(handle.0) };
    let raw = HANDLE(file.as_raw_handle());
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: the handle is open for the call and `info` is writable.
    unsafe { GetFileInformationByHandle(raw, &mut info) }.map_err(|error| context(path.display(), os(error)))?;
    let facts = access::FileFacts {
        reparse_point: is_reparse_point(info.dwFileAttributes),
        links: info.nNumberOfLinks,
        owner: read_dir_security(raw).map_err(|error| context(path.display(), error))?.owner,
    };
    access::check_file(&facts, owner).map_err(|refusal| refused(refusal.to_string()))?;
    Ok(file)
}

/// What [`remove_untrusted`] did.
#[derive(Debug, Default)]
pub struct Removed {
    /// How many directories set aside it removed.
    pub removed: usize,
    /// What it left, and why: a link, which is never followed, or one that
    /// couldn't be removed.
    pub kept: Vec<(PathBuf, String)>,
}

/// Removes the directories [`protected_dir`] set aside in `dir`
/// ([`server_dir::is_untrusted_name`]), where it can. A link among them is
/// left alone, never followed, and what can't be removed stays for a later
/// call; no `dir` is nothing to remove.
pub fn remove_untrusted(dir: &Path) -> Removed {
    let mut removed = Removed::default();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return removed;
    };
    for entry in entries.flatten() {
        if !entry.file_name().to_str().is_some_and(server_dir::is_untrusted_name) {
            continue;
        }
        let path = entry.path();
        let link = std::fs::symlink_metadata(&path).map(|metadata| is_reparse_point(metadata.file_attributes()));
        match link {
            Ok(true) => removed.kept.push((path, "it is a link".into())),
            // `remove_dir_all` opens what it removes without following it and
            // works relative to the handles it holds, so an entry swapped
            // for a link after this check, or a link inside, is deleted as a
            // link and never descended into. A file set aside is removed as
            // a file (a file link as the link itself).
            Ok(false) => match std::fs::remove_dir_all(&path).or_else(|_| std::fs::remove_file(&path)) {
                Ok(()) => removed.removed += 1,
                Err(error) => removed.kept.push((path, format!("can't remove it: {error}"))),
            },
            Err(error) => removed.kept.push((path, format!("can't read it: {error}"))),
        }
    }
    removed
}

#[cfg(test)]
mod tests {
    use std::fs::{File, OpenOptions};
    use std::os::windows::fs::OpenOptionsExt;

    use windows::Win32::Storage::FileSystem::GetShortPathNameW;

    use windows::Win32::Foundation::INVALID_HANDLE_VALUE;
    use windows::Win32::Storage::FileSystem::{FILE_FLAG_FIRST_PIPE_INSTANCE, PIPE_ACCESS_DUPLEX};
    use windows::Win32::System::Pipes::{
        CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
    };

    use super::*;

    /// A pipe this process serves, created with the server's protections,
    /// and its name.
    fn server_pipe(tag: &str) -> (File, String) {
        let name = format!(r"\\.\pipe\brindle-unit-{tag}-{}", std::process::id());
        let security = Security::for_pipe().unwrap();
        // SAFETY: the attributes outlive the call; the handle is owned by
        // the File below.
        let handle = unsafe {
            CreateNamedPipeW(
                &HSTRING::from(name.as_str()),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                4096,
                4096,
                0,
                Some(security.attributes()),
            )
        };
        assert_ne!(
            handle,
            INVALID_HANDLE_VALUE,
            "{}",
            io::Error::last_os_error()
        );
        // SAFETY: a fresh handle nothing else owns.
        (unsafe { File::from_raw_handle(handle.0) }, name)
    }

    #[test]
    fn this_process_has_a_user_sid_and_an_integrity_level() {
        let own = own_principal().unwrap();
        assert!(own.sid.starts_with("S-1-5-"), "{}", own.sid);
        assert_eq!(own_sid().unwrap(), own.sid);
        assert_eq!(own_integrity().unwrap(), own.integrity);
        // Tests run as a normal or an elevated user (CI's runner).
        assert!(own.integrity >= Integrity::MEDIUM, "{}", own.integrity);
        assert_eq!(is_elevated().unwrap(), own.integrity >= Integrity::HIGH);
    }

    #[test]
    fn this_process_read_by_pid_is_the_same_principal() {
        assert_eq!(
            principal_of(std::process::id()).unwrap(),
            own_principal().unwrap()
        );
    }

    #[test]
    fn a_process_that_cannot_be_opened_is_an_error() {
        // Pids are multiples of four, so this one never exists.
        let error = principal_of(u32::MAX).unwrap_err();
        assert!(
            error
                .to_string()
                .starts_with("process 4294967295: can't open it"),
            "{error}"
        );
    }

    #[test]
    fn a_server_pipe_allows_only_the_user_and_system() {
        let (pipe, _name) = server_pipe("dacl");
        let dacl = Dacl::of_handle(pipe.as_raw_handle()).unwrap();
        let own = own_sid().unwrap();
        assert!(dacl.allows_only(&[&own, SYSTEM_SID]), "{dacl:?}");
        assert_eq!(dacl.allowed.len(), 2);
    }

    #[test]
    fn both_labels_build_a_descriptor() {
        // Only the label at this process's level can be put on a pipe, but
        // both strings must parse.
        let own = own_sid().unwrap();
        for elevated in [false, true] {
            Security::from_sddl(&pipe_sddl(&own, elevated)).unwrap();
        }
        let error = Security::from_sddl("D:P(A;;GA;;;nobody)").err().unwrap();
        assert!(error.to_string().contains("D:P(A;;GA;;;nobody)"), "{error}");
    }

    #[test]
    fn a_pipe_this_process_serves_passes_the_client_check() {
        let (_pipe, name) = server_pipe("client");
        let client = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&name)
            .unwrap();
        assert_eq!(pipe_server_pid(&client).unwrap(), std::process::id());
        check_server(&client).unwrap();
    }

    #[test]
    fn a_handle_that_is_no_pipe_fails_the_client_check() {
        let file = File::open(std::env::current_exe().unwrap()).unwrap();
        let error = check_server(&file).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(
            error
                .to_string()
                .starts_with("refusing the server on this pipe"),
            "{error}"
        );
    }

    #[test]
    fn a_server_of_another_user_or_level_is_refused() {
        // What `check_server` decides with, for servers this test can't run.
        let own = own_principal().unwrap();
        let other_user = Principal {
            sid: format!("{}9", own.sid),
            ..own.clone()
        };
        let refusal = peer_allowed(&own, &other_user, Side::Client).unwrap_err();
        assert!(matches!(refusal, Refusal::OtherUser { .. }), "{refusal}");
        for level in [
            Integrity::LOW,
            Integrity::MEDIUM,
            Integrity::HIGH,
            Integrity::SYSTEM,
        ] {
            let server = Principal {
                integrity: level,
                ..own.clone()
            };
            let answer = peer_allowed(&own, &server, Side::Client);
            assert_eq!(
                answer.is_ok(),
                level == own.integrity,
                "{level}: {answer:?}"
            );
        }
    }

    #[test]
    fn a_private_directory_allows_only_the_user_and_system() {
        let dir = std::env::temp_dir().join(format!("brindle-unit-win-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        create_private_dir(&dir).unwrap();
        let own = own_sid().unwrap();
        let dacl = Dacl::of_path(&dir).unwrap();
        assert!(dacl.allows_only(&[&own, SYSTEM_SID]), "{dacl:?}");
        let error = create_private_dir(&dir).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::AlreadyExists);

        // A file made inside inherits the same two entries.
        let file = dir.join("core.lock");
        std::fs::write(&file, b"").unwrap();
        let inherited = Dacl::of_path(&file).unwrap();
        let mut sids: Vec<&str> = inherited
            .allowed
            .iter()
            .map(|ace| ace.sid.as_str())
            .collect();
        sids.sort_unstable();
        let mut wanted = [own.as_str(), SYSTEM_SID];
        wanted.sort_unstable();
        assert_eq!(sids, wanted, "{inherited:?}");

        // An ordinary directory inherits its parent's entries until protected.
        let plain = dir.join("plain");
        std::fs::create_dir(&plain).unwrap();
        assert!(!Dacl::of_path(&plain).unwrap().protected);
        protect_dir(&plain).unwrap();
        let dacl = Dacl::of_path(&plain).unwrap();
        assert!(dacl.allows_only(&[&own, SYSTEM_SID]), "{dacl:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("brindle-win-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }

    fn cmd_exe() -> PathBuf {
        PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join(r"System32\cmd.exe")
    }

    #[test]
    fn file_identity_tells_a_recreated_file_from_the_first() {
        let dir = scratch("identity");
        let path = dir.join("a");
        let first = File::create(&path).unwrap();
        let again = File::open(&path).unwrap();
        assert_eq!(
            file_identity(&first).unwrap(),
            file_identity(&again).unwrap()
        );
        std::fs::remove_file(&path).unwrap();
        let second = File::create(&path).unwrap();
        assert_ne!(
            file_identity(&first).unwrap(),
            file_identity(&second).unwrap()
        );
        drop((first, again, second));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn own_image_path_is_this_test_binary() {
        let path = own_image_path().unwrap();
        assert!(path.is_absolute() && path.is_file());
        assert_eq!(path.extension().unwrap(), "exe");
    }

    #[test]
    fn the_copy_is_named_by_build_and_keeps_the_last_write_time() {
        let dir = scratch("copy");
        let installed = dir.join("installed.exe");
        std::fs::write(&installed, b"a helper").unwrap();
        let written = std::time::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
        File::options()
            .write(true)
            .open(&installed)
            .unwrap()
            .set_modified(written)
            .unwrap();
        let copies = dir.join("bin");

        let copy = ensure_helper_copy(&installed, &copies).unwrap();
        let build = crate::build_id::of_binary(b"a helper");
        assert_eq!(copy, crate::server_dir::helper_copy(&copies, &build));
        assert_eq!(std::fs::read(&copy).unwrap(), b"a helper");
        assert_eq!(
            std::fs::metadata(&copy).unwrap().modified().unwrap(),
            written
        );
        assert_eq!(names(copy.parent().unwrap()), ["brindle-remote.exe"]);

        // A second call finds the copy and leaves it alone.
        let identity = file_identity(&File::open(&copy).unwrap()).unwrap();
        assert_eq!(ensure_helper_copy(&installed, &copies).unwrap(), copy);
        assert_eq!(
            file_identity(&File::open(&copy).unwrap()).unwrap(),
            identity
        );

        // Another build gets its own directory.
        std::fs::write(&installed, b"a newer helper").unwrap();
        let newer = ensure_helper_copy(&installed, &copies).unwrap();
        assert_ne!(newer, copy);
        assert_eq!(std::fs::read(&copy).unwrap(), b"a helper");
        assert_eq!(std::fs::read(&newer).unwrap(), b"a newer helper");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_wrong_file_under_the_copys_name_is_replaced() {
        let dir = scratch("wrong");
        let installed = dir.join("installed.exe");
        std::fs::write(&installed, b"a helper").unwrap();
        let copies = dir.join("bin");
        let build = crate::build_id::of_binary(b"a helper");
        let copy = crate::server_dir::helper_copy(&copies, &build);
        std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
        std::fs::write(&copy, b"cut short").unwrap();

        assert_eq!(ensure_helper_copy(&installed, &copies).unwrap(), copy);
        assert_eq!(std::fs::read(&copy).unwrap(), b"a helper");
        assert_eq!(names(copy.parent().unwrap()), ["brindle-remote.exe"]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_helper_is_an_error_naming_it() {
        let dir = scratch("missing");
        let error = ensure_helper_copy(&dir.join("nope.exe"), &dir.join("bin")).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(error.to_string().contains("nope.exe"), "{error}");
        assert!(!dir.join("bin").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn racing_copies_end_with_one_valid_copy() {
        let dir = scratch("race");
        let installed = dir.join("installed.exe");
        let bytes: Vec<u8> = (0..4_000_000u32).map(|n| (n % 251) as u8).collect();
        std::fs::write(&installed, &bytes).unwrap();
        let copies = dir.join("bin");

        for round in 0..8 {
            let _ = std::fs::remove_dir_all(&copies);
            let barrier = std::sync::Barrier::new(2);
            let race = || {
                barrier.wait();
                ensure_helper_copy(&installed, &copies)
            };
            let (a, b) = std::thread::scope(|scope| {
                let a = scope.spawn(race);
                let b = scope.spawn(race);
                (a.join().unwrap(), b.join().unwrap())
            });
            let (a, b) = (a.unwrap(), b.unwrap());
            assert_eq!(a, b, "round {round}");
            assert_eq!(std::fs::read(&a).unwrap(), bytes, "round {round}");
            assert_eq!(names(&copies).len(), 1, "round {round}");
            assert_eq!(
                names(a.parent().unwrap()),
                ["brindle-remote.exe"],
                "round {round}"
            );
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn pruning_skips_a_build_whose_helper_is_running() {
        let dir = scratch("prune");
        let copies = dir.join("bin");
        let running = crate::server_dir::helper_copy(&copies, "running");
        let stale = crate::server_dir::helper_copy(&copies, "stale");
        let kept = crate::server_dir::helper_copy(&copies, "kept");
        for copy in [&running, &stale, &kept] {
            std::fs::create_dir_all(copy.parent().unwrap()).unwrap();
            std::fs::copy(cmd_exe(), copy).unwrap();
        }
        std::fs::write(running.with_file_name("extra.txt"), b"").unwrap();
        std::fs::write(copies.join("a-file"), b"").unwrap();
        let mut child = std::process::Command::new(&running)
            .args(["/c", "ping -n 30 127.0.0.1 >nul"])
            .spawn()
            .unwrap();

        let pruned = prune_helper_copies(&copies, "kept");
        assert_eq!(names(&copies), ["a-file", "kept", "running"]);
        assert!(kept.is_file());
        // The directory in use is left whole, not half emptied, and kept
        // with its reason.
        assert_eq!(
            names(running.parent().unwrap()),
            ["brindle-remote.exe", "extra.txt"]
        );
        assert_eq!(pruned.removed, 1, "{pruned:?}");
        assert_eq!(pruned.kept.len(), 1, "{pruned:?}");
        let (held, why) = &pruned.kept[0];
        assert_eq!(held.as_path(), running.parent().unwrap(), "{pruned:?}");
        assert!(why.contains("its image can't be removed"), "{why}");

        child.kill().unwrap();
        child.wait().unwrap();
        // The dead process's image is released soon after its exit, not at
        // it; the limit is scaled.
        assert!(
            crate::test_support::eventually(std::time::Duration::from_secs(20), || {
                prune_helper_copies(&copies, "kept");
                !running.parent().unwrap().exists()
            }),
            "the copy stayed"
        );
        assert_eq!(names(&copies), ["a-file", "kept"]);

        // No directory at all is nothing to prune, and nothing kept.
        let pruned = prune_helper_copies(&dir.join("absent"), "kept");
        assert_eq!((pruned.removed, pruned.kept.len()), (0, 0), "{pruned:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A junction among the copies is no build's copy: neither it nor what
    /// it points to is touched.
    #[test]
    fn pruning_leaves_a_junction_and_its_target_alone() {
        let dir = scratch("prune-junction");
        let copies = dir.join("bin");
        let target = dir.join("elsewhere");
        std::fs::create_dir_all(&copies).unwrap();
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("brindle-remote.exe"), b"x").unwrap();
        std::fs::write(target.join("other"), b"x").unwrap();
        junction(&copies.join("stale"), &target);
        let pruned = prune_helper_copies(&copies, "kept");
        assert_eq!((pruned.removed, pruned.kept.len()), (0, 0), "{pruned:?}");
        assert_eq!(names(&copies), ["stale"]);
        assert_eq!(names(&target), ["brindle-remote.exe", "other"]);
        std::fs::remove_dir(copies.join("stale")).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_processs_image_path_is_its_executable() {
        let path = image_path_of(std::process::id()).unwrap();
        let own = std::env::current_exe().unwrap();
        assert!(
            path.as_os_str().eq_ignore_ascii_case(own.as_os_str()),
            "{} isn't {}",
            path.display(),
            own.display()
        );
        // No process has pid 0's image to give.
        assert!(image_path_of(0).is_err());
    }

    /// The protected form for this test's own user at medium, which an
    /// unelevated process can make: [`access::elevated_dir_sddl`]'s shape.
    fn own_protection() -> (String, impl Fn(&DirSecurity) -> bool) {
        let sid = own_sid().unwrap();
        let sddl = format!("O:{sid}D:P(A;OICI;FA;;;{sid})(A;OICI;FA;;;SY)S:(ML;OICI;NW;;;ME)");
        (sddl, move |found: &DirSecurity| access::matches_owned(found, &sid, Integrity::MEDIUM))
    }

    fn protect(path: &Path, contents: Contents) -> io::Result<ProtectedDir> {
        let (sddl, matches) = own_protection();
        protected_dir_as(path, contents, &sddl, &matches)
    }

    fn read_security(path: &Path) -> DirSecurity {
        let handle = open_entry(path, READ_CONTROL.0, false).unwrap();
        read_dir_security(HANDLE(handle.as_raw_handle())).unwrap()
    }

    /// Makes `link` a junction to `target`, which needs no privilege.
    fn junction(link: &Path, target: &Path) {
        let status = std::process::Command::new(cmd_exe())
            .arg("/c")
            .arg("mklink")
            .arg("/J")
            .arg(link)
            .arg(target)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "mklink /J {} {}", link.display(), target.display());
    }

    fn set_aside(dir: &Path) -> Vec<String> {
        names(dir).into_iter().filter(|name| server_dir::is_untrusted_name(name)).collect()
    }

    #[test]
    fn a_missing_protected_directory_is_made_and_then_used() {
        let dir = scratch("protect-missing");
        let path = dir.join("server-elevated");
        let (_, matches) = own_protection();
        let made = protect(&path, Contents::Executables).unwrap();
        assert_eq!(made.path(), path);
        assert!(matches(&read_security(&path)), "{:?}", read_security(&path));
        std::fs::write(path.join("kept"), "x").unwrap();
        drop(made);
        // Found as made: used, with what it holds.
        let used = protect(&path, Contents::Executables).unwrap();
        assert!(path.join("kept").exists());
        assert!(set_aside(&dir).is_empty(), "{:?}", names(&dir));
        drop(used);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A junction in the directory's place is renamed aside, never
    /// followed: what it points to is left as it was.
    #[test]
    fn a_junction_in_the_directory_s_place_is_set_aside_and_its_target_untouched() {
        let dir = scratch("protect-junction");
        let target = dir.join("elsewhere");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("planted.exe"), "x").unwrap();
        let before = read_security(&target);
        let path = dir.join("server-elevated");
        junction(&path, &target);
        for contents in [Contents::Executables, Contents::State] {
            let held = protect(&path, contents).unwrap();
            assert!(!is_reparse_point(std::fs::symlink_metadata(&path).unwrap().file_attributes()));
            assert!(names(&path).is_empty(), "{:?}", names(&path));
            drop(held);
            std::fs::remove_dir(&path).unwrap();
            junction(&path, &target);
        }
        assert_eq!(names(&target), ["planted.exe"]);
        assert_eq!(read_security(&target), before);
        assert_eq!(set_aside(&dir).len(), 2, "{:?}", names(&dir));
        std::fs::remove_dir(&path).unwrap();
        assert_eq!(remove_untrusted(&dir).removed, 0, "both set aside are links");
        assert_eq!(names(&target), ["planted.exe"]);
        for name in set_aside(&dir) {
            std::fs::remove_dir(dir.join(name)).unwrap();
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A directory the user made holds what can't be trusted to run: it is
    /// set aside whole and made fresh.
    #[test]
    fn a_directory_for_executables_with_another_descriptor_is_set_aside() {
        let dir = scratch("protect-executables");
        let path = dir.join("server-elevated");
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("brindle-remote.exe"), "planted").unwrap();
        let held = protect(&path, Contents::Executables).unwrap();
        assert!(names(&path).is_empty(), "{:?}", names(&path));
        let aside = set_aside(&dir);
        assert_eq!(aside.len(), 1, "{:?}", names(&dir));
        assert_eq!(names(&dir.join(&aside[0])), ["brindle-remote.exe"]);
        drop(held);
        assert_eq!(remove_untrusted(&dir).removed, 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A state directory is protected where it is: the directory first,
    /// then links and directories inside are deleted (never followed) and
    /// files take its descriptor, one held open by an older core included.
    #[test]
    fn a_state_directory_with_another_descriptor_is_protected_in_place() {
        let dir = scratch("protect-state");
        let target = dir.join("elsewhere");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("outside"), "x").unwrap();
        let path = dir.join("run-elevated");
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("core.log"), "log").unwrap();
        std::fs::create_dir(path.join("inner")).unwrap();
        std::fs::write(path.join("inner").join("file"), "x").unwrap();
        junction(&path.join("link"), &target);
        // An older core's lock, open with no sharing at all.
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .share_mode(0)
            .open(path.join("core.lock"))
            .unwrap();
        let (_, matches) = own_protection();
        let held = protect(&path, Contents::State).unwrap();
        assert!(matches(&read_security(&path)), "{:?}", read_security(&path));
        assert_eq!(names(&path), ["core.lock", "core.log"]);
        assert_eq!(std::fs::read_to_string(path.join("core.log")).unwrap(), "log");
        let sid = own_sid().unwrap();
        for file in ["core.lock", "core.log"] {
            let found = read_security(&path.join(file));
            assert_eq!(found.owner, sid, "{file}");
            assert!(found.dacl.protected, "{file}: {found:?}");
            assert!(found.dacl.allows_only(&[&sid, SYSTEM_SID]), "{file}: {found:?}");
        }
        assert_eq!(names(&target), ["outside"]);
        assert!(set_aside(&dir).is_empty(), "{:?}", names(&dir));
        drop((held, lock));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A parent that is a link, or under one, is refused before anything
    /// is made in what it points to.
    #[test]
    fn a_directory_under_a_link_is_refused_and_nothing_is_made_there() {
        let dir = scratch("protect-linked-parent");
        let target = dir.join("elsewhere");
        std::fs::create_dir(&target).unwrap();
        let linked = dir.join("brindle");
        junction(&linked, &target);
        for path in [linked.join("server-elevated"), linked.join("deeper").join("server-elevated")] {
            let error = protect(&path, Contents::Executables).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{error}");
            assert!(error.to_string().contains("resolves to"), "{error}");
            assert!(hold_protected_dir_as(&path, &own_protection().1).is_err());
        }
        // `create_dir_all` made the deeper parent, but no protected one.
        assert_eq!(names(&target), ["deeper"]);
        assert!(names(&target.join("deeper")).is_empty());
        std::fs::remove_dir(&linked).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// While held, neither the directory nor its parent can be renamed, so
    /// the path checked stays the one used.
    #[test]
    fn a_held_directory_and_its_parent_cannot_be_renamed() {
        let dir = scratch("protect-held");
        let parent = dir.join("brindle");
        let path = parent.join("server-elevated");
        let held = protect(&path, Contents::Executables).unwrap();
        assert!(std::fs::rename(&path, parent.join("moved")).is_err());
        assert!(std::fs::rename(&parent, dir.join("moved")).is_err());
        drop(held);
        std::fs::rename(&path, parent.join("moved")).unwrap();
        std::fs::rename(&parent, dir.join("moved")).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A path with short (8.3) names, as `%TEMP%` often has, is where it
    /// was asked for.
    #[test]
    fn a_path_with_short_names_is_accepted() {
        let dir = scratch("protect-short-name-dir");
        let long = HSTRING::from(dir.as_path());
        let mut buffer = vec![0u16; 1024];
        // SAFETY: the name is NUL-terminated and the buffer writable.
        let len = unsafe { GetShortPathNameW(&long, Some(&mut buffer)) } as usize;
        assert!(len > 0 && len < buffer.len(), "{}", io::Error::last_os_error());
        let short = PathBuf::from(String::from_utf16_lossy(&buffer[..len]));
        let path = short.join("server-elevated");
        let held = protect(&path, Contents::Executables).unwrap();
        assert!(hold_protected_dir_as(&path, &own_protection().1).unwrap().is_some());
        drop(held);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A role checks the directory it runs from without changing it.
    #[test]
    fn holding_a_protected_directory_changes_nothing() {
        let dir = scratch("protect-hold");
        let path = dir.join("server-elevated");
        let matches = own_protection().1;
        assert!(hold_protected_dir_as(&path, &matches).unwrap().is_none());
        std::fs::create_dir(&path).unwrap();
        let error = hold_protected_dir_as(&path, &matches).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{error}");
        assert!(set_aside(&dir).is_empty());
        std::fs::remove_dir(&path).unwrap();
        drop(protect(&path, Contents::Executables).unwrap());
        assert!(hold_protected_dir_as(&path, &matches).unwrap().is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A lock or log of the user's own is made with the given descriptor
    /// and opened again; a hard link, a link and a file someone else owns
    /// are refused, and what a link points to is left alone.
    #[test]
    fn an_elevated_role_s_files_are_its_own_plain_files() {
        let dir = scratch("owned-file");
        let sid = own_sid().unwrap();
        let sddl = format!("O:{sid}D:P(A;;FA;;;{sid})(A;;FA;;;SY)S:(ML;;NW;;;ME)");
        let open = |path: &Path, how: FileOpen| open_owned_file(path, how, &sddl, &sid);
        let lock = dir.join("core.lock");
        let log = dir.join("core.log");
        drop(open(&lock, FileOpen::Lock { create: true }).unwrap());
        drop(open(&lock, FileOpen::Lock { create: false }).unwrap());
        assert_eq!(read_security(&lock).owner, sid);
        {
            use std::io::Write as _;
            open(&log, FileOpen::Log).unwrap().write_all(b"one\n").unwrap();
            open(&log, FileOpen::Log).unwrap().write_all(b"two\n").unwrap();
        }
        assert_eq!(std::fs::read_to_string(&log).unwrap(), "one\ntwo\n");
        let missing = open(&dir.join("absent.lock"), FileOpen::Lock { create: false }).unwrap_err();
        assert_eq!(missing.kind(), io::ErrorKind::NotFound, "{missing}");

        // A second name for a file elsewhere.
        let outside = dir.join("outside");
        std::fs::write(&outside, b"keep").unwrap();
        std::fs::hard_link(&outside, dir.join("holder-b-0.log")).unwrap();
        let error = open(&dir.join("holder-b-0.log"), FileOpen::Log).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{error}");
        assert!(error.to_string().contains("2 names"), "{error}");
        // A junction in a file's place: refused, its target untouched.
        let target = dir.join("elsewhere");
        std::fs::create_dir(&target).unwrap();
        junction(&dir.join("holder-b-1.lock"), &target);
        assert!(open(&dir.join("holder-b-1.lock"), FileOpen::Lock { create: true }).is_err());
        assert!(names(&target).is_empty());
        // A file another owner made.
        let error = open_owned_file(&lock, FileOpen::Lock { create: true }, &sddl, SYSTEM_SID).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied, "{error}");
        assert!(error.to_string().contains(&format!("owned by {sid}")), "{error}");
        assert_eq!(std::fs::read(&outside).unwrap(), b"keep");
        std::fs::remove_dir(dir.join("holder-b-1.lock")).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// What was set aside is removed later, except a link, which is never
    /// followed.
    #[test]
    fn set_aside_directories_are_removed_but_a_link_among_them_is_kept() {
        let dir = scratch("protect-untrusted");
        let target = dir.join("elsewhere");
        std::fs::create_dir(&target).unwrap();
        std::fs::write(target.join("outside"), "x").unwrap();
        let aside = dir.join(server_dir::untrusted_name("server-elevated", 1, 0));
        std::fs::create_dir(&aside).unwrap();
        std::fs::write(aside.join("planted.exe"), "x").unwrap();
        let link = dir.join(server_dir::untrusted_name("conpty-elevated", 1, 1));
        junction(&link, &target);
        let removed = remove_untrusted(&dir);
        assert_eq!(removed.removed, 1);
        assert_eq!(removed.kept.len(), 1, "{:?}", removed.kept);
        assert_eq!(removed.kept[0].0, link);
        assert!(!aside.exists());
        assert_eq!(names(&target), ["outside"]);
        // A set-aside directory swapped for a junction after the check, as
        // another process could: the removal deletes the link itself and
        // never descends into its target (std's `remove_dir_all` opens what
        // it removes without following it, and works relative to handles).
        std::fs::remove_dir_all(&link).unwrap();
        assert!(std::fs::symlink_metadata(&link).is_err());
        assert_eq!(names(&target), ["outside"]);
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(remove_untrusted(&dir).removed, 0);
    }
}
