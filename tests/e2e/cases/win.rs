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
        display("windows-title-child", title_child),
        display("windows-pwsh-integration", pwsh_integration),
        display("windows-caption", caption),
        display("windows-wsl-tmux", wsl_tmux),
    ]
}

/// A control-mode profile through WSL attaches a throwaway tmux server
/// inside it, with each tmux window a tab. Skipped without tmux in WSL.
fn wsl_tmux(ctx: &Ctx) -> Result<Outcome> {
    let wsl = |args: &[&str]| Command::new("wsl").args(args).stdin(Stdio::null()).output();
    match wsl(&["--exec", "tmux", "-V"]) {
        Ok(out) if out.status.success() => {}
        _ => return Ok(Outcome::Skip("no tmux in WSL (`wsl --exec tmux -V` failed)".into())),
    }
    let socket = format!("brindle-e2e-{}", std::process::id());
    let tmux = |args: &[&str]| {
        let mut all = vec!["--exec", "tmux", "-L", socket.as_str(), "-f", "/dev/null"];
        all.extend_from_slice(args);
        wsl(&all)
    };
    tmux(&["new-session", "-d", "-s", "main", "-n", "first"]).map_err(|e| e.to_string())?;
    tmux(&["new-window", "-d", "-t", "main", "-n", "second"]).map_err(|e| e.to_string())?;
    let config = format!(
        "default_profile = \"wsl\"\n\n[[profiles]]\nname = \"wsl\"\ntmux = \"control\"\ncommand = \"wsl\"\n\
         tmux_args = [\"tmux\", \"-L\", \"{socket}\", \"-f\", \"/dev/null\"]\n"
    );
    let ran = run_with(ctx, &config, "attach", &["-p", "wsl"], &[], 6.0, None);
    tmux(&["kill-server"]).ok();
    let ran = ran?;
    let titles: Vec<&str> = ran.dump.tabs.iter().map(|(t, _)| t.as_str()).collect();
    ensure!(titles == ["first", "second"], "tabs: {titles:?}; see attach.log");
    Ok(Outcome::Pass)
}

/// What Windows' hit test (`WM_NCHITTEST`) reports at points of the tab
/// strip, given in logical pixels from the client area's top-left (negative
/// x from its right) and the window's `scale`, then what Windows does with a
/// double-click on the `strip` point and a press on the `max` point. GPUI
/// answers for the last mouse position, so each point is first sent to the
/// window as a `WM_MOUSEMOVE`. These are messages to Brindle's own window,
/// not input, so the cursor and other windows are untouched.
fn caption_probe(pid: u32, scale: f32, points: &[(&str, f32, f32)]) -> String {
    use windows::Win32::Foundation::{HWND, LPARAM, RECT, WPARAM};
    use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetClientRect, GetWindowRect, GetWindowThreadProcessId, HTCAPTION, HTMAXBUTTON, IsWindowVisible,
        IsZoomed, SendMessageW, WM_MOUSEMOVE, WM_NCHITTEST, WM_NCLBUTTONDBLCLK, WM_NCLBUTTONDOWN, WM_NCLBUTTONUP,
    };
    use windows::core::BOOL;

    struct Found {
        pid: u32,
        hwnd: Option<HWND>,
    }
    unsafe extern "system" fn each(hwnd: HWND, found: LPARAM) -> BOOL {
        // SAFETY: `found` is the `Found` passed to EnumWindows below.
        let found = unsafe { &mut *(found.0 as *mut Found) };
        let mut pid = 0;
        // SAFETY: plain queries about a window that exists.
        unsafe {
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            if pid == found.pid && IsWindowVisible(hwnd).as_bool() {
                found.hwnd = Some(hwnd);
                return BOOL(0);
            }
        }
        BOOL(1)
    }
    // Physical pixels, as Brindle's window sees them: a DPI-unaware thread
    // gets its window rectangles scaled down.
    // SAFETY: changes only this thread's DPI awareness.
    unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    let mut found = Found { pid, hwnd: None };
    // SAFETY: `found` outlives the enumeration.
    unsafe { EnumWindows(Some(each), LPARAM(&mut found as *mut Found as isize)) }.ok();
    let Some(hwnd) = found.hwnd else { return "no window".into() };
    let pack = |x: i32, y: i32| LPARAM(((y as u16 as isize) << 16) | (x as u16 as isize));
    // A point as the client coordinates WM_MOUSEMOVE takes and the screen
    // coordinates the caption messages take, from the window as it is now.
    let at = |name: &str| {
        let &(_, x, y) = points.iter().find(|p| p.0 == name).expect("a named point");
        let (mut window, mut client) = (RECT::default(), RECT::default());
        // SAFETY: plain queries.
        unsafe {
            GetWindowRect(hwnd, &mut window).ok();
            GetClientRect(hwnd, &mut client).ok();
        }
        // Any frame around the client area is equal at the sides and bottom
        // (and at the top too when maximized).
        let side = ((window.right - window.left) - client.right) / 2;
        let top = (window.bottom - window.top) - client.bottom - side;
        let x = (if x < 0.0 { client.right as f32 + x * scale } else { x * scale }) as i32;
        let y = (y * scale) as i32;
        (pack(x, y), pack(window.left + side + x, window.top + top + y))
    };
    // SAFETY: messages to Brindle's own window, with plain values.
    let send = |msg, wparam: u32, lparam| unsafe { SendMessageW(hwnd, msg, Some(WPARAM(wparam as usize)), Some(lparam)) };
    let mut out: Vec<String> = points
        .iter()
        .map(|&(name, ..)| {
            let (client, screen) = at(name);
            send(WM_MOUSEMOVE, 0, client);
            format!("{name}={}", send(WM_NCHITTEST, 0, screen).0)
        })
        .collect();
    // The caption buttons act through ShowWindowAsync, so wait a little.
    let zoomed = |want: bool| {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            // SAFETY: a plain query.
            let zoomed = unsafe { IsZoomed(hwnd) }.as_bool();
            if zoomed == want || Instant::now() > deadline {
                return if zoomed { "maximized" } else { "windowed" };
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    let (client, screen) = at("strip");
    send(WM_MOUSEMOVE, 0, client);
    send(WM_NCLBUTTONDBLCLK, HTCAPTION, screen);
    out.push(format!("double-click={}", zoomed(true)));
    let (client, screen) = at("max");
    send(WM_MOUSEMOVE, 0, client);
    send(WM_NCLBUTTONDOWN, HTMAXBUTTON, screen);
    send(WM_NCLBUTTONUP, HTMAXBUTTON, screen);
    out.push(format!("maximize={}", zoomed(false)));
    out.join(" ")
}

/// The tab strip acts as a native caption: the empty strip is the caption
/// (drag, double-click, menu, Snap), the three buttons are Windows' own,
/// and tabs stay client area. Windows acts on a double-click on the caption
/// (maximizing) and a press on maximize (restoring), which it does only when
/// Brindle leaves the press unhandled.
fn caption(ctx: &Ctx) -> Result<Outcome> {
    // Logical pixels: the strip is 38 high; the buttons are 26 wide, 4 apart,
    // 8 from the right edge, and centered in it; tabs sit at the bottom.
    let points = [
        ("close", -(8.0 + 13.0), 19.0),
        ("max", -(8.0 + 26.0 + 4.0 + 13.0), 19.0),
        ("min", -(8.0 + 2.0 * (26.0 + 4.0) + 13.0), 19.0),
        ("strip", -(8.0 + 3.0 * 26.0 + 2.0 * 4.0 + 60.0), 19.0),
        ("tab", 48.0, 23.0),
    ];
    // The scale comes from a first run's dump.
    let first = run(ctx, "scale", &["-p", "cmd"], &[], 2.0, None)?;
    let scale = first.dump.grid.as_ref().map(|g| g.scale).ok_or("no grid in the dump")?;
    let probe = move |pid| caption_probe(pid, scale, &points);
    let ran = run(ctx, "caption", &["-p", "cmd"], &[], 7.0, Some((2.5, &probe)))?;
    // HTCLOSE, HTMAXBUTTON, HTMINBUTTON, HTCAPTION, HTCLIENT.
    let expected = "close=20 max=9 min=8 strip=2 tab=1 double-click=maximized maximize=windowed";
    ensure!(ran.during == expected, "caption at scale {scale}: {} (want {expected})", ran.during);
    Ok(Outcome::Pass)
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
fn run(ctx: &Ctx, label: &str, args: &[&str], env: &[(&str, &str)], dump_at: f64, during: During) -> Result<Ran> {
    run_with(ctx, CONFIG, label, args, env, dump_at, during)
}

type During<'a> = Option<(f64, &'a dyn Fn(u32) -> String)>;

/// [`run`] with another config.
fn run_with(
    ctx: &Ctx,
    config_text: &str,
    label: &str,
    args: &[&str],
    env: &[(&str, &str)],
    dump_at: f64,
    during: During,
) -> Result<Ran> {
    let config = ctx.dir.join("config.toml");
    std::fs::write(&config, config_text).map_err(|e| e.to_string())?;
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

/// Under the console's default title, the program's path, the tab is named
/// after the newest process under its program: `PING`, as `PING.EXE` is
/// named on disk, then `pwsh` once ping exits. Elevated PowerShell (as on
/// CI) sets a title of its own, so the default is set first.
fn title_child(ctx: &Ctx) -> Result<Outcome> {
    let title = |dump: &Dump| dump.tabs.first().map(|(t, _)| t.clone()).unwrap_or_default();
    let ping = |count: u32| format!("[Console]::Title = (Get-Process -Id $PID).Path; ping -n {count} 127.0.0.1\r");
    let during = run(ctx, "during", &["-p", "pwsh", "--send", &ping(8)], &[], 4.0, None)?;
    ensure!(title(&during.dump) == "PING", "title while ping runs: {:?}", title(&during.dump));
    let after = run(ctx, "after", &["-p", "pwsh", "--send", &ping(1)], &[], 5.0, None)?;
    ensure!(title(&after.dump) == "pwsh", "title after ping: {:?}", title(&after.dump));
    Ok(Outcome::Pass)
}

/// Brindle's PowerShell script in PowerShell 7 and 5.1, under a prompt the
/// user defined: the prompt still draws, and the directory the script
/// reports is where a new tab, and a restored one, starts. (The marks and
/// `$LASTEXITCODE` are checked byte for byte by a unit test in conpty.rs.)
fn pwsh_integration(ctx: &Ctx) -> Result<Outcome> {
    // The README's line, run by the brindle.exe under test.
    let bin = ctx.bin.display().to_string().replace('\\', "/");
    let load = format!("& '{bin}' --shell-integration pwsh | Out-String | Invoke-Expression");
    for shell in ["pwsh.exe", "powershell.exe"] {
        let config = format!(
            "default_profile = \"si\"\nrestore_session = true\n\n[[profiles]]\nname = \"si\"\ncommand = \"{shell}\"\n\
             args = [\"-NoLogo\", \"-NoProfile\", \"-NoExit\", \"-Command\", \"function prompt {{ 'MYPROMPT> ' }}; {load}\"]\n"
        );
        let label = shell.trim_end_matches(".exe");
        // New tab: inherits the reported directory.
        let ran = run_with(
            ctx,
            &config,
            label,
            &[
                "--send",
                "cd C:\\Windows\r",
                "--action",
                "new_tab",
                "--send",
                "\"dir=$PWD\"\r",
            ],
            &[],
            9.0,
            None,
        )?;
        let text = screen(&ran.dump);
        ensure!(ran.dump.tabs.len() == 2, "{shell}: tabs {:?}", ran.dump.tabs);
        ensure!(text.contains("MYPROMPT> "), "{shell}: the user's prompt is gone:\n{text}");
        ensure!(text.contains("dir=C:\\Windows"), "{shell}: the new tab's directory:\n{text}");
        // Restore: the session written on quit reopens both tabs in C:\Windows.
        let restored = run_with(ctx, &config, &format!("{label}-restored"), &["--send", "\"dir=$PWD\"\r"], &[], 5.0, None)?;
        let text = screen(&restored.dump);
        ensure!(text.contains("dir=C:\\Windows"), "{shell}: the restored tab's directory:\n{text}");
        std::fs::remove_dir_all(ctx.dir.join("state")).ok();
    }
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
