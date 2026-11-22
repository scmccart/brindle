//! Right-click paste (`right_click = "paste"`) and the clipboard settings
//! read when they are used, and the settings dialog's Clipboard section that
//! writes them. The clipboard is set through OSC 52 from the shell, to a
//! command whose text doesn't show its own output (`echo pasted-$((6*7))`
//! prints `pasted-42`); the mouse is driven with Brindle's `--right-click`,
//! `--click` and `--action select_all`.
//!
//! Then the clipboard across the process boundary: the pane's own shell is
//! "the other application", a separate process (and on Linux a separate X
//! client) that reads or sets the clipboard with `xclip` (`Get-Clipboard` and
//! `Set-Clipboard` on Windows). Each such case is one function for both
//! platforms, whose shell commands come from the helpers below; the Linux
//! ones skip without `xclip` ([`xclip_skip`]).

use std::path::PathBuf;

use super::step_time;
use crate::brindle::{Output, Run, config_with};
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

/// How long after the last step the dumps that check the mouse label are
/// taken, in seconds and not stretched: the label shows for about 800 ms
/// and then fades for 200 ms (`terminal/feedback.rs`), so a dump 0.3 s after
/// the click is inside its shown phase. Brindle's own limit, so it stays
/// unscaled (`testing R5`).
const LABEL_SOON: f64 = 0.3;

/// Takes the dump while the label of a mouse copy or paste shows, when the
/// run's last step is the mouse action. On Windows PowerShell's start is
/// waited for first (`prompt`), which delays every later step by an unknown
/// time, so the click's time isn't known: the dump is taken at `late` and the
/// label isn't checked there ([`expect_label`]).
fn dump_while_label_shows(run: Run<'_>, late: f64) -> Run<'_> {
    if cfg!(windows) { run.dump_at(late) } else { run.dump_after_last_step(LABEL_SOON) }
}

/// The label's kind in the dump, where [`dump_while_label_shows`] dumped
/// while it showed.
fn expect_label(out: &Output, kind: &str) -> Result {
    if !cfg!(windows) {
        ensure!(
            out.dump.feedback.as_deref() == Some(kind),
            "the label is {:?}, want {kind:?}; screen {:?}",
            out.dump.feedback,
            out.dump.screen
        );
    }
    Ok(())
}

/// With `right_click = "paste"` and nothing selected, a right-click pastes
/// the clipboard, as the paste key does, and says so.
pub fn right_click_paste(ctx: &Ctx) -> Result<Outcome> {
    let out = dump_while_label_shows(run(ctx, "run").top(PASTE).right_click("0,0"), 7.0).run()?;
    ensure!(marks(&out.dump.screen) == 2, "screen {:?}", out.dump.screen);
    expect_label(&out, "pasted")?;
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

/// With `right_click = "paste"`, a right-click over a selection clears it
/// and pastes, whether `copy_on_select` is off or on (the selection comes
/// from `select_all`, which copies nothing), and says so.
pub fn right_click_selection(ctx: &Ctx) -> Result<Outcome> {
    for (label, top) in [("off", PASTE.to_string()), ("on", format!("{PASTE}copy_on_select = true\n"))] {
        let out = dump_while_label_shows(run(ctx, label).top(&top).action("select_all").right_click("0,0"), 10.0).run()?;
        ensure!(out.dump.selection.is_none(), "copy_on_select {label}: the selection is left: {:?}", out.dump.selection);
        ensure!(marks(&out.dump.screen) == 2, "copy_on_select {label}: nothing was pasted over the selection: {:?}", out.dump.screen);
        expect_label(&out, "pasted")?;
    }
    Ok(Outcome::Pass)
}

/// With nothing on the clipboard, a right-click over a selection pastes
/// nothing and leaves the selection, and says there was nothing to paste.
/// The shell empties the clipboard first with `xclip`, so Linux only.
pub fn right_click_empty(ctx: &Ctx) -> Result<Outcome> {
    if let Some(skip) = xclip_skip()? {
        return Ok(skip);
    }
    let run = Run::new(ctx, "run")
        .top(PASTE)
        .send(&format!("{} -selection clipboard -i /dev/null; echo clr-''ok\\r", xclip(ctx)))
        .wait_for("clr-ok")
        .action("select_all")
        .right_click("0,0");
    let out = dump_while_label_shows(run, 6.0).run()?;
    waited(&out)?;
    ensure!(out.dump.selection.is_some(), "the selection is gone; screen {:?}", out.dump.screen);
    ensure!(marks(&out.dump.screen) == 0, "something was pasted: {:?}", out.dump.screen);
    expect_label(&out, "nothing-to-paste")?;
    kill_xclip(ctx);
    Ok(Outcome::Pass)
}

/// A drag selection with `copy_on_select` on copies, and the label says
/// copied; with it off the drag copies nothing and shows no label. Linux
/// only: the drag is the run's last step, which a Windows run can't time.
pub fn select_label(ctx: &Ctx) -> Result<Outcome> {
    for (label, top, want) in [("on", "copy_on_select = true\n", Some("copied")), ("off", "", None)] {
        let run = Run::new(ctx, label).top(top).send(&show(&[(10, "copy-", "me")])).wait_for("copy-me");
        let out = select_rows(run, 9, 9).dump_after_last_step(LABEL_SOON).run()?;
        ensure!(out.dump.wait_timeouts.is_empty() && out.dump.wait_pending.is_empty(), "copy_on_select {label}: a wait gave up: {:?} {:?}", out.dump.wait_timeouts, out.dump.wait_pending);
        ensure!(out.dump.selection.is_some(), "copy_on_select {label}: nothing is selected; screen {:?}", out.dump.screen);
        ensure!(out.dump.feedback.as_deref() == want, "copy_on_select {label}: the label is {:?}, want {want:?}", out.dump.feedback);
    }
    Ok(Outcome::Pass)
}

/// A shell script in the case directory.
fn script(ctx: &Ctx, name: &str, body: &str) -> Result<PathBuf> {
    let path = ctx.dir.join(name);
    std::fs::write(&path, format!("#!/bin/sh\n{body}")).map_err(|e| e.to_string())?;
    Ok(path)
}

/// While a program reports the mouse, a right press is reported and nothing
/// is pasted. Shift sends it to Brindle instead, which pastes it, with
/// nothing selected and over a selection alike (clearing it), with no report.
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
        .dump_after_last_step(LABEL_SOON)
        .run()?;
    let text = out.dump.screen.concat();
    ensure!(out.dump.mode.contains("MOUSE_REPORT_CLICK"), "mouse reporting is off: {}", out.dump.mode);
    ensure!(text.contains("[<2;6;2M"), "the right press wasn't reported: {text:?}");
    ensure!(!text.contains("[<2;6;3") && !text.contains("[<2;6;4"), "a shifted press was reported: {text:?}");
    ensure!(marks(&out.dump.screen) == 2, "want the clipboard text typed twice, by the shifted presses: {text:?}");
    ensure!(out.dump.selection.is_none(), "the shifted press over the selection left it: {:?}", out.dump.selection);
    expect_label(&out, "pasted")?;
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

/// A comment of the user's in the config, which writes from the settings
/// dialog keep.
const COMMENT: &str = "# my clipboard settings\n";

fn config_text(ctx: &Ctx) -> Result<String> {
    std::fs::read_to_string(ctx.dir.join("config.toml")).map_err(|e| e.to_string())
}

/// Opens the settings dialog on its Clipboard section, the last, where focus
/// is on the first switch, "Copy on select". Two steps.
fn open_section(run: Run) -> Run {
    run.top(COMMENT).action("open_settings").action("settings_section_last")
}

/// The settings dialog's Clipboard section shows the config's values; tab,
/// space and enter turn on "Right-click pastes" and apply it, which writes
/// `right_click = "paste"` alone, keeps the file's comments and closes the
/// dialog, and a right-click then pastes in the open tab. Escape after
/// turning both switches on writes nothing.
pub fn settings_section(ctx: &Ctx) -> Result<Outcome> {
    let original = config_with("e2e", "Shell", COMMENT);

    let out = open_section(Run::new(ctx, "open")).dump_at(step_time(2) + 0.8).run()?;
    let settings = out.dump.settings.as_deref().unwrap_or("");
    ensure!(
        settings.starts_with("section=Clipboard mode=form copy_on_select=false right_click=none field=\"Copy on select\""),
        "dialog state on the Clipboard section: {settings:?}"
    );
    ensure!(out.dump.focus_stop.as_deref() == Some("Copy on select"), "focus stop: {:?}", out.dump.focus_stop);

    // Space flips the focused switch, and tab goes on to the next.
    let both = |label: &'static str| open_section(Run::new(ctx, label)).key("space").key("tab").key("space");
    let out = both("both").dump_at(step_time(5) + 0.8).run()?;
    let settings = out.dump.settings.as_deref().unwrap_or("");
    ensure!(
        settings.contains("copy_on_select=true right_click=paste field=\"Right-click pastes\" can_apply=true"),
        "dialog state with both switches on: {settings:?}"
    );
    ensure!(config_text(ctx)? == original, "turning the switches on changed the config");

    let out = both("escape").key("escape").dump_at(step_time(6) + 0.8).run()?;
    ensure!(out.dump.settings.is_none(), "the dialog is still open after escape: {:?}", out.dump.settings);
    ensure!(out.dump.focus.as_deref() == Some("terminal"), "focus after escape: {:?}", out.dump.focus);
    ensure!(config_text(ctx)? == original, "escape changed the config:\n{}", config_text(ctx)?);

    // With the clipboard set: the Right-click pastes switch, applied with
    // enter, then a right-click in the tab.
    let out = open_section(run(ctx, "apply"))
        .key("tab")
        .key("space")
        .key("enter")
        .right_click("0,0")
        .send("\\r")
        .wait_for(PRINTED)
        .dump_at(step_time(9) + 2.0)
        .run()?;
    ensure!(out.dump.settings.is_none(), "the dialog is still open after enter: {:?}", out.dump.settings);
    ensure!(out.dump.focus.as_deref() == Some("terminal"), "focus after apply: {:?}", out.dump.focus);
    let want = original.replacen("padding = 6\n", "padding = 6\nright_click = \"paste\"\n", 1);
    ensure!(config_text(ctx)? == want, "config after apply:\n{}", config_text(ctx)?);
    ensure!(
        out.dump.wait_timeouts.is_empty(),
        "no paste after apply: {:?}; screen {:?}",
        out.dump.wait_timeouts,
        out.dump.screen
    );
    ensure!(marks(&out.dump.screen) == 2, "screen {:?}", out.dump.screen);
    Ok(Outcome::Pass)
}

// ---- across the process boundary ---------------------------------------------------

/// What a case that needs `tool` does where it isn't installed: skips, or
/// fails when CI says the tool must be there (`required`), so a runner
/// without it can't pass by skipping.
fn missing_tool(tool: &str, found: bool, required: bool) -> Result<Option<Outcome>> {
    if found {
        return Ok(None);
    }
    let why = format!("{tool} isn't installed");
    if required { Err(format!("{why} (BRINDLE_E2E_CLIPBOARD_REQUIRED=1)")) } else { Ok(Some(Outcome::Skip(why))) }
}

/// The skip, or the failure under `BRINDLE_E2E_CLIPBOARD_REQUIRED=1`, for a
/// case that runs `xclip`. Windows needs none: its cases use PowerShell's
/// own clipboard commands.
fn xclip_skip() -> Result<Option<Outcome>> {
    if cfg!(windows) {
        return Ok(None);
    }
    let found = std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join("xclip").is_file()));
    missing_tool("xclip", found, std::env::var("BRINDLE_E2E_CLIPBOARD_REQUIRED").as_deref() == Ok("1"))
}

/// `xclip` for the case's display, so it never reaches another's.
fn xclip(ctx: &Ctx) -> String {
    format!("xclip -display {}", ctx.display)
}

/// A `xclip` that set a clipboard stays as its owner until another program
/// takes it over, so the next case would find it. This ends the case's
/// (the display's), once the case is done with the clipboard.
fn kill_xclip(ctx: &Ctx) {
    if cfg!(windows) {
        return;
    }
    std::process::Command::new("pkill")
        .args(["-f", &format!("^xclip -display {}", ctx.display)])
        .stdin(std::process::Stdio::null())
        .status()
        .ok();
}

/// The step that prints the lines `(row, head, tail)` at those rows (from 1)
/// of the screen, from column 3. The text is `head` then `tail`, typed apart
/// so the command line doesn't show it, and printed at fixed places so a drag
/// can select it whatever the prompt is. The cursor ends on the line after
/// the last.
fn show(lines: &[(u16, &str, &str)]) -> String {
    if cfg!(windows) {
        let parts: Vec<String> =
            lines.iter().map(|(row, head, tail)| format!("\"$([char]27)[{row};3H\" + '{head}' + '{tail}'")).collect();
        format!("[Console]::Write({} + \"`n\")\\r", parts.join(" + "))
    } else {
        let formats: String = lines.iter().map(|(row, ..)| format!("\\033[{row};3H%s%s")).collect();
        let args: Vec<String> = lines.iter().map(|(_, head, tail)| format!("'{head}' '{tail}'")).collect();
        format!("printf '{formats}\\012' {}\\r", args.join(" "))
    }
}

/// Drags over the rows `first` to `last` (from 0), from the first column to
/// well past the text: whichever half of the first cell the pointer is
/// taken to hit, the text's own cells are all selected, and the blank ones
/// after it are not part of the copy.
fn select_rows(run: Run<'_>, first: u16, last: u16) -> Run<'_> {
    run.press(&format!("0,{first}")).hover(&format!("20,{last}")).release(&format!("20,{last}"))
}

/// The step that prints the clipboard as `clip=[...]`.
fn read_clipboard(ctx: &Ctx) -> String {
    if cfg!(windows) {
        "'cl' + 'ip=[' + (Get-Clipboard -Raw) + ']'\\r".to_string()
    } else {
        format!("echo \"cl\"\"ip=[$({} -o -selection clipboard)]\"\\r", xclip(ctx))
    }
}

/// The step that prints the bytes of the clipboard's text without its
/// spaces, as `bytes=[...]`: hexadecimal on Linux (`od`), decimal on Windows.
fn read_clipboard_bytes(ctx: &Ctx) -> String {
    if cfg!(windows) {
        "'by' + 'tes=[' + (([int[]][char[]]((Get-Clipboard -Raw) -replace ' ', '')) -join ' ') + ']'\\r".to_string()
    } else {
        format!("echo \"by\"\"tes=[$({} -o -selection clipboard | tr -d ' ' | od -An -tx1)]\"\\r", xclip(ctx))
    }
}

/// `bytes` as [`read_clipboard_bytes`] prints them, one space apart.
fn byte_listing(bytes: &[u8]) -> String {
    let each: Vec<String> = bytes.iter().map(|b| if cfg!(windows) { b.to_string() } else { format!("{b:02x}") }).collect();
    each.join(" ")
}

/// What is between `key` and the next `]` on the screen (its rows joined, so
/// a line that wrapped is whole, with no space added at the join), with the spaces round it and between
/// words collapsed.
fn bracketed(screen: &[String], key: &str) -> Option<String> {
    let text = screen.concat();
    let start = text.find(key)? + key.len();
    let end = start + text[start..].find(']')?;
    Some(text[start..end].split_whitespace().collect::<Vec<_>>().join(" "))
}

/// The step that makes the pane's shell set the clipboard to a command that
/// prints `from-42` without showing it, and print `set-ok`.
fn set_clipboard_command(ctx: &Ctx) -> String {
    if cfg!(windows) {
        "Set-Clipboard -Value \"'from-' + (6*7)\"; 'set-' + 'ok'\\r".to_string()
    } else {
        format!("printf 'echo from-$((6*7))' | {} -selection clipboard; echo set-''ok\\r", xclip(ctx))
    }
}

/// When the dump is taken for a run of `steps` steps: a time that every wait
/// is over by, which only caps the run (it quits at the dump). PowerShell's
/// start adds seconds on Windows.
fn dump_limit(steps: usize) -> f64 {
    steps as f64 + 4.0 + if cfg!(windows) { 10.0 } else { 0.0 }
}

/// Fails with the waits that gave up, if any.
fn waited(out: &Output) -> Result {
    ensure!(
        out.dump.wait_timeouts.is_empty() && out.dump.wait_pending.is_empty(),
        "a wait gave up or didn't finish: {:?} {:?}; screen {:?}",
        out.dump.wait_timeouts,
        out.dump.wait_pending,
        out.dump.screen
    );
    Ok(())
}

/// A selection copied with the Copy action is on the clipboard for another
/// program to read.
pub fn copy_to_app(ctx: &Ctx) -> Result<Outcome> {
    if let Some(skip) = xclip_skip()? {
        return Ok(skip);
    }
    let run = Run::new(ctx, "run").prompt().send(&show(&[(10, "copy-", "me")])).wait_for("copy-me");
    let out = select_rows(run, 9, 9)
        .action("copy")
        .send(&read_clipboard(ctx))
        .wait_for("clip=[")
        .dump_at(dump_limit(9))
        .run()?;
    waited(&out)?;
    let clip = bracketed(&out.dump.screen, "clip=[");
    ensure!(clip.as_deref() == Some("copy-me"), "the other program read {clip:?}; screen {:?}", out.dump.screen);
    Ok(Outcome::Pass)
}

/// Text another program put on the clipboard pastes into the tab.
pub fn paste_from_app(ctx: &Ctx) -> Result<Outcome> {
    if let Some(skip) = xclip_skip()? {
        return Ok(skip);
    }
    let out = Run::new(ctx, "run")
        .prompt()
        .send(&set_clipboard_command(ctx))
        .wait_for("set-ok")
        .action("paste")
        .send("\\r")
        .wait_for("from-42")
        .dump_at(dump_limit(6))
        .run()?;
    waited(&out)?;
    kill_xclip(ctx);
    Ok(Outcome::Pass)
}

/// A program's OSC 52 write reaches the clipboard another program reads.
pub fn osc52_to_app(ctx: &Ctx) -> Result<Outcome> {
    if let Some(skip) = xclip_skip()? {
        return Ok(skip);
    }
    let out = run(ctx, "run").send(&read_clipboard(ctx)).wait_for("clip=[").dump_at(dump_limit(4)).run()?;
    waited(&out)?;
    let clip = bracketed(&out.dump.screen, "clip=[");
    ensure!(clip.as_deref() == Some(CLIP), "the other program read {clip:?}, want {CLIP:?}; screen {:?}", out.dump.screen);
    Ok(Outcome::Pass)
}

/// A drag selection is the primary selection another program reads
/// (`xclip -o -selection primary`), and text another program made the
/// primary selection pastes with the primary paste (`--action paste_primary`;
/// the debug hooks have no middle button). Linux only: Windows has no
/// primary selection of the system's.
pub fn primary_both_ways(ctx: &Ctx) -> Result<Outcome> {
    if let Some(skip) = xclip_skip()? {
        return Ok(skip);
    }
    let run = Run::new(ctx, "to-app").send(&show(&[(10, "copy-", "me")])).wait_for("copy-me");
    let out = select_rows(run, 9, 9)
        .send(&format!("echo \"pr\"\"im=[$({} -o -selection primary)]\"\\r", xclip(ctx)))
        .wait_for("prim=[")
        .dump_at(dump_limit(6))
        .run()?;
    waited(&out)?;
    let primary = bracketed(&out.dump.screen, "prim=[");
    ensure!(primary.as_deref() == Some("copy-me"), "the other program read {primary:?}; screen {:?}", out.dump.screen);

    let out = Run::new(ctx, "from-app")
        .send(&format!("printf 'echo prim-$((6*7))' | {} -selection primary; echo set-''ok\\r", xclip(ctx)))
        .wait_for("set-ok")
        .action("paste_primary")
        .send("\\r")
        .wait_for("prim-42")
        .dump_at(dump_limit(6))
        .run()?;
    waited(&out)?;
    kill_xclip(ctx);
    Ok(Outcome::Pass)
}

/// A 1x1 PNG, base64.
const PNG: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";

/// An image on the clipboard pastes nothing: what is typed after the paste
/// is the whole of the command line.
pub fn image_pastes_nothing(ctx: &Ctx) -> Result<Outcome> {
    if let Some(skip) = xclip_skip()? {
        return Ok(skip);
    }
    let (set, line, prompt) = if cfg!(windows) {
        (
            "Add-Type -AssemblyName System.Windows.Forms, System.Drawing; \
             [Windows.Forms.Clipboard]::SetImage((New-Object Drawing.Bitmap 4, 4)); 'set-' + 'ok'\\r"
                .to_string(),
            "'after-' + (1+1)",
            "PS>",
        )
    } else {
        (
            format!("printf %s '{PNG}' | base64 -d | {} -selection clipboard -t image/png; echo set-''ok\\r", xclip(ctx)),
            "echo after-$((1+1))",
            "$",
        )
    };
    let run = Run::new(ctx, "run").prompt();
    let out = (if cfg!(windows) { run } else { run.send("PS1='$ '\\r") })
        .send(&set)
        .wait_for("set-ok")
        .action("paste")
        .send(&format!("{line}\\r"))
        .wait_for("after-2")
        .dump_at(dump_limit(7))
        .run()?;
    waited(&out)?;
    let typed = out.dump.screen.iter().find(|row| row.contains(line));
    let want = format!("{prompt} {line}");
    ensure!(typed == Some(&want), "the command line is {typed:?}, want {want:?}; screen {:?}", out.dump.screen);
    kill_xclip(ctx);
    Ok(Outcome::Pass)
}

/// Two lines copied are on the clipboard with the platform's line end, `\n`
/// on Linux and `\r\n` on Windows (the other program reads them as bytes);
/// and `a\r\nb` that another program set arrives in the program as `a\rb`,
/// a paste's Enter.
pub fn multiline_line_ends(ctx: &Ctx) -> Result<Outcome> {
    if let Some(skip) = xclip_skip()? {
        return Ok(skip);
    }
    let run = Run::new(ctx, "copied").prompt().send(&show(&[(10, "on", "e"), (11, "tw", "o")])).wait_for("two");
    let out = select_rows(run, 9, 10)
        .action("copy")
        .send(&read_clipboard_bytes(ctx))
        .wait_for("bytes=[")
        .dump_at(dump_limit(9))
        .run()?;
    waited(&out)?;
    let want = byte_listing(if cfg!(windows) { b"one\r\ntwo" } else { b"one\ntwo" });
    let got = bracketed(&out.dump.screen, "bytes=[");
    ensure!(got.as_deref() == Some(&want[..]), "the other program read the bytes {got:?}, want {want:?}; screen {:?}", out.dump.screen);

    let (set, read) = if cfg!(windows) {
        (
            "Set-Clipboard -Value ('a' + [char]13 + [char]10 + 'b'); 'set-' + 'ok'\\r".to_string(),
            "'rea' + 'dy'; $k = 1..3 | % { [int][Console]::ReadKey($true).KeyChar }; 'ke' + 'ys=[' + ($k -join ' ') + ']'\\r"
                .to_string(),
        )
    } else {
        (
            format!("printf 'a\\015\\012b' | {} -selection clipboard; echo set-''ok\\r", xclip(ctx)),
            "stty raw -echo; echo rea''dy; got=$(dd bs=1 count=3 2>/dev/null | od -An -tx1); stty sane; echo \"ke\"\"ys=[$got]\"\\r"
                .to_string(),
        )
    };
    let out = Run::new(ctx, "pasted")
        .prompt()
        .send(&set)
        .wait_for("set-ok")
        .send(&read)
        .wait_for("ready")
        .action("paste")
        .wait_for("keys=[")
        .dump_at(dump_limit(8))
        .run()?;
    waited(&out)?;
    let want = byte_listing(b"a\rb");
    let got = bracketed(&out.dump.screen, "keys=[");
    ensure!(got.as_deref() == Some(&want[..]), "the program read {got:?}, want {want:?}; screen {:?}", out.dump.screen);
    kill_xclip(ctx);
    Ok(Outcome::Pass)
}

/// The harness's own checks of this file's helpers (`--self-test`).
pub fn self_test() -> Result {
    ensure!(matches!(missing_tool("xclip", true, true), Ok(None)), "a tool that is there runs the case");
    ensure!(
        missing_tool("xclip", false, false).is_ok_and(|o| matches!(o, Some(Outcome::Skip(why)) if why == "xclip isn't installed")),
        "a missing tool skips with its name"
    );
    let required = missing_tool("xclip", false, true);
    ensure!(
        required.as_ref().err().is_some_and(|why| why.contains("xclip isn't installed") && why.contains("BRINDLE_E2E_CLIPBOARD_REQUIRED")),
        "a missing tool fails when required: {:?}",
        required.map(|_| ())
    );
    let screen: Vec<String> = ["$ echo x", "clip=[  one   two", "three]", "$ "].iter().map(|s| s.to_string()).collect();
    ensure!(bracketed(&screen, "clip=[").as_deref() == Some("one twothree"), "bracketed: {:?}", bracketed(&screen, "clip=["));
    ensure!(bracketed(&screen, "nope=[").is_none(), "a key that isn't there");
    ensure!(bracketed(&["clip=[open".to_string()], "clip=[").is_none(), "no closing bracket");
    let shown = show(&[(10, "on", "e"), (11, "tw", "o")]);
    ensure!(shown.matches("[10;3H").count() == 1 && shown.matches("[11;3H").count() == 1, "show: {shown}");
    ensure!(!shown.contains("one") && !shown.contains("two"), "the typed line shows the text: {shown}");
    ensure!(byte_listing(b"a\rb") == if cfg!(windows) { "97 13 98" } else { "61 0d 62" }, "byte listing {}", byte_listing(b"a\rb"));
    Ok(())
}
