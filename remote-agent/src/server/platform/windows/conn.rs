//! The stream under a framed connection: to be a named pipe.

use std::io::{self, Read, Write};
use std::time::Duration;

use super::event_loop::Source;
use super::unsupported;
use crate::server::event_loop::Interest;

/// A connected stream. It blocks until [`Stream::set_nonblocking`], after
/// which reads and writes that can't proceed fail with `WouldBlock`.
#[allow(dead_code, reason = "a stub, never constructed")]
pub struct Stream(());

impl Stream {
    pub fn set_nonblocking(&self) -> io::Result<()> {
        Err(unsupported())
    }

    /// How long a blocking read waits (`None` waits forever).
    pub fn set_read_timeout(&self, _limit: Option<Duration>) -> io::Result<()> {
        Err(unsupported())
    }

    /// What an event loop waits on for this stream.
    pub fn source(&self) -> Source {
        Source
    }

    /// Whether bytes are waiting to be read.
    pub fn input_waiting(&self) -> bool {
        false
    }

    /// Ends this side's output; the peer reads EOF after what was sent.
    pub fn shutdown_write(&self) -> io::Result<()> {
        Err(unsupported())
    }
}

impl Read for Stream {
    fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
        Err(unsupported())
    }
}

impl Write for Stream {
    fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
        Err(unsupported())
    }

    fn flush(&mut self) -> io::Result<()> {
        Err(unsupported())
    }
}

/// Waits up to `timeout` until one of `streams` is ready for what it is
/// waited for, or has hung up.
pub fn wait(_streams: &[(&Stream, Interest)], _timeout: Duration) {}
