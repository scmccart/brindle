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
use std::io::{Read as _, Write as _};
use std::net::{Ipv4Addr, TcpListener};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::brindle::{self, Dump, Run};
use crate::{Ctx, Outcome, Result, ensure};

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
        "{}\n[[profiles]]\n\
         name = {profile:?}\n\
         command = \"/bin/bash\"\n\
         args = [\"--norc\", \"--noprofile\", \"-i\"]\n\
         env = {{ SHELL = \"/bin/bash\", PS1 = \"REMOTE_CONTAINER_READY> \", BRINDLE_E2E_MARKER = \"container-exec\" }}\n\
         remote = {{ type = \"container\", engine = \"docker\", container = {container_name:?}, user = \"remote\" }}\n",
        brindle::config_with("e2e", CONTAINER_PROFILE, ""),
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
    let steps = vec![
        "--wait-for".into(),
        "REMOTE_CONTAINER_READY>".into(),
        "--send".into(),
        set_cwd,
        "--wait-for".into(),
        "REMOTE_CWD_SET".into(),
        "--action".into(),
        "split_zone_right".into(),
        "--wait-for".into(),
        "REMOTE_CONTAINER_READY>".into(),
        "--send".into(),
        exercise,
        "--wait-for".into(),
        "REMOTE_CONTAINER_STATUS_READY".into(),
    ];
    let dump = run_brindle(
        ctx,
        "container-pty",
        &config,
        CONTAINER_PROFILE,
        &steps,
        &agent,
        16.0,
    )?;
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

    Ok(Outcome::Pass)
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

    let config = format!(
        "{}{}",
        brindle::config_with("e2e", SSH_PROFILE, ""),
        ssh.profile(SSH_PROFILE, "ssh-upload", false),
    );
    let steps = vec![
        "--wait-for".into(),
        SSH_PROMPT.into(),
        "--send".into(),
        "printf 'SSH_REMOTE_IO:%s' \"$BRINDLE_E2E_MARKER\"; echo\\r".into(),
        "--wait-for".into(),
        "SSH_REMOTE_IO:ssh-upload".into(),
    ];
    let dump = run_brindle(
        ctx,
        "ssh-pty",
        &config,
        SSH_PROFILE,
        &steps,
        &needs.agent,
        12.0,
    )?;
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
    let left = poll(ctx.secs(1.0), 10.0, "the tab's session detached", || {
        let sessions = docker.sessions()?;
        Ok(match sessions.as_slice() {
            [session] if !session.attached => Some(session.clone()),
            _ => None,
        })
    })?;
    ctx.save("left.txt", format!("{left:?}\nprogram {program}\n"));
    docker.runs_in(program, "sleep", left.pid)?;

    let attached = ctx.dir.join("attached.txt");
    let (save, checked, slow) = (attached.clone(), docker.clone(), ctx.secs(1.0));
    let second = needs
        .run(ctx, "second", &config)
        .restore()
        .default_profile(SSH_PROFILE)
        .wait_for("sleeping-42")
        .action("scroll_to_top")
        .call_at(1.0, move |_| {
            let again = poll(slow, 30.0, "the session attached again", || {
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
    let (save, docker, slow) = (counts.clone(), ssh.container.docker.clone(), ctx.secs(1.0));
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
            count_connections(&docker, brindle, slow, &save, COUNTED)
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
    slow: f64,
    save: &Path,
    counted: &str,
) -> Result {
    let sessions = poll(slow, 30.0, "three attached sessions", || {
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
    poll(slow, 15.0, "the closed tab's programs gone", || {
        let gone = pids.iter().all(|&pid| docker.stat(pid).is_none());
        Ok((gone && docker.sessions()?.is_empty()).then_some(()))
    })?;
    // The connection closes a few seconds after nothing uses it.
    poll(slow, 20.0, "the unused connection's ssh gone", || {
        Ok(ssh_processes(brindle).is_empty().then_some(()))
    })?;
    poll(slow, 10.0, "its relay gone", || {
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
        slow: ctx.secs(1.0),
    };
    let out = needs
        .run(ctx, "run", &ssh.profile(SSH_PROFILE, "ssh-network", false))
        .profile(SSH_PROFILE)
        // Waits of up to 45 s: the loss takes up to the dead-link limit.
        .step_interval(1.5)
        .wait_for(SSH_PROMPT)
        .send(&format!("echo $$ > {PID_FILE}; echo before-$((1+1)) $$\r"))
        .wait_for("before-2")
        .wait_for("lost; reconnecting")
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
    slow: f64,
}

impl Cut {
    fn run(&self, brindle: u32) -> Result {
        let (docker, slow) = (&self.docker, self.slow);
        let shell = poll(slow, 30.0, "the shell's pid", || {
            let text = docker.exec(&["cat", self.pid_file]).ok();
            Ok(text.and_then(|pid| pid.trim().parse::<u32>().ok()))
        })?;
        let session = poll(slow, 10.0, "the shell's session attached", || {
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
        poll(slow, 45.0, "the silent link noticed", || {
            let noticed = logged(&self.log, &ended) > 0 && !alive(links[0]);
            Ok(noticed.then_some(()))
        })?;
        let noticed = cut.elapsed();
        // Brindle retries while the network is down, and fails.
        let failed = format!("can't connect to {}", self.endpoint);
        poll(slow, 30.0, "a reconnect failing", || {
            Ok((logged(&self.log, &failed) > 0).then_some(()))
        })?;
        let down = cut.elapsed();
        docker.connect(&self.network, &self.address)?;
        let restored = Instant::now();
        let again = poll(slow, 30.0, "the session attached again", || {
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
        let relays = poll(slow, 15.0, "one relay left", || {
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
    let left = poll(ctx.secs(1.0), 10.0, "the tab's session detached", || {
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

    let (helper, server_dir, slow) = (ctx.helper.clone(), ctx.server_dir.clone(), ctx.secs(1.0));
    let second = needs
        .run(ctx, "second", &ssh.profile(SSH_PROFILE, "ssh-eager", true))
        .restore()
        .wait_for(SWEPT)
        .action("next_tab")
        .wait_for("left-2")
        .call_at(1.0, move |_| {
            poll(
                slow,
                30.0,
                "the detached session attached at launch",
                || {
                    let sessions = docker.sessions()?;
                    Ok(sessions
                        .iter()
                        .any(|s| s.attached && s.pid == shell)
                        .then_some(()))
                },
            )?;
            // The launch's own tab is the local one, which shows the marker.
            let local = poll(slow, 10.0, "the launch's local tab", || {
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
        second.dump.screen.iter().any(|line| *line == shown),
        "the attached tab doesn't show {shown:?}: {:?}",
        second.dump.screen
    );
    Ok(Outcome::Pass)
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
    for endpoint in [
        format!("remote@{}", container.name),
        format!("root@127.0.0.1:{port}"),
    ] {
        let offline = format!("{endpoint}  offline: ");
        ensure!(
            palette.iter().any(|row| row.starts_with(&offline)),
            "no offline row for {endpoint}: {palette:?}"
        );
    }
    ensure!(
        !container.running()?,
        "the stopped container {} is running",
        container.name
    );
    Ok(Outcome::Pass)
}

/// What a remote case needs from the machine: a Docker daemon, the Linux
/// helper Brindle deploys, and the fixture image.
struct Needs {
    agent: PathBuf,
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
        Run::new(ctx, label)
            .config(config)
            .env("BRINDLE_REMOTE_AGENT", &self.agent.to_string_lossy())
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

/// Polls `probe` every 100 ms until it gives a value, for at most `secs`
/// stretched by `slow` (the case's `ctx.secs(1.0)`). The last error it gave
/// says why it failed.
fn poll<T>(
    slow: f64,
    secs: f64,
    what: &str,
    mut probe: impl FnMut() -> Result<Option<T>>,
) -> Result<T> {
    let deadline = Instant::now() + Duration::from_secs_f64(secs * slow);
    let mut last = String::new();
    loop {
        match probe() {
            Ok(Some(value)) => return Ok(value),
            Ok(None) => {}
            Err(why) => last = why,
        }
        if Instant::now() >= deadline {
            let why = if last.is_empty() {
                String::new()
            } else {
                format!(" (last: {last})")
            };
            return Err(format!("{what}: not within {secs} s{why}"));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
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

fn remote_agent(ctx: &Ctx) -> Result<Option<PathBuf>> {
    let path = std::env::var_os("BRINDLE_REMOTE_AGENT")
        .map(PathBuf::from)
        .unwrap_or_else(|| ctx.bin.with_file_name("brindle-remote"));
    if !path.is_file() {
        if std::env::var("BRINDLE_REMOTE_E2E_REQUIRED").as_deref() == Ok("1") {
            return Err(format!(
                "Linux remote helper not found at {}; run `cargo build -p brindle-remote-agent`",
                path.display()
            ));
        }
        return Ok(None);
    }
    fs::canonicalize(&path)
        .map(Some)
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
        let deadline = Instant::now() + Duration::from_secs_f64(ctx.secs(10.0));
        loop {
            let output = Command::new("docker")
                .args(["inspect", "--format", "{{.State.Running}}", &self.name])
                .output()
                .map_err(|error| format!("probing Docker fixture container: {error}"))?;
            let message = output_message(&output);
            if output.status.success() && String::from_utf8_lossy(&output.stdout).trim() == "true" {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(format!("Docker fixture container was not ready: {message}"));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
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
    let deadline = Instant::now() + Duration::from_secs_f64(ctx.secs(15.0));
    loop {
        let output = ssh_command(identity, port)
            .arg(format!("root@{host}"))
            .arg("printf BRINDLE_SSH_READY")
            .output()
            .map_err(|error| format!("probing fixture SSH server: {error}"))?;
        let message = output_message(&output);
        if output.status.success() && String::from_utf8_lossy(&output.stdout) == "BRINDLE_SSH_READY"
        {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("fixture SSH server was not ready: {message}"));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
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

fn run_brindle(
    ctx: &Ctx,
    label: &str,
    config: &str,
    profile: &str,
    steps: &[String],
    agent: &Path,
    dump_after: f64,
) -> Result<Dump> {
    let config_path = ctx.dir.join(format!("{label}.toml"));
    fs::write(&config_path, config)
        .map_err(|error| format!("writing {}: {error}", config_path.display()))?;
    let log_path = ctx.dir.join(format!("{label}.log"));
    let log = fs::File::create(&log_path)
        .map_err(|error| format!("creating {}: {error}", log_path.display()))?;
    let dump_after = ctx.secs(dump_after);
    let mut command = Command::new(&ctx.bin);
    let child = command
        .args(["-p", profile])
        .args(steps)
        .args(ctx.step_interval())
        .args(["--dump-screen-after", &dump_after.to_string()])
        .env("BRINDLE_CONFIG", &config_path)
        .env("BRINDLE_STATE_DIR", crate::brindle::state_dir(ctx))
        .env("BRINDLE_REMOTE_AGENT", agent)
        .env("BRINDLE_SERVER_DIR", &ctx.server_dir)
        .env(
            "BRINDLE_NOTIFY_LOG",
            ctx.dir.join(format!("{label}.notify")),
        )
        .env("DISPLAY", &ctx.display)
        .env_remove("WAYLAND_DISPLAY")
        .stdout(Stdio::piped())
        .stderr(log)
        .spawn()
        .map_err(|error| format!("starting {}: {error}", ctx.bin.display()))?;
    let mut child = ChildGuard { child };
    let deadline = Instant::now() + Duration::from_secs_f64(dump_after + 30.0);
    let status = loop {
        if let Some(status) = child
            .child
            .try_wait()
            .map_err(|error| format!("waiting for Brindle: {error}"))?
        {
            break status;
        }
        if Instant::now() >= deadline {
            return Err(format!("Brindle did not exit; see {}", log_path.display()));
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    let mut stdout = String::new();
    child
        .child
        .stdout
        .take()
        .ok_or_else(|| "Brindle stdout was not piped".to_string())?
        .read_to_string(&mut stdout)
        .map_err(|error| format!("reading Brindle screen dump: {error}"))?;
    ctx.save(&format!("{label}.dump"), &stdout);
    ensure!(
        status.success(),
        "Brindle exited with {status}; see {}",
        log_path.display()
    );
    let dump = Dump::parse(&stdout);
    ensure!(
        dump.size.is_some(),
        "no screen in {label}.dump; see {}",
        log_path.display()
    );
    Ok(dump)
}

struct ChildGuard {
    child: Child,
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        if self.child.try_wait().ok().flatten().is_none() {
            self.child.kill().ok();
            self.child.wait().ok();
        }
    }
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
