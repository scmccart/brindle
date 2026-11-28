use std::fmt;

use brindle_remote_protocol::holder::MAX_CHECKPOINT_LEN;
use brindle_remote_protocol::serde_json::Value;
use brindle_transcript::EntryId;

use crate::acp::dialect::Dialect;
use crate::acp::rpc::BadParts;

/// The four bytes a checkpoint starts with.
pub const MAGIC: [u8; 4] = *b"BACP";

/// The format [`encode`] writes and the newest [`decode`] reads.
pub const FORMAT: u16 = 2;

/// The format before segments, which [`decode`] still reads.
pub(super) const FORMAT_1: u16 = 1;

/// The magic and the format.
pub(super) const PREFIX_LEN: usize = 4 + 2;

/// Format 1's header: the magic, the format and the state's length.
pub(super) const HEADER_LEN: usize = PREFIX_LEN + 4;

/// A format 2 record's kind, sequence number and state length.
pub(super) const RECORD_HEADER_LEN: usize = 1 + 4 + 4;

/// A base record's kind.
pub(super) const BASE: u8 = b'B';

/// A segment record's kind.
pub(super) const SEGMENT: u8 = b'S';

/// [`encode`]'s result would be past [`MAX_CHECKPOINT_LEN`]; `len` is its
/// length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TooLarge {
    pub len: u64,
}

impl fmt::Display for TooLarge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "the agent session's checkpoint is {} bytes, over the limit of {} MiB",
            self.len,
            MAX_CHECKPOINT_LEN / (1024 * 1024)
        )
    }
}

/// Why a checkpoint can't be read; its text follows [`NOT_RESTORED`] in
/// the notice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Unreadable {
    /// Shorter than its header.
    Short { len: usize },
    /// It doesn't start with [`MAGIC`].
    NotACheckpoint,
    /// A format this core doesn't read: a newer core wrote it, or it is 0.
    Format { format: u16 },
    /// The state's length runs past the end.
    Truncated,
    /// The state isn't a JSON object.
    NotJson { error: String },
    /// A field is missing or has the wrong form.
    Field { name: &'static str },
    /// The conversation speaks another ACP version.
    Acp { version: u64 },
    /// A logged event doesn't decode or doesn't fold.
    Log { error: String },
    /// The request bookkeeping doesn't hold together.
    Rpc { parts: BadParts },
    /// The state names an entry the log doesn't have, or has of another
    /// kind; or the log has a pending permission request the state's
    /// `permissions` doesn't await.
    Entry { name: &'static str, entry: EntryId },
    /// Partial line bytes come with a partial that isn't being collected.
    StrayBytes { len: usize },
    /// A format 2 checkpoint whose first record isn't a base.
    NoBase,
    /// A format 2 record of a kind other than a base first and segments
    /// after it.
    RecordKind { kind: u8 },
    /// A format 2 record whose sequence number isn't the next one: a
    /// segment was lost.
    Sequence { expected: u32, found: u32 },
}

impl fmt::Display for Unreadable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Short { len } => write!(f, "its checkpoint is only {len} bytes"),
            Self::NotACheckpoint => f.write_str("its checkpoint isn't an agent session's"),
            Self::Format { format } => write!(
                f,
                "its checkpoint is in format {format}, and this terminal server reads formats {FORMAT_1} and {FORMAT}"
            ),
            Self::Truncated => f.write_str("its checkpoint is cut short"),
            Self::NotJson { error } => write!(f, "its checkpoint's state isn't JSON: {error}"),
            Self::Field { name } => write!(f, "its checkpoint's {name} is missing or malformed"),
            Self::Acp { version } => write!(
                f,
                "its conversation speaks ACP version {version}, and this terminal server speaks {}",
                Dialect::newest().version()
            ),
            Self::Log { error } => write!(f, "its checkpoint's log doesn't hold together: {error}"),
            Self::Rpc { parts } => {
                let why = match parts {
                    BadParts::IdNotBelowNext => "a request id at or past the next one",
                    BadParts::TooManyPending => "more waiting permission requests than allowed",
                };
                write!(f, "its checkpoint's requests don't hold together: {why}")
            }
            Self::Entry { name, entry } => {
                write!(
                    f,
                    "its checkpoint's {name} and its log disagree about entry {entry}"
                )
            }
            Self::StrayBytes { len } => {
                write!(
                    f,
                    "its checkpoint carries {len} bytes of a line it isn't collecting"
                )
            }
            Self::NoBase => f.write_str("its checkpoint doesn't start with its base"),
            Self::RecordKind { kind } => {
                write!(f, "its checkpoint has a record of unknown kind {kind:#04x}")
            }
            Self::Sequence { expected, found } => write!(
                f,
                "its checkpoint's record {found} came where record {expected} was due, so a part of it is missing"
            ),
        }
    }
}

/// Appends a format 2 record to `out`, or refuses when `out` would pass
/// `limit`.
pub(super) fn push_record(
    out: &mut Vec<u8>,
    kind: u8,
    seq: u32,
    state: &Value,
    partial: &[u8],
    limit: u64,
) -> Result<(), TooLarge> {
    let state = state.to_string();
    let len = out.len() as u64
        + RECORD_HEADER_LEN as u64
        + state.len() as u64
        + 4
        + partial.len() as u64;
    let state_len = u32::try_from(state.len()).map_err(|_| TooLarge { len })?;
    let partial_len = u32::try_from(partial.len()).map_err(|_| TooLarge { len })?;
    if len > limit {
        return Err(TooLarge { len });
    }
    out.reserve(RECORD_HEADER_LEN + state.len() + 4 + partial.len());
    out.push(kind);
    out.extend_from_slice(&seq.to_be_bytes());
    out.extend_from_slice(&state_len.to_be_bytes());
    out.extend_from_slice(state.as_bytes());
    out.extend_from_slice(&partial_len.to_be_bytes());
    out.extend_from_slice(partial);
    Ok(())
}

/// One format 2 record.
pub(super) struct Record<'a> {
    pub(super) kind: u8,
    pub(super) seq: u32,
    pub(super) state: &'a [u8],
    pub(super) partial: &'a [u8],
}

/// The next record of `rest`, taken off it; `None` at its end.
pub(super) fn next_record<'a>(rest: &mut &'a [u8]) -> Result<Option<Record<'a>>, Unreadable> {
    if rest.is_empty() {
        return Ok(None);
    }
    let (kind, bytes) = rest.split_first().ok_or(Unreadable::Truncated)?;
    let (seq, bytes) = take_u32(bytes)?;
    let (state_len, bytes) = take_u32(bytes)?;
    let (state, bytes) = take(bytes, state_len)?;
    let (partial_len, bytes) = take_u32(bytes)?;
    let (partial, bytes) = take(bytes, partial_len)?;
    *rest = bytes;
    Ok(Some(Record {
        kind: *kind,
        seq,
        state,
        partial,
    }))
}

fn take_u32(bytes: &[u8]) -> Result<(u32, &[u8]), Unreadable> {
    let (number, rest) = bytes
        .split_first_chunk::<4>()
        .ok_or(Unreadable::Truncated)?;
    Ok((u32::from_be_bytes(*number), rest))
}

fn take(bytes: &[u8], len: u32) -> Result<(&[u8], &[u8]), Unreadable> {
    let len = usize::try_from(len).map_err(|_| Unreadable::Truncated)?;
    if len > bytes.len() {
        return Err(Unreadable::Truncated);
    }
    Ok(bytes.split_at(len))
}
