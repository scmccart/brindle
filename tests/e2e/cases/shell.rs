//! Shell integration (OSC 133): prompt jumps, failed-command marks and
//! last-output selection, in local tabs. VS Code's OSC 633: its marks, a
//! reported command line copied with Copy Last Command Line, and its
//! directory report (in a container tab, where a new tab starts in the
//! reported directory); and each shipped script reporting its command lines.
//!
//! What Copy Last Command Line put on the clipboard is read back by pasting
//! it into a bash `read` in the tab ([`Clip`]), which saves it to a file.

use std::path::PathBuf;

use crate::brindle::{Output, Run, theme};
use crate::capture::{Cells, PxRect};
use crate::{Ctx, Outcome, Result, ensure};

/// Typed first in every case: a plain prompt, Brindle's bash script, then a
/// clean screen so the first marked prompt is on row 0.
fn setup(ctx: &Ctx) -> String {
    format!("PS1='$ '; eval \"$({} --shell-integration bash)\"; clear\\r", ctx.bin.display())
}

/// Lines `seq` printed after two jumps up: the earlier prompt is on top.
fn expect_jumped(out: &Output) -> Result {
    let screen = &out.dump.screen;
    ensure!(
        screen.first().map(String::as_str) == Some("$ seq 1 60") && screen.get(1).map(String::as_str) == Some("1"),
        "after two jumps the view should start at the first prompt; it starts {:?}",
        &screen[..screen.len().min(3)]
    );
    Ok(())
}

/// Two long outputs, then ctrl-shift-up twice (as actions): the first
/// prompt ends up as the top line.
pub fn prompt_jump(ctx: &Ctx) -> Result<Outcome> {
    let out = Run::new(ctx, "run")
        .profile("Shell")
        .send(&setup(ctx))
        .send("seq 1 60\\r")
        .send("seq 61 120\\r")
        .action("jump_to_previous_prompt")
        .action("jump_to_previous_prompt")
        .dump_at(7.0)
        .run()?;
    expect_jumped(&out)?;
    Ok(Outcome::Pass)
}

/// Without shell integration the jump keys scroll one line, as they did.
pub fn no_marks_scroll_line(ctx: &Ctx) -> Result<Outcome> {
    let out = Run::new(ctx, "run")
        .profile("Shell")
        .send("PS1='$ '; clear\\r")
        .send("seq 1 60\\r")
        .action("jump_to_previous_prompt")
        .dump_at(4.5)
        .run()?;
    let (_, rows) = out.dump.size.ok_or("no size")?;
    // Lines: "$ seq 1 60", 1..=60, "$ " (62); one line up from the bottom
    // the top line is line 62 - rows, which shows the number before it.
    let expected = (61 - rows as i32).to_string();
    ensure!(
        out.dump.screen.first() == Some(&expected),
        "expected the view one line up (top line {expected:?}), got {:?}",
        out.dump.screen.first()
    );
    Ok(Outcome::Pass)
}

/// `false` gets a red bar in the left padding beside its prompt; `true`
/// doesn't.
pub fn failed_mark(ctx: &Ctx) -> Result<Outcome> {
    let out = Run::new(ctx, "run")
        .profile("Shell")
        .send(&setup(ctx))
        .send("false\\r")
        .send("true\\r")
        .capture_at(4.5)
        .dump_at(5.0)
        .run()?;
    let screen = &out.dump.screen;
    let row = |text: &str| screen.iter().position(|l| l == text).ok_or(format!("no {text:?} line in {screen:?}"));
    let (failed, passed) = (row("$ false")? as u16, row("$ true")? as u16);
    let grid = out.dump.grid()?;
    let cells = Cells::from_pane(grid, 0, 0);
    let image = out.image.as_ref().unwrap();
    // The bar is 3 px wide, 1 px left of the grid (padding is 6).
    let gutter = |row: u16| {
        let cell = cells.rect(0, row, 1, 1);
        let px = |v: f32| (v * grid.scale).round() as u32;
        PxRect { x0: px(grid.x - 4.0), x1: px(grid.x - 1.0), y0: cell.y0, y1: cell.y1 }
    };
    let bar = gutter(failed);
    let area = ((bar.x1 - bar.x0) * (bar.y1 - bar.y0)) as usize;
    image.expect("gutter beside `false`", bar, theme::RED, area / 2)?;
    image.expect_none("gutter beside `true`", gutter(passed), theme::RED)?;
    // Nothing red over the prompt text itself.
    image.expect_none("first cell of the `false` prompt", cells.rect(0, failed, 1, 1), theme::RED)?;
    Ok(Outcome::Pass)
}

/// Select Last Command Output selects exactly what `printf` printed.
/// (Octal newlines: `--send` turns a backslash-n into a real newline.)
pub fn select_output(ctx: &Ctx) -> Result<Outcome> {
    let out = Run::new(ctx, "run")
        .profile("Shell")
        .send(&setup(ctx))
        .send("printf 'one\\012two\\012'\\r")
        .action("select_last_output")
        .dump_at(4.5)
        .run()?;
    ensure!(
        out.dump.selection.as_deref() == Some("one\ntwo"),
        "selection {:?}, screen {:?}",
        out.dump.selection,
        out.dump.screen
    );
    Ok(Outcome::Pass)
}

/// Up then down after a clear at a marked prompt lower on the screen: down
/// must reach the `false` prompt (the view's bottom, where it and the
/// current prompt are), not stop at the erased prompt's leftover mark. The
/// clear is `CSI H CSI J CSI 3 J` (erase in place), from `printf`.
pub fn clear_up_down(ctx: &Ctx) -> Result<Outcome> {
    let out = Run::new(ctx, "run")
        .send(&setup(ctx))
        .send(r"seq 1 5\r")
        .send(r"printf '\033[H\033[J\033[3J'\r")
        .send(r"seq 1 60\r")
        .send(r"false\r")
        .action("jump_to_previous_prompt")
        .action("jump_to_next_prompt")
        .dump_at(9.0)
        .run()?;
    let screen = &out.dump.screen;
    let tail: Vec<&str> = screen.iter().rev().take(2).rev().map(String::as_str).collect();
    ensure!(
        tail == ["$ false", "$"],
        "after up then down the view should be back at the bottom showing the `false` prompt; \
         it ends {tail:?} (starts {:?})",
        screen.first()
    );
    Ok(Outcome::Pass)
}

// ---- OSC 633 ----------------------------------------------------------------------

/// The clipboard, read back: a bash script typed into the tab, which waits
/// for a paste ended by GS (`\x1d`, an ordinary byte to a tty, which then
/// passes it on with the line) and saves what it read to a file.
pub(super) struct Clip {
    name: &'static str,
    script: PathBuf,
    file: PathBuf,
}

/// GS: what ends the paste [`Clip`]'s script reads.
const PASTE_END: char = '\u{1d}';

impl Clip {
    pub(super) fn new(ctx: &Ctx, name: &'static str) -> Result<Self> {
        let file = ctx.dir.join(format!("{name}.txt"));
        let script = ctx.dir.join(format!("{name}.sh"));
        let _ = std::fs::remove_file(&file);
        // Its markers are printed in pieces, so the line that runs it
        // doesn't show them.
        let body = format!(
            "printf '%s-%s\\n' {name} ready\n\
             IFS= read -r -d $'\\035' clip\n\
             printf '%s' \"$clip\" > '{}'\n\
             printf '%s-%s\\n' {name} saved\n",
            file.display()
        );
        std::fs::write(&script, body).map_err(|e| format!("writing {}: {e}", script.display()))?;
        Ok(Clip { name, script, file })
    }

    /// Copy Last Command Line, then the paste into the script, until it has
    /// saved it.
    pub(super) fn copy<'a>(&self, run: Run<'a>) -> Run<'a> {
        run.action("copy_last_command")
            .send(&format!("bash '{}'\\r", self.script.display()))
            .wait_for(&format!("{}-ready", self.name))
            .action("paste")
            .send(&format!("{PASTE_END}\\r"))
            .wait_for(&format!("{}-saved", self.name))
    }

    /// The clipboard held `expected`, and every wait was met.
    pub(super) fn expect(&self, expected: &str, out: &Output) -> Result {
        ensure!(
            out.dump.wait_timeouts.is_empty() && out.dump.wait_pending.is_empty(),
            "waits {:?} {:?}; screen {:?}",
            out.dump.wait_timeouts,
            out.dump.wait_pending,
            out.dump.screen
        );
        let text = std::fs::read_to_string(&self.file)
            .map_err(|e| format!("the clipboard wasn't saved to {}: {e}", self.file.display()))?;
        ensure!(text == expected, "{}: the clipboard held {text:?}, not {expected:?}", self.name);
        Ok(())
    }
}

/// A bash command that prints a whole command with OSC 633 marks, as a
/// shell would: `$ wrong` on the screen between `B` and `C`, then `E` with
/// `line` (as it goes in the sequence, escaped, then a nonce), and
/// `fake-done` after it.
pub(super) fn fake_633_command(line: &str) -> String {
    format!(
        r"printf '\033]633;A\007$ \033]633;B\007wrong\015\012\033]633;E;{line};n0nce\007\033]633;C\007\033]633;D;0\007'; printf 'fake-%s\012' done\r"
    )
}

/// A bash prompt that marks itself with OSC 633 only, as VS Code's scripts
/// do, then the jumps that [`prompt_jump`] makes.
pub fn marks_633(ctx: &Ctx) -> Result<Outcome> {
    let setup = r#"PS1='\[\033]633;A\007\]$ \[\033]633;B\007\]'; PS0='\033]633;C\007'; PROMPT_COMMAND='printf "\033]633;D;%s\007" $?'; clear\r"#;
    let out = Run::new(ctx, "run")
        .profile("Shell")
        .send(setup)
        .send("seq 1 60\\r")
        .send("seq 61 120\\r")
        .action("jump_to_previous_prompt")
        .action("jump_to_previous_prompt")
        .dump_at(7.0)
        .run()?;
    expect_jumped(&out)?;
    Ok(Outcome::Pass)
}

/// The line a program reports with `633;E`, decoded, is what Copy Last
/// Command Line copies, not the screen's text between the marks.
pub fn copy_line_633(ctx: &Ctx) -> Result<Outcome> {
    let clip = Clip::new(ctx, "clip")?;
    let run = Run::new(ctx, "run")
        .profile("Shell")
        .send("PS1='$ '; HISTFILE=; clear\\r")
        .send(&fake_633_command(r"echo copied-$((6*7))\\x3b echo \\\\done"))
        .wait_for("fake-done");
    let out = clip.copy(run).dump_at(12.0).run()?;
    clip.expect(r"echo copied-$((6*7)); echo \done", &out)?;
    Ok(Outcome::Pass)
}

/// `633;P;Cwd` sets the directory a new tab on the same endpoint starts
/// in. Local Linux tabs start in their process's directory, so this runs in
/// a container tab, whose shell reports a directory it never went to.
pub fn cwd_633(ctx: &Ctx) -> Result<Outcome> {
    use super::remote::{CONTAINER_PROMPT, in_container_tab};
    in_container_tab(ctx, "run", |run| {
        let out = run
            .wait_for(CONTAINER_PROMPT)
            .send(r"mkdir -p '/tmp/a;b'; printf '\033]633;P;Cwd=%s\007' '/tmp/a\x3bb'; printf 'cwd-%s\012' set\r")
            .wait_for("cwd-set")
            .action("new_tab")
            .wait_for(CONTAINER_PROMPT)
            .send(r"printf 'pwd-%s:' is; pwd\r")
            .wait_for("pwd-is:/")
            .dump_at(20.0)
            .run()?;
        ensure!(out.dump.wait_timeouts.is_empty(), "waits {:?}; screen {:?}", out.dump.wait_timeouts, out.dump.screen);
        ensure!(out.dump.tabs.len() == 2, "tabs: {:?}", out.dump.tabs);
        ensure!(
            out.dump.screen.iter().any(|l| l == "pwd-is:/tmp/a;b"),
            "the new tab didn't start in the reported directory: {:?}",
            out.dump.screen
        );
        Ok(Outcome::Pass)
    })
}

/// Whether `program` is on `PATH`.
fn installed(program: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
}

/// Brindle's bash script reports a line from history: Copy Last Command
/// Line gets it, `;` and `\` included. With `HISTCONTROL=ignoreboth` a
/// command typed after a space adds no entry, so none is sent, and the copy
/// is the screen's text between the marks: `ls -d /`, not the `echo make`
/// before it.
pub fn script_line_bash(ctx: &Ctx) -> Result<Outcome> {
    let (first, second) = (Clip::new(ctx, "first")?, Clip::new(ctx, "second")?);
    let run = Run::new(ctx, "run")
        .profile("Shell")
        .send(&format!("HISTFILE=; {}", setup(ctx)))
        .send(r#"echo "a;b" 'c\d'\r"#);
    let run = first.copy(run).send("HISTCONTROL=ignoreboth\\r").send("echo make\\r").send(" ls -d /\\r");
    let out = second.copy(run).dump_at(20.0).run()?;
    first.expect(r#"echo "a;b" 'c\d'"#, &out)?;
    second.expect("ls -d /", &out)?;
    Ok(Outcome::Pass)
}

/// Brindle's zsh script reports the line as typed: without the right
/// prompt the screen shows beside it, and a command continued onto a
/// second line with both lines.
pub fn script_line_zsh(ctx: &Ctx) -> Result<Outcome> {
    if !installed("zsh") {
        return Ok(Outcome::Skip("zsh isn't installed".into()));
    }
    let (first, second) = (Clip::new(ctx, "first")?, Clip::new(ctx, "second")?);
    let setup =
        format!("PS1='$ '; RPROMPT='right-prompt'; eval \"$({} --shell-integration zsh)\"; clear\\r", ctx.bin.display());
    let run = Run::new(ctx, "run").command(&["zsh", "-f"]).send(&setup).send("ls -la /\\r");
    // `\\r` after a backslash: the line ends `one \`, and Enter continues it.
    let run = first.copy(run).send(r"echo one \\rtwo\r");
    let out = second.copy(run).dump_at(18.0).run()?;
    first.expect("ls -la /", &out)?;
    second.expect("echo one \\\ntwo", &out)?;
    ensure!(
        out.dump.screen.iter().any(|l| l.ends_with("right-prompt")),
        "no right prompt on the screen: {:?}",
        out.dump.screen
    );
    Ok(Outcome::Pass)
}

/// Brindle's fish script, which sends no `B`, reports the line: the only
/// way Copy Last Command Line gets it.
pub fn script_line_fish(ctx: &Ctx) -> Result<Outcome> {
    if !installed("fish") {
        return Ok(Outcome::Skip("fish isn't installed".into()));
    }
    let clip = Clip::new(ctx, "clip")?;
    // fish keeps its universal variables under these; not the user's.
    let config = format!("XDG_CONFIG_HOME={}", ctx.dir.join("fish-config").display());
    let data = format!("XDG_DATA_HOME={}", ctx.dir.join("fish-data").display());
    let setup =
        format!("function fish_prompt; echo -n '$ '; end; {} --shell-integration fish | source; clear\\r", ctx.bin.display());
    let run = Run::new(ctx, "run")
        .command(&["env", &config, &data, "fish", "--no-config"])
        .send(&setup)
        .send(r#"echo "a;b"\r"#);
    let out = clip.copy(run).dump_at(12.0).run()?;
    clip.expect(r#"echo "a;b""#, &out)?;
    Ok(Outcome::Pass)
}
