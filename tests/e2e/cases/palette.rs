//! The command palette in control-mode tabs: a tmux command run from it
//! acts like its shortcut, and tmux commands are listed only where they
//! apply. (Running a general command is `input-palette-run`.) The `--key`
//! cases press real keys through gpui's keymap: dismissal and focus
//! return, the one-overlay rule with the settings dialog, Tab staying in
//! the palette, wrapping, home and end in the filter, and a prompt reached
//! from the list.

use super::server;
use crate::brindle::{Input, Output, Run, matches_tmux};
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

// ---- keys through gpui's own dispatch (`--key`) ----

/// When step `step` (counted from 1) runs: Brindle runs its steps one
/// second apart, the first 1.5 s in.
fn step_time(step: usize) -> f64 {
    0.5 + step as f64
}

fn expect_focus(out: &Output, place: &str, what: &str) -> Result {
    ensure!(out.dump.focus.as_deref() == Some(place), "{what}: focus {:?}, want {place}", out.dump.focus);
    Ok(())
}

fn expect_closed(out: &Output, what: &str) -> Result {
    ensure!(out.dump.palette.is_empty(), "{what}: the palette is still open: {:?}", out.dump.palette);
    ensure!(out.dump.settings.is_none(), "{what}: the settings dialog is open: {:?}", out.dump.settings);
    Ok(())
}

/// ctrl-shift-p opens the command palette with focus in it, and escape
/// closes it with focus back on the tab's terminal. The one-overlay rule,
/// by key both ways: the palette replaces the settings dialog and the
/// dialog the palette, and closing what replaced the other still gives
/// focus back to the tab.
pub fn escape(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let out = Run::new(ctx, &server, "open").key("ctrl-shift-p").dump_at(step_time(1) + 1.0).run()?;
    ensure!(out.dump.palette.first().is_some_and(|l| l == "list query=\"\""), "palette {:?}", out.dump.palette);
    expect_focus(&out, "palette", "with the palette open")?;

    let out = Run::new(ctx, &server, "escape").key("ctrl-shift-p").key("escape").dump_at(step_time(2) + 1.0).run()?;
    expect_closed(&out, "escape")?;
    expect_focus(&out, "terminal", "after escape")?;

    let out = Run::new(ctx, &server, "over-settings")
        .action("open_settings")
        .key("ctrl-shift-p")
        .dump_at(step_time(2) + 1.0)
        .run()?;
    ensure!(out.dump.settings.is_none(), "the dialog stayed open under the palette: {:?}", out.dump.settings);
    ensure!(!out.dump.palette.is_empty(), "the palette didn't open over the dialog");
    expect_focus(&out, "palette", "the palette over the dialog")?;
    let out = Run::new(ctx, &server, "over-settings-escape")
        .action("open_settings")
        .key("ctrl-shift-p")
        .key("escape")
        .dump_at(step_time(3) + 1.0)
        .run()?;
    expect_closed(&out, "escape from the palette that replaced the dialog")?;
    expect_focus(&out, "terminal", "after the palette that replaced the dialog")?;

    let out = Run::new(ctx, &server, "settings-over")
        .key("ctrl-shift-p")
        .key("ctrl-shift-,")
        .key("escape")
        .dump_at(step_time(3) + 1.0)
        .run()?;
    expect_closed(&out, "escape from the dialog that replaced the palette")?;
    expect_focus(&out, "terminal", "after the dialog that replaced the palette")?;
    Ok(Outcome::Pass)
}

/// Tab and shift-tab keep focus in the palette's filter, with its text.
pub fn tab_stays(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let out = Run::new(ctx, &server, "run")
        .key("ctrl-shift-p")
        .send("split")
        .key("tab")
        .key("shift-tab")
        .key("tab")
        .dump_at(step_time(5) + 1.0)
        .run()?;
    expect_focus(&out, "palette", "after tab and shift-tab")?;
    let want = Input { text: "split".into(), cursor: 5, selection: (5, 5) };
    ensure!(out.dump.input.as_ref() == Some(&want), "input {:?}, want {want:?}", out.dump.input);
    ensure!(out.dump.palette.first().is_some_and(|l| l == "list query=\"split\""), "palette {:?}", out.dump.palette);
    ensure!(!out.dump.screen.iter().any(|l| l.contains("spl")), "typed into the terminal: {:?}", out.dump.screen);
    Ok(Outcome::Pass)
}

/// The font-size commands, which the filter `font size` leaves alone in
/// this order.
const FONT_SIZE: [&str; 3] = ["Increase Font Size", "Decrease Font Size", "Reset Font Size"];

/// Up from the first row highlights the last, and down from the last the
/// first, shown by what enter then runs: with `font size` listing Increase,
/// Decrease and Reset, up twice from Increase is Decrease (without wrapping
/// it would stay on Increase), and down three times is Increase again
/// (without wrapping it would stop on Reset, which changes nothing).
pub fn wrap(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let listed = Run::new(ctx, &server, "listed").key("ctrl-shift-p").send("font size").dump_at(step_time(2) + 1.0).run()?;
    let labels: Vec<&str> = listed.dump.palette.iter().skip(1).map(|r| label(r)).collect();
    ensure!(labels == FONT_SIZE, "rows for \"font size\": {:?}", listed.dump.palette);
    let before = listed.dump.grid()?.cell_width;

    let up = Run::new(ctx, &server, "up")
        .key("ctrl-shift-p")
        .send("font size")
        .key("up")
        .key("up")
        .key("enter")
        .dump_at(step_time(5) + 1.0)
        .run()?;
    expect_closed(&up, "enter")?;
    expect_focus(&up, "terminal", "after running a command")?;
    let after = up.dump.grid()?.cell_width;
    ensure!(after < before, "up, up from the first row didn't run Decrease Font Size: cell width {before} then {after}");

    let down = Run::new(ctx, &server, "down")
        .key("ctrl-shift-p")
        .send("font size")
        .key("down")
        .key("down")
        .key("down")
        .key("enter")
        .dump_at(step_time(6) + 1.0)
        .run()?;
    let after = down.dump.grid()?.cell_width;
    ensure!(after > before, "down past the last row didn't run Increase Font Size: cell width {before} then {after}");

    // Home and end leave the highlight alone: Increase, the first row, still
    // runs (had end gone to the list, Reset would run and change nothing).
    let edges = Run::new(ctx, &server, "home-end")
        .key("ctrl-shift-p")
        .send("font size")
        .key("home")
        .key("end")
        .key("enter")
        .dump_at(step_time(5) + 1.0)
        .run()?;
    let after = edges.dump.grid()?.cell_width;
    ensure!(after > before, "home, end moved the highlight off the first row: cell width {before} then {after}");
    Ok(Outcome::Pass)
}

/// Home and end move the filter's caret, not the highlight.
pub fn home_end(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let out = Run::new(ctx, &server, "home")
        .key("ctrl-shift-p")
        .send("split")
        .key("home")
        .send("x")
        .dump_at(step_time(4) + 1.0)
        .run()?;
    let want = Input { text: "xsplit".into(), cursor: 1, selection: (1, 1) };
    ensure!(out.dump.input.as_ref() == Some(&want), "after home: input {:?}, want {want:?}", out.dump.input);
    expect_focus(&out, "palette", "after home")?;

    let out = Run::new(ctx, &server, "end")
        .key("ctrl-shift-p")
        .send("split")
        .key("home")
        .key("end")
        .dump_at(step_time(4) + 1.0)
        .run()?;
    let want = Input { text: "split".into(), cursor: 5, selection: (5, 5) };
    ensure!(out.dump.input.as_ref() == Some(&want), "after end: input {:?}, want {want:?}", out.dump.input);
    Ok(Outcome::Pass)
}

/// A command that asks for text, run from the palette: the prompt has
/// focus, enter submits it, and focus is back on the tab after.
pub fn prompt_focus(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let pick = |label: &'static str| {
        Run::new(ctx, &server, label).key("ctrl-shift-p").send("tmux: rename window").key("enter")
    };
    let out = pick("asks").dump_at(step_time(3) + 1.0).run()?;
    let prompt = out.dump.palette.first().map(String::as_str).unwrap_or("");
    ensure!(prompt.starts_with("prompt \"Rename tmux window\""), "palette {:?}", out.dump.palette);
    expect_focus(&out, "palette", "in the prompt")?;
    ensure!(out.dump.input.is_some(), "the prompt's input has no focus");

    let out = pick("submits").send("renamed").key("enter").dump_at(step_time(5) + 1.0).run()?;
    expect_closed(&out, "after submitting")?;
    expect_focus(&out, "terminal", "after submitting")?;
    let name = server.run(&["display-message", "-p", "-t", "e2e", "#{window_name}"])?;
    ensure!(name.trim() == "renamed", "window name {name:?}");
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
