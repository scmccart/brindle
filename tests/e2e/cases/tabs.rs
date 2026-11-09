//! The tab strip: its colours (the window's chrome tokens, with the active
//! tab in its own terminal's colours), the strip as a keyboard stop
//! (`focus_tab_strip`, then the tab list's arrows, home and end, and enter
//! or escape back to the terminal), and tab actions leaving focus where it
//! was. Pointer presses can't be posted on Linux, so a click on a tab stays
//! a manual check.

use super::server;
use crate::brindle::{Output, Run, theme};
use crate::capture::{Image, PxRect, Rgb};
use crate::{Ctx, Outcome, Result, ensure};

/// A local bash profile with its own theme, `e2e-alt`, while the global
/// theme stays `e2e`.
const ALT_PROFILE: &str = "\n[[profiles]]\nname = \"Alt\"\ncommand = \"bash\"\nargs = [\"--norc\", \"--noprofile\"]\ntheme = \"e2e-alt\"\n";

/// The strip's height (`TAB_BAR_HEIGHT`) and the test config's terminal
/// padding, in logical pixels.
const STRIP_HEIGHT: f32 = 38.0;
const PADDING: f32 = 6.0;

/// When step `step` (counted from 1) runs: Brindle runs its steps one
/// second apart, the first 1.5 s in.
fn step_time(step: usize) -> f64 {
    0.5 + step as f64
}

/// Where the strip is in a capture. The dump's grid origin is the active
/// terminal's top-left plus the padding, and the terminal sits right under
/// the strip, so the strip's bottom-left is the origin less the padding.
struct Strip {
    left: f32,
    top: f32,
    scale: f32,
    /// The window's width in logical pixels.
    window_width: f32,
}

impl Strip {
    fn locate(out: &Output, image: &Image) -> Result<Strip> {
        let grid = out.dump.grid()?;
        Ok(Strip {
            left: grid.x - PADDING,
            top: grid.y - PADDING - STRIP_HEIGHT,
            scale: grid.scale,
            window_width: image.width as f32 / grid.scale,
        })
    }

    /// `w` x `h` logical pixels at `x`,`y` from the strip's top-left corner,
    /// in device pixels.
    fn rect(&self, x: f32, y: f32, w: f32, h: f32) -> PxRect {
        let px = |v: f32| (v * self.scale).round().max(0.0) as u32;
        PxRect { x0: px(self.left + x), y0: px(self.top + y), x1: px(self.left + x + w), y1: px(self.top + y + h) }
    }

    /// As [`Strip::rect`], with `x` measured leftwards from the window's
    /// right edge to the rectangle's right side.
    fn rect_from_right(&self, x: f32, y: f32, w: f32, h: f32) -> PxRect {
        self.rect(self.window_width - self.left - x - w, y, w, h)
    }
}

/// The most common colour in `r` is `color` (within a little rounding
/// through HSL), so text and icons drawn over a surface don't matter.
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

/// With a profile theme (`e2e-alt`) differing from the global one (`e2e`),
/// the strip is drawn in the global theme's tokens and the active tab in
/// its terminal's background, the same colour as the terminal under it.
pub fn strip_colors(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let out = Run::new(ctx, &server, "run").profile("Alt").config(ALT_PROFILE).capture_at(3.5).dump_at(4.0).run()?;
    let image = out.image.as_ref().ok_or("no capture")?;
    let strip = Strip::locate(&out, image)?;
    // The empty caption, left of the window controls: the global theme's
    // chrome background, not the alt theme's (which would be #1a1515).
    let caption = strip.rect_from_right(148.0, 12.0, 40.0, 14.0);
    expect_dominant(image, "strip background", caption, theme::CHROME_BACKGROUND)?;
    // The only tab, active, begins 11px in (the strip's and the list's
    // padding) and is at least 80px wide; below its 2px primary line its
    // face is the Alt profile's terminal background.
    let face = strip.rect(14.0, 11.0, 70.0, 26.0);
    expect_dominant(image, "active tab face", face, theme::ALT_BACKGROUND)?;
    // So is the terminal's padding right under the strip: the face joins it.
    let under = strip.rect(14.0, STRIP_HEIGHT + 1.0, 70.0, PADDING - 2.0);
    expect_dominant(image, "terminal under the tab", under, theme::ALT_BACKGROUND)?;
    Ok(Outcome::Pass)
}

/// A run starting in a bash tab whose `new_tab` opens another bash tab (the
/// test config's default profile is otherwise `tmux`, whose windows arrive
/// as tabs on their own time).
fn shell(run: Run) -> Run {
    run.profile("Shell").default_profile("Shell")
}

/// Three bash tabs, the third active: a new tab opens next to the active
/// one and becomes active.
fn three_tabs(run: Run) -> Run {
    shell(run).action("new_tab").action("new_tab")
}

fn expect_active(out: &Output, count: usize, active: usize, what: &str) -> Result {
    let found = out.dump.tabs.iter().position(|(_, active)| *active);
    ensure!(
        out.dump.tabs.len() == count && found == Some(active),
        "{what}: tabs {:?}, want tab {} of {count} active",
        out.dump.tabs,
        active + 1
    );
    Ok(())
}

fn expect_focus(out: &Output, place: &str, what: &str) -> Result {
    ensure!(out.dump.focus.as_deref() == Some(place), "{what}: focus {:?}, want {place}", out.dump.focus);
    Ok(())
}

/// `focus_tab_strip` puts focus on the strip without changing the tab;
/// right from the last tab wraps to the first with focus staying on the
/// strip; home goes to the first and escape gives focus back to its
/// terminal; left goes to the previous and enter gives focus back.
pub fn strip_keys(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let out = three_tabs(Run::new(ctx, &server, "focus")).action("focus_tab_strip").dump_at(step_time(3) + 1.0).run()?;
    expect_active(&out, 3, 2, "after focus_tab_strip")?;
    expect_focus(&out, "tabs", "after focus_tab_strip")?;

    let out = three_tabs(Run::new(ctx, &server, "right"))
        .action("focus_tab_strip")
        .key("right")
        .dump_at(step_time(4) + 1.0)
        .run()?;
    expect_active(&out, 3, 0, "after right from the last tab")?;
    expect_focus(&out, "tabs", "after right")?;

    let out = three_tabs(Run::new(ctx, &server, "home-escape"))
        .action("focus_tab_strip")
        .key("home")
        .key("escape")
        .dump_at(step_time(5) + 1.0)
        .run()?;
    expect_active(&out, 3, 0, "after home and escape")?;
    expect_focus(&out, "terminal", "after escape")?;

    let out = three_tabs(Run::new(ctx, &server, "left-enter"))
        .action("focus_tab_strip")
        .key("left")
        .key("enter")
        .dump_at(step_time(5) + 1.0)
        .run()?;
    expect_active(&out, 3, 1, "after left and enter")?;
    expect_focus(&out, "terminal", "after enter")?;
    Ok(Outcome::Pass)
}

/// `new_tab` and `close_tab` leave focus in the active tab's terminal, and
/// a new tab opened while the strip has focus takes focus into its own.
pub fn actions_keep_focus(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let out = shell(Run::new(ctx, &server, "new-tab")).action("new_tab").dump_at(step_time(1) + 1.5).run()?;
    expect_active(&out, 2, 1, "after new_tab")?;
    expect_focus(&out, "terminal", "after new_tab")?;

    let out = shell(Run::new(ctx, &server, "close-tab"))
        .action("new_tab")
        .action("close_tab")
        .dump_at(step_time(2) + 1.5)
        .run()?;
    expect_active(&out, 1, 0, "after new_tab and close_tab")?;
    expect_focus(&out, "terminal", "after close_tab")?;

    let out = shell(Run::new(ctx, &server, "from-strip"))
        .action("focus_tab_strip")
        .action("new_tab")
        .dump_at(step_time(2) + 1.5)
        .run()?;
    expect_active(&out, 2, 1, "after new_tab from the strip")?;
    expect_focus(&out, "terminal", "after new_tab from the strip")?;
    Ok(Outcome::Pass)
}
