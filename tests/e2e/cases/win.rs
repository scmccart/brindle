//! Windows cases: local tabs through Brindle's own pseudoconsole. They run
//! PowerShell or cmd instead of a tmux server, and check the screen dump,
//! the log, the notification log and the console host process.

use std::io::Read as _;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};

use crate::brindle::Dump;
use crate::{Case, Ctx, Outcome, Result, ensure};

pub fn all() -> Vec<Case> {
    let display = |name, run| Case { name, display: true, run };
    vec![
        display("windows-conpty-host", conpty_host),
        display("windows-conpty-fallback", conpty_fallback),
        display("windows-passthrough", passthrough),
        display("windows-glyph-width", glyph_width),
        display("windows-close-mid-output", close_mid_output),
    ]
}

const CONFIG: &str = r##"default_profile = "cmd"
theme = "e2e"

[cursor]
blink = false

# Window focus varies on a desktop, so notifications don't depend on it.
[notifications]
when = "always"

[[profiles]]
name = "cmd"
command = "cmd.exe"
args = ["/D"]

[[profiles]]
name = "pwsh"
command = "pwsh.exe"
args = ["-NoLogo", "-NoProfile"]

[themes.e2e]
foreground = "#c0c0c0"
background = "#101418"
"##;

/// What one run left behind.
struct Ran {
    dump: Dump,
    log: String,
    /// What `during` returned.
    during: String,
}

/// Runs Brindle with `args` (and `--dump-screen-after dump_at`), calling
/// `during` with its pid after `during_at` seconds.
fn run(
    ctx: &Ctx,
    label: &str,
    args: &[&str],
    env: &[(&str, &str)],
    dump_at: f64,
    during: Option<(f64, &dyn Fn(u32) -> String)>,
) -> Result<Ran> {
    let config = ctx.dir.join("config.toml");
    std::fs::write(&config, CONFIG).map_err(|e| e.to_string())?;
    let log_path = ctx.dir.join(format!("{label}.log"));
    let log = std::fs::File::create(&log_path).map_err(|e| e.to_string())?;
    let mut child = Command::new(&ctx.bin)
        .args(args)
        .args(ctx.step_interval())
        .args(["--dump-screen-after", &ctx.secs(dump_at).to_string()])
        .env("BRINDLE_CONFIG", &config)
        .env("BRINDLE_STATE_DIR", ctx.dir.join("state"))
        .env("RUST_LOG", "brindle=info,warn")
        .envs(env.iter().copied())
        .stdout(Stdio::piped())
        .stderr(log)
        .spawn()
        .map_err(|e| format!("starting {}: {e}", ctx.bin.display()))?;
    let started = Instant::now();
    let mut seen = String::new();
    if let Some((at, during)) = during {
        let due = started + Duration::from_secs_f64(ctx.secs(at));
        std::thread::sleep(due.saturating_duration_since(Instant::now()));
        seen = during(child.id());
    }
    let deadline = started + Duration::from_secs_f64(ctx.secs(dump_at) + 30.0);
    while child.try_wait().map_err(|e| e.to_string())?.is_none() {
        if Instant::now() > deadline {
            child.kill().ok();
            return Err(format!("Brindle didn't exit; see {label}.log"));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let mut stdout = String::new();
    child.stdout.take().unwrap().read_to_string(&mut stdout).ok();
    ctx.save(&format!("{label}.dump"), &stdout);
    let log = std::fs::read_to_string(&log_path).unwrap_or_default();
    Ok(Ran { dump: Dump::parse(&stdout), log, during: seen })
}

/// The executable names of `pid`'s direct children.
fn children(pid: u32) -> Vec<String> {
    let mut names = Vec::new();
    // SAFETY: a process snapshot, walked with a correctly sized entry and
    // closed afterwards.
    unsafe {
        let Ok(snapshot) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return names };
        let mut entry = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut more = Process32FirstW(snapshot, &mut entry).is_ok();
        while more {
            if entry.th32ParentProcessID == pid {
                let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
                names.push(String::from_utf16_lossy(&entry.szExeFile[..len]));
            }
            more = Process32NextW(snapshot, &mut entry).is_ok();
        }
        CloseHandle(snapshot).ok();
    }
    names
}

fn host_children(pid: u32) -> String {
    children(pid).join(",")
}

fn screen(dump: &Dump) -> String {
    dump.screen.join("\n")
}

/// A PowerShell script in the case directory, run in a pwsh tab.
fn script(ctx: &Ctx, name: &str, body: &str) -> Result<String> {
    let path = ctx.dir.join(name);
    std::fs::write(&path, body).map_err(|e| e.to_string())?;
    // Forward slashes: `--send` turns `\t`, `\e` and so on into controls.
    Ok(format!("& '{}'\r", path.display().to_string().replace('\\', "/")))
}

fn conpty_host(ctx: &Ctx) -> Result<Outcome> {
    let ran = run(ctx, "host", &["-p", "cmd", "--send", "echo hello-host\r"], &[], 4.0, Some((2.5, &host_children)))?;
    ensure!(ran.during.split(',').any(|n| n == "OpenConsole.exe"), "console hosts under Brindle: {}", ran.during);
    ensure!(!ran.during.split(',').any(|n| n == "conhost.exe"), "a conhost under Brindle: {}", ran.during);
    ensure!(ran.log.contains("local tabs use ConPTY"), "no packaged-ConPTY line in host.log");
    ensure!(screen(&ran.dump).contains("hello-host"), "screen:\n{}", screen(&ran.dump));
    Ok(Outcome::Pass)
}

fn conpty_fallback(ctx: &Ctx) -> Result<Outcome> {
    let ran = run(
        ctx,
        "fallback",
        &["-p", "cmd", "--send", "echo hello-inbox\r", "--action", "new_tab"],
        &[("BRINDLE_CONPTY", "inbox")],
        5.0,
        Some((3.5, &host_children)),
    )?;
    ensure!(ran.during.split(',').any(|n| n == "conhost.exe"), "console hosts under Brindle: {}", ran.during);
    ensure!(!ran.during.split(',').any(|n| n == "OpenConsole.exe"), "OpenConsole under Brindle: {}", ran.during);
    let warnings = ran.log.matches("BRINDLE_CONPTY=inbox").count();
    ensure!(warnings == 1, "{warnings} fallback warnings for two tabs; see fallback.log");
    ensure!(ran.dump.tabs.len() == 2, "tabs: {:?}", ran.dump.tabs);
    Ok(Outcome::Pass)
}

fn passthrough(ctx: &Ctx) -> Result<Outcome> {
    let notes = ctx.dir.join("notify.log");
    let send = script(
        ctx,
        "passthrough.ps1",
        r#"$e = [char]27; $b = [char]7
[Console]::Write("building`n")
[Console]::Write("$e]9;e2e-note$b")
[Console]::Write("done`n")
[Console]::Write("$e[?1000h$e[?1006h")
[Console]::Write("$e]11;?$b")
Start-Sleep -Milliseconds 500
$reply = ''
while ([Console]::KeyAvailable) { $reply += [Console]::ReadKey($true).KeyChar }
"reply=" + ($reply -replace $e, 'ESC' -replace $b, 'BEL')
"#,
    )?;
    let notes_env = notes.display().to_string();
    let ran = run(ctx, "passthrough", &["-p", "pwsh", "--send", &send], &[("BRINDLE_NOTIFY_LOG", &notes_env)], 6.0, None)?;
    let text = screen(&ran.dump);
    ensure!(text.contains("building") && text.contains("done"), "screen:\n{text}");
    // Brindle answers OSC 11 with the e2e theme's background (#101418).
    ensure!(text.contains("reply=ESC]11;rgb:1010/1414/1818"), "screen:\n{text}");
    ensure!(ran.dump.mode.contains("MOUSE_REPORT_CLICK") && ran.dump.mode.contains("SGR_MOUSE"), "mode: {}", ran.dump.mode);
    let noted = std::fs::read_to_string(&notes).unwrap_or_default();
    ensure!(noted.contains("e2e-note"), "notifications: {noted:?}");
    Ok(Outcome::Pass)
}

fn glyph_width(ctx: &Ctx) -> Result<Outcome> {
    let send = script(
        ctx,
        "width.ps1",
        "[Console]::OutputEncoding = [Text.Encoding]::UTF8\n\
         [Console]::Write(\"`r$([char]0x2764)$([char]0xFE0F)\"); $heart = [Console]::CursorLeft\n\
         [Console]::Write(\"`re$([char]0x301)\"); $accent = [Console]::CursorLeft\n\
         \"`nwidths=$heart,$accent\"\n",
    )?;
    let ran = run(ctx, "width", &["-p", "pwsh", "--send", &send], &[], 5.0, None)?;
    let text = screen(&ran.dump);
    ensure!(text.contains("widths=1,1"), "screen:\n{text}");
    Ok(Outcome::Pass)
}

fn close_mid_output(ctx: &Ctx) -> Result<Outcome> {
    let started = Instant::now();
    let ran = run(
        ctx,
        "close",
        &[
            "-p",
            "cmd",
            "--send",
            "for /L %i in (1,0,2) do @echo flood %i\r",
            "--action",
            "new_tab",
            "--action",
            "activate_tab_1",
            "--action",
            "close_tab",
        ],
        &[],
        6.0,
        None,
    )?;
    ensure!(ran.dump.tabs.len() == 1, "tabs: {:?}", ran.dump.tabs);
    // The dump comes on time only if closing the flooding tab didn't block.
    let took = started.elapsed().as_secs_f64();
    ensure!(took < ctx.secs(6.0) + 5.0, "took {took:.1} s");
    Ok(Outcome::Pass)
}
