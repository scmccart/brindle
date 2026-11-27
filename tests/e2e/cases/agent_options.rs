//! An agent's slash commands and its session's options (OpenSpec change
//! `agents-commands`): the completion list above the prompt input,
//! Brindle's option commands, the options palette, `agent_next_mode`, the
//! refusals, and the lists across a relaunch and a continue, each played by
//! the fake ACP agent.

use std::path::Path;

use serde_json::{Value, json};

use super::agent::{
    EXITED, READY, RESUME, Script, answered, continued, from_shell, gate, has_notice, messages, prompts_sent,
    recorded_count, records, says, stop, touch, waited,
};
use crate::brindle::{Output, Run};
use crate::{Ctx, Outcome, Result, ensure};

/// What `--wait-for` waits for while a turn runs.
const WORKING: &str = "turn=working";

/// What `--wait-for` waits for once the agent's lists are in: its two
/// commands and the model option at Opus.
const LISTED: &str = "commands count=2";

/// How long a notice shows on the window's config line
/// (`workspace::notices::NOTICE_LIFETIME`, 10 s), on Brindle's own clock;
/// the dump comes this long after the last step, well within it.
const NOTICE_SHOWN: f64 = 3.0;

/// A `send` step of a `session/update` whose `update` is `update`.
fn update(update: Value) -> String {
    format!("send {}", json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s1", "update": update}}))
}

/// v1's `modes`, in mode `current`: Default and Plan.
fn modes_in(current: &str) -> Value {
    json!({"currentModeId": current, "availableModes": [
        {"id": "default", "name": "Default"}, {"id": "plan", "name": "Plan"}]})
}

/// The config options: a model select at `model` and a `fast` toggle `fast`.
fn config_options(model: &str, fast: bool) -> Value {
    json!([
        {"id": "model", "name": "Model", "category": "model", "type": "select", "currentValue": model,
         "options": [{"value": "opus", "name": "Opus"}, {"value": "sonnet", "name": "Sonnet"}]},
        {"id": "fast", "name": "Fast", "category": "speed", "type": "boolean", "currentValue": fast}])
}

/// A `send` step of the agent's commands `names`, each with a description.
fn commands_step(names: &[&str]) -> String {
    let commands: Vec<Value> =
        names.iter().map(|name| json!({"name": name, "description": format!("Runs {name}")})).collect();
    update(json!({"sessionUpdate": "available_commands_update", "availableCommands": commands}))
}

/// An agent offering `capabilities` that answers `session/new` with the
/// options in mode `default` (the model at Opus, Fast off) and sends the
/// commands `names`; the case adds the rest.
fn offering(received: &Path, capabilities: &str, names: &[&str]) -> Script {
    let answer = json!({"sessionId": "s1", "modes": modes_in("default"), "configOptions": config_options("opus", false)});
    Script::initialized(received, capabilities)
        .step("expect session/new")
        .step(format!("respond {answer}"))
        .step(commands_step(names))
}

/// The `--- option` line of option `id`, without its `option id="ID" `.
fn option_line<'a>(out: &'a Output, id: &str) -> Option<&'a str> {
    let prefix = format!("option id={id:?} ");
    out.dump.agent.iter().find_map(|line| line.strip_prefix(prefix.as_str()))
}

/// The `--- completion` line, without its `completion`.
fn completion(out: &Output) -> Option<&str> {
    out.dump.agent.iter().find_map(|line| line.strip_prefix("completion"))
}

/// The prompt input's text.
fn input(out: &Output) -> Option<&str> {
    out.dump.input.as_ref().map(|input| input.text.as_str())
}

/// The `params` of each message with `method` the agent's record holds.
fn params_of(received: &Path, method: &str) -> Result<Vec<Value>> {
    let text = std::fs::read_to_string(received).map_err(|e| format!("reading {}: {e}", received.display()))?;
    let mut found = Vec::new();
    for line in text.lines() {
        let value: Value = serde_json::from_str(line).map_err(|e| format!("{line}: {e}"))?;
        if value["method"] == method {
            found.push(value["params"].clone());
        }
    }
    Ok(found)
}

/// The values of option `config` the agent was asked to set, in order.
fn config_sets(received: &Path, config: &str) -> Result<Vec<Value>> {
    let sets = params_of(received, "session/set_config_option")?;
    Ok(sets.into_iter().filter(|p| p["configId"] == config).map(|p| p["value"].clone()).collect())
}

/// The modes the agent was asked to set, in order.
fn mode_sets(received: &Path) -> Result<Vec<String>> {
    let sets = params_of(received, "session/set_mode")?;
    Ok(sets.iter().map(|p| p["modeId"].as_str().unwrap_or_default().to_string()).collect())
}

/// 7.1: `/` lists the commands, typing narrows them, down and enter take
/// one (its name and a space written in, the list gone), and enter then
/// sends `/name …` as a prompt. During a turn, escape closes the list and
/// the turn goes on, and tab, with no list, moves focus to the queue row.
pub fn command_completion(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("received.jsonl");
    let profile = offering(&received, "{}", &["compact", "context"])
        .step("expect session/prompt")
        .step(says("Hello!"))
        .step(stop("end_turn"))
        .profile(ctx, "completion")?;
    let started = |label| Run::new(ctx, label).config(&profile).agent("Fake").wait_for(READY).wait_for(LISTED);

    let out = started("slash").send("/").dump_at(6.0).run()?;
    waited(&out)?;
    ensure!(completion(&out) == Some(" query=\"\" highlighted=0"), "the list on `/`: {:?}", out.dump.agent);
    ensure!(
        out.dump.completion_rows.iter().take(2).eq(["/compact    Runs compact", "/context    Runs context"].iter()),
        "the agent's commands first: {:?}",
        out.dump.completion_rows
    );
    ensure!(out.dump.completion_rows.iter().any(|row| row.starts_with("/model")), "and Brindle's: {:?}", out.dump.completion_rows);

    let out = started("narrow").send("/co").key("down").dump_at(7.0).run()?;
    waited(&out)?;
    ensure!(completion(&out) == Some(" query=\"co\" highlighted=1"), "narrowed, down once: {:?}", out.dump.agent);
    ensure!(out.dump.completion_rows.len() == 2, "only the two commands: {:?}", out.dump.completion_rows);

    let out = started("send")
        .send("/co")
        .key("down")
        .key("enter")
        .send("now")
        .key("enter")
        .wait_for(&answered())
        .dump_at(12.0)
        .run()?;
    waited(&out)?;
    ensure!(prompts_sent(&received)? == ["/context now"], "sent: {:?}", prompts_sent(&received)?);
    ensure!(completion(&out).is_none(), "no list once sent: {:?}", out.dump.agent);

    let held = ctx.dir.join("held.jsonl");
    let profile = offering(&held, "{}", &["compact", "context"])
        .step("expect session/prompt")
        .step(format!("wait-file {}", gate(ctx, "never").display()))
        .profile(ctx, "held")?;
    let out = Run::new(ctx, "escape")
        .config(&profile)
        .agent("Fake")
        .first_prompt("hi")
        .wait_for(WORKING)
        .wait_for(LISTED)
        .send("next")
        .key("enter")
        .wait_for("queued waiting")
        .send("/co")
        .key("escape")
        .key("tab")
        .dump_at(10.0)
        .run()?;
    waited(&out)?;
    ensure!(completion(&out).is_none(), "escape closed the list: {:?}", out.dump.agent);
    // With the list closed, tab is the zone's again: the queue row's button.
    ensure!(out.dump.focus_stop.as_deref() == Some("queued:remove"), "tab: {:?}", out.dump.focus_stop);
    ensure!(out.dump.agent_field("turn") == Some("working"), "the turn goes on: {:?}", out.dump.agent);
    ensure!(recorded_count(&held, "session/cancel") == 0, "a cancel was sent");
    Ok(Outcome::Pass)
}

/// 7.1a: Brindle's option commands. `/mo` lists `/mode` and `/model`;
/// `/mode ` lists the choices, the current one highlighted; `/model so`
/// sets the model with no entry and an empty input, ` /mode plan` (a space
/// first) sets the mode, ` /compact` reaches the agent as `/compact`, and
/// `/model gpt` keeps its text with the reason. During a turn `/mode plan`
/// sets at once and queues nothing. An agent command named `model` is the
/// agent's: Brindle's is listed and reached as `/brindle:model`.
pub fn option_command(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("received.jsonl");
    let profile = offering(&received, "{}", &["compact", "review"])
        .step("expect session/set_config_option")
        .step(format!("respond {}", json!({"configOptions": config_options("sonnet", false)})))
        .step("expect session/set_mode")
        .step("respond {}")
        .step("expect session/prompt")
        .step(says("Hello!"))
        .step(stop("end_turn"))
        .profile(ctx, "commands")?;
    let started = |label| Run::new(ctx, label).config(&profile).agent("Fake").wait_for(READY).wait_for(LISTED);

    let out = started("list").send("/mo").dump_at(6.0).run()?;
    waited(&out)?;
    let mut labels: Vec<&str> = out.dump.completion_rows.iter().filter_map(|row| row.split("  ").next()).collect();
    labels.sort_unstable();
    ensure!(labels == ["/mode", "/model"], "`/mo` lists: {:?}", out.dump.completion_rows);

    let out = started("choices").send("/mode ").dump_at(6.0).run()?;
    waited(&out)?;
    ensure!(completion(&out) == Some(" command=\"mode\" query=\"\" highlighted=0"), "{:?}", out.dump.agent);
    ensure!(out.dump.completion_rows == ["Default", "Plan"], "the choices: {:?}", out.dump.completion_rows);

    let out = started("no-choice").send("/model gpt").key("enter").dump_at(7.0).run()?;
    waited(&out)?;
    ensure!(input(&out) == Some("/model gpt"), "the text stays: {:?}", out.dump.input);
    let prompt = out.dump.agent.iter().find(|line| line.starts_with("prompt ")).cloned().unwrap_or_default();
    ensure!(prompt.contains("refusal=") && prompt.contains("has no choice"), "the reason beside it: {prompt}");

    std::fs::remove_file(&received).ok();
    let out = started("set")
        .send("/model so")
        .key("enter")
        .wait_for("current=\"sonnet\"")
        .send(" /mode plan")
        .key("enter")
        .wait_for("current=\"plan\"")
        // Enter would take the highlighted command: escape closes the list.
        .send(" /compact")
        .key("escape")
        .key("enter")
        .wait_for(&answered())
        .dump_at(14.0)
        .run()?;
    waited(&out)?;
    ensure!(config_sets(&received, "model")? == [json!("sonnet")], "model sets: {:?}", config_sets(&received, "model")?);
    ensure!(mode_sets(&received)? == ["plan"], "mode sets: {:?}", mode_sets(&received)?);
    ensure!(prompts_sent(&received)? == ["/compact"], "sent: {:?}", prompts_sent(&received)?);
    ensure!(messages(&out) == ["user \"/compact\"", "agent \"Hello!\""], "messages: {:?}", out.dump.agent);
    ensure!(input(&out) == Some(""), "the input emptied: {:?}", out.dump.input);

    // During a turn a set goes at once and nothing is queued.
    let busy = ctx.dir.join("busy.jsonl");
    let profile = offering(&busy, "{}", &["compact"])
        .step("expect session/prompt")
        .step("expect session/set_mode")
        .step("respond {}")
        .step(format!("wait-file {}", gate(ctx, "never").display()))
        .profile(ctx, "busy")?;
    let out = Run::new(ctx, "turn")
        .config(&profile)
        .agent("Fake")
        .first_prompt("hi")
        .wait_for(WORKING)
        .send("/mode plan")
        .key("enter")
        .wait_for("current=\"plan\"")
        .dump_at(9.0)
        .run()?;
    waited(&out)?;
    ensure!(mode_sets(&busy)? == ["plan"], "mode sets: {:?}", mode_sets(&busy)?);
    ensure!(!out.dump.agent.iter().any(|line| line.starts_with("queued ")), "something was queued: {:?}", out.dump.agent);
    ensure!(input(&out) == Some(""), "the input emptied: {:?}", out.dump.input);

    // An agent command named like an option.
    let shadowed = ctx.dir.join("shadowed.jsonl");
    let profile = offering(&shadowed, "{}", &["compact", "model"])
        .step("expect session/prompt")
        .step(says("Hello!"))
        .step(stop("end_turn"))
        .step("expect session/set_config_option")
        .step(format!("respond {}", json!({"configOptions": config_options("sonnet", false)})))
        .step("expect session/set_mode")
        .step("respond {}")
        .profile(ctx, "shadowed")?;
    let out = Run::new(ctx, "shadowed-list")
        .config(&profile)
        .agent("Fake")
        .wait_for(READY)
        .wait_for(LISTED)
        .send("/mod")
        .dump_at(6.0)
        .run()?;
    waited(&out)?;
    let labels: Vec<&str> = out.dump.completion_rows.iter().filter_map(|row| row.split("  ").next()).collect();
    ensure!(
        labels.contains(&"/model") && labels.contains(&"/brindle:model") && labels.contains(&"/mode"),
        "the agent's /model and Brindle's long one: {:?}",
        out.dump.completion_rows
    );
    let out = Run::new(ctx, "shadowed")
        .config(&profile)
        .agent("Fake")
        .wait_for(READY)
        .wait_for(LISTED)
        .send("/model x")
        .key("enter")
        .wait_for(&answered())
        .send("/brindle:model so")
        .key("enter")
        .wait_for("current=\"sonnet\"")
        .send("/brindle:mode plan")
        .key("enter")
        .wait_for("current=\"plan\"")
        .dump_at(14.0)
        .run()?;
    waited(&out)?;
    ensure!(prompts_sent(&shadowed)? == ["/model x"], "the agent's command: {:?}", prompts_sent(&shadowed)?);
    ensure!(config_sets(&shadowed, "model")? == [json!("sonnet")], "{:?}", config_sets(&shadowed, "model")?);
    ensure!(mode_sets(&shadowed)? == ["plan"], "mode sets: {:?}", mode_sets(&shadowed)?);
    Ok(Outcome::Pass)
}

/// The options palette's steps choosing Sonnet: its rows are Mode, Model,
/// Fast; down to Model opens its choices, down to Sonnet sets it.
fn chosen(run: Run<'_>) -> Run<'_> {
    run.action("agent_options").key("down").action("picker_confirm").key("down").action("picker_confirm")
}

/// 7.2: the options in the dump; `agent_options` lists them, a select's row
/// lists its choices, and a choice sets it: the value changes only once
/// the agent answers (held behind a gate), and the options follow an
/// update the agent sends on its own.
pub fn option_set(ctx: &Ctx) -> Result<Outcome> {
    let go = gate(ctx, "answer");
    let script = |received: &Path| {
        offering(received, "{}", &["compact", "review"])
            .step("expect session/set_config_option")
            .step(format!("wait-file {}", go.display()))
            .step(format!("respond {}", json!({"configOptions": config_options("sonnet", false)})))
            .step(update(json!({"sessionUpdate": "config_option_update", "configOptions": config_options("sonnet", true)})))
    };
    let received = ctx.dir.join("held.jsonl");
    let profile = script(&received).profile(ctx, "held")?;
    std::fs::remove_file(&go).ok();
    let out = Run::new(ctx, "listed")
        .config(&profile)
        .agent("Fake")
        .wait_for(READY)
        .wait_for(LISTED)
        .action("agent_options")
        .dump_at(6.0)
        .run()?;
    waited(&out)?;
    ensure!(
        option_line(&out, "mode") == Some("category=mode kind=select current=\"default\" name=\"Mode\" choices=2"),
        "mode: {:?}",
        out.dump.agent
    );
    ensure!(
        option_line(&out, "model") == Some("category=model kind=select current=\"opus\" name=\"Model\" choices=2"),
        "model: {:?}",
        out.dump.agent
    );
    ensure!(
        option_line(&out, "fast") == Some("category=other:speed kind=toggle current=off name=\"Fast\""),
        "fast: {:?}",
        out.dump.agent
    );
    let rows: Vec<&str> = out.dump.palette.iter().skip(1).map(String::as_str).collect();
    ensure!(rows == ["Mode: Default", "Model: Opus", "Fast: off"], "the palette: {:?}", out.dump.palette);

    let out = chosen(Run::new(ctx, "held").config(&profile).agent("Fake").wait_for(READY).wait_for(LISTED))
        .dump_at(9.0)
        .run()?;
    waited(&out)?;
    ensure!(config_sets(&received, "model")? == [json!("sonnet")], "{:?}", config_sets(&received, "model")?);
    ensure!(option_line(&out, "model").is_some_and(|line| line.contains("current=\"opus\"")), "before the answer: {:?}", out.dump.agent);
    ensure!(out.dump.palette.is_empty(), "the palette closed: {:?}", out.dump.palette);

    let received = ctx.dir.join("answered.jsonl");
    let profile = script(&received).profile(ctx, "answered")?;
    let run = chosen(Run::new(ctx, "answered").config(&profile).agent("Fake").wait_for(READY).wait_for(LISTED));
    let out = from_shell(run, &touch(&go), true).wait_for("current=\"sonnet\"").wait_for("current=on").dump_at(14.0).run()?;
    waited(&out)?;
    ensure!(option_line(&out, "model").is_some_and(|line| line.contains("current=\"sonnet\"")), "{:?}", out.dump.agent);
    ensure!(option_line(&out, "fast").is_some_and(|line| line.contains("current=on")), "the agent's own update: {:?}", out.dump.agent);
    Ok(Outcome::Pass)
}

/// 7.3: an agent with modes alone shows one option, the mode;
/// `agent_next_mode` steps it and wraps, each change a `Mode:` notice.
pub fn option_modes(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("received.jsonl");
    let answer = json!({"sessionId": "s1", "modes": modes_in("default")});
    let profile = Script::initialized(&received, "{}")
        .step("expect session/new")
        .step(format!("respond {answer}"))
        .step("expect session/set_mode")
        .step("respond {}")
        .step("expect session/set_mode")
        .step("respond {}")
        .profile(ctx, "modes")?;
    let out = Run::new(ctx, "run")
        .config(&profile)
        .agent("Fake")
        .wait_for(READY)
        .wait_for("current=\"default\"")
        .action("agent_next_mode")
        .wait_for("Mode: Plan")
        .action("agent_next_mode")
        .wait_for("Mode: Default")
        .dump_at(10.0)
        .run()?;
    waited(&out)?;
    let options: Vec<&String> = out.dump.agent.iter().filter(|line| line.starts_with("option ")).collect();
    ensure!(options.len() == 1, "one option: {options:?}");
    ensure!(option_line(&out, "mode").is_some_and(|line| line.contains("current=\"default\"")), "{:?}", out.dump.agent);
    ensure!(mode_sets(&received)? == ["plan", "default"], "mode sets: {:?}", mode_sets(&received)?);
    ensure!(has_notice(&out, "Mode: Plan") && has_notice(&out, "Mode: Default"), "notices: {:?}", out.dump.agent);
    Ok(Outcome::Pass)
}

/// 7.4: a second set while one is unanswered is refused by the server,
/// its reason on the window's config line; the agent's own refusal is a
/// notice in the conversation, the value unchanged.
pub fn option_refused(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("held.jsonl");
    let profile = offering(&received, "{}", &["compact", "review"])
        .step("expect session/set_config_option")
        .step(format!("wait-file {}", gate(ctx, "never").display()))
        .profile(ctx, "held")?;
    let out = Run::new(ctx, "second")
        .config(&profile)
        .agent("Fake")
        .wait_for(READY)
        .wait_for(LISTED)
        .send("/model sonnet")
        .key("enter")
        .send("/mode plan")
        .key("enter")
        .dump_after_last_step(NOTICE_SHOWN)
        .run()?;
    waited(&out)?;
    let line = out.dump.config.as_deref().unwrap_or_default();
    ensure!(
        line.contains("Mode wasn't changed") && line.contains("hasn't been answered yet"),
        "config line {:?}",
        out.dump.config
    );
    ensure!(mode_sets(&received)?.is_empty(), "the second set reached the agent");

    let received = ctx.dir.join("refused.jsonl");
    let profile = offering(&received, "{}", &["compact", "review"])
        .step("expect session/set_config_option")
        .step(r#"respond-error -32602 "model unavailable""#)
        .profile(ctx, "refused")?;
    let out = Run::new(ctx, "agent")
        .config(&profile)
        .agent("Fake")
        .wait_for(READY)
        .wait_for(LISTED)
        .send("/model sonnet")
        .key("enter")
        .wait_for("model unavailable")
        .dump_at(8.0)
        .run()?;
    waited(&out)?;
    ensure!(has_notice(&out, "didn't change Model") && has_notice(&out, "model unavailable"), "{:?}", out.dump.agent);
    ensure!(option_line(&out, "model").is_some_and(|line| line.contains("current=\"opus\"")), "{:?}", out.dump.agent);
    Ok(Outcome::Pass)
}

/// 7.5: quit and relaunch: the restored zone shows the same commands and
/// options, and the agent, started once, sends them once.
pub fn option_restore(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("received.jsonl");
    let profile = offering(&received, "{}", &["compact", "review"]).profile(ctx, "restore")?;
    let out = Run::new(ctx, "first")
        .config(&profile)
        .agent("Fake")
        .keep_detached()
        .wait_for(READY)
        .wait_for(LISTED)
        .dump_at(5.0)
        .run()?;
    waited(&out)?;
    let lines = |out: &Output| -> Vec<String> {
        out.dump.agent.iter().filter(|l| l.starts_with("commands ") || l.starts_with("option ")).cloned().collect()
    };
    let before = lines(&out);
    ensure!(before.len() == 4, "the lists: {before:?}");

    let out = Run::new(ctx, "relaunch").config(&profile).restore().wait_for(LISTED).dump_at(5.0).run()?;
    waited(&out)?;
    ensure!(lines(&out) == before, "restored: {:?}, before {before:?}", lines(&out));
    ensure!(recorded_count(&received, "initialize") == 1, "the agent was started again");
    Ok(Outcome::Pass)
}

/// 7.5a (decision 12): the agent exits and its zone shows no options or
/// commands and no list on `/`; Continue brings them back from the
/// continued agent, with a `Mode:` notice since it starts in another mode.
pub fn option_continue(ctx: &Ctx) -> Result<Outcome> {
    let go = gate(ctx, "exit");
    let first = |received: &Path| {
        offering(received, RESUME, &["compact", "review"])
            .step("expect session/prompt")
            .step(says("Hello!"))
            .step(stop("end_turn"))
            .step(format!("wait-file {}", go.display()))
            .step("exit 1")
    };
    let next = |received: &Path| {
        let answer = json!({"modes": modes_in("plan"), "configOptions": config_options("opus", false)});
        Script::initialized(received, RESUME)
            .step("expect session/resume")
            .step(format!("respond {answer}"))
            .step(commands_step(&["compact", "review"]))
    };

    let (one, two) = records(ctx);
    let (profile, swap) = continued(ctx, first(&one), next(&two), &touch(&go))?;
    let run = Run::new(ctx, "exited").config(&profile).agent("Fake").first_prompt("hi").wait_for(&answered());
    let out = from_shell(run, &swap, true).wait_for(EXITED).send("/").dump_at(10.0).run()?;
    waited(&out)?;
    ensure!(
        !out.dump.agent.iter().any(|l| l.starts_with("option ") || l.starts_with("commands ") || l.starts_with("completion")),
        "lists while exited: {:?}",
        out.dump.agent
    );

    let (one, two) = records(ctx);
    let (profile, swap) = continued(ctx, first(&one), next(&two), &touch(&go))?;
    let run = Run::new(ctx, "continued").config(&profile).agent("Fake").first_prompt("hi").wait_for(&answered());
    let out = from_shell(run, &swap, true)
        .wait_for(EXITED)
        .action("agent_continue")
        .wait_for(READY)
        .wait_for(LISTED)
        .wait_for("current=\"plan\"")
        .dump_at(13.0)
        .run()?;
    waited(&out)?;
    ensure!(option_line(&out, "model").is_some(), "the options back: {:?}", out.dump.agent);
    ensure!(has_notice(&out, "Mode: Plan"), "no mode notice: {:?}", out.dump.agent);
    ensure!(recorded_count(&two, "session/resume") == 1, "not continued by resume");
    Ok(Outcome::Pass)
}
