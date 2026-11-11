//! The server pipes and the peer check: to be named pipe instances whose
//! clients must run as the server's user, at its integrity level.

use std::io;
use std::path::Path;

use super::conn::Stream;
use super::event_loop::Source;
use super::unsupported;

/// Who a server runs as, which its peers must match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Identity;

impl Identity {
    /// This process's own.
    pub fn own() -> Self {
        Self
    }
}

/// Fails if a server can't listen at `path`.
pub fn check_path(_path: &Path) -> io::Result<()> {
    Err(unsupported())
}

/// Connects to the server listening at `path`, blocking.
pub fn connect(_path: &Path) -> io::Result<Stream> {
    Err(unsupported())
}

/// A non-blocking listener.
#[allow(dead_code, reason = "a stub, never constructed")]
pub struct Listener(());

impl Listener {
    /// Listens at `path` for peers that run as `identity`.
    pub fn bind(_path: &Path, _identity: Identity) -> io::Result<Self> {
        Err(unsupported())
    }

    /// What an event loop waits on: readable when a connection is waiting.
    pub fn source(&self) -> Source {
        unreachable!("a stub, never constructed")
    }

    /// Accepts the next connection waiting; `Ok(None)` means none is. A
    /// refused peer comes back as a `PermissionDenied` error.
    pub fn accept(&self) -> io::Result<Option<Stream>> {
        Err(unsupported())
    }
}
