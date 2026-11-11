//! Zones and tmux windows drawn as cards (OpenSpec change `zones-shadcn`):
//! a card's 1px border in its terminal theme's colours, the 6px gap round
//! and between cards in the chrome's `background` token, the strip's colour
//! (no dividers), the corners rounded over the terminal in that colour, the
//! focus ring on the active zone of a split tab only while focus came from
//! the keyboard, in the global theme's ring colour, and one card round a
//! whole tmux window.
//!
//! Every case uses a profile theme of `ALT_FOREGROUND` on `ALT_BACKGROUND`
//! (`e2e-zone` for local tabs, `e2e-alt` for tmux) while the global theme
//! stays `e2e`, so a card's border (`ALT_CARD_BORDER`) differs from the
//! global border token (`CHROME_BORDER`), which the old dividers were drawn
//! in, and its terminal's padding (`ALT_BACKGROUND`) from the gap round it
//! (`CHROME_BACKGROUND`). Borders are matched within 2 per channel: at the
//! usual 24, the border colours and the backgrounds all match each other.

use super::zones::{ZONE_ACCENT, ZONE_PROFILE};
use super::{CARD_BORDER, CARD_INSET, Shots, ZONE_GAP, ring_between, server, step_time, styles};
use crate::brindle::{Output, Run, theme};
use crate::capture::{Changes, Image, PxRect, Rgb};
use crate::{Ctx, Outcome, Result, ensure};

/// The test config's terminal padding and a zone header's height
/// (`ZONE_HEADER_HEIGHT`, its bottom line included), in logical pixels.
const PADDING: f32 = 6.0;
const HEADER_HEIGHT: f32 = 28.0;

/// Logical pixels to device pixels.
fn dev(v: f32, scale: f32) -> u32 {
    (v * scale).round().max(0.0) as u32
}

/// `got` is `want` device pixels, give or take one.
fn near_px(got: u32, want: u32) -> bool {
    got.abs_diff(want) <= 1
}

fn image(out: &Output) -> Result<&Image> {
    out.image.as_ref().ok_or_else(|| "no capture".to_string())
}

/// A row or a column of device pixels.
#[derive(Debug, Clone, Copy)]
enum Line {
    Row(u32),
    Column(u32),
}

impl Line {
    fn runs(self, image: &Image, range: (u32, u32), color: Rgb) -> Vec<(u32, u32)> {
        match self {
            Line::Row(y) => image.runs_in_row(y, range, color, 2),
            Line::Column(x) => image.runs_in_column(x, range, color, 2),
        }
    }

    /// The colours along `range`, for failure messages.
    fn summary(self, image: &Image, (a, b): (u32, u32)) -> String {
        let r = match self {
            Line::Row(y) => PxRect { x0: a, y0: y, x1: b, y1: y + 1 },
            Line::Column(x) => PxRect { x0: x, y0: a, x1: x + 1, y1: b },
        };
        image.summary(r)
    }
}

/// The runs of the cards' border colour along `line` over `range`, after
/// checking that nothing there is in the global theme's border colour (a
/// card in the chrome's tokens, or a divider).
fn border_runs(image: &Image, what: &str, line: Line, range: (u32, u32)) -> Result<Vec<(u32, u32)>> {
    let chrome = line.runs(image, range, theme::CHROME_BORDER);
    ensure!(
        chrome.is_empty(),
        "{what}: the global border colour {} at {chrome:?} along {line:?} {range:?}; found {}",
        theme::CHROME_BORDER.hex(),
        line.summary(image, range)
    );
    Ok(line.runs(image, range, theme::ALT_CARD_BORDER))
}

/// Exactly one border run along `line` over `range`, starting at `at` (when
/// given) and one logical pixel thick.
fn one_border(image: &Image, what: &str, line: Line, range: (u32, u32), at: Option<u32>, scale: f32) -> Result<(u32, u32)> {
    let runs = border_runs(image, what, line, range)?;
    let found = || format!("{runs:?} along {line:?} {range:?}; found {}", line.summary(image, range));
    ensure!(runs.len() == 1, "{what}: want one run of {}, got {}", theme::ALT_CARD_BORDER.hex(), found());
    let run = runs[0];
    ensure!(near_px(run.1 - run.0, dev(CARD_BORDER, scale)), "{what}: the border isn't 1px: {}", found());
    if let Some(at) = at {
        ensure!(near_px(run.0, at), "{what}: the border isn't at {at}: {}", found());
    }
    Ok(run)
}

/// Every pixel along `line` over `range` is `color`.
fn expect_all(image: &Image, what: &str, line: Line, range: (u32, u32), color: Rgb) -> Result {
    ensure!(range.0 < range.1, "{what}: empty range {range:?} along {line:?}");
    let runs = line.runs(image, range, color);
    ensure!(
        runs == [range],
        "{what}: {} runs {runs:?} along {line:?}, want all of {range:?}; found {}",
        color.hex(),
        line.summary(image, range)
    );
    Ok(())
}

/// The gap round and between cards: the chrome's background token.
fn expect_gap(image: &Image, what: &str, line: Line, range: (u32, u32)) -> Result {
    expect_all(image, what, line, range, theme::CHROME_BACKGROUND)
}

/// The spec's "Unsplit tab is a card" and "Card border follows the zone's
/// theme": a one-zone tab of the `Alt` profile (`e2e-zone`) is a card with
/// a 1px border in that theme's card border colour (not the global
/// theme's border token) round its terminal and its padding (in the
/// terminal's background), and a 6px margin in the chrome's background on
/// every side: left and right along a row below the prompt, above and
/// below down a column.
pub fn zone_border(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let out = Run::new(ctx, &server, "run").profile("Alt").config(ZONE_PROFILE).capture_at(3.0).dump_at(3.5).run()?;
    ensure!(out.dump.zones.is_none(), "one zone, but --- zones {:?}", out.dump.zones);
    let image = image(&out)?;
    let grid = out.dump.grid()?;
    let s = grid.scale;
    let (cols, rows) = out.dump.size.ok_or("no size")?;
    let gap = dev(ZONE_GAP, s);
    // The card's outer corner: the grid less the padding and the border.
    let (left, top) = (grid.x - PADDING - CARD_BORDER, grid.y - PADDING - CARD_BORDER);
    let (grid_right, grid_bottom) = (grid.x + grid.cell_width * cols as f32, grid.y + grid.line_height * rows as f32);

    // Along a row below the prompt: from the window's left edge to the
    // grid, and from the grid's last cell to the window's right edge.
    let row = Line::Row(dev(grid.y + grid.line_height * 4.5, s));
    let (b0, b1) = one_border(image, "left edge", row, (0, dev(grid.x, s)), Some(dev(left, s)), s)?;
    ensure!(near_px(b0, gap), "left margin: the border at {b0}, want {gap} px from the window's edge");
    // The top margin is in the strip's colour, so it shows only in where the
    // grid is: the strip (42px: the tab list and the 6px above it), the
    // margin, the border and the padding.
    let want = (ZONE_GAP + CARD_BORDER + PADDING, 42.0 + ZONE_GAP + CARD_BORDER + PADDING);
    ensure!(
        (grid.x - want.0).abs() <= 1.0 && (grid.y - want.1).abs() <= 1.0,
        "the grid starts at {},{}, want {},{}",
        grid.x,
        grid.y,
        want.0,
        want.1
    );
    expect_gap(image, "left margin", row, (b0 + 1 - gap.min(b0 + 1), b0))?;
    expect_all(image, "left padding", row, (b1, dev(grid.x, s)), theme::ALT_BACKGROUND)?;
    let (b0, b1) = one_border(image, "right edge", row, (dev(grid_right, s), image.width), None, s)?;
    expect_all(image, "right padding", row, (dev(grid_right, s), b0), theme::ALT_BACKGROUND)?;
    expect_gap(image, "right margin", row, (b1, image.width))?;
    ensure!(near_px(image.width - b1, gap), "right margin: {} px from the border at {b0} to the window's edge", image.width - b1);

    // Down a column past the prompt: from the margin's top to the grid,
    // and from the grid's last row to the window's bottom edge. The strip,
    // with no line under it, runs on into the top margin in its colour.
    let column = Line::Column(dev(grid.x + grid.cell_width * (cols / 2) as f32, s));
    let margin_top = dev(top, s).saturating_sub(gap);
    let (b0, b1) = one_border(image, "top edge", column, (margin_top, dev(grid.y, s)), Some(dev(top, s)), s)?;
    expect_gap(image, "top margin", column, (b0.saturating_sub(gap), b0))?;
    expect_all(image, "top padding", column, (b1, dev(grid.y, s)), theme::ALT_BACKGROUND)?;
    let (_, b1) = one_border(image, "bottom edge", column, (dev(grid_bottom, s), image.height), None, s)?;
    expect_gap(image, "bottom margin", column, (b1, image.height))?;
    ensure!(near_px(image.height - b1, gap), "bottom margin: {} px under the border", image.height - b1);
    Ok(Outcome::Pass)
}

/// The spec's "Corners are rounded over the terminal" and the card's
/// corner cutouts: with no padding, a terminal whose cells are green right
/// to its top-left corner (a screen cleared with a green background) still
/// shows the card's rounded corner. The corner's pixels just inside the
/// border are the gap colour, painted over the terminal; the cells beside
/// it are green.
pub fn zone_corners(ctx: &Ctx) -> Result<Outcome> {
    const GREEN: Rgb = Rgb(0x00, 0xc8, 0x00);
    let server = server()?;
    let out = Run::new(ctx, &server, "run")
        .profile("Alt")
        .config(ZONE_PROFILE)
        .padding(0)
        .send("printf '\\033[48;2;0;200;0m\\033[2J'\r")
        .capture_at(step_time(1) + 1.5)
        .dump_at(step_time(1) + 2.0)
        .run()?;
    let image = image(&out)?;
    let grid = out.dump.grid()?;
    let s = grid.scale;
    // With no padding the grid starts right inside the border, so its
    // first logical pixel is the card's corner, a pixel in from the outer
    // one: well outside the 10px rounding.
    let corner = PxRect { x0: dev(grid.x, s), y0: dev(grid.y, s), x1: dev(grid.x + 1.0, s), y1: dev(grid.y + 1.0, s) };
    let got = image.dominant(corner);
    ensure!(
        got.is_some_and(|c| c.near(theme::CHROME_BACKGROUND, 4)),
        "corner: {:?} in {corner:?}, want the gap's {} over the green cell; found {}",
        got.map(Rgb::hex),
        theme::CHROME_BACKGROUND.hex(),
        image.summary(corner)
    );
    // Cells 2 to 5 of the first row, clear of the corner.
    let cells = PxRect {
        x0: dev(grid.x + grid.cell_width * 2.0, s),
        y0: dev(grid.y + 2.0, s),
        x1: dev(grid.x + grid.cell_width * 6.0, s),
        y1: dev(grid.y + grid.line_height - 2.0, s),
    };
    let got = image.dominant(cells);
    ensure!(
        got.is_some_and(|c| c.near(GREEN, 4)),
        "first row: {:?} in {cells:?}, want the cleared screen's {}; found {}",
        got.map(Rgb::hex),
        GREEN.hex(),
        image.summary(cells)
    );
    Ok(Outcome::Pass)
}

/// The spec's "Split tab shows a gap, not a divider": a tab split right has
/// two cards with a 6px gap in the chrome's background between their
/// borders, and nothing in the old divider's colour, along a row through
/// the headers and a row through the terminals. Focus is counted as from
/// the pointer, so the active zone's ring isn't drawn in the gap.
pub fn zone_gap(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let out = Run::new(ctx, &server, "run")
        .profile("Alt")
        .config(ZONE_PROFILE)
        .action("split_zone_right")
        .modality("pointer")
        .capture_at(step_time(2) + 1.0)
        .dump_at(step_time(2) + 1.5)
        .run()?;
    ensure!(out.dump.zones.as_deref() == Some("count=2 active=2"), "zones {:?}", out.dump.zones);
    let image = image(&out)?;
    let grid = out.dump.grid()?;
    let s = grid.scale;
    // The right zone's card (the dump's grid is the active zone's) and the
    // left one's border, a gap to its left. The scan takes 2px of each
    // zone, clear of the left header's buttons.
    let right = grid.x - PADDING - CARD_BORDER;
    let range = (dev(right - ZONE_GAP - CARD_BORDER - 2.0, s), dev(right + 3.0, s));
    let header_middle = grid.y - PADDING - HEADER_HEIGHT / 2.0;
    for (what, y) in [("headers", header_middle), ("terminals", grid.y + grid.line_height * 4.5)] {
        let row = Line::Row(dev(y, s));
        let runs = border_runs(image, what, row, range)?;
        let found = || format!("{runs:?} along {row:?} {range:?}; found {}", row.summary(image, range));
        ensure!(runs.len() == 2, "{what}: want two borders, got {}", found());
        let [(a0, a1), (b0, b1)] = [runs[0], runs[1]];
        ensure!(
            near_px(a1 - a0, dev(CARD_BORDER, s)) && near_px(b1 - b0, dev(CARD_BORDER, s)),
            "{what}: borders aren't 1px: {}",
            found()
        );
        ensure!(near_px(b0, dev(right, s)), "{what}: the right card's border isn't at {}: {}", dev(right, s), found());
        ensure!(near_px(b0 - a1, dev(ZONE_GAP, s)), "{what}: the gap is {} px: {}", b0 - a1, found());
        expect_gap(image, &format!("{what}' gap"), row, (a1, b0))?;
    }
    Ok(Outcome::Pass)
}

/// The focus ring between two captures, anywhere in the window.
fn ring(shots: &Shots, ctx: &Ctx, what: &str, pair: (&str, &str), scale: f32) -> Result<Changes> {
    shots
        .compare(ctx, pair, |a, b| {
            let (w, h) = (a.width as f32 / scale, a.height as f32 / scale);
            ring_between(a, b, scale, (100.0, w), (100.0, h))
        })
        .map_err(|e| format!("{what}: {e}"))
}

/// The spec's "Keyboard focus moves between zones", "Pointer focus" and
/// "One zone", and the `ui-theme-tokens` "Zone ring stays on the tokens":
/// in a tab split right, the active (right) zone's card has the ring while
/// focus counts as from the keyboard, and loses it from the pointer;
/// alt-left moves focus and the ring to the left card. Each ring lies in
/// the gap round its card, the two meeting in the gap between the cards
/// without overlapping. The ring is the global theme's ring token (its
/// accent, #ff8800, at half opacity over the gap's chrome background:
/// orange), not the zone
/// theme's blue accent. A tab of one zone shows no ring either way.
pub fn zone_ring(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    let shots = Shots::default();
    let run = Run::new(ctx, &server, "split").profile("Alt").config(ZONE_PROFILE).action("split_zone_right");
    let run = shots.after(run.modality("keyboard"), ctx, 2, "right-key");
    let run = shots.after(run.modality("pointer"), ctx, 3, "right-pointer");
    let run = shots.after(run.key("alt-left"), ctx, 4, "left-key");
    let run = shots.after(run.modality("pointer"), ctx, 5, "left-pointer");
    let out = run.dump_at(step_time(5) + 0.9).run()?;
    ensure!(out.dump.zones.as_deref() == Some("count=2 active=1"), "zones after alt-left {:?}", out.dump.zones);
    ensure!(
        out.dump.focus.as_deref() == Some("terminal") && out.dump.modality.as_deref() == Some("pointer"),
        "focus {:?} modality {:?}",
        out.dump.focus,
        out.dump.modality
    );
    let grid = out.dump.grid()?;
    let s = grid.scale;
    let right = ring(&shots, ctx, "the right zone's ring", ("right-key", "right-pointer"), s)?;
    let left = ring(&shots, ctx, "the left zone's ring", ("left-key", "left-pointer"), s)?;
    let (r, l) = (right.bbox.ok_or("no right ring")?, left.bbox.ok_or("no left ring")?);

    // The left card (the dump's grid is its terminal's, under its header),
    // and its ring 3px outside its border.
    let (card_left, card_top) = (grid.x - PADDING - CARD_BORDER, grid.y - PADDING - HEADER_HEIGHT - CARD_BORDER);
    let (want_x, want_y) = (dev(card_left - 3.0, s), dev(card_top - 3.0, s));
    ensure!(
        l.x0.abs_diff(want_x) <= dev(1.5, s) && l.y0.abs_diff(want_y) <= dev(1.5, s),
        "the left ring's box {l:?} doesn't start 3px outside the card at {want_x},{want_y}"
    );
    ensure!(
        r.x0 + 1 >= l.x1 && r.x0 <= l.x1 + dev(2.0, s),
        "the rings don't meet in the gap without overlapping: left {l:?}, right {r:?}"
    );
    ensure!(
        r.y0.abs_diff(l.y0) <= 1 && r.y1.abs_diff(l.y1) <= 1 && (r.x1 - r.x0).abs_diff(l.x1 - l.x0) <= dev(2.0, s),
        "the rings differ in size: left {l:?}, right {r:?}"
    );
    for (what, changes) in [("left", left), ("right", right)] {
        let color = changes.color.ok_or("no ring colour")?;
        ensure!(
            color.0 > color.2.saturating_add(60) && color.0 > color.1,
            "the {what} ring is {}, want the global ring token's orange (about #854b08), not the zone theme's {}",
            color.hex(),
            ZONE_ACCENT.hex()
        );
    }

    // One zone: the margin round the card is the same from the keyboard
    // and from the pointer.
    let shots = Shots::default();
    let run = Run::new(ctx, &server, "one-zone").profile("Alt").config(ZONE_PROFILE);
    let run = shots.after(run.modality("keyboard"), ctx, 1, "one-key");
    let run = shots.after(run.modality("pointer"), ctx, 2, "one-pointer");
    let out = run.dump_at(step_time(2) + 0.9).run()?;
    ensure!(out.dump.zones.is_none(), "one zone, but --- zones {:?}", out.dump.zones);
    let grid = out.dump.grid()?;
    let s = grid.scale;
    let (card_left, card_top) = (dev(grid.x - PADDING - CARD_BORDER, s), dev(grid.y - PADDING - CARD_BORDER, s));
    shots.compare(ctx, ("one-key", "one-pointer"), |a, b| {
        let top = PxRect { x0: 0, y0: card_top.saturating_sub(dev(ZONE_GAP, s)), x1: a.width, y1: card_top };
        let side = PxRect { x0: 0, y0: card_top, x1: card_left, y1: a.height };
        for (what, r) in [("above", top), ("left of", side)] {
            let changes = a.changes(b, r, 16)?;
            ensure!(changes.count == 0, "one zone: a ring {what} the card: {changes:?}");
        }
        Ok(())
    })?;
    Ok(Outcome::Pass)
}

/// The spec's "Window card" and "Size reported to tmux": a tmux window of
/// two panes side by side, in a profile theme of `ALT_FOREGROUND` on
/// `ALT_BACKGROUND` (`tmux-alt`), is one card. Along a row through both
/// panes there are exactly two runs of the card's border colour, its left
/// and right edges, each with the 6px margin outside it, and none at the
/// divider between the panes. The size Brindle reports to tmux is the
/// card's inner area less the padding, in whole cells.
pub fn tmux_window(ctx: &Ctx) -> Result<Outcome> {
    let server = server()?;
    server.run(&["split-window", "-h", "-t", "e2e"])?;
    for pane in ["%0", "%1"] {
        server.type_line(pane, "clear")?;
    }
    ctx.sleep(0.5);
    let out = Run::new(ctx, &server, "run")
        .profile("tmux-alt")
        .config(&styles::alt_profile(&server))
        .record_at(3.0, "size", &["display-message", "-p", "-t", "e2e", "#{window_width} #{window_height}"])
        .capture_at(3.5)
        .dump_at(5.0)
        .run()?;
    let image = image(&out)?;
    let grid = out.dump.grid()?;
    let s = grid.scale;
    // The window's first cell, from the active pane's.
    let [left, top, ..] = server.geometry(&server.active_pane()?)?;
    let (x, y) = (grid.x - grid.cell_width * left as f32, grid.y - grid.line_height * top as f32);
    let card_left = x - PADDING - CARD_BORDER;

    let row = Line::Row(dev(y + grid.line_height * 5.5, s));
    let range = (0, image.width);
    let runs = border_runs(image, "a row through both panes", row, range)?;
    let found = || format!("{runs:?} along {row:?}; found {}", row.summary(image, range));
    ensure!(runs.len() == 2, "want the card's two edges and no border per pane, got {}", found());
    let [(a0, a1), (b0, b1)] = [runs[0], runs[1]];
    ensure!(
        near_px(a1 - a0, dev(CARD_BORDER, s)) && near_px(b1 - b0, dev(CARD_BORDER, s)),
        "borders aren't 1px: {}",
        found()
    );
    ensure!(near_px(a0, dev(card_left, s)), "the card's left edge isn't at {}: {}", dev(card_left, s), found());
    let gap = dev(ZONE_GAP, s);
    ensure!(a0 + 1 >= gap && near_px(image.width - b1, gap), "the margins aren't 6px: {}", found());
    expect_gap(image, "left margin", row, (a0 + 1 - gap.min(a0 + 1), a0))?;
    expect_gap(image, "right margin", row, (b1, image.width))?;

    // The card's inner area, from the window's first cell less the padding
    // to the window's edges less the margin and the border; the grid is
    // that less the padding, in whole cells (`fit_grid`).
    let (width, height) = (image.width as f32 / s, image.height as f32 / s);
    let inner = (width - CARD_INSET - (x - PADDING), height - CARD_INSET - (y - PADDING));
    let want = (
        ((inner.0 - 2.0 * PADDING) / grid.cell_width).floor() as u16,
        ((inner.1 - 2.0 * PADDING) / grid.line_height).floor() as u16,
    );
    let size = Run::recorded(ctx, "run", "size")?;
    let got = size
        .trim()
        .split_once(' ')
        .and_then(|(w, h)| Some((w.parse::<u16>().ok()?, h.parse::<u16>().ok()?)))
        .ok_or(format!("tmux window size {size:?}"))?;
    ensure!(got == want, "tmux's window is {got:?} cells, want {want:?} from the card's inner {inner:?} px");
    Ok(Outcome::Pass)
}
