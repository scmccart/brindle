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
pub(super) fn says(text: &str) -> String {
    let text = text.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace('\t', "\\t");
    format!(
        r#"send {{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"s1","update":{{"sessionUpdate":"agent_message_chunk","content":{{"type":"text","text":"{text}"}}}}}}}}"#
    )
}

/// A fake agent's script, built step by step and written into the case
/// directory.
pub(super) struct Script(Vec<String>);

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

    pub(super) fn step(mut self, step: impl Into<String>) -> Self {
        self.0.push(step.into());
        self
    }

    /// Waits for a prompt, then holds its turn until `gate` exists.
    fn prompt_held(self, gate: &Path) -> Self {
        self.step("expect session/prompt").step(format!("wait-file {}", gate.display()))
    }

    /// An agent as [`Script::ready`] whose `initialize` offers
    /// `capabilities` (its `agentCapabilities`, [`RESUME`] or another), so
    /// its conversation `s1` can be continued unless it offers [`NEITHER`].
    fn offering(received: &Path, capabilities: &str) -> Self {
        Script::initialized(received, capabilities).step("expect session/new").step(r#"respond {"sessionId":"s1"}"#)
    }

    /// An agent that records what it is sent in `received` and answers
    /// `initialize` offering `capabilities`; the case adds the rest (a
    /// continue's `session/resume` or `session/load`).
    pub(super) fn initialized(received: &Path, capabilities: &str) -> Self {
        Script(vec![
            format!("record {}", received.display()),
            "expect initialize".into(),
            format!(
                r#"respond {{"protocolVersion":1,"agentInfo":{{"name":"Fake Agent","version":"1.0"}},"agentCapabilities":{capabilities}}}"#
            ),
        ])
    }

    /// Writes the script as `NAME.script` in the case directory.
    pub(super) fn write(self, ctx: &Ctx, name: &str) -> Result<PathBuf> {
        let path = ctx.dir.join(format!("{name}.script"));
        std::fs::write(&path, self.0.join("\n") + "\n").map_err(|e| format!("writing {}: {e}", path.display()))?;
        Ok(path)
    }

    /// Writes the script as `NAME.script` in the case directory and
    /// returns the `Fake` profile running it.
    pub(super) fn profile(self, ctx: &Ctx, name: &str) -> Result<String> {
        Ok(fake_profile(ctx, &self.write(ctx, name)?))
    }
}

/// A file in the case directory a fake agent's `wait-file` waits for,
/// which the case creates; it is never created when it holds a turn for
/// the whole run.
pub(super) fn gate(ctx: &Ctx, name: &str) -> PathBuf {
    ctx.dir.join(name)
}

/// What the agent zone's `--wait-for` text shows while a turn runs.
const WORKING: &str = "turn=working";

/// Waits until the case server lists `count` sessions, which shows that
/// closed agents ended rather than being left detached.
pub(super) fn sessions_listed(ctx: &Ctx, count: usize) -> Result {
    ctx.scale().eventually(10.0, &format!("{count} listed sessions"), || {
        crate::brindle::listed_sessions(ctx).is_ok_and(|listed| listed.len() == count)
    })
}

/// What `--wait-for` waits for until the agent is ready.
pub(super) const READY: &str = "state=ready";

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
/// zone and tab ones, and none that acts on a terminal; a terminal's
/// palette lists none of the agent's (Copy Last Code Block, from
/// `agents-markdown`, included).
pub fn palette_commands(ctx: &Ctx) -> Result<Outcome> {
    let (profile, _) = hello_agent(ctx)?;
    let listed = |out: &crate::brindle::Output, label: &str| {
        out.dump.palette.iter().skip(1).any(|row| row.starts_with(&format!("{label}  ")) || row == label)
    };
    let out = Run::new(ctx, "palette").config(&profile).agent("Fake").wait_for(READY).action("open_command_palette").dump_at(4.0).run()?;
    let agent = ["Stop Agent Turn", "Copy Last Agent Message", "Copy Last Code Block", "Scroll Agent to Bottom"];
    for label in agent.iter().chain(&["Split Agent Right", "Split Terminal Right", "Close Tab"]) {
        ensure!(listed(&out, label), "{label} isn't listed: {:?}", out.dump.palette);
    }
    for label in ["Clear Scrollback", "Find", "Scroll to Bottom", "Copy"] {
        ensure!(!listed(&out, label), "{label} is listed: {:?}", out.dump.palette);
    }

    let out = Run::new(ctx, "terminal").config(&profile).prompt().action("open_command_palette").dump_at(4.0).run()?;
    ensure!(listed(&out, "Clear Scrollback"), "a terminal's palette: {:?}", out.dump.palette);
    for label in agent {
        ensure!(!listed(&out, label), "{label} is listed for a terminal: {:?}", out.dump.palette);
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
        .action("split_agent_right")
        .action("new_tab")
        .action("prev_tab")
        .action("close_tab")
        .dump_at(8.0)
        .run()?;
    ensure!(out.dump.confirm.as_deref() == Some("close target=tab:1 busy=1:working"), "confirm: {:?}", out.dump.confirm);
    let out = started("tab-closed")
        .keep_detached()
        .action("split_agent_right")
        .action("new_tab")
        .action("prev_tab")
        .action("close_tab")
        .key("tab")
        .key("enter")
        .dump_at(10.0)
        .run()?;
    ensure!(out.dump.confirm.is_none(), "the confirmation closed: {:?}", out.dump.confirm);
    ensure!(out.dump.tabs.len() == 1 && out.dump.tabs[0].0 != "Fake", "only the shell tab is left: {:?}", out.dump.tabs);
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
    ensure!(out.dump.tabs.len() == 1 && out.dump.tabs[0].0 != "Fake", "the agent tab closed: {:?}", out.dump.tabs);
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
    ensure!(out.dump.tabs == [("Fake".to_string(), true)], "tabs: {:?}", out.dump.tabs);

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
pub(super) fn stop(stop: &str) -> String {
    format!(r#"respond {{"stopReason":"{stop}"}}"#)
}

/// The prompts' texts the agent was sent, in order, from its record.
pub(super) fn prompts_sent(received: &Path) -> Result<Vec<String>> {
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
pub(super) fn recorded(received: &Path, method: &str) -> bool {
    let text = std::fs::read_to_string(received).unwrap_or_default();
    text.lines().any(|line| line.contains(&format!("\"method\":\"{method}\"")))
}

/// The steps that open `gate` (a fake agent's `wait-file`) from a Shell
/// tab, so it opens after the steps before: a new tab the first time
/// (`first`), tab 2 after that, its shell typing `touch`; then back to the
/// agent's tab, the first, whose prompt input takes focus again.
fn touch_from_shell<'a>(run: Run<'a>, gate: &Path, first: bool) -> Run<'a> {
    from_shell(run, &format!("touch '{}'", gate.display()), first)
}

/// The steps that run `command` from a Shell tab, as [`touch_from_shell`]
/// does, so it runs after the steps before.
pub(super) fn from_shell<'a>(run: Run<'a>, command: &str, first: bool) -> Run<'a> {
    let run = if first { run.action("new_tab") } else { run.action("activate_tab_2") };
    run.send(&format!("{command}\r")).action("activate_tab_1")
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
/// prompt reaches its buttons, reject first though the agent listed allow
/// first (`agents-zone-attention` decision 4), and enter on one answers
/// it: the agent gets the option chosen, and the turn goes on to its end.
/// The dump's entry line keeps the agent's order. The request names a call
/// the agent never sent, so the server adds its tool entry first, entry 2
/// (`agents-tool-content`), and the request is entry 3.
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
    ensure!(out.dump.focus_stop.as_deref() == Some("permission:3:reject-once"), "focus: {:?}", out.dump.focus_stop);

    std::fs::remove_file(&received).ok();
    let out = started("reject").key("tab").key("enter").wait_for("agent \"done\"").dump_at(7.0).run()?;
    waited(&out)?;
    agent_has(&out, "entry 3 permission call=\"call_1\" options=allow-once:\"Allow\"|reject-once:\"Reject\" outcome=selected:reject-once")?;
    ensure!(out.dump.agent_field("turn") == Some("ended:end_turn"), "agent: {:?}", out.dump.agent);
    let sent = std::fs::read_to_string(&received).map_err(|e| e.to_string())?;
    ensure!(
        sent.lines().any(|l| l.contains("\"id\":\"p1\"") && l.contains("\"optionId\":\"reject-once\"")),
        "the agent wasn't answered reject-once: {sent}"
    );
    Ok(Outcome::Pass)
}

/// The zone set's `card` token for the `e2e` theme, an agent zone's agent
/// bubble and an answered permission: `BACKGROUND` (the theme's own,
/// `Theme::zone_tokens`) mixed 4% towards the foreground.
const ZONE_CARD: crate::capture::Rgb = crate::capture::Rgb(0x17, 0x1b, 0x1f);

/// The zone set's `destructive_tint` for the `e2e` theme, a pending
/// permission's background: [`ZONE_CARD`] mixed the most, 12%, towards the
/// red (`RED`, 4.1:1 on the card, so `destructive` as it is), since the
/// foreground keeps 8.6:1 and the red 3.7:1 on it (`theme::destructive_tint`).
const ZONE_DESTRUCTIVE_TINT: crate::capture::Rgb = crate::capture::Rgb(0x2f, 0x1f, 0x23);

/// The working row and the spinners (OpenSpec change
/// `agents-zone-attention`, decisions 1 to 3): while a tool call is in
/// progress the zone's last line is `--- working "Read main.rs"`, the tab's
/// face spins and the motion clock runs; split, the working zone's header
/// spins too. Once the turn ends (the case opens the gate from a Shell
/// tab) there is no working line and no spinner, and the clock is stopped.
pub fn working(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("received.jsonl");
    let done = gate(ctx, "done");
    let tool = |status: &str| {
        update(&format!(
            r#"{{"sessionUpdate":"tool_call","toolCallId":"call_1","title":"Read main.rs","kind":"read","status":"{status}"}}"#
        ))
    };
    let profile = Script::ready(&received)
        .step("expect session/prompt")
        .step(tool("in_progress"))
        .step(format!("wait-file {}", done.display()))
        .step(update(r#"{"sessionUpdate":"tool_call_update","toolCallId":"call_1","status":"completed"}"#))
        .step(says("done"))
        .step(stop("end_turn"))
        .profile(ctx, "working")?;
    let in_progress = "working \"Read main.rs\"";
    let started = |label| Run::new(ctx, label).config(&profile).agent("Fake").first_prompt("hi").wait_for(in_progress);

    let out = started("working").dump_at(5.0).run()?;
    waited(&out)?;
    ensure!(out.dump.agent_working().as_deref() == Some("Read main.rs"), "working row: {:?}", out.dump.agent);
    ensure!(out.dump.agent.last().is_some_and(|l| l.starts_with("prompt ")), "agent lines {:?}", out.dump.agent);
    ensure!(out.dump.spinners == ["tab=1"], "spinners {:?}", out.dump.spinners);
    ensure!(out.dump.motion.as_deref() == Some("running"), "motion {:?}", out.dump.motion);

    // Split: the new zone is active and idle, the working one's header
    // spins.
    let out = started("split").action("split_agent_right").wait_for("conversation entries=0").dump_at(6.0).run()?;
    waited(&out)?;
    ensure!(out.dump.zones.as_deref() == Some("count=2 active=2"), "zones {:?}", out.dump.zones);
    ensure!(out.dump.spinners == ["tab=1", "zone=1"], "spinners {:?}", out.dump.spinners);
    ensure!(out.dump.motion.as_deref() == Some("running"), "motion {:?}", out.dump.motion);

    let out = started("ended")
        .action("new_tab")
        .send(&format!("touch '{}'\r", done.display()))
        .action("activate_tab_1")
        .wait_for("turn=ended:end_turn")
        .dump_at(8.0)
        .run()?;
    waited(&out)?;
    ensure!(out.dump.agent_working().is_none(), "a working row at the end: {:?}", out.dump.agent);
    ensure!(out.dump.spinners.is_empty(), "spinners at the end {:?}", out.dump.spinners);
    ensure!(out.dump.motion.as_deref() == Some("stopped"), "motion {:?}", out.dump.motion);
    Ok(Outcome::Pass)
}

/// A pending permission is drawn as a risk (OpenSpec change
/// `agents-zone-attention`, decision 4), in pixels: its row has the zone
/// set's destructive tint behind its title and the allow button's
/// destructive fill (the `e2e` theme's red); once allowed (tab, tab,
/// enter: reject comes first) the row is back on the card's colour.
pub fn permission_style(ctx: &Ctx) -> Result<Outcome> {
    use crate::brindle::theme;
    let profile = Script::ready(&ctx.dir.join("received.jsonl"))
        .step("expect session/prompt")
        .step(ask("p1"))
        .step(r#"expect-response "p1""#)
        .step(format!("wait-file {}", gate(ctx, "never").display()))
        .profile(ctx, "style")?;
    let started =
        |label| Run::new(ctx, label).config(&profile).agent("Fake").first_prompt("hi").wait_for("outcome=pending");

    let out = started("pending").capture_at(5.0).dump_at(5.5).run()?;
    waited(&out)?;
    let image = out.image.as_ref().ok_or("no capture")?;
    let scale = window_scale(image, theme::CHROME_BORDER)?;
    let zone = zone_inner(image, scale, [0.0, 0.0, 1.0, 1.0]);
    let area = scale * scale;
    // The title row is the zone's width less its paddings by some 30
    // logical pixels; the allow button about 60 by 28.
    let tint = image.count_near(zone, ZONE_DESTRUCTIVE_TINT, TOKEN_NEAR);
    ensure!(
        tint as f32 >= 2_000.0 * area,
        "the tint: {tint} pixels of {} in {zone:?}; found {}",
        ZONE_DESTRUCTIVE_TINT.hex(),
        image.summary(zone)
    );
    let fill = image.count_near(zone, theme::RED, TOKEN_NEAR);
    ensure!(
        fill as f32 >= 500.0 * area,
        "the allow button's fill: {fill} pixels of {} in {zone:?}; found {}",
        theme::RED.hex(),
        image.summary(zone)
    );

    let out = started("answered")
        .key("tab")
        .key("tab")
        .key("enter")
        .wait_for("outcome=selected:allow-once")
        .capture_at(7.0)
        .dump_at(7.5)
        .run()?;
    waited(&out)?;
    let image = out.image.as_ref().ok_or("no capture")?;
    let scale = window_scale(image, theme::CHROME_BORDER)?;
    let zone = zone_inner(image, scale, [0.0, 0.0, 1.0, 1.0]);
    // Antialiased red edges pass through the tint's colour; a row of it
    // doesn't.
    let tint = image.count_near(zone, ZONE_DESTRUCTIVE_TINT, TOKEN_NEAR);
    ensure!(
        (tint as f32) < 100.0 * scale * scale,
        "answered, {tint} pixels of the tint {} remain in {zone:?}; found {}",
        ZONE_DESTRUCTIVE_TINT.hex(),
        image.summary(zone)
    );
    let card = image.count_near(zone, ZONE_CARD, TOKEN_NEAR);
    ensure!(card as f32 >= 2_000.0 * scale * scale, "the card: {card} pixels of {}; found {}", ZONE_CARD.hex(), image.summary(zone));
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
/// (`at=away`), and Scroll Agent to Bottom returns to it. The lines are
/// paragraphs: a single line break is a space in Markdown, which would
/// make them one short paragraph.
pub fn scroll(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("received.jsonl");
    let long: Vec<String> = (1..=200).map(|n| format!("line {n}")).collect();
    let profile = Script::ready(&received)
        .step("expect session/prompt")
        .step(says(&long.join("\n\n")))
        .step(says("\n\nthe end"))
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

/// The steps of a long conversation's turn (OpenSpec change
/// `agents-log-paging` task 6.1): after the prompt, the conversation
/// titled `title` when there is one, then `entries` chunks alternating a
/// thought of `size` bytes and a short agent message, each a new entry, and
/// the turn's end. The thoughts carry the bytes, so a page holds many
/// entries but draws few lines (a closed thought is one row), and
/// shift-pageup reaches each page's top in a few presses.
fn long_turn(entries: usize, size: usize, title: Option<&str>) -> Vec<String> {
    let mut steps = vec!["expect session/prompt".to_string()];
    steps.extend(title.map(|title| titles(Some(title))));
    for n in 0..entries {
        steps.push(if n % 2 == 0 {
            let head = format!("thought {n} ");
            let text = head.clone() + &"lorem ipsum ".repeat(size.saturating_sub(head.len()) / 12);
            update(&format!(r#"{{"sessionUpdate":"agent_thought_chunk","content":{{"type":"text","text":"{text}"}}}}"#))
        } else {
            says(&format!("message {n}"))
        });
    }
    steps.push(stop("end_turn"));
    steps
}

/// The `Fake` profile whose agent answers the first prompt with
/// [`long_turn`], written as `NAME.script`.
fn long_conversation(ctx: &Ctx, name: &str, entries: usize, size: usize, title: Option<&str>) -> Result<String> {
    long_turn(entries, size, title)
        .into_iter()
        .fold(Script::ready(&ctx.dir.join(format!("{name}.jsonl"))), Script::step)
        .profile(ctx, name)
}

/// The long conversation of the `agent-earlier-*` cases: about 1 MiB in
/// 400 entries after the prompt, so an attach's 256 KiB page leaves most of
/// it to fetch.
const LONG_ENTRIES: u64 = 400;
const LONG_SIZE: usize = 5 * 1024;
const LONG_TITLE: &str = "A long conversation";

/// The shift-pageup presses that take the `agent-earlier-*` conversation
/// from its end to its start, fetching each page as its row shows: a page
/// draws about 100 one-row entries.
const PAGE_UPS: usize = 60;

/// [`long_turn`]'s steps: the chunks alternate, each kind's text where it
/// should be, the title first, and the turn ends.
pub fn self_test_long_turn() -> Result {
    let steps = long_turn(6, 200, Some("T"));
    ensure!(steps.len() == 1 + 1 + 6 + 1, "{} steps", steps.len());
    ensure!(steps[0] == "expect session/prompt" && steps[1].contains("session_info_update"), "{:?}", &steps[..2]);
    let chunks = &steps[2..8];
    for (n, step) in chunks.iter().enumerate() {
        let kind = if n % 2 == 0 { "agent_thought_chunk" } else { "agent_message_chunk" };
        ensure!(step.starts_with("send ") && step.contains(kind), "chunk {n}: {step}");
        let value: serde_json::Value = serde_json::from_str(&step[5..]).map_err(|e| format!("chunk {n}: {e}"))?;
        let text = value["params"]["update"]["content"]["text"].as_str().unwrap_or_default();
        ensure!(text.starts_with(&format!("{} {n}", if n % 2 == 0 { "thought" } else { "message" })), "chunk {n}: {text}");
        ensure!(n % 2 == 1 || (190..=200).contains(&text.len()), "thought {n} is {} bytes", text.len());
    }
    ensure!(steps[8] == stop("end_turn"), "{}", steps[8]);
    ensure!(long_turn(3, 10, None).len() == 1 + 3 + 1, "untitled");
    Ok(())
}

/// `n` shift-pageup presses.
fn page_ups(mut run: Run<'_>, n: usize) -> Run<'_> {
    for _ in 0..n {
        run = run.key("shift-pageup");
    }
    run
}

/// The ids of the dump's `--- entry` lines, in order.
fn entry_ids(out: &crate::brindle::Output) -> Vec<u64> {
    out.dump.agent.iter().filter_map(|line| line.strip_prefix("entry ")?.split(' ').next()?.parse().ok()).collect()
}

/// The `--- conversation` line's `earlier=` value.
fn earlier(out: &crate::brindle::Output) -> Option<&str> {
    let line = out.dump.agent.iter().find_map(|line| line.strip_prefix("conversation "))?;
    line.split(' ').find_map(|f| f.strip_prefix("earlier="))
}

/// The ids from `first` to the conversation's last entry, each once and
/// in order: the zone holds one stretch of it.
fn held_through_the_end(out: &crate::brindle::Output, first: u64) -> Result {
    let ids = entry_ids(out);
    let want: Vec<u64> = (first..=LONG_ENTRIES + 1).collect();
    ensure!(ids == want, "entry ids {:?}…{:?} ({} of them), not {first}..={}", ids.first(), ids.last(), ids.len(), LONG_ENTRIES + 1);
    Ok(())
}

/// Runs the long conversation's turn live and leaves it detached, its
/// tab recorded for the next run to restore.
fn long_turn_detached(ctx: &Ctx, profile: &str) -> Result {
    let out = Run::new(ctx, "live")
        .config(profile)
        .agent("Fake")
        .first_prompt("go")
        .keep_detached()
        .wait_for(ENDED)
        .dump_at(8.0)
        .run()?;
    waited(&out)?;
    ensure!(earlier(&out) == Some("none"), "a live zone holds the whole conversation: {:?}", out.dump.agent.last());
    held_through_the_end(&out, 1)
}

/// An attach of a long conversation (OpenSpec change `agents-log-paging`,
/// task 6.2): the restored zone holds the newest page, from an entry past
/// the first through the last, the conversation line counting the entries
/// before it, and the tab is titled with the conversation's title, which
/// every page carries.
pub fn earlier_page(ctx: &Ctx) -> Result<Outcome> {
    let profile = long_conversation(ctx, "long", LONG_ENTRIES as usize, LONG_SIZE, Some(LONG_TITLE))?;
    long_turn_detached(ctx, &profile)?;
    let out = Run::new(ctx, "attached").config(&profile).restore().wait_for(ENDED).dump_at(6.0).run()?;
    waited(&out)?;
    let first = entry_ids(&out).first().copied().ok_or("no entries")?;
    ensure!(first > 1, "the whole conversation was sent: the first entry is {first}");
    held_through_the_end(&out, first)?;
    ensure!(earlier(&out) == Some((first - 1).to_string().as_str()), "earlier={:?} with entry {first} first", earlier(&out));
    ensure!(out.dump.tabs.first().map(|(title, _)| title.as_str()) == Some(LONG_TITLE), "tabs: {:?}", out.dump.tabs);
    Ok(Outcome::Pass)
}

/// Scrolling an attached long conversation to its start (task 6.3):
/// shift-pageup reaches the Earlier row, each page goes in front, and the
/// zone ends up holding every entry with its parts, away from the end,
/// the motion clock stopped once nothing loads and nothing notified.
pub fn earlier_scroll(ctx: &Ctx) -> Result<Outcome> {
    let profile = long_conversation(ctx, "long", LONG_ENTRIES as usize, LONG_SIZE, Some(LONG_TITLE))?;
    long_turn_detached(ctx, &profile)?;
    let log = ctx.dir.join("scroll.notify");
    let run = Run::new(ctx, "scroll")
        .config(&profile)
        .config("\n[notifications]\nwhen = \"always\"\n")
        .env("BRINDLE_NOTIFY_LOG", log.to_str().ok_or("the log's path isn't UTF-8")?)
        .restore()
        .step_interval(0.2)
        .wait_for(ENDED);
    let out = page_ups(run, PAGE_UPS).wait_for("earlier=none").dump_at(30.0).run()?;
    waited(&out)?;
    held_through_the_end(&out, 1)?;
    for entry in 2..=LONG_ENTRIES + 1 {
        ensure!(!out.dump.agent_blocks(entry).is_empty(), "entry {entry} has no parts");
    }
    agent_has(&out, "conversation entries=401 at=away earlier=none")?;
    ensure!(out.dump.motion.as_deref() == Some("stopped"), "motion {:?}", out.dump.motion);
    ensure!(notices(&log).is_empty(), "fetched pages notified: {:?}", notices(&log));
    Ok(Outcome::Pass)
}

/// A long conversation attached again after it was fetched to its start
/// (task 6.4): each attach starts from the newest page, with as many
/// entries before it as the first, and scrolling up fetches them again.
pub fn earlier_reattach(ctx: &Ctx) -> Result<Outcome> {
    let profile = long_conversation(ctx, "long", LONG_ENTRIES as usize, LONG_SIZE, Some(LONG_TITLE))?;
    long_turn_detached(ctx, &profile)?;
    let out = Run::new(ctx, "first").config(&profile).restore().keep_detached().wait_for(ENDED).dump_at(6.0).run()?;
    waited(&out)?;
    let count = earlier(&out).ok_or("no conversation line")?.to_string();
    ensure!(count.parse::<u64>().is_ok_and(|n| n > 0), "the first attach's earlier={count}");
    let scrolled = |label| {
        let run = Run::new(ctx, label).config(&profile).restore().keep_detached().step_interval(0.2);
        let run = run.wait_for(ENDED).wait_for(&format!("earlier={count}"));
        page_ups(run, PAGE_UPS).wait_for("earlier=none").dump_at(30.0).run()
    };
    for label in ["fetched", "again"] {
        let out = scrolled(label)?;
        waited(&out)?;
        held_through_the_end(&out, 1)?;
    }
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

/// What `--wait-for` waits for until a turn has ended normally.
const ENDED: &str = "turn=ended:end_turn";

/// An agent profile `Fake` whose agent answers the first prompt with
/// `text`, in one chunk, and ends the turn; its script is `NAME.script`
/// and its record `NAME.jsonl`.
pub(super) fn answering(ctx: &Ctx, name: &str, text: &str) -> Result<String> {
    Script::ready(&ctx.dir.join(format!("{name}.jsonl")))
        .step("expect session/prompt")
        .step(says(text))
        .step(stop("end_turn"))
        .profile(ctx, name)
}

/// A part as [`Dump::agent_blocks`](crate::brindle::Dump::agent_blocks)
/// gives it: `part=P kind=K "TEXT"`.
pub(super) fn part(part: usize, kind: &str, text: &str) -> String {
    format!("part={part} kind={kind} {text:?}")
}

/// A link as [`Dump::agent_links`](crate::brindle::Dump::agent_links)
/// gives it: `part=P target="…" text="…" action=A`.
pub(super) fn link(part: usize, target: &str, text: &str, action: &str) -> String {
    format!("part={part} target={target:?} text={text:?} action={action}")
}

/// An agent message using every block kind and inline style the renderer
/// knows, for `agent-markdown-render`.
const EVERY_KIND: &str = "# Title\n\n\
    Some *em*, **strong**, ~~gone~~ and `code`.\n\n\
    - one\n- two\n\n\
    1. first\n2. second\n\n\
    - [ ] todo\n- [x] done\n\n\
    > quoted\n\n\
    ---\n\n\
    | L | C | R |\n|:--|:-:|--:|\n| a | b | c |\n\n\
    Inline <b>bold</b> html.\n\n\
    <div>\nblock\n</div>\n\n\
    ![chart](https://example.com/c.png)\n\n\
    ```rust\nfn main() {}\n```\n";

/// An agent message in Markdown (design.md decision 11): the dump prints
/// a `--- block` line per part after its entry, in order, with its kind
/// and shown text (every block kind, inline styles dropped, HTML as
/// written, an image as its text), and a thought's parts while it is
/// closed; the entry lines keep the source, and a user's `**hi**` has no
/// parts.
pub fn markdown_render(ctx: &Ctx) -> Result<Outcome> {
    let thought = "- a\n- b";
    let profile = Script::ready(&ctx.dir.join("received.jsonl"))
        .step("expect session/prompt")
        .step(says(EVERY_KIND))
        .step(update(&format!(
            r#"{{"sessionUpdate":"agent_thought_chunk","content":{{"type":"text","text":"{}"}}}}"#,
            thought.replace('\n', "\\n")
        )))
        .step(stop("end_turn"))
        .profile(ctx, "render")?;
    let out = Run::new(ctx, "run").config(&profile).agent("Fake").first_prompt("**hi**").wait_for(ENDED).dump_at(5.0).run()?;
    waited(&out)?;
    let entries = [r#"user "**hi**""#.to_string(), format!("agent {EVERY_KIND:?}"), format!("thought open=false {thought:?}")];
    ensure!(out.dump.agent_entries() == entries, "entries {:?}", out.dump.agent);
    ensure!(out.dump.agent_blocks(1).is_empty(), "the user's message has parts: {:?}", out.dump.agent);
    let image = "[image: chart] https://example.com/c.png";
    let parts = [
        part(0, "heading:1", "Title"),
        part(1, "paragraph", "Some em, strong, gone and code."),
        part(2, "list", "- one\n- two"),
        part(3, "list", "1. first\n2. second"),
        part(4, "list", "- [ ] todo\n- [x] done"),
        part(5, "quote", "> quoted"),
        part(6, "rule", ""),
        part(7, "table", "L | C | R\na | b | c"),
        part(8, "paragraph", "Inline <b>bold</b> html."),
        part(9, "paragraph", "<div>\nblock\n</div>"),
        part(10, "paragraph", image),
        part(11, "code:rust", "fn main() {}\n"),
    ];
    ensure!(out.dump.agent_blocks(2) == parts, "the message's parts {:#?}", out.dump.agent_blocks(2));
    let links = [link(10, "https://example.com/c.png", image, "open")];
    ensure!(out.dump.agent_links(2) == links, "the message's links {:?}", out.dump.agent_links(2));
    ensure!(out.dump.agent_blocks(3) == [part(0, "list", thought)], "the thought's parts {:?}", out.dump.agent);
    Ok(Outcome::Pass)
}

/// The fake agent's script for `agent-markdown-stream`'s run `label`: a
/// paragraph and an opening fence with two lines, then, once the gate
/// `LABEL-fence` exists, the closing fence and a paragraph using a
/// reference link not yet defined, then, once `LABEL-definition` exists
/// (unless `define` is false), its definition and the turn's end.
fn stream_profile(ctx: &Ctx, label: &str, define: bool) -> Result<String> {
    let script = Script::ready(&ctx.dir.join(format!("{label}.jsonl")))
        .step("expect session/prompt")
        .step(says("Intro.\n\n```rust\nfn a() {}\nfn b() {}\n"))
        .step(format!("wait-file {}", gate(ctx, &format!("{label}-fence")).display()))
        .step(says("```\n\nSee [docs][d].\n\n"));
    let script = if define { script } else { script.step(format!("wait-file {}", gate(ctx, "never").display())) };
    script.step(says("[d]: https://example.com\n")).step(stop("end_turn")).profile(ctx, label)
}

/// A message streamed in chunks (design.md decision 5): an unclosed
/// fence is already a `code:rust` part while the agent holds the rest
/// (the `--wait-for` before the gate opens); the closing fence keeps it
/// one part, not cut, and the paragraph after it is the next part; a
/// reference link shows as written until its definition arrives, and
/// then as a link.
pub fn markdown_stream(ctx: &Ctx) -> Result<Outcome> {
    let code = part(1, "code:rust", "fn a() {}\nfn b() {}\n");
    for (label, define) in [("held", false), ("defined", true)] {
        let profile = stream_profile(ctx, label, define)?;
        let run = Run::new(ctx, label).config(&profile).agent("Fake").first_prompt("hi").wait_for("kind=code:rust");
        let run = touch_from_shell(run, &gate(ctx, &format!("{label}-fence")), true);
        let out = if define {
            run.wait_for(ENDED).dump_at(9.0).run()?
        } else {
            run.wait_for(r#"kind=paragraph "See [docs][d].""#).dump_at(9.0).run()?
        };
        waited(&out)?;
        let (paragraph, links) = if define {
            (part(2, "paragraph", "See docs."), vec![link(2, "https://example.com", "docs", "open")])
        } else {
            (part(2, "paragraph", "See [docs][d]."), Vec::new())
        };
        let parts = [part(0, "paragraph", "Intro."), code.clone(), paragraph];
        ensure!(out.dump.agent_blocks(2) == parts, "{label}: parts {:#?}", out.dump.agent_blocks(2));
        ensure!(out.dump.agent_links(2) == links, "{label}: links {:?}", out.dump.agent_links(2));
        let turn = if define { "ended:end_turn" } else { "working" };
        ensure!(out.dump.agent_field("turn") == Some(turn), "{label}: agent {:?}", out.dump.agent);
    }
    Ok(Outcome::Pass)
}

/// Copy Last Code Block puts the last code block of the agent's last
/// message on the clipboard, its text as the agent wrote it, and Copy Last
/// Agent Message still copies the message's Markdown source; a paste into
/// the prompt input shows each.
pub fn markdown_copy(ctx: &Ctx) -> Result<Outcome> {
    let answer = "First:\n\n```sh\necho one\n```\n\nThen:\n\n```\necho two\n```\n";
    let profile = answering(ctx, "copy", answer)?;
    for (label, action, pasted) in
        [("code", "copy_last_code_block", "echo two\n"), ("message", "copy_last_agent_message", answer)]
    {
        let out = Run::new(ctx, label)
            .config(&profile)
            .agent("Fake")
            .first_prompt("hi")
            .wait_for(ENDED)
            .action(action)
            .action("text_input_paste")
            .dump_at(6.0)
            .run()?;
        waited(&out)?;
        ensure!(input_text(&out) == Some(pasted), "{label}: input {:?}", out.dump.input);
    }
    Ok(Outcome::Pass)
}

/// An agent message's links (design.md decision 7), each in a `--- link`
/// line with its target, its text and what a ctrl-click does: `open` for
/// the web (inline, reference, bare and an image's), `confirm` for
/// `mailto:` and a `file:` link on this machine, `refuse:scheme` for
/// `javascript:` and `refuse:other-host` for another host's file; a URL in
/// code is no link.
pub fn markdown_links(ctx: &Ctx) -> Result<Outcome> {
    let answer = "See [inline](https://example.com/inline) and:\n\n\
        - [ref][r]\n\
        - bare https://example.com/bare\n\
        - <mailto:dev@example.com>\n\
        - [x](javascript:void)\n\
        - file://otherhost/share/x\n\
        - file:///tmp/x\n\
        - ![pic](https://example.com/p.png)\n\
        - `https://example.com/code`\n\n\
        [r]: https://example.com/ref\n";
    let profile = answering(ctx, "links", answer)?;
    let out = Run::new(ctx, "run").config(&profile).agent("Fake").first_prompt("hi").wait_for(ENDED).dump_at(5.0).run()?;
    waited(&out)?;
    let links = [
        link(0, "https://example.com/inline", "inline", "open"),
        link(1, "https://example.com/ref", "ref", "open"),
        link(1, "https://example.com/bare", "https://example.com/bare", "open"),
        link(1, "mailto:dev@example.com", "mailto:dev@example.com", "confirm"),
        link(1, "javascript:void", "x", "refuse:scheme"),
        link(1, "file://otherhost/share/x", "file://otherhost/share/x", "refuse:other-host"),
        link(1, "file:///tmp/x", "file:///tmp/x", "confirm"),
        link(1, "https://example.com/p.png", "[image: pic] https://example.com/p.png", "open"),
    ];
    ensure!(out.dump.agent_links(2) == links, "links {:#?}", out.dump.agent_links(2));
    ensure!(
        out.dump.agent_blocks(2).last().is_some_and(|list| list.ends_with(r#"\n- https://example.com/code""#)),
        "the code span's URL is shown: {:?}",
        out.dump.agent_blocks(2)
    );
    Ok(Outcome::Pass)
}

/// An agent zone's rem in the test config, in logical pixels: the default
/// font size, which the zone draws its components in.
const ZONE_REM: f32 = 14.0;

/// `agent-markdown-layout`'s paragraph: longer than the zone is wide.
const LONG_PARAGRAPH: &str = "This paragraph is much longer than the zone is wide, so it wraps over several lines \
    inside the bubble rather than widening it past the short line above, and the bubble's right edge stays \
    straight from that short line down to the code block under it, as one bubble across the conversation's \
    width should, however long each of its parts happens to be.";

/// A message's parts are one bubble and a code block's label one line
/// (OpenSpec change `agents-markdown-layout-fixes`): the agent answers
/// `Hi.`, a paragraph longer than the zone and a `rust` block; in the
/// capture, the bubble's right border is at one column on every row its
/// left border spans, one `px_3` inside the card's content edge, with
/// nothing of the border's colour right of it, and the code block's header
/// is at most two of the label's lines tall.
pub fn markdown_layout(ctx: &Ctx) -> Result<Outcome> {
    use crate::brindle::theme;
    let answer = format!("Hi.\n\n{LONG_PARAGRAPH}\n\n```rust\nlet x = 1;\n```\n");
    let profile = answering(ctx, "layout", &answer)?;
    let out = Run::new(ctx, "run")
        .config(&profile)
        .agent("Fake")
        .first_prompt("hi")
        .wait_for(ENDED)
        .capture_at(6.0)
        .dump_at(6.5)
        .run()?;
    waited(&out)?;
    let parts = [part(0, "paragraph", "Hi."), part(1, "paragraph", LONG_PARAGRAPH), part(2, "code:rust", "let x = 1;\n")];
    ensure!(out.dump.agent_blocks(2) == parts, "the message's parts {:#?}", out.dump.agent_blocks(2));
    let image = out.image.as_ref().ok_or("no capture")?;
    // The zone's rem is its font size, the default 14px; the label is
    // 0.75rem at gpui's default line height (the golden ratio).
    // The zone is drawn in the `e2e` theme's zone set: its border is the
    // window's, its `muted` the theme's own background's mix.
    let layout = measure_layout(image, theme::CHROME_BORDER, theme::ZONE_MUTED, ZONE_REM)?;
    let label_line = 0.75 * ZONE_REM * 1.618 * layout.scale;
    ensure!(
        layout.header as f32 <= 2.0 * label_line,
        "the code block's header is {} px, more than two label lines ({label_line:.1} px each): {layout:?}",
        layout.header
    );
    Ok(Outcome::Pass)
}

/// What [`markdown_layout`] reads from a capture of one agent zone, in
/// device pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Layout {
    /// Device pixels per logical one, from the gap right of the card.
    scale: f32,
    /// The card's content edges: the ends of its left and right borders.
    content: (u32, u32),
    /// The bubble's rows: its left border's run down the column `px_3`
    /// inside the card, between its rounded corners.
    rows: (u32, u32),
    /// The column of the bubble's right border on every one of `rows`.
    right: u32,
    /// A code block's header: from the end of its top border to the first
    /// row of its `muted` surface.
    header: u32,
}

/// Measures an agent zone's capture for [`markdown_layout`], in the
/// border colour `border` and the code's surface colour `muted`, with the
/// zone's rem `rem` logical pixels (its font size).
///
/// The card's borders are the first and last runs of `border` along the
/// middle row, and the scale is the gap right of the card over
/// `ZONE_GAP`. The bubble's left border is the longest run of `border` down
/// the column `px_3` (0.75rem) inside the card (give or take a pixel); its
/// right border must be one run over the same rows, `px_3` inside the
/// card's right edge (give or take a pixel), with no `border` right of it
/// on those rows. The code block's header runs from the last `border` run
/// above the longest `muted` run, down the column inside the block's left
/// border by 6px (past its rounded corner, short of the label's `pl_3`), to
/// that `muted` run.
pub(super) fn measure_layout(
    image: &crate::capture::Image,
    border: crate::capture::Rgb,
    muted: crate::capture::Rgb,
    rem: f32,
) -> Result<Layout> {
    const NEAR: u8 = 2;
    let (px_3, px_4) = (0.75 * rem, rem);
    let longest = |runs: Vec<(u32, u32)>| runs.into_iter().max_by_key(|&(a, b)| b - a);
    let mid = image.height / 2;
    let card = image.runs_in_row(mid, (0, image.width), border, NEAR);
    let (Some(&(_, left)), Some(&(right_border, after))) = (card.first(), card.last()) else {
        return Err(format!("no card border along row {mid}: {card:?}"));
    };
    ensure!(left < right_border, "one border run along row {mid}: {card:?}");
    let scale = (image.width - after) as f32 / super::ZONE_GAP;
    ensure!(scale > 0.0, "no gap right of the card (it ends at {after} of {})", image.width);
    let dev = |v: f32| (v * scale).round() as u32;
    let near = |x: u32| x.saturating_sub(1)..=x + 1;

    let at = left + dev(px_3);
    let rows = near(at)
        .filter_map(|x| longest(image.runs_in_column(x, (0, image.height), border, NEAR)))
        .max_by_key(|&(a, b)| b - a)
        .ok_or_else(|| format!("no bubble border down the columns round {at}"))?;
    ensure!(rows.1 - rows.0 > dev(40.0), "the bubble's left border {rows:?} is too short to be a message's");

    let expected = right_border - dev(px_3) - dev(1.0);
    let right = near(expected)
        .find(|&x| match image.runs_in_column(x, rows, border, NEAR)[..] {
            [(a, b)] => a <= rows.0 + 1 && b + 1 >= rows.1,
            _ => false,
        })
        .ok_or_else(|| {
            let found: Vec<_> = near(expected).map(|x| (x, image.runs_in_column(x, rows, border, NEAR))).collect();
            format!("the bubble's right border isn't one run over its rows {rows:?} round column {expected}: {found:?}")
        })?;
    for y in rows.0..rows.1 {
        let past = image.runs_in_row(y, (right + 2, right_border), border, NEAR);
        ensure!(past.is_empty(), "row {y} has the border's colour right of the bubble (column {right}): {past:?}");
    }

    // The bubble's border, its px_4, the block's border, then 6px.
    let column = left + dev(px_3 + 1.0 + px_4 + 1.0 + 6.0);
    let surface = longest(image.runs_in_column(column, rows, muted, NEAR))
        .ok_or_else(|| format!("no code surface down column {column} over {rows:?}"))?;
    let top = image
        .runs_in_column(column, (rows.0, surface.0), border, NEAR)
        .last()
        .copied()
        .ok_or_else(|| format!("no code block border above its surface {surface:?} down column {column}"))?;
    Ok(Layout { scale, content: (left, right_border), rows, right, header: surface.0 - top.1 })
}

/// [`measure_layout`] on painted captures: one bubble with straight edges
/// and a code block, and a jagged one.
pub fn self_test_layout() -> Result {
    use crate::capture::{Image, Rgb};
    const BG: Rgb = Rgb(0x10, 0x14, 0x18);
    const GAP: Rgb = Rgb(0x0a, 0x0d, 0x10);
    const LINE: Rgb = Rgb(0x25, 0x29, 0x2c);
    const MUTED: Rgb = Rgb(0x19, 0x1b, 0x1e);
    // A 200 × 120 window: the card's borders at columns 6 and 193 (a 6px
    // gap each side), the bubble's at 19 and 180 over rows 20..100, a code
    // block's top border at row 60 over columns 36..170 and its surface
    // over rows 70..90.
    let paint = |jagged: bool| {
        move |x: u32, y: u32| {
            let right = if jagged && y < 50 { 100 } else { 180 };
            let card = x == 6 || x == 193;
            let bubble = (20..100).contains(&y) && (x == 19 || x == right);
            let code_top = y == 60 && (36..170).contains(&x);
            if !(6..=193).contains(&x) {
                GAP
            } else if card || bubble || code_top {
                LINE
            } else if (70..90).contains(&y) && (36..170).contains(&x) {
                MUTED
            } else {
                BG
            }
        }
    };
    // At a 16px rem, px_3 is 12px: the borders at 7 + 12 and 193 - 12 - 1.
    let got = measure_layout(&Image::painted(200, 120, paint(false)), LINE, MUTED, 16.0)?;
    let want = Layout { scale: 1.0, content: (7, 193), rows: (20, 100), right: 180, header: 9 };
    ensure!(got == want, "one bubble: {got:?}, want {want:?}");
    let jagged = measure_layout(&Image::painted(200, 120, paint(true)), LINE, MUTED, 16.0);
    ensure!(jagged.is_err(), "a jagged bubble measured {jagged:?}");
    Ok(())
}

/// `agent-zone-theme`'s profile theme: `e2e-alt`'s foreground and
/// background with an accent of its own (`ZONE_ACCENT`; `e2e-alt` shares
/// `e2e`'s, which would hide a primary drawn in the window's), set on the
/// `Fake` profile `profile` ends with.
fn zone_theme_config(profile: &str) -> String {
    use super::zones::ZONE_ACCENT;
    use crate::brindle::theme::{ALT_BACKGROUND, ALT_FOREGROUND};
    format!(
        "{profile}theme = \"e2e-agent\"\n\n[themes.e2e-agent]\nforeground = \"{}\"\nbackground = \"{}\"\naccent = \"{}\"\n",
        ALT_FOREGROUND.hex(),
        ALT_BACKGROUND.hex(),
        ZONE_ACCENT.hex()
    )
}

/// How close a pixel must be to a token's colour to count as it here:
/// the two sets' surfaces are closer than [`Image::count`]'s 24 apart.
///
/// [`Image::count`]: crate::capture::Image::count
const TOKEN_NEAR: u8 = 3;

/// The spec's "A profile theme unlike the window's" (`agents-ui`) and
/// "Tooltips and overlays in a theme scope" (`ui-components`), in pixels
/// (OpenSpec change `agents-zone-theme-scope`): an agent zone whose profile
/// theme isn't the window's draws its components in that theme's zone
/// set. Its agent bubble and permission (`card`), code block and the
/// person's bubble (`muted`) and allow button (the profile's accent) show
/// inside the zone, and the window set's `card` and `muted` and the global
/// accent don't, while the strip keeps the window's background. Then, split
/// and with the zone header's menu open, no pixel of the window set's
/// `popover` is in either zone, and the zone set's is in the active one.
pub fn zone_theme(ctx: &Ctx) -> Result<Outcome> {
    use super::zones::ZONE_ACCENT;
    use crate::brindle::theme;
    let profile = Script::ready(&ctx.dir.join("theme.jsonl"))
        .step("expect session/prompt")
        .step(says("Here:\n\n```rust\nlet x = 1;\n```\n"))
        .step(ask("p1"))
        .profile(ctx, "theme")?;
    let config = zone_theme_config(&profile);
    let pending = "outcome=pending";
    let started = |label| Run::new(ctx, label).config(&config).agent("Fake").first_prompt("hi").wait_for(pending);

    let out = started("zone").capture_at(5.0).dump_at(5.5).run()?;
    waited(&out)?;
    let image = out.image.as_ref().ok_or("no capture")?;
    let scale = window_scale(image, theme::ALT_CARD_BORDER)?;
    let zone = zone_inner(image, scale, [0.0, 0.0, 1.0, 1.0]);
    let present = [
        ("the zone set's card", theme::ALT_ZONE_CARD),
        ("the zone set's muted", theme::ALT_ZONE_MUTED),
        ("the profile's accent", ZONE_ACCENT),
    ];
    for (what, color) in present {
        let n = image.count_near(zone, color, TOKEN_NEAR);
        ensure!(n >= 100, "{what}: {n} pixels of {} in the zone {zone:?}; found {}", color.hex(), image.summary(zone));
    }
    for (what, color) in [("the window set's card", theme::WINDOW_CARD), ("the window set's muted", theme::WINDOW_MUTED)] {
        let n = image.count_near(zone, color, TOKEN_NEAR);
        ensure!(n <= 2, "{what}: {n} pixels of {} in the zone {zone:?}; found {}", color.hex(), image.summary(zone));
    }
    image.expect_none("the window's accent in the zone", zone, theme::ACCENT)?;
    // The empty caption left of the window controls, as `tab-faces` reads it.
    let caption = {
        let right = image.width as f32 / scale - (8.0 + 3.0 * 32.0 + 2.0 * 4.0) - 30.0;
        logical_rect(scale, [right - 40.0, 12.0, 40.0, 14.0])
    };
    let got = image.dominant(caption);
    ensure!(
        got.is_some_and(|c| c.near(theme::CHROME_BACKGROUND, 2)),
        "the strip: dominant {:?} in {caption:?}, want the window's {}; found {}",
        got.map(crate::capture::Rgb::hex),
        theme::CHROME_BACKGROUND.hex(),
        image.summary(caption)
    );

    let out = started("menu")
        .action("split_agent_right")
        .wait_for("conversation entries=0")
        .wait_for(READY)
        .modality("pointer")
        .action("open_zone_menu")
        .capture_at(8.5)
        .dump_at(9.0)
        .run()?;
    waited(&out)?;
    ensure!(out.dump.zones.as_deref() == Some("count=2 active=2"), "zones {:?}", out.dump.zones);
    ensure!(!out.dump.menu.is_empty(), "no menu open: focus {:?}", out.dump.focus);
    let image = out.image.as_ref().ok_or("no capture")?;
    let scale = window_scale(image, theme::ALT_CARD_BORDER)?;
    for line in &out.dump.zone_list {
        let rect = parse_zone_rect(line).ok_or_else(|| format!("no rect in the zone line {line:?}"))?;
        let zone = zone_inner(image, scale, rect);
        let n = image.count_near(zone, theme::WINDOW_POPOVER, TOKEN_NEAR);
        ensure!(n <= 2, "the window set's popover: {n} pixels in zone {line:?} ({zone:?}); found {}", image.summary(zone));
        if line.ends_with(" active") {
            // The menu's panel is some 240 × 120 logical pixels (about
            // 20,000 of its colour at scale 1); the empty zone round it
            // shows next to none.
            let n = image.count_near(zone, theme::ALT_ZONE_POPOVER, TOKEN_NEAR);
            ensure!(n >= (10_000.0 * scale * scale) as usize,"the menu: {n} pixels of the zone set's popover in {line:?}; found {}", image.summary(zone));
        }
    }
    Ok(Outcome::Pass)
}

/// The window's scale, device pixels per logical one, from the gap right
/// of the rightmost card: the last run of `border` (the card's border)
/// along the middle row ends `ZONE_GAP` from the window's right edge.
pub(super) fn window_scale(image: &crate::capture::Image, border: crate::capture::Rgb) -> Result<f32> {
    let mid = image.height / 2;
    let runs = image.runs_in_row(mid, (0, image.width), border, 2);
    let &(_, after) = runs.last().ok_or_else(|| format!("no card border along row {mid}"))?;
    let scale = (image.width - after) as f32 / super::ZONE_GAP;
    ensure!(scale > 0.0, "no gap right of the card (it ends at {after} of {})", image.width);
    Ok(scale)
}

/// `[x, y, width, height]` logical pixels, in device pixels at `scale`.
fn logical_rect(scale: f32, [x, y, w, h]: [f32; 4]) -> crate::capture::PxRect {
    let px = |v: f32| (v * scale).round().max(0.0) as u32;
    crate::capture::PxRect { x0: px(x), y0: px(y), x1: px(x + w), y1: px(y + h) }
}

/// A zone's inside, in device pixels: its dump `rect` (`[x, y, width,
/// height]` in `[0, 1]` of the tab's area, the window below the strip)
/// less the gap, the card's border and 12px more, clear of the card's
/// 10px rounded corners and their edge blending.
pub(super) fn zone_inner(image: &crate::capture::Image, scale: f32, [x, y, w, h]: [f32; 4]) -> crate::capture::PxRect {
    use super::tabs::STRIP_HEIGHT;
    const INSET: f32 = super::CARD_INSET + 12.0;
    let (width, height) = (image.width as f32 / scale, image.height as f32 / scale - STRIP_HEIGHT);
    let (left, top) = (x * width, STRIP_HEIGHT + y * height);
    logical_rect(scale, [left + INSET, top + INSET, w * width - 2.0 * INSET, h * height - 2.0 * INSET])
}

/// A dump zone line's `rect=X,Y,W,H`, as `[x, y, width, height]`.
pub(super) fn parse_zone_rect(line: &str) -> Option<[f32; 4]> {
    let rect = line.split(' ').find_map(|word| word.strip_prefix("rect="))?;
    let values: Vec<f32> = rect.split(',').map(str::parse).collect::<std::result::Result<_, _>>().ok()?;
    values.try_into().ok()
}

/// [`window_scale`], [`zone_inner`] and [`parse_zone_rect`] on painted
/// captures and dump lines.
pub fn self_test_zone_theme() -> Result {
    use crate::capture::{Image, PxRect, Rgb};
    const GAP: Rgb = Rgb(0x0a, 0x0d, 0x10);
    const LINE: Rgb = Rgb(0x36, 0x2f, 0x2f);
    // A 400 × 300 window at scale 2: a card's borders at columns 12 and
    // 387, the gap 12 device pixels right of it.
    let painted = Image::painted(400, 300, |x, _| if x == 12 || x == 387 { LINE } else { GAP });
    let scale = window_scale(&painted, LINE)?;
    ensure!(scale == 2.0, "scale {scale}");
    // 200 × 150 logical; the tab's area is 200 × 108 below the strip.
    let got = zone_inner(&painted, scale, [0.5, 0.0, 0.5, 1.0]);
    let want = PxRect { x0: 238, y0: 122, x1: 362, y1: 262 };
    ensure!(got == want, "the right zone's inside {got:?}, want {want:?}");
    ensure!(window_scale(&Image::painted(40, 20, |_, _| GAP), LINE).is_err(), "a scale with no card");
    ensure!(parse_zone_rect("id=2 rect=0.5,0,0.5,1 active") == Some([0.5, 0.0, 0.5, 1.0]), "a zone line");
    ensure!(parse_zone_rect("id=1 rect=0,0,1").is_none(), "three values");
    ensure!(parse_zone_rect("id=1").is_none(), "no rect");
    Ok(())
}

/// Split Agent Right in an agent tab opens a second agent zone on the same
/// profile, with a session of its own, focused; the first keeps its turn.
pub fn split(ctx: &Ctx) -> Result<Outcome> {
    let (profile, received) = hello_agent(ctx)?;
    let out = Run::new(ctx, "run")
        .config(&profile)
        .agent("Fake")
        .first_prompt("hi")
        .wait_for(&answered())
        .action("split_agent_right")
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
    // `agents-login`: an agent offering no method has no login row, and
    // its first prompt is a plain draft, not one waiting for a login.
    ensure!(out.dump.agent_logins().is_empty(), "a login row: {:?}", out.dump.agent);
    ensure!(!out.dump.agent.iter().any(|l| l.contains(" first=")), "a waiting first prompt: {:?}", out.dump.agent);
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
    ensure!(out.dump.agent_logins().is_empty(), "no method, no login row: {:?}", out.dump.agent);

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
    // The request's call, which the agent never sent, is the tool entry
    // before it (`agents-tool-content`).
    let entries = out.dump.agent_entries();
    ensure!(
        entries.len() == 3
            && entries[0] == "user \"one\""
            && entries[1].starts_with("tool call=\"call_1\"")
            && entries[2].ends_with("outcome=pending \"Write a file\""),
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
    ensure!(out.dump.tabs.len() == 2 && out.dump.tabs[1] == ("Fake".to_string(), true), "tabs: {:?}", out.dump.tabs);
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
    ensure!(out.dump.tabs.len() == 2 && out.dump.tabs[1] == ("Fake".to_string(), true), "tabs: {:?}", out.dump.tabs);
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

/// The command an agent asks to run in `agent-notify`.
const ASKED: &str = "cargo test";

/// A permission notice's body in the log, its two lines joined by a space
/// (`agents-zone-attention` decision 6): the call's kind and title, then
/// the options, reject first.
const ASKED_BODY: &str = "Command: cargo test Reject · Allow";

/// A permission notice's title: the profile's name and what it asks.
const ASKED_TITLE: &str = "Fake needs permission";

/// `script` starting a tool call `call_1` that runs [`ASKED`], then asking
/// permission for it with the JSON-RPC id `id`.
fn asks_to_run(script: Script, id: &str) -> Script {
    script
        .step(update(&format!(
            r#"{{"sessionUpdate":"tool_call","toolCallId":"call_1","title":"{ASKED}","kind":"execute","status":"pending"}}"#
        )))
        .step(ask_about(id, ASKED))
}

/// The notification log's lines for an agent's notice (`source<TAB>title
/// <TAB>body`): the profile's name, what it asks, and the conversation's
/// title once the agent names it; the tab's automatic title isn't
/// repeated (`agents-zone-attention` decision 6).
fn agent_notice(title: &str, body: &str) -> String {
    format!("agent\t{title}\t{body}")
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
///   notifies titled `Fake needs permission`, its body the command and
///   the options, and a turn that ends normally notifies `Done` titled
///   `Fake`, the profile's name; answering the permission in front raises
///   nothing;
/// - with `when = "always"` the same turn in the active tab notifies both,
///   and with `notifications.agent = false` it notifies nothing;
/// - a replay after a quit and relaunch notifies nothing, though it shows
///   the pending permission again.
pub fn notify(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("received.jsonl");
    let (asked, answered) = (gate(ctx, "asked"), gate(ctx, "answered"));
    let background = asks_to_run(
        Script::ready(&received).step("expect session/prompt").step(format!("wait-file {}", asked.display())),
        "p1",
    )
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
        .key("tab")
        .key("enter")
        .wait_for("outcome=selected:allow-once")
        .action("activate_tab_2")
        .send(&format!("touch '{}'; {}", answered.display(), until_logged(&log, "Done", "notice-2")))
        .wait_for("notice-2")
        .dump_at(14.0)
        .run()?;
    waited(&out)?;
    ensure!(out.dump.tabs.len() == 2 && out.dump.tabs[1].1, "the Shell tab in front: {:?}", out.dump.tabs);
    let want = [agent_notice(ASKED_TITLE, ASKED_BODY), agent_notice("Fake", "Done")];
    ensure!(notices(&log) == want, "background: notices {:?}, expected {want:?}", notices(&log));

    let in_front = asks_to_run(Script::ready(&received).step("expect session/prompt"), "p1")
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
            .key("tab")
            .key("enter")
            .wait_for("agent \"done\"")
            .wait_for("status=done")
            .dump_at(8.0)
            .run()?;
        waited(&out)?;
        ensure!(notices(&log) == want, "{label}: notices {:?}, expected {want:?}", notices(&log));
    }

    let pending = asks_to_run(Script::ready(&received).step("expect session/prompt"), "p1")
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
    ensure!(notices(&live) == [agent_notice(ASKED_TITLE, ASKED_BODY)], "live: notices {:?}", notices(&live));
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

/// The title an agent gives its conversation in `agent-session-title`.
const NAMED: &str = "Hello.txt file creation";

/// A `send` step of a `session_info_update` for `s1` titling the
/// conversation `title`, or clearing its title for `None`.
fn titles(title: Option<&str>) -> String {
    let title = serde_json::to_string(&title).unwrap_or_default();
    format!(
        r#"send {{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"s1","update":{{"sessionUpdate":"session_info_update","title":{title}}}}}}}"#
    )
}

/// The agent zone's tab title and its conversation's (OpenSpec change
/// `agents-session-title`), with an agent whose `initialize` names it
/// `@fake/acp`, titled `Fake Agent`:
/// - the tab reads the profile's name, `Fake`, and the zone's dump names
///   the agent by its readable title;
/// - after the agent titles its conversation (on the first prompt) the
///   tab reads the title, and the turn's `Done` notification is titled
///   with the profile's name and it (`agents-zone-attention` decision 6);
/// - after the agent clears it (on the second prompt) the tab and the
///   notification read `Fake` again;
/// - a tab renamed `Build` through the rename prompt keeps its name after
///   the agent's title, and the notification adds it after the zone's
///   title.
pub fn session_title(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("received.jsonl");
    // Holds the second turn until it shows as working, so its `Done` is a
    // change of status the typed prompt didn't mark seen.
    let cleared = gate(ctx, "cleared");
    let profile = Script(vec![
        format!("record {}", received.display()),
        "expect initialize".into(),
        r#"respond {"protocolVersion":1,"agentInfo":{"name":"@fake/acp","title":"Fake Agent","version":"1.0"}}"#.into(),
        "expect session/new".into(),
        r#"respond {"sessionId":"s1"}"#.into(),
    ])
    .step("expect session/prompt")
    .step(titles(Some("Hello.txt\nfile  creation")))
    .step(says("named"))
    .step(stop("end_turn"))
    .step("expect session/prompt")
    .step(format!("wait-file {}", cleared.display()))
    .step(titles(None))
    .step(says("cleared"))
    .step(stop("end_turn"))
    .profile(ctx, "titled")?;
    let always = "\n[notifications]\nwhen = \"always\"\n";
    let tab = |out: &crate::brindle::Output| out.dump.tabs.first().map(|(title, _)| title.clone()).unwrap_or_default();

    let out = Run::new(ctx, "untitled").config(&profile).agent("Fake").wait_for(READY).dump_at(4.0).run()?;
    waited(&out)?;
    ensure!(out.dump.tabs == [("Fake".to_string(), true)], "the profile's name: {:?}", out.dump.tabs);
    ensure!(
        out.dump.agent.first().is_some_and(|line| line.contains(" name=\"Fake Agent\" ")),
        "the agent's readable name: {:?}",
        out.dump.agent
    );

    let log = ctx.dir.join("titled.notify");
    let out = Run::new(ctx, "titled")
        .config(&profile)
        .config(always)
        .env("BRINDLE_NOTIFY_LOG", log.to_str().unwrap())
        .agent("Fake")
        .first_prompt("hi")
        .wait_for("agent \"named\"")
        .wait_for("status=done")
        .dump_at(5.0)
        .run()?;
    waited(&out)?;
    ensure!(tab(&out) == NAMED, "the conversation's title, on one line: {:?}", out.dump.tabs);
    ensure!(notices(&log) == [format!("agent\tFake — {NAMED}\tDone")], "titled: notices {:?}", notices(&log));

    let log = ctx.dir.join("cleared.notify");
    let out = Run::new(ctx, "cleared")
        .config(&profile)
        .config(always)
        .env("BRINDLE_NOTIFY_LOG", log.to_str().unwrap())
        .agent("Fake")
        .first_prompt("hi")
        .wait_for("agent \"named\"")
        .wait_for("status=done")
        .send("again")
        .key("enter")
        .wait_for(WORKING)
        .action("new_tab")
        .send(&format!(
            "touch '{}'; until [ \"$(grep -c Done '{}')\" -ge 2 ]; do sleep 0.1; done; echo cle\"\"ared-done\r",
            cleared.display(),
            log.display()
        ))
        .wait_for("cleared-done")
        .dump_at(10.0)
        .run()?;
    waited(&out)?;
    ensure!(out.dump.tabs.len() == 2 && out.dump.tabs[1].1, "the Shell tab in front: {:?}", out.dump.tabs);
    ensure!(tab(&out) == "Fake", "the profile's name again: {:?}", out.dump.tabs);
    let want = [format!("agent\tFake — {NAMED}\tDone"), "agent\tFake\tDone".to_string()];
    ensure!(notices(&log) == want, "cleared: notices {:?}, expected {want:?}", notices(&log));

    let log = ctx.dir.join("renamed.notify");
    let out = Run::new(ctx, "renamed")
        .config(&profile)
        .config(always)
        .env("BRINDLE_NOTIFY_LOG", log.to_str().unwrap())
        .agent("Fake")
        .wait_for(READY)
        .action("rename_tab")
        .send("Build")
        .action("picker_confirm")
        .send("hi")
        .key("enter")
        .wait_for("agent \"named\"")
        .wait_for("status=done")
        .dump_at(8.0)
        .run()?;
    waited(&out)?;
    ensure!(tab(&out) == "Build", "the manual title wins: {:?}", out.dump.tabs);
    let want = [format!("agent\tFake — {NAMED} — Build\tDone")];
    ensure!(notices(&log) == want, "renamed: notices {:?}, expected {want:?}", notices(&log));
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

// Continuing a conversation (OpenSpec change `agents-session-recovery`):
// the zone's profile runs `agent.script`, and the case's Shell tab copies
// the continue's script over it (`cp`) before it lets the first agent end,
// so the restart plays the second script; the agents record into
// `first.jsonl` and `second.jsonl`.

/// `agentCapabilities` offering `session/resume`.
pub(super) const RESUME: &str = r#"{"sessionCapabilities":{"resume":{}}}"#;
/// `agentCapabilities` offering `session/load` alone (as Copilot does).
const LOAD: &str = r#"{"loadSession":true}"#;
/// `agentCapabilities` offering neither.
pub(super) const NEITHER: &str = "{}";

/// What `--wait-for` waits for until the first agent has exited.
pub(super) const EXITED: &str = "state=exited:1";

/// The notice the core adds when it starts the agent again.
pub(super) const RESTARTED: &str = "the agent was started again to continue the conversation";

/// Where the first and the second agent record what they are sent.
pub(super) fn records(ctx: &Ctx) -> (PathBuf, PathBuf) {
    let (first, second) = (ctx.dir.join("first.jsonl"), ctx.dir.join("second.jsonl"));
    // A run's records start empty, as its gates start closed.
    for path in [&first, &second] {
        std::fs::remove_file(path).ok();
    }
    (first, second)
}

/// An agent offering `capabilities` that answers its first prompt with
/// [`HELLO`], waits for `gate`, then exits with status 1.
pub(super) fn turn_then_exit(received: &Path, capabilities: &str, gate: &Path) -> Script {
    Script::offering(received, capabilities)
        .step("expect session/prompt")
        .step(says(HELLO))
        .step(stop("end_turn"))
        .step(format!("wait-file {}", gate.display()))
        .step("exit 1")
}

/// A `session/update` step replaying the user's `text`, as `session/load`
/// does.
fn user_says(text: &str) -> String {
    update(&format!(r#"{{"sessionUpdate":"user_message_chunk","content":{{"type":"text","text":"{text}"}}}}"#))
}

/// The agent continued by `session/load` replaying the first turn
/// (`hi`, then [`HELLO`]: two updates).
fn loaded(received: &Path) -> Script {
    Script::initialized(received, LOAD)
        .step("expect session/load")
        .step(user_says("hi"))
        .step(says(HELLO))
        .step("respond {}")
}

/// Writes `first` as the profile's `agent.script` and `next` beside it,
/// closes `gate`, and gives the `Fake` profile and the shell command that
/// puts `next` in `first`'s place, then runs `then` (opening a gate).
pub(super) fn continued(ctx: &Ctx, first: Script, next: Script, then: &str) -> Result<(String, String)> {
    let next = next.write(ctx, "next")?;
    let profile = first.profile(ctx, "agent")?;
    let current = ctx.dir.join("agent.script");
    Ok((profile, format!("cp '{}' '{}' && {then}", next.display(), current.display())))
}

/// `touch` of `gate`, closed first.
pub(super) fn touch(gate: &Path) -> String {
    std::fs::remove_file(gate).ok();
    format!("touch '{}'", gate.display())
}

/// The conversation's user and agent messages, in order.
pub(super) fn messages(out: &crate::brindle::Output) -> Vec<&str> {
    out.dump.agent_entries().into_iter().filter(|e| e.starts_with("user ") || e.starts_with("agent ")).collect()
}

/// Whether the conversation has a notice containing `text`.
pub(super) fn has_notice(out: &crate::brindle::Output, text: &str) -> bool {
    out.dump.agent_entries().iter().any(|e| e.starts_with("notice ") && e.contains(text))
}

/// How many messages with `method` the agent's record holds.
pub(super) fn recorded_count(received: &Path, method: &str) -> usize {
    let text = std::fs::read_to_string(received).unwrap_or_default();
    text.lines().filter(|line| line.contains(&format!("\"method\":\"{method}\""))).count()
}

/// An agent that offered `session/resume` exits after a turn: the zone
/// offers Continue conversation one Tab from the prompt, and continues
/// nothing on its own; enter on the button starts the agent again on the
/// same session with `session/resume`, the earlier turn stays shown once,
/// and a new prompt reaches the new agent.
pub fn continue_resume(ctx: &Ctx) -> Result<Outcome> {
    let go = gate(ctx, "exit");
    let next = |second: &Path| {
        Script::initialized(second, RESUME)
            .step("expect session/resume")
            .step("respond {}")
            .step("expect session/prompt")
            .step(says("Again"))
            .step(stop("end_turn"))
    };

    let (first, second) = records(ctx);
    let (profile, swap) = continued(ctx, turn_then_exit(&first, RESUME, &go), next(&second), &touch(&go))?;
    let run = Run::new(ctx, "button").config(&profile).agent("Fake").first_prompt("hi").wait_for(&answered());
    let out = from_shell(run, &swap, true).wait_for(EXITED).wait_for("continue=restart").key("tab").dump_at(10.0).run()?;
    waited(&out)?;
    ensure!(
        out.dump.focus.as_deref() == Some("agent") && out.dump.focus_stop.as_deref() == Some("continue"),
        "Tab from the prompt reaches the button: {:?} {:?}",
        out.dump.focus,
        out.dump.focus_stop
    );
    ensure!(out.dump.agent_field("state") == Some("exited:1"), "agent: {:?}", out.dump.agent);
    ensure!(!recorded(&second, "initialize"), "the agent was started again on its own");

    let (first, second) = records(ctx);
    let (profile, swap) = continued(ctx, turn_then_exit(&first, RESUME, &go), next(&second), &touch(&go))?;
    let run = Run::new(ctx, "continue").config(&profile).agent("Fake").first_prompt("hi").wait_for(&answered());
    let out = from_shell(run, &swap, true)
        .wait_for(EXITED)
        .key("tab")
        .key("enter")
        .wait_for(READY)
        .send("again")
        .key("enter")
        .wait_for("agent \"Again\"")
        .dump_at(16.0)
        .run()?;
    waited(&out)?;
    let want = ["user \"hi\"", "agent \"Hello!\"", "user \"again\"", "agent \"Again\""];
    ensure!(messages(&out) == want, "messages: {:?}", out.dump.agent);
    ensure!(has_notice(&out, RESTARTED), "no restart notice: {:?}", out.dump.agent);
    ensure!(out.dump.agent_field("session") == Some("open"), "agent: {:?}", out.dump.agent);
    ensure!(prompts_sent(&first)? == ["hi"] && prompts_sent(&second)? == ["again"], "sent: {:?} then {:?}", prompts_sent(&first)?, prompts_sent(&second)?);
    let resumed = std::fs::read_to_string(&second).unwrap_or_default();
    ensure!(
        resumed.lines().any(|l| l.contains("\"session/resume\"") && l.contains("\"sessionId\":\"s1\"")),
        "no session/resume of s1: {resumed}"
    );
    ensure!(recorded_count(&second, "session/new") == 0, "a new conversation was started: {resumed}");
    Ok(Outcome::Pass)
}

/// An agent offering `session/load` alone (Copilot-like) is continued by a
/// load whose replay isn't shown: the earlier turn shows once, then the
/// notice that the replay of 2 updates wasn't shown.
pub fn continue_load(ctx: &Ctx) -> Result<Outcome> {
    let go = gate(ctx, "exit");
    let (first, second) = records(ctx);
    let (profile, swap) = continued(ctx, turn_then_exit(&first, LOAD, &go), loaded(&second), &touch(&go))?;
    let run = Run::new(ctx, "run").config(&profile).agent("Fake").first_prompt("hi").wait_for(&answered());
    let out = from_shell(run, &swap, true).wait_for(EXITED).action("agent_continue").wait_for(READY).dump_at(11.0).run()?;
    waited(&out)?;
    ensure!(messages(&out) == ["user \"hi\"", "agent \"Hello!\""], "messages: {:?}", out.dump.agent);
    ensure!(has_notice(&out, "replay of 2 updates wasn't shown"), "no muted replay's notice: {:?}", out.dump.agent);
    ensure!(recorded(&second, "session/load"), "no session/load was sent");
    Ok(Outcome::Pass)
}

/// A prompt waiting when the agent exits is held; continued, its
/// send-again comes back once the agent is ready and sends it to the new
/// agent, while the turn in flight at the exit failed and is never sent
/// again.
pub fn continue_held(ctx: &Ctx) -> Result<Outcome> {
    let go = gate(ctx, "exit");
    let (first, second) = records(ctx);
    let next = Script::initialized(&second, RESUME)
        .step("expect session/resume")
        .step("respond {}")
        .step("expect session/prompt")
        .step(says("B"))
        .step(stop("end_turn"));
    let first_agent = Script::offering(&first, RESUME).prompt_held(&go).step("exit 1");
    let (profile, swap) = continued(ctx, first_agent, next, &touch(&go))?;
    let run = Run::new(ctx, "run")
        .config(&profile)
        .agent("Fake")
        .first_prompt("one")
        .wait_for(WORKING)
        .send("two")
        .key("enter")
        .wait_for("queued waiting \"two\"");
    let out = from_shell(run, &swap, true)
        .wait_for(EXITED)
        .wait_for("queued held reason=agent-stopped resend=no \"two\"")
        .action("agent_continue")
        .wait_for(READY)
        .wait_for("queued held reason=agent-stopped resend=yes \"two\"")
        .action("agent_resend")
        .wait_for("agent \"B\"")
        .dump_at(20.0)
        .run()?;
    waited(&out)?;
    no_queued_row(&out)?;
    ensure!(messages(&out) == ["user \"one\"", "user \"two\"", "agent \"B\""], "messages: {:?}", out.dump.agent);
    ensure!(prompts_sent(&first)? == ["one"], "the first agent was sent {:?}", prompts_sent(&first)?);
    ensure!(prompts_sent(&second)? == ["two"], "the continued agent was sent {:?}", prompts_sent(&second)?);
    Ok(Outcome::Pass)
}

/// Continue This Conversation from the command palette continues the
/// conversation as the button does.
pub fn continue_palette(ctx: &Ctx) -> Result<Outcome> {
    let go = gate(ctx, "exit");
    let (first, second) = records(ctx);
    let next = Script::initialized(&second, RESUME).step("expect session/resume").step("respond {}");
    let (profile, swap) = continued(ctx, turn_then_exit(&first, RESUME, &go), next, &touch(&go))?;
    let run = Run::new(ctx, "run").config(&profile).agent("Fake").first_prompt("hi").wait_for(&answered());
    let out = from_shell(run, &swap, true)
        .wait_for(EXITED)
        .action("open_command_palette")
        // `agent_continue`'s name since `agents-session-list`.
        .send("Continue This Conversation")
        .action("picker_confirm")
        .wait_for(READY)
        .dump_at(13.0)
        .run()?;
    waited(&out)?;
    ensure!(out.dump.palette.is_empty(), "the palette closed: {:?}", out.dump.palette);
    ensure!(messages(&out) == ["user \"hi\"", "agent \"Hello!\""], "messages: {:?}", out.dump.agent);
    ensure!(has_notice(&out, RESTARTED), "no restart notice: {:?}", out.dump.agent);
    ensure!(recorded(&second, "session/resume"), "no session/resume was sent");
    Ok(Outcome::Pass)
}

/// The session running the fake agent, from `brindle-remote list --json`:
/// its pid, `None` while none is listed running.
pub(super) fn agent_pid(helper: &Path, server_dir: &Path) -> Result<Option<u32>> {
    let out = std::process::Command::new(helper)
        .args(["list", "--json"])
        .env("BRINDLE_SERVER_DIR", server_dir)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("running {}: {e}", helper.display()))?;
    ensure!(out.status.success(), "`brindle-remote list --json` failed: {}", String::from_utf8_lossy(&out.stderr));
    let listed: serde_json::Value =
        serde_json::from_slice(&out.stdout).map_err(|e| format!("`brindle-remote list --json`: {e}"))?;
    Ok(listed.as_array().into_iter().flatten().find_map(|session| {
        let command = session["command"].as_str()?;
        let pid = session["pid"].as_u64()?;
        (command.contains("brindle-fake-acp-agent") && pid != 0).then_some(pid as u32)
    }))
}

/// The holder running an agent that offered `session/load` is killed: the
/// zone says the agent was lost with its holder and offers to continue on
/// the same session, which a muted load does.
pub fn continue_holder_lost(ctx: &Ctx) -> Result<Outcome> {
    let (first, second) = records(ctx);
    let first_agent = Script::offering(&first, LOAD).step("expect session/prompt").step(says(HELLO)).step(stop("end_turn"));
    let go = gate(ctx, "kill-holder");
    let (profile, swap) = continued(ctx, first_agent, loaded(&second), &touch(&go))?;
    let (helper, server_dir, scale) = (ctx.helper.clone(), ctx.server_dir.clone(), ctx.scale());
    let run = Run::new(ctx, "run")
        .config(&profile)
        .agent("Fake")
        .first_prompt("hi")
        // The case's go-ahead comes from its Shell tab, after the turn.
        .call_at(1.0, move |_| {
            scale.eventually(60.0, "the Shell tab's go-ahead to kill the holder", || go.exists())?;
            let agent = agent_pid(&helper, &server_dir)?.ok_or("no running agent session listed")?;
            let holder = super::local_server::parent(agent)?
                .filter(|&pid| super::local_server::is_holder(pid))
                .ok_or_else(|| format!("the agent {agent}'s parent isn't a holder"))?;
            super::local_server::kill(holder)
        })
        .wait_for(&answered());
    let out = from_shell(run, &swap, true)
        .wait_for("state=holder-lost")
        .wait_for("continue=restart")
        .action("agent_continue")
        .wait_for(READY)
        .dump_at(14.0)
        .run()?;
    waited(&out)?;
    ensure!(messages(&out) == ["user \"hi\"", "agent \"Hello!\""], "messages: {:?}", out.dump.agent);
    ensure!(has_notice(&out, "replay of 2 updates wasn't shown"), "no muted replay's notice: {:?}", out.dump.agent);
    ensure!(out.dump.agent_field("session") == Some("open"), "the same session: {:?}", out.dump.agent);
    ensure!(recorded(&second, "session/load"), "no session/load was sent");
    Ok(Outcome::Pass)
}

/// The core killed while the agent has exited loses the ended session: the
/// zone says the session was lost and offers to continue it in a new one,
/// whose log is the agent's replay after the notice that it continues an
/// earlier conversation.
pub fn continue_lost(ctx: &Ctx) -> Result<Outcome> {
    let go = gate(ctx, "exit");
    let (first, second) = records(ctx);
    let (profile, swap) = continued(ctx, turn_then_exit(&first, LOAD, &go), loaded(&second), &touch(&go))?;
    let kill_core = gate(ctx, "kill-core");
    let ask = touch(&kill_core);
    let (server_dir, scale) = (ctx.server_dir.clone(), ctx.scale());
    let run = Run::new(ctx, "run")
        .config(&profile)
        .agent("Fake")
        .first_prompt("hi")
        // The go-ahead comes from the Shell tab once the agent has exited.
        .call_at(1.0, move |_| {
            scale.eventually(60.0, "the Shell tab's go-ahead to kill the core", || kill_core.exists())?;
            let core = crate::server_processes(&server_dir).into_iter().find(|(_, role)| role == "serve");
            let (core, _) = core.ok_or("no core running")?;
            super::local_server::kill(core)
        })
        .wait_for(&answered());
    let run = from_shell(run, &swap, true).wait_for(EXITED);
    let out = from_shell(run, &ask, false)
        .wait_for("state=lost")
        .wait_for("continue=respawn")
        .action("agent_continue")
        .wait_for(READY)
        .dump_at(25.0)
        .run()?;
    waited(&out)?;
    let entries = out.dump.agent_entries();
    ensure!(
        entries.len() == 3
            && entries[0].starts_with("notice ")
            && entries[0].contains("this continues an earlier conversation")
            && entries[1..] == ["user \"hi\"", "agent \"Hello!\""],
        "a fresh log of the replay: {entries:?}"
    );
    ensure!(out.dump.agent_field("session") == Some("open"), "agent: {:?}", out.dump.agent);
    ensure!(recorded(&second, "session/load"), "no session/load was sent");
    Ok(Outcome::Pass)
}

/// An agent that offered neither `session/load` nor `session/resume`
/// exits: no Continue conversation button (Tab from the prompt reaches
/// Conversations…, the first of the other buttons since
/// `agents-session-list`), the dump says why, and `agent_continue` starts
/// nothing.
pub fn continue_none(ctx: &Ctx) -> Result<Outcome> {
    let (first, _) = records(ctx);
    let profile = Script::offering(&first, NEITHER)
        .step("expect session/prompt")
        .step(says(HELLO))
        .step(stop("end_turn"))
        .step("exit 1")
        .profile(ctx, "agent")?;
    let out = Run::new(ctx, "run")
        .config(&profile)
        .agent("Fake")
        .first_prompt("hi")
        .wait_for(EXITED)
        .wait_for("continue=none:not-offered")
        .key("tab")
        .action("agent_continue")
        .dump_at(7.0)
        .run()?;
    waited(&out)?;
    ensure!(out.dump.focus_stop.as_deref() == Some("conversations"), "no Continue button to Tab to: {:?}", out.dump.focus_stop);
    let prompt = out.dump.agent.iter().find_map(|line| line.strip_prefix("prompt ")).unwrap_or("");
    ensure!(prompt.ends_with("actions=conversations,new"), "the other buttons: {:?}", out.dump.agent);
    ensure!(out.dump.agent_field("state") == Some("exited:1"), "agent: {:?}", out.dump.agent);
    ensure!(recorded_count(&first, "initialize") == 1, "the agent was started again");
    Ok(Outcome::Pass)
}

/// The zone's agent profile removed from the config after its agent
/// exited: no button, the dump says the profile is gone, and
/// `agent_continue` starts nothing.
pub fn continue_profile_gone(ctx: &Ctx) -> Result<Outcome> {
    let go = gate(ctx, "exit");
    let (first, _) = records(ctx);
    let without = ctx.dir.join("without-fake.toml");
    std::fs::write(&without, crate::brindle::config("e2e")).map_err(|e| format!("writing {}: {e}", without.display()))?;
    let profile = turn_then_exit(&first, RESUME, &go).profile(ctx, "agent")?;
    let swap = format!("cp '{}' '{}' && {}", without.display(), ctx.dir.join("config.toml").display(), touch(&go));
    let run = Run::new(ctx, "run").config(&profile).agent("Fake").first_prompt("hi").wait_for(&answered());
    let out = from_shell(run, &swap, true)
        .wait_for(EXITED)
        .action("reload_config")
        .wait_for("continue=none:profile-gone")
        .key("tab")
        .action("agent_continue")
        .dump_at(13.0)
        .run()?;
    waited(&out)?;
    ensure!(out.dump.focus_stop.as_deref() == Some("prompt"), "no button to Tab to: {:?}", out.dump.focus_stop);
    ensure!(messages(&out) == ["user \"hi\"", "agent \"Hello!\""], "messages: {:?}", out.dump.agent);
    ensure!(recorded_count(&first, "initialize") == 1, "the agent was started again");
    Ok(Outcome::Pass)
}

/// A continue the agent refuses (`session/resume` answered `no such
/// session`) leaves the zone saying the agent can't be used, with the
/// agent's message, its session ended and the earlier turn kept.
pub fn continue_refused(ctx: &Ctx) -> Result<Outcome> {
    let go = gate(ctx, "exit");
    let (first, second) = records(ctx);
    let next = Script::initialized(&second, RESUME)
        .step("expect session/resume")
        .step(r#"respond-error -32603 "no such session""#);
    let (profile, swap) = continued(ctx, turn_then_exit(&first, RESUME, &go), next, &touch(&go))?;
    let run = Run::new(ctx, "run").config(&profile).agent("Fake").first_prompt("hi").wait_for(&answered());
    let out = from_shell(run, &swap, true)
        .wait_for(EXITED)
        .action("agent_continue")
        .wait_for("state=failed")
        .wait_for("session=ended")
        .dump_at(12.0)
        .run()?;
    waited(&out)?;
    let line = out.dump.agent.iter().find(|l| l.starts_with("agent ")).cloned().unwrap_or_default();
    ensure!(line.contains("no such session"), "the agent's message isn't shown: {line}");
    ensure!(out.dump.agent_field("continue") == Some("none:not-ended"), "offered again: {line}");
    ensure!(messages(&out) == ["user \"hi\"", "agent \"Hello!\""], "messages: {:?}", out.dump.agent);
    Ok(Outcome::Pass)
}

/// Quitting with an exited agent that could be continued leaves no session
/// on the server, and the next launch doesn't bring the zone back.
pub fn continue_not_restored(ctx: &Ctx) -> Result<Outcome> {
    let (first, _) = records(ctx);
    let profile = Script::offering(&first, RESUME)
        .step("expect session/prompt")
        .step(says(HELLO))
        .step(stop("end_turn"))
        .step("exit 1")
        .profile(ctx, "agent")?;
    Run::new(ctx, "quit")
        .config(&profile)
        .agent("Fake")
        .first_prompt("hi")
        .keep_detached()
        .wait_for(EXITED)
        .wait_for("continue=restart")
        .action("quit")
        .expect_exit()
        .dump_at(8.0)
        .run()?;
    ctx.scale().eventually(10.0, "no session listed after the quit", || {
        crate::brindle::listed_sessions(ctx).is_ok_and(|listed| listed.is_empty())
    })?;
    let out = Run::new(ctx, "relaunch").config(&profile).restore().dump_at(4.0).run()?;
    ensure!(out.dump.agent.is_empty(), "an agent zone came back: {:?}", out.dump.agent);
    ensure!(!out.dump.tabs.iter().any(|(title, _)| title == "Fake"), "tabs: {:?}", out.dump.tabs);
    Ok(Outcome::Pass)
}

// OpenSpec change `agents-login`, tasks 7.3 and 7.4: logging in from the
// zone. Methods are the agent's `initialize` `authMethods`; the core sends
// `authenticate` for an agent method and Brindle runs a terminal one.

/// Two agent methods, in this order.
pub(super) const TWO_LOGINS: &str =
    r#"[{"id":"oauth-personal","name":"Log in with Google"},{"id":"api-key","name":"API Key","description":"Use an API key"}]"#;

/// One terminal method: the profile's command with `login`, setting
/// `LOGIN_MODE` and asking for `PATH`, which Brindle leaves out.
pub(super) const TERMINAL_LOGIN: &str = r#"[{"id":"cli-login","name":"Log in from the terminal","type":"terminal","args":["login"],"env":{"LOGIN_MODE":"device","PATH":"/nowhere"}}]"#;

/// What `--wait-for` waits for while the agent needs a login.
pub(super) const NEEDS_LOGIN: &str = "state=login-required";

/// What `--wait-for` waits for while the agent logs in.
pub(super) const LOGGING_IN: &str = "state=logging-in";

/// The two agent methods' choices, in order, as `--wait-for` sees them.
pub(super) const TWO_CHOICES: &str = "--- login choice method=\"oauth-personal\" kind=agent \"Log in with Google\"\n\
                           --- login choice method=\"api-key\" kind=agent \"API Key\" description=\"Use an API key\"";

impl Script {
    /// An agent that records what it is sent in `received` and answers
    /// `initialize` offering `capabilities` and the login `methods` (a
    /// JSON array such as [`TWO_LOGINS`]).
    pub(super) fn offering_logins(received: &Path, capabilities: &str, methods: &str) -> Self {
        Script(vec![
            format!("record {}", received.display()),
            "expect initialize".into(),
            format!(
                r#"respond {{"protocolVersion":1,"agentInfo":{{"name":"Fake Agent","version":"1.0"}},"agentCapabilities":{capabilities},"authMethods":{methods}}}"#
            ),
        ])
    }

    /// As [`Script::offering_logins`], whose `session/new` needs a login.
    pub(super) fn login_at_start(received: &Path, methods: &str) -> Self {
        Script::offering_logins(received, NEITHER, methods)
            .step("expect session/new")
            .step(r#"respond-error -32000 "run fake login""#)
    }

    /// The agent writes its environment to `env` first (`record-env`),
    /// whatever its arguments: the step leads the steps played when no
    /// `args` section matches.
    pub(super) fn recording_env(mut self, env: &Path) -> Self {
        self.0.insert(0, format!("record-env {}", env.display()));
        self
    }

    /// The login's `session/new`, then a turn answering [`HELLO`].
    pub(super) fn then_hello(self) -> Self {
        self.step("expect session/new")
            .step(r#"respond {"sessionId":"s1"}"#)
            .step("expect session/prompt")
            .step(says(HELLO))
            .step(stop("end_turn"))
    }
}

/// The `authenticate` requests' method ids, in order, from a record.
pub(super) fn logins_sent(received: &Path) -> Result<Vec<String>> {
    let text = std::fs::read_to_string(received).map_err(|e| format!("reading {}: {e}", received.display()))?;
    let mut methods = Vec::new();
    for line in text.lines() {
        let value: serde_json::Value = serde_json::from_str(line).map_err(|e| format!("{line}: {e}"))?;
        if value["method"] == "authenticate" {
            methods.push(value["params"]["methodId"].as_str().unwrap_or_default().to_string());
        }
    }
    Ok(methods)
}

/// An agent needing a login at the start offers its two methods in order,
/// nothing chosen; tab then enter chooses the first, the zone shows it
/// logging in until the agent answers `authenticate` (a gate), then the
/// agent is ready and a prompt is answered.
pub fn login_methods(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("received.jsonl");
    let go = gate(ctx, "logged-in");
    let profile = Script::login_at_start(&received, TWO_LOGINS)
        .step("expect authenticate")
        .step(format!("wait-file {}", go.display()))
        .step("respond {}")
        .then_hello()
        .profile(ctx, "methods")?;
    let run = Run::new(ctx, "run")
        .config(&profile)
        .agent("Fake")
        .wait_for(NEEDS_LOGIN)
        .wait_for(TWO_CHOICES)
        .key("tab")
        .key("enter")
        .wait_for(LOGGING_IN)
        .wait_for("--- login running method=\"oauth-personal\" \"Log in with Google\"");
    let out = touch_from_shell(run, &go, true)
        .wait_for(READY)
        .send("hi")
        .key("enter")
        .wait_for(&answered())
        .dump_at(16.0)
        .run()?;
    waited(&out)?;
    ensure!(out.dump.agent_logins().is_empty(), "the row went: {:?}", out.dump.agent);
    ensure!(messages(&out) == ["user \"hi\"", "agent \"Hello!\""], "messages: {:?}", out.dump.agent);
    ensure!(logins_sent(&received)? == ["oauth-personal"], "logins: {:?}", logins_sent(&received)?);
    Ok(Outcome::Pass)
}

/// A login the agent fails offers the methods again with its words;
/// choosing the other (tab, tab, enter) logs in.
pub fn login_fails(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("received.jsonl");
    let go = gate(ctx, "fail");
    // The failure waits for the case, so the zone shows the login running
    // (its methods' stops gone, focus back in the prompt input) first.
    let profile = Script::login_at_start(&received, TWO_LOGINS)
        .step("expect authenticate")
        .step(format!("wait-file {}", go.display()))
        .step(r#"respond-error -32603 "browser login timed out""#)
        .step("expect authenticate")
        .step("respond {}")
        .then_hello()
        .profile(ctx, "fails")?;
    let run = Run::new(ctx, "run")
        .config(&profile)
        .agent("Fake")
        .wait_for(TWO_CHOICES)
        .key("tab")
        .key("enter")
        .wait_for(LOGGING_IN);
    let out = touch_from_shell(run, &go, true)
        .wait_for("browser login timed out")
        .wait_for(TWO_CHOICES)
        .key("tab")
        .key("tab")
        .key("enter")
        .wait_for(READY)
        .dump_at(14.0)
        .run()?;
    waited(&out)?;
    ensure!(logins_sent(&received)? == ["oauth-personal", "api-key"], "logins: {:?}", logins_sent(&received)?);
    ensure!(out.dump.agent_logins().is_empty(), "the row went: {:?}", out.dump.agent);
    Ok(Outcome::Pass)
}

/// Cancel login (the one stop while logging in, `login:cancel`) sends
/// `$/cancel_request`, the methods come back, the agent's late answer to
/// the cancelled `authenticate` starts nothing, and choosing again logs
/// in.
pub fn login_cancel(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("received.jsonl");
    let profile = Script::login_at_start(&received, TWO_LOGINS)
        .step("expect authenticate")
        .step("expect $/cancel_request")
        .step("respond-to 3 {}")
        .step("expect authenticate")
        .step("respond {}")
        .then_hello()
        .profile(ctx, "cancel")?;
    let started = |label| {
        Run::new(ctx, label).config(&profile).agent("Fake").wait_for(TWO_CHOICES).key("tab").key("enter").wait_for(LOGGING_IN)
    };
    let out = started("focus").key("tab").dump_at(6.0).run()?;
    waited(&out)?;
    ensure!(out.dump.focus_stop.as_deref() == Some("login:cancel"), "Cancel login: {:?}", out.dump.focus_stop);

    std::fs::remove_file(&received).ok();
    let out = started("cancel")
        .key("tab")
        .key("enter")
        .wait_for(NEEDS_LOGIN)
        .wait_for(TWO_CHOICES)
        .key("tab")
        .key("enter")
        .wait_for(READY)
        .dump_at(12.0)
        .run()?;
    waited(&out)?;
    ensure!(recorded(&received, "$/cancel_request"), "no $/cancel_request was sent");
    ensure!(logins_sent(&received)? == ["oauth-personal", "oauth-personal"], "logins: {:?}", logins_sent(&received)?);
    ensure!(recorded_count(&received, "session/new") == 2, "the late answer started a session");
    Ok(Outcome::Pass)
}

/// A login demanded by a prompt while another waits: the turn fails and
/// the waiting prompt is held; after the login (shift-tab reaches the last
/// method from the prompt) the failed turn is kept, nothing is sent on its
/// own, and `agent_resend` sends the held prompt to the same session.
pub fn login_prompt(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("received.jsonl");
    let go = gate(ctx, "expire");
    let profile = Script::offering_logins(&received, NEITHER, TWO_LOGINS)
        .step("expect session/new")
        .step(r#"respond {"sessionId":"s1"}"#)
        .step("expect session/prompt")
        .step(format!("wait-file {}", go.display()))
        .step(r#"respond-error -32000 "session expired""#)
        .step("expect authenticate")
        .step("respond {}")
        .step("expect session/prompt")
        .step(says("sent again"))
        .step(stop("end_turn"))
        .profile(ctx, "prompt")?;
    let run = Run::new(ctx, "run")
        .config(&profile)
        .agent("Fake")
        .first_prompt("one")
        .wait_for(WORKING)
        .send("two")
        .key("enter")
        .wait_for("queued waiting \"two\"");
    let out = touch_from_shell(run, &go, true)
        .wait_for(NEEDS_LOGIN)
        .wait_for("queued held reason=login-required resend=no \"two\"")
        .key("shift-tab")
        .key("shift-tab")
        .key("enter")
        .wait_for(READY)
        .wait_for("queued held reason=login-required resend=yes \"two\"")
        .action("agent_resend")
        .wait_for("agent \"sent again\"")
        .dump_at(18.0)
        .run()?;
    waited(&out)?;
    ensure!(logins_sent(&received)? == ["oauth-personal"], "logins: {:?}", logins_sent(&received)?);
    ensure!(prompts_sent(&received)? == ["one", "two"], "sent: {:?}", prompts_sent(&received)?);
    ensure!(recorded_count(&received, "session/new") == 1, "the same session");
    ensure!(messages(&out).starts_with(&["user \"one\""]), "the failed turn kept: {:?}", out.dump.agent);
    Ok(Outcome::Pass)
}

/// `-p hi` to an agent needing a login: the prompt input shows `hi`
/// read-only (`first="hi"`) through the login, and `hi` is sent once when
/// the agent is ready.
pub fn first_prompt_after_login(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("received.jsonl");
    let go = gate(ctx, "logged-in");
    let profile = Script::login_at_start(&received, TWO_LOGINS)
        .step("expect authenticate")
        .step(format!("wait-file {}", go.display()))
        .step("respond {}")
        .then_hello()
        .profile(ctx, "first")?;
    let run = Run::new(ctx, "run")
        .config(&profile)
        .agent("Fake")
        .first_prompt("hi")
        .wait_for("prompt status=disabled first=\"hi\"")
        .key("tab")
        .key("enter")
        .wait_for(LOGGING_IN)
        .wait_for("first=\"hi\"");
    let out = touch_from_shell(run, &go, true).wait_for(&answered()).dump_at(14.0).run()?;
    waited(&out)?;
    ensure!(prompts_sent(&received)? == ["hi"], "sent: {:?}", prompts_sent(&received)?);
    ensure!(messages(&out) == ["user \"hi\"", "agent \"Hello!\""], "messages: {:?}", out.dump.agent);
    ensure!(!out.dump.agent.iter().any(|l| l.contains(" first=")), "still waiting: {:?}", out.dump.agent);
    Ok(Outcome::Pass)
}

/// Closing a zone whose agent logs in asks nothing (a login is no turn),
/// and its agent ends.
pub fn login_close(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("received.jsonl");
    let profile = Script::login_at_start(&received, TWO_LOGINS)
        .step("expect authenticate")
        .step(format!("wait-file {}", gate(ctx, "never").display()))
        .profile(ctx, "close")?;
    let out = Run::new(ctx, "run")
        .config(&profile)
        .agent("Fake")
        .keep_detached()
        .wait_for(TWO_CHOICES)
        .key("tab")
        .key("enter")
        .wait_for(LOGGING_IN)
        .action("new_tab")
        .action("prev_tab")
        .action("close_zone")
        .dump_at(9.0)
        .run()?;
    waited(&out)?;
    ensure!(out.dump.confirm.is_none(), "nothing asked: {:?}", out.dump.confirm);
    ensure!(out.dump.tabs.len() == 1 && out.dump.tabs[0].0 != "Fake", "the agent tab closed: {:?}", out.dump.tabs);
    sessions_listed(ctx, 1)?;
    crate::brindle::hang_up_detached(ctx)?;
    Ok(Outcome::Pass)
}

/// The fake agent's script for a terminal login: the agent part as
/// `agent` (the steps with no arguments), then the login program's
/// section (`args login`): it records its environment in `env`, then
/// plays `login`.
pub(super) fn with_terminal_login(agent: Script, env: &Path, login: &[String]) -> Script {
    let mut script = agent.step("args login").step(format!("record-env {}", env.display()));
    for step in login {
        script = script.step(step.clone());
    }
    script
}

/// The terminal method's choice line, as `--wait-for` sees it.
pub(super) const TERMINAL_CHOICE: &str = "--- login choice method=\"cli-login\" kind=terminal \"Log in from the terminal\" \
                               args=\"login\" env=\"LOGIN_MODE=device\" dropped=\"PATH\"";

/// A terminal login after a prompt's demand: the choice says what it runs
/// and leaves out; enter opens a terminal zone beside the agent zone
/// running the profile's command with `login`; its exit 0 starts the agent
/// again (the core's notice), which resumes the same conversation, and a
/// prompt is answered there, the earlier turn kept.
pub fn login_terminal(ctx: &Ctx) -> Result<Outcome> {
    let (first, second) = records(ctx);
    let env = ctx.dir.join("login.env");
    std::fs::remove_file(&env).ok();
    let go = gate(ctx, "login-done");
    std::fs::remove_file(&go).ok();
    let agent = Script::offering_logins(&first, RESUME, TERMINAL_LOGIN)
        .step("expect session/new")
        .step(r#"respond {"sessionId":"s1"}"#)
        .step("expect session/prompt")
        .step(says(HELLO))
        .step(stop("end_turn"))
        .step("expect session/prompt")
        .step(r#"respond-error -32000 "session expired""#);
    let login = [format!("wait-file {}", go.display()), "exit 0".to_string()];
    let profile = with_terminal_login(agent, &env, &login).profile(ctx, "agent")?;
    let next = Script::offering_logins(&second, RESUME, TERMINAL_LOGIN)
        .step("expect session/resume")
        .step("respond {}")
        .step("expect session/prompt")
        .step(says("Again"))
        .step(stop("end_turn"))
        .write(ctx, "next")?;
    let current = ctx.dir.join("agent.script");
    let scale = ctx.scale();
    let out = Run::new(ctx, "run")
        .config(&profile)
        .agent("Fake")
        .first_prompt("one")
        // Once the login program has read its script (its environment is
        // recorded), the agent started again reads the next one; then the
        // login program exits 0.
        .call_at(1.0, move |_| {
            scale.eventually(60.0, "the login program's environment", || env.exists())?;
            std::fs::copy(&next, &current).map_err(|e| format!("swapping the script: {e}"))?;
            std::fs::write(&go, "").map_err(|e| format!("opening the gate: {e}"))
        })
        .wait_for(&answered())
        .send("two")
        .key("enter")
        .wait_for(NEEDS_LOGIN)
        .wait_for(TERMINAL_CHOICE)
        .key("tab")
        .key("enter")
        // The login zone is active until its program exits.
        .wait_for(READY)
        .send("three")
        .key("enter")
        .wait_for("agent \"Again\"")
        .dump_at(22.0)
        .run()?;
    waited(&out)?;
    ensure!(out.dump.zones.is_none(), "the login zone closed: {:?}", out.dump.zones);
    ensure!(has_notice(&out, "the agent is started again after its login"), "no restart notice: {:?}", out.dump.agent);
    let entries = messages(&out);
    ensure!(
        entries.starts_with(&["user \"one\"", "agent \"Hello!\"", "user \"two\""]) && entries.ends_with(&["user \"three\"", "agent \"Again\""]),
        "the conversation kept: {entries:?}"
    );
    ensure!(recorded(&second, "session/resume"), "the same conversation wasn't resumed");
    ensure!(prompts_sent(&second)? == ["three"], "sent after the login: {:?}", prompts_sent(&second)?);
    Ok(Outcome::Pass)
}

/// A terminal login whose program exits 1: its zone closes, the methods
/// show again with the status, and the agent still needs a login.
pub fn login_terminal_fails(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("received.jsonl");
    let env = ctx.dir.join("login.env");
    let agent = Script::login_at_start(&received, TERMINAL_LOGIN);
    let profile = with_terminal_login(agent, &env, &["exit 1".to_string()]).profile(ctx, "fails")?;
    let out = Run::new(ctx, "run")
        .config(&profile)
        .agent("Fake")
        .wait_for(TERMINAL_CHOICE)
        .key("tab")
        .key("enter")
        .wait_for("refusal=\"the login program exited with status 1\"")
        .dump_at(8.0)
        .run()?;
    waited(&out)?;
    ensure!(out.dump.zones.is_none(), "the login zone closed: {:?}", out.dump.zones);
    ensure!(out.dump.agent_field("state") == Some("login-required"), "agent: {:?}", out.dump.agent);
    ensure!(
        out.dump.agent_logins().iter().any(|l| l.starts_with("choice method=\"cli-login\"")),
        "the methods again: {:?}",
        out.dump.agent
    );
    ensure!(!recorded(&received, "authenticate"), "a terminal method was sent to the agent");
    Ok(Outcome::Pass)
}

/// A running terminal login: its zone sits right of the agent zone, the
/// agent zone shows it running, and the login program got the method's
/// `LOGIN_MODE` but not its `PATH`.
pub fn login_terminal_env(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("received.jsonl");
    let env = ctx.dir.join("login.env");
    std::fs::remove_file(&env).ok();
    let agent = Script::login_at_start(&received, TERMINAL_LOGIN);
    let login = [format!("wait-file {}", gate(ctx, "never").display())];
    let profile = with_terminal_login(agent, &env, &login).profile(ctx, "env")?;
    let scale = ctx.scale();
    let recorded_env = env.clone();
    let out = Run::new(ctx, "run")
        .config(&profile)
        .agent("Fake")
        .call_at(1.0, move |_| scale.eventually(60.0, "the login program's environment", || recorded_env.exists()))
        .wait_for(TERMINAL_CHOICE)
        .key("tab")
        .key("enter")
        .action("focus_zone_left")
        .wait_for("--- login running method=\"cli-login\" \"Log in from the terminal\"")
        .dump_at(9.0)
        .run()?;
    waited(&out)?;
    ensure!(out.dump.zones.as_deref() == Some("count=2 active=1"), "zones: {:?}", out.dump.zones);
    ensure!(out.dump.agent_field("state") == Some("login-required"), "agent: {:?}", out.dump.agent);
    agent_has(&out, "prompt status=disabled")?;
    let text = std::fs::read_to_string(&env).map_err(|e| format!("reading {}: {e}", env.display()))?;
    ensure!(text.lines().any(|l| l == "LOGIN_MODE=device"), "LOGIN_MODE not applied: {text}");
    ensure!(text.lines().any(|l| l.starts_with("PATH=")) && !text.lines().any(|l| l == "PATH=/nowhere"), "PATH applied: {text}");
    Ok(Outcome::Pass)
}

// --- Tool content (OpenSpec change `agents-tool-content`, section 8) -----

/// The zone set's `diff_added` for the `e2e` theme, an added diff line's
/// background: `ZONE_MUTED` mixed the most, 20%, towards the theme's green
/// (`ANSI[2]`), since the foreground stays legible on it
/// (`theme::legible_tint`, decision 11).
const ZONE_DIFF_ADDED: crate::capture::Rgb = crate::capture::Rgb(0x25, 0x42, 0x2a);
/// The same towards the theme's red (`ANSI[1]`): `diff_removed`.
const ZONE_DIFF_REMOVED: crate::capture::Rgb = crate::capture::Rgb(0x45, 0x28, 0x2a);

/// A diff item editing `/w/notes.txt` from apple to banana, as ACP's JSON.
const NOTES_DIFF: &str = r#"{"type":"diff","path":"/w/notes.txt","oldText":"first\nan apple a day\nlast\n","newText":"first\nan banana a day\nlast\n"}"#;

/// [`NOTES_DIFF`]'s `--- item` line, without `item entry=N `.
const NOTES_ITEM: &str = "index=0 kind=diff op=modify path=\"/w/notes.txt\" added=1 removed=1 omitted=0";

/// A tool line holding [`NOTES_DIFF`]'s count and summary.
const NOTES_COUNTS: &str = "items=1 summary=\"+1 \u{2212}1\"";

/// A `tool_call` for `call`, titled `title`, with `status` and `content`
/// (a JSON array) and any `more` fields (`,"rawInput":…`).
fn tool_call(call: &str, title: &str, status: &str, content: &str, more: &str) -> String {
    update(&format!(
        r#"{{"sessionUpdate":"tool_call","toolCallId":"{call}","title":"{title}","kind":"edit","status":"{status}","content":{content}{more}}}"#
    ))
}

/// A `tool_call_update` setting `call`'s status to `completed`.
fn tool_completed(call: &str) -> String {
    update(&format!(r#"{{"sessionUpdate":"tool_call_update","toolCallId":"{call}","status":"completed"}}"#))
}

/// A `tool_call_update` for `call` carrying Codex's `_meta` (`meta`, an
/// object's members) and any `more` fields.
fn tool_meta(call: &str, meta: &str, more: &str) -> String {
    update(&format!(r#"{{"sessionUpdate":"tool_call_update","toolCallId":"{call}"{more},"_meta":{{{meta}}}}}"#))
}

/// The line of tool entry `entry` holds `want`.
fn tool_line(out: &crate::brindle::Output, entry: u64, want: &str) -> Result {
    let line = out.dump.agent_entry(entry);
    ensure!(line.is_some_and(|l| l.contains(want)), "entry {entry}: no `{want}` in {line:?}; agent {:?}", out.dump.agent);
    Ok(())
}

/// A diff in a running call's open row: its item and hunk lines, the
/// row's summary, its added and removed lines in the zone set's diff
/// colours; once the call
/// completes the row closes, and Toggle Tool Details opens it again, where
/// it stays.
pub fn tool_diff(ctx: &Ctx) -> Result<Outcome> {
    let done = gate(ctx, "done");
    let profile = Script::ready(&ctx.dir.join("received.jsonl"))
        .step("expect session/prompt")
        .step(tool_call("c1", "Edit notes.txt", "in_progress", &format!("[{NOTES_DIFF}]"), ""))
        .step(format!("wait-file {}", done.display()))
        .step(tool_completed("c1"))
        .step(says("done"))
        .step(stop("end_turn"))
        .profile(ctx, "diff")?;
    let started = |label| Run::new(ctx, label).config(&profile).agent("Fake").first_prompt("hi");

    let out = started("open").wait_for("open=yes items=1").capture_at(5.0).dump_at(5.5).run()?;
    waited(&out)?;
    // The row's line carries its summary, lines added and removed.
    tool_line(
        &out,
        2,
        "tool call=\"c1\" kind=edit status=in_progress open=yes items=1 summary=\"+1 \u{2212}1\" \"Edit notes.txt\"",
    )?;
    let items = out.dump.agent_items(2);
    ensure!(items == [NOTES_ITEM], "items {items:?}");
    let hunks = out.dump.agent_hunks(2);
    ensure!(
        hunks.len() == 1 && hunks[0].contains("\\n-an apple a day\\n+an banana a day\\n"),
        "hunks {hunks:?}"
    );
    let image = out.image.as_ref().ok_or("no capture")?;
    let scale = window_scale(image, crate::brindle::theme::CHROME_BORDER)?;
    let zone = zone_inner(image, scale, [0.0, 0.0, 1.0, 1.0]);
    // A line is the zone's width less its paddings, some 20 pixels high.
    for (what, colour) in [("added", ZONE_DIFF_ADDED), ("removed", ZONE_DIFF_REMOVED)] {
        let count = image.count_near(zone, colour, TOKEN_NEAR);
        ensure!(
            count as f32 >= 2_000.0 * scale * scale,
            "the {what} line: {count} pixels of {} in {zone:?}; found {}",
            colour.hex(),
            image.summary(zone)
        );
    }

    let ended = |label| {
        touch_from_shell(started(label).wait_for("open=yes"), &done, true).wait_for("turn=ended:end_turn")
    };
    let out = ended("closed").dump_at(8.0).run()?;
    waited(&out)?;
    tool_line(&out, 2, "status=completed open=no items=1")?;
    // The gate again, closed, for the next run's turn to wait at.
    std::fs::remove_file(&done).map_err(|e| format!("removing {}: {e}", done.display()))?;
    let out = ended("toggled").action("toggle_agent_tool_details").wait_for("open=yes").dump_at(9.0).run()?;
    waited(&out)?;
    tool_line(&out, 2, "status=completed open=yes items=1")?;
    Ok(Outcome::Pass)
}

/// Command output streamed in Codex's `_meta` pieces into an open row: the
/// text so far while it runs, the conversation kept at its end as it
/// grows; then its whole text and exit once it ends.
pub fn tool_output(ctx: &Ctx) -> Result<Outcome> {
    let done = gate(ctx, "done");
    let lines: String = (1..=200).map(|n| format!("line {n}\\n")).collect();
    let profile = Script::ready(&ctx.dir.join("received.jsonl"))
        .step("expect session/prompt")
        .step(update(
            r#"{"sessionUpdate":"tool_call","toolCallId":"c1","title":"Run tests","kind":"execute","status":"in_progress","content":[{"type":"terminal","terminalId":"t1"}],"_meta":{"terminal_info":{"terminal_id":"t1","cwd":"/w","command":"cargo test"}}}"#,
        ))
        .step(tool_meta("c1", r#""terminal_output_delta":{"terminal_id":"t1","data":"first\n"}"#, ""))
        .step(tool_meta("c1", &format!(r#""terminal_output_delta":{{"terminal_id":"t1","data":"{lines}"}}"#), ""))
        .step(format!("wait-file {}", done.display()))
        .step(tool_meta("c1", r#""terminal_output_delta":{"terminal_id":"t1","data":"last\n"}"#, ""))
        .step(tool_meta("c1", r#""terminal_exit":{"terminal_id":"t1","exit_code":1,"signal":null}"#, r#","status":"failed""#))
        .step(says("done"))
        .step(stop("end_turn"))
        .profile(ctx, "output")?;
    let started = |label| Run::new(ctx, label).config(&profile).agent("Fake").first_prompt("hi").wait_for("line 200");

    let out = started("running").dump_at(5.0).run()?;
    waited(&out)?;
    tool_line(&out, 2, "kind=execute status=in_progress open=yes items=1")?;
    let items = out.dump.agent_items(2);
    ensure!(
        items.len() == 1
            && items[0].starts_with("index=0 kind=output command=\"cargo test\" cwd=\"/w\" exit=none omitted=0 \"first\\nline 1\\n")
            && items[0].ends_with("line 200\\n\""),
        "items {items:?}"
    );
    ensure!(
        out.dump.agent.iter().any(|l| l.starts_with("conversation ") && l.contains(" at=end")),
        "the conversation left its end: {:?}",
        out.dump.agent
    );

    let out = touch_from_shell(started("ended"), &done, true).wait_for("turn=ended:end_turn").dump_at(8.0).run()?;
    waited(&out)?;
    tool_line(&out, 2, "status=failed open=no items=1 summary=\"exit 1\"")?;
    let items = out.dump.agent_items(2);
    ensure!(
        items.len() == 1 && items[0].contains(" exit=1 omitted=0 ") && items[0].ends_with("line 200\\nlast\\n\""),
        "items {items:?}"
    );
    Ok(Outcome::Pass)
}

/// A call with only a raw input shows it; a call with content doesn't.
pub fn tool_raw(ctx: &Ctx) -> Result<Outcome> {
    let text = r#"[{"type":"content","content":{"type":"text","text":"**patched**"}}]"#;
    let profile = Script::ready(&ctx.dir.join("received.jsonl"))
        .step("expect session/prompt")
        .step(tool_call("c1", "Apply patch", "completed", "[]", r#","rawInput":"*** Begin Patch""#))
        .step(tool_call("c2", "Apply another", "completed", text, r#","rawInput":"*** Begin Patch""#))
        .step(says("done"))
        .step(stop("end_turn"))
        .profile(ctx, "raw")?;
    let out = Run::new(ctx, "run")
        .config(&profile)
        .agent("Fake")
        .first_prompt("hi")
        .wait_for("turn=ended:end_turn")
        .dump_at(4.0)
        .run()?;
    waited(&out)?;
    ensure!(out.dump.agent_raws(2) == ["which=input \"*** Begin Patch\""], "raw {:?}", out.dump.agent);
    tool_line(&out, 2, "open=no items=0")?;
    ensure!(out.dump.agent_raws(3).is_empty(), "a raw value beside content: {:?}", out.dump.agent);
    ensure!(out.dump.agent_items(3) == ["index=0 kind=text cut=no \"**patched**\""], "items {:?}", out.dump.agent);
    Ok(Outcome::Pass)
}

/// A permission request for `call` carrying `content` (a JSON array, or
/// empty for none), with the JSON-RPC id `id`.
fn ask_with(id: &str, call: &str, content: &str) -> String {
    let content = if content.is_empty() { String::new() } else { format!(r#","content":{content}"#) };
    format!(
        r#"send {{"jsonrpc":"2.0","id":"{id}","method":"session/request_permission","params":{{"sessionId":"s1","toolCall":{{"toolCallId":"{call}","title":"Write a file"{content}}},"options":[{{"optionId":"allow-once","name":"Allow","kind":"allow_once"}},{{"optionId":"reject-once","name":"Reject","kind":"reject_once"}}]}}}}"#
    )
}

/// A pending permission shows its call's diff while the call's row reads
/// `open=permission`, with either `tool_details`; allowed, the detail is
/// back in the running call's row. Content sent only in the request shows
/// too.
pub fn permission_detail(ctx: &Ctx) -> Result<Outcome> {
    let never = gate(ctx, "never");
    let profile = Script::ready(&ctx.dir.join("received.jsonl"))
        .step("expect session/prompt")
        .step(tool_call("call_1", "Edit notes.txt", "in_progress", &format!("[{NOTES_DIFF}]"), ""))
        .step(ask_with("p1", "call_1", ""))
        .step(r#"expect-response "p1""#)
        .step(format!("wait-file {}", never.display()))
        .profile(ctx, "detail")?;
    for (label, settings) in [("open", ""), ("closed", "\n[agent]\ntool_details = \"closed\"\n")] {
        let out = Run::new(ctx, label)
            .config(&format!("{settings}{profile}"))
            .agent("Fake")
            .first_prompt("hi")
            .wait_for("outcome=pending detail=1")
            .dump_at(5.0)
            .run()?;
        waited(&out)?;
        tool_line(&out, 2, "status=in_progress open=permission items=1")?;
        tool_line(&out, 3, "outcome=pending detail=1 \"Write a file\"")?;
        let items = out.dump.agent_items(2);
        ensure!(items == [NOTES_ITEM], "{label}: items {items:?}");
    }
    let out = Run::new(ctx, "allowed")
        .config(&profile)
        .agent("Fake")
        .first_prompt("hi")
        .wait_for("outcome=pending")
        .key("tab")
        .key("tab")
        .key("enter")
        .wait_for("outcome=selected:allow-once")
        .dump_at(7.0)
        .run()?;
    waited(&out)?;
    tool_line(&out, 3, "outcome=selected:allow-once \"Write a file\"")?;
    tool_line(&out, 2, "status=in_progress open=yes items=1")?;

    let profile = Script::ready(&ctx.dir.join("received.jsonl"))
        .step("expect session/prompt")
        .step(ask_with("p1", "call_2", &format!("[{NOTES_DIFF}]")))
        .step(r#"expect-response "p1""#)
        .profile(ctx, "request-only")?;
    let out = Run::new(ctx, "request-only")
        .config(&profile)
        .agent("Fake")
        .first_prompt("hi")
        .wait_for("outcome=pending detail=1")
        .dump_at(5.0)
        .run()?;
    waited(&out)?;
    ensure!(
        out.dump.agent_entries().iter().any(|l| l.starts_with("tool call=\"call_2\"") && l.contains("open=permission items=1")),
        "no tool entry from the request: {:?}",
        out.dump.agent
    );
    Ok(Outcome::Pass)
}

/// `[agent]`'s keys: `tool_details = "closed"` keeps a running call's row
/// closed and `tool_details_when_done = "open"` a finished one open; the
/// config rewritten while Brindle runs opens the finished row on its
/// reload; a bad value shows on the config line.
pub fn tool_settings(ctx: &Ctx) -> Result<Outcome> {
    let never = gate(ctx, "never");
    let profile = Script::ready(&ctx.dir.join("received.jsonl"))
        .step("expect session/prompt")
        .step(tool_call("c1", "Edit notes.txt", "in_progress", &format!("[{NOTES_DIFF}]"), ""))
        .step(tool_completed("c1"))
        .step(tool_call("c2", "Edit more.txt", "in_progress", &format!("[{NOTES_DIFF}]"), ""))
        .step(format!("wait-file {}", never.display()))
        .profile(ctx, "settings")?;
    let agent = |keys: &str| format!("\n[agent]\n{keys}\n");
    let both = agent("tool_details = \"closed\"\ntool_details_when_done = \"open\"");
    let started = |label, extra: &str| {
        Run::new(ctx, label).config(&format!("{extra}{profile}")).agent("Fake").first_prompt("hi").wait_for("call=\"c2\"")
    };

    let out = started("flipped", &both).dump_at(4.0).run()?;
    waited(&out)?;
    tool_line(&out, 2, "status=completed open=yes")?;
    tool_line(&out, 3, "status=in_progress open=no")?;

    let rewritten = ctx.dir.join("done-open.toml");
    let text = format!("{}{}{profile}", crate::brindle::config("e2e"), agent("tool_details_when_done = \"open\""));
    std::fs::write(&rewritten, text).map_err(|e| format!("writing {}: {e}", rewritten.display()))?;
    let copy = format!("cp '{}' '{}'", rewritten.display(), ctx.dir.join("config.toml").display());
    let out = from_shell(started("reloaded", "").wait_for("status=completed open=no"), &copy, true)
        .action("reload_config")
        .wait_for("status=completed open=yes")
        .dump_at(9.0)
        .run()?;
    waited(&out)?;
    tool_line(&out, 2, "status=completed open=yes")?;
    tool_line(&out, 3, "status=in_progress open=yes")?;

    // A config that doesn't load is the defaults', so no `Fake` profile: a
    // terminal tab (the implicit `Shell`) shows the line.
    let out = Run::new(ctx, "bad").config(&agent("tool_details = \"maybe\"")).dump_at(3.0).run()?;
    let line = out.dump.config.as_deref().unwrap_or_default();
    ensure!(line.contains("agent.tool_details") && line.contains("`open` or `closed`"), "config line {:?}", out.dump.config);
    Ok(Outcome::Pass)
}

/// `agent-tool-bounds`' output: lines of [`BOUNDS_LINE_BYTES`] bytes,
/// some 200 KiB in all, sent [`BOUNDS_PER_DELTA`] lines a `_meta` delta.
const BOUNDS_LINES: usize = 18_600;
const BOUNDS_PER_DELTA: usize = 400;
/// `out 000000\n`.
const BOUNDS_LINE_BYTES: usize = 11;
/// What an output item keeps (`brindle_transcript::ITEM_LIMIT`).
const BOUNDS_ITEM_LIMIT: usize = 64 * 1024;

/// A tool call's bounds (design decision 5): 200 KiB of output keeps its
/// newest whole lines within 64 KiB and counts the bytes it dropped, and
/// a call with 40 items keeps 32 and counts the rest.
pub fn tool_bounds(ctx: &Ctx) -> Result<Outcome> {
    let mut script = Script::ready(&ctx.dir.join("received.jsonl")).step("expect session/prompt").step(update(
        r#"{"sessionUpdate":"tool_call","toolCallId":"c1","title":"Run tests","kind":"execute","status":"in_progress","content":[{"type":"terminal","terminalId":"t1"}],"_meta":{"terminal_info":{"terminal_id":"t1","cwd":"/w","command":"cargo test"}}}"#,
    ));
    for start in (0..BOUNDS_LINES).step_by(BOUNDS_PER_DELTA) {
        let data: String = (start..(start + BOUNDS_PER_DELTA).min(BOUNDS_LINES)).map(|n| format!("out {n:06}\\n")).collect();
        script = script.step(tool_meta("c1", &format!(r#""terminal_output_delta":{{"terminal_id":"t1","data":"{data}"}}"#), ""));
    }
    let items: Vec<String> =
        (0..40).map(|n| format!(r#"{{"type":"content","content":{{"type":"text","text":"item {n}"}}}}"#)).collect();
    let profile = script
        .step(tool_meta("c1", r#""terminal_exit":{"terminal_id":"t1","exit_code":0,"signal":null}"#, r#","status":"completed""#))
        .step(tool_call("c2", "Many items", "completed", &format!("[{}]", items.join(",")), ""))
        .step(says("done"))
        .step(stop("end_turn"))
        .profile(ctx, "bounds")?;
    let out = Run::new(ctx, "run").config(&profile).agent("Fake").first_prompt("hi").wait_for(ENDED).dump_at(6.0).run()?;
    waited(&out)?;

    tool_line(&out, 2, "status=completed open=no items=1 summary=\"exit 0\"")?;
    let items = out.dump.agent_items(2);
    let item = match items.as_slice() {
        [item] => *item,
        _ => return Err(format!("items {items:?}")),
    };
    let omitted: usize = item
        .split(' ')
        .find_map(|field| field.strip_prefix("omitted="))
        .and_then(|n| n.parse().ok())
        .ok_or_else(|| format!("no omitted= in {item:.200}"))?;
    // The kept text is escaped onto the line: one `\n` per line kept.
    let kept = item.matches("\\n").count() * BOUNDS_LINE_BYTES;
    ensure!(
        omitted > 0 && kept <= BOUNDS_ITEM_LIMIT && omitted + kept == BOUNDS_LINES * BOUNDS_LINE_BYTES,
        "omitted={omitted} with {kept} bytes kept, of {}",
        BOUNDS_LINES * BOUNDS_LINE_BYTES
    );
    ensure!(item.ends_with(&format!("out {:06}\\n\"", BOUNDS_LINES - 1)), "the output's end: …{}", &item[item.len().saturating_sub(80)..]);

    tool_line(&out, 3, "status=completed open=no items=32 more=8 \"Many items\"")?;
    let items = out.dump.agent_items(3);
    ensure!(
        items.len() == 32 && items[31] == "index=31 kind=text cut=no \"item 31\"",
        "{} items, the last {:?}",
        items.len(),
        items.last()
    );
    Ok(Outcome::Pass)
}

/// A turn holding a running call with a diff and a pending permission
/// about it: the agent's script, recording what it was sent, never ends it.
fn diff_then_permission(ctx: &Ctx, name: &str, between: &[String]) -> Result<String> {
    between
        .iter()
        .cloned()
        .fold(
            Script::ready(&ctx.dir.join(format!("{name}.jsonl")))
                .step("expect session/prompt")
                .step(tool_call("c1", "Edit notes.txt", "in_progress", &format!("[{NOTES_DIFF}]"), "")),
            Script::step,
        )
        .step(ask_with("p1", "c1", ""))
        .step(format!("wait-file {}", gate(ctx, "never").display()))
        .profile(ctx, name)
}

/// What a pending permission showing its call's diff reads as.
const DIFF_PENDING: &str = "outcome=pending detail=1";

/// Quitting mid-turn with a diff and a pending permission, the next
/// launch restores the zone with both: the call's diff, shown with the
/// permission while it waits.
pub fn tool_restore(ctx: &Ctx) -> Result<Outcome> {
    let profile = diff_then_permission(ctx, "restore", &[])?;
    Run::new(ctx, "quit")
        .config(&profile)
        .agent("Fake")
        .first_prompt("hi")
        .keep_detached()
        .wait_for(DIFF_PENDING)
        .action("quit")
        .expect_exit()
        .dump_at(8.0)
        .run()?;
    let out = Run::new(ctx, "restored").config(&profile).restore().wait_for(DIFF_PENDING).dump_at(5.0).run()?;
    waited(&out)?;
    tool_line(&out, 2, &format!("tool call=\"c1\" kind=edit status=in_progress open=permission {NOTES_COUNTS} \"Edit notes.txt\""))?;
    let items = out.dump.agent_items(2);
    ensure!(items == [NOTES_ITEM], "items {items:?}");
    let hunks = out.dump.agent_hunks(2);
    ensure!(hunks.len() == 1 && hunks[0].contains("\\n-an apple a day\\n+an banana a day\\n"), "hunks {hunks:?}");
    tool_line(&out, 3, "permission call=\"c1\" options=")?;
    tool_line(&out, 3, &format!("{DIFF_PENDING} \"Write a file\""))?;
    Ok(Outcome::Pass)
}

/// The entries between `agent-tool-page`'s call and its permission: 80
/// thoughts of 5 KiB and 80 short messages, some 400 KiB, more than a
/// page (`PAGE_BYTES`, 256 KiB).
const PAGE_BETWEEN: usize = 160;

/// A session waiting on a permission whose tool call is more than a page
/// back (design decision 15): attached again, the zone's first entry is
/// that call, with the entry before it left to fetch, and the permission
/// shows its diff.
pub fn tool_page(ctx: &Ctx) -> Result<Outcome> {
    let turn = long_turn(PAGE_BETWEEN, LONG_SIZE, None);
    // `long_turn`'s entries, without its prompt and its end.
    let profile = diff_then_permission(ctx, "page", &turn[1..turn.len() - 1])?;
    let out = Run::new(ctx, "live")
        .config(&profile)
        .agent("Fake")
        .first_prompt("go")
        .keep_detached()
        .wait_for(DIFF_PENDING)
        .dump_at(8.0)
        .run()?;
    waited(&out)?;
    ensure!(earlier(&out) == Some("none"), "a live zone holds the whole conversation: {:?}", out.dump.agent.last());

    let out = Run::new(ctx, "attached").config(&profile).restore().wait_for(DIFF_PENDING).dump_at(6.0).run()?;
    waited(&out)?;
    let ids = entry_ids(&out);
    let asked = 2 + PAGE_BETWEEN as u64 + 1;
    ensure!(
        ids.first() == Some(&2) && ids.last() == Some(&asked),
        "entry ids {:?}…{:?}, not 2…{asked}",
        ids.first(),
        ids.last()
    );
    ensure!(earlier(&out) == Some("1"), "earlier={:?} with the call first", earlier(&out));
    tool_line(&out, 2, &format!("status=in_progress open=permission {NOTES_COUNTS}"))?;
    ensure!(out.dump.agent_items(2) == [NOTES_ITEM], "items {:?}", out.dump.agent_items(2));
    tool_line(&out, asked, &format!("{DIFF_PENDING} \"Write a file\""))?;
    Ok(Outcome::Pass)
}

/// A dark profile theme in a light window (ui-theme-tokens' "A profile
/// theme unlike the window's"): an agent profile with the `e2e` theme in a
/// `brindle-light` window draws a diff's added and removed lines in the
/// `e2e` zone set's `diff_added` and `diff_removed`, as an `e2e` window's
/// agent zone does.
pub fn tool_theme(ctx: &Ctx) -> Result<Outcome> {
    let profile = Script::ready(&ctx.dir.join("received.jsonl"))
        .step("expect session/prompt")
        .step(tool_call("c1", "Edit notes.txt", "in_progress", &format!("[{NOTES_DIFF}]"), ""))
        .step(format!("wait-file {}", gate(ctx, "never").display()))
        .profile(ctx, "theme")?;
    let out = Run::new(ctx, "run")
        .initial_theme("brindle-light")
        .config(&format!("{profile}theme = \"e2e\"\n"))
        .agent("Fake")
        .first_prompt("hi")
        .wait_for("open=yes items=1")
        .capture_at(5.0)
        .dump_at(5.5)
        .run()?;
    waited(&out)?;
    tool_line(&out, 2, &format!("status=in_progress open=yes {NOTES_COUNTS}"))?;
    let image = out.image.as_ref().ok_or("no capture")?;
    // The zone's card border is its profile theme's (`e2e`).
    let scale = window_scale(image, crate::brindle::theme::CHROME_BORDER)?;
    let zone = zone_inner(image, scale, [0.0, 0.0, 1.0, 1.0]);
    for (what, colour) in [("added", ZONE_DIFF_ADDED), ("removed", ZONE_DIFF_REMOVED)] {
        let count = image.count_near(zone, colour, TOKEN_NEAR);
        ensure!(
            count as f32 >= 2_000.0 * scale * scale,
            "the {what} line: {count} pixels of {} in {zone:?}; found {}",
            colour.hex(),
            image.summary(zone)
        );
    }
    Ok(Outcome::Pass)
}

// A session being set up (OpenSpec change `agents-client-state-fixes`, D-1):
// `BRINDLE_TEST_SESSION_START_DELAY` holds the session's thread between the
// core's answer and the running session, while the reader folds the agent's
// `Ready`, so the zone shows the agent ready before it can send.

/// The session start delay, in milliseconds: long enough that the steps
/// after the agent shows ready fall inside it, stretched with the other
/// delays.
fn start_delay(ctx: &Ctx) -> String {
    format!("{:.0}", ctx.scale().secs(3.0) * 1000.0)
}

/// The agent lines that show a refusal beside the prompt input.
fn refusals(out: &crate::brindle::Output) -> Vec<&String> {
    out.dump.agent.iter().filter(|line| line.contains("refusal=")).collect()
}

/// A launch's first prompt waits for the session to run, then goes once:
/// never refused because the session was still being set up.
pub fn first_prompt_before_the_session_runs(ctx: &Ctx) -> Result<Outcome> {
    let (profile, received) = hello_agent(ctx)?;
    let out = Run::new(ctx, "run")
        .config(&profile)
        .agent("Fake")
        .first_prompt("one")
        .env("BRINDLE_TEST_SESSION_START_DELAY", &start_delay(ctx))
        .wait_for(&answered())
        .dump_at(8.0)
        .run()?;
    waited(&out)?;
    ensure!(refusals(&out).is_empty(), "refused: {:?}", refusals(&out));
    ensure!(out.dump.agent_entries() == ["user \"one\"", "agent \"Hello!\""], "entries: {:?}", out.dump.agent);
    ensure!(prompts_sent(&received)? == ["one"], "sent: {:?}", prompts_sent(&received)?);
    Ok(Outcome::Pass)
}

/// An attached zone whose replay shows the agent ready, while its session
/// is still being set up, takes no prompt: the input says it is
/// connecting. Once the session runs it is ready, and enter sends the
/// prompt, once, with no refusal. (Before the fix the input was ready
/// during the delay, and enter was refused.)
pub fn prompt_before_the_attached_session_runs(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("received.jsonl");
    let profile = Script(hello_script(&received).lines().map(str::to_string).collect())
        .step("expect session/prompt")
        .step(says("again"))
        .step(stop("end_turn"))
        .profile(ctx, "attach")?;
    let out = Run::new(ctx, "first")
        .config(&profile)
        .agent("Fake")
        .first_prompt("one")
        .keep_detached()
        .wait_for(&answered())
        .dump_at(4.0)
        .run()?;
    waited(&out)?;
    let out = Run::new(ctx, "attached")
        .config(&profile)
        .restore()
        .step_interval(0.25)
        .env("BRINDLE_TEST_SESSION_START_DELAY", &start_delay(ctx))
        .wait_for("prompt status=ready")
        .send("hi")
        .key("enter")
        .wait_for("user \"hi\"")
        .dump_at(10.0)
        .run()?;
    waited(&out)?;
    ensure!(refusals(&out).is_empty(), "refused: {:?}", refusals(&out));
    ensure!(prompts_sent(&received)? == ["one", "hi"], "sent: {:?}", prompts_sent(&received)?);
    Ok(Outcome::Pass)
}
