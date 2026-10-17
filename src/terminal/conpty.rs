//! Brindle's own pseudoconsole on Windows.
//!
//! alacritty's ConPTY backend creates the pseudoconsole with no flags, finds
//! `conpty.dll` through the standard DLL search, and keeps its pipes
//! private. Brindle creates the pseudoconsole itself instead, and hands its
//! pieces to alacritty's `EventLoop` through `pty::MarkedPty`:
//!
//! - [`Api`] is resolved once. It is the packaged ConPTY embedded in the
//!   binary (`build.rs`), loaded by absolute path, either from beside the exe
//!   or from an extracted copy, and only when its bytes match the embedded
//!   ones; otherwise the ConPTY built into Windows, with one warning.
//!   `BRINDLE_CONPTY=inbox` forces the built-in one.
//! - [`spawn`] creates the pseudoconsole with wcwidth glyph widths (the way
//!   alacritty's grid counts cells) and starts the program on it.
//! - [`PipeReader`] and [`PipeWriter`] do blocking pipe I/O on their own
//!   threads and wake the event loop with IOCP completion packets.
//!   alacritty polls level-triggered, so the reader posts again whenever a
//!   read leaves data behind.
//! - [`ChildWatcher`] reports the program's exit.
//! - Dropping the [`Pseudoconsole`] releases it and closes it on a
//!   short-lived thread, while the reader thread drains output to EOF, so
//!   nothing waits on a host that blocks in `ClosePseudoConsole`.

use std::collections::VecDeque;
use std::ffi::{OsString, c_void};
use std::io::{self, Read, Write};
use std::os::windows::ffi::OsStrExt as _;
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
use std::os::windows::process::ExitStatusExt as _;
use std::path::{Path, PathBuf};
use std::process::ExitStatus;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, mpsc};

use alacritty_terminal::event::WindowSize;
use alacritty_terminal::tty::ChildEvent;
use polling::os::iocp::{CompletionPacket, PollerIocpExt as _};
use polling::{Event, Poller};
use windows::Win32::Foundation::{
    CloseHandle, ERROR_BROKEN_PIPE, HANDLE, HMODULE, HWND, INVALID_HANDLE_VALUE,
};
use windows::Win32::Storage::FileSystem::{ReadFile, WriteFile};
use windows::Win32::System::Console::{COORD, HPCON};
use windows::Win32::System::LibraryLoader::{
    GetModuleHandleW, GetProcAddress, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
};
use windows::Win32::System::Pipes::CreatePipe;
use windows::Win32::System::Threading::{
    CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList, EXTENDED_STARTUPINFO_PRESENT,
    GetExitCodeProcess, INFINITE, InitializeProcThreadAttributeList, LPPROC_THREAD_ATTRIBUTE_LIST,
    PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE, PROCESS_INFORMATION, RegisterWaitForSingleObject, STARTF_USESTDHANDLES,
    STARTUPINFOEXW, UnregisterWaitEx, UpdateProcThreadAttribute, WT_EXECUTEONLYONCE,
};
use windows::core::{HRESULT, PCSTR, PCWSTR, PWSTR, s, w};

/// The packaged ConPTY embedded by `build.rs`: `VERSION` and `FILES`.
mod bundle {
    include!(concat!(env!("OUT_DIR"), "/conpty.rs"));
}

/// Count cells per code point, as `unicode-width` (alacritty's grid) does.
/// From the package's `conpty.h`; the built-in host accepts and ignores it.
const PSEUDOCONSOLE_GLYPH_WIDTH_WCSWIDTH: u32 = 0x10;

/// Bytes buffered per pipe direction.
const CAPACITY: usize = 0x1_0000;

const DLL: &str = "conpty.dll";
const HOST: &str = "OpenConsole.exe";

// ---- which ConPTY ----------------------------------------------------------

type CreateFn = unsafe extern "system" fn(COORD, HANDLE, HANDLE, u32, *mut HPCON) -> HRESULT;
type ResizeFn = unsafe extern "system" fn(HPCON, COORD) -> HRESULT;
type CloseFn = unsafe extern "system" fn(HPCON);
type ReleaseFn = unsafe extern "system" fn(HPCON) -> HRESULT;
type ReparentFn = unsafe extern "system" fn(HPCON, HWND) -> HRESULT;

/// The pseudoconsole functions of the ConPTY in use.
struct Api {
    create: CreateFn,
    resize: ResizeFn,
    close: CloseFn,
    /// Lets the host exit once the client is gone. Not on every build.
    release: Option<ReleaseFn>,
    /// Only the packaged host has it.
    reparent: Option<ReparentFn>,
}

fn api() -> &'static Api {
    static API: OnceLock<Api> = OnceLock::new();
    API.get_or_init(resolve)
}

fn resolve() -> Api {
    if std::env::var_os("BRINDLE_CONPTY").is_some_and(|v| v == "inbox") {
        log::warn!("BRINDLE_CONPTY=inbox: local tabs use the ConPTY built into Windows");
        return inbox();
    }
    let Some(files) = bundle::FILES else {
        log::warn!(
            "this build embeds no ConPTY (run scripts/fetch-conpty.ps1 before building): local tabs use the \
             ConPTY built into Windows, which repaints output and drops mouse modes"
        );
        return inbox();
    };
    let exe_dir = std::env::current_exe().ok().and_then(|exe| exe.parent().map(Path::to_path_buf));
    let extract_dir = dirs::data_local_dir().map(|dir| dir.join("brindle").join("conpty").join(bundle::VERSION));
    let loaded = shipped_dir(exe_dir.as_deref(), extract_dir.as_deref(), files)
        .and_then(|dir| load(&dir).map(|api| (api, dir)).map_err(|err| format!("loading it: {err}")));
    match loaded {
        Ok((api, dir)) => {
            log::info!("local tabs use ConPTY {} from {}", bundle::VERSION, dir.display());
            api
        }
        Err(why) => {
            log::warn!("the packaged ConPTY {} isn't usable ({why}): local tabs use the ConPTY built into Windows", bundle::VERSION);
            inbox()
        }
    }
}

/// Where to load the packaged ConPTY from: `exe_dir` when both files there
/// are the embedded ones (an MSI install, or `cargo run`), else
/// `extract_dir`, after writing the embedded files there if they differ.
/// The current directory and `PATH` are never candidates.
fn shipped_dir(exe_dir: Option<&Path>, extract_dir: Option<&Path>, (dll, host): (&[u8], &[u8])) -> Result<PathBuf, String> {
    let matches = |dir: &Path| same(&dir.join(DLL), dll) && same(&dir.join(HOST), host);
    if let Some(dir) = exe_dir.filter(|dir| matches(dir)) {
        return Ok(dir.to_path_buf());
    }
    let dir = extract_dir.ok_or("no local app data directory")?;
    if !matches(dir) {
        std::fs::create_dir_all(dir).map_err(|err| format!("creating {}: {err}", dir.display()))?;
        for (name, bytes) in [(DLL, dll), (HOST, host)] {
            write_replacing(&dir.join(name), bytes).map_err(|err| format!("writing {}: {err}", dir.join(name).display()))?;
        }
    }
    Ok(dir.to_path_buf())
}

fn same(path: &Path, bytes: &[u8]) -> bool {
    std::fs::read(path).is_ok_and(|on_disk| on_disk == bytes)
}

/// Writes `bytes` to a temporary file beside `path`, then renames it over
/// `path`, so a concurrent Brindle never loads half a file.
fn write_replacing(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        std::fs::remove_file(&tmp).ok();
    })
}

/// Loads `dir\conpty.dll` by absolute path. It finds `OpenConsole.exe`
/// beside itself.
fn load(dir: &Path) -> io::Result<Api> {
    let path = wide(dir.join(DLL).as_os_str());
    // SAFETY: an absolute, NUL-terminated path; the flags restrict the DLL's
    // own dependencies to its directory and System32.
    let module = unsafe {
        LoadLibraryExW(PCWSTR(path.as_ptr()), None, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_SYSTEM32)
    }?;
    // SAFETY: the signatures are those `conpty.h` declares for these names.
    unsafe {
        Ok(Api {
            create: function(module, s!("ConptyCreatePseudoConsole")).ok_or_else(missing)?,
            resize: function(module, s!("ConptyResizePseudoConsole")).ok_or_else(missing)?,
            close: function(module, s!("ConptyClosePseudoConsole")).ok_or_else(missing)?,
            release: function(module, s!("ConptyReleasePseudoConsole")),
            reparent: function(module, s!("ConptyReparentPseudoConsole")),
        })
    }
}

fn inbox() -> Api {
    // SAFETY: kernel32 is always loaded, and these are its documented
    // pseudoconsole functions (Windows 10 1809 and later).
    unsafe {
        let module = GetModuleHandleW(w!("kernel32.dll")).expect("kernel32 is loaded");
        Api {
            create: function(module, s!("CreatePseudoConsole")).expect("CreatePseudoConsole"),
            resize: function(module, s!("ResizePseudoConsole")).expect("ResizePseudoConsole"),
            close: function(module, s!("ClosePseudoConsole")).expect("ClosePseudoConsole"),
            release: function(module, s!("ReleasePseudoConsole")),
            reparent: None,
        }
    }
}

fn missing() -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, "conpty.dll lacks a pseudoconsole function")
}

/// # Safety
/// `F` must be the export's real function type.
unsafe fn function<F: Copy>(module: HMODULE, name: PCSTR) -> Option<F> {
    // SAFETY: GetProcAddress only reads the module's export table.
    let address = unsafe { GetProcAddress(module, name) }?;
    // SAFETY: the caller names the export's type; both are fn pointers.
    Some(unsafe { std::mem::transmute_copy::<unsafe extern "system" fn() -> isize, F>(&address) })
}

fn wide(text: &std::ffi::OsStr) -> Vec<u16> {
    text.encode_wide().chain(Some(0)).collect()
}

// ---- the pseudoconsole -----------------------------------------------------

/// An open pseudoconsole. Dropping it releases and closes it without
/// blocking the dropping thread.
pub struct Pseudoconsole {
    handle: HPCON,
}

// SAFETY: a pseudoconsole handle may be used from any thread.
unsafe impl Send for Pseudoconsole {}

impl Pseudoconsole {
    pub fn resize(&self, size: WindowSize) {
        // SAFETY: `handle` is open until drop.
        let result = unsafe { (api().resize)(self.handle, coord(size)) };
        if let Err(err) = result.ok() {
            log::warn!("resizing the pseudoconsole: {err}");
        }
    }

    /// Parents the console window programs can ask for (`GetConsoleWindow`)
    /// to `window`, so their dialogs belong to Brindle's window. Only the
    /// packaged host supports it.
    #[allow(dead_code)]
    pub fn reparent(&self, window: HWND) {
        if let Some(reparent) = api().reparent {
            // SAFETY: `handle` is open until drop; any HWND is accepted.
            unsafe { reparent(self.handle, window) }.ok().ok();
        }
    }
}

impl Drop for Pseudoconsole {
    fn drop(&mut self) {
        let api = api();
        if let Some(release) = api.release {
            // SAFETY: `handle` is still open.
            unsafe { release(self.handle) }.ok().ok();
        }
        // An older host blocks in Close until its output is read, and the
        // reader thread may still be reading it, so close elsewhere.
        let handle = self.handle.0;
        let close = api.close;
        let closed = std::thread::Builder::new().name("conpty-close".into()).spawn(move || {
            // SAFETY: the handle is closed exactly once, here.
            unsafe { close(HPCON(handle)) }
        });
        if closed.is_err() {
            log::warn!("couldn't start a thread to close a pseudoconsole; leaking it");
        }
    }
}

fn coord(size: WindowSize) -> COORD {
    COORD { X: size.num_cols as i16, Y: size.num_lines as i16 }
}

// ---- waking the event loop -------------------------------------------------

/// The event loop's poller and the event to post to it.
struct Wake {
    poller: Arc<Poller>,
    event: Event,
}

impl Wake {
    fn post(&self) {
        self.poller.post(CompletionPacket::new(self.event)).ok();
    }
}

struct Shared<S> {
    state: Mutex<S>,
    changed: Condvar,
}

impl<S> Shared<S> {
    fn new(state: S) -> Arc<Self> {
        Arc::new(Self { state: Mutex::new(state), changed: Condvar::new() })
    }

    fn lock(&self) -> MutexGuard<'_, S> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

// ---- reading output --------------------------------------------------------

#[derive(Default)]
struct ReadState {
    buf: VecDeque<u8>,
    eof: bool,
    error: Option<io::Error>,
    wake: Option<Wake>,
    /// The reader is gone: keep draining the pipe, but drop what's read.
    dropped: bool,
}

impl ReadState {
    fn ready(&self) -> bool {
        !self.buf.is_empty() || self.eof || self.error.is_some()
    }

    fn post(&self) {
        if let Some(wake) = &self.wake {
            wake.post();
        }
    }
}

/// The pseudoconsole's output, read on a thread of its own.
pub struct PipeReader {
    shared: Arc<Shared<ReadState>>,
}

impl PipeReader {
    pub fn new(pipe: OwnedHandle) -> io::Result<Self> {
        let shared = Shared::new(ReadState::default());
        let theirs = shared.clone();
        std::thread::Builder::new().name("conpty-reader".into()).spawn(move || read_loop(&pipe, &theirs))?;
        Ok(Self { shared })
    }

    /// Wakes `poller` with `event`'s key whenever output is ready, if it
    /// asks to read.
    pub fn register(&self, poller: &Arc<Poller>, event: Event) {
        let mut state = self.shared.lock();
        state.wake = event.readable.then(|| Wake { poller: poller.clone(), event: Event::readable(event.key) });
        if state.ready() {
            state.post();
        }
    }

    pub fn deregister(&self) {
        self.shared.lock().wake = None;
    }
}

impl Read for PipeReader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let mut state = self.shared.lock();
        if state.buf.is_empty() {
            if let Some(err) = state.error.take() {
                return Err(err);
            }
            return if state.eof { Ok(0) } else { Err(io::ErrorKind::WouldBlock.into()) };
        }
        let n = out.len().min(state.buf.len());
        for (slot, byte) in out.iter_mut().zip(state.buf.drain(..n)) {
            *slot = byte;
        }
        self.shared.changed.notify_all();
        // Level-triggered: say so again while there is more.
        if state.ready() {
            state.post();
        }
        Ok(n)
    }
}

impl Drop for PipeReader {
    fn drop(&mut self) {
        let mut state = self.shared.lock();
        state.dropped = true;
        state.buf.clear();
        self.shared.changed.notify_all();
    }
}

fn read_loop(pipe: &OwnedHandle, shared: &Shared<ReadState>) {
    let handle = HANDLE(pipe.as_raw_handle());
    let mut chunk = vec![0u8; CAPACITY];
    loop {
        {
            let mut state = shared.lock();
            while state.buf.len() >= CAPACITY && !state.dropped {
                state = shared.changed.wait(state).unwrap_or_else(|e| e.into_inner());
            }
        }
        let mut n = 0;
        // SAFETY: `handle` stays open for the loop; `chunk` outlives the call.
        let read = unsafe { ReadFile(handle, Some(&mut chunk), Some(&mut n), None) };
        let mut state = shared.lock();
        match read {
            Ok(()) if n == 0 => continue,
            Ok(()) => {
                if !state.dropped {
                    state.buf.extend(&chunk[..n as usize]);
                    state.post();
                }
            }
            Err(err) => {
                if err.code() == ERROR_BROKEN_PIPE.to_hresult() {
                    state.eof = true;
                } else {
                    state.error = Some(err.into());
                }
                state.post();
                return;
            }
        }
    }
}

// ---- writing input ---------------------------------------------------------

#[derive(Default)]
struct WriteState {
    buf: VecDeque<u8>,
    error: Option<io::Error>,
    wake: Option<Wake>,
    /// The writer is gone: write what's left, then close the pipe.
    dropped: bool,
}

/// The pseudoconsole's input, written on a thread of its own so a full pipe
/// never blocks the event loop.
pub struct PipeWriter {
    shared: Arc<Shared<WriteState>>,
}

impl PipeWriter {
    pub fn new(pipe: OwnedHandle) -> io::Result<Self> {
        let shared = Shared::new(WriteState::default());
        let theirs = shared.clone();
        std::thread::Builder::new().name("conpty-writer".into()).spawn(move || write_loop(pipe, &theirs))?;
        Ok(Self { shared })
    }

    /// Wakes `poller` with `event`'s key whenever there is room, if it asks
    /// to write.
    pub fn register(&self, poller: &Arc<Poller>, event: Event) {
        let mut state = self.shared.lock();
        state.wake = event.writable.then(|| Wake { poller: poller.clone(), event: Event::writable(event.key) });
        if state.buf.len() < CAPACITY
            && let Some(wake) = &state.wake
        {
            wake.post();
        }
    }

    pub fn deregister(&self) {
        self.shared.lock().wake = None;
    }
}

impl Write for PipeWriter {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        let mut state = self.shared.lock();
        if let Some(err) = state.error.take() {
            return Err(err);
        }
        let room = CAPACITY - state.buf.len();
        if room == 0 {
            return Err(io::ErrorKind::WouldBlock.into());
        }
        let n = room.min(data.len());
        state.buf.extend(&data[..n]);
        self.shared.changed.notify_all();
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for PipeWriter {
    fn drop(&mut self) {
        self.shared.lock().dropped = true;
        self.shared.changed.notify_all();
    }
}

fn write_loop(pipe: OwnedHandle, shared: &Shared<WriteState>) {
    let handle = HANDLE(pipe.as_raw_handle());
    loop {
        let chunk: Vec<u8> = {
            let mut state = shared.lock();
            while state.buf.is_empty() && !state.dropped {
                state = shared.changed.wait(state).unwrap_or_else(|e| e.into_inner());
            }
            if state.buf.is_empty() {
                return;
            }
            state.buf.drain(..).collect()
        };
        let mut written = 0;
        while written < chunk.len() {
            let mut n = 0;
            // SAFETY: `handle` stays open for the loop; `chunk` outlives the call.
            if let Err(err) = unsafe { WriteFile(handle, Some(&chunk[written..]), Some(&mut n), None) } {
                let mut state = shared.lock();
                state.error = Some(err.into());
                if let Some(wake) = &state.wake {
                    wake.post();
                }
                return;
            }
            written += n as usize;
        }
        // There is room again.
        if let Some(wake) = &shared.lock().wake {
            wake.post();
        }
    }
}

// ---- the program's exit ----------------------------------------------------

struct ChildShared {
    process: HANDLE,
    events: Mutex<mpsc::Sender<ChildEvent>>,
    exited: AtomicBool,
    wake: Mutex<Option<Wake>>,
}

// SAFETY: the process handle is only read from, by any thread.
unsafe impl Send for ChildShared {}
unsafe impl Sync for ChildShared {}

/// Reports when the program exits, with its exit code.
pub struct ChildWatcher {
    _process: OwnedHandle,
    wait: HANDLE,
    shared: *const ChildShared,
    events: mpsc::Receiver<ChildEvent>,
}

// SAFETY: `shared` is an `Arc` that is released only in drop.
unsafe impl Send for ChildWatcher {}

impl ChildWatcher {
    pub fn new(process: OwnedHandle) -> io::Result<Self> {
        let (tx, events) = mpsc::channel();
        let raw = HANDLE(process.as_raw_handle());
        let shared = Arc::into_raw(Arc::new(ChildShared {
            process: raw,
            events: Mutex::new(tx),
            exited: AtomicBool::new(false),
            wake: Mutex::new(None),
        }));
        let mut wait = HANDLE::default();
        // SAFETY: `shared` lives until drop, which unregisters the wait
        // (waiting for a running callback) before releasing it.
        let registered = unsafe {
            RegisterWaitForSingleObject(&mut wait, raw, Some(exited), Some(shared.cast()), INFINITE, WT_EXECUTEONLYONCE)
        };
        if let Err(err) = registered {
            // SAFETY: no callback was registered, so this is the only owner.
            drop(unsafe { Arc::from_raw(shared) });
            return Err(err.into());
        }
        Ok(Self { _process: process, wait, shared, events })
    }

    pub fn register(&self, poller: &Arc<Poller>, key: usize) {
        // SAFETY: `shared` is alive until drop.
        let shared = unsafe { &*self.shared };
        let mut slot = shared.wake.lock().unwrap_or_else(|e| e.into_inner());
        let wake = slot.insert(Wake { poller: poller.clone(), event: Event::readable(key) });
        // It may have exited before the event loop started. Checked with the
        // slot held, so an exit now either sees the waker or is seen here.
        if shared.exited.load(Ordering::SeqCst) {
            wake.post();
        }
    }

    pub fn deregister(&self) {
        // SAFETY: `shared` is alive until drop.
        *unsafe { &*self.shared }.wake.lock().unwrap_or_else(|e| e.into_inner()) = None;
    }

    pub fn next_event(&self) -> Option<ChildEvent> {
        match self.events.try_recv() {
            Ok(event) => Some(event),
            Err(mpsc::TryRecvError::Empty) => None,
            Err(mpsc::TryRecvError::Disconnected) => Some(ChildEvent::Exited(None)),
        }
    }
}

impl Drop for ChildWatcher {
    fn drop(&mut self) {
        // SAFETY: `wait` is the registered wait; INVALID_HANDLE_VALUE waits
        // for a callback that is already running.
        unsafe { UnregisterWaitEx(self.wait, Some(INVALID_HANDLE_VALUE)) }.ok();
        // SAFETY: no callback can run any more, so this releases the last
        // reference the wait held.
        drop(unsafe { Arc::from_raw(self.shared) });
    }
}

unsafe extern "system" fn exited(context: *mut c_void, _timed_out: bool) {
    // SAFETY: `context` is the `ChildShared` registered in `new`, alive
    // until the wait is unregistered.
    let shared = unsafe { &*(context as *const ChildShared) };
    let mut code = 0u32;
    // SAFETY: the process handle is open for as long as the watcher lives.
    let status = unsafe { GetExitCodeProcess(shared.process, &mut code) }.ok().map(|()| ExitStatus::from_raw(code));
    shared.events.lock().unwrap_or_else(|e| e.into_inner()).send(ChildEvent::Exited(status)).ok();
    let wake = shared.wake.lock().unwrap_or_else(|e| e.into_inner());
    shared.exited.store(true, Ordering::SeqCst);
    if let Some(wake) = &*wake {
        wake.post();
    }
}

// ---- starting a program ----------------------------------------------------

/// A program to start on a new pseudoconsole.
pub struct Command {
    /// The whole command line (see `cmdline`).
    pub line: String,
    pub cwd: Option<PathBuf>,
    /// Set on top of Brindle's own environment (names case-insensitive).
    pub env: Vec<(String, String)>,
}

/// A program running on its pseudoconsole.
pub struct Spawned {
    pub console: Pseudoconsole,
    pub reader: PipeReader,
    pub writer: PipeWriter,
    pub child: ChildWatcher,
    pub pid: u32,
}

pub fn spawn(command: &Command, size: WindowSize) -> io::Result<Spawned> {
    let (input_read, input_write) = pipe()?;
    let (output_read, output_write) = pipe()?;
    let mut handle = HPCON::default();
    // SAFETY: the pipe handles are valid; the pseudoconsole duplicates what
    // it keeps, so ours are closed when they drop below.
    unsafe {
        (api().create)(
            coord(size),
            HANDLE(input_read.as_raw_handle()),
            HANDLE(output_write.as_raw_handle()),
            PSEUDOCONSOLE_GLYPH_WIDTH_WCSWIDTH,
            &mut handle,
        )
    }
    .ok()?;
    let console = Pseudoconsole { handle };
    drop((input_read, output_write));

    let pid = start(command, console.handle)?;
    let child = ChildWatcher::new(pid.1)?;
    Ok(Spawned { console, reader: PipeReader::new(output_read)?, writer: PipeWriter::new(input_write)?, child, pid: pid.0 })
}

fn pipe() -> io::Result<(OwnedHandle, OwnedHandle)> {
    let (mut read, mut write) = (HANDLE::default(), HANDLE::default());
    // SAFETY: CreatePipe fills both handles; they aren't inheritable.
    unsafe { CreatePipe(&mut read, &mut write, None, 0) }?;
    // SAFETY: both handles are new and owned by nothing else.
    Ok(unsafe { (OwnedHandle::from_raw_handle(read.0), OwnedHandle::from_raw_handle(write.0)) })
}

/// Starts `command` attached to `console`, returning its pid and process.
fn start(command: &Command, console: HPCON) -> io::Result<(u32, OwnedHandle)> {
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
            Some(console.0 as *const c_void),
            std::mem::size_of::<HPCON>(),
            None,
            None,
        )
    }?;

    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;
    // With null standard handles, the program gets the pseudoconsole's
    // instead of anything of Brindle's.
    startup.StartupInfo.dwFlags = STARTF_USESTDHANDLES;
    startup.lpAttributeList = list.0;

    let mut line: Vec<u16> = command.line.encode_utf16().chain(Some(0)).collect();
    let env = environment_block(&command.env);
    let cwd = command.cwd.as_ref().map(|dir| wide(dir.as_os_str()));
    let mut info = PROCESS_INFORMATION::default();
    // SAFETY: every pointer is valid for the call; the command line buffer
    // is writable, as CreateProcessW requires.
    unsafe {
        CreateProcessW(
            PCWSTR::null(),
            Some(PWSTR(line.as_mut_ptr())),
            None,
            None,
            false,
            EXTENDED_STARTUPINFO_PRESENT | CREATE_UNICODE_ENVIRONMENT,
            Some(env.as_ptr().cast()),
            cwd.as_ref().map_or(PCWSTR::null(), |dir| PCWSTR(dir.as_ptr())),
            &startup.StartupInfo,
            &mut info,
        )
    }?;
    // SAFETY: the thread handle is ours and unused.
    unsafe { CloseHandle(info.hThread) }.ok();
    // SAFETY: the process handle is ours.
    Ok((info.dwProcessId, unsafe { OwnedHandle::from_raw_handle(info.hProcess.0) }))
}

/// `name=value` pairs, NUL-separated and NUL-terminated, in UTF-16.
fn environment_block(overrides: &[(String, String)]) -> Vec<u16> {
    let inherited: Vec<(String, OsString)> =
        std::env::vars_os().filter_map(|(name, value)| Some((name.into_string().ok()?, value))).collect();
    let overrides = overrides.iter().map(|(name, value)| (name.clone(), OsString::from(value))).collect();
    let mut block = Vec::new();
    for (name, value) in super::cmdline::merge_environment(inherited, overrides) {
        block.extend(name.encode_utf16());
        block.push(u16::from(b'='));
        block.extend(value.encode_wide());
        block.push(0);
    }
    block.push(0);
    block
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("brindle-conpty-{}-{name}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    const FILES: (&[u8], &[u8]) = (b"dll bytes", b"host bytes");

    #[test]
    fn matching_files_beside_the_exe_are_used() {
        let exe = scratch("exe-match");
        std::fs::write(exe.join(DLL), FILES.0).unwrap();
        std::fs::write(exe.join(HOST), FILES.1).unwrap();
        let extract = scratch("extract-unused");
        assert_eq!(shipped_dir(Some(&exe), Some(&extract), FILES), Ok(exe.clone()));
        assert!(!extract.join(DLL).exists(), "nothing extracted");
    }

    #[test]
    fn a_different_dll_beside_the_exe_is_ignored() {
        let exe = scratch("exe-planted");
        std::fs::write(exe.join(DLL), b"someone else's").unwrap();
        std::fs::write(exe.join(HOST), FILES.1).unwrap();
        let extract = scratch("extract-planted");
        assert_eq!(shipped_dir(Some(&exe), Some(&extract), FILES), Ok(extract.clone()));
        assert_eq!(std::fs::read(extract.join(DLL)).unwrap(), FILES.0);
        assert_eq!(std::fs::read(extract.join(HOST)).unwrap(), FILES.1);
    }

    #[test]
    fn a_stale_extraction_is_rewritten() {
        let extract = scratch("extract-stale");
        std::fs::write(extract.join(DLL), b"old").unwrap();
        std::fs::write(extract.join(HOST), FILES.1).unwrap();
        assert_eq!(shipped_dir(None, Some(&extract), FILES), Ok(extract.clone()));
        assert_eq!(std::fs::read(extract.join(DLL)).unwrap(), FILES.0);
    }

    #[test]
    fn no_candidates_is_an_error() {
        assert!(shipped_dir(None, None, FILES).is_err());
    }

    /// Waits for `poller` to report an event with `key`.
    fn wait_for(poller: &Poller, key: usize) -> Event {
        let mut events = polling::Events::new();
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            events.clear();
            poller.wait(&mut events, Some(Duration::from_millis(100))).unwrap();
            if let Some(event) = events.iter().find(|e| e.key == key) {
                return event;
            }
            assert!(Instant::now() < deadline, "no event for key {key}");
        }
    }

    #[test]
    fn reader_wakes_the_poller_and_reads_what_was_written() {
        let (read, write) = pipe().unwrap();
        let poller = Arc::new(Poller::new().unwrap());
        let mut reader = PipeReader::new(read).unwrap();
        reader.register(&poller, Event::readable(7));
        let mut buf = [0u8; 16];
        assert_eq!(reader.read(&mut buf).unwrap_err().kind(), io::ErrorKind::WouldBlock);

        let mut raw = std::fs::File::from(write);
        raw.write_all(b"hello world").unwrap();
        assert!(wait_for(&poller, 7).readable);
        let mut got = Vec::new();
        while got.len() < 11 {
            match reader.read(&mut buf[..4]) {
                Ok(n) => got.extend_from_slice(&buf[..n]),
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                    wait_for(&poller, 7);
                }
                Err(err) => panic!("{err}"),
            }
        }
        assert_eq!(got, b"hello world");

        drop(raw);
        wait_for(&poller, 7);
        assert_eq!(reader.read(&mut buf).unwrap(), 0, "EOF once the writer closes");
    }

    #[test]
    fn a_partial_read_posts_again() {
        let (read, write) = pipe().unwrap();
        let poller = Arc::new(Poller::new().unwrap());
        let mut reader = PipeReader::new(read).unwrap();
        reader.register(&poller, Event::readable(3));
        let mut raw = std::fs::File::from(write);
        raw.write_all(b"abcdef").unwrap();
        wait_for(&poller, 3);
        let mut buf = [0u8; 2];
        // Data may arrive in pieces; read until something is left behind.
        let mut got = Vec::new();
        while got.len() < 6 {
            match reader.read(&mut buf) {
                Ok(n) => {
                    got.extend_from_slice(&buf[..n]);
                    if got.len() < 6 {
                        // More is buffered, or arriving: either way a new
                        // event comes without anything else being written.
                        wait_for(&poller, 3);
                    }
                }
                Err(_) => {
                    wait_for(&poller, 3);
                }
            }
        }
        assert_eq!(got, b"abcdef");
    }

    #[test]
    fn writer_delivers_in_order_and_reports_room() {
        let (read, write) = pipe().unwrap();
        let poller = Arc::new(Poller::new().unwrap());
        let mut writer = PipeWriter::new(write).unwrap();
        writer.register(&poller, Event::writable(9));
        assert!(wait_for(&poller, 9).writable, "room right away");
        let reading = std::thread::spawn(move || {
            let mut all = Vec::new();
            std::fs::File::from(read).read_to_end(&mut all).unwrap();
            all
        });
        let data: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let mut sent = 0;
        while sent < data.len() {
            match writer.write(&data[sent..]) {
                Ok(n) => sent += n,
                Err(err) if err.kind() == io::ErrorKind::WouldBlock => {
                    wait_for(&poller, 9);
                }
                Err(err) => panic!("{err}"),
            }
        }
        drop(writer);
        assert_eq!(reading.join().unwrap(), data);
    }

    #[test]
    fn a_program_runs_and_its_exit_is_reported() {
        let command = Command {
            line: super::super::cmdline::command_line("cmd.exe", &["/D".into(), "/C".into(), "echo brindle-%TEST_VAR%& exit 3".into()]),
            cwd: None,
            env: vec![("TEST_VAR".into(), "ok".into())],
        };
        let size = WindowSize { num_lines: 24, num_cols: 80, cell_width: 8, cell_height: 16 };
        let mut spawned = spawn(&command, size).unwrap();
        let poller = Arc::new(Poller::new().unwrap());
        spawned.reader.register(&poller, Event::readable(1));
        spawned.child.register(&poller, 2);
        let mut output = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(20);
        let status = loop {
            assert!(Instant::now() < deadline, "no exit; output so far: {:?}", String::from_utf8_lossy(&output));
            let mut events = polling::Events::new();
            poller.wait(&mut events, Some(Duration::from_millis(200))).unwrap();
            let mut buf = [0u8; 4096];
            while let Ok(n) = spawned.reader.read(&mut buf) {
                if n == 0 {
                    break;
                }
                output.extend_from_slice(&buf[..n]);
            }
            if let Some(ChildEvent::Exited(status)) = spawned.child.next_event() {
                break status;
            }
        };
        assert_eq!(status.and_then(|s| s.code()), Some(3));
        // The output can arrive after the exit is reported.
        let text = |output: &[u8]| String::from_utf8_lossy(output).into_owned();
        let until = Instant::now() + Duration::from_secs(5);
        while !text(&output).contains("brindle-ok") && Instant::now() < until {
            let mut buf = [0u8; 4096];
            if let Ok(n) = spawned.reader.read(&mut buf) {
                output.extend_from_slice(&buf[..n]);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(text(&output).contains("brindle-ok"), "output: {:?}", text(&output));
        drop(spawned);
    }
}
