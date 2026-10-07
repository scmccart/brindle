//! A single-line text input: the palettes' filter and prompt, and the
//! settings dialog's fields.
//!
//! [`EditState`] holds the text, selection and IME composition and does all
//! the editing; it has no GPUI dependency, so it is unit-tested. [`TextInput`]
//! is the entity around it: focus, key actions, mouse, clipboard, and the
//! platform input handler that typed text and input methods arrive through.
//! [`TextInputElement`] shapes the line and paints the selection and caret.

use std::ops::Range;

use gpui::{
    App, Bounds, ClipboardItem, Context, ContentMask, CursorStyle, Element, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, EventEmitter, FocusHandle, Focusable, GlobalElementId, InteractiveElement, IntoElement,
    LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, PaintQuad, ParentElement, Pixels, Point,
    Render, ShapedLine, SharedString, Style, Styled, TextRun, UTF16Selection, UnderlineStyle, Window, div, fill,
    point, px, relative, size,
};

use crate::actions::{
    TextInputBackspace, TextInputCopy, TextInputCut, TextInputDelete, TextInputDeleteWordBack, TextInputEnd,
    TextInputHome, TextInputLeft, TextInputPaste, TextInputRight, TextInputSelectEnd, TextInputSelectHome,
    TextInputSelectLeft, TextInputSelectRight, TextInputSelectWordLeft, TextInputSelectWordRight, TextInputWordLeft,
    TextInputWordRight,
};

// ---- editing core -----------------------------------------------------------

/// A cursor movement.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Motion {
    Left,
    Right,
    WordLeft,
    WordRight,
    Home,
    End,
}

/// The text of an input and what is selected in it. Offsets are byte
/// indices into `text`, always on char boundaries. The selection runs from
/// `anchor` to `head`, where the cursor is; they are equal when nothing is
/// selected.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EditState {
    text: String,
    anchor: usize,
    head: usize,
    /// Text an input method is composing.
    marked: Option<Range<usize>>,
}

/// Input text is a single line: line breaks become spaces and other control
/// characters (a typed tab, say) are dropped.
pub fn single_line(text: &str) -> String {
    text.replace("\r\n", " ")
        .chars()
        .filter_map(|c| match c {
            '\r' | '\n' => Some(' '),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect()
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl EditState {
    /// `text` with the cursor at its end.
    pub fn new(text: &str) -> Self {
        let text = single_line(text);
        let end = text.len();
        Self { text, anchor: end, head: end, marked: None }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn cursor(&self) -> usize {
        self.head
    }

    pub fn selection(&self) -> Range<usize> {
        self.anchor.min(self.head)..self.anchor.max(self.head)
    }

    /// Whether the cursor is at the start of the selection.
    pub fn reversed(&self) -> bool {
        self.head < self.anchor
    }

    pub fn marked(&self) -> Option<Range<usize>> {
        self.marked.clone()
    }

    pub fn selected_text(&self) -> &str {
        &self.text[self.selection()]
    }

    /// Replaces the whole text, cursor at the end.
    pub fn set_text(&mut self, text: &str) {
        *self = Self::new(text);
    }

    pub fn select_all(&mut self) {
        self.anchor = 0;
        self.head = self.text.len();
    }

    /// Sets the selection; `reversed` puts the cursor at its start.
    pub fn select(&mut self, range: Range<usize>, reversed: bool) {
        let start = self.clamp(range.start);
        let end = self.clamp(range.end).max(start);
        (self.anchor, self.head) = if reversed { (end, start) } else { (start, end) };
    }

    /// Moves the cursor to `offset`, extending the selection when `extend`.
    pub fn move_to(&mut self, offset: usize, extend: bool) {
        self.head = self.clamp(offset);
        if !extend {
            self.anchor = self.head;
        }
    }

    fn clamp(&self, offset: usize) -> usize {
        let mut offset = offset.min(self.text.len());
        while !self.text.is_char_boundary(offset) {
            offset -= 1;
        }
        offset
    }

    fn prev_char(&self, offset: usize) -> usize {
        self.text[..offset].char_indices().next_back().map_or(0, |(ix, _)| ix)
    }

    fn next_char(&self, offset: usize) -> usize {
        self.text[offset..].chars().next().map_or(offset, |c| offset + c.len_utf8())
    }

    /// The start of the word before `offset` (skipping separators first).
    fn prev_word(&self, offset: usize) -> usize {
        let mut chars = self.text[..offset].char_indices().rev().peekable();
        while chars.next_if(|(_, c)| !is_word_char(*c)).is_some() {}
        let mut start = chars.peek().map_or(0, |(ix, c)| ix + c.len_utf8());
        while let Some((ix, _)) = chars.next_if(|(_, c)| is_word_char(*c)) {
            start = ix;
        }
        start
    }

    /// The end of the word after `offset` (skipping separators first).
    fn next_word(&self, offset: usize) -> usize {
        let rest = &self.text[offset..];
        let mut chars = rest.char_indices().peekable();
        while chars.next_if(|(_, c)| !is_word_char(*c)).is_some() {}
        while chars.next_if(|(_, c)| is_word_char(*c)).is_some() {}
        offset + chars.peek().map_or(rest.len(), |(ix, _)| *ix)
    }

    /// Applies a movement. Without `extend`, left and right collapse a
    /// selection to its start or end.
    pub fn motion(&mut self, motion: Motion, extend: bool) {
        let selection = self.selection();
        let target = match motion {
            Motion::Left if !extend && !selection.is_empty() => selection.start,
            Motion::Right if !extend && !selection.is_empty() => selection.end,
            Motion::Left => self.prev_char(self.head),
            Motion::Right => self.next_char(self.head),
            Motion::WordLeft => self.prev_word(self.head),
            Motion::WordRight => self.next_word(self.head),
            Motion::Home => 0,
            Motion::End => self.text.len(),
        };
        self.move_to(target, extend);
    }

    /// Replaces `range` with `new` and puts the cursor after it.
    pub fn replace(&mut self, range: Range<usize>, new: &str) {
        let new = single_line(new);
        let start = self.clamp(range.start);
        let range = start..self.clamp(range.end).max(start);
        self.text.replace_range(range.clone(), &new);
        self.move_to(range.start + new.len(), false);
        self.marked = None;
    }

    /// Typing and pasting: replaces the composition or the selection.
    pub fn insert(&mut self, new: &str) {
        let range = self.marked.clone().unwrap_or_else(|| self.selection());
        self.replace(range, new);
    }

    /// An input method's composition: replaces `range` (the current
    /// composition or selection when `None`) with `new`, marks it, and
    /// selects `selected` within it (the cursor after it when `None`).
    pub fn replace_and_mark(&mut self, range: Option<Range<usize>>, new: &str, selected: Option<Range<usize>>) {
        let range = range.or_else(|| self.marked.clone()).unwrap_or_else(|| self.selection());
        let new = single_line(new);
        let start = self.clamp(range.start);
        self.replace(start..range.end, &new);
        self.marked = (!new.is_empty()).then(|| start..start + new.len());
        match selected {
            Some(sel) => self.select(start + sel.start..start + sel.end, false),
            None => self.move_to(start + new.len(), false),
        }
    }

    pub fn unmark(&mut self) {
        self.marked = None;
    }

    /// Deletes the selection, or the range from the cursor to `target`.
    fn delete_to(&mut self, target: impl FnOnce(&Self) -> usize) {
        let selection = self.selection();
        let range = if selection.is_empty() {
            let target = target(self);
            self.head.min(target)..self.head.max(target)
        } else {
            selection
        };
        self.replace(range, "");
    }

    pub fn backspace(&mut self) {
        self.delete_to(|s| s.prev_char(s.head));
    }

    pub fn delete(&mut self) {
        self.delete_to(|s| s.next_char(s.head));
    }

    pub fn delete_word_back(&mut self) {
        self.delete_to(|s| s.prev_word(s.head));
    }

    /// Double-click: selects the word (or run of separators) at `offset`.
    pub fn select_word_at(&mut self, offset: usize) {
        let offset = self.clamp(offset);
        let word = |c: char| is_word_char(c);
        let kind = self.text[offset..].chars().next().or_else(|| self.text[..offset].chars().next_back());
        let Some(kind) = kind.map(word) else { return };
        let start = self.text[..offset]
            .char_indices()
            .rev()
            .take_while(|(_, c)| word(*c) == kind)
            .last()
            .map_or(offset, |(ix, _)| ix);
        let end = offset
            + self.text[offset..].char_indices().find(|(_, c)| word(*c) != kind).map_or(self.text.len() - offset, |(ix, _)| ix);
        self.select(start..end, false);
    }

    pub fn to_utf16(&self, offset: usize) -> usize {
        self.text[..self.clamp(offset)].encode_utf16().count()
    }

    pub fn from_utf16(&self, offset: usize) -> usize {
        let mut units = 0;
        for (ix, c) in self.text.char_indices() {
            if units >= offset {
                return ix;
            }
            units += c.len_utf16();
        }
        self.text.len()
    }

    pub fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.to_utf16(range.start)..self.to_utf16(range.end)
    }

    pub fn range_from_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.from_utf16(range.start)..self.from_utf16(range.end)
    }

    /// Debug description for `--dump-screen-after`.
    pub fn describe(&self) -> String {
        let sel = self.selection();
        format!("text={:?} cursor={} selection={}..{}", self.text, self.head, sel.start, sel.end)
    }
}

/// The horizontal scroll that keeps the caret visible: unchanged while the
/// caret is inside the field, otherwise just enough to bring it in. Text
/// narrower than the field is never scrolled.
pub fn scroll_for_caret(scroll: f32, caret_x: f32, width: f32, text_width: f32) -> f32 {
    const CARET: f32 = 2.0;
    let max = (text_width + CARET - width).max(0.0);
    let scroll = if caret_x < scroll {
        caret_x
    } else if caret_x + CARET > scroll + width {
        caret_x + CARET - width
    } else {
        scroll
    };
    scroll.clamp(0.0, max)
}

// ---- entity -----------------------------------------------------------------

pub enum TextInputEvent {
    /// The text changed through the user (typing, paste, deletion, IME).
    Changed,
}

pub struct TextInput {
    focus_handle: FocusHandle,
    state: EditState,
    placeholder: SharedString,
    /// Horizontal scroll of the text, in pixels.
    scroll_x: f32,
    /// The last painted line and where its first glyph starts, for mouse
    /// positioning and the input method's geometry.
    last_line: Option<ShapedLine>,
    last_bounds: Option<Bounds<Pixels>>,
    text_origin_x: Pixels,
    /// A drag selection is in progress.
    selecting: bool,
}

impl EventEmitter<TextInputEvent> for TextInput {}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl TextInput {
    pub fn new(text: &str, cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            state: EditState::new(text),
            placeholder: SharedString::default(),
            scroll_x: 0.0,
            last_line: None,
            last_bounds: None,
            text_origin_x: px(0.0),
            selecting: false,
        }
    }

    pub fn with_placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// Makes the input a tab stop, so `Window::focus_next` reaches it.
    pub fn tab_stop(mut self) -> Self {
        self.focus_handle = self.focus_handle.tab_stop(true);
        self
    }

    pub fn text(&self) -> &str {
        self.state.text()
    }

    /// Replaces the text from the owner. This emits no `Changed`.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if self.state.text() != text {
            self.state.set_text(text);
            cx.notify();
        }
    }

    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        self.state.select_all();
        cx.notify();
    }

    /// Inserts text as if typed (the `--send` debug hook).
    pub fn insert(&mut self, text: &str, cx: &mut Context<Self>) {
        self.edit(cx, |s| s.insert(text));
    }

    pub fn is_focused(&self, window: &Window) -> bool {
        self.focus_handle.is_focused(window)
    }

    pub fn describe(&self) -> String {
        self.state.describe()
    }

    /// Runs an edit, redraws, and reports a change of text.
    fn edit(&mut self, cx: &mut Context<Self>, f: impl FnOnce(&mut EditState)) {
        let old = self.state.text().to_string();
        f(&mut self.state);
        if old != self.state.text() {
            cx.emit(TextInputEvent::Changed);
        }
        cx.notify();
    }

    fn motion(&mut self, motion: Motion, extend: bool, cx: &mut Context<Self>) {
        self.state.motion(motion, extend);
        cx.notify();
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
        if !text.is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
            self.edit(cx, |s| s.insert(""));
        }
    }
    fn paste(&mut self, _: &TextInputPaste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.edit(cx, |s| s.insert(&text));
        }
    }

    /// The text offset under a window position.
    fn offset_for_position(&self, position: Point<Pixels>) -> usize {
        let (Some(line), Some(bounds)) = (&self.last_line, self.last_bounds) else { return 0 };
        if self.state.text().is_empty() {
            return 0;
        }
        if position.y < bounds.top() {
            return 0;
        }
        if position.y > bounds.bottom() {
            return self.state.text().len();
        }
        line.closest_index_for_x(position.x - self.text_origin_x)
    }

    fn mouse_down(&mut self, e: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle);
        let offset = self.offset_for_position(e.position);
        if e.click_count >= 2 {
            self.state.select_word_at(offset);
        } else {
            self.state.move_to(offset, e.modifiers.shift);
            self.selecting = true;
        }
        cx.notify();
    }

    fn mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.selecting = false;
    }

    fn mouse_move(&mut self, e: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.selecting && e.pressed_button == Some(MouseButton::Left) {
            self.state.move_to(self.offset_for_position(e.position), true);
            cx.notify();
        }
    }
}

impl EntityInputHandler for TextInput {
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
            let selected = selected_utf16.map(|r| {
                let state = EditState::new(text);
                state.range_from_utf16(&r)
            });
            s.replace_and_mark(range, text, selected);
        });
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let line = self.last_line.as_ref()?;
        let range = self.state.range_from_utf16(&range_utf16);
        let left = self.text_origin_x;
        Some(Bounds::from_corners(
            point(left + line.x_for_index(range.start), bounds.top()),
            point(left + line.x_for_index(range.end), bounds.bottom()),
        ))
    }

    fn character_index_for_point(
        &mut self,
        position: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        let line = self.last_line.as_ref()?;
        let index = line.index_for_x(position.x - self.text_origin_x)?;
        Some(self.state.to_utf16(index))
    }
}

impl Render for TextInput {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context("TextInput")
            .track_focus(&self.focus_handle)
            .cursor(CursorStyle::IBeam)
            .flex()
            .flex_1()
            .min_w_0()
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
            .child(TextInputElement { input: cx.entity() })
    }
}

// ---- element ----------------------------------------------------------------

/// Paints a [`TextInput`]'s line, in the inherited text style: the
/// selection and caret use the text color, the placeholder a faded one.
struct TextInputElement {
    input: Entity<TextInput>,
}

struct Prepainted {
    line: Option<ShapedLine>,
    origin_x: Pixels,
    selection: Option<PaintQuad>,
    caret: Option<PaintQuad>,
}

impl IntoElement for TextInputElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextInputElement {
    type RequestLayoutState = ();
    type PrepaintState = Prepainted;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
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
        let input = self.input.read(cx);
        let state = &input.state;
        let style = window.text_style();
        let color = style.color;
        let placeholder = state.text().is_empty() && state.marked().is_none();
        let (text, text_color): (SharedString, _) = if placeholder {
            (input.placeholder.clone(), color.opacity(0.45))
        } else {
            (state.text().to_string().into(), color)
        };
        let run = TextRun {
            len: text.len(),
            font: style.font(),
            color: text_color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs: Vec<TextRun> = match state.marked().filter(|_| !placeholder) {
            Some(marked) => [
                TextRun { len: marked.start, ..run.clone() },
                TextRun {
                    len: marked.end - marked.start,
                    underline: Some(UnderlineStyle { color: Some(color), thickness: px(1.0), wavy: false }),
                    ..run.clone()
                },
                TextRun { len: text.len() - marked.end, ..run },
            ]
            .into_iter()
            .filter(|r| r.len > 0)
            .collect(),
            None => vec![run],
        };
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window.text_system().shape_line(text, font_size, &runs, None);

        let caret_x = if placeholder { px(0.0) } else { line.x_for_index(state.cursor()) };
        let scroll = scroll_for_caret(input.scroll_x, caret_x.into(), bounds.size.width.into(), line.width.into());
        let origin_x = bounds.left() - px(scroll);

        let selection = state.selection();
        let selection = (!selection.is_empty()).then(|| {
            fill(
                Bounds::from_corners(
                    point(origin_x + line.x_for_index(selection.start), bounds.top()),
                    point(origin_x + line.x_for_index(selection.end), bounds.bottom()),
                ),
                color.opacity(0.28),
            )
        });
        let caret = fill(Bounds::new(point(origin_x + caret_x, bounds.top()), size(px(1.5), bounds.size.height)), color);
        self.input.update(cx, |input, _| input.scroll_x = scroll);
        Prepainted { line: Some(line), origin_x, selection, caret: Some(caret) }
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
        let focus_handle = self.input.read(cx).focus_handle.clone();
        window.handle_input(&focus_handle, ElementInputHandler::new(bounds, self.input.clone()), cx);
        let focused = focus_handle.is_focused(window);
        let Some(line) = prepainted.line.take() else { return };
        window.with_content_mask(Some(ContentMask { bounds }), |window| {
            if let Some(selection) = prepainted.selection.take() {
                window.paint_quad(selection);
            }
            line.paint(point(prepainted.origin_x, bounds.top()), window.line_height(), window, cx).ok();
            if focused && let Some(caret) = prepainted.caret.take() {
                window.paint_quad(caret);
            }
        });
        let origin_x = prepainted.origin_x;
        self.input.update(cx, |input, _| {
            input.last_line = Some(line);
            input.last_bounds = Some(bounds);
            input.text_origin_x = origin_x;
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A state with the cursor at the `|` in `marked`.
    fn at(marked: &str) -> EditState {
        let cursor = marked.find('|').unwrap();
        let mut s = EditState::new(&marked.replace('|', ""));
        s.move_to(cursor, false);
        s
    }

    /// The state written with `|` at the cursor, or `[...]` around the
    /// selection.
    fn show(s: &EditState) -> String {
        let sel = s.selection();
        let t = s.text();
        if sel.is_empty() {
            format!("{}|{}", &t[..sel.start], &t[sel.start..])
        } else {
            format!("{}[{}]{}", &t[..sel.start], &t[sel.clone()], &t[sel.end..])
        }
    }

    #[test]
    fn insert_in_the_middle() {
        let mut s = at("t|mx");
        s.insert("u");
        assert_eq!(show(&s), "tu|mx");
    }

    #[test]
    fn multibyte_characters() {
        let mut s = at("café|");
        s.motion(Motion::Left, false);
        assert_eq!(show(&s), "caf|é");
        s.backspace();
        assert_eq!(show(&s), "ca|é");
        s.delete();
        assert_eq!(show(&s), "ca|");
        // Offsets past the end or inside a character are clamped.
        let mut s = EditState::new("é");
        s.move_to(1, false);
        assert_eq!(s.cursor(), 0);
        s.move_to(9, false);
        assert_eq!(s.cursor(), 2);
    }

    #[test]
    fn word_movement() {
        let mut s = at("tmux -L work|");
        s.motion(Motion::WordLeft, false);
        assert_eq!(show(&s), "tmux -L |work");
        s.motion(Motion::WordLeft, false);
        assert_eq!(show(&s), "tmux -|L work");
        s.motion(Motion::WordRight, false);
        assert_eq!(show(&s), "tmux -L| work");
        s.motion(Motion::WordRight, false);
        assert_eq!(show(&s), "tmux -L work|");
        s.motion(Motion::WordRight, false);
        assert_eq!(show(&s), "tmux -L work|");
        let mut s = at("|  ab_c9 x");
        s.motion(Motion::WordRight, false);
        assert_eq!(show(&s), "  ab_c9| x");
    }

    #[test]
    fn home_and_end() {
        let mut s = at("ab|c");
        s.motion(Motion::Home, false);
        s.insert("x");
        assert_eq!(show(&s), "x|abc");
        s.motion(Motion::End, false);
        assert_eq!(show(&s), "xabc|");
    }

    #[test]
    fn deleting() {
        let mut s = at("~/src/brindle|");
        s.delete_word_back();
        assert_eq!(show(&s), "~/src/|");
        s.delete_word_back();
        assert_eq!(show(&s), "~/|");
        let mut s = at("|ab");
        s.backspace();
        assert_eq!(show(&s), "|ab");
        let mut s = at("ab|");
        s.delete();
        assert_eq!(show(&s), "ab|");
        // A selection is deleted instead.
        let mut s = at("abcd|");
        s.motion(Motion::Left, true);
        s.motion(Motion::Left, true);
        s.delete();
        assert_eq!(show(&s), "ab|");
    }

    #[test]
    fn selection() {
        let mut s = at("main|");
        s.motion(Motion::Home, true);
        assert_eq!(show(&s), "[main]");
        assert!(s.reversed());
        assert_eq!(s.selected_text(), "main");
        s.insert("work");
        assert_eq!(show(&s), "work|");
        // Left and right collapse a selection to its ends.
        let mut s = at("a|bcd");
        s.motion(Motion::Right, true);
        s.motion(Motion::Right, true);
        assert_eq!(show(&s), "a[bc]d");
        let mut left = s.clone();
        left.motion(Motion::Left, false);
        assert_eq!(show(&left), "a|bcd");
        s.motion(Motion::Right, false);
        assert_eq!(show(&s), "abc|d");
        // Shrinking past the anchor flips the selection.
        let mut s = at("ab|cd");
        s.motion(Motion::Right, true);
        s.motion(Motion::Left, true);
        s.motion(Motion::Left, true);
        assert_eq!(show(&s), "a[b]cd");
        assert!(s.reversed());
    }

    #[test]
    fn paste_replaces_the_selection_on_one_line() {
        let mut s = EditState::new("/bin/bash");
        s.select(5..9, false);
        s.insert("zsh");
        assert_eq!(show(&s), "/bin/zsh|");
        let mut s = EditState::new("");
        s.insert("one\ntwo\r\nthree\tfour");
        assert_eq!(s.text(), "one two threefour");
        assert_eq!(EditState::new("a\rb").text(), "a b");
    }

    #[test]
    fn mouse_selection() {
        // Press, then drag left and right of the anchor.
        let mut s = EditState::new("/bin/zsh");
        s.move_to(5, false);
        s.move_to(8, true);
        assert_eq!(show(&s), "/bin/[zsh]");
        s.move_to(1, true);
        assert_eq!(show(&s), "/[bin/]zsh");
        // Shift-click extends from the anchor.
        s.move_to(5, false);
        s.move_to(0, true);
        assert_eq!(show(&s), "[/bin/]zsh");
    }

    #[test]
    fn double_click_selects_a_word() {
        let mut s = EditState::new("tmux -L work_1");
        s.select_word_at(10);
        assert_eq!(show(&s), "tmux -L [work_1]");
        s.select_word_at(14);
        assert_eq!(show(&s), "tmux -L [work_1]");
        s.select_word_at(4);
        assert_eq!(show(&s), "tmux[ -]L work_1");
        let mut empty = EditState::new("");
        empty.select_word_at(0);
        assert_eq!(show(&empty), "|");
    }

    #[test]
    fn utf16_conversions() {
        // 'é' is 2 bytes / 1 unit, '😀' is 4 bytes / 2 units.
        let s = EditState::new("aé😀b");
        assert_eq!(s.to_utf16(0), 0);
        assert_eq!(s.to_utf16(1), 1);
        assert_eq!(s.to_utf16(3), 2);
        assert_eq!(s.to_utf16(7), 4);
        assert_eq!(s.to_utf16(8), 5);
        assert_eq!(s.from_utf16(2), 3);
        assert_eq!(s.from_utf16(4), 7);
        assert_eq!(s.from_utf16(5), 8);
        assert_eq!(s.from_utf16(99), 8);
        assert_eq!(s.range_from_utf16(&(1..4)), 1..7);
        assert_eq!(s.range_to_utf16(&(1..7)), 1..4);
    }

    #[test]
    fn composition() {
        let mut s = at("ab|");
        s.replace_and_mark(None, "e", None);
        assert_eq!(s.marked(), Some(2..3));
        s.replace_and_mark(None, "é", Some(0..0));
        assert_eq!(s.text(), "abé");
        assert_eq!(s.marked(), Some(2..4));
        assert_eq!(s.cursor(), 2);
        // Committing replaces the composition.
        s.insert("é");
        assert_eq!(show(&s), "abé|");
        assert_eq!(s.marked(), None);
        // A composition started over a selection replaces it.
        let mut s = EditState::new("xyz");
        s.select(0..3, false);
        s.replace_and_mark(None, "k", None);
        assert_eq!(s.text(), "k");
        s.insert("か");
        assert_eq!(show(&s), "か|");
        // An empty composition clears the mark.
        let mut s = at("a|");
        s.replace_and_mark(None, "x", None);
        s.replace_and_mark(None, "", None);
        assert_eq!((s.text(), s.marked()), ("a", None));
    }

    #[test]
    fn caret_scroll() {
        // Fits: never scrolled.
        assert_eq!(scroll_for_caret(0.0, 50.0, 100.0, 60.0), 0.0);
        // Caret past the right edge: scroll just enough.
        assert_eq!(scroll_for_caret(0.0, 150.0, 100.0, 200.0), 52.0);
        // Caret inside the visible part: unchanged.
        assert_eq!(scroll_for_caret(52.0, 100.0, 100.0, 200.0), 52.0);
        // Caret left of the visible part: scroll back to it.
        assert_eq!(scroll_for_caret(52.0, 20.0, 100.0, 200.0), 20.0);
        // Text got shorter: no empty space after it.
        assert_eq!(scroll_for_caret(80.0, 90.0, 100.0, 120.0), 22.0);
    }

    #[test]
    fn describe() {
        let mut s = EditState::new("split");
        s.move_to(4, false);
        assert_eq!(s.describe(), "text=\"split\" cursor=4 selection=4..4");
    }
}
