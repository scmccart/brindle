//! A tab showing one tmux window: its panes laid out on the cell grid
//! exactly as tmux arranged them, with native dividers.

use std::collections::HashMap;
use std::rc::Rc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    App, AppContext as _, Bounds, Context, Entity, EventEmitter, FocusHandle, Hsla, InteractiveElement,
    IntoElement, ParentElement, Pixels, Render, Styled, Subscription, Window, canvas, div, fill,
    point, px, size,
};

use crate::actions::*;
use crate::picker::PromptRequest;
use crate::settings::Settings;
use crate::terminal::Terminal;
use crate::terminal_element::{cell_metrics, fit_grid};
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
    views: HashMap<PaneId, (Entity<TerminalView>, Subscription)>,
    /// Pane rects and dividers from the window's layout, refreshed whenever
    /// the session changes rather than on every frame.
    panes: Vec<(PaneId, Rect)>,
    dividers: Vec<Rect>,
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
            dividers: Vec::new(),
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
        self.dividers = layout.map(|l| l.dividers()).unwrap_or_default();
        self.zoomed = tmux_window.is_some_and(|w| w.zoomed);
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

    fn rename_window(&mut self, _: &TmuxRenameWindow, _: &mut Window, cx: &mut Context<Self>) {
        let (session, window) = (self.session.clone(), self.window_id);
        let initial = session.read(cx).window(window).map(|w| w.name.clone()).unwrap_or_default();
        cx.emit(PromptRequest {
            label: "Rename tmux window".into(),
            initial,
            submit: Rc::new(move |name, cx| {
                session.update(cx, |s, _| s.rename_window(window, &name));
                let (tx, rx) = futures::channel::oneshot::channel();
                tx.send(Ok(())).ok();
                rx
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

        let pane_elements = self.panes.iter().filter_map(|(id, rect)| {
            let (view, _) = self.views.get(id)?;
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
            .on_action(cx.listener(|this, _: &TmuxDetach, _, cx| this.with_session(cx, |s, _| s.detach())))
            .on_action(cx.listener(|this, _: &TmuxSplitRight, _, cx| this.with_session(cx, |s, w| s.split(w, true))))
            .on_action(cx.listener(|this, _: &TmuxSplitDown, _, cx| this.with_session(cx, |s, w| s.split(w, false))))
            .on_action(cx.listener(|this, _: &TmuxClosePane, _, cx| this.with_session(cx, |s, w| s.kill_pane(w))))
            .on_action(cx.listener(|this, _: &TmuxZoomPane, _, cx| this.with_session(cx, |s, w| s.toggle_zoom(w))))
            .on_action(cx.listener(|this, _: &TmuxNewWindow, _, cx| this.with_session(cx, |s, _| s.new_window())))
            .on_action(cx.listener(|this, _: &TmuxLayoutEvenHorizontal, _, cx| {
                this.with_session(cx, |s, w| s.select_layout(w, "even-horizontal"))
            }))
            .on_action(cx.listener(|this, _: &TmuxLayoutEvenVertical, _, cx| {
                this.with_session(cx, |s, w| s.select_layout(w, "even-vertical"))
            }))
            .on_action(cx.listener(|this, _: &TmuxLayoutMainVertical, _, cx| {
                this.with_session(cx, |s, w| s.select_layout(w, "main-vertical"))
            }))
            .on_action(cx.listener(|this, _: &TmuxLayoutTiled, _, cx| {
                this.with_session(cx, |s, w| s.select_layout(w, "tiled"))
            }))
            .on_action(cx.listener(|this, _: &TmuxNextLayout, _, cx| this.with_session(cx, |s, w| s.next_layout(w))))
            .on_action(cx.listener(|this, _: &TmuxRotatePanes, _, cx| this.with_session(cx, |s, w| s.rotate(w))))
            .on_action(cx.listener(|this, _: &TmuxSwapPanePrev, _, cx| this.with_session(cx, |s, w| s.swap_pane(w, true))))
            .on_action(cx.listener(|this, _: &TmuxSwapPaneNext, _, cx| this.with_session(cx, |s, w| s.swap_pane(w, false))))
            .on_action(cx.listener(Self::rename_window))
            .on_action(cx.listener(Self::run_command))
            .on_action(cx.listener(|this, _: &TmuxBreakPane, _, cx| this.with_session(cx, |s, w| s.break_pane(w))))
            .on_action(cx.listener(|this, _: &TmuxFocusLeft, _, cx| {
                this.with_session(cx, |s, w| s.focus_direction(w, Direction::Left))
            }))
            .on_action(cx.listener(|this, _: &TmuxFocusRight, _, cx| {
                this.with_session(cx, |s, w| s.focus_direction(w, Direction::Right))
            }))
            .on_action(cx.listener(|this, _: &TmuxFocusUp, _, cx| {
                this.with_session(cx, |s, w| s.focus_direction(w, Direction::Up))
            }))
            .on_action(cx.listener(|this, _: &TmuxFocusDown, _, cx| {
                this.with_session(cx, |s, w| s.focus_direction(w, Direction::Down))
            }))
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
                    move |bounds, _, window, _| {
                        let origin = bounds.origin + point(padding, padding);
                        paint_dividers(&dividers, active_rect.filter(|_| multiple), origin, cw, lh, &theme, window);
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
