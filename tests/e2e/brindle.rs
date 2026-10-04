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

fn config(socket: &str, theme_name: &str) -> String {
    use theme::*;
    let ansi: Vec<String> = ANSI.iter().map(|c| format!("\"{}\"", c.hex())).collect();
    format!(
        r#"default_profile = "tmux"
theme = "{theme_name}"
padding = 6

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
    steps: Vec<String>,
    env: Vec<(String, String)>,
    timeline: Vec<(f64, Event)>,
    dump_at: f64,
    capture_at: Option<f64>,
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
            steps: Vec::new(),
            env: Vec::new(),
            timeline: Vec::new(),
            dump_at: 4.0,
            capture_at: None,
        }
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
        std::fs::write(&config_path, config(&self.server.socket, "e2e")).map_err(|e| e.to_string())?;
        let log = std::fs::File::create(ctx.dir.join(format!("{}.log", self.label))).map_err(|e| e.to_string())?;

        let mut child = Command::new(&ctx.bin)
            .args(["-p", "tmux"])
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
    pub grid: Option<Grid>,
    pub screen: Vec<String>,
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
                dump.size = size.split_once('x').and_then(|(c, r)| Some((c.parse().ok()?, r.parse().ok()?)));
                in_screen = true;
            } else if let Some(rest) = line.strip_prefix("--- grid ") {
                dump.grid = parse_grid(rest);
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
    let grid = Grid { x: 6.0, y: 51.5, cell_width: 8.4, line_height: 18.0, scale: 2.0 };
    ensure!(dump.grid == Some(grid), "grid {:?}", dump.grid);
    ensure!(dump.screen == ["hello", "", "world"], "screen {:?}", dump.screen);
    ensure!(Dump::parse("nothing").size.is_none(), "size from nothing");
    Ok(())
}
