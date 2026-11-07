//! The terminal server's core ↔ holder protocol, H1.
//!
//! A holder owns many sessions' PTYs and parses nothing; the core drives it.
//! H1 is kept minimal so holders rarely need to change. Frames use the
//! crate's header (`type: u8`, `len: u32 BE`). Binary messages start with a
//! `u32` BE session id. JSON messages are one object each; unknown fields are
//! ignored and missing optional fields take their defaults. A new message
//! type needs a new version.
//!
//! | type | name               | direction     | payload                                                          |
//! |-----:|--------------------|---------------|------------------------------------------------------------------|
//! |    1 | `HELLO`            | core → holder | `min, max, core_build, instance`                                 |
//! |    2 | `SPAWN`            | core → holder | `session, command, args, cwd, env, cols, rows, cell_w, cell_h, meta` |
//! |    3 | `INPUT`            | core → holder | session, bytes                                                   |
//! |    4 | `RESIZE`           | core → holder | `session, cols, rows, cell_w, cell_h`                            |
//! |    5 | `HANGUP`           | core → holder | `session`                                                        |
//! |    6 | `CHECKPOINT_BEGIN` | both          | session, `len: u64 BE`                                           |
//! |    7 | `CHECKPOINT_CHUNK` | both          | session, bytes                                                   |
//! |    8 | `CHECKPOINT_END`   | both          | session; from the core, then `covers: u64 BE`                    |
//! |    9 | `DRAIN`            | core → holder | `{}`                                                             |
//! |   10 | `GOODBYE`          | core → holder | `handover`                                                       |
//! |   11 | `REPARENT`         | core → holder | `session, hwnd` (a Linux holder answers `ERROR`)                 |
//! |   12 | `PAUSE`            | core → holder | `session`                                                        |
//! |   13 | `RESUME`           | core → holder | `session`                                                        |
//! |   32 | `WELCOME`          | holder → core | `protocol, build, pid, previous, sessions: [{session, pid, cols, rows, exit, meta, truncated}]` |
//! |   33 | `OUTPUT`           | holder → core | session, bytes                                                   |
//! |   34 | `SPAWNED`          | holder → core | `session, pid`                                                   |
//! |   35 | `SPAWN_FAILED`     | holder → core | `session, message`                                               |
//! |   36 | `EXITED`           | holder → core | `session, code?`                                                 |
//! |   37 | `ERROR`            | holder → core | `session?, message`                                              |
//! |   38 | `TAIL_BEGIN`       | holder → core | session, `len: u64 BE`                                           |
//! |   39 | `TAIL_CHUNK`       | holder → core | session, bytes                                                   |
//! |   40 | `TAIL_END`         | holder → core | session                                                          |
//!
//! **Checkpoints and tails are chunked.** A checkpoint (the core's replay
//! stream for a session) can pass the 1 MiB frame limit, and a full tail
//! plus its session id does too. Each is sent as `*_BEGIN` with the total
//! length, any number of `*_CHUNK`s whose lengths add up to it, and `*_END`.
//! Chunks of one blob may interleave with other sessions' messages, but not
//! with another blob of the same kind for the same session. The holder
//! replaces its stored checkpoint only on `CHECKPOINT_END`; a connection
//! lost mid-blob leaves the previous one in place. [`split`] cuts a blob
//! into messages and [`Assembler`] rebuilds it.
//!
//! **What a checkpoint covers.** Output the holder read may still be on its
//! way to the core when the core renders a checkpoint, so the core's
//! `CHECKPOINT_END` says how much of the tail the checkpoint covers
//! (`covers`: bytes read since the previous checkpoint, or for a core that
//! adopted the session, since the start of the tail it got). The holder
//! drops that much of the tail and keeps the rest, so the next core's
//! rebuild from the checkpoint and the tail is exact.
//!
//! **Flow control.** `PAUSE` makes the holder stop reading a session's PTY,
//! so its program blocks, and `RESUME` reads it again. The core pauses the
//! sessions of a client that isn't keeping up, so one stalled client
//! doesn't hold up the connection's other sessions. A holder forgets the
//! pauses when its core leaves. An unknown session is ignored, since it may
//! just have ended.
//!
//! **Adopting.** `WELCOME` is followed, for each session it lists, by that
//! session's checkpoint and then its tail, each possibly empty (`len` 0).
//!
//! **`meta`** is an opaque JSON value the core owns (policy, owner token,
//! scrollback, command and directory for `list`); the holder stores it and
//! returns it in `WELCOME`. It is a [`serde_json::Value`] rather than raw
//! bytes: it stays readable in transcripts, a holder rejects a `SPAWN` whose
//! meta isn't JSON instead of handing garbage to the next core, and it needs
//! no base64 or `raw_value` feature. serde_json may sort an object's keys on
//! the way back (unless `preserve_order` is enabled in the build), which a
//! JSON consumer can't observe; the core writes them sorted.

use std::collections::{BTreeMap, HashMap};
use std::io;

use serde::{Deserialize, Serialize};

use crate::Frame;
use crate::wire::{self, SessionId};

pub use crate::version::{NegotiationError, VersionRange, negotiate};

/// The H1 versions this build speaks.
pub const PROTOCOLS: VersionRange = VersionRange::new(1, 1);

pub const MSG_HELLO: u8 = 1;
pub const MSG_SPAWN: u8 = 2;
pub const MSG_INPUT: u8 = 3;
pub const MSG_RESIZE: u8 = 4;
pub const MSG_HANGUP: u8 = 5;
pub const MSG_CHECKPOINT_BEGIN: u8 = 6;
pub const MSG_CHECKPOINT_CHUNK: u8 = 7;
pub const MSG_CHECKPOINT_END: u8 = 8;
pub const MSG_DRAIN: u8 = 9;
pub const MSG_GOODBYE: u8 = 10;
pub const MSG_REPARENT: u8 = 11;
pub const MSG_PAUSE: u8 = 12;
pub const MSG_RESUME: u8 = 13;
pub const MSG_WELCOME: u8 = 32;
pub const MSG_OUTPUT: u8 = 33;
pub const MSG_SPAWNED: u8 = 34;
pub const MSG_SPAWN_FAILED: u8 = 35;
pub const MSG_EXITED: u8 = 36;
pub const MSG_ERROR: u8 = 37;
pub const MSG_TAIL_BEGIN: u8 = 38;
pub const MSG_TAIL_CHUNK: u8 = 39;
pub const MSG_TAIL_END: u8 = 40;

/// How much of a blob [`split`] puts in each chunk by default. Smaller than
/// the frame limit so one session's checkpoint doesn't hold up the others.
pub const CHUNK_LEN: usize = 64 * 1024;
/// The largest chunk a frame can carry.
pub const MAX_CHUNK_LEN: usize = crate::MAX_FRAME_PAYLOAD_SIZE - wire::SESSION_PREFIX_LEN;
/// The most raw output a holder keeps per session between checkpoints; past
/// it, the oldest bytes go and the session is marked `truncated`.
pub const TAIL_LIMIT: u64 = 1024 * 1024;
/// The largest checkpoint either side accepts.
pub const MAX_CHECKPOINT_LEN: u64 = 64 * 1024 * 1024;

/// Bytes for one session: `INPUT`, `OUTPUT`, or a blob's chunk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Data {
    pub session: SessionId,
    pub bytes: Vec<u8>,
}

/// The start of a chunked blob: `CHECKPOINT_BEGIN` or `TAIL_BEGIN`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlobBegin {
    pub session: SessionId,
    /// The blob's total length.
    pub len: u64,
}

/// The end of a chunked blob: `TAIL_END`, or the holder's `CHECKPOINT_END`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlobEnd {
    pub session: SessionId,
}

/// The core's `CHECKPOINT_END`. The checkpoint covers the first `covers`
/// bytes of output the holder read since the previous checkpoint (for a
/// core that adopted the session, since the start of the tail it got).
/// The holder drops those from the tail and keeps the rest, which the core
/// hadn't fed when it rendered the checkpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CheckpointEnd {
    pub session: SessionId,
    pub covers: u64,
}

impl CheckpointEnd {
    pub fn blob_end(self) -> BlobEnd {
        BlobEnd {
            session: self.session,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub min: u32,
    pub max: u32,
    #[serde(default)]
    pub core_build: String,
    /// Distinguishes core processes. A `HELLO` takes the holder over from
    /// whichever core was connected before it.
    #[serde(default)]
    pub instance: u64,
}

impl Hello {
    pub fn range(&self) -> VersionRange {
        VersionRange::new(self.min, self.max)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spawn {
    pub session: SessionId,
    /// `None` runs the user's login shell.
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    /// The program's complete environment.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub cols: u16,
    pub rows: u16,
    #[serde(default)]
    pub cell_w: u16,
    #[serde(default)]
    pub cell_h: u16,
    #[serde(default)]
    pub meta: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resize {
    pub session: SessionId,
    pub cols: u16,
    pub rows: u16,
    #[serde(default)]
    pub cell_w: u16,
    #[serde(default)]
    pub cell_h: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRef {
    pub session: SessionId,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Drain {}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Goodbye {
    /// The core is stepping down for an upgrade, not crashing or stopping:
    /// the holder reports `previous: handover` to the next core.
    #[serde(default)]
    pub handover: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reparent {
    pub session: SessionId,
    pub hwnd: u64,
}

/// How the holder's previous core connection ended.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Previous {
    /// No core was connected before (a new holder).
    #[default]
    None,
    /// The previous core sent `GOODBYE { handover: true }`.
    Handover,
    /// The previous connection ended without a handover.
    Crash,
}

/// How a held session's program ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExitStatus {
    /// `None` when it was ended by a signal.
    #[serde(default)]
    pub code: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeldSession {
    pub session: SessionId,
    pub pid: u32,
    pub cols: u16,
    pub rows: u16,
    /// `None` while the program runs; set when it exited and no core has
    /// been told yet.
    #[serde(default)]
    pub exit: Option<ExitStatus>,
    #[serde(default)]
    pub meta: serde_json::Value,
    /// The tail passed [`TAIL_LIMIT`] since the last checkpoint, so only its
    /// newest bytes are kept.
    #[serde(default)]
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Welcome {
    pub protocol: u32,
    #[serde(default)]
    pub build: String,
    pub pid: u32,
    #[serde(default)]
    pub previous: Previous,
    #[serde(default)]
    pub sessions: Vec<HeldSession>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spawned {
    pub session: SessionId,
    pub pid: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnFailed {
    pub session: SessionId,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Exited {
    pub session: SessionId,
    /// `None` when the program was ended by a signal.
    #[serde(default)]
    pub code: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorMessage {
    #[serde(default)]
    pub session: Option<SessionId>,
    pub message: String,
}

/// Messages from the core to a holder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToHolder {
    Hello(Hello),
    Spawn(Spawn),
    Input(Data),
    Resize(Resize),
    Hangup(SessionRef),
    CheckpointBegin(BlobBegin),
    CheckpointChunk(Data),
    CheckpointEnd(CheckpointEnd),
    Drain(Drain),
    Goodbye(Goodbye),
    Reparent(Reparent),
    /// Stop reading the session's PTY until `RESUME`.
    Pause(SessionRef),
    Resume(SessionRef),
}

impl ToHolder {
    pub fn encode(&self) -> io::Result<Frame> {
        match self {
            Self::Hello(m) => wire::json(MSG_HELLO, m),
            Self::Spawn(m) => wire::json(MSG_SPAWN, m),
            Self::Input(m) => encode_data(MSG_INPUT, m),
            Self::Resize(m) => wire::json(MSG_RESIZE, m),
            Self::Hangup(m) => wire::json(MSG_HANGUP, m),
            Self::CheckpointBegin(m) => encode_begin(MSG_CHECKPOINT_BEGIN, m),
            Self::CheckpointChunk(m) => encode_data(MSG_CHECKPOINT_CHUNK, m),
            Self::CheckpointEnd(m) => {
                wire::session_bytes(MSG_CHECKPOINT_END, m.session, &m.covers.to_be_bytes())
            }
            Self::Drain(m) => wire::json(MSG_DRAIN, m),
            Self::Goodbye(m) => wire::json(MSG_GOODBYE, m),
            Self::Reparent(m) => wire::json(MSG_REPARENT, m),
            Self::Pause(m) => wire::json(MSG_PAUSE, m),
            Self::Resume(m) => wire::json(MSG_RESUME, m),
        }
    }

    pub fn decode(frame: &Frame) -> io::Result<Self> {
        Ok(match frame.msg_type {
            MSG_HELLO => Self::Hello(wire::from_json("HELLO", frame)?),
            MSG_SPAWN => Self::Spawn(wire::from_json("SPAWN", frame)?),
            MSG_INPUT => Self::Input(decode_data("INPUT", frame)?),
            MSG_RESIZE => Self::Resize(wire::from_json("RESIZE", frame)?),
            MSG_HANGUP => Self::Hangup(wire::from_json("HANGUP", frame)?),
            MSG_CHECKPOINT_BEGIN => Self::CheckpointBegin(decode_begin("CHECKPOINT_BEGIN", frame)?),
            MSG_CHECKPOINT_CHUNK => Self::CheckpointChunk(decode_data("CHECKPOINT_CHUNK", frame)?),
            MSG_CHECKPOINT_END => {
                let (session, covers) = wire::session_fixed::<8>("CHECKPOINT_END", frame)?;
                Self::CheckpointEnd(CheckpointEnd {
                    session,
                    covers: u64::from_be_bytes(covers),
                })
            }
            MSG_DRAIN => Self::Drain(wire::from_json("DRAIN", frame)?),
            MSG_GOODBYE => Self::Goodbye(wire::from_json("GOODBYE", frame)?),
            MSG_REPARENT => Self::Reparent(wire::from_json("REPARENT", frame)?),
            MSG_PAUSE => Self::Pause(wire::from_json("PAUSE", frame)?),
            MSG_RESUME => Self::Resume(wire::from_json("RESUME", frame)?),
            other => return Err(wire::unknown_type("core-to-holder", other)),
        })
    }

    /// A checkpoint covering `covers` bytes of the tail, as
    /// `CHECKPOINT_BEGIN`, chunks of [`CHUNK_LEN`] and `CHECKPOINT_END`.
    pub fn checkpoint(session: SessionId, bytes: &[u8], covers: u64) -> Vec<Self> {
        let (begin, chunks, end) = split(session, bytes, CHUNK_LEN);
        std::iter::once(Self::CheckpointBegin(begin))
            .chain(chunks.into_iter().map(Self::CheckpointChunk))
            .chain(std::iter::once(Self::CheckpointEnd(CheckpointEnd {
                session: end.session,
                covers,
            })))
            .collect()
    }
}

/// Messages from a holder to the core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToCore {
    Welcome(Welcome),
    Output(Data),
    Spawned(Spawned),
    SpawnFailed(SpawnFailed),
    Exited(Exited),
    Error(ErrorMessage),
    CheckpointBegin(BlobBegin),
    CheckpointChunk(Data),
    CheckpointEnd(BlobEnd),
    TailBegin(BlobBegin),
    TailChunk(Data),
    TailEnd(BlobEnd),
}

impl ToCore {
    pub fn encode(&self) -> io::Result<Frame> {
        match self {
            Self::Welcome(m) => wire::json(MSG_WELCOME, m),
            Self::Output(m) => encode_data(MSG_OUTPUT, m),
            Self::Spawned(m) => wire::json(MSG_SPAWNED, m),
            Self::SpawnFailed(m) => wire::json(MSG_SPAWN_FAILED, m),
            Self::Exited(m) => wire::json(MSG_EXITED, m),
            Self::Error(m) => wire::json(MSG_ERROR, m),
            Self::CheckpointBegin(m) => encode_begin(MSG_CHECKPOINT_BEGIN, m),
            Self::CheckpointChunk(m) => encode_data(MSG_CHECKPOINT_CHUNK, m),
            Self::CheckpointEnd(m) => encode_end(MSG_CHECKPOINT_END, m),
            Self::TailBegin(m) => encode_begin(MSG_TAIL_BEGIN, m),
            Self::TailChunk(m) => encode_data(MSG_TAIL_CHUNK, m),
            Self::TailEnd(m) => encode_end(MSG_TAIL_END, m),
        }
    }

    pub fn decode(frame: &Frame) -> io::Result<Self> {
        Ok(match frame.msg_type {
            MSG_WELCOME => Self::Welcome(wire::from_json("WELCOME", frame)?),
            MSG_OUTPUT => Self::Output(decode_data("OUTPUT", frame)?),
            MSG_SPAWNED => Self::Spawned(wire::from_json("SPAWNED", frame)?),
            MSG_SPAWN_FAILED => Self::SpawnFailed(wire::from_json("SPAWN_FAILED", frame)?),
            MSG_EXITED => Self::Exited(wire::from_json("EXITED", frame)?),
            MSG_ERROR => Self::Error(wire::from_json("ERROR", frame)?),
            MSG_CHECKPOINT_BEGIN => Self::CheckpointBegin(decode_begin("CHECKPOINT_BEGIN", frame)?),
            MSG_CHECKPOINT_CHUNK => Self::CheckpointChunk(decode_data("CHECKPOINT_CHUNK", frame)?),
            MSG_CHECKPOINT_END => Self::CheckpointEnd(decode_end("CHECKPOINT_END", frame)?),
            MSG_TAIL_BEGIN => Self::TailBegin(decode_begin("TAIL_BEGIN", frame)?),
            MSG_TAIL_CHUNK => Self::TailChunk(decode_data("TAIL_CHUNK", frame)?),
            MSG_TAIL_END => Self::TailEnd(decode_end("TAIL_END", frame)?),
            other => return Err(wire::unknown_type("holder-to-core", other)),
        })
    }

    /// A stored checkpoint, sent after `WELCOME`, in chunks of [`CHUNK_LEN`].
    pub fn checkpoint(session: SessionId, bytes: &[u8]) -> Vec<Self> {
        let (begin, chunks, end) = split(session, bytes, CHUNK_LEN);
        std::iter::once(Self::CheckpointBegin(begin))
            .chain(chunks.into_iter().map(Self::CheckpointChunk))
            .chain(std::iter::once(Self::CheckpointEnd(end)))
            .collect()
    }

    /// A session's tail, sent after its checkpoint, in chunks of [`CHUNK_LEN`].
    pub fn tail(session: SessionId, bytes: &[u8]) -> Vec<Self> {
        let (begin, chunks, end) = split(session, bytes, CHUNK_LEN);
        std::iter::once(Self::TailBegin(begin))
            .chain(chunks.into_iter().map(Self::TailChunk))
            .chain(std::iter::once(Self::TailEnd(end)))
            .collect()
    }
}

fn encode_data(msg_type: u8, data: &Data) -> io::Result<Frame> {
    wire::session_bytes(msg_type, data.session, &data.bytes)
}

fn decode_data(name: &str, frame: &Frame) -> io::Result<Data> {
    let (session, bytes) = wire::split_session(name, frame)?;
    Ok(Data {
        session,
        bytes: bytes.to_vec(),
    })
}

fn encode_begin(msg_type: u8, begin: &BlobBegin) -> io::Result<Frame> {
    wire::session_bytes(msg_type, begin.session, &begin.len.to_be_bytes())
}

fn decode_begin(name: &str, frame: &Frame) -> io::Result<BlobBegin> {
    let (session, len) = wire::session_fixed::<8>(name, frame)?;
    Ok(BlobBegin {
        session,
        len: u64::from_be_bytes(len),
    })
}

fn encode_end(msg_type: u8, end: &BlobEnd) -> io::Result<Frame> {
    wire::session_bytes(msg_type, end.session, &[])
}

fn decode_end(name: &str, frame: &Frame) -> io::Result<BlobEnd> {
    let (session, []) = wire::session_fixed::<0>(name, frame)?;
    Ok(BlobEnd { session })
}

/// Cuts a blob into its begin marker, chunks of at most `chunk_len` bytes
/// (clamped to `1..=`[`MAX_CHUNK_LEN`]), and end marker. An empty blob has
/// no chunks.
pub fn split(
    session: SessionId,
    bytes: &[u8],
    chunk_len: usize,
) -> (BlobBegin, Vec<Data>, BlobEnd) {
    let chunk_len = chunk_len.clamp(1, MAX_CHUNK_LEN);
    let chunks = bytes
        .chunks(chunk_len)
        .map(|chunk| Data {
            session,
            bytes: chunk.to_vec(),
        })
        .collect();
    (
        BlobBegin {
            session,
            len: bytes.len() as u64,
        },
        chunks,
        BlobEnd { session },
    )
}

/// Rebuilds chunked blobs of one kind (checkpoints, or tails), per session.
/// Every violation of the begin/chunk/end sequence is an `InvalidData`
/// error, after which that session's partial blob is dropped.
#[derive(Debug)]
pub struct Assembler {
    limit: u64,
    open: HashMap<SessionId, Partial>,
}

#[derive(Debug)]
struct Partial {
    len: u64,
    bytes: Vec<u8>,
}

impl Assembler {
    /// An assembler that refuses blobs longer than `limit`, such as
    /// [`MAX_CHECKPOINT_LEN`] or [`TAIL_LIMIT`].
    pub fn new(limit: u64) -> Self {
        Self {
            limit,
            open: HashMap::new(),
        }
    }

    pub fn begin(&mut self, begin: BlobBegin) -> io::Result<()> {
        if self.open.remove(&begin.session).is_some() {
            return Err(wire::invalid(format!(
                "session {} began a blob before ending the last one",
                begin.session
            )));
        }
        if begin.len > self.limit {
            return Err(wire::invalid(format!(
                "session {}'s blob is {} bytes; the limit is {}",
                begin.session, begin.len, self.limit
            )));
        }
        // Don't trust the announced length for the allocation up front.
        let capacity = begin.len.min(MAX_CHUNK_LEN as u64) as usize;
        self.open.insert(
            begin.session,
            Partial {
                len: begin.len,
                bytes: Vec::with_capacity(capacity),
            },
        );
        Ok(())
    }

    pub fn chunk(&mut self, chunk: &Data) -> io::Result<()> {
        let Some(partial) = self.open.get_mut(&chunk.session) else {
            return Err(wire::invalid(format!(
                "session {} sent a chunk outside a blob",
                chunk.session
            )));
        };
        if partial.bytes.len() as u64 + chunk.bytes.len() as u64 > partial.len {
            let len = partial.len;
            self.open.remove(&chunk.session);
            return Err(wire::invalid(format!(
                "session {}'s chunks pass the blob's {len} bytes",
                chunk.session
            )));
        }
        partial.bytes.extend_from_slice(&chunk.bytes);
        Ok(())
    }

    pub fn end(&mut self, end: BlobEnd) -> io::Result<Vec<u8>> {
        let Some(partial) = self.open.remove(&end.session) else {
            return Err(wire::invalid(format!(
                "session {} ended a blob it never began",
                end.session
            )));
        };
        if partial.bytes.len() as u64 != partial.len {
            return Err(wire::invalid(format!(
                "session {}'s blob ended after {} of {} bytes",
                end.session,
                partial.bytes.len(),
                partial.len
            )));
        }
        Ok(partial.bytes)
    }

    /// Drops a session's partial blob, for a session that ended or a
    /// connection that was lost.
    pub fn abandon(&mut self, session: SessionId) {
        self.open.remove(&session);
    }

    pub fn is_open(&self, session: SessionId) -> bool {
        self.open.contains_key(&session)
    }
}

/// What the core keeps in a session's `meta`. The holder never reads it, but
/// every later core does when it adopts the session, so it is defined here
/// with the protocols. Fields are additive with defaults, like the JSON
/// messages', and declared in sorted order, which [`SessionMeta::to_value`]
/// keeps whether or not serde_json preserves insertion order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionMeta {
    /// The program and its arguments, for `list`; empty for the login shell.
    pub command: String,
    pub cwd: Option<String>,
    /// The owning client's `client_token`.
    pub owner: String,
    pub policy: crate::client::Policy,
    /// The emulator's scrollback limit, in lines.
    pub scrollback: u32,
}

impl SessionMeta {
    pub fn to_value(&self) -> serde_json::Value {
        serde_json::to_value(self).unwrap_or_default()
    }

    /// Reads a held session's meta. Anything that isn't an object of the
    /// expected fields gives the defaults for what's missing or malformed.
    pub fn from_value(value: &serde_json::Value) -> Self {
        let Some(object) = value.as_object() else {
            return Self::default();
        };
        fn parse<T: serde::de::DeserializeOwned + Default>(
            object: &serde_json::Map<String, serde_json::Value>,
            name: &str,
        ) -> T {
            object
                .get(name)
                .and_then(|value| serde_json::from_value(value.clone()).ok())
                .unwrap_or_default()
        }
        Self {
            command: parse(object, "command"),
            cwd: parse(object, "cwd"),
            owner: parse(object, "owner"),
            policy: parse(object, "policy"),
            scrollback: parse(object, "scrollback"),
        }
    }
}

/// A holder's answer to a `HELLO`: the version both speak, or the `ERROR` to
/// send before closing the connection.
pub fn answer_hello(hello: &Hello, supported: VersionRange) -> Result<u32, ErrorMessage> {
    negotiate(hello.range(), supported).map_err(|error| ErrorMessage {
        session: None,
        message: error.to_string(),
    })
}

#[cfg(test)]
pub(crate) mod samples {
    use serde_json::json;

    use super::*;

    pub fn meta() -> serde_json::Value {
        json!({
            "policy": "kill",
            "owner": "4f1c9a",
            "scrollback": 10000,
            "command": "bash",
            "cwd": "/home/u",
        })
    }

    pub fn to_holder() -> Vec<ToHolder> {
        vec![
            ToHolder::Hello(Hello {
                min: 1,
                max: 1,
                core_build: "abc123".into(),
                instance: 77,
            }),
            ToHolder::Spawn(Spawn {
                session: 1,
                command: Some("/bin/bash".into()),
                args: vec!["-l".into()],
                cwd: Some("/home/u".into()),
                env: BTreeMap::from([("TERM".into(), "xterm-256color".into())]),
                cols: 80,
                rows: 24,
                cell_w: 9,
                cell_h: 18,
                meta: meta(),
            }),
            ToHolder::Input(Data {
                session: 1,
                bytes: b"ls\r".to_vec(),
            }),
            ToHolder::Resize(Resize {
                session: 1,
                cols: 100,
                rows: 30,
                cell_w: 9,
                cell_h: 18,
            }),
            ToHolder::Hangup(SessionRef { session: 1 }),
            ToHolder::CheckpointBegin(BlobBegin { session: 1, len: 3 }),
            ToHolder::CheckpointChunk(Data {
                session: 1,
                bytes: b"\x1bc!".to_vec(),
            }),
            ToHolder::CheckpointEnd(CheckpointEnd {
                session: 1,
                covers: 0x1_0000_0002,
            }),
            ToHolder::Drain(Drain {}),
            ToHolder::Goodbye(Goodbye { handover: true }),
            ToHolder::Reparent(Reparent {
                session: 1,
                hwnd: 0x0001_0a2c,
            }),
            ToHolder::Pause(SessionRef { session: 1 }),
            ToHolder::Resume(SessionRef { session: 1 }),
        ]
    }

    pub fn to_core() -> Vec<ToCore> {
        vec![
            ToCore::Welcome(Welcome {
                protocol: 1,
                build: "abc123".into(),
                pid: 4243,
                previous: Previous::Crash,
                sessions: vec![HeldSession {
                    session: 1,
                    pid: 5000,
                    cols: 80,
                    rows: 24,
                    exit: Some(ExitStatus { code: Some(0) }),
                    meta: meta(),
                    truncated: true,
                }],
            }),
            ToCore::Output(Data {
                session: 1,
                bytes: b"\x1b[31mred\r\n".to_vec(),
            }),
            ToCore::Spawned(Spawned {
                session: 1,
                pid: 5000,
            }),
            ToCore::SpawnFailed(SpawnFailed {
                session: 2,
                message: "nope: No such file or directory (os error 2)".into(),
            }),
            ToCore::Exited(Exited {
                session: 1,
                code: Some(3),
            }),
            ToCore::Error(ErrorMessage {
                session: Some(1),
                message: "unsupported".into(),
            }),
            ToCore::CheckpointBegin(BlobBegin {
                session: 1,
                len: u64::MAX,
            }),
            ToCore::CheckpointChunk(Data {
                session: 1,
                bytes: Vec::new(),
            }),
            ToCore::CheckpointEnd(BlobEnd { session: 1 }),
            ToCore::TailBegin(BlobBegin { session: 1, len: 0 }),
            ToCore::TailChunk(Data {
                session: 1,
                bytes: b"x".to_vec(),
            }),
            ToCore::TailEnd(BlobEnd { session: 1 }),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::samples::*;
    use super::*;

    fn over_the_wire(frame: Frame) -> Frame {
        let mut bytes = Vec::new();
        frame.write_to(&mut bytes).unwrap();
        Frame::read_from(&mut bytes.as_slice()).unwrap()
    }

    #[test]
    fn every_core_message_roundtrips() {
        let mut types = Vec::new();
        for message in &to_holder() {
            let frame = over_the_wire(message.encode().unwrap());
            types.push(frame.msg_type);
            assert_eq!(&ToHolder::decode(&frame).unwrap(), message);
        }
        for expected in MSG_HELLO..=MSG_RESUME {
            assert!(types.contains(&expected), "no sample of type {expected}");
        }
    }

    #[test]
    fn every_holder_message_roundtrips() {
        let mut types = Vec::new();
        for message in &to_core() {
            let frame = over_the_wire(message.encode().unwrap());
            types.push(frame.msg_type);
            assert_eq!(&ToCore::decode(&frame).unwrap(), message);
        }
        for expected in
            (MSG_CHECKPOINT_BEGIN..=MSG_CHECKPOINT_END).chain(MSG_WELCOME..=MSG_TAIL_END)
        {
            assert!(types.contains(&expected), "no sample of type {expected}");
        }
    }

    #[test]
    fn binary_messages_start_with_the_session_id() {
        let frame = ToHolder::Input(Data {
            session: 0x0a0b_0c0d,
            bytes: b"q".to_vec(),
        })
        .encode()
        .unwrap();
        assert_eq!(
            (frame.msg_type, &frame.payload[..]),
            (MSG_INPUT, &[10, 11, 12, 13, b'q'][..])
        );

        let frame = ToCore::TailBegin(BlobBegin {
            session: 2,
            len: 0x0102,
        })
        .encode()
        .unwrap();
        assert_eq!(frame.payload, [0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 1, 2]);

        let frame = ToCore::TailEnd(BlobEnd { session: 2 }).encode().unwrap();
        assert_eq!(frame.payload, [0, 0, 0, 2]);

        for (msg_type, payload) in [
            (MSG_OUTPUT, &[0u8, 0, 1][..]),
            (MSG_TAIL_BEGIN, &[0, 0, 0, 2, 0, 0, 0]),
            (MSG_TAIL_BEGIN, &[0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0]),
            (MSG_TAIL_END, &[0, 0, 0, 2, 0]),
            (MSG_CHECKPOINT_END, &[]),
        ] {
            let frame = Frame::new(msg_type, payload.to_vec()).unwrap();
            let error = ToCore::decode(&frame).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{error}");
        }
    }

    #[test]
    fn the_cores_meta_has_sorted_keys_and_tolerates_anything() {
        let meta = SessionMeta {
            command: "bash -l".into(),
            cwd: Some("/home/u".into()),
            owner: "4f1c9a".into(),
            policy: crate::client::Policy::Detach,
            scrollback: 10_000,
        };
        assert_eq!(
            serde_json::to_string(&meta.to_value()).unwrap(),
            r#"{"command":"bash -l","cwd":"/home/u","owner":"4f1c9a","policy":"detach","scrollback":10000}"#
        );
        assert_eq!(SessionMeta::from_value(&meta.to_value()), meta);
        assert_eq!(SessionMeta::from_value(&samples::meta()).owner, "4f1c9a");

        let odd = serde_json::json!({"owner": 7, "policy": "kill", "later": [1]});
        assert_eq!(
            SessionMeta::from_value(&odd),
            SessionMeta {
                policy: crate::client::Policy::Kill,
                ..SessionMeta::default()
            }
        );
        assert_eq!(
            SessionMeta::from_value(&serde_json::Value::Null),
            SessionMeta::default()
        );
    }

    #[test]
    fn meta_is_kept_as_json() {
        let frame = ToHolder::Spawn(Spawn {
            session: 4,
            command: None,
            args: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
            cols: 80,
            rows: 24,
            cell_w: 0,
            cell_h: 0,
            // Sorted keys: serde_json sorts them, or keeps insertion order
            // when another crate in the build enables `preserve_order`.
            meta: serde_json::json!({"a": "b", "z": [1, {"a": null}]}),
        })
        .encode()
        .unwrap();
        let text = String::from_utf8(frame.payload.clone()).unwrap();
        assert!(
            text.ends_with(r#""meta":{"a":"b","z":[1,{"a":null}]}}"#),
            "{text}"
        );
        let ToHolder::Spawn(spawn) = ToHolder::decode(&frame).unwrap() else {
            panic!("not a spawn");
        };
        assert_eq!(spawn.meta["z"][1]["a"], serde_json::Value::Null);
        // A holder stores meta and sends back the same bytes.
        assert_eq!(ToHolder::Spawn(spawn).encode().unwrap(), frame);
    }

    #[test]
    fn unknown_fields_are_ignored_and_missing_ones_defaulted() {
        let frame = Frame::new(
            MSG_WELCOME,
            br#"{"protocol":1,"pid":9,"later":true,"sessions":[{"session":3,"pid":10,"cols":80,"rows":24,"new":1}]}"#
                .to_vec(),
        )
        .unwrap();
        assert_eq!(
            ToCore::decode(&frame).unwrap(),
            ToCore::Welcome(Welcome {
                protocol: 1,
                build: String::new(),
                pid: 9,
                previous: Previous::None,
                sessions: vec![HeldSession {
                    session: 3,
                    pid: 10,
                    cols: 80,
                    rows: 24,
                    exit: None,
                    meta: serde_json::Value::Null,
                    truncated: false,
                }],
            })
        );
        let frame = Frame::new(MSG_GOODBYE, b"{}".to_vec()).unwrap();
        assert_eq!(
            ToHolder::decode(&frame).unwrap(),
            ToHolder::Goodbye(Goodbye { handover: false })
        );
        let frame = Frame::new(MSG_DRAIN, br#"{"soon":1}"#.to_vec()).unwrap();
        assert_eq!(ToHolder::decode(&frame).unwrap(), ToHolder::Drain(Drain {}));
        let frame = Frame::new(MSG_EXITED, br#"{"session":5}"#.to_vec()).unwrap();
        assert_eq!(
            ToCore::decode(&frame).unwrap(),
            ToCore::Exited(Exited {
                session: 5,
                code: None
            })
        );
    }

    #[test]
    fn previous_uses_the_documented_names() {
        for (previous, name) in [
            (Previous::None, "none"),
            (Previous::Handover, "handover"),
            (Previous::Crash, "crash"),
        ] {
            assert_eq!(
                serde_json::to_string(&previous).unwrap(),
                format!("\"{name}\"")
            );
        }
        let frame = Frame::new(
            MSG_WELCOME,
            br#"{"protocol":1,"pid":1,"previous":"reboot"}"#.to_vec(),
        )
        .unwrap();
        assert!(ToCore::decode(&frame).is_err());
    }

    #[test]
    fn unknown_and_wrong_direction_types_are_invalid_data() {
        for msg_type in [0, MSG_WELCOME, MSG_OUTPUT, MSG_TAIL_BEGIN, 14, 255] {
            let frame = Frame::new(msg_type, vec![0, 0, 0, 1]).unwrap();
            let error = ToHolder::decode(&frame).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        }
        for msg_type in [0, MSG_HELLO, MSG_INPUT, MSG_REPARENT, MSG_RESUME, 41, 255] {
            let frame = Frame::new(msg_type, vec![0, 0, 0, 1]).unwrap();
            assert!(ToCore::decode(&frame).is_err());
        }
    }

    #[test]
    fn split_and_assemble_roundtrip() {
        let blob: Vec<u8> = (0..=255u8).cycle().take(1000).collect();
        for chunk_len in [0, 1, 7, 999, 1000, 4096] {
            let (begin, chunks, end) = split(9, &blob, chunk_len);
            assert_eq!(
                begin,
                BlobBegin {
                    session: 9,
                    len: 1000
                }
            );
            assert!(chunks.iter().all(|c| c.session == 9 && !c.bytes.is_empty()));
            let mut assembler = Assembler::new(TAIL_LIMIT);
            assembler.begin(begin).unwrap();
            for chunk in &chunks {
                assembler.chunk(chunk).unwrap();
            }
            assert_eq!(assembler.end(end).unwrap(), blob);
            assert!(!assembler.is_open(9));
        }
        let (begin, chunks, end) = split(1, b"", CHUNK_LEN);
        assert_eq!(begin.len, 0);
        assert!(chunks.is_empty());
        let mut assembler = Assembler::new(0);
        assembler.begin(begin).unwrap();
        assert_eq!(assembler.end(end).unwrap(), b"");
    }

    #[test]
    fn checkpoints_larger_than_a_frame_are_chunked() {
        let blob = vec![b'x'; 3 * 1024 * 1024 + 5];
        let messages = ToHolder::checkpoint(4, &blob, 7);
        assert_eq!(messages.len(), 2 + blob.len().div_ceil(CHUNK_LEN));
        let mut assembler = Assembler::new(MAX_CHECKPOINT_LEN);
        let mut done = None;
        for message in &messages {
            let frame = message.encode().unwrap();
            assert!(frame.payload.len() <= crate::MAX_FRAME_PAYLOAD_SIZE);
            match ToHolder::decode(&frame).unwrap() {
                ToHolder::CheckpointBegin(begin) => assembler.begin(begin).unwrap(),
                ToHolder::CheckpointChunk(chunk) => assembler.chunk(&chunk).unwrap(),
                ToHolder::CheckpointEnd(end) => {
                    assert_eq!(end.covers, 7);
                    done = Some(assembler.end(end.blob_end()).unwrap());
                }
                other => panic!("unexpected {other:?}"),
            }
        }
        assert_eq!(done.unwrap(), blob);

        let tail = ToCore::tail(4, &[1; 10]);
        assert!(matches!(
            tail[0],
            ToCore::TailBegin(BlobBegin { len: 10, .. })
        ));
        assert!(matches!(tail[2], ToCore::TailEnd(_)));
        let checkpoint = ToCore::checkpoint(4, &[]);
        assert_eq!(checkpoint.len(), 2);
    }

    #[test]
    fn the_assembler_rejects_broken_sequences() {
        let chunk = |session, bytes: &[u8]| Data {
            session,
            bytes: bytes.to_vec(),
        };
        let mut a = Assembler::new(8);
        assert!(a.chunk(&chunk(1, b"x")).is_err(), "chunk outside a blob");
        assert!(a.end(BlobEnd { session: 1 }).is_err(), "end without begin");
        assert!(
            a.begin(BlobBegin { session: 1, len: 9 }).is_err(),
            "over the limit"
        );
        assert!(!a.is_open(1));

        a.begin(BlobBegin { session: 1, len: 4 }).unwrap();
        a.begin(BlobBegin { session: 2, len: 2 }).unwrap();
        a.chunk(&chunk(1, b"ab")).unwrap();
        a.chunk(&chunk(2, b"cd")).unwrap();
        assert!(a.chunk(&chunk(2, b"e")).is_err(), "past the length");
        assert!(!a.is_open(2));
        assert!(a.end(BlobEnd { session: 1 }).is_err(), "short");

        a.begin(BlobBegin { session: 3, len: 1 }).unwrap();
        assert!(a.begin(BlobBegin { session: 3, len: 1 }).is_err(), "nested");
        a.begin(BlobBegin { session: 3, len: 1 }).unwrap();
        a.abandon(3);
        assert!(!a.is_open(3));
        // Interleaved sessions each get their own bytes.
        a.begin(BlobBegin { session: 5, len: 2 }).unwrap();
        a.begin(BlobBegin { session: 6, len: 1 }).unwrap();
        a.chunk(&chunk(5, b"a")).unwrap();
        a.chunk(&chunk(6, b"z")).unwrap();
        a.chunk(&chunk(5, b"b")).unwrap();
        assert_eq!(a.end(BlobEnd { session: 6 }).unwrap(), b"z");
        assert_eq!(a.end(BlobEnd { session: 5 }).unwrap(), b"ab");
    }

    #[test]
    fn hello_is_answered_with_the_highest_common_version() {
        let hello = |min, max| Hello {
            min,
            max,
            core_build: String::new(),
            instance: 0,
        };
        assert_eq!(answer_hello(&hello(1, 3), PROTOCOLS), Ok(1));
        let error = answer_hello(&hello(2, 2), PROTOCOLS).unwrap_err();
        assert!(
            error.message.contains("offered 2, supported 1"),
            "{}",
            error.message
        );
    }
}
