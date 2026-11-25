//! `brindle-fake-acp-agent SCRIPT`: an ACP agent played from a script, for
//! the terminal server's tests (OpenSpec change `agents-sessions`, design
//! decision 16). It is built with the helper and never packaged.
//!
//! The script's steps are [`script`]'s, read and checked whole before the
//! first one runs. This is the thin loop that plays them: it reads JSON-RPC
//! lines from stdin, writes lines to stdout and stderr, appends the messages
//! an `expect` matched to the `record` file, and waits for `wait-file`
//! gates.
//!
//! - **When the script runs out** without an `exit`, the agent idles as a
//!   real one would: it reads (and drops) stdin until it closes, then exits
//!   0.
//! - **A `wait-file`** looks for its file every 10 ms, with no limit: the
//!   test that wrote the gate creates the file, and a hang-up ends an agent
//!   still waiting.
//! - **Exit statuses:** a script's own `exit`, 0 after idling, 2 for a usage
//!   error or a script that doesn't parse, and 3 when a step can't be played
//!   (stdin closed before an `expect`'s method came, a response to a
//!   message with no `id`, or an IO error), said on stderr.

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
    /// Stdin closed before a message with this method came.
    Closed { method: String },
    /// A `respond` or `respond-error` after an `expect` that matched a
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
    let Some(path) = std::env::args_os().nth(1) else {
        eprintln!("usage: brindle-fake-acp-agent SCRIPT");
        return ExitCode::from(2);
    };
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
    let steps = match script::parse(&text) {
        Ok(steps) => steps,
        Err(bad) => {
            eprintln!("brindle-fake-acp-agent: {}: {bad}", path.display());
            return ExitCode::from(2);
        }
    };
    match play(&steps) {
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
    let mut last: Option<Last> = None;
    let mut line = Vec::new();
    for step in steps {
        match step {
            Step::Record { path } => {
                let file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(path)
                    .map_err(Fault::io(format!("opening {path}")))?;
                record = Some(file);
            }
            Step::Expect { method } => {
                let id = expect(&mut input, &mut line, method, record.as_mut())?;
                last = Some(Last {
                    method: method.clone(),
                    id,
                });
            }
            Step::Send { line } => write_line(&mut output, line)?,
            Step::SendRaw { text } => write_line(&mut output, text)?,
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

/// Reads lines until a message with `method` comes, appending it to
/// `record`; gives its `id`. Lines before it are skipped.
fn expect(
    input: &mut impl BufRead,
    line: &mut Vec<u8>,
    method: &str,
    record: Option<&mut File>,
) -> Result<Option<Value>, Fault> {
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
        if received.method.as_deref() != Some(method) {
            continue;
        }
        if let Some(file) = record {
            // One write per message, so a reader sees whole lines.
            if line.last() != Some(&b'\n') {
                line.push(b'\n');
            }
            file.write_all(line)
                .map_err(Fault::io("writing the record"))?;
        }
        return Ok(received.id);
    }
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
