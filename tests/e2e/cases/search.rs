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

/// The find bar in a capture, found by its accent border (drawn while the
/// query has focus), just below the terminal's top edge.
fn find_bar(out: &Output) -> Result<PxRect> {
    let image = image(out)?;
    let grid = out.dump.grid()?;
    let px = |v: f32| (v * grid.scale).round().max(0.0) as u32;
    let (top, bottom) = (px(grid.y - 6.0), px(grid.y + 60.0));
    let mut bar: Option<PxRect> = None;
    for y in top..bottom.min(image.height) {
        for x in 0..image.width {
            if image.pixel(x, y).near(theme::ACCENT, 8) {
                let b = bar.get_or_insert(PxRect { x0: x, y0: y, x1: x + 1, y1: y + 1 });
                (b.x0, b.y0, b.x1, b.y1) = (b.x0.min(x), b.y0.min(y), b.x1.max(x + 1), b.y1.max(y + 1));
            }
        }
    }
    let bar = bar.ok_or("no find bar border in the capture")?;
    ensure!(bar.x1 - bar.x0 > px(200.0), "find bar too narrow: {bar:?}");
    Ok(bar)
}

/// A strip of the bar's interior between `x0` and `x1` logical pixels from
/// its left edge.
fn bar_strip(out: &Output, bar: PxRect, x0: f32, x1: f32) -> Result<PxRect> {
    let scale = out.dump.grid()?.scale;
    let px = |v: f32| (v * scale).round() as u32;
    Ok(PxRect { x0: bar.x0 + px(x0), y0: bar.y0 + px(4.0), x1: bar.x0 + px(x1), y1: bar.y1 - px(4.0) })
}

fn mix(a: u8, b: u8, t: f32) -> u8 {
    (a as f32 + (b as f32 - a as f32) * t).round() as u8
}

/// The find bar's background: the theme's chrome, its background mixed 35%
/// towards black.
fn panel() -> crate::capture::Rgb {
    let bg = theme::BACKGROUND;
    crate::capture::Rgb(mix(bg.0, 0, 0.35), mix(bg.1, 0, 0.35), mix(bg.2, 0, 0.35))
}

/// A query wider than the field scrolls to keep the cursor (at its end)
/// visible and stays clear of the buttons. The query starts with sparse
/// dots and continues with dense `W`s, so the field's left edge shows `W`s
/// only when the text has scrolled.
pub fn long_query(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let query = format!(".....{}", "W".repeat(85));
    let out = Run::new(ctx, &server, "run").profile("Shell").find(&query).capture_at(3.0).dump_at(3.5).run()?;
    ensure!(out.dump.find_field("count") == Some("0"), "find line: {:?}", out.dump.find);
    let input = out.dump.input.as_ref().ok_or("no focused input in the dump")?;
    ensure!(input.text == query && input.cursor == query.len(), "input {input:?}");
    let image = image(&out)?;
    let bar = find_bar(&out)?;
    let width = (bar.x1 - bar.x0) as f32 / out.dump.grid()?.scale;
    let field_start = bar_strip(&out, bar, 10.0, 40.0)?;
    let buttons = bar_strip(&out, bar, width - 140.0, width - 4.0)?;
    // Ink (anything but the bar's background) at the field's left edge:
    // dense Ws when scrolled to the end, a few dots when showing the start.
    let area = ((field_start.x1 - field_start.x0) * (field_start.y1 - field_start.y0)) as usize;
    let ink = area - image.count(field_start, panel());
    ensure!(ink >= area / 6, "{ink} of {area} pixels inked at the field's left edge: the query didn't scroll to its end");
    image.expect_none("query text under the buttons", buttons, theme::FOREGROUND)?;
    Ok(Outcome::Pass)
}

/// The selection: reopening the bar selects the restored query, which is
/// highlighted (the text color at 28% over the bar), and the cursor keys and
/// typing then edit the input, which searches again. A control run without a
/// selection shows no highlight.
pub fn input_selection(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    // The selection: the text color at 28% over the bar.
    let panel = panel();
    let fg = theme::FOREGROUND;
    let selection = crate::capture::Rgb(mix(panel.0, fg.0, 0.28), mix(panel.1, fg.1, 0.28), mix(panel.2, fg.2, 0.28));
    // Inside the selected "foo", clear of the field's left padding.
    let selected_span = |out: &Output, bar| bar_strip(out, bar, 12.0, 26.0);

    let out = Run::new(ctx, &server, "selected")
        .profile("Shell")
        .find("foo")
        .action("find_close")
        .action("find")
        .capture_at(4.2)
        .action("text_input_end")
        .action("text_input_left")
        .send("z")
        .dump_at(7.5)
        .run()?;
    let capture = image(&out)?;
    let span = selected_span(&out, find_bar(&out)?)?;
    let area = ((span.x1 - span.x0) * (span.y1 - span.y0)) as usize;
    capture.expect("selection highlight behind the restored query", span, selection, area * 2 / 5)?;
    let input = out.dump.input.as_ref().ok_or("no focused input in the dump")?;
    ensure!(input.text == "fozo" && input.cursor == 3 && input.selection == (3, 3), "after end, left and typing z: {input:?}");
    ensure!(out.dump.find.as_deref().is_some_and(|f| f.contains("query=\"fozo\"")), "the search didn't follow the edit: {:?}", out.dump.find);

    let control = Run::new(ctx, &server, "unselected").profile("Shell").find("foo").capture_at(2.5).dump_at(3.0).run()?;
    let input = control.dump.input.as_ref().ok_or("no focused input in the control dump")?;
    ensure!(input.selection == (3, 3), "control run has a selection: {input:?}");
    // Anti-aliased glyph edges can come close to the selection color; a
    // highlight covers the span.
    let span = selected_span(&control, find_bar(&control)?)?;
    let area = ((span.x1 - span.x0) * (span.y1 - span.y0)) as usize;
    let near = image(&control)?.count(span, selection);
    ensure!(near < area / 10, "{near} of {area} pixels look selected without a selection");
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
