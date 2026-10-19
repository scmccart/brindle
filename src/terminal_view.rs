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
    IntoElement, KeyDownEvent, Modifiers, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ParentElement, Pixels, Point, Render, ScrollDelta, ScrollWheelEvent, Styled, Subscription,
    Task, UTF16Selection, Window, div,
};

use crate::actions::*;
use crate::find_bar::{FindBar, LastFind};
use crate::latency::Latency;
use crate::terminal::mouse::{self, Button};
use crate::terminal::{Link, Terminal, keys, links, report_focus};
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

    /// The `--dump-screen-after` line that lets pixel checks find cells:
    /// the first cell's corner and the cell size in logical window pixels,
    /// and the scale factor to device pixels.
    pub fn describe(&self, scale: f32) -> String {
        format!(
            "--- grid origin={},{} cell={}x{} scale={}\n",
            f32::from(self.origin.x),
            f32::from(self.origin.y),
            f32::from(self.cell_width),
            f32::from(self.line_height),
            scale
        )
    }

    /// Whether `position` is over one of the grid's cells.
    pub fn contains(&self, position: Point<Pixels>) -> bool {
        let right = self.origin.x + self.cell_width * self.cols as f32;
        position.x >= self.origin.x
            && position.x < right
            && self.contains_y(position.y) == std::cmp::Ordering::Equal
    }

    /// Logical window position of the centre of a cell (for debug hooks).
    pub fn cell_center(&self, col: usize, line: usize) -> Point<Pixels> {
        Point {
            x: self.origin.x + self.cell_width * (col as f32 + 0.5),
            y: self.origin.y + self.line_height * (line as f32 + 0.5),
        }
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

/// A `--hover` or `--click` debug step: `[mods-]COL,ROW`, zero-based cells
/// of the active terminal, e.g. `ctrl-3,0` or `ctrl-shift-10,2`.
#[derive(Clone, Debug, PartialEq)]
pub struct PointerStep {
    pub modifiers: Modifiers,
    pub col: usize,
    pub row: usize,
    pub click: bool,
}

impl PointerStep {
    pub fn parse(text: &str, click: bool) -> Result<PointerStep, String> {
        let mut parts: Vec<&str> = text.split('-').collect();
        let position = parts.pop().unwrap_or_default();
        let mut modifiers = Modifiers::default();
        for part in parts {
            match part {
                "ctrl" | "control" => modifiers.control = true,
                "shift" => modifiers.shift = true,
                "alt" => modifiers.alt = true,
                "super" | "platform" => modifiers.platform = true,
                other => return Err(format!("unknown modifier {other:?} in {text:?}")),
            }
        }
        let (col, row) = position.split_once(',').ok_or_else(|| format!("expected COL,ROW in {text:?}"))?;
        let number = |s: &str| s.trim().parse::<usize>().map_err(|_| format!("bad cell {s:?} in {text:?}"));
        Ok(PointerStep { modifiers, col: number(col)?, row: number(row)?, click })
    }
}

#[derive(Default)]
struct MouseState {
    selecting: bool,
    /// Button held while the app is receiving mouse reports.
    reported_button: Option<Button>,
    last_reported_cell: Option<(usize, usize)>,
    scroll_remainder: f32,
    /// Last position seen over this terminal.
    pointer: Option<Point<Pixels>>,
    /// The cell `hovered_link` was computed for (`None` when ctrl isn't held).
    hover_cell: Option<(usize, usize)>,
    /// Modifiers of the last pointer event.
    modifiers: Modifiers,
    /// A ctrl-press on this link opens it if released over the same link.
    pending_link: Option<Link>,
}

/// Link activation takes ctrl without alt (alt-drag stays a block selection).
fn wants_links(modifiers: &Modifiers) -> bool {
    modifiers.control && !modifiers.alt
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
    /// The link under the mouse while ctrl is held; underlined when painted.
    pub hovered_link: Option<Link>,
    /// The open find bar, if any (see `find_bar.rs`).
    pub find: Option<FindBar>,
    /// The last search, restored when the bar opens again.
    pub last_find: Option<LastFind>,
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
        #[cfg(windows)]
        terminal.read(cx).parent_console_to(window);
        let focus_handle = cx.focus_handle();
        let subscriptions = vec![
            cx.observe(&terminal, |this, _, cx| {
                // Output may have changed the text under an underlined link.
                if this.hovered_link.is_some() {
                    this.refresh_hover(cx);
                }
                this.find_terminal_changed(cx);
                cx.notify()
            }),
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
                // A ctrl release while another window is active never reaches us.
                if !window.is_window_active() {
                    this.clear_hover(cx);
                }
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
            hovered_link: None,
            find: None,
            last_find: None,
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

    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // Keys bubbling up from the find bar never reach the program.
        if !self.focus_handle.is_focused(window) {
            return;
        }
        let mode = self.mode(cx);
        log::debug!("key {} reached the terminal unbound", event.keystroke.unparse());
        if let Some(bytes) = keys::to_esc_str(&event.keystroke, mode) {
            self.probe_latency(|| event.keystroke.unparse(), cx);
            self.reset_blink();
            self.terminal.update(cx, |t, cx| t.input(bytes, cx));
            cx.stop_propagation();
        }
    }

    /// Starts a latency probe for a keystroke (only with `--latency-*`).
    fn probe_latency(&self, label: impl FnOnce() -> String, cx: &mut Context<Self>) {
        if Latency::enabled(cx) {
            let terminal = self.terminal.read(cx);
            let generation = terminal.output_marker().generation();
            Latency::key(self.terminal.entity_id(), label, generation, cx);
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
        if let Some(text) = crate::primary::read_primary(cx).and_then(|item| item.text()) {
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

    // ---- links ----------------------------------------------------------------

    /// The link at a window position over this terminal's grid. Looks at the
    /// grid, so call it from event handlers, never while painting.
    fn link_under(&self, position: Point<Pixels>, cx: &App) -> Option<Link> {
        let layout = self.layout.get()?;
        if !layout.contains(position) {
            return None;
        }
        let (col, line, _) = layout.cell_at(position);
        self.terminal.read(cx).link_at_viewport(col, line)
    }

    fn set_hovered(&mut self, link: Option<Link>, cx: &mut Context<Self>) {
        if self.hovered_link != link {
            self.hovered_link = link;
            cx.notify();
        }
    }

    /// Updates the hovered link for the pointer at `position`. The link is
    /// looked up only while ctrl is held, and only when the cell changed
    /// unless `force` (the text or the scroll position may have changed).
    pub fn hover(&mut self, position: Point<Pixels>, modifiers: Modifiers, force: bool, cx: &mut Context<Self>) {
        self.mouse.pointer = Some(position);
        self.mouse.modifiers = modifiers;
        // No link feedback in the middle of a (block) selection or a drag
        // the program is receiving.
        let dragging = self.mouse.selecting || self.mouse.reported_button.is_some();
        if !wants_links(&modifiers) || dragging {
            self.mouse.hover_cell = None;
            self.set_hovered(None, cx);
            return;
        }
        let cell = self.layout.get().filter(|l| l.contains(position)).map(|l| {
            let (col, line, _) = l.cell_at(position);
            (col, line)
        });
        if !force && cell.is_some() && cell == self.mouse.hover_cell {
            return;
        }
        self.mouse.hover_cell = cell;
        let link = cell.and_then(|_| self.link_under(position, cx));
        self.set_hovered(link, cx);
    }

    /// Recomputes the hovered link where the pointer last was.
    fn refresh_hover(&mut self, cx: &mut Context<Self>) {
        if let Some(position) = self.mouse.pointer {
            self.hover(position, self.mouse.modifiers, true, cx);
        }
    }

    /// Drops hover feedback (the pointer left, or the window lost focus).
    pub fn clear_hover(&mut self, cx: &mut Context<Self>) {
        self.mouse.hover_cell = None;
        self.set_hovered(None, cx);
    }

    /// Ctrl pressed or released over this terminal.
    pub fn modifiers_changed(&mut self, position: Point<Pixels>, modifiers: Modifiers, cx: &mut Context<Self>) {
        self.hover(position, modifiers, true, cx);
    }

    fn copy_link(&mut self, _: &CopyLink, window: &mut Window, cx: &mut Context<Self>) {
        let position = self.mouse.pointer.unwrap_or_else(|| window.mouse_position());
        if let Some(link) = self.link_under(position, cx) {
            cx.write_to_clipboard(ClipboardItem::new_string(link.uri));
        }
    }

    /// Jumps between shell-integration prompts, or scrolls one line when
    /// the terminal has none (the keys used to scroll by line).
    fn jump_to_prompt(&mut self, previous: bool, cx: &mut Context<Self>) {
        let jumped = self.terminal.update(cx, |t, cx| t.jump_to_prompt(previous, cx));
        if !jumped {
            self.scroll(Scroll::Delta(if previous { 1 } else { -1 }), cx);
        }
    }

    fn select_last_output(&mut self, _: &SelectLastOutput, _: &mut Window, cx: &mut Context<Self>) {
        self.terminal.update(cx, |t, cx| t.select_last_output(cx));
    }

    fn copy_last_output(&mut self, _: &CopyLastOutput, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.terminal.read(cx).last_output_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
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

        // A ctrl-click on a link is ours, even when the program reports the
        // mouse (as shift is for selection). It opens on release.
        if e.button == gpui::MouseButton::Left
            && wants_links(&e.modifiers)
            && let Some(link) = self.link_under(e.position, cx)
        {
            self.mouse.pending_link = Some(link);
            return;
        }

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
        self.hover(e.position, e.modifiers, false, cx);
        if self.mouse.pending_link.is_some() {
            return;
        }
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
        if e.button == gpui::MouseButton::Left
            && let Some(pressed) = self.mouse.pending_link.take()
        {
            if self.link_under(e.position, cx).is_some_and(|link| link.uri == pressed.uri) {
                links::open(&pressed.uri);
            }
            return;
        }
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
        // Different text may be under the pointer now.
        self.hover(e.position, e.modifiers, true, cx);
    }

    /// Debug hook for `--hover` / `--click`: feeds GPUI mouse events for a
    /// cell straight to this view's handlers. No OS-level input is made.
    pub fn debug_pointer(&mut self, step: &PointerStep, window: &mut Window, cx: &mut Context<Self>) {
        let Some(layout) = self.layout.get() else {
            log::error!("--hover/--click: the terminal hasn't been painted yet");
            return;
        };
        let position = layout.cell_center(step.col, step.row);
        let modifiers = step.modifiers;
        self.mouse_move(&MouseMoveEvent { position, pressed_button: None, modifiers }, window, cx);
        if step.click {
            let button = gpui::MouseButton::Left;
            self.mouse_down(&MouseDownEvent { button, position, modifiers, click_count: 1, first_mouse: false }, window, cx);
            self.mouse_up(&MouseUpEvent { button, position, modifiers, click_count: 1 }, window, cx);
        }
    }

    /// A selection drag, a reported button press or a press on a link is in
    /// progress.
    pub fn is_dragging(&self) -> bool {
        self.mouse.selecting || self.mouse.reported_button.is_some() || self.mouse.pending_link.is_some()
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let find_bar = self.render_find_bar(window, cx);
        div()
            .id("terminal")
            .key_context("Terminal")
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::key_down))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::paste_primary))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(Self::copy_link))
            .on_action(cx.listener(|this, _: &ScrollLineUp, _, cx| this.scroll(Scroll::Delta(1), cx)))
            .on_action(cx.listener(|this, _: &ScrollLineDown, _, cx| this.scroll(Scroll::Delta(-1), cx)))
            .on_action(cx.listener(|this, _: &ScrollPageUp, _, cx| this.scroll(Scroll::PageUp, cx)))
            .on_action(cx.listener(|this, _: &ScrollPageDown, _, cx| this.scroll(Scroll::PageDown, cx)))
            .on_action(cx.listener(|this, _: &ScrollToTop, _, cx| this.scroll(Scroll::Top, cx)))
            .on_action(cx.listener(|this, _: &ScrollToBottom, _, cx| this.scroll(Scroll::Bottom, cx)))
            .on_action(cx.listener(Self::clear_scrollback))
            .on_action(cx.listener(|this, _: &JumpToPreviousPrompt, _, cx| this.jump_to_prompt(true, cx)))
            .on_action(cx.listener(|this, _: &JumpToNextPrompt, _, cx| this.jump_to_prompt(false, cx)))
            .on_action(cx.listener(Self::select_last_output))
            .on_action(cx.listener(Self::copy_last_output))
            .on_action(cx.listener(Self::find))
            .on_action(cx.listener(Self::find_next))
            .on_action(cx.listener(Self::find_previous))
            .on_action(cx.listener(Self::find_close))
            .on_action(cx.listener(Self::find_toggle_case))
            .on_action(cx.listener(Self::find_toggle_regex))
            .relative()
            .size_full()
            .child(TerminalElement::new(cx.entity()))
            .children(find_bar)
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
        log::debug!("text {text:?} from the input method");
        self.marked_text = None;
        self.probe_latency(|| text.to_string(), cx);
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

    #[test]
    fn grid_description() {
        let grid = GridLayout {
            origin: gpui::point(gpui::px(6.0), gpui::px(51.0)),
            cell_width: gpui::px(8.4),
            line_height: gpui::px(18.0),
            ..Default::default()
        };
        assert_eq!(grid.describe(1.0), "--- grid origin=6,51 cell=8.4x18 scale=1\n");
        assert_eq!(grid.describe(2.0), "--- grid origin=6,51 cell=8.4x18 scale=2\n");
    }
    use gpui::{point, px};

    #[test]
    fn pointer_steps() {
        let step = PointerStep::parse("ctrl-3,0", true).unwrap();
        assert_eq!((step.col, step.row, step.click), (3, 0, true));
        assert!(step.modifiers.control && !step.modifiers.shift && !step.modifiers.alt);
        let step = PointerStep::parse("ctrl-shift-10,2", false).unwrap();
        assert!(step.modifiers.control && step.modifiers.shift);
        assert_eq!((step.col, step.row), (10, 2));
        assert_eq!(PointerStep::parse("4,5", false).unwrap().modifiers, Modifiers::default());
        assert!(PointerStep::parse("hyper-1,1", false).is_err());
        assert!(PointerStep::parse("ctrl-1", false).is_err());
        assert!(PointerStep::parse("ctrl-a,1", false).is_err());
        assert!(PointerStep::parse("", false).is_err());
    }

    #[test]
    fn grid_contains_and_centres() {
        let layout = GridLayout {
            origin: point(px(10.0), px(20.0)),
            cell_width: px(8.0),
            line_height: px(16.0),
            cols: 80,
            rows: 24,
            cursor: None,
        };
        assert!(layout.contains(point(px(10.0), px(20.0))));
        assert!(!layout.contains(point(px(9.0), px(20.0))));
        assert!(!layout.contains(point(px(10.0 + 8.0 * 80.0), px(30.0))));
        assert!(!layout.contains(point(px(20.0), px(20.0 + 16.0 * 24.0))));
        let centre = layout.cell_center(3, 2);
        assert_eq!(layout.cell_at(centre), (3, 2, Side::Left));
        assert_eq!(centre, point(px(10.0 + 8.0 * 3.5), px(20.0 + 16.0 * 2.5)));
    }

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
