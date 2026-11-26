//! `Tool`: AI Elements' tool header, one row for a tool call.
//!
//! An icon the owner chooses (lucide `file-text` for a read, `terminal`
//! for a command, `wrench` for anything else), the title cut with an
//! ellipsis when the row is too narrow, an optional summary the owner gives
//! (`+12 −3`, `exit 1`) and a status badge, both always shown whole: the
//! badge reads pending, running, done, or failed in the destructive colour.
//!
//! AI Elements' header is a collapsible's trigger over the call's input
//! and output. Here the owner holds the content and whether it shows: with
//! an open state ([`Tool::open`]) the row ends in a chevron that turns
//! over while open, and a primary press on the row calls `on_toggle` with
//! the state asked for, as [`Reasoning`](crate::Reasoning)'s trigger does;
//! the owner draws the content below. Without one the row has no chevron
//! and a press does nothing. The row takes no focus: a conversation holds
//! many, and the owner gives the keyboard its own path (a toggle action).

use std::rc::Rc;

use gpui::{App, ElementId, Hsla, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div, rems};
use gpui::{FontWeight, InteractiveElement, StatefulInteractiveElement, prelude::FluentBuilder};

use crate::components::{accepts_click, trigger_label};
use crate::icon::{Icon, IconElement};
use crate::theme::{ActiveTheme, Tokens};

/// Where a tool call is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToolStatus {
    Pending,
    Running,
    Done,
    Failed,
}

impl ToolStatus {
    /// The badge's word.
    pub fn label(self) -> &'static str {
        match self {
            ToolStatus::Pending => "pending",
            ToolStatus::Running => "running",
            ToolStatus::Done => "done",
            ToolStatus::Failed => "failed",
        }
    }
}

/// The badge's text and border colours: the destructive colour for a
/// failed call, the foreground for a running one, muted otherwise.
pub fn tool_status_colors(status: ToolStatus, t: &Tokens) -> (Hsla, Hsla) {
    match status {
        ToolStatus::Failed => (t.destructive, t.destructive),
        ToolStatus::Running => (t.foreground, t.border),
        ToolStatus::Pending | ToolStatus::Done => (t.muted_foreground, t.border),
    }
}

type OnToggle = Rc<dyn Fn(bool, &mut Window, &mut App) + 'static>;

#[derive(IntoElement)]
pub struct Tool {
    id: Option<ElementId>,
    icon: Icon,
    title: SharedString,
    status: ToolStatus,
    summary: Option<SharedString>,
    open: Option<bool>,
    on_toggle: Option<OnToggle>,
}

impl Tool {
    pub fn new(icon: Icon, title: impl Into<SharedString>, status: ToolStatus) -> Self {
        Self { id: None, icon, title: title.into(), status, summary: None, open: None, on_toggle: None }
    }

    /// The row's id, unique among the rows of one list: a row that opens
    /// needs one to take presses.
    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Words on the line before the badge, in the muted colour, kept whole.
    pub fn summary(mut self, summary: impl Into<SharedString>) -> Self {
        self.summary = Some(summary.into());
        self
    }

    /// Whether the owner's content is open: `None`, the row doesn't open.
    pub fn open(mut self, open: Option<bool>) -> Self {
        self.open = open;
        self
    }

    /// Called on a primary press on a row that opens, with the state
    /// asked for: `true` to open.
    pub fn on_toggle(mut self, on_toggle: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_toggle = Some(Rc::new(on_toggle));
        self
    }
}

impl RenderOnce for Tool {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = &cx.ui_theme();
        let (text, border) = tool_status_colors(self.status, t);
        let (muted, foreground) = (t.muted_foreground, t.foreground);
        // flex w-full items-center gap-2 rounded-md border px-3 py-2
        // text-sm; the badge rounded-full border px-2 text-xs.
        let row = div()
            .flex()
            .items_center()
            .gap_2()
            .w_full()
            .min_w_0()
            .px_3()
            .py_2()
            .rounded(t.radius_md())
            .border_1()
            .border_color(t.border)
            .text_size(rems(0.875))
            .text_color(t.foreground)
            .child(self.icon.size(rems(1.)).color(t.muted_foreground))
            .child(trigger_label(self.title).debug_selector(|| "tool-title".into()))
            .when_some(self.summary, |d, summary| {
                d.child(
                    div()
                        .debug_selector(|| "tool-summary".into())
                        .flex_none()
                        .whitespace_nowrap()
                        .text_size(rems(0.75))
                        .text_color(muted)
                        .child(summary),
                )
            })
            .child(
                div()
                    .debug_selector(|| "tool-status".into())
                    .flex_none()
                    .px_2()
                    .py_0p5()
                    .rounded_full()
                    .border_1()
                    .border_color(border)
                    .text_size(rems(0.75))
                    .font_weight(FontWeight::MEDIUM)
                    .whitespace_nowrap()
                    .text_color(text)
                    .child(self.status.label()),
            );
        let Some(open) = self.open else { return row.into_any_element() };
        let chevron = div()
            .debug_selector(|| "tool-chevron".into())
            .flex_none()
            .text_color(muted)
            .child(IconElement::new(Icon::ChevronDown).size(rems(1.)).turn(if open { 0.5 } else { 0. }));
        let on_toggle = self.on_toggle;
        row.child(chevron)
            .id(self.id.unwrap_or_else(|| "tool".into()))
            .when(on_toggle.is_some(), |d| d.cursor_pointer().hover(move |s| s.text_color(foreground)))
            .when_some(on_toggle, |d, on_toggle| {
                d.on_click(move |event, window, cx| {
                    if accepts_click(event) {
                        on_toggle(!open, window, cx);
                    }
                })
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use gpui::{Context, Modifiers, Pixels, Render, px};

    use super::*;
    use crate::test_support;

    #[test]
    fn statuses_read_as_words_and_failed_is_destructive() {
        let t = Tokens::neutral_light();
        let all = [ToolStatus::Pending, ToolStatus::Running, ToolStatus::Done, ToolStatus::Failed];
        assert_eq!(all.map(ToolStatus::label), ["pending", "running", "done", "failed"]);
        assert_eq!(tool_status_colors(ToolStatus::Failed, &t), (t.destructive, t.destructive));
        assert_eq!(tool_status_colors(ToolStatus::Done, &t), (t.muted_foreground, t.border));
        assert_eq!(tool_status_colors(ToolStatus::Running, &t).0, t.foreground);
    }

    struct Host(Pixels);

    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().w(self.0).child(
                Tool::new(Icon::Terminal, "cargo test --workspace --locked -- --include-ignored", ToolStatus::Running)
                    .summary("+12 \u{2212}3")
                    .open(Some(false)),
            )
        }
    }

    /// The spec's "Long title": cut with an ellipsis, and the summary and
    /// the badge whole.
    #[test]
    fn a_long_title_is_cut_and_the_summary_and_badge_kept() {
        let measure = |width: f32| {
            let mut app = test_support::app();
            let (_, cx) = app.add_window_view(move |_, _| Host(px(width)));
            cx.run_until_parked();
            let mut bounds = |name| cx.debug_bounds(name).expect("drawn");
            (bounds("tool-title"), bounds("tool-summary"), bounds("tool-status"), bounds("tool-chevron"))
        };
        let (wide_title, wide_summary, wide_badge, _) = measure(1000.);
        let (title, summary, badge, chevron) = measure(260.);
        assert_eq!(badge.size, wide_badge.size, "the badge stays whole");
        assert_eq!(summary.size, wide_summary.size, "the summary stays whole");
        assert!(chevron.right() <= px(260. - 12.), "{chevron:?}");
        assert!(title.size.width < wide_title.size.width, "{title:?} {wide_title:?}");
        assert!(title.right() <= summary.left() && summary.right() <= badge.left() && badge.right() <= chevron.left());
    }

    struct Toggled {
        open: Option<bool>,
        handler: bool,
        asked: Rc<Cell<Option<bool>>>,
    }

    impl Render for Toggled {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let asked = self.asked.clone();
            let tool = Tool::new(Icon::Pencil, "Edit main.rs", ToolStatus::Done).id("row").open(self.open);
            let tool = if self.handler { tool.on_toggle(move |open, _, _| asked.set(Some(open))) } else { tool };
            div().w(px(300.)).child(tool)
        }
    }

    fn pressed(open: Option<bool>, handler: bool) -> (Option<bool>, bool) {
        let mut app = test_support::app();
        let asked = Rc::new(Cell::new(None));
        let shared = asked.clone();
        let (_, cx) = app.add_window_view(move |_, _| Toggled { open, handler, asked: shared });
        cx.run_until_parked();
        let chevron = cx.debug_bounds("tool-chevron").is_some();
        let title = cx.debug_bounds("tool-title").expect("drawn");
        cx.simulate_click(title.center(), Modifiers::none());
        cx.run_until_parked();
        (asked.get(), chevron)
    }

    /// The spec's "Asked to open" and "A row that doesn't open": a press on
    /// a row that opens asks the owner for the other state; a row with no
    /// open state shows no chevron and asks nothing.
    #[test]
    fn a_press_asks_the_owner_to_flip_a_row_that_opens() {
        assert_eq!(pressed(Some(false), true), (Some(true), true));
        assert_eq!(pressed(Some(true), true), (Some(false), true));
        assert_eq!(pressed(None, true), (None, false), "no open state: no chevron, no press");
        assert_eq!(pressed(Some(false), false), (None, true), "no handler: nothing asked");
    }
}
