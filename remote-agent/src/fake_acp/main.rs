//! `brindle-fake-acp-agent SCRIPT [ARG…]`: an ACP agent played from a
//! script, for the terminal server's tests (OpenSpec change
//! `agents-sessions`, design decision 16). It is built with the helper and
//! never packaged.
//!
//! The script's steps are [`script`]'s, read and checked whole before the
//! first one runs; the `ARG`s after the script choose its `args` section
//! (none: the steps before the first section), and an `ARG` list no
//! section is for is a usage error. This is the thin loop that plays them:
//! it reads JSON-RPC lines from stdin, writes lines to stdout and stderr,
//! appends the messages an `expect` or `expect-response` matched to the
//! `record` file, writes its environment for `record-env`, and waits for
//! `wait-file` gates. A `respond` answers the last request an `expect`
//! matched: a notification matched since doesn't change which; a
//! `respond-to` answers the request its id names.
//!
//! - **When the script runs out** without an `exit`, the agent idles as a
//!   real one would: it reads (and drops) stdin until it closes, then exits
//!   0.
//! - **A `wait-file`** looks for its file every 10 ms, with no limit: the
//!   test that wrote the gate creates the file, and a hang-up ends an agent
//!   still waiting.
//! - **Exit statuses:** a script's own `exit`, 0 after idling, 2 for a usage
//!   error or a script that doesn't parse, and 3 when a step can't be played
//!   (stdin closed before an `expect`'s method or an `expect-response`'s
//!   response came, a response when every `expect` so far matched a
//!   notification, or an IO error), said on stderr.

// railway R3, types R2: no `_ =>` on one of Brindle's own enums. Non-test
// code only, as in the helper's root.
#![cfg_attr(not(test), warn(clippy::wildcard_enum_match_arm))]

mod script;

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, Write};
use std::path::Path;
use std::process::ExitCode;
use std::thread;
use std::time::Duration;

use brindle_remote_protocol::serde_json::Value;

use script::{Received, Step};

/// How often a `wait-file` looks for its file.
const GATE_POLL: Duration = Duration::from_millis(10);

/// Why a step couldn't be played.
enum Fault {
    /// Stdin closed before a message with this method (or the response
    /// named so) came.
    Closed { method: String },
    /// A `respond` or `respond-error` when every `expect` so far matched a
    /// notification, which has no `id` to answer.
    NoId { method: String },
    /// A read or write failed; `what` names it.
    Io { what: String, error: io::Error },
}

impl Fault {
    fn io(what: impl Into<String>) -> impl FnOnce(io::Error) -> Self {
        let what = what.into();
        move |error| Self::Io { what, error }
    }
}

impl std::fmt::Display for Fault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Closed { method } => write!(f, "stdin closed before {method} came"),
            Self::NoId { method } => {
                write!(f, "the {method} it answers is a notification, with no id")
            }
            Self::Io { what, error } => write!(f, "{what}: {error}"),
        }
    }
}

/// How the script ended.
enum End {
    /// An `exit` step.
    Exit(i32),
    /// The steps ran out, and stdin then closed.
    Idled,
}

fn main() -> ExitCode {
    let mut args = std::env::args_os().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: brindle-fake-acp-agent SCRIPT [ARG…]");
        return ExitCode::from(2);
    };
    let words: Vec<String> = args.map(|arg| arg.to_string_lossy().into_owned()).collect();
    let path = Path::new(&path);
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) => {
            eprintln!(
                "brindle-fake-acp-agent: can't read {}: {error}",
                path.display()
            );
            return ExitCode::from(2);
        }
    };
    let script = match script::parse(&text) {
        Ok(script) => script,
        Err(bad) => {
            eprintln!("brindle-fake-acp-agent: {}: {bad}", path.display());
            return ExitCode::from(2);
        }
    };
    let Some(steps) = script.steps_for(&words) else {
        eprintln!(
            "brindle-fake-acp-agent: {} has no args section for {:?}",
            path.display(),
            words.join(" ")
        );
        return ExitCode::from(2);
    };
    match play(steps) {
        Ok(End::Exit(code)) => std::process::exit(code),
        Ok(End::Idled) => ExitCode::SUCCESS,
        Err(fault) => {
            eprintln!("brindle-fake-acp-agent: {fault}");
            ExitCode::from(3)
        }
    }
}

/// The last request an `expect` matched: its method and its `id`, if any.
struct Last {
    method: String,
    id: Option<Value>,
}

/// Plays `steps` in order.
fn play(steps: &[Step]) -> Result<End, Fault> {
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    let mut record: Option<File> = None;
    let mut all: Option<File> = None;
    let mut last: Option<Last> = None;
    let mut line = Vec::new();
    for step in steps {
        match step {
            Step::Record { path } => record = Some(appending(path)?),
            Step::RecordAll { path } => all = Some(appending(path)?),
            Step::Expect { method } => {
                let wanted = |received: &Received| received.method.as_deref() == Some(method);
                let records = Records {
                    matched: record.as_mut(),
                    all: all.as_mut(),
                };
                let id = expect(&mut input, &mut line, method, wanted, records)?;
                // A notification leaves the request a response answers.
                if id.is_some() || last.is_none() {
                    last = Some(Last {
                        method: method.clone(),
                        id,
                    });
                }
            }
            Step::ExpectResponse { id } => {
                let what = format!("the response to {id}");
                let wanted = |received: &Received| received.answers(id);
                let records = Records {
                    matched: record.as_mut(),
                    all: all.as_mut(),
                };
                expect(&mut input, &mut line, &what, wanted, records)?;
            }
            Step::Send { line } => write_line(&mut output, line)?,
            Step::SendRaw { text } => write_line(&mut output, text)?,
            Step::SendPart { text } => write_text(&mut output, text)?,
            Step::Respond { result } => {
                let id = answering(last.as_ref())?;
                write_text(&mut output, &script::result_line(id, result))?;
            }
            Step::RespondError {
                code,
                message,
                data,
            } => {
                let id = answering(last.as_ref())?;
                let text = script::error_line(id, *code, message, data.as_ref());
                write_text(&mut output, &text)?;
            }
            Step::RespondTo { id, result } => {
                write_text(&mut output, &script::result_line(id, result))?;
            }
            Step::RespondErrorTo {
                id,
                code,
                message,
                data,
            } => {
                let text = script::error_line(id, *code, message, data.as_ref());
                write_text(&mut output, &text)?;
            }
            Step::RecordEnv { path } => record_env(path)?,
            Step::Stderr { text } => {
                let mut error = io::stderr().lock();
                writeln!(error, "{text}")
                    .and_then(|()| error.flush())
                    .map_err(Fault::io("writing to stderr"))?;
            }
            Step::WaitFile { path } => {
                while !Path::new(path).exists() {
                    thread::sleep(GATE_POLL);
                }
            }
            Step::Exit { code } => return Ok(End::Exit(*code)),
        }
    }
    // Idle until stdin closes, as an agent with nothing left to say.
    loop {
        line.clear();
        let read = input
            .read_until(b'\n', &mut line)
            .map_err(Fault::io("reading stdin"))?;
        if read == 0 {
            return Ok(End::Idled);
        }
    }
}

/// Reads lines until a message `wanted` comes (`method` names it for the
/// fault), appending it to `record`; gives its `id`. Lines before it are
/// skipped.
fn expect(
    input: &mut impl BufRead,
    line: &mut Vec<u8>,
    method: &str,
    wanted: impl Fn(&Received) -> bool,
    records: Records<'_>,
) -> Result<Option<Value>, Fault> {
    let Records { matched, mut all } = records;
    loop {
        line.clear();
        let read = input
            .read_until(b'\n', line)
            .map_err(Fault::io("reading stdin"))?;
        if read == 0 {
            return Err(Fault::Closed {
                method: method.to_owned(),
            });
        }
        let Some(received) = Received::parse(line) else {
            continue;
        };
        // One write per message, so a reader sees whole lines.
        if line.last() != Some(&b'\n') {
            line.push(b'\n');
        }
        if let Some(file) = all.as_deref_mut() {
            file.write_all(line)
                .map_err(Fault::io("writing the record of every message"))?;
        }
        if !wanted(&received) {
            continue;
        }
        if let Some(file) = matched {
            file.write_all(line)
                .map_err(Fault::io("writing the record"))?;
        }
        return Ok(received.id);
    }
}

/// The files an `expect` appends to: the message it matched (`record`'s),
/// and every message it read (`record-all`'s).
struct Records<'a> {
    matched: Option<&'a mut File>,
    all: Option<&'a mut File>,
}

/// `path` opened for appending, created if it isn't there.
fn appending(path: &str) -> Result<File, Fault> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(Fault::io(format!("opening {path}")))
}

/// The `id` a response answers: the last expected message's.
fn answering(last: Option<&Last>) -> Result<&Value, Fault> {
    // The parser refuses a response before any expect, so `last` is set.
    let last = last.ok_or_else(|| Fault::NoId {
        method: "nothing".to_owned(),
    })?;
    last.id.as_ref().ok_or_else(|| Fault::NoId {
        method: last.method.clone(),
    })
}

/// Writes the agent's environment to `path` ([`script::env_text`]), whole:
/// through a file beside it renamed into place, so a reader never sees a
/// part.
fn record_env(path: &str) -> Result<(), Fault> {
    let vars = std::env::vars_os().map(|(key, value)| {
        (
            key.to_string_lossy().into_owned(),
            value.to_string_lossy().into_owned(),
        )
    });
    let partial = format!("{path}.partial");
    fs::write(&partial, script::env_text(vars))
        .and_then(|()| fs::rename(&partial, path))
        .map_err(Fault::io(format!("writing the environment to {path}")))
}

fn write_line(output: &mut impl Write, text: &str) -> Result<(), Fault> {
    let mut line = String::with_capacity(text.len() + 1);
    line.push_str(text);
    line.push('\n');
    write_text(output, &line)
}

/// Writes `text` (whole lines) and flushes, so the reader gets it at once.
fn write_text(output: &mut impl Write, text: &str) -> Result<(), Fault> {
    output
        .write_all(text.as_bytes())
        .and_then(|()| output.flush())
        .map_err(Fault::io("writing to stdout"))
}
