//! A PTY wrapper whose reader scans output for notifications.
//!
//! alacritty's I/O thread reads the PTY straight into the grid, so the only
//! place to see the bytes is the reader it is given. [`ScanPty`] hands it a
//! [`ScanReader`] over a duplicate of the PTY's file descriptor (the same
//! open file, so non-blocking mode and readiness are shared) and delegates
//! everything else to the real [`Pty`].

use std::fs::File;
use std::io::{self, Read};
use std::sync::Arc;

use alacritty_terminal::event::{OnResize, WindowSize};
use alacritty_terminal::tty::{ChildEvent, EventedPty, EventedReadWrite, Pty};
use futures::channel::mpsc::UnboundedSender;
use polling::{Event, PollMode, Poller};

use super::ChannelEvent;
use crate::notify::osc::Scanner;

/// Reads from `inner`, reporting the notifications in what it read.
pub struct ScanReader<R> {
    inner: R,
    scanner: Scanner,
    tx: UnboundedSender<ChannelEvent>,
}

impl<R> ScanReader<R> {
    pub fn new(inner: R, tx: UnboundedSender<ChannelEvent>) -> Self {
        Self { inner, scanner: Scanner::new(), tx }
    }
}

impl<R: Read> Read for ScanReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        for notice in self.scanner.scan(&buf[..n]) {
            self.tx.unbounded_send(ChannelEvent::Notice(notice)).ok();
        }
        Ok(n)
    }
}

pub struct ScanPty {
    pty: Pty,
    reader: ScanReader<File>,
}

impl ScanPty {
    pub fn new(pty: Pty, tx: UnboundedSender<ChannelEvent>) -> io::Result<Self> {
        let file = pty.file().try_clone()?;
        Ok(Self { pty, reader: ScanReader::new(file, tx) })
    }

    pub fn pty(&self) -> &Pty {
        &self.pty
    }
}

impl EventedReadWrite for ScanPty {
    type Reader = ScanReader<File>;
    type Writer = File;

    unsafe fn register(&mut self, poll: &Arc<Poller>, interest: Event, mode: PollMode) -> io::Result<()> {
        // SAFETY: forwarded unchanged; the caller upholds `Pty::register`'s
        // contract (deregister before the PTY is dropped).
        unsafe { self.pty.register(poll, interest, mode) }
    }

    fn reregister(&mut self, poll: &Arc<Poller>, interest: Event, mode: PollMode) -> io::Result<()> {
        self.pty.reregister(poll, interest, mode)
    }

    fn deregister(&mut self, poll: &Arc<Poller>) -> io::Result<()> {
        self.pty.deregister(poll)
    }

    fn reader(&mut self) -> &mut Self::Reader {
        &mut self.reader
    }

    fn writer(&mut self) -> &mut File {
        self.pty.writer()
    }
}

impl EventedPty for ScanPty {
    fn next_child_event(&mut self) -> Option<ChildEvent> {
        self.pty.next_child_event()
    }
}

impl OnResize for ScanPty {
    fn on_resize(&mut self, size: WindowSize) {
        self.pty.on_resize(size)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write as _;
    use std::os::unix::net::UnixStream;

    use futures::channel::mpsc::unbounded;

    use super::*;
    use crate::notify::osc::Source;

    #[test]
    fn reports_a_notice_split_across_reads() {
        let (mut writer, reader) = UnixStream::pair().unwrap();
        let (tx, mut rx) = unbounded();
        let mut reader = ScanReader::new(reader, tx);
        let mut buf = [0u8; 64];

        writer.write_all(b"out\x1b]777;noti").unwrap();
        let n = reader.read(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"out\x1b]777;noti", "bytes pass through unchanged");
        assert!(rx.try_next().is_err(), "nothing yet");

        writer.write_all(b"fy;T;B\x07more").unwrap();
        let n = reader.read(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"fy;T;B\x07more");
        match rx.try_next() {
            Ok(Some(ChannelEvent::Notice(notice))) => {
                assert_eq!(notice.source, Source::Osc777);
                assert_eq!((notice.title.as_str(), notice.body.as_str()), ("T", "B"));
            }
            _ => panic!("expected a notice"),
        }
    }
}
