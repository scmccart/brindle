//! The palette: a small filterable list shown over the terminal, used for
//! both the new-tab (profile) palette and the command palette. It can also
//! turn into a one-line prompt for commands that need text.

use std::rc::Rc;

use futures::channel::oneshot;
use gpui::{
    App, Context, EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement,
    KeyDownEvent, Keystroke, MouseButton, ParentElement, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement, Styled, Task, Window, div, px,
};

use crate::actions::{Paste, PickerCancel, PickerConfirm, PickerDown, PickerUp};
use crate::theme::Theme;

pub enum PaletteEvent {
    /// A row was chosen; the index is into the rows the palette was built with.
    Confirmed(usize),
    Dismissed,
}

pub struct PaletteRow {
    pub label: SharedString,
    pub detail: Option<SharedString>,
    pub shortcut: Option<SharedString>,
}

/// Runs a prompt's text. The palette closes on `Ok` or if the sender is
/// dropped, and shows the message on `Err`.
pub type SubmitFn = Rc<dyn Fn(String, &mut App) -> oneshot::Receiver<Result<(), String>>>;

/// A request to ask the user for one line of text.
#[derive(Clone)]
pub struct PromptRequest {
    pub label: SharedString,
    pub initial: String,
    pub submit: SubmitFn,
}

/// One line of editable text: typing appends, backspace deletes the last
/// character, and pasted line breaks become spaces. Shared by the palettes
/// and the settings dialog; there is no cursor movement within the text.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TextField {
    pub text: String,
}

impl TextField {
    pub fn new(text: &str) -> Self {
        Self { text: single_line(text) }
    }

    pub fn paste(&mut self, text: &str) {
        self.text.push_str(&single_line(text));
    }

    /// Whether `ks` edits text (as opposed to being a shortcut or a
    /// navigation key that a keybinding handles).
    pub fn is_edit_key(ks: &Keystroke) -> bool {
        !(ks.modifiers.control || ks.modifiers.alt || ks.modifiers.platform)
            && !matches!(ks.key.as_str(), "enter" | "tab" | "escape")
            && (ks.key == "backspace" || ks.key_char.is_some())
    }

    /// Applies an edit key (see [`Self::is_edit_key`]).
    pub fn apply_key(&mut self, ks: &Keystroke) {
        if ks.key == "backspace" {
            self.text.pop();
        } else if let Some(ch) = ks.key_char.as_deref() {
            self.text.push_str(&single_line(ch));
        }
    }
}

struct Prompt {
    label: SharedString,
    text: TextField,
    error: Option<String>,
    submit: SubmitFn,
    /// Waiting for the submitted command's result.
    running: Option<Task<()>>,
}

enum Mode {
    List(List),
    Prompt(Prompt),
}

struct List {
    rows: Vec<PaletteRow>,
    placeholder: SharedString,
    query: TextField,
    /// Indices of the rows matching `query`, refreshed on every edit.
    matches: Vec<usize>,
    /// Index into `matches`.
    selected: usize,
}

impl List {
    fn refilter(&mut self) {
        self.matches = filter(&self.query.text, self.rows.iter().map(|r| r.label.as_ref()));
        self.selected = 0;
    }
}

pub struct Palette {
    focus_handle: FocusHandle,
    scroll: ScrollHandle,
    mode: Mode,
    theme: Theme,
}

impl EventEmitter<PaletteEvent> for Palette {}

impl Focusable for Palette {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

/// Case-insensitive subsequence match.
pub fn fuzzy_match(query: &str, candidate: &str) -> bool {
    let mut chars = candidate.chars().flat_map(char::to_lowercase);
    query
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|c| !c.is_whitespace())
        .all(|q| chars.any(|c| c == q))
}

/// Indices of the labels matching `query`, in their original order.
pub fn filter<'a>(query: &str, labels: impl IntoIterator<Item = &'a str>) -> Vec<usize> {
    labels
        .into_iter()
        .enumerate()
        .filter(|(_, label)| fuzzy_match(query, label))
        .map(|(ix, _)| ix)
        .collect()
}

/// Prompt text is a single line: line breaks become spaces.
pub fn single_line(text: &str) -> String {
    text.replace("\r\n", " ").replace(['\r', '\n'], " ")
}

/// A prompt result that is already known.
pub fn ready(result: Result<(), String>) -> oneshot::Receiver<Result<(), String>> {
    let (tx, rx) = oneshot::channel();
    tx.send(result).ok();
    rx
}

impl Palette {
    pub fn list(rows: Vec<PaletteRow>, placeholder: impl Into<SharedString>, theme: Theme, cx: &mut Context<Self>) -> Self {
        let matches = (0..rows.len()).collect();
        let list = List { rows, placeholder: placeholder.into(), query: TextField::default(), matches, selected: 0 };
        Self { focus_handle: cx.focus_handle(), scroll: ScrollHandle::new(), mode: Mode::List(list), theme }
    }

    pub fn prompt(request: PromptRequest, theme: Theme, cx: &mut Context<Self>) -> Self {
        let prompt = Prompt {
            label: request.label,
            text: TextField::new(&request.initial),
            error: None,
            submit: request.submit,
            running: None,
        };
        Self { focus_handle: cx.focus_handle(), scroll: ScrollHandle::new(), mode: Mode::Prompt(prompt), theme }
    }

    /// Debug description for `--dump-screen-after`.
    pub fn describe(&self) -> String {
        match &self.mode {
            Mode::List(list) => {
                let mut out = format!("--- palette list query={:?}\n", list.query.text);
                for &ix in &list.matches {
                    let row = &list.rows[ix];
                    out.push_str(&format!("{}", row.label));
                    if let Some(shortcut) = &row.shortcut {
                        out.push_str(&format!("  [{shortcut}]"));
                    }
                    out.push('\n');
                }
                out
            }
            Mode::Prompt(p) => format!(
                "--- palette prompt {:?} text={:?} error={:?} running={}\n",
                p.label,
                p.text.text,
                p.error,
                p.running.is_some()
            ),
        }
    }

    fn move_selection(&mut self, forward: bool, cx: &mut Context<Self>) {
        if let Mode::List(list) = &mut self.mode
            && !list.matches.is_empty()
        {
            let count = list.matches.len();
            list.selected = if forward { (list.selected + 1) % count } else { (list.selected + count - 1) % count };
            self.scroll.scroll_to_item(list.selected);
            cx.notify();
        }
    }

    fn up(&mut self, _: &PickerUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(false, cx);
    }

    fn down(&mut self, _: &PickerDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_selection(true, cx);
    }

    fn confirm(&mut self, _: &PickerConfirm, _: &mut Window, cx: &mut Context<Self>) {
        match &mut self.mode {
            Mode::List(list) => {
                if let Some(&ix) = list.matches.get(list.selected) {
                    cx.emit(PaletteEvent::Confirmed(ix));
                }
            }
            Mode::Prompt(prompt) => {
                if prompt.running.is_some() {
                    return;
                }
                prompt.error = None;
                let result = (prompt.submit)(prompt.text.text.clone(), cx);
                prompt.running = Some(cx.spawn(async move |this, cx| {
                    let result = result.await;
                    this.update(cx, |this, cx| {
                        let Mode::Prompt(prompt) = &mut this.mode else { return };
                        prompt.running = None;
                        match result {
                            Ok(Err(message)) => {
                                prompt.error = Some(message);
                                cx.notify();
                            }
                            // Success, or the command went away (e.g. tmux exited).
                            Ok(Ok(())) | Err(_) => cx.emit(PaletteEvent::Dismissed),
                        }
                    })
                    .ok();
                }));
                cx.notify();
            }
        }
    }

    fn cancel(&mut self, _: &PickerCancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(PaletteEvent::Dismissed);
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.edit(|field| field.paste(&text), cx);
        }
    }

    /// Applies an edit to the query or prompt text.
    fn edit(&mut self, f: impl FnOnce(&mut TextField), cx: &mut Context<Self>) {
        match &mut self.mode {
            Mode::List(list) => {
                f(&mut list.query);
                list.refilter();
                self.scroll.scroll_to_item(0);
            }
            Mode::Prompt(prompt) => {
                if prompt.running.is_some() {
                    return;
                }
                f(&mut prompt.text);
                prompt.error = None;
            }
        }
        cx.notify();
    }

    fn key_down(&mut self, e: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let ks = &e.keystroke;
        if !TextField::is_edit_key(ks) {
            return;
        }
        self.edit(|field| field.apply_key(ks), cx);
        cx.stop_propagation();
    }

    fn render_list(&self, list: &List, cx: &mut Context<Self>) -> gpui::AnyElement {
        let t = &self.theme;
        let fg = t.foreground.hsla();
        let muted = t.muted_foreground().hsla();
        let panel = t.chrome_background();
        let accent = t.accent();

        let items = list.matches.iter().enumerate().map(|(n, &ix)| {
            let row = &list.rows[ix];
            let is_selected = n == list.selected;
            div()
                .id(("row", ix))
                .flex()
                .flex_none()
                .items_center()
                .justify_between()
                .gap_4()
                .px_3()
                .py_1p5()
                .rounded_md()
                .when(is_selected, |d| d.bg(panel.mix(accent, 0.22).hsla()))
                .hover(|d| d.bg(panel.mix(t.foreground, 0.08).hsla()))
                .on_click(cx.listener(move |_, _, _, cx| cx.emit(PaletteEvent::Confirmed(ix))))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .child(div().text_color(fg).child(row.label.clone()))
                        .when_some(row.detail.clone(), |d, detail| {
                            d.child(div().text_xs().text_color(muted).child(detail))
                        }),
                )
                .when_some(row.shortcut.clone(), |d, shortcut| {
                    d.child(div().flex_none().text_xs().text_color(muted).child(shortcut))
                })
        });

        let query = &list.query.text;
        let header: SharedString = if query.is_empty() { list.placeholder.clone() } else { query.clone().into() };
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .px_3()
                    .py_2()
                    .mb_1()
                    .border_b_1()
                    .border_color(t.chrome_border().hsla())
                    .text_color(if query.is_empty() { muted } else { fg })
                    .child(header),
            )
            .child(
                div()
                    .id("palette-rows")
                    .flex()
                    .flex_col()
                    .max_h(px(360.0))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .children(items),
            )
            .when(list.matches.is_empty(), |d| {
                d.child(div().px_3().py_2().text_color(muted).child("Nothing matches"))
            })
            .into_any_element()
    }

    fn render_prompt(&self, prompt: &Prompt) -> gpui::AnyElement {
        let t = &self.theme;
        let muted = t.muted_foreground().hsla();
        div()
            .flex()
            .flex_col()
            .gap_1()
            .px_3()
            .py_2()
            .child(div().text_xs().text_color(muted).child(prompt.label.clone()))
            .child(
                div()
                    .flex()
                    .text_color(t.foreground.hsla())
                    .child(SharedString::from(prompt.text.text.clone()))
                    .child(div().w(px(1.5)).h(px(16.0)).bg(t.accent().hsla())),
            )
            .when(prompt.running.is_some(), |d| d.child(div().text_xs().text_color(muted).child("Running…")))
            .when_some(prompt.error.clone(), |d, error| {
                d.child(div().text_xs().text_color(t.ansi[1].hsla()).child(SharedString::from(error)))
            })
            .into_any_element()
    }
}

impl Render for Palette {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match &self.mode {
            Mode::List(list) => self.render_list(list, cx),
            Mode::Prompt(prompt) => self.render_prompt(prompt),
        };
        let t = &self.theme;
        div()
            .key_context("Palette")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::paste))
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(PaletteEvent::Dismissed)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .w(px(460.0))
            .flex()
            .flex_col()
            .p_2()
            .bg(t.chrome_background().hsla())
            .border_1()
            .border_color(t.chrome_border().hsla())
            .rounded_lg()
            .shadow_lg()
            .text_sm()
            .child(content)
    }
}

use gpui::prelude::FluentBuilder as _;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fuzzy() {
        assert!(fuzzy_match("", "Shell"));
        assert!(fuzzy_match("tm", "tmux (classic)"));
        assert!(fuzzy_match("TC", "tmux (classic)"));
        assert!(fuzzy_match("tmux cl", "tmux (classic)"));
        assert!(!fuzzy_match("zsh", "Shell"));
    }

    #[test]
    fn filtering_keeps_order() {
        let labels = ["Reload Config", "Clear Scrollback", "Close Tab", "tmux: Split Right"];
        assert_eq!(filter("", labels), vec![0, 1, 2, 3]);
        assert_eq!(filter("c", labels), vec![0, 1, 2]);
        assert_eq!(filter("clrsc", labels), vec![1]);
        assert!(filter("zzz", labels).is_empty());
    }

    #[test]
    fn text_field_edits() {
        let key = |s: &str| Keystroke::parse(s).unwrap();
        let mut field = TextField::new("a\nb");
        assert_eq!(field.text, "a b");
        field.paste("c\r\nd");
        assert_eq!(field.text, "a bc d");
        assert!(TextField::is_edit_key(&key("backspace")));
        field.apply_key(&key("backspace"));
        assert_eq!(field.text, "a bc ");
        for k in ["enter", "tab", "escape", "ctrl-a", "up"] {
            assert!(!TextField::is_edit_key(&key(k)), "{k}");
        }
    }

    #[test]
    fn prompt_text_is_one_line() {
        assert_eq!(single_line("a\nb\r\nc\rd"), "a b c d");
        assert_eq!(single_line("plain"), "plain");
    }
}
