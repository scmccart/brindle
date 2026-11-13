//! The Windows platform layer: the event loop (`event_loop`, with its pipe
//! sources in `pipe`), the stream (`conn`), the pipe listener (`listen`),
//! the locks (`lock`), the directory (`dir`), starting roles (`daemon`) and
//! the sessions' pseudoconsoles (`pty`). The holder listens at its pipe name
//! (`brindle_remote_protocol::endpoint`). The core still passes a Unix
//! socket path to the listener, so `serve` doesn't run on Windows until the
//! `terminal-server-windows` change moves it to its pipe name.

pub mod conn;
pub mod daemon;
pub mod dir;
pub mod event_loop;
pub mod listen;
pub mod lock;
mod pipe;
pub mod pty;
pub mod sys;
