//! What the core and the holders need from the operating system, behind
//! one set of modules per platform with the same items:
//!
//! - `event_loop`: the loop (`Loop`), what it waits on (`Source`) and its
//!   `Waker`;
//! - `conn`: the `Stream` under a framed connection;
//! - `listen`: a `Listener` that accepts with the peer check, and
//!   connecting to one;
//! - `lock`: the single-instance and holder locks;
//! - `daemon`: starting a role detached, and its ready signal;
//! - `pty`: a session's PTY and program (spawn, I/O, resize, hang up, exit);
//! - `dir`: finding and securing the server directory;
//! - `sys`: the running binary and random numbers.
//!
//! Linux has the implementations. Windows has `event_loop` and `conn`, and
//! stubs for the rest until the `terminal-server-windows` change fills them
//! in.

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;
