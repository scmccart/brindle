//! Starting a server process detached and telling its starter it is ready
//! (the platform's part), and the log lines' timestamp.

use std::time::{SystemTime, UNIX_EPOCH};

/// The trampoline a role is started through on Windows (`launch`).
#[cfg(windows)]
pub use super::platform::daemon::launch;
pub use super::platform::daemon::{
    ReadyPipe, daemonize, open_log, ready_from_args, role_args, start_role,
};

/// Seconds since the epoch, for log lines.
pub fn timestamp() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}.{:03}", now.as_secs(), now.subsec_millis())
}
