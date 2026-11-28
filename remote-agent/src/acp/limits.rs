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
