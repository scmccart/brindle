//! PTY sessions shared by the stdio agent (`--server`) and the terminal
//! server's holder: what to run on a new PTY and the shutdown timings here,
//! and the Linux PTY itself (spawning, resizing, signalling, `/proc`
//! queries) in [`unix`].

use std::collections::BTreeMap;
use std::time::Duration;

#[cfg(target_os = "linux")]
mod unix;
#[cfg(target_os = "linux")]
pub use unix::*;

/// How much PTY output is read at once.
pub const OUTPUT_CHUNK_SIZE: usize = 32 * 1024;
/// How long a hung-up session's process group has to exit after `SIGHUP`
/// before it gets `SIGKILL`.
pub const GRACEFUL_SHUTDOWN: Duration = Duration::from_secs(1);
/// After the program exits, output is still read until it has been quiet
/// for this long...
pub const OUTPUT_DRAIN_IDLE: Duration = Duration::from_millis(250);
/// ...or for at most this long.
pub const OUTPUT_DRAIN_LIMIT: Duration = Duration::from_secs(2);

/// What to run on a new PTY.
pub struct PtyCommand<'a> {
    /// `None` runs the user's login shell.
    pub command: Option<&'a str>,
    pub args: &'a [String],
    pub cwd: Option<&'a str>,
    pub env: &'a BTreeMap<String, String>,
    pub cols: u16,
    pub rows: u16,
    #[cfg_attr(windows, allow(dead_code, reason = "a console has no pixel size"))]
    pub cell_width: u16,
    #[cfg_attr(windows, allow(dead_code, reason = "a console has no pixel size"))]
    pub cell_height: u16,
    /// `env` is the program's whole environment. Otherwise it is added to
    /// this process's own, with a default `TERM`.
    pub exact_env: bool,
}
