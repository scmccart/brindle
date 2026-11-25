//! A session's program on plain pipes for the holder (H1 version 3's
//! `SPAWN_PIPES`): its standard input, output and error, each an event-loop
//! [`Source`] read and written through its owner (`process R8`), and the
//! [`Program`] and its group, shared with the PTY sessions, for its exit and
//! hang-up.
//!
//! Declared with the same items on every platform so the holder names none
//! of their types (`ports R5`). Here there is no pseudoconsole:
//!
//! - **Pipes:** three one-way pipes named as a PTY session's are
//!   (`server_dir::pty_pipe`, each name's first and only instance, with the
//!   server pipes' security; `pty::pipe`). The holder's ends are overlapped,
//!   each a [`Source`] on the loop. The program's are synchronous and made
//!   inheritable, and it inherits them alone: they are its standard handles
//!   (`STARTF_USESTDHANDLES`) and the whole of its
//!   `PROC_THREAD_ATTRIBUTE_HANDLE_LIST` (`piped::ChildEnds::handle_list`).
//! - **Creation:** `piped::CREATION_FLAGS` (no console window, a process
//!   group of its own, a Unicode environment, the extended startup info).
//!   The program, its command line and its environment are `piped::launch`'s:
//!   the environment exactly as given, the program found in its `PATH`, and
//!   a bare name that is a batch file refused with the `cmd /c` form to use.
//! - **The job:** each session's program starts in a job of its own
//!   (`PROC_THREAD_ATTRIBUTE_JOB_LIST`, so it is in the job before it runs),
//!   with `piped::JOB_LIMITS` (`KILL_ON_JOB_CLOSE`): what it starts stays in
//!   the job, a hang-up ends them all (`TerminateJobObject`, [`Group`]), and
//!   they end with the holder.
//! - **Exit** is watched as a PTY program's is (`Waker::wake_on_exit`).
//!
//! [`Group`]: super::pty::Group

use std::collections::BTreeMap;
use std::ffi::c_void;
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;

use windows::Win32::Foundation::{
    CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE, HANDLE_FLAG_INHERIT, SetHandleInformation,
};
use windows::Win32::Storage::FileSystem::{
    FILE_READ_ATTRIBUTES, FILE_WRITE_ATTRIBUTES, PIPE_ACCESS_INBOUND, PIPE_ACCESS_OUTBOUND,
};
use windows::Win32::System::JobObjects::{
    CreateJobObjectW, JOB_OBJECT_LIMIT, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JobObjectExtendedLimitInformation, SetInformationJobObject, TerminateJobObject,
};
use windows::Win32::System::Threading::{
    CreateProcessW, DeleteProcThreadAttributeList, InitializeProcThreadAttributeList,
    LPPROC_THREAD_ATTRIBUTE_LIST, PROC_THREAD_ATTRIBUTE_HANDLE_LIST,
    PROC_THREAD_ATTRIBUTE_JOB_LIST, PROCESS_CREATION_FLAGS, PROCESS_INFORMATION,
    STARTUPINFOEXW, STARTUPINFOW_FLAGS, UpdateProcThreadAttribute,
};
use windows::core::{PCWSTR, PWSTR};

use super::super::piped::{self as decide, ChildEnds, CREATION_FLAGS, JOB_LIMITS, STARTUP_FLAGS};
use super::event_loop::{Source, os_error, win32};
use super::pipe::Pipe;
use super::pty::{Program, pipe};
use crate::conpty::Process;
use crate::conpty::cmdline::environment_block;

/// The exit code of the processes a hang-up ends.
const ENDED: u32 = 1;

/// What to run on pipes. `env` is the program's whole environment.
pub struct PipeCommand<'a> {
    pub command: &'a str,
    pub args: &'a [String],
    pub cwd: Option<&'a str>,
    pub env: &'a BTreeMap<String, String>,
}

/// The holder's ends of a program's three pipes, overlapped: reads and
/// writes never block.
pub struct Piped {
    /// Where the program's stdin is written; `None` once closed.
    input: Option<Pipe>,
    output: Pipe,
    error: Pipe,
}

impl Piped {
    /// Writable while the program's stdin takes input; `None` once it was
    /// closed ([`Piped::close_input`]).
    pub fn input_source(&self) -> Option<Source> {
        self.input.as_ref().map(Pipe::source)
    }

    /// Readable while the program's stdout has output waiting.
    pub fn output_source(&self) -> Source {
        self.output.source()
    }

    /// Readable while the program's stderr has bytes waiting.
    pub fn error_source(&self) -> Source {
        self.error.source()
    }

    /// Writes to the program's stdin, as much as is taken; `WouldBlock`
    /// when none is, `BrokenPipe` once it is closed.
    pub fn write_input(&mut self, bytes: &[u8]) -> io::Result<usize> {
        match &self.input {
            Some(input) => input.write(bytes),
            None => Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "the program's stdin is closed",
            )),
        }
    }

    /// Closes the program's stdin, its first sign to end. What wasn't
    /// taken yet is dropped: the holder drops the session's queued input
    /// on a hang-up too.
    pub fn close_input(&mut self) {
        self.input = None;
    }

    /// Reads the program's stdout. `Ok(0)` means there will be no more;
    /// `WouldBlock` that none is waiting.
    pub fn read_output(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.output.read(buffer)
    }

    /// Reads the program's stderr, as [`Piped::read_output`] reads stdout.
    pub fn read_error(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.error.read(buffer)
    }
}

/// A pipe session's job: its processes end when the last handle to it
/// closes (`KILL_ON_JOB_CLOSE`), or when it is ended.
pub(super) struct Job(OwnedHandle);

impl Job {
    fn new() -> io::Result<Self> {
        // SAFETY: an unnamed job with default security.
        let job = unsafe { CreateJobObjectW(None, PCWSTR::null()) }
            .map_err(|error| os_error(win32(&error)))?;
        // SAFETY: the handle is new and owned by nothing else.
        let job = Self(unsafe { OwnedHandle::from_raw_handle(job.0) });
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT(JOB_LIMITS);
        // SAFETY: the handle is open, and `limits` is the class's struct and
        // outlives the call.
        unsafe {
            SetInformationJobObject(
                job.raw(),
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        }
        .map_err(|error| os_error(win32(&error)))?;
        Ok(job)
    }

    fn raw(&self) -> HANDLE {
        HANDLE(self.0.as_raw_handle())
    }

    /// Ends every process in the job. One that has none left, or whose
    /// processes are ending already, is no error.
    pub(super) fn terminate(&self) {
        // SAFETY: the handle is open and may end the job's processes.
        let _ = unsafe { TerminateJobObject(self.raw(), ENDED) };
    }
}

/// Makes `handle` inheritable, so a child named in its handle list gets it.
fn inheritable(handle: &OwnedHandle) -> io::Result<()> {
    // SAFETY: the handle is open; only its inherit flag changes.
    unsafe {
        SetHandleInformation(
            HANDLE(handle.as_raw_handle()),
            HANDLE_FLAG_INHERIT.0,
            HANDLE_FLAG_INHERIT,
        )
    }
    .map_err(|error| os_error(win32(&error)))
}

/// A process attribute list of `count` attributes, deleted on drop. The
/// values it is given must outlive it.
struct AttributeList {
    _buffer: Vec<u8>,
    list: LPPROC_THREAD_ATTRIBUTE_LIST,
}

impl AttributeList {
    fn new(count: u32) -> io::Result<Self> {
        let mut size = 0;
        // SAFETY: the first call only reports the size the list needs.
        let _ = unsafe { InitializeProcThreadAttributeList(None, count, None, &mut size) };
        let mut buffer = vec![0u8; size];
        let list = LPPROC_THREAD_ATTRIBUTE_LIST(buffer.as_mut_ptr().cast());
        // SAFETY: `buffer` has the reported size and outlives the list.
        unsafe { InitializeProcThreadAttributeList(Some(list), count, None, &mut size) }
            .map_err(|error| os_error(win32(&error)))?;
        Ok(Self {
            _buffer: buffer,
            list,
        })
    }

    /// Sets `attribute` to the array `values`, which must outlive the list.
    fn set(&mut self, attribute: u32, values: &[HANDLE]) -> io::Result<()> {
        // SAFETY: the list was initialized, and the array outlives it (the
        // caller keeps it until the process is created).
        unsafe {
            UpdateProcThreadAttribute(
                self.list,
                0,
                attribute as usize,
                Some(values.as_ptr().cast::<c_void>()),
                std::mem::size_of_val(values),
                None,
                None,
            )
        }
        .map_err(|error| os_error(win32(&error)))
    }
}

impl Drop for AttributeList {
    fn drop(&mut self) {
        // SAFETY: initialized in `new`, deleted once.
        unsafe { DeleteProcThreadAttributeList(self.list) };
    }
}

/// Starts `request`'s program with its standard streams on pipes, in a job
/// of its own.
pub fn spawn_piped(request: &PipeCommand) -> io::Result<(Piped, Program)> {
    let launch = decide::launch(request.command, request.args, request.env, |file| {
        Path::new(file).is_file()
    })
    .map_err(|error| io::Error::new(io::ErrorKind::NotFound, error.to_string()))?;

    // The program's ends may also be asked about and have their mode set,
    // as runtimes that wrap their standard handles do.
    let (input, child_input) =
        pipe(PIPE_ACCESS_OUTBOUND, GENERIC_READ.0 | FILE_WRITE_ATTRIBUTES.0)?;
    let (output, child_output) =
        pipe(PIPE_ACCESS_INBOUND, GENERIC_WRITE.0 | FILE_READ_ATTRIBUTES.0)?;
    let (error, child_error) =
        pipe(PIPE_ACCESS_INBOUND, GENERIC_WRITE.0 | FILE_READ_ATTRIBUTES.0)?;
    for end in [&child_input, &child_output, &child_error] {
        inheritable(end)?;
    }
    let raw = |end: &OwnedHandle| HANDLE(end.as_raw_handle());
    let ends = ChildEnds {
        input: raw(&child_input),
        output: raw(&child_output),
        error: raw(&child_error),
    };
    let handles = ends.handle_list();
    let job = Job::new()?;
    let jobs = [job.raw()];

    let mut attributes = AttributeList::new(2)?;
    attributes.set(PROC_THREAD_ATTRIBUTE_HANDLE_LIST, &handles)?;
    attributes.set(PROC_THREAD_ATTRIBUTE_JOB_LIST, &jobs)?;

    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTUPINFOW_FLAGS(STARTUP_FLAGS);
    startup.StartupInfo.hStdInput = ends.input;
    startup.StartupInfo.hStdOutput = ends.output;
    startup.StartupInfo.hStdError = ends.error;
    startup.lpAttributeList = attributes.list;

    let application: Vec<u16> = launch.application.encode_utf16().chain(Some(0)).collect();
    let mut line: Vec<u16> = launch.line.encode_utf16().chain(Some(0)).collect();
    let env = environment_block(
        launch
            .environment
            .iter()
            .map(|(name, value)| (name.as_str(), value.as_str())),
    );
    let cwd: Option<Vec<u16>> = request
        .cwd
        .map(|dir| dir.encode_utf16().chain(Some(0)).collect());
    let mut info = PROCESS_INFORMATION::default();
    // SAFETY: every pointer is valid for the call, the attribute list's
    // values (`handles`, `jobs`) included; the command line buffer is
    // writable, as CreateProcessW requires. Only the handle list is
    // inherited, though inheritance is on.
    unsafe {
        CreateProcessW(
            PCWSTR(application.as_ptr()),
            Some(PWSTR(line.as_mut_ptr())),
            None,
            None,
            true,
            PROCESS_CREATION_FLAGS(CREATION_FLAGS),
            Some(env.as_ptr().cast()),
            cwd.as_ref()
                .map_or(PCWSTR::null(), |dir| PCWSTR(dir.as_ptr())),
            &startup.StartupInfo,
            &mut info,
        )
    }
    .map_err(|error| os_error(win32(&error)))?;
    drop(attributes);
    // The program has its own copies: the holder's reads end when the
    // program and what it started have closed theirs.
    drop((child_input, child_output, child_error));
    // SAFETY: the thread handle is ours and unused.
    let _ = unsafe { CloseHandle(info.hThread) };
    // SAFETY: the process handle is ours.
    let process = unsafe { OwnedHandle::from_raw_handle(info.hProcess.0) };
    Ok((
        Piped {
            input: Some(Pipe::of(None, Some(input))),
            output: Pipe::of(Some(output), None),
            error: Pipe::of(Some(error), None),
        },
        Program::in_job(Process::new(info.dwProcessId, process), job),
    ))
}

#[cfg(test)]
mod tests {
    use windows::Win32::System::JobObjects::JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
    use windows::Win32::System::Threading::{
        CREATE_NEW_PROCESS_GROUP, CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT,
        EXTENDED_STARTUPINFO_PRESENT, STARTF_USESTDHANDLES,
    };

    use super::*;

    #[test]
    fn the_neutral_numbers_are_windows_own() {
        assert_eq!(
            CREATION_FLAGS,
            (CREATE_NO_WINDOW
                | CREATE_NEW_PROCESS_GROUP
                | CREATE_UNICODE_ENVIRONMENT
                | EXTENDED_STARTUPINFO_PRESENT)
                .0
        );
        assert_eq!(STARTUP_FLAGS, STARTF_USESTDHANDLES.0);
        assert_eq!(JOB_LIMITS, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE.0);
    }

    #[test]
    fn a_program_missing_from_the_path_fails_to_start() {
        let env = BTreeMap::from([("PATH".to_string(), r"C:\nonexistent".to_string())]);
        let result = spawn_piped(&PipeCommand {
            command: "brindle-no-such-agent",
            args: &[],
            cwd: None,
            env: &env,
        });
        let Err(error) = result else {
            panic!("a missing program started");
        };
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
        assert!(error.to_string().contains("brindle-no-such-agent"), "{error}");
    }
}
