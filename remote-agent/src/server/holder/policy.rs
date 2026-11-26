//! The holder's policies, as pure functions of what its loop gathers
//! (`fcis R1`): the handlers in `holder.rs` read their state, call these,
//! and apply what comes back. `now` is always an argument.
//!
//! - **Spawns:** which `SPAWN`s and `SPAWN_PIPES` are refused, and with
//!   what text ([`spawn_refusal`], [`pipe_spawn_refusal`]).
//! - **Pipe sessions and older cores:** whether a core is shown a session
//!   ([`shown`]: not a pipe session, below H1 version 3).
//! - **Exits:** when a reaped program's session is finished, its output
//!   drained ([`drained`]).
//! - **Supervision:** when a holder with sessions and no core starts one,
//!   with backoff ([`CoreStart`]).

use std::fmt;
use std::time::{Duration, Instant};

use brindle_remote_protocol::SessionId;
use brindle_remote_protocol::holder::{Spawn, SpawnMode, SpawnPipes};

use super::{CORE_BACKOFF_MAX, CORE_BACKOFF_START, CORE_GRACE};
use crate::pty::{OUTPUT_DRAIN_IDLE, OUTPUT_DRAIN_LIMIT};

/// Why a `SPAWN` is refused; its text is the `SPAWN_FAILED`'s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SpawnRefusal {
    /// The holder was told to `DRAIN`.
    Draining,
    /// It has a session with this id already.
    Exists(SessionId),
    /// The command is there, but empty.
    EmptyCommand,
    /// The PTY would have no rows or no columns.
    NoSize,
}

impl fmt::Display for SpawnRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Draining => f.write_str("the holder is draining and takes no new sessions"),
            Self::Exists(id) => write!(f, "session {id} already exists"),
            Self::EmptyCommand => f.write_str("command must not be empty"),
            Self::NoSize => f.write_str("PTY rows and columns must be non-zero"),
        }
    }
}

/// Why `spawn` is refused by a holder that is `draining` or (`exists`)
/// already has a session with its id; the first reason in that order.
pub(super) fn spawn_refusal(draining: bool, exists: bool, spawn: &Spawn) -> Option<SpawnRefusal> {
    if draining {
        Some(SpawnRefusal::Draining)
    } else if exists {
        Some(SpawnRefusal::Exists(spawn.session))
    } else if spawn.command.as_deref() == Some("") {
        Some(SpawnRefusal::EmptyCommand)
    } else if spawn.cols == 0 || spawn.rows == 0 {
        Some(SpawnRefusal::NoSize)
    } else {
        None
    }
}

/// Why a `SPAWN_PIPES` is refused by a holder that is `draining` or
/// (`exists`) already has a session with its id; the first reason in that
/// order, as for a `SPAWN`. Its command is required, so an empty one is
/// refused too (the core refuses it first, decision 2).
pub(super) fn pipe_spawn_refusal(
    draining: bool,
    exists: bool,
    spawn: &SpawnPipes,
) -> Option<SpawnRefusal> {
    if draining {
        Some(SpawnRefusal::Draining)
    } else if exists {
        Some(SpawnRefusal::Exists(spawn.session))
    } else if spawn.command.is_empty() {
        Some(SpawnRefusal::EmptyCommand)
    } else {
        None
    }
}

/// The H1 version from which a core is shown pipe sessions (the core's
/// `policy::PIPES_PROTOCOL` is the same number, for placing them).
pub(super) const PIPES_PROTOCOL: u32 = 3;

/// Whether a core that negotiated H1 `protocol` is shown a session of
/// `mode`: in `WELCOME`, by its output, checkpoints' tails and exit. One
/// below [`PIPES_PROTOCOL`] isn't shown pipe sessions at all (an older core
/// adopting a newer holder, decision 2): they keep running, their output
/// only grows the tail, and they come back with a core that speaks it.
pub(super) fn shown(protocol: u32, mode: SpawnMode) -> bool {
    match mode {
        SpawnMode::Pty => true,
        SpawnMode::Pipes => protocol >= PIPES_PROTOCOL,
    }
}

/// Whether more of a session's output may come from its PTY.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Output {
    Open,
    /// It ended (the program's side closed, or reading failed), or the
    /// session was hung up and its PTY closed.
    Closed,
}

/// Whether the session of a program reaped at `reaped_at` is finished at
/// `now`: its output is closed, or has been quiet for [`OUTPUT_DRAIN_IDLE`]
/// (since the reap or its `last_output`, whichever is later), or the reap
/// is [`OUTPUT_DRAIN_LIMIT`] old, so a child that keeps the PTY open and
/// writing doesn't keep the session.
pub(super) fn drained(now: Instant, reaped_at: Instant, last_output: Instant, output: Output) -> bool {
    let quiet_since = reaped_at.max(last_output);
    match output {
        Output::Closed => true,
        Output::Open => now >= quiet_since + OUTPUT_DRAIN_IDLE || now >= reaped_at + OUTPUT_DRAIN_LIMIT,
    }
}

/// When a holder with sessions and no core starts one: after
/// [`CORE_GRACE`] without a core, then again after a backoff that starts at
/// [`CORE_BACKOFF_START`] and doubles up to [`CORE_BACKOFF_MAX`], until a
/// core connects.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CoreStart {
    /// When it starts the next one; `None` until a core is needed.
    next: Option<Instant>,
    /// How long it waits after that one.
    backoff: Duration,
}

impl CoreStart {
    pub(super) fn new() -> Self {
        Self {
            next: None,
            backoff: CORE_BACKOFF_START,
        }
    }

    /// A core connected: the next one it misses gets the grace and the
    /// first backoff again.
    pub(super) fn connected(&mut self) {
        *self = Self::new();
    }

    /// The core left at `now`: the next one starts after [`CORE_GRACE`],
    /// unless one connects first.
    pub(super) fn lost(&mut self, now: Instant) {
        self.next = Some(now + CORE_GRACE);
    }

    /// One turn of the loop at `now`, in which a core is `needed` (it has
    /// sessions and no core): whether to start one now.
    pub(super) fn step(&mut self, now: Instant, needed: bool) -> bool {
        if !needed {
            return false;
        }
        let due = *self.next.get_or_insert(now + CORE_GRACE);
        if now < due {
            return false;
        }
        self.next = Some(now + self.backoff);
        self.backoff = (self.backoff * 2).min(CORE_BACKOFF_MAX);
        true
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use brindle_remote_protocol::serde_json::Value;

    use super::*;

    fn spawn(command: Option<&str>, cols: u16, rows: u16) -> Spawn {
        Spawn {
            session: 7,
            command: command.map(str::to_string),
            args: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
            cols,
            rows,
            cell_w: 0,
            cell_h: 0,
            meta: Value::Null,
        }
    }

    #[test]
    fn a_spawn_is_refused_with_todays_texts_in_order() {
        let good = spawn(Some("sh"), 80, 24);
        let cases = [
            (false, false, good.clone(), None),
            (false, false, spawn(None, 80, 24), None),
            (true, true, spawn(Some(""), 0, 0), Some("the holder is draining and takes no new sessions")),
            (false, true, spawn(Some(""), 0, 0), Some("session 7 already exists")),
            (false, false, spawn(Some(""), 0, 0), Some("command must not be empty")),
            (false, false, spawn(Some("sh"), 0, 24), Some("PTY rows and columns must be non-zero")),
            (false, false, spawn(None, 80, 0), Some("PTY rows and columns must be non-zero")),
        ];
        for (draining, exists, spawn, refused) in cases {
            let refusal = spawn_refusal(draining, exists, &spawn).map(|refusal| refusal.to_string());
            assert_eq!(refusal.as_deref(), refused, "{draining} {exists} {spawn:?}");
        }
    }

    #[test]
    fn a_pipe_spawn_is_refused_draining_then_existing_then_with_no_command() {
        let spawn = |command: &str| SpawnPipes {
            session: 7,
            command: command.into(),
            args: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
            meta: Value::Null,
        };
        let cases = [
            (false, false, spawn("agent"), None),
            (true, true, spawn(""), Some("the holder is draining and takes no new sessions")),
            (false, true, spawn(""), Some("session 7 already exists")),
            (false, false, spawn(""), Some("command must not be empty")),
        ];
        for (draining, exists, spawn, refused) in cases {
            let refusal = pipe_spawn_refusal(draining, exists, &spawn).map(|refusal| refusal.to_string());
            assert_eq!(refusal.as_deref(), refused, "{draining} {exists} {spawn:?}");
        }
    }

    #[test]
    fn a_core_below_h1_3_is_shown_only_pty_sessions() {
        for protocol in 1..PIPES_PROTOCOL {
            assert!(shown(protocol, SpawnMode::Pty));
            assert!(!shown(protocol, SpawnMode::Pipes), "H1 {protocol}");
        }
        for protocol in [PIPES_PROTOCOL, PIPES_PROTOCOL + 1] {
            assert!(shown(protocol, SpawnMode::Pty));
            assert!(shown(protocol, SpawnMode::Pipes), "H1 {protocol}");
        }
    }

    #[test]
    fn a_reaped_session_finishes_once_its_output_is_closed_quiet_or_too_long() {
        let reaped = Instant::now();
        let ms = Duration::from_millis;
        let cases = [
            // Closed: at once.
            (reaped, reaped, Output::Closed, true),
            // Open: quiet for the idle time since the reap...
            (reaped + OUTPUT_DRAIN_IDLE - ms(1), reaped, Output::Open, false),
            (reaped + OUTPUT_DRAIN_IDLE, reaped, Output::Open, true),
            // ...or since its last output, if later...
            (reaped + OUTPUT_DRAIN_IDLE, reaped + ms(10), Output::Open, false),
            (reaped + ms(10) + OUTPUT_DRAIN_IDLE, reaped + ms(10), Output::Open, true),
            // ...output before the reap counts from the reap...
            (reaped + OUTPUT_DRAIN_IDLE, reaped - ms(10), Output::Open, true),
            // ...and never past the limit.
            (reaped + OUTPUT_DRAIN_LIMIT - ms(1), reaped + OUTPUT_DRAIN_LIMIT - ms(2), Output::Open, false),
            (reaped + OUTPUT_DRAIN_LIMIT, reaped + OUTPUT_DRAIN_LIMIT, Output::Open, true),
        ];
        for (now, last_output, output, finished) in cases {
            assert_eq!(
                drained(now, reaped, last_output, output),
                finished,
                "{:?} after the reap, output {:?} after, {output:?}",
                now - reaped,
                last_output.checked_duration_since(reaped),
            );
        }
    }

    #[test]
    fn a_core_starts_after_the_grace_then_with_doubling_backoff() {
        let t0 = Instant::now();
        let secs = Duration::from_secs;
        let mut start = CoreStart::new();
        // Not needed: nothing, and no grace starts.
        assert!(!start.step(t0, false));
        assert_eq!(start, CoreStart::new());
        // Needed: the grace runs from the first turn that needs one.
        assert!(!start.step(t0 + secs(1), true));
        assert!(!start.step(t0 + secs(1) + CORE_GRACE - Duration::from_millis(1), true));
        let mut at = t0 + secs(1) + CORE_GRACE;
        assert!(start.step(at, true));
        // Then the backoff, doubling up to its cap.
        let mut backoff = CORE_BACKOFF_START;
        for _ in 0..8 {
            assert!(!start.step(at + backoff - Duration::from_millis(1), true));
            at += backoff;
            assert!(start.step(at, true));
            backoff = (backoff * 2).min(CORE_BACKOFF_MAX);
        }
        assert_eq!(backoff, CORE_BACKOFF_MAX);
    }

    #[test]
    fn a_connected_core_resets_the_backoff_and_a_lost_one_gives_the_grace() {
        let t0 = Instant::now();
        let mut start = CoreStart::new();
        assert!(!start.step(t0, true));
        assert!(start.step(t0 + CORE_GRACE, true));
        assert!(start.step(t0 + CORE_GRACE + CORE_BACKOFF_START, true));
        start.connected();
        assert_eq!(start, CoreStart::new());

        // A core lost at t1: the next starts after the grace from then,
        // with the first backoff.
        let t1 = t0 + Duration::from_secs(100);
        start.lost(t1);
        assert!(!start.step(t1 + CORE_GRACE - Duration::from_millis(1), true));
        assert!(start.step(t1 + CORE_GRACE, true));
        assert!(!start.step(t1 + CORE_GRACE + CORE_BACKOFF_START - Duration::from_millis(1), true));
        assert!(start.step(t1 + CORE_GRACE + CORE_BACKOFF_START, true));
    }
}
