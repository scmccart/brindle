//! Divider junctions: lines meet with no gap, and the active pane's
//! highlight turns its corners on the lines it meets.
//!
//! Border styles make the lines magenta and the highlight yellow, so both
//! are known and differ from the background.

use super::{cells, server};
use crate::brindle::{Output, Run, theme};
use crate::capture::{Arm, Junction};
use crate::tmux::TmuxServer;
use crate::{Ctx, Outcome, Result, ensure};

fn styled(server: &TmuxServer, ctx: &Ctx) -> Result {
    server.run(&["set", "-w", "-t", "e2e", "pane-border-style", "fg=magenta"])?;
    server.run(&["set", "-w", "-t", "e2e", "pane-active-border-style", "fg=yellow"])?;
    let panes = server.run(&["list-panes", "-t", "e2e", "-F", "#{pane_id}"])?;
    for pane in panes.lines() {
        server.type_line(pane, "clear")?;
    }
    ctx.sleep(0.5);
    Ok(())
}

/// The junction where the vertical divider in column `col` meets the
/// horizontal one in row `row`, read from the cells around it.
fn junction(server: &TmuxServer, out: &Output, active: &str, col: u16, row: u16) -> Result<Junction> {
    let cells = cells(server, out, active)?;
    let image = out.image.as_ref().ok_or("no capture")?;
    image.junction(cells.rect(col - 1, row - 1, 3, 3), &[theme::YELLOW, theme::MAGENTA])
}

fn expect(what: &str, got: Junction, want: Junction) -> Result {
    ensure!(got == want, "{what}: arms {got:?}, want {want:?}");
    Ok(())
}

/// Four panes tiled 2x2 with the top-left one active: the highlight runs
/// down the vertical divider and along the horizontal one, turning at the
/// cross.
pub fn tiled(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    server.run(&["split-window", "-h", "-t", "%0"])?;
    server.run(&["split-window", "-v", "-t", "%0"])?;
    server.run(&["split-window", "-v", "-t", "%1"])?;
    server.run(&["select-layout", "-t", "e2e", "tiled"])?;
    server.run(&["select-pane", "-t", "e2e.0"])?;
    styled(&server, ctx)?;
    let out = Run::new(ctx, &server, "run").capture_at(3.5).dump_at(5.0).run()?;
    let active = server.active_pane()?;
    let [left, top, width, height] = server.geometry(&active)?;
    ensure!((left, top) == (0, 0), "the active pane {active} isn't top-left");
    let got = junction(&server, &out, &active, left + width, top + height)?;
    let (yellow, magenta) = (Arm::Color(theme::YELLOW), Arm::Color(theme::MAGENTA));
    expect("cross", got, Junction { up: yellow, left: yellow, down: magenta, right: magenta })?;
    Ok(Outcome::Pass)
}

/// One pane on the left and two stacked on the right, the top-right one
/// active: the horizontal divider reaches the vertical one, and the
/// highlight turns from it onto the vertical divider.
pub fn t_junction(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    server.run(&["split-window", "-h", "-t", "%0"])?;
    server.run(&["split-window", "-v", "-t", "%1"])?;
    server.run(&["select-pane", "-t", "%1"])?;
    styled(&server, ctx)?;
    let out = Run::new(ctx, &server, "run").capture_at(3.5).dump_at(5.0).run()?;
    let [left, top, _, height] = server.geometry("%1")?;
    let got = junction(&server, &out, "%1", left - 1, top + height)?;
    let (yellow, magenta) = (Arm::Color(theme::YELLOW), Arm::Color(theme::MAGENTA));
    expect("T-junction", got, Junction { up: yellow, right: yellow, down: magenta, left: Arm::None })?;
    Ok(Outcome::Pass)
}
