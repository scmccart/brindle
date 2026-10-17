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
//!
//! On Unix, [`MarkedPty`] wraps alacritty's `Pty` and reads a clone of its
//! master. On Windows it wraps Brindle's own pseudoconsole
//! ([`super::conpty`]), whose output pipe it reads directly.

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

pub struct ParsingReader<R> {
    /// The PTY, scanned for OSC 9/777 notifications as it is read.
    file: ScanReader<R>,
    term: Arc<FairMutex<Term<Listener>>>,
    parser: Arc<Mutex<Parser>>,
    marks: Arc<Mutex<Marks>>,
    listener: Listener,
    buf: Box<[u8]>,
}

impl<R> ParsingReader<R> {
    fn new(
        file: R,
        term: Arc<FairMutex<Term<Listener>>>,
        parser: Arc<Mutex<Parser>>,
        marks: Arc<Mutex<Marks>>,
        listener: Listener,
        tx: UnboundedSender<ChannelEvent>,
    ) -> Self {
        let file = ScanReader::new(file, tx);
        Self { file, term, parser, marks, listener, buf: vec![0; READ_SIZE].into_boxed_slice() }
    }
}

impl<R: Read> Read for ParsingReader<R> {
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
#[cfg(unix)]
pub struct MarkedPty {
    pty: tty::Pty,
    reader: ParsingReader<std::fs::File>,
}

#[cfg(unix)]
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
        let file = pty.file().try_clone()?;
        Ok(Self { reader: ParsingReader::new(file, term, parser, marks, listener, tx), pty })
    }
}

#[cfg(unix)]
impl EventedReadWrite for MarkedPty {
    type Reader = ParsingReader<std::fs::File>;
    type Writer = std::fs::File;

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

    fn reader(&mut self) -> &mut Self::Reader {
        &mut self.reader
    }

    fn writer(&mut self) -> &mut std::fs::File {
        self.pty.writer()
    }
}

#[cfg(unix)]
impl EventedPty for MarkedPty {
    fn next_child_event(&mut self) -> Option<ChildEvent> {
        self.pty.next_child_event()
    }
}

#[cfg(unix)]
impl OnResize for MarkedPty {
    fn on_resize(&mut self, size: WindowSize) {
        self.pty.on_resize(size)
    }
}

/// Brindle's pseudoconsole with a [`ParsingReader`] on its output.
#[cfg(windows)]
pub struct MarkedPty {
    // First, so it is released and closed before the pipes drop.
    console: super::conpty::Pseudoconsole,
    reader: ParsingReader<super::conpty::PipeReader>,
    writer: super::conpty::PipeWriter,
    child: super::conpty::ChildWatcher,
}

#[cfg(windows)]
impl MarkedPty {
    pub fn new(
        spawned: super::conpty::Spawned,
        term: Arc<FairMutex<Term<Listener>>>,
        parser: Arc<Mutex<Parser>>,
        marks: Arc<Mutex<Marks>>,
        listener: Listener,
        tx: UnboundedSender<ChannelEvent>,
    ) -> Self {
        let super::conpty::Spawned { console, reader, writer, child, .. } = spawned;
        Self { console, reader: ParsingReader::new(reader, term, parser, marks, listener, tx), writer, child }
    }

    /// The event loop wakes on completion packets, so every registration
    /// just records what to post.
    fn watch(&mut self, poll: &Arc<Poller>, mut interest: Event) {
        interest.key = tty::PTY_READ_WRITE_TOKEN;
        self.reader.file.get_ref().register(poll, interest);
        self.writer.register(poll, interest);
        self.child.register(poll, tty::PTY_CHILD_EVENT_TOKEN);
    }
}

#[cfg(windows)]
impl EventedReadWrite for MarkedPty {
    type Reader = ParsingReader<super::conpty::PipeReader>;
    type Writer = super::conpty::PipeWriter;

    unsafe fn register(&mut self, poll: &Arc<Poller>, interest: Event, _: PollMode) -> io::Result<()> {
        self.watch(poll, interest);
        Ok(())
    }

    fn reregister(&mut self, poll: &Arc<Poller>, interest: Event, _: PollMode) -> io::Result<()> {
        self.watch(poll, interest);
        Ok(())
    }

    fn deregister(&mut self, _: &Arc<Poller>) -> io::Result<()> {
        self.reader.file.get_ref().deregister();
        self.writer.deregister();
        self.child.deregister();
        Ok(())
    }

    fn reader(&mut self) -> &mut Self::Reader {
        &mut self.reader
    }

    fn writer(&mut self) -> &mut Self::Writer {
        &mut self.writer
    }
}

#[cfg(windows)]
impl EventedPty for MarkedPty {
    fn next_child_event(&mut self) -> Option<ChildEvent> {
        self.child.next_event()
    }
}

#[cfg(windows)]
impl OnResize for MarkedPty {
    fn on_resize(&mut self, size: WindowSize) {
        self.console.resize(size)
    }
}
