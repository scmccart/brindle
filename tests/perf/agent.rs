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

use crate::cases::{spawn_env, spawn_with};
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

/// The trickled run's spacing of the agent's writes (OpenSpec change
/// `agents-absorb-cost`, decision 5): 2,048 chunks take about 20.5 s.
pub const TRICKLE_MS: u64 = 10;

/// `script` with a `pace ms` step right after its gate, so the sends after
/// it trickle.
pub fn paced(script: &str, ms: u64) -> String {
    let mut out = String::with_capacity(script.len() + 16);
    for line in script.lines() {
        out.push_str(line);
        out.push('\n');
        if line.starts_with("wait-file ") {
            out.push_str(&format!("pace {ms}\n"));
        }
    }
    out
}

/// `script` writing a file at `done` once its last send has gone, before
/// the turn's end (`record-env`, the one step that writes a file).
pub fn marking_done(script: &str, done: &Path) -> String {
    let mut lines: Vec<String> = script.lines().map(String::from).collect();
    let end = lines.iter().rposition(|line| line.starts_with("respond ")).unwrap_or(lines.len());
    lines.insert(end, format!("record-env {}", done.display()));
    lines.join("\n") + "\n"
}

/// Brindle's log line (`Agent::log_renders`) when the agent zone was left:
/// its renders and the bytes they were given; the last one when there are
/// several.
pub fn render_counts(log: &str) -> Option<(u64, u64)> {
    let line = log.lines().rev().find_map(|line| line.split_once("agent renders=").map(|(_, rest)| rest))?;
    let mut fields = line.split_whitespace();
    let renders = fields.next()?.parse().ok()?;
    let bytes = fields.next()?.strip_prefix("bytes=")?.parse().ok()?;
    Some((renders, bytes))
}

/// The filter the stream case's Brindle logs with, so the render counts are
/// in its log.
const RENDER_LOG: (&str, &str) = ("RUST_LOG", "brindle=debug,warn");

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
/// `last`. The agent's script is `make(record, gate)`. Its files are
/// `LABEL.*` in the case directory, and its terminal server is stopped
/// before the next run, its logs kept in `LABEL/`. Returns the time from
/// the gate to the exit.
fn run(ctx: &Ctx, agent: &Path, label: &str, last: &str, make: &dyn Fn(&Path, &Path) -> String) -> Result<Duration> {
    run_env(ctx, agent, label, last, make, &[])
}

/// [`run`], with `env` set for Brindle.
fn run_env(
    ctx: &Ctx,
    agent: &Path,
    label: &str,
    last: &str,
    make: &dyn Fn(&Path, &Path) -> String,
    env: &[(&str, &str)],
) -> Result<Duration> {
    let record = ctx.dir.join(format!("{label}.jsonl"));
    let gate = ctx.dir.join(format!("{label}.gate"));
    let script_path = ctx.dir.join(format!("{label}.script"));
    std::fs::write(&script_path, make(&record, &gate)).map_err(|e| format!("writing the script: {e}"))?;
    let interval = STEP_INTERVAL.as_secs_f64().to_string();
    let args =
        ["agent", "Fake", "--no-restore", "-p", "go", "--step-interval", &interval, "--wait-for", last, "--action", "quit"];
    let mut child = spawn_env(ctx, label, &config(agent, &script_path), &args, env)?;

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
    let env = [RENDER_LOG];
    let streamed_time =
        run_env(ctx, &agent.path, "streamed", LAST_LINE, &|record, gate| script(record, gate, &streamed), &env)?;
    let streamed_counts = logged_counts(ctx, "streamed")?;
    let whole_time =
        run_env(ctx, &agent.path, "whole", LAST_LINE, &|record, gate| script(record, gate, &[text.as_str()]), &env)?;
    // The same chunks, about TRICKLE_MS apart, as a real agent writes them
    // (OpenSpec change `agents-absorb-cost`, decision 5).
    let trickled = |record: &Path, gate: &Path| paced(&script(record, gate, &streamed), TRICKLE_MS);
    let trickled_time = run_env(ctx, &agent.path, "trickled", LAST_LINE, &trickled, &env)?;
    let trickled_counts = logged_counts(ctx, "trickled")?;
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    Ok(Measured::Stream(StreamResult {
        bytes: text.len() as u64,
        chunks: streamed.len() as u64,
        streamed_ms: ms(streamed_time),
        whole_ms: ms(whole_time),
        difference_ms: ms(streamed_time) - ms(whole_time),
        small_ms: None,
        step_interval_ms: ms(STEP_INTERVAL),
        trickled_ms: Some(ms(trickled_time)),
        streamed_renders: Some(streamed_counts.0),
        streamed_render_bytes: Some(streamed_counts.1),
        trickled_renders: Some(trickled_counts.0),
        trickled_render_bytes: Some(trickled_counts.1),
    }))
}

/// The render counts run `label`'s Brindle logged as it quit.
fn logged_counts(ctx: &Ctx, label: &str) -> Result<(u64, u64)> {
    let log = std::fs::read_to_string(ctx.dir.join(format!("{label}.log"))).unwrap_or_default();
    render_counts(&log).ok_or_else(|| format!("{label}: no `agent renders=` line; see {label}.log"))
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

    // a_render_count_line_reads_back: the zone's line as env_logger writes
    // it, the last of several kept.
    let logged = "[2026-11-28T10:00:00Z DEBUG brindle::agent] agent renders=12 bytes=3456\n\
                  [2026-11-28T10:00:01Z DEBUG brindle::agent] agent renders=2049 bytes=67108864\n";
    ensure!(render_counts(logged) == Some((2049, 67_108_864)), "read {:?}", render_counts(logged));
    ensure!(render_counts("[INFO brindle] ready\n").is_none(), "a log without the line reads as counts");
    ensure!(render_counts("agent renders=x bytes=1").is_none(), "a bad count reads");

    // The trickled script paces the sends after the gate, and the latency
    // case's marks its last send.
    let base = script(record, &gate, &["a", "b"]);
    let trickled: Vec<String> = paced(&base, TRICKLE_MS).lines().map(String::from).collect();
    let at = trickled.iter().position(|l| l == "wait-file /case/g").ok_or("no gate")?;
    ensure!(trickled[at + 1] == format!("pace {TRICKLE_MS}"), "no pace after the gate: {trickled:?}");
    ensure!(trickled.len() == base.lines().count() + 1, "{trickled:?}");
    let marked: Vec<String> = marking_done(&base, Path::new("/case/done")).lines().map(String::from).collect();
    let n = marked.len();
    ensure!(marked[n - 2] == "record-env /case/done" && marked[n - 3].starts_with("send "), "{marked:?}");
    Ok(())
}

// `agent-attach-long` (OpenSpec change `agents-log-paging`, design.md
// decision 14): how long a launch takes to show a long agent conversation
// it attaches again, which a paged attach bounds by its newest 256 KiB.

/// The conversation's thoughts, each followed by a short agent message.
pub const ATTACH_THOUGHTS: usize = 1_900;
/// Each thought's length, exactly.
pub const THOUGHT_BYTES: usize = 8 * 1024;
/// The conversation's last entry, an agent message, and what the attach's
/// `--wait-for` waits for.
pub const LAST_ENTRY: &str = "End of the long conversation.";
/// What the server keeps of an agent's log before trimming it
/// (`remote-agent/src/acp/reduce.rs`'s `LOG_LIMIT`): the conversation stays
/// under it, encoding included, so the attach has all of it to page.
const LOG_LIMIT: usize = 16 * 1024 * 1024;
/// How long the live run may take to show the whole conversation.
const LIVE_LIMIT: Duration = Duration::from_secs(300);

/// Thought `n`, [`THOUGHT_BYTES`] long.
fn thought(n: usize) -> String {
    let head = format!("Thought {n}: ");
    let mut text = head + &"lorem ipsum ".repeat(THOUGHT_BYTES / 12);
    text.truncate(THOUGHT_BYTES - 1);
    text.push('.');
    text
}

/// The conversation's replies to the first prompt, in order: thought 0,
/// `Message 0`, …, the last message [`LAST_ENTRY`]; each a new entry.
pub fn long_replies() -> Vec<(&'static str, String)> {
    let mut replies = Vec::with_capacity(2 * ATTACH_THOUGHTS);
    for n in 0..ATTACH_THOUGHTS {
        replies.push(("agent_thought_chunk", thought(n)));
        let message = if n + 1 == ATTACH_THOUGHTS { LAST_ENTRY.to_string() } else { format!("Message {n}") };
        replies.push(("agent_message_chunk", message));
    }
    replies
}

/// The fake agent's script for the long conversation: ready at once,
/// recording what it is sent in `record`; on the prompt it sends
/// [`long_replies`] and ends the turn.
pub fn long_script(record: &Path) -> String {
    let mut lines = vec![
        format!("record {}", record.display()),
        "expect initialize".to_string(),
        r#"respond {"protocolVersion":1,"agentInfo":{"name":"Fake Agent","version":"1.0"}}"#.to_string(),
        "expect session/new".to_string(),
        r#"respond {"sessionId":"s1"}"#.to_string(),
        "expect session/prompt".to_string(),
    ];
    lines.extend(long_replies().into_iter().map(|(kind, text)| {
        format!(
            r#"send {{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"s1","update":{{"sessionUpdate":"{kind}","content":{{"type":"text","text":"{}"}}}}}}}}"#,
            json_escape(&text)
        )
    }));
    lines.push(r#"respond {"stopReason":"end_turn"}"#.to_string());
    lines.join("\n") + "\n"
}

/// Waits for Brindle to exit within `limit`, ending it past that; then
/// checks its log for a `--wait-for` that gave up.
fn exited(ctx: &Ctx, mut child: Child, label: &str, limit: Duration) -> Result {
    let started = Instant::now();
    while child.try_wait().map_err(|e| e.to_string())?.is_none() {
        if started.elapsed() > limit {
            end(child);
            return Err(format!("{label}: Brindle didn't exit within {}s; see {label}.log", limit.as_secs()));
        }
        std::thread::sleep(POLL);
    }
    let log = std::fs::read_to_string(ctx.dir.join(format!("{label}.log"))).unwrap_or_default();
    ensure!(!wait_gave_up(&log), "{label}: the last entry never showed; see {label}.log");
    Ok(())
}

/// The live run shows the whole conversation and quits, leaving its
/// session detached and its tab recorded; the timed run restores the tab,
/// which attaches the session, and quits once it shows the last entry.
pub fn attach_long(ctx: &Ctx) -> Result<Measured> {
    let agent = fake_agent::find(&ctx.default_bin);
    if let Err(why) = fake_agent::check(&agent, &ctx.default_bin) {
        return Ok(Measured::Skip(why));
    }
    let (record, script_path) = (ctx.dir.join("long.jsonl"), ctx.dir.join("long.script"));
    std::fs::write(&script_path, long_script(&record)).map_err(|e| format!("writing the script: {e}"))?;
    let config = config(&agent.path, &script_path);
    let interval = STEP_INTERVAL.as_secs_f64().to_string();
    let live = ["agent", "Fake", "--no-restore", "-p", "go", "--step-interval", &interval, "--wait-for", LAST_ENTRY];
    let child = spawn_with(ctx, "live", &config, &[&live[..], &["--action", "quit"]].concat())?;
    exited(ctx, child, "live", LIVE_LIMIT)?;

    let launched = Instant::now();
    let child = spawn_with(ctx, "attach", &config, &["--step-interval", &interval, "--wait-for", LAST_ENTRY, "--action", "quit"])?;
    exited(ctx, child, "attach", EXIT_LIMIT)?;
    let took = launched.elapsed();
    let replies = long_replies();
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    Ok(Measured::Attach(crate::report::AttachResult {
        bytes: replies.iter().map(|(_, text)| text.len() as u64).sum::<u64>() + 2,
        entries: 1 + replies.len() as u64,
        attach_ms: ms(took),
        step_interval_ms: ms(STEP_INTERVAL),
    }))
}

/// The long conversation and its script (`--self-test`).
pub fn self_test_attach() -> Result {
    let replies = long_replies();
    ensure!(replies.len() == 2 * ATTACH_THOUGHTS, "{} replies", replies.len());
    for (n, pair) in replies.chunks(2).enumerate() {
        ensure!(pair[0].0 == "agent_thought_chunk" && pair[0].1.len() == THOUGHT_BYTES, "thought {n}");
        ensure!(pair[0].1.starts_with(&format!("Thought {n}: ")), "thought {n}'s text");
        ensure!(pair[1].0 == "agent_message_chunk", "message {n}");
    }
    ensure!(replies.last().is_some_and(|(_, text)| text == LAST_ENTRY), "the last entry");
    ensure!(replies.iter().filter(|(_, text)| text.contains(LAST_ENTRY)).count() == 1, "the last entry shows twice");
    let bytes: usize = replies.iter().map(|(_, text)| text.len()).sum();
    ensure!(bytes > 14 * 1024 * 1024, "only {bytes} bytes");
    // Each entry encodes with some 200 bytes of JSON around its text.
    ensure!(bytes + replies.len() * 200 < LOG_LIMIT, "{bytes} bytes would be trimmed");

    let lines: Vec<String> = long_script(Path::new("/case/r.jsonl")).lines().map(String::from).collect();
    ensure!(lines[0] == "record /case/r.jsonl", "{:?}", &lines[..1]);
    let at = lines.iter().position(|l| l == "expect session/prompt").ok_or("no prompt step")?;
    let sends = &lines[at + 1..lines.len() - 1];
    ensure!(sends.len() == replies.len(), "{} sends", sends.len());
    let value: serde_json::Value = serde_json::from_str(&sends[0][5..]).map_err(|e| e.to_string())?;
    ensure!(value["params"]["update"]["content"]["text"] == replies[0].1, "the first send's text");
    ensure!(lines.last().is_some_and(|l| l == r#"respond {"stopReason":"end_turn"}"#), "no end of turn");
    Ok(())
}

// `agent-tool-output` (OpenSpec change `agents-tool-content`, design.md
// decision 17, flag 13): how long an agent zone takes to show a command's
// output streamed into its tool call's open row, in Codex's `_meta`
// deltas, against the same output sent in one delta. The row keeps its
// newest 64 KiB, so the check is that a long output costs the view only
// what it keeps and shows.

/// The output's length, exactly.
pub const OUTPUT_BYTES: usize = 1024 * 1024;
/// The streamed run's delta size, the last delta excepted.
pub const DELTA_BYTES: usize = 4 * 1024;
/// The output's last line, and what Brindle's `--wait-for` waits for: the
/// output's `--- item` line holds it only once the last delta is in.
pub const OUTPUT_LAST_LINE: &str = "End of the streamed output.";

/// The output: numbered test lines, a filler line, and
/// [`OUTPUT_LAST_LINE`] with its line break, exactly [`OUTPUT_BYTES`].
/// ASCII, so every byte offset is a char boundary.
pub fn command_output() -> String {
    let tail = format!("{OUTPUT_LAST_LINE}\n");
    let mut out = String::with_capacity(OUTPUT_BYTES);
    for n in 0.. {
        let line = format!("test case {n:06} ... ok\n");
        if out.len() + line.len() + tail.len() > OUTPUT_BYTES {
            break;
        }
        out.push_str(&line);
    }
    // The gap left is shorter than a line: dashes and a line break.
    let gap = OUTPUT_BYTES - out.len() - tail.len();
    if gap > 0 {
        out.push_str(&"-".repeat(gap - 1));
        out.push('\n');
    }
    out.push_str(&tail);
    out
}

/// A `send` step of a `session/update` whose `update` is `update` (JSON).
fn send_update(update: &str) -> String {
    format!(r#"send {{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"s1","update":{update}}}}}"#)
}

/// The fake agent's script: ready at once, recording what it is sent in
/// `record`; on the prompt it starts a command's tool call, its row open
/// while it runs, and waits for `gate`; then it sends each of `pieces` as
/// a `terminal_output_delta`, the command's exit, and ends the turn.
pub fn output_script(record: &Path, gate: &Path, pieces: &[&str]) -> String {
    let mut lines = vec![
        format!("record {}", record.display()),
        "expect initialize".to_string(),
        r#"respond {"protocolVersion":1,"agentInfo":{"name":"Fake Agent","version":"1.0"}}"#.to_string(),
        "expect session/new".to_string(),
        r#"respond {"sessionId":"s1"}"#.to_string(),
        "expect session/prompt".to_string(),
        send_update(
            r#"{"sessionUpdate":"tool_call","toolCallId":"c1","title":"Run tests","kind":"execute","status":"in_progress","content":[{"type":"terminal","terminalId":"t1"}],"_meta":{"terminal_info":{"terminal_id":"t1","cwd":"/w","command":"cargo test"}}}"#,
        ),
        format!("wait-file {}", gate.display()),
    ];
    lines.extend(pieces.iter().map(|piece| {
        send_update(&format!(
            r#"{{"sessionUpdate":"tool_call_update","toolCallId":"c1","_meta":{{"terminal_output_delta":{{"terminal_id":"t1","data":"{}"}}}}}}"#,
            json_escape(piece)
        ))
    }));
    lines.push(send_update(
        r#"{"sessionUpdate":"tool_call_update","toolCallId":"c1","status":"completed","_meta":{"terminal_exit":{"terminal_id":"t1","exit_code":0,"signal":null}}}"#,
    ));
    lines.push(r#"respond {"stopReason":"end_turn"}"#.to_string());
    lines.join("\n") + "\n"
}

pub fn tool_output(ctx: &Ctx) -> Result<Measured> {
    let agent = fake_agent::find(&ctx.default_bin);
    if let Err(why) = fake_agent::check(&agent, &ctx.default_bin) {
        return Ok(Measured::Skip(why));
    }
    let text = command_output();
    let deltas = chunks(&text, DELTA_BYTES);
    let streamed_time =
        run(ctx, &agent.path, "streamed", OUTPUT_LAST_LINE, &|record, gate| output_script(record, gate, &deltas))?;
    let whole_time =
        run(ctx, &agent.path, "whole", OUTPUT_LAST_LINE, &|record, gate| output_script(record, gate, &[text.as_str()]))?;
    // A-4 of the agent review: past the kept 64 KiB, each small delta once
    // moved the whole kept text (OpenSpec change `agents-checkpoint-deltas`).
    let small = chunks(&text, SMALL_DELTA_BYTES);
    let small_time =
        run(ctx, &agent.path, "small", OUTPUT_LAST_LINE, &|record, gate| output_script(record, gate, &small))?;
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    Ok(Measured::Stream(StreamResult {
        bytes: text.len() as u64,
        chunks: deltas.len() as u64,
        streamed_ms: ms(streamed_time),
        whole_ms: ms(whole_time),
        difference_ms: ms(streamed_time) - ms(whole_time),
        small_ms: Some(ms(small_time)),
        step_interval_ms: ms(STEP_INTERVAL),
        ..StreamResult::default()
    }))
}

/// The `small` run's delta size.
pub const SMALL_DELTA_BYTES: usize = 32;

// `agent-tool-heavy` (OpenSpec change `agents-checkpoint-deltas`, design.md
// decision 15): a turn of permission-gated tool calls and one long line,
// the conversation whose checkpoints once grew with every call.

/// The turn's tool calls.
pub const HEAVY_CALLS: usize = 3_000;
/// The long line's length, without its line end: the longest line the
/// server keeps (`acp::lines::MAX_LINE`).
pub const HEAVY_LINE_BYTES: usize = 8 * 1024 * 1024;
/// The pieces the long line is written in.
pub const HEAVY_PIECE_BYTES: usize = 64 * 1024;
/// The turn's last message, and what Brindle's `--wait-for` waits for.
pub const HEAVY_LAST: &str = "End of the tool-heavy turn.";

/// One `session/update` line of `update` (JSON), as the agent writes it.
fn update_line(update: &str) -> String {
    format!(r#"{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"s1","update":{update}}}}}"#)
}

/// The long line: a completed `tool_call_update` of a new call whose diff
/// item's new text fills it to exactly [`HEAVY_LINE_BYTES`]. Compact JSON
/// of `z`s, so no piece starts or ends with a space.
pub fn heavy_line() -> String {
    let around = |fill: &str| {
        update_line(&format!(
            r#"{{"sessionUpdate":"tool_call_update","toolCallId":"toolu_big","status":"completed","content":[{{"type":"diff","path":"/w/big.txt","oldText":null,"newText":"{fill}"}}]}}"#
        ))
    };
    around(&"z".repeat(HEAVY_LINE_BYTES - around("").len()))
}

/// The fake agent's script: ready at once, recording what it is sent in
/// `record`; on the prompt it waits for `gate`, then makes
/// [`HEAVY_CALLS`] tool calls, each a `tool_call`, a
/// `session/request_permission`, a `$/cancel_request` withdrawing it and
/// a completing `tool_call_update`; then writes [`heavy_line`] in pieces
/// of [`HEAVY_PIECE_BYTES`] (`send-part`) and its line end; then sends
/// [`HEAVY_LAST`] and ends the turn.
pub fn tool_heavy_script(record: &Path, gate: &Path) -> String {
    let mut lines = vec![
        format!("record {}", record.display()),
        "expect initialize".to_string(),
        r#"respond {"protocolVersion":1,"agentInfo":{"name":"Fake Agent","version":"1.0"}}"#.to_string(),
        "expect session/new".to_string(),
        r#"respond {"sessionId":"s1"}"#.to_string(),
        "expect session/prompt".to_string(),
        format!("wait-file {}", gate.display()),
    ];
    for n in 0..HEAVY_CALLS {
        let call = format!("toolu_{n:024x}");
        lines.push(send_update(&format!(
            r#"{{"sessionUpdate":"tool_call","toolCallId":"{call}","title":"Run","kind":"execute","status":"pending"}}"#
        )));
        lines.push(format!(
            r#"send {{"jsonrpc":"2.0","id":"p{n}","method":"session/request_permission","params":{{"sessionId":"s1","toolCall":{{"toolCallId":"{call}"}},"options":[{{"optionId":"allow","name":"Allow","kind":"allow_once"}},{{"optionId":"reject","name":"Reject","kind":"reject_once"}}]}}}}"#
        ));
        lines.push(format!(r#"send {{"jsonrpc":"2.0","method":"$/cancel_request","params":{{"requestId":"p{n}"}}}}"#));
        lines.push(send_update(&format!(
            r#"{{"sessionUpdate":"tool_call_update","toolCallId":"{call}","status":"completed","content":[{{"type":"content","content":{{"type":"text","text":"done {n}"}}}}]}}"#
        )));
    }
    let line = heavy_line();
    lines.extend(chunks(&line, HEAVY_PIECE_BYTES).into_iter().map(|piece| format!("send-part {piece}")));
    lines.push("send-raw".to_string());
    lines.push(send_update(&format!(
        r#"{{"sessionUpdate":"agent_message_chunk","content":{{"type":"text","text":"{HEAVY_LAST}"}}}}"#
    )));
    lines.push(r#"respond {"stopReason":"end_turn"}"#.to_string());
    lines.join("\n") + "\n"
}

pub fn tool_heavy(ctx: &Ctx) -> Result<Measured> {
    let agent = fake_agent::find(&ctx.default_bin);
    if let Err(why) = fake_agent::check(&agent, &ctx.default_bin) {
        return Ok(Measured::Skip(why));
    }
    let took = run(ctx, &agent.path, "heavy", HEAVY_LAST, &tool_heavy_script)?;
    let ms = |d: Duration| d.as_secs_f64() * 1000.0;
    Ok(Measured::ToolHeavy(crate::report::ToolHeavyResult {
        calls: HEAVY_CALLS as u64,
        line_bytes: HEAVY_LINE_BYTES as u64,
        turn_ms: ms(took),
        step_interval_ms: ms(STEP_INTERVAL),
    }))
}

/// The tool-heavy script (`--self-test`): its step count, and that its
/// pieces join to one line of the long update.
pub fn self_test_tool_heavy() -> Result {
    let line = heavy_line();
    ensure!(line.len() == HEAVY_LINE_BYTES, "the long line is {} bytes", line.len());
    ensure!(!line.contains('\n'), "the long line holds a line break");
    let value: serde_json::Value = serde_json::from_str(&line).map_err(|e| e.to_string())?;
    ensure!(value["params"]["update"]["toolCallId"] == "toolu_big", "the long line's call");

    let (record, gate) = (Path::new("/case/r.jsonl"), PathBuf::from("/case/g"));
    let script = tool_heavy_script(record, &gate);
    let lines: Vec<&str> = script.lines().collect();
    let pieces = HEAVY_LINE_BYTES / HEAVY_PIECE_BYTES;
    // Seven steps before the calls, four per call, the pieces, the line
    // end, the last message and the turn's end.
    ensure!(lines.len() == 7 + 4 * HEAVY_CALLS + pieces + 3, "{} steps", lines.len());
    ensure!(lines[6] == "wait-file /case/g", "the gate doesn't follow the prompt: {:?}", &lines[..7]);
    let call: Vec<&str> = lines[7..11].to_vec();
    ensure!(call[0].contains(r#""sessionUpdate":"tool_call""#), "{}", call[0]);
    ensure!(call[1].contains(r#""id":"p0","method":"session/request_permission""#), "{}", call[1]);
    ensure!(call[2].contains(r#""$/cancel_request","params":{"requestId":"p0"}"#), "{}", call[2]);
    ensure!(call[3].contains(r#""status":"completed""#), "{}", call[3]);
    let start = 7 + 4 * HEAVY_CALLS;
    let parts: Vec<&str> = lines[start..start + pieces].iter().map(|l| l.strip_prefix("send-part ").unwrap_or("")).collect();
    ensure!(parts.iter().all(|part| part.len() == HEAVY_PIECE_BYTES), "a piece isn't {HEAVY_PIECE_BYTES} bytes");
    ensure!(parts.iter().all(|part| part.trim() == *part), "a piece would lose a space to the parser");
    ensure!(parts.concat() == line, "the pieces don't join to the long line");
    ensure!(lines[start + pieces] == "send-raw", "no line end after the pieces");
    ensure!(lines[start + pieces + 1].contains(HEAVY_LAST), "no last message");
    ensure!(lines.last().is_some_and(|l| *l == r#"respond {"stopReason":"end_turn"}"#), "no end of turn");
    ensure!(script.matches(HEAVY_LAST).count() == 1, "the last message shows twice");
    Ok(())
}

/// The output, its deltas and the script (`--self-test`).
pub fn self_test_tool_output() -> Result {
    let text = command_output();
    ensure!(text.len() == OUTPUT_BYTES, "the output is {} bytes", text.len());
    ensure!(text.is_ascii(), "the output isn't ASCII");
    ensure!(text.ends_with(&format!("\n{OUTPUT_LAST_LINE}\n")), "the last line isn't a line of its own");
    ensure!(text.matches(OUTPUT_LAST_LINE).count() == 1, "the last line shows more than once");
    ensure!(text.starts_with("test case 000000 ... ok\n"), "the first line: {:?}", &text[..24]);

    let deltas = chunks(&text, DELTA_BYTES);
    ensure!(deltas.len() == OUTPUT_BYTES / DELTA_BYTES, "{} deltas", deltas.len());
    ensure!(deltas.concat() == text, "the deltas don't make the output");
    let small = chunks(&text, SMALL_DELTA_BYTES);
    ensure!(small.len() == OUTPUT_BYTES / SMALL_DELTA_BYTES, "{} small deltas", small.len());
    ensure!(small.concat() == text, "the small deltas don't make the output");
    let before_last = &text[..text.len() - deltas[deltas.len() - 1].len()];
    ensure!(!before_last.contains(OUTPUT_LAST_LINE), "the last line is whole before the last delta");

    let (record, gate) = (Path::new("/case/r.jsonl"), PathBuf::from("/case/g"));
    let lines: Vec<String> = output_script(record, &gate, &["a\"\n", "b\n"]).lines().map(String::from).collect();
    ensure!(lines[0] == "record /case/r.jsonl", "{lines:?}");
    let at = lines.iter().position(|l| l == "expect session/prompt").ok_or("no prompt step")?;
    // The call starts before the gate, so the timed run streams into an
    // open row.
    ensure!(lines[at + 1].contains(r#""sessionUpdate":"tool_call""#), "no call before the gate: {lines:?}");
    ensure!(lines[at + 2] == "wait-file /case/g", "the gate doesn't follow the call: {lines:?}");
    let sends = &lines[at + 3..lines.len() - 1];
    ensure!(sends.len() == 3, "{} sends after the gate", sends.len());
    let value: serde_json::Value = serde_json::from_str(&sends[0][5..]).map_err(|e| e.to_string())?;
    let delta = &value["params"]["update"]["_meta"]["terminal_output_delta"];
    ensure!(delta["terminal_id"] == "t1" && delta["data"] == "a\"\n", "the first delta: {}", sends[0]);
    ensure!(sends[2].contains(r#""terminal_exit""#) && sends[2].contains(r#""status":"completed""#), "no exit: {}", sends[2]);
    ensure!(lines.last().is_some_and(|l| l == r#"respond {"stopReason":"end_turn"}"#), "no end of turn: {lines:?}");
    Ok(())
}

// `agent-latency-quiet` and `agent-latency-trickle` (OpenSpec change
// `agents-absorb-cost`, design.md decision 9): a shell's key echo beside an
// agent zone that is idle, and beside one whose long reply trickles in
// while the bench types. One reader thread serves both zones' sessions, so
// this is what holding the fold's lock across a render costs a shell.

/// The quiet case's reply, and what its `--wait-for` waits for before the
/// bench.
pub const QUIET_REPLY: &str = "A short reply, done.";

/// What the agent zone shows once its session takes prompts.
const ZONE_READY: &str = "--- prompt status=ready";

/// The latency cases' Brindle arguments: a `Shell` tab, the agent split
/// beside it, `go` sent once its prompt takes it, `wait` (if any) waited
/// for in the agent zone, then focus back in the shell and the bench.
pub fn latency_args(log: &str, samples: &str, wait: Option<&str>) -> Vec<String> {
    let mut args: Vec<String> = ["terminal", "Shell", "--no-restore", "--action", "split_agent_right", "--wait-for", ZONE_READY]
        .map(String::from)
        .to_vec();
    args.extend(["--send", "go", "--key", "enter"].map(String::from));
    if let Some(text) = wait {
        args.extend(["--wait-for".to_string(), text.to_string()]);
    }
    args.extend(["--action", "focus_zone_left", "--latency-log", log, "--latency-bench", samples].map(String::from));
    args
}

/// One latency run beside an agent zone whose agent plays `script` (its
/// gate already open); `done` is the file the agent writes once its last
/// chunk is sent, when it marks one.
fn agent_latency(ctx: &Ctx, script_text: &str, wait: Option<&str>, done: Option<&Path>) -> Result<Measured> {
    let agent = fake_agent::find(&ctx.default_bin);
    if let Err(why) = fake_agent::check(&agent, &ctx.default_bin) {
        return Ok(Measured::Skip(why));
    }
    let script_path = ctx.dir.join("agent.script");
    std::fs::write(&script_path, script_text).map_err(|e| format!("writing the script: {e}"))?;
    let log = ctx.dir.join("latency.jsonl");
    let samples = ctx.samples.to_string();
    let args = latency_args(log.to_str().ok_or("a non-UTF-8 path")?, &samples, wait);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let child = spawn_with(ctx, "brindle", &config(&agent.path, &script_path), &args)?;
    // The steps, then each keystroke at most 1 s for its echo and a gap.
    let limit = Duration::from_secs(120) + Duration::from_millis(1050) * (ctx.samples as u32 + 10);
    let stdout = crate::cases::wait(child, "brindle", limit)?;
    // Read as Brindle exits, the bench's end: still missing, the stream
    // outlasted it.
    let outlasted = done.map(|done| !done.exists());
    ctx.save("brindle.out", &stdout);
    let mut result = crate::report::parse_latency(&stdout).map_err(|e| format!("{e}; see brindle.log"))?;
    ensure!(result.samples == ctx.samples as u64, "{} samples, expected {}", result.samples, ctx.samples);
    if outlasted == Some(false) {
        println!("         the reply ended before the bench did: part of it ran beside an idle zone");
    }
    result.stream_outlasted = outlasted;
    Ok(Measured::Latency(result))
}

pub fn latency_quiet(ctx: &Ctx) -> Result<Measured> {
    let (record, gate) = (ctx.dir.join("agent.jsonl"), ctx.dir.join("agent.gate"));
    std::fs::write(&gate, b"").map_err(|e| format!("opening the gate: {e}"))?;
    agent_latency(ctx, &script(&record, &gate, &[QUIET_REPLY]), Some(QUIET_REPLY), None)
}

pub fn latency_trickle(ctx: &Ctx) -> Result<Measured> {
    let (record, gate, done) = (ctx.dir.join("agent.jsonl"), ctx.dir.join("agent.gate"), ctx.dir.join("stream.done"));
    std::fs::write(&gate, b"").map_err(|e| format!("opening the gate: {e}"))?;
    let text = answer();
    let pieces = chunks(&text, CHUNK_BYTES);
    let script_text = marking_done(&paced(&script(&record, &gate, &pieces), TRICKLE_MS), &done);
    agent_latency(ctx, &script_text, None, Some(&done))
}

/// The latency cases' arguments (`--self-test`).
pub fn self_test_latency() -> Result {
    let args = latency_args("/case/l.jsonl", "500", Some(QUIET_REPLY));
    let at = |word: &str| args.iter().position(|a| a == word).ok_or(format!("no {word}: {args:?}"));
    ensure!(args[..3] == ["terminal", "Shell", "--no-restore"], "{args:?}");
    ensure!(at("split_agent_right")? < at(ZONE_READY)?, "the wait for the zone comes after the split: {args:?}");
    ensure!(at(ZONE_READY)? < at("go")? && at("go")? < at("enter")?, "the prompt is sent once the zone is ready");
    ensure!(at("enter")? < at(QUIET_REPLY)? && at(QUIET_REPLY)? < at("focus_zone_left")?, "{args:?}");
    ensure!(args.ends_with(&["--latency-log", "/case/l.jsonl", "--latency-bench", "500"].map(String::from)), "{args:?}");
    let trickle = latency_args("/l", "5", None);
    ensure!(!trickle.contains(&QUIET_REPLY.to_string()) && trickle.len() == args.len() - 2, "{trickle:?}");
    Ok(())
}
