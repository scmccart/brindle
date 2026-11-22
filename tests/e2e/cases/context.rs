//! Program contexts (UAPI.15's `OSC 3008`): the stack in the dump
//! (`--- context …`), the badges shown (`--- badge tab=N …` and `--- badge
//! zone=ID …`), a context outliving `ESC c`, ends and nesting, the tab
//! showing only its active zone's badge, and the privileged badge's yellow
//! warning icon in a capture.
//! The sequences are printed with `printf`, since no runner can rely on
//! `run0` or `systemd-nspawn`.

use super::tabs::{FACE_HEIGHT, FACE_LEFT, FACE_TOP, Strip};
use crate::brindle::{Output, Run, theme};
use crate::capture::{Image, PxRect};
use crate::{Ctx, Outcome, Result, ensure};

/// A plain `$ ` prompt with no shell integration.
const PLAIN: &str = "PS1='$ '; clear\\r";

/// A shell command printing `OSC 3008 ; <body> ST`.
pub(super) fn context(body: &str) -> String {
    format!(r"printf '\033]3008;{body}\033\\'")
}

/// `echo` of `word` split by an empty quote at `at`, so the command line on
/// the screen doesn't show the word its output does.
pub(super) fn echo(word: &str, at: usize) -> String {
    format!("echo {}''{}", &word[..at], &word[at..])
}

/// The `run0` start of the spec's scenario.
pub(super) const RUN0: &str = "start=a1;type=elevate;user=me;targetuser=root";

/// What systemd 259.9's `run0 bash` really sends (captured): no `elevate`,
/// a root `session` that is never ended.
pub(super) const RUN0_SESSION: &str = "start=bdb3ea70355040c9979877f40a12ac0a;user=root;hostname=A8;\
    machineid=0123456789abcdef0123456789abcdef;bootid=fedcba9876543210fedcba9876543210;\
    pid=2947629;pidfdid=2963364;comm=(bash);targetuser=root;sessionid=c6;type=session";

fn expect(out: &Output, contexts: &[&str], badges: &[&str], what: &str) -> Result {
    ensure!(out.dump.wait_timeouts.is_empty(), "{what}: a wait gave up: {:?}; screen {:?}", out.dump.wait_timeouts, out.dump.screen);
    ensure!(out.dump.contexts == contexts, "{what}: contexts {:?}, expected {contexts:?}", out.dump.contexts);
    ensure!(out.dump.badges == badges, "{what}: badges {:?}, expected {badges:?}", out.dump.badges);
    Ok(())
}

/// `w` × `h` logical pixels at `x`,`y` from the window's top-left, in
/// device pixels.
fn window_rect(scale: f32, x: f32, y: f32, w: f32, h: f32) -> PxRect {
    let px = |v: f32| (v * scale).round().max(0.0) as u32;
    PxRect { x0: px(x), y0: px(y), x1: px(x + w), y1: px(y + h) }
}

/// `run0`'s start, then `ESC c`: the context stays, the tab's face shows a
/// warning icon after its title in the `e2e` theme's yellow (ANSI 3, which
/// reads on its background as it is), its target user only in the dump's
/// label and the tooltip, and the face is as tall as ever (nothing yellow
/// under it).
pub fn badge(ctx: &Ctx) -> Result<Outcome> {
    let out = Run::new(ctx, "run0")
        .profile("Shell")
        .send(PLAIN)
        .send(&format!("{}; printf '\\033c'; {}\\r", context(RUN0), echo("elevated-now", 4)))
        .wait_for("elevated-now")
        .capture_at(5.0)
        .dump_at(5.5)
        .run()?;
    expect(
        &out,
        &["id=\"a1\" type=elevate label=\"root\""],
        &["tab=1 kind=privileged label=\"root\" draw=icon"],
        "inside run0, after a reset",
    )?;
    let image: &Image = out.image.as_ref().ok_or("no capture")?;
    let strip = Strip::locate(&out, image)?;
    let scale = out.dump.grid()?.scale;
    let yellow = theme::YELLOW;
    // The icon follows the title (`bash`, about 30px after the face's 12px
    // padding), within the face's height; nothing before the title is
    // yellow, and nothing is red, as the badge was before.
    let face = strip.rect(FACE_LEFT + 30.0, FACE_TOP + 4.0, 80.0, FACE_HEIGHT - 8.0);
    image.expect("the yellow warning icon on the face", face, yellow, (6.0 * scale * scale) as usize)?;
    image.expect_none("yellow before the title", strip.rect(FACE_LEFT + 2.0, FACE_TOP + 4.0, 10.0, FACE_HEIGHT - 8.0), yellow)?;
    image.expect_none("red on the face", face, theme::RED)?;
    image.expect_none("yellow under the face", strip.under_face(0), yellow)?;
    Ok(Outcome::Pass)
}

/// The real `run0` start, a root `session`, badges the tab as privileged,
/// and a session for any other user doesn't.
pub fn run0_session(ctx: &Ctx) -> Result<Outcome> {
    let out = Run::new(ctx, "run0-session")
        .profile("Shell")
        .send(PLAIN)
        .send(&format!("{}; {}\\r", context(RUN0_SESSION), echo("session-set", 4)))
        .wait_for("session-set")
        .dump_at(4.5)
        .run()?;
    expect(
        &out,
        &["id=\"bdb3ea70355040c9979877f40a12ac0a\" type=session label=\"root\""],
        &["tab=1 kind=privileged label=\"root\" draw=icon"],
        "inside run0's root session",
    )?;
    let other = RUN0_SESSION.replace("targetuser=root", "targetuser=nobody");
    let out = Run::new(ctx, "other-session")
        .profile("Shell")
        .send(PLAIN)
        .send(&format!("{}; {}\\r", context(&other), echo("session-set", 4)))
        .wait_for("session-set")
        .dump_at(4.5)
        .run()?;
    expect(&out, &["id=\"bdb3ea70355040c9979877f40a12ac0a\" type=session"], &[], "inside another user's session")?;
    Ok(Outcome::Pass)
}

/// Nesting and ends. A shell inside a container inside an elevation shows
/// the container (the innermost notable context), its long name cut to 24
/// characters; an unknown end changes nothing, and ending the elevation
/// ends everything inside it.
pub fn end(ctx: &Ctx) -> Result<Outcome> {
    let starts = [
        context(RUN0),
        context("start=c1;type=container;container=a-very-long-container-name-indeed"),
        context("start=s1;type=shell"),
    ]
    .join("; ");
    let out = Run::new(ctx, "nested")
        .profile("Shell")
        .send(PLAIN)
        .send(&format!("{starts}; {}\\r", echo("nested-set", 3)))
        .wait_for("nested-set")
        .dump_at(4.5)
        .run()?;
    expect(
        &out,
        &[
            "id=\"a1\" type=elevate label=\"root\"",
            "id=\"c1\" type=container label=\"a-very-long-container-n…\"",
            "id=\"s1\" type=shell",
        ],
        &["tab=1 kind=elsewhere label=\"a-very-long-container-n…\" draw=text"],
        "nested",
    )?;

    let ends = [context("end=zzz"), context("end=a1")].join("; ");
    let out = Run::new(ctx, "ended")
        .profile("Shell")
        .send(PLAIN)
        .send(&format!("{starts}; {ends}; {}\\r", echo("all-ended", 3)))
        .wait_for("all-ended")
        .dump_at(4.5)
        .run()?;
    expect(&out, &[], &[], "after an unknown end and the elevation's")?;
    Ok(Outcome::Pass)
}

/// Two zones, the left inside `run0`, the right (active) in none: the tab
/// face shows no badge, since its active zone has none, while the left
/// zone's header shows the yellow warning icon and the right one's doesn't.
pub fn zones(ctx: &Ctx) -> Result<Outcome> {
    let out = Run::new(ctx, "zones")
        .profile("Shell")
        .send(PLAIN)
        .send(&format!("{}; {}\\r", context(RUN0), echo("left-set", 4)))
        .wait_for("left-set")
        .action("split_zone_right")
        .send(&format!("{}\\r", echo("right-up", 5)))
        .wait_for("right-up")
        .capture_at(6.0)
        .dump_at(6.5)
        .run()?;
    ensure!(out.dump.zones.as_deref() == Some("count=2 active=2"), "zones {:?}", out.dump.zones);
    expect(&out, &[], &["zone=1 kind=privileged label=\"root\" draw=icon"], "two zones, the left elevated")?;
    let image = out.image.as_ref().ok_or("no capture")?;
    let grid = out.dump.grid()?;
    let yellow = theme::YELLOW;
    // The right zone's grid starts a padding below its header, which is
    // 28px tall; the left zone's header is at the same height. Each badge
    // follows its header's title.
    let header_top = grid.y - PADDING - 26.0;
    let left = window_rect(grid.scale, 16.0, header_top, grid.x - 40.0, 24.0);
    image.expect("the left zone's yellow warning icon", left, yellow, (6.0 * grid.scale * grid.scale) as usize)?;
    let right = window_rect(grid.scale, grid.x, header_top, 160.0, 24.0);
    image.expect_none("yellow in the right zone's header", right, yellow)?;
    Ok(Outcome::Pass)
}

/// The test config's terminal padding, in logical pixels.
const PADDING: f32 = 6.0;
