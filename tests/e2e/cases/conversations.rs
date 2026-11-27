//! An agent's earlier conversations (OpenSpec change `agents-session-list`):
//! the conversations palette, the choosing screen and `brindle agent -c`
//! and `-r`, each played by the fake ACP agent answering `session/list`.

use std::path::Path;

use serde_json::{Value, json};

use super::agent::{
    EXITED, READY, Script, answered, continued, from_shell, gate, recorded_count, records,
    sessions_listed, touch, turn_then_exit, waited,
};
use super::cli::usage_refusal;
use crate::brindle::{Output, Run};
use crate::{Ctx, Outcome, Result, ensure};

/// `agentCapabilities` offering `session/list` and `session/resume`.
const LISTS: &str = r#"{"sessionCapabilities":{"list":{},"resume":{}}}"#;

/// What `--wait-for` waits for once the palette has listed.
const LISTED: &str = "state=listed";

/// The case directory, where every agent here works and every listed
/// conversation was had.
fn dir(ctx: &Ctx) -> String {
    ctx.dir.display().to_string()
}

/// A conversation of the case directory, as `session/list` gives one.
fn entry(ctx: &Ctx, id: &str, title: Option<&str>) -> Value {
    let mut entry = json!({ "sessionId": id, "cwd": dir(ctx) });
    if let Some(title) = title {
        entry["title"] = json!(title);
    }
    entry
}

/// A `respond` step answering `session/list` with `sessions` and `next`.
fn listed(sessions: Vec<Value>, next: Option<&str>) -> String {
    let mut result = json!({ "sessions": sessions });
    if let Some(next) = next {
        result["nextCursor"] = json!(next);
    }
    format!("respond {result}")
}

/// An agent that offers `capabilities`, records what it is sent in
/// `received`, and is ready with conversation `s1`.
fn ready_agent(received: &Path, capabilities: &str) -> Script {
    Script::initialized(received, capabilities)
        .step("expect session/new")
        .step(r#"respond {"sessionId":"s1"}"#)
}

/// An agent started to choose: it lists `sessions`, then continues the
/// one chosen by `session/resume`.
fn chooser(received: &Path, sessions: Vec<Value>) -> Script {
    Script::initialized(received, LISTS)
        .step("expect session/list")
        .step(listed(sessions, None))
        .step("expect session/resume")
        .step("respond {}")
}

/// The conversations palette's own line (`--- palette conversations …`),
/// without its prefix.
fn conversations_line(out: &Output) -> Result<&str> {
    out.dump
        .palette
        .iter()
        .find_map(|line| line.strip_prefix("conversations "))
        .ok_or_else(|| format!("no conversations line in {:?}", out.dump.palette))
}

/// The open palette's rows, after its list header.
fn rows(out: &Output) -> Vec<&str> {
    let after = out
        .dump
        .palette
        .iter()
        .position(|line| line.starts_with("list "))
        .map_or(usize::MAX, |at| at + 1);
    out.dump
        .palette
        .iter()
        .skip(after)
        .map(String::as_str)
        .collect()
}

/// The `--- prompt` line of the agent zone, without its prefix.
fn prompt_line(out: &Output) -> &str {
    out.dump
        .agent
        .iter()
        .find_map(|line| line.strip_prefix("prompt "))
        .unwrap_or("")
}

/// Whether the agent's record has `method` with `id` as its `sessionId`.
fn continued_with(received: &Path, method: &str, id: &str) -> bool {
    let text = std::fs::read_to_string(received).unwrap_or_default();
    text.lines().any(|l| {
        l.contains(&format!("\"method\":\"{method}\""))
            && l.contains(&format!("\"sessionId\":\"{id}\""))
    })
}

/// 7.1: from an agent zone, its own agent lists (no second start): titled
/// and untitled rows, "More…" appending the next page; from a shell, an
/// agent started for the list, an empty directory saying so; and with two
/// agent profiles, the question which first.
pub fn list(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("received.jsonl");
    let profile = ready_agent(&received, LISTS)
        .step("expect session/list")
        .step(listed(
            vec![
                entry(ctx, "c1", Some("Fix the flaky upload test")),
                entry(ctx, "7f3a9c00", None),
            ],
            Some("p2"),
        ))
        .step("expect session/list")
        .step(listed(vec![entry(ctx, "c3", Some("Third"))], None))
        .profile(ctx, "zone")?;
    let out = Run::new(ctx, "zone")
        .config(&profile)
        .agent("Fake")
        .arg("-d")
        .arg(&dir(ctx))
        .wait_for(READY)
        .action("agent_conversations")
        .wait_for("state=listed more=yes")
        .action("list_last")
        .action("picker_confirm")
        .wait_for("more=no")
        .dump_at(10.0)
        .run()?;
    waited(&out)?;
    let line = conversations_line(&out)?;
    let name = ctx
        .dir
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    ensure!(
        line.starts_with("profile=\"Fake\" dir=") && line.contains(&name),
        "the line names the profile and directory: {line}"
    );
    ensure!(line.ends_with("state=listed more=no"), "{line}");
    ensure!(
        rows(&out) == ["Fix the flaky upload test", "Untitled 7f3a9c", "Third"],
        "rows: {:?}",
        out.dump.palette
    );
    ensure!(
        recorded_count(&received, "initialize") == 1,
        "the zone's own agent listed"
    );

    let received = ctx.dir.join("empty.jsonl");
    let profile = Script::initialized(&received, LISTS)
        .step("expect session/list")
        .step(listed(Vec::new(), None))
        .profile(ctx, "empty")?;
    let out = Run::new(ctx, "empty")
        .config(&profile)
        .arg("-d")
        .arg(&dir(ctx))
        .action("agent_continue_conversation")
        .wait_for("state=empty")
        .dump_at(5.0)
        .run()?;
    waited(&out)?;
    ensure!(
        conversations_line(&out)?.ends_with("state=empty more=no"),
        "{:?}",
        out.dump.palette
    );
    ensure!(rows(&out).is_empty(), "no rows: {:?}", out.dump.palette);
    ensure!(
        recorded_count(&received, "session/new") == 0,
        "the chooser started no conversation"
    );

    let other = format!(
        "\n[[profiles]]\nname = \"Other\"\ntype = \"agent\"\ncommand = '{}'\nargs = ['{}']\n",
        ctx.fake_agent.display(),
        ctx.dir.join("empty.script").display()
    );
    let out = Run::new(ctx, "question")
        .config(&(profile + &other))
        .action("agent_continue_conversation")
        .dump_at(3.0)
        .run()?;
    let asked: Vec<&str> = rows(&out)
        .iter()
        .map(|row| row.split("  ").next().unwrap_or(""))
        .collect();
    ensure!(
        asked == ["Fake", "Other"],
        "the agent profiles asked: {:?}",
        out.dump.palette
    );
    ensure!(conversations_line(&out).is_err(), "nothing listed yet");
    Ok(Outcome::Pass)
}

/// 7.2: from a shell a chosen conversation asks where (Split Right first,
/// escape back to the list), then opens in a split or a new tab through the
/// agent that listed it (one start); one a zone shows is marked "open" and
/// shows that zone; from a running agent zone, the question.
pub fn open(ctx: &Ctx) -> Result<Outcome> {
    let from_shell_listing = |label: &'static str, received: &Path| -> Result<Run<'_>> {
        let profile =
            chooser(received, vec![entry(ctx, "c1", Some("First"))]).profile(ctx, label)?;
        Ok(Run::new(ctx, label)
            .config(&profile)
            .arg("-d")
            .arg(&dir(ctx))
            .action("agent_continue_conversation")
            .wait_for(LISTED))
    };
    let received = ctx.dir.join("ask.jsonl");
    let out = from_shell_listing("ask", &received)?
        .action("picker_confirm")
        .dump_at(6.0)
        .run()?;
    waited(&out)?;
    ensure!(
        rows(&out) == ["Split Right", "Split Down", "New Tab"],
        "the question: {:?}",
        out.dump.palette
    );
    ensure!(
        out.dump.palette_highlighted()? == Some(0),
        "Split Right first: {:?}",
        out.dump.palette
    );
    ensure!(
        conversations_line(&out).is_err(),
        "the question has no list line"
    );

    let out = from_shell_listing("back", &ctx.dir.join("back.jsonl"))?
        .action("picker_confirm")
        .key("escape")
        .dump_at(7.0)
        .run()?;
    ensure!(
        conversations_line(&out)?.contains(LISTED) && rows(&out) == ["First"],
        "the list again: {:?}",
        out.dump.palette
    );

    for (label, place) in [("split", None), ("tab", Some("list_last"))] {
        let received = ctx.dir.join(format!("{label}.jsonl"));
        let run = from_shell_listing(label, &received)?.action("picker_confirm");
        let run = match place {
            Some(action) => run.action(action),
            None => run,
        };
        let out = run
            .action("picker_confirm")
            .wait_for(READY)
            .dump_at(10.0)
            .run()?;
        waited(&out)?;
        ensure!(
            out.dump.agent_field("state") == Some("ready"),
            "{label}: {:?}",
            out.dump.agent
        );
        match place {
            None => ensure!(
                out.dump
                    .zones
                    .as_deref()
                    .is_some_and(|z| z.starts_with("count=2")),
                "a split: {:?}",
                out.dump.zones
            ),
            Some(_) => ensure!(
                out.dump.tabs.len() == 2 && out.dump.tabs[1].1,
                "a new tab, active: {:?}",
                out.dump.tabs
            ),
        }
        ensure!(
            recorded_count(&received, "initialize") == 1,
            "{label}: one start for the list and the choice"
        );
        ensure!(
            continued_with(&received, "session/resume", "c1"),
            "{label}: c1 wasn't resumed"
        );
        ensure!(
            recorded_count(&received, "session/new") == 0,
            "{label}: no new conversation"
        );
    }

    // A conversation a zone shows: marked, and shown.
    let shown = |label: &'static str| -> Result<Run<'_>> {
        let received = ctx.dir.join(format!("{label}.jsonl"));
        let profile = ready_agent(&received, LISTS)
            .step("expect session/list")
            .step(listed(vec![entry(ctx, "s1", Some("Current"))], None))
            .profile(ctx, label)?;
        Ok(Run::new(ctx, label)
            .config(&profile)
            .agent("Fake")
            .arg("-d")
            .arg(&dir(ctx))
            .wait_for(READY)
            .action("new_tab")
            .action("agent_continue_conversation")
            .wait_for(LISTED))
    };
    let out = shown("marked")?.dump_at(7.0).run()?;
    waited(&out)?;
    ensure!(
        rows(&out) == ["Current  open"],
        "the open mark: {:?}",
        out.dump.palette
    );
    let out = shown("shown")?
        .action("picker_confirm")
        .dump_at(8.0)
        .run()?;
    waited(&out)?;
    ensure!(
        out.dump.tabs.len() == 2 && out.dump.tabs[0].1,
        "the zone's tab is active: {:?}",
        out.dump.tabs
    );
    ensure!(
        out.dump.focus.as_deref() == Some("agent"),
        "the zone has focus: {:?}",
        out.dump.focus
    );
    ensure!(
        recorded_count(&ctx.dir.join("shown.jsonl"), "initialize") == 1,
        "nothing was started"
    );

    let received = ctx.dir.join("running.jsonl");
    let profile = ready_agent(&received, LISTS)
        .step("expect session/list")
        .step(listed(vec![entry(ctx, "c9", Some("Other"))], None))
        .profile(ctx, "running")?;
    let out = Run::new(ctx, "running")
        .config(&profile)
        .agent("Fake")
        .arg("-d")
        .arg(&dir(ctx))
        .wait_for(READY)
        .action("agent_conversations")
        .wait_for(LISTED)
        .action("picker_confirm")
        .dump_at(7.0)
        .run()?;
    waited(&out)?;
    ensure!(
        rows(&out) == ["Split Right", "Split Down", "New Tab"],
        "a running agent asks: {:?}",
        out.dump.palette
    );
    Ok(Outcome::Pass)
}

/// 7.3: an exited agent's zone shows Continue conversation,
/// Conversations… and New conversation; Conversations… and a choice take
/// the conversation in place; New conversation starts over in a new
/// session; a profile removed from the config offers none.
pub fn stopped(ctx: &Ctx) -> Result<Outcome> {
    let go = gate(ctx, "exit");
    let (first, _) = records(ctx);
    let (profile, swap) = continued(
        ctx,
        turn_then_exit(&first, LISTS, &go),
        ready_agent(&ctx.dir.join("unused.jsonl"), LISTS),
        &touch(&go),
    )?;
    let run = Run::new(ctx, "buttons")
        .config(&profile)
        .agent("Fake")
        .arg("-d")
        .arg(&dir(ctx))
        .first_prompt("hi")
        .wait_for(&answered());
    let out = from_shell(run, &swap, true)
        .wait_for(EXITED)
        .key("tab")
        .key("tab")
        .dump_at(9.0)
        .run()?;
    waited(&out)?;
    ensure!(
        prompt_line(&out).ends_with("actions=continue,conversations,new"),
        "the buttons: {:?}",
        out.dump.agent
    );
    ensure!(
        out.dump.focus_stop.as_deref() == Some("conversations"),
        "the second stop: {:?}",
        out.dump.focus_stop
    );

    // Conversations…, then a choice: the zone shows the agent that listed.
    let (first, second) = records(ctx);
    let (profile, swap) = continued(
        ctx,
        turn_then_exit(&first, LISTS, &go),
        chooser(&second, vec![entry(ctx, "c1", Some("First"))]),
        &touch(&go),
    )?;
    let run = Run::new(ctx, "here")
        .config(&profile)
        .agent("Fake")
        .arg("-d")
        .arg(&dir(ctx))
        .first_prompt("hi")
        .wait_for(&answered());
    let out = from_shell(run, &swap, true)
        .wait_for(EXITED)
        .key("tab")
        .key("tab")
        .key("enter")
        .wait_for(LISTED)
        .action("picker_confirm")
        .wait_for(READY)
        .dump_at(14.0)
        .run()?;
    waited(&out)?;
    ensure!(
        out.dump.zones.is_none() && out.dump.tabs.len() == 2,
        "in place, no split: {:?} {:?}",
        out.dump.zones,
        out.dump.tabs
    );
    ensure!(
        out.dump.agent_field("state") == Some("ready"),
        "{:?}",
        out.dump.agent
    );
    ensure!(
        continued_with(&second, "session/resume", "c1"),
        "c1 wasn't resumed"
    );
    ensure!(
        recorded_count(&second, "initialize") == 1,
        "one start for the list and the choice"
    );

    // New conversation: a new session, no conversation continued.
    let (first, second) = records(ctx);
    let (profile, swap) = continued(
        ctx,
        turn_then_exit(&first, LISTS, &go),
        ready_agent(&second, LISTS),
        &touch(&go),
    )?;
    let run = Run::new(ctx, "new")
        .config(&profile)
        .agent("Fake")
        .arg("-d")
        .arg(&dir(ctx))
        .first_prompt("hi")
        .wait_for(&answered());
    let out = from_shell(run, &swap, true)
        .wait_for(EXITED)
        .key("tab")
        .key("tab")
        .key("tab")
        .key("enter")
        .wait_for(READY)
        .dump_at(12.0)
        .run()?;
    waited(&out)?;
    ensure!(
        recorded_count(&second, "session/new") == 1
            && recorded_count(&second, "session/resume") == 0,
        "a new conversation"
    );
    ensure!(
        out.dump.agent_entries().is_empty(),
        "the old conversation went: {:?}",
        out.dump.agent
    );

    // The profile gone: no buttons.
    let (first, _) = records(ctx);
    let gone = ctx.dir.join("gone.toml");
    std::fs::write(&gone, crate::brindle::config("e2e"))
        .map_err(|e| format!("writing {}: {e}", gone.display()))?;
    let profile = turn_then_exit(&first, LISTS, &go).profile(ctx, "agent")?;
    let drop = format!(
        "cp '{}' '{}'",
        gone.display(),
        ctx.dir.join("config.toml").display()
    );
    let run = Run::new(ctx, "gone")
        .config(&profile)
        .agent("Fake")
        .first_prompt("hi")
        .wait_for(&answered());
    let run = from_shell(run, &touch(&go), true).wait_for(EXITED);
    let out = from_shell(run, &drop, false)
        .wait_for("continue=none:profile-gone")
        .dump_at(12.0)
        .run()?;
    waited(&out)?;
    ensure!(
        !prompt_line(&out).contains("actions="),
        "no buttons: {:?}",
        out.dump.agent
    );
    Ok(Outcome::Pass)
}

/// 7.4 for both platforms: `brindle agent Fake -c` continues the first
/// listed conversation, with one start of the agent.
pub fn continue_latest(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("latest.jsonl");
    let profile = chooser(
        &received,
        vec![entry(ctx, "c1", Some("First")), entry(ctx, "c2", None)],
    )
    .profile(ctx, "latest")?;
    let out = Run::new(ctx, "latest")
        .config(&profile)
        .agent("Fake")
        .arg("-d")
        .arg(&dir(ctx))
        .arg("-c")
        .wait_for(READY)
        .dump_at(5.0)
        .run()?;
    waited(&out)?;
    ensure!(
        out.dump.agent_field("state") == Some("ready"),
        "{:?}",
        out.dump.agent
    );
    ensure!(
        continued_with(&received, "session/resume", "c1"),
        "the first listed wasn't continued"
    );
    ensure!(
        recorded_count(&received, "initialize") == 1
            && recorded_count(&received, "session/new") == 0,
        "one start, no new conversation"
    );
    Ok(Outcome::Pass)
}

/// 7.4: `-c`, and with none listed the choosing screen with its reason,
/// then New conversation; `-r` opening the list on the zone, a choice
/// taken in it, escape leaving the choosing screen; `-c -r` a usage error;
/// a quit while choosing leaving nothing to restore.
pub fn cli(ctx: &Ctx) -> Result<Outcome> {
    continue_latest(ctx)?;
    let none = |received: &Path, label: &'static str| -> Result<String> {
        Script::initialized(received, LISTS)
            .step("expect session/list")
            .step(listed(Vec::new(), None))
            .step("expect session/new")
            .step(r#"respond {"sessionId":"s9"}"#)
            .profile(ctx, label)
    };
    let received = ctx.dir.join("none.jsonl");
    let reason = "reason=\"no earlier conversation in ";
    let out = Run::new(ctx, "none")
        .config(&none(&received, "none")?)
        .agent("Fake")
        .arg("-d")
        .arg(&dir(ctx))
        .arg("-c")
        .wait_for(reason)
        .key("tab")
        .dump_at(5.0)
        .run()?;
    waited(&out)?;
    ensure!(
        out.dump.agent_field("state") == Some("choosing"),
        "{:?}",
        out.dump.agent
    );
    ensure!(
        prompt_line(&out).contains(reason)
            && prompt_line(&out).ends_with("actions=conversations,new"),
        "{:?}",
        out.dump.agent
    );
    ensure!(
        out.dump.focus_stop.as_deref() == Some("conversations"),
        "{:?}",
        out.dump.focus_stop
    );
    let received = ctx.dir.join("anew.jsonl");
    let out = Run::new(ctx, "anew")
        .config(&none(&received, "anew")?)
        .agent("Fake")
        .arg("-d")
        .arg(&dir(ctx))
        .arg("-c")
        .wait_for(reason)
        .key("tab")
        .key("tab")
        .key("enter")
        .wait_for(READY)
        .dump_at(7.0)
        .run()?;
    waited(&out)?;
    ensure!(
        recorded_count(&received, "session/new") == 1,
        "New conversation started one"
    );

    let list = |label: &'static str| -> Result<Run<'_>> {
        let received = ctx.dir.join(format!("{label}.jsonl"));
        let profile =
            chooser(&received, vec![entry(ctx, "c1", Some("First"))]).profile(ctx, label)?;
        Ok(Run::new(ctx, label)
            .config(&profile)
            .agent("Fake")
            .arg("-d")
            .arg(&dir(ctx))
            .arg("-r")
            .wait_for(LISTED))
    };
    let out = list("resume")?
        .action("picker_confirm")
        .wait_for(READY)
        .dump_at(6.0)
        .run()?;
    waited(&out)?;
    ensure!(
        out.dump.tabs.len() == 1 && out.dump.zones.is_none(),
        "taken in the tab: {:?}",
        out.dump.tabs
    );
    ensure!(
        continued_with(&ctx.dir.join("resume.jsonl"), "session/resume", "c1"),
        "c1 wasn't resumed"
    );
    let out = list("escape")?.key("escape").dump_at(5.0).run()?;
    waited(&out)?;
    ensure!(
        out.dump.palette.is_empty() && out.dump.agent_field("state") == Some("choosing"),
        "{:?} {:?}",
        out.dump.palette,
        out.dump.agent
    );
    ensure!(
        prompt_line(&out).contains("reason=\"choose a conversation, or start a new one\""),
        "{:?}",
        out.dump.agent
    );

    let said = usage_refusal(
        ctx,
        "both",
        &ctx.dir.join("config.toml"),
        &["agent", "Fake", "-c", "-r"],
    )?;
    ensure!(said.contains("-c and -r can't be given together"), "{said}");

    // A quit while choosing closes the session: nothing is left detached.
    let out = list("quit")?
        .key("escape")
        .dump_at(4.0)
        .keep_detached()
        .run()?;
    waited(&out)?;
    sessions_listed(ctx, 0)?;
    Ok(Outcome::Pass)
}

/// 7.5: an agent that can't list, one whose `session/list` fails, and a
/// refused continue sending a choosing zone back to the choosing screen
/// with the agent's words.
pub fn failures(ctx: &Ctx) -> Result<Outcome> {
    let unlisted = ready_agent(
        &ctx.dir.join("unlisted.jsonl"),
        r#"{"sessionCapabilities":{"resume":{}}}"#,
    )
    .profile(ctx, "unlisted")?;
    let out = Run::new(ctx, "unlisted")
        .config(&unlisted)
        .agent("Fake")
        .wait_for(READY)
        .action("agent_conversations")
        .wait_for("state=failed")
        .dump_at(5.0)
        .run()?;
    waited(&out)?;
    ensure!(
        conversations_line(&out)?.ends_with("why=\"the agent can't list conversations\""),
        "{:?}",
        out.dump.palette
    );

    let error = ready_agent(&ctx.dir.join("error.jsonl"), LISTS)
        .step("expect session/list")
        .step(r#"respond-error -32603 "the index is locked""#)
        .profile(ctx, "error")?;
    let out = Run::new(ctx, "error")
        .config(&error)
        .agent("Fake")
        .wait_for(READY)
        .action("agent_conversations")
        .wait_for("state=failed")
        .dump_at(5.0)
        .run()?;
    waited(&out)?;
    ensure!(
        conversations_line(&out)?.contains("the index is locked"),
        "the agent's words: {:?}",
        out.dump.palette
    );

    let refused = Script::initialized(&ctx.dir.join("refused.jsonl"), LISTS)
        .step("expect session/list")
        .step(listed(vec![entry(ctx, "c1", Some("First"))], None))
        .step("expect session/resume")
        .step(r#"respond-error -32603 "c1 is gone""#)
        .profile(ctx, "refused")?;
    let out = Run::new(ctx, "refused")
        .config(&refused)
        .agent("Fake")
        .arg("-d")
        .arg(&dir(ctx))
        .arg("-c")
        .wait_for("c1 is gone")
        .dump_at(5.0)
        .run()?;
    waited(&out)?;
    ensure!(
        out.dump.agent_field("state") == Some("choosing"),
        "back to choosing: {:?}",
        out.dump.agent
    );
    ensure!(
        prompt_line(&out).contains("c1 is gone")
            && prompt_line(&out).ends_with("actions=conversations,new"),
        "{:?}",
        out.dump.agent
    );
    Ok(Outcome::Pass)
}
