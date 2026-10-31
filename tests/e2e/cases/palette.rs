//! The command palette in control-mode tabs: a tmux command run from it
//! acts like its shortcut, and tmux commands are listed only where they
//! apply. (Running a general command is `input-palette-run`.)

use super::server;
use crate::brindle::{Output, Run, matches_tmux};
use crate::{Ctx, Outcome, Result, ensure};

/// "tmux: Split Right" from the palette splits the active pane, and focus
/// moves to the new pane.
pub fn tmux_action(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let out = Run::new(ctx, &server, "run")
        .action("open_command_palette")
        .send("tmux: split right")
        .action("picker_confirm")
        .dump_at(5.0)
        .run()?;
    ensure!(out.dump.palette.is_empty(), "the palette is still open: {:?}", out.dump.palette);
    let panes = server.run(&["list-panes", "-t", "e2e", "-F", "#{pane_id}"])?;
    ensure!(panes.lines().collect::<Vec<_>>() == ["%0", "%1"], "panes after the split: {panes:?}");
    ensure!(server.active_pane()? == "%1", "the new pane isn't active in tmux");
    matches_tmux(&server, &out.dump, "%1")?;
    Ok(Outcome::Pass)
}

/// The palette's rows, opened with no filter.
fn rows(out: &Output) -> Result<&[String]> {
    let (header, rows) = out.dump.palette.split_first().ok_or("the palette isn't open")?;
    ensure!(header == "list query=\"\"", "palette header {header:?}");
    Ok(rows)
}

fn label(row: &str) -> &str {
    row.split("  ").next().unwrap_or(row)
}

/// tmux commands are listed for a control-mode tab, with their shortcuts;
/// a local tab lists zone commands and only "tmux: New Session" of the tmux
/// commands.
pub fn lists_for_tab(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let out = Run::new(ctx, &server, "tmux").action("open_command_palette").dump_at(3.5).run()?;
    let tmux = rows(&out)?;
    ensure!(tmux.iter().any(|r| label(r) == "tmux: Split Right"), "tmux tab rows: {tmux:?}");
    ensure!(tmux.iter().any(|r| r == "tmux: Detach  [ctrl-shift-d]"), "tmux tab rows: {tmux:?}");
    ensure!(!tmux.iter().any(|r| label(r).contains("Zone")), "zone commands in a tmux tab: {tmux:?}");
    ensure!(tmux.iter().any(|r| label(r) == "Clear Scrollback"), "tmux tab rows: {tmux:?}");
    ensure!(tmux.iter().any(|r| label(r) == "Rename Tab"), "tmux tab rows: {tmux:?}");

    let out = Run::new(ctx, &server, "local").profile("Shell").action("open_command_palette").dump_at(3.5).run()?;
    let local = rows(&out)?;
    let listed: Vec<&str> = local.iter().map(|r| label(r)).filter(|l| l.starts_with("tmux: ")).collect();
    ensure!(listed == ["tmux: New Session"], "tmux commands in a local tab: {listed:?}");
    ensure!(local.iter().any(|r| r == "Split Zone Right  [ctrl-shift-e]"), "local tab rows: {local:?}");
    ensure!(local.iter().any(|r| r == "Split Zone Down  [ctrl-shift-o]"), "local tab rows: {local:?}");
    ensure!(local.iter().any(|r| r == "Close Zone  [ctrl-shift-x]"), "local tab rows: {local:?}");
    ensure!(local.iter().any(|r| label(r) == "Focus Zone Left"), "local tab rows: {local:?}");
    ensure!(local.iter().any(|r| label(r) == "Clear Scrollback"), "local tab rows: {local:?}");
    ensure!(local.iter().any(|r| label(r) == "Rename Tab"), "local tab rows: {local:?}");
    Ok(Outcome::Pass)
}
