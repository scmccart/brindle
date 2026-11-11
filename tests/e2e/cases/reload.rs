//! Live reload: a theme change redraws open tabs and their zones without
//! restarting them. (The colors programs get from OSC 10/11 after a reload
//! are `colors-reload`.)

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
