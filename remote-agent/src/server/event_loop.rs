//! The single-threaded event loop the core and the holders run on.
//!
//! [`EventLoop`] is the small platform seam: register, modify and deregister
//! a [`Source`], wait with a timeout, and be woken from another thread or
//! when a child exits through a [`Waker`]. The platform provides [`Loop`]:
//! on Linux `PollLoop`, over `poll(2)`, whose sources are file descriptors;
//! on Windows an IOCP loop whose sources own their handle and buffers.
//!
//! Handlers never touch what a source wraps. They read and write through
//! the source's owner (a `Conn`, a `Listener`, a `Pty`, a `ReadyPipe`), so
//! a platform is free to do the I/O itself and hand over buffered bytes.
//!
//! Readiness is level-triggered: a source that is still readable is reported
//! again on the next wait, so a handler may stop after a fair share of work.

use std::io;
use std::time::Duration;

pub use super::platform::event_loop::{Loop, Source, Waker};

/// Names a registered source in the events a wait returns.
pub type Token = usize;

/// What a source is waited for. With neither, it stays registered but is
/// never reported (not even for hang-ups), which is how a reader pauses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Interest {
    pub readable: bool,
    pub writable: bool,
}

impl Interest {
    pub const NONE: Self = Self {
        readable: false,
        writable: false,
    };
    pub const READABLE: Self = Self {
        readable: true,
        writable: false,
    };
}

/// One ready source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Event {
    pub token: Token,
    pub readable: bool,
    pub writable: bool,
    /// The peer hung up or the source failed. A read then returns EOF or
    /// the error, so handlers treat it as readable.
    pub closed: bool,
}

pub trait EventLoop {
    /// Starts reporting `source` as `token`. Tokens must be unique. The
    /// source's owner stays the one to read, write and close it.
    fn register(&mut self, source: &Source, token: Token, interest: Interest) -> io::Result<()>;
    fn modify(&mut self, token: Token, interest: Interest) -> io::Result<()>;
    /// Stops reporting `token`. The caller still owns and closes the source.
    fn deregister(&mut self, token: Token) -> io::Result<()>;
    /// Waits until a source is ready, the loop is woken, or `timeout`
    /// passes (`None` waits forever). Fills `events` (cleared first) and
    /// returns whether a [`Waker`] fired.
    fn wait(&mut self, events: &mut Vec<Event>, timeout: Option<Duration>) -> io::Result<bool>;
    fn waker(&self) -> Waker;
}
