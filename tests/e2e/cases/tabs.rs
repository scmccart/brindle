//! The tab strip: its colours (the window's chrome tokens, the faces in a
//! muted pill, each in its own terminal's colours with no line), its `+`
//! drawn legibly, the strip as a keyboard stop (`focus_tab_strip`, then the
//! tab list's arrows, home and end, and enter or escape back to the
//! terminal), and tab actions leaving focus where it was. Pointer presses
//! can't be posted on Linux, so a click on a tab or a button, and hovering
//! for a tooltip, stay manual checks.

use super::server;
use crate::brindle::{Output, Run, theme};
use crate::capture::{Image, PxRect, Rgb};
use crate::{Ctx, Outcome, Result, ensure};

/// The `e2e-tab` theme's background and accent, both unlike the global
/// `e2e` theme's (whose accent is the chrome's `primary` token). No face
/// draws the accent: it shows that no line marks the active tab.
const TAB_BACKGROUND: Rgb = Rgb(0x28, 0x20, 0x20);
const TAB_ACCENT: Rgb = Rgb(0x20, 0xc0, 0xa0);

/// A local bash profile, `Alt`, with its own theme, `e2e-tab`, while the
/// global theme stays `e2e`.
fn alt_profile() -> String {
    let ansi: Vec<String> = theme::ANSI.iter().map(|c| format!("\"{}\"", c.hex())).collect();
    format!(
        "\n[[profiles]]\nname = \"Alt\"\ncommand = \"bash\"\nargs = [\"--norc\", \"--noprofile\"]\ntheme = \"e2e-tab\"\n\
         \n[themes.e2e-tab]\nforeground = \"{}\"\nbackground = \"{}\"\naccent = \"{}\"\nansi = [{}]\n",
        theme::ALT_FOREGROUND.hex(),
        TAB_BACKGROUND.hex(),
        TAB_ACCENT.hex(),
        ansi.join(", ")
    )
}

/// The strip's height (`TAB_BAR_HEIGHT`: the 36px pill tab list and the
/// 1px border under it) and the test config's terminal padding, in
/// logical pixels.
const STRIP_HEIGHT: f32 = 37.0;
const PADDING: f32 = 6.0;

/// Where a tab's face is, from the strip's top-left: the pill's 3px of
/// padding all round puts the faces 3px down and 30px high; they start
/// 11px in (the strip's and the list's padding), are at least 80px wide
/// (a `bash` tab is exactly that) and side by side. The samples keep clear
/// of the rounded corners.
const FACE_TOP: f32 = 3.0;
const FACE_HEIGHT: f32 = 30.0;
const FACE_LEFT: f32 = 11.0;
const FACE_STEP: f32 = 80.0;

/// The right end of the strip: the window controls when the window draws
/// them (8px of padding and three 32px `IconSm` buttons 4px apart; under
/// Xvfb's window manager it doesn't), so the empty caption starts this far
/// from the right edge.
const RIGHT_BUTTONS: f32 = 8.0 + 3.0 * 32.0 + 2.0 * 4.0;

/// The middle of the strip's buttons, from its top: `IconSm` buttons
/// centred in the 36px above the border, their 16px icons too.
const BUTTON_MIDDLE: f32 = 18.0;

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

impl Strip {
    /// Tab `ix`'s face, clear of its rounded corners.
    fn face(&self, ix: usize) -> PxRect {
        let left = FACE_LEFT + FACE_STEP * ix as f32;
        self.rect(left + 6.0, FACE_TOP + 5.0, 60.0, FACE_HEIGHT - 10.0)
    }

    /// Two rows just inside the top of tab `ix`'s face, where a line
    /// marking the active tab would run into it. They start 1px below the
    /// face's top edge: which device row the edge falls on varied by one
    /// between runs (the strip's origin is derived from the dump's logical
    /// pixels; the exact cause was not chased), and a band that began on
    /// the edge read the pill's muted background in two of seven runs.
    fn face_top(&self, ix: usize) -> PxRect {
        let left = FACE_LEFT + FACE_STEP * ix as f32;
        self.rect(left + 8.0, FACE_TOP + 1.0, 56.0, 2.0)
    }

    /// The pill's bottom padding under tab `ix`'s face.
    fn under_face(&self, ix: usize) -> PxRect {
        let left = FACE_LEFT + FACE_STEP * ix as f32;
        self.rect(left + 8.0, FACE_TOP + FACE_HEIGHT + 0.5, 56.0, 2.0)
    }
}

/// How far `p` is from `from`: the channels' differences added up.
fn distance(p: Rgb, from: Rgb) -> u32 {
    [(p.0, from.0), (p.1, from.1), (p.2, from.2)].iter().map(|&(a, b)| a.abs_diff(b) as u32).sum()
}

/// The `+` of the new-tab button: how wide its horizontal arm runs and how
/// thick each arm is, in logical pixels, measured in a capture on the
/// strip's background `bg`.
#[derive(Debug)]
struct PlusGlyph {
    across: f32,
    vertical: f32,
    horizontal: f32,
}

/// Finds the `+` right of the tab list (whose pill is the last run of
/// `muted` along the buttons' middle row) and measures it. A stroke's
/// thickness is its pixels' coverage added up across it, each pixel's
/// coverage its distance from `bg` against the glyph's inkiest pixel, so
/// anti-aliased edges count for what they cover.
fn measure_plus(image: &Image, strip: &Strip, bg: Rgb, muted: Rgb) -> Result<PlusGlyph> {
    let s = strip.scale;
    let dev = |v: f32| (v * s).round() as u32;
    let row = dev(strip.top + BUTTON_MIDDLE);
    let (from, to) = (dev(strip.left + FACE_LEFT), dev(strip.left + FACE_LEFT + 400.0).min(image.width));
    let list_end = (from..to)
        .filter(|&x| image.pixel(x, row).near(muted, 2))
        .last()
        .ok_or_else(|| format!("no tab list pill along y={row}: {}", image.summary(PxRect { x0: from, y0: row, x1: to, y1: row + 1 })))?;
    // The horizontal arm: the first run of ink past the list.
    let inked = |x: u32, y: u32| distance(image.pixel(x, y), bg) > 24;
    let start = (list_end + 1..to).find(|&x| inked(x, row)).ok_or("no + right of the tab list")?;
    let end = (start..to).find(|&x| !inked(x, row)).unwrap_or(to);
    let center_x = (start + end) as f32 / 2.0;
    // The inkiest pixel in the glyph's box, where the arms cross.
    let half = dev(8.0);
    let (cx, cy) = (center_x.round() as u32, row);
    let ink = (cx - half..cx + half)
        .flat_map(|x| (cy - half..cy + half).map(move |y| (x, y)))
        .map(|(x, y)| distance(image.pixel(x, y), bg))
        .max()
        .unwrap_or(0)
        .max(1);
    let coverage = |x: u32, y: u32| (distance(image.pixel(x, y), bg) as f32 / ink as f32).min(1.0);
    // Across the vertical arm, 3px above the middle (clear of the
    // horizontal one), and down through the horizontal arm 3px left of it.
    let probe = dev(4.0);
    let vertical: f32 = (cx - probe..=cx + probe).map(|x| coverage(x, cy - dev(3.0))).sum();
    let horizontal: f32 = (cy - probe..=cy + probe).map(|y| coverage(cx - dev(3.0), y)).sum();
    Ok(PlusGlyph { across: (end - start) as f32 / s, vertical: vertical / s, horizontal: horizontal / s })
}

/// The spec's "Mixed themes", "Active tab marker" and "New-tab icon": with
/// a profile theme (`e2e-tab`) differing from the global one (`e2e`), the
/// strip's empty space is the global chrome background token and the tab
/// list's pill the muted token round the faces; the Alt tab's face is its
/// own theme's background whether it is active or not, and no face has a
/// line along its top (neither the tab theme's accent nor the global
/// one's). A Shell tab beside it is in the global theme's terminal
/// background. The `+` is a 16px icon (its arms lucide's 14 of 24, 9.3px)
/// with strokes at least two pixels wide.
pub fn strip_colors(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let out = Run::new(ctx, &server, "active").profile("Alt").config(&alt_profile()).capture_at(3.5).dump_at(4.0).run()?;
    let image = out.image.as_ref().ok_or("no capture")?;
    let strip = Strip::locate(&out, image)?;
    // The empty caption, left of the window controls: the global theme's
    // chrome background, not the tab theme's.
    let caption = strip.rect_from_right(RIGHT_BUTTONS + 30.0, 12.0, 40.0, 14.0);
    expect_dominant(image, "strip background", caption, theme::CHROME_BACKGROUND)?;
    expect_dominant(image, "active Alt face", strip.face(0), TAB_BACKGROUND)?;
    image.expect_none("a line on the active Alt face", strip.face_top(0), TAB_ACCENT)?;
    image.expect_none("a primary line on the active Alt face", strip.face_top(0), theme::ACCENT)?;
    expect_dominant(image, "top of the active Alt face", strip.face_top(0), TAB_BACKGROUND)?;
    // Right of the face: the pill's padding, then the strip's background.
    let pill = strip.rect(FACE_LEFT + FACE_STEP + 0.5, FACE_TOP + 8.0, 2.0, FACE_HEIGHT - 16.0);
    expect_dominant(image, "the pill right of the face", pill, theme::CHROME_MUTED)?;

    let plus = measure_plus(image, &strip, theme::CHROME_BACKGROUND, theme::CHROME_MUTED)?;
    ensure!(plus.across >= 9.0, "the + is {:.1}px across, under a 16px icon's 9.3: {plus:?}", plus.across);
    ensure!(
        plus.vertical >= 1.8 && plus.horizontal >= 1.8,
        "the + strokes are {:.2}px and {:.2}px, under two pixels: {plus:?}",
        plus.vertical,
        plus.horizontal
    );

    // The Alt tab in the background, a Shell tab (the global theme) active.
    let out = Run::new(ctx, &server, "background")
        .profile("Alt")
        .default_profile("Shell")
        .config(&alt_profile())
        .action("new_tab")
        .capture_at(step_time(1) + 2.0)
        .dump_at(step_time(1) + 2.5)
        .run()?;
    expect_active(&out, 2, 1, "after new_tab")?;
    let image = out.image.as_ref().ok_or("no capture")?;
    let strip = Strip::locate(&out, image)?;
    expect_dominant(image, "background Alt face", strip.face(0), TAB_BACKGROUND)?;
    image.expect_none("a line on the background Alt face", strip.face_top(0), TAB_ACCENT)?;
    // Under the inactive face (the active one's shadow is clear of it): the
    // pill's padding, in the muted token.
    expect_dominant(image, "under the background face", strip.under_face(0), theme::CHROME_MUTED)?;
    expect_dominant(image, "active Shell face", strip.face(1), theme::BACKGROUND)?;
    image.expect_none("a line on the active Shell face", strip.face_top(1), theme::ACCENT)?;
    expect_dominant(image, "top of the active Shell face", strip.face_top(1), theme::BACKGROUND)?;
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

