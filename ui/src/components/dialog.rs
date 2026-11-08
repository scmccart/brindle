//! `Dialog`: shadcn's dialog panel, the chrome of a modal overlay.
//!
//! A `RenderOnce` builder for the panel alone: an optional title and
//! description, a body (its children), a footer row of actions and, when
//! the owner gives an `on_close` handler, a close `X` in the top right
//! corner. The overlay around it comes from a modal [`Overlay`], which
//! supplies the dimmed backdrop, centres the panel, keeps Tab inside it,
//! turns Escape and a press on the backdrop into a [`Dismiss`] request, and
//! fades it in on each opening.
//!
//! The dialog's content is a view of the app's own, a `ManagedView` that
//! also implements [`FocusStops`]; its `render` returns a `Dialog`:
//!
//! ```ignore
//! impl Render for RenameDialog {
//!     fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
//!         div()
//!             // The view's own handle, which the overlay needs tracked.
//!             .track_focus(&self.focus)
//!             // Optional: answering `Dismiss` here keeps the dialog open,
//!             // since the layer's default (closing it) never sees it.
//!             .on_action(cx.listener(|this, _: &Dismiss, _, cx| this.confirm_discard(cx)))
//!             .child(
//!                 Dialog::new()
//!                     .title("Rename tab")
//!                     .description("The name shows in the tab strip.")
//!                     .close_focus(&self.close)
//!                     .on_close(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(DismissEvent)))
//!                     .child(Input::new(&self.name))
//!                     .footer(Button::new("cancel", "Cancel").variant(ButtonVariant::Outline).focus(&self.cancel))
//!                     .footer(Button::new("save", "Save").focus(&self.save)),
//!             )
//!     }
//! }
//!
//! impl FocusStops for RenameDialog {
//!     // Tab order: the body, the footer, then the close button, as in
//!     // shadcn's markup.
//!     fn stops(&self, cx: &App) -> Vec<FocusHandle> {
//!         vec![self.name.focus_handle(cx), self.cancel.clone(), self.save.clone(), self.close.clone()]
//!     }
//! }
//! ```
//!
//! The owner keeps an `Overlay::modal()` slot, opens it with a new view
//! (`self.rename.open(view, |owner| &mut owner.rename, window, cx)`) and
//! renders `self.rename.render(window, cx)` as the last child of its window
//! root, since nothing lifts a modal layer above what follows it in the
//! tree. On close, focus goes back to what had it when the dialog opened.
//!
//! The close button runs `on_close`, which decides what closing means:
//! usually emitting `DismissEvent`, or dispatching `Dismiss` to ask the
//! view first. It takes focus only through the handle given to
//! `close_focus`, which the view must list among its stops for Tab to
//! reach it.
//!
//! [`Overlay`]: crate::primitives::overlay::Overlay
//! [`Dismiss`]: crate::actions::Dismiss
//! [`FocusStops`]: crate::primitives::focus_scope::FocusStops

use gpui::{
    AnyElement, App, ClickEvent, FocusHandle, FontWeight, InteractiveElement, IntoElement, MouseButton,
    ParentElement, Pixels, Refineable, RenderOnce, SharedString, StyleRefinement, Styled,
    Window, div, prelude::FluentBuilder, px, relative, rems,
};

use crate::components::{ACTIVATION_KEYS, ClickHandler, FocusRing, activates};
use crate::icon::Icon;
use crate::primitives::focus_visible::{focus_from_pointer, modality, shows_ring};
use crate::style::{
    DIALOG_CLOSE_OPACITY, DIALOG_CLOSE_RADIUS, DIALOG_EDGE_GAP, DIALOG_MAX_WIDTH, dialog_style, focus_ring,
};
use crate::theme::ActiveTheme;

/// A dialog panel. Its children are the body; refine its layout through
/// `Styled` (a different width, say).
#[derive(Default, IntoElement)]
pub struct Dialog {
    title: Option<SharedString>,
    description: Option<SharedString>,
    body: Vec<AnyElement>,
    footer: Vec<AnyElement>,
    on_close: Option<ClickHandler>,
    close_focus: Option<FocusHandle>,
    style: StyleRefinement,
}

impl Dialog {
    pub fn new() -> Self {
        Self::default()
    }

    /// The heading, shadcn's DialogTitle.
    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// A line under the title, in the muted colour (DialogDescription).
    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Adds an element to the footer, a row of actions on the right, in
    /// the order given (DialogFooter).
    pub fn footer(mut self, element: impl IntoElement) -> Self {
        self.footer.push(element.into_any_element());
        self
    }

    /// Shows the close `X`, which runs `on_close` on a primary click, and
    /// on the enter or space key-down while it has focus.
    pub fn on_close(mut self, on_close: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_close = Some(Box::new(on_close));
        self
    }

    /// The handle the close `X` is focused through. Without one it can't
    /// take focus, so it is for the pointer only.
    pub fn close_focus(mut self, handle: &FocusHandle) -> Self {
        self.close_focus = Some(handle.clone());
        self
    }
}

impl ParentElement for Dialog {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.body.extend(elements);
    }
}

impl Styled for Dialog {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

/// A dialog's width in a window `viewport_width` wide: shadcn's `w-full
/// max-w-[calc(100%-2rem)] sm:max-w-lg`, so at most 32rem, and at least
/// 1rem from each side in a narrower window.
pub fn dialog_width(viewport_width: Pixels, rem_size: Pixels) -> Pixels {
    let max = DIALOG_MAX_WIDTH.to_pixels(rem_size);
    let room = viewport_width - DIALOG_EDGE_GAP.to_pixels(rem_size) * 2.;
    room.min(max).max(px(0.))
}

impl RenderOnce for Dialog {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.ui_theme();
        let style = dialog_style(t);
        let ring_color = focus_ring(t);
        let width = dialog_width(window.viewport_size().width, window.rem_size());

        // DialogHeader: flex flex-col gap-2 sm:text-left. The title is
        // text-lg leading-none font-semibold, the description
        // text-muted-foreground text-sm.
        let header = (self.title.is_some() || self.description.is_some()).then(|| {
            div()
                .flex()
                .flex_col()
                .gap_2()
                .children(self.title.map(|title| {
                    div()
                        .text_size(rems(1.125))
                        .line_height(relative(1.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(title)
                }))
                .children(
                    self.description
                        .map(|description| div().text_size(rems(0.875)).text_color(style.description).child(description)),
                )
        });
        // DialogFooter: sm:flex-row sm:justify-end gap-2.
        let footer = (!self.footer.is_empty())
            .then(|| div().flex().flex_row().justify_end().gap_2().children(self.footer));

        // The close button: absolute top-4 right-4 rounded-xs opacity-70
        // hover:opacity-100, a size-4 X. Last, as in shadcn's markup.
        let close = self.on_close.map(|on_close| {
            let focused = self.close_focus.as_ref().is_some_and(|handle| handle.is_focused(window));
            let ring_shown = shows_ring(focused, modality(window, cx), false);
            let close = div()
                .id("ui-dialog-close")
                .absolute()
                .top_4()
                .right_4()
                .flex()
                .items_center()
                .justify_center()
                .rounded(DIALOG_CLOSE_RADIUS)
                .opacity(DIALOG_CLOSE_OPACITY)
                .cursor_pointer()
                .hover(|s| s.opacity(1.))
                .focus_ring(ring_shown, ring_color, window)
                .when_some(self.close_focus, |d, handle| {
                    d.track_focus(&handle)
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| focus_from_pointer(&handle, window, cx))
                });
            activates(close, &ACTIVATION_KEYS, on_close).child(Icon::X.size(rems(1.)))
        });

        // DialogContent: grid gap-4 rounded-lg border p-6 shadow-lg, in the
        // background colour.
        let mut dialog = div()
            .relative()
            .flex()
            .flex_col()
            .gap_4()
            .w(width)
            .p_6()
            .rounded(style.radius)
            .border_1()
            .border_color(style.border)
            .bg(style.background)
            .text_color(style.foreground)
            .shadow_lg()
            .children(header)
            .children(self.body)
            .children(footer)
            .children(close);
        // The owner's layout refinements win over the component's.
        dialog.style().refine(&self.style);
        dialog
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// At most 32rem wide, and 1rem clear of each side in a narrow window.
    #[test]
    fn dialog_width_fits_the_window() {
        let rem = px(16.);
        // Room to spare: sm:max-w-lg.
        assert_eq!(dialog_width(px(1200.), rem), px(512.));
        // Exactly enough: 512 + 2 * 16.
        assert_eq!(dialog_width(px(544.), rem), px(512.));
        // Narrower: the window less 2rem.
        assert_eq!(dialog_width(px(400.), rem), px(368.));
        // Never negative.
        assert_eq!(dialog_width(px(20.), rem), px(0.));
        // It scales with the root text size.
        assert_eq!(dialog_width(px(2400.), px(32.)), px(1024.));
    }

    #[test]
    fn builders_fill_the_parts() {
        let dialog = Dialog::new().title("Rename").description("Pick a name");
        assert_eq!(dialog.title.as_ref().map(|s| s.as_ref()), Some("Rename"));
        assert_eq!(dialog.description.as_ref().map(|s| s.as_ref()), Some("Pick a name"));
        // No close handler, no X.
        assert!(dialog.on_close.is_none());
        let dialog = dialog.on_close(|_, _, _| {}).footer(div()).footer(div()).child(div());
        assert!(dialog.on_close.is_some());
        assert_eq!((dialog.footer.len(), dialog.body.len()), (2, 1));
    }
}
