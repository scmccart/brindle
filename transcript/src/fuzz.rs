//! A dependency-free fuzz test of the decoder and the fold: a fixed-seed
//! xorshift generator feeds random and mutated JSON to [`Event::from_value`]
//! and random events, valid or not, to [`Transcript::apply`]. Neither may
//! panic, and a refused event leaves the log as it was. It also builds the
//! random valid logs `lib.rs`'s replay property runs over.

use serde_json::{Value, json};

use crate::*;

pub(crate) struct XorShift(u64);

impl XorShift {
    pub(crate) fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    pub(crate) fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }

    pub(crate) fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }

    fn chance(&mut self, one_in: usize) -> bool {
        self.below(one_in) == 0
    }
}

/// Characters that stress the encoding: quotes, escapes, control
/// characters, multibyte text.
const CHARS: &[char] = &[
    'a', 'z', ' ', '"', '\\', '\n', '\t', '\r', '\u{1}', '\u{7f}', 'é', '€', '🦀', '\u{fffd}',
];

fn text(rng: &mut XorShift) -> String {
    // Now and then past a chunk, so replays split it.
    let len = if rng.chance(40) {
        TEXT_CHUNK + rng.below(TEXT_CHUNK)
    } else {
        rng.below(12)
    };
    if len > 64 {
        let piece: String = (0..16).map(|_| CHARS[rng.below(CHARS.len())]).collect();
        return piece.repeat(len / piece.len().max(1) + 1);
    }
    (0..len).map(|_| CHARS[rng.below(CHARS.len())]).collect()
}

const TOOL_KINDS: [ToolKind; 10] = [
    ToolKind::Read,
    ToolKind::Edit,
    ToolKind::Delete,
    ToolKind::Move,
    ToolKind::Search,
    ToolKind::Execute,
    ToolKind::Think,
    ToolKind::Fetch,
    ToolKind::SwitchMode,
    ToolKind::Other,
];
const TOOL_STATUSES: [ToolStatus; 4] = [
    ToolStatus::Pending,
    ToolStatus::InProgress,
    ToolStatus::Completed,
    ToolStatus::Failed,
];
const PERMISSION_KINDS: [PermissionKind; 4] = [
    PermissionKind::AllowOnce,
    PermissionKind::AllowAlways,
    PermissionKind::RejectOnce,
    PermissionKind::RejectAlways,
];
const NOTICE_LEVELS: [NoticeLevel; 3] =
    [NoticeLevel::Info, NoticeLevel::Warning, NoticeLevel::Error];

fn stop(rng: &mut XorShift) -> StopReason {
    match rng.below(6) {
        0 => StopReason::EndTurn,
        1 => StopReason::MaxTokens,
        2 => StopReason::MaxTurnRequests,
        3 => StopReason::Refusal,
        4 => StopReason::Cancelled,
        _ => StopReason::Other { name: text(rng) },
    }
}

fn outcome(rng: &mut XorShift) -> PermissionOutcome {
    if rng.chance(2) {
        PermissionOutcome::Cancelled
    } else {
        PermissionOutcome::Selected { option: text(rng) }
    }
}

fn entry(rng: &mut XorShift) -> Entry {
    match rng.below(6) {
        0 => Entry::User { text: text(rng) },
        1 => Entry::Agent { text: text(rng) },
        2 => Entry::Thought { text: text(rng) },
        3 => Entry::Tool {
            // A few calls, so permissions name them.
            call: CALLS[rng.below(CALLS.len())].into(),
            title: text(rng),
            kind: TOOL_KINDS[rng.below(TOOL_KINDS.len())],
            status: TOOL_STATUSES[rng.below(TOOL_STATUSES.len())],
            content: (0..rng.below(4)).map(|_| tool_item(rng)).collect(),
            more: rng.below(3) as u32,
            locations: locations(rng),
            input: rng.chance(2).then(|| text(rng)),
            result: rng.chance(3).then(|| text(rng)),
        },
        4 => Entry::Permission {
            call: rng
                .chance(2)
                .then(|| CALLS[rng.below(CALLS.len())].to_string()),
            title: text(rng),
            options: (0..rng.below(4))
                .map(|_| PermissionOption {
                    id: text(rng),
                    name: text(rng),
                    kind: PERMISSION_KINDS[rng.below(PERMISSION_KINDS.len())],
                })
                .collect(),
            outcome: rng.chance(3).then(|| outcome(rng)),
        },
        _ => Entry::Notice {
            level: NOTICE_LEVELS[rng.below(NOTICE_LEVELS.len())],
            text: text(rng),
        },
    }
}

const CALLS: [&str; 3] = ["call_1", "call_2", "call \"3\""];

fn diff_line(rng: &mut XorShift) -> DiffLine {
    match rng.below(3) {
        0 => DiffLine::Context(text(rng)),
        1 => DiffLine::Added(text(rng)),
        _ => DiffLine::Removed(text(rng)),
    }
}

fn file_op(rng: &mut XorShift) -> FileOp {
    match rng.below(6) {
        0 => FileOp::Add,
        1 => FileOp::Delete,
        2 => FileOp::Modify,
        3 => FileOp::Move { from: text(rng) },
        4 => FileOp::Copy { from: text(rng) },
        _ => FileOp::Other { name: text(rng) },
    }
}

fn exit(rng: &mut XorShift) -> Exit {
    if rng.chance(2) {
        Exit::Code {
            code: rng.next() as i32,
        }
    } else {
        Exit::Signal { name: text(rng) }
    }
}

/// Output text with lines, sometimes past [`ITEM_LIMIT`] so the fold drops
/// its start.
fn output_text(rng: &mut XorShift) -> String {
    if rng.chance(6) {
        let line: String = (0..rng.below(200)).map(|_| CHARS[rng.below(CHARS.len())]).collect();
        let mut text = String::new();
        while text.len() < ITEM_LIMIT + rng.below(ITEM_LIMIT) {
            text.push_str(&line);
            if rng.chance(3) {
                text.push('\n');
            }
        }
        return text;
    }
    text(rng)
}

/// A hunk's start, as the producer makes one (from 1), so the logs built
/// from these items encode and decode; the starts the decoder refuses are
/// `a_hunk_near_the_edges_decodes_numbered_within_u32_or_is_refused`'s.
fn hunk_start(rng: &mut XorShift) -> u32 {
    1 + rng.below(100) as u32
}

pub(crate) fn tool_item(rng: &mut XorShift) -> ToolItem {
    match rng.below(3) {
        0 => ToolItem::Text {
            text: text(rng),
            cut: rng.chance(4),
        },
        1 => ToolItem::Diff(FileChange {
            op: file_op(rng),
            path: text(rng),
            hunks: (0..rng.below(3))
                .map(|_| Hunk {
                    old_start: hunk_start(rng),
                    new_start: hunk_start(rng),
                    lines: (0..rng.below(5)).map(|_| diff_line(rng)).collect(),
                })
                .collect(),
            omitted: rng.below(3) as u32,
        }),
        _ => {
            // An output as the fold leaves one: start empty, append.
            let mut output = Output {
                command: rng.chance(2).then(|| text(rng)),
                cwd: rng.chance(2).then(|| text(rng)),
                ..Output::default()
            };
            output.append(&output_text(rng));
            output.exit = rng.chance(2).then(|| exit(rng));
            ToolItem::Output(output)
        }
    }
}

fn locations(rng: &mut XorShift) -> Vec<Location> {
    (0..rng.below(3))
        .map(|_| Location {
            path: text(rng),
            line: rng.chance(2).then(|| rng.below(1000) as u32),
        })
        .collect()
}

fn slot(rng: &mut XorShift) -> Slot {
    match rng.below(3) {
        0 => Slot::Empty,
        1 => Slot::Waiting { text: text(rng) },
        _ => Slot::Held {
            text: text(rng),
            reason: match rng.below(6) {
                0 => HoldReason::Cancelled,
                1 => HoldReason::Failed,
                2 => HoldReason::Stopped { stop: stop(rng) },
                3 => HoldReason::LoginRequired,
                4 => HoldReason::AgentStopped,
                _ => HoldReason::OutputLost,
            },
        },
    }
}

fn turn(rng: &mut XorShift) -> TurnState {
    match rng.below(6) {
        0 => TurnState::Idle,
        1 => TurnState::Working,
        2 => TurnState::Blocked,
        3 => TurnState::Cancelling,
        4 => TurnState::Ended { stop: stop(rng) },
        _ => TurnState::Failed { message: text(rng) },
    }
}

fn login_method(rng: &mut XorShift) -> LoginMethod {
    LoginMethod {
        id: text(rng),
        name: text(rng),
        description: rng.chance(2).then(|| text(rng)),
        kind: if rng.chance(2) {
            LoginKind::Agent
        } else {
            LoginKind::Terminal {
                args: (0..rng.below(3)).map(|_| text(rng)).collect(),
                env: (0..rng.below(3)).map(|_| (text(rng), text(rng))).collect(),
            }
        },
    }
}

fn agent(rng: &mut XorShift) -> AgentState {
    match rng.below(8) {
        0 => AgentState::Starting,
        6 => AgentState::Choosing {
            message: rng.chance(2).then(|| text(rng)),
        },
        1 => AgentState::Ready {
            name: rng.chance(2).then(|| text(rng)),
            version: rng.chance(2).then(|| text(rng)),
        },
        2 => AgentState::LoginRequired {
            message: text(rng),
            methods: (0..rng.below(4)).map(|_| login_method(rng)).collect(),
        },
        3 => AgentState::Failed { message: text(rng) },
        4 => AgentState::Lost { message: text(rng) },
        5 => AgentState::LoggingIn {
            method: text(rng),
            name: text(rng),
        },
        _ => AgentState::Exited {
            code: rng.chance(2).then(|| rng.next() as i32),
            stderr: text(rng),
        },
    }
}

const CONTINUATIONS: [Continuation; 4] = [
    Continuation::ResumeAndLoad,
    Continuation::Resume,
    Continuation::Load,
    Continuation::Neither,
];

fn conversation(rng: &mut XorShift) -> Conversation {
    Conversation {
        id: text(rng),
        cwd: text(rng),
        continuation: CONTINUATIONS[rng.below(CONTINUATIONS.len())],
        title: rng.chance(2).then(|| text(rng)),
    }
}

fn short_text(rng: &mut XorShift) -> String {
    (0..rng.below(6))
        .map(|_| CHARS[rng.below(CHARS.len())])
        .collect()
}

fn commands(rng: &mut XorShift) -> Vec<SlashCommand> {
    (0..rng.below(4))
        .map(|_| SlashCommand {
            name: short_text(rng),
            description: short_text(rng),
            hint: rng.chance(2).then(|| short_text(rng)),
        })
        .collect()
}

pub(crate) fn session_option(rng: &mut XorShift) -> SessionOption {
    let category = match rng.below(5) {
        0 => OptionCategory::Mode,
        1 => OptionCategory::Model,
        2 => OptionCategory::ModelConfig,
        3 => OptionCategory::ThoughtLevel,
        _ => OptionCategory::Other {
            name: short_text(rng),
        },
    };
    let kind = if rng.chance(3) {
        OptionControl::Toggle { on: rng.chance(2) }
    } else {
        OptionControl::Select {
            current: short_text(rng),
            choices: (0..rng.below(4))
                .map(|_| OptionChoice {
                    value: short_text(rng),
                    name: short_text(rng),
                    description: rng.chance(2).then(|| short_text(rng)),
                    group: rng.chance(2).then(|| short_text(rng)),
                })
                .collect(),
        }
    };
    SessionOption {
        id: short_text(rng),
        name: short_text(rng),
        description: rng.chance(2).then(|| short_text(rng)),
        category,
        kind,
    }
}

fn options(rng: &mut XorShift) -> Vec<SessionOption> {
    (0..rng.below(4)).map(|_| session_option(rng)).collect()
}

fn tool_changed(rng: &mut XorShift, id: EntryId) -> Event {
    Event::ToolChanged {
        id,
        title: rng.chance(2).then(|| text(rng)),
        kind: rng
            .chance(2)
            .then(|| TOOL_KINDS[rng.below(TOOL_KINDS.len())]),
        status: rng
            .chance(2)
            .then(|| TOOL_STATUSES[rng.below(TOOL_STATUSES.len())]),
        locations: rng.chance(3).then(|| locations(rng)),
        input: rng.chance(3).then(|| rng.chance(2).then(|| text(rng))),
        result: rng.chance(3).then(|| rng.chance(2).then(|| text(rng))),
        more: rng.chance(4).then(|| rng.below(3) as u32),
    }
}

/// Any event, with ids near the log's, valid or not.
fn event(rng: &mut XorShift, near: u64) -> Event {
    let id = EntryId(near.saturating_sub(3) + rng.below(6) as u64);
    match rng.below(15) {
        0 => Event::Added {
            id,
            entry: entry(rng),
        },
        1 => Event::Appended {
            id,
            text: text(rng),
        },
        2 => tool_changed(rng, id),
        10 => Event::ToolItems {
            id,
            from: rng.below(5) as u32,
            items: (0..rng.below(3)).map(|_| tool_item(rng)).collect(),
        },
        11 => Event::ToolOutput {
            id,
            item: rng.below(4) as u32,
            text: output_text(rng),
            exit: rng.chance(3).then(|| exit(rng)),
        },
        3 => Event::Answered {
            id,
            outcome: outcome(rng),
        },
        4 => Event::Queue { slot: slot(rng) },
        5 => Event::Turn { state: turn(rng) },
        6 => Event::Agent { state: agent(rng) },
        7 => Event::Conversation {
            conversation: conversation(rng),
        },
        8 => Event::Trimmed {
            through: id,
            dropped: rng.below(4) as u64,
        },
        12 => Event::Commands {
            commands: commands(rng),
        },
        13 => Event::Options {
            options: options(rng),
        },
        _ => Event::Window {
            from: id,
            earlier: rng.below(4) as u64,
        },
    }
}

/// Any event, valid or not, near the first ids.
pub(crate) fn any_event(rng: &mut XorShift) -> Event {
    event(rng, 3)
}

/// A valid log, built only from events that apply: entries of every kind,
/// appends, tool changes, answers, trims, and every kind of slot.
pub(crate) fn random_transcript(rng: &mut XorShift) -> Transcript {
    let mut transcript = Transcript::new();
    for _ in 0..rng.below(40) {
        step(rng, &mut transcript);
    }
    transcript
}

/// One random valid change to a log, applied, and the event that made it
/// (a trim's `Trimmed` included); `None` when the change drawn had nothing
/// to act on.
fn step(rng: &mut XorShift, transcript: &mut Transcript) -> Option<Event> {
    let ids: Vec<(EntryId, Entry)> = transcript
        .entries()
        .map(|(id, entry)| (id, entry.clone()))
        .collect();
    let event = match rng.below(9) {
        0..=2 => Event::Added {
            id: transcript.next_id(),
            entry: entry(rng),
        },
        3 => {
            let (id, _) = ids.iter().rev().find(|(_, entry)| {
                matches!(
                    entry,
                    Entry::User { .. } | Entry::Agent { .. } | Entry::Thought { .. }
                )
            })?;
            Event::Appended {
                id: *id,
                text: text(rng),
            }
        }
        4 => {
            let tools: Vec<&(EntryId, Entry)> = ids
                .iter()
                .filter(|(_, entry)| matches!(entry, Entry::Tool { .. }))
                .collect();
            let (id, entry) = tools.get(rng.below(tools.len()))?;
            let Entry::Tool { content, .. } = entry else {
                return None;
            };
            let outputs: Vec<usize> = content
                .iter()
                .enumerate()
                .filter(|(_, item)| matches!(item, ToolItem::Output(_)))
                .map(|(index, _)| index)
                .collect();
            match rng.below(3) {
                0 => tool_changed(rng, *id),
                1 => Event::ToolItems {
                    id: *id,
                    from: rng.below(content.len() + 1) as u32,
                    items: (0..rng.below(3)).map(|_| tool_item(rng)).collect(),
                },
                _ => Event::ToolOutput {
                    id: *id,
                    item: *outputs.get(rng.below(outputs.len()))? as u32,
                    text: output_text(rng),
                    exit: rng.chance(3).then(|| exit(rng)),
                },
            }
        }
        5 => {
            let (id, _) = ids
                .iter()
                .find(|(_, entry)| matches!(entry, Entry::Permission { outcome: None, .. }))?;
            Event::Answered {
                id: *id,
                outcome: outcome(rng),
            }
        }
        6 => Event::Queue { slot: slot(rng) },
        7 => match rng.below(5) {
            0 => Event::Turn { state: turn(rng) },
            1 => Event::Agent { state: agent(rng) },
            2 => Event::Commands {
                commands: commands(rng),
            },
            3 => Event::Options {
                options: options(rng),
            },
            _ => Event::Conversation {
                conversation: conversation(rng),
            },
        },
        _ => {
            let limit = transcript.encoded_len() / (1 + rng.below(4));
            return transcript.trim_to(limit);
        }
    };
    assert_eq!(transcript.apply(&event), Ok(()), "{event:?}");
    Some(event)
}

/// Field names and tag values of the encoding, so random objects reach the
/// decoder's branches.
const WORDS: &[&str] = &[
    "event",
    "added",
    "appended",
    "tool_changed",
    "answered",
    "queue",
    "turn",
    "agent",
    "trimmed",
    "id",
    "entry",
    "text",
    "title",
    "kind",
    "status",
    "outcome",
    "slot",
    "state",
    "through",
    "dropped",
    "type",
    "user",
    "thought",
    "tool",
    "permission",
    "notice",
    "call",
    "options",
    "name",
    "level",
    "waiting",
    "held",
    "reason",
    "hold",
    "stop",
    "ended",
    "failed",
    "message",
    "ready",
    "exited",
    "code",
    "stderr",
    "selected",
    "cancelled",
    "conversation",
    "continuation",
    "cwd",
    "resume_and_load",
    "resume",
    "load",
    "neither",
    "lost",
    "option",
    "end_turn",
    "other",
    "read",
    "pending",
    "info",
    "allow_once",
    "login_required",
    "logging_in",
    "methods",
    "method",
    "kind",
    "terminal",
    "args",
    "env",
    "description",
    "window",
    "from",
    "earlier",
    "tool_items",
    "tool_output",
    "item",
    "items",
    "diff",
    "output",
    "hunks",
    "old_start",
    "new_start",
    "lines",
    " a",
    "+b",
    "-c",
    "op",
    "modify",
    "move",
    "path",
    "omitted",
    "partial",
    "exit",
    "signal",
    "locations",
    "line",
    "input",
    "result",
    "more",
    "cut",
    "content",
    "commands",
    "hint",
    "category",
    "control",
    "select",
    "toggle",
    "current",
    "choices",
    "value",
    "group",
    "on",
    "mode",
    "model",
    "model_config",
    "thought_level",
];

fn random_json(rng: &mut XorShift, depth: usize) -> Value {
    match rng.below(if depth == 0 { 5 } else { 7 }) {
        0 => Value::Null,
        1 => Value::Bool(rng.chance(2)),
        2 => match rng.below(4) {
            0 => json!(-1 - rng.below(1000) as i64),
            1 => json!(u64::MAX - rng.below(2) as u64),
            2 => json!(1.5),
            _ => json!(rng.below(10)),
        },
        3 | 4 => Value::String(if rng.chance(3) {
            text(rng)
        } else {
            WORDS[rng.below(WORDS.len())].to_string()
        }),
        5 => Value::Array(
            (0..rng.below(4))
                .map(|_| random_json(rng, depth - 1))
                .collect(),
        ),
        _ => Value::Object(
            (0..rng.below(6))
                .map(|_| {
                    (
                        WORDS[rng.below(WORDS.len())].to_string(),
                        random_json(rng, depth - 1),
                    )
                })
                .collect(),
        ),
    }
}

/// A valid event's value with one field somewhere replaced, dropped or
/// added.
fn mutated(rng: &mut XorShift, value: &mut Value) {
    match value {
        Value::Object(object) if !object.is_empty() && rng.chance(2) => {
            let keys: Vec<String> = object.keys().cloned().collect();
            let key = &keys[rng.below(keys.len())];
            if let Some(inner) = object.get_mut(key)
                && rng.chance(2)
            {
                mutated(rng, inner);
                return;
            }
            match rng.below(3) {
                0 => {
                    object.remove(key);
                }
                1 => {
                    object.insert(key.clone(), random_json(rng, 2));
                }
                _ => {
                    object.insert(
                        WORDS[rng.below(WORDS.len())].to_string(),
                        random_json(rng, 2),
                    );
                }
            }
        }
        _ => *value = random_json(rng, 2),
    }
}

#[test]
fn the_decoder_never_panics_on_random_or_mutated_values() {
    let mut rng = XorShift::new(0x7e57_0001);
    let mut decoded = 0usize;
    let mut refused = 0usize;
    for _ in 0..6000 {
        let value = if rng.chance(3) {
            random_json(&mut rng, 4)
        } else {
            let mut value = event(&mut rng, 3).to_value();
            mutated(&mut rng, &mut value);
            value
        };
        match Event::from_value(&value) {
            Ok(event) => {
                // What decodes encodes and decodes to itself.
                assert_eq!(Event::from_value(&event.to_value()), Ok(event));
                decoded += 1;
            }
            Err(error) => {
                assert!(!error.to_string().is_empty());
                refused += 1;
            }
        }
    }
    assert!(
        decoded > 0 && refused > 0,
        "{decoded} decoded, {refused} refused"
    );
}

#[test]
fn the_fold_never_panics_and_a_refused_event_changes_nothing() {
    let mut rng = XorShift::new(0x7e57_0002);
    let mut applied = 0usize;
    let mut refused = 0usize;
    for _ in 0..300 {
        let mut transcript = random_transcript(&mut rng);
        for _ in 0..40 {
            let event = event(&mut rng, transcript.next_id().0);
            let before = transcript.clone();
            match transcript.apply(&event) {
                Ok(()) => applied += 1,
                Err(error) => {
                    assert_eq!(transcript, before, "{error}");
                    refused += 1;
                }
            }
            if rng.chance(8) {
                transcript.trim_to(rng.below(4096));
            }
            if rng.chance(6) {
                // A fetched page of arbitrary events, more or not.
                let page: Vec<Event> = (0..rng.below(5))
                    .map(|_| self::event(&mut rng, transcript.next_id().0.saturating_sub(4)))
                    .collect();
                let before = transcript.clone();
                match transcript.prepend(&page, rng.chance(2)) {
                    Ok(()) => applied += 1,
                    Err(error) => {
                        assert_eq!(transcript, before, "{error}");
                        refused += 1;
                    }
                }
            }
            let _ = transcript.activity();
        }
        assert_eq!(
            {
                let mut rebuilt = Transcript::new();
                for event in transcript.replay() {
                    rebuilt.apply(&event).unwrap();
                }
                rebuilt
            },
            transcript
        );
    }
    assert!(
        applied > 0 && refused > 0,
        "{applied} applied, {refused} refused"
    );
}

// The paging laws (OpenSpec change `agents-log-paging`, design decision 4),
// each over at least 1,000 random logs.

const LOGS: usize = 1200;

/// Budgets from nothing to past the log: 0, a random part of it,
/// `PAGE_BYTES` and the whole log.
fn budgets(rng: &mut XorShift, log: &Transcript) -> [usize; 4] {
    [
        0,
        1 + rng.below(log.encoded_len().max(1)),
        PAGE_BYTES,
        log.encoded_len() + 1,
    ]
}

/// A copy folded from `events`, which must fold.
fn folded(events: &[Event]) -> Transcript {
    let mut copy = Transcript::new();
    for event in events {
        assert_eq!(copy.apply(event), Ok(()), "{event:?}");
    }
    copy
}

/// Where a copy's held entries start, for [`Transcript::suffix`]: its
/// window's `from`, or the start of time when it holds everything.
fn held_from(copy: &Transcript) -> EntryId {
    copy.window().map_or(EntryId(0), |window| window.from)
}

/// Fetches back from the server until nothing more remains, folding each
/// page in front, with random budgets.
fn fetch_to_start(rng: &mut XorShift, server: &Transcript, copy: &mut Transcript) {
    while let Some(window) = copy.window() {
        let budget = budgets(rng, server)[rng.below(4)];
        let fetched = server.fetch(window.from, budget);
        assert_eq!(copy.prepend(&fetched.events, fetched.more), Ok(()));
        assert_eq!(copy, &server.suffix(held_from(copy)));
    }
}

#[test]
fn a_page_folds_to_the_logs_suffix_and_holds_what_it_must() {
    let mut rng = XorShift::new(0x7e57_0003);
    let mut paged = 0usize;
    let mut tools = 0usize;
    let mut listed = 0usize;
    for _ in 0..LOGS {
        let log = random_transcript(&mut rng);
        for budget in budgets(&mut rng, &log) {
            let page = log.page(budget);
            let copy = folded(&page);
            let from = held_from(&copy);
            assert_eq!(copy, log.suffix(from));
            paged += usize::from(copy.window().is_some());
            // Every pending permission, the tool entry each names and the
            // newest entry are held.
            let mut lowest = None;
            for (id, entry) in log.entries() {
                if let Entry::Permission {
                    outcome: None,
                    call,
                    ..
                } = entry
                {
                    assert!(copy.entry(id).is_some(), "pending permission {id}");
                    lowest = lowest.min(Some(id)).or(Some(id));
                    let tool = call.as_ref().and_then(|call| {
                        log.entries()
                            .take_while(|(before, _)| *before < id)
                            .filter(|(_, entry)| {
                                matches!(entry, Entry::Tool { call: own, .. } if own == call)
                            })
                            .last()
                            .map(|(tool, _)| tool)
                    });
                    if let Some(tool) = tool {
                        assert!(copy.entry(tool).is_some(), "permission {id}'s tool {tool}");
                        lowest = lowest.min(Some(tool));
                        tools += 1;
                    }
                }
            }
            assert_eq!(
                copy.entries().last().map(|(id, _)| id),
                log.entries().last().map(|(id, _)| id)
            );
            // The state events are all there (the equality above holds
            // them too), and the entries fit unless one alone or the
            // oldest pending permission pushed them past.
            assert_eq!(
                (copy.slot(), copy.turn(), copy.agent(), copy.conversation()),
                (log.slot(), log.turn(), log.agent(), log.conversation())
            );
            assert_eq!(
                (copy.commands(), copy.options()),
                (log.commands(), log.options())
            );
            listed += usize::from(!log.commands().is_empty() && !log.options().is_empty());
            if copy.encoded_len() > budget {
                assert!(
                    copy.entries().count() == 1 || copy.entries().next().map(|(id, _)| id) == lowest,
                    "a page past its budget of {budget}"
                );
            }
        }
    }
    assert!(paged > LOGS / 2, "only {paged} pages had a window");
    assert!(tools > LOGS / 20, "only {tools} pending permissions named a tool entry");
    assert!(listed > 0, "no paged log held both commands and options");
}

#[test]
fn fetched_pages_folded_in_front_give_the_whole_log() {
    let mut rng = XorShift::new(0x7e57_0004);
    for _ in 0..LOGS {
        let log = random_transcript(&mut rng);
        let budget = budgets(&mut rng, &log)[rng.below(2)];
        let mut copy = folded(&log.page(budget));
        fetch_to_start(&mut rng, &log, &mut copy);
        assert_eq!(copy, log);
    }
}

#[test]
fn live_events_after_a_page_keep_the_copy_equal_to_the_servers_suffix() {
    let mut rng = XorShift::new(0x7e57_0005);
    for _ in 0..LOGS {
        let mut log = random_transcript(&mut rng);
        let budget = budgets(&mut rng, &log)[rng.below(2)];
        let mut copy = folded(&log.page(budget));
        let mut from = held_from(&copy);
        for _ in 0..rng.below(12) {
            let Some(event) = step(&mut rng, &mut log) else {
                continue;
            };
            assert_eq!(copy.apply(&event), Ok(()), "{event:?}");
            // The floor moves only by a trim, and only by taking the window.
            if matches!(event, Event::Trimmed { .. }) && held_from(&copy) != from {
                assert_eq!(copy.window(), None, "{event:?}");
                from = held_from(&copy);
            }
            assert_eq!(held_from(&copy), from, "{event:?}");
            assert_eq!(copy, log.suffix(held_from(&copy)), "{event:?}");
        }
        // A fetch after them returns the entries as the log holds them now.
        fetch_to_start(&mut rng, &log, &mut copy);
        assert_eq!(copy, log);
    }
}

#[test]
fn a_trim_between_fetches_never_makes_a_fold_error() {
    let mut rng = XorShift::new(0x7e57_0006);
    let mut raced = 0usize;
    for _ in 0..LOGS {
        let mut log = random_transcript(&mut rng);
        let mut copy = folded(&log.page(rng.below(log.encoded_len().max(1))));
        while let Some(window) = copy.window() {
            let budget = budgets(&mut rng, &log)[rng.below(3)];
            // Through the floor, past it, or anywhere below it.
            let limit = log.encoded_len() / (1 + rng.below(6));
            if rng.chance(2) {
                // The core handled the fetch first: its page arrives, then
                // the trim.
                let fetched = log.fetch(window.from, budget);
                assert_eq!(copy.prepend(&fetched.events, fetched.more), Ok(()));
                if let Some(trimmed) = log.trim_to(limit) {
                    assert_eq!(copy.apply(&trimmed), Ok(()));
                    raced += 1;
                }
            } else {
                // The trim came first: the fetch, sent before it, reads
                // the trimmed log.
                if let Some(trimmed) = log.trim_to(limit) {
                    assert_eq!(copy.apply(&trimmed), Ok(()));
                    raced += 1;
                }
                let fetched = log.fetch(window.from, budget);
                assert_eq!(copy.prepend(&fetched.events, fetched.more), Ok(()));
            }
            assert_eq!(copy, log.suffix(held_from(&copy)));
        }
        assert_eq!(copy, log);
    }
    assert!(raced > LOGS / 2, "only {raced} trims raced a fetch");
}

// The fold's clamps (OpenSpec change `agents-event-bounds`, design decision
// 6).

/// A string of [`CHARS`] of `least` bytes and up to `spread` more, or a
/// little past that.
fn long_text(rng: &mut XorShift, least: usize, spread: usize) -> String {
    let len = least + rng.below(spread);
    let piece: String = (0..16).map(|_| CHARS[rng.below(CHARS.len())]).collect();
    piece.repeat(len / piece.len().max(1) + 1)
}

/// A tool item now and then past every bound of its own: a long text, a
/// diff of many short lines with long paths, an output with a long command,
/// directory and signal name.
fn big_tool_item(rng: &mut XorShift) -> ToolItem {
    if rng.chance(2) {
        return tool_item(rng);
    }
    match rng.below(3) {
        0 => ToolItem::Text {
            text: long_text(rng, ITEM_LIMIT, ITEM_LIMIT),
            cut: rng.chance(4),
        },
        1 => {
            let line = DiffLine::Added("x".repeat(rng.below(3)));
            ToolItem::Diff(FileChange {
                op: if rng.chance(2) {
                    FileOp::Move {
                        from: long_text(rng, MAX_PATH, MAX_PATH),
                    }
                } else {
                    FileOp::Add
                },
                path: long_text(rng, 0, 3 * MAX_PATH),
                hunks: (0..1 + rng.below(3))
                    .map(|n| Hunk {
                        old_start: 1 + n as u32,
                        new_start: 1 + n as u32,
                        lines: vec![line.clone(); rng.below(20_000)],
                    })
                    .collect(),
                omitted: rng.below(3) as u32,
            })
        }
        _ => {
            let mut output = Output {
                command: Some(long_text(rng, 0, 2 * TEXT_CHUNK)),
                cwd: Some(long_text(rng, 0, 3 * MAX_PATH)),
                ..Output::default()
            };
            output.append(&output_text(rng));
            output.exit = Some(Exit::Signal {
                name: long_text(rng, 0, 4 * SIGNAL_RESERVE),
            });
            ToolItem::Output(output)
        }
    }
}

fn big_items(rng: &mut XorShift) -> Vec<ToolItem> {
    (0..rng.below(41)).map(|_| big_tool_item(rng)).collect()
}

fn big_locations(rng: &mut XorShift) -> Vec<Location> {
    (0..rng.below(41))
        .map(|_| Location {
            path: long_text(rng, 0, 3 * MAX_PATH),
            line: None,
        })
        .collect()
}

fn big_raw(rng: &mut XorShift) -> Option<String> {
    rng.chance(2)
        .then(|| long_text(rng, 0, 3 * RAW_LIMIT))
}

/// A tool entry's content, locations and raw values past their bounds, now
/// and then.
fn big_tool(rng: &mut XorShift) -> Entry {
    Entry::Tool {
        call: "call_1".into(),
        title: text(rng),
        kind: ToolKind::Edit,
        status: ToolStatus::InProgress,
        content: big_items(rng),
        more: rng.below(3) as u32,
        locations: big_locations(rng),
        input: big_raw(rng),
        result: big_raw(rng),
    }
}

/// Whether a tool entry is within the tool content bounds; any other entry
/// is.
fn within_bounds(entry: &Entry) -> bool {
    let Entry::Tool {
        content,
        locations,
        input,
        result,
        ..
    } = entry
    else {
        return true;
    };
    let total: usize = content.iter().map(ToolItem::wire_len).sum();
    content.len() <= MAX_ITEMS
        && total <= CALL_LIMIT
        && content.iter().all(|item| {
            let mut bounded = item.clone();
            bounded.bound();
            bounded == *item
        })
        && locations.len() <= MAX_LOCATIONS
        && locations.iter().all(|location| location.path.len() <= MAX_PATH)
        && [input, result]
            .iter()
            .all(|raw| raw.as_ref().is_none_or(|raw| raw.len() <= RAW_LIMIT))
}

#[test]
fn the_fold_never_holds_tool_content_past_its_bounds() {
    let mut rng = XorShift::new(0xeb_0101);
    let mut clamped = 0usize;
    for _ in 0..60 {
        let mut transcript = Transcript::new();
        for _ in 0..12 {
            let next = transcript.next_id();
            let tool = transcript
                .entries()
                .filter(|(_, entry)| matches!(entry, Entry::Tool { .. }))
                .map(|(id, _)| id)
                .last();
            let event = match (rng.below(4), tool) {
                (0, _) | (_, None) => {
                    let entry = big_tool(&mut rng);
                    clamped += usize::from(!within_bounds(&entry));
                    Event::Added { id: next, entry }
                }
                (1, Some(id)) => Event::ToolChanged {
                    id,
                    title: None,
                    kind: None,
                    status: None,
                    locations: rng.chance(2).then(|| big_locations(&mut rng)),
                    input: rng.chance(2).then(|| big_raw(&mut rng)),
                    result: rng.chance(2).then(|| big_raw(&mut rng)),
                    more: None,
                },
                (2, Some(id)) => {
                    let len = match transcript.entry(id) {
                        Some(Entry::Tool { content, .. }) => content.len(),
                        _ => 0,
                    };
                    Event::ToolItems {
                        id,
                        from: rng.below(len + 1) as u32,
                        items: big_items(&mut rng),
                    }
                }
                (_, Some(id)) => {
                    let output = match transcript.entry(id) {
                        Some(Entry::Tool { content, .. }) => content
                            .iter()
                            .position(|item| matches!(item, ToolItem::Output(_))),
                        _ => None,
                    };
                    let Some(item) = output else { continue };
                    Event::ToolOutput {
                        id,
                        item: item as u32,
                        text: output_text(&mut rng),
                        exit: Some(Exit::Signal {
                            name: long_text(&mut rng, 4 * SIGNAL_RESERVE, 1),
                        }),
                    }
                }
            };
            assert_eq!(transcript.apply(&event), Ok(()));
            for (id, entry) in transcript.entries() {
                assert!(within_bounds(entry), "entry {id} is past the bounds");
            }
        }
    }
    assert!(clamped > 20, "only {clamped} entries were past the bounds");
}

#[test]
fn clamping_is_the_identity_within_the_bounds_and_idempotent() {
    let mut rng = XorShift::new(0xeb_0102);
    let mut within = 0usize;
    for _ in 0..400 {
        let entry = if rng.chance(2) { big_tool(&mut rng) } else { entry(&mut rng) };
        let mut once = entry.clone();
        once.bound_tool();
        if within_bounds(&entry) {
            assert_eq!(once, entry, "a clamp changed an entry within the bounds");
            within += 1;
        }
        assert!(within_bounds(&once));
        let mut twice = once.clone();
        twice.bound_tool();
        assert_eq!(twice, once, "a clamped entry clamps to something else");
    }
    assert!(within > 50, "only {within} entries were within the bounds");
}

#[test]
fn folding_a_tool_calls_pieces_equals_folding_their_concatenation_past_the_bounds() {
    let mut rng = XorShift::new(0xeb_0103);
    for _ in 0..200 {
        let items = big_items(&mut rng);
        let header = Event::Added {
            id: EntryId(1),
            entry: Entry::tool("call_1".into(), "t".into(), ToolKind::Edit, ToolStatus::Pending),
        };
        let mut whole = Transcript::new();
        whole.apply(&header).unwrap();
        whole
            .apply(&Event::ToolItems {
                id: EntryId(1),
                from: 0,
                items: items.clone(),
            })
            .unwrap();
        let mut pieces = Transcript::new();
        pieces.apply(&header).unwrap();
        let mut rest = &items[..];
        while !rest.is_empty() {
            let (piece, after) = rest.split_at(1 + rng.below(rest.len()));
            let from = match pieces.entry(EntryId(1)) {
                Some(Entry::Tool { content, .. }) => content.len() as u32,
                _ => 0,
            };
            pieces
                .apply(&Event::ToolItems {
                    id: EntryId(1),
                    from,
                    items: piece.to_vec(),
                })
                .unwrap();
            rest = after;
        }
        assert_eq!(pieces, whole);
    }
}

/// A hunk value near 0 or `u32::MAX`, with random lines, decodes to a hunk
/// whose every line number fits, or is a decode error, never a panic.
#[test]
fn a_hunk_near_the_edges_decodes_numbered_within_u32_or_is_refused() {
    let mut rng = XorShift::new(0xeb_0104);
    let signs = [" a", "+b", "-c"];
    let (mut decoded, mut refused) = (0usize, 0usize);
    for _ in 0..3000 {
        let start = |rng: &mut XorShift| -> u64 {
            match rng.below(3) {
                0 => rng.below(3) as u64,
                1 => u64::from(u32::MAX) - rng.below(4) as u64,
                _ => 1 + rng.below(1000) as u64,
            }
        };
        let lines: Vec<&str> = (0..rng.below(6)).map(|_| signs[rng.below(3)]).collect();
        let value = json!({"event": "tool_items", "id": 1, "from": 0, "items": [{"item": "diff",
            "op": "add", "path": "/p", "hunks": [{"old_start": start(&mut rng),
            "new_start": start(&mut rng), "lines": lines}]}]});
        match Event::from_value(&value) {
            Ok(Event::ToolItems { items, .. }) => {
                let [ToolItem::Diff(change)] = &items[..] else {
                    panic!("not one diff")
                };
                for hunk in &change.hunks {
                    let (mut old, mut new) = (Some(hunk.old_start), Some(hunk.new_start));
                    assert!(hunk.old_start > 0 && hunk.new_start > 0);
                    for line in &hunk.lines {
                        let (o, n) = match line {
                            DiffLine::Context(_) => (true, true),
                            DiffLine::Removed(_) => (true, false),
                            DiffLine::Added(_) => (false, true),
                        };
                        if o {
                            assert!(old.is_some(), "an old line past u32::MAX");
                            old = old.and_then(|at| at.checked_add(1));
                        }
                        if n {
                            assert!(new.is_some(), "a new line past u32::MAX");
                            new = new.and_then(|at| at.checked_add(1));
                        }
                    }
                }
                decoded += 1;
            }
            Ok(other) => panic!("{other:?}"),
            Err(_) => refused += 1,
        }
    }
    assert!(decoded > 100 && refused > 100, "{decoded} decoded, {refused} refused");
}
