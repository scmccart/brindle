//! A prompt's attachments (OpenSpec change `agents-prompt-context`): a
//! terminal's last command's output sent to an agent zone, a file attached
//! by its path, their bounds, the queue and a restore. The agent zones run
//! the fake ACP agent (`Case::needs_fake_agent`), which records the
//! `session/prompt` parts it was sent; the kept texts are read from the
//! case's cache directory (`common::server::cache_dir`).

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::agent::{READY, Script, answered, gate, says, stop, waited};
use crate::brindle::{Output, Run};
use crate::{Ctx, Outcome, Result, ensure};

/// An agent that takes embedded context, as every captured agent does.
pub(super) const EMBEDS: &str = r#"{"promptCapabilities":{"embeddedContext":true}}"#;

/// An agent named `name` that records what it is sent in `received`,
/// takes embedded context and is ready at once.
pub(super) fn ready(received: &Path, name: &str) -> Script {
    Script(vec![
        format!("record {}", received.display()),
        "expect initialize".into(),
        format!(
            r#"respond {{"protocolVersion":1,"agentInfo":{{"name":"{name}","version":"1.0"}},"agentCapabilities":{EMBEDS}}}"#
        ),
        "expect session/new".into(),
        r#"respond {"sessionId":"s1"}"#.into(),
    ])
}

/// `script`'s agent answering each of `turns` prompts with `Hello!`
/// ([`answered`]).
pub(super) fn answering(script: Script, turns: usize) -> Script {
    (0..turns).fold(script, |script, _| script.step("expect session/prompt").step(says("Hello!")).step(stop("end_turn")))
}

/// An agent profile `name` running `script` from `LABEL.script`, as TOML
/// for the test config.
pub(super) fn profile(ctx: &Ctx, name: &str, label: &str, script: Script) -> Result<String> {
    let path = script.write(ctx, label)?;
    Ok(format!(
        "\n[[profiles]]\nname = \"{name}\"\ntype = \"agent\"\ncommand = '{}'\nargs = ['{}']\n",
        ctx.fake_agent.display(),
        path.display()
    ))
}

/// The Shell profile's bash with Brindle's integration loaded and a plain
/// prompt, so a command's output is marked.
fn integrated(ctx: &Ctx) -> String {
    format!("PS1='$ '; eval \"$({} --shell-integration bash)\"; clear\r", ctx.bin.display())
}

/// A new Shell tab (tab `tab` once open) with integration, running
/// `command` until its output shows `shown`; then Send Last Command Output
/// to Agent.
pub(super) fn send_output<'a>(ctx: &Ctx, run: Run<'a>, first: bool, tab: usize, command: &str, shown: &str) -> Run<'a> {
    let run = if first { run.action("new_tab").send(&integrated(ctx)) } else { run.action(&format!("activate_tab_{tab}")) };
    run.send(&format!("{command}\r")).wait_for(shown).action("send_last_output_to_agent")
}

/// Attach File… with `path` typed into its prompt.
pub(super) fn attach_file<'a>(run: Run<'a>, path: &str) -> Run<'a> {
    run.action("agent_attach_file").send(path).action("picker_confirm")
}

/// The parts of each `session/prompt` the agent recording in `received`
/// was sent, in order.
pub(super) fn prompt_parts(received: &Path) -> Result<Vec<Vec<Value>>> {
    let text = std::fs::read_to_string(received).map_err(|e| format!("reading {}: {e}", received.display()))?;
    let mut prompts = Vec::new();
    for line in text.lines() {
        let value: Value = serde_json::from_str(line).map_err(|e| format!("{line}: {e}"))?;
        if value["method"] == "session/prompt" {
            prompts.push(value["params"]["prompt"].as_array().cloned().unwrap_or_default());
        }
    }
    Ok(prompts)
}

/// The `cwd` of the first `session/new` the agent was sent.
fn session_cwd(received: &Path) -> Result<String> {
    let text = std::fs::read_to_string(received).map_err(|e| format!("reading {}: {e}", received.display()))?;
    for line in text.lines() {
        let value: Value = serde_json::from_str(line).map_err(|e| format!("{line}: {e}"))?;
        if value["method"] == "session/new" {
            return value["params"]["cwd"].as_str().map(str::to_string).ok_or_else(|| format!("no cwd: {line}"));
        }
    }
    Err(format!("no session/new in {}", received.display()))
}

/// The path a `file://` URI names (the cases' paths need no decoding).
fn uri_path(uri: &str) -> Result<PathBuf> {
    uri.strip_prefix("file://").map(PathBuf::from).ok_or_else(|| format!("not a file URI: {uri}"))
}

/// Every kept text file under `root` (each store's files but its
/// `expires` marker), sorted.
fn kept_files(root: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(stores) = std::fs::read_dir(root) else { return files };
    for store in stores.flatten() {
        let Ok(entries) = std::fs::read_dir(store.path()) else { continue };
        files.extend(entries.flatten().map(|e| e.path()).filter(|p| p.file_name().is_some_and(|n| n != "expires")));
    }
    files.sort();
    files
}

/// The case's default attachments directory.
fn attachments_root(ctx: &Ctx) -> PathBuf {
    crate::common::server::cache_dir(&ctx.server_dir).join("attachments")
}

/// The texts kept under `root`, sorted.
fn kept_texts(root: &Path) -> Vec<String> {
    let mut texts: Vec<String> = kept_files(root).iter().filter_map(|p| std::fs::read_to_string(p).ok()).collect();
    texts.sort();
    texts
}

/// A prompt part of `kind` (`text`, `resource`, `resource_link`).
pub(super) fn parts_of<'a>(parts: &'a [Value], kind: &str) -> Vec<&'a Value> {
    parts.iter().filter(|part| part["type"] == kind).collect()
}

/// The dump's focus is the agent zone's prompt input.
fn prompt_focused(out: &Output, what: &str) -> Result {
    ensure!(
        out.dump.focus.as_deref() == Some("agent") && out.dump.focus_stop.as_deref() == Some("prompt"),
        "{what}: focus {:?} stop {:?}",
        out.dump.focus,
        out.dump.focus_stop
    );
    Ok(())
}

/// The active tab's place, from 1.
fn active_tab(out: &Output) -> Option<usize> {
    out.dump.tabs.iter().position(|(_, active)| *active).map(|i| i + 1)
}

/// The output's draft line for `echo out-$((40+2))`'s output, `out-42`.
const OUT_42: &str = r#"index=0 kind=text name="echo out-$((40+2))" bytes=6 "out-42""#;

/// The spec's "Send a command's output": in a shell tab, Send Last Command
/// Output to Agent makes the output a draft in the only agent tab, named
/// by its command line, shows that tab with its prompt focused and sends
/// nothing; sent, the user message shows its attachment's reference and
/// the agent receives the text embedded.
pub fn output(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("attach.jsonl");
    let config = profile(ctx, "Fake", "attach", answering(ready(&received, "Fake Agent"), 1))?;
    let run = Run::new(ctx, "attach").config(&config).agent("Fake").wait_for(READY);
    let out = send_output(ctx, run, true, 2, "echo out-$((40+2))", "out-42").wait_for("draft index=0").dump_at(10.0).run()?;
    waited(&out)?;
    ensure!(out.dump.agent_draft() == [OUT_42], "attach: draft {:?}", out.dump.agent);
    ensure!(active_tab(&out) == Some(1), "attach: tabs {:?}", out.dump.tabs);
    prompt_focused(&out, "attach")?;
    ensure!(prompt_parts(&received)?.is_empty(), "attach: a prompt went: {:?}", prompt_parts(&received)?);

    let received = ctx.dir.join("send.jsonl");
    let config = profile(ctx, "Fake", "send", answering(ready(&received, "Fake Agent"), 1))?;
    let run = Run::new(ctx, "send").config(&config).agent("Fake").wait_for(READY);
    let out = send_output(ctx, run, true, 2, "echo out-$((40+2))", "out-42")
        .wait_for("draft index=0")
        .send("what is this?")
        .key("enter")
        .wait_for(&answered())
        .dump_at(14.0)
        .run()?;
    waited(&out)?;
    ensure!(out.dump.agent_entry(1) == Some("user attachments=1 \"what is this?\""), "send: entries {:?}", out.dump.agent);
    let attached = out.dump.agent_attached(1);
    ensure!(
        attached.len() == 1
            && attached[0].starts_with("index=0 kind=text id=")
            && attached[0].ends_with(r#"name="echo out-$((40+2))" bytes=6 state=closed"#),
        "send: attached {attached:?}"
    );
    ensure!(out.dump.agent_draft().is_empty(), "send: the draft stayed {:?}", out.dump.agent);
    let prompts = prompt_parts(&received)?;
    let [parts] = prompts.as_slice() else { return Err(format!("send: prompts {prompts:?}")) };
    ensure!(parts_of(parts, "text").first().is_some_and(|t| t["text"] == "what is this?"), "send: parts {parts:?}");
    let embedded = parts_of(parts, "resource");
    let [resource] = embedded.as_slice() else { return Err(format!("send: parts {parts:?}")) };
    ensure!(resource["resource"]["text"] == "out-42", "send: embedded {resource}");
    let kept = uri_path(resource["resource"]["uri"].as_str().unwrap_or_default())?;
    ensure!(std::fs::read_to_string(&kept).ok().as_deref() == Some("out-42"), "send: kept {}", kept.display());
    Ok(Outcome::Pass)
}

/// The spec's "Several agent zones" and "No agent zone": with two agent
/// tabs the output asks which, the most recently focused first; the
/// second row chosen takes it, and its tab is shown. With none, the
/// notice line says so and the terminal keeps focus.
pub fn choose(ctx: &Ctx) -> Result<Outcome> {
    let alpha = profile(ctx, "Alpha", "alpha", ready(&ctx.dir.join("alpha.jsonl"), "Alpha Agent"))?;
    let beta = profile(ctx, "Beta", "beta", ready(&ctx.dir.join("beta.jsonl"), "Beta Agent"))?;
    let config = alpha + &beta;
    // Alpha's tab is 1, Beta's (profile 3) 2 and the shell's 3.
    let opened = |label| {
        Run::new(ctx, label)
            .config(&config)
            .agent("Alpha")
            .wait_for(READY)
            .action("new_tab_with_profile_3")
            .wait_for(READY)
    };
    let out = send_output(ctx, opened("list"), true, 3, "echo out-$((40+2))", "out-42").dump_at(12.0).run()?;
    waited(&out)?;
    let rows: Vec<&String> = out.dump.palette.iter().skip(1).collect();
    ensure!(
        rows.len() == 2 && rows[0].contains("Beta") && rows[1].contains("Alpha"),
        "list: palette {:?}",
        out.dump.palette
    );
    ensure!(active_tab(&out) == Some(3), "list: tabs {:?}", out.dump.tabs);

    let out = send_output(ctx, opened("choose"), true, 3, "echo out-$((40+2))", "out-42")
        .key("down")
        .key("enter")
        .wait_for("draft index=0")
        .dump_at(14.0)
        .run()?;
    waited(&out)?;
    ensure!(active_tab(&out) == Some(1), "choose: tabs {:?}", out.dump.tabs);
    ensure!(out.dump.agent_draft() == [OUT_42], "choose: draft {:?}", out.dump.agent);
    prompt_focused(&out, "choose")?;

    let run = Run::new(ctx, "none").profile("Shell").send(&integrated(ctx));
    let out = run.send("echo out-$((40+2))\r").wait_for("out-42").action("send_last_output_to_agent").dump_at(6.0).run()?;
    waited(&out)?;
    ensure!(out.dump.config.as_deref() == Some("no agent zone is open"), "none: config line {:?}", out.dump.config);
    ensure!(out.dump.focus.as_deref() == Some("terminal"), "none: focus {:?}", out.dump.focus);
    Ok(Outcome::Pass)
}

/// The notice line when the last command's output isn't in the scrollback
/// (`terminal::NoOutput::NotInScrollback`).
const NOT_IN_SCROLLBACK: &str =
    "no command output to send: the last command's output has left the scrollback, or it printed nothing";

/// The notice line when the shell sends no prompt marks
/// (`terminal::NoOutput::NoPromptMarks`).
const NO_PROMPT_MARKS: &str = "no command output to send: the shell sends no prompt marks (no shell integration)";

/// The spec's "Output that has scrolled away" and "A shell without prompt
/// marks": with no output to send, the notice line says why, the terminal
/// keeps focus and the agent zone gets nothing. 200 lines in one write
/// under 50 lines of scrollback leave it at once, as `1..20000` did under
/// 10,000 in task 1.1's captures, which forgets the command's marks.
pub fn nothing(ctx: &Ctx) -> Result<Outcome> {
    let config = profile(ctx, "Fake", "scrolled", ready(&ctx.dir.join("scrolled.jsonl"), "Fake Agent"))?;
    let run = Run::new(ctx, "scrolled").top("scrollback_lines = 50").config(&config).agent("Fake").wait_for(READY);
    let out = send_output(ctx, run, true, 2, "printf '%s\\n' $(seq 1 200) seq-$((1+1))", "seq-2").dump_at(10.0).run()?;
    waited(&out)?;
    ensure!(out.dump.config.as_deref() == Some(NOT_IN_SCROLLBACK), "scrolled: config line {:?}", out.dump.config);
    ensure!(
        active_tab(&out) == Some(2) && out.dump.focus.as_deref() == Some("terminal"),
        "scrolled: tabs {:?} focus {:?}",
        out.dump.tabs,
        out.dump.focus
    );

    // A shell without integration, the action run as a bound key would run
    // it; the agent's tab, shown afterwards, has no draft.
    let config = profile(ctx, "Fake", "unmarked", ready(&ctx.dir.join("unmarked.jsonl"), "Fake Agent"))?;
    let run = Run::new(ctx, "unmarked").config(&config).agent("Fake").wait_for(READY);
    let out = run
        .action("new_tab")
        .send("echo out-$((40+2))\r")
        .wait_for("out-42")
        .action("send_last_output_to_agent")
        .action("activate_tab_1")
        .dump_at(8.0)
        .run()?;
    waited(&out)?;
    ensure!(out.dump.config.as_deref() == Some(NO_PROMPT_MARKS), "unmarked: config line {:?}", out.dump.config);
    ensure!(
        active_tab(&out) == Some(1) && out.dump.agent_draft().is_empty(),
        "unmarked: tabs {:?} agent {:?}",
        out.dump.tabs,
        out.dump.agent
    );
    Ok(Outcome::Pass)
}

/// The spec's "Attach a file": Attach File… with a relative path drafts a
/// link, which the agent receives resolved against its directory; escape
/// attaches nothing; backspace at the start of the text removes the last
/// item and Clear Attachments every one.
pub fn file(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("file.jsonl");
    let config = profile(ctx, "Fake", "file", answering(ready(&received, "Fake Agent"), 1))?;
    let run = Run::new(ctx, "file").config(&config).agent("Fake").wait_for(READY);
    let run = attach_file(run, "a.rs").wait_for(r#"draft index=0 kind=link path="a.rs" name="a.rs""#);
    let run = attach_file(run, "b.rs").wait_for(r#"draft index=1 kind=link path="b.rs""#);
    // Backspace takes b.rs, so c.rs is the second.
    let run = attach_file(run.key("backspace"), "c.rs").wait_for(r#"draft index=1 kind=link path="c.rs""#);
    // Clear takes both, so d.rs is the first.
    let run = attach_file(run.action("agent_clear_attachments"), "sub/d.rs")
        .wait_for(r#"draft index=0 kind=link path="sub/d.rs" name="d.rs""#);
    let out = run
        .action("agent_attach_file")
        .send("nope.rs")
        .key("escape")
        .send("read it")
        .key("enter")
        .wait_for(&answered())
        .dump_at(26.0)
        .run()?;
    waited(&out)?;
    ensure!(out.dump.agent_entry(1) == Some("user attachments=1 \"read it\""), "entries {:?}", out.dump.agent);
    ensure!(
        out.dump.agent_attached(1) == [r#"index=0 kind=link path="sub/d.rs" name="d.rs""#],
        "attached {:?}",
        out.dump.agent
    );
    let prompts = prompt_parts(&received)?;
    let [parts] = prompts.as_slice() else { return Err(format!("prompts {prompts:?}")) };
    let links = parts_of(parts, "resource_link");
    let [link] = links.as_slice() else { return Err(format!("parts {parts:?}")) };
    let cwd = session_cwd(&received)?;
    let expected = format!("file://{}/sub/d.rs", cwd.trim_end_matches('/'));
    ensure!(link["uri"] == expected.as_str() && link["name"] == "d.rs", "link {link}, expected {expected}");
    Ok(Outcome::Pass)
}

/// A command printing `lines` lines of 59 zeros, each 60 bytes with its
/// newline, then [`ZEROS_DONE`], which its own line doesn't show.
fn zeros(lines: usize) -> String {
    format!("yes $(printf '%059d' 0) | head -n {lines}; printf '%s-%s\\012' gen done")
}

/// What [`zeros`] prints last.
const ZEROS_DONE: &str = "gen-done";

/// The spec's bounds: a ninth attachment is refused on the notice line;
/// an output over 256 KiB is refused with its size and the limit, the
/// terminal keeping focus; one between 64 KiB and 256 KiB is sent whole as
/// a link to its kept file; enter with no text sends nothing, attachments
/// or not.
pub fn bounds(ctx: &Ctx) -> Result<Outcome> {
    let config = profile(ctx, "Fake", "ninth", ready(&ctx.dir.join("ninth.jsonl"), "Fake Agent"))?;
    let run = Run::new(ctx, "ninth").config(&config).agent("Fake").step_interval(0.5).wait_for(READY);
    let run = (1..=9).fold(run, |run, n| attach_file(run, &format!("f{n}.rs")));
    let out = run.dump_at(24.0).run()?;
    waited(&out)?;
    ensure!(out.dump.agent_draft().len() == 8, "ninth: draft {:?}", out.dump.agent);
    ensure!(
        out.dump.config.as_deref() == Some("a prompt takes at most 8 attachments"),
        "ninth: config line {:?}",
        out.dump.config
    );

    let config = profile(ctx, "Fake", "long", ready(&ctx.dir.join("long.jsonl"), "Fake Agent"))?;
    let run = Run::new(ctx, "long").config(&config).agent("Fake").wait_for(READY);
    let out = send_output(ctx, run, true, 2, &zeros(5000), ZEROS_DONE).dump_at(10.0).run()?;
    waited(&out)?;
    let line = out.dump.config.as_deref().unwrap_or_default();
    ensure!(
        line.starts_with("the output is ") && line.ends_with(" KiB, and an attachment takes at most 256 KiB"),
        "long: config line {:?}",
        out.dump.config
    );
    ensure!(active_tab(&out) == Some(2) && out.dump.focus.as_deref() == Some("terminal"), "long: focus {:?}", out.dump.focus);

    let received = ctx.dir.join("kept.jsonl");
    let config = profile(ctx, "Fake", "kept", answering(ready(&received, "Fake Agent"), 1))?;
    let run = Run::new(ctx, "kept").config(&config).agent("Fake").wait_for(READY);
    let out = send_output(ctx, run, true, 2, &zeros(2000), ZEROS_DONE)
        .wait_for("draft index=0")
        .key("enter")
        .send("look")
        .key("enter")
        .wait_for(&answered())
        .dump_at(16.0)
        .run()?;
    waited(&out)?;
    let prompts = prompt_parts(&received)?;
    let [parts] = prompts.as_slice() else { return Err(format!("kept: prompts {prompts:?}")) };
    ensure!(parts_of(parts, "resource").is_empty(), "kept: embedded {parts:?}");
    let links = parts_of(parts, "resource_link");
    let [link] = links.as_slice() else { return Err(format!("kept: parts {parts:?}")) };
    let kept = std::fs::read_to_string(uri_path(link["uri"].as_str().unwrap_or_default())?)
        .map_err(|e| format!("kept: reading {link}: {e}"))?;
    let line = format!("{:059}", 0);
    ensure!(kept.lines().filter(|l| *l == line).count() == 2000, "kept: {} bytes, not 2000 lines", kept.len());
    let attached = out.dump.agent_attached(1);
    ensure!(
        attached.len() == 1 && attached[0].contains(&format!(" bytes={} ", kept.len())),
        "kept: attached {attached:?} for {} bytes",
        kept.len()
    );
    Ok(Outcome::Pass)
}

/// The spec's queue with attachments: a prompt with one sent during a turn
/// waits with its count, and the agent receives both when the turn ends; a
/// held one sent again keeps it; a discarded one leaves no kept text.
pub fn queue(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("queue.jsonl");
    let g1 = gate(ctx, "queue-1");
    let script = ready(&received, "Fake Agent")
        .step("expect session/prompt")
        .step(format!("wait-file {}", g1.display()))
        .step(stop("end_turn"));
    let config = profile(ctx, "Fake", "queue", answering(script, 1))?;
    let run = Run::new(ctx, "queue").config(&config).agent("Fake").first_prompt("one").wait_for("turn=working");
    let out = send_output(ctx, run, true, 2, "echo q-$((1+1))", "q-2")
        .send("two")
        .key("enter")
        .wait_for("queued waiting attachments=1 \"two\"")
        .action("activate_tab_2")
        .send(&format!("touch '{}'\r", g1.display()))
        .action("activate_tab_1")
        .wait_for(&answered())
        .dump_at(18.0)
        .run()?;
    waited(&out)?;
    let prompts = prompt_parts(&received)?;
    ensure!(prompts.len() == 2, "queue: prompts {prompts:?}");
    let embedded = parts_of(&prompts[1], "resource");
    ensure!(embedded.len() == 1 && embedded[0]["resource"]["text"] == "q-2", "queue: second prompt {:?}", prompts[1]);

    let received = ctx.dir.join("held.jsonl");
    let g1 = gate(ctx, "held-1");
    let script = ready(&received, "Fake Agent")
        .step("expect session/prompt")
        .step(format!("wait-file {}", g1.display()))
        .step(stop("max_tokens"));
    let config = profile(ctx, "Fake", "held", answering(script, 1))?;
    let run = Run::new(ctx, "held").config(&config).agent("Fake").first_prompt("one").wait_for("turn=working");
    let out = send_output(ctx, run, true, 2, "echo h-$((1+1))", "h-2")
        .send("two")
        .key("enter")
        .wait_for("queued waiting attachments=1 \"two\"")
        .action("activate_tab_2")
        .send(&format!("touch '{}'\r", g1.display()))
        .action("activate_tab_1")
        .wait_for("queued held reason=stopped:max_tokens resend=yes attachments=1 \"two\"")
        .action("agent_resend")
        .wait_for(&answered())
        .dump_at(20.0)
        .run()?;
    waited(&out)?;
    let prompts = prompt_parts(&received)?;
    ensure!(prompts.len() == 2, "held: prompts {prompts:?}");
    let embedded = parts_of(&prompts[1], "resource");
    ensure!(embedded.len() == 1 && embedded[0]["resource"]["text"] == "h-2", "held: sent again {:?}", prompts[1]);

    let received = ctx.dir.join("discard.jsonl");
    let script = ready(&received, "Fake Agent")
        .step("expect session/prompt")
        .step(format!("wait-file {}", gate(ctx, "never").display()));
    let config = profile(ctx, "Fake", "discard", script)?;
    let run = Run::new(ctx, "discard").config(&config).agent("Fake").first_prompt("one").wait_for("turn=working");
    let out = send_output(ctx, run, true, 2, "echo d-$((1+1))", "d-2")
        .send("two")
        .key("enter")
        .wait_for("queued waiting attachments=1 \"two\"")
        .action("agent_unqueue")
        .dump_at(14.0)
        .run()?;
    waited(&out)?;
    ensure!(!out.dump.agent.iter().any(|l| l.starts_with("queued")), "discard: queued {:?}", out.dump.agent);
    let root = attachments_root(ctx);
    ensure!(!kept_texts(&root).iter().any(|t| t == "d-2"), "discard: kept {:?}", kept_files(&root));
    Ok(Outcome::Pass)
}

/// The spec's restore: quit with a sent prompt's attachment in the
/// conversation and a waiting prompt's; the relaunched zone shows both,
/// and the session's attachment directory still holds their texts.
pub fn restore(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("restore.jsonl");
    let script = ready(&received, "Fake Agent")
        .step("expect session/prompt")
        .step(format!("wait-file {}", gate(ctx, "never").display()));
    let config = profile(ctx, "Fake", "restore", script)?;
    let waiting = "queued waiting attachments=1 \"two\"";
    let run = Run::new(ctx, "live").config(&config).agent("Fake").keep_detached().wait_for(READY);
    let run = send_output(ctx, run, true, 2, "echo a-$((0+1))", "a-1").send("one").key("enter").wait_for("turn=working");
    let out = send_output(ctx, run, false, 2, "echo b-$((1+1))", "b-2")
        .send("two")
        .key("enter")
        .wait_for(waiting)
        .dump_at(18.0)
        .run()?;
    waited(&out)?;
    let out = Run::new(ctx, "restored").config(&config).restore().wait_for(waiting).dump_at(6.0).run()?;
    waited(&out)?;
    ensure!(out.dump.agent_entry(1) == Some("user attachments=1 \"one\""), "restored: entries {:?}", out.dump.agent);
    let attached = out.dump.agent_attached(1);
    ensure!(
        attached.len() == 1 && attached[0].ends_with(r#"name="echo a-$((0+1))" bytes=3 state=closed"#),
        "restored: attached {attached:?}"
    );
    let root = attachments_root(ctx);
    ensure!(kept_texts(&root) == ["a-1", "b-2"], "restored: kept {:?}", kept_files(&root));
    ensure!(prompt_parts(&received)?.len() == 1, "restored: prompts {:?}", prompt_parts(&received)?);
    Ok(Outcome::Pass)
}

/// The spec's attachment settings: `[agent] attachments_dir` is where a
/// local agent's text is kept; `attachments_keep_days = 0` leaves no
/// directory once the zone is closed; a keep time out of range shows the
/// config line.
pub fn settings(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("dir.jsonl");
    let dir = ctx.dir.join("kept");
    let config = format!("\n[agent]\nattachments_dir = '{}'\n", dir.display())
        + &profile(ctx, "Fake", "dir", answering(ready(&received, "Fake Agent"), 1))?;
    let run = Run::new(ctx, "dir").config(&config).agent("Fake").wait_for(READY);
    let out = send_output(ctx, run, true, 2, "echo k-$((1+1))", "k-2")
        .send("look")
        .key("enter")
        .wait_for(&answered())
        .dump_at(12.0)
        .run()?;
    waited(&out)?;
    ensure!(kept_texts(&dir) == ["k-2"], "dir: kept {:?} in {}", kept_files(&dir), dir.display());
    let prompts = prompt_parts(&received)?;
    let uri = prompts.first().and_then(|parts| parts_of(parts, "resource").first().map(|r| r["resource"]["uri"].clone()));
    let uri = uri.and_then(|uri| uri.as_str().map(str::to_string)).unwrap_or_default();
    ensure!(uri_path(&uri)?.starts_with(&dir), "dir: the agent was given {uri}");

    let received = ctx.dir.join("zero.jsonl");
    let config = "\n[agent]\nattachments_keep_days = 0\n".to_string()
        + &profile(ctx, "Fake", "zero", answering(ready(&received, "Fake Agent"), 1))?;
    let run = Run::new(ctx, "zero").config(&config).agent("Fake").wait_for(READY);
    let out = send_output(ctx, run, true, 2, "echo z-$((1+1))", "z-2")
        .send("look")
        .key("enter")
        .wait_for(&answered())
        .action("close_tab")
        .wait_for("$ ")
        .dump_at(14.0)
        .run()?;
    waited(&out)?;
    ensure!(out.dump.tabs.len() == 1, "zero: tabs {:?}", out.dump.tabs);
    let root = attachments_root(ctx);
    ctx.scale().eventually(5.0, "the store removed", || kept_files(&root).is_empty() && std::fs::read_dir(&root).map_or(true, |mut d| d.next().is_none()))?;

    let config = "\n[agent]\nattachments_keep_days = -3\n".to_string()
        + &profile(ctx, "Fake", "range", ready(&ctx.dir.join("range.jsonl"), "Fake Agent"))?;
    let out = Run::new(ctx, "range").config(&config).agent("Fake").wait_for(READY).dump_at(4.0).run()?;
    waited(&out)?;
    let line = out.dump.config.as_deref().unwrap_or_default();
    ensure!(line.contains("agent.attachments_keep_days"), "range: config line {:?}", out.dump.config);
    Ok(Outcome::Pass)
}

/// The dump parser's attachment lines (`--self-test`).
pub fn self_test() -> Result {
    let dump = crate::brindle::Dump::parse(concat!(
        "--- tab 1 \"Fake Agent\" (active)\n",
        "--- agent zone=1 state=ready\n",
        "--- entry 1 user attachments=2 \"why?\"\n",
        "--- attached entry=1 index=0 kind=text id=3 name=\"cargo test\" bytes=6 state=closed\n",
        "--- attached entry=1 index=1 kind=link path=\"src/a.rs\" name=\"a.rs\"\n",
        "--- conversation entries=1 at=end earlier=none\n",
        "--- draft index=0 kind=link path=\"b.rs\" name=\"b.rs\"\n",
        "--- prompt status=ready\n",
    ));
    ensure!(
        dump.agent_attached(1)
            == [
                "index=0 kind=text id=3 name=\"cargo test\" bytes=6 state=closed",
                "index=1 kind=link path=\"src/a.rs\" name=\"a.rs\""
            ],
        "attached {:?}",
        dump.agent
    );
    ensure!(dump.agent_draft() == ["index=0 kind=link path=\"b.rs\" name=\"b.rs\""], "draft {:?}", dump.agent);
    ensure!(dump.agent_entry(1) == Some("user attachments=2 \"why?\""), "entry {:?}", dump.agent);
    Ok(())
}
