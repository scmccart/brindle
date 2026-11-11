//! The debug dump's grid line, which every pixel case relies on.

use super::cells;
use crate::brindle::{Run, theme};
use crate::{Ctx, Outcome, Result};

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
