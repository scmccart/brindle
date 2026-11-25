//! The Linux platform layer: Unix sockets, `flock`, the double fork, PTYs,
//! programs on plain pipes (`piped`) and `poll(2)`.

pub mod conn;
pub mod daemon;
pub mod dir;
pub mod event_loop;
pub mod listen;
pub mod lock;
#[allow(dead_code, reason = "the pipe sessions that use it come with agents-sessions section 5")]
pub mod piped;
pub mod pty;
pub mod sys;
