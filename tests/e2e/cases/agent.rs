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
fn hello_agent(ctx: &Ctx) -> Result<(String, PathBuf)> {
    let (script, received) = (ctx.dir.join("agent.script"), ctx.dir.join("received.jsonl"));
    std::fs::write(&script, hello_script(&received)).map_err(|e| format!("writing the script: {e}"))?;
    Ok((fake_profile(ctx, &script), received))
}

/// The waits for the answer to the first prompt.
fn answered() -> String {
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
