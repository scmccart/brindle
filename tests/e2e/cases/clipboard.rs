//! Right-click paste (`right_click = "paste"`) and the clipboard settings
//! read when they are used. The clipboard is set through OSC 52 from the
//! shell, to a command whose text doesn't show its own output
//! (`echo pasted-$((6*7))` prints `pasted-42`); the mouse is driven with
//! Brindle's `--right-click`, `--click` and `--action select_all`.

use std::path::PathBuf;

use crate::brindle::{Run, config_with};
use crate::{Ctx, Outcome, Result, ensure};

/// What the clipboard holds: a command that prints `pasted-42`.
const CLIP: &str = "echo pasted-$((6*7))";
/// What the pasted command prints, which the typed one doesn't contain.
const PRINTED: &str = "pasted-42";
/// A piece of the clipboard text that is left on the screen wherever it was
/// typed or pasted, and shows no output.
const MARK: &str = "$((6*7))";

const PASTE: &str = "right_click = \"paste\"\n";

/// The command that puts [`CLIP`] on the clipboard through OSC 52, for bash
/// or, on Windows, for PowerShell, with the Enter that runs it.
fn set_clipboard() -> String {
    if cfg!(windows) {
        format!(
            "[Console]::Out.Write(\"$([char]27)]52;c;\" + [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes('{CLIP}')) + \"$([char]7)\")\\r"
        )
    } else {
        format!("printf '\\033]52;c;%s\\007' \"$(printf '%s' '{CLIP}' | base64)\"\\r")
    }
}

/// How many times the clipboard text's marker is on the screen: once for
/// the command that set the clipboard, and once more for each paste.
fn marks(screen: &[String]) -> usize {
    // A long line wraps onto the next row, which the dump keeps apart.
    screen.concat().matches(MARK).count()
}

fn run<'a>(ctx: &'a Ctx, label: &'static str) -> Run<'a> {
    Run::new(ctx, label).prompt().send(&set_clipboard())
}

/// With `right_click = "paste"` and nothing selected, a right-click pastes
/// the clipboard, as the paste key does.
pub fn right_click_paste(ctx: &Ctx) -> Result<Outcome> {
    let out = run(ctx, "run")
        .top(PASTE)
        .right_click("0,0")
        .send("\\r")
        .wait_for(PRINTED)
        .dump_at(7.0)
        .run()?;
    ensure!(out.dump.wait_timeouts.is_empty(), "the paste didn't run: {:?}; screen {:?}", out.dump.wait_timeouts, out.dump.screen);
    ensure!(marks(&out.dump.screen) == 2, "screen {:?}", out.dump.screen);
    Ok(Outcome::Pass)
}

/// The default config: a right-click types nothing. A command typed after it
/// shows that the steps ran.
pub fn right_click_off(ctx: &Ctx) -> Result<Outcome> {
    let out = run(ctx, "run")
        .right_click("0,0")
        .send("\\r")
        .send("echo end-$((1+1))\\r")
        .wait_for("end-2")
        .dump_at(7.0)
        .run()?;
    let text = out.dump.screen.join("\n");
    ensure!(out.dump.wait_timeouts.is_empty(), "the shell didn't answer: {:?}; screen {text:?}", out.dump.wait_timeouts);
    ensure!(!text.contains(PRINTED), "the right-click pasted: {text:?}");
    ensure!(marks(&out.dump.screen) == 1, "screen {:?}", out.dump.screen);
    Ok(Outcome::Pass)
}

/// A right-click over a selection does nothing and leaves it; once a left
/// click clears the selection, the next right-click pastes.
pub fn right_click_selection(ctx: &Ctx) -> Result<Outcome> {
    let over = run(ctx, "over")
        .top(PASTE)
        .action("select_all")
        .right_click("0,0")
        .dump_at(5.5)
        .run()?;
    ensure!(over.dump.selection.is_some(), "the selection is gone; screen {:?}", over.dump.screen);
    ensure!(marks(&over.dump.screen) == 1, "something was pasted over the selection: {:?}", over.dump.screen);

    let cleared = run(ctx, "cleared")
        .top(PASTE)
        .action("select_all")
        .right_click("0,0")
        .click("0,0")
        .right_click("0,0")
        .send("\\r")
        .wait_for(PRINTED)
        .dump_at(9.0)
        .run()?;
    ensure!(cleared.dump.wait_timeouts.is_empty(), "no paste after the click: {:?}; screen {:?}", cleared.dump.wait_timeouts, cleared.dump.screen);
    ensure!(cleared.dump.selection.is_none(), "a selection is left: {:?}", cleared.dump.selection);
    ensure!(marks(&cleared.dump.screen) == 2, "screen {:?}", cleared.dump.screen);
    Ok(Outcome::Pass)
}

/// A shell script in the case directory.
fn script(ctx: &Ctx, name: &str, body: &str) -> Result<PathBuf> {
    let path = ctx.dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}")).map_err(|e| e.to_string())?;
    Ok(path)
}

/// While a program reports the mouse, a right press is reported and nothing
/// is pasted. Shift sends it to Brindle instead, which pastes it when
/// nothing is selected and does nothing over a selection, with no report.
pub fn right_click_mouse_mode(ctx: &Ctx) -> Result<Outcome> {
    // `cat -v` shows the reports the terminal sends as they are echoed.
    let show = script(ctx, "show", "clear\nprintf '\\033[?1000h\\033[?1006h'\nprintf 'mouse-%s\\n' on\nexec cat -v\n")?;
    let out = run(ctx, "run")
        .top(PASTE)
        .send(&format!("sh {}\\r", show.display()))
        .wait_for("mouse-on")
        .right_click("5,1")
        .right_click("shift-5,2")
        .action("select_all")
        .right_click("shift-5,3")
        .dump_at(9.0)
        .run()?;
    let text = out.dump.screen.concat();
    ensure!(out.dump.mode.contains("MOUSE_REPORT_CLICK"), "mouse reporting is off: {}", out.dump.mode);
    ensure!(text.contains("[<2;6;2M"), "the right press wasn't reported: {text:?}");
    ensure!(!text.contains("[<2;6;3") && !text.contains("[<2;6;4"), "a shifted press was reported: {text:?}");
    ensure!(marks(&out.dump.screen) == 1, "want the clipboard text typed once, by the shifted press with nothing selected: {text:?}");
    ensure!(out.dump.selection.is_some(), "the selection is gone; screen {:?}", out.dump.screen);
    Ok(Outcome::Pass)
}

/// A tab open with the default config pastes on a right-click once the
/// config says `right_click = "paste"` and is reloaded.
pub fn right_click_reload(ctx: &Ctx) -> Result<Outcome> {
    let config = ctx.dir.join("config.toml");
    let paste = config_with("e2e", "Shell", PASTE);
    let out = run(ctx, "run")
        // Well after the window opened, before the reload step.
        .call_at(2.0, move |_| std::fs::write(&config, paste).map_err(|e| format!("rewriting the config: {e}")))
        .action("reload_config")
        .right_click("0,0")
        .send("\\r")
        .wait_for(PRINTED)
        .dump_at(8.0)
        .run()?;
    ensure!(out.dump.wait_timeouts.is_empty(), "no paste after the reload: {:?}; screen {:?}", out.dump.wait_timeouts, out.dump.screen);
    Ok(Outcome::Pass)
}
