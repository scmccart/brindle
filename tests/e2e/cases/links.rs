//! Clickable links: ctrl-click opens detected URLs and OSC 8 hyperlinks
//! (through `BRINDLE_LINK_OPENER`, so no browser starts), ctrl-hover
//! underlines them, and mouse-reporting programs don't get those clicks.
//! The mouse is driven with Brindle's `--hover`/`--click` hooks.

use std::path::{Path, PathBuf};

use super::server;
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
    let server = server()?;
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
    let out = Run::new(ctx, &server, "run")
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
    let server = server()?;
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
    let out = Run::new(ctx, &server, "run")
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

/// In a control-mode pane, an OSC 8 link printed before Brindle attached
/// (restored from `capture-pane -e`) and one printed afterwards (live
/// `%output`) both open, as does a detected URL.
pub fn tmux(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let (opener, log) = recorder(ctx)?;
    let before = script(ctx, "before", "printf '\\033]8;;https://example.com/before\\033\\\\restored\\033]8;;\\033\\\\\\n'\n")?;
    let after = script(
        ctx,
        "after",
        "printf '\\033]8;;https://example.com/after\\033\\\\live\\033]8;;\\033\\\\ https://example.com/plain\\n'\n",
    )?;
    // Short relative paths keep the command line from wrapping.
    let dir = ctx.dir.display();
    server.type_line("%0", &format!("cd '{dir}'; clear; sh ./{}", before.file_name().unwrap().to_string_lossy()))?;
    ctx.sleep(0.5);
    // Rows: 0 "restored", 1 prompt + "sh after", 2 "live https://…".
    let out = Run::new(ctx, &server, "run")
        .env("BRINDLE_LINK_OPENER", &opener.display().to_string())
        .send(&format!("sh ./{}\r", after.file_name().unwrap().to_string_lossy()))
        .click("ctrl-2,0")
        .click("ctrl-1,2")
        .click("ctrl-12,2")
        .hover("ctrl-2,0")
        .dump_at(6.0)
        .run()?;
    let want = ["https://example.com/before", "https://example.com/after", "https://example.com/plain"];
    ensure!(opened(&log) == want, "opened {:?}, want {want:?}; screen {:?}", opened(&log), out.dump.screen);
    ensure!(out.dump.link.as_deref() == Some("https://example.com/before"), "hovered link {:?}", out.dump.link);
    Ok(Outcome::Pass)
}
