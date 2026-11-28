//! Each line the core writes to an ACP v1 agent ([`encode`] of a
//! [`Request`]), one line ending in `\n` (design decisions 4, 8 and 11 of
//! `agents-sessions`).
//!
//! The lines are written here, not through the schema crate's types: the
//! crate encodes `clientCapabilities` with explicit `false` fields, and
//! decision 4 offers nothing but what Brindle does, which since the
//! OpenSpec change `agents-login` is `{"auth":{"terminal":true}}` (a client
//! runs an agent's terminal login methods), and since `agents-commands`
//! also `{"session":{"configOptions":{"boolean":{}}}}` (the zone shows a
//! toggle). Every string goes through
//! JSON's own escaping, so no line holds a raw newline, and a request id
//! from the agent is echoed as it was sent.

use brindle_remote_protocol::client::OptionValue;
use brindle_remote_protocol::serde_json::Value;

use crate::acp::envelope::Id;
use crate::acp::forms::{Continued, OptionKey, PromptPart, Request};

/// The ACP version this module speaks.
pub const VERSION: u16 = 1;

/// The line that writes `request`.
pub fn encode(request: Request<'_>) -> Vec<u8> {
    match request {
        Request::Initialize { id, version } => call(
            id,
            "initialize",
            &format!(
                r#"{{"protocolVersion":{VERSION},"clientCapabilities":{{"auth":{{"terminal":true}},"session":{{"configOptions":{{"boolean":{{}}}}}}}},"clientInfo":{{"name":"brindle","version":{}}}}}"#,
                quote(version)
            ),
        ),
        Request::Authenticate { id, method } => call(
            id,
            "authenticate",
            &format!(r#"{{"methodId":{}}}"#, quote(method)),
        ),
        Request::NewSession {
            id,
            cwd,
            directories,
        } => call(
            id,
            "session/new",
            &format!(
                r#"{{"cwd":{}{},"mcpServers":[]}}"#,
                quote(cwd),
                additional(directories)
            ),
        ),
        Request::Continue {
            id,
            how,
            session,
            cwd,
            directories,
        } => {
            let method = continue_method(how);
            let params = format!(
                r#"{{"sessionId":{},"cwd":{}{},"mcpServers":[]}}"#,
                quote(session),
                quote(cwd),
                additional(directories)
            );
            call(id, method, &params)
        }
        Request::Prompt { id, session, parts } => {
            let blocks: Vec<String> = parts.iter().map(|part| block(*part)).collect();
            call(
                id,
                "session/prompt",
                &format!(
                    r#"{{"sessionId":{},"prompt":[{}]}}"#,
                    quote(session),
                    blocks.join(",")
                ),
            )
        }
        Request::List { id, cwd, cursor } => {
            let cursor = cursor.map_or_else(String::new, |cursor| format!(r#","cursor":{}"#, quote(cursor)));
            call(id, "session/list", &format!(r#"{{"cwd":{}{cursor}}}"#, quote(cwd)))
        }
        Request::SetOption {
            id,
            session,
            key,
            value,
        } => {
            let text = match value {
                OptionValue::Choice { value } => quote(value),
                OptionValue::Toggle { on } => quote(if *on { "true" } else { "false" }),
            };
            match (key, value) {
                (OptionKey::Config { id: config }, OptionValue::Toggle { on }) => call(
                    id,
                    "session/set_config_option",
                    &format!(
                        r#"{{"sessionId":{},"configId":{},"type":"boolean","value":{on}}}"#,
                        quote(session),
                        quote(config)
                    ),
                ),
                (OptionKey::Config { id: config }, OptionValue::Choice { .. }) => call(
                    id,
                    "session/set_config_option",
                    &format!(
                        r#"{{"sessionId":{},"configId":{},"value":{text}}}"#,
                        quote(session),
                        quote(config)
                    ),
                ),
                (OptionKey::Modes, _) => call(
                    id,
                    "session/set_mode",
                    &format!(r#"{{"sessionId":{},"modeId":{text}}}"#, quote(session)),
                ),
                (OptionKey::Models, _) => call(
                    id,
                    "session/set_model",
                    &format!(r#"{{"sessionId":{},"modelId":{text}}}"#, quote(session)),
                ),
            }
        }
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

/// A prompt part as its v1 content block (`agents-prompt-context`,
/// decision 6): `text`, a `resource_link`, or a `resource` holding the text
/// as `text/plain`.
fn block(part: PromptPart<'_>) -> String {
    match part {
        PromptPart::Text(text) => format!(r#"{{"type":"text","text":{}}}"#, quote(text)),
        PromptPart::Link { uri, name } => format!(
            r#"{{"type":"resource_link","uri":{},"name":{}}}"#,
            quote(uri),
            quote(name)
        ),
        PromptPart::Embedded { uri, name: _, text } => format!(
            r#"{{"type":"resource","resource":{{"uri":{},"mimeType":"text/plain","text":{}}}}}"#,
            quote(uri),
            quote(text)
        ),
    }
}

/// A session request's `additionalDirectories` field, with its comma, or
/// nothing when there are none, so the line is as before.
fn additional(directories: &[String]) -> String {
    if directories.is_empty() {
        return String::new();
    }
    let quoted: Vec<String> = directories.iter().map(|dir| quote(dir)).collect();
    format!(r#","additionalDirectories":[{}]"#, quoted.join(","))
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

#[cfg(test)]
mod tests {
    use super::*;
    use brindle_remote_protocol::serde_json;

    fn set(id: u64, session: &str, key: &OptionKey, value: &OptionValue) -> String {
        String::from_utf8(encode(Request::SetOption {
            id,
            session,
            key,
            value,
        }))
        .unwrap_or_default()
    }

    fn text(bytes: Vec<u8>) -> String {
        String::from_utf8(bytes).unwrap_or_default()
    }

    /// A prompt's parts in order (OpenSpec change `agents-prompt-context`,
    /// decision 6): text alone is the line from before, a link a
    /// `resource_link`, a kept text a `resource` of `text/plain`.
    #[test]
    fn a_prompts_parts_are_text_resource_links_and_resources_in_order() {
        let prompt = |parts: &[PromptPart<'_>]| {
            text(encode(Request::Prompt {
                id: 4,
                session: "s1",
                parts,
            }))
        };
        assert_eq!(
            prompt(&[PromptPart::Text("hi")]),
            "{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"session/prompt\",\"params\":\
             {\"sessionId\":\"s1\",\"prompt\":[{\"type\":\"text\",\"text\":\"hi\"}]}}\n"
        );
        assert_eq!(
            prompt(&[
                PromptPart::Text("why?"),
                PromptPart::Embedded {
                    uri: "file:///c/1",
                    name: "cargo test",
                    text: "a\n\"b\"",
                },
                PromptPart::Link {
                    uri: "file:///home/u/a%20b.rs",
                    name: "a b.rs",
                },
            ]),
            "{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"session/prompt\",\"params\":\
             {\"sessionId\":\"s1\",\"prompt\":[{\"type\":\"text\",\"text\":\"why?\"},\
             {\"type\":\"resource\",\"resource\":{\"uri\":\"file:///c/1\",\"mimeType\":\"text/plain\",\"text\":\"a\\n\\\"b\\\"\"}},\
             {\"type\":\"resource_link\",\"uri\":\"file:///home/u/a%20b.rs\",\"name\":\"a b.rs\"}]}}\n"
        );
        // Each part reads back as the schema's content block.
        let line = prompt(&[
            PromptPart::Text("t"),
            PromptPart::Embedded {
                uri: "file:///c/1",
                name: "n",
                text: "x",
            },
            PromptPart::Link {
                uri: "file:///a",
                name: "a",
            },
        ]);
        let value: serde_json::Value = serde_json::from_str(&line).unwrap();
        let request: agent_client_protocol_schema::v1::PromptRequest =
            serde_json::from_value(value["params"].clone()).unwrap();
        assert_eq!(request.prompt.len(), 3);
    }

    /// `additionalDirectories` is written only when there are some, so a
    /// session's start is the line from before for every other agent.
    #[test]
    fn session_requests_name_more_directories_only_when_given() {
        let dirs = ["/home/u/.cache/brindle/attachments/s1".to_owned()];
        assert_eq!(
            text(encode(Request::NewSession {
                id: 2,
                cwd: "/w",
                directories: &dirs,
            })),
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"session/new\",\"params\":\
             {\"cwd\":\"/w\",\"additionalDirectories\":[\"/home/u/.cache/brindle/attachments/s1\"],\"mcpServers\":[]}}\n"
        );
        assert_eq!(
            text(encode(Request::NewSession {
                id: 2,
                cwd: "/w",
                directories: &[],
            })),
            "{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"session/new\",\"params\":\
             {\"cwd\":\"/w\",\"mcpServers\":[]}}\n"
        );
        assert_eq!(
            text(encode(Request::Continue {
                id: 3,
                how: Continued::Resume,
                session: "s1",
                cwd: "/w",
                directories: &dirs,
            })),
            "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"session/resume\",\"params\":\
             {\"sessionId\":\"s1\",\"cwd\":\"/w\",\"additionalDirectories\":[\"/home/u/.cache/brindle/attachments/s1\"],\"mcpServers\":[]}}\n"
        );
        for line in [
            encode(Request::NewSession { id: 2, cwd: "/w", directories: &dirs }),
            encode(Request::Continue { id: 3, how: Continued::Load, session: "s1", cwd: "/w", directories: &dirs }),
        ] {
            let value: serde_json::Value = serde_json::from_slice(&line).unwrap();
            assert_eq!(value["params"]["additionalDirectories"][0], dirs[0].as_str());
        }
    }

    /// The modes' and the model list's sets (OpenSpec change
    /// `agents-commands`); the config options' are pinned in `rpc.rs`.
    #[test]
    fn a_modes_set_is_set_mode_and_a_models_set_model() {
        let plan = OptionValue::Choice {
            value: "plan".into(),
        };
        assert_eq!(
            set(11, "s1", &OptionKey::Modes, &plan),
            "{\"jsonrpc\":\"2.0\",\"id\":11,\"method\":\"session/set_mode\",\"params\":\
             {\"sessionId\":\"s1\",\"modeId\":\"plan\"}}\n"
        );
        let model = OptionValue::Choice {
            value: "sonnet \"x\"".into(),
        };
        assert_eq!(
            set(12, "s1", &OptionKey::Models, &model),
            "{\"jsonrpc\":\"2.0\",\"id\":12,\"method\":\"session/set_model\",\"params\":\
             {\"sessionId\":\"s1\",\"modelId\":\"sonnet \\\"x\\\"\"}}\n"
        );
    }

    /// The captured `session/set_mode` and `session/set_model` lines (Codex
    /// 2.12, Antigravity 1.5.16, claude-code-acp 0.16.2's `set_mode`) are
    /// the ones Brindle writes.
    #[test]
    fn the_captured_sets_are_written_as_brindle_writes_them() {
        let captures = [
            include_str!("../../../tests/fixtures/acp/codex-acp-2.12.0-list.log"),
            include_str!("../../../tests/fixtures/acp/antigravity-1.5.16-reject.log"),
            include_str!("../../../tests/fixtures/acp/claude-code-acp-0.16.2-reject.log"),
        ];
        let mut seen = Vec::new();
        for capture in captures {
            let sets = capture
                .lines()
                .filter_map(|line| line.strip_prefix("c>a\t"))
                .map(|line| serde_json::from_str::<Value>(line).unwrap_or_else(|e| panic!("{e}")))
                .filter(|line| {
                    line["method"] == "session/set_mode" || line["method"] == "session/set_model"
                });
            for sent in sets {
                let method = sent["method"].as_str().unwrap_or_default().to_owned();
                let params = &sent["params"];
                let (key, field) = if method == "session/set_mode" {
                    (OptionKey::Modes, "modeId")
                } else {
                    (OptionKey::Models, "modelId")
                };
                let value = OptionValue::Choice {
                    value: params[field].as_str().unwrap_or_else(|| panic!("{method}")).into(),
                };
                let id = sent["id"].as_u64().unwrap_or_else(|| panic!("{method}"));
                let session = params["sessionId"].as_str().unwrap_or_else(|| panic!("{method}"));
                let ours: Value = serde_json::from_str(&set(id, session, &key, &value))
                    .unwrap_or_else(|e| panic!("{e}"));
                assert_eq!(ours, sent, "{method}");
                seen.push(method);
            }
        }
        assert!(seen.iter().any(|method| method == "session/set_mode"), "{seen:?}");
        assert!(seen.iter().any(|method| method == "session/set_model"), "{seen:?}");
    }
}
