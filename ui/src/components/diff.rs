//! `Diff`: one file's change, as plain lines the owner gives: a header
//! with the operation's word, the path (and the path it came from, for a
//! move or copy) and a copy button, then the lines.
//!
//! Each line shows a sign column (none for context, `+` added, `−`
//! removed), its old and new line numbers and its text in the owner's
//! monospace family. Context lines are on `muted`, as a code block's text
//! is; added lines on `diff_added`, removed on `diff_removed`; the sign
//! means colour is never the only cue. A border line separates hunks. The
//! colours come from the active token set, so a diff follows an agent
//! zone's theme scope.
//!
//! It does no diffing: the owner gives the lines, numbered. A long diff is
//! drawn by the owner as several blocks, one per run of lines: only the
//! first carries the header ([`Diff::header`]), and [`Diff::joined`] says
//! which edges continue another block, so the frame reads as one. The copy
//! button is pointer-only, as [`CodeBlock`](crate::CodeBlock)'s: it takes
//! no focus and asks the owner to copy (`on_copy`); the block writes no
//! clipboard. Nothing in it takes focus.

use std::rc::Rc;

use gpui::{
    App, ElementId, Hsla, InteractiveElement, IntoElement, MouseButton, ParentElement, RenderOnce, SharedString,
    Styled, Window, div, prelude::FluentBuilder, rems,
};

use crate::components::tooltip::TooltipFor;
use crate::components::{Button, trigger_label};
use crate::icon::Icon;
use crate::style::{ButtonSize, ButtonVariant};
use crate::theme::{ActiveTheme, Tokens};

/// The copy button's tooltip.
pub const COPY_DIFF: &str = "Copy diff";

/// A diff line's kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffSign {
    Context,
    Added,
    Removed,
}

impl DiffSign {
    /// The sign column's character: the minus sign (U+2212) for a removed
    /// line, as a reader sees it, not the patch's hyphen.
    pub fn mark(self) -> &'static str {
        match self {
            DiffSign::Context => " ",
            DiffSign::Added => "+",
            DiffSign::Removed => "\u{2212}",
        }
    }
}

/// One line as drawn.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffRowData {
    pub sign: DiffSign,
    pub old: Option<u32>,
    pub new: Option<u32>,
    pub text: SharedString,
    /// The first line of a hunk after another: a border line above it.
    pub hunk_start: bool,
}

/// A line's background: `muted` for context, the diff tints otherwise.
pub fn diff_line_background(sign: DiffSign, t: &Tokens) -> Hsla {
    match sign {
        DiffSign::Context => t.muted,
        DiffSign::Added => t.diff_added,
        DiffSign::Removed => t.diff_removed,
    }
}

type OnCopy = Rc<dyn Fn(&mut Window, &mut App) + 'static>;

/// A diff block.
#[derive(IntoElement)]
pub struct Diff {
    id: ElementId,
    header: Option<(SharedString, SharedString, Option<SharedString>)>,
    rows: Vec<DiffRowData>,
    font: Option<SharedString>,
    joined: (bool, bool),
    on_copy: Option<OnCopy>,
}

impl Diff {
    /// `id` is unique among its siblings.
    pub fn new(id: impl Into<ElementId>, rows: impl IntoIterator<Item = DiffRowData>) -> Self {
        Self { id: id.into(), header: None, rows: rows.into_iter().collect(), font: None, joined: (false, false), on_copy: None }
    }

    /// The header: the operation's word (`Modified`), the path and, for a
    /// move or copy, the path it came from.
    pub fn header(
        mut self,
        op: impl Into<SharedString>,
        path: impl Into<SharedString>,
        from: Option<SharedString>,
    ) -> Self {
        self.header = Some((op.into(), path.into(), from));
        self
    }

    /// The family the lines are drawn in: the owner's monospace one.
    pub fn font(mut self, family: impl Into<SharedString>) -> Self {
        self.font = Some(family.into());
        self
    }

    /// Whether the block continues another above it and below it: a
    /// joined edge has no rounding and no border, so the blocks read as
    /// one frame.
    pub fn joined(mut self, above: bool, below: bool) -> Self {
        self.joined = (above, below);
        self
    }

    /// Called on a press on the header's copy button.
    pub fn on_copy(mut self, on_copy: impl Fn(&mut Window, &mut App) + 'static) -> Self {
        self.on_copy = Some(Rc::new(on_copy));
        self
    }
}

/// The text of a line number cell: the number, or nothing.
fn number(line: Option<u32>) -> SharedString {
    line.map_or_else(SharedString::default, |line| line.to_string().into())
}

impl RenderOnce for Diff {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.ui_theme();
        let (above, below) = self.joined;
        let radius = t.radius_md();
        let numbers = |text: SharedString| {
            div().flex_none().w(rems(2.5)).pr_2().text_right().text_color(t.muted_foreground).child(text)
        };
        let lines = self.rows.into_iter().map(|row| {
            div()
                .debug_selector(|| "diff-line".into())
                .flex()
                .w_full()
                .min_w_0()
                .bg(diff_line_background(row.sign, &t))
                .when(row.hunk_start, |d| d.border_t_1().border_color(t.border))
                .child(numbers(number(row.old)))
                .child(numbers(number(row.new)))
                .child(div().flex_none().w(rems(1.25)).text_color(t.foreground).child(row.sign.mark()))
                .child(div().flex_1().min_w_0().pr_3().text_color(t.foreground).whitespace_normal().child(row.text))
        });
        let header = self.header.map(|(op, path, from)| {
            let path = match from {
                Some(from) => SharedString::from(format!("{from} \u{2192} {path}")),
                None => path,
            };
            let copy = Button::icon_only("copy", Icon::Copy)
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::IconXs)
                .tooltip_for(COPY_DIFF, None)
                .when_some(self.on_copy, |button, on_copy| button.on_click(move |_, window, cx| on_copy(window, cx)));
            div()
                .flex()
                .items_center()
                .gap_2()
                .pl_3()
                .pr_1()
                .py_1()
                .text_size(rems(0.75))
                .child(div().debug_selector(|| "diff-op".into()).flex_none().text_color(t.muted_foreground).child(op))
                .child(trigger_label(path).debug_selector(|| "diff-path".into()).text_color(t.foreground))
                .child(
                    // Pointer-only: nothing around it takes focus either.
                    div()
                        .debug_selector(|| "diff-copy".into())
                        .flex_none()
                        .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
                        .child(copy),
                )
        });
        div()
            .id(self.id)
            .flex()
            .flex_col()
            .w_full()
            .min_w_0()
            .border_l_1()
            .border_r_1()
            .border_color(t.border)
            .when(!above, |d| d.border_t_1().rounded_t(radius))
            .when(!below, |d| d.border_b_1().rounded_b(radius))
            .overflow_hidden()
            .children(header)
            .child(
                div()
                    .debug_selector(|| "diff-lines".into())
                    .flex()
                    .flex_col()
                    .w_full()
                    .text_size(rems(0.875))
                    .when_some(self.font, |d, family| d.font_family(family))
                    .children(lines),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use gpui::{Context, Modifiers, Render, px};

    use super::*;
    use crate::primitives::ThemeScope;
    use crate::test_support;

    fn rows() -> Vec<DiffRowData> {
        let row = |sign, old, new, text: &str, hunk_start| DiffRowData { sign, old, new, text: text.to_string().into(), hunk_start };
        vec![
            row(DiffSign::Context, Some(10), Some(12), "fn main() {", false),
            row(DiffSign::Removed, Some(11), None, "    old();", false),
            row(DiffSign::Added, None, Some(13), "    new();", false),
            row(DiffSign::Added, None, Some(40), "more();", true),
        ]
    }

    #[test]
    fn signs_and_backgrounds_mark_each_line() {
        let t = Tokens::neutral_dark();
        assert_eq!([DiffSign::Context, DiffSign::Added, DiffSign::Removed].map(DiffSign::mark), [" ", "+", "\u{2212}"]);
        assert_eq!(diff_line_background(DiffSign::Context, &t), t.muted);
        assert_eq!(diff_line_background(DiffSign::Added, &t), t.diff_added);
        assert_eq!(diff_line_background(DiffSign::Removed, &t), t.diff_removed);
        assert_eq!(number(Some(10)).as_ref(), "10");
        assert_eq!(number(None).as_ref(), "");
    }

    struct Host {
        asked: Rc<Cell<u32>>,
    }

    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let asked = self.asked.clone();
            div().w(px(400.)).child(
                Diff::new("diff", rows())
                    .header("Modified", "/p/main.rs", None)
                    .on_copy(move |_, _| asked.set(asked.get() + 1)),
            )
        }
    }

    /// The spec's "Two hunks" and "Copy asked of the owner": the second
    /// hunk's first line has a border above it; the copy button asks the
    /// owner and focuses nothing.
    #[test]
    fn hunks_are_separated_and_copy_asks_the_owner() {
        let mut app = test_support::app();
        let asked = Rc::new(Cell::new(0));
        let shared = asked.clone();
        let (_, cx) = app.add_window_view(move |_, _| Host { asked: shared });
        cx.run_until_parked();
        let lines = cx.debug_bounds("diff-lines").expect("drawn");
        let path = cx.debug_bounds("diff-path").expect("drawn");
        assert!(path.bottom() <= lines.top(), "the header is above the lines");
        let copy = cx.debug_bounds("diff-copy").expect("drawn");
        cx.simulate_click(copy.center(), Modifiers::none());
        cx.run_until_parked();
        assert_eq!(asked.get(), 1);
        let focused = cx.update(|window, cx| window.focused(cx));
        assert!(focused.is_none(), "nothing took focus");
    }

    struct Scoped {
        seen: Rc<Cell<Option<Hsla>>>,
    }

    impl Render for Scoped {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let seen = self.seen.clone();
            let scoped = Tokens { diff_added: gpui::red(), ..Tokens::neutral_dark() };
            let probe = gpui::canvas(|_, _, _| {}, move |_, _, _, cx| seen.set(Some(cx.ui_theme().diff_added)));
            div().w(px(400.)).child(ThemeScope::new(
                scoped,
                div().child(Diff::new("diff", rows())).child(probe.h(px(1.)).w_full()),
            ))
        }
    }

    /// The lines take their colours from the active set, so inside a theme
    /// scope (an agent zone's) they are the scope's.
    #[test]
    fn the_colours_come_from_a_theme_scopes_set() {
        let mut app = test_support::app();
        let seen = Rc::new(Cell::new(None));
        let shared = seen.clone();
        let (_, cx) = app.add_window_view(move |_, _| Scoped { seen: shared });
        cx.run_until_parked();
        assert!(cx.debug_bounds("diff-line").is_some());
        assert_eq!(seen.get(), Some(gpui::red()), "the scope's set is the active one beside the diff");
        let outside = cx.update(|_, cx| cx.ui_theme().diff_added);
        assert_ne!(outside, gpui::red(), "and only inside it");
    }
}
