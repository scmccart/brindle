//! A zone header's menu, opened with the hidden `open_zone_menu` action
//! and driven with `--key` (a right-click on a header can't be posted on
//! Linux, so that, and the zone it focuses first, stay manual checks): its
//! items, splitting and closing the zone it is for, disabled splits on a
//! zone too small for them, escape and a bound key.

use crate::brindle::{Output, Run};
use crate::{Ctx, Outcome, Result, ensure};

/// When step `step` (counted from 1) runs: Brindle runs its steps one
/// second apart, the first 1.5 s in.
fn step_time(step: usize) -> f64 {
    0.5 + step as f64
}

/// A bash tab split right: zone 1 on the left, zone 2 (active) on the right.
fn split(run: Run) -> Run {
    run.profile("Shell").default_profile("Shell").action("split_zone_right")
}

fn expect_focus(out: &Output, place: &str, what: &str) -> Result {
    ensure!(out.dump.focus.as_deref() == Some(place), "{what}: focus {:?}, want {place}", out.dump.focus);
    Ok(())
}

fn expect_items(out: &Output, items: &str, what: &str) -> Result {
    let header = out.dump.menu.first().map(String::as_str);
    ensure!(header == Some(&*format!("items={items} highlighted=none")), "{what}: menu {:?}", out.dump.menu);
    Ok(())
}

/// The menu's items in a config with no agent profile (the spec's "No
/// agent profile": the agent splits are disabled).
const ITEMS: &str = "Split Terminal Right|Split Terminal Down|[Split Agent Right]|[Split Agent Down]\
                     |Split Right With Profile…|Split Down With Profile…|-|Close Zone";

fn expect_closed(out: &Output, what: &str) -> Result {
    ensure!(out.dump.menu.is_empty(), "{what}: the menu is open: {:?}", out.dump.menu);
    Ok(())
}

/// The active zone's menu lists the splits and Close Zone with their
/// default keys and has focus; escape closes it and gives focus back to
/// the zone's terminal; a key the config binds to `open_zone_menu` opens
/// it; and a tab of one zone has the menu too.
pub fn items(ctx: &Ctx) -> Result<Outcome> {
    let out = split(Run::new(ctx, "open")).action("open_zone_menu").dump_at(step_time(2) + 1.0).run()?;
    expect_items(&out, ITEMS, "two zones")?;
    let rows = &out.dump.menu[1..];
    ensure!(
        rows == [
            "Split Terminal Right  [ctrl-shift-e]",
            "Split Terminal Down  [ctrl-shift-o]",
            // GPUI prints the modifiers in its own order.
            "Split Agent Right  [ctrl-alt-shift-e]",
            "Split Agent Down  [ctrl-alt-shift-o]",
            "Split Right With Profile…",
            "Split Down With Profile…",
            "Close Zone  [ctrl-shift-x]",
        ],
        "rows: {rows:?}"
    );
    ensure!(out.dump.zones.as_deref() == Some("count=2 active=2"), "zones {:?}", out.dump.zones);
    expect_focus(&out, "menu", "after open_zone_menu")?;

    let out = split(Run::new(ctx, "escape"))
        .action("open_zone_menu")
        .key("down")
        .key("escape")
        .dump_at(step_time(4) + 1.0)
        .run()?;
    expect_closed(&out, "after escape")?;
    ensure!(out.dump.zones.as_deref() == Some("count=2 active=2"), "zones after escape {:?}", out.dump.zones);
    expect_focus(&out, "terminal", "after escape")?;

    let out = split(Run::new(ctx, "bound-key"))
        .config("\n[keybindings]\n\"ctrl-shift-m\" = \"open_zone_menu\"\n")
        .key("ctrl-shift-m")
        .dump_at(step_time(2) + 1.0)
        .run()?;
    expect_items(&out, ITEMS, "after the bound key")?;
    expect_focus(&out, "menu", "after the bound key")?;

    let out = Run::new(ctx, "one-zone")
        .profile("Shell")
        .default_profile("Shell")
        .action("open_zone_menu")
        .dump_at(step_time(1) + 1.0)
        .run()?;
    expect_items(&out, ITEMS, "one zone")?;
    Ok(Outcome::Pass)
}

/// The spec's "Split from the menu": with the left zone focused, Split
/// Zone Down (down, down, enter) splits that zone, with the new zone below
/// it taking focus. Close Zone (end, enter) closes the zone the menu is
/// for, leaving one zone in the tab.
pub fn choose(ctx: &Ctx) -> Result<Outcome> {
    let out = split(Run::new(ctx, "split-down"))
        .action("focus_zone_left")
        .action("open_zone_menu")
        .key("down")
        .key("down")
        .key("enter")
        .dump_at(step_time(6) + 1.5)
        .run()?;
    expect_closed(&out, "after Split Zone Down")?;
    ensure!(out.dump.zones.as_deref() == Some("count=3 active=3"), "zones {:?}", out.dump.zones);
    ensure!(
        out.dump.zone_list
            == [
                "id=1 rect=0,0,0.5,0.5 kind=terminal profile=\"Shell\"",
                "id=3 rect=0,0.5,0.5,0.5 kind=terminal profile=\"Shell\" active",
                "id=2 rect=0.5,0,0.5,1 kind=terminal profile=\"Shell\"",
            ],
        "zone_list {:?}",
        out.dump.zone_list
    );
    expect_focus(&out, "terminal", "after Split Zone Down")?;

    let out = split(Run::new(ctx, "close"))
        .send("echo zone-two\r")
        .action("focus_zone_left")
        .send("echo zone-one\r")
        .action("open_zone_menu")
        .key("end")
        .key("enter")
        .dump_at(step_time(7) + 1.5)
        .run()?;
    expect_closed(&out, "after Close Zone")?;
    ensure!(out.dump.tabs.len() == 1, "tabs after Close Zone: {:?}", out.dump.tabs);
    ensure!(out.dump.zones.is_none(), "zones after Close Zone: {:?}", out.dump.zones);
    ensure!(out.dump.screen.iter().any(|l| l.contains("zone-two")), "the zone left isn't zone 2: {:?}", out.dump.screen);
    expect_focus(&out, "terminal", "after Close Zone")?;
    Ok(Outcome::Pass)
}

/// The spec's "Too small to split": after splitting down and right until
/// the zone can't be split again, its menu has both splits disabled and
/// Close Zone enabled.
pub fn too_small(ctx: &Ctx) -> Result<Outcome> {
    let mut run = Run::new(ctx, "small").profile("Shell").default_profile("Shell");
    let splits = ["split_zone_down"; 5].into_iter().chain(["split_zone_right"; 7]);
    let steps = splits.clone().count();
    for action in splits {
        run = run.action(action);
    }
    let out = run.action("open_zone_menu").dump_at(step_time(steps + 1) + 1.0).run()?;
    expect_items(
        &out,
        "[Split Terminal Right]|[Split Terminal Down]|[Split Agent Right]|[Split Agent Down]\
         |[Split Right With Profile…]|[Split Down With Profile…]|-|Close Zone",
        "the smallest zone",
    )?;
    Ok(Outcome::Pass)
}
