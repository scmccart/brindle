//! A reader that scans PTY output for notifications.
//!
//! The shell-integration PTY reader (`pty.rs`) reads the PTY through a
//! [`ScanReader`], so OSC 9/777 are found on alacritty's I/O thread before
//! the bytes are parsed.

use std::io::{self, Read};

use futures::channel::mpsc::UnboundedSender;

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
