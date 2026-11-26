//! Running Brindle: the test config, a timeline of checks and config
//! changes alongside it, and the `--dump-screen-after` output.

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::capture::{self, Image};
use crate::common::server::{Isolation, isolate};
use crate::common::trim_screen;
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
    pub const YELLOW: Rgb = ANSI[3];
    pub const BLUE: Rgb = ANSI[4];

    /// The window chrome's `background` token for the `e2e` theme: its
    /// background mixed 35% towards black (`Theme::ui_tokens`), which the
    /// tab strip is drawn in, and the gaps round the cards below it.
    pub const CHROME_BACKGROUND: Rgb = Rgb(0x0a, 0x0d, 0x10);
    /// The `border` token for the `e2e` theme: its background mixed 12%
    /// towards its foreground. A card's border is the same mix of its own
    /// terminal theme's colours (`Theme::card_border`), so an `e2e` tab's
    /// cards are drawn in it too.
    pub const CHROME_BORDER: Rgb = Rgb(0x25, 0x29, 0x2c);
    /// The `muted` token for the `e2e` theme, a code block's surface and
    /// the person's bubble: `CHROME_BACKGROUND` mixed 8% towards the
    /// foreground (`Theme::ui_tokens`; `FOREGROUND` is legible on the
    /// chrome's background as it is).
    pub const MUTED: Rgb = Rgb(0x19, 0x1b, 0x1e);

    /// Find bar highlights in the `e2e` theme.
    pub const SEARCH_MATCH_BG: Rgb = Rgb(0x20, 0x50, 0x90);
    pub const SEARCH_MATCH_FG: Rgb = Rgb(0xf0, 0xf0, 0xf0);
    pub const SEARCH_FOCUSED_BG: Rgb = Rgb(0xff, 0x00, 0xa0);
    pub const SEARCH_FOCUSED_FG: Rgb = Rgb(0x00, 0x00, 0x00);

    /// The built-in `brindle-dark` theme's background, which an unknown theme
    /// name falls back to (`Theme::default`).
    pub const BRINDLE_DARK_BACKGROUND: Rgb = Rgb(0x1b, 0x1a, 0x18);

    /// The second theme, for reload checks.
    pub const ALT_FOREGROUND: Rgb = Rgb(0xa0, 0xa0, 0xa0);
    pub const ALT_BACKGROUND: Rgb = Rgb(0x28, 0x20, 0x20);
    /// The second theme's blue; its other ANSI colors are the first's.
    pub const ALT_BLUE: Rgb = Rgb(0x30, 0x60, 0xc0);
    /// The border of a zone's card in a theme of
    /// `ALT_FOREGROUND` on `ALT_BACKGROUND` (`e2e-alt`, and the cases' own
    /// themes with those colours): the background mixed 12% towards the
    /// foreground (`Theme::card_border`).
    pub const ALT_CARD_BORDER: Rgb = Rgb(0x36, 0x2f, 0x2f);
    /// An inactive zone header's title in such a theme: the foreground
    /// mixed 45% towards the background (#6a6666, 2.8:1 on it), pushed 5%
    /// towards white to reach 3:1 (`Theme::card_muted_foreground`).
    pub const ALT_CARD_MUTED: Rgb = Rgb(0x71, 0x6e, 0x6e);
}

/// The test config, with `theme_name` as the global theme. Its one profile,
/// `Shell`, is the default.
pub fn config(theme_name: &str) -> String {
    config_with(theme_name, "Shell", "")
}

/// The test config with another default profile and extra top-level lines.
pub fn config_with(theme_name: &str, default_profile: &str, top: &str) -> String {
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
{shell}

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
        shell = SHELL_PROFILE,
    )
}

/// The `Shell` profile's command: bash, or on Windows PowerShell 7 with the
/// prompt [`PROMPT`].
const SHELL_PROFILE: &str = if cfg!(windows) {
    r#"command = "pwsh.exe"
args = ["-NoLogo", "-NoProfile", "-NoExit", "-Command", "function prompt { 'PS> ' }"]"#
} else {
    r#"command = "bash"
args = ["--norc", "--noprofile"]"#
};

/// The `Shell` profile's prompt on Windows, as the screen shows it at a
/// line's end (without its trailing space).
pub const PROMPT: &str = "PS>";

enum Event {
    /// Rewrites the config with another theme (Brindle reloads it).
    Theme(&'static str),
    /// Runs a check or an action, given Brindle's pid; an error fails the run.
    Call(Box<dyn FnOnce(u32) -> Result + Send>),
}

/// One run of Brindle.
pub struct Run<'a> {
    ctx: &'a Ctx,
    label: &'static str,
    profile: &'static str,
    /// The config's global `theme`: `e2e`, the test config's own.
    theme: &'static str,
    /// Appended to the test config (after its tables).
    extra_config: String,
    /// The whole config file, in place of the test config and `extra_config`.
    whole_config: Option<String>,
    /// Top-level keys, placed before the test config's tables.
    top: String,
    /// Brindle should quit by itself (its window closed) before the dump.
    expect_exit: bool,
    /// A timeline call kills Brindle, so it prints no dump.
    expect_killed: bool,
    /// Another Brindle binary than `ctx.bin`.
    bin: Option<std::path::PathBuf>,
    steps: Vec<String>,
    env: Vec<(String, String)>,
    env_remove: Vec<String>,
    timeline: Vec<(f64, Event)>,
    dump_at: f64,
    /// Dump this many seconds, unscaled, after the last step, in place of
    /// `dump_at` ([`Run::dump_after_last_step`]).
    dump_after_last_step: Option<f64>,
    capture_at: Option<f64>,
    /// Session restore: a plain launch (no `terminal PROFILE` or `--no-restore`), with the state in
    /// `<case dir>/state` as for every run.
    restore: bool,
    default_profile: &'static str,
    args: Vec<String>,
    /// `-e`'s command and arguments, when set.
    command: Vec<String>,
    /// Leave the sessions this run left detached for the next run to
    /// attach, instead of hanging them up when it exits.
    keep_detached: bool,
    /// The terminal padding, in place of the test config's 6.
    padding: Option<u32>,
    /// The steps' spacing in seconds before stretching, in place of 1.
    interval: Option<f64>,
    /// Run with no state directory, from the case directory
    /// ([`Run::no_state_dir`]).
    no_state_dir: bool,
    /// The run opens `brindle agent PROFILE` rather than `brindle
    /// terminal PROFILE` ([`Run::agent`]).
    agent: bool,
    /// Brindle's standard input ([`Run::stdin`]), else the harness's.
    stdin: Option<Stdin>,
}

/// What a run gives Brindle as its standard input ([`Run::stdin`]).
pub enum Stdin {
    /// A file, opened for reading.
    File(std::path::PathBuf),
    /// A pipe the harness writes these bytes into, then closes.
    Pipe(Vec<u8>),
}

/// The case server's sessions as `brindle-remote list` prints them
/// (`id<TAB>pid<TAB>attached|detached<TAB>…`), as `(id, attached)`. Empty
/// while no core runs: `list` never starts one.
pub fn listed_sessions(ctx: &Ctx) -> Result<Vec<(String, bool)>> {
    let out = Command::new(&ctx.helper)
        .arg("list")
        .env("BRINDLE_SERVER_DIR", &ctx.server_dir)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("running {}: {e}", ctx.helper.display()))?;
    parse_listing(out.status.success(), &String::from_utf8_lossy(&out.stdout), &String::from_utf8_lossy(&out.stderr))
}

/// What `brindle-remote list` printed as `(id, attached)`: an error naming
/// its stderr when it failed, since a failed `list` printing nothing looks
/// like a server with no sessions.
fn parse_listing(success: bool, stdout: &str, stderr: &str) -> Result<Vec<(String, bool)>> {
    ensure!(success, "`brindle-remote list` failed: {}", stderr.trim());
    Ok(stdout
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split('\t').collect();
            match fields.get(2) {
                Some(&"attached") => Some((fields[0].to_string(), true)),
                Some(&"detached") => Some((fields[0].to_string(), false)),
                _ => None,
            }
        })
        .collect())
}

/// Hangs up the case server's detached sessions and waits for them to go,
/// so the case's next Brindle (which attaches detached sessions at launch)
/// starts with only the tabs it asks for.
pub fn hang_up_detached(ctx: &Ctx) -> Result {
    // Only the terminal server (Linux and Windows) leaves sessions detached.
    if cfg!(not(any(target_os = "linux", windows))) {
        return Ok(());
    }
    let detached = || -> Result<Vec<String>> {
        Ok(listed_sessions(ctx)?.into_iter().filter(|(_, attached)| !attached).map(|(id, _)| id).collect())
    };
    let hung_up = detached()?;
    // A kill that failed matters only for a session that is still there
    // afterwards: one that ended by itself meanwhile makes `kill` fail too.
    let mut refused: Vec<String> = Vec::new();
    for id in &hung_up {
        let out = Command::new(&ctx.helper)
            .args(["kill", id])
            .env("BRINDLE_SERVER_DIR", &ctx.server_dir)
            .stdin(Stdio::null())
            .output()
            .map_err(|e| format!("running {} kill {id}: {e}", ctx.helper.display()))?;
        if !out.status.success() {
            refused.push(format!("kill {id}: {}", String::from_utf8_lossy(&out.stderr).trim()));
        }
    }
    if hung_up.is_empty() {
        return Ok(());
    }
    ctx.scale().poll(10.0, &format!("detached sessions {hung_up:?} ending"), || {
        let left = detached()?;
        if left.iter().any(|id| hung_up.contains(id)) {
            Err(format!("still listed: {left:?}; {}", if refused.is_empty() { "no kill failed".into() } else { refused.join("; ") }))
        } else {
            Ok(Some(()))
        }
    })
}

/// A thread's panic payload as text, for the error that reports it.
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload
        .downcast_ref::<&str>()
        .map(|text| (*text).to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "a payload that isn't text".into())
}

pub struct Output {
    pub dump: Dump,
    pub image: Option<Image>,
}

impl<'a> Run<'a> {
    /// `label` names the run's files in the case directory.
    pub fn new(ctx: &'a Ctx, label: &'static str) -> Self {
        Run {
            ctx,
            label,
            profile: "Shell",
            theme: "e2e",
            extra_config: String::new(),
            whole_config: None,
            top: String::new(),
            expect_exit: false,
            expect_killed: false,
            bin: None,
            steps: Vec::new(),
            env: Vec::new(),
            env_remove: Vec::new(),
            timeline: Vec::new(),
            dump_at: 4.0,
            dump_after_last_step: None,
            capture_at: None,
            restore: false,
            default_profile: "Shell",
            args: Vec::new(),
            command: Vec::new(),
            keep_detached: false,
            padding: None,
            interval: None,
            no_state_dir: false,
            agent: false,
            stdin: None,
        }
    }

    /// Starts with `brindle agent PROFILE` (an agent profile from
    /// [`Run::config`]) instead of `brindle terminal PROFILE`.
    pub fn agent(mut self, profile: &'static str) -> Self {
        self.profile = profile;
        self.agent = true;
        self
    }

    /// `brindle agent`'s first prompt (`-p TEXT`), for a run started with
    /// [`Run::agent`]. Not `prompt`: that name waits for a shell's prompt.
    pub fn first_prompt(mut self, text: &str) -> Self {
        self.args.extend(["-p".to_string(), text.to_string()]);
        self
    }

    /// Gives Brindle `stdin` as its standard input (for `-p -`), in place
    /// of the harness's own.
    pub fn stdin(mut self, stdin: Stdin) -> Self {
        self.stdin = Some(stdin);
        self
    }

    /// Runs with no state directory: without the `BRINDLE_STATE_DIR` every
    /// other run gets, `HOME` and `XDG_STATE_HOME`, and from the case
    /// directory, where the state went when Brindle fell back on `.`.
    pub fn no_state_dir(mut self) -> Self {
        self.no_state_dir = true;
        self
    }

    /// Spaces the steps `secs` apart (stretched like the other delays)
    /// instead of one second, which also scales a `--wait-for` limit.
    pub fn step_interval(mut self, secs: f64) -> Self {
        self.interval = Some(secs);
        self
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

    /// Starts with `name` as the config's global theme in place of `e2e`,
    /// which needn't exist (an unknown one is what a check is for).
    pub fn initial_theme(mut self, name: &'static str) -> Self {
        self.theme = name;
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

    /// Starts with another profile (default `Shell`, the test config's local
    /// bash; others come from [`Run::config`]).
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

    /// Right-clicks a cell of the active terminal, e.g. `shift-3,0`
    /// (`--right-click`).
    pub fn right_click(mut self, pos: &str) -> Self {
        self.steps.extend(["--right-click".to_string(), pos.to_string()]);
        self
    }

    /// Presses the left button on a cell of the active terminal and holds
    /// it, e.g. `0,0` (`--press`); a [`Run::hover`] then drags.
    pub fn press(mut self, pos: &str) -> Self {
        self.steps.extend(["--press".to_string(), pos.to_string()]);
        self
    }

    /// Releases the held left button over a cell of the active terminal,
    /// with no move first (`--release`).
    pub fn release(mut self, pos: &str) -> Self {
        self.steps.extend(["--release".to_string(), pos.to_string()]);
        self
    }

    /// Adds TOML to the test config, e.g. a `[notifications]` table.
    pub fn config(mut self, text: &str) -> Self {
        self.extra_config.push_str(text);
        self
    }

    /// Makes `text` the whole config file, in place of the test config, for a
    /// check that needs a file the test config's tables would make invalid
    /// (`themes = 3`). It has no `e2e` theme and no profile, so the window
    /// uses the built-in theme and the implicit `Shell` profile.
    pub fn whole_config(mut self, text: &str) -> Self {
        self.whole_config = Some(text.to_string());
        self
    }

    /// Sets the config's terminal `padding` (logical pixels) instead of the
    /// test config's 6.
    pub fn padding(mut self, padding: u32) -> Self {
        self.padding = Some(padding);
        self
    }

    /// Adds top-level keys to the test config, e.g. `scrollback = 100`.
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
    /// step (`--wait-for`, at most 30 step intervals). It takes a step's interval even
    /// when the text is already there, so it also spaces steps out.
    pub fn wait_for(mut self, text: &str) -> Self {
        self.steps.extend(["--wait-for".to_string(), text.to_string()]);
        self
    }

    /// Waits for the `Shell` profile's prompt on Windows, whose PowerShell
    /// takes a while to start; bash needs no wait.
    pub fn prompt(self) -> Self {
        if cfg!(windows) { self.wait_for(PROMPT) } else { self }
    }

    /// Calls `call` with Brindle's pid `secs` after Brindle starts (stretched
    /// like the other timeline events), on the timeline thread. Its error
    /// fails the run; later events wait for it to return.
    pub fn call_at(mut self, secs: f64, call: impl FnOnce(u32) -> Result + Send + 'static) -> Self {
        self.timeline.push((secs, Event::Call(Box::new(call))));
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

    /// Switches the config to another theme `secs` after Brindle starts.
    pub fn theme_at(mut self, secs: f64, theme: &'static str) -> Self {
        self.timeline.push((secs, Event::Theme(theme)));
        self
    }

    pub fn dump_at(mut self, secs: f64) -> Self {
        self.dump_at = secs;
        self.dump_after_last_step = None;
        self
    }

    /// Dumps `secs` after the last step, in place of [`Run::dump_at`]. The
    /// steps' times are scaled as usual, but `secs` is not: it is an offset
    /// measured against one of the app's own limits, which don't stretch
    /// with `BRINDLE_E2E_SLOW` (`testing R5`), so the caller names that
    /// limit beside the call. Brindle counts steps and dump from the same
    /// origin, so the gap on its own clock is exactly `secs`, unless a
    /// `--wait-for` step held the steps back.
    pub fn dump_after_last_step(mut self, secs: f64) -> Self {
        self.dump_after_last_step = Some(secs);
        self
    }

    /// When Brindle dumps, in seconds on its own clock, as passed to
    /// `--dump-screen-after`.
    fn dump_secs(&self) -> f64 {
        let ctx = self.ctx;
        match self.dump_after_last_step {
            Some(offset) => {
                let interval = ctx.secs(self.interval.unwrap_or(1.0));
                last_step_secs(self.steps.len() / 2, interval) + offset
            }
            None => ctx.secs(self.dump_at),
        }
    }

    /// Captures the window `secs` after it first appears, which is when
    /// Brindle's steps and dump start counting (other timeline events count
    /// from the spawn).
    pub fn capture_at(mut self, secs: f64) -> Self {
        self.capture_at = Some(secs);
        self
    }

    pub fn run(mut self) -> Result<Output> {
        let ctx = self.ctx;
        // A fault switch the case didn't declare would fail confusingly on a
        // build without it, instead of skipping (`testing R18`).
        if let Some(why) = self.env.iter().find_map(|(key, _)| crate::switches::undeclared(key, ctx.needs)) {
            return Err(why);
        }
        let dump_secs = self.dump_secs();
        let config_path = ctx.dir.join("config.toml");
        let mut text = config_with(self.theme, self.default_profile, &self.top);
        if let Some(padding) = self.padding {
            text = text.replacen("\npadding = 6\n", &format!("\npadding = {padding}\n"), 1);
        }
        text += &self.extra_config;
        if let Some(whole) = &self.whole_config {
            text.clone_from(whole);
        }
        std::fs::write(&config_path, text).map_err(|e| e.to_string())?;
        let log = std::fs::File::create(ctx.dir.join(format!("{}.log", self.label))).map_err(|e| e.to_string())?;

        let mut command = Command::new(self.bin.as_ref().unwrap_or(&ctx.bin));
        // Every run has its own config, state and server directories (every
        // launch records the session, and must never touch the real one);
        // the case's own removals and additions come after.
        let state = state_dir(ctx);
        isolate(
            &mut command,
            &Isolation { config: &config_path, state_dir: &state, server_dir: &ctx.server_dir, display: &ctx.display },
        );
        if !self.restore {
            // `brindle terminal PROFILE` alone restores too; these runs
            // start clean. `--attach` opens a tab of its own and can't be
            // given with a subcommand, so an attaching run names none.
            if self.args.iter().any(|arg| arg == "--attach") {
                command.arg("--no-restore");
            } else {
                let subcommand = if self.agent { "agent" } else { "terminal" };
                command.args([subcommand, self.profile, "--no-restore"]);
            }
        }
        // A restoring run with a command still needs the subcommand, which
        // `-e` belongs to.
        let subcommand = if self.restore && !self.command.is_empty() { &["terminal"][..] } else { &[][..] };
        for key in &self.env_remove {
            command.env_remove(key);
        }
        if self.no_state_dir {
            command.env_remove("BRINDLE_STATE_DIR").env_remove("HOME").env_remove("XDG_STATE_HOME").current_dir(&ctx.dir);
        }
        match &self.stdin {
            Some(Stdin::File(path)) => {
                let file = std::fs::File::open(path).map_err(|e| format!("opening {}: {e}", path.display()))?;
                command.stdin(file);
            }
            Some(Stdin::Pipe(_)) => {
                command.stdin(Stdio::piped());
            }
            None => {}
        }
        let mut child = command
            .args(&self.args)
            .args(&self.steps)
            .args(match self.interval {
                Some(secs) => vec!["--step-interval".to_string(), ctx.secs(secs).to_string()],
                None => ctx.step_interval(),
            })
            .args(["--dump-screen-after", &dump_secs.to_string()])
            .args(subcommand)
            .args(if self.command.is_empty() { &[][..] } else { &["-e"][..] })
            .args(&self.command)
            .envs(self.env.iter().map(|(k, v)| (k, v)))
            .stdout(Stdio::piped())
            .stderr(log)
            .spawn()
            .map(Reaped)
            .map_err(|e| format!("starting {}: {e}", ctx.bin.display()))?;
        if let (Some(Stdin::Pipe(bytes)), Some(mut pipe)) = (&self.stdin, child.stdin.take()) {
            // Brindle reads it before it opens a window; the pipe closes
            // when `pipe` drops.
            std::io::Write::write_all(&mut pipe, bytes).map_err(|e| format!("writing Brindle's stdin: {e}"))?;
        }
        let started = Instant::now();
        // Captures count from the window's first appearance, as Brindle's
        // steps and dump do; the rest of the timeline counts from the spawn.
        let window_seen = self.capture_at.map(|_| capture::watch_window(&ctx.display, child.id(), ctx.secs(1.0)));

        // The timeline runs on its own thread, so captures can happen meanwhile.
        self.timeline.sort_by(|a, b| a.0.total_cmp(&b.0));
        let timeline: Vec<(f64, Event)> = std::mem::take(&mut self.timeline);
        let slow = ctx.secs(1.0);
        let config_for_reload = config_path.clone();
        let brindle_pid = child.id();
        let events = std::thread::spawn(move || -> Vec<String> {
            let mut errors = Vec::new();
            for (secs, event) in timeline {
                let due = started + Duration::from_secs_f64(secs * slow);
                std::thread::sleep(due.saturating_duration_since(Instant::now()));
                match event {
                    Event::Theme(theme) => {
                        if let Err(e) = std::fs::write(&config_for_reload, config(theme)) {
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
        if let (Some(at), Some(window_seen)) = (self.capture_at, window_seen) {
            let seen = window_seen.recv().map_err(|_| {
                format!(
                    "no viewable window for pid {} on {} within {:.1} s",
                    child.id(),
                    ctx.display,
                    ctx.secs(capture::WINDOW_WAIT_SECS)
                )
            })?;
            let due = seen + Duration::from_secs_f64(ctx.secs(at));
            std::thread::sleep(due.saturating_duration_since(Instant::now()));
            let captured = capture::grab(&ctx.display, child.id(), ctx.secs(1.0))?;
            let path = ctx.dir.join(format!("{}.ppm", self.label));
            captured.save_ppm(&path).map_err(|e| format!("saving {}: {e}", path.display()))?;
            image = Some(captured);
        }

        // The dump's time, then 30 s (stretched) to print it and quit.
        let deadline = started + Duration::from_secs_f64(dump_secs + ctx.secs(30.0));
        let status = loop {
            if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
                break status;
            }
            if Instant::now() > deadline {
                return Err(format!("Brindle didn't exit; see {}.log", self.label));
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        let mut stdout = String::new();
        child.stdout.take().unwrap().read_to_string(&mut stdout).ok();
        ctx.save(&format!("{}.dump", self.label), &stdout);
        let errors = events.join().map_err(|payload| format!("the timeline thread panicked: {}", panic_message(&*payload)))?;
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
        ensure!(dump.size.is_some() || !dump.agent.is_empty(), "no screen or agent zone in the dump; see {}.log", self.label);
        Ok(Output { dump, image })
    }
}

/// Brindle as `Run::run` spawned it. Dropping it kills Brindle if it is
/// still running and waits for it, so a run that fails early (no window to
/// capture, no exit by the deadline) never leaves it running into the next
/// case, where it would later die on a closed stdout.
struct Reaped(Child);

impl std::ops::Deref for Reaped {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.0
    }
}

impl std::ops::DerefMut for Reaped {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.0
    }
}

impl Drop for Reaped {
    fn drop(&mut self) {
        if let Ok(None) = self.0.try_wait() {
            self.0.kill().ok();
        }
        self.0.wait().ok();
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

/// The first words of an agent zone's dump lines ([`Dump::agent`]).
const AGENT_LINES: [&str; 9] =
    ["agent ", "login ", "note ", "entry ", "block ", "link ", "queued ", "conversation ", "prompt "];

/// Parsed `--dump-screen-after` output.
#[derive(Debug, Default)]
pub struct Dump {
    /// Tab titles, and which tab is active.
    pub tabs: Vec<(String, bool)>,
    pub size: Option<(u16, u16)>,
    /// The active terminal's `TermMode(...)` flags, as printed.
    pub mode: String,
    pub grid: Option<Grid>,
    /// The families in effect, preview included (`--- fonts`).
    pub fonts: Option<Fonts>,
    /// The window's config line, unescaped (`--- config`), when it shows one.
    pub config: Option<String>,
    /// The link under the mouse while hover feedback shows (`--- link`).
    pub link: Option<String>,
    pub screen: Vec<String>,
    /// The settings dialog's line, when it is open.
    pub settings: Option<String>,
    /// The palette's lines (its header first), when it is open.
    pub palette: Vec<String>,
    /// An open menu's lines, when one is open: its header without the
    /// prefix (`items=A|[B]|-|C highlighted=N|none`), then one line per item
    /// (`label  description  [keys]`).
    pub menu: Vec<String>,
    /// The focused text input, in a palette, the settings dialog or the
    /// find bar.
    pub input: Option<Input>,
    /// The active terminal's selected text, when there is a selection.
    pub selection: Option<String>,
    /// The active terminal's program status records (`--- status …`), one
    /// line each without the prefix.
    pub status: Vec<String>,
    /// The progress bars shown (`--- progress tab=N …` for each tab with
    /// one, then `--- progress zone=ID …` for the active tab's zones), one
    /// line each without the prefix.
    pub progress: Vec<String>,
    /// The active terminal's open contexts (`--- context …`), outermost
    /// first, one line each without the prefix.
    pub contexts: Vec<String>,
    /// The context badges shown (`--- badge tab=N …` for each tab with one,
    /// then `--- badge zone=ID …` for the active tab's zones), one line
    /// each without the prefix.
    pub badges: Vec<String>,
    /// The label a mouse copy or paste shows at the pointer, while one isn't
    /// gone (`--- feedback kind=copied|pasted|nothing-to-paste`): the kind.
    pub feedback: Option<String>,
    /// The program's pointer shape on the active terminal's screen
    /// (`--- pointer shape=NAME screen=primary|alternate`), without the
    /// prefix; none while the pointer is Brindle's own.
    pub pointer: Option<String>,
    /// The pointer the active terminal's last painted frame chose for its
    /// grid (`--- grid-pointer NAME`, a CSS name such as `text`), without
    /// the prefix.
    pub grid_pointer: Option<String>,
    /// The active tab's zone header (`--- zones …`), when it has 2+ zones.
    pub zones: Option<String>,
    /// The active tab's zone lines (`--- zone …`), one line each without the prefix.
    pub zone_list: Vec<String>,
    /// The active terminal's find bar line, when it is open.
    pub find: Option<String>,
    /// Where keyboard focus is (`--- focus`): `terminal`, `palette`,
    /// `settings`, `tabs` (the tab strip), `menu`, `none` or `other`.
    pub focus: Option<String>,
    /// The focused settings-dialog control (`--- focus settings stop=…`):
    /// `sections`, `list`, `close`, a field's or a button's label; an
    /// agent zone's stop (`--- focus agent stop=prompt|queued:remove|…`);
    /// the close confirmation's button (`--- focus confirm stop=cancel|close`).
    pub focus_stop: Option<String>,
    /// The close confirmation's line while it is open (`--- confirm close
    /// target=zone:ID|tab:N busy=…`), without the `--- confirm `.
    pub confirm: Option<String>,
    /// The window's focus-visible modality (`modality=`): `keyboard` or
    /// `pointer`.
    pub modality: Option<String>,
    /// The `--wait-for` texts that gave up (`--- wait-for timeout "TEXT"`).
    pub wait_timeouts: Vec<String>,
    /// The `--wait-for` texts still waiting at the dump (`--- wait-for pending "TEXT"`).
    pub wait_pending: Vec<String>,
    /// An active agent zone's block (`--- agent …`, `--- note …`, `--- entry …`,
    /// `--- queued …`, `--- conversation …`, `--- prompt …`), in place of a
    /// screen: one line each without the `--- `.
    pub agent: Vec<String>,
}

/// `--- fonts terminal="DejaVu Sans Mono" ui="Inter"`
#[derive(Debug, PartialEq)]
pub struct Fonts {
    pub terminal: String,
    pub ui: String,
}

fn parse_fonts(rest: &str) -> Option<Fonts> {
    let (terminal, ui) = rest.strip_prefix("terminal=")?.rsplit_once(" ui=")?;
    Some(Fonts { terminal: unquote(terminal)?, ui: unquote(ui)? })
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
    // The text is Rust-debug quoted (an agent's prompt can hold line breaks).
    let text = unquote(text)?;
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
            } else if let Some(rest) = line.strip_prefix("--- fonts ") {
                dump.fonts = parse_fonts(rest);
            } else if let Some(rest) = line.strip_prefix("--- config ") {
                dump.config = unquote(rest);
            } else if let Some(rest) = line.strip_prefix("--- link ").filter(|rest| !rest.starts_with("entry=")) {
                // A terminal's hovered link; an agent zone's links name
                // their entry and are agent lines.
                dump.link = Some(rest.to_string());
            } else if let Some(rest) = line.strip_prefix("--- selection ") {
                dump.selection = unquote(rest);
            } else if let Some(rest) = line.strip_prefix("--- wait-for timeout ") {
                dump.wait_timeouts.extend(unquote(rest));
                in_screen = false;
            } else if let Some(rest) = line.strip_prefix("--- wait-for pending ") {
                dump.wait_pending.extend(unquote(rest));
                in_screen = false;
            } else if let Some(rest) = line.strip_prefix("--- status ") {
                dump.status.push(rest.to_string());
            } else if let Some(rest) = line.strip_prefix("--- progress ") {
                dump.progress.push(rest.to_string());
            } else if let Some(rest) = line.strip_prefix("--- context ") {
                dump.contexts.push(rest.to_string());
            } else if let Some(rest) = line.strip_prefix("--- badge ") {
                dump.badges.push(rest.to_string());
            } else if let Some(rest) = line.strip_prefix("--- feedback ") {
                dump.feedback = rest.strip_prefix("kind=").map(str::to_string);
            } else if let Some(rest) = line.strip_prefix("--- pointer ") {
                dump.pointer = Some(rest.to_string());
            } else if let Some(rest) = line.strip_prefix("--- grid-pointer ") {
                dump.grid_pointer = Some(rest.to_string());
            } else if let Some(rest) =
                line.strip_prefix("--- ").filter(|rest| AGENT_LINES.iter().any(|word| rest.starts_with(word)))
            {
                dump.agent.push(rest.to_string());
                in_screen = false;
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
            } else if let Some(rest) = line.strip_prefix("--- menu ") {
                dump.menu.push(rest.to_string());
                in_screen = false;
            } else if let Some(rest) = line.strip_prefix("--- input ") {
                dump.input = parse_input(rest);
                in_screen = false;
            } else if let Some(rest) = line.strip_prefix("--- find ") {
                dump.find = Some(rest.to_string());
                in_screen = false;
            } else if let Some(rest) = line.strip_prefix("--- confirm ") {
                dump.confirm = Some(rest.to_string());
                in_screen = false;
            } else if let Some(rest) = line.strip_prefix("--- focus ") {
                let (rest, modality) = rest.rsplit_once(" modality=").map_or((rest, None), |(r, m)| (r, Some(m)));
                // The settings dialog quotes its stop (a label); an agent
                // zone's and the close confirmation's are bare words.
                let (place, stop) =
                    rest.split_once(" stop=").map_or((rest, None), |(p, s)| (p, unquote(s).or_else(|| Some(s.to_string()))));
                dump.focus = Some(place.to_string());
                dump.focus_stop = stop;
                dump.modality = modality.map(str::to_string);
                in_screen = false;
            } else if !dump.palette.is_empty() && dump.input.is_none() && !in_screen {
                dump.palette.push(line.to_string());
            } else if !dump.menu.is_empty() && dump.input.is_none() && dump.focus.is_none() && !in_screen {
                dump.menu.push(line.to_string());
            } else if in_screen {
                screen.push(line);
            }
        }
        dump.screen = trim_screen(screen.into_iter());
        dump
    }

    /// A `key=value` field of the agent zone's `--- agent` line (values
    /// without spaces: `state`, `turn`, `session`, …).
    pub fn agent_field(&self, key: &str) -> Option<&str> {
        let line = self.agent.iter().find_map(|line| line.strip_prefix("agent "))?;
        line.split(' ').find_map(|f| f.strip_prefix(key)?.strip_prefix('='))
    }

    /// The agent zone's login row (`--- login choice …` or `--- login
    /// running …`), each line without `login `.
    pub fn agent_logins(&self) -> Vec<&str> {
        self.agent.iter().filter_map(|line| line.strip_prefix("login ")).collect()
    }

    /// The agent zone's entries (`--- entry N KIND …`), each as `KIND …`
    /// without its id: `user "hi"`, `agent "Hello!"`.
    pub fn agent_entries(&self) -> Vec<&str> {
        self.agent
            .iter()
            .filter_map(|line| line.strip_prefix("entry ")?.split_once(' ').map(|(_, rest)| rest))
            .collect()
    }

    /// Entry `entry`'s parts (`--- block entry=N part=P kind=K "TEXT"`),
    /// each as `part=P kind=K "TEXT"`.
    pub fn agent_blocks(&self, entry: u64) -> Vec<&str> {
        self.entry_lines("block ", entry)
    }

    /// The links in entry `entry`'s parts (`--- link entry=N part=P
    /// target="…" text="…" action=A`), each as `part=P target=… text=…
    /// action=A`.
    pub fn agent_links(&self, entry: u64) -> Vec<&str> {
        self.entry_lines("link ", entry)
    }

    fn entry_lines(&self, word: &str, entry: u64) -> Vec<&str> {
        let prefix = format!("entry={entry} ");
        self.agent.iter().filter_map(|line| line.strip_prefix(word)?.strip_prefix(prefix.as_str())).collect()
    }

    /// A `key=value` field of the find line (values without spaces).
    pub fn find_field(&self, key: &str) -> Option<&str> {
        let line = self.find.as_deref()?;
        line.split(' ').find_map(|f| f.strip_prefix(key)?.strip_prefix('='))
    }

    /// The open palette list's highlighted row (`highlighted=N|none` on its
    /// header line): its index among the rows the dump lists, from 0, or
    /// `None` when nothing is highlighted.
    pub fn palette_highlighted(&self) -> Result<Option<usize>> {
        let header = self.palette.first().filter(|line| line.starts_with("list ")).ok_or("no palette list in the dump")?;
        // The query is quoted and may hold spaces, so the field is the last one.
        let field = header.rsplit_once(" highlighted=").ok_or_else(|| format!("no highlighted= on {header:?}"))?.1;
        match field {
            "none" => Ok(None),
            at => at.parse().map(Some).map_err(|_| format!("highlighted={at:?} on {header:?}")),
        }
    }

    pub fn grid(&self) -> Result<Grid> {
        self.grid.ok_or_else(|| "no grid line in the dump".to_string())
    }

    pub fn fonts(&self) -> Result<&Fonts> {
        self.fonts.as_ref().ok_or_else(|| "no fonts line in the dump".to_string())
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

/// When Brindle runs the last of `steps` steps spaced `interval` seconds
/// apart (already scaled), on its own clock: it waits half an interval,
/// then one interval before each step (`main.rs`'s step task).
fn last_step_secs(steps: usize, interval: f64) -> f64 {
    interval / 2.0 + steps as f64 * interval
}

pub fn self_test_dump_after_last_step() -> Result {
    ensure!(last_step_secs(0, 1.0) == 0.5, "no steps: {}", last_step_secs(0, 1.0));
    ensure!(last_step_secs(3, 1.0) == 3.5, "3 steps, 1 s apart: {}", last_step_secs(3, 1.0));
    // At `BRINDLE_E2E_SLOW=3` the interval is 3 s; the offset after it isn't
    // scaled.
    let dump = last_step_secs(4, 3.0) + 0.7;
    ensure!((dump - 14.2).abs() < 1e-9, "4 steps, 3 s apart, then 0.7 s: {dump}");

    // A run's own steps, at factor 3.
    let ctx = Ctx {
        name: "self-test",
        needs: &[],
        dir: std::env::temp_dir(),
        bin: std::path::PathBuf::new(),
        helper: std::path::PathBuf::new(),
        display: String::new(),
        server_dir: std::env::temp_dir(),
        fake_agent: std::path::PathBuf::new(),
        scale: crate::Scale::new(3.0),
    };
    let run = Run::new(&ctx, "self-test").action("new_tab").key("enter").dump_after_last_step(0.7);
    let dump = run.dump_secs();
    ensure!((dump - 8.2).abs() < 1e-9, "2 steps, 3 s apart, then 0.7 s: {dump}");
    let run = run.dump_at(4.0);
    ensure!(run.dump_secs() == 12.0, "dump_at(4) at factor 3: {}", run.dump_secs());
    // A fault switch the case didn't declare refuses the run before it launches.
    let refused = Run::new(&ctx, "self-test").env("BRINDLE_TEST_SINK_PANIC", "x").run();
    ensure!(
        refused.as_ref().err().is_some_and(|why| why.contains("BRINDLE_TEST_SINK_PANIC") && why.contains("needs")),
        "an undeclared switch gave {:?}",
        refused.map(|_| ())
    );
    Ok(())
}

pub fn self_test_list() -> Result {
    let listing = "1\t10\tattached\t-\t-\tkill\t1s\t-\tbash\t/\n2\t11\tdetached\towned\t3s\tdetach\t5s\t-\tsh\t/\n";
    ensure!(
        parse_listing(true, listing, "") == Ok(vec![("1".into(), true), ("2".into(), false)]),
        "listing {:?}",
        parse_listing(true, listing, "")
    );
    ensure!(parse_listing(true, "", "") == Ok(Vec::new()), "no core is no sessions");
    let failed = parse_listing(false, "", "can't read the server directory: Not a directory\n");
    ensure!(
        failed.as_ref().is_err_and(|why| why.contains("failed") && why.contains("can't read the server directory")),
        "a failed list is an error with its stderr: {failed:?}"
    );
    ensure!(panic_message(&*Box::new("boom") as &(dyn std::any::Any + Send)) == "boom", "a str payload");
    ensure!(panic_message(&*Box::new(String::from("bang")) as &(dyn std::any::Any + Send)) == "bang", "a String payload");
    ensure!(panic_message(&*Box::new(7_u8) as &(dyn std::any::Any + Send)).contains("isn't text"), "another payload");
    Ok(())
}

pub fn self_test_dump() -> Result {
    let dump = Dump::parse(
        "--- tab 1 \"bash\" (active)\n--- tab 2 \"vi\"\n--- screen 56x32 mode TermMode(SHOW_CURSOR)\n\
         --- grid origin=6,51.5 cell=8.4x18 scale=2\n--- fonts terminal=\"DejaVu Sans Mono\" ui=\".SystemUIFont\"\n\
         hello   \n\nworld\n\n\n",
    );
    let fonts = Fonts { terminal: "DejaVu Sans Mono".into(), ui: ".SystemUIFont".into() };
    ensure!(dump.fonts == Some(fonts), "fonts {:?}", dump.fonts);
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
    ensure!(dump.fonts.is_none(), "fonts {:?}", dump.fonts);
    // With no terminal, the fonts line follows the tabs.
    let dump = Dump::parse("--- tab 1 \"x\" (active)\n--- fonts terminal=\"Cascadia Mono\" ui=\"Segoe UI\"\n");
    ensure!(dump.fonts().map(|f| f.ui.as_str()) == Ok("Segoe UI"), "fonts without a terminal {:?}", dump.fonts);
    ensure!(dump.screen.is_empty(), "screen without a terminal {:?}", dump.screen);
    ensure!(dump.selection.is_none(), "selection {:?}", dump.selection);
    ensure!(dump.config.is_none(), "config {:?}", dump.config);
    // The config line follows the fonts line, and is not screen text.
    let dump = Dump::parse(
        "--- screen 2x1 mode TermMode(SHOW_CURSOR)\n--- fonts terminal=\"A\" ui=\"B\"\n\
         --- config \"Config: unknown theme \\\"x\\\", using brindle-dark\"\nhi\n",
    );
    ensure!(
        dump.config.as_deref() == Some("Config: unknown theme \"x\", using brindle-dark"),
        "config {:?}",
        dump.config
    );
    ensure!(dump.screen == ["hi"], "screen after config {:?}", dump.screen);
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
        "--- screen 2x1 mode TermMode(SHOW_CURSOR)\n--- progress tab=1 state=paused percent=70\n\
         --- progress zone=2 state=paused percent=70\n--- progress zone=1 state=normal percent=30\nhi\n",
    );
    ensure!(
        dump.progress
            == ["tab=1 state=paused percent=70", "zone=2 state=paused percent=70", "zone=1 state=normal percent=30"],
        "progress {:?}",
        dump.progress
    );
    ensure!(dump.screen == ["hi"], "screen after progress {:?}", dump.screen);
    let dump = Dump::parse(
        "--- screen 2x1 mode TermMode(SHOW_CURSOR)\n--- context id=\"s\" type=session\n\
         --- context id=\"e\" type=elevate label=\"root\"\n--- badge tab=1 kind=privileged label=\"root\" draw=icon\n\
         --- badge zone=1 kind=privileged label=\"root\" draw=icon\nhi\n",
    );
    ensure!(
        dump.contexts == ["id=\"s\" type=session", "id=\"e\" type=elevate label=\"root\""],
        "contexts {:?}",
        dump.contexts
    );
    ensure!(
        dump.badges == ["tab=1 kind=privileged label=\"root\" draw=icon", "zone=1 kind=privileged label=\"root\" draw=icon"],
        "badges {:?}",
        dump.badges
    );
    ensure!(dump.screen == ["hi"], "screen after contexts {:?}", dump.screen);
    ensure!(dump.pointer.is_none(), "no pointer line: {:?}", dump.pointer);
    ensure!(dump.feedback.is_none(), "no feedback line: {:?}", dump.feedback);
    for kind in ["copied", "pasted", "nothing-to-paste"] {
        let dump = Dump::parse(&format!(
            "--- screen 2x1 mode TermMode(SHOW_CURSOR)\n--- badge tab=1 kind=privileged label=\"root\" draw=icon\n\
             --- feedback kind={kind}\nhi\n"
        ));
        ensure!(dump.feedback.as_deref() == Some(kind), "feedback {:?}", dump.feedback);
        ensure!(dump.badges.len() == 1, "badges beside the feedback {:?}", dump.badges);
        ensure!(dump.screen == ["hi"], "screen after the feedback {:?}", dump.screen);
    }
    let dump = Dump::parse(
        "--- screen 2x1 mode TermMode(SHOW_CURSOR | ALT_SCREEN)\n--- pointer shape=crosshair screen=alternate\n\
         --- grid-pointer text\nhi\n",
    );
    ensure!(dump.pointer.as_deref() == Some("shape=crosshair screen=alternate"), "pointer {:?}", dump.pointer);
    ensure!(dump.grid_pointer.as_deref() == Some("text"), "grid pointer {:?}", dump.grid_pointer);
    ensure!(dump.screen == ["hi"], "screen after the pointer {:?}", dump.screen);
    // An agent zone's block, in place of a screen, before the palette and
    // focus lines.
    let dump = Dump::parse(
        "--- tab 1 \"Fake Agent\" (active)\n--- fonts terminal=\"A\" ui=\"B\"\n\
         --- agent zone=1 state=ready name=\"Fake Agent\" turn=ended:end_turn activity=done status=done link=up session=open \
         continue=none:not-ended\n\
         --- note \"gone\"\n--- entry 1 user \"hi there\"\n--- entry 2 agent \"Hello!\"\n\
         --- block entry=2 part=0 kind=paragraph \"Hello!\"\n\
         --- link entry=2 part=0 target=\"https://example.com\" text=\"Hello\" action=open\n--- queued waiting \"next\"\n\
         --- conversation entries=2 at=end\n--- prompt status=ready\n--- focus other modality=keyboard\n",
    );
    ensure!(dump.agent.len() == 9, "agent lines {:?}", dump.agent);
    ensure!(dump.agent_blocks(2) == ["part=0 kind=paragraph \"Hello!\""], "blocks {:?}", dump.agent_blocks(2));
    ensure!(dump.agent_blocks(1).is_empty() && dump.agent_links(1).is_empty(), "entry 1's {:?}", dump.agent);
    ensure!(
        dump.agent_links(2) == ["part=0 target=\"https://example.com\" text=\"Hello\" action=open"],
        "links {:?}",
        dump.agent_links(2)
    );
    ensure!(dump.link.is_none(), "an agent's link isn't the terminal's hovered one: {:?}", dump.link);
    ensure!(dump.agent_field("state") == Some("ready") && dump.agent_field("session") == Some("open"), "{:?}", dump.agent);
    ensure!(dump.agent_field("zone") == Some("1") && dump.agent_field("missing").is_none(), "{:?}", dump.agent);
    // The login row, after the agent line.
    let login = Dump::parse(
        "--- tab 1 \"Fake Agent\" (active)\n\
         --- agent zone=1 state=login-required turn=idle activity=error status=error link=up session=open\n\
         --- login choice method=\"google\" kind=agent \"Log in with Google\"\n\
         --- login choice method=\"cli\" kind=terminal \"CLI\" args=\"login\" env=\"\" dropped=\"PATH\"\n\
         --- conversation entries=0 at=end\n--- prompt status=disabled first=\"hi\"\n--- focus other modality=keyboard\n",
    );
    ensure!(
        login.agent_logins()
            == [
                "choice method=\"google\" kind=agent \"Log in with Google\"",
                "choice method=\"cli\" kind=terminal \"CLI\" args=\"login\" env=\"\" dropped=\"PATH\""
            ],
        "logins {:?}",
        login.agent
    );
    ensure!(login.agent.len() == 5, "agent lines with a login row {:?}", login.agent);
    ensure!(dump.agent_logins().is_empty(), "no login row {:?}", dump.agent);
    ensure!(dump.agent_field("continue") == Some("none:not-ended"), "{:?}", dump.agent);
    ensure!(dump.agent_entries() == ["user \"hi there\"", "agent \"Hello!\""], "entries {:?}", dump.agent_entries());
    ensure!(dump.size.is_none() && dump.screen.is_empty(), "no screen {:?}", dump.screen);
    ensure!(dump.focus.as_deref() == Some("other"), "focus after the agent block {:?}", dump.focus);
    ensure!(Dump::parse("--- palette prompt \"Name\" text=\"\"\n").agent.is_empty(), "a palette prompt isn't an agent line");
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
        "--- screen 2x1 mode TermMode(SHOW_CURSOR)\nhi\n--- palette list query=\"splt\"\nSplit Zone Right  [ctrl-shift-e]\n\
         --- input text=\"split\" cursor=4 selection=1..4\n",
    );
    ensure!(dump.screen == ["hi"], "screen before the palette {:?}", dump.screen);
    ensure!(dump.palette == ["list query=\"splt\"", "Split Zone Right  [ctrl-shift-e]"], "palette {:?}", dump.palette);
    let input = Input { text: "split".into(), cursor: 4, selection: (1, 4) };
    ensure!(dump.input == Some(input), "input {:?}", dump.input);
    ensure!(dump.palette_highlighted().is_err(), "a header with no highlighted= has none to read");
    let dump = Dump::parse(
        "--- palette list query=\"a highlighted=9\" highlighted=2\nbuild  /tmp · 5m ago\nlogs\nvim\n\
         --- input text=\"\" cursor=0 selection=0..0\n",
    );
    ensure!(dump.palette_highlighted() == Ok(Some(2)), "highlighted {:?}", dump.palette_highlighted());
    ensure!(dump.palette.len() == 4, "the rows after the header {:?}", dump.palette);
    let dump = Dump::parse("--- palette list query=\"\" highlighted=none\nA8  listing sessions…\n");
    ensure!(dump.palette_highlighted() == Ok(None), "highlighted {:?}", dump.palette_highlighted());
    ensure!(Dump::parse("--- palette prompt \"Name\" text=\"\"\n").palette_highlighted().is_err(), "a prompt has no list");
    ensure!(Dump::parse("nothing").palette_highlighted().is_err(), "no palette, no highlight");
    let line = "--- settings section=Profiles mode=editor name=\"Shell\" field=\"Theme\" theme=\"e2e-alt\" default=true invalid=[] can_save=true error=None\n";
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
    ensure!(dump.confirm.is_none(), "no confirmation {:?}", dump.confirm);
    // The close confirmation's line and its bare-word stop.
    let dump = Dump::parse(
        "--- tab 1 \"Fake Agent\" (active)\n--- prompt status=streaming\n\
         --- confirm close target=zone:1 busy=1:working\n--- focus confirm stop=cancel modality=keyboard\n",
    );
    ensure!(dump.confirm.as_deref() == Some("close target=zone:1 busy=1:working"), "confirm {:?}", dump.confirm);
    ensure!(dump.focus.as_deref() == Some("confirm") && dump.focus_stop.as_deref() == Some("cancel"), "{:?}", dump.focus_stop);
    let dump = Dump::parse("--- focus agent stop=permission:5:allow-once modality=keyboard\n");
    ensure!(dump.focus.as_deref() == Some("agent"), "focus {:?}", dump.focus);
    ensure!(dump.focus_stop.as_deref() == Some("permission:5:allow-once"), "stop {:?}", dump.focus_stop);
    let dump = Dump::parse(
        "--- screen 2x1 mode TermMode(SHOW_CURSOR)\nhi\n--- menu items=Shell|[Off]|Other highlighted=0\n\
         Shell  bash  [ctrl-alt-1]\nOff\nOther  sh · /tmp  [ctrl-alt-3]\n--- focus menu modality=keyboard\n",
    );
    ensure!(dump.screen == ["hi"], "screen before the menu {:?}", dump.screen);
    ensure!(
        dump.menu
            == [
                "items=Shell|[Off]|Other highlighted=0",
                "Shell  bash  [ctrl-alt-1]",
                "Off",
                "Other  sh · /tmp  [ctrl-alt-3]"
            ],
        "menu {:?}",
        dump.menu
    );
    ensure!(dump.palette.is_empty(), "palette {:?}", dump.palette);
    ensure!(dump.focus.as_deref() == Some("menu"), "focus after the menu {:?}", dump.focus);
    let dump = Dump::parse("--- screen 2x1 mode TermMode(SHOW_CURSOR)\nhi\n--- focus terminal modality=keyboard\n");
    ensure!(dump.wait_timeouts.is_empty() && dump.wait_pending.is_empty(), "waits from a dump without any");
    let dump = Dump::parse(
        "--- screen 2x1 mode TermMode(SHOW_CURSOR)\nhi\n--- focus terminal modality=keyboard\n\
         --- wait-for timeout \"NEVER\"\n--- wait-for pending \"a\\nb \\\"q\\\"\"\n--- wait-for timeout \"two\"\n",
    );
    ensure!(dump.wait_timeouts == ["NEVER", "two"], "timeouts {:?}", dump.wait_timeouts);
    ensure!(dump.wait_pending == ["a\nb \"q\""], "pending {:?}", dump.wait_pending);
    ensure!(dump.screen == ["hi"], "screen before the wait lines {:?}", dump.screen);
    let dump = Dump::parse("--- screen 2x1 mode TermMode(SHOW_CURSOR)\nhi\n--- wait-for pending \"x\"\nlate\n");
    ensure!(dump.screen == ["hi"], "a wait line ends the screen {:?}", dump.screen);
    Ok(())
}
