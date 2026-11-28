//! Every public item of `brindle-transcript` keeps its path at the crate
//! root (OpenSpec change `agents-module-structure`, design decision 4):
//! this file fails to compile when a re-export goes missing.

#[allow(unused_imports)]
use brindle_transcript::{
    Activity, AgentState, CALL_LIMIT, ContentBudget, Continuation, Conversation, DecodeError,
    DiffLine, ENTRY_FRAMING, Entry, EntryId, Event, Exit, Fetched, FileChange, FileOp, FoldError,
    HUNK_FRAMING, HoldReason, Hunk, ITEM_LIMIT, LINE_FRAMING, Location, LoginKind, LoginMethod,
    MAX_CHOICES, MAX_COMMANDS, MAX_EVENT, MAX_ITEMS, MAX_LOCATIONS, MAX_OPTIONS, MAX_PATH,
    NoticeLevel, OptionCategory, OptionChoice, OptionControl, Output, PAGE_BYTES,
    PermissionKind, PermissionOption, PermissionOutcome, RAW_LIMIT, SIGNAL_RESERVE,
    SessionOption, SlashCommand, Slot, StopReason, TEXT_CHUNK, ToolItem, ToolKind, ToolStatus,
    Transcript, TurnState, Window, chunks, clean, cleaned_len, escaped_len, item_pieces,
};

#[test]
fn every_public_item_keeps_its_path() {}
