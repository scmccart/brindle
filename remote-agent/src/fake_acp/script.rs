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
//! - `record-all <path>`: from then on, every message read from stdin is
//!   appended to this file, the ones an `expect` skips included, so a test
//!   counts what the agent was sent however the steps after it matched
//!   (OpenSpec change `agents-process-lifecycle`: an answer written twice).
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
//! - `send-part <text>`: write the text with no line end and flush, so a
//!   run of them and a `send-raw` after make one long line arriving in
//!   pieces (OpenSpec change `agents-checkpoint-deltas`). The text is
//!   trimmed, as every step's argument is, so a piece neither starts nor
//!   ends with a space.
//! - `respond <json>`: answer the last request an `expect` matched with
//!   that `result`; a notification matched since (`session/cancel`)
//!   doesn't change which.
//! - `respond-error <code> <message> [<data>]`: answer it with an error;
//!   the message is one word, or a JSON string for one with spaces, and the
//!   optional JSON after it is the error's `data` (a `-32022`'s
//!   `{ requested, supported }`).
//! - `respond-to <id> <json>`: answer the agent's earlier request `id` (a
//!   JSON value: `3`, `"p1"`) with that `result`, whichever request an
//!   `expect` matched last (OpenSpec change `agents-login`: a late answer
//!   to an `authenticate` the client cancelled).
//! - `respond-error-to <id> <code> <message> [<data>]`: answer request
//!   `id` with an error, its arguments after the id as `respond-error`'s.
//! - `stderr <text>`: write the text as a line to stderr.
//! - `wait-file <path>`: wait until the file exists, a gate the test
//!   creates (never a sleep).
//! - `record-env <path>`: write the agent's environment to the file, one
//!   `KEY=value` line per variable, sorted by name ([`env_text`]).
//! - `exit <code>`: exit with that status.
//! - `args <word…>`: the steps after it, up to the next `args`, are a
//!   section played when the agent's arguments after the script are
//!   exactly those words; the steps before the first `args` are played
//!   when there are none ([`Script::steps_for`]). A terminal login runs
//!   the agent's command with the login method's arguments, so one script
//!   plays both the agent and its login program.
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
    /// Every message read from then on goes to `path`.
    RecordAll {
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
    /// Written with no line end, so pieces make one long line.
    SendPart {
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
    /// The answer to the agent's earlier request `id`.
    RespondTo {
        id: Value,
        result: Value,
    },
    /// An error answering the agent's earlier request `id`.
    RespondErrorTo {
        id: Value,
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
    RecordEnv {
        path: String,
    },
    Exit {
        code: i32,
    },
}

/// A parsed script: its sections, the first one (played with no
/// arguments) always there, maybe empty.
#[derive(Debug, Clone, PartialEq)]
pub struct Script {
    sections: Vec<Section>,
}

/// The steps played when the agent's arguments after the script are
/// exactly `args`.
#[derive(Debug, Clone, PartialEq)]
struct Section {
    args: Vec<String>,
    steps: Vec<Step>,
}

impl Script {
    /// The steps to play for the agent's arguments after the script, or
    /// `None` when no section is for them.
    pub fn steps_for(&self, args: &[String]) -> Option<&[Step]> {
        self.sections
            .iter()
            .find(|section| section.args == args)
            .map(|section| section.steps.as_slice())
    }
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
    /// A `respond` or `respond-error` before any `expect` of its section:
    /// there is no request to answer.
    NoRequest { step: &'static str },
    /// A second `args` section for the same words.
    RepeatedSection { words: Vec<String> },
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
            Problem::RepeatedSection { words } => {
                write!(f, "a second args section for {:?}", words.join(" "))
            }
        }
    }
}

/// The script `text`: its steps in order, in their sections.
pub fn parse(text: &str) -> Result<Script, BadScript> {
    let mut sections = vec![Section {
        args: Vec::new(),
        steps: Vec::new(),
    }];
    // Whether the section so far has an `expect` a response can answer.
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
        if word == "args" {
            let words: Vec<String> = rest.split_whitespace().map(str::to_owned).collect();
            if words.is_empty() {
                return Err(bad(Problem::Missing {
                    step: "args",
                    what: "a word",
                }));
            }
            if sections.iter().any(|section| section.args == words) {
                return Err(bad(Problem::RepeatedSection { words }));
            }
            sections.push(Section {
                args: words,
                steps: Vec::new(),
            });
            expected = false;
            continue;
        }
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
            | Step::RecordAll { .. }
            | Step::ExpectResponse { .. }
            | Step::Send { .. }
            | Step::SendRaw { .. }
            | Step::SendPart { .. }
            | Step::Respond { .. }
            | Step::RespondError { .. }
            | Step::RespondTo { .. }
            | Step::RespondErrorTo { .. }
            | Step::Stderr { .. }
            | Step::WaitFile { .. }
            | Step::RecordEnv { .. }
            | Step::Exit { .. } => {}
        }
        if let Some(section) = sections.last_mut() {
            section.steps.push(step);
        }
    }
    Ok(Script { sections })
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
        "record-all" => Step::RecordAll {
            path: needed("record-all", "a file")?,
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
        "send-part" => Step::SendPart {
            text: needed("send-part", "some text")?,
        },
        "respond" => Step::Respond {
            result: json_value("respond", &needed("respond", "a JSON result")?)?,
        },
        "respond-error" => {
            let (code, message, data) = error_parts("respond-error", rest)?;
            Step::RespondError {
                code,
                message,
                data,
            }
        }
        "respond-to" => {
            let (id, rest) = request_id("respond-to", rest)?;
            if rest.is_empty() {
                return Err(Problem::Missing {
                    step: "respond-to",
                    what: "a JSON result",
                });
            }
            Step::RespondTo {
                id,
                result: json_value("respond-to", rest)?,
            }
        }
        "respond-error-to" => {
            let (id, rest) = request_id("respond-error-to", rest)?;
            let (code, message, data) = error_parts("respond-error-to", rest)?;
            Step::RespondErrorTo {
                id,
                code,
                message,
                data,
            }
        }
        "stderr" => Step::Stderr {
            text: rest.to_owned(),
        },
        "wait-file" => Step::WaitFile {
            path: needed("wait-file", "a file")?,
        },
        "record-env" => Step::RecordEnv {
            path: needed("record-env", "a file")?,
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

/// The request id a `respond-to` or `respond-error-to` starts with (one
/// JSON value), and the text after it.
fn request_id<'a>(step: &'static str, rest: &'a str) -> Result<(Value, &'a str), Problem> {
    let mut values = serde_json::Deserializer::from_str(rest).into_iter::<Value>();
    let id = match values.next() {
        Some(Ok(id)) => id,
        Some(Err(error)) => {
            return Err(Problem::BadJson {
                step,
                error: error.to_string(),
            });
        }
        None => {
            return Err(Problem::Missing {
                step,
                what: "a request id",
            });
        }
    };
    let after = rest.get(values.byte_offset()..).unwrap_or_default();
    Ok((id, after.trim()))
}

/// An error answer's `<code> <message> [<data>]`, for `step`.
fn error_parts(step: &'static str, rest: &str) -> Result<(i64, String, Option<Value>), Problem> {
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
    Ok((code, message, data))
}

/// What `record-env` writes of the environment `vars`: one `KEY=value`
/// line per variable, sorted by name (a value's own line breaks are
/// written as they are).
pub fn env_text(vars: impl IntoIterator<Item = (String, String)>) -> String {
    let mut vars: Vec<(String, String)> = vars.into_iter().collect();
    vars.sort();
    vars.iter()
        .map(|(key, value)| format!("{key}={value}\n"))
        .collect()
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

    /// The steps `text` plays with no arguments.
    fn steps(text: &str) -> Result<Vec<Step>, BadScript> {
        parse(text).map(|script| script.steps_for(&[]).unwrap_or_default().to_vec())
    }

    fn one(text: &str) -> Step {
        let steps = steps(text).unwrap_or_else(|bad| panic!("{text:?}: {bad}"));
        assert_eq!(steps.len(), 1, "{steps:?}");
        steps.into_iter().next().expect("one step")
    }

    /// `text`'s last step, after an `expect` so a response may follow.
    fn answer(text: &str) -> Step {
        let steps = steps(&format!("expect session/new\n{text}"))
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
    fn record_all_names_the_file_every_message_goes_to() {
        assert_eq!(
            one("record-all /tmp/case/all.jsonl"),
            Step::RecordAll {
                path: "/tmp/case/all.jsonl".into()
            }
        );
        assert_eq!(
            problem("record-all").problem,
            Problem::Missing {
                step: "record-all",
                what: "a file"
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
    fn send_part_keeps_its_text_and_needs_some() {
        assert_eq!(
            one(r#"send-part {"jsonrpc":"2.0","#),
            Step::SendPart {
                text: r#"{"jsonrpc":"2.0","#.into()
            }
        );
        assert!(parse("send-part\n").is_err());
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
    fn respond_to_answers_a_request_named_by_its_json_id_with_no_expect_needed() {
        assert_eq!(
            one("respond-to 3 {}"),
            Step::RespondTo {
                id: json!(3),
                result: json!({})
            }
        );
        assert_eq!(
            one(r#"respond-to "auth-1" {"ok":true}"#),
            Step::RespondTo {
                id: json!("auth-1"),
                result: json!({"ok": true})
            }
        );
        let missing = |text: &str| problem(text).problem;
        assert_eq!(
            missing("respond-to"),
            Problem::Missing {
                step: "respond-to",
                what: "a request id"
            }
        );
        assert_eq!(
            missing("respond-to 3"),
            Problem::Missing {
                step: "respond-to",
                what: "a JSON result"
            }
        );
        assert!(matches!(
            missing("respond-to auth-1 {}"),
            Problem::BadJson {
                step: "respond-to",
                ..
            }
        ));
        assert!(matches!(
            missing("respond-to 3 {} {}"),
            Problem::BadJson {
                step: "respond-to",
                ..
            }
        ));
    }

    #[test]
    fn respond_error_to_takes_an_id_then_respond_errors_arguments() {
        assert_eq!(
            one(r#"respond-error-to 3 -32000 "browser login timed out""#),
            Step::RespondErrorTo {
                id: json!(3),
                code: -32000,
                message: "browser login timed out".into(),
                data: None
            }
        );
        assert_eq!(
            one(r#"respond-error-to "a" -32800 cancelled {"why":"user"}"#),
            Step::RespondErrorTo {
                id: json!("a"),
                code: -32800,
                message: "cancelled".into(),
                data: Some(json!({"why": "user"}))
            }
        );
        let missing = |text: &str| problem(text).problem;
        assert_eq!(
            missing("respond-error-to 3"),
            Problem::Missing {
                step: "respond-error-to",
                what: "a code"
            }
        );
        assert_eq!(
            missing("respond-error-to 3 -32000"),
            Problem::Missing {
                step: "respond-error-to",
                what: "a message"
            }
        );
        assert_eq!(
            missing("respond-error-to 3 x y"),
            Problem::BadCode {
                step: "respond-error-to",
                text: "x".into()
            }
        );
    }

    #[test]
    fn record_env_names_its_file() {
        assert_eq!(
            one("record-env /tmp/case/env.txt"),
            Step::RecordEnv {
                path: "/tmp/case/env.txt".into()
            }
        );
        assert_eq!(
            problem("record-env").problem,
            Problem::Missing {
                step: "record-env",
                what: "a file"
            }
        );
    }

    #[test]
    fn the_environment_is_written_sorted_one_variable_a_line() {
        let vars = [
            ("PATH", "/usr/bin"),
            ("LOGIN_MODE", "device"),
            ("A", ""),
            ("EQ", "a=b"),
        ]
        .map(|(key, value)| (key.to_owned(), value.to_owned()));
        assert_eq!(
            env_text(vars),
            "A=\nEQ=a=b\nLOGIN_MODE=device\nPATH=/usr/bin\n"
        );
        assert_eq!(env_text([]), "");
    }

    #[test]
    fn args_sections_are_chosen_by_the_agents_exact_arguments() {
        let script = parse(
            "expect initialize\nrespond {}\n\
             args login\nexit 0\n\
             args login --device\n  stderr device\nexit 1\n",
        )
        .expect("a script");
        let words = |words: &[&str]| words.iter().map(|word| word.to_string()).collect::<Vec<_>>();
        assert_eq!(
            script.steps_for(&[]),
            Some(
                &[
                    Step::Expect {
                        method: "initialize".into()
                    },
                    Step::Respond { result: json!({}) },
                ][..]
            )
        );
        assert_eq!(
            script.steps_for(&words(&["login"])),
            Some(&[Step::Exit { code: 0 }][..])
        );
        assert_eq!(
            script.steps_for(&words(&["login", "--device"])),
            Some(
                &[
                    Step::Stderr {
                        text: "device".into()
                    },
                    Step::Exit { code: 1 },
                ][..]
            )
        );
        // Exactly those words, in that order.
        assert_eq!(script.steps_for(&words(&["--device", "login"])), None);
        assert_eq!(script.steps_for(&words(&["logout"])), None);
        // A script with no sections plays only without arguments.
        let plain = parse("exit 0").expect("a script");
        assert_eq!(plain.steps_for(&words(&["login"])), None);
        assert_eq!(plain.steps_for(&[]), Some(&[Step::Exit { code: 0 }][..]));
    }

    #[test]
    fn an_args_section_needs_words_of_its_own_and_its_own_expect() {
        assert_eq!(
            problem("exit 0\nargs").problem,
            Problem::Missing {
                step: "args",
                what: "a word"
            }
        );
        let repeated = problem("args login\nexit 0\nargs  login \nexit 1");
        assert_eq!(
            repeated,
            BadScript {
                line: 3,
                problem: Problem::RepeatedSection {
                    words: vec!["login".into()]
                }
            }
        );
        assert_eq!(
            repeated.to_string(),
            "line 3: a second args section for \"login\""
        );
        // An `expect` in an earlier section answers nothing in this one.
        assert_eq!(
            problem("expect initialize\nargs login\nrespond {}").problem,
            Problem::NoRequest { step: "respond" }
        );
    }

    /// Task 4.1 of `agents-login`: a script playing an agent that offers
    /// login methods, has a login cancelled, answers the cancelled
    /// `authenticate` late and the next one on time; and its login
    /// program's section.
    #[test]
    fn a_login_script_parses() {
        let initialize = json!({
            "protocolVersion": 1,
            "authMethods": [
                {"id": "oauth", "name": "Log in with a browser"},
                {"id": "cli-login", "name": "Log in from the terminal", "type": "terminal",
                 "args": ["login"], "env": {"LOGIN_MODE": "device"}},
            ],
        });
        let text = format!(
            "record got.jsonl\n\
             expect initialize\n\
             respond {initialize}\n\
             expect session/new\n\
             respond-error -32000 \"log in first\"\n\
             expect authenticate\n\
             expect $/cancel_request\n\
             respond-to 3 {{}}\n\
             expect authenticate\n\
             respond-error-to 4 -32000 \"browser login timed out\"\n\
             expect authenticate\n\
             respond {{}}\n\
             expect session/new\n\
             respond {{\"sessionId\":\"s1\"}}\n\
             args login\n\
             record-env env.txt\n\
             exit 0\n"
        );
        let script = parse(&text).unwrap_or_else(|bad| panic!("{bad}"));
        let steps = script.steps_for(&[]).expect("the agent's steps");
        assert_eq!(steps.len(), 14, "{steps:?}");
        assert_eq!(
            steps[2],
            Step::Respond {
                result: initialize
            }
        );
        assert_eq!(
            steps[6],
            Step::Expect {
                method: "$/cancel_request".into()
            }
        );
        assert_eq!(
            steps[7],
            Step::RespondTo {
                id: json!(3),
                result: json!({})
            }
        );
        assert_eq!(
            steps[9],
            Step::RespondErrorTo {
                id: json!(4),
                code: -32000,
                message: "browser login timed out".into(),
                data: None
            }
        );
        assert_eq!(
            script.steps_for(&["login".to_owned()]),
            Some(
                &[
                    Step::RecordEnv {
                        path: "env.txt".into()
                    },
                    Step::Exit { code: 0 },
                ][..]
            )
        );
    }

    #[test]
    fn comments_and_blank_lines_are_skipped_and_steps_keep_their_order() {
        let script = "# a fake agent\n\nrecord got.jsonl\n  expect initialize\n\
                      respond {\"protocolVersion\":1}\r\nexit 0\n";
        assert_eq!(
            steps(script),
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
            "respond-to",
            "respond-error-to",
            "stderr",
            "wait-file",
            "record-env",
            "args",
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
