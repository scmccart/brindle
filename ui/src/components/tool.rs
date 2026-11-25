//! `Tool`: AI Elements' tool header, one row for a tool call.
//!
//! An icon the owner chooses (lucide `file-text` for a read, `terminal`
//! for a command, `wrench` for anything else), the title cut with an
//! ellipsis when the row is too narrow, and a status badge that always
//! shows whole: pending, running, done, or failed in the destructive
//! colour. AI Elements' header is a collapsible's trigger over the call's
//! input and output; this row has no content, and takes no focus.

use gpui::{App, Hsla, IntoElement, ParentElement, RenderOnce, SharedString, Styled, Window, div, rems};
use gpui::{FontWeight, InteractiveElement};

use crate::components::trigger_label;
use crate::icon::Icon;
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

#[derive(IntoElement)]
pub struct Tool {
    icon: Icon,
    title: SharedString,
    status: ToolStatus,
}

impl Tool {
    pub fn new(icon: Icon, title: impl Into<SharedString>, status: ToolStatus) -> Self {
        Self { icon, title: title.into(), status }
    }
}

impl RenderOnce for Tool {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.ui_theme();
        let (text, border) = tool_status_colors(self.status, t);
        // flex w-full items-center gap-2 rounded-md border px-3 py-2
        // text-sm; the badge rounded-full border px-2 text-xs.
        div()
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
            )
    }
}

#[cfg(test)]
mod tests {
    use gpui::{Context, Pixels, Render, px};

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
            div().w(self.0).child(Tool::new(
                Icon::Terminal,
                "cargo test --workspace --locked -- --include-ignored",
                ToolStatus::Running,
            ))
        }
    }

    /// The spec's "Long title": cut with an ellipsis, and the badge whole.
    #[test]
    fn a_long_title_is_cut_and_the_badge_kept() {
        let measure = |width: f32| {
            let mut app = test_support::app();
            let (_, cx) = app.add_window_view(move |_, _| Host(px(width)));
            cx.run_until_parked();
            (cx.debug_bounds("tool-title").expect("drawn"), cx.debug_bounds("tool-status").expect("drawn"))
        };
        let (wide_title, wide_badge) = measure(1000.);
        let (title, badge) = measure(200.);
        assert_eq!(badge.size, wide_badge.size, "the badge stays whole");
        assert!(badge.right() <= px(200. - 12.), "{badge:?}");
        assert!(title.size.width < wide_title.size.width, "{title:?} {wide_title:?}");
        assert!(title.right() <= badge.left());
    }
}
