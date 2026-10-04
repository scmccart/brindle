//! tmux pane styles: `window-style`, `window-active-style` and the border
//! styles.

use super::{cells, server};
use crate::brindle::{Run, theme};
use crate::capture::Rgb;
use crate::tmux::TmuxServer;
use crate::{Ctx, Outcome, Result, ensure};

fn split() -> Result<TmuxServer> {
    let server = server()?;
    server.run(&["split-window", "-h", "-t", "e2e"])?;
    Ok(server)
}

/// `window-style fg=blue` tints a pane's plain text; SGR colors stay.
pub fn tint(ctx: &Ctx) -> Result<Outcome> {
    let server = split()?;
    server.run(&["set", "-p", "-t", "%1", "window-style", "fg=blue"])?;
    for pane in ["%0", "%1"] {
        server.type_line(pane, r"clear; echo plain text; printf '\e[31mred text\e[0m\n'")?;
    }
    ctx.sleep(0.5);
    let out = Run::new(ctx, &server, "run").capture_at(3.5).dump_at(5.0).run()?;
    let image = out.image.as_ref().unwrap();
    let cells = cells(&server, &out, &server.active_pane()?)?;
    let [left, ..] = server.geometry("%1")?;
    image.expect("styled plain text", cells.rect(left, 0, 10, 1), theme::BLUE, 10)?;
    image.expect("styled red text", cells.rect(left, 1, 8, 1), theme::RED, 10)?;
    image.expect("plain text", cells.rect(0, 0, 10, 1), theme::FOREGROUND, 10)?;
    image.expect_none("unstyled pane", cells.rect(0, 0, 10, 2), theme::BLUE)?;
    image.expect("red text", cells.rect(0, 1, 8, 1), theme::RED, 10)?;
    Ok(Outcome::Pass)
}

/// `window-active-style` follows the active pane.
pub fn active_pane(ctx: &Ctx) -> Result<Outcome> {
    let server = split()?;
    server.run(&["set", "-w", "-t", "e2e", "window-active-style", "bg=#303050"])?;
    server.run(&["select-pane", "-t", "%0"])?;
    let out = Run::new(ctx, &server, "run").at(2.0, &["select-pane", "-t", "%1"]).capture_at(4.5).dump_at(6.0).run()?;
    let image = out.image.as_ref().unwrap();
    let cells = cells(&server, &out, "%1")?;
    let [left, ..] = server.geometry("%1")?;
    let active = image.dominant(cells.rect(left + 5, 10, 20, 5));
    let inactive = image.dominant(cells.rect(5, 10, 20, 5));
    ensure!(active == Some(Rgb(0x30, 0x30, 0x50)), "active pane background {:?}", active.map(Rgb::hex));
    ensure!(inactive == Some(theme::BACKGROUND), "inactive pane background {:?}", inactive.map(Rgb::hex));
    Ok(Outcome::Pass)
}

/// Border styles color the dividers; tmux's built-in values (the green
/// active border) leave the theme's colors.
pub fn borders(ctx: &Ctx) -> Result<Outcome> {
    let run = |label: &'static str, styled: bool| -> Result {
        let server = split()?;
        server.run(&["split-window", "-v", "-t", "%1"])?;
        server.run(&["select-pane", "-t", "%0"])?;
        if styled {
            server.run(&["set", "-w", "-t", "e2e", "pane-border-style", "fg=magenta"])?;
            server.run(&["set", "-w", "-t", "e2e", "pane-active-border-style", "fg=yellow"])?;
        }
        let out = Run::new(ctx, &server, label).capture_at(3.5).dump_at(5.0).run()?;
        let image = out.image.as_ref().unwrap();
        let cells = cells(&server, &out, "%0")?;
        let [right, ..] = server.geometry("%1")?;
        let [_, lower, ..] = server.geometry("%2")?;
        // The vertical divider borders the active pane; the horizontal one doesn't.
        let vertical = cells.rect(right - 1, 2, 1, 8);
        let horizontal = cells.rect(right + 2, lower - 1, 20, 1);
        if styled {
            image.expect("active divider", vertical, theme::YELLOW, 5)?;
            image.expect("divider", horizontal, theme::MAGENTA, 5)?;
        } else {
            image.expect("active divider", vertical, theme::ACCENT, 5)?;
            image.expect_none("active divider", vertical, theme::GREEN)?;
            image.expect_none("divider", horizontal, theme::MAGENTA)?;
            image.expect_none("divider", horizontal, theme::YELLOW)?;
        }
        Ok(())
    };
    run("styled", true)?;
    run("default", false)?;
    Ok(Outcome::Pass)
}
