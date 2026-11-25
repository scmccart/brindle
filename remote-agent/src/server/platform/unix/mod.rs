//! The Linux platform layer: Unix sockets, `flock`, the double fork, PTYs,
//! programs on plain pipes (`piped`) and `poll(2)`.

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
pub mod pty;
pub mod sys;
