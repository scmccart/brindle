//! A non-blocking framed connection for the event loops: an input buffer
//! that yields whole frames, and an outgoing queue whose length is the
//! backpressure signal.

use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use brindle_remote_protocol::{FRAME_HEADER_LEN, Frame, MAX_FRAME_PAYLOAD_SIZE};

/// How much one `read_available` call reads before giving other sources a
/// turn; the loop is level-triggered, so the rest is read next time.
const READ_BUDGET: usize = 1024 * 1024;

pub struct Conn {
    stream: UnixStream,
    input: Vec<u8>,
    input_start: usize,
    output: Vec<u8>,
    output_start: usize,
}

impl Conn {
    /// Wraps a connected stream, making it non-blocking.
    pub fn new(stream: UnixStream) -> io::Result<Self> {
        stream.set_nonblocking(true)?;
        Ok(Self {
            stream,
            input: Vec::new(),
            input_start: 0,
            output: Vec::new(),
            output_start: 0,
        })
    }

    pub fn fd(&self) -> RawFd {
        self.stream.as_raw_fd()
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

    /// Sends as much of the queue as the socket takes without blocking.
    pub fn flush(&mut self) -> io::Result<()> {
        while self.output_start < self.output.len() {
            match self.stream.write(&self.output[self.output_start..]) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(written) => self.output_start += written,
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

    /// Flushes the whole queue, waiting for the peer for at most `limit`.
    pub fn flush_for(&mut self, limit: Duration) -> io::Result<()> {
        let deadline = Instant::now() + limit;
        loop {
            self.flush()?;
            if self.queued() == 0 {
                return Ok(());
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "the peer stopped reading",
                ));
            }
            let mut pollfd = libc::pollfd {
                fd: self.fd(),
                events: libc::POLLOUT,
                revents: 0,
            };
            let timeout = remaining.as_millis().clamp(1, 100) as libc::c_int;
            unsafe { libc::poll(&mut pollfd, 1, timeout) };
        }
    }
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
        let (a, b) = UnixStream::pair().unwrap();
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

    #[test]
    fn an_oversized_header_is_an_error() {
        let (mut a, b) = UnixStream::pair().unwrap();
        let mut receiver = Conn::new(b).unwrap();
        a.write_all(&[1, 0xff, 0xff, 0xff, 0xff]).unwrap();
        receiver.read_available().unwrap();
        assert_eq!(
            receiver.next_frame().unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }
}
