//! A PTY whose reader parses output itself, so OSC 133 marks can be
//! recorded against the exact terminal state they belong to.
//!
//! alacritty's `EventLoop` reads through `EventedReadWrite::reader()` and
//! parses whatever it read with its own processor. [`ParsingReader`] instead
//! reads the PTY, runs the bytes through Brindle's [`Parser`] under the term
//! lock, and returns `Ok(0)`. In alacritty 0.26 `pty_read` takes `Ok(0)`
//! with nothing buffered as "no more for now" and goes back to its
//! level-triggered poll, so the loop keeps doing everything else (polling,
//! the write queue, resize, child exit, drain on exit) unchanged.

use std::fs::File;
use std::io::{self, Read};
use std::sync::Arc;

use alacritty_terminal::event::{Event as AlacEvent, EventListener, OnResize, WindowSize};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Term;
use alacritty_terminal::tty::{self, ChildEvent, EventedPty, EventedReadWrite};
use parking_lot::Mutex;
use polling::{Event, PollMode, Poller};

use futures::channel::mpsc::UnboundedSender;

use super::scan::ScanReader;
use super::{ChannelEvent, Listener};
use super::shell_integration::{Marks, Parser};

/// Bytes read from the PTY per call (alacritty locks for up to 64 KiB too).
const READ_SIZE: usize = 0x1_0000;

pub struct ParsingReader {
    /// The PTY, scanned for OSC 9/777 notifications as it is read.
    file: ScanReader<File>,
    term: Arc<FairMutex<Term<Listener>>>,
    parser: Arc<Mutex<Parser>>,
    marks: Arc<Mutex<Marks>>,
    listener: Listener,
    buf: Box<[u8]>,
}

impl Read for ParsingReader {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        let n = self.file.read(&mut self.buf)?;
        if n == 0 {
            return Ok(0);
        }
        {
            // Lock order: parser, then term, then marks. The event loop
            // holds the FairMutex lease on this thread, so take the term
            // unfairly (as alacritty's own `pty_read` does).
            let mut parser = self.parser.lock();
            let mut term = self.term.lock_unfair();
            parser.advance(&mut *term, &self.marks, &self.buf[..n]);
        }
        // Always wake the UI, even when the bytes are held by a
        // synchronized update: it schedules the update's timeout flush.
        self.listener.send_event(AlacEvent::Wakeup);
        Ok(0)
    }
}

/// alacritty's `Pty` with a [`ParsingReader`].
pub struct MarkedPty {
    pty: tty::Pty,
    reader: ParsingReader,
}

impl MarkedPty {
    pub fn new(
        pty: tty::Pty,
        term: Arc<FairMutex<Term<Listener>>>,
        parser: Arc<Mutex<Parser>>,
        marks: Arc<Mutex<Marks>>,
        listener: Listener,
        tx: UnboundedSender<ChannelEvent>,
    ) -> io::Result<Self> {
        // Same open file description, so the same non-blocking flag.
        let file = ScanReader::new(pty.file().try_clone()?, tx);
        let reader =
            ParsingReader { file, term, parser, marks, listener, buf: vec![0; READ_SIZE].into_boxed_slice() };
        Ok(Self { pty, reader })
    }
}

impl EventedReadWrite for MarkedPty {
    type Reader = ParsingReader;
    type Writer = File;

    unsafe fn register(&mut self, poll: &Arc<Poller>, interest: Event, mode: PollMode) -> io::Result<()> {
        // SAFETY: forwarded unchanged; the inner PTY outlives its registration
        // exactly as it would without the wrapper.
        unsafe { self.pty.register(poll, interest, mode) }
    }

    fn reregister(&mut self, poll: &Arc<Poller>, interest: Event, mode: PollMode) -> io::Result<()> {
        self.pty.reregister(poll, interest, mode)
    }

    fn deregister(&mut self, poll: &Arc<Poller>) -> io::Result<()> {
        self.pty.deregister(poll)
    }

    fn reader(&mut self) -> &mut ParsingReader {
        &mut self.reader
    }

    fn writer(&mut self) -> &mut File {
        self.pty.writer()
    }
}

impl EventedPty for MarkedPty {
    fn next_child_event(&mut self) -> Option<ChildEvent> {
        self.pty.next_child_event()
    }
}

impl OnResize for MarkedPty {
    fn on_resize(&mut self, size: WindowSize) {
        self.pty.on_resize(size)
    }
}
