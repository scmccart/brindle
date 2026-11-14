//! A non-blocking framed connection for the event loops: an input buffer
//! that yields whole frames, and an outgoing queue whose length is the
//! backpressure signal. The platform provides the [`Stream`] under it.

use std::io::{self, Read, Write};
use std::time::{Duration, Instant};

use brindle_remote_protocol::{FRAME_HEADER_LEN, Frame, MAX_FRAME_PAYLOAD_SIZE};

use super::event_loop::{Interest, Source};
use super::platform::conn::{self as sys, Stream};

/// How much one `read_available` call reads before giving other sources a
/// turn; the loop is level-triggered, so the rest is read next time.
const READ_BUDGET: usize = 1024 * 1024;

pub struct Conn {
    stream: Stream,
    input: Vec<u8>,
    input_start: usize,
    output: Vec<u8>,
    output_start: usize,
    /// How many bytes the stream has taken, over the connection's life.
    written: u64,
    write_shut: bool,
}

impl Conn {
    /// Wraps a connected stream, making it non-blocking.
    pub fn new(stream: Stream) -> io::Result<Self> {
        stream.set_nonblocking()?;
        Ok(Self {
            stream,
            input: Vec::new(),
            input_start: 0,
            output: Vec::new(),
            output_start: 0,
            written: 0,
            write_shut: false,
        })
    }

    /// What an event loop waits on for this connection.
    pub fn source(&self) -> Source {
        self.stream.source()
    }

    /// Whether the peer has sent more than this side has read: bytes are
    /// waiting in the socket, or part of a frame was read.
    pub fn input_waiting(&self) -> bool {
        self.input.len() > self.input_start || self.stream.input_waiting()
    }

    /// Reads what the socket has, up to a budget. Returns `true` at EOF.
    pub fn read_available(&mut self) -> io::Result<bool> {
        let mut buffer = [0u8; 64 * 1024];
        let mut read_total = 0;
        while read_total < READ_BUDGET {
            match self.stream.read(&mut buffer) {
                Ok(0) => return Ok(true),
                Ok(read) => {
                    read_total += read;
                    self.input.extend_from_slice(&buffer[..read]);
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
        Ok(false)
    }

    /// The next whole frame read so far. An oversized header is an error,
    /// after which the stream can't be resynchronised.
    pub fn next_frame(&mut self) -> io::Result<Option<Frame>> {
        let pending = &self.input[self.input_start..];
        let Some(header) = pending.first_chunk::<FRAME_HEADER_LEN>() else {
            self.compact_input();
            return Ok(None);
        };
        let len = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
        if len > MAX_FRAME_PAYLOAD_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("frame payload is {len} bytes; maximum is {MAX_FRAME_PAYLOAD_SIZE}"),
            ));
        }
        if pending.len() < FRAME_HEADER_LEN + len {
            self.compact_input();
            return Ok(None);
        }
        let frame = Frame::new(
            header[0],
            pending[FRAME_HEADER_LEN..FRAME_HEADER_LEN + len].to_vec(),
        )?;
        self.input_start += FRAME_HEADER_LEN + len;
        Ok(Some(frame))
    }

    fn compact_input(&mut self) {
        if self.input_start > 0 {
            self.input.drain(..self.input_start);
            self.input_start = 0;
        }
    }

    /// Adds a frame to the outgoing queue; [`Conn::flush`] sends it.
    pub fn queue(&mut self, frame: &Frame) {
        self.output.push(frame.msg_type);
        self.output
            .extend_from_slice(&(frame.payload.len() as u32).to_be_bytes());
        self.output.extend_from_slice(&frame.payload);
    }

    /// Bytes waiting to be sent.
    pub fn queued(&self) -> usize {
        self.output.len() - self.output_start
    }

    /// How many bytes the socket or pipe has taken so far. The peer reads
    /// those even if this side ends right after: they are no longer this
    /// process's to lose. A Windows stream buffers writes of its own
    /// ([`Stream::unsent`]), which a process that ends cancels, so those
    /// don't count yet.
    pub fn written(&self) -> u64 {
        self.written - self.stream.unsent() as u64
    }

    /// What [`Conn::written`] is once everything queued so far is sent:
    /// where the last queued frame ends.
    pub fn queued_end(&self) -> u64 {
        self.written + self.queued() as u64
    }

    /// Sends as much of the queue as the socket takes without blocking.
    pub fn flush(&mut self) -> io::Result<()> {
        while self.output_start < self.output.len() {
            match self.stream.write(&self.output[self.output_start..]) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(written) => {
                    self.output_start += written;
                    self.written += written as u64;
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
        if self.output_start == self.output.len() {
            self.output.clear();
            self.output_start = 0;
        } else if self.output_start > 1024 * 1024 && self.output_start * 2 > self.output.len() {
            self.output.drain(..self.output_start);
            self.output_start = 0;
        }
        Ok(())
    }

    /// One non-blocking step of a graceful close: sends what the socket
    /// takes, shuts the write side down once the queue is empty, and reads
    /// what the peer sends (into the input, for [`Conn::next_frame`]).
    /// Returns `true` once the peer has closed its side too.
    ///
    /// The point is never to close a socket with unread input: Linux then
    /// resets the connection, and the peer loses what it hadn't read yet,
    /// our last messages included.
    pub fn close_step(&mut self) -> io::Result<bool> {
        if !self.write_shut {
            self.flush()?;
            if self.queued() == 0 {
                self.write_shut = true;
                // The peer may have closed already; reading tells.
                let _ = self.stream.shutdown_write();
            }
        }
        self.read_available()
    }

    /// Drops what was read and not yet taken as frames.
    pub fn discard_input(&mut self) {
        self.input.clear();
        self.input_start = 0;
    }

    /// Closes gracefully ([`Conn::close_step`]) within `limit`, discarding
    /// what the peer sends meanwhile. An error names what was left undone.
    pub fn close_for(&mut self, limit: Duration) -> io::Result<()> {
        let deadline = Instant::now() + limit;
        loop {
            let closed = self.close_step();
            self.discard_input();
            if closed? {
                return Ok(());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(self.unfinished());
            }
            wait(&[&*self], remaining);
        }
    }

    /// Why a graceful close didn't finish.
    pub fn unfinished(&self) -> io::Error {
        // What the stream took and hasn't passed on is unsent too (Windows).
        let unsent = self.queued() + self.stream.unsent();
        let message = if self.write_shut && unsent == 0 {
            "the peer didn't close its side".to_string()
        } else {
            format!("the peer stopped reading with {unsent} bytes unsent")
        };
        io::Error::new(io::ErrorKind::TimedOut, message)
    }

    /// Flushes the whole queue, waiting for the peer for at most `limit`.
    /// Once it returns `Ok`, dropping the connection loses none of it.
    pub fn flush_for(&mut self, limit: Duration) -> io::Result<()> {
        let deadline = Instant::now() + limit;
        let timed_out = || io::Error::new(io::ErrorKind::TimedOut, "the peer stopped reading");
        loop {
            self.flush()?;
            let remaining = deadline.saturating_duration_since(Instant::now());
            if self.queued() == 0 {
                // The stream may still hold part of it (Windows), which a
                // drop would cancel.
                return match self.stream.drain_for(remaining)? {
                    true => Ok(()),
                    false => Err(timed_out()),
                };
            }
            if remaining.is_zero() {
                return Err(timed_out());
            }
            let writable = Interest {
                readable: false,
                writable: true,
            };
            sys::wait(&[(&self.stream, writable)], wait_slice(remaining));
        }
    }
}

/// Waits up to `limit` (at most 100 ms at a time) until one of `conns` can
/// make progress on a graceful close: readable, or writable while it has
/// output queued.
pub fn wait(conns: &[&Conn], limit: Duration) {
    let streams: Vec<(&Stream, Interest)> = conns
        .iter()
        .map(|conn| {
            let interest = Interest {
                readable: true,
                writable: !conn.write_shut && conn.queued() > 0,
            };
            (&conn.stream, interest)
        })
        .collect();
    sys::wait(&streams, wait_slice(limit));
}

/// One wait of a blocking flush or close: `limit`, but between 1 and 100 ms.
fn wait_slice(limit: Duration) -> Duration {
    Duration::from_millis(limit.as_millis().clamp(1, 100) as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frames_survive_any_split() {
        let frames = [
            Frame::new(1, b"hello".to_vec()).unwrap(),
            Frame::new(2, Vec::new()).unwrap(),
            Frame::new(3, vec![7; 70_000]).unwrap(),
        ];
        let (a, b) = Stream::pair().unwrap();
        let mut sender = Conn::new(a).unwrap();
        let mut receiver = Conn::new(b).unwrap();
        for frame in &frames {
            sender.queue(frame);
        }
        assert_eq!(sender.queued(), 15 + 70_005);
        let mut got = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while got.len() < frames.len() && Instant::now() < deadline {
            sender.flush().unwrap();
            assert!(!receiver.read_available().unwrap());
            while let Some(frame) = receiver.next_frame().unwrap() {
                got.push(frame);
            }
        }
        assert_eq!(got, frames);
        assert_eq!(sender.queued(), 0);
        drop(sender);
        assert!(receiver.read_available().unwrap(), "EOF");
    }

    /// Linux only: a socket takes part of a write. A Windows pipe takes
    /// each overlapped write whole, once its buffer has room, so with an
    /// unread peer nothing counts as written yet; `tests/core.rs`'s
    /// `an_attach_holds_when_the_core_is_killed_as_its_replay_starts`
    /// covers what the count is for on both.
    #[cfg(unix)]
    #[test]
    fn the_written_count_stops_where_the_socket_does() {
        let (a, b) = Stream::pair().unwrap();
        let mut sender = Conn::new(a).unwrap();
        let mut receiver = Conn::new(b).unwrap();
        assert_eq!((sender.written(), sender.queued_end()), (0, 0));
        sender.queue(&Frame::new(1, b"first".to_vec()).unwrap());
        let first_end = sender.queued_end();
        assert_eq!((sender.written(), first_end), (0, 10));
        // Far more than a socket holds while its peer doesn't read.
        let big = Frame::new(2, vec![7; 1024 * 1024]).unwrap();
        for _ in 0..8 {
            sender.queue(&big);
        }
        let end = sender.queued_end();
        assert_eq!(end, 10 + 8 * (5 + 1024 * 1024));
        sender.flush().unwrap();
        let taken = sender.written();
        assert!(taken >= first_end && taken < end, "{taken} of {end}");
        assert_eq!(sender.queued_end(), end, "queued frames end where they did");

        let deadline = Instant::now() + Duration::from_secs(10);
        while sender.written() < end && Instant::now() < deadline {
            assert!(!receiver.read_available().unwrap());
            while receiver.next_frame().unwrap().is_some() {}
            sender.flush().unwrap();
        }
        assert_eq!((sender.written(), sender.queued()), (end, 0));
    }

    #[test]
    fn input_waiting_sees_unread_bytes_and_partial_frames() {
        let (mut a, b) = Stream::pair().unwrap();
        let mut receiver = Conn::new(b).unwrap();
        assert!(!receiver.input_waiting());
        let frame = [1, 0, 0, 0, 2, b'h', b'i'];
        a.write_all(&frame).unwrap();
        assert!(receiver.input_waiting(), "unread in the socket");
        receiver.read_available().unwrap();
        assert!(receiver.input_waiting(), "read, but not taken as a frame");
        assert!(receiver.next_frame().unwrap().is_some());
        assert!(!receiver.input_waiting());
        a.write_all(&frame[..3]).unwrap();
        receiver.read_available().unwrap();
        assert!(receiver.next_frame().unwrap().is_none());
        assert!(receiver.input_waiting(), "part of a frame");
    }

    #[test]
    fn a_graceful_close_delivers_the_queue_and_then_the_end() {
        // More than a socket's or a pipe's buffer holds.
        let frame = Frame::new(9, vec![3; 400_000]).unwrap();
        let (a, b) = Stream::pair().unwrap();
        let mut closer = Conn::new(a).unwrap();
        let mut peer = Conn::new(b).unwrap();
        closer.queue(&frame);
        let reader = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut got = Vec::new();
            loop {
                let eof = peer.read_available().unwrap();
                while let Some(frame) = peer.next_frame().unwrap() {
                    got.push(frame);
                }
                if eof {
                    // Dropping `peer` closes its side, which ends the close.
                    return got;
                }
                assert!(
                    Instant::now() < deadline,
                    "no EOF after {} frames",
                    got.len()
                );
                wait(&[&peer], Duration::from_millis(100));
            }
        });
        closer.close_for(Duration::from_secs(10)).unwrap();
        assert_eq!(reader.join().unwrap(), [frame]);
    }

    #[test]
    fn a_drop_after_a_timed_flush_loses_nothing() {
        // More than a socket's or a pipe's buffer holds, so the peer has to
        // read before the last of it has left this side.
        let frame = Frame::new(9, vec![5; 400_000]).unwrap();
        let (a, b) = Stream::pair().unwrap();
        let mut sender = Conn::new(a).unwrap();
        let mut peer = Conn::new(b).unwrap();
        sender.queue(&frame);
        let reader = std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            // The sender is waiting by now.
            std::thread::sleep(Duration::from_millis(50));
            loop {
                let eof = peer.read_available().unwrap();
                if let Some(frame) = peer.next_frame().unwrap() {
                    return frame;
                }
                assert!(!eof, "the connection ended before the frame did");
                assert!(Instant::now() < deadline, "no frame");
                wait(&[&peer], Duration::from_millis(100));
            }
        });
        sender.flush_for(Duration::from_secs(10)).unwrap();
        drop(sender);
        assert_eq!(reader.join().unwrap(), frame);

        // A peer that doesn't read is reported, not waited for.
        let (a, _b) = Stream::pair().unwrap();
        let mut sender = Conn::new(a).unwrap();
        sender.queue(&frame);
        let error = sender.flush_for(Duration::from_millis(50)).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(sender.unfinished().to_string().contains("bytes unsent"));
    }

    #[test]
    fn an_oversized_header_is_an_error() {
        let (mut a, b) = Stream::pair().unwrap();
        let mut receiver = Conn::new(b).unwrap();
        a.write_all(&[1, 0xff, 0xff, 0xff, 0xff]).unwrap();
        receiver.read_available().unwrap();
        assert_eq!(
            receiver.next_frame().unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}
