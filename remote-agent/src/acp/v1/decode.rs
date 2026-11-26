//! What an ACP v1 agent writes, read into the forms (`acp::forms`): the
//! params or result of each method the reducer reads, decoded into
//! Brindle's own closed types through the schema crate's types where the
//! spike found them lenient enough (design decision 5 of `agents-sessions`).
//! With [`super::encode`], the only code naming `agent_client_protocol_schema::v1`,
//! a v1 method name or a v1 field name (`ports R14`).
//!
//! Each failure is confined to one message: an unknown method, an unknown
//! update kind or a malformed message is one skipped item ([`BadParams`]).
//!
//! - **`session/update`** is dispatched here on its `sessionUpdate` kind,
//!   never through the crate's `SessionUpdate` enum, which costs the helper
//!   more than a megabyte (decision 5, check 2). Only chunks, tool calls,
//!   tool call updates and `session_info_update`'s title are decoded; every
//!   other kind is [`Update::Ignored`].
//! - **The stop reason** is read from the result's `Value`, since the crate's
//!   `StopReason` has no catch-all: a reason Brindle doesn't know is
//!   [`StopReason::Other`].
//! - **A permission request** is decoded by hand, keeping an option kind
//!   Brindle doesn't know as text ([`OptionKind::Unknown`]), and its id is in
//!   the envelope, so even a request whose params fail to decode can be
//!   answered.
//! - **Login methods** (`initialize`'s `authMethods`) are read by hand, one
//!   at a time ([`auth_methods`], OpenSpec change `agents-login`), and left
//!   out of what the schema crate decodes, so a bad method is one left out,
//!   never a lost `initialize`.
//!
//! The crate's enums are `#[non_exhaustive]`, so they are mapped onto
//! `brindle_transcript`'s closed ones here, and the reducer never matches
//! one of them.

use std::collections::BTreeMap;

use agent_client_protocol_schema::v1 as acp;
use brindle_remote_protocol::serde_json::{self, Map, Value};
use brindle_transcript::{
    Exit, Location, LoginKind, LoginMethod, PermissionKind, StopReason, ToolKind, ToolStatus,
};

use crate::acp::envelope::{BadParams, Id, RpcError};
use crate::acp::forms::{
    AgentInfo, Content, ContentKind, Continued, Initialized, MAX_LOGIN_ARG, MAX_LOGIN_ARGS,
    MAX_LOGIN_DESCRIPTION, MAX_LOGIN_ID, MAX_LOGIN_METHODS, MAX_LOGIN_METHODS_LEN, Methods,
    NewSession, NotificationKind, OptionKind, OutputForm, Patch, PermissionRequest, RequestKind,
    RequestedOption, SessionUpdate, ToolCall, ToolChange, ToolDetail, ToolItemForm,
    UNSUPPORTED_VERSION, Update,
};
use brindle_transcript::Continuation;

pub fn request_kind(method: &str) -> RequestKind {
    match method {
        "session/request_permission" => RequestKind::Permission,
        _ => RequestKind::Unoffered,
    }
}

pub fn notification_kind(method: &str) -> NotificationKind {
    match method {
        "session/update" => NotificationKind::SessionUpdate,
        "$/cancel_request" => NotificationKind::CancelRequest,
        _ => NotificationKind::Other,
    }
}

/// `initialize`'s answer, read through the schema crate. `authMethods` is
/// left to [`auth_methods`], which reads it one method at a time, so a bad
/// method never costs the answer.
pub fn initialize_result(result: &Value) -> Result<Initialized, BadParams> {
    let mut result = result.clone();
    if let Some(object) = result.as_object_mut() {
        object.remove("authMethods");
    }
    let response: acp::InitializeResponse =
        serde_json::from_value(result).map_err(|e| BadParams::from_serde(&e))?;
    let capabilities = &response.agent_capabilities;
    Ok(Initialized {
        protocol_version: response.protocol_version.as_u16(),
        continuation: Continuation::from_offers(
            capabilities.load_session,
            capabilities.session_capabilities.resume.is_some(),
        ),
        agent: response.agent_info.map(|info| AgentInfo {
            name: info.name,
            title: info.title,
            version: info.version,
        }),
    })
}

/// Reads `initialize`'s `authMethods` by hand, one method at a time
/// (`agents-login` design decision 3), so one bad method never loses the
/// others:
///
/// - no `type`, or `"agent"`: [`LoginKind::Agent`];
/// - `"terminal"`: [`LoginKind::Terminal`] with its `args` and `env`, kept
///   only when Brindle advertised `auth.terminal` (`terminal_advertised`);
///   an agent that sends one anyway breaks ACP's MUST;
/// - any other `type`, a missing, empty or non-string `id` or `name`, an
///   `id` with a control character, malformed `args` or `env` (not a list of
///   strings, or not a map of strings; a control character, an empty key or
///   one with `=`), or one past a bound: left out and counted.
///
/// `_meta` is never read, so Zed's `terminal-auth` (Copilot's) is ignored.
/// No `authMethods`, or `null`, is no methods; one that isn't a list counts
/// as one left out.
pub fn auth_methods(result: &Value, terminal_advertised: bool) -> Methods {
    let mut read = Methods::default();
    let list = match result.get("authMethods") {
        None | Some(Value::Null) => return read,
        Some(Value::Array(list)) => list,
        Some(_) => {
            read.dropped = 1;
            return read;
        }
    };
    let mut len = 0usize;
    for method in list {
        let Some((method, cut)) = auth_method(method, terminal_advertised) else {
            read.dropped += 1;
            continue;
        };
        let encoded = serde_json::to_vec(&method).map_or(usize::MAX, |bytes| bytes.len());
        if read.methods.len() == MAX_LOGIN_METHODS
            || len.saturating_add(encoded) > MAX_LOGIN_METHODS_LEN
        {
            read.dropped += 1;
            continue;
        }
        len += encoded;
        read.cut += usize::from(cut);
        read.methods.push(method);
    }
    read
}

/// One method, and whether its description was cut; `None` when it is left
/// out.
fn auth_method(method: &Value, terminal_advertised: bool) -> Option<(LoginMethod, bool)> {
    let text = |key: &str| method.get(key).and_then(Value::as_str);
    let id = text("id").filter(|id| {
        !id.is_empty() && id.len() <= MAX_LOGIN_ID && !id.chars().any(char::is_control)
    })?;
    let name = text("name").filter(|name| !name.is_empty() && name.len() <= MAX_LOGIN_ID)?;
    let description = match method.get("description") {
        None | Some(Value::Null) => None,
        Some(Value::String(description)) => Some(description.as_str()),
        Some(_) => return None,
    };
    let kind = match method.get("type") {
        None | Some(Value::Null) => LoginKind::Agent,
        Some(Value::String(kind)) => match kind.as_str() {
            "agent" => LoginKind::Agent,
            "terminal" if terminal_advertised => LoginKind::Terminal {
                args: login_args(method.get("args"))?,
                env: login_env(method.get("env"))?,
            },
            _ => return None,
        },
        Some(_) => return None,
    };
    let (description, cut) = match description {
        Some(description) if description.len() > MAX_LOGIN_DESCRIPTION => {
            (Some(cut_to(description, MAX_LOGIN_DESCRIPTION)), true)
        }
        Some(description) => (Some(description.to_owned()), false),
        None => (None, false),
    };
    Some((
        LoginMethod {
            id: id.to_owned(),
            name: name.to_owned(),
            description,
            kind,
        },
        cut,
    ))
}

/// A word a terminal login passes on: bounded, with no control character.
fn login_word(word: &str) -> bool {
    word.len() <= MAX_LOGIN_ARG && !word.chars().any(char::is_control)
}

fn login_args(args: Option<&Value>) -> Option<Vec<String>> {
    let args = match args {
        None | Some(Value::Null) => return Some(Vec::new()),
        Some(args) => args.as_array()?,
    };
    if args.len() > MAX_LOGIN_ARGS {
        return None;
    }
    args.iter()
        .map(|arg| {
            arg.as_str()
                .filter(|arg| login_word(arg))
                .map(str::to_owned)
        })
        .collect()
}

fn login_env(env: Option<&Value>) -> Option<BTreeMap<String, String>> {
    let env = match env {
        None | Some(Value::Null) => return Some(BTreeMap::new()),
        Some(env) => env.as_object()?,
    };
    if env.len() > MAX_LOGIN_ARGS {
        return None;
    }
    env.iter()
        .map(|(key, value)| {
            let value = value.as_str()?;
            let key_ok = !key.is_empty() && !key.contains('=') && login_word(key);
            (key_ok && login_word(value)).then(|| (key.clone(), value.to_owned()))
        })
        .collect()
}

/// `text` cut to at most `limit` bytes at a character boundary.
fn cut_to(text: &str, limit: usize) -> String {
    let mut cut = limit.min(text.len());
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    text[..cut].to_owned()
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

pub fn new_session_result(result: &Value) -> Result<NewSession, BadParams> {
    let response: acp::NewSessionResponse =
        serde_json::from_value(result.clone()).map_err(|e| BadParams::from_serde(&e))?;
    Ok(NewSession {
        session_id: response.session_id.0.to_string(),
    })
}

/// That `session/resume` succeeded: its result has nothing the reducer
/// reads, but must be an object the schema crate accepts.
fn resume_result(result: &Value) -> Result<(), BadParams> {
    let _: acp::ResumeSessionResponse =
        serde_json::from_value(result.clone()).map_err(|e| BadParams::from_serde(&e))?;
    Ok(())
}

/// That `session/load` succeeded, as [`resume_result`] for `session/resume`.
/// The replay came before it, as `session/update` notifications.
fn load_result(result: &Value) -> Result<(), BadParams> {
    let _: acp::LoadSessionResponse =
        serde_json::from_value(result.clone()).map_err(|e| BadParams::from_serde(&e))?;
    Ok(())
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
        "session_info_update" => Update::SessionInfo {
            title: session_title(update),
        },
        other => Update::Ignored {
            kind: other.to_owned(),
        },
    };
    Ok(SessionUpdate { session_id, update })
}

/// A `session_info_update`'s `title`, read by hand: no `title` keeps the
/// title, `null` clears it, a string sets it, and any other JSON type
/// keeps it, so one odd field never costs the update (`railway R5`).
fn session_title(update: &Value) -> Patch<String> {
    match update.get("title") {
        None => Patch::Keep,
        Some(Value::Null) => Patch::Clear,
        Some(Value::String(title)) => Patch::Set(title.clone()),
        Some(Value::Bool(_) | Value::Number(_) | Value::Array(_) | Value::Object(_)) => Patch::Keep,
    }
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

/// The fields of a tool call read by hand (design decision 3): taken out
/// before the crate decodes the rest, so one bad item never costs the
/// update.
const BY_HAND: [&str; 5] = ["content", "locations", "rawInput", "rawOutput", "_meta"];

/// The update without [`BY_HAND`]'s fields, and those fields.
fn split(update: &Value) -> (Value, Map<String, Value>) {
    let mut rest = update.clone();
    let mut taken = Map::new();
    if let Some(object) = rest.as_object_mut() {
        for key in BY_HAND {
            if let Some(value) = object.remove(key) {
                taken.insert(key.to_owned(), value);
            }
        }
    }
    (rest, taken)
}

fn tool_call(update: &Value) -> Result<ToolCall, BadParams> {
    let (rest, taken) = split(update);
    let call: acp::ToolCall =
        serde_json::from_value(rest).map_err(|e| BadParams::from_serde(&e))?;
    let mut detail = tool_detail(&taken);
    // A new call with no content has none.
    if detail.content == Patch::Keep {
        detail.content = Patch::Set(Vec::new());
    }
    Ok(ToolCall {
        call: call.tool_call_id.0.to_string(),
        title: call.title,
        kind: tool_kind(call.kind),
        status: tool_status(call.status).unwrap_or(ToolStatus::Pending),
        detail,
    })
}

fn tool_change(update: &Value) -> Result<ToolChange, BadParams> {
    let (rest, taken) = split(update);
    let update: acp::ToolCallUpdate =
        serde_json::from_value(rest).map_err(|e| BadParams::from_serde(&e))?;
    // v1 can't clear a tool call's field: `null` reads as left out.
    Ok(ToolChange {
        call: update.tool_call_id.0.to_string(),
        title: Patch::kept_or_set(update.fields.title),
        kind: Patch::kept_or_set(update.fields.kind.map(tool_kind)),
        status: Patch::kept_or_set(update.fields.status.and_then(tool_status)),
        detail: tool_detail(&taken),
    })
}

/// [`BY_HAND`]'s fields, read one at a time. A field that is absent or
/// `null` is kept (v1 can't clear one); `content` that isn't a list counts
/// as one unreadable item.
fn tool_detail(taken: &Map<String, Value>) -> ToolDetail {
    let mut detail = ToolDetail::default();
    match taken.get("content") {
        None | Some(Value::Null) => {}
        Some(Value::Array(items)) => {
            let mut kept = Vec::new();
            for item in items {
                match tool_item(item) {
                    Some(item) => kept.push(item),
                    None => detail.unreadable = detail.unreadable.saturating_add(1),
                }
            }
            detail.content = Patch::Set(kept);
        }
        Some(_) => detail.unreadable = 1,
    }
    if let Some(Value::Array(list)) = taken.get("locations") {
        detail.locations = Patch::Set(
            list.iter()
                .filter_map(|location| {
                    let location: acp::ToolCallLocation =
                        serde_json::from_value(location.clone()).ok()?;
                    Some(Location {
                        path: location.path.to_string_lossy().into_owned(),
                        line: location.line,
                    })
                })
                .collect(),
        );
    }
    detail.input = raw(taken.get("rawInput"));
    detail.result = raw(taken.get("rawOutput"));
    if let Some(meta) = taken.get("_meta") {
        detail.outputs = terminal_meta(meta);
    }
    detail
}

/// One content item, through the crate's `ToolCallContent`; an item of a
/// type the crate doesn't know is [`ToolItemForm::Unknown`], and one that
/// doesn't decode is `None`.
#[allow(
    clippy::wildcard_enum_match_arm,
    reason = "foreign enum: ToolCallContent, which is non_exhaustive"
)]
fn tool_item(item: &Value) -> Option<ToolItemForm> {
    let kind = item.get("type")?.as_str()?;
    if !matches!(kind, "content" | "diff" | "terminal") {
        return Some(ToolItemForm::Unknown {
            kind: kind.to_owned(),
        });
    }
    let item: acp::ToolCallContent = serde_json::from_value(item.clone()).ok()?;
    Some(match item {
        acp::ToolCallContent::Content(block) => ToolItemForm::Content(content(block.content)),
        acp::ToolCallContent::Diff(diff) => ToolItemForm::RawDiff {
            path: diff.path.to_string_lossy().into_owned(),
            old: diff.old_text,
            new: diff.new_text,
        },
        acp::ToolCallContent::Terminal(terminal) => ToolItemForm::Terminal {
            terminal: terminal.terminal_id.0.to_string(),
        },
        _ => ToolItemForm::Unknown {
            kind: kind.to_owned(),
        },
    })
}

/// `rawInput` or `rawOutput` as text: a string as itself, anything else as
/// indented JSON; absent or `null` keeps it.
fn raw(value: Option<&Value>) -> Patch<String> {
    match value {
        None | Some(Value::Null) => Patch::Keep,
        Some(Value::String(text)) => Patch::Set(text.clone()),
        Some(value) => serde_json::to_string_pretty(value).map_or(Patch::Keep, Patch::Set),
    }
}

/// Codex's terminal convention in `_meta` (flag 1; decision 6's As built):
/// `terminal_info`, `terminal_output_delta` and `terminal_exit`, in that
/// order. Each needs a string `terminal_id`; one without is left out.
fn terminal_meta(meta: &Value) -> Vec<OutputForm> {
    let mut outputs = Vec::new();
    let field = |key: &str| meta.get(key).filter(|value| value.is_object());
    let text = |value: &Value, key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
    if let Some(info) = field("terminal_info")
        && let Some(terminal) = text(info, "terminal_id")
    {
        outputs.push(OutputForm::Opened {
            terminal,
            cwd: text(info, "cwd"),
            command: text(info, "command"),
        });
    }
    if let Some(delta) = field("terminal_output_delta")
        && let Some(terminal) = text(delta, "terminal_id")
        && let Some(data) = text(delta, "data")
    {
        outputs.push(OutputForm::Appended {
            terminal,
            bytes: data.into_bytes(),
        });
    }
    if let Some(exit) = field("terminal_exit")
        && let Some(terminal) = text(exit, "terminal_id")
    {
        let code = exit
            .get("exit_code")
            .and_then(Value::as_i64)
            .and_then(|code| i32::try_from(code).ok());
        let exit = match (code, text(exit, "signal")) {
            (Some(code), _) => Some(Exit::Code { code }),
            (None, Some(name)) => Some(Exit::Signal { name }),
            (None, None) => None,
        };
        outputs.push(OutputForm::Exited { terminal, exit });
    }
    outputs
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

/// That `session/resume` or `session/load` succeeded, as `how` says: its
/// result has nothing the reducer reads, but must be an object the schema
/// crate accepts. A load's replay came before it, as `session/update`
/// notifications.
pub fn continued_result(how: Continued, result: &Value) -> Result<(), BadParams> {
    match how {
        Continued::Resume => resume_result(result),
        Continued::Load => load_result(result),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;
    use crate::acp::envelope::{Malformed, Message, decode};
    use crate::acp::forms::AUTH_REQUIRED;

    fn ok(line: &str) -> Message {
        decode(line.as_bytes()).unwrap_or_else(|e| panic!("{line}: {e:?}"))
    }
    use brindle_remote_protocol::serde_json::json;

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
    fn a_session_info_updates_title_is_kept_cleared_or_set() {
        let rows = [
            (json!({"_meta": {"codex": {}}}), Patch::Keep),
            (json!({"title": null}), Patch::Clear),
            (
                json!({"title": "Hello.txt file creation", "updatedAt": "2026-11-25T02:15:11.854Z"}),
                Patch::Set("Hello.txt file creation".to_owned()),
            ),
            (json!({"title": ""}), Patch::Set(String::new())),
            // Any other type is no title: the update stays.
            (json!({"title": 7}), Patch::Keep),
            (json!({"title": ["a"]}), Patch::Keep),
            (json!({"title": {"text": "a"}}), Patch::Keep),
            (json!({"title": true}), Patch::Keep),
        ];
        for (fields, title) in rows {
            let mut update = json!({"sessionUpdate": "session_info_update"});
            for (key, value) in fields.as_object().into_iter().flatten() {
                update[key] = value.clone();
            }
            assert_eq!(
                session_update(&json!({"sessionId": "s", "update": update})).map(|u| u.update),
                Ok(Update::SessionInfo { title }),
                "{fields}"
            );
        }
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
        assert_eq!(change.kind, Patch::Set(ToolKind::Other));
        assert_eq!(change.status, Patch::Keep);
        assert_eq!(change.title, Patch::Keep);
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
        assert_eq!(request.call.map(|c| c.title), Some(Patch::Set("T".into())));
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
    const CAPTURES: [(&str, &[u8]); 16] = [
        (
            "antigravity-1.5.16-reject",
            include_bytes!("../../../tests/fixtures/acp/antigravity-1.5.16-reject.log"),
        ),
        (
            "antigravity-1.5.16-trusted-workspace",
            include_bytes!("../../../tests/fixtures/acp/antigravity-1.5.16-trusted-workspace.log"),
        ),
        (
            "claude-agent-acp-0.112.0-auto",
            include_bytes!("../../../tests/fixtures/acp/claude-agent-acp-0.112.0-auto.log"),
        ),
        (
            "claude-agent-acp-0.112.0-haiku-reject",
            include_bytes!("../../../tests/fixtures/acp/claude-agent-acp-0.112.0-haiku-reject.log"),
        ),
        (
            "claude-agent-acp-0.112.0-reject",
            include_bytes!("../../../tests/fixtures/acp/claude-agent-acp-0.112.0-reject.log"),
        ),
        (
            "claude-code-acp-0.16.2-reject",
            include_bytes!("../../../tests/fixtures/acp/claude-code-acp-0.16.2-reject.log"),
        ),
        (
            "codex-acp-2.12.0-reject",
            include_bytes!("../../../tests/fixtures/acp/codex-acp-2.12.0-reject.log"),
        ),
        (
            "copilot-1.0.115-reject",
            include_bytes!("../../../tests/fixtures/acp/copilot-1.0.115-reject.log"),
        ),
        (
            "gemini-0.71.1-login-on-stdout",
            include_bytes!("../../../tests/fixtures/acp/gemini-0.71.1-login-on-stdout.log"),
        ),
        (
            "handwritten-unknown-values",
            include_bytes!("../../../tests/fixtures/acp/handwritten-unknown-values.log"),
        ),
        // `agents-tool-content` task 1.1.
        (
            "antigravity-1.5.16-edit-run",
            include_bytes!("../../../tests/fixtures/acp/antigravity-1.5.16-edit-run.log"),
        ),
        (
            "antigravity-1.5.16-edit-run-terminal",
            include_bytes!("../../../tests/fixtures/acp/antigravity-1.5.16-edit-run-terminal.log"),
        ),
        (
            "claude-agent-acp-0.112.0-edit-run",
            include_bytes!("../../../tests/fixtures/acp/claude-agent-acp-0.112.0-edit-run.log"),
        ),
        (
            "claude-agent-acp-0.112.0-edit-run-terminal",
            include_bytes!(
                "../../../tests/fixtures/acp/claude-agent-acp-0.112.0-edit-run-terminal.log"
            ),
        ),
        (
            "codex-acp-2.12.0-edit-run",
            include_bytes!("../../../tests/fixtures/acp/codex-acp-2.12.0-edit-run.log"),
        ),
        (
            "codex-acp-2.12.0-edit-run-terminal",
            include_bytes!("../../../tests/fixtures/acp/codex-acp-2.12.0-edit-run-terminal.log"),
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
            // The edit-run captures: Antigravity asks for the edit and the
            // command, Claude for the edit alone, Codex for neither.
            let asks = match name {
                n if n.ends_with("-auto") || n.ends_with("-trusted-workspace") => 0,
                n if n.starts_with("antigravity") && n.contains("-edit-run") => 2,
                n if n.starts_with("codex") && n.contains("-edit-run") => 0,
                _ => 1,
            };
            assert_eq!(kinds.3, asks, "{name}: permission requests");
        }
        assert_eq!(lines, 310 + 323);
    }

    /// What each captured agent's `initialize` offers for continuing a
    /// conversation: Copilot only `session/load`, the others both.
    #[test]
    fn the_captured_agents_offer_load_and_resume_but_copilot_only_load() {
        let mut offers = Vec::new();
        for (name, bytes) in CAPTURES {
            for line in decode_capture(bytes).1.into_iter().flatten() {
                if let Decoded::Initialized(init) = line {
                    offers.push((name, init.continuation));
                }
            }
        }
        assert_eq!(
            offers,
            [
                ("antigravity-1.5.16-reject", Continuation::ResumeAndLoad),
                (
                    "antigravity-1.5.16-trusted-workspace",
                    Continuation::ResumeAndLoad
                ),
                ("claude-agent-acp-0.112.0-auto", Continuation::ResumeAndLoad),
                (
                    "claude-agent-acp-0.112.0-haiku-reject",
                    Continuation::ResumeAndLoad
                ),
                (
                    "claude-agent-acp-0.112.0-reject",
                    Continuation::ResumeAndLoad
                ),
                ("claude-code-acp-0.16.2-reject", Continuation::ResumeAndLoad),
                ("codex-acp-2.12.0-reject", Continuation::ResumeAndLoad),
                ("copilot-1.0.115-reject", Continuation::Load),
                ("antigravity-1.5.16-edit-run", Continuation::ResumeAndLoad),
                (
                    "antigravity-1.5.16-edit-run-terminal",
                    Continuation::ResumeAndLoad
                ),
                (
                    "claude-agent-acp-0.112.0-edit-run",
                    Continuation::ResumeAndLoad
                ),
                (
                    "claude-agent-acp-0.112.0-edit-run-terminal",
                    Continuation::ResumeAndLoad
                ),
                ("codex-acp-2.12.0-edit-run", Continuation::ResumeAndLoad),
                (
                    "codex-acp-2.12.0-edit-run-terminal",
                    Continuation::ResumeAndLoad
                ),
            ]
        );
    }

    /// Hand-written capabilities no captured agent sent: resume alone,
    /// neither, and malformed ones, which read as not offered.
    #[test]
    fn initialize_reads_load_and_resume_and_a_malformed_one_as_not_offered() {
        let offer = |capabilities: Value| {
            initialize_result(&json!({"protocolVersion": 1, "agentCapabilities": capabilities}))
                .map(|init| init.continuation)
        };
        assert_eq!(
            offer(json!({"sessionCapabilities": {"resume": {}}})),
            Ok(Continuation::Resume)
        );
        assert_eq!(
            offer(json!({"loadSession": false, "sessionCapabilities": {"list": {}}})),
            Ok(Continuation::Neither)
        );
        assert_eq!(offer(json!({})), Ok(Continuation::Neither));
        assert_eq!(
            offer(json!({"loadSession": true, "sessionCapabilities": {"resume": null}})),
            Ok(Continuation::Load)
        );
        assert_eq!(
            offer(json!({"loadSession": "yes", "sessionCapabilities": {"resume": 7}})),
            Ok(Continuation::Neither)
        );
        assert_eq!(
            offer(json!({"loadSession": true, "sessionCapabilities": "all"})),
            Ok(Continuation::Load)
        );
        assert_eq!(
            initialize_result(&json!({"protocolVersion": 1})).map(|init| init.continuation),
            Ok(Continuation::Neither)
        );
    }

    // agents-login 3.1: the login methods.

    /// The captures made with `clientCapabilities.auth.terminal: true`
    /// (`agents-login` task 1.1).
    const AUTH_CAPTURES: [(&str, &[u8]); 4] = [
        (
            "antigravity-1.5.16-auth-terminal",
            include_bytes!("../../../tests/fixtures/acp/antigravity-1.5.16-auth-terminal.log"),
        ),
        (
            "claude-code-acp-0.16.2-auth-terminal",
            include_bytes!("../../../tests/fixtures/acp/claude-code-acp-0.16.2-auth-terminal.log"),
        ),
        (
            "codex-acp-2.12.0-auth-terminal",
            include_bytes!("../../../tests/fixtures/acp/codex-acp-2.12.0-auth-terminal.log"),
        ),
        (
            "copilot-1.0.115-auth-terminal",
            include_bytes!("../../../tests/fixtures/acp/copilot-1.0.115-auth-terminal.log"),
        ),
    ];

    /// The result of each answered `initialize` in a capture.
    fn initialize_answers(bytes: &[u8]) -> Vec<Value> {
        let mut asked = Vec::new();
        let mut answers = Vec::new();
        for (direction, line) in capture_lines(bytes) {
            match decode(line) {
                Ok(Message::Request { id, method, .. })
                    if direction == "c>a" && method == "initialize" =>
                {
                    asked.push(id.encode());
                }
                Ok(Message::Response { id, result }) if asked.contains(&id.encode()) => {
                    answers.push(result);
                }
                Ok(_) | Err(_) => {}
            }
        }
        answers
    }

    fn agent_method(id: &str, name: &str, description: Option<&str>) -> LoginMethod {
        LoginMethod {
            id: id.into(),
            name: name.into(),
            description: description.map(Into::into),
            kind: LoginKind::Agent,
        }
    }

    fn ids(methods: &Methods) -> Vec<&str> {
        methods.methods.iter().map(|m| m.id.as_str()).collect()
    }

    #[test]
    fn every_captured_initialize_answer_gives_its_agents_methods() {
        let antigravity = [
            "oauth-personal",
            "oauth-business",
            "gemini-api-key",
            "agent-platform",
        ];
        let mut seen = Vec::new();
        for (name, bytes) in CAPTURES.iter().chain(AUTH_CAPTURES.iter()) {
            for answer in initialize_answers(bytes) {
                // Every capture but the terminal one: the same with or
                // without auth.terminal, and nothing left out.
                for advertised in [true, false] {
                    let read = auth_methods(&answer, advertised);
                    let expected: Vec<&str> = match *name {
                        // Those answering an `initialize` that offered
                        // `auth.terminal`.
                        n if n.starts_with("antigravity")
                            && (n.ends_with("-auth-terminal") || n.contains("-edit-run"))
                            && advertised =>
                        {
                            antigravity
                                .iter()
                                .copied()
                                .chain(["terminal-login"])
                                .collect()
                        }
                        n if n.starts_with("antigravity") => antigravity.to_vec(),
                        // 0.112's two terminal logins, offered to the later
                        // `initialize` only.
                        n if n.starts_with("claude-agent-acp")
                            && n.contains("-edit-run")
                            && advertised =>
                        {
                            vec!["claude-ai-login", "console-login"]
                        }
                        n if n.starts_with("claude-agent-acp") => Vec::new(),
                        n if n.starts_with("claude-code-acp") => vec!["claude-login"],
                        n if n.starts_with("codex") => vec!["api-key", "chat-gpt"],
                        n if n.starts_with("copilot") => vec!["copilot-login"],
                        n => panic!("{n}: an initialize answer this test doesn't expect"),
                    };
                    assert_eq!(ids(&read), expected, "{name} ({advertised})");
                    let dropped = match *name {
                        _ if advertised => 0,
                        n if n.starts_with("antigravity")
                            && (n.ends_with("-auth-terminal") || n.contains("-edit-run")) =>
                        {
                            1
                        }
                        n if n.starts_with("claude-agent-acp") && n.contains("-edit-run") => 2,
                        _ => 0,
                    };
                    assert_eq!(
                        (read.dropped, read.cut),
                        (dropped, 0),
                        "{name} ({advertised})"
                    );
                    // `initialize` itself still reads.
                    assert!(initialize_result(&answer).is_ok(), "{name}");
                }
                seen.push(*name);
            }
        }
        assert_eq!(seen.len(), 18, "{seen:?}");
    }

    #[test]
    fn the_captured_methods_are_agent_methods_but_antigravitys_terminal_login() {
        let answer = |bytes| initialize_answers(bytes).remove(0);
        let read = auth_methods(&answer(AUTH_CAPTURES[0].1), true);
        assert_eq!(
            read.methods.last(),
            Some(&LoginMethod {
                id: "terminal-login".into(),
                name: "Log in from the terminal".into(),
                description: Some(
                    "Log in with your Google account by pasting a code in a terminal; works over SSH"
                        .into()
                ),
                kind: LoginKind::Terminal {
                    args: vec!["--login".into()],
                    env: BTreeMap::new(),
                },
            })
        );
        // Copilot's `_meta.terminal-auth` is never read: an agent method.
        let read = auth_methods(&answer(AUTH_CAPTURES[3].1), true);
        assert_eq!(
            read.methods,
            [agent_method(
                "copilot-login",
                "Log in with Copilot CLI",
                Some("Run `copilot login` in the terminal")
            )]
        );
        // Codex's `_meta.api-key` likewise.
        let read = auth_methods(&answer(AUTH_CAPTURES[2].1), true);
        assert_eq!(
            read.methods,
            [
                agent_method("api-key", "API Key", Some("Use an API key to authenticate")),
                agent_method("chat-gpt", "ChatGPT", Some("Use ChatGPT to authenticate")),
            ]
        );
    }

    #[test]
    fn a_method_of_another_type_or_malformed_is_left_out_and_counted() {
        let read = auth_methods(
            &json!({"authMethods": [
                {"id": "env", "name": "Env", "type": "env_var", "vars": [{"name": "KEY"}]},
                {"id": "plain", "name": "Plain"},
                {"name": "No id"},
                {"id": 7, "name": "Number id"},
                {"id": "", "name": "Empty id"},
                {"id": "a\u{1b}b", "name": "Escape in its id"},
                {"id": "no-name"},
                {"id": "typed", "name": "Typed", "type": 1},
                {"id": "described", "name": "Described", "description": ["x"]},
                {"id": "agent", "name": "Agent", "type": "agent", "description": null},
                "a string",
            ]}),
            true,
        );
        assert_eq!(
            read.methods,
            [
                agent_method("plain", "Plain", None),
                agent_method("agent", "Agent", None)
            ]
        );
        assert_eq!((read.dropped, read.cut), (9, 0));
        // None, or null, is no methods; anything else not a list, one
        // left out.
        assert_eq!(auth_methods(&json!({}), true), Methods::default());
        assert_eq!(
            auth_methods(&json!({"authMethods": null}), true),
            Methods::default()
        );
        assert_eq!(
            auth_methods(&json!({"authMethods": {"id": "x"}}), true).dropped,
            1
        );
        // `initialize` reads whatever the methods are.
        assert!(
            initialize_result(&json!({"protocolVersion": 1, "authMethods": [{"type": 1}, 3]}))
                .is_ok()
        );
    }

    #[test]
    fn past_16_methods_the_rest_are_left_out() {
        let methods: Vec<Value> = (0..17)
            .map(|n| json!({"id": format!("m{n}"), "name": format!("Method {n}")}))
            .collect();
        let read = auth_methods(&json!({ "authMethods": methods }), true);
        assert_eq!(read.methods.len(), MAX_LOGIN_METHODS);
        assert_eq!(read.methods.last().map(|m| m.id.as_str()), Some("m15"));
        assert_eq!(read.dropped, 1);
    }

    #[test]
    fn an_over_long_description_is_cut_at_a_character_and_an_over_long_id_or_name_left_out() {
        let long = "é".repeat(MAX_LOGIN_DESCRIPTION);
        let read = auth_methods(
            &json!({"authMethods": [
                {"id": "a", "name": "A", "description": long},
                {"id": "x".repeat(MAX_LOGIN_ID + 1), "name": "Long id"},
                {"id": "b", "name": "n".repeat(MAX_LOGIN_ID + 1)},
                {"id": "x".repeat(MAX_LOGIN_ID), "name": "n".repeat(MAX_LOGIN_ID)},
            ]}),
            true,
        );
        assert_eq!(read.methods.len(), 2);
        let description = read.methods[0].description.as_deref().unwrap_or_default();
        assert_eq!(description.len(), MAX_LOGIN_DESCRIPTION);
        assert!(description.chars().all(|c| c == 'é'));
        assert_eq!((read.dropped, read.cut), (2, 1));
    }

    #[test]
    fn methods_past_their_total_length_are_left_out_so_their_event_fits_a_frame() {
        // About 96 KiB each: two fit, the third doesn't.
        let big = |n: usize| {
            json!({"id": format!("t{n}"), "name": "T", "type": "terminal",
                   "args": vec!["a".repeat(MAX_LOGIN_ARG); 24]})
        };
        let mut methods: Vec<Value> = (0..MAX_LOGIN_METHODS - 1).map(big).collect();
        methods.push(json!({"id": "small", "name": "Small"}));
        let read = auth_methods(&json!({ "authMethods": methods }), true);
        assert_eq!(ids(&read), ["t0", "t1", "small"]);
        assert_eq!(read.dropped, MAX_LOGIN_METHODS - 3);
        let len = serde_json::to_vec(&read.methods).map_or(usize::MAX, |bytes| bytes.len());
        assert!(len <= MAX_LOGIN_METHODS_LEN + 16, "{len}");
        // At the most a terminal method can be on its own, it is left out
        // too: past the total by itself.
        let env: Map<String, Value> = (0..MAX_LOGIN_ARGS)
            .map(|k| (format!("K{k}"), json!("v".repeat(MAX_LOGIN_ARG))))
            .collect();
        let largest = json!({"id": "x", "name": "X", "type": "terminal",
                             "args": vec!["a".repeat(MAX_LOGIN_ARG); MAX_LOGIN_ARGS], "env": env});
        let read = auth_methods(&json!({ "authMethods": [largest] }), true);
        assert_eq!((read.methods.len(), read.dropped), (0, 1));
    }

    #[test]
    fn a_terminal_method_is_kept_only_when_brindle_advertised_auth_terminal() {
        let answer = json!({"authMethods": [
            {"id": "cli-login", "name": "CLI", "type": "terminal", "args": ["login"], "env": {"LOGIN_MODE": "device", "B": ""}},
            {"id": "bare", "name": "Bare", "type": "terminal"},
            {"id": "nulls", "name": "Nulls", "type": "terminal", "args": null, "env": null},
        ]});
        let read = auth_methods(&answer, true);
        let terminal = |id: &str, name: &str, args: &[&str], env: &[(&str, &str)]| LoginMethod {
            id: id.into(),
            name: name.into(),
            description: None,
            kind: LoginKind::Terminal {
                args: args.iter().map(|a| (*a).into()).collect(),
                env: env
                    .iter()
                    .map(|(k, v)| ((*k).into(), (*v).into()))
                    .collect(),
            },
        };
        assert_eq!(
            read.methods,
            [
                terminal(
                    "cli-login",
                    "CLI",
                    &["login"],
                    &[("B", ""), ("LOGIN_MODE", "device")]
                ),
                terminal("bare", "Bare", &[], &[]),
                terminal("nulls", "Nulls", &[], &[]),
            ]
        );
        assert_eq!(read.dropped, 0);
        let read = auth_methods(&answer, false);
        assert!(read.methods.is_empty());
        assert_eq!(read.dropped, 3);
    }

    #[test]
    fn a_terminal_method_with_malformed_or_over_long_args_or_env_is_left_out() {
        let many: Vec<String> = (0..=MAX_LOGIN_ARGS).map(|n| n.to_string()).collect();
        let many_env: Map<String, Value> = (0..=MAX_LOGIN_ARGS)
            .map(|n| (format!("K{n}"), json!("v")))
            .collect();
        let long = "a".repeat(MAX_LOGIN_ARG + 1);
        let bad = [
            json!({"args": ["auth", "--port", 8123]}),
            json!({"args": "login"}),
            json!({"args": ["log\nin"]}),
            json!({"args": [long]}),
            json!({"args": many}),
            json!({"env": {"DEBUG": 1}}),
            json!({"env": ["A=1"]}),
            json!({"env": {"": "x"}}),
            json!({"env": {"A=B": "x"}}),
            json!({"env": {"A": "x\u{0}"}}),
            json!({"env": {"A": long}}),
            json!({"env": many_env}),
        ];
        for fields in bad {
            let mut method = json!({"id": "t", "name": "T", "type": "terminal"});
            for (key, value) in fields.as_object().into_iter().flatten() {
                method[key] = value.clone();
            }
            let read = auth_methods(&json!({ "authMethods": [method] }), true);
            assert_eq!((read.methods.len(), read.dropped), (0, 1), "{fields}");
        }
        // At the bounds, kept.
        let at: Vec<String> = (0..MAX_LOGIN_ARGS)
            .map(|_| "a".repeat(MAX_LOGIN_ARG))
            .collect();
        let read = auth_methods(
            &json!({"authMethods": [{"id": "t", "name": "T", "type": "terminal", "args": at}]}),
            true,
        );
        assert_eq!(read.methods.len(), 1);
    }

    #[test]
    fn resume_and_load_results_are_objects_or_null() {
        for result in [
            json!({}),
            json!({"modes": null}),
            json!({"configOptions": [1]}),
            Value::Null,
        ] {
            assert_eq!(resume_result(&result), Ok(()), "{result}");
            assert_eq!(load_result(&result), Ok(()), "{result}");
        }
        for result in [json!("ok"), json!(7), json!(true)] {
            assert!(resume_result(&result).is_err(), "{result}");
            assert!(load_result(&result).is_err(), "{result}");
        }
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
                        Update::SessionInfo { .. } => "session info",
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
            "session info",
            "ignored",
        ] {
            assert!(seen.contains(&kind), "{kind}");
        }
    }

    /// Claude, Antigravity and Copilot name the conversation; Codex sends
    /// `_meta` alone, which keeps the title.
    #[test]
    fn the_captured_session_info_updates_set_a_title_but_codexs_keep_it() {
        let titles = |name: &str| -> Vec<Patch<String>> {
            let (_, bytes) = CAPTURES
                .iter()
                .find(|(capture, _)| *capture == name)
                .unwrap_or_else(|| panic!("no capture {name}"));
            decode_capture(bytes)
                .1
                .into_iter()
                .flatten()
                .filter_map(|line| match line {
                    Decoded::Update(Update::SessionInfo { title }) => Some(title),
                    Decoded::Update(_)
                    | Decoded::Permission(_)
                    | Decoded::Initialized(_)
                    | Decoded::NewSession
                    | Decoded::Stopped(_)
                    | Decoded::NotBrindles
                    | Decoded::Ignored
                    | Decoded::Bad(_) => None,
                })
                .collect()
        };
        let set = |title: &str| Patch::Set(title.to_owned());
        assert_eq!(
            titles("claude-agent-acp-0.112.0-auto"),
            [set("Hello.txt file creation")]
        );
        assert_eq!(
            titles("antigravity-1.5.16-reject"),
            [set("Create Hello.txt File")]
        );
        let copilot = titles("copilot-1.0.115-reject");
        assert!(
            matches!(&copilot[..], [Patch::Set(title)] if title.starts_with("Please create a file named hello.txt")),
            "{copilot:?}"
        );
        assert_eq!(titles("codex-acp-2.12.0-reject"), vec![Patch::Keep; 4]);
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
            title: Patch::Set("Teleport a file".into()),
            kind: Patch::Set(ToolKind::Other),
            status,
            detail: ToolDetail::default(),
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
                    detail: ToolDetail {
                        content: Patch::Set(Vec::new()),
                        ..ToolDetail::default()
                    },
                })),
                Decoded::Update(Update::ToolChanged(ToolChange {
                    title: Patch::Keep,
                    ..teleport(Patch::Keep)
                })),
                Decoded::Bad(String::new()),
                Decoded::Permission(PermissionRequest {
                    session_id: "sess-1".into(),
                    call: Some(teleport(Patch::Keep)),
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
                    call: Some(teleport(Patch::Keep)),
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
            "../../../tests/fixtures/acp/initialize-32022.jsonl"
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
            "../../../tests/fixtures/acp/initialize-v2-reply.jsonl"
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
            "../../../tests/fixtures/acp/prompt-auth-required.jsonl"
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
                let _ = resume_result(&value);
                let _ = load_result(&value);
                let _ = prompt_result(&value);
                let _ = cancel_request(&value);
            }
        }
    }

    // --- tool content (`agents-tool-content` task 3.4) -------------------

    /// Every tool call, update and permission request's detail in a capture.
    fn details(bytes: &[u8]) -> Vec<ToolDetail> {
        decode_capture(bytes)
            .1
            .into_iter()
            .flatten()
            .filter_map(|line| match line {
                Decoded::Update(Update::ToolCall(call)) => Some(call.detail),
                Decoded::Update(Update::ToolChanged(change)) => Some(change.detail),
                Decoded::Permission(request) => request.call.map(|call| call.detail),
                Decoded::Update(_)
                | Decoded::Initialized(_)
                | Decoded::NewSession
                | Decoded::Stopped(_)
                | Decoded::NotBrindles
                | Decoded::Ignored
                | Decoded::Bad(_) => None,
            })
            .collect()
    }

    fn capture(name: &str) -> &'static [u8] {
        CAPTURES
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, bytes)| *bytes)
            .unwrap_or_else(|| panic!("no capture {name}"))
    }

    #[test]
    fn every_captured_tool_calls_content_reads_whole() {
        let mut items = 0;
        for (name, bytes) in CAPTURES {
            for detail in details(bytes) {
                assert_eq!(detail.unreadable, 0, "{name}: {detail:?}");
                if let Patch::Set(content) = &detail.content {
                    items += content.len();
                    for item in content {
                        assert!(
                            !matches!(item, ToolItemForm::Unknown { .. }),
                            "{name}: {item:?}"
                        );
                    }
                }
            }
        }
        assert!(items > 20, "{items}");
        // The edits: Claude's fragment and Antigravity's request and
        // whole file, each a diff with its old text.
        for name in [
            "claude-agent-acp-0.112.0-edit-run",
            "antigravity-1.5.16-edit-run",
        ] {
            let diffs: Vec<(String, Option<String>, String)> = details(capture(name))
                .into_iter()
                .filter_map(|detail| match detail.content {
                    Patch::Set(items) => Some(items),
                    Patch::Keep | Patch::Clear => None,
                })
                .flatten()
                .filter_map(|item| match item {
                    ToolItemForm::RawDiff { path, old, new } => Some((path, old, new)),
                    ToolItemForm::Content(_)
                    | ToolItemForm::Terminal { .. }
                    | ToolItemForm::Unknown { .. } => None,
                })
                .collect();
            assert!(!diffs.is_empty(), "{name}");
            for (path, old, new) in diffs {
                assert!(path.ends_with("notes.txt"), "{name}: {path}");
                assert!(old.is_some_and(|old| old.contains("apple")), "{name}");
                assert!(new.contains("banana"), "{name}");
            }
        }
    }

    #[test]
    fn codexs_meta_updates_read_as_output_forms() {
        let outputs: Vec<OutputForm> = details(capture("codex-acp-2.12.0-edit-run"))
            .into_iter()
            .flat_map(|detail| detail.outputs)
            .collect();
        assert!(outputs.iter().any(|form| matches!(form,
            OutputForm::Opened { cwd: Some(cwd), command: None, .. } if cwd == "/home/user/scratch")));
        assert!(outputs.iter().any(|form| matches!(form,
            OutputForm::Appended { bytes, .. } if bytes == b"1\n2\n3\n4\n")));
        assert!(outputs.iter().any(|form| matches!(
            form,
            OutputForm::Exited {
                exit: Some(Exit::Code { code: 0 }),
                ..
            }
        )));
        // The rejected command's exit has neither a code nor a signal.
        let rejected: Vec<OutputForm> = details(capture("codex-acp-2.12.0-reject"))
            .into_iter()
            .flat_map(|detail| detail.outputs)
            .collect();
        assert!(
            rejected
                .iter()
                .any(|form| matches!(form, OutputForm::Exited { exit: None, .. })),
            "{rejected:?}"
        );
    }

    fn changed(update: Value) -> ToolChange {
        let mut params =
            json!({"sessionUpdate": "tool_call_update", "toolCallId": "c1", "status": "completed"});
        if let (Some(params), Some(update)) = (params.as_object_mut(), update.as_object()) {
            params.extend(update.clone());
        }
        match session_update(&json!({"sessionId": "s", "update": params})) {
            Ok(SessionUpdate {
                update: Update::ToolChanged(change),
                ..
            }) => change,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_bad_item_is_left_out_and_counted_keeping_the_update() {
        let change = changed(json!({"content": [
            {"type": "content", "content": {"type": "text", "text": "ok"}},
            {"type": "diff", "path": "/a", "oldText": "x"},
            {"type": "content"},
            {"no": "type"},
            {"type": "hologram", "id": 1},
            {"type": "content", "content": {"type": "image", "data": "", "mimeType": "image/png"}},
            {"type": "diff", "path": "/b", "newText": "new"},
            {"type": "terminal", "terminalId": "t1"},
        ]}));
        assert_eq!(change.status, Patch::Set(ToolStatus::Completed));
        assert_eq!(change.detail.unreadable, 3);
        assert_eq!(
            change.detail.content,
            Patch::Set(vec![
                ToolItemForm::Content(Content::Text("ok".into())),
                ToolItemForm::Unknown {
                    kind: "hologram".into()
                },
                ToolItemForm::Content(Content::Other(ContentKind::Image)),
                ToolItemForm::RawDiff {
                    path: "/b".into(),
                    old: None,
                    new: "new".into()
                },
                ToolItemForm::Terminal {
                    terminal: "t1".into()
                },
            ])
        );
        // Content that isn't a list is one unreadable item.
        let change = changed(json!({"content": {"type": "terminal"}}));
        assert_eq!(
            (change.detail.unreadable, change.detail.content),
            (1, Patch::Keep)
        );
        // No content, or null, keeps the call's.
        assert_eq!(changed(json!({})).detail.content, Patch::Keep);
        assert_eq!(
            changed(json!({"content": null})).detail.content,
            Patch::Keep
        );
    }

    #[test]
    fn raw_values_read_as_text_and_locations_by_their_path() {
        let change = changed(json!({
            "rawInput": "ls -la",
            "rawOutput": {"output": "a\n", "exit_code": 0},
            "locations": [{"path": "/w/a.rs", "line": 3}, {"path": "/w/b.rs"}, {"line": 4}],
        }));
        assert_eq!(change.detail.input, Patch::Set("ls -la".into()));
        assert_eq!(
            change.detail.result,
            // Indented, in the order sent.
            Patch::Set("{\n  \"output\": \"a\\n\",\n  \"exit_code\": 0\n}".into())
        );
        assert_eq!(
            change.detail.locations,
            Patch::Set(vec![
                Location {
                    path: "/w/a.rs".into(),
                    line: Some(3)
                },
                Location {
                    path: "/w/b.rs".into(),
                    line: None
                },
            ])
        );
        // A 9 MiB string reads whole; the reducer bounds it.
        let huge = "r".repeat(9 * 1024 * 1024);
        assert_eq!(
            changed(json!({"rawInput": huge})).detail.input,
            Patch::Set(huge)
        );
        assert_eq!(changed(json!({"rawInput": null})).detail.input, Patch::Keep);
    }

    #[test]
    fn a_new_call_without_content_has_none() {
        let Ok(SessionUpdate {
            update: Update::ToolCall(call),
            ..
        }) = session_update(&json!({"sessionId": "s", "update":
            {"sessionUpdate": "tool_call", "toolCallId": "c1", "title": "Run"}}))
        else {
            panic!("didn't decode");
        };
        assert_eq!(call.detail.content, Patch::Set(Vec::new()));
        // A permission request's toolCall reads the same way.
        let Ok(request) = permission_request(&json!({"sessionId": "s", "options": [],
            "toolCall": {"toolCallId": "c1", "rawInput": {"command": "rm"},
                "content": [{"type": "diff", "path": "/a", "oldText": "1", "newText": "2"}]}}))
        else {
            panic!("didn't decode");
        };
        let detail = request.call.map(|call| call.detail).unwrap_or_default();
        assert_eq!(
            detail.input,
            Patch::Set("{\n  \"command\": \"rm\"\n}".into())
        );
        assert!(matches!(&detail.content, Patch::Set(items) if items.len() == 1));
    }
}
