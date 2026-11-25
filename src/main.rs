// A GUI program on Windows, so launching it opens no console window (see
// `console_output`). Test builds keep the console subsystem.
#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]
// railway R3, types R2: no `_ =>` on one of Brindle's own enums. Non-test
// code only; `docs/architecture/README.md` lists the exceptions.
#![cfg_attr(not(test), warn(clippy::wildcard_enum_match_arm))]

mod actions;
mod config;
mod config_edit;
mod cursor;
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
use crate::terminal_view::{PointerAction, PointerButton};
use crate::session::{PendingWrite, SessionStore, WindowState};
use crate::workspace::{Launch, LaunchRequest, Workspace};

const HELP: &str = "\
Brindle — a GPU-accelerated terminal

USAGE:
    brindle [OPTIONS]            Open the window (the restored tabs, else the
                                 default profile)
    brindle terminal [PROFILE] [-d DIR] [-e COMMAND [ARGS]...] [OPTIONS]
                                 Open a terminal tab
    brindle --attach <ID> [OPTIONS]
                                 Open a tab on a detached session

TERMINAL:
    PROFILE                      A profile's name (default: config's
                                 default_profile)
    -d, --working-directory <DIR>
                                 Start in DIR
    -e, --command <CMD> [ARGS]   Run CMD instead of the profile's program (must
                                 be last: it takes every argument after it)

OPTIONS:
        --config <FILE>          Use FILE instead of ~/.config/brindle/config.toml
        --no-restore             Reopen nothing this time: no saved tabs, no
                                 detached sessions
        --attach <ID>            Open a tab attached to the terminal server's
                                 detached session ID (see brindle-remote list);
                                 not with a subcommand
        --list-actions           Print bindable action names and exit
        --list-themes            Print built-in theme names and exit
        --export-ui-tokens       Print the built-in themes' UI tokens as CSS
                                 (shadcn/ui variable names) and exit
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
        --right-click [MODS-]COL,ROW
                                 Right-click that cell (e.g. shift-5,1)
        --press [MODS-]COL,ROW   Press the left button on that cell and hold it, so
                                 a --hover drags
        --release [MODS-]COL,ROW Release the held left button over that cell
        --find <TEXT>            Open the find bar on the active terminal and search for TEXT
        --wait-for <TEXT>        Hold the later steps until the active terminal shows
                                 TEXT (\\n = a line break), for at most 30 step
                                 intervals (30s by default)
                                 Steps run in order, 1s apart
        --step-interval <S>      Run the steps S seconds apart instead (default 1)
        --dump-screen-after <S>  Print the active tab's screen after S seconds and exit
        --list-fault-switches    Print the BRINDLE_TEST_* fault switches this build
                                 honours (none in a release build) and exit
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
    /// `--hover` / `--click` / `--right-click` / `--press` / `--release`.
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

/// The kinds of tab a subcommand opens: the subcommand's word, the profile
/// check's kind and the hand-off's `kind`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabKind {
    /// `brindle terminal`.
    Terminal,
}

impl TabKind {
    /// The subcommand's word, which is also the hand-off's `kind`.
    pub fn word(self) -> &'static str {
        match self {
            Self::Terminal => "terminal",
        }
    }
}

/// A program and its arguments (`brindle terminal -e`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub program: String,
    pub args: Vec<String>,
}

impl Command {
    /// The command from an argument list: `None` when it is empty.
    pub fn from_argv(argv: &[String]) -> Option<Self> {
        let (program, args) = argv.split_first()?;
        Some(Self { program: program.clone(), args: args.to_vec() })
    }

    /// The program followed by its arguments.
    pub fn argv(&self) -> Vec<String> {
        std::iter::once(self.program.clone()).chain(self.args.iter().cloned()).collect()
    }
}

/// `brindle terminal [PROFILE] [-d DIR] [-e COMMAND…]`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TerminalLaunch {
    pub profile: Option<String>,
    pub cwd: Option<PathBuf>,
    pub command: Option<Command>,
}

/// What a launch asks for beyond the window.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
enum Asked {
    /// Bare `brindle`: the restored tabs, else one default-profile tab; a
    /// hand-off only raises the window.
    #[default]
    Window,
    /// `brindle terminal …`: a new terminal tab, even with nothing named.
    Terminal(TerminalLaunch),
    /// `--attach ID`.
    Attach(u32),
}

/// What the arguments ask for: a run, or text to print before exiting 0
/// (the parser does no IO).
#[derive(Debug)]
enum Parsed {
    Run(Box<Args>),
    Print(Info),
}

/// The informational options' output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Info {
    Help,
    Version,
    Actions,
    FaultSwitches,
    Themes,
    UiTokens,
    /// `--shell-integration SHELL`: that shell's script.
    ShellScript(&'static str),
}

/// What a launch opens as its active tab, after anything restored.
enum Opening {
    Tab(LaunchRequest),
    Attach(u32),
}

#[derive(Default, Debug)]
struct Args {
    /// The subcommand, or `--attach`.
    asked: Asked,
    /// `--config`: the config file, which `main` hands on as
    /// `BRINDLE_CONFIG` before anything reads it.
    config: Option<PathBuf>,
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
}

impl Args {
    /// `--attach`'s session.
    fn attach(&self) -> Option<u32> {
        match &self.asked {
            Asked::Attach(session) => Some(*session),
            Asked::Window | Asked::Terminal(_) => None,
        }
    }

    /// What the launch opens as its active tab, `profile` being the checked
    /// profile of a terminal launch: nothing for bare `brindle`.
    fn opening(&self, profile: usize) -> Option<Opening> {
        match &self.asked {
            Asked::Window => None,
            Asked::Terminal(launch) => Some(Opening::Tab(LaunchRequest {
                profile,
                command: launch.command.clone().map(|c| (c.program, c.args)),
                cwd: launch.cwd.clone(),
            })),
            Asked::Attach(session) => Some(Opening::Attach(*session)),
        }
    }

    /// A run driven by debug options, which runs on its own beside a
    /// running Brindle instead of handing it the request.
    fn is_debug_run(&self) -> bool {
        !self.steps.is_empty() || self.dump_after.is_some() || self.latency_log.is_some() || self.latency_bench.is_some()
    }

    /// This launch as a request to the running Brindle, made where the
    /// launch runs (`here`) with the launcher's activation `token`.
    fn instance_request(&self, here: Option<PathBuf>, token: Option<String>) -> instance::Request {
        let mut request = instance::Request { launch_dir: here, activation_token: token, ..Default::default() };
        match &self.asked {
            Asked::Window => {}
            Asked::Terminal(launch) => {
                request.kind = Some(TabKind::Terminal.word().into());
                request.profile = launch.profile.clone();
                // Relative to where this launch runs, not the instance.
                request.cwd = launch.cwd.as_ref().map(|d| match &request.launch_dir {
                    Some(here) => here.join(d),
                    None => d.clone(),
                });
                request.command = launch.command.as_ref().map(Command::argv);
            }
            Asked::Attach(session) => request.attach = Some(*session),
        }
        request
    }
}

fn parse_args() -> Args {
    match parse_args_from(std::env::args().skip(1), dirs::home_dir().as_deref()) {
        Ok(Parsed::Run(args)) => *args,
        Ok(Parsed::Print(info)) => {
            console_output();
            print_info(info);
            std::process::exit(0);
        }
        Err(message) => usage_error(&message),
    }
}

/// Prints what an informational option asks for.
fn print_info(info: Info) {
    match info {
        Info::Help => print!("{HELP}"),
        Info::Version => println!("brindle {}", env!("CARGO_PKG_VERSION")),
        Info::Actions => {
            for name in actions::action_names() {
                println!("{name}");
            }
        }
        Info::FaultSwitches => {
            for name in fault_switches() {
                println!("{name}");
            }
        }
        Info::Themes => {
            for name in theme::BUILTIN_THEMES {
                println!("{name}");
            }
        }
        Info::UiTokens => print!("{}", theme::ui_tokens_css(env!("CARGO_PKG_VERSION"))),
        Info::ShellScript(script) => print!("{script}"),
    }
}

/// `word` as a shell would need it typed: as it is when it is plain, else
/// in single quotes.
fn shell_word(word: &str) -> String {
    let plain = |c: char| c.is_ascii_alphanumeric() || "_./:=@%+,-~".contains(c);
    if !word.is_empty() && word.chars().all(plain) {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

/// `words` as a shell would need them typed, `placeholder` when there are
/// none.
fn shell_words(words: &[String], placeholder: &str) -> String {
    if words.is_empty() {
        placeholder.to_string()
    } else {
        words.iter().map(|w| shell_word(w)).collect::<Vec<_>>().join(" ")
    }
}

/// The value `option` takes: the next argument, or a usage error naming
/// the option and `what` it needs.
fn value_of(it: &mut impl Iterator<Item = String>, option: &str, what: &str) -> Result<String, String> {
    it.next().ok_or_else(|| format!("{option} needs {what}"))
}

/// What `agent` says until agent tabs exist (`agents-profiles`).
const NO_AGENT_TABS: &str = "agent tabs aren't in this version of Brindle (run brindle terminal for a terminal tab)";

/// The arguments, with a leading `~` in a path expanded to `home`, or the
/// usage error to print: an unknown argument, an option without its value,
/// a value it can't use, a removed option, or a subcommand combined with
/// what it can't take, each naming the option. The debug steps are checked
/// here, so a run never starts with one it can't take.
///
/// One pass, left to right (design.md decision 1): the global options are
/// taken anywhere; the first bare word is the subcommand, and the first
/// after it its PROFILE; `-d` and `-e` belong to the subcommand, and `-e`
/// takes every argument after it.
fn parse_args_from(mut it: impl Iterator<Item = String>, home: Option<&Path>) -> Result<Parsed, String> {
    let mut args = Args::default();
    let (mut install_desktop, mut uninstall_desktop, mut prefix) = (false, false, None);
    let mut subcommand: Option<TabKind> = None;
    let mut terminal = TerminalLaunch::default();
    let mut attach = None;
    while let Some(arg) = it.next() {
        let it = &mut it;
        match arg.as_str() {
            "-h" | "--help" => return Ok(Parsed::Print(Info::Help)),
            "-V" | "--version" => return Ok(Parsed::Print(Info::Version)),
            "--list-actions" => return Ok(Parsed::Print(Info::Actions)),
            "--list-fault-switches" => return Ok(Parsed::Print(Info::FaultSwitches)),
            "--list-themes" => return Ok(Parsed::Print(Info::Themes)),
            "--export-ui-tokens" => return Ok(Parsed::Print(Info::UiTokens)),
            "--shell-integration" => {
                let shell = it.next().unwrap_or_default();
                match SHELL_SCRIPTS.iter().find(|(name, _)| *name == shell) {
                    Some((_, script)) => return Ok(Parsed::Print(Info::ShellScript(script))),
                    None => {
                        let names: Vec<&str> = SHELL_SCRIPTS.iter().map(|(name, _)| *name).collect();
                        return Err(format!(
                            "--shell-integration needs a shell: {} (got {shell:?})",
                            names.join(", ")
                        ));
                    }
                }
            }
            // The flat launch options, removed (decision 3): each says what
            // to type instead, with the user's own values.
            "-p" | "--profile" => {
                let name = it.next().map_or_else(|| "NAME".to_string(), |name| shell_word(&name));
                return Err(format!("{arg} was removed: name the profile after the subcommand, as brindle terminal {name}"));
            }
            "-d" | "--working-directory" if subcommand.is_none() => {
                let dir = it.next().map_or_else(|| "DIR".to_string(), |dir| shell_word(&dir));
                return Err(format!("{arg} belongs to a subcommand now: brindle terminal {arg} {dir}"));
            }
            "-e" | "--command" if subcommand.is_none() => {
                let command = shell_words(&it.collect::<Vec<_>>(), "COMMAND");
                return Err(format!("{arg} belongs to a subcommand now: brindle terminal {arg} {command}"));
            }
            "-e" | "--command" => {
                let rest: Vec<String> = it.collect();
                let Some(command) = Command::from_argv(&rest) else {
                    return Err(format!("{arg} needs a command"));
                };
                terminal.command = Some(command);
            }
            "-d" | "--working-directory" => {
                terminal.cwd = Some(config::expand_tilde(&value_of(it, &arg, "a directory")?, home));
            }
            "--config" => args.config = Some(PathBuf::from(value_of(it, &arg, "a file")?)),
            "--no-restore" => args.no_restore = true,
            "--attach" if cfg!(not(any(target_os = "linux", windows))) => {
                return Err("--attach needs the terminal server, which runs only on Linux and Windows".into());
            }
            "--attach" => match it.next().and_then(|id| id.parse().ok()) {
                Some(id) => attach = Some(id),
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
            "--hover" | "--click" | "--right-click" | "--press" | "--release" => {
                let pos = value_of(it, &arg, "a cell, [MODS-]COL,ROW")?;
                let action = match arg.as_str() {
                    "--click" => PointerAction::Click(PointerButton::Left),
                    "--right-click" => PointerAction::Click(PointerButton::Right),
                    "--press" => PointerAction::Press,
                    "--release" => PointerAction::Release,
                    _ => PointerAction::Hover,
                };
                let step = terminal_view::PointerStep::parse(&pos, action).map_err(|why| format!("{arg}: {why}"))?;
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
            option if option.starts_with('-') => return Err(format!("unknown argument {option:?}")),
            // A bare word: the subcommand, then its profile.
            word => match subcommand {
                None => match word {
                    "terminal" => subcommand = Some(TabKind::Terminal),
                    "agent" => return Err(NO_AGENT_TABS.into()),
                    other => {
                        return Err(format!(
                            "unknown argument {other:?}: to open a profile, run brindle terminal {}",
                            shell_word(other)
                        ));
                    }
                },
                Some(TabKind::Terminal) if terminal.profile.is_none() => terminal.profile = Some(word.to_string()),
                Some(TabKind::Terminal) => {
                    return Err(format!("unexpected argument {word:?}: brindle terminal takes one profile"));
                }
            },
        }
    }
    args.asked = match (subcommand, attach) {
        (Some(kind), Some(_)) => {
            return Err(format!(
                "--attach opens a tab of its own, so it can't be given with brindle {}",
                kind.word()
            ));
        }
        (Some(TabKind::Terminal), None) => Asked::Terminal(terminal),
        (None, Some(session)) => Asked::Attach(session),
        (None, None) => Asked::Window,
    };
    args.desktop = desktop::request(install_desktop, uninstall_desktop, prefix).map_err(str::to_string)?;
    Ok(Parsed::Run(Box::new(args)))
}

/// Prints `message` with the help and exits 2. On Windows a launch with no
/// console, such as a shortcut's, shows the message in a message box too.
fn usage_error(message: &str) -> ! {
    console_output();
    eprintln!("brindle: {message}\n\n{HELP}");
    show_unprinted(&format!("brindle: {message}"));
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

/// Shows `text`, already printed after [`console_output`], in a message box
/// when the launch has no console for it (on Windows, a Start Menu launch).
/// Elsewhere stderr is all: a desktop launch on Linux has the journal.
fn show_unprinted(text: &str) {
    #[cfg(windows)]
    win::show_unprinted(text);
    #[cfg(not(windows))]
    let _ = text;
}

/// Whether this process runs elevated: from its token on Windows, never on
/// Linux.
#[cfg(windows)]
fn elevation() -> Result<bool, String> {
    win::elevated()
}

#[cfg(not(windows))]
fn elevation() -> Result<bool, String> {
    Ok(false)
}

/// Makes and holds an elevated Brindle's own state directory, for
/// Administrators and SYSTEM only; only Windows has one.
#[cfg(windows)]
fn protect_state_dir(dir: &Path) -> Result<(), String> {
    win::protect_state_dir(dir)
}

#[cfg(not(windows))]
fn protect_state_dir(_: &Path) -> Result<(), String> {
    Ok(())
}

/// The launch's state directory, or the window's notice saying why it has
/// none. An elevated Brindle's own on Windows is made for Administrators
/// and SYSTEM only, and held, before the session lock is taken in it.
fn launch_state_dir() -> Result<PathBuf, String> {
    let mut env = session::StateEnv::read();
    match elevation() {
        Ok(elevated) => env.elevated = elevated,
        Err(why) if session::depends_on_elevation(&env) => return Err(session::unknown_elevation_notice(&why)),
        // A named directory is used whatever the level.
        Err(_) => {}
    }
    let dir = session::state_dir(&env).ok_or_else(|| session::NO_STATE_DIR.to_string())?;
    if session::is_elevated_default(&env) {
        protect_state_dir(&dir).map_err(|why| session::elevated_dir_notice(&why))?;
    }
    Ok(dir)
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
    reply: std::sync::mpsc::Sender<instance::Answer>,
}

/// Why a later launch's request opens nothing.
#[derive(Debug, PartialEq, Eq)]
enum Refusal {
    /// Its profile can't open its tab: a usage error for the launch.
    Usage(workspace::LaunchRefusal),
    /// It asks for something this Brindle can't open.
    Other(String),
}

impl Refusal {
    fn answer(self) -> instance::Answer {
        match self {
            Self::Usage(refusal) => instance::Answer::Usage(refusal.to_string()),
            Self::Other(why) => instance::Answer::Refused(why),
        }
    }
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
            let answer = window
                .update(cx, |ws, window, cx| {
                    let settings = Settings::get(cx);
                    match forwarded_launch(&request, &listing, &settings.config, settings.has_config_error()) {
                        Ok(launch) => {
                            if let Some(launch) = launch {
                                ws.open(launch, window, cx);
                            }
                            notify::activation::raise(window, request.activation_token.as_deref(), cx);
                            instance::Answer::Accepted
                        }
                        Err(refusal) => {
                            log::warn!("refused a launch: {refusal:?}");
                            refusal.answer()
                        }
                    }
                })
                .unwrap_or_else(|_| instance::Answer::Refused("Brindle's window is gone".into()));
            reply.send(answer).ok();
        }
    })
    .detach();
}

/// What a later launch's request opens: nothing for a plain launch, which
/// only raises the window. Its profile is checked against `config`, the
/// one the tab would open with (`config_error`: it failed to load, so the
/// defaults apply), and a refused one opens nothing.
fn forwarded_launch(
    request: &instance::Request,
    listing: &Listing,
    config: &Config,
    config_error: bool,
) -> Result<Option<Launch>, Refusal> {
    match request.asked().map_err(Refusal::Other)? {
        instance::Forward::Raise => Ok(None),
        instance::Forward::Attach(session) => {
            Ok(Some(Launch::Attach { session, target: listing.attach_target(session) }))
        }
        instance::Forward::Terminal(launch) => {
            let profile = workspace::profile_for(config, TabKind::Terminal, launch.profile.as_deref(), config_error)
                .map_err(Refusal::Usage)?;
            let command = launch.command.map(|c| (c.program, c.args));
            // A command starts where it was typed, unless `-d` says
            // otherwise.
            let cwd = launch.cwd.or_else(|| command.as_ref().and(request.launch_dir.clone()));
            Ok(Some(Launch::Request(LaunchRequest { profile, command, cwd })))
        }
    }
}

/// The profile a launch that opens its own tab opens with: a terminal
/// launch's, checked against `config` ([`workspace::profile_for`]), else
/// the default one.
fn launch_profile(asked: &Asked, config: &Config, config_error: bool) -> Result<usize, workspace::LaunchRefusal> {
    match asked {
        Asked::Terminal(launch) => {
            workspace::profile_for(config, TabKind::Terminal, launch.profile.as_deref(), config_error)
        }
        Asked::Window | Asked::Attach(_) => Ok(config.default_profile_index()),
    }
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
/// window (the state file's windows merged), whether it `restoring`, the
/// tab it `asked` for (`brindle terminal`'s, or `--attach`'s session) and
/// the `profile` a default tab opens with (design.md decision 3).
///
/// When restoring, `detached` is the listed sessions that are detached and
/// aren't `--attach`'s. The restore is `recorded` without the zones whose
/// session isn't among them; when the listing failed (or was skipped)
/// nothing tells a lost zone from a live one, so every zone is kept and
/// nothing is detached. The asked tab comes after a restore, and without
/// one it is the window's first content, else a default-profile tab is.
fn startup_launch(
    listing: &Listing,
    recorded: Option<WindowState>,
    restoring: bool,
    asked: Option<Opening>,
    profile: usize,
) -> Startup {
    let attach = match &asked {
        Some(Opening::Attach(session)) => Some(*session),
        Some(Opening::Tab(_)) | None => None,
    };
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
    let asked = asked.map(|asked| match asked {
        Opening::Attach(session) => Launch::Attach { session, target: listing.attach_target(session) },
        Opening::Tab(request) => Launch::Request(request),
    });
    match restored {
        Some(window) => Startup { detached, first: Launch::Restore(window), then: asked },
        None => Startup {
            detached,
            first: asked.unwrap_or_else(|| Launch::Request(LaunchRequest::profile(profile))),
            then: None,
        },
    }
}

/// The launch listing's fault switch, which [`launch_listing`] reads and
/// [`fault_switches`] lists.
const LAUNCH_LISTING: &str = "BRINDLE_TEST_LAUNCH_LISTING";

/// The fault switches (`process R20`) this build honours, as
/// `--list-fault-switches` prints them: both in a debug build, none in a
/// release build, which never reads them.
fn fault_switches() -> &'static [&'static str] {
    if cfg!(debug_assertions) { &[terminal::SINK_PANIC, LAUNCH_LISTING] } else { &[] }
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
    match test_listing(std::env::var(LAUNCH_LISTING).ok().as_deref()) {
        Some(TestListing::Fail) => {
            let why = format!("{LAUNCH_LISTING}=fail");
            log::warn!("not attaching detached sessions: {why}");
            return Listing::Failed(why);
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
        // its tabs' sessions (`brindle terminal -e` ones close) itself, every debug run's
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
            let request = args.instance_request(
                std::env::current_dir().ok(),
                instance::launch_token(
                    std::env::var("XDG_ACTIVATION_TOKEN").ok(),
                    std::env::var("DESKTOP_STARTUP_ID").ok(),
                ),
            );
            let line = request.encode();
            if let Err(why) = instance::fits(&line) {
                console_output();
                eprintln!("brindle: {why}");
                show_unprinted(&why);
                std::process::exit(1);
            }
            let refused = match instance::forward(state_dir, &request) {
                Ok(instance::Answer::Accepted) => std::process::exit(0),
                Ok(instance::Answer::Usage(why)) => {
                    console_output();
                    eprintln!("brindle: {why}");
                    show_unprinted(&why);
                    std::process::exit(2);
                }
                Ok(instance::Answer::Refused(why)) | Err(why) => why,
            };
            // The running Brindle may have quit meanwhile.
            match session::Lock::acquire(state_dir) {
                Ok(lock) => {
                    log::info!("the running Brindle went away ({refused}); starting");
                    Some(lock)
                }
                Err(_) => {
                    console_output();
                    eprintln!("brindle: {refused}");
                    show_unprinted(&refused);
                    std::process::exit(1);
                }
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
    if let Some(path) = &args.config {
        // SAFETY: single-threaded at this point, before anything reads the
        // config.
        unsafe { std::env::set_var("BRINDLE_CONFIG", path) };
    }
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
    let (state_dir, notices) = match launch_state_dir() {
        Ok(dir) => (Some(dir), Vec::new()),
        Err(notice) => (None, vec![notice]),
    };
    let lock = match &state_dir {
        Some(state_dir) => session_lock(state_dir, &args),
        None => {
            log::warn!("no state directory: running on its own, without restoring or recording ({})", notices.join("; "));
            None
        }
    };
    // A plain value, read before any event loop exists. This launch opens
    // its own tab, so its profile is checked against the config it opens
    // it with, before any window (a handed-off launch is checked by the
    // running Brindle).
    let (config, report) = Config::load_or_default();
    let profile = launch_profile(&args.asked, &config, report.error.is_some())
        .unwrap_or_else(|refusal| usage_error(&refusal.to_string()));
    let (forwarded, requests) = futures::channel::mpsc::unbounded::<Forwarded>();
    let mut socket = None;
    if lock.is_some()
        && let Some(state_dir) = &state_dir
    {
        let handler = move |request: instance::Request| {
            // Listed here, off the UI thread.
            let listing = request.attach.map_or(Listing::Skipped, |_| running_sessions());
            let (reply, answer) = std::sync::mpsc::channel();
            if forwarded.unbounded_send(Forwarded { request, listing, reply }).is_err() {
                return instance::Answer::Refused("Brindle is quitting".into());
            }
            answer
                .recv_timeout(instance::FORWARD_LIMIT)
                .unwrap_or_else(|_| instance::Answer::Refused("Brindle didn't take the request in time".into()))
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
    let reads = start_launch_reads(restore_from.as_deref(), args.attach().is_some());

    // Before any window: windows, shortcuts and toasts share one identity.
    #[cfg(windows)]
    win::set_app_id();

    Application::new().run(move |cx: &mut App| {
        // This Brindle's id in its tabs' layouts, made once: its pid and
        // when it started.
        let since_epoch = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).unwrap_or_default();
        let origin: gpui::SharedString = layout::origin_id(std::process::id(), since_epoch).into();
        let settings = Settings::from_platform(config, report, cx).with_notices(notices);
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
        let asked = args.opening(profile);
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
                let startup = startup_launch(&reads.listing, reads.recorded, restoring, asked, profile);
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

    fn parsed(args: &[&str]) -> Result<Parsed, String> {
        parse_args_from(args.iter().map(|s| s.to_string()), Some(Path::new("/home/u")))
    }

    fn parse(args: &[&str]) -> Args {
        match parsed(args) {
            Ok(Parsed::Run(parsed)) => *parsed,
            Ok(Parsed::Print(info)) => panic!("{args:?} prints {info:?}"),
            Err(why) => panic!("{args:?}: {why}"),
        }
    }

    /// The usage error `args` give.
    fn refused(args: &[&str]) -> String {
        match parsed(args) {
            Ok(parsed) => panic!("{args:?} parsed: {parsed:?}"),
            Err(why) => why,
        }
    }

    /// The terminal launch `args` ask for.
    fn terminal(args: &[&str]) -> TerminalLaunch {
        match parse(args).asked {
            Asked::Terminal(launch) => launch,
            other => panic!("{args:?} asks for {other:?}"),
        }
    }

    fn command(program: &str, args: &[&str]) -> Option<Command> {
        Some(Command { program: program.into(), args: args.iter().map(|s| s.to_string()).collect() })
    }

    #[test]
    fn an_option_without_its_value_is_a_usage_error() {
        for option in ["-d", "--working-directory"] {
            let why = refused(&["terminal", option]);
            assert!(why.starts_with(&format!("{option} needs ")), "{option}: {why}");
        }
        for option in [
            "--config",
            "--send",
            "--action",
            "--find",
            "--wait-for",
            "--key",
            "--hover",
            "--click",
            "--right-click",
            "--press",
            "--release",
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
        assert_eq!(refused(&["terminal", "-e"]), "-e needs a command");
        assert_eq!(refused(&["terminal", "work", "--command"]), "--command needs a command");
    }

    #[test]
    fn a_terminal_launch_takes_a_profile_a_directory_and_a_command() {
        assert_eq!(terminal(&["terminal"]), TerminalLaunch::default(), "brindle terminal alone asks for a tab");
        let expected =
            TerminalLaunch { profile: Some("Work".into()), cwd: Some("/srv".into()), command: command("htop", &[]) };
        assert_eq!(terminal(&["terminal", "Work", "-d", "/srv", "-e", "htop"]), expected);
        assert_eq!(terminal(&["terminal", "-d", "/srv", "Work", "-e", "htop"]), expected, "PROFILE and -d in either order");
        assert_eq!(
            terminal(&["terminal", "--working-directory", "/srv", "Work", "--command", "htop"]),
            expected,
            "the long forms"
        );
        assert_eq!(terminal(&["terminal", "-d", "~/src"]).cwd, Some(PathBuf::from("/home/u/src")));
    }

    /// The position decides: the first bare word after the subcommand is
    /// its profile, whatever it is called.
    #[test]
    fn a_profile_may_be_named_like_a_subcommand() {
        assert_eq!(terminal(&["terminal", "terminal"]).profile.as_deref(), Some("terminal"));
        assert_eq!(terminal(&["terminal", "agent"]).profile.as_deref(), Some("agent"));
    }

    #[test]
    fn a_second_profile_is_refused() {
        assert_eq!(
            refused(&["terminal", "Work", "Shell"]),
            "unexpected argument \"Shell\": brindle terminal takes one profile"
        );
    }

    /// `-e` takes every argument after it as the command's, debug options
    /// and `-d` included.
    #[test]
    fn dash_e_takes_the_rest() {
        let args = parse(&["terminal", "-e", "grep", "--send", "x", "-d", "/srv"]);
        assert!(!args.is_debug_run(), "no step runs");
        let Asked::Terminal(launch) = args.asked else { panic!("{:?}", args.asked) };
        assert_eq!(launch.command, command("grep", &["--send", "x", "-d", "/srv"]));
        assert_eq!(launch.cwd, None);
    }

    #[test]
    fn global_options_work_before_and_after_the_subcommand() {
        let after = parse(&["terminal", "Shell", "--no-restore", "--send", "echo hi\\r", "--dump-screen-after", "3"]);
        let before = parse(&["--no-restore", "--send", "echo hi\\r", "--dump-screen-after", "3", "terminal", "Shell"]);
        assert_eq!(format!("{after:?}"), format!("{before:?}"));
        assert_eq!(after.asked, Asked::Terminal(TerminalLaunch { profile: Some("Shell".into()), ..Default::default() }));
        assert!(after.no_restore && after.is_debug_run());
    }

    #[cfg(any(target_os = "linux", windows))]
    #[test]
    fn attach_with_a_subcommand_is_refused() {
        let why = "--attach opens a tab of its own, so it can't be given with brindle terminal";
        assert_eq!(refused(&["terminal", "Work", "--attach", "7"]), why);
        assert_eq!(refused(&["--attach", "7", "terminal"]), why);
    }

    #[test]
    fn the_agent_subcommand_is_reserved() {
        for args in [&["agent"][..], &["agent", "x", "-p", "hi"], &["--no-restore", "agent"]] {
            assert_eq!(
                refused(args),
                "agent tabs aren't in this version of Brindle (run brindle terminal for a terminal tab)",
                "{args:?}"
            );
        }
    }

    #[test]
    fn another_bare_word_points_at_the_terminal_subcommand() {
        assert_eq!(refused(&["work"]), "unknown argument \"work\": to open a profile, run brindle terminal work");
        assert_eq!(
            refused(&["Terminal"]),
            "unknown argument \"Terminal\": to open a profile, run brindle terminal Terminal",
            "subcommands are matched case-sensitively"
        );
        assert_eq!(
            refused(&["my work"]),
            "unknown argument \"my work\": to open a profile, run brindle terminal 'my work'"
        );
        assert_eq!(refused(&["--frobnicate"]), "unknown argument \"--frobnicate\"");
    }

    #[test]
    fn the_flat_launch_options_say_what_to_type_instead() {
        let cases: &[(&[&str], &str)] = &[
            (&["-p", "work"], "-p was removed: name the profile after the subcommand, as brindle terminal work"),
            (&["--profile", "my work"], "--profile was removed: name the profile after the subcommand, as brindle terminal 'my work'"),
            (&["-p"], "-p was removed: name the profile after the subcommand, as brindle terminal NAME"),
            (&["terminal", "-p", "work"], "-p was removed: name the profile after the subcommand, as brindle terminal work"),
            (&["-d", "/srv"], "-d belongs to a subcommand now: brindle terminal -d /srv"),
            (&["--working-directory", "/srv"], "--working-directory belongs to a subcommand now: brindle terminal --working-directory /srv"),
            (&["-d"], "-d belongs to a subcommand now: brindle terminal -d DIR"),
            (&["-e", "htop"], "-e belongs to a subcommand now: brindle terminal -e htop"),
            (&["--command", "sh", "-c", "echo hi"], "--command belongs to a subcommand now: brindle terminal --command sh -c 'echo hi'"),
            (&["-e"], "-e belongs to a subcommand now: brindle terminal -e COMMAND"),
            (&["-d", "/srv", "terminal"], "-d belongs to a subcommand now: brindle terminal -d /srv"),
        ];
        for (args, why) in cases {
            assert_eq!(refused(args), *why, "{args:?}");
        }
    }

    #[test]
    fn informational_options_print_wherever_they_are() {
        assert!(matches!(parsed(&["terminal", "--version"]), Ok(Parsed::Print(Info::Version))));
        assert!(matches!(parsed(&["--help"]), Ok(Parsed::Print(Info::Help))));
        assert!(matches!(parsed(&["terminal", "Work", "--list-actions"]), Ok(Parsed::Print(Info::Actions))));
        assert!(matches!(parsed(&["--shell-integration", "bash"]), Ok(Parsed::Print(Info::ShellScript(_)))));
        assert!(refused(&["--shell-integration", "tcsh"]).starts_with("--shell-integration needs a shell: "));
    }

    /// `--config` is handed back for `main` to set, never set here.
    #[test]
    fn the_config_option_is_returned() {
        let before = std::env::var_os("BRINDLE_CONFIG");
        assert_eq!(parse(&["--config", "/tmp/x.toml", "terminal"]).config, Some(PathBuf::from("/tmp/x.toml")));
        assert_eq!(std::env::var_os("BRINDLE_CONFIG"), before);
        assert_eq!(parse(&[]).config, None);
    }

    #[test]
    fn the_help_names_the_subcommand_and_not_the_flat_options() {
        for named in ["brindle terminal", "-d, --working-directory", "-e, --command", "--attach"] {
            assert!(HELP.contains(named), "{named}");
        }
        assert!(!HELP.contains("-p, --profile"));
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
        for (option, position) in
            [
            ("--hover", "3"),
            ("--click", "hyper-3,0"),
            ("--click", "a,b"),
            ("--right-click", "3"),
            ("--right-click", "hyper-3,0"),
            ("--press", "3"),
            ("--release", "a,b"),
        ]
        {
            let why = refused(&[option, position]);
            assert!(why.starts_with(&format!("{option}: ")), "{option} {position}: {why}");
        }
        let args = parse(&["--click", "ctrl-3,1"]);
        let [Step::Pointer(step)] = &args.steps[..] else { panic!("{:?}", args.steps) };
        assert_eq!((step.col, step.row, step.action, step.modifiers.control), (3, 1, PointerAction::Click(PointerButton::Left), true));
        let args = parse(&["--right-click", "shift-5,2"]);
        let [Step::Pointer(step)] = &args.steps[..] else { panic!("{:?}", args.steps) };
        assert_eq!((step.col, step.row, step.action, step.modifiers.shift), (5, 2, PointerAction::Click(PointerButton::Right), true));
        let args = parse(&["--press", "0,0", "--release", "alt-4,1"]);
        let [Step::Pointer(press), Step::Pointer(release)] = &args.steps[..] else { panic!("{:?}", args.steps) };
        assert_eq!((press.col, press.row, press.action), (0, 0, PointerAction::Press));
        assert_eq!((release.col, release.row, release.action, release.modifiers.alt), (4, 1, PointerAction::Release, true));
    }

    #[test]
    fn a_tilde_in_a_path_argument_is_the_home_directory() {
        assert_eq!(terminal(&["terminal", "-d", "~/src"]).cwd, Some(PathBuf::from("/home/u/src")));
        assert_eq!(terminal(&["terminal", "-d", "~"]).cwd, Some(PathBuf::from("/home/u")));
        assert_eq!(parse(&["--latency-log", "~/l.tsv"]).latency_log, Some(PathBuf::from("/home/u/l.tsv")));
        assert_eq!(terminal(&["terminal", "-d", "/srv"]).cwd, Some(PathBuf::from("/srv")));
    }

    /// Bare `brindle` opens nothing of its own; `brindle terminal`, even
    /// alone, asks for a tab; `--attach` for its session.
    #[test]
    fn what_a_launch_opens_as_its_active_tab() {
        for args in [&[][..], &["--send", "x", "--dump-screen-after", "3"], &["--no-restore"]] {
            assert!(parse(args).opening(0).is_none(), "{args:?}");
        }
        let Some(Opening::Tab(tab)) = parse(&["terminal"]).opening(3) else { panic!("brindle terminal opens a tab") };
        assert_eq!((tab.profile, tab.command, tab.cwd), (3, None, None));
        let Some(Opening::Tab(tab)) = parse(&["terminal", "-d", "/tmp", "-e", "htop", "-d", "1"]).opening(1) else {
            panic!("a command opens a tab")
        };
        assert_eq!(
            (tab.profile, tab.command, tab.cwd),
            (1, Some(("htop".into(), vec!["-d".into(), "1".into()])), Some("/tmp".into()))
        );
        #[cfg(any(target_os = "linux", windows))]
        assert!(matches!(parse(&["--attach", "7"]).opening(0), Some(Opening::Attach(7))));
    }

    #[test]
    fn debug_options_make_a_debug_run() {
        assert!(!parse(&["terminal", "work", "-e", "htop"]).is_debug_run());
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
        let here = PathBuf::from("/home/u/src");
        let request = parse(&["terminal", "work", "-d", "rel", "-e", "htop", "-d", "1"])
            .instance_request(Some(here.clone()), Some("tok".into()));
        assert_eq!(request.kind.as_deref(), Some("terminal"));
        assert_eq!(request.profile.as_deref(), Some("work"));
        assert_eq!(request.cwd, Some(here.join("rel")), "-d is made absolute here");
        assert_eq!(request.command, Some(vec!["htop".into(), "-d".into(), "1".into()]));
        assert_eq!((request.launch_dir, request.activation_token.as_deref()), (Some(here), Some("tok")));
        // A bare launch stays as older Brindles read it, and `brindle
        // terminal` alone names only its kind.
        assert_eq!(parse(&[]).instance_request(None, None).encode(), "{}\n");
        assert_eq!(parse(&["terminal"]).instance_request(None, None).encode(), "{\"kind\":\"terminal\"}\n");
        #[cfg(any(target_os = "linux", windows))]
        assert_eq!(parse(&["--attach", "7"]).instance_request(None, None).encode(), "{\"attach\":7}\n");
    }

    #[test]
    fn a_launch_that_opens_its_own_tab_checks_its_profile() {
        let config = Config::parse("default_profile = \"b\"\n[[profiles]]\nname = \"a\"\n[[profiles]]\nname = \"b\"\n").unwrap().config;
        assert_eq!(launch_profile(&parse(&[]).asked, &config, false), Ok(1));
        assert_eq!(launch_profile(&parse(&["terminal"]).asked, &config, false), Ok(1));
        assert_eq!(launch_profile(&parse(&["terminal", "A"]).asked, &config, false), Ok(0));
        let refusal = launch_profile(&parse(&["terminal", "wrk"]).asked, &config, false).unwrap_err();
        assert_eq!(refusal.to_string(), "no profile is named \"wrk\" (the profiles are: a, b)");
    }

    #[test]
    fn forwarded_requests_open_tabs() {
        let config = Config::parse("default_profile = \"a\"\n[[profiles]]\nname = \"a\"\n[[profiles]]\nname = \"b\"\n").unwrap().config;
        let request = |r: instance::Request| forwarded_launch(&r, &Listing::Skipped, &config, false).map(|l| l.map(opens));
        assert_eq!(request(instance::Request { launch_dir: Some("/x".into()), ..Default::default() }), Ok(None), "a plain launch only raises");
        assert_eq!(
            request(instance::Request { command: Some(vec!["htop".into()]), launch_dir: Some("/srv".into()), ..Default::default() }),
            Ok(Some(Opens::Tab(0, Some(("htop".into(), vec![])), Some("/srv".into())))),
            "an older launch's command opens a tab where it was typed"
        );
        assert_eq!(
            request(instance::Request { profile: Some("b".into()), launch_dir: Some("/srv".into()), ..Default::default() }),
            Ok(Some(Opens::Tab(1, None, None))),
            "only a command starts where it was typed"
        );
        assert_eq!(
            request(instance::Request { kind: Some("terminal".into()), ..Default::default() }),
            Ok(Some(Opens::Tab(0, None, None))),
            "brindle terminal alone opens a default-profile tab"
        );
        let Ok(Some(Opens::Attach(7, Err(_)))) = request(instance::Request { attach: Some(7), ..Default::default() }) else {
            panic!("--attach without a listing says why")
        };
    }

    #[test]
    fn a_forwarded_unknown_profile_or_kind_is_refused() {
        let config = Config::parse("[[profiles]]\nname = \"Shell\"\n").unwrap().config;
        let request = |r: instance::Request, error| forwarded_launch(&r, &Listing::Skipped, &config, error).err();
        let wrk = || instance::Request { kind: Some("terminal".into()), profile: Some("wrk".into()), ..Default::default() };
        let Some(Refusal::Usage(refusal)) = request(wrk(), false) else { panic!("an unknown profile is a usage error") };
        assert_eq!(refusal.to_string(), "no profile is named \"wrk\" (the profiles are: Shell)");
        let Some(Refusal::Usage(refusal)) = request(wrk(), true) else { panic!("an unknown profile is a usage error") };
        assert!(refusal.to_string().contains("the config has an error"), "{refusal}");
        let answer = Refusal::Usage(refusal).answer();
        assert!(matches!(answer, instance::Answer::Usage(_)), "{answer:?}");
        let agent = instance::Request { kind: Some("agent".into()), ..Default::default() };
        let Some(Refusal::Other(why)) = request(agent, false) else { panic!("an unknown kind is refused") };
        assert!(why.contains("can't open agent tabs"), "{why}");
    }

    #[test]
    fn forwarded_attach_says_why_listing_failed() {
        let config = Config::parse("[[profiles]]\nname = \"a\"\n").unwrap().config;
        let attach = instance::Request { attach: Some(7), ..Default::default() };
        let opened = |listing: &Listing| forwarded_launch(&attach, listing, &config, false).ok().flatten().map(opens);
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
            Launch::Request(LaunchRequest { profile, command, cwd }) => Opens::Tab(profile, command, cwd),
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
        let asked = match (attach, asked) {
            (Some(_), Some(_)) => panic!("the parser refuses --attach with a subcommand"),
            (Some(session), None) => Some(Opening::Attach(session)),
            (None, asked) => asked.map(Opening::Tab),
        };
        let startup = startup_launch(listing, recorded, restoring, asked, 2);
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
        LaunchRequest { profile: 1, command: Some(("htop".into(), vec![])), cwd: None }
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
        let (detached, first, then) = startup(&listing, Some(window(vec![tab(1, "a")])), true, Some(7), None);
        assert_eq!(first, Opens::Restore(window(vec![tab(1, "a")])));
        assert_eq!(then, Some(Opens::Attach(7, Ok(listed(7, "g", false)))));
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

    #[test]
    fn fault_switches_are_listed_only_in_debug_builds() {
        let listed = fault_switches();
        if cfg!(debug_assertions) {
            assert_eq!(listed, ["BRINDLE_TEST_SINK_PANIC", "BRINDLE_TEST_LAUNCH_LISTING"]);
        } else {
            assert!(listed.is_empty(), "{listed:?}");
        }
        assert!(listed.iter().all(|name| name.starts_with("BRINDLE_TEST_")));
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
        assert_eq!((args.attach(), args.asked), (Some(7), Asked::Attach(7)));
        assert_eq!(parse(&[]).attach(), None);
        assert_eq!(parse(&["terminal"]).attach(), None);
    }
}
