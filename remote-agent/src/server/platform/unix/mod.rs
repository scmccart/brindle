//! The Linux platform layer: Unix sockets, `flock`, the double fork, PTYs
//! and `poll(2)`.

pub mod conn;
pub mod daemon;
pub mod dir;
pub mod event_loop;
pub mod listen;
pub mod lock;
pub mod pty;
pub mod sys;
