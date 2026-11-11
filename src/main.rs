// A GUI program on Windows, so launching it opens no console window (see
// `console_output`). Test builds keep the console subsystem.
#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]

mod actions;
mod config;
mod config_edit;
mod desktop;
mod find_bar;
mod instance;
mod latency;
mod notify;
mod picker;
mod primary;
mod reattach;
mod remote;
mod session;
mod settings;
mod settings_dialog;
mod terminal;
mod terminal_element;
mod terminal_view;
mod theme;
#[cfg(windows)]
mod win;
mod workspace;
mod zone_view;

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use gpui::{
    App, AppContext as _, Application, BorrowAppContext as _, Bounds, Keystroke, WindowBackgroundAppearance,
    WindowBounds, WindowDecorations, WindowOptions, px, size,
};

use crate::actions::{Quit, ReloadConfig};
use crate::config::Config;
use crate::latency::Latency;
use crate::settings::Settings;
use crate::session::{SessionStore, WindowState};
use crate::workspace::{Launch, LaunchRequest, Workspace};

const HELP: &str = "\
Brindle — a GPU-accelerated terminal

USAGE:
    brindle [OPTIONS] [-e COMMAND [ARGS]...]

OPTIONS:
    -e, --command <CMD> [ARGS]   Run CMD instead of the profile's shell (must be last)
    -p, --profile <NAME>         Open a tab with this profile (default: config's
                                 default_profile)
    -d, --working-directory <DIR>
                                 Start in DIR
        --config <FILE>          Use FILE instead of ~/.config/brindle/config.toml
        --no-restore             Reopen nothing this time: no saved tabs, no
                                 detached sessions
        --attach <ID>            Open a tab attached to the terminal server's
                                 detached session ID (see brindle-remote list)
        --list-actions           Print bindable action names and exit
        --list-themes            Print built-in theme names and exit
        --shell-integration <SHELL>
                                 Print the bash, zsh, fish or pwsh (PowerShell)
                                 integration script (OSC 133 prompt marks) and
                                 exit, e.g. in ~/.bashrc:
                                 eval \"$(brindle --shell-integration bash)\"
        --install-desktop        Add Brindle to the desktop's launcher, then exit
        --uninstall-desktop      Remove it from the launcher, then exit
        --prefix <DIR>           With the two above: use DIR/share instead of
                                 ~/.local/share (e.g. /usr/local)
    -h, --help                   Print help
    -V, --version                Print version

DEBUGGING:
        --send <TEXT>            Type TEXT into the focused text field, else
                                 the active tab (\\r = Enter)
        --action <NAME>          Run an action (see --list-actions)
        --key <KEYSTROKE>        Press a key (e.g. tab, shift-tab, enter) through the
                                 app's own key dispatch; in-process, not OS input
        --modality <M>           Count focus as moved by the pointer or the keyboard
                                 from now on, which decides whether focus rings show
        --hover [MODS-]COL,ROW   Move the mouse over a cell of the active terminal
                                 (e.g. ctrl-3,0); in-process, not OS input
        --click [MODS-]COL,ROW   Left-click that cell (e.g. ctrl-3,0 opens a link)
        --find <TEXT>            Open the find bar on the active terminal and search for TEXT
        --wait-for <TEXT>        Hold the later steps until the active terminal shows
                                 TEXT (\\n = a line break), for at most 30 step
                                 intervals (30s by default)
                                 Steps run in order, 1s apart
        --step-interval <S>      Run the steps S seconds apart instead (default 1)
        --dump-screen-after <S>  Print the active tab's screen after S seconds and exit
        --latency-log <FILE>     Time each keystroke (key to output, paint and next
                                 frame), append them to FILE as JSON lines, and
                                 print a summary on exit
        --latency-bench <N>      After the steps, type N keystrokes into the active
                                 tab, print latency percentiles and exit
";

/// Shell integration scripts, by shell (`--shell-integration`).
const SHELL_SCRIPTS: &[(&str, &str)] = &[
    ("bash", include_str!("../shell-integration/brindle.bash")),
    ("zsh", include_str!("../shell-integration/brindle.zsh")),
    ("fish", include_str!("../shell-integration/brindle.fish")),
    ("pwsh", include_str!("../shell-integration/brindle.ps1")),
];

#[derive(Debug, Clone)]
enum Step {
    Send(String),
    Action(String),
    /// `--hover` / `--click`, or the reason the position didn't parse.
    Pointer(Result<terminal_view::PointerStep, String>),
    Find(String),
    /// `--wait-for`: the text the active terminal must show before the
    /// steps after it run.
    WaitFor(String),
    /// `--key`: a keystroke through gpui's own key dispatch (bindings, key
    /// listeners, keystroke observers), as a real key press reaches them.
    Key(String),
    /// `--modality`: how focus counts as moved in the window from now on,
    /// as after a pointer press on a control (`pointer`) or a key press
    /// (`keyboard`); it decides whether focus rings show.
    Modality(brindle_ui::Modality),
}

/// How many step intervals a `--wait-for` step waits before giving up (30 s
/// at the default interval, longer when `--step-interval` is stretched).
const WAIT_FOR_INTERVALS: u32 = 30;

/// How long a `--wait-for` step waits before giving up, for the run's step
/// interval.
fn wait_for_limit(interval: Duration) -> Duration {
    interval * WAIT_FOR_INTERVALS
}

/// Where a `--wait-for` step that hasn't found its text stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WaitState {
    /// Still waiting.
    Pending,
    /// Gave up after its limit.
    TimedOut,
}

/// The `--wait-for` steps that haven't found their text, shared by the step
/// runner (which records them) and the dump (which reports them). A wait
/// that finds its text, or whose window closed, leaves no entry.
#[derive(Debug, Default, Clone)]
struct WaitLog {
    entries: std::sync::Arc<std::sync::Mutex<Vec<(u64, String, WaitState)>>>,
}

impl WaitLog {
    /// Records a wait as pending, and returns its handle for [`Self::set`]
    /// and [`Self::clear`].
    fn begin(&self, text: &str) -> u64 {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let id = entries.last().map_or(0, |(id, ..)| id + 1);
        entries.push((id, text.to_string(), WaitState::Pending));
        id
    }

    fn set(&self, id: u64, state: WaitState) {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = entries.iter_mut().find(|(i, ..)| *i == id) {
            entry.2 = state;
        }
    }

    fn clear(&self, id: u64) {
        self.entries.lock().unwrap_or_else(|e| e.into_inner()).retain(|(i, ..)| *i != id);
    }

    /// The dump's lines: `--- wait-for timeout "TEXT"` for each wait that
    /// timed out and `--- wait-for pending "TEXT"` for one still waiting,
    /// the text escaped onto one line as the selection line is.
    fn describe(&self) -> String {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries
            .iter()
            .map(|(_, text, state)| {
                let state = match state {
                    WaitState::Pending => "pending",
                    WaitState::TimedOut => "timeout",
                };
                format!("--- wait-for {state} {text:?}\n")
            })
            .collect()
    }
}

#[derive(Default, Debug)]
struct Args {
    command: Option<(String, Vec<String>)>,
    profile: Option<String>,
    cwd: Option<PathBuf>,
    /// `--send` / `--action` / `--find` / `--wait-for` steps, run in order one
    /// second apart.
    steps: Vec<Step>,
    /// `--step-interval`: the steps' spacing in seconds, when not one.
    step_interval: Option<f64>,
    dump_after: Option<f64>,
    latency_log: Option<PathBuf>,
    latency_bench: Option<usize>,
    desktop: Option<desktop::Request>,
    no_restore: bool,
    /// `--attach`: the terminal server session the window opens with.
    attach: Option<u32>,
}

impl Args {
    /// Whether the launch asks for a tab of its own: a profile, directory or
    /// command (`--attach` is its own request).
    fn asks_for_a_tab(&self) -> bool {
        self.profile.is_some() || self.cwd.is_some() || self.command.is_some()
    }

    /// A run driven by debug options, which runs on its own beside a
    /// running Brindle instead of handing it the request.
    fn is_debug_run(&self) -> bool {
        !self.steps.is_empty() || self.dump_after.is_some() || self.latency_log.is_some() || self.latency_bench.is_some()
    }

    /// This launch as a request to the running Brindle.
    fn instance_request(&self) -> instance::Request {
        let here = std::env::current_dir().ok();
        instance::Request {
            profile: self.profile.clone(),
            // Relative to where this launch runs, not the instance.
            cwd: self.cwd.as_ref().map(|d| here.as_ref().map(|h| h.join(d)).unwrap_or_else(|| d.clone())),
            command: self.command.as_ref().map(|(program, args)| std::iter::once(program.clone()).chain(args.iter().cloned()).collect()),
            attach: self.attach,
            launch_dir: here,
            activation_token: instance::Request::launch_token(),
        }
    }
}

fn parse_args() -> Args {
    parse_args_from(std::env::args().skip(1))
}

fn parse_args_from(mut it: impl Iterator<Item = String>) -> Args {
    let mut args = Args::default();
    let (mut install_desktop, mut uninstall_desktop, mut prefix) = (false, false, None);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                console_output();
                print!("{HELP}");
                std::process::exit(0);
            }
            "-V" | "--version" => {
                console_output();
                println!("brindle {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--list-actions" => {
                console_output();
                for name in actions::action_names() {
                    println!("{name}");
                }
                std::process::exit(0);
            }
            "--list-themes" => {
                console_output();
                for name in theme::BUILTIN_THEMES {
                    println!("{name}");
                }
                std::process::exit(0);
            }
            "--shell-integration" => {
                let shell = it.next().unwrap_or_default();
                match SHELL_SCRIPTS.iter().find(|(name, _)| *name == shell) {
                    Some((_, script)) => {
                        console_output();
                        print!("{script}");
                        std::process::exit(0);
                    }
                    None => {
                        let names: Vec<&str> = SHELL_SCRIPTS.iter().map(|(name, _)| *name).collect();
                        usage_error(&format!(
                            "--shell-integration needs a shell: {} (got {shell:?})",
                            names.join(", ")
                        ))
                    }
                }
            }
            "-e" | "--command" => {
                let rest: Vec<String> = it.by_ref().collect();
                if let Some((program, rest)) = rest.split_first() {
                    args.command = Some((program.clone(), rest.to_vec()));
                }
            }
            "-p" | "--profile" => args.profile = it.next(),
            "-d" | "--working-directory" => args.cwd = it.next().map(|d| config::expand_tilde(&d)),
            "--config" => {
                if let Some(path) = it.next() {
                    // SAFETY: single-threaded at this point.
                    unsafe { std::env::set_var("BRINDLE_CONFIG", path) };
                }
            }
            "--no-restore" => args.no_restore = true,
            "--attach" if cfg!(not(target_os = "linux")) => {
                usage_error("--attach needs the terminal server, which runs only on Linux")
            }
            "--attach" => match it.next().and_then(|id| id.parse().ok()) {
                Some(id) => args.attach = Some(id),
                None => usage_error("--attach needs a session id (brindle-remote list prints them)"),
            },
            "--send" => args.steps.extend(it.next().map(|s| Step::Send(unescape(&s)))),
            "--action" => args.steps.extend(it.next().map(Step::Action)),
            "--hover" | "--click" => {
                let click = arg == "--click";
                let pos = it.next().unwrap_or_default();
                args.steps.push(Step::Pointer(terminal_view::PointerStep::parse(&pos, click)));
            }
            "--find" => args.steps.extend(it.next().map(Step::Find)),
            "--wait-for" => args.steps.extend(it.next().map(|s| Step::WaitFor(unescape(&s)))),
            "--key" => args.steps.extend(it.next().map(Step::Key)),
            "--modality" => match it.next().as_deref() {
                Some("pointer") => args.steps.push(Step::Modality(brindle_ui::Modality::Pointer)),
                Some("keyboard") => args.steps.push(Step::Modality(brindle_ui::Modality::Keyboard)),
                _ => usage_error("--modality needs pointer or keyboard"),
            },
            "--step-interval" => match it.next().and_then(|s| s.parse::<f64>().ok()) {
                Some(secs) if secs > 0.0 && secs.is_finite() => args.step_interval = Some(secs),
                _ => usage_error("--step-interval needs a positive number of seconds"),
            },
            "--dump-screen-after" => args.dump_after = it.next().and_then(|s| s.parse().ok()),
            "--latency-log" => match it.next() {
                Some(path) => args.latency_log = Some(config::expand_tilde(&path)),
                None => usage_error("--latency-log needs a file"),
            },
            "--latency-bench" => match it.next().and_then(|n| n.parse().ok()) {
                Some(n) if n > 0 => args.latency_bench = Some(n),
                _ => usage_error("--latency-bench needs a positive number of keystrokes"),
            },
            "--install-desktop" => install_desktop = true,
            "--uninstall-desktop" => uninstall_desktop = true,
            "--prefix" => match it.next() {
                Some(dir) => prefix = Some(config::expand_tilde(&dir)),
                None => usage_error("--prefix needs a directory"),
            },
            other => usage_error(&format!("unknown argument {other:?}")),
        }
    }
    match desktop::request(install_desktop, uninstall_desktop, prefix) {
        Ok(request) => args.desktop = request,
        Err(message) => usage_error(message),
    }
    args
}

fn usage_error(message: &str) -> ! {
    console_output();
    eprintln!("brindle: {message}\n\n{HELP}");
    std::process::exit(2);
}

/// Called before printing on a path that exits right after. On Windows,
/// where Brindle is a GUI program with no console of its own, it attaches
/// to the console Brindle was started from. A GUI session never attaches
/// while it runs, so ctrl-c in that console, or closing it, can't end it.
fn console_output() {
    #[cfg(windows)]
    win::attach_parent_console();
}

fn unescape(s: &str) -> String {
    s.replace("\\r", "\r").replace("\\n", "\n").replace("\\e", "\x1b").replace("\\t", "\t")
}

pub fn open_window(launch: Launch, cx: &mut App) -> Option<gpui::WindowHandle<Workspace>> {
    let (mut width, mut height, mut maximized) = (1000.0, 660.0, false);
    if let Launch::Restore(state) = &launch {
        width = state.width.unwrap_or(width).max(session::MIN_SIZE.0);
        height = state.height.unwrap_or(height).max(session::MIN_SIZE.1);
        maximized = state.maximized;
    }
    let bounds = Bounds::centered(None, size(px(width), px(height)), cx);
    let options = WindowOptions {
        window_bounds: Some(if maximized { WindowBounds::Maximized(bounds) } else { WindowBounds::Windowed(bounds) }),
        titlebar: Some(gpui::TitlebarOptions {
            title: Some("Brindle".into()),
            appears_transparent: true,
            traffic_light_position: None,
        }),
        window_background: WindowBackgroundAppearance::Opaque,
        window_decorations: Some(WindowDecorations::Client),
        window_min_size: Some(size(px(360.0), px(200.0))),
        app_id: Some("brindle".into()),
        ..Default::default()
    };
    match cx.open_window(options, |window, cx| cx.new(|cx| Workspace::new(launch, window, cx))) {
        Ok(handle) => {
            cx.activate(true);
            Some(handle)
        }
        Err(err) => {
            log::error!("failed to open window: {err:#}");
            None
        }
    }
}

fn reload_config(cx: &mut App) {
    reload_config_renaming(cx, &[]);
}

/// Reloads the config into every window. `renames` are profiles renamed in
/// the settings dialog, whose open tabs follow the new name.
pub(crate) fn reload_config_renaming(cx: &mut App, renames: &[(String, String)]) {
    let (config, error) = Config::load_or_default();
    let zoom = Settings::get(cx).zoom;
    let mut settings = Settings::new(config, error, cx);
    settings.zoom = zoom;
    cx.set_global(settings);
    Settings::apply_ui_theme(cx);
    actions::bind_keys(cx);
    for window in cx.windows() {
        if let Some(window) = window.downcast::<Workspace>() {
            window.update(cx, |workspace, _, cx| workspace.apply_config(renames, cx)).ok();
        }
    }
    cx.refresh_windows();
}

fn config_mtime() -> Option<SystemTime> {
    std::fs::metadata(Config::path()).and_then(|m| m.modified()).ok()
}

/// Reloads the config whenever the file changes on disk.
fn watch_config(cx: &mut App) {
    cx.spawn(async move |cx| {
        let mut last = config_mtime();
        loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let now = config_mtime();
            if now != last {
                last = now;
                log::info!("config changed, reloading");
                if cx.update(reload_config).is_err() {
                    break;
                }
            }
        }
    })
    .detach();
}

/// A later launch's request, handed from the instance socket's thread to
/// the UI thread.
struct Forwarded {
    request: instance::Request,
    /// For `--attach`: the server's sessions, listed on the socket's thread.
    listing: Option<Option<Vec<remote::protocol::client::SessionInfo>>>,
    reply: std::sync::mpsc::Sender<Result<(), String>>,
}

/// Opens later launches' requests in `window` and raises it.
fn take_launches(
    window: gpui::WindowHandle<Workspace>,
    mut requests: futures::channel::mpsc::UnboundedReceiver<Forwarded>,
    cx: &mut App,
) {
    use futures::StreamExt as _;
    cx.spawn(async move |cx| {
        while let Some(Forwarded { request, listing, reply }) = requests.next().await {
            let result = window
                .update(cx, |ws, window, cx| {
                    if let Some(launch) = forwarded_launch(&request, listing, &Settings::get(cx).config) {
                        ws.open(launch, window, cx);
                    }
                    notify::activation::raise(window, request.activation_token.as_deref());
                })
                .map_err(|_| "Brindle's window is gone".to_string());
            reply.send(result).ok();
        }
    })
    .detach();
}

/// What a later launch's request opens: nothing for a plain launch, which
/// only raises the window.
fn forwarded_launch(
    request: &instance::Request,
    listing: Option<Option<Vec<remote::protocol::client::SessionInfo>>>,
    config: &Config,
) -> Option<Launch> {
    if let Some(session) = request.attach {
        let target = match listing.flatten() {
            Some(listing) => reattach::attach_target(session, &listing),
            None => Err("Brindle could not list the terminal server's sessions".into()),
        };
        return Some(Launch::Attach { session, target });
    }
    if request.profile.is_none() && request.cwd.is_none() && request.command.is_none() {
        return None;
    }
    let profile = match &request.profile {
        Some(name) => config.profile_index(name).unwrap_or_else(|| {
            log::warn!("no profile named {name:?}");
            config.default_profile_index()
        }),
        None => config.default_profile_index(),
    };
    let command = request.command.as_ref().and_then(|c| c.split_first()).map(|(p, rest)| (p.clone(), rest.to_vec()));
    // A command starts where it was typed, unless `-d` says otherwise.
    let cwd = request.cwd.clone().or_else(|| command.as_ref().and(request.launch_dir.clone()));
    Some(Launch::Request(LaunchRequest { profile, command, cwd }))
}

/// The running terminal server's sessions, attached or not: none when no
/// server runs (none is started), `None` when listing fails (logged).
fn running_sessions() -> Option<Vec<remote::protocol::client::SessionInfo>> {
    #[cfg(target_os = "linux")]
    match remote::local::running_sessions(Duration::from_secs(2)) {
        Ok(sessions) => Some(sessions),
        Err(why) => {
            log::warn!("not attaching detached sessions: {why:#}");
            None
        }
    }
    #[cfg(not(target_os = "linux"))]
    Some(Vec::new())
}

/// The recorded windows, merged into the one window Brindle has, without
/// the zones whose sessions are gone (when `listed`: the `Detached` global
/// holds the launch's listing); `None` when the state file is missing,
/// unusable (logged) or leaves nothing to restore.
fn restorable_window(listed: bool, cx: &mut App) -> Option<WindowState> {
    let state = match session::load(&session::state_dir()) {
        Ok(state) => state?,
        Err(why) => {
            log::warn!("{why}");
            return None;
        }
    };
    let window = session::merge_windows(state.windows)?;
    if !listed {
        return Some(window);
    }
    let detached = cx.global::<reattach::Detached>();
    session::drop_lost_zones(window, |recorded| detached.has(recorded))
}

/// Writes the session state when it changes (off the UI thread), and once
/// more, synchronously, when Brindle quits.
fn record_session(cx: &mut App) {
    fn pending(cx: &mut App) -> Option<String> {
        cx.update_global(|store: &mut SessionStore, _| {
            if store.is_owner() { store.pending_write() } else { None }
        })
    }
    fn write(text: &str, generation: u64) {
        let dir = session::state_dir();
        if let Err(err) = session::write_atomic(&dir, text, generation) {
            log::warn!("could not write session state to {}: {err}", session::state_path(&dir).display());
        }
    }
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let Ok(text) = cx.update(pending) else { break };
            if let Some(text) = text {
                let generation = session::next_generation();
                cx.background_executor().spawn(async move { write(&text, generation) }).detach();
            }
        }
    })
    .detach();
    cx.on_app_quit(|cx| {
        // The windows still open now are the ones to keep.
        for window in cx.windows() {
            if let Some(window) = window.downcast::<Workspace>() {
                window.update(cx, |ws, window, cx| ws.record_session(window, cx)).ok();
            }
        }
        if let Some(text) = pending(cx) {
            write(&text, session::next_generation());
        }
        async {}
    })
    .detach();
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("brindle=info,warn"))
        .init();
    let args = parse_args();
    // Desktop registration needs no display, so it runs before GPUI starts.
    if let Some(request) = &args.desktop {
        console_output();
        if let Err(err) = desktop::run(request) {
            eprintln!("brindle: {err:#}");
            std::process::exit(1);
        }
        return;
    }

    // One Brindle per state directory: the process that takes the session
    // lock is the instance, and a later launch hands it its request and
    // exits. A debug run beside it runs on its own instead.
    let state_dir = session::state_dir();
    let lock = match session::Lock::acquire(&state_dir) {
        Ok(lock) => Some(lock),
        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock && !args.is_debug_run() => {
            match instance::forward(&state_dir, &args.instance_request()) {
                Ok(()) => std::process::exit(0),
                // The running Brindle may have quit meanwhile.
                Err(why) => match session::Lock::acquire(&state_dir) {
                    Ok(lock) => {
                        log::info!("the running Brindle went away ({why}); starting");
                        Some(lock)
                    }
                    Err(_) => {
                        console_output();
                        eprintln!("brindle: {why}");
                        std::process::exit(1);
                    }
                },
            }
        }
        Err(err) => {
            log::info!(
                "another Brindle owns the session in {} ({err}); not restoring or recording",
                state_dir.display()
            );
            None
        }
    };
    let (forwarded, requests) = futures::channel::mpsc::unbounded::<Forwarded>();
    let mut socket = None;
    if lock.is_some() {
        let handler = move |request: instance::Request| {
            // Listed here, off the UI thread.
            let listing = request.attach.map(|_| running_sessions());
            let (reply, answer) = std::sync::mpsc::channel();
            forwarded
                .unbounded_send(Forwarded { request, listing, reply })
                .map_err(|_| "Brindle is quitting".to_string())?;
            answer
                .recv_timeout(instance::FORWARD_LIMIT)
                .map_err(|_| "Brindle didn't take the request in time".to_string())?
        };
        match instance::listen(&state_dir, Box::new(handler)) {
            Ok(path) => {
                log::debug!("taking launches at {}", path.display());
                socket = Some(path);
            }
            Err(err) => log::warn!("later launches can't reach this Brindle: {err}"),
        }
    }

    // Before any window: windows, shortcuts and toasts share one identity.
    #[cfg(windows)]
    win::set_app_id();

    Application::new().run(move |cx: &mut App| {
        let (config, error) = Config::load_or_default();
        let settings = Settings::new(config, error, cx);
        let profile = match &args.profile {
            Some(name) => settings.config.profile_index(name).unwrap_or_else(|| {
                log::warn!("no profile named {name:?}");
                settings.config.default_profile_index()
            }),
            None => settings.config.default_profile_index(),
        };
        cx.set_global(settings);
        Settings::apply_ui_theme(cx);
        notify::Notifier::init(cx);
        actions::bind_keys(cx);
        // brindle-ui's keystroke observer: a key sets keyboard modality, so
        // focus rings come back after a click had hidden them.
        brindle_ui::init(cx);
        // RUST_LOG=brindle=debug shows every keystroke and what it ran.
        cx.observe_keystrokes(|e, _, _| {
            let action = e.action.as_ref().map(|a| a.name()).unwrap_or("no action");
            log::debug!("keystroke {} -> {action}", e.keystroke.unparse());
        })
        .detach();
        if args.latency_log.is_some() || args.latency_bench.is_some() {
            if let Err(err) = Latency::install(args.latency_log.as_deref(), cx) {
                console_output();
                eprintln!("brindle: --latency-log: {err}");
                std::process::exit(1);
            }
            cx.on_app_quit(|cx| {
                if let Some(summary) = Latency::log_summary(cx) {
                    console_output();
                    eprint!("{summary}");
                }
                async {}
            })
            .detach();
        }

        // Deferred, like ReloadConfig: the prompt opens in a window.
        cx.on_action(|_: &Quit, cx| cx.quit());
        // Global listeners run inside the dispatching window's update, where
        // `reload_config` couldn't update that window; defer until it's back.
        cx.on_action(|_: &ReloadConfig, cx| cx.defer(reload_config));
        cx.on_window_closed(|cx| {
            let live: Vec<u64> = cx.windows().iter().map(|w| w.window_id().as_u64()).collect();
            cx.update_global(|store: &mut SessionStore, _| store.prune(&live));
            if live.is_empty() {
                // Not inside the close: when the window manager closed the
                // window, GPUI's X11 client is still borrowed here, and
                // quitting needs it.
                cx.spawn(async |cx| cx.update(|cx| cx.quit()).ok()).detach();
            }
        })
        .detach();

        cx.set_global(SessionStore::new(lock));
        // The next instance would replace it anyway; this keeps the runtime
        // directory tidy.
        #[cfg(unix)]
        if let Some(socket) = socket {
            cx.on_app_quit(move |_| {
                std::fs::remove_file(&socket).ok();
                async {}
            })
            .detach();
        }
        #[cfg(not(unix))]
        let _ = socket;
        // The Brindle that owns the session state restores it and attaches
        // the server's detached sessions (restored zones take theirs, the
        // rest become tabs), unless `--no-restore`. Then it opens what the
        // launch asked for as the active tab.
        let restoring = !args.no_restore && cx.update_global(|store: &mut SessionStore, _| store.is_owner());
        let listed = if restoring || args.attach.is_some() { running_sessions() } else { Some(Vec::new()) };
        let detached = listed
            .iter()
            .flatten()
            .filter(|s| restoring && !s.attached && Some(s.session) != args.attach)
            .cloned()
            .collect();
        cx.set_global(reattach::Detached::new(detached));
        // Without a listing, nothing tells a lost zone from a live one, so
        // none is dropped.
        let restored = if restoring { restorable_window(listed.is_some(), cx) } else { None };
        let request = match args.attach {
            Some(session) => Some(Launch::Attach { session, target: reattach::attach_target(session, listed.as_deref().unwrap_or_default()) }),
            None if args.asks_for_a_tab() => {
                Some(Launch::Request(LaunchRequest { profile, command: args.command.clone(), cwd: args.cwd.clone() }))
            }
            None => None,
        };
        let (first, request) = match restored {
            Some(state) => {
                log::info!("restoring {} tab(s) from {}", state.tabs.len(), session::state_path(&session::state_dir()).display());
                (Launch::Restore(state), request)
            }
            None => (request.unwrap_or_else(|| Launch::Request(LaunchRequest::profile(profile))), None),
        };
        let Some(window) = open_window(first, cx) else {
            cx.quit();
            return;
        };
        let rest = cx.update_global(|detached: &mut reattach::Detached, _| detached.take_rest());
        if !rest.is_empty() {
            log::info!("attaching {} detached session(s)", rest.len());
            window.update(cx, |ws, window, cx| ws.attach_detached(rest, window, cx)).ok();
        }
        if let Some(request) = request {
            window.update(cx, |ws, window, cx| ws.open(request, window, cx)).ok();
        }
        watch_config(cx);
        record_session(cx);
        take_launches(window, requests, cx);

        let steps = args.steps.clone();
        let latency_bench = args.latency_bench;
        let interval = Duration::from_secs_f64(args.step_interval.unwrap_or(1.0));
        let waits = WaitLog::default();
        let step_waits = waits.clone();
        cx.spawn(async move |cx| {
            cx.background_executor().timer(interval / 2).await;
            for step in steps {
                cx.background_executor().timer(interval).await;
                if let Step::WaitFor(text) = &step {
                    wait_for_text(window, text, wait_for_limit(interval), &step_waits, cx).await;
                    continue;
                }
                if let Step::Key(key) = &step {
                    // Not inside the workspace's update: the key's actions
                    // update it themselves.
                    let key = key.clone();
                    cx.update_window(window.into(), move |_, window, cx| match Keystroke::parse(&key) {
                        Ok(keystroke) => {
                            window.dispatch_keystroke(keystroke, cx);
                        }
                        Err(err) => log::error!("--key: bad keystroke {key:?}: {err:?}"),
                    })
                    .ok();
                    continue;
                }
                window
                    .update(cx, |ws, window, cx| match &step {
                        Step::Send(text) => ws.send_to_active(text.as_bytes(), window, cx),
                        Step::Find(text) => ws.find_in_active(text, window, cx),
                        Step::Action(name) => match actions::action_by_name(name) {
                            Some(action) => window.dispatch_action(action, cx),
                            None => log::error!("--action: unknown action {name:?}"),
                        },
                        Step::Pointer(Ok(step)) => ws.debug_pointer(step, window, cx),
                        Step::Pointer(Err(why)) => log::error!("--hover/--click: {why}"),
                        Step::Modality(modality) => brindle_ui::set_modality(window, *modality, cx),
                        Step::WaitFor(_) | Step::Key(_) => {}
                    })
                    .ok();
            }
            if let Some(samples) = latency_bench {
                latency_bench_run(window, samples, cx).await;
                cx.update(|cx| {
                    console_output();
                    print!("{}", Latency::summary(cx));
                    cx.quit();
                })
                .ok();
            }
        })
        .detach();
        if let Some(secs) = args.dump_after {
            cx.spawn(async move |cx| {
                cx.background_executor().timer(Duration::from_secs_f64(secs)).await;
                console_output();
                window
                    .update(cx, |ws, window, cx| {
                        // The wait lines come last, after the focus line, so
                        // a dump that shows an unexpected screen says which
                        // wait failed.
                        print!("{}{}", ws.dump_active_screen(window, cx), waits.describe());
                    })
                    .ok();
                cx.update(|cx| cx.quit()).ok();
            })
            .detach();
        }
    });
}

/// `--wait-for`: polls the active terminal's screen until it shows `text`,
/// for at most `limit` ([`wait_for_limit`]). `waits` shows it as pending
/// while it waits, and as timed out if it gives up.
async fn wait_for_text(
    window: gpui::WindowHandle<Workspace>,
    text: &str,
    limit: Duration,
    waits: &WaitLog,
    cx: &mut gpui::AsyncApp,
) {
    let deadline = std::time::Instant::now() + limit;
    let id = waits.begin(text);
    loop {
        let shown = window.update(cx, |ws, _, cx| ws.active_screen_text(cx).is_some_and(|screen| screen.contains(text)));
        match shown {
            // A closed window has nothing to wait for.
            Ok(true) | Err(_) => {
                waits.clear(id);
                return;
            }
            Ok(false) if std::time::Instant::now() >= deadline => {
                log::warn!("--wait-for: {text:?} didn't appear within {limit:?}");
                waits.set(id, WaitState::TimedOut);
                return;
            }
            Ok(false) => cx.background_executor().timer(Duration::from_millis(100)).await,
        }
    }
}

/// `--latency-bench`: types `samples` keystrokes (after a warm-up) into the
/// active tab through GPUI's own key dispatch, the path real key presses
/// take, one at a time with a randomized gap so they don't line up with
/// frames.
async fn latency_bench_run(window: gpui::WindowHandle<Workspace>, samples: usize, cx: &mut gpui::AsyncApp) {
    const WARMUP: usize = 10;
    let mut seed = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(1, |d| d.as_nanos() as u64) | 1;
    let mut gap = move || {
        // xorshift64
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        Duration::from_millis(10 + seed % 31)
    };
    let mut waiting_for_tab = 0;
    let mut i = 0;
    while i < WARMUP + samples {
        if i == WARMUP {
            cx.update(Latency::reset).ok();
        }
        let key = if i % 2 == 0 { "x" } else { "backspace" };
        let keystroke = gpui::Keystroke::parse(key).expect("valid keystroke");
        let Ok(before) = cx.update(|cx| Latency::started(cx)) else { return };
        // Not `window.update`: that holds the Workspace, and a key handled
        // by the Workspace itself would then panic.
        if cx.update_window(window.into(), |_, window, cx| window.dispatch_keystroke(keystroke, cx)).is_err() {
            return;
        }
        if cx.update(|cx| Latency::started(cx)).ok() == Some(before) {
            // No terminal took it: the tab isn't ready yet.
            waiting_for_tab += 1;
            if waiting_for_tab > 50 {
                log::error!("--latency-bench: the active tab doesn't take keystrokes");
                return;
            }
            cx.background_executor().timer(Duration::from_millis(200)).await;
            continue;
        }
        loop {
            cx.background_executor().timer(Duration::from_millis(1)).await;
            match cx.update(Latency::poll) {
                Ok(Some((_, true))) => break,
                Ok(Some(_)) => {}
                _ => return,
            }
        }
        cx.background_executor().timer(gap()).await;
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Args {
        parse_args_from(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn profile_directory_and_command_ask_for_a_tab() {
        for args in [&[][..], &["--send", "x", "--dump-screen-after", "3"], &["--no-restore"]] {
            assert!(!parse(args).asks_for_a_tab(), "{args:?}");
        }
        for args in [&["-p", "work"][..], &["-d", "/tmp"], &["-e", "htop"]] {
            assert!(parse(args).asks_for_a_tab(), "{args:?}");
        }
    }

    #[test]
    fn debug_options_make_a_debug_run() {
        assert!(!parse(&["-p", "work", "-e", "htop"]).is_debug_run());
        for args in [
            &["--send", "x"][..],
            &["--action", "new_tab"],
            &["--dump-screen-after", "2"],
            &["--latency-bench", "5"],
            &["--modality", "pointer"],
        ] {
            assert!(parse(args).is_debug_run(), "{args:?}");
        }
    }

    #[test]
    fn the_wait_limit_is_thirty_step_intervals() {
        assert_eq!(wait_for_limit(Duration::from_secs(1)), Duration::from_secs(30), "the default interval");
        assert_eq!(wait_for_limit(Duration::from_secs(2)), Duration::from_secs(60));
        assert_eq!(wait_for_limit(Duration::from_secs_f64(0.1)), Duration::from_secs(3));
        let stretched = parse(&["--step-interval", "2", "--wait-for", "x"]);
        assert_eq!(wait_for_limit(Duration::from_secs_f64(stretched.step_interval.unwrap_or(1.0))), Duration::from_secs(60));
    }

    #[test]
    fn the_wait_log_describes_pending_and_timed_out_waits() {
        let waits = WaitLog::default();
        assert_eq!(waits.describe(), "", "no wait step, no line");
        let found = waits.begin("found");
        waits.clear(found);
        assert_eq!(waits.describe(), "", "a wait that found its text leaves nothing");
        let first = waits.begin("NEVER");
        assert_eq!(waits.describe(), "--- wait-for pending \"NEVER\"\n");
        waits.set(first, WaitState::TimedOut);
        waits.begin("two\nlines \"q\"");
        assert_eq!(
            waits.describe(),
            "--- wait-for timeout \"NEVER\"\n--- wait-for pending \"two\\nlines \\\"q\\\"\"\n",
            "text is escaped onto one line"
        );
    }

    #[test]
    fn key_and_modality_steps_keep_their_order() {
        let args = parse(&["--key", "shift-tab", "--modality", "pointer", "--action", "x", "--modality", "keyboard"]);
        assert!(args.is_debug_run());
        let steps: Vec<String> = args.steps.iter().map(|s| format!("{s:?}")).collect();
        assert_eq!(steps, [r#"Key("shift-tab")"#, "Modality(Pointer)", r#"Action("x")"#, "Modality(Keyboard)"]);
    }

    #[test]
    fn a_launch_becomes_a_request_for_the_instance() {
        let request = parse(&["-p", "work", "-d", "rel", "-e", "htop", "-d", "1"]).instance_request();
        let here = std::env::current_dir().unwrap();
        assert_eq!(request.profile.as_deref(), Some("work"));
        assert_eq!(request.cwd, Some(here.join("rel")), "-d is made absolute here");
        assert_eq!(request.command, Some(vec!["htop".into(), "-d".into(), "1".into()]));
        assert_eq!(request.launch_dir, Some(here));
    }

    #[test]
    fn forwarded_requests_open_tabs() {
        let config = Config::parse("default_profile = \"a\"\n[[profiles]]\nname = \"a\"\n[[profiles]]\nname = \"b\"\n").unwrap();
        let request = |r: instance::Request| forwarded_launch(&r, None, &config);
        assert!(request(instance::Request { launch_dir: Some("/x".into()), ..Default::default() }).is_none(), "a plain launch only raises");
        let Some(Launch::Request(tab)) = request(instance::Request {
            command: Some(vec!["htop".into()]),
            launch_dir: Some("/srv".into()),
            ..Default::default()
        }) else {
            panic!("a command opens a tab")
        };
        assert_eq!((tab.profile, tab.command, tab.cwd), (0, Some(("htop".into(), vec![])), Some("/srv".into())));
        let Some(Launch::Request(tab)) =
            request(instance::Request { profile: Some("b".into()), launch_dir: Some("/srv".into()), ..Default::default() })
        else {
            panic!("a profile opens a tab")
        };
        assert_eq!((tab.profile, tab.cwd), (1, None), "only a command starts where it was typed");
        let Some(Launch::Attach { session: 7, target: Err(_) }) =
            request(instance::Request { attach: Some(7), ..Default::default() })
        else {
            panic!("--attach without a listing says why")
        };
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn attach_takes_a_session_id() {
        let args = parse(&["--attach", "7", "--dump-screen-after", "2"]);
        assert_eq!(args.attach, Some(7));
        assert!(!args.asks_for_a_tab());
        assert_eq!(parse(&[]).attach, None);
    }
}
