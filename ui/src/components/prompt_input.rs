//! `PromptInput`: AI Elements' prompt input, a [`Textarea`] with a submit
//! button, a queue row above it and a reason line beside the button, which
//! may end with an owner's action button.
//!
//! An entity: it owns its [`TextArea`] (the draft) and the focus handles of
//! its buttons, and the owner gives it a [`PromptStatus`], an optional
//! [`Queued`] text, an optional reason and an optional action (a labelled
//! button after the reason: what to do instead, while the input can't
//! send), and hears back through
//! [`PromptInputEvent`]s. It never changes its text on its own: a
//! `Submit` leaves the draft for the owner to clear once the prompt is
//! taken, and the queue row's buttons ask the owner and touch nothing.
//!
//! The submit button reads the status: ready, lucide `arrow-up`, disabled
//! while the text is blank; submitted, a turning `loader-circle`, disabled;
//! streaming, lucide `square`, which asks the owner to stop; disabled, the
//! arrow disabled and the text area read-only.
//!
//! Keys: around the text area, the `PromptInput` context binds enter to
//! [`PromptSubmit`] (which a text area passes on unless an input method is
//! composing) and escape to [`PromptStop`]. Enter asks the owner to submit
//! while ready or streaming (the owner queues a prompt sent during a turn)
//! with text that isn't blank, and is kept from the owner otherwise.
//! Escape asks the owner to stop while streaming and goes on to the owner's
//! own escape otherwise.
//!
//! Focus stops, for the owner's [`FocusStops`](crate::FocusStops): the text
//! area ([`Focusable`]), the reason line's action while it shows
//! ([`PromptInput::action_stop`]), the queue row's enabled buttons
//! ([`PromptInput::row_stops`]) and, while streaming, the stop button
//! ([`PromptInput::stop_button`]). The submit button takes no focus while
//! ready, since enter in the text area does what it does. A button the
//! status or the row takes away takes its focus with it: the owner moves
//! focus back to the text area then.

use std::time::Duration;

use gpui::{
    Animation, AnimationExt, App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, Hsla,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString, Styled, Subscription, Window, div,
    prelude::FluentBuilder, rems,
};

use crate::actions::{PromptStop, PromptSubmit};
use crate::components::{Button, Textarea, TooltipFor, trigger_label};
use crate::icon::Icon;
use crate::style::{ButtonSize, ButtonVariant};
use crate::text_area::{TextArea, TextAreaEvent};
use crate::theme::ActiveTheme;

/// What the owner is doing with prompts, which the submit button shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PromptStatus {
    /// Ready for a prompt: the arrow, enabled when the text isn't blank.
    #[default]
    Ready,
    /// A prompt is on its way: a spinner, disabled.
    Submitted,
    /// A turn is running: the stop square.
    Streaming,
    /// No prompt can be sent: the arrow disabled, the text read-only.
    Disabled,
}

/// A [`PromptInput`] stop that can have keyboard focus
/// ([`PromptInput::focused`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptFocus {
    /// The text area.
    Text,
    /// A waiting text's remove button.
    Remove,
    /// A held text's send-again button.
    SendAgain,
    /// A held text's discard button.
    Discard,
    /// The stop button.
    Stop,
    /// The reason line's action.
    Action,
}

/// Whether a held text can be sent again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Resend {
    Available,
    /// Not now, and why (the disabled button's tooltip).
    Unavailable(SharedString),
}

/// A text queued apart from the draft, shown in a row above it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Queued {
    /// Waiting to be sent: a remove button.
    Waiting { text: SharedString },
    /// Not sent, for `reason`: send-again and discard buttons.
    Held { text: SharedString, reason: SharedString, resend: Resend },
}

impl Queued {
    pub fn text(&self) -> &SharedString {
        match self {
            Queued::Waiting { text } | Queued::Held { text, .. } => text,
        }
    }

    /// The row's label: `queued`, or `not sent: ` and the reason.
    pub fn label(&self) -> SharedString {
        match self {
            Queued::Waiting { .. } => "queued".into(),
            Queued::Held { reason, .. } => format!("not sent: {reason}").into(),
        }
    }
}

/// What a prompt input asks its owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptInputEvent {
    /// Send this text (the draft, which stays until the owner clears it).
    Submit(String),
    /// Stop the running turn.
    Stop,
    /// Remove the queued text (a waiting one, or discard a held one).
    RemoveQueued,
    /// Send the held text again.
    ResendQueued,
    /// Do what the reason line's action says.
    Action,
}

/// What the submit button asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PromptRequest {
    Submit,
    Stop,
}

/// Whether a text counts as nothing to send.
pub fn is_blank(text: &str) -> bool {
    text.trim().is_empty()
}

/// The submit button's request, `None` while it is disabled.
pub fn button_request(status: PromptStatus, blank: bool) -> Option<PromptRequest> {
    match status {
        PromptStatus::Ready => (!blank).then_some(PromptRequest::Submit),
        PromptStatus::Streaming => Some(PromptRequest::Stop),
        PromptStatus::Submitted | PromptStatus::Disabled => None,
    }
}

/// Whether enter in the text area asks the owner to submit: while ready or
/// streaming, with text that isn't blank, and no input method composing.
pub fn enter_submits(status: PromptStatus, blank: bool, composing: bool) -> bool {
    match status {
        PromptStatus::Ready | PromptStatus::Streaming => !blank && !composing,
        PromptStatus::Submitted | PromptStatus::Disabled => false,
    }
}

/// Whether escape in the text area asks the owner to stop.
pub fn escape_stops(status: PromptStatus) -> bool {
    match status {
        PromptStatus::Streaming => true,
        PromptStatus::Ready | PromptStatus::Submitted | PromptStatus::Disabled => false,
    }
}

/// The first line of a queued text, as its row shows it.
pub fn first_line(text: &str) -> &str {
    text.lines().next().unwrap_or("")
}

/// How long the spinner takes for a turn.
const SPIN: Duration = Duration::from_millis(1000);

pub struct PromptInput {
    area: Entity<TextArea>,
    status: PromptStatus,
    queued: Option<Queued>,
    reason: Option<SharedString>,
    /// The reason line's action button's label.
    action: Option<SharedString>,
    held_color: Option<Hsla>,
    stop_handle: FocusHandle,
    action_handle: FocusHandle,
    remove_handle: FocusHandle,
    resend_handle: FocusHandle,
    discard_handle: FocusHandle,
    _changed: Subscription,
}

impl EventEmitter<PromptInputEvent> for PromptInput {}

impl Focusable for PromptInput {
    /// The text area's.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.area.read(cx).focus_handle(cx)
    }
}

impl PromptInput {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let area = cx.new(|cx| TextArea::new("", cx));
        // The button follows the text's blankness.
        let changed = cx.subscribe(&area, |_, _, _: &TextAreaEvent, cx| cx.notify());
        Self {
            area,
            status: PromptStatus::Ready,
            queued: None,
            reason: None,
            action: None,
            held_color: None,
            stop_handle: cx.focus_handle(),
            action_handle: cx.focus_handle(),
            remove_handle: cx.focus_handle(),
            resend_handle: cx.focus_handle(),
            discard_handle: cx.focus_handle(),
            _changed: changed,
        }
    }

    /// The text area: its text, placeholder, row limit and `Changed`.
    pub fn area(&self) -> &Entity<TextArea> {
        &self.area
    }

    pub fn text<'a>(&self, cx: &'a App) -> &'a str {
        self.area.read(cx).text()
    }

    /// Replaces the draft (a taken prompt clears it), the cursor at its end.
    pub fn set_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.area.update(cx, |area, cx| {
            area.set_text(text, cx);
            area.move_to_end(cx);
        });
        cx.notify();
    }

    pub fn set_placeholder(&mut self, placeholder: impl Into<SharedString>, cx: &mut Context<Self>) {
        let placeholder = placeholder.into();
        self.area.update(cx, |area, cx| area.set_placeholder(placeholder, cx));
    }

    pub fn status(&self) -> PromptStatus {
        self.status
    }

    pub fn set_status(&mut self, status: PromptStatus, cx: &mut Context<Self>) {
        if self.status != status {
            self.status = status;
            let read_only = status == PromptStatus::Disabled;
            self.area.update(cx, |area, cx| area.set_read_only(read_only, cx));
            cx.notify();
        }
    }

    pub fn queued(&self) -> Option<&Queued> {
        self.queued.as_ref()
    }

    pub fn set_queued(&mut self, queued: Option<Queued>, cx: &mut Context<Self>) {
        if self.queued != queued {
            self.queued = queued;
            cx.notify();
        }
    }

    /// A line beside the button: why the last submit was refused, why the
    /// input is disabled.
    pub fn set_reason(&mut self, reason: Option<SharedString>, cx: &mut Context<Self>) {
        if self.reason != reason {
            self.reason = reason;
            cx.notify();
        }
    }

    pub fn reason(&self) -> Option<&SharedString> {
        self.reason.as_ref()
    }

    /// A button labelled `label` after the reason, which asks the owner
    /// ([`PromptInputEvent::Action`]); `None` takes it away, and its focus
    /// with it (the owner moves focus back to the text area).
    pub fn set_action(&mut self, label: Option<SharedString>, cx: &mut Context<Self>) {
        if self.action != label {
            self.action = label;
            cx.notify();
        }
    }

    pub fn action(&self) -> Option<&SharedString> {
        self.action.as_ref()
    }

    /// The reason line's action button's handle while it shows.
    pub fn action_stop(&self) -> Option<FocusHandle> {
        self.action.as_ref().map(|_| self.action_handle.clone())
    }

    /// The colour of a held text's `not sent` label (the app's warning
    /// colour); the destructive colour by default.
    pub fn set_held_color(&mut self, color: Hsla, cx: &mut Context<Self>) {
        self.held_color = Some(color);
        cx.notify();
    }

    /// The queue row's enabled buttons, in order: remove for a waiting
    /// text; send again (when available) and discard for a held one.
    pub fn row_stops(&self) -> Vec<FocusHandle> {
        match &self.queued {
            None => Vec::new(),
            Some(Queued::Waiting { .. }) => vec![self.remove_handle.clone()],
            Some(Queued::Held { resend: Resend::Available, .. }) => {
                vec![self.resend_handle.clone(), self.discard_handle.clone()]
            }
            Some(Queued::Held { resend: Resend::Unavailable(_), .. }) => vec![self.discard_handle.clone()],
        }
    }

    /// The stop button's handle while it shows (streaming).
    pub fn stop_button(&self) -> Option<FocusHandle> {
        (self.status == PromptStatus::Streaming).then(|| self.stop_handle.clone())
    }

    /// Which of its stops has keyboard focus, if one of them has it (for an
    /// owner that reports where focus is).
    pub fn focused(&self, window: &Window, cx: &App) -> Option<PromptFocus> {
        let stops = [
            (self.area.read(cx).focus_handle(cx), PromptFocus::Text),
            (self.remove_handle.clone(), PromptFocus::Remove),
            (self.resend_handle.clone(), PromptFocus::SendAgain),
            (self.discard_handle.clone(), PromptFocus::Discard),
            (self.stop_handle.clone(), PromptFocus::Stop),
            (self.action_handle.clone(), PromptFocus::Action),
        ];
        stops.into_iter().find(|(handle, _)| handle.is_focused(window)).map(|(_, stop)| stop)
    }

    fn blank(&self, cx: &App) -> bool {
        is_blank(self.area.read(cx).text())
    }

    fn request(&mut self, request: PromptRequest, cx: &mut Context<Self>) {
        match request {
            PromptRequest::Submit => {
                let text = self.area.read(cx).text().to_string();
                cx.emit(PromptInputEvent::Submit(text));
            }
            PromptRequest::Stop => cx.emit(PromptInputEvent::Stop),
        }
    }

    fn on_submit(&mut self, _: &PromptSubmit, _: &mut Window, cx: &mut Context<Self>) {
        let area = self.area.read(cx);
        if enter_submits(self.status, is_blank(area.text()), area.composing()) {
            self.request(PromptRequest::Submit, cx);
        }
        // Otherwise the enter is kept: nothing is asked, and nothing
        // around the input acts on it either.
    }

    fn on_stop(&mut self, _: &PromptStop, _: &mut Window, cx: &mut Context<Self>) {
        if escape_stops(self.status) {
            self.request(PromptRequest::Stop, cx);
        } else {
            cx.propagate();
        }
    }

    fn submit_button(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let request = button_request(self.status, self.blank(cx));
        let icon = match self.status {
            PromptStatus::Ready | PromptStatus::Disabled => Icon::ArrowUp,
            PromptStatus::Submitted => Icon::LoaderCircle,
            PromptStatus::Streaming => Icon::Square,
        };
        let mut button = Button::icon_only("prompt-submit", icon)
            .size(ButtonSize::IconSm)
            .disabled(request.is_none())
            .on_click(cx.listener(move |this, _, _, cx| {
                if let Some(request) = request {
                    this.request(request, cx);
                }
            }));
        button = match self.status {
            PromptStatus::Streaming => button.focus(&self.stop_handle).tooltip_for("Stop", None),
            PromptStatus::Ready => button.tooltip_for("Send", None),
            PromptStatus::Submitted | PromptStatus::Disabled => button,
        };
        let wrapper = div().debug_selector(|| "prompt-submit".into()).flex_none();
        match self.status {
            PromptStatus::Submitted => wrapper
                .child(button.with_animation("prompt-spinner", Animation::new(SPIN).repeat(), |b, turn| b.icon_turn(turn)))
                .into_any_element(),
            PromptStatus::Ready | PromptStatus::Streaming | PromptStatus::Disabled => {
                wrapper.child(button).into_any_element()
            }
        }
    }

    fn queue_row(&self, queued: &Queued, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.ui_theme();
        let (muted, border, radius) = (t.muted_foreground, t.border, t.radius_md());
        let label_color = match queued {
            Queued::Waiting { .. } => muted,
            Queued::Held { .. } => self.held_color.unwrap_or(t.destructive),
        };
        let row_button = |id: &'static str, icon: Icon| {
            Button::icon_only(id, icon).variant(ButtonVariant::Ghost).size(ButtonSize::IconXs)
        };
        let emit = |event: PromptInputEvent| cx.listener(move |_, _, _, cx| cx.emit(event.clone()));
        let buttons: Vec<gpui::AnyElement> = match queued {
            Queued::Waiting { .. } => vec![
                div()
                    .debug_selector(|| "prompt-queue-remove".into())
                    .child(
                        row_button("prompt-queue-remove", Icon::X)
                            .focus(&self.remove_handle)
                            .tooltip_for("Remove queued prompt", None)
                            .on_click(emit(PromptInputEvent::RemoveQueued)),
                    )
                    .into_any_element(),
            ],
            Queued::Held { resend, .. } => {
                let resend_button = row_button("prompt-queue-resend", Icon::Send);
                let resend = match resend {
                    Resend::Available => div()
                        .id("prompt-queue-resend-frame")
                        .child(
                            resend_button
                                .focus(&self.resend_handle)
                                .tooltip_for("Send again", None)
                                .on_click(emit(PromptInputEvent::ResendQueued)),
                        ),
                    // A disabled button shows no tooltip; its frame says
                    // why it is disabled.
                    Resend::Unavailable(why) => div()
                        .id("prompt-queue-resend-frame")
                        .tooltip_for(why.clone(), None)
                        .child(resend_button.disabled(true)),
                };
                vec![
                    resend.debug_selector(|| "prompt-queue-resend".into()).into_any_element(),
                    div()
                        .debug_selector(|| "prompt-queue-discard".into())
                        .child(
                            row_button("prompt-queue-discard", Icon::X)
                                .focus(&self.discard_handle)
                                .tooltip_for("Discard", None)
                                .on_click(emit(PromptInputEvent::RemoveQueued)),
                        )
                        .into_any_element(),
                ]
            }
        };
        // rounded-md border px-3 py-1 text-xs: the text's first line, cut,
        // then the label, then the buttons.
        div()
            .debug_selector(|| "prompt-queue-row".into())
            .flex()
            .items_center()
            .gap_2()
            .w_full()
            .min_w_0()
            .pl_3()
            .pr_1()
            .py_0p5()
            .rounded(radius)
            .border_1()
            .border_color(border)
            .text_size(rems(0.75))
            // Both cut with an ellipsis (`truncate` cuts nothing in a flex
            // row in gpui 0.2.2): the text takes the room left, keeping a
            // little; the label its own width, giving way after.
            .child(trigger_label(first_line(queued.text()).to_string().into()).min_w(rems(3.)))
            .child(trigger_label(queued.label()).flex_initial().text_color(label_color))
            .children(buttons)
    }
}

impl Render for PromptInput {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let row = self.queued.clone().map(|queued| self.queue_row(&queued, cx));
        let button = self.submit_button(cx);
        let muted = cx.ui_theme().muted_foreground;
        // An outline button after the reason, at the toolbar's height.
        let action = self.action.clone().map(|label| {
            div().debug_selector(|| "prompt-action".into()).flex_none().child(
                Button::new("prompt-action", label)
                    .variant(ButtonVariant::Outline)
                    .size(ButtonSize::Sm)
                    .focus(&self.action_handle)
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(PromptInputEvent::Action))),
            )
        });
        div()
            .flex()
            .flex_col()
            .gap_1p5()
            .w_full()
            .min_w_0()
            .children(row)
            // The context wraps the text area alone, so the row's and the
            // toolbar's buttons keep their own enter.
            .child(
                div()
                    .debug_selector(|| "prompt-area".into())
                    .key_context("PromptInput")
                    .on_action(cx.listener(Self::on_submit))
                    .on_action(cx.listener(Self::on_stop))
                    .child(Textarea::new(&self.area)),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .min_w_0()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                div()
                                    .min_w_0()
                                    .text_size(rems(0.75))
                                    .text_color(muted)
                                    .when_some(self.reason.clone(), |d, reason| d.child(reason)),
                            )
                            .children(action),
                    )
                    .child(button),
            )
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui::{AppContext, Bounds, EntityInputHandler, Modifiers, Pixels, VisualTestContext, actions, px};

    use super::*;
    use crate::test_support;

    actions!(prompt_input_tests, [OwnerEscape]);

    #[test]
    fn the_button_and_the_keys_follow_the_status() {
        use PromptStatus::{Disabled, Ready, Streaming, Submitted};
        assert_eq!(button_request(Ready, false), Some(PromptRequest::Submit));
        assert_eq!(button_request(Ready, true), None);
        assert_eq!(button_request(Streaming, true), Some(PromptRequest::Stop));
        assert_eq!(button_request(Streaming, false), Some(PromptRequest::Stop));
        assert_eq!(button_request(Submitted, false), None);
        assert_eq!(button_request(Disabled, false), None);
        assert!(enter_submits(Ready, false, false));
        assert!(enter_submits(Streaming, false, false));
        assert!(!enter_submits(Ready, true, false));
        assert!(!enter_submits(Ready, false, true));
        assert!(!enter_submits(Submitted, false, false));
        assert!(!enter_submits(Disabled, false, false));
        assert!(escape_stops(Streaming));
        assert!(!escape_stops(Ready) && !escape_stops(Submitted) && !escape_stops(Disabled));
        assert!(is_blank("") && is_blank(" \n\t ") && !is_blank(" x "));
    }

    #[test]
    fn the_row_shows_the_first_line_and_the_label() {
        assert_eq!(first_line("next\nand more"), "next");
        assert_eq!(first_line(""), "");
        let waiting = Queued::Waiting { text: "next".into() };
        assert_eq!(waiting.label(), "queued");
        let held =
            Queued::Held { text: "next".into(), reason: "the turn was stopped".into(), resend: Resend::Available };
        assert_eq!((held.text().as_ref(), held.label().as_ref()), ("next", "not sent: the turn was stopped"));
    }

    struct Host {
        input: Entity<PromptInput>,
        events: Rc<RefCell<Vec<PromptInputEvent>>>,
        escapes: usize,
        _events: Subscription,
    }

    impl Render for Host {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .key_context("Owner")
                .on_action(cx.listener(|this, _: &OwnerEscape, _, _| this.escapes += 1))
                .w(px(300.))
                .p(px(10.))
                .child(self.input.clone())
        }
    }

    type Events = Rc<RefCell<Vec<PromptInputEvent>>>;

    fn host() -> (Entity<Host>, Entity<PromptInput>, Events, &'static mut VisualTestContext) {
        let mut app = test_support::app();
        app.update(|cx| cx.bind_keys([gpui::KeyBinding::new("escape", OwnerEscape, Some("Owner"))]));
        let (host, cx) = app.add_window_view(|_, cx| {
            let input = cx.new(PromptInput::new);
            let events = Events::default();
            let sink = events.clone();
            let subscription = cx.subscribe(&input, move |_, _, event: &PromptInputEvent, _| {
                sink.borrow_mut().push(event.clone());
            });
            Host { input, events, escapes: 0, _events: subscription }
        });
        let (input, events) = host.read_with(cx, |host, _| (host.input.clone(), host.events.clone()));
        let cx: &'static mut VisualTestContext = Box::leak(Box::new(cx.clone()));
        cx.update(|window, cx| input.read(cx).focus_handle(cx).focus(window));
        cx.run_until_parked();
        (host, input, events, cx)
    }

    fn update(input: &Entity<PromptInput>, cx: &mut VisualTestContext, f: impl FnOnce(&mut PromptInput, &mut Context<PromptInput>)) {
        cx.update(|_, cx| input.update(cx, f));
        cx.run_until_parked();
    }

    fn bounds(selector: &'static str, cx: &mut VisualTestContext) -> Option<Bounds<Pixels>> {
        cx.debug_bounds(selector)
    }

    fn click(selector: &'static str, cx: &mut VisualTestContext) {
        let at = bounds(selector, cx).unwrap_or_else(|| panic!("{selector} is drawn")).center();
        cx.simulate_click(at, Modifiers::none());
        cx.run_until_parked();
    }

    fn take(events: &Events) -> Vec<PromptInputEvent> {
        std::mem::take(&mut *events.borrow_mut())
    }

    /// The spec's "Stop while streaming": the button asks to stop, not to
    /// submit, and so does escape; ready, escape is the owner's.
    #[test]
    fn stop_while_streaming() {
        let (host, input, events, cx) = host();
        cx.simulate_input("hello");
        update(&input, cx, |input, cx| input.set_status(PromptStatus::Streaming, cx));
        click("prompt-submit", cx);
        assert_eq!(take(&events), [PromptInputEvent::Stop]);
        cx.update(|window, cx| input.read(cx).focus_handle(cx).focus(window));
        cx.simulate_keystrokes("escape");
        assert_eq!(take(&events), [PromptInputEvent::Stop]);
        // While streaming, enter still submits (the owner queues it).
        cx.simulate_keystrokes("enter");
        assert_eq!(take(&events), [PromptInputEvent::Submit("hello".into())]);
        // The stop button is a stop, and takes enter itself.
        let stop = input.read_with(cx, |input, _| input.stop_button()).expect("streaming");
        cx.update(|window, _| stop.focus(window));
        cx.run_until_parked();
        cx.simulate_keystrokes("enter");
        assert_eq!(take(&events), [PromptInputEvent::Stop]);
        update(&input, cx, |input, cx| input.set_status(PromptStatus::Ready, cx));
        assert!(input.read_with(cx, |input, _| input.stop_button()).is_none());
        cx.update(|window, cx| input.read(cx).focus_handle(cx).focus(window));
        cx.simulate_keystrokes("escape");
        assert!(take(&events).is_empty());
        assert_eq!(host.read_with(cx, |host, _| host.escapes), 1);
        // The draft is the owner's to clear.
        assert_eq!(input.read_with(cx, |input, cx| input.text(cx).to_string()), "hello");
    }

    /// The spec's "Blank text": ready and blank, the button is disabled
    /// and enter asks nothing; with text, both submit.
    #[test]
    fn blank_text_asks_nothing() {
        let (_, input, events, cx) = host();
        cx.simulate_keystrokes("enter");
        click("prompt-submit", cx);
        cx.simulate_input("  ");
        cx.simulate_keystrokes("enter");
        assert!(take(&events).is_empty());
        assert_eq!(input.read_with(cx, |input, cx| input.text(cx).to_string()), "  ");
        cx.simulate_input("go");
        click("prompt-submit", cx);
        assert_eq!(take(&events), [PromptInputEvent::Submit("  go".into())]);
        cx.update(|window, cx| input.read(cx).focus_handle(cx).focus(window));
        cx.simulate_keystrokes("enter");
        assert_eq!(take(&events), [PromptInputEvent::Submit("  go".into())]);
        // Submitted and disabled ask nothing either.
        for status in [PromptStatus::Submitted, PromptStatus::Disabled] {
            update(&input, cx, |input, cx| input.set_status(status, cx));
            cx.simulate_keystrokes("enter");
            click("prompt-submit", cx);
            assert!(take(&events).is_empty(), "{status:?}");
        }
        // Disabled, the text is read-only.
        cx.update(|window, cx| input.read(cx).focus_handle(cx).focus(window));
        cx.simulate_input("x");
        assert_eq!(input.read_with(cx, |input, cx| input.text(cx).to_string()), "  go");
    }

    /// Enter while an input method composes commits the text and asks
    /// nothing.
    #[test]
    fn enter_while_composing_submits_nothing() {
        let (_, input, events, cx) = host();
        cx.simulate_input("ab");
        let area = input.read_with(cx, |input, _| input.area().clone());
        cx.update(|window, cx| {
            area.update(cx, |area, cx| area.replace_and_mark_text_in_range(None, "e", None, window, cx));
        });
        cx.simulate_keystrokes("enter");
        assert!(take(&events).is_empty());
    }

    /// The spec's "Queued row": a row above the text area shows the text
    /// with a remove button, which asks to remove it, by pointer and by
    /// key, and the draft is untouched.
    #[test]
    fn a_queued_row_asks_to_remove() {
        let (_, input, events, cx) = host();
        cx.simulate_input("draft");
        update(&input, cx, |input, cx| input.set_queued(Some(Queued::Waiting { text: "next".into() }), cx));
        let row = bounds("prompt-queue-row", cx).expect("the row shows");
        let area = bounds("prompt-area", cx).expect("drawn");
        assert!(row.bottom() <= area.top(), "{row:?} above {area:?}");
        click("prompt-queue-remove", cx);
        assert_eq!(take(&events), [PromptInputEvent::RemoveQueued]);
        let stops = input.read_with(cx, |input, _| input.row_stops());
        assert_eq!(stops.len(), 1);
        cx.update(|window, _| stops[0].focus(window));
        cx.run_until_parked();
        cx.simulate_keystrokes("space");
        cx.simulate_keystrokes("enter");
        assert_eq!(take(&events), [PromptInputEvent::RemoveQueued, PromptInputEvent::RemoveQueued]);
        assert_eq!(input.read_with(cx, |input, cx| input.text(cx).to_string()), "draft");
        // Gone, the text area moves up to the top (gpui keeps a selector's
        // last bounds, so the row's own can't show it is gone).
        update(&input, cx, |input, cx| input.set_queued(None, cx));
        assert_eq!(bounds("prompt-area", cx).map(|b| b.top()), Some(px(10.)));
        assert!(input.read_with(cx, |input, _| input.row_stops()).is_empty());
    }

    /// The spec's "Held row": send again and discard each ask the owner,
    /// and the text area's text is unchanged; send again unavailable is no
    /// stop and asks nothing.
    #[test]
    fn a_held_row_asks_to_resend_or_discard() {
        let (_, input, events, cx) = host();
        cx.simulate_input("draft");
        let held = |resend| Queued::Held { text: "next".into(), reason: "the turn was stopped".into(), resend };
        update(&input, cx, |input, cx| input.set_queued(Some(held(Resend::Available)), cx));
        click("prompt-queue-resend", cx);
        click("prompt-queue-discard", cx);
        assert_eq!(take(&events), [PromptInputEvent::ResendQueued, PromptInputEvent::RemoveQueued]);
        let stops = input.read_with(cx, |input, _| input.row_stops());
        assert_eq!(stops.len(), 2);
        for stop in &stops {
            cx.update(|window, _| stop.focus(window));
            cx.run_until_parked();
            cx.simulate_keystrokes("space");
        }
        assert_eq!(take(&events), [PromptInputEvent::ResendQueued, PromptInputEvent::RemoveQueued]);
        assert_eq!(input.read_with(cx, |input, cx| input.text(cx).to_string()), "draft");
        update(&input, cx, |input, cx| {
            input.set_queued(Some(held(Resend::Unavailable("the agent isn't ready".into()))), cx)
        });
        click("prompt-queue-resend", cx);
        assert!(take(&events).is_empty());
        assert_eq!(input.read_with(cx, |input, _| input.row_stops()), [stops[1].clone()]);
    }

    /// An action after the reason asks the owner, by pointer and by key,
    /// is a stop while it shows and is named by `focused`; taken away, it is
    /// no stop. The draft is untouched.
    #[test]
    fn the_reason_lines_action_asks_the_owner() {
        let (_, input, events, cx) = host();
        cx.simulate_input("draft");
        assert!(input.read_with(cx, |input, _| input.action_stop()).is_none());
        update(&input, cx, |input, cx| {
            input.set_status(PromptStatus::Disabled, cx);
            input.set_reason(Some("the agent has exited".into()), cx);
            input.set_action(Some("Continue conversation".into()), cx);
        });
        let reason = bounds("prompt-action", cx).expect("the action shows");
        let area = bounds("prompt-area", cx).expect("drawn");
        assert!(reason.top() >= area.bottom(), "{reason:?} under {area:?}");
        click("prompt-action", cx);
        assert_eq!(take(&events), [PromptInputEvent::Action]);
        let stop = input.read_with(cx, |input, _| input.action_stop()).expect("a stop");
        cx.update(|window, _| stop.focus(window));
        cx.run_until_parked();
        assert_eq!(cx.update(|window, cx| input.read(cx).focused(window, cx)), Some(PromptFocus::Action));
        cx.simulate_keystrokes("enter");
        cx.simulate_keystrokes("space");
        assert_eq!(take(&events), [PromptInputEvent::Action, PromptInputEvent::Action]);
        assert_eq!(input.read_with(cx, |input, cx| input.text(cx).to_string()), "draft");
        update(&input, cx, |input, cx| input.set_action(None, cx));
        assert!(input.read_with(cx, |input, _| input.action_stop()).is_none());
    }

    /// Submitted draws the spinner, and a set text replaces the draft.
    #[test]
    fn submitted_spins_and_the_owner_sets_the_text() {
        let (_, input, _, cx) = host();
        update(&input, cx, |input, cx| {
            input.set_text("again", cx);
            input.set_status(PromptStatus::Submitted, cx);
            input.set_reason(Some("a prompt is already queued".into()), cx);
        });
        assert!(bounds("prompt-submit", cx).is_some());
        assert_eq!(input.read_with(cx, |input, cx| input.text(cx).to_string()), "again");
        assert_eq!(input.read_with(cx, |input, _| input.reason().cloned()), Some("a prompt is already queued".into()));
    }
}
