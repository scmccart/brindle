//! The tab strip: its colours (the window's chrome tokens, with each tab's
//! face in its own terminal's colours and the active one's top line in its
//! theme's accent), the strip as a keyboard stop (`focus_tab_strip`, then
//! the tab list's arrows, home and end, and enter or escape back to the
//! terminal), tab actions leaving focus where it was, and the split and
//! find buttons for the active tab (pressed through `--strip-button`).
//! Pointer presses can't be posted on Linux, so a click on a tab or a
//! button, and hovering for a tooltip, stay manual checks.

use super::server;
use crate::brindle::{Output, Run, theme};
use crate::capture::{Image, PxRect, Rgb};
use crate::{Ctx, Outcome, Result, ensure};

/// The `e2e-tab` theme's background and accent, both unlike the global
/// `e2e` theme's (whose accent is the chrome's `primary` token).
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

/// The strip's height (`TAB_BAR_HEIGHT`: the 36px `line` tab list and the
/// 1px border under it) and the test config's terminal padding, in
/// logical pixels.
const STRIP_HEIGHT: f32 = 37.0;
const PADDING: f32 = 6.0;

/// Where a tab's face is, from the strip's top-left: the list's 3px of
/// padding all round puts the faces 3px down (their 2px active line
/// first) and 30px high; they start 11px in (the strip's and the list's
/// padding), are at least 80px wide and 4px apart. The samples keep clear
/// of the rounded corners and the line.
const FACE_TOP: f32 = 3.0;
const FACE_HEIGHT: f32 = 30.0;
const FACE_LEFT: f32 = 11.0;
const FACE_STEP: f32 = 84.0;

/// The right end of the strip: the window controls when the window draws
/// them (8px of padding and three 24px buttons 4px apart; under Xvfb's
/// window manager it doesn't) and the tab action buttons left of them
/// (4px of padding each side, three 24px buttons 2px apart), so the empty
/// caption starts this far from the right edge.
const RIGHT_BUTTONS: f32 = 88.0 + 84.0;

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
    /// Tab `ix`'s face below its top line, clear of its rounded corners.
    fn face(&self, ix: usize) -> PxRect {
        let left = FACE_LEFT + FACE_STEP * ix as f32;
        self.rect(left + 6.0, FACE_TOP + 5.0, 60.0, FACE_HEIGHT - 10.0)
    }

    /// The 2px line along the top of tab `ix`'s face (where the active
    /// tab's line is drawn).
    fn face_line(&self, ix: usize) -> PxRect {
        let left = FACE_LEFT + FACE_STEP * ix as f32;
        self.rect(left + 8.0, FACE_TOP, 56.0, 2.0)
    }
}

/// The spec's "Mixed themes" and "Active tab marker": with a profile theme
/// (`e2e-tab`) differing from the global one (`e2e`), the strip's empty
/// space is the global chrome background token; the Alt tab's face is its
/// own theme's background whether it is active or not, and while active it
/// has a 2px line in its theme's accent (not the global `primary`). A Shell
/// tab beside it is in the global theme's terminal background, with the
/// global accent's line while it is active.
pub fn strip_colors(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let out = Run::new(ctx, &server, "active").profile("Alt").config(&alt_profile()).capture_at(3.5).dump_at(4.0).run()?;
    let image = out.image.as_ref().ok_or("no capture")?;
    let strip = Strip::locate(&out, image)?;
    // The empty caption, left of the tab action buttons and the window
    // controls: the global theme's chrome background, not the tab theme's.
    let caption = strip.rect_from_right(RIGHT_BUTTONS + 30.0, 12.0, 40.0, 14.0);
    expect_dominant(image, "strip background", caption, theme::CHROME_BACKGROUND)?;
    expect_dominant(image, "active Alt face", strip.face(0), TAB_BACKGROUND)?;
    expect_dominant(image, "active Alt line", strip.face_line(0), TAB_ACCENT)?;
    // Between the face and the terminal: the list's bottom padding, on the
    // strip's background (the faces are `line` tabs, not attached).
    let below = strip.rect(FACE_LEFT + 8.0, FACE_TOP + FACE_HEIGHT + 0.5, 56.0, 2.0);
    expect_dominant(image, "under the face", below, theme::CHROME_BACKGROUND)?;

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
    image.expect_none("background Alt line", strip.face_line(0), TAB_ACCENT)?;
    expect_dominant(image, "active Shell face", strip.face(1), theme::BACKGROUND)?;
    expect_dominant(image, "active Shell line", strip.face_line(1), theme::ACCENT)?;
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

fn expect_buttons(out: &Output, buttons: &str, what: &str) -> Result {
    let header = out.dump.strip.first().map(String::as_str);
    ensure!(header == Some(&*format!("buttons={buttons}")), "{what}: strip {:?}", out.dump.strip);
    Ok(())
}

/// The spec's "Active-tab action buttons", pressed through `--strip-button`
/// (the handler a click runs): the tooltips name the bound keys; Split
/// Right splits a local tab of one zone side by side with focus in the new
/// zone; Find opens the active terminal's find bar with its query focused;
/// on a zone too small to split both split buttons are disabled and a
/// press does nothing; on a tmux tab Split Down splits tmux's pane.
pub fn strip_buttons(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let out = shell(Run::new(ctx, &server, "split-right")).strip_button("split-right").dump_at(step_time(1) + 1.5).run()?;
    ensure!(
        out.dump.strip[1..] == ["tip=Split right  [ctrl-shift-e]", "tip=Split down  [ctrl-shift-o]", "tip=Find  [ctrl-shift-f]"],
        "tooltips: {:?}",
        out.dump.strip
    );
    expect_buttons(&out, "split-right|split-down|find", "two zones")?;
    ensure!(out.dump.zones.as_deref() == Some("count=2 active=2"), "zones {:?}", out.dump.zones);
    ensure!(out.dump.zone_list == ["id=1 rect=0,0,0.5,1", "id=2 rect=0.5,0,0.5,1 active"], "zone_list {:?}", out.dump.zone_list);
    expect_focus(&out, "terminal", "after Split Right")?;

    let out = shell(Run::new(ctx, &server, "find")).strip_button("find").dump_at(step_time(1) + 1.5).run()?;
    ensure!(out.dump.find.is_some(), "no find bar after Find: {:?}", out.dump.find);
    ensure!(out.dump.input.is_some(), "the find query isn't focused (focus {:?})", out.dump.focus);

    // As zone_menu's "too small": split down and right until the zone can't
    // be split again, then press both split buttons.
    let splits = ["split_zone_down"; 5].into_iter().chain(["split_zone_right"; 7]);
    let steps = splits.clone().count();
    let too_small = |label| splits.clone().fold(shell(Run::new(ctx, &server, label)), |run, action| run.action(action));
    let before = too_small("too-small").dump_at(step_time(steps) + 1.5).run()?;
    let out = too_small("pressed")
        .strip_button("split-right")
        .strip_button("split-down")
        .dump_at(step_time(steps + 2) + 1.5)
        .run()?;
    expect_buttons(&before, "[split-right]|[split-down]|find", "the smallest zone")?;
    expect_buttons(&out, "[split-right]|[split-down]|find", "after pressing them")?;
    ensure!(
        out.dump.zones == before.dump.zones && out.dump.zone_list.len() == before.dump.zone_list.len(),
        "a disabled split button split: {:?} then {:?}",
        before.dump.zones,
        out.dump.zones
    );

    let panes = || -> Result<usize> { Ok(server.run(&["list-panes", "-t", "e2e"])?.lines().count()) };
    let out = Run::new(ctx, &server, "tmux").send("").strip_button("split-down").dump_at(step_time(2) + 1.5).run()?;
    expect_buttons(&out, "split-right|split-down|find", "a tmux tab")?;
    ensure!(panes()? == 2, "tmux panes after Split Down: {:?}", server.run(&["list-panes", "-t", "e2e"])?);
    let layout = server.run(&["display-message", "-p", "-t", "e2e", "#{window_layout}"])?;
    ensure!(layout.contains('['), "Split Down didn't split top and bottom: {layout}");
    Ok(Outcome::Pass)
}
