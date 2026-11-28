//! What the core and the holders need from the operating system, behind
//! one set of modules per platform with the same items:
//!
//! - `event_loop`: the loop (`Loop`), what it waits on (`Source`) and its
//!   `Waker`;
//! - `conn`: the `Stream` under a framed connection;
//! - `listen`: a `Listener` that accepts with the peer check, and
//!   connecting to one;
//! - `lock`: the single-instance and holder locks;
//! - `daemon`: starting a role detached, and its ready signal;
//! - `pty`: a session's PTY and program (spawn, I/O, resize, hang up, exit),
//!   and for the core its foreground program (none on Windows);
//! - `{unix,windows}/piped.rs`: a session's program on plain pipes
//!   (`Piped`, its three streams, `PipeCommand` and `spawn_piped`,
//!   re-exported at this module's top, since `piped` here names the
//!   neutral module below), sharing `pty`'s `Program` and `Group` (on
//!   Windows a pipe session's group is its job);
//! - `dir`: finding and securing the server directory;
//! - `sys`: the running binary and random numbers;
//! - `store`: the attachment store's files (OpenSpec change
//!   `agents-prompt-context`): private directories, texts written once
//!   and read-only, reads, removals and listings ([`Listed`]), and
//!   `WINDOWS`, whether this host's paths are Windows paths.
//!
//! Both platforms implement every module. On Windows, `listen`'s paths are
//! pipe names.
//!
//! What those modules decide sits beside them in modules of no platform,
//! each pure and tested on every platform; the platform modules gather,
//! call them and apply what they say:
//!
//! - `ready_arg`: a role's ready-pipe argument, `--ready-fd N` or
//!   `--ready-handle N` (both `daemon`s);
//! - `ensure_dir`: what making sure of the Windows server directory does
//!   with what it found (`dir`);
//! - `pipe_state`: the Windows pipes' read and write buffers and their
//!   readiness (`pipe`);
//! - `wait`: a wait's timeout in whole milliseconds, never `INFINITE`
//!   (the Windows `event_loop`, `pipe` and `listen`);
//! - `piped`: a pipe session's Windows creation flags, handle list and job
//!   limit, its program, command line and environment (through
//!   `conpty::cmdline`) with the batch-file hint, and its hang-up steps
//!   (both `piped`s).

#[cfg_attr(
    windows,
    allow(dead_code, reason = "parse_fd is the Linux daemon's")
)]
#[cfg_attr(
    unix,
    allow(dead_code, reason = "parse_handle is the Windows daemon's")
)]
mod ready_arg;

#[cfg_attr(
    not(windows),
    allow(dead_code, reason = "only the Windows server directory uses it")
)]
mod ensure_dir;

#[cfg_attr(
    not(windows),
    allow(dead_code, reason = "only the Windows pipes keep it")
)]
mod pipe_state;

#[cfg_attr(
    not(windows),
    allow(dead_code, reason = "only the Windows waits take milliseconds")
)]
mod wait;

#[cfg_attr(
    all(not(windows), not(test)),
    allow(
        dead_code,
        reason = "the creation flags, handle list and launch are the Windows spawn's"
    )
)]
pub mod piped;

/// A file or directory a store listing found (`store::list`), with its
/// size and when it was last written, in seconds since the epoch (`None`
/// when the system doesn't say). Neutral, so the core's store plan reads
/// it on every platform.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub name: String,
    pub dir: bool,
    pub bytes: u64,
    pub modified: Option<u64>,
}

#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;
