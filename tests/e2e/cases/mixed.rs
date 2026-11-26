//! Tabs of both kinds (OpenSpec change `zones-mixed-tabs`): the split
//! commands of each kind from a terminal zone and from an agent zone, the
//! palette a split asks with, the directory a split starts in, a mixed tab
//! restored and swept, each zone in its own theme, and a terminal login's
//! zone that splits with nothing. The agent zones run the fake ACP agent
//! (`Case::needs_fake_agent`), from scripts written into the case directory.

use std::path::{Path, PathBuf};

use super::agent::{READY, Script, TERMINAL_CHOICE, TERMINAL_LOGIN, answered, prompts_sent, says, waited, with_terminal_login};
use crate::brindle::{Output, Run, theme};
use crate::{Ctx, Outcome, Result, ensure};

/// An agent that records what it is sent in `received`, is ready at once,
/// answers its first prompt with `Hello!` ([`answered`]) and then idles
/// until its session ends.
fn hello(received: &Path) -> Script {
    Script::initialized(received, "{}")
        .step("expect session/new")
        .step(r#"respond {"sessionId":"s1"}"#)
        .step("expect session/prompt")
        .step(says("Hello!"))
        .step(r#"respond {"stopReason":"end_turn"}"#)
}

/// An agent profile `name` running [`hello`] from `LABEL.script`, recording
/// in `LABEL.jsonl`, with `extra` TOML lines in its table: the profile's
/// TOML and where its agents record.
fn fake(ctx: &Ctx, name: &str, label: &str, extra: &str) -> Result<(String, PathBuf)> {
    let received = ctx.dir.join(format!("{label}.jsonl"));
    let script = hello(&received).write(ctx, label)?;
    let profile = format!(
        "\n[[profiles]]\nname = \"{name}\"\ntype = \"agent\"\ncommand = '{}'\nargs = ['{}']\n{extra}",
        ctx.fake_agent.display(),
        script.display()
    );
    Ok((profile, received))
}

/// The `params` of each `session/new` the agents recording in `received`
/// were sent: one per agent zone started. None when no agent started.
fn sessions_started(received: &Path) -> Result<Vec<serde_json::Value>> {
    let Ok(text) = std::fs::read_to_string(received) else { return Ok(Vec::new()) };
    let mut started = Vec::new();
    for line in text.lines() {
        let value: serde_json::Value = serde_json::from_str(line).map_err(|e| format!("{line}: {e}"))?;
        if value["method"] == "session/new" {
            started.push(value["params"].clone());
        }
    }
    Ok(started)
}

/// The directories the agents recording in `received` started their
/// sessions in (`session/new`'s `cwd`), with symbolic links resolved.
fn session_dirs(received: &Path) -> Result<Vec<PathBuf>> {
    sessions_started(received)?
        .iter()
        .map(|params| {
            let cwd = params["cwd"].as_str().ok_or_else(|| format!("a session/new without a cwd: {params}"))?;
            std::fs::canonicalize(cwd).map_err(|e| format!("{cwd}: {e}"))
        })
        .collect()
}

/// A directory `name` in the case directory, created, with symbolic links
/// resolved (as a shell's `pwd -P` prints it).
fn dir(ctx: &Ctx, name: &str) -> Result<PathBuf> {
    let dir = ctx.dir.join(name);
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::fs::canonicalize(&dir).map_err(|e| format!("{}: {e}", dir.display()))
}

fn expect_zones(out: &Output, header: &str, lines: &[&str], what: &str) -> Result {
    ensure!(out.dump.zones.as_deref() == Some(header), "{what}: zones {:?}, want {header}", out.dump.zones);
    ensure!(out.dump.zone_list == lines, "{what}: zone lines {:#?}", out.dump.zone_list);
    Ok(())
}

/// The spec's "Split to a terminal" and "Split to an agent", by their keys:
/// each of the four from a shell's zone and from an agent's, the terminal
/// splits running the default profile (`Shell`) and the agent splits the
/// only agent profile (`Fake`), or the agent zone's own, each new zone
/// focused; and the old `split_zone_right`, from an agent zone, splits a
/// terminal.
pub fn split_kinds(ctx: &Ctx) -> Result<Outcome> {
    let (profile, received) = fake(ctx, "Fake", "from-shell", "")?;
    let out = Run::new(ctx, "from-shell")
        .config(&profile)
        .key("ctrl-shift-e")
        .action("focus_zone_left")
        .key("ctrl-shift-o")
        .key("ctrl-shift-alt-e")
        .wait_for(READY)
        .action("focus_zone_up")
        .key("ctrl-shift-alt-o")
        .wait_for(READY)
        .dump_at(12.0)
        .run()?;
    waited(&out)?;
    expect_zones(
        &out,
        "count=5 active=5",
        &[
            "id=1 rect=0,0,0.5,0.25 kind=terminal profile=\"Shell\"",
            "id=5 rect=0,0.25,0.5,0.25 kind=agent profile=\"Fake\" active",
            "id=3 rect=0,0.5,0.25,0.5 kind=terminal profile=\"Shell\"",
            "id=4 rect=0.25,0.5,0.25,0.5 kind=agent profile=\"Fake\"",
            "id=2 rect=0.5,0,0.5,1 kind=terminal profile=\"Shell\"",
        ],
        "from a shell",
    )?;
    ensure!(sessions_started(&received)?.len() == 2, "from a shell: {:?} agents started", sessions_started(&received)?);

    let (profile, received) = fake(ctx, "Fake", "from-agent", "")?;
    let out = Run::new(ctx, "from-agent")
        .config(&profile)
        .agent("Fake")
        .wait_for(READY)
        .key("ctrl-shift-e")
        .action("focus_zone_left")
        .key("ctrl-shift-o")
        .action("focus_zone_up")
        .key("ctrl-shift-alt-e")
        .wait_for(READY)
        .action("focus_zone_left")
        .key("ctrl-shift-alt-o")
        .wait_for(READY)
        .dump_at(15.0)
        .run()?;
    waited(&out)?;
    expect_zones(
        &out,
        "count=5 active=5",
        &[
            "id=1 rect=0,0,0.25,0.25 kind=agent profile=\"Fake\"",
            "id=5 rect=0,0.25,0.25,0.25 kind=agent profile=\"Fake\" active",
            "id=4 rect=0.25,0,0.25,0.5 kind=agent profile=\"Fake\"",
            "id=3 rect=0,0.5,0.5,0.5 kind=terminal profile=\"Shell\"",
            "id=2 rect=0.5,0,0.5,1 kind=terminal profile=\"Shell\"",
        ],
        "from an agent",
    )?;
    ensure!(sessions_started(&received)?.len() == 3, "from an agent: {:?} agents started", sessions_started(&received)?);

    let (profile, received) = fake(ctx, "Fake", "old-name", "")?;
    let out = Run::new(ctx, "old-name")
        .config(&profile)
        .agent("Fake")
        .wait_for(READY)
        .action("split_zone_right")
        .send("echo old-$((1+1))\r")
        .wait_for("old-2")
        .dump_at(7.0)
        .run()?;
    waited(&out)?;
    expect_zones(
        &out,
        "count=2 active=2",
        &["id=1 rect=0,0,0.5,1 kind=agent profile=\"Fake\"", "id=2 rect=0.5,0,0.5,1 kind=terminal profile=\"Shell\" active"],
        "split_zone_right",
    )?;
    ensure!(sessions_started(&received)?.len() == 1, "split_zone_right started an agent");
    Ok(Outcome::Pass)
}

/// The rows of the open palette, after its header, as `(label, detail)`.
fn palette_rows(out: &Output) -> Vec<(&str, &str)> {
    out.dump.palette.iter().skip(1).map(|row| row.split_once("  ").unwrap_or((row.as_str(), ""))).collect()
}

/// The spec's "Several profiles of the kind" and "Split with a chosen
/// profile": with two agent profiles and neither the default, Split Agent
/// Right from a shell asks with a palette of both, in the config's order;
/// the list actions choose the second, which splits the shell's zone; and
/// escape splits nothing. Split Right With Profile… lists every profile,
/// the default first, and a second terminal profile chosen there splits
/// the zone with it. With no agent profile, Split Agent Right splits
/// nothing and says so on the config line.
pub fn split_ask(ctx: &Ctx) -> Result<Outcome> {
    let (first, _) = fake(ctx, "Fake", "fake", "")?;
    let (second, received) = fake(ctx, "Fake2", "fake2", "")?;
    let work = "\n[[profiles]]\nname = \"Work\"\ncommand = \"bash\"\nargs = [\"--norc\", \"--noprofile\"]\n";
    let config = format!("{first}{second}{work}");

    let out = Run::new(ctx, "ask").config(&config).key("ctrl-shift-alt-e").dump_at(3.5).run()?;
    ensure!(out.dump.palette.first().is_some_and(|l| l == "list query=\"\" highlighted=0"), "the palette: {:?}", out.dump.palette);
    let rows = palette_rows(&out);
    ensure!(
        rows.len() == 2 && rows[0].0 == "Fake" && rows[1].0 == "Fake2" && rows.iter().all(|(_, d)| d.starts_with("agent · ")),
        "the agent profiles' rows: {rows:?}"
    );
    ensure!(out.dump.focus.as_deref() == Some("palette"), "focus: {:?}", out.dump.focus);
    ensure!(out.dump.zones.is_none(), "split before a choice: {:?}", out.dump.zones);

    let out = Run::new(ctx, "choose")
        .config(&config)
        .key("ctrl-shift-alt-e")
        .action("list_down")
        .action("list_confirm")
        .wait_for(READY)
        .dump_at(7.0)
        .run()?;
    waited(&out)?;
    ensure!(out.dump.palette.is_empty(), "the palette is still open: {:?}", out.dump.palette);
    expect_zones(
        &out,
        "count=2 active=2",
        &["id=1 rect=0,0,0.5,1 kind=terminal profile=\"Shell\"", "id=2 rect=0.5,0,0.5,1 kind=agent profile=\"Fake2\" active"],
        "Fake2 chosen",
    )?;
    ensure!(sessions_started(&received)?.len() == 1, "Fake2's agent didn't start");

    let out = Run::new(ctx, "escape").config(&config).key("ctrl-shift-alt-e").key("escape").dump_at(4.5).run()?;
    ensure!(out.dump.palette.is_empty(), "the palette is still open: {:?}", out.dump.palette);
    ensure!(out.dump.zones.is_none(), "escape split the zone: {:?}", out.dump.zones);
    ensure!(out.dump.focus.as_deref() == Some("terminal"), "focus after escape: {:?}", out.dump.focus);

    let out = Run::new(ctx, "with-profile").config(&config).action("split_right_with_profile").dump_at(3.5).run()?;
    let labels: Vec<&str> = palette_rows(&out).iter().map(|(label, _)| *label).collect();
    ensure!(labels == ["Shell", "Fake", "Fake2", "Work"], "Split Right With Profile…'s rows: {:?}", out.dump.palette);

    let out = Run::new(ctx, "with-work")
        .config(&config)
        .action("split_right_with_profile")
        .send("Work")
        .action("list_confirm")
        .send("echo work-$((6*7))\r")
        .wait_for("work-42")
        .dump_at(8.0)
        .run()?;
    waited(&out)?;
    expect_zones(
        &out,
        "count=2 active=2",
        &["id=1 rect=0,0,0.5,1 kind=terminal profile=\"Shell\"", "id=2 rect=0.5,0,0.5,1 kind=terminal profile=\"Work\" active"],
        "Work chosen",
    )?;

    let out = Run::new(ctx, "refused").key("ctrl-shift-alt-e").dump_at(3.5).run()?;
    ensure!(out.dump.zones.is_none(), "split with no agent profile: {:?}", out.dump.zones);
    ensure!(out.dump.palette.is_empty(), "a palette with no agent profile: {:?}", out.dump.palette);
    ensure!(
        out.dump.config.as_deref().is_some_and(|line| line.contains("no agent profile to split with")),
        "the config line: {:?}",
        out.dump.config
    );
    Ok(Outcome::Pass)
}

/// The spec's "Split starts in the split zone's directory": an agent split
/// of a shell that has changed directory starts its session there, though
/// its profile sets `cwd`; and a terminal split of an agent zone starts in
/// the directory the agent's session started in, its profile's.
pub fn split_directory(ctx: &Ctx) -> Result<Outcome> {
    let (shell_dir, profile_dir) = (dir(ctx, "shell-dir")?, dir(ctx, "profile-dir")?);
    let extra = format!("cwd = '{}'\n", profile_dir.display());

    let (profile, received) = fake(ctx, "Fake", "beside-shell", &extra)?;
    let out = Run::new(ctx, "agent-split")
        .config(&profile)
        .send(&format!("cd '{}'\r", shell_dir.display()))
        .key("ctrl-shift-alt-e")
        .wait_for(READY)
        .dump_at(6.0)
        .run()?;
    waited(&out)?;
    ensure!(out.dump.zones.as_deref() == Some("count=2 active=2"), "zones: {:?}", out.dump.zones);
    let dirs = session_dirs(&received)?;
    ensure!(dirs == [shell_dir.clone()], "the agent split's session started in {dirs:?}, not the shell's {shell_dir:?}");

    let (profile, received) = fake(ctx, "Fake", "in-agent-dir", &extra)?;
    let out = Run::new(ctx, "terminal-split")
        .config(&profile)
        .agent("Fake")
        .wait_for(READY)
        .key("ctrl-shift-e")
        .send(&format!("[ \"$(pwd -P)\" = '{}' ] && echo same-$((1+1))\r", profile_dir.display()))
        .wait_for("same-2")
        .dump_at(7.0)
        .run()?;
    waited(&out)?;
    expect_zones(
        &out,
        "count=2 active=2",
        &["id=1 rect=0,0,0.5,1 kind=agent profile=\"Fake\"", "id=2 rect=0.5,0,0.5,1 kind=terminal profile=\"Shell\" active"],
        "a terminal split of an agent",
    )?;
    let dirs = session_dirs(&received)?;
    ensure!(dirs == [profile_dir.clone()], "the agent's session started in {dirs:?}, not its profile's {profile_dir:?}");
    Ok(Outcome::Pass)
}

/// The spec's "Mixed tab restored" and "Mixed layout swept": a tab of a
/// shell and an agent zone, quit and relaunched, comes back whole, each zone
/// attached to its own session (the shell's screen kept, the conversation
/// shown and nothing sent again); after a `--no-restore` launch replaced the
/// state file, the next launch sweeps the same sessions back as one tab.
pub fn mixed_restore(ctx: &Ctx) -> Result<Outcome> {
    let shell_dir = dir(ctx, "shell-dir")?;
    let (profile, received) = fake(ctx, "Fake", "agent", "")?;
    let out = Run::new(ctx, "record")
        .config(&profile)
        .restore()
        .keep_detached()
        .send(&format!("cd '{}'; echo kept-$((2+3))\r", shell_dir.display()))
        .wait_for("kept-5")
        .key("ctrl-shift-alt-e")
        .wait_for(READY)
        .send("hi")
        .key("enter")
        .wait_for(&answered())
        .dump_at(9.0)
        .run()?;
    waited(&out)?;
    ensure!(out.dump.zones.as_deref() == Some("count=2 active=2"), "recorded zones: {:?}", out.dump.zones);
    let state = std::fs::read_to_string(crate::brindle::state_dir(ctx).join("session.toml")).map_err(|e| e.to_string())?;
    ctx.save("recorded.toml", &state);

    let both = |active: usize| {
        let line = |id: usize, rest: &str| {
            let mark = if id == active { " active" } else { "" };
            format!("id={id} {rest}{mark}")
        };
        [line(1, "rect=0,0,0.5,1 kind=terminal profile=\"Shell\""), line(2, "rect=0.5,0,0.5,1 kind=agent profile=\"Fake\"")]
    };
    let out = Run::new(ctx, "restored")
        .config(&profile)
        .restore()
        .keep_detached()
        .wait_for(&answered())
        .action("focus_zone_left")
        .wait_for("kept-5")
        .dump_at(6.0)
        .run()?;
    waited(&out)?;
    ensure!(out.dump.tabs.len() == 1, "restored tabs: {:?}", out.dump.tabs);
    let want = both(1);
    expect_zones(&out, "count=2 active=1", &[&want[0], &want[1]], "restored")?;
    ensure!(sessions_started(&received)?.len() == 1, "the restore started another agent");
    ensure!(prompts_sent(&received)? == ["hi"], "restored: sent {:?}", prompts_sent(&received)?);

    let other = Run::new(ctx, "no-restore").config(&profile).keep_detached().dump_at(3.0).run()?;
    ensure!(other.dump.tabs.len() == 1, "the no-restore run's tabs: {:?}", other.dump.tabs);
    let out = Run::new(ctx, "swept")
        .config(&profile)
        .restore()
        .action("next_tab")
        .wait_for("kept-5")
        .action("focus_zone_right")
        .wait_for(&answered())
        .dump_at(7.0)
        .run()?;
    waited(&out)?;
    ensure!(out.dump.tabs.len() == 2 && out.dump.tabs[1].1, "swept tabs: {:?}", out.dump.tabs);
    let want = both(2);
    expect_zones(&out, "count=2 active=2", &[&want[0], &want[1]], "swept")?;
    ensure!(sessions_started(&received)?.len() == 1, "the sweep started another agent");
    ensure!(prompts_sent(&received)? == ["hi"], "swept: sent {:?}", prompts_sent(&received)?);
    Ok(Outcome::Pass)
}

/// The spec's "Each zone in its own theme" and "A tab follows its active
/// zone": a shell zone in the global `e2e` theme beside an agent zone whose
/// profile's theme is `e2e-agent`, each card and header in its own zone's
/// background, and the tab's face in the active zone's, before and after
/// `focus_zone_left`.
pub fn mixed_themes(ctx: &Ctx) -> Result<Outcome> {
    use super::agent::{parse_zone_rect, window_scale, zone_inner};
    use super::tabs::{STRIP_HEIGHT, Strip};
    use super::zones::{ZONE_ACCENT, expect_dominant};
    let (profile, _) = fake(ctx, "Fake", "themed", "theme = \"e2e-agent\"\n")?;
    let config = format!(
        "{profile}\n[themes.e2e-agent]\nforeground = \"{}\"\nbackground = \"{}\"\naccent = \"{}\"\n",
        theme::ALT_FOREGROUND.hex(),
        theme::ALT_BACKGROUND.hex(),
        ZONE_ACCENT.hex()
    );
    let split = |label| Run::new(ctx, label).config(&config).key("ctrl-shift-alt-e").wait_for(READY);
    let runs = [
        ("agent active", split("agent-active").capture_at(5.0).dump_at(5.5).run()?, theme::ALT_BACKGROUND, "active=2"),
        (
            "shell active",
            split("shell-active").action("focus_zone_left").capture_at(6.0).dump_at(6.5).run()?,
            theme::BACKGROUND,
            "active=1",
        ),
    ];
    for (what, out, face, active) in &runs {
        waited(out)?;
        ensure!(out.dump.zones.as_deref() == Some(&*format!("count=2 {active}")), "{what}: zones {:?}", out.dump.zones);
        let image = out.image.as_ref().ok_or("no capture")?;
        // The agent zone, on the right, has the rightmost card.
        let scale = window_scale(image, theme::ALT_CARD_BORDER)?;
        for (line, background) in out.dump.zone_list.iter().zip([theme::BACKGROUND, theme::ALT_BACKGROUND]) {
            let rect = parse_zone_rect(line).ok_or_else(|| format!("no rect in the zone line {line:?}"))?;
            expect_dominant(image, &format!("{what}: the card of {line:?}"), zone_inner(image, scale, rect), background)?;
            // Between the header's title and its buttons.
            let width = image.width as f32 / scale;
            let [x, y, w, _] = rect;
            let header = Strip::window(image, scale).rect(
                x * width + super::CARD_INSET + 60.0,
                STRIP_HEIGHT + y * (image.height as f32 / scale - STRIP_HEIGHT) + super::CARD_INSET + 4.0,
                w * width - 2.0 * super::CARD_INSET - 160.0,
                20.0,
            );
            expect_dominant(image, &format!("{what}: the header of {line:?}"), header, background)?;
        }
        expect_dominant(image, &format!("{what}: the tab's face"), Strip::window(image, scale).face(0), *face)?;
    }
    Ok(Outcome::Pass)
}

/// The spec's "Login zone can't be split": in a terminal login's zone,
/// which has focus while its program runs, the four split keys and Split
/// Right With Profile… split nothing and open no palette.
pub fn login_no_split(ctx: &Ctx) -> Result<Outcome> {
    let received = ctx.dir.join("received.jsonl");
    let env = ctx.dir.join("login.env");
    let never = ctx.dir.join("never");
    let agent = Script::login_at_start(&received, TERMINAL_LOGIN);
    let profile = with_terminal_login(agent, &env, &[format!("wait-file {}", never.display())]).profile(ctx, "login")?;
    let out = Run::new(ctx, "run")
        .config(&profile)
        .agent("Fake")
        .wait_for(TERMINAL_CHOICE)
        .key("tab")
        .key("enter")
        .key("ctrl-shift-e")
        .key("ctrl-shift-o")
        .key("ctrl-shift-alt-e")
        .key("ctrl-shift-alt-o")
        .action("split_right_with_profile")
        .dump_at(10.0)
        .run()?;
    waited(&out)?;
    ensure!(env.exists(), "the login program didn't run");
    ensure!(out.dump.zones.as_deref() == Some("count=2 active=2"), "zones: {:?} {:?}", out.dump.zones, out.dump.zone_list);
    ensure!(out.dump.zone_list[1].contains(" kind=terminal "), "the login zone: {:?}", out.dump.zone_list);
    ensure!(out.dump.palette.is_empty(), "a palette opened: {:?}", out.dump.palette);
    ensure!(out.dump.focus.as_deref() == Some("terminal"), "focus: {:?}", out.dump.focus);
    Ok(Outcome::Pass)
}

/// An agent split of a shell, by its key, then a turn in it: the agent zone
/// beside the shell, running the only agent profile, answers in its own
/// zone. Windows runs it too (`windows-agent-split`), through ConPTY.
pub fn agent_beside_shell(ctx: &Ctx) -> Result<Outcome> {
    let (profile, received) = fake(ctx, "Fake", "beside", "")?;
    let out = Run::new(ctx, "run")
        .config(&profile)
        .prompt()
        .key("ctrl-shift-alt-e")
        .wait_for(READY)
        .send("hi")
        .key("enter")
        .wait_for(&answered())
        .dump_at(7.0)
        .run()?;
    waited(&out)?;
    expect_zones(
        &out,
        "count=2 active=2",
        &["id=1 rect=0,0,0.5,1 kind=terminal profile=\"Shell\"", "id=2 rect=0.5,0,0.5,1 kind=agent profile=\"Fake\" active"],
        "an agent beside a shell",
    )?;
    ensure!(out.dump.agent_entries() == ["user \"hi\"", "agent \"Hello!\""], "entries: {:?}", out.dump.agent);
    ensure!(prompts_sent(&received)? == ["hi"], "sent: {:?}", prompts_sent(&received)?);
    Ok(Outcome::Pass)
}
