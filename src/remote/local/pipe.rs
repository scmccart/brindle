//! The connection to the core on Windows: the client end of its named pipe.
//!
//! The pipe is opened with `FILE_FLAG_OVERLAPPED`, because a synchronous
//! pipe handle serializes all its I/O: the reader thread's blocked
//! `ReadFile` would stall the writer thread's `WriteFile`. [`Stream`] still
//! blocks like a socket. Each read and each write is one overlapped
//! operation waited for on an event of its own (one for reads, one for
//! writes), so a read and a write run at the same time, from clones or
//! through `&Stream`. Two reads take turns, as two writes do.
//!
//! [`Stream::shutdown`] ends every operation, waiting or later, as
//! `UnixStream::shutdown` does on Linux. The pipe itself closes with the
//! last clone.

use std::io::{self, Read, Write};
use std::os::windows::ffi::OsStrExt as _;
use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use windows::Win32::Foundation::{
    ERROR_BROKEN_PIPE, ERROR_HANDLE_EOF, ERROR_IO_PENDING, ERROR_NO_DATA, ERROR_OPERATION_ABORTED,
    ERROR_PIPE_BUSY, ERROR_PIPE_NOT_CONNECTED, GENERIC_READ, GENERIC_WRITE, HANDLE, WAIT_OBJECT_0,
    WAIT_TIMEOUT,
};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_OVERLAPPED, FILE_SHARE_NONE, OPEN_EXISTING, ReadFile,
    SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT, WriteFile,
};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Pipes::WaitNamedPipeW;
use windows::Win32::System::Threading::{
    CreateEventW, INFINITE, SetEvent, WaitForMultipleObjects, WaitForSingleObject,
};
use windows::core::PCWSTR;

use crate::remote::protocol::endpoint::Endpoints;
use crate::remote::protocol::win;

fn raw(handle: &OwnedHandle) -> HANDLE {
    HANDLE(handle.as_raw_handle())
}

/// The Win32 error code in `error`.
fn code(error: &windows::core::Error) -> u32 {
    (error.code().0 & 0xffff) as u32
}

fn os(error: &windows::core::Error) -> io::Error {
    io::Error::from_raw_os_error(code(error) as i32)
}

/// A manual-reset event, not yet set.
fn event() -> io::Result<OwnedHandle> {
    // SAFETY: no name and no security attributes; the handle is owned at
    // once.
    let event = unsafe { CreateEventW(None, true, false, None) }.map_err(|error| os(&error))?;
    // SAFETY: a new handle nothing else owns.
    Ok(unsafe { OwnedHandle::from_raw_handle(event.0) })
}

/// One direction's event, held for the length of an operation, and how long
/// an operation may take.
struct Direction {
    done: Mutex<OwnedHandle>,
    limit: Mutex<Option<Duration>>,
}

impl Direction {
    fn new() -> io::Result<Self> {
        Ok(Self { done: Mutex::new(event()?), limit: Mutex::new(None) })
    }
}

/// What the clones of a [`Stream`] share.
struct Pipe {
    handle: OwnedHandle,
    /// Set by `shutdown`: every read and write ends.
    closed: OwnedHandle,
    reads: Direction,
    writes: Direction,
}

/// What a read or a write was given.
enum Transfer<'a> {
    Read(&'a mut [u8]),
    Write(&'a [u8]),
}

/// A blocking connection over an overlapped pipe handle.
#[derive(Clone)]
pub struct Stream(Arc<Pipe>);

impl std::fmt::Debug for Stream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Stream({:?})", self.0.handle.as_raw_handle())
    }
}

impl Stream {
    /// A stream over `handle`, a pipe end opened for overlapped I/O.
    fn new(handle: OwnedHandle) -> io::Result<Self> {
        Ok(Self(Arc::new(Pipe {
            handle,
            closed: event()?,
            reads: Direction::new()?,
            writes: Direction::new()?,
        })))
    }

    /// Another handle on the same connection, for another thread.
    pub fn try_clone(&self) -> io::Result<Self> {
        Ok(self.clone())
    }

    /// A read then fails with `TimedOut` when nothing came for `limit`. As
    /// with a socket, the limit is the connection's, not this clone's.
    pub fn set_read_timeout(&self, limit: Option<Duration>) -> io::Result<()> {
        *self.0.reads.limit.lock() = limit;
        Ok(())
    }

    /// A write then fails with `TimedOut` when the core took nothing for
    /// `limit`.
    pub fn set_write_timeout(&self, limit: Option<Duration>) -> io::Result<()> {
        *self.0.writes.limit.lock() = limit;
        Ok(())
    }

    /// Ends the reads and writes of every clone, those waiting now and
    /// later ones: a read reports the end of the stream, a write fails.
    /// Both directions end whatever `_how` says, which is all the
    /// connection asks for.
    pub fn shutdown(&self, _how: std::net::Shutdown) -> io::Result<()> {
        // SAFETY: the event is open.
        unsafe { SetEvent(raw(&self.0.closed)) }.map_err(|error| os(&error))
    }

    /// One overlapped read or write, waited for. The end of the pipe reads
    /// as 0 bytes.
    fn transfer(&self, transfer: Transfer<'_>) -> io::Result<usize> {
        let pipe = &*self.0;
        let reading = matches!(transfer, Transfer::Read(_));
        let direction = if reading { &pipe.reads } else { &pipe.writes };
        // Held until the operation has ended: the event is this one's.
        let done = direction.done.lock();
        let limit = *direction.limit.lock();
        let shut = || match reading {
            true => Ok(0),
            false => Err(io::Error::new(io::ErrorKind::BrokenPipe, "the connection was shut down")),
        };
        let closed = raw(&pipe.closed);
        // SAFETY: the event is open.
        if unsafe { WaitForSingleObject(closed, 0) } == WAIT_OBJECT_0 {
            return shut();
        }
        let mut overlapped = OVERLAPPED { hEvent: raw(&done), ..Default::default() };
        let handle = raw(&pipe.handle);
        // SAFETY: the buffer and `overlapped` outlive the operation, which
        // has ended (or was cancelled and has ended) before this returns.
        let issued = unsafe {
            match transfer {
                Transfer::Read(buffer) => ReadFile(handle, Some(buffer), None, Some(&mut overlapped)),
                Transfer::Write(bytes) => WriteFile(handle, Some(bytes), None, Some(&mut overlapped)),
            }
        };
        let mut timed_out = false;
        match issued {
            Ok(()) => {}
            Err(error) if code(&error) == ERROR_IO_PENDING.0 => {
                let millis = limit.map_or(INFINITE, |limit| limit.as_millis().min(1 << 30) as u32);
                // SAFETY: both events are open.
                let waited = unsafe { WaitForMultipleObjects(&[raw(&done), closed], false, millis) };
                if waited != WAIT_OBJECT_0 {
                    timed_out = waited == WAIT_TIMEOUT;
                    // SAFETY: cancels this operation only; its end is
                    // waited for below.
                    let _ = unsafe { CancelIoEx(handle, Some(&overlapped)) };
                }
            }
            Err(error) => return ended(reading, &error),
        }
        let mut transferred = 0u32;
        // SAFETY: waits for the operation `overlapped` names to end.
        match unsafe { GetOverlappedResult(handle, &overlapped, &mut transferred, true) } {
            // Also when it ended just before the cancel reached it.
            Ok(()) => Ok(transferred as usize),
            Err(error) if code(&error) == ERROR_OPERATION_ABORTED.0 => match timed_out {
                true => Err(io::ErrorKind::TimedOut.into()),
                false => shut(),
            },
            Err(error) => ended(reading, &error),
        }
    }
}

/// How a failed read or write ends: the other side having closed the pipe
/// is the end of what can be read.
fn ended(reading: bool, error: &windows::core::Error) -> io::Result<usize> {
    let gone = [ERROR_BROKEN_PIPE.0, ERROR_HANDLE_EOF.0, ERROR_PIPE_NOT_CONNECTED.0, ERROR_NO_DATA.0];
    if reading && gone.contains(&code(error)) {
        return Ok(0);
    }
    Err(os(error))
}

impl Read for Stream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.transfer(Transfer::Read(buffer))
    }
}

impl Read for &Stream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.transfer(Transfer::Read(buffer))
    }
}

impl Write for Stream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.transfer(Transfer::Write(bytes))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Write for &Stream {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.transfer(Transfer::Write(bytes))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn wide(path: &Path) -> Vec<u16> {
    path.as_os_str().encode_wide().chain([0]).collect()
}

/// The core's pipe name for the server directory `dir`, for messages. The
/// name hashes the directory's canonical path, so a directory that doesn't
/// exist has none.
pub fn address(dir: &Path) -> String {
    match Endpoints::of(dir) {
        Ok(endpoints) => endpoints.core().display().to_string(),
        Err(_) => format!("the pipe of {}", dir.display()),
    }
}

/// Connects to the core of the server directory `dir`, and checks who runs
/// it before anything is written: a server that runs as another user or at
/// another integrity level is `PermissionDenied`. `NotFound` when no core
/// listens (or the directory doesn't exist).
///
/// The core makes its next pipe instance when it accepts a client, so the
/// pipe is busy while another client waits to be accepted, as it does while
/// a new core adopts its holders. That is waited out for `busy`, then
/// `TimedOut`.
pub fn connect(dir: &Path, busy: Duration) -> io::Result<Stream> {
    let path = Endpoints::of(dir)?.core();
    let name = wide(&path);
    let deadline = Instant::now() + busy;
    let handle = loop {
        // The server may identify this client, and no more.
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
            Err(error) if code(&error) == ERROR_PIPE_BUSY.0 => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        format!("it accepted no connection within {} s", busy.as_secs()),
                    ));
                }
                // Returns when an instance is free, or failed: the next
                // open tells which.
                let millis = (remaining.as_millis().min(1 << 30) as u32).max(1);
                // SAFETY: as above.
                let _ = unsafe { WaitNamedPipeW(PCWSTR(name.as_ptr()), millis) };
            }
            Err(error) => return Err(os(&error)),
        }
    };
    win::check_server(&handle)?;
    Stream::new(handle)
}

/// A stand-in for the core in tests: serves the pipe the core of `dir`
/// would. Like a socket's backlog, an instance is always waiting, so a
/// client connects before `accept` is called.
#[cfg(test)]
pub struct Listener {
    name: Vec<u16>,
    waiting: Mutex<OwnedHandle>,
}

#[cfg(test)]
impl Listener {
    pub fn bind(dir: &Path) -> Self {
        let name = wide(&Endpoints::of(dir).unwrap().core());
        let waiting = Mutex::new(Self::instance(&name));
        Self { name, waiting }
    }

    fn instance(name: &[u16]) -> OwnedHandle {
        use windows::Win32::Storage::FileSystem::PIPE_ACCESS_DUPLEX;
        use windows::Win32::System::Pipes::{
            CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
        };

        // SAFETY: the name is NUL-terminated and outlives the call.
        let handle = unsafe {
            CreateNamedPipeW(
                PCWSTR(name.as_ptr()),
                PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                PIPE_UNLIMITED_INSTANCES,
                64 * 1024,
                64 * 1024,
                0,
                None,
            )
        };
        assert!(!handle.is_invalid(), "{}", io::Error::last_os_error());
        // SAFETY: a new handle nothing else owns.
        unsafe { OwnedHandle::from_raw_handle(handle.0) }
    }

    /// The next client's connection.
    pub fn accept(&self) -> Stream {
        use windows::Win32::Foundation::ERROR_PIPE_CONNECTED;
        use windows::Win32::System::Pipes::ConnectNamedPipe;

        let mut waiting = self.waiting.lock();
        let done = event().unwrap();
        let mut overlapped = OVERLAPPED { hEvent: raw(&done), ..Default::default() };
        // SAFETY: `overlapped` outlives the operation, which is waited for.
        match unsafe { ConnectNamedPipe(raw(&waiting), Some(&mut overlapped)) } {
            Ok(()) => {}
            // The client was there first.
            Err(error) if code(&error) == ERROR_PIPE_CONNECTED.0 => {}
            Err(error) if code(&error) == ERROR_IO_PENDING.0 => {
                let mut transferred = 0u32;
                // SAFETY: waits for the operation `overlapped` names.
                unsafe { GetOverlappedResult(raw(&waiting), &overlapped, &mut transferred, true) }.unwrap();
            }
            Err(error) => panic!("accept: {error}"),
        }
        let connected = std::mem::replace(&mut *waiting, Self::instance(&self.name));
        Stream::new(connected).unwrap()
    }
}
