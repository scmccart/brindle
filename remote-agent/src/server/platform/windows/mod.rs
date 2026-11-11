//! The Windows platform layer. The event loop (`event_loop`, with its pipe
//! sources in `pipe`) and the stream (`conn`) are in place. Until the
//! `terminal-server-windows` change fills in the rest, the other modules
//! are stubs with the Linux modules' items: nothing can be created, and
//! what would create it fails with [`unsupported`], so `serve`, `hold` and
//! the management commands end with that message.
//!
//! The stubs' types that stand for a resource are never constructed (hence
//! their `dead_code` allowances), so their methods are never called.

use std::io;

pub mod conn;
pub mod daemon;
pub mod dir;
pub mod event_loop;
pub mod listen;
pub mod lock;
mod pipe;
pub mod pty;
pub mod sys;

fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "the terminal server is not implemented on Windows yet",
    )
}
