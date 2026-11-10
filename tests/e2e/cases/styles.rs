//! tmux pane styles: `window-style`, `window-active-style` and the border
//! styles.

use super::{CARD_INSET, cells, server};
use crate::brindle::{Run, theme};
use crate::capture::{PxRect, Rgb};
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

/// A tmux profile whose theme (`e2e-alt`) differs from the global `e2e`
/// one, which the window chrome's tokens come from.
pub(super) fn alt_profile(server: &TmuxServer) -> String {
    format!(
        "\n[[profiles]]\nname = \"tmux-alt\"\ntmux = \"control\"\ntmux_session = \"e2e\"\n\
         tmux_args = [\"-L\", \"{}\", \"-f\", \"/dev/null\"]\ntheme = \"e2e-alt\"\n",
        server.socket
    )
}

/// With no border styles, a divider away from the active pane and the
/// zoomed badge take the chrome's tokens (the global theme's), not the
/// tab's theme: the divider is the `border` token 15% towards the
/// foreground and the badge sits on `secondary`. The old colours, from the
/// tab's theme, were #464040 and #945410.
pub fn chrome_fallback(ctx: &Ctx) -> Result<Outcome> {
    let server = split()?;
    server.run(&["split-window", "-v", "-t", "%1"])?;
    server.run(&["select-pane", "-t", "%0"])?;
    let out = Run::new(ctx, &server, "divider")
        .profile("tmux-alt")
        .config(&alt_profile(&server))
        .capture_at(3.5)
        .dump_at(5.0)
        .run()?;
    let image = out.image.as_ref().ok_or("no capture")?;
    let cells = cells(&server, &out, "%0")?;
    let [right, ..] = server.geometry("%1")?;
    let [_, lower, ..] = server.geometry("%2")?;
    // The horizontal divider doesn't border the active pane. It is one
    // pixel row in the middle of its cell row, so each row is looked at.
    let cell = cells.rect(right + 2, lower - 1, 20, 1);
    let rows: Vec<Option<Rgb>> = (cell.y0..cell.y1).map(|y| image.dominant(PxRect { y0: y, y1: y + 1, ..cell })).collect();
    ensure!(
        rows.iter().any(|c| c.is_some_and(|c| c.near(theme::TMUX_DIVIDER, 2))),
        "divider: rows {:?} in {cell:?}, want one of {}",
        rows.iter().map(|c| c.map(Rgb::hex)).collect::<Vec<_>>(),
        theme::TMUX_DIVIDER.hex()
    );

    let server = split()?;
    server.run(&["resize-pane", "-Z", "-t", "%0"])?;
    let out = Run::new(ctx, &server, "zoomed")
        .profile("tmux-alt")
        .config(&alt_profile(&server))
        .capture_at(3.5)
        .dump_at(5.0)
        .run()?;
    let image = out.image.as_ref().ok_or("no capture")?;
    let grid = out.dump.grid()?;
    // The badge is 4 px below the top of the window's card (the grid's,
    // less the 6 px padding) and 8 px in from its right, which is the
    // card's margin and border in from the window's; sample inside it,
    // clear of its rounded corners.
    let scale = grid.scale;
    let (right, top) = (image.width as f32 / scale - CARD_INSET - 8.0, grid.y - 6.0 + 4.0);
    let px = |v: f32| (v * scale).round().max(0.0) as u32;
    let badge = PxRect { x0: px(right - 40.0), y0: px(top + 3.0), x1: px(right - 6.0), y1: px(top + 10.0) };
    let got = image.dominant(badge);
    ensure!(
        got.is_some_and(|c| c.near(theme::CHROME_SECONDARY, 2)),
        "badge: dominant {:?} in {badge:?}, want {}; found {}",
        got.map(Rgb::hex),
        theme::CHROME_SECONDARY.hex(),
        image.summary(badge)
    );
    Ok(Outcome::Pass)
}
