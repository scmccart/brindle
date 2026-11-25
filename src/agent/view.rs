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
//! ([`FocusStops`]): the prompt, the queue row's buttons, the pending
//! permission requests' buttons, and the stop button while a turn runs.
//! When a stop the focus is on goes away (the row's buttons when the slot
//! changes, the stop button when the turn ends, a permission answered),
//! focus goes back to the prompt.

use gpui::{
    App, AppContext as _, ClipboardItem, Context, ElementId, Entity, FocusHandle, Focusable, InteractiveElement as _,
    IntoElement, ParentElement as _, Pixels, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    Subscription, WeakEntity, Window, div, prelude::FluentBuilder as _,
};

use brindle_ui::{
    Alert as UiAlert, AlertVariant, BubblePart, Button, ButtonSize, ButtonVariant, Confirmation, ConfirmationAction,
    ConfirmationState, FocusScope, FocusStops, Icon, ItemKey, Message, MessageFrom, MessageScroller, PageDirection,
    PromptInput, PromptInputEvent, PromptStatus as UiStatus, Queued, Reasoning, Resend as UiResend, TextAreaEvent, Tool,
    ToolStatus as UiToolStatus, TooltipFor as _,
};

use crate::actions::{
    AgentCancel, AgentResend, AgentScrollToBottom, AgentSubmit, AgentUnqueue, CopyLastAgentMessage, CopyQueuedPrompt,
    ScrollPageDown, ScrollPageUp, ScrollToBottom, ToggleAgentThoughts,
};
use crate::agent::model::{Block, BlockBody, ChatModel, PromptStatus, QueueRow, Resend, Speaker};
use crate::agent::transcript::{EntryId, NoticeLevel, PermissionKind, PermissionOutcome, ToolKind, ToolStatus};
use crate::agent::{Agent, AgentUpdate};
use crate::settings::Settings;
use crate::theme::Theme;

/// What the prompt input shows for the model: its status, its queue row
/// and the line beside its button (why it is disabled, else the last
/// refusal).
pub fn prompt_state(model: &ChatModel) -> (UiStatus, Option<Queued>, Option<SharedString>) {
    let (status, reason) = match model.prompt_status() {
        PromptStatus::Ready => (UiStatus::Ready, None),
        PromptStatus::Submitted => (UiStatus::Submitted, None),
        PromptStatus::Streaming => (UiStatus::Streaming, None),
        PromptStatus::Disabled(why) => (UiStatus::Disabled, Some(SharedString::from(why.text()))),
    };
    let reason = reason.or_else(|| model.refusal().map(|refusal| SharedString::from(refusal.to_string())));
    (status, queued_of(model.queue()), reason)
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
    theme: Theme,
    /// The profile's font size, else the terminal size in effect.
    font_size_override: Option<f32>,
    _subscriptions: Vec<Subscription>,
}

impl Focusable for AgentView {
    /// The prompt input's: focusing the zone puts the caret in the prompt.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.prompt.read(cx).focus_handle(cx)
    }
}

impl FocusStops for AgentView {
    /// The prompt, the queue row's buttons, the pending permissions'
    /// buttons, then the stop button while a turn runs.
    fn stops(&self, cx: &App) -> Vec<FocusHandle> {
        let prompt = self.prompt.read(cx);
        let mut stops = vec![prompt.focus_handle(cx)];
        stops.extend(prompt.row_stops());
        stops.extend(self.permissions.iter().map(|(_, handle)| handle.clone()));
        stops.extend(prompt.stop_button());
        stops
    }
}

#[allow(dead_code, reason = "group 6 puts the view in `ZoneContent::Agent` and reads it through these accessors")]
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
        let model = self.agent.read(cx).model();
        let (status, queued, reason) = prompt_state(model);
        let pending = model.pending_permissions();
        self.prompt.update(cx, |prompt, cx| {
            prompt.set_status(status, cx);
            prompt.set_queued(queued, cx);
            prompt.set_reason(reason, cx);
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

    fn toggle_thoughts(&mut self, _: &ToggleAgentThoughts, _: &mut Window, cx: &mut Context<Self>) {
        self.agent.update(cx, |agent, cx| agent.toggle_thoughts(cx));
    }

    fn scroll_to_end(&mut self, cx: &mut Context<Self>) {
        self.scroller.update(cx, |scroller, cx| scroller.scroll_to_end(cx));
    }

    fn page(&mut self, dir: PageDirection, cx: &mut Context<Self>) {
        self.scroller.update(cx, |scroller, cx| scroller.scroll_page(dir, cx));
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
    let body = block_element(view.clone(), &block, &handles, colors);
    div().px_3().py_1().w_full().child(body).into_any_element()
}

fn block_element(
    view: WeakEntity<AgentView>,
    block: &Block,
    handles: &[(SharedString, FocusHandle)],
    (error, warning): (gpui::Hsla, gpui::Hsla),
) -> gpui::AnyElement {
    let entry = block.entry;
    let id = |what: &str| ElementId::Name(format!("agent-{what}-{}-{}", entry.0, block.part).into());
    match &block.body {
        BlockBody::Text { speaker, text } => {
            let from = match speaker {
                Speaker::User => MessageFrom::User,
                Speaker::Agent => MessageFrom::Assistant,
            };
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
            Message::text(id("message"), from, text.clone()).part(bubble_part(block.part, block.last)).action(copy).into_any_element()
        }
        BlockBody::Thought { text, open } => Reasoning::new(id("thought"), *open)
            .label("Thought")
            .on_toggle(move |_, _, cx| {
                view.update(cx, |view, cx| view.agent.update(cx, |agent, cx| agent.toggle_thought(entry, cx))).ok();
            })
            .child(text.clone())
            .into_any_element(),
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
        let settings = Settings::get(cx);
        let (size, family) = (self.text_size(cx), settings.ui_font_family());
        let (bg, fg) = (self.theme.background.hsla(), self.theme.foreground.hsla());
        let content = div()
            .key_context("AgentZone")
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::unqueue))
            .on_action(cx.listener(Self::resend))
            .on_action(cx.listener(Self::copy_queued))
            .on_action(cx.listener(Self::copy_last))
            .on_action(cx.listener(Self::toggle_thoughts))
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
                            .child(self.prompt.clone()),
                    ),
            );
        RemScope { size, child: content.into_any_element() }
    }
}

#[cfg(test)]
mod tests;
