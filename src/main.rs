mod actions;
mod config;
mod picker;
mod settings;
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
    App, AppContext as _, Application, Bounds, WindowBackgroundAppearance, WindowBounds,
    WindowDecorations, WindowOptions, px, size,
};

use crate::actions::{NewWindow, Quit, ReloadConfig};
use crate::config::Config;
use crate::settings::Settings;
use crate::workspace::{LaunchRequest, Workspace};

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
        --list-actions           Print bindable action names and exit
        --list-themes            Print built-in theme names and exit
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
}

fn parse_args() -> Args {
    let mut args = Args::default();
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
            "--send" => args.steps.extend(it.next().map(|s| Step::Send(unescape(&s)))),
            "--action" => args.steps.extend(it.next().map(Step::Action)),
            "--dump-screen-after" => args.dump_after = it.next().and_then(|s| s.parse().ok()),
            other => {
                eprintln!("brindle: unknown argument {other:?}\n\n{HELP}");
                std::process::exit(2);
            }
        }
    }
    args
}

fn unescape(s: &str) -> String {
    s.replace("\\r", "\r").replace("\\n", "\n").replace("\\e", "\x1b").replace("\\t", "\t")
}

pub fn open_window(launch: LaunchRequest, cx: &mut App) -> Option<gpui::WindowHandle<Workspace>> {
    let bounds = Bounds::centered(None, size(px(1000.0), px(660.0)), cx);
    let options = WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(bounds)),
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

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("brindle=info,warn"))
        .init();
    let args = parse_args();

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

        cx.on_action(|_: &Quit, cx| cx.quit());
        cx.on_action(|_: &ReloadConfig, cx| reload_config(cx));
        cx.on_action(|_: &NewWindow, cx| {
            let profile = Settings::get(cx).config.default_profile_index();
            open_window(LaunchRequest::profile(profile), cx);
        });
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                cx.quit();
            }
        })
        .detach();

        let launch = LaunchRequest { profile, command: args.command.clone(), cwd: args.cwd.clone() };
        let Some(window) = open_window(launch, cx) else {
            cx.quit();
            return;
        };
        watch_config(cx);

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
                    .update(cx, |ws, _, cx| {
                        print!("{}", ws.dump_active_screen(cx));
                    })
                    .ok();
                cx.update(|cx| cx.quit()).ok();
            })
            .detach();
        }
    });
}
