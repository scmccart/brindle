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
//! | that many | the partial line's bytes |
//!
//! A segment's state has the base's fields except `log`, which becomes
//! `events`: the events the session applied since the previous record, in
//! order, with consecutive `Appended`s of one entry merged up to
//! [`TEXT_CHUNK`] and consecutive `ToolOutput`s of one item up to
//! [`ITEM_LIMIT`], the later exit kept (`agents-tool-content` decision 8).
//! Every other field is whole and replaces the previous
//! record's. [`decode`] folds the base's log, then each segment's events,
//! and takes the rest from the last record. [`encode`] writes the header
//! and a base; [`encode_segment`] one segment record.
//!
//! Format 1, which builds before this change wrote, is still read: the
//! header's format, then the JSON state's length (4 bytes, big-endian),
//! the state, and the rest the partial line's bytes.
//!
//! The base's state holds the log as the events of [`Transcript::replay`] (the
//! queue slot, the turn and the agent's state among them), the ACP version
//! (`acp`, always 1 here), the [`Phase`] (with the ACP session id once
//! ready, what a spawn starts or continues, what `initialize` offered, and
//! a load's muted count, and a login's message and what follows it), the
//! login methods the agent advertised (`methods`, left out when there are
//! none; OpenSpec change `agents-login`), the [`Rpc`] parts (the next outgoing id, the core's requests
//! awaiting a response and what each was for, the agent's waiting
//! permission requests by entry with their JSON-RPC ids as sent), the chunk
//! run, the tool entries by call id, the outputs still appended to by the
//! agent's terminal id (`outputs`: each one's entry, its `Output` item and
//! its plain-text filter's state, a half-received escape sequence or
//! character included; left out when there are none, as in a checkpoint
//! from before `agents-tool-content`), the count of a bad-line run not yet
//! written as a notice, and the [`Lines`] state: which partial it is and
//! `covers`, every stdout byte fed, the partial line's included. Since
//! OpenSpec change `agents-commands` (format 2, amended) it also holds
//! each held option's key by its id (`option_keys`) and the mode values
//! last in effect (`modes`), each left out when empty; the commands and
//! options themselves are log events. A set of an option out isn't kept.
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

use std::time::Instant;

use brindle_remote_protocol::holder::MAX_CHECKPOINT_LEN;
use brindle_remote_protocol::serde_json::Value;
use brindle_transcript::Event;

use super::reduce::AgentSession;

mod adopt;
mod format;
mod state;

#[cfg(test)]
mod harness;

pub use adopt::{NOT_RESTORED, adopt};
pub use format::{FORMAT, MAGIC, TooLarge, Unreadable};

use format::{BASE, FORMAT_1, HEADER_LEN, PREFIX_LEN, SEGMENT, next_record, push_record};
use state::{array, fold_log, merged, restore, restore_log, state_fields, state_object};

/// Encodes `session` whole (the header and a base record), or refuses
/// past [`MAX_CHECKPOINT_LEN`].
pub fn encode(session: &AgentSession) -> Result<Vec<u8>, TooLarge> {
    encode_within(session, MAX_CHECKPOINT_LEN)
}

fn encode_within(session: &AgentSession, limit: u64) -> Result<Vec<u8>, TooLarge> {
    let (mut state, partial) = state_fields(session);
    let events: Vec<Value> = session
        .transcript
        .replay()
        .iter()
        .map(Event::to_value)
        .collect();
    state.insert("log".into(), Value::Array(events));
    let mut out = MAGIC.to_vec();
    out.extend_from_slice(&FORMAT.to_be_bytes());
    push_record(&mut out, BASE, 0, &Value::Object(state), partial, limit)?;
    Ok(out)
}

/// One segment record, numbered `seq`, to append to a stored checkpoint:
/// `events`, the events `session` applied since the previous record, with
/// consecutive `Appended`s of one entry merged, and the rest of the session
/// whole. Refused past [`MAX_CHECKPOINT_LEN`] on its own; whether it fits
/// beside what is stored is the core's to judge.
pub fn encode_segment(
    session: &AgentSession,
    events: &[Event],
    seq: u32,
) -> Result<Vec<u8>, TooLarge> {
    let (mut state, partial) = state_fields(session);
    let events: Vec<Value> = merged(events).iter().map(Event::to_value).collect();
    state.insert("events".into(), Value::Array(events));
    let mut out = Vec::new();
    push_record(
        &mut out,
        SEGMENT,
        seq,
        &Value::Object(state),
        partial,
        MAX_CHECKPOINT_LEN,
    )?;
    Ok(out)
}

/// The session `bytes` encode, in format 1 or 2, or why they don't. A
/// bad-line run comes back with `now` as its last line's time, so its
/// notice is written a quiet second after the adoption.
pub fn decode(bytes: &[u8], now: Instant) -> Result<AgentSession, Unreadable> {
    let prefix = bytes
        .get(..PREFIX_LEN)
        .ok_or(Unreadable::Short { len: bytes.len() })?;
    let (magic, format) = prefix.split_at(4);
    if magic != MAGIC {
        return Err(Unreadable::NotACheckpoint);
    }
    match u16::from_be_bytes([format[0], format[1]]) {
        FORMAT_1 => decode_1(bytes, now),
        FORMAT => decode_2(&bytes[PREFIX_LEN..], now),
        format => Err(Unreadable::Format { format }),
    }
}

/// Format 1: the state's length, the state, the partial line.
fn decode_1(bytes: &[u8], now: Instant) -> Result<AgentSession, Unreadable> {
    let header = bytes
        .get(..HEADER_LEN)
        .ok_or(Unreadable::Short { len: bytes.len() })?;
    let state_len = u32::from_be_bytes([header[6], header[7], header[8], header[9]]);
    let body = &bytes[HEADER_LEN..];
    let state_len = usize::try_from(state_len).map_err(|_| Unreadable::Truncated)?;
    if state_len > body.len() {
        return Err(Unreadable::Truncated);
    }
    let (state, partial) = body.split_at(state_len);
    let state = state_object(state)?;
    restore(&state, None, partial, now)
}

/// Format 2: a base record, then segments in sequence.
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
    let mut partial = base.partial;
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
        partial = record.partial;
    }
    restore(&state, Some(transcript), partial, now)
}

#[cfg(test)]
mod tests {
    use brindle_remote_protocol::serde_json::{self, json};
    use brindle_transcript::{Entry, EntryId, Slot, TEXT_CHUNK};

    use crate::acp::dialect::Dialect;
    use crate::acp::lines::{Lines, Partial};
    use crate::acp::limits;
    use crate::acp::reduce::{self, Command, Input, Output, Phase, Refused, Replay, step};
    use crate::acp::rpc::BadParts;

    use super::format::RECORD_HEADER_LEN;
    use super::harness::*;
    use super::*;

    // --- bounds ---------------------------------------------------------

    #[test]
    fn a_checkpoint_past_the_limit_is_refused_with_its_length() {
        let agent = Agent::busy();
        let len = encode(&agent.s).expect("it fits").len() as u64;
        assert_eq!(
            encode_within(&agent.s, len),
            Ok(encode(&agent.s).expect("it fits"))
        );
        assert_eq!(encode_within(&agent.s, len - 1), Err(TooLarge { len }));
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
        agent.s.lines = Lines::restore(
            Partial::Collecting(vec![b'"'; super::super::lines::MAX_LINE + 1]),
            0,
        );
        let bytes = encode(&agent.s).expect("it fits in 64 MiB");
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
        let bytes = encode(&agent.s).expect("it fits");
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
        let mut old = bytes[..PREFIX_LEN].to_vec();
        push_record(&mut old, BASE, 0, &state, &partial, MAX_CHECKPOINT_LEN).expect("it fits");
        assert_eq!(decode(&old, agent.now).expect("it reads"), agent.s);
    }

    // --- checkpoints that don't read ------------------------------------

    /// A phase this core doesn't know (a later build's) is adopted as a
    /// conversation that could not be restored, its agent left running.
    #[test]
    fn an_unknown_phase_is_adopted_as_not_restored() {
        let agent = Agent::ready();
        let (mut state, partial) = state_of(&encode(&agent.s).expect("it fits"));
        state["phase"] = json!({"phase": "logging_out", "method": "m"});
        let adopted = adopt(&rebuild(&state, &partial), false, "/home/u/src", "0.3.0", agent.now);
        assert_eq!(adopted.restored, Err(Unreadable::Field { name: "phase" }));
        assert_not_restored(&adopted);
    }

    #[test]
    fn a_newer_format_is_adopted_as_a_conversation_that_could_not_be_restored() {
        let mut bytes = encode(&Agent::busy().s).expect("it fits");
        bytes[4..6].copy_from_slice(&3u16.to_be_bytes());
        let adopted = adopt(&bytes, false, "/home/u/src", "0.3.0", Instant::now());
        assert_eq!(adopted.restored, Err(Unreadable::Format { format: 3 }));
        assert_not_restored(&adopted);
        let entries = entries(&adopted.session);
        assert!(matches!(&entries[0], Entry::Notice { text, .. }
            if text.contains("format 3") && text.contains("reads formats 1 and 2")));
    }

    #[test]
    fn bytes_that_arent_a_checkpoint_are_adopted_as_not_restored() {
        for (bytes, why) in [
            (&b"BAC"[..], Unreadable::Short { len: 3 }),
            (&b"\x1bc\x1b[2Jhello there"[..], Unreadable::NotACheckpoint),
            (
                &b"BACP\x00\x01\x00\x00\x00\x09{}"[..],
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
            Input::Command(Command::Prompt { text: "hi".into() }),
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
        let (state, partial) = state_of(&encode(&agent.s).expect("it fits"));
        let mut bad_run = state.clone();
        bad_run["run"] = json!({"kind": "agent", "entry": 99});
        let mut bad_tool = state.clone();
        bad_tool["tools"] = json!({"c9": 99});
        let mut bad_permission = state.clone();
        bad_permission["rpc"]["permissions"] = json!([[1, 0]]);
        for (state, name, entry) in [
            (bad_run, "run", 99),
            (bad_tool, "tools", 99),
            (bad_permission, "permissions", 1),
        ] {
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
        let (mut state, partial) = state_of(&encode(&agent.s).expect("it fits"));
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
        let (mut state, partial) = state_of(&encode(&agent.s).expect("it fits"));
        state["acp"] = json!(2);
        assert_eq!(
            decode(&rebuild(&state, &partial), agent.now),
            Err(Unreadable::Acp { version: 2 })
        );
    }

    #[test]
    fn a_log_that_doesnt_fold_is_unreadable() {
        let agent = Agent::busy();
        let (mut state, partial) = state_of(&encode(&agent.s).expect("it fits"));
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
        let (mut state, partial) = state_of(&encode(&agent.s).expect("it fits"));
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
        agent.s.lines = Lines::restore(Partial::Resyncing { len: 3 }, 10);
        let (state, _) = state_of(&encode(&agent.s).expect("it fits"));
        assert_eq!(
            decode(&rebuild(&state, b"abc"), agent.now),
            Err(Unreadable::StrayBytes { len: 3 })
        );
    }

    #[test]
    fn a_missing_field_is_unreadable_naming_it() {
        let agent = Agent::ready();
        let (state, partial) = state_of(&encode(&agent.s).expect("it fits"));
        for name in [
            "acp",
            "cwd",
            "version",
            "phase",
            "log",
            "rpc",
            "run",
            "tools",
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
        assert!(matches!(decoded.lines.partial(), Partial::Collecting(bytes) if bytes.is_empty()));
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
            merged(&events),
            [appended(a, "one two"), appended(b, "x"), appended(a, "three")]
        );
        let half = "y".repeat(TEXT_CHUNK / 2 + 1);
        assert_eq!(
            merged(&[appended(a, &half), appended(a, &half)]).len(),
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

    /// A format 2 record of `kind`, numbered `seq`, with the state `state`.
    fn record(kind: u8, seq: u32, state: &Value) -> Vec<u8> {
        let mut out = Vec::new();
        push_record(&mut out, kind, seq, state, &[], MAX_CHECKPOINT_LEN).expect("it fits");
        out
    }

    #[test]
    fn a_format_2_checkpoint_out_of_order_or_cut_short_is_adopted_as_not_restored() {
        let agent = Agent::ready();
        let base = encode(&agent.s).expect("it fits");
        let (state, _) = state_of(&base);
        let mut segment_state = state.clone();
        if let Some(object) = segment_state.as_object_mut() {
            object.remove("log");
            object.insert("events".into(), json!([]));
        }
        let header = base[..PREFIX_LEN].to_vec();
        let joined = |parts: &[&[u8]]| parts.concat();
        let segment = |seq| record(SEGMENT, seq, &segment_state);
        let cases: Vec<(Vec<u8>, Unreadable)> = vec![
            (header.clone(), Unreadable::NoBase),
            (joined(&[&header, &segment(0)]), Unreadable::NoBase),
            (
                joined(&[&header, &record(b'X', 0, &state)]),
                Unreadable::RecordKind { kind: b'X' },
            ),
            (
                joined(&[&base, &record(BASE, 1, &state)]),
                Unreadable::RecordKind { kind: BASE },
            ),
            (
                joined(&[&header, &record(BASE, 1, &state)]),
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
    }

    #[test]
    fn a_format_1_checkpoint_with_a_title_and_a_login_still_adopts() {
        let mut agent = Agent::ready();
        agent.s.methods = creating().s.methods;
        agent.feed(&update(json!({"sessionUpdate": "session_info_update",
            "title": "Hello.txt file creation"})));
        assert_eq!(agent.prompt("hi").reply, Some(Ok(())));
        agent.feed(&auth_error(3, "expired"));
        let (state, partial) = state_of(&encode(&agent.s).expect("it fits"));
        let format_1 = rebuild(&state, &partial);
        assert_eq!(&format_1[4..6], &[0, 1]);
        let adopted = adopt(&format_1, false, "/home/u/src", "0.3.0", agent.now);
        assert_eq!(adopted.restored, Ok(()));
        assert_eq!(adopted.session, agent.s);
        assert!(matches!(adopted.session.phase, Phase::LoginRequired(_)));
    }

    /// `agents-tool-content` decision 2: the `acp` field a checkpoint from
    /// before `Dialect` recorded restores the session's dialect, in either
    /// format, and the pinned bytes below still decode.
    #[test]
    fn a_format_1_or_2_checkpoint_restores_its_dialect() {
        let agent = Agent::ready();
        let format_2 = encode(&agent.s).expect("it fits");
        let (state, partial) = state_of(&format_2);
        assert_eq!(state["acp"], 1);
        let format_1 = rebuild(&state, &partial);
        for bytes in [format_1, format_2] {
            let session = decode(&bytes, agent.now).expect("it decodes");
            assert_eq!(session.rpc.dialect(), Dialect::newest());
            assert_eq!(session.rpc.dialect().version(), 1);
        }
        let pinned: Vec<u8> = (0..PINNED.len())
            .step_by(2)
            .map(|at| u8::from_str_radix(&PINNED[at..at + 2], 16).expect("hex"))
            .collect();
        let session = decode(&pinned, Instant::now()).expect("it decodes");
        assert_eq!(session.rpc.dialect().version(), 1);
    }

    #[test]
    fn a_small_sessions_format_2_bytes_are_pinned() {
        let session = AgentSession::new("/w".into(), "0.3.0".into());
        let bytes = encode(&session).expect("it fits");
        let hex: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, PINNED);
        assert_eq!(decode(&bytes, Instant::now()), Ok(session));
    }

    /// `BACP`, format 2, a base record numbered 0 with the state's length,
    /// the state, and no partial line.
    const PINNED: &str = concat!(
        "4241435000024200000000000001267b22616370223a312c22637764223a222f77222c2276657273",
        "696f6e223a22302e332e30222c227068617365223a7b227068617365223a22737061776e696e6722",
        "7d2c22727063223a7b226e6578745f6964223a312c226f7574676f696e67223a5b5d2c227065726d",
        "697373696f6e73223a5b5d7d2c2272756e223a6e756c6c2c22746f6f6c73223a7b7d2c226261645f",
        "6c696e6573223a302c226c696e6573223a7b227061727469616c223a22636f6c6c656374696e6722",
        "2c22636f76657273223a307d2c226c6f67223a5b7b226576656e74223a227475726e222c22737461",
        "7465223a7b227475726e223a2269646c65227d7d2c7b226576656e74223a226167656e74222c2273",
        "74617465223a7b226167656e74223a227374617274696e67227d7d5d7d00000000",
    );

    fn error(id: u64, code: i64, message: &str) -> Vec<u8> {
        line(json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}))
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
            for _ in 0..40 {
                let out = match rng.below(20) {
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
                        Some(id) => agent.feed(&error(id, crate::acp::forms::AUTH_REQUIRED, "log in")),
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
        let valid = encode(&Agent::busy().s).expect("it fits");
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
        for round in 0..3000 {
            let valid = if round % 3 == 0 { &segmented } else { &valid };
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
