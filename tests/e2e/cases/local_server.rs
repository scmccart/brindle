//! Local tabs in the terminal server (Linux and Windows): where their
//! programs run, that closing ends them and quitting keeps them, reattaching
//! on the next launch, the sessions palette and `--attach`, the environment
//! they get, reconnecting after the core restarts, sessions lost for good, a
//! missing helper, an upgrade, and the management commands. Windows adds
//! the owner of a program's dialog, the inbox console host beside the
//! packaged one, and the elevated server.
//!
//! Every case has its own server directory (`ctx.server_dir`), which the
//! runner stops and checks afterwards. Checks that must happen while
//! Brindle runs are [`Run::call_at`] calls, which poll rather than trust
//! timing; a call that must hold a later step back writes a marker to the
//! tab's terminal (on Windows, creates a file the tab's program waits for),
//! which a [`Run::wait_for`] step waits for.
//!
//! The tabs run bash, or on Windows PowerShell 7 with the prompt
//! `PS> `; the helpers below give each command in both. On Windows,
//! `--send` turns `\t`, `\e`, `\r` and `\n` into controls, so paths in typed
//! commands use `/`, and PowerShell scripts go in files.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use super::multi_window::two_processes;
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
#[cfg(unix)]
fn stat(pid: u32) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, rest) = text.rsplit_once(") ")?;
    Some(rest.split(' ').map(str::to_string).collect())
}

/// Running, not a zombie.
#[cfg(unix)]
fn alive(pid: u32) -> bool {
    stat(pid).is_some_and(|fields| fields[0] != "Z")
}

#[cfg(windows)]
fn alive(pid: u32) -> bool {
    crate::winproc::alive(pid)
}

#[cfg(unix)]
fn parent(pid: u32) -> Option<u32> {
    stat(pid)?.get(1)?.parse().ok()
}

#[cfg(windows)]
fn parent(pid: u32) -> Option<u32> {
    crate::winproc::parent(pid)
}

#[cfg(unix)]
fn cmdline(pid: u32) -> Vec<String> {
    let bytes = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default();
    bytes.split(|&b| b == 0).filter(|a| !a.is_empty()).map(|a| String::from_utf8_lossy(a).into_owned()).collect()
}

/// `pid`'s command line, for messages.
#[cfg(unix)]
fn described(pid: u32) -> String {
    format!("{:?}", cmdline(pid))
}

#[cfg(windows)]
fn described(pid: u32) -> String {
    format!("{:?}", crate::winproc::command_line(pid))
}

/// `brindle-remote hold`, as the server starts it.
#[cfg(unix)]
fn is_holder(pid: u32) -> bool {
    cmdline(pid).get(1).is_some_and(|role| role == "hold")
}

/// `brindle-remote.exe hold …`, as the server starts it.
#[cfg(windows)]
fn is_holder(pid: u32) -> bool {
    crate::winproc::command_line(pid).as_deref().and_then(crate::winproc::first_argument) == Some("hold")
}

/// Ends `pid` at once: SIGKILL.
#[cfg(unix)]
fn kill(pid: u32) -> Result {
    let status = std::process::Command::new("kill").args(["-KILL", &pid.to_string()]).status();
    ensure!(status.is_ok_and(|s| s.success()), "kill -KILL {pid} failed");
    Ok(())
}

/// Ends `pid` at once: `taskkill /F`, which is `TerminateProcess`.
#[cfg(windows)]
fn kill(pid: u32) -> Result {
    let out = std::process::Command::new("taskkill")
        .args(["/F", "/PID", &pid.to_string()])
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|e| format!("taskkill: {e}"))?;
    ensure!(out.status.success(), "taskkill /F /PID {pid}: {}", String::from_utf8_lossy(&out.stderr).trim());
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

/// `path` as a typed command can name it: with `/` on Windows.
fn slashed(path: &Path) -> String {
    let text = path.display().to_string();
    if cfg!(windows) { text.replace('\\', "/") } else { text }
}

/// The command for bash or, on Windows, for PowerShell.
fn sh(unix: &str, windows: &str) -> String {
    if cfg!(windows) { windows } else { unix }.to_string()
}

/// A command that prints `text`, typed so its line doesn't show `text`.
fn echo(text: &str) -> String {
    let (head, tail) = text.split_at(1);
    sh(&format!("echo {text}"), &format!("'{head}' + '{tail}'"))
}

/// A command that prints `<prefix>-<a + b>`.
fn echo_sum(prefix: &str, a: u32, b: u32) -> String {
    sh(&format!("echo {prefix}-$(({a}+{b}))"), &format!("'{prefix}-' + ({a}+{b})"))
}

/// A command that prints `<name>=<the shell's pid>`.
fn echo_pid(name: &str) -> String {
    let (head, tail) = name.split_at(1);
    sh(&format!("echo {name}=$$"), &format!("'{head}' + \"{tail}=$PID\""))
}

/// A command that writes the shell's pid to `file`.
fn save_pid(file: &Path) -> String {
    sh(&format!("echo $$ > \"{}\"", file.display()), &format!("\"$PID\" > '{}'", slashed(file)))
}

/// Writes a PowerShell script to the case directory and gives the command
/// that runs it in a PowerShell of its own, a child of the tab's shell.
fn pwsh_script(ctx: &Ctx, name: &str, body: &str) -> Result<String> {
    let path = ctx.dir.join(name);
    std::fs::write(&path, body).map_err(|e| format!("writing {}: {e}", path.display()))?;
    Ok(format!("pwsh -NoLogo -NoProfile -File '{}'\r", slashed(&path)))
}

/// A command for the tab that runs a program, a child of the tab's shell,
/// that writes its pid to `file`, prints `<said>-42` (computed, so the
/// typed line doesn't show it) when `said` is given, and sleeps.
fn sleeper(ctx: &Ctx, file: &Path, said: Option<&str>) -> Result<String> {
    if cfg!(windows) {
        let say = said.map(|said| format!("'{said}-' + (40+2)\n")).unwrap_or_default();
        let name = format!("{}.ps1", file.file_name().unwrap_or_default().to_string_lossy());
        let body = format!("\"$PID\" | Set-Content -LiteralPath '{}'\n{say}Start-Sleep 1000\n", file.display());
        return pwsh_script(ctx, &name, &body);
    }
    Ok(match said {
        Some(said) => format!("sh -c 'echo $$ > \"{}\"; echo {said}-$((40+2)); exec sleep 1000'\r", file.display()),
        None => recorded(file, "sleep 1000"),
    })
}

/// The first line of the shell's prompt starts with this.
const PROMPT_START: &str = if cfg!(windows) { "PS>" } else { "bash-" };

/// Seconds for a dump: Windows' PowerShell is slower to start than bash.
fn later(secs: f64) -> f64 {
    if cfg!(windows) { secs + 3.0 } else { secs }
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
    let srv = Server::of(ctx);
    let found = ctx.dir.join("session-pid");
    let save = found.clone();
    let out = Run::new(ctx, "run")
        .keep_detached()
        .prompt()
        .send(&sh("echo shell=$$\r", "\"shell=$PID\"\r"))
        .call_at(2.0, move |_| {
            let listed = srv.attached(10.0)?;
            let sessions = srv.list()?;
            ensure!(sessions.len() == 1, "sessions: {sessions:?}");
            let holder = parent(listed.pid).ok_or("the shell has no parent")?;
            ensure!(is_holder(holder), "the shell's parent {holder} is {}, not a holder", described(holder));
            std::fs::write(&save, listed.pid.to_string()).map_err(|e| e.to_string())
        })
        .dump_at(later(5.0))
        .run()?;
    let pid = read_pid(&found).ok_or("the session wasn't checked")?;
    ensure!(out.dump.screen.iter().any(|l| l == &format!("shell={pid}")), "the tab's shell isn't {pid}: {:?}", out.dump.screen);
    Ok(Outcome::Pass)
}

/// Closing a tab running `sleep 1000` ends `sleep` and its session; the
/// other tab's stays attached.
pub fn close(ctx: &Ctx) -> Result<Outcome> {
    let srv = Server::of(ctx);
    let file = ctx.dir.join("sleep-pid");
    let watched = file.clone();
    let out = Run::new(ctx, "run")
        .keep_detached()
        .action("new_tab")
        .prompt()
        .send(&sleeper(ctx, &file, Some("sleeping"))?)
        .wait_for("sleeping-42")
        .action("close_tab")
        .call_at(1.0, move |brindle| {
            let pid = srv.wait(15.0, "sleep started", || Ok(read_pid(&watched)))?;
            gone(&srv, 10.0, &[pid])?;
            listed_when(&srv, 10.0, "the other tab's session alone", |s| s.len() == 1 && s[0].attached)?;
            ensure!(alive(brindle), "Brindle quit");
            Ok(())
        })
        .dump_at(later(10.0))
        .run()?;
    ensure!(out.dump.tabs.len() == 1, "tabs: {:?}", out.dump.tabs);
    ensure!(read_pid(&file).is_some(), "sleep never started");
    Ok(Outcome::Pass)
}

/// Starts `sleep 1000` in two tabs, recording their pids.
fn two_sleeps(ctx: &Ctx) -> Result<(Run<'_>, [PathBuf; 2])> {
    let files = [ctx.dir.join("sleep-1"), ctx.dir.join("sleep-2")];
    let run = Run::new(ctx, "run")
        .keep_detached()
        .prompt()
        .send(&sleeper(ctx, &files[0], None)?)
        .action("new_tab")
        .prompt()
        .send(&sleeper(ctx, &files[1], None)?);
    Ok((run, files))
}

fn wait_pids(srv: &Server, files: &[PathBuf; 2]) -> Result<[u32; 2]> {
    let first = srv.wait(15.0, "the first sleep", || Ok(read_pid(&files[0])))?;
    let second = srv.wait(15.0, "the second sleep", || Ok(read_pid(&files[1])))?;
    Ok([first, second])
}

/// Quitting Brindle leaves every tab's program running, detached.
pub fn quit(ctx: &Ctx) -> Result<Outcome> {
    let srv = Server::of(ctx);
    let (run, files) = two_sleeps(ctx)?;
    let checked = srv.clone();
    let watched = files.clone();
    run.call_at(1.0, move |_| {
        let pids = wait_pids(&checked, &watched)?;
        ensure!(pids.iter().all(|&pid| alive(pid)), "the sleeps {pids:?} aren't running");
        Ok(())
    })
    .dump_at(later(6.0))
    .run()?;
    let pids = wait_pids(&srv, &files)?;
    let detached = detached_pids(&srv, 10.0, 2)?;
    for pid in pids {
        ensure!(in_session(pid, &detached), "sleep {pid} isn't in a detached session's shell {detached:?}");
    }
    Ok(Outcome::Pass)
}

/// Brindle killed (SIGKILL, on Windows `taskkill /F`): its programs keep
/// running, detached.
pub fn crash(ctx: &Ctx) -> Result<Outcome> {
    let srv = Server::of(ctx);
    let (run, files) = two_sleeps(ctx)?;
    let checked = srv.clone();
    let watched = files.clone();
    run.call_at(1.0, move |brindle| {
        wait_pids(&checked, &watched)?;
        kill(brindle)
    })
    .dump_at(20.0)
    .expect_killed()
    .run()?;
    let pids = wait_pids(&srv, &files)?;
    let detached = detached_pids(&srv, 10.0, 2)?;
    for pid in pids {
        ensure!(in_session(pid, &detached), "sleep {pid} isn't in a detached session's shell {detached:?}");
    }
    Ok(Outcome::Pass)
}

/// Closing a zone ends its program; the tab keeps the other zone, whose
/// shell quitting then leaves detached.
pub fn close_zone(ctx: &Ctx) -> Result<Outcome> {
    let srv = Server::of(ctx);
    let file = ctx.dir.join("sleep-pid");
    let out = Run::new(ctx, "run")
        .keep_detached()
        .prompt()
        .send(&format!("{}\r", echo("left-zone")))
        .action("split_zone_right")
        .prompt()
        .send(&sleeper(ctx, &file, Some("zoned"))?)
        .wait_for("zoned-42")
        .action("close_zone")
        .dump_at(later(8.0))
        .run()?;
    ensure!(out.dump.zones.is_none(), "zones left: {:?}", out.dump.zones);
    screen_has(&out, "left-zone")?;
    let pid = read_pid(&file).ok_or("sleep never started")?;
    gone(&srv, 5.0, &[pid])?;
    detached_pids(&srv, 10.0, 1)?;
    Ok(Outcome::Pass)
}

/// Closing the window from the window manager (`WM_DELETE_WINDOW`; on
/// Windows `WM_CLOSE`) quits Brindle without asking and leaves the program
/// detached, still running.
pub fn close_window(ctx: &Ctx) -> Result<Outcome> {
    let srv = Server::of(ctx);
    let file = ctx.dir.join("sleep-pid");
    let watched = file.clone();
    let display = ctx.display.clone();
    Run::new(ctx, "run")
        .keep_detached()
        .prompt()
        .send(&sleeper(ctx, &file, Some("windowed"))?)
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
    let srv = Server::of(ctx);
    let file = ctx.dir.join("sleep-pid");
    let script = format!("echo $$ > \"{}\"; echo commanded-$((40+2)); exec sleep 1000", file.display());
    let ps1 = ctx.dir.join("commanded.ps1");
    let ps1_text = ps1.display().to_string();
    let command: Vec<&str> = if cfg!(windows) {
        let body = format!("\"$PID\" | Set-Content -LiteralPath '{}'\n'commanded-' + (40+2)\nStart-Sleep 1000\n", file.display());
        std::fs::write(&ps1, body).map_err(|e| e.to_string())?;
        vec!["pwsh.exe", "-NoLogo", "-NoProfile", "-File", &ps1_text]
    } else {
        vec!["sh", "-c", &script]
    };
    Run::new(ctx, "run")
        .keep_detached()
        .wait_for("commanded-42")
        .action("close_window")
        .expect_exit()
        .dump_at(later(10.0))
        .command(&command)
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
/// and its scrollback, and no bell or notification from the replay.
pub fn reattach_quit(ctx: &Ctx) -> Result<Outcome> {
    let notify = "\n[notifications]\nwhen = \"always\"\n";
    let (live_log, replay_log) = (ctx.dir.join("notify-first.log"), ctx.dir.join("notify-second.log"));
    let first = Run::new(ctx, "first")
        .keep_detached()
        .restore()
        .config(notify)
        .env("BRINDLE_NOTIFY_LOG", live_log.to_str().unwrap())
        .prompt()
        .send(&sh(
            r"printf '\a\033]9;Ping\a'; seq -f scroll-%g 1 300; echo first=$$\r",
            "[Console]::Write(\"`a`e]9;Ping`a\"); 1..300 | ForEach-Object { \"scroll-$_\" }; \"first=$PID\"\r",
        ))
        .wait_for("scroll-300")
        .dump_at(later(5.0))
        .run()?;
    let pid = shown_pid(&first, "first=")?;
    detached_pids(&Server::of(ctx), 10.0, 1)?;
    let again = ctx.dir.join("again-pid");
    let second = Run::new(ctx, "second")
        .keep_detached()
        .restore()
        .config(notify)
        .env("BRINDLE_NOTIFY_LOG", replay_log.to_str().unwrap())
        .send(&format!("{}; {}\r", save_pid(&again), echo_sum("again", 1, 1)))
        .wait_for("again-2")
        .action("scroll_to_top")
        .dump_at(later(6.0))
        .run()?;
    ensure!(second.dump.tabs.len() == 1, "tabs: {:?}", second.dump.tabs);
    ensure!(read_pid(&again) == Some(pid), "another shell: {:?}, not {pid}", read_pid(&again));
    // The top of the scrollback, which the replay brought back.
    ensure!(second.dump.screen.iter().any(|l| l == "scroll-1"), "no scroll-1 at the top: {:?}", second.dump.screen);
    let replayed = std::fs::read_to_string(&replay_log).unwrap_or_default();
    ensure!(replayed.is_empty(), "the replay notified: {replayed:?}");
    Ok(Outcome::Pass)
}

/// Brindle killed (SIGKILL, on Windows `taskkill /F`) with a split tab: a
/// plain launch brings back both zones' sessions, and the left one's output.
pub fn reattach_crash(ctx: &Ctx) -> Result<Outcome> {
    let srv = Server::of(ctx);
    let saved = ctx.dir.join("pids");
    let save = saved.clone();
    let checked = srv.clone();
    Run::new(ctx, "first")
        .keep_detached()
        .restore()
        .prompt()
        .send(&sh("seq -f scroll-%g 1 300\r", "1..300 | ForEach-Object { \"scroll-$_\" }\r"))
        .wait_for("scroll-300")
        .action("split_zone_right")
        .prompt()
        .send(&format!("{}\r", echo_sum("right", 1, 1)))
        .wait_for("right-2")
        .call_at(later(5.0), move |brindle| {
            let sessions = listed_when(&checked, 10.0, "two attached sessions", |s| {
                s.len() == 2 && s.iter().all(|s| s.attached)
            })?;
            // The state file is written within a second of a change.
            std::thread::sleep(Duration::from_secs_f64(3.0 * checked.slow));
            let mut pids: Vec<u32> = sessions.iter().map(|s| s.pid).collect();
            pids.sort();
            std::fs::write(&save, format!("{pids:?}")).map_err(|e| e.to_string())?;
            kill(brindle)
        })
        .dump_at(later(20.0))
        .expect_killed()
        .run()?;
    let before = std::fs::read_to_string(&saved).map_err(|_| "the sessions weren't saved")?;
    detached_pids(&srv, 10.0, 2)?;
    let checked = srv.clone();
    let after = ctx.dir.join("pids-after");
    let save = after.clone();
    let out = Run::new(ctx, "second")
        .keep_detached()
        .restore()
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
        .dump_at(later(6.0))
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
    let srv = Server::of(ctx);
    let first = Run::new(ctx, "first")
        .keep_detached()
        .restore()
        .prompt()
        .send(&format!("{}\r", echo_pid("left")))
        .wait_for("left=")
        .action("split_zone_right")
        .prompt()
        .send(&format!("{}\r", echo_pid("right")))
        .wait_for("right=")
        .dump_at(later(6.0))
        .run()?;
    let right = shown_pid(&first, "right=")?;
    let sessions = listed_when(&srv, 10.0, "two detached sessions", |s| s.len() == 2 && s.iter().all(|s| !s.attached))?;
    let killed = sessions.iter().find(|s| s.pid == right).ok_or("the right zone's session isn't listed")?.id;
    srv.command(&["kill", &killed.to_string()])?;
    gone(&srv, 5.0, &[right])?;
    let second = Run::new(ctx, "second")
        .keep_detached()
        .restore()
        .send(&format!("{}\r", echo_pid("again")))
        .wait_for("again=")
        .dump_at(later(5.0))
        .run()?;
    ensure!(second.dump.tabs.len() == 1, "tabs: {:?}", second.dump.tabs);
    ensure!(second.dump.zones.is_none(), "zones: {:?}", second.dump.zones);
    let left = shown_pid(&second, "again=")?;
    screen_has(&second, &format!("left={left}"))?;

    // Now the left zone's session goes too.
    let id = listed_when(&srv, 10.0, "the left session detached", |s| s.len() == 1 && !s[0].attached)?[0].id;
    srv.command(&["kill", &id.to_string()])?;
    gone(&srv, 5.0, &[left])?;
    let third = Run::new(ctx, "third")
        .restore()
        .prompt()
        .send(&format!("{}\r", echo_pid("third")))
        .wait_for("third=")
        .dump_at(later(4.0))
        .run()?;
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
    Run::new(ctx, "first")
        .keep_detached()
        .restore()
        .prompt()
        .send(&sh(r"clear; echo hidden-$((1+1))\r", "Clear-Host; 'hidden-' + (1+1)\r"))
        .wait_for("hidden-2")
        .action("new_tab")
        .prompt()
        .send(&format!("{}\r", echo("shown")))
        .dump_at(later(5.0))
        .run()?;
    detached_pids(&Server::of(ctx), 10.0, 2)?;
    let out = Run::new(ctx, "second").keep_detached().restore().action("prev_tab").dump_at(later(5.0)).run()?;
    ensure!(out.dump.tabs.len() == 2 && out.dump.tabs[0].1, "tabs: {:?}", out.dump.tabs);
    let screen = &out.dump.screen;
    let output = screen.iter().rposition(|l| l == "hidden-2").ok_or_else(|| format!("no output: {screen:?}"))?;
    let last = screen.iter().rposition(|l| !l.trim().is_empty()).unwrap_or(0);
    ensure!(
        last == output + 1 && screen[last].starts_with(PROMPT_START),
        "the prompt isn't right after the output: {screen:?}"
    );
    Ok(Outcome::Pass)
}

/// A tab closed before quitting doesn't come back: its program ended.
pub fn reattach_closed_tab(ctx: &Ctx) -> Result<Outcome> {
    Run::new(ctx, "first")
        .keep_detached()
        .restore()
        .prompt()
        .send(&format!("{}\r", echo("restored-tab")))
        .action("new_tab")
        .prompt()
        .send(&format!("{}\r", echo("closed-tab")))
        .action("close_tab")
        .dump_at(later(6.0))
        .run()?;
    detached_pids(&Server::of(ctx), 10.0, 1)?;
    let out = Run::new(ctx, "second").keep_detached().restore().dump_at(later(5.0)).run()?;
    ensure!(out.dump.tabs.len() == 1, "tabs: {:?}", out.dump.tabs);
    screen_has(&out, "restored-tab")?;
    Ok(Outcome::Pass)
}

/// `--no-restore` leaves the detached sessions alone, and the next launch
/// attaches them after the tabs it restores.
pub fn reattach_no_restore(ctx: &Ctx) -> Result<Outcome> {
    let srv = Server::of(ctx);
    Run::new(ctx, "first")
        .keep_detached()
        .restore()
        .prompt()
        .send(&format!("{}\r", echo_sum("left", 1, 1)))
        .wait_for("left-2")
        .dump_at(later(4.0))
        .run()?;
    let before = detached_pids(&srv, 10.0, 1)?;
    let checked = srv.clone();
    let out = Run::new(ctx, "second")
        .keep_detached()
        .prompt()
        .send(&format!("{}\r", echo("own-tab")))
        .call_at(3.0, move |_| {
            let sessions = listed_when(&checked, 10.0, "the new tab's session", |s| s.len() == 2)?;
            let detached: Vec<u32> = sessions.iter().filter(|s| !s.attached).map(|s| s.pid).collect();
            ensure!(detached == before, "detached {detached:?}, before {before:?}");
            Ok(())
        })
        .dump_at(later(5.0))
        .run()?;
    ensure!(out.dump.tabs.len() == 1, "tabs: {:?}", out.dump.tabs);
    let third = Run::new(ctx, "third").keep_detached().restore().action("next_tab").dump_at(later(5.0)).run()?;
    ensure!(third.dump.tabs.len() == 2 && third.dump.tabs[1].1, "tabs: {:?}", third.dump.tabs);
    screen_has(&third, "left-2")?;
    Ok(Outcome::Pass)
}

/// A debug run beside a running Brindle runs on its own: it attaches none
/// of the detached sessions.
pub fn reattach_second(ctx: &Ctx) -> Result<Outcome> {
    let srv = Server::of(ctx);
    Run::new(ctx, "quit").keep_detached().restore().prompt().dump_at(later(4.0)).run()?;
    let before = detached_pids(&srv, 10.0, 1)?;
    // The instance leaves them alone too (`--no-restore`).
    let first = Run::new(ctx, "first").keep_detached().dump_at(later(9.0));
    let checked = srv.clone();
    let second = Run::new(ctx, "second")
        .keep_detached()
        .restore()
        .call_at(2.0, move |_| {
            let sessions = listed_when(&checked, 10.0, "both Brindles' tabs", |s| s.len() == 3)?;
            let detached: Vec<u32> = sessions.iter().filter(|s| !s.attached).map(|s| s.pid).collect();
            ensure!(detached == before, "detached {detached:?}, before {before:?}");
            Ok(())
        })
        .dump_at(later(4.0));
    let (_, second) = two_processes(ctx, 3.0, first, second)?;
    ensure!(second.dump.tabs.len() == 1, "the second Brindle's tabs: {:?}", second.dump.tabs);
    Ok(Outcome::Pass)
}

/// The sessions palette lists detached sessions: ctrl-shift-delete kills
/// the selected (newest) one, enter attaches the next, and an empty list
/// says so.
pub fn palette(ctx: &Ctx) -> Result<Outcome> {
    let srv = Server::of(ctx);
    let [older, newer] = [ctx.dir.join("older-pid"), ctx.dir.join("newer-pid")];
    // Quitting leaves both tabs' shells detached.
    Run::new(ctx, "first")
        .keep_detached()
        .prompt()
        .send(&format!("{}; {}\r", save_pid(&older), echo_sum("older", 1, 1)))
        .wait_for("older-2")
        .action("new_tab")
        .prompt()
        .send(&format!("{}\r", save_pid(&newer)))
        .dump_at(later(5.0))
        .run()?;
    detached_pids(&srv, 10.0, 2)?;
    let out = Run::new(ctx, "run")
        .keep_detached()
        .prompt()
        .action("attach_session")
        .key("ctrl-shift-delete")
        .key("enter")
        .action("attach_session")
        .dump_at(later(14.0))
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
    Run::new(ctx, "first")
        .keep_detached()
        .prompt()
        .send(&format!("{}\r", echo("unflagged")))
        .action("new_tab")
        .prompt()
        .send(&format!("{}\r", echo_sum("flagged", 1, 1)))
        .wait_for("flagged-2")
        .dump_at(later(6.0))
        .run()?;
    let srv = Server::of(ctx);
    let sessions = listed_when(&srv, 10.0, "two detached sessions", |s| s.len() == 2 && s.iter().all(|s| !s.attached))?;
    // The new tab spawned after the first, so its session has the higher id.
    let id = sessions.iter().map(|s| s.id).max().ok_or("no sessions")?;
    let out = Run::new(ctx, "second").keep_detached().arg("--attach").arg(&id.to_string()).dump_at(later(5.0)).run()?;
    screen_has(&out, "flagged-2")?;
    let missing = Run::new(ctx, "missing")
        .keep_detached()
        .arg("--no-restore")
        .arg("--attach")
        .arg("999")
        .dump_at(later(4.0))
        .run()?;
    screen_has(&missing, "could not attach session 999")?;
    screen_has(&missing, "no session 999")?;
    Ok(Outcome::Pass)
}

/// A second Brindle with `FOO=2` gives its tabs `FOO=2`, though the first
/// Brindle (`FOO=1`) started the server.
pub fn env(ctx: &Ctx) -> Result<Outcome> {
    let shown = sh("echo foo-$FOO-x\r", "\"foo-$env:FOO-x\"\r");
    let first = Run::new(ctx, "first").keep_detached().env("FOO", "1").prompt().send(&shown).dump_at(later(8.0));
    let second = Run::new(ctx, "second").keep_detached().env("FOO", "2").prompt().send(&shown).dump_at(later(4.0));
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
    kill(old)?;
    srv.wait(20.0, "a new core", || Ok(srv.core().filter(|&pid| pid != old)))?;
    srv.wait(20.0, "the session attached again", || {
        Ok(srv.list()?.into_iter().find(|s| s.attached && s.pid == listed.pid))
    })?;
    Ok(())
}

/// The core SIGKILLed while the tab runs `less`: the tab gets its screen
/// back, `less` keeps its pid and answers keys (G shows the end).
pub fn core_restart(ctx: &Ctx) -> Result<Outcome> {
    if cfg!(windows) { core_restart_windows(ctx) } else { core_restart_less(ctx) }
}

/// [`core_restart`] with `less`, whose terminal the case writes its marker to.
fn core_restart_less(ctx: &Ctx) -> Result<Outcome> {
    let srv = Server::of(ctx);
    let text: String = (1..=99).map(|n| format!("line-{n:02}\n")).collect();
    let lines = ctx.dir.join("lines.txt");
    std::fs::write(&lines, text).map_err(|e| e.to_string())?;
    let file = ctx.dir.join("less-pid");
    let watched = file.clone();
    let out = Run::new(ctx, "run")
        .keep_detached()
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

/// [`core_restart`] on Windows, where the core is ended with
/// `TerminateProcess` and there is no `less`: a PowerShell script draws on
/// the alternate screen, keeps its pid, and answers a key. It waits for a
/// file the case creates once the session is attached again, then prints a
/// marker.
fn core_restart_windows(ctx: &Ctx) -> Result<Outcome> {
    let srv = Server::of(ctx);
    let (file, go) = (ctx.dir.join("program-pid"), ctx.dir.join("reconnected"));
    let script = pwsh_script(
        ctx,
        "alternate.ps1",
        &format!(
            r#"$e = [char]27
"$PID" | Set-Content -LiteralPath '{file}'
[Console]::Write("$e[?1049h$e[H")
1..20 | ForEach-Object {{ 'line-{{0:D2}}' -f $_ }}
while (-not (Test-Path -LiteralPath '{go}')) {{ Start-Sleep -Milliseconds 100 }}
'RECONNECTED-' + 'MARK'
$key = [Console]::ReadKey($true)
'key=' + $key.KeyChar
Start-Sleep 600
"#,
            file = file.display(),
            go = go.display(),
        ),
    )?;
    let watched = file.clone();
    let out = Run::new(ctx, "run")
        .keep_detached()
        .prompt()
        .send(&script)
        .wait_for("line-01")
        .call_at(2.0, move |_| {
            let program = srv.wait(15.0, "the program started", || Ok(read_pid(&watched)))?;
            let listed = srv.attached(10.0)?;
            // Let it draw before the core goes.
            std::thread::sleep(Duration::from_secs_f64(srv.slow));
            restart_core(&srv, &listed)?;
            ensure!(alive(program), "the program {program} is gone");
            std::fs::write(&go, "").map_err(|e| e.to_string())
        })
        .wait_for("RECONNECTED-MARK")
        .send("G")
        .wait_for("key=G")
        .dump_at(30.0)
        .run()?;
    screen_has(&out, "key=G")?;
    // The replay brought back what was drawn before the core went.
    screen_has(&out, "line-01")?;
    ensure!(out.dump.mode.contains("ALT_SCREEN"), "not on the alternate screen: {}", out.dump.mode);
    Ok(Outcome::Pass)
}

/// A bell, an OSC 9 and a `blocked` status since the last checkpoint
/// notify once, live, and not again when the core restarts and the tab
/// gets its replay; the status record is back.
pub fn replay_quiet(ctx: &Ctx) -> Result<Outcome> {
    let srv = Server::of(ctx);
    let log = ctx.dir.join("notify.log");
    let watched = log.clone();
    let blocked = r"\033]7501;state=blocked:kind=permission:app=e2e:msg=T2s/\033\\";
    let out = Run::new(ctx, "run")
        .keep_detached()
        .config("\n[notifications]\nwhen = \"always\"\n")
        .env("BRINDLE_NOTIFY_LOG", log.to_str().unwrap())
        .prompt()
        .send(&sh(
            &format!(r"printf '\a\033]9;Ping\a{blocked}'; cat\r"),
            "[Console]::Write(\"`a`e]9;Ping`a`e]7501;state=blocked:kind=permission:app=e2e:msg=T2s/`a\"); \
             $null = [Console]::ReadLine()\r",
        ))
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
        .dump_at(later(22.0))
        .run()?;
    ensure!(
        out.dump.status.iter().any(|s| s.starts_with("state=blocked")),
        "the replay didn't restore the status: {:?}",
        out.dump.status
    );
    Ok(Outcome::Pass)
}

/// The session's holder killed: the tab stays, with its screen and a
/// "session lost" line.
pub fn holder_lost(ctx: &Ctx) -> Result<Outcome> {
    let srv = Server::of(ctx);
    let out = Run::new(ctx, "run")
        .keep_detached()
        .prompt()
        .send(&format!("{}\r", echo_sum("before", 1, 1)))
        .wait_for("before-2")
        .call_at(later(2.0), move |_| {
            let listed = srv.attached(10.0)?;
            let holder = parent(listed.pid).filter(|&pid| is_holder(pid)).ok_or("no holder")?;
            std::thread::sleep(Duration::from_secs_f64(srv.slow));
            kill(holder)
        })
        .dump_at(later(6.0))
        .run()?;
    ensure!(out.dump.tabs.len() == 1, "tabs: {:?}", out.dump.tabs);
    screen_has(&out, "before-2")?;
    screen_has(&out, "Session lost:")?;
    Ok(Outcome::Pass)
}

/// Brindle with no helper beside it shows the missing-helper error; a tab
/// opened once the helper is installed beside it runs in the server. On
/// Windows, `BRINDLE_SERVER_HELPER` names the helper's place while it is
/// missing.
pub fn no_helper(ctx: &Ctx) -> Result<Outcome> {
    let srv = Server::of(ctx);
    let proof = ctx.dir.join("in-server");
    let run = if cfg!(windows) {
        let dir = ctx.dir.join("helper");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let missing = dir.join(ctx.helper.file_name().ok_or("the helper has no file name")?);
        let helper = ctx.helper.clone();
        let installed = missing.clone();
        Run::new(ctx, "run")
            .keep_detached()
            .env("BRINDLE_SERVER_HELPER", missing.to_str().unwrap())
            .wait_for("does not name a file")
            .call_at(3.0, move |_| link_or_copy(&helper, &installed))
            .wait_for("does not name a file")
            .wait_for("does not name a file")
    } else {
        let bin_dir = ctx.dir.join("bin");
        std::fs::create_dir_all(&bin_dir).map_err(|e| e.to_string())?;
        let bin = bin_dir.join("brindle");
        link_or_copy(&ctx.bin, &bin)?;
        let installed = bin_dir.join("brindle-remote");
        let helper = ctx.helper.clone();
        Run::new(ctx, "run")
            .keep_detached()
            .bin(&bin)
            .env_remove("BRINDLE_REMOTE_AGENT")
            .env_remove("BRINDLE_SERVER_HELPER")
            .wait_for("helper not found")
            .call_at(2.0, move |_| link_or_copy(&helper, &installed))
            .wait_for("helper not found")
            .wait_for("helper not found")
    };
    let out = run
        .action("new_tab")
        .prompt()
        .send(&sh(&format!("echo in-server > \"{}\"\r", proof.display()), &format!("'in-server' > '{}'\r", slashed(&proof))))
        .call_at(later(5.0), move |_| {
            let listed = srv.attached(10.0)?;
            let holder = parent(listed.pid).unwrap_or(0);
            ensure!(is_holder(holder), "the new tab's shell's parent {holder} isn't a holder");
            Ok(())
        })
        .action("prev_tab")
        .dump_at(later(9.0))
        .run()?;
    ensure!(out.dump.tabs.len() == 2, "tabs: {:?}", out.dump.tabs);
    if cfg!(windows) {
        screen_has(&out, "BRINDLE_SERVER_HELPER does not name a file")?;
    } else {
        screen_has(&out, "terminal server helper not found")?;
        screen_has(&out, "BRINDLE_SERVER_HELPER")?;
    }
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
fn other_build_elf(ctx: &Ctx) -> Result<(PathBuf, String, String)> {
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
    // The case runs only on Linux, but the suite also compiles on Windows.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        file.set_permissions(std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    }
    file.set_modified(SystemTime::now() + Duration::from_secs(60)).map_err(|e| e.to_string())?;
    Ok((path, old, new))
}

/// A copy of the helper with another build id and a newer file time: what
/// an install of a new build leaves beside Brindle. A PE file has no GNU
/// build-id note, so its build id is a hash of its bytes, and bytes appended
/// after its image change it without changing the program. Returns it and
/// both build ids.
fn other_build_pe(ctx: &Ctx) -> Result<(PathBuf, String, String)> {
    use brindle_remote_protocol::build_id;
    let mut bytes = std::fs::read(&ctx.helper).map_err(|e| format!("reading {}: {e}", ctx.helper.display()))?;
    let old = build_id::of_binary(&bytes);
    bytes.extend_from_slice(b"brindle-e2e: another build");
    let new = build_id::of_binary(&bytes);
    ensure!(new != old, "the appended build id is still {old}");
    let dir = ctx.dir.join("upgrade");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let path = dir.join(ctx.helper.file_name().ok_or("the helper has no file name")?);
    std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
    let file = std::fs::File::options().write(true).open(&path).map_err(|e| e.to_string())?;
    file.set_modified(SystemTime::now() + Duration::from_secs(60)).map_err(|e| e.to_string())?;
    Ok((path, old, new))
}

/// Waits until the server shows the core of `new_build`, with a holder of
/// `old_build` draining, saving that status to `saved`; then until the
/// session `listed` is attached again.
fn upgraded(srv: &Server, listed: &Listed, old_build: &str, new_build: &str, saved: &Path) -> Result {
    let status = srv.wait(30.0, "the upgraded core, with the old holder draining", || {
        let status = srv.status()?;
        let draining = status["holders"].as_array().is_some_and(|holders| {
            holders.iter().any(|h| h["build"] == old_build && h["draining"] == true)
        });
        Ok((status["core_build"] == new_build && draining).then_some(status))
    })?;
    std::fs::write(saved, status.to_string()).ok();
    srv.wait(20.0, "the first tab's session attached again", || {
        Ok(srv.list()?.into_iter().find(|s| s.attached && s.pid == listed.pid))
    })?;
    Ok(())
}

/// [`upgrade`] on Windows, where the case can't write to the tab's
/// terminal: the first tab runs a program that waits for a file the case
/// creates once the session is attached again.
fn upgrade_windows(ctx: &Ctx) -> Result<Outcome> {
    let srv = Server::of(ctx);
    let (newer, old_build, new_build) = other_build_pe(ctx)?;
    let report = ctx.dir.join("status.json");
    let saved = report.clone();
    let go = ctx.dir.join("upgraded");
    let waits = pwsh_script(
        ctx,
        "wait.ps1",
        &format!(
            "while (-not (Test-Path -LiteralPath '{}')) {{ Start-Sleep -Milliseconds 100 }}\n'UPGRADED-' + 'MARK'\n",
            go.display()
        ),
    )?;
    let first = Run::new(ctx, "first")
        .keep_detached()
        .prompt()
        .send(&format!("{}\r", echo_pid("first")))
        .send(&waits)
        .call_at(2.0, move |_| {
            let listed = srv.attached(10.0)?;
            let status = srv.status()?;
            ensure!(status["core_build"] == old_build.as_str(), "the first core's build: {status}");
            upgraded(&srv, &listed, &old_build, &new_build, &saved)?;
            std::fs::write(&go, "").map_err(|e| e.to_string())
        })
        .wait_for("UPGRADED-MARK")
        .send("'after-' + (3+4) + \"=$PID\"\r")
        .wait_for("after-7")
        .dump_at(30.0);
    let second = Run::new(ctx, "second")
        .keep_detached()
        .env("BRINDLE_SERVER_HELPER", newer.to_str().unwrap())
        .prompt()
        .send(&format!("{}\r", echo_sum("second", 1, 1)))
        .dump_at(later(6.0));
    let (first, second) = two_processes(ctx, 4.0, first, second)?;
    screen_has(&second, "second-2")?;
    let pid = shown_pid(&first, "first=")?;
    screen_has(&first, &format!("after-7={pid}"))?;
    ensure!(report.exists(), "no status saved");
    Ok(Outcome::Pass)
}

/// A second Brindle with a newer build of the helper upgrades the core: the
/// first Brindle's tab keeps its program and answers, and `status` shows
/// the new core and the old holder draining.
pub fn upgrade(ctx: &Ctx) -> Result<Outcome> {
    if cfg!(windows) {
        return upgrade_windows(ctx);
    }
    let srv = Server::of(ctx);
    let (newer, old_build, new_build) = other_build_elf(ctx)?;
    let report = ctx.dir.join("status.json");
    let saved = report.clone();
    let first = Run::new(ctx, "first")
        .keep_detached()
        .send("echo first=$$\r")
        .call_at(2.0, move |_| {
            let listed = srv.attached(10.0)?;
            let status = srv.status()?;
            ensure!(status["core_build"] == old_build.as_str(), "the first core's build: {status}");
            upgraded(&srv, &listed, &old_build, &new_build, &saved)?;
            mark(listed.pid, "UPGRADED-MARK\r\n")
        })
        .wait_for("UPGRADED-MARK")
        .send("echo after-$((3+4))=$$\r")
        .wait_for("after-7")
        .dump_at(20.0);
    let second = Run::new(ctx, "second")
        .keep_detached()
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
    let out = Run::new(ctx, "run")
        .keep_detached()
        .prompt()
        .send(&sh(
            &format!("echo pid=$$; \"{}\" list\r", ctx.helper.display()),
            &format!("\"pid=$PID\"; & '{}' list\r", slashed(&ctx.helper)),
        ))
        .dump_at(later(4.0))
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

/// A message box a program in a tab opens, owned by its console window, is
/// owned (through that window) by Brindle's window; and after a quit and a
/// relaunch that reattaches the session, by the new Brindle's window. The
/// case closes each box with a message to it, never with input.
#[cfg(windows)]
pub fn message_box(ctx: &Ctx) -> Result<Outcome> {
    let script = ctx.dir.join("box.ps1");
    std::fs::write(
        &script,
        r#"param([string]$Title)
if (-not ('BrindleE2e.Box' -as [type])) {
    Add-Type -Name Box -Namespace BrindleE2e -MemberDefinition @'
[DllImport("kernel32.dll")] public static extern System.IntPtr GetConsoleWindow();
[DllImport("user32.dll", CharSet = CharSet.Unicode)]
public static extern int MessageBoxW(System.IntPtr owner, string text, string caption, uint type);
'@
}
$answer = [BrindleE2e.Box]::MessageBoxW([BrindleE2e.Box]::GetConsoleWindow(), 'Owned by the console window', $Title, 0)
'box-closed-' + $Title + '=' + $answer
"#,
    )
    .map_err(|e| e.to_string())?;
    let show = |title: &str| format!("& '{}' {title}\r", slashed(&script));
    for (label, title) in [("first", "brindle-e2e-box-1"), ("second", "brindle-e2e-box-2")] {
        let chain = ctx.dir.join(format!("{label}-owners.txt"));
        let saved = chain.clone();
        let slow = ctx.secs(1.0);
        let out = Run::new(ctx, label)
            .keep_detached()
            .restore()
            .prompt()
            .send(&show(title))
            .call_at(2.0, move |brindle| check_box_owner(brindle, title, &saved, slow))
            .wait_for(&format!("box-closed-{title}="))
            .dump_at(later(14.0))
            .run()?;
        screen_has(&out, &format!("box-closed-{title}=1"))?;
    }
    // The second run reattached the first's session.
    detached_pids(&Server::of(ctx), 10.0, 1)?;
    Ok(Outcome::Pass)
}

/// Waits for the message box titled `title`, then for its owner chain to
/// reach Brindle's window (reparenting is asynchronous), saving the chain to
/// `saved`, and closes the box with `WM_CLOSE` whatever was found.
#[cfg(windows)]
fn check_box_owner(brindle: u32, title: &str, saved: &Path, slow: f64) -> Result {
    use crate::winproc::{owner, post_close, window_of, window_titled};
    let wait = |secs: f64| Instant::now() + Duration::from_secs_f64(secs * slow);
    let deadline = wait(20.0);
    let found = loop {
        if let Some(found) = window_titled(title) {
            break found;
        }
        ensure!(Instant::now() < deadline, "no message box {title:?} within 20 s");
        std::thread::sleep(Duration::from_millis(100));
    };
    let chain_of = |window| {
        let mut chain = vec![window];
        while let Some(next) = owner(*chain.last().unwrap()).filter(|next| !chain.contains(next)) {
            chain.push(next);
        }
        chain
    };
    let deadline = wait(10.0);
    let checked = loop {
        let main = window_of(brindle);
        let chain = chain_of(found);
        let text: Vec<String> = chain.iter().map(|w| format!("{w:#x}")).collect();
        std::fs::write(saved, format!("brindle {main:x?}\nbox and owners {}\n", text.join(" "))).ok();
        // The box's owner is the console window, whose owner is Brindle's.
        let owned = main.is_some_and(|main| chain[1..].contains(&main));
        if owned {
            break Ok(());
        }
        if Instant::now() >= deadline {
            break Err(format!(
                "the box's owners {} don't reach Brindle's window {main:x?} within 10 s",
                text.join(" → ")
            ));
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    post_close(found)?;
    checked
}

/// One holder runs a session on the inbox console host (`BRINDLE_CONPTY=inbox`
/// in its profile's environment) beside one on the packaged host, and both
/// answer.
#[cfg(windows)]
pub fn inbox_host(ctx: &Ctx) -> Result<Outcome> {
    let srv = Server::of(ctx);
    let hosts = ctx.dir.join("hosts.txt");
    let saved = hosts.clone();
    let inbox = "\n[[profiles]]\nname = \"Inbox\"\ncommand = \"pwsh.exe\"\n\
                 args = [\"-NoLogo\", \"-NoProfile\", \"-NoExit\", \"-Command\", \"function prompt { 'PS> ' }\"]\n\
                 env = { BRINDLE_CONPTY = \"inbox\" }\n";
    let out = Run::new(ctx, "run")
        .config(inbox)
        .prompt()
        .send(&format!("{}\r", echo("packaged-host")))
        // The test config's profiles are Shell, then this one.
        .action("new_tab_with_profile_2")
        .prompt()
        .send(&format!("{}\r", echo("inbox-host")))
        .call_at(later(4.0), move |_| {
            srv.wait(15.0, "one holder with an OpenConsole and a conhost", || {
                let holders: Vec<u32> =
                    crate::server_processes(&srv.dir).into_iter().filter(|(_, role)| role == "hold").map(|(pid, _)| pid).collect();
                let all = crate::winproc::all();
                let under: Vec<String> =
                    all.iter().filter(|p| holders.contains(&p.parent)).map(|p| p.name.clone()).collect();
                std::fs::write(&saved, format!("holders {holders:?}\n{}\n", under.join("\n"))).ok();
                let count = |name: &str| under.iter().filter(|n| n.eq_ignore_ascii_case(name)).count();
                ensure!(holders.len() == 1, "holders: {holders:?}");
                Ok((count("OpenConsole.exe") == 1 && count("conhost.exe") == 1).then_some(()))
            })
        })
        .dump_at(later(8.0))
        .run()?;
    ensure!(out.dump.tabs.len() == 2, "tabs: {:?}", out.dump.tabs);
    screen_has(&out, "inbox-host")?;
    let mut logs = String::new();
    for entry in std::fs::read_dir(&ctx.server_dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("holder-") && name.ends_with(".log") {
            logs.push_str(&std::fs::read_to_string(entry.path()).unwrap_or_default());
        }
    }
    ctx.save("holders.log", &logs);
    ensure!(logs.contains("conpty: sessions use ConPTY "), "no packaged-ConPTY line; see holders.log");
    ensure!(logs.matches("BRINDLE_CONPTY=inbox").count() == 1, "not one inbox line; see holders.log");
    Ok(Outcome::Pass)
}

/// An elevated Brindle and an unelevated one, with no `BRINDLE_SERVER_DIR`,
/// run separate servers: `run-elevated` and `run` under `%LOCALAPPDATA%\brindle`
/// (here a private `LOCALAPPDATA` in the case directory), each listing only
/// its own session. Needs an elevated runner whose desktop shell runs
/// unelevated (UAC on), whose token starts the unelevated Brindle; skipped
/// otherwise, since elevating would ask through UAC.
#[cfg(windows)]
pub fn elevated(ctx: &Ctx) -> Result<Outcome> {
    use brindle_remote_protocol::server_dir::resolve_windows;
    if !crate::winproc::elevated() {
        return Ok(Outcome::Skip(
            "not elevated: an elevated server beside an unelevated one needs an elevated runner \
             (elevating here would ask through UAC)"
                .into(),
        ));
    }
    let appdata = ctx.dir.join("appdata");
    std::fs::create_dir_all(&appdata).map_err(|e| e.to_string())?;
    let resolve = |elevated| resolve_windows(None, Some(appdata.clone().into_os_string()), elevated);
    let high = resolve(true).map_err(|e| e.to_string())?;
    let medium = resolve(false).map_err(|e| e.to_string())?;
    // Nothing may reach the user's own `%LOCALAPPDATA%\brindle`.
    ensure!(high.starts_with(&ctx.dir) && medium.starts_with(&ctx.dir), "{high:?} and {medium:?} aren't private");
    ensure!(high.ends_with(r"brindle\run-elevated") && medium.ends_with(r"brindle\run"), "{high:?}, {medium:?}");
    let result = elevated_pair(ctx, &appdata, &high, &medium);
    // Stop the elevated server (the unelevated run stopped its own) and end
    // anything left from either copy.
    for dir in [&high, &medium] {
        std::process::Command::new(&ctx.helper)
            .arg("stop")
            .env("BRINDLE_SERVER_DIR", dir)
            .stdin(std::process::Stdio::null())
            .output()
            .ok();
    }
    let copies = appdata.join("brindle").join("server");
    let left = || -> Vec<u32> {
        let name = ctx.helper.file_name().unwrap_or_default().to_string_lossy().into_owned();
        crate::winproc::all()
            .into_iter()
            .filter(|p| p.name.eq_ignore_ascii_case(&name))
            // However the image's path is spelled (`\\?\`, case).
            .filter(|p| crate::winproc::image(p.pid).is_some_and(|image| crate::under(&image, &copies)))
            .map(|p| p.pid)
            .collect()
    };
    let deadline = Instant::now() + Duration::from_secs(5);
    while !left().is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
    }
    let leftover = left();
    leftover.iter().for_each(|&pid| crate::winproc::kill(pid));
    let outcome = result?;
    ensure!(leftover.is_empty(), "left running after `brindle-remote stop`: {leftover:?}");
    Ok(outcome)
}

#[cfg(windows)]
fn elevated_pair(ctx: &Ctx, appdata: &Path, high: &Path, medium: &Path) -> Result<Outcome> {
    // The elevated Brindle: an empty BRINDLE_SERVER_DIR counts as unset.
    let out = Run::new(ctx, "elevated")
        .keep_detached()
        .env("BRINDLE_SERVER_DIR", "")
        .env("LOCALAPPDATA", appdata.to_str().unwrap())
        .prompt()
        .send(&format!("{}\r", echo("elevated-tab")))
        .dump_at(later(4.0))
        .run()?;
    screen_has(&out, "elevated-tab")?;
    let high_server = Server { dir: high.to_path_buf(), helper: ctx.helper.clone(), slow: ctx.secs(1.0) };
    // Not `detached_pids`: with `BRINDLE_SERVER_DIR` unset the copies the
    // roles run from are in `appdata\brindle\server`, shared by both
    // servers, not in the server directory's `bin`, so `Server::core` never
    // finds the core. `list` never starts one, so it is polled as it is.
    let elevated_sessions = high_server.wait(10.0, "1 detached session in the elevated server", || {
        let sessions = high_server.list()?;
        Ok((sessions.len() == 1 && !sessions[0].attached).then(|| vec![sessions[0].pid]))
    })?;

    // The unelevated one, through cmd, which sets its environment.
    let done = ctx.dir.join("unelevated-done");
    let (dump, log, listed) =
        (ctx.dir.join("unelevated.dump"), ctx.dir.join("unelevated.log"), ctx.dir.join("unelevated-list.txt"));
    let state = ctx.dir.join("unelevated-state");
    let script = ctx.dir.join("unelevated.cmd");
    let quote = |path: &Path| format!("\"{}\"", path.display());
    let body = [
        "@echo off".to_string(),
        "set \"BRINDLE_SERVER_DIR=\"".to_string(),
        format!("set \"LOCALAPPDATA={}\"", appdata.display()),
        format!("set \"BRINDLE_CONFIG={}\"", ctx.dir.join("config.toml").display()),
        format!("set \"BRINDLE_STATE_DIR={}\"", state.display()),
        // The elevated run's helper: this environment is the user's own,
        // not the test's.
        format!("set \"BRINDLE_SERVER_HELPER={}\"", ctx.helper.display()),
        format!(
            "{} -p Shell --no-restore --wait-for \"PS>\" --send \"'unelev' + 'ated-tab'\\r\" {} --dump-screen-after {} > {} 2> {}",
            quote(&ctx.bin),
            ctx.step_interval().join(" "),
            ctx.secs(later(4.0)),
            quote(&dump),
            quote(&log)
        ),
        format!("{} list > {} 2>&1", quote(&ctx.helper), quote(&listed)),
        // Only an unelevated client may stop the unelevated server.
        format!("{} stop > nul 2>&1", quote(&ctx.helper)),
        format!("echo done > {}", quote(&done)),
    ];
    std::fs::write(&script, body.join("\r\n") + "\r\n").map_err(|e| e.to_string())?;
    let line = format!("cmd.exe /d /c \"{}\"", script.display());
    let cmd = match crate::winproc::start_unelevated(&line) {
        Ok(pid) => pid,
        Err(why) => return Ok(Outcome::Skip(format!("can't run an unelevated Brindle beside: {why}"))),
    };
    let deadline = Instant::now() + Duration::from_secs_f64(ctx.secs(60.0));
    while !done.exists() {
        // The script writes `done` last, so a cmd gone without it stopped
        // early: most likely it couldn't write to the case directory.
        if !crate::winproc::alive(cmd) && !done.exists() {
            let present = |path: &Path| if path.exists() { "present" } else { "missing" };
            return Err(format!(
                "the unelevated script ended without finishing (unelevated.dump {}, unelevated.log {}, \
                 unelevated-list.txt {}); can the unelevated user write {}?",
                present(&dump),
                present(&log),
                present(&listed),
                ctx.dir.display()
            ));
        }
        if Instant::now() >= deadline {
            // Brindle and the helper commands are cmd's children.
            let children: Vec<u32> = crate::winproc::all()
                .into_iter()
                .filter(|p| p.parent == cmd)
                .map(|p| p.pid)
                .collect();
            children.into_iter().chain([cmd]).for_each(crate::winproc::kill);
            return Err("the unelevated run didn't finish; see unelevated.log".into());
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let screen = std::fs::read_to_string(&dump).unwrap_or_default();
    ensure!(screen.contains("unelevated-tab"), "the unelevated tab: see unelevated.dump");
    ensure!(medium.join("core.lock").exists(), "the unelevated Brindle didn't use {}", medium.display());
    let unelevated_sessions: Vec<u32> =
        parse_list(&std::fs::read_to_string(&listed).unwrap_or_default()).iter().map(|s| s.pid).collect();
    ensure!(unelevated_sessions.len() == 1, "the unelevated server lists {unelevated_sessions:?}");
    ensure!(
        !elevated_sessions.iter().any(|pid| unelevated_sessions.contains(pid)),
        "the servers share a session: {elevated_sessions:?}, {unelevated_sessions:?}"
    );
    let again = high_server.list()?;
    ensure!(again.len() == 1, "the elevated server lists {again:?}");
    Ok(Outcome::Pass)
}
