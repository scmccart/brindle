//! What a launch says when it can't open what was asked for: an unknown
//! profile under `brindle terminal`, and a profile shortcut past the last
//! profile.

use crate::brindle::Run;
use super::cli::usage_refusal;
use crate::{Ctx, Outcome, Result, ensure};

/// `brindle terminal wrk`, which no profile is named: the launch exits 2
/// before any window opens (it gets no display), naming `wrk` and listing
/// the profiles; and with a config that doesn't parse, the refusal says the
/// config has an error and lists the default config's profiles.
pub fn unknown_profile(ctx: &Ctx) -> Result<Outcome> {
    let config = ctx.dir.join("config.toml");
    std::fs::write(&config, crate::brindle::config("e2e")).map_err(|e| format!("writing the config: {e}"))?;
    let said = usage_refusal(ctx, "unknown", &config, &["terminal", "wrk"])?;
    ensure!(said.contains("no profile is named \"wrk\" (the profiles are: Shell)"), "the refusal says {said:?}");

    let broken = ctx.dir.join("broken.toml");
    std::fs::write(&broken, "[[profiles]\nname = \"Work\"\n").map_err(|e| format!("writing the config: {e}"))?;
    let said = usage_refusal(ctx, "broken", &broken, &["terminal", "Work"])?;
    ensure!(
        said.contains("no profile is named \"Work\" (the config has an error") && said.ends_with("whose profiles are: Shell)"),
        "the refusal says {said:?}"
    );
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
