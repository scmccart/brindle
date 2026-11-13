//! Windows cases: local tabs in the case's own terminal server, on its
//! holder's pseudoconsoles. They run PowerShell or cmd instead of a tmux
//! server, and check the screen dump, Brindle's and the holder's logs, the
//! notification log and the console host process.

use std::io::Read as _;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::brindle::Dump;
use crate::{Case, Ctx, Outcome, Result, ensure, winproc};

pub fn all() -> Vec<Case> {
    let display = |name, run| Case { name, display: true, run };
    vec![
        display("windows-conpty-host", conpty_host),
        display("windows-conpty-fallback", conpty_fallback),
        display("windows-passthrough", passthrough),
        display("windows-glyph-width", glyph_width),
        display("windows-close-mid-output", close_mid_output),
        display("windows-flood-responsive", flood_responsive),
        display("windows-title-child", title_child),
        display("windows-pwsh-integration", pwsh_integration),
        display("windows-caption", caption),
        display("windows-wsl-tmux", wsl_tmux),
        display("windows-zones", zones),
        display("remote-container-exec", super::remote::container_exec),
        display("remote-ssh-container", super::remote::ssh_container),
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
    let ran = run_with(
        ctx,
        &config,
        "attach",
        &["-p", "wsl", "--action", "rename_tab", "--send", "  review  ", "--action", "picker_confirm"],
        &[],
        8.0,
        None,
    );
    let windows = tmux(&["list-windows", "-t", "main", "-F", "#{window_name}"]);
    tmux(&["kill-server"]).ok();
    let ran = ran?;
    let titles: Vec<&str> = ran.dump.tabs.iter().map(|(t, _)| t.as_str()).collect();
    ensure!(titles == ["review", "second"], "tabs: {titles:?}; see attach.log");
    let windows = String::from_utf8_lossy(&windows.map_err(|e| e.to_string())?.stdout).into_owned();
    ensure!(windows.lines().collect::<Vec<_>>() == ["first", "second"], "tmux windows: {windows:?}");
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
    // Logical pixels: the strip is 37 high (the 36px pill tab list and a
    // 1px border); the caption buttons are 32 wide (the `IconSm` button
    // size), 4 apart, 8 from the right edge, and centered in the list's
    // 36px, so empty space ends 112 from the right. Tabs are 3 below the
    // top, 30 high, 11 in (the strip's and the pill's padding), at least 80
    // wide (a `cmd` tab is exactly that), with the pill's 3px after the
    // last; the new-tab button is 4 past the pill, 32 wide.
    let points = [
        ("close", -(8.0 + 16.0), 18.0),
        ("max", -(8.0 + 32.0 + 4.0 + 16.0), 18.0),
        ("min", -(8.0 + 2.0 * (32.0 + 4.0) + 16.0), 18.0),
        ("strip", -(8.0 + 3.0 * 32.0 + 2.0 * 4.0 + 60.0), 18.0),
        ("tab", 48.0, 18.0),
        // The new-tab button: client area, a press on it is Brindle's.
        ("new-tab", 8.0 + 3.0 + 80.0 + 3.0 + 4.0 + 16.0, 18.0),
    ];
    // The scale comes from a first run's dump.
    let first = run(ctx, "scale", &["-p", "cmd"], &[], 2.0, None)?;
    let scale = first.dump.grid.as_ref().map(|g| g.scale).ok_or("no grid in the dump")?;
    let probe = move |pid| caption_probe(pid, scale, &points);
    let ran = run(ctx, "caption", &["-p", "cmd"], &[], 7.0, Some((2.5, &probe)))?;
    // HTCLOSE, HTMAXBUTTON, HTMINBUTTON, HTCAPTION, HTCLIENT, HTCLIENT.
    let expected = "close=20 max=9 min=8 strip=2 tab=1 new-tab=1 double-click=maximized maximize=windowed";
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

/// [`run`] with another config. The sessions the run leaves detached are
/// hung up, so the case's next run starts with only the tabs it asks for.
fn run_with(
    ctx: &Ctx,
    config_text: &str,
    label: &str,
    args: &[&str],
    env: &[(&str, &str)],
    dump_at: f64,
    during: During,
) -> Result<Ran> {
    let ran = run_keeping(ctx, config_text, label, args, env, dump_at, during)?;
    crate::brindle::hang_up_detached(ctx)?;
    Ok(ran)
}

/// [`run_with`], leaving the sessions the run leaves detached (a quit
/// leaves every profile tab's) for the next run to attach.
fn run_keeping(
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
    // `-p` alone restores the previous run's tabs too; such runs start clean.
    let no_restore: &[&str] = if args.contains(&"-p") { &["--no-restore"] } else { &[] };
    let mut child = Command::new(&ctx.bin)
        .args(args)
        .args(no_restore)
        .args(ctx.step_interval())
        .args(["--dump-screen-after", &ctx.secs(dump_at).to_string()])
        .env("BRINDLE_CONFIG", &config)
        .env("BRINDLE_STATE_DIR", ctx.dir.join("state"))
        .env("BRINDLE_SERVER_DIR", &ctx.server_dir)
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

/// The executable names of the processes the case's holders started: their
/// sessions' console hosts and programs.
fn holder_children(ctx: &Ctx) -> String {
    let holders: Vec<u32> =
        crate::server_processes(&ctx.server_dir).into_iter().filter(|(_, role)| role == "hold").map(|(pid, _)| pid).collect();
    if holders.is_empty() {
        return "no holder".into();
    }
    let all = winproc::all();
    let listed: Vec<String> = all.iter().map(|p| format!("{}\t{}\t{}\n", p.pid, p.parent, p.name)).collect();
    ctx.save("processes.txt", format!("holders: {holders:?}\npid\tparent\tname\n{}", listed.concat()));
    let names: Vec<String> = all.into_iter().filter(|p| holders.contains(&p.parent)).map(|p| p.name).collect();
    names.join(",")
}

/// What the case's holders have logged, which includes the ConPTY their
/// sessions got. Saved as `holders.log`.
fn holder_logs(ctx: &Ctx) -> String {
    let mut logs = String::new();
    for entry in std::fs::read_dir(&ctx.server_dir).into_iter().flatten().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("holder-") && name.ends_with(".log") {
            logs.push_str(&std::fs::read_to_string(entry.path()).unwrap_or_default());
        }
    }
    ctx.save("holders.log", &logs);
    logs
}

/// Whether `exe` is on `PATH`.
fn on_path(exe: &str) -> bool {
    std::env::var_os("PATH").is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(exe).is_file()))
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

/// A tab's console host is the packaged `OpenConsole.exe`, started by the
/// holder that owns the session, never by Brindle.
fn conpty_host(ctx: &Ctx) -> Result<Outcome> {
    let hosts = |brindle: u32| format!("{};{}", holder_children(ctx), brindle_hosts(brindle));
    let ran = run(ctx, "host", &["-p", "cmd", "--send", "echo hello-host\r"], &[], 4.0, Some((2.5, &hosts)))?;
    let logs = holder_logs(ctx);
    let (holder, brindle) = ran.during.split_once(';').unwrap_or_default();
    ensure!(holder.split(',').any(|n| n == "OpenConsole.exe"), "under the holder: {holder}");
    ensure!(!holder.split(',').any(|n| n == "conhost.exe"), "a conhost under the holder: {holder}");
    ensure!(holder.split(',').any(|n| n == "cmd.exe"), "the tab's program isn't the holder's: {holder}");
    ensure!(brindle.is_empty(), "console hosts under Brindle: {brindle}");
    ensure!(logs.contains("conpty: sessions use ConPTY "), "no packaged-ConPTY line in holders.log");
    ensure!(screen(&ran.dump).contains("hello-host"), "screen:\n{}", screen(&ran.dump));
    Ok(Outcome::Pass)
}

/// The console hosts among `pid`'s direct children.
fn brindle_hosts(pid: u32) -> String {
    let hosts = winproc::all().into_iter().filter(|p| p.parent == pid).map(|p| p.name);
    hosts.filter(|name| name == "OpenConsole.exe" || name == "conhost.exe").collect::<Vec<_>>().join(",")
}

/// `BRINDLE_CONPTY=inbox` in Brindle's environment reaches the holder in
/// each session's environment: both tabs get the `conhost.exe` built into
/// Windows, and the holder says so once.
fn conpty_fallback(ctx: &Ctx) -> Result<Outcome> {
    let hosts = |_| holder_children(ctx);
    let ran = run(
        ctx,
        "fallback",
        &["-p", "cmd", "--send", "echo hello-inbox\r", "--action", "new_tab"],
        &[("BRINDLE_CONPTY", "inbox")],
        // The steps count from when the window is up, the probe from when
        // Brindle starts, so it leaves the second tab's session time.
        6.5,
        Some((5.0, &hosts)),
    )?;
    let logs = holder_logs(ctx);
    let conhosts = ran.during.split(',').filter(|n| *n == "conhost.exe").count();
    ensure!(conhosts == 2, "{conhosts} conhosts under the holder for two tabs: {}", ran.during);
    ensure!(!ran.during.split(',').any(|n| n == "OpenConsole.exe"), "OpenConsole under the holder: {}", ran.during);
    let lines = logs.matches("BRINDLE_CONPTY=inbox").count();
    ensure!(lines == 1, "{lines} inbox lines for two tabs; see holders.log");
    ensure!(!logs.contains("conpty: sessions use ConPTY "), "the packaged ConPTY was loaded; see holders.log");
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
    let renamed = run(
        ctx,
        "rename",
        &[
            "-p",
            "pwsh",
            "--action",
            "rename_tab",
            "--send",
            "  server  ",
            "--action",
            "picker_confirm",
            "--send",
            "[Console]::Write(\"`e]2;ignored-title`a\")\r",
        ],
        &[],
        6.5,
        None,
    )?;
    ensure!(title(&renamed.dump) == "server", "title after rename_tab: {:?}", title(&renamed.dump));
    let cleared = run(
        ctx,
        "clear-rename",
        &[
            "-p",
            "pwsh",
            "--send",
            "[Console]::Write(\"`e]2;auto-title`a\")\r",
            "--action",
            "rename_tab",
            "--send",
            "  server  ",
            "--action",
            "picker_confirm",
            "--action",
            "rename_tab",
            "--send",
            "   ",
            "--action",
            "picker_confirm",
        ],
        &[],
        9.5,
        None,
    )?;
    ensure!(title(&cleared.dump) == "auto-title", "title after clearing manual title: {:?}", title(&cleared.dump));
    Ok(Outcome::Pass)
}

/// Brindle's PowerShell script in PowerShell 7 and 5.1, under a prompt the
/// user defined: the prompt still draws, its commands are marked (a jump
/// finds a prompt and the last output is selected exactly), `$LASTEXITCODE`
/// survives the prompt, and the directory the script reports is where a new
/// tab, and a restored one, starts. Windows PowerShell is skipped where it
/// isn't installed.
fn pwsh_integration(ctx: &Ctx) -> Result<Outcome> {
    // The README's line, run by the brindle.exe under test.
    let bin = ctx.bin.display().to_string().replace('\\', "/");
    let load = format!("& '{bin}' --shell-integration pwsh | Out-String | Invoke-Expression");
    for shell in ["pwsh.exe", "powershell.exe"] {
        if shell == "powershell.exe" && !on_path(shell) {
            println!("         {shell} isn't on PATH; its checks are skipped");
            continue;
        }
        let config = format!(
            "default_profile = \"si\"\n\n[[profiles]]\nname = \"si\"\ncommand = \"{shell}\"\n\
             args = [\"-NoLogo\", \"-NoProfile\", \"-NoExit\", \"-Command\", \"function prompt {{ 'MYPROMPT> ' }}; {load}\"]\n"
        );
        let label = shell.trim_end_matches(".exe");
        // Marks: OSC 133 A puts the jump at the first prompt, C and D (or
        // the next A) bound the output that gets selected, and the exit
        // code of the command before is still there for the next line.
        let marked = run_with(
            ctx,
            &config,
            &format!("{label}-marks"),
            &[
                "-p",
                "si",
                "--wait-for",
                "MYPROMPT>",
                "--send",
                "1..60\r",
                "--wait-for",
                "60\\nMYPROMPT>",
                "--send",
                "cmd /c exit 3\r",
                "--wait-for",
                "exit 3\\nMYPROMPT>",
                "--send",
                "\"code=$LASTEXITCODE\"\r",
                "--wait-for",
                "code=3\\nMYPROMPT>",
                "--action",
                "select_last_output",
                "--action",
                "jump_to_previous_prompt",
            ],
            &[],
            15.0,
            None,
        )?;
        let selection = marked.dump.selection.as_deref();
        ensure!(selection == Some("code=3"), "{shell}: the last output selected is {selection:?}; see {label}-marks.dump");
        let top: Vec<&str> = marked.dump.screen.iter().take(2).map(String::as_str).collect();
        ensure!(top == ["MYPROMPT> 1..60", "1"], "{shell}: after a jump the view starts {top:?}");
        // New tab: inherits the reported directory. The steps wait for the
        // prompts they need, since Windows PowerShell can take longer than a
        // step to start and load the script, and the directory is reported
        // with the prompt after `cd` (`cddone` is split so the typed line
        // doesn't match). Its tabs' sessions stay for the restore.
        let ran = run_keeping(
            ctx,
            &config,
            label,
            &[
                "--wait-for",
                "MYPROMPT>",
                "--send",
                "cd C:\\Windows; 'cd' + 'done'\r",
                "--wait-for",
                "cddone\\nMYPROMPT>",
                "--action",
                "new_tab",
                "--wait-for",
                "MYPROMPT>",
                "--send",
                "\"dir=$PWD\"\r",
            ],
            &[],
            15.0,
            None,
        )?;
        let text = screen(&ran.dump);
        ensure!(ran.dump.tabs.len() == 2, "{shell}: tabs {:?}", ran.dump.tabs);
        ensure!(text.contains("MYPROMPT> "), "{shell}: the user's prompt is gone:\n{text}");
        ensure!(text.contains("dir=C:\\Windows"), "{shell}: the new tab's directory:\n{text}");
        // Restore: the session written on quit reattaches both tabs, the
        // active one in C:\Windows. Its replay already shows `dir=`, so the
        // live shell answers with `redir=` (split so the typed line doesn't
        // match).
        let restored = run_with(
            ctx,
            &config,
            &format!("{label}-restored"),
            &["--wait-for", "MYPROMPT>", "--send", "'re' + \"dir=$PWD\"\r", "--wait-for", "redir="],
            &[],
            8.0,
            None,
        )?;
        let text = screen(&restored.dump);
        ensure!(restored.dump.tabs.len() == 2, "{shell}: restored tabs {:?}", restored.dump.tabs);
        ensure!(text.contains("redir=C:\\Windows"), "{shell}: the restored tab's directory:\n{text}");
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

/// A tab that prints without pause must not starve the window: every GPUI
/// main-thread wake is a posted message, and Windows hands out `WM_PAINT`
/// and `WM_QUIT` only when none is waiting. The dump comes on time only if
/// the window kept getting idle gaps.
fn flood_responsive(ctx: &Ctx) -> Result<Outcome> {
    let started = Instant::now();
    let ran = run(ctx, "flood", &["-p", "cmd", "--send", "for /L %i in (1,0,2) do @echo flood %i\r"], &[], 6.0, None)?;
    let took = started.elapsed().as_secs_f64();
    ensure!(took < ctx.secs(6.0) + 5.0, "took {took:.1} s");
    ensure!(ran.dump.screen.iter().any(|line| line.starts_with("flood ")), "screen {:?}", ran.dump.screen);
    Ok(Outcome::Pass)
}

fn zones(ctx: &Ctx) -> Result<Outcome> {
    let restore_config = CONFIG.to_string();
    // Its zones' sessions stay for the restore, which reattaches them.
    let split = run_keeping(
        ctx,
        &restore_config,
        "zones-split",
        &[
            "-p",
            "cmd",
            "--send",
            "echo zone1-left\r",
            "--action",
            "split_zone_right",
            "--action",
            "split_zone_down",
            "--action",
            "focus_zone_left",
        ],
        &[],
        6.5,
        None,
    )?;
    ensure!(split.dump.zones.as_deref() == Some("count=3 active=1"), "zones: {:?}", split.dump.zones);
    ensure!(
        split.dump.zone_list
            == [
                "id=1 rect=0,0,0.5,1 active",
                "id=2 rect=0.5,0,0.5,0.5",
                "id=3 rect=0.5,0.5,0.5,0.5",
            ],
        "zone_list: {:?}",
        split.dump.zone_list
    );
    ensure!(screen(&split.dump).contains("zone1-left"), "screen: {}", screen(&split.dump));
    let state_path = ctx.dir.join("state").join("session.toml");
    let state_toml = std::fs::read_to_string(&state_path).map_err(|e| format!("reading session.toml: {e}"))?;
    ctx.save("zones-split-session.toml", &state_toml);
    ensure!(state_toml.contains("split = \"horizontal\""), "session.toml missing split:\n{state_toml}");
    ensure!(state_toml.contains("active = true"), "session.toml missing active leaf:\n{state_toml}");

    let restored = run_with(
        ctx,
        &restore_config,
        "zones-restored",
        &["--action", "open_command_palette"],
        &[],
        3.5,
        None,
    )?;
    ensure!(restored.dump.zones.as_deref() == Some("count=3 active=1"), "restored zones: {:?}", restored.dump.zones);
    ensure!(
        restored.dump.zone_list
            == [
                "id=1 rect=0,0,0.5,1 active",
                "id=2 rect=0.5,0,0.5,0.5",
                "id=3 rect=0.5,0.5,0.5,0.5",
            ],
        "restored zone_list: {:?}",
        restored.dump.zone_list
    );
    ensure!(screen(&restored.dump).contains("zone1-left"), "restored screen: {}", screen(&restored.dump));
    ensure!(
        restored.dump.palette.iter().any(|r| r == "Split Zone Right  [ctrl-shift-e]"),
        "palette rows: {:?}",
        restored.dump.palette
    );
    ensure!(
        restored.dump.palette.iter().any(|r| r == "Close Zone  [ctrl-shift-x]"),
        "palette rows: {:?}",
        restored.dump.palette
    );

    let closed = run(
        ctx,
        "zones-close",
        &[
            "-p",
            "cmd",
            "--send",
            "echo zone1-kept\r",
            "--action",
            "split_zone_right",
            "--action",
            "close_zone",
        ],
        &[],
        5.5,
        None,
    )?;
    ensure!(closed.dump.zones.is_none(), "single zone should have no --- zones: {:?}", closed.dump.zones);
    ensure!(screen(&closed.dump).contains("zone1-kept"), "screen: {}", screen(&closed.dump));
    Ok(Outcome::Pass)
}
