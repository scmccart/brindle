// A GUI program on Windows, so launching it opens no console window (see
// `console_output`). Test builds keep the console subsystem.
#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]
// railway R3, types R2: no `_ =>` on one of Brindle's own enums. Non-test
// code only; `docs/architecture/README.md` lists the exceptions.
#![cfg_attr(not(test), warn(clippy::wildcard_enum_match_arm))]

mod actions;
mod config;
mod config_edit;
mod desktop;
mod find_bar;
mod instance;
mod latency;
mod layout;
mod notify;
mod picker;
mod primary;
mod queue_marks;
mod reattach;
mod remote;
mod session;
mod settings;
mod settings_dialog;
mod terminal;
mod terminal_element;
mod terminal_view;
#[cfg(test)]
mod test_support;
mod theme;
#[cfg(windows)]
mod win;
mod workspace;
mod zone_view;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use gpui::{
    App, AppContext as _, Application, BorrowAppContext as _, Bounds, Global, Keystroke, WindowBackgroundAppearance,
    WindowBounds, WindowDecorations, WindowOptions, px, size,
};

use crate::actions::{Quit, ReloadConfig};
use crate::config::{Config, ConfigReport};
use crate::latency::Latency;
use crate::reattach::Listing;
use crate::remote::hosts::Hosts;
use crate::remote::protocol::client::SessionInfo;
use crate::settings::Settings;
use crate::session::{PendingWrite, SessionStore, WindowState};
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
    /// `--action`: an action's name, checked against the action table when
    /// the arguments were parsed.
    Action(String),
    /// `--hover` / `--click`.
    Pointer(terminal_view::PointerStep),
    Find(String),
    /// `--wait-for`: the text the active terminal must show before the
    /// steps after it run.
    WaitFor(String),
    /// `--key`: a keystroke through gpui's own key dispatch (bindings, key
    /// listeners, keystroke observers), as a real key press reaches them.
    Key(Keystroke),
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
    waits: std::sync::Arc<std::sync::Mutex<Waits>>,
}

/// What a [`WaitLog`] holds: its entries, and the handle the next wait
/// gets. Handles are never given out twice, so a wait that ended can't
/// reach a later one.
#[derive(Debug, Default)]
struct Waits {
    next: u64,
    entries: Vec<(u64, String, WaitState)>,
}

impl WaitLog {
    fn lock(&self) -> std::sync::MutexGuard<'_, Waits> {
        self.waits.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Records a wait as pending, and returns its handle for [`Self::set`]
    /// and [`Self::clear`].
    fn begin(&self, text: &str) -> u64 {
        let mut waits = self.lock();
        let id = waits.next;
        waits.next += 1;
        waits.entries.push((id, text.to_string(), WaitState::Pending));
        id
    }

    fn set(&self, id: u64, state: WaitState) {
        if let Some(entry) = self.lock().entries.iter_mut().find(|(i, ..)| *i == id) {
            entry.2 = state;
        }
    }

    fn clear(&self, id: u64) {
        self.lock().entries.retain(|(i, ..)| *i != id);
    }

    /// The dump's lines: `--- wait-for timeout "TEXT"` for each wait that
    /// timed out and `--- wait-for pending "TEXT"` for one still waiting,
    /// the text escaped onto one line as the selection line is.
    fn describe(&self) -> String {
        self.lock()
            .entries
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
            activation_token: instance::launch_token(
                std::env::var("XDG_ACTIVATION_TOKEN").ok(),
                std::env::var("DESKTOP_STARTUP_ID").ok(),
            ),
        }
    }
}

fn parse_args() -> Args {
    parse_args_from(std::env::args().skip(1), dirs::home_dir().as_deref()).unwrap_or_else(|message| usage_error(&message))
}

/// The value `option` takes: the next argument, or a usage error naming
/// the option and `what` it needs.
fn value_of(it: &mut impl Iterator<Item = String>, option: &str, what: &str) -> Result<String, String> {
    it.next().ok_or_else(|| format!("{option} needs {what}"))
}

/// The arguments, with a leading `~` in a path expanded to `home`, or the
/// usage error to print: an unknown argument, an option without its value,
/// or a value it can't use, each naming the option. The debug steps are
/// checked here, so a run never starts with one it can't take.
fn parse_args_from(mut it: impl Iterator<Item = String>, home: Option<&Path>) -> Result<Args, String> {
    let mut args = Args::default();
    let (mut install_desktop, mut uninstall_desktop, mut prefix) = (false, false, None);
    while let Some(arg) = it.next() {
        let it = &mut it;
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
                        return Err(format!(
                            "--shell-integration needs a shell: {} (got {shell:?})",
                            names.join(", ")
                        ));
                    }
                }
            }
            "-e" | "--command" => {
                let rest: Vec<String> = it.collect();
                let Some((program, rest)) = rest.split_first() else {
                    return Err(format!("{arg} needs a command"));
                };
                args.command = Some((program.clone(), rest.to_vec()));
            }
            "-p" | "--profile" => args.profile = Some(value_of(it, &arg, "a profile name")?),
            "-d" | "--working-directory" => {
                args.cwd = Some(config::expand_tilde(&value_of(it, &arg, "a directory")?, home));
            }
            "--config" => {
                let path = value_of(it, &arg, "a file")?;
                // SAFETY: single-threaded at this point.
                unsafe { std::env::set_var("BRINDLE_CONFIG", path) };
            }
            "--no-restore" => args.no_restore = true,
            "--attach" if cfg!(not(any(target_os = "linux", windows))) => {
                return Err("--attach needs the terminal server, which runs only on Linux and Windows".into());
            }
            "--attach" => match it.next().and_then(|id| id.parse().ok()) {
                Some(id) => args.attach = Some(id),
                None => return Err("--attach needs a session id (brindle-remote list prints them)".into()),
            },
            "--send" => args.steps.push(Step::Send(unescape(&value_of(it, &arg, "text to type")?))),
            "--action" => {
                let name = value_of(it, &arg, "an action name (see --list-actions)")?;
                if actions::action_by_name(&name).is_none() {
                    return Err(format!("{arg}: no action is named {name:?} (see --list-actions)"));
                }
                args.steps.push(Step::Action(name));
            }
            "--hover" | "--click" => {
                let pos = value_of(it, &arg, "a cell, [MODS-]COL,ROW")?;
                let step = terminal_view::PointerStep::parse(&pos, arg == "--click").map_err(|why| format!("{arg}: {why}"))?;
                args.steps.push(Step::Pointer(step));
            }
            "--find" => args.steps.push(Step::Find(value_of(it, &arg, "text to find")?)),
            "--wait-for" => args.steps.push(Step::WaitFor(unescape(&value_of(it, &arg, "text to wait for")?))),
            "--key" => {
                let key = value_of(it, &arg, "a keystroke (e.g. shift-tab)")?;
                let keystroke = Keystroke::parse(&key).map_err(|why| format!("{arg}: {why}"))?;
                args.steps.push(Step::Key(keystroke));
            }
            "--modality" => match it.next().as_deref() {
                Some("pointer") => args.steps.push(Step::Modality(brindle_ui::Modality::Pointer)),
                Some("keyboard") => args.steps.push(Step::Modality(brindle_ui::Modality::Keyboard)),
                _ => return Err("--modality needs pointer or keyboard".into()),
            },
            "--step-interval" => match it.next().and_then(|s| s.parse::<f64>().ok()) {
                Some(secs) if secs > 0.0 && secs.is_finite() => args.step_interval = Some(secs),
                _ => return Err("--step-interval needs a positive number of seconds".into()),
            },
            "--dump-screen-after" => match it.next().and_then(|s| s.parse::<f64>().ok()) {
                Some(secs) if secs >= 0.0 && secs.is_finite() => args.dump_after = Some(secs),
                _ => return Err("--dump-screen-after needs a number of seconds".into()),
            },
            "--latency-log" => args.latency_log = Some(config::expand_tilde(&value_of(it, &arg, "a file")?, home)),
            "--latency-bench" => match it.next().and_then(|n| n.parse().ok()) {
                Some(n) if n > 0 => args.latency_bench = Some(n),
                _ => return Err("--latency-bench needs a positive number of keystrokes".into()),
            },
            "--install-desktop" => install_desktop = true,
            "--uninstall-desktop" => uninstall_desktop = true,
            "--prefix" => prefix = Some(config::expand_tilde(&value_of(it, &arg, "a directory")?, home)),
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    args.desktop = desktop::request(install_desktop, uninstall_desktop, prefix).map_err(str::to_string)?;
    Ok(args)
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

/// Opens Brindle's window on what `launch` asks for. `origin` is this
/// Brindle's id in its tabs' layouts ([`layout::origin_id`]).
pub fn open_window(launch: Launch, origin: gpui::SharedString, cx: &mut App) -> Option<gpui::WindowHandle<Workspace>> {
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
    match cx.open_window(options, |window, cx| cx.new(|cx| Workspace::new(launch, origin, window, cx))) {
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

/// The queue of config reloads ([`request_config_reload`]), which one
/// worker task takes in request order ([`start_config_reloads`],
/// design.md decision 4).
struct ConfigReloads(futures::channel::mpsc::UnboundedSender<Vec<(String, String)>>);

impl Global for ConfigReloads {}

/// Asks for the config file to be read again and applied to every window,
/// after the reloads asked for before. `renames` are profiles renamed in
/// the settings dialog, whose open tabs follow the new name. It returns at
/// once: the worker reads the file off the UI thread, and applies it
/// outside any window's update, so a window's own handler can ask.
pub(crate) fn request_config_reload(renames: Vec<(String, String)>, cx: &mut App) {
    if let Some(reloads) = cx.try_global::<ConfigReloads>() {
        reloads.0.unbounded_send(renames).ok();
    }
}

/// Starts the config reload worker: one foreground task that takes a
/// request with whatever else is queued by then, reads the file once on the
/// background executor, and applies it. One worker keeps reloads in order,
/// so a slow read never lands after a newer one, and the queue holds at
/// most what came during one read (`process R9`).
fn start_config_reloads(cx: &mut App) {
    use futures::StreamExt as _;

    let (requests, mut queued) = futures::channel::mpsc::unbounded::<Vec<(String, String)>>();
    cx.set_global(ConfigReloads(requests));
    cx.spawn(async move |cx| {
        while let Some(first) = queued.next().await {
            let mut batch = vec![first];
            while let Ok(Some(more)) = queued.try_next() {
                batch.push(more);
            }
            let renames = merge_renames(batch);
            let (config, report) = cx.background_executor().spawn(async { Config::load_or_default() }).await;
            if cx.update(|cx| apply_config(config, report, &renames, cx)).is_err() {
                break;
            }
        }
    })
    .detach();
}

/// Binds the keys, and adds the config's keybindings that couldn't be bound
/// to the window's config line.
fn bind_keys(cx: &mut App) {
    let findings = actions::bind_keys(cx);
    cx.update_global(|settings: &mut Settings, _| settings.add_binding_findings(findings));
}

/// Applies a config read from the file to every window. `renames` are
/// profiles renamed in the settings dialog, whose open tabs follow the new
/// name.
fn apply_config(config: Config, report: ConfigReport, renames: &[(String, String)], cx: &mut App) {
    let (zoom, notices) = (Settings::get(cx).zoom, Settings::get(cx).notices().to_vec());
    let mut settings = Settings::from_platform(config, report, cx).with_notices(notices);
    settings.zoom = zoom;
    cx.set_global(settings);
    Settings::apply_ui_theme(cx);
    bind_keys(cx);
    for window in cx.windows() {
        if let Some(window) = window.downcast::<Workspace>() {
            window.update(cx, |workspace, _, cx| workspace.apply_config(renames, cx)).ok();
        }
    }
    keep_hosts_connected(cx);
    cx.refresh_windows();
}

/// Keeps the remote endpoints' states in the [`Hosts`] global, for the
/// sessions palette (design.md decision 11): every remote connection reports
/// its changes on a channel this drains on the UI thread, so nothing polls
/// them. Then connects to the endpoints of the profiles that connect at
/// launch, on every launch (`--no-restore` included). Before any window
/// opens, so the restored tabs' connections report too.
fn watch_hosts(cx: &mut App) {
    cx.set_global(Hosts::default());
    #[cfg(any(target_os = "linux", windows))]
    {
        use futures::StreamExt as _;

        let (reported, mut reports) = futures::channel::mpsc::unbounded();
        remote::local::watch_hosts(move |host, conn, state| {
            reported.unbounded_send((host.clone(), conn, state)).ok();
        });
        cx.spawn(async move |cx| {
            while let Some(report) = reports.next().await {
                // What has come meanwhile, applied in one update.
                let mut batch = vec![report];
                while let Ok(Some(report)) = reports.try_next() {
                    batch.push(report);
                }
                let applied = cx.update(|cx| {
                    let changed = cx.update_global(|hosts: &mut Hosts, _| hosts.apply_all(batch));
                    if changed {
                        hosts_changed(cx);
                    }
                });
                if applied.is_err() {
                    break;
                }
            }
        })
        .detach();
    }
    keep_hosts_connected(cx);
}

/// Keeps the endpoints of the config's profiles that connect at launch
/// connected, and lets go of those it no longer names (a reload).
fn keep_hosts_connected(cx: &mut App) {
    let eager = reattach::eager_hosts(&Settings::get(cx).config);
    let changed = cx.update_global(|hosts: &mut Hosts, _| hosts.want(&eager));
    #[cfg(any(target_os = "linux", windows))]
    remote::local::keep_connected(eager);
    if changed {
        hosts_changed(cx);
    }
}

/// Tells every window the endpoints' states changed.
fn hosts_changed(cx: &mut App) {
    for window in cx.windows() {
        if let Some(window) = window.downcast::<Workspace>() {
            window.update(cx, |workspace, window, cx| workspace.hosts_changed(window, cx)).ok();
        }
    }
}

fn config_mtime() -> Option<SystemTime> {
    std::fs::metadata(Config::path()).and_then(|m| m.modified()).ok()
}

/// Reloads the config whenever the file changes on disk: stats it once a
/// second on the background executor.
fn watch_config(cx: &mut App) {
    cx.spawn(async move |cx| {
        let mut last = cx.background_executor().spawn(async { config_mtime() }).await;
        loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            let now = cx.background_executor().spawn(async { config_mtime() }).await;
            if now != last {
                last = now;
                log::info!("config changed, reloading");
                if cx.update(|cx| request_config_reload(Vec::new(), cx)).is_err() {
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
    /// For `--attach`: the server's sessions, listed on the socket's thread;
    /// [`Listing::Skipped`] otherwise.
    listing: Listing,
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
                    if let Some(launch) = forwarded_launch(&request, &listing, &Settings::get(cx).config) {
                        ws.open(launch, window, cx);
                    }
                    notify::activation::raise(window, request.activation_token.as_deref(), cx);
                })
                .map_err(|_| "Brindle's window is gone".to_string());
            reply.send(result).ok();
        }
    })
    .detach();
}

/// What a later launch's request opens: nothing for a plain launch, which
/// only raises the window.
fn forwarded_launch(request: &instance::Request, listing: &Listing, config: &Config) -> Option<Launch> {
    if let Some(session) = request.attach {
        return Some(Launch::Attach { session, target: listing.attach_target(session) });
    }
    if request.profile.is_none() && request.cwd.is_none() && request.command.is_none() {
        return None;
    }
    let chosen = workspace::profile_for(config, request.profile.as_deref());
    if let Some(note) = chosen.note(config) {
        log::warn!("{note}");
    }
    let command = request.command.as_ref().and_then(|c| c.split_first()).map(|(p, rest)| (p.clone(), rest.to_vec()));
    // A command starts where it was typed, unless `-d` says otherwise.
    let cwd = request.cwd.clone().or_else(|| command.as_ref().and(request.launch_dir.clone()));
    Some(Launch::Request(LaunchRequest { profile: chosen.index, command, cwd, note: chosen.note(config) }))
}

/// What a launch opens ([`startup_launch`]).
struct Startup {
    /// What the [`reattach::Detached`] global starts with: the restored
    /// zones take theirs from it, and the rest open as tabs.
    detached: Vec<SessionInfo>,
    /// The window's first content.
    first: Launch,
    /// The tab opened after it, active: the asked tab, behind a restore.
    then: Option<Launch>,
}

/// What a launch opens, given this machine's `listing`, the `recorded`
/// window (the state file's windows merged), whether it `restoring`, its
/// `--attach` session, the tab it `asked` for (`-p`/`-d`/`-e`) and the
/// `profile` a default tab opens with (design.md decision 3).
///
/// When restoring, `detached` is the listed sessions that are detached and
/// aren't `--attach`'s. The restore is `recorded` without the zones whose
/// session isn't among them; when the listing failed (or was skipped)
/// nothing tells a lost zone from a live one, so every zone is kept and
/// nothing is detached. The asked tab is `--attach`'s, else the request.
/// It comes after a restore, and without one it is the window's first
/// content, else a default-profile tab is.
fn startup_launch(
    listing: &Listing,
    recorded: Option<WindowState>,
    restoring: bool,
    attach: Option<u32>,
    asked: Option<LaunchRequest>,
    profile: usize,
) -> Startup {
    let detached: Vec<SessionInfo> = match listing {
        Listing::Listed(sessions) if restoring => {
            sessions.iter().filter(|s| !s.attached && Some(s.session) != attach).cloned().collect()
        }
        Listing::Listed(_) | Listing::Failed(_) | Listing::Skipped => Vec::new(),
    };
    let restored = if restoring {
        match listing {
            Listing::Listed(_) => {
                // A local copy: the global is the shell's to set.
                let alive = reattach::Detached::new(detached.clone());
                recorded.and_then(|window| session::drop_lost_zones(window, |recorded| alive.has(recorded)))
            }
            Listing::Failed(_) | Listing::Skipped => recorded,
        }
    } else {
        None
    };
    let asked = match attach {
        Some(session) => Some(Launch::Attach { session, target: listing.attach_target(session) }),
        None => asked.map(Launch::Request),
    };
    match restored {
        Some(window) => Startup { detached, first: Launch::Restore(window), then: asked },
        None => Startup {
            detached,
            first: asked.unwrap_or_else(|| Launch::Request(LaunchRequest::profile(profile))),
            then: None,
        },
    }
}

/// `BRINDLE_TEST_LAUNCH_LISTING` (debug builds only, `process R20`): what
/// the e2e cases make the launch's listing do.
#[cfg(debug_assertions)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TestListing {
    /// Wait this long, then list.
    Delay(Duration),
    /// Fail without listing.
    Fail,
}

/// The test switch's value: `fail`, or a whole number of milliseconds to
/// wait; anything else, or nothing, leaves the listing alone.
#[cfg(debug_assertions)]
fn test_listing(value: Option<&str>) -> Option<TestListing> {
    match value? {
        "fail" => Some(TestListing::Fail),
        millis if !millis.is_empty() && millis.bytes().all(|b| b.is_ascii_digit()) => {
            millis.parse().ok().map(|millis| TestListing::Delay(Duration::from_millis(millis)))
        }
        _ => None,
    }
}

/// The rename lists of the config reloads taken in one turn, as one list
/// in request order: a profile renamed `a`→`b`, then `b`→`c`, stays two
/// steps, applied in turn.
fn merge_renames(batch: Vec<Vec<(String, String)>>) -> Vec<(String, String)> {
    batch.into_iter().flatten().collect()
}

/// The running terminal server's sessions, attached or not: none when no
/// server runs (none is started), [`Listing::Failed`] when listing fails
/// (logged). It blocks (see `remote::local::running_sessions`), so it runs
/// on the `launch-list` thread and the instance listener's, never the UI
/// thread.
fn running_sessions() -> Listing {
    #[cfg(any(target_os = "linux", windows))]
    match remote::local::running_sessions(Duration::from_secs(2)) {
        Ok(sessions) => Listing::Listed(sessions),
        Err(why) => {
            log::warn!("not attaching detached sessions: {why:#}");
            Listing::Failed(format!("{why:#}"))
        }
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    Listing::Listed(Vec::new())
}

/// What a launch reads before its window opens ([`read_launch`]).
struct LaunchReads {
    listing: Listing,
    /// The state file's windows merged into one, when restoring; `None`
    /// when it is missing, unusable (logged) or has no tabs.
    recorded: Option<WindowState>,
}

/// The launch's blocking reads, on the `launch-list` thread: this machine's
/// sessions when the launch restores or attaches, and the recorded window
/// when it restores (from the state directory `restore_from` names).
fn read_launch(restore_from: Option<&Path>, attach: bool) -> LaunchReads {
    let listing = if restore_from.is_some() || attach { launch_listing() } else { Listing::Skipped };
    let recorded = restore_from.and_then(recorded_window);
    LaunchReads { listing, recorded }
}

/// [`running_sessions`], slowed or failed by `BRINDLE_TEST_LAUNCH_LISTING`
/// in debug builds.
fn launch_listing() -> Listing {
    #[cfg(debug_assertions)]
    match test_listing(std::env::var("BRINDLE_TEST_LAUNCH_LISTING").ok().as_deref()) {
        Some(TestListing::Fail) => {
            let why = "BRINDLE_TEST_LAUNCH_LISTING=fail";
            log::warn!("not attaching detached sessions: {why}");
            return Listing::Failed(why.into());
        }
        // The fault injector's delay, not a wait for a condition.
        Some(TestListing::Delay(delay)) => std::thread::sleep(delay),
        None => {}
    }
    running_sessions()
}

/// The windows recorded in `state_dir`, merged into the one window Brindle
/// has; `None` when the state file is missing, unusable (logged) or has no
/// tabs.
fn recorded_window(state_dir: &Path) -> Option<WindowState> {
    match session::load(state_dir) {
        Ok(state) => session::merge_windows(state?.windows),
        Err(why) => {
            log::warn!("{why}");
            None
        }
    }
}

/// Runs [`read_launch`] on a thread named `launch-list` and answers on the
/// returned channel. When no thread can be started, the reads run here, on
/// the main thread before GPUI starts, where nothing waits on them yet
/// (design.md decision 1).
fn start_launch_reads(restore_from: Option<&Path>, attach: bool) -> futures::channel::oneshot::Receiver<LaunchReads> {
    let (answer, answered) = futures::channel::oneshot::channel();
    let dir = restore_from.map(Path::to_path_buf);
    let spawned = std::thread::Builder::new().name("launch-list".into()).spawn(move || {
        answer.send(read_launch(dir.as_deref(), attach)).ok();
    });
    match spawned {
        Ok(_) => answered,
        Err(error) => {
            log::warn!("no launch-list thread ({error}): listing sessions before the window can start");
            let (answer, answered) = futures::channel::oneshot::channel();
            answer.send(read_launch(restore_from, attach)).ok();
            answered
        }
    }
}

/// For a restoring launch: connects, on background threads, to the remote
/// endpoints of the restored zones and of the profiles that connect at
/// launch (which [`watch_hosts`] keeps connected whether or not the launch
/// restores); on each one's first listing, attaches its detached sessions
/// that nothing restored as tabs at the end ([`Workspace::attach_swept`],
/// design.md decision 10).
fn sweep_launch_hosts(window: gpui::WindowHandle<Workspace>, cx: &mut App) {
    #[cfg(any(target_os = "linux", windows))]
    {
        use futures::StreamExt as _;

        let restored = window.update(cx, |ws, _, _| ws.restored_hosts().to_vec()).unwrap_or_default();
        let hosts = reattach::launch_hosts(&restored, &Settings::get(cx).config);
        if hosts.is_empty() {
            return;
        }
        let (listed, mut listings) = futures::channel::mpsc::unbounded();
        for host in hosts {
            let listed = listed.clone();
            let spawned = std::thread::Builder::new().name("launch-connect".into()).spawn(move || {
                let named = host.clone();
                let hook: remote::local::ListingHook = Box::new(move |sessions| {
                    listed.unbounded_send((named, sessions)).ok();
                });
                remote::local::when_listed(&host, hook);
            });
            if let Err(error) = spawned {
                log::warn!("not connecting at launch: {error}");
            }
        }
        // Ends once every hook has run or was dropped with its connection.
        drop(listed);
        cx.spawn(async move |cx| {
            while let Some((host, sessions)) = listings.next().await {
                window.update(cx, |ws, window, cx| ws.attach_swept(host, sessions, window, cx)).ok();
            }
        })
        .detach();
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    let _ = (window, cx);
}

/// Writes the session state when it changes (off the UI thread), and sends
/// changed tab layouts to the terminal server, both checked once a second;
/// and writes the state once
/// more, synchronously, when Brindle quits, after each window still open
/// has recorded itself and left its terminals' sessions (`Workspace::quit`).
fn record_session(state_dir: Option<PathBuf>, cx: &mut App) {
    fn pending(file: &session::StateFile, cx: &mut App) -> Option<String> {
        let outcome = cx.update_global(|store: &mut SessionStore, _| {
            if store.is_owner() { store.pending_write() } else { PendingWrite::Nothing }
        });
        match outcome {
            PendingWrite::Write(text) => Some(text),
            PendingWrite::Unserializable(err) => {
                log::warn!("not writing session state to {}: it can't be serialized: {err}", file.path().display());
                None
            }
            PendingWrite::Nothing | PendingWrite::StillUnserializable => None,
        }
    }
    fn write(file: &session::StateFile, text: &str, ticket: u64) {
        if let Err(err) = file.write(text, ticket) {
            log::warn!("could not write session state to {}: {err}", file.path().display());
        }
    }
    // One ordering for the tick's background writes and the last one. No
    // state directory, no file: the layouts still go to the server.
    let file = state_dir.map(|dir| Arc::new(session::StateFile::new(dir)));
    let ticking = file.clone();
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            // Each window's tabs keep their sessions' layouts current on
            // the same tick: a throttle, changes only (`send_layouts`).
            let sent = cx.update(|cx| {
                for window in cx.windows() {
                    if let Some(window) = window.downcast::<Workspace>() {
                        window.read_with(cx, |ws, cx| ws.send_layouts(cx)).ok();
                    }
                }
            });
            if sent.is_err() {
                break;
            }
            let Some(ticking) = &ticking else { continue };
            let Ok(text) = cx.update(|cx| pending(ticking, cx)) else { break };
            if let Some(text) = text {
                let (file, ticket) = (ticking.clone(), ticking.ticket());
                cx.background_executor().spawn(async move { write(&file, &text, ticket) }).detach();
            }
        }
    })
    .detach();
    cx.on_app_quit(move |cx| {
        // The windows still open now are the ones to keep. Each detaches
        // its tabs' sessions (`-e` ones close) itself, every debug run's
        // included, rather than leaving it to the terminals' drops, which
        // on Windows don't run before the exit. A window that closed did
        // so as it closed.
        for window in cx.windows() {
            if let Some(window) = window.downcast::<Workspace>() {
                window.update(cx, |ws, window, cx| ws.quit(window, cx)).ok();
            }
        }
        if let Some(file) = &file
            && let Some(text) = pending(file, cx)
        {
            write(file, &text, file.ticket());
        }
        async {}
    })
    .detach();
}

/// Takes `state_dir`'s session lock, making this process the instance, or
/// hands a launch that isn't a debug run to the running instance and exits.
/// `None` when another Brindle holds the lock and this one runs beside it.
fn session_lock(state_dir: &Path, args: &Args) -> Option<session::Lock> {
    match session::Lock::acquire(state_dir) {
        Ok(lock) => Some(lock),
        Err(err) if err.kind() == std::io::ErrorKind::WouldBlock && !args.is_debug_run() => {
            match instance::forward(state_dir, &args.instance_request()) {
                Ok(()) => std::process::exit(0),
                // The running Brindle may have quit meanwhile.
                Err(why) => match session::Lock::acquire(state_dir) {
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
    }
}

fn main() {
    let started = Instant::now();
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
    // exits. A debug run beside it runs on its own instead, and so does a
    // Brindle with no state directory, whose window says so.
    let state_dir = session::state_dir(&session::StateEnv::read());
    let lock = match &state_dir {
        Some(state_dir) => session_lock(state_dir, &args),
        None => {
            log::warn!("no state directory: running on its own, without restoring or recording");
            None
        }
    };
    let notices: Vec<String> = if state_dir.is_none() { vec![session::NO_STATE_DIR.to_string()] } else { Vec::new() };
    let (forwarded, requests) = futures::channel::mpsc::unbounded::<Forwarded>();
    let mut socket = None;
    if lock.is_some()
        && let Some(state_dir) = &state_dir
    {
        let handler = move |request: instance::Request| {
            // Listed here, off the UI thread.
            let listing = request.attach.map_or(Listing::Skipped, |_| running_sessions());
            let (reply, answer) = std::sync::mpsc::channel();
            forwarded
                .unbounded_send(Forwarded { request, listing, reply })
                .map_err(|_| "Brindle is quitting".to_string())?;
            answer
                .recv_timeout(instance::FORWARD_LIMIT)
                .map_err(|_| "Brindle didn't take the request in time".to_string())?
        };
        match instance::listen(state_dir, Box::new(handler)) {
            Ok(path) => {
                log::debug!("taking launches at {}", path.display());
                socket = Some(path);
            }
            Err(err) => log::warn!("later launches can't reach this Brindle: {err}"),
        }
    }
    // The Brindle that owns the session state restores it and attaches the
    // server's detached sessions (restored zones take theirs, the rest
    // become tabs), unless `--no-restore`. What that needs is read on a
    // thread of its own while GPUI starts, so the UI thread never waits on
    // the terminal server or the state file (`process R13`).
    let restoring = !args.no_restore && lock.is_some();
    let restore_from = state_dir.clone().filter(|_| restoring);
    let reads = start_launch_reads(restore_from.as_deref(), args.attach.is_some());
    // A plain value, read before any event loop exists.
    let (config, report) = Config::load_or_default();

    // Before any window: windows, shortcuts and toasts share one identity.
    #[cfg(windows)]
    win::set_app_id();

    Application::new().run(move |cx: &mut App| {
        // This Brindle's id in its tabs' layouts, made once: its pid and
        // when it started.
        let since_epoch = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
        let origin: gpui::SharedString = layout::origin_id(std::process::id(), since_epoch).into();
        let settings = Settings::from_platform(config, report, cx).with_notices(notices);
        let chosen = workspace::profile_for(&settings.config, args.profile.as_deref());
        if let Some(note) = chosen.note(&settings.config) {
            log::warn!("{note}");
        }
        let note = chosen.note(&settings.config);
        let profile = chosen.index;
        cx.set_global(settings);
        Settings::apply_ui_theme(cx);
        notify::Notifier::init(cx);
        bind_keys(cx);
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

        cx.on_action(|_: &Quit, cx| cx.quit());
        // Global listeners run inside the dispatching window's update, which
        // a reload couldn't update; the worker applies it after that.
        start_config_reloads(cx);
        cx.on_action(|_: &ReloadConfig, cx| request_config_reload(Vec::new(), cx));
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
        // What the launch asked for opens as the active tab.
        let asked = args
            .asks_for_a_tab()
            .then(|| LaunchRequest { profile, command: args.command.clone(), cwd: args.cwd.clone(), note });
        // Before the wait: the endpoints of profiles that connect at launch
        // start connecting while this machine's listing runs.
        watch_hosts(cx);
        // The window opens once the launch's reads have answered or failed,
        // with what the launch opens for that answer (design.md decision 2).
        cx.spawn(async move |cx| {
            let reads = match reads.await {
                Ok(reads) => reads,
                Err(futures::channel::oneshot::Canceled) => {
                    let why = "the launch's listing thread ended without an answer";
                    log::error!("{why}; not attaching detached sessions");
                    // Read again, so a bug in the listing can't cost the
                    // saved tabs, which the next record would overwrite.
                    let recorded = match restore_from.clone() {
                        Some(dir) => cx.background_executor().spawn(async move { recorded_window(&dir) }).await,
                        None => None,
                    };
                    LaunchReads { listing: Listing::Failed(why.into()), recorded }
                }
            };
            let answered = started.elapsed();
            cx.update(|cx| {
                let startup = startup_launch(&reads.listing, reads.recorded, restoring, args.attach, asked, profile);
                cx.set_global(reattach::Detached::new(startup.detached));
                if let (Launch::Restore(state), Some(dir)) = (&startup.first, &restore_from) {
                    log::info!("restoring {} tab(s) from {}", state.tabs.len(), session::state_path(dir).display());
                }
                let Some(window) = open_window(startup.first, origin, cx) else {
                    cx.quit();
                    return;
                };
                // Raises with an activation token need an attachment to the
                // display, made off the UI thread from now on.
                window.update(cx, |_, window, cx| notify::activation::prepare(window, cx)).ok();
                log::debug!(
                    "launch: the listing answered {answered:?} and the window opened {:?} after the start",
                    started.elapsed()
                );
                let rest = cx.update_global(|detached: &mut reattach::Detached, _| detached.take_rest());
                if !rest.is_empty() {
                    log::info!("attaching {} detached session(s)", rest.len());
                    window.update(cx, |ws, window, cx| ws.attach_detached(rest, window, cx)).ok();
                }
                if let Some(request) = startup.then {
                    window.update(cx, |ws, window, cx| ws.open(request, window, cx)).ok();
                }
                if restoring {
                    sweep_launch_hosts(window, cx);
                }
                watch_config(cx);
                record_session(state_dir.clone(), cx);
                take_launches(window, requests, cx);
                run_debug_steps(window, &args, cx);
            })
            .ok();
        })
        .detach();
    });
    // Every way out of `run` has detached (or closed) the tabs' sessions:
    // the quit (`record_session`'s quit handler) or the window's close
    // (`Workspace::quit`). Those messages are only queued for the
    // connections' writer threads, which the exit would end (Windows ends
    // them at once). A lost `DETACH` leaves a session its owner, so the
    // next launch wouldn't open it as a tab.
    #[cfg(any(target_os = "linux", windows))]
    if !remote::local::flush(remote::local::QUIT_FLUSH) {
        log::info!("quitting before every terminal server connection had written what its tabs sent");
    }
}

/// The debug steps (`--send`, `--action`, `--key`, …, `--latency-bench`)
/// and the `--dump-screen-after` timer, counted from the window's opening.
fn run_debug_steps(window: gpui::WindowHandle<Workspace>, args: &Args, cx: &mut App) {
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
            if let Step::Key(keystroke) = &step {
                // Not inside the workspace's update: the key's actions
                // update it themselves.
                let keystroke = keystroke.clone();
                cx.update_window(window.into(), move |_, window, cx| {
                    window.dispatch_keystroke(keystroke, cx);
                })
                .ok();
                continue;
            }
            window
                .update(cx, |ws, window, cx| match &step {
                    Step::Send(text) => ws.send_to_active(text.as_bytes(), window, cx),
                    Step::Find(text) => ws.find_in_active(text, window, cx),
                    Step::Action(name) => {
                        let action = actions::action_by_name(name).expect("checked when the arguments were parsed");
                        window.dispatch_action(action, cx);
                    }
                    Step::Pointer(step) => ws.debug_pointer(step, window, cx),
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
        parse_args_from(args.iter().map(|s| s.to_string()), Some(Path::new("/home/u")))
            .unwrap_or_else(|why| panic!("{args:?}: {why}"))
    }

    /// The usage error `args` give.
    fn refused(args: &[&str]) -> String {
        match parse_args_from(args.iter().map(|s| s.to_string()), Some(Path::new("/home/u"))) {
            Ok(parsed) => panic!("{args:?} parsed: {parsed:?}"),
            Err(why) => why,
        }
    }

    #[test]
    fn an_option_without_its_value_is_a_usage_error() {
        for option in [
            "-p",
            "--profile",
            "-d",
            "--working-directory",
            "--config",
            "--send",
            "--action",
            "--find",
            "--wait-for",
            "--key",
            "--hover",
            "--click",
            "--dump-screen-after",
            "--latency-log",
            "--prefix",
        ] {
            let why = refused(&[option]);
            assert!(why.starts_with(&format!("{option} needs ")), "{option}: {why}");
        }
        assert_eq!(refused(&["--dump-screen-after", "soon"]), "--dump-screen-after needs a number of seconds");
        assert_eq!(refused(&["--dump-screen-after", "-1"]), "--dump-screen-after needs a number of seconds");
        assert_eq!(parse(&["--dump-screen-after", "2.5"]).dump_after, Some(2.5));
    }

    #[test]
    fn dash_e_without_a_command_is_a_usage_error() {
        assert_eq!(refused(&["-e"]), "-e needs a command");
        assert_eq!(refused(&["-p", "work", "--command"]), "--command needs a command");
    }

    #[test]
    fn an_unknown_action_is_a_usage_error() {
        let why = refused(&["--action", "split_zone_rihgt", "--dump-screen-after", "3"]);
        assert!(why.contains("--action") && why.contains("\"split_zone_rihgt\""), "{why}");
        assert!(matches!(&parse(&["--action", "split_zone_right"]).steps[..], [Step::Action(name)] if name == "split_zone_right"));
    }

    #[test]
    fn a_bad_key_is_a_usage_error() {
        let why = refused(&["--key", "ctrl-a-b"]);
        assert!(why.starts_with("--key: ") && why.contains("ctrl-a-b"), "{why}");
    }

    #[test]
    fn a_bad_pointer_position_is_a_usage_error() {
        for (option, position) in [("--hover", "3"), ("--click", "hyper-3,0"), ("--click", "a,b")] {
            let why = refused(&[option, position]);
            assert!(why.starts_with(&format!("{option}: ")), "{option} {position}: {why}");
        }
        let args = parse(&["--click", "ctrl-3,1"]);
        let [Step::Pointer(step)] = &args.steps[..] else { panic!("{:?}", args.steps) };
        assert_eq!((step.col, step.row, step.click, step.modifiers.control), (3, 1, true, true));
    }

    #[test]
    fn a_tilde_in_a_path_argument_is_the_home_directory() {
        assert_eq!(parse(&["-d", "~/src"]).cwd, Some(PathBuf::from("/home/u/src")));
        assert_eq!(parse(&["-d", "~"]).cwd, Some(PathBuf::from("/home/u")));
        assert_eq!(parse(&["--latency-log", "~/l.tsv"]).latency_log, Some(PathBuf::from("/home/u/l.tsv")));
        assert_eq!(parse(&["-d", "/srv"]).cwd, Some(PathBuf::from("/srv")));
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

    /// A handle is given out once: a wait that ended, its entry cleared,
    /// can't mark or clear a later wait, even the next one begun.
    #[test]
    fn the_wait_log_never_reuses_a_handle() {
        let waits = WaitLog::default();
        let first = waits.begin("one");
        waits.clear(first);
        let second = waits.begin("two");
        assert_ne!(first, second);
        waits.set(first, WaitState::TimedOut);
        waits.clear(first);
        assert_eq!(waits.describe(), "--- wait-for pending \"two\"\n");
        waits.clear(second);
        let third = waits.begin("three");
        assert!(third != first && third != second);
        waits.set(third, WaitState::TimedOut);
        assert_eq!(waits.describe(), "--- wait-for timeout \"three\"\n");
    }

    /// `--send` and `--wait-for` text: `\r`, `\n`, `\e` and `\t` become
    /// the controls; a lone backslash and any other text stay as they are.
    #[test]
    fn step_text_turns_four_escapes_into_controls() {
        assert_eq!(unescape(r"echo hi\r"), "echo hi\r");
        assert_eq!(unescape(r"one\ntwo"), "one\ntwo");
        assert_eq!(unescape(r"\e[A"), "\x1b[A");
        assert_eq!(unescape(r"a\tb"), "a\tb");
        assert_eq!(unescape(r"\r\n\e\t"), "\r\n\x1b\t");
        assert_eq!(unescape("\\"), "\\");
        assert_eq!(unescape(r"C:\Users"), r"C:\Users");
        assert_eq!(unescape(r"\a \x41 \0"), r"\a \x41 \0");
        assert_eq!(unescape("plain text, ünïcode"), "plain text, ünïcode");
        assert_eq!(unescape(""), "");
    }

    #[test]
    fn key_and_modality_steps_keep_their_order() {
        let args =
            parse(&["--key", "shift-tab", "--modality", "pointer", "--action", "new_tab", "--modality", "keyboard"]);
        assert!(args.is_debug_run());
        let steps: Vec<String> = args
            .steps
            .iter()
            .map(|s| match s {
                Step::Key(keystroke) => format!("Key({})", keystroke.unparse()),
                other => format!("{other:?}"),
            })
            .collect();
        assert_eq!(steps, ["Key(shift-tab)", "Modality(Pointer)", r#"Action("new_tab")"#, "Modality(Keyboard)"]);
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
        let config = Config::parse("default_profile = \"a\"\n[[profiles]]\nname = \"a\"\n[[profiles]]\nname = \"b\"\n").unwrap().config;
        let request = |r: instance::Request| forwarded_launch(&r, &Listing::Skipped, &config);
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

    #[test]
    fn forwarded_attach_says_why_listing_failed() {
        let config = Config::parse("[[profiles]]\nname = \"a\"\n").unwrap().config;
        let attach = instance::Request { attach: Some(7), ..Default::default() };
        let opened = |listing: &Listing| forwarded_launch(&attach, listing, &config).map(opens);
        assert_eq!(
            opened(&Listing::Failed("timed out".into())),
            Some(Opens::Attach(7, Err("Brindle could not list the terminal server's sessions: timed out".into())))
        );
        assert_eq!(
            opened(&Listing::Listed(vec![listed(7, "g", false)])),
            Some(Opens::Attach(7, Ok(listed(7, "g", false))))
        );
        assert_eq!(
            opened(&Listing::Listed(Vec::new())),
            Some(Opens::Attach(7, Err("the terminal server has no session 7".into()))),
            "only a listing that answered says the session doesn't exist"
        );
    }

    /// A [`Launch`] as a value to compare.
    #[derive(Debug, PartialEq)]
    enum Opens {
        Tab(usize, Option<(String, Vec<String>)>, Option<PathBuf>),
        Restore(WindowState),
        Attach(u32, Result<SessionInfo, String>),
    }

    fn opens(launch: Launch) -> Opens {
        match launch {
            Launch::Request(LaunchRequest { profile, command, cwd, .. }) => Opens::Tab(profile, command, cwd),
            Launch::Restore(window) => Opens::Restore(window),
            Launch::Attach { session, target } => Opens::Attach(session, target),
        }
    }

    /// What [`startup_launch`] gives, as values to compare.
    fn startup(
        listing: &Listing,
        recorded: Option<WindowState>,
        restoring: bool,
        attach: Option<u32>,
        asked: Option<LaunchRequest>,
    ) -> (Vec<u32>, Opens, Option<Opens>) {
        let startup = startup_launch(listing, recorded, restoring, attach, asked, 2);
        (startup.detached.iter().map(|s| s.session).collect(), opens(startup.first), startup.then.map(opens))
    }

    fn listed(session: u32, key: &str, attached: bool) -> SessionInfo {
        SessionInfo { session, key: key.into(), attached, ..Default::default() }
    }

    /// A local tab that recorded session `id` with `key`.
    fn tab(id: u32, key: &str) -> session::TabState {
        session::TabState::Local {
            profile: "Shell".into(),
            remote: None,
            cwd: Some(PathBuf::from(format!("/t{id}"))),
            title: None,
            zones: None,
            session: Some(session::ServerSession { id, key: key.into() }),
        }
    }

    fn window(tabs: Vec<session::TabState>) -> WindowState {
        WindowState { width: Some(800.0), height: Some(600.0), maximized: false, active: 0, tabs }
    }

    fn htop() -> LaunchRequest {
        LaunchRequest { profile: 1, command: Some(("htop".into(), vec![])), cwd: None, note: None }
    }

    #[test]
    fn a_listing_restores_live_zones_and_sweeps_the_rest() {
        let listing = Listing::Listed(vec![
            listed(1, "a", false),
            listed(2, "new", false),
            listed(3, "c", true),
            listed(4, "d", false),
        ]);
        // Zone 1's session is detached; zone 2's id was reused by another
        // session; zone 3's is shown by another Brindle.
        let recorded = window(vec![tab(1, "a"), tab(2, "old"), tab(3, "c")]);
        let (detached, first, then) = startup(&listing, Some(recorded), true, None, None);
        assert_eq!(first, Opens::Restore(window(vec![tab(1, "a")])), "only the live zone is restored");
        assert_eq!(detached, [1, 2, 4], "every detached session, the restored zone's included, and no attached one");
        assert_eq!(then, None);
    }

    #[test]
    fn a_failed_listing_keeps_every_zone_and_sweeps_nothing() {
        let recorded = window(vec![tab(1, "a"), tab(2, "b")]);
        for listing in [Listing::Failed("refused".into()), Listing::Skipped] {
            let (detached, first, then) = startup(&listing, Some(recorded.clone()), true, None, Some(htop()));
            assert_eq!(first, Opens::Restore(recorded.clone()), "{listing:?}: nothing tells a lost zone from a live one");
            assert!(detached.is_empty(), "{listing:?}");
            assert_eq!(then, Some(Opens::Tab(1, Some(("htop".into(), vec![])), None)), "{listing:?}");
        }
    }

    #[test]
    fn a_launch_that_restores_nothing_opens_the_asked_tab_or_a_default_one() {
        // `--no-restore`, or a Brindle that doesn't own the state.
        let (detached, first, then) = startup(&Listing::Skipped, None, false, None, Some(htop()));
        assert_eq!((detached.is_empty(), first, then), (true, Opens::Tab(1, Some(("htop".into(), vec![])), None), None));
        let (_, first, then) = startup(&Listing::Skipped, None, false, None, None);
        assert_eq!((first, then), (Opens::Tab(2, None, None), None), "the default profile's tab");
        // Not restoring: a recorded window is never opened.
        let (_, first, _) = startup(&Listing::Skipped, Some(window(vec![tab(1, "a")])), false, None, None);
        assert_eq!(first, Opens::Tab(2, None, None));
        // No state file: the asked tab, and the detached sessions still
        // open after it.
        let listing = Listing::Listed(vec![listed(5, "e", false)]);
        let (detached, first, then) = startup(&listing, None, true, None, Some(htop()));
        assert_eq!((detached, first, then), (vec![5], Opens::Tab(1, Some(("htop".into(), vec![])), None), None));
        // Every recorded zone lost.
        let (_, first, then) = startup(&Listing::Listed(Vec::new()), Some(window(vec![tab(1, "a")])), true, None, None);
        assert_eq!((first, then), (Opens::Tab(2, None, None), None));
    }

    #[test]
    fn attach_opens_after_the_restored_tabs_and_is_not_swept() {
        let listing = Listing::Listed(vec![listed(1, "a", false), listed(7, "g", false), listed(8, "h", false)]);
        let (detached, first, then) = startup(&listing, Some(window(vec![tab(1, "a")])), true, Some(7), Some(htop()));
        assert_eq!(first, Opens::Restore(window(vec![tab(1, "a")])));
        assert_eq!(then, Some(Opens::Attach(7, Ok(listed(7, "g", false)))), "--attach wins over -p/-d/-e");
        assert_eq!(detached, [1, 8], "--attach's session isn't swept into a tab of its own");
        // `--no-restore --attach 7`: the attached session is the window's.
        let (detached, first, then) = startup(&listing, None, false, Some(7), None);
        assert_eq!((detached.is_empty(), first, then), (true, Opens::Attach(7, Ok(listed(7, "g", false))), None));
    }

    #[test]
    fn attach_without_a_listing_says_why() {
        let failed = Listing::Failed("timed out".into());
        let why = "Brindle could not list the terminal server's sessions: timed out";
        let (_, first, then) = startup(&failed, None, false, Some(7), None);
        assert_eq!((first, then), (Opens::Attach(7, Err(why.into())), None));
        let recorded = window(vec![tab(1, "a")]);
        let (detached, first, then) = startup(&failed, Some(recorded.clone()), true, Some(7), None);
        assert_eq!((detached.is_empty(), first, then), (true, Opens::Restore(recorded), Some(Opens::Attach(7, Err(why.into())))));
    }

    #[cfg(debug_assertions)]
    #[test]
    fn the_launch_listing_switch_reads_fail_and_a_delay() {
        assert_eq!(test_listing(Some("fail")), Some(TestListing::Fail));
        assert_eq!(test_listing(Some("3000")), Some(TestListing::Delay(Duration::from_secs(3))));
        assert_eq!(test_listing(Some("0")), Some(TestListing::Delay(Duration::ZERO)));
        for other in ["", "FAIL", "1.5", "-1", "+5", " 300", "3s", "99999999999999999999999"] {
            assert_eq!(test_listing(Some(other)), None, "{other:?}");
        }
        assert_eq!(test_listing(None), None, "unset");
    }

    #[test]
    fn reload_renames_keep_their_order() {
        let pair = |from: &str, to: &str| (from.to_string(), to.to_string());
        let merged = merge_renames(vec![vec![pair("a", "b")], Vec::new(), vec![pair("b", "c"), pair("x", "y")]]);
        assert_eq!(merged, [pair("a", "b"), pair("b", "c"), pair("x", "y")], "two steps, in request order");
        assert!(merge_renames(Vec::new()).is_empty());
    }

    #[cfg(any(target_os = "linux", windows))]
    #[test]
    fn attach_takes_a_session_id() {
        let args = parse(&["--attach", "7", "--dump-screen-after", "2"]);
        assert_eq!(args.attach, Some(7));
        assert!(!args.asks_for_a_tab());
        assert_eq!(parse(&[]).attach, None);
    }
}
