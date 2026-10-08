//! Shell integration (OSC 133): prompt jumps, failed-command marks and
//! last-output selection, in local tabs and control-mode panes.

use super::server;
use crate::brindle::{Output, Run, theme};
use crate::capture::{Cells, PxRect};
use crate::tmux::ControlClient;
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
    let server = server()?;
    let out = Run::new(ctx, &server, "run")
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
    let server = server()?;
    let out = Run::new(ctx, &server, "run")
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
    let server = server()?;
    let out = Run::new(ctx, &server, "run")
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
    let server = server()?;
    let out = Run::new(ctx, &server, "run")
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

/// Prompt jumps in a control-mode pane: tmux forwards the pane's OSC 133
/// marks in `%output`.
pub fn tmux_prompt_jump(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let out = Run::new(ctx, &server, "run")
        .send(&setup(ctx))
        .send("seq 1 60\\r")
        .send("seq 61 120\\r")
        .action("jump_to_previous_prompt")
        .action("jump_to_previous_prompt")
        .dump_at(7.5)
        .run()?;
    expect_jumped(&out)?;
    Ok(Outcome::Pass)
}

/// tmux passes OSC 133 from a pane to control clients unchanged (both
/// terminators). `Terminal::feed` relies on this for marks in panes.
pub fn tmux_passthrough(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let client = ControlClient::attach(&server, 100, 30)?;
    ctx.sleep(0.3);
    let mark = client.mark();
    server.type_line("%0", r"printf '\e]133;A\a\e]133;D;3\e\\x\n'")?;
    let result = client
        .wait_for(ctx, mark, 3.0, |l| l.starts_with("%output %0") && l.contains(r"\033]133;A\007"))
        .ok_or("no %output with OSC 133;A within 3 s".to_string())
        .and_then(|line| {
            ensure!(line.contains(r"\033]133;D;3\033\134"), "ST-terminated mark changed: {line:?}");
            Ok(())
        });
    ctx.save("transcript.txt", client.transcript());
    result.map(|()| Outcome::Pass)
}
