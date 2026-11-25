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

use std::sync::Arc;

use gpui::{
    App, AppContext as _, ClipboardItem, Context, ElementId, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement as _, IntoElement, Modifiers, ParentElement as _, Pixels, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, WeakEntity, Window, div, prelude::FluentBuilder as _,
};

use brindle_ui::{
    Alert as UiAlert, AlertVariant, BubblePart, Button, ButtonSize, ButtonVariant, Confirmation, ConfirmationAction,
    ConfirmationState, FocusScope, FocusStops, Icon, ItemKey, Message, MessageFrom, MessageScroller, PageDirection,
    PromptFocus, PromptInput, PromptInputEvent, PromptStatus as UiStatus, Queued, Reasoning, Resend as UiResend,
    Response, RichBlock, TextAreaEvent, Tool, ToolStatus as UiToolStatus, TooltipFor as _,
};

use crate::actions::{
    AgentCancel, AgentContinue, AgentResend, AgentScrollToBottom, AgentSubmit, AgentUnqueue, CopyLastAgentMessage, CopyLastCodeBlock,
    CopyQueuedPrompt, ScrollPageDown, ScrollPageUp, ScrollToBottom, ToggleAgentThoughts,
};
use crate::agent::login::MethodOffer;
use crate::agent::markdown::{LinkAction, link_action};
use crate::agent::model::{
    AgentPhase, Block, BlockBody, ChatModel, LoginRow, MethodChoice, NoOffer, Offer, PromptStatus, QueueRow, Resend,
};
use crate::agent::transcript::{EntryId, NoticeLevel, PermissionKind, PermissionOutcome, ToolKind, ToolStatus};
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
        BlockBody::UserText { .. } | BlockBody::AgentPart { .. } => {
            let edges = bubble_part(block.part, block.last).edges();
            (edges.top, edges.bottom)
        }
        BlockBody::Thought { .. } | BlockBody::Tool { .. } | BlockBody::Permission { .. } | BlockBody::Notice { .. } => {
            (true, true)
        }
    }
}

/// The Continue conversation button's label.
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
/// line, `reason` then the `action` button's label.
#[derive(Debug, Clone, PartialEq)]
pub struct PromptState {
    pub status: UiStatus,
    pub queued: Option<Queued>,
    pub reason: Option<SharedString>,
    pub action: Option<SharedString>,
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
    let (reason, action) = match offer {
        Offer::Restart | Offer::Respawn { .. } => (refusal.or(reason), Some(SharedString::from(CONTINUE))),
        Offer::None(NoOffer::NotEnded) => (reason.or(refusal), None),
        Offer::None(NoOffer::NoConversation) if matches!(model.phase(), AgentPhase::NotStarted(_)) => {
            (reason.or(refusal), None)
        }
        Offer::None(why @ (NoOffer::NoConversation | NoOffer::NotOffered | NoOffer::ProfileGone { .. })) => {
            (Some(SharedString::from(why.text())), None)
        }
    };
    PromptState { status, queued: queued_of(model.queue()), reason, action }
}

/// The prompt input's queue row for the model's [`QueueRow`].
pub fn queued_of(row: &QueueRow) -> Option<Queued> {
    match row {
        QueueRow::None => None,
        QueueRow::Waiting { text } => Some(Queued::Waiting { text: text.clone() }),
        QueueRow::Held { text, why, resend, reason: _ } => Some(Queued::Held {
            text: text.clone(),
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

/// An agent zone's view.
pub struct AgentView {
    agent: Entity<Agent>,
    scroller: Entity<MessageScroller>,
    prompt: Entity<PromptInput>,
    /// The pending permission requests' buttons, in the conversation's
    /// order, keyed by entry and option (`types R3`).
    permissions: Vec<((EntryId, SharedString), FocusHandle)>,
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
        stops.extend(prompt.action_stop());
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
        let count = agent.read(cx).model().blocks().len();
        let view = cx.weak_entity();
        let scroller = cx.new(|cx| MessageScroller::new(count, move |ix, _, cx| render_block(&view, ix, cx), cx));
        let area = prompt.read(cx).area().clone();
        let subscriptions = vec![
            cx.subscribe_in(&agent, window, |this, _, update: &AgentUpdate, _, cx| this.on_update(update, cx)),
            cx.observe_in(&agent, window, |this, _, window, cx| this.sync(window, cx)),
            // The continue offer reads the config's profiles: an edit can
            // take the button away or bring it back.
            cx.observe_global_in::<Settings>(window, |this, window, cx| this.sync(window, cx)),
            cx.subscribe(&prompt, |this, _, event: &PromptInputEvent, cx| this.on_prompt(event, cx)),
            cx.subscribe(&area, |this, _, _: &TextAreaEvent, cx| {
                this.agent.update(cx, |agent, cx| agent.seen(cx));
            }),
        ];
        let mut this = Self {
            agent,
            scroller,
            prompt,
            permissions: Vec::new(),
            login: LoginRow::None,
            logins: Vec::new(),
            cancel_login: cx.focus_handle(),
            theme,
            font_size_override,
            _subscriptions: subscriptions,
        };
        this.refresh(cx);
        this
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
            return Some(
                match stop {
                    PromptFocus::Text => "prompt",
                    PromptFocus::Remove => "queued:remove",
                    PromptFocus::SendAgain => "queued:send-again",
                    PromptFocus::Discard => "queued:discard",
                    PromptFocus::Stop => "stop",
                    PromptFocus::Action => "continue",
                }
                .to_string(),
            );
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

    /// Takes the tab's profile's theme (the card's background and text,
    /// and the held row's warning colour) and font size (`None`: the
    /// terminal size in effect), as the config now has them or the
    /// settings dialog previews them.
    pub fn set_profile(&mut self, theme: Theme, font_size_override: Option<f32>, cx: &mut Context<Self>) {
        let held = theme.ansi[3].hsla();
        self.prompt.update(cx, |prompt, cx| prompt.set_held_color(held, cx));
        self.theme = theme;
        self.font_size_override = font_size_override;
        // Every block is measured again at the new size.
        self.scroller.update(cx, |scroller, cx| {
            let count = scroller.item_count();
            scroller.splice(0..count, count, cx);
        });
        cx.notify();
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
            PromptInputEvent::Action => self.agent.update(cx, |agent, cx| agent.continue_conversation(cx)),
        }
    }

    /// The agent changed: the prompt input and the permission stops follow
    /// the model, and a stop that had focus and went away hands it back to
    /// the prompt.
    fn sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let before = self.stops(cx);
        self.refresh(cx);
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
        self.prompt.update(cx, |prompt, cx| {
            prompt.set_status(state.status, cx);
            prompt.set_queued(state.queued, cx);
            prompt.set_reason(state.reason, cx);
            prompt.set_action(state.action, cx);
        });
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
    let Some(block) = this.agent.read(cx).model().blocks().get(ix).cloned() else { return div().into_any_element() };
    let handles: Vec<(SharedString, FocusHandle)> = this
        .permissions
        .iter()
        .filter(|((entry, _), _)| *entry == block.entry)
        .map(|((_, option), handle)| (option.clone(), handle.clone()))
        .collect();
    let colors = (this.theme.ansi[1].hsla(), this.theme.ansi[3].hsla());
    let code_font = Settings::get(cx).font().family;
    let body = block_element(view.clone(), &block, &handles, colors, code_font);
    let (top, bottom) = item_padding(&block);
    div().px_3().when(top, |d| d.pt_1()).when(bottom, |d| d.pb_1()).w_full().child(body).into_any_element()
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

fn block_element(
    view: WeakEntity<AgentView>,
    block: &Block,
    handles: &[(SharedString, FocusHandle)],
    (error, warning): (gpui::Hsla, gpui::Hsla),
    code_font: SharedString,
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
            .child(div().when(block.part > 0, |d| d.pt_3()).child(content))
            .action(copy)
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
        BlockBody::Tool { kind, title, status } => {
            Tool::new(tool_icon(*kind), title.clone(), tool_status(*status)).into_any_element()
        }
        BlockBody::Permission { title, options, outcome } => {
            let state = match outcome {
                None => ConfirmationState::Requested,
                Some(PermissionOutcome::Selected { option }) => match options.iter().find(|o| o.id.as_ref() == option) {
                    Some(chosen) if allows(chosen.kind) => ConfirmationState::Accepted { label: chosen.name.clone() },
                    Some(chosen) => ConfirmationState::Rejected { label: chosen.name.clone() },
                    None => ConfirmationState::Accepted { label: option.clone().into() },
                },
                Some(PermissionOutcome::Cancelled) => ConfirmationState::Cancelled,
            };
            let mut confirmation = Confirmation::new(id("permission"), title.clone(), state);
            for option in options.iter().filter(|_| outcome.is_none()) {
                let mut action = ConfirmationAction::new(option.name.clone()).key(ItemKey::new(option.id.clone())).variant(
                    if allows(option.kind) { ButtonVariant::Default } else { ButtonVariant::Outline },
                );
                if let Some((_, handle)) = handles.iter().find(|(own, _)| *own == option.id) {
                    action = action.focus(handle);
                }
                confirmation = confirmation.action(action);
            }
            confirmation
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
        let note = model.note().map(str::to_string);
        let alert = self.alert(cx);
        let login = self.login_row(cx);
        let settings = Settings::get(cx);
        let (size, family) = (self.text_size(cx), settings.ui_font_family());
        let (bg, fg) = (self.theme.background.hsla(), self.theme.foreground.hsla());
        let content = div()
            .key_context("AgentZone")
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::unqueue))
            .on_action(cx.listener(Self::resend))
            .on_action(cx.listener(Self::continue_conversation))
            .on_action(cx.listener(Self::copy_queued))
            .on_action(cx.listener(Self::copy_last))
            .on_action(cx.listener(Self::copy_last_code))
            .on_action(cx.listener(Self::toggle_thoughts))
            // A link shows the pointing hand only while ctrl is held: a
            // modifiers change with the pointer still reaches only the
            // focused path (the prompt input, inside this), so the frame is
            // drawn again here for the links' pointer to follow it.
            .on_modifiers_changed(cx.listener(|_, _, _, cx| cx.notify()))
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
                    .children(note.map(|note| {
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
                            .child(self.prompt.clone()),
                    ),
            );
        RemScope { size, child: content.into_any_element() }
    }
}

#[cfg(test)]
mod tests;
