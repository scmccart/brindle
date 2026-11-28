use super::blocks::{ToolDraw, blocks_of};
use super::*;
use crate::agent::markdown;
use crate::agent::shared::AgentShared;
use crate::agent::tool::RowState;
use crate::agent::transcript::{
    DiffLine, Event, FileChange, FileOp, HoldReason, Hunk, OptionCategory, OptionControl, PAGE_BYTES, PermissionKind,
    PermissionOption, StopReason, ToolItem, ToolKind, ToolStatus,
};
use crate::terminal::links::Plan;

/// The continue offer of `model` with its profile found, as the dump
/// takes it.
pub(super) fn found(model: &ChatModel) -> Offer {
    model.offer(&ProfileFound::Agent)
}

/// A zone's model fed through the fold, as the view feeds it.
pub(super) struct Zone {
    pub(super) shared: AgentShared,
    pub(super) model: ChatModel,
}

impl Zone {
    /// A zone whose session runs, as most tests want.
    pub(super) fn new(first: Option<&str>) -> Self {
        let mut zone = Self::starting(first);
        zone.model.session_running();
        zone
    }

    /// A zone whose session isn't handed over yet.
    pub(super) fn starting(first: Option<&str>) -> Self {
        let first = first.map(|text| FirstPrompt::new(text.to_string()).unwrap());
        Self { shared: AgentShared::new(), model: ChatModel::new("Fake", first) }
    }

    pub(super) fn feed(&mut self, events: &[Event]) -> Absorbed {
        let values: Vec<_> = events.iter().map(Event::to_value).collect();
        self.shared.apply(&values).unwrap();
        self.absorb()
    }

    pub(super) fn absorb(&mut self) -> Absorbed {
        let changes = self.shared.take_changes();
        self.model.absorb(self.shared.transcript(), changes)
    }

    pub(super) fn replay(&mut self, events: &[Event]) -> Absorbed {
        self.shared.replay_start();
        let values: Vec<_> = events.iter().map(Event::to_value).collect();
        self.shared.apply(&values).unwrap();
        self.shared.replay_end();
        self.absorb()
    }

    /// A started zone: the agent ready, named "Fake Agent".
    pub(super) fn ready() -> Self {
        let mut zone = Self::new(None);
        zone.feed(&[ready()]);
        zone
    }
}

pub(super) fn ready() -> Event {
    Event::Agent { state: AgentState::Ready { name: Some("Fake Agent".into()), version: None } }
}

pub(super) fn agent_state(state: AgentState) -> Event {
    Event::Agent { state }
}

pub(super) fn turn(state: TurnState) -> Event {
    Event::Turn { state }
}

pub(super) fn queue(slot: Slot) -> Event {
    Event::Queue { slot }
}

pub(super) fn added(id: u64, entry: Entry) -> Event {
    Event::Added { id: EntryId(id), entry }
}

/// Tool call `id` completed, its other fields unchanged.
pub(super) fn tool_completed(id: u64) -> Event {
    Event::ToolChanged {
        id: EntryId(id),
        title: None,
        kind: None,
        status: Some(ToolStatus::Completed),
        locations: None,
        input: None,
        result: None,
        more: None,
    }
}

pub(super) fn user(text: &str) -> Entry {
    Entry::User { text: text.into() }
}

pub(super) fn agent(text: &str) -> Entry {
    Entry::Agent { text: text.into() }
}

pub(super) fn thought(text: &str) -> Entry {
    Entry::Thought { text: text.into() }
}

pub(super) fn option(id: &str, name: &str, kind: PermissionKind) -> PermissionOption {
    PermissionOption { id: id.into(), name: name.into(), kind }
}

pub(super) fn permission(title: &str) -> Entry {
    Entry::Permission {
        call: Some("call_2".into()),
        title: title.into(),
        options: vec![
            option("reject-once", "Reject", PermissionKind::RejectOnce),
            option("allow-once", "Allow once", PermissionKind::AllowOnce),
        ],
        outcome: None,
    }
}

pub(super) fn waiting() -> Slot {
    Slot::Waiting { text: "next".into() }
}

pub(super) fn held(reason: HoldReason) -> Slot {
    Slot::Held { text: "next".into(), reason }
}

pub(super) fn texts(model: &ChatModel) -> Vec<String> {
    model
        .blocks()
        .iter()
        .map(|block| match &block.body {
            BlockBody::UserText { text } | BlockBody::Notice { text, .. } => text.to_string(),
            BlockBody::AgentPart { block } => markdown::shown_text(block),
            BlockBody::Thought { blocks, .. } => {
                blocks.iter().map(|block| markdown::shown_text(block)).collect::<Vec<_>>().join("\n\n")
            }
            BlockBody::Tool { title, .. } | BlockBody::Permission { title, .. } => title.to_string(),
            BlockBody::ToolDetail(shown) => format!("{:?}", shown.part),
        })
        .collect()
}

/// The plan Brindle would make on a Linux machine named `thishost`.
pub(super) fn linux(uri: &str) -> Plan {
    crate::terminal::links::plan(uri, "thishost", false)
}

/// An entry's blocks, rendered as the model renders them.
pub(super) fn drawn(id: EntryId, entry: &Entry, open: bool) -> Vec<Block> {
    let rendered = render_entry(entry).map(|(parts, _)| parts).unwrap_or_default();
    blocks_of(id, entry, &rendered, open, ToolDraw::default())
}

pub(super) const EVERY_HOLD: [HoldReason; 9] = [
    HoldReason::Cancelled,
    HoldReason::Failed,
    HoldReason::Stopped { stop: StopReason::MaxTokens },
    HoldReason::Stopped { stop: StopReason::MaxTurnRequests },
    HoldReason::Stopped { stop: StopReason::Refusal },
    HoldReason::Stopped { stop: StopReason::Cancelled },
    HoldReason::LoginRequired,
    HoldReason::AgentStopped,
    HoldReason::OutputLost,
];

pub(super) fn every_phase() -> Vec<AgentPhase> {
    vec![
        AgentPhase::Connecting,
        AgentPhase::NotStarted(StartFailure::NoServer("no helper".into())),
        AgentPhase::NotStarted(StartFailure::TooOld { endpoint: "A8".into() }),
        AgentPhase::NotStarted(StartFailure::Refused { command: "fake".into(), cause: "not found".into() }),
        AgentPhase::Live(AgentState::Starting),
        AgentPhase::Live(AgentState::Ready { name: None, version: None }),
        AgentPhase::Live(AgentState::LoginRequired { message: "log in".into(), methods: Vec::new() }),
        AgentPhase::Live(AgentState::Failed { message: "bad".into() }),
        AgentPhase::Live(AgentState::Exited { code: Some(1), stderr: "boom".into() }),
        AgentPhase::Live(AgentState::Lost { message: "its holder ended".into() }),
        AgentPhase::Lost { reason: "gone".into() },
        AgentPhase::Broken { reason: "bad event".into() },
    ]
}

// `agents-session-recovery` section 4: continuing a conversation.

pub(super) fn conversation(continuation: crate::agent::transcript::Continuation) -> Conversation {
    Conversation { id: "sess-1".into(), cwd: "/home/u/src".into(), continuation, title: None }
}

/// The conversation `sess-1` as the core sends it when the agent
/// titles it, or clears its title for `None`.
pub(super) fn titled(title: Option<&str>) -> Event {
    let conversation =
        Conversation { title: title.map(str::to_owned), ..conversation(crate::agent::transcript::Continuation::Resume) };
    Event::Conversation { conversation }
}

pub(super) fn conversation_event(continuation: crate::agent::transcript::Continuation) -> Event {
    Event::Conversation { conversation: conversation(continuation) }
}

// OpenSpec change `agents-log-paging`, tasks 5.3 and 5.4: the window,
// the Earlier row's fetches and the page fold.

/// A server's log: the agent ready, the conversation titled, a user's
/// message, then `count` agent messages of 20 KiB, a thought among
/// them, the turn ended.
pub(super) fn long_log(count: u64) -> Transcript {
    let mut log = Transcript::new();
    let mut events = vec![
        ready(),
        Event::Conversation { conversation: Conversation { title: Some("Long talk".into()), ..conversation(crate::agent::transcript::Continuation::Resume) } },
        added(1, user("go")),
        turn(TurnState::Working),
    ];
    for n in 0..count {
        let entry = if n == 3 { thought("# Thinking\n\nabout it") } else { agent(&format!("**{n}** {}", "x".repeat(20 * 1024))) };
        events.push(added(n + 2, entry));
    }
    events.push(turn(TurnState::Ended { stop: StopReason::EndTurn }));
    for event in &events {
        log.apply(event).unwrap();
    }
    log
}

/// A zone attached to `server`'s log with its newest page.
pub(super) fn paged(server: &Transcript) -> Zone {
    let mut zone = Zone::new(None);
    zone.replay(&server.page(PAGE_BYTES));
    zone
}

/// The server's answer to the zone's fetch, folded as the sink folds it
/// (the page first), then the reply.
pub(super) fn answer_fetch(zone: &mut Zone, server: &Transcript, fetch: Fetch, request: RequestId) -> Absorbed {
    let fetched = server.fetch(fetch.before, fetch.max_bytes);
    let values: Vec<_> = fetched.events.iter().map(Event::to_value).collect();
    zone.shared.prepend(&values, fetched.more).unwrap();
    let absorbed = zone.absorb();
    assert_eq!(zone.model.replied(request, Ok(())), Replied::Nothing);
    absorbed
}

pub(super) fn held_slot() -> Slot {
    held(HoldReason::Stopped { stop: StopReason::MaxTokens })
}

/// A ready zone whose agent offers `/compact` and `/context`, a mode
/// (auto, plan), a model (opus, sonnet) and a toggle.
pub(super) fn with_lists() -> Zone {
    use crate::agent::transcript::OptionChoice;
    let mut zone = Zone::ready();
    let command = |name: &str, hint: Option<&str>| SlashCommand {
        name: name.into(),
        description: format!("about {name}"),
        hint: hint.map(Into::into),
    };
    let choice =
        |value: &str, name: &str| OptionChoice { value: value.into(), name: name.into(), description: None, group: None };
    let select = |id: &str, name: &str, category, current: &str, choices| SessionOption {
        id: id.into(),
        name: name.into(),
        description: None,
        category,
        kind: OptionControl::Select { current: current.into(), choices },
    };
    let options = vec![
        select("mode", "Mode", OptionCategory::Mode, "auto", vec![choice("auto", "Auto"), choice("plan", "Plan")]),
        select("model", "Model", OptionCategory::Model, "opus", vec![choice("opus", "Opus"), choice("sonnet", "Sonnet")]),
        SessionOption {
            id: "fast".into(),
            name: "Fast \"mode\"".into(),
            description: None,
            category: OptionCategory::Other { name: "speed".into() },
            kind: OptionControl::Toggle { on: false },
        },
    ];
    zone.feed(&[
        Event::Commands { commands: vec![command("compact", Some("<how>")), command("context", None)] },
        Event::Options { options },
    ]);
    zone
}

/// Entry `id`'s cached parts.
pub(super) fn parts_of(model: &ChatModel, id: u64) -> Vec<Part> {
    model.rendered.get(&EntryId(id)).cloned().unwrap_or_default()
}

/// The model's blocks are what a fresh model fed the same entries
/// whole would draw, and its cache holds exactly the agent messages and
/// thoughts, each rendered from its whole text.
pub(super) fn in_step(model: &ChatModel) {
    let expected: Vec<Block> =
        model.entries.iter().flat_map(|(id, entry)| drawn(*id, entry, model.thought_open(*id))).collect();
    assert_eq!(model.blocks, expected);
    let markdown: Vec<EntryId> = model
        .entries
        .iter()
        .filter(|(_, entry)| matches!(entry, Entry::Agent { .. } | Entry::Thought { .. }))
        .map(|(id, _)| *id)
        .collect();
    assert_eq!(model.rendered.keys().copied().collect::<Vec<_>>(), markdown);
    for (id, entry) in &model.entries {
        if let Entry::Agent { text } | Entry::Thought { text } = entry {
            assert_eq!(model.rendered[id], markdown::render(text), "entry {id}");
        }
    }
}

pub(super) fn diff_item(path: &str) -> ToolItem {
    ToolItem::Diff(FileChange {
        op: FileOp::Modify,
        path: path.into(),
        hunks: vec![Hunk {
            old_start: 10,
            new_start: 10,
            lines: vec![
                DiffLine::Context("fn main() {".into()),
                DiffLine::Removed("    old();".into()),
                DiffLine::Added("    new();".into()),
                DiffLine::Context("}".into()),
            ],
        }],
        omitted: 0,
    })
}

pub(super) fn tool_entry(call: &str, status: ToolStatus, content: Vec<ToolItem>) -> Entry {
    let Entry::Tool { call, title, kind, .. } = Entry::tool(call.into(), format!("Edit {call}"), ToolKind::Edit, status)
    else {
        unreachable!()
    };
    Entry::Tool { call, title, kind, status, content, more: 0, locations: Vec::new(), input: None, result: None }
}

/// The drawn state of tool entry `id`'s row, and its count of blocks.
pub(super) fn row(model: &ChatModel, id: u64) -> (RowState, usize) {
    let range = model.entry_blocks(EntryId(id));
    let BlockBody::Tool { state, .. } = &model.blocks()[range.start].body else { panic!("{:?}", model.blocks()) };
    (*state, range.len())
}

pub(super) fn in_progress(call: &str, content: Vec<ToolItem>) -> Entry {
    tool_entry(call, ToolStatus::InProgress, content)
}

/// The zone's blocks are what drawing every entry afresh gives: the
/// splices kept the list right.
pub(super) fn assert_drawn_afresh(model: &ChatModel) {
    let fresh: Vec<Block> =
        model.entries.iter().flat_map(|(id, entry)| model.draw(*id, entry, model.thought_open(*id))).collect();
    assert_eq!(model.blocks(), fresh.as_slice());
}

// `agents-session-list` 4.3: the choosing screen.

pub(super) fn choosing(message: Option<&str>) -> Event {
    agent_state(AgentState::Choosing { message: message.map(Into::into) })
}
