//! Transcript tests that pin client protocols 1–4, the relay link and H1 on
//! the wire.
//!
//! Each fixture in `tests/fixtures/` is one connection's frames in order, as
//! hex text: `<sender> <type> <length> <payload>` per frame, with a comment
//! above it naming the message and showing a JSON payload as text. Comments
//! and blank lines are ignored when decoding.
//!
//! - `fixtures_decode_to_the_expected_messages` decodes every fixture and
//!   compares it with the messages below.
//! - `encoding_the_messages_reproduces_the_fixtures` encodes the messages
//!   and compares the result with each fixture, byte for byte.
//! - `every_message_variant_and_form_appears_in_a_transcript` checks that
//!   every variant of the five message enums, and the nested forms named in
//!   [`expected_forms`], is in some transcript.
//! - `every_meta_in_the_fixtures_is_read_whole` reads every session meta
//!   the fixtures carry, none of which may be refused.
//!
//! A change to any message's encoding fails the first two. A fixture that
//! has shipped is frozen (process R5): a new message, field or form gets a
//! new transcript, and the old fixtures keep passing byte for byte.
//! `BRINDLE_REGEN_FIXTURES=1 cargo test -p brindle-remote-protocol --test
//! transcripts` writes only fixtures that don't exist yet; it never rewrites
//! one. To rework a fixture that never shipped, delete it and regenerate.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::PathBuf;
use std::sync::Once;

use brindle_remote_protocol::Frame;
use brindle_remote_protocol::client::{self, KillTarget, Policy, VersionRange};
use brindle_remote_protocol::holder::{self, Previous};
use brindle_remote_protocol::relay;
use serde_json::json;

/// Declares an enum's forms: a function naming the form of a value, from a
/// match over every variant with no `_ =>` arm (railway R3), and the list
/// of every name. A new variant fails to compile here until it is named,
/// and once named it must appear in a transcript.
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

forms!(to_core_form, TO_CORE_FORMS, client::ToCore, "client::ToCore::":
    Hello, Spawn, Data, Resize, Close, Attach, Detach, List, Kill, Status, Stop, StepDown,
    Reparent, SetLayout, SpawnAgent, AgentPrompt, AgentCancel, AgentPermission, AgentUnqueue,
    AgentResend, AgentRestart, AgentLogin, AgentFetch, AgentList, AgentChoose, AgentSetOption);
forms!(option_value_form, OPTION_VALUE_FORMS, client::OptionValue, "client::OptionValue::":
    Choice, Toggle);
forms!(to_client_form, TO_CLIENT_FORMS, client::ToClient, "client::ToClient::":
    Welcome, Error, Spawned, Data, Exit, ReplayStart, ReplayEnd, Sessions, ServerStatus, Ok,
    Process, Transcript, AgentPage, AgentConversations);
forms!(to_holder_form, TO_HOLDER_FORMS, holder::ToHolder, "holder::ToHolder::":
    Hello, Spawn, Input, Resize, Hangup, CheckpointBegin, CheckpointChunk, CheckpointEnd, Drain,
    Goodbye, Reparent, Pause, Resume, SetMeta, SpawnPipes, CheckpointAppend);
forms!(holder_to_core_form, HOLDER_TO_CORE_FORMS, holder::ToCore, "holder::ToCore::":
    Welcome, Output, Spawned, SpawnFailed, Exited, Error, CheckpointBegin, CheckpointChunk,
    CheckpointEnd, TailBegin, TailChunk, TailEnd, StderrTail);
forms!(activity_form, ACTIVITY_FORMS, client::AgentActivity, "client::AgentActivity::":
    Idle, Working, Blocked, Done, Error);
forms!(spawn_mode_form, SPAWN_MODE_FORMS, holder::SpawnMode, "holder::SpawnMode::": Pty, Pipes);

/// Whether a permission answer chose an option or answered it cancelled.
fn permission_option_form(answer: &client::AgentPermission) -> &'static str {
    match answer.option {
        Some(_) => "client::AgentPermission.option=Some",
        None => "client::AgentPermission.option=None",
    }
}
const PERMISSION_OPTION_FORMS: &[&str] = &[
    "client::AgentPermission.option=Some",
    "client::AgentPermission.option=None",
];
forms!(relay_form, RELAY_FORMS, relay::RelayMessage, "relay::RelayMessage::":
    Host, Heartbeat, Error);
forms!(policy_form, POLICY_FORMS, Policy, "client::Policy::": Kill, Detach);
forms!(kill_target_form, KILL_TARGET_FORMS, KillTarget, "client::KillTarget::": Session, All);
forms!(previous_form, PREVIOUS_FORMS, Previous, "holder::Previous::": None, Handover, Crash);

/// Whether an `EXIT` says the server lost its session.
fn lost_form(exit: &client::Exit) -> &'static str {
    match exit.lost {
        None => "client::Exit.lost=None",
        Some(_) => "client::Exit.lost=Some",
    }
}
const LOST_FORMS: &[&str] = &["client::Exit.lost=None", "client::Exit.lost=Some"];

/// Whether a held session that exited was ended by a signal.
fn exit_status_form(status: &holder::ExitStatus) -> &'static str {
    match status.code {
        Some(_) => "holder::ExitStatus.code=Some",
        None => "holder::ExitStatus.code=None",
    }
}
const EXIT_STATUS_FORMS: &[&str] = &[
    "holder::ExitStatus.code=Some",
    "holder::ExitStatus.code=None",
];

/// Whether a relay's host frame carries a login environment, and which.
fn login_form(host: &relay::RelayHost) -> &'static str {
    match host.login {
        None => "relay::RelayHost.login=None",
        Some(relay::LoginEnv::Captured(_)) => "relay::RelayHost.login=Captured",
        Some(relay::LoginEnv::Failed(_)) => "relay::RelayHost.login=Failed",
    }
}
const LOGIN_FORMS: &[&str] = &[
    "relay::RelayHost.login=None",
    "relay::RelayHost.login=Captured",
    "relay::RelayHost.login=Failed",
];

/// Whether an `AGENT_PAGE` frame is a fetch's last, and what it says.
fn page_end_form(page: &client::AgentPage) -> &'static str {
    match page.last {
        None => "client::AgentPage.last=None",
        Some(client::PageEnd { more: true }) => "client::AgentPage.last=more",
        Some(client::PageEnd { more: false }) => "client::AgentPage.last=no-more",
    }
}
const PAGE_END_FORMS: &[&str] = &[
    "client::AgentPage.last=None",
    "client::AgentPage.last=more",
    "client::AgentPage.last=no-more",
];

/// Every form some transcript must have.
fn expected_forms() -> Vec<&'static str> {
    [
        TO_CORE_FORMS,
        TO_CLIENT_FORMS,
        TO_HOLDER_FORMS,
        HOLDER_TO_CORE_FORMS,
        RELAY_FORMS,
        LOGIN_FORMS,
        POLICY_FORMS,
        KILL_TARGET_FORMS,
        PREVIOUS_FORMS,
        LOST_FORMS,
        EXIT_STATUS_FORMS,
        ACTIVITY_FORMS,
        SPAWN_MODE_FORMS,
        PERMISSION_OPTION_FORMS,
        PAGE_END_FORMS,
        OPTION_VALUE_FORMS,
    ]
    .concat()
}

/// One frame of a transcript, with its sender.
#[derive(Debug, Clone, PartialEq)]
enum Step {
    Client(client::ToCore),
    CoreToClient(client::ToClient),
    CoreToHolder(holder::ToHolder),
    Holder(holder::ToCore),
    /// A frame of the relay's own, from `brindle-remote relay`.
    Relay(relay::RelayMessage),
}

impl Step {
    fn sender(&self) -> &'static str {
        match self {
            Self::Client(_) => "client",
            Self::CoreToClient(_) | Self::CoreToHolder(_) => "core",
            Self::Holder(_) => "holder",
            Self::Relay(_) => "relay",
        }
    }

    fn encode(&self) -> Frame {
        match self {
            Self::Client(m) => m.encode(),
            Self::CoreToClient(m) => m.encode(),
            Self::CoreToHolder(m) => m.encode(),
            Self::Holder(m) => m.encode(),
            Self::Relay(m) => m.encode(),
        }
        .unwrap()
    }

    fn name(&self) -> String {
        let debug = match self {
            Self::Client(m) => format!("{m:?}"),
            Self::CoreToClient(m) => format!("{m:?}"),
            Self::CoreToHolder(m) => format!("{m:?}"),
            Self::Holder(m) => format!("{m:?}"),
            Self::Relay(m) => format!("Relay{m:?}"),
        };
        debug.split('(').next().unwrap().to_string()
    }

    /// The step's variant, such as `client::ToClient::Data`, and the nested
    /// forms it carries (see [`expected_forms`]).
    fn forms(&self) -> Vec<&'static str> {
        match self {
            Self::Client(message) => {
                let mut forms = vec![to_core_form(message)];
                if let client::ToCore::Spawn(spawn) = message {
                    forms.push(policy_form(&spawn.policy));
                }
                if let client::ToCore::Kill(kill) = message {
                    forms.push(kill_target_form(&kill.target));
                }
                if let client::ToCore::SpawnAgent(spawn) = message {
                    forms.push(policy_form(&spawn.policy));
                }
                if let client::ToCore::AgentPermission(answer) = message {
                    forms.push(permission_option_form(answer));
                }
                if let client::ToCore::AgentSetOption(set) = message {
                    forms.push(option_value_form(&set.value));
                }
                forms
            }
            Self::CoreToClient(message) => {
                let mut forms = vec![to_client_form(message)];
                if let client::ToClient::Exit(exit) = message {
                    forms.push(lost_form(exit));
                }
                if let client::ToClient::AgentPage(page) = message {
                    forms.push(page_end_form(page));
                }
                if let client::ToClient::Sessions(sessions) = message {
                    forms.extend(
                        sessions
                            .sessions
                            .iter()
                            .map(|info| policy_form(&info.policy)),
                    );
                    forms.extend(
                        sessions
                            .sessions
                            .iter()
                            .filter_map(|info| info.agent.as_ref())
                            .map(|agent| activity_form(&agent.activity)),
                    );
                }
                forms
            }
            Self::CoreToHolder(message) => vec![to_holder_form(message)],
            Self::Holder(message) => {
                let mut forms = vec![holder_to_core_form(message)];
                if let holder::ToCore::Welcome(welcome) = message {
                    forms.push(previous_form(&welcome.previous));
                    forms.extend(
                        welcome
                            .sessions
                            .iter()
                            .filter_map(|held| held.exit.as_ref())
                            .map(exit_status_form),
                    );
                    forms.extend(welcome.sessions.iter().map(|held| spawn_mode_form(&held.mode)));
                }
                forms
            }
            Self::Relay(message) => {
                let mut forms = vec![relay_form(message)];
                if let relay::RelayMessage::Host(host) = message {
                    forms.push(login_form(host));
                }
                forms
            }
        }
    }
}

#[derive(Clone, Copy)]
enum Protocol {
    /// Client protocol 1.
    Client,
    /// Client protocol 2: the same frames plus `PROCESS`, and `HELLO`'s
    /// `linger_secs` and `process_info`.
    Client2,
    /// Client protocol 3: protocol 2 plus `SET_LAYOUT`.
    Client3,
    /// Client protocol 4: protocol 3 plus agent sessions and `TRANSCRIPT`.
    Client4,
    /// The link to a `brindle-remote relay`: the client protocol and the
    /// relay's own frames.
    Relay,
    Holder,
}

impl Protocol {
    fn decode(self, sender: &str, frame: &Frame) -> std::io::Result<Step> {
        Ok(match (self, sender) {
            (Self::Client | Self::Client2 | Self::Client3 | Self::Client4 | Self::Relay, "client") => {
                Step::Client(client::ToCore::decode(frame)?)
            }
            (Self::Client | Self::Client2 | Self::Client3 | Self::Client4 | Self::Relay, "core") => {
                Step::CoreToClient(client::ToClient::decode(frame)?)
            }
            (Self::Relay, "relay") => Step::Relay(relay::RelayMessage::decode(frame)?),
            (Self::Holder, "core") => Step::CoreToHolder(holder::ToHolder::decode(frame)?),
            (Self::Holder, "holder") => Step::Holder(holder::ToCore::decode(frame)?),
            _ => panic!("unknown sender {sender:?}"),
        })
    }

    fn title(self) -> &'static str {
        match self {
            Self::Client => "protocol 1 (client <-> core)",
            Self::Client2 => "protocol 2 (client <-> core)",
            Self::Client3 => "protocol 3 (client <-> core)",
            Self::Client4 => "protocol 4 (client <-> core)",
            Self::Relay => "relay link (Brindle <-> brindle-remote relay)",
            Self::Holder => "H1 (core <-> holder)",
        }
    }
}

struct Transcript {
    name: &'static str,
    protocol: Protocol,
    about: &'static str,
    steps: Vec<Step>,
}

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format!("{name}.hex"))
}

/// Shows a payload for a comment: JSON as text, binary as its session id and
/// the rest escaped.
fn describe(name: &str, frame: &Frame) -> String {
    if name.starts_with("Relay") && frame.payload.is_empty() {
        return "(empty)".to_string();
    }
    if frame.payload.first() == Some(&b'{')
        && let Ok(text) = std::str::from_utf8(&frame.payload)
    {
        return text.to_string();
    }
    let (session, rest) = frame.payload.split_at(4.min(frame.payload.len()));
    let session = u32::from_be_bytes(session.try_into().unwrap_or([0; 4]));
    if name.ends_with("End") {
        if let Ok(covers) = <[u8; 8]>::try_from(rest) {
            return format!("session {session}, covers {}", u64::from_be_bytes(covers));
        }
        return format!("session {session}");
    }
    if let (true, Ok(len)) = (name.ends_with("Begin"), <[u8; 8]>::try_from(rest)) {
        return format!("session {session}, len {}", u64::from_be_bytes(len));
    }
    let rest: String = rest
        .iter()
        .flat_map(|b| std::ascii::escape_default(*b))
        .map(char::from)
        .collect();
    format!("session {session} \"{rest}\"")
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().fold(String::new(), |mut out, b| {
        let _ = write!(out, "{b:02x}");
        out
    })
}

fn render(transcript: &Transcript) -> String {
    let mut out = format!(
        "# {}: {}\n# {}\n# Each frame is `<sender> <type> <length> <payload>` in hex. Generated by\n# tests/transcripts.rs with BRINDLE_REGEN_FIXTURES=1; don't edit by hand.\n",
        transcript.protocol.title(),
        transcript.name,
        transcript.about,
    );
    for step in &transcript.steps {
        let frame = step.encode();
        let _ = writeln!(
            out,
            "\n# {} {}",
            step.name(),
            describe(&step.name(), &frame)
        );
        let _ = writeln!(
            out,
            "{} {:02x} {:08x} {}",
            step.sender(),
            frame.msg_type,
            frame.payload.len(),
            hex(&frame.payload)
        );
    }
    out
}

fn parse(transcript: &Transcript, text: &str) -> Vec<Step> {
    let mut steps = Vec::new();
    for (number, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let at = format!("{} line {}", transcript.name, number + 1);
        let (sender, digits) = line
            .split_once(' ')
            .unwrap_or_else(|| panic!("{at}: no sender"));
        let digits: String = digits.chars().filter(|c| !c.is_whitespace()).collect();
        assert!(digits.is_ascii(), "{at}: not hex");
        assert!(digits.len().is_multiple_of(2), "{at}: odd hex length");
        let bytes: Vec<u8> = (0..digits.len())
            .step_by(2)
            .map(|i| {
                u8::from_str_radix(&digits[i..i + 2], 16)
                    .unwrap_or_else(|_| panic!("{at}: bad hex"))
            })
            .collect();
        let mut reader = bytes.as_slice();
        let frame = Frame::read_from(&mut reader).unwrap_or_else(|e| panic!("{at}: {e}"));
        assert!(reader.is_empty(), "{at}: bytes after the frame");
        let step = transcript
            .protocol
            .decode(sender, &frame)
            .unwrap_or_else(|e| panic!("{at}: {e}"));
        steps.push(step);
    }
    steps
}

static REGENERATE: Once = Once::new();

/// Why a fixture that differs from its transcript can't just be rewritten.
const FROZEN: &str = "shipped fixtures are frozen (process R5): add a new transcript instead, \
     or, if this fixture never shipped, delete it and regenerate with BRINDLE_REGEN_FIXTURES=1";

/// Every transcript and its fixture's text. With `BRINDLE_REGEN_FIXTURES=1`,
/// the fixtures that don't exist yet are written first; an existing one is
/// never rewritten.
fn read_fixtures() -> Vec<(Transcript, String)> {
    let transcripts = transcripts();
    REGENERATE.call_once(|| {
        if std::env::var_os("BRINDLE_REGEN_FIXTURES").is_some_and(|v| v == "1") {
            for transcript in &transcripts {
                let path = fixture_path(transcript.name);
                if path.exists() {
                    continue;
                }
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, render(transcript)).unwrap();
            }
        }
    });
    transcripts
        .into_iter()
        .map(|transcript| {
            let path = fixture_path(transcript.name);
            let text = std::fs::read_to_string(&path).unwrap_or_else(|e| {
                panic!(
                    "{}: {e}; regenerate with BRINDLE_REGEN_FIXTURES=1",
                    path.display()
                )
            });
            (transcript, text)
        })
        .collect()
}

#[test]
fn fixtures_decode_to_the_expected_messages() {
    for (transcript, text) in read_fixtures() {
        let decoded = parse(&transcript, &text);
        assert_eq!(
            decoded.len(),
            transcript.steps.len(),
            "{}: frame count",
            transcript.name
        );
        for (index, (got, want)) in decoded.iter().zip(&transcript.steps).enumerate() {
            assert_eq!(got, want, "{} frame {}", transcript.name, index + 1);
        }
    }
}

#[test]
fn encoding_the_messages_reproduces_the_fixtures() {
    for (transcript, text) in read_fixtures() {
        let rendered = render(&transcript);
        for (index, (got, want)) in rendered
            .lines()
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .zip(
                text.lines()
                    .filter(|l| !l.is_empty() && !l.starts_with('#')),
            )
            .enumerate()
        {
            assert_eq!(
                got,
                want,
                "{} frame {}: encoding changed; {FROZEN}",
                transcript.name,
                index + 1
            );
        }
        assert_eq!(
            rendered, text,
            "{}: the fixture differs from its transcript; {FROZEN}",
            transcript.name
        );
    }
}

#[test]
fn every_message_variant_and_form_appears_in_a_transcript() {
    let seen: BTreeSet<&str> = transcripts()
        .iter()
        .flat_map(|transcript| transcript.steps.iter().flat_map(Step::forms))
        .collect();
    let missing: Vec<&str> = expected_forms()
        .into_iter()
        .filter(|form| !seen.contains(form))
        .collect();
    assert!(
        missing.is_empty(),
        "no transcript has {}",
        missing.join(", ")
    );
}

/// Every session meta the fixtures carry (in `SPAWN`, `SET_META` and
/// `WELCOME`'s sessions) is one the next core reads whole: none is refused
/// by `SessionMeta::from_value`.
#[test]
fn every_meta_in_the_fixtures_is_read_whole() {
    let mut read = 0;
    for (transcript, text) in read_fixtures() {
        for step in parse(&transcript, &text) {
            let metas: Vec<serde_json::Value> = match step {
                Step::CoreToHolder(holder::ToHolder::Spawn(spawn)) => vec![spawn.meta],
                Step::CoreToHolder(holder::ToHolder::SpawnPipes(spawn)) => vec![spawn.meta],
                Step::CoreToHolder(holder::ToHolder::SetMeta(update)) => vec![update.meta],
                Step::Holder(holder::ToCore::Welcome(welcome)) => {
                    welcome.sessions.into_iter().map(|held| held.meta).collect()
                }
                _ => Vec::new(),
            };
            for meta in metas {
                let decoded = holder::SessionMeta::from_value(&meta);
                assert!(decoded.is_ok(), "{}: {meta}: {decoded:?}", transcript.name);
                read += 1;
            }
        }
    }
    assert!(read > 0, "the fixtures carry no meta");
}

// The transcripts.

const TOKEN: &str = "6c1f0e9a2b7d4c35";
const KEY: &str = "9f2c41d07a5b3e86c1d4f0a2b7e95c13";

fn c(m: client::ToCore) -> Step {
    Step::Client(m)
}
fn s(m: client::ToClient) -> Step {
    Step::CoreToClient(m)
}
fn ch(m: holder::ToHolder) -> Step {
    Step::CoreToHolder(m)
}
fn h(m: holder::ToCore) -> Step {
    Step::Holder(m)
}

fn hello(token: &str) -> Step {
    c(client::ToCore::Hello(client::Hello {
        min: 1,
        max: 1,
        client_version: "0.3.0".into(),
        client_token: token.into(),
        ..Default::default()
    }))
}

/// What a protocol-2 client sends from a remote host: it lingers and wants
/// process info.
fn hello_2(token: &str) -> Step {
    c(client::ToCore::Hello(client::Hello {
        min: 1,
        max: 2,
        client_version: "0.4.0".into(),
        client_token: token.into(),
        linger_secs: 300,
        process_info: true,
    }))
}

fn welcome_2() -> Step {
    s(client::ToClient::Welcome(client::Welcome {
        protocol: 2,
        core_version: "0.4.0".into(),
        core_build: "c2e1".into(),
        pid: 4242,
        holder_protocols: vec![1, 2],
    }))
}

fn process(session: u32, name: Option<&str>, cwd: Option<&str>) -> Step {
    s(client::ToClient::Process(client::Process {
        session,
        name: name.map(Into::into),
        cwd: cwd.map(Into::into),
    }))
}

/// The versions a core that still speaks only protocol 1 answers with.
const CORE_PROTOCOL_1: VersionRange = VersionRange::new(1, 1);

fn welcome() -> Step {
    s(client::ToClient::Welcome(client::Welcome {
        protocol: 1,
        core_version: "0.3.0".into(),
        core_build: "b1d0".into(),
        pid: 4242,
        holder_protocols: vec![1],
    }))
}

fn input(session: u32, bytes: &[u8]) -> Step {
    c(client::ToCore::Data(client::Data {
        session,
        bytes: bytes.to_vec(),
    }))
}

fn output(session: u32, bytes: &[u8]) -> Step {
    s(client::ToClient::Data(client::Data {
        session,
        bytes: bytes.to_vec(),
    }))
}

fn spawn(req: u64, command: &str, policy: Policy) -> Step {
    c(client::ToCore::Spawn(spawn_message(req, command, policy)))
}

fn spawn_message(req: u64, command: &str, policy: Policy) -> client::Spawn {
    client::Spawn {
        req,
        command: Some(command.into()),
        args: vec!["-l".into()],
        cwd: Some("/home/u".into()),
        env: BTreeMap::from([
            ("HOME".into(), "/home/u".into()),
            ("TERM".into(), "xterm-256color".into()),
        ]),
        cols: 80,
        rows: 24,
        cell_w: 9,
        cell_h: 18,
        scrollback: 10_000,
        policy,
        profile: None,
    }
}

fn spawned(req: u64, session: u32, pid: u32) -> Step {
    s(client::ToClient::Spawned(client::Spawned {
        req,
        session,
        pid,
        key: String::new(),
    }))
}

fn exit(session: u32, code: Option<i32>) -> Step {
    s(client::ToClient::Exit(client::Exit { session, code, lost: None }))
}

fn error(req: Option<u64>, session: Option<u32>, message: &str) -> Step {
    s(client::ToClient::Error(client::ErrorMessage {
        req,
        session,
        message: message.into(),
    }))
}

fn replay(req: Option<u64>, session: u32, pid: u32, screen: &[&[u8]]) -> Vec<Step> {
    let mut steps = vec![s(client::ToClient::ReplayStart(client::ReplayStart {
        req,
        session,
        pid,
    }))];
    steps.extend(screen.iter().map(|bytes| output(session, bytes)));
    steps.push(s(client::ToClient::ReplayEnd(client::SessionRef {
        session,
    })));
    steps
}

fn meta(policy: &str, command: &str) -> serde_json::Value {
    json!({
        "command": command,
        "cwd": "/home/u",
        "owner": TOKEN,
        "policy": policy,
        "scrollback": 10000,
    })
}

fn holder_hello(instance: u64) -> Step {
    ch(holder::ToHolder::Hello(holder::Hello {
        min: 1,
        max: 1,
        core_build: "b1d0".into(),
        instance,
    }))
}

fn checkpoint_to_holder(session: u32, bytes: &[u8], chunk_len: usize, covers: u64) -> Vec<Step> {
    let (begin, chunks, _) = holder::split(session, bytes, chunk_len);
    std::iter::once(holder::ToHolder::CheckpointBegin(begin))
        .chain(chunks.into_iter().map(holder::ToHolder::CheckpointChunk))
        .chain(std::iter::once(holder::ToHolder::CheckpointEnd(
            holder::CheckpointEnd { session, covers },
        )))
        .map(ch)
        .collect()
}

fn adopted(session: u32, checkpoint: &[u8], tail: &[u8], chunk_len: usize) -> Vec<Step> {
    let (begin, chunks, end) = holder::split(session, checkpoint, chunk_len);
    let mut steps: Vec<Step> = std::iter::once(holder::ToCore::CheckpointBegin(begin))
        .chain(chunks.into_iter().map(holder::ToCore::CheckpointChunk))
        .chain(std::iter::once(holder::ToCore::CheckpointEnd(end)))
        .map(h)
        .collect();
    let (begin, chunks, end) = holder::split(session, tail, chunk_len);
    steps.extend(
        std::iter::once(holder::ToCore::TailBegin(begin))
            .chain(chunks.into_iter().map(holder::ToCore::TailChunk))
            .chain(std::iter::once(holder::ToCore::TailEnd(end)))
            .map(h),
    );
    steps
}

const CHECKPOINT: &[u8] = b"\x1bc$ make\r\nok\r\n\x1b]133;A\x07$ \x1b[?2004h";

fn transcripts() -> Vec<Transcript> {
    vec![
        Transcript {
            name: "client-spawn-io",
            protocol: Protocol::Client,
            about: "Handshake, a session's I/O, resize and exit, a failed spawn, and a close.",
            steps: vec![
                hello(TOKEN),
                welcome(),
                spawn(1, "/bin/bash", Policy::Kill),
                spawned(1, 1, 5000),
                output(1, b"$ "),
                input(1, b"echo hi\r"),
                output(1, b"echo hi\r\nhi\r\n$ "),
                c(client::ToCore::Resize(client::Resize {
                    session: 1,
                    cols: 120,
                    rows: 40,
                    cell_w: 9,
                    cell_h: 18,
                })),
                input(1, b"exit 3\r"),
                exit(1, Some(3)),
                spawn(2, "/nonexistent", Policy::Kill),
                error(
                    Some(2),
                    None,
                    "cannot run /nonexistent: No such file or directory (os error 2)",
                ),
                spawn(3, "/bin/bash", Policy::Kill),
                spawned(3, 2, 5001),
                c(client::ToCore::Close(client::SessionRef { session: 2 })),
                exit(2, None),
            ],
        },
        Transcript {
            name: "client-attach",
            protocol: Protocol::Client,
            about: "Attaching to a detached session with its replay, detaching, and an attach refused.",
            steps: [
                vec![
                    hello(TOKEN),
                    welcome(),
                    c(client::ToCore::Attach(client::Attach {
                        req: 1,
                        session: 3,
                        cols: 100,
                        rows: 30,
                        page: None,
                    })),
                ],
                replay(
                    Some(1),
                    3,
                    5003,
                    &[
                        b"\x1bc\x1b[?1049h\x1b[1;1H\x1b[7m htop \x1b[m",
                        b"\x1b[?1000h\x1b[?1006h\x1b[5;9H",
                    ],
                ),
                vec![
                    output(3, b"\x1b[2;1HCPU 3%"),
                    input(3, b"q"),
                    c(client::ToCore::Detach(client::Detach {
                        session: 3,
                        layout: None,
                    })),
                    c(client::ToCore::Attach(client::Attach {
                        req: 2,
                        session: 4,
                        cols: 100,
                        rows: 30,
                        page: None,
                    })),
                    error(Some(2), Some(4), "session 4 is attached"),
                ],
            ]
            .concat(),
        },
        Transcript {
            name: "client-layout",
            protocol: Protocol::Client,
            about: "A quitting client detaches a split tab's sessions with the tab's layout; a later client lists it, and attaching clears it.",
            steps: {
                // Keys in sorted order, which is how they encode whether or
                // not serde_json's `preserve_order` is on (feature
                // unification turns it on in a workspace build).
                let layout = json!({
                    "index": 0,
                    "origin": "1f2e-9a",
                    "title": "logs",
                    "v": 1,
                    "zones": {
                        "first": {"active": true, "key": KEY},
                        "second": {"key": "3c0b7e2f5a9d4186b2e7c0f19d3a5b48"},
                        "split": "horizontal",
                    },
                });
                let listed = |session: u32, key: &str, layout: Option<serde_json::Value>| client::SessionInfo {
                    session,
                    pid: 5000 + session,
                    command: "bash -l".into(),
                    policy: Policy::Detach,
                    cols: 80,
                    rows: 24,
                    key: key.into(),
                    started: 1_794_000_000,
                    profile: Some("Shell".into()),
                    layout,
                    ..Default::default()
                };
                vec![
                    hello(TOKEN),
                    welcome(),
                    c(client::ToCore::Detach(client::Detach { session: 1, layout: Some(layout.clone()) })),
                    c(client::ToCore::Detach(client::Detach { session: 2, layout: Some(layout.clone()) })),
                    c(client::ToCore::List(client::Request { req: 1 })),
                    s(client::ToClient::Sessions(client::Sessions {
                        req: 1,
                        sessions: vec![
                            listed(1, KEY, Some(layout.clone())),
                            listed(2, "3c0b7e2f5a9d4186b2e7c0f19d3a5b48", Some(layout)),
                        ],
                    })),
                    c(client::ToCore::Attach(client::Attach { req: 2, session: 1, cols: 80, rows: 24, page: None })),
                    s(client::ToClient::ReplayStart(client::ReplayStart { req: Some(2), session: 1, pid: 5001 })),
                    s(client::ToClient::ReplayEnd(client::SessionRef { session: 1 })),
                    c(client::ToCore::List(client::Request { req: 3 })),
                    s(client::ToClient::Sessions(client::Sessions {
                        req: 3,
                        sessions: vec![client::SessionInfo { attached: true, ..listed(1, KEY, None) }],
                    })),
                ]
            },
        },
        Transcript {
            name: "client-set-layout",
            protocol: Protocol::Client3,
            about: "Protocol 3: a client keeps an attached session's layout current with SET_LAYOUT; one for a session it hasn't attached is refused.",
            steps: {
                // Keys in sorted order (see client-layout).
                let layout = |index: u32| {
                    json!({"index": index, "origin": "1f2e-9a", "v": 1, "zones": {"active": true, "key": KEY}})
                };
                vec![
                    c(client::ToCore::Hello(client::Hello {
                        min: 1,
                        max: 3,
                        client_version: "0.5.0".into(),
                        client_token: TOKEN.into(),
                        ..Default::default()
                    })),
                    s(client::ToClient::Welcome(client::Welcome {
                        protocol: 3,
                        core_version: "0.5.0".into(),
                        core_build: "d3f2".into(),
                        pid: 4242,
                        holder_protocols: vec![2],
                    })),
                    c(client::ToCore::SetLayout(client::SetLayout { session: 1, layout: Some(layout(0)) })),
                    c(client::ToCore::SetLayout(client::SetLayout { session: 1, layout: Some(layout(1)) })),
                    c(client::ToCore::SetLayout(client::SetLayout { session: 9, layout: Some(layout(0)) })),
                    error(None, Some(9), "session 9 isn't attached to this client"),
                    c(client::ToCore::SetLayout(client::SetLayout { session: 1, layout: None })),
                ]
            },
        },
        Transcript {
            name: "client-reconnect",
            protocol: Protocol::Client,
            about: "A client reconnecting with its token after a core restart gets each session's replay.",
            steps: [
                vec![hello(TOKEN), welcome()],
                replay(None, 1, 5000, &[b"\x1bc$ ls\r\na  b\r\n$ "]),
                replay(None, 2, 5001, &[b"\x1bc\x1b[?1049h", b"\x1b[H vim "]),
                vec![input(1, b"pwd\r"), output(1, b"pwd\r\n/home/u\r\n$ ")],
            ]
            .concat(),
        },
        Transcript {
            name: "client-management",
            protocol: Protocol::Client,
            about: "The management commands: list, kill, status, a refused reparent, step-down and stop.",
            steps: vec![
                hello("9e8d7c6b5a493827"),
                welcome(),
                c(client::ToCore::List(client::Request { req: 1 })),
                s(client::ToClient::Sessions(client::Sessions {
                    req: 1,
                    sessions: vec![
                        client::SessionInfo {
                            session: 1,
                            pid: 5000,
                            attached: true,
                            command: "bash -l".into(),
                            cwd: Some("/home/u".into()),
                            policy: Policy::Kill,
                            cols: 80,
                            rows: 24,
                            ..Default::default()
                        },
                        client::SessionInfo {
                            session: 3,
                            pid: 5003,
                            attached: false,
                            command: "htop".into(),
                            cwd: Some("/tmp".into()),
                            policy: Policy::Detach,
                            cols: 100,
                            rows: 30,
                            ..Default::default()
                        },
                    ],
                })),
                c(client::ToCore::Kill(client::Kill {
                    req: 2,
                    target: KillTarget::Session(3),
                })),
                s(client::ToClient::Ok(client::Request { req: 2 })),
                c(client::ToCore::Kill(client::Kill {
                    req: 3,
                    target: KillTarget::Session(99),
                })),
                error(Some(3), Some(99), "no session 99"),
                c(client::ToCore::Status(client::Request { req: 4 })),
                s(client::ToClient::ServerStatus(client::ServerStatus {
                    req: 4,
                    core_version: "0.3.0".into(),
                    core_build: "b1d0".into(),
                    protocols: CORE_PROTOCOL_1,
                    holder_protocols: VersionRange::new(1, 1),
                    pid: 4242,
                    uptime_secs: 3600,
                    clients: 2,
                    holders: vec![
                        client::HolderStatus {
                            pid: 4243,
                            build: "b1d0".into(),
                            protocol: 1,
                            sessions: 1,
                            draining: false,
                        },
                        client::HolderStatus {
                            pid: 4100,
                            build: "a0c9".into(),
                            protocol: 1,
                            sessions: 1,
                            draining: true,
                        },
                    ],
                })),
                c(client::ToCore::Reparent(client::Reparent {
                    session: 1,
                    hwnd: 0x0003_04a2,
                })),
                error(None, Some(1), "REPARENT is not supported on Linux"),
                c(client::ToCore::Kill(client::Kill {
                    req: 5,
                    target: KillTarget::All,
                })),
                s(client::ToClient::Ok(client::Request { req: 5 })),
                c(client::ToCore::StepDown(client::Request { req: 6 })),
                s(client::ToClient::Ok(client::Request { req: 6 })),
                c(client::ToCore::Stop(client::Request { req: 7 })),
                s(client::ToClient::Ok(client::Request { req: 7 })),
            ],
        },
        Transcript {
            name: "client-hello-refused",
            protocol: Protocol::Client,
            about: "A client whose versions the core doesn't speak (here a protocol-1 core) gets an ERROR naming both ranges.",
            steps: vec![
                c(client::ToCore::Hello(client::Hello {
                    min: 2,
                    max: 3,
                    client_version: "0.9.0".into(),
                    client_token: TOKEN.into(),
                    ..Default::default()
                })),
                error(
                    None,
                    None,
                    &client::negotiate(VersionRange::new(2, 3), CORE_PROTOCOL_1)
                        .unwrap_err()
                        .to_string(),
                ),
            ],
        },
        Transcript {
            name: "client-process-info",
            protocol: Protocol::Client2,
            about: "Protocol 2: a HELLO asking for linger and process info, PROCESS on changes, and the owner state in a list.",
            steps: vec![
                hello_2(TOKEN),
                welcome_2(),
                c(client::ToCore::Spawn(client::Spawn {
                    profile: Some("A8".into()),
                    ..spawn_message(1, "/bin/bash", Policy::Detach)
                })),
                spawned(1, 1, 5000),
                process(1, Some("bash"), Some("/home/u")),
                input(1, b"sleep 30\r"),
                process(1, Some("sleep"), None),
                c(client::ToCore::List(client::Request { req: 2 })),
                s(client::ToClient::Sessions(client::Sessions {
                    req: 2,
                    sessions: vec![
                        client::SessionInfo {
                            session: 1,
                            pid: 5000,
                            attached: true,
                            command: "/bin/bash -l".into(),
                            policy: Policy::Detach,
                            cols: 80,
                            rows: 24,
                            owned: true,
                            ..Default::default()
                        },
                        client::SessionInfo {
                            session: 2,
                            pid: 5001,
                            command: "htop".into(),
                            policy: Policy::Detach,
                            cols: 100,
                            rows: 30,
                            owned: true,
                            detached_secs: 125,
                            ..Default::default()
                        },
                    ],
                })),
            ],
        },
        Transcript {
            name: "client-old-core",
            protocol: Protocol::Client2,
            about: "A protocol-1 and 2 client's HELLO to a core that speaks only 1: it is answered with protocol 1 and gets no PROCESS.",
            steps: vec![
                hello_2(TOKEN),
                welcome(),
                spawn(1, "/bin/bash", Policy::Kill),
                spawned(1, 1, 5000),
                output(1, b"$ "),
            ],
        },
        Transcript {
            name: "relay-link",
            protocol: Protocol::Relay,
            about: "A relay's link: its host frame, the client protocol piped through, and a heartbeat.",
            steps: vec![
                Step::Relay(relay::RelayMessage::Host(relay::RelayHost {
                    version: "0.4.0".into(),
                    build: "c2e1".into(),
                    env: BTreeMap::from([
                        ("HOME".into(), "/home/u".into()),
                        ("PATH".into(), "/usr/local/bin:/usr/bin".into()),
                        ("SHELL".into(), "/bin/bash".into()),
                    ]),
                    shell: "/bin/bash".into(),
                    home: "/home/u".into(),
                    server_dir: "/run/user/1000/brindle".into(),
                    core_pid: 4242,
                    login: None,
                })),
                hello_2(TOKEN),
                welcome_2(),
                Step::Relay(relay::RelayMessage::Heartbeat),
                spawn(1, "/bin/bash", Policy::Detach),
                spawned(1, 1, 5000),
                process(1, Some("bash"), Some("/home/u")),
                Step::Relay(relay::RelayMessage::Heartbeat),
            ],
        },
        Transcript {
            name: "relay-host-login",
            protocol: Protocol::Relay,
            about: "A relay started with --login-environment reports the login shell's environment, and on another connection why it wasn't read.",
            steps: vec![
                Step::Relay(relay::RelayMessage::Host(relay::RelayHost {
                    version: "0.5.0".into(),
                    build: "d3f2".into(),
                    env: BTreeMap::from([
                        ("HOME".into(), "/home/u".into()),
                        ("PATH".into(), "/usr/local/bin:/usr/bin".into()),
                        ("SHELL".into(), "/bin/bash".into()),
                    ]),
                    shell: "/bin/bash".into(),
                    home: "/home/u".into(),
                    server_dir: "/run/user/1000/brindle".into(),
                    core_pid: 4242,
                    login: Some(relay::LoginEnv::Captured(BTreeMap::from([
                        ("EDITOR".into(), "vim".into()),
                        ("HOME".into(), "/home/u".into()),
                        ("PATH".into(), "/home/u/.local/bin:/usr/local/bin:/usr/bin".into()),
                        ("SHELL".into(), "/bin/bash".into()),
                    ]))),
                })),
                Step::Relay(relay::RelayMessage::Host(relay::RelayHost {
                    version: "0.5.0".into(),
                    build: "d3f2".into(),
                    env: BTreeMap::from([
                        ("HOME".into(), "/home/u".into()),
                        ("PATH".into(), "/usr/local/bin:/usr/bin".into()),
                        ("SHELL".into(), "/bin/bash".into()),
                    ]),
                    shell: "/bin/bash".into(),
                    home: "/home/u".into(),
                    server_dir: "/run/user/1000/brindle".into(),
                    core_pid: 4242,
                    login: Some(relay::LoginEnv::Failed("it took longer than 10 s".into())),
                })),
            ],
        },
        Transcript {
            name: "relay-error",
            protocol: Protocol::Relay,
            about: "A relay that can't reach or start its core says why and ends.",
            steps: vec![Step::Relay(relay::RelayMessage::Error(relay::RelayError {
                message: "can't start the terminal server: no server directory ($HOME is unset); core log /tmp/brindle/core.log".into(),
            }))],
        },
        Transcript {
            name: "holder-spawn-io",
            protocol: Protocol::Holder,
            about: "A new holder: handshake, spawn with meta, I/O, resize, pause and resume, a failed spawn, reparent refused, hangup.",
            steps: vec![
                holder_hello(1),
                h(holder::ToCore::Welcome(holder::Welcome {
                    protocol: 1,
                    build: "b1d0".into(),
                    pid: 4243,
                    previous: Previous::None,
                    sessions: Vec::new(),
                })),
                ch(holder::ToHolder::Spawn(holder::Spawn {
                    session: 1,
                    command: Some("/bin/bash".into()),
                    args: vec!["-l".into()],
                    cwd: Some("/home/u".into()),
                    env: BTreeMap::from([("TERM".into(), "xterm-256color".into())]),
                    cols: 80,
                    rows: 24,
                    cell_w: 9,
                    cell_h: 18,
                    meta: meta("kill", "bash -l"),
                })),
                h(holder::ToCore::Spawned(holder::Spawned {
                    session: 1,
                    pid: 5000,
                })),
                h(holder::ToCore::Output(holder::Data {
                    session: 1,
                    bytes: b"$ ".to_vec(),
                })),
                ch(holder::ToHolder::Input(holder::Data {
                    session: 1,
                    bytes: b"echo hi\r".to_vec(),
                })),
                h(holder::ToCore::Output(holder::Data {
                    session: 1,
                    bytes: b"echo hi\r\nhi\r\n$ ".to_vec(),
                })),
                ch(holder::ToHolder::Resize(holder::Resize {
                    session: 1,
                    cols: 120,
                    rows: 40,
                    cell_w: 9,
                    cell_h: 18,
                })),
                ch(holder::ToHolder::Pause(holder::SessionRef { session: 1 })),
                ch(holder::ToHolder::Resume(holder::SessionRef { session: 1 })),
                ch(holder::ToHolder::Spawn(holder::Spawn {
                    session: 2,
                    command: Some("/nonexistent".into()),
                    args: Vec::new(),
                    cwd: None,
                    env: BTreeMap::new(),
                    cols: 80,
                    rows: 24,
                    cell_w: 9,
                    cell_h: 18,
                    meta: meta("detach", "/nonexistent"),
                })),
                h(holder::ToCore::SpawnFailed(holder::SpawnFailed {
                    session: 2,
                    message: "cannot run /nonexistent: No such file or directory (os error 2)"
                        .into(),
                })),
                ch(holder::ToHolder::Reparent(holder::Reparent {
                    session: 1,
                    hwnd: 0x0003_04a2,
                })),
                h(holder::ToCore::Error(holder::ErrorMessage {
                    session: Some(1),
                    message: "unsupported".into(),
                })),
                ch(holder::ToHolder::Input(holder::Data {
                    session: 1,
                    bytes: b"exit 3\r".to_vec(),
                })),
                h(holder::ToCore::Exited(holder::Exited {
                    session: 1,
                    code: Some(3),
                })),
                ch(holder::ToHolder::Hangup(holder::SessionRef { session: 3 })),
                h(holder::ToCore::Exited(holder::Exited {
                    session: 3,
                    code: None,
                })),
            ],
        },
        Transcript {
            name: "holder-checkpoint",
            protocol: Protocol::Holder,
            about: "A core checkpoints its session in chunks and steps down with a handover.",
            steps: [
                vec![
                    holder_hello(1),
                    h(holder::ToCore::Welcome(holder::Welcome {
                        protocol: 1,
                        build: "b1d0".into(),
                        pid: 4243,
                        previous: Previous::None,
                        sessions: Vec::new(),
                    })),
                ],
                checkpoint_to_holder(1, CHECKPOINT, 16, 4096),
                vec![ch(holder::ToHolder::Goodbye(holder::Goodbye {
                    handover: true,
                }))],
            ]
            .concat(),
        },
        Transcript {
            name: "holder-adopt",
            protocol: Protocol::Holder,
            about: "A newer core adopts an older holder: the held sessions' checkpoints and tails, then drain.",
            steps: [
                vec![
                    ch(holder::ToHolder::Hello(holder::Hello {
                        min: 1,
                        max: 1,
                        core_build: "c2e1".into(),
                        instance: 2,
                    })),
                    h(holder::ToCore::Welcome(holder::Welcome {
                        protocol: 1,
                        build: "b1d0".into(),
                        pid: 4243,
                        previous: Previous::Handover,
                        sessions: vec![
                            holder::HeldSession {
                                session: 1,
                                pid: 5000,
                                cols: 120,
                                rows: 40,
                                exit: None,
                                meta: meta("kill", "bash -l"),
                                truncated: false,
                                mode: holder::SpawnMode::Pty,
                            },
                            holder::HeldSession {
                                session: 4,
                                pid: 5004,
                                cols: 80,
                                rows: 24,
                                exit: Some(holder::ExitStatus { code: Some(0) }),
                                meta: meta("detach", "make"),
                                truncated: true,
                                mode: holder::SpawnMode::Pty,
                            },
                        ],
                    })),
                ],
                adopted(1, CHECKPOINT, b"make test\r\n", 16),
                adopted(4, b"", b"...last bytes of a long build\r\n", 16),
                vec![
                    ch(holder::ToHolder::Drain(holder::Drain {})),
                    ch(holder::ToHolder::Input(holder::Data {
                        session: 1,
                        bytes: b"\x03".to_vec(),
                    })),
                    h(holder::ToCore::Output(holder::Data {
                        session: 1,
                        bytes: b"^C\r\n$ ".to_vec(),
                    })),
                ],
            ]
            .concat(),
        },
        Transcript {
            name: "client-session-keys",
            protocol: Protocol::Client,
            about: "Fields added within protocol 1: a spawn's profile, a session's key, and the key, title, start time and profile in a list.",
            steps: vec![
                hello(TOKEN),
                welcome(),
                c(client::ToCore::Spawn(client::Spawn {
                    profile: Some("Shell".into()),
                    ..spawn_message(1, "/bin/bash", Policy::Detach)
                })),
                s(client::ToClient::Spawned(client::Spawned {
                    req: 1,
                    session: 7,
                    pid: 5000,
                    key: KEY.into(),
                })),
                c(client::ToCore::Detach(client::Detach {
                        session: 7,
                        layout: None,
                    })),
                c(client::ToCore::List(client::Request { req: 2 })),
                s(client::ToClient::Sessions(client::Sessions {
                    req: 2,
                    sessions: vec![client::SessionInfo {
                        session: 7,
                        pid: 5000,
                        attached: false,
                        command: "/bin/bash -l".into(),
                        cwd: Some("/home/u".into()),
                        policy: Policy::Detach,
                        cols: 80,
                        rows: 24,
                        key: KEY.into(),
                        title: "vim notes.md".into(),
                        started: 1_794_000_000,
                        profile: Some("Shell".into()),
                        ..Default::default()
                    }],
                })),
                c(client::ToCore::Attach(client::Attach {
                    req: 3,
                    session: 7,
                    cols: 100,
                    rows: 30,
                    page: None,
                })),
            ]
            .into_iter()
            .chain(replay(Some(3), 7, 5000, &[&b"\x1bc"[..], b"vim"]))
            .collect(),
        },
        Transcript {
            name: "holder-set-meta-layout",
            protocol: Protocol::Holder,
            about: "H1 version 2: a detach's layout is stored in the session's meta, and an attach's SET_META drops it.",
            steps: vec![
                ch(holder::ToHolder::Hello(holder::Hello {
                    min: 1,
                    max: 2,
                    core_build: "b1d0".into(),
                    instance: 1,
                })),
                h(holder::ToCore::Welcome(holder::Welcome {
                    protocol: 2,
                    build: "b1d0".into(),
                    pid: 4243,
                    previous: Previous::None,
                    sessions: Vec::new(),
                })),
                ch(holder::ToHolder::SetMeta(holder::SetMeta {
                    session: 1,
                    meta: holder::SessionMeta {
                        key: KEY.into(),
                        layout: Some(json!({"v": 1, "zones": {"active": true, "key": KEY}})),
                        policy: Policy::Detach,
                        profile: Some("Shell".into()),
                        scrollback: 10000,
                        started: 1_794_000_000,
                        ..Default::default()
                    }
                    .to_value(),
                })),
                ch(holder::ToHolder::SetMeta(holder::SetMeta {
                    session: 1,
                    meta: holder::SessionMeta {
                        key: KEY.into(),
                        owner: "b".into(),
                        policy: Policy::Detach,
                        profile: Some("Shell".into()),
                        scrollback: 10000,
                        started: 1_794_000_000,
                        ..Default::default()
                    }
                    .to_value(),
                })),
            ],
        },
        Transcript {
            name: "holder-set-meta",
            protocol: Protocol::Holder,
            about: "H1 version 2: the core replaces a session's stored meta, and the next core gets it.",
            steps: vec![
                ch(holder::ToHolder::Hello(holder::Hello {
                    min: 1,
                    max: 2,
                    core_build: "b1d0".into(),
                    instance: 1,
                })),
                h(holder::ToCore::Welcome(holder::Welcome {
                    protocol: 2,
                    build: "b1d0".into(),
                    pid: 4243,
                    previous: Previous::None,
                    sessions: Vec::new(),
                })),
                ch(holder::ToHolder::SetMeta(holder::SetMeta {
                    session: 1,
                    meta: json!({
                        "command": "bash -l",
                        "cwd": "/home/u",
                        "key": KEY,
                        "owner": "",
                        "policy": "detach",
                        "profile": "Shell",
                        "scrollback": 10000,
                        "started": 1_794_000_000u64,
                    }),
                })),
            ],
        },
        Transcript {
            name: "client-session-lost",
            protocol: Protocol::Client,
            about: "A session the server lost (its holder ended) ends with an EXIT that says why and has no code.",
            steps: vec![
                hello(TOKEN),
                welcome(),
                spawn(1, "/bin/bash", Policy::Detach),
                spawned(1, 1, 5000),
                output(1, b"$ "),
                s(client::ToClient::Exit(client::Exit {
                    session: 1,
                    code: None,
                    lost: Some("its holder ended".into()),
                })),
            ],
        },
        Transcript {
            name: "holder-adopt-after-crash",
            protocol: Protocol::Holder,
            about: "A core adopts a holder whose last core crashed: one session still running and one a signal ended, each with its checkpoint and tail.",
            steps: [
                vec![
                    ch(holder::ToHolder::Hello(holder::Hello {
                        min: 1,
                        max: 2,
                        core_build: "b1d0".into(),
                        instance: 3,
                    })),
                    h(holder::ToCore::Welcome(holder::Welcome {
                        protocol: 2,
                        build: "b1d0".into(),
                        pid: 4243,
                        previous: Previous::Crash,
                        sessions: vec![
                            holder::HeldSession {
                                session: 1,
                                pid: 5000,
                                cols: 80,
                                rows: 24,
                                exit: None,
                                meta: meta("detach", "bash -l"),
                                truncated: false,
                                mode: holder::SpawnMode::Pty,
                            },
                            holder::HeldSession {
                                session: 2,
                                pid: 5002,
                                cols: 80,
                                rows: 24,
                                exit: Some(holder::ExitStatus { code: None }),
                                meta: meta("kill", "sleep 1000"),
                                truncated: false,
                                mode: holder::SpawnMode::Pty,
                            },
                        ],
                    })),
                ],
                adopted(1, CHECKPOINT, b"ls\r\nnotes.md\r\n$ ", 16),
                adopted(2, b"\x1bc$ sleep 1000\r\n", b"", 16),
            ]
            .concat(),
        },
    ]
    .into_iter()
    .chain(agent_transcripts())
    .chain(paging_transcripts())
    .chain(session_list_transcripts())
    .chain(options_transcripts())
    .collect()
}

fn set_option(req: u64, session: u32, option: &str, value: client::OptionValue) -> Step {
    c(client::ToCore::AgentSetOption(client::AgentSetOption {
        req,
        session,
        option: option.into(),
        value,
    }))
}

fn choice_value(value: &str) -> client::OptionValue {
    client::OptionValue::Choice {
        value: value.into(),
    }
}

/// The fake agent's options in the options transcripts, with the mode, the
/// model and the fast toggle as given.
fn fake_options(mode: &str, model: &str, fast: bool) -> serde_json::Value {
    json!({"event": "options", "options": [
        {"id": "mode", "name": "Mode", "category": {"category": "mode"},
         "kind": {"control": "select", "current": mode, "choices": [
             {"value": "default", "name": "Default"},
             {"value": "plan", "name": "Plan", "description": "Plans before it edits"},
         ]}},
        {"id": "model", "name": "Model", "category": {"category": "model"},
         "kind": {"control": "select", "current": model, "choices": [
             {"value": "opus", "name": "Opus"},
             {"value": "sonnet", "name": "Sonnet"},
         ]}},
        {"id": "fast", "name": "Fast mode", "category": {"category": "other", "name": "speed"},
         "kind": {"control": "toggle", "on": fast}},
    ]})
}

/// Transcripts added by `agents-commands`, within protocol 4 before it
/// shipped (its design decisions 2 and 4).
fn options_transcripts() -> Vec<Transcript> {
    vec![
        Transcript {
            name: "client-agent-set-option",
            protocol: Protocol::Client4,
            about: "Protocol 4 with options: an agent session's commands and options after its start, a model chosen, a toggle turned on, the mode changed with its notice, and a set naming the current value answered with nothing sent.",
            steps: vec![
                hello_4(TOKEN),
                welcome_4(),
                c(client::ToCore::SpawnAgent(client::SpawnAgent {
                    req: 1,
                    command: "fake-acp-agent".into(),
                    args: vec!["--script".into(), "/home/u/options.script".into()],
                    cwd: Some("~/src".into()),
                    env: BTreeMap::from([("HOME".into(), "/home/u".into())]),
                    policy: Policy::Detach,
                    profile: Some("Agent".into()),
                    conversation: None,
                    start: None,
                })),
                s(client::ToClient::Spawned(client::Spawned {
                    req: 1,
                    session: 1,
                    pid: 5000,
                    key: KEY.into(),
                })),
                transcript(
                    1,
                    vec![
                        json!({"event": "commands", "commands": [
                            {"name": "compact", "description": "Summarize the conversation", "hint": "<instructions>"},
                            {"name": "review", "description": "Review the changes"},
                        ]}),
                        conversation("resume_and_load"),
                        fake_options("default", "opus", false),
                        ready(),
                    ],
                ),
                set_option(2, 1, "model", choice_value("sonnet")),
                ok(2),
                transcript(1, vec![fake_options("default", "sonnet", false)]),
                set_option(3, 1, "fast", client::OptionValue::Toggle { on: true }),
                ok(3),
                transcript(1, vec![fake_options("default", "sonnet", true)]),
                set_option(4, 1, "mode", choice_value("plan")),
                ok(4),
                transcript(
                    1,
                    vec![
                        fake_options("plan", "sonnet", true),
                        added(1, notice("Mode: Plan")),
                    ],
                ),
                set_option(5, 1, "mode", choice_value("plan")),
                ok(5),
            ],
        },
        Transcript {
            name: "client-agent-set-option-refused",
            protocol: Protocol::Client4,
            about: "Protocol 4 with options: sets refused before the agent is ready, for an option the agent doesn't offer, for a choice it doesn't list and for the wrong kind of value, and while an earlier set is unanswered; then the agent refusing a set, which leaves a notice.",
            steps: vec![
                hello_4(TOKEN),
                welcome_4(),
                spawn_choosing(1),
                spawned(1, 1, 5000),
                transcript(1, vec![choosing(None)]),
                set_option(2, 1, "mode", choice_value("plan")),
                error(Some(2), Some(1), "session 1: the agent isn't ready"),
                choose(3, 1, None),
                ok(3),
                transcript(
                    1,
                    vec![
                        conversation("resume_and_load"),
                        fake_options("default", "opus", false),
                        ready(),
                    ],
                ),
                set_option(4, 1, "speed", choice_value("fast")),
                error(Some(4), Some(1), "session 1: the agent offers no option \"speed\""),
                set_option(5, 1, "model", choice_value("gpt-5")),
                error(Some(5), Some(1), "session 1: option \"model\" offers no choice \"gpt-5\""),
                set_option(6, 1, "fast", choice_value("on")),
                error(Some(6), Some(1), "session 1: option \"fast\" offers no choice \"on\""),
                set_option(7, 1, "model", choice_value("sonnet")),
                ok(7),
                set_option(8, 1, "mode", choice_value("plan")),
                error(
                    Some(8),
                    Some(1),
                    "session 1: an earlier change of an option hasn't been answered yet",
                ),
                transcript(
                    1,
                    vec![added(
                        1,
                        json!({"type": "notice", "level": "warning", "text": "the agent didn't change Model to Sonnet: model unavailable"}),
                    )],
                ),
                set_option(9, 1, "mode", choice_value("plan")),
                ok(9),
            ],
        },
        Transcript {
            name: "client-agent-set-option-old-client",
            protocol: Protocol::Client3,
            about: "A protocol 3 client sending AGENT_SET_OPTION is refused.",
            steps: vec![
                c(client::ToCore::Hello(client::Hello {
                    min: 1,
                    max: 3,
                    client_version: "0.5.0".into(),
                    client_token: TOKEN.into(),
                    ..Default::default()
                })),
                s(client::ToClient::Welcome(client::Welcome {
                    protocol: 3,
                    core_version: "0.6.0".into(),
                    core_build: "e4a3".into(),
                    pid: 4242,
                    holder_protocols: vec![3],
                })),
                set_option(2, 2, "mode", choice_value("plan")),
                error(Some(2), Some(2), "AGENT_SET_OPTION needs client protocol 4"),
            ],
        },
    ]
}

/// A `SPAWN_AGENT` of the fake agent in `~/src` that chooses before it
/// starts a conversation.
fn spawn_choosing(req: u64) -> Step {
    c(client::ToCore::SpawnAgent(client::SpawnAgent {
        req,
        command: "fake-acp-agent".into(),
        args: vec!["--script".into(), "/home/u/list.script".into()],
        cwd: Some("~/src".into()),
        env: BTreeMap::from([("HOME".into(), "/home/u".into())]),
        policy: Policy::Detach,
        profile: Some("Agent".into()),
        conversation: None,
        start: Some(client::SpawnStart::Choose),
    }))
}

fn choosing(message: Option<&str>) -> serde_json::Value {
    match message {
        Some(message) => json!({"event": "agent", "state": {"agent": "choosing", "message": message}}),
        None => json!({"event": "agent", "state": {"agent": "choosing"}}),
    }
}

fn list(req: u64, session: u32, cursor: Option<&str>) -> Step {
    c(client::ToCore::AgentList(client::AgentList {
        req,
        session,
        cursor: cursor.map(str::to_owned),
    }))
}

fn choose(req: u64, session: u32, conversation: Option<&str>) -> Step {
    c(client::ToCore::AgentChoose(client::AgentChoose {
        req,
        session,
        conversation: conversation.map(str::to_owned),
    }))
}

fn listed(id: &str, title: Option<&str>, updated: Option<&str>) -> client::ListedConversation {
    client::ListedConversation {
        id: id.into(),
        title: title.map(str::to_owned),
        updated: updated.map(str::to_owned),
    }
}

fn conversations(
    req: u64,
    session: u32,
    conversations: Vec<client::ListedConversation>,
    next: Option<&str>,
) -> Step {
    s(client::ToClient::AgentConversations(client::AgentConversations {
        req,
        session,
        conversations,
        next: next.map(str::to_owned),
    }))
}

/// Transcripts added by `agents-session-list`, within protocol 4 before it
/// shipped (its design decision 9).
fn session_list_transcripts() -> Vec<Transcript> {
    vec![
        Transcript {
            name: "client-agent-choose",
            protocol: Protocol::Client4,
            about: "Protocol 4 with listing: an agent session started to choose, two pages of its conversations, then one chosen, which the agent continues with its replay shown.",
            steps: vec![
                hello_4(TOKEN),
                welcome_4(),
                spawn_choosing(1),
                spawned(1, 1, 5000),
                transcript(1, vec![choosing(None)]),
                list(2, 1, None),
                conversations(
                    2,
                    1,
                    vec![
                        listed("sess-9a", Some("Fix the upload test"), Some("2026-11-26T18:05:00Z")),
                        listed("sess-7f", None, None),
                    ],
                    Some("page-2"),
                ),
                list(3, 1, Some("page-2")),
                conversations(
                    3,
                    1,
                    vec![listed(CONVERSATION, Some("Add paging"), Some("2026-11-20T09:00:00.125Z"))],
                    None,
                ),
                choose(4, 1, Some(CONVERSATION)),
                ok(4),
                transcript(
                    1,
                    vec![
                        added(
                            1,
                            notice("this continues an earlier conversation; what follows is the agent's replay of it"),
                        ),
                        added(2, text_entry("user", "add paging")),
                        added(3, text_entry("agent", "Paging is in.")),
                        conversation("resume_and_load"),
                        ready(),
                    ],
                ),
            ],
        },
        Transcript {
            name: "client-agent-choose-new",
            protocol: Protocol::Client4,
            about: "Protocol 4 with listing: a choosing agent with no conversations in its directory, and a new conversation chosen.",
            steps: vec![
                hello_4(TOKEN),
                welcome_4(),
                spawn_choosing(1),
                spawned(1, 1, 5000),
                transcript(1, vec![choosing(None)]),
                list(2, 1, None),
                conversations(2, 1, vec![], None),
                choose(3, 1, None),
                ok(3),
                transcript(1, vec![conversation("resume_and_load"), ready()]),
            ],
        },
        Transcript {
            name: "client-agent-list-refused",
            protocol: Protocol::Client4,
            about: "Protocol 4 with listing: a prompt while choosing, a second list before the first is answered, the agent's error to a list, a refused choice returning to choosing with its message, a choice once ready, and a list from an agent that offers none.",
            steps: vec![
                hello_4(TOKEN),
                welcome_4(),
                spawn_choosing(1),
                spawned(1, 1, 5000),
                transcript(1, vec![choosing(None)]),
                c(client::ToCore::AgentPrompt(client::AgentPrompt {
                    req: 2,
                    session: 1,
                    text: "hi".into(),
                })),
                error(Some(2), Some(1), "session 1: the agent isn't ready"),
                list(3, 1, None),
                list(4, 1, None),
                error(Some(4), Some(1), "session 1: a list of conversations is already being answered"),
                error(
                    Some(3),
                    Some(1),
                    "session 1: the agent couldn't list its conversations: index unavailable",
                ),
                choose(5, 1, Some("sess-gone")),
                ok(5),
                transcript(1, vec![choosing(Some("no such session"))]),
                choose(6, 1, None),
                ok(6),
                transcript(1, vec![conversation("resume_and_load"), ready()]),
                choose(7, 1, None),
                error(Some(7), Some(1), "session 1: the agent isn't choosing"),
                list(8, 2, None),
                error(Some(8), Some(2), "session 2: the agent can't list conversations"),
            ],
        },
        Transcript {
            name: "client-agent-list-old-client",
            protocol: Protocol::Client3,
            about: "A protocol 3 client sending AGENT_LIST and AGENT_CHOOSE is refused.",
            steps: vec![
                c(client::ToCore::Hello(client::Hello {
                    min: 1,
                    max: 3,
                    client_version: "0.5.0".into(),
                    client_token: TOKEN.into(),
                    ..Default::default()
                })),
                s(client::ToClient::Welcome(client::Welcome {
                    protocol: 3,
                    core_version: "0.6.0".into(),
                    core_build: "e4a3".into(),
                    pid: 4242,
                    holder_protocols: vec![3],
                })),
                list(2, 2, None),
                error(Some(2), Some(2), "AGENT_LIST needs client protocol 4"),
                choose(3, 2, None),
                error(Some(3), Some(2), "AGENT_CHOOSE needs client protocol 4"),
            ],
        },
    ]
}

// Agent sessions: client protocol 4 and H1 version 3.

/// `value` with every object's keys inserted in sorted order, so it encodes
/// the same whether or not serde_json's `preserve_order` is on (see
/// client-layout).
fn sorted(value: serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(object) => {
            let mut entries: Vec<(String, serde_json::Value)> = object.into_iter().collect();
            entries.sort_by(|a, b| a.0.cmp(&b.0));
            serde_json::Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key, sorted(value)))
                    .collect(),
            )
        }
        serde_json::Value::Array(items) => {
            serde_json::Value::Array(items.into_iter().map(sorted).collect())
        }
        other => other,
    }
}

/// A `TRANSCRIPT` of `events`, which are `brindle-transcript`'s protocol 4
/// forms (pinned in `transcript/tests/fixtures/v4/`).
fn transcript(session: u32, events: Vec<serde_json::Value>) -> Step {
    s(client::ToClient::Transcript(client::Transcript {
        session,
        events: events.into_iter().map(sorted).collect(),
    }))
}

fn ok(req: u64) -> Step {
    s(client::ToClient::Ok(client::Request { req }))
}

fn hello_4(token: &str) -> Step {
    c(client::ToCore::Hello(client::Hello {
        min: 1,
        max: 4,
        client_version: "0.6.0".into(),
        client_token: token.into(),
        ..Default::default()
    }))
}

fn welcome_4() -> Step {
    s(client::ToClient::Welcome(client::Welcome {
        protocol: 4,
        core_version: "0.6.0".into(),
        core_build: "e4a3".into(),
        pid: 4242,
        holder_protocols: vec![3],
    }))
}

fn prompt(req: u64, session: u32, text: &str) -> Step {
    c(client::ToCore::AgentPrompt(client::AgentPrompt {
        req,
        session,
        text: text.into(),
    }))
}

fn answer(req: u64, session: u32, entry: u64, option: Option<&str>) -> Step {
    c(client::ToCore::AgentPermission(client::AgentPermission {
        req,
        session,
        entry,
        option: option.map(Into::into),
    }))
}

fn login(req: u64, session: u32, method: &str) -> Step {
    c(client::ToCore::AgentLogin(client::AgentLogin {
        req,
        session,
        method: method.into(),
    }))
}

/// The agent state "login required" with `message` and two `agent` methods.
fn login_required(message: &str) -> serde_json::Value {
    json!({"event": "agent", "state": {"agent": "login_required", "message": message, "methods": [
        {"id": "oauth-personal", "name": "Log in with Google", "kind": "agent"},
        {"id": "api-key", "name": "API Key", "description": "Use an API key", "kind": "agent"},
    ]}})
}

fn logging_in() -> serde_json::Value {
    json!({"event": "agent", "state": {"agent": "logging_in", "method": "oauth-personal", "name": "Log in with Google"}})
}

fn added(id: u64, entry: serde_json::Value) -> serde_json::Value {
    json!({"event": "added", "id": id, "entry": entry})
}

fn text_entry(kind: &str, text: &str) -> serde_json::Value {
    json!({"type": kind, "text": text})
}

fn turn(state: serde_json::Value) -> serde_json::Value {
    json!({"event": "turn", "state": state})
}

fn queue(slot: serde_json::Value) -> serde_json::Value {
    json!({"event": "queue", "slot": slot})
}

fn permission_entry(call: Option<&str>, title: &str) -> serde_json::Value {
    let mut entry = json!({
        "type": "permission",
        "title": title,
        "options": [
            {"id": "allow-once", "name": "Allow", "kind": "allow_once"},
            {"id": "reject-once", "name": "Reject", "kind": "reject_once"},
        ],
    });
    if let Some(call) = call {
        entry["call"] = json!(call);
    }
    entry
}

fn ready() -> serde_json::Value {
    json!({"event": "agent", "state": {"agent": "ready", "name": "fake-acp-agent", "version": "1.0.0"}})
}

/// The agent's ACP session id in the agent fixtures.
const CONVERSATION: &str = "sess-4f1c";
const KEY_2: &str = "0d7e3a91c4b25f68e1a9d0c3b7f24e85";

fn conversation(continuation: &str) -> serde_json::Value {
    json!({"event": "conversation", "conversation": {"id": CONVERSATION, "cwd": "/home/u/src", "continuation": continuation}})
}

fn notice(text: &str) -> serde_json::Value {
    json!({"type": "notice", "level": "info", "text": text})
}

const AGENT: &str = "fake-acp-agent --script /home/u/turn.script";

fn agent_info(session: u32, activity: client::AgentActivity) -> client::SessionInfo {
    client::SessionInfo {
        session,
        pid: 5000 + session,
        command: AGENT.into(),
        cwd: Some("/home/u/src".into()),
        policy: Policy::Detach,
        key: format!("{session:032x}"),
        started: 1_794_000_000,
        profile: Some("Agent".into()),
        owned: true,
        detached_secs: 30,
        agent: Some(client::AgentInfo {
            activity,
            name: Some("fake-acp-agent".into()),
        }),
        ..Default::default()
    }
}

/// An `AGENT_FETCH`.
fn fetch(req: u64, session: u32, before: u64, max_bytes: u32) -> Step {
    c(client::ToCore::AgentFetch(client::AgentFetch {
        req,
        session,
        before,
        max_bytes,
    }))
}

/// One `AGENT_PAGE` frame of `events`, the last when `last` is given.
fn page(req: u64, session: u32, events: Vec<serde_json::Value>, last: Option<bool>) -> Step {
    s(client::ToClient::AgentPage(client::AgentPage {
        req,
        session,
        events: events.into_iter().map(sorted).collect(),
        last: last.map(|more| client::PageEnd { more }),
    }))
}

fn window(from: u64, earlier: u64) -> serde_json::Value {
    json!({"event": "window", "from": from, "earlier": earlier})
}

fn trimmed(through: u64, dropped: u64) -> serde_json::Value {
    json!({"event": "trimmed", "through": through, "dropped": dropped})
}

/// Transcripts added by `agents-log-paging`, within protocol 4 and H1 3
/// before they shipped (its design decision 2).
fn paging_transcripts() -> Vec<Transcript> {
    let attach = |req: u64, session: u32, page: Option<u32>| {
        c(client::ToCore::Attach(client::Attach {
            req,
            session,
            cols: 0,
            rows: 0,
            page,
        }))
    };
    let replay_start = |req: u64, session: u32| {
        s(client::ToClient::ReplayStart(client::ReplayStart {
            req: Some(req),
            session,
            pid: 5000 + session,
        }))
    };
    let replay_end = |session: u32| s(client::ToClient::ReplayEnd(client::SessionRef { session }));
    vec![
        Transcript {
            name: "client-agent-attach-paged",
            protocol: Protocol::Client4,
            about: "Protocol 4 with paging: attaching a long agent session with a smaller page asked for, its replay a page of the newest entries behind a trim and a window, then live events, one below the window (ignored by the client) and one above it.",
            steps: vec![
                hello_4(TOKEN),
                welcome_4(),
                attach(2, 2, Some(16_384)),
                replay_start(2, 2),
                transcript(
                    2,
                    vec![
                        trimmed(40, 40),
                        window(118, 77),
                        added(118, text_entry("user", "and then the tests")),
                        added(
                            119,
                            json!({"type": "tool", "call": "call_9", "title": "Run cargo test", "kind": "execute", "status": "in_progress"}),
                        ),
                        added(120, text_entry("agent", "The tests are running.")),
                        json!({"event": "conversation", "conversation": {"id": CONVERSATION, "cwd": "/home/u/src", "continuation": "resume", "title": "Notes and tests"}}),
                        turn(json!({"turn": "working"})),
                        ready(),
                    ],
                ),
                replay_end(2),
                transcript(
                    2,
                    vec![
                        json!({"event": "tool_changed", "id": 60, "status": "completed"}),
                        json!({"event": "tool_changed", "id": 119, "status": "completed"}),
                        json!({"event": "appended", "id": 120, "text": " They pass."}),
                        turn(json!({"turn": "ended", "stop": {"reason": "end_turn"}})),
                    ],
                ),
            ],
        },
        Transcript {
            name: "client-agent-fetch",
            protocol: Protocol::Client4,
            about: "Protocol 4 with paging: an attach's page behind a window, a fetch answered in two frames with more to come, a trim that takes the window while a second fetch is out, and that fetch's empty page.",
            steps: vec![
                hello_4(TOKEN),
                welcome_4(),
                attach(2, 3, None),
                replay_start(2, 3),
                transcript(
                    3,
                    vec![
                        window(5, 4),
                        added(5, text_entry("agent", "The fifth entry.")),
                        turn(json!({"turn": "ended", "stop": {"reason": "end_turn"}})),
                        ready(),
                    ],
                ),
                replay_end(3),
                fetch(3, 3, 5, 262_144),
                page(
                    3,
                    3,
                    vec![
                        added(3, text_entry("user", "Write notes.md")),
                        added(4, permission_entry(Some("call_1"), "Write notes.md")),
                        json!({"event": "answered", "id": 4, "outcome": {"outcome": "selected", "option": "allow-once"}}),
                    ],
                    None,
                ),
                page(3, 3, vec![], Some(true)),
                fetch(4, 3, 3, 262_144),
                transcript(3, vec![trimmed(2, 2)]),
                page(4, 3, vec![], Some(false)),
                prompt(5, 3, "and then the tests"),
                ok(5),
                transcript(
                    3,
                    vec![
                        added(6, text_entry("user", "and then the tests")),
                        turn(json!({"turn": "working"})),
                    ],
                ),
            ],
        },
        Transcript {
            name: "client-agent-fetch-refused",
            protocol: Protocol::Client4,
            about: "Protocol 4 with paging: fetches refused for a session the core doesn't have, a terminal session, an agent session attached to no one here, and one whose replay hasn't ended; and one from a protocol 3 client.",
            steps: vec![
                hello_4(TOKEN),
                welcome_4(),
                fetch(2, 9, 5, 262_144),
                error(Some(2), Some(9), "no session 9"),
                fetch(3, 1, 5, 262_144),
                error(Some(3), Some(1), "session 1 isn't an agent session"),
                fetch(4, 2, 5, 262_144),
                error(Some(4), Some(2), "session 2 isn't attached to this client"),
                attach(5, 2, None),
                fetch(6, 2, 5, 262_144),
                error(Some(6), Some(2), "session 2's replay hasn't ended"),
                replay_start(5, 2),
                transcript(2, vec![window(5, 4), added(5, text_entry("agent", "hi")), ready()]),
                replay_end(2),
            ],
        },
        Transcript {
            name: "client-agent-fetch-old-client",
            protocol: Protocol::Client3,
            about: "A protocol 3 client sending AGENT_FETCH is refused.",
            steps: vec![
                c(client::ToCore::Hello(client::Hello {
                    min: 1,
                    max: 3,
                    client_version: "0.5.0".into(),
                    client_token: TOKEN.into(),
                    ..Default::default()
                })),
                s(client::ToClient::Welcome(client::Welcome {
                    protocol: 3,
                    core_version: "0.6.0".into(),
                    core_build: "e4a3".into(),
                    pid: 4242,
                    holder_protocols: vec![3],
                })),
                fetch(2, 2, 5, 262_144),
                error(Some(2), Some(2), "AGENT_FETCH needs client protocol 4"),
            ],
        },
        Transcript {
            name: "holder-checkpoint-append",
            protocol: Protocol::Holder,
            about: "H1 version 3 with appended checkpoints: an agent session's whole checkpoint, two appends to it, the core leaving, and on the next core's connection the adoption, the stored checkpoint coming back as the three parts in one blob.",
            steps: [
                vec![
                    ch(holder::ToHolder::Hello(holder::Hello {
                        min: 1,
                        max: 3,
                        core_build: "e4a3".into(),
                        instance: 1,
                    })),
                    h(holder::ToCore::Welcome(holder::Welcome {
                        protocol: 3,
                        build: "e4a3".into(),
                        pid: 4243,
                        previous: Previous::None,
                        sessions: Vec::new(),
                    })),
                    ch(holder::ToHolder::SpawnPipes(holder::SpawnPipes {
                        session: 2,
                        command: "fake-acp-agent".into(),
                        args: vec!["--script".into(), "/home/u/turn.script".into()],
                        cwd: Some("/home/u/src".into()),
                        env: BTreeMap::from([("HOME".into(), "/home/u".into())]),
                        meta: meta("detach", AGENT),
                    })),
                    h(holder::ToCore::Spawned(holder::Spawned { session: 2, pid: 5002 })),
                ],
                checkpoint_to_holder(2, APPEND_BASE, 16, 120),
                append_to_holder(2, APPEND_FIRST, 16, 64),
                append_to_holder(2, APPEND_SECOND, 16, 30),
                vec![
                    ch(holder::ToHolder::Goodbye(holder::Goodbye { handover: false })),
                    ch(holder::ToHolder::Hello(holder::Hello {
                        min: 1,
                        max: 3,
                        core_build: "e4a3".into(),
                        instance: 2,
                    })),
                    h(holder::ToCore::Welcome(holder::Welcome {
                        protocol: 3,
                        build: "e4a3".into(),
                        pid: 4243,
                        previous: Previous::Crash,
                        sessions: vec![holder::HeldSession {
                            session: 2,
                            pid: 5002,
                            cols: 0,
                            rows: 0,
                            exit: None,
                            meta: meta("detach", AGENT),
                            truncated: false,
                            mode: holder::SpawnMode::Pipes,
                        }],
                    })),
                ],
                adopted(2, &[APPEND_BASE, APPEND_FIRST, APPEND_SECOND].concat(), b"{\"jsonrpc\"", 16),
            ]
            .concat(),
        },
        Transcript {
            name: "holder-checkpoint-append-refused",
            protocol: Protocol::Holder,
            about: "H1 with appended checkpoints refused: an append onto a session with no stored checkpoint, one announcing more than the 64 MiB limit, and on the next core's connection one from a core that negotiated version 2.",
            steps: [
                vec![
                    ch(holder::ToHolder::Hello(holder::Hello {
                        min: 1,
                        max: 3,
                        core_build: "e4a3".into(),
                        instance: 1,
                    })),
                    h(holder::ToCore::Welcome(holder::Welcome {
                        protocol: 3,
                        build: "e4a3".into(),
                        pid: 4243,
                        previous: Previous::None,
                        sessions: Vec::new(),
                    })),
                    ch(holder::ToHolder::SpawnPipes(holder::SpawnPipes {
                        session: 2,
                        command: "fake-acp-agent".into(),
                        args: Vec::new(),
                        cwd: None,
                        env: BTreeMap::new(),
                        meta: meta("detach", AGENT),
                    })),
                    h(holder::ToCore::Spawned(holder::Spawned { session: 2, pid: 5002 })),
                ],
                append_to_holder(2, APPEND_FIRST, 16, 64),
                vec![
                    h(holder::ToCore::Error(holder::ErrorMessage {
                        session: Some(2),
                        message: "the checkpoint append was refused: nothing to append to".into(),
                    })),
                    ch(holder::ToHolder::CheckpointAppend(holder::BlobBegin {
                        session: 2,
                        len: holder::MAX_CHECKPOINT_LEN + 1,
                    })),
                    h(holder::ToCore::Error(holder::ErrorMessage {
                        session: Some(2),
                        message: "session 2's blob is 67108865 bytes; the limit is 67108864".into(),
                    })),
                    ch(holder::ToHolder::Goodbye(holder::Goodbye { handover: false })),
                    ch(holder::ToHolder::Hello(holder::Hello {
                        min: 1,
                        max: 2,
                        core_build: "d3f2".into(),
                        instance: 2,
                    })),
                    h(holder::ToCore::Welcome(holder::Welcome {
                        protocol: 2,
                        build: "e4a3".into(),
                        pid: 4243,
                        previous: Previous::Crash,
                        sessions: Vec::new(),
                    })),
                    ch(holder::ToHolder::CheckpointAppend(holder::BlobBegin { session: 2, len: 5 })),
                    h(holder::ToCore::Error(holder::ErrorMessage {
                        session: Some(2),
                        message: "CHECKPOINT_APPEND needs H1 version 3".into(),
                    })),
                ],
            ]
            .concat(),
        },
    ]
}

/// A checkpoint's base and two appended segments, as the core's format 2
/// would write them (the holder reads none of it).
const APPEND_BASE: &[u8] = b"BACP\x00\x02B\x00\x00\x00\x00{\"log\":[]}";
const APPEND_FIRST: &[u8] = b"S\x00\x00\x00\x01{\"events\":[1]}";
const APPEND_SECOND: &[u8] = b"S\x00\x00\x00\x02{\"events\":[2]}";

fn append_to_holder(session: u32, bytes: &[u8], chunk_len: usize, covers: u64) -> Vec<Step> {
    let mut steps = checkpoint_to_holder(session, bytes, chunk_len, covers);
    let Step::CoreToHolder(holder::ToHolder::CheckpointBegin(begin)) = steps[0].clone() else {
        unreachable!("a checkpoint starts with its begin");
    };
    steps[0] = ch(holder::ToHolder::CheckpointAppend(begin));
    steps
}

fn agent_transcripts() -> Vec<Transcript> {
    let held_waiting = "and then the tests";
    vec![
        Transcript {
            name: "client-agent-session",
            protocol: Protocol::Client4,
            about: "Protocol 4: an agent session's spawn and its conversation, a turn with a waiting prompt discarded and queued again, a permission answered (and answered again, refused), a cancel holding the waiting prompt, its resend, a permission answered cancelled, the agent's exit and its restart continuing the conversation, a continuing spawn into a new session, and the closes.",
            steps: vec![
                hello_4(TOKEN),
                welcome_4(),
                c(client::ToCore::SpawnAgent(client::SpawnAgent {
                    req: 1,
                    command: "fake-acp-agent".into(),
                    args: vec!["--script".into(), "/home/u/turn.script".into()],
                    cwd: Some("~/src".into()),
                    env: BTreeMap::from([
                        ("HOME".into(), "/home/u".into()),
                        ("PATH".into(), "/usr/local/bin:/usr/bin".into()),
                    ]),
                    policy: Policy::Detach,
                    profile: Some("Agent".into()),
                    conversation: None,
                    start: None,
                })),
                s(client::ToClient::Spawned(client::Spawned {
                    req: 1,
                    session: 1,
                    pid: 5000,
                    key: KEY.into(),
                })),
                transcript(1, vec![conversation("resume_and_load"), ready()]),
                prompt(2, 1, "Write notes.md"),
                ok(2),
                transcript(
                    1,
                    vec![
                        added(1, text_entry("user", "Write notes.md")),
                        turn(json!({"turn": "working"})),
                    ],
                ),
                transcript(1, vec![added(2, text_entry("agent", "I'll"))]),
                transcript(1, vec![json!({"event": "appended", "id": 2, "text": " write it.\n"})]),
                prompt(3, 1, held_waiting),
                ok(3),
                transcript(1, vec![queue(json!({"slot": "waiting", "text": held_waiting}))]),
                prompt(4, 1, "and lint"),
                error(Some(4), Some(1), "a prompt is already queued"),
                c(client::ToCore::AgentUnqueue(client::AgentRequest { req: 5, session: 1 })),
                ok(5),
                transcript(1, vec![queue(json!({"slot": "empty"}))]),
                prompt(6, 1, held_waiting),
                ok(6),
                transcript(1, vec![queue(json!({"slot": "waiting", "text": held_waiting}))]),
                transcript(
                    1,
                    vec![
                        added(
                            3,
                            json!({"type": "tool", "call": "call_1", "title": "Write notes.md", "kind": "edit", "status": "pending"}),
                        ),
                        added(4, permission_entry(Some("call_1"), "Write notes.md")),
                        turn(json!({"turn": "blocked"})),
                    ],
                ),
                answer(7, 1, 4, Some("allow-once")),
                ok(7),
                transcript(
                    1,
                    vec![
                        json!({"event": "answered", "id": 4, "outcome": {"outcome": "selected", "option": "allow-once"}}),
                        turn(json!({"turn": "working"})),
                        json!({"event": "tool_changed", "id": 3, "status": "completed"}),
                    ],
                ),
                answer(8, 1, 4, Some("reject-once")),
                error(Some(8), Some(1), "permission request 4 was already answered"),
                c(client::ToCore::AgentCancel(client::SessionRef { session: 1 })),
                transcript(
                    1,
                    vec![
                        queue(json!({"slot": "held", "text": held_waiting, "reason": {"hold": "cancelled"}})),
                        turn(json!({"turn": "cancelling"})),
                    ],
                ),
                transcript(1, vec![turn(json!({"turn": "ended", "stop": {"reason": "cancelled"}}))]),
                c(client::ToCore::AgentResend(client::AgentRequest { req: 9, session: 1 })),
                ok(9),
                transcript(
                    1,
                    vec![
                        queue(json!({"slot": "empty"})),
                        added(5, text_entry("user", held_waiting)),
                        turn(json!({"turn": "working"})),
                    ],
                ),
                transcript(
                    1,
                    vec![
                        added(6, permission_entry(None, "Run cargo test")),
                        turn(json!({"turn": "blocked"})),
                    ],
                ),
                answer(10, 1, 6, None),
                ok(10),
                transcript(
                    1,
                    vec![
                        json!({"event": "answered", "id": 6, "outcome": {"outcome": "cancelled"}}),
                        turn(json!({"turn": "working"})),
                    ],
                ),
                transcript(
                    1,
                    vec![
                        added(7, text_entry("agent", "I didn't run the tests.")),
                        turn(json!({"turn": "ended", "stop": {"reason": "end_turn"}})),
                    ],
                ),
                c(client::ToCore::AgentUnqueue(client::AgentRequest { req: 11, session: 1 })),
                error(Some(11), Some(1), "nothing is queued"),
                // The agent exits; its conversation can be continued, so the
                // session stays, and the client restarts it.
                transcript(
                    1,
                    vec![json!({"event": "agent", "state": {"agent": "exited", "code": 1, "stderr": "fatal: out of memory\n"}})],
                ),
                c(client::ToCore::AgentRestart(client::AgentRestart {
                    req: 12,
                    session: 1,
                    command: "fake-acp-agent".into(),
                    args: vec!["--script".into(), "/home/u/again.script".into()],
                    env: BTreeMap::from([
                        ("HOME".into(), "/home/u".into()),
                        ("PATH".into(), "/usr/local/bin:/usr/bin".into()),
                    ]),
                })),
                ok(12),
                transcript(
                    1,
                    vec![
                        added(8, notice("the agent was started again to continue the conversation")),
                        json!({"event": "agent", "state": {"agent": "starting"}}),
                    ],
                ),
                transcript(1, vec![conversation("resume_and_load"), ready()]),
                // A client whose session the server lost continues the
                // conversation in a new one.
                c(client::ToCore::SpawnAgent(client::SpawnAgent {
                    req: 13,
                    command: "fake-acp-agent".into(),
                    args: vec!["--script".into(), "/home/u/load.script".into()],
                    cwd: None,
                    env: BTreeMap::from([("HOME".into(), "/home/u".into())]),
                    policy: Policy::Detach,
                    profile: Some("Agent".into()),
                    conversation: Some(client::AgentConversation {
                        id: CONVERSATION.into(),
                        cwd: "/home/u/src".into(),
                    }),
                    start: None,
                })),
                s(client::ToClient::Spawned(client::Spawned {
                    req: 13,
                    session: 2,
                    pid: 5001,
                    key: KEY_2.into(),
                })),
                transcript(
                    2,
                    vec![
                        added(1, notice("this continues an earlier conversation; what follows is the agent's replay of it")),
                        added(2, text_entry("user", "Write notes.md")),
                        added(3, text_entry("agent", "I'll write it.\n")),
                        conversation("resume_and_load"),
                        ready(),
                    ],
                ),
                c(client::ToCore::Close(client::SessionRef { session: 1 })),
                transcript(
                    1,
                    vec![json!({"event": "agent", "state": {"agent": "exited", "stderr": ""}})],
                ),
                exit(1, None),
                c(client::ToCore::Close(client::SessionRef { session: 2 })),
                transcript(
                    2,
                    vec![json!({"event": "agent", "state": {"agent": "exited", "stderr": ""}})],
                ),
                exit(2, None),
            ],
        },
        // Added by `agents-login`, within protocol 4 before it shipped (its
        // design decision 2).
        Transcript {
            name: "client-agent-login",
            protocol: Protocol::Client4,
            about: "Protocol 4: an agent that needs a login at its start, offering two methods; a login refused for a method it doesn't offer, one accepted and cancelled, one accepted again and answered, and the session ready.",
            steps: vec![
                hello_4(TOKEN),
                welcome_4(),
                c(client::ToCore::SpawnAgent(client::SpawnAgent {
                    req: 1,
                    command: "fake-acp-agent".into(),
                    args: vec!["--script".into(), "/home/u/login.script".into()],
                    cwd: Some("~/src".into()),
                    env: BTreeMap::from([("HOME".into(), "/home/u".into())]),
                    policy: Policy::Detach,
                    profile: Some("Agent".into()),
                    conversation: None,
                    start: None,
                })),
                s(client::ToClient::Spawned(client::Spawned {
                    req: 1,
                    session: 1,
                    pid: 5000,
                    key: KEY.into(),
                })),
                transcript(1, vec![login_required("Authentication required")]),
                login(2, 1, "no-such-method"),
                error(
                    Some(2),
                    Some(1),
                    "session 1: the agent offers no login method \"no-such-method\"",
                ),
                login(3, 1, "oauth-personal"),
                ok(3),
                transcript(1, vec![logging_in()]),
                c(client::ToCore::AgentCancel(client::SessionRef { session: 1 })),
                transcript(1, vec![login_required("Authentication required")]),
                login(4, 1, "oauth-personal"),
                ok(4),
                transcript(1, vec![logging_in()]),
                transcript(1, vec![json!({"event": "agent", "state": {"agent": "starting"}})]),
                transcript(1, vec![conversation("resume_and_load"), ready()]),
                c(client::ToCore::Close(client::SessionRef { session: 1 })),
                transcript(
                    1,
                    vec![json!({"event": "agent", "state": {"agent": "exited", "stderr": ""}})],
                ),
                exit(1, None),
            ],
        },
        Transcript {
            name: "client-agent-attach",
            protocol: Protocol::Client4,
            about: "Protocol 4: listing agent sessions, attaching one blocked on a permission request (its replay, then live events), a refused resize, a layout, and the list after a detach.",
            steps: vec![
                hello_4(TOKEN),
                welcome_4(),
                c(client::ToCore::List(client::Request { req: 1 })),
                s(client::ToClient::Sessions(client::Sessions {
                    req: 1,
                    sessions: vec![
                        client::SessionInfo {
                            session: 1,
                            pid: 5001,
                            command: "bash -l".into(),
                            cwd: Some("/home/u".into()),
                            policy: Policy::Detach,
                            cols: 80,
                            rows: 24,
                            key: KEY.into(),
                            ..Default::default()
                        },
                        client::SessionInfo {
                            layout: Some(sorted(json!({"v": 1, "zones": {"active": true, "key": format!("{:032x}", 2)}}))),
                            ..agent_info(2, client::AgentActivity::Blocked)
                        },
                        agent_info(3, client::AgentActivity::Idle),
                        agent_info(4, client::AgentActivity::Done),
                        agent_info(5, client::AgentActivity::Error),
                    ],
                })),
                c(client::ToCore::Attach(client::Attach {
                    req: 2,
                    session: 2,
                    cols: 0,
                    rows: 0,
                    page: None,
                })),
                s(client::ToClient::ReplayStart(client::ReplayStart {
                    req: Some(2),
                    session: 2,
                    pid: 5002,
                })),
                transcript(
                    2,
                    vec![
                        added(1, text_entry("user", "Write notes.md")),
                        added(2, text_entry("thought", "The user wants a file.")),
                        added(3, permission_entry(Some("call_1"), "Write notes.md")),
                        queue(json!({"slot": "waiting", "text": "and then the tests"})),
                        turn(json!({"turn": "blocked"})),
                        ready(),
                    ],
                ),
                s(client::ToClient::ReplayEnd(client::SessionRef { session: 2 })),
                answer(3, 2, 3, Some("reject-once")),
                ok(3),
                transcript(
                    2,
                    vec![
                        json!({"event": "answered", "id": 3, "outcome": {"outcome": "selected", "option": "reject-once"}}),
                        turn(json!({"turn": "working"})),
                    ],
                ),
                c(client::ToCore::Resize(client::Resize {
                    session: 2,
                    cols: 80,
                    rows: 24,
                    cell_w: 9,
                    cell_h: 18,
                })),
                error(None, Some(2), "session 2 is an agent session, which has no size"),
                transcript(
                    2,
                    vec![
                        added(4, text_entry("agent", "I won't write it, then.")),
                        turn(json!({"turn": "ended", "stop": {"reason": "end_turn"}})),
                        queue(json!({"slot": "empty"})),
                        added(5, text_entry("user", "and then the tests")),
                        turn(json!({"turn": "working"})),
                    ],
                ),
                c(client::ToCore::SetLayout(client::SetLayout {
                    session: 2,
                    layout: Some(sorted(json!({"v": 1, "zones": {"key": format!("{:032x}", 2)}}))),
                })),
                c(client::ToCore::Detach(client::Detach {
                    session: 2,
                    layout: Some(sorted(json!({"v": 1, "zones": {"active": true, "key": format!("{:032x}", 2)}}))),
                })),
                c(client::ToCore::List(client::Request { req: 4 })),
                s(client::ToClient::Sessions(client::Sessions {
                    req: 4,
                    sessions: vec![client::SessionInfo {
                        owned: false,
                        detached_secs: 0,
                        layout: Some(sorted(json!({"v": 1, "zones": {"active": true, "key": format!("{:032x}", 2)}}))),
                        ..agent_info(2, client::AgentActivity::Working)
                    }],
                })),
            ],
        },
        Transcript {
            name: "client-agent-old-client",
            protocol: Protocol::Client3,
            about: "A protocol 3 client of a core with agent sessions: the list leaves them out, and attaching one by its id is refused.",
            steps: vec![
                c(client::ToCore::Hello(client::Hello {
                    min: 1,
                    max: 3,
                    client_version: "0.5.0".into(),
                    client_token: TOKEN.into(),
                    ..Default::default()
                })),
                s(client::ToClient::Welcome(client::Welcome {
                    protocol: 3,
                    core_version: "0.6.0".into(),
                    core_build: "e4a3".into(),
                    pid: 4242,
                    holder_protocols: vec![3],
                })),
                c(client::ToCore::List(client::Request { req: 1 })),
                s(client::ToClient::Sessions(client::Sessions {
                    req: 1,
                    sessions: vec![client::SessionInfo {
                        session: 1,
                        pid: 5001,
                        command: "bash -l".into(),
                        cwd: Some("/home/u".into()),
                        policy: Policy::Detach,
                        cols: 80,
                        rows: 24,
                        key: KEY.into(),
                        ..Default::default()
                    }],
                })),
                c(client::ToCore::Attach(client::Attach {
                    req: 2,
                    session: 2,
                    cols: 80,
                    rows: 24,
                    page: None,
                })),
                error(
                    Some(2),
                    Some(2),
                    "session 2 is an agent session, which needs client protocol 4",
                ),
            ],
        },
        Transcript {
            name: "holder-spawn-pipes",
            protocol: Protocol::Holder,
            about: "H1 version 3: a program on plain pipes, its JSON lines both ways, a refused resize, a failed spawn, and its stderr tail before its exit.",
            steps: vec![
                ch(holder::ToHolder::Hello(holder::Hello {
                    min: 1,
                    max: 3,
                    core_build: "e4a3".into(),
                    instance: 1,
                })),
                h(holder::ToCore::Welcome(holder::Welcome {
                    protocol: 3,
                    build: "e4a3".into(),
                    pid: 4243,
                    previous: Previous::None,
                    sessions: Vec::new(),
                })),
                ch(holder::ToHolder::SpawnPipes(holder::SpawnPipes {
                    session: 1,
                    command: "fake-acp-agent".into(),
                    args: vec!["--script".into(), "/home/u/turn.script".into()],
                    cwd: Some("/home/u/src".into()),
                    env: BTreeMap::from([("HOME".into(), "/home/u".into())]),
                    meta: meta("detach", AGENT),
                })),
                h(holder::ToCore::Spawned(holder::Spawned {
                    session: 1,
                    pid: 5000,
                })),
                ch(holder::ToHolder::Input(holder::Data {
                    session: 1,
                    bytes: br#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":1,"clientCapabilities":{}}}"#
                        .iter()
                        .copied()
                        .chain(*b"\n")
                        .collect(),
                })),
                h(holder::ToCore::Output(holder::Data {
                    session: 1,
                    bytes: b"{\"jsonrpc\":\"2.0\",\"id\":0,\"result\":{\"protocolVersion\":1,\"text\":\"tab\\t\\r\\n\xc3\xa9\"}}\n".to_vec(),
                })),
                ch(holder::ToHolder::Resize(holder::Resize {
                    session: 1,
                    cols: 80,
                    rows: 24,
                    cell_w: 9,
                    cell_h: 18,
                })),
                h(holder::ToCore::Error(holder::ErrorMessage {
                    session: Some(1),
                    message: "session 1 is on pipes and has no size".into(),
                })),
                ch(holder::ToHolder::SpawnPipes(holder::SpawnPipes {
                    session: 2,
                    command: "/nonexistent".into(),
                    args: Vec::new(),
                    cwd: None,
                    env: BTreeMap::new(),
                    meta: meta("kill", "/nonexistent"),
                })),
                h(holder::ToCore::SpawnFailed(holder::SpawnFailed {
                    session: 2,
                    message: "cannot run /nonexistent: No such file or directory (os error 2)"
                        .into(),
                })),
                ch(holder::ToHolder::Hangup(holder::SessionRef { session: 1 })),
                h(holder::ToCore::StderrTail(holder::Data {
                    session: 1,
                    bytes: b"stdin closed; exiting\n".to_vec(),
                })),
                h(holder::ToCore::Exited(holder::Exited {
                    session: 1,
                    code: Some(0),
                })),
            ],
        },
        Transcript {
            name: "holder-pipes-adopt",
            protocol: Protocol::Holder,
            about: "H1 version 3: a core adopts a holder with a terminal session and two pipe sessions, one running and one that exited with no core, whose stderr tail follows its tail.",
            steps: [
                vec![
                    ch(holder::ToHolder::Hello(holder::Hello {
                        min: 1,
                        max: 3,
                        core_build: "e4a3".into(),
                        instance: 2,
                    })),
                    h(holder::ToCore::Welcome(holder::Welcome {
                        protocol: 3,
                        build: "e4a3".into(),
                        pid: 4243,
                        previous: Previous::Crash,
                        sessions: vec![
                            holder::HeldSession {
                                session: 1,
                                pid: 5001,
                                cols: 80,
                                rows: 24,
                                exit: None,
                                meta: meta("detach", "bash -l"),
                                truncated: false,
                                mode: holder::SpawnMode::Pty,
                            },
                            holder::HeldSession {
                                session: 2,
                                pid: 5002,
                                cols: 0,
                                rows: 0,
                                exit: None,
                                meta: meta("detach", AGENT),
                                truncated: false,
                                mode: holder::SpawnMode::Pipes,
                            },
                            holder::HeldSession {
                                session: 3,
                                pid: 5003,
                                cols: 0,
                                rows: 0,
                                exit: Some(holder::ExitStatus { code: Some(1) }),
                                meta: meta("kill", AGENT),
                                truncated: false,
                                mode: holder::SpawnMode::Pipes,
                            },
                        ],
                    })),
                ],
                adopted(1, CHECKPOINT, b"ls\r\n", 16),
                adopted(
                    2,
                    b"agent-checkpoint 1\n{\"transcript\":[]}",
                    b"{\"jsonrpc\":\"2.0\",\"method\":\"session/update\",\"params\":{}}\n{\"jsonrpc\"",
                    16,
                ),
                adopted(3, b"agent-checkpoint 1\n{}", b"", 16),
                vec![
                    h(holder::ToCore::StderrTail(holder::Data {
                        session: 3,
                        bytes: b"fatal: no model\n".to_vec(),
                    })),
                    ch(holder::ToHolder::Input(holder::Data {
                        session: 2,
                        bytes: b"{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"session/cancel\"}\n".to_vec(),
                    })),
                ],
            ]
            .concat(),
        },
        Transcript {
            name: "holder-pipes-old-core",
            protocol: Protocol::Holder,
            about: "An H1 version 2 core adopting a version 3 holder with a pipe session: WELCOME lists only the terminal session, and only its output is sent.",
            steps: [
                vec![
                    ch(holder::ToHolder::Hello(holder::Hello {
                        min: 1,
                        max: 2,
                        core_build: "d3f2".into(),
                        instance: 3,
                    })),
                    h(holder::ToCore::Welcome(holder::Welcome {
                        protocol: 2,
                        build: "e4a3".into(),
                        pid: 4243,
                        previous: Previous::Handover,
                        sessions: vec![holder::HeldSession {
                            session: 1,
                            pid: 5001,
                            cols: 80,
                            rows: 24,
                            exit: None,
                            meta: meta("detach", "bash -l"),
                            truncated: false,
                            mode: holder::SpawnMode::Pty,
                        }],
                    })),
                ],
                adopted(1, CHECKPOINT, b"", 16),
                vec![h(holder::ToCore::Output(holder::Data {
                    session: 1,
                    bytes: b"$ ".to_vec(),
                }))],
            ]
            .concat(),
        },
    ]
}
