//! Each line the core writes to an ACP v1 agent ([`encode`] of a
//! [`Request`]), one line ending in `\n` (design decisions 4, 8 and 11 of
//! `agents-sessions`).
//!
//! The lines are written here, not through the schema crate's types: the
//! crate encodes `clientCapabilities` with explicit `false` fields, and
//! decision 4 offers nothing but what Brindle does, which since the
//! OpenSpec change `agents-login` is `{"auth":{"terminal":true}}` (a client
//! runs an agent's terminal login methods). Every string goes through
//! JSON's own escaping, so no line holds a raw newline, and a request id
//! from the agent is echoed as it was sent.

use brindle_remote_protocol::serde_json::Value;

use crate::acp::envelope::Id;
use crate::acp::forms::{Continued, Request};

/// The ACP version this module speaks.
pub const VERSION: u16 = 1;

/// The line that writes `request`.
pub fn encode(request: Request<'_>) -> Vec<u8> {
    match request {
        Request::Initialize { id, version } => call(
            id,
            "initialize",
            &format!(
                r#"{{"protocolVersion":{VERSION},"clientCapabilities":{{"auth":{{"terminal":true}}}},"clientInfo":{{"name":"brindle","version":{}}}}}"#,
                quote(version)
            ),
        ),
        Request::Authenticate { id, method } => call(
            id,
            "authenticate",
            &format!(r#"{{"methodId":{}}}"#, quote(method)),
        ),
        Request::NewSession { id, cwd } => call(
            id,
            "session/new",
            &format!(r#"{{"cwd":{},"mcpServers":[]}}"#, quote(cwd)),
        ),
        Request::Continue {
            id,
            how,
            session,
            cwd,
        } => {
            let method = continue_method(how);
            let params = format!(
                r#"{{"sessionId":{},"cwd":{},"mcpServers":[]}}"#,
                quote(session),
                quote(cwd)
            );
            call(id, method, &params)
        }
        Request::Prompt { id, session, text } => call(
            id,
            "session/prompt",
            &format!(
                r#"{{"sessionId":{},"prompt":[{{"type":"text","text":{}}}]}}"#,
                quote(session),
                quote(text)
            ),
        ),
        Request::Cancel { session } => line(format!(
            r#"{{"jsonrpc":"2.0","method":"session/cancel","params":{{"sessionId":{}}}}}"#,
            quote(session)
        )),
        Request::Withdraw { id } => line(format!(
            r#"{{"jsonrpc":"2.0","method":"$/cancel_request","params":{{"requestId":{id}}}}}"#
        )),
        Request::PermissionAnswer { id, option } => {
            let outcome = match option {
                Some(option) => {
                    format!(r#"{{"outcome":"selected","optionId":{}}}"#, quote(option))
                }
                None => r#"{"outcome":"cancelled"}"#.to_owned(),
            };
            line(format!(
                r#"{{"jsonrpc":"2.0","id":{},"result":{{"outcome":{outcome}}}}}"#,
                id.encode()
            ))
        }
        Request::Error { id, refusal } => {
            let id = id.map_or_else(|| "null".to_owned(), Id::encode);
            line(format!(
                r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":{},"message":{}}}}}"#,
                refusal.code(),
                quote(refusal.message())
            ))
        }
    }
}

/// The method that continues a conversation as `how` says.
pub fn continue_method(how: Continued) -> &'static str {
    match how {
        Continued::Resume => "session/resume",
        Continued::Load => "session/load",
    }
}

/// A request of the core's, by its id.
fn call(id: u64, method: &str, params: &str) -> Vec<u8> {
    line(format!(
        r#"{{"jsonrpc":"2.0","id":{id},"method":{},"params":{params}}}"#,
        quote(method)
    ))
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
