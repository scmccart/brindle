//! The terminal server's processes: the core (`serve`) and the holder
//! (`hold`), with the pieces both use: the server directory, the
//! `--ready-fd` start, the peer check, the build id, framed connections and
//! the event loop.

pub mod build_id;
pub mod check;
pub mod cli;
pub mod conn;
pub mod core;
pub mod daemon;
pub mod dir;
pub mod event_loop;
pub mod holder;
pub mod peer;
