//! What a launch says when it can't open what was asked for: an unknown
//! `-p` profile, and a profile shortcut past the last profile.

use crate::brindle::Run;
use crate::{Ctx, Outcome, Result, ensure};

/// `-p wrk`, which no profile is named: the default profile opens, and its
/// tab says so on its first line.
pub fn unknown_profile(ctx: &Ctx) -> Result<Outcome> {
    let out = Run::new(ctx, "run").profile("wrk").prompt().dump_at(3.0).run()?;
    ensure!(out.dump.tabs.len() == 1, "tabs: {:?}", out.dump.tabs);
    let text = out.dump.screen.join("\n");
    ensure!(text.contains("no profile is named \"wrk\"; using Shell"), "the tab doesn't say so: {text:?}");
    Ok(Outcome::Pass)
}

/// ctrl-alt-5 with one profile: a message tab says there is no profile 5,
/// and the first tab is still there.
pub fn shortcut_missing(ctx: &Ctx) -> Result<Outcome> {
    let out = Run::new(ctx, "run").prompt().key("ctrl-alt-5").wait_for("no profile 5").dump_at(4.0).run()?;
    ensure!(out.dump.tabs.len() == 2, "tabs: {:?}", out.dump.tabs);
    let text = out.dump.screen.join("\n");
    ensure!(text.contains("There is no profile 5: the config has 1 profile."), "the message tab says: {text:?}");
    ensure!(out.dump.wait_timeouts.is_empty(), "waits that gave up: {:?}", out.dump.wait_timeouts);
    Ok(Outcome::Pass)
}
