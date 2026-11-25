//! The fake ACP agent's script (design decision 16 of the OpenSpec change
//! `agents-sessions`): what `brindle-fake-acp-agent` reads, writes and does,
//! one step per line, so a test plays an agent without a real one.
//!
//! ```text
//! # A comment; blank lines are skipped too.
//! record /tmp/case/received.jsonl
//! expect initialize
//! respond {"protocolVersion":1,"agentInfo":{"name":"fake","version":"1.0"}}
//! expect session/new
//! respond-error -32000 "run fake login"
//! stderr fake: no login
//! wait-file /tmp/case/go
//! send {"jsonrpc":"2.0","method":"session/update","params":{}}
//! send-raw hello world
//! exit 1
//! ```
//!
//! - `record <path>`: each message a later `expect` matches is appended to
//!   this file, as the line it came in, so the test reads what the agent
//!   was sent.
//! - `expect <method>`: read lines from stdin until a message with that
//!   method comes (lines before it are skipped).
//! - `expect-response <id>`: read lines from stdin until the response to
//!   the agent's own request `id` (a JSON value: `"p1"`, `7`) comes, and
//!   record it as `expect` does, so a test reads what the agent was
//!   answered (lines before it are skipped).
//! - `send <json>`: write one JSON value as a line to stdout, exactly as
//!   written.
//! - `send-raw <text>`: write the text as a line, whatever it is (lines
//!   that aren't JSON, unknown variants).
//! - `respond <json>`: answer the last request an `expect` matched with
//!   that `result`; a notification matched since (`session/cancel`)
//!   doesn't change which.
//! - `respond-error <code> <message> [<data>]`: answer it with an error;
//!   the message is one word, or a JSON string for one with spaces, and the
//!   optional JSON after it is the error's `data` (a `-32022`'s
//!   `{ requested, supported }`).
//! - `stderr <text>`: write the text as a line to stderr.
//! - `wait-file <path>`: wait until the file exists, a gate the test
//!   creates (never a sleep).
//! - `exit <code>`: exit with that status.
//!
//! [`parse`] reads a script whole, or says which line is wrong and why,
//! and never panics; [`Received::parse`], [`result_line`] and
//! [`error_line`] are what the player reads and writes. A whole pure module
//! (`fcis` R2, R3; `PURE_MODULES`): the binary's `main` does the IO.

use std::fmt;

use brindle_remote_protocol::serde_json::{self, Map, Value, json};

/// One step of a script.
#[derive(Debug, Clone, PartialEq)]
pub enum Step {
    Record {
        path: String,
    },
    Expect {
        method: String,
    },
    /// The response to the agent's request `id`.
    ExpectResponse {
        id: Value,
    },
    /// `line` is the JSON as written, checked to be one value.
    Send {
        line: String,
    },
    SendRaw {
        text: String,
    },
    Respond {
        result: Value,
    },
    RespondError {
        code: i64,
        message: String,
        data: Option<Value>,
    },
    Stderr {
        text: String,
    },
    WaitFile {
        path: String,
    },
    Exit {
        code: i32,
    },
}

/// What is wrong with a script's line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Problem {
    /// A word that isn't a step.
    UnknownStep { word: String },
    /// A step without the argument it needs.
    Missing {
        step: &'static str,
        what: &'static str,
    },
    /// An argument that isn't the JSON it must be.
    BadJson { step: &'static str, error: String },
    /// A code that isn't an integer of its range.
    BadCode { step: &'static str, text: String },
    /// A `respond` or `respond-error` before any `expect`: there is no
    /// request to answer.
    NoRequest { step: &'static str },
}

/// A script that doesn't parse: the line (from 1) and its problem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BadScript {
    pub line: usize,
    pub problem: Problem,
}

impl fmt::Display for BadScript {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}: ", self.line)?;
        match &self.problem {
            Problem::UnknownStep { word } => write!(f, "{word:?} isn't a step"),
            Problem::Missing { step, what } => write!(f, "{step} needs {what}"),
            Problem::BadJson { step, error } => write!(f, "{step}'s JSON doesn't parse: {error}"),
            Problem::BadCode { step, text } => write!(f, "{step}'s code {text:?} isn't a number"),
            Problem::NoRequest { step } => {
                write!(
                    f,
                    "{step} comes before any expect, so there is no request to answer"
                )
            }
        }
    }
}

/// The steps of `text`, in order.
pub fn parse(text: &str) -> Result<Vec<Step>, BadScript> {
    let mut steps = Vec::new();
    let mut expected = false;
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim_start();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let bad = |problem| BadScript {
            line: index + 1,
            problem,
        };
        let (word, rest) = match line.split_once(char::is_whitespace) {
            Some((word, rest)) => (word, rest.trim()),
            None => (line.trim_end(), ""),
        };
        let step = step(word, rest).map_err(bad)?;
        match &step {
            Step::Expect { .. } => expected = true,
            Step::Respond { .. } if !expected => {
                return Err(bad(Problem::NoRequest { step: "respond" }));
            }
            Step::RespondError { .. } if !expected => {
                return Err(bad(Problem::NoRequest {
                    step: "respond-error",
                }));
            }
            Step::Record { .. }
            | Step::ExpectResponse { .. }
            | Step::Send { .. }
            | Step::SendRaw { .. }
            | Step::Respond { .. }
            | Step::RespondError { .. }
            | Step::Stderr { .. }
            | Step::WaitFile { .. }
            | Step::Exit { .. } => {}
        }
        steps.push(step);
    }
    Ok(steps)
}

/// The step `word` with its argument `rest` (trimmed).
fn step(word: &str, rest: &str) -> Result<Step, Problem> {
    let needed = |step: &'static str, what: &'static str| {
        if rest.is_empty() {
            Err(Problem::Missing { step, what })
        } else {
            Ok(rest.to_owned())
        }
    };
    Ok(match word {
        "record" => Step::Record {
            path: needed("record", "a file")?,
        },
        "expect" => Step::Expect {
            method: needed("expect", "a method")?,
        },
        "expect-response" => Step::ExpectResponse {
            id: json_value(
                "expect-response",
                &needed("expect-response", "a request id")?,
            )?,
        },
        "send" => {
            let line = needed("send", "a JSON value")?;
            json_value("send", &line)?;
            Step::Send { line }
        }
        "send-raw" => Step::SendRaw {
            text: rest.to_owned(),
        },
        "respond" => Step::Respond {
            result: json_value("respond", &needed("respond", "a JSON result")?)?,
        },
        "respond-error" => respond_error(rest)?,
        "stderr" => Step::Stderr {
            text: rest.to_owned(),
        },
        "wait-file" => Step::WaitFile {
            path: needed("wait-file", "a file")?,
        },
        "exit" => {
            let text = needed("exit", "a status")?;
            let code = text
                .parse()
                .map_err(|_| Problem::BadCode { step: "exit", text })?;
            Step::Exit { code }
        }
        _ => {
            return Err(Problem::UnknownStep {
                word: word.to_owned(),
            });
        }
    })
}

/// `text` as exactly one JSON value.
fn json_value(step: &'static str, text: &str) -> Result<Value, Problem> {
    serde_json::from_str(text).map_err(|error| Problem::BadJson {
        step,
        error: error.to_string(),
    })
}

/// `respond-error <code> <message> [<data>]`.
fn respond_error(rest: &str) -> Result<Step, Problem> {
    let step = "respond-error";
    let (code, rest) = rest
        .split_once(char::is_whitespace)
        .map_or((rest, ""), |(code, rest)| (code, rest.trim_start()));
    if code.is_empty() {
        return Err(Problem::Missing {
            step,
            what: "a code",
        });
    }
    let code = code.parse().map_err(|_| Problem::BadCode {
        step,
        text: code.to_owned(),
    })?;
    if rest.is_empty() {
        return Err(Problem::Missing {
            step,
            what: "a message",
        });
    }
    let (message, rest) = if rest.starts_with('"') {
        // A JSON string, and what follows it.
        let mut values = serde_json::Deserializer::from_str(rest).into_iter::<String>();
        let message = match values.next() {
            Some(Ok(message)) => message,
            Some(Err(error)) => {
                return Err(Problem::BadJson {
                    step,
                    error: error.to_string(),
                });
            }
            None => {
                return Err(Problem::Missing {
                    step,
                    what: "a message",
                });
            }
        };
        let after = rest.get(values.byte_offset()..).unwrap_or_default();
        (message, after.trim())
    } else {
        rest.split_once(char::is_whitespace)
            .map_or((rest.to_owned(), ""), |(word, rest)| {
                (word.to_owned(), rest.trim())
            })
    };
    let data = if rest.is_empty() {
        None
    } else {
        Some(json_value(step, rest)?)
    };
    Ok(Step::RespondError {
        code,
        message,
        data,
    })
}

/// What the player reads of a line from stdin.
#[derive(Debug, Clone, PartialEq)]
pub struct Received {
    /// Its `method`, for a request or a notification.
    pub method: Option<String>,
    /// Its `id`, for a request (or a response).
    pub id: Option<Value>,
}

impl Received {
    /// `line` as a JSON-RPC message, or `None` for a line that isn't a JSON
    /// object.
    pub fn parse(line: &[u8]) -> Option<Self> {
        let value: Map<String, Value> = serde_json::from_slice(line).ok()?;
        Some(Self {
            method: value
                .get("method")
                .and_then(Value::as_str)
                .map(str::to_owned),
            id: value.get("id").filter(|id| !id.is_null()).cloned(),
        })
    }

    /// Whether it is the response to request `id`: no method, that id.
    pub fn answers(&self, id: &Value) -> bool {
        self.method.is_none() && self.id.as_ref() == Some(id)
    }
}

/// The line answering request `id` with `result`, ending in `\n`.
pub fn result_line(id: &Value, result: &Value) -> String {
    let mut line = json!({ "jsonrpc": "2.0", "id": id, "result": result }).to_string();
    line.push('\n');
    line
}

/// The line answering request `id` with an error, ending in `\n`; `data`
/// is left out when there is none.
pub fn error_line(id: &Value, code: i64, message: &str, data: Option<&Value>) -> String {
    let mut error = json!({ "code": code, "message": message });
    if let (Some(data), Some(object)) = (data, error.as_object_mut()) {
        object.insert("data".to_owned(), data.clone());
    }
    let mut line = json!({ "jsonrpc": "2.0", "id": id, "error": error }).to_string();
    line.push('\n');
    line
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one(text: &str) -> Step {
        let steps = parse(text).unwrap_or_else(|bad| panic!("{text:?}: {bad}"));
        assert_eq!(steps.len(), 1, "{steps:?}");
        steps.into_iter().next().expect("one step")
    }

    /// `text`'s last step, after an `expect` so a response may follow.
    fn answer(text: &str) -> Step {
        let steps = parse(&format!("expect session/new\n{text}"))
            .unwrap_or_else(|bad| panic!("{text:?}: {bad}"));
        steps.last().cloned().expect("a step")
    }

    fn problem(text: &str) -> BadScript {
        parse(text).expect_err("a bad script")
    }

    // --- each step --------------------------------------------------------

    #[test]
    fn record_names_the_file_expected_messages_go_to() {
        assert_eq!(
            one("record /tmp/case dir/got.jsonl"),
            Step::Record {
                path: "/tmp/case dir/got.jsonl".into()
            }
        );
    }

    #[test]
    fn expect_names_a_method() {
        assert_eq!(
            one("expect session/prompt"),
            Step::Expect {
                method: "session/prompt".into()
            }
        );
    }

    #[test]
    fn expect_response_names_the_agents_request_id_as_json() {
        assert_eq!(
            one(r#"expect-response "perm-1""#),
            Step::ExpectResponse { id: json!("perm-1") }
        );
        assert_eq!(one("expect-response 7"), Step::ExpectResponse { id: json!(7) });
        assert_eq!(
            problem("expect-response").problem,
            Problem::Missing {
                step: "expect-response",
                what: "a request id"
            }
        );
        assert!(matches!(
            problem("expect-response perm-1").problem,
            Problem::BadJson {
                step: "expect-response",
                ..
            }
        ));
    }

    #[test]
    fn send_keeps_its_json_exactly_as_written() {
        let line = r#"{"jsonrpc":"2.0", "method":"session/update","params":{"b":1,"a":2}}"#;
        assert_eq!(
            one(&format!("send {line}")),
            Step::Send { line: line.into() }
        );
    }

    #[test]
    fn send_raw_keeps_any_text_even_none() {
        assert_eq!(
            one("send-raw hello {world"),
            Step::SendRaw {
                text: "hello {world".into()
            }
        );
        assert_eq!(
            one("send-raw"),
            Step::SendRaw {
                text: String::new()
            }
        );
    }

    #[test]
    fn respond_takes_a_json_result() {
        assert_eq!(
            answer(r#"respond {"sessionId":"s1"}"#),
            Step::Respond {
                result: json!({"sessionId": "s1"})
            }
        );
        assert_eq!(
            answer("respond null"),
            Step::Respond {
                result: Value::Null
            }
        );
    }

    #[test]
    fn respond_error_takes_a_code_and_a_one_word_message() {
        assert_eq!(
            answer("respond-error -32000 auth_required"),
            Step::RespondError {
                code: -32000,
                message: "auth_required".into(),
                data: None
            }
        );
    }

    #[test]
    fn respond_error_takes_a_quoted_message_with_spaces() {
        assert_eq!(
            answer(r#"respond-error -32000 "run agent login""#),
            Step::RespondError {
                code: -32000,
                message: "run agent login".into(),
                data: None
            }
        );
    }

    #[test]
    fn respond_error_takes_its_data_after_the_message() {
        let expected = Step::RespondError {
            code: -32022,
            message: "Unsupported protocol version".into(),
            data: Some(json!({"requested": 1, "supported": [2]})),
        };
        assert_eq!(
            answer(
                r#"respond-error -32022 "Unsupported protocol version" {"requested":1,"supported":[2]}"#
            ),
            expected
        );
        assert_eq!(
            answer(r#"respond-error -32022 unsupported {"requested":1}"#),
            Step::RespondError {
                code: -32022,
                message: "unsupported".into(),
                data: Some(json!({"requested": 1})),
            }
        );
    }

    #[test]
    fn stderr_keeps_its_text() {
        assert_eq!(
            one("stderr fatal: no model"),
            Step::Stderr {
                text: "fatal: no model".into()
            }
        );
    }

    #[test]
    fn wait_file_names_the_gate() {
        assert_eq!(
            one("wait-file /tmp/case/go"),
            Step::WaitFile {
                path: "/tmp/case/go".into()
            }
        );
    }

    #[test]
    fn exit_takes_a_status_negative_ones_included() {
        assert_eq!(one("exit 0"), Step::Exit { code: 0 });
        assert_eq!(one("exit -1"), Step::Exit { code: -1 });
    }

    #[test]
    fn comments_and_blank_lines_are_skipped_and_steps_keep_their_order() {
        let script = "# a fake agent\n\nrecord got.jsonl\n  expect initialize\n\
                      respond {\"protocolVersion\":1}\r\nexit 0\n";
        assert_eq!(
            parse(script),
            Ok(vec![
                Step::Record {
                    path: "got.jsonl".into()
                },
                Step::Expect {
                    method: "initialize".into()
                },
                Step::Respond {
                    result: json!({"protocolVersion": 1})
                },
                Step::Exit { code: 0 },
            ])
        );
    }

    // --- bad scripts ------------------------------------------------------

    #[test]
    fn an_unknown_step_is_refused_with_its_line() {
        assert_eq!(
            problem("expect initialize\n\nsleep 5"),
            BadScript {
                line: 3,
                problem: Problem::UnknownStep {
                    word: "sleep".into()
                }
            }
        );
        assert_eq!(
            problem("sleep 5").to_string(),
            "line 1: \"sleep\" isn't a step"
        );
    }

    #[test]
    fn a_step_without_its_argument_is_refused() {
        for (text, step, what) in [
            ("record", "record", "a file"),
            ("expect", "expect", "a method"),
            ("send", "send", "a JSON value"),
            ("wait-file  ", "wait-file", "a file"),
            ("exit", "exit", "a status"),
        ] {
            assert_eq!(
                problem(text).problem,
                Problem::Missing { step, what },
                "{text}"
            );
        }
        let missing = |text: &str| {
            parse(&format!("expect x\n{text}"))
                .expect_err("bad")
                .problem
        };
        assert_eq!(
            missing("respond"),
            Problem::Missing {
                step: "respond",
                what: "a JSON result"
            }
        );
        assert_eq!(
            missing("respond-error"),
            Problem::Missing {
                step: "respond-error",
                what: "a code"
            }
        );
        assert_eq!(
            missing("respond-error -32000"),
            Problem::Missing {
                step: "respond-error",
                what: "a message"
            }
        );
    }

    #[test]
    fn json_that_doesnt_parse_is_refused() {
        assert!(matches!(
            problem("send {\"jsonrpc\":").problem,
            Problem::BadJson { step: "send", .. }
        ));
        assert!(matches!(
            problem("send {} {}").problem,
            Problem::BadJson { step: "send", .. }
        ));
        let bad = |text: &str| parse(&format!("expect x\n{text}")).expect_err("bad");
        assert!(matches!(
            bad("respond {nope}").problem,
            Problem::BadJson {
                step: "respond",
                ..
            }
        ));
        assert!(matches!(
            bad(r#"respond-error -32022 "unterminated"#).problem,
            Problem::BadJson {
                step: "respond-error",
                ..
            }
        ));
        assert!(matches!(
            bad(r#"respond-error -32022 "message" {"data":"#).problem,
            Problem::BadJson {
                step: "respond-error",
                ..
            }
        ));
        assert_eq!(bad("respond {nope}").line, 2);
    }

    #[test]
    fn a_code_that_isnt_a_number_is_refused() {
        assert_eq!(
            problem("exit one").problem,
            Problem::BadCode {
                step: "exit",
                text: "one".into()
            }
        );
        assert_eq!(
            problem("exit 4294967296").problem,
            Problem::BadCode {
                step: "exit",
                text: "4294967296".into()
            }
        );
        let bad = parse("expect x\nrespond-error -32k0 oops").expect_err("bad");
        assert_eq!(
            bad.problem,
            Problem::BadCode {
                step: "respond-error",
                text: "-32k0".into()
            }
        );
    }

    #[test]
    fn a_response_before_any_expect_is_refused() {
        assert_eq!(
            problem("send {}\nrespond {}"),
            BadScript {
                line: 2,
                problem: Problem::NoRequest { step: "respond" }
            }
        );
        assert_eq!(
            problem("respond-error -1 no").problem,
            Problem::NoRequest {
                step: "respond-error"
            }
        );
    }

    #[test]
    fn parsing_never_panics_on_arbitrary_text() {
        let words = [
            "record",
            "expect",
            "send",
            "send-raw",
            "respond",
            "respond-error",
            "stderr",
            "wait-file",
            "exit",
            "\"",
            "{",
            "}",
            "-32022",
            "é",
            "\t",
            "\r",
            "#",
            "[1",
            "null",
        ];
        let mut seed: u64 = 0x5c41_97f0_0dd5_eed5;
        let mut next = |n: usize| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed % n as u64) as usize
        };
        for _ in 0..2000 {
            let mut text = String::new();
            for _ in 0..next(8) {
                for _ in 0..next(5) {
                    text.push_str(words[next(words.len())]);
                    text.push(if next(3) == 0 { '\n' } else { ' ' });
                }
            }
            let _ = parse(&text).map_err(|bad| bad.to_string());
        }
    }

    // --- what the player reads and writes ---------------------------------

    #[test]
    fn a_received_line_gives_its_method_and_id() {
        let request = br#"{"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{}}"#;
        assert_eq!(
            Received::parse(request),
            Some(Received {
                method: Some("session/prompt".into()),
                id: Some(json!(3))
            })
        );
        let notification = br#"{"jsonrpc":"2.0","method":"session/cancel","params":{}}"#;
        assert_eq!(
            Received::parse(notification),
            Some(Received {
                method: Some("session/cancel".into()),
                id: None
            })
        );
        let answer = br#"{"jsonrpc":"2.0","id":"p1","result":{}}"#;
        assert_eq!(
            Received::parse(answer),
            Some(Received {
                method: None,
                id: Some(json!("p1"))
            })
        );
        assert_eq!(Received::parse(b"not json"), None);
        assert_eq!(Received::parse(b"[1,2]"), None);
    }

    #[test]
    fn only_a_response_with_its_id_answers_a_request() {
        let parsed = |line: &[u8]| Received::parse(line).expect("a message");
        let answer = parsed(br#"{"jsonrpc":"2.0","id":"p1","result":{}}"#);
        assert!(answer.answers(&json!("p1")));
        assert!(!answer.answers(&json!("p2")));
        assert!(!answer.answers(&json!(1)), "an id's type counts");
        let error = parsed(br#"{"jsonrpc":"2.0","id":4,"error":{"code":-32601,"message":"x"}}"#);
        assert!(error.answers(&json!(4)));
        let request = parsed(br#"{"jsonrpc":"2.0","id":"p1","method":"session/prompt"}"#);
        assert!(!request.answers(&json!("p1")), "a request isn't a response");
    }

    #[test]
    fn a_result_line_answers_the_id_as_sent() {
        assert_eq!(
            result_line(&json!("p1"), &json!({"stopReason": "end_turn"})),
            "{\"jsonrpc\":\"2.0\",\"id\":\"p1\",\"result\":{\"stopReason\":\"end_turn\"}}\n"
        );
    }

    #[test]
    fn an_error_line_carries_data_only_when_given() {
        assert_eq!(
            error_line(&json!(1), -32000, "run login", None),
            "{\"jsonrpc\":\"2.0\",\"id\":1,\"error\":{\"code\":-32000,\"message\":\"run login\"}}\n"
        );
        let data = json!({"requested": 1, "supported": [2]});
        let line = error_line(&json!(1), -32022, "unsupported", Some(&data));
        let value: Value = serde_json::from_str(&line).expect("JSON");
        assert_eq!(value["error"]["data"], data);
        assert!(line.ends_with('\n'));
    }
}
