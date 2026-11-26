//! The JSON-RPC envelope of every line an agent writes (design decision 5
//! of `agents-sessions`), decoded by hand: [`decode`] reads a line into a
//! [`Message`], with `params` and `result` kept as [`Value`] for the
//! session's dialect (`acp::dialect`) to read. No ACP version, method or
//! field is named here (`ports R14`).
//!
//! - **A malformed line** is one skipped item ([`Malformed`]), never a lost
//!   connection; a known method whose params don't decode is [`BadParams`].
//! - **Request ids** ([`Id`]) are kept as the JSON number or string the agent
//!   sent (Antigravity sends `"1"`; others start at `0`), and echoed back
//!   verbatim.

use brindle_remote_protocol::serde_json::{self, Map, Value};

/// The longest request id kept, as encoded JSON: a request with a longer id
/// is answered `-32600` with a null id (decision 13).
pub const MAX_ID_LEN: usize = 1024;

/// A JSON-RPC request id: a number or a string, at most [`MAX_ID_LEN`]
/// bytes encoded, kept as sent so an answer echoes it verbatim.
#[derive(Debug, Clone, PartialEq)]
pub struct Id(Value);

impl Id {
    /// The id of a request the core sends (the core's own come from
    /// `Rpc`; the tests build them).
    #[cfg(test)]
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
    pub fn missing(what: &str) -> Self {
        Self(format!("no {what}"))
    }

    pub fn from_serde(error: &serde_json::Error) -> Self {
        Self(error.to_string())
    }
}

#[cfg(test)]
mod tests {
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
}
