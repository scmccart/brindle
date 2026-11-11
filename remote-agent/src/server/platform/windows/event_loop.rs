//! The event loop: to be an IOCP loop whose sources own their handle and
//! their read and write buffers.

use std::io;
use std::time::Duration;

use super::unsupported;
use crate::server::event_loop::{Event, EventLoop, Interest, Token};

/// The loop the core and the holders run on.
#[allow(dead_code, reason = "a stub, never constructed")]
pub struct Loop(());

impl Loop {
    pub fn new() -> io::Result<Self> {
        Err(unsupported())
    }
}

/// Something a loop waits on.
pub struct Source;

/// Wakes a loop's wait from another thread.
#[derive(Debug, Clone)]
pub struct Waker;

impl Waker {
    #[allow(dead_code, reason = "for worker threads, as on Linux")]
    pub fn wake(&self) -> io::Result<()> {
        Err(unsupported())
    }

    /// Wakes the loop whenever a child of this process exits.
    pub fn wake_on_child_exit(&self) -> io::Result<()> {
        Err(unsupported())
    }
}

impl EventLoop for Loop {
    fn register(&mut self, _: &Source, _: Token, _: Interest) -> io::Result<()> {
        Err(unsupported())
    }

    fn modify(&mut self, _: Token, _: Interest) -> io::Result<()> {
        Err(unsupported())
    }

    fn deregister(&mut self, _: Token) -> io::Result<()> {
        Err(unsupported())
    }

    fn wait(&mut self, _: &mut Vec<Event>, _: Option<Duration>) -> io::Result<bool> {
        Err(unsupported())
    }

    fn waker(&self) -> Waker {
        Waker
    }
}
