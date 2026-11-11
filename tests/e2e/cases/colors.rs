//! Replies to color queries (OSC 10/11 and OSC 4) from programs in local
//! tabs and zones: Brindle answers them from the terminal's theme.

use crate::brindle::{Run, theme};
use crate::capture::Rgb;
use crate::query;
use crate::{Ctx, Outcome, Result, ensure};

fn expect(what: &str, screen: &[String], bg: Rgb, fg: Rgb) -> Result<Outcome> {
    let (got_bg, got_fg) = query::replies(screen);
    ensure!(
        got_bg == bg.report() && got_fg == fg.report(),
        "{what}: replies bg={got_bg:?} fg={got_fg:?}, want {} {}",
        bg.report(),
        fg.report()
    );
    Ok(Outcome::Pass)
}

pub fn local(ctx: &Ctx) -> Result<Outcome> {
    let script = query::script(ctx)?;
    let out = Run::new(ctx, "run").send(&format!("clear; {}\\r", script.display())).dump_at(5.0).run()?;
    expect("in a local tab", &out.dump.screen, theme::BACKGROUND, theme::FOREGROUND)
}

/// A zone split off the tab answers the same way, in its own terminal.
pub fn split_zone(ctx: &Ctx) -> Result<Outcome> {
    let script = query::script(ctx)?;
    let out = Run::new(ctx, "run")
        .action("split_zone_right")
        .send(&format!("clear; {}\\r", script.display()))
        .dump_at(5.0)
        .run()?;
    ensure!(out.dump.zones.as_deref() == Some("count=2 active=2"), "the split isn't the active zone: {:?}", out.dump.zones);
    expect("in a new zone", &out.dump.screen, theme::BACKGROUND, theme::FOREGROUND)
}

pub fn reload(ctx: &Ctx) -> Result<Outcome> {
    let script = query::script(ctx)?;
    let out = Run::new(ctx, "run")
        .theme_at(1.5, "e2e-alt")
        .send("clear\\r")
        .send("clear\\r")
        .send("clear\\r")
        .send(&format!("clear; {}\\r", script.display()))
        .dump_at(8.0)
        .run()?;
    expect("after a reload", &out.dump.screen, theme::ALT_BACKGROUND, theme::ALT_FOREGROUND)
}

/// Palette queries (OSC 4): the theme's ANSI colours, and the xterm colour
/// cube past them.
pub fn palette(ctx: &Ctx) -> Result<Outcome> {
    let script = query::palette_script(ctx)?;
    let out = Run::new(ctx, "run").send(&format!("clear; {} 4 196 1\\r", script.display())).dump_at(5.0).run()?;
    let screen = &out.dump.screen;
    query::expect_palette("from the theme", screen, 4, &theme::BLUE.report())?;
    query::expect_palette("from the cube", screen, 196, &Rgb(0xff, 0, 0).report())?;
    query::expect_palette("from the theme", screen, 1, &theme::RED.report())?;
    Ok(Outcome::Pass)
}

pub fn palette_reload(ctx: &Ctx) -> Result<Outcome> {
    let script = query::palette_script(ctx)?;
    let out = Run::new(ctx, "run")
        .theme_at(1.5, "e2e-alt")
        .send("clear\\r")
        .send("clear\\r")
        .send("clear\\r")
        .send(&format!("clear; {} 4\\r", script.display()))
        .dump_at(8.0)
        .run()?;
    query::expect_palette("after a reload", &out.dump.screen, 4, &theme::ALT_BLUE.report())?;
    Ok(Outcome::Pass)
}
