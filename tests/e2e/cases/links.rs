//! Clickable links: ctrl-click opens detected URLs and OSC 8 hyperlinks
//! (through `BRINDLE_LINK_OPENER`, so no browser starts), ctrl-hover
//! underlines them, and mouse-reporting programs don't get those clicks. A
//! link that isn't opened is a notice on the window's config line, which
//! goes after its lifetime or when dismissed. The mouse is driven with
//! Brindle's `--hover`/`--click` hooks. The `refused`, `opener_fails` and
//! notice cases run on Windows too.

use std::path::{Path, PathBuf};

use crate::brindle::{Run, theme};
use crate::capture::Cells;
use crate::{Ctx, Outcome, Result, ensure};

/// A shell script in the case directory.
fn script(ctx: &Ctx, name: &str, body: &str) -> Result<PathBuf> {
    let path = ctx.dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}")).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    Ok(path)
}

/// An opener that appends each link it is given to `opened.txt`.
fn recorder(ctx: &Ctx) -> Result<(PathBuf, PathBuf)> {
    let log = ctx.dir.join("opened.txt");
    let opener = script(ctx, "opener", &format!("printf '%s\\n' \"$1\" >> '{}'\n", log.display()))?;
    Ok((opener, log))
}

fn opened(log: &Path) -> Vec<String> {
    std::fs::read_to_string(log).unwrap_or_default().lines().map(str::to_string).collect()
}

const LONG_TAIL: usize = 140;

fn long_url() -> String {
    format!("https://example.com/{}/end", "a".repeat(LONG_TAIL))
}

/// In a local shell: a plain URL with a trailing full stop, an OSC 8 link,
/// and a URL long enough to wrap open on ctrl-click; a click without ctrl
/// opens nothing; ctrl-hover reports the link and underlines its cells.
pub fn local_click(ctx: &Ctx) -> Result<Outcome> {
    let (opener, log) = recorder(ctx)?;
    let show = script(
        ctx,
        "show",
        &format!(
            "clear\n\
             printf 'see https://example.com/plain.\\n'\n\
             printf '\\033]8;;https://example.com/osc8\\033\\\\go  here\\033]8;;\\033\\\\\\n'\n\
             printf '\\033]8;;https://example.com/other\\033\\\\a  b\\033]8;;\\033\\\\\\n'\n\
             printf '%s\\n' '{}'\n",
            long_url()
        ),
    )?;
    let out = Run::new(ctx, "run")
        .profile("Shell")
        .env("BRINDLE_LINK_OPENER", &opener.display().to_string())
        .send(&format!("sh {}\r", show.display()))
        .click("ctrl-10,0")
        .click("ctrl-0,1")
        .click("ctrl-5,4")
        .click("10,0")
        .hover("ctrl-0,1")
        // Late in the hover step, so the repaint has landed even under load.
        .capture_at(6.9)
        .dump_at(7.2)
        .run()?;

    let screen = &out.dump.screen;
    ensure!(screen.first().map(String::as_str) == Some("see https://example.com/plain."), "screen {screen:?}");
    let (cols, _) = out.dump.size.ok_or("no size")?;
    ensure!((cols as usize) < long_url().len(), "{cols} columns: the long URL doesn't wrap");

    let want = ["https://example.com/plain".to_string(), "https://example.com/osc8".to_string(), long_url()];
    ensure!(opened(&log) == want, "opened {:?}, want {want:?}", opened(&log));
    ensure!(
        out.dump.link.as_deref() == Some("https://example.com/osc8"),
        "hovered link {:?}",
        out.dump.link
    );

    // The hovered link's blank cells (between "go" and "here") get an
    // underline; the same gap in the other, unhovered link doesn't.
    let image = out.image.as_ref().ok_or("no capture")?;
    let cells = Cells::from_pane(out.dump.grid()?, 0, 0);
    let gap = cells.rect(2, 1, 2, 1);
    image.expect("underline under the hovered link", gap, theme::FOREGROUND, ((gap.x1 - gap.x0) * 3 / 4) as usize)?;
    image.expect_none("no underline under another link", cells.rect(1, 2, 2, 1), theme::FOREGROUND)?;
    Ok(Outcome::Pass)
}

/// With mouse reporting on, a ctrl-click on a link opens it and reports
/// nothing, while clicks elsewhere (with or without ctrl) are reported.
pub fn mouse_mode(ctx: &Ctx) -> Result<Outcome> {
    let (opener, log) = recorder(ctx)?;
    // `cat -v` shows the reports the terminal sends as they are echoed.
    let show = script(
        ctx,
        "show",
        "clear\n\
         printf 'https://example.com/mouse\\n\\n\\n'\n\
         printf '\\033[?1000h\\033[?1006h'\n\
         exec cat -v\n",
    )?;
    let out = Run::new(ctx, "run")
        .profile("Shell")
        .env("BRINDLE_LINK_OPENER", &opener.display().to_string())
        .send(&format!("sh {}\r", show.display()))
        .click("ctrl-5,0")
        .click("5,1")
        .click("ctrl-5,2")
        .dump_at(5.5)
        .run()?;

    ensure!(out.dump.mode.contains("MOUSE_REPORT_CLICK"), "mouse reporting is off: {}", out.dump.mode);
    ensure!(opened(&log) == ["https://example.com/mouse"], "opened {:?}", opened(&log));
    let text = out.dump.screen.join("\n");
    ensure!(text.contains("[<0;6;2M"), "the plain click wasn't reported: {text:?}");
    ensure!(text.contains("[<16;6;3M"), "the ctrl-click off the link wasn't reported: {text:?}");
    ensure!(!text.contains(";6;1M") && !text.contains(";6;1m"), "the link click was reported: {text:?}");
    Ok(Outcome::Pass)
}

/// `text` without its whitespace, so a line the terminal wrapped reads as
/// it was written.
fn squeezed(text: &str) -> String {
    text.chars().filter(|c| !c.is_whitespace()).collect()
}

/// How long a notice shows on the window's config line
/// (`workspace::notices::NOTICE_LIFETIME`), on Brindle's own clock, which
/// `BRINDLE_E2E_SLOW` doesn't stretch.
const NOTICE_LIFETIME: f64 = 10.0;

/// When a notice case dumps, unscaled, after its last step: well within
/// [`NOTICE_LIFETIME`] of its clicks, and late enough that a slow first
/// prompt (`Run::prompt`, a `--wait-for` on Windows) holding the steps back
/// still leaves the last step before the dump (`Run::dump_after_last_step`).
const NOTICE_SHOWN: f64 = 4.0;

/// A link detected in plain text that Brindle refuses on every platform: a
/// `file:` link to another host.
const OTHER_HOST: &str = "file://elsewhere.invalid/x.exe";

/// What the config line says of [`OTHER_HOST`].
const OTHER_HOST_SAYS: &str = "couldn't open file://elsewhere.invalid/x.exe: it names another host (elsewhere.invalid)";

/// Writes `lines` (escapes and all) to `name` in the case directory, after a
/// clear screen and a cursor home, and gives the command that prints the
/// file in the tab's shell, so the lines start at the top-left cell: `cat`
/// in bash, `[Console]::Out.Write` in PowerShell. Nothing is typed but the
/// file's path, so `--send` rewrites none of the escapes.
fn shows(ctx: &Ctx, name: &str, lines: &[&str]) -> Result<String> {
    let path = ctx.dir.join(name);
    std::fs::write(&path, format!("\x1b[2J\x1b[H{}\r\n", lines.join("\r\n"))).map_err(|e| e.to_string())?;
    let path = path.display().to_string();
    Ok(if cfg!(windows) {
        format!("[Console]::Out.Write([IO.File]::ReadAllText('{}'))\r", path.replace('\\', "/"))
    } else {
        format!("cat '{path}'\r")
    })
}

/// The config line, which a case expects to show.
fn config_line(out: &crate::brindle::Output) -> Result<&str> {
    out.dump.config.as_deref().ok_or_else(|| format!("no --- config line; screen {:?}", out.dump.screen))
}

/// The screen holds none of Brindle's notices: they are the config line's.
fn screen_untouched(out: &crate::brindle::Output) -> Result {
    let text = out.dump.screen.join("\n");
    ensure!(!squeezed(&text).contains(&squeezed("couldn't open")), "a notice was written into the terminal: {text:?}");
    Ok(())
}

/// Links Brindle doesn't open: an OSC 8 hyperlink whose scheme isn't one it
/// detects, and one to a `file:` link on another host. A ctrl-click on each
/// says why on the window's config line, naming the link, and writes
/// nothing into the terminal. The opener is never started: it doesn't
/// exist, and nothing says it couldn't run.
pub fn refused(ctx: &Ctx) -> Result<Outcome> {
    let missing = ctx.dir.join("no-such-opener");
    let print = shows(
        ctx,
        "links.txt",
        &[
            "\x1b]8;;ms-settings:privacy\x1b\\[settings]\x1b]8;;\x1b\\",
            "\x1b]8;;file://elsewhere.invalid/x.exe\x1b\\[tool]\x1b]8;;\x1b\\",
        ],
    )?;
    let out = Run::new(ctx, "run")
        .env("BRINDLE_LINK_OPENER", &missing.display().to_string())
        .prompt()
        .send(&print)
        .wait_for("[tool]")
        .click("ctrl-2,0")
        .click("ctrl-2,1")
        .dump_after_last_step(NOTICE_SHOWN)
        .run()?;
    ensure!(out.dump.wait_timeouts.is_empty(), "waits that gave up: {:?}; screen {:?}", out.dump.wait_timeouts, out.dump.screen);
    let line = config_line(&out)?;
    for says in [
        "couldn't open ms-settings:privacy: its scheme (ms-settings) isn't one Brindle opens",
        OTHER_HOST_SAYS,
    ] {
        ensure!(line.contains(says), "the config line doesn't say {says:?}: {line:?}");
    }
    ensure!(!line.contains("no-such-opener"), "the opener was started: {line:?}");
    screen_untouched(&out)?;
    Ok(Outcome::Pass)
}

/// An opener that can't run: a ctrl-click on a link says on the config line
/// that it couldn't be opened, with the link and the cause.
pub fn opener_fails(ctx: &Ctx) -> Result<Outcome> {
    let missing = ctx.dir.join("no-such-opener");
    let print = shows(ctx, "link.txt", &["see https://example.com/fails"])?;
    let out = Run::new(ctx, "run")
        .env("BRINDLE_LINK_OPENER", &missing.display().to_string())
        .prompt()
        .send(&print)
        .wait_for("example.com/fails")
        .click("ctrl-10,0")
        .dump_after_last_step(NOTICE_SHOWN)
        .run()?;
    ensure!(out.dump.wait_timeouts.is_empty(), "waits that gave up: {:?}; screen {:?}", out.dump.wait_timeouts, out.dump.screen);
    let line = config_line(&out)?;
    ensure!(line.contains("couldn't open https://example.com/fails"), "the config line doesn't name the link: {line:?}");
    ensure!(line.contains("no-such-opener"), "the config line doesn't name the opener: {line:?}");
    screen_untouched(&out)?;
    Ok(Outcome::Pass)
}

/// A run that prints [`OTHER_HOST`] at the top of the tab and ctrl-clicks
/// it, as its last step.
fn click_other_host<'a>(ctx: &'a Ctx, label: &'static str) -> Result<Run<'a>> {
    let print = shows(ctx, "other-host.txt", &[OTHER_HOST])?;
    Ok(Run::new(ctx, label).prompt().send(&print).wait_for(OTHER_HOST).click("ctrl-2,0"))
}

/// A notice goes by itself once [`NOTICE_LIFETIME`] has passed: the run
/// clicks a refused link as `links-notice-dismissed`'s first run does, and
/// dumps after the notice's lifetime. That the click was refused is in the
/// run's log.
pub fn notice_expires(ctx: &Ctx) -> Result<Outcome> {
    let out = click_other_host(ctx, "run")?
        // `NOTICE_LIFETIME` after the click, unscaled, with the room a slow
        // first prompt holds the click back by.
        .dump_after_last_step(NOTICE_LIFETIME + NOTICE_SHOWN)
        .run()?;
    ensure!(out.dump.wait_timeouts.is_empty(), "waits that gave up: {:?}; screen {:?}", out.dump.wait_timeouts, out.dump.screen);
    let log = std::fs::read_to_string(ctx.dir.join("run.log")).map_err(|e| format!("run.log: {e}"))?;
    ensure!(log.contains(OTHER_HOST_SAYS), "the click wasn't refused (run.log doesn't say {OTHER_HOST_SAYS:?})");
    ensure!(
        out.dump.config.as_deref().is_none_or(|line| !line.contains("couldn't open")),
        "the notice outlived {NOTICE_LIFETIME} s: {:?}",
        out.dump.config
    );
    screen_untouched(&out)?;
    Ok(Outcome::Pass)
}

/// `dismiss_notices` drops a link's notice and leaves a config finding (an
/// unknown theme) on the config line. The same run without it keeps both.
pub fn notice_dismissed(ctx: &Ctx) -> Result<Outcome> {
    const FINDING: &str = "unknown theme \"no-such-theme\"";
    let kept = click_other_host(ctx, "kept")?.initial_theme("no-such-theme").dump_after_last_step(NOTICE_SHOWN).run()?;
    ensure!(kept.dump.wait_timeouts.is_empty(), "kept: waits that gave up: {:?}", kept.dump.wait_timeouts);
    let line = config_line(&kept)?;
    ensure!(line.contains(FINDING) && line.contains(OTHER_HOST_SAYS), "kept: the config line {line:?}");

    let dismissed = click_other_host(ctx, "dismissed")?
        .initial_theme("no-such-theme")
        .action("dismiss_notices")
        .dump_after_last_step(NOTICE_SHOWN)
        .run()?;
    ensure!(dismissed.dump.wait_timeouts.is_empty(), "dismissed: waits that gave up: {:?}", dismissed.dump.wait_timeouts);
    let line = config_line(&dismissed)?;
    ensure!(line.contains(FINDING), "dismissing took the config finding: {line:?}");
    ensure!(!line.contains("couldn't open"), "the notice is still shown: {line:?}");
    screen_untouched(&dismissed)?;
    Ok(Outcome::Pass)
}
