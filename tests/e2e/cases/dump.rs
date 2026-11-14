//! The debug dump's grid line, which every pixel case relies on, and its
//! `--wait-for` lines.

use super::cells;
use crate::brindle::{Run, theme};
use crate::{Ctx, Outcome, Result, ensure};

/// A red cell printed at column 5 of row 2 is found where the dumped grid
/// geometry says that cell is, and its neighbours aren't red. (Octal
/// escapes: `--send` turns a backslash-n or -e into a real control.)
pub fn grid_geometry(ctx: &Ctx) -> Result<Outcome> {
    let print = r"clear; printf '\012\012     \033[41m \033[0m\012'\r";
    let out = Run::new(ctx, "run").send(print).capture_at(3.5).dump_at(5.0).run()?;
    let image = out.image.as_ref().unwrap();
    let cells = cells(&out)?;
    let red = cells.rect(5, 2, 1, 1);
    let area = ((red.x1 - red.x0) * (red.y1 - red.y0)) as usize;
    image.expect("cell 5,2", red, theme::RED, area * 3 / 4)?;
    for (col, row) in [(4, 2), (6, 2), (5, 1), (5, 3)] {
        image.expect_none(&format!("cell {col},{row}"), cells.rect(col, row, 1, 1), theme::RED)?;
    }
    Ok(Outcome::Pass)
}

/// A `--wait-for` whose text never appears gives up after 30 step
/// intervals (3 s at 0.1 s), and the dump names it. The text is one no
/// program prints, so nothing depends on the shell.
pub fn wait_for_timeout(ctx: &Ctx) -> Result<Outcome> {
    let out = Run::new(ctx, "run").step_interval(0.1).wait_for("NEVER").dump_at(6.0).run()?;
    ensure!(out.dump.wait_timeouts == ["NEVER"], "timeouts {:?}", out.dump.wait_timeouts);
    ensure!(out.dump.wait_pending.is_empty(), "pending {:?}", out.dump.wait_pending);
    let raw = std::fs::read_to_string(ctx.dir.join("run.dump")).map_err(|e| e.to_string())?;
    ensure!(raw.lines().any(|l| l == "--- wait-for timeout \"NEVER\""), "no timeout line in the dump:\n{raw}");
    Ok(Outcome::Pass)
}

/// A `--wait-for` still waiting when the dump is taken (its limit is 30
/// step intervals, long past the dump) is reported as pending.
pub fn wait_for_pending(ctx: &Ctx) -> Result<Outcome> {
    let out = Run::new(ctx, "run").wait_for("NEVER").dump_at(3.0).run()?;
    ensure!(out.dump.wait_pending == ["NEVER"], "pending {:?}", out.dump.wait_pending);
    ensure!(out.dump.wait_timeouts.is_empty(), "timeouts {:?}", out.dump.wait_timeouts);
    let raw = std::fs::read_to_string(ctx.dir.join("run.dump")).map_err(|e| e.to_string())?;
    ensure!(raw.lines().any(|l| l == "--- wait-for pending \"NEVER\""), "no pending line in the dump:\n{raw}");
    Ok(Outcome::Pass)
}

/// A wait that finds its text leaves neither line, nor does a dump with no
/// `--wait-for` step.
pub fn wait_for_clean(ctx: &Ctx) -> Result<Outcome> {
    let out = Run::new(ctx, "found").send("echo clean-$((1+1))\r").wait_for("clean-2").dump_at(5.0).run()?;
    ensure!(out.dump.wait_pending.is_empty() && out.dump.wait_timeouts.is_empty(), "found wait left lines in {:?}", out.dump);
    let out = Run::new(ctx, "none").dump_at(2.0).run()?;
    ensure!(out.dump.wait_pending.is_empty() && out.dump.wait_timeouts.is_empty(), "wait lines in {:?}", out.dump);
    let raw = std::fs::read_to_string(ctx.dir.join("none.dump")).map_err(|e| e.to_string())?;
    ensure!(!raw.contains("--- wait-for"), "wait lines in the dump:\n{raw}");
    Ok(Outcome::Pass)
}
