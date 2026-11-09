//! Local tabs in the terminal server (Linux): where their programs run,
//! that closing ends them and quitting keeps them, reattaching on the next launch, the sessions
//! palette and `--attach`, the environment they get, reconnecting after the
//! core restarts, sessions lost for good, a missing helper, an upgrade, and
//! the management commands.
//!
//! Every case has its own server directory (`ctx.server_dir`), which the
//! runner stops and checks afterwards. Checks that must happen while
//! Brindle runs are [`Run::call_at`] calls, which poll rather than trust
//! timing; a call that must hold a later step back writes a marker to the
//! tab's terminal, which a [`Run::wait_for`] step waits for.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use super::multi_window::two_processes;
use super::server;
use crate::brindle::Run;
use crate::{Ctx, Outcome, Result, ensure};

/// The case's terminal server, as `brindle-remote` sees it.
#[derive(Clone)]
struct Server {
    dir: PathBuf,
    helper: PathBuf,
    slow: f64,
}

/// One line of `brindle-remote list`.
#[derive(Clone, Debug)]
struct Listed {
    id: u32,
    pid: u32,
    attached: bool,
}

impl Server {
    fn of(ctx: &Ctx) -> Self {
        Self { dir: ctx.server_dir.clone(), helper: ctx.helper.clone(), slow: ctx.secs(1.0) }
    }

    /// Runs a `brindle-remote` command against the case's server.
    fn command(&self, args: &[&str]) -> Result<String> {
        let out = std::process::Command::new(&self.helper)
            .args(args)
            .env("BRINDLE_SERVER_DIR", &self.dir)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| format!("running {}: {e}", self.helper.display()))?;
        ensure!(
            out.status.success(),
            "brindle-remote {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    fn list(&self) -> Result<Vec<Listed>> {
        Ok(parse_list(&self.command(&["list"])?))
    }

    /// `status --json`.
    fn status(&self) -> Result<serde_json::Value> {
        let text = self.command(&["status", "--json"])?;
        serde_json::from_str(&text).map_err(|e| format!("status --json {text:?}: {e}"))
    }

    /// The running core's pid.
    fn core(&self) -> Option<u32> {
        crate::server_processes(&self.dir).into_iter().find(|(_, role)| role == "serve").map(|(pid, _)| pid)
    }

    /// Polls `probe` until it gives a value, for at most `secs` (stretched).
    fn wait<T>(&self, secs: f64, what: &str, mut probe: impl FnMut() -> Result<Option<T>>) -> Result<T> {
        let deadline = Instant::now() + Duration::from_secs_f64(secs * self.slow);
        let mut last = String::new();
        loop {
            match probe() {
                Ok(Some(value)) => return Ok(value),
                Ok(None) => {}
                Err(why) => last = why,
            }
            if Instant::now() >= deadline {
                let why = if last.is_empty() { String::new() } else { format!(" (last: {last})") };
                return Err(format!("{what}: not within {secs} s{why}"));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// The one attached session.
    fn attached(&self, secs: f64) -> Result<Listed> {
        self.wait(secs, "an attached session", || Ok(self.list()?.into_iter().find(|s| s.attached)))
    }
}

/// `id<TAB>pid<TAB>attached|detached<TAB>policy<TAB>…` lines.
fn parse_list(text: &str) -> Vec<Listed> {
    text.lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            Some(Listed {
                id: fields.first()?.parse().ok()?,
                pid: fields.get(1)?.parse().ok()?,
                attached: *fields.get(2)? == "attached",
            })
        })
        .collect()
}

/// The listed sessions, once `ok` holds for them.
fn listed_when(srv: &Server, secs: f64, what: &str, ok: impl Fn(&[Listed]) -> bool) -> Result<Vec<Listed>> {
    srv.wait(secs, what, || {
        let sessions = if srv.core().is_some() { srv.list()? } else { Vec::new() };
        Ok(ok(&sessions).then_some(sessions))
    })
}

/// `program` runs, as a child of one of the `shells` (sessions' pids).
fn in_session(program: u32, shells: &[u32]) -> bool {
    alive(program) && parent(program).is_some_and(|p| shells.contains(&p))
}

/// The detached sessions' pids, once there are `count`.
fn detached_pids(srv: &Server, secs: f64, count: usize) -> Result<Vec<u32>> {
    let sessions = listed_when(srv, secs, &format!("{count} detached session(s)"), |sessions| {
        sessions.iter().filter(|s| !s.attached).count() == count
    })?;
    Ok(sessions.iter().filter(|s| !s.attached).map(|s| s.pid).collect())
}

/// `/proc/<pid>/stat` fields after the command name: state, ppid, ...
fn stat(pid: u32) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, rest) = text.rsplit_once(") ")?;
    Some(rest.split(' ').map(str::to_string).collect())
}

/// Running, not a zombie.
fn alive(pid: u32) -> bool {
    stat(pid).is_some_and(|fields| fields[0] != "Z")
}

fn parent(pid: u32) -> Option<u32> {
    stat(pid)?.get(1)?.parse().ok()
}

fn cmdline(pid: u32) -> Vec<String> {
    let bytes = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
    bytes.split(|&b| b == 0).filter(|a| !a.is_empty()).map(|a| String::from_utf8_lossy(a).into_owned()).collect()
}

/// `brindle-remote hold`, as the server starts it.
fn is_holder(pid: u32) -> bool {
    cmdline(pid).get(1).is_some_and(|role| role == "hold")
}

fn kill(pid: u32, signal: &str) -> Result {
    let status = std::process::Command::new("kill").args([signal, &pid.to_string()]).status();
    ensure!(status.is_ok_and(|s| s.success()), "kill {signal} {pid} failed");
    Ok(())
}

/// The pid a program wrote to `file`, once it has.
fn read_pid(file: &Path) -> Option<u32> {
    std::fs::read_to_string(file).ok()?.trim().parse().ok()
}

/// A command for the tab that writes its pid to `file`, then runs `program`
/// in its place, so `file` holds the program's pid.
fn recorded(file: &Path, program: &str) -> String {
    format!("sh -c 'echo $$ > \"{}\"; exec {program}'\r", file.display())
}

/// Writes `text` to the terminal of `pid`'s program, which shows it: a
/// marker for a [`Run::wait_for`] step.
fn mark(pid: u32, text: &str) -> Result {
    let mut tty = std::fs::OpenOptions::new()
        .write(true)
        .open(format!("/proc/{pid}/fd/1"))
        .map_err(|e| format!("opening {pid}'s terminal: {e}"))?;
    tty.write_all(text.as_bytes()).map_err(|e| format!("writing to {pid}'s terminal: {e}"))
}

/// Waits for `pids` to be gone.
fn gone(server: &Server, secs: f64, pids: &[u32]) -> Result {
    server.wait(secs, &format!("programs {pids:?} gone"), || Ok(pids.iter().all(|&pid| !alive(pid)).then_some(())))
}

fn screen_has(out: &crate::brindle::Output, text: &str) -> Result {
    ensure!(out.dump.screen.iter().any(|l| l.contains(text)), "no {text:?} on the screen: {:?}", out.dump.screen);
    Ok(())
}

/// The shell's parent is a holder, and `list` shows its session attached.
pub fn session(ctx: &Ctx) -> Result<Outcome> {
    let tmux = server()?;
    let srv = Server::of(ctx);
    let found = ctx.dir.join("session-pid");
    let save = found.clone();
    let out = Run::new(ctx, &tmux, "run").keep_detached()
        .profile("Shell")
        .send("echo shell=$$\r")
        .call_at(2.0, move |_| {
            let listed = srv.attached(10.0)?;
            let sessions = srv.list()?;
            ensure!(sessions.len() == 1, "sessions: {sessions:?}");
            let holder = parent(listed.pid).ok_or("the shell has no parent")?;
            ensure!(is_holder(holder), "the shell's parent {holder} is {:?}, not a holder", cmdline(holder));
            std::fs::write(&save, listed.pid.to_string()).map_err(|e| e.to_string())
        })
        .dump_at(5.0)
        .run()?;
    let pid = read_pid(&found).ok_or("the session wasn't checked")?;
    ensure!(out.dump.screen.iter().any(|l| l == &format!("shell={pid}")), "the tab's shell isn't {pid}: {:?}", out.dump.screen);
    Ok(Outcome::Pass)
}

/// Closing a tab running `sleep 1000` ends `sleep` and its session; the
/// other tab's stays attached.
pub fn close(ctx: &Ctx) -> Result<Outcome> {
    let tmux = server()?;
    let srv = Server::of(ctx);
    let file = ctx.dir.join("sleep-pid");
    let watched = file.clone();
    let out = Run::new(ctx, &tmux, "run").keep_detached()
        .profile("Shell")
        .default_profile("Shell")
        .action("new_tab")
        .send(&format!("sh -c 'echo $$ > \"{}\"; echo sleeping-$((40+2)); exec sleep 1000'\r", file.display()))
        .wait_for("sleeping-42")
        .action("close_tab")
        .call_at(1.0, move |brindle| {
            let pid = srv.wait(15.0, "sleep started", || Ok(read_pid(&watched)))?;
            gone(&srv, 10.0, &[pid])?;
            listed_when(&srv, 10.0, "the other tab's session alone", |s| s.len() == 1 && s[0].attached)?;
            ensure!(alive(brindle), "Brindle quit");
            Ok(())
        })
        .dump_at(10.0)
        .run()?;
    ensure!(out.dump.tabs.len() == 1, "tabs: {:?}", out.dump.tabs);
    ensure!(read_pid(&file).is_some(), "sleep never started");
    Ok(Outcome::Pass)
}

/// Starts `sleep 1000` in two tabs, recording their pids.
fn two_sleeps<'a>(ctx: &'a Ctx, tmux: &'a crate::tmux::TmuxServer) -> (Run<'a>, [PathBuf; 2]) {
    let files = [ctx.dir.join("sleep-1"), ctx.dir.join("sleep-2")];
    let run = Run::new(ctx, tmux, "run").keep_detached()
        .profile("Shell")
        .default_profile("Shell")
        .send(&recorded(&files[0], "sleep 1000"))
        .action("new_tab")
        .send(&recorded(&files[1], "sleep 1000"));
    (run, files)
}

fn wait_pids(srv: &Server, files: &[PathBuf; 2]) -> Result<[u32; 2]> {
    let first = srv.wait(15.0, "the first sleep", || Ok(read_pid(&files[0])))?;
    let second = srv.wait(15.0, "the second sleep", || Ok(read_pid(&files[1])))?;
    Ok([first, second])
}

/// Quitting Brindle leaves every tab's program running, detached.
pub fn quit(ctx: &Ctx) -> Result<Outcome> {
    let tmux = server()?;
    let srv = Server::of(ctx);
    let (run, files) = two_sleeps(ctx, &tmux);
    let checked = srv.clone();
    let watched = files.clone();
    run.call_at(1.0, move |_| {
        let pids = wait_pids(&checked, &watched)?;
        ensure!(pids.iter().all(|&pid| alive(pid)), "the sleeps {pids:?} aren't running");
        Ok(())
    })
    .dump_at(6.0)
    .run()?;
    let pids = wait_pids(&srv, &files)?;
    let detached = detached_pids(&srv, 10.0, 2)?;
    for pid in pids {
        ensure!(in_session(pid, &detached), "sleep {pid} isn't in a detached session's shell {detached:?}");
    }
    Ok(Outcome::Pass)
}

/// Brindle killed with SIGKILL: its programs keep running, detached.
pub fn crash(ctx: &Ctx) -> Result<Outcome> {
    let tmux = server()?;
    let srv = Server::of(ctx);
    let (run, files) = two_sleeps(ctx, &tmux);
    let checked = srv.clone();
    let watched = files.clone();
    run.call_at(1.0, move |brindle| {
        wait_pids(&checked, &watched)?;
        kill(brindle, "-KILL")
    })
    .dump_at(20.0)
    .expect_killed()
    .run()?;
    let pids = wait_pids(&srv, &files)?;
    detached_pids(&srv, 10.0, 2)?;
    ensure!(pids.iter().all(|&pid| alive(pid)), "the sleeps {pids:?} ended");
    Ok(Outcome::Pass)
}

/// Closing a zone ends its program; the tab keeps the other zone, whose
/// shell quitting then leaves detached.
pub fn close_zone(ctx: &Ctx) -> Result<Outcome> {
    let tmux = server()?;
    let srv = Server::of(ctx);
    let file = ctx.dir.join("sleep-pid");
    let out = Run::new(ctx, &tmux, "run").keep_detached()
        .profile("Shell")
        .send("echo left-zone\r")
        .action("split_zone_right")
        .send(&format!("sh -c 'echo $$ > \"{}\"; echo zoned-$((40+2)); exec sleep 1000'\r", file.display()))
        .wait_for("zoned-42")
        .action("close_zone")
        .dump_at(8.0)
        .run()?;
    ensure!(out.dump.zones.is_none(), "zones left: {:?}", out.dump.zones);
    screen_has(&out, "left-zone")?;
    let pid = read_pid(&file).ok_or("sleep never started")?;
    gone(&srv, 5.0, &[pid])?;
    detached_pids(&srv, 10.0, 1)?;
    Ok(Outcome::Pass)
}

/// Closing the window from the window manager (`WM_DELETE_WINDOW`) quits
/// Brindle without asking and leaves the program detached, still running.
pub fn close_window(ctx: &Ctx) -> Result<Outcome> {
    let tmux = server()?;
    let srv = Server::of(ctx);
    let file = ctx.dir.join("sleep-pid");
    let watched = file.clone();
    let display = ctx.display.clone();
    Run::new(ctx, &tmux, "run").keep_detached()
        .profile("Shell")
        .send(&format!("sh -c 'echo $$ > \"{}\"; echo windowed-$((40+2)); exec sleep 1000'\r", file.display()))
        .wait_for("windowed-42")
        .call_at(3.0, move |brindle| {
            srv.wait(15.0, "sleep started", || Ok(read_pid(&watched)))?;
            crate::capture::request_close(&display, brindle)
        })
        .expect_exit()
        .dump_at(20.0)
        .run()?;
    let pid = read_pid(&file).ok_or("sleep never started")?;
    let detached = detached_pids(&Server::of(ctx), 10.0, 1)?;
    ensure!(in_session(pid, &detached), "sleep {pid} isn't detached: {detached:?}");
    Ok(Outcome::Pass)
}

/// An `-e` window's program ends with its window, and leaves nothing.
pub fn close_command(ctx: &Ctx) -> Result<Outcome> {
    let tmux = server()?;
    let srv = Server::of(ctx);
    let file = ctx.dir.join("sleep-pid");
    let script = format!("echo $$ > \"{}\"; echo commanded-$((40+2)); exec sleep 1000", file.display());
    Run::new(ctx, &tmux, "run").keep_detached()
        .profile("Shell")
        .wait_for("commanded-42")
        .action("close_window")
        .expect_exit()
        .dump_at(10.0)
        .command(&["sh", "-c", &script])
        .run()?;
    let pid = read_pid(&file).ok_or("sleep never started")?;
    gone(&srv, 5.0, &[pid])?;
    listed_when(&srv, 10.0, "no sessions", <[Listed]>::is_empty)?;
    Ok(Outcome::Pass)
}

/// `pid=<n>` from the dumped screen.
fn shown_pid(out: &crate::brindle::Output, prefix: &str) -> Result<u32> {
    out.dump
        .screen
        .iter()
        .find_map(|l| l.strip_prefix(prefix)?.trim().parse().ok())
        .ok_or_else(|| format!("no {prefix} line: {:?}", out.dump.screen))
}

/// Quit, then a plain launch: the tab is back with its shell (same pid)
/// and its screen, and no bell or notification from the replay.
pub fn reattach_quit(ctx: &Ctx) -> Result<Outcome> {
    let tmux = server()?;
    let notify = "\n[notifications]\nwhen = \"always\"\n";
    let (live_log, replay_log) = (ctx.dir.join("notify-first.log"), ctx.dir.join("notify-second.log"));
    let first = Run::new(ctx, &tmux, "first").keep_detached()
        .restore()
        .default_profile("Shell")
        .config(notify)
        .env("BRINDLE_NOTIFY_LOG", live_log.to_str().unwrap())
        .send(r"printf '\a\033]9;Ping\a'; echo first=$$\r")
        .dump_at(4.0)
        .run()?;
    let pid = shown_pid(&first, "first=")?;
    detached_pids(&Server::of(ctx), 10.0, 1)?;
    let second = Run::new(ctx, &tmux, "second").keep_detached()
        .restore()
        .default_profile("Shell")
        .config(notify)
        .env("BRINDLE_NOTIFY_LOG", replay_log.to_str().unwrap())
        .send("echo again=$$\r")
        .wait_for("again=")
        .dump_at(5.0)
        .run()?;
    ensure!(second.dump.tabs.len() == 1, "tabs: {:?}", second.dump.tabs);
    screen_has(&second, &format!("first={pid}"))?;
    ensure!(shown_pid(&second, "again=")? == pid, "another shell: {:?}", second.dump.screen);
    let replayed = std::fs::read_to_string(&replay_log).unwrap_or_default();
    ensure!(replayed.is_empty(), "the replay notified: {replayed:?}");
    Ok(Outcome::Pass)
}

/// Brindle SIGKILLed with a split tab: a plain launch brings back both
/// zones' sessions, and the left one's output.
pub fn reattach_crash(ctx: &Ctx) -> Result<Outcome> {
    let tmux = server()?;
    let srv = Server::of(ctx);
    let saved = ctx.dir.join("pids");
    let save = saved.clone();
    let checked = srv.clone();
    Run::new(ctx, &tmux, "first").keep_detached()
        .restore()
        .default_profile("Shell")
        .send("seq -f scroll-%g 1 300\r")
        .wait_for("scroll-300")
        .action("split_zone_right")
        .send("echo right-$((1+1))\r")
        .wait_for("right-2")
        .call_at(5.0, move |brindle| {
            let sessions = listed_when(&checked, 10.0, "two attached sessions", |s| {
                s.len() == 2 && s.iter().all(|s| s.attached)
            })?;
            // The state file is written within a second of a change.
            std::thread::sleep(Duration::from_secs_f64(3.0 * checked.slow));
            let mut pids: Vec<u32> = sessions.iter().map(|s| s.pid).collect();
            pids.sort();
            std::fs::write(&save, format!("{pids:?}")).map_err(|e| e.to_string())?;
            kill(brindle, "-KILL")
        })
        .dump_at(20.0)
        .expect_killed()
        .run()?;
    let before = std::fs::read_to_string(&saved).map_err(|_| "the sessions weren't saved")?;
    detached_pids(&srv, 10.0, 2)?;
    let checked = srv.clone();
    let after = ctx.dir.join("pids-after");
    let save = after.clone();
    let out = Run::new(ctx, &tmux, "second").keep_detached()
        .restore()
        .default_profile("Shell")
        .call_at(2.0, move |_| {
            let sessions = listed_when(&checked, 10.0, "both attached again", |s| {
                s.len() == 2 && s.iter().all(|s| s.attached)
            })?;
            let mut pids: Vec<u32> = sessions.iter().map(|s| s.pid).collect();
            pids.sort();
            std::fs::write(&save, format!("{pids:?}")).map_err(|e| e.to_string())
        })
        .action("focus_zone_left")
        .action("focus_zone_left")
        .dump_at(6.0)
        .run()?;
    let after = std::fs::read_to_string(&after).map_err(|_| "the second run's sessions weren't checked")?;
    ensure!(before == after, "sessions before {before}, after {after}");
    ensure!(out.dump.tabs.len() == 1, "tabs: {:?}", out.dump.tabs);
    ensure!(out.dump.zones.as_deref().is_some_and(|z| z.starts_with("count=2")), "zones: {:?}", out.dump.zones);
    screen_has(&out, "scroll-300")?;
    Ok(Outcome::Pass)
}

/// A recorded session killed while Brindle was gone isn't restored: in a
/// split, the other zone takes the tab; with every session gone, the
/// launch opens one fresh tab.
pub fn reattach_killed(ctx: &Ctx) -> Result<Outcome> {
    let tmux = server()?;
    let srv = Server::of(ctx);
    let first = Run::new(ctx, &tmux, "first").keep_detached()
        .restore()
        .default_profile("Shell")
        .send("echo left=$$\r")
        .wait_for("left=")
        .action("split_zone_right")
        .send("echo right=$$\r")
        .wait_for("right=")
        .dump_at(6.0)
        .run()?;
    let right = shown_pid(&first, "right=")?;
    let sessions = listed_when(&srv, 10.0, "two detached sessions", |s| s.len() == 2 && s.iter().all(|s| !s.attached))?;
    let killed = sessions.iter().find(|s| s.pid == right).ok_or("the right zone's session isn't listed")?.id;
    srv.command(&["kill", &killed.to_string()])?;
    gone(&srv, 5.0, &[right])?;
    let second = Run::new(ctx, &tmux, "second").keep_detached()
        .restore()
        .default_profile("Shell")
        .send("echo again=$$\r")
        .wait_for("again=")
        .dump_at(5.0)
        .run()?;
    ensure!(second.dump.tabs.len() == 1, "tabs: {:?}", second.dump.tabs);
    ensure!(second.dump.zones.is_none(), "zones: {:?}", second.dump.zones);
    let left = shown_pid(&second, "again=")?;
    screen_has(&second, &format!("left={left}"))?;

    // Now the left zone's session goes too.
    let id = listed_when(&srv, 10.0, "the left session detached", |s| s.len() == 1 && !s[0].attached)?[0].id;
    srv.command(&["kill", &id.to_string()])?;
    gone(&srv, 5.0, &[left])?;
    let third = Run::new(ctx, &tmux, "third").restore().default_profile("Shell").send("echo third=$$\r").wait_for("third=").dump_at(4.0).run()?;
    ensure!(third.dump.tabs.len() == 1, "tabs: {:?}", third.dump.tabs);
    ensure!(!third.dump.screen.concat().contains("left="), "the old screen is back: {:?}", third.dump.screen);
    Ok(Outcome::Pass)
}

/// A restored tab that isn't active (so not laid out when its replay
/// arrives) shows its screen where it was once shown: its prompt right
/// after its output. The replay is rendered at the session's size, taller
/// than an unlaid-out terminal's 24 rows, so fed at the wrong size its rows
/// scroll away and the cursor ends up far below the prompt.
pub fn reattach_inactive(ctx: &Ctx) -> Result<Outcome> {
    let tmux = server()?;
    Run::new(ctx, &tmux, "first").keep_detached()
        .restore()
        .default_profile("Shell")
        .send(r"clear; echo hidden-$((1+1))\r")
        .wait_for("hidden-2")
        .action("new_tab")
        .send("echo shown\r")
        .dump_at(5.0)
        .run()?;
    detached_pids(&Server::of(ctx), 10.0, 2)?;
    let out = Run::new(ctx, &tmux, "second").keep_detached()
        .restore()
        .default_profile("Shell")
        .action("prev_tab")
        .dump_at(5.0)
        .run()?;
    ensure!(out.dump.tabs.len() == 2 && out.dump.tabs[0].1, "tabs: {:?}", out.dump.tabs);
    let screen = &out.dump.screen;
    let output = screen.iter().rposition(|l| l == "hidden-2").ok_or_else(|| format!("no output: {screen:?}"))?;
    let last = screen.iter().rposition(|l| !l.trim().is_empty()).unwrap_or(0);
    ensure!(
        last == output + 1 && screen[last].starts_with("bash-"),
        "the prompt isn't right after the output: {screen:?}"
    );
    Ok(Outcome::Pass)
}

/// A tab closed before quitting doesn't come back: its program ended.
pub fn reattach_closed_tab(ctx: &Ctx) -> Result<Outcome> {
    let tmux = server()?;
    Run::new(ctx, &tmux, "first").keep_detached()
        .restore()
        .default_profile("Shell")
        .send("echo restored-tab\r")
        .action("new_tab")
        .send("echo closed-tab\r")
        .action("close_tab")
        .dump_at(6.0)
        .run()?;
    detached_pids(&Server::of(ctx), 10.0, 1)?;
    let out = Run::new(ctx, &tmux, "second").keep_detached().restore().default_profile("Shell").dump_at(5.0).run()?;
    ensure!(out.dump.tabs.len() == 1, "tabs: {:?}", out.dump.tabs);
    screen_has(&out, "restored-tab")?;
    Ok(Outcome::Pass)
}

/// `--no-restore` leaves the detached sessions alone, and the next launch
/// attaches them after the tabs it restores.
pub fn reattach_no_restore(ctx: &Ctx) -> Result<Outcome> {
    let tmux = server()?;
    let srv = Server::of(ctx);
    Run::new(ctx, &tmux, "first").keep_detached().restore().default_profile("Shell").send("echo left-$((1+1))\r").dump_at(4.0).run()?;
    let before = detached_pids(&srv, 10.0, 1)?;
    let checked = srv.clone();
    let out = Run::new(ctx, &tmux, "second").keep_detached()
        .profile("Shell")
        .send("echo own-tab\r")
        .call_at(3.0, move |_| {
            let sessions = listed_when(&checked, 10.0, "the new tab's session", |s| s.len() == 2)?;
            let detached: Vec<u32> = sessions.iter().filter(|s| !s.attached).map(|s| s.pid).collect();
            ensure!(detached == before, "detached {detached:?}, before {before:?}");
            Ok(())
        })
        .dump_at(5.0)
        .run()?;
    ensure!(out.dump.tabs.len() == 1, "tabs: {:?}", out.dump.tabs);
    let third = Run::new(ctx, &tmux, "third").keep_detached().restore().default_profile("Shell").action("next_tab").dump_at(5.0).run()?;
    ensure!(third.dump.tabs.len() == 2 && third.dump.tabs[1].1, "tabs: {:?}", third.dump.tabs);
    screen_has(&third, "left-2")?;
    Ok(Outcome::Pass)
}

/// A debug run beside a running Brindle runs on its own: it attaches none
/// of the detached sessions.
pub fn reattach_second(ctx: &Ctx) -> Result<Outcome> {
    let tmux = server()?;
    let srv = Server::of(ctx);
    Run::new(ctx, &tmux, "quit").keep_detached().restore().default_profile("Shell").dump_at(4.0).run()?;
    let before = detached_pids(&srv, 10.0, 1)?;
    // The instance leaves them alone too (`--no-restore`).
    let first = Run::new(ctx, &tmux, "first").keep_detached().profile("Shell").dump_at(9.0);
    let checked = srv.clone();
    let second = Run::new(ctx, &tmux, "second")
        .keep_detached()
        .restore()
        .default_profile("Shell")
        .call_at(2.0, move |_| {
            let sessions = listed_when(&checked, 10.0, "both Brindles' tabs", |s| s.len() == 3)?;
            let detached: Vec<u32> = sessions.iter().filter(|s| !s.attached).map(|s| s.pid).collect();
            ensure!(detached == before, "detached {detached:?}, before {before:?}");
            Ok(())
        })
        .dump_at(4.0);
    let (_, second) = two_processes(ctx, 3.0, first, second)?;
    ensure!(second.dump.tabs.len() == 1, "the second Brindle's tabs: {:?}", second.dump.tabs);
    Ok(Outcome::Pass)
}

/// The sessions palette lists detached sessions: ctrl-shift-delete kills
/// the selected (newest) one, enter attaches the next, and an empty list
/// says so.
pub fn palette(ctx: &Ctx) -> Result<Outcome> {
    let tmux = server()?;
    let srv = Server::of(ctx);
    let [older, newer] = [ctx.dir.join("older-pid"), ctx.dir.join("newer-pid")];
    // Quitting leaves both tabs' shells detached.
    Run::new(ctx, &tmux, "first").keep_detached()
        .profile("Shell")
        .default_profile("Shell")
        .send(&format!("echo $$ > \"{}\"; echo older-$((1+1))\r", older.display()))
        .wait_for("older-2")
        .action("new_tab")
        .send(&format!("echo $$ > \"{}\"\r", newer.display()))
        .dump_at(5.0)
        .run()?;
    detached_pids(&srv, 10.0, 2)?;
    let out = Run::new(ctx, &tmux, "run").keep_detached()
        .profile("Shell")
        .default_profile("Shell")
        .action("attach_session")
        .key("ctrl-shift-delete")
        .key("enter")
        .action("attach_session")
        .dump_at(14.0)
        .run()?;
    ensure!(out.dump.tabs.len() == 2, "tabs: {:?}", out.dump.tabs);
    screen_has(&out, "older-2")?;
    // Open, with no rows (its empty message says "No detached sessions").
    ensure!(
        out.dump.palette.len() == 1 && out.dump.palette[0].starts_with("list "),
        "palette: {:?}",
        out.dump.palette
    );
    let newer = read_pid(&newer).ok_or("the newer shell never ran")?;
    gone(&srv, 5.0, &[newer])?;
    let older = read_pid(&older).ok_or("the older shell never ran")?;
    ensure!(alive(older), "the attached shell {older} ended");
    Ok(Outcome::Pass)
}

/// `brindle --attach ID` opens that session as the active tab; an id the
/// server doesn't have gives a tab saying so.
pub fn attach_flag(ctx: &Ctx) -> Result<Outcome> {
    let tmux = server()?;
    Run::new(ctx, &tmux, "first").keep_detached()
        .profile("Shell")
        .default_profile("Shell")
        .send("echo unflagged\r")
        .action("new_tab")
        .send("echo flagged-$((1+1))\r")
        .wait_for("flagged-2")
        .dump_at(6.0)
        .run()?;
    let srv = Server::of(ctx);
    let sessions = listed_when(&srv, 10.0, "two detached sessions", |s| s.len() == 2 && s.iter().all(|s| !s.attached))?;
    // The new tab spawned after the first, so its session has the higher id.
    let id = sessions.iter().map(|s| s.id).max().ok_or("no sessions")?;
    let out = Run::new(ctx, &tmux, "second").keep_detached()
        .profile("Shell")
        .arg("--attach")
        .arg(&id.to_string())
        .dump_at(5.0)
        .run()?;
    screen_has(&out, "flagged-2")?;
    let missing = Run::new(ctx, &tmux, "missing").keep_detached()
        .profile("Shell")
        .arg("--no-restore")
        .arg("--attach")
        .arg("999")
        .dump_at(4.0)
        .run()?;
    screen_has(&missing, "could not attach session 999")?;
    screen_has(&missing, "no session 999")?;
    Ok(Outcome::Pass)
}

/// A second Brindle with `FOO=2` gives its tabs `FOO=2`, though the first
/// Brindle (`FOO=1`) started the server.
pub fn env(ctx: &Ctx) -> Result<Outcome> {
    let tmux = server()?;
    let first = Run::new(ctx, &tmux, "first").keep_detached().profile("Shell").env("FOO", "1").send("echo foo-$FOO-x\r").dump_at(8.0);
    let second =
        Run::new(ctx, &tmux, "second").keep_detached().profile("Shell").env("FOO", "2").send("echo foo-$FOO-x\r").dump_at(4.0);
    let (first, second) = two_processes(ctx, 3.0, first, second)?;
    screen_has(&first, "foo-1-x")?;
    screen_has(&second, "foo-2-x")?;
    ensure!(!second.dump.screen.concat().contains("foo-1-x"), "second: {:?}", second.dump.screen);
    Ok(Outcome::Pass)
}

/// Kills the core, then waits for another core and for the session
/// `listed` to be attached again with the same pid.
fn restart_core(srv: &Server, listed: &Listed) -> Result {
    let old = srv.core().ok_or("no core running")?;
    kill(old, "-KILL")?;
    srv.wait(20.0, "a new core", || Ok(srv.core().filter(|&pid| pid != old)))?;
    srv.wait(20.0, "the session attached again", || {
        Ok(srv.list()?.into_iter().find(|s| s.attached && s.pid == listed.pid))
    })?;
    Ok(())
}

/// The core SIGKILLed while the tab runs `less`: the tab gets its screen
/// back, `less` keeps its pid and answers keys (G shows the end).
pub fn core_restart(ctx: &Ctx) -> Result<Outcome> {
    let tmux = server()?;
    let srv = Server::of(ctx);
    let text: String = (1..=99).map(|n| format!("line-{n:02}\n")).collect();
    let lines = ctx.dir.join("lines.txt");
    std::fs::write(&lines, text).map_err(|e| e.to_string())?;
    let file = ctx.dir.join("less-pid");
    let watched = file.clone();
    let out = Run::new(ctx, &tmux, "run").keep_detached()
        .profile("Shell")
        .send(&recorded(&file, &format!("less \"{}\"", lines.display())))
        .wait_for("line-01")
        .call_at(2.0, move |_| {
            let less = srv.wait(15.0, "less started", || Ok(read_pid(&watched)))?;
            let listed = srv.attached(10.0)?;
            // Let less draw before the core goes.
            std::thread::sleep(Duration::from_secs_f64(srv.slow));
            restart_core(&srv, &listed)?;
            ensure!(alive(less), "less {less} is gone");
            // The tab shows it once the replay arrived.
            mark(less, "\x1b[2;1HRECONNECTED-MARK")
        })
        .wait_for("RECONNECTED-MARK")
        .send("G")
        .wait_for("line-99")
        .dump_at(25.0)
        .run()?;
    screen_has(&out, "line-99")?;
    ensure!(!out.dump.screen.concat().contains("line-01"), "less didn't move to the end: {:?}", out.dump.screen);
    ensure!(out.dump.mode.contains("ALT_SCREEN"), "not on the alternate screen: {}", out.dump.mode);
    Ok(Outcome::Pass)
}

/// A bell, an OSC 9 and a `blocked` status since the last checkpoint
/// notify once, live, and not again when the core restarts and the tab
/// gets its replay; the status record is back.
pub fn replay_quiet(ctx: &Ctx) -> Result<Outcome> {
    let tmux = server()?;
    let srv = Server::of(ctx);
    let log = ctx.dir.join("notify.log");
    let watched = log.clone();
    let blocked = r"\033]7501;state=blocked:kind=permission:app=e2e:msg=T2s/\033\\";
    let out = Run::new(ctx, &tmux, "run").keep_detached()
        .profile("Shell")
        .config("\n[notifications]\nwhen = \"always\"\n")
        .env("BRINDLE_NOTIFY_LOG", log.to_str().unwrap())
        .send(&format!(r"printf '\a\033]9;Ping\a{blocked}'; cat\r"))
        .call_at(2.0, move |_| {
            let read = || std::fs::read_to_string(&watched).unwrap_or_default();
            let live = srv.wait(15.0, "the live notifications", || {
                let text = read();
                Ok((text.contains("osc9\t") && text.contains("osc7501\t")).then_some(()))
            });
            live?;
            let listed = srv.attached(10.0)?;
            // Anything late from the live output arrives first.
            std::thread::sleep(Duration::from_secs_f64(srv.slow));
            let before = read();
            restart_core(&srv, &listed)?;
            std::thread::sleep(Duration::from_secs_f64(3.0 * srv.slow));
            let after = read();
            ensure!(after == before, "the reconnect notified: before {before:?}, after {after:?}");
            Ok(())
        })
        .dump_at(22.0)
        .run()?;
    ensure!(
        out.dump.status.iter().any(|s| s.starts_with("state=blocked")),
        "the replay didn't restore the status: {:?}",
        out.dump.status
    );
    Ok(Outcome::Pass)
}

/// The session's holder SIGKILLed: the tab stays, with its screen and a
/// "session lost" line.
pub fn holder_lost(ctx: &Ctx) -> Result<Outcome> {
    let tmux = server()?;
    let srv = Server::of(ctx);
    let out = Run::new(ctx, &tmux, "run").keep_detached()
        .profile("Shell")
        .send("echo before-$((1+1))\r")
        .call_at(2.0, move |_| {
            let listed = srv.attached(10.0)?;
            let holder = parent(listed.pid).filter(|&pid| is_holder(pid)).ok_or("no holder")?;
            std::thread::sleep(Duration::from_secs_f64(srv.slow));
            kill(holder, "-KILL")
        })
        .dump_at(6.0)
        .run()?;
    ensure!(out.dump.tabs.len() == 1, "tabs: {:?}", out.dump.tabs);
    screen_has(&out, "before-2")?;
    screen_has(&out, "Session lost:")?;
    Ok(Outcome::Pass)
}

/// Brindle with no helper beside it shows the missing-helper error; a tab
/// opened once the helper is installed beside it runs in the server.
pub fn no_helper(ctx: &Ctx) -> Result<Outcome> {
    let tmux = server()?;
    let srv = Server::of(ctx);
    let bin_dir = ctx.dir.join("bin");
    std::fs::create_dir_all(&bin_dir).map_err(|e| e.to_string())?;
    let bin = bin_dir.join("brindle");
    link_or_copy(&ctx.bin, &bin)?;
    let installed = bin_dir.join("brindle-remote");
    let helper = ctx.helper.clone();
    let proof = ctx.dir.join("in-server");
    let out = Run::new(ctx, &tmux, "run").keep_detached()
        .bin(&bin)
        .env_remove("BRINDLE_REMOTE_AGENT")
        .profile("Shell")
        .default_profile("Shell")
        .wait_for("helper not found")
        .call_at(2.0, move |_| link_or_copy(&helper, &installed))
        .wait_for("helper not found")
        .wait_for("helper not found")
        .action("new_tab")
        .send(&format!("echo in-server > \"{}\"\r", proof.display()))
        .call_at(5.0, move |_| {
            let listed = srv.attached(10.0)?;
            let holder = parent(listed.pid).unwrap_or(0);
            ensure!(is_holder(holder), "the new tab's shell's parent {holder} isn't a holder");
            Ok(())
        })
        .action("prev_tab")
        .dump_at(9.0)
        .run()?;
    ensure!(out.dump.tabs.len() == 2, "tabs: {:?}", out.dump.tabs);
    screen_has(&out, "terminal server helper not found")?;
    screen_has(&out, "BRINDLE_REMOTE_AGENT")?;
    let wrote = std::fs::read_to_string(ctx.dir.join("in-server")).unwrap_or_default();
    ensure!(wrote.trim() == "in-server", "the second tab didn't run its command");
    Ok(Outcome::Pass)
}

/// A hard link to `from` at `to`, or a copy on another file system.
fn link_or_copy(from: &Path, to: &Path) -> Result {
    std::fs::hard_link(from, to)
        .or_else(|_| std::fs::copy(from, to).map(drop))
        .map_err(|e| format!("putting {} at {}: {e}", from.display(), to.display()))
}

/// A copy of the helper with another build id (a byte of its GNU build-id
/// note flipped) and a newer file time: what an install of a new build
/// leaves beside Brindle. Returns it and both build ids.
fn other_build(ctx: &Ctx) -> Result<(PathBuf, String, String)> {
    use brindle_remote_protocol::build_id;
    let mut bytes = std::fs::read(&ctx.helper).map_err(|e| format!("reading {}: {e}", ctx.helper.display()))?;
    let old = build_id::of_binary(&bytes);
    let id: Vec<u8> = (0..old.len()).step_by(2).filter_map(|at| u8::from_str_radix(&old[at..at + 2], 16).ok()).collect();
    let at = bytes.windows(id.len()).position(|window| window == id).ok_or("the helper has no GNU build-id note")?;
    bytes[at] ^= 0xff;
    let new = build_id::of_binary(&bytes);
    ensure!(new != old, "the patched build id is still {old}");
    let dir = ctx.dir.join("upgrade");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join("brindle-remote");
    std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
    let file = std::fs::File::options().write(true).open(&path).map_err(|e| e.to_string())?;
    // The cases run only on Linux, but the suite also compiles on Windows.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        file.set_permissions(std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    file.set_modified(SystemTime::now() + Duration::from_secs(60)).map_err(|e| e.to_string())?;
    Ok((path, old, new))
}

/// A second Brindle with a newer build of the helper upgrades the core: the
/// first Brindle's tab keeps its program and answers, and `status` shows
/// the new core and the old holder draining.
pub fn upgrade(ctx: &Ctx) -> Result<Outcome> {
    let tmux = server()?;
    let srv = Server::of(ctx);
    let (newer, old_build, new_build) = other_build(ctx)?;
    let report = ctx.dir.join("status.json");
    let saved = report.clone();
    let first = Run::new(ctx, &tmux, "first").keep_detached()
        .profile("Shell")
        .send("echo first=$$\r")
        .call_at(2.0, move |_| {
            let listed = srv.attached(10.0)?;
            let status = srv.status()?;
            ensure!(status["core_build"] == old_build.as_str(), "the first core's build: {status}");
            let status = srv.wait(30.0, "the upgraded core, with the old holder draining", || {
                let status = srv.status()?;
                let draining = status["holders"].as_array().is_some_and(|holders| {
                    holders.iter().any(|h| h["build"] == old_build.as_str() && h["draining"] == true)
                });
                Ok((status["core_build"] == new_build.as_str() && draining).then_some(status))
            })?;
            std::fs::write(&saved, status.to_string()).ok();
            srv.wait(20.0, "the first tab's session attached again", || {
                Ok(srv.list()?.into_iter().find(|s| s.attached && s.pid == listed.pid))
            })?;
            mark(listed.pid, "UPGRADED-MARK\r\n")
        })
        .wait_for("UPGRADED-MARK")
        .send("echo after-$((3+4))=$$\r")
        .wait_for("after-7")
        .dump_at(20.0);
    let second = Run::new(ctx, &tmux, "second").keep_detached()
        .profile("Shell")
        .env("BRINDLE_REMOTE_AGENT", newer.to_str().unwrap())
        .send("echo second-$((1+1))\r")
        .dump_at(6.0);
    let (first, second) = two_processes(ctx, 3.0, first, second)?;
    screen_has(&second, "second-2")?;
    let pid = first
        .dump
        .screen
        .iter()
        .find_map(|l| l.strip_prefix("first="))
        .ok_or_else(|| format!("no first= line: {:?}", first.dump.screen))?
        .to_string();
    screen_has(&first, &format!("after-7={pid}"))?;
    ensure!(report.exists(), "no status saved");
    Ok(Outcome::Pass)
}

/// `brindle-remote list` in a tab lists that tab's own session.
pub fn manage(ctx: &Ctx) -> Result<Outcome> {
    let tmux = server()?;
    let out = Run::new(ctx, &tmux, "run").keep_detached()
        .profile("Shell")
        .send(&format!("echo pid=$$; \"{}\" list\r", ctx.helper.display()))
        .dump_at(4.0)
        .run()?;
    let pid = out
        .dump
        .screen
        .iter()
        .find_map(|l| l.strip_prefix("pid="))
        .ok_or_else(|| format!("no pid= line: {:?}", out.dump.screen))?
        .to_string();
    let listed = out.dump.screen.iter().any(|l| {
        let fields: Vec<&str> = l.split_whitespace().collect();
        fields.len() >= 3 && fields[1] == pid && fields[2] == "attached"
    });
    ensure!(listed, "no attached session with pid {pid}: {:?}", out.dump.screen);
    Ok(Outcome::Pass)
}
