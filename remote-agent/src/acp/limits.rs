//! The bounds on what an agent session holds and sends, in one place
//! (OpenSpec change `agents-event-bounds`, design decision 1), and the two
//! invariants they add up to:
//!
//! - **I1.** Every transcript event the reducer emits, and every event of
//!   [`Transcript::replay`](brindle_transcript::Transcript::replay),
//!   `page` and `fetch` of a log the reducer built, encodes to at most
//!   [`MAX_EVENT`], so it fits a `TRANSCRIPT` or an `AGENT_PAGE` frame
//!   with its envelope (asserted below). The reducer measures each event
//!   before applying it, and one past the bound fails the session saying
//!   what it was (`Step::emit`): the bounds make that unreachable, and the
//!   guard makes a later mistake loud.
//! - **I2.** After every step the log is within [`LOG_LIMIT`], and a whole
//!   checkpoint of any session the reducer can reach encodes within
//!   [`MAX_CHECKPOINT_LEN`], so `checkpoint::TooLarge` is unreachable.
//!
//! **The derivation of I1** (design decision 1's table). Every string the
//! reducer emits is cleaned, so it encodes to at most twice its bytes
//! (`brindle_transcript::escaped_len`); a permission option's id is left
//! out when it holds a control character, so it does too; `refuse`'s
//! message is cut uncleaned, at most six times. With `F` = 128 bytes for an
//! event's tag, id and field names, and `T` = [`TEXT_CHUNK`]:
//!
//! | Event | Largest fields | At most (bytes) |
//! | --- | --- | --- |
//! | `Added` of text; `Appended` | one `chunks` piece | 2T + F |
//! | `Added` of a notice | a [`FIELD_LIMIT`] text | 2T + F |
//! | `Added` of a tool call; `ToolChanged` | call and title [`FIELD_LIMIT`], 32 locations of [`MAX_PATH`], raw input and result [`RAW_LIMIT`] | [`TOOL_HEADER_EVENT`] |
//! | `ToolItems` | items of at most T of `wire_len` together, or one larger item alone (at most [`LARGEST_ITEM`]) | [`TOOL_ITEMS_EVENT`] |
//! | `ToolOutput` | a piece or the filter's held line, a [`SIGNAL_RESERVE`] name | 2T + 128 + F |
//! | `Added` of a permission | call, title, [`MAX_PERMISSION_OPTIONS`] options of [`MAX_PERMISSION_ID`] and [`MAX_PERMISSION_NAME`] | [`PERMISSION_EVENT`] |
//! | `Answered` | an option id | 2 × [`MAX_PERMISSION_ID`] + F |
//! | `Queue` | a [`MAX_PROMPT`] prompt, a held slot's [`FIELD_LIMIT`] stop name | [`QUEUE_EVENT`] |
//! | `Turn` | a failure message or stop name | 2T + F |
//! | `Agent` | `LoginRequired`'s message and `MAX_LOGIN_METHODS_LEN` of methods, or `refuse`'s 6T | 2T + `MAX_LOGIN_METHODS_LEN` + F |
//! | `Conversation` | a [`MAX_SESSION_ID`] id, the directory the agent was started in (at most 96 KiB as UTF-8), a 256-character title | 2 × (1 KiB + 96 KiB + 1 KiB) + F |
//! | `Commands`, `Options` | the whole list, kept within T encoded | T + F |
//! | `Trimmed`, `Window` | two integers | F |
//!
//! A replay sends the same events: text in `chunks` pieces, a tool entry's
//! items grouped as the live split groups them (`item_pieces`), and the
//! state as its latest events.
//!
//! **The derivation of I2.** A trim at each step's end drops whole entries
//! down to [`LOG_TRIM_TO`], which it reaches when the newest entry alone is
//! within it: a text entry holds at most [`ENTRY_LIMIT`] (the reducer
//! starts a new one past it), so the largest entry, [`LARGEST_ENTRY`], is.
//! A stored log (its replay and the tools map) is at most twice its
//! measure, since the measure counts `ENTRY_FRAMING` an entry; the rest of
//! the state is the partial line, the login methods, the open outputs and
//! [`STATE_SLACK`] for everything else. [`LARGEST_CHECKPOINT`] sums them.
//!
//! **Every other bound and its home**: `TEXT_CHUNK`, `ITEM_LIMIT`,
//! `MAX_ITEMS`, `CALL_LIMIT`, `RAW_LIMIT`, `MAX_LOCATIONS`, `MAX_PATH`,
//! `PAGE_BYTES`, `MAX_EVENT` and the framings (`brindle-transcript`);
//! `MAX_COMMANDS`, `MAX_OPTIONS`, `MAX_CHOICES` (transcript) with
//! `MAX_OPTION_ID`, `MAX_OPTION_NAME`, `MAX_OPTION_DESCRIPTION`,
//! `MAX_SESSION_TITLE`, `MAX_LISTED*`, `MAX_CURSOR` and `MAX_OPEN_OUTPUTS`
//! (`acp::reduce`); the login methods' bounds (`acp::forms`);
//! `MAX_PENDING_PERMISSIONS` (`acp::rpc`); `MAX_ID_LEN` (`acp::envelope`);
//! `MAX_LINE` (`acp::lines`); `MAX_DIFF_LINES` (`acp::diff`);
//! `plain::MAX_STRING`; and the frame, envelopes, checkpoint and tails
//! (`brindle-remote-protocol`). A new field an agent fills gets a bound
//! here or beside its decoder, a row above, and a generator in this
//! module's property test.
//!
//! The `const` assertions write the linear sums from the constants, so
//! raising a bound past them fails the build. The factor 2 and
//! [`STATE_SLACK`] aren't constants of the code: the derivation test builds
//! each extreme, and the property test drives a session with adversarial
//! agent output and checks both invariants after every step.
//!
//! A whole pure module (`fcis` R2, R3; `PURE_MODULES`).

use brindle_remote_protocol::MAX_FRAME_PAYLOAD_SIZE;
use brindle_remote_protocol::client::{AGENT_PAGE_ENVELOPE, TRANSCRIPT_ENVELOPE};
use brindle_remote_protocol::holder::MAX_CHECKPOINT_LEN;
use brindle_transcript::{
    ENTRY_FRAMING, ITEM_LIMIT, MAX_EVENT, MAX_ITEMS, MAX_LOCATIONS, MAX_PATH, RAW_LIMIT,
    SIGNAL_RESERVE, TEXT_CHUNK,
};

use super::forms::MAX_LOGIN_METHODS_LEN;
use super::lines::MAX_LINE;
use super::reduce::MAX_OPEN_OUTPUTS;

/// The most the log keeps, measured (`agents-sessions` decision 14, each
/// entry with its `ENTRY_FRAMING`): past it the oldest entries are dropped
/// whole, with a `Trimmed` event, down to [`LOG_TRIM_TO`].
pub const LOG_LIMIT: usize = 16 * 1024 * 1024;

/// What a trim leaves the log at: a low mark below [`LOG_LIMIT`], so a long
/// conversation trims once per 4 MiB rather than once per entry, and each
/// trim's whole checkpoint is rare (OpenSpec change `agents-log-paging`,
/// decision 6).
pub const LOG_TRIM_TO: usize = 12 * 1024 * 1024;

/// The longest prompt taken, in bytes of UTF-8 (`agents-sessions`
/// decision 10).
pub const MAX_PROMPT: usize = 256 * 1024;

/// The most text one user, agent or thought entry holds (design decision
/// 2): streamed text past it continues in a new entry of the same kind.
pub const ENTRY_LIMIT: usize = 1024 * 1024;

/// The most every other string field keeps, cut at a character boundary:
/// titles, call ids, commands, notices, messages, a stop reason's name.
pub const FIELD_LIMIT: usize = TEXT_CHUNK;

/// The most options a permission request keeps, in the agent's order.
pub const MAX_PERMISSION_OPTIONS: usize = 16;

/// The longest permission option id kept: an answer sends it back, so a
/// longer one (or an empty one, or one holding a control character) leaves
/// its option out rather than be cut.
pub const MAX_PERMISSION_ID: usize = 1024;

/// A permission option's name is cut at this many bytes.
pub const MAX_PERMISSION_NAME: usize = 4 * 1024;

/// A terminal id longer than this is ignored: its output isn't kept, since
/// a cut id could merge two terminals.
pub const MAX_TERMINAL_ID: usize = 1024;

/// The longest ACP session id taken from `session/new`: every later
/// request sends it back, so a longer one fails the start.
pub const MAX_SESSION_ID: usize = 1024;

/// What a whole checkpoint holds beside the log, the partial line, the
/// login methods and the open outputs: the state events, the phase with its
/// greeting, login and conversation, the pending permissions' ids, the run,
/// the lines' state, the option keys and modes, the headers.
pub const STATE_SLACK: usize = 3 * 1024 * 1024;

/// An event's tag, id and field names, at most.
const EVENT_FIELDS: usize = 128;

/// The most a tool entry's header event (`Added` without content, or
/// `ToolChanged`) encodes to.
pub const TOOL_HEADER_EVENT: usize = 2 * FIELD_LIMIT
    + 2 * FIELD_LIMIT
    + MAX_LOCATIONS * (2 * MAX_PATH + 40)
    + 2 * 2 * RAW_LIMIT
    + EVENT_FIELDS;

/// The largest one tool item measures (`ToolItem::wire_len`): an output's
/// text, command, directory and exit.
pub const LARGEST_ITEM: usize = ITEM_LIMIT + FIELD_LIMIT + MAX_PATH + SIGNAL_RESERVE;

/// The most a `ToolItems` event encodes to: a piece of at most `TEXT_CHUNK`
/// of `wire_len` or one item alone, escaped, with each item's own fields.
pub const TOOL_ITEMS_EVENT: usize = 2 * LARGEST_ITEM + MAX_ITEMS * 192 + EVENT_FIELDS;

/// The most a permission's `Added` encodes to.
pub const PERMISSION_EVENT: usize = 2 * FIELD_LIMIT
    + 2 * FIELD_LIMIT
    + MAX_PERMISSION_OPTIONS * (2 * MAX_PERMISSION_ID + 2 * MAX_PERMISSION_NAME + 48)
    + EVENT_FIELDS;

/// The most a `Queue` event encodes to: the largest of all.
pub const QUEUE_EVENT: usize = 2 * MAX_PROMPT + 2 * FIELD_LIMIT + EVENT_FIELDS;

/// The largest entry the log can hold, measured: a text entry of
/// [`ENTRY_LIMIT`], escaped, with its framing and its fields.
pub const LARGEST_ENTRY: usize = 2 * ENTRY_LIMIT + ENTRY_FRAMING + 64;

/// The most a whole checkpoint of a reachable session encodes to (I2).
pub const LARGEST_CHECKPOINT: u64 = (2 * LOG_LIMIT
    + MAX_LINE
    + 1
    + MAX_LOGIN_METHODS_LEN
    + MAX_OPEN_OUTPUTS * (MAX_TERMINAL_ID + 2 * ITEM_LIMIT + 128)
    + STATE_SLACK) as u64;

// I1: one event fits either frame with its envelope, and the largest of
// each row is within `MAX_EVENT`.
const _: () = assert!(MAX_EVENT + AGENT_PAGE_ENVELOPE <= MAX_FRAME_PAYLOAD_SIZE);
const _: () = assert!(AGENT_PAGE_ENVELOPE >= TRANSCRIPT_ENVELOPE);
const _: () = assert!(2 * TEXT_CHUNK + 128 + EVENT_FIELDS <= MAX_EVENT);
const _: () = assert!(TOOL_HEADER_EVENT <= MAX_EVENT);
const _: () = assert!(TOOL_ITEMS_EVENT <= MAX_EVENT);
const _: () = assert!(PERMISSION_EVENT <= MAX_EVENT);
const _: () = assert!(QUEUE_EVENT <= MAX_EVENT);
const _: () = assert!(6 * FIELD_LIMIT + EVENT_FIELDS <= MAX_EVENT);
const _: () = assert!(2 * FIELD_LIMIT + MAX_LOGIN_METHODS_LEN + EVENT_FIELDS <= MAX_EVENT);
// I2: every trim makes progress and ends within its mark, and a whole
// checkpoint fits what a holder stores.
const _: () = assert!(LARGEST_ENTRY <= LOG_TRIM_TO);
const _: () = assert!(TOOL_HEADER_EVENT + 2 * 256 * 1024 + MAX_ITEMS * 192 <= LOG_TRIM_TO);
const _: () = assert!(LOG_TRIM_TO < LOG_LIMIT);
const _: () = assert!(LARGEST_CHECKPOINT <= MAX_CHECKPOINT_LEN);

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use brindle_remote_protocol::client::pack_transcript;
    use brindle_remote_protocol::serde_json::{Value, json};
    use brindle_transcript::{
        AgentState, CALL_LIMIT, Continuation, Conversation, Entry, EntryId, Event, Exit,
        HoldReason, Location, LoginKind, LoginMethod, NoticeLevel, Output, PAGE_BYTES,
        PermissionKind, PermissionOption, PermissionOutcome, Slot, StopReason, ToolItem, ToolKind,
        ToolStatus, Transcript, TurnState,
    };

    use super::super::checkpoint;
    use super::super::lines::{Lines, Partial};
    use super::super::plain;
    use super::super::reduce::{AgentSession, Command, Input, OpenOutput, step};
    use super::*;

    /// `len` bytes of `"`, which escapes to twice its bytes.
    fn quotes(len: usize) -> String {
        "\"".repeat(len)
    }

    fn assert_fits(name: &str, event: &Event) {
        let len = event.encoded_len();
        assert!(len <= MAX_EVENT, "{name}: {len} bytes");
    }

    /// Design decision 8's derivation test: each row of decision 1's table
    /// at its largest, every field at its bound and escaping to twice its
    /// bytes, within `MAX_EVENT`; the session of I2's sum encodes; and a
    /// log's stored form is at most twice its measure for each entry
    /// kind's smallest form.
    #[test]
    fn the_frame_and_checkpoint_bounds_follow_from_the_constants() {
        let id = EntryId(u64::MAX);
        let text = quotes(TEXT_CHUNK);
        let field = quotes(FIELD_LIMIT);
        let locations: Vec<Location> = (0..MAX_LOCATIONS)
            .map(|_| Location {
                path: quotes(MAX_PATH),
                line: Some(u32::MAX),
            })
            .collect();
        let tool = Entry::Tool {
            call: field.clone(),
            title: field.clone(),
            kind: ToolKind::Other,
            status: ToolStatus::InProgress,
            content: Vec::new(),
            more: u32::MAX,
            locations: locations.clone(),
            input: Some(quotes(RAW_LIMIT)),
            result: Some(quotes(RAW_LIMIT)),
        };
        let output = ToolItem::Output(Output {
            command: Some(field.clone()),
            cwd: Some(quotes(MAX_PATH)),
            text: quotes(ITEM_LIMIT).into(),
            omitted: u64::MAX,
            partial: true,
            exit: Some(Exit::Signal {
                name: quotes(SIGNAL_RESERVE),
            }),
        });
        assert!(output.wire_len() <= LARGEST_ITEM);
        let small: Vec<ToolItem> = (0..MAX_ITEMS)
            .map(|_| ToolItem::Text {
                text: quotes(TEXT_CHUNK / MAX_ITEMS),
                cut: true,
            })
            .collect();
        assert_eq!(brindle_transcript::item_pieces(&small).len(), 1);
        let options: Vec<PermissionOption> = (0..MAX_PERMISSION_OPTIONS)
            .map(|_| PermissionOption {
                id: quotes(MAX_PERMISSION_ID),
                name: quotes(MAX_PERMISSION_NAME),
                kind: PermissionKind::RejectAlways,
            })
            .collect();
        let events = [
            ("agent text", Event::Added { id, entry: Entry::Agent { text: text.clone() } }),
            ("appended", Event::Appended { id, text: text.clone() }),
            ("notice", Event::Added { id, entry: Entry::Notice { level: NoticeLevel::Warning, text: field.clone() } }),
            ("tool", Event::Added { id, entry: tool }),
            ("tool changed", Event::ToolChanged {
                id,
                title: Some(field.clone()),
                kind: Some(ToolKind::Other),
                status: Some(ToolStatus::InProgress),
                locations: Some(locations),
                input: Some(Some(quotes(RAW_LIMIT))),
                result: Some(Some(quotes(RAW_LIMIT))),
                more: Some(u32::MAX),
            }),
            ("one output", Event::ToolItems { id, from: u32::MAX, items: vec![output] }),
            ("small items", Event::ToolItems { id, from: u32::MAX, items: small }),
            ("tool output", Event::ToolOutput {
                id,
                item: u32::MAX,
                text: quotes(ITEM_LIMIT),
                exit: Some(Exit::Signal { name: quotes(SIGNAL_RESERVE) }),
            }),
            ("permission", Event::Added {
                id,
                entry: Entry::Permission {
                    call: Some(field.clone()),
                    title: field.clone(),
                    options,
                    outcome: Some(PermissionOutcome::Selected { option: quotes(MAX_PERMISSION_ID) }),
                },
            }),
            ("answered", Event::Answered {
                id,
                outcome: PermissionOutcome::Selected { option: quotes(MAX_PERMISSION_ID) },
            }),
            ("queue", Event::Queue {
                slot: Slot::Held {
                    text: quotes(MAX_PROMPT),
                    reason: HoldReason::Stopped { stop: StopReason::Other { name: field.clone() } },
                },
            }),
            ("turn", Event::Turn { state: TurnState::Failed { message: field.clone() } }),
            ("refused", Event::Agent {
                state: AgentState::Failed { message: "\u{1}".repeat(FIELD_LIMIT) },
            }),
            ("conversation", Event::Conversation {
                conversation: Conversation {
                    id: quotes(MAX_SESSION_ID),
                    cwd: quotes(96 * 1024),
                    continuation: Continuation::ResumeAndLoad,
                    title: Some("\u{10ffff}".repeat(256)),
                },
            }),
            ("trimmed", Event::Trimmed { through: id, dropped: u64::MAX }),
            ("window", Event::Window { from: id, earlier: u64::MAX }),
        ];
        for (name, event) in &events {
            assert_fits(name, event);
        }
        // The largest login state: methods at their encoded bound beside a
        // full message.
        let method = |n: usize| LoginMethod {
            id: format!("m{n}"),
            name: quotes(1024),
            description: Some(quotes(4 * 1024)),
            kind: LoginKind::Agent,
        };
        let mut methods = Vec::new();
        while brindle_remote_protocol::serde_json::to_vec(&methods).map_or(0, |v| v.len())
            < MAX_LOGIN_METHODS_LEN - 11 * 1024
        {
            methods.push(method(methods.len()));
        }
        let login = Event::Agent {
            state: AgentState::LoginRequired {
                message: field.clone(),
                methods: methods.clone(),
            },
        };
        assert_fits("login required", &login);

        // I2's session: a log of `LOG_LIMIT` in empty user entries with
        // 20-digit ids, 64 open outputs with full held lines, full methods
        // and partial line, and the largest slot, conversation and agent
        // state.
        let mut session = AgentSession::new(quotes(96 * 1024), "0.3.0".into());
        let log = &mut session.transcript;
        log.apply(&Event::Trimmed {
            through: EntryId(10_000_000_000_000_000_000),
            dropped: 1,
        })
        .expect("a trim on an empty log");
        while log.encoded_len() + 100 <= LOG_LIMIT {
            let id = log.next_id();
            log.apply(&Event::Added { id, entry: Entry::User { text: String::new() } })
                .expect("an entry");
        }
        for (name, event) in &events[10..14] {
            log.apply(event).unwrap_or_else(|e| panic!("{name}: {e}"));
        }
        for n in 0..MAX_OPEN_OUTPUTS {
            session.process.book.outputs.insert(
                format!("{n:0>width$}", width = MAX_TERMINAL_ID),
                OpenOutput {
                    entry: EntryId(u64::MAX),
                    item: u32::MAX,
                    filter: plain::State {
                        line: quotes(ITEM_LIMIT),
                        ..plain::State::default()
                    },
                },
            );
        }
        session.methods = methods;
        session.process.lines = Lines::restore(Partial::Collecting(vec![b'"'; MAX_LINE + 1]), u64::MAX);
        let bytes = checkpoint::encode(&session).expect("the largest session fits").bytes;
        assert!(bytes.len() as u64 <= LARGEST_CHECKPOINT, "{}", bytes.len());

        // A stored log is at most twice its measure, for each entry kind's
        // smallest form (decision 1, I2's second fact), at 20-digit ids.
        let smallest = [
            Entry::User { text: String::new() },
            Entry::Agent { text: String::new() },
            Entry::Thought { text: String::new() },
            Entry::Notice { level: NoticeLevel::Info, text: String::new() },
            Entry::tool(String::new(), String::new(), ToolKind::Other, ToolStatus::Pending),
            Entry::Tool {
                call: "c".into(),
                title: String::new(),
                kind: ToolKind::Other,
                status: ToolStatus::Pending,
                content: (0..MAX_ITEMS).map(|_| ToolItem::Text { text: String::new(), cut: false }).collect(),
                more: 0,
                locations: Vec::new(),
                input: None,
                result: None,
            },
            Entry::Permission {
                call: None,
                title: String::new(),
                options: Vec::new(),
                outcome: Some(PermissionOutcome::Cancelled),
            },
        ];
        for entry in smallest {
            let mut log = Transcript::new();
            log.apply(&Event::Trimmed {
                through: EntryId(10_000_000_000_000_000_000),
                dropped: 1,
            })
            .expect("a trim");
            for _ in 0..100 {
                let id = log.next_id();
                log.apply(&Event::Added { id, entry: entry.clone() }).expect("an entry");
            }
            let stored: usize = log.replay().iter().map(Event::encoded_len).sum::<usize>()
                + log
                    .entries()
                    .map(|(_, entry)| match entry {
                        Entry::Tool { call, .. } => call.len() + 24,
                        Entry::User { .. }
                        | Entry::Agent { .. }
                        | Entry::Thought { .. }
                        | Entry::Permission { .. }
                        | Entry::Notice { .. } => 0,
                    })
                    .sum::<usize>();
            assert!(stored <= 2 * log.encoded_len(), "{entry:?}: {stored} stored, {} measured", log.encoded_len());
        }
    }

    // --- the adversarial property (design decision 8) -------------------

    /// A seeded XorShift, and whether this move is an extreme one, whose
    /// every size is its largest.
    struct XorShift(u64, bool);

    impl XorShift {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, n: usize) -> usize {
            (self.next() % n.max(1) as u64) as usize
        }

        /// A count below `n`: in an extreme move `n - 1`.
        fn count(&mut self, n: usize) -> usize {
            if self.1 { n - 1 } else { self.below(n) }
        }

        /// A size: in an extreme move `large`, else mostly small and
        /// sometimes up to `large`.
        fn size(&mut self, large: usize) -> usize {
            if self.1 {
                return large;
            }
            match self.below(10) {
                0 => self.below(large),
                1..=3 => self.below(large.min(64 * 1024)),
                _ => self.below(1024),
            }
        }

        /// Quote-, backslash- and control-heavy text of `len` bytes or a
        /// little more, built from a repeated piece so it costs little.
        fn text(&mut self, len: usize) -> String {
            const CHARS: [&str; 10] = ["\"", "\\", "\u{1}", "\n", "é", "a", "\t", "\r", "\u{7f}", "\u{1b}"];
            let piece: String = (0..16).map(|_| CHARS[self.below(CHARS.len())]).collect();
            piece.repeat(len / piece.len() + 1)
        }

        /// [`Self::text`] of a [`Self::size`] up to `large`.
        fn sized(&mut self, large: usize) -> String {
            let len = self.size(large);
            self.text(len)
        }

        /// [`Self::text`] of up to `most` bytes.
        fn upto(&mut self, most: usize) -> String {
            let len = self.below(most);
            self.text(len)
        }
    }

    fn line(value: Value) -> Vec<u8> {
        let mut line = value.to_string().into_bytes();
        line.push(b'\n');
        line
    }

    fn update(update: Value) -> Vec<u8> {
        line(json!({"jsonrpc": "2.0", "method": "session/update",
            "params": {"sessionId": "s1", "update": update}}))
    }

    /// One adversarial line from the agent, or a prompt, per decision 8's
    /// generators.
    enum Move {
        Line(Vec<u8>),
        Prompt(String),
        Answer(Value),
    }

    fn terminal(rng: &mut XorShift) -> String {
        if rng.below(10) == 0 {
            "t".repeat(64 * 1024)
        } else {
            format!("t{}", rng.below(6))
        }
    }

    fn content(rng: &mut XorShift) -> Value {
        let items: Vec<Value> = (0..rng.count(41))
            .map(|_| match rng.below(5) {
                0 => json!({"type": "content", "content": {"type": "text", "text": rng.sized(200 * 1024)}}),
                1 => json!({"type": "diff", "path": rng.sized(64 * 1024), "oldText": null,
                    "newText": "\n".repeat(rng.count(300_001))}),
                2 => {
                    let lines = super::super::diff::MAX_DIFF_LINES / 2 + 1;
                    json!({"type": "diff", "path": "/p", "oldText": "o\n".repeat(lines), "newText": "n\"\n".repeat(lines)})
                }
                _ => json!({"type": "terminal", "terminalId": terminal(rng)}),
            })
            .collect();
        Value::Array(items)
    }

    fn raw(rng: &mut XorShift) -> Value {
        let mut value = Value::Array(vec![json!(0); rng.count(20_001)]);
        for _ in 0..rng.count(101) {
            value = Value::Array(vec![value]);
        }
        if rng.below(2) == 0 {
            json!(rng.sized(1024 * 1024))
        } else {
            value
        }
    }

    fn adversarial(rng: &mut XorShift, prompt: Option<u64>, asked: &mut u64) -> Move {
        rng.1 = rng.below(8) == 0;
        let call = format!("c{}", rng.below(8));
        match rng.below(12) {
            0..=2 => {
                let kind = ["agent_message_chunk", "agent_thought_chunk", "user_message_chunk"][rng.below(3)];
                let text = rng.sized(2 * 1024 * 1024);
                Move::Line(update(json!({"sessionUpdate": kind, "content": {"type": "text", "text": text}})))
            }
            3 | 4 => {
                let kind = ["tool_call", "tool_call_update"][rng.below(2)];
                let locations: Vec<Value> = (0..rng.count(41))
                    .map(|_| json!({"path": rng.sized(64 * 1024), "line": rng.below(100)}))
                    .collect();
                Move::Line(update(json!({"sessionUpdate": kind, "toolCallId": call,
                    "title": rng.sized(100 * 1024), "kind": "execute", "status": "in_progress",
                    "content": content(rng), "locations": locations, "rawInput": raw(rng), "rawOutput": raw(rng)})))
            }
            5 | 6 => {
                let id = terminal(rng);
                let meta = match rng.below(3) {
                    0 => json!({"terminal_info": {"terminal_id": id, "cwd": rng.sized(64 * 1024),
                        "command": rng.sized(100 * 1024)}}),
                    1 => json!({"terminal_output_delta": {"terminal_id": id, "data": rng.sized(256 * 1024)}}),
                    _ => json!({"terminal_exit": {"terminal_id": id, "exit_code": null,
                        "signal": rng.sized(64 * 1024)}}),
                };
                Move::Line(update(json!({"sessionUpdate": "tool_call_update", "toolCallId": call, "_meta": meta})))
            }
            7 => {
                *asked += 1;
                let options: Vec<Value> = (0..rng.count(41))
                    .map(|n| {
                        // Mostly ids Brindle can send back, so the kept
                        // options reach their bound.
                        let id = match rng.below(8) {
                            0 => String::new(),
                            1 => "i".repeat(2048),
                            2 => format!("o{n}\u{1}"),
                            3 => format!("{n:0>1024}"),
                            _ => format!("o{n}"),
                        };
                        json!({"optionId": id, "name": rng.sized(64 * 1024), "kind": "allow_once"})
                    })
                    .collect();
                Move::Line(line(json!({"jsonrpc": "2.0", "id": format!("p{asked}"), "method": "session/request_permission",
                    "params": {"sessionId": "s1", "toolCall": {"toolCallId": call, "title": rng.sized(100 * 1024)},
                        "options": options}})))
            }
            8 | 9 => match prompt {
                Some(id) => {
                    let stop = if rng.below(2) == 0 { "end_turn".to_owned() } else { "z".repeat(rng.size(100 * 1024)) };
                    Move::Answer(json!({"jsonrpc": "2.0", "id": id, "result": {"stopReason": stop}}))
                }
                None => Move::Prompt(rng.upto(MAX_PROMPT / 2)),
            },
            _ => Move::Prompt(rng.upto(MAX_PROMPT / 2)),
        }
    }

    /// The tool entry an event may change.
    fn touched(event: &Event) -> Option<EntryId> {
        match event {
            Event::Added { id, .. }
            | Event::ToolChanged { id, .. }
            | Event::ToolItems { id, .. }
            | Event::ToolOutput { id, .. } => Some(*id),
            Event::Appended { .. }
            | Event::Answered { .. }
            | Event::Queue { .. }
            | Event::Turn { .. }
            | Event::Agent { .. }
            | Event::Conversation { .. }
            | Event::Trimmed { .. }
            | Event::Window { .. }
            | Event::Commands { .. }
            | Event::Options { .. } => None,
        }
    }

    /// Every event of `events` encodes within `MAX_EVENT`.
    fn all_fit(what: &str, events: &[Event]) {
        for event in events {
            let len = event.encoded_len();
            assert!(len <= MAX_EVENT, "{what}: a {} event of {len} bytes", event.kind());
        }
    }

    /// How many steps [`every_event_fits_a_frame_over_adversarial_agent_output`]
    /// takes. Design decision 8 asks for 2,000, which took 110 s in a
    /// debug build (the reducer's own handling of the large lines alone
    /// about 51 s) before one move in eight became an extreme one, past
    /// the debug suite's 30 s (task 5.2), so it is cut to 300 (about 20 s
    /// alone): the log still passes `LOG_LIMIT` and is trimmed, which the
    /// test asserts.
    const STEPS: usize = 300;

    /// Design decision 8's property: [`STEPS`] seeded steps of
    /// adversarial agent output, each step's events within `MAX_EVENT`
    /// and packed, the log within `LOG_LIMIT`, the tool entries the step
    /// changed within the fold's bounds, every open output an output
    /// item, no fault; every 40 steps the replay, pages and a fetch within
    /// `MAX_EVENT` (each a walk of the whole log, about 0.4 s in a debug
    /// build, too slow for every step), and every 100 a whole checkpoint.
    /// A waiting permission is answered and a held prompt let go in place
    /// of a prompt, so turns keep coming. A long session id is `reduce`'s
    /// own test's (`a_session_id_past_its_bound_fails_the_start`), since
    /// it ends the session.
    #[test]
    fn every_event_fits_a_frame_over_adversarial_agent_output() {
        let now = Instant::now();
        let mut s = AgentSession::new("/w".into(), "0.3.0".into());
        step(&mut s, Input::Spawned, now);
        step(
            &mut s,
            Input::Stdout(&line(json!({"jsonrpc": "2.0", "id": 1,
                "result": {"protocolVersion": 1, "agentInfo": {"name": "fake", "version": "1.0"}}}))),
            now,
        );
        step(&mut s, Input::Stdout(&line(json!({"jsonrpc": "2.0", "id": 2, "result": {"sessionId": "s1"}}))), now);
        let mut rng = XorShift(0xeb_0005_2000, false);
        let mut prompt: Option<u64> = None;
        let mut asked = 0;
        // Events by kind, to show the run reached what it is for.
        let mut kinds = std::collections::BTreeMap::<&str, usize>::new();
        for at in 0..STEPS {
            let out = match adversarial(&mut rng, prompt, &mut asked) {
                Move::Line(bytes) => step(&mut s, Input::Stdout(&bytes), now),
                Move::Answer(answer) => {
                    prompt = None;
                    step(&mut s, Input::Stdout(&line(answer)), now)
                }
                // A permission still waiting is answered, and a prompt
                // held by an unknown stop reason let go, so turns keep
                // coming.
                Move::Prompt(text) => {
                    let waiting = s.transcript().entries().find_map(|(id, entry)| match entry {
                        Entry::Permission { options, outcome: None, .. } => {
                            Some((id, options.first().map(|option| option.id.clone())))
                        }
                        Entry::User { .. }
                        | Entry::Agent { .. }
                        | Entry::Thought { .. }
                        | Entry::Tool { .. }
                        | Entry::Permission { .. }
                        | Entry::Notice { .. } => None,
                    });
                    let command = match (waiting, s.transcript().slot()) {
                        (Some((entry, option)), _) => Command::Answer { entry, option },
                        (None, Slot::Held { .. }) => Command::Unqueue,
                        (None, Slot::Empty | Slot::Waiting { .. }) => Command::Prompt { text },
                    };
                    step(&mut s, Input::Command(command), now)
                }
            };
            for sent in out.stdin.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
                let sent: Value = brindle_remote_protocol::serde_json::from_slice(sent).expect("a JSON line");
                if sent["method"] == "session/prompt" {
                    prompt = sent["id"].as_u64();
                }
            }
            all_fit(&format!("step {at}"), &out.events);
            for event in &out.events {
                *kinds.entry(event.kind()).or_default() += 1;
            }
            let values: Vec<Value> = out.events.iter().map(Event::to_value).collect();
            assert!(values.is_empty() || pack_transcript(1, values).is_ok(), "step {at}");
            let log = s.transcript();
            assert!(log.encoded_len() <= LOG_LIMIT, "step {at}: {}", log.encoded_len());
            assert!(!matches!(log.agent(), AgentState::Failed { .. }), "step {at}: {:?}", log.agent());
            // The entries this step changed; the others were checked when
            // they last changed.
            for id in out.events.iter().filter_map(touched) {
                if let Some(Entry::Tool { content, locations, input, result, .. }) = log.entry(id) {
                    assert!(content.len() <= MAX_ITEMS, "step {at}: {id}");
                    assert!(content.iter().map(ToolItem::wire_len).sum::<usize>() <= CALL_LIMIT, "step {at}: {id}");
                    for item in content {
                        let mut bounded = item.clone();
                        bounded.bound();
                        assert_eq!(&bounded, item, "step {at}: {id}");
                    }
                    assert!(locations.len() <= MAX_LOCATIONS);
                    assert!(locations.iter().all(|location| location.path.len() <= MAX_PATH));
                    assert!([input, result].into_iter().flatten().all(|raw| raw.len() <= RAW_LIMIT));
                }
            }
            for (terminal, open) in &s.process.book.outputs {
                assert!(terminal.len() <= MAX_TERMINAL_ID);
                assert!(
                    matches!(log.entry(open.entry), Some(Entry::Tool { content, .. })
                        if matches!(content.get(open.item as usize), Some(ToolItem::Output(_)))),
                    "step {at}: {terminal:.16} names no output"
                );
            }
            if at % 40 == 39 {
                all_fit("replay", &log.replay());
                for budget in [1, 16 * 1024, PAGE_BYTES] {
                    all_fit("page", &log.page(budget));
                }
                all_fit("fetch", &log.fetch(log.next_id(), PAGE_BYTES).events);
            }
            if at % 100 == 99 {
                assert!(checkpoint::encode(&s).is_ok(), "step {at}");
            }
        }
        for kind in ["trimmed", "tool_items", "tool_output", "answered", "turn", "queue"] {
            assert!(kinds.get(kind).is_some_and(|n| *n > 0), "no {kind} event in {kinds:?}");
        }
    }
}
