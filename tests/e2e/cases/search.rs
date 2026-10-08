//! Scrollback search: the find bar's matches, the current match scrolled
//! into view and drawn in the theme's focused-match color, escape putting
//! the view back, and a control-mode pane searching only itself.
//!
//! A local tab prints `line-1` … `line-300`. Searching `line-4` matches
//! `line-4` and `line-40` … `line-49` (11 matches); the newest, `line-49`,
//! is far up in the history, so the view must scroll to it.

use super::{cells, server};
use crate::brindle::{Output, Run, theme};
use crate::capture::{Cells, Image, PxRect};
use crate::{Ctx, Outcome, Result, ensure};

const PRINT_LINES: &str = "clear; for i in $(seq 1 300); do echo line-$i; done\\r";

/// The current match's (row, column) on screen, from the find line.
fn focused(out: &Output) -> Result<(u16, u16)> {
    let field = out.dump.find_field("focused").ok_or_else(|| format!("no find line: {:?}", out.dump.find))?;
    let (row, col) = field.split_once(',').ok_or_else(|| format!("current match not on screen: {field}"))?;
    Ok((row.parse().map_err(|_| field.to_string())?, col.parse().map_err(|_| field.to_string())?))
}

fn image(out: &Output) -> Result<&Image> {
    out.image.as_ref().ok_or_else(|| "no capture".to_string())
}

/// At least half of `r` is in `color` (the rest is glyph pixels).
fn mostly(image: &Image, what: &str, r: PxRect, color: crate::capture::Rgb) -> Result {
    let area = ((r.x1 - r.x0) * (r.y1 - r.y0)) as usize;
    image.expect(what, r, color, area / 2)
}

fn local_cells(out: &Output) -> Result<Cells> {
    Ok(Cells::from_pane(out.dump.grid()?, 0, 0))
}

pub fn local(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let out = Run::new(ctx, &server, "run")
        .profile("Shell")
        .send(PRINT_LINES)
        .find("line-4")
        .capture_at(4.5)
        .dump_at(5.0)
        .run()?;
    let dump = &out.dump;
    ensure!(dump.find_field("count") == Some("11"), "find line: {:?}", dump.find);
    ensure!(dump.find_field("index") == Some("1"), "find line: {:?}", dump.find);
    let (row, col) = focused(&out)?;
    let (_, rows) = dump.size.ok_or("no size")?;
    ensure!(col == 0, "current match at column {col}");
    ensure!(
        dump.screen.get(row as usize).map(String::as_str) == Some("line-49"),
        "row {row} is {:?}, want line-49; screen starts {:?}",
        dump.screen.get(row as usize),
        dump.screen.first()
    );
    // Scrolled so the match sits in the middle, not at an edge.
    ensure!(row.abs_diff(rows / 2) <= 1, "current match on row {row} of {rows}, not the middle");

    let image = image(&out)?;
    let cells = local_cells(&out)?;
    mostly(image, "current match", cells.rect(0, row, 6, 1), theme::SEARCH_FOCUSED_BG)?;
    // line-48, one row up, is another match.
    mostly(image, "other match", cells.rect(0, row - 1, 6, 1), theme::SEARCH_MATCH_BG)?;
    // line-50, one row down, is not.
    let below = cells.rect(0, row + 1, 6, 1);
    image.expect_none("non-match", below, theme::SEARCH_MATCH_BG)?;
    image.expect_none("non-match", below, theme::SEARCH_FOCUSED_BG)?;
    Ok(Outcome::Pass)
}

/// The text on the current match's row.
fn focused_line(out: &Output) -> Result<String> {
    let (row, _) = focused(out)?;
    Ok(out.dump.screen.get(row as usize).cloned().unwrap_or_default())
}

/// Next and previous, through the actions' bubbling from the bar to the
/// terminal: one step up, back down, then down past the newest match,
/// which wraps to the oldest (`line-4`, number 11).
pub fn step(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let up = Run::new(ctx, &server, "up").profile("Shell").send(PRINT_LINES).find("line-4").action("find_next").dump_at(5.0).run()?;
    ensure!(up.dump.find_field("index") == Some("2"), "after find_next: {:?}", up.dump.find);
    ensure!(focused_line(&up)? == "line-48", "after find_next the current match is on {:?}", focused_line(&up)?);

    let wrap = Run::new(ctx, &server, "wrap")
        .profile("Shell")
        .send(PRINT_LINES)
        .find("line-4")
        .action("find_next")
        .action("find_previous")
        .action("find_previous")
        .dump_at(7.0)
        .run()?;
    ensure!(wrap.dump.find_field("index") == Some("11"), "after wrapping: {:?}", wrap.dump.find);
    ensure!(focused_line(&wrap)? == "line-4", "after wrapping the current match is on {:?}", focused_line(&wrap)?);
    Ok(Outcome::Pass)
}

pub fn close_restores(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let out = Run::new(ctx, &server, "run")
        .profile("Shell")
        .send(PRINT_LINES)
        .find("line-4")
        .action("find_close")
        .capture_at(5.5)
        .dump_at(6.0)
        .run()?;
    let dump = &out.dump;
    ensure!(dump.find.is_none(), "the find bar is still open: {:?}", dump.find);
    ensure!(
        dump.screen.iter().any(|l| l == "line-300"),
        "the view didn't go back to the bottom; screen ends {:?}",
        dump.screen.last()
    );
    let image = image(&out)?;
    let (cols, rows) = dump.size.ok_or("no size")?;
    let all = local_cells(&out)?.rect(0, 0, cols, rows);
    image.expect_none("after close", all, theme::SEARCH_MATCH_BG)?;
    image.expect_none("after close", all, theme::SEARCH_FOCUSED_BG)?;
    Ok(Outcome::Pass)
}

/// Both panes show `needle`; searching in the active one (%1) highlights
/// only its own.
pub fn tmux_pane(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    server.run(&["split-window", "-h", "-t", "e2e"])?;
    // printf keeps the word itself out of the typed command line.
    for pane in ["%0", "%1"] {
        server.type_line(pane, "clear; printf 'nee%s\\n' dle")?;
    }
    ctx.sleep(0.5);
    let out = Run::new(ctx, &server, "run").find("needle").capture_at(3.5).dump_at(4.0).run()?;
    ensure!(server.active_pane()? == "%1", "the split isn't the active pane");
    ensure!(out.dump.find_field("count") == Some("1"), "find line: {:?}", out.dump.find);
    let (row, col) = focused(&out)?;
    let window = cells(&server, &out, "%1")?;
    let [left1, top1, _, _] = server.geometry("%1")?;
    let image = image(&out)?;
    mostly(image, "active pane's needle", window.rect(left1 + col, top1 + row, 6, 1), theme::SEARCH_FOCUSED_BG)?;

    let [left0, top0, _, _] = server.geometry("%0")?;
    let other_row = server
        .capture("%0")?
        .iter()
        .position(|l| l == "needle")
        .ok_or("no needle line in %0")? as u16;
    let other = window.rect(left0, top0 + other_row, 6, 1);
    image.expect_none("other pane's needle", other, theme::SEARCH_FOCUSED_BG)?;
    image.expect_none("other pane's needle", other, theme::SEARCH_MATCH_BG)?;
    Ok(Outcome::Pass)
}
