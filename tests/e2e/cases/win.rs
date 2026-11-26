//! Windows cases: local tabs in the case's own terminal server, on its
//! holder's pseudoconsoles. They run PowerShell or cmd, and check the
//! screen dump, Brindle's and the holder's logs, the notification log and
//! the console host process.

use std::io::Read as _;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use super::local_server;
use crate::brindle::{Dump, Run, Stdin};
use crate::common::server::{Isolation, isolate};
use crate::switches::TestSwitch;
use crate::{Case, Ctx, Outcome, Result, ensure, winproc};

pub fn all() -> Vec<Case> {
    let (display, needing) = (Case::display, Case::needing);
    vec![
        Case::headless("cli-list-fault-switches", super::cli::list_fault_switches),
        display("windows-conpty-host", conpty_host),
        display("windows-conpty-fallback", conpty_fallback),
        display("windows-passthrough", passthrough),
        display("windows-glyph-width", glyph_width),
        display("windows-close-mid-output", close_mid_output),
        display("windows-flood-responsive", flood_responsive),
        display("windows-title-child", title_child),
        display("windows-pwsh-integration", pwsh_integration),
        display("windows-pwsh-line", pwsh_line),
        display("windows-caption", caption),
        display("windows-tab-menu-background", tab_menu_background),
        display("windows-zones", zones),
        display("clipboard-right-click-paste", super::clipboard::right_click_paste),
        display("clipboard-right-click-selection", super::clipboard::right_click_selection),
        display("clipboard-copy-to-app", super::clipboard::copy_to_app),
        display("clipboard-paste-from-app", super::clipboard::paste_from_app),
        display("clipboard-osc52-to-app", super::clipboard::osc52_to_app),
        display("clipboard-image-pastes-nothing", super::clipboard::image_pastes_nothing),
        display("clipboard-multiline-line-ends", super::clipboard::multiline_line_ends),
        display("links-refused", super::links::refused),
        display("links-opener-fails", super::links::opener_fails),
        display("links-notice-expires", super::links::notice_expires),
        display("links-notice-dismissed", super::links::notice_dismissed),
        display("settings-fonts", super::fonts::settings_fonts),
        display("ui-font-config", super::fonts::ui_font_config),
        display("local-server-session", local_server::session),
        display("local-server-close", local_server::close),
        display("local-server-quit", local_server::quit),
        display("local-server-crash", local_server::crash),
        display("local-server-close-zone", local_server::close_zone),
        display("local-server-close-window", local_server::close_window),
        display("local-server-close-command", local_server::close_command),
        display("local-server-reattach-quit", local_server::reattach_quit),
        display("local-server-reattach-crash", local_server::reattach_crash),
        display("local-server-reattach-killed", local_server::reattach_killed),
        display("local-server-reattach-inactive", local_server::reattach_inactive),
        display("local-server-reattach-closed-tab", local_server::reattach_closed_tab),
        display("local-server-reattach-no-restore", local_server::reattach_no_restore),
        display("local-server-reattach-second", local_server::reattach_second),
        display("local-server-layout", local_server::layout),
        display("local-server-layout-live", local_server::layout_live),
        display("local-server-palette", local_server::palette),
        display("local-server-palette-highlight", local_server::palette_highlight),
        display("local-server-palette-unlisted", local_server::palette_unlisted),
        display("local-server-attach-flag", local_server::attach_flag),
        display("local-server-env", local_server::env),
        display("local-server-core-restart", local_server::core_restart),
        display("local-server-replay-quiet", local_server::replay_quiet),
        display("local-server-holder-lost", local_server::holder_lost),
        display("local-server-reused-id", local_server::reused_id),
        needing("local-server-sink-panic", &[TestSwitch::SinkPanic], local_server::sink_panic),
        display("local-server-no-helper", local_server::no_helper),
        display("local-server-upgrade", local_server::upgrade),
        display("local-server-manage", local_server::manage),
        display("local-server-message-box", local_server::message_box),
        display("local-server-inbox-host", local_server::inbox_host),
        display("local-server-elevated", local_server::elevated),
        display("windows-elevated-instances", elevated_instances),
        display("windows-elevation-badge", elevation_badge),
        display("windows-agent-turn", super::agent::turn).needs_fake_agent(),
        display("windows-agent-close-confirm", super::agent::close_confirm).needs_fake_agent(),
        // The agent never starts: no fake agent is run.
        display("windows-agent-batch-hint", agent_batch_hint),
        display("windows-agent-prompt-stdin", agent_prompt_stdin).needs_fake_agent(),
        display("remote-container-exec", super::remote::container_exec),
        display("remote-ssh-container", super::remote::ssh_container),
    ]
}

/// What Windows' hit test (`WM_NCHITTEST`) reports at points of the tab
/// strip, given in logical pixels from the client area's top-left (negative
/// x from its right) and the window's `scale`, then what Windows does with a
/// double-click on the `strip` point and a press on the `max` point. GPUI
/// answers for the last mouse position, so each point is first sent to the
/// window as a `WM_MOUSEMOVE`. These are messages to Brindle's own window,
/// not input, so the cursor and other windows are untouched. Until the hit
/// tests read `hits` (the window may not be laid out yet), it stops before
/// the double-click and press, so a probe that is polled changes nothing.
/// `waits` stretches the wait for each of those to take effect.
fn caption_probe(pid: u32, scale: f32, points: &[(&str, f32, f32)], hits: &str, waits: crate::Scale) -> String {
    use windows::Win32::Foundation::{LPARAM, RECT, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        GetClientRect, GetWindowRect, HTCAPTION, HTMAXBUTTON, IsZoomed, SendMessageW, WM_MOUSEMOVE, WM_NCHITTEST,
        WM_NCLBUTTONDBLCLK, WM_NCLBUTTONDOWN, WM_NCLBUTTONUP,
    };

    let Some(hwnd) = brindle_window(pid) else { return "no window".into() };
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
    if out.join(" ") != hits {
        return out.join(" ");
    }
    // The caption buttons act through ShowWindowAsync, so wait (2 s,
    // stretched) for the state wanted; the state then is reported either way.
    let zoomed = |want: bool| {
        // SAFETY: a plain query.
        let now = || unsafe { IsZoomed(hwnd) }.as_bool();
        let zoomed = waits.poll(2.0, "the window's zoom", || Ok((now() == want).then_some(want))).unwrap_or_else(|_| now());
        if zoomed { "maximized" } else { "windowed" }
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

/// Brindle's visible window, by its pid, for messages to it. The thread is
/// made per-monitor DPI aware first, so rectangles and positions are in the
/// window's physical pixels (a DPI-unaware thread gets them scaled down).
fn brindle_window(pid: u32) -> Option<windows::Win32::Foundation::HWND> {
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::UI::HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetThreadDpiAwarenessContext};
    use windows::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, IsWindowVisible};
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
    // SAFETY: changes only this thread's DPI awareness.
    unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) };
    let mut found = Found { pid, hwnd: None };
    // SAFETY: `found` outlives the enumeration.
    unsafe { EnumWindows(Some(each), LPARAM(&mut found as *mut Found as isize)) }.ok();
    found.hwnd
}

/// Right-clicks client point (`x`, `y`), in logical pixels at `scale`, in
/// Brindle's window: `WM_MOUSEMOVE`, `WM_RBUTTONDOWN` and `WM_RBUTTONUP` at
/// the same point, messages to its own window as the caption probe sends.
fn right_click(pid: u32, scale: f32, x: f32, y: f32) -> String {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{SendMessageW, WM_MOUSEMOVE, WM_RBUTTONDOWN, WM_RBUTTONUP};

    let Some(hwnd) = brindle_window(pid) else { return "no window".into() };
    let (x, y) = ((x * scale) as i32, (y * scale) as i32);
    let at = LPARAM(((y as u16 as isize) << 16) | (x as u16 as isize));
    // SAFETY: messages to Brindle's own window, with plain values.
    let send = |msg, wparam: u32| unsafe { SendMessageW(hwnd, msg, Some(WPARAM(wparam as usize)), Some(at)) };
    send(WM_MOUSEMOVE, 0);
    // MK_RBUTTON: the right button is down.
    send(WM_RBUTTONDOWN, 0x0002);
    send(WM_RBUTTONUP, 0);
    "clicked".into()
}

/// Right-clicking a background tab opens its menu without activating it:
/// with the second of two tabs active, a right-click on the first's face
/// leaves the second active, with the first tab's menu open (Move Tab Left
/// disabled, since it is the first). The first tab is the one clicked
/// because its left edge is fixed (6 in) and it is at least 80 wide, while
/// the second's place depends on the first's title (on an elevated runner
/// the console host's title makes the first tab wider).
fn tab_menu_background(ctx: &Ctx) -> Result<Outcome> {
    let args = ["terminal", "cmd", "--action", "new_tab"];
    let first = run(ctx, "scale", &["terminal", "cmd"], &[], 2.0, None)?;
    let scale = first.dump.grid.as_ref().map(|g| g.scale).ok_or("no grid in the dump")?;
    // The first tab's face, 40 in from its left edge; the list's 36px
    // start 6 down.
    let (x, y) = (6.0 + 40.0, 6.0 + 18.0);
    // After the step (from about a second in, one interval after start).
    let start = Instant::now();
    let after = Duration::from_secs_f64(ctx.secs(4.0));
    let probe = move |pid| if start.elapsed() < after { "waiting".into() } else { right_click(pid, scale, x, y) };
    let clicked = |seen: &str| seen == "clicked";
    let ran = run(ctx, "tab-menu", &args, &[], 7.0, Some(Probe { run: &probe, done: &clicked }))?;
    ensure!(ran.during == "clicked", "never right-clicked: {}", ran.during);
    let active: Vec<bool> = ran.dump.tabs.iter().map(|(_, active)| *active).collect();
    ensure!(active == [false, true], "tabs after the right-click: {:?}", ran.dump.tabs);
    let header = ran.dump.menu.first().ok_or("no menu open")?;
    ensure!(
        header.starts_with("items=Rename Tab|[Move Tab Left]|Move Tab Right|-|Close Tab "),
        "not the first tab's menu: {header}"
    );
    Ok(Outcome::Pass)
}

/// The tab strip acts as a native caption: the empty strip is the caption
/// (drag, double-click, menu, Snap), the three buttons are Windows' own,
/// and tabs stay client area. Windows acts on a double-click on the caption
/// (maximizing) and a press on maximize (restoring), which it does only when
/// Brindle leaves the press unhandled.
fn caption(ctx: &Ctx) -> Result<Outcome> {
    // Logical pixels: the strip is 42 high (6px of padding above the 36px
    // tab list, no border under it); the caption buttons are 32 wide
    // (the `IconSm` button size), 4 apart, 8 from the right edge, and
    // centered in the list's 36px (24 down), so empty space ends 112 from
    // the right. Tabs are 9 below the top, 30 high, 6 in (the strip's
    // padding; the list has none at its left and no pill), at least 80 wide
    // (a `cmd` tab is exactly that), with the list's 3px after the last; the
    // new-tab button is 4 past the list, 32 wide.
    let middle = 6.0 + 18.0;
    let points = [
        ("close", -(8.0 + 16.0), middle),
        ("max", -(8.0 + 32.0 + 4.0 + 16.0), middle),
        ("min", -(8.0 + 2.0 * (32.0 + 4.0) + 16.0), middle),
        ("strip", -(8.0 + 3.0 * 32.0 + 2.0 * 4.0 + 60.0), middle),
        ("tab", 48.0, middle),
        // The new-tab button: client area, a press on it is Brindle's.
        ("new-tab", 6.0 + 80.0 + 3.0 + 4.0 + 16.0, middle),
    ];
    // The scale comes from a first run's dump.
    let first = run(ctx, "scale", &["terminal", "cmd"], &[], 2.0, None)?;
    let scale = first.dump.grid.as_ref().map(|g| g.scale).ok_or("no grid in the dump")?;
    // HTCLOSE, HTMAXBUTTON, HTMINBUTTON, HTCAPTION, HTCLIENT, HTCLIENT.
    let hits = "close=20 max=9 min=8 strip=2 tab=1 new-tab=1";
    let expected = format!("{hits} double-click=maximized maximize=windowed");
    let waits = ctx.scale();
    let probe = move |pid| caption_probe(pid, scale, &points, hits, waits);
    // Done once it has clicked: that happens once only.
    let clicked = |seen: &str| seen.contains("double-click=");
    let ran = run(ctx, "caption", &["terminal", "cmd"], &[], 7.0, Some(Probe { run: &probe, done: &clicked }))?;
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
    /// What the `during` probe returned last.
    during: String,
}

/// Runs Brindle with `args` (and `--dump-screen-after dump_at`), polling
/// `during` with its pid while it runs.
fn run(ctx: &Ctx, label: &str, args: &[&str], env: &[(&str, &str)], dump_at: f64, during: During) -> Result<Ran> {
    run_with(ctx, CONFIG, label, args, env, dump_at, during)
}

/// A probe of a running Brindle, called with its pid every 100 ms from its
/// start until `done` accepts what it returned or Brindle exits (after its
/// dump), within the run's deadline. The harness's clock starts at the spawn
/// and Brindle's at its `main`, which a cold first start of the binary can
/// put seconds apart, so neither a probe at a fixed time nor one that stops
/// at the dump time by the harness's clock can rely on Brindle being up.
struct Probe<'a> {
    run: &'a dyn Fn(u32) -> String,
    done: &'a dyn Fn(&str) -> bool,
}

type During<'a> = Option<Probe<'a>>;

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
    // `brindle terminal PROFILE` alone restores the previous run's tabs
    // too; such runs start clean.
    let no_restore: &[&str] = if args.contains(&"terminal") { &["--no-restore"] } else { &[] };
    let mut command = Command::new(&ctx.bin);
    let state = crate::brindle::state_dir(ctx);
    isolate(
        &mut command,
        &Isolation { config: &config, state_dir: &state, server_dir: &ctx.server_dir, display: &ctx.display },
    );
    let mut child = command
        .args(args)
        .args(no_restore)
        .args(ctx.step_interval())
        .args(["--dump-screen-after", &ctx.secs(dump_at).to_string()])
        .env("RUST_LOG", "brindle=info,warn")
        .envs(env.iter().copied())
        .stdout(Stdio::piped())
        .stderr(log)
        .spawn()
        .map_err(|e| format!("starting {}: {e}", ctx.bin.display()))?;
    // Each wait: the dump's time, then 30 s to print it and quit (stretched).
    let (waits, limit) = (ctx.scale(), dump_at + 30.0);
    let mut seen = String::new();
    if let Some(probe) = during {
        // Until it says it is done or Brindle exits; what it saw last is
        // reported either way.
        let pid = child.id();
        waits
            .poll(limit, "the probe", || {
                seen = (probe.run)(pid);
                let exited = child.try_wait().map_err(|e| e.to_string())?.is_some();
                Ok(((probe.done)(&seen) || exited).then_some(()))
            })
            .ok();
    }
    if waits.poll(limit, "Brindle exiting", || child.try_wait().map_err(|e| e.to_string())).is_err() {
        child.kill().ok();
        return Err(format!("Brindle didn't exit; see {label}.log"));
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
    // A process list that can't be read stands in for the names, so the
    // case's check fails with the cause.
    let all = match winproc::all() {
        Ok(all) => all,
        Err(error) => return format!("can't list processes: {error}"),
    };
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

/// How many of a `host_children` list are named `name`.
fn count(list: &str, name: &str) -> usize {
    list.split(',').filter(|n| *n == name).count()
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
    // Done once the holder has started the tab's console host and program.
    let started = |seen: &str| {
        let holder = seen.split_once(';').map_or("", |(holder, _)| holder);
        count(holder, "OpenConsole.exe") > 0 && count(holder, "cmd.exe") > 0
    };
    let probe = Probe { run: &hosts, done: &started };
    let ran = run(ctx, "host", &["terminal", "cmd", "--send", "echo hello-host\r"], &[], 4.0, Some(probe))?;
    let logs = holder_logs(ctx);
    let (holder, brindle) = ran.during.split_once(';').unwrap_or_default();
    ensure!(count(holder, "OpenConsole.exe") > 0, "under the holder: {holder}");
    ensure!(count(holder, "conhost.exe") == 0, "a conhost under the holder: {holder}");
    ensure!(count(holder, "cmd.exe") > 0, "the tab's program isn't the holder's: {holder}");
    ensure!(brindle.is_empty(), "console hosts under Brindle: {brindle}");
    ensure!(logs.contains("conpty: sessions use ConPTY "), "no packaged-ConPTY line in holders.log");
    ensure!(screen(&ran.dump).contains("hello-host"), "screen:\n{}", screen(&ran.dump));
    Ok(Outcome::Pass)
}

/// The console hosts among `pid`'s direct children.
fn brindle_hosts(pid: u32) -> String {
    let all = match winproc::all() {
        Ok(all) => all,
        Err(error) => return format!("can't list processes: {error}"),
    };
    let hosts = all.into_iter().filter(|p| p.parent == pid).map(|p| p.name);
    hosts.filter(|name| name == "OpenConsole.exe" || name == "conhost.exe").collect::<Vec<_>>().join(",")
}

/// `BRINDLE_CONPTY=inbox` in Brindle's environment reaches the holder in
/// each session's environment: both tabs get the `conhost.exe` built into
/// Windows, and the holder says so once.
fn conpty_fallback(ctx: &Ctx) -> Result<Outcome> {
    let hosts = |_: u32| holder_children(ctx);
    let ran = run(
        ctx,
        "fallback",
        &["terminal", "cmd", "--send", "echo hello-inbox\r", "--action", "new_tab"],
        &[("BRINDLE_CONPTY", "inbox")],
        6.5,
        // One host per tab: done once the second tab's is there.
        Some(Probe { run: &hosts, done: &|seen| count(seen, "conhost.exe") >= 2 }),
    )?;
    let logs = holder_logs(ctx);
    let conhosts = count(&ran.during, "conhost.exe");
    ensure!(conhosts == 2, "{conhosts} conhosts under the holder for two tabs: {}", ran.during);
    ensure!(count(&ran.during, "OpenConsole.exe") == 0, "OpenConsole under the holder: {}", ran.during);
    let lines = logs.matches("BRINDLE_CONPTY=inbox").count();
    ensure!(lines == 1, "{lines} inbox lines for two tabs; see holders.log");
    ensure!(!logs.contains("conpty: sessions use ConPTY "), "the packaged ConPTY was loaded; see holders.log");
    ensure!(ran.dump.tabs.len() == 2, "tabs: {:?}", ran.dump.tabs);
    Ok(Outcome::Pass)
}

fn passthrough(ctx: &Ctx) -> Result<Outcome> {
    let notes = ctx.dir.join("notify.log");
    // The reply is read until its terminator (BEL, or ST), for at most 5 s
    // (stretched).
    let send = script(
        ctx,
        "passthrough.ps1",
        &format!(
            r#"$e = [char]27; $b = [char]7
[Console]::Write("building`n")
[Console]::Write("$e]9;e2e-note$b")
[Console]::Write("done`n")
[Console]::Write("$e[?1000h$e[?1006h")
[Console]::Write("$e]11;?$b")
$reply = ''
$until = (Get-Date).AddSeconds({limit})
while (-not ($reply.EndsWith($b) -or $reply.EndsWith("$e\")) -and (Get-Date) -lt $until) {{
    if ([Console]::KeyAvailable) {{ $reply += [Console]::ReadKey($true).KeyChar }} else {{ Start-Sleep -Milliseconds 20 }}
}}
"reply=" + ($reply -replace $e, 'ESC' -replace $b, 'BEL')
"#,
            limit = ctx.secs(5.0)
        ),
    )?;
    let notes_env = notes.display().to_string();
    let ran = run(ctx, "passthrough", &["terminal", "pwsh", "--send", &send], &[("BRINDLE_NOTIFY_LOG", &notes_env)], 6.0, None)?;
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
    let ran = run(ctx, "width", &["terminal", "pwsh", "--send", &send], &[], 5.0, None)?;
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
    // `ping -n N` runs for N - 1 s: past the dump (4 s, stretched), with 4 s
    // to spare.
    let lasting = ctx.secs(4.0).ceil() as u32 + 4;
    let during = run(ctx, "during", &["terminal", "pwsh", "--send", &ping(lasting)], &[], 4.0, None)?;
    ensure!(title(&during.dump) == "PING", "title while ping runs: {:?}", title(&during.dump));
    let after = run(ctx, "after", &["terminal", "pwsh", "--send", &ping(1)], &[], 5.0, None)?;
    ensure!(title(&after.dump) == "pwsh", "title after ping: {:?}", title(&after.dump));
    let renamed = run(
        ctx,
        "rename",
        &[
            "terminal",
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
            "terminal",
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
        if shell == "powershell.exe" {
            powershell_warmup(ctx, &bin);
        }
        // Marks: OSC 133 A puts the jump at the first prompt, C and D (or
        // the next A) bound the output that gets selected, and the exit
        // code of the command before is still there for the next line.
        let marked = run_with(
            ctx,
            &config,
            &format!("{label}-marks"),
            &[
                "terminal",
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

/// Brindle's PowerShell script reports each command line with OSC 633 `E`,
/// in PowerShell 7 and 5.1: Copy Last Command Line copies the line as typed,
/// `;` included, which the shell then reads back with `Get-Clipboard`
/// (printed as `clip=[…]`, split so the typed line doesn't match). Windows
/// PowerShell is skipped where it isn't installed. The command prints only
/// its marker: a table before it would end in a blank line, and the wait for
/// the marker over the next prompt would never match.
fn pwsh_line(ctx: &Ctx) -> Result<Outcome> {
    let bin = ctx.bin.display().to_string().replace('\\', "/");
    let load = format!("& '{bin}' --shell-integration pwsh | Out-String | Invoke-Expression");
    let typed = "$null = Get-ChildItem C:/; 'gci' + 'done'";
    let send = format!("{typed}\r");
    for shell in ["pwsh.exe", "powershell.exe"] {
        if shell == "powershell.exe" && !on_path(shell) {
            println!("         {shell} isn't on PATH; its checks are skipped");
            continue;
        }
        let config = format!(
            "default_profile = \"si\"\n\n[[profiles]]\nname = \"si\"\ncommand = \"{shell}\"\n\
             args = [\"-NoLogo\", \"-NoProfile\", \"-NoExit\", \"-Command\", \"function prompt {{ 'MYPROMPT> ' }}; {load}\"]\n"
        );
        let label = format!("{}-line", shell.trim_end_matches(".exe"));
        if shell == "powershell.exe" {
            powershell_warmup(ctx, &bin);
        }
        let ran = run_with(
            ctx,
            &config,
            &label,
            &[
                "terminal",
                "si",
                "--wait-for",
                "MYPROMPT>",
                "--send",
                send.as_str(),
                "--wait-for",
                "gcidone\\nMYPROMPT>",
                "--action",
                "copy_last_command",
                "--send",
                "'cl' + 'ip=[' + (Get-Clipboard -Raw) + ']'\r",
                "--wait-for",
                "clip=[",
            ],
            &[],
            15.0,
            None,
        )?;
        let text = screen(&ran.dump);
        ensure!(ran.dump.wait_timeouts.is_empty(), "{shell}: waits {:?}; see {label}.dump", ran.dump.wait_timeouts);
        ensure!(text.contains(&format!("clip=[{typed}]")), "{shell}: the clipboard isn't the typed line:\n{text}");
    }
    Ok(Outcome::Pass)
}

/// Starts Windows PowerShell 5.1 once, headless, doing what the case's tabs
/// do before their first prompt, and writes how long it took to
/// `powershell-warmup.txt`. On CI the case's tab is the job's first 5.1
/// start, and twice it printed nothing for the whole dump time: the warm-up
/// takes that cold start out of the GUI run, and its time shows whether the
/// runner was cold. A failed or slow warm-up is recorded, not a failure.
fn powershell_warmup(ctx: &Ctx, bin: &str) {
    // Seconds, stretched.
    const LIMIT: f64 = 120.0;
    let command = format!(
        "Get-Command PSConsoleHostReadLine -ErrorAction Ignore | Out-Null; \
         & '{bin}' --shell-integration pwsh | Out-String | Out-Null"
    );
    let started = Instant::now();
    let outcome = match Command::new("powershell.exe")
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command", &command])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Err(e) => format!("not started: {e}"),
        Ok(mut child) => match ctx.scale().poll(LIMIT, "the warm-up", || child.try_wait().map_err(|e| e.to_string())) {
            Ok(status) => format!("exited with {status}"),
            Err(why) => {
                child.kill().ok();
                format!("killed: {why}")
            }
        },
    };
    ctx.save("powershell-warmup.txt", format!("{:.1} s, {outcome}\n", started.elapsed().as_secs_f64()));
}

fn close_mid_output(ctx: &Ctx) -> Result<Outcome> {
    let ran = run(
        ctx,
        "close",
        &[
            "terminal",
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
    // Closing the flooding tab didn't block: a blocked window gives no dump,
    // and the run's own limit fails the case. (A slow but unfrozen window
    // isn't caught; the bug this guards was a freeze.)
    ensure!(ran.dump.tabs.len() == 1, "tabs: {:?}", ran.dump.tabs);
    Ok(Outcome::Pass)
}

/// A tab that prints without pause must not starve the window: every GPUI
/// main-thread wake is a posted message, and Windows hands out `WM_PAINT`
/// and `WM_QUIT` only when none is waiting. A starved window gives no dump,
/// and the run's own limit fails the case. (A slow but unfrozen window
/// isn't caught; the bug this guards was a freeze.)
fn flood_responsive(ctx: &Ctx) -> Result<Outcome> {
    let ran = run(ctx, "flood", &["terminal", "cmd", "--send", "for /L %i in (1,0,2) do @echo flood %i\r"], &[], 6.0, None)?;
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
            "terminal",
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
            "terminal",
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

/// An elevated and an unelevated Brindle, both with no `BRINDLE_STATE_DIR`,
/// run at once, each as an instance with its own `session.toml`: the
/// elevated one in `state-elevated` under `%LOCALAPPDATA%\brindle` (here a
/// private `LOCALAPPDATA` in the case directory), made for Administrators
/// and SYSTEM only, and the unelevated one in `brindle` itself. A
/// `state-elevated` already there without that protection, with a
/// `session.toml` planted in it, is set aside, not protected again with the
/// file in it. Neither runs on its own for want of the other's session
/// lock. The unelevated one
/// starts once the elevated one's tab is up, and the elevated one's shell
/// waits for it to be done before Brindle quits, so they overlap. Needs an
/// elevated runner whose desktop shell runs unelevated (UAC on); skipped
/// otherwise, since elevating would ask through UAC.
fn elevated_instances(ctx: &Ctx) -> Result<Outcome> {
    if !winproc::elevated().map_err(|e| format!("reading this process's token: {e}"))? {
        return Ok(Outcome::Skip(
            "not elevated: an elevated Brindle beside an unelevated one needs an elevated runner \
             (elevating here would ask through UAC)"
                .into(),
        ));
    }
    let appdata = ctx.dir.join("appdata");
    std::fs::create_dir_all(&appdata).map_err(|e| e.to_string())?;
    let appdata_text = appdata.to_str().ok_or("the case directory isn't UTF-8")?.to_string();
    let (medium, high) = (appdata.join("brindle"), appdata.join("brindle").join("state-elevated"));
    // A `state-elevated` someone else made, with a session in it: the
    // elevated Brindle sets it aside and makes the directory fresh, rather
    // than protecting it again with the planted file in it.
    const PLANTED: &str = "version = 1\n# planted\n";
    std::fs::create_dir_all(&high).map_err(|e| e.to_string())?;
    std::fs::write(high.join("session.toml"), PLANTED).map_err(|e| e.to_string())?;
    let done = ctx.dir.join("unelevated-done");
    let slashed = |path: &std::path::Path| path.display().to_string().replace('\\', "/");
    // The elevated tab's shell prints `both-ran` once the unelevated run is
    // done, and Brindle quits after that, so the two overlap however slowly
    // either runs. `--wait-for` gives up after 30 step intervals; the
    // unelevated run is given less (`UNELEVATED_LIMIT`).
    let gate = format!("while (-not (Test-Path '{}')) {{ Start-Sleep -Milliseconds 100 }}; 'both-' + 'ran'\r", slashed(&done));
    let elevated = crate::brindle::Run::new(ctx, "elevated")
        .env("BRINDLE_STATE_DIR", "")
        .env("LOCALAPPDATA", &appdata_text)
        .prompt()
        .send(&gate)
        .wait_for("both-ran")
        .action("quit")
        .expect_exit()
        // A limit only: the run quits as soon as its shell has printed.
        .dump_at(90.0);
    let (beside, ran) = std::thread::scope(|scope| {
        let running = scope.spawn(move || elevated.run());
        let beside = unelevated_beside(ctx, &appdata, &done, &|| running.is_finished());
        // Whatever happened beside it, the elevated Brindle may quit now.
        if !done.exists() {
            std::fs::write(&done, "released by the case").ok();
        }
        (beside, running.join().unwrap_or_else(|_| Err("the elevated run panicked".into())))
    });
    let outcome = beside?;
    ran?;
    if let Outcome::Skip(why) = outcome {
        return Ok(Outcome::Skip(why));
    }

    let log = |label: &str| std::fs::read_to_string(ctx.dir.join(format!("{label}.log"))).unwrap_or_default();
    for (label, dir) in [("elevated", &high), ("unelevated", &medium)] {
        let text = log(label);
        ensure!(!text.contains("another Brindle owns the session"), "the {label} Brindle wasn't an instance; see {label}.log");
        ensure!(!text.contains("no state directory"), "the {label} Brindle had no state directory; see {label}.log");
        ensure!(dir.join("session.toml").is_file(), "the {label} Brindle wrote no {}", dir.join("session.toml").display());
    }
    // Only Administrators and SYSTEM can change the elevated directory.
    match brindle_remote_protocol::win::hold_protected_dir(&high) {
        Ok(Some(_)) => {}
        Ok(None) => return Err(format!("{} is missing", high.display())),
        Err(why) => return Err(format!("{} isn't protected: {why}", high.display())),
    }
    let kept = std::fs::read_to_string(high.join("session.toml")).unwrap_or_default();
    ensure!(!kept.contains("# planted"), "the planted session.toml is still in {}", high.display());
    let set_aside = std::fs::read_dir(&medium)
        .map_err(|e| e.to_string())?
        .flatten()
        .filter(|entry| {
            let name = entry.file_name().to_string_lossy().into_owned();
            brindle_remote_protocol::server_dir::is_untrusted_name(&name) && name.starts_with("state-elevated.")
        })
        .any(|entry| std::fs::read_to_string(entry.path().join("session.toml")).is_ok_and(|text| text == PLANTED));
    ensure!(set_aside, "the planted state-elevated wasn't set aside beside {}", high.display());
    let dump = Dump::parse(&std::fs::read_to_string(ctx.dir.join("unelevated.dump")).unwrap_or_default());
    ensure!(screen(&dump).contains("unelevated-tab"), "the unelevated tab: see unelevated.dump");
    ensure!(dump.config.is_none(), "the unelevated window's config line: {:?}", dump.config);
    ensure!(dump.badges.is_empty(), "the unelevated window shows a badge: {:?}", dump.badges);
    Ok(Outcome::Pass)
}

/// A tab whose program runs elevated shows the privileged badge, labelled
/// `Administrator`, on its face (the dump's `--- badge tab=1 …` line: the
/// tab lines count from 1), and one that doesn't shows none. This process's
/// token decides which Brindle's programs get, so an unelevated runner can
/// check the second half only (and skips, saying so); the unelevated
/// Brindle beside an elevated one has its dump checked for a badge in
/// [`elevated_instances`]. The badge is read once a second, so the dump is
/// taken well after the shell is up.
fn elevation_badge(ctx: &Ctx) -> Result<Outcome> {
    let elevated = winproc::elevated().map_err(|e| format!("reading this process's token: {e}"))?;
    let out = crate::brindle::Run::new(ctx, "run")
        .prompt()
        .send("'rea' + 'dy'\r")
        .wait_for("ready")
        .dump_at(12.0)
        .run()?;
    ensure!(out.dump.wait_timeouts.is_empty(), "a wait gave up: {:?}; screen {:?}", out.dump.wait_timeouts, out.dump.screen);
    if !elevated {
        ensure!(out.dump.badges.is_empty(), "an unelevated tab shows a badge: {:?}", out.dump.badges);
        return Ok(Outcome::Skip(
            "not elevated: an unelevated tab shows no badge, but the elevated half needs an elevated runner \
             (elevating here would ask through UAC)"
                .into(),
        ));
    }
    let want = ["tab=1 kind=privileged label=\"Administrator\" draw=icon"];
    ensure!(out.dump.badges == want, "an elevated tab's badges: {:?}, want {want:?}", out.dump.badges);
    Ok(Outcome::Pass)
}

/// How long the unelevated run may take, under the elevated run's
/// `--wait-for` limit of 30 step intervals.
const UNELEVATED_LIMIT: f64 = 25.0;

/// Runs the unelevated Brindle of [`elevated_instances`] once the elevated
/// one's tab is attached in the case's server, and waits for its script to
/// write `done`, which must come while the elevated one still runs
/// (`elevated_ended` says whether its run has ended). Its terminal server
/// is a directory of its own in the case directory, which it stops itself:
/// an unelevated client may not use the elevated one's.
fn unelevated_beside(
    ctx: &Ctx,
    appdata: &std::path::Path,
    done: &std::path::Path,
    elevated_ended: &dyn Fn() -> bool,
) -> Result<Outcome> {
    let up = ctx.scale().poll(30.0, "the elevated Brindle's tab attached", || {
        if elevated_ended() {
            return Ok(Some(false));
        }
        Ok(crate::brindle::listed_sessions(ctx)?.iter().any(|&(_, attached)| attached).then_some(true))
    })?;
    ensure!(up, "the elevated run ended before its tab was attached; see elevated.log");
    let server = ctx.dir.join("unelevated-server");
    let (dump, log) = (ctx.dir.join("unelevated.dump"), ctx.dir.join("unelevated.log"));
    let script = ctx.dir.join("unelevated.cmd");
    let quote = |path: &std::path::Path| format!("\"{}\"", path.display());
    let body = [
        "@echo off".to_string(),
        "set \"BRINDLE_STATE_DIR=\"".to_string(),
        format!("set \"LOCALAPPDATA={}\"", appdata.display()),
        format!("set \"BRINDLE_SERVER_DIR={}\"", server.display()),
        format!("set \"BRINDLE_CONFIG={}\"", ctx.dir.join("config.toml").display()),
        // The case's helper: this environment is the user's own, not the
        // test's.
        format!("set \"BRINDLE_SERVER_HELPER={}\"", ctx.helper.display()),
        "set \"RUST_LOG=brindle=info,warn\"".to_string(),
        format!(
            "{} -p Shell --no-restore --wait-for \"{}\" --send \"'unelev' + 'ated-tab'\\r\" {} --dump-screen-after {} > {} 2> {}",
            quote(&ctx.bin),
            crate::brindle::PROMPT,
            ctx.step_interval().join(" "),
            ctx.secs(7.0),
            quote(&dump),
            quote(&log)
        ),
        // Only an unelevated client may stop the unelevated server.
        format!("{} stop > nul 2>&1", quote(&ctx.helper)),
        format!("echo done > {}", quote(done)),
    ];
    std::fs::write(&script, body.join("\r\n") + "\r\n").map_err(|e| e.to_string())?;
    let line = format!("cmd.exe /d /c \"{}\"", script.display());
    let cmd = match winproc::start_unelevated(&line) {
        Ok(pid) => pid,
        Err(why) => return Ok(Outcome::Skip(format!("can't run an unelevated Brindle beside: {why}"))),
    };
    let finished = ctx.scale().poll(UNELEVATED_LIMIT, "the unelevated run", || {
        if done.exists() {
            return Ok(Some(Some(elevated_ended())));
        }
        // The script writes `done` last, so a cmd gone without it stopped
        // early: most likely it couldn't write to the case directory.
        Ok((!winproc::alive(cmd) && !done.exists()).then_some(None))
    });
    let result = match finished {
        Ok(Some(false)) => Ok(Outcome::Pass),
        Ok(Some(true)) => Err("the elevated Brindle had quit before the unelevated one was done; see elevated.log".into()),
        Ok(None) => Err(format!(
            "the unelevated script ended without finishing; can the unelevated user write {}? See unelevated.log",
            ctx.dir.display()
        )),
        Err(why) => {
            // Brindle and the helper commands are cmd's children.
            let children: Vec<u32> = winproc::all()
                .map(|all| all.into_iter().filter(|p| p.parent == cmd).map(|p| p.pid).collect())
                .unwrap_or_default();
            children.into_iter().chain([cmd]).for_each(winproc::kill);
            Err(format!("{why}; see unelevated.log"))
        }
    };
    // The unelevated server's roles run from its own copies; what `stop`
    // left is ended and reported.
    let copies = server.join("bin");
    let left = || -> std::io::Result<Vec<u32>> {
        let name = ctx.helper.file_name().unwrap_or_default().to_string_lossy().into_owned();
        Ok(winproc::all()?
            .into_iter()
            .filter(|p| p.name.eq_ignore_ascii_case(&name))
            .filter(|p| winproc::image(p.pid).is_some_and(|image| crate::under(&image, &copies)))
            .map(|p| p.pid)
            .collect())
    };
    ctx.scale()
        .poll(5.0, "the unelevated server's roles ending", || {
            Ok(left().map_err(|e| format!("listing processes: {e}"))?.is_empty().then_some(()))
        })
        .ok();
    let leftover = left();
    leftover.iter().flatten().for_each(|&pid| winproc::kill(pid));
    let outcome = result?;
    let leftover = leftover.map_err(|e| format!("can't tell whether roles are left running: {e}"))?;
    ensure!(leftover.is_empty(), "left running after `brindle-remote stop`: {leftover:?}");
    Ok(outcome)
}

/// An agent profile whose command is a batch file on the `PATH` (`my-agent`,
/// with `my-agent.cmd` in a directory the profile's `PATH` starts with)
/// isn't started: the zone says the server's cause, which names the file
/// and the `cmd /c my-agent` form that runs it, and offers no prompt.
fn agent_batch_hint(ctx: &Ctx) -> Result<Outcome> {
    let bin = ctx.dir.join("bin");
    std::fs::create_dir_all(&bin).map_err(|e| format!("creating {}: {e}", bin.display()))?;
    let batch = bin.join("my-agent.cmd");
    std::fs::write(&batch, "@echo off\r\nexit /b 0\r\n").map_err(|e| format!("writing {}: {e}", batch.display()))?;
    let path = format!("{};{}", bin.display(), std::env::var("PATH").unwrap_or_default());
    // A TOML literal string takes the path as it is, backslashes included.
    ensure!(!path.contains('\''), "PATH has a quote in it: {path}");
    let profile =
        format!("\n[[profiles]]\nname = \"Batch\"\ntype = \"agent\"\ncommand = \"my-agent\"\nenv = {{ PATH = '{path}' }}\n");
    let out = Run::new(ctx, "run")
        .config(&profile)
        .agent("Batch")
        .wait_for("state=not-started")
        .dump_at(4.0)
        .run()?;
    super::agent::waited(&out)?;
    let line = out.dump.agent.first().map(String::as_str).unwrap_or_default();
    ensure!(
        // The server's refusal reads "cannot run my-agent: …" (the holder's
        // spawn-failure form), after the zone's own prefix.
        line.starts_with("agent zone=1 state=not-started message=\"the agent my-agent didn't start: ")
            && line.contains("my-agent is a batch file (")
            && line.contains("my-agent.cmd); run it as `cmd /c my-agent`\""),
        "not the batch file's hint: {line}"
    );
    super::agent::agent_has(&out, "prompt status=disabled")?;
    ensure!(crate::brindle::listed_sessions(ctx)?.is_empty(), "sessions left: {:?}", crate::brindle::listed_sessions(ctx)?);
    Ok(Outcome::Pass)
}

/// `brindle agent Fake -p -` reads its first prompt from the standard input
/// a parent gives it, though `brindle.exe` is a GUI program: a file handed
/// to the child directly, and cmd's `< file`; each sends the file's text,
/// its line ending dropped. With no standard input handle at all, as a GUI
/// program started from a console with nothing redirected has, it is a
/// usage error (status 2) at once. PowerShell's pipeline is a manual check:
/// PowerShell doesn't wait for a GUI program.
fn agent_prompt_stdin(ctx: &Ctx) -> Result<Outcome> {
    let (profile, received) = super::agent::hello_agent(ctx)?;
    let file = ctx.dir.join("prompt.txt");
    std::fs::write(&file, "from a file\r\n").map_err(|e| format!("writing {}: {e}", file.display()))?;
    let wanted = ["user \"from a file\"", "agent \"Hello!\""];

    let out = Run::new(ctx, "file")
        .config(&profile)
        .agent("Fake")
        .first_prompt("-")
        .stdin(Stdin::File(file.clone()))
        .wait_for(&super::agent::answered())
        .dump_at(5.0)
        .run()?;
    super::agent::waited(&out)?;
    ensure!(out.dump.agent_entries() == wanted, "a file: entries {:?}", out.dump.agent);

    let config = ctx.dir.join("config.toml");
    std::fs::write(&config, crate::brindle::config("e2e") + &profile).map_err(|e| format!("writing the config: {e}"))?;
    std::fs::remove_file(&received).ok();
    let dump = agent_through_cmd(ctx, "cmd", &config, &file, 5.0)?;
    ensure!(dump.wait_timeouts.is_empty() && dump.wait_pending.is_empty(), "cmd: waits {:?} {:?}", dump.wait_timeouts, dump.wait_pending);
    ensure!(dump.agent_entries() == wanted, "cmd's `<`: entries {:?}", dump.agent);

    let said = refused_without_stdin(ctx, "absent", &config)?;
    ensure!(said == "brindle: -p - needs a file or pipe on standard input", "no standard input: {said:?}");
    Ok(Outcome::Pass)
}

/// Runs `cmd /d /v:off /c "BRINDLE agent Fake --no-restore -p - … < FILE >
/// DUMP 2> LOG"` with the case's isolation and `config`, so cmd's `<` gives
/// Brindle its standard input, and returns Brindle's dump once it has
/// exited. Brindle is cmd's child; it is waited for as such, whether or not
/// cmd waits for a GUI program, and ended with cmd if the run overruns.
fn agent_through_cmd(ctx: &Ctx, label: &str, config: &std::path::Path, file: &std::path::Path, dump_at: f64) -> Result<Dump> {
    use std::os::windows::process::CommandExt as _;
    let (dump, log) = (ctx.dir.join(format!("{label}.dump")), ctx.dir.join(format!("{label}.log")));
    let quote = |path: &std::path::Path| format!("\"{}\"", path.display());
    // `Hello!` is in the answer's entry line; no quotes or spaces, so cmd
    // passes it as it is (`!` is plain with delayed expansion off).
    let inner = format!(
        "{} agent Fake --no-restore -p - {} --wait-for Hello! --dump-screen-after {} < {} > {} 2> {}",
        quote(&ctx.bin),
        ctx.step_interval().join(" "),
        ctx.secs(dump_at),
        quote(file),
        quote(&dump),
        quote(&log)
    );
    let mut command = Command::new("cmd.exe");
    isolate(
        &mut command,
        &Isolation {
            config,
            state_dir: &crate::brindle::state_dir(ctx),
            server_dir: &ctx.server_dir,
            display: &ctx.display,
        },
    );
    let mut cmd = command
        .raw_arg(format!("/d /v:off /c \"{inner}\""))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("starting cmd: {e}"))?;
    let (pid, name) = (cmd.id(), ctx.bin.file_name().unwrap_or_default().to_string_lossy().into_owned());
    let brindles = || -> Result<Vec<u32>> {
        Ok(winproc::all()
            .map_err(|e| format!("listing processes: {e}"))?
            .into_iter()
            .filter(|p| p.parent == pid && p.name.eq_ignore_ascii_case(&name))
            .map(|p| p.pid)
            .collect())
    };
    // The dump's time, then 30 s to print it and quit (stretched).
    let finished = ctx.scale().poll(dump_at + 30.0, "Brindle started through cmd exiting", || {
        if cmd.try_wait().map_err(|e| e.to_string())?.is_none() {
            return Ok(None);
        }
        Ok(brindles()?.into_iter().all(|brindle| !winproc::alive(brindle)).then_some(()))
    });
    if let Err(why) = finished {
        brindles().unwrap_or_default().into_iter().for_each(winproc::kill);
        cmd.kill().ok();
        cmd.wait().ok();
        return Err(format!("{why}; see {label}.log"));
    }
    let text = std::fs::read_to_string(&dump).map_err(|e| format!("reading {}: {e}; see {label}.log", dump.display()))?;
    crate::brindle::hang_up_detached(ctx)?;
    Ok(Dump::parse(&text))
}

/// Runs `brindle agent Fake -p -` with `config` and no standard input or
/// output handle ([`winproc::start_without_stdin`]), and returns the
/// `brindle: ` line it printed, once it has exited with status 2.
fn refused_without_stdin(ctx: &Ctx, label: &str, config: &std::path::Path) -> Result<String> {
    let path = ctx.dir.join(format!("{label}.log"));
    let log = std::fs::File::create(&path).map_err(|e| format!("creating {}: {e}", path.display()))?;
    let mut command = Command::new(&ctx.bin);
    isolate(
        &mut command,
        &Isolation {
            config,
            state_dir: &crate::brindle::state_dir(ctx),
            server_dir: &ctx.server_dir,
            display: &ctx.display,
        },
    );
    command.args(["agent", "Fake", "-p", "-"]);
    let started = winproc::start_without_stdin(&command, &log)?;
    drop(log);
    let code = ctx.scale().poll(10.0, "brindle agent Fake -p - with no standard input exiting", || Ok(started.exit_code()));
    let code = match code {
        Ok(code) => code,
        Err(why) => {
            started.kill();
            return Err(format!("{why}; see {label}.log"));
        }
    };
    let text = std::fs::read_to_string(&path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    ensure!(code == 2, "brindle agent Fake -p - with no standard input exited with {code}; see {label}.log");
    let said = text.lines().find(|line| line.starts_with("brindle: "));
    Ok(said.ok_or_else(|| format!("no `brindle: ` line; see {label}.log"))?.to_string())
}
