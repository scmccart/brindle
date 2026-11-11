//! Windows tokens and security descriptors for the terminal server, shared
//! by the server (`brindle-remote`) and its clients: who this process and
//! the process at the other end of a pipe run as, and the protections pipes
//! and the server directory are created with. The rules themselves are in
//! [`crate::access`], re-exported here.

use std::ffi::c_void;
use std::fs::File;
use std::io;
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
    ACCESS_ALLOWED_ACE, ACL, DACL_SECURITY_INFORMATION, GetAce, GetSecurityDescriptorControl,
    GetSecurityDescriptorDacl, GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation,
    PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SE_DACL_PROTECTED,
    SECURITY_ATTRIBUTES, TOKEN_INFORMATION_CLASS, TOKEN_MANDATORY_LABEL, TOKEN_QUERY, TOKEN_USER,
    TokenIntegrityLevel, TokenUser,
};
use windows::Win32::Storage::FileSystem::{
    BY_HANDLE_FILE_INFORMATION, CreateDirectoryW, GetFileInformationByHandle, MOVE_FILE_FLAGS,
    MoveFileExW,
};
use windows::Win32::System::Pipes::GetNamedPipeServerProcessId;
use windows::Win32::System::Threading::{
    GetCurrentProcess, OpenProcess, OpenProcessToken, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::core::{BOOL, HSTRING, PWSTR};

pub use crate::access::{Integrity, Principal, Refusal, Side, dir_sddl, peer_allowed, pipe_sddl};

/// `ACCESS_ALLOWED_ACE_TYPE`: the only entry type [`Dacl`] reads a SID from.
const ACCESS_ALLOWED: u8 = 0;

fn context(what: impl std::fmt::Display, error: io::Error) -> io::Error {
    io::Error::new(error.kind(), format!("{what}: {error}"))
}

/// The `windows` crate's error as an OS error, so its `kind()` is the usual
/// one: the crate's own conversion keeps the HRESULT, which std doesn't know.
fn os(error: windows::core::Error) -> io::Error {
    let code = error.code().0 as u32;
    if code >> 16 == 0x8007 {
        io::Error::from_raw_os_error((code & 0xffff) as i32)
    } else {
        io::Error::other(error)
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

/// One access-allowed entry of a DACL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ace {
    /// The trustee, as a SID string.
    pub sid: String,
    /// The access mask.
    pub mask: u32,
    /// The entry's flags (inheritance).
    pub flags: u8,
}

/// An object's discretionary ACL, as read back from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dacl {
    /// Whether the DACL is protected from inheriting its parent's entries.
    pub protected: bool,
    /// The access-allowed entries, in order.
    pub allowed: Vec<Ace>,
    /// How many entries are of another kind (deny, and so on).
    pub others: usize,
}

impl Dacl {
    /// Whether the DACL is protected and allows exactly `sids`, each once,
    /// with no other entries.
    pub fn allows_only(&self, sids: &[&str]) -> bool {
        let mut found: Vec<&str> = self.allowed.iter().map(|ace| ace.sid.as_str()).collect();
        let mut wanted = sids.to_vec();
        found.sort_unstable();
        wanted.sort_unstable();
        self.protected && self.others == 0 && found == wanted
    }

    /// Reads the descriptor `GetSecurityInfo` or its named form returned.
    fn read(descriptor: PSECURITY_DESCRIPTOR, dacl: *const ACL) -> io::Result<Self> {
        let _free = Local(descriptor.0);
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
    let copy = crate::server_dir::helper_copy(copies, &build);
    if build_of(&copy).as_deref() == Some(&build) {
        return Ok(copy);
    }
    let dir = copy.parent().expect("the copy is in its build's directory");
    std::fs::create_dir_all(dir).map_err(|error| context(dir.display(), error))?;
    let temp = dir.join(format!(
        "brindle-remote.exe.{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
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
    if build_of(temp).as_deref() != Some(build) {
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
    if build_of(copy).as_deref() == Some(build) {
        // Another process won the rename with the same build.
        return std::fs::remove_file(temp);
    }
    if copy.exists() {
        // Something else is under the copy's name: replace it.
        return std::fs::rename(temp, copy).map_err(|error| context(copy.display(), error));
    }
    Err(context(copy.display(), os(error)))
}

/// Removes every build's directory under `copies` except `keep`'s, where it
/// can: Windows refuses to delete a running image, so a build that still
/// has a role running keeps its directory. Nothing here fails; what can't
/// be removed stays for a later call.
pub fn prune_helper_copies(copies: &Path, keep: &str) {
    let Ok(entries) = std::fs::read_dir(copies) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_name() == keep || !entry.file_type().is_ok_and(|kind| kind.is_dir()) {
            continue;
        }
        let dir = entry.path();
        // The image first: while it runs, the directory is left whole.
        match std::fs::remove_file(dir.join("brindle-remote.exe")) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(_) => continue,
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}

/// SYSTEM's SID, the one entry beside the user's in the server's DACLs.
pub const SYSTEM_SID: &str = "S-1-5-18";

#[cfg(test)]
mod tests {
    use std::fs::{File, OpenOptions};

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

        prune_helper_copies(&copies, "kept");
        assert_eq!(names(&copies), ["a-file", "kept", "running"]);
        assert!(kept.is_file());
        // The directory in use is left whole, not half emptied.
        assert_eq!(
            names(running.parent().unwrap()),
            ["brindle-remote.exe", "extra.txt"]
        );

        child.kill().unwrap();
        child.wait().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        loop {
            prune_helper_copies(&copies, "kept");
            if !running.parent().unwrap().exists() {
                break;
            }
            assert!(std::time::Instant::now() < deadline, "the copy stayed");
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert_eq!(names(&copies), ["a-file", "kept"]);

        // No directory at all is nothing to prune.
        prune_helper_copies(&dir.join("absent"), "kept");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
