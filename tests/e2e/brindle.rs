//! Running Brindle: the test config, a timeline of tmux commands alongside
//! it, and the `--dump-screen-after` output.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::capture::{self, Image};
use crate::tmux::{TmuxServer, trim_screen};
use crate::{Ctx, Result, ensure};

/// The test config's theme. Cases assert against these colors, so they
/// don't depend on Brindle's built-in themes.
pub mod theme {
    use crate::capture::Rgb;

    pub const FOREGROUND: Rgb = Rgb(0xc0, 0xc0, 0xc0);
    pub const BACKGROUND: Rgb = Rgb(0x10, 0x14, 0x18);
    pub const ACCENT: Rgb = Rgb(0xff, 0x88, 0x00);
    pub const ANSI: [Rgb; 16] = [
        Rgb(0x00, 0x00, 0x00),
        Rgb(0xe0, 0x40, 0x40),
        Rgb(0x40, 0xc0, 0x40),
        Rgb(0xe0, 0xe0, 0x40),
        Rgb(0x40, 0x80, 0xff),
        Rgb(0xd0, 0x40, 0xd0),
        Rgb(0x40, 0xd0, 0xd0),
        Rgb(0xd0, 0xd0, 0xd0),
        Rgb(0x60, 0x60, 0x60),
        Rgb(0xff, 0x60, 0x60),
        Rgb(0x60, 0xff, 0x60),
        Rgb(0xff, 0xff, 0x60),
        Rgb(0x60, 0xa0, 0xff),
        Rgb(0xff, 0x60, 0xff),
        Rgb(0x60, 0xff, 0xff),
        Rgb(0xff, 0xff, 0xff),
    ];
    pub const RED: Rgb = ANSI[1];
    pub const GREEN: Rgb = ANSI[2];
    pub const YELLOW: Rgb = ANSI[3];
    pub const BLUE: Rgb = ANSI[4];
    pub const MAGENTA: Rgb = ANSI[5];

    /// Find bar highlights in the `e2e` theme.
    pub const SEARCH_MATCH_BG: Rgb = Rgb(0x20, 0x50, 0x90);
    pub const SEARCH_MATCH_FG: Rgb = Rgb(0xf0, 0xf0, 0xf0);
    pub const SEARCH_FOCUSED_BG: Rgb = Rgb(0xff, 0x00, 0xa0);
    pub const SEARCH_FOCUSED_FG: Rgb = Rgb(0x00, 0x00, 0x00);

    /// The second theme, for reload checks.
    pub const ALT_FOREGROUND: Rgb = Rgb(0xa0, 0xa0, 0xa0);
    pub const ALT_BACKGROUND: Rgb = Rgb(0x28, 0x20, 0x20);
    /// The second theme's blue; its other ANSI colors are the first's.
    pub const ALT_BLUE: Rgb = Rgb(0x30, 0x60, 0xc0);
}

/// The test config, with `theme_name` as the global theme.
pub fn config(socket: &str, theme_name: &str) -> String {
    config_with(socket, theme_name, "tmux", "")
}

/// The test config with another default profile and extra top-level lines.
pub fn config_with(socket: &str, theme_name: &str, default_profile: &str, top: &str) -> String {
    use theme::*;
    let ansi: Vec<String> = ANSI.iter().map(|c| format!("\"{}\"", c.hex())).collect();
    let mut alt_ansi = ansi.clone();
    alt_ansi[4] = format!("\"{}\"", ALT_BLUE.hex());
    format!(
        r#"default_profile = "{default_profile}"
theme = "{theme_name}"
{top}padding = 6

[cursor]
blink = false

[[profiles]]
name = "Shell"
command = "bash"
args = ["--norc", "--noprofile"]

[[profiles]]
name = "tmux"
tmux = "control"
tmux_session = "e2e"
tmux_args = ["-L", "{socket}", "-f", "/dev/null"]

[themes.e2e]
foreground = "{fg}"
background = "{bg}"
accent = "{accent}"
ansi = [{ansi}]
search_match_background = "{match_bg}"
search_match_foreground = "{match_fg}"
search_focused_match_background = "{focused_bg}"
search_focused_match_foreground = "{focused_fg}"

[themes.e2e-alt]
foreground = "{alt_fg}"
background = "{alt_bg}"
accent = "{accent}"
ansi = [{alt_ansi}]
"#,
        fg = FOREGROUND.hex(),
        bg = BACKGROUND.hex(),
        accent = ACCENT.hex(),
        alt_fg = ALT_FOREGROUND.hex(),
        alt_bg = ALT_BACKGROUND.hex(),
        ansi = ansi.join(", "),
        alt_ansi = alt_ansi.join(", "),
        match_bg = SEARCH_MATCH_BG.hex(),
        match_fg = SEARCH_MATCH_FG.hex(),
        focused_bg = SEARCH_FOCUSED_BG.hex(),
        focused_fg = SEARCH_FOCUSED_FG.hex(),
    )
}

enum Event {
    Tmux(Vec<String>),
    /// Runs a tmux command and saves its output as `<label>-<name>.txt`.
    Record(Vec<String>, String),
    /// Rewrites the config with another theme (Brindle reloads it).
    Theme(&'static str),
    /// Runs a check or an action, given Brindle's pid; an error fails the run.
    Call(Box<dyn FnOnce(u32) -> Result + Send>),
}

/// One run of Brindle attached to a test server's session.
pub struct Run<'a> {
    ctx: &'a Ctx,
    server: &'a TmuxServer,
    label: &'static str,
    profile: &'static str,
    /// Appended to the test config (after its tables).
    extra_config: String,
    /// Top-level keys, placed before the test config's tables.
    top: String,
    /// Brindle should quit by itself (its window closed) before the dump.
    expect_exit: bool,
    /// The window has no tabs at the dump, only a palette.
    without_tabs: bool,
    /// A timeline call kills Brindle, so it prints no dump.
    expect_killed: bool,
    /// Another Brindle binary than `ctx.bin`.
    bin: Option<std::path::PathBuf>,
    steps: Vec<String>,
    env: Vec<(String, String)>,
    env_remove: Vec<String>,
    timeline: Vec<(f64, Event)>,
    dump_at: f64,
    capture_at: Option<f64>,
    /// Session restore: a plain launch (no `-p` or `--no-restore`), with the state in
    /// `<case dir>/state` as for every run.
    restore: bool,
    default_profile: &'static str,
    args: Vec<String>,
    /// `-e`'s command and arguments, when set.
    command: Vec<String>,
    /// Leave the sessions this run left detached for the next run to
    /// attach, instead of hanging them up when it exits.
    keep_detached: bool,
}

/// Hangs up the case server's detached sessions and waits for them to go,
/// so the case's next Brindle (which attaches detached sessions at launch)
/// starts with only the tabs it asks for.
fn hang_up_detached(ctx: &Ctx) -> Result {
    // Only Linux tabs run in the terminal server.
    if cfg!(not(target_os = "linux")) {
        return Ok(());
    }
    let list = || -> Result<Vec<String>> {
        let out = Command::new(&ctx.helper)
            .arg("list")
            .env("BRINDLE_SERVER_DIR", &ctx.server_dir)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("running {}: {e}", ctx.helper.display()))?;
        let text = String::from_utf8_lossy(&out.stdout).into_owned();
        // `id<TAB>pid<TAB>attached|detached<TAB>…`
        Ok(text
            .lines()
            .filter_map(|line| {
                let fields: Vec<&str> = line.split('\t').collect();
                (fields.get(2) == Some(&"detached")).then(|| fields[0].to_string())
            })
            .collect())
    };
    let detached = list()?;
    for id in &detached {
        Command::new(&ctx.helper)
            .args(["kill", id])
            .env("BRINDLE_SERVER_DIR", &ctx.server_dir)
            .stdin(Stdio::null())
            .output()
            .ok();
    }
    let deadline = Instant::now() + Duration::from_secs(10);
    while !detached.is_empty() {
        let left = list()?;
        if !left.iter().any(|id| detached.contains(id)) {
            break;
        }
        ensure!(Instant::now() < deadline, "detached sessions {left:?} didn't end");
        std::thread::sleep(Duration::from_millis(100));
    }
    Ok(())
}

pub struct Output {
    pub dump: Dump,
    pub image: Option<Image>,
}

impl<'a> Run<'a> {
    /// `label` names the run's files in the case directory.
    pub fn new(ctx: &'a Ctx, server: &'a TmuxServer, label: &'static str) -> Self {
        Run {
            ctx,
            server,
            label,
            profile: "tmux",
            extra_config: String::new(),
            top: String::new(),
            expect_exit: false,
            without_tabs: false,
            expect_killed: false,
            bin: None,
            steps: Vec::new(),
            env: Vec::new(),
            env_remove: Vec::new(),
            timeline: Vec::new(),
            dump_at: 4.0,
            capture_at: None,
            restore: false,
            default_profile: "tmux",
            args: Vec::new(),
            command: Vec::new(),
            keep_detached: false,
        }
    }

    /// Launches without `-p`, with session restore on and its state in
    /// [`state_dir`].
    pub fn restore(mut self) -> Self {
        self.restore = true;
        self
    }

    pub fn default_profile(mut self, name: &'static str) -> Self {
        self.default_profile = name;
        self
    }

    /// An extra command-line argument.
    pub fn arg(mut self, arg: &str) -> Self {
        self.args.push(arg.to_string());
        self
    }

    /// Keeps the sessions this run leaves detached (a quit leaves every
    /// local tab's), for the case to check or the next run to attach;
    /// otherwise they are hung up when Brindle exits.
    pub fn keep_detached(mut self) -> Self {
        self.keep_detached = true;
        self
    }

    /// `-e COMMAND ARGS…`, placed after every other argument, since `-e`
    /// takes the rest of the command line.
    pub fn command(mut self, argv: &[&str]) -> Self {
        self.command = argv.iter().map(|a| a.to_string()).collect();
        self
    }

    /// Types text into the active tab (`\r` is Enter). Steps run 1 s apart.
    pub fn send(mut self, text: &str) -> Self {
        self.steps.extend(["--send".to_string(), text.to_string()]);
        self
    }

    /// Sets an environment variable for Brindle (e.g. `RUST_LOG`; its log
    /// is `<label>.log`).
    pub fn env(mut self, key: &str, value: &str) -> Self {
        self.env.push((key.to_string(), value.to_string()));
        self
    }

    /// Removes an environment variable from Brindle's (e.g. `LANG`).
    pub fn env_remove(mut self, key: &str) -> Self {
        self.env_remove.push(key.to_string());
        self
    }

    /// Starts with another profile of the test config (default `tmux`;
    /// `Shell` is a local bash).
    pub fn profile(mut self, name: &'static str) -> Self {
        self.profile = name;
        self
    }

    /// Moves the mouse over a cell of the active terminal, e.g. `ctrl-3,0`
    /// (Brindle's `--hover`, which uses no OS input).
    pub fn hover(mut self, pos: &str) -> Self {
        self.steps.extend(["--hover".to_string(), pos.to_string()]);
        self
    }

    /// Clicks a cell of the active terminal, e.g. `ctrl-3,0` (`--click`).
    pub fn click(mut self, pos: &str) -> Self {
        self.steps.extend(["--click".to_string(), pos.to_string()]);
        self
    }

    /// Adds TOML to the test config, e.g. a `[notifications]` table.
    pub fn config(mut self, text: &str) -> Self {
        self.extra_config.push_str(text);
        self
    }

    /// Adds top-level keys to the test config, e.g. `tmux_session_end = "ask"`.
    pub fn top(mut self, text: &str) -> Self {
        self.top.push_str(text);
        if !text.ends_with('\n') {
            self.top.push('\n');
        }
        self
    }

    /// Expects Brindle to quit by itself before the dump is due (its only
    /// window closed): the run fails if a dump is printed instead. The
    /// returned dump is empty.
    pub fn expect_exit(mut self) -> Self {
        self.expect_exit = true;
        self
    }

    /// Expects a [`Run::call_at`] to kill Brindle before the dump: the run
    /// fails if Brindle exits successfully instead. The dump is empty.
    pub fn expect_killed(mut self) -> Self {
        self.expect_killed = true;
        self
    }

    /// Runs another Brindle binary.
    pub fn bin(mut self, path: &std::path::Path) -> Self {
        self.bin = Some(path.to_path_buf());
        self
    }

    /// Waits for `text` on the active terminal's screen before the next
    /// step (`--wait-for`, at most 30 s). It takes a step's interval even
    /// when the text is already there, so it also spaces steps out.
    pub fn wait_for(mut self, text: &str) -> Self {
        self.steps.extend(["--wait-for".to_string(), text.to_string()]);
        self
    }

    /// Calls `call` with Brindle's pid `secs` after Brindle starts (stretched
    /// like the other timeline events), on the timeline thread. Its error
    /// fails the run; later events wait for it to return.
    pub fn call_at(mut self, secs: f64, call: impl FnOnce(u32) -> Result + Send + 'static) -> Self {
        self.timeline.push((secs, Event::Call(Box::new(call))));
        self
    }

    /// Expects the dumped window to have no tabs, only an open palette, so
    /// the dump has no screen.
    pub fn without_tabs(mut self) -> Self {
        self.without_tabs = true;
        self
    }

    /// Opens the find bar on the active terminal and searches for `text`.
    pub fn find(mut self, text: &str) -> Self {
        self.steps.extend(["--find".to_string(), text.to_string()]);
        self
    }

    pub fn action(mut self, name: &str) -> Self {
        self.steps.extend(["--action".to_string(), name.to_string()]);
        self
    }

    /// Presses a key (`tab`, `shift-tab`, `enter`) through Brindle's own key
    /// dispatch: its bindings, key listeners and keystroke observers, as a
    /// real key press reaches them. In-process, not OS input.
    pub fn key(mut self, keystroke: &str) -> Self {
        self.steps.extend(["--key".to_string(), keystroke.to_string()]);
        self
    }

    /// Counts focus as moved by `pointer` or `keyboard` from this step on,
    /// as a pointer press on a control or a key press would.
    pub fn modality(mut self, modality: &str) -> Self {
        self.steps.extend(["--modality".to_string(), modality.to_string()]);
        self
    }

    /// Runs a tmux command `secs` after Brindle starts.
    pub fn at(mut self, secs: f64, args: &[&str]) -> Self {
        self.timeline.push((secs, Event::Tmux(args.iter().map(|a| a.to_string()).collect())));
        self
    }

    /// Runs a tmux command `secs` after Brindle starts and keeps its output,
    /// read back with [`Run::recorded`].
    pub fn record_at(mut self, secs: f64, name: &str, args: &[&str]) -> Self {
        self.timeline.push((secs, Event::Record(args.iter().map(|a| a.to_string()).collect(), name.to_string())));
        self
    }

    /// The output of a [`Run::record_at`] command of a run labelled `label`.
    pub fn recorded(ctx: &Ctx, label: &str, name: &str) -> Result<String> {
        let path = ctx.dir.join(format!("{label}-{name}.txt"));
        std::fs::read_to_string(&path).map_err(|e| format!("reading {}: {e}", path.display()))
    }

    /// Switches the config to another theme `secs` after Brindle starts.
    pub fn theme_at(mut self, secs: f64, theme: &'static str) -> Self {
        self.timeline.push((secs, Event::Theme(theme)));
        self
    }

    pub fn dump_at(mut self, secs: f64) -> Self {
        self.dump_at = secs;
        self
    }

    /// Captures the window `secs` after Brindle starts.
    pub fn capture_at(mut self, secs: f64) -> Self {
        self.capture_at = Some(secs);
        self
    }

    pub fn run(mut self) -> Result<Output> {
        let ctx = self.ctx;
        let config_path = ctx.dir.join("config.toml");
        let text = config_with(&self.server.socket, "e2e", self.default_profile, &self.top) + &self.extra_config;
        std::fs::write(&config_path, text).map_err(|e| e.to_string())?;
        let log = std::fs::File::create(ctx.dir.join(format!("{}.log", self.label))).map_err(|e| e.to_string())?;

        let mut command = Command::new(self.bin.as_ref().unwrap_or(&ctx.bin));
        // Every run has its own state directory: every launch records the
        // session now, and must never touch the real one.
        command.env("BRINDLE_STATE_DIR", state_dir(ctx));
        if !self.restore {
            // `-p` alone restores too; these runs start clean.
            command.args(["-p", self.profile, "--no-restore"]);
        }
        for key in &self.env_remove {
            command.env_remove(key);
        }
        let mut child = command
            .args(&self.args)
            .args(&self.steps)
            .args(ctx.step_interval())
            .args(["--dump-screen-after", &ctx.secs(self.dump_at).to_string()])
            .args(if self.command.is_empty() { &[][..] } else { &["-e"][..] })
            .args(&self.command)
            .env("BRINDLE_CONFIG", &config_path)
            .env("BRINDLE_SERVER_DIR", &ctx.server_dir)
            .env("BRINDLE_NO_SCOPE", "1")
            .env("DISPLAY", &ctx.display)
            .env_remove("WAYLAND_DISPLAY")
            .envs(self.env.iter().map(|(k, v)| (k, v)))
            .stdout(Stdio::piped())
            .stderr(log)
            .spawn()
            .map_err(|e| format!("starting {}: {e}", ctx.bin.display()))?;
        let started = Instant::now();

        // The timeline runs on its own thread, so captures can happen meanwhile.
        self.timeline.sort_by(|a, b| a.0.total_cmp(&b.0));
        let timeline: Vec<(f64, Event)> = std::mem::take(&mut self.timeline);
        let socket = self.server.socket.clone();
        let slow = ctx.secs(1.0);
        let config_for_reload = config_path.clone();
        let (case_dir, label) = (ctx.dir.clone(), self.label);
        let brindle_pid = child.id();
        let events = std::thread::spawn(move || -> Vec<String> {
            let mut errors = Vec::new();
            for (secs, event) in timeline {
                let due = started + Duration::from_secs_f64(secs * slow);
                std::thread::sleep(due.saturating_duration_since(Instant::now()));
                match event {
                    Event::Tmux(args) => {
                        let out = Command::new("tmux").args(["-L", &socket, "-f", "/dev/null"]).args(&args).output();
                        match out {
                            Ok(o) if o.status.success() => {}
                            Ok(o) => errors.push(format!(
                                "tmux {}: {}",
                                args.join(" "),
                                String::from_utf8_lossy(&o.stderr).trim()
                            )),
                            Err(e) => errors.push(format!("tmux: {e}")),
                        }
                    }
                    Event::Record(args, name) => {
                        let out = Command::new("tmux").args(["-L", &socket, "-f", "/dev/null"]).args(&args).output();
                        let text = match out {
                            Ok(o) => String::from_utf8_lossy(&o.stdout).into_owned(),
                            Err(e) => {
                                errors.push(format!("tmux: {e}"));
                                String::new()
                            }
                        };
                        if let Err(e) = std::fs::write(case_dir.join(format!("{label}-{name}.txt")), text) {
                            errors.push(format!("saving {name}: {e}"));
                        }
                    }
                    Event::Theme(theme) => {
                        if let Err(e) = std::fs::write(&config_for_reload, config(&socket, theme)) {
                            errors.push(format!("rewriting config: {e}"));
                        }
                    }
                    Event::Call(call) => {
                        if let Err(e) = call(brindle_pid) {
                            errors.push(e);
                        }
                    }
                }
            }
            errors
        });

        let mut image = None;
        if let Some(at) = self.capture_at {
            let due = started + Duration::from_secs_f64(ctx.secs(at));
            std::thread::sleep(due.saturating_duration_since(Instant::now()));
            let captured = capture::grab(&ctx.display, child.id())?;
            captured.save_ppm(&ctx.dir.join(format!("{}.ppm", self.label)));
            image = Some(captured);
        }

        let deadline = started + Duration::from_secs_f64(ctx.secs(self.dump_at) + 30.0);
        let status = loop {
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                break status;
            }
            if Instant::now() > deadline {
                child.kill().ok();
                return Err(format!("Brindle didn't exit; see {}.log", self.label));
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        let mut stdout = String::new();
        child.stdout.take().unwrap().read_to_string(&mut stdout).ok();
        ctx.save(&format!("{}.dump", self.label), &stdout);
        let errors = events.join().unwrap_or_default();
        ensure!(errors.is_empty(), "timeline: {}", errors.join("; "));
        if !self.keep_detached {
            hang_up_detached(ctx)?;
        }
        let dump = Dump::parse(&stdout);
        if self.expect_killed {
            ensure!(!status.success(), "Brindle exited by itself ({status}); see {}.log", self.label);
            return Ok(Output { dump, image });
        }
        if self.expect_exit {
            ensure!(
                dump.tabs.is_empty() && dump.size.is_none(),
                "Brindle was still open at the dump; see {}.dump",
                self.label
            );
            ensure!(status.success(), "Brindle exited with {status}; see {}.log", self.label);
            return Ok(Output { dump, image });
        }
        if self.without_tabs {
            ensure!(
                dump.tabs.is_empty() && !dump.palette.is_empty(),
                "expected a palette and no tabs; see {}.dump",
                self.label
            );
        } else {
            ensure!(dump.size.is_some(), "no screen in the dump; see {}.log", self.label);
        }
        Ok(Output { dump, image })
    }
}

/// The session-restore state directory of every run in the case.
pub fn state_dir(ctx: &Ctx) -> std::path::PathBuf {
    ctx.dir.join("state")
}

/// Where the active terminal's cells are, in logical window pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grid {
    pub x: f32,
    pub y: f32,
    pub cell_width: f32,
    pub line_height: f32,
    pub scale: f32,
}

/// Parsed `--dump-screen-after` output.
#[derive(Debug, Default)]
pub struct Dump {
    /// Tab titles, and which tab is active.
    pub tabs: Vec<(String, bool)>,
    pub size: Option<(u16, u16)>,
    /// The active terminal's `TermMode(...)` flags, as printed.
    pub mode: String,
    pub grid: Option<Grid>,
    /// The link under the mouse while hover feedback shows (`--- link`).
    pub link: Option<String>,
    pub screen: Vec<String>,
    /// The settings dialog's line, when it is open.
    pub settings: Option<String>,
    /// The palette's lines (its header first), when it is open.
    pub palette: Vec<String>,
    /// The focused text input, in a palette, the settings dialog or the
    /// find bar.
    pub input: Option<Input>,
    /// The active terminal's selected text, when there is a selection.
    pub selection: Option<String>,
    /// The active terminal's program status records (`--- status …`), one
    /// line each without the prefix.
    pub status: Vec<String>,
    /// The active tab's zone header (`--- zones …`), when it has 2+ zones.
    pub zones: Option<String>,
    /// The active tab's zone lines (`--- zone …`), one line each without the prefix.
    pub zone_list: Vec<String>,
    /// The active terminal's find bar line, when it is open.
    pub find: Option<String>,
    /// Where keyboard focus is (`--- focus`): `terminal`, `palette`,
    /// `settings`, `none` or `other`.
    pub focus: Option<String>,
    /// The focused settings-dialog control (`--- focus settings stop=…`):
    /// `list`, `session_end`, `close`, a field's or a button's label.
    pub focus_stop: Option<String>,
    /// The window's focus-visible modality (`modality=`): `keyboard` or
    /// `pointer`.
    pub modality: Option<String>,
}

/// `--- input text="split" cursor=4 selection=4..4`
#[derive(Debug, PartialEq)]
pub struct Input {
    pub text: String,
    pub cursor: usize,
    pub selection: (usize, usize),
}

fn parse_input(rest: &str) -> Option<Input> {
    let (text, fields) = rest.strip_prefix("text=")?.rsplit_once(" cursor=")?;
    let (cursor, selection) = fields.split_once(" selection=")?;
    let (start, end) = selection.split_once("..")?;
    // The text is Rust-debug quoted; tests use plain ASCII without escapes.
    let text = text.strip_prefix('"')?.strip_suffix('"')?.replace("\\\"", "\"");
    Some(Input { text, cursor: cursor.parse().ok()?, selection: (start.parse().ok()?, end.parse().ok()?) })
}

impl Dump {
    pub fn parse(text: &str) -> Dump {
        let mut dump = Dump::default();
        let mut screen = Vec::new();
        let mut in_screen = false;
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("--- tab ") {
                let title = rest.split_once(' ').map(|(_, t)| t).unwrap_or("");
                let active = title.ends_with(" (active)");
                let title = title.trim_end_matches(" (active)").trim_matches('"').to_string();
                dump.tabs.push((title, active));
            } else if let Some(rest) = line.strip_prefix("--- screen ") {
                let size = rest.split(' ').next().unwrap_or("");
                dump.mode = rest.split_once(" mode ").map(|(_, m)| m.to_string()).unwrap_or_default();
                dump.size = size.split_once('x').and_then(|(c, r)| Some((c.parse().ok()?, r.parse().ok()?)));
                in_screen = true;
            } else if let Some(rest) = line.strip_prefix("--- grid ") {
                dump.grid = parse_grid(rest);
            } else if let Some(rest) = line.strip_prefix("--- link ") {
                dump.link = Some(rest.to_string());
            } else if let Some(rest) = line.strip_prefix("--- selection ") {
                dump.selection = unquote(rest);
            } else if let Some(rest) = line.strip_prefix("--- status ") {
                dump.status.push(rest.to_string());
            } else if let Some(rest) = line.strip_prefix("--- zones ") {
                dump.zones = Some(rest.to_string());
            } else if let Some(rest) = line.strip_prefix("--- zone ") {
                dump.zone_list.push(rest.to_string());
            } else if let Some(rest) = line.strip_prefix("--- settings ") {
                dump.settings = Some(rest.to_string());
                in_screen = false;
            } else if let Some(rest) = line.strip_prefix("--- palette ") {
                dump.palette.push(rest.to_string());
                in_screen = false;
            } else if let Some(rest) = line.strip_prefix("--- input ") {
                dump.input = parse_input(rest);
                in_screen = false;
            } else if let Some(rest) = line.strip_prefix("--- find ") {
                dump.find = Some(rest.to_string());
                in_screen = false;
            } else if let Some(rest) = line.strip_prefix("--- focus ") {
                let (rest, modality) = rest.rsplit_once(" modality=").map_or((rest, None), |(r, m)| (r, Some(m)));
                let (place, stop) = rest.split_once(" stop=").map_or((rest, None), |(p, s)| (p, unquote(s)));
                dump.focus = Some(place.to_string());
                dump.focus_stop = stop;
                dump.modality = modality.map(str::to_string);
                in_screen = false;
            } else if !dump.palette.is_empty() && dump.input.is_none() && !in_screen {
                dump.palette.push(line.to_string());
            } else if in_screen {
                screen.push(line);
            }
        }
        dump.screen = trim_screen(screen.into_iter());
        dump
    }

    /// A `key=value` field of the find line (values without spaces).
    pub fn find_field(&self, key: &str) -> Option<&str> {
        let line = self.find.as_deref()?;
        line.split(' ').find_map(|f| f.strip_prefix(key)?.strip_prefix('='))
    }

    pub fn grid(&self) -> Result<Grid> {
        self.grid.ok_or_else(|| "no grid line in the dump".to_string())
    }
}

/// Undoes the Rust `{:?}` quoting of a string (the escapes it produces
/// for ordinary text).
fn unquote(text: &str) -> Option<String> {
    let inner = text.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next()? {
            'n' => out.push('\n'),
            't' => out.push('\t'),
            'r' => out.push('\r'),
            '0' => out.push('\0'),
            'u' => {
                let hex: String = chars.by_ref().skip(1).take_while(|&c| c != '}').collect();
                out.push(char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?);
            }
            other => out.push(other),
        }
    }
    Some(out)
}

/// `origin=6,51 cell=8.4x18 scale=1`
fn parse_grid(text: &str) -> Option<Grid> {
    let field = |name: &str| text.split(' ').find_map(|f| f.strip_prefix(name));
    let (x, y) = field("origin=")?.split_once(',')?;
    let (w, h) = field("cell=")?.split_once('x')?;
    Some(Grid {
        x: x.parse().ok()?,
        y: y.parse().ok()?,
        cell_width: w.parse().ok()?,
        line_height: h.parse().ok()?,
        scale: field("scale=")?.parse().ok()?,
    })
}

/// Checks the active pane's text and size against tmux.
pub fn matches_tmux(server: &TmuxServer, dump: &Dump, pane: &str) -> Result {
    let [_, _, width, height] = server.geometry(pane)?;
    ensure!(
        dump.size == Some((width, height)),
        "Brindle's grid is {:?}, tmux's pane {pane} is {width}x{height}",
        dump.size
    );
    let tmux = server.capture(pane)?;
    if dump.screen != tmux {
        let first = dump.screen.iter().zip(&tmux).position(|(a, b)| a != b).unwrap_or(dump.screen.len().min(tmux.len()));
        return Err(format!(
            "screen differs from capture-pane of {pane} at line {first}: Brindle {:?}, tmux {:?}",
            dump.screen.get(first),
            tmux.get(first)
        ));
    }
    Ok(())
}

pub fn self_test_dump() -> Result {
    let dump = Dump::parse(
        "--- tab 1 \"bash\" (active)\n--- tab 2 \"vi\"\n--- screen 56x32 mode TermMode(SHOW_CURSOR)\n\
         --- grid origin=6,51.5 cell=8.4x18 scale=2\nhello   \n\nworld\n\n\n",
    );
    ensure!(dump.tabs == [("bash".into(), true), ("vi".into(), false)], "tabs {:?}", dump.tabs);
    ensure!(dump.size == Some((56, 32)), "size {:?}", dump.size);
    ensure!(dump.mode == "TermMode(SHOW_CURSOR)", "mode {:?}", dump.mode);
    let grid = Grid { x: 6.0, y: 51.5, cell_width: 8.4, line_height: 18.0, scale: 2.0 };
    ensure!(dump.grid == Some(grid), "grid {:?}", dump.grid);
    ensure!(dump.screen == ["hello", "", "world"], "screen {:?}", dump.screen);
    ensure!(dump.settings.is_none(), "settings {:?}", dump.settings);
    ensure!(dump.link.is_none(), "link {:?}", dump.link);
    let dump = Dump::parse(
        "--- screen 20x2 mode TermMode(SHOW_CURSOR)\n--- grid origin=0,0 cell=8x16 scale=1\n\
         --- link https://example.com/a\nsee https://example.com/a\n",
    );
    ensure!(dump.link.as_deref() == Some("https://example.com/a"), "link {:?}", dump.link);
    ensure!(dump.screen == ["see https://example.com/a"], "screen with a link {:?}", dump.screen);
    ensure!(dump.selection.is_none(), "selection {:?}", dump.selection);
    let dump = Dump::parse(
        "--- screen 2x1 mode TermMode(SHOW_CURSOR)\n--- selection \"one\\ntwo \\\"q\\\" \\u{1b}\"\nhi\n",
    );
    ensure!(dump.selection.as_deref() == Some("one\ntwo \"q\" \x1b"), "selection {:?}", dump.selection);
    ensure!(dump.screen == ["hi"], "screen after selection {:?}", dump.screen);
    let dump = Dump::parse(
        "--- screen 2x1 mode TermMode(SHOW_CURSOR)\n--- status state=done app=\"brew\"\n\
         --- status state=blocked id=\"b\" kind=auth\nhi\n",
    );
    ensure!(dump.status == ["state=done app=\"brew\"", "state=blocked id=\"b\" kind=auth"], "status {:?}", dump.status);
    ensure!(dump.screen == ["hi"], "screen after status {:?}", dump.screen);
    let dump = Dump::parse(
        "--- screen 2x1 mode TermMode(SHOW_CURSOR)\n--- zones count=2 active=2\n\
         --- zone id=1 rect=0,0,0.5,1\n--- zone id=2 rect=0.5,0,0.5,1 active\nhi\n",
    );
    ensure!(dump.zones.as_deref() == Some("count=2 active=2"), "zones {:?}", dump.zones);
    ensure!(
        dump.zone_list == ["id=1 rect=0,0,0.5,1", "id=2 rect=0.5,0,0.5,1 active"],
        "zone_list {:?}",
        dump.zone_list
    );
    ensure!(dump.screen == ["hi"], "screen after zones {:?}", dump.screen);
    let dump = Dump::parse("--- screen 2x1 mode TermMode(SHOW_CURSOR)\nhi\n--- settings section=Theme mode=list\n");
    ensure!(dump.screen == ["hi"], "screen before settings {:?}", dump.screen);
    ensure!(dump.settings.as_deref() == Some("section=Theme mode=list"), "settings {:?}", dump.settings);
    let dump = Dump::parse(
        "--- screen 2x1 mode TermMode(SHOW_CURSOR)\nhi\n--- palette list query=\"splt\"\ntmux: Split Right  [ctrl-shift-e]\n\
         --- input text=\"split\" cursor=4 selection=1..4\n",
    );
    ensure!(dump.screen == ["hi"], "screen before the palette {:?}", dump.screen);
    ensure!(dump.palette == ["list query=\"splt\"", "tmux: Split Right  [ctrl-shift-e]"], "palette {:?}", dump.palette);
    let input = Input { text: "split".into(), cursor: 4, selection: (1, 4) };
    ensure!(dump.input == Some(input), "input {:?}", dump.input);
    let line = "--- settings section=Profiles mode=editor name=\"tmux\" field=\"Theme\" theme=\"e2e-alt\" tmux=\"control\" default=true invalid=[] can_save=true error=None\n";
    let dump = Dump::parse(line);
    let settings = dump.settings.as_deref().unwrap_or("");
    ensure!(settings.starts_with("section=Profiles mode=editor") && settings.contains("theme=\"e2e-alt\""), "settings {settings:?}");
    ensure!(Dump::parse("nothing").size.is_none(), "size from nothing");
    let dump = Dump::parse(
        "--- screen 2x1 mode TermMode(SHOW_CURSOR)\nhi\n\
         --- find query=\"x\" case=false regex=false count=11 index=1 focused=10,0 invalid=false\n",
    );
    ensure!(dump.screen == ["hi"], "screen before find {:?}", dump.screen);
    ensure!(dump.find_field("count") == Some("11"), "count {:?}", dump.find_field("count"));
    ensure!(dump.find_field("focused") == Some("10,0"), "focused {:?}", dump.find_field("focused"));
    ensure!(dump.find_field("nope").is_none(), "missing field");
    let dump = Dump::parse("--- screen 2x1 mode TermMode(SHOW_CURSOR)\nhi\n--- focus terminal modality=keyboard\n");
    ensure!(dump.screen == ["hi"], "screen before focus {:?}", dump.screen);
    ensure!(dump.focus.as_deref() == Some("terminal"), "focus {:?}", dump.focus);
    ensure!(dump.focus_stop.is_none(), "stop {:?}", dump.focus_stop);
    ensure!(dump.modality.as_deref() == Some("keyboard"), "modality {:?}", dump.modality);
    let dump = Dump::parse(
        "--- palette list query=\"\"\nrow\n--- input text=\"\" cursor=0 selection=0..0\n--- focus palette modality=pointer\n",
    );
    ensure!(dump.palette == ["list query=\"\"", "row"], "palette before focus {:?}", dump.palette);
    ensure!(dump.focus.as_deref() == Some("palette"), "focus after the palette {:?}", dump.focus);
    ensure!(dump.focus_stop.is_none(), "stop {:?}", dump.focus_stop);
    ensure!(dump.modality.as_deref() == Some("pointer"), "modality {:?}", dump.modality);
    let dump = Dump::parse("--- focus settings stop=\"Font size\" modality=keyboard\n");
    ensure!(dump.focus.as_deref() == Some("settings"), "focus {:?}", dump.focus);
    ensure!(dump.focus_stop.as_deref() == Some("Font size"), "stop {:?}", dump.focus_stop);
    Ok(())
}
