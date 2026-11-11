//! The stream under a framed connection: a Unix stream socket.

use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::time::Duration;

use super::event_loop::Source;
use crate::server::event_loop::Interest;

/// A connected Unix stream socket. It blocks until
/// [`Stream::set_nonblocking`], after which reads and writes that can't
/// proceed fail with `WouldBlock`.
pub struct Stream(UnixStream);

impl Stream {
    pub(super) fn new(stream: UnixStream) -> Self {
        Self(stream)
    }

    pub fn set_nonblocking(&self) -> io::Result<()> {
        self.0.set_nonblocking(true)
    }

    /// How long a blocking read waits (`None` waits forever).
    pub fn set_read_timeout(&self, limit: Option<Duration>) -> io::Result<()> {
        self.0.set_read_timeout(limit)
    }

    /// What an event loop waits on for this stream.
    pub fn source(&self) -> Source {
        Source::of(&self.0)
    }

    /// Whether bytes are waiting in the socket.
    pub fn input_waiting(&self) -> bool {
        let mut waiting: libc::c_int = 0;
        // SAFETY: FIONREAD writes one int, and the fd is open while `self` is.
        let result = unsafe { libc::ioctl(self.0.as_raw_fd(), libc::FIONREAD, &mut waiting) };
        result == 0 && waiting > 0
    }

    /// Ends this side's output; the peer reads EOF after what was sent.
    pub fn shutdown_write(&self) -> io::Result<()> {
        self.0.shutdown(std::net::Shutdown::Write)
    }

    /// Two connected streams.
    #[cfg(test)]
    pub fn pair() -> io::Result<(Self, Self)> {
        let (a, b) = UnixStream::pair()?;
        Ok((Self(a), Self(b)))
    }
}

impl Read for Stream {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.0.read(buffer)
    }
}

impl Write for Stream {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.0.write(buffer)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

/// Waits up to `timeout` until one of `streams` is ready for what it is
/// waited for, or has hung up.
pub fn wait(streams: &[(&Stream, Interest)], timeout: Duration) {
    let mut pollfds: Vec<libc::pollfd> = streams
        .iter()
        .map(|(stream, interest)| {
            let mut events = 0;
            if interest.readable {
                events |= libc::POLLIN;
            }
            if interest.writable {
                events |= libc::POLLOUT;
            }
            libc::pollfd {
                fd: stream.0.as_raw_fd(),
                events,
                revents: 0,
            }
        })
        .collect();
    let timeout = timeout.as_millis().min(libc::c_int::MAX as u128) as libc::c_int;
    unsafe { libc::poll(pollfds.as_mut_ptr(), pollfds.len() as libc::nfds_t, timeout) };
}
