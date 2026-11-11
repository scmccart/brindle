//! Starting a server process detached: to be `--ready-handle N`, a
//! detached process that breaks away from its starter's job, and the one
//! ready byte.

use std::ffi::{OsStr, OsString};
use std::fs::File;
use std::io;
use std::path::Path;
use std::process::Child;

use super::event_loop::Source;
use super::unsupported;

/// The pipe to write the ready byte to.
#[allow(dead_code, reason = "a stub, never constructed")]
pub struct Ready(());

impl Ready {
    /// Tells the starter the server is listening, and closes the pipe.
    pub fn signal(self) -> io::Result<()> {
        Err(unsupported())
    }
}

/// The ready pipe a role was started with, from its arguments, or nothing
/// for a role that stays in the foreground.
pub fn ready_from_args(_role: &str, args: &[String]) -> Result<Option<Ready>, String> {
    if args.is_empty() {
        Ok(None)
    } else {
        Err(unsupported().to_string())
    }
}

/// The arguments that start `role` detached with [`start_role`].
pub fn role_args(role: &str) -> Vec<String> {
    vec![role.into()]
}

/// The read end of a started role's ready pipe.
#[allow(dead_code, reason = "a stub, never constructed")]
pub struct ReadyPipe(());

impl ReadyPipe {
    /// What an event loop waits on: readable once the role is listening or
    /// has failed.
    pub fn source(&self) -> Source {
        Source
    }

    /// Whether the role is listening (`true`) or ended before it was
    /// (`false`); `None` while neither is known.
    pub fn read(&self) -> Option<bool> {
        Some(false)
    }
}

/// A role started by [`start_role`].
#[allow(dead_code, reason = "a stub, never constructed")]
pub struct Started {
    /// The starter, or the role itself.
    pub child: Child,
    pub ready: ReadyPipe,
    /// What was run, for the log.
    pub program: OsString,
}

/// Starts a role of this process's own binary, detached, with `env` added
/// to this process's environment. `unscoped` is told why when the role
/// can't be started apart from its starter.
pub fn start_role(
    _role: &str,
    _args: &[String],
    _env: &[(&str, &OsStr)],
    _unscoped: impl FnOnce(&str),
) -> io::Result<Started> {
    Err(unsupported())
}

/// Opens (appending) a server log.
pub fn open_log(_path: &Path) -> io::Result<File> {
    Err(unsupported())
}

/// Detaches this process, with its output going to `log`.
pub fn daemonize(_log: &File) -> io::Result<()> {
    Err(unsupported())
}
