//! What the agent writes on stdout, decoded in two steps (design decision 5):
//! first the JSON-RPC envelope, by hand ([`decode`] into [`Message`], with
//! `params` and `result` kept as [`Value`]), then the params or result of
//! each method the reducer reads, into Brindle's own closed types, through
//! the schema crate's types where the spike found them lenient enough.
//!
//! The two steps confine every failure to one message: an unknown method,
//! an unknown update kind or a malformed message is one skipped item
//! ([`Malformed`], [`BadParams`]), never a lost connection.
//!
//! - **`session/update`** is dispatched here on its `sessionUpdate` kind,
//!   never through the crate's `SessionUpdate` enum, which costs the helper
//!   more than a megabyte (decision 5, check 2). Only chunks, tool calls and
//!   tool call updates are decoded; every other kind is [`Update::Ignored`].
//! - **The stop reason** is read from the result's `Value`, since the crate's
//!   `StopReason` has no catch-all: a reason Brindle doesn't know is
//!   [`StopReason::Other`].
//! - **A permission request** is decoded by hand, keeping an option kind
//!   Brindle doesn't know as text ([`OptionKind::Unknown`]), and its id is in
//!   the envelope, so even a request whose params fail to decode can be
//!   answered.
//! - **Request ids** ([`Id`]) are kept as the JSON number or string the agent
//!   sent (Antigravity sends `"1"`; others start at `0`), and echoed back
//!   verbatim.
//!
//! The crate's enums are `#[non_exhaustive]`, so they are mapped onto
//! `brindle_transcript`'s closed ones here, and the reducer never matches
//! one of them.

use agent_client_protocol_schema::v1 as acp;
use brindle_remote_protocol::serde_json::{self, Map, Value};
use brindle_transcript::{PermissionKind, StopReason, ToolKind, ToolStatus};

/// The longest request id kept, as encoded JSON: a request with a longer id
/// is answered `-32600` with a null id (decision 13).
pub const MAX_ID_LEN: usize = 1024;

/// ACP's error codes the core reads: authentication required (decision
/// 17), and an `initialize` refused for its version (decision 4). The codes
/// the core sends are `rpc`'s.
pub const AUTH_REQUIRED: i64 = -32000;
pub const UNSUPPORTED_VERSION: i64 = -32022;

/// A JSON-RPC request id: a number or a string, at most [`MAX_ID_LEN`]
/// bytes encoded, kept as sent so an answer echoes it verbatim.
#[derive(Debug, Clone, PartialEq)]
pub struct Id(Value);

impl Id {
    /// The id of a request the core sends.
    pub fn number(id: u64) -> Self {
        Self(Value::from(id))
    }

    /// The id `value` is, when it is a number or a string within
    /// [`MAX_ID_LEN`] encoded.
    pub fn parse(value: &Value) -> Option<Self> {
        let scalar = matches!(value, Value::Number(_) | Value::String(_));
        (scalar && value.to_string().len() <= MAX_ID_LEN).then(|| Self(value.clone()))
    }

    /// The id as a non-negative integer, as the core's own requests have.
    pub fn as_u64(&self) -> Option<u64> {
        self.0.as_u64()
    }

    /// The id as JSON text, as it goes back on the wire.
    pub fn encode(&self) -> String {
        self.0.to_string()
    }

    /// The id as a JSON value.
    pub fn to_value(&self) -> Value {
        self.0.clone()
    }
}

/// A JSON-RPC error object.
#[derive(Debug, Clone, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    pub data: Option<Value>,
}

/// One line from the agent, as JSON-RPC 2.0.
#[derive(Debug, Clone, PartialEq)]
pub enum Message {
    /// A request, which the core must answer.
    Request {
        id: Id,
        method: String,
        params: Value,
    },
    /// A notification, which nobody answers.
    Notification { method: String, params: Value },
    /// A success answering one of the core's requests.
    Response { id: Id, result: Value },
    /// An error answering one of the core's requests; `id` is `None` when
    /// the agent couldn't read which (JSON-RPC's `null`).
    Error { id: Option<Id>, error: RpcError },
}

/// Why a line isn't a message. Every kind is skipped and counted (decision
/// 13); only [`Malformed::InvalidRequest`] is also answered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Malformed {
    /// Empty, or only whitespace.
    Blank,
    /// Not JSON.
    NotJson,
    /// A JSON-RPC batch, which ACP doesn't use.
    Batch,
    /// JSON, but not an object.
    NotObject,
    /// An object without `"jsonrpc": "2.0"`.
    NotJsonRpc,
    /// A request (it has a method and an id) whose method isn't a string or
    /// whose id isn't a number or string within [`MAX_ID_LEN`]: answered
    /// `-32600` with a null id.
    InvalidRequest,
    /// Anything else that isn't a message: a notification whose method isn't
    /// a string, a response without a usable id, with both or neither of
    /// `result` and `error`, or with an error that isn't `{ code, message }`.
    BadShape,
}

/// Decodes one line (without its newline). Invalid UTF-8 is replaced
/// first, so a line that is otherwise JSON-RPC still decodes.
pub fn decode(line: &[u8]) -> Result<Message, Malformed> {
    let text = String::from_utf8_lossy(line);
    if text.trim().is_empty() {
        return Err(Malformed::Blank);
    }
    let value: Value = serde_json::from_str(&text).map_err(|_| Malformed::NotJson)?;
    let mut object = match value {
        Value::Object(object) => object,
        Value::Array(_) => return Err(Malformed::Batch),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {
            return Err(Malformed::NotObject);
        }
    };
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0") {
        return Err(Malformed::NotJsonRpc);
    }
    let id = object.remove("id");
    match (object.remove("method"), id) {
        (Some(Value::String(method)), None) => Ok(Message::Notification {
            method,
            params: object.remove("params").unwrap_or(Value::Null),
        }),
        (Some(Value::String(method)), Some(id)) => {
            let id = Id::parse(&id).ok_or(Malformed::InvalidRequest)?;
            Ok(Message::Request {
                id,
                method,
                params: object.remove("params").unwrap_or(Value::Null),
            })
        }
        (Some(_), Some(_)) => Err(Malformed::InvalidRequest),
        (Some(_), None) => Err(Malformed::BadShape),
        (None, id) => answer(object, id),
    }
}

/// A message with no method: a response or an error.
fn answer(mut object: Map<String, Value>, id: Option<Value>) -> Result<Message, Malformed> {
    match (object.remove("result"), object.remove("error")) {
        (Some(result), None) => {
            let id = id.as_ref().and_then(Id::parse).ok_or(Malformed::BadShape)?;
            Ok(Message::Response { id, result })
        }
        (None, Some(error)) => {
            let id = match id {
                None | Some(Value::Null) => None,
                Some(id) => Some(Id::parse(&id).ok_or(Malformed::BadShape)?),
            };
            let error = rpc_error(&error).ok_or(Malformed::BadShape)?;
            Ok(Message::Error { id, error })
        }
        (Some(_), Some(_)) | (None, None) => Err(Malformed::BadShape),
    }
}

fn rpc_error(value: &Value) -> Option<RpcError> {
    Some(RpcError {
        code: value.get("code")?.as_i64()?,
        message: value.get("message")?.as_str()?.to_owned(),
        data: value.get("data").cloned(),
    })
}

/// Why a known method's params or result didn't decode: the message is
/// skipped (a request is still answered by its envelope id). The text is
/// for tests and debugging, never shown with agent data in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadParams(pub String);

impl BadParams {
    fn missing(what: &str) -> Self {
        Self(format!("no {what}"))
    }

    fn from_serde(error: &serde_json::Error) -> Self {
        Self(error.to_string())
    }
}

/// A request from the agent, by its method.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestKind {
    /// `session/request_permission`.
    Permission,
    /// Anything the core didn't offer (`fs/*`, `terminal/*`, elicitation,
    /// extensions): answered `-32601`.
    Unoffered,
}

pub fn request_kind(method: &str) -> RequestKind {
    match method {
        "session/request_permission" => RequestKind::Permission,
        _ => RequestKind::Unoffered,
    }
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

pub fn notification_kind(method: &str) -> NotificationKind {
    match method {
        "session/update" => NotificationKind::SessionUpdate,
        "$/cancel_request" => NotificationKind::CancelRequest,
        _ => NotificationKind::Other,
    }
}

/// What `initialize` answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Initialized {
    /// The version the agent speaks; Brindle speaks 1.
    pub protocol_version: u16,
    /// `agentInfo`, when the agent gave a well-formed one.
    pub agent: Option<AgentInfo>,
}

/// The agent's `agentInfo`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentInfo {
    pub name: String,
    pub title: Option<String>,
    pub version: String,
}

pub fn initialize_result(result: &Value) -> Result<Initialized, BadParams> {
    let response: acp::InitializeResponse =
        serde_json::from_value(result.clone()).map_err(|e| BadParams::from_serde(&e))?;
    Ok(Initialized {
        protocol_version: response.protocol_version.as_u16(),
        agent: response.agent_info.map(|info| AgentInfo {
            name: info.name,
            title: info.title,
            version: info.version,
        }),
    })
}

/// The versions an `initialize` error `-32022` says the agent supports:
/// `None` when the error isn't `-32022`, or its `data.supported` is missing,
/// empty or not a list of versions.
pub fn supported_versions(error: &RpcError) -> Option<Vec<u16>> {
    if error.code != UNSUPPORTED_VERSION {
        return None;
    }
    let supported = error.data.as_ref()?.get("supported")?.as_array()?;
    let versions = supported
        .iter()
        .map(|version| version.as_u64().and_then(|v| u16::try_from(v).ok()))
        .collect::<Option<Vec<u16>>>()?;
    (!versions.is_empty()).then_some(versions)
}

/// What `session/new` answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewSession {
    /// The ACP session id every later request names.
    pub session_id: String,
}

pub fn new_session_result(result: &Value) -> Result<NewSession, BadParams> {
    let response: acp::NewSessionResponse =
        serde_json::from_value(result.clone()).map_err(|e| BadParams::from_serde(&e))?;
    Ok(NewSession {
        session_id: response.session_id.0.to_string(),
    })
}

/// Why the turn a `session/prompt` result ends, read from `stopReason`
/// itself: a reason Brindle doesn't know keeps its name.
pub fn prompt_result(result: &Value) -> Result<StopReason, BadParams> {
    let name = result
        .get("stopReason")
        .and_then(Value::as_str)
        .ok_or_else(|| BadParams::missing("stopReason"))?;
    Ok(match name {
        "end_turn" => StopReason::EndTurn,
        "max_tokens" => StopReason::MaxTokens,
        "max_turn_requests" => StopReason::MaxTurnRequests,
        "refusal" => StopReason::Refusal,
        "cancelled" => StopReason::Cancelled,
        other => StopReason::Other {
            name: other.to_owned(),
        },
    })
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
    /// Any other kind, known (`plan`, `usage_update`,
    /// `available_commands_update`, `session_info_update`, …) or not.
    Ignored { kind: String },
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

/// A new tool call. Its content isn't kept (tool rows are one line).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolCall {
    /// The agent's id for it.
    pub call: String,
    pub title: String,
    pub kind: ToolKind,
    pub status: ToolStatus,
}

/// A tool call's changed fields; `None` leaves a field as it was.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolChange {
    pub call: String,
    pub title: Option<String>,
    pub kind: Option<ToolKind>,
    pub status: Option<ToolStatus>,
}

pub fn session_update(params: &Value) -> Result<SessionUpdate, BadParams> {
    let session_id = params
        .get("sessionId")
        .and_then(Value::as_str)
        .ok_or_else(|| BadParams::missing("sessionId"))?
        .to_owned();
    let update = params
        .get("update")
        .filter(|update| update.is_object())
        .ok_or_else(|| BadParams::missing("update"))?;
    let kind = update
        .get("sessionUpdate")
        .and_then(Value::as_str)
        .ok_or_else(|| BadParams::missing("sessionUpdate"))?;
    let update = match kind {
        "user_message_chunk" => Update::UserChunk(chunk(update)?),
        "agent_message_chunk" => Update::AgentChunk(chunk(update)?),
        "agent_thought_chunk" => Update::ThoughtChunk(chunk(update)?),
        "tool_call" => Update::ToolCall(tool_call(update)?),
        "tool_call_update" => Update::ToolChanged(tool_change(update)?),
        other => Update::Ignored {
            kind: other.to_owned(),
        },
    };
    Ok(SessionUpdate { session_id, update })
}

fn chunk(update: &Value) -> Result<Content, BadParams> {
    let chunk: acp::ContentChunk =
        serde_json::from_value(update.clone()).map_err(|e| BadParams::from_serde(&e))?;
    Ok(content(chunk.content))
}

#[allow(
    clippy::wildcard_enum_match_arm,
    reason = "foreign enum: ContentBlock, which is non_exhaustive"
)]
fn content(block: acp::ContentBlock) -> Content {
    match block {
        acp::ContentBlock::Text(text) => Content::Text(text.text),
        acp::ContentBlock::Image(_) => Content::Other(ContentKind::Image),
        acp::ContentBlock::Audio(_) => Content::Other(ContentKind::Audio),
        acp::ContentBlock::ResourceLink(_) => Content::Other(ContentKind::ResourceLink),
        acp::ContentBlock::Resource(_) => Content::Other(ContentKind::Resource),
        _ => Content::Other(ContentKind::Unknown),
    }
}

fn tool_call(update: &Value) -> Result<ToolCall, BadParams> {
    let call: acp::ToolCall =
        serde_json::from_value(update.clone()).map_err(|e| BadParams::from_serde(&e))?;
    Ok(ToolCall {
        call: call.tool_call_id.0.to_string(),
        title: call.title,
        kind: tool_kind(call.kind),
        status: tool_status(call.status).unwrap_or(ToolStatus::Pending),
    })
}

fn tool_change(update: &Value) -> Result<ToolChange, BadParams> {
    let update: acp::ToolCallUpdate =
        serde_json::from_value(update.clone()).map_err(|e| BadParams::from_serde(&e))?;
    Ok(ToolChange {
        call: update.tool_call_id.0.to_string(),
        title: update.fields.title,
        kind: update.fields.kind.map(tool_kind),
        status: update.fields.status.and_then(tool_status),
    })
}

#[allow(
    clippy::wildcard_enum_match_arm,
    reason = "foreign enum: ToolKind, which is non_exhaustive"
)]
fn tool_kind(kind: acp::ToolKind) -> ToolKind {
    match kind {
        acp::ToolKind::Read => ToolKind::Read,
        acp::ToolKind::Edit => ToolKind::Edit,
        acp::ToolKind::Delete => ToolKind::Delete,
        acp::ToolKind::Move => ToolKind::Move,
        acp::ToolKind::Search => ToolKind::Search,
        acp::ToolKind::Execute => ToolKind::Execute,
        acp::ToolKind::Think => ToolKind::Think,
        acp::ToolKind::Fetch => ToolKind::Fetch,
        acp::ToolKind::SwitchMode => ToolKind::SwitchMode,
        _ => ToolKind::Other,
    }
}

/// `None` for a status the crate added after 1.22.0: the field is left as
/// it was, as the crate does with a status it can't read.
#[allow(
    clippy::wildcard_enum_match_arm,
    reason = "foreign enum: ToolCallStatus, which is non_exhaustive"
)]
fn tool_status(status: acp::ToolCallStatus) -> Option<ToolStatus> {
    match status {
        acp::ToolCallStatus::Pending => Some(ToolStatus::Pending),
        acp::ToolCallStatus::InProgress => Some(ToolStatus::InProgress),
        acp::ToolCallStatus::Completed => Some(ToolStatus::Completed),
        acp::ToolCallStatus::Failed => Some(ToolStatus::Failed),
        _ => None,
    }
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

/// Decodes a permission request by hand, so an option kind Brindle doesn't
/// know doesn't lose the request (the crate rejects it whole). An option
/// missing its id, name or kind is left out; `options` that isn't a list
/// fails the decode.
pub fn permission_request(params: &Value) -> Result<PermissionRequest, BadParams> {
    let session_id = params
        .get("sessionId")
        .and_then(Value::as_str)
        .ok_or_else(|| BadParams::missing("sessionId"))?
        .to_owned();
    let options = params
        .get("options")
        .and_then(Value::as_array)
        .ok_or_else(|| BadParams::missing("options"))?
        .iter()
        .filter_map(requested_option)
        .collect();
    let call = params
        .get("toolCall")
        .and_then(|call| tool_change(call).ok());
    Ok(PermissionRequest {
        session_id,
        call,
        options,
    })
}

fn requested_option(option: &Value) -> Option<RequestedOption> {
    let text = |key: &str| option.get(key).and_then(Value::as_str).map(str::to_owned);
    let kind = text("kind")?;
    Some(RequestedOption {
        id: text("optionId")?,
        name: text("name")?,
        kind: match kind.as_str() {
            "allow_once" => OptionKind::Known(PermissionKind::AllowOnce),
            "allow_always" => OptionKind::Known(PermissionKind::AllowAlways),
            "reject_once" => OptionKind::Known(PermissionKind::RejectOnce),
            "reject_always" => OptionKind::Known(PermissionKind::RejectAlways),
            _ => OptionKind::Unknown { name: kind },
        },
    })
}

/// The id a `$/cancel_request` withdraws.
pub fn cancel_request(params: &Value) -> Result<Id, BadParams> {
    params
        .get("requestId")
        .and_then(Id::parse)
        .ok_or_else(|| BadParams::missing("requestId"))
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use brindle_remote_protocol::serde_json::json;

    fn ok(line: &str) -> Message {
        decode(line.as_bytes()).unwrap_or_else(|e| panic!("{line}: {e:?}"))
    }

    #[test]
    fn a_request_has_its_id_method_and_params() {
        let message =
            ok(r#"{"jsonrpc":"2.0","id":5,"method":"fs/read_text_file","params":{"path":"/x"}}"#);
        assert_eq!(
            message,
            Message::Request {
                id: Id::number(5),
                method: "fs/read_text_file".into(),
                params: json!({"path": "/x"}),
            }
        );
    }

    #[test]
    fn a_notification_is_a_method_with_no_id() {
        let message = ok(r#"{"jsonrpc":"2.0","method":"session/update","params":{"a":1}}"#);
        assert_eq!(
            message,
            Message::Notification {
                method: "session/update".into(),
                params: json!({"a": 1}),
            }
        );
    }

    #[test]
    fn missing_params_are_null() {
        assert_eq!(
            ok(r#"{"jsonrpc":"2.0","method":"x"}"#),
            Message::Notification {
                method: "x".into(),
                params: Value::Null
            }
        );
    }

    #[test]
    fn a_response_has_its_id_and_result() {
        assert_eq!(
            ok(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s"}}"#),
            Message::Response {
                id: Id::number(2),
                result: json!({"sessionId": "s"}),
            }
        );
    }

    #[test]
    fn an_error_has_its_code_message_and_data() {
        assert_eq!(
            ok(
                r#"{"jsonrpc":"2.0","id":1,"error":{"code":-32022,"message":"no","data":{"supported":[2]}}}"#
            ),
            Message::Error {
                id: Some(Id::number(1)),
                error: RpcError {
                    code: -32022,
                    message: "no".into(),
                    data: Some(json!({"supported": [2]})),
                },
            }
        );
    }

    #[test]
    fn an_error_with_a_null_id_has_none() {
        let Message::Error { id, error } =
            ok(r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32700,"message":"Parse error"}}"#)
        else {
            panic!("not an error");
        };
        assert_eq!((id, error.code, error.data), (None, -32700, None));
    }

    #[test]
    fn an_error_without_a_code_or_message_is_bad_shape() {
        for line in [
            r#"{"jsonrpc":"2.0","id":1,"error":{"message":"no"}}"#,
            r#"{"jsonrpc":"2.0","id":1,"error":{"code":1}}"#,
            r#"{"jsonrpc":"2.0","id":1,"error":"no"}"#,
        ] {
            assert_eq!(decode(line.as_bytes()), Err(Malformed::BadShape), "{line}");
        }
    }

    #[test]
    fn a_response_with_both_or_neither_result_and_error_is_bad_shape() {
        for line in [
            r#"{"jsonrpc":"2.0","id":1,"result":{},"error":{"code":1,"message":"m"}}"#,
            r#"{"jsonrpc":"2.0","id":1}"#,
            r#"{"jsonrpc":"2.0","result":{}}"#,
        ] {
            assert_eq!(decode(line.as_bytes()), Err(Malformed::BadShape), "{line}");
        }
    }

    #[test]
    fn a_request_id_can_be_a_string_and_is_echoed_verbatim() {
        let Message::Request { id, .. } =
            ok(r#"{"jsonrpc":"2.0","method":"session/request_permission","params":{},"id":"1"}"#)
        else {
            panic!("not a request");
        };
        assert_eq!(id.encode(), r#""1""#);
        assert_eq!(id.as_u64(), None);
    }

    #[test]
    fn an_id_round_trips_through_its_json_value() {
        for id in [Id::number(0), Id::number(u64::MAX)] {
            assert_eq!(Id::parse(&id.to_value()), Some(id));
        }
        let text = Id::parse(&json!("1")).unwrap_or_else(|| panic!("not an id"));
        assert_eq!(Id::parse(&text.to_value()), Some(text));
    }

    #[test]
    fn a_request_id_can_be_a_number_including_zero() {
        let Message::Request { id, .. } = ok(r#"{"jsonrpc":"2.0","id":0,"method":"m"}"#) else {
            panic!("not a request");
        };
        assert_eq!((id.encode().as_str(), id.as_u64()), ("0", Some(0)));
    }

    #[test]
    fn a_line_without_jsonrpc_2_is_not_json_rpc() {
        for line in [
            r#"{"id":1,"method":"m"}"#,
            r#"{"jsonrpc":"1.0","id":1,"method":"m"}"#,
            r#"{"jsonrpc":2,"id":1,"method":"m"}"#,
        ] {
            assert_eq!(
                decode(line.as_bytes()),
                Err(Malformed::NotJsonRpc),
                "{line}"
            );
        }
    }

    #[test]
    fn a_request_id_over_1_kib_is_an_invalid_request() {
        let at_limit = "a".repeat(MAX_ID_LEN - 2);
        let line = format!(r#"{{"jsonrpc":"2.0","id":"{at_limit}","method":"m"}}"#);
        assert!(matches!(
            decode(line.as_bytes()),
            Ok(Message::Request { .. })
        ));
        let over = "a".repeat(MAX_ID_LEN - 1);
        let line = format!(r#"{{"jsonrpc":"2.0","id":"{over}","method":"m"}}"#);
        assert_eq!(decode(line.as_bytes()), Err(Malformed::InvalidRequest));
    }

    #[test]
    fn a_response_id_over_1_kib_is_bad_shape() {
        let over = "a".repeat(MAX_ID_LEN);
        let line = format!(r#"{{"jsonrpc":"2.0","id":"{over}","result":{{}}}}"#);
        assert_eq!(decode(line.as_bytes()), Err(Malformed::BadShape));
    }

    #[test]
    fn a_request_whose_id_is_not_a_number_or_string_is_an_invalid_request() {
        for id in ["null", "true", "{}", "[1]"] {
            let line = format!(r#"{{"jsonrpc":"2.0","id":{id},"method":"m"}}"#);
            assert_eq!(
                decode(line.as_bytes()),
                Err(Malformed::InvalidRequest),
                "{line}"
            );
        }
    }

    #[test]
    fn a_request_whose_method_is_not_a_string_is_an_invalid_request() {
        assert_eq!(
            decode(br#"{"jsonrpc":"2.0","id":1,"method":7}"#),
            Err(Malformed::InvalidRequest)
        );
        assert_eq!(
            decode(br#"{"jsonrpc":"2.0","method":7}"#),
            Err(Malformed::BadShape)
        );
    }

    #[test]
    fn a_batch_array_is_rejected() {
        assert_eq!(
            decode(br#"[{"jsonrpc":"2.0","id":1,"method":"m"}]"#),
            Err(Malformed::Batch)
        );
        assert_eq!(decode(b"[]"), Err(Malformed::Batch));
    }

    #[test]
    fn json_that_is_not_an_object_is_not_an_object() {
        for line in ["1", "\"x\"", "null", "true"] {
            assert_eq!(decode(line.as_bytes()), Err(Malformed::NotObject), "{line}");
        }
    }

    #[test]
    fn a_blank_line_is_blank_and_text_is_not_json() {
        assert_eq!(decode(b""), Err(Malformed::Blank));
        assert_eq!(decode(b" \t\r"), Err(Malformed::Blank));
        assert_eq!(
            decode(b"Please visit the following URL"),
            Err(Malformed::NotJson)
        );
    }

    #[test]
    fn invalid_utf8_is_replaced_and_the_line_still_decodes() {
        let mut line = br#"{"jsonrpc":"2.0","method":"m","params":{"t":""#.to_vec();
        line.extend_from_slice(&[0xff, 0xfe]);
        line.extend_from_slice(br#""}}"#);
        let Ok(Message::Notification { params, .. }) = decode(&line) else {
            panic!("did not decode");
        };
        assert_eq!(params["t"], json!("\u{fffd}\u{fffd}"));
    }

    #[test]
    fn methods_map_to_their_kinds() {
        assert_eq!(
            request_kind("session/request_permission"),
            RequestKind::Permission
        );
        for method in [
            "fs/read_text_file",
            "terminal/create",
            "elicitation/create",
            "_x/y",
        ] {
            assert_eq!(request_kind(method), RequestKind::Unoffered, "{method}");
        }
        assert_eq!(
            notification_kind("session/update"),
            NotificationKind::SessionUpdate
        );
        assert_eq!(
            notification_kind("$/cancel_request"),
            NotificationKind::CancelRequest
        );
        assert_eq!(
            notification_kind("_auth/status_update"),
            NotificationKind::Other
        );
    }

    #[test]
    fn a_stop_reason_brindle_does_not_know_keeps_its_name() {
        assert_eq!(
            prompt_result(&json!({"stopReason": "paused_for_review"})),
            Ok(StopReason::Other {
                name: "paused_for_review".into()
            })
        );
        assert_eq!(
            prompt_result(&json!({"stopReason": "cancelled"})),
            Ok(StopReason::Cancelled)
        );
        assert!(prompt_result(&json!({})).is_err());
    }

    #[test]
    fn supported_versions_need_a_32022_with_a_list() {
        let error = |code, data: Option<Value>| RpcError {
            code,
            message: String::new(),
            data,
        };
        assert_eq!(
            supported_versions(&error(
                -32022,
                Some(json!({"requested": 1, "supported": [2, 3]}))
            )),
            Some(vec![2, 3])
        );
        for data in [
            None,
            Some(json!({})),
            Some(json!({"supported": []})),
            Some(json!({"supported": ["2"]})),
            Some(json!({"supported": [70000]})),
            Some(json!("2")),
        ] {
            assert_eq!(
                supported_versions(&error(-32022, data.clone())),
                None,
                "{data:?}"
            );
        }
        assert_eq!(
            supported_versions(&error(-32000, Some(json!({"supported": [2]})))),
            None
        );
    }

    #[test]
    fn an_unknown_update_kind_is_ignored_not_rejected() {
        let update = session_update(&json!({
            "sessionId": "s",
            "update": {"sessionUpdate": "brand_new_kind", "payload": 1}
        }));
        assert_eq!(
            update.map(|u| u.update),
            Ok(Update::Ignored {
                kind: "brand_new_kind".into()
            })
        );
    }

    #[test]
    fn an_unknown_tool_kind_is_other_and_an_unknown_status_is_left_out() {
        let Ok(SessionUpdate {
            update: Update::ToolChanged(change),
            ..
        }) = session_update(&json!({
            "sessionId": "s",
            "update": {"sessionUpdate": "tool_call_update", "toolCallId": "c", "kind": "teleport", "status": "paused"}
        }))
        else {
            panic!("did not decode");
        };
        assert_eq!(change.kind, Some(ToolKind::Other));
        assert_eq!(change.status, None);
        assert_eq!(change.title, None);
    }

    #[test]
    fn a_non_text_chunk_is_other_content() {
        let update = session_update(&json!({
            "sessionId": "s",
            "update": {"sessionUpdate": "agent_message_chunk",
                       "content": {"type": "image", "data": "", "mimeType": "image/png"}}
        }));
        assert_eq!(
            update.map(|u| u.update),
            Ok(Update::AgentChunk(Content::Other(ContentKind::Image)))
        );
    }

    #[test]
    fn a_permission_option_kind_brindle_does_not_know_is_kept_as_text() {
        let request = permission_request(&json!({
            "sessionId": "s",
            "toolCall": {"toolCallId": "c", "title": "T"},
            "options": [
                {"optionId": "a", "name": "Allow", "kind": "allow_once"},
                {"optionId": "s", "name": "Session", "kind": "allow_for_session"},
                {"optionId": "x", "name": "No kind"}
            ]
        }));
        let Ok(request) = request else {
            panic!("did not decode");
        };
        assert_eq!(
            request.options,
            [
                RequestedOption {
                    id: "a".into(),
                    name: "Allow".into(),
                    kind: OptionKind::Known(PermissionKind::AllowOnce),
                },
                RequestedOption {
                    id: "s".into(),
                    name: "Session".into(),
                    kind: OptionKind::Unknown {
                        name: "allow_for_session".into()
                    },
                },
            ]
        );
        assert_eq!(request.call.map(|c| c.title), Some(Some("T".into())));
    }

    #[test]
    fn a_permission_request_without_options_fails_but_keeps_its_id() {
        let line = r#"{"jsonrpc":"2.0","id":"7","method":"session/request_permission","params":{"sessionId":"s"}}"#;
        let Message::Request { id, params, .. } = ok(line) else {
            panic!("not a request");
        };
        assert!(permission_request(&params).is_err());
        assert_eq!(id.encode(), r#""7""#);
    }

    #[test]
    fn a_cancel_request_names_the_withdrawn_id() {
        assert_eq!(cancel_request(&json!({"requestId": 0})), Ok(Id::number(0)));
        assert_eq!(
            cancel_request(&json!({"requestId": "1"})).map(|id| id.encode()),
            Ok(r#""1""#.into())
        );
        assert!(cancel_request(&json!({})).is_err());
    }

    /// The captured sessions (task 1.2 and the later captures), each line
    /// `c>a` or `a>c`, a tab, then the line as sent.
    const CAPTURES: [(&str, &[u8]); 10] = [
        (
            "antigravity-1.5.16-reject",
            include_bytes!("../../tests/fixtures/acp/antigravity-1.5.16-reject.log"),
        ),
        (
            "antigravity-1.5.16-trusted-workspace",
            include_bytes!("../../tests/fixtures/acp/antigravity-1.5.16-trusted-workspace.log"),
        ),
        (
            "claude-agent-acp-0.112.0-auto",
            include_bytes!("../../tests/fixtures/acp/claude-agent-acp-0.112.0-auto.log"),
        ),
        (
            "claude-agent-acp-0.112.0-haiku-reject",
            include_bytes!("../../tests/fixtures/acp/claude-agent-acp-0.112.0-haiku-reject.log"),
        ),
        (
            "claude-agent-acp-0.112.0-reject",
            include_bytes!("../../tests/fixtures/acp/claude-agent-acp-0.112.0-reject.log"),
        ),
        (
            "claude-code-acp-0.16.2-reject",
            include_bytes!("../../tests/fixtures/acp/claude-code-acp-0.16.2-reject.log"),
        ),
        (
            "codex-acp-2.12.0-reject",
            include_bytes!("../../tests/fixtures/acp/codex-acp-2.12.0-reject.log"),
        ),
        (
            "copilot-1.0.115-reject",
            include_bytes!("../../tests/fixtures/acp/copilot-1.0.115-reject.log"),
        ),
        (
            "gemini-0.71.1-login-on-stdout",
            include_bytes!("../../tests/fixtures/acp/gemini-0.71.1-login-on-stdout.log"),
        ),
        (
            "handwritten-unknown-values",
            include_bytes!("../../tests/fixtures/acp/handwritten-unknown-values.log"),
        ),
    ];

    /// What the second step made of a captured line from the agent.
    #[derive(Debug, PartialEq, Eq)]
    enum Decoded {
        Update(Update),
        Permission(PermissionRequest),
        Initialized(Initialized),
        NewSession,
        Stopped(StopReason),
        /// A response to a request Brindle never sends (`session/set_mode`,
        /// `session/set_model`, `session/set_config_option`).
        NotBrindles,
        Ignored,
        Bad(String),
    }

    /// Splits a capture into its lines, as (direction, line).
    fn capture_lines(bytes: &[u8]) -> Vec<(&str, &[u8])> {
        bytes
            .split(|b| *b == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| {
                let tab = line
                    .iter()
                    .position(|b| *b == b'\t')
                    .unwrap_or_else(|| panic!("no tab"));
                let direction = std::str::from_utf8(&line[..tab]).unwrap_or_default();
                assert!(direction == "c>a" || direction == "a>c", "{direction}");
                (direction, &line[tab + 1..])
            })
            .collect()
    }

    /// Decodes every line of a capture, both directions, and returns what
    /// each line from the agent became.
    fn decode_capture(bytes: &[u8]) -> (usize, Vec<Result<Decoded, Malformed>>) {
        let lines = capture_lines(bytes);
        let mut sent: HashMap<String, String> = HashMap::new();
        let mut decoded = Vec::new();
        for (direction, line) in &lines {
            let message = decode(line);
            if *direction == "c>a" {
                // What the capture script sent is JSON-RPC too.
                match message {
                    Ok(Message::Request { id, method, .. }) => {
                        sent.insert(id.encode(), method);
                    }
                    Ok(Message::Response { .. }) => {}
                    other => panic!("client line: {other:?}"),
                }
                continue;
            }
            decoded.push(message.map(|message| {
                match message {
                    Message::Notification { method, params } => match notification_kind(&method) {
                        NotificationKind::SessionUpdate => session_update(&params)
                            .map_or_else(|e| Decoded::Bad(e.0), |u| Decoded::Update(u.update)),
                        NotificationKind::CancelRequest => Decoded::Ignored,
                        NotificationKind::Other => Decoded::Ignored,
                    },
                    Message::Request { method, params, .. } => match request_kind(&method) {
                        RequestKind::Permission => permission_request(&params)
                            .map_or_else(|e| Decoded::Bad(e.0), Decoded::Permission),
                        RequestKind::Unoffered => Decoded::Ignored,
                    },
                    Message::Response { id, result } => {
                        match sent.get(&id.encode()).map(String::as_str) {
                            Some("initialize") => initialize_result(&result)
                                .map_or_else(|e| Decoded::Bad(e.0), Decoded::Initialized),
                            Some("session/new") => new_session_result(&result)
                                .map_or_else(|e| Decoded::Bad(e.0), |_| Decoded::NewSession),
                            Some("session/prompt") => prompt_result(&result)
                                .map_or_else(|e| Decoded::Bad(e.0), Decoded::Stopped),
                            Some(_) => Decoded::NotBrindles,
                            None => panic!("a response to nothing sent: {}", id.encode()),
                        }
                    }
                    Message::Error { error, .. } => Decoded::Bad(error.message),
                }
            }));
        }
        (lines.len(), decoded)
    }

    #[test]
    fn every_line_of_the_real_agents_captures_decodes() {
        let mut lines = 0;
        for (name, bytes) in CAPTURES {
            if name.starts_with("gemini") || name.starts_with("handwritten") {
                continue;
            }
            let (count, decoded) = decode_capture(bytes);
            lines += count;
            let mut kinds = (0, 0, 0, 0);
            for line in &decoded {
                match line {
                    Ok(Decoded::Bad(why)) => panic!("{name}: {why}"),
                    Err(malformed) => panic!("{name}: {malformed:?}"),
                    Ok(Decoded::Initialized(init)) => {
                        assert_eq!(init.protocol_version, 1, "{name}");
                        assert!(init.agent.is_some(), "{name}: no agentInfo");
                        kinds.0 += 1;
                    }
                    Ok(Decoded::NewSession) => kinds.1 += 1,
                    Ok(Decoded::Stopped(stop)) => {
                        assert!(
                            matches!(stop, StopReason::EndTurn | StopReason::Cancelled),
                            "{name}: {stop:?}"
                        );
                        kinds.2 += 1;
                    }
                    Ok(Decoded::Permission(request)) => {
                        assert!(
                            request
                                .options
                                .iter()
                                .all(|o| matches!(o.kind, OptionKind::Known(_))),
                            "{name}: {request:?}"
                        );
                        assert!(request.call.is_some(), "{name}: {request:?}");
                        kinds.3 += 1;
                    }
                    Ok(Decoded::Update(_) | Decoded::NotBrindles | Decoded::Ignored) => {}
                }
            }
            assert_eq!((kinds.0, kinds.1, kinds.2), (1, 1, 1), "{name}");
            let asks = !name.ends_with("-auto") && !name.ends_with("-trusted-workspace");
            assert_eq!(kinds.3, usize::from(asks), "{name}: permission requests");
        }
        assert_eq!(lines, 310);
    }

    #[test]
    fn the_captured_agents_send_every_kind_the_reducer_reads() {
        let mut seen = Vec::new();
        for (_, bytes) in &CAPTURES[..8] {
            for line in decode_capture(bytes).1.into_iter().flatten() {
                if let Decoded::Update(update) = line {
                    seen.push(match update {
                        Update::UserChunk(_) => "user",
                        Update::AgentChunk(Content::Text(_)) => "agent text",
                        Update::AgentChunk(Content::Other(_)) => "agent other",
                        Update::ThoughtChunk(_) => "thought",
                        Update::ToolCall(_) => "tool call",
                        Update::ToolChanged(_) => "tool changed",
                        Update::Ignored { .. } => "ignored",
                    });
                }
            }
        }
        for kind in [
            "agent text",
            "thought",
            "tool call",
            "tool changed",
            "ignored",
        ] {
            assert!(seen.contains(&kind), "{kind}");
        }
    }

    #[test]
    fn antigravitys_string_permission_id_is_kept_verbatim() {
        let lines = capture_lines(CAPTURES[0].1);
        let ids: Vec<String> = lines
            .iter()
            .filter(|(d, _)| *d == "a>c")
            .filter_map(|(_, line)| match decode(line) {
                Ok(Message::Request { id, .. }) => Some(id.encode()),
                _ => None,
            })
            .collect();
        assert_eq!(ids, [r#""1""#]);
    }

    #[test]
    fn geminis_login_prompt_on_stdout_is_never_a_message() {
        let (count, decoded) = decode_capture(CAPTURES[8].1);
        assert_eq!(count, 11);
        assert_eq!(decoded.len(), 10);
        let blank = decoded
            .iter()
            .filter(|d| **d == Err(Malformed::Blank))
            .count();
        let text = decoded
            .iter()
            .filter(|d| **d == Err(Malformed::NotJson))
            .count();
        assert_eq!((blank, text), (4, 6));
    }

    #[test]
    fn the_handwritten_unknown_values_are_each_confined_to_their_message() {
        let (count, decoded) = decode_capture(CAPTURES[9].1);
        assert_eq!(count, 11);
        let decoded: Vec<Decoded> = decoded
            .into_iter()
            .map(|d| d.unwrap_or_else(|e| panic!("{e:?}")))
            .map(|d| match d {
                Decoded::Bad(_) => Decoded::Bad(String::new()),
                other => other,
            })
            .collect();
        let teleport = |status| ToolChange {
            call: "call-1".into(),
            title: Some("Teleport a file".into()),
            kind: Some(ToolKind::Other),
            status,
        };
        let option = |id: &str, name: &str, kind| RequestedOption {
            id: id.into(),
            name: name.into(),
            kind,
        };
        assert_eq!(
            decoded,
            [
                Decoded::Update(Update::Ignored {
                    kind: "brand_new_kind".into()
                }),
                // An unknown content block and a tool call without a title
                // each lose only their own update.
                Decoded::Bad(String::new()),
                Decoded::Update(Update::ToolCall(ToolCall {
                    call: "call-1".into(),
                    title: "Teleport a file".into(),
                    kind: ToolKind::Other,
                    status: ToolStatus::Pending,
                })),
                Decoded::Update(Update::ToolChanged(ToolChange {
                    title: None,
                    ..teleport(None)
                })),
                Decoded::Bad(String::new()),
                Decoded::Permission(PermissionRequest {
                    session_id: "sess-1".into(),
                    call: Some(teleport(None)),
                    options: vec![
                        option(
                            "allow",
                            "Allow",
                            OptionKind::Known(PermissionKind::AllowOnce)
                        ),
                        option(
                            "session",
                            "Allow for this session",
                            OptionKind::Unknown {
                                name: "allow_for_session".into()
                            }
                        ),
                        option(
                            "reject",
                            "Reject",
                            OptionKind::Known(PermissionKind::RejectOnce)
                        ),
                    ],
                }),
                Decoded::Permission(PermissionRequest {
                    session_id: "sess-1".into(),
                    call: Some(teleport(None)),
                    options: vec![
                        option(
                            "allow",
                            "Allow",
                            OptionKind::Known(PermissionKind::AllowOnce)
                        ),
                        option(
                            "reject",
                            "Reject",
                            OptionKind::Known(PermissionKind::RejectOnce)
                        ),
                    ],
                }),
                Decoded::Stopped(StopReason::Other {
                    name: "paused_for_review".into()
                }),
                Decoded::Stopped(StopReason::EndTurn),
            ]
        );
    }

    /// Hand-written answers no captured agent gave, one JSON-RPC line each.
    fn jsonl(bytes: &[u8]) -> Vec<Message> {
        bytes
            .split(|b| *b == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| decode(line).unwrap_or_else(|e| panic!("{e:?}")))
            .collect()
    }

    #[test]
    fn an_initialize_32022_names_the_versions_or_none_without_data() {
        let messages = jsonl(include_bytes!(
            "../../tests/fixtures/acp/initialize-32022.jsonl"
        ));
        let versions: Vec<Option<Vec<u16>>> = messages
            .iter()
            .map(|message| match message {
                Message::Error { id, error } => {
                    assert_eq!(id, &Some(Id::number(1)));
                    supported_versions(error)
                }
                other => panic!("not an error: {other:?}"),
            })
            .collect();
        assert_eq!(versions, [Some(vec![2]), None]);
    }

    #[test]
    fn an_initialize_success_with_version_2_decodes_with_its_version() {
        let messages = jsonl(include_bytes!(
            "../../tests/fixtures/acp/initialize-v2-reply.jsonl"
        ));
        let [Message::Response { id, result }] = messages.as_slice() else {
            panic!("{messages:?}");
        };
        assert_eq!(id, &Id::number(1));
        assert_eq!(
            initialize_result(result).map(|init| init.protocol_version),
            Ok(2)
        );
    }

    #[test]
    fn a_prompt_answered_32000_is_an_auth_required_error() {
        let messages = jsonl(include_bytes!(
            "../../tests/fixtures/acp/prompt-auth-required.jsonl"
        ));
        let [Message::Error { id, error }] = messages.as_slice() else {
            panic!("{messages:?}");
        };
        assert_eq!((id, error.code), (&Some(Id::number(3)), AUTH_REQUIRED));
        assert!(!error.message.is_empty());
    }

    /// A fixed-seed xorshift, so the fuzz run is the same every time.
    struct XorShift(u64);

    impl XorShift {
        fn below(&mut self, n: usize) -> usize {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 % n.max(1) as u64) as usize
        }
    }

    #[test]
    fn mutated_lines_never_panic_the_decoders() {
        let mut rng = XorShift(0x9e37_79b9_7f4a_7c15);
        let seeds: Vec<&[u8]> = CAPTURES
            .iter()
            .flat_map(|(_, bytes)| capture_lines(bytes))
            .map(|(_, line)| line)
            .collect();
        let alphabet = b"{}[]\":,0123456789-.eE \\ntrufalsjoic\xff\xc3";
        for _ in 0..20_000 {
            let mut line = seeds[rng.below(seeds.len())].to_vec();
            for _ in 0..=rng.below(4) {
                let at = rng.below(line.len() + 1);
                match rng.below(3) {
                    0 if at < line.len() => {
                        line.remove(at);
                    }
                    1 => line.insert(at, alphabet[rng.below(alphabet.len())]),
                    _ => line.truncate(at),
                }
            }
            if let Ok(message) = decode(&line) {
                let value = match message {
                    Message::Request { params, .. } | Message::Notification { params, .. } => {
                        params
                    }
                    Message::Response { result, .. } => result,
                    Message::Error { error, .. } => error.data.unwrap_or(Value::Null),
                };
                let _ = session_update(&value);
                let _ = permission_request(&value);
                let _ = initialize_result(&value);
                let _ = new_session_result(&value);
                let _ = prompt_result(&value);
                let _ = cancel_request(&value);
            }
        }
    }
}
