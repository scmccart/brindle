//! A tab's menu, opened with the hidden `open_tab_menu` action and driven
//! with `--key` (a right-click on a tab can't be posted on Linux, so that
//! stays a manual check): its items and their ends, Move Tab Left, Rename
//! Tab's prompt, Close Tab, escape, a bound key, and the one-overlay rule
//! with the profile menu.

use crate::brindle::{Output, Run};
use crate::{Ctx, Outcome, Result, ensure};

/// When step `step` (counted from 1) runs: Brindle runs its steps one
/// second apart, the first 1.5 s in.
fn step_time(step: usize) -> f64 {
    0.5 + step as f64
}

/// A run starting in a bash tab titled `first`.
fn shell(run: Run) -> Run {
    run.send("printf '\\033]2;first\\007'\r")
}

/// Three bash tabs titled first, second and third, the third active.
fn three_tabs(run: Run) -> Run {
    shell(run)
        .action("new_tab")
        .send("printf '\\033]2;second\\007'\r")
        .action("new_tab")
        .send("printf '\\033]2;third\\007'\r")
}

/// The steps [`three_tabs`] takes.
const THREE_TABS: usize = 5;

fn expect_focus(out: &Output, place: &str, what: &str) -> Result {
    ensure!(out.dump.focus.as_deref() == Some(place), "{what}: focus {:?}, want {place}", out.dump.focus);
    Ok(())
}

fn expect_items(out: &Output, items: &str, what: &str) -> Result {
    let header = out.dump.menu.first().map(String::as_str);
    ensure!(header == Some(&*format!("items={items} highlighted=none")), "{what}: menu {:?}", out.dump.menu);
    Ok(())
}

fn expect_closed(out: &Output, what: &str) -> Result {
    ensure!(out.dump.menu.is_empty(), "{what}: the menu is open: {:?}", out.dump.menu);
    Ok(())
}

fn titles(out: &Output) -> Vec<(&str, bool)> {
    out.dump.tabs.iter().map(|(title, active)| (title.as_str(), *active)).collect()
}

/// The last tab's menu lists the items with Move Tab Right disabled and
/// the default keys, and has focus; the first tab's has Move Tab Left
/// disabled, and a lone tab's both. Escape closes it and gives focus back
/// to the terminal, and a key the config binds to `open_tab_menu` opens it.
pub fn items(ctx: &Ctx) -> Result<Outcome> {
    let out = three_tabs(Run::new(ctx, "last")).action("open_tab_menu").dump_at(step_time(THREE_TABS + 1) + 1.0).run()?;
    expect_items(&out, "Rename Tab|Move Tab Left|[Move Tab Right]|-|Close Tab", "the last tab")?;
    let rows = &out.dump.menu[1..];
    ensure!(
        rows == ["Rename Tab", "Move Tab Left  [ctrl-shift-pageup]", "Move Tab Right  [ctrl-shift-pagedown]", "Close Tab  [ctrl-shift-w]"],
        "the last tab's rows: {rows:?}"
    );
    ensure!(titles(&out) == [("first", false), ("second", false), ("third", true)], "tabs {:?}", out.dump.tabs);
    expect_focus(&out, "menu", "after open_tab_menu")?;

    let out = three_tabs(Run::new(ctx, "first"))
        .action("activate_tab_1")
        .action("open_tab_menu")
        .dump_at(step_time(THREE_TABS + 2) + 1.0)
        .run()?;
    expect_items(&out, "Rename Tab|[Move Tab Left]|Move Tab Right|-|Close Tab", "the first tab")?;

    let out = shell(Run::new(ctx, "lone")).action("open_tab_menu").dump_at(step_time(2) + 1.0).run()?;
    expect_items(&out, "Rename Tab|[Move Tab Left]|[Move Tab Right]|-|Close Tab", "a lone tab")?;

    let out = shell(Run::new(ctx, "escape"))
        .action("open_tab_menu")
        .key("down")
        .key("escape")
        .dump_at(step_time(4) + 1.0)
        .run()?;
    expect_closed(&out, "after escape")?;
    ensure!(out.dump.palette.is_empty(), "a palette is open after escape: {:?}", out.dump.palette);
    expect_focus(&out, "terminal", "after escape")?;

    let out = shell(Run::new(ctx, "bound-key"))
        .config("\n[keybindings]\n\"ctrl-shift-m\" = \"open_tab_menu\"\n")
        .key("ctrl-shift-m")
        .dump_at(step_time(2) + 1.0)
        .run()?;
    expect_items(&out, "Rename Tab|[Move Tab Left]|[Move Tab Right]|-|Close Tab", "after the bound key")?;
    expect_focus(&out, "menu", "after the bound key")?;
    Ok(Outcome::Pass)
}

/// The spec's "Move left": Move Tab Left (down, down, enter) on the second
/// of three tabs makes it the first, and it stays active with focus in its
/// terminal. Rename Tab (down, enter) opens the rename prompt.
pub fn choose(ctx: &Ctx) -> Result<Outcome> {
    let out = three_tabs(Run::new(ctx, "move-left"))
        .action("activate_tab_2")
        .action("open_tab_menu")
        .key("down")
        .key("down")
        .key("enter")
        .dump_at(step_time(THREE_TABS + 5) + 1.0)
        .run()?;
    expect_closed(&out, "after Move Tab Left")?;
    ensure!(titles(&out) == [("second", true), ("first", false), ("third", false)], "tabs {:?}", out.dump.tabs);
    expect_focus(&out, "terminal", "after Move Tab Left")?;

    let out = shell(Run::new(ctx, "rename"))
        .action("open_tab_menu")
        .key("down")
        .key("enter")
        .dump_at(step_time(4) + 1.0)
        .run()?;
    expect_closed(&out, "after Rename Tab")?;
    let header = out.dump.palette.first().map(String::as_str).unwrap_or("");
    ensure!(header.starts_with("prompt \"Rename tab\" text=\"first\""), "palette after Rename Tab: {:?}", out.dump.palette);
    expect_focus(&out, "palette", "after Rename Tab")?;

    // Close Tab (end, enter) on the last of three tabs closes it, and the
    // tab before it becomes active.
    let out = three_tabs(Run::new(ctx, "close"))
        .action("open_tab_menu")
        .key("end")
        .key("enter")
        .dump_at(step_time(THREE_TABS + 3) + 1.5)
        .run()?;
    expect_closed(&out, "after Close Tab")?;
    ensure!(titles(&out) == [("first", false), ("second", true)], "tabs after Close Tab: {:?}", out.dump.tabs);
    Ok(Outcome::Pass)
}

/// The tab menu and the profile menu are never open together: either one
/// opened over the other replaces it.
pub fn exclusive(ctx: &Ctx) -> Result<Outcome> {
    let out = shell(Run::new(ctx, "tab-then-profile"))
        .action("open_tab_menu")
        .action("open_profile_menu")
        .dump_at(step_time(3) + 1.0)
        .run()?;
    ensure!(out.dump.menu.first().is_some_and(|h| h.starts_with("items=Shell ")), "menu {:?}", out.dump.menu);
    expect_focus(&out, "menu", "tab menu, then the profile menu")?;

    let out = shell(Run::new(ctx, "profile-then-tab"))
        .action("open_profile_menu")
        .action("open_tab_menu")
        .dump_at(step_time(3) + 1.0)
        .run()?;
    expect_items(&out, "Rename Tab|[Move Tab Left]|[Move Tab Right]|-|Close Tab", "profile menu, then the tab menu")?;
    expect_focus(&out, "menu", "profile menu, then the tab menu")?;

    let out = shell(Run::new(ctx, "palette-then-tab"))
        .action("open_command_palette")
        .action("open_tab_menu")
        .key("escape")
        .dump_at(step_time(4) + 1.0)
        .run()?;
    expect_closed(&out, "command palette, tab menu, escape")?;
    ensure!(out.dump.palette.is_empty(), "palette still open: {:?}", out.dump.palette);
    expect_focus(&out, "terminal", "command palette, tab menu, escape")?;
    Ok(Outcome::Pass)
}
