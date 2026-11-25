//! Pins client protocol 4's transcript event forms (design decision 6):
//! every event kind and every value of the closed enums inside an event,
//! each in a fixture of `tests/fixtures/v4/`.
//!
//! - `fixtures_decode_to_their_events` reads each fixture and compares it
//!   with its event below.
//! - `encoding_the_events_reproduces_the_fixtures` encodes each event and
//!   compares it with its fixture, byte for byte, in the fixtures' form:
//!   compact JSON with every object's keys sorted, so the bytes don't depend
//!   on whether a build turns on serde_json's `preserve_order`.
//! - `every_form_appears_in_a_fixture` checks that every variant of the
//!   event's enums is in some fixture.
//! - `an_unknown_field_anywhere_is_ignored`,
//!   `an_unknown_kind_or_value_is_a_decode_error`: the rules a protocol 4
//!   client relies on.
//!
//! A fixture is frozen once it has shipped (process R5): a new form gets a
//! new fixture, and the old ones keep passing byte for byte.
//! `BRINDLE_REGEN_FIXTURES=1 cargo test -p brindle-transcript --test events`
//! writes only fixtures that don't exist yet; it never rewrites one. To
//! rework a fixture that never shipped, delete it and regenerate.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Once;

use brindle_transcript::{
    AgentState, Entry, EntryId, Event, HoldReason, NoticeLevel, PermissionKind, PermissionOption,
    PermissionOutcome, Slot, StopReason, ToolKind, ToolStatus, TurnState,
};
use serde_json::Value;

/// Declares an enum's forms: a function naming the form of a value, from a
/// match over every variant with no `_ =>` arm (railway R3), and the list
/// of every name. A new variant fails to compile here until it is named,
/// and once named it must appear in a fixture.
macro_rules! forms {
    ($name:ident, $all:ident, $ty:ty, $prefix:literal: $($variant:ident),+ $(,)?) => {
        fn $name(value: &$ty) -> &'static str {
            type Enum = $ty;
            match value {
                $(Enum::$variant { .. } => concat!($prefix, stringify!($variant)),)+
            }
        }
        const $all: &[&str] = &[$(concat!($prefix, stringify!($variant))),+];
    };
}

forms!(event_form, EVENT_FORMS, Event, "Event::":
    Added, Appended, ToolChanged, Answered, Queue, Turn, Agent, Trimmed);
forms!(entry_form, ENTRY_FORMS, Entry, "Entry::": User, Agent, Thought, Tool, Permission, Notice);
forms!(tool_kind_form, TOOL_KIND_FORMS, ToolKind, "ToolKind::":
    Read, Edit, Delete, Move, Search, Execute, Think, Fetch, SwitchMode, Other);
forms!(tool_status_form, TOOL_STATUS_FORMS, ToolStatus, "ToolStatus::":
    Pending, InProgress, Completed, Failed);
forms!(notice_level_form, NOTICE_LEVEL_FORMS, NoticeLevel, "NoticeLevel::": Info, Warning, Error);
forms!(permission_kind_form, PERMISSION_KIND_FORMS, PermissionKind, "PermissionKind::":
    AllowOnce, AllowAlways, RejectOnce, RejectAlways);
forms!(outcome_form, OUTCOME_FORMS, PermissionOutcome, "PermissionOutcome::": Selected, Cancelled);
forms!(slot_form, SLOT_FORMS, Slot, "Slot::": Empty, Waiting, Held);
forms!(hold_form, HOLD_FORMS, HoldReason, "HoldReason::":
    Cancelled, Failed, Stopped, LoginRequired, AgentStopped, OutputLost);
forms!(turn_form, TURN_FORMS, TurnState, "TurnState::":
    Idle, Working, Blocked, Cancelling, Ended, Failed);
forms!(stop_form, STOP_FORMS, StopReason, "StopReason::":
    EndTurn, MaxTokens, MaxTurnRequests, Refusal, Cancelled, Other);
forms!(agent_form, AGENT_FORMS, AgentState, "AgentState::":
    Starting, Ready, LoginRequired, Failed, Exited);

fn expected_forms() -> Vec<&'static str> {
    [
        EVENT_FORMS,
        ENTRY_FORMS,
        TOOL_KIND_FORMS,
        TOOL_STATUS_FORMS,
        NOTICE_LEVEL_FORMS,
        PERMISSION_KIND_FORMS,
        OUTCOME_FORMS,
        SLOT_FORMS,
        HOLD_FORMS,
        TURN_FORMS,
        STOP_FORMS,
        AGENT_FORMS,
    ]
    .concat()
}

fn stop_forms(stop: &StopReason, forms: &mut Vec<&'static str>) {
    forms.push(stop_form(stop));
}

fn entry_forms(entry: &Entry, forms: &mut Vec<&'static str>) {
    forms.push(entry_form(entry));
    match entry {
        Entry::Tool { kind, status, .. } => {
            forms.push(tool_kind_form(kind));
            forms.push(tool_status_form(status));
        }
        Entry::Permission {
            options, outcome, ..
        } => {
            forms.extend(
                options
                    .iter()
                    .map(|option| permission_kind_form(&option.kind)),
            );
            forms.extend(outcome.iter().map(outcome_form));
        }
        Entry::Notice { level, .. } => forms.push(notice_level_form(level)),
        Entry::User { .. } | Entry::Agent { .. } | Entry::Thought { .. } => {}
    }
}

/// The event's form and the forms of every enum value inside it.
fn forms(event: &Event) -> Vec<&'static str> {
    let mut forms = vec![event_form(event)];
    match event {
        Event::Added { entry, .. } => entry_forms(entry, &mut forms),
        Event::ToolChanged { kind, status, .. } => {
            forms.extend(kind.iter().map(tool_kind_form));
            forms.extend(status.iter().map(tool_status_form));
        }
        Event::Answered { outcome, .. } => forms.push(outcome_form(outcome)),
        Event::Queue { slot } => {
            forms.push(slot_form(slot));
            if let Slot::Held { reason, .. } = slot {
                forms.push(hold_form(reason));
                if let HoldReason::Stopped { stop } = reason {
                    stop_forms(stop, &mut forms);
                }
            }
        }
        Event::Turn { state } => {
            forms.push(turn_form(state));
            if let TurnState::Ended { stop } = state {
                stop_forms(stop, &mut forms);
            }
        }
        Event::Agent { state } => forms.push(agent_form(state)),
        Event::Appended { .. } | Event::Trimmed { .. } => {}
    }
    forms
}

// The fixtures.

fn id(n: u64) -> EntryId {
    EntryId(n)
}

fn added(n: u64, entry: Entry) -> Event {
    Event::Added { id: id(n), entry }
}

fn tool_changed(kind: ToolKind, status: ToolStatus) -> Event {
    Event::ToolChanged {
        id: id(4),
        title: None,
        kind: Some(kind),
        status: Some(status),
    }
}

fn held(reason: HoldReason) -> Event {
    Event::Queue {
        slot: Slot::Held {
            text: "and then the tests".into(),
            reason,
        },
    }
}

fn turn(state: TurnState) -> Event {
    Event::Turn { state }
}

fn ended(stop: StopReason) -> Event {
    turn(TurnState::Ended { stop })
}

fn agent(state: AgentState) -> Event {
    Event::Agent { state }
}

fn option(id: &str, name: &str, kind: PermissionKind) -> PermissionOption {
    PermissionOption {
        id: id.into(),
        name: name.into(),
        kind,
    }
}

/// Every fixture: its name and its event.
fn fixtures() -> Vec<(&'static str, Event)> {
    vec![
        (
            "added-user",
            added(
                1,
                Entry::User {
                    text: "Write notes.md\nwith a \"title\"\tand é".into(),
                },
            ),
        ),
        ("added-agent", added(2, Entry::Agent { text: "Hel".into() })),
        (
            "appended",
            Event::Appended {
                id: id(2),
                text: "lo!".into(),
            },
        ),
        (
            "added-thought",
            added(
                3,
                Entry::Thought {
                    text: "The user wants a file.".into(),
                },
            ),
        ),
        (
            "added-tool",
            added(
                4,
                Entry::Tool {
                    call: "call_1".into(),
                    title: "Write notes.md".into(),
                    kind: ToolKind::Edit,
                    status: ToolStatus::Pending,
                },
            ),
        ),
        (
            "added-permission",
            added(
                5,
                Entry::Permission {
                    call: Some("call_1".into()),
                    title: "Write notes.md".into(),
                    options: vec![
                        option("allow-once", "Allow", PermissionKind::AllowOnce),
                        option("allow-always", "Always allow", PermissionKind::AllowAlways),
                        option("reject-once", "Reject", PermissionKind::RejectOnce),
                        option(
                            "reject-always",
                            "Always reject",
                            PermissionKind::RejectAlways,
                        ),
                    ],
                    outcome: None,
                },
            ),
        ),
        (
            "added-permission-answered",
            added(
                6,
                Entry::Permission {
                    call: None,
                    title: "Run make".into(),
                    options: vec![option("allow-once", "Allow", PermissionKind::AllowOnce)],
                    outcome: Some(PermissionOutcome::Selected {
                        option: "allow-once".into(),
                    }),
                },
            ),
        ),
        (
            "added-notice-info",
            added(
                7,
                Entry::Notice {
                    level: NoticeLevel::Info,
                    text: "2 lines were not ACP messages".into(),
                },
            ),
        ),
        (
            "added-notice-warning",
            added(
                8,
                Entry::Notice {
                    level: NoticeLevel::Warning,
                    text: "output was lost while the terminal server restarted".into(),
                },
            ),
        ),
        (
            "added-notice-error",
            added(
                9,
                Entry::Notice {
                    level: NoticeLevel::Error,
                    text: "fatal: no model".into(),
                },
            ),
        ),
        (
            "tool-changed",
            Event::ToolChanged {
                id: id(4),
                title: Some("Write notes.md (3 lines)".into()),
                kind: Some(ToolKind::Edit),
                status: Some(ToolStatus::InProgress),
            },
        ),
        (
            "tool-changed-nothing",
            Event::ToolChanged {
                id: id(4),
                title: None,
                kind: None,
                status: None,
            },
        ),
        (
            "tool-changed-read",
            tool_changed(ToolKind::Read, ToolStatus::Pending),
        ),
        (
            "tool-changed-delete",
            tool_changed(ToolKind::Delete, ToolStatus::InProgress),
        ),
        (
            "tool-changed-move",
            tool_changed(ToolKind::Move, ToolStatus::Completed),
        ),
        (
            "tool-changed-search",
            tool_changed(ToolKind::Search, ToolStatus::Failed),
        ),
        (
            "tool-changed-execute",
            tool_changed(ToolKind::Execute, ToolStatus::Pending),
        ),
        (
            "tool-changed-think",
            tool_changed(ToolKind::Think, ToolStatus::InProgress),
        ),
        (
            "tool-changed-fetch",
            tool_changed(ToolKind::Fetch, ToolStatus::Completed),
        ),
        (
            "tool-changed-switch-mode",
            tool_changed(ToolKind::SwitchMode, ToolStatus::Failed),
        ),
        (
            "tool-changed-other",
            tool_changed(ToolKind::Other, ToolStatus::Completed),
        ),
        (
            "answered-selected",
            Event::Answered {
                id: id(5),
                outcome: PermissionOutcome::Selected {
                    option: "reject-once".into(),
                },
            },
        ),
        (
            "answered-cancelled",
            Event::Answered {
                id: id(5),
                outcome: PermissionOutcome::Cancelled,
            },
        ),
        ("queue-empty", Event::Queue { slot: Slot::Empty }),
        (
            "queue-waiting",
            Event::Queue {
                slot: Slot::Waiting {
                    text: "and then the tests".into(),
                },
            },
        ),
        ("queue-held-cancelled", held(HoldReason::Cancelled)),
        ("queue-held-failed", held(HoldReason::Failed)),
        (
            "queue-held-stopped",
            held(HoldReason::Stopped {
                stop: StopReason::MaxTokens,
            }),
        ),
        ("queue-held-login-required", held(HoldReason::LoginRequired)),
        ("queue-held-agent-stopped", held(HoldReason::AgentStopped)),
        ("queue-held-output-lost", held(HoldReason::OutputLost)),
        ("turn-idle", turn(TurnState::Idle)),
        ("turn-working", turn(TurnState::Working)),
        ("turn-blocked", turn(TurnState::Blocked)),
        ("turn-cancelling", turn(TurnState::Cancelling)),
        ("turn-ended-end-turn", ended(StopReason::EndTurn)),
        ("turn-ended-max-tokens", ended(StopReason::MaxTokens)),
        (
            "turn-ended-max-turn-requests",
            ended(StopReason::MaxTurnRequests),
        ),
        ("turn-ended-refusal", ended(StopReason::Refusal)),
        ("turn-ended-cancelled", ended(StopReason::Cancelled)),
        (
            "turn-ended-other",
            ended(StopReason::Other {
                name: "context_full".into(),
            }),
        ),
        (
            "turn-failed",
            turn(TurnState::Failed {
                message: "rate limited".into(),
            }),
        ),
        ("agent-starting", agent(AgentState::Starting)),
        (
            "agent-ready",
            agent(AgentState::Ready {
                name: Some("fake-acp-agent".into()),
                version: Some("1.0.0".into()),
            }),
        ),
        (
            "agent-ready-unnamed",
            agent(AgentState::Ready {
                name: None,
                version: None,
            }),
        ),
        (
            "agent-login-required",
            agent(AgentState::LoginRequired {
                message: "run agent login".into(),
            }),
        ),
        (
            "agent-failed",
            agent(AgentState::Failed {
                message: "the agent supports ACP versions 2; Brindle speaks 1".into(),
            }),
        ),
        (
            "agent-exited",
            agent(AgentState::Exited {
                code: Some(1),
                stderr: "fatal: no model\n".into(),
            }),
        ),
        (
            "agent-exited-by-signal",
            agent(AgentState::Exited {
                code: None,
                stderr: String::new(),
            }),
        ),
        (
            "trimmed",
            Event::Trimmed {
                through: id(40),
                dropped: 40,
            },
        ),
    ]
}

// The fixture files.

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/v4")
        .join(format!("{name}.json"))
}

/// Compact JSON with every object's keys sorted, then a newline.
fn canonical(value: &Value) -> String {
    fn write(value: &Value, out: &mut String) {
        match value {
            Value::Object(object) => {
                let mut keys: Vec<&String> = object.keys().collect();
                keys.sort();
                out.push('{');
                for (index, key) in keys.into_iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    out.push_str(&Value::String(key.clone()).to_string());
                    out.push(':');
                    write(&object[key], out);
                }
                out.push('}');
            }
            Value::Array(items) => {
                out.push('[');
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    write(item, out);
                }
                out.push(']');
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
                out.push_str(&value.to_string())
            }
        }
    }
    let mut out = String::new();
    write(value, &mut out);
    out.push('\n');
    out
}

static REGENERATE: Once = Once::new();

const FROZEN: &str = "shipped fixtures are frozen (process R5): add a new fixture instead, \
     or, if this fixture never shipped, delete it and regenerate with BRINDLE_REGEN_FIXTURES=1";

/// Every fixture with its file's text. With `BRINDLE_REGEN_FIXTURES=1`, the
/// files that don't exist yet are written first; an existing one is never
/// rewritten.
fn read_fixtures() -> Vec<(&'static str, Event, String)> {
    let fixtures = fixtures();
    REGENERATE.call_once(|| {
        if std::env::var_os("BRINDLE_REGEN_FIXTURES").is_some_and(|v| v == "1") {
            for (name, event) in &fixtures {
                let path = fixture_path(name);
                if path.exists() {
                    continue;
                }
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, canonical(&event.to_value())).unwrap();
            }
        }
    });
    fixtures
        .into_iter()
        .map(|(name, event)| {
            let path = fixture_path(name);
            let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
                panic!(
                    "{}: {e}; regenerate with BRINDLE_REGEN_FIXTURES=1",
                    path.display()
                )
            });
            (name, event, text)
        })
        .collect()
}

#[test]
fn fixture_names_are_unique() {
    let names: Vec<&str> = fixtures().iter().map(|(name, _)| *name).collect();
    let unique: BTreeSet<&str> = names.iter().copied().collect();
    assert_eq!(unique.len(), names.len(), "a fixture name repeats");
}

#[test]
fn fixtures_decode_to_their_events() {
    for (name, event, text) in read_fixtures() {
        let value: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(Event::from_value(&value), Ok(event), "{name}");
    }
}

#[test]
fn encoding_the_events_reproduces_the_fixtures() {
    for (name, event, text) in read_fixtures() {
        assert_eq!(
            canonical(&event.to_value()),
            text,
            "{name}: the encoding changed; {FROZEN}"
        );
    }
}

#[test]
fn every_form_appears_in_a_fixture() {
    let seen: BTreeSet<&str> = fixtures()
        .iter()
        .flat_map(|(_, event)| forms(event))
        .collect();
    let missing: Vec<&str> = expected_forms()
        .into_iter()
        .filter(|form| !seen.contains(form))
        .collect();
    assert!(missing.is_empty(), "no fixture has {}", missing.join(", "));
}

/// `value` with a field it doesn't know added to every object in it.
fn with_unknown_fields(value: &Value) -> Value {
    match value {
        Value::Object(object) => {
            let mut object: serde_json::Map<String, Value> = object
                .iter()
                .map(|(key, inner)| (key.clone(), with_unknown_fields(inner)))
                .collect();
            object.insert(
                "zz_later".into(),
                serde_json::json!({"added": [1, null], "in": "v4"}),
            );
            Value::Object(object)
        }
        Value::Array(items) => Value::Array(items.iter().map(with_unknown_fields).collect()),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => value.clone(),
    }
}

#[test]
fn an_unknown_field_anywhere_is_ignored() {
    for (name, event, text) in read_fixtures() {
        let value: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            Event::from_value(&with_unknown_fields(&value)),
            Ok(event),
            "{name}"
        );
    }
}

/// The fields whose strings are a closed enum's values: the tags and the
/// fieldless enums. Every other string is free text.
const ENUM_FIELDS: [&str; 11] = [
    "event", "type", "turn", "agent", "slot", "hold", "reason", "outcome", "kind", "status",
    "level",
];

/// Each copy of `value` with one enum-valued string replaced by `unknown`.
fn with_one_unknown_value(value: &Value, unknown: &str) -> Vec<Value> {
    let mut copies = Vec::new();
    match value {
        Value::Object(object) => {
            for (key, inner) in object {
                if ENUM_FIELDS.contains(&key.as_str()) && inner.is_string() {
                    let mut copy = object.clone();
                    copy.insert(key.clone(), Value::String(unknown.into()));
                    copies.push(Value::Object(copy));
                }
                for changed in with_one_unknown_value(inner, unknown) {
                    let mut copy = object.clone();
                    copy.insert(key.clone(), changed);
                    copies.push(Value::Object(copy));
                }
            }
        }
        Value::Array(items) => {
            for (index, item) in items.iter().enumerate() {
                for changed in with_one_unknown_value(item, unknown) {
                    let mut copy = items.clone();
                    copy[index] = changed;
                    copies.push(Value::Array(copy));
                }
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
    copies
}

#[test]
fn an_unknown_kind_or_value_is_a_decode_error() {
    let mut tried = 0usize;
    for (name, _, text) in read_fixtures() {
        let value: Value = serde_json::from_str(&text).unwrap();
        for unknown in ["zz_later", "", "Added", "end-turn"] {
            for changed in with_one_unknown_value(&value, unknown) {
                assert!(
                    Event::from_value(&changed).is_err(),
                    "{name}: {changed} decoded"
                );
                tried += 1;
            }
        }
    }
    // Every enum the fixtures hold was tried, the event kinds included.
    assert!(tried > 4 * fixtures().len(), "only {tried} values tried");
    assert!(
        Event::from_value(&serde_json::json!({"event": "later", "id": 1})).is_err(),
        "an unknown event kind"
    );
}
