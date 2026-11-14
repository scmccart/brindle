//! What the client needs from Windows: the core's named pipe ([`Stream`],
//! in `pipe.rs`), the starter, and a process's image and end.
//!
//! **Starting a core.** A server role never runs from the installed helper,
//! whose image a role running for days would keep locked against every
//! upgrade and rebuild. The helper is copied to
//! `<copies>\<build id>\brindle-remote.exe` (`win::ensure_helper_copy`),
//! the other builds' copies are removed where nothing runs from them, and
//! the copy is run as `launch serve --ready-handle N`. That is the helper's
//! trampoline: it starts `serve` detached, outside Brindle's job and
//! process tree, and exits at once. `N` is the write end of a pipe the core
//! writes one byte to once it listens.
//!
//! The trampoline is created with a `PROC_THREAD_ATTRIBUTE_HANDLE_LIST`, so
//! of Brindle's handles it gets only that write end and the pipe that is its
//! stderr. The core inherits both from it, and closes the stderr pipe when
//! it has its log.

use std::io::{self, Read as _};
use std::marker::PhantomData;
use std::os::windows::ffi::OsStrExt as _;
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use anyhow::{Context as _, Result};
use parking_lot::Mutex;
use windows::Win32::Foundation::{
    HANDLE, HANDLE_FLAG_INHERIT, HANDLE_FLAGS, SetHandleInformation, WAIT_OBJECT_0,
};
use windows::Win32::System::Threading::{
    CREATE_NO_WINDOW, CreateProcessW, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT,
    GetExitCodeProcess, InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST, OpenProcess,
    PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROCESS_INFORMATION, PROCESS_TERMINATE, STARTF_USESTDHANDLES,
    STARTUPINFOEXW, TerminateProcess, UpdateProcThreadAttribute, WaitForSingleObject,
};
use windows::core::{PCWSTR, PWSTR};

pub use super::pipe::{Stream, address, connect};
#[cfg(test)]
pub use super::pipe::Listener;
use super::{Endpoint, Start};
use crate::remote::protocol::win;

/// How long the trampoline may take to exit, and its stderr to end, once
/// the core has signalled. Both happen before the signal, bar a core that
/// was started inside Brindle's job and says so late.
const SETTLE: Duration = Duration::from_secs(1);

fn os(error: windows::core::Error) -> io::Error {
    io::Error::from_raw_os_error((error.code().0 & 0xffff) as i32)
}

fn wide(text: &std::ffi::OsStr) -> Vec<u16> {
    text.encode_wide().chain([0]).collect()
}

/// Runs the per-build copy of the helper as `launch serve --ready-handle N`
/// and waits up to `limit` for the core's ready byte.
pub fn start(endpoint: &Endpoint, limit: Duration) -> Result<Start> {
    let copy = win::ensure_helper_copy(&endpoint.helper, &endpoint.copies).with_context(|| {
        format!(
            "terminal server didn't start: can't copy {} to {}",
            endpoint.helper.display(),
            endpoint.copies.display()
        )
    })?;
    // The copy is `<copies>\<build>\brindle-remote.exe`.
    if let Some(build) = copy.parent().and_then(Path::file_name).and_then(|name| name.to_str()) {
        win::prune_helper_copies(&endpoint.copies, build);
    }
    let (ready_read, ready_write) = io::pipe().context("starting the terminal server: pipe")?;
    let (stderr_read, stderr_write) = io::pipe().context("starting the terminal server: pipe")?;
    let ready_handle = HANDLE(ready_write.as_raw_handle());
    let line = format!("\"{}\" launch serve --ready-handle {}", copy.display(), ready_handle.0 as usize);
    let process = create(&copy, &line, ready_handle, HANDLE(stderr_write.as_raw_handle()))
        .with_context(|| format!("terminal server didn't start: can't run {}", copy.display()))?;
    // Only the trampoline and the core hold the write ends now.
    drop((ready_write, stderr_write));

    // The byte, or the end of the pipe when the core is gone.
    let (signal, signalled) = mpsc::channel();
    let mut ready_read = ready_read;
    spawn_reader("server-ready", move || {
        let mut byte = [0u8; 1];
        signal.send(matches!(ready_read.read(&mut byte), Ok(1))).ok();
    })?;
    let said = Arc::new(Mutex::new(Vec::new()));
    let (finished, stderr_ended) = mpsc::channel();
    let saying = said.clone();
    let mut stderr_read = stderr_read;
    spawn_reader("server-stderr", move || {
        let mut buffer = [0u8; 1024];
        while let Ok(read @ 1..) = stderr_read.read(&mut buffer) {
            saying.lock().extend_from_slice(&buffer[..read]);
        }
        finished.send(()).ok();
    })?;

    let ready = signalled.recv_timeout(limit).ok();
    // By the core's doing (its byte, or the end of the pipe as it and the
    // trampoline exited), not by the time limit.
    let signalled = ready.is_some();
    let mut status = "still running".to_string();
    if signalled {
        // A core that never signals keeps the stderr pipe, so its end is
        // only waited for here.
        stderr_ended.recv_timeout(SETTLE).ok();
        // SAFETY: the process handle is open.
        if unsafe { WaitForSingleObject(HANDLE(process.as_raw_handle()), SETTLE.as_millis() as u32) }
            == WAIT_OBJECT_0
        {
            let mut code = 0u32;
            // SAFETY: as above; `code` is writable.
            if unsafe { GetExitCodeProcess(HANDLE(process.as_raw_handle()), &mut code) }.is_ok() {
                status = format!("exit code: {code}");
            }
        }
    }
    let stderr = String::from_utf8_lossy(&said.lock()).into_owned();
    let ready = ready == Some(true);
    if ready {
        // The trampoline's one line when Brindle's job forbids breakaway.
        for line in stderr.lines().filter(|line| !line.trim().is_empty()) {
            log::warn!("starting the terminal server: {}", line.trim());
        }
    }
    Ok(Start { ready, signalled, status, stderr })
}

fn spawn_reader(name: &str, read: impl FnOnce() + Send + 'static) -> Result<()> {
    std::thread::Builder::new().name(name.into()).spawn(read).context("starting a thread")?;
    Ok(())
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
        let list = Self { list, _memory: memory, _handles: PhantomData };
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

/// Creates the process `image` with the command line `line`, no window, no
/// stdin or stdout, `stderr` as its stderr, and Brindle's environment and
/// directory. It inherits `ready` and `stderr` and no other handle.
fn create(image: &Path, line: &str, ready: HANDLE, stderr: HANDLE) -> io::Result<OwnedHandle> {
    let handles = [ready, stderr];
    // A listed handle must be inheritable. Until this process closes them,
    // right after the create, another thread's `Command` could pass them on
    // too; the waits above are bounded for that.
    for handle in handles {
        // SAFETY: the caller's open handle.
        unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT.0, HANDLE_FLAG_INHERIT) }.map_err(os)?;
    }
    let list = HandleList::new(&handles)?;
    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = size_of::<STARTUPINFOEXW>() as u32;
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.StartupInfo.hStdError = stderr;
    startup.lpAttributeList = list.list;
    let image = wide(image.as_os_str());
    // The call may write to the line.
    let mut line = wide(line.as_ref());
    let mut info = PROCESS_INFORMATION::default();
    // SAFETY: every pointer is valid for the call, the line is writable,
    // and the startup info is the extended one, as the flags say.
    let created = unsafe {
        CreateProcessW(
            PCWSTR(image.as_ptr()),
            Some(PWSTR(line.as_mut_ptr())),
            None,
            None,
            true,
            // The helper is a console program, and Brindle has no console.
            CREATE_NO_WINDOW | EXTENDED_STARTUPINFO_PRESENT,
            None,
            PCWSTR::null(),
            &startup.StartupInfo,
            &mut info,
        )
    };
    for handle in handles {
        // SAFETY: as above. The write ends are closed next anyway.
        let _ = unsafe { SetHandleInformation(handle, HANDLE_FLAG_INHERIT.0, HANDLE_FLAGS(0)) };
    }
    created.map_err(os)?;
    // SAFETY: both handles are new and this process's own.
    unsafe {
        drop(OwnedHandle::from_raw_handle(info.hThread.0));
        Ok(OwnedHandle::from_raw_handle(info.hProcess.0))
    }
}

/// The image of the running process `pid` (`QueryFullProcessImageNameW`):
/// a core's is its per-build copy, which keeps the helper's file time.
pub fn image_of(pid: u32) -> io::Result<PathBuf> {
    win::image_path_of(pid)
}

/// Ends the process `pid`, a helper command this process started.
pub fn kill(pid: u32) {
    // SAFETY: opens a process by pid; the handle is owned at once.
    let Ok(process) = (unsafe { OpenProcess(PROCESS_TERMINATE, false, pid) }) else {
        return;
    };
    // SAFETY: a new handle nothing else owns, which may end the process.
    unsafe {
        let process = OwnedHandle::from_raw_handle(process.0);
        let _ = TerminateProcess(HANDLE(process.as_raw_handle()), 1);
    }
}
