//! Every wait the real-process tests make, and the only module of theirs
//! that sleeps (`testing R7`).
//!
//! - [`eventually`] and [`until`] poll a condition within a limit scaled by
//!   `BRINDLE_TEST_SLOW` ([`limit`]): a slow machine stretches only the
//!   limits, which passing runs never reach.
//! - [`Quiet`] tells when a sampled value has stood still for a window,
//!   polled through [`eventually`] rather than slept.
//! - [`throttle`] and [`retry_busy_exec`] sleep, but are no waits: the first
//!   is a slow reader's stimulus, the second a bounded retry of a proven OS
//!   transient (`testing R9`).
//! - [`skip`] records a test that skipped for want of a capability, and fails
//!   it where `BRINDLE_TEST_REQUIRE` says the capability must be there.

use std::fs::OpenOptions;
use std::io::{self, Write as _};
use std::path::Path;
use std::sync::OnceLock;
use std::thread;
use std::time::{Duration, Instant};

/// The variable whose positive number scales every [`limit`] (default 1).
pub const SLOW_VAR: &str = "BRINDLE_TEST_SLOW";

/// The variable that lists, comma-separated, the [`Capability`] names whose
/// [`skip`] fails the test instead.
pub const REQUIRE_VAR: &str = "BRINDLE_TEST_REQUIRE";

/// How often [`eventually`] and [`until`] look again.
const POLL: Duration = Duration::from_millis(20);

/// The factor `value` (the variable's text, if set) gives, or why it gives
/// none.
pub fn parse_factor(value: Option<&str>) -> Result<f64, String> {
    let Some(text) = value else {
        return Ok(1.0);
    };
    match text.trim().parse::<f64>() {
        Ok(factor) if factor.is_finite() && factor > 0.0 => Ok(factor),
        _ => Err(format!("{SLOW_VAR} must be a positive number, not {text:?}")),
    }
}

/// The factor, read once; a bad value is a test-setup error.
fn factor() -> f64 {
    static FACTOR: OnceLock<f64> = OnceLock::new();
    *FACTOR.get_or_init(|| parse_factor(variable(SLOW_VAR).as_deref()).unwrap_or_else(|error| panic!("{error}")))
}

/// The variable's text, if set; one that isn't UTF-8 is a setup error.
fn variable(name: &str) -> Option<String> {
    let value = std::env::var_os(name)?;
    Some(value.into_string().unwrap_or_else(|value| panic!("{name} is not UTF-8: {value:?}")))
}

/// `base` scaled by [`SLOW_VAR`].
pub fn limit(base: Duration) -> Duration {
    base.mul_f64(factor())
}

/// Polls `condition` every 20 ms until it holds (true) or
/// [`limit`]`(timeout)` has passed (false).
pub fn eventually(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + limit(timeout);
    loop {
        if condition() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(POLL);
    }
}

/// Polls `probe` until it gives a value, within [`limit`]`(timeout)`; past
/// that it panics, naming `what`.
pub fn until<T>(what: &str, timeout: Duration, mut probe: impl FnMut() -> Option<T>) -> T {
    let mut found = None;
    if eventually(timeout, || {
        found = probe();
        found.is_some()
    }) {
        if let Some(value) = found {
            return value;
        }
    }
    panic!("never: {what} (within {:?})", limit(timeout));
}

/// Watches a sampled value, without blocking: [`Quiet::observe`] reports
/// true once the value has stayed the same for the window. Poll it through
/// [`eventually`], so the window is measured and never slept.
pub struct Quiet {
    window: Duration,
    since: Option<(u64, Instant)>,
}

impl Quiet {
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            since: None,
        }
    }

    /// Notes `value` now; `None` (no sample) starts the window again.
    pub fn observe(&mut self, value: Option<u64>) -> bool {
        let now = Instant::now();
        let Some(value) = value else {
            self.since = None;
            return false;
        };
        match self.since {
            Some((seen, since)) if seen == value => now.saturating_duration_since(since) >= self.window,
            _ => {
                self.since = Some((value, now));
                false
            }
        }
    }
}

/// Pauses a slow reader or client for `pause`. This is a test's stimulus,
/// not a wait: the subject is how the server copes with a slow peer, so it
/// isn't scaled.
pub fn throttle(pause: Duration) {
    thread::sleep(pause);
}

/// Runs `spawn` until it doesn't fail with `ETXTBSY`, at most 100 times, 20
/// ms apart.
///
/// The cause, a proven OS transient (`testing R9`): a freshly written binary
/// can be briefly busy, because another test thread's fork may still hold
/// the writer's fd until that child execs. `brindle`'s `write_executable`
/// documents the same mechanism.
pub fn retry_busy_exec<T>(mut spawn: impl FnMut() -> io::Result<T>) -> io::Result<T> {
    let mut attempts = 0;
    loop {
        match spawn() {
            Err(error) if error.kind() == io::ErrorKind::ExecutableFileBusy && attempts < 100 => {
                attempts += 1;
                thread::sleep(Duration::from_millis(20));
            }
            result => return result,
        }
    }
}

/// What a test may find missing on a machine, and skip for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Capability {
    /// A user manager that makes `systemd-run --user --scope` scopes.
    Scopes,
    /// The packaged ConPTY (`conpty.dll`) beside the test helper.
    PackagedConpty,
}

impl Capability {
    const ALL: [Capability; 2] = [Capability::Scopes, Capability::PackagedConpty];

    /// Its name in [`REQUIRE_VAR`] and in `skipped.log`.
    pub fn name(self) -> &'static str {
        match self {
            Capability::Scopes => "scopes",
            Capability::PackagedConpty => "packaged-conpty",
        }
    }
}

/// The capabilities `value` (the variable's text, if set) requires, or the
/// name it gives that isn't one.
pub fn parse_required(value: Option<&str>) -> Result<Vec<Capability>, String> {
    let Some(text) = value else {
        return Ok(Vec::new());
    };
    text.split(',')
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .map(|name| {
            Capability::ALL.into_iter().find(|capability| capability.name() == name).ok_or_else(|| {
                let names: Vec<_> = Capability::ALL.iter().map(|capability| capability.name()).collect();
                format!("{REQUIRE_VAR} names {name:?}, which isn't one of {}", names.join(", "))
            })
        })
        .collect()
}

/// The capabilities [`REQUIRE_VAR`] requires, read once.
fn required() -> &'static [Capability] {
    static REQUIRED: OnceLock<Vec<Capability>> = OnceLock::new();
    REQUIRED.get_or_init(|| parse_required(variable(REQUIRE_VAR).as_deref()).unwrap_or_else(|error| panic!("{error}")))
}

/// Records that the running test skips for want of `capability`, and why:
/// a line `<test>\t<capability>\t<reason>` appended to
/// `$CARGO_TARGET_TMPDIR/skipped.log` and written to stderr. The caller
/// returns after it. Where [`REQUIRE_VAR`] names the capability it panics
/// with the reason instead, since the machine was meant to have it.
pub fn skip(capability: Capability, reason: &str) {
    let test = thread::current().name().unwrap_or("<unnamed>").to_string();
    if required().contains(&capability) {
        panic!(
            "{test} needs {} ({REQUIRE_VAR} requires it): {reason}",
            capability.name()
        );
    }
    let line = format!("{test}\t{}\t{}\n", capability.name(), reason.replace('\n', " "));
    let log = Path::new(env!("CARGO_TARGET_TMPDIR")).join("skipped.log");
    let written = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log)
        .and_then(|mut file| file.write_all(line.as_bytes()));
    if let Err(error) = written {
        eprintln!("couldn't record the skip in {}: {error}", log.display());
    }
    eprint!("skipped: {line}");
}
