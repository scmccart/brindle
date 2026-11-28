//! An agent zone's view (`agent-zones`): the conversation in a
//! [`MessageScroller`], the alert for the zone's phase, the reconnecting
//! banner and the [`PromptInput`] with its queue row.
//!
//! It draws what [`Agent`]'s [`ChatModel`] holds and never takes the
//! shared fold's lock (`process R14`); it sends nothing itself, asking the
//! agent, which decides through the model (`fcis R1`).
//!
//! Keys (design.md decision 1): in the `AgentZone` context escape stops the
//! turn and the page keys scroll; the prompt input's and its text area's
//! contexts have enter and shift-enter. Tab walks the zone's stops
//! ([`FocusStops`]): the prompt, the Continue conversation button while an
//! ended agent can be continued, the queue row's buttons, the pending
//! permission requests' buttons, and the stop button while a turn runs.
//! When a stop the focus is on goes away (the row's buttons when the slot
//! changes, the stop button when the turn ends, a permission answered, the
//! Continue button once pressed), focus goes back to the prompt.

use std::ops::Range;
use std::sync::Arc;

use gpui::{
    App, AppContext as _, ClipboardItem, Context, ElementId, Entity, EventEmitter, ExternalPaths, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, Modifiers, ParentElement as _, Pixels, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, WeakEntity, Window, div, prelude::FluentBuilder as _,
};

use brindle_ui::{
    ActiveTheme as _, Alert as UiAlert, AlertVariant, BubblePart, Button, ButtonSize, ButtonVariant, Confirmation, ConfirmationAction,
    ConfirmationState, ConfirmationTone, FocusScope, FocusStops, Icon, ItemKey, Loader, Message, MessageFrom, MessageScroller,
    PageDirection, PromptAction, PromptFocus, PromptInput, PromptInputEvent, PromptStatus as UiStatus, Queued, Reasoning,
    Resend as UiResend, Response, RichBlock, Shimmer, TextAreaEvent, Tool, ToolStatus as UiToolStatus, TooltipFor as _,
};
use brindle_ui::{AttachmentItem, FooterItem, Suggestion, SuggestionKey, Suggestions, TextInputBackspace};

use crate::agent::commands::{Completion, CompletionKey, OptionItem};
use brindle_ui::{Code, CodeBlock, Diff, IconElement};

use crate::actions::{
    AgentAttachFile, AgentCancel, AgentClearAttachments, AgentContinue, AgentConversations, AgentNextMode, AgentOptions, AgentResend, AgentScrollToBottom, AgentSubmit, AgentUnqueue, CopyLastAgentMessage, CopyLastCodeBlock,
    CopyQueuedPrompt, ScrollPageDown, ScrollPageUp, ScrollToBottom, ToggleAgentThoughts, ToggleAgentToolDetails,
};
use crate::agent::login::MethodOffer;
use crate::agent::markdown::{LinkAction, link_action};
use crate::agent::model::{
    self as model, AgentPhase, Attached, Block, BlockBody, Draft, DraftKey, ChatModel, Earlier, LoginRow, MethodChoice, NoOffer, Offer, PromptStatus,
    QueueRow, Resend, ShownPart, TextState, WorkingRow, ZoneChoice,
};
use crate::agent::tool::{self, DetailPart};
use crate::motion::{self, Motion};
use crate::agent::transcript::{
    Attachment, EntryId, NoticeLevel, PermissionKind, PermissionOutcome, ToolKind, ToolStatus,
};
use crate::agent::{Agent, AgentUpdate};
use crate::settings::Settings;
use crate::terminal::links::{self, OpenError, Plan};
use crate::terminal_view::LinkFailed;
use crate::theme::Theme;

/// A link in the conversation asks before it opens (a scheme the terminal
/// opens, other than http and https): the window shows the link
/// confirmation for `uri`, and gives focus back to `back` (the zone's
/// prompt input) when it closes.
#[derive(Clone, Debug)]
pub struct ConfirmLink {
    pub uri: String,
    pub back: FocusHandle,
}

/// What a press released on a link with `modifiers` does: nothing without
/// the terminal's link gesture ([`links::wants_links`]), else what
/// [`link_action`] says of `target` with its `plan`.
pub fn link_press(target: &str, modifiers: &Modifiers, plan: impl FnOnce(&str) -> Plan) -> Option<LinkAction> {
    links::wants_links(modifiers).then(|| link_action(target, plan(target)))
}

/// The link gesture's new state when `modifiers` change it from `was`
/// ([`links::wants_links`]: ctrl held, alt not), else `None`: only a flip
/// changes what a link's pointer shows (OpenSpec change
/// `agents-zone-focus-and-scroll` decision 6).
pub fn link_gesture_flip(was: bool, modifiers: &Modifiers) -> Option<bool> {
    let now = links::wants_links(modifiers);
    (now != was).then_some(now)
}

/// Whether a focused permission button gives focus to the prompt after a
/// frame whose items in view are `in_view`: its block (`None` when the
/// request is no longer pending) isn't among them (decision 4).
pub fn gives_back(block: Option<usize>, in_view: &Range<usize>) -> bool {
    block.is_none_or(|block| !in_view.contains(&block))
}

/// What an agent zone does with a link pressed with the gesture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkStep {
    /// Hand it to the opener now.
    Open,
    /// Ask first: the window's link confirmation.
    Confirm,
    /// Tell the window why not, on its config line.
    Fail(String),
}

/// The step for `action` on `target`: a refusal is the terminal's
/// failure text (`railway R8`: the action, then the cause).
pub fn link_step(target: &str, action: LinkAction) -> LinkStep {
    match action {
        LinkAction::Open => LinkStep::Open,
        LinkAction::Confirm => LinkStep::Confirm,
        LinkAction::Refuse(refusal) => LinkStep::Fail(links::failure_text(target, &OpenError::Refused(refusal))),
    }
}

/// The list item's padding above and below a block: a message's parts
/// draw one bubble, so only its outer edges are padded; every other block
/// is padded on both.
pub fn item_padding(block: &Block) -> (bool, bool) {
    match &block.body {
        BlockBody::UserText { .. } | BlockBody::AgentPart { .. } | BlockBody::Attached { .. } => {
            let edges = bubble_part(block.part, block.last).edges();
            (edges.top, edges.bottom)
        }
        // A tool row and its open detail read as one: padded above its
        // line and below its last part.
        BlockBody::Tool { .. } | BlockBody::ToolDetail(_) => (block.part == 0, block.last),
        BlockBody::Thought { .. } | BlockBody::Permission { .. } | BlockBody::Notice { .. } => (true, true),
    }
}

/// The Continue conversation button's label.
#[cfg(test)]
pub const CONTINUE: &str = "Continue conversation";

/// The login row's Cancel login button's label.
pub const CANCEL_LOGIN: &str = "Cancel login";

/// The login row's title (OpenSpec change `agents-login`, decision 6).
pub fn login_title(row: &LoginRow) -> Option<SharedString> {
    match row {
        LoginRow::None => None,
        LoginRow::Choose { .. } => Some("Log in to continue".into()),
        LoginRow::Running { name, .. } => Some(format!("Logging in with {name}…").into()),
    }
}

/// What a method's button says under its name: the agent's description
/// as plain text (backticks kept), then what a terminal login runs and
/// sets, or why it is withheld.
pub fn method_description(choice: &MethodChoice) -> Option<SharedString> {
    let extra = match &choice.offer {
        MethodOffer::Agent => None,
        MethodOffer::Terminal(plan) => Some(plan.shown()).filter(|shown| !shown.is_empty()),
        MethodOffer::Withheld(withheld) => Some(withheld.why.clone()),
    };
    let parts: Vec<&str> = choice.description.as_ref().map(|d| d.as_ref()).into_iter().chain(extra.as_deref()).collect();
    (!parts.is_empty()).then(|| parts.join(" · ").into())
}

/// What the prompt input shows: its status and queue row, and its reason
/// line, `reason` then the `actions`' buttons (OpenSpec change
/// `agents-session-list` decision 6).
#[derive(Debug, Clone, PartialEq)]
pub struct PromptState {
    pub status: UiStatus,
    pub queued: Option<Queued>,
    pub reason: Option<SharedString>,
    pub actions: Vec<ZoneChoice>,
}

impl PromptState {
    /// The first button's label: Continue conversation when it shows.
    #[cfg(test)]
    pub fn action(&self) -> Option<SharedString> {
        self.actions.first().map(|choice| choice.label().into())
    }
}

/// The reason line's buttons for `choices`, each keyed by its stop's name.
pub fn prompt_actions(choices: &[ZoneChoice]) -> Vec<PromptAction> {
    choices.iter().map(|choice| PromptAction { key: choice.word().into(), label: choice.label().into() }).collect()
}

/// What the prompt input shows for the model and its continue `offer`
/// ([`Agent::offer`]). The reason is why it is disabled, else the last
/// refusal; an ended agent that can be continued shows the Continue
/// conversation button after the reason, or after a refused continue's
/// words; one that can't says why not instead (a never-started agent
/// keeps its own reason).
pub fn prompt_state(model: &ChatModel, offer: &Offer) -> PromptState {
    let (status, reason) = match model.prompt_status() {
        PromptStatus::Ready => (UiStatus::Ready, None),
        PromptStatus::Submitted => (UiStatus::Submitted, None),
        PromptStatus::Streaming => (UiStatus::Streaming, None),
        PromptStatus::Disabled(why) => (UiStatus::Disabled, Some(SharedString::from(why.text()))),
    };
    let refusal = model.refusal().map(|refusal| SharedString::from(refusal.to_string()));
    let reason = match offer {
        Offer::Restart | Offer::Respawn { .. } => refusal.or(reason),
        // A choosing agent's reason is the screen's (decision 6).
        Offer::None(NoOffer::NotEnded) if model.is_choosing() => {
            refusal.or_else(|| Some(SharedString::from(model.choosing_reason())))
        }
        Offer::None(NoOffer::NotEnded) => reason.or(refusal),
        Offer::None(NoOffer::NoConversation) if matches!(model.phase(), AgentPhase::NotStarted(_)) => reason.or(refusal),
        Offer::None(why @ (NoOffer::NoConversation | NoOffer::NotOffered | NoOffer::ProfileGone { .. })) => {
            Some(SharedString::from(why.text()))
        }
    };
    PromptState { status, queued: queued_of(model.queue()), reason, actions: model.zone_choices(offer) }
}

/// The prompt input's suggestions for the model's completion list (OpenSpec
/// change `agents-commands`), each row keyed by its label.
pub fn suggestions_of(completion: &Completion) -> Suggestions {
    let rows = completion
        .items
        .iter()
        .map(|item| Suggestion {
            key: item.label.clone().into(),
            label: item.label.clone().into(),
            hint: item.hint.clone().map(Into::into),
            description: item.description.clone().map(Into::into),
        })
        .collect();
    Suggestions { rows, highlighted: completion.highlighted }
}

/// The prompt input's attachment items for the draft (OpenSpec change
/// `agents-prompt-context`), each keyed by its [`DraftKey`]: a link by its
/// name with its path as the detail, a text with its size.
pub fn attachment_items(draft: &[Attached]) -> Vec<AttachmentItem> {
    draft
        .iter()
        .map(|Attached { key, draft }| {
            let icon = match draft {
                Draft::Link { .. } => Icon::FileText,
                Draft::Text { .. } => Icon::Terminal,
            };
            AttachmentItem {
                key: key.0.to_string().into(),
                icon,
                name: draft.name().to_string().into(),
                detail: Some(draft.detail().into()),
            }
        })
        .collect()
}

/// The draft key a remove mark's item key names.
pub fn draft_key_of(key: &str) -> Option<DraftKey> {
    key.parse().ok().map(DraftKey)
}

/// The prompt input's footer for the options under it, each keyed by its
/// option's id.
pub fn footer_of(items: &[OptionItem]) -> Vec<FooterItem> {
    items.iter().map(|item| FooterItem { key: item.id.clone().into(), label: item.label.clone().into() }).collect()
}

/// The completion list's key for the prompt input's.
pub fn completion_key_of(key: SuggestionKey) -> CompletionKey {
    match key {
        SuggestionKey::Up => CompletionKey::Up,
        SuggestionKey::Down => CompletionKey::Down,
        SuggestionKey::Enter => CompletionKey::Enter,
        SuggestionKey::Tab => CompletionKey::Tab,
        SuggestionKey::Escape => CompletionKey::Escape,
    }
}

/// A queued prompt's text as its row shows it, its attachments counted
/// after it (`next · 2 attachments`).
fn counted(text: &SharedString, attachments: usize) -> SharedString {
    match model::attachment_count(attachments) {
        Some(count) => format!("{text} · {count}").into(),
        None => text.clone(),
    }
}

/// The prompt input's queue row for the model's [`QueueRow`].
pub fn queued_of(row: &QueueRow) -> Option<Queued> {
    match row {
        QueueRow::None => None,
        QueueRow::Waiting { text, attachments } => Some(Queued::Waiting { text: counted(text, *attachments) }),
        QueueRow::Held { text, attachments, why, resend, reason: _ } => Some(Queued::Held {
            text: counted(text, *attachments),
            reason: why.clone(),
            resend: match resend {
                Resend::Available => UiResend::Available,
                Resend::Unavailable(why) => UiResend::Unavailable(why.clone()),
            },
        }),
    }
}

/// How a block of a message split into several list items draws its
/// bubble: whole, or the first, a middle or the last part.
pub fn bubble_part(part: usize, last: bool) -> BubblePart {
    match (part == 0, last) {
        (true, true) => BubblePart::Whole,
        (true, false) => BubblePart::First,
        (false, false) => BubblePart::Middle,
        (false, true) => BubblePart::Last,
    }
}

/// The icon a tool call's row shows for its kind.
pub fn tool_icon(kind: ToolKind) -> Icon {
    match kind {
        ToolKind::Read => Icon::FileText,
        ToolKind::Edit => Icon::Pencil,
        ToolKind::Delete => Icon::Trash,
        ToolKind::Move => Icon::Move,
        ToolKind::Search => Icon::Search,
        ToolKind::Execute => Icon::Terminal,
        ToolKind::Think => Icon::Brain,
        ToolKind::Fetch => Icon::Globe,
        ToolKind::SwitchMode | ToolKind::Other => Icon::Wrench,
    }
}

pub fn tool_status(status: ToolStatus) -> UiToolStatus {
    match status {
        ToolStatus::Pending => UiToolStatus::Pending,
        ToolStatus::InProgress => UiToolStatus::Running,
        ToolStatus::Completed => UiToolStatus::Done,
        ToolStatus::Failed => UiToolStatus::Failed,
    }
}

fn allows(kind: PermissionKind) -> bool {
    match kind {
        PermissionKind::AllowOnce | PermissionKind::AllowAlways => true,
        PermissionKind::RejectOnce | PermissionKind::RejectAlways => false,
    }
}

/// A permission option's button: an allow option is destructive while
/// the request is pending, a reject option an outline (decision 4).
pub fn permission_variant(kind: PermissionKind) -> ButtonVariant {
    if allows(kind) { ButtonVariant::Destructive } else { ButtonVariant::Outline }
}

/// What a conversation item's height depends on besides its text and the
/// width (which the list follows itself): the text size and the two
/// families (OpenSpec change `agents-zone-focus-and-scroll` decision 3).
/// The zone doesn't read the terminal's line height; a change that makes
/// it do so adds it here.
#[derive(Clone, Debug, PartialEq)]
pub struct Measure {
    pub size: Pixels,
    pub ui_family: SharedString,
    pub code_family: SharedString,
}

/// The measure in effect: the profile's size, else the terminal size in
/// effect (zoom included), and the UI and code families.
fn measure_of(font_size_override: Option<f32>, cx: &App) -> Measure {
    let settings = Settings::get(cx);
    Measure {
        size: settings.font_size(font_size_override),
        ui_family: settings.ui_font_family(),
        code_family: settings.font().family,
    }
}

/// An agent zone's view.
pub struct AgentView {
    agent: Entity<Agent>,
    scroller: Entity<MessageScroller>,
    prompt: Entity<PromptInput>,
    /// The pending permission requests' buttons, in the conversation's
    /// order, keyed by entry and option (`types R3`).
    permissions: Vec<((EntryId, SharedString), FocusHandle)>,
    /// How many times `refresh` rebuilt [`Self::permissions`] (OpenSpec
    /// change `agents-absorb-cost`, task 8.1).
    #[cfg(test)]
    permission_rebuilds: usize,
    /// The login row as the model last gave it.
    login: LoginRow,
    /// The login row's enabled method buttons, keyed by method id
    /// (`types R3`).
    logins: Vec<(SharedString, FocusHandle)>,
    /// Cancel login's handle, a stop while a login runs.
    cancel_login: FocusHandle,
    theme: Theme,
    /// The profile's font size, else the terminal size in effect.
    font_size_override: Option<f32>,
    /// The Earlier row as the model last gave it (OpenSpec change
    /// `agents-log-paging` decision 11): the scroller's top row while there
    /// is one; the view wants the motion clock while it loads.
    earlier: Earlier,
    /// What the items were last measured at; `remeasure_if_changed`, from
    /// `set_profile` and the Settings observer, refreshes it.
    measure: Measure,
    /// Whether the modifiers held last made the link gesture.
    link_gesture: bool,
    _subscriptions: Vec<Subscription>,
}

impl EventEmitter<LinkFailed> for AgentView {}
impl EventEmitter<ConfirmLink> for AgentView {}

impl Focusable for AgentView {
    /// The prompt input's: focusing the zone puts the caret in the prompt.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.prompt.read(cx).focus_handle(cx)
    }
}

impl FocusStops for AgentView {
    /// The prompt, the Continue conversation button while it shows, the
    /// queue row's buttons, the login row's method buttons or its Cancel
    /// login, the pending permissions' buttons, then the stop button while
    /// a turn runs.
    fn stops(&self, cx: &App) -> Vec<FocusHandle> {
        let prompt = self.prompt.read(cx);
        let mut stops = vec![prompt.focus_handle(cx)];
        stops.extend(prompt.action_stops());
        stops.extend(prompt.row_stops());
        stops.extend(self.login_stops());
        stops.extend(self.permissions.iter().map(|(_, handle)| handle.clone()));
        stops.extend(prompt.stop_button());
        stops
    }
}

impl AgentView {
    pub fn new(
        agent: Entity<Agent>,
        theme: Theme,
        font_size_override: Option<f32>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let prompt = cx.new(PromptInput::new);
        prompt.update(cx, |prompt, cx| {
            prompt.set_placeholder("Message the agent", cx);
            prompt.set_held_color(theme.ansi[3].hsla(), cx);
        });
        let count = agent.read(cx).model().item_count();
        let view = cx.weak_entity();
        let (top, told, gave) = (view.clone(), view.clone(), view.clone());
        let window_handle = window.window_handle();
        let scroller = cx.new(|cx| {
            MessageScroller::new(count, move |ix, _, cx| render_block(&view, ix, cx), cx)
                .with_top_row(move |_, cx| earlier_row(&top, cx))
                // The row came into view, or went: a fetch may be due.
                .on_top_row_view(move |in_view, cx| {
                    told.update(cx, |this, cx| {
                        this.agent.update(cx, |agent, cx| agent.fetch_if_due(in_view, !in_view, cx));
                    })
                    .ok();
                })
                // A focused permission's block may have left the view.
                .on_items_view(move |in_view, cx| {
                    window_handle
                        .update(cx, |_, window, cx| gave.update(cx, |this, cx| this.give_back(in_view, window, cx)))
                        .ok();
                })
        });
        let area = prompt.read(cx).area().clone();
        let subscriptions = vec![
            cx.subscribe_in(&agent, window, |this, _, update: &AgentUpdate, _, cx| this.on_update(update, cx)),
            cx.observe_in(&agent, window, |this, _, window, cx| this.sync(window, cx)),
            // The continue offer reads the config's profiles: an edit can
            // take the button away or bring it back. A zoom, a font
            // preview or a reload may change what the items measure at.
            cx.observe_global_in::<Settings>(window, |this, window, cx| {
                this.remeasure_if_changed(cx);
                this.sync(window, cx);
            }),
            cx.subscribe(&prompt, |this, _, event: &PromptInputEvent, cx| this.on_prompt(event, cx)),
            cx.subscribe(&area, |this, _, _: &TextAreaEvent, cx| {
                this.agent.update(cx, |agent, cx| agent.seen(cx));
            }),
            // The completion list follows the text and the cursor (OpenSpec
            // change `agents-commands`): any change of the area's.
            cx.observe(&area, |this, area, cx| {
                let area = area.read(cx);
                let (text, cursor, composing) = (area.text().to_string(), area.cursor(), area.composing());
                this.agent.update(cx, |agent, cx| agent.typed(&text, cursor, composing, cx));
            }),
            // A closed zone can't keep the clock running.
            cx.on_release({
                let id = cx.entity_id();
                move |_, cx| Motion::want(id, false, cx)
            }),
        ];
        let mut this = Self {
            agent,
            scroller,
            prompt,
            permissions: Vec::new(),
            #[cfg(test)]
            permission_rebuilds: 0,
            login: LoginRow::None,
            logins: Vec::new(),
            cancel_login: cx.focus_handle(),
            theme,
            font_size_override,
            earlier: Earlier::None,
            measure: measure_of(font_size_override, cx),
            link_gesture: false,
            _subscriptions: subscriptions,
        };
        this.refresh(cx);
        this.follow_earlier(cx);
        this.refresh_motion(cx);
        this
    }

    /// Wants the motion clock while the working row shows or the Earlier
    /// row loads.
    fn refresh_motion(&self, cx: &mut Context<Self>) {
        let loading = match self.earlier {
            Earlier::Loading { .. } => true,
            Earlier::None | Earlier::Available { .. } | Earlier::Failed { .. } => false,
        };
        let working = self.agent.read(cx).model().working_row().is_some();
        Motion::want(cx.entity_id(), working || loading, cx);
    }

    /// The model's Earlier row: the scroller shows it while there is one
    /// and draws it again when it changes; while it shows in view, a fetch
    /// may be due (a page that left it in view, a reconnect's replay).
    fn follow_earlier(&mut self, cx: &mut Context<Self>) {
        let earlier = self.agent.read(cx).model().earlier().clone();
        if earlier != self.earlier {
            let shown = earlier != Earlier::None;
            self.scroller.update(cx, |scroller, cx| {
                scroller.show_top_row(shown, cx);
                scroller.refresh_top_row(cx);
            });
            self.earlier = earlier;
            self.refresh_motion(cx);
        }
        if self.scroller.read(cx).top_row_in_view() {
            self.agent.update(cx, |agent, cx| agent.fetch_if_due(true, true, cx));
        }
    }

    pub fn agent(&self) -> &Entity<Agent> {
        &self.agent
    }

    pub fn prompt(&self) -> &Entity<PromptInput> {
        &self.prompt
    }

    /// The zone's stop with keyboard focus, for the dump's focus line
    /// (`agent stop=…`): `prompt`, `continue`, `queued:remove`,
    /// `queued:send-again`, `queued:discard`, `permission:<entry>:<option>`
    /// or `stop`.
    pub fn focused_stop(&self, window: &Window, cx: &App) -> Option<String> {
        if let Some(stop) = self.prompt.read(cx).focused(window, cx) {
            return Some(match stop {
                PromptFocus::Text => "prompt".into(),
                PromptFocus::Remove => "queued:remove".into(),
                PromptFocus::SendAgain => "queued:send-again".into(),
                PromptFocus::Discard => "queued:discard".into(),
                PromptFocus::Stop => "stop".into(),
                // Keyed by the choice's word: `continue`, `conversations`,
                // `new`.
                PromptFocus::Action(key) => key.to_string(),
            });
        }
        if let Some((id, _)) = self.logins.iter().find(|(_, handle)| handle.is_focused(window)) {
            return Some(format!("login:{id}"));
        }
        if self.cancel_login.is_focused(window) {
            return Some("login:cancel".into());
        }
        self.permissions
            .iter()
            .find(|(_, handle)| handle.is_focused(window))
            .map(|((entry, option), _)| format!("permission:{}:{option}", entry.0))
    }

    /// The login row's stops: its enabled method buttons in order, or
    /// Cancel login while a login runs.
    fn login_stops(&self) -> Vec<FocusHandle> {
        match &self.login {
            LoginRow::None => Vec::new(),
            LoginRow::Choose { .. } => self.logins.iter().map(|(_, handle)| handle.clone()).collect(),
            LoginRow::Running { .. } => vec![self.cancel_login.clone()],
        }
    }

    /// The login row's method `id`'s focus handle, while it is a stop.
    #[cfg(test)]
    pub fn login_handle(&self, id: &str) -> Option<&FocusHandle> {
        self.logins.iter().find(|(own, _)| own.as_ref() == id).map(|(_, handle)| handle)
    }

    /// Cancel login's handle.
    #[cfg(test)]
    pub fn cancel_login_handle(&self) -> &FocusHandle {
        &self.cancel_login
    }

    pub fn scroller(&self) -> &Entity<MessageScroller> {
        &self.scroller
    }

    /// Takes the tab's profile's theme (the notices' red and the held
    /// row's warning colour, the ANSI red and yellow, as there are no
    /// warning tokens) and font size (`None`: the terminal size in effect),
    /// as the config now has them or the settings dialog previews them.
    /// Everything else the zone draws, its root's background and text
    /// included, takes the theme scope's tokens it is drawn in: the
    /// theme's zone tokens, which `ZoneTreeView` keeps and wraps the zone
    /// in.
    pub fn set_profile(&mut self, theme: Theme, font_size_override: Option<f32>, cx: &mut Context<Self>) {
        let held = theme.ansi[3].hsla();
        self.prompt.update(cx, |prompt, cx| prompt.set_held_color(held, cx));
        self.theme = theme;
        self.font_size_override = font_size_override;
        // A theme changes colours only; a size may change the heights.
        self.remeasure_if_changed(cx);
        cx.notify();
    }

    /// Measures every item again, keeping the view's place, when what they
    /// measure at changed; otherwise nothing (decision 3).
    fn remeasure_if_changed(&mut self, cx: &mut Context<Self>) {
        let measure = measure_of(self.font_size_override, cx);
        if measure != self.measure {
            self.measure = measure;
            self.scroller.update(cx, |scroller, cx| scroller.remeasure_all(cx));
        }
    }

    /// The size the conversation and the prompt are drawn at: the
    /// profile's font size, else the terminal size in effect (zoom
    /// included).
    pub fn text_size(&self, cx: &App) -> Pixels {
        Settings::get(cx).font_size(self.font_size_override)
    }

    fn on_update(&mut self, update: &AgentUpdate, cx: &mut Context<Self>) {
        match update {
            AgentUpdate::Splices(splices) => self.scroller.update(cx, |scroller, cx| {
                for splice in splices {
                    scroller.splice(splice.range.clone(), splice.count, cx);
                }
            }),
            AgentUpdate::ClearDraft => self.prompt.update(cx, |prompt, cx| prompt.set_text("", cx)),
            AgentUpdate::SetDraft(text) => self.prompt.update(cx, |prompt, cx| prompt.set_text(text, cx)),
            AgentUpdate::SetDraftAt(text, cursor) => {
                self.prompt.update(cx, |prompt, cx| prompt.set_text_at(text, *cursor, cx));
            }
            AgentUpdate::SendFirstIfUnchanged(text) => {
                if self.prompt.read(cx).area().read(cx).text() == text.as_str() {
                    self.agent.update(cx, |agent, cx| agent.send_first(text, cx));
                }
            }
        }
    }

    fn on_prompt(&mut self, event: &PromptInputEvent, cx: &mut Context<Self>) {
        match event {
            PromptInputEvent::Submit(text) => {
                let composing = self.prompt.read(cx).area().read(cx).composing();
                let text = text.clone();
                self.agent.update(cx, |agent, cx| agent.prompt(&text, composing, cx));
            }
            PromptInputEvent::Stop => self.agent.update(cx, |agent, cx| agent.cancel(cx)),
            PromptInputEvent::RemoveQueued => self.agent.update(cx, |agent, cx| agent.unqueue(cx)),
            PromptInputEvent::ResendQueued => self.agent.update(cx, |agent, cx| agent.resend(cx)),
            PromptInputEvent::SuggestionKey(key) => {
                let key = completion_key_of(*key);
                self.agent.update(cx, |agent, cx| agent.completion(Some(key), None, cx));
            }
            PromptInputEvent::SuggestionPressed(label) => {
                let label = label.to_string();
                self.agent.update(cx, |agent, cx| agent.completion(None, Some(&label), cx));
            }
            PromptInputEvent::Footer(id) => {
                let id = id.to_string();
                self.agent.update(cx, |agent, cx| agent.footer_pressed(&id, cx));
            }
            PromptInputEvent::RemoveAttachment(key) => {
                if let Some(key) = draft_key_of(key) {
                    self.agent.update(cx, |agent, cx| agent.remove_attachment(key, cx));
                }
            }
            PromptInputEvent::Action(key) => {
                let choice = [ZoneChoice::Continue, ZoneChoice::Conversations, ZoneChoice::New]
                    .into_iter()
                    .find(|choice| choice.word() == key.as_ref());
                self.agent.update(cx, |agent, cx| match choice {
                    Some(ZoneChoice::Continue) => agent.continue_conversation(cx),
                    Some(ZoneChoice::Conversations) => agent.ask_conversations(cx),
                    Some(ZoneChoice::New) => agent.new_conversation(cx),
                    None => {}
                });
            }
        }
    }

    /// The agent changed: the prompt input and the permission stops follow
    /// the model, and a stop that had focus and went away hands it back to
    /// the prompt.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let before = self.stops(cx);
        self.refresh(cx);
        self.follow_earlier(cx);
        // The working row came or went.
        self.refresh_motion(cx);
        let after = self.stops(cx);
        let focused = window.focused(cx);
        if focused.as_ref().is_some_and(|focused| before.contains(focused) && !after.contains(focused)) {
            let prompt = self.prompt.read(cx).focus_handle(cx);
            window.focus(&prompt);
        }
        cx.notify();
    }

    fn refresh(&mut self, cx: &mut Context<Self>) {
        let agent = self.agent.read(cx);
        let state = prompt_state(agent.model(), &agent.offer(cx));
        let pending = agent.model().pending_permissions();
        let login = agent.model().login_row();
        let mut kept = std::mem::take(&mut self.logins);
        if let LoginRow::Choose { methods, .. } = &login {
            for choice in methods.iter().filter(|choice| choice.offer.usable()) {
                let handle = match kept.iter().position(|(own, _)| *own == choice.id) {
                    Some(at) => kept.swap_remove(at).1,
                    None => cx.focus_handle(),
                };
                self.logins.push((choice.id.clone(), handle));
            }
        }
        self.login = login;
        let suggestions = agent.model().completion().map(suggestions_of);
        let footer = footer_of(&agent.model().option_items());
        let attachments = attachment_items(agent.model().attachments());
        self.prompt.update(cx, |prompt, cx| {
            prompt.set_status(state.status, cx);
            prompt.set_queued(state.queued, cx);
            prompt.set_attachments(attachments, cx);
            prompt.set_reason(state.reason, cx);
            prompt.set_actions(prompt_actions(&state.actions), cx);
            prompt.set_suggestions(suggestions, cx);
            prompt.set_footer(footer, cx);
        });
        // The set (16 requests at most) as the handles were built from:
        // kept while it is the same, as while a message streams.
        let keys = pending.iter().flat_map(|(entry, options)| options.iter().map(move |option| (*entry, option)));
        if keys.eq(self.permissions.iter().map(|((entry, option), _)| (*entry, option))) {
            return;
        }
        #[cfg(test)]
        {
            self.permission_rebuilds += 1;
        }
        let mut kept = std::mem::take(&mut self.permissions);
        for (entry, options) in pending {
            for option in options {
                let key = (entry, option);
                let handle = match kept.iter().position(|(own, _)| *own == key) {
                    Some(at) => kept.swap_remove(at).1,
                    None => cx.focus_handle(),
                };
                self.permissions.push((key, handle));
            }
        }
    }

    /// The pending request whose button `handle` is, if it is one.
    fn permission_of(&self, handle: &FocusHandle) -> Option<EntryId> {
        self.permissions.iter().find(|(_, own)| own == handle).map(|((entry, _), _)| *entry)
    }

    /// Brings the block of the permission button Tab just focused into
    /// view; any other stop is in the prompt area, always shown.
    fn reveal_permission(&mut self, handle: &FocusHandle, cx: &mut Context<Self>) {
        let Some(entry) = self.permission_of(handle) else { return };
        if let Some(block) = self.agent.read(cx).model().permission_block(entry) {
            self.scroller.update(cx, |scroller, cx| scroller.reveal(block, cx));
        }
    }

    /// A frame changed the items in view to `in_view`: a focused permission
    /// button whose block left the view gives focus to the prompt, as a
    /// stop that left the model does (`sync`). Nothing is answered.
    fn give_back(&mut self, in_view: Range<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(focused) = window.focused(cx) else { return };
        let Some(entry) = self.permission_of(&focused) else { return };
        let block = self.agent.read(cx).model().permission_block(entry);
        if gives_back(block, &in_view) {
            let prompt = self.prompt.read(cx).focus_handle(cx);
            window.focus(&prompt);
        }
    }

    /// The focus handle of permission request `entry`'s button `option`,
    /// while it is pending.
    #[cfg(test)]
    pub fn permission_handle(&self, entry: EntryId, option: &str) -> Option<&FocusHandle> {
        self.permissions.iter().find(|((own, id), _)| *own == entry && id == option).map(|(_, handle)| handle)
    }

    fn submit(&mut self, _: &AgentSubmit, _: &mut Window, cx: &mut Context<Self>) {
        let area = self.prompt.read(cx).area().read(cx);
        let (text, composing) = (area.text().to_string(), area.composing());
        self.agent.update(cx, |agent, cx| agent.prompt(&text, composing, cx));
    }

    fn cancel(&mut self, _: &AgentCancel, _: &mut Window, cx: &mut Context<Self>) {
        self.agent.update(cx, |agent, cx| agent.cancel(cx));
    }

    fn unqueue(&mut self, _: &AgentUnqueue, _: &mut Window, cx: &mut Context<Self>) {
        self.agent.update(cx, |agent, cx| agent.unqueue(cx));
    }

    fn resend(&mut self, _: &AgentResend, _: &mut Window, cx: &mut Context<Self>) {
        self.agent.update(cx, |agent, cx| agent.resend(cx));
    }

    /// `agent_continue`: as the Continue conversation button, nothing when
    /// it isn't offered.
    fn continue_conversation(&mut self, _: &AgentContinue, _: &mut Window, cx: &mut Context<Self>) {
        self.agent.update(cx, |agent, cx| agent.continue_conversation(cx));
    }

    /// `agent_conversations`: as the Conversations… button, for the zone's
    /// profile and directory.
    fn conversations(&mut self, _: &AgentConversations, _: &mut Window, cx: &mut Context<Self>) {
        self.agent.update(cx, |agent, cx| agent.ask_conversations(cx));
    }

    /// `agent_options`: the window's options palette on the zone.
    fn options(&mut self, _: &AgentOptions, _: &mut Window, cx: &mut Context<Self>) {
        self.agent.update(cx, |agent, cx| agent.ask_options(None, cx));
    }

    /// `agent_next_mode`: the mode after the current one.
    fn next_mode(&mut self, _: &AgentNextMode, _: &mut Window, cx: &mut Context<Self>) {
        self.agent.update(cx, |agent, cx| agent.next_mode(cx));
    }

    /// `agent_attach_file`: the window asks for a path.
    fn attach_file(&mut self, _: &AgentAttachFile, _: &mut Window, cx: &mut Context<Self>) {
        self.agent.update(cx, |agent, cx| agent.ask_attach_file(cx));
    }

    /// `agent_clear_attachments`.
    fn clear_attachments(&mut self, _: &AgentClearAttachments, _: &mut Window, cx: &mut Context<Self>) {
        self.agent.update(cx, |agent, cx| agent.clear_attachments(cx));
    }

    /// Backspace in the prompt input, before the text area takes it (a
    /// capture listener): at the very start of the text with nothing
    /// selected it removes the last attachment, and the text area never
    /// sees the key; otherwise it goes on to the text area.
    fn backspace(&mut self, _: &TextInputBackspace, window: &mut Window, cx: &mut Context<Self>) {
        let area = self.prompt.read(cx).area().read(cx);
        if !area.is_focused(window) {
            return;
        }
        let selection = area.selection();
        let (at_start, selected) = (selection.end == 0, !selection.is_empty());
        if self.agent.update(cx, |agent, cx| agent.backspace_at_start(at_start, selected, cx)) {
            cx.stop_propagation();
        }
    }

    fn copy_queued(&mut self, _: &CopyQueuedPrompt, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.agent.read(cx).model().queued_text().map(str::to_string) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn copy_last(&mut self, _: &CopyLastAgentMessage, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.agent.read(cx).model().last_agent_message().map(str::to_string) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    /// `copy_last_code_block`: the clipboard is left alone when no agent
    /// message has a code block.
    fn copy_last_code(&mut self, _: &CopyLastCodeBlock, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.agent.read(cx).model().last_code_block().map(str::to_string) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    /// A press released on a link in the conversation, with `modifiers`:
    /// with the link gesture it opens, asks first or says why not
    /// ([`link_press`], [`link_step`]); without it nothing happens.
    pub fn press_link(&mut self, target: &str, modifiers: Modifiers, cx: &mut Context<Self>) {
        let Some(action) = link_press(target, &modifiers, links::plan_here) else { return };
        match link_step(target, action) {
            LinkStep::Open => {
                let failed = links::open_reporting(target);
                cx.spawn(async move |this, cx| {
                    if let Some(text) = failed.await {
                        this.update(cx, |_, cx| cx.emit(LinkFailed(text))).ok();
                    }
                })
                .detach();
            }
            LinkStep::Confirm => {
                let back = self.prompt.read(cx).focus_handle(cx);
                cx.emit(ConfirmLink { uri: target.to_string(), back });
            }
            LinkStep::Fail(text) => {
                log::warn!("{text}");
                cx.emit(LinkFailed(text));
            }
        }
    }

    fn toggle_thoughts(&mut self, _: &ToggleAgentThoughts, _: &mut Window, cx: &mut Context<Self>) {
        self.agent.update(cx, |agent, cx| agent.toggle_thoughts(cx));
    }

    fn toggle_tool_details(&mut self, _: &ToggleAgentToolDetails, _: &mut Window, cx: &mut Context<Self>) {
        self.agent.update(cx, |agent, cx| agent.toggle_tool_details(cx));
    }

    fn scroll_to_end(&mut self, cx: &mut Context<Self>) {
        self.scroller.update(cx, |scroller, cx| scroller.scroll_to_end(cx));
    }

    fn page(&mut self, dir: PageDirection, cx: &mut Context<Self>) {
        self.scroller.update(cx, |scroller, cx| scroller.scroll_page(dir, cx));
    }

    /// The login row (decision 6): `brindle-ui`'s confirmation, one plain
    /// action per method in the agent's order with its description,
    /// withheld ones disabled, none focused; Cancel login while one runs.
    /// A note (a refused login, a login program's exit) under the title.
    fn login_row(&self, cx: &App) -> Option<impl IntoElement + use<>> {
        let title = login_title(&self.login)?;
        let note = self.agent.read(cx).model().login_note().map(str::to_string);
        let error = self.theme.ansi[1].hsla();
        let mut row = Confirmation::new("agent-login", title, ConfirmationState::Requested);
        match &self.login {
            LoginRow::None => return None,
            LoginRow::Choose { methods, .. } => {
                for choice in methods {
                    let mut action = ConfirmationAction::new(choice.name.clone())
                        .key(ItemKey::new(choice.id.clone()))
                        .disabled(!choice.offer.usable());
                    if let Some(description) = method_description(choice) {
                        action = action.description(description);
                    }
                    if let Some((_, handle)) = self.logins.iter().find(|(id, _)| *id == choice.id) {
                        action = action.focus(handle);
                    }
                    row = row.action(action);
                }
                let agent = self.agent.downgrade();
                row = row.on_action(move |key: &ItemKey, _, cx| {
                    let method = key.as_str().to_string();
                    agent.update(cx, |agent, cx| agent.choose_login(&method, cx)).ok();
                });
            }
            LoginRow::Running { .. } => {
                row = row.action(ConfirmationAction::new(CANCEL_LOGIN).key("cancel").focus(&self.cancel_login));
                let agent = self.agent.downgrade();
                row = row.on_action(move |_: &ItemKey, _, cx| {
                    agent.update(cx, |agent, cx| agent.cancel_login(cx)).ok();
                });
            }
        }
        let row = row.children(note.map(|note| {
            div().debug_selector(|| "agent-login-note".into()).text_color(error).whitespace_normal().child(note)
        }));
        Some(div().debug_selector(|| "agent-login".into()).child(row))
    }

    /// The alert for the zone's phase: its text, the agent's own words in
    /// a scrollable block, and a command on a monospace line of its own (a
    /// Windows batch file's `cmd /c …` form).
    fn alert(&self, cx: &App) -> Option<impl IntoElement + use<>> {
        let agent = self.agent.read(cx);
        let (command, endpoint) = agent.names();
        let alert = agent.model().alert(command, &endpoint)?;
        let mono = Settings::get(cx).font().family;
        let variant = if alert.destructive { AlertVariant::Destructive } else { AlertVariant::Default };
        Some(
            div().debug_selector(|| "agent-alert".into()).child(
                UiAlert::new(alert.text)
                    .variant(variant)
                    .icon(Icon::CircleAlert)
                    .when_some(alert.detail, |a, detail| {
                        a.child(
                            div()
                                .id("agent-alert-detail")
                                .max_h(gpui::rems(8.))
                                .overflow_y_scroll()
                                .whitespace_normal()
                                .child(detail),
                        )
                    })
                    .when_some(alert.mono, |a, line| {
                        a.child(div().debug_selector(|| "agent-alert-mono".into()).font_family(mono).child(line))
                    }),
            ),
        )
    }
}

/// Draws block `ix` of the conversation, from the model as it is.
fn render_block(view: &WeakEntity<AgentView>, ix: usize, cx: &mut App) -> gpui::AnyElement {
    let Some(entity) = view.upgrade() else { return div().into_any_element() };
    let this = entity.read(cx);
    let blocks = this.agent.read(cx).model().blocks().len();
    if ix == blocks
        && let Some(row) = this.agent.read(cx).model().working_row()
    {
        return working_row(&row, Motion::step(cx));
    }
    let Some(block) = this.agent.read(cx).model().blocks().get(ix).cloned() else { return div().into_any_element() };
    let handles: Vec<(SharedString, FocusHandle)> = this
        .permissions
        .iter()
        .filter(|((entry, _), _)| *entry == block.entry)
        .map(|((_, option), handle)| (option.clone(), handle.clone()))
        .collect();
    let colors = (this.theme.ansi[1].hsla(), this.theme.ansi[3].hsla());
    let code_font = Settings::get(cx).font().family;
    // A long diff's or output's runs read as one frame.
    let below = if let BlockBody::ToolDetail(part) = &block.body
        && let Some(Block { body: BlockBody::ToolDetail(next), .. }) = this.agent.read(cx).model().blocks().get(ix + 1)
        && next.entry == part.entry
    {
        tool::continues_into(&part.part, Some(&next.part))
    } else {
        false
    };
    let muted = cx.ui_theme().muted_foreground;
    let body = block_element(view.clone(), &block, &handles, colors, code_font, (below, muted));
    let (top, bottom) = item_padding(&block);
    div().px_3().when(top, |d| d.pt_1()).when(bottom, |d| d.pb_1()).w_full().child(body).into_any_element()
}

/// The Earlier row's words.
const EARLIER_MESSAGES: &str = "Earlier messages";

/// The Earlier row (OpenSpec change `agents-log-paging` decision 11), the
/// scroller's top row, in the zone's tokens: "Earlier messages", with the
/// stepped loader while a page is on its way; after a failure, why,
/// pressable to ask again. It takes no focus: shift-pageup scrolls it into
/// view, which asks.
fn earlier_row(view: &WeakEntity<AgentView>, cx: &mut App) -> gpui::AnyElement {
    let Some(entity) = view.upgrade() else { return div().into_any_element() };
    let this = entity.read(cx);
    let (earlier, agent) = (this.earlier.clone(), this.agent.downgrade());
    let tokens = cx.ui_theme();
    let row = div()
        .debug_selector(|| "agent-earlier".into())
        .px_3()
        .py_2()
        .w_full()
        .flex()
        .items_center()
        .justify_center()
        .gap_2()
        .text_size(gpui::rems(0.875))
        .text_color(tokens.muted_foreground);
    match earlier {
        Earlier::None => row.into_any_element(),
        Earlier::Available { .. } => row.child(EARLIER_MESSAGES).into_any_element(),
        Earlier::Loading { .. } => row
            .child(Loader::new().step(motion::loader_step(Motion::step(cx))).color(tokens.muted_foreground))
            .child(EARLIER_MESSAGES)
            .into_any_element(),
        Earlier::Failed { why, .. } => row
            .id("agent-earlier")
            .cursor_pointer()
            .whitespace_normal()
            .text_color(tokens.destructive)
            .child(format!("Earlier messages couldn't be loaded: {why}"))
            .on_click(move |_, _, cx| {
                agent.update(cx, |agent, cx| agent.retry_fetch(cx)).ok();
            })
            .into_any_element(),
    }
}

/// The working row (decision 2): its words shimmering at the motion
/// clock's `step`, muted, padded as a list item.
fn working_row(row: &WorkingRow, step: u32) -> gpui::AnyElement {
    let text = row.text();
    let position = motion::shimmer_position(step, text.chars().count());
    div()
        .debug_selector(|| "agent-working".into())
        .px_3()
        .py_1()
        .w_full()
        .text_size(gpui::rems(0.875))
        .child(Shimmer::new(text).position(position))
        .into_any_element()
}

/// Rich-text `blocks` as a [`Response`] in `code_font`, its links acting
/// on ctrl ([`AgentView::press_link`]) and its code blocks' copy buttons
/// writing the clipboard.
fn response(
    view: WeakEntity<AgentView>,
    id: ElementId,
    blocks: impl IntoIterator<Item = Arc<RichBlock>>,
    code_font: SharedString,
) -> Response {
    Response::new(id, blocks)
        .code_font(code_font)
        .link_modifiers(Modifiers::control())
        .on_link(move |link, modifiers, _, cx| {
            let target = link.target.clone();
            view.update(cx, |view, cx| view.press_link(&target, modifiers, cx)).ok();
        })
        .on_copy_code(|text, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(text.to_string())))
}

/// Writes the copy text of item `item` of tool call `entry` (a unified
/// diff, an output's text) to the clipboard.
fn copy_tool_item(view: &WeakEntity<AgentView>, entry: EntryId, item: usize, cx: &mut App) {
    let Some(view) = view.upgrade() else { return };
    let text = view.read(cx).agent.read(cx).model().tool_copy_text(entry, item);
    if let Some(text) = text {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }
}

/// One part of a tool call's detail (`agents-tool-content` decision 10):
/// a text item as a response, a run of a diff's lines as a [`Diff`], a run
/// of an output's or a raw value as a [`CodeBlock`], the locations as a
/// muted list, a mark in muted words. `below`: the part's frame goes on
/// into the next block.
fn detail_element(
    view: &WeakEntity<AgentView>,
    id: ElementId,
    shown: &ShownPart,
    below: bool,
    code_font: &SharedString,
    muted: gpui::Hsla,
) -> gpui::AnyElement {
    let entry = shown.entry;
    match &shown.part {
        DetailPart::Text { .. } => {
            response(view.clone(), id, shown.text.iter().cloned(), code_font.clone()).into_any_element()
        }
        DetailPart::Diff { item, head, rows } => {
            let (view, item) = (view.clone(), *item);
            let diff = Diff::new(id, rows.iter().cloned()).font(code_font.clone()).joined(head.is_none(), below);
            match head {
                Some(head) => diff
                    .header(head.op.clone(), head.path.clone(), head.from.clone())
                    .on_copy(move |_, cx| copy_tool_item(&view, entry, item, cx))
                    .into_any_element(),
                None => diff.into_any_element(),
            }
        }
        DetailPart::Output { item, head, text } => {
            let (view, item) = (view.clone(), *item);
            let code = Code { label: head.as_ref().map(|head| head.label.clone()), text: text.clone() };
            CodeBlock::new(id, code)
                .font(code_font.clone())
                .joined(head.is_none(), below)
                .on_copy(move |_, _, cx| copy_tool_item(&view, entry, item, cx))
                .into_any_element()
        }
        DetailPart::Raw { which, text } => {
            CodeBlock::new(id, Code { label: Some(which.word().into()), text: text.clone() })
                .font(code_font.clone())
                .on_copy(|text, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(text.to_string())))
                .into_any_element()
        }
        DetailPart::Locations { lines } => div()
            .debug_selector(|| "agent-tool-locations".into())
            .flex()
            .flex_col()
            .text_size(gpui::rems(0.75))
            .text_color(muted)
            .font_family(code_font.clone())
            .children(lines.iter().cloned())
            .into_any_element(),
        DetailPart::Mark { text } => {
            div().text_size(gpui::rems(0.75)).text_color(muted).child(text.clone()).into_any_element()
        }
    }
}

fn block_element(
    view: WeakEntity<AgentView>,
    block: &Block,
    handles: &[(SharedString, FocusHandle)],
    (error, warning): (gpui::Hsla, gpui::Hsla),
    code_font: SharedString,
    (below, muted): (bool, gpui::Hsla),
) -> gpui::AnyElement {
    let entry = block.entry;
    let id = |what: &str| ElementId::Name(format!("agent-{what}-{}-{}", entry.0, block.part).into());
    // One part of a message: a later part is set apart from the one above
    // it as a response's blocks are (`gap-3`), inside the same bubble.
    let message = |view: WeakEntity<AgentView>, from: MessageFrom, content: gpui::AnyElement| {
        let copy = Button::icon_only(id("copy"), Icon::Copy)
            .variant(ButtonVariant::Ghost)
            .size(ButtonSize::IconXs)
            .tooltip_for("Copy", None)
            .on_click(move |_, _, cx| {
                let Some(view) = view.upgrade() else { return };
                let text = view.read(cx).agent.read(cx).model().message_text(entry).map(str::to_string);
                if let Some(text) = text {
                    cx.write_to_clipboard(ClipboardItem::new_string(text));
                }
            });
        Message::new(id("message"), from)
            .part(bubble_part(block.part, block.last))
            .child(
                div()
                    .debug_selector(|| "agent-message-content".into())
                    .when(block.part > 0, |d| d.pt_3())
                    .child(content),
            )
            .action(div().debug_selector(|| "agent-message-copy".into()).child(copy))
            .into_any_element()
    };
    match &block.body {
        BlockBody::UserText { text } => message(view, MessageFrom::User, text.clone().into_any_element()),
        BlockBody::AgentPart { block: rich } => {
            let content = response(view.clone(), id("response"), [rich.clone()], code_font).into_any_element();
            message(view, MessageFrom::Assistant, content)
        }
        BlockBody::Thought { blocks, open } => {
            let content = response(view.clone(), id("thought-response"), blocks.iter().cloned(), code_font);
            Reasoning::new(id("thought"), *open)
                .label("Thought")
                .on_toggle(move |_, _, cx| {
                    view.update(cx, |view, cx| view.agent.update(cx, |agent, cx| agent.toggle_thought(entry, cx))).ok();
                })
                .child(content)
                .into_any_element()
        }
        BlockBody::Tool { kind, title, status, summary, state } => {
            Tool::new(tool_icon(*kind), title.clone(), tool_status(*status))
                .id(id("tool"))
                .when_some(summary.clone(), |tool, summary| tool.summary(summary))
                .open(state.open())
                .on_toggle(move |_, _, cx| {
                    view.update(cx, |view, cx| view.agent.update(cx, |agent, cx| agent.toggle_tool(entry, cx))).ok();
                })
                .into_any_element()
        }
        BlockBody::ToolDetail(shown) => {
            div().pt_1().child(detail_element(&view, id("detail"), shown, below, &code_font, muted)).into_any_element()
        }
        BlockBody::Permission { title, options, outcome, detail, .. } => {
            let state = match outcome {
                None => ConfirmationState::Requested,
                Some(PermissionOutcome::Selected { option }) => match options.iter().find(|o| o.id.as_ref() == option) {
                    Some(chosen) if allows(chosen.kind) => ConfirmationState::Accepted { label: chosen.name.clone() },
                    Some(chosen) => ConfirmationState::Rejected { label: chosen.name.clone() },
                    None => ConfirmationState::Accepted { label: option.clone().into() },
                },
                Some(PermissionOutcome::Cancelled) => ConfirmationState::Cancelled,
            };
            // Drawn as a risk while pending; the component draws an
            // answered one plain whatever its tone.
            let mut confirmation =
                Confirmation::new(id("permission"), title.clone(), state).tone(ConfirmationTone::Destructive);
            for option in options.iter().filter(|_| outcome.is_none()) {
                let mut action = ConfirmationAction::new(option.name.clone())
                    .key(ItemKey::new(option.id.clone()))
                    .variant(permission_variant(option.kind));
                if let Some((_, handle)) = handles.iter().find(|(own, _)| *own == option.id) {
                    action = action.focus(handle);
                }
                confirmation = confirmation.action(action);
            }
            // A pending request shows its call's detail inside its alert,
            // between the title and the buttons.
            let parts = detail.iter().enumerate().map(|(k, shown)| {
                let below = tool::continues_into(&shown.part, detail.get(k + 1).map(|next| &next.part));
                let id = ElementId::Name(format!("agent-permission-detail-{}-{k}", entry.0).into());
                detail_element(&view, id, shown, below, &code_font, muted)
            });
            let parts: Vec<gpui::AnyElement> = parts.collect();
            confirmation
                .when(!parts.is_empty(), |c| {
                    c.child(div().debug_selector(|| "agent-permission-detail".into()).flex().flex_col().children(parts))
                })
                .on_action(move |key: &ItemKey, _, cx| {
                    let option = key.as_str().to_string();
                    view.update(cx, |view, cx| view.agent.update(cx, |agent, cx| agent.answer(entry, &option, cx)))
                        .ok();
                })
                .into_any_element()
        }
        BlockBody::Notice { level, text } => {
            let color = match level {
                NoticeLevel::Info => None,
                NoticeLevel::Warning => Some(warning),
                NoticeLevel::Error => Some(error),
            };
            div()
                .text_size(gpui::rems(0.75))
                .opacity(if color.is_some() { 1. } else { 0.7 })
                .when_some(color, |d, color| d.text_color(color))
                .child(text.clone())
                .into_any_element()
        }
        BlockBody::Attached { index, attachment, state } => {
            let content = attached_element(view.clone(), entry, *index, attachment, state, &code_font, muted);
            message(view, MessageFrom::User, content)
        }
    }
}

/// A user message's attachment, inside its bubble under its text (OpenSpec
/// change `agents-prompt-context`, decision 9): a link as its path; a kept
/// text as a row with its name and size, which a press opens and closes,
/// and under it the text as code, in words while it loads, or why it is
/// gone. Nothing in it moves, and the row is no keyboard stop.
fn attached_element(
    view: WeakEntity<AgentView>,
    entry: EntryId,
    index: usize,
    attachment: &Attachment,
    state: &TextState,
    code_font: &SharedString,
    muted: gpui::Hsla,
) -> gpui::AnyElement {
    let id = |what: &str| ElementId::Name(format!("agent-attached-{what}-{}-{index}", entry.0).into());
    let line = |icon: Icon, name: String, detail: String| {
        div()
            .flex()
            .items_center()
            .gap_1()
            .text_size(gpui::rems(0.75))
            .child(IconElement::new(icon).size(gpui::rems(0.875)).color(muted))
            .child(div().min_w_0().truncate().child(name))
            .child(div().flex_none().text_color(muted).child(detail))
    };
    match attachment {
        Attachment::Link { path, name } => {
            line(Icon::FileText, name.clone(), path.clone()).debug_selector(|| "agent-attached-link".into()).into_any_element()
        }
        Attachment::Text { name, bytes, .. } => {
            let open = matches!(state, TextState::Loading | TextState::Shown(_));
            let row = line(Icon::Terminal, name.clone(), model::size(*bytes))
                .id(id("row"))
                .debug_selector(|| "agent-attached-text".into())
                .cursor_pointer()
                .child(IconElement::new(Icon::ChevronDown).size(gpui::rems(0.75)).color(muted).turn(if open { 0.5 } else { 0. }))
                .on_click(move |_, _, cx| {
                    view.update(cx, |view, cx| view.agent.update(cx, |agent, cx| agent.press_row(entry, index, cx))).ok();
                });
            let below = match state {
                TextState::Closed => None,
                TextState::Loading => Some(div().text_size(gpui::rems(0.75)).text_color(muted).child("Loading…").into_any_element()),
                TextState::Shown(text) => Some(
                    CodeBlock::new(id("code"), Code { label: Some(name.clone().into()), text: text.clone().into() })
                        .font(code_font.clone())
                        .on_copy(|text, _, cx| cx.write_to_clipboard(ClipboardItem::new_string(text.to_string())))
                        .into_any_element(),
                ),
                TextState::Gone(why) => {
                    Some(div().text_size(gpui::rems(0.75)).text_color(muted).child(why.clone()).into_any_element())
                }
            };
            div().flex().flex_col().gap_1().child(row).children(below).into_any_element()
        }
    }
}

/// Lays out, prepaints and paints its child with the rem as `size`, so the
/// components inside, sized in rems, follow the zone's text size (the
/// profile's font size, else the terminal's) rather than the chrome's.
struct RemScope {
    size: Pixels,
    child: gpui::AnyElement,
}

impl IntoElement for RemScope {
    type Element = Self;

    fn into_element(self) -> Self {
        self
    }
}

impl gpui::Element for RemScope {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, ()) {
        let child = &mut self.child;
        (window.with_rem_size(Some(self.size), |window| child.request_layout(window, cx)), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: gpui::Bounds<Pixels>,
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let child = &mut self.child;
        window.with_rem_size(Some(self.size), |window| child.prepaint(window, cx));
    }

    fn paint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: gpui::Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let child = &mut self.child;
        window.with_rem_size(Some(self.size), |window| child.paint(window, cx));
    }
}

impl Render for AgentView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let model = self.agent.read(cx).model();
        let endpoint = self.agent.read(cx).names().1;
        let banner = model.banner(&endpoint);
        let notes: Vec<String> = model.notes().map(str::to_string).collect();
        let alert = self.alert(cx);
        let login = self.login_row(cx);
        let settings = Settings::get(cx);
        let (size, family) = (self.text_size(cx), settings.ui_font_family());
        // The scope's set (the zone's tokens), one source with the
        // components inside.
        let tokens = cx.ui_theme();
        let (bg, fg, ring) = (tokens.background, tokens.foreground, tokens.ring);
        let prompt_handle = self.prompt.read(cx).focus_handle(cx);
        let content = div()
            .key_context("AgentZone")
            // The zone's press rule (OpenSpec change
            // `agents-zone-focus-and-scroll` decision 1): a press, of any
            // button, that no control under it took focuses the prompt. A
            // control that takes focus (the text area, a permission's,
            // the login row's or the queue row's buttons, stop, Continue)
            // has already focused itself and prevented the default, so
            // this does nothing for it; bubble listeners run child first.
            // Without this, the zone tree's tracked focus would take the
            // press and leave the keyboard in no stop.
            .on_any_mouse_down(move |_, window, cx| {
                if !window.default_prevented() {
                    brindle_ui::focus_from_pointer(&prompt_handle, window, cx);
                }
            })
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::unqueue))
            .on_action(cx.listener(Self::resend))
            .on_action(cx.listener(Self::continue_conversation))
            .on_action(cx.listener(Self::conversations))
            .on_action(cx.listener(Self::options))
            .on_action(cx.listener(Self::next_mode))
            .on_action(cx.listener(Self::copy_queued))
            .on_action(cx.listener(Self::attach_file))
            .on_action(cx.listener(Self::clear_attachments))
            .capture_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::copy_last))
            .on_action(cx.listener(Self::copy_last_code))
            .on_action(cx.listener(Self::toggle_thoughts))
            .on_action(cx.listener(Self::toggle_tool_details))
            // A link shows the pointing hand only while ctrl is held: a
            // modifiers change with the pointer still reaches only the
            // focused path (the prompt input, inside this), so the frame is
            // drawn again here for the links' pointer to follow it, when
            // the gesture flipped and only then (decision 6).
            .on_modifiers_changed(cx.listener(|this, event: &gpui::ModifiersChangedEvent, _, cx| {
                if let Some(now) = link_gesture_flip(this.link_gesture, &event.modifiers) {
                    this.link_gesture = now;
                    cx.notify();
                }
            }))
            .on_action(cx.listener(|this, _: &AgentScrollToBottom, _, cx| this.scroll_to_end(cx)))
            .on_action(cx.listener(|this, _: &ScrollToBottom, _, cx| this.scroll_to_end(cx)))
            .on_action(cx.listener(|this, _: &ScrollPageUp, _, cx| this.page(PageDirection::Up, cx)))
            .on_action(cx.listener(|this, _: &ScrollPageDown, _, cx| this.page(PageDirection::Down, cx)))
            .size_full()
            .min_h_0()
            .bg(bg)
            .text_color(fg)
            .text_size(size)
            .font_family(family)
            .child(
                FocusScope::new(&cx.entity())
                    // Tab onto a permission button brings its block into
                    // view (decision 4).
                    .on_step({
                        let view = cx.weak_entity();
                        move |handle, _, cx| {
                            view.update(cx, |this, cx| this.reveal_permission(handle, cx)).ok();
                        }
                    })
                    .size_full()
                    .min_h_0()
                    .flex()
                    .flex_col()
                    .children(banner.map(|banner| {
                        div()
                            .debug_selector(|| "agent-banner".into())
                            .flex_none()
                            .px_3()
                            .py_1()
                            .text_size(gpui::rems(0.75))
                            .opacity(0.8)
                            .child(banner)
                    }))
                    .children(notes.into_iter().map(|note| {
                        div()
                            .debug_selector(|| "agent-note".into())
                            .flex_none()
                            .px_3()
                            .py_1()
                            .text_size(gpui::rems(0.75))
                            .opacity(0.8)
                            .child(note)
                    }))
                    .child(div().flex_1().min_h_0().flex().flex_col().child(self.scroller.clone()))
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .flex_col()
                            .gap_2()
                            .px_3()
                            .pt_1()
                            .pb_3()
                            .children(alert)
                            .children(login)
                            // Files hovering over the zone highlight the
                            // prompt input they would join.
                            .child(
                                div()
                                    .debug_selector(|| "agent-prompt-drop".into())
                                    .rounded(tokens.radius_md())
                                    .border_1()
                                    .border_color(gpui::transparent_black())
                                    .drag_over::<ExternalPaths>(move |style, _, _, _| style.border_color(ring))
                                    .child(self.prompt.clone()),
                            ),
                    ),
            )
            // Files dropped anywhere on the zone (decision 8).
            .on_drop(cx.listener(|this, paths: &ExternalPaths, _, cx| {
                let paths = paths.paths().to_vec();
                this.agent.update(cx, |agent, cx| agent.drop_paths(paths, cx));
            }));
        RemScope { size, child: content.into_any_element() }
    }
}

#[cfg(test)]
mod tests;
