//! A session's pseudoconsole and program for the holder: ConPTY
//! ([`crate::conpty`]) over a pair of named pipes whose holder ends are
//! overlapped and reported by the holder's loop.
//!
//! - **Pipes:** each session has two one-way pipes, one for the program's
//!   input and one for its output, named
//!   `brindle_remote_protocol::server_dir::pty_pipe` with two numbers in a
//!   row. Each is the name's first and only instance, with the server
//!   pipes' security, and takes no remote client. The holder's ends are
//!   overlapped and make one [`Source`]; the console host gets plain
//!   (synchronous) ends, which is what ConPTY reads and writes.
//! - **The host** is chosen per session: the ConPTY built into Windows when
//!   the session's environment has `BRINDLE_CONPTY=inbox`, else the
//!   packaged one. The holder's own environment doesn't count.
//! - **The environment** is the session's, complete, as the client sent it.
//! - **Exit:** [`Program::watch`] has the loop woken when the program
//!   exits. [`Program::try_wait`] then closes the pseudoconsole, which
//!   releases and closes it on a short-lived thread
//!   ([`crate::conpty::Pseudoconsole`]'s drop), while the loop goes on
//!   reading: the host writes what it still holds and exits, and only then
//!   does [`Pty::read`] report the end. So the end of output follows the
//!   last byte, and the holder reports `EXITED` after it.
//! - **Hangup:** [`Group::hang_up`], like dropping the [`Pty`], closes the
//!   pseudoconsole in the same way, never waiting for it. The console host
//!   then ends the programs attached to the console, as closing a console
//!   window does. The close first waits, on its thread, for the program to
//!   be on the console: one that reaches the host sooner (a session hung
//!   up as it starts) reaches neither the program nor what it starts. The
//!   host takes the end of its input as the close too, so a copy of the
//!   holder's end stays open until then. A program another one starts just
//!   as the close goes out can still miss it. After the grace the holder
//!   calls [`Group::kill_remaining`], which terminates the session's root
//!   process if it is still running. A program that left the console (a GUI
//!   program started from the shell) keeps running: no job object holds a
//!   session's processes.
//! - **The console window's owner:** [`reparent`] has the packaged host
//!   make a window the owner of the console window, the one programs get
//!   from `GetConsoleWindow`, so that their dialogs belong to that window.
//!   The ConPTY built into Windows can't, which is logged once. The owner
//!   is never cleared: when its window is destroyed the console window
//!   stays, with no owner, until the next one is set.

use std::ffi::OsString;
use std::io;
use std::os::windows::io::{AsHandle, AsRawHandle, BorrowedHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use brindle_remote_protocol::{server_dir, win};
use windows::Win32::Foundation::{
    FILETIME, GENERIC_READ, GENERIC_WRITE, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, FILE_FLAGS_AND_ATTRIBUTES,
    FILE_SHARE_NONE, OPEN_EXISTING, PIPE_ACCESS_INBOUND, PIPE_ACCESS_OUTBOUND,
    SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT,
};
use windows::Win32::System::Pipes::{
    CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows::Win32::System::Threading::{
    GetProcessTimes, INFINITE, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
    TerminateProcess, WaitForSingleObject,
};
use windows::core::PCWSTR;

use super::event_loop::{ExitWatch, Source, Waker, os_error, win32};
use super::pipe::Pipe;
use crate::conpty::cmdline::launch_line;
use crate::conpty::{Process, Pseudoconsole};
use crate::pty::PtyCommand;

/// Each pipe's own buffers.
const PIPE_BUFFER: u32 = 64 * 1024;

/// What the session's environment sets `BRINDLE_CONPTY` to for the ConPTY
/// built into Windows.
const INBOX: &str = "inbox";

/// The exit code of a root process the holder terminated.
const KILLED: u32 = 1;

/// A session's pseudoconsole, shared by its [`Pty`] and its [`Program`]:
/// whichever is done with it first (the program exited, or the session was
/// hung up) takes it out, which closes it.
type Console = Arc<Mutex<Option<Pseudoconsole>>>;

fn console(shared: &Console) -> MutexGuard<'_, Option<Pseudoconsole>> {
    shared.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A session's pseudoconsole: its input and output, which never block.
/// Dropping it closes the pseudoconsole.
pub struct Pty {
    /// The holder's ends of the input and the output pipe.
    pipe: Pipe,
    console: Console,
    /// Whether the packaged ConPTY made the pseudoconsole.
    #[cfg(test)]
    packaged: bool,
}

impl Pty {
    /// What an event loop waits on: readable while the program's output is
    /// waiting, writable while the pseudoconsole takes input.
    pub fn source(&self) -> Source {
        self.pipe.source()
    }

    /// Reads the program's output. `Ok(0)` means there will be no more
    /// (the console host has exited); `WouldBlock` that none is waiting.
    pub fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.pipe.read(buffer)
    }

    /// Writes input for the program, as much as is taken; `WouldBlock` when
    /// none is.
    pub fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.pipe.write(bytes)
    }

    /// Resizes the console to `cols` by `rows` cells. A console has no
    /// pixel size, and one whose program has exited no size to change.
    pub fn resize(&self, cols: u16, rows: u16, _cell_w: u16, _cell_h: u16) -> io::Result<()> {
        match console(&self.console).as_ref() {
            Some(console) => console.resize(cols, rows),
            None => Ok(()),
        }
    }
}

impl Drop for Pty {
    fn drop(&mut self) {
        // Closing never blocks (see `Pseudoconsole`'s drop). The pipes
        // close after it, so a host that is still writing stops.
        console(&self.console).take();
    }
}

/// A session's program: the process started on the pseudoconsole.
pub struct Program {
    process: Process,
    /// When it started, which tells it from a later process with its pid.
    started: u64,
    console: Console,
    /// Wakes the loop when the process exits.
    watch: Option<ExitWatch>,
}

/// How a program ended.
pub struct Exit {
    /// Its exit code; `None` when a signal killed it (never on Windows).
    pub code: Option<i32>,
    /// The signal that killed it (never on Windows).
    pub signal: Option<i32>,
}

impl Program {
    pub fn pid(&self) -> u32 {
        self.process.pid()
    }

    /// The processes to hang up and kill with this program.
    pub fn group(&self) -> Group {
        Group {
            pid: self.process.pid(),
            started: self.started,
            console: Arc::clone(&self.console),
        }
    }

    /// Has `waker`'s loop woken when the program exits, for its owner to
    /// call [`Program::try_wait`].
    pub fn watch(&mut self, waker: &Waker) -> io::Result<()> {
        self.watch = Some(waker.wake_on_exit(self.process.handle())?);
        Ok(())
    }

    /// Whether the program has exited, and how. Once it has, the
    /// pseudoconsole is closed, so that its output ends after what the
    /// host still holds.
    pub fn try_wait(&mut self) -> io::Result<Option<Exit>> {
        let Some(code) = self.process.exit_code()? else {
            return Ok(None);
        };
        self.watch = None;
        console(&self.console).take();
        Ok(Some(Exit {
            // Exit codes are unsigned here; an NTSTATUS reads as negative.
            code: Some(code as i32),
            signal: None,
        }))
    }

    /// Waits for the program to exit.
    pub fn wait(&mut self) -> io::Result<()> {
        // SAFETY: the process handle is open while `self` lives.
        let waited = unsafe { WaitForSingleObject(raw(self.process.handle()), INFINITE) };
        if waited != WAIT_OBJECT_0 {
            return Err(io::Error::last_os_error());
        }
        self.try_wait().map(drop)
    }
}

/// The processes that end with a session's program: those attached to its
/// console, and its root process. It outlives the program.
///
/// Windows has no process group to signal. The console's processes are the
/// console host's to end, which it does when the pseudoconsole closes. The
/// root process is named by its pid and start time, so that a later process
/// with the same pid is left alone. Processes that left the console aren't
/// in the group.
#[derive(Clone)]
pub struct Group {
    pid: u32,
    started: u64,
    console: Console,
}

impl Group {
    /// Tells the processes their terminal is gone: closes the pseudoconsole
    /// if it is still open, on a thread of its own
    /// ([`crate::conpty::Pseudoconsole`]'s drop), so this never waits for
    /// the console host. The host then ends the console's processes.
    pub fn hang_up(&self) {
        console(&self.console).take();
    }

    /// Hangs the console's processes up and ends the root process.
    pub fn kill(&self) {
        self.hang_up();
        self.kill_remaining();
    }

    /// Ends what a hang-up left running. That is the root process, if it is
    /// still the one that was started: the console's other processes are
    /// ended by the console host.
    pub fn kill_remaining(&self) {
        let access = PROCESS_TERMINATE | PROCESS_QUERY_LIMITED_INFORMATION;
        // SAFETY: opens a process by pid; the handle is owned below.
        let Ok(handle) = (unsafe { OpenProcess(access, false, self.pid) }) else {
            // Gone, with every handle to it.
            return;
        };
        // SAFETY: the handle is new and owned by nothing else.
        let process = unsafe { OwnedHandle::from_raw_handle(handle.0) };
        if start_time(process.as_handle()).is_ok_and(|started| started == self.started) {
            // Fails for a process that has exited, which is as wanted.
            // SAFETY: the handle is open and may terminate the process.
            let _ = unsafe { TerminateProcess(raw(process.as_handle()), KILLED) };
        }
    }
}

/// Makes the window `hwnd` the owner of the console window of `program`'s
/// session (`REPARENT`). It is advisory: nothing happens for a session that
/// is gone (`None`) or whose pseudoconsole has closed, or with the ConPTY
/// built into Windows, which logs that once. The host applies it a moment
/// later, and a failure is only logged.
pub fn reparent(program: Option<&Program>, hwnd: u64) -> io::Result<()> {
    if let Some(program) = program
        && let Some(console) = console(&program.console).as_ref()
    {
        console.reparent(hwnd);
    }
    Ok(())
}

/// An error of [`crate::conpty`] with its Win32 code. Those carry it as an
/// `HRESULT`, which `io::Error` can't tell the kind of.
fn plain(error: io::Error) -> io::Error {
    match error.raw_os_error().map(|code| code as u32) {
        Some(code) if code & 0xffff_0000 == 0x8007_0000 => os_error(code & 0xffff),
        _ => error,
    }
}

fn raw(handle: BorrowedHandle<'_>) -> HANDLE {
    HANDLE(handle.as_raw_handle())
}

/// When `process` was created, in 100 ns units since 1601.
fn start_time(process: BorrowedHandle<'_>) -> io::Result<u64> {
    let mut created = FILETIME::default();
    let mut unused = [FILETIME::default(); 3];
    let [exited, kernel, user] = &mut unused;
    // SAFETY: the handle is open, and each time outlives the call.
    unsafe { GetProcessTimes(raw(process), &mut created, exited, kernel, user) }
        .map_err(|error| os_error(win32(&error)))?;
    Ok(u64::from(created.dwHighDateTime) << 32 | u64::from(created.dwLowDateTime))
}

/// The program `request` runs: its command, or else the session's command
/// interpreter. Brindle always names the program.
pub fn program_name(request: &PtyCommand) -> OsString {
    let program = request.command.or_else(|| {
        let own = |name: &str| name.eq_ignore_ascii_case("COMSPEC");
        request
            .env
            .iter()
            .find(|(name, value)| own(name) && !value.is_empty())
            .map(|(_, value)| value.as_str())
    });
    program.unwrap_or("cmd.exe").into()
}

/// The program's environment: the request's when it is exact, else this
/// process's with the request's over it, by name whatever the case.
fn environment(request: &PtyCommand) -> Vec<(String, String)> {
    let given = request
        .env
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()));
    if request.exact_env {
        return given.collect();
    }
    let set = |name: &str| request.env.keys().any(|set| set.eq_ignore_ascii_case(name));
    std::env::vars_os()
        .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
        .filter(|(name, _)| !set(name))
        .chain(given)
        .collect()
}

/// Whether the session asks for the ConPTY built into Windows.
fn wants_inbox(request: &PtyCommand) -> bool {
    request
        .env
        .iter()
        .any(|(name, value)| name.eq_ignore_ascii_case("BRINDLE_CONPTY") && value == INBOX)
}

/// A one-way pipe of this process's own: the holder's overlapped end, with
/// `access` (inbound to read from it, outbound to write to it), and the
/// plain end for the console host, opened with `client_access`.
fn pipe(
    access: FILE_FLAGS_AND_ATTRIBUTES,
    client_access: u32,
) -> io::Result<(OwnedHandle, OwnedHandle)> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let name = server_dir::pty_pipe(std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed));
    let wide: Vec<u16> = name.encode_utf16().chain([0]).collect();
    let failed = |what: &str, error: io::Error| {
        io::Error::new(error.kind(), format!("can't {what} the pipe {name}: {error}"))
    };
    let security = win::Security::for_pipe()?;
    // The name's first and only instance: nobody else serves it, and only
    // the open below connects to it.
    // SAFETY: the name is NUL-terminated, and it and the security
    // attributes outlive the call.
    let server = unsafe {
        CreateNamedPipeW(
            PCWSTR(wide.as_ptr()),
            access | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
            1,
            PIPE_BUFFER,
            PIPE_BUFFER,
            0,
            Some(security.attributes()),
        )
    };
    if server == INVALID_HANDLE_VALUE {
        return Err(failed("create", io::Error::last_os_error()));
    }
    // SAFETY: the handle is new and owned by nothing else.
    let server = unsafe { OwnedHandle::from_raw_handle(server.0) };
    // Not overlapped: ConPTY reads and writes its ends synchronously. It
    // isn't inherited; ConPTY duplicates it for its host.
    // SAFETY: the name is NUL-terminated and outlives the call.
    let client = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            client_access,
            FILE_SHARE_NONE,
            None,
            OPEN_EXISTING,
            SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
            None,
        )
    }
    .map_err(|error| failed("open", os_error(win32(&error))))?;
    // SAFETY: as above.
    let client = unsafe { OwnedHandle::from_raw_handle(client.0) };
    Ok((server, client))
}

/// Opens a pseudoconsole and starts `request`'s program on it.
pub fn spawn(request: &PtyCommand) -> io::Result<(Pty, Program)> {
    let (input, host_input) = pipe(PIPE_ACCESS_OUTBOUND, GENERIC_READ.0)?;
    let (output, host_output) = pipe(PIPE_ACCESS_INBOUND, GENERIC_WRITE.0)?;
    let mut pseudoconsole = Pseudoconsole::create(
        wants_inbox(request),
        request.cols,
        request.rows,
        host_input.as_handle(),
        host_output.as_handle(),
    )
    .map_err(plain)?;
    // The host has its own copies, so the output ends when the host exits.
    drop((host_input, host_output));
    // The host takes the end of its input as a close too, and the holder's
    // end goes with the `Pty`: this copy stays until the close goes out.
    pseudoconsole.hold_input_open(input.try_clone()?);
    #[cfg(test)]
    let packaged = pseudoconsole.is_packaged();

    let program = program_name(request);
    let line = launch_line(&program.to_string_lossy(), request.args);
    let env = environment(request);
    let process = pseudoconsole.start(
        &line,
        request.cwd.map(Path::new),
        env.iter().map(|(name, value)| (name.as_str(), value.as_str())),
    )
    .map_err(plain)?;
    let started = match start_time(process.handle()) {
        Ok(started) => started,
        Err(error) => {
            // Without it the program couldn't be ended later.
            // SAFETY: the handle is open and may terminate the process.
            let _ = unsafe { TerminateProcess(raw(process.handle()), KILLED) };
            return Err(error);
        }
    };
    let console = Arc::new(Mutex::new(Some(pseudoconsole)));
    Ok((
        Pty {
            pipe: Pipe::of(Some(output), Some(input)),
            console: Arc::clone(&console),
            #[cfg(test)]
            packaged,
        },
        Program {
            process,
            started,
            console,
            watch: None,
        },
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::time::{Duration, Instant};

    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
        TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::Threading::PROCESS_SYNCHRONIZE;
    use windows::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, DispatchMessageW, GW_OWNER, GetWindow, MSG, PM_REMOVE,
        PeekMessageW, WINDOW_EX_STYLE, WS_OVERLAPPED,
    };
    use windows::core::w;

    use super::*;
    use crate::server::event_loop::{Event, EventLoop, Interest, Loop};

    const LIMIT: Duration = Duration::from_secs(60);
    const TOKEN: usize = 7;

    /// This process's environment, with `extra` over it.
    fn env(extra: &[(&str, &str)]) -> BTreeMap<String, String> {
        let set = |name: &str| extra.iter().any(|(set, _)| set.eq_ignore_ascii_case(name));
        std::env::vars_os()
            .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
            .filter(|(name, _)| !set(name))
            .chain(extra.iter().map(|(name, value)| (name.to_string(), value.to_string())))
            .collect()
    }

    /// Starts `command` on a new pseudoconsole, in the temporary directory.
    fn run(command: &str, args: &[&str], extra: &[(&str, &str)], cols: u16, rows: u16) -> (Pty, Program) {
        let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
        let env = env(extra);
        let temp = std::env::temp_dir();
        let (pty, program) = spawn(&PtyCommand {
            command: Some(command),
            args: &args,
            cwd: temp.to_str(),
            env: &env,
            cols,
            rows,
            cell_width: 0,
            cell_height: 0,
            exact_env: true,
        })
        .unwrap();
        assert_ne!(program.pid(), 0);
        (pty, program)
    }

    /// The processes `parent` started that are still running: each one's
    /// pid, a handle to wait for it with, and its program's file name.
    fn children(parent: u32) -> Vec<(u32, OwnedHandle, String)> {
        // SAFETY: the snapshot handle is new and owned by nothing else.
        let snapshot = unsafe {
            OwnedHandle::from_raw_handle(CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).unwrap().0)
        };
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        let mut found = Vec::new();
        // SAFETY: the snapshot is open, and `entry` has its size set.
        let mut more = unsafe { Process32FirstW(raw(snapshot.as_handle()), &mut entry) }.is_ok();
        while more {
            if entry.th32ParentProcessID == parent
                && let Some(process) = open(entry.th32ProcessID)
            {
                let name = entry.szExeFile.split(|unit| *unit == 0).next().unwrap();
                found.push((entry.th32ProcessID, process, String::from_utf16_lossy(name)));
            }
            // SAFETY: as above.
            more = unsafe { Process32NextW(raw(snapshot.as_handle()), &mut entry) }.is_ok();
        }
        found
    }

    /// The running process `pid`, to wait for.
    fn open(pid: u32) -> Option<OwnedHandle> {
        let access = PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION;
        // SAFETY: opens a process by pid; the handle is owned at once.
        let process = unsafe { OpenProcess(access, false, pid) }.ok()?;
        // SAFETY: the handle is new and owned by nothing else.
        let process = unsafe { OwnedHandle::from_raw_handle(process.0) };
        (!has_exited(&process)).then_some(process)
    }

    fn has_exited(process: &OwnedHandle) -> bool {
        // SAFETY: the handle is open.
        unsafe { WaitForSingleObject(raw(process.as_handle()), 0) == WAIT_OBJECT_0 }
    }

    /// A hidden top-level window of this thread's, destroyed on drop.
    struct Window(HWND);

    impl Window {
        fn new() -> Self {
            // SAFETY: a window of a system class, with no parent, menu or
            // parameter; it is never shown.
            let window = unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("STATIC"),
                    w!("brindle test owner"),
                    WS_OVERLAPPED,
                    0,
                    0,
                    100,
                    100,
                    None,
                    None,
                    None,
                    None,
                )
            };
            Self(window.unwrap())
        }

        fn id(&self) -> u64 {
            self.0.0 as usize as u64
        }

        /// Handles what other threads sent this thread's windows: a host
        /// that makes one an owner may wait for it to answer.
        fn pump() {
            let mut message = MSG::default();
            // SAFETY: `message` outlives each call.
            while unsafe { PeekMessageW(&mut message, None, 0, 0, PM_REMOVE) }.as_bool() {
                // SAFETY: the message was just read.
                unsafe { DispatchMessageW(&message) };
            }
        }
    }

    impl Drop for Window {
        fn drop(&mut self) {
            // SAFETY: the window is this thread's, as `Window` isn't `Send`.
            unsafe { DestroyWindow(self.0) }.unwrap();
        }
    }

    /// The owner of the window `window`, if it has one (0 if not).
    fn owner(window: u64) -> u64 {
        let window = HWND(window as usize as *mut std::ffi::c_void);
        // SAFETY: any window handle may be asked about.
        let owner = unsafe { GetWindow(window, GW_OWNER) };
        owner.map_or(0, |owner| owner.0 as usize as u64)
    }

    /// PowerShell that prints its console window as `hwnd=N;` and waits.
    const CONSOLE_WINDOW: &str = "$w = (Add-Type -Name Console -Namespace BrindleTest -PassThru \
        -MemberDefinition '[DllImport(\"kernel32.dll\")] public static extern System.IntPtr \
        GetConsoleWindow();')::GetConsoleWindow(); Write-Host ('hwnd=' + $w + ';'); \
        Start-Sleep -Seconds 600";

    /// A session of [`CONSOLE_WINDOW`], and its console window.
    fn session_with_a_console_window(extra: &[(&str, &str)]) -> (Session, u64) {
        let args = ["-NoProfile", "-Command", CONSOLE_WINDOW];
        let mut session = Session::start("powershell.exe", &args, extra);
        session.until(";");
        let text = session.text();
        let digits = text.split("hwnd=").nth(1).and_then(|rest| rest.split(';').next());
        let window: u64 = digits.and_then(|digits| digits.parse().ok()).unwrap_or(0);
        assert_ne!(window, 0, "no console window in {text:?}");
        (session, window)
    }

    /// A session as the holder runs one: its pseudoconsole registered with
    /// a loop, and its program watched.
    struct Session {
        ev: Loop,
        pty: Pty,
        program: Program,
        output: Vec<u8>,
        /// The output has ended.
        ended: bool,
        exit: Option<i32>,
    }

    impl Session {
        fn start(command: &str, args: &[&str], extra: &[(&str, &str)]) -> Self {
            Self::sized(command, args, extra, 80, 24)
        }

        fn sized(command: &str, args: &[&str], extra: &[(&str, &str)], cols: u16, rows: u16) -> Self {
            let (pty, mut program) = run(command, args, extra, cols, rows);
            let mut ev = Loop::new().unwrap();
            ev.register(&pty.source(), TOKEN, Interest::READABLE).unwrap();
            program.watch(&ev.waker()).unwrap();
            Self {
                ev,
                pty,
                program,
                output: Vec::new(),
                ended: false,
                exit: None,
            }
        }

        /// One turn of the holder's loop: waits, reads what is there, and
        /// reaps. Says whether the loop was woken.
        fn turn(&mut self) -> bool {
            let mut events: Vec<Event> = Vec::new();
            let woken = self.ev.wait(&mut events, Some(Duration::from_millis(100))).unwrap();
            if !events.is_empty() {
                self.read();
            }
            if self.exit.is_none()
                && let Some(exit) = self.program.try_wait().unwrap()
            {
                assert_eq!(exit.signal, None);
                self.exit = exit.code;
            }
            woken
        }

        fn read(&mut self) {
            let mut buffer = [0u8; 32 * 1024];
            while !self.ended {
                match self.pty.read(&mut buffer) {
                    Ok(0) => self.ended = true,
                    Ok(read) => self.output.extend_from_slice(&buffer[..read]),
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(error) => panic!("reading output: {error}"),
                }
            }
        }

        fn text(&self) -> String {
            String::from_utf8_lossy(&self.output).into_owned()
        }

        /// Turns until the output has `text`.
        fn until(&mut self, text: &str) {
            let deadline = Instant::now() + LIMIT;
            while !self.text().contains(text) {
                assert!(
                    Instant::now() < deadline && !self.ended,
                    "no {text:?} in {:?}",
                    self.text()
                );
                self.turn();
            }
        }

        fn send(&mut self, text: &str) {
            let mut bytes = text.as_bytes();
            let deadline = Instant::now() + LIMIT;
            while !bytes.is_empty() {
                match self.pty.write(bytes) {
                    Ok(written) => bytes = &bytes[written..],
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        assert!(Instant::now() < deadline, "the console takes no input");
                        self.turn();
                    }
                    Err(error) => panic!("writing input: {error}"),
                }
            }
        }

        /// Turns until the program has exited and its output has ended;
        /// the output and the exit code.
        fn finish(mut self) -> (String, i32) {
            let deadline = Instant::now() + LIMIT;
            while !(self.ended && self.exit.is_some()) {
                assert!(
                    Instant::now() < deadline,
                    "ended: {}, exit: {:?}, {} bytes of output",
                    self.ended,
                    self.exit,
                    self.output.len()
                );
                self.turn();
            }
            (self.text(), self.exit.unwrap())
        }
    }

    #[test]
    fn a_program_runs_and_its_output_ends_after_it_exits() {
        let (output, code) = Session::start("cmd.exe", &["/c", "echo hello"], &[]).finish();
        assert!(output.contains("hello"), "output: {output:?}");
        assert_eq!(code, 0);
    }

    #[test]
    fn typed_input_reaches_an_interactive_program() {
        let mut session = Session::start("cmd.exe", &["/D"], &[("PROMPT", "ready$G")]);
        session.until("ready>");
        // The echo of what is typed has the caret; the command's output
        // doesn't.
        session.send("echo brin^dle\r");
        session.until("brindle");
        session.send("exit 5\r");
        let (_, code) = session.finish();
        assert_eq!(code, 5);
    }

    #[test]
    fn a_resize_reaches_the_program() {
        const SIZE: &str = "powershell -NoProfile -Command \"$s = $Host.UI.RawUI.WindowSize; \
                            Write-Host ('size=' + $s.Width + 'x' + $s.Height + ';')\"\r";
        let mut session = Session::sized("cmd.exe", &["/D"], &[("PROMPT", "ready$G")], 90, 20);
        session.until("ready>");
        session.send(SIZE);
        session.until("size=90x20;");
        session.pty.resize(101, 33, 0, 0).unwrap();
        // The resize and the input travel separately, so ask until it has
        // arrived.
        let reports = |session: &Session| {
            // The typed line's echo has no digit after `size=`.
            let text = session.text();
            let digit = |at: usize| text[at + 5..].starts_with(|next: char| next.is_ascii_digit());
            text.match_indices("size=").filter(|(at, _)| digit(*at)).count()
        };
        let deadline = Instant::now() + LIMIT;
        while !session.text().contains("size=101x33;") {
            let before = reports(&session);
            session.send(SIZE);
            // The report, then the next prompt.
            while reports(&session) == before || !session.text().ends_with("ready>") {
                assert!(Instant::now() < deadline, "output: {:?}", session.text());
                session.turn();
            }
        }
        session.send("exit\r");
        session.finish();
    }

    #[test]
    fn the_exit_wakes_the_loop_and_gives_the_exit_code() {
        let mut session = Session::start("cmd.exe", &["/c", "exit 3"], &[]);
        // Nothing is read, so only the exit can end a wait.
        session.ev.modify(TOKEN, Interest::NONE).unwrap();
        let mut events = Vec::new();
        assert!(session.ev.wait(&mut events, Some(LIMIT)).unwrap(), "not woken");
        let exit = session.program.try_wait().unwrap().expect("it has exited");
        assert_eq!((exit.code, exit.signal), (Some(3), None));
        session.exit = exit.code;
        session.pty.resize(100, 30, 0, 0).unwrap();
        session.program.wait().unwrap();
        session.ev.modify(TOKEN, Interest::READABLE).unwrap();
        assert_eq!(session.finish().1, 3);
    }

    #[test]
    fn a_flood_followed_by_exit_loses_no_output() {
        const LINES: usize = 40_000;
        let path = std::env::temp_dir().join(format!("brindle-pty-flood-{}.txt", std::process::id()));
        let mut file = String::new();
        for line in 0..LINES {
            file.push_str(&format!("#{line:07}# {}\r\n", "x".repeat(60)));
        }
        std::fs::write(&path, &file).unwrap();
        let session = Session::sized("cmd.exe", &["/c", "type", path.to_str().unwrap()], &[], 100, 30);
        let (output, code) = session.finish();
        std::fs::remove_file(&path).ok();
        assert_eq!(code, 0);
        assert!(output.len() >= file.len(), "{} of {} bytes", output.len(), file.len());
        let mut from = 0;
        for line in 0..LINES {
            let marker = format!("#{line:07}# {}", "x".repeat(60));
            match output[from..].find(&marker) {
                Some(at) => from += at + marker.len(),
                None => panic!("line {line} is missing after byte {from} of {}", output.len()),
            }
        }
    }

    #[test]
    fn the_sessions_environment_picks_the_host() {
        let inbox = Session::start("cmd.exe", &["/c", "echo %BRINDLE_CONPTY%"], &[("BRINDLE_CONPTY", "inbox")]);
        assert!(!inbox.pty.packaged);
        let (output, code) = inbox.finish();
        assert!(output.contains("inbox"), "output: {output:?}");
        assert_eq!(code, 0);

        // Another value, and the holder's own environment, don't count.
        let packaged = Session::start("cmd.exe", &["/c", "echo hello"], &[("BRINDLE_CONPTY", "other")]);
        assert_eq!(packaged.pty.packaged, crate::conpty::has_embedded());
        assert_eq!(packaged.finish().1, 0);
    }

    #[test]
    fn a_program_that_cannot_start_fails_the_spawn() {
        let env = env(&[]);
        let spawned = spawn(&PtyCommand {
            command: Some("brindle-no-such-program.exe"),
            args: &[],
            cwd: None,
            env: &env,
            cols: 80,
            rows: 24,
            cell_width: 0,
            cell_height: 0,
            exact_env: true,
        });
        assert_eq!(spawned.err().map(|error| error.kind()), Some(io::ErrorKind::NotFound));
    }

    #[test]
    fn dropping_the_pty_ends_the_program_on_its_console() {
        let mut session = Session::start("cmd.exe", &["/D"], &[("PROMPT", "ready$G")]);
        session.until("ready>");
        let Session { pty, mut program, .. } = session;
        // A hang-up: the pseudoconsole closes, and the console's programs
        // end with it.
        drop(pty);
        let deadline = Instant::now() + LIMIT;
        while program.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "the program outlived its console");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn a_kill_ends_the_root_process_and_only_that_one() {
        let mut session = Session::start("cmd.exe", &["/D"], &[("PROMPT", "ready$G")]);
        session.until("ready>");
        let group = session.program.group();
        // Another process's start time: this pid isn't the group's.
        let other = Group { started: group.started + 1, ..group.clone() };
        other.kill_remaining();
        assert!(session.program.try_wait().unwrap().is_none());
        group.kill_remaining();
        let (_, code) = session.finish();
        assert_eq!(code, KILLED as i32);
        group.kill_remaining();
        group.kill();
    }

    #[test]
    fn a_hang_up_ends_the_program_and_its_console_children() {
        let mut session = Session::start("cmd.exe", &["/c", "ping -t localhost"], &[]);
        session.until("Reply from");
        let running = children(session.program.pid());
        let ping = running.iter().find(|(_, _, name)| name.eq_ignore_ascii_case("PING.EXE"));
        let (_, ping, _) = ping.expect("ping is the shell's child");

        // As the holder's `HANGUP` does: the PTY goes, the group is hung
        // up, and after the grace what remains is killed.
        let Session { mut ev, pty, mut program, .. } = session;
        let group = program.group();
        let hung_up = Instant::now();
        ev.deregister(TOKEN).unwrap();
        drop(pty);
        group.hang_up();
        assert!(hung_up.elapsed() < Duration::from_millis(500), "the hang-up waited");
        let grace = hung_up + crate::pty::GRACEFUL_SHUTDOWN;
        while !(has_exited(ping) && program.try_wait().unwrap().is_some()) {
            assert!(Instant::now() < grace, "the console's programs outlived the grace");
            std::thread::sleep(Duration::from_millis(10));
        }
        // Nothing remains, and the pid may be another process's by now.
        group.kill_remaining();
        assert_ne!(program.try_wait().unwrap().unwrap().code, Some(KILLED as i32));
    }

    /// Learns the console host of `program` while it is on its console, on
    /// a thread of its own, as it may not be on it for long: a handle to
    /// the host, to wait for and end, or `None` if the program exited first.
    fn watch_console_host(program: &Program) -> std::thread::JoinHandle<Option<OwnedHandle>> {
        let process = program.process.handle().try_clone_to_owned().unwrap();
        std::thread::spawn(move || {
            loop {
                let exited = has_exited(&process);
                if let Some(host) = crate::conpty::console_host(process.as_handle()) {
                    return open_to_end(host);
                }
                if exited {
                    return None;
                }
                std::thread::yield_now();
            }
        })
    }

    /// The running process `pid`, to wait for and terminate.
    fn open_to_end(pid: u32) -> Option<OwnedHandle> {
        let access = PROCESS_SYNCHRONIZE | PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE;
        // SAFETY: opens a process by pid; the handle is owned at once.
        let process = unsafe { OpenProcess(access, false, pid) }.ok()?;
        // SAFETY: the handle is new and owned by nothing else.
        let process = unsafe { OwnedHandle::from_raw_handle(process.0) };
        (!has_exited(&process)).then_some(process)
    }

    /// What a session of `cmd /c ping` may leave running: its root, the
    /// `ping` the root started, and its console host. They are ended on
    /// drop, also when a test fails.
    struct Leftovers {
        group: Group,
        root: u32,
        host: Option<OwnedHandle>,
    }

    impl Leftovers {
        /// The `ping`s the root started that still run.
        fn pings(&self) -> Vec<u32> {
            let running = children(self.root).into_iter();
            let pings = running.filter(|(_, _, name)| name.eq_ignore_ascii_case("PING.EXE"));
            pings.map(|(pid, ..)| pid).collect()
        }

        fn host_runs(&self) -> bool {
            self.host.as_ref().is_some_and(|host| !has_exited(host))
        }
    }

    impl Drop for Leftovers {
        fn drop(&mut self) {
            self.group.kill();
            let pings = self.pings().into_iter().filter_map(open_to_end);
            for process in pings.chain(self.host.take()) {
                // SAFETY: the handle is open and may terminate the process.
                let _ = unsafe { TerminateProcess(raw(process.as_handle()), KILLED) };
            }
        }
    }

    #[test]
    fn a_hang_up_as_the_program_starts_ends_it_and_its_console_children() {
        const ROUNDS: usize = 20;
        // The close can reach the console host before `cmd` is on the
        // console, and `ping` starts only once it is. Each host, in turn.
        for round in 0..ROUNDS {
            let host: &[(&str, &str)] = match round % 2 {
                0 => &[],
                _ => &[("BRINDLE_CONPTY", "inbox")],
            };
            let (pty, program) = run("cmd.exe", &["/c", "ping -t localhost"], host, 80, 24);
            let watcher = watch_console_host(&program);
            let mut left = Leftovers {
                group: program.group(),
                root: program.pid(),
                host: None,
            };

            // As the holder's `HANGUP` does, right after the spawn.
            let hung_up = Instant::now();
            drop(pty);
            left.group.hang_up();
            assert!(hung_up.elapsed() < Duration::from_millis(500), "the hang-up waited");
            let grace = hung_up + crate::pty::GRACEFUL_SHUTDOWN;
            let deadline = grace + Duration::from_secs(2);
            let mut killed = false;
            while program.process.exit_code().unwrap().is_none() {
                assert!(Instant::now() < deadline, "round {round}: the program outlived the grace");
                if !killed && Instant::now() >= grace {
                    left.group.kill_remaining();
                    killed = true;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            left.host = watcher.join().unwrap();
            // Nothing on the console outlives it, nor does the host.
            while !left.pings().is_empty() || left.host_runs() {
                assert!(
                    Instant::now() < deadline,
                    "round {round}: ping {:?} and the console host ({}) outlived the grace; \
                     the program exited with {:#x?}",
                    left.pings(),
                    if left.host_runs() { "running" } else { "gone" },
                    program.process.exit_code().unwrap(),
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            let code = program.process.exit_code().unwrap();
            assert_ne!(code, Some(KILLED), "round {round}: the close never reached the program");
        }
    }

    #[test]
    fn a_kill_after_the_grace_ends_a_root_that_left_its_console() {
        // No console close reaches a root whose console is someone else's,
        // which stands for one that ignores the close.
        let mut session = Session::start("cmd.exe", &["/D"], &[("PROMPT", "ready$G")]);
        session.until("ready>");
        let group = session.program.group();
        let stale = Group { console: Arc::default(), ..group.clone() };
        stale.hang_up();
        assert!(session.program.try_wait().unwrap().is_none());
        stale.kill_remaining();
        assert_eq!(session.finish().1, KILLED as i32);
    }

    #[test]
    fn the_loop_serves_another_session_while_a_close_is_in_flight() {
        const OTHER: usize = 8;
        // A session whose output nobody reads: a host that closes only
        // once its output is read (older ones) can't finish closing.
        let flood = ["/c", "for /l %i in () do @echo brindle brindle brindle brindle"];
        let (stuck, mut stuck_program) = run("cmd.exe", &flood, &[], 80, 24);
        let mut session = Session::start("cmd.exe", &["/D"], &[("PROMPT", "ready$G")]);
        session.ev.register(&stuck.source(), OTHER, Interest::NONE).unwrap();
        session.until("ready>");

        let hung_up = Instant::now();
        stuck_program.group().hang_up();
        assert!(hung_up.elapsed() < Duration::from_millis(500), "the hang-up waited");
        // The close is in flight, or blocked, and the loop goes on.
        for round in 0..3 {
            session.send(&format!("echo round^{round}\r"));
            session.until(&format!("round{round}"));
        }

        // Once the pipes are gone, the host can't be writing.
        session.ev.deregister(OTHER).unwrap();
        drop(stuck);
        let deadline = Instant::now() + LIMIT;
        while stuck_program.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline, "the program outlived its console");
            if Instant::now() > hung_up + crate::pty::GRACEFUL_SHUTDOWN {
                stuck_program.group().kill_remaining();
            }
            session.turn();
        }
        session.send("exit\r");
        session.finish();
    }

    #[test]
    fn a_reparent_gives_the_console_window_an_owner() {
        if !crate::conpty::has_embedded() {
            eprintln!("skipped: this build has no packaged ConPTY");
            return;
        }
        let (mut session, console_window) = session_with_a_console_window(&[]);
        assert_eq!(owner(console_window), 0);
        let first = Window::new();
        let second = Window::new();
        for window in [&first, &second] {
            reparent(Some(&session.program), window.id()).unwrap();
            // The host does it a moment later.
            let deadline = Instant::now() + Duration::from_secs(5);
            while owner(console_window) != window.id() {
                assert!(
                    Instant::now() < deadline,
                    "the owner is {:#x}, not {:#x}",
                    owner(console_window),
                    window.id()
                );
                Window::pump();
                session.turn();
            }
        }
        // A destroyed owner leaves the console window, with no owner.
        drop(second);
        session.turn();
        assert_eq!(owner(console_window), 0);
        reparent(Some(&session.program), first.id()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while owner(console_window) != first.id() {
            assert!(Instant::now() < deadline, "no owner after the first one went");
            Window::pump();
            session.turn();
        }

        // A session that is gone, and one whose console has closed.
        reparent(None, first.id()).unwrap();
        session.program.group().kill();
        reparent(Some(&session.program), first.id()).unwrap();
        session.finish();
    }

    #[test]
    fn the_inbox_host_ignores_a_reparent() {
        let (mut session, console_window) =
            session_with_a_console_window(&[("BRINDLE_CONPTY", "inbox")]);
        assert!(!session.pty.packaged);
        let window = Window::new();
        for _ in 0..2 {
            reparent(Some(&session.program), window.id()).unwrap();
        }
        let settled = Instant::now() + Duration::from_secs(1);
        while Instant::now() < settled {
            Window::pump();
            session.turn();
        }
        assert_eq!(owner(console_window), 0);
        session.program.group().kill();
        session.finish();
    }

    #[test]
    fn the_program_is_the_command_or_the_command_interpreter() {
        fn request<'a>(
            command: Option<&'a str>,
            env: &'a BTreeMap<String, String>,
        ) -> PtyCommand<'a> {
            PtyCommand {
                command,
                args: &[],
                cwd: None,
                env,
                cols: 80,
                rows: 24,
                cell_width: 0,
                cell_height: 0,
                exact_env: true,
            }
        }
        let mut env = BTreeMap::new();
        assert_eq!(program_name(&request(Some("pwsh.exe"), &env)), "pwsh.exe");
        assert_eq!(program_name(&request(None, &env)), "cmd.exe");
        env.insert("ComSpec".to_string(), r"C:\Windows\System32\cmd.exe".to_string());
        env.insert("brindle_conpty".to_string(), "inbox".to_string());
        assert_eq!(program_name(&request(None, &env)), r"C:\Windows\System32\cmd.exe");
        assert!(wants_inbox(&request(None, &env)));
        env.insert("brindle_conpty".to_string(), "Inbox".to_string());
        assert!(!wants_inbox(&request(None, &env)));
    }
}
