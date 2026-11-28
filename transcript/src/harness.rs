//! Helpers the transcript's unit tests share.

use std::collections::BTreeMap;

use crate::*;

pub(crate) fn added(id: u64, entry: Entry) -> Event {
    Event::Added {
        id: EntryId(id),
        entry,
    }
}

pub(crate) fn agent(text: &str) -> Entry {
    Entry::Agent { text: text.into() }
}

pub(crate) fn tool(call: &str) -> Entry {
    Entry::tool(
        call.into(),
        "Read notes.md".into(),
        ToolKind::Read,
        ToolStatus::Pending,
    )
}

/// A `ToolChanged` of the title, kind and status alone.
pub(crate) fn changed(
    id: u64,
    title: Option<&str>,
    kind: Option<ToolKind>,
    status: Option<ToolStatus>,
) -> Event {
    Event::ToolChanged {
        id: EntryId(id),
        title: title.map(Into::into),
        kind,
        status,
        locations: None,
        input: None,
        result: None,
        more: None,
    }
}

pub(crate) fn permission() -> Entry {
    Entry::Permission {
        call: Some("call_1".into()),
        title: "Write notes.md".into(),
        options: vec![
            PermissionOption {
                id: "allow-once".into(),
                name: "Allow".into(),
                kind: PermissionKind::AllowOnce,
            },
            PermissionOption {
                id: "reject-once".into(),
                name: "Reject".into(),
                kind: PermissionKind::RejectOnce,
            },
        ],
        outcome: None,
    }
}

pub(crate) fn fold(events: &[Event]) -> Result<Transcript, FoldError> {
    let mut transcript = Transcript::new();
    for event in events {
        transcript.apply(event)?;
    }
    Ok(transcript)
}

pub(crate) fn conversation(id: &str, continuation: Continuation) -> Conversation {
    Conversation {
        id: id.into(),
        cwd: "/home/u/src".into(),
        continuation,
        title: None,
    }
}

pub(crate) fn with(agent: AgentState, turn: TurnState) -> Transcript {
    fold(&[Event::Agent { state: agent }, Event::Turn { state: turn }]).unwrap()
}

pub(crate) fn ready() -> AgentState {
    AgentState::Ready {
        name: Some("fake".into()),
        version: None,
    }
}

pub(crate) fn ended(stop: StopReason) -> TurnState {
    TurnState::Ended { stop }
}

pub(crate) fn agent_method(id: &str, name: &str) -> LoginMethod {
    LoginMethod {
        id: id.into(),
        name: name.into(),
        description: None,
        kind: LoginKind::Agent,
    }
}

pub(crate) fn terminal_method() -> LoginMethod {
    LoginMethod {
        id: "cli-login".into(),
        name: "Log in from the terminal".into(),
        description: Some("Paste a code".into()),
        kind: LoginKind::Terminal {
            args: vec!["login".into()],
            env: BTreeMap::from([("LOGIN_MODE".into(), "device".into())]),
        },
    }
}

// Paging (OpenSpec change `agents-log-paging`, design decisions 3, 4).

/// A log of `count` agent entries of about `size` bytes each, then a
/// ready agent.
pub(crate) fn long(count: u64, size: usize) -> Transcript {
    let mut events: Vec<Event> = (1..=count)
        .map(|id| added(id, agent(&"x".repeat(size))))
        .collect();
    events.push(Event::Agent { state: ready() });
    fold(&events).unwrap()
}

pub(crate) fn window(from: u64, earlier: u64) -> Event {
    Event::Window {
        from: EntryId(from),
        earlier,
    }
}

pub(crate) fn appended(id: u64, text: &str) -> Event {
    Event::Appended {
        id: EntryId(id),
        text: text.into(),
    }
}

/// A copy paged from entry 5 of `long(8, 10)`.
pub(crate) fn paged() -> Transcript {
    let server = long(8, 10);
    let copy = fold(&server.replay_from(4)).unwrap();
    assert_eq!(
        copy.window(),
        Some(Window {
            from: EntryId(5),
            earlier: 4
        })
    );
    copy
}

// Tool content (OpenSpec change `agents-tool-content`).

pub(crate) fn text_item(text: &str) -> ToolItem {
    ToolItem::Text {
        text: text.into(),
        cut: false,
    }
}

pub(crate) fn output_item(command: Option<&str>) -> ToolItem {
    ToolItem::Output(Output {
        command: command.map(Into::into),
        ..Output::default()
    })
}

pub(crate) fn diff_item() -> ToolItem {
    ToolItem::Diff(FileChange {
        op: FileOp::Modify,
        path: "/p/main.rs".into(),
        hunks: vec![Hunk {
            old_start: 10,
            new_start: 10,
            lines: vec![
                DiffLine::Context("fn main() {".into()),
                DiffLine::Removed("    old();".into()),
                DiffLine::Added("    new();".into()),
            ],
        }],
        omitted: 0,
    })
}

/// `tool(call)` with this content and raw input.
pub(crate) fn tool_with(call: &str, items: Vec<ToolItem>, raw_input: Option<&str>) -> Entry {
    let mut entry = tool(call);
    if let Entry::Tool { content, input, .. } = &mut entry {
        *content = items;
        *input = raw_input.map(Into::into);
    }
    entry
}

pub(crate) fn items(id: u64, from: u32, items: Vec<ToolItem>) -> Event {
    Event::ToolItems {
        id: EntryId(id),
        from,
        items,
    }
}

pub(crate) fn output(id: u64, item: u32, text: &str, exit: Option<Exit>) -> Event {
    Event::ToolOutput {
        id: EntryId(id),
        item,
        text: text.into(),
        exit,
    }
}

pub(crate) fn content_of(transcript: &Transcript, id: u64) -> Vec<ToolItem> {
    match transcript.entry(EntryId(id)) {
        Some(Entry::Tool { content, .. }) => content.clone(),
        other => panic!("entry {id} is {other:?}"),
    }
}

/// The log's measure computed afresh: each entry's encoding and its
/// framing.
pub(crate) fn measured(transcript: &Transcript) -> usize {
    transcript
        .entries()
        .map(|(_, entry)| serde_json::to_vec(entry).unwrap().len() + ENTRY_FRAMING)
        .sum()
}

pub(crate) fn compact() -> SlashCommand {
    SlashCommand {
        name: "compact".into(),
        description: "Free up context".into(),
        hint: Some("<instructions>".into()),
    }
}

pub(crate) fn mode_option(current: &str) -> SessionOption {
    SessionOption {
        id: "mode".into(),
        name: "Mode".into(),
        description: None,
        category: OptionCategory::Mode,
        kind: OptionControl::Select {
            current: current.into(),
            choices: vec![
                OptionChoice {
                    value: "ask".into(),
                    name: "Ask".into(),
                    description: Some("Asks first".into()),
                    group: None,
                },
                OptionChoice {
                    value: "code".into(),
                    name: "Code".into(),
                    description: None,
                    group: Some("Edits".into()),
                },
            ],
        },
    }
}

pub(crate) fn fast_option() -> SessionOption {
    SessionOption {
        id: "fast".into(),
        name: "Fast mode".into(),
        description: Some("Speed".into()),
        category: OptionCategory::Other {
            name: "speed".into(),
        },
        kind: OptionControl::Toggle { on: false },
    }
}

pub(crate) fn lines_diff(lines: &[&str], hunks: usize) -> ToolItem {
    ToolItem::Diff(FileChange {
        op: FileOp::Move { from: "/a".into() },
        path: "/b/c".into(),
        hunks: (0..hunks)
            .map(|n| Hunk {
                old_start: 1 + n as u32 * 100,
                new_start: 1 + n as u32 * 100,
                lines: lines.iter().map(|line| DiffLine::Added((*line).into())).collect(),
            })
            .collect(),
        omitted: 0,
    })
}
