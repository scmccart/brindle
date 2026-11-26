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
            call: text(rng),
            title: text(rng),
            kind: TOOL_KINDS[rng.below(TOOL_KINDS.len())],
            status: TOOL_STATUSES[rng.below(TOOL_STATUSES.len())],
        },
        4 => Entry::Permission {
            call: rng.chance(2).then(|| text(rng)),
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
    match rng.below(7) {
        0 => AgentState::Starting,
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
    }
}

/// Any event, with ids near the log's, valid or not.
fn event(rng: &mut XorShift, near: u64) -> Event {
    let id = EntryId(near.saturating_sub(3) + rng.below(6) as u64);
    match rng.below(9) {
        0 => Event::Added {
            id,
            entry: entry(rng),
        },
        1 => Event::Appended {
            id,
            text: text(rng),
        },
        2 => Event::ToolChanged {
            id,
            title: rng.chance(2).then(|| text(rng)),
            kind: rng
                .chance(2)
                .then(|| TOOL_KINDS[rng.below(TOOL_KINDS.len())]),
            status: rng
                .chance(2)
                .then(|| TOOL_STATUSES[rng.below(TOOL_STATUSES.len())]),
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
        _ => Event::Trimmed {
            through: id,
            dropped: rng.below(4) as u64,
        },
    }
}

/// A valid log, built only from events that apply: entries of every kind,
/// appends, tool changes, answers, trims, and every kind of slot.
pub(crate) fn random_transcript(rng: &mut XorShift) -> Transcript {
    let mut transcript = Transcript::new();
    for _ in 0..rng.below(40) {
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
                let Some((id, _)) = ids.iter().rev().find(|(_, entry)| {
                    matches!(
                        entry,
                        Entry::User { .. } | Entry::Agent { .. } | Entry::Thought { .. }
                    )
                }) else {
                    continue;
                };
                Event::Appended {
                    id: *id,
                    text: text(rng),
                }
            }
            4 => {
                let Some((id, _)) = ids
                    .iter()
                    .find(|(_, entry)| matches!(entry, Entry::Tool { .. }))
                else {
                    continue;
                };
                Event::ToolChanged {
                    id: *id,
                    title: rng.chance(2).then(|| text(rng)),
                    kind: rng
                        .chance(2)
                        .then(|| TOOL_KINDS[rng.below(TOOL_KINDS.len())]),
                    status: rng
                        .chance(2)
                        .then(|| TOOL_STATUSES[rng.below(TOOL_STATUSES.len())]),
                }
            }
            5 => {
                let Some((id, _)) = ids
                    .iter()
                    .find(|(_, entry)| matches!(entry, Entry::Permission { outcome: None, .. }))
                else {
                    continue;
                };
                Event::Answered {
                    id: *id,
                    outcome: outcome(rng),
                }
            }
            6 => Event::Queue { slot: slot(rng) },
            7 => match rng.below(3) {
                0 => Event::Turn { state: turn(rng) },
                1 => Event::Agent { state: agent(rng) },
                _ => Event::Conversation {
                    conversation: conversation(rng),
                },
            },
            _ => {
                let limit = transcript.encoded_len() / (1 + rng.below(4));
                transcript.trim_to(limit);
                continue;
            }
        };
        assert_eq!(transcript.apply(&event), Ok(()), "{event:?}");
    }
    transcript
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
