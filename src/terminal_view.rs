//! GPUI view for a single terminal: input, mouse, IME, cursor blinking.
//! Drawing lives in [`crate::terminal_element`].

use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;
use std::time::Duration;

use alacritty_terminal::grid::Scroll;
use alacritty_terminal::index::Side;
use alacritty_terminal::selection::SelectionType;
use alacritty_terminal::term::TermMode;
use gpui::{
    App, Bounds, ClipboardItem, Context, Entity, FocusHandle, Focusable, InteractiveElement,
    IntoElement, KeyDownEvent, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement,
    Pixels, Point, Render, ScrollDelta, ScrollWheelEvent, Styled, Subscription, Task,
    UTF16Selection, Window, div,
};

use crate::actions::*;
use crate::terminal::mouse::{self, Button};
use crate::terminal::{Terminal, keys, report_focus};
use crate::terminal_element::TerminalElement;

/// Geometry of the last painted frame, used for hit-testing.
#[derive(Clone, Copy, Debug, Default)]
pub struct GridLayout {
    pub origin: Point<Pixels>,
    pub cell_width: Pixels,
    pub line_height: Pixels,
    pub cols: usize,
    pub rows: usize,
    pub cursor: Option<Bounds<Pixels>>,
}

impl GridLayout {
    /// Grid cell under `position`, clamped to the grid, plus which half of
    /// the cell was hit.
    pub fn cell_at(&self, position: Point<Pixels>) -> (usize, usize, Side) {
        let x = f32::from(position.x - self.origin.x) / f32::from(self.cell_width);
        let y = f32::from(position.y - self.origin.y) / f32::from(self.line_height);
        let col = (x.max(0.0) as usize).min(self.cols.saturating_sub(1));
        let line = (y.max(0.0) as usize).min(self.rows.saturating_sub(1));
        let side = if x.fract() > 0.5 && x >= 0.0 { Side::Right } else { Side::Left };
        (col, line, side)
    }

    pub fn contains_y(&self, y: Pixels) -> std::cmp::Ordering {
        let top = self.origin.y;
        let bottom = top + self.line_height * self.rows as f32;
        if y < top {
            std::cmp::Ordering::Less
        } else if y >= bottom {
            std::cmp::Ordering::Greater
        } else {
            std::cmp::Ordering::Equal
        }
    }
}

#[derive(Default)]
struct MouseState {
    selecting: bool,
    /// Button held while the app is receiving mouse reports.
    reported_button: Option<Button>,
    last_reported_cell: Option<(usize, usize)>,
    scroll_remainder: f32,
}

pub struct TerminalView {
    pub terminal: Entity<Terminal>,
    pub focus_handle: FocusHandle,
    pub font_size_override: Option<f32>,
    /// Grid size is dictated by someone else (tmux panes); don't resize to fit.
    pub fixed_size: bool,
    pub layout: Rc<Cell<Option<GridLayout>>>,
    pub blink_visible: bool,
    pub marked_text: Option<String>,
    mouse: MouseState,
    window_active: bool,
    /// Cursor blink timer; only runs while this terminal has focus.
    _blink: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for TerminalView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

const BLINK_INTERVAL: Duration = Duration::from_millis(530);

impl TerminalView {
    pub fn new(
        terminal: Entity<Terminal>,
        font_size_override: Option<f32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        let subscriptions = vec![
            cx.observe(&terminal, |_, _, cx| cx.notify()),
            cx.on_focus_in(&focus_handle, window, |this, window, cx| {
                this.blink_visible = true;
                this.start_blinking(window, cx);
                this.report_focus(window.is_window_active(), cx);
                cx.notify();
            }),
            cx.on_focus_out(&focus_handle, window, |this, _, _, cx| {
                // Unfocused terminals draw a hollow cursor and don't blink.
                this._blink = None;
                this.blink_visible = true;
                this.report_focus(false, cx);
                cx.notify();
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                let focused = this.focus_handle.is_focused(window) && window.is_window_active();
                this.report_focus(focused, cx);
                cx.notify();
            }),
        ];

        Self {
            terminal,
            focus_handle,
            font_size_override,
            fixed_size: false,
            layout: Rc::new(Cell::new(None)),
            blink_visible: true,
            marked_text: None,
            mouse: MouseState::default(),
            window_active: true,
            _blink: None,
            _subscriptions: subscriptions,
        }
    }

    fn report_focus(&mut self, focused: bool, cx: &mut Context<Self>) {
        if self.window_active != focused {
            self.window_active = focused;
            report_focus(self.terminal.read(cx), focused);
        }
    }

    fn start_blinking(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !crate::settings::Settings::get(cx).config.cursor.blink {
            return;
        }
        self._blink = Some(cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(BLINK_INTERVAL).await;
                let alive = this.update_in(cx, |this, window, cx| {
                    let idle = this.terminal.read(cx).last_activity.elapsed() > BLINK_INTERVAL;
                    let visible = !window.is_window_active() || !idle || !this.blink_visible;
                    if visible != this.blink_visible {
                        this.blink_visible = visible;
                        cx.notify();
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        }));
    }

    fn reset_blink(&mut self) {
        self.blink_visible = true;
    }

    fn mode(&self, cx: &App) -> TermMode {
        self.terminal.read(cx).mode()
    }

    // ---- keyboard -------------------------------------------------------------

    fn key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let mode = self.mode(cx);
        if let Some(bytes) = keys::to_esc_str(&event.keystroke, mode) {
            self.reset_blink();
            self.terminal.update(cx, |t, cx| t.input(bytes, cx));
            cx.stop_propagation();
        }
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.terminal.read(cx).selection_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.terminal.update(cx, |t, cx| t.paste(&text, cx));
        }
    }

    fn paste_primary(&mut self, _: &PastePrimary, _: &mut Window, cx: &mut Context<Self>) {
        self.paste_primary_selection(cx);
    }

    fn paste_primary_selection(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_primary().and_then(|item| item.text()) {
            self.terminal.update(cx, |t, cx| t.paste(&text, cx));
        }
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.terminal.update(cx, |t, cx| t.select_all(cx));
    }

    fn scroll(&mut self, scroll: Scroll, cx: &mut Context<Self>) {
        self.terminal.update(cx, |t, cx| t.scroll(scroll, cx));
    }

    fn clear_scrollback(&mut self, _: &ClearScrollback, _: &mut Window, cx: &mut Context<Self>) {
        self.terminal.update(cx, |t, cx| t.clear_scrollback(cx));
    }

    // ---- mouse ----------------------------------------------------------------

    /// Whether mouse events go to the application rather than to selection.
    /// Holding shift always selects, as in xterm.
    fn reporting(&self, mode: TermMode, modifiers: &gpui::Modifiers) -> bool {
        mode.intersects(TermMode::MOUSE_MODE) && !modifiers.shift
    }

    pub fn mouse_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle);
        let Some(layout) = self.layout.get() else { return };
        let (col, line, side) = layout.cell_at(e.position);
        let mode = self.mode(cx);

        if self.reporting(mode, &e.modifiers) {
            if let Some(button) = Button::from_gpui(e.button)
                && let Some(bytes) =
                    mouse::report(col, line, button, mouse::Action::Press, &e.modifiers, mode)
            {
                self.terminal.read(cx).write(bytes);
                self.mouse.reported_button = Some(button);
                self.mouse.last_reported_cell = Some((col, line));
            }
            return;
        }

        match e.button {
            gpui::MouseButton::Left => {
                let ty = match e.click_count {
                    0 | 1 if e.modifiers.alt || e.modifiers.control => SelectionType::Block,
                    0 | 1 => SelectionType::Simple,
                    2 => SelectionType::Semantic,
                    _ => SelectionType::Lines,
                };
                self.terminal.update(cx, |t, cx| t.start_selection(col, line, side, ty, cx));
                self.mouse.selecting = true;
            }
            gpui::MouseButton::Middle => self.paste_primary_selection(cx),
            _ => {}
        }
    }

    pub fn mouse_move(&mut self, e: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(layout) = self.layout.get() else { return };
        let (col, line, side) = layout.cell_at(e.position);
        let mode = self.mode(cx);

        if self.mouse.selecting && e.pressed_button == Some(gpui::MouseButton::Left) {
            // Drag past the top/bottom edge scrolls the scrollback.
            match layout.contains_y(e.position.y) {
                std::cmp::Ordering::Less => self.terminal.update(cx, |t, cx| t.scroll(Scroll::Delta(1), cx)),
                std::cmp::Ordering::Greater => self.terminal.update(cx, |t, cx| t.scroll(Scroll::Delta(-1), cx)),
                std::cmp::Ordering::Equal => {}
            }
            self.terminal.update(cx, |t, cx| t.update_selection(col, line, side, cx));
            return;
        }

        if mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION) && !e.modifiers.shift {
            if self.mouse.last_reported_cell == Some((col, line)) {
                return;
            }
            let button = self.mouse.reported_button.unwrap_or(Button::None);
            if let Some(bytes) =
                mouse::report(col, line, button, mouse::Action::Motion, &e.modifiers, mode)
            {
                self.mouse.last_reported_cell = Some((col, line));
                self.terminal.read(cx).write(bytes);
            }
        }
    }

    pub fn mouse_up(&mut self, e: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.mouse.selecting && e.button == gpui::MouseButton::Left {
            self.mouse.selecting = false;
            self.terminal.update(cx, |t, cx| t.finish_selection(cx));
            return;
        }
        let Some(layout) = self.layout.get() else { return };
        if let Some(button) = self.mouse.reported_button.take() {
            let (col, line, _) = layout.cell_at(e.position);
            let mode = self.mode(cx);
            if let Some(bytes) =
                mouse::report(col, line, button, mouse::Action::Release, &e.modifiers, mode)
            {
                self.terminal.read(cx).write(bytes);
            }
            self.mouse.last_reported_cell = None;
        }
    }

    pub fn scroll_wheel(&mut self, e: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(layout) = self.layout.get() else { return };
        let lines = match e.delta {
            ScrollDelta::Pixels(delta) => f32::from(delta.y) / f32::from(layout.line_height),
            ScrollDelta::Lines(delta) => delta.y,
        };
        self.mouse.scroll_remainder += lines;
        let whole = self.mouse.scroll_remainder.trunc();
        self.mouse.scroll_remainder -= whole;
        let lines = whole as i32;
        if lines == 0 {
            return;
        }

        let mode = self.mode(cx);
        if self.reporting(mode, &e.modifiers) {
            let (col, line, _) = layout.cell_at(e.position);
            let button = if lines > 0 { Button::WheelUp } else { Button::WheelDown };
            let terminal = self.terminal.read(cx);
            for _ in 0..lines.unsigned_abs() {
                if let Some(bytes) =
                    mouse::report(col, line, button, mouse::Action::Press, &e.modifiers, mode)
                {
                    terminal.write(bytes);
                }
            }
        } else if let Some(bytes) = mouse::alternate_scroll(lines, mode) {
            self.terminal.read(cx).write(bytes);
        } else {
            self.terminal.update(cx, |t, cx| t.scroll(Scroll::Delta(lines), cx));
        }
    }

    /// A selection drag or a reported button press is in progress.
    pub fn is_dragging(&self) -> bool {
        self.mouse.selecting || self.mouse.reported_button.is_some()
    }
}

impl Render for TerminalView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("terminal")
            .key_context("Terminal")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::key_down))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::paste_primary))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(|this, _: &ScrollLineUp, _, cx| this.scroll(Scroll::Delta(1), cx)))
            .on_action(cx.listener(|this, _: &ScrollLineDown, _, cx| this.scroll(Scroll::Delta(-1), cx)))
            .on_action(cx.listener(|this, _: &ScrollPageUp, _, cx| this.scroll(Scroll::PageUp, cx)))
            .on_action(cx.listener(|this, _: &ScrollPageDown, _, cx| this.scroll(Scroll::PageDown, cx)))
            .on_action(cx.listener(|this, _: &ScrollToTop, _, cx| this.scroll(Scroll::Top, cx)))
            .on_action(cx.listener(|this, _: &ScrollToBottom, _, cx| this.scroll(Scroll::Bottom, cx)))
            .on_action(cx.listener(Self::clear_scrollback))
            .size_full()
            .child(TerminalElement::new(cx.entity()))
    }
}

// ---- IME ------------------------------------------------------------------------

impl gpui::EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        _range: Range<usize>,
        _adjusted: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        None
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection { range: 0..0, reversed: false })
    }

    fn marked_text_range(&self, _window: &mut Window, _cx: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked_text.as_ref().map(|text| 0..text.encode_utf16().count())
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.marked_text = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked_text = None;
        self.reset_blink();
        let bytes = text.as_bytes().to_vec();
        self.terminal.update(cx, |t, cx| t.input(bytes, cx));
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<Range<usize>>,
        new_text: &str,
        _new_selected_range: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked_text = (!new_text.is_empty()).then(|| new_text.to_string());
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: Range<usize>,
        _element_bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        self.layout.get().and_then(|layout| layout.cursor)
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, px};

    #[test]
    fn hit_testing() {
        let layout = GridLayout {
            origin: point(px(10.0), px(20.0)),
            cell_width: px(8.0),
            line_height: px(16.0),
            cols: 80,
            rows: 24,
            cursor: None,
        };
        assert_eq!(layout.cell_at(point(px(10.0), px(20.0))), (0, 0, Side::Left));
        assert_eq!(layout.cell_at(point(px(10.0 + 8.0 * 3.0 + 7.0), px(20.0 + 16.0 * 2.0))), (3, 2, Side::Right));
        // Outside the grid clamps.
        assert_eq!(layout.cell_at(point(px(0.0), px(0.0))).0, 0);
        assert_eq!(layout.cell_at(point(px(5000.0), px(5000.0))), (79, 23, Side::Right));
    }
}
