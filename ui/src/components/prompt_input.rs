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
    InteractiveElement, IntoElement, KeyContext, ParentElement, Render, SharedString, Styled, Subscription, Window, div, prelude::FluentBuilder, rems,
};

use crate::actions::{
    PromptStop, PromptSubmit, PromptSuggestionDown, PromptSuggestionEnter, PromptSuggestionEscape, PromptSuggestionTab,
    PromptSuggestionUp,
};
use crate::components::{Button, Textarea, Tooltip, TooltipFor, trigger_label};
use crate::icon::{Icon, IconElement};
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
#[derive(Clone, Debug, PartialEq, Eq)]
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
    /// The reason line's action with this key.
    Action(SharedString),
}

/// A button on the reason line: its owner's key for it, and its label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PromptAction {
    pub key: SharedString,
    pub label: SharedString,
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

/// A row of the suggestions above the text area: the owner's key for it,
/// its label, and an optional hint and description, each cut to one line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub key: SharedString,
    pub label: SharedString,
    pub hint: Option<SharedString>,
    pub description: Option<SharedString>,
}

/// The suggestions the owner holds, and the row it highlights.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestions {
    pub rows: Vec<Suggestion>,
    pub highlighted: usize,
}

/// A key the suggestions take while they show, before the text area's own:
/// the owner says what each does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuggestionKey {
    Up,
    Down,
    Enter,
    Tab,
    Escape,
}

/// An attachment the owner's next prompt takes, shown in a row above the
/// text area: its icon, its name (cut with an ellipsis), an optional detail
/// such as `2.1 KiB`, and the owner's key for it, which a press on its
/// remove mark reports ([`PromptInputEvent::RemoveAttachment`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttachmentItem {
    pub key: SharedString,
    pub icon: Icon,
    pub name: SharedString,
    pub detail: Option<SharedString>,
}

/// An item under the text area: the owner's key for it, and its label.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FooterItem {
    pub key: SharedString,
    pub label: SharedString,
}

/// What a prompt input asks its owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PromptInputEvent {
    /// A key the suggestions take while they show.
    SuggestionKey(SuggestionKey),
    /// The suggestion with this key was pressed: accept it.
    SuggestionPressed(SharedString),
    /// The footer item with this key was pressed.
    Footer(SharedString),
    /// Send this text (the draft, which stays until the owner clears it).
    Submit(String),
    /// Stop the running turn.
    Stop,
    /// Remove the queued text (a waiting one, or discard a held one).
    RemoveQueued,
    /// Send the held text again.
    ResendQueued,
    /// Do what the reason line's action with this key says.
    Action(SharedString),
    /// Remove the attachment with this key; the row is unchanged until the
    /// owner changes it.
    RemoveAttachment(SharedString),
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
    /// The owner's attachments, in a row under the queue row.
    attachments: Vec<AttachmentItem>,
    reason: Option<SharedString>,
    /// The reason line's action buttons, in order.
    actions: Vec<PromptAction>,
    /// The owner's suggestions above the text area, while it gives some.
    suggestions: Option<Suggestions>,
    /// The owner's items under the text area.
    footer: Vec<FooterItem>,
    held_color: Option<Hsla>,
    stop_handle: FocusHandle,
    /// The action buttons' handles by key (`types R3`): a button keeps its
    /// handle, and its focus, while the others change.
    action_handles: Vec<(SharedString, FocusHandle)>,
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
            attachments: Vec::new(),
            reason: None,
            actions: Vec::new(),
            suggestions: None,
            footer: Vec::new(),
            held_color: None,
            stop_handle: cx.focus_handle(),
            action_handles: Vec::new(),
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

    /// Replaces the draft with the cursor at byte `cursor` (an owner's
    /// suggestion written in).
    pub fn set_text_at(&mut self, text: &str, cursor: usize, cx: &mut Context<Self>) {
        let cursor = cursor.min(text.len());
        self.area.update(cx, |area, cx| {
            area.set_text(text, cx);
            area.select(cursor..cursor, cx);
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
    /// The attachments the row above the text area shows; none hides it.
    /// The items are no focus stops: removing one by key is the owner's
    /// (backspace at the text's start, a command).
    pub fn set_attachments(&mut self, attachments: Vec<AttachmentItem>, cx: &mut Context<Self>) {
        if self.attachments != attachments {
            self.attachments = attachments;
            cx.notify();
        }
    }

    pub fn attachments(&self) -> &[AttachmentItem] {
        &self.attachments
    }

    pub fn set_reason(&mut self, reason: Option<SharedString>, cx: &mut Context<Self>) {
        if self.reason != reason {
            self.reason = reason;
            cx.notify();
        }
    }

    pub fn reason(&self) -> Option<&SharedString> {
        self.reason.as_ref()
    }

    /// One button labelled `label` after the reason, keyed by its label,
    /// which asks the owner ([`PromptInputEvent::Action`]); `None` takes it
    /// away, and its focus with it (the owner moves focus back to the text
    /// area).
    pub fn set_action(&mut self, label: Option<SharedString>, cx: &mut Context<Self>) {
        let actions = label.map(|label| PromptAction { key: label.clone(), label }).into_iter().collect();
        self.set_actions(actions, cx);
    }

    /// Buttons for `actions` after the reason, in order, each asking the
    /// owner with its key ([`PromptInputEvent::Action`]); none takes them
    /// away.
    pub fn set_actions(&mut self, actions: Vec<PromptAction>, cx: &mut Context<Self>) {
        for action in &actions {
            if !self.action_handles.iter().any(|(key, _)| *key == action.key) {
                self.action_handles.push((action.key.clone(), cx.focus_handle()));
            }
        }
        if self.actions != actions {
            self.actions = actions;
            cx.notify();
        }
    }

    /// Suggestions above the text area, none taking focus: while they show,
    /// up, down, enter, tab and escape in the text area ask the owner
    /// ([`PromptInputEvent::SuggestionKey`]) rather than doing their own,
    /// enter going on to the text area while an input method composes; a
    /// press on a row asks the owner with its key. `None` (or no rows)
    /// takes them away. The owner holds the rows and the highlight, and
    /// changes the text itself.
    pub fn set_suggestions(&mut self, suggestions: Option<Suggestions>, cx: &mut Context<Self>) {
        let suggestions = suggestions.filter(|s| !s.rows.is_empty());
        if self.suggestions != suggestions {
            self.suggestions = suggestions;
            cx.notify();
        }
    }

    pub fn suggestions(&self) -> Option<&Suggestions> {
        self.suggestions.as_ref()
    }

    /// Items under the text area, separated by a middle dot, each asking
    /// the owner with its key on a press ([`PromptInputEvent::Footer`]);
    /// none is a keyboard stop.
    pub fn set_footer(&mut self, footer: Vec<FooterItem>, cx: &mut Context<Self>) {
        if self.footer != footer {
            self.footer = footer;
            cx.notify();
        }
    }

    pub fn footer(&self) -> &[FooterItem] {
        &self.footer
    }

    /// The first action's label.
    pub fn action(&self) -> Option<&SharedString> {
        self.actions.first().map(|action| &action.label)
    }

    /// The actions, in order.
    pub fn actions(&self) -> &[PromptAction] {
        &self.actions
    }

    fn action_handle(&self, key: &SharedString) -> Option<&FocusHandle> {
        self.action_handles.iter().find(|(k, _)| k == key).map(|(_, handle)| handle)
    }

    /// The first action button's handle while it shows.
    pub fn action_stop(&self) -> Option<FocusHandle> {
        self.action_stops().into_iter().next()
    }

    /// The action buttons' handles while they show, in order.
    pub fn action_stops(&self) -> Vec<FocusHandle> {
        self.actions.iter().filter_map(|action| self.action_handle(&action.key).cloned()).collect()
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
        ];
        let actions = self.action_handles.iter().map(|(key, handle)| (handle.clone(), PromptFocus::Action(key.clone())));
        stops.into_iter().chain(actions).find(|(handle, _)| handle.is_focused(window)).map(|(_, stop)| stop)
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

    /// A key the suggestions take: the owner's while they show. Enter while
    /// an input method composes goes on to the text area, which commits it.
    fn suggestion_key(&mut self, key: SuggestionKey, cx: &mut Context<Self>) {
        let composing = key == SuggestionKey::Enter && self.area.read(cx).composing();
        if self.suggestions.is_none() || composing {
            cx.propagate();
            return;
        }
        cx.emit(PromptInputEvent::SuggestionKey(key));
    }

    /// The suggestions above the text area: a row per suggestion, its
    /// label, hint and description on one line, the highlighted one on the
    /// accent; a press asks the owner. No row takes focus.
    fn suggestion_list(&self, suggestions: &Suggestions, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.ui_theme();
        let (muted, accent, accent_fg, border, radius) =
            (t.muted_foreground, t.accent, t.accent_foreground, t.border, t.radius_md());
        let rows: Vec<gpui::AnyElement> = suggestions
            .rows
            .iter()
            .enumerate()
            .map(|(ix, row)| {
                let key = row.key.clone();
                let highlighted = ix == suggestions.highlighted;
                let selector: SharedString = format!("prompt-suggestion-{ix}").into();
                div()
                    .id(gpui::ElementId::Name(selector.clone()))
                    .debug_selector(move || selector.to_string())
                    .flex()
                    .items_center()
                    .gap_2()
                    .w_full()
                    .min_w_0()
                    .px_2()
                    .py_0p5()
                    .rounded(radius)
                    .cursor_pointer()
                    .when(highlighted, |d| d.bg(accent).text_color(accent_fg))
                    .child(trigger_label(row.label.clone()).flex_initial())
                    .when_some(row.hint.clone(), |d, hint| d.child(trigger_label(hint).flex_initial().text_color(muted)))
                    .when_some(row.description.clone(), |d, description| {
                        d.child(trigger_label(description).min_w(rems(3.)).text_color(muted))
                    })
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |_, _, window, cx| {
                        // The text area keeps focus: the press is the owner's.
                        window.prevent_default();
                        cx.emit(PromptInputEvent::SuggestionPressed(key.clone()));
                        cx.stop_propagation();
                    }))
                    .into_any_element()
            })
            .collect();
        div()
            .debug_selector(|| "prompt-suggestions".into())
            .flex()
            .flex_col()
            .w_full()
            .min_w_0()
            .p_1()
            .rounded(radius)
            .border_1()
            .border_color(border)
            .text_size(rems(0.8125))
            .children(rows)
    }

    /// The items under the text area, `·` between them: small ghost
    /// labels, each a press asking the owner, none a stop.
    fn footer_row(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.ui_theme();
        let (muted, fg, hover, radius) = (t.muted_foreground, t.foreground, t.accent, t.radius_md());
        let mut children: Vec<gpui::AnyElement> = Vec::new();
        for (ix, item) in self.footer.iter().enumerate() {
            if ix > 0 {
                children.push(div().text_color(muted).child("·").into_any_element());
            }
            let key = item.key.clone();
            let selector: SharedString = format!("prompt-footer-{ix}").into();
            children.push(
                div()
                    .id(gpui::ElementId::Name(selector.clone()))
                    .debug_selector(move || selector.to_string())
                    .px_1()
                    .rounded(radius)
                    .cursor_pointer()
                    .text_color(muted)
                    .hover(move |s| s.bg(hover).text_color(fg))
                    .child(item.label.clone())
                    .on_mouse_down(gpui::MouseButton::Left, cx.listener(move |_, _, window, cx| {
                        window.prevent_default();
                        cx.emit(PromptInputEvent::Footer(key.clone()));
                        cx.stop_propagation();
                    }))
                    .into_any_element(),
            );
        }
        div()
            .debug_selector(|| "prompt-footer".into())
            .flex()
            .flex_wrap()
            .items_center()
            .gap_1()
            .text_size(rems(0.75))
            .children(children)
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
                    // On a plain element, so the theme scope's set is
                    // carried by hand, as a component's tooltip carries it.
                    Resend::Unavailable(why) => {
                        let (why, scope) = (why.clone(), cx.scoped_theme());
                        div()
                            .id("prompt-queue-resend-frame")
                            .tooltip_with(move |_, _| Tooltip::new(why.clone()).in_scope(scope))
                            .child(resend_button.disabled(true))
                    }
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

    /// The attachments' row: each item a small bordered chip, wrapping onto
    /// more lines when the row is full.
    fn attachment_row(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let t = cx.ui_theme();
        let (muted, border, radius) = (t.muted_foreground, t.border, t.radius_md());
        let items = self.attachments.iter().enumerate().map(|(ix, item)| {
            let key = item.key.clone();
            let remove = Button::icon_only(gpui::ElementId::NamedInteger("prompt-attachment-remove".into(), ix as u64), Icon::X)
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::IconXs)
                .tooltip_for("Remove attachment", None)
                .on_click(cx.listener(move |_, _, _, cx| cx.emit(PromptInputEvent::RemoveAttachment(key.clone()))));
            let selector: SharedString = format!("prompt-attachment-{ix}").into();
            let remove_selector: SharedString = format!("prompt-attachment-remove-{ix}").into();
            div()
                .debug_selector(move || selector.to_string())
                .flex()
                .items_center()
                .gap_1()
                .max_w(rems(16.))
                .min_w_0()
                .pl_2()
                .rounded(radius)
                .border_1()
                .border_color(border)
                .child(IconElement::new(item.icon).size(rems(0.875)).color(muted))
                .child(trigger_label(item.name.clone()).min_w(rems(2.)))
                .when_some(item.detail.clone(), |chip, detail| {
                    chip.child(div().flex_none().whitespace_nowrap().text_color(muted).child(detail))
                })
                .child(div().debug_selector(move || remove_selector.to_string()).child(remove))
        });
        div()
            .debug_selector(|| "prompt-attachments".into())
            .flex()
            .flex_wrap()
            .gap_1()
            .w_full()
            .min_w_0()
            .text_size(rems(0.75))
            .children(items)
    }
}

impl Render for PromptInput {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let row = self.queued.clone().map(|queued| self.queue_row(&queued, cx));
        let attachments = (!self.attachments.is_empty()).then(|| self.attachment_row(cx));
        let button = self.submit_button(cx);
        let muted = cx.ui_theme().muted_foreground;
        // An outline button after the reason, at the toolbar's height.
        // The first is `prompt-action`, as with one; the others add their
        // place (`prompt-action-1`, …).
        let action: Vec<_> = self
            .actions
            .iter()
            .enumerate()
            .filter_map(|(ix, action)| {
                let handle = self.action_handle(&action.key)?.clone();
                let id: SharedString = if ix == 0 { "prompt-action".into() } else { format!("prompt-action-{ix}").into() };
                let key = action.key.clone();
                Some(
                    div().debug_selector({
                        let id = id.clone();
                        move || id.to_string()
                    })
                    .flex_none()
                    .child(
                        Button::new(id, action.label.clone())
                            .variant(ButtonVariant::Outline)
                            .size(ButtonSize::Sm)
                            .focus(&handle)
                            .on_click(cx.listener(move |_, _, _, cx| cx.emit(PromptInputEvent::Action(key.clone())))),
                    ),
                )
            })
            .collect();
        let suggestions = self.suggestions.clone().map(|suggestions| self.suggestion_list(&suggestions, cx));
        let footer = (!self.footer.is_empty()).then(|| self.footer_row(cx));
        // While suggestions show, their keys are bound in this context.
        let mut context = KeyContext::default();
        context.add("PromptInput");
        if self.suggestions.is_some() {
            context.add("PromptSuggesting");
        }
        div()
            .flex()
            .flex_col()
            .gap_1p5()
            .w_full()
            .min_w_0()
            .children(row)
            .children(attachments)
            .children(suggestions)
            // The context wraps the text area alone, so the row's and the
            // toolbar's buttons keep their own enter.
            .child(
                div()
                    .debug_selector(|| "prompt-area".into())
                    .key_context(context)
                    .on_action(cx.listener(Self::on_submit))
                    .on_action(cx.listener(Self::on_stop))
                    .on_action(cx.listener(|this, _: &PromptSuggestionUp, _, cx| this.suggestion_key(SuggestionKey::Up, cx)))
                    .on_action(
                        cx.listener(|this, _: &PromptSuggestionDown, _, cx| this.suggestion_key(SuggestionKey::Down, cx)),
                    )
                    .on_action(
                        cx.listener(|this, _: &PromptSuggestionEnter, _, cx| this.suggestion_key(SuggestionKey::Enter, cx)),
                    )
                    .on_action(cx.listener(|this, _: &PromptSuggestionTab, _, cx| this.suggestion_key(SuggestionKey::Tab, cx)))
                    .on_action(
                        cx.listener(|this, _: &PromptSuggestionEscape, _, cx| {
                            this.suggestion_key(SuggestionKey::Escape, cx)
                        }),
                    )
                    .child(Textarea::new(&self.area)),
            )
            .children(footer)
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
                            // The reason grows from a zero basis into the
                            // row's room, so it wraps at the row's width and
                            // never at a measure of its own that gpui 0.2.2
                            // may keep from a narrower pass (the wrapping
                            // text pitfall); the action sits at its end.
                            .child(
                                div()
                                    .debug_selector(|| "prompt-reason".into())
                                    .flex_1()
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

    fn item(key: &str, name: &str, detail: Option<&str>) -> AttachmentItem {
        AttachmentItem {
            key: key.to_string().into(),
            icon: Icon::FileText,
            name: name.to_string().into(),
            detail: detail.map(|detail| detail.to_string().into()),
        }
    }

    /// The spec's "Prompt input attachments": the row sits under the queue
    /// row and above the text area, a long name is cut to the row's width,
    /// a press on a remove mark asks the owner with its key and changes
    /// nothing, and the items add no focus stop; none hides the row.
    #[test]
    fn the_attachments_row_asks_the_owner_to_remove_and_takes_no_stop() {
        let (_, input, events, cx) = host();
        cx.simulate_input("draft");
        update(&input, cx, |input, cx| input.set_queued(Some(Queued::Waiting { text: "next".into() }), cx));
        let stops = input.read_with(cx, |input, _| input.row_stops()).len();
        let long = "cargo test --workspace --locked -- --nocapture and a great deal more besides";
        let items = vec![item("a", long, Some("12 KiB")), item("b", "main.rs", None)];
        update(&input, cx, |input, cx| input.set_attachments(items.clone(), cx));
        let (queue, row, area) = (
            bounds("prompt-queue-row", cx).expect("the queue row shows"),
            bounds("prompt-attachments", cx).expect("the row shows"),
            bounds("prompt-area", cx).expect("drawn"),
        );
        assert!(queue.bottom() <= row.top() && row.bottom() <= area.top(), "{queue:?} {row:?} {area:?}");
        let first = bounds("prompt-attachment-0", cx).expect("drawn");
        assert!(first.right() <= row.right() + px(0.5), "the long name is cut: {first:?} in {row:?}");
        click("prompt-attachment-remove-1", cx);
        assert_eq!(take(&events), [PromptInputEvent::RemoveAttachment("b".into())]);
        assert_eq!(input.read_with(cx, |input, _| input.attachments().to_vec()), items, "the owner's to change");
        assert_eq!(input.read_with(cx, |input, _| input.row_stops()).len(), stops, "no stop added");
        update(&input, cx, |input, cx| {
            input.set_queued(None, cx);
            input.set_attachments(Vec::new(), cx);
        });
        assert_eq!(bounds("prompt-area", cx).map(|b| b.top()), Some(px(10.)), "no row");
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
        let continuing = || PromptInputEvent::Action("Continue conversation".into());
        assert_eq!(take(&events), [continuing()]);
        let stop = input.read_with(cx, |input, _| input.action_stop()).expect("a stop");
        cx.update(|window, _| stop.focus(window));
        cx.run_until_parked();
        assert_eq!(
            cx.update(|window, cx| input.read(cx).focused(window, cx)),
            Some(PromptFocus::Action("Continue conversation".into()))
        );
        cx.simulate_keystrokes("enter");
        cx.simulate_keystrokes("space");
        assert_eq!(take(&events), [continuing(), continuing()]);
        assert_eq!(input.read_with(cx, |input, cx| input.text(cx).to_string()), "draft");
        update(&input, cx, |input, cx| input.set_action(None, cx));
        assert!(input.read_with(cx, |input, _| input.action_stop()).is_none());
    }

    /// Several actions show in order, each a stop that asks with its key;
    /// one kept while the others change keeps its handle, and its focus.
    #[test]
    fn several_actions_ask_by_key_and_keep_their_stops() {
        let (_, input, events, cx) = host();
        let action = |key: &'static str, label: &'static str| PromptAction { key: key.into(), label: label.into() };
        update(&input, cx, |input, cx| {
            input.set_status(PromptStatus::Disabled, cx);
            input.set_actions(vec![action("continue", "Continue"), action("list", "List…"), action("new", "New")], cx);
        });
        let first = bounds("prompt-action", cx).expect("the first shows");
        let second = bounds("prompt-action-1", cx).expect("the second shows");
        let third = bounds("prompt-action-2", cx).expect("the third shows");
        assert!(first.right() <= second.left() && second.right() <= third.left(), "in order");
        click("prompt-action-2", cx);
        assert_eq!(take(&events), [PromptInputEvent::Action("new".into())]);
        let stops = input.read_with(cx, |input, _| input.action_stops());
        assert_eq!(stops.len(), 3);
        cx.update(|window, _| stops[1].focus(window));
        cx.run_until_parked();
        update(&input, cx, |input, cx| input.set_actions(vec![action("list", "List…"), action("new", "New")], cx));
        assert_eq!(input.read_with(cx, |input, _| input.action_stops()), stops[1..].to_vec());
        assert_eq!(cx.update(|window, cx| input.read(cx).focused(window, cx)), Some(PromptFocus::Action("list".into())));
        cx.simulate_keystrokes("enter");
        assert_eq!(take(&events), [PromptInputEvent::Action("list".into())]);
    }

    /// A disabled prompt input showing `CONNECTING` in a host of a given
    /// width, and a line of the reason's size to measure one line by.
    struct WidthHost {
        width: f32,
        input: Entity<PromptInput>,
    }

    const CONNECTING: &str = "connecting to the terminal server";

    impl Render for WidthHost {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .flex()
                .flex_col()
                .child(div().debug_selector(|| "one-line".into()).text_size(rems(0.75)).child("x"))
                .child(div().w(px(self.width)).flex().flex_col().child(self.input.clone()))
        }
    }

    fn sized(width: f32) -> (Entity<WidthHost>, &'static mut VisualTestContext) {
        let mut app = test_support::app();
        let (host, cx) = app.add_window_view(move |_, cx| {
            let input = cx.new(|cx| {
                let mut input = PromptInput::new(cx);
                input.set_status(PromptStatus::Disabled, cx);
                input.set_reason(Some(CONNECTING.into()), cx);
                input
            });
            WidthHost { width, input }
        });
        let cx: &'static mut VisualTestContext = Box::leak(Box::new(cx.clone()));
        cx.run_until_parked();
        (host, cx)
    }

    /// The spec's "Reason line in a narrow input": at 240px the reason
    /// wraps at the input's width, holding at least `connecting` (0.6em a
    /// character in gpui's test text system) on its first line, on at most
    /// two lines.
    #[test]
    fn the_reason_wraps_at_a_narrow_inputs_width() {
        let (_, cx) = sized(240.);
        let line = bounds("one-line", cx).expect("drawn");
        let reason = bounds("prompt-reason", cx).expect("drawn");
        assert!(reason.size.width >= px("connecting".len() as f32 * 0.6 * 12.), "{reason:?}");
        assert!(reason.size.height <= line.size.height * 2., "{reason:?} on more than two lines of {line:?}");
    }

    /// The spec's "After the width changes": drawn at 600px and then at
    /// 240px, the reason is laid out as a first draw at 240px lays it out.
    #[test]
    fn the_reason_follows_a_width_change() {
        let (_, fresh) = sized(240.);
        let expected = bounds("prompt-reason", fresh).expect("drawn");
        let (host, cx) = sized(600.);
        let wide = bounds("prompt-reason", cx).expect("drawn");
        assert!(wide.size.width >= expected.size.width, "{wide:?}");
        cx.update(|_, cx| {
            host.update(cx, |host, cx| {
                host.width = 240.;
                cx.notify();
            })
        });
        cx.run_until_parked();
        assert_eq!(bounds("prompt-reason", cx), Some(expected));
    }

    fn suggestions(highlighted: usize) -> Suggestions {
        let row = |key: &'static str, hint: Option<&'static str>| Suggestion {
            key: key.into(),
            label: format!("/{key}").into(),
            hint: hint.map(Into::into),
            description: Some(format!("about {key}").into()),
        };
        Suggestions { rows: vec![row("compact", Some("<how>")), row("context", None), row("review", None)], highlighted }
    }

    /// The spec's "Prompt input suggestions": rows above the text area,
    /// which keeps focus; a press asks the owner by key and changes no
    /// text; taken away, no list shows.
    #[test]
    fn suggestions_show_above_and_a_press_asks_the_owner() {
        let (_, input, events, cx) = host();
        cx.simulate_input("/co");
        update(&input, cx, |input, cx| input.set_suggestions(Some(suggestions(1)), cx));
        let list = bounds("prompt-suggestions", cx).expect("the list shows");
        let area = bounds("prompt-area", cx).expect("drawn");
        assert!(list.bottom() <= area.top(), "{list:?} above {area:?}");
        for ix in 0..3 {
            assert!(cx.debug_bounds(["prompt-suggestion-0", "prompt-suggestion-1", "prompt-suggestion-2"][ix]).is_some());
        }
        click("prompt-suggestion-2", cx);
        assert_eq!(take(&events), [PromptInputEvent::SuggestionPressed("review".into())]);
        assert_eq!(input.read_with(cx, |input, cx| input.text(cx).to_string()), "/co", "the owner's to change");
        let focused = cx.update(|window, cx| input.read(cx).focused(window, cx));
        assert_eq!(focused, Some(PromptFocus::Text), "the text area keeps focus");
        // Typing goes on in the text area.
        cx.simulate_input("m");
        assert_eq!(input.read_with(cx, |input, cx| input.text(cx).to_string()), "/com");
        update(&input, cx, |input, cx| input.set_suggestions(None, cx));
        assert!(input.read_with(cx, |input, _| input.suggestions().is_none()));
        let empty = Suggestions { rows: Vec::new(), highlighted: 0 };
        update(&input, cx, |input, cx| input.set_suggestions(Some(empty), cx));
        assert!(input.read_with(cx, |input, _| input.suggestions().is_none()), "no rows, no list");
    }

    /// While suggestions show, up, down, enter, tab and escape ask the
    /// owner and do nothing else: enter submits nothing, escape doesn't
    /// stop a turn, tab and the arrows leave the text alone. Enter while an
    /// input method composes goes to the text area. Without them, the keys
    /// are as before.
    #[test]
    fn the_suggestions_take_their_keys_first() {
        let (host, input, events, cx) = host();
        cx.simulate_input("/co");
        update(&input, cx, |input, cx| {
            input.set_status(PromptStatus::Streaming, cx);
            input.set_suggestions(Some(suggestions(0)), cx);
        });
        for (key, asked) in [
            ("up", SuggestionKey::Up),
            ("down", SuggestionKey::Down),
            ("enter", SuggestionKey::Enter),
            ("tab", SuggestionKey::Tab),
            ("escape", SuggestionKey::Escape),
        ] {
            cx.simulate_keystrokes(key);
            assert_eq!(take(&events), [PromptInputEvent::SuggestionKey(asked)], "{key}");
        }
        assert_eq!(input.read_with(cx, |input, cx| input.text(cx).to_string()), "/co");
        assert_eq!(host.read_with(cx, |host, _| host.escapes), 0);
        let area = input.read_with(cx, |input, _| input.area().clone());
        cx.update(|window, cx| {
            area.update(cx, |area, cx| area.replace_and_mark_text_in_range(None, "e", None, window, cx));
        });
        cx.simulate_keystrokes("enter");
        assert!(take(&events).is_empty(), "the composition's");
        update(&input, cx, |input, cx| input.set_suggestions(None, cx));
        cx.update(|window, cx| input.read(cx).focus_handle(cx).focus(window));
        cx.simulate_keystrokes("escape");
        assert_eq!(take(&events), [PromptInputEvent::Stop], "escape stops again");
    }

    /// The spec's "Prompt input footer items": the items show under the
    /// text area with a dot between them, a press asks the owner with the
    /// item's key, and none is a stop.
    #[test]
    fn footer_items_ask_the_owner_and_are_no_stops() {
        let (_, input, events, cx) = host();
        let stops = input.read_with(cx, |input, _| (input.row_stops(), input.action_stops()));
        let item = |key: &'static str, label: &'static str| FooterItem { key: key.into(), label: label.into() };
        update(&input, cx, |input, cx| {
            input.set_footer(vec![item("mode", "Auto"), item("model", "Opus"), item("effort", "Medium")], cx)
        });
        let footer = bounds("prompt-footer", cx).expect("the footer shows");
        let area = bounds("prompt-area", cx).expect("drawn");
        assert!(footer.top() >= area.bottom(), "{footer:?} under {area:?}");
        click("prompt-footer-1", cx);
        assert_eq!(take(&events), [PromptInputEvent::Footer("model".into())]);
        let focused = cx.update(|window, cx| input.read(cx).focused(window, cx));
        assert_eq!(focused, Some(PromptFocus::Text), "the press takes no focus");
        assert_eq!(input.read_with(cx, |input, _| (input.row_stops(), input.action_stops())), stops, "no new stops");
        assert_eq!(input.read_with(cx, |input, _| input.footer().len()), 3);
        update(&input, cx, |input, cx| input.set_footer(Vec::new(), cx));
        assert!(input.read_with(cx, |input, _| input.footer().is_empty()));
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
