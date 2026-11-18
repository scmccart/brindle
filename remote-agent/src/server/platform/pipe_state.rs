//! The bookkeeping of a Windows overlapped pipe, without its handles: what
//! a finished read or write means, what is buffered and queued, and when
//! the pipe is readable, writable, sent or hung up.
//!
//! A GPUI-free model, so it is tested on every platform. The Windows
//! `pipe` keeps the handles, the operations in flight, the read buffer's
//! memory and the unsafe slices over what is being sent; it records each
//! operation's end here and asks this model what to do next.

use std::mem;

/// The Win32 codes a finished operation is told apart by, as plain numbers
/// so the model builds everywhere. A Windows test checks them against the
/// `windows` crate's.
pub const NO_ERROR: u32 = 0;
pub const WRITE_FAULT: u32 = 29;
pub const HANDLE_EOF: u32 = 38;
pub const BROKEN_PIPE: u32 = 109;
pub const MORE_DATA: u32 = 234;
pub const OPERATION_ABORTED: u32 = 995;

/// A pipe takes writes while fewer bytes than this wait for the pipe.
pub const WRITE_MARK: usize = 1024 * 1024;

/// A write buffer that grew past this is freed once it is empty.
pub const KEEP: usize = 64 * 1024;

/// Whether a side's handle is still open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    Open,
    /// Closed: the owner is gone, or shut a duplex pipe down.
    Closed,
}

/// What a finished read means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadEnd {
    /// It filled the buffer with this many bytes.
    Bytes(usize),
    /// The end of input.
    End,
    /// It was cancelled: nothing was read, and a later read may follow.
    Cancelled,
    Failed(u32),
}

/// What a read that ended with `bytes` and `error` means. A read of nothing
/// is a message pipe's empty message, which stands for the end like
/// `Read`'s `Ok(0)`; the peer closing its handle (`ERROR_BROKEN_PIPE`) is
/// the end too.
pub fn read_ended(bytes: u32, error: u32) -> ReadEnd {
    match error {
        NO_ERROR | MORE_DATA if bytes > 0 => ReadEnd::Bytes(bytes as usize),
        NO_ERROR | MORE_DATA | BROKEN_PIPE | HANDLE_EOF => ReadEnd::End,
        OPERATION_ABORTED => ReadEnd::Cancelled,
        error => ReadEnd::Failed(error),
    }
}

/// What a finished write means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WriteEnd {
    /// The pipe took this many bytes.
    Took(usize),
    Failed(u32),
}

/// What a write that ended with `bytes` and `error` means. A write that
/// took nothing would be issued again for ever, so it fails.
pub fn write_ended(bytes: u32, error: u32) -> WriteEnd {
    match error {
        NO_ERROR if bytes > 0 => WriteEnd::Took(bytes as usize),
        NO_ERROR => WriteEnd::Failed(WRITE_FAULT),
        error => WriteEnd::Failed(error),
    }
}

/// The read side: what the one read buffer holds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReadState {
    /// The bytes read and not yet taken are `buffer[start..end]`.
    pub start: usize,
    pub end: usize,
    /// No read will be issued again: the buffer holds the last input.
    pub eof: bool,
    pub error: Option<u32>,
}

impl ReadState {
    /// Whether there is something to read: bytes, the end or an error.
    pub fn ready(&self) -> bool {
        self.end > self.start || self.eof || self.error.is_some()
    }

    /// Records a finished read. A reader whose handle is closed is at its
    /// end, whatever the read said.
    pub fn apply(&mut self, end: ReadEnd, handle: Handle) {
        match end {
            ReadEnd::Bytes(count) => self.end = count,
            ReadEnd::End => self.eof = true,
            ReadEnd::Cancelled => {}
            ReadEnd::Failed(error) => self.error = Some(error),
        }
        if handle == Handle::Closed {
            self.eof = true;
        }
    }
}

/// The write side: what is being sent and what waits to be.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WriteState {
    /// The write in flight covers `sending[sent..]`; it isn't touched
    /// meanwhile.
    pub sending: Vec<u8>,
    pub sent: usize,
    /// What the next write takes.
    pub queued: Vec<u8>,
    pub error: Option<u32>,
    /// The owner shut the writes down: the handle closes once everything
    /// is sent.
    pub shut: bool,
}

/// What the writer does next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rotation {
    /// Issue a write of `sending[sent..]`.
    Send,
    /// Nothing to send, or the writer failed.
    Idle,
    /// A shut writer sent everything: close its handle.
    Close,
}

impl WriteState {
    /// Bytes the pipe hasn't taken yet.
    pub fn unsent(&self) -> usize {
        self.sending.len() - self.sent + self.queued.len()
    }

    /// Whether the pipe takes writes: under [`WRITE_MARK`], or failed (the
    /// write then fails).
    pub fn ready(&self) -> bool {
        self.unsent() < WRITE_MARK || self.error.is_some()
    }

    /// Fails the writer with `error`, dropping whatever wasn't sent.
    pub fn fail(&mut self, error: u32) {
        self.error = Some(error);
        self.sending = Vec::new();
        self.sent = 0;
        self.queued = Vec::new();
    }

    /// Records a finished write. A count beyond what was in flight takes
    /// only that.
    pub fn apply(&mut self, end: WriteEnd) {
        match end {
            WriteEnd::Took(count) => {
                let left = self.sending.len() - self.sent;
                self.sent += left.min(count);
            }
            WriteEnd::Failed(error) => self.fail(error),
        }
    }

    /// What to do once no write is in flight. When everything in flight is
    /// sent, the queue is sent next and the emptied buffer becomes the
    /// queue, unless it grew past [`KEEP`]. With nothing left, a shut writer
    /// closes. Output with no `Open` handle to go to fails the writer.
    pub fn rotate(&mut self, handle: Handle) -> Rotation {
        if self.error.is_some() {
            return Rotation::Idle;
        }
        if self.sent == self.sending.len() {
            self.sending.clear();
            self.sent = 0;
            mem::swap(&mut self.sending, &mut self.queued);
            if self.queued.capacity() > KEEP {
                self.queued = Vec::new();
            }
            if self.sending.is_empty() {
                return if self.shut && handle == Handle::Open {
                    Rotation::Close
                } else {
                    Rotation::Idle
                };
            }
        }
        match handle {
            Handle::Open => Rotation::Send,
            Handle::Closed => {
                self.fail(BROKEN_PIPE);
                Rotation::Idle
            }
        }
    }
}

/// What a wait on a pipe is for.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Need {
    /// Input, its end, or an error to read.
    pub readable: bool,
    /// Room for a write.
    pub writable: bool,
    /// The pipe having taken everything written.
    pub sent: bool,
}

/// A pipe's two sides, either of which it may lack, for its readiness.
#[derive(Debug, Clone, Copy)]
pub struct Sides<'a> {
    pub reader: Option<&'a ReadState>,
    pub writer: Option<&'a WriteState>,
}

impl Sides<'_> {
    pub fn readable(&self) -> bool {
        self.reader.is_some_and(ReadState::ready)
    }

    pub fn writable(&self) -> bool {
        self.writer.is_some_and(WriteState::ready)
    }

    /// Whether the pipe has taken everything written, or never will.
    pub fn sent(&self) -> bool {
        self.writer
            .is_none_or(|writer| writer.unsent() == 0 || writer.error.is_some())
    }

    /// The peer is gone or the pipe failed: a read returns the end or an
    /// error (for a pipe with nothing to read from, a write fails). So a
    /// handler that reads when told of it always gets one of the two.
    pub fn hung_up(&self) -> bool {
        match (self.reader, self.writer) {
            (Some(reader), _) => reader.eof || reader.error.is_some(),
            (None, Some(writer)) => writer.error.is_some(),
            (None, None) => false,
        }
    }

    /// Whether a wait for `need` is over. A hang-up ends it too: the peer
    /// closing ends the input, and fails a write that is waiting.
    pub fn meets(&self, need: Need) -> bool {
        (need.readable && self.readable())
            || (need.writable && self.writable())
            || (need.sent && self.sent())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reader() -> ReadState {
        ReadState::default()
    }

    fn writer(sending: usize, sent: usize, queued: usize) -> WriteState {
        WriteState {
            sending: vec![1; sending],
            sent,
            queued: vec![2; queued],
            ..WriteState::default()
        }
    }

    #[test]
    fn a_read_of_bytes_fills_the_buffer_and_a_read_of_nothing_is_the_end() {
        assert_eq!(read_ended(5, NO_ERROR), ReadEnd::Bytes(5));
        // A message pipe read as messages reports a longer message so.
        assert_eq!(read_ended(5, MORE_DATA), ReadEnd::Bytes(5));
        assert_eq!(read_ended(0, NO_ERROR), ReadEnd::End);
        assert_eq!(read_ended(0, MORE_DATA), ReadEnd::End);

        let mut state = reader();
        assert!(!state.ready());
        state.apply(ReadEnd::Bytes(5), Handle::Open);
        assert_eq!((state.start, state.end), (0, 5));
        assert!(state.ready() && !state.eof);
        state.start = 5;
        assert!(!state.ready());
        state.apply(ReadEnd::End, Handle::Open);
        assert!(state.eof && state.ready());
    }

    #[test]
    fn a_broken_pipe_or_eof_is_the_end() {
        for error in [BROKEN_PIPE, HANDLE_EOF] {
            // Whatever the count says.
            assert_eq!(read_ended(0, error), ReadEnd::End);
            assert_eq!(read_ended(3, error), ReadEnd::End);
        }
    }

    #[test]
    fn a_cancelled_read_changes_nothing() {
        assert_eq!(read_ended(0, OPERATION_ABORTED), ReadEnd::Cancelled);
        let mut state = reader();
        state.apply(ReadEnd::Cancelled, Handle::Open);
        assert_eq!(state, reader());
        assert!(!state.ready());
    }

    #[test]
    fn another_read_error_is_kept() {
        // ERROR_PIPE_NOT_CONNECTED.
        assert_eq!(read_ended(0, 233), ReadEnd::Failed(233));
        assert_eq!(read_ended(7, WRITE_FAULT), ReadEnd::Failed(WRITE_FAULT));
        let mut state = reader();
        state.apply(ReadEnd::Failed(233), Handle::Open);
        assert_eq!(state.error, Some(233));
        assert!(!state.eof && state.ready());
    }

    #[test]
    fn a_closed_reader_is_at_its_end_whatever_the_read_said() {
        for end in [
            ReadEnd::Bytes(4),
            ReadEnd::End,
            ReadEnd::Cancelled,
            ReadEnd::Failed(233),
        ] {
            let mut state = reader();
            state.apply(end, Handle::Closed);
            assert!(state.eof, "{end:?}");
            assert!(state.ready(), "{end:?}");
        }
        // What was read is still there to take.
        let mut state = reader();
        state.apply(ReadEnd::Bytes(4), Handle::Closed);
        assert_eq!(state.end, 4);
    }

    #[test]
    fn a_write_takes_at_most_what_was_sent() {
        assert_eq!(write_ended(10, NO_ERROR), WriteEnd::Took(10));
        let mut state = writer(10, 4, 3);
        state.apply(WriteEnd::Took(2));
        assert_eq!(state.sent, 6);
        assert_eq!(state.unsent(), 4 + 3);
        // A count beyond what was in flight takes only that.
        state.apply(WriteEnd::Took(100));
        assert_eq!(state.sent, 10);
        assert_eq!(state.unsent(), 3);
        assert_eq!(state.error, None);
    }

    #[test]
    fn a_write_that_took_nothing_fails_with_write_fault() {
        assert_eq!(write_ended(0, NO_ERROR), WriteEnd::Failed(WRITE_FAULT));
        // Any error fails it, whatever the count, an end of the pipe too.
        assert_eq!(write_ended(3, BROKEN_PIPE), WriteEnd::Failed(BROKEN_PIPE));
        assert_eq!(write_ended(0, 232), WriteEnd::Failed(232));
        assert_eq!(write_ended(5, MORE_DATA), WriteEnd::Failed(MORE_DATA));
    }

    #[test]
    fn a_failed_write_drops_what_was_queued() {
        let mut state = writer(10, 4, 3);
        state.apply(WriteEnd::Failed(232));
        assert_eq!(state.error, Some(232));
        assert_eq!(state.unsent(), 0);
        assert!(state.sending.is_empty() && state.queued.is_empty());
        assert_eq!(state.sent, 0);

        let mut state = writer(10, 4, 3);
        state.fail(BROKEN_PIPE);
        assert_eq!(state.error, Some(BROKEN_PIPE));
        assert_eq!(state.unsent(), 0);
    }

    #[test]
    fn the_writer_is_writable_under_the_mark_or_once_failed() {
        assert!(writer(0, 0, 0).ready());
        assert!(writer(0, 0, WRITE_MARK - 1).ready());
        assert!(!writer(0, 0, WRITE_MARK).ready());
        // What is in flight counts, what the pipe took doesn't.
        assert!(!writer(WRITE_MARK / 2, 0, WRITE_MARK / 2).ready());
        assert!(writer(WRITE_MARK / 2, 1, WRITE_MARK / 2).ready());
        // A failed writer is writable: the write then fails.
        let mut state = writer(0, 0, WRITE_MARK);
        state.error = Some(BROKEN_PIPE);
        assert!(state.ready());
    }

    #[test]
    fn rotating_drops_a_large_queues_capacity() {
        // Something left to send in flight: nothing moves.
        let mut state = writer(10, 4, 3);
        assert_eq!(state.rotate(Handle::Open), Rotation::Send);
        assert_eq!((state.sending.len(), state.sent, state.queued.len()), (10, 4, 3));

        // All sent: the queue is sent next, and the sent buffer becomes the
        // queue, empty.
        let mut state = writer(10, 10, 3);
        assert_eq!(state.rotate(Handle::Open), Rotation::Send);
        assert_eq!(state.sending, [2; 3]);
        assert_eq!((state.sent, state.queued.len()), (0, 0));
        assert!(state.queued.capacity() >= 10);

        // A sent buffer that grew past `KEEP` isn't kept as the queue.
        let mut state = writer(KEEP + 1, KEEP + 1, 3);
        assert_eq!(state.rotate(Handle::Open), Rotation::Send);
        assert_eq!(state.queued.capacity(), 0);
        let mut state = writer(KEEP, KEEP, 3);
        assert_eq!(state.rotate(Handle::Open), Rotation::Send);
        assert!(state.queued.capacity() >= KEEP);

        // Nothing to send.
        let mut state = writer(0, 0, 0);
        assert_eq!(state.rotate(Handle::Open), Rotation::Idle);
        // A failed writer sends nothing.
        let mut state = writer(0, 0, 0);
        state.fail(BROKEN_PIPE);
        assert_eq!(state.rotate(Handle::Open), Rotation::Idle);
        // Output with no handle to write it to fails the writer.
        let mut state = writer(0, 0, 3);
        assert_eq!(state.rotate(Handle::Closed), Rotation::Idle);
        assert_eq!(state.error, Some(BROKEN_PIPE));
        assert_eq!(state.unsent(), 0);
    }

    #[test]
    fn a_shut_writer_closes_once_everything_is_sent() {
        let mut state = writer(10, 4, 3);
        state.shut = true;
        assert_eq!(state.rotate(Handle::Open), Rotation::Send);
        state.sent = 10;
        assert_eq!(state.rotate(Handle::Open), Rotation::Send);
        state.sent = 3;
        assert_eq!(state.rotate(Handle::Open), Rotation::Close);
        assert_eq!(state.unsent(), 0);
        // Once closed, there is nothing more to do.
        assert_eq!(state.rotate(Handle::Closed), Rotation::Idle);
        // A shut writer that failed doesn't close: it stays failed.
        let mut state = writer(0, 0, 0);
        state.shut = true;
        state.fail(BROKEN_PIPE);
        assert_eq!(state.rotate(Handle::Open), Rotation::Idle);
    }

    #[test]
    fn hung_up_is_the_readers_end_or_the_writers_error_without_a_reader() {
        let (open, mut ended, mut failed) = (reader(), reader(), reader());
        ended.eof = true;
        failed.error = Some(233);
        let (fine, mut broken) = (writer(0, 0, 0), writer(0, 0, 0));
        broken.fail(BROKEN_PIPE);

        fn hung_up(reader: Option<&ReadState>, writer: Option<&WriteState>) -> bool {
            Sides { reader, writer }.hung_up()
        }
        assert!(!hung_up(Some(&open), Some(&fine)));
        assert!(hung_up(Some(&ended), Some(&fine)));
        assert!(hung_up(Some(&failed), None));
        // With a reader, only the reader tells.
        assert!(!hung_up(Some(&open), Some(&broken)));
        assert!(hung_up(None, Some(&broken)));
        assert!(!hung_up(None, Some(&fine)));
        assert!(!hung_up(None, None));
    }

    #[test]
    fn a_wait_is_over_when_any_need_is_met() {
        let readable = Need {
            readable: true,
            ..Need::default()
        };
        let writable = Need {
            writable: true,
            ..Need::default()
        };
        let sent = Need {
            sent: true,
            ..Need::default()
        };
        let (empty, mut holding) = (reader(), reader());
        holding.end = 3;
        let (full, idle, mut in_flight) = (writer(0, 0, WRITE_MARK), writer(0, 0, 0), writer(4, 0, 0));
        fn sides<'a>(reader: Option<&'a ReadState>, writer: Option<&'a WriteState>) -> Sides<'a> {
            Sides { reader, writer }
        }

        let quiet = sides(Some(&empty), Some(&full));
        assert!(!quiet.readable() && !quiet.writable() && !quiet.sent());
        assert!(!quiet.meets(readable) && !quiet.meets(writable) && !quiet.meets(sent));
        assert!(!quiet.meets(Need::default()));

        let input = sides(Some(&holding), Some(&full));
        assert!(input.readable() && input.meets(readable));
        assert!(!input.meets(writable));
        let both = Need {
            readable: true,
            writable: true,
            sent: false,
        };
        assert!(input.meets(both));

        let room = sides(Some(&empty), Some(&in_flight));
        assert!(room.writable() && room.meets(writable));
        assert!(!room.sent() && !room.meets(sent));
        in_flight.sent = 4;
        let room = sides(Some(&empty), Some(&in_flight));
        assert!(room.sent() && room.meets(sent));
        assert!(sides(None, Some(&idle)).meets(sent));

        // No writer has nothing to send, and no side is ever readable or
        // writable.
        let none = sides(None, None);
        assert!(none.sent() && !none.readable() && !none.writable());
        // A failed writer is sent: it never will be.
        let mut failed = writer(4, 0, 0);
        failed.fail(BROKEN_PIPE);
        assert!(sides(None, Some(&failed)).meets(sent));
    }
}
