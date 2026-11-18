//! Every wait the e2e cases make goes through [`Scale`], so each limit
//! stretches with `BRINDLE_E2E_SLOW` (`testing R5`, `R7`): a slow machine
//! stretches only the limits, which passing runs never reach.

use std::time::{Duration, Instant};

use crate::{Result, ensure};

/// How often [`Scale::poll`] and [`Scale::observe_for`] look again.
const POLL: Duration = Duration::from_millis(100);

/// The suite's factor (`BRINDLE_E2E_SLOW`), as a `Copy` value that a
/// `call_at` closure can carry where the case's `Ctx` can't go.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scale {
    factor: f64,
}

impl Scale {
    pub fn new(factor: f64) -> Self {
        Self { factor }
    }

    /// `secs`, stretched by the factor.
    pub fn secs(self, secs: f64) -> f64 {
        secs * self.factor
    }

    /// Polls `probe` every 100 ms until it gives a value, for at most `secs`
    /// stretched. The last error it gave says why it failed.
    pub fn poll<T>(self, secs: f64, what: &str, mut probe: impl FnMut() -> Result<Option<T>>) -> Result<T> {
        let deadline = Instant::now() + Duration::from_secs_f64(self.secs(secs));
        let mut last = String::new();
        loop {
            match probe() {
                Ok(Some(value)) => return Ok(value),
                Ok(None) => {}
                Err(why) => last = why,
            }
            if Instant::now() >= deadline {
                let why = if last.is_empty() { String::new() } else { format!(" (last: {last})") };
                return Err(format!("{what}: not within {secs} s{why}"));
            }
            std::thread::sleep(POLL);
        }
    }

    /// Polls `condition` until it holds, for at most `secs` stretched.
    pub fn eventually(self, secs: f64, what: &str, mut condition: impl FnMut() -> bool) -> Result {
        self.poll(secs, what, || Ok(condition().then_some(())))
    }

    /// Calls `sample` every 100 ms for `secs` stretched: a window that
    /// watches for something appearing (a process), where nothing ends it
    /// sooner. The one absence check the suite makes this way.
    pub fn observe_for(self, secs: f64, mut sample: impl FnMut()) {
        let end = Instant::now() + Duration::from_secs_f64(self.secs(secs));
        loop {
            sample();
            if Instant::now() >= end {
                return;
            }
            std::thread::sleep(POLL);
        }
    }
}

/// The harness's own checks of [`Scale`] (`--self-test`).
pub fn self_test() -> Result {
    let scale = Scale::new(2.5);
    ensure!(scale.secs(4.0) == 10.0, "2.5 × 4 s is {}", scale.secs(4.0));
    ensure!(Scale::new(1.0).secs(3.0) == 3.0, "1 × 3 s is {}", Scale::new(1.0).secs(3.0));
    let mut tries = 0;
    let found = Scale::new(1.0).poll(5.0, "the third try", || {
        tries += 1;
        Ok((tries == 3).then_some(tries))
    })?;
    ensure!(found == 3, "poll gave {found}");
    let failed = Scale::new(1.0).poll::<()>(0.0, "nothing", || Err("not yet".into()));
    ensure!(failed == Err("nothing: not within 0 s (last: not yet)".into()), "poll failed with {failed:?}");
    ensure!(Scale::new(1.0).eventually(0.0, "never", || false).is_err(), "eventually held for false");
    let mut samples = 0;
    Scale::new(1.0).observe_for(0.0, || samples += 1);
    ensure!(samples == 1, "an empty window sampled {samples} times");
    Ok(())
}
