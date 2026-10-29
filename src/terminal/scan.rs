//! A reader that scans PTY output for the sequences Brindle handles beside
//! the parser (notifications, program status, prompt starts).
//!
//! The shell-integration PTY reader (`pty.rs`) reads the PTY through a
//! [`ScanReader`], so they are found on alacritty's I/O thread before the
//! bytes are parsed.

use std::io::{self, Read};

use futures::channel::mpsc::UnboundedSender;

use super::ChannelEvent;
use crate::notify::osc::Scanner;

/// Reads from `inner`, reporting what the scanner finds in what it read.
pub struct ScanReader<R> {
    inner: R,
    scanner: Scanner,
    tx: UnboundedSender<ChannelEvent>,
}

impl<R> ScanReader<R> {
    pub fn new(inner: R, tx: UnboundedSender<ChannelEvent>) -> Self {
        Self { inner, scanner: Scanner::new(), tx }
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn get_ref(&self) -> &R {
        &self.inner
    }
}

impl<R: Read> Read for ScanReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.inner.read(buf)?;
        for found in self.scanner.scan(&buf[..n]) {
            self.tx.unbounded_send(ChannelEvent::Osc(found)).ok();
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use futures::channel::mpsc::unbounded;

    use super::*;
    use crate::notify::osc::{Found, Source};

    /// Returns one chunk per read, like a PTY delivering output in pieces.
    struct Chunks(VecDeque<&'static [u8]>);

    impl Read for Chunks {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            let Some(chunk) = self.0.pop_front() else { return Ok(0) };
            buf[..chunk.len()].copy_from_slice(chunk);
            Ok(chunk.len())
        }
    }

    #[test]
    fn reports_a_notice_split_across_reads() {
        let chunks = Chunks(VecDeque::from([&b"out\x1b]777;noti"[..], &b"fy;T;B\x07more"[..]]));
        let (tx, mut rx) = unbounded();
        let mut reader = ScanReader::new(chunks, tx);
        let mut buf = [0u8; 64];

        let n = reader.read(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"out\x1b]777;noti", "bytes pass through unchanged");
        assert!(rx.try_next().is_err(), "nothing yet");

        let n = reader.read(&mut buf).unwrap();
        assert_eq!(&buf[..n], b"fy;T;B\x07more");
        match rx.try_next() {
            Ok(Some(ChannelEvent::Osc(Found::Notice(notice)))) => {
                assert_eq!(notice.source, Source::Osc777);
                assert_eq!((notice.title.as_str(), notice.body.as_str()), ("T", "B"));
            }
            _ => panic!("expected a notice"),
        }
    }
}
