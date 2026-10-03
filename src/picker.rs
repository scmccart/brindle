//! The profile picker: a small filterable list shown over the terminal.

use gpui::{
    App, Context, EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement,
    KeyDownEvent, MouseButton, ParentElement, Render, SharedString, StatefulInteractiveElement,
    Styled, Window, div, px,
};

use crate::actions::{PickerCancel, PickerConfirm, PickerDown, PickerUp};
use crate::config::TmuxMode;
use crate::settings::Settings;
use crate::theme::Theme;

pub enum PickerEvent {
    Confirmed(usize),
    Dismissed,
}

pub struct ProfilePicker {
    focus_handle: FocusHandle,
    query: String,
    selected: usize,
    theme: Theme,
}

impl EventEmitter<PickerEvent> for ProfilePicker {}

impl Focusable for ProfilePicker {
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

impl ProfilePicker {
    pub fn new(theme: Theme, cx: &mut Context<Self>) -> Self {
        Self { focus_handle: cx.focus_handle(), query: String::new(), selected: 0, theme }
    }

    /// Profile indices matching the query, in config order.
    fn matches(&self, cx: &App) -> Vec<usize> {
        Settings::get(cx)
            .config
            .profiles
            .iter()
            .enumerate()
            .filter(|(_, p)| fuzzy_match(&self.query, &p.name))
            .map(|(ix, _)| ix)
            .collect()
    }

    fn up(&mut self, _: &PickerUp, _: &mut Window, cx: &mut Context<Self>) {
        let count = self.matches(cx).len();
        if count > 0 {
            self.selected = (self.selected + count - 1) % count;
            cx.notify();
        }
    }

    fn down(&mut self, _: &PickerDown, _: &mut Window, cx: &mut Context<Self>) {
        let count = self.matches(cx).len();
        if count > 0 {
            self.selected = (self.selected + 1) % count;
            cx.notify();
        }
    }

    fn confirm(&mut self, _: &PickerConfirm, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(&ix) = self.matches(cx).get(self.selected) {
            cx.emit(PickerEvent::Confirmed(ix));
        }
    }

    fn cancel(&mut self, _: &PickerCancel, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(PickerEvent::Dismissed);
    }

    fn key_down(&mut self, e: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let ks = &e.keystroke;
        if ks.modifiers.control || ks.modifiers.alt || ks.modifiers.platform {
            return;
        }
        if ks.key == "backspace" {
            self.query.pop();
        } else if let Some(ch) = ks.key_char.as_deref() {
            self.query.push_str(ch);
        } else {
            return;
        }
        self.selected = 0;
        cx.stop_propagation();
        cx.notify();
    }
}

impl Render for ProfilePicker {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let matches = self.matches(cx);
        let profiles = &Settings::get(cx).config.profiles;
        let selected = self.selected.min(matches.len().saturating_sub(1));
        let fg = t.foreground.hsla();
        let muted = t.muted_foreground().hsla();
        let panel = t.chrome_background();
        let accent = t.accent();

        let rows = matches.iter().enumerate().map(|(row, &ix)| {
            let profile = &profiles[ix];
            let detail: SharedString = match profile.tmux {
                TmuxMode::Control => format!(
                    "tmux control mode · {}",
                    profile.tmux_session.as_deref().unwrap_or("main")
                )
                .into(),
                TmuxMode::Plain => {
                    format!("tmux · {}", profile.tmux_session.as_deref().unwrap_or("main")).into()
                }
                TmuxMode::None => profile
                    .command
                    .clone()
                    .unwrap_or_else(|| "default shell".into())
                    .into(),
            };
            let shortcut: SharedString =
                if ix < 9 { format!("ctrl-alt-{}", ix + 1).into() } else { "".into() };
            let is_selected = row == selected;
            div()
                .id(("profile", ix))
                .flex()
                .items_center()
                .justify_between()
                .gap_4()
                .px_3()
                .py_1p5()
                .rounded_md()
                .when(is_selected, |d| d.bg(panel.mix(accent, 0.22).hsla()))
                .hover(|d| d.bg(panel.mix(t.foreground, 0.08).hsla()))
                .on_click(cx.listener(move |_, _, _, cx| cx.emit(PickerEvent::Confirmed(ix))))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(div().text_color(fg).child(SharedString::from(profile.name.clone())))
                        .child(div().text_xs().text_color(muted).child(detail)),
                )
                .child(div().text_xs().text_color(muted).child(shortcut))
        });

        let query: SharedString = if self.query.is_empty() {
            "Open profile…".into()
        } else {
            self.query.clone().into()
        };

        div()
            .key_context("ProfilePicker")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::cancel))
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(PickerEvent::Dismissed)))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .w(px(420.0))
            .flex()
            .flex_col()
            .p_2()
            .gap_1()
            .bg(panel.hsla())
            .border_1()
            .border_color(t.chrome_border().hsla())
            .rounded_lg()
            .shadow_lg()
            .text_sm()
            .child(
                div()
                    .px_3()
                    .py_2()
                    .mb_1()
                    .border_b_1()
                    .border_color(t.chrome_border().hsla())
                    .text_color(if self.query.is_empty() { muted } else { fg })
                    .child(query),
            )
            .children(rows)
            .when(matches.is_empty(), |d| {
                d.child(div().px_3().py_2().text_color(muted).child("No matching profiles"))
            })
    }
}

use gpui::prelude::FluentBuilder as _;

#[cfg(test)]
mod tests {
    use super::fuzzy_match;

    #[test]
    fn fuzzy() {
        assert!(fuzzy_match("", "Shell"));
        assert!(fuzzy_match("tm", "tmux (classic)"));
        assert!(fuzzy_match("TC", "tmux (classic)"));
        assert!(fuzzy_match("tmux cl", "tmux (classic)"));
        assert!(!fuzzy_match("zsh", "Shell"));
    }
}
