//! The event loop: one I/O completion port, with the level-triggered
//! readiness the core and the holders expect laid over overlapped pipes
//! and pipe listeners.
//!
//! Windows reports that an operation has *completed*, not that a handle is
//! ready. So a [`Source`] does its I/O itself, into and out of buffers it
//! owns, and the loop reports what the buffers hold (see [`super::pipe`]):
//! readable while input is buffered or has ended, writable while the output
//! buffer is under its mark. A listener ([`super::listen`]) is readable
//! once a client has connected. A wait first takes the completion packets that
//! are queued, then looks at every registered source, and only blocks on
//! the port when none is ready. That makes readiness level-triggered, as
//! with `poll(2)`.
//!
//! - **Registering** a source associates its handles with the loop's port,
//!   which Windows never undoes: a source can be deregistered and
//!   registered again, but only with the same loop. With
//!   [`Interest::NONE`] it stays registered and silent; its read in flight
//!   may still complete, into the source's buffer, and is reported when the
//!   interest returns.
//! - **Deregistering** cancels the source's read in flight (`CancelIoEx`).
//!   Its writes go on, since the owner may still flush and close it.
//! - **Dropping the loop** cancels every operation in flight of every
//!   source it ever registered, and takes their packets before it returns,
//!   since Windows stores an operation's result when its packet is taken.
//!   A source that outlives its loop goes on without one; if a write was
//!   cancelled, what the pipe hadn't taken is lost and its writes fail.
//! - **Waking:** [`Waker::wake`] posts a packet, at most one at a time.
//! - **A program's exit:** Windows has no `SIGCHLD`, so
//!   [`Waker::wake_on_child_exit`] has nothing to arrange. Instead the
//!   holder watches each program it starts with [`Waker::wake_on_exit`],
//!   which wakes the loop like [`Waker::wake`] when that process exits
//!   (a wait returns `true`), for the holder to reap.
//! - **One thread:** a loop and the sources registered with it are used
//!   from one thread. Their owners take packets themselves when they read,
//!   write or wait between the loop's waits (a blocking flush or close
//!   does), which is why a wake taken that way is kept for the next wait.

use std::collections::HashMap;
use std::ffi::c_void;
use std::fmt;
use std::io;
use std::mem::{self, ManuallyDrop};
use std::os::windows::io::{AsRawHandle, BorrowedHandle, FromRawHandle, OwnedHandle};
use std::ptr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE, WAIT_TIMEOUT};
use windows::Win32::System::IO::{
    CreateIoCompletionPort, GetQueuedCompletionStatus, OVERLAPPED, PostQueuedCompletionStatus,
};
use windows::Win32::System::Threading::{
    INFINITE, RegisterWaitForSingleObject, UnregisterWaitEx, WT_EXECUTEONLYONCE,
};

use crate::server::event_loop::{Event, EventLoop, Interest, Token};

/// The completion key of a wake packet. I/O packets carry the key their
/// handle was associated with, and an `OVERLAPPED`, which posted packets
/// lack.
const WAKE: usize = 1;
const IO: usize = 0;

/// The most packets one [`Port::pump`] takes, so that a flood of
/// completions can't keep a wait from returning.
const BATCH: usize = 64;

/// How often a wait looks at operations that report through an event
/// instead of a packet: a write issued before its source was registered.
const POLL: Duration = Duration::from_millis(10);

/// How long a dropped loop waits for the packets of the operations it
/// cancelled. They come at once; what is still missing afterwards keeps its
/// buffers for good.
const DRAIN_LIMIT: Duration = Duration::from_secs(5);

/// The Win32 code of an error from the `windows` crate, which wraps it as
/// an `HRESULT`.
pub(super) fn win32(error: &windows::core::Error) -> u32 {
    let code = error.code().0 as u32;
    if code & 0xffff_0000 == 0x8007_0000 {
        code & 0xffff
    } else {
        code
    }
}

pub(super) fn os_error(code: u32) -> io::Error {
    io::Error::from_raw_os_error(code as i32)
}

/// A timeout in milliseconds for a Win32 wait, rounded up so that a short
/// one doesn't spin at 0 ms, and never `INFINITE`.
pub(super) fn millis(timeout: Duration) -> u32 {
    timeout
        .as_nanos()
        .div_ceil(1_000_000)
        .min(u128::from(INFINITE - 1)) as u32
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Takes the result of an operation whose packet was taken: its
/// `OVERLAPPED`, the bytes transferred and its Win32 error (0 for none).
/// Called once, with the `OVERLAPPED` it was given to [`Port::expect`] for.
pub(super) type Completion = unsafe fn(overlapped: *mut OVERLAPPED, bytes: u32, error: u32);

/// What a loop reports: the state a source's owner (a pipe, a listener)
/// shares with the loop, which does the I/O itself.
pub(super) trait Watched: Send + Sync {
    /// Makes `port`'s loop the one that reports this: associates its
    /// handles with the port the first time, and issues what it waits for.
    fn attach(self: Arc<Self>, port: &Arc<Port>) -> io::Result<()>;

    /// The loop stops reporting this.
    fn detach(&self);

    /// Cancels the operations that are due a packet, for a port that is
    /// shutting down. Their packets still come.
    fn cancel_packets(&self);

    /// What the loop reports for it. `polling` is set when an operation in
    /// flight reports through an event, which the loop then has to come
    /// back for.
    fn readiness(
        self: Arc<Self>,
        token: Token,
        interest: Interest,
        polling: &mut bool,
    ) -> Option<Event>;
}

/// A loop's completion port, shared with its wakers and the sources
/// registered with it.
pub(super) struct Port {
    handle: OwnedHandle,
    /// A wake packet is queued and not yet taken.
    wake_queued: AtomicBool,
    /// A wake packet was taken, and no wait has reported it yet.
    woken: AtomicBool,
    /// The loop is there to take packets.
    alive: AtomicBool,
    /// The operations whose packets are due, by the address of their
    /// `OVERLAPPED`, each with what takes its result. Any other packet is
    /// ignored.
    due: Mutex<HashMap<usize, Completion>>,
    /// Every source whose handles are associated with the port.
    sources: Mutex<Vec<Weak<dyn Watched>>>,
}

impl Port {
    fn new() -> io::Result<Arc<Self>> {
        // SAFETY: creates a port with no file; one thread takes packets.
        let handle = unsafe { CreateIoCompletionPort(INVALID_HANDLE_VALUE, None, 0, 1) }
            .map_err(|error| os_error(win32(&error)))?;
        Ok(Arc::new(Self {
            // SAFETY: the handle is new and owned by nothing else.
            handle: unsafe { OwnedHandle::from_raw_handle(handle.0) },
            wake_queued: AtomicBool::new(false),
            woken: AtomicBool::new(false),
            alive: AtomicBool::new(true),
            due: Mutex::new(HashMap::new()),
            sources: Mutex::new(Vec::new()),
        }))
    }

    fn raw(&self) -> HANDLE {
        HANDLE(self.handle.as_raw_handle())
    }

    /// Whether the loop is there to take packets. Operations issued for a
    /// port that isn't report through an event instead.
    pub(super) fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    /// Associates a pipe handle with the port, for good: once only, and
    /// with no other port.
    pub(super) fn associate(&self, handle: HANDLE) -> io::Result<()> {
        // SAFETY: both handles are open.
        unsafe { CreateIoCompletionPort(handle, Some(self.raw()), IO, 0) }
            .map(drop)
            .map_err(|error| os_error(win32(&error)))
    }

    /// Remembers a source whose handles are associated with the port, to
    /// cancel its operations when the loop goes.
    pub(super) fn adopt<W: Watched + 'static>(&self, source: &Arc<W>) {
        let source: Weak<dyn Watched> = Arc::<W>::downgrade(source);
        let mut sources = lock(&self.sources);
        sources.retain(|source| source.strong_count() > 0);
        sources.push(source);
    }

    /// Expects a packet for the operation with this `OVERLAPPED`, which
    /// `completion` is then called with, once.
    pub(super) fn expect(&self, overlapped: *const OVERLAPPED, completion: Completion) {
        lock(&self.due).insert(overlapped as usize, completion);
    }

    /// The operation failed outright, so no packet follows.
    pub(super) fn forget(&self, overlapped: *const OVERLAPPED) {
        lock(&self.due).remove(&(overlapped as usize));
    }

    /// Takes queued packets, waiting up to `timeout` for the first, and
    /// records what each one says: a wake, or the end of an operation.
    pub(super) fn pump(&self, timeout: Duration) -> io::Result<()> {
        let mut wait = millis(timeout);
        for _ in 0..BATCH {
            let mut bytes = 0u32;
            let mut key = 0usize;
            let mut overlapped: *mut OVERLAPPED = ptr::null_mut();
            // SAFETY: the port is open and the out-pointers are valid.
            let result = unsafe {
                GetQueuedCompletionStatus(self.raw(), &mut bytes, &mut key, &mut overlapped, wait)
            };
            wait = 0;
            if overlapped.is_null() {
                match result {
                    // A posted packet.
                    Ok(()) if key == WAKE => {
                        self.wake_queued.store(false, Ordering::Release);
                        self.woken.store(true, Ordering::Release);
                    }
                    Ok(()) => {}
                    Err(error) if win32(&error) == WAIT_TIMEOUT.0 => return Ok(()),
                    Err(error) => return Err(os_error(win32(&error))),
                }
                continue;
            }
            // An operation's packet: a failure is the operation's.
            let error = result.err().map_or(0, |error| win32(&error));
            let completion = lock(&self.due).remove(&(overlapped as usize));
            if let Some(completion) = completion {
                // SAFETY: the packet was expected, so it is the one packet
                // of the operation `completion` was given for.
                unsafe { completion(overlapped, bytes, error) };
            }
        }
        Ok(())
    }

    fn wake(&self) -> io::Result<()> {
        if self.wake_queued.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        // SAFETY: the port is open.
        unsafe { PostQueuedCompletionStatus(self.raw(), 0, WAKE, None) }.map_err(|error| {
            self.wake_queued.store(false, Ordering::Release);
            os_error(win32(&error))
        })
    }

    /// The loop is going: cancels every operation that is due a packet and
    /// takes the packets, so that no operation is left to write to memory
    /// its source frees. Sources issue nothing for the port from here on.
    fn shut_down(&self) {
        self.alive.store(false, Ordering::Release);
        let sources = mem::take(&mut *lock(&self.sources));
        for source in sources.iter().filter_map(Weak::upgrade) {
            source.cancel_packets();
        }
        let start = Instant::now();
        loop {
            let due = lock(&self.due).len();
            let remaining = DRAIN_LIMIT.saturating_sub(start.elapsed());
            if due == 0 || remaining.is_zero() || self.pump(remaining).is_err() {
                // Each operation left holds a reference to its source's
                // state, which is never given back: a leak, not a
                // use-after-free.
                break;
            }
        }
    }
}

struct Entry {
    token: Token,
    source: Arc<dyn Watched>,
    interest: Interest,
}

/// The loop the core and the holders run on: [`EventLoop`] over one I/O
/// completion port.
pub struct Loop {
    port: Arc<Port>,
    entries: Vec<Entry>,
}

impl Loop {
    pub fn new() -> io::Result<Self> {
        Ok(Self {
            port: Port::new()?,
            entries: Vec::new(),
        })
    }

    fn position(&self, token: Token) -> io::Result<usize> {
        self.entries
            .iter()
            .position(|entry| entry.token == token)
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, format!("no token {token}")))
    }
}

impl Drop for Loop {
    fn drop(&mut self) {
        for entry in self.entries.drain(..) {
            entry.source.detach();
        }
        self.port.shut_down();
    }
}

/// Something a loop waits on: the state a pipe or a listener shares with
/// its owner (a stream, a listener; later a PTY, a ready pipe), which does
/// the I/O. It stays valid when the owner is gone, and then reports nothing.
#[derive(Clone)]
pub struct Source(Arc<dyn Watched>);

impl Source {
    pub(super) fn of(shared: Arc<dyn Watched>) -> Self {
        Self(shared)
    }
}

/// Wakes a loop's wait from another thread, or when a process exits.
#[derive(Clone)]
pub struct Waker {
    port: Arc<Port>,
}

impl fmt::Debug for Waker {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Waker").finish_non_exhaustive()
    }
}

impl Waker {
    #[allow(dead_code, reason = "for worker threads, as on Linux")]
    pub fn wake(&self) -> io::Result<()> {
        self.port.wake()
    }

    /// Nothing to arrange on Windows, which has no `SIGCHLD`: each program
    /// is watched on its own, with [`Waker::wake_on_exit`].
    pub fn wake_on_child_exit(&self) -> io::Result<()> {
        Ok(())
    }

    /// Wakes the loop when `process` exits, as [`Waker::wake`] does, for
    /// its starter to reap. The watch lasts until the returned value is
    /// dropped, and fires once; `process` may be closed meanwhile.
    pub fn wake_on_exit(&self, process: BorrowedHandle<'_>) -> io::Result<ExitWatch> {
        // The wait needs a handle that stays open, whatever the caller does
        // with its own.
        let process = process.try_clone_to_owned()?;
        let port = Arc::into_raw(Arc::clone(&self.port));
        let mut wait = HANDLE::default();
        // SAFETY: `port` stays valid until the watch is dropped, which
        // first waits for the callback to finish.
        let registered = unsafe {
            RegisterWaitForSingleObject(
                &mut wait,
                HANDLE(process.as_raw_handle()),
                Some(process_exited),
                Some(port.cast::<c_void>()),
                INFINITE,
                WT_EXECUTEONLYONCE,
            )
        };
        match registered {
            Ok(()) => Ok(ExitWatch {
                wait,
                port,
                process: ManuallyDrop::new(process),
            }),
            Err(error) => {
                // SAFETY: the reference made above, which nothing took.
                drop(unsafe { Arc::from_raw(port) });
                Err(os_error(win32(&error)))
            }
        }
    }
}

/// Runs on a thread pool thread when a watched process exits.
unsafe extern "system" fn process_exited(port: *mut c_void, _timed_out: bool) {
    // SAFETY: the watch holds a reference to the port until this returns.
    let port = unsafe { &*port.cast::<Port>() };
    let _ = port.wake();
}

/// A process watched with [`Waker::wake_on_exit`]. Dropping it ends the
/// watch.
pub struct ExitWatch {
    wait: HANDLE,
    /// The reference (`Arc::into_raw`) the callback uses.
    port: *const Port,
    /// The wait's own handle to the process, open while the wait is.
    process: ManuallyDrop<OwnedHandle>,
}

// SAFETY: the wait handle and the port reference are tied to no thread.
unsafe impl Send for ExitWatch {}

impl Drop for ExitWatch {
    fn drop(&mut self) {
        // With this event value, the call returns once the callback isn't
        // running and never will.
        // SAFETY: `wait` came from `RegisterWaitForSingleObject`.
        if unsafe { UnregisterWaitEx(self.wait, Some(INVALID_HANDLE_VALUE)) }.is_ok() {
            // SAFETY: the reference made in `wake_on_exit`, and the handle
            // the wait no longer uses; each dropped once, here.
            unsafe {
                drop(Arc::from_raw(self.port));
                ManuallyDrop::drop(&mut self.process);
            }
        }
        // Otherwise the wait may still be there: it keeps its reference and
        // its handle.
    }
}

impl EventLoop for Loop {
    fn register(&mut self, source: &Source, token: Token, interest: Interest) -> io::Result<()> {
        if self.entries.iter().any(|entry| entry.token == token) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("token {token} is already registered"),
            ));
        }
        Arc::clone(&source.0).attach(&self.port)?;
        self.entries.push(Entry {
            token,
            source: Arc::clone(&source.0),
            interest,
        });
        Ok(())
    }

    fn modify(&mut self, token: Token, interest: Interest) -> io::Result<()> {
        let index = self.position(token)?;
        self.entries[index].interest = interest;
        Ok(())
    }

    fn deregister(&mut self, token: Token) -> io::Result<()> {
        let index = self.position(token)?;
        self.entries.remove(index).source.detach();
        Ok(())
    }

    fn wait(&mut self, events: &mut Vec<Event>, timeout: Option<Duration>) -> io::Result<bool> {
        events.clear();
        let start = Instant::now();
        // What has completed is taken even when a source is ready already,
        // so that one busy source can't keep the others' input waiting.
        self.port.pump(Duration::ZERO)?;
        loop {
            let mut polling = false;
            for entry in &self.entries {
                events.extend(Arc::clone(&entry.source).readiness(
                    entry.token,
                    entry.interest,
                    &mut polling,
                ));
            }
            if !events.is_empty() || self.port.woken.load(Ordering::Acquire) {
                return Ok(self.port.woken.swap(false, Ordering::AcqRel));
            }
            let mut turn = match timeout {
                Some(timeout) => {
                    let remaining = timeout.saturating_sub(start.elapsed());
                    if remaining.is_zero() {
                        return Ok(false);
                    }
                    remaining
                }
                None => Duration::MAX,
            };
            if polling {
                turn = turn.min(POLL);
            }
            self.port.pump(turn)?;
        }
    }

    fn waker(&self) -> Waker {
        Waker {
            port: Arc::clone(&self.port),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::os::windows::io::AsHandle;
    use std::process::{Command, Stdio};

    use super::super::pipe::{self, testing::pipes};
    use super::*;

    const SOON: Option<Duration> = Some(Duration::from_secs(10));

    const WRITABLE: Interest = Interest {
        readable: false,
        writable: true,
    };

    #[test]
    fn reports_registered_sources_by_token() {
        let mut ev = Loop::new().unwrap();
        let (a, a_peer) = pipes();
        let (b, _b_peer) = pipes();
        ev.register(&a.source(), 7, Interest::READABLE).unwrap();
        ev.register(&b.source(), 8, Interest::READABLE).unwrap();
        assert!(ev.register(&b.source(), 8, Interest::READABLE).is_err());
        assert!(
            ev.register(&b.source(), 9, Interest::READABLE).is_err(),
            "a source is registered once"
        );

        let mut events = Vec::new();
        let start = Instant::now();
        assert!(
            !ev.wait(&mut events, Some(Duration::from_millis(30)))
                .unwrap()
        );
        assert!(events.is_empty());
        assert!(start.elapsed() >= Duration::from_millis(25));

        assert_eq!(a_peer.write(b"x").unwrap(), 1);
        assert!(!ev.wait(&mut events, SOON).unwrap());
        assert_eq!(
            events,
            [Event {
                token: 7,
                readable: true,
                writable: false,
                closed: false
            }]
        );
        // Level-triggered: still readable until read.
        ev.wait(&mut events, SOON).unwrap();
        assert_eq!(events.len(), 1);
        let mut byte = [0u8; 4];
        assert_eq!(a.read(&mut byte).unwrap(), 1);
        ev.wait(&mut events, Some(Duration::from_millis(20)))
            .unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn modify_pauses_and_writable_reports() {
        let mut ev = Loop::new().unwrap();
        let (read, write) = pipes();
        ev.register(&read.source(), 1, Interest::READABLE).unwrap();
        ev.register(&write.source(), 2, Interest::NONE).unwrap();
        assert_eq!(write.write(b"x").unwrap(), 1);
        drop(write);
        ev.deregister(2).unwrap();
        assert!(ev.deregister(2).is_err());

        let mut events = Vec::new();
        ev.modify(1, Interest::NONE).unwrap();
        ev.wait(&mut events, Some(Duration::from_millis(20)))
            .unwrap();
        assert!(
            events.is_empty(),
            "a paused source reports nothing, not even its hang-up"
        );
        ev.modify(1, Interest::READABLE).unwrap();
        ev.wait(&mut events, SOON).unwrap();
        assert_eq!(events.len(), 1);
        // The read that sees the hang-up is issued once the byte is taken.
        assert!(events[0].readable && !events[0].closed);
        let mut byte = [0u8; 4];
        assert_eq!(read.read(&mut byte).unwrap(), 1);
        ev.wait(&mut events, SOON).unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].readable && events[0].closed);
        assert_eq!(read.read(&mut byte).unwrap(), 0);

        let (_peer, write) = pipes();
        ev.register(&write.source(), 3, WRITABLE).unwrap();
        ev.deregister(1).unwrap();
        ev.wait(&mut events, SOON).unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].token == 3 && events[0].writable);
        assert!(ev.modify(1, Interest::NONE).is_err());
        // Deregistered sources can come back to the same loop.
        ev.register(&read.source(), 4, Interest::READABLE).unwrap();
        ev.wait(&mut events, SOON).unwrap();
        assert_eq!(events.len(), 2);
        assert!(events[1].token == 4 && events[1].closed);
    }

    #[test]
    fn a_full_output_buffer_is_not_writable() {
        let mut ev = Loop::new().unwrap();
        let (write, peer) = pipes();
        ev.register(&write.source(), 1, WRITABLE).unwrap();
        // The peer isn't reading: the pipe takes its own buffer's worth,
        // and the source the rest, up to its mark.
        let chunk = vec![5u8; 256 * 1024];
        let mut taken = 0;
        loop {
            match write.write(&chunk) {
                Ok(count) => taken += count,
                Err(error) => {
                    assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
                    break;
                }
            }
        }
        assert_eq!(taken, pipe::WRITE_MARK);
        let mut events = Vec::new();
        ev.wait(&mut events, Some(Duration::from_millis(20)))
            .unwrap();
        assert!(events.is_empty());

        // Once the peer reads, the write completes and room returns.
        let reader = std::thread::spawn(move || {
            pipe::testing::read_exactly(&peer, taken);
            peer
        });
        ev.wait(&mut events, SOON).unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].writable);
        let _peer = reader.join().unwrap();
    }

    #[test]
    fn the_end_is_reported_after_the_peer_closes() {
        let mut ev = Loop::new().unwrap();
        let (read, peer) = pipes();
        ev.register(&read.source(), 1, Interest::READABLE).unwrap();
        assert_eq!(peer.write(b"last words").unwrap(), 10);
        drop(peer);
        let mut events = Vec::new();
        let mut got = Vec::new();
        let mut buffer = [0u8; 4];
        loop {
            ev.wait(&mut events, SOON).unwrap();
            assert_eq!(events.len(), 1);
            assert!(events[0].readable);
            match read.read(&mut buffer).unwrap() {
                0 => break,
                count => got.extend_from_slice(&buffer[..count]),
            }
        }
        assert_eq!(got, b"last words");
        assert!(events[0].closed);
        // The end stays readable, and reads keep returning it.
        ev.wait(&mut events, SOON).unwrap();
        assert!(events[0].readable && events[0].closed);
        assert_eq!(read.read(&mut buffer).unwrap(), 0);
        // A write to the closed pipe fails; it doesn't wait.
        let error = read.write(b"x").unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::BrokenPipe, "{error}");
    }

    #[test]
    fn a_waker_wakes_a_wait_from_another_thread() {
        let mut ev = Loop::new().unwrap();
        let waker = ev.waker();
        let start = Instant::now();
        let thread = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(50));
            waker.wake().unwrap();
        });
        let mut events = Vec::new();
        assert!(ev.wait(&mut events, Some(Duration::from_secs(30))).unwrap());
        assert!(start.elapsed() < Duration::from_secs(20));
        thread.join().unwrap();
        // The wake-up was consumed.
        assert!(
            !ev.wait(&mut events, Some(Duration::from_millis(10)))
                .unwrap()
        );

        // Many wakes before a wait are one wake-up, and never block.
        let waker = ev.waker();
        for _ in 0..100_000 {
            waker.wake().unwrap();
        }
        assert!(ev.wait(&mut events, SOON).unwrap());
        assert!(
            !ev.wait(&mut events, Some(Duration::from_millis(10)))
                .unwrap()
        );
        // A waker that outlives its loop does nothing.
        drop(ev);
        waker.wake().unwrap();
    }

    #[test]
    fn a_process_exit_wakes_a_wait() {
        let mut ev = Loop::new().unwrap();
        let mut child = Command::new("cmd").args(["/c", "exit 3"]).spawn().unwrap();
        let watch = ev.waker().wake_on_exit(child.as_handle()).unwrap();
        let mut events = Vec::new();
        assert!(ev.wait(&mut events, Some(Duration::from_secs(30))).unwrap());
        assert!(events.is_empty());
        // The process has exited by the time the loop wakes.
        let status = child.try_wait().unwrap().expect("the process has exited");
        assert_eq!(status.code(), Some(3));
        // One exit is one wake-up.
        assert!(
            !ev.wait(&mut events, Some(Duration::from_millis(20)))
                .unwrap()
        );
        drop(watch);

        // A watch dropped first never wakes the loop, and one that outlives
        // the loop is harmless.
        let mut slow = Command::new("ping")
            .args(["-n", "2", "127.0.0.1"])
            .stdout(Stdio::null())
            .spawn()
            .unwrap();
        drop(ev.waker().wake_on_exit(slow.as_handle()).unwrap());
        let late = ev.waker().wake_on_exit(slow.as_handle()).unwrap();
        drop(ev);
        slow.wait().unwrap();
        drop(late);
    }
}
