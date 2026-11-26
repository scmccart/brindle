//! `Confirmation`: AI Elements' confirmation, an inline row asking the
//! person to approve something (a tool call's permission).
//!
//! A title and, while [`ConfirmationState::Requested`], one small button
//! per action the owner gives, in its order, each a keyboard stop through
//! the focus handle the owner passes (enter, space or a primary click
//! activates it) and asking the owner for its action by key
//! (`on_action`). Once the owner marks it answered it shows the outcome:
//! lucide `check` and the chosen action's label when accepted, `x` and the
//! label when rejected, `x` and "Cancelled" when it was withdrawn; and no
//! buttons.
//!
//! The owner keys each action ([`ItemKey`]), so a press names the action
//! the person saw even when the list is rebuilt.
//!
//! An action may carry a description (what it does, in plain text) and be
//! disabled (drawn muted, no stop, asking nothing). When any action has a
//! description, the actions stack, each with its description under it,
//! instead of sitting in one row.

use std::rc::Rc;

use gpui::{
    AnyElement, App, ElementId, FocusHandle, InteractiveElement, IntoElement, ParentElement, RenderOnce, SharedString,
    Styled, Window, div, prelude::FluentBuilder as _, rems,
};

use crate::components::{Button, ItemKey};
use crate::icon::Icon;
use crate::style::{ButtonSize, ButtonVariant};
use crate::theme::ActiveTheme;

/// Where a confirmation is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConfirmationState {
    /// Waiting for an answer: the buttons show.
    Requested,
    /// Approved with the action labelled `label`.
    Accepted { label: SharedString },
    /// Refused with the action labelled `label`.
    Rejected { label: SharedString },
    /// Withdrawn before an answer.
    Cancelled,
}

impl ConfirmationState {
    /// What an answered confirmation shows: an icon and a label. `None`
    /// while requested.
    pub fn outcome(&self) -> Option<(Icon, SharedString)> {
        match self {
            ConfirmationState::Requested => None,
            ConfirmationState::Accepted { label } => Some((Icon::Check, label.clone())),
            ConfirmationState::Rejected { label } => Some((Icon::X, label.clone())),
            ConfirmationState::Cancelled => Some((Icon::X, "Cancelled".into())),
        }
    }
}

/// One of a confirmation's actions.
pub struct ConfirmationAction {
    pub key: ItemKey,
    pub label: SharedString,
    pub variant: ButtonVariant,
    pub focus: Option<FocusHandle>,
    pub description: Option<SharedString>,
    pub disabled: bool,
}

impl ConfirmationAction {
    /// An outline button keyed by its label.
    pub fn new(label: impl Into<SharedString>) -> Self {
        let label = label.into();
        Self {
            key: ItemKey::new(label.clone()),
            label,
            variant: ButtonVariant::Outline,
            focus: None,
            description: None,
            disabled: false,
        }
    }

    /// Plain text shown under the button, saying what it does.
    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// A disabled action is drawn muted, is no stop and asks nothing.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn key(mut self, key: impl Into<ItemKey>) -> Self {
        self.key = key.into();
        self
    }

    /// The button's variant (`Default` for the action to take first).
    pub fn variant(mut self, variant: ButtonVariant) -> Self {
        self.variant = variant;
        self
    }

    /// The handle that makes the button a keyboard stop.
    pub fn focus(mut self, handle: &FocusHandle) -> Self {
        self.focus = Some(handle.clone());
        self
    }
}

type OnAction = Rc<dyn Fn(&ItemKey, &mut Window, &mut App) + 'static>;

/// Whether the actions stack, each with its description under it: when
/// any has one. Otherwise they sit in one row at the end.
pub fn actions_stack(actions: &[ConfirmationAction]) -> bool {
    actions.iter().any(|action| action.description.is_some())
}

#[derive(IntoElement)]
pub struct Confirmation {
    id: ElementId,
    title: SharedString,
    state: ConfirmationState,
    actions: Vec<ConfirmationAction>,
    on_action: Option<OnAction>,
    children: Vec<AnyElement>,
}

impl Confirmation {
    /// `id` is unique among the confirmations of one list.
    pub fn new(id: impl Into<ElementId>, title: impl Into<SharedString>, state: ConfirmationState) -> Self {
        Self { id: id.into(), title: title.into(), state, actions: Vec::new(), on_action: None, children: Vec::new() }
    }

    pub fn action(mut self, action: ConfirmationAction) -> Self {
        self.actions.push(action);
        self
    }

    /// Called with the pressed action's key.
    pub fn on_action(mut self, on_action: impl Fn(&ItemKey, &mut Window, &mut App) + 'static) -> Self {
        self.on_action = Some(Rc::new(on_action));
        self
    }
}

/// Children go under the title (what is asked about, in detail).
impl ParentElement for Confirmation {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.children.extend(elements);
    }
}

impl RenderOnce for Confirmation {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        let t = cx.ui_theme();
        let stacked = actions_stack(&self.actions);
        let muted = t.muted_foreground;
        let footer: AnyElement = match self.state.outcome() {
            None => div()
                .debug_selector(|| "confirmation-actions".into())
                .flex()
                .gap_2()
                .when(stacked, |d| d.flex_col().items_start())
                .when(!stacked, |d| d.flex_wrap().justify_end())
                .children(self.actions.into_iter().enumerate().map(|(ix, action)| {
                    let id = ElementId::NamedInteger("confirmation-action".into(), ix as u64);
                    let mut button = Button::new(id, action.label)
                        .variant(action.variant)
                        .size(ButtonSize::Sm)
                        .disabled(action.disabled);
                    if let Some(handle) = action.focus.as_ref().filter(|_| !action.disabled) {
                        button = button.focus(handle);
                    }
                    if let Some(on_action) = self.on_action.clone().filter(|_| !action.disabled) {
                        let key = action.key;
                        button = button.on_click(move |_, window, cx| on_action(&key, window, cx));
                    }
                    // Stacked, each action is as wide as the confirmation
                    // (a definite width), its button at the button's own
                    // width and its description wrapping at the action's
                    // (the wrapping text pitfall: never at a measure of its
                    // own).
                    div()
                        .debug_selector(move || format!("confirmation-action-{ix}"))
                        .flex()
                        .flex_col()
                        .gap_1()
                        .min_w_0()
                        .when(stacked, |d| d.w_full().items_start())
                        .child(div().debug_selector(move || format!("confirmation-button-{ix}")).child(button))
                        .when_some(action.description, |d, description| {
                            d.child(
                                div()
                                    .debug_selector(move || format!("confirmation-description-{ix}"))
                                    .w_full()
                                    .text_size(rems(0.75))
                                    .text_color(muted)
                                    .whitespace_normal()
                                    .child(description),
                            )
                        })
                }))
                .into_any_element(),
            Some((icon, label)) => div()
                .debug_selector(|| "confirmation-outcome".into())
                .flex()
                .items_center()
                .gap_2()
                .text_color(t.muted_foreground)
                .child(icon.size(rems(1.)))
                .child(label)
                .into_any_element(),
        };
        // An alert's frame: rounded-lg border bg-card px-4 py-3 text-sm.
        // Its id scopes the buttons'.
        div()
            .id(self.id)
            .flex()
            .flex_col()
            .gap_2()
            .w_full()
            .min_w_0()
            .px_4()
            .py_3()
            .rounded(t.radius_lg())
            .border_1()
            .border_color(t.border)
            .bg(t.card)
            .text_color(t.card_foreground)
            .text_size(rems(0.875))
            .child(div().font_weight(gpui::FontWeight::MEDIUM).child(self.title))
            .children(self.children)
            .child(footer)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use gpui::{Context, Focusable, Render, px};

    use super::*;
    use crate::test_support;

    #[test]
    fn answered_shows_the_outcome() {
        assert_eq!(ConfirmationState::Requested.outcome(), None);
        assert_eq!(
            ConfirmationState::Accepted { label: "Allow once".into() }.outcome(),
            Some((Icon::Check, "Allow once".into()))
        );
        assert_eq!(ConfirmationState::Rejected { label: "Reject".into() }.outcome(), Some((Icon::X, "Reject".into())));
        assert_eq!(ConfirmationState::Cancelled.outcome(), Some((Icon::X, "Cancelled".into())));
    }

    #[test]
    fn a_description_stacks_the_actions() {
        assert!(!actions_stack(&[ConfirmationAction::new("Allow"), ConfirmationAction::new("Reject")]));
        assert!(actions_stack(&[ConfirmationAction::new("A"), ConfirmationAction::new("B").description("Use a key")]));
    }

    struct Described {
        first: FocusHandle,
        asked: Rc<RefCell<Vec<ItemKey>>>,
    }

    impl Focusable for Described {
        fn focus_handle(&self, _: &App) -> FocusHandle {
            self.first.clone()
        }
    }

    impl Render for Described {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let asked = self.asked.clone();
            div().w(px(400.)).child(
                Confirmation::new("login", "Log in to continue", ConfirmationState::Requested)
                    .action(ConfirmationAction::new("Log in with Google").key("google").focus(&self.first))
                    .action(ConfirmationAction::new("API Key").key("key").description("Use an API key"))
                    .action(ConfirmationAction::new("Terminal").key("terminal").description("unavailable").disabled(true))
                    .on_action(move |key, _, _| asked.borrow_mut().push(key.clone())),
            )
        }
    }

    /// The spec's "Action descriptions": each description sits under its
    /// button, the actions stack in the owner's order, and a disabled one
    /// asks nothing when clicked.
    #[test]
    fn described_actions_stack_and_a_disabled_one_asks_nothing() {
        let mut app = test_support::app();
        let asked = Rc::new(RefCell::new(Vec::new()));
        let asked2 = asked.clone();
        let (_, cx) = app.add_window_view(move |_, cx| Described { first: cx.focus_handle(), asked: asked2 });
        cx.run_until_parked();
        let first = cx.debug_bounds("confirmation-action-0").expect("the first action");
        let second = cx.debug_bounds("confirmation-action-1").expect("the second action");
        let third = cx.debug_bounds("confirmation-action-2").expect("the third action");
        let description = cx.debug_bounds("confirmation-description-1").expect("its description");
        assert!(first.bottom() <= second.top() && second.bottom() <= third.top(), "stacked in order");
        assert!(description.top() >= second.top() && description.bottom() <= second.bottom(), "under its button");
        cx.simulate_click(second.origin + gpui::point(px(4.), px(4.)), gpui::Modifiers::none());
        assert_eq!(*asked.borrow(), [ItemKey::from("key")]);
        cx.simulate_click(third.origin + gpui::point(px(4.), px(4.)), gpui::Modifiers::none());
        assert_eq!(*asked.borrow(), [ItemKey::from("key")], "a disabled action asks nothing");
    }

    /// Stacked actions in a 240px confirmation, the second with a
    /// description much longer than that.
    struct Narrow;

    const LONG: &str = "the login can't be run through `cmd /c`, so run it in a terminal of your own and come back";

    impl Render for Narrow {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .flex()
                .flex_col()
                .child(div().debug_selector(|| "one-line".into()).text_size(rems(0.75)).child("x"))
                .child(
                    div().w(px(240.)).child(
                        Confirmation::new("login", "Log in to continue", ConfirmationState::Requested)
                            .action(ConfirmationAction::new("Log in with Google").key("google"))
                            .action(ConfirmationAction::new("Terminal").key("terminal").description(LONG)),
                    ),
                )
        }
    }

    /// A long description wraps at the confirmation's width (inside its
    /// border and px-4), over several lines, and each button keeps its own
    /// width.
    #[test]
    fn a_long_description_wraps_at_the_confirmations_width() {
        let mut app = test_support::app();
        let (_, cx) = app.add_window_view(|_, _| Narrow);
        cx.run_until_parked();
        let line = cx.debug_bounds("one-line").expect("drawn");
        let description = cx.debug_bounds("confirmation-description-1").expect("drawn");
        let google = cx.debug_bounds("confirmation-button-0").expect("drawn");
        let terminal = cx.debug_bounds("confirmation-button-1").expect("drawn");
        let inside = (px(1. + 16.), px(240. - 1. - 16.));
        assert_eq!((description.left(), description.right()), inside, "{description:?}");
        assert!(description.size.height >= line.size.height * 3., "{description:?} isn't wrapped");
        assert!(description.top() >= terminal.bottom(), "{description:?} under {terminal:?}");
        // Each button at its own width, from the start: Terminal is
        // narrower than Log in with Google.
        assert_eq!((google.left(), terminal.left()), (inside.0, inside.0), "{google:?} {terminal:?}");
        assert!(google.right() < inside.1, "{google:?} stretched");
        assert!(terminal.size.width < google.size.width, "{terminal:?} {google:?}");
    }

    struct Host {
        state: ConfirmationState,
        allow: FocusHandle,
        reject: FocusHandle,
        asked: Rc<RefCell<Vec<ItemKey>>>,
        footer_drawn: Rc<RefCell<Vec<&'static str>>>,
    }

    impl Focusable for Host {
        fn focus_handle(&self, _: &App) -> FocusHandle {
            self.allow.clone()
        }
    }

    impl Render for Host {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            let asked = self.asked.clone();
            div().w(px(400.)).child(
                Confirmation::new("permission", "Run `cargo test`?", self.state.clone())
                    .action(ConfirmationAction::new("Allow once").key("allow").variant(ButtonVariant::Default).focus(&self.allow))
                    .action(ConfirmationAction::new("Reject").key("reject").focus(&self.reject))
                    .on_action(move |key, _, _| asked.borrow_mut().push(key.clone()))
                    .child({
                        let drawn = self.footer_drawn.clone();
                        let state = self.state.clone();
                        gpui::canvas(|_, _, _| {}, move |_, _, _, _| {
                            drawn.borrow_mut().push(if state.outcome().is_some() { "answered" } else { "requested" })
                        })
                        .h(px(1.))
                    }),
            )
        }
    }

    /// The spec's "Requested" and "Answered": space on the focused Reject
    /// asks for it, a click on Allow once for that; answered, the outcome
    /// shows and no buttons.
    #[test]
    fn requested_then_answered() {
        let mut app = test_support::app();
        let asked = Rc::new(RefCell::new(Vec::new()));
        let drawn = Rc::new(RefCell::new(Vec::new()));
        let (asked2, drawn2) = (asked.clone(), drawn.clone());
        let (host, cx) = app.add_window_view(move |_, cx| Host {
            state: ConfirmationState::Requested,
            allow: cx.focus_handle(),
            reject: cx.focus_handle(),
            asked: asked2,
            footer_drawn: drawn2,
        });
        cx.run_until_parked();
        cx.debug_bounds("confirmation-actions").expect("the buttons show");
        let allow = cx.debug_bounds("confirmation-action-0").expect("Allow once shows");
        let reject_button = cx.debug_bounds("confirmation-action-1").expect("Reject shows");
        assert!(allow.right() <= reject_button.left(), "in the owner's order");
        let reject = host.read_with(cx, |host, _| host.reject.clone());
        cx.update(|window, _| reject.focus(window));
        cx.run_until_parked();
        cx.simulate_keystrokes("space");
        assert_eq!(*asked.borrow(), [ItemKey::from("reject")]);
        cx.simulate_click(allow.center(), gpui::Modifiers::none());
        assert_eq!(*asked.borrow(), [ItemKey::from("reject"), ItemKey::from("allow")]);
        drawn.borrow_mut().clear();
        cx.update(|_, cx| {
            host.update(cx, |host, cx| {
                host.state = ConfirmationState::Accepted { label: "Allow once".into() };
                cx.notify();
            })
        });
        cx.run_until_parked();
        assert_eq!(*drawn.borrow(), ["answered"]);
        let outcome = cx.debug_bounds("confirmation-outcome").expect("the outcome shows");
        assert!(outcome.size.width > px(0.));
        // No buttons: space on the old handle asks nothing.
        cx.update(|window, _| reject.focus(window));
        cx.simulate_keystrokes("space");
        cx.simulate_click(allow.center(), gpui::Modifiers::none());
        assert_eq!(asked.borrow().len(), 2);
    }
}
