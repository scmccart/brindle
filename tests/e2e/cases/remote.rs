//! Remote Linux profiles exercised against disposable Docker containers.
//! The fixture deliberately contains no Brindle helper: each case's first
//! connection must deploy the sibling `brindle-remote` binary before it can
//! show a shell.
//!
//! Remote tabs run in the terminal server of the container's user, reached
//! through `brindle-remote relay` (root's over SSH, with its server
//! directory in `~/.cache/brindle/run`, which outlives the SSH login). The
//! persistence cases read that server with the deployed helper's own `list`
//! (`docker exec`, which needs no network), and Brindle's `ssh` processes
//! from `/proc`:
//!
//! - `remote-ssh-quit`: quitting leaves an SSH tab's program detached, and
//!   the next launch restores the tab with the same process and scrollback;
//! - `remote-ssh-split`: three zones of an SSH tab share one `ssh` and one
//!   relay, and closing the tab ends their programs and then the `ssh`;
//! - `remote-ssh-network`: with the container cut off its network (`docker
//!   network disconnect`), Brindle notices and says so, and once the network
//!   is back it has the same session again, with what was typed meanwhile;
//! - `remote-ssh-connect-at-launch`: a profile that connects at launch
//!   attaches a session an earlier run left detached on its host;
//! - `remote-ssh-restored-closed`: a restored SSH tab whose host now
//!   refuses, closed while it connects, takes the launch's wait for that
//!   host with it, so Brindle stops dialling it;
//! - `remote-offline`: with its container stopped, and an SSH host that
//!   refuses, "Attach Session…" lists each eager profile's endpoint offline.
//!
//! Checks that must happen while Brindle runs are [`Run::call_at`] calls that
//! poll; one that must hold a later step back writes a marker to a tab's
//! terminal, which a [`Run::wait_for`] step waits for. Every container, and
//! the network a case makes, is removed when the case ends, passed or not.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::io::Write as _;
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::brindle::{self, Dump, Run};
use crate::{Ctx, Outcome, Result, Scale, ensure};

const CONTAINER_PROFILE: &str = "Remote container";
const SSH_PROFILE: &str = "Remote SSH";
const OFFLINE_CONTAINER_PROFILE: &str = "Stopped container";
const OFFLINE_SSH_PROFILE: &str = "Refused SSH";
const REMOTE_CWD: &str = "/tmp/brindle-e2e-osc7";
/// The SSH profiles' prompt (`PS1`), without its trailing space.
const SSH_PROMPT: &str = "REMOTE_SSH_READY>";
/// The SSH profiles' `ssh_args`: no config, keys or known hosts of the
/// user's, and no prompts.
const SSH_ARGS: &str = "[\"-F\", \"/dev/null\", \"-o\", \"BatchMode=yes\", \
     \"-o\", \"IdentitiesOnly=yes\", \"-o\", \"StrictHostKeyChecking=no\", \
     \"-o\", \"UserKnownHostsFile=/dev/null\", \"-o\", \"GlobalKnownHostsFile=/dev/null\", \
     \"-o\", \"LogLevel=ERROR\"]";
/// Runs root's cached helper in the container with the script's arguments:
/// `~/.cache/brindle/<version>-<digest>/brindle-remote`, against root's
/// server (`~/.cache/brindle/run`, as `docker exec` and SSH both resolve it).
const ROOT_HELPER: &str = concat!(
    "h=$(find /root/.cache/brindle -mindepth 2 -maxdepth 2 -type f -name brindle-remote ",
    "2>/dev/null | head -n 1); ",
    r#"test -n "$h" || { echo "no cached helper under /root/.cache/brindle" >&2; exit 3; }; "#,
    r#"exec "$h" "$@""#,
);
/// Prints `pid<TAB>argv` for each of the container's processes, the
/// arguments joined by U+001F.
const PROCESSES: &str = concat!(
    r#"for p in /proc/[0-9]*; do printf '%s\t' "${p#/proc/}"; "#,
    r#"tr '\0' '\037' < "$p/cmdline"; echo; done 2>/dev/null; exit 0"#,
);
/// Prints the servers' logs in the container (root's, and the profile
/// user's), each under its path.
const SERVER_LOGS: &str = concat!(
    r#"for f in /root/.cache/brindle/run/*.log /home/*/.cache/brindle/run/*.log; do "#,
    r#"test -f "$f" && printf '==> %s <==\n' "$f" && cat "$f"; done; exit 0"#,
);

static CONTAINER_SEQUENCE: AtomicU64 = AtomicU64::new(0);
static FIXTURE_IMAGE: OnceLock<Result<String>> = OnceLock::new();

/// A vanilla container profile covers helper injection, PTY input/output,
/// OSC 7 directory inheritance in a split zone, OSC 7501, and OSC 9/777
/// notifications, in one disposable container.
pub fn container_exec(ctx: &Ctx) -> Result<Outcome> {
    if let Some(outcome) = docker_outcome()? {
        return Ok(outcome);
    }
    let Some(agent) = remote_agent(ctx)? else {
        return Ok(Outcome::Skip(
            "Linux remote helper was not built; run `cargo build -p brindle-remote-agent`".into(),
        ));
    };
    let image = fixture_image(ctx)?;
    let container = Container::start(ctx, &image, &[])?;
    let planted = format!("/tmp/brindle-remote-{}", env!("CARGO_PKG_VERSION"));
    let marker = "/tmp/brindle-e2e-planted-helper-ran";
    container.exec(&[
        "sh",
        "-lc",
        &format!(
            "printf '#!/bin/sh\\ntouch {marker}\\necho {}\\n' > {planted}; chmod 755 {planted}",
            env!("CARGO_PKG_VERSION")
        ),
    ])?;

    let mut config = format!(
        "\n[[profiles]]\n\
         name = {profile:?}\n\
         command = \"/bin/bash\"\n\
         args = [\"--norc\", \"--noprofile\", \"-i\"]\n\
         env = {{ SHELL = \"/bin/bash\", PS1 = \"REMOTE_CONTAINER_READY> \", BRINDLE_E2E_MARKER = \"container-exec\" }}\n\
         remote = {{ type = \"container\", engine = \"docker\", container = {container_name:?}, user = \"remote\" }}\n",
        profile = CONTAINER_PROFILE,
        container_name = container.name,
    );
    config.push_str("\n[notifications]\nwhen = \"always\"\n");
    // The marker is assembled by printf so --wait-for cannot match the
    // shell's echo of the command before OSC 7 has actually been consumed.
    let set_cwd = format!(
        r#"mkdir -p {REMOTE_CWD}; cd {REMOTE_CWD}; printf '\033]7;file://remote%s\033\\' "$PWD"; printf 'REMOTE_%s' CWD_SET; echo\r"#
    );
    let exercise = concat!(
        "printf 'REMOTE_CONTAINER_IO:%s' \"$BRINDLE_E2E_MARKER\"; echo; ",
        "printf 'REMOTE_CONTAINER_CWD:%s' \"$PWD\"; echo; ",
        "printf '\\033]7501;state=working:id=remote:progress=73:app=remote-build\\033\\\\'; ",
        "printf '\\033]9;remote osc9\\033\\\\'; ",
        "printf '\\033]777;notify;Remote build;done\\033\\\\'; ",
        "printf 'REMOTE_%s' CONTAINER_STATUS_READY; echo; sleep 30\\r"
    )
    .to_string();
    let label = "container-pty";
    let run = Run::new(ctx, label)
        .profile(CONTAINER_PROFILE)
        .default_profile(CONTAINER_PROFILE)
        .config(&config);
    let dump = agent
        .give(run)
        .env("BRINDLE_NOTIFY_LOG", &notify_log(ctx, label))
        .wait_for("REMOTE_CONTAINER_READY>")
        .send(&set_cwd)
        .wait_for("REMOTE_CWD_SET")
        .action("split_zone_right")
        .wait_for("REMOTE_CONTAINER_READY>")
        .send(&exercise)
        .wait_for("REMOTE_CONTAINER_STATUS_READY")
        .dump_at(16.0)
        .run()?
        .dump;
    ensure!(
        dump.zones.as_deref() == Some("count=2 active=2"),
        "remote split zones: {:?}",
        dump.zones
    );
    ensure!(
        dump.screen
            .iter()
            .any(|line| line == "REMOTE_CONTAINER_IO:container-exec"),
        "remote PTY output missing from screen: {:?}",
        dump.screen
    );
    let expected_cwd = format!("REMOTE_CONTAINER_CWD:{REMOTE_CWD}");
    ensure!(
        dump.screen.iter().any(|line| line == &expected_cwd),
        "the split did not inherit the OSC 7 directory: {:?}",
        dump.screen
    );
    let expected_status = ["state=working id=\"remote\" progress=73 app=\"remote-build\""];
    ensure!(
        dump.status == expected_status,
        "remote status {:?}, expected {expected_status:?}",
        dump.status
    );
    let notifications =
        fs::read_to_string(ctx.dir.join("container-pty.notify")).unwrap_or_default();
    ensure!(
        notifications.lines().any(|line| line.starts_with("osc9\t")),
        "remote OSC 9 notification missing: {notifications:?}"
    );
    ensure!(
        notifications
            .lines()
            .any(|line| line.starts_with("osc777\tRemote build")),
        "remote OSC 777 notification missing: {notifications:?}"
    );
    container.assert_absent(marker)?;
    container.exec(&["rm", "--force", &planted])?;
    // The container runs no systemd, so its relay starts the core without a
    // scope and says so plainly: no user manager could have given one, so no
    // warning that the sessions may not outlive the login.
    let logs = container.exec(&["sh", "-c", SERVER_LOGS])?;
    ensure!(
        logs.contains("starting the core without a scope of its own"),
        "the container's core.log doesn't say the core started without a scope: {logs:?}"
    );
    ensure!(
        !logs.contains("may not outlive"),
        "the container's core.log warns about the login: {logs:?}"
    );

    Ok(Outcome::Pass)
}

/// The prompt (`PS1`) of [`in_container_tab`]'s tabs, without its trailing
/// space.
pub(super) const CONTAINER_PROMPT: &str = "REMOTE_CONTAINER_READY>";

/// Runs `case` with a [`Run`] that opens a [`CONTAINER_PROFILE`] tab, a bash
/// in a disposable container, for a case elsewhere that needs a remote tab:
/// a new tab there starts in the directory the active one reported, where a
/// local Linux tab's starts in its process's. Skips without Docker or the
/// helper, as this file's cases do; the container is removed afterwards.
pub(super) fn in_container_tab(
    ctx: &Ctx,
    label: &'static str,
    case: impl FnOnce(Run<'_>) -> Result<Outcome>,
) -> Result<Outcome> {
    if let Some(outcome) = docker_outcome()? {
        return Ok(outcome);
    }
    let Some(agent) = remote_agent(ctx)? else {
        return Ok(Outcome::Skip(
            "Linux remote helper was not built; run `cargo build -p brindle-remote-agent`".into(),
        ));
    };
    let image = fixture_image(ctx)?;
    let container = Container::start(ctx, &image, &[])?;
    let config = format!(
        "\n[[profiles]]\n\
         name = {CONTAINER_PROFILE:?}\n\
         command = \"/bin/bash\"\n\
         args = [\"--norc\", \"--noprofile\", \"-i\"]\n\
         env = {{ SHELL = \"/bin/bash\", PS1 = \"{CONTAINER_PROMPT} \" }}\n\
         remote = {{ type = \"container\", engine = \"docker\", container = {:?}, user = \"remote\" }}\n",
        container.name,
    );
    let run = agent.give(
        Run::new(ctx, label)
            .profile(CONTAINER_PROFILE)
            .default_profile(CONTAINER_PROFILE)
            .config(&config),
    );
    let outcome = case(run);
    drop(container);
    outcome
}

/// An SSH endpoint starts without a cached helper. Brindle must upload it
/// through the generated key before the remote interactive shell can run.
pub fn ssh_container(ctx: &Ctx) -> Result<Outcome> {
    let needs = match Needs::check(ctx)? {
        Ok(needs) => needs,
        Err(skip) => return Ok(skip),
    };
    let ssh = Ssh::start(ctx, &needs.image, Net::Published)?;
    let cache_dir = "/root/.cache/brindle";
    ssh.container.assert_absent(cache_dir)?;

    let config = ssh.profile(SSH_PROFILE, "ssh-upload", false);
    let label = "ssh-pty";
    let dump = needs
        .run(ctx, label, &config)
        .profile(SSH_PROFILE)
        .default_profile(SSH_PROFILE)
        .env("BRINDLE_NOTIFY_LOG", &notify_log(ctx, label))
        .wait_for(SSH_PROMPT)
        .send("printf 'SSH_REMOTE_IO:%s' \"$BRINDLE_E2E_MARKER\"; echo\\r")
        .wait_for("SSH_REMOTE_IO:ssh-upload")
        .dump_at(12.0)
        .run()?
        .dump;
    ensure!(
        dump.screen
            .iter()
            .any(|line| line == "SSH_REMOTE_IO:ssh-upload"),
        "SSH remote PTY output missing from screen: {:?}",
        dump.screen
    );
    ssh.container.assert_cached_agent(cache_dir)?;

    Ok(Outcome::Pass)
}

/// The agent profile [`agent_ssh`] opens.
const AGENT_PROFILE: &str = "Remote Agent";

/// An agent profile over SSH: the agent session runs in root's terminal
/// server in the container, which the first connection deploys, with the
/// static fake ACP agent copied in to play the agent. `brindle agent
/// "Remote Agent" -p hi` sends `hi` once the agent is ready and shows its
/// answer; the agent recorded one prompt. The agent then exits, and
/// `agent_continue` starts the profile's command again in the container,
/// on the same session, which resumes the conversation: a new prompt
/// reaches the continued agent alone, and after the quit root's server
/// lists the one session. Skips like the other remote cases, and also when the static
/// fake agent isn't built (an error with `BRINDLE_REMOTE_E2E_REQUIRED=1`).
pub fn agent_ssh(ctx: &Ctx) -> Result<Outcome> {
    let needs = match Needs::check(ctx)? {
        Ok(needs) => needs,
        Err(skip) => return Ok(skip),
    };
    let Some(fake) = static_fake_agent()? else {
        return Ok(Outcome::Skip(format!(
            "the static fake ACP agent isn't built ({} is missing): run {}",
            crate::common::fake_agent::STATIC_LINUX,
            crate::common::fake_agent::STATIC_LINUX_BUILD
        )));
    };
    let ssh = Ssh::start(ctx, &needs.image, Net::Published)?;
    use super::agent::{EXITED, READY, RESTARTED, RESUME, Script};
    let agent = "/usr/local/bin/brindle-fake-acp-agent";
    let (script, next, gate) = ("/root/agent.script", "/root/next.script", "/root/exit");
    let (first, second) = ("/root/first.jsonl", "/root/second.jsonl");
    ssh.container.copy_to(&fake, agent)?;
    ssh.container.exec(&["chmod", "755", agent])?;
    // The first agent answers, then exits once the case opens `gate`; the
    // continue runs the profile's command again, so the case first puts
    // the second agent's script in the first's place in the container.
    let first_agent = super::agent::turn_then_exit(Path::new(first), RESUME, Path::new(gate)).write(ctx, "agent")?;
    let second_agent = Script::initialized(Path::new(second), RESUME)
        .step("expect session/resume")
        .step("respond {}")
        .step("expect session/prompt")
        .step(super::agent::says("Again from the container"))
        .step(super::agent::stop("end_turn"))
        .write(ctx, "next")?;
    ssh.container.copy_to(&first_agent, script)?;
    ssh.container.copy_to(&second_agent, next)?;

    let config = ssh.agent_profile(AGENT_PROFILE, agent, script);
    let (docker, scale) = (ssh.container.docker.clone(), ctx.scale());
    let out = needs
        .run(ctx, "agent", &config)
        .agent(AGENT_PROFILE)
        .first_prompt("hi")
        .keep_detached()
        // Once the first agent has its prompt (it read its script when it
        // started), the second's script takes its place and the gate opens.
        .call_at(1.0, move |_| {
            scale.eventually(60.0, "the first agent's prompt", || {
                docker.exec(&["cat", first]).is_ok_and(|sent| sent.contains("\"session/prompt\""))
            })?;
            docker.exec(&["sh", "-c", &format!("cp {next} {script} && touch {gate}")])?;
            Ok(())
        })
        .wait_for(&super::agent::answered())
        .wait_for(EXITED)
        .wait_for("continue=restart")
        .action("agent_continue")
        .wait_for(READY)
        .send("again")
        .key("enter")
        .wait_for("agent \"Again from the container\"")
        .dump_at(25.0)
        .run()?;
    super::agent::waited(&out)?;
    let want = ["user \"hi\"", "agent \"Hello!\"", "user \"again\"", "agent \"Again from the container\""];
    ensure!(super::agent::messages(&out) == want, "messages: {:?}", out.dump.agent);
    ensure!(super::agent::has_notice(&out, RESTARTED), "no restart notice: {:?}", out.dump.agent);
    ensure!(out.dump.agent_field("turn") == Some("ended:end_turn"), "agent: {:?}", out.dump.agent);
    ensure!(out.dump.agent_field("session") == Some("open"), "agent: {:?}", out.dump.agent);
    let prompts = |sent: &str| sent.lines().filter(|line| line.contains("\"session/prompt\"")).count();
    let sent = ssh.container.exec(&["cat", first])?;
    ensure!(prompts(&sent) == 1, "the first agent was sent one prompt: {sent}");
    let sent = ssh.container.exec(&["cat", second])?;
    ensure!(prompts(&sent) == 1 && sent.contains("\"text\":\"again\""), "the continued agent was sent `again` alone: {sent}");
    ensure!(
        sent.lines().any(|line| line.contains("\"session/resume\"") && line.contains("\"sessionId\":\"s1\"")),
        "no session/resume of s1: {sent}"
    );
    ensure!(!sent.contains("\"session/new\""), "a new conversation was started: {sent}");
    ssh.container.assert_cached_agent("/root/.cache/brindle")?;
    // Quitting left the continued agent's session, the same one, detached
    // in root's server.
    let listed = parse_list(&ssh.container.helper(&["list"])?);
    ensure!(listed.len() == 1 && !listed[0].attached, "root's server lists {listed:?}");
    Ok(Outcome::Pass)
}

/// The static fake ACP agent ([`crate::common::fake_agent::STATIC_LINUX`])
/// when it is built and no older than its sources; `None` when it is
/// missing, an error then with `BRINDLE_REMOTE_E2E_REQUIRED=1`.
fn static_fake_agent() -> Result<Option<PathBuf>> {
    use crate::common::fake_agent::{self, FakeAgent, STATIC_LINUX, STATIC_LINUX_BUILD};
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(STATIC_LINUX);
    if !path.is_file() {
        if std::env::var("BRINDLE_REMOTE_E2E_REQUIRED").as_deref() == Ok("1") {
            return Err(format!(
                "the static fake ACP agent isn't built ({} is missing): run {STATIC_LINUX_BUILD}",
                path.display()
            ));
        }
        return Ok(None);
    }
    fake_agent::check_built_by(&FakeAgent { path: path.clone(), named: false }, STATIC_LINUX_BUILD)?;
    Ok(Some(path))
}

/// Quitting leaves an SSH tab's program running, detached in the
/// container's server, and the next launch restores the tab attached to the
/// same session: the same program (same pid) and its scrollback.
pub fn ssh_quit(ctx: &Ctx) -> Result<Outcome> {
    let needs = match Needs::check(ctx)? {
        Ok(needs) => needs,
        Err(skip) => return Ok(skip),
    };
    let ssh = Ssh::start(ctx, &needs.image, Net::Published)?;
    let config = ssh.profile(SSH_PROFILE, "ssh-quit", false);
    let first = needs
        .run(ctx, "first", &config)
        .restore()
        .default_profile(SSH_PROFILE)
        .wait_for(SSH_PROMPT)
        .send(concat!(
            "seq -f scroll-%g 1 300; ",
            "sh -c 'echo program=$$; echo sleeping-$((40+2)); exec sleep 1000'\r"
        ))
        .wait_for("sleeping-42")
        .dump_at(8.0)
        .run()?;
    no_waits_left(&first.dump)?;
    let program = shown_pid(&first.dump, "program=")?;
    let docker = ssh.container.docker.clone();
    let left = ctx.scale().poll(10.0, "the tab's session detached", || {
        let sessions = docker.sessions()?;
        Ok(match sessions.as_slice() {
            [session] if !session.attached => Some(session.clone()),
            _ => None,
        })
    })?;
    ctx.save("left.txt", format!("{left:?}\nprogram {program}\n"));
    docker.runs_in(program, "sleep", left.pid)?;

    let attached = ctx.dir.join("attached.txt");
    let (save, checked, scale) = (attached.clone(), docker.clone(), ctx.scale());
    let second = needs
        .run(ctx, "second", &config)
        .restore()
        .default_profile(SSH_PROFILE)
        .wait_for("sleeping-42")
        .action("scroll_to_top")
        .call_at(1.0, move |_| {
            let again = scale.poll(30.0, "the session attached again", || {
                let sessions = checked.sessions()?;
                Ok(match sessions.as_slice() {
                    [session] if session.attached => Some(session.clone()),
                    _ => None,
                })
            })?;
            fs::write(&save, format!("{} {}", again.id, again.pid)).map_err(|e| e.to_string())
        })
        .dump_at(8.0)
        .run()?;
    no_waits_left(&second.dump)?;
    let again = fs::read_to_string(&attached).map_err(|_| "the relaunch wasn't checked")?;
    ensure!(
        again == format!("{} {}", left.id, left.pid),
        "the relaunch attached session (id pid) {again}, not {} {}",
        left.id,
        left.pid
    );
    // Titled by the foreground program, which the host's server reports.
    ensure!(
        second.dump.tabs == [("sleep".to_string(), true)],
        "tabs: {:?}",
        second.dump.tabs
    );
    ensure!(
        second.dump.zones.is_none(),
        "zones: {:?}",
        second.dump.zones
    );
    // The top of the scrollback, which the replay brought back.
    ensure!(
        second.dump.screen.iter().any(|line| line == "scroll-1"),
        "no scroll-1 at the top: {:?}",
        second.dump.screen
    );
    docker.runs_in(program, "sleep", left.pid)?;
    Ok(Outcome::Pass)
}

/// A restored SSH tab takes its window's size: a first run leaves an SSH
/// tab's shell detached, and the next launch, whose grid is smaller (more
/// padding), restores the tab attached to it. Once attached, the remote
/// PTY's size (`stty size`) is the restored terminal's grid, not the size
/// the session was detached at.
pub fn ssh_reattach_size(ctx: &Ctx) -> Result<Outcome> {
    // The command's own echo says `bef""ore=`, so only its output matches.
    let size = |marker: &str| {
        let (head, tail) = marker.split_at(3);
        format!("echo \"{head}\"\"{tail}=$(stty size)\"\r")
    };
    let needs = match Needs::check(ctx)? {
        Ok(needs) => needs,
        Err(skip) => return Ok(skip),
    };
    let ssh = Ssh::start(ctx, &needs.image, Net::Published)?;
    let config = ssh.profile(SSH_PROFILE, "ssh-reattach-size", false);
    let first = needs
        .run(ctx, "first", &config)
        .restore()
        .default_profile(SSH_PROFILE)
        .wait_for(SSH_PROMPT)
        .send(&size("before"))
        .wait_for("before=")
        .dump_at(8.0)
        .run()?;
    no_waits_left(&first.dump)?;
    let before = shown_stty_size(&first.dump, "before=")?;
    ensure!(
        first.dump.size == Some(before),
        "the first run's PTY is {before:?}, its grid {:?}",
        first.dump.size
    );
    let docker = ssh.container.docker.clone();
    ctx.scale().poll(10.0, "the tab's session detached", || {
        let sessions = docker.sessions()?;
        Ok(matches!(sessions.as_slice(), [session] if !session.attached).then_some(()))
    })?;

    let second = needs
        .run(ctx, "second", &config)
        .restore()
        .default_profile(SSH_PROFILE)
        // A smaller grid in the same window.
        .padding(80)
        // The replay brings the first run's line back.
        .wait_for("before=")
        .send(&size("after"))
        .wait_for("after=")
        .dump_at(10.0)
        .run()?;
    no_waits_left(&second.dump)?;
    let after = shown_stty_size(&second.dump, "after=")?;
    ensure!(
        second.dump.size != Some(before),
        "the second run's grid {:?} is the first's: the case checks nothing",
        second.dump.size
    );
    ensure!(
        second.dump.size == Some(after),
        "the reattached session's PTY is {after:?}, its grid {:?} (detached at {before:?})",
        second.dump.size
    );
    Ok(Outcome::Pass)
}

/// A restored SSH tab in the background shows what its program draws at the
/// session's own size. A first run leaves a local tab (active) and an SSH tab
/// detached, the SSH tab's shell waiting for a file in the container. The
/// restoring run reattaches the SSH tab while the local one is shown, so
/// nothing has laid the SSH tab out; the file is then created, and the
/// program clears the screen and draws `MARK` at column 90, past the
/// default grid's 80 columns. Shown, the tab has `MARK` at column 90: the
/// draw landed on a grid at the session's size, not clamped at column 80.
pub fn ssh_reattach_background(ctx: &Ctx) -> Result<Outcome> {
    const DRAWN: &str = "BACKGROUND-DRAWN";
    let needs = match Needs::check(ctx)? {
        Ok(needs) => needs,
        Err(skip) => return Ok(skip),
    };
    let ssh = Ssh::start(ctx, &needs.image, Net::Published)?;
    let config = ssh.profile(SSH_PROFILE, "ssh-reattach-background", false);
    let first = needs
        .run(ctx, "first", &config)
        .restore()
        // The local tab's session is the next run's too.
        .keep_detached()
        .action("new_tab_with_profile_2")
        .wait_for(SSH_PROMPT)
        .send(concat!(
            "while [ ! -e /tmp/go ]; do sleep 0.1; done; ",
            "printf '\\033[H\\033[2J\\033[1;90HMARK'; touch /tmp/drawn; exec sleep 1000\r"
        ))
        .action("prev_tab")
        .dump_at(8.0)
        .run()?;
    no_waits_left(&first.dump)?;
    let tabs = &first.dump.tabs;
    ensure!(tabs.len() == 2 && tabs[0].1, "tabs: {tabs:?}");
    let docker = ssh.container.docker.clone();
    ctx.scale().poll(10.0, "the SSH tab's session detached", || {
        let sessions = docker.sessions()?;
        Ok(matches!(sessions.as_slice(), [session] if !session.attached).then_some(()))
    })?;
    ctx.scale().poll(10.0, "the local tab's session detached", || {
        let local = local_sessions(&ctx.helper, &ctx.server_dir)?;
        Ok(matches!(local.as_slice(), [session] if !session.attached).then_some(()))
    })?;

    let (helper, server_dir, scale) = (ctx.helper.clone(), ctx.server_dir.clone(), ctx.scale());
    let second = needs
        .run(ctx, "second", &config)
        .restore()
        .call_at(1.0, move |_| {
            scale.poll(30.0, "the SSH tab's session attached again", || {
                let sessions = docker.sessions()?;
                Ok(matches!(sessions.as_slice(), [session] if session.attached).then_some(()))
            })?;
            docker.exec(&["touch", "/tmp/go"])?;
            scale.poll(10.0, "the SSH tab's program drawn", || {
                Ok(docker.exec(&["test", "-e", "/tmp/drawn"]).is_ok().then_some(()))
            })?;
            let local = scale.poll(10.0, "the restored local tab", || {
                Ok(local_sessions(&helper, &server_dir)?
                    .into_iter()
                    .find(|s| s.attached))
            })?;
            mark_tty(local.pid, DRAWN)
        })
        // The local tab, active at the quit, is shown while the SSH tab's
        // program draws.
        .wait_for(DRAWN)
        .action("next_tab")
        .dump_at(10.0)
        .run()?;
    no_waits_left(&second.dump)?;
    let tabs = &second.dump.tabs;
    ensure!(tabs.len() == 2 && tabs[1].1, "tabs: {tabs:?}");
    // The drawn line, not the command line that typed it.
    let marked = second
        .dump
        .screen
        .iter()
        .find(|line| line.trim_start().starts_with("MARK"))
        .map(|line| line.len() - line.trim_start().len());
    ensure!(
        marked == Some(89),
        "MARK at column {marked:?}, not 89 (from 0), on a {:?} grid: {:?}",
        second.dump.size,
        second.dump.screen
    );
    Ok(Outcome::Pass)
}

/// The `(cols, rows)` that a `MARKERROWS COLS` line (`stty size`) shows.
fn stty_size(line: &str, marker: &str) -> Option<(u16, u16)> {
    let (rows, cols) = line.strip_prefix(marker)?.trim().split_once(' ')?;
    Some((cols.parse().ok()?, rows.parse().ok()?))
}

/// The size the last line starting with `marker` on the screen shows.
fn shown_stty_size(dump: &Dump, marker: &str) -> Result<(u16, u16)> {
    dump.screen
        .iter()
        .rev()
        .find_map(|line| stty_size(line, marker))
        .ok_or_else(|| format!("no {marker:?} line: {:?}", dump.screen))
}

/// An SSH tab split twice keeps one connection: Brindle runs one `ssh` and
/// the container one relay for the three zones, whose sessions its server
/// lists. Closing the tab ends the three programs, and then the unused
/// connection's `ssh`.
pub fn ssh_split(ctx: &Ctx) -> Result<Outcome> {
    const COUNTED: &str = "CONNECTIONS-COUNTED";
    let needs = match Needs::check(ctx)? {
        Ok(needs) => needs,
        Err(skip) => return Ok(skip),
    };
    let ssh = Ssh::start(ctx, &needs.image, Net::Published)?;
    let counts = ctx.dir.join("counts.txt");
    let (save, docker, scale) = (counts.clone(), ssh.container.docker.clone(), ctx.scale());
    let out = needs
        .run(ctx, "run", &ssh.profile(SSH_PROFILE, "ssh-split", false))
        // A local tab first, so the window stays once the SSH tab closes.
        .action("new_tab_with_profile_2")
        .wait_for(SSH_PROMPT)
        .action("split_zone_right")
        .wait_for(SSH_PROMPT)
        .action("split_zone_down")
        .wait_for(SSH_PROMPT)
        .wait_for(COUNTED)
        .action("close_tab")
        .call_at(1.0, move |brindle| {
            count_connections(&docker, brindle, scale, &save, COUNTED)
        })
        .dump_at(22.0)
        .run()?;
    no_waits_left(&out.dump)?;
    ensure!(out.dump.tabs.len() == 1, "tabs: {:?}", out.dump.tabs);
    ensure!(
        fs::read_to_string(&counts).is_ok(),
        "the connections weren't counted"
    );
    Ok(Outcome::Pass)
}

/// `remote-ssh-split`'s timeline: once the SSH tab's three zones are
/// attached, Brindle (`brindle`) has one `ssh` and the container one relay,
/// saved to `save`. Then it marks the active zone with `counted`, which
/// lets Brindle close the tab, and the three programs, then the `ssh` and
/// the relay, go.
fn count_connections(
    docker: &Docker,
    brindle: u32,
    scale: Scale,
    save: &Path,
    counted: &str,
) -> Result {
    let sessions = scale.poll(30.0, "three attached sessions", || {
        let sessions = docker.sessions()?;
        let all = sessions.len() == 3 && sessions.iter().all(|s| s.attached);
        Ok(all.then_some(sessions))
    })?;
    let (links, relays) = (ssh_processes(brindle), docker.relays()?);
    let text = format!("sessions {sessions:?}\nssh {links:?}\nrelays {relays:?}\n");
    fs::write(save, text).map_err(|e| e.to_string())?;
    ensure!(
        links.len() == 1,
        "Brindle's ssh processes for the three zones: {links:?}"
    );
    ensure!(
        relays.len() == 1,
        "the container's relays for the three zones: {relays:?}"
    );
    // The active zone is the last split's, the newest session.
    let newest = sessions.iter().max_by_key(|s| s.id).ok_or("no sessions")?;
    docker.mark(newest.pid, counted)?;
    let pids: Vec<u32> = sessions.iter().map(|s| s.pid).collect();
    scale.poll(15.0, "the closed tab's programs gone", || {
        let gone = pids.iter().all(|&pid| docker.stat(pid).is_none());
        Ok((gone && docker.sessions()?.is_empty()).then_some(()))
    })?;
    // The connection closes a few seconds after nothing uses it.
    scale.poll(20.0, "the unused connection's ssh gone", || {
        Ok(ssh_processes(brindle).is_empty().then_some(()))
    })?;
    scale.poll(10.0, "its relay gone", || {
        Ok(docker.relays()?.is_empty().then_some(()))
    })
}

/// The container is cut off its network while an SSH tab runs a shell, so
/// the link goes silent without closing. Brindle notices (ssh's keepalives,
/// else its own dead-link limit), says so in the tab, and keeps trying,
/// failing while the network is down. Once it is back, the tab has the same
/// session (same id and pid, one relay left), what was typed meanwhile runs
/// once, and new output and input work.
pub fn ssh_network(ctx: &Ctx) -> Result<Outcome> {
    const PID_FILE: &str = "/tmp/brindle-e2e-shell-pid";
    let needs = match Needs::check(ctx)? {
        Ok(needs) => needs,
        Err(skip) => return Ok(skip),
    };
    let ssh = Ssh::start(ctx, &needs.image, Net::Private)?;
    let network = ssh
        .network
        .as_ref()
        .ok_or("the SSH fixture has no network")?;
    let timings = ctx.dir.join("timings.txt");
    let cut = Cut {
        docker: ssh.container.docker.clone(),
        network: network.name.clone(),
        address: ssh.host.clone(),
        endpoint: format!("root@{}:{}", ssh.host, ssh.port),
        pid_file: PID_FILE,
        log: ctx.dir.join("run.log"),
        save: timings.clone(),
        scale: ctx.scale(),
    };
    let out = needs
        .run(ctx, "run", &ssh.profile(SSH_PROFILE, "ssh-network", false))
        .profile(SSH_PROFILE)
        // Waits of up to 45 s: the loss takes up to the dead-link limit.
        .step_interval(1.5)
        .wait_for(SSH_PROMPT)
        .send(&format!("echo $$ > {PID_FILE}; echo before-$((1+1)) $$\r"))
        .wait_for("before-2")
        .wait_for("); reconnecting")
        // Typed while the link is down: queued, and run once it is back.
        .send("echo queued-$((40+2))\r")
        .wait_for("queued-42")
        .send("echo after-$((1+1)) $$\r")
        .wait_for("after-2")
        .call_at(1.0, move |brindle| cut.run(brindle))
        // The steps end about 30 s in, when ssh's keepalives notice the cut
        // (about 20 s); this leaves room for the dead-link limit (30 s).
        .dump_at(50.0)
        .run()?;
    no_waits_left(&out.dump)?;
    // The lost line the run waited for names why the link ended in
    // parentheses (it is gone from the screen by now, the reconnect's replay
    // having redrawn it): ssh's own account, or that nothing was read for
    // the dead-link limit, which the log says in the same words.
    let log = fs::read_to_string(ctx.dir.join("run.log")).unwrap_or_default();
    ensure!(
        log.contains("ended (ssh") || log.contains("ended (nothing read"),
        "the link's end doesn't name its cause: {log:?}"
    );
    let before = shown_pid(&out.dump, "before-2 ")?;
    let after = shown_pid(&out.dump, "after-2 ")?;
    ensure!(
        before == after,
        "the shell was {before}, and is {after} after the reconnect"
    );
    let queued = out
        .dump
        .screen
        .iter()
        .filter(|line| *line == "queued-42")
        .count();
    ensure!(
        queued == 1,
        "what was typed while disconnected ran {queued} times: {:?}",
        out.dump.screen
    );
    ensure!(
        fs::read_to_string(&timings).is_ok(),
        "the reconnect wasn't checked"
    );
    Ok(Outcome::Pass)
}

/// `remote-ssh-network`'s timeline: cuts the container off its network
/// once the tab's shell runs, and restores it once Brindle has noticed and
/// failed to reconnect, checking that the same session comes back.
struct Cut {
    docker: Docker,
    network: String,
    /// The container's address on `network`, kept across the cut.
    address: String,
    /// The endpoint, as Brindle's log names it (`root@ADDRESS:22`).
    endpoint: String,
    /// Where the tab's shell writes its pid in the container.
    pid_file: &'static str,
    /// Brindle's log, which says when the link ended and a retry failed.
    log: PathBuf,
    /// Where the timings go, for the report.
    save: PathBuf,
    scale: Scale,
}

impl Cut {
    fn run(&self, brindle: u32) -> Result {
        let (docker, scale) = (&self.docker, self.scale);
        let shell = scale.poll(30.0, "the shell's pid", || {
            let text = docker.exec(&["cat", self.pid_file]).ok();
            Ok(text.and_then(|pid| pid.trim().parse::<u32>().ok()))
        })?;
        let session = scale.poll(10.0, "the shell's session attached", || {
            let sessions = docker.sessions()?;
            Ok(sessions.into_iter().find(|s| s.attached && s.pid == shell))
        })?;
        let links = ssh_processes(brindle);
        ensure!(
            links.len() == 1,
            "Brindle's ssh processes before the cut: {links:?}"
        );
        let cut = Instant::now();
        docker.disconnect(&self.network)?;
        let ended = format!("connection to {} ended", self.endpoint);
        scale.poll(45.0, "the silent link noticed", || {
            let noticed = logged(&self.log, &ended) > 0 && !alive(links[0]);
            Ok(noticed.then_some(()))
        })?;
        let noticed = cut.elapsed();
        // The log says what ended the link: ssh's own exit, or the dead-link
        // limit, and never the local core's wording.
        let line = log_line(&self.log, &ended).ok_or("the link's end isn't in Brindle's log")?;
        ensure!(
            line.contains(&format!("{ended} (")) && !line.contains("the terminal server closed the connection"),
            "the link ended without a cause of its own: {line:?}"
        );
        ensure!(
            line.contains("ssh exited with 255") || line.contains("nothing read from"),
            "the link's end doesn't name ssh's exit or the silence: {line:?}"
        );
        // Brindle retries while the network is down, and fails.
        let failed = format!("can't connect to {}", self.endpoint);
        scale.poll(30.0, "a reconnect failing", || {
            Ok((logged(&self.log, &failed) > 0).then_some(()))
        })?;
        let down = cut.elapsed();
        docker.connect(&self.network, &self.address)?;
        let restored = Instant::now();
        let again = scale.poll(30.0, "the session attached again", || {
            let sessions = docker.sessions()?;
            Ok(match sessions.as_slice() {
                [again] if again.attached => Some(again.clone()),
                _ => None,
            })
        })?;
        let back = restored.elapsed();
        ensure!(
            (again.id, again.pid) == (session.id, session.pid),
            "after the reconnect the session is {again:?}, not {session:?}"
        );
        // The new relay took the client over, and the old one ends.
        let relays = scale.poll(15.0, "one relay left", || {
            let relays = docker.relays()?;
            Ok((relays.len() == 1).then_some(relays))
        })?;
        let relayed = cut.elapsed();
        let links = ssh_processes(brindle);
        let secs = |d: Duration| format!("{:.1}s", d.as_secs_f64());
        let text = format!(
            "shell {shell}\nnoticed {}\nfailed attempt by {}\nback {} after the restore\n\
             one relay {} after the cut: {relays:?}\nssh {links:?}\n",
            secs(noticed),
            secs(down),
            secs(back),
            secs(relayed),
        );
        fs::write(&self.save, text).map_err(|e| e.to_string())?;
        ensure!(
            links.len() == 1,
            "Brindle's ssh processes after the reconnect: {links:?}"
        );
        Ok(())
    }
}

/// A profile that connects at launch: a session an earlier run left
/// detached on its host (by quitting, so it has no owner) opens as a tab of
/// its own after the launch's tabs, and the active tab stays the launch's.
pub fn ssh_connect_at_launch(ctx: &Ctx) -> Result<Outcome> {
    const SWEPT: &str = "SESSION-SWEPT";
    let needs = match Needs::check(ctx)? {
        Ok(needs) => needs,
        Err(skip) => return Ok(skip),
    };
    let ssh = Ssh::start(ctx, &needs.image, Net::Published)?;
    let first = needs
        .run(ctx, "first", &ssh.profile(SSH_PROFILE, "ssh-eager", false))
        .profile(SSH_PROFILE)
        .wait_for(SSH_PROMPT)
        .send("echo left-$((1+1)) $$\r")
        .wait_for("left-2")
        .dump_at(6.0)
        .run()?;
    no_waits_left(&first.dump)?;
    let shell = shown_pid(&first.dump, "left-2 ")?;
    let docker = ssh.container.docker.clone();
    let left = ctx.scale().poll(10.0, "the tab's session detached", || {
        let sessions = docker.sessions()?;
        Ok(match sessions.as_slice() {
            [session] if !session.attached && session.pid == shell => Some(session.clone()),
            _ => None,
        })
    })?;
    ensure!(
        !left.owned,
        "the quit left its session owned, so a launch leaves it alone for 5 minutes: {left:?}"
    );
    // Nothing records the session, so only the launch's connection brings
    // it back.
    fs::remove_dir_all(brindle::state_dir(ctx)).ok();

    let (helper, server_dir, scale) = (ctx.helper.clone(), ctx.server_dir.clone(), ctx.scale());
    let second = needs
        .run(ctx, "second", &ssh.profile(SSH_PROFILE, "ssh-eager", true))
        .restore()
        .wait_for(SWEPT)
        .action("next_tab")
        .wait_for("left-2")
        .call_at(1.0, move |_| {
            scale.poll(30.0, "the detached session attached at launch", || {
                let sessions = docker.sessions()?;
                Ok(sessions
                    .iter()
                    .any(|s| s.attached && s.pid == shell)
                    .then_some(()))
            })?;
            // The launch's own tab is the local one, which shows the marker.
            let local = scale.poll(10.0, "the launch's local tab", || {
                Ok(local_sessions(&helper, &server_dir)?
                    .into_iter()
                    .find(|s| s.attached))
            })?;
            mark_tty(local.pid, SWEPT)
        })
        .dump_at(10.0)
        .run()?;
    no_waits_left(&second.dump)?;
    // `next_tab` went from the first tab to the second, so the launch's tab
    // was still the active one.
    let tabs = &second.dump.tabs;
    ensure!(tabs.len() == 2 && tabs[1].1, "tabs: {tabs:?}");
    let shown = format!("left-2 {shell}");
    ensure!(
        second.dump.screen.contains(&shown),
        "the attached tab doesn't show {shown:?}: {:?}",
        second.dump.screen
    );
    Ok(Outcome::Pass)
}

/// A restored SSH tab closed while its host refuses: a first run leaves a
/// local tab and an SSH tab detached, and the SSH fixture is then stopped,
/// so its port refuses. The restoring run shows the SSH zone connecting and
/// closes its tab, the window's last on that host, which isn't one a profile
/// connects to at launch: that withdraws the launch's wait for the host's
/// sessions, so its connection ends, and Brindle dials it no
/// more (no `ssh`, no failed attempt), and "Attach Session…" no longer
/// lists it.
pub fn ssh_restored_closed(ctx: &Ctx) -> Result<Outcome> {
    const CHECKED: &str = "CLOSED-CHECKED";
    let needs = match Needs::check(ctx)? {
        Ok(needs) => needs,
        Err(skip) => return Ok(skip),
    };
    let ssh = Ssh::start(ctx, &needs.image, Net::Published)?;
    let config = ssh.profile(SSH_PROFILE, "ssh-restored-closed", false);
    let first = needs
        .run(ctx, "first", &config)
        .restore()
        // The local tab's session is the next run's too.
        .keep_detached()
        .action("new_tab_with_profile_2")
        .wait_for(SSH_PROMPT)
        .dump_at(8.0)
        .run()?;
    no_waits_left(&first.dump)?;
    let tabs = &first.dump.tabs;
    ensure!(tabs.len() == 2 && tabs[1].1, "tabs: {tabs:?}");
    let docker = ssh.container.docker.clone();
    ctx.scale().poll(10.0, "the SSH tab's session detached", || {
        let sessions = docker.sessions()?;
        Ok(matches!(sessions.as_slice(), [session] if !session.attached).then_some(()))
    })?;
    ctx.scale().poll(10.0, "the local tab's session detached", || {
        let local = local_sessions(&ctx.helper, &ctx.server_dir)?;
        Ok(matches!(local.as_slice(), [session] if !session.attached).then_some(()))
    })?;
    ssh.container.stop()?;
    let (host, port) = (ssh.host.clone(), ssh.port);
    ctx.scale().poll(10.0, "the stopped fixture's port refusing", || {
        Ok(TcpStream::connect((host.as_str(), port)).is_err().then_some(()))
    })?;

    let endpoint = format!("root@{}:{}", ssh.host, ssh.port);
    let checks = ctx.dir.join("checks.txt");
    let closed = ClosedTab {
        endpoint: endpoint.clone(),
        log: ctx.dir.join("second.log"),
        save: checks.clone(),
        helper: ctx.helper.clone(),
        server_dir: ctx.server_dir.clone(),
        checked: CHECKED,
        scale: ctx.scale(),
    };
    let second = needs
        .run(ctx, "second", &config)
        .restore()
        // The checks take up to 25 s after the close, which the marker's
        // wait (30 step intervals) must cover.
        .step_interval(1.5)
        // The restored SSH tab is the active one, as it was at the quit.
        .wait_for(&format!("connecting to {endpoint}"))
        .action("close_tab")
        .wait_for(CHECKED)
        .action("attach_session")
        .call_at(1.0, move |brindle| closed.run(brindle))
        .dump_at(40.0)
        .run()?;
    no_waits_left(&second.dump)?;
    ensure!(
        second.dump.tabs.len() == 1,
        "tabs: {:?}",
        second.dump.tabs
    );
    ensure!(
        fs::read_to_string(&checks).is_ok(),
        "the closed tab's connection wasn't checked"
    );
    let palette = &second.dump.palette;
    ensure!(!palette.is_empty(), "\"Attach Session…\" didn't open");
    ensure!(
        !palette.iter().any(|row| row.contains(&endpoint)),
        "\"Attach Session…\" still lists {endpoint}: {palette:?}"
    );
    Ok(Outcome::Pass)
}

/// `remote-ssh-restored-closed`'s timeline: once the restored SSH tab has
/// closed, withdrawing the launch's wait for its endpoint's sessions (the
/// log says so), the endpoint's connection ends within 15 s, unused or at
/// its next failed attempt, and its `ssh` is gone, and in the 10 s after
/// that Brindle starts no `ssh` and fails no attempt. Then it marks the
/// local tab with `checked`.
struct ClosedTab {
    /// The endpoint, as Brindle's log names it (`root@HOST:PORT`).
    endpoint: String,
    /// Brindle's log.
    log: PathBuf,
    /// Where the timings go, for the report.
    save: PathBuf,
    /// The case's terminal server, which the local tab runs in.
    helper: PathBuf,
    server_dir: PathBuf,
    checked: &'static str,
    scale: Scale,
}

impl ClosedTab {
    fn run(&self, brindle: u32) -> Result {
        let scale = self.scale;
        let withdrawn = format!("not waiting for {}'s sessions any more", self.endpoint);
        scale.poll(30.0, "the SSH tab closed, withdrawing the launch's wait", || {
            Ok((logged(&self.log, &withdrawn) > 0).then_some(()))
        })?;
        let closed = Instant::now();
        // It was dialling the stopped endpoint until then.
        let failed = format!("can't connect to {}", self.endpoint);
        ensure!(
            logged(&self.log, &failed) > 0,
            "no failed attempt on {} before the close",
            self.endpoint
        );
        // With nothing waiting on it, the connection ends at its next
        // failed attempt, which logs only the failure, or `Timings::idle`
        // (5 s) later, unused, whichever comes first.
        let at_close = logged(&self.log, &failed);
        let unused = format!("closing the unused connection to {}", self.endpoint);
        scale.poll(15.0, "the connection ended and its ssh gone", || {
            let ended = logged(&self.log, &unused) > 0 || logged(&self.log, &failed) > at_close;
            Ok((ended && ssh_processes(brindle).is_empty()).then_some(()))
        })?;
        let ended = closed.elapsed();
        let attempts = logged(&self.log, &failed);
        // A refused `ssh` lives for milliseconds, which sampling can miss,
        // but each attempt logs its failure.
        let mut started: Vec<u32> = Vec::new();
        scale.observe_for(10.0, || {
            for pid in ssh_processes(brindle) {
                if !started.contains(&pid) {
                    started.push(pid);
                }
            }
        });
        let later = logged(&self.log, &failed) - attempts;
        let text = format!(
            "connection ended {:.1}s after the close\nfailed attempts {attempts} before, {later} after\n\
             ssh after {started:?}\n",
            ended.as_secs_f64()
        );
        fs::write(&self.save, text).map_err(|e| e.to_string())?;
        ensure!(
            started.is_empty(),
            "Brindle started ssh {started:?} after the connection ended"
        );
        ensure!(
            later == 0,
            "Brindle dialled {} {later} more times after the connection ended",
            self.endpoint
        );
        let local = scale.poll(10.0, "the restored local tab", || {
            Ok(local_sessions(&self.helper, &self.server_dir)?
                .into_iter()
                .find(|s| s.attached))
        })?;
        mark_tty(local.pid, self.checked)
    }
}

/// Profiles that connect at launch to endpoints that can't be reached: a
/// stopped container and an SSH port that refuses. "Attach Session…" lists
/// each as offline, and the container stays stopped.
pub fn offline(ctx: &Ctx) -> Result<Outcome> {
    let needs = match Needs::check(ctx)? {
        Ok(needs) => needs,
        Err(skip) => return Ok(skip),
    };
    let container = Container::start(ctx, &needs.image, &[])?;
    container.stop()?;
    let port = refused_port()?;
    let config = format!(
        "\n[[profiles]]\n\
         name = {OFFLINE_CONTAINER_PROFILE:?}\n\
         remote = {{ type = \"container\", engine = \"docker\", container = {name:?}, user = \"remote\", connect_at_launch = true }}\n\
         \n[[profiles]]\n\
         name = {OFFLINE_SSH_PROFILE:?}\n\
         remote = {{ type = \"ssh\", host = \"127.0.0.1\", user = \"root\", port = {port}, ssh_args = {SSH_ARGS}, connect_at_launch = true }}\n",
        name = container.name,
    );
    let out = needs
        .run(ctx, "run", &config)
        .action("attach_session")
        .dump_at(6.0)
        .run()?;
    let palette = &out.dump.palette;
    // Each row's reason names its cause (the row shows its first line).
    let mut offline_rows = Vec::new();
    for (endpoint, cause) in [
        (format!("remote@{}", container.name), "is not running"),
        (format!("root@127.0.0.1:{port}"), "Connection refused"),
    ] {
        let offline = format!("{endpoint}  offline: ");
        let at = palette.iter().position(|row| row.starts_with(&offline));
        let Some(at) = at else {
            return Err(format!("no offline row for {endpoint}: {palette:?}"));
        };
        let row = &palette[at];
        ensure!(row.contains(cause), "{endpoint}'s row doesn't say {cause:?}: {row:?}");
        offline_rows.push(at);
    }
    // Nothing is detached anywhere, and every endpoint has answered: the
    // palette says so, after both endpoints' rows.
    let note = palette.iter().position(|row| row == "No detached sessions");
    let Some(note) = note else {
        return Err(format!("no \"No detached sessions\" row: {palette:?}"));
    };
    ensure!(
        offline_rows.iter().all(|&at| at < note),
        "the note (row {note}) doesn't come after both offline rows {offline_rows:?}: {palette:?}"
    );
    ensure!(
        !container.running()?,
        "the stopped container {} is running",
        container.name
    );
    Ok(Outcome::Pass)
}

/// The sessions palette opens without waiting for a host that is slow to
/// answer. A first run leaves a local shell detached. The second connects at
/// launch to the SSH fixture, which is paused (`docker pause`) once Brindle
/// is connected to it: the host's link stays up but goes silent. "Attach
/// Session…" then opens within the open grace, with this machine's session
/// and the host's `listing sessions…` row, long before the listing gives up
/// (`LIST_LIMIT`, 2 s).
///
/// Two of Brindle's own limits, which `BRINDLE_E2E_SLOW` doesn't stretch,
/// bound the timeline. The paused link is given up once ssh's keepalives go
/// unanswered: Brindle runs ssh with `ServerAliveInterval=5` and
/// `ServerAliveCountMax=3` (`client.rs`), about 15 s, sooner than its own
/// `Timings::dead` (30 s), so the palette's step must come less than that
/// after the pause. The listing gives up after `LIST_LIMIT` (2 s), so the
/// dump must come less than that after the palette's step. The pause is
/// made by a timeline call once the relay runs in the container and the
/// step before the palette's has run (it creates a file), so the palette's
/// step follows it by one step interval; the dump follows that step by
/// 0.7 s, unstretched.
pub fn palette_slow_host(ctx: &Ctx) -> Result<Outcome> {
    // The run's steps: the modality, the one the pause waits for, then the
    // palette's, and the dump just after it.
    const INTERVAL: f64 = 3.0;
    // Under ssh's keepalive limit (about 15 s), with room for the pause.
    if ctx.secs(INTERVAL) >= 12.0 {
        return Ok(Outcome::Skip("BRINDLE_E2E_SLOW stretches the palette step past ssh's 15 s keepalive limit".into()));
    }
    let needs = match Needs::check(ctx)? {
        Ok(needs) => needs,
        Err(skip) => return Ok(skip),
    };
    let ssh = Ssh::start(ctx, &needs.image, Net::Published)?;
    needs
        .run(ctx, "first", "")
        .keep_detached()
        .send("echo left-$((1+1))\r")
        .wait_for("left-2")
        .dump_at(5.0)
        .run()?;
    let left = ctx.scale().poll(10.0, "the first run's shell detached", || {
        let sessions = local_sessions(&ctx.helper, &ctx.server_dir)?;
        Ok(match sessions.as_slice() {
            [session] if !session.attached => Some(session.clone()),
            _ => None,
        })
    })?;

    let docker = ssh.container.docker.clone();
    let scale = ctx.scale();
    let stepped = ctx.dir.join("stepped");
    let seen = stepped.clone();
    let out = needs
        .run(ctx, "second", &ssh.profile(SSH_PROFILE, "ssh-slow-host", true))
        .step_interval(INTERVAL)
        .modality("keyboard")
        .send(&format!("touch '{}'\r", stepped.display()))
        .action("attach_session")
        // Once Brindle's relay runs in the container (the endpoint is
        // connected) and the step before the palette's has run, the host
        // goes silent.
        .call_at(1.0, move |_| {
            scale.poll(10.0, "Brindle's relay in the container", || {
                Ok((!docker.relays()?.is_empty()).then_some(()))
            })?;
            scale.poll(30.0, "the step before the palette's", || Ok(seen.exists().then_some(())))?;
            docker.pause()
        })
        // Within `LIST_LIMIT` (2 s) of the palette's step, unstretched.
        .dump_after_last_step(0.7)
        .run();
    // Back in order, so the container's logs and removal work.
    ssh.container.unpause().ok();
    let out = out?;

    let palette = &out.dump.palette;
    let endpoint = format!("root@{}:{}", ssh.host, ssh.port);
    let pending = format!("{endpoint}  listing sessions…");
    ensure!(
        palette.len() == 3 && palette[2] == pending,
        "want this machine's session, then {pending:?}, not listed yet or offline: {palette:?}"
    );
    ensure!(
        !palette[1].starts_with(&endpoint) && !palette[1].starts_with("No detached"),
        "the first row isn't this machine's session {left:?}: {palette:?}"
    );
    ensure!(
        out.dump.palette_highlighted()? == Some(0),
        "this machine's session isn't highlighted: {palette:?}"
    );
    Ok(Outcome::Pass)
}

/// What a remote case needs from the machine: a Docker daemon, the Linux
/// helper Brindle deploys, and the fixture image.
struct Needs {
    agent: Agent,
    image: String,
}

impl Needs {
    /// The case's needs, or (`Err`) its outcome when one is missing.
    fn check(ctx: &Ctx) -> Result<std::result::Result<Self, Outcome>> {
        if let Some(outcome) = docker_outcome()? {
            return Ok(Err(outcome));
        }
        let Some(agent) = remote_agent(ctx)? else {
            return Ok(Err(Outcome::Skip(
                "Linux remote helper was not built; run `cargo build -p brindle-remote-agent`"
                    .into(),
            )));
        };
        let image = fixture_image(ctx)?;
        Ok(Ok(Self { agent, image }))
    }

    /// A run of Brindle with `config` (profiles) after the test config, that
    /// deploys the helper.
    fn run<'a>(&self, ctx: &'a Ctx, label: &'static str, config: &str) -> Run<'a> {
        self.agent.give(Run::new(ctx, label).config(config))
    }
}

/// How the SSH fixture is reached.
enum Net {
    /// Through a port published on 127.0.0.1 (or `BRINDLE_REMOTE_E2E_SSH_HOST`).
    Published,
    /// At the container's own address on a network of the case's, which a
    /// case can cut and restore.
    Private,
}

/// An SSH server in a disposable container, taking a generated key for
/// root. Its fields drop in order: the container, then its network.
struct Ssh {
    container: Container,
    network: Option<Network>,
    key: KeyPair,
    host: String,
    port: u16,
}

impl Ssh {
    fn start(ctx: &Ctx, image: &str, net: Net) -> Result<Self> {
        let key = KeyPair::generate(ctx)?;
        let network = match net {
            Net::Published => None,
            Net::Private => Some(Network::create(ctx)?),
        };
        let (container, host) = match &network {
            None => (
                Container::start(ctx, image, &["--publish", "127.0.0.1::22"])?,
                std::env::var("BRINDLE_REMOTE_E2E_SSH_HOST").unwrap_or_else(|_| "127.0.0.1".into()),
            ),
            Some(network) => {
                let address = network.address(2);
                let args = ["--network", &network.name, "--ip", &address];
                (Container::start(ctx, image, &args)?, address)
            }
        };
        container.copy_to(&key.public, "/root/.ssh/authorized_keys")?;
        // `docker cp` can keep the host file's owner, and sshd's StrictModes
        // refuses keys that root doesn't own.
        container.exec(&["chown", "root:root", "/root/.ssh/authorized_keys"])?;
        container.exec(&["chmod", "700", "/root/.ssh"])?;
        container.exec(&["chmod", "600", "/root/.ssh/authorized_keys"])?;
        container.exec(&["/usr/sbin/sshd", "-t"])?;
        container.exec_detached(&["/usr/sbin/sshd", "-D", "-e"])?;
        let port = match network {
            None => container.ssh_port()?,
            Some(_) => 22,
        };
        wait_for_ssh(ctx, &key.private, &host, port)?;
        Ok(Self {
            container,
            network,
            key,
            host,
            port,
        })
    }

    /// A profile named `name` for root's interactive bash on the fixture,
    /// with the prompt [`SSH_PROMPT`] and `$BRINDLE_E2E_MARKER` set to
    /// `marker`, as TOML to append to the test config.
    fn profile(&self, name: &str, marker: &str, connect_at_launch: bool) -> String {
        let eager = if connect_at_launch {
            ", connect_at_launch = true"
        } else {
            ""
        };
        format!(
            "\n[[profiles]]\n\
             name = {name:?}\n\
             command = \"/bin/bash\"\n\
             args = [\"--norc\", \"--noprofile\", \"-i\"]\n\
             env = {{ SHELL = \"/bin/bash\", PS1 = \"{SSH_PROMPT} \", BRINDLE_E2E_MARKER = {marker:?} }}\n\
             remote = {{ type = \"ssh\", host = {host:?}, user = \"root\", port = {port}, identity_file = {identity:?}, ssh_args = {SSH_ARGS}{eager} }}\n",
            host = self.host,
            port = self.port,
            identity = self.key.private.to_string_lossy(),
        )
    }

    /// An agent profile named `name` running `command` with the fake
    /// agent's `script` as root on the fixture, as TOML to append to the
    /// test config.
    fn agent_profile(&self, name: &str, command: &str, script: &str) -> String {
        format!(
            "\n[[profiles]]\n\
             name = {name:?}\n\
             type = \"agent\"\n\
             command = {command:?}\n\
             args = [{script:?}]\n\
             remote = {{ type = \"ssh\", host = {host:?}, user = \"root\", port = {port}, identity_file = {identity:?}, ssh_args = {SSH_ARGS} }}\n",
            host = self.host,
            port = self.port,
            identity = self.key.private.to_string_lossy(),
        )
    }
}

/// A Docker network of the case's, with a subnet it chose, so a container
/// disconnected from it can be connected again at the same address. Removed
/// when dropped, after the containers on it.
struct Network {
    name: String,
    owner: String,
    /// The subnet's network address.
    base: Ipv4Addr,
}

impl Network {
    fn create(ctx: &Ctx) -> Result<Self> {
        let owner = unique_owner();
        let labels = [
            format!("org.brindle.e2e.case={}", ctx.name),
            format!("org.brindle.e2e.owner={owner}"),
        ];
        let create = |name: &str, subnet: Option<&str>| {
            let mut args = vec!["network", "create"];
            if let Some(subnet) = subnet {
                args.extend(["--subnet", subnet]);
            }
            for label in &labels {
                args.extend(["--label", label]);
            }
            args.push(name);
            docker_ok(&args)
        };
        // Only a network made with its subnet given lets `--ip` pin an
        // address, so a first network finds a subnet Docker counts as free,
        // and the case's own is made with it.
        let probe = format!("brindle-e2e-probe-{owner}");
        let subnets = create(&probe, None).and_then(|_| {
            docker_ok(&[
                "network",
                "inspect",
                "--format",
                "{{range .IPAM.Config}}{{.Subnet}} {{end}}",
                &probe,
            ])
        });
        remove_network(&probe, &owner);
        let subnets = subnets?;
        let (base, prefix) = subnets
            .split_whitespace()
            .find_map(parse_subnet)
            .ok_or_else(|| format!("Docker gave no IPv4 subnet: {subnets:?}"))?;
        let name = format!("brindle-e2e-net-{owner}");
        let made = create(&name, Some(&format!("{base}/{prefix}")));
        // Removed by its drop if it was made after all.
        let network = Self { name, owner, base };
        made.map(|_| network)
    }

    /// The subnet's `n`th address (the gateway is the first).
    fn address(&self, n: u32) -> String {
        Ipv4Addr::from(u32::from(self.base) + n).to_string()
    }
}

impl Drop for Network {
    fn drop(&mut self) {
        remove_network(&self.name, &self.owner);
    }
}

/// `a.b.c.d/n` as its network address and prefix, for a subnet with room
/// for a gateway and a container.
fn parse_subnet(text: &str) -> Option<(Ipv4Addr, u8)> {
    let (address, prefix) = text.split_once('/')?;
    let (address, prefix): (Ipv4Addr, u8) = (address.parse().ok()?, prefix.parse().ok()?);
    let mask = u32::MAX.checked_shl(32 - u32::from(prefix)).unwrap_or(0);
    (8..=29)
        .contains(&prefix)
        .then(|| (Ipv4Addr::from(u32::from(address) & mask), prefix))
}

/// Removes the network `name` if it carries the case's `owner` label.
fn remove_network(name: &str, owner: &str) {
    let inspect = Command::new("docker")
        .args([
            "network",
            "inspect",
            "--format",
            "{{ index .Labels \"org.brindle.e2e.owner\" }}",
            name,
        ])
        .output();
    let ours = inspect.is_ok_and(|output| {
        output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == owner
    });
    if ours {
        Command::new("docker")
            .args(["network", "rm", name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .ok();
    }
}

/// Runs `docker ARGS`, giving its stdout, or its message when it fails.
fn docker_ok(args: &[&str]) -> Result<String> {
    let output = Command::new("docker")
        .args(args)
        .output()
        .map_err(|error| format!("docker {}: {error}", args.join(" ")))?;
    ensure!(
        output.status.success(),
        "docker {} failed: {}",
        args.join(" "),
        output_message(&output)
    );
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// A unique tag for the containers and networks this run makes.
fn unique_owner() -> String {
    let sequence = CONTAINER_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!("{}-{nonce}-{sequence}", std::process::id())
}

/// One line of `brindle-remote list`.
#[derive(Clone, Debug, PartialEq)]
struct Listed {
    id: u32,
    pid: u32,
    attached: bool,
    /// Detached by a client that left without detaching it, so it may come
    /// back for it.
    owned: bool,
}

/// `id<TAB>pid<TAB>attached|detached<TAB>owned|unowned|-<TAB>…` lines.
fn parse_list(text: &str) -> Vec<Listed> {
    text.lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            Some(Listed {
                id: fields.first()?.parse().ok()?,
                pid: fields.get(1)?.parse().ok()?,
                attached: *fields.get(2)? == "attached",
                owned: *fields.get(3)? == "owned",
            })
        })
        .collect()
}

/// The case's local server's sessions.
fn local_sessions(helper: &Path, server_dir: &Path) -> Result<Vec<Listed>> {
    let output = Command::new(helper)
        .arg("list")
        .env("BRINDLE_SERVER_DIR", server_dir)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("running {}: {error}", helper.display()))?;
    ensure!(
        output.status.success(),
        "brindle-remote list: {}",
        output_message(&output)
    );
    Ok(parse_list(&String::from_utf8_lossy(&output.stdout)))
}

/// `pid argv…` lines as [`PROCESSES`] prints them.
fn parse_processes(text: &str) -> Vec<(u32, Vec<String>)> {
    text.lines()
        .filter_map(|line| {
            let (pid, args) = line.split_once('\t')?;
            let args = args
                .split('\u{1f}')
                .filter(|arg| !arg.is_empty())
                .map(str::to_string);
            Some((pid.parse().ok()?, args.collect()))
        })
        .collect()
}

/// `brindle-remote relay`, as a launch runs it: the cached helper by path.
fn is_relay(argv: &[String]) -> bool {
    argv.first()
        .is_some_and(|program| program.ends_with("/brindle-remote"))
        && argv.get(1).is_some_and(|role| role == "relay")
}

/// A local process's command name and parent, from `/proc/<pid>/stat`;
/// `None` once it is gone or a zombie.
fn stat_text(text: &str) -> Option<(String, u32)> {
    let (head, rest) = text.rsplit_once(") ")?;
    let comm = head.split_once(" (")?.1;
    let mut fields = rest.split(' ');
    let state = fields.next()?;
    let parent = fields.next()?.parse().ok()?;
    (state != "Z").then(|| (comm.to_string(), parent))
}

/// Whether this machine's process `pid` runs (not a zombie).
fn alive(pid: u32) -> bool {
    fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|text| stat_text(&text))
        .is_some()
}

/// Brindle's `ssh` processes: its relay links, and an upload while one runs.
fn ssh_processes(brindle: u32) -> Vec<u32> {
    let Ok(entries) = fs::read_dir("/proc") else {
        return Vec::new();
    };
    let mut pids: Vec<u32> = entries
        .flatten()
        .filter_map(|entry| {
            let pid = entry.file_name().to_str()?.parse().ok()?;
            let text = fs::read_to_string(entry.path().join("stat")).ok()?;
            let (comm, parent) = stat_text(&text)?;
            (comm == "ssh" && parent == brindle).then_some(pid)
        })
        .collect();
    pids.sort();
    pids
}

/// The first line of the log at `path` that contains `text`.
fn log_line(path: &Path, text: &str) -> Option<String> {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .find(|line| line.contains(text))
        .map(str::to_string)
}

/// How many lines of the log at `path` contain `text`.
fn logged(path: &Path, text: &str) -> usize {
    fs::read_to_string(path)
        .unwrap_or_default()
        .lines()
        .filter(|line| line.contains(text))
        .count()
}

/// Writes `text` on a line of its own to the terminal of this machine's
/// process `pid`, which shows it: a marker for a [`Run::wait_for`] step.
fn mark_tty(pid: u32, text: &str) -> Result {
    let mut tty = fs::OpenOptions::new()
        .write(true)
        .open(format!("/proc/{pid}/fd/1"))
        .map_err(|e| format!("opening {pid}'s terminal: {e}"))?;
    tty.write_all(format!("\r\n{text}\r\n").as_bytes())
        .map_err(|e| format!("writing to {pid}'s terminal: {e}"))
}

/// A port on 127.0.0.1 that nothing listens on (it was free a moment ago).
fn refused_port() -> Result<u16> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|error| format!("finding a free port: {error}"))?;
    listener
        .local_addr()
        .map(|address| address.port())
        .map_err(|error| format!("finding a free port: {error}"))
}


/// The number after `prefix` on a line of the dumped screen.
fn shown_pid(dump: &Dump, prefix: &str) -> Result<u32> {
    dump.screen
        .iter()
        .find_map(|line| line.strip_prefix(prefix)?.trim().parse().ok())
        .ok_or_else(|| format!("no {prefix:?} line: {:?}", dump.screen))
}

/// Every `--wait-for` of the run found its text before the dump.
fn no_waits_left(dump: &Dump) -> Result {
    ensure!(
        dump.wait_timeouts.is_empty() && dump.wait_pending.is_empty(),
        "waits that timed out {:?}, still pending {:?}; screen: {:?}",
        dump.wait_timeouts,
        dump.wait_pending,
        dump.screen
    );
    Ok(())
}

/// The harness's own tests of this file's parsers (`--self-test`).
pub fn self_test() -> Result {
    ensure!(stty_size("after=24 80", "after=") == Some((80, 24)), "stty size");
    ensure!(
        stty_size("after=24", "after=").is_none() && stty_size("echo \"aft\"\"er=", "after=").is_none(),
        "stty size junk"
    );
    let listed = parse_list(
        "3\t4242\tattached\t-\t-\tdetach\t1m05s\tbash\tbash -i\t/root\n\
         4\t77\tdetached\towned\t30s\tdetach\t2m\tsleep\tsleep 9\t/\n\
         5\t78\tdetached\tunowned\t1s\tkill\t2m\t\t\t\n\
         no server\n",
    );
    let expected = [
        Listed {
            id: 3,
            pid: 4242,
            attached: true,
            owned: false,
        },
        Listed {
            id: 4,
            pid: 77,
            attached: false,
            owned: true,
        },
        Listed {
            id: 5,
            pid: 78,
            attached: false,
            owned: false,
        },
    ];
    ensure!(listed == expected, "list lines: {listed:?}");
    let processes = parse_processes(
        "1\tsleep\x1finfinity\x1f\n\
         40\t/root/.cache/brindle/0.9.0-ab/brindle-remote\x1frelay\x1f\n\
         41\t/proc/39/exe\x1fserve\x1f--ready-fd\x1f3\x1f\n\
         42\t\n\
         x\tsh\n",
    );
    let relays: Vec<u32> = processes
        .iter()
        .filter(|(_, argv)| is_relay(argv))
        .map(|(pid, _)| *pid)
        .collect();
    ensure!(
        processes.len() == 4 && relays == [40],
        "processes {processes:?}, relays {relays:?}"
    );
    ensure!(
        stat_text("123 (ssh) S 99 123 123 0 -1") == Some(("ssh".into(), 99)),
        "a running process's stat"
    );
    ensure!(
        stat_text("124 (a) b) (c) S 1 2") == Some(("a) b) (c".into(), 1)),
        "a command name with parentheses"
    );
    ensure!(
        stat_text("125 (ssh) Z 99 0").is_none(),
        "a zombie counts as gone"
    );
    ensure!(
        parse_subnet("172.19.0.0/16") == Some((Ipv4Addr::new(172, 19, 0, 0), 16)),
        "a /16"
    );
    ensure!(
        parse_subnet("192.168.32.5/20") == Some((Ipv4Addr::new(192, 168, 32, 0), 20)),
        "an address inside a /20 gives the network's"
    );
    for bad in ["fd00::/64", "10.0.0.0/30", "10.0.0.0", "10.0.0.0/x"] {
        ensure!(parse_subnet(bad).is_none(), "{bad:?} accepted");
    }
    Ok(())
}

fn docker_outcome() -> Result<Option<Outcome>> {
    let output = Command::new("docker")
        .args(["version", "--format", "{{.Server.Version}}"])
        .output();
    let unavailable = match output {
        Ok(output) if output.status.success() => return Ok(None),
        Ok(output) => format!("docker daemon is unavailable: {}", output_message(&output)),
        Err(error) => format!("docker is unavailable: {error}"),
    };
    if std::env::var("BRINDLE_REMOTE_E2E_REQUIRED").as_deref() == Ok("1") {
        Err(format!("{unavailable} (BRINDLE_REMOTE_E2E_REQUIRED=1)"))
    } else {
        Ok(Some(Outcome::Skip(unavailable)))
    }
}

/// The Linux helper a remote case's Brindle deploys ([`remote_agent`]).
enum Agent {
    /// `BRINDLE_REMOTE_AGENT` named it: passed on resolved, since a run's
    /// working directory may not be the suite's.
    Named(PathBuf),
    /// Beside the binary under test, where Brindle finds it itself, so the
    /// run names none: an installed or unpacked archive's helper is
    /// deployed as a user's Brindle finds it.
    Beside,
}

impl Agent {
    /// `run`, told about the helper when Brindle wouldn't find it itself.
    fn give<'a>(&self, run: Run<'a>) -> Run<'a> {
        match self {
            Agent::Named(path) => run.env("BRINDLE_REMOTE_AGENT", &path.to_string_lossy()),
            Agent::Beside => run,
        }
    }
}

/// The helper Brindle deploys: `BRINDLE_REMOTE_AGENT`, else the first of
/// its names for the Linux helper beside the binary under test (an
/// archive's `brindle-remote-linux-x86_64`, then cargo's `brindle-remote`),
/// as Brindle looks for it. `None` when there is none, which
/// `BRINDLE_REMOTE_E2E_REQUIRED=1` makes an error.
fn remote_agent(ctx: &Ctx) -> Result<Option<Agent>> {
    use crate::common::server::{LINUX_HELPER_NAMES, beside, bin_paths};
    let named = std::env::var_os("BRINDLE_REMOTE_AGENT").filter(|value| !value.is_empty()).map(PathBuf::from);
    let found = match &named {
        Some(path) => path.is_file().then(|| path.clone()),
        None => beside(&bin_paths(&ctx.bin), &LINUX_HELPER_NAMES, &|path: &Path| path.is_file()),
    };
    let Some(path) = found else {
        if std::env::var("BRINDLE_REMOTE_E2E_REQUIRED").as_deref() == Ok("1") {
            let looked = named.map_or_else(
                || format!("beside {} ({})", ctx.bin.display(), LINUX_HELPER_NAMES.join(", ")),
                |path| format!("at {}", path.display()),
            );
            return Err(format!("Linux remote helper not found {looked}; run `cargo build -p brindle-remote-agent`"));
        }
        return Ok(None);
    };
    if named.is_none() {
        return Ok(Some(Agent::Beside));
    }
    fs::canonicalize(&path)
        .map(|path| Some(Agent::Named(path)))
        .map_err(|error| format!("resolving remote helper {}: {error}", path.display()))
}

fn fixture_image(ctx: &Ctx) -> Result<String> {
    FIXTURE_IMAGE
        .get_or_init(|| build_fixture_image(ctx))
        .clone()
}

fn build_fixture_image(ctx: &Ctx) -> Result<String> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/remote");
    let mut hasher = DefaultHasher::new();
    for name in ["Dockerfile", "sshd_config"] {
        name.hash(&mut hasher);
        let path = root.join(name);
        fs::read(&path)
            .map_err(|error| format!("reading remote fixture {}: {error}", path.display()))?
            .hash(&mut hasher);
    }
    let digest = format!("{:016x}", hasher.finish());
    let image = format!("brindle-e2e/remote-fixture:{digest}");
    let label = format!("org.brindle.e2e.fixture={digest}");
    let inspect = Command::new("docker")
        .args([
            "image",
            "inspect",
            "--format",
            "{{ index .Config.Labels \"org.brindle.e2e.fixture\" }}",
            &image,
        ])
        .output()
        .map_err(|error| format!("inspecting Docker fixture image: {error}"))?;
    if inspect.status.success() && String::from_utf8_lossy(&inspect.stdout).trim() == digest {
        return Ok(image);
    }

    let output = Command::new("docker")
        .args(["build", "--label", &label, "--tag", &image])
        .arg(&root)
        .output()
        .map_err(|error| format!("building Docker fixture image: {error}"))?;
    let mut log = output.stdout.clone();
    log.extend_from_slice(&output.stderr);
    ctx.save("docker-build.log", log);
    ensure!(
        output.status.success(),
        "building Docker fixture image failed: {}",
        output_message(&output)
    );
    Ok(image)
}

/// A disposable container of the case's, removed when dropped. Its
/// [`Docker`] handle runs commands in it.
struct Container {
    docker: Docker,
    owner: String,
    /// Where the servers' logs in it are kept when it is removed.
    logs: PathBuf,
}

/// A container to run commands in, by name: cheap to clone into a
/// [`Run::call_at`] call.
#[derive(Clone)]
struct Docker {
    name: String,
}

impl std::ops::Deref for Container {
    type Target = Docker;

    fn deref(&self) -> &Docker {
        &self.docker
    }
}

impl Container {
    /// Runs the fixture image's `sleep infinity`, with `args` for `docker run`.
    fn start(ctx: &Ctx, image: &str, args: &[&str]) -> Result<Self> {
        let owner = unique_owner();
        let name = format!("brindle-e2e-remote-{owner}");
        let mut command = Command::new("docker");
        command
            .args(["run", "--detach", "--name"])
            .arg(&name)
            .args(["--label", &format!("org.brindle.e2e.case={}", ctx.name)])
            .args(["--label", &format!("org.brindle.e2e.owner={owner}")])
            .args(args);
        command.args([image, "sleep", "infinity"]);
        let output = match command.output() {
            Ok(output) => output,
            Err(error) => {
                remove_container(&name, &owner);
                return Err(format!("starting Docker fixture container: {error}"));
            }
        };
        if !output.status.success() {
            remove_container(&name, &owner);
            return Err(format!(
                "starting Docker fixture container failed: {}",
                output_message(&output)
            ));
        }
        let logs = ctx.dir.join(format!("{name}-server.log"));
        let container = Self {
            docker: Docker { name },
            owner,
            logs,
        };
        container.wait_running(ctx)?;
        Ok(container)
    }
}

impl Docker {
    fn wait_running(&self, ctx: &Ctx) -> Result {
        ctx.scale().poll(10.0, "the Docker fixture container running", || {
            let output = Command::new("docker")
                .args(["inspect", "--format", "{{.State.Running}}", &self.name])
                .output()
                .map_err(|error| format!("probing Docker fixture container: {error}"))?;
            if output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "true" {
                Ok(Some(()))
            } else {
                Err(output_message(&output))
            }
        })
    }

    fn exec_output(&self, args: &[&str]) -> Result<Output> {
        Command::new("docker")
            .arg("exec")
            .arg(&self.name)
            .args(args)
            .output()
            .map_err(|error| format!("docker exec {}: {error}", args.join(" ")))
    }

    fn exec(&self, args: &[&str]) -> Result<String> {
        let output = self.exec_output(args)?;
        ensure!(
            output.status.success(),
            "docker exec {} failed: {}",
            args.join(" "),
            output_message(&output)
        );
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    }

    fn exec_detached(&self, args: &[&str]) -> Result {
        let output = Command::new("docker")
            .args(["exec", "--detach", &self.name])
            .args(args)
            .output()
            .map_err(|error| {
                format!("starting detached docker exec {}: {error}", args.join(" "))
            })?;
        ensure!(
            output.status.success(),
            "detached docker exec {} failed: {}",
            args.join(" "),
            output_message(&output)
        );
        Ok(())
    }

    fn copy_to(&self, source: &Path, destination: &str) -> Result {
        let target = format!("{}:{destination}", self.name);
        let output = Command::new("docker")
            .arg("cp")
            .arg(source)
            .arg(&target)
            .output()
            .map_err(|error| {
                format!("copying {} into Docker fixture: {error}", source.display())
            })?;
        ensure!(
            output.status.success(),
            "docker cp failed: {}",
            output_message(&output)
        );
        Ok(())
    }

    fn assert_absent(&self, path: &str) -> Result {
        self.exec(&["test", "!", "-e", path]).map(|_| ())
    }

    fn assert_cached_agent(&self, cache_dir: &str) -> Result {
        let path = self.exec(&[
            "sh",
            "-lc",
            &format!("find {cache_dir} -type f -name brindle-remote -perm /111 -print -quit"),
        ])?;
        let path = path.trim();
        ensure!(!path.is_empty(), "no cached SSH helper under {cache_dir}");
        let version = self.exec(&[path, "--version"])?;
        ensure!(
            version.trim() == env!("CARGO_PKG_VERSION"),
            "deployed helper version {:?}, expected {}",
            version.trim(),
            env!("CARGO_PKG_VERSION")
        );
        Ok(())
    }

    fn ssh_port(&self) -> Result<u16> {
        let output = Command::new("docker")
            .args(["port", &self.name, "22/tcp"])
            .output()
            .map_err(|error| format!("reading Docker SSH port: {error}"))?;
        ensure!(
            output.status.success(),
            "docker port failed: {}",
            output_message(&output)
        );
        let address = String::from_utf8_lossy(&output.stdout);
        address
            .lines()
            .find_map(|line| line.trim().rsplit_once(':')?.1.parse().ok())
            .ok_or_else(|| format!("docker port returned no TCP port: {address:?}"))
    }

    /// Runs root's cached helper: `brindle-remote ARGS` against root's server.
    fn helper(&self, args: &[&str]) -> Result<String> {
        let mut command = vec!["sh", "-c", ROOT_HELPER, "sh"];
        command.extend_from_slice(args);
        self.exec(&command)
    }

    /// The sessions of root's server (none while no core runs).
    fn sessions(&self) -> Result<Vec<Listed>> {
        Ok(parse_list(&self.helper(&["list"])?))
    }

    /// The container's processes, with their arguments.
    fn processes(&self) -> Result<Vec<(u32, Vec<String>)>> {
        Ok(parse_processes(&self.exec(&["sh", "-c", PROCESSES])?))
    }

    /// The pids of the relays running in the container.
    fn relays(&self) -> Result<Vec<u32>> {
        Ok(self
            .processes()?
            .into_iter()
            .filter(|(_, argv)| is_relay(argv))
            .map(|(pid, _)| pid)
            .collect())
    }

    /// Process `pid`'s command name and parent in the container; `None` once
    /// it is gone.
    fn stat(&self, pid: u32) -> Option<(String, u32)> {
        let output = self
            .exec_output(&["cat", &format!("/proc/{pid}/stat")])
            .ok()?;
        output
            .status
            .success()
            .then(|| stat_text(&String::from_utf8_lossy(&output.stdout)))
            .flatten()
    }

    /// Process `pid` in the container runs `comm`, a child of `parent`.
    fn runs_in(&self, pid: u32, comm: &str, parent: u32) -> Result {
        let found = self.stat(pid);
        ensure!(
            found == Some((comm.to_string(), parent)),
            "process {pid} in the container is {found:?}, not {comm} under {parent}"
        );
        Ok(())
    }

    /// Writes `text` on a line of its own to the terminal of process `pid`
    /// in the container, which shows it: a marker for a [`Run::wait_for`].
    fn mark(&self, pid: u32, text: &str) -> Result {
        let script = r#"printf '\r\n%s\r\n' "$1" > "/proc/$2/fd/1""#;
        self.exec(&["sh", "-c", script, "sh", text, &pid.to_string()])
            .map(drop)
    }

    fn running(&self) -> Result<bool> {
        let state = docker_ok(&["inspect", "--format", "{{.State.Running}}", &self.name])?;
        Ok(state.trim() == "true")
    }

    /// Freezes its processes (`docker pause`): connections to it stay open,
    /// and nothing answers on them.
    fn pause(&self) -> Result {
        docker_ok(&["pause", &self.name]).map(drop)
    }

    fn unpause(&self) -> Result {
        docker_ok(&["unpause", &self.name]).map(drop)
    }

    /// Stops it at once (its `sleep` ignores SIGTERM).
    fn stop(&self) -> Result {
        docker_ok(&["stop", "--time", "0", &self.name]).map(drop)
    }

    /// Cuts it off `network`, which it can't then be reached on, without
    /// closing its connections.
    fn disconnect(&self, network: &str) -> Result {
        docker_ok(&["network", "disconnect", network, &self.name]).map(drop)
    }

    /// Connects it to `network` again, at `address`.
    fn connect(&self, network: &str, address: &str) -> Result {
        docker_ok(&["network", "connect", "--ip", address, network, &self.name]).map(drop)
    }
}

impl Drop for Container {
    fn drop(&mut self) {
        // The servers' logs in it, for a failed case.
        if let Ok(output) = self.exec_output(&["sh", "-c", SERVER_LOGS])
            && !output.stdout.is_empty()
        {
            fs::write(&self.logs, &output.stdout).ok();
        }
        remove_container(&self.name, &self.owner);
    }
}

fn remove_container(name: &str, owner: &str) {
    let inspect = Command::new("docker")
        .args([
            "inspect",
            "--format",
            "{{ index .Config.Labels \"org.brindle.e2e.owner\" }}",
            name,
        ])
        .output();
    let ours = inspect.is_ok_and(|output| {
        output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == owner
    });
    if ours {
        Command::new("docker")
            .args(["rm", "--force", name])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .ok();
    }
}

struct KeyPair {
    private: PathBuf,
    public: PathBuf,
}

impl KeyPair {
    fn generate(ctx: &Ctx) -> Result<Self> {
        let private = ctx.dir.join("ssh-key");
        let public = ctx.dir.join("ssh-key.pub");
        fs::remove_file(&private).ok();
        fs::remove_file(&public).ok();
        let output = Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-f"])
            .arg(&private)
            .output()
            .map_err(|error| format!("generating temporary SSH key: {error}"))?;
        if !output.status.success() {
            fs::remove_file(&private).ok();
            fs::remove_file(&public).ok();
            return Err(format!("ssh-keygen failed: {}", output_message(&output)));
        }
        Ok(Self { private, public })
    }
}

impl Drop for KeyPair {
    fn drop(&mut self) {
        fs::remove_file(&self.private).ok();
        fs::remove_file(&self.public).ok();
    }
}

fn wait_for_ssh(ctx: &Ctx, identity: &Path, host: &str, port: u16) -> Result {
    ctx.scale().poll(15.0, "the fixture SSH server ready", || {
        let output = ssh_command(identity, port)
            .arg(format!("root@{host}"))
            .arg("printf BRINDLE_SSH_READY")
            .output()
            .map_err(|error| format!("probing fixture SSH server: {error}"))?;
        if output.status.success() && String::from_utf8_lossy(&output.stdout) == "BRINDLE_SSH_READY" {
            Ok(Some(()))
        } else {
            Err(output_message(&output))
        }
    })
}

fn ssh_command(identity: &Path, port: u16) -> Command {
    let mut command = Command::new("ssh");
    command
        .args(["-F", "/dev/null", "-p", &port.to_string(), "-i"])
        .arg(identity)
        .args([
            "-o",
            "BatchMode=yes",
            "-o",
            "IdentitiesOnly=yes",
            "-o",
            "StrictHostKeyChecking=no",
            "-o",
            "UserKnownHostsFile=/dev/null",
            "-o",
            "GlobalKnownHostsFile=/dev/null",
            "-o",
            "LogLevel=ERROR",
            "-o",
            "ConnectTimeout=2",
        ]);
    command
}

/// Where a run labelled `label` logs its notifications
/// (`BRINDLE_NOTIFY_LOG`): `<label>.notify` in the case's directory.
fn notify_log(ctx: &Ctx, label: &str) -> String {
    ctx.dir.join(format!("{label}.notify")).to_string_lossy().into_owned()
}

fn output_message(output: &Output) -> String {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    match (stdout.trim(), stderr.trim()) {
        ("", "") => output.status.to_string(),
        (stdout, "") => stdout.to_owned(),
        ("", stderr) => stderr.to_owned(),
        (stdout, stderr) => format!("{stdout}; {stderr}"),
    }
}
