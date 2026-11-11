//! The strip's `⌄` profile menu, opened with the hidden `open_profile_menu`
//! action and driven with `--key` (a press on `⌄` can't be posted on Linux,
//! so that stays a manual check): its items, choosing a profile through the
//! picker's path, and the one-overlay rule with the palette and the
//! settings dialog.

use crate::brindle::{Output, Run};
use crate::{Ctx, Outcome, Result, ensure};

/// When step `step` (counted from 1) runs: Brindle runs its steps one
/// second apart, the first 1.5 s in.
fn step_time(step: usize) -> f64 {
    0.5 + step as f64
}

/// A run starting in a `Shell` tab, with a second profile, `Other` (a
/// plain `sh`), after it in the config.
fn two_profiles(run: Run) -> Run {
    run.config("\n[[profiles]]\nname = \"Other\"\ncommand = \"sh\"\n")
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
    let out = two_profiles(Run::new(ctx, "open")).action("open_profile_menu").key("down").dump_at(step_time(2) + 1.0).run()?;
    let menu = &out.dump.menu;
    ensure!(menu.first().map(String::as_str) == Some("items=Shell|Other highlighted=0"), "menu {menu:?}");
    ensure!(
        menu.get(1).is_some_and(|r| r.starts_with("Shell  ") && r.ends_with("  [ctrl-alt-1]")),
        "Shell item: {menu:?}"
    );
    ensure!(
        menu.get(2).is_some_and(|r| r.starts_with("Other  sh") && r.ends_with("  [ctrl-alt-2]")),
        "Other item: {menu:?}"
    );
    ensure!(out.dump.palette.is_empty(), "a palette is open: {:?}", out.dump.palette);
    ensure!(out.dump.tabs.len() == 1, "tabs: {:?}", out.dump.tabs);
    expect_focus(&out, "menu", "after open_profile_menu")?;

    let out = two_profiles(Run::new(ctx, "escape"))
        .action("open_profile_menu")
        .key("down")
        .key("escape")
        .dump_at(step_time(3) + 1.0)
        .run()?;
    expect_closed(&out, "after escape")?;
    ensure!(out.dump.tabs.len() == 1, "tabs after escape: {:?}", out.dump.tabs);
    expect_focus(&out, "terminal", "after escape")?;

    let out = two_profiles(Run::new(ctx, "toggle"))
        .action("open_profile_menu")
        .action("open_profile_menu")
        .dump_at(step_time(2) + 1.0)
        .run()?;
    expect_closed(&out, "after open_profile_menu twice")?;
    expect_focus(&out, "terminal", "after open_profile_menu twice")?;

    // The spec's keyboard path: a key the config binds to the action, pressed
    // through the keymap, opens the menu.
    let out = two_profiles(Run::new(ctx, "bound-key"))
        .config("\n[keybindings]\n\"ctrl-shift-m\" = \"open_profile_menu\"\n")
        .key("ctrl-shift-m")
        .dump_at(step_time(1) + 1.0)
        .run()?;
    ensure!(
        out.dump.menu.first().is_some_and(|h| h.starts_with("items=Shell|Other ")),
        "menu after the bound key: {:?}",
        out.dump.menu
    );
    expect_focus(&out, "menu", "after the bound key")?;
    Ok(Outcome::Pass)
}

/// Choosing the second profile (down, down, enter) opens it as ctrl-alt-2
/// would: the `Other` profile opens in a new, active tab running `sh`.
pub fn pick(ctx: &Ctx) -> Result<Outcome> {
    let out = two_profiles(Run::new(ctx, "pick"))
        .action("open_profile_menu")
        .key("down")
        .key("down")
        .key("enter")
        .send("echo \"picked $((6*7))\"\r")
        .dump_at(step_time(5) + 1.5)
        .run()?;
    expect_closed(&out, "after enter")?;
    ensure!(out.dump.tabs.len() == 2 && out.dump.tabs[1].1, "tabs: {:?}", out.dump.tabs);
    ensure!(out.dump.screen.iter().any(|l| l == "picked 42"), "the new tab's screen: {:?}", out.dump.screen);
    expect_focus(&out, "terminal", "after choosing Other")?;
    Ok(Outcome::Pass)
}

/// The menu and the palette are never open together: either one opened
/// over the other replaces it, and so does the settings dialog. Escape
/// from the palette that replaced the menu gives focus back to the
/// terminal.
pub fn palette_exclusive(ctx: &Ctx) -> Result<Outcome> {
    let out = two_profiles(Run::new(ctx, "menu-palette"))
        .action("open_profile_menu")
        .action("open_profile_picker")
        .dump_at(step_time(2) + 1.0)
        .run()?;
    expect_closed(&out, "menu, then the picker")?;
    ensure!(!out.dump.palette.is_empty(), "no palette after the picker: {:?}", out.dump.menu);
    expect_focus(&out, "palette", "menu, then the picker")?;

    let out = two_profiles(Run::new(ctx, "palette-menu"))
        .action("open_profile_picker")
        .action("open_profile_menu")
        .dump_at(step_time(2) + 1.0)
        .run()?;
    ensure!(out.dump.palette.is_empty(), "palette still open: {:?}", out.dump.palette);
    ensure!(!out.dump.menu.is_empty(), "no menu after open_profile_menu");
    expect_focus(&out, "menu", "picker, then the menu")?;

    let out = two_profiles(Run::new(ctx, "commands-menu"))
        .action("open_command_palette")
        .action("open_profile_menu")
        .action("open_command_palette")
        .dump_at(step_time(3) + 1.0)
        .run()?;
    expect_closed(&out, "command palette, menu, command palette")?;
    ensure!(!out.dump.palette.is_empty(), "no command palette");
    expect_focus(&out, "palette", "command palette, menu, command palette")?;

    let out = two_profiles(Run::new(ctx, "menu-settings"))
        .action("open_profile_menu")
        .action("open_settings")
        .dump_at(step_time(2) + 1.0)
        .run()?;
    expect_closed(&out, "menu, then settings")?;
    ensure!(out.dump.settings.is_some(), "no settings dialog");
    expect_focus(&out, "settings", "menu, then settings")?;

    let out = two_profiles(Run::new(ctx, "menu-palette-escape"))
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
