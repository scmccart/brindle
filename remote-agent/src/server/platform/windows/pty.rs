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
//! - **Hangup:** dropping the [`Pty`] closes the pseudoconsole in the same
//!   way, which ends the programs attached to the console. [`Group`] is a
//!   minimal stand-in until terminal-server-windows 5.3: its hang-up does
//!   nothing more, and its kills end only the session's root process.

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

/// The exit code of a program the holder ended.
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

/// The processes that end with a session's program. It outlives the
/// program.
///
/// A minimal stand-in until terminal-server-windows 5.3: only the root
/// process, named by its pid and start time, so that a later process with
/// the same pid is left alone.
#[derive(Debug, Clone, Copy)]
pub struct Group {
    pid: u32,
    started: u64,
}

impl Group {
    /// Tells the processes their terminal is gone. Closing the
    /// pseudoconsole (dropping the session's [`Pty`]) is what does that.
    pub fn hang_up(self) {}

    /// Ends the root process.
    pub fn kill(self) {
        self.kill_remaining();
    }

    /// Ends the root process if it is still running.
    pub fn kill_remaining(self) {
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
    let pseudoconsole = Pseudoconsole::create(
        wants_inbox(request),
        request.cols,
        request.rows,
        host_input.as_handle(),
        host_output.as_handle(),
    )
    .map_err(plain)?;
    // The host has its own copies, so the output ends when the host exits.
    drop((host_input, host_output));
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
            let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
            let env = env(extra);
            let temp = std::env::temp_dir();
            let (pty, mut program) = spawn(&PtyCommand {
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
        Group { started: group.started + 1, ..group }.kill();
        assert!(session.program.try_wait().unwrap().is_none());
        group.hang_up();
        assert!(session.program.try_wait().unwrap().is_none());
        group.kill();
        let (_, code) = session.finish();
        assert_eq!(code, KILLED as i32);
        group.kill_remaining();
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
