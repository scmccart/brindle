//! Live reload: a theme change redraws open tabs and their zones without
//! restarting them, and ctrl-, opens the config in an editor on this
//! machine. (The colors programs get from OSC 10/11 after a reload are
//! `colors-reload`.)

use super::cells;
use crate::brindle::{Run, theme};
use crate::capture::PxRect;
use crate::{Ctx, Outcome, Result, ensure};

pub fn theme_attached(ctx: &Ctx) -> Result<Outcome> {
    // Two zones side by side with a line of plain text each; the right one
    // (the split) is active, so the dump's grid is its.
    let out = Run::new(ctx, "zones")
        .send("clear; echo plain text\r")
        .action("split_zone_right")
        .send("clear; echo plain text\r")
        .theme_at(4.0, "e2e-alt")
        .capture_at(6.0)
        .dump_at(6.5)
        .run()?;
    ensure!(out.dump.zones.as_deref() == Some("count=2 active=2"), "zones {:?}", out.dump.zones);
    let image = out.image.as_ref().unwrap();
    let right = cells(&out)?;
    let right_text = right.rect(0, 0, 10, 1);
    let right_background = right.rect(2, 8, 20, 10);
    // The left zone's cells lie on the same rows, left of the window's middle.
    let middle = image.width / 2;
    let left_text = PxRect { x0: 0, x1: middle, ..right_text };
    let left_background = PxRect { x0: middle / 8, x1: middle * 3 / 4, ..right_background };
    for (zone, text, background) in [("left", left_text, left_background), ("right", right_text, right_background)] {
        let got = image.dominant(background);
        ensure!(got == Some(theme::ALT_BACKGROUND), "{zone} zone background {:?}", got.map(|c| c.hex()));
        image.expect(&format!("{zone} zone text"), text, theme::ALT_FOREGROUND, 10)?;
        image.expect_none(&format!("{zone} zone text"), text, theme::FOREGROUND)?;
    }

    // A second tab, shown after the switch.
    let out = Run::new(ctx, "local")
        .action("new_tab")
        .send("")
        .action("activate_tab_1")
        .theme_at(2.5, "e2e-alt")
        .capture_at(5.0)
        .dump_at(6.0)
        .run()?;
    ensure!(out.dump.tabs.len() == 2 && out.dump.tabs[0].1, "tabs {:?}", out.dump.tabs);
    let image = out.image.as_ref().unwrap();
    let background = image.dominant(cells(&out)?.rect(2, 8, 20, 10));
    ensure!(background == Some(theme::ALT_BACKGROUND), "tab background {:?}", background.map(|c| c.hex()));
    Ok(Outcome::Pass)
}

/// ctrl-, (`open_config`) with a default profile on an SSH host that can't
/// be resolved: the editor runs in a new tab on this machine, on this
/// machine's config file, rather than trying to reach that host. Linux
/// only, since the editor command differs on Windows.
pub fn config_editor_local(ctx: &Ctx) -> Result<Outcome> {
    const AWAY: &str = "\n[[profiles]]\nname = \"Away\"\nremote = { type = \"ssh\", host = \"brindle-e2e.invalid\" }\n";
    let config = ctx.dir.join("config.toml");
    let out = Run::new(ctx, "run")
        .config(AWAY)
        .default_profile("Away")
        .env("VISUAL", r#"sh -c 'echo editing "$0"; sleep 60'"#)
        .action("open_config")
        .wait_for("editing")
        .dump_at(6.0)
        .run()?;
    ensure!(out.dump.tabs.len() == 2, "tabs: {:?}", out.dump.tabs);
    // A long path can wrap onto the next row, which the dump keeps apart.
    let want = format!("editing {}", config.display());
    ensure!(out.dump.screen.concat().contains(&want), "no {want:?} on the screen: {:?}", out.dump.screen);
    Ok(Outcome::Pass)
}

/// A valid config that Brindle doesn't use as written: an unknown theme, a
/// font size out of range and a binding to an action that doesn't exist.
/// The window's config line names all three (`--- config`), the tab uses
/// the built-in `brindle-dark` theme, and fixing the file while Brindle runs
/// takes the line away.
pub fn config_findings(ctx: &Ctx) -> Result<Outcome> {
    const BAD: &str = "\n[font]\nsize = 200\n\n[keybindings]\n\"ctrl-shift-y\" = \"split_zone_rihgt\"\n";
    let out = Run::new(ctx, "findings").initial_theme("tokyonight").config(BAD).capture_at(5.0).dump_at(6.0).run()?;
    let line = out.dump.config.as_deref().ok_or("no --- config line in the dump")?;
    for (what, text) in [
        ("the unknown theme", "tokyonight"),
        ("the theme used instead", "brindle-dark"),
        ("the clamped size", "font.size 200 is out of range, using 96"),
        ("the binding's keys", "ctrl-shift-y"),
        ("the binding's action", "split_zone_rihgt"),
    ] {
        ensure!(line.contains(text), "the config line doesn't name {what} ({text:?}): {line:?}");
    }
    // The grid is of 96 px text: sample the last row, left of the prompt's
    // reach, rather than the middle of the window.
    let image = out.image.as_ref().ok_or("no capture")?;
    let (cols, rows) = out.dump.size.ok_or("no size")?;
    let background = image.dominant(cells(&out)?.rect(0, rows - 1, cols.min(8), 1));
    ensure!(
        background == Some(theme::BRINDLE_DARK_BACKGROUND),
        "background {:?}, not brindle-dark's",
        background.map(|c| c.hex())
    );

    // The same file, fixed while Brindle runs (`theme_at` rewrites the test
    // config): the line goes, and the test config's own theme applies.
    let out = Run::new(ctx, "fixed")
        .initial_theme("tokyonight")
        .config(BAD)
        .theme_at(3.0, "e2e")
        .capture_at(5.0)
        .dump_at(6.0)
        .run()?;
    ensure!(out.dump.config.is_none(), "the config line stayed after the fix: {:?}", out.dump.config);
    let image = out.image.as_ref().ok_or("no capture")?;
    let background = image.dominant(cells(&out)?.rect(2, 8, 20, 10));
    ensure!(background == Some(theme::BACKGROUND), "background after the fix {:?}", background.map(|c| c.hex()));
    Ok(Outcome::Pass)
}
