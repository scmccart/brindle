//! Tab commands: Rename Tab's manual title.

use crate::brindle::Run;
use crate::{Ctx, Outcome, Result, ensure};

/// `rename_tab` trims the submitted text, overrides OSC title changes, and
/// clears when given only whitespace so automatic titles resume.
pub fn rename_tab(ctx: &Ctx) -> Result<Outcome> {
    let title = |out: &crate::brindle::Output| out.dump.tabs.first().map(|(t, _)| t.clone()).unwrap_or_default();

    let renamed = Run::new(ctx, "rename")
        .action("rename_tab")
        .send("  server  ")
        .action("picker_confirm")
        .send("printf '\\033]2;osc-title\\007'\r")
        .dump_at(6.5)
        .run()?;
    ensure!(title(&renamed) == "server", "renamed title: {:?}", renamed.dump.tabs);

    let cleared = Run::new(ctx, "clear")
        .send("printf '\\033]2;auto-title\\007'\r")
        .action("rename_tab")
        .send("  server  ")
        .action("picker_confirm")
        .action("rename_tab")
        .send("   ")
        .action("picker_confirm")
        .dump_at(9.5)
        .run()?;
    ensure!(title(&cleared) == "auto-title", "cleared title: {:?}", cleared.dump.tabs);
    Ok(Outcome::Pass)
}
