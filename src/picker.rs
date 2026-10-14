//! The palette: a small filterable list shown over the terminal, used for
//! both the new-tab (profile) palette and the command palette. It can also
//! turn into a one-line prompt for commands that need text.

use std::rc::Rc;

use futures::channel::oneshot;
use gpui::{
    App, AppContext as _, Context, CursorStyle, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement,
    MouseButton, ParentElement, Render, ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Subscription,
    Task, Window, div, px,
};

use crate::actions::{PickerCancel, PickerConfirm, PickerDown, PickerUp};
use crate::text_input::{TextInput, TextInputEvent};
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

struct Prompt {
    label: SharedString,
    input: Entity<TextInput>,
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
    query: Entity<TextInput>,
    /// Indices of the rows matching `query`, refreshed on every edit.
    matches: Vec<usize>,
    /// Index into `matches`.
    selected: usize,
}

impl List {
    fn refilter(&mut self, cx: &App) {
        self.matches = filter(self.query.read(cx).text(), self.rows.iter().map(|r| r.label.as_ref()));
        self.selected = 0;
    }
}

pub struct Palette {
    scroll: ScrollHandle,
    mode: Mode,
    theme: Theme,
    /// A click outside dismisses the palette (unless made modal).
    dismiss_on_click_outside: bool,
    _input_subscription: Subscription,
}

impl EventEmitter<PaletteEvent> for Palette {}

/// Focus goes to the text input; the palette's actions bubble up from it.
impl Focusable for Palette {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.input().focus_handle(cx)
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

/// A prompt result that is already known.
pub fn ready(result: Result<(), String>) -> oneshot::Receiver<Result<(), String>> {
    let (tx, rx) = oneshot::channel();
    tx.send(result).ok();
    rx
}

impl Palette {
    pub fn list(rows: Vec<PaletteRow>, placeholder: impl Into<SharedString>, theme: Theme, cx: &mut Context<Self>) -> Self {
        let matches = (0..rows.len()).collect();
        let placeholder = placeholder.into();
        let query = cx.new(|cx| TextInput::new("", cx).with_placeholder(placeholder));
        let subscription = cx.subscribe(&query, |this, _, _: &TextInputEvent, cx| {
            if let Mode::List(list) = &mut this.mode {
                list.refilter(cx);
                this.scroll.scroll_to_item(0);
                cx.notify();
            }
        });
        let list = List { rows, query, matches, selected: 0 };
        Self {
            scroll: ScrollHandle::new(),
            mode: Mode::List(list),
            theme,
            dismiss_on_click_outside: true,
            _input_subscription: subscription,
        }
    }

    pub fn prompt(request: PromptRequest, theme: Theme, cx: &mut Context<Self>) -> Self {
        // The pre-filled value is selected, so typing replaces it.
        let input = cx.new(|cx| {
            let mut input = TextInput::new(&request.initial, cx);
            input.select_all(cx);
            input
        });
        let subscription = cx.subscribe(&input, |this, _, _: &TextInputEvent, cx| {
            if let Mode::Prompt(prompt) = &mut this.mode {
                prompt.error = None;
                cx.notify();
            }
        });
        let prompt = Prompt { label: request.label, input, error: None, submit: request.submit, running: None };
        Self {
            scroll: ScrollHandle::new(),
            mode: Mode::Prompt(prompt),
            theme,
            dismiss_on_click_outside: true,
            _input_subscription: subscription,
        }
    }

    /// Keeps the palette open on a click outside it; only escape or a
    /// choice closes it.
    pub fn modal(mut self) -> Self {
        self.dismiss_on_click_outside = false;
        self
    }

    /// The filter or prompt input.
    pub fn input(&self) -> &Entity<TextInput> {
        match &self.mode {
            Mode::List(list) => &list.query,
            Mode::Prompt(prompt) => &prompt.input,
        }
    }

    /// Debug description for `--dump-screen-after`.
    pub fn describe(&self, cx: &App) -> String {
        match &self.mode {
            Mode::List(list) => {
                let mut out = format!("--- palette list query={:?}\n", list.query.read(cx).text());
                for &ix in &list.matches {
                    let row = &list.rows[ix];
                    out.push_str(&format!("{}", row.label));
                    if let Some(detail) = &row.detail {
                        out.push_str(&format!("  {detail}"));
                    }
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
                p.input.read(cx).text(),
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
                let text = prompt.input.read(cx).text().to_string();
                let result = (prompt.submit)(text, cx);
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
                    .text_color(fg)
                    .child(list.query.clone()),
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
            .child(div().flex().text_color(t.foreground.hsla()).child(prompt.input.clone()))
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
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::cancel))
            .on_mouse_down_out(cx.listener(|this, _, _, cx| {
                if this.dismiss_on_click_outside {
                    cx.emit(PaletteEvent::Dismissed);
                }
            }))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            // Hide the terminal behind from the mouse, so its I-beam cursor
            // doesn't show over the palette.
            .occlude()
            .cursor(CursorStyle::Arrow)
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
}
