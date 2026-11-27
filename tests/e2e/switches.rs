//! Fault switches (`process R20`): which a case needs, which a binary
//! honours, and whether the case can run (`testing R18`).
//!
//! A switch is a debug-only `BRINDLE_TEST_*` environment variable. The
//! harness picks its binaries at run time (`BRINDLE_E2E_BIN`, the helper
//! overrides), so it asks each one (`--list-fault-switches`) instead of
//! guessing from its own build. Everything here is pure: the runner probes
//! and reads `BRINDLE_E2E_SWITCHES_REQUIRED`, and these decide.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use crate::{Result, ensure};

/// The prefix of every fault switch's name.
const PREFIX: &str = "BRINDLE_TEST_";

/// The switches cases use. The helper's `FEED_PANIC` isn't here: only the
/// real-process tests use it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TestSwitch {
    /// Brindle's emulator panics on output holding the marker.
    SinkPanic,
    /// The launch's listing of the server is slowed or fails.
    LaunchListing,
    /// A session's thread waits between the core's answer and the running
    /// session (OpenSpec change `agents-client-state-fixes`).
    SessionStartDelay,
    /// A holder panics handling input holding the marker.
    HolderPanic,
}

/// Which binary honours a switch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SwitchHost {
    Brindle,
    Helper,
}

impl TestSwitch {
    /// The environment variable, as `--list-fault-switches` prints it.
    pub fn name(self) -> &'static str {
        match self {
            TestSwitch::SinkPanic => "BRINDLE_TEST_SINK_PANIC",
            TestSwitch::LaunchListing => "BRINDLE_TEST_LAUNCH_LISTING",
            TestSwitch::SessionStartDelay => "BRINDLE_TEST_SESSION_START_DELAY",
            TestSwitch::HolderPanic => "BRINDLE_TEST_HOLDER_PANIC",
        }
    }

    pub fn host(self) -> SwitchHost {
        match self {
            TestSwitch::SinkPanic | TestSwitch::LaunchListing | TestSwitch::SessionStartDelay => SwitchHost::Brindle,
            TestSwitch::HolderPanic => SwitchHost::Helper,
        }
    }
}

/// What one binary says it honours: the switches it lists, or why it
/// couldn't say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing {
    pub binary: PathBuf,
    pub switches: Result<BTreeSet<String>>,
}

impl Listing {
    /// A host no selected case needs: never consulted, so never probed.
    pub fn unasked(binary: &Path) -> Self {
        Listing { binary: binary.to_path_buf(), switches: Ok(BTreeSet::new()) }
    }
}

/// The switches `--list-fault-switches`' output names: one per line, blank
/// lines ignored.
pub fn parse_listing(text: &str) -> BTreeSet<String> {
    text.lines().map(str::trim).filter(|line| !line.is_empty()).map(str::to_string).collect()
}

/// Whether a case can run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Readiness {
    Run,
    /// Not here: the reason names what is missing and how to get it.
    Skip(String),
    /// Missing where `BRINDLE_E2E_SWITCHES_REQUIRED` says it mustn't be.
    Fail(String),
}

/// Whether a case needing `needs` can run against these binaries. A missing
/// switch, or a binary that couldn't say, skips the case, or fails it when
/// `required`.
pub fn readiness(needs: &[TestSwitch], brindle: &Listing, helper: &Listing, required: bool) -> Readiness {
    for switch in needs {
        let listing = match switch.host() {
            SwitchHost::Brindle => brindle,
            SwitchHost::Helper => helper,
        };
        let why = match &listing.switches {
            Ok(listed) if listed.contains(switch.name()) => continue,
            Ok(_) => format!(
                "needs {}, which {} doesn't list (a release build has none: run without --release, \
                 or point {} at a debug build)",
                switch.name(),
                listing.binary.display(),
                match switch.host() {
                    SwitchHost::Brindle => "BRINDLE_E2E_BIN",
                    SwitchHost::Helper => crate::common::server::HELPER_VARIABLE,
                }
            ),
            Err(why) => format!("needs {}, and {} couldn't say whether it has it: {why}", switch.name(), listing.binary.display()),
        };
        return if required { Readiness::Fail(why) } else { Readiness::Skip(why) };
    }
    Readiness::Run
}

/// The message for `key`, a variable a case sets on Brindle, when it is a
/// fault switch the case didn't declare: it would skip nothing and fail
/// confusingly on a build without it. `None` for any other variable, and
/// for a declared switch.
pub fn undeclared(key: &str, needs: &[TestSwitch]) -> Option<String> {
    if !key.starts_with(PREFIX) || needs.iter().any(|switch| switch.name() == key) {
        return None;
    }
    Some(format!("the case sets {key} but doesn't declare it in its `needs` (tests/e2e/switches.rs)"))
}

/// The harness's own checks of this module (`--self-test`).
pub fn self_test() -> Result {
    let listing = |binary: &str, names: &[&str]| Listing {
        binary: PathBuf::from(binary),
        switches: Ok(names.iter().map(|name| name.to_string()).collect()),
    };
    let debug_brindle = listing(
        "/debug/brindle",
        &["BRINDLE_TEST_SINK_PANIC", "BRINDLE_TEST_LAUNCH_LISTING", "BRINDLE_TEST_SESSION_START_DELAY"],
    );
    let debug_helper = listing("/debug/brindle-remote", &["BRINDLE_TEST_HOLDER_PANIC", "BRINDLE_TEST_FEED_PANIC"]);
    let release_brindle = listing("/release/brindle", &[]);
    let release_helper = listing("/release/brindle-remote", &[]);
    let all = [TestSwitch::SinkPanic, TestSwitch::LaunchListing, TestSwitch::SessionStartDelay, TestSwitch::HolderPanic];

    // A listed switch runs, whatever the other host has; no needs always run.
    ensure!(readiness(&all, &debug_brindle, &debug_helper, true) == Readiness::Run, "debug binaries didn't run");
    ensure!(
        readiness(&[TestSwitch::SinkPanic], &debug_brindle, &release_helper, false) == Readiness::Run,
        "a Brindle switch needed the helper's listing"
    );
    ensure!(
        readiness(&[TestSwitch::HolderPanic], &release_brindle, &debug_helper, false) == Readiness::Run,
        "a helper switch needed Brindle's listing"
    );
    ensure!(
        readiness(&[TestSwitch::SessionStartDelay], &debug_brindle, &release_helper, false) == Readiness::Run,
        "the session start delay is Brindle's"
    );
    ensure!(
        matches!(readiness(&[TestSwitch::SessionStartDelay], &release_brindle, &debug_helper, false), Readiness::Skip(_)),
        "a release Brindle has no session start delay"
    );
    ensure!(readiness(&[], &release_brindle, &release_helper, true) == Readiness::Run, "no needs didn't run");

    // A missing one skips, naming the switch, the binary and the remedy.
    let skipped = readiness(&[TestSwitch::SinkPanic], &release_brindle, &release_helper, false);
    let Readiness::Skip(why) = &skipped else {
        return Err(format!("a missing switch gave {skipped:?}, not a skip"));
    };
    ensure!(
        why.contains("BRINDLE_TEST_SINK_PANIC") && why.contains("/release/brindle") && why.contains("--release") && why.contains("BRINDLE_E2E_BIN"),
        "the skip says {why:?}"
    );
    let skipped = readiness(&[TestSwitch::HolderPanic], &release_brindle, &release_helper, false);
    let Readiness::Skip(why) = &skipped else {
        return Err(format!("a missing helper switch gave {skipped:?}, not a skip"));
    };
    ensure!(
        why.contains("BRINDLE_TEST_HOLDER_PANIC") && why.contains("/release/brindle-remote") && why.contains(crate::common::server::HELPER_VARIABLE),
        "the skip says {why:?}"
    );
    // One of several missing is enough.
    ensure!(
        matches!(readiness(&all, &debug_brindle, &release_helper, false), Readiness::Skip(_)),
        "a missing helper switch among others didn't skip"
    );

    // Required turns the skip into a failure with the same reason.
    ensure!(
        readiness(&[TestSwitch::SinkPanic], &release_brindle, &release_helper, true)
            == Readiness::Fail(why_of(&readiness(&[TestSwitch::SinkPanic], &release_brindle, &release_helper, false))?),
        "required didn't fail with the skip's reason"
    );

    // A probe that failed says why, and the case isn't run.
    let broken = Listing { binary: PathBuf::from("/old/brindle"), switches: Err("exited with status 2".into()) };
    let skipped = readiness(&[TestSwitch::LaunchListing], &broken, &debug_helper, false);
    let Readiness::Skip(why) = &skipped else {
        return Err(format!("a failed probe gave {skipped:?}, not a skip"));
    };
    ensure!(why.contains("/old/brindle") && why.contains("exited with status 2"), "the skip says {why:?}");
    ensure!(
        matches!(readiness(&[TestSwitch::LaunchListing], &broken, &debug_helper, true), Readiness::Fail(_)),
        "a failed probe didn't fail when required"
    );
    // A host no case needs is never consulted.
    ensure!(
        readiness(&[TestSwitch::SinkPanic], &debug_brindle, &Listing::unasked(Path::new("/x")), true) == Readiness::Run,
        "an unasked host was consulted"
    );

    // Listings parse one name per line.
    ensure!(parse_listing("").is_empty(), "an empty listing has names");
    ensure!(
        parse_listing("BRINDLE_TEST_SINK_PANIC\r\n\n  BRINDLE_TEST_LAUNCH_LISTING \n")
            == ["BRINDLE_TEST_SINK_PANIC", "BRINDLE_TEST_LAUNCH_LISTING"].map(String::from).into_iter().collect(),
        "the listing parsed wrongly"
    );

    // An undeclared switch is a message; anything else is None.
    ensure!(undeclared("BRINDLE_TEST_SINK_PANIC", &[TestSwitch::SinkPanic]).is_none(), "a declared switch was refused");
    let message = undeclared("BRINDLE_TEST_SINK_PANIC", &[]);
    ensure!(message.as_deref().is_some_and(|m| m.contains("BRINDLE_TEST_SINK_PANIC")), "undeclared said {message:?}");
    ensure!(undeclared("BRINDLE_TEST_SINK_PANIC", &[TestSwitch::HolderPanic]).is_some(), "another switch declared it");
    ensure!(undeclared("BRINDLE_NOTIFY_LOG", &[]).is_none(), "another variable was refused");
    ensure!(undeclared("HOME", &[]).is_none(), "HOME was refused");

    // Every switch's name is a fault switch's.
    for switch in all {
        ensure!(switch.name().starts_with(PREFIX), "{switch:?} is named {}", switch.name());
    }
    Ok(())
}

fn why_of(readiness: &Readiness) -> Result<String> {
    match readiness {
        Readiness::Skip(why) | Readiness::Fail(why) => Ok(why.clone()),
        Readiness::Run => Err("a case that should skip would run".into()),
    }
}
