//! A text area: text of several lines, wrapped at its width, for a prompt.
//!
//! It shares the single-line input's editing core: [`EditState`] in its
//! `Lines::Many` mode holds the text, selection and composition and does
//! every edit, and the pure [`WrapMap`] (built from the shaped rows each
//! frame) answers where the cursor goes on up and down ([`motion_vertical`])
//! and which rows are shown ([`first_row_for_cursor`]). [`TextArea`] is the
//! entity: focus, the `TextArea` key context's actions, mouse, clipboard,
//! the platform input handler, and growth from one row to `max_lines` rows,
//! after which it scrolls to keep the cursor's row in view.
//!
//! It binds no escape, tab or shift-tab, binds enter only to pass it on
//! (`TextAreaEnter`, which keeps it while an input method is composing),
//! and lets up on its first row and down on its last go on, so its owner
//! can bind them (a prompt input binds enter to submit). Shift-enter
//! inserts a line break. A lone line break or tab arriving as typed text (a
//! key nothing bound) is dropped; pasted text keeps them.
//!
//! [`EditState`]: crate::text_input::EditState

use std::ops::Range;

use gpui::{
    App, AvailableSpace, Bounds, ClipboardItem, Context, ContentMask, CursorStyle, Element, ElementId,
    ElementInputHandler, Entity, EntityInputHandler, EventEmitter, FocusHandle, Focusable, Font, GlobalElementId,
    InteractiveElement, IntoElement, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad,
    ParentElement, Pixels, Point, Render, ScrollWheelEvent, SharedString, Style, Styled, TextAlign, TextRun,
    UTF16Selection, UnderlineStyle, Window, WrappedLine, div, fill, point, px, relative, size,
};

use crate::actions::{
    TextAreaDown, TextAreaEnter, TextAreaLineEnd, TextAreaLineStart, TextAreaNewline, TextAreaSelectDown, TextAreaSelectLineEnd,
    TextAreaSelectLineStart, TextAreaSelectUp, TextAreaUp, TextInputBackspace, TextInputCopy, TextInputCut,
    TextInputDelete, TextInputDeleteWordBack, TextInputEnd, TextInputHome, TextInputLeft, TextInputPaste,
    TextInputRight, TextInputSelectAll, TextInputSelectEnd, TextInputSelectHome, TextInputSelectLeft,
    TextInputSelectRight, TextInputSelectWordLeft, TextInputSelectWordRight, TextInputWordLeft, TextInputWordRight,
};
use crate::primitives::focus_visible::focus_from_pointer;
use crate::text_input::{
    EditState, Motion, Underline, Vertical, VerticalMotion, WrapMap, WrapRow, first_row_for_cursor, motion_vertical,
};

/// The rows a text area grows to before it scrolls, unless its owner sets
/// another number.
pub const DEFAULT_MAX_LINES: usize = 8;

pub enum TextAreaEvent {
    /// The text changed through the user (typing, paste, deletion, IME).
    Changed,
}

/// Whether text that arrived as typed input is a key nothing bound (enter
/// or tab), which a text area leaves to its owner rather than insert.
pub fn is_unbound_key_text(text: &str) -> bool {
    matches!(text, "\n" | "\r" | "\r\n" | "\t")
}

/// How many rows a text area shows for `rows` rows of text: at least one,
/// at most `max_lines`.
pub fn shown_rows(rows: usize, max_lines: usize) -> usize {
    rows.clamp(1, max_lines.max(1))
}

/// The rows of shaped lines as a [`WrapMap`]: each line of the text (its
/// shaped glyphs and wrap boundaries), the text's lines in order with one
/// line break between each.
fn wrap_map(lines: &[WrappedLine]) -> WrapMap {
    let mut rows = Vec::new();
    let mut base = 0;
    for line in lines {
        let layout = &line.unwrapped_layout;
        let glyphs: Vec<(usize, Pixels)> =
            layout.runs.iter().flat_map(|run| run.glyphs.iter().map(|glyph| (glyph.index, glyph.position.x))).collect();
        let glyph_at = |run_ix: usize, glyph_ix: usize| {
            layout.runs.get(run_ix).and_then(|run| run.glyphs.get(glyph_ix)).map(|g| (g.index, g.position.x))
        };
        // Where each row starts, as (index in the line, x).
        let mut starts = vec![(0, px(0.))];
        starts.extend(line.wrap_boundaries.iter().filter_map(|b| glyph_at(b.run_ix, b.glyph_ix)));
        // Every char boundary's x, in one pass over the glyphs.
        let mut glyph = 0;
        let bounds: Vec<(usize, Pixels)> = line
            .text
            .char_indices()
            .map(|(ix, _)| ix)
            .chain([line.text.len()])
            .map(|ix| {
                while glyphs.get(glyph).is_some_and(|(index, _)| *index < ix) {
                    glyph += 1;
                }
                (ix, glyphs.get(glyph).map_or(layout.width, |(_, x)| *x))
            })
            .collect();
        for (row, &(start, start_x)) in starts.iter().enumerate() {
            let next = starts.get(row + 1).map(|(ix, _)| *ix);
            let end = next.unwrap_or(line.text.len());
            let stops = bounds
                .iter()
                .filter(|(ix, _)| (start..=end).contains(ix))
                .map(|(ix, x)| (base + ix, f32::from(*x - start_x)))
                .collect();
            rows.push(WrapRow { start: base + start, end: base + end, wraps: next.is_some(), stops });
        }
        base += line.text.len() + 1;
    }
    WrapMap::new(rows)
}

/// The x of `offset` on `row`, from the row's start.
fn x_in_row(row: &WrapRow, offset: usize) -> f32 {
    let ix = row.stops.partition_point(|(stop, _)| *stop < offset);
    row.stops.get(ix).or(row.stops.last()).map_or(0., |(_, x)| *x)
}

/// The layout the last paint used: what the pointer and the input method
/// are measured against.
struct Painted {
    wrap: WrapMap,
    bounds: Bounds<Pixels>,
    line_height: Pixels,
    first_row: usize,
    visible: usize,
}

/// What the wrap map was built from, so an unchanged text at an unchanged
/// width isn't measured again.
#[derive(Clone, PartialEq)]
struct WrapKey {
    generation: u64,
    width: Pixels,
    font_size: Pixels,
    font: Font,
}

pub struct TextArea {
    focus_handle: FocusHandle,
    state: EditState,
    placeholder: SharedString,
    max_lines: usize,
    read_only: bool,
    /// The x a run of up and down keys aims at.
    goal_x: Option<f32>,
    /// The first row shown, once the text is taller than the area.
    first_row: usize,
    /// Bring the cursor's row into view at the next paint (it moved or the
    /// text changed); a wheel scroll leaves it where it is.
    follow_cursor: bool,
    /// Bumped on every change of the text, for the wrap map's cache.
    generation: u64,
    wrap_cache: Option<(WrapKey, WrapMap)>,
    painted: Option<Painted>,
    /// A drag selection is in progress.
    selecting: bool,
}

impl EventEmitter<TextAreaEvent> for TextArea {}

impl Focusable for TextArea {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl TextArea {
    pub fn new(text: &str, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            state: EditState::multiline(text),
            placeholder: SharedString::default(),
            max_lines: DEFAULT_MAX_LINES,
            read_only: false,
            goal_x: None,
            first_row: 0,
            follow_cursor: true,
            generation: 0,
            wrap_cache: None,
            painted: None,
            selecting: false,
        }
    }

    pub fn with_placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    pub fn set_placeholder(&mut self, placeholder: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.placeholder = placeholder.into();
        cx.notify();
    }

    /// The most rows it grows to before it scrolls (at least one).
    pub fn with_max_lines(mut self, max_lines: usize) -> Self {
        self.max_lines = max_lines.max(1);
        self
    }

    pub fn set_max_lines(&mut self, max_lines: usize, cx: &mut Context<Self>) {
        self.max_lines = max_lines.max(1);
        cx.notify();
    }

    pub fn max_lines(&self) -> usize {
        self.max_lines
    }

    /// Makes the area a tab stop, so `Window::focus_next` reaches it.
    pub fn tab_stop(mut self) -> Self {
        self.focus_handle = self.focus_handle.tab_stop(true);
        self
    }

    /// A read-only area still takes focus, moves the cursor, selects and
    /// copies, but changes nothing (its owner's prompt is disabled).
    pub fn set_read_only(&mut self, read_only: bool, cx: &mut Context<Self>) {
        if self.read_only != read_only {
            self.read_only = read_only;
            cx.notify();
        }
    }

    pub fn read_only(&self) -> bool {
        self.read_only
    }

    pub fn text(&self) -> &str {
        self.state.text()
    }

    pub fn cursor(&self) -> usize {
        self.state.cursor()
    }

    pub fn selection(&self) -> Range<usize> {
        self.state.selection()
    }

    /// Whether an input method is composing text in the area: an enter
    /// then commits the composition and is no one else's.
    pub fn composing(&self) -> bool {
        self.state.marked().is_some()
    }

    /// The first row shown and how many rows show, as last painted.
    pub fn shown(&self) -> Option<(usize, usize)> {
        self.painted.as_ref().map(|p| (p.first_row, p.visible))
    }

    /// How many rows the text took at the last paint.
    pub fn rows(&self) -> Option<usize> {
        self.painted.as_ref().map(|p| p.wrap.rows().len())
    }

    /// Replaces the text from the owner, cursor at the end. This emits no
    /// `Changed`.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if self.state.text() != text {
            self.state.set_text(text);
            self.changed_text();
            cx.notify();
        }
    }

    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        self.state.select_all();
        cx.notify();
    }

    /// Puts the cursor at the end of the text (a press below it).
    pub fn move_to_end(&mut self, cx: &mut Context<Self>) {
        self.motion(Motion::End, false, cx);
    }

    /// Inserts text as if typed (the `--send` debug hook): line breaks and
    /// tabs are kept, as a paste keeps them.
    pub fn insert(&mut self, text: &str, cx: &mut Context<Self>) {
        self.edit(cx, |s| s.insert(text));
    }

    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus_handle.is_focused(window)
    }

    pub fn describe(&self) -> String {
        self.state.describe()
    }

    fn changed_text(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.goal_x = None;
        self.follow_cursor = true;
    }

    /// Runs an edit, redraws, and reports a change of text. A read-only
    /// area runs none.
    fn edit(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut EditState)) {
        if self.read_only {
            return;
        }
        let old = self.state.text().to_string();
        f(&mut self.state);
        self.goal_x = None;
        self.follow_cursor = true;
        if old != self.state.text() {
            self.changed_text();
            cx.emit(TextAreaEvent::Changed);
        }
        cx.notify();
    }

    fn motion(&mut self, motion: Motion, extend: bool, cx: &mut Context<Self>) {
        self.state.motion(motion, extend);
        self.goal_x = None;
        self.follow_cursor = true;
        cx.notify();
    }

    /// Moves to `offset` (a shown line's start or end), keeping no goal.
    fn move_within_row(&mut self, end: bool, extend: bool, cx: &mut Context<Self>) {
        let Some(painted) = &self.painted else {
            self.motion(if end { Motion::End } else { Motion::Home }, extend, cx);
            return;
        };
        let cursor = self.state.cursor();
        let target = if end { painted.wrap.row_end(cursor) } else { painted.wrap.row_start(cursor) };
        self.state.move_to(target, extend);
        self.goal_x = None;
        self.follow_cursor = true;
        cx.notify();
    }

    /// Up or down a shown line. On the first row up, or the last down, the
    /// key goes on to the owner; with shift there, the selection extends
    /// to the text's start or end instead.
    fn vertical(&mut self, dir: Vertical, extend: bool, cx: &mut Context<Self>) {
        let Some(painted) = &self.painted else {
            cx.propagate();
            return;
        };
        match motion_vertical(&mut self.state, &painted.wrap, dir, self.goal_x, extend) {
            VerticalMotion::Moved { goal_x } => {
                self.goal_x = Some(goal_x);
                self.follow_cursor = true;
                cx.notify();
            }
            VerticalMotion::AtEdge if extend => {
                let motion = match dir {
                    Vertical::Up => Motion::Home,
                    Vertical::Down => Motion::End,
                };
                self.motion(motion, true, cx);
            }
            VerticalMotion::AtEdge => cx.propagate(),
        }
    }

    fn up(&mut self, _: &TextAreaUp, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(Vertical::Up, false, cx);
    }
    fn down(&mut self, _: &TextAreaDown, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(Vertical::Down, false, cx);
    }
    fn select_up(&mut self, _: &TextAreaSelectUp, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(Vertical::Up, true, cx);
    }
    fn select_down(&mut self, _: &TextAreaSelectDown, _: &mut Window, cx: &mut Context<Self>) {
        self.vertical(Vertical::Down, true, cx);
    }
    fn line_start(&mut self, _: &TextAreaLineStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_within_row(false, false, cx);
    }
    fn line_end(&mut self, _: &TextAreaLineEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_within_row(true, false, cx);
    }
    fn select_line_start(&mut self, _: &TextAreaSelectLineStart, _: &mut Window, cx: &mut Context<Self>) {
        self.move_within_row(false, true, cx);
    }
    fn select_line_end(&mut self, _: &TextAreaSelectLineEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.move_within_row(true, true, cx);
    }
    fn newline(&mut self, _: &TextAreaNewline, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, |s| s.insert("\n"));
    }
    /// Enter is the owner's, except while an input method is composing:
    /// then it is the input method's, which commits the composition
    /// through the input handler, and goes no further.
    fn enter(&mut self, _: &TextAreaEnter, _: &mut Window, cx: &mut Context<Self>) {
        if !self.composing() {
            cx.propagate();
        }
    }

    fn left(&mut self, _: &TextInputLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.motion(Motion::Left, false, cx);
    }
    fn right(&mut self, _: &TextInputRight, _: &mut Window, cx: &mut Context<Self>) {
        self.motion(Motion::Right, false, cx);
    }
    fn word_left(&mut self, _: &TextInputWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.motion(Motion::WordLeft, false, cx);
    }
    fn word_right(&mut self, _: &TextInputWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.motion(Motion::WordRight, false, cx);
    }
    fn home(&mut self, _: &TextInputHome, _: &mut Window, cx: &mut Context<Self>) {
        self.motion(Motion::Home, false, cx);
    }
    fn end(&mut self, _: &TextInputEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.motion(Motion::End, false, cx);
    }
    fn select_left(&mut self, _: &TextInputSelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.motion(Motion::Left, true, cx);
    }
    fn select_right(&mut self, _: &TextInputSelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.motion(Motion::Right, true, cx);
    }
    fn select_word_left(&mut self, _: &TextInputSelectWordLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.motion(Motion::WordLeft, true, cx);
    }
    fn select_word_right(&mut self, _: &TextInputSelectWordRight, _: &mut Window, cx: &mut Context<Self>) {
        self.motion(Motion::WordRight, true, cx);
    }
    fn select_home(&mut self, _: &TextInputSelectHome, _: &mut Window, cx: &mut Context<Self>) {
        self.motion(Motion::Home, true, cx);
    }
    fn select_end(&mut self, _: &TextInputSelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        self.motion(Motion::End, true, cx);
    }
    fn select_all_action(&mut self, _: &TextInputSelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.select_all(cx);
    }

    fn backspace(&mut self, _: &TextInputBackspace, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, EditState::backspace);
    }
    fn delete(&mut self, _: &TextInputDelete, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, EditState::delete);
    }
    fn delete_word_back(&mut self, _: &TextInputDeleteWordBack, _: &mut Window, cx: &mut Context<Self>) {
        self.edit(cx, EditState::delete_word_back);
    }

    fn copy(&mut self, _: &TextInputCopy, _: &mut Window, cx: &mut Context<Self>) {
        let text = self.state.selected_text();
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text.to_string()));
        }
    }
    fn cut(&mut self, _: &TextInputCut, _: &mut Window, cx: &mut Context<Self>) {
        let text = self.state.selected_text().to_string();
        if !text.is_empty() && !self.read_only {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            self.edit(cx, |s| s.insert(""));
        }
    }
    fn paste(&mut self, _: &TextInputPaste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.edit(cx, |s| s.insert(&text));
        }
    }

    /// The text offset under a window position: the nearest place on the
    /// shown row under it (the first or last row above or below the area).
    fn offset_for_position(&self, position: Point<Pixels>) -> usize {
        let Some(painted) = &self.painted else { return 0 };
        let rows = painted.wrap.rows().len();
        if rows == 0 {
            return 0;
        }
        let from_top = ((position.y - painted.bounds.top()) / painted.line_height).floor();
        let row = (painted.first_row as f32 + from_top).clamp(0., (rows - 1) as f32) as usize;
        painted.wrap.offset_at(row, f32::from(position.x - painted.bounds.left()))
    }

    fn mouse_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // The press is the area's: a frame around it doesn't also move the
        // cursor to the end.
        cx.stop_propagation();
        focus_from_pointer(&self.focus_handle, window, cx);
        let offset = self.offset_for_position(e.position);
        if self.state.press(offset, e.click_count, e.modifiers.shift) {
            self.selecting = true;
        }
        self.goal_x = None;
        cx.notify();
    }

    fn mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.selecting = false;
    }

    fn mouse_move(&mut self, e: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.selecting && e.pressed_button == Some(MouseButton::Left) {
            self.state.move_to(self.offset_for_position(e.position), true);
            self.follow_cursor = true;
            cx.notify();
        }
    }

    /// A wheel over a text taller than the area scrolls its rows, leaving
    /// the cursor where it is.
    fn scroll_wheel(&mut self, e: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let Some(painted) = &self.painted else { return };
        let total = painted.wrap.rows().len();
        if total <= painted.visible {
            return;
        }
        let delta = e.delta.pixel_delta(painted.line_height).y;
        let rows = -(delta / painted.line_height).round() as isize;
        let max = (total - painted.visible) as isize;
        let first = (self.first_row as isize + rows).clamp(0, max) as usize;
        if first != self.first_row {
            self.first_row = first;
            self.follow_cursor = false;
            cx.notify();
        }
        cx.stop_propagation();
    }
}

impl EntityInputHandler for TextArea {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.state.range_from_utf16(&range_utf16);
        actual_range.replace(self.state.range_to_utf16(&range));
        Some(self.state.text()[range].to_string())
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        Some(UTF16Selection { range: self.state.range_to_utf16(&self.state.selection()), reversed: self.state.reversed() })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.state.marked().map(|range| self.state.range_to_utf16(&range))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.state.unmark();
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Enter or tab with nothing bound to it is the owner's to have
        // left alone, not a character to insert.
        if range_utf16.is_none() && !self.composing() && is_unbound_key_text(text) {
            return;
        }
        self.edit(cx, |s| match range_utf16 {
            Some(range) => {
                let range = s.range_from_utf16(&range);
                s.replace(range, text);
            }
            None => s.insert(text),
        });
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        text: &str,
        selected_utf16: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.edit(cx, |s| {
            let range = range_utf16.map(|r| s.range_from_utf16(&r));
            // The selection is relative to the new text, in its UTF-16 units.
            let selected = selected_utf16.map(|r| EditState::multiline(text).range_from_utf16(&r));
            s.replace_and_mark(range, text, selected);
        });
    }

    /// The range's place on its shown row, for the input method's window:
    /// from its start to its end when both are on that row, else from its
    /// start to the row's end.
    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let painted = self.painted.as_ref()?;
        let range = self.state.range_from_utf16(&range_utf16);
        let row_ix = painted.wrap.row_of(range.start);
        let row = painted.wrap.rows().get(row_ix)?;
        let end = if painted.wrap.row_of(range.end) == row_ix { range.end } else { row.end };
        let top = painted.bounds.top() + painted.line_height * (row_ix as f32 - painted.first_row as f32);
        let left = painted.bounds.left();
        Some(Bounds::from_corners(
            point(left + px(x_in_row(row, range.start)), top),
            point(left + px(x_in_row(row, end)), top + painted.line_height),
        ))
    }

    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        self.painted.as_ref()?;
        let offset = self.offset_for_position(position);
        Some(self.state.to_utf16(offset))
    }
}

impl Render for TextArea {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("TextArea")
            .track_focus(&self.focus_handle)
            .cursor(CursorStyle::IBeam)
            .flex()
            .flex_1()
            .min_w_0()
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::select_up))
            .on_action(cx.listener(Self::select_down))
            .on_action(cx.listener(Self::line_start))
            .on_action(cx.listener(Self::line_end))
            .on_action(cx.listener(Self::select_line_start))
            .on_action(cx.listener(Self::select_line_end))
            .on_action(cx.listener(Self::newline))
            .on_action(cx.listener(Self::enter))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::word_left))
            .on_action(cx.listener(Self::word_right))
            .on_action(cx.listener(Self::home))
            .on_action(cx.listener(Self::end))
            .on_action(cx.listener(Self::select_left))
            .on_action(cx.listener(Self::select_right))
            .on_action(cx.listener(Self::select_word_left))
            .on_action(cx.listener(Self::select_word_right))
            .on_action(cx.listener(Self::select_home))
            .on_action(cx.listener(Self::select_end))
            .on_action(cx.listener(Self::select_all_action))
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::delete_word_back))
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::cut))
            .on_action(cx.listener(Self::paste))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_scroll_wheel(cx.listener(Self::scroll_wheel))
            .child(TextAreaElement { area: cx.entity() })
    }
}

// ---- element ----------------------------------------------------------------

/// Paints a [`TextArea`]'s rows, in the inherited text style: the
/// selection and caret use the text color, the placeholder a faded one.
struct TextAreaElement {
    area: Entity<TextArea>,
}

struct Prepainted {
    /// The shaped lines, each with the y its first row is drawn at.
    lines: Vec<(WrappedLine, Pixels)>,
    selection: Vec<PaintQuad>,
    caret: Option<PaintQuad>,
}

/// What the area shows (its text, else the placeholder) and the runs to
/// shape it with.
fn shown_text(area: &TextArea, window: &Window) -> (SharedString, Vec<TextRun>) {
    let style = window.text_style();
    let color = style.color;
    let run = TextRun {
        len: 0,
        font: style.font(),
        color,
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    if area.state.shows_placeholder() {
        let text = area.placeholder.clone();
        let faded = TextRun { len: text.len(), color: color.opacity(0.45), ..run };
        return (text, vec![faded]);
    }
    let color = if area.read_only { color.opacity(0.5) } else { color };
    let runs = area
        .state
        .run_lens()
        .into_iter()
        .map(|(len, underline)| TextRun {
            len,
            color,
            underline: match underline {
                Underline::None => None,
                Underline::Composition => Some(UnderlineStyle { color: Some(color), thickness: px(1.0), wavy: false }),
            },
            ..run.clone()
        })
        .collect();
    (area.state.text().to_string().into(), runs)
}

impl IntoElement for TextAreaElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextAreaElement {
    type RequestLayoutState = ();
    type PrepaintState = Prepainted;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    /// As tall as its rows at the width it is given, from one to
    /// `max_lines`.
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let area = self.area.read(cx);
        let (text, runs) = shown_text(area, window);
        let max_lines = area.max_lines;
        let font_size = window.text_style().font_size.to_pixels(window.rem_size());
        let line_height = window.line_height();
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        let layout = window.request_measured_layout(style, move |known, available, window, _| {
            let width = known.width.or(match available.width {
                AvailableSpace::Definite(width) => Some(width),
                AvailableSpace::MinContent | AvailableSpace::MaxContent => None,
            });
            let rows = window
                .text_system()
                .shape_text(text.clone(), font_size, &runs, width, None)
                .map_or(1, |lines| lines.iter().map(|line| line.wrap_boundaries().len() + 1).sum());
            size(width.unwrap_or(px(0.)), line_height * shown_rows(rows, max_lines) as f32)
        });
        (layout, ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) -> Prepainted {
        let area = self.area.read(cx);
        let style = window.text_style();
        let color = style.color;
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line_height = window.line_height();
        let (text, runs) = shown_text(area, window);
        let placeholder = area.state.shows_placeholder();
        let shaped: Vec<WrappedLine> = window
            .text_system()
            .shape_text(text, font_size, &runs, Some(bounds.size.width), None)
            .map(|lines| lines.into_vec())
            .unwrap_or_default();
        // The rows of the text itself (an empty text's one row while the
        // placeholder shows), from the cache while nothing changed.
        let key = WrapKey { generation: area.generation, width: bounds.size.width, font_size, font: style.font() };
        let wrap = match &area.wrap_cache {
            Some((cached, wrap)) if *cached == key && !placeholder => wrap.clone(),
            _ if placeholder => WrapMap::new(vec![WrapRow { start: 0, end: 0, wraps: false, stops: vec![(0, 0.)] }]),
            _ => wrap_map(&shaped),
        };
        let total = wrap.rows().len();
        let visible = ((bounds.size.height / line_height).round() as usize).max(1);
        let cursor = area.state.cursor();
        let cursor_row = wrap.row_of(cursor);
        let first_row = if area.follow_cursor {
            first_row_for_cursor(area.first_row, cursor_row, visible, total)
        } else {
            area.first_row.min(total.saturating_sub(visible))
        };
        let row_top = |row: usize| bounds.top() + line_height * (row as f32 - first_row as f32);

        // Each shaped line's first row is where its rows start counting.
        let mut lines = Vec::with_capacity(shaped.len());
        let mut row = 0;
        for line in shaped {
            let rows = line.wrap_boundaries().len() + 1;
            lines.push((line, row_top(row)));
            row += rows;
        }

        let selection = area.state.selection();
        let selection_quads = if selection.is_empty() || placeholder {
            Vec::new()
        } else {
            wrap.rows()
                .iter()
                .enumerate()
                .filter(|(_, r)| r.start <= selection.end && selection.start <= r.end)
                .map(|(ix, r)| {
                    let from = x_in_row(r, selection.start.max(r.start));
                    let to = x_in_row(r, selection.end.min(r.end));
                    // A selected line break shows as a sliver past the row.
                    let to = if selection.end > r.end && !r.wraps { to + 4. } else { to };
                    fill(
                        Bounds::from_corners(
                            point(bounds.left() + px(from), row_top(ix)),
                            point(bounds.left() + px(to), row_top(ix) + line_height),
                        ),
                        color.opacity(0.28),
                    )
                })
                .collect()
        };
        let caret_x = if placeholder { 0. } else { wrap.x_of(cursor) };
        let caret = (!area.read_only).then(|| {
            fill(Bounds::new(point(bounds.left() + px(caret_x), row_top(cursor_row)), size(px(1.5), line_height)), color)
        });
        self.area.update(cx, |area, _| {
            area.first_row = first_row;
            area.follow_cursor = false;
            if !placeholder {
                area.wrap_cache = Some((key, wrap.clone()));
            }
            area.painted = Some(Painted { wrap, bounds, line_height, first_row, visible });
        });
        Prepainted { lines, selection: selection_quads, caret }
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        prepainted: &mut Prepainted,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.area.read(cx).focus_handle.clone();
        window.handle_input(&focus_handle, ElementInputHandler::new(bounds, self.area.clone()), cx);
        let focused = focus_handle.is_focused(window);
        let line_height = window.line_height();
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            for quad in prepainted.selection.drain(..) {
                window.paint_quad(quad);
            }
            for (line, top) in &prepainted.lines {
                let height = line_height * (line.wrap_boundaries().len() + 1) as f32;
                // Lines wholly outside the area paint nothing.
                if *top + height < bounds.top() || *top > bounds.bottom() {
                    continue;
                }
                line.paint(point(bounds.left(), *top), line_height, TextAlign::Left, None, window, cx).ok();
            }
            if focused && let Some(caret) = prepainted.caret.take() {
                window.paint_quad(caret);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use gpui::{
        AppContext, ClipboardItem, Context, Entity, InteractiveElement, IntoElement, KeyBinding, ParentElement,
        Render, Styled, VisualTestContext, Window, actions, div, px,
    };

    use super::*;
    use crate::test_support;

    actions!(text_area_test, [OwnerEnter, OwnerUp, OwnerDown, OwnerTab, OwnerEscape]);

    /// An owner around a text area that binds the keys the area leaves it,
    /// and counts them.
    struct Host {
        area: Entity<TextArea>,
        owned: Vec<&'static str>,
    }

    impl Render for Host {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .key_context("Owner")
                .on_action(cx.listener(|host, _: &OwnerEnter, _, _| host.owned.push("enter")))
                .on_action(cx.listener(|host, _: &OwnerUp, _, _| host.owned.push("up")))
                .on_action(cx.listener(|host, _: &OwnerDown, _, _| host.owned.push("down")))
                .on_action(cx.listener(|host, _: &OwnerTab, _, _| host.owned.push("tab")))
                .on_action(cx.listener(|host, _: &OwnerEscape, _, _| host.owned.push("escape")))
                // 20 characters wide: the test text system's characters
                // are 0.6em, so 6px at 10px.
                .w(px(120.))
                .text_size(px(10.))
                .line_height(px(10.))
                .child(self.area.clone())
        }
    }

    /// A focused text area holding `text`, cursor at its end, in an owner.
    fn area(text: &str) -> (Entity<Host>, Entity<TextArea>, &'static mut VisualTestContext) {
        let mut app = test_support::app();
        app.update(|cx| {
            cx.bind_keys([
                KeyBinding::new("enter", OwnerEnter, Some("Owner")),
                KeyBinding::new("up", OwnerUp, Some("Owner")),
                KeyBinding::new("down", OwnerDown, Some("Owner")),
                KeyBinding::new("tab", OwnerTab, Some("Owner")),
                KeyBinding::new("escape", OwnerEscape, Some("Owner")),
            ])
        });
        let text = text.to_string();
        let (host, cx) = app.add_window_view(move |_, cx| Host { area: cx.new(|cx| TextArea::new(&text, cx)), owned: Vec::new() });
        let area = host.read_with(cx, |host, _| host.area.clone());
        cx.update(|window, cx| window.focus(&area.focus_handle(cx)));
        cx.run_until_parked();
        // `add_window_view` keeps its context until the test ends.
        let cx: &'static mut VisualTestContext = Box::leak(Box::new(cx.clone()));
        (host, area, cx)
    }

    fn text(area: &Entity<TextArea>, cx: &mut VisualTestContext) -> String {
        area.read_with(cx, |area, _| area.text().to_string())
    }

    fn owned(host: &Entity<Host>, cx: &mut VisualTestContext) -> Vec<&'static str> {
        host.read_with(cx, |host, _| host.owned.clone())
    }

    /// The spec's "Multi-line paste".
    #[test]
    fn a_paste_keeps_its_lines() {
        let (_, area, cx) = area("");
        cx.write_to_clipboard(ClipboardItem::new_string("one\r\ntwo".to_string()));
        cx.simulate_keystrokes("ctrl-v");
        assert_eq!(text(&area, cx), "one\ntwo");
        assert_eq!(area.read_with(cx, |area, _| area.rows()), Some(2));
    }

    /// The spec's "Wrapping": 30 characters in 20 show on two rows and
    /// hold no line break.
    #[test]
    fn long_lines_wrap_at_the_width() {
        let (_, area, cx) = area("");
        cx.simulate_input("abcdefghijklmnopqrstuvwxyz0123");
        assert_eq!(text(&area, cx), "abcdefghijklmnopqrstuvwxyz0123");
        assert_eq!(area.read_with(cx, |area, _| area.rows()), Some(2));
    }

    /// The spec's "Down keeps the column".
    #[test]
    fn down_keeps_the_column() {
        let (host, area, cx) = area("abcdef\nxy\nabcdef");
        cx.simulate_keystrokes("ctrl-home right right right right down down");
        assert_eq!(area.read_with(cx, |area, _| area.cursor()), 14);
        // On the last row, down is the owner's; so is up on the first.
        cx.simulate_keystrokes("down");
        cx.simulate_keystrokes("ctrl-home up");
        assert_eq!(owned(&host, cx), ["down", "up"]);
        // With shift there, the selection reaches the text's end instead.
        cx.simulate_keystrokes("shift-down shift-down shift-down");
        assert_eq!(area.read_with(cx, |area, _| area.selection()), 0..16);
        assert_eq!(owned(&host, cx), ["down", "up"]);
    }

    /// The spec's "Shift-enter".
    #[test]
    fn shift_enter_breaks_the_line() {
        let (_, area, cx) = area("ab");
        cx.simulate_keystrokes("ctrl-home right shift-enter");
        assert_eq!(text(&area, cx), "a\nb");
    }

    /// The spec's "Enter goes to the owner", and the other keys it leaves.
    #[test]
    fn enter_escape_and_tab_go_to_the_owner() {
        let (host, area, cx) = area("ab");
        cx.simulate_keystrokes("enter tab escape");
        assert_eq!(owned(&host, cx), ["enter", "tab", "escape"]);
        assert_eq!(text(&area, cx), "ab");
    }

    /// With no owner binding them, enter and tab still insert nothing.
    #[test]
    fn unbound_enter_and_tab_insert_nothing() {
        let mut app = test_support::app();
        let (area, cx) = app.add_window_view(|_, cx| TextArea::new("ab", cx));
        cx.update(|window, cx| window.focus(&area.focus_handle(cx)));
        cx.simulate_keystrokes("enter tab");
        assert_eq!(area.read_with(cx, |area, _| area.text().to_string()), "ab");
        assert!(is_unbound_key_text("\r\n") && !is_unbound_key_text("a\n"));
    }

    /// The spec's "Composing": enter commits the composition, which the
    /// input method sends, and never reaches the owner.
    #[test]
    fn enter_while_composing_is_not_the_owners() {
        let (host, area, cx) = area("ab");
        cx.update(|window, cx| {
            area.update(cx, |area, cx| area.replace_and_mark_text_in_range(None, "e", None, window, cx));
        });
        assert!(area.read_with(cx, |area, _| area.composing()));
        cx.simulate_keystrokes("enter");
        cx.update(|window, cx| area.update(cx, |area, cx| area.replace_text_in_range(None, "é", window, cx)));
        assert_eq!(text(&area, cx), "abé");
        assert!(!area.read_with(cx, |area, _| area.composing()));
        assert!(owned(&host, cx).is_empty());
        // Composed, enter is the owner's again.
        cx.simulate_keystrokes("enter");
        assert_eq!(owned(&host, cx), ["enter"]);
    }

    /// The spec's "Grows then scrolls": 12 lines in an area of 8 make it 8
    /// rows tall, showing the cursor's row.
    #[test]
    fn grows_to_its_lines_then_scrolls() {
        let (_, area, cx) = area("one");
        let height = |cx: &mut VisualTestContext| {
            area.read_with(cx, |area, _| area.painted.as_ref().map(|p| f32::from(p.bounds.size.height)))
        };
        assert_eq!(height(cx), Some(10.));
        cx.simulate_keystrokes("shift-enter shift-enter");
        assert_eq!(height(cx), Some(30.));
        let twelve: Vec<String> = (1..=12).map(|n| n.to_string()).collect();
        cx.update(|_, cx| area.update(cx, |area, cx| area.set_text(&twelve.join("\n"), cx)));
        cx.run_until_parked();
        assert_eq!(height(cx), Some(80.));
        assert_eq!(area.read_with(cx, |area, _| (area.shown(), area.rows())), (Some((4, 8)), Some(12)));
        // Back to the first line: its row comes into view.
        cx.simulate_keystrokes("ctrl-home");
        assert_eq!(area.read_with(cx, |area, _| area.shown()), Some((0, 8)));
        // An owner's lower limit holds too.
        cx.update(|_, cx| area.update(cx, |area, cx| area.set_max_lines(3, cx)));
        cx.run_until_parked();
        assert_eq!(height(cx), Some(30.));
    }

    /// Home and end go to the shown line's ends, ctrl-home and ctrl-end to
    /// the text's.
    #[test]
    fn home_and_end_of_the_shown_line() {
        let (_, area, cx) = area("abcdefghijklmnopqrstuvwxyz0123\nxy");
        let cursor = |cx: &mut VisualTestContext| area.read_with(cx, |area, _| area.cursor());
        cx.simulate_keystrokes("ctrl-home right right end");
        assert_eq!(cursor(cx), 19);
        cx.simulate_keystrokes("down home");
        assert_eq!(cursor(cx), 20);
        cx.simulate_keystrokes("end");
        assert_eq!(cursor(cx), 30);
        cx.simulate_keystrokes("ctrl-end");
        assert_eq!(cursor(cx), 33);
        cx.simulate_keystrokes("shift-home");
        assert_eq!(area.read_with(cx, |area, _| area.selection()), 31..33);
    }

    /// A read-only area moves and selects but changes nothing.
    #[test]
    fn a_read_only_area_changes_nothing() {
        let (_, area, cx) = area("ab");
        cx.update(|_, cx| area.update(cx, |area, cx| area.set_read_only(true, cx)));
        cx.simulate_input("x");
        cx.simulate_keystrokes("backspace shift-enter left");
        assert_eq!(text(&area, cx), "ab");
        assert_eq!(area.read_with(cx, |area, _| area.cursor()), 1);
    }

    /// The input method's window sits at the range on its wrapped row.
    #[test]
    fn caret_bounds_follow_wrapped_rows() {
        let (_, area, cx) = area("abcdefghijklmnopqrstuvwxyz0123");
        let bounds = cx.update(|window, cx| {
            area.update(cx, |area, cx| area.bounds_for_range(22..23, Bounds::default(), window, cx))
        });
        let bounds = bounds.expect("bounds on a painted area");
        let origin = area.read_with(cx, |area, _| area.painted.as_ref().map(|p| p.bounds.origin)).unwrap_or_default();
        assert_eq!(f32::from(bounds.top() - origin.y), 10.);
        assert_eq!(f32::from(bounds.left() - origin.x), 12.);
        assert_eq!(f32::from(bounds.size.width), 6.);
    }
}
