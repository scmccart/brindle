//! A tab showing one tmux window: its panes laid out on the cell grid
//! exactly as tmux arranged them, with native dividers.

use std::collections::HashMap;
use std::rc::Rc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, AppContext as _, Bounds, Context, Entity, EventEmitter, FocusHandle, FontWeight, Hsla,
    InteractiveElement, IntoElement, ParentElement, Pixels, Render, ShapedLine, SharedString, Styled,
    Subscription, TextRun, Window, canvas, div, fill, point, px, size,
};
use unicode_width::UnicodeWidthChar as _;

use crate::actions::*;
use crate::picker::{PromptRequest, ready};
use crate::settings::Settings;
use crate::terminal::Terminal;
use crate::terminal_element::{CellMetrics, cell_metrics, fit_grid};
use crate::terminal_view::TerminalView;
use crate::theme::Theme;
use crate::tmux::layout::Rect;
use crate::tmux::protocol::{PaneId, WindowId};
use crate::tmux::style::{StyledRun, TmuxColor};
use crate::tmux::{BorderStatus, Direction, TmuxSession};

pub struct TmuxWindowView {
    session: Entity<TmuxSession>,
    pub window_id: WindowId,
    pub theme: Theme,
    font_size_override: Option<f32>,
    views: HashMap<PaneId, (Entity<TerminalView>, Subscription)>,
    /// Pane cells and dividers from the window's layout, refreshed whenever
    /// the session changes rather than on every frame.
    panes: Vec<(PaneId, Rect)>,
    /// Where each pane sits in its cell; tmux may keep rows of the cell for
    /// a border status line.
    pane_rects: HashMap<PaneId, Rect>,
    dividers: Vec<Rect>,
    /// Where title lines go (`pane-border-status`), and the window's height.
    border_status: BorderStatus,
    window_rows: u16,
    /// Shaped title lines, rebuilt only when what they show changes.
    titles: HashMap<PaneId, (TitleKey, Vec<TitleRun>)>,
    active: Option<PaneId>,
    zoomed: bool,
    focus_handle: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

/// Prompted commands ask the workspace to show the prompt.
impl EventEmitter<PromptRequest> for TmuxWindowView {}

impl TmuxWindowView {
    pub fn new(
        session: Entity<TmuxSession>,
        window_id: WindowId,
        theme: Theme,
        font_size_override: Option<f32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions =
            vec![cx.observe_in(&session, window, |this, _, window, cx| this.sync(window, cx))];
        let mut this = Self {
            session,
            window_id,
            theme,
            font_size_override,
            views: HashMap::new(),
            panes: Vec::new(),
            pane_rects: HashMap::new(),
            dividers: Vec::new(),
            border_status: BorderStatus::Off,
            window_rows: 0,
            titles: HashMap::new(),
            active: None,
            zoomed: false,
            focus_handle: cx.focus_handle(),
            _subscriptions: subscriptions,
        };
        this.sync(window, cx);
        this
    }

    pub fn title(&self, cx: &App) -> String {
        let session = self.session.read(cx);
        session
            .window(self.window_id)
            .map(|w| w.name.clone())
            .unwrap_or_else(|| session.session_name.clone())
    }

    pub fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.active
            .and_then(|id| self.views.get(&id))
            .map(|(view, _)| view.read(cx).focus_handle.clone())
            .unwrap_or_else(|| self.focus_handle.clone())
    }

    pub fn active_terminal(&self, cx: &App) -> Option<Entity<Terminal>> {
        self.session.read(cx).pane_terminal(self.active?)
    }

    /// Applies a reloaded profile (theme and font size).
    pub fn set_profile(&mut self, theme: Theme, font_size: Option<f32>, cx: &mut Context<Self>) {
        self.theme = theme.clone();
        self.font_size_override = font_size;
        self.titles.clear();
        for (view, _) in self.views.values() {
            let theme = theme.clone();
            view.update(cx, |view, cx| {
                view.font_size_override = font_size;
                view.terminal.update(cx, |t, _| t.theme = theme)
            });
        }
        cx.notify();
    }

    /// Mirrors the session: creates views for new panes, drops closed ones,
    /// and moves focus when tmux changes the active pane.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let session = self.session.read(cx);
        let tmux_window = session.window(self.window_id);
        let layout = tmux_window.and_then(|w| w.layout.as_ref());
        self.panes = layout.map(|l| l.panes()).unwrap_or_default();
        self.pane_rects = self.panes.iter().map(|&(id, cell)| (id, session.pane_rect(id, cell))).collect();
        self.dividers = layout.map(|l| l.dividers()).unwrap_or_default();
        self.zoomed = tmux_window.is_some_and(|w| w.zoomed);
        self.border_status = tmux_window.map_or(BorderStatus::Off, |w| w.border_status);
        self.window_rows = layout.map_or(0, |l| l.rect().height);
        let active = session.active_pane_of(self.window_id);
        let new_terminals: Vec<(PaneId, Entity<Terminal>)> = self
            .panes
            .iter()
            .filter(|(id, _)| !self.views.contains_key(id))
            .filter_map(|(id, _)| Some((*id, session.pane_terminal(*id)?)))
            .collect();

        let panes = &self.panes;
        self.views.retain(|id, _| panes.iter().any(|(p, _)| p == id));
        for (id, terminal) in new_terminals {
            let font_size = self.font_size_override;
            let view = cx.new(|cx| {
                let mut view = TerminalView::new(terminal, font_size, window, cx);
                view.fixed_size = true;
                view
            });
            let focus = view.read(cx).focus_handle.clone();
            let session = self.session.clone();
            let subscription = cx.on_focus_in(&focus, window, move |_, _, cx| {
                session.update(cx, |s, cx| s.select_pane(id, cx));
            });
            self.views.insert(id, (view, subscription));
        }

        // Follow tmux-driven focus changes (select-pane, a pane closing, …)
        // while this tab has focus.
        if active != self.active {
            let had_focus = self.focus_handle.contains_focused(window, cx)
                || self.views.values().any(|(v, _)| v.read(cx).focus_handle.is_focused(window));
            self.active = active;
            if had_focus {
                window.focus(&self.focus_handle(cx));
            }
        }
        cx.notify();
    }

    fn with_session(&self, cx: &mut Context<Self>, f: impl FnOnce(&mut TmuxSession, WindowId)) {
        let id = self.window_id;
        self.session.update(cx, |s, _| f(s, id));
    }

    /// A listener for an action that runs `f` against this tab's window.
    fn on_session<A>(f: fn(&mut TmuxSession, WindowId)) -> impl Fn(&mut Self, &A, &mut Window, &mut Context<Self>) {
        move |this, _, _, cx| this.with_session(cx, f)
    }

    fn rename_window(&mut self, _: &TmuxRenameWindow, _: &mut Window, cx: &mut Context<Self>) {
        let (session, window) = (self.session.clone(), self.window_id);
        let initial = session.read(cx).window(window).map(|w| w.name.clone()).unwrap_or_default();
        cx.emit(PromptRequest {
            label: "Rename tmux window".into(),
            initial,
            submit: Rc::new(move |name, cx| {
                session.update(cx, |s, _| s.rename_window(window, &name));
                ready(Ok(()))
            }),
        });
    }

    fn run_command(&mut self, _: &TmuxCommand, _: &mut Window, cx: &mut Context<Self>) {
        let session = self.session.clone();
        cx.emit(PromptRequest {
            label: "tmux command".into(),
            initial: String::new(),
            submit: Rc::new(move |line, cx| session.update(cx, |s, _| s.run_user_command(&line))),
        });
    }
}

/// What a pane's title line shows; its shaped runs are reused until it changes.
struct TitleKey {
    runs: Vec<StyledRun>,
    active: bool,
    columns: u16,
    font_size: Pixels,
}

/// One shaped piece of a title line, `col` cells from the line's start.
#[derive(Clone)]
struct TitleRun {
    col: u16,
    cols: u16,
    background: Hsla,
    line: ShapedLine,
}

/// A title line ready to paint, in window cells.
struct TitleLine {
    row: u16,
    col: u16,
    /// Columns of a reserved row inside the pane's own cell, which no
    /// divider crosses, so a border line is drawn there (`x`, width).
    border: Option<(u16, u16)>,
    border_color: Hsla,
    runs: Vec<TitleRun>,
}

impl TmuxWindowView {
    /// Title lines for the visible panes, as tmux draws them for terminal
    /// clients when `pane-border-status` is on.
    fn title_lines(&mut self, metrics: &CellMetrics, window: &Window, cx: &App) -> Vec<TitleLine> {
        if self.border_status == BorderStatus::Off {
            self.titles.clear();
            return Vec::new();
        }
        let session = self.session.read(cx);
        let theme = &self.theme;
        let (normal, accent) = border_colors(theme);
        let mut lines = Vec::new();
        for &(id, cell) in &self.panes {
            let Some(&rect) = self.pane_rects.get(&id) else { continue };
            let Some((row, col, columns)) = title_span(rect, self.border_status, self.window_rows) else {
                continue;
            };
            let active = Some(id) == self.active;
            let title = session.pane_title(id);
            let runs = match self.titles.get(&id) {
                Some((k, runs))
                    if k.runs == title && k.active == active && k.columns == columns && k.font_size == metrics.font_size =>
                {
                    runs.clone()
                }
                _ => {
                    let key = TitleKey { runs: title.to_vec(), active, columns, font_size: metrics.font_size };
                    let runs = shape_title(&key, theme, metrics, window);
                    self.titles.insert(id, (key, runs.clone()));
                    runs
                }
            };
            let inside_cell = row >= cell.y && row < cell.y + cell.height;
            lines.push(TitleLine {
                row,
                col,
                border: inside_cell.then_some((rect.x, rect.width)),
                border_color: if active { accent } else { normal },
                runs,
            });
        }
        let panes = &self.panes;
        self.titles.retain(|id, _| panes.iter().any(|(p, _)| p == id));
        lines
    }
}

/// Where tmux puts a pane's title line: the row above (`top`) or below
/// (`bottom`) the pane, from two columns in to the pane's right edge.
/// Returns `(row, first column, columns)`.
fn title_span(pane: Rect, status: BorderStatus, window_rows: u16) -> Option<(u16, u16, u16)> {
    let row = match status {
        BorderStatus::Off => return None,
        BorderStatus::Top => pane.y.checked_sub(1)?,
        BorderStatus::Bottom => pane.y + pane.height,
    };
    let columns = pane.width.saturating_sub(2);
    (row < window_rows && columns > 0).then_some((row, pane.x + 2, columns))
}

/// Shapes a title's runs, clipped to its columns. Runs without a color use
/// the border's: the accent for the active pane, the foreground otherwise.
fn shape_title(key: &TitleKey, theme: &Theme, metrics: &CellMetrics, window: &Window) -> Vec<TitleRun> {
    let base_fg = if key.active { theme.accent() } else { theme.foreground };
    let color = |c: Option<TmuxColor>| match c {
        Some(TmuxColor::Indexed(i)) => Some(theme.indexed(i)),
        Some(TmuxColor::Rgb(c)) => Some(c),
        None => None,
    };
    let mut out = Vec::new();
    let mut col = 0;
    for run in &key.runs {
        let mut text = String::new();
        let mut cols = 0;
        for ch in run.text.chars() {
            let w = ch.width().unwrap_or(0) as u16;
            if col + cols + w > key.columns {
                break;
            }
            text.push(ch);
            cols += w;
        }
        if text.is_empty() {
            continue;
        }
        let (mut fg, mut bg) = (color(run.style.fg).unwrap_or(base_fg), color(run.style.bg).unwrap_or(theme.background));
        if run.style.reverse {
            std::mem::swap(&mut fg, &mut bg);
        }
        let mut font = metrics.font.clone();
        if run.style.bold {
            font.weight = FontWeight::BOLD;
        }
        let text_run = TextRun {
            len: text.len(),
            font,
            color: fg.hsla(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line = window.text_system().shape_line(SharedString::from(text), metrics.font_size, &[text_run], None);
        out.push(TitleRun { col, cols, background: bg.hsla(), line });
        col += cols;
    }
    out
}

fn paint_titles(titles: &[TitleLine], origin: gpui::Point<Pixels>, cw: Pixels, lh: Pixels, window: &mut Window, cx: &mut App) {
    for title in titles {
        let y = origin.y + lh * title.row as f32;
        if let Some((x, width)) = title.border {
            let line_y = (y + lh / 2.0).floor();
            let x0 = (origin.x + cw * x as f32).floor();
            let x1 = (origin.x + cw * (x + width) as f32).floor();
            window.paint_quad(fill(Bounds::new(point(x0, line_y), size(x1 - x0, px(1.0))), title.border_color));
        }
        for run in &title.runs {
            let x = origin.x + cw * (title.col + run.col) as f32;
            let (x0, x1) = (x.floor(), (x + cw * run.cols as f32).floor());
            window.paint_quad(fill(Bounds::new(point(x0, y.floor()), size(x1 - x0, lh)), run.background));
            run.line.paint(point(x, y), lh, window, cx).ok();
        }
    }
}

/// Divider colors: a muted line, and the accent next to the active pane.
fn border_colors(theme: &Theme) -> (Hsla, Hsla) {
    (theme.chrome_border().mix(theme.foreground, 0.15).hsla(), theme.accent().hsla())
}

impl Render for TmuxWindowView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let metrics = cell_metrics(self.font_size_override, window, cx);
        let (cw, lh) = (metrics.cell_width, metrics.line_height);
        let padding = px(Settings::get(cx).config.padding);
        let active_rect = self.panes.iter().find(|(id, _)| Some(*id) == self.active).map(|(_, r)| *r);
        let theme = self.theme.clone();
        let session = self.session.clone();
        let dividers = self.dividers.clone();
        let multiple = self.panes.len() > 1;
        let titles = self.title_lines(&metrics, window, cx);

        let pane_elements = self.panes.iter().filter_map(|(id, _)| {
            let (view, _) = self.views.get(id)?;
            let rect = self.pane_rects.get(id)?;
            Some(
                div()
                    .absolute()
                    .left(padding + cw * rect.x as f32)
                    .top(padding + lh * rect.y as f32)
                    .w(cw * rect.width as f32)
                    .h(lh * rect.height as f32)
                    .child(view.clone()),
            )
        });

        div()
            .key_context("TmuxWindow")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::on_session::<TmuxDetach>(|s, _| s.detach())))
            .on_action(cx.listener(Self::on_session::<TmuxSplitRight>(|s, w| s.split(w, true))))
            .on_action(cx.listener(Self::on_session::<TmuxSplitDown>(|s, w| s.split(w, false))))
            .on_action(cx.listener(Self::on_session::<TmuxClosePane>(|s, w| s.kill_pane(w))))
            .on_action(cx.listener(Self::on_session::<TmuxZoomPane>(|s, w| s.toggle_zoom(w))))
            .on_action(cx.listener(Self::on_session::<TmuxNewWindow>(|s, _| s.new_window())))
            .on_action(cx.listener(Self::on_session::<TmuxLayoutEvenHorizontal>(|s, w| {
                s.select_layout(w, "even-horizontal")
            })))
            .on_action(cx.listener(Self::on_session::<TmuxLayoutEvenVertical>(|s, w| s.select_layout(w, "even-vertical"))))
            .on_action(cx.listener(Self::on_session::<TmuxLayoutMainVertical>(|s, w| s.select_layout(w, "main-vertical"))))
            .on_action(cx.listener(Self::on_session::<TmuxLayoutTiled>(|s, w| s.select_layout(w, "tiled"))))
            .on_action(cx.listener(Self::on_session::<TmuxNextLayout>(|s, w| s.next_layout(w))))
            .on_action(cx.listener(Self::on_session::<TmuxRotatePanes>(|s, w| s.rotate(w))))
            .on_action(cx.listener(Self::on_session::<TmuxSwapPanePrev>(|s, w| s.swap_pane(w, true))))
            .on_action(cx.listener(Self::on_session::<TmuxSwapPaneNext>(|s, w| s.swap_pane(w, false))))
            .on_action(cx.listener(Self::on_session::<TmuxBreakPane>(|s, w| s.break_pane(w))))
            .on_action(cx.listener(Self::on_session::<TmuxFocusLeft>(|s, w| s.focus_direction(w, Direction::Left))))
            .on_action(cx.listener(Self::on_session::<TmuxFocusRight>(|s, w| s.focus_direction(w, Direction::Right))))
            .on_action(cx.listener(Self::on_session::<TmuxFocusUp>(|s, w| s.focus_direction(w, Direction::Up))))
            .on_action(cx.listener(Self::on_session::<TmuxFocusDown>(|s, w| s.focus_direction(w, Direction::Down))))
            .on_action(cx.listener(Self::rename_window))
            .on_action(cx.listener(Self::run_command))
            .relative()
            .size_full()
            .bg(theme.background.hsla())
            .child(
                canvas(
                    move |bounds, _, cx| {
                        // The client size tells tmux how much room its windows get.
                        let (cols, rows) = fit_grid(bounds.size, padding, cw, lh);
                        session.update(cx, |s, _| s.set_client_size(cols as u16, rows as u16));
                    },
                    move |bounds, _, window, cx| {
                        let origin = bounds.origin + point(padding, padding);
                        paint_dividers(&dividers, active_rect.filter(|_| multiple), origin, cw, lh, &theme, window);
                        paint_titles(&titles, origin, cw, lh, window, cx);
                    },
                )
                .absolute()
                .size_full(),
            )
            .children(pane_elements)
            .when(self.zoomed, |d| {
                d.child(
                    div()
                        .absolute()
                        .top(px(4.0))
                        .right(px(8.0))
                        .px_2()
                        .rounded_md()
                        .text_xs()
                        .bg(self.theme.accent().mix(self.theme.background, 0.5).hsla())
                        .text_color(self.theme.foreground.hsla())
                        .child("zoomed"),
                )
            })
    }
}

/// Draws one-pixel divider lines centred in tmux's one-cell gaps, using the
/// accent colour where a divider borders the active pane.
fn paint_dividers(
    dividers: &[Rect],
    active: Option<Rect>,
    origin: gpui::Point<Pixels>,
    cw: Pixels,
    lh: Pixels,
    theme: &Theme,
    window: &mut Window,
) {
    let (normal, accent) = border_colors(theme);
    for (ix, d) in dividers.iter().enumerate() {
        let vertical = d.width == 1 && d.height > 1;
        // Along the divider: cell size, origin, and where a crossing line sits.
        let (cell, start) = if vertical { (lh, origin.y) } else { (cw, origin.x) };
        let line = |c: u16| (start + cell * c as f32 + cell / 2.0).floor();
        let pixel = |edge: Edge| match edge {
            Edge::Cell(c) => start + cell * c as f32,
            Edge::AtLine(c) => line(c),
            Edge::PastLine(c) => line(c) + px(1.0),
        };
        for (from, to, highlighted) in divider_segments(d, vertical, active) {
            let color: Hsla = if highlighted { accent } else { normal };
            let (p0, p1) = (pixel(divider_edge(ix, dividers, vertical, from)), pixel(divider_edge(ix, dividers, vertical, to)));
            let bounds = if vertical {
                let x = (origin.x + cw * d.x as f32 + cw / 2.0).floor();
                Bounds::new(point(x, p0), size(px(1.0), p1 - p0))
            } else {
                let y = (origin.y + lh * d.y as f32 + lh / 2.0).floor();
                Bounds::new(point(p0, y), size(p1 - p0, px(1.0)))
            };
            window.paint_quad(fill(bounds, color));
        }
    }
}

/// Where a divider segment starts or ends, along the divider.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    /// At the leading edge of cell `c`.
    Cell(u16),
    /// On the line drawn through cell `c`, including it.
    AtLine(u16),
    /// Just past the line drawn through cell `c`.
    PastLine(u16),
}

/// Where boundary `at` (a cell position along divider `ix`) is drawn. Lines
/// meet: an end that runs into another divider reaches its line, and a
/// colour change next to a crossing divider happens on that divider's line.
pub fn divider_edge(ix: usize, dividers: &[Rect], vertical: bool, at: u16) -> Edge {
    let d = dividers[ix];
    let covered = |x: i32, y: i32| {
        dividers.iter().enumerate().any(|(other, r)| {
            other != ix
                && (r.x as i32..(r.x + r.width) as i32).contains(&x)
                && (r.y as i32..(r.y + r.height) as i32).contains(&y)
        })
    };
    // Cell `c` along the divider, and its two neighbours across it.
    let on = |c: i32| if vertical { covered(d.x as i32, c) } else { covered(c, d.y as i32) };
    let crossed = |c: i32| {
        if vertical {
            covered(d.x as i32 - 1, c) || covered(d.x as i32 + 1, c)
        } else {
            covered(c, d.y as i32 - 1) || covered(c, d.y as i32 + 1)
        }
    };
    let (start, end) = if vertical { (d.y, d.y + d.height) } else { (d.x, d.x + d.width) };
    let c = at as i32;
    if at == start {
        if on(c - 1) { Edge::AtLine(at - 1) } else { Edge::Cell(at) }
    } else if at == end {
        if on(c) { Edge::PastLine(at) } else { Edge::Cell(at) }
    } else if crossed(c - 1) {
        Edge::AtLine(at - 1)
    } else if crossed(c) {
        Edge::PastLine(at)
    } else {
        Edge::Cell(at)
    }
}

/// Splits a divider into (start, end, highlighted) runs along its length,
/// highlighting the part adjacent to the active pane.
pub fn divider_segments(d: &Rect, vertical: bool, active: Option<Rect>) -> Vec<(u16, u16, bool)> {
    let (start, end) = if vertical { (d.y, d.y + d.height) } else { (d.x, d.x + d.width) };
    let Some(a) = active else { return vec![(start, end, false)] };
    let touches = if vertical {
        a.x + a.width == d.x || d.x + 1 == a.x
    } else {
        a.y + a.height == d.y || d.y + 1 == a.y
    };
    let (a_start, a_end) = if vertical { (a.y, a.y + a.height) } else { (a.x, a.x + a.width) };
    let (lo, hi) = (a_start.max(start), a_end.min(end));
    if !touches || lo >= hi {
        return vec![(start, end, false)];
    }
    let mut out = Vec::new();
    if start < lo {
        out.push((start, lo, false));
    }
    out.push((lo, hi, true));
    if hi < end {
        out.push((hi, end, false));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: u16, y: u16, width: u16, height: u16) -> Rect {
        Rect { x, y, width, height }
    }

    #[test]
    fn title_spans() {
        let pane = r(51, 1, 49, 14);
        assert_eq!(title_span(pane, BorderStatus::Off, 30), None);
        assert_eq!(title_span(pane, BorderStatus::Top, 30), Some((0, 53, 47)));
        assert_eq!(title_span(r(51, 16, 49, 13), BorderStatus::Top, 30), Some((15, 53, 47)));
        assert_eq!(title_span(r(0, 0, 50, 29), BorderStatus::Bottom, 30), Some((29, 2, 48)));
        // Zoomed: one pane covering the window, title in the reserved row.
        assert_eq!(title_span(r(0, 1, 113, 32), BorderStatus::Top, 33), Some((0, 2, 111)));
        // No row to put it in, or no room in it.
        assert_eq!(title_span(r(0, 0, 50, 30), BorderStatus::Top, 30), None);
        assert_eq!(title_span(r(0, 0, 50, 30), BorderStatus::Bottom, 30), None);
        assert_eq!(title_span(r(0, 1, 2, 10), BorderStatus::Top, 30), None);
    }

    #[test]
    fn dividers_meet_at_junctions() {
        use Edge::*;
        // Left pane full height; right column split at row 16 (a T).
        let t = [r(57, 0, 1, 33), r(58, 16, 56, 1)];
        // The vertical's ends touch nothing; its highlight for the lower
        // right pane starts on the horizontal's line, not at row 17.
        assert_eq!(divider_edge(0, &t, true, 0), Cell(0));
        assert_eq!(divider_edge(0, &t, true, 33), Cell(33));
        assert_eq!(divider_edge(0, &t, true, 17), AtLine(16));
        // The horizontal starts on the vertical's line.
        assert_eq!(divider_edge(1, &t, false, 58), AtLine(57));
        assert_eq!(divider_edge(1, &t, false, 114), Cell(114));

        // 2x2 grid: a full-width horizontal crossed by a vertical in each row.
        let grid = [r(0, 15, 114, 1), r(56, 0, 1, 15), r(56, 16, 1, 17)];
        // The top-right highlight on the horizontal starts on the verticals' line.
        assert_eq!(divider_edge(0, &grid, false, 57), AtLine(56));
        // The verticals reach the horizontal's line from above and below.
        assert_eq!(divider_edge(1, &grid, true, 15), PastLine(15));
        assert_eq!(divider_edge(2, &grid, true, 16), AtLine(15));
        // A boundary away from any junction stays on the cell edge.
        assert_eq!(divider_edge(0, &grid, false, 30), Cell(30));
    }

    #[test]
    fn highlights_the_part_next_to_the_active_pane() {
        // Left pane full height, right column split top/bottom; active = bottom right.
        let divider = r(60, 0, 1, 40);
        let active = r(61, 21, 59, 19);
        assert_eq!(divider_segments(&divider, true, Some(active)), vec![(0, 21, false), (21, 40, true)]);
        // A pane not adjacent to the divider leaves it plain.
        assert_eq!(divider_segments(&divider, true, Some(r(100, 0, 5, 5))), vec![(0, 40, false)]);
        let horizontal = r(61, 20, 59, 1);
        assert_eq!(divider_segments(&horizontal, false, Some(active)), vec![(61, 120, true)]);
    }
}
