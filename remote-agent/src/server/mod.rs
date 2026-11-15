//! The terminal server's processes: the core (`serve`) and the holder
//! (`hold`), with the pieces both use: the server directory, the detached
//! start, the build id, framed connections and the event loop. What they
//! need from the operating system is in [`platform`]. On Linux, the relay
//! (`relay`) reaches the core for a client on another machine.

pub mod build_id;
pub mod check;
pub mod cli;
pub mod conn;
pub mod core;
pub mod daemon;
pub mod dir;
pub mod event_loop;
pub mod holder;
pub mod platform;
#[cfg(unix)]
pub mod relay;
