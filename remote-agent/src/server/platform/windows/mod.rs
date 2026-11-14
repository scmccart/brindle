//! The Windows platform layer: the event loop (`event_loop`, with its pipe
//! sources in `pipe`), the stream (`conn`), the pipe listener (`listen`),
//! the locks (`lock`), the directory (`dir`), starting roles (`daemon`) and
//! the sessions' pseudoconsoles (`pty`). The core and the holders listen at
//! their pipe names (`brindle_remote_protocol::endpoint`).

pub mod conn;
pub mod daemon;
pub mod dir;
pub mod event_loop;
pub mod listen;
pub mod lock;
mod pipe;
pub mod pty;
pub mod sys;
