//! The find bar: searching one terminal's scrollback from a small bar at its
//! top right. It lives inside its [`TerminalView`] (see the design of
//! `add-scrollback-search`): every terminal on screen, local tab or tmux
//! pane, gets its own, and focus moves between a terminal and its bar
//! without leaving the terminal's focus subtree.

use std::time::Duration;

use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::Point;
use alacritty_terminal::term::search::{Match, RegexSearch};
use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, FocusHandle, Hsla, InteractiveElement, IntoElement, KeyDownEvent, MouseButton,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, Task, Window, div, px, relative,
};

use crate::actions::*;
use crate::picker::TextField;
use crate::terminal::search::{self, COUNT_CAP, Count, SearchOptions, Step};
use crate::terminal_view::TerminalView;
use crate::theme::Theme;

/// Delay before counting after the query changes, so fast typing scans once.
const COUNT_AFTER_EDIT: Duration = Duration::from_millis(100);
/// Delay before recounting after new output while the bar is open.
const COUNT_AFTER_OUTPUT: Duration = Duration::from_millis(300);

pub struct FindBar {
    pub focus: FocusHandle,
    query: TextField,
    /// The query was restored when the bar opened and is shown selected:
    /// the next edit replaces it.
    selected: bool,
    options: SearchOptions,
    /// `None` for an empty query or an invalid regex.
    regex: Option<RegexSearch>,
    invalid: bool,
    focused: Option<Match>,
    /// Where incremental search starts: the bottom of the view when the bar
    /// opened, then the current match after stepping.
    origin: Point,
    /// The display offset to restore on close.
    saved_offset: usize,
    /// `None` until counted (or for an empty query).
    count: Option<Count>,
    recount: Option<Task<()>>,
}

/// What a closed bar leaves behind for the next one.
#[derive(Clone, Debug, Default)]
pub struct LastFind {
    query: String,
    options: SearchOptions,
}

/// The highlights a frame draws: matches in view and the current match.
pub struct Highlights {
    pub matches: Vec<Match>,
    pub focused: Option<Match>,
}

impl TerminalView {
    /// `Find`: opens the bar, or focuses it if it is open.
    pub fn find(&mut self, _: &Find, window: &mut Window, cx: &mut Context<Self>) {
        self.open_find_bar(window, cx);
        if let Some(bar) = &mut self.find {
            bar.selected = !bar.query.text.is_empty();
        }
        cx.notify();
    }

    fn open_find_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.find.is_none() {
            let (origin, saved_offset) = {
                let term = self.terminal.read(cx).term().lock();
                (search::view_bottom(&term), term.grid().display_offset())
            };
            let last = self.last_find.clone().unwrap_or_default();
            self.find = Some(FindBar {
                focus: cx.focus_handle(),
                query: TextField::new(&last.query),
                selected: false,
                options: last.options,
                regex: None,
                invalid: false,
                focused: None,
                origin,
                saved_offset,
                count: None,
                recount: None,
            });
            self.research(cx);
        }
        if let Some(bar) = &self.find {
            window.focus(&bar.focus);
        }
    }

    /// Debug hook for `--find`: opens the bar with `query`, as if typed.
    pub fn find_text(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.open_find_bar(window, cx);
        self.edit_query(|field| *field = TextField::new(query), cx);
    }

    /// Applies an edit to the query (replacing a restored query) and searches again.
    fn edit_query(&mut self, f: impl FnOnce(&mut TextField), cx: &mut Context<Self>) {
        let Some(bar) = &mut self.find else { return };
        if bar.selected {
            bar.query.text.clear();
            bar.selected = false;
        }
        f(&mut bar.query);
        self.research(cx);
    }

    /// Recompiles the query and finds the first match from the origin.
    fn research(&mut self, cx: &mut Context<Self>) {
        let Some(bar) = &mut self.find else { return };
        let compiled = search::compile(&bar.query.text, bar.options);
        bar.invalid = matches!(compiled, Some(Err(_)));
        bar.regex = compiled.and_then(Result::ok);
        bar.focused = None;
        bar.count = None;
        bar.recount = None;
        if let Some(regex) = &mut bar.regex {
            let mut term = self.terminal.read(cx).term().lock();
            bar.focused = search::find_from(&term, regex, bar.origin);
            if let Some(m) = &bar.focused {
                search::scroll_to(&mut term, m);
            }
            drop(term);
            self.schedule_count(COUNT_AFTER_EDIT, cx);
        }
        cx.notify();
    }

    fn step_match(&mut self, step: Step, cx: &mut Context<Self>) {
        let Some(bar) = &mut self.find else { return };
        let Some(regex) = &mut bar.regex else { return };
        let mut term = self.terminal.read(cx).term().lock();
        let next = match &bar.focused {
            Some(current) => search::step(&term, regex, current, step),
            None => search::find_from(&term, regex, search::view_bottom(&term)),
        };
        if let Some(m) = &next {
            bar.origin = *m.start();
            search::scroll_to(&mut term, m);
        }
        bar.focused = next;
        bar.count = Some(search::count(&term, regex, bar.focused.as_ref(), COUNT_CAP));
        bar.recount = None;
        drop(term);
        cx.notify();
    }

    fn schedule_count(&mut self, delay: Duration, cx: &mut Context<Self>) {
        let Some(bar) = &mut self.find else { return };
        bar.recount = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(delay).await;
            this.update(cx, |this, cx| this.recount(cx)).ok();
        }));
    }

    /// Counts the matches, and drops the current match if output has moved
    /// the text out from under it.
    fn recount(&mut self, cx: &mut Context<Self>) {
        let Some(bar) = &mut self.find else { return };
        bar.recount = None;
        let Some(regex) = &mut bar.regex else { return };
        let term = self.terminal.read(cx).term().lock();
        let count = search::count(&term, regex, bar.focused.as_ref(), COUNT_CAP);
        drop(term);
        if bar.focused.is_some() && count.index.is_none() && !count.capped {
            bar.focused = None;
        }
        bar.count = Some(count);
        cx.notify();
    }

    /// The terminal has new content: recount soon, once.
    pub fn find_terminal_changed(&mut self, cx: &mut Context<Self>) {
        if let Some(bar) = &self.find
            && bar.regex.is_some()
            && bar.recount.is_none()
        {
            self.schedule_count(COUNT_AFTER_OUTPUT, cx);
        }
    }

    pub fn find_next(&mut self, _: &FindNext, _: &mut Window, cx: &mut Context<Self>) {
        self.step_match(Step::Up, cx);
    }

    pub fn find_previous(&mut self, _: &FindPrevious, _: &mut Window, cx: &mut Context<Self>) {
        self.step_match(Step::Down, cx);
    }

    pub fn find_toggle_case(&mut self, _: &FindToggleCase, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_option(|o| o.case_sensitive = !o.case_sensitive, cx);
    }

    pub fn find_toggle_regex(&mut self, _: &FindToggleRegex, _: &mut Window, cx: &mut Context<Self>) {
        self.toggle_option(|o| o.regex = !o.regex, cx);
    }

    fn toggle_option(&mut self, f: impl FnOnce(&mut SearchOptions), cx: &mut Context<Self>) {
        let Some(bar) = &mut self.find else { return };
        f(&mut bar.options);
        self.research(cx);
    }

    pub fn find_close(&mut self, _: &FindClose, window: &mut Window, cx: &mut Context<Self>) {
        self.close_find_bar(window, cx);
    }

    /// Closes the bar, puts the view back where it was when the bar opened,
    /// and gives focus back to the terminal if the bar had it.
    fn close_find_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(bar) = self.find.take() else { return };
        self.last_find = Some(LastFind { query: bar.query.text.clone(), options: bar.options });
        {
            let mut term = self.terminal.read(cx).term().lock();
            let grid = term.grid();
            let history = grid.total_lines() - grid.screen_lines();
            let target = bar.saved_offset.min(history) as i32;
            let delta = target - grid.display_offset() as i32;
            if delta != 0 {
                term.scroll_display(Scroll::Delta(delta));
            }
        }
        if bar.focus.is_focused(window) {
            window.focus(&self.focus_handle);
        }
        cx.notify();
    }

    fn find_key_down(&mut self, e: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let ks = &e.keystroke;
        if !TextField::is_edit_key(ks) {
            return;
        }
        self.edit_query(|field| field.apply_key(ks), cx);
        cx.stop_propagation();
    }

    fn find_paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.edit_query(|field| field.paste(&text), cx);
        }
    }

    /// The matches to draw this frame. Called from prepaint; the work is
    /// bounded by the view (see `search::visible_matches`).
    pub fn find_highlights(&mut self, cx: &mut Context<Self>) -> Option<Highlights> {
        let bar = self.find.as_mut()?;
        let regex = bar.regex.as_mut()?;
        let term = self.terminal.read(cx).term().lock();
        let matches = search::visible_matches(&term, regex);
        Some(Highlights { matches, focused: bar.focused.clone() })
    }

    /// The bar's state for `--dump-screen-after`.
    pub fn describe_find(&self, cx: &gpui::App) -> Option<String> {
        let bar = self.find.as_ref()?;
        let count = match (&bar.regex, bar.count) {
            (None, _) => "none".to_string(),
            (Some(_), None) => "pending".to_string(),
            (Some(_), Some(c)) => format!("{}{}", c.total, if c.capped { "+" } else { "" }),
        };
        let index = bar.count.and_then(|c| c.index).map_or("-".to_string(), |i| i.to_string());
        let focused = match &bar.focused {
            None => "none".to_string(),
            Some(m) => {
                let term = self.terminal.read(cx).term().lock();
                match search::viewport_position(*m.start(), term.grid().display_offset(), term.screen_lines()) {
                    Some((row, col)) => format!("{row},{col}"),
                    None => "offscreen".to_string(),
                }
            }
        };
        Some(format!(
            "--- find query={:?} case={} regex={} count={count} index={index} focused={focused} invalid={}\n",
            bar.query.text, bar.options.case_sensitive, bar.options.regex, bar.invalid
        ))
    }

    /// The bar, drawn over the terminal's top-right corner.
    pub fn render_find_bar(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let bar = self.find.as_ref()?;
        let theme: Theme = self.terminal.read(cx).theme.clone();
        let t = &theme;
        let fg = t.foreground.hsla();
        let muted = t.muted_foreground().hsla();
        let accent = t.accent();
        let panel = t.chrome_background();
        let bar_focused = bar.focus.is_focused(window);

        let status: (SharedString, Hsla) = if bar.invalid {
            ("Invalid regex".into(), t.ansi[1].hsla())
        } else {
            match (&bar.regex, bar.count) {
                (None, _) => ("".into(), muted),
                (Some(_), None) => ("…".into(), muted),
                (Some(_), Some(c)) if c.total == 0 => ("No results".into(), muted),
                (Some(_), Some(c)) => {
                    let index = c.index.map_or("–".to_string(), |i| i.to_string());
                    (format!("{index}/{}{}", c.total, if c.capped { "+" } else { "" }).into(), muted)
                }
            }
        };

        let query = &bar.query.text;
        let query_text = div()
            .flex()
            .flex_1()
            .min_w_0()
            .overflow_hidden()
            .whitespace_nowrap()
            .items_center()
            .when(query.is_empty(), |d| d.text_color(muted).child("Find"))
            .when(!query.is_empty(), |d| {
                d.child(
                    div()
                        .text_color(fg)
                        .when(bar.selected, |d| d.bg(t.selection_background.hsla()))
                        .child(SharedString::from(query.clone())),
                )
            })
            .when(bar_focused, |d| d.child(div().flex_none().w(px(1.5)).h(px(15.0)).bg(accent.hsla())));

        let toggle = |id: &'static str, label: &'static str, on: bool| {
            div()
                .id(id)
                .flex_none()
                .px_1()
                .rounded_sm()
                .text_xs()
                .text_color(if on { fg } else { muted })
                .when(on, |d| d.bg(panel.mix(accent, 0.35).hsla()).border_1().border_color(accent.hsla()))
                .hover(|d| d.bg(panel.mix(t.foreground, 0.12).hsla()))
                .child(label)
        };
        let button = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .flex_none()
                .px_1()
                .rounded_sm()
                .text_color(muted)
                .hover(|d| d.bg(panel.mix(t.foreground, 0.12).hsla()).text_color(fg))
                .child(label)
        };

        Some(
            div()
                .id("find-bar")
                .key_context("FindBar")
                .track_focus(&bar.focus)
                .on_key_down(cx.listener(Self::find_key_down))
                .on_action(cx.listener(Self::find_paste))
                .occlude()
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| {
                        if let Some(bar) = &this.find {
                            window.focus(&bar.focus);
                        }
                        cx.stop_propagation();
                    }),
                )
                .absolute()
                .top(px(6.0))
                .right(px(10.0))
                .w(px(400.0))
                .max_w(relative(0.9))
                .flex()
                .items_center()
                .gap_1()
                .px_2()
                .py_1()
                .bg(panel.hsla())
                .border_1()
                .border_color(if bar_focused { accent.hsla() } else { t.chrome_border().hsla() })
                .rounded_md()
                .shadow_md()
                .text_sm()
                .cursor(gpui::CursorStyle::Arrow)
                .child(query_text)
                .child(div().flex_none().text_xs().text_color(status.1).child(status.0))
                .child(toggle("find-case", "Aa", bar.options.case_sensitive).on_click(
                    cx.listener(|this, _, w, cx| this.find_toggle_case(&FindToggleCase, w, cx)),
                ))
                .child(toggle("find-regex", ".*", bar.options.regex).on_click(
                    cx.listener(|this, _, w, cx| this.find_toggle_regex(&FindToggleRegex, w, cx)),
                ))
                .child(button("find-up", "↑").on_click(cx.listener(|this, _, _, cx| this.step_match(Step::Up, cx))))
                .child(button("find-down", "↓").on_click(cx.listener(|this, _, _, cx| this.step_match(Step::Down, cx))))
                .child(button("find-close", "×").on_click(cx.listener(|this, _, w, cx| this.close_find_bar(w, cx))))
                .into_any_element(),
        )
    }
}
