//! An agent session's checkpoint (design decision 11 of the OpenSpec change
//! `agents-sessions`): what the core stores in the session's holder so that
//! the next core, after a crash or an upgrade, carries on the conversation.
//!
//! **The format** (2, OpenSpec change `agents-log-paging`, design decision
//! 7): a header, then a base record and any number of segment records, so
//! the core can append what changed since its last checkpoint
//! (`CHECKPOINT_APPEND`) instead of rewriting the whole log:
//!
//! | bytes | what |
//! | --- | --- |
//! | 4 | [`MAGIC`], `BACP` |
//! | 2 | the format, big-endian: [`FORMAT`] (2) |
//! | then, repeated | a record |
//! | 1 | its kind: `B` (the base, first and only once) or `S` (a segment) |
//! | 4 | its sequence number, big-endian: the base 0, each segment one more |
//! | 4 | the JSON state's length in bytes, big-endian |
//! | that many | the JSON state: one object |
//! | 4 | the partial line's length, big-endian (0 unless [`Partial::Collecting`]) |
//! | that many | the partial line's bytes, from `lines.from` on |
//!
//! **The state** is in the *delta shape* (OpenSpec change
//! `agents-checkpoint-deltas`, which amended format 2 in place). The base's
//! holds the log as the events of [`Transcript::replay`] (the queue slot,
//! the turn and the agent's state among them, streamed from the session
//! straight into the record), the ACP version (`acp`, always 1 here), the
//! cwd and version, the [`Phase`] (with the ACP session id once ready,
//! what a spawn starts or continues, what `initialize` offered, a load's
//! muted count, and a login's message and what follows it), the [`Rpc`]
//! parts (the next outgoing id, the core's requests awaiting a response
//! and what each was for, the agent's waiting permission requests by entry
//! with their JSON-RPC ids as sent), the chunk run, `tool_epoch` (the first
//! entry id of the running agent process), the outputs still appended to
//! by the agent's terminal id (`outputs`: each one's entry, its `Output`
//! item and its plain-text filter's state, a half-received escape sequence
//! or character included; left out when there are none), the count of a
//! bad-line run not yet written as a notice, the [`Lines`] state (which
//! partial it is and `covers`, every stdout byte fed, the partial line's
//! included), the login methods the agent advertised (`methods`, left out
//! when there are none), `lists` and `chose` (left out when false), and
//! each held option's key by its id (`option_keys`) and the mode values
//! last in effect (`modes`), each left out when empty; the commands and
//! options themselves are log events, and a set of an option out isn't
//! kept. The tool calls by call id aren't written: [`decode`] rebuilds
//! them from the log, as the tool entries from `tool_epoch` on.
//!
//! A segment's state has `events` in place of `log`: the events the session
//! applied since the previous record, in order, with consecutive
//! `Appended`s of one entry merged up to [`TEXT_CHUNK`] and consecutive
//! `ToolOutput`s of one item up to [`ITEM_LIMIT`], the later exit kept
//! (`agents-tool-content` decision 8). Its other fields are written by
//! size (decision 4):
//!
//! | field | in a segment |
//! | --- | --- |
//! | `events` | always |
//! | `lines` and the partial's bytes | always; the bytes as a delta (below) |
//! | `tool_epoch`, `acp`, `cwd`, `version`, `phase`, `rpc`, `run`, `bad_lines`, `lists`, `chose`, `option_keys`, `modes` | always, whole |
//! | `methods` | only when they changed since the last record; `[]` when emptied |
//! | `outputs` | a patch: `{terminal: output}` for each one opened or changed, `{terminal: null}` for each one closed; left out when none changed |
//!
//! A new field bounded under 1 KiB in the typical case and under 64 KiB at
//! worst is written whole in every record; a larger one only when it
//! changed, or as a patch.
//!
//! **The partial line's delta** (decision 3). When the partial line is the
//! one the last record collected, in the same agent process (the same
//! `tool_epoch`), started at the same offset and grown, a segment writes
//! `lines.from`, how many of its bytes are stored already, and only the
//! bytes past them; otherwise `from` is left out and the bytes are whole.
//! A base always writes them whole. [`Written`] is what the core keeps of
//! the last record to tell.
//!
//! [`decode`] folds the base's log, then each segment's events; joins the
//! partial line's bytes; takes the last `methods` written and patches the
//! `outputs`; and takes the rest from the last record. A record with no
//! `tool_epoch` (every one written before `agents-checkpoint-deltas`), a
//! delta that doesn't continue the record before it, and a patch closing an
//! output the chain doesn't hold are unreadable, and so is format 1:
//! checkpoints from before the delta shape aren't read (decision 1).
//! [`encode`] writes the header and a base; [`encode_segment`] one segment
//! record.
//!
//! **Bounds.** A checkpoint is at most [`MAX_CHECKPOINT_LEN`] (64 MiB, what a
//! holder keeps). That a whole one fits is `acp::limits`' invariant I2:
//! the log's stored form is at most twice its 16 MiB measure, beside an
//! 8 MiB partial line, the login methods, 64 open outputs and the rest of
//! the state, `LARGEST_CHECKPOINT` in all, with about 12.7 MiB to spare
//! (`agents-event-bounds` decision 1). [`encode`] still refuses one past
//! it rather than send what the holder drops.
//!
//! **Reading one never panics** (`railway R5`). A checkpoint that isn't one,
//! that a newer core wrote in a format this one doesn't know, or whose state
//! doesn't hold together is adopted as a session whose conversation could
//! not be restored ([`adopt`]): a log holding one notice, the agent
//! `Failed` and left running, its lines ignored, so prompts are refused
//! rather than sent with lost ids.
//!
//! A whole pure module (`fcis` R2, R3; `PURE_MODULES`): time is an argument.

use std::collections::BTreeMap;
use std::time::Instant;

use brindle_remote_protocol::holder::MAX_CHECKPOINT_LEN;
use brindle_transcript::{EntryId, Event, LoginMethod};

use super::lines::Partial;
use super::reduce::{AgentSession, OpenOutput};

mod adopt;
mod format;
mod state;

#[cfg(test)]
mod harness;

pub use adopt::{NOT_RESTORED, adopt};
pub use format::{FORMAT, MAGIC, TooLarge, Unreadable};

use format::{BASE, PREFIX_LEN, SEGMENT, next_record, push_record};
use state::{
    Body, Changes, Folded, array, fold_log, merged, partial_bytes, restore, restore_log,
    state_object, write_state,
};

/// What the stored records leave of the fields a segment writes as changes
/// (OpenSpec change `agents-checkpoint-deltas`, decision 5): the partial
/// line the last record collected, the login methods and the open outputs
/// last written. The core keeps it beside what the holder stores, and the
/// next segment is written against it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Written {
    line: Option<LineMark>,
    methods: Vec<LoginMethod>,
    outputs: BTreeMap<String, OpenOutput>,
}

/// A collected partial line as a record left it: the tool epoch (its agent
/// process), the stream offset its line started at, and how many of its
/// bytes are stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LineMark {
    epoch: EntryId,
    start: u64,
    len: u64,
}

/// What a segment wrote of the fields kept as changes, for
/// [`Written::apply`] once it is appended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrittenChanges {
    line: Option<LineMark>,
    /// The methods, when they changed.
    methods: Option<Vec<LoginMethod>>,
    /// Each output set or, `None`, closed.
    outputs: Vec<(String, Option<OpenOutput>)>,
}

impl Written {
    /// Folds in what an appended segment wrote.
    pub fn apply(&mut self, changes: WrittenChanges) {
        self.line = changes.line;
        if let Some(methods) = changes.methods {
            self.methods = methods;
        }
        for (terminal, open) in changes.outputs {
            match open {
                Some(open) => {
                    self.outputs.insert(terminal, open);
                }
                None => {
                    self.outputs.remove(&terminal);
                }
            }
        }
    }
}

/// A whole checkpoint, and what it leaves for the next segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Encoded {
    pub bytes: Vec<u8>,
    pub written: Written,
}

/// One segment record, and what it changes of [`Written`] once appended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub bytes: Vec<u8>,
    pub changes: WrittenChanges,
}

/// The partial line `session` collects, as a record of it leaves it.
fn line_mark(session: &AgentSession) -> Option<LineMark> {
    match session.process.lines.partial() {
        Partial::Collecting(bytes) => Some(LineMark {
            epoch: session.process.book.tool_epoch,
            start: session.process.lines.covers().saturating_sub(bytes.len() as u64),
            len: bytes.len() as u64,
        }),
        Partial::TooLong { .. } | Partial::Resyncing { .. } => None,
    }
}

/// Where a segment's partial line bytes start (decision 3): past those
/// `last` stored when the partial is the same line, in the same agent
/// process, grown; 0 (written whole) otherwise.
fn line_from(now: Option<LineMark>, last: Option<LineMark>) -> u64 {
    match (now, last) {
        (Some(now), Some(last))
            if now.epoch == last.epoch && now.start == last.start && now.len >= last.len =>
        {
            last.len
        }
        (Some(_) | None, _) => 0,
    }
}

/// Encodes `session` whole (the header and a base record), or refuses
/// past [`MAX_CHECKPOINT_LEN`].
pub fn encode(session: &AgentSession) -> Result<Encoded, TooLarge> {
    encode_within(session, MAX_CHECKPOINT_LEN)
}

fn encode_within(session: &AgentSession, limit: u64) -> Result<Encoded, TooLarge> {
    let mut bytes = Vec::new();
    let written = encode_into(session, &mut bytes, limit)?;
    Ok(Encoded { bytes, written })
}

/// Writes `session` whole into `out`, the log streamed from the session
/// (decision 8); past `limit` it is refused and `out` holds at most the
/// header.
fn encode_into(session: &AgentSession, out: &mut Vec<u8>, limit: u64) -> Result<Written, TooLarge> {
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&FORMAT.to_be_bytes());
    let book = &session.process.book;
    let changes = Changes {
        from: 0,
        methods: (!session.methods.is_empty()).then_some(session.methods.as_slice()),
        outputs: book
            .outputs
            .iter()
            .map(|(terminal, open)| (terminal.as_str(), Some(open)))
            .collect(),
    };
    push_record(
        out,
        BASE,
        0,
        |out| {
            let log = Body::Log(session.transcript.replay_ref());
            write_state(out, session, &changes, log)
        },
        partial_bytes(session),
        limit,
    )?;
    Ok(Written {
        line: line_mark(session),
        methods: session.methods.clone(),
        outputs: book.outputs.clone(),
    })
}

/// One segment record, numbered `seq`, to append to a stored checkpoint
/// whose records left `last`: `events`, the events `session` applied since
/// the previous record, with consecutive `Appended`s of one entry merged;
/// the partial line's bytes past those stored; the methods when they
/// changed, the outputs that changed; and the rest of the session whole.
/// Refused past [`MAX_CHECKPOINT_LEN`] on its own; whether it fits beside
/// what is stored is the core's to judge.
pub fn encode_segment(
    session: &AgentSession,
    events: Vec<Event>,
    seq: u32,
    last: &Written,
) -> Result<Segment, TooLarge> {
    let events = merged(events);
    let line = line_mark(session);
    let from = line_from(line, last.line);
    let methods = (session.methods != last.methods).then_some(session.methods.as_slice());
    let now = &session.process.book.outputs;
    let mut outputs: Vec<(&str, Option<&OpenOutput>)> = now
        .iter()
        .filter(|(terminal, open)| last.outputs.get(*terminal) != Some(*open))
        .map(|(terminal, open)| (terminal.as_str(), Some(open)))
        .collect();
    outputs.extend(
        last.outputs
            .keys()
            .filter(|terminal| !now.contains_key(*terminal))
            .map(|terminal| (terminal.as_str(), None)),
    );
    let changes = Changes {
        from,
        methods,
        outputs,
    };
    let partial = partial_bytes(session);
    let mut bytes = Vec::new();
    push_record(
        &mut bytes,
        SEGMENT,
        seq,
        |out| write_state(out, session, &changes, Body::Events(&events)),
        partial.get(from as usize..).unwrap_or_default(),
        MAX_CHECKPOINT_LEN,
    )?;
    let changes = WrittenChanges {
        line,
        methods: changes.methods.map(<[LoginMethod]>::to_vec),
        outputs: changes
            .outputs
            .into_iter()
            .map(|(terminal, open)| (terminal.to_owned(), open.cloned()))
            .collect(),
    };
    Ok(Segment { bytes, changes })
}

/// The session `bytes` encode, or why they don't. A bad-line run comes
/// back with `now` as its last line's time, so its notice is written a
/// quiet second after the adoption.
pub fn decode(bytes: &[u8], now: Instant) -> Result<AgentSession, Unreadable> {
    let prefix = bytes
        .get(..PREFIX_LEN)
        .ok_or(Unreadable::Short { len: bytes.len() })?;
    let (magic, format) = prefix.split_at(4);
    if magic != MAGIC {
        return Err(Unreadable::NotACheckpoint);
    }
    match u16::from_be_bytes([format[0], format[1]]) {
        FORMAT => decode_2(&bytes[PREFIX_LEN..], now),
        format => Err(Unreadable::Format { format }),
    }
}

/// Format 2: a base record, then segments in sequence (decision 6).
fn decode_2(mut rest: &[u8], now: Instant) -> Result<AgentSession, Unreadable> {
    let Some(base) = next_record(&mut rest)? else {
        return Err(Unreadable::NoBase);
    };
    match base.kind {
        BASE => {}
        SEGMENT => return Err(Unreadable::NoBase),
        kind => return Err(Unreadable::RecordKind { kind }),
    }
    if base.seq != 0 {
        return Err(Unreadable::Sequence {
            expected: 0,
            found: base.seq,
        });
    }
    let mut state = state_object(base.state)?;
    let mut transcript = restore_log(array(&state, "log")?)?;
    let mut folded = Folded::base(&mut state, base.partial)?;
    let mut expected: u32 = 1;
    while let Some(record) = next_record(&mut rest)? {
        if record.kind != SEGMENT {
            return Err(Unreadable::RecordKind { kind: record.kind });
        }
        if record.seq != expected {
            return Err(Unreadable::Sequence {
                expected,
                found: record.seq,
            });
        }
        expected = expected.saturating_add(1);
        state = state_object(record.state)?;
        fold_log(&mut transcript, array(&state, "events")?)?;
        folded.segment(&mut state, record.partial)?;
    }
    restore(&state, transcript, folded, now)
}

#[cfg(test)]
mod tests {
    use brindle_remote_protocol::serde_json::{self, Value, json};
    use brindle_transcript::{Entry, Slot, TEXT_CHUNK};

    use crate::acp::dialect::Dialect;
    use crate::acp::lines::{Lines, Partial};
    use crate::acp::limits;
    use crate::acp::reduce::{self, Command, Input, Output, Phase, Refused, Replay, step};
    use crate::acp::rpc::BadParts;

    use super::format::{Limited, RECORD_HEADER_LEN};
    use super::harness::*;
    use super::*;

    // --- bounds ---------------------------------------------------------

    #[test]
    fn a_checkpoint_past_the_limit_is_refused_with_its_length() {
        let agent = Agent::busy();
        let len = encode(&agent.s).expect("it fits").bytes.len() as u64;
        assert_eq!(
            encode_within(&agent.s, len),
            Ok(encode(&agent.s).expect("it fits"))
        );
        assert_eq!(encode_within(&agent.s, len - 1), Err(TooLarge { len }));
    }

    /// `agents-checkpoint-deltas` decision 8: the state is written straight
    /// into the checkpoint's buffer, which never passes the limit; past it
    /// the rest is only counted, so the refusal still names the length.
    #[test]
    fn a_checkpoint_past_the_limit_is_refused_while_it_is_written() {
        use std::io::Write;
        let mut out = b"head".to_vec();
        let mut limited = Limited::new(&mut out, 10);
        limited.write_all(b"123456").expect("it writes");
        limited.write_all(b"789").expect("past the room, counted");
        limited.write_all(b"0").expect("counted");
        assert_eq!(limited.over(), 4);
        assert_eq!(out, b"head123456");
        // A whole checkpoint a byte past its limit leaves no more than the
        // limit in its buffer before it is refused.
        let agent = Agent::busy();
        let len = encode(&agent.s).expect("it fits").bytes.len() as u64;
        let mut out = Vec::new();
        assert_eq!(encode_into(&agent.s, &mut out, len - 1), Err(TooLarge { len }));
        assert!((out.len() as u64) < len, "{} bytes held", out.len());
    }

    #[test]
    fn a_full_log_a_full_partial_line_and_a_full_prompt_fit_in_64_mib() {
        let mut agent = Agent::ready();
        agent.prompt("go");
        // Entries of text that escapes to twice its length, up to just
        // below the log's bound (16 MiB encoded), where it is largest.
        // Agent and thought chunks alternate, so each is an entry of its own.
        let piece = "\"".repeat(32 * 1024);
        let thought = update(json!({"sessionUpdate": "agent_thought_chunk",
            "content": {"type": "text", "text": piece}}));
        while agent.s.transcript.encoded_len() <= limits::LOG_LIMIT - 300 * 1024 {
            agent.feed(&chunk(&piece));
            agent.feed(&thought);
        }
        assert_eq!(agent.s.transcript.dropped(), 0, "no trim yet");
        assert!(agent.s.transcript.encoded_len() <= limits::LOG_LIMIT);
        assert!(agent.s.transcript.encoded_len() > limits::LOG_LIMIT - 512 * 1024);
        agent.prompt(&"q".repeat(limits::MAX_PROMPT));
        assert!(matches!(agent.s.transcript.slot(), Slot::Waiting { .. }));
        agent.s.process.lines = Lines::restore(
            Partial::Collecting(vec![b'"'; super::super::lines::MAX_LINE + 1]),
            0,
        );
        let bytes = encode(&agent.s).expect("it fits in 64 MiB").bytes;
        assert!(
            (bytes.len() as u64) < MAX_CHECKPOINT_LEN / 2,
            "{}",
            bytes.len()
        );
        assert_eq!(decode(&bytes, agent.now).expect("it reads"), agent.s);
    }

    /// A format 2 checkpoint written before a replay grouped a tool entry's
    /// items, one item a `tool_items` event, still decodes to the same
    /// session (`agents-event-bounds` task 2.3).
    #[test]
    fn a_format_2_checkpoint_written_with_one_item_an_event_still_decodes() {
        let mut agent = Agent::ready();
        agent.prompt("go");
        agent.feed(&update(json!({"sessionUpdate": "tool_call", "toolCallId": "c1",
            "title": "Read", "kind": "read", "status": "completed",
            "content": [
                {"type": "content", "content": {"type": "text", "text": "one"}},
                {"type": "content", "content": {"type": "text", "text": "two"}},
                {"type": "content", "content": {"type": "text", "text": "three"}},
            ]})));
        let bytes = encode(&agent.s).expect("it fits").bytes;
        let (mut state, partial) = state_of(&bytes);
        let log = state["log"].as_array().expect("a log").clone();
        let mut split = Vec::new();
        let mut grouped = 0;
        for event in log {
            match event["event"].as_str() {
                Some("tool_items") if event["items"].as_array().is_some_and(|items| items.len() > 1) => {
                    grouped += 1;
                    let from = event["from"].as_u64().expect("from");
                    for (at, item) in event["items"].as_array().expect("items").iter().enumerate() {
                        let mut one = event.clone();
                        one["from"] = json!(from + at as u64);
                        one["items"] = json!([item]);
                        split.push(one);
                    }
                }
                Some(_) | None => split.push(event),
            }
        }
        assert_eq!(grouped, 1, "the replay groups the three items");
        state["log"] = Value::Array(split);
        assert_eq!(decode(&rebuild(&state, &partial), agent.now).expect("it reads"), agent.s);
    }

    // --- checkpoints that don't read ------------------------------------

    /// A phase this core doesn't know (a later build's) is adopted as a
    /// conversation that could not be restored, its agent left running.
    #[test]
    fn an_unknown_phase_is_adopted_as_not_restored() {
        let agent = Agent::ready();
        let (mut state, partial) = state_of(&encode(&agent.s).expect("it fits").bytes);
        state["phase"] = json!({"phase": "logging_out", "method": "m"});
        let adopted = adopt(&rebuild(&state, &partial), false, "/home/u/src", "0.3.0", agent.now);
        assert_eq!(adopted.restored, Err(Unreadable::Field { name: "phase" }));
        assert_not_restored(&adopted);
    }

    #[test]
    fn a_newer_format_is_adopted_as_a_conversation_that_could_not_be_restored() {
        let mut bytes = encode(&Agent::busy().s).expect("it fits").bytes;
        bytes[4..6].copy_from_slice(&3u16.to_be_bytes());
        let adopted = adopt(&bytes, false, "/home/u/src", "0.3.0", Instant::now());
        assert_eq!(adopted.restored, Err(Unreadable::Format { format: 3 }));
        assert_not_restored(&adopted);
        let entries = entries(&adopted.session);
        assert!(matches!(&entries[0], Entry::Notice { text, .. }
            if text.contains("format 3") && text.contains("reads format 2")));
    }

    #[test]
    fn bytes_that_arent_a_checkpoint_are_adopted_as_not_restored() {
        for (bytes, why) in [
            (&b"BAC"[..], Unreadable::Short { len: 3 }),
            (&b"\x1bc\x1b[2Jhello there"[..], Unreadable::NotACheckpoint),
            (
                &b"BACP\x00\x02B\x00\x00\x00\x00\x00\x00\x00\x09{}"[..],
                Unreadable::Truncated,
            ),
            (
                &b"BACP\x00\x00\x00\x00\x00\x00"[..],
                Unreadable::Format { format: 0 },
            ),
        ] {
            let adopted = adopt(bytes, false, "/home/u/src", "0.3.0", Instant::now());
            assert_eq!(adopted.restored, Err(why));
            assert_not_restored(&adopted);
        }
    }

    #[test]
    fn a_session_that_wasnt_restored_refuses_prompts_and_ignores_its_agent() {
        let adopted = adopt(
            b"garbage!!!!!",
            false,
            "/home/u/src",
            "0.3.0",
            Instant::now(),
        );
        let mut session = adopted.session;
        let now = Instant::now();
        let out = step(
            &mut session,
            Input::Command(Command::prompt("hi")),
            now,
        );
        assert_eq!(
            out.reply,
            Some(Err(Refused::Failed {
                message: NOT_RESTORED.into()
            }))
        );
        let out = step(&mut session, Input::Stdout(&chunk("ignored")), now);
        assert!(out.events.is_empty() && out.stdin.is_empty());
        assert!(!out.hang_up, "the agent is left running");
    }

    #[test]
    fn a_state_that_names_an_entry_the_log_doesnt_hold_is_unreadable() {
        let agent = Agent::busy();
        let (state, partial) = state_of(&encode(&agent.s).expect("it fits").bytes);
        let mut bad_run = state.clone();
        bad_run["run"] = json!({"kind": "agent", "entry": 99});
        let mut bad_permission = state.clone();
        bad_permission["rpc"]["permissions"] = json!([[1, 0]]);
        // The tool calls aren't stored: they are the log's (decision 2).
        for (state, name, entry) in [(bad_run, "run", 99), (bad_permission, "permissions", 1)] {
            assert_eq!(
                decode(&rebuild(&state, &partial), agent.now),
                Err(Unreadable::Entry {
                    name,
                    entry: EntryId(entry)
                })
            );
        }
    }

    /// The converse of the check above (`agents-process-lifecycle`
    /// decision 3): a pending permission request in the log that `Rpc`
    /// doesn't await could never be answered, so the checkpoint is refused.
    #[test]
    fn a_checkpoint_with_a_pending_permission_rpc_doesnt_await_is_unreadable() {
        let agent = Agent::busy();
        let entry = *agent.s.rpc.permissions().keys().next().expect("a request");
        let (mut state, partial) = state_of(&encode(&agent.s).expect("it fits").bytes);
        state["rpc"]["permissions"] = json!([]);
        let refused = decode(&rebuild(&state, &partial), agent.now);
        assert_eq!(
            refused,
            Err(Unreadable::Entry {
                name: "permissions",
                entry
            })
        );
        assert_eq!(
            refused.unwrap_err().to_string(),
            format!("its checkpoint's permissions and its log disagree about entry {entry}")
        );
    }

    #[test]
    fn another_acp_version_is_unreadable() {
        let agent = Agent::ready();
        let (mut state, partial) = state_of(&encode(&agent.s).expect("it fits").bytes);
        state["acp"] = json!(2);
        assert_eq!(
            decode(&rebuild(&state, &partial), agent.now),
            Err(Unreadable::Acp { version: 2 })
        );
    }

    #[test]
    fn a_log_that_doesnt_fold_is_unreadable() {
        let agent = Agent::busy();
        let (mut state, partial) = state_of(&encode(&agent.s).expect("it fits").bytes);
        let log = state["log"].as_array_mut().expect("an array");
        log.insert(0, log[0].clone());
        assert!(matches!(
            decode(&rebuild(&state, &partial), agent.now),
            Err(Unreadable::Log { .. })
        ));
    }

    #[test]
    fn requests_that_dont_hold_together_are_unreadable() {
        let agent = Agent::busy();
        let (mut state, partial) = state_of(&encode(&agent.s).expect("it fits").bytes);
        state["rpc"]["next_id"] = json!(3);
        assert_eq!(
            decode(&rebuild(&state, &partial), agent.now),
            Err(Unreadable::Rpc {
                parts: BadParts::IdNotBelowNext
            })
        );
    }

    #[test]
    fn line_bytes_beside_a_partial_that_isnt_collected_are_unreadable() {
        let mut agent = Agent::ready();
        agent.s.process.lines = Lines::restore(Partial::Resyncing { len: 3 }, 10);
        let (state, _) = state_of(&encode(&agent.s).expect("it fits").bytes);
        assert_eq!(
            decode(&rebuild(&state, b"abc"), agent.now),
            Err(Unreadable::StrayBytes { len: 3 })
        );
    }

    #[test]
    fn a_missing_field_is_unreadable_naming_it() {
        let agent = Agent::ready();
        let (state, partial) = state_of(&encode(&agent.s).expect("it fits").bytes);
        for name in [
            "acp",
            "cwd",
            "version",
            "phase",
            "log",
            "rpc",
            "run",
            "tool_epoch",
            "bad_lines",
            "lines",
        ] {
            let mut state = state.clone();
            state.as_object_mut().expect("an object").remove(name);
            assert_eq!(
                decode(&rebuild(&state, &partial), agent.now),
                Err(Unreadable::Field { name }),
                "{name}"
            );
        }
    }

    // --- format 2: segments (agents-log-paging decision 7) ----------------

    #[test]
    fn a_base_and_three_segments_decode_to_the_live_session_the_last_record_winning() {
        let mut agent = Agent::ready();
        let mut stored = Stored::whole(&agent.s);
        stored.saw(agent.prompt("first"));
        stored.saw(agent.feed(&chunk("Working on ")));
        stored.saw(agent.feed(&chunk("it")));
        stored.append(&agent.s);
        stored.saw(agent.feed(&update(
            json!({"sessionUpdate": "tool_call", "toolCallId": "c1",
            "title": "Write", "kind": "edit", "status": "pending"}),
        )));
        stored.saw(agent.feed(&permission(json!("p1"))));
        stored.saw(agent.prompt("second"));
        stored.saw(agent.feed(b"{\"jsonrpc\":\"2.0\",\"method\":\"session/up"));
        stored.append(&agent.s);
        assert_eq!(stored.decode(agent.now), agent.s);
        assert_eq!(stored.decode(agent.now).rpc.permissions().len(), 1);
        // The partial line completes, the turn ends and the permission goes:
        // the last record's partial, phase and requests win.
        stored.saw(agent.feed(b"date\",\"params\":{\"sessionId\":\"s1\",\"update\":{\"sessionUpdate\":\"agent_message_chunk\",\"content\":{\"type\":\"text\",\"text\":\" done\"}}}}\n"));
        stored.saw(agent.feed(&result(3, json!({"stopReason": "end_turn"}))));
        agent.s.methods = creating().s.methods;
        stored.append(&agent.s);
        assert_eq!(stored.seq, 3);
        let decoded = stored.decode(agent.now);
        assert_eq!(decoded, agent.s);
        assert!(matches!(decoded.process.lines.partial(), Partial::Collecting(bytes) if bytes.is_empty()));
        assert_eq!(decoded.methods.len(), 2);
    }

    #[test]
    fn a_segment_written_while_logging_in_round_trips() {
        let mut agent = creating();
        let mut stored = Stored::whole(&agent.s);
        stored.saw(agent.feed(&auth_error(2, "Authentication required")));
        stored.append(&agent.s);
        assert!(matches!(stored.decode(agent.now).phase, Phase::LoginRequired(_)));
        assert_eq!(stored.decode(agent.now), agent.s);
        stored.saw(login_cmd(&mut agent, "oauth-personal"));
        stored.append(&agent.s);
        assert!(matches!(stored.decode(agent.now).phase, Phase::LoggingIn { .. }));
        assert_eq!(stored.decode(agent.now), agent.s);
    }

    #[test]
    fn a_segment_written_during_a_muted_load_round_trips() {
        let mut agent = Agent::ready();
        let mut stored = Stored::whole(&agent.s);
        stored.saw(agent.prompt("first"));
        stored.saw(agent.feed(&chunk("Done.")));
        stored.saw(agent.feed(&result(3, json!({"stopReason": "end_turn"}))));
        stored.saw(agent.step(Input::Exited {
            code: Some(1),
            stderr: b"",
        }));
        stored.append(&agent.s);
        stored.saw(agent.step(Input::Restart));
        stored.saw(agent.step(Input::Spawned));
        stored.saw(agent.feed(&result(
            4,
            json!({"protocolVersion": 1, "agentCapabilities": {"loadSession": true}}),
        )));
        stored.saw(agent.feed(&chunk("replayed")));
        stored.saw(agent.feed(&permission(json!(9))));
        stored.append(&agent.s);
        let decoded = stored.decode(agent.now);
        assert!(matches!(
            decoded.phase,
            Phase::Loading {
                replay: Replay::Muted { updates: 1 },
                ..
            }
        ));
        assert_eq!(decoded, agent.s);
    }

    #[test]
    fn a_segment_written_after_a_retitle_round_trips_with_the_title() {
        let mut agent = Agent::ready();
        let mut stored = Stored::whole(&agent.s);
        stored.saw(agent.feed(&update(json!({"sessionUpdate": "session_info_update",
            "title": "Hello.txt file creation"}))));
        stored.append(&agent.s);
        let decoded = stored.decode(agent.now);
        assert_eq!(decoded, agent.s);
        assert_eq!(
            decoded.transcript.conversation().and_then(|c| c.title.as_deref()),
            Some("Hello.txt file creation")
        );
    }

    #[test]
    fn appended_text_is_merged_per_entry_within_a_chunk_and_folds_the_same() {
        let a = EntryId(1);
        let b = EntryId(2);
        let appended = |id, text: &str| Event::Appended {
            id,
            text: text.into(),
        };
        let events = [
            appended(a, "one "),
            appended(a, "two"),
            appended(b, "x"),
            appended(a, "three"),
        ];
        assert_eq!(
            merged(events.to_vec()),
            [appended(a, "one two"), appended(b, "x"), appended(a, "three")]
        );
        let half = "y".repeat(TEXT_CHUNK / 2 + 1);
        assert_eq!(
            merged(vec![appended(a, &half), appended(a, &half)]).len(),
            2,
            "a merge never passes TEXT_CHUNK"
        );
        // A run of chunks makes a segment of a few events that folds to the
        // same text.
        let mut agent = Agent::ready();
        let mut stored = Stored::whole(&agent.s);
        stored.saw(agent.prompt("go"));
        for n in 0..200 {
            stored.saw(agent.feed(&chunk(&format!("piece {n} "))));
        }
        let unsaved = stored.unsaved.len();
        stored.append(&agent.s);
        let mut rest = &stored.bytes[PREFIX_LEN..];
        next_record(&mut rest).expect("a base");
        let segment = next_record(&mut rest).expect("a record").expect("a segment");
        let state: Value = serde_json::from_slice(segment.state).expect("JSON");
        let written = state["events"].as_array().expect("events").len();
        assert!(written < unsaved / 10, "{written} of {unsaved}");
        assert_eq!(stored.decode(agent.now), agent.s);
    }

    #[test]
    fn a_format_2_checkpoint_out_of_order_or_cut_short_is_adopted_as_not_restored() {
        let agent = Agent::ready();
        let base = encode(&agent.s).expect("it fits").bytes;
        let (state, _) = state_of(&base);
        let mut segment_state = state.clone();
        if let Some(object) = segment_state.as_object_mut() {
            object.remove("log");
            object.insert("events".into(), json!([]));
        }
        let header = base[..PREFIX_LEN].to_vec();
        let joined = |parts: &[&[u8]]| parts.concat();
        let segment = |seq| record(SEGMENT, seq, &segment_state, &[]);
        let cases: Vec<(Vec<u8>, Unreadable)> = vec![
            (header.clone(), Unreadable::NoBase),
            (joined(&[&header, &segment(0)]), Unreadable::NoBase),
            (
                joined(&[&header, &record(b'X', 0, &state, &[])]),
                Unreadable::RecordKind { kind: b'X' },
            ),
            (
                joined(&[&base, &record(BASE, 1, &state, &[])]),
                Unreadable::RecordKind { kind: BASE },
            ),
            (
                joined(&[&header, &record(BASE, 1, &state, &[])]),
                Unreadable::Sequence {
                    expected: 0,
                    found: 1,
                },
            ),
            (
                joined(&[&base, &segment(2)]),
                Unreadable::Sequence {
                    expected: 1,
                    found: 2,
                },
            ),
            (
                joined(&[&base, &segment(1)[..7]]),
                Unreadable::Truncated,
            ),
            (base[..base.len() - 1].to_vec(), Unreadable::Truncated),
        ];
        for (bytes, why) in cases {
            let adopted = adopt(&bytes, false, "/home/u/src", "0.3.0", agent.now);
            assert_eq!(adopted.restored, Err(why));
            assert_not_restored(&adopted);
        }
        // The segment itself reads.
        let fine = joined(&[&base, &segment(1)]);
        assert_eq!(decode(&fine, agent.now), Ok(agent.s.clone()));
    }

    #[test]
    fn the_new_refusals_read_as_what_went_wrong() {
        assert_eq!(
            Unreadable::NoBase.to_string(),
            "its checkpoint doesn't start with its base"
        );
        assert_eq!(
            Unreadable::RecordKind { kind: b'X' }.to_string(),
            "its checkpoint has a record of unknown kind 0x58"
        );
        assert_eq!(
            Unreadable::Sequence {
                expected: 2,
                found: 3
            }
            .to_string(),
            "its checkpoint's record 3 came where record 2 was due, so a part of it is missing"
        );
        assert_eq!(
            Unreadable::PartialGap { from: 5, held: 3 }.to_string(),
            "its checkpoint's partial line continues from byte 5, and the record before it held 3"
        );
    }

    #[test]
    fn a_checkpoint_with_a_title_and_a_login_adopts() {
        let mut agent = Agent::ready();
        agent.s.methods = creating().s.methods;
        agent.feed(&update(json!({"sessionUpdate": "session_info_update",
            "title": "Hello.txt file creation"})));
        assert_eq!(agent.prompt("hi").reply, Some(Ok(())));
        agent.feed(&auth_error(3, "expired"));
        let bytes = encode(&agent.s).expect("it fits").bytes;
        let adopted = adopt(&bytes, false, "/home/u/src", "0.3.0", agent.now);
        assert_eq!(adopted.restored, Ok(()));
        assert_eq!(adopted.session, agent.s);
        assert!(matches!(adopted.session.phase, Phase::LoginRequired(_)));
    }

    /// `agents-tool-content` decision 2: the `acp` field restores the
    /// session's dialect, and the pinned bytes below decode to it.
    #[test]
    fn a_checkpoint_restores_its_dialect() {
        let agent = Agent::ready();
        let bytes = encode(&agent.s).expect("it fits").bytes;
        assert_eq!(state_of(&bytes).0["acp"], 1);
        let session = decode(&bytes, agent.now).expect("it decodes");
        assert_eq!(session.rpc.dialect(), Dialect::newest());
        assert_eq!(session.rpc.dialect().version(), 1);
        let session = decode(&unhex(PINNED), Instant::now()).expect("it decodes");
        assert_eq!(session.rpc.dialect().version(), 1);
    }

    fn unhex(hex: &str) -> Vec<u8> {
        (0..hex.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&hex[at..at + 2], 16).expect("hex"))
            .collect()
    }

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn a_small_sessions_format_2_bytes_are_pinned() {
        let session = AgentSession::new("/w".into(), "0.3.0".into());
        let bytes = encode(&session).expect("it fits").bytes;
        assert_eq!(hex(&bytes), PINNED);
        assert_eq!(decode(&bytes, Instant::now()), Ok(session));
    }

    /// `BACP`, format 2, a base record numbered 0 with the state's length,
    /// the delta-shaped state (`agents-checkpoint-deltas`), and no partial
    /// line.
    const PINNED: &str = concat!(
        "42414350000242000000000000012a7b22616370223a312c22637764223a222f7722",
        "2c2276657273696f6e223a22302e332e30222c227068617365223a7b227068617365",
        "223a22737061776e696e67227d2c22727063223a7b226e6578745f6964223a312c22",
        "6f7574676f696e67223a5b5d2c227065726d697373696f6e73223a5b5d7d2c227275",
        "6e223a6e756c6c2c22746f6f6c5f65706f6368223a312c226261645f6c696e657322",
        "3a302c226c696e6573223a7b227061727469616c223a22636f6c6c656374696e6722",
        "2c22636f76657273223a307d2c226c6f67223a5b7b226576656e74223a227475726e",
        "222c227374617465223a7b227475726e223a2269646c65227d7d2c7b226576656e74",
        "223a226167656e74222c227374617465223a7b226167656e74223a22737461727469",
        "6e67227d7d5d7d00000000",
    );

    /// A base and two segments (`agents-checkpoint-deltas` decisions 3 and
    /// 4): the first segment holds a partial line whole, the second its
    /// next bytes from where the first left off, and the login methods,
    /// which changed.
    #[test]
    fn a_base_and_two_segments_are_pinned() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        let mut stored = Stored::whole(&agent.s);
        stored.saw(agent.feed(b"{\"jsonrpc\":\"2.0\","));
        stored.append(&agent.s);
        stored.saw(agent.feed(b"\"id\":1,"));
        agent.s.methods = creating().s.methods;
        stored.append(&agent.s);
        let records = records(&stored.bytes);
        assert_eq!(records[2].1["lines"]["from"], 17);
        assert_eq!(records[2].2, b"\"id\":1,");
        assert_eq!(records[2].1["methods"].as_array().map(Vec::len), Some(2));
        assert_eq!(hex(&stored.bytes), PINNED_CHAIN);
        assert_eq!(decode(&unhex(PINNED_CHAIN), agent.now), Ok(agent.s));
    }

    /// The base (no partial line), a segment holding the partial line's 17
    /// bytes whole, and one holding its next 7 from byte 17 and the
    /// methods.
    const PINNED_CHAIN: &str = concat!(
        "4241435000024200000000000001477b22616370223a312c22637764223a222f686f",
        "6d652f752f737263222c2276657273696f6e223a22302e332e30222c227068617365",
        "223a7b227068617365223a22696e697469616c697a696e67227d2c22727063223a7b",
        "226e6578745f6964223a322c226f7574676f696e67223a5b5b312c22696e69746961",
        "6c697a65225d5d2c227065726d697373696f6e73223a5b5d7d2c2272756e223a6e75",
        "6c6c2c22746f6f6c5f65706f6368223a312c226261645f6c696e6573223a302c226c",
        "696e6573223a7b227061727469616c223a22636f6c6c656374696e67222c22636f76",
        "657273223a307d2c226c6f67223a5b7b226576656e74223a227475726e222c227374",
        "617465223a7b227475726e223a2269646c65227d7d2c7b226576656e74223a226167",
        "656e74222c227374617465223a7b226167656e74223a227374617274696e67227d7d",
        "5d7d000000005300000001000000f47b22616370223a312c22637764223a222f686f",
        "6d652f752f737263222c2276657273696f6e223a22302e332e30222c227068617365",
        "223a7b227068617365223a22696e697469616c697a696e67227d2c22727063223a7b",
        "226e6578745f6964223a322c226f7574676f696e67223a5b5b312c22696e69746961",
        "6c697a65225d5d2c227065726d697373696f6e73223a5b5d7d2c2272756e223a6e75",
        "6c6c2c22746f6f6c5f65706f6368223a312c226261645f6c696e6573223a302c226c",
        "696e6573223a7b227061727469616c223a22636f6c6c656374696e67222c22636f76",
        "657273223a31377d2c226576656e7473223a5b5d7d000000117b226a736f6e727063",
        "223a22322e30222c53000000020000019c7b22616370223a312c22637764223a222f",
        "686f6d652f752f737263222c2276657273696f6e223a22302e332e30222c22706861",
        "7365223a7b227068617365223a22696e697469616c697a696e67227d2c2272706322",
        "3a7b226e6578745f6964223a322c226f7574676f696e67223a5b5b312c22696e6974",
        "69616c697a65225d5d2c227065726d697373696f6e73223a5b5d7d2c2272756e223a",
        "6e756c6c2c22746f6f6c5f65706f6368223a312c226261645f6c696e6573223a302c",
        "226c696e6573223a7b227061727469616c223a22636f6c6c656374696e67222c2263",
        "6f76657273223a32342c2266726f6d223a31377d2c226d6574686f6473223a5b7b22",
        "6964223a226f617574682d706572736f6e616c222c226e616d65223a224c6f672069",
        "6e207769746820476f6f676c65222c226b696e64223a226167656e74227d2c7b2269",
        "64223a22636c69222c226e616d65223a22434c49222c226b696e64223a227465726d",
        "696e616c222c2261726773223a5b222d2d6c6f67696e225d2c22656e76223a7b2241",
        "223a2231227d7d5d2c226576656e7473223a5b5d7d00000007226964223a312c",
    );

    /// The base record a build before `agents-checkpoint-deltas` wrote for
    /// a new session: a `tools` map and no `tool_epoch`.
    const PINNED_BEFORE_DELTAS: &str = concat!(
        "4241435000024200000000000001267b22616370223a312c22637764223a222f77222c2276657273",
        "696f6e223a22302e332e30222c227068617365223a7b227068617365223a22737061776e696e6722",
        "7d2c22727063223a7b226e6578745f6964223a312c226f7574676f696e67223a5b5d2c227065726d",
        "697373696f6e73223a5b5d7d2c2272756e223a6e756c6c2c22746f6f6c73223a7b7d2c226261645f",
        "6c696e6573223a302c226c696e6573223a7b227061727469616c223a22636f6c6c656374696e6722",
        "2c22636f76657273223a307d2c226c6f67223a5b7b226576656e74223a227475726e222c22737461",
        "7465223a7b227475726e223a2269646c65227d7d2c7b226576656e74223a226167656e74222c2273",
        "74617465223a7b226167656e74223a227374617274696e67227d7d5d7d00000000",
    );

    // --- the delta shape (agents-checkpoint-deltas) ---------------------

    /// Decision 1: a checkpoint of the earlier shape, or of format 1, isn't
    /// read; its session is adopted as not restored.
    #[test]
    fn a_checkpoint_from_before_the_delta_shape_is_unreadable() {
        let before = unhex(PINNED_BEFORE_DELTAS);
        assert_eq!(
            decode(&before, Instant::now()),
            Err(Unreadable::Field { name: "tool_epoch" })
        );
        let adopted = adopt(&before, false, "/w", "0.3.0", Instant::now());
        assert_not_restored(&adopted);
        let mut format_1 = before[..PREFIX_LEN].to_vec();
        format_1[5] = 1;
        format_1.extend_from_slice(b"\x00\x00\x00\x02{}");
        assert_eq!(
            decode(&format_1, Instant::now()),
            Err(Unreadable::Format { format: 1 })
        );
        assert_eq!(
            Unreadable::Format { format: 1 }.to_string(),
            "its checkpoint is in format 1, and this terminal server reads format 2"
        );
    }

    #[test]
    fn a_record_without_an_epoch_is_unreadable() {
        let mut agent = Agent::ready();
        let mut stored = Stored::whole(&agent.s);
        stored.saw(agent.prompt("go"));
        stored.append(&agent.s);
        let good = records(&stored.bytes);
        for at in 0..good.len() {
            let mut bad = good.clone();
            bad[at].1.as_object_mut().expect("an object").remove("tool_epoch");
            assert_eq!(
                decode(&chain(&bad), agent.now),
                Err(Unreadable::Field { name: "tool_epoch" }),
                "record {at}"
            );
        }
        assert_eq!(decode(&chain(&good), agent.now), Ok(agent.s));
    }

    /// Decision 2: no record writes the tool calls, so a segment's size
    /// doesn't grow with them.
    #[test]
    fn a_segment_carries_no_tool_index_and_its_size_doesnt_grow_with_the_tool_calls() {
        let mut agent = Agent::ready();
        let mut stored = Stored::whole(&agent.s);
        stored.saw(agent.prompt("go"));
        stored.append(&agent.s);
        let mut sizes = Vec::new();
        for call in 1..=2000 {
            stored.saw(agent.feed(&update(json!({"sessionUpdate": "tool_call",
                "toolCallId": format!("toolu_{call:024}"), "title": "Read", "kind": "read",
                "status": "pending"}))));
            let before = stored.bytes.len();
            stored.append(&agent.s);
            if call == 10 || call == 2000 {
                let (mut state, _) = record_at(&stored.bytes[before..]);
                assert!(state.get("tools").is_none(), "{state}");
                let events = state["events"].to_string().len();
                let object = state.as_object_mut().expect("an object");
                object.remove("events");
                object["lines"]["covers"] = json!(0);
                sizes.push((stored.bytes.len() - before - events, state));
            }
        }
        assert_eq!(agent.s.process.book.tools.len(), 2000);
        let [(early, early_state), (late, late_state)] = &sizes[..] else {
            panic!("two segments");
        };
        assert_eq!(early_state, late_state, "the same but for the events and covers");
        assert!(late <= &(early + 8), "{early} bytes then {late}, past the events");
        assert_eq!(stored.decode(agent.now), agent.s);
    }

    #[test]
    fn a_restored_session_rebuilds_its_tool_index_from_the_log_and_epoch() {
        let mut agent = Agent::ready();
        let mut stored = Stored::whole(&agent.s);
        agent.prompt("go");
        agent.feed(&update(json!({"sessionUpdate": "tool_call", "toolCallId": "c1",
            "title": "Read", "kind": "read", "status": "pending"})));
        agent.step(Input::Exited {
            code: Some(1),
            stderr: b"",
        });
        restarted(&mut agent, &mut stored);
        assert!(matches!(agent.s.phase, Phase::Ready { .. }), "{:?}", agent.s.phase);
        let epoch = agent.s.process.book.tool_epoch;
        assert!(epoch > EntryId::FIRST);
        agent.feed(&update(json!({"sessionUpdate": "tool_call", "toolCallId": "c1",
            "title": "Read again", "kind": "read", "status": "pending"})));
        let second = agent.s.process.book.tools["c1"];
        let bytes = encode(&agent.s).expect("it fits").bytes;
        let (state, _) = state_of(&bytes);
        assert_eq!(state["tool_epoch"], epoch.0);
        assert!(state.get("tools").is_none());
        let decoded = decode(&bytes, agent.now).expect("it reads");
        assert_eq!(decoded.process.book.tool_epoch, epoch);
        assert_eq!(
            decoded.process.book.tools,
            std::collections::BTreeMap::from([("c1".to_owned(), second)])
        );
        assert_eq!(decoded, agent.s);
    }

    /// Decision 3: a long line arriving in pieces is written once, each
    /// segment holding the bytes since the last.
    #[test]
    fn a_long_line_is_written_once_across_its_segments_and_decodes_whole() {
        let mut agent = Agent::ready();
        let mut stored = Stored::whole(&agent.s);
        stored.saw(agent.prompt("edit"));
        stored.append(&agent.s);
        let line = update(json!({"sessionUpdate": "tool_call", "toolCallId": "c1",
            "title": "Edit", "kind": "edit", "status": "pending",
            "content": [{"type": "diff", "path": "/w/a", "newText": "x".repeat(1 << 20)}]}));
        let body = &line[..line.len() - 1];
        let (mut written, mut fed) = (0, 0);
        for piece in body.chunks(64 * 1024) {
            stored.saw(agent.feed(piece));
            fed += piece.len();
            let before = stored.bytes.len();
            stored.append(&agent.s);
            let (state, partial) = record_at(&stored.bytes[before..]);
            let from = state["lines"]["from"].as_u64().unwrap_or(0) as usize;
            assert_eq!((from, from + partial.len()), (written, fed));
            written += partial.len();
            assert_eq!(stored.decode(agent.now), agent.s);
        }
        assert_eq!(written, body.len(), "each byte once");
        stored.saw(agent.feed(b"\n"));
        stored.append(&agent.s);
        let decoded = stored.decode(agent.now);
        assert_eq!(decoded, agent.s);
        assert!(decoded.process.book.tools.contains_key("c1"));
    }

    /// Feeds `bytes`, appends a segment and returns its `lines` and line
    /// bytes, checking the chain decodes to the session.
    fn append_lines(agent: &mut Agent, stored: &mut Stored, bytes: &[u8]) -> (Value, Vec<u8>) {
        if !bytes.is_empty() {
            stored.saw(agent.feed(bytes));
        }
        let before = stored.bytes.len();
        stored.append(&agent.s);
        assert_eq!(stored.decode(agent.now), agent.s);
        let (state, partial) = record_at(&stored.bytes[before..]);
        (state["lines"].clone(), partial)
    }

    #[test]
    fn a_partial_line_that_ended_or_was_cut_resets_its_delta() {
        let mut agent = Agent::ready();
        let mut stored = Stored::whole(&agent.s);
        let (lines, bytes) = append_lines(&mut agent, &mut stored, b"{\"jsonrpc\"");
        assert_eq!((lines.get("from"), &bytes[..]), (None, &b"{\"jsonrpc\""[..]));
        let (lines, bytes) = append_lines(&mut agent, &mut stored, b":\"2.0\"");
        assert_eq!((&lines["from"], &bytes[..]), (&json!(10), &b":\"2.0\""[..]));
        // The line ends and another starts: written whole.
        let (lines, bytes) =
            append_lines(&mut agent, &mut stored, b",\"method\":\"x\"}\n{\"a");
        assert_eq!((lines.get("from"), &bytes[..]), (None, &b"{\"a"[..]));
        // Cut, then resyncing: no bytes, and the next line is whole.
        let covers = agent.s.process.lines.covers();
        agent.s.process.lines = Lines::restore(Partial::TooLong { len: 9 << 20 }, covers + (9 << 20));
        let (lines, bytes) = append_lines(&mut agent, &mut stored, b"");
        assert_eq!((&lines["partial"], lines.get("from"), bytes.len()), (&json!("too_long"), None, 0));
        let covers = agent.s.process.lines.covers();
        agent.s.process.lines = Lines::restore(Partial::Resyncing { len: 3 }, covers);
        let (lines, bytes) = append_lines(&mut agent, &mut stored, b"");
        assert_eq!((&lines["partial"], bytes.len()), (&json!("resyncing"), 0));
        let (lines, bytes) = append_lines(&mut agent, &mut stored, b"zz\n{\"q");
        assert_eq!((&lines["partial"], lines.get("from"), &bytes[..]), (&json!("collecting"), None, &b"{\"q"[..]));
        let (lines, bytes) = append_lines(&mut agent, &mut stored, b"\"");
        assert_eq!((&lines["from"], &bytes[..]), (&json!(3), &b"\""[..]));
    }

    /// Decision 4.
    #[test]
    fn methods_are_written_only_when_they_change_and_an_emptied_list_reads_empty() {
        let mut agent = Agent::ready();
        let mut stored = Stored::whole(&agent.s);
        let methods_of = |stored: &Stored, before: usize| record_at(&stored.bytes[before..]).0.get("methods").cloned();
        agent.s.methods = creating().s.methods;
        let before = stored.bytes.len();
        stored.append(&agent.s);
        assert_eq!(methods_of(&stored, before).and_then(|m| m.as_array().map(Vec::len)), Some(2));
        let before = stored.bytes.len();
        stored.saw(agent.prompt("go"));
        stored.append(&agent.s);
        assert_eq!(methods_of(&stored, before), None, "unchanged");
        assert_eq!(stored.decode(agent.now).methods.len(), 2);
        agent.s.methods.clear();
        let before = stored.bytes.len();
        stored.append(&agent.s);
        assert_eq!(methods_of(&stored, before), Some(json!([])));
        let decoded = stored.decode(agent.now);
        assert!(decoded.methods.is_empty());
        assert_eq!(decoded, agent.s);
    }

    fn terminal(call: &str, meta: Value) -> Vec<u8> {
        update(json!({"sessionUpdate": "tool_call_update", "toolCallId": call, "_meta": meta}))
    }

    /// A tool call `call` showing the terminal `terminal`.
    fn command(call: &str, terminal: &str) -> Vec<u8> {
        update(json!({"sessionUpdate": "tool_call", "toolCallId": call, "title": "Run",
            "kind": "execute", "content": [{"type": "terminal", "terminalId": terminal}]}))
    }

    #[test]
    fn outputs_are_patched_per_terminal_and_a_closed_one_is_removed() {
        let mut agent = Agent::ready();
        let mut stored = Stored::whole(&agent.s);
        stored.saw(agent.prompt("run"));
        stored.saw(agent.feed(&command("c1", "t1")));
        stored.saw(agent.feed(&command("c2", "t2")));
        let mut patch = |agent: &mut Agent, bytes: &[u8]| {
            if !bytes.is_empty() {
                stored.saw(agent.feed(bytes));
            }
            let before = stored.bytes.len();
            stored.append(&agent.s);
            assert_eq!(stored.decode(agent.now), agent.s);
            record_at(&stored.bytes[before..]).0.get("outputs").cloned()
        };
        let keys = |patch: Option<Value>| {
            patch.and_then(|patch| Some(patch.as_object()?.keys().cloned().collect::<Vec<_>>()))
        };
        // The two commands opened an output each.
        assert_eq!(keys(patch(&mut agent, b"")), Some(vec!["t1".to_owned(), "t2".to_owned()]));
        let delta = |terminal: &str, data: &str| {
            json!({"terminal_output_delta": {"terminal_id": terminal, "data": data}})
        };
        // A line held in a filter changes its output alone.
        let changed = patch(&mut agent, &terminal("c1", delta("t1", "x")));
        assert_eq!(keys(changed), Some(vec!["t1".to_owned()]));
        let changed = patch(&mut agent, &terminal("c2", delta("t2", "y")));
        assert_eq!(keys(changed), Some(vec!["t2".to_owned()]));
        assert_eq!(patch(&mut agent, b""), None, "nothing changed");
        let closed = patch(&mut agent, &terminal("c1", json!({"terminal_exit":
            {"terminal_id": "t1", "exit_code": 0}})));
        assert_eq!(closed, Some(json!({"t1": null})));
        assert_eq!(agent.s.process.book.outputs.len(), 1);
        assert_eq!(patch(&mut agent, b""), None);
    }

    #[test]
    fn an_outputs_patch_closing_a_terminal_it_doesnt_hold_is_unreadable() {
        let mut agent = Agent::ready();
        let mut stored = Stored::whole(&agent.s);
        stored.saw(agent.prompt("run"));
        stored.saw(agent.feed(&command("c1", "t1")));
        stored.saw(agent.feed(&terminal("c1", json!({"terminal_output_delta":
            {"terminal_id": "t1", "data": "x"}}))));
        stored.append(&agent.s);
        stored.saw(agent.feed(&terminal("c1", json!({"terminal_exit":
            {"terminal_id": "t1", "exit_code": 0}}))));
        stored.append(&agent.s);
        let good = records(&stored.bytes);
        assert_eq!(good[2].1["outputs"], json!({"t1": null}));
        assert_eq!(decode(&chain(&good), agent.now), Ok(agent.s.clone()));
        for (at, patch) in [(1, json!({"t9": null})), (2, json!({"t9": null})), (2, json!({"t1": null, "t1x": null}))] {
            let mut bad = good.clone();
            bad[at].1["outputs"] = patch;
            assert_eq!(
                decode(&chain(&bad), agent.now),
                Err(Unreadable::Field { name: "outputs" }),
                "record {at}"
            );
        }
    }

    #[test]
    fn a_segment_whose_partial_doesnt_continue_the_last_is_unreadable() {
        let mut agent = Agent::ready();
        let mut stored = Stored::whole(&agent.s);
        stored.saw(agent.feed(b"{\"a"));
        stored.append(&agent.s);
        stored.saw(agent.feed(b"bc"));
        stored.append(&agent.s);
        let good = records(&stored.bytes);
        assert_eq!(good[2].1["lines"]["from"], 3);
        assert_eq!(decode(&chain(&good), agent.now), Ok(agent.s.clone()));
        // Past what the record before held.
        let mut past = good.clone();
        past[2].1["lines"]["from"] = json!(5);
        // Not where that line started.
        let mut moved = good.clone();
        moved[2].1["lines"]["covers"] = json!(good[2].1["lines"]["covers"].as_u64().unwrap() + 1);
        // After a record that held no line.
        let mut lone = good.clone();
        lone[1].2.clear();
        lone[1].1["lines"]["covers"] = json!(0);
        // A base continuing nothing.
        let mut base = good.clone();
        base[0].1["lines"]["from"] = json!(2);
        base[0].2 = b"xy".to_vec();
        base[0].1["lines"]["covers"] = json!(4);
        for (bad, from, held) in [(past, 5, 3), (moved, 3, 3), (lone, 3, 0), (base, 2, 0)] {
            assert_eq!(
                decode(&chain(&bad), agent.now),
                Err(Unreadable::PartialGap { from, held }),
                "from {from}, held {held}"
            );
        }
        let mut malformed = good;
        malformed[2].1["lines"]["from"] = json!("3");
        assert_eq!(decode(&chain(&malformed), agent.now), Err(Unreadable::Field { name: "lines" }));
    }

    /// The log's pending permission requests are the ones `Rpc` awaits
    /// (`agents-process-lifecycle` decision 3), and a process the session
    /// is done with has nothing out: no permission and no list.
    fn invariant(session: &AgentSession) {
        assert_eq!(
            reduce::pending_permissions(session.transcript()),
            session.rpc.waiting_permissions(),
            "the log's pending permission requests aren't the ones awaited"
        );
        let closed = match session.phase {
            Phase::Restarting { .. } | Phase::Refused | Phase::Broken | Phase::Exited | Phase::Lost => true,
            Phase::Spawning { .. }
            | Phase::Initializing { .. }
            | Phase::Creating { .. }
            | Phase::Resuming { .. }
            | Phase::Loading { .. }
            | Phase::Ready { .. }
            | Phase::Choosing { .. }
            | Phase::LoginRequired(_)
            | Phase::LoggingIn { .. } => false,
        };
        if closed {
            assert!(
                session.rpc.waiting_permissions().is_empty() && session.rpc.listing().is_none(),
                "{:?} left requests out: {:?}",
                session.phase,
                session.rpc
            );
        }
    }

    /// `Input::Restart` as the core steps it: after a login demand the old
    /// agent is hung up and the start is made again; once placed, the new
    /// agent's `SPAWNED`, and the answers that bring it to ready. Every
    /// output but the last is noted in `stored`; the last is returned.
    fn restarted(agent: &mut Agent, stored: &mut Stored) -> Output {
        let out = agent.step(Input::Restart);
        if out.reply != Some(Ok(())) {
            return out;
        }
        invariant(&agent.s);
        let mut outs = vec![out];
        if matches!(agent.s.phase, Phase::Restarting { .. }) {
            outs.push(agent.step(Input::Restart));
            invariant(&agent.s);
        }
        outs.push(agent.step(Input::Spawned));
        invariant(&agent.s);
        let request = |out: &Output| {
            sent(&out.stdin)
                .into_iter()
                .find_map(|m| Some((m["method"].as_str()?.to_owned(), m["id"].as_u64()?)))
        };
        if let Some((_, id)) = outs.last().and_then(request) {
            outs.push(agent.feed(&result(
                id,
                json!({"protocolVersion": 1,
                "agentCapabilities": {"loadSession": true, "sessionCapabilities": {"resume": {}}},
                "agentInfo": {"name": "fake", "version": "1.0"}}),
            )));
        }
        match outs.last().and_then(request) {
            Some((method, id)) if method == "session/new" => {
                outs.push(agent.feed(&result(id, json!({"sessionId": "s1"}))));
            }
            Some((_, id)) => outs.push(agent.feed(&result(id, json!({})))),
            None => {}
        }
        let last = outs.pop().unwrap_or_default();
        for out in outs {
            stored.saw(out);
            invariant(&agent.s);
        }
        last
    }

    /// Random reducer steps checkpointed at random points, whole or
    /// appended: what the holder stores always decodes to the live session,
    /// and the log's pending permission requests are always the ones
    /// awaited. The steps include the ends of an agent process: a login
    /// demand and its restart, an exit and the restart after it, a fault,
    /// answers and withdrawals of permissions, and a list out.
    #[test]
    fn random_steps_checkpointed_whole_or_appended_decode_to_the_live_session() {
        let mut rng = XorShift(0x5eed_5e9e_0000_0002);
        for _ in 0..300 {
            let mut agent = Agent::ready();
            // It offers lists too, so a list can be out when it stops.
            agent.s.lists = true;
            let mut stored = Stored::whole(&agent.s);
            let mut prompt_id = None;
            let mut list_id = None;
            let mut req = 0;
            // A long line arriving in pieces (`agents-checkpoint-deltas`
            // decision 3), and how much of it is fed.
            let mut long: Option<(Vec<u8>, usize)> = None;
            for _ in 0..40 {
                let out = match rng.below(22) {
                    0 => {
                        let out = agent.prompt("go on");
                        if let Some(id) = sent(&out.stdin).first().and_then(|m| m["id"].as_u64()) {
                            prompt_id = Some(id);
                        }
                        out
                    }
                    1 | 2 => agent.feed(&chunk(&"text ".repeat(1 + rng.below(30)))),
                    3 => agent.feed(&update(json!({"sessionUpdate": "agent_thought_chunk",
                        "content": {"type": "text", "text": "hmm"}}))),
                    4 => {
                        let call = format!("c{}", rng.below(4));
                        agent.feed(&update(json!({"sessionUpdate": "tool_call",
                            "toolCallId": call, "title": "Run", "kind": "execute",
                            "status": "pending"})))
                    }
                    5 => {
                        let call = format!("c{}", rng.below(4));
                        agent.feed(&update(json!({"sessionUpdate": "tool_call_update",
                            "toolCallId": call, "status": "completed"})))
                    }
                    6 => agent.feed(&permission(json!(format!("p{}", rng.below(100))))),
                    7 => match prompt_id.take() {
                        Some(id) => agent.feed(&result(id, json!({"stopReason": "end_turn"}))),
                        None => agent.step(Input::Command(Command::Cancel)),
                    },
                    8 => agent.feed(&update(json!({"sessionUpdate": "session_info_update",
                        "title": format!("title {}", rng.below(3))}))),
                    9 => {
                        let line = chunk("split");
                        let at = rng.below(line.len());
                        agent.feed(&line[..at])
                    }
                    10 => agent.feed(b"not json\n"),
                    11 => agent.step(Input::Command(Command::Unqueue)),
                    // A login demand for the prompt out.
                    12 => match prompt_id.take() {
                        Some(id) => agent.feed(&auth_error(id, "log in")),
                        None => agent.step(Input::Tick),
                    },
                    // A restart: after a login demand or an exit, the agent
                    // is started again and brought to ready if it can be.
                    13 => {
                        let out = restarted(&mut agent, &mut stored);
                        // A refused restart leaves what was out.
                        prompt_id = prompt_id.filter(|id| agent.s.rpc.outgoing().contains_key(id));
                        list_id = list_id.filter(|id| agent.s.rpc.outgoing().contains_key(id));
                        out
                    }
                    14 => {
                        prompt_id = None;
                        list_id = None;
                        agent.step(Input::Exited {
                            code: Some(1),
                            stderr: b"gone",
                        })
                    }
                    15 => agent.step(Input::Fault {
                        reason: "random fault".into(),
                    }),
                    16 => match agent.s.rpc.waiting_permissions().first() {
                        Some(&entry) => agent.step(Input::Command(Command::Answer {
                            entry,
                            option: Some("allow".into()),
                        })),
                        None => agent.step(Input::Tick),
                    },
                    17 => agent.feed(&line(json!({"jsonrpc": "2.0", "method": "$/cancel_request",
                        "params": {"requestId": format!("p{}", rng.below(100))}}))),
                    18 => {
                        req += 1;
                        let out = agent.step(Input::Command(Command::List { req, cursor: None }));
                        if let Some(id) = sent(&out.stdin).first().and_then(|m| m["id"].as_u64()) {
                            list_id = Some(id);
                        }
                        out
                    }
                    19 | 20 => {
                        let (line, fed) = long.get_or_insert_with(|| {
                            let call = format!("c{}", rng.below(4));
                            let text = "z".repeat(1024 * (1 + rng.below(64)));
                            let line = update(json!({"sessionUpdate": "tool_call_update",
                                "toolCallId": call, "content": [{"type": "content",
                                    "content": {"type": "text", "text": text}}]}));
                            (line, 0)
                        });
                        let to = (*fed + 1 + rng.below(16 * 1024)).min(line.len());
                        let out = agent.feed(&line[*fed..to]);
                        *fed = to;
                        if to == line.len() {
                            long = None;
                        }
                        out
                    }
                    _ => match list_id.take() {
                        Some(id) => agent.feed(&result(id, json!({"sessions": []}))),
                        None => agent.step(Input::Tick),
                    },
                };
                stored.saw(out);
                invariant(&agent.s);
                match rng.below(5) {
                    0 => stored = Stored::whole(&agent.s),
                    1 | 2 => stored.append(&agent.s),
                    _ => {}
                }
            }
            // A list out isn't checkpointed (`agents-session-list` flag
            // 4): it is answered before the comparison.
            if let Some(id) = list_id {
                stored.saw(agent.feed(&result(id, json!({"sessions": []}))));
                invariant(&agent.s);
            }
            stored.append(&agent.s);
            assert_eq!(stored.decode(agent.now), agent.s);
        }
    }

    // --- fuzz -----------------------------------------------------------

    #[test]
    fn fuzz_decoding_never_panics_on_random_or_mutated_checkpoints() {
        let mut rng = XorShift(0x5eed_ac01_c4ec_0001);
        let now = Instant::now();
        let valid = encode(&Agent::busy().s).expect("it fits").bytes;
        // A base and two segments, so mutations reach format 2's records.
        let segmented = {
            let mut agent = Agent::ready();
            let mut stored = Stored::whole(&agent.s);
            stored.saw(agent.prompt("first"));
            stored.saw(agent.feed(&chunk("Working")));
            stored.append(&agent.s);
            stored.saw(agent.feed(&permission(json!("p1"))));
            stored.append(&agent.s);
            stored.bytes
        };
        // A delta-shaped chain: a long partial line across three segments
        // and an output opened, then closed by a patch.
        let deltas = {
            let mut agent = Agent::ready();
            let mut stored = Stored::whole(&agent.s);
            stored.saw(agent.prompt("run"));
            stored.saw(agent.feed(&command("c1", "t1")));
            stored.saw(agent.feed(&terminal("c1", json!({"terminal_output_delta":
                {"terminal_id": "t1", "data": "x\n"}}))));
            let long = chunk(&"z".repeat(300));
            stored.saw(agent.feed(&long[..100]));
            stored.append(&agent.s);
            stored.saw(agent.feed(&long[100..200]));
            stored.append(&agent.s);
            stored.saw(agent.feed(&long[200..]));
            stored.saw(agent.feed(&terminal("c1", json!({"terminal_exit":
                {"terminal_id": "t1", "exit_code": 0}}))));
            stored.append(&agent.s);
            assert_eq!(stored.decode(agent.now), agent.s);
            stored.bytes
        };
        for round in 0..4500 {
            let valid = match round % 3 {
                0 => &segmented,
                1 => &deltas,
                _ => &valid,
            };
            let bytes: Vec<u8> = match round % 4 {
                // Random bytes, sometimes behind a good header.
                0 => (0..rng.below(64)).map(|_| rng.next() as u8).collect(),
                1 => {
                    let mut bytes = valid[..PREFIX_LEN + rng.below(RECORD_HEADER_LEN)].to_vec();
                    bytes.extend((0..rng.below(256)).map(|_| rng.next() as u8));
                    bytes
                }
                // A valid checkpoint cut short or with bytes flipped.
                2 => valid[..rng.below(valid.len())].to_vec(),
                _ => {
                    let mut bytes = valid.to_vec();
                    for _ in 0..1 + rng.below(4) {
                        let at = rng.below(bytes.len());
                        bytes[at] ^= 1 << rng.below(8);
                    }
                    bytes
                }
            };
            let adopted = adopt(&bytes, round % 2 == 0, "/home/u/src", "0.3.0", now);
            if adopted.restored.is_ok() {
                // What reads encodes again.
                encode(&adopted.session).expect("it fits");
            }
        }
    }
}
