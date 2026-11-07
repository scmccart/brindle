//! The terminal server's processes: the holder (`hold`) and, later, the
//! core (`serve`), with the pieces both use: the server directory, the
//! `--ready-fd` start, the peer check, the build id, framed connections and
//! the event loop.

pub mod build_id;
pub mod conn;
pub mod daemon;
pub mod dir;
pub mod event_loop;
pub mod holder;
pub mod peer;
