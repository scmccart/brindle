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
mod piped;
pub use piped::{PipeCommand, Piped, agent_directory, spawn_piped};
pub mod pty;
pub mod sys;
