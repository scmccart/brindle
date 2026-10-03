//! A tab showing one tmux window: its panes laid out on the cell grid
//! exactly as tmux arranged them, with native dividers.

use std::collections::HashMap;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, AppContext as _, Bounds, Context, Entity, FocusHandle, Hsla, InteractiveElement,
    IntoElement, ParentElement, Pixels, Render, Styled, Subscription, Window, canvas, div, fill,
    point, px, size,
};

use crate::actions::*;
use crate::settings::Settings;
use crate::terminal::Terminal;
use crate::terminal_element::cell_metrics;
use crate::terminal_view::TerminalView;
use crate::theme::Theme;
use crate::tmux::layout::Rect;
use crate::tmux::protocol::{PaneId, WindowId};
use crate::tmux::{Direction, TmuxSession};

pub struct TmuxWindowView {
    session: Entity<TmuxSession>,
    pub window_id: WindowId,
    pub theme: Theme,
    font_size_override: Option<f32>,
    panes: HashMap<PaneId, (Entity<TerminalView>, Subscription)>,
    focus_handle: FocusHandle,
    /// Active pane as of the last render, to follow tmux-driven focus changes.
    last_active: Option<PaneId>,
    _subscriptions: Vec<Subscription>,
}

impl TmuxWindowView {
    pub fn new(
        session: Entity<TmuxSession>,
        window_id: WindowId,
        theme: Theme,
        font_size_override: Option<f32>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![cx.observe(&session, |_, _, cx| cx.notify())];
        Self {
            session,
            window_id,
            theme,
            font_size_override,
            panes: HashMap::new(),
            focus_handle: cx.focus_handle(),
            last_active: None,
            _subscriptions: subscriptions,
        }
    }

    pub fn title(&self, cx: &App) -> String {
        let session = self.session.read(cx);
        session
            .windows
            .get(&self.window_id)
            .map(|w| w.name.clone())
            .unwrap_or_else(|| session.session_name.clone())
    }

    fn active_pane(&self, cx: &App) -> Option<PaneId> {
        self.session.read(cx).active_pane_of(Some(self.window_id))
    }

    pub fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.active_pane(cx)
            .and_then(|id| self.panes.get(&id))
            .map(|(view, _)| view.read(cx).focus_handle.clone())
            .unwrap_or_else(|| self.focus_handle.clone())
    }

    pub fn active_terminal(&self, cx: &App) -> Option<Entity<Terminal>> {
        self.session.read(cx).pane_terminal(self.active_pane(cx)?)
    }

    pub fn set_theme(&mut self, theme: Theme, cx: &mut Context<Self>) {
        self.theme = theme.clone();
        for (view, _) in self.panes.values() {
            let theme = theme.clone();
            view.update(cx, |view, cx| view.terminal.update(cx, |t, _| t.theme = theme));
        }
        cx.notify();
    }

    /// Creates views for new panes and drops views for closed ones.
    fn sync_panes(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Vec<(PaneId, Rect)> {
        let session = self.session.read(cx);
        let layout_panes = session
            .windows
            .get(&self.window_id)
            .and_then(|w| w.layout.as_ref())
            .map(|l| l.panes())
            .unwrap_or_default();
        let terminals: Vec<(PaneId, Option<Entity<Terminal>>)> =
            layout_panes.iter().map(|(id, _)| (*id, session.pane_terminal(*id))).collect();

        self.panes.retain(|id, _| layout_panes.iter().any(|(p, _)| p == id));
        for (id, terminal) in terminals {
            if self.panes.contains_key(&id) {
                continue;
            }
            let Some(terminal) = terminal else { continue };
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
            self.panes.insert(id, (view, subscription));
        }
        layout_panes
    }

    fn split_right(&mut self, _: &TmuxSplitRight, _: &mut Window, cx: &mut Context<Self>) {
        let id = self.window_id;
        self.session.update(cx, |s, _| s.split(id, true));
    }

    fn split_down(&mut self, _: &TmuxSplitDown, _: &mut Window, cx: &mut Context<Self>) {
        let id = self.window_id;
        self.session.update(cx, |s, _| s.split(id, false));
    }

    fn close_pane(&mut self, _: &TmuxClosePane, _: &mut Window, cx: &mut Context<Self>) {
        let id = self.window_id;
        self.session.update(cx, |s, _| s.kill_pane(id));
    }

    fn zoom_pane(&mut self, _: &TmuxZoomPane, _: &mut Window, cx: &mut Context<Self>) {
        let id = self.window_id;
        self.session.update(cx, |s, _| s.toggle_zoom(id));
    }

    fn focus(&mut self, direction: Direction, cx: &mut Context<Self>) {
        let id = self.window_id;
        self.session.update(cx, |s, _| s.focus_direction(id, direction));
    }
}

impl Render for TmuxWindowView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panes = self.sync_panes(window, cx);
        let active = self.active_pane(cx);
        let metrics = cell_metrics(self.font_size_override, window, cx);
        let (cw, lh) = (metrics.cell_width, metrics.line_height);
        let padding = px(Settings::get(cx).config.padding);
        let zoomed = self.session.read(cx).windows.get(&self.window_id).is_some_and(|w| w.zoomed);
        let dividers = self
            .session
            .read(cx)
            .windows
            .get(&self.window_id)
            .and_then(|w| w.layout.as_ref())
            .map(|l| l.dividers())
            .unwrap_or_default();

        // Follow focus when tmux changes the active pane (select-pane, a pane
        // closing, …) while this tab has focus.
        if active != self.last_active {
            self.last_active = active;
            let has_focus = self.focus_handle.contains_focused(window, cx)
                || self.panes.values().any(|(v, _)| v.read(cx).focus_handle.is_focused(window));
            if has_focus && let Some((view, _)) = active.and_then(|id| self.panes.get(&id)) {
                window.focus(&view.read(cx).focus_handle);
            }
        }

        let active_rect = panes.iter().find(|(id, _)| Some(*id) == active).map(|(_, r)| *r);
        let theme = self.theme.clone();
        let session = self.session.clone();
        let multiple = panes.len() > 1;

        let pane_elements = panes.iter().filter_map(|(id, rect)| {
            let (view, _) = self.panes.get(id)?;
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
            .on_action(cx.listener(Self::split_right))
            .on_action(cx.listener(Self::split_down))
            .on_action(cx.listener(Self::close_pane))
            .on_action(cx.listener(Self::zoom_pane))
            .on_action(cx.listener(|this, _: &TmuxFocusLeft, _, cx| this.focus(Direction::Left, cx)))
            .on_action(cx.listener(|this, _: &TmuxFocusRight, _, cx| this.focus(Direction::Right, cx)))
            .on_action(cx.listener(|this, _: &TmuxFocusUp, _, cx| this.focus(Direction::Up, cx)))
            .on_action(cx.listener(|this, _: &TmuxFocusDown, _, cx| this.focus(Direction::Down, cx)))
            .relative()
            .size_full()
            .bg(theme.background.hsla())
            .child(
                canvas(
                    move |bounds, _, cx| {
                        // The client size tells tmux how much room its windows get.
                        let avail = bounds.size - size(padding * 2.0, padding * 2.0);
                        let cols = (avail.width / cw).floor().max(2.0) as u16;
                        let rows = (avail.height / lh).floor().max(2.0) as u16;
                        session.update(cx, |s, _| s.set_client_size(cols, rows));
                    },
                    move |bounds, _, window, _| {
                        let origin = bounds.origin + point(padding, padding);
                        paint_dividers(&dividers, active_rect.filter(|_| multiple), origin, cw, lh, &theme, window);
                    },
                )
                .absolute()
                .size_full(),
            )
            .children(pane_elements)
            .when(zoomed, |d| {
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
    let normal = theme.chrome_border().mix(theme.foreground, 0.15).hsla();
    let accent = theme.accent().hsla();
    for d in dividers {
        let vertical = d.width == 1 && d.height > 1;
        let segments = divider_segments(d, vertical, active);
        for (start, end, highlighted) in segments {
            let color: Hsla = if highlighted { accent } else { normal };
            let bounds = if vertical {
                let x = (origin.x + cw * d.x as f32 + cw / 2.0).floor();
                let y0 = origin.y + lh * start as f32;
                let y1 = origin.y + lh * end as f32;
                Bounds::new(point(x, y0), size(px(1.0), y1 - y0))
            } else {
                let y = (origin.y + lh * d.y as f32 + lh / 2.0).floor();
                let x0 = origin.x + cw * start as f32;
                let x1 = origin.x + cw * end as f32;
                Bounds::new(point(x0, y), size(x1 - x0, px(1.0)))
            };
            window.paint_quad(fill(bounds, color));
        }
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
