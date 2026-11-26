//! Agent tabs (OpenSpec change `agents-profiles`): each case plays an agent
//! with the fake ACP agent (`Case::needs_fake_agent`), from a script it
//! writes into its case directory.

use std::path::{Path, PathBuf};

use super::cli::{usage_refusal, usage_refusal_from};
use super::instance::Launcher;
use crate::brindle::{Run, Stdin};
use crate::{Ctx, Outcome, Result, ensure};

/// What a turn's answer says.
const HELLO: &str = "Hello!";

/// The fake agent's script for one agent that is ready at once and
/// answers its first prompt with [`HELLO`], recording what it was sent in
/// `received.jsonl`, then idles until its session ends.
fn hello_script(received: &Path) -> String {
    [
        format!("record {}", received.display()),
        "expect initialize".into(),
        r#"respond {"protocolVersion":1,"agentInfo":{"name":"Fake Agent","version":"1.0"}}"#.into(),
        "expect session/new".into(),
        r#"respond {"sessionId":"s1"}"#.into(),
        "expect session/prompt".into(),
        says(HELLO),
        r#"respond {"stopReason":"end_turn"}"#.into(),
    ]
    .join("\n")
        + "\n"
}

/// An agent profile `Fake` running the fake agent on `script`, as TOML to
/// add to the test config.
fn fake_profile(ctx: &Ctx, script: &Path) -> String {
    format!(
        "\n[[profiles]]\nname = \"Fake\"\ntype = \"agent\"\ncommand = '{}'\nargs = ['{}']\n",
        ctx.fake_agent.display(),
        script.display()
    )
}

/// Writes [`hello_script`] into the case directory: the `Fake` profile's
/// TOML, and where the agent records what it was sent.
pub(super) fn hello_agent(ctx: &Ctx) -> Result<(String, PathBuf)> {
    let (script, received) = (ctx.dir.join("agent.script"), ctx.dir.join("received.jsonl"));
    std::fs::write(&script, hello_script(&received)).map_err(|e| format!("writing the script: {e}"))?;
    Ok((fake_profile(ctx, &script), received))
}

/// The waits for the answer to the first prompt.
pub(super) fn answered() -> String {
    format!("agent {HELLO:?}")
}

/// A `send` step streaming `text` into the agent's message.
fn says(text: &str) -> String {
    let text = text.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace('\t', "\\t");
    format!(
        r#"send {{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"s1","update":{{"sessionUpdate":"agent_message_chunk","content":{{"type":"text","text":"{text}"}}}}}}}}"#
    )
}

/// A fake agent's script, built step by step and written into the case
/// directory.
struct Script(Vec<String>);

impl Script {
    /// An agent that records what it is sent in `received`, names itself
    /// `Fake Agent` and is ready at once.
    fn ready(received: &Path) -> Self {
        Script(vec![
            format!("record {}", received.display()),
            "expect initialize".into(),
            r#"respond {"protocolVersion":1,"agentInfo":{"name":"Fake Agent","version":"1.0"}}"#.into(),
            "expect session/new".into(),
            r#"respond {"sessionId":"s1"}"#.into(),
        ])
    }

    fn step(mut self, step: impl Into<String>) -> Self {
        self.0.push(step.into());
        self
    }

    /// Waits for a prompt, then holds its turn until `gate` exists.
    fn prompt_held(self, gate: &Path) -> Self {
        self.step("expect session/prompt").step(format!("wait-file {}", gate.display()))
    }

    /// Writes the script as `NAME.script` in the case directory and
    /// returns the `Fake` profile running it.
    fn profile(self, ctx: &Ctx, name: &str) -> Result<String> {
        let path = ctx.dir.join(format!("{name}.script"));
        std::fs::write(&path, self.0.join("\n") + "\n").map_err(|e| format!("writing {}: {e}", path.display()))?;
        Ok(fake_profile(ctx, &path))
    }
}

/// A file in the case directory a fake agent's `wait-file` waits for,
/// which the case creates; it is never created when it holds a turn for
/// the whole run.
fn gate(ctx: &Ctx, name: &str) -> PathBuf {
    ctx.dir.join(name)
}

/// What the agent zone's `--wait-for` text shows while a turn runs.
const WORKING: &str = "turn=working";

/// Waits until the case server lists `count` sessions, which shows that
/// closed agents ended rather than being left detached.
fn sessions_listed(ctx: &Ctx, count: usize) -> Result {
    ctx.scale().eventually(10.0, &format!("{count} listed sessions"), || {
        crate::brindle::listed_sessions(ctx).is_ok_and(|listed| listed.len() == count)
    })
}

/// What `--wait-for` waits for until the agent is ready.
const READY: &str = "state=ready";

/// A turn typed into the prompt input (`--send`, then enter): the prompt
/// has focus at launch, `--send` types the draft (shown in `--- input`),
/// enter sends it once the agent is ready, and the reply streamed in three
/// chunks reads as one agent message; the turn ends `done`, the input is
/// empty and keeps focus.
pub fn turn(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("received.jsonl");
    let profile = Script::ready(&received)
        .step("expect session/prompt")
        .step(says("Hel"))
        .step(says("lo"))
        .step(says("!"))
        .step(r#"respond {"stopReason":"end_turn"}"#)
        .profile(ctx, "turn")?;
    let out = Run::new(ctx, "draft").config(&profile).agent("Fake").wait_for(READY).send("hi").dump_at(4.0).run()?;
    ensure!(out.dump.focus.as_deref() == Some("agent") && out.dump.focus_stop.as_deref() == Some("prompt"), "{:?}", out.dump.focus_stop);
    ensure!(out.dump.input.as_ref().map(|i| i.text.as_str()) == Some("hi"), "the draft: {:?}", out.dump.input);
    ensure!(out.dump.agent_entries().is_empty(), "nothing sent yet: {:?}", out.dump.agent);

    let out = Run::new(ctx, "turn")
        .config(&profile)
        .agent("Fake")
        .wait_for(READY)
        .send("hi")
        .key("enter")
        .wait_for(&answered())
        .dump_at(6.0)
        .run()?;
    ensure!(out.dump.wait_timeouts.is_empty(), "waits that gave up: {:?}", out.dump.wait_timeouts);
    ensure!(out.dump.agent_entries() == ["user \"hi\"", "agent \"Hello!\""], "entries: {:?}", out.dump.agent);
    ensure!(out.dump.agent_field("turn") == Some("ended:end_turn"), "agent: {:?}", out.dump.agent);
    ensure!(out.dump.agent_field("status") == Some("done"), "agent: {:?}", out.dump.agent);
    ensure!(out.dump.input.as_ref().map(|i| i.text.as_str()) == Some(""), "the input emptied: {:?}", out.dump.input);
    ensure!(out.dump.focus_stop.as_deref() == Some("prompt"), "focus stays in the prompt: {:?}", out.dump.focus_stop);
    let sent = std::fs::read_to_string(&received).map_err(|e| format!("reading {}: {e}", received.display()))?;
    ensure!(sent.lines().filter(|l| l.contains("\"session/prompt\"")).count() == 1, "one prompt sent: {sent}");
    Ok(Outcome::Pass)
}

/// `brindle agent Fake -p -` reads its first prompt from standard input,
/// a file or a pipe, one trailing line break dropped, and sends it once
/// the agent is ready. Over 256 KiB, empty and not UTF-8 are usage errors
/// (status 2) before any window.
pub fn prompt_stdin(ctx: &Ctx) -> Result<Outcome> {
    let (profile, _) = hello_agent(ctx)?;
    let file = ctx.dir.join("prompt.txt");
    std::fs::write(&file, "from a file\n").map_err(|e| format!("writing {}: {e}", file.display()))?;
    for (label, stdin, text) in [
        ("file", Stdin::File(file.clone()), "from a file"),
        ("pipe", Stdin::Pipe(b"from a pipe\r\n".to_vec()), "from a pipe"),
    ] {
        let out = Run::new(ctx, label)
            .config(&profile)
            .agent("Fake")
            .first_prompt("-")
            .stdin(stdin)
            .wait_for(&answered())
            .dump_at(5.0)
            .run()?;
        ensure!(out.dump.wait_timeouts.is_empty(), "{label}: waits that gave up: {:?}", out.dump.wait_timeouts);
        let user = format!("user {text:?}");
        ensure!(out.dump.agent_entries() == [user.as_str(), "agent \"Hello!\""], "{label}: entries {:?}", out.dump.agent);
    }

    let config = ctx.dir.join("config.toml");
    std::fs::write(&config, crate::brindle::config("e2e") + &profile).map_err(|e| format!("writing the config: {e}"))?;
    let refusals: [(&str, Vec<u8>, &str); 3] = [
        ("long", vec![b'a'; 256 * 1024 + 1], "the prompt is over the 256 KiB limit"),
        ("empty", Vec::new(), "the prompt is blank"),
        ("not-utf8", b"\xff\xfe hi".to_vec(), "the prompt isn't UTF-8"),
    ];
    for (label, bytes, says) in refusals {
        let path = ctx.dir.join(format!("{label}.txt"));
        std::fs::write(&path, bytes).map_err(|e| format!("writing {}: {e}", path.display()))?;
        let stdin = std::fs::File::open(&path).map_err(|e| format!("opening {}: {e}", path.display()))?;
        let said = usage_refusal_from(ctx, label, &config, &["agent", "Fake", "-p", "-"], stdin.into())?;
        ensure!(said.contains(says), "{label}: brindle agent -p - said {said:?}, not {says:?}");
    }
    Ok(Outcome::Pass)
}

/// The seen rule on the tab's dot: after a turn that ended normally the
/// agent tab's face shows the green dot (seen from a Shell tab opened in
/// front of it, so the strip can be found from its grid); typing a key into
/// the prompt input first leaves no dot.
pub fn seen(ctx: &Ctx) -> Result<Outcome> {
    use super::tabs::{FACE_HEIGHT, FACE_LEFT, FACE_TOP, Strip};
    use crate::brindle::theme;
    let (profile, _) = hello_agent(ctx)?;
    let green = theme::ANSI[2];
    for (label, typed) in [("done", false), ("seen", true)] {
        let mut run = Run::new(ctx, label).config(&profile).agent("Fake").first_prompt("hi").wait_for(&answered());
        if typed {
            run = run.send("x");
        }
        let out = run.action("new_tab").capture_at(7.0).dump_at(7.5).run()?;
        ensure!(out.dump.wait_timeouts.is_empty(), "{label}: waits that gave up: {:?}", out.dump.wait_timeouts);
        ensure!(out.dump.tabs.len() == 2 && out.dump.tabs[1].1, "{label}: the Shell tab in front: {:?}", out.dump.tabs);
        let image = out.image.as_ref().ok_or("no capture")?;
        let strip = Strip::locate(&out, image)?;
        let scale = out.dump.grid()?.scale;
        let lead = strip.rect(FACE_LEFT + 8.0, FACE_TOP + 8.0, 14.0, FACE_HEIGHT - 16.0);
        if typed {
            image.expect_none("seen: no dot on the agent tab", lead, green)?;
        } else {
            image.expect("done: the green dot on the agent tab", lead, green, (10.0 * scale * scale) as usize)?;
        }
    }
    Ok(Outcome::Pass)
}

/// The command palette in an agent zone lists the agent's commands with the
/// zone and tab ones, and none that acts on a terminal.
pub fn palette_commands(ctx: &Ctx) -> Result<Outcome> {
    let (profile, _) = hello_agent(ctx)?;
    let out = Run::new(ctx, "palette").config(&profile).agent("Fake").wait_for(READY).action("open_command_palette").dump_at(4.0).run()?;
    let listed = |label: &str| out.dump.palette.iter().skip(1).any(|row| row.starts_with(&format!("{label}  ")) || row == label);
    for label in ["Stop Agent Turn", "Copy Last Agent Message", "Scroll Agent to Bottom", "Split Zone Right", "Close Tab"] {
        ensure!(listed(label), "{label} isn't listed: {:?}", out.dump.palette);
    }
    for label in ["Clear Scrollback", "Find", "Scroll to Bottom", "Copy"] {
        ensure!(!listed(label), "{label} is listed: {:?}", out.dump.palette);
    }
    Ok(Outcome::Pass)
}

/// Closing a working agent asks first (design.md decision 12): Close
/// Zone opens the confirmation naming the working agent, Cancel focused,
/// with the turn going on; enter and escape cancel, leaving the zone and
/// its turn; on a tab with a working and an idle agent zone, Close Tab
/// names only the working one, and tab then enter closes the tab, ending
/// both agents (only the other tab's shell is left to detach at quit).
pub fn close_confirm(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("received.jsonl");
    let profile = Script::ready(&received).prompt_held(&gate(ctx, "never")).profile(ctx, "held")?;
    let started = |label| Run::new(ctx, label).config(&profile).agent("Fake").first_prompt("hi").wait_for(WORKING);

    let out = started("asks").action("close_zone").dump_at(5.0).run()?;
    ensure!(out.dump.wait_timeouts.is_empty(), "waits that gave up: {:?}", out.dump.wait_timeouts);
    ensure!(out.dump.confirm.as_deref() == Some("close target=zone:1 busy=1:working"), "confirm: {:?}", out.dump.confirm);
    ensure!(
        out.dump.focus.as_deref() == Some("confirm") && out.dump.focus_stop.as_deref() == Some("cancel"),
        "Cancel has focus: {:?} {:?}",
        out.dump.focus,
        out.dump.focus_stop
    );
    ensure!(out.dump.agent_field("turn") == Some("working"), "the turn goes on: {:?}", out.dump.agent);

    for (label, key) in [("enter", "enter"), ("escape", "escape")] {
        let out = started(label).action("close_zone").key(key).dump_at(6.0).run()?;
        ensure!(out.dump.confirm.is_none(), "{key} closed the confirmation: {:?}", out.dump.confirm);
        ensure!(out.dump.tabs.len() == 1, "{key}: the tab stays: {:?}", out.dump.tabs);
        ensure!(out.dump.agent_field("turn") == Some("working"), "{key}: the turn goes on: {:?}", out.dump.agent);
        ensure!(
            out.dump.focus.as_deref() == Some("agent") && out.dump.focus_stop.as_deref() == Some("prompt"),
            "{key}: focus back in the prompt input: {:?} {:?}",
            out.dump.focus,
            out.dump.focus_stop
        );
    }

    // A tab of two agent zones, the first working and the second idle, and
    // a shell tab beside it.
    let out = started("tab")
        .action("split_zone_right")
        .action("new_tab")
        .action("prev_tab")
        .action("close_tab")
        .dump_at(8.0)
        .run()?;
    ensure!(out.dump.confirm.as_deref() == Some("close target=tab:1 busy=1:working"), "confirm: {:?}", out.dump.confirm);
    let out = started("tab-closed")
        .keep_detached()
        .action("split_zone_right")
        .action("new_tab")
        .action("prev_tab")
        .action("close_tab")
        .key("tab")
        .key("enter")
        .dump_at(10.0)
        .run()?;
    ensure!(out.dump.confirm.is_none(), "the confirmation closed: {:?}", out.dump.confirm);
    ensure!(out.dump.tabs.len() == 1 && out.dump.tabs[0].0 != "Fake Agent", "only the shell tab is left: {:?}", out.dump.tabs);
    sessions_listed(ctx, 1)?;
    crate::brindle::hang_up_detached(ctx)?;
    Ok(Outcome::Pass)
}

/// Closing an agent zone whose turn has ended asks nothing: the zone (and
/// its tab, the last zone) closes at once, and its agent ends.
pub fn close_idle(ctx: &Ctx) -> Result<Outcome> {
    let (profile, _) = hello_agent(ctx)?;
    let out = Run::new(ctx, "run")
        .config(&profile)
        .agent("Fake")
        .first_prompt("hi")
        .keep_detached()
        .wait_for(&answered())
        .action("new_tab")
        .action("prev_tab")
        .action("close_zone")
        .dump_at(7.0)
        .run()?;
    ensure!(out.dump.wait_timeouts.is_empty(), "waits that gave up: {:?}", out.dump.wait_timeouts);
    ensure!(out.dump.confirm.is_none(), "nothing asked: {:?}", out.dump.confirm);
    ensure!(out.dump.tabs.len() == 1 && out.dump.tabs[0].0 != "Fake Agent", "the agent tab closed: {:?}", out.dump.tabs);
    sessions_listed(ctx, 1)?;
    crate::brindle::hang_up_detached(ctx)?;
    Ok(Outcome::Pass)
}

/// `brindle agent Fake -p hi` opens an agent tab that sends `hi` once the
/// agent is ready; the answer follows it. A relaunch restores the tab on
/// the same session, showing the turn, and sends nothing again.
pub fn first_prompt(ctx: &Ctx) -> Result<Outcome> {
    let (profile, _) = hello_agent(ctx)?;
    let out = Run::new(ctx, "first")
        .config(&profile)
        .agent("Fake")
        .first_prompt("hi")
        .keep_detached()
        .wait_for(&answered())
        .dump_at(4.0)
        .run()?;
    ensure!(out.dump.wait_timeouts.is_empty(), "waits that gave up: {:?}", out.dump.wait_timeouts);
    ensure!(out.dump.agent_entries() == ["user \"hi\"", "agent \"Hello!\""], "entries: {:?}", out.dump.agent);
    ensure!(out.dump.agent_field("state") == Some("ready"), "agent: {:?}", out.dump.agent);
    ensure!(out.dump.tabs == [("Fake Agent".to_string(), true)], "tabs: {:?}", out.dump.tabs);

    let out = Run::new(ctx, "relaunch").config(&profile).restore().wait_for(&answered()).dump_at(4.0).run()?;
    ensure!(out.dump.wait_timeouts.is_empty(), "waits that gave up: {:?}", out.dump.wait_timeouts);
    ensure!(
        out.dump.agent_entries() == ["user \"hi\"", "agent \"Hello!\""],
        "the restored conversation, with the first prompt sent once: {:?}",
        out.dump.agent
    );
    ensure!(out.dump.agent_field("session") == Some("open"), "agent: {:?}", out.dump.agent);
    Ok(Outcome::Pass)
}

/// A launch naming an unknown profile, an agent launch naming a terminal
/// profile, a terminal launch naming an agent profile, and an agent launch
/// with no agent profile in the config are usage errors at status 2: alone,
/// before any window opens, and handed to a running Brindle, whose window
/// gains no tab.
pub fn launch_kinds(ctx: &Ctx) -> Result<Outcome> {
    let (profile, _) = hello_agent(ctx)?;
    let refusals: [(&[&str], &str); 3] = [
        (&["agent", "wrk"], "no profile is named \"wrk\" (the profiles are: Shell, Fake)"),
        (&["agent", "Shell"], "\"Shell\" is a terminal profile: open it with brindle terminal Shell"),
        (&["terminal", "Fake"], "\"Fake\" is an agent profile: open it with brindle agent Fake"),
    ];
    // Alone: the config the runs below use.
    let config = ctx.dir.join("config.toml");
    std::fs::write(&config, crate::brindle::config("e2e") + &profile).map_err(|e| format!("writing the config: {e}"))?;
    for (ix, (args, says)) in refusals.iter().enumerate() {
        let said = usage_refusal(ctx, ["alone-0", "alone-1", "alone-2"][ix], &config, args)?;
        ensure!(said.contains(says), "brindle {args:?} said {said:?}, not {says:?}");
    }
    let shells = ctx.dir.join("shells.toml");
    std::fs::write(&shells, crate::brindle::config("e2e")).map_err(|e| format!("writing the config: {e}"))?;
    let said = usage_refusal(ctx, "alone-none", &shells, &["agent"])?;
    ensure!(said.contains("the config has no agent profile"), "brindle agent said {said:?}");

    // Handed off.
    let (launcher, cwd) = (Launcher::of(ctx), ctx.dir.clone());
    let out = Run::new(ctx, "handed")
        .config(&profile)
        .call_at(2.0, move |_| {
            for (args, says) in refusals {
                let said = launcher.refused(&cwd, args)?;
                ensure!(said.contains(says), "a handed-off brindle {args:?} said {said:?}, not {says:?}");
            }
            Ok(())
        })
        .dump_at(5.0)
        .run()?;
    ensure!(out.dump.tabs.len() == 1, "the window gained a tab: {:?}", out.dump.tabs);

    let (launcher, cwd) = (Launcher::of(ctx), ctx.dir.clone());
    let out = Run::new(ctx, "handed-none")
        .call_at(2.0, move |_| {
            let said = launcher.refused(&cwd, &["agent"])?;
            ensure!(said.contains("the config has no agent profile"), "a handed-off brindle agent said {said:?}");
            Ok(())
        })
        .dump_at(4.0)
        .run()?;
    ensure!(out.dump.tabs.len() == 1, "the window gained a tab: {:?}", out.dump.tabs);
    Ok(Outcome::Pass)
}

/// `brindle agent Fake -p hi`, run from another directory beside a running
/// Brindle, is handed to it: its window gains an active agent tab whose
/// agent starts in the launching command's directory and is sent `hi`.
pub fn handoff(ctx: &Ctx) -> Result<Outcome> {
    let (profile, received) = hello_agent(ctx)?;
    let elsewhere = ctx.dir.join("elsewhere");
    std::fs::create_dir_all(&elsewhere).map_err(|e| e.to_string())?;
    let (launcher, cwd) = (Launcher::of(ctx), elsewhere.clone());
    let out = Run::new(ctx, "run")
        .config(&profile)
        .call_at(2.0, move |_| launcher.launch(&cwd, &["agent", "Fake", "-p", "hi"]))
        .wait_for(&answered())
        .dump_at(6.0)
        .run()?;
    ensure!(out.dump.wait_timeouts.is_empty(), "waits that gave up: {:?}", out.dump.wait_timeouts);
    ensure!(out.dump.tabs.len() == 2 && out.dump.tabs[1].1, "tabs: {:?}", out.dump.tabs);
    ensure!(out.dump.agent_entries() == ["user \"hi\"", "agent \"Hello!\""], "entries: {:?}", out.dump.agent);
    let sent = std::fs::read_to_string(&received).map_err(|e| format!("reading {}: {e}", received.display()))?;
    let new = sent.lines().find(|line| line.contains("\"session/new\"")).ok_or_else(|| format!("no session/new: {sent:?}"))?;
    let dir = format!("\"cwd\":\"{}\"", elsewhere.display());
    ensure!(new.contains(&dir), "the agent didn't start in {}: {new}", elsewhere.display());
    Ok(Outcome::Pass)
}

/// A `send` step of a `session/update` whose `update` is `update` (JSON).
fn update(update: &str) -> String {
    format!(r#"send {{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"s1","update":{update}}}}}"#)
}

/// A `send` step of a permission request with the JSON-RPC id `id` about
/// tool call `call_1`, offering `allow-once` and `reject-once`.
fn ask(id: &str) -> String {
    ask_about(id, "Write a file")
}

/// [`ask`] with the tool call titled `title`.
fn ask_about(id: &str, title: &str) -> String {
    format!(
        r#"send {{"jsonrpc":"2.0","id":"{id}","method":"session/request_permission","params":{{"sessionId":"s1","toolCall":{{"toolCallId":"call_1","title":"{title}"}},"options":[{{"optionId":"allow-once","name":"Allow","kind":"allow_once"}},{{"optionId":"reject-once","name":"Reject","kind":"reject_once"}}]}}}}"#
    )
}

/// A `respond` step ending the prompt's turn with `stop`.
fn stop(stop: &str) -> String {
    format!(r#"respond {{"stopReason":"{stop}"}}"#)
}

/// The prompts' texts the agent was sent, in order, from its record.
fn prompts_sent(received: &Path) -> Result<Vec<String>> {
    let text = std::fs::read_to_string(received).map_err(|e| format!("reading {}: {e}", received.display()))?;
    let mut prompts = Vec::new();
    for line in text.lines() {
        let value: serde_json::Value = serde_json::from_str(line).map_err(|e| format!("{line}: {e}"))?;
        if value["method"] == "session/prompt" {
            let blocks = value["params"]["prompt"].as_array().cloned().unwrap_or_default();
            prompts.push(blocks.iter().filter_map(|b| b["text"].as_str()).collect::<String>());
        }
    }
    Ok(prompts)
}

/// Whether the agent's record holds a message with `method`.
fn recorded(received: &Path, method: &str) -> bool {
    let text = std::fs::read_to_string(received).unwrap_or_default();
    text.lines().any(|line| line.contains(&format!("\"method\":\"{method}\"")))
}

/// The steps that open `gate` (a fake agent's `wait-file`) from a Shell
/// tab, so it opens after the steps before: a new tab the first time
/// (`first`), tab 2 after that, its shell typing `touch`; then back to the
/// agent's tab, the first, whose prompt input takes focus again.
fn touch_from_shell<'a>(run: Run<'a>, gate: &Path, first: bool) -> Run<'a> {
    let run = if first { run.action("new_tab") } else { run.action("activate_tab_2") };
    run.send(&format!("touch '{}'\r", gate.display())).action("activate_tab_1")
}

/// The dump's agent lines start with `prefix`, one of them.
pub(super) fn agent_has(out: &crate::brindle::Output, prefix: &str) -> Result {
    ensure!(out.dump.agent.iter().any(|line| line.starts_with(prefix)), "no `--- {prefix}…` in {:?}", out.dump.agent);
    Ok(())
}

/// No `--wait-for` gave up or was still waiting at the dump.
pub(super) fn waited(out: &crate::brindle::Output) -> Result {
    ensure!(
        out.dump.wait_timeouts.is_empty() && out.dump.wait_pending.is_empty(),
        "waits that gave up: {:?}, still waiting: {:?}; agent {:?}",
        out.dump.wait_timeouts,
        out.dump.wait_pending,
        out.dump.agent
    );
    Ok(())
}

/// Shift-enter adds a line to the prompt and enter sends both lines as one
/// prompt; the agent's answer keeps its own line break.
pub fn multiline(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("received.jsonl");
    let profile = Script::ready(&received)
        .step("expect session/prompt")
        .step(says("first\nsecond"))
        .step(stop("end_turn"))
        .profile(ctx, "multiline")?;
    let out = Run::new(ctx, "run")
        .config(&profile)
        .agent("Fake")
        .wait_for(READY)
        .send("line one")
        .key("shift-enter")
        .send("line two")
        .key("enter")
        .wait_for("turn=ended:end_turn")
        .dump_at(8.0)
        .run()?;
    waited(&out)?;
    let want = ["user \"line one\\nline two\"", "agent \"first\\nsecond\""];
    ensure!(out.dump.agent_entries() == want, "entries: {:?}", out.dump.agent);
    ensure!(prompts_sent(&received)? == ["line one\nline two"], "sent: {:?}", prompts_sent(&received)?);
    ensure!(out.dump.input.as_ref().map(|i| i.text.as_str()) == Some(""), "input: {:?}", out.dump.input);
    Ok(Outcome::Pass)
}

/// Escape during a turn stops it: the agent is sent `session/cancel`, the
/// turn ends `cancelled`, and the zone's status is `idle`; the prompt
/// input keeps its draft.
pub fn cancel(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("received.jsonl");
    let profile = Script::ready(&received)
        .step("expect session/prompt")
        .step("expect session/cancel")
        .step(stop("cancelled"))
        .profile(ctx, "cancel")?;
    let out = Run::new(ctx, "run")
        .config(&profile)
        .agent("Fake")
        .first_prompt("hi")
        .wait_for(WORKING)
        .send("draft")
        .key("escape")
        .wait_for("turn=ended:cancelled")
        .dump_at(6.0)
        .run()?;
    waited(&out)?;
    ensure!(recorded(&received, "session/cancel"), "no session/cancel was sent");
    ensure!(out.dump.agent_field("status") == Some("idle"), "agent: {:?}", out.dump.agent);
    ensure!(out.dump.input.as_ref().map(|i| i.text.as_str()) == Some("draft"), "input: {:?}", out.dump.input);
    Ok(Outcome::Pass)
}

/// A permission request blocks the turn (`status=blocked`); tab from the
/// prompt reaches its buttons, allow first, and enter on one answers it:
/// the agent gets the option chosen, and the turn goes on to its end.
pub fn permission_keys(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("received.jsonl");
    let profile = Script::ready(&received)
        .step("expect session/prompt")
        .step(ask("p1"))
        .step(r#"expect-response "p1""#)
        .step(says("done"))
        .step(stop("end_turn"))
        .profile(ctx, "permission")?;
    let pending = "outcome=pending";
    let started = |label| Run::new(ctx, label).config(&profile).agent("Fake").first_prompt("hi").wait_for(pending);

    let out = started("focus").key("tab").dump_at(5.0).run()?;
    waited(&out)?;
    ensure!(out.dump.agent_field("status") == Some("blocked"), "agent: {:?}", out.dump.agent);
    ensure!(out.dump.focus_stop.as_deref() == Some("permission:2:allow-once"), "focus: {:?}", out.dump.focus_stop);

    std::fs::remove_file(&received).ok();
    let out = started("reject").key("tab").key("tab").key("enter").wait_for("agent \"done\"").dump_at(7.0).run()?;
    waited(&out)?;
    agent_has(&out, "entry 2 permission call=\"call_1\" options=allow-once:\"Allow\"|reject-once:\"Reject\" outcome=selected:reject-once")?;
    ensure!(out.dump.agent_field("turn") == Some("ended:end_turn"), "agent: {:?}", out.dump.agent);
    let sent = std::fs::read_to_string(&received).map_err(|e| e.to_string())?;
    ensure!(
        sent.lines().any(|l| l.contains("\"id\":\"p1\"") && l.contains("\"optionId\":\"reject-once\"")),
        "the agent wasn't answered reject-once: {sent}"
    );
    Ok(Outcome::Pass)
}

/// A thought shows closed; Toggle Agent Thoughts opens it.
pub fn thoughts(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("received.jsonl");
    let profile = Script::ready(&received)
        .step("expect session/prompt")
        .step(update(r#"{"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"pondering"}}"#))
        .step(says(HELLO))
        .step(stop("end_turn"))
        .profile(ctx, "thoughts")?;
    let started = |label| Run::new(ctx, label).config(&profile).agent("Fake").first_prompt("hi").wait_for(&answered());
    let out = started("closed").dump_at(4.0).run()?;
    waited(&out)?;
    agent_has(&out, "entry 2 thought open=false \"pondering\"")?;
    let out = started("open").action("toggle_agent_thoughts").dump_at(5.0).run()?;
    waited(&out)?;
    agent_has(&out, "entry 2 thought open=true \"pondering\"")?;
    Ok(Outcome::Pass)
}

/// A conversation longer than the zone: shift-pageup leaves its end
/// (`at=away`), and Scroll Agent to Bottom returns to it.
pub fn scroll(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("received.jsonl");
    let long: Vec<String> = (1..=200).map(|n| format!("line {n}")).collect();
    let profile = Script::ready(&received)
        .step("expect session/prompt")
        .step(says(&long.join("\n")))
        .step(says("\nthe end"))
        .step(stop("end_turn"))
        .profile(ctx, "scroll")?;
    let started =
        |label| Run::new(ctx, label).config(&profile).agent("Fake").first_prompt("hi").wait_for("turn=ended:end_turn");
    let out = started("end").dump_at(4.0).run()?;
    waited(&out)?;
    agent_has(&out, "conversation entries=2 at=end")?;
    let out = started("away").key("shift-pageup").dump_at(5.0).run()?;
    waited(&out)?;
    agent_has(&out, "conversation entries=2 at=away")?;
    let out = started("back").key("shift-pageup").action("agent_scroll_to_bottom").dump_at(6.0).run()?;
    waited(&out)?;
    agent_has(&out, "conversation entries=2 at=end")?;
    Ok(Outcome::Pass)
}

/// Copy Last Agent Message puts the agent's last message on the
/// clipboard, which a paste into the prompt input shows.
pub fn copy(ctx: &Ctx) -> Result<Outcome> {
    let (profile, _) = hello_agent(ctx)?;
    let out = Run::new(ctx, "run")
        .config(&profile)
        .agent("Fake")
        .first_prompt("hi")
        .wait_for(&answered())
        .action("copy_last_agent_message")
        .action("text_input_paste")
        .dump_at(6.0)
        .run()?;
    waited(&out)?;
    ensure!(out.dump.input.as_ref().map(|i| i.text.as_str()) == Some(HELLO), "input: {:?}", out.dump.input);
    Ok(Outcome::Pass)
}

/// Split Zone Right in an agent tab opens a second agent zone on the same
/// profile, with a session of its own, focused; the first keeps its turn.
pub fn split(ctx: &Ctx) -> Result<Outcome> {
    let (profile, received) = hello_agent(ctx)?;
    let out = Run::new(ctx, "run")
        .config(&profile)
        .agent("Fake")
        .first_prompt("hi")
        .wait_for(&answered())
        .action("split_zone_right")
        .wait_for("conversation entries=0")
        .wait_for(READY)
        .dump_at(7.0)
        .run()?;
    waited(&out)?;
    ensure!(out.dump.zones.as_deref() == Some("count=2 active=2"), "zones: {:?}", out.dump.zones);
    ensure!(out.dump.agent_field("zone") == Some("2"), "agent: {:?}", out.dump.agent);
    ensure!(out.dump.agent_field("state") == Some("ready"), "agent: {:?}", out.dump.agent);
    ensure!(out.dump.focus_stop.as_deref() == Some("prompt"), "focus: {:?}", out.dump.focus_stop);
    let sent = std::fs::read_to_string(&received).map_err(|e| e.to_string())?;
    ensure!(sent.lines().filter(|l| l.contains("\"session/new\"")).count() == 2, "two sessions: {sent}");
    Ok(Outcome::Pass)
}

/// An agent profile whose command doesn't exist: the zone can't start
/// (its words, naming the command and the cause, are `failure_text`'s,
/// unit-tested; the dump gives the state), the prompt input is
/// unavailable, the first prompt becomes the draft, and nothing is left
/// running in the server.
pub fn no_command(ctx: &Ctx) -> Result<Outcome> {
    let profile = "\n[[profiles]]\nname = \"Fake\"\ntype = \"agent\"\ncommand = \"no-such-agent\"\n";
    let out = Run::new(ctx, "run")
        .config(profile)
        .agent("Fake")
        .first_prompt("hi")
        .keep_detached()
        .wait_for("state=not-started")
        .dump_at(4.0)
        .run()?;
    waited(&out)?;
    agent_has(&out, "agent zone=1 state=not-started message=\"the agent no-such-agent didn't start: ")?;
    agent_has(&out, "prompt status=disabled")?;
    ensure!(out.dump.input.as_ref().map(|i| i.text.as_str()) == Some("hi"), "the draft: {:?}", out.dump.input);
    ensure!(crate::brindle::listed_sessions(ctx)?.is_empty(), "sessions left: {:?}", crate::brindle::listed_sessions(ctx)?);
    Ok(Outcome::Pass)
}

/// A first prompt to an agent that needs a login is never sent: the zone
/// says it needs a login, and the prompt is left as the draft.
pub fn first_prompt_login(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("received.jsonl");
    let profile = Script(vec![
        format!("record {}", received.display()),
        "expect initialize".into(),
        r#"respond {"protocolVersion":1,"agentInfo":{"name":"Fake Agent","version":"1.0"}}"#.into(),
        "expect session/new".into(),
        r#"respond-error -32000 "run fake login""#.into(),
    ])
    .profile(ctx, "login")?;
    let out = Run::new(ctx, "run")
        .config(&profile)
        .agent("Fake")
        .first_prompt("hi")
        .wait_for("state=login-required")
        .dump_at(5.0)
        .run()?;
    waited(&out)?;
    ensure!(out.dump.input.as_ref().map(|i| i.text.as_str()) == Some("hi"), "the draft: {:?}", out.dump.input);
    ensure!(prompts_sent(&received)?.is_empty(), "a prompt was sent: {:?}", prompts_sent(&received)?);
    Ok(Outcome::Pass)
}

/// A login demanded by `session/new`: the zone says so with the agent's
/// message and offers no prompt. Then (design.md decision 16's script) one
/// demanded by a prompt after a finished turn while another prompt
/// waits: the turn fails, the waiting prompt is held and can't be sent
/// again, discard clears it, the prompt input is disabled, and the first
/// turn is still there.
pub fn login(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("received.jsonl");
    let at_start = Script(vec![
        format!("record {}", received.display()),
        "expect initialize".into(),
        r#"respond {"protocolVersion":1,"agentInfo":{"name":"Fake Agent","version":"1.0"}}"#.into(),
        "expect session/new".into(),
        r#"respond-error -32000 "run fake login""#.into(),
    ])
    .profile(ctx, "start")?;
    let out = Run::new(ctx, "start").config(&at_start).agent("Fake").wait_for("state=login-required").dump_at(4.0).run()?;
    waited(&out)?;
    ensure!(out.dump.agent.iter().any(|l| l.contains("run fake login")), "the agent's message: {:?}", out.dump.agent);
    agent_has(&out, "prompt status=disabled")?;

    let go = gate(ctx, "expire");
    let later = Script::ready(&received)
        .step("expect session/prompt")
        .step(says(HELLO))
        .step(stop("end_turn"))
        .step("expect session/prompt")
        .step(format!("wait-file {}", go.display()))
        .step(r#"respond-error -32000 "session expired""#)
        .step(format!("wait-file {}", gate(ctx, "never").display()))
        .profile(ctx, "later")?;
    let held = "queued held reason=login-required resend=no \"three\"";
    let run = Run::new(ctx, "later")
        .config(&later)
        .agent("Fake")
        .first_prompt("one")
        .wait_for(&answered())
        .send("two")
        .key("enter")
        .wait_for(WORKING)
        .send("three")
        .key("enter")
        .wait_for("queued waiting \"three\"");
    let out = touch_from_shell(run, &go, true)
        .wait_for("state=login-required")
        .wait_for(held)
        .action("agent_unqueue")
        .dump_at(16.0)
        .run()?;
    waited(&out)?;
    ensure!(!out.dump.agent.iter().any(|l| l.starts_with("queued")), "discard cleared the row: {:?}", out.dump.agent);
    ensure!(out.dump.agent_field("turn") == Some("failed"), "agent: {:?}", out.dump.agent);
    agent_has(&out, "prompt status=disabled")?;
    ensure!(out.dump.agent.iter().any(|l| l.contains("session expired")), "the agent's message: {:?}", out.dump.agent);
    let entries = out.dump.agent_entries();
    ensure!(entries.starts_with(&["user \"one\"", "agent \"Hello!\"", "user \"two\""]), "entries: {entries:?}");
    ensure!(prompts_sent(&received)? == ["one", "two"], "sent: {:?}", prompts_sent(&received)?);
    Ok(Outcome::Pass)
}

/// An agent that refuses ACP version 1 (`-32022` with the versions it
/// supports), and one that answers with version 2: each zone ends
/// `state=failed session=ended` with the version message and a notice
/// with the agent's standard error, never an exit status; the second is
/// sent no `session/new`.
pub fn acp_version(ctx: &Ctx) -> Result<Outcome> {
    let cases = [
        (
            "refused",
            r#"respond-error -32022 "Unsupported protocol version" {"requested":1,"supported":[2]}"#,
            "only ACP v2",
            "the agent supports ACP version 2; Brindle speaks 1",
        ),
        ("other", r#"respond {"protocolVersion":2,"agentCapabilities":{}}"#, "v2 only", "the agent speaks ACP version 2; Brindle speaks 1"),
    ];
    for (label, answer, stderr, says) in cases {
        let received = ctx.dir.join(format!("{label}.jsonl"));
        let profile = Script(vec![
            format!("record {}", received.display()),
            "expect initialize".into(),
            answer.into(),
            format!("stderr {stderr}"),
            "expect session/new".into(),
        ])
        .profile(ctx, label)?;
        let out = Run::new(ctx, label).config(&profile).agent("Fake").wait_for("session=ended").dump_at(5.0).run()?;
        waited(&out)?;
        ensure!(out.dump.agent_field("state") == Some("failed"), "{label}: agent {:?}", out.dump.agent);
        ensure!(out.dump.agent.iter().any(|l| l.contains(says)), "{label}: no {says:?} in {:?}", out.dump.agent);
        ensure!(
            out.dump.agent.iter().any(|l| l.starts_with("entry ") && l.contains("notice") && l.contains(stderr)),
            "{label}: no notice with the agent's standard error: {:?}",
            out.dump.agent
        );
        ensure!(!out.dump.agent.iter().any(|l| l.contains("exited")), "{label}: an exit status: {:?}", out.dump.agent);
        ensure!(!recorded(&received, "session/new"), "{label}: session/new was sent");
    }
    Ok(Outcome::Pass)
}

/// An agent that exits during a turn with a prompt waiting: the zone shows
/// the exit, the waiting prompt is held and can't be sent again, and
/// discard clears its row.
pub fn exit(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("received.jsonl");
    let go = gate(ctx, "exit");
    let profile = Script::ready(&received).prompt_held(&go).step("exit 3").profile(ctx, "exit")?;
    let run = Run::new(ctx, "run")
        .config(&profile)
        .agent("Fake")
        .first_prompt("one")
        .wait_for(WORKING)
        .send("two")
        .key("enter")
        .wait_for("queued waiting \"two\"");
    let out = touch_from_shell(run, &go, true)
        .wait_for("state=exited:3")
        .wait_for("queued held reason=agent-stopped resend=no \"two\"")
        .action("agent_unqueue")
        .dump_at(13.0)
        .run()?;
    waited(&out)?;
    ensure!(!out.dump.agent.iter().any(|l| l.starts_with("queued")), "discard cleared the row: {:?}", out.dump.agent);
    ensure!(out.dump.agent_field("status") == Some("error"), "agent: {:?}", out.dump.agent);
    ensure!(prompts_sent(&received)? == ["one"], "sent: {:?}", prompts_sent(&received)?);
    Ok(Outcome::Pass)
}

/// The input's text in the dump.
fn input_text(out: &crate::brindle::Output) -> Option<&str> {
    out.dump.input.as_ref().map(|i| i.text.as_str())
}

/// No queued row in the dump.
fn no_queued_row(out: &crate::brindle::Output) -> Result {
    ensure!(!out.dump.agent.iter().any(|l| l.starts_with("queued")), "a queued row: {:?}", out.dump.agent);
    Ok(())
}

/// The prompt queue (design.md decision 11), in four runs:
/// - `queue`: a prompt sent during a turn waits; remove takes it back; the
///   next one waits in its place and a third is kept in the input; the
///   turn's `end_turn` sends the waiting one.
/// - `held`: a `max_tokens` ending holds the waiting prompt, leaving the
///   input as it was; a new prompt is sent beside it; send again during
///   that turn queues it, and the turn's end sends it.
/// - `stop`: a stop holds the waiting prompt; the row's send-again button,
///   from the keyboard, sends it while idle; a failed turn holds the next
///   one, and the row's discard button clears it.
/// - `restore`: a held prompt survives quit and restore, and nothing is
///   sent again.
pub fn queue(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("received.jsonl");
    let g1 = gate(ctx, "queue-1");
    let profile = Script::ready(&received)
        .prompt_held(&g1)
        .step(says("A"))
        .step(stop("end_turn"))
        .step("expect session/prompt")
        .step(says("B"))
        .step(stop("end_turn"))
        .profile(ctx, "queue")?;
    let run = Run::new(ctx, "queue")
        .config(&profile)
        .agent("Fake")
        .first_prompt("one")
        .wait_for(WORKING)
        .send("gone")
        .key("enter")
        .wait_for("queued waiting \"gone\"")
        .action("agent_unqueue")
        .send("two")
        .key("enter")
        .wait_for("queued waiting \"two\"")
        .send("three")
        .key("enter");
    let out = touch_from_shell(run, &g1, true).wait_for("agent \"B\"").dump_at(18.0).run()?;
    waited(&out)?;
    let entries = ["user \"one\"", "agent \"A\"", "user \"two\"", "agent \"B\""];
    ensure!(out.dump.agent_entries() == entries, "queue: entries {:?}", out.dump.agent);
    ensure!(prompts_sent(&received)? == ["one", "two"], "queue: sent {:?}", prompts_sent(&received)?);
    ensure!(input_text(&out) == Some("three"), "queue: the third prompt kept: {:?}", out.dump.input);
    no_queued_row(&out)?;

    let received = ctx.dir.join("held.jsonl");
    let (g1, g2) = (gate(ctx, "held-1"), gate(ctx, "held-2"));
    let profile = Script::ready(&received)
        .prompt_held(&g1)
        .step(stop("max_tokens"))
        .prompt_held(&g2)
        .step(stop("end_turn"))
        .step("expect session/prompt")
        .step(stop("end_turn"))
        .profile(ctx, "held")?;
    let run = Run::new(ctx, "held")
        .config(&profile)
        .agent("Fake")
        .first_prompt("one")
        .wait_for(WORKING)
        .send("two")
        .key("enter")
        .wait_for("queued waiting \"two\"");
    let run = touch_from_shell(run, &g1, true)
        .wait_for("queued held reason=stopped:max_tokens resend=yes \"two\"")
        .send("three")
        .key("enter")
        .wait_for(WORKING)
        .action("agent_resend")
        .wait_for("queued waiting \"two\"");
    let out = touch_from_shell(run, &g2, false)
        .wait_for("user \"two\"")
        .wait_for("turn=ended:end_turn")
        .dump_at(24.0)
        .run()?;
    waited(&out)?;
    let entries = ["user \"one\"", "user \"three\"", "user \"two\""];
    ensure!(out.dump.agent_entries() == entries, "held: entries {:?}", out.dump.agent);
    ensure!(prompts_sent(&received)? == ["one", "three", "two"], "held: sent {:?}", prompts_sent(&received)?);
    ensure!(input_text(&out) == Some(""), "held: the input untouched: {:?}", out.dump.input);
    no_queued_row(&out)?;

    let received = ctx.dir.join("stop.jsonl");
    let g1 = gate(ctx, "stop-1");
    let profile = Script::ready(&received)
        .step("expect session/prompt")
        .step("expect session/cancel")
        .step(stop("cancelled"))
        .prompt_held(&g1)
        .step(r#"respond-error -32603 "boom""#)
        .profile(ctx, "stop")?;
    let run = Run::new(ctx, "stop")
        .config(&profile)
        .agent("Fake")
        .first_prompt("one")
        .wait_for(WORKING)
        .send("two")
        .key("enter")
        .wait_for("queued waiting \"two\"")
        .send("draft")
        .key("escape")
        .wait_for("queued held reason=cancelled resend=yes \"two\"")
        .key("tab")
        .key("enter")
        .wait_for("user \"two\"")
        .send("x")
        .key("enter")
        .wait_for("queued waiting \"draftx\"");
    let out = touch_from_shell(run, &g1, true)
        .wait_for("queued held reason=failed resend=yes \"draftx\"")
        .key("tab")
        .key("tab")
        .key("enter")
        .dump_at(26.0)
        .run()?;
    waited(&out)?;
    no_queued_row(&out)?;
    ensure!(out.dump.agent_field("turn") == Some("failed"), "stop: agent {:?}", out.dump.agent);
    ensure!(prompts_sent(&received)? == ["one", "two"], "stop: sent {:?}", prompts_sent(&received)?);
    ensure!(input_text(&out) == Some(""), "stop: the input untouched: {:?}", out.dump.input);

    let received = ctx.dir.join("restore.jsonl");
    let profile = Script::ready(&received)
        .step("expect session/prompt")
        .step("expect session/cancel")
        .step(stop("cancelled"))
        .step(format!("wait-file {}", gate(ctx, "never").display()))
        .profile(ctx, "restore")?;
    let held = "queued held reason=cancelled resend=yes \"two\"";
    let out = Run::new(ctx, "restore-1")
        .config(&profile)
        .agent("Fake")
        .first_prompt("one")
        .keep_detached()
        .wait_for(WORKING)
        .send("two")
        .key("enter")
        .wait_for("queued waiting \"two\"")
        .key("escape")
        .wait_for(held)
        .dump_at(9.0)
        .run()?;
    waited(&out)?;
    let out = Run::new(ctx, "restore-2").config(&profile).restore().wait_for(held).dump_at(5.0).run()?;
    waited(&out)?;
    ensure!(prompts_sent(&received)? == ["one"], "restore: sent {:?}", prompts_sent(&received)?);
    Ok(Outcome::Pass)
}

/// Quitting with an agent waiting on a permission and a prompt waiting
/// asks nothing; the next launch restores the tab with both, and the
/// first prompt isn't sent again.
pub fn quit_restore(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("received.jsonl");
    let profile = Script::ready(&received)
        .step("expect session/prompt")
        .step(ask("p1"))
        .step(format!("wait-file {}", gate(ctx, "never").display()))
        .profile(ctx, "quit")?;
    let pending = "outcome=pending";
    Run::new(ctx, "quit")
        .config(&profile)
        .agent("Fake")
        .first_prompt("one")
        .keep_detached()
        .wait_for(pending)
        .send("two")
        .key("enter")
        .wait_for("queued waiting \"two\"")
        .action("quit")
        .expect_exit()
        .dump_at(12.0)
        .run()?;
    let out = Run::new(ctx, "restored")
        .config(&profile)
        .restore()
        .wait_for(pending)
        .wait_for("queued waiting \"two\"")
        .dump_at(5.0)
        .run()?;
    waited(&out)?;
    let entries = out.dump.agent_entries();
    ensure!(
        entries.len() == 2 && entries[0] == "user \"one\"" && entries[1].ends_with("outcome=pending \"Write a file\""),
        "entries: {entries:?}"
    );
    ensure!(prompts_sent(&received)? == ["one"], "sent: {:?}", prompts_sent(&received)?);
    Ok(Outcome::Pass)
}

/// An agent session left detached and recorded nowhere (a later run's
/// quit replaced the state file) is swept into an agent tab at the next
/// launch, its conversation shown and nothing sent again.
pub fn sweep(ctx: &Ctx) -> Result<Outcome> {
    let (profile, received) = hello_agent(ctx)?;
    let out = Run::new(ctx, "agent")
        .config(&profile)
        .agent("Fake")
        .first_prompt("hi")
        .keep_detached()
        .wait_for(&answered())
        .dump_at(4.0)
        .run()?;
    waited(&out)?;
    let other = Run::new(ctx, "no-restore").config(&profile).keep_detached().dump_at(3.0).run()?;
    ensure!(other.dump.tabs.len() == 1, "the no-restore run's tabs: {:?}", other.dump.tabs);
    let out = Run::new(ctx, "back")
        .config(&profile)
        .restore()
        .action("next_tab")
        .wait_for(&answered())
        .dump_at(5.0)
        .run()?;
    waited(&out)?;
    ensure!(out.dump.tabs.len() == 2 && out.dump.tabs[1] == ("Fake Agent".to_string(), true), "tabs: {:?}", out.dump.tabs);
    ensure!(out.dump.agent_entries() == ["user \"hi\"", "agent \"Hello!\""], "entries: {:?}", out.dump.agent);
    ensure!(prompts_sent(&received)? == ["hi"], "sent: {:?}", prompts_sent(&received)?);
    Ok(Outcome::Pass)
}

/// The sessions palette lists a detached agent session as an agent with
/// its activity (`agent · blocked`) and its command; enter attaches it as
/// an agent tab showing the pending permission; `picker_kill` ends it.
pub fn sessions_palette(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("received.jsonl");
    let profile = Script::ready(&received)
        .step("expect session/prompt")
        .step(ask("p1"))
        .step(format!("wait-file {}", gate(ctx, "never").display()))
        .profile(ctx, "blocked")?;
    let pending = "outcome=pending";
    let out = Run::new(ctx, "agent")
        .config(&profile)
        .agent("Fake")
        .first_prompt("hi")
        .keep_detached()
        .wait_for(pending)
        .dump_at(4.0)
        .run()?;
    waited(&out)?;
    // The filter reads the rows' labels: only an agent's reads `agent · …`
    // (the shells' read their titles).
    let query = "agent";
    let out = Run::new(ctx, "list").config(&profile).keep_detached().action("attach_session").dump_at(4.0).run()?;
    ensure!(
        out.dump.palette.iter().skip(1).any(|row| row.starts_with("agent · blocked") && row.contains("brindle-fake-acp-agent")),
        "no agent row: {:?}",
        out.dump.palette
    );
    let out = Run::new(ctx, "attach")
        .config(&profile)
        .keep_detached()
        .action("attach_session")
        .send(query)
        .key("enter")
        .wait_for(pending)
        .dump_at(7.0)
        .run()?;
    waited(&out)?;
    ensure!(out.dump.tabs.len() == 2 && out.dump.tabs[1] == ("Fake Agent".to_string(), true), "tabs: {:?}", out.dump.tabs);
    ensure!(out.dump.agent_field("status") == Some("blocked"), "agent: {:?}", out.dump.agent);
    let out = Run::new(ctx, "kill")
        .config(&profile)
        .keep_detached()
        .action("attach_session")
        .send(query)
        .action("picker_kill")
        .dump_at(6.0)
        .run()?;
    ensure!(out.dump.palette.len() == 1, "the agent's row is still listed: {:?}", out.dump.palette);
    ctx.scale().eventually(10.0, "the agent session ended", || agent_listed(ctx).is_ok_and(|listed| !listed))?;
    crate::brindle::hang_up_detached(ctx)?;
    Ok(Outcome::Pass)
}

/// Whether `brindle-remote list` lists a session running the fake agent.
fn agent_listed(ctx: &Ctx) -> Result<bool> {
    let out = std::process::Command::new(&ctx.helper)
        .arg("list")
        .env("BRINDLE_SERVER_DIR", &ctx.server_dir)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("running {}: {e}", ctx.helper.display()))?;
    ensure!(out.status.success(), "`brindle-remote list` failed: {}", String::from_utf8_lossy(&out.stderr));
    Ok(String::from_utf8_lossy(&out.stdout).contains("brindle-fake-acp-agent"))
}

/// The permission an agent asks for in `agent-notify`.
const ASKED: &str = "Run cargo test?";

/// The notification log's lines for an agent's notice (`source<TAB>title
/// <TAB>body`): the agent's name, then the tab's title.
fn agent_notice(body: &str) -> String {
    format!("agent\tFake Agent — Fake Agent\t{body}")
}

/// The lines of a run's notification log; none when it wasn't written.
fn notices(log: &Path) -> Vec<String> {
    std::fs::read_to_string(log).unwrap_or_default().lines().map(str::to_string).collect()
}

/// A Shell tab's command that waits until `log` holds `text`, then prints
/// `label`, spelt apart on the command line so `--wait-for label` waits
/// for the output rather than the typed command.
fn until_logged(log: &Path, text: &str, label: &str) -> String {
    let (head, tail) = label.split_at(label.len() / 2);
    format!("until grep -qF '{text}' '{}'; do sleep 0.1; done; echo {head}\"\"{tail}\r", log.display())
}

/// An agent's notifications (`BRINDLE_NOTIFY_LOG`, source `agent`):
/// - in a background tab, with the default `when`, a permission request
///   notifies with its title and a turn that ends normally with `Done`,
///   each titled `Fake Agent — Fake Agent`; answering the permission in
///   front raises nothing;
/// - with `when = "always"` the same turn in the active tab notifies both,
///   and with `notifications.agent = false` it notifies nothing;
/// - a replay after a quit and relaunch notifies nothing, though it shows
///   the pending permission again.
pub fn notify(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("received.jsonl");
    let (asked, answered) = (gate(ctx, "asked"), gate(ctx, "answered"));
    let background = Script::ready(&received)
        .step("expect session/prompt")
        .step(format!("wait-file {}", asked.display()))
        .step(ask_about("p1", ASKED))
        .step(r#"expect-response "p1""#)
        .step(format!("wait-file {}", answered.display()))
        .step(says("done"))
        .step(stop("end_turn"))
        .profile(ctx, "background")?;
    let log = ctx.dir.join("background.notify");
    let out = Run::new(ctx, "background")
        .config(&background)
        .env("BRINDLE_NOTIFY_LOG", log.to_str().unwrap())
        .agent("Fake")
        .first_prompt("hi")
        .wait_for(WORKING)
        .action("new_tab")
        .send(&format!("touch '{}'; {}", asked.display(), until_logged(&log, ASKED, "notice-1")))
        .wait_for("notice-1")
        .action("activate_tab_1")
        .wait_for("outcome=pending")
        .key("tab")
        .key("enter")
        .wait_for("outcome=selected:allow-once")
        .action("activate_tab_2")
        .send(&format!("touch '{}'; {}", answered.display(), until_logged(&log, "Done", "notice-2")))
        .wait_for("notice-2")
        .dump_at(12.0)
        .run()?;
    waited(&out)?;
    ensure!(out.dump.tabs.len() == 2 && out.dump.tabs[1].1, "the Shell tab in front: {:?}", out.dump.tabs);
    let want = [agent_notice(ASKED), agent_notice("Done")];
    ensure!(notices(&log) == want, "background: notices {:?}, expected {want:?}", notices(&log));

    let in_front = Script::ready(&received)
        .step("expect session/prompt")
        .step(ask_about("p1", ASKED))
        .step(r#"expect-response "p1""#)
        .step(says("done"))
        .step(stop("end_turn"))
        .profile(ctx, "front")?;
    for (label, agent, want) in [("always", true, want.to_vec()), ("off", false, Vec::new())] {
        let log = ctx.dir.join(format!("{label}.notify"));
        let out = Run::new(ctx, label)
            .config(&in_front)
            .config(&format!("\n[notifications]\nwhen = \"always\"\nagent = {agent}\n"))
            .env("BRINDLE_NOTIFY_LOG", log.to_str().unwrap())
            .agent("Fake")
            .first_prompt("hi")
            .wait_for("outcome=pending")
            .key("tab")
            .key("enter")
            .wait_for("agent \"done\"")
            .wait_for("status=done")
            .dump_at(7.0)
            .run()?;
        waited(&out)?;
        ensure!(notices(&log) == want, "{label}: notices {:?}, expected {want:?}", notices(&log));
    }

    let pending = Script::ready(&received)
        .step("expect session/prompt")
        .step(ask_about("p1", ASKED))
        .step(format!("wait-file {}", gate(ctx, "never").display()))
        .profile(ctx, "replay")?;
    let notifications = "\n[notifications]\nwhen = \"always\"\n";
    let (live, replay) = (ctx.dir.join("live.notify"), ctx.dir.join("replay.notify"));
    Run::new(ctx, "live")
        .config(&pending)
        .config(notifications)
        .env("BRINDLE_NOTIFY_LOG", live.to_str().unwrap())
        .agent("Fake")
        .first_prompt("hi")
        .keep_detached()
        .wait_for("outcome=pending")
        .action("quit")
        .expect_exit()
        .dump_at(12.0)
        .run()?;
    ensure!(notices(&live) == [agent_notice(ASKED)], "live: notices {:?}", notices(&live));
    let out = Run::new(ctx, "replayed")
        .config(&pending)
        .config(notifications)
        .env("BRINDLE_NOTIFY_LOG", replay.to_str().unwrap())
        .restore()
        .wait_for("outcome=pending")
        .dump_at(5.0)
        .run()?;
    waited(&out)?;
    ensure!(out.dump.agent_field("status") == Some("blocked"), "replayed: agent {:?}", out.dump.agent);
    ensure!(notices(&replay).is_empty(), "the replay notified: {:?}", notices(&replay));
    Ok(Outcome::Pass)
}

/// An agent profile's table as a user writes it, after the test config's
/// profiles; the agent never runs.
const AGENT_TABLE: &str = r#"# my agent, kept as written
[[profiles]]
name = "Agent" # the menu's name
type = "agent"
command = "agent-acp"
args = ["--acp"] # its ACP mode
env = { AGENT_LOG = "debug" }
"#;

/// The case's config file as the last run left it.
fn config_file(ctx: &Ctx) -> Result<String> {
    std::fs::read_to_string(ctx.dir.join("config.toml")).map_err(|e| format!("reading the config: {e}"))
}

/// The settings dialog's Profiles section with an agent profile:
/// - it is listed last, marked (`agent=["Agent"]`), and Edit and Duplicate
///   are refused with the config-file note, writing nothing;
/// - saving another profile (`Shell` renamed `Login`) leaves its table byte
///   for byte;
/// - alt-up moves it above `Shell`, whole, still selected.
pub fn settings_profiles(ctx: &Ctx) -> Result<Outcome> {
    use super::step_time;
    let extra = format!("\n{AGENT_TABLE}");
    let written = crate::brindle::config("e2e") + &extra;
    let settings = |out: &crate::brindle::Output| out.dump.settings.clone().unwrap_or_default();

    let out = Run::new(ctx, "list")
        .config(&extra)
        .action("open_settings")
        .action("settings_last")
        .action("settings_edit")
        .action("settings_duplicate")
        .dump_at(step_time(4) + 0.8)
        .run()?;
    let line = settings(&out);
    ensure!(line.starts_with("section=Profiles mode=list selected=\"Agent\""), "the dialog: {line:?}");
    ensure!(line.contains("agent=[\"Agent\"]"), "Agent isn't marked: {line:?}");
    ensure!(line.ends_with("error=Some(\"Agent profiles are edited in the config file\")"), "the refusal: {line:?}");
    ensure!(config_file(ctx)? == written, "the refused edits wrote:\n{}", config_file(ctx)?);

    let out = Run::new(ctx, "save")
        .config(&extra)
        .action("open_settings")
        .action("settings_edit")
        .action("text_input_select_home")
        .send("Login")
        .action("settings_confirm")
        .dump_at(step_time(5) + 0.8)
        .run()?;
    let line = settings(&out);
    ensure!(line.starts_with("section=Profiles mode=list selected=\"Login\""), "after the save: {line:?}");
    let config = config_file(ctx)?;
    ensure!(config.contains("name = \"Login\"") && !config.contains("name = \"Shell\""), "Shell wasn't saved:\n{config}");
    ensure!(config.matches(AGENT_TABLE).count() == 1, "the agent's table changed:\n{config}");

    let out = Run::new(ctx, "move")
        .config(&extra)
        .action("open_settings")
        .action("settings_last")
        .action("settings_move_up")
        .dump_at(step_time(3) + 0.8)
        .run()?;
    let line = settings(&out);
    ensure!(line.starts_with("section=Profiles mode=list selected=\"Agent\""), "after the move: {line:?}");
    let config = config_file(ctx)?;
    ensure!(config.matches(AGENT_TABLE).count() == 1, "the agent's table changed:\n{config}");
    let at = |name: &str| config.find(&format!("name = \"{name}\""));
    ensure!(at("Agent") < at("Shell") && at("Shell").is_some(), "Agent isn't above Shell:\n{config}");
    Ok(Outcome::Pass)
}

/// What the case sets `BRINDLE_TEST_SINK_PANIC` to, and the agent says.
const SINK_PANIC: &str = "brindle-test-agent-panic";

/// Brindle failing on an agent session's events (the fault switch
/// `BRINDLE_TEST_SINK_PANIC`) lets go of the session: the zone stays and
/// says it can't show the session (`state=broken`), and the session keeps
/// running in the server.
pub fn broken(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("received.jsonl");
    let profile = Script::ready(&received)
        .step("expect session/prompt")
        .step(says(SINK_PANIC))
        .step(stop("end_turn"))
        .step(format!("wait-file {}", gate(ctx, "never").display()))
        .profile(ctx, "broken")?;
    let out = Run::new(ctx, "run")
        .config(&profile)
        .agent("Fake")
        .first_prompt("hi")
        .keep_detached()
        .env("BRINDLE_TEST_SINK_PANIC", SINK_PANIC)
        .wait_for("state=broken")
        .dump_at(6.0)
        .run()?;
    waited(&out)?;
    ensure!(out.dump.tabs.len() == 1, "the zone stays: {:?}", out.dump.tabs);
    // Let go, not ended: still running in the server after the quit.
    let listed = crate::brindle::listed_sessions(ctx)?;
    ensure!(listed.len() == 1 && !listed[0].1, "the agent session isn't listed detached: {listed:?}");
    crate::brindle::hang_up_detached(ctx)?;
    Ok(Outcome::Pass)
}
