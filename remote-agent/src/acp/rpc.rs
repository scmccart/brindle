//! The core's side of the JSON-RPC conversation with an agent (design
//! decisions 4, 8 and 11): [`Rpc`] keeps the next outgoing id, the core's
//! requests still awaiting a response and what each was for, and the
//! agent's permission requests still waiting for a person, by the
//! transcript entry that shows each, with the JSON-RPC id to answer it by.
//! Its encoders write every line the core sends, each one line ending in
//! `\n`.
//!
//! The lines are written here, not through the schema crate's types: the
//! crate encodes `clientCapabilities` with explicit `false` fields, and
//! decision 4 offers `{}`. Every string goes through JSON's own escaping,
//! so no line holds a raw newline, and a request id from the agent is
//! echoed as it was sent.

use std::collections::BTreeMap;

use brindle_remote_protocol::serde_json::Value;
use brindle_transcript::EntryId;

use super::wire::Id;

/// The most permission requests an agent may have waiting at once; one
/// past it is answered `cancelled` at once, with a notice (decision 4).
pub const MAX_PENDING_PERMISSIONS: usize = 16;

/// The ACP version Brindle asks for and speaks.
pub const PROTOCOL_VERSION: u16 = 1;

/// What one of the core's requests was for, which says how its response
/// is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Purpose {
    Initialize,
    NewSession,
    Prompt,
    /// `session/resume`: continue a conversation with no replay.
    Resume,
    /// `session/load`: continue a conversation, replayed first.
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

    fn message(self) -> &'static str {
        match self {
            Self::MethodNotFound => "Method not found",
            Self::InvalidRequest => "Invalid Request",
        }
    }
}

/// The cap of [`MAX_PENDING_PERMISSIONS`] is reached: answer this request
/// `cancelled` ([`permission_answer`] with no option).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Full;

/// Why checkpointed parts don't make an [`Rpc`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BadParts {
    /// An outgoing id at or past the next id, which a later request would
    /// reuse.
    IdNotBelowNext,
    /// More pending permission requests than the cap.
    TooManyPending,
}

/// The core's JSON-RPC bookkeeping for one agent.
#[derive(Debug, Clone, PartialEq)]
pub struct Rpc {
    next_id: u64,
    outgoing: BTreeMap<u64, Purpose>,
    permissions: BTreeMap<EntryId, Id>,
}

impl Default for Rpc {
    fn default() -> Self {
        Self::new()
    }
}

impl Rpc {
    /// Before the first request: ids start at 1.
    pub fn new() -> Self {
        Self {
            next_id: 1,
            outgoing: BTreeMap::new(),
            permissions: BTreeMap::new(),
        }
    }

    /// As a checkpoint left it.
    pub fn from_parts(
        next_id: u64,
        outgoing: BTreeMap<u64, Purpose>,
        permissions: BTreeMap<EntryId, Id>,
    ) -> Result<Self, BadParts> {
        if outgoing.keys().any(|id| *id >= next_id) {
            return Err(BadParts::IdNotBelowNext);
        }
        if permissions.len() > MAX_PENDING_PERMISSIONS {
            return Err(BadParts::TooManyPending);
        }
        Ok(Self {
            next_id,
            outgoing,
            permissions,
        })
    }

    /// The id the next request gets.
    pub fn next_id(&self) -> u64 {
        self.next_id
    }

    /// The core's requests awaiting a response, by id.
    pub fn outgoing(&self) -> &BTreeMap<u64, Purpose> {
        &self.outgoing
    }

    /// The agent's permission requests awaiting an answer, by entry.
    pub fn permissions(&self) -> &BTreeMap<EntryId, Id> {
        &self.permissions
    }

    fn request(&mut self, purpose: Purpose, method: &str, params: &str) -> Vec<u8> {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        self.outgoing.insert(id, purpose);
        line(format!(
            r#"{{"jsonrpc":"2.0","id":{id},"method":{},"params":{params}}}"#,
            quote(method)
        ))
    }

    /// `initialize`, asking for [`PROTOCOL_VERSION`] with no client
    /// capabilities, as Brindle at `version`.
    pub fn initialize(&mut self, version: &str) -> Vec<u8> {
        let params = format!(
            r#"{{"protocolVersion":{PROTOCOL_VERSION},"clientCapabilities":{{}},"clientInfo":{{"name":"brindle","version":{}}}}}"#,
            quote(version)
        );
        self.request(Purpose::Initialize, "initialize", &params)
    }

    /// `session/new` in `cwd` (already absolute), with no MCP servers.
    pub fn new_session(&mut self, cwd: &str) -> Vec<u8> {
        let params = format!(r#"{{"cwd":{},"mcpServers":[]}}"#, quote(cwd));
        self.request(Purpose::NewSession, "session/new", &params)
    }

    /// `session/resume` of conversation `session` in `cwd` (its own
    /// directory), with no MCP servers.
    pub fn resume(&mut self, session: &str, cwd: &str) -> Vec<u8> {
        self.request(Purpose::Resume, "session/resume", &continuing(session, cwd))
    }

    /// `session/load` of conversation `session` in `cwd`, as
    /// [`resume`](Self::resume).
    pub fn load(&mut self, session: &str, cwd: &str) -> Vec<u8> {
        self.request(Purpose::Load, "session/load", &continuing(session, cwd))
    }

    /// `session/prompt` with one text block.
    pub fn prompt(&mut self, session: &str, text: &str) -> Vec<u8> {
        let params = format!(
            r#"{{"sessionId":{},"prompt":[{{"type":"text","text":{}}}]}}"#,
            quote(session),
            quote(text)
        );
        self.request(Purpose::Prompt, "session/prompt", &params)
    }

    /// The request a response or error answers, which is then no longer
    /// awaited; `None` for an id the core isn't waiting on (a string, a
    /// number it never sent, one already answered).
    pub fn answered(&mut self, id: &Id) -> Option<Purpose> {
        self.outgoing.remove(&id.as_u64()?)
    }

    /// Stops awaiting every request for `purpose` (a prompt whose response
    /// a truncated tail may have lost), saying whether one was awaited.
    pub fn forget(&mut self, purpose: Purpose) -> bool {
        let before = self.outgoing.len();
        self.outgoing.retain(|_, awaited| *awaited != purpose);
        self.outgoing.len() != before
    }

    /// Forgets every request awaited and every permission request waiting,
    /// keeping the next id: for an agent started again, to which the ended
    /// agent's requests mean nothing, and whose ids must not repeat them.
    pub fn restart(&mut self) {
        self.outgoing.clear();
        self.permissions.clear();
    }

    /// Keeps a permission request, shown as `entry`, until it is answered.
    /// Past the cap it isn't kept, and the caller answers it `cancelled`.
    pub fn hold_permission(&mut self, entry: EntryId, id: Id) -> Result<(), Full> {
        if self.permissions.len() >= MAX_PENDING_PERMISSIONS {
            return Err(Full);
        }
        self.permissions.insert(entry, id);
        Ok(())
    }

    /// Answers the permission request shown as `entry` with `option`, or
    /// `cancelled` with none; `None` when it isn't waiting (answered,
    /// withdrawn, or never one).
    pub fn answer_permission(&mut self, entry: EntryId, option: Option<&str>) -> Option<Vec<u8>> {
        let id = self.permissions.remove(&entry)?;
        Some(permission_answer(&id, option))
    }

    /// Answers every waiting permission request `cancelled` (after a
    /// `session/cancel`), in entry order.
    pub fn cancel_permissions(&mut self) -> Vec<(EntryId, Vec<u8>)> {
        std::mem::take(&mut self.permissions)
            .into_iter()
            .map(|(entry, id)| (entry, permission_answer(&id, None)))
            .collect()
    }

    /// The agent's `$/cancel_request` for one of its permission requests:
    /// answers it `cancelled` and says which entry it was; `None` when no
    /// waiting request has that id.
    pub fn withdraw(&mut self, id: &Id) -> Option<(EntryId, Vec<u8>)> {
        let entry = self
            .permissions
            .iter()
            .find_map(|(entry, waiting)| (waiting == id).then_some(*entry))?;
        self.answer_permission(entry, None)
            .map(|answer| (entry, answer))
    }
}

/// `session/cancel` for `session`, a notification.
pub fn cancel(session: &str) -> Vec<u8> {
    line(format!(
        r#"{{"jsonrpc":"2.0","method":"session/cancel","params":{{"sessionId":{}}}}}"#,
        quote(session)
    ))
}

/// The answer to permission request `id`: the option chosen, or
/// `cancelled` with none.
pub fn permission_answer(id: &Id, option: Option<&str>) -> Vec<u8> {
    let outcome = match option {
        Some(option) => format!(r#"{{"outcome":"selected","optionId":{}}}"#, quote(option)),
        None => r#"{"outcome":"cancelled"}"#.to_owned(),
    };
    line(format!(
        r#"{{"jsonrpc":"2.0","id":{},"result":{{"outcome":{outcome}}}}}"#,
        id.encode()
    ))
}

/// An error answering request `id`, or a null id when the request's id
/// couldn't be read.
pub fn error(id: Option<&Id>, refusal: Refusal) -> Vec<u8> {
    let id = id.map_or_else(|| "null".to_owned(), Id::encode);
    line(format!(
        r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":{},"message":{}}}}}"#,
        refusal.code(),
        quote(refusal.message())
    ))
}

/// The params of `session/resume` and `session/load`.
fn continuing(session: &str, cwd: &str) -> String {
    format!(
        r#"{{"sessionId":{},"cwd":{},"mcpServers":[]}}"#,
        quote(session),
        quote(cwd)
    )
}

/// `text` as a JSON string.
fn quote(text: &str) -> String {
    Value::from(text).to_string()
}

fn line(text: String) -> Vec<u8> {
    let mut bytes = text.into_bytes();
    bytes.push(b'\n');
    bytes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::acp::wire::{Message, decode};
    use brindle_remote_protocol::serde_json::{self, json};

    fn text(bytes: &[u8]) -> &str {
        std::str::from_utf8(bytes).unwrap_or_else(|e| panic!("{e}"))
    }

    /// The line the capture script sent for `prefix`, as Brindle would.
    fn captured(capture: &str, prefix: &str) -> String {
        capture
            .lines()
            .filter_map(|line| line.strip_prefix("c>a\t"))
            .find(|line| line.starts_with(prefix))
            .map(|line| format!("{line}\n"))
            .unwrap_or_else(|| panic!("no {prefix}"))
    }

    const CLAUDE: &str = include_str!("../../tests/fixtures/acp/claude-code-acp-0.16.2-reject.log");
    const ANTIGRAVITY: &str =
        include_str!("../../tests/fixtures/acp/antigravity-1.5.16-reject.log");

    #[test]
    fn initialize_asks_for_version_1_with_empty_capabilities() {
        let mut rpc = Rpc::new();
        assert_eq!(
            text(&rpc.initialize("0.3.0")),
            captured(CLAUDE, r#"{"jsonrpc":"2.0","id":1,"method":"initialize""#)
        );
        assert_eq!(
            text(&Rpc::new().initialize("1.2.3")),
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":1,\
             \"clientCapabilities\":{},\"clientInfo\":{\"name\":\"brindle\",\"version\":\"1.2.3\"}}}\n"
        );
    }

    #[test]
    fn session_new_names_the_directory_and_no_mcp_servers() {
        let mut rpc = Rpc::new();
        rpc.initialize("0.3.0");
        assert_eq!(
            text(&rpc.new_session("/home/user/scratch")),
            captured(CLAUDE, r#"{"jsonrpc":"2.0","id":2,"method":"session/new""#)
        );
    }

    #[test]
    fn resume_and_load_name_the_conversation_its_directory_and_no_mcp_servers() {
        let mut rpc = Rpc::from_parts(7, BTreeMap::new(), BTreeMap::new())
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(
            text(&rpc.resume("sess-1", "/home/u/src")),
            "{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"session/resume\",\"params\":\
             {\"sessionId\":\"sess-1\",\"cwd\":\"/home/u/src\",\"mcpServers\":[]}}\n"
        );
        assert_eq!(
            text(&rpc.load("sess-1", "C:\\Users\\u \"x\"")),
            "{\"jsonrpc\":\"2.0\",\"id\":8,\"method\":\"session/load\",\"params\":\
             {\"sessionId\":\"sess-1\",\"cwd\":\"C:\\\\Users\\\\u \\\"x\\\"\",\"mcpServers\":[]}}\n"
        );
        assert_eq!(
            rpc.outgoing()
                .iter()
                .map(|(id, p)| (*id, *p))
                .collect::<Vec<_>>(),
            [(7, Purpose::Resume), (8, Purpose::Load)]
        );
    }

    #[test]
    fn a_prompt_is_one_text_block_in_the_session() {
        let want = captured(
            CLAUDE,
            r#"{"jsonrpc":"2.0","id":4,"method":"session/prompt""#,
        );
        let Ok(Message::Request { params, .. }) = decode(want.trim_end().as_bytes()) else {
            panic!("not a request");
        };
        let session = params["sessionId"].as_str().unwrap_or_default();
        let prompt = params["prompt"][0]["text"].as_str().unwrap_or_default();
        let mut rpc = Rpc::from_parts(4, BTreeMap::new(), BTreeMap::new())
            .unwrap_or_else(|e| panic!("{e:?}"));
        assert_eq!(text(&rpc.prompt(session, prompt)), want);
    }

    #[test]
    fn text_with_newlines_quotes_and_controls_stays_on_one_line() {
        let awkward = "line one\nline \"two\"\t\u{1b}[31m\u{2028}é";
        let bytes = Rpc::new().prompt("s\n", awkward);
        assert_eq!(bytes.iter().filter(|b| **b == b'\n').count(), 1);
        assert_eq!(bytes.last(), Some(&b'\n'));
        let Ok(Message::Request { params, .. }) = decode(&bytes[..bytes.len() - 1]) else {
            panic!("not a request");
        };
        assert_eq!(
            params,
            json!({"sessionId": "s\n", "prompt": [{"type": "text", "text": awkward}]})
        );
    }

    #[test]
    fn cancel_is_a_notification_naming_the_session() {
        assert_eq!(
            text(&cancel("sess-1")),
            "{\"jsonrpc\":\"2.0\",\"method\":\"session/cancel\",\"params\":{\"sessionId\":\"sess-1\"}}\n"
        );
    }

    #[test]
    fn a_permission_answer_echoes_a_number_id_verbatim() {
        assert_eq!(
            text(&permission_answer(&Id::number(0), Some("reject"))),
            captured(CLAUDE, r#"{"jsonrpc":"2.0","id":0,"result""#)
        );
    }

    #[test]
    fn a_permission_answer_echoes_a_string_id_verbatim() {
        let id = Id::parse(&json!("1")).unwrap_or_else(|| panic!("not an id"));
        assert_eq!(
            text(&permission_answer(&id, Some("deny"))),
            captured(ANTIGRAVITY, r#"{"jsonrpc":"2.0","id":"1","result""#)
        );
    }

    #[test]
    fn a_cancelled_permission_answer_names_no_option() {
        assert_eq!(
            text(&permission_answer(&Id::number(7), None)),
            "{\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"outcome\":{\"outcome\":\"cancelled\"}}}\n"
        );
    }

    #[test]
    fn an_unoffered_method_is_answered_32601() {
        assert_eq!(
            text(&error(Some(&Id::number(9)), Refusal::MethodNotFound)),
            "{\"jsonrpc\":\"2.0\",\"id\":9,\"error\":{\"code\":-32601,\"message\":\"Method not found\"}}\n"
        );
    }

    #[test]
    fn an_unreadable_request_is_answered_32600_with_a_null_id() {
        assert_eq!(
            text(&error(None, Refusal::InvalidRequest)),
            "{\"jsonrpc\":\"2.0\",\"id\":null,\"error\":{\"code\":-32600,\"message\":\"Invalid Request\"}}\n"
        );
    }

    #[test]
    fn every_line_the_core_writes_is_json_rpc_it_can_read_back() {
        let id = Id::parse(&json!("x")).unwrap_or_else(|| panic!("not an id"));
        let mut rpc = Rpc::new();
        for bytes in [
            rpc.initialize("0.3.0"),
            rpc.new_session("/"),
            rpc.resume("s\n", "/"),
            rpc.load("s", "/\n"),
            rpc.prompt("s", "hi"),
            cancel("s"),
            permission_answer(&id, Some("a")),
            permission_answer(&id, None),
            error(Some(&id), Refusal::MethodNotFound),
            error(None, Refusal::InvalidRequest),
        ] {
            let body = bytes
                .strip_suffix(b"\n")
                .unwrap_or_else(|| panic!("no newline"));
            assert!(!body.contains(&b'\n'));
            let value: Value = serde_json::from_slice(body).unwrap_or_else(|e| panic!("{e}"));
            assert_eq!(value["jsonrpc"], "2.0");
        }
    }

    #[test]
    fn requests_get_ids_from_1_up_each_with_its_purpose() {
        let mut rpc = Rpc::new();
        rpc.initialize("0.3.0");
        rpc.new_session("/");
        rpc.prompt("s", "hi");
        assert_eq!(rpc.next_id(), 4);
        assert_eq!(
            rpc.outgoing()
                .iter()
                .map(|(id, p)| (*id, *p))
                .collect::<Vec<_>>(),
            [
                (1, Purpose::Initialize),
                (2, Purpose::NewSession),
                (3, Purpose::Prompt)
            ]
        );
    }

    #[test]
    fn a_response_takes_its_request_once() {
        let mut rpc = Rpc::new();
        rpc.initialize("0.3.0");
        assert_eq!(rpc.answered(&Id::number(1)), Some(Purpose::Initialize));
        assert_eq!(rpc.answered(&Id::number(1)), None);
        assert!(rpc.outgoing().is_empty());
    }

    #[test]
    fn a_response_to_an_id_never_sent_or_a_string_id_matches_nothing() {
        let mut rpc = Rpc::new();
        rpc.initialize("0.3.0");
        let string = Id::parse(&json!("1")).unwrap_or_else(|| panic!("not an id"));
        assert_eq!(rpc.answered(&string), None);
        assert_eq!(rpc.answered(&Id::number(2)), None);
        assert_eq!(rpc.outgoing().len(), 1);
    }

    #[test]
    fn forgetting_a_purpose_drops_only_its_requests() {
        let mut rpc = Rpc::new();
        rpc.new_session("/");
        rpc.prompt("s", "hi");
        assert!(rpc.forget(Purpose::Prompt));
        assert!(!rpc.forget(Purpose::Prompt));
        assert_eq!(
            rpc.outgoing().values().copied().collect::<Vec<_>>(),
            [Purpose::NewSession]
        );
    }

    #[test]
    fn a_held_permission_is_answered_once_by_its_entry() {
        let mut rpc = Rpc::new();
        assert_eq!(rpc.hold_permission(EntryId(4), Id::number(0)), Ok(()));
        assert_eq!(
            rpc.answer_permission(EntryId(4), Some("allow")).as_deref(),
            Some(&b"{\"jsonrpc\":\"2.0\",\"id\":0,\"result\":{\"outcome\":{\"outcome\":\"selected\",\"optionId\":\"allow\"}}}\n"[..])
        );
        assert_eq!(rpc.answer_permission(EntryId(4), Some("allow")), None);
        assert_eq!(rpc.answer_permission(EntryId(5), None), None);
    }

    #[test]
    fn permissions_past_16_are_refused_and_answering_one_makes_room() {
        let mut rpc = Rpc::new();
        for n in 0..16 {
            assert_eq!(rpc.hold_permission(EntryId(n + 1), Id::number(n)), Ok(()));
        }
        assert_eq!(rpc.hold_permission(EntryId(17), Id::number(16)), Err(Full));
        assert_eq!(rpc.permissions().len(), MAX_PENDING_PERMISSIONS);
        assert!(rpc.answer_permission(EntryId(1), None).is_some());
        assert_eq!(rpc.hold_permission(EntryId(17), Id::number(16)), Ok(()));
    }

    #[test]
    fn a_cancel_answers_every_waiting_permission_cancelled_in_entry_order() {
        let mut rpc = Rpc::new();
        let string = Id::parse(&json!("b")).unwrap_or_else(|| panic!("not an id"));
        assert_eq!(rpc.hold_permission(EntryId(9), string.clone()), Ok(()));
        assert_eq!(rpc.hold_permission(EntryId(3), Id::number(1)), Ok(()));
        assert_eq!(
            rpc.cancel_permissions(),
            [
                (EntryId(3), permission_answer(&Id::number(1), None)),
                (EntryId(9), permission_answer(&string, None)),
            ]
        );
        assert!(rpc.permissions().is_empty());
    }

    #[test]
    fn a_withdrawn_permission_is_answered_cancelled_and_names_its_entry() {
        let mut rpc = Rpc::new();
        assert_eq!(rpc.hold_permission(EntryId(2), Id::number(0)), Ok(()));
        assert_eq!(
            rpc.withdraw(&Id::number(0)),
            Some((EntryId(2), permission_answer(&Id::number(0), None)))
        );
        assert_eq!(rpc.withdraw(&Id::number(0)), None);
    }

    #[test]
    fn a_restart_forgets_every_request_and_keeps_counting() {
        let mut rpc = Rpc::new();
        rpc.initialize("0.3.0");
        rpc.prompt("s", "hi");
        assert_eq!(rpc.hold_permission(EntryId(1), Id::number(0)), Ok(()));
        rpc.restart();
        assert!(rpc.outgoing().is_empty() && rpc.permissions().is_empty());
        assert_eq!(rpc.answered(&Id::number(2)), None);
        assert_eq!(rpc.next_id(), 3);
        rpc.initialize("0.3.0");
        assert_eq!(rpc.outgoing().keys().copied().collect::<Vec<_>>(), [3]);
    }

    #[test]
    fn parts_round_trip_and_bad_parts_are_refused() {
        let mut rpc = Rpc::new();
        rpc.initialize("0.3.0");
        assert_eq!(rpc.hold_permission(EntryId(1), Id::number(0)), Ok(()));
        let rebuilt = Rpc::from_parts(
            rpc.next_id(),
            rpc.outgoing().clone(),
            rpc.permissions().clone(),
        );
        assert_eq!(rebuilt, Ok(rpc.clone()));
        assert_eq!(
            Rpc::from_parts(1, rpc.outgoing().clone(), BTreeMap::new()),
            Err(BadParts::IdNotBelowNext)
        );
        let many = (0..17).map(|n| (EntryId(n + 1), Id::number(n))).collect();
        assert_eq!(
            Rpc::from_parts(2, BTreeMap::new(), many),
            Err(BadParts::TooManyPending)
        );
    }
}
