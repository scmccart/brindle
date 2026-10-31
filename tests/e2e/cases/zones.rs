//! Local tab zone splitting, working directory inheritance, directional
//! focus, closing zones vs closing the tab, and session restore of a split
//! tab.

use std::path::PathBuf;

use super::server;
use crate::brindle::{Run, state_dir};
use crate::{Ctx, Outcome, Result, ensure};

fn dir(ctx: &Ctx, name: &str) -> Result<PathBuf> {
    let dir = ctx.dir.join(name);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir)
}

/// Splitting a local tab right and down produces a binary tree of zones,
/// inherits the active zone's working directory, and supports directional
/// focus (`focus_zone_*`).
pub fn split_and_focus(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let sub = dir(ctx, "work-sub")?;
    let out = Run::new(ctx, &server, "split")
        .profile("Shell")
        .send(&format!("cd '{}'\r", sub.display()))
        .action("split_zone_right")
        .send("pwd\r")
        .action("split_zone_down")
        .send("echo zone3-here\r")
        .action("focus_zone_left")
        .send("echo zone1-focused\r")
        .dump_at(8.5)
        .run()?;

    ensure!(out.dump.zones.as_deref() == Some("count=3 active=1"), "zones header: {:?}", out.dump.zones);
    ensure!(
        out.dump.zone_list
            == [
                "id=1 rect=0,0,0.5,1 active",
                "id=2 rect=0.5,0,0.5,0.5",
                "id=3 rect=0.5,0.5,0.5,0.5",
            ],
        "zone_list: {:?}",
        out.dump.zone_list
    );
    ensure!(
        out.dump.screen.iter().any(|l| l.contains("zone1-focused")),
        "left zone didn't receive input after focus_zone_left: {:?}",
        out.dump.screen
    );

    // Move right from the tall left zone: tie-breaker picks the top-right zone (id=2),
    // which ran `pwd` inside `sub`.
    let out2 = Run::new(ctx, &server, "tie-break")
        .profile("Shell")
        .send(&format!("cd '{}'\r", sub.display()))
        .action("split_zone_right")
        .send("pwd\r")
        .action("split_zone_down")
        .action("focus_zone_left")
        .action("focus_zone_right")
        .dump_at(8.5)
        .run()?;
    ensure!(out2.dump.zones.as_deref() == Some("count=3 active=2"), "zones header: {:?}", out2.dump.zones);
    ensure!(
        out2.dump.screen.concat().contains(&sub.display().to_string()),
        "zone 2 didn't inherit working directory {:?}: {:?}",
        sub,
        out2.dump.screen
    );
    Ok(Outcome::Pass)
}

/// Closing a zone (`close_zone` or shell `exit`) promotes its sibling and
/// hides the zone lines when back to a single zone; `close_tab` closes all
/// zones and the tab together.
pub fn close(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;

    // Split right, type in zone 1 first, then close zone 2 with `close_zone`:
    // zone 1 remains and is back to a single unsplit zone (no `--- zones` line).
    let out = Run::new(ctx, &server, "close-zone")
        .profile("Shell")
        .send("echo kept-zone1\r")
        .action("split_zone_right")
        .action("close_zone")
        .dump_at(6.0)
        .run()?;
    ensure!(out.dump.tabs.len() == 1, "tabs after close_zone: {:?}", out.dump.tabs);
    ensure!(out.dump.zones.is_none(), "single zone should not emit --- zones: {:?}", out.dump.zones);
    ensure!(
        out.dump.screen.iter().any(|l| l.contains("kept-zone1")),
        "zone 1 screen after zone 2 closed: {:?}",
        out.dump.screen
    );

    // Shell `exit` in one of two zones closes only that zone.
    let out_exit = Run::new(ctx, &server, "exit-zone")
        .profile("Shell")
        .send("echo survivor\r")
        .action("split_zone_down")
        .send("exit\r")
        .dump_at(6.0)
        .run()?;
    ensure!(out_exit.dump.tabs.len() == 1, "tabs after exit in zone 2: {:?}", out_exit.dump.tabs);
    ensure!(out_exit.dump.zones.is_none(), "zones after exit in zone 2: {:?}", out_exit.dump.zones);
    ensure!(
        out_exit.dump.screen.iter().any(|l| l.contains("survivor")),
        "survivor screen after zone 2 exited: {:?}",
        out_exit.dump.screen
    );

    // `close_tab` on a split tab closes the whole tab (leaving the second tab).
    let out_tab = Run::new(ctx, &server, "close-tab")
        .profile("Shell")
        .action("new_tab")
        .send("echo second-tab\r")
        .action("prev_tab")
        .action("split_zone_right")
        .action("close_tab")
        .dump_at(8.0)
        .run()?;
    ensure!(out_tab.dump.tabs.len() == 1, "tabs after close_tab on split tab: {:?}", out_tab.dump.tabs);
    ensure!(
        out_tab.dump.screen.iter().any(|l| l.contains("second-tab")),
        "remaining tab screen: {:?}",
        out_tab.dump.screen
    );
    Ok(Outcome::Pass)
}

/// Session restore records and reconstructs a tab's zone split tree,
/// per-zone working directories, and active zone.
pub fn restore(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let (a, b) = (dir(ctx, "zone-a")?, dir(ctx, "zone-b")?);
    Run::new(ctx, &server, "record")
        .restore()
        .default_profile("Shell")
        .send(&format!("cd '{}'\r", a.display()))
        .action("split_zone_right")
        .send(&format!("cd '{}'\r", b.display()))
        .dump_at(6.5)
        .run()?;

    let state = std::fs::read_to_string(state_dir(ctx).join("session.toml")).map_err(|e| e.to_string())?;
    ctx.save("recorded-zones.toml", &state);
    ensure!(state.contains("split = \"horizontal\""), "recorded state lacks split:\n{state}");

    let out = Run::new(ctx, &server, "restore")
        .restore()
        .default_profile("Shell")
        .send("pwd\r")
        .dump_at(4.5)
        .run()?;
    ensure!(out.dump.zones.as_deref() == Some("count=2 active=2"), "restored zones: {:?}", out.dump.zones);
    ensure!(
        out.dump.zone_list == ["id=1 rect=0,0,0.5,1", "id=2 rect=0.5,0,0.5,1 active"],
        "restored zone_list: {:?}",
        out.dump.zone_list
    );
    ensure!(
        out.dump.screen.concat().contains(&b.display().to_string()),
        "active restored zone pwd: {:?}",
        out.dump.screen
    );
    Ok(Outcome::Pass)
}
