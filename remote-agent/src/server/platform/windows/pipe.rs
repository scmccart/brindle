//! Pipe sources: overlapped pipe I/O behind the loop's readiness facade.
//!
//! A [`Pipe`] is the owner's side (a stream; later a PTY and a ready pipe)
//! of up to two pipe handles opened with `FILE_FLAG_OVERLAPPED`: one to
//! read from and one to write to, which are the same handle for a duplex
//! pipe. Its [`Source`] is the loop's side of the same state. Pipes are
//! read as bytes (`PIPE_READMODE_BYTE`), and their handles keep the default
//! completion modes: every operation that doesn't fail outright queues its
//! packet, also one that ends at once.
//!
//! - **Reading:** one overlapped read is in flight into a buffer the pipe
//!   owns ([`READ_BUFFER`] bytes). The pipe is *readable* while that buffer
//!   holds bytes, or once the read hit the end or failed. The next read is
//!   issued when the owner has taken every byte, so an owner that stops
//!   reading holds back one buffer, and the peer's writes then wait in the
//!   pipe. A registered pipe always has its read issued; otherwise the
//!   owner's first read or wait issues it.
//! - **Writing:** writes copy into a buffer the pipe owns and are issued
//!   overlapped, one at a time. The pipe is *writable* while fewer than
//!   [`WRITE_MARK`] bytes wait there for the pipe to take them.
//! - **Completions** come in one of two ways, chosen for each operation:
//!   - *as a packet* on a loop's port, once the pipe has been registered
//!     with a loop (its handles stay associated with that loop's port, so
//!     a pipe belongs to one loop for good). Whoever takes the packet
//!     ([`Port::pump`]: the loop's wait, or this module's reads, writes and
//!     waits) records the result here.
//!   - *through an event*, for a pipe no loop has seen and for one whose
//!     loop is gone: the `OVERLAPPED` names an event with its low bit set,
//!     which keeps the completion off any port. The owner's reads, writes
//!     and waits look at the operation, and [`wait`] waits on the event.
//!     Windows cancels such an operation when the thread that issued it
//!     exits: a read is then issued again when one is wanted, but a write
//!     fails the pipe, so a pipe isn't left to another thread with output
//!     on its way.
//! - **The end:** `ERROR_BROKEN_PIPE` on a read is the end of input (the
//!   peer closed its handle; everything it wrote was read first), as is
//!   closing this side's own handle. Other read errors, and every write
//!   error, stay errors: `ERROR_NO_DATA` (a write after the peer closed)
//!   and `ERROR_BROKEN_PIPE` are `BrokenPipe`, and `ERROR_PIPE_NOT_CONNECTED`
//!   means the server end disconnected, which discards what was unread.
//! - **Shutting down writes:** a pipe handle has no half-close. Once
//!   everything written has been taken by the pipe, [`Pipe::shutdown_write`]
//!   closes the handle writes go to. The peer reads every byte and then the
//!   end, also when it had written something this side never read. For a
//!   duplex pipe that is the handle reads come from too, so this side then
//!   reads the end as well, and what the peer sends afterwards is lost.
//! - **Lifetimes:** Windows may write to an operation's `OVERLAPPED` and
//!   buffer until the operation has ended, and for a packet until the
//!   packet is taken (the result is stored then, by the taking thread). So
//!   an operation due a packet holds a reference to the pipe's state, given
//!   back in [`complete`]: dropping the owner cancels its I/O with
//!   `CancelIoEx` and closes the handles at once, and the buffers go when
//!   the last packet has been taken. An operation that reports through an
//!   event is cancelled and waited for before the owner's drop returns.
//!   Cancelling a write loses whatever the pipe hadn't taken, and how much
//!   that was isn't known, so only a drop (and a loop's drop) does it.

use std::ffi::c_void;
use std::io;
use std::mem::{self, ManuallyDrop};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr::{self, NonNull};
use std::slice;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{
    ERROR_BROKEN_PIPE, ERROR_HANDLE_EOF, ERROR_IO_INCOMPLETE, ERROR_IO_PENDING, ERROR_MORE_DATA,
    ERROR_OPERATION_ABORTED, ERROR_WRITE_FAULT, HANDLE, WAIT_OBJECT_0,
};
use windows::Win32::Storage::FileSystem::{ReadFile, WriteFile};
use windows::Win32::System::IO::{CancelIoEx, GetOverlappedResult, OVERLAPPED};
use windows::Win32::System::Pipes::PeekNamedPipe;
use windows::Win32::System::Threading::{
    CreateEventW, WaitForMultipleObjects, WaitForSingleObject,
};
use windows::core::PCWSTR;

use super::event_loop::{Port, Source, Watched, millis, os_error, win32};
use crate::server::event_loop::{Event, Interest, Token};

/// How much one read takes from a pipe, and so how much input a pipe holds
/// for an owner that isn't reading.
pub(super) const READ_BUFFER: usize = 64 * 1024;

/// A pipe takes writes while fewer bytes than this wait for the pipe.
pub(super) const WRITE_MARK: usize = 1024 * 1024;

/// A write buffer that grew past this is freed once it is empty.
const KEEP: usize = 64 * 1024;

/// How long a cancelled operation gets to end before its memory is leaked
/// instead of freed. A cancelled pipe operation ends at once.
const RECALL_LIMIT: Duration = Duration::from_secs(5);

/// How long [`wait`] waits on one kind of completion before it looks at the
/// other, when it is given pipes of both kinds.
const TURN: Duration = Duration::from_millis(10);

/// The most handles one `WaitForMultipleObjects` takes.
const MAX_WAIT_HANDLES: usize = 64;

const NO_ERROR: u32 = 0;
const IO_PENDING: u32 = ERROR_IO_PENDING.0;
const IO_INCOMPLETE: u32 = ERROR_IO_INCOMPLETE.0;
const MORE_DATA: u32 = ERROR_MORE_DATA.0;
const BROKEN_PIPE: u32 = ERROR_BROKEN_PIPE.0;
const HANDLE_EOF: u32 = ERROR_HANDLE_EOF.0;
const OPERATION_ABORTED: u32 = ERROR_OPERATION_ABORTED.0;

/// How an operation in flight reports its end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// As a packet on the pipe's port.
    Packet,
    /// Through the operation's event.
    Event,
}

#[derive(Debug, Clone, Copy)]
enum Direction {
    Read,
    Write,
}

/// What Windows is given for one operation. A packet names its operation by
/// the address of its `OVERLAPPED`, which is this block's address.
#[repr(C)]
struct Block {
    overlapped: OVERLAPPED,
    direction: Direction,
    /// The reference to the pipe's state (`Arc::into_raw`) the operation
    /// holds while its packet is due.
    owner: *const Shared,
}

/// One direction's overlapped operation: at most one is in flight.
struct Op {
    /// Never moved or freed while an operation is in flight. Reached only
    /// through raw pointers, since Windows writes to it meanwhile.
    block: NonNull<Block>,
    /// Signalled when an operation in [`Mode::Event`] ends. Made when first
    /// needed.
    event: Option<OwnedHandle>,
    in_flight: Option<Mode>,
}

// SAFETY: `block` is this value's own allocation, and nothing in it is tied
// to a thread.
unsafe impl Send for Op {}

/// How an operation stands when the call that issued it returns.
enum Issued {
    InFlight,
    Done { bytes: u32, error: u32 },
}

/// What became of an operation [`Op::recall`] took back.
enum Recalled {
    /// It wasn't in flight, or is due a packet.
    Not,
    Ended {
        bytes: u32,
        error: u32,
    },
    /// It hasn't ended, so its memory can't be freed.
    Stuck,
}

impl Op {
    fn new(direction: Direction) -> Self {
        let block = Box::new(Block {
            overlapped: OVERLAPPED::default(),
            direction,
            owner: ptr::null(),
        });
        Self {
            block: NonNull::from(Box::leak(block)),
            event: None,
            in_flight: None,
        }
    }

    fn overlapped(&self) -> *mut OVERLAPPED {
        // `Block` is `repr(C)` with the `OVERLAPPED` first.
        self.block.as_ptr().cast()
    }

    fn event(&mut self) -> Result<HANDLE, u32> {
        if self.event.is_none() {
            // SAFETY: a manual-reset event with no name and no security.
            let event = unsafe { CreateEventW(None, true, false, PCWSTR::null()) }
                .map_err(|error| win32(&error))?;
            // SAFETY: the handle is new and owned by nothing else.
            self.event = Some(unsafe { OwnedHandle::from_raw_handle(event.0) });
        }
        Ok(raw(self.event.as_ref().expect("the event was just made")))
    }

    /// Issues an operation through `call`, which gets the `OVERLAPPED` and
    /// where to put the count of an operation that ends at once. With a
    /// `port` the operation is due a packet there, and holds a reference to
    /// `me` until [`complete`] takes it; without, it reports through the
    /// event.
    fn issue(
        &mut self,
        port: Option<&Arc<Port>>,
        me: &Arc<Shared>,
        call: impl FnOnce(*mut OVERLAPPED, *mut u32) -> windows::core::Result<()>,
    ) -> Issued {
        debug_assert!(self.in_flight.is_none());
        let block = self.block.as_ptr();
        let overlapped = self.overlapped();
        let mut prepared = OVERLAPPED::default();
        match port {
            Some(port) => {
                // SAFETY: nothing is in flight, so the block is ours alone.
                unsafe { (*block).owner = Arc::into_raw(Arc::clone(me)) };
                // Before the call: the packet may be taken as soon as the
                // operation is issued.
                port.expect(overlapped, complete);
            }
            None => match self.event() {
                // The low bit keeps the completion off a port the handle is
                // associated with.
                Ok(event) => prepared.hEvent = HANDLE((event.0 as usize | 1) as *mut c_void),
                Err(error) => return Issued::Done { bytes: 0, error },
            },
        }
        // SAFETY: as above.
        unsafe { overlapped.write(prepared) };
        let mut bytes = 0u32;
        let error = match call(overlapped, &mut bytes) {
            Ok(()) => NO_ERROR,
            Err(error) => win32(&error),
        };
        match (port, error) {
            // A packet follows an operation that ended at once, too.
            // `ERROR_MORE_DATA` is such an end (a message pipe read as
            // messages), though these pipes are read as bytes.
            (Some(_), NO_ERROR | IO_PENDING | MORE_DATA) => {
                self.in_flight = Some(Mode::Packet);
                Issued::InFlight
            }
            (Some(port), _) => {
                // It failed outright: no packet, so the reference comes back.
                port.forget(overlapped);
                // SAFETY: the reference made above, which nothing else took.
                unsafe {
                    drop(Arc::from_raw((*block).owner));
                    (*block).owner = ptr::null();
                }
                Issued::Done { bytes: 0, error }
            }
            (None, IO_PENDING) => {
                self.in_flight = Some(Mode::Event);
                Issued::InFlight
            }
            (None, _) => Issued::Done { bytes, error },
        }
    }

    /// The result of an operation that reports through its event, once it
    /// has ended: its byte count and error.
    fn poll(&mut self, handle: HANDLE) -> Option<(u32, u32)> {
        if self.in_flight != Some(Mode::Event) {
            return None;
        }
        let mut bytes = 0u32;
        // SAFETY: the block is alive, and this doesn't wait.
        let result = unsafe { GetOverlappedResult(handle, self.overlapped(), &mut bytes, false) };
        let error = match result {
            Ok(()) => NO_ERROR,
            Err(error) => win32(&error),
        };
        if error == IO_INCOMPLETE {
            return None;
        }
        self.in_flight = None;
        Some((bytes, error))
    }

    /// Cancels an operation that reports through its event and waits for it
    /// to end, so that its memory can be freed or used again.
    fn recall(&mut self, handle: HANDLE) -> Recalled {
        if self.in_flight != Some(Mode::Event) {
            return Recalled::Not;
        }
        let Some(event) = self.event.as_ref().map(raw) else {
            return Recalled::Stuck;
        };
        // SAFETY: the handle is open and the block alive. This fails when
        // the operation has ended already, which the wait then sees.
        let _ = unsafe { CancelIoEx(handle, Some(self.overlapped())) };
        // SAFETY: the event is this operation's own.
        if unsafe { WaitForSingleObject(event, millis(RECALL_LIMIT)) } != WAIT_OBJECT_0 {
            return Recalled::Stuck;
        }
        match self.poll(handle) {
            Some((bytes, error)) => Recalled::Ended { bytes, error },
            None => Recalled::Stuck,
        }
    }

    /// Cancels an operation that is due a packet. The packet still comes.
    fn cancel_packet(&self, handle: HANDLE) {
        if self.in_flight == Some(Mode::Packet) {
            // SAFETY: the handle is open and the block alive.
            let _ = unsafe { CancelIoEx(handle, Some(self.overlapped())) };
        }
    }
}

impl Drop for Op {
    fn drop(&mut self) {
        // An operation due a packet holds a reference to the state this is
        // part of, so it has ended by now. One that reports through its
        // event and couldn't be recalled keeps its block (and its reader's
        // buffer or its writer's bytes): leaked, since Windows may still
        // write there.
        if self.in_flight.is_none() {
            // SAFETY: allocated in `new`, and Windows is done with it.
            drop(unsafe { Box::from_raw(self.block.as_ptr()) });
        }
    }
}

/// The bytes a read fills. Reached through a raw pointer, since Windows
/// writes to it while a read is in flight.
struct Buffer {
    pointer: NonNull<u8>,
    capacity: usize,
}

// SAFETY: the allocation is this value's own.
unsafe impl Send for Buffer {}

impl Buffer {
    fn new(capacity: usize) -> Self {
        let bytes = Box::leak(vec![0u8; capacity].into_boxed_slice());
        Self {
            pointer: NonNull::from(bytes).cast(),
            capacity,
        }
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        let bytes = ptr::slice_from_raw_parts_mut(self.pointer.as_ptr(), self.capacity);
        // SAFETY: allocated in `new` as a boxed slice of this length.
        drop(unsafe { Box::from_raw(bytes) });
    }
}

struct Reader {
    /// Closed (`None`) once the owner is gone or shut a duplex pipe down.
    handle: Option<Arc<OwnedHandle>>,
    op: Op,
    /// Freed with the reader, unless a read never ended.
    buffer: ManuallyDrop<Buffer>,
    /// The bytes read and not yet taken are `buffer[start..end]`.
    start: usize,
    end: usize,
    /// No read will be issued again: the buffer holds the last input.
    eof: bool,
    error: Option<u32>,
}

impl Reader {
    fn new(handle: Arc<OwnedHandle>) -> Self {
        Self {
            handle: Some(handle),
            op: Op::new(Direction::Read),
            buffer: ManuallyDrop::new(Buffer::new(READ_BUFFER)),
            start: 0,
            end: 0,
            eof: false,
            error: None,
        }
    }

    fn ready(&self) -> bool {
        self.end > self.start || self.eof || self.error.is_some()
    }
}

impl Drop for Reader {
    fn drop(&mut self) {
        // A read that couldn't be recalled may still fill the buffer.
        if self.op.in_flight.is_none() {
            // SAFETY: dropped once, here.
            unsafe { ManuallyDrop::drop(&mut self.buffer) };
        }
    }
}

struct Writer {
    /// Closed (`None`) once the owner is gone or shut the writes down.
    handle: Option<Arc<OwnedHandle>>,
    op: Op,
    /// The write in flight covers `sending[sent..]`; it isn't touched
    /// meanwhile.
    sending: Vec<u8>,
    sent: usize,
    /// What the next write takes.
    queued: Vec<u8>,
    error: Option<u32>,
    /// [`Pipe::shutdown_write`] was called: the handle closes once
    /// everything is sent.
    shut: bool,
}

impl Writer {
    fn new(handle: Arc<OwnedHandle>) -> Self {
        Self {
            handle: Some(handle),
            op: Op::new(Direction::Write),
            sending: Vec::new(),
            sent: 0,
            queued: Vec::new(),
            error: None,
            shut: false,
        }
    }

    /// Bytes the pipe hasn't taken yet.
    fn unsent(&self) -> usize {
        self.sending.len() - self.sent + self.queued.len()
    }

    fn ready(&self) -> bool {
        self.unsent() < WRITE_MARK || self.error.is_some()
    }

    fn fail(&mut self, error: u32) {
        self.error = Some(error);
        self.sending = Vec::new();
        self.sent = 0;
        self.queued = Vec::new();
    }
}

impl Drop for Writer {
    fn drop(&mut self) {
        // A write that couldn't be recalled may still read its bytes.
        if self.op.in_flight.is_some() {
            mem::forget(mem::take(&mut self.sending));
        }
    }
}

struct State {
    reader: Option<Reader>,
    writer: Option<Writer>,
    /// The port the handles are associated with, from the first
    /// registration on. A handle can't leave its port.
    port: Option<Arc<Port>>,
    /// A loop reports this pipe.
    registered: bool,
    /// The owner is gone: the handles are closed, and a loop that still
    /// lists the pipe reports nothing for it.
    abandoned: bool,
}

/// What a pipe's owner, the loop that reports it and its operations in
/// flight share.
pub(super) struct Shared {
    state: Mutex<State>,
}

fn raw(handle: &OwnedHandle) -> HANDLE {
    HANDLE(handle.as_raw_handle())
}

/// Where the next operation's packet goes: the pipe's port while its loop
/// lives. `None` makes the operation report through its event.
fn packets(port: &Option<Arc<Port>>) -> Option<&Arc<Port>> {
    port.as_ref().filter(|port| port.is_alive())
}

impl State {
    /// The pipe's handles, each once.
    fn handles(&self) -> impl Iterator<Item = HANDLE> {
        let read = self
            .reader
            .as_ref()
            .and_then(|reader| reader.handle.as_ref());
        let write = self
            .writer
            .as_ref()
            .and_then(|writer| writer.handle.as_ref())
            .filter(|write| !read.is_some_and(|read| Arc::ptr_eq(read, write)));
        read.into_iter().chain(write).map(|handle| raw(handle))
    }

    /// Issues the read when one is due: none in flight, nothing buffered,
    /// and more to come.
    fn start_read(&mut self, me: &Arc<Shared>) {
        let Some(reader) = self.reader.as_mut() else {
            return;
        };
        if reader.op.in_flight.is_some() || reader.ready() {
            return;
        }
        let Some(handle) = reader.handle.as_deref().map(raw) else {
            reader.eof = true;
            return;
        };
        reader.start = 0;
        reader.end = 0;
        let (pointer, capacity) = (reader.buffer.pointer.as_ptr(), reader.buffer.capacity);
        let port = packets(&self.port);
        let issued = reader.op.issue(port, me, |overlapped, count| {
            // SAFETY: the buffer and the block live until the read has
            // ended (and its packet is taken), and nothing else uses the
            // buffer while the read is in flight.
            unsafe {
                let buffer = slice::from_raw_parts_mut(pointer, capacity);
                ReadFile(handle, Some(buffer), Some(count), Some(overlapped))
            }
        });
        if let Issued::Done { bytes, error } = issued {
            self.read_done(bytes, error);
        }
    }

    fn read_done(&mut self, bytes: u32, error: u32) {
        let Some(reader) = self.reader.as_mut() else {
            return;
        };
        reader.op.in_flight = None;
        match error {
            NO_ERROR | MORE_DATA if bytes > 0 => reader.end = bytes as usize,
            // A read of nothing is a message pipe's empty message, which
            // stands for the end like `Read`'s `Ok(0)`.
            NO_ERROR | MORE_DATA | BROKEN_PIPE | HANDLE_EOF => reader.eof = true,
            // Cancelled: nothing was read, and a later read may follow.
            OPERATION_ABORTED => {}
            error => reader.error = Some(error),
        }
        if reader.handle.is_none() {
            reader.eof = true;
        }
    }

    /// Issues writes while there is output and none is in flight. Once a
    /// shut writer has sent everything, closes its handle.
    fn send(&mut self, me: &Arc<Shared>) {
        loop {
            let Some(writer) = self.writer.as_mut() else {
                return;
            };
            if writer.op.in_flight.is_some() || writer.error.is_some() {
                return;
            }
            if writer.sent == writer.sending.len() {
                writer.sending.clear();
                writer.sent = 0;
                mem::swap(&mut writer.sending, &mut writer.queued);
                if writer.queued.capacity() > KEEP {
                    writer.queued = Vec::new();
                }
                if writer.sending.is_empty() {
                    if writer.shut && writer.handle.is_some() {
                        self.close_writer();
                    }
                    return;
                }
            }
            let Some(handle) = writer.handle.as_deref().map(raw) else {
                writer.fail(BROKEN_PIPE);
                return;
            };
            let unsent = &writer.sending[writer.sent..];
            let (pointer, length) = (unsent.as_ptr(), unsent.len());
            let port = packets(&self.port);
            let issued = writer.op.issue(port, me, |overlapped, count| {
                // SAFETY: `sending` and the block live, untouched, until
                // the write has ended (and its packet is taken).
                unsafe {
                    let bytes = slice::from_raw_parts(pointer, length);
                    WriteFile(handle, Some(bytes), Some(count), Some(overlapped))
                }
            });
            match issued {
                Issued::InFlight => return,
                Issued::Done { bytes, error } => self.write_done(bytes, error),
            }
        }
    }

    fn write_done(&mut self, bytes: u32, error: u32) {
        let Some(writer) = self.writer.as_mut() else {
            return;
        };
        writer.op.in_flight = None;
        let left = writer.sending.len() - writer.sent;
        match error {
            NO_ERROR if bytes > 0 => writer.sent += left.min(bytes as usize),
            // A write that took nothing would be issued again for ever.
            NO_ERROR => writer.fail(ERROR_WRITE_FAULT.0),
            error => writer.fail(error),
        }
    }

    /// Closes the handle writes go to. For a duplex pipe that is the handle
    /// reads come from, so the read side ends with it.
    fn close_writer(&mut self) {
        let Some(handle) = self.writer.as_mut().and_then(|writer| writer.handle.take()) else {
            return;
        };
        if let Some(reader) = self.reader.as_mut()
            && reader
                .handle
                .as_ref()
                .is_some_and(|read| Arc::ptr_eq(read, &handle))
        {
            reader.op.cancel_packet(raw(&handle));
            let recalled = reader.op.recall(raw(&handle));
            reader.handle = None;
            match recalled {
                Recalled::Ended { bytes, error } => self.read_done(bytes, error),
                // Its packet ends the input, or nothing was in flight.
                Recalled::Not if reader.op.in_flight.is_some() => {}
                Recalled::Not => reader.eof = true,
                // Never polled again: the handle to poll it with is gone.
                Recalled::Stuck => reader.eof = true,
            }
        }
    }

    /// Takes in what has completed without a packet, and issues what is due
    /// next.
    fn settle(&mut self, me: &Arc<Shared>) {
        if let Some(reader) = self.reader.as_mut()
            && let Some(handle) = reader.handle.as_deref().map(raw)
            && let Some((bytes, error)) = reader.op.poll(handle)
        {
            self.read_done(bytes, error);
        }
        if let Some(writer) = self.writer.as_mut()
            && let Some(handle) = writer.handle.as_deref().map(raw)
            && let Some((bytes, error)) = writer.op.poll(handle)
        {
            self.write_done(bytes, error);
        }
        self.send(me);
        if self.registered {
            self.start_read(me);
        }
    }

    fn readable(&self) -> bool {
        self.reader.as_ref().is_some_and(Reader::ready)
    }

    fn writable(&self) -> bool {
        self.writer.as_ref().is_some_and(Writer::ready)
    }

    /// Whether the pipe has taken everything written, or never will.
    fn sent(&self) -> bool {
        self.writer
            .as_ref()
            .is_none_or(|writer| writer.unsent() == 0 || writer.error.is_some())
    }

    /// The peer is gone or the pipe failed: a read returns the end or an
    /// error (for a pipe with nothing to read from, a write fails). So a
    /// handler that reads when told of it always gets one of the two.
    fn hung_up(&self) -> bool {
        match (&self.reader, &self.writer) {
            (Some(reader), _) => reader.eof || reader.error.is_some(),
            (None, Some(writer)) => writer.error.is_some(),
            (None, None) => false,
        }
    }

    /// Whether a wait for `need` is over. A hang-up ends it too: the peer
    /// closing ends the input, and fails a write that is waiting.
    fn meets(&self, need: Need) -> bool {
        (need.readable && self.readable())
            || (need.writable && self.writable())
            || (need.sent && self.sent())
    }

    /// What a wait for `need` waits on: the ports and events of the
    /// operations in flight that can bring it about.
    fn waits(&self, need: Need, ports: &mut Vec<Arc<Port>>, events: &mut Vec<HANDLE>) {
        let read = self.reader.as_ref().map(|reader| &reader.op);
        let write = self.writer.as_ref().map(|writer| &writer.op);
        // A shut writer's last write closes the handle, which ends input.
        let shutting = self.writer.as_ref().is_some_and(|writer| writer.shut);
        let wanted = [
            read.filter(|_| need.readable),
            write.filter(|_| need.writable || need.sent || (need.readable && shutting)),
        ];
        for op in wanted.into_iter().flatten() {
            match op.in_flight {
                Some(Mode::Packet) => {
                    if let Some(port) = &self.port
                        && !ports.iter().any(|known| Arc::ptr_eq(known, port))
                    {
                        ports.push(Arc::clone(port));
                    }
                }
                Some(Mode::Event) => events.extend(op.event.as_ref().map(raw)),
                None => {}
            }
        }
    }

    /// The port to pump for `direction`'s operation, when it is due a
    /// packet.
    fn due(&self, direction: Direction) -> Option<Arc<Port>> {
        let in_flight = match direction {
            Direction::Read => self.reader.as_ref()?.op.in_flight,
            Direction::Write => self.writer.as_ref()?.op.in_flight,
        };
        (in_flight == Some(Mode::Packet))
            .then(|| self.port.clone())
            .flatten()
    }
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Makes `port`'s loop the one that reports this pipe: associates the
    /// handles with the port the first time, and issues the read.
    pub(super) fn attach(self: &Arc<Self>, port: &Arc<Port>) -> io::Result<()> {
        let mut state = self.lock();
        if state.registered {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "the source is already registered",
            ));
        }
        match &state.port {
            Some(bound) if Arc::ptr_eq(bound, port) => {}
            Some(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::Unsupported,
                    "the source belongs to another event loop",
                ));
            }
            None => {
                // A read issued before now reports through its event, which
                // the loop doesn't wait on. Taking it back loses nothing;
                // it is issued again below, for a packet. (A write issued
                // before now can't be taken back, so the loop polls it.)
                if let Some(reader) = state.reader.as_mut()
                    && let Some(handle) = reader.handle.as_deref().map(raw)
                {
                    match reader.op.recall(handle) {
                        Recalled::Not => {}
                        Recalled::Ended { bytes, error } => state.read_done(bytes, error),
                        Recalled::Stuck => {
                            return Err(io::Error::new(
                                io::ErrorKind::TimedOut,
                                "a pending read couldn't be cancelled",
                            ));
                        }
                    }
                }
                for handle in state.handles() {
                    port.associate(handle)?;
                }
                state.port = Some(Arc::clone(port));
                port.adopt(self);
            }
        }
        state.registered = true;
        state.settle(self);
        Ok(())
    }

    /// The pipe's loop stops reporting it: the read in flight is cancelled.
    /// Writes go on, since the owner may still flush and close.
    pub(super) fn detach(&self) {
        let mut state = self.lock();
        state.registered = false;
        if let Some(reader) = state.reader.as_ref()
            && let Some(handle) = reader.handle.as_deref().map(raw)
        {
            reader.op.cancel_packet(handle);
        }
    }

    /// Cancels the operations that are due a packet, for a port that is
    /// shutting down. Their packets still come.
    pub(super) fn cancel_packets(&self) {
        let state = self.lock();
        if let Some(reader) = state.reader.as_ref()
            && let Some(handle) = reader.handle.as_deref().map(raw)
        {
            reader.op.cancel_packet(handle);
        }
        if let Some(writer) = state.writer.as_ref()
            && let Some(handle) = writer.handle.as_deref().map(raw)
        {
            writer.op.cancel_packet(handle);
        }
    }

    /// What a loop reports for the pipe. `polling` is set when an operation
    /// in flight reports through its event, which the loop then has to come
    /// back for.
    pub(super) fn readiness(
        self: &Arc<Self>,
        token: Token,
        interest: Interest,
        polling: &mut bool,
    ) -> Option<Event> {
        let mut state = self.lock();
        if state.abandoned {
            return None;
        }
        state.settle(self);
        let in_flight = [
            state.reader.as_ref().map(|reader| reader.op.in_flight),
            state.writer.as_ref().map(|writer| writer.op.in_flight),
        ];
        *polling |= in_flight.contains(&Some(Some(Mode::Event)));
        if interest == Interest::NONE {
            return None;
        }
        let event = Event {
            token,
            readable: interest.readable && state.readable(),
            writable: interest.writable && state.writable(),
            closed: state.hung_up(),
        };
        (event.readable || event.writable || event.closed).then_some(event)
    }
}

impl Watched for Shared {
    fn attach(self: Arc<Self>, port: &Arc<Port>) -> io::Result<()> {
        Shared::attach(&self, port)
    }

    fn detach(&self) {
        Shared::detach(self);
    }

    fn cancel_packets(&self) {
        Shared::cancel_packets(self);
    }

    fn readiness(
        self: Arc<Self>,
        token: Token,
        interest: Interest,
        polling: &mut bool,
    ) -> Option<Event> {
        Shared::readiness(&self, token, interest, polling)
    }
}

/// Records the end of the operation a packet names, and issues what is due
/// next: the [`Completion`](super::event_loop::Completion) of every
/// operation issued for a packet.
///
/// # Safety
///
/// `overlapped` must come from the packet of an operation [`Op::issue`]
/// expected with this function, and each packet must be handed in once.
unsafe fn complete(overlapped: *mut OVERLAPPED, bytes: u32, error: u32) {
    let block = overlapped.cast::<Block>();
    // SAFETY: the operation's reference kept the state, and so the block,
    // alive until now. Nothing else touches the block while its operation
    // is in flight.
    let (shared, direction) = unsafe {
        let owner = mem::replace(&mut (*block).owner, ptr::null());
        (Arc::from_raw(owner), (*block).direction)
    };
    let mut state = shared.lock();
    match direction {
        Direction::Read => state.read_done(bytes, error),
        Direction::Write => state.write_done(bytes, error),
    }
    state.settle(&shared);
    drop(state);
    // With the owner gone, this may be the last reference: the buffers go
    // now that the packet is taken.
}

/// What a wait on a pipe is for.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Need {
    /// Input, its end, or an error to read.
    pub readable: bool,
    /// Room for a write.
    pub writable: bool,
    /// The pipe having taken everything written.
    pub sent: bool,
}

impl From<Interest> for Need {
    fn from(interest: Interest) -> Self {
        Self {
            readable: interest.readable,
            writable: interest.writable,
            sent: false,
        }
    }
}

/// The owner's side of a pipe source. Reads and writes never block.
pub(super) struct Pipe {
    shared: Arc<Shared>,
}

impl Pipe {
    /// A duplex pipe: one handle to read from and write to.
    pub(super) fn duplex(handle: OwnedHandle) -> Self {
        let handle = Arc::new(handle);
        Self::with(
            Some(Reader::new(Arc::clone(&handle))),
            Some(Writer::new(handle)),
        )
    }

    /// A pipe end to read from, one to write to, or one of each: two pipes
    /// that make one source.
    #[allow(
        dead_code,
        reason = "for the PTY's pipes and the ready pipe (tasks 4.2 and 5.2)"
    )]
    pub(super) fn of(read: Option<OwnedHandle>, write: Option<OwnedHandle>) -> Self {
        Self::with(
            read.map(|handle| Reader::new(Arc::new(handle))),
            write.map(|handle| Writer::new(Arc::new(handle))),
        )
    }

    /// A duplex pipe whose handle is associated with `port` already: a
    /// listener's instance that a client connected to. Its operations are
    /// due packets there from the start, and only that port's loop can
    /// report it.
    pub(super) fn duplex_on(handle: OwnedHandle, port: &Arc<Port>) -> Self {
        let pipe = Self::duplex(handle);
        pipe.shared.lock().port = Some(Arc::clone(port));
        port.adopt(&pipe.shared);
        pipe
    }

    fn with(reader: Option<Reader>, writer: Option<Writer>) -> Self {
        Self {
            shared: Arc::new(Shared {
                state: Mutex::new(State {
                    reader,
                    writer,
                    port: None,
                    registered: false,
                    abandoned: false,
                }),
            }),
        }
    }

    /// What an event loop waits on for this pipe.
    pub(super) fn source(&self) -> Source {
        Source::of(Arc::clone(&self.shared) as Arc<dyn Watched>)
    }

    /// Takes bytes that were read. `Ok(0)` is the end of input, and
    /// `WouldBlock` means nothing has arrived.
    pub(super) fn read(&self, buffer: &mut [u8]) -> io::Result<usize> {
        // A second look after taking the read's packet, and a third after
        // issuing a read that ended at once.
        let mut pumped = false;
        for _ in 0..3 {
            let port = {
                let mut state = self.shared.lock();
                state.settle(&self.shared);
                let Some(reader) = state.reader.as_mut() else {
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "the pipe has no end to read from",
                    ));
                };
                if reader.end > reader.start {
                    let count = buffer.len().min(reader.end - reader.start);
                    // SAFETY: no read is in flight while the buffer holds
                    // bytes, so they are initialised and nothing writes to
                    // them.
                    unsafe {
                        let from = reader.buffer.pointer.as_ptr().add(reader.start);
                        ptr::copy_nonoverlapping(from, buffer.as_mut_ptr(), count);
                    }
                    reader.start += count;
                    state.start_read(&self.shared);
                    return Ok(count);
                }
                if let Some(error) = reader.error {
                    return Err(os_error(error));
                }
                if reader.eof {
                    return Ok(0);
                }
                if reader.op.in_flight.is_none() {
                    state.start_read(&self.shared);
                    continue;
                }
                match state.due(Direction::Read) {
                    Some(port) if !pumped => port,
                    _ => break,
                }
            };
            pumped = true;
            port.pump(Duration::ZERO)?;
        }
        Err(io::ErrorKind::WouldBlock.into())
    }

    /// Takes bytes to write, as many as fit under [`WRITE_MARK`].
    /// `WouldBlock` means none do.
    pub(super) fn write(&self, bytes: &[u8]) -> io::Result<usize> {
        let mut pumped = false;
        loop {
            let port = {
                let mut state = self.shared.lock();
                state.settle(&self.shared);
                let Some(writer) = state.writer.as_mut() else {
                    return Err(io::Error::new(
                        io::ErrorKind::Unsupported,
                        "the pipe has no end to write to",
                    ));
                };
                if let Some(error) = writer.error {
                    return Err(os_error(error));
                }
                if writer.shut {
                    return Err(io::Error::new(
                        io::ErrorKind::BrokenPipe,
                        "the pipe's write side was shut down",
                    ));
                }
                let unsent = writer.unsent();
                if unsent < WRITE_MARK {
                    let count = bytes.len().min(WRITE_MARK - unsent);
                    writer.queued.extend_from_slice(&bytes[..count]);
                    state.send(&self.shared);
                    // A write to a pipe whose peer is gone fails at once.
                    return match state.writer.as_ref().and_then(|writer| writer.error) {
                        Some(error) => Err(os_error(error)),
                        None => Ok(count),
                    };
                }
                match state.due(Direction::Write) {
                    Some(port) if !pumped => port,
                    _ => return Err(io::ErrorKind::WouldBlock.into()),
                }
            };
            pumped = true;
            port.pump(Duration::ZERO)?;
        }
    }

    /// Whether the peer has sent bytes this side hasn't taken: read into the
    /// buffer, or waiting in the pipe.
    pub(super) fn input_waiting(&self) -> bool {
        let buffered = |state: &State| {
            state
                .reader
                .as_ref()
                .is_some_and(|reader| reader.end > reader.start)
        };
        let port = {
            let mut state = self.shared.lock();
            state.settle(&self.shared);
            if buffered(&state) {
                return true;
            }
            state.due(Direction::Read)
        };
        // The read in flight may have taken the bytes already.
        if let Some(port) = port {
            let _ = port.pump(Duration::ZERO);
        }
        let state = self.shared.lock();
        if buffered(&state) {
            return true;
        }
        let Some(handle) = state
            .reader
            .as_ref()
            .and_then(|reader| reader.handle.as_deref())
        else {
            return false;
        };
        let mut waiting = 0u32;
        // SAFETY: the handle is open, and only the count is asked for.
        unsafe { PeekNamedPipe(raw(handle), None, 0, None, Some(&mut waiting), None) }.is_ok()
            && waiting > 0
    }

    /// How many bytes written here the pipe hasn't taken yet.
    pub(super) fn unsent(&self) -> usize {
        let state = self.shared.lock();
        state.writer.as_ref().map_or(0, Writer::unsent)
    }

    /// Whether the pipe has taken everything written. An error is the
    /// write's.
    pub(super) fn sent(&self) -> io::Result<bool> {
        let port = {
            let mut state = self.shared.lock();
            state.settle(&self.shared);
            state.due(Direction::Write)
        };
        if let Some(port) = port {
            port.pump(Duration::ZERO)?;
        }
        let state = self.shared.lock();
        match state.writer.as_ref() {
            Some(writer) => match writer.error {
                Some(error) => Err(os_error(error)),
                None => Ok(writer.unsent() == 0),
            },
            None => Ok(true),
        }
    }

    /// Ends this side's output: once the pipe has taken everything written,
    /// the handle writes go to is closed, and the peer reads the end after
    /// the last byte. See the module's notes for a duplex pipe.
    pub(super) fn shutdown_write(&self) {
        let mut state = self.shared.lock();
        if let Some(writer) = state.writer.as_mut() {
            writer.shut = true;
        }
        state.settle(&self.shared);
    }
}

impl Drop for Pipe {
    fn drop(&mut self) {
        let mut state = self.shared.lock();
        state.abandoned = true;
        for handle in state.handles() {
            // SAFETY: the handle is open. This fails when nothing is in
            // flight.
            let _ = unsafe { CancelIoEx(handle, None) };
        }
        // Operations due a packet keep the state alive until their packets
        // are taken. The others end here, before the handles close.
        if let Some(reader) = state.reader.as_mut()
            && let Some(handle) = reader.handle.take()
            && let Recalled::Ended { bytes, error } = reader.op.recall(raw(&handle))
        {
            state.read_done(bytes, error);
        }
        if let Some(writer) = state.writer.as_mut()
            && let Some(handle) = writer.handle.take()
            && let Recalled::Ended { bytes, error } = writer.op.recall(raw(&handle))
        {
            state.write_done(bytes, error);
        }
    }
}

/// Waits up to `timeout` until one of `pipes` has what it is waited for, or
/// has hung up.
pub(super) fn wait(pipes: &[(&Pipe, Need)], timeout: Duration) {
    let start = Instant::now();
    loop {
        let mut ports: Vec<Arc<Port>> = Vec::new();
        let mut events: Vec<HANDLE> = Vec::new();
        for (pipe, need) in pipes {
            let mut state = pipe.shared.lock();
            state.settle(&pipe.shared);
            if need.readable {
                state.start_read(&pipe.shared);
            }
            if state.meets(*need) {
                return;
            }
            state.waits(*need, &mut ports, &mut events);
        }
        let remaining = timeout.saturating_sub(start.elapsed());
        if remaining.is_zero() {
            return;
        }
        match (ports.as_slice(), events.as_slice()) {
            // Nothing in flight can change anything.
            ([], []) => {
                thread::sleep(remaining);
                return;
            }
            ([port], []) => {
                if port.pump(remaining).is_err() {
                    return;
                }
            }
            ([], _) => wait_any(&events, remaining),
            // Both kinds, or several ports: take turns.
            _ => {
                for port in &ports {
                    let _ = port.pump(Duration::ZERO);
                }
                let turn = remaining.min(TURN);
                if events.is_empty() {
                    thread::sleep(turn);
                } else {
                    wait_any(&events, turn);
                }
            }
        }
    }
}

fn wait_any(events: &[HANDLE], timeout: Duration) {
    let (events, timeout) = if events.len() > MAX_WAIT_HANDLES {
        (&events[..MAX_WAIT_HANDLES], timeout.min(TURN))
    } else {
        (events, timeout)
    };
    // SAFETY: the events belong to pipes the caller borrows.
    unsafe { WaitForMultipleObjects(events, false, millis(timeout)) };
}

#[cfg(test)]
pub(super) mod testing {
    use std::sync::atomic::{AtomicU32, Ordering};

    use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE};
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED,
        FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_NONE, OPEN_EXISTING, PIPE_ACCESS_DUPLEX,
        PIPE_ACCESS_INBOUND,
    };
    use windows::Win32::System::Pipes::{
        CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE, PIPE_WAIT,
    };

    use super::*;

    /// The pipes' own buffers, each way.
    pub const PIPE_BUFFER: usize = 64 * 1024;

    static NEXT: AtomicU32 = AtomicU32::new(0);

    fn connected(access: FILE_FLAGS_AND_ATTRIBUTES, client: u32) -> (OwnedHandle, OwnedHandle) {
        let name: Vec<u16> = format!(
            r"\\.\pipe\brindle-test-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        )
        .encode_utf16()
        .chain([0])
        .collect();
        // SAFETY: the name is NUL-terminated and outlives both calls, and
        // each handle is owned by what is returned.
        unsafe {
            let server = CreateNamedPipeW(
                PCWSTR(name.as_ptr()),
                access | FILE_FLAG_OVERLAPPED | FILE_FLAG_FIRST_PIPE_INSTANCE,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                PIPE_BUFFER as u32,
                PIPE_BUFFER as u32,
                0,
                None,
            );
            assert_ne!(
                server,
                INVALID_HANDLE_VALUE,
                "{}",
                io::Error::last_os_error()
            );
            let server = OwnedHandle::from_raw_handle(server.0);
            let client = CreateFileW(
                PCWSTR(name.as_ptr()),
                client,
                FILE_SHARE_NONE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                None,
            )
            .expect("open the pipe's client end");
            (server, OwnedHandle::from_raw_handle(client.0))
        }
    }

    /// The two ends of a connected duplex pipe, both overlapped.
    pub fn duplex() -> (OwnedHandle, OwnedHandle) {
        connected(PIPE_ACCESS_DUPLEX, GENERIC_READ.0 | GENERIC_WRITE.0)
    }

    /// A one-way pipe: the end to read from, and the end to write to.
    pub fn one_way() -> (OwnedHandle, OwnedHandle) {
        connected(PIPE_ACCESS_INBOUND, GENERIC_WRITE.0)
    }

    /// The two ends of a duplex pipe, as pipes.
    pub fn pipes() -> (Pipe, Pipe) {
        let (a, b) = duplex();
        (Pipe::duplex(a), Pipe::duplex(b))
    }

    /// Reads `count` bytes from `pipe`, waiting for them.
    pub fn read_exactly(pipe: &Pipe, count: usize) -> Vec<u8> {
        let mut bytes = vec![0u8; count];
        let mut filled = 0;
        while filled < count {
            match pipe.read(&mut bytes[filled..]) {
                Ok(0) => panic!("the pipe ended after {filled} of {count} bytes"),
                Ok(read) => filled += read,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    let readable = Need {
                        readable: true,
                        ..Need::default()
                    };
                    wait(&[(pipe, readable)], Duration::from_secs(10));
                }
                Err(error) => panic!("read failed: {error}"),
            }
        }
        bytes
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Weak;

    use super::testing::{PIPE_BUFFER, one_way, pipes, read_exactly};
    use super::*;
    use crate::server::event_loop::{EventLoop, Loop};

    const SOON: Option<Duration> = Some(Duration::from_secs(10));

    fn state_of(pipe: &Pipe) -> Weak<Shared> {
        Arc::downgrade(&pipe.shared)
    }

    /// Has `ev` take what has completed until `done`.
    fn turn_until(ev: &mut Loop, mut done: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut events = Vec::new();
        while !done() {
            assert!(Instant::now() < deadline, "timed out");
            ev.wait(&mut events, Some(Duration::ZERO)).unwrap();
            thread::yield_now();
        }
    }

    #[test]
    fn closing_with_a_read_in_flight_frees_the_buffers_after_its_packet() {
        let mut ev = Loop::new().unwrap();
        for round in 0..500 {
            let (pipe, mut peer) = pipes();
            let state = state_of(&pipe);
            ev.register(&pipe.source(), round, Interest::READABLE)
                .unwrap();
            match round % 5 {
                // The owner goes first, then the registration.
                0 => {
                    drop(pipe);
                    ev.deregister(round).unwrap();
                }
                1 => {
                    ev.deregister(round).unwrap();
                    drop(pipe);
                }
                // The read ends on its own just before.
                2 => {
                    assert_eq!(peer.write(b"x").unwrap(), 1);
                    drop(pipe);
                    ev.deregister(round).unwrap();
                }
                // A write the pipe can't take is in flight as well.
                3 => {
                    let big = vec![7u8; PIPE_BUFFER * 3];
                    assert_eq!(pipe.write(&big).unwrap(), big.len());
                    ev.deregister(round).unwrap();
                    drop(pipe);
                }
                // Input arrives from another thread while the owner goes.
                _ => {
                    let writer = thread::spawn(move || {
                        // Fails once the owner is gone.
                        while peer.write(&[1u8; 512]).is_ok() {}
                        peer
                    });
                    thread::yield_now();
                    drop(pipe);
                    ev.deregister(round).unwrap();
                    peer = writer.join().unwrap();
                }
            }
            // The read's packet hasn't been taken, so Windows may still
            // store its result: the state has to be there for it.
            assert!(state.upgrade().is_some(), "round {round}");
            turn_until(&mut ev, || state.upgrade().is_none());
            // The peer sees the end, not an error.
            let mut byte = [0u8; 1];
            let need = Need {
                readable: true,
                ..Need::default()
            };
            wait(&[(&peer, need)], Duration::from_secs(10));
            match peer.read(&mut byte) {
                Ok(_) => {}
                Err(error) => panic!("round {round}: {error}"),
            }
        }
    }

    #[test]
    fn dropping_the_loop_cancels_and_drains_its_sources() {
        for round in 0..100 {
            let mut ev = Loop::new().unwrap();
            let (pipe, peer) = pipes();
            let (other, _other_peer) = pipes();
            let other_state = state_of(&other);
            ev.register(&pipe.source(), 1, Interest::READABLE).unwrap();
            ev.register(&other.source(), 2, Interest::READABLE).unwrap();
            if round % 2 == 0 {
                // One owner is gone already, its read's packet untaken.
                drop(other);
                assert!(other_state.upgrade().is_some());
                drop(ev);
                assert!(other_state.upgrade().is_none(), "drained by the drop");
            } else {
                drop(ev);
                drop(other);
                assert!(other_state.upgrade().is_none());
            }
            // The pipe outlives its loop, and goes on through its events.
            assert_eq!(peer.write(b"after").unwrap(), 5);
            assert_eq!(read_exactly(&pipe, 5), b"after");
            assert_eq!(pipe.write(b"back").unwrap(), 4);
            assert_eq!(read_exactly(&peer, 4), b"back");
            // But only its own loop could have reported it.
            let mut second = Loop::new().unwrap();
            assert!(
                second
                    .register(&pipe.source(), 1, Interest::READABLE)
                    .is_err()
            );
        }
    }

    #[test]
    fn a_write_pending_before_registration_still_completes() {
        let mut ev = Loop::new().unwrap();
        let (pipe, peer) = pipes();
        // As much as the pipe source takes, and more than the pipe holds:
        // the write stays in flight, reporting through its event.
        let big = vec![3u8; WRITE_MARK];
        assert_eq!(pipe.write(&big).unwrap(), big.len());
        assert!(!pipe.sent().unwrap());
        let writable = Interest {
            readable: false,
            writable: true,
        };
        ev.register(&pipe.source(), 1, writable).unwrap();
        let mut events = Vec::new();
        ev.wait(&mut events, Some(Duration::from_millis(30)))
            .unwrap();
        assert!(events.is_empty(), "no room yet");

        // No packet tells the loop that this write has ended, and nothing
        // else looks at the pipe: the wait comes back for it by itself.
        let reader = thread::spawn(move || read_exactly(&peer, WRITE_MARK + 4));
        let start = Instant::now();
        ev.wait(&mut events, Some(Duration::from_secs(30))).unwrap();
        assert_eq!(events.len(), 1);
        assert!(events[0].writable);
        assert!(start.elapsed() < Duration::from_secs(20));
        // What follows goes through packets.
        assert_eq!(pipe.write(b"tail").unwrap(), 4);
        let got = reader.join().unwrap();
        assert!(got[..big.len()] == big[..]);
        assert_eq!(got[big.len()..], *b"tail");
    }

    #[test]
    fn two_one_way_pipes_make_one_source() {
        let mut ev = Loop::new().unwrap();
        let (input, input_peer) = one_way();
        let (output_peer, output) = one_way();
        let pipe = Pipe::of(Some(input), Some(output));
        let input_peer = Pipe::of(None, Some(input_peer));
        let output_peer = Pipe::of(Some(output_peer), None);
        let both = Interest {
            readable: true,
            writable: true,
        };
        ev.register(&pipe.source(), 1, both).unwrap();
        let mut events = Vec::new();
        ev.wait(&mut events, SOON).unwrap();
        assert_eq!(
            events,
            [Event {
                token: 1,
                readable: false,
                writable: true,
                closed: false
            }]
        );
        assert_eq!(input_peer.write(b"in").unwrap(), 2);
        turn_until(&mut ev, || pipe.input_waiting());
        assert_eq!(read_exactly(&pipe, 2), b"in");
        assert_eq!(pipe.write(b"out").unwrap(), 3);
        assert_eq!(read_exactly(&output_peer, 3), b"out");

        // With a handle each way, shutting down writes is a half-close:
        // the output's reader sees the end, and input still arrives.
        // (The handle closes once the last write's packet is taken.)
        pipe.shutdown_write();
        let mut byte = [0u8; 1];
        turn_until(&mut ev, || matches!(output_peer.read(&mut byte), Ok(0)));
        assert_eq!(
            pipe.write(b"x").unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        assert_eq!(input_peer.write(b"more").unwrap(), 4);
        turn_until(&mut ev, || pipe.input_waiting());
        assert_eq!(read_exactly(&pipe, 4), b"more");
        // An end that can't be read from or written to says so.
        assert_eq!(
            input_peer.read(&mut byte).unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
        assert_eq!(
            output_peer.write(b"x").unwrap_err().kind(),
            io::ErrorKind::Unsupported
        );
    }
}
