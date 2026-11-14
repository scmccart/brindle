//! Local tab zone splitting, working directory inheritance, directional
//! focus, closing zones vs closing the tab, and session restore of a split
//! tab.

use std::path::PathBuf;

use super::{CARD_BORDER, ZONE_GAP};
use crate::brindle::{Run, state_dir, theme};
use crate::capture::{Image, PxRect, Rgb};
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
    let sub = dir(ctx, "work-sub")?;
    let out = Run::new(ctx, "split")
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
    let out2 = Run::new(ctx, "tie-break")
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

    // Split right, type in zone 1 first, then close zone 2 with `close_zone`:
    // zone 1 remains and is back to a single unsplit zone (no `--- zones` line).
    let out = Run::new(ctx, "close-zone")
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
    let out_exit = Run::new(ctx, "exit-zone")
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
    let out_tab = Run::new(ctx, "close-tab")
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

/// A bash profile with its own theme, `e2e-zone` (`ALT_FOREGROUND` on
/// `ALT_BACKGROUND`, with its own accent), while the global theme stays
/// `e2e`. Its zones' cards and headers are drawn in its colours, the card
/// border in `ALT_CARD_BORDER`; the gaps round them, the focus ring and the
/// status dots stay on the global theme's tokens.
pub(super) const ZONE_PROFILE: &str = "\n[[profiles]]\nname = \"Alt\"\ncommand = \"bash\"\nargs = [\"--norc\", \"--noprofile\"]\ntheme = \"e2e-zone\"\n\n[themes.e2e-zone]\nforeground = \"#a0a0a0\"\nbackground = \"#282020\"\naccent = \"#00a0ff\"\n";
/// `e2e-zone`'s accent, which nothing of a zone's chrome may show.
pub(super) const ZONE_ACCENT: Rgb = Rgb(0x00, 0xa0, 0xff);

/// The header's height (`ZONE_HEADER_HEIGHT`, its bottom line included)
/// and the test config's terminal padding, in logical pixels.
const HEADER_HEIGHT: f32 = 28.0;
const PADDING: f32 = 6.0;

/// The most common colour in `r` is `color`, so text and icons drawn over
/// the surface don't matter.
fn expect_dominant(image: &Image, what: &str, r: PxRect, color: Rgb) -> Result {
    let got = image.dominant(r);
    ensure!(
        got.is_some_and(|c| c.near(color, 2)),
        "{what}: dominant {:?} in {r:?}, want {}; found {}",
        got.map(Rgb::hex),
        color.hex(),
        image.summary(r)
    );
    Ok(())
}

/// The spec's "Header uses the terminal's theme" (`zones` and
/// `ui-theme-tokens`): with the tab's profile theme (`e2e-zone`) differing
/// from the global one (`e2e`), a split tab's zone headers are drawn on the
/// zone terminal's background, not the chrome's; the active zone's title
/// is in the terminal's foreground and the other's in its muted
/// foreground, neither in the global theme's foreground; and no line marks
/// the active header, in the global accent (`primary`) or the zone's.
pub fn header_colors(ctx: &Ctx) -> Result<Outcome> {
    let out = Run::new(ctx, "run")
        .profile("Alt")
        .config(ZONE_PROFILE)
        .action("split_zone_right")
        .capture_at(3.5)
        .dump_at(4.0)
        .run()?;
    ensure!(out.dump.zones.as_deref() == Some("count=2 active=2"), "zones {:?}", out.dump.zones);
    let image = out.image.as_ref().ok_or("no capture")?;
    // The dump's grid is the active zone's (the right one): its terminal's
    // top-left plus the padding, right under its header, inside its card's
    // border. The left card is as wide, a gap to its left.
    let grid = out.dump.grid()?;
    let scale = grid.scale;
    let (left, top) = (grid.x - PADDING - CARD_BORDER, grid.y - PADDING - HEADER_HEIGHT);
    let width = image.width as f32 / scale - ZONE_GAP - left;
    let other_left = left - ZONE_GAP - width;
    let rect = |x: f32, y: f32, w: f32, h: f32| {
        let px = |v: f32| (v * scale).round().max(0.0) as u32;
        PxRect { x0: px(x), y0: px(y), x1: px(x + w), y1: px(y + h) }
    };
    ensure!(left > 260.0 && width > 260.0, "zone 2 at x={left}, {width} wide: too narrow to sample");

    for (what, x) in [("active header", left), ("inactive header", other_left)] {
        // Between the title and the buttons: the zone terminal's background.
        let header = rect(x + 60.0, top + 4.0, width - 160.0, 20.0);
        expect_dominant(image, what, header, theme::ALT_BACKGROUND)?;
        image.expect_none(&format!("{what} on the chrome's background"), header, theme::CHROME_BACKGROUND)?;
        // Along its top, under the card's border: no line.
        let line = rect(x + 12.0, top, width - 24.0, 2.0);
        image.expect_none(&format!("a primary line on the {what}"), line, theme::ACCENT)?;
        image.expect_none(&format!("an accent line on the {what}"), line, ZONE_ACCENT)?;
        expect_dominant(image, &format!("top of the {what}"), line, theme::ALT_BACKGROUND)?;
    }

    // The titles, past the header's 10px of padding.
    let title = |x: f32| rect(x + CARD_BORDER + 8.0, top + 6.0, 100.0, 16.0);
    image.expect("active title", title(left), theme::ALT_FOREGROUND, 5)?;
    image.expect_none("active title in the global foreground", title(left), theme::FOREGROUND)?;
    image.expect("inactive title", title(other_left), theme::ALT_CARD_MUTED, 5)?;
    image.expect_none("inactive title in the foreground", title(other_left), theme::ALT_FOREGROUND)?;
    Ok(Outcome::Pass)
}

/// Session restore records and reconstructs a tab's zone split tree,
/// per-zone working directories, and active zone.
pub fn restore(ctx: &Ctx) -> Result<Outcome> {
    let (a, b) = (dir(ctx, "zone-a")?, dir(ctx, "zone-b")?);
    Run::new(ctx, "record")
        // The next run attaches the same shells, in their directories.
        .keep_detached()
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

    let out = Run::new(ctx, "restore")
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
