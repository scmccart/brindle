//! One open link to a core, as a [`super::Connection`]'s two threads use it:
//! the reader reads frames from [`Link::reader`], and the writer writes them
//! to [`Link::wire`]. A local link is the core's socket or pipe; a relay's
//! is the pipes of the process that runs `brindle-remote relay` at the
//! endpoint (`relay.rs`), or in tests, in-memory pipes.

use std::io::{self, Read};
use std::sync::Arc;

use super::Stream;
use crate::remote::protocol::Frame;

/// The writing half of a link.
pub trait Wire: Send + Sync {
    /// Writes one whole frame.
    fn send(&self, frame: &Frame) -> io::Result<()>;
    /// Ends the link: the reader's read returns (the end of the stream, or
    /// an error) and later sends fail.
    fn end(&self);
    /// Why the link ended, when the far side knows more than the end of the
    /// stream says (a relay process's exit status and what it wrote to
    /// stderr); empty otherwise.
    fn why(&self) -> String {
        String::new()
    }
}

/// An open link.
pub struct Link {
    pub reader: Box<dyn Read + Send>,
    pub wire: Arc<dyn Wire>,
}

impl Link {
    /// The local core's socket or pipe.
    pub fn local(stream: Stream) -> io::Result<Self> {
        let wire = Arc::new(stream.try_clone()?);
        Ok(Self { reader: Box::new(stream), wire })
    }
}

impl Wire for Stream {
    fn send(&self, frame: &Frame) -> io::Result<()> {
        frame.write_to(&mut &*self)
    }

    fn end(&self) {
        self.shutdown(std::net::Shutdown::Both).ok();
    }
}
