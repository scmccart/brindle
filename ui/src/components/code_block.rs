//! `CodeBlock`: AI Elements' code block, a block of code in the owner's
//! monospace family with its label and a copy button above it.
//!
//! The frame is a rounded border. Its header sits on the surface around it
//! (a bubble's card, say): the label, when there is one, in the muted
//! foreground, and a ghost `copy` button at the end. Under the header the
//! code is drawn on the muted surface in the foreground colour and the
//! owner's family, keeping its line breaks and spaces; long lines wrap.
//! There is no highlighting.
//!
//! The copy button is drawn always (not on hover) and is pointer-only: it
//! takes no keyboard focus, and a press on it leaves focus where it was. It
//! gives the owner the code's text through `on_copy`; the component never
//! writes the clipboard. The owner gives the keyboard its own path (an
//! action that copies the last code block, say).

use std::rc::Rc;

use gpui::{
    App, ElementId, InteractiveElement, IntoElement, MouseButton, ParentElement, RenderOnce, SharedString, Styled,
    Window, div, prelude::FluentBuilder, rems,
};

use crate::components::Button;
use crate::components::response::Code;
use crate::components::tooltip::TooltipFor;
use crate::icon::Icon;
use crate::style::{ButtonSize, ButtonVariant};
use crate::theme::ActiveTheme;

/// What the owner is given when the copy button is pressed: the code.
pub(crate) type OnCopy = Rc<dyn Fn(SharedString, &mut Window, &mut App) + 'static>;

/// The copy button's tooltip.
pub const COPY_CODE: &str = "Copy code";

/// The text a code block shows: its text without the one line break that
/// ends its last line, which would otherwise draw an empty line. The text
/// copied is the whole text.
pub fn shown_code(text: &SharedString) -> SharedString {
    match text.strip_suffix('\n') {
        Some(shown) => SharedString::from(shown.to_string()),
        None => text.clone(),
    }
}

/// A code block.
#[derive(IntoElement)]
pub struct CodeBlock {
    id: ElementId,
    code: Code,
    font: Option<SharedString>,
    on_copy: Option<OnCopy>,
}

impl CodeBlock {
    /// `id` is unique among its siblings.
    pub fn new(id: impl Into<ElementId>, code: Code) -> Self {
        Self { id: id.into(), code, font: None, on_copy: None }
    }

    /// The family the code is drawn in: the owner's monospace one. Without
    /// it, the family around the block.
    pub fn font(mut self, family: impl Into<SharedString>) -> Self {
        self.font = Some(family.into());
        self
    }

    /// Called with the code's text on a press on the copy button.
    pub fn on_copy(mut self, on_copy: impl Fn(SharedString, &mut Window, &mut App) + 'static) -> Self {
        self.on_copy = Some(Rc::new(on_copy));
        self
    }

    /// The same, for a response passing its owner's handler on.
    pub(crate) fn on_copy_shared(mut self, on_copy: Option<OnCopy>) -> Self {
        self.on_copy = on_copy;
        self
    }
}

impl RenderOnce for CodeBlock {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.ui_theme();
        let Code { label, text } = self.code;
        let shown = shown_code(&text);
        let copy = Button::icon_only("copy", Icon::Copy)
            .variant(ButtonVariant::Ghost)
            .size(ButtonSize::IconXs)
            .tooltip_for(COPY_CODE, None)
            .when_some(self.on_copy, |button, on_copy| {
                button.on_click(move |_, window, cx| on_copy(text.clone(), window, cx))
            });
        // The button takes no focus, and nothing around it may either: a
        // press would focus the nearest ancestor that tracks focus.
        let copy = div()
            .debug_selector(|| "code-block-copy".into())
            .on_mouse_down(MouseButton::Left, |_, window, _| window.prevent_default())
            .child(copy);
        div()
            .id(self.id)
            .flex()
            .flex_col()
            .w_full()
            .min_w_0()
            .rounded(t.radius_md())
            .border_1()
            .border_color(t.border)
            .overflow_hidden()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .pl_3()
                    .pr_1()
                    .py_1()
                    .child(
                        div()
                            .min_w_0()
                            .text_size(rems(0.75))
                            .text_color(t.muted_foreground)
                            .children(label),
                    )
                    .child(copy),
            )
            .child(
                div()
                    .debug_selector(|| "code-block-code".into())
                    .px_3()
                    .py_2()
                    .bg(t.muted)
                    .text_color(t.foreground)
                    .when_some(self.font, |d, family| d.font_family(family))
                    .child(shown),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use gpui::{Context, FocusHandle, Modifiers, Render, px};

    use super::*;
    use crate::test_support;

    #[test]
    fn the_last_line_break_is_not_shown() {
        assert_eq!(shown_code(&"fn main() {}\n".into()).as_ref(), "fn main() {}");
        assert_eq!(shown_code(&"a\n\n".into()).as_ref(), "a\n");
        assert_eq!(shown_code(&"unclosed".into()).as_ref(), "unclosed");
        assert_eq!(shown_code(&"".into()).as_ref(), "");
    }

    struct Host {
        root: FocusHandle,
        elsewhere: FocusHandle,
        copied: Rc<RefCell<Vec<SharedString>>>,
    }

    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let copied = self.copied.clone();
            // The root tracks focus, as an app's view does: a press nothing
            // prevents would focus it.
            div()
                .track_focus(&self.root)
                .w(px(400.))
                .flex()
                .flex_col()
                .child(div().track_focus(&self.elsewhere).h(px(20.)))
                .child(
                    CodeBlock::new("code", Code { label: Some("rust".into()), text: "fn main() {}\n".into() })
                        .font("Mono")
                        .on_copy(move |text, _, _| copied.borrow_mut().push(text)),
                )
        }
    }

    /// The spec's "Copy button": a press reports the block's text, the
    /// last line break included, and focus stays where it was.
    #[test]
    fn a_press_on_copy_reports_the_text_and_keeps_focus() {
        let mut app = test_support::app();
        let copied = Rc::new(RefCell::new(Vec::new()));
        let held = copied.clone();
        let (host, cx) = app.add_window_view(move |_, cx| Host {
            root: cx.focus_handle(),
            elsewhere: cx.focus_handle(),
            copied: held,
        });
        let elsewhere = cx.update(|_, cx| host.read(cx).elsewhere.clone());
        cx.update(|window, _| window.focus(&elsewhere));
        cx.run_until_parked();
        let button = cx.debug_bounds("code-block-copy").expect("drawn");
        cx.simulate_click(button.center(), Modifiers::none());
        cx.run_until_parked();
        assert_eq!(copied.borrow().as_slice(), &[SharedString::from("fn main() {}\n")]);
        assert!(cx.update(|window, _| elsewhere.is_focused(window)), "the press moved focus");
        // The code sits under the header, inside the frame.
        let code = cx.debug_bounds("code-block-code").expect("drawn");
        assert!(code.top() >= button.bottom(), "{code:?} {button:?}");
    }
}
