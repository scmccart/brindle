//! The Windows platform layer: the event loop (`event_loop`, with its pipe
//! sources in `pipe`), the stream (`conn`), the pipe listener (`listen`),
//! the locks (`lock`), the directory (`dir`), starting roles (`daemon`),
//! the sessions' pseudoconsoles (`pty`) and programs on plain pipes
//! (`piped`). The core and the holders listen at their pipe names
//! (`brindle_remote_protocol::endpoint`).

pub mod conn;
pub mod daemon;
pub mod dir;
pub mod event_loop;
pub mod listen;
pub mod lock;
// Its items are the platform's (`platform::spawn_piped`); `platform::piped`
// is the neutral module of what they decide.
#[allow(dead_code, reason = "the pipe sessions that use it come with agents-sessions section 5")]
mod piped;
#[allow(unused_imports, reason = "the pipe sessions that use it come with agents-sessions section 5")]
pub use piped::{PipeCommand, Piped, spawn_piped};
mod pipe;
pub mod pty;
pub mod sys;
