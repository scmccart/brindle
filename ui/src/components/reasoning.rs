//! `Reasoning`: AI Elements' reasoning block, a trigger over collapsible
//! content (an agent's thoughts).
//!
//! The owner holds whether it is open and passes it in on each render; a
//! primary press on the trigger calls `on_toggle` with the state asked for,
//! and the owner flips its own. The trigger is lucide `brain`, the owner's
//! label and a chevron that turns over while open, in the muted colour
//! (the foreground under the pointer). While closed only the trigger
//! shows; open, the children follow it in the muted colour.
//!
//! The trigger takes no focus (AI Elements' is a button): a conversation
//! holds many of them, and Tab moving through each would bury the prompt.
//! The owner gives the keyboard its own path (a toggle action).

use std::rc::Rc;

use gpui::{
    AnyElement, App, ElementId, InteractiveElement, IntoElement, ParentElement, RenderOnce, SharedString,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, rems,
};

use crate::components::accepts_click;
use crate::icon::{Icon, IconElement};
use crate::theme::ActiveTheme;

type OnToggle = Rc<dyn Fn(bool, &mut Window, &mut App) + 'static>;

/// A reasoning block. Its children are the content.
#[derive(IntoElement)]
pub struct Reasoning {
    id: ElementId,
    open: bool,
    label: SharedString,
    on_toggle: Option<OnToggle>,
    children: Vec<AnyElement>,
}

impl Reasoning {
    /// `id` is unique among the blocks of one list; `open` is the owner's.
    pub fn new(id: impl Into<ElementId>, open: bool) -> Self {
        Self { id: id.into(), open, label: "Reasoning".into(), on_toggle: None, children: Vec::new() }
    }

    /// The trigger's text ("Thinking…", "Thought for 4s").
    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = label.into();
        self
    }

    /// Called on a press on the trigger with the state asked for: `true`
    /// to open.
    pub fn on_toggle(mut self, on_toggle: impl Fn(bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_toggle = Some(Rc::new(on_toggle));
        self
    }
}

impl ParentElement for Reasoning {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Reasoning {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.ui_theme();
        let (muted, foreground) = (t.muted_foreground, t.foreground);
        let open = self.open;
        let on_toggle = self.on_toggle;
        // flex items-center gap-2 text-sm text-muted-foreground
        // hover:text-foreground; the chevron rotate-180 while open.
        let trigger = div()
            .id(self.id.clone())
            .debug_selector(|| "reasoning-trigger".into())
            .flex()
            .items_center()
            .gap_2()
            .text_size(rems(0.875))
            .text_color(muted)
            .hover(move |s| s.text_color(foreground))
            .when(on_toggle.is_some(), |d| d.cursor_pointer())
            .child(Icon::Brain.size(rems(1.)))
            .child(div().min_w_0().truncate().child(self.label))
            .child(IconElement::new(Icon::ChevronDown).size(rems(1.)).turn(if open { 0.5 } else { 0. }))
            .when_some(on_toggle, |d, on_toggle| {
                d.on_click(move |event, window, cx| {
                    if accepts_click(event) {
                        on_toggle(!open, window, cx);
                    }
                })
            });
        div().flex().flex_col().w_full().min_w_0().child(trigger).when(open, |d| {
            d.child(
                div()
                    .debug_selector(|| "reasoning-content".into())
                    .mt_2()
                    .text_size(rems(0.875))
                    .text_color(muted)
                    .children(self.children),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use gpui::{Context, Modifiers, Render, px};

    use super::*;
    use crate::test_support;

    struct Host {
        open: bool,
        asked: Rc<Cell<Option<bool>>>,
        content_drawn: Rc<Cell<bool>>,
    }

    impl Render for Host {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let asked = self.asked.clone();
            let drawn = self.content_drawn.clone();
            let host = cx.weak_entity();
            div().w(px(300.)).child(
                Reasoning::new("thought", self.open)
                    .label("Thought for 4s")
                    .on_toggle(move |open, _, cx| {
                        asked.set(Some(open));
                        host.update(cx, |host, cx| {
                            host.open = open;
                            cx.notify();
                        })
                        .ok();
                    })
                    .child(gpui::canvas(|_, _, _| {}, move |_, _, _, _| drawn.set(true)).h(px(10.)).w_full()),
            )
        }
    }

    /// The spec's "Open on press": a press asks the owner to open it, and
    /// once open the content shows below the trigger; another closes it.
    #[test]
    fn a_press_asks_to_flip_it() {
        let mut app = test_support::app();
        let asked = Rc::new(Cell::new(None));
        let drawn = Rc::new(Cell::new(false));
        let (asked2, drawn2) = (asked.clone(), drawn.clone());
        let (_, cx) =
            app.add_window_view(move |_, _| Host { open: false, asked: asked2, content_drawn: drawn2 });
        cx.run_until_parked();
        assert!(!drawn.get(), "closed, only the trigger shows");
        let trigger = cx.debug_bounds("reasoning-trigger").expect("drawn");
        cx.simulate_click(trigger.center(), Modifiers::none());
        cx.run_until_parked();
        assert_eq!(asked.get(), Some(true));
        assert!(drawn.get());
        let content = cx.debug_bounds("reasoning-content").expect("open");
        assert!(content.top() >= trigger.bottom());
        cx.simulate_click(trigger.center(), Modifiers::none());
        cx.run_until_parked();
        assert_eq!(asked.get(), Some(false));
        // The next frame draws the trigger alone (the press's own frame
        // came before the owner closed it).
        drawn.set(false);
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();
        assert!(!drawn.get());
    }
}
