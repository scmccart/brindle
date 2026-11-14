//! A pseudoconsole and the program started on it.

use std::ffi::c_void;
use std::io;
use std::os::windows::io::{
    AsHandle as _, AsRawHandle as _, BorrowedHandle, FromRawHandle as _, OwnedHandle,
};
use std::path::Path;
use std::sync::Once;
use std::time::{Duration, Instant};

use windows::Wdk::System::Threading::{NtQueryInformationProcess, PROCESSINFOCLASS};
use windows::Win32::Foundation::{CloseHandle, HANDLE, HWND, WAIT_OBJECT_0};
use windows::Win32::System::Console::{COORD, HPCON};
use windows::Win32::System::Threading::{
    CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, InitializeProcThreadAttributeList,
    LPPROC_THREAD_ATTRIBUTE_LIST, PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, PROCESS_INFORMATION,
    STARTF_USESTDHANDLES, STARTUPINFOEXW, UpdateProcThreadAttribute, WaitForSingleObject,
};
use windows::core::{PCWSTR, PWSTR};

use super::api::{Api, api, wide};
use super::cmdline::environment_block;

/// Count cells per code point, as `unicode-width` (the grid) does. From the
/// package's `conpty.h`; the built-in host accepts and ignores it.
const PSEUDOCONSOLE_GLYPH_WIDTH_WCSWIDTH: u32 = 0x10;

/// `NtQueryInformationProcess`'s `ProcessConsoleHostProcess`, which isn't
/// documented: the console host a process is on, with flags in the low two
/// bits. Until the process has connected to a console it names the process
/// that started it. It stays once the process has exited.
const PROCESS_CONSOLE_HOST_PROCESS: PROCESSINFOCLASS = PROCESSINFOCLASS(49);
/// The flag in its answer that the process has connected to the host.
const CONNECTED: usize = 1;

/// How long closing waits for the program to be on the console: as long as
/// the holder gives a hung-up program before it terminates it.
const CONNECT_LIMIT: Duration = crate::pty::GRACEFUL_SHUTDOWN;
/// How often closing looks.
const CONNECT_POLL: Duration = Duration::from_millis(1);

/// An open pseudoconsole. Dropping it releases and closes it without
/// blocking the dropping thread.
///
/// Closing has the host send `CTRL_CLOSE_EVENT` to the programs on the
/// console and then exit once none is left. The host does that once, on
/// the close or on the end of its input, whichever comes first. A program
/// still starting isn't on the console yet: the host takes it afterwards,
/// untold, and stays for it, so it and what it starts outlive the console.
/// So the close waits for the program started here to be on the console
/// (or gone) first, for at most [`CONNECT_LIMIT`], and keeps the input open
/// until then ([`Pseudoconsole::hold_input_open`]). A program that a
/// program on the console starts as the close goes out can still miss it;
/// [`console_host`] finds those for the caller to end.
pub struct Pseudoconsole {
    handle: HPCON,
    /// The ConPTY that made it, which every later call must go to.
    api: &'static Api,
    /// The program started on it, which the close waits for.
    program: Option<OwnedHandle>,
    /// A copy of the caller's end of the input, closed as the close goes
    /// out.
    input: Option<OwnedHandle>,
}

// SAFETY: a pseudoconsole handle may be used from any thread.
unsafe impl Send for Pseudoconsole {}
// SAFETY: as above; its functions take the handle by value.
unsafe impl Sync for Pseudoconsole {}

impl Pseudoconsole {
    /// Creates a pseudoconsole of `cols` by `rows` cells that reads its
    /// input from `input` and writes its output to `output`, which must be
    /// synchronous pipe ends. It duplicates both, so the caller closes its
    /// copies of them afterwards; the output pipe then reaches its end when
    /// the console host exits.
    ///
    /// `inbox` picks the ConPTY built into Windows rather than the packaged
    /// one (a session with `BRINDLE_CONPTY=inbox` in its environment). The
    /// built-in one is also used when the packaged one isn't usable.
    pub fn create(
        inbox: bool,
        cols: u16,
        rows: u16,
        input: BorrowedHandle<'_>,
        output: BorrowedHandle<'_>,
    ) -> io::Result<Self> {
        let api = api(inbox);
        let mut handle = HPCON::default();
        // SAFETY: both handles are open for the call, and `handle` is
        // written only on success.
        unsafe {
            (api.create)(
                coord(cols, rows),
                HANDLE(input.as_raw_handle()),
                HANDLE(output.as_raw_handle()),
                PSEUDOCONSOLE_GLYPH_WIDTH_WCSWIDTH,
                &mut handle,
            )
        }
        .ok()?;
        Ok(Self {
            handle,
            api,
            program: None,
            input: None,
        })
    }

    /// Keeps `input`, a copy of the caller's end of the input pipe, open
    /// until the close goes out. The host takes the end of its input as a
    /// close, which mustn't come before the program is on the console
    /// either, and the caller's own end may go first.
    pub fn hold_input_open(&mut self, input: OwnedHandle) {
        self.input = Some(input);
    }

    /// Whether the packaged ConPTY made this pseudoconsole, not the one
    /// built into Windows.
    #[cfg_attr(not(test), allow(dead_code, reason = "read by tests"))]
    pub fn is_packaged(&self) -> bool {
        self.api.packaged
    }

    pub fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        // SAFETY: `handle` is open until drop.
        unsafe { (self.api.resize)(self.handle, coord(cols, rows)) }.ok()?;
        Ok(())
    }

    /// Makes the window `hwnd` the owner of the console window programs can
    /// ask for (`GetConsoleWindow`), so their dialogs belong to that window.
    /// The host does it a moment later. Only the packaged host supports it;
    /// with the built-in one this does nothing, and says so in the log the
    /// first time.
    pub fn reparent(&self, hwnd: u64) {
        let Some(reparent) = self.api.reparent else {
            static LOGGED: Once = Once::new();
            LOGGED.call_once(|| {
                eprintln!(
                    "conpty: the ConPTY built into Windows can't give a console window an \
                     owner; ignoring that for window {hwnd:#x} and from now on"
                );
            });
            return;
        };
        let window = HWND(hwnd as usize as *mut c_void);
        // SAFETY: `handle` is open until drop; any HWND is accepted.
        if let Err(err) = unsafe { reparent(self.handle, window) }.ok() {
            eprintln!("conpty: parenting a console to window {hwnd:#x}: {err}");
        }
    }

    /// Starts the program of the command line `line` (see
    /// [`super::cmdline`]) on this pseudoconsole, in `cwd` (this process's
    /// directory when `None`), with exactly the environment `env`. Closing
    /// the pseudoconsole waits for it to be on the console.
    pub fn start<'a>(
        &mut self,
        line: &str,
        cwd: Option<&Path>,
        env: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> io::Result<Process> {
        let mut size = 0;
        // SAFETY: the first call only reports the size the list needs.
        unsafe { InitializeProcThreadAttributeList(None, 1, None, &mut size) }.ok();
        let mut list_buf = vec![0u8; size];
        let list = LPPROC_THREAD_ATTRIBUTE_LIST(list_buf.as_mut_ptr().cast());
        // SAFETY: `list_buf` has the reported size and outlives the list.
        unsafe { InitializeProcThreadAttributeList(Some(list), 1, None, &mut size) }?;
        struct List(LPPROC_THREAD_ATTRIBUTE_LIST);
        impl Drop for List {
            fn drop(&mut self) {
                // SAFETY: initialized above, deleted once.
                unsafe { DeleteProcThreadAttributeList(self.0) };
            }
        }
        let list = List(list);
        // SAFETY: the pseudoconsole attribute's value is the HPCON itself.
        unsafe {
            UpdateProcThreadAttribute(
                list.0,
                0,
                PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE as usize,
                Some(self.handle.0 as *const c_void),
                std::mem::size_of::<HPCON>(),
                None,
                None,
            )
        }?;

        let mut startup = STARTUPINFOEXW::default();
        startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
        // With null standard handles, the program gets the pseudoconsole's
        // instead of anything of this process's.
        startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
        startup.lpAttributeList = list.0;

        let mut line: Vec<u16> = line.encode_utf16().chain(Some(0)).collect();
        let env = environment_block(env);
        let cwd = cwd.map(|dir| wide(dir.as_os_str()));
        let mut info = PROCESS_INFORMATION::default();
        // SAFETY: every pointer is valid for the call; the command line
        // buffer is writable, as CreateProcessW requires.
        unsafe {
            CreateProcessW(
                PCWSTR::null(),
                Some(PWSTR(line.as_mut_ptr())),
                None,
                None,
                false,
                EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
                Some(env.as_ptr().cast()),
                cwd.as_ref()
                    .map_or(PCWSTR::null(), |dir| PCWSTR(dir.as_ptr())),
                &startup.StartupInfo,
                &mut info,
            )
        }?;
        // SAFETY: the thread handle is ours and unused.
        unsafe { CloseHandle(info.hThread) }.ok();
        // SAFETY: the process handle is ours.
        let handle = unsafe { OwnedHandle::from_raw_handle(info.hProcess.0) };
        // The close waits through a copy. Copying a handle of ours doesn't
        // fail, and without one the close just wouldn't wait.
        self.program = handle.try_clone().ok();
        Ok(Process {
            pid: info.dwProcessId,
            handle,
        })
    }
}

impl Drop for Pseudoconsole {
    fn drop(&mut self) {
        // Both calls can wait on a busy host (an older one blocks in Close
        // until its output is read), so they run on a thread of their own,
        // as does the wait for the program to be on the console.
        let (release, close) = (self.api.release, self.api.close);
        let handle = self.handle.0;
        let program = self.program.take();
        let input = self.input.take();
        let closed = std::thread::Builder::new()
            .name("conpty-close".into())
            .spawn(move || {
                if let Some(program) = program {
                    wait_until_on_console(program.as_handle(), CONNECT_LIMIT);
                }
                drop(input);
                if let Some(release) = release {
                    // SAFETY: the handle is still open.
                    unsafe { release(HPCON(handle)) }.ok().ok();
                }
                // SAFETY: the handle is closed exactly once, here.
                unsafe { close(HPCON(handle)) }
            });
        if closed.is_err() {
            eprintln!("conpty: couldn't start a thread to close a pseudoconsole; leaking it");
        }
    }
}

/// A console size. The API's cells are `i16`, and a console has at least
/// one cell.
fn coord(cols: u16, rows: u16) -> COORD {
    let cells = |n: u16| n.clamp(1, i16::MAX as u16) as i16;
    COORD {
        X: cells(cols),
        Y: cells(rows),
    }
}

/// Waits until `program` is on its console or has exited, for at most
/// `limit`. One that never connects (a GUI program) takes all of it.
fn wait_until_on_console(program: BorrowedHandle<'_>, limit: Duration) {
    let deadline = Instant::now() + limit;
    let process = HANDLE(program.as_raw_handle());
    // SAFETY: the handle is open for the call.
    while unsafe { WaitForSingleObject(process, 0) } != WAIT_OBJECT_0
        && Instant::now() < deadline
    {
        if on_console(program) {
            return;
        }
        // Not a timed wait on the process: those take the system's timer
        // tick (often 15.6 ms), and the close should follow the connection
        // closely, before the program starts others.
        std::thread::sleep(CONNECT_POLL);
    }
}

/// Whether `process`, started by this process, has connected to its
/// console. Until it has, its console host is said to be this process. When
/// Windows doesn't say, it counts as connected, so a close isn't held up.
fn on_console(process: BorrowedHandle<'_>) -> bool {
    host_process(process).is_none_or(|host| host & !3 != std::process::id() as usize)
}

/// The pid of the console host `process` is on, or was on when it exited.
/// `None` before it has connected to one, and when Windows doesn't say.
/// It needs `PROCESS_QUERY_LIMITED_INFORMATION`.
pub fn console_host(process: BorrowedHandle<'_>) -> Option<u32> {
    let host = host_process(process).filter(|host| host & CONNECTED != 0)?;
    // The low bits are flags; process ids are multiples of four.
    u32::try_from(host & !3).ok().filter(|&host| host != 0)
}

/// What Windows says `process`'s console host is
/// ([`PROCESS_CONSOLE_HOST_PROCESS`]), flags included, or `None` when the
/// query fails.
fn host_process(process: BorrowedHandle<'_>) -> Option<usize> {
    let mut host: usize = 0;
    // SAFETY: the handle is open for the call, and `host` is as large as the
    // class's answer and outlives the call.
    let status = unsafe {
        NtQueryInformationProcess(
            HANDLE(process.as_raw_handle()),
            PROCESS_CONSOLE_HOST_PROCESS,
            (&raw mut host).cast(),
            std::mem::size_of::<usize>() as u32,
            std::ptr::null_mut(),
        )
    };
    status.is_ok().then_some(host)
}

/// A program started on a pseudoconsole.
pub struct Process {
    pid: u32,
    handle: OwnedHandle,
}

impl Process {
    pub fn pid(&self) -> u32 {
        self.pid
    }

    /// The process, to wait on or terminate. It is signalled at exit.
    pub fn handle(&self) -> BorrowedHandle<'_> {
        self.handle.as_handle()
    }

    /// The program's exit code, or `None` while it runs.
    pub fn exit_code(&self) -> io::Result<Option<u32>> {
        let handle = HANDLE(self.handle.as_raw_handle());
        // An exit code can equal STILL_ACTIVE, so ask whether it has exited.
        // SAFETY: the process handle is open while `self` lives.
        if unsafe { WaitForSingleObject(handle, 0) } != WAIT_OBJECT_0 {
            return Ok(None);
        }
        let mut code = 0;
        // SAFETY: as above; `code` outlives the call.
        unsafe { GetExitCodeProcess(handle, &mut code) }?;
        Ok(Some(code))
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read as _;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use windows::Win32::System::Pipes::CreatePipe;

    use super::super::api::bundle;
    use super::super::cmdline::launch_line;
    use super::*;

    fn pipe() -> (OwnedHandle, OwnedHandle) {
        let (mut read, mut write) = (HANDLE::default(), HANDLE::default());
        // SAFETY: CreatePipe fills both handles; they aren't inheritable.
        unsafe { CreatePipe(&mut read, &mut write, None, 0) }.unwrap();
        // SAFETY: both handles are new and owned by nothing else.
        unsafe {
            (
                OwnedHandle::from_raw_handle(read.0),
                OwnedHandle::from_raw_handle(write.0),
            )
        }
    }

    fn console(inbox: bool, input: &OwnedHandle, output: &OwnedHandle) -> Pseudoconsole {
        Pseudoconsole::create(inbox, 80, 24, input.as_handle(), output.as_handle()).unwrap()
    }

    struct Ran {
        output: String,
        code: u32,
        packaged: bool,
    }

    /// Runs `cmd.exe` with `args` on a new pseudoconsole over anonymous
    /// pipes, in the temporary directory, with this process's environment
    /// and `extra`, and collects its output to the pipe's end.
    fn run(inbox: bool, args: &[&str], extra: &[(&str, &str)]) -> Ran {
        let (input_read, _input_write) = pipe();
        let (output_read, output_write) = pipe();
        let mut console = console(inbox, &input_read, &output_write);
        drop((input_read, output_write));
        let packaged = console.is_packaged();

        let (done, output) = mpsc::channel();
        std::thread::spawn(move || {
            let mut all = Vec::new();
            std::fs::File::from(output_read).read_to_end(&mut all).ok();
            done.send(all).ok();
        });

        let set = |name: &str| extra.iter().any(|(set, _)| set.eq_ignore_ascii_case(name));
        let env: Vec<(String, String)> = std::env::vars_os()
            .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
            .filter(|(name, _)| !set(name))
            .chain(extra.iter().map(|(name, value)| (name.to_string(), value.to_string())))
            .collect();
        let args: Vec<String> = args.iter().map(|arg| arg.to_string()).collect();
        let process = console
            .start(
                &launch_line("cmd.exe", &args),
                Some(&std::env::temp_dir()),
                env.iter().map(|(name, value)| (name.as_str(), value.as_str())),
            )
            .unwrap();
        assert_ne!(process.pid(), 0);
        console.resize(100, 30).unwrap();

        let deadline = Instant::now() + Duration::from_secs(30);
        let code = loop {
            if let Some(code) = process.exit_code().unwrap() {
                break code;
            }
            assert!(Instant::now() < deadline, "the program never exited");
            std::thread::sleep(Duration::from_millis(20));
        };
        // Closing the pseudoconsole ends its host, which ends the output.
        drop(console);
        let output = output
            .recv_timeout(Duration::from_secs(30))
            .expect("the output pipe never ended");
        Ran {
            output: String::from_utf8_lossy(&output).into_owned(),
            code,
            packaged,
        }
    }

    #[test]
    fn a_program_runs_on_the_packaged_conpty() {
        let ran = run(false, &["/c", "echo hello"], &[]);
        assert!(ran.output.contains("hello"), "output: {:?}", ran.output);
        assert_eq!(ran.code, 0);
        assert_eq!(
            ran.packaged,
            bundle::FILES.is_some(),
            "the embedded ConPTY is the one used"
        );
    }

    #[test]
    fn a_program_runs_on_the_inbox_conpty() {
        let ran = run(true, &["/c", "echo hello"], &[]);
        assert!(ran.output.contains("hello"), "output: {:?}", ran.output);
        assert_eq!(ran.code, 0);
        assert!(!ran.packaged);
    }

    #[test]
    fn the_program_gets_the_environment_the_directory_and_its_exit_code() {
        let ran = run(
            false,
            &["/D", "/C", "echo brindle-%TEST_VAR%& cd & exit 3"],
            &[("TEST_VAR", "ok")],
        );
        assert!(ran.output.contains("brindle-ok"), "output: {:?}", ran.output);
        let temp = std::env::temp_dir().display().to_string().to_lowercase();
        let temp = temp.trim_end_matches('\\');
        assert!(
            ran.output.to_lowercase().contains(temp),
            "no {temp:?} in {:?}",
            ran.output
        );
        assert_eq!(ran.code, 3);
    }

    #[test]
    fn reparenting_to_no_window_is_harmless() {
        let (input_read, _input_write) = pipe();
        let (_output_read, output_write) = pipe();
        for inbox in [false, true] {
            console(inbox, &input_read, &output_write).reparent(0);
        }
    }

    #[test]
    fn sizes_fit_the_api() {
        let size = coord(0, u16::MAX);
        assert_eq!((size.X, size.Y), (1, i16::MAX));
    }
}
