//! Program progress (ConEmu's `OSC 9 ; 4`): the bars in the dump
//! (`--- progress tab=N …` and `--- progress zone=ID …`), a report split
//! across writes, no notification, the bar gone at the next prompt, the
//! tab taking its most pressing zone's, and the bar's colour in a capture.

use super::Gate;
use super::tabs::{FACE_HEIGHT, FACE_LEFT, FACE_TOP, FACE_WIDTH, Strip};
use crate::brindle::{Output, Run, theme};
use crate::capture::{Image, Rgb};
use crate::{Ctx, Outcome, Result, ensure};

/// A plain `$ ` prompt with no shell integration, so a prompt doesn't drop
/// the progress.
const PLAIN: &str = "PS1='$ '; clear\\r";

/// The `$ ` prompt with Brindle's bash integration, so prompts send OSC 133
/// `A`.
fn integrated(ctx: &Ctx) -> String {
    format!("PS1='$ '; eval \"$({} --shell-integration bash)\"; clear\\r", ctx.bin.display())
}

/// A shell command printing `OSC 9 ; <body> ST`.
fn report(body: &str) -> String {
    format!(r"printf '\033]9;{body}\033\\'")
}

/// `echo` of `word` split by an empty quote at `at`, so the command line on
/// the screen doesn't show the word its output does.
fn echo(word: &str, at: usize) -> String {
    format!("echo {}''{}", &word[..at], &word[at..])
}

fn notifications(ctx: &Ctx) -> Vec<String> {
    let text = std::fs::read_to_string(ctx.dir.join("notify.log")).unwrap_or_default();
    text.lines().map(str::to_string).collect()
}

fn expect_progress(out: &Output, want: &[&str], what: &str) -> Result {
    ensure!(out.dump.wait_timeouts.is_empty(), "{what}: a wait gave up: {:?}; screen {:?}", out.dump.wait_timeouts, out.dump.screen);
    ensure!(out.dump.progress == want, "{what}: progress {:?}, expected {want:?}; screen {:?}", out.dump.progress, out.dump.screen);
    Ok(())
}

/// `ESC ] 9 ; 4 ; 1 ; 3` and, once a file the case creates exists, `0 ST`:
/// the tab's bar is normal at 30%. Notifications are on for every tab
/// (`when = "always"`), and none is logged; the screen shows only the
/// echo after the report.
pub fn report_split(ctx: &Ctx) -> Result<Outcome> {
    let log = ctx.dir.join("notify.log");
    let gate = Gate::new(ctx, "second-half");
    let open = gate.0.clone();
    let command = format!(r"printf '\033]9;4;1;3'; {}; printf '0\033\\'; {}\r", gate.wait(), echo("reported", 2));
    let out = Run::new(ctx, "split")
        .profile("Shell")
        .config("\n[notifications]\nwhen = \"always\"\n")
        .env("BRINDLE_NOTIFY_LOG", log.to_str().unwrap())
        .send(PLAIN)
        .send(&command)
        // The shell waits for the file between the two writes.
        .call_at(5.0, move |_| std::fs::write(&open, b"").map_err(|e| e.to_string()))
        .wait_for("reported")
        .dump_at(8.0)
        .run()?;
    expect_progress(&out, &["tab=1 state=normal percent=30"], "after a split report")?;
    let tail: Vec<&str> = out.dump.screen.iter().rev().take(2).rev().map(String::as_str).collect();
    ensure!(tail == ["reported", "$"], "the report should print nothing; screen ends {tail:?}: {:?}", out.dump.screen);
    let got = notifications(ctx);
    ensure!(got.is_empty(), "a progress report notified: {got:?}");
    Ok(Outcome::Pass)
}

/// With Brindle's bash integration: a program that reported 40% and still
/// runs keeps the bar; once ctrl-c kills it and the shell prints its next
/// prompt (OSC 133 `A`), the bar is gone.
pub fn clears_at_prompt(ctx: &Ctx) -> Result<Outcome> {
    // The program: a report, then a wait for a file nothing creates.
    let running = format!("{}; {}\r", report("4;1;40"), Gate::new(ctx, "never").wait());
    let out = Run::new(ctx, "running")
        .profile("Shell")
        .send(&integrated(ctx))
        .send(&running)
        .dump_at(5.0)
        .run()?;
    expect_progress(&out, &["tab=1 state=normal percent=40"], "while the program runs")?;

    let out = Run::new(ctx, "killed")
        .profile("Shell")
        .send(&integrated(ctx))
        .send(&running)
        .key("ctrl-c")
        .send(&format!("{}\r", echo("prompt-back", 3)))
        .wait_for("prompt-back")
        .dump_at(8.0)
        .run()?;
    expect_progress(&out, &[], "after ctrl-c and the next prompt")?;
    Ok(Outcome::Pass)
}

/// A capture's device rectangle, `w` × `h` logical pixels at `x`,`y` from
/// the window's top-left.
fn window_rect(scale: f32, x: f32, y: f32, w: f32, h: f32) -> crate::capture::PxRect {
    let px = |v: f32| (v * scale).round().max(0.0) as u32;
    crate::capture::PxRect { x0: px(x), y0: px(y), x1: px(x + w), y1: px(y + h) }
}

/// The test config's terminal padding, in logical pixels.
const PADDING: f32 = 6.0;

/// Two zones: the left reports normal at 30%, the right paused at 70%. The
/// tab shows the paused one, each zone's header its own, and the right
/// zone's header bar is yellow (the global theme's ANSI 3) where the left
/// one's isn't.
pub fn zones(ctx: &Ctx) -> Result<Outcome> {
    let out = Run::new(ctx, "zones")
        .profile("Shell")
        .send(PLAIN)
        .send(&format!("{}; {}\r", report("4;1;30"), echo("left-set", 4)))
        .wait_for("left-set")
        .action("split_zone_right")
        .send(&format!("{}; {}\r", report("4;4;70"), echo("right-set", 5)))
        .wait_for("right-set")
        .capture_at(7.0)
        .dump_at(7.5)
        .run()?;
    ensure!(out.dump.zones.as_deref() == Some("count=2 active=2"), "zones {:?}", out.dump.zones);
    expect_progress(
        &out,
        &["tab=1 state=paused percent=70", "zone=2 state=paused percent=70", "zone=1 state=normal percent=30"],
        "two zones",
    )?;
    let image = out.image.as_ref().ok_or("no capture")?;
    let grid = out.dump.grid()?;
    // The right zone's header ends a padding above its grid, its 1px line
    // last; the bar is the 2px above that line. The band takes a little
    // either side, and starts clear of the header's left edge.
    let yellow: Rgb = theme::ANSI[3];
    let band_top = grid.y - PADDING - 6.0;
    let right = window_rect(grid.scale, grid.x + 2.0, band_top, 40.0, 6.0);
    image.expect("the right zone's paused bar", right, yellow, (40.0 * grid.scale * grid.scale) as usize)?;
    // The left zone's header, at the same height: its bar is muted.
    let left = window_rect(grid.scale, 16.0, band_top, 40.0, 6.0);
    image.expect_none("yellow in the left zone's header", left, yellow)?;
    Ok(Outcome::Pass)
}

/// An error at 50%: the tab's bar is the global theme's red (ANSI 1) along
/// the bottom of the face's left half, not its right half, and nothing red
/// shows under the face, which is as tall as ever.
pub fn bar_colour(ctx: &Ctx) -> Result<Outcome> {
    let out = Run::new(ctx, "error")
        .profile("Shell")
        .send(PLAIN)
        .send(&format!("{}; {}\r", report("4;2;50"), echo("error-set", 5)))
        .wait_for("error-set")
        .capture_at(5.0)
        .dump_at(5.5)
        .run()?;
    expect_progress(&out, &["tab=1 state=error percent=50"], "an error")?;
    let image: &Image = out.image.as_ref().ok_or("no capture")?;
    let strip = Strip::locate(&out, image)?;
    let scale = out.dump.grid()?.scale;
    let red = theme::RED;
    // The bottom few rows of the face. The bar is inset by the face's
    // rounded corner (6px) and is half of what is left long: from 6px to
    // 40px of the 80px face.
    let bottom = FACE_TOP + FACE_HEIGHT - 5.0;
    let filled = strip.rect(FACE_LEFT + 10.0, bottom, 24.0, 5.0);
    image.expect("the error bar", filled, red, (24.0 * 1.5 * scale * scale) as usize)?;
    let empty = strip.rect(FACE_LEFT + 46.0, bottom, FACE_WIDTH - 56.0, 5.0);
    image.expect_none("red past the error bar's 50%", empty, red)?;
    image.expect_none("red under the face", strip.under_face(0), red)?;
    Ok(Outcome::Pass)
}
