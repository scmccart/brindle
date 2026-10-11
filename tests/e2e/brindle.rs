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

    /// The second theme, for reload checks.
    pub const ALT_FOREGROUND: Rgb = Rgb(0xa0, 0xa0, 0xa0);
    pub const ALT_BACKGROUND: Rgb = Rgb(0x28, 0x20, 0x20);
}

/// The test config, with `theme_name` as the global theme.
pub fn config(socket: &str, theme_name: &str) -> String {
    config_with(socket, theme_name, "tmux", "")
}

/// The test config with another default profile and extra top-level lines.
pub fn config_with(socket: &str, theme_name: &str, default_profile: &str, top: &str) -> String {
    use theme::*;
    let ansi: Vec<String> = ANSI.iter().map(|c| format!("\"{}\"", c.hex())).collect();
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

[themes.e2e-alt]
foreground = "{alt_fg}"
background = "{alt_bg}"
accent = "{accent}"
ansi = [{ansi}]
"#,
        fg = FOREGROUND.hex(),
        bg = BACKGROUND.hex(),
        accent = ACCENT.hex(),
        alt_fg = ALT_FOREGROUND.hex(),
        alt_bg = ALT_BACKGROUND.hex(),
        ansi = ansi.join(", "),
    )
}

enum Event {
    Tmux(Vec<String>),
    /// Rewrites the config with another theme (Brindle reloads it).
    Theme(&'static str),
}

/// One run of Brindle attached to a test server's session.
pub struct Run<'a> {
    ctx: &'a Ctx,
    server: &'a TmuxServer,
    label: &'static str,
    profile: &'static str,
    steps: Vec<String>,
    env: Vec<(String, String)>,
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
            steps: Vec::new(),
            env: Vec::new(),
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

    pub fn action(mut self, name: &str) -> Self {
        self.steps.extend(["--action".to_string(), name.to_string()]);
        self
    }

    /// Runs a tmux command `secs` after Brindle starts.
    pub fn at(mut self, secs: f64, args: &[&str]) -> Self {
        self.timeline.push((secs, Event::Tmux(args.iter().map(|a| a.to_string()).collect())));
        self
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
        let top = if self.restore { "restore_session = true\n" } else { "" };
        let text = config_with(&self.server.socket, "e2e", self.default_profile, top);
        std::fs::write(&config_path, text).map_err(|e| e.to_string())?;
        let log = std::fs::File::create(ctx.dir.join(format!("{}.log", self.label))).map_err(|e| e.to_string())?;

        let mut command = Command::new(&ctx.bin);
        if self.restore {
            command.env("BRINDLE_STATE_DIR", state_dir(ctx));
        } else {
            command.args(["-p", self.profile]);
        }
        let mut child = command
            .args(&self.args)
            .args(&self.steps)
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
        loop {
            if child.try_wait().map_err(|e| e.to_string())?.is_some() {
                break;
            }
            if Instant::now() > deadline {
                child.kill().ok();
                return Err(format!("Brindle didn't exit; see {}.log", self.label));
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let mut stdout = String::new();
        child.stdout.take().unwrap().read_to_string(&mut stdout).ok();
        ctx.save(&format!("{}.dump", self.label), &stdout);
        let errors = events.join().unwrap_or_default();
        ensure!(errors.is_empty(), "timeline: {}", errors.join("; "));
        let dump = Dump::parse(&stdout);
        ensure!(dump.size.is_some(), "no screen in the dump; see {}.log", self.label);
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
    /// The focused text input, in a palette or the settings dialog.
    pub input: Option<Input>,
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
            } else if let Some(rest) = line.strip_prefix("--- settings ") {
                dump.settings = Some(rest.to_string());
                in_screen = false;
            } else if let Some(rest) = line.strip_prefix("--- palette ") {
                dump.palette.push(rest.to_string());
                in_screen = false;
            } else if let Some(rest) = line.strip_prefix("--- input ") {
                dump.input = parse_input(rest);
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

    pub fn grid(&self) -> Result<Grid> {
        self.grid.ok_or_else(|| "no grid line in the dump".to_string())
    }
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
    Ok(())
}
