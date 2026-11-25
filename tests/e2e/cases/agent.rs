//! Agent tabs (OpenSpec change `agents-profiles`): each case plays an agent
//! with the fake ACP agent (`Case::needs_fake_agent`), from a script it
//! writes into its case directory.

use std::path::{Path, PathBuf};

use super::cli::usage_refusal;
use super::instance::Launcher;
use crate::brindle::Run;
use crate::{Ctx, Outcome, Result, ensure};

/// What a turn's answer says.
const HELLO: &str = "Hello!";

/// The fake agent's script for one agent that is ready at once and
/// answers its first prompt with [`HELLO`], recording what it was sent in
/// `received.jsonl`, then idles until its session ends.
fn hello_script(received: &Path) -> String {
    let chunk = format!(
        r#"{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"s1","update":{{"sessionUpdate":"agent_message_chunk","content":{{"type":"text","text":"{HELLO}"}}}}}}}}"#
    );
    [
        format!("record {}", received.display()),
        "expect initialize".into(),
        r#"respond {"protocolVersion":1,"agentInfo":{"name":"Fake Agent","version":"1.0"}}"#.into(),
        "expect session/new".into(),
        r#"respond {"sessionId":"s1"}"#.into(),
        "expect session/prompt".into(),
        format!("send {chunk}"),
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
