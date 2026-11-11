//! The stream under a framed connection: one end of a duplex named pipe,
//! opened with `FILE_FLAG_OVERLAPPED`.
//!
//! The stream reads and writes through the buffers of its pipe source
//! ([`super::pipe`]), so it works the same registered with an event loop or
//! on its own. What differs from a socket:
//!
//! - **Writes are buffered here.** A non-blocking write takes bytes while
//!   fewer than [`WRITE_MARK`](super::pipe::WRITE_MARK) (1 MiB) wait for
//!   the pipe, then fails with `WouldBlock`. So a connection's own queue
//!   starts to grow that much later than on Linux. A blocking write returns
//!   once the pipe has taken its bytes, like a write to a socket.
//! - **Dropping** the stream cancels what the pipe hadn't taken yet, as
//!   closing a socket drops what the kernel hadn't accepted. What the pipe
//!   has taken stays readable by the peer, also when this side leaves
//!   input unread: a pipe has no reset.
//! - **[`Stream::shutdown_write`]** can't half-close: a pipe handle is
//!   either open or closed. It closes the handle once the pipe has taken
//!   everything written, so the peer reads every byte and then the end, as
//!   after a socket's `shutdown`. This side then reads the end too, and
//!   what the peer sends from then on is lost, where Linux would still
//!   read it. A graceful close is therefore: write, shut down, read until
//!   the end, which comes when the last byte has gone out.
//! - **Errors:** a read returns `Ok(0)` when the peer has closed
//!   (`ERROR_BROKEN_PIPE`), after everything the peer wrote. A write to a
//!   closed pipe fails with `BrokenPipe` (`ERROR_NO_DATA`). A server end
//!   that disconnects its client discards what was unread, and both
//!   directions then fail with `ERROR_PIPE_NOT_CONNECTED`.

use std::io::{self, Read, Write};
use std::os::windows::io::OwnedHandle;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use super::event_loop::Source;
use super::pipe::{self, Need, Pipe};
use crate::server::event_loop::Interest;

/// How long one wait of a blocking read or write lasts before it looks
/// again.
const BLOCK: Duration = Duration::from_secs(60);

const READABLE: Need = Need {
    readable: true,
    writable: false,
    sent: false,
};

const WRITABLE: Need = Need {
    readable: false,
    writable: true,
    sent: false,
};

const SENT: Need = Need {
    readable: false,
    writable: false,
    sent: true,
};

/// A connected named pipe. It blocks until [`Stream::set_nonblocking`],
/// after which reads and writes that can't proceed fail with `WouldBlock`.
pub struct Stream {
    pipe: Pipe,
    nonblocking: AtomicBool,
    read_timeout: Mutex<Option<Duration>>,
}

impl Stream {
    /// Wraps one end of a connected duplex pipe, opened for overlapped I/O
    /// and read as bytes.
    #[allow(dead_code, reason = "for the listener and connecting (task 3.2)")]
    pub(super) fn new(handle: OwnedHandle) -> Self {
        Self {
            pipe: Pipe::duplex(handle),
            nonblocking: AtomicBool::new(false),
            read_timeout: Mutex::new(None),
        }
    }

    pub fn set_nonblocking(&self) -> io::Result<()> {
        self.nonblocking.store(true, Ordering::Release);
        Ok(())
    }

    /// How long a blocking read waits (`None` waits forever), after which
    /// it fails with `TimedOut`.
    pub fn set_read_timeout(&self, limit: Option<Duration>) -> io::Result<()> {
        *self
            .read_timeout
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = limit;
        Ok(())
    }

    /// What an event loop waits on for this stream.
    pub fn source(&self) -> Source {
        self.pipe.source()
    }

    /// Whether bytes are waiting to be read: taken from the pipe already,
    /// or still in it.
    pub fn input_waiting(&self) -> bool {
        self.pipe.input_waiting()
    }

    /// Ends this side's output; the peer reads EOF after what was sent.
    /// The pipe's handle closes when the last byte has gone out, so this
    /// side reads EOF then as well (see the module's notes).
    pub fn shutdown_write(&self) -> io::Result<()> {
        self.pipe.shutdown_write();
        Ok(())
    }

    fn blocking(&self) -> bool {
        !self.nonblocking.load(Ordering::Acquire)
    }

    /// Waits until the pipe has taken everything written.
    fn finish_sending(&self) -> io::Result<()> {
        while !self.pipe.sent()? {
            pipe::wait(&[(&self.pipe, SENT)], BLOCK);
        }
        Ok(())
    }

    /// Two connected streams.
    #[cfg(test)]
    pub fn pair() -> io::Result<(Self, Self)> {
        let (a, b) = pipe::testing::duplex();
        Ok((Self::new(a), Self::new(b)))
    }
}

impl Read for Stream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let limit = *self
            .read_timeout
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let start = Instant::now();
        loop {
            match self.pipe.read(buffer) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock && self.blocking() => {
                    let turn = match limit {
                        Some(limit) => limit.saturating_sub(start.elapsed()),
                        None => BLOCK,
                    };
                    if turn.is_zero() {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "nothing arrived in time",
                        ));
                    }
                    pipe::wait(&[(&self.pipe, READABLE)], turn.min(BLOCK));
                }
                result => return result,
            }
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        loop {
            match self.pipe.write(buffer) {
                Err(error) if error.kind() == io::ErrorKind::WouldBlock && self.blocking() => {
                    pipe::wait(&[(&self.pipe, WRITABLE)], BLOCK);
                }
                Ok(count) if self.blocking() => {
                    self.finish_sending()?;
                    return Ok(count);
                }
                result => return result,
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.blocking() {
            self.finish_sending()?;
        }
        Ok(())
    }
}

/// Waits up to `timeout` until one of `streams` is ready for what it is
/// waited for, or has hung up.
pub fn wait(streams: &[(&Stream, Interest)], timeout: Duration) {
    let pipes: Vec<(&Pipe, Need)> = streams
        .iter()
        .map(|(stream, interest)| (&stream.pipe, Need::from(*interest)))
        .collect();
    pipe::wait(&pipes, timeout);
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::super::pipe::WRITE_MARK;
    use super::super::pipe::testing::PIPE_BUFFER;
    use super::*;
    use crate::server::event_loop::{EventLoop, Loop};

    /// Position-dependent bytes: a block whose length shares no factor with
    /// any buffer size, repeated.
    fn pattern() -> Vec<u8> {
        let mut state = 0x2545_f491_4f6c_dd1du64;
        (0..1_000_003)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                (state >> 32) as u8
            })
            .collect()
    }

    /// Checks `bytes` against the pattern at `offset`.
    fn check(pattern: &[u8], offset: u64, bytes: &[u8]) {
        let mut at = (offset % pattern.len() as u64) as usize;
        let mut bytes = bytes;
        while !bytes.is_empty() {
            let count = bytes.len().min(pattern.len() - at);
            assert!(
                bytes[..count] == pattern[at..at + count],
                "the stream differs from what was sent near byte {offset}"
            );
            bytes = &bytes[count..];
            at = (at + count) % pattern.len();
        }
    }

    /// Writes `total` bytes of the pattern through a blocking stream.
    fn send_pattern(stream: &mut Stream, pattern: &[u8], total: u64) {
        let mut sent = 0u64;
        while sent < total {
            let at = (sent % pattern.len() as u64) as usize;
            let count = (pattern.len() - at).min((total - sent) as usize);
            stream.write_all(&pattern[at..at + count]).unwrap();
            sent += count as u64;
        }
    }

    #[test]
    fn a_flood_arrives_in_order() {
        const TOTAL: u64 = 64 * 1024 * 1024;
        let (mut sender, receiver) = Stream::pair().unwrap();
        let writer = thread::spawn(move || {
            send_pattern(&mut sender, &pattern(), TOTAL);
            // Dropped here: everything was taken by the pipe.
        });
        let pattern = pattern();
        let mut ev = Loop::new().unwrap();
        let mut receiver = receiver;
        receiver.set_nonblocking().unwrap();
        ev.register(&receiver.source(), 1, Interest::READABLE)
            .unwrap();
        let mut events = Vec::new();
        let mut buffer = vec![0u8; 48 * 1024];
        let mut received = 0u64;
        'flood: loop {
            ev.wait(&mut events, Some(Duration::from_secs(30))).unwrap();
            assert_eq!(events.len(), 1, "the flood stalled at byte {received}");
            // A fair share per turn, as a handler takes.
            for _ in 0..8 {
                match receiver.read(&mut buffer) {
                    Ok(0) => break 'flood,
                    Ok(count) => {
                        check(&pattern, received, &buffer[..count]);
                        received += count as u64;
                    }
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                    Err(error) => panic!("read failed at byte {received}: {error}"),
                }
            }
        }
        assert_eq!(received, TOTAL);
        writer.join().unwrap();
    }

    #[test]
    fn a_flood_arrives_in_order_between_two_loops() {
        // Both ends non-blocking, each on its own loop, with the sender
        // held back by the mark.
        const TOTAL: u64 = 16 * 1024 * 1024;
        let (sender, mut receiver) = Stream::pair().unwrap();
        let writer = thread::spawn(move || {
            let pattern = pattern();
            let mut sender = sender;
            sender.set_nonblocking().unwrap();
            let mut ev = Loop::new().unwrap();
            let writable = Interest {
                readable: false,
                writable: true,
            };
            ev.register(&sender.source(), 1, writable).unwrap();
            let mut events = Vec::new();
            let mut sent = 0u64;
            while sent < TOTAL {
                ev.wait(&mut events, Some(Duration::from_secs(30))).unwrap();
                assert_eq!(events.len(), 1, "the sender stalled at byte {sent}");
                loop {
                    let at = (sent % pattern.len() as u64) as usize;
                    let count = (pattern.len() - at).min((TOTAL - sent) as usize);
                    if count == 0 {
                        break;
                    }
                    match sender.write(&pattern[at..at + count]) {
                        Ok(count) => sent += count as u64,
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                        Err(error) => panic!("write failed at byte {sent}: {error}"),
                    }
                }
            }
            // The graceful close: the end follows the last byte.
            sender.shutdown_write().unwrap();
            let mut byte = [0u8; 1];
            loop {
                match sender.read(&mut byte) {
                    Ok(0) => break,
                    Ok(_) => panic!("the receiver sent nothing"),
                    Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                        ev.modify(1, Interest::READABLE).unwrap();
                        ev.wait(&mut events, Some(Duration::from_secs(30))).unwrap();
                    }
                    Err(error) => panic!("read failed: {error}"),
                }
            }
        });
        let pattern = pattern();
        receiver
            .set_read_timeout(Some(Duration::from_secs(30)))
            .unwrap();
        let mut buffer = vec![0u8; 100_000];
        let mut received = 0u64;
        loop {
            match receiver.read(&mut buffer).unwrap() {
                0 => break,
                count => {
                    check(&pattern, received, &buffer[..count]);
                    received += count as u64;
                }
            }
        }
        assert_eq!(received, TOTAL);
        writer.join().unwrap();
    }

    #[test]
    fn shutting_down_writes_delivers_everything_and_then_the_end() {
        // More than the pipe holds and less than the mark: the stream takes
        // it all, and most of it is still on its way when writes are shut
        // down.
        let total = (PIPE_BUFFER * 5) as u64;
        assert!((total as usize) < WRITE_MARK);
        let pattern = pattern();
        let (mut sender, mut receiver) = Stream::pair().unwrap();
        // The peer has written something this side never reads. A socket
        // closed like that is reset, and loses what it had sent.
        receiver.write_all(b"never read").unwrap();

        sender.set_nonblocking().unwrap();
        assert_eq!(
            sender.write(&pattern[..total as usize]).unwrap(),
            total as usize
        );
        sender.shutdown_write().unwrap();
        assert_eq!(
            sender.write(b"x").unwrap_err().kind(),
            io::ErrorKind::BrokenPipe
        );
        // This side's end comes only once the last byte has gone out.
        let mut byte = [0u8; 16];
        let mut unread = Vec::new();
        loop {
            match sender.read(&mut byte) {
                Ok(0) => panic!("the stream closed with output unsent"),
                Ok(count) => unread.extend_from_slice(&byte[..count]),
                Err(error) => {
                    assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
                    break;
                }
            }
        }

        // The peer reads slowly, and gets every byte before the end.
        let reader = thread::spawn(move || {
            let pattern = pattern;
            let mut buffer = vec![0u8; 10_000];
            let mut received = 0u64;
            loop {
                thread::sleep(Duration::from_millis(1));
                match receiver.read(&mut buffer).unwrap() {
                    0 => break,
                    count => {
                        check(&pattern, received, &buffer[..count]);
                        received += count as u64;
                    }
                }
            }
            // After the end, the peer's own writes fail.
            let error = receiver.write(b"x").unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::BrokenPipe, "{error}");
            received
        });
        let start = Instant::now();
        loop {
            match sender.read(&mut byte) {
                Ok(0) => break,
                Ok(count) => unread.extend_from_slice(&byte[..count]),
                Err(error) => {
                    assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
                    assert!(start.elapsed() < Duration::from_secs(30));
                    wait(&[(&sender, Interest::READABLE)], Duration::from_secs(1));
                }
            }
        }
        assert_eq!(reader.join().unwrap(), total);
        // What the peer had sent before was readable until the handle
        // closed; this test read it or not, but never an error.
        assert!(b"never read".starts_with(&unread));
    }

    #[test]
    fn dropping_a_stream_with_unread_input_keeps_what_it_sent() {
        let (mut a, mut b) = Stream::pair().unwrap();
        b.write_all(b"never read").unwrap();
        a.write_all(b"the last message").unwrap();
        drop(a);
        let mut got = Vec::new();
        b.read_to_end(&mut got).unwrap();
        assert_eq!(got, b"the last message");
        assert_eq!(b.write(b"x").unwrap_err().kind(), io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn dropping_a_stream_cancels_what_the_pipe_had_not_taken() {
        let (a, mut b) = Stream::pair().unwrap();
        a.set_nonblocking().unwrap();
        let mut a = a;
        let big = vec![1u8; PIPE_BUFFER * 4];
        assert_eq!(a.write(&big).unwrap(), big.len());
        drop(a);
        // The write was in flight: the peer gets the end, and at most what
        // it could have read before, never part of a message out of order.
        let mut got = Vec::new();
        b.read_to_end(&mut got).unwrap();
        assert!(got.len() < big.len());
        assert!(got.iter().all(|&byte| byte == 1));
    }

    #[test]
    fn blocking_reads_wait_and_time_out() {
        let (mut a, mut b) = Stream::pair().unwrap();
        a.set_read_timeout(Some(Duration::from_millis(50))).unwrap();
        let start = Instant::now();
        let mut buffer = [0u8; 8];
        let error = a.read(&mut buffer).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::TimedOut);
        assert!(start.elapsed() >= Duration::from_millis(45));
        assert!(!a.input_waiting());

        // The read that timed out is still in flight, and gets what comes.
        b.write_all(b"late").unwrap();
        assert!(a.input_waiting());
        assert_eq!(a.read(&mut buffer).unwrap(), 4);
        assert_eq!(&buffer[..4], b"late");
        assert!(!a.input_waiting());

        a.set_read_timeout(None).unwrap();
        let writer = thread::spawn(move || {
            thread::sleep(Duration::from_millis(50));
            b.write_all(b"later").unwrap();
            b
        });
        assert_eq!(a.read(&mut buffer).unwrap(), 5);
        let _b = writer.join().unwrap();
    }

    #[test]
    fn input_waiting_sees_bytes_in_the_pipe_and_in_the_buffer() {
        let (mut a, b) = Stream::pair().unwrap();
        b.set_nonblocking().unwrap();
        let mut b = b;
        assert!(!b.input_waiting());
        a.write_all(b"abc").unwrap();
        assert!(b.input_waiting(), "in the pipe, with no read issued");
        let mut byte = [0u8; 1];
        assert_eq!(b.read(&mut byte).unwrap(), 1);
        assert!(b.input_waiting(), "read from the pipe, not yet taken");
        assert_eq!(b.read(&mut byte).unwrap(), 1);
        assert_eq!(b.read(&mut byte).unwrap(), 1);
        assert!(!b.input_waiting());

        // Registered, the read in flight takes the bytes at once.
        let mut ev = Loop::new().unwrap();
        ev.register(&b.source(), 1, Interest::NONE).unwrap();
        assert!(!b.input_waiting());
        a.write_all(b"de").unwrap();
        assert!(b.input_waiting(), "taken by the read in flight");
        assert_eq!(b.read(&mut byte).unwrap(), 1);
        assert_eq!(b.read(&mut byte).unwrap(), 1);
        assert!(!b.input_waiting());
        assert_eq!(
            b.read(&mut byte).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
    }

    #[test]
    fn a_registered_stream_closes_gracefully_between_the_loops_waits() {
        // What a connection's blocking flush and close do: the loop isn't
        // waiting, so the stream's own waits take the packets.
        let mut ev = Loop::new().unwrap();
        let (a, mut b) = Stream::pair().unwrap();
        a.set_nonblocking().unwrap();
        let mut a = a;
        ev.register(&a.source(), 1, Interest::READABLE).unwrap();
        ev.waker().wake().unwrap();

        let big = vec![9u8; PIPE_BUFFER * 4];
        assert_eq!(a.write(&big).unwrap(), big.len());
        a.shutdown_write().unwrap();
        let reader = thread::spawn(move || {
            let mut got = Vec::new();
            b.read_to_end(&mut got).unwrap();
            got.len()
        });
        let start = Instant::now();
        let mut byte = [0u8; 1];
        loop {
            match a.read(&mut byte) {
                Ok(0) => break,
                Ok(_) => panic!("the peer sent nothing"),
                Err(error) => {
                    assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
                    assert!(start.elapsed() < Duration::from_secs(30));
                    wait(&[(&a, Interest::READABLE)], Duration::from_millis(100));
                }
            }
        }
        assert_eq!(reader.join().unwrap(), big.len());

        // The wake those waits took with the packets isn't lost.
        let mut events = Vec::new();
        assert!(ev.wait(&mut events, Some(Duration::ZERO)).unwrap());
        assert_eq!(events.len(), 1);
        assert!(events[0].readable && events[0].closed);
    }

    #[test]
    fn a_wait_takes_streams_with_and_without_a_loop() {
        let mut ev = Loop::new().unwrap();
        let (mut with, with_peer) = Stream::pair().unwrap();
        let (mut without, without_peer) = Stream::pair().unwrap();
        with.set_nonblocking().unwrap();
        without.set_nonblocking().unwrap();
        ev.register(&with.source(), 1, Interest::READABLE).unwrap();

        let readable = Interest::READABLE;
        let start = Instant::now();
        wait(
            &[(&with, readable), (&without, readable)],
            Duration::from_millis(60),
        );
        assert!(start.elapsed() >= Duration::from_millis(50), "nothing came");

        // One reports through a packet and the other through an event.
        let mut byte = [0u8; 1];
        let mut peers = Vec::new();
        for (mut peer, first) in [(with_peer, true), (without_peer, false)] {
            let writer = thread::spawn(move || {
                thread::sleep(Duration::from_millis(30));
                peer.write_all(b"x").unwrap();
                peer
            });
            let start = Instant::now();
            wait(
                &[(&with, readable), (&without, readable)],
                Duration::from_secs(10),
            );
            assert!(start.elapsed() < Duration::from_secs(5));
            let stream = if first { &mut with } else { &mut without };
            assert_eq!(stream.read(&mut byte).unwrap(), 1);
            // Kept open: a closed peer would end the next wait at once.
            peers.push(writer.join().unwrap());
        }
    }

    #[test]
    fn a_read_left_by_a_thread_that_exits_still_gets_its_input() {
        let (a, mut b) = Stream::pair().unwrap();
        // Windows cancels a thread's pending I/O when the thread exits.
        let mut a = thread::spawn(move || {
            let mut a = a;
            a.set_nonblocking().unwrap();
            let mut byte = [0u8; 1];
            assert_eq!(
                a.read(&mut byte).unwrap_err().kind(),
                io::ErrorKind::WouldBlock
            );
            a
        })
        .join()
        .unwrap();
        b.write_all(b"x").unwrap();
        wait(&[(&a, Interest::READABLE)], Duration::from_secs(10));
        let mut byte = [0u8; 4];
        assert_eq!(a.read(&mut byte).unwrap(), 1);
        assert_eq!(byte[0], b'x');
    }
}
