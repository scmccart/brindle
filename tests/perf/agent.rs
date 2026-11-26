//! `agent-markdown-stream` (OpenSpec change `agents-markdown`, design.md
//! decisions 6 and 13): how long an agent zone takes to show a long
//! Markdown reply streamed in small chunks, against the same reply sent
//! whole. The fake ACP agent (`tests/common/fake_agent.rs`) plays the
//! agent; it holds the reply behind a gate the case opens once the agent
//! has the prompt, so each time runs from the gate to Brindle's exit on a
//! `--wait-for` of the reply's last line. Numbers only (`testing R14`).

use std::path::{Path, PathBuf};
use std::process::Child;
use std::time::{Duration, Instant};

use crate::cases::spawn_with;
use crate::common::fake_agent;
use crate::report::StreamResult;
use crate::{Ctx, Measured, Result, ensure};

/// The reply's length, exactly.
pub const ANSWER_BYTES: usize = 64 * 1024;
/// The lines inside the reply's one fence.
pub const FENCE_LINES: usize = 1_500;
/// The streamed run's chunk size, the last chunk excepted.
pub const CHUNK_BYTES: usize = 32;
/// The reply's last line, and what Brindle's `--wait-for` waits for: it is
/// complete only with the reply's last byte.
pub const LAST_LINE: &str = "End of the streamed answer.";
/// Brindle's step interval: the quit comes one interval after the wait
/// sees the last line, in both runs. Its `--wait-for` gives up after 30 of
/// them.
const STEP_INTERVAL: Duration = Duration::from_secs(1);
/// How long the agent may take to be sent the prompt.
const PROMPT_LIMIT: Duration = Duration::from_secs(60);
/// How long Brindle may take to exit once the gate is open: past the
/// `--wait-for`'s own limit, which then quits it anyway.
const EXIT_LIMIT: Duration = Duration::from_secs(60);
/// How often the case looks at the record file and Brindle's process.
const POLL: Duration = Duration::from_millis(5);

/// A paragraph of the prose around the fence, numbered.
fn prose(n: usize) -> String {
    format!(
        "Paragraph {n}: the renderer parses the growing message again on each absorb, and only the \
         parts that changed are spliced into the list. This one has *emphasis*, **strong** text, \
         `inline code` and a [link](https://example.com/{n}).\n\n"
    )
}

/// The reply: a heading, prose, bulleted, nested, numbered and task
/// lists, a fence of [`FENCE_LINES`] lines, more lists and prose, and
/// [`LAST_LINE`] as its own paragraph, padded with prose to exactly
/// [`ANSWER_BYTES`]. ASCII, so every byte offset is a char boundary.
pub fn answer() -> String {
    let mut out = String::from("# Streaming benchmark\n\n");
    out.push_str(&prose(0));
    for item in 1..=20 {
        out.push_str(&format!("- Item {item} with `code` and *emphasis*\n"));
        if item % 5 == 0 {
            out.push_str(&format!("  - Nested under {item}\n  - And another\n"));
        }
    }
    out.push('\n');
    for item in 1..=20 {
        out.push_str(&format!("{item}. Step {item} of the plan\n"));
    }
    out.push('\n');
    for item in 1..=10 {
        out.push_str(&format!("- [{}] Task {item}\n", if item % 2 == 0 { "x" } else { " " }));
    }
    out.push_str("\n```rust\n");
    for line in 1..=FENCE_LINES {
        out.push_str(&format!("let v{line:04} = v{:04} + {line};\n", line - 1));
    }
    out.push_str("```\n\n");
    for item in 1..=10 {
        out.push_str(&format!("- After the fence, item {item}\n"));
    }
    out.push('\n');
    let mut n = 1;
    loop {
        let paragraph = prose(n);
        if out.len() + paragraph.len() + LAST_LINE.len() > ANSWER_BYTES {
            break;
        }
        out.push_str(&paragraph);
        n += 1;
    }
    // The gap left is shorter than a paragraph: part of one, then a blank
    // line, keeps the last line a paragraph of its own.
    let gap = ANSWER_BYTES - out.len() - LAST_LINE.len();
    if gap >= 2 {
        out.push_str(&prose(n)[..gap - 2]);
        out.push_str("\n\n");
    } else {
        out.push_str(&" "[..gap]);
    }
    out.push_str(LAST_LINE);
    out
}

/// `text` in pieces of at most `size` bytes, each ending on a char
/// boundary, in order.
pub fn chunks(text: &str, size: usize) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        let mut end = size.min(rest.len());
        while !rest.is_char_boundary(end) {
            end -= 1;
        }
        // A char wider than `size` goes whole.
        if end == 0 {
            end = rest.char_indices().nth(1).map_or(rest.len(), |(at, _)| at);
        }
        let (head, tail) = rest.split_at(end);
        out.push(head);
        rest = tail;
    }
    out
}

/// `text` as the inside of a JSON string.
pub fn json_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// The fake agent's script: ready at once, recording what it is sent in
/// `record`; on the prompt it waits for `gate`, then sends each of
/// `pieces` as an `agent_message_chunk` and ends the turn.
pub fn script(record: &Path, gate: &Path, pieces: &[&str]) -> String {
    let mut lines = vec![
        format!("record {}", record.display()),
        "expect initialize".to_string(),
        r#"respond {"protocolVersion":1,"agentInfo":{"name":"Fake Agent","version":"1.0"}}"#.to_string(),
        "expect session/new".to_string(),
        r#"respond {"sessionId":"s1"}"#.to_string(),
        "expect session/prompt".to_string(),
        format!("wait-file {}", gate.display()),
    ];
    lines.extend(pieces.iter().map(|piece| {
        format!(
            r#"send {{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"s1","update":{{"sessionUpdate":"agent_message_chunk","content":{{"type":"text","text":"{}"}}}}}}}}"#,
            json_escape(piece)
        )
    }));
    lines.push(r#"respond {"stopReason":"end_turn"}"#.to_string());
    lines.join("\n") + "\n"
}

/// The config: the perf suite's shell settings and an agent profile
/// `Fake` running the fake agent `agent` on `script`.
fn config(agent: &Path, script: &Path) -> String {
    format!(
        "{}\n[[profiles]]\nname = \"Fake\"\ntype = \"agent\"\ncommand = '{}'\nargs = ['{}']\n",
        crate::cases::CONFIG,
        agent.display(),
        script.display()
    )
}

/// The record file shows the prompt the agent was sent.
pub fn prompt_recorded(record: &str) -> bool {
    record.lines().any(|line| line.contains("\"session/prompt\""))
}

/// Brindle's log says a `--wait-for` gave up (`main.rs`'s
/// `wait_for_text`), so the time it quit at isn't the reply's.
pub fn wait_gave_up(log: &str) -> bool {
    log.lines().any(|line| line.contains("--wait-for:") && line.contains("didn't appear"))
}

/// Ends `child` when a step fails, so no window outlives the case.
fn end(mut child: Child) {
    child.kill().ok();
    child.wait().ok();
}

/// One run: Brindle opens `Fake` with the first prompt `go`, the case
/// opens the gate once the record shows the prompt, and Brindle quits on
/// the last line. Its files are `LABEL.*` in the case directory, and its
/// terminal server is stopped before the next run, its logs kept in
/// `LABEL/`. Returns the time from the gate to the exit.
fn run(ctx: &Ctx, agent: &Path, label: &str, pieces: &[&str]) -> Result<Duration> {
    let record = ctx.dir.join(format!("{label}.jsonl"));
    let gate = ctx.dir.join(format!("{label}.gate"));
    let script_path = ctx.dir.join(format!("{label}.script"));
    std::fs::write(&script_path, script(&record, &gate, pieces)).map_err(|e| format!("writing the script: {e}"))?;
    let interval = STEP_INTERVAL.as_secs_f64().to_string();
    let args = [
        "agent",
        "Fake",
        "--no-restore",
        "-p",
        "go",
        "--step-interval",
        &interval,
        "--wait-for",
        LAST_LINE,
        "--action",
        "quit",
    ];
    let mut child = spawn_with(ctx, label, &config(agent, &script_path), &args)?;

    let asked = Instant::now();
    loop {
        if std::fs::read_to_string(&record).is_ok_and(|text| prompt_recorded(&text)) {
            break;
        }
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            return Err(format!("{label}: Brindle exited before the agent had the prompt; see {label}.log"));
        }
        if asked.elapsed() > PROMPT_LIMIT {
            end(child);
            return Err(format!("{label}: no prompt within {}s; see {label}.log", PROMPT_LIMIT.as_secs()));
        }
        std::thread::sleep(POLL);
    }
    std::fs::write(&gate, b"").map_err(|e| format!("opening the gate: {e}"))?;
    let opened = Instant::now();
    let took = loop {
        if child.try_wait().map_err(|e| e.to_string())?.is_some() {
            break opened.elapsed();
        }
        if opened.elapsed() > EXIT_LIMIT {
            end(child);
            return Err(format!("{label}: Brindle didn't exit within {}s; see {label}.log", EXIT_LIMIT.as_secs()));
        }
        std::thread::sleep(POLL);
    };
    let log = std::fs::read_to_string(ctx.dir.join(format!("{label}.log"))).unwrap_or_default();
    ensure!(!wait_gave_up(&log), "{label}: the last line never showed; see {label}.log");

    let logs = ctx.dir.join(label);
    std::fs::create_dir_all(&logs).map_err(|e| format!("creating {}: {e}", logs.display()))?;
    crate::common::server::stop(&ctx.helper, &ctx.server_dir, &logs)?;
    Ok(took)
}

pub fn markdown_stream(ctx: &Ctx) -> Result<Measured> {
    let agent = fake_agent::find(&ctx.default_bin);
    if let Err(why) = fake_agent::check(&agent, &ctx.default_bin) {
        return Ok(Measured::Skip(why));
    }
    let text = answer();
    let streamed = chunks(&text, CHUNK_BYTES);
    let streamed_time = run(ctx, &agent.path, "streamed", &streamed)?;
    let whole_time = run(ctx, &agent.path, "whole", &[text.as_str()])?;
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    Ok(Measured::Stream(StreamResult {
        bytes: text.len() as u64,
        chunks: streamed.len() as u64,
        streamed_ms: ms(streamed_time),
        whole_ms: ms(whole_time),
        difference_ms: ms(streamed_time) - ms(whole_time),
        step_interval_ms: ms(STEP_INTERVAL),
    }))
}

/// The reply, its chunks, the script and the run's readers (`--self-test`).
pub fn self_test() -> Result {
    let text = answer();
    ensure!(text.len() == ANSWER_BYTES, "the reply is {} bytes", text.len());
    ensure!(text.is_ascii(), "the reply isn't ASCII");
    ensure!(text.ends_with(&format!("\n\n{LAST_LINE}")), "the last line isn't a paragraph of its own");
    ensure!(text.matches(LAST_LINE).count() == 1, "the last line shows more than once");
    let fence: Vec<&str> = text.split("```").collect();
    ensure!(fence.len() == 3, "{} fence markers", fence.len() - 1);
    let code = fence[1].strip_prefix("rust\n").ok_or("the fence isn't labelled rust")?;
    ensure!(code.lines().count() == FENCE_LINES, "the fence has {} lines", code.lines().count());
    ensure!(text.contains("\n- Item 5 ") && text.contains("\n  - Nested under 5\n"), "no nested list");
    ensure!(text.contains("\n1. Step 1 ") && text.contains("\n- [x] Task 2\n"), "no numbered or task list");
    ensure!(text.split("```").nth(2).is_some_and(|after| after.contains("Paragraph 1:")), "no prose after the fence");

    let pieces = chunks(&text, CHUNK_BYTES);
    ensure!(pieces.len() == ANSWER_BYTES / CHUNK_BYTES, "{} chunks", pieces.len());
    ensure!(pieces.iter().all(|piece| piece.len() == CHUNK_BYTES), "a chunk isn't {CHUNK_BYTES} bytes");
    ensure!(pieces.concat() == text, "the chunks don't make the reply");
    let before_last = &text[..text.len() - pieces[pieces.len() - 1].len()];
    ensure!(!before_last.contains(LAST_LINE), "the last line is whole before the last chunk");
    // Never inside a char, and a char wider than the size goes whole.
    ensure!(chunks("aé€b", 2) == ["a", "é", "€", "b"], "{:?}", chunks("aé€b", 2));
    ensure!(chunks("", 32).is_empty(), "an empty text has chunks");

    let escaped = json_escape("a\"b\\c\nd\te\u{1}é");
    ensure!(escaped == "a\\\"b\\\\c\\nd\\te\\u0001é", "escaped {escaped}");

    let (record, gate) = (Path::new("/case/r.jsonl"), PathBuf::from("/case/g"));
    let lines: Vec<String> = script(record, &gate, &["a\"", "b"]).lines().map(String::from).collect();
    ensure!(lines[0] == "record /case/r.jsonl", "{lines:?}");
    let at = lines.iter().position(|l| l == "expect session/prompt").ok_or("no prompt step")?;
    ensure!(lines[at + 1] == "wait-file /case/g", "the gate doesn't follow the prompt: {lines:?}");
    let sends: Vec<&String> = lines.iter().filter(|l| l.starts_with("send ")).collect();
    ensure!(sends.len() == 2, "{} sends", sends.len());
    ensure!(sends[0].contains(r#""agent_message_chunk","content":{"type":"text","text":"a\""}"#), "{}", sends[0]);
    ensure!(lines.last().is_some_and(|l| l == r#"respond {"stopReason":"end_turn"}"#), "no end of turn: {lines:?}");

    let prompt = r#"{"jsonrpc":"2.0","id":2,"method":"session/prompt","params":{}}"#;
    ensure!(prompt_recorded(&format!("{{\"method\":\"initialize\"}}\n{prompt}\n")), "the prompt isn't seen");
    ensure!(!prompt_recorded("{\"method\":\"session/new\"}\n"), "a prompt seen in session/new");
    let gave_up = "[2026-11-25T10:00:00Z WARN  brindle] --wait-for: \"End\" didn't appear within 30s";
    ensure!(wait_gave_up(&format!("info\n{gave_up}\n")), "a wait that gave up isn't seen");
    ensure!(!wait_gave_up("[INFO brindle] ready\n"), "a quiet log reads as a wait that gave up");
    Ok(())
}
