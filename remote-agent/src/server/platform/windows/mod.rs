//! The Windows platform layer. Until the `terminal-server-windows` change
//! fills it in, these are stubs with the Linux modules' items: nothing can
//! be created, and what would create it fails with [`unsupported`], so
//! `serve`, `hold` and the management commands end with that message.
//!
//! The types that stand for a resource are never constructed (hence their
//! `dead_code` allowances), so their methods are never called.

use std::io;

pub mod conn;
pub mod daemon;
pub mod dir;
pub mod event_loop;
pub mod listen;
pub mod lock;
pub mod pty;
pub mod sys;

fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "the terminal server is not implemented on Windows yet",
    )
}
