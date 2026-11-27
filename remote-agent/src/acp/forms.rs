//! What the reducer reads from an agent and asks to be written to it, in
//! Brindle's own closed types, whatever ACP version the session speaks
//! (`ports R14`): the session's dialect (`acp::dialect`) decodes each
//! message into these and encodes each [`Request`] as that version's line.
//! Plain data, with no ACP method or field name in it.

use brindle_transcript::{
    Continuation, Exit, Location, LoginMethod, PermissionKind, ToolKind, ToolStatus,
};

use super::envelope::Id;

/// ACP's error codes the core reads: authentication required (decision
/// 17), and an `initialize` refused for its version (decision 4). The codes
/// the core sends are `rpc`'s.
pub const AUTH_REQUIRED: i64 = -32000;
pub const UNSUPPORTED_VERSION: i64 = -32022;

/// A request from the agent, by its method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestKind {
    /// `session/request_permission`.
    Permission,
    /// Anything the core didn't offer (`fs/*`, `terminal/*`, elicitation,
    /// extensions): answered `-32601`.
    Unoffered,
}

/// A notification from the agent, by its method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationKind {
    /// `session/update`.
    SessionUpdate,
    /// `$/cancel_request`: the agent withdraws one of its requests.
    CancelRequest,
    /// Anything else (`_auth/status_update` and the like): ignored, and
    /// never logged, since some carry the account's email.
    Other,
}

/// What `initialize` answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Initialized {
    /// The version the agent speaks; Brindle speaks 1.
    pub protocol_version: u16,
    /// `agentInfo`, when the agent gave a well-formed one.
    pub agent: Option<AgentInfo>,
    /// Whether the agent offers `session/load`
    /// (`agentCapabilities.loadSession`) and `session/resume`
    /// (`agentCapabilities.sessionCapabilities.resume`). A malformed
    /// capability reads as not offered.
    pub continuation: Continuation,
    /// Whether the agent lists its conversations
    /// (`agentCapabilities.sessionCapabilities.list`; OpenSpec change
    /// `agents-session-list`). A malformed capability reads as not offered.
    pub lists: bool,
}

/// A page of the agent's conversations, as its answer to a list gave them,
/// in its order (`agents-session-list`, decision 2): each read on its own,
/// so `unreadable` counts the ones that weren't.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Listing {
    pub conversations: Vec<Listed>,
    /// The cursor for the next page, when the agent has more.
    pub next: Option<String>,
    pub unreadable: usize,
}

/// One listed conversation, as sent (not yet bounded or cleaned).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub id: String,
    /// Its directory; the reducer keeps only the session's.
    pub cwd: String,
    pub title: Option<String>,
    /// When it was last active, in the agent's text.
    pub updated: Option<String>,
}

/// The agent's `agentInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentInfo {
    pub name: String,
    pub title: Option<String>,
    pub version: String,
}

/// The most login methods kept from `initialize` (OpenSpec change
/// `agents-login`, design decision 3); the rest are left out.
pub const MAX_LOGIN_METHODS: usize = 16;
/// The longest method id and name kept, in bytes: a method with a longer
/// one is left out (an id can't be cut, since a login names it).
pub const MAX_LOGIN_ID: usize = 1024;
/// The longest method description kept, in bytes: a longer one is cut.
pub const MAX_LOGIN_DESCRIPTION: usize = 4 * 1024;
/// The most `args` and `env` entries a terminal method may have, and the
/// longest each argument, key and value may be, in bytes: past either, the
/// method is left out, since a cut argument would run something else.
pub const MAX_LOGIN_ARGS: usize = 32;
pub const MAX_LOGIN_ARG: usize = 4 * 1024;
/// The most the kept methods may take, encoded as the transcript sends
/// them: the methods past it are left out, so a `LoginRequired` event,
/// which carries them all, fits a frame (`pack_transcript`).
pub const MAX_LOGIN_METHODS_LEN: usize = 256 * 1024;

/// The login methods `initialize` advertised, as [`auth_methods`] read them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Methods {
    /// The methods kept, in the agent's order. Their name and description
    /// are as sent (bounded, not yet cleaned).
    pub methods: Vec<LoginMethod>,
    /// How many methods were left out: of a type Brindle doesn't run, a
    /// terminal one Brindle didn't advertise, malformed, past a bound, or
    /// past [`MAX_LOGIN_METHODS`] or [`MAX_LOGIN_METHODS_LEN`].
    pub dropped: usize,
    /// How many kept descriptions were cut to [`MAX_LOGIN_DESCRIPTION`].
    pub cut: usize,
}

/// What `session/new` answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSession {
    /// The ACP session id every later request names.
    pub session_id: String,
}

/// A `session/update` notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionUpdate {
    pub session_id: String,
    pub update: Update,
}

/// What a `session/update` says, by its `sessionUpdate` kind.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Update {
    /// `user_message_chunk`.
    UserChunk(Content),
    /// `agent_message_chunk`.
    AgentChunk(Content),
    /// `agent_thought_chunk`.
    ThoughtChunk(Content),
    /// `tool_call`: a new tool call.
    ToolCall(ToolCall),
    /// `tool_call_update`: the fields that arrived, in no assumed order.
    ToolChanged(ToolChange),
    /// `session_info_update`: what it says of the conversation's title
    /// (OpenSpec change `agents-session-title`, decision 2). Its
    /// `updatedAt` and `_meta` aren't read.
    SessionInfo { title: Patch<String> },
    /// Any other kind, known (`plan`, `usage_update`,
    /// `available_commands_update`, …) or not.
    Ignored { kind: String },
}

/// What an update says of one optional field, ACP's "undefined, null or a
/// value" (`MaybeUndefined`): left out keeps it, `null` clears it, a value
/// sets it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Patch<T> {
    Keep,
    Clear,
    Set(T),
}

impl<T> Patch<T> {
    /// `Set` for a value, `Keep` for none: a field a version can't clear.
    pub fn kept_or_set(value: Option<T>) -> Self {
        value.map_or(Self::Keep, Self::Set)
    }

    /// The value it sets, if it sets one.
    pub fn value(&self) -> Option<&T> {
        match self {
            Self::Set(value) => Some(value),
            Self::Keep | Self::Clear => None,
        }
    }

    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Patch<U> {
        match self {
            Self::Keep => Patch::Keep,
            Self::Clear => Patch::Clear,
            Self::Set(value) => Patch::Set(f(value)),
        }
    }

    /// The field's new value, `reset` when it is cleared; `None` when it is
    /// kept.
    pub fn or_reset(self, reset: impl FnOnce() -> T) -> Option<T> {
        match self {
            Self::Keep => None,
            Self::Clear => Some(reset()),
            Self::Set(value) => Some(value),
        }
    }
}

/// A chunk's content block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Content {
    Text(String),
    /// A block that isn't text, by its kind; the reducer shows a placeholder.
    Other(ContentKind),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentKind {
    Image,
    Audio,
    ResourceLink,
    Resource,
    /// A kind the schema crate added after 1.22.0 (it can't decode one yet).
    Unknown,
}

/// A new tool call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    /// The agent's id for it.
    pub call: String,
    pub title: String,
    pub kind: ToolKind,
    pub status: ToolStatus,
    /// Its content (set, empty when it sent none), locations, raw values
    /// and output.
    pub detail: ToolDetail,
}

/// What a tool call carries beyond its row (design decision 3 of
/// `agents-tool-content`), on a new call, an update or a permission
/// request's `toolCall`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDetail {
    /// The whole content list, which replaces the call's when set.
    pub content: Patch<Vec<ToolItemForm>>,
    /// Content items left out because they couldn't be read.
    pub unreadable: u32,
    pub locations: Patch<Vec<Location>>,
    /// The raw input and result, as text: a JSON string as itself,
    /// anything else as indented JSON. Not yet bounded.
    pub input: Patch<String>,
    pub result: Patch<String>,
    /// Output for the call's terminals, in the order sent.
    pub outputs: Vec<OutputForm>,
}

impl Default for ToolDetail {
    /// Nothing said of any of it.
    fn default() -> Self {
        Self {
            content: Patch::Keep,
            unreadable: 0,
            locations: Patch::Keep,
            input: Patch::Keep,
            result: Patch::Keep,
            outputs: Vec::new(),
        }
    }
}

/// One content item of a tool call, as read; the reducer makes each a
/// transcript `ToolItem`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ToolItemForm {
    /// A content block: text, or a placeholder for another kind.
    Content(Content),
    /// A file's old and new text; no old text is a new file.
    RawDiff {
        path: String,
        old: Option<String>,
        new: String,
    },
    /// A terminal's output, by the agent's terminal id.
    Terminal { terminal: String },
    /// An item of a kind Brindle doesn't show, by its type.
    Unknown { kind: String },
}

/// Output for one of a tool call's terminals, by the agent's terminal id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputForm {
    Opened {
        terminal: String,
        cwd: Option<String>,
        command: Option<String>,
    },
    /// The command wrote `bytes`, escape sequences and all.
    Appended { terminal: String, bytes: Vec<u8> },
    /// The command ended: by a code, a signal, or neither said.
    Exited {
        terminal: String,
        exit: Option<Exit>,
    },
}

/// A tool call's changed fields, each kept, cleared or set ([`Patch`]); the
/// reducer handles all three for every version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolChange {
    pub call: String,
    /// Cleared, the title becomes the call's id.
    pub title: Patch<String>,
    /// Cleared, the kind becomes `other`.
    pub kind: Patch<ToolKind>,
    /// Cleared, the status becomes `pending`.
    pub status: Patch<ToolStatus>,
    pub detail: ToolDetail,
}

/// A `session/request_permission` request's params.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRequest {
    pub session_id: String,
    /// The tool call it asks about, when `toolCall` decodes; its title may
    /// differ from the `tool_call`'s (Copilot's does).
    pub call: Option<ToolChange>,
    /// The options that are well-formed, in the agent's order.
    pub options: Vec<RequestedOption>,
}

/// One option a permission request offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestedOption {
    /// The agent's id for it, which the answer names. Ids vary between
    /// agents (`reject_once`, `deny`, `cancel`), so choose by `kind`.
    pub id: String,
    pub name: String,
    pub kind: OptionKind,
}

/// What choosing an option does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OptionKind {
    Known(PermissionKind),
    /// A kind ACP v1 doesn't name, kept as sent.
    Unknown {
        name: String,
    },
}

/// Which way a conversation is continued, as the reducer awaits it: its
/// answer is read by [`Continued`] (`agents-session-recovery`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Continued {
    /// Continued with no replay.
    Resume,
    /// Continued with the conversation replayed first.
    Load,
}

/// The JSON-RPC errors the core answers an agent's request with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// `-32601`: a method the core didn't offer (`fs/*`, `terminal/*`, …).
    MethodNotFound,
    /// `-32600`: a request the core can't read, such as one with an id
    /// over 1 KiB, answered with a null id.
    InvalidRequest,
}

impl Refusal {
    pub fn code(self) -> i64 {
        match self {
            Self::MethodNotFound => -32601,
            Self::InvalidRequest => -32600,
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Self::MethodNotFound => "Method not found",
            Self::InvalidRequest => "Invalid Request",
        }
    }
}

/// A line the reducer asks to be written to the agent, which the session's
/// dialect encodes: one of the core's requests by its id (from `Rpc`), a
/// notification, or an answer to one of the agent's requests.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Request<'a> {
    /// Asks for the dialect's version as Brindle at `version`, with one
    /// client capability: `auth.terminal`, since a client runs an agent's
    /// terminal login methods (`agents-login`). No `fs` and no `terminal`.
    Initialize { id: u64, version: &'a str },
    /// Logs in with the login method `method` (`agents-login`).
    Authenticate { id: u64, method: &'a str },
    /// A new conversation in `cwd` (already absolute), with no MCP servers.
    NewSession { id: u64, cwd: &'a str },
    /// Continues conversation `session` in `cwd` (its own directory), with
    /// no MCP servers, as [`Continued`] says.
    Continue {
        id: u64,
        how: Continued,
        session: &'a str,
        cwd: &'a str,
    },
    /// One text block for conversation `session`.
    Prompt {
        id: u64,
        session: &'a str,
        text: &'a str,
    },
    /// A page of the agent's conversations in `cwd`, after `cursor`
    /// (`agents-session-list`).
    List {
        id: u64,
        cwd: &'a str,
        cursor: Option<&'a str>,
    },
    /// Cancels `session`'s turn, a notification.
    Cancel { session: &'a str },
    /// Cancels the core's own request `id` (a login the user cancelled), a
    /// notification. The agent may ignore it, but must still answer the
    /// request.
    Withdraw { id: u64 },
    /// Answers permission request `id`: the option chosen, or `cancelled`
    /// with none.
    PermissionAnswer { id: &'a Id, option: Option<&'a str> },
    /// An error answering request `id`, or a null id when the request's id
    /// couldn't be read.
    Error {
        id: Option<&'a Id>,
        refusal: Refusal,
    },
}
