//! The terminal server's client protocol, between a client (Brindle, or the
//! `brindle-remote` management commands) and the core. This is protocols 1
//! to 4.
//!
//! Frames use the crate's header (`type: u8`, `len: u32 BE`). `DATA` is
//! binary: a `u32` BE session id, then the bytes (input from the client,
//! output from the core). Every other message is a JSON object; unknown
//! fields are ignored and missing optional fields take their defaults, so
//! fields can be added within a version. A new message type needs a new
//! version.
//!
//! | type | name            | direction     | payload                                                        |
//! |-----:|-----------------|---------------|----------------------------------------------------------------|
//! |    1 | `DATA`          | both          | session, bytes                                                 |
//! |    2 | `HELLO`         | client → core | `min, max, client_version, client_token, linger_secs?, process_info?` |
//! |    3 | `SPAWN`         | client → core | `req, command, args, cwd, env, cols, rows, cell_w, cell_h, scrollback, policy, profile?` |
//! |    4 | `RESIZE`        | client → core | `session, cols, rows, cell_w, cell_h`                          |
//! |    5 | `CLOSE`         | client → core | `session`                                                      |
//! |    6 | `ATTACH`        | client → core | `req, session, cols, rows, page?`                              |
//! |    7 | `DETACH`        | client → core | `session, layout?`                                             |
//! |    8 | `LIST`          | client → core | `req`                                                          |
//! |    9 | `KILL`          | client → core | `req, target: {"session": id} \| "all"`                        |
//! |   10 | `STATUS`        | client → core | `req`                                                          |
//! |   11 | `STOP`          | client → core | `req`                                                          |
//! |   12 | `STEP_DOWN`     | client → core | `req`                                                          |
//! |   13 | `REPARENT`      | client → core | `session, hwnd` (reserved; a Linux core refuses it)            |
//! |   14 | `SET_LAYOUT`    | client → core | `session, layout?` (protocol 3 only)                           |
//! |   15 | `SPAWN_AGENT`   | client → core | `req, command, args, cwd, env, policy, profile?, conversation?, start?` (protocol 4) |
//! |   16 | `AGENT_PROMPT`  | client → core | `req, session, text` (protocol 4)                              |
//! |   17 | `AGENT_CANCEL`  | client → core | `session` (protocol 4)                                         |
//! |   18 | `AGENT_PERMISSION` | client → core | `req, session, entry, option?` (protocol 4)                 |
//! |   19 | `AGENT_UNQUEUE` | client → core | `req, session` (protocol 4)                                    |
//! |   20 | `AGENT_RESEND`  | client → core | `req, session` (protocol 4)                                    |
//! |   21 | `AGENT_RESTART` | client → core | `req, session, command, args, env` (protocol 4)                |
//! |   22 | `AGENT_LOGIN`   | client → core | `req, session, method` (protocol 4)                            |
//! |   23 | `AGENT_FETCH`   | client → core | `req, session, before, max_bytes` (protocol 4)                 |
//! |   24 | `AGENT_SET_OPTION` | client → core | `req, session, option, value` (protocol 4)                  |
//! |   31 | `AGENT_LIST`    | client → core | `req, session, cursor?` (protocol 4)                           |
//! |   64 | `AGENT_CHOOSE`  | client → core | `req, session, conversation?` (protocol 4)                     |
//! |   32 | `WELCOME`       | core → client | `protocol, core_version, core_build, pid, holder_protocols`    |
//! |   33 | `ERROR`         | core → client | `req?, session?, message`                                      |
//! |   34 | `SPAWNED`       | core → client | `req, session, pid, key?`                                      |
//! |   35 | `EXIT`          | core → client | `session, code?`                                               |
//! |   36 | `REPLAY_START`  | core → client | `req?, session, pid`                                           |
//! |   37 | `REPLAY_END`    | core → client | `session`                                                      |
//! |   38 | `SESSIONS`      | core → client | `req, sessions: [{session, pid, attached, command, cwd, policy, cols, rows, key?, title?, started?, profile?, owned?, detached_secs?, layout?, agent?}]` |
//! |   39 | `SERVER_STATUS` | core → client | `req, core_version, core_build, protocols, holder_protocols, pid, uptime_secs, clients, holders` |
//! |   40 | `OK`            | core → client | `req`                                                          |
//! |   41 | `PROCESS`       | core → client | `session, name?, cwd?` (protocol 2 only)                       |
//! |   42 | `TRANSCRIPT`    | core → client | `session, events: [object]` (protocol 4)                       |
//! |   43 | `AGENT_PAGE`    | core → client | `req, session, events: [object], last?: {more}` (protocol 4)   |
//! |   45 | `AGENT_CONVERSATIONS` | core → client | `req, session, conversations: [{id, title?, updated?}], next?` (protocol 4) |
//!
//! The two directions share one number space: clients' messages are below
//! 32 and from 64, the core's from 32 to 63.
//!
//! Exchanges:
//! - `HELLO` → `WELCOME`, or `ERROR` naming both ranges ([`negotiate`]).
//!   After `WELCOME`, the core replays each session it still holds for the
//!   client's `client_token` (`REPLAY_START` without `req`, `DATA`…,
//!   `REPLAY_END`).
//! - `SPAWN` → `SPAWNED` or `ERROR`; `ATTACH` → `REPLAY_START`, `DATA`…,
//!   `REPLAY_END`, or `ERROR`.
//! - `LIST` → `SESSIONS`; `STATUS` → `SERVER_STATUS`; `KILL`, `STOP` and
//!   `STEP_DOWN` → `OK` or `ERROR`.
//! - `RESIZE`, `CLOSE`, `DETACH`, `REPARENT` and `SET_LAYOUT` have no reply; a refused one
//!   gets an `ERROR` with its `session`.
//!
//! Fields marked `?` were added within protocol 1. They are left out when
//! empty, so messages that don't use them encode exactly as before.
//! - `key` identifies a session across server lifetimes, where its id may
//!   be reused (a new core starts its ids at 1).
//! - `title` is the session's title as its emulator keeps it, `started` its
//!   start time in unix seconds, and `profile` the client's name for what
//!   started it, kept for clients that attach it later.
//! - `linger_secs` (a `HELLO` field, capped by the core) asks the core to keep
//!   the client's `kill` sessions that long after its connection ends, and
//!   `process_info` asks for `PROCESS`.
//! - `owned` (a session has an owner and no client is attached) and
//!   `detached_secs` (seconds since a client was last attached) tell a client
//!   whether a detached session's owner may still come back.
//! - `layout` (on `DETACH`, and in `SESSIONS` while the session stays
//!   detached) is the client's record of where the session sat, which the
//!   core keeps in the session's meta without reading it, up to
//!   [`MAX_LAYOUT`] bytes of JSON, and drops when a client attaches it.
//!
//! Protocol 2 adds `PROCESS`, sent only on connections that negotiated 2 and
//! set `process_info`: the session's foreground program name and working
//! directory when either changed (a field is `null` when unknown). Nothing
//! else differs from protocol 1, so a core that speaks both and a client that
//! speaks only 1 agree on 1.
//!
//! Protocol 3 adds `SET_LAYOUT`, which a client sends only on a connection that
//! negotiated 3: it replaces the layout of a session the client has attached,
//! as `DETACH`'s `layout` does, so a client that ends without detaching still
//! leaves its last one. The core refuses it with an `ERROR` naming the session
//! on a connection below 3, from a client without the session attached, or for
//! a session it doesn't have. Nothing else differs from protocol 2.
//!
//! Protocol 4 adds agent sessions: an ACP agent the core runs on plain pipes
//! and speaks ACP to, whose conversation clients see as transcript events.
//! The new messages are sent only on connections that negotiated 4:
//! - `SPAWN_AGENT` → `SPAWNED` or `ERROR` starts one; `command` is required
//!   to be non-empty (the core refuses an empty one), there is no grid.
//! - `AGENT_PROMPT`, `AGENT_PERMISSION` (`entry` is the permission request's
//!   entry id; no `option` answers it cancelled), `AGENT_UNQUEUE` (discards
//!   the queue slot's prompt) and `AGENT_RESEND` (sends a held prompt again)
//!   → `OK` or `ERROR`; `AGENT_CANCEL` has no reply, and a refused one gets
//!   an `ERROR` with its `session`. Only the client that has the session
//!   attached may send them. Whether a prompt was sent or queued, the client
//!   learns from the events, not the reply.
//! - `AGENT_RESTART` → `OK` or `ERROR` starts a new agent process for an
//!   agent session whose agent ended (exited, or lost with its holder), to
//!   continue its conversation in the same log; the client sends the
//!   program to run, and the conversation's directory is the session's.
//!   Only the client that has the session attached may send it, and the
//!   core refuses it for a session whose agent hasn't ended or can't
//!   continue. The outcome arrives as transcript events.
//! - `AGENT_LOGIN` → `OK` or `ERROR` logs in an agent that needs a login
//!   with one of the login methods its "login required" state carries,
//!   which the core runs (ACP's `authenticate`); the agent's state then
//!   says it is logging in. Only the client that has the session attached
//!   may send it, and the core refuses it for a method the agent doesn't
//!   offer or one the client runs (a terminal login), while the agent
//!   doesn't need a login, and while a login runs. While a login runs,
//!   `AGENT_CANCEL` cancels it rather than a turn. The outcome arrives as
//!   transcript events.
//! - `SPAWN_AGENT`'s `conversation` (`id`, `cwd`) asks the new agent to
//!   continue that conversation (by `session/load` or `session/resume`)
//!   instead of starting one, for a client whose session the server no
//!   longer has; left out, the agent starts a new one.
//! - `TRANSCRIPT` carries a session's transcript events, each a JSON object
//!   this crate doesn't read: `brindle-transcript` encodes and decodes them
//!   on both sides. It is checked only to be an array of objects, and a
//!   frame holds whole events ([`pack_transcript`]). `ATTACH` of an agent
//!   session is answered with `REPLAY_START`, `TRANSCRIPT`…, `REPLAY_END`,
//!   then live `TRANSCRIPT`s; its `cols` and `rows` are ignored.
//! - Paging (OpenSpec change `agents-log-paging`, amending protocol 4
//!   before it shipped): an agent session's replay is a page of its newest
//!   whole entries (`brindle-transcript`'s `Transcript::page`), about 256
//!   KiB, behind a `window` event when older entries were left out.
//!   `ATTACH`'s `page` asks for a smaller page, in bytes (the core clamps
//!   it to its own; left out, the core's). `AGENT_FETCH` → `AGENT_PAGE`…
//!   or `ERROR` asks for the entries before entry `before`, up to
//!   `max_bytes` (clamped the same way): one or more `AGENT_PAGE` frames of
//!   whole events with the request's `req` ([`pack_page`]), the last with
//!   `last: {more}`, saying whether older entries remain; an empty page is
//!   one frame. Only the client that has the session attached, and whose
//!   replay of it has ended, may fetch.
//! - `SessionInfo.agent` (`activity`, `name?`) is set for agent sessions,
//!   which only protocol 4 lists, with size 0×0; it is left out otherwise.
//!   [`AgentActivity`] is this crate's own closed enum, in program status's
//!   names.
//! - Listing (OpenSpec change `agents-session-list`, amending protocol 4
//!   before it shipped): `SPAWN_AGENT`'s `start: "choose"` starts an agent
//!   that answers `initialize` and then waits, its state `choosing`, with
//!   no conversation. `AGENT_CHOOSE` → `OK` or `ERROR` ends the wait:
//!   with `conversation`, the agent continues that conversation in the
//!   session's directory; without, it starts a new one. `AGENT_LIST` →
//!   `AGENT_CONVERSATIONS` or `ERROR` asks a ready or choosing agent for a
//!   page of its conversations in the session's directory, `cursor` being
//!   an earlier reply's `next`; the core keeps no list. Only the client
//!   that has the session attached may send either.
//! - Options (OpenSpec change `agents-commands`, amending protocol 4
//!   before it shipped): the agent's slash commands and session options
//!   travel as transcript events. `AGENT_SET_OPTION` → `OK` or `ERROR`
//!   asks a ready agent to set option `option` (an id the session's
//!   options list) to `value` ([`OptionValue`]): `OK` means asked (or
//!   already so), and the agent's answer arrives as transcript events.
//!   Any client attached to the session may send it, during a turn too.
//!
//! A client below 4 isn't shown agent sessions, and is refused an `ATTACH`
//! of one with an `ERROR` naming the session. Nothing else differs from
//! protocol 3.

use std::collections::BTreeMap;
use std::io;

use serde::{Deserialize, Serialize};

use crate::Frame;
use crate::wire::{self, RequestId, SessionId};

pub use crate::version::{NegotiationError, VersionRange, negotiate};

/// The client protocol versions this build speaks.
pub const PROTOCOLS: VersionRange = VersionRange::new(1, 4);

pub const MSG_DATA: u8 = 1;
pub const MSG_HELLO: u8 = 2;
pub const MSG_SPAWN: u8 = 3;
pub const MSG_RESIZE: u8 = 4;
pub const MSG_CLOSE: u8 = 5;
pub const MSG_ATTACH: u8 = 6;
pub const MSG_DETACH: u8 = 7;
pub const MSG_LIST: u8 = 8;
pub const MSG_KILL: u8 = 9;
pub const MSG_STATUS: u8 = 10;
pub const MSG_STOP: u8 = 11;
pub const MSG_STEP_DOWN: u8 = 12;
pub const MSG_REPARENT: u8 = 13;
/// Protocol 3.
pub const MSG_SET_LAYOUT: u8 = 14;
/// Protocol 4.
pub const MSG_SPAWN_AGENT: u8 = 15;
/// Protocol 4.
pub const MSG_AGENT_PROMPT: u8 = 16;
/// Protocol 4.
pub const MSG_AGENT_CANCEL: u8 = 17;
/// Protocol 4.
pub const MSG_AGENT_PERMISSION: u8 = 18;
/// Protocol 4.
pub const MSG_AGENT_UNQUEUE: u8 = 19;
/// Protocol 4.
pub const MSG_AGENT_RESEND: u8 = 20;
/// Protocol 4.
pub const MSG_AGENT_RESTART: u8 = 21;
/// Protocol 4.
pub const MSG_AGENT_LOGIN: u8 = 22;
/// Protocol 4.
pub const MSG_AGENT_FETCH: u8 = 23;
/// Protocol 4 (`agents-commands`).
pub const MSG_AGENT_SET_OPTION: u8 = 24;
/// Protocol 4 (`agents-session-list`; 25 to 30 are kept for the changes
/// planned before it).
pub const MSG_AGENT_LIST: u8 = 31;
/// Protocol 4 (`agents-session-list`): the first of the clients' second
/// block, since 32 to 63 are the core's.
pub const MSG_AGENT_CHOOSE: u8 = 64;
pub const MSG_WELCOME: u8 = 32;
pub const MSG_ERROR: u8 = 33;
pub const MSG_SPAWNED: u8 = 34;
pub const MSG_EXIT: u8 = 35;
pub const MSG_REPLAY_START: u8 = 36;
pub const MSG_REPLAY_END: u8 = 37;
pub const MSG_SESSIONS: u8 = 38;
pub const MSG_SERVER_STATUS: u8 = 39;
pub const MSG_OK: u8 = 40;
/// Protocol 2.
pub const MSG_PROCESS: u8 = 41;
/// Protocol 4.
pub const MSG_TRANSCRIPT: u8 = 42;
/// Protocol 4.
pub const MSG_AGENT_PAGE: u8 = 43;
/// Protocol 4 (`agents-session-list`; 44 is kept for a change planned
/// before it).
pub const MSG_AGENT_CONVERSATIONS: u8 = 45;

/// Bytes for one session: input from the client, output from the core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Data {
    pub session: SessionId,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub min: u32,
    pub max: u32,
    /// The client's release, for the core's log and `status`.
    #[serde(default)]
    pub client_version: String,
    /// Random per Brindle process; a reconnecting client gets back the
    /// sessions it owns under this token.
    pub client_token: String,
    /// How long the core keeps this client's `kill` sessions running after
    /// its connection ends, so a reconnect gets them back; 0 hangs them up
    /// at once. The core caps it.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub linger_secs: u32,
    /// Asks for `PROCESS` messages (protocol 2).
    #[serde(default, skip_serializing_if = "is_false")]
    pub process_info: bool,
}

impl Hello {
    pub fn range(&self) -> VersionRange {
        VersionRange::new(self.min, self.max)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Welcome {
    /// The negotiated version; everything after `WELCOME` uses it.
    pub protocol: u32,
    pub core_version: String,
    pub core_build: String,
    pub pid: u32,
    /// The H1 versions of the holders the core is driving.
    #[serde(default)]
    pub holder_protocols: Vec<u32>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Policy {
    /// The session ends when its client closes it or is gone.
    #[default]
    Kill,
    /// The session keeps running unattached when its client is gone.
    Detach,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spawn {
    pub req: RequestId,
    /// `None` runs the user's login shell.
    #[serde(default)]
    pub command: Option<String>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    /// The program's complete environment; the server adds nothing.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    pub cols: u16,
    pub rows: u16,
    #[serde(default)]
    pub cell_w: u16,
    #[serde(default)]
    pub cell_h: u16,
    /// The emulator's scrollback limit, in lines.
    #[serde(default)]
    pub scrollback: u32,
    #[serde(default)]
    pub policy: Policy,
    /// The client's name for what started the session (a profile).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Spawned {
    pub req: RequestId,
    pub session: SessionId,
    pub pid: u32,
    /// The session's key; empty from a core that predates keys.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub key: String,
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

/// A message that names only a session: `CLOSE`, `REPLAY_END`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRef {
    pub session: SessionId,
}

/// `DETACH`: the session keeps running with no client.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Detach {
    pub session: SessionId,
    /// The client's record of where the session sat (its tab), kept for
    /// whichever client attaches it later. Opaque to the core.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<serde_json::Value>,
}

/// `SET_LAYOUT` (protocol 3): the layout of a session the client has
/// attached, replacing the one it had; `None` clears it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetLayout {
    pub session: SessionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<serde_json::Value>,
}

/// The largest layout the core keeps (`DETACH`, `SET_LAYOUT`), as encoded
/// JSON.
pub const MAX_LAYOUT: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Exit {
    pub session: SessionId,
    /// `None` when the program was ended by a signal.
    #[serde(default)]
    pub code: Option<i32>,
    /// Set when the session ended because the server lost it (its holder
    /// ended), not because its program exited: why. A client keeps such a
    /// tab open and says so.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lost: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attach {
    pub req: RequestId,
    pub session: SessionId,
    pub cols: u16,
    pub rows: u16,
    /// Protocol 4: the most an agent session's replay page holds, in
    /// bytes; the core clamps it to its own. Left out when unset, so an
    /// `ATTACH` without it encodes as before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplayStart {
    /// The `ATTACH` this answers; `None` for a reconnect's replays.
    #[serde(default)]
    pub req: Option<RequestId>,
    pub session: SessionId,
    pub pid: u32,
}

/// A request with nothing but its id: `LIST`, `STATUS`, `STOP`, `STEP_DOWN`,
/// and the `OK` reply.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Request {
    pub req: RequestId,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KillTarget {
    Session(SessionId),
    All,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Kill {
    pub req: RequestId,
    pub target: KillTarget,
}

/// Reserved in protocol 1 for Windows; a Linux core answers with `ERROR`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Reparent {
    pub session: SessionId,
    pub hwnd: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorMessage {
    #[serde(default)]
    pub req: Option<RequestId>,
    #[serde(default)]
    pub session: Option<SessionId>,
    pub message: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInfo {
    pub session: SessionId,
    pub pid: u32,
    #[serde(default)]
    pub attached: bool,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub policy: Policy,
    #[serde(default)]
    pub cols: u16,
    #[serde(default)]
    pub rows: u16,
    /// Empty from a core that predates keys.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub key: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    /// Unix seconds; 0 when unknown.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub started: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// An owner is set and no client is attached: the owner may still come
    /// back for the session. Always false while attached.
    #[serde(default, skip_serializing_if = "is_false")]
    pub owned: bool,
    /// Seconds since a client was last attached; 0 when attached or unknown.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub detached_secs: u64,
    /// The layout its last client detached it with ([`Detach::layout`]);
    /// none while attached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<serde_json::Value>,
    /// Set for an agent session (protocol 4), which is listed with size 0×0.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<AgentInfo>,
}

/// What the session list says about an agent session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentInfo {
    pub activity: AgentActivity,
    /// The name the agent gave in `initialize`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// What an agent session is doing, in program status's names: `working`
/// while starting or in a turn, `blocked` on a permission request, `done`
/// after a turn that ended with `end_turn`, `idle` with no turn or after a
/// cancelled one, `error` when the agent or its turn stopped otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentActivity {
    Idle,
    Working,
    Blocked,
    Done,
    Error,
}

/// `SPAWN_AGENT` (protocol 4): an ACP agent run on plain pipes, with no
/// grid. The core refuses an empty `command`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpawnAgent {
    pub req: RequestId,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    /// The agent's complete environment; the server adds nothing.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub policy: Policy,
    /// The client's name for what started the session (a profile).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// A conversation for the agent to continue instead of starting one;
    /// the session then runs in the conversation's `cwd`, which ACP ties
    /// the conversation to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<AgentConversation>,
    /// How the agent starts when no `conversation` is named: left out, a
    /// new conversation; `choose`, none until a client sends
    /// `AGENT_CHOOSE` (OpenSpec change `agents-session-list`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<SpawnStart>,
}

/// `SPAWN_AGENT`'s `start`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpawnStart {
    /// Initialize the agent, then wait for a choice.
    Choose,
}

/// The `ERROR` text answering an `AGENT_LIST` whose agent doesn't offer a
/// list of conversations. A client tells this refusal apart by it (the
/// choosing screen then offers New conversation alone), so the core writes
/// exactly this.
pub const LIST_NOT_OFFERED: &str = "the agent can't list conversations";

/// `AGENT_LIST` (protocol 4): a page of the agent's earlier conversations
/// in the session's directory, after `cursor` (an earlier reply's `next`),
/// answered by `AGENT_CONVERSATIONS`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentList {
    pub req: RequestId,
    pub session: SessionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

/// `AGENT_CHOOSE` (protocol 4): ends a choosing session's wait, continuing
/// the agent's conversation `conversation` in the session's directory, or
/// starting a new one when it is left out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentChoose {
    pub req: RequestId,
    pub session: SessionId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<String>,
}

/// `AGENT_CONVERSATIONS` (protocol 4): the answer to an `AGENT_LIST`, in the
/// agent's order, with `next` to ask for the page after it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentConversations {
    pub req: RequestId,
    pub session: SessionId,
    #[serde(default)]
    pub conversations: Vec<ListedConversation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<String>,
}

/// One conversation of an `AGENT_CONVERSATIONS`: the agent's id for it, its
/// title, and when it was last active, as the agent's text.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListedConversation {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updated: Option<String>,
}

/// An agent's conversation, as `SPAWN_AGENT` names one to continue: the ACP
/// session id the agent gave, and the directory it was started in.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentConversation {
    pub id: String,
    pub cwd: String,
}

/// `AGENT_RESTART` (protocol 4): run the agent again for a session whose
/// agent ended, continuing its conversation. The core refuses an empty
/// `command`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRestart {
    pub req: RequestId,
    pub session: SessionId,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    /// The agent's complete environment; the server adds nothing.
    #[serde(default)]
    pub env: BTreeMap<String, String>,
}

/// `AGENT_LOGIN` (protocol 4): log the agent in with the login method whose
/// id is `method`, which the core runs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentLogin {
    pub req: RequestId,
    pub session: SessionId,
    pub method: String,
}

/// `AGENT_SET_OPTION` (protocol 4, OpenSpec change `agents-commands`): set
/// the session option whose id is `option` to `value`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSetOption {
    pub req: RequestId,
    pub session: SessionId,
    pub option: String,
    pub value: OptionValue,
}

/// The value an `AGENT_SET_OPTION` sets: one of a select option's choices,
/// by its value, or a toggle's state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OptionValue {
    Choice { value: String },
    Toggle { on: bool },
}

/// `AGENT_FETCH` (protocol 4): the entries before entry `before`, up to
/// `max_bytes` of them (the core clamps it), answered by `AGENT_PAGE`s.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentFetch {
    pub req: RequestId,
    pub session: SessionId,
    pub before: u64,
    pub max_bytes: u32,
}

/// `AGENT_PAGE` (protocol 4): one frame of an `AGENT_FETCH`'s answer, whole
/// transcript events in order; the last frame carries `last`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentPage {
    pub req: RequestId,
    pub session: SessionId,
    #[serde(default)]
    pub events: Vec<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last: Option<PageEnd>,
}

/// How a fetch's answer ends: whether older entries remain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageEnd {
    pub more: bool,
}

impl AgentPage {
    /// Refuses events that aren't JSON objects.
    fn check(&self) -> io::Result<()> {
        match self.events.iter().position(|event| !event.is_object()) {
            None => Ok(()),
            Some(index) => Err(wire::invalid(format!(
                "AGENT_PAGE event {index} of session {} isn't a JSON object",
                self.session
            ))),
        }
    }
}

/// `AGENT_PROMPT` (protocol 4): plain text for the agent, sent now or
/// queued.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentPrompt {
    pub req: RequestId,
    pub session: SessionId,
    pub text: String,
}

/// `AGENT_PERMISSION` (protocol 4): the answer to the permission request in
/// entry `entry`, the option chosen, or none to answer it cancelled.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentPermission {
    pub req: RequestId,
    pub session: SessionId,
    pub entry: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub option: Option<String>,
}

/// A request about an agent session with nothing but its ids:
/// `AGENT_UNQUEUE` and `AGENT_RESEND` (protocol 4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentRequest {
    pub req: RequestId,
    pub session: SessionId,
}

/// `TRANSCRIPT` (protocol 4): a session's transcript events, each a JSON
/// object `brindle-transcript` gives meaning to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Transcript {
    pub session: SessionId,
    #[serde(default)]
    pub events: Vec<serde_json::Value>,
}

impl Transcript {
    /// Refuses events that aren't JSON objects.
    fn check(&self) -> io::Result<()> {
        match self.events.iter().position(|event| !event.is_object()) {
            None => Ok(()),
            Some(index) => Err(wire::invalid(format!(
                "TRANSCRIPT event {index} of session {} isn't a JSON object",
                self.session
            ))),
        }
    }
}

/// The most a `TRANSCRIPT` takes around its events:
/// `{"session":4294967295,"events":[]}`, the longest session id's; each
/// event after the first adds a comma.
pub const TRANSCRIPT_ENVELOPE: usize = 34;

/// The most an `AGENT_PAGE` takes around its events:
/// `{"req":18446744073709551615,"session":4294967295,"events":[],
/// "last":{"more":false}}`, the longest ids'. No smaller than
/// [`TRANSCRIPT_ENVELOPE`], so an event that fits a page fits a
/// `TRANSCRIPT` (`acp::limits` I1 in `brindle-remote-agent`).
pub const AGENT_PAGE_ENVELOPE: usize = 83;

/// The `TRANSCRIPT` messages that carry `events` for `session` in order,
/// each holding whole events and fitting one frame. An event that can't fit
/// a frame on its own is an `InvalidInput` error, as is one that isn't a
/// JSON object; the core's reducer keeps every event within
/// `brindle_transcript::MAX_EVENT`, which fits (`acp::limits` I1).
pub fn pack_transcript(
    session: SessionId,
    events: Vec<serde_json::Value>,
) -> io::Result<Vec<Transcript>> {
    Ok(pack_events(session, events, TRANSCRIPT_ENVELOPE)?
        .into_iter()
        .map(|events| Transcript { session, events })
        .collect())
}

/// The `AGENT_PAGE` messages that answer fetch `req` with `events` for
/// `session` in order: whole events per frame, `last` on the final one,
/// and one frame with no events for an empty page. Refused as
/// [`pack_transcript`] refuses.
pub fn pack_page(
    req: RequestId,
    session: SessionId,
    events: Vec<serde_json::Value>,
    more: bool,
) -> io::Result<Vec<AgentPage>> {
    let mut pages: Vec<AgentPage> = pack_events(session, events, AGENT_PAGE_ENVELOPE)?
        .into_iter()
        .map(|events| AgentPage {
            req,
            session,
            events,
            last: None,
        })
        .collect();
    if pages.is_empty() {
        pages.push(AgentPage {
            req,
            session,
            events: Vec::new(),
            last: None,
        });
    }
    if let Some(page) = pages.last_mut() {
        page.last = Some(PageEnd { more });
    }
    Ok(pages)
}

/// `events` cut into groups of whole events that each fit a frame with an
/// `envelope` of that many bytes around them. One event too large for a
/// frame alone is an error; the server's reducer keeps every event within
/// `brindle_transcript::MAX_EVENT`, which leaves at least
/// [`AGENT_PAGE_ENVELOPE`] (the server's `acp::limits`, invariant I1).
fn pack_events(
    session: SessionId,
    events: Vec<serde_json::Value>,
    envelope: usize,
) -> io::Result<Vec<Vec<serde_json::Value>>> {
    let mut groups = Vec::new();
    let mut current = Vec::new();
    let mut size = envelope;
    for (index, event) in events.into_iter().enumerate() {
        if !event.is_object() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("transcript event {index} of session {session} isn't a JSON object"),
            ));
        }
        let len = serde_json::to_vec(&event).map_or(usize::MAX, |bytes| bytes.len());
        if envelope.saturating_add(len) > crate::MAX_FRAME_PAYLOAD_SIZE {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "transcript event {index} of session {session} is {len} bytes, past a frame's {}",
                    crate::MAX_FRAME_PAYLOAD_SIZE
                ),
            ));
        }
        if !current.is_empty() && size + 1 + len > crate::MAX_FRAME_PAYLOAD_SIZE {
            groups.push(std::mem::take(&mut current));
            size = envelope;
        }
        size += len + usize::from(!current.is_empty());
        current.push(event);
    }
    if !current.is_empty() {
        groups.push(current);
    }
    Ok(groups)
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

fn is_zero_u32(value: &u32) -> bool {
    *value == 0
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// The foreground program of a session and its working directory (protocol 2).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Process {
    pub session: SessionId,
    /// The foreground process's name (`comm`); `None` when unknown.
    #[serde(default)]
    pub name: Option<String>,
    /// The foreground process's working directory; `None` when unknown.
    #[serde(default)]
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sessions {
    pub req: RequestId,
    #[serde(default)]
    pub sessions: Vec<SessionInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HolderStatus {
    pub pid: u32,
    #[serde(default)]
    pub build: String,
    /// The H1 version the core drives this holder in.
    #[serde(default)]
    pub protocol: u32,
    #[serde(default)]
    pub sessions: u32,
    #[serde(default)]
    pub draining: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerStatus {
    pub req: RequestId,
    pub core_version: String,
    #[serde(default)]
    pub core_build: String,
    /// The client protocol versions the core speaks.
    pub protocols: VersionRange,
    /// The H1 versions the core speaks.
    pub holder_protocols: VersionRange,
    pub pid: u32,
    #[serde(default)]
    pub uptime_secs: u64,
    #[serde(default)]
    pub clients: u32,
    #[serde(default)]
    pub holders: Vec<HolderStatus>,
}

/// Messages from a client to the core.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToCore {
    Hello(Hello),
    Spawn(Spawn),
    /// Input for a session.
    Data(Data),
    Resize(Resize),
    Close(SessionRef),
    Attach(Attach),
    Detach(Detach),
    List(Request),
    Kill(Kill),
    Status(Request),
    Stop(Request),
    StepDown(Request),
    Reparent(Reparent),
    /// Protocol 3.
    SetLayout(SetLayout),
    /// Protocol 4.
    SpawnAgent(SpawnAgent),
    /// Protocol 4.
    AgentPrompt(AgentPrompt),
    /// Protocol 4: cancel the running turn.
    AgentCancel(SessionRef),
    /// Protocol 4.
    AgentPermission(AgentPermission),
    /// Protocol 4: discard the queue slot's prompt.
    AgentUnqueue(AgentRequest),
    /// Protocol 4: send a held prompt again.
    AgentResend(AgentRequest),
    /// Protocol 4: run an ended agent again to continue its conversation.
    AgentRestart(AgentRestart),
    /// Protocol 4: log in an agent that needs a login.
    AgentLogin(AgentLogin),
    /// Protocol 4: fetch entries older than the client holds.
    AgentFetch(AgentFetch),
    /// Protocol 4: list the agent's earlier conversations.
    AgentList(AgentList),
    /// Protocol 4: end a choosing agent's wait.
    AgentChoose(AgentChoose),
    /// Protocol 4: set one of the session's options.
    AgentSetOption(AgentSetOption),
}

impl ToCore {
    pub fn encode(&self) -> io::Result<Frame> {
        match self {
            Self::Hello(m) => wire::json(MSG_HELLO, m),
            Self::Spawn(m) => wire::json(MSG_SPAWN, m),
            Self::Data(m) => wire::session_bytes(MSG_DATA, m.session, &m.bytes),
            Self::Resize(m) => wire::json(MSG_RESIZE, m),
            Self::Close(m) => wire::json(MSG_CLOSE, m),
            Self::Attach(m) => wire::json(MSG_ATTACH, m),
            Self::Detach(m) => wire::json(MSG_DETACH, m),
            Self::List(m) => wire::json(MSG_LIST, m),
            Self::Kill(m) => wire::json(MSG_KILL, m),
            Self::Status(m) => wire::json(MSG_STATUS, m),
            Self::Stop(m) => wire::json(MSG_STOP, m),
            Self::StepDown(m) => wire::json(MSG_STEP_DOWN, m),
            Self::Reparent(m) => wire::json(MSG_REPARENT, m),
            Self::SetLayout(m) => wire::json(MSG_SET_LAYOUT, m),
            Self::SpawnAgent(m) => wire::json(MSG_SPAWN_AGENT, m),
            Self::AgentPrompt(m) => wire::json(MSG_AGENT_PROMPT, m),
            Self::AgentCancel(m) => wire::json(MSG_AGENT_CANCEL, m),
            Self::AgentPermission(m) => wire::json(MSG_AGENT_PERMISSION, m),
            Self::AgentUnqueue(m) => wire::json(MSG_AGENT_UNQUEUE, m),
            Self::AgentResend(m) => wire::json(MSG_AGENT_RESEND, m),
            Self::AgentRestart(m) => wire::json(MSG_AGENT_RESTART, m),
            Self::AgentLogin(m) => wire::json(MSG_AGENT_LOGIN, m),
            Self::AgentFetch(m) => wire::json(MSG_AGENT_FETCH, m),
            Self::AgentList(m) => wire::json(MSG_AGENT_LIST, m),
            Self::AgentChoose(m) => wire::json(MSG_AGENT_CHOOSE, m),
            Self::AgentSetOption(m) => wire::json(MSG_AGENT_SET_OPTION, m),
        }
    }

    pub fn decode(frame: &Frame) -> io::Result<Self> {
        Ok(match frame.msg_type {
            MSG_HELLO => Self::Hello(wire::from_json("HELLO", frame)?),
            MSG_SPAWN => Self::Spawn(wire::from_json("SPAWN", frame)?),
            MSG_DATA => Self::Data(decode_data(frame)?),
            MSG_RESIZE => Self::Resize(wire::from_json("RESIZE", frame)?),
            MSG_CLOSE => Self::Close(wire::from_json("CLOSE", frame)?),
            MSG_ATTACH => Self::Attach(wire::from_json("ATTACH", frame)?),
            MSG_DETACH => Self::Detach(wire::from_json("DETACH", frame)?),
            MSG_LIST => Self::List(wire::from_json("LIST", frame)?),
            MSG_KILL => Self::Kill(wire::from_json("KILL", frame)?),
            MSG_STATUS => Self::Status(wire::from_json("STATUS", frame)?),
            MSG_STOP => Self::Stop(wire::from_json("STOP", frame)?),
            MSG_STEP_DOWN => Self::StepDown(wire::from_json("STEP_DOWN", frame)?),
            MSG_REPARENT => Self::Reparent(wire::from_json("REPARENT", frame)?),
            MSG_SET_LAYOUT => Self::SetLayout(wire::from_json("SET_LAYOUT", frame)?),
            MSG_SPAWN_AGENT => Self::SpawnAgent(wire::from_json("SPAWN_AGENT", frame)?),
            MSG_AGENT_PROMPT => Self::AgentPrompt(wire::from_json("AGENT_PROMPT", frame)?),
            MSG_AGENT_CANCEL => Self::AgentCancel(wire::from_json("AGENT_CANCEL", frame)?),
            MSG_AGENT_PERMISSION => {
                Self::AgentPermission(wire::from_json("AGENT_PERMISSION", frame)?)
            }
            MSG_AGENT_UNQUEUE => Self::AgentUnqueue(wire::from_json("AGENT_UNQUEUE", frame)?),
            MSG_AGENT_RESEND => Self::AgentResend(wire::from_json("AGENT_RESEND", frame)?),
            MSG_AGENT_RESTART => Self::AgentRestart(wire::from_json("AGENT_RESTART", frame)?),
            MSG_AGENT_LOGIN => Self::AgentLogin(wire::from_json("AGENT_LOGIN", frame)?),
            MSG_AGENT_FETCH => Self::AgentFetch(wire::from_json("AGENT_FETCH", frame)?),
            MSG_AGENT_LIST => Self::AgentList(wire::from_json("AGENT_LIST", frame)?),
            MSG_AGENT_CHOOSE => Self::AgentChoose(wire::from_json("AGENT_CHOOSE", frame)?),
            MSG_AGENT_SET_OPTION => {
                Self::AgentSetOption(wire::from_json("AGENT_SET_OPTION", frame)?)
            }
            other => return Err(wire::unknown_type("client-to-core", other)),
        })
    }
}

/// Messages from the core to a client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToClient {
    Welcome(Welcome),
    Error(ErrorMessage),
    Spawned(Spawned),
    /// Output from a session, live or within a replay.
    Data(Data),
    Exit(Exit),
    ReplayStart(ReplayStart),
    ReplayEnd(SessionRef),
    Sessions(Sessions),
    ServerStatus(ServerStatus),
    Ok(Request),
    /// Protocol 2, and only when the client's `HELLO` asked for it.
    Process(Process),
    /// Protocol 4: an agent session's transcript events, live or within a
    /// replay.
    Transcript(Transcript),
    /// Protocol 4: a frame of an `AGENT_FETCH`'s answer.
    AgentPage(AgentPage),
    /// Protocol 4: the answer to an `AGENT_LIST`.
    AgentConversations(AgentConversations),
}

impl ToClient {
    pub fn encode(&self) -> io::Result<Frame> {
        match self {
            Self::Welcome(m) => wire::json(MSG_WELCOME, m),
            Self::Error(m) => wire::json(MSG_ERROR, m),
            Self::Spawned(m) => wire::json(MSG_SPAWNED, m),
            Self::Data(m) => wire::session_bytes(MSG_DATA, m.session, &m.bytes),
            Self::Exit(m) => wire::json(MSG_EXIT, m),
            Self::ReplayStart(m) => wire::json(MSG_REPLAY_START, m),
            Self::ReplayEnd(m) => wire::json(MSG_REPLAY_END, m),
            Self::Sessions(m) => wire::json(MSG_SESSIONS, m),
            Self::ServerStatus(m) => wire::json(MSG_SERVER_STATUS, m),
            Self::Ok(m) => wire::json(MSG_OK, m),
            Self::Process(m) => wire::json(MSG_PROCESS, m),
            Self::Transcript(m) => {
                m.check()
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
                wire::json(MSG_TRANSCRIPT, m)
            }
            Self::AgentPage(m) => {
                m.check()
                    .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
                wire::json(MSG_AGENT_PAGE, m)
            }
            Self::AgentConversations(m) => wire::json(MSG_AGENT_CONVERSATIONS, m),
        }
    }

    pub fn decode(frame: &Frame) -> io::Result<Self> {
        Ok(match frame.msg_type {
            MSG_WELCOME => Self::Welcome(wire::from_json("WELCOME", frame)?),
            MSG_ERROR => Self::Error(wire::from_json("ERROR", frame)?),
            MSG_SPAWNED => Self::Spawned(wire::from_json("SPAWNED", frame)?),
            MSG_DATA => Self::Data(decode_data(frame)?),
            MSG_EXIT => Self::Exit(wire::from_json("EXIT", frame)?),
            MSG_REPLAY_START => Self::ReplayStart(wire::from_json("REPLAY_START", frame)?),
            MSG_REPLAY_END => Self::ReplayEnd(wire::from_json("REPLAY_END", frame)?),
            MSG_SESSIONS => Self::Sessions(wire::from_json("SESSIONS", frame)?),
            MSG_SERVER_STATUS => Self::ServerStatus(wire::from_json("SERVER_STATUS", frame)?),
            MSG_OK => Self::Ok(wire::from_json("OK", frame)?),
            MSG_PROCESS => Self::Process(wire::from_json("PROCESS", frame)?),
            MSG_TRANSCRIPT => {
                let transcript: Transcript = wire::from_json("TRANSCRIPT", frame)?;
                transcript.check()?;
                Self::Transcript(transcript)
            }
            MSG_AGENT_PAGE => {
                let page: AgentPage = wire::from_json("AGENT_PAGE", frame)?;
                page.check()?;
                Self::AgentPage(page)
            }
            MSG_AGENT_CONVERSATIONS => {
                Self::AgentConversations(wire::from_json("AGENT_CONVERSATIONS", frame)?)
            }
            other => return Err(wire::unknown_type("core-to-client", other)),
        })
    }
}

fn decode_data(frame: &Frame) -> io::Result<Data> {
    let (session, bytes) = wire::split_session("DATA", frame)?;
    Ok(Data {
        session,
        bytes: bytes.to_vec(),
    })
}

/// The core's answer to a `HELLO`: the version both speak, or the `ERROR`
/// to send back (naming both ranges) before closing the connection.
pub fn answer_hello(hello: &Hello, supported: VersionRange) -> Result<u32, ErrorMessage> {
    negotiate(hello.range(), supported).map_err(|error| ErrorMessage {
        req: None,
        session: None,
        message: error.to_string(),
    })
}

#[cfg(test)]
pub(crate) mod samples {
    use super::*;

    pub fn to_core() -> Vec<ToCore> {
        vec![
            ToCore::Hello(Hello {
                min: 1,
                max: 1,
                client_version: "0.3.0".into(),
                client_token: "4f1c9a".into(),
                ..Default::default()
            }),
            ToCore::Hello(Hello {
                min: 1,
                max: 2,
                client_version: "0.4.0".into(),
                client_token: "4f1c9b".into(),
                linger_secs: 300,
                process_info: true,
            }),
            ToCore::Spawn(Spawn {
                req: 1,
                command: Some("/bin/bash".into()),
                args: vec!["-l".into()],
                cwd: Some("/home/u".into()),
                env: BTreeMap::from([("TERM".into(), "xterm-256color".into())]),
                cols: 80,
                rows: 24,
                cell_w: 9,
                cell_h: 18,
                scrollback: 10_000,
                policy: Policy::Detach,
                profile: Some("Shell".into()),
            }),
            ToCore::Data(Data {
                session: 7,
                bytes: b"ls\r\0\xff".to_vec(),
            }),
            ToCore::Data(Data {
                session: u32::MAX,
                bytes: Vec::new(),
            }),
            ToCore::Resize(Resize {
                session: 7,
                cols: 120,
                rows: 40,
                cell_w: 10,
                cell_h: 20,
            }),
            ToCore::Close(SessionRef { session: 7 }),
            ToCore::Attach(Attach {
                req: 2,
                session: 3,
                cols: 100,
                rows: 30,
                page: None,
            }),
            ToCore::Attach(Attach {
                req: 3,
                session: 9,
                cols: 0,
                rows: 0,
                page: Some(16_384),
            }),
            ToCore::Detach(Detach {
                session: 3,
                layout: None,
            }),
            ToCore::Detach(Detach {
                session: 4,
                layout: Some(serde_json::json!({"v": 1, "zones": {"key": "k4", "active": true}})),
            }),
            ToCore::List(Request { req: 3 }),
            ToCore::Kill(Kill {
                req: 4,
                target: KillTarget::Session(3),
            }),
            ToCore::Kill(Kill {
                req: 5,
                target: KillTarget::All,
            }),
            ToCore::Status(Request { req: 6 }),
            ToCore::Stop(Request { req: 7 }),
            ToCore::StepDown(Request { req: 8 }),
            ToCore::Reparent(Reparent {
                session: 7,
                hwnd: 0x0001_0a2c,
            }),
            ToCore::SetLayout(SetLayout {
                session: 7,
                layout: Some(serde_json::json!({"v": 1, "zones": {"key": "k7"}})),
            }),
            ToCore::SetLayout(SetLayout {
                session: 8,
                layout: None,
            }),
            ToCore::SpawnAgent(SpawnAgent {
                req: 9,
                command: "npx".into(),
                args: vec!["@agentclientprotocol/claude-agent-acp".into()],
                cwd: Some("~/src".into()),
                env: BTreeMap::from([("HOME".into(), "/home/u".into())]),
                policy: Policy::Detach,
                profile: Some("Claude".into()),
                conversation: None,
                start: None,
            }),
            ToCore::SpawnAgent(SpawnAgent {
                req: 15,
                command: "copilot".into(),
                args: vec!["--acp".into()],
                cwd: None,
                env: BTreeMap::new(),
                policy: Policy::Kill,
                profile: Some("Copilot".into()),
                conversation: Some(AgentConversation {
                    id: "sess-4f1c".into(),
                    cwd: "/home/u/src".into(),
                }),
                start: None,
            }),
            ToCore::AgentPrompt(AgentPrompt {
                req: 10,
                session: 9,
                text: "Write notes.md\n".into(),
            }),
            ToCore::AgentCancel(SessionRef { session: 9 }),
            ToCore::AgentPermission(AgentPermission {
                req: 11,
                session: 9,
                entry: 4,
                option: Some("allow-once".into()),
            }),
            ToCore::AgentPermission(AgentPermission {
                req: 12,
                session: 9,
                entry: 5,
                option: None,
            }),
            ToCore::AgentUnqueue(AgentRequest {
                req: 13,
                session: 9,
            }),
            ToCore::AgentResend(AgentRequest {
                req: 14,
                session: 9,
            }),
            ToCore::AgentRestart(AgentRestart {
                req: 16,
                session: 9,
                command: "npx".into(),
                args: vec!["@agentclientprotocol/claude-agent-acp".into()],
                env: BTreeMap::from([("HOME".into(), "/home/u".into())]),
            }),
            ToCore::AgentLogin(AgentLogin {
                req: 17,
                session: 9,
                method: "oauth-personal".into(),
            }),
            ToCore::AgentFetch(AgentFetch {
                req: 18,
                session: 9,
                before: 120,
                max_bytes: 262_144,
            }),
            ToCore::AgentList(AgentList {
                req: 19,
                session: 9,
                cursor: None,
            }),
            ToCore::AgentList(AgentList {
                req: 20,
                session: 9,
                cursor: Some("page-2".into()),
            }),
            ToCore::AgentChoose(AgentChoose {
                req: 21,
                session: 9,
                conversation: Some("sess-9a".into()),
            }),
            ToCore::AgentChoose(AgentChoose {
                req: 22,
                session: 9,
                conversation: None,
            }),
            ToCore::AgentSetOption(AgentSetOption {
                req: 23,
                session: 9,
                option: "model".into(),
                value: OptionValue::Choice {
                    value: "sonnet".into(),
                },
            }),
            ToCore::AgentSetOption(AgentSetOption {
                req: 24,
                session: 9,
                option: "fast".into(),
                value: OptionValue::Toggle { on: true },
            }),
            ToCore::SpawnAgent(SpawnAgent {
                req: 23,
                command: "fake-acp-agent".into(),
                start: Some(SpawnStart::Choose),
                ..Default::default()
            }),
        ]
    }

    pub fn to_client() -> Vec<ToClient> {
        vec![
            ToClient::Welcome(Welcome {
                protocol: 1,
                core_version: "0.3.0".into(),
                core_build: "abc123".into(),
                pid: 4242,
                holder_protocols: vec![1],
            }),
            ToClient::Error(ErrorMessage {
                req: Some(2),
                session: Some(3),
                message: "session 3 is attached".into(),
            }),
            ToClient::Error(ErrorMessage {
                req: None,
                session: None,
                message: "no common protocol version".into(),
            }),
            ToClient::Spawned(Spawned {
                req: 1,
                session: 7,
                pid: 5000,
                key: "9f2c41d07a5b3e86c1d4f0a2b7e95c13".into(),
            }),
            ToClient::Data(Data {
                session: 7,
                bytes: b"\x1b[1mhi\x1b[m\r\n".to_vec(),
            }),
            ToClient::Exit(Exit {
                session: 7,
                code: Some(3),
                lost: None,
            }),
            ToClient::Exit(Exit {
                session: 8,
                code: None,
                lost: None,
            }),
            ToClient::Exit(Exit {
                session: 9,
                code: None,
                lost: Some("its holder ended".into()),
            }),
            ToClient::ReplayStart(ReplayStart {
                req: Some(2),
                session: 3,
                pid: 5001,
            }),
            ToClient::ReplayEnd(SessionRef { session: 3 }),
            ToClient::Sessions(Sessions {
                req: 3,
                sessions: vec![SessionInfo {
                    session: 3,
                    pid: 5001,
                    attached: false,
                    command: "htop".into(),
                    cwd: Some("/tmp".into()),
                    policy: Policy::Detach,
                    cols: 100,
                    rows: 30,
                    key: "9f2c41d07a5b3e86c1d4f0a2b7e95c13".into(),
                    title: "htop".into(),
                    started: 1_794_000_000,
                    profile: Some("Shell".into()),
                    owned: true,
                    detached_secs: 42,
                    layout: Some(serde_json::json!({"v": 1, "zones": {"key": "k3"}})),
                    agent: None,
                }],
            }),
            ToClient::Sessions(Sessions {
                req: 4,
                sessions: vec![SessionInfo {
                    session: 9,
                    pid: 5009,
                    command: "npx @agentclientprotocol/claude-agent-acp".into(),
                    cwd: Some("/home/u/src".into()),
                    policy: Policy::Detach,
                    agent: Some(AgentInfo {
                        activity: AgentActivity::Blocked,
                        name: Some("claude-agent-acp".into()),
                    }),
                    ..Default::default()
                }],
            }),
            ToClient::Transcript(Transcript {
                session: 9,
                events: vec![
                    serde_json::json!({"event": "added", "id": 1, "entry": {"type": "user", "text": "hi"}}),
                    serde_json::json!({"event": "turn", "state": {"turn": "working"}}),
                ],
            }),
            ToClient::Transcript(Transcript {
                session: 9,
                events: Vec::new(),
            }),
            ToClient::AgentPage(AgentPage {
                req: 18,
                session: 9,
                events: vec![
                    serde_json::json!({"event": "added", "id": 118, "entry": {"type": "user", "text": "hi"}}),
                ],
                last: None,
            }),
            ToClient::AgentPage(AgentPage {
                req: 18,
                session: 9,
                events: Vec::new(),
                last: Some(PageEnd { more: true }),
            }),
            ToClient::AgentConversations(AgentConversations {
                req: 19,
                session: 9,
                conversations: vec![
                    ListedConversation {
                        id: "sess-9a".into(),
                        title: Some("Fix the upload test".into()),
                        updated: Some("2026-11-26T18:05:00Z".into()),
                    },
                    ListedConversation {
                        id: "sess-7f".into(),
                        title: None,
                        updated: None,
                    },
                ],
                next: Some("page-2".into()),
            }),
            ToClient::AgentConversations(AgentConversations {
                req: 20,
                session: 9,
                conversations: Vec::new(),
                next: None,
            }),
            ToClient::ServerStatus(ServerStatus {
                req: 6,
                core_version: "0.3.0".into(),
                core_build: "abc123".into(),
                protocols: PROTOCOLS,
                holder_protocols: VersionRange::new(1, 1),
                pid: 4242,
                uptime_secs: 61,
                clients: 2,
                holders: vec![HolderStatus {
                    pid: 4243,
                    build: "abc123".into(),
                    protocol: 1,
                    sessions: 2,
                    draining: false,
                }],
            }),
            ToClient::Ok(Request { req: 7 }),
            ToClient::Process(Process {
                session: 7,
                name: Some("vim".into()),
                cwd: Some("/home/u/src".into()),
            }),
            ToClient::Process(Process {
                session: 8,
                name: None,
                cwd: None,
            }),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::samples::*;
    use super::*;

    fn roundtrip_wire<T>(message: &T, encode: impl Fn(&T) -> io::Result<Frame>) -> Frame {
        let frame = encode(message).unwrap();
        let mut bytes = Vec::new();
        frame.write_to(&mut bytes).unwrap();
        Frame::read_from(&mut bytes.as_slice()).unwrap()
    }

    #[test]
    fn every_client_message_roundtrips() {
        let messages = to_core();
        let mut types: Vec<u8> = Vec::new();
        for message in &messages {
            let frame = roundtrip_wire(message, ToCore::encode);
            types.push(frame.msg_type);
            assert_eq!(&ToCore::decode(&frame).unwrap(), message);
        }
        for expected in (MSG_DATA..=MSG_AGENT_SET_OPTION).chain([MSG_AGENT_LIST, MSG_AGENT_CHOOSE])
        {
            assert!(types.contains(&expected), "no sample of type {expected}");
        }
    }

    #[test]
    fn every_core_message_roundtrips() {
        let mut types: Vec<u8> = Vec::new();
        for message in &to_client() {
            let frame = roundtrip_wire(message, ToClient::encode);
            types.push(frame.msg_type);
            assert_eq!(&ToClient::decode(&frame).unwrap(), message);
        }
        assert!(types.contains(&MSG_DATA));
        for expected in (MSG_WELCOME..=MSG_AGENT_PAGE).chain([MSG_AGENT_CONVERSATIONS]) {
            assert!(types.contains(&expected), "no sample of type {expected}");
        }
    }

    #[test]
    fn data_starts_with_the_session_id() {
        let frame = ToClient::Data(Data {
            session: 0x0000_0102,
            bytes: b"ok".to_vec(),
        })
        .encode()
        .unwrap();
        assert_eq!(frame.msg_type, MSG_DATA);
        assert_eq!(frame.payload, [0, 0, 1, 2, b'o', b'k']);

        let short = Frame::new(MSG_DATA, vec![0, 0, 1]).unwrap();
        assert_eq!(
            ToCore::decode(&short).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        assert!(ToClient::decode(&short).is_err());
    }

    #[test]
    fn json_messages_use_the_documented_field_names() {
        let frame = ToCore::Kill(Kill {
            req: 4,
            target: KillTarget::Session(3),
        })
        .encode()
        .unwrap();
        assert_eq!(frame.payload, br#"{"req":4,"target":{"session":3}}"#);
        let frame = ToCore::Kill(Kill {
            req: 5,
            target: KillTarget::All,
        })
        .encode()
        .unwrap();
        assert_eq!(frame.payload, br#"{"req":5,"target":"all"}"#);
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let frame = Frame::new(
            MSG_HELLO,
            br#"{"min":1,"max":2,"client_version":"9","client_token":"t","future":{"x":[1]}}"#
                .to_vec(),
        )
        .unwrap();
        assert_eq!(
            ToCore::decode(&frame).unwrap(),
            ToCore::Hello(Hello {
                min: 1,
                max: 2,
                client_version: "9".into(),
                client_token: "t".into(),
                ..Default::default()
            })
        );
        let frame = Frame::new(MSG_OK, br#"{"req":9,"note":"later"}"#.to_vec()).unwrap();
        assert_eq!(
            ToClient::decode(&frame).unwrap(),
            ToClient::Ok(Request { req: 9 })
        );
    }

    #[test]
    fn missing_optional_fields_take_defaults() {
        let frame = Frame::new(MSG_SPAWN, br#"{"req":1,"cols":80,"rows":24}"#.to_vec()).unwrap();
        assert_eq!(
            ToCore::decode(&frame).unwrap(),
            ToCore::Spawn(Spawn {
                req: 1,
                command: None,
                args: Vec::new(),
                cwd: None,
                env: BTreeMap::new(),
                cols: 80,
                rows: 24,
                cell_w: 0,
                cell_h: 0,
                scrollback: 0,
                policy: Policy::Kill,
                profile: None,
            })
        );
        let frame = Frame::new(MSG_ERROR, br#"{"message":"m"}"#.to_vec()).unwrap();
        assert_eq!(
            ToClient::decode(&frame).unwrap(),
            ToClient::Error(ErrorMessage {
                req: None,
                session: None,
                message: "m".into(),
            })
        );
        let frame = Frame::new(MSG_REPLAY_START, br#"{"session":2,"pid":9}"#.to_vec()).unwrap();
        assert_eq!(
            ToClient::decode(&frame).unwrap(),
            ToClient::ReplayStart(ReplayStart {
                req: None,
                session: 2,
                pid: 9,
            })
        );
    }

    #[test]
    fn required_fields_and_bad_json_are_invalid_data() {
        for (msg_type, payload) in [
            (MSG_SPAWN, &br#"{"cols":80,"rows":24}"#[..]),
            (MSG_RESIZE, br#"{"cols":80,"rows":24}"#),
            (MSG_KILL, br#"{"req":1,"target":"some"}"#),
            (MSG_HELLO, br#"{"min":-1,"max":1,"client_token":"t"}"#),
            (MSG_LIST, b""),
            (MSG_LIST, b"\"req\""),
            (MSG_LIST, br#"{"req":"1"}"#),
            (MSG_STOP, b"{\"req\":1"),
        ] {
            let frame = Frame::new(msg_type, payload.to_vec()).unwrap();
            let error = ToCore::decode(&frame).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{error}");
        }
    }

    #[test]
    fn unknown_and_wrong_direction_types_are_invalid_data() {
        for msg_type in [0, 23, 30, MSG_WELCOME, MSG_OK, MSG_PROCESS, MSG_TRANSCRIPT, 43, 45, 63, 65, 255] {
            let frame = Frame::new(msg_type, b"{}".to_vec()).unwrap();
            let error = ToCore::decode(&frame).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        }
        for msg_type in [
            0,
            MSG_HELLO,
            MSG_REPARENT,
            MSG_SPAWN_AGENT,
            MSG_AGENT_RESEND,
            MSG_AGENT_RESTART,
            MSG_AGENT_LOGIN,
            MSG_AGENT_LIST,
            MSG_AGENT_CHOOSE,
            MSG_AGENT_SET_OPTION,
            43,
            44,
            255,
        ] {
            let frame = Frame::new(msg_type, b"{}".to_vec()).unwrap();
            assert!(ToClient::decode(&frame).is_err());
        }
    }

    #[test]
    fn hello_is_answered_with_the_highest_common_version() {
        let hello = |min, max| Hello {
            min,
            max,
            client_token: "t".into(),
            ..Default::default()
        };
        assert_eq!(answer_hello(&hello(1, 1), PROTOCOLS), Ok(1));
        assert_eq!(answer_hello(&hello(1, 2), PROTOCOLS), Ok(2));
        assert_eq!(answer_hello(&hello(1, 4), VersionRange::new(1, 3)), Ok(3));
        assert_eq!(answer_hello(&hello(1, 3), PROTOCOLS), Ok(3));
        assert_eq!(answer_hello(&hello(1, 4), PROTOCOLS), Ok(4));
        let error = answer_hello(&hello(5, 6), PROTOCOLS).unwrap_err();
        assert!(error.message.contains("5–6"), "{}", error.message);
        assert!(error.message.contains("supported 1–4"), "{}", error.message);
        assert_eq!(error.req, None);
        // A core that still speaks only protocol 1 refuses a client that
        // speaks only 2.
        let error = answer_hello(&hello(2, 2), VersionRange::new(1, 1)).unwrap_err();
        assert!(error.message.contains("supported 1"), "{}", error.message);
    }

    /// A client that speaks 1–3 and a core of the previous release (1–2)
    /// agree on 2: the client then sends no `SET_LAYOUT`.
    #[test]
    fn a_protocol_3_client_and_a_protocol_2_core_agree_on_2() {
        let client = Hello {
            min: 1,
            max: 3,
            client_token: "t".into(),
            ..Default::default()
        };
        assert_eq!(answer_hello(&client, VersionRange::new(1, 2)), Ok(2));
        assert_eq!(answer_hello(&client, PROTOCOLS), Ok(3));
    }

    /// A client that speaks 1–4 and a core of the previous release (1–3)
    /// agree on 3: the client then starts no agent session.
    #[test]
    fn a_protocol_4_client_and_a_protocol_3_core_agree_on_3() {
        let client = Hello {
            min: PROTOCOLS.min,
            max: PROTOCOLS.max,
            client_token: "t".into(),
            ..Default::default()
        };
        assert_eq!(answer_hello(&client, VersionRange::new(1, 3)), Ok(3));
        assert_eq!(answer_hello(&client, PROTOCOLS), Ok(4));
    }

    #[test]
    fn agent_messages_use_the_documented_field_names() {
        let frame = ToCore::AgentPermission(AgentPermission {
            req: 3,
            session: 2,
            entry: 7,
            option: None,
        })
        .encode()
        .unwrap();
        assert_eq!(frame.payload, br#"{"req":3,"session":2,"entry":7}"#);
        let frame = ToCore::SpawnAgent(SpawnAgent {
            req: 1,
            command: "agent".into(),
            ..Default::default()
        })
        .encode()
        .unwrap();
        assert_eq!(
            frame.payload,
            br#"{"req":1,"command":"agent","args":[],"cwd":null,"env":{},"policy":"kill"}"#
        );
        // A SPAWN_AGENT without a command decodes, for the core to refuse
        // with a reason.
        let frame = Frame::new(MSG_SPAWN_AGENT, br#"{"req":1}"#.to_vec()).unwrap();
        let ToCore::SpawnAgent(spawn) = ToCore::decode(&frame).unwrap() else {
            panic!("not SPAWN_AGENT");
        };
        assert_eq!(spawn.command, "");
        assert_eq!(spawn.conversation, None);
        let frame = ToCore::SpawnAgent(SpawnAgent {
            req: 1,
            command: "agent".into(),
            conversation: Some(AgentConversation {
                id: "s1".into(),
                cwd: "/w".into(),
            }),
            ..Default::default()
        })
        .encode()
        .unwrap();
        assert_eq!(
            frame.payload,
            br#"{"req":1,"command":"agent","args":[],"cwd":null,"env":{},"policy":"kill","conversation":{"id":"s1","cwd":"/w"}}"#
        );
        let frame = ToCore::AgentRestart(AgentRestart {
            req: 2,
            session: 5,
            command: "agent".into(),
            ..Default::default()
        })
        .encode()
        .unwrap();
        assert_eq!(frame.msg_type, 21);
        assert_eq!(
            frame.payload,
            br#"{"req":2,"session":5,"command":"agent","args":[],"env":{}}"#
        );
        // An AGENT_RESTART without a command decodes, for the core to refuse
        // with a reason; without a session it doesn't.
        let frame = Frame::new(MSG_AGENT_RESTART, br#"{"req":2,"session":5}"#.to_vec()).unwrap();
        let ToCore::AgentRestart(restart) = ToCore::decode(&frame).unwrap() else {
            panic!("not AGENT_RESTART");
        };
        assert_eq!(restart.command, "");
        let frame = Frame::new(MSG_AGENT_RESTART, br#"{"req":2}"#.to_vec()).unwrap();
        assert!(ToCore::decode(&frame).is_err());
        // AGENT_LOGIN is type 22 and names its method; without one it
        // doesn't decode.
        let frame = ToCore::AgentLogin(AgentLogin {
            req: 3,
            session: 5,
            method: "chat-gpt".into(),
        })
        .encode()
        .unwrap();
        assert_eq!(frame.msg_type, 22);
        assert_eq!(
            frame.payload,
            br#"{"req":3,"session":5,"method":"chat-gpt"}"#
        );
        for payload in [
            &br#"{"req":3,"session":5}"#[..],
            br#"{"req":3,"method":"m"}"#,
            br#"{"session":5,"method":"m"}"#,
            br#"{"req":3,"session":5,"method":7}"#,
        ] {
            let frame = Frame::new(MSG_AGENT_LOGIN, payload.to_vec()).unwrap();
            assert!(ToCore::decode(&frame).is_err());
        }
        // AGENT_SET_OPTION is type 24, its value tagged by kind; a missing
        // field, a wrong type or an unknown kind doesn't decode.
        let frame = ToCore::AgentSetOption(AgentSetOption {
            req: 4,
            session: 5,
            option: "mode".into(),
            value: OptionValue::Choice {
                value: "plan".into(),
            },
        })
        .encode()
        .unwrap();
        assert_eq!(frame.msg_type, 24);
        assert_eq!(
            frame.payload,
            br#"{"req":4,"session":5,"option":"mode","value":{"kind":"choice","value":"plan"}}"#
        );
        let frame = ToCore::AgentSetOption(AgentSetOption {
            req: 4,
            session: 5,
            option: "fast".into(),
            value: OptionValue::Toggle { on: false },
        })
        .encode()
        .unwrap();
        assert_eq!(
            frame.payload,
            br#"{"req":4,"session":5,"option":"fast","value":{"kind":"toggle","on":false}}"#
        );
        for payload in [
            &br#"{"req":4,"session":5,"option":"mode"}"#[..],
            br#"{"req":4,"session":5,"value":{"kind":"toggle","on":true}}"#,
            br#"{"session":5,"option":"m","value":{"kind":"toggle","on":true}}"#,
            br#"{"req":4,"session":5,"option":7,"value":{"kind":"toggle","on":true}}"#,
            br#"{"req":4,"session":5,"option":"m","value":{"kind":"toggle","on":"yes"}}"#,
            br#"{"req":4,"session":5,"option":"m","value":{"kind":"choice"}}"#,
            br#"{"req":4,"session":5,"option":"m","value":{"kind":"number","value":3}}"#,
            br#"{"req":4,"session":5,"option":"m","value":"plan"}"#,
        ] {
            let frame = Frame::new(MSG_AGENT_SET_OPTION, payload.to_vec()).unwrap();
            assert!(ToCore::decode(&frame).is_err(), "{payload:?}");
        }
        // A conversation without its id is refused.
        let frame = Frame::new(
            MSG_SPAWN_AGENT,
            br#"{"req":1,"command":"a","conversation":{"cwd":"/w"}}"#.to_vec(),
        )
        .unwrap();
        assert!(ToCore::decode(&frame).is_err());
        for (activity, name) in [
            (AgentActivity::Idle, "idle"),
            (AgentActivity::Working, "working"),
            (AgentActivity::Blocked, "blocked"),
            (AgentActivity::Done, "done"),
            (AgentActivity::Error, "error"),
        ] {
            assert_eq!(
                serde_json::to_string(&activity).unwrap(),
                format!("\"{name}\"")
            );
        }
    }

    #[test]
    fn a_session_list_without_agents_encodes_as_before() {
        let info = SessionInfo {
            session: 1,
            pid: 2,
            ..Default::default()
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(!json.contains("agent"), "{json}");
        let info = SessionInfo {
            agent: Some(AgentInfo {
                activity: AgentActivity::Done,
                name: None,
            }),
            ..info
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.ends_with(r#""agent":{"activity":"done"}}"#), "{json}");
        let frame = Frame::new(
            MSG_SESSIONS,
            br#"{"req":1,"sessions":[{"session":1,"pid":2,"agent":{"activity":"asleep"}}]}"#.to_vec(),
        )
        .unwrap();
        assert!(ToClient::decode(&frame).is_err(), "an unknown activity");
    }

    #[test]
    fn transcript_events_must_be_objects() {
        for payload in [
            &br#"{"session":1,"events":[{"event":"turn"},7]}"#[..],
            br#"{"session":1,"events":["added"]}"#,
            br#"{"session":1,"events":[null]}"#,
            br#"{"session":1,"events":{"event":"turn"}}"#,
            br#"{"events":[]}"#,
        ] {
            let frame = Frame::new(MSG_TRANSCRIPT, payload.to_vec()).unwrap();
            let error = ToClient::decode(&frame).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData, "{error}");
        }
        let error = ToClient::Transcript(Transcript {
            session: 1,
            events: vec![serde_json::json!([1])],
        })
        .encode()
        .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        // What the crate doesn't read it keeps as sent.
        let frame = Frame::new(
            MSG_TRANSCRIPT,
            br#"{"session":1,"events":[{"later":{"x":[1]}}]}"#.to_vec(),
        )
        .unwrap();
        let ToClient::Transcript(transcript) = ToClient::decode(&frame).unwrap() else {
            panic!("not TRANSCRIPT");
        };
        assert_eq!(transcript.events, [serde_json::json!({"later": {"x": [1]}})]);
    }

    #[test]
    fn paging_messages_use_the_documented_field_names() {
        let attach = |page| {
            ToCore::Attach(Attach {
                req: 2,
                session: 3,
                cols: 0,
                rows: 0,
                page,
            })
            .encode()
            .unwrap()
            .payload
        };
        assert_eq!(attach(None), br#"{"req":2,"session":3,"cols":0,"rows":0}"#, "as before");
        assert_eq!(
            attach(Some(4096)),
            br#"{"req":2,"session":3,"cols":0,"rows":0,"page":4096}"#
        );
        let frame = ToCore::AgentFetch(AgentFetch {
            req: 5,
            session: 3,
            before: 40,
            max_bytes: 1024,
        })
        .encode()
        .unwrap();
        assert_eq!(frame.msg_type, 23);
        assert_eq!(
            frame.payload,
            br#"{"req":5,"session":3,"before":40,"max_bytes":1024}"#
        );
        let page = |last| {
            ToClient::AgentPage(AgentPage {
                req: 5,
                session: 3,
                events: Vec::new(),
                last,
            })
            .encode()
            .unwrap()
        };
        assert_eq!(page(None).msg_type, 43);
        assert_eq!(page(None).payload, br#"{"req":5,"session":3,"events":[]}"#);
        assert_eq!(
            page(Some(PageEnd { more: false })).payload,
            br#"{"req":5,"session":3,"events":[],"last":{"more":false}}"#
        );
        // A page's events must be objects, both ways.
        let bad = AgentPage {
            req: 5,
            session: 3,
            events: vec![serde_json::json!(1)],
            last: None,
        };
        assert_eq!(
            ToClient::AgentPage(bad).encode().unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        let frame = Frame::new(MSG_AGENT_PAGE, br#"{"req":5,"session":3,"events":[1]}"#.to_vec()).unwrap();
        assert_eq!(
            ToClient::decode(&frame).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn a_page_is_packed_into_frames_of_whole_events_with_last_on_the_final_one() {
        // A 3 MiB page: 48 events of 64 KiB.
        let event = |n: usize| serde_json::json!({"event": "appended", "id": n, "text": "x".repeat(64 * 1024)});
        let events: Vec<serde_json::Value> = (0..48).map(event).collect();
        let packed = pack_page(u64::MAX, u32::MAX, events.clone(), true).unwrap();
        assert!(packed.len() >= 3, "3 MiB needs at least three frames");
        let mut unpacked = Vec::new();
        for (index, page) in packed.iter().enumerate() {
            assert_eq!((page.req, page.session), (u64::MAX, u32::MAX));
            assert!(!page.events.is_empty());
            let last = index + 1 == packed.len();
            assert_eq!(page.last, last.then_some(PageEnd { more: true }));
            let frame = ToClient::AgentPage(page.clone()).encode().unwrap();
            assert!(frame.payload.len() <= crate::MAX_FRAME_PAYLOAD_SIZE);
            unpacked.extend(page.events.clone());
        }
        assert_eq!(unpacked, events);
        // An empty page is one frame.
        assert_eq!(
            pack_page(1, 2, Vec::new(), false).unwrap(),
            [AgentPage {
                req: 1,
                session: 2,
                events: Vec::new(),
                last: Some(PageEnd { more: false }),
            }]
        );
        // The largest event that fits a frame on its own does, with the
        // longest ids and `last`.
        let fill = crate::MAX_FRAME_PAYLOAD_SIZE
            - br#"{"req":18446744073709551615,"session":4294967295,"events":[{"t":""}],"last":{"more":false}}"#.len();
        let largest = serde_json::json!({"t": "x".repeat(fill)});
        let packed = pack_page(u64::MAX, u32::MAX, vec![largest], false).unwrap();
        assert!(ToClient::AgentPage(packed[0].clone()).encode().is_ok());
        assert_eq!(
            pack_page(1, 2, vec![serde_json::json!("turn")], false).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
    }

    #[test]
    fn transcripts_are_packed_into_frames_of_whole_events() {
        let event = |n: usize| serde_json::json!({"event": "appended", "id": n, "text": "x".repeat(200 * 1024)});
        let events: Vec<serde_json::Value> = (0..12).map(event).collect();
        let packed = pack_transcript(4, events.clone()).unwrap();
        assert!(packed.len() > 1, "12 × 200 KiB needs several frames");
        let mut unpacked = Vec::new();
        for message in packed {
            assert_eq!(message.session, 4);
            assert!(!message.events.is_empty());
            let frame = ToClient::Transcript(message.clone()).encode().unwrap();
            assert!(frame.payload.len() <= crate::MAX_FRAME_PAYLOAD_SIZE);
            unpacked.extend(message.events);
        }
        assert_eq!(unpacked, events);
        assert!(pack_transcript(4, Vec::new()).unwrap().is_empty());
        // An event as large as a frame, or not an object, can't be sent.
        let huge = serde_json::json!({"text": "x".repeat(crate::MAX_FRAME_PAYLOAD_SIZE)});
        assert_eq!(
            pack_transcript(4, vec![huge]).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            pack_transcript(4, vec![serde_json::json!("turn")]).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        // The largest event that fits a frame on its own does.
        let fill = crate::MAX_FRAME_PAYLOAD_SIZE - br#"{"session":4294967295,"events":[{"t":""}]}"#.len();
        let largest = serde_json::json!({"t": "x".repeat(fill)});
        let packed = pack_transcript(u32::MAX, vec![largest]).unwrap();
        assert!(ToClient::Transcript(packed[0].clone()).encode().is_ok());
    }

    /// Each envelope constant equals the encoded length of an empty batch
    /// with the largest ids (OpenSpec change `agents-event-bounds`).
    #[test]
    fn each_envelope_is_an_empty_batchs_length_with_the_largest_ids() {
        let transcript = Transcript {
            session: u32::MAX,
            events: Vec::new(),
        };
        assert_eq!(serde_json::to_vec(&transcript).unwrap().len(), TRANSCRIPT_ENVELOPE);
        let page = AgentPage {
            req: u64::MAX,
            session: u32::MAX,
            events: Vec::new(),
            last: Some(PageEnd { more: false }),
        };
        assert_eq!(serde_json::to_vec(&page).unwrap().len(), AGENT_PAGE_ENVELOPE);
        const { assert!(AGENT_PAGE_ENVELOPE >= TRANSCRIPT_ENVELOPE) };
    }

    #[test]
    fn a_protocol_1_and_2_client_and_a_protocol_1_core_agree_on_1() {
        let client = Hello {
            min: PROTOCOLS.min,
            max: PROTOCOLS.max,
            client_token: "t".into(),
            linger_secs: 300,
            process_info: true,
            ..Default::default()
        };
        assert_eq!(answer_hello(&client, VersionRange::new(1, 1)), Ok(1));
        // And the reverse: a client that speaks only 1 gets 1 from this core.
        let old_client = Hello {
            min: 1,
            max: 1,
            client_token: "t".into(),
            ..Default::default()
        };
        assert_eq!(answer_hello(&old_client, PROTOCOLS), Ok(1));
    }

    #[test]
    fn old_hello_frames_decode_with_the_new_fields_defaulted() {
        // What a protocol-1 client sent before `linger_secs` and `process_info`.
        let frame = Frame::new(
            MSG_HELLO,
            br#"{"min":1,"max":1,"client_version":"0.3.0","client_token":"t"}"#.to_vec(),
        )
        .unwrap();
        let ToCore::Hello(hello) = ToCore::decode(&frame).unwrap() else {
            panic!("not a HELLO");
        };
        assert_eq!((hello.linger_secs, hello.process_info), (0, false));
        // And a HELLO that sets neither encodes exactly as before.
        let plain = ToCore::Hello(Hello {
            min: 1,
            max: 1,
            client_version: "0.3.0".into(),
            client_token: "t".into(),
            ..Default::default()
        })
        .encode()
        .unwrap();
        assert_eq!(plain.payload, frame.payload);
        let asking = ToCore::Hello(Hello {
            min: 1,
            max: 2,
            client_token: "t".into(),
            linger_secs: 300,
            process_info: true,
            ..Default::default()
        })
        .encode()
        .unwrap();
        assert_eq!(
            asking.payload,
            br#"{"min":1,"max":2,"client_version":"","client_token":"t","linger_secs":300,"process_info":true}"#
        );
    }

    #[test]
    fn old_sessions_frames_decode_with_the_new_fields_defaulted() {
        let frame = Frame::new(
            MSG_SESSIONS,
            br#"{"req":3,"sessions":[{"session":3,"pid":5001,"attached":false,"command":"htop","cwd":"/tmp","policy":"detach","cols":100,"rows":30,"key":"k","title":"htop","started":5,"profile":"Shell"}]}"#
                .to_vec(),
        )
        .unwrap();
        let ToClient::Sessions(sessions) = ToClient::decode(&frame).unwrap() else {
            panic!("not SESSIONS");
        };
        let info = &sessions.sessions[0];
        assert_eq!((info.owned, info.detached_secs), (false, 0));
        // An unowned, never-detached session encodes without the new fields.
        let encoded = ToClient::Sessions(sessions).encode().unwrap();
        assert_eq!(encoded.payload, frame.payload);
        let info = SessionInfo {
            session: 1,
            owned: true,
            detached_secs: 7,
            ..Default::default()
        };
        let json = serde_json::to_string(&info).unwrap();
        assert!(json.ends_with(r#""owned":true,"detached_secs":7}"#), "{json}");
    }

    #[test]
    fn process_names_and_directories_are_optional() {
        let frame = Frame::new(MSG_PROCESS, br#"{"session":4}"#.to_vec()).unwrap();
        assert_eq!(
            ToClient::decode(&frame).unwrap(),
            ToClient::Process(Process {
                session: 4,
                name: None,
                cwd: None,
            })
        );
        let frame = ToClient::Process(Process {
            session: 4,
            name: Some("sleep".into()),
            cwd: Some("/tmp".into()),
        })
        .encode()
        .unwrap();
        assert_eq!(frame.msg_type, 41);
        assert_eq!(frame.payload, br#"{"session":4,"name":"sleep","cwd":"/tmp"}"#);
        // The core-bound decoder doesn't know it.
        assert!(ToCore::decode(&frame).is_err());
        // A session id is required.
        let frame = Frame::new(MSG_PROCESS, br#"{"name":"x"}"#.to_vec()).unwrap();
        assert!(ToClient::decode(&frame).is_err());
    }
}
