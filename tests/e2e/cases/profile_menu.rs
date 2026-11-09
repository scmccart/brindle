//! The strip's `⌄` profile menu, opened with the hidden `open_profile_menu`
//! action and driven with `--key` (a press on `⌄` can't be posted on Linux,
//! so that stays a manual check): its items, choosing a profile through the
//! picker's path (a session the window shows included), and the one-overlay
//! rule with the palette and the settings dialog.

use super::server;
use crate::brindle::{Output, Run};
use crate::{Ctx, Outcome, Result, ensure};

const LIST_CLIENTS: &[&str] = &["list-clients", "-t", "e2e"];

/// When step `step` (counted from 1) runs: Brindle runs its steps one
/// second apart, the first 1.5 s in.
fn step_time(step: usize) -> f64 {
    0.5 + step as f64
}

/// A run starting in a bash tab (the test config's default profile is
/// `tmux`).
fn shell(run: Run) -> Run {
    run.profile("Shell").default_profile("Shell")
}

/// How many clients `list-clients` recorded for the session.
fn clients(ctx: &Ctx, label: &str) -> Result<usize> {
    Ok(Run::recorded(ctx, label, "clients")?.lines().filter(|l| !l.trim().is_empty()).count())
}

fn expect_focus(out: &Output, place: &str, what: &str) -> Result {
    ensure!(out.dump.focus.as_deref() == Some(place), "{what}: focus {:?}, want {place}", out.dump.focus);
    Ok(())
}

fn expect_closed(out: &Output, what: &str) -> Result {
    ensure!(out.dump.menu.is_empty(), "{what}: the menu is open: {:?}", out.dump.menu);
    Ok(())
}

/// The menu lists the profiles in configuration order with the picker's
/// descriptions and ctrl-alt-N; down highlights the first, and focus is in
/// the menu. Escape closes it and gives focus back to the terminal, and so
/// does `open_profile_menu` again (the toggle).
pub fn items(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let out = shell(Run::new(ctx, &server, "open")).action("open_profile_menu").key("down").dump_at(step_time(2) + 1.0).run()?;
    let menu = &out.dump.menu;
    ensure!(menu.first().map(String::as_str) == Some("items=Shell|tmux highlighted=0"), "menu {menu:?}");
    ensure!(
        menu.get(1).is_some_and(|r| r.starts_with("Shell  ") && r.ends_with("  [ctrl-alt-1]")),
        "Shell item: {menu:?}"
    );
    ensure!(menu.get(2).map(String::as_str) == Some("tmux  tmux control mode · e2e  [ctrl-alt-2]"), "tmux item: {menu:?}");
    ensure!(out.dump.palette.is_empty(), "a palette is open: {:?}", out.dump.palette);
    ensure!(out.dump.tabs.len() == 1, "tabs: {:?}", out.dump.tabs);
    expect_focus(&out, "menu", "after open_profile_menu")?;

    let out = shell(Run::new(ctx, &server, "escape"))
        .action("open_profile_menu")
        .key("down")
        .key("escape")
        .dump_at(step_time(3) + 1.0)
        .run()?;
    expect_closed(&out, "after escape")?;
    ensure!(out.dump.tabs.len() == 1, "tabs after escape: {:?}", out.dump.tabs);
    expect_focus(&out, "terminal", "after escape")?;

    let out = shell(Run::new(ctx, &server, "toggle"))
        .action("open_profile_menu")
        .action("open_profile_menu")
        .dump_at(step_time(2) + 1.0)
        .run()?;
    expect_closed(&out, "after open_profile_menu twice")?;
    expect_focus(&out, "terminal", "after open_profile_menu twice")?;

    // The spec's keyboard path: a key the config binds to the action, pressed
    // through the keymap, opens the menu.
    let out = shell(Run::new(ctx, &server, "bound-key"))
        .config("\n[keybindings]\n\"ctrl-shift-m\" = \"open_profile_menu\"\n")
        .key("ctrl-shift-m")
        .dump_at(step_time(1) + 1.0)
        .run()?;
    ensure!(
        out.dump.menu.first().is_some_and(|h| h.starts_with("items=Shell|tmux ")),
        "menu after the bound key: {:?}",
        out.dump.menu
    );
    expect_focus(&out, "menu", "after the bound key")?;
    Ok(Outcome::Pass)
}

/// Choosing the second profile (down, down, enter) opens it as ctrl-alt-2
/// would: the `tmux` profile attaches session `e2e` in a new, active tab,
/// with one control client.
pub fn pick(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let out = shell(Run::new(ctx, &server, "pick"))
        .action("open_profile_menu")
        .key("down")
        .key("down")
        .key("enter")
        .record_at(step_time(4) + 2.0, "clients", LIST_CLIENTS)
        .dump_at(step_time(4) + 2.5)
        .run()?;
    expect_closed(&out, "after enter")?;
    ensure!(out.dump.tabs.len() == 2 && out.dump.tabs[1].1, "tabs: {:?}", out.dump.tabs);
    ensure!(clients(ctx, "pick")? == 1, "clients: {:?}", Run::recorded(ctx, "pick", "clients")?);
    expect_focus(&out, "terminal", "after choosing tmux")?;
    Ok(Outcome::Pass)
}

/// With a shell tab and the session's tab in the background, the `tmux`
/// item says "open in this window", and choosing it switches to that tab:
/// no second tab, no second client.
pub fn shown(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let shown = |label| {
        Run::new(ctx, &server, label).profile("Shell").action("new_tab_with_profile_2").send("").action("prev_tab")
    };
    let out = shown("items").action("open_profile_menu").dump_at(step_time(4) + 1.0).run()?;
    let menu = &out.dump.menu;
    ensure!(
        menu.iter().any(|r| r.starts_with("tmux  ") && r.contains("tmux control mode · e2e · open in this window")),
        "menu {menu:?}"
    );
    ensure!(menu.iter().any(|r| r.starts_with("Shell  ") && !r.contains("open in")), "menu {menu:?}");

    let out = shown("choose")
        .action("open_profile_menu")
        .key("down")
        .key("down")
        .key("enter")
        .record_at(step_time(7) + 0.5, "clients", LIST_CLIENTS)
        .dump_at(step_time(7) + 1.0)
        .run()?;
    expect_closed(&out, "after choosing tmux")?;
    ensure!(out.dump.palette.is_empty(), "a palette is open: {:?}", out.dump.palette);
    ensure!(out.dump.tabs.len() == 2 && out.dump.tabs[1].1, "tabs: {:?}", out.dump.tabs);
    ensure!(clients(ctx, "choose")? == 1, "clients: {:?}", Run::recorded(ctx, "choose", "clients")?);
    Ok(Outcome::Pass)
}

/// The menu and the palette are never open together: either one opened
/// over the other replaces it, and so does the settings dialog. Escape
/// from the palette that replaced the menu gives focus back to the
/// terminal.
pub fn palette_exclusive(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let out = shell(Run::new(ctx, &server, "menu-palette"))
        .action("open_profile_menu")
        .action("open_profile_picker")
        .dump_at(step_time(2) + 1.0)
        .run()?;
    expect_closed(&out, "menu, then the picker")?;
    ensure!(!out.dump.palette.is_empty(), "no palette after the picker: {:?}", out.dump.menu);
    expect_focus(&out, "palette", "menu, then the picker")?;

    let out = shell(Run::new(ctx, &server, "palette-menu"))
        .action("open_profile_picker")
        .action("open_profile_menu")
        .dump_at(step_time(2) + 1.0)
        .run()?;
    ensure!(out.dump.palette.is_empty(), "palette still open: {:?}", out.dump.palette);
    ensure!(!out.dump.menu.is_empty(), "no menu after open_profile_menu");
    expect_focus(&out, "menu", "picker, then the menu")?;

    let out = shell(Run::new(ctx, &server, "commands-menu"))
        .action("open_command_palette")
        .action("open_profile_menu")
        .action("open_command_palette")
        .dump_at(step_time(3) + 1.0)
        .run()?;
    expect_closed(&out, "command palette, menu, command palette")?;
    ensure!(!out.dump.palette.is_empty(), "no command palette");
    expect_focus(&out, "palette", "command palette, menu, command palette")?;

    let out = shell(Run::new(ctx, &server, "menu-settings"))
        .action("open_profile_menu")
        .action("open_settings")
        .dump_at(step_time(2) + 1.0)
        .run()?;
    expect_closed(&out, "menu, then settings")?;
    ensure!(out.dump.settings.is_some(), "no settings dialog");
    expect_focus(&out, "settings", "menu, then settings")?;

    let out = shell(Run::new(ctx, &server, "menu-palette-escape"))
        .action("open_profile_menu")
        .action("open_profile_picker")
        .key("escape")
        .dump_at(step_time(3) + 1.0)
        .run()?;
    expect_closed(&out, "after escape")?;
    ensure!(out.dump.palette.is_empty(), "palette still open: {:?}", out.dump.palette);
    expect_focus(&out, "terminal", "after escape from the palette")?;
    Ok(Outcome::Pass)
}
