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
    steps: Vec<String>,
    env: Vec<(String, String)>,
    env_remove: Vec<String>,
    timeline: Vec<(f64, Event)>,
    dump_at: f64,
    capture_at: Option<f64>,
    /// Session restore: a plain launch (no `-p`) with `restore_session`
    /// on and the state in `<case dir>/state`.
    restore: bool,
    default_profile: &'static str,
    args: Vec<String>,
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
            steps: Vec::new(),
            env: Vec::new(),
            env_remove: Vec::new(),
            timeline: Vec::new(),
            dump_at: 4.0,
            capture_at: None,
            restore: false,
            default_profile: "tmux",
            args: Vec::new(),
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
        let mut top = self.top.clone();
        if self.restore {
            top.push_str("restore_session = true\n");
        }
        let text = config_with(&self.server.socket, "e2e", self.default_profile, &top) + &self.extra_config;
        std::fs::write(&config_path, text).map_err(|e| e.to_string())?;
        let log = std::fs::File::create(ctx.dir.join(format!("{}.log", self.label))).map_err(|e| e.to_string())?;

        let mut command = Command::new(&ctx.bin);
        if self.restore {
            command.env("BRINDLE_STATE_DIR", state_dir(ctx));
        } else {
            command.args(["-p", self.profile]);
        }
        for key in &self.env_remove {
            command.env_remove(key);
        }
        let mut child = command
            .args(&self.args)
            .args(&self.steps)
            .args(ctx.step_interval())
            .args(["--dump-screen-after", &ctx.secs(self.dump_at).to_string()])
            .env("BRINDLE_CONFIG", &config_path)
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
        let dump = Dump::parse(&stdout);
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

/// The session-restore state directory of runs made with [`Run::restore`].
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
    /// The active terminal's find bar line, when it is open.
    pub find: Option<String>,
    /// The tab titles of the other windows (`--- window N tabs=[…]`), by
    /// window number.
    pub windows: Vec<(usize, Vec<String>)>,
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
            } else if let Some(rest) = line.strip_prefix("--- settings ") {
                dump.settings = Some(rest.to_string());
                in_screen = false;
            } else if let Some(rest) = line.strip_prefix("--- palette ") {
                dump.palette.push(rest.to_string());
                in_screen = false;
            } else if let Some(rest) = line.strip_prefix("--- input ") {
                dump.input = parse_input(rest);
                in_screen = false;
            } else if let Some(rest) = line.strip_prefix("--- window ") {
                if let Some(window) = parse_window(rest) {
                    dump.windows.push(window);
                }
                in_screen = false;
            } else if let Some(rest) = line.strip_prefix("--- find ") {
                dump.find = Some(rest.to_string());
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

/// `2 tabs=["bash", "w0"]` (titles without commas or quotes).
fn parse_window(text: &str) -> Option<(usize, Vec<String>)> {
    let (number, tabs) = text.split_once(" tabs=")?;
    let inner = tabs.strip_prefix('[')?.strip_suffix(']')?;
    let titles = inner.split(", ").filter(|t| !t.is_empty()).map(|t| t.trim_matches('"').to_string()).collect();
    Some((number.parse().ok()?, titles))
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
    let dump = Dump::parse("--- screen 2x1 mode TermMode(SHOW_CURSOR)\nhi\n--- window 2 tabs=[\"bash\", \"w0\"]\n--- window 3 tabs=[]\n");
    ensure!(dump.screen == ["hi"], "screen before windows {:?}", dump.screen);
    ensure!(dump.windows == [(2, vec!["bash".to_string(), "w0".to_string()]), (3, vec![])], "windows {:?}", dump.windows);
    Ok(())
}
