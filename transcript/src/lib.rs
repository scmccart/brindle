//! The agent transcript that Brindle and its terminal server share: the
//! events the core sends over client protocol 4 and the fold both sides
//! apply them with. Pure as a crate (fcis R4): it takes values and returns
//! values, and does no IO and reads no clock.
//!
//! - **Events** ([`Event`]) are Brindle's own, never ACP's: the core's
//!   reducer turns the agent's messages into them, applies each to its log
//!   and sends it; a client applies the same events to its copy. They travel
//!   as JSON objects ([`Event::to_value`], [`Event::from_value`]) inside
//!   client protocol 4's `TRANSCRIPT`, which `brindle-remote-protocol`
//!   carries without reading.
//! - **The fold** ([`Transcript::apply`]) is the one way a log changes. An
//!   event it can't apply is a [`FoldError`], never a panic. A log rebuilt
//!   from [`Transcript::replay`] equals the log it came from.
//! - **Pages** (OpenSpec change `agents-log-paging`): an attach sends
//!   [`Transcript::page`], the newest whole entries within a byte budget
//!   behind an [`Event::Window`], and a client fetches older ones with
//!   [`Transcript::fetch`] and folds them in front with
//!   [`Transcript::prepend`]. The laws, pinned by property tests: a page
//!   folds to the log's suffix from its window, live events keep the copy
//!   equal to the server's suffix, and fetched pages folded in front until
//!   nothing more remains give the whole log.
//! - **Forms are frozen per client protocol version** (`process R5`): the
//!   event kinds and the values of every closed enum inside an event are
//!   protocol 4's, pinned by `tests/fixtures/v4/`. Protocol 4's forms
//!   freeze with the release that first ships it; until then the `agents-*`
//!   changes amend them in place. Once shipped, an event may only gain
//!   optional fields, defaulted and omitted when unset;
//!   [`Event::from_value`] ignores fields it doesn't know, and an unknown
//!   kind or value is a [`DecodeError`].
//!
//! The encoding: every enum that carries data is an object tagged by a
//! field of its own (`event`, `type` for an [`Entry`], `turn`, `agent`,
//! `slot`, `hold`, `reason`, `outcome`), and every enum that doesn't is a
//! `snake_case` string. An optional field is left out when unset.

// railway R3, types R2: no `_ =>` on one of Brindle's own enums. railway R5,
// R9: a decoder never panics on bytes from outside, and non-test code has no
// `unwrap` or `expect`. Non-test code only.
#![cfg_attr(not(test), warn(clippy::wildcard_enum_match_arm))]
#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]

use std::collections::VecDeque;

mod event;
mod fold;
mod output;
mod page;

#[cfg(test)]
mod fuzz;
#[cfg(test)]
mod harness;

pub use event::{
    AgentState, CALL_LIMIT, ContentBudget, Continuation, Conversation, DecodeError, DiffLine,
    ENTRY_FRAMING, Entry, EntryId, Event, Exit, FileChange, FileOp, HUNK_FRAMING, HoldReason, Hunk,
    ITEM_LIMIT, LINE_FRAMING, Location, LoginKind, LoginMethod, MAX_CHOICES, MAX_COMMANDS,
    MAX_EVENT, MAX_ITEMS, MAX_LOCATIONS, MAX_OPTIONS, MAX_PATH, NoticeLevel, OptionCategory,
    OptionChoice, OptionControl, PermissionKind, PermissionOption, PermissionOutcome, RAW_LIMIT,
    SIGNAL_RESERVE, SessionOption, SlashCommand, Slot, StopReason, TEXT_CHUNK, ToolItem, ToolKind,
    ToolStatus, TurnState, clean, cleaned_len, item_pieces,
};
pub use fold::{Activity, FoldError, chunks, escaped_len};
pub use output::{Output, OutputText};
pub use page::{Fetched, PAGE_BYTES, ReplayRef, Window};

/// An entry the log keeps, with its measure: its encoded length and
/// [`ENTRY_FRAMING`].
#[derive(Debug, Clone, PartialEq, Eq)]
struct Kept {
    id: EntryId,
    entry: Entry,
    size: usize,
}

/// The entries trimmed so far.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Trim {
    through: EntryId,
    dropped: u64,
}

/// An agent session's log: its entries in id order, the queue slot, the
/// turn and the agent's state. It changes only through [`Transcript::apply`]
/// and, on a client's copy, [`Transcript::prepend`], so the core's log and
/// a client's copy fold the same events the same way.
///
/// A client's copy may hold only a suffix of the log (OpenSpec change
/// `agents-log-paging`, design decision 4): a page's [`Event::Window`]
/// sets a [`Window`], events for entries below it are ignored (a later
/// fetch brings them current), a `Trimmed` below it lowers its count, and
/// fetched pages folded in front with [`Transcript::prepend`] lower it
/// until it goes. The server's log never has a window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transcript {
    entries: VecDeque<Kept>,
    next: EntryId,
    slot: Slot,
    turn: TurnState,
    agent: AgentState,
    /// Set by [`Event::Conversation`]; `None` until the agent gave one.
    conversation: Option<Conversation>,
    trimmed: Option<Trim>,
    /// The kept entries' encoded length.
    size: usize,
    /// Set by [`Event::Window`]; `None` when every kept entry is held.
    window: Option<Window>,
    /// Set by [`Event::Commands`]: the agent's latest list.
    commands: Vec<SlashCommand>,
    /// Set by [`Event::Options`]: the agent's latest list.
    options: Vec<SessionOption>,
}

impl Default for Transcript {
    fn default() -> Self {
        Self {
            entries: VecDeque::new(),
            next: EntryId::FIRST,
            slot: Slot::Empty,
            turn: TurnState::Idle,
            agent: AgentState::Starting,
            conversation: None,
            trimmed: None,
            size: 0,
            window: None,
            commands: Vec::new(),
            options: Vec::new(),
        }
    }
}
