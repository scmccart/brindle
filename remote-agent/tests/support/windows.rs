//! The Windows half of the tests' support: pipe connections, starting a
//! server process through `launch` with `--ready-handle`, process checks
//! through Toolhelp and process handles, and a window to own a session's
//! console window.

use std::fs;
use std::io::{self, Read, Write};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use brindle_remote_protocol::win;
use windows::Win32::Foundation::{
    ERROR_BROKEN_PIPE, ERROR_HANDLE_EOF, ERROR_IO_PENDING, ERROR_NO_DATA, ERROR_OPERATION_ABORTED,
    ERROR_PIPE_BUSY, ERROR_PIPE_NOT_CONNECTED, FILETIME, GENERIC_READ, GENERIC_WRITE, HANDLE,
    HANDLE_FLAG_INHERIT, HANDLE_FLAGS, HWND, SetHandleInformation, WAIT_OBJECT_0, WAIT_TIMEOUT,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_OVERLAPPED, FILE_SHARE_NONE, OPEN_EXISTING, ReadFile,
    SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT, WriteFile,
};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Pipes::WaitNamedPipeW;
use windows::Win32::System::Threading::{
    CreateEventW, GetProcessTimes, INFINITE, OpenProcess, PROCESS_ACCESS_RIGHTS,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE, SetEvent,
    TerminateProcess, WaitForMultipleObjects, WaitForSingleObject,
};
use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, DispatchMessageW, GW_OWNER, GetWindow, MSG, PM_REMOVE,
    PeekMessageW, WINDOW_EX_STYLE, WS_OVERLAPPED,
};
use windows::core::{PCWSTR, w};

use super::TIMEOUT;

fn raw(handle: &OwnedHandle) -> HANDLE {
    HANDLE(handle.as_raw_handle())
}

fn os(error: windows::core::Error) -> io::Error {
    io::Error::from_raw_os_error((error.code().0 & 0xffff) as i32)
}

fn code(error: &windows::core::Error) -> u32 {
    (error.code().0 & 0xffff) as u32
}

/// A manual-reset event, not yet set.
fn event() -> io::Result<OwnedHandle> {
    // SAFETY: no name and no security attributes; the handle is owned at
    // once.
    let event = unsafe { CreateEventW(None, true, false, None) }.map_err(os)?;
    // SAFETY: a new handle nothing else owns.
    Ok(unsafe { OwnedHandle::from_raw_handle(event.0) })
}

/// What the clones of a [`Stream`] share.
struct Pipe {
    handle: OwnedHandle,
    /// Set by [`close`]: every read and write ends.
    closed: OwnedHandle,
}

/// A connection to a server's endpoint: the client end of a named pipe,
/// opened overlapped, because a thread blocked reading a synchronous pipe
/// handle would stall another thread's write. Reads and writes block, each
/// on an event of its own. The pipe closes with its last clone.
pub struct Stream {
    pipe: Arc<Pipe>,
    /// How long this clone's reads wait.
    read_timeout: Mutex<Option<Duration>>,
}

/// What a read or a write was given.
enum Transfer<'a> {
    Read(&'a mut [u8]),
    Write(&'a [u8]),
}

impl Stream {
    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(Self {
            pipe: Arc::clone(&self.pipe),
            read_timeout: Mutex::new(None),
        })
    }

    /// A read then fails with `TimedOut` when nothing came for `limit`.
    pub fn set_read_timeout(&self, limit: Option<Duration>) -> io::Result<()> {
        *self.read_timeout.lock().unwrap() = limit;
        Ok(())
    }

    /// One overlapped read or write, waited for. The end of the pipe reads
    /// as 0 bytes.
    fn transfer(&self, transfer: Transfer<'_>, limit: Option<Duration>) -> io::Result<usize> {
        let reading = matches!(transfer, Transfer::Read(_));
        let aborted = || io::Error::new(io::ErrorKind::ConnectionAborted, "the stream was closed");
        let closed = raw(&self.pipe.closed);
        // SAFETY: the event is open.
        if unsafe { WaitForSingleObject(closed, 0) } == WAIT_OBJECT_0 {
            return if reading { Ok(0) } else { Err(aborted()) };
        }
        let done = event()?;
        let mut overlapped = OVERLAPPED {
            hEvent: raw(&done),
            ..Default::default()
        };
        let handle = raw(&self.pipe.handle);
        // SAFETY: the buffer and `overlapped` outlive the operation, which
        // has ended (or was cancelled and has ended) before this returns.
        let issued = unsafe {
            match transfer {
                Transfer::Read(buffer) => {
                    ReadFile(handle, Some(buffer), None, Some(&mut overlapped))
                }
                Transfer::Write(bytes) => {
                    WriteFile(handle, Some(bytes), None, Some(&mut overlapped))
                }
            }
        };
        let mut gave_up = None;
        match issued {
            Ok(()) => {}
            Err(error) if code(&error) == ERROR_IO_PENDING.0 => {
                let millis = limit.map_or(INFINITE, |limit| limit.as_millis().min(1 << 30) as u32);
                // SAFETY: both events are open.
                let waited =
                    unsafe { WaitForMultipleObjects(&[raw(&done), closed], false, millis) };
                if waited != WAIT_OBJECT_0 {
                    gave_up = Some(waited == WAIT_TIMEOUT);
                    // SAFETY: cancels this operation only; its end is
                    // waited for below.
                    let _ = unsafe { CancelIoEx(handle, Some(&overlapped)) };
                }
            }
            Err(error) => return end(reading, &error),
        }
        let mut transferred = 0u32;
        // SAFETY: waits for the operation `overlapped` names to end.
        match unsafe { GetOverlappedResult(handle, &overlapped, &mut transferred, true) } {
            Ok(()) => Ok(transferred as usize),
            Err(error) if code(&error) == ERROR_OPERATION_ABORTED.0 => match gave_up {
                Some(true) => Err(io::ErrorKind::TimedOut.into()),
                _ if reading => Ok(0),
                _ => Err(aborted()),
            },
            Err(error) => end(reading, &error),
        }
    }
}

/// How a failed read or write ends: the other side having closed the pipe
/// is the end of what can be read.
fn end(reading: bool, error: &windows::core::Error) -> io::Result<usize> {
    let gone = [
        ERROR_BROKEN_PIPE.0,
        ERROR_HANDLE_EOF.0,
        ERROR_PIPE_NOT_CONNECTED.0,
        ERROR_NO_DATA.0,
    ];
    if reading && gone.contains(&code(error)) {
        return Ok(0);
    }
    Err(io::Error::from_raw_os_error(code(error) as i32))
}

impl Read for Stream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let limit = *self.read_timeout.lock().unwrap();
        self.transfer(Transfer::Read(buffer), limit)
    }
}

impl Write for Stream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.transfer(Transfer::Write(bytes), None)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain([0]).collect()
}

/// Connects to the server listening at the pipe name `endpoint`, and
/// checks that it runs as this user, as every client does.
pub fn connect(endpoint: &Path) -> io::Result<Stream> {
    let name = wide(endpoint);
    let handle = loop {
        // SAFETY: the name is NUL-terminated and outlives the call.
        let opened = unsafe {
            CreateFileW(
                PCWSTR(name.as_ptr()),
                GENERIC_READ.0 | GENERIC_WRITE.0,
                FILE_SHARE_NONE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION,
                None,
            )
        };
        match opened {
            // SAFETY: a new handle nothing else owns.
            Ok(handle) => break unsafe { OwnedHandle::from_raw_handle(handle.0) },
            // The server makes its next instance when it accepts.
            Err(error) if code(&error) == ERROR_PIPE_BUSY.0 => {
                // SAFETY: as above.
                let _ = unsafe { WaitNamedPipeW(PCWSTR(name.as_ptr()), 5000) };
            }
            Err(error) => return Err(os(error)),
        }
    };
    win::check_server(&handle)?;
    Ok(Stream {
        pipe: Arc::new(Pipe {
            handle,
            closed: event()?,
        }),
        read_timeout: Mutex::new(None),
    })
}

/// Ends the reads and writes of every clone of `stream`, as its process
/// exiting or being killed would. The server sees the connection close when
/// the last clone is dropped.
pub fn close(stream: &Stream) {
    // SAFETY: the event is open.
    let _ = unsafe { SetEvent(raw(&stream.pipe.closed)) };
}

/// The temp directory is the user's own already.
pub fn private_dir(_dir: &Path) {}

/// The arguments a holder starts a real core with
/// (`BRINDLE_TEST_HOLDER_CORE_ARGS`).
pub const SERVE_ARGS: &str = "serve --ready-handle {ready_handle}";

/// A handle to the running process `pid`, with `access` and the right to
/// wait for it.
fn open(pid: u32, access: PROCESS_ACCESS_RIGHTS) -> Option<OwnedHandle> {
    let access = access | PROCESS_SYNCHRONIZE;
    // SAFETY: opens a process by pid; the handle is owned at once.
    let process = unsafe { OpenProcess(access, false, pid) }.ok()?;
    // SAFETY: a new handle nothing else owns.
    let process = unsafe { OwnedHandle::from_raw_handle(process.0) };
    // SAFETY: the handle is open.
    (unsafe { WaitForSingleObject(raw(&process), 0) } == WAIT_TIMEOUT).then_some(process)
}

/// Whether `pid` is a process that hasn't exited.
pub fn alive(pid: u32) -> bool {
    open(pid, PROCESS_QUERY_LIMITED_INFORMATION).is_some()
}

/// Terminates `pid`, with exit code 1, and waits for it to be gone.
pub fn kill(pid: u32) {
    if let Some(process) = open(pid, PROCESS_TERMINATE) {
        // SAFETY: the handle is open and may terminate the process.
        unsafe {
            let _ = TerminateProcess(raw(&process), 1);
            WaitForSingleObject(raw(&process), TIMEOUT.as_millis() as u32);
        }
    }
}

/// The file name of `pid`'s image, while it runs.
fn image_name(pid: u32) -> Option<String> {
    // An exited process can still be opened while a handle to it is open.
    let _running = open(pid, PROCESS_QUERY_LIMITED_INFORMATION)?;
    let path = win::image_path_of(pid).ok()?;
    Some(path.file_name()?.to_string_lossy().into_owned())
}

/// Whether `pid` runs `brindle-remote` (or a test's copy of it): a pid a
/// log names may be another process's by now.
fn is_server(pid: u32) -> bool {
    image_name(pid).is_some_and(|name| name.to_ascii_lowercase().starts_with("brindle-remote"))
}

/// Kills the server process `pid`, if it still runs, and what it started
/// that is still running then: a holder's console hosts and programs. They
/// end with their holder, except one that was only just starting.
pub fn kill_server(pid: u32) {
    if !is_server(pid) {
        return;
    }
    let processes = processes();
    let mut started = vec![pid];
    let mut next = 0;
    while next < started.len() {
        // A pid is used again: only a process younger than the one its
        // parent pid names was started by it.
        let parent = started[next];
        let since = created(parent);
        let children = processes.iter().filter(|(child, of, _)| {
            *of == parent && since.is_some() && created(*child) >= since
        });
        let children = children.map(|(child, ..)| *child);
        let new: Vec<u32> = children.filter(|child| !started.contains(child)).collect();
        started.extend(new);
        next += 1;
    }
    for process in started {
        kill(process);
    }
}

/// When the running process `pid` was created, as a `FILETIME`'s count.
fn created(pid: u32) -> Option<u64> {
    let process = open(pid, PROCESS_QUERY_LIMITED_INFORMATION)?;
    let (mut creation, mut exit) = (FILETIME::default(), FILETIME::default());
    let (mut kernel, mut user) = (FILETIME::default(), FILETIME::default());
    // SAFETY: the handle is open and the four times are writable.
    unsafe { GetProcessTimes(raw(&process), &mut creation, &mut exit, &mut kernel, &mut user) }
        .ok()?;
    Some(u64::from(creation.dwHighDateTime) << 32 | u64::from(creation.dwLowDateTime))
}

/// Every running process: its pid, its parent's pid and its image's file
/// name.
fn processes() -> Vec<(u32, u32, String)> {
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
    let mut more = unsafe { Process32FirstW(raw(&snapshot), &mut entry) }.is_ok();
    while more {
        let name = entry.szExeFile.split(|unit| *unit == 0).next().unwrap();
        found.push((
            entry.th32ProcessID,
            entry.th32ParentProcessID,
            String::from_utf16_lossy(name),
        ));
        // SAFETY: as above.
        more = unsafe { Process32NextW(raw(&snapshot), &mut entry) }.is_ok();
    }
    found
}

/// The pid that started `pid`, which may have exited since (and may be
/// another process's pid by now).
pub fn parent(pid: u32) -> Option<u32> {
    let processes = processes();
    let found = processes.iter().find(|(process, ..)| *process == pid);
    found.map(|(_, parent, _)| *parent)
}

/// The running children of `pid` whose image is `name` (`ping` or
/// `PING.EXE`).
pub fn children_named(pid: u32, name: &str) -> Vec<u32> {
    let wanted = name.to_ascii_lowercase();
    let wanted = wanted.strip_suffix(".exe").unwrap_or(&wanted);
    let is_named = |image: &str| {
        let image = image.to_ascii_lowercase();
        image.strip_suffix(".exe").unwrap_or(&image) == wanted
    };
    processes()
        .into_iter()
        .filter(|(child, parent, image)| *parent == pid && is_named(image) && alive(*child))
        .map(|(child, ..)| child)
        .collect()
}

/// The running `brindle-remote <role>` processes (`serve` or `hold`) of the
/// server directory `dir`. Another process's environment can't be read
/// here, so they are the pids the directory's logs name (`core[pid]` in
/// `core.log`, `holder[pid]` in each holder's) that still run the helper.
pub fn server_processes(dir: &Path, role: &str) -> Vec<u32> {
    let (prefix, tag) = match role {
        "serve" => ("core", " core["),
        _ => ("holder-", " holder["),
    };
    let mut pids: Vec<u32> = Vec::new();
    for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !(name.starts_with(prefix) && name.ends_with(".log")) {
            continue;
        }
        let log = fs::read(entry.path()).unwrap_or_default();
        for line in String::from_utf8_lossy(&log).lines() {
            let pid = line
                .split_once(tag)
                .and_then(|(_, rest)| rest.split_once("]:"));
            if let Some(pid) = pid.and_then(|(pid, _)| pid.parse().ok())
                && !pids.contains(&pid)
            {
                pids.push(pid);
            }
        }
    }
    pids.retain(|&pid| is_server(pid));
    pids.sort_unstable();
    pids
}

/// Held while a handle of this process can be inherited: one start at a
/// time, so that no other start's process gets that handle.
static STARTING: Mutex<()> = Mutex::new(());

/// Runs `bin launch <role> --ready-handle N` for `dir` and waits for the
/// role's ready byte. Fails, with what the role said before it had its
/// log, if it ends before being ready.
pub fn start(bin: &Path, role: &str, dir: &Path, env: &[(&str, &str)]) -> io::Result<()> {
    let (mut read, write) = io::pipe()?;
    let handle = HANDLE(write.as_raw_handle());
    let child = {
        let _starting = STARTING
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        // SAFETY: the write end is open; the trampoline and the role it
        // names inherit it under the same value.
        unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT.0, HANDLE_FLAG_INHERIT) }
            .map_err(os)?;
        let child = Command::new(bin)
            .args(["launch", role, "--ready-handle"])
            .arg((handle.0 as usize).to_string())
            .env("BRINDLE_SERVER_DIR", dir)
            .envs(env.iter().copied())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn();
        drop(write);
        child?
    };
    // The byte, or the end of the pipe when the role is gone.
    let (sender, ready) = mpsc::channel();
    thread::spawn(move || {
        let mut byte = [0u8; 1];
        let _ = sender.send(matches!(read.read(&mut byte), Ok(1)));
    });
    match ready.recv_timeout(TIMEOUT) {
        Ok(true) => {}
        // The role is gone, and with it the starter's stderr it held.
        Ok(false) => {
            let output = child.wait_with_output()?;
            return Err(io::Error::other(format!(
                "{role} wasn't ready ({}): {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        // It is still there, holding the starter's stderr: there is no end
        // of that to wait for.
        Err(_) => return Err(io::Error::other(format!("{role} wasn't ready in time"))),
    }
    // Ends once the role has its log, and so has closed what it inherited.
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err(io::Error::other(format!(
            "the starter exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(())
}

/// Hands `stream` to a `ping` process, which inherits its handle, so killing
/// that process closes the connection as a client dying does. The caller
/// drops its own side.
pub fn hand_to_process(stream: &Stream) -> Child {
    let handle = raw(&stream.pipe.handle);
    let _starting = STARTING
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    // SAFETY: the handle is open; it can be inherited only while the lock
    // is held.
    unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT.0, HANDLE_FLAG_INHERIT) }.unwrap();
    let child = Command::new("ping.exe")
        .args(["-n", "1000", "127.0.0.1"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    // SAFETY: as above.
    unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT.0, HANDLE_FLAGS(0)) }.unwrap();
    child.unwrap()
}

/// The working set of `pid`, in KiB.
pub fn working_set_kib(pid: u32) -> u64 {
    let process = open(pid, PROCESS_QUERY_LIMITED_INFORMATION).expect("the process");
    let mut counters = PROCESS_MEMORY_COUNTERS::default();
    let size = std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32;
    // SAFETY: the handle is open, and `counters` has `size` bytes.
    unsafe { GetProcessMemoryInfo(raw(&process), &mut counters, size) }.unwrap();
    counters.WorkingSetSize as u64 / 1024
}

/// A hidden top-level window of this thread's, destroyed on drop.
pub struct Window(HWND);

impl Window {
    pub fn new() -> Self {
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

    pub fn id(&self) -> u64 {
        self.0.0 as usize as u64
    }

    /// Handles what other threads sent this thread's windows: a host
    /// that makes one an owner may wait for it to answer.
    pub fn pump() {
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
pub fn window_owner(window: u64) -> u64 {
    let window = HWND(window as usize as *mut std::ffi::c_void);
    // SAFETY: any window handle may be asked about.
    let owner = unsafe { GetWindow(window, GW_OWNER) };
    owner.map_or(0, |owner| owner.0 as usize as u64)
}
