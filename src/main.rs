mod actions;
mod config;
mod config_edit;
mod desktop;
mod picker;
mod session;
mod settings;
mod settings_dialog;
mod terminal;
mod terminal_element;
mod terminal_view;
mod theme;
mod tmux;
mod tmux_view;
mod workspace;

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use gpui::{
    App, AppContext as _, Application, BorrowAppContext as _, Bounds, WindowBackgroundAppearance, WindowBounds,
    WindowDecorations, WindowOptions, px, size,
};

use crate::actions::{NewWindow, Quit, ReloadConfig};
use crate::config::Config;
use crate::settings::Settings;
use crate::session::{SessionStore, WindowState};
use crate::workspace::{Launch, LaunchRequest, Workspace};

const HELP: &str = "\
Brindle — a GPU-accelerated terminal

USAGE:
    brindle [OPTIONS] [-e COMMAND [ARGS]...]

OPTIONS:
    -e, --command <CMD> [ARGS]   Run CMD instead of the profile's shell (must be last)
    -p, --profile <NAME>         Profile to open (default: config's default_profile)
    -d, --working-directory <DIR>
                                 Start in DIR
        --config <FILE>          Use FILE instead of ~/.config/brindle/config.toml
        --no-restore             Don't reopen the last session this time
                                 (with restore_session = true in the config)
        --list-actions           Print bindable action names and exit
        --list-themes            Print built-in theme names and exit
        --install-desktop        Add Brindle to the desktop's launcher, then exit
        --uninstall-desktop      Remove it from the launcher, then exit
        --prefix <DIR>           With the two above: use DIR/share instead of
                                 ~/.local/share (e.g. /usr/local)
    -h, --help                   Print help
    -V, --version                Print version

DEBUGGING:
        --send <TEXT>            Type TEXT into the active tab (\\r = Enter)
        --action <NAME>          Run an action (see --list-actions)
                                 --send/--action steps run in order, 1s apart
        --dump-screen-after <S>  Print the active tab's screen after S seconds and exit
";

#[derive(Debug, Clone)]
enum Step {
    Send(String),
    Action(String),
}

#[derive(Default, Debug)]
struct Args {
    command: Option<(String, Vec<String>)>,
    profile: Option<String>,
    cwd: Option<PathBuf>,
    /// `--send` / `--action` steps, run in order one second apart.
    steps: Vec<Step>,
    dump_after: Option<f64>,
    desktop: Option<desktop::Request>,
    no_restore: bool,
}

impl Args {
    /// A launch that may restore the session: no profile, directory or
    /// command was asked for. Debug steps don't count.
    fn is_plain(&self) -> bool {
        self.profile.is_none() && self.cwd.is_none() && self.command.is_none() && !self.no_restore
    }
}

fn parse_args() -> Args {
    let mut args = Args::default();
    let (mut install_desktop, mut uninstall_desktop, mut prefix) = (false, false, None);
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-h" | "--help" => {
                print!("{HELP}");
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("brindle {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--list-actions" => {
                for name in actions::action_names() {
                    println!("{name}");
                }
                std::process::exit(0);
            }
            "--list-themes" => {
                for name in theme::BUILTIN_THEMES {
                    println!("{name}");
                }
                std::process::exit(0);
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
            "--send" => args.steps.extend(it.next().map(|s| Step::Send(unescape(&s)))),
            "--action" => args.steps.extend(it.next().map(Step::Action)),
            "--dump-screen-after" => args.dump_after = it.next().and_then(|s| s.parse().ok()),
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
    eprintln!("brindle: {message}\n\n{HELP}");
    std::process::exit(2);
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
    let (config, error) = Config::load_or_default();
    let zoom = Settings::get(cx).zoom;
    let mut settings = Settings::new(config, error, cx);
    settings.zoom = zoom;
    cx.set_global(settings);
    actions::bind_keys(cx);
    for window in cx.windows() {
        if let Some(window) = window.downcast::<Workspace>() {
            window.update(cx, |workspace, _, cx| workspace.apply_config(cx)).ok();
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

/// The windows to restore, if session restore is on, this process owns the
/// state and the state file is usable. Problems are logged and ignored.
fn restorable_windows(cx: &mut App) -> Vec<WindowState> {
    let enabled = Settings::get(cx).config.restore_session;
    if !cx.update_global(|store: &mut SessionStore, _| store.is_owner(enabled)) {
        return Vec::new();
    }
    match session::load(&session::state_dir()) {
        Ok(Some(state)) => state.windows,
        Ok(None) => Vec::new(),
        Err(why) => {
            log::warn!("{why}");
            Vec::new()
        }
    }
}

/// Writes the session state when it changes (off the UI thread), and once
/// more, synchronously, when Brindle quits.
fn record_session(cx: &mut App) {
    fn pending(cx: &mut App) -> Option<String> {
        let enabled = Settings::get(cx).config.restore_session;
        cx.update_global(|store: &mut SessionStore, _| {
            if store.is_owner(enabled) { store.pending_write() } else { None }
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
        if let Err(err) = desktop::run(request) {
            eprintln!("brindle: {err:#}");
            std::process::exit(1);
        }
        return;
    }

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
        actions::bind_keys(cx);
        // RUST_LOG=brindle=debug shows every keystroke and what it ran.
        cx.observe_keystrokes(|e, _, _| {
            let action = e.action.as_ref().map(|a| a.name()).unwrap_or("no action");
            log::debug!("keystroke {} -> {action}", e.keystroke.unparse());
        })
        .detach();

        cx.on_action(|_: &Quit, cx| cx.quit());
        // Global listeners run inside the dispatching window's update, where
        // `reload_config` couldn't update that window; defer until it's back.
        cx.on_action(|_: &ReloadConfig, cx| cx.defer(reload_config));
        cx.on_action(|_: &NewWindow, cx| {
            let profile = Settings::get(cx).config.default_profile_index();
            open_window(Launch::Request(LaunchRequest::profile(profile)), cx);
        });
        cx.on_window_closed(|cx| {
            let live: Vec<u64> = cx.windows().iter().map(|w| w.window_id().as_u64()).collect();
            cx.update_global(|store: &mut SessionStore, _| store.prune(&live));
            if live.is_empty() {
                cx.quit();
            }
        })
        .detach();

        cx.set_global(SessionStore::default());
        let restored = if args.is_plain() { restorable_windows(cx) } else { Vec::new() };
        let mut windows = Vec::new();
        if restored.is_empty() {
            let launch = LaunchRequest { profile, command: args.command.clone(), cwd: args.cwd.clone() };
            windows.extend(open_window(Launch::Request(launch), cx));
        } else {
            log::info!("restoring {} window(s) from {}", restored.len(), session::state_path(&session::state_dir()).display());
            for state in restored {
                windows.extend(open_window(Launch::Restore(state), cx));
            }
        }
        // Debug steps and the dump act on the first window.
        let Some(&window) = windows.first() else {
            cx.quit();
            return;
        };
        watch_config(cx);
        record_session(cx);

        let steps = args.steps.clone();
        cx.spawn(async move |cx| {
            cx.background_executor().timer(Duration::from_millis(500)).await;
            for step in steps {
                cx.background_executor().timer(Duration::from_secs(1)).await;
                window
                    .update(cx, |ws, window, cx| match &step {
                        Step::Send(text) => ws.send_to_active(text.as_bytes(), cx),
                        Step::Action(name) => match actions::action_by_name(name) {
                            Some(action) => window.dispatch_action(action, cx),
                            None => log::error!("--action: unknown action {name:?}"),
                        },
                    })
                    .ok();
            }
        })
        .detach();
        if let Some(secs) = args.dump_after {
            cx.spawn(async move |cx| {
                cx.background_executor().timer(Duration::from_secs_f64(secs)).await;
                window
                    .update(cx, |ws, window, cx| {
                        print!("{}", ws.dump_active_screen(window.scale_factor(), cx));
                    })
                    .ok();
                cx.update(|cx| cx.quit()).ok();
            })
            .detach();
        }
    });
}
