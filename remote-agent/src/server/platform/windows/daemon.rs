//! Starting a server process detached: the `launch` trampoline,
//! `--ready-handle N`, breaking away from the starter's job, stdio to the
//! log, and the one ready byte.
//!
//! The starter runs `brindle-remote launch <role> --ready-handle N` with the
//! write end of a pipe as the inheritable handle `N`, and waits for one byte
//! on the read end. The end of the pipe instead means the role failed before
//! it was listening (the starter's log and the role's say why).
//!
//! - **The trampoline** (`launch`) starts `brindle-remote <role>
//!   --ready-handle N` from its own image and exits at once, like the
//!   process between Linux's two forks. So a role's parent is never a
//!   running process, and killing the starter's process tree (`taskkill
//!   /T`, Task Manager's "End process tree"), which follows parent pids,
//!   doesn't reach the role.
//! - **The role is created** with `DETACHED_PROCESS |
//!   CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW | CREATE_BREAKAWAY_FROM_JOB`:
//!   it has no console, and it is outside the starter's job, so a job that
//!   ends its processes when it closes leaves the role alone. A job that
//!   forbids breakaway fails the create with `ERROR_ACCESS_DENIED`. The role
//!   is then started inside the job, and the trampoline says so in one line
//!   on its stderr, which is its starter's (a core's or a holder's log).
//! - **Handles:** the role inherits only what a
//!   `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` names: the ready pipe's write end,
//!   and the trampoline's stdout and stderr, for what the role says before
//!   it has its log. An inherited handle keeps its value, so `N` names the
//!   pipe all the way down. The trampoline itself is started through std,
//!   which passes on every inheritable handle of the starter; a core or a
//!   holder keeps none besides the pipe's.
//! - **The role** takes `N` out of inheritance ([`ready_from_args`]), and
//!   once it has its place in the server directory it finishes detaching
//!   ([`daemonize`]): stdout and stderr go to its log.

use std::ffi::{OsStr, OsString, c_void};
use std::fs::{File, OpenOptions};
use std::io::{self, Write as _};
use std::marker::PhantomData;
use std::os::windows::ffi::OsStrExt as _;
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, IntoRawHandle as _, OwnedHandle};
use std::os::windows::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use brindle_remote_protocol::win;
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, GENERIC_WRITE, HANDLE, HANDLE_FLAG_INHERIT, HANDLE_FLAGS,
    INVALID_HANDLE_VALUE, SetHandleInformation,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, FILE_FLAGS_AND_ATTRIBUTES,
    FILE_SHARE_NONE, FILE_TYPE_DISK, FILE_TYPE_PIPE, GetFileType, OPEN_EXISTING,
    PIPE_ACCESS_INBOUND,
};
use windows::Win32::System::Console::{
    GetStdHandle, STD_ERROR_HANDLE, STD_HANDLE, STD_OUTPUT_HANDLE, SetConsoleCtrlHandler,
    SetStdHandle,
};
use windows::Win32::System::Pipes::{
    CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows::Win32::System::Threading::{
    CREATE_BREAKAWAY_FROM_JOB, CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW,
    CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DETACHED_PROCESS, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROCESS_CREATION_FLAGS, PROCESS_INFORMATION,
    STARTF_USESTDHANDLES, STARTUPINFOEXW, STARTUPINFOW, UpdateProcThreadAttribute,
};
use windows::core::{HSTRING, PCWSTR, PWSTR};

use super::event_loop::{Source, os_error, win32};
use super::pipe::Pipe;
use crate::conpty::cmdline::command_line;

/// The command that starts a role and exits.
const LAUNCH: &str = "launch";
/// The argument before the ready pipe's handle.
const READY_FLAG: &str = "--ready-handle";
/// Stands for the ready pipe's handle in a role's arguments.
const READY_PLACEHOLDER: &str = "{ready_handle}";
/// What a detached role is created with, plus `CREATE_BREAKAWAY_FROM_JOB`
/// where the job allows it. `CREATE_NO_WINDOW` means nothing beside
/// `DETACHED_PROCESS`, and is there in case that one is ever dropped.
const DETACHED: PROCESS_CREATION_FLAGS = PROCESS_CREATION_FLAGS(
    DETACHED_PROCESS.0
        | CREATE_NEW_PROCESS_GROUP.0
        | CREATE_NO_WINDOW.0
        | CREATE_UNICODE_ENVIRONMENT.0,
);

fn os(error: windows::core::Error) -> io::Error {
    os_error(win32(&error))
}

/// `text` as a NUL-terminated wide string.
fn wide(text: &OsStr) -> Vec<u16> {
    text.encode_wide().chain([0]).collect()
}

/// The handle `value` names, if it is an open pipe in this process.
fn pipe_handle(value: usize) -> Option<HANDLE> {
    let handle = HANDLE(value as *mut c_void);
    // SAFETY: only asks what the value names; anything else is "unknown".
    (unsafe { GetFileType(handle) } == FILE_TYPE_PIPE).then_some(handle)
}

/// Whether processes this one starts with handle inheritance get `handle`.
fn set_inherited(handle: HANDLE, inherited: bool) -> io::Result<()> {
    let flags = if inherited {
        HANDLE_FLAG_INHERIT
    } else {
        HANDLE_FLAGS(0)
    };
    // SAFETY: the caller's open handle.
    unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT.0, flags) }.map_err(os)
}

/// The pipe to write the ready byte to.
pub struct Ready(File);

impl Ready {
    /// Takes ownership of an inherited handle, checking that it is an open
    /// pipe and taking it out of inheritance, so nothing this process
    /// starts holds it.
    fn from_handle(value: usize) -> io::Result<Self> {
        let handle = pipe_handle(value).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{READY_FLAG} {value} is not an open pipe handle"),
            )
        })?;
        set_inherited(handle, false)?;
        // SAFETY: the starter passed this handle on for this process to
        // own, and nothing else here knows it.
        Ok(Self(unsafe { File::from_raw_handle(handle.0) }))
    }

    /// Tells the starter the server is listening, and closes the pipe.
    pub fn signal(mut self) -> io::Result<()> {
        self.0.write_all(b"R")
    }
}

/// The ready pipe a role was started with, from its arguments:
/// `--ready-handle N`, or nothing for a role that stays in the foreground.
/// Nothing else is accepted.
pub fn ready_from_args(role: &str, args: &[String]) -> Result<Option<Ready>, String> {
    parse_ready_handle(role, args)?
        .map(Ready::from_handle)
        .transpose()
        .map_err(|error| error.to_string())
}

fn parse_ready_handle(role: &str, args: &[String]) -> Result<Option<usize>, String> {
    match args {
        [] => Ok(None),
        [flag, handle] if flag == READY_FLAG => handle
            .parse::<usize>()
            .ok()
            .filter(|handle| *handle != 0)
            .map(Some)
            .ok_or_else(|| format!("{role}: {READY_FLAG} needs a handle number, not {handle:?}")),
        _ => Err(format!("usage: brindle-remote {role} [{READY_FLAG} N]")),
    }
}

/// The arguments that start `role` detached with [`start_role`]:
/// `<role> --ready-handle {ready_handle}`.
pub fn role_args(role: &str) -> Vec<String> {
    vec![role.into(), READY_FLAG.into(), READY_PLACEHOLDER.into()]
}

/// The read end of a started role's ready pipe.
pub struct ReadyPipe(Pipe);

impl ReadyPipe {
    /// What an event loop waits on: readable once the role is listening or
    /// has failed.
    pub fn source(&self) -> Source {
        self.0.source()
    }

    /// Whether the role is listening (`true`) or ended before it was
    /// (`false`); `None` while neither is known.
    pub fn read(&self) -> Option<bool> {
        let mut buffer = [0u8; 16];
        match self.0.read(&mut buffer) {
            Ok(read) => Some(read > 0),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => None,
            Err(_) => Some(false),
        }
    }
}

/// A new ready pipe: the end the starter reads from, overlapped for its
/// event loop, and the end a role writes its byte to.
fn ready_pipe() -> io::Result<(OwnedHandle, OwnedHandle)> {
    static NEXT: AtomicU32 = AtomicU32::new(0);

    // The time too, so the name can't be one an earlier process with this
    // pid left behind, held open by a role that never signalled.
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let name = HSTRING::from(format!(
        r"\\.\pipe\brindle-ready-{}-{}-{:x}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed),
        now.as_nanos()
    ));
    let security = win::Security::for_pipe()?;
    // SAFETY: the name and the security attributes outlive the call. One
    // instance, and the first of its name: nobody else serves this pipe.
    let read = unsafe {
        CreateNamedPipeW(
            &name,
            PIPE_ACCESS_INBOUND | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            0,
            // Room for the byte, so a role never waits for its starter.
            16,
            0,
            Some(security.attributes()),
        )
    };
    if read == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: a new handle nothing else owns.
    let read = unsafe { OwnedHandle::from_raw_handle(read.0) };
    // The one client the pipe takes, opened for synchronous writes.
    // SAFETY: the name outlives the call.
    let write = unsafe {
        CreateFileW(
            &name,
            GENERIC_WRITE.0,
            FILE_SHARE_NONE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
    }
    .map_err(os)?;
    // SAFETY: as above.
    Ok((read, unsafe { OwnedHandle::from_raw_handle(write.0) }))
}

/// A role started by [`start_role`].
pub struct Started {
    /// The trampoline, which exits once it has started the role.
    pub child: Child,
    pub ready: ReadyPipe,
    /// What was run, for the log.
    pub program: OsString,
}

/// The `brindle-remote` that roles are started from: its image, and the
/// arguments that come before a command's own. There are none; the tests'
/// stand-in for the binary has some.
struct Program {
    image: PathBuf,
    prefix: Vec<String>,
}

impl Program {
    /// This process's own image, which for a server role is its per-build
    /// copy.
    fn own() -> io::Result<Self> {
        Ok(Self {
            image: win::own_image_path()?,
            prefix: Vec::new(),
        })
    }
}

/// Starts a role of this process's own binary, detached: `args` (see
/// [`role_args`]; `{ready_handle}` in them stands for the ready pipe's
/// handle) with `env` added to this process's environment.
///
/// The binary is run as `launch <args>`, the trampoline that starts the
/// role outside this process's job and its process tree. Whether the job
/// lets the role go is known only to the trampoline, which says so on its
/// stderr (this process's) instead of through `_unscoped`.
pub fn start_role(
    _role: &str,
    args: &[String],
    env: &[(&str, &OsStr)],
    _unscoped: impl FnOnce(&str),
) -> io::Result<Started> {
    start(&Program::own()?, args, env)
}

fn start(program: &Program, args: &[String], env: &[(&str, &OsStr)]) -> io::Result<Started> {
    let (read, write) = ready_pipe()?;
    let handle = HANDLE(write.as_raw_handle());
    let number = (handle.0 as usize).to_string();
    set_inherited(handle, true)?;
    let child = Command::new(&program.image)
        .args(&program.prefix)
        .arg(LAUNCH)
        .args(
            args.iter()
                .map(|arg| arg.replace(READY_PLACEHOLDER, &number)),
        )
        .envs(env.iter().copied())
        .stdin(Stdio::null())
        // The helper is a console program: without this, a starter that has
        // no console would open one for it.
        .creation_flags(DETACHED_PROCESS.0)
        .spawn()?;
    drop(write);
    Ok(Started {
        child,
        ready: ReadyPipe(Pipe::of(Some(read), None)),
        program: program.image.clone().into_os_string(),
    })
}

/// Runs `brindle-remote launch <role> [--ready-handle N]`, the trampoline:
/// starts `brindle-remote <role> [--ready-handle N]` from this process's
/// image, detached (see the module), and returns without waiting for it.
pub fn launch(args: &[String]) -> Result<(), String> {
    let program = Program::own().map_err(|error| error.to_string())?;
    launch_from(&program, args)
}

fn launch_from(program: &Program, args: &[String]) -> Result<(), String> {
    let Some(role) = args.first() else {
        return Err(format!(
            "usage: brindle-remote {LAUNCH} <role> [{READY_FLAG} N]"
        ));
    };
    let ready = inherited_ready(args)?;
    let detached =
        detach(program, args, ready).map_err(|error| format!("can't start {role}: {error}"))?;
    if !detached.broke_away {
        // Not `eprintln!`, which panics when stderr can't be written to.
        let _ = writeln!(
            io::stderr(),
            "{} {LAUNCH}[{}]: the job this process is in forbids breakaway, so {role} (pid {}) \
             runs inside it and ends when the job does",
            crate::server::daemon::timestamp(),
            std::process::id(),
            detached.pid
        );
    }
    Ok(())
}

/// The ready pipe's handle in a role's arguments, which this process must
/// have inherited.
fn inherited_ready(args: &[String]) -> Result<Option<HANDLE>, String> {
    let Some(flag) = args.iter().position(|arg| arg == READY_FLAG) else {
        return Ok(None);
    };
    args.get(flag + 1)
        .and_then(|value| value.parse::<usize>().ok())
        .and_then(pipe_handle)
        .map(Some)
        .ok_or_else(|| format!("{LAUNCH}: {READY_FLAG} needs the handle of an inherited pipe"))
}

/// This process's stdout or stderr, when a detached role can write to it:
/// a file or a pipe, not a console (the role has none) or the null device.
fn passable(id: STD_HANDLE) -> Option<HANDLE> {
    // SAFETY: plain calls; the handle is this process's, open or not.
    let handle = unsafe { GetStdHandle(id) }.ok()?;
    let kind = unsafe { GetFileType(handle) };
    (kind == FILE_TYPE_DISK || kind == FILE_TYPE_PIPE).then_some(handle)
}

/// A process attribute list that names the only handles a new process
/// inherits.
struct HandleList<'a> {
    list: LPPROC_THREAD_ATTRIBUTE_LIST,
    _memory: Vec<usize>,
    /// The list points at the handles.
    _handles: PhantomData<&'a [HANDLE]>,
}

impl<'a> HandleList<'a> {
    fn new(handles: &'a [HANDLE]) -> io::Result<Self> {
        let mut size = 0;
        // SAFETY: the first call only reports the size a list of one
        // attribute needs, and fails doing so.
        unsafe { InitializeProcThreadAttributeList(None, 1, None, &mut size) }.ok();
        let mut memory = vec![0usize; size.div_ceil(size_of::<usize>())];
        let list = LPPROC_THREAD_ATTRIBUTE_LIST(memory.as_mut_ptr().cast());
        // SAFETY: `memory` has the reported size, and stays where it is and
        // alive as long as the list.
        unsafe { InitializeProcThreadAttributeList(Some(list), 1, None, &mut size) }.map_err(os)?;
        let list = Self {
            list,
            _memory: memory,
            _handles: PhantomData,
        };
        // SAFETY: the list keeps the pointer, and `handles` outlive it.
        unsafe {
            UpdateProcThreadAttribute(
                list.list,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                Some(handles.as_ptr().cast()),
                size_of_val(handles),
                None,
                None,
            )
        }
        .map_err(os)?;
        Ok(list)
    }
}

impl Drop for HandleList<'_> {
    fn drop(&mut self) {
        // SAFETY: initialised in `new`, deleted once.
        unsafe { DeleteProcThreadAttributeList(self.list) };
    }
}

/// A role the trampoline started.
struct Detached {
    pid: u32,
    /// It left this process's job, and the jobs around that one up to the
    /// first that forbids breakaway. `false` when this process's own job
    /// forbids it: the role is then in every job this process is in.
    broke_away: bool,
}

/// Creates the process `<image> <prefix> <args>`, detached, with this
/// process's environment and directory. It inherits `ready` and this
/// process's stdout and stderr, and nothing else.
fn detach(program: &Program, args: &[String], ready: Option<HANDLE>) -> io::Result<Detached> {
    let stdout = passable(STD_OUTPUT_HANDLE);
    let stderr = passable(STD_ERROR_HANDLE);
    let mut handles: Vec<HANDLE> = Vec::new();
    for handle in [ready, stdout, stderr].into_iter().flatten() {
        // A handle is listed once.
        if !handles.contains(&handle) {
            set_inherited(handle, true)?;
            handles.push(handle);
        }
    }
    // An empty list can't be given; without one, nothing is inherited.
    let list = match handles.as_slice() {
        [] => None,
        handles => Some(HandleList::new(handles)?),
    };

    let mut startup = STARTUPINFOEXW::default();
    let mut flags = DETACHED;
    startup.StartupInfo.cb = size_of::<STARTUPINFOW>() as u32;
    // The role has no stdin, and no stdout or stderr that isn't passed on.
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdOutput = stdout.unwrap_or_default();
    startup.StartupInfo.hStdError = stderr.unwrap_or_default();
    if let Some(list) = &list {
        startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
        startup.lpAttributeList = list.list;
        flags |= EXTENDED_STARTUPINFO_PRESENT;
    }

    let image = wide(program.image.as_os_str());
    let arguments: Vec<String> = program.prefix.iter().chain(args).cloned().collect();
    // The image is named apart from the line, so the line's first token is
    // only what the role sees as its own name.
    let line = command_line(&program.image.to_string_lossy(), &arguments);
    let create = |flags: PROCESS_CREATION_FLAGS| {
        // A fresh copy each time: the call may write to the line.
        let mut line = wide(OsStr::new(&line));
        let mut info = PROCESS_INFORMATION::default();
        // SAFETY: every pointer is valid for the call, the line is
        // writable, and the startup info is the extended one exactly when
        // the flags say so.
        unsafe {
            CreateProcessW(
                PCWSTR(image.as_ptr()),
                Some(PWSTR(line.as_mut_ptr())),
                None,
                None,
                list.is_some(),
                flags,
                None,
                PCWSTR::null(),
                &startup.StartupInfo,
                &mut info,
            )
        }
        .map(|()| info)
    };
    let (info, broke_away) = match create(flags | CREATE_BREAKAWAY_FROM_JOB) {
        Ok(info) => (info, true),
        // This process's job doesn't let its processes go.
        Err(error) if error.code() == ERROR_ACCESS_DENIED.to_hresult() => {
            (create(flags).map_err(os)?, false)
        }
        Err(error) => return Err(os(error)),
    };
    // SAFETY: both handles are new and this process's own.
    unsafe {
        CloseHandle(info.hThread).ok();
        CloseHandle(info.hProcess).ok();
    }
    Ok(Detached {
        pid: info.dwProcessId,
        broke_away,
    })
}

/// Opens (appending) a server log. The server directory's protection is the
/// log's.
pub fn open_log(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

/// Finishes detaching this process, which the trampoline started outside
/// its starter's console, job and process tree: points stdout and stderr at
/// `log` and closes the starter's, takes Ctrl+C again, and moves to the
/// image's directory.
pub fn daemonize(log: &File) -> io::Result<()> {
    // Both copies first, so a failure changes nothing.
    let (stdout, stderr) = (log.try_clone()?, log.try_clone()?);
    let mut inherited: Vec<HANDLE> = Vec::new();
    for (id, file) in [(STD_OUTPUT_HANDLE, stdout), (STD_ERROR_HANDLE, stderr)] {
        // SAFETY: a plain call; the handle is this process's, open or not.
        let old = unsafe { GetStdHandle(id) }.ok();
        let new = HANDLE(file.into_raw_handle());
        // SAFETY: `new` is open, and from here on it is the standard
        // handle's to keep.
        if let Err(error) = unsafe { SetStdHandle(id, new) } {
            // SAFETY: nothing took the handle.
            unsafe { CloseHandle(new) }.ok();
            return Err(os(error));
        }
        inherited.extend(old.filter(|old| !inherited.contains(old)));
    }
    for handle in inherited {
        // The starter's stdout and stderr (a pipe it reads to its end, or
        // its log), which this process got to say why it couldn't start.
        // SAFETY: std looks a standard handle up each time it writes, so
        // nothing uses these any more.
        unsafe { CloseHandle(handle) }.ok();
    }
    // A new process group ignores Ctrl+C, and the programs a holder starts
    // would inherit that.
    // SAFETY: no handler is passed; this only clears the process's flag.
    unsafe { SetConsoleCtrlHandler(None, false) }.ok();
    // Leave the starter's directory, which a server running for days would
    // keep in use. The image's own directory is in use anyway.
    if let Some(dir) = win::own_image_path().ok().as_deref().and_then(Path::parent) {
        let _ = std::env::set_current_dir(dir);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::thread;
    use std::time::{Duration, Instant};

    use windows::Win32::Foundation::{GetHandleInformation, WAIT_OBJECT_0};
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, IsProcessInJob, JOB_OBJECT_LIMIT,
        JOB_OBJECT_LIMIT_BREAKAWAY_OK, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };
    use windows::Win32::System::Threading::{
        OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
        TerminateProcess, WaitForSingleObject,
    };
    use windows::core::BOOL;

    use super::*;
    use crate::server::dir::tests::TempDir;
    use crate::server::event_loop::{EventLoop, Interest, Loop};

    /// Set in the children of these tests, which are this test binary run
    /// again as a starter, a trampoline or a role: the directory for what
    /// they write.
    const CHILD_DIR: &str = "BRINDLE_TEST_DAEMON_DIR";
    /// How long a starter waits for the ready byte (design.md).
    const READY_LIMIT: Duration = Duration::from_secs(2);
    /// For what should happen soon.
    const TIMEOUT: Duration = Duration::from_secs(30);
    /// How long a child stays when no test kills it.
    const CHILD_LIFE: Duration = Duration::from_secs(60);
    /// What the trampoline says about a job that keeps the role.
    const KEPT_IN_JOB: &str = "forbids breakaway";

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|arg| arg.to_string()).collect()
    }

    /// This test binary standing in for `brindle-remote`: with these
    /// arguments first it runs only [`child`], which takes what follows
    /// `--` as the helper takes its arguments.
    fn stand_in() -> Program {
        let module = module_path!();
        let module = module.split_once("::").map_or(module, |(_, path)| path);
        Program {
            image: std::env::current_exe().unwrap(),
            prefix: args(&[&format!("{module}::child"), "--exact", "--nocapture", "--"]),
        }
    }

    /// The children's half of the tests below; run as a test of its own it
    /// does nothing. As a child it is the helper's `main` for the commands
    /// the tests use: `launch`, a starter, and the roles.
    #[test]
    fn child() {
        let Some(dir) = std::env::var_os(CHILD_DIR).map(PathBuf::from) else {
            return;
        };
        let arguments: Vec<String> = std::env::args()
            .skip_while(|arg| arg != "--")
            .skip(1)
            .collect();
        let (command, rest) = arguments.split_first().expect("a command after --");
        match command.as_str() {
            LAUNCH => launch_from(&stand_in(), rest).unwrap(),
            "starter" => starter(&dir, &rest[0], &rest[1..]),
            role => run_role(&dir, role, rest),
        }
    }

    /// A role, detached like `serve` and `hold`: its log is `<role>.log`,
    /// where it writes its pid. `ready` then signals and stays, `silent`
    /// stays without signalling, and `gone` ends without signalling.
    /// `chain` first starts a `ready` of its own, as a core starts a
    /// holder, and writes to its log whether that one came up.
    fn run_role(dir: &Path, role: &str, arguments: &[String]) {
        let ready = ready_from_args(role, arguments)
            .unwrap()
            .expect("a ready handle");
        let handle = HANDLE(arguments[1].parse::<usize>().unwrap() as *mut c_void);
        let mut flags = 0;
        // SAFETY: the handle is open, and `flags` is writable.
        unsafe { GetHandleInformation(handle, &mut flags) }.unwrap();
        let inheritable = flags & HANDLE_FLAG_INHERIT.0 != 0;
        let log = open_log(&dir.join(format!("{role}.log"))).unwrap();
        daemonize(&log).unwrap();
        // A whole line in one write, so a reader never sees half of one.
        let say = |line: String| io::stderr().write_all(line.as_bytes()).unwrap();
        let line = format!("{role} on stdout, ready pipe inheritable: {inheritable}\n");
        io::stdout().write_all(line.as_bytes()).unwrap();
        io::stdout().flush().unwrap();
        say(format!("{role} pid {}\n", std::process::id()));
        match role {
            "ready" => ready.signal().unwrap(),
            "chain" => {
                let started = start(&stand_in(), &role_args("ready"), &[]).unwrap();
                let mut ev = watch(&started.ready);
                let listening = wait_ready(&mut ev, &started.ready, TIMEOUT);
                say(format!("chain started ready: {listening:?}\n"));
                ready.signal().unwrap();
            }
            "silent" => {
                thread::sleep(CHILD_LIFE);
                drop(ready);
                return;
            }
            "gone" => return,
            other => panic!("no role {other}"),
        }
        thread::sleep(CHILD_LIFE);
    }

    /// A starter in a process of its own, as a core is for its holders.
    /// Once the file `go` is there it starts a role with `arguments`, waits
    /// up to `limit` milliseconds for its ready byte, then for the pipe to
    /// end and for the trampoline to exit, writes what happened to the file
    /// `started`, and stays.
    fn starter(dir: &Path, limit: &str, arguments: &[String]) {
        let limit = Duration::from_millis(limit.parse().unwrap());
        eventually("the test to say go", || dir.join("go").exists());
        let mut started = start(&stand_in(), arguments, &[]).unwrap();
        let mut ev = watch(&started.ready);
        let mut outcome = format!("{:?}", wait_ready(&mut ev, &started.ready, limit));
        if outcome == "Some(true)" {
            // Nothing holds the pipe once the role has signalled.
            let end = wait_ready(&mut ev, &started.ready, TIMEOUT);
            outcome += &format!(" then {end:?}");
        }
        let status = started.child.wait().unwrap();
        outcome += &format!(", the trampoline exited with {:?}", status.code());
        fs::write(dir.join("started.tmp"), outcome).unwrap();
        fs::rename(dir.join("started.tmp"), dir.join("started")).unwrap();
        thread::sleep(CHILD_LIFE);
    }

    /// What a starter writes when its role is listening.
    const LISTENING: &str = "Some(true) then Some(false), the trampoline exited with Some(0)";

    fn eventually(what: &str, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + TIMEOUT;
        while !done() {
            assert!(Instant::now() < deadline, "waited too long for {what}");
            thread::sleep(Duration::from_millis(20));
        }
    }

    /// A loop that waits on `ready`, as a core's or a holder's does.
    fn watch(ready: &ReadyPipe) -> Loop {
        let mut ev = Loop::new().unwrap();
        ev.register(&ready.source(), 1, Interest::READABLE).unwrap();
        ev
    }

    /// Waits up to `limit` for the role to be listening (`Some(true)`) or
    /// gone (`Some(false)`).
    fn wait_ready(ev: &mut Loop, ready: &ReadyPipe, limit: Duration) -> Option<bool> {
        let deadline = Instant::now() + limit;
        let mut events = Vec::new();
        loop {
            if let Some(listening) = ready.read() {
                return Some(listening);
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return None;
            }
            ev.wait(&mut events, Some(remaining)).unwrap();
        }
    }

    /// A role's process, killed when the test is done with it.
    struct RoleProcess(OwnedHandle);

    impl RoleProcess {
        /// The role that wrote its pid to `<role>.log` in `dir`.
        fn of(dir: &Path, role: &str) -> Self {
            let log = dir.join(format!("{role}.log"));
            let prefix = format!("{role} pid ");
            let mut pid = None;
            eventually("the role to write its pid", || {
                pid = fs::read_to_string(&log).ok().and_then(|text| {
                    text.split_inclusive('\n')
                        .filter(|line| line.ends_with('\n'))
                        .find_map(|line| line.strip_prefix(&prefix)?.trim().parse::<u32>().ok())
                });
                pid.is_some()
            });
            let access =
                PROCESS_TERMINATE | PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION;
            // SAFETY: plain arguments; the handle is owned below.
            let process =
                unsafe { OpenProcess(access, false, pid.unwrap()) }.expect("the role is running");
            // SAFETY: a new handle nothing else owns.
            Self(unsafe { OwnedHandle::from_raw_handle(process.0) })
        }

        fn handle(&self) -> HANDLE {
            HANDLE(self.0.as_raw_handle())
        }

        fn ends_within(&self, limit: Duration) -> bool {
            // SAFETY: the handle is open.
            unsafe { WaitForSingleObject(self.handle(), limit.as_millis() as u32) == WAIT_OBJECT_0 }
        }

        fn is_in(&self, job: &OwnedHandle) -> bool {
            let mut inside = BOOL::default();
            // SAFETY: both handles are open, and `inside` is writable.
            unsafe {
                IsProcessInJob(
                    self.handle(),
                    Some(HANDLE(job.as_raw_handle())),
                    &mut inside,
                )
            }
            .unwrap();
            inside.as_bool()
        }
    }

    impl Drop for RoleProcess {
        fn drop(&mut self) {
            // SAFETY: the handle is open. The wait is for the role's files
            // to close, so its directory can be removed.
            unsafe {
                TerminateProcess(self.handle(), 1).ok();
                WaitForSingleObject(self.handle(), TIMEOUT.as_millis() as u32);
            }
        }
    }

    /// A job that ends its processes when its handle closes, with `more`.
    fn job(more: JOB_OBJECT_LIMIT) -> OwnedHandle {
        // SAFETY: no attributes and no name; the handle is owned below.
        let job = unsafe { CreateJobObjectW(None, PCWSTR::null()) }.unwrap();
        // SAFETY: a new handle nothing else owns.
        let job = unsafe { OwnedHandle::from_raw_handle(job.0) };
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | more;
        // SAFETY: the information is the class's structure, of its size.
        unsafe {
            SetInformationJobObject(
                HANDLE(job.as_raw_handle()),
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                size_of_val(&limits) as u32,
            )
        }
        .unwrap();
        job
    }

    /// A starter in a process of its own ([`starter`]), killed when the test
    /// is done with it.
    struct StarterProcess {
        child: Child,
        dir: PathBuf,
    }

    impl StarterProcess {
        /// Runs a starter that starts a role with `arguments` and gives it
        /// `limit` to be ready, in `job` if any. What it and its trampoline
        /// say goes to `starter.err` in `dir`.
        fn spawn(
            dir: &Path,
            arguments: &[String],
            limit: Duration,
            job: Option<&OwnedHandle>,
        ) -> Self {
            let program = stand_in();
            let child = Command::new(&program.image)
                .args(&program.prefix)
                .args(["starter", &limit.as_millis().to_string()])
                .args(arguments)
                .env(CHILD_DIR, dir)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(File::create(dir.join("starter.err")).unwrap())
                .spawn()
                .unwrap();
            let starter = Self {
                child,
                dir: dir.to_path_buf(),
            };
            if let Some(job) = job {
                // SAFETY: both handles are open.
                unsafe {
                    AssignProcessToJobObject(
                        HANDLE(job.as_raw_handle()),
                        HANDLE(starter.child.as_raw_handle()),
                    )
                }
                .unwrap();
            }
            // Only now does it start the role: in the job, if there is one.
            fs::write(dir.join("go"), b"").unwrap();
            starter
        }

        /// What the starter and its trampoline said on stderr.
        fn said(&self) -> String {
            fs::read_to_string(self.dir.join("starter.err")).unwrap_or_default()
        }

        /// What happened to the role, once the starter has written it.
        fn outcome(&mut self) -> String {
            let started = self.dir.join("started");
            eventually("the starter", || {
                let ended = self.child.try_wait().unwrap();
                assert!(
                    ended.is_none(),
                    "the starter ended: {ended:?}\n{}",
                    self.said()
                );
                started.exists()
            });
            fs::read_to_string(started).unwrap()
        }

        /// How many times the trampoline said the job kept the role.
        fn kept_in_job_lines(&self) -> usize {
            self.said()
                .lines()
                .filter(|line| line.contains(KEPT_IN_JOB))
                .count()
        }

        /// Waits for the starter to end, which something else brings about.
        fn ended(&mut self) {
            self.child.wait().unwrap();
        }
    }

    impl Drop for StarterProcess {
        fn drop(&mut self) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    #[test]
    fn ready_handle_arguments() {
        assert_eq!(parse_ready_handle("hold", &[]), Ok(None));
        assert_eq!(
            parse_ready_handle("hold", &args(&["--ready-handle", "1234"])),
            Ok(Some(1234))
        );
        assert!(parse_ready_handle("hold", &args(&["--ready-handle", "0"])).is_err());
        assert!(parse_ready_handle("hold", &args(&["--ready-handle", "-4"])).is_err());
        assert!(parse_ready_handle("hold", &args(&["--ready-handle"])).is_err());
        assert!(parse_ready_handle("hold", &args(&["--ready-fd", "3"])).is_err());
        assert_eq!(
            role_args("hold"),
            args(&["hold", "--ready-handle", "{ready_handle}"])
        );
    }

    #[test]
    fn the_trampoline_passes_on_only_a_pipe_it_holds() {
        assert_eq!(inherited_ready(&args(&["hold"])), Ok(None));
        let (_read, write) = ready_pipe().unwrap();
        let number = (write.as_raw_handle() as usize).to_string();
        assert_eq!(
            inherited_ready(&args(&["hold", "--ready-handle", &number])),
            Ok(Some(HANDLE(write.as_raw_handle())))
        );
        // A file is no ready pipe, and neither is a number that names
        // nothing.
        let temp = TempDir::new("daemon-handle");
        let file = File::create(temp.0.join("file")).unwrap();
        let file = (file.as_raw_handle() as usize).to_string();
        for bad in [file.as_str(), "3", "x"] {
            let error = inherited_ready(&args(&["hold", "--ready-handle", bad])).unwrap_err();
            assert!(error.contains("inherited pipe"), "{error}");
        }
        assert!(inherited_ready(&args(&["hold", "--ready-handle"])).is_err());
        let error = launch_from(&stand_in(), &[]).unwrap_err();
        assert!(error.starts_with("usage: brindle-remote launch"), "{error}");
    }

    #[test]
    fn the_ready_byte_reaches_the_pipe() {
        let (read, write) = ready_pipe().unwrap();
        let read = ReadyPipe(Pipe::of(Some(read), None));
        let mut ev = watch(&read);
        assert_eq!(read.read(), None);
        let ready = Ready::from_handle(write.into_raw_handle() as usize).unwrap();
        ready.signal().unwrap();
        assert_eq!(wait_ready(&mut ev, &read, TIMEOUT), Some(true));
        // The write end was closed with the Ready.
        assert_eq!(wait_ready(&mut ev, &read, TIMEOUT), Some(false));
        assert!(Ready::from_handle(3).is_err());
    }

    #[test]
    fn a_role_says_it_is_listening_and_outlives_its_trampoline() {
        let dir = TempDir::new("daemon-ready");
        let mut starter = StarterProcess::spawn(&dir.0, &role_args("ready"), TIMEOUT, None);
        // The byte came, then the pipe's end (nothing else held it), and
        // the trampoline had gone.
        assert_eq!(starter.outcome(), LISTENING, "{}", starter.said());
        let role = RoleProcess::of(&dir.0, "ready");
        assert!(!role.ends_within(Duration::from_millis(500)));
        // Its stdout and stderr are its log, and it took the ready pipe out
        // of inheritance.
        let log = fs::read_to_string(dir.0.join("ready.log")).unwrap();
        assert!(
            log.contains("ready on stdout, ready pipe inheritable: false\n"),
            "{log}"
        );
    }

    #[test]
    fn a_role_that_never_signals_fails_the_starters_wait() {
        let dir = TempDir::new("daemon-silent");
        let begin = Instant::now();
        let mut starter = StarterProcess::spawn(&dir.0, &role_args("silent"), READY_LIMIT, None);
        let _role = RoleProcess::of(&dir.0, "silent");
        assert_eq!(
            starter.outcome(),
            "None, the trampoline exited with Some(0)",
            "{}",
            starter.said()
        );
        assert!(begin.elapsed() >= READY_LIMIT);
    }

    #[test]
    fn a_role_that_ends_before_listening_ends_the_pipe() {
        let dir = TempDir::new("daemon-gone");
        let mut starter = StarterProcess::spawn(&dir.0, &role_args("gone"), TIMEOUT, None);
        assert_eq!(
            starter.outcome(),
            "Some(false), the trampoline exited with Some(0)",
            "{}",
            starter.said()
        );
    }

    #[test]
    fn a_trampoline_that_starts_nothing_ends_the_pipe() {
        let dir = TempDir::new("daemon-unstarted");
        // No handle of the trampoline's: it refuses, and exits with an error.
        let arguments = args(&["ready", "--ready-handle", "3"]);
        let mut starter = StarterProcess::spawn(&dir.0, &arguments, TIMEOUT, None);
        let outcome = starter.outcome();
        assert!(outcome.starts_with("Some(false), "), "{outcome}");
        assert!(!outcome.ends_with("Some(0)"), "{outcome}");
        assert!(
            starter.said().contains("inherited pipe"),
            "{}",
            starter.said()
        );
        assert!(!dir.0.join("ready.log").exists());
    }

    #[test]
    fn a_job_that_forbids_breakaway_keeps_the_role_and_the_trampoline_says_so() {
        let dir = TempDir::new("daemon-kept");
        let job = job(JOB_OBJECT_LIMIT(0));
        let mut starter = StarterProcess::spawn(&dir.0, &role_args("ready"), TIMEOUT, Some(&job));
        assert_eq!(starter.outcome(), LISTENING, "{}", starter.said());
        let role = RoleProcess::of(&dir.0, "ready");
        assert!(role.is_in(&job));
        assert_eq!(starter.kept_in_job_lines(), 1, "{}", starter.said());
        // Which is what the line warns of: the job ends the role too.
        drop(job);
        assert!(role.ends_within(TIMEOUT));
        starter.ended();
    }

    #[test]
    fn a_detached_role_starts_another_and_its_log_gets_the_trampolines_line() {
        let dir = TempDir::new("daemon-chain");
        // In a job that keeps them, so that each trampoline has its line.
        let job = job(JOB_OBJECT_LIMIT(0));
        let mut starter = StarterProcess::spawn(&dir.0, &role_args("chain"), TIMEOUT, Some(&job));
        assert_eq!(starter.outcome(), LISTENING, "{}", starter.said());
        // The first trampoline wrote to the starter's stderr, and the
        // second to the first role's, which was its log by then.
        assert_eq!(starter.kept_in_job_lines(), 1, "{}", starter.said());
        let log = fs::read_to_string(dir.0.join("chain.log")).unwrap();
        assert!(log.contains("chain started ready: Some(true)\n"), "{log}");
        let kept: Vec<&str> = log
            .lines()
            .filter(|line| line.contains(KEPT_IN_JOB))
            .collect();
        assert_eq!(kept.len(), 1, "{log}");
        assert!(kept[0].contains("so ready (pid "), "{log}");
        // The second role took its own log.
        let role = RoleProcess::of(&dir.0, "ready");
        assert!(role.is_in(&job));
        drop(job);
        assert!(role.ends_within(TIMEOUT));
        starter.ended();
    }

    #[test]
    fn a_role_leaves_a_job_that_allows_breakaway_and_outlives_it() {
        let dir = TempDir::new("daemon-left");
        let job = job(JOB_OBJECT_LIMIT_BREAKAWAY_OK);
        let mut starter = StarterProcess::spawn(&dir.0, &role_args("ready"), TIMEOUT, Some(&job));
        assert_eq!(starter.outcome(), LISTENING, "{}", starter.said());
        let role = RoleProcess::of(&dir.0, "ready");
        assert!(!role.is_in(&job));
        assert_eq!(starter.kept_in_job_lines(), 0, "{}", starter.said());
        // Closing the job ends the starter, and only the starter.
        drop(job);
        starter.ended();
        assert!(!role.ends_within(Duration::from_secs(1)));
    }

    #[test]
    fn killing_the_starters_process_tree_leaves_the_role() {
        let dir = TempDir::new("daemon-tree");
        let mut starter = StarterProcess::spawn(&dir.0, &role_args("ready"), TIMEOUT, None);
        assert_eq!(starter.outcome(), LISTENING, "{}", starter.said());
        let role = RoleProcess::of(&dir.0, "ready");
        let kill = Command::new("taskkill")
            .args(["/T", "/F", "/PID", &starter.child.id().to_string()])
            .output()
            .unwrap();
        assert!(kill.status.success(), "{kill:?}");
        starter.ended();
        assert!(!role.ends_within(Duration::from_secs(1)));
    }
}
