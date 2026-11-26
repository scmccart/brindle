//! User configuration: fonts, themes, profiles and keybindings.
//!
//! Loaded from `$XDG_CONFIG_HOME/brindle/config.toml`. A commented default
//! file is written on first launch.

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use crate::theme::{Theme, builtin_theme};

pub const DEFAULT_CONFIG: &str = include_str!("../assets/default-config.toml");

/// Something Brindle used in place of what the config file says, or ignored
/// in it, though the file is otherwise valid. The window's config line lists
/// each one, with what was used instead, until the file is fixed
/// (`railway R10`).
#[derive(Debug, Clone, PartialEq)]
pub enum ConfigFinding {
    /// A theme name that is neither built in nor under `[themes]`: the
    /// default theme is used. `profile` is the profile that names it, or
    /// `None` for the global `theme`.
    UnknownTheme { name: String, profile: Option<String> },
    /// A value outside its range, clamped to `used`.
    Clamped { key: &'static str, given: f32, used: f32 },
    /// A removed key the file still sets, which is ignored. `hint` says what
    /// to do instead.
    Removed { key: String, profile: Option<String>, hint: Option<&'static str> },
    /// The default file couldn't be written where the config belongs.
    DefaultNotWritten { path: PathBuf, error: String },
    /// A keybinding to a name that is no action: ignored.
    UnknownAction { keys: String, action: String },
    /// A keybinding whose keys can't be parsed: ignored.
    InvalidKeys { keys: String, error: String },
}

impl fmt::Display for ConfigFinding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownTheme { name, profile } => {
                write!(f, "unknown theme {name:?}")?;
                if let Some(profile) = profile {
                    write!(f, " (profile {profile})")?;
                }
                write!(f, ", using {DEFAULT_THEME}")
            }
            Self::Clamped { key, given, used } => write!(f, "{key} {given} is out of range, using {used}"),
            Self::Removed { key, profile, hint } => {
                if let Some(profile) = profile {
                    write!(f, "profile {profile:?}: ")?;
                }
                write!(f, "{key} is ignored")?;
                match hint {
                    Some(hint) => write!(f, ": {hint}"),
                    None => Ok(()),
                }
            }
            Self::DefaultNotWritten { path, error } => {
                write!(f, "the default config could not be written to {}: {error}", path.display())
            }
            Self::UnknownAction { keys, action } => {
                write!(f, "keybinding {keys:?} names an unknown action {action:?}, ignored")
            }
            Self::InvalidKeys { keys, error } => write!(f, "keybinding {keys:?} is invalid ({error}), ignored"),
        }
    }
}

/// The theme an unknown name falls back to.
const DEFAULT_THEME: &str = "brindle-dark";

/// A parsed config and what was used in place of the file's own values.
#[derive(Debug, Clone)]
pub struct Loaded {
    pub config: Config,
    pub findings: Vec<ConfigFinding>,
}

/// What loading the config left to say: the error that made the defaults
/// apply, and the findings of a file that did load (or of the defaults).
#[derive(Debug, Clone, Default)]
pub struct ConfigReport {
    pub error: Option<String>,
    pub findings: Vec<ConfigFinding>,
}

/// The text of the window's config line, or `None` when there is nothing to
/// say. An error gives the banner's text with the findings after it;
/// otherwise the findings follow `Config: `. `notices` are window notices
/// that aren't about the config (Brindle can't save its session, say), and
/// come last.
pub fn config_line(error: Option<&str>, findings: &[ConfigFinding], notices: &[String]) -> Option<String> {
    let found = findings.iter().map(ToString::to_string).collect::<Vec<_>>().join("; ");
    let mut line = match (error, found.is_empty()) {
        (Some(error), true) => format!("Config error (using defaults): {error}"),
        (Some(error), false) => format!("Config error (using defaults): {error}; {found}"),
        (None, false) => format!("Config: {found}"),
        (None, true) => String::new(),
    };
    for notice in notices {
        if !line.is_empty() {
            line.push_str("; ");
        }
        line.push_str(notice);
    }
    (!line.is_empty()).then_some(line)
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Name of the profile used for new tabs and windows.
    pub default_profile: Option<String>,
    pub font: FontConfig,
    /// The window chrome's font (tab strip, menus, dialogs).
    pub ui_font: UiFontConfig,
    /// Name of the default theme (built-in or defined under `[themes]`).
    pub theme: String,
    pub scrollback_lines: usize,
    /// Pixels of padding between the window edge and the terminal grid.
    pub padding: f32,
    pub cursor: CursorConfig,
    /// Copy selected text to the clipboard as soon as a selection is made.
    pub copy_on_select: bool,
    /// What a right-click does in a terminal.
    pub right_click: RightClick,
    /// Allow programs to read the clipboard via OSC 52 (writing is always allowed).
    pub osc52_paste: bool,
    /// The removed `restore_session` key, still accepted so old configs
    /// load: restoring is always on now. Parsing reports that it's ignored.
    #[serde(rename = "restore_session")]
    removed_restore_session: Option<serde::de::IgnoredAny>,
    /// Draw a red bar beside prompts whose command failed (needs shell
    /// integration; see `shell-integration/`).
    pub mark_failed_commands: bool,
    /// The removed `tmux_session_end` key, still accepted (whatever its
    /// value) so old configs load. Parsing reports that it's ignored.
    #[serde(rename = "tmux_session_end")]
    removed_tmux_session_end: Option<toml::Value>,
    /// Desktop notifications from bells, OSC 9/777, kitty's OSC 99 and
    /// program status records.
    pub notifications: NotificationConfig,
    /// Agent zones' settings (OpenSpec change `agents-tool-content`).
    pub agent: AgentConfig,
    pub profiles: Vec<Profile>,
    pub themes: BTreeMap<String, Theme>,
    /// Extra keybindings: `"ctrl-shift-x" = "action_name"`.
    pub keybindings: BTreeMap<String, String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            default_profile: None,
            font: FontConfig::default(),
            ui_font: UiFontConfig::default(),
            theme: "brindle-dark".into(),
            scrollback_lines: 10_000,
            padding: 6.0,
            cursor: CursorConfig::default(),
            copy_on_select: false,
            right_click: RightClick::Nothing,
            osc52_paste: false,
            removed_restore_session: None,
            mark_failed_commands: true,
            removed_tmux_session_end: None,
            notifications: NotificationConfig::default(),
            agent: AgentConfig::default(),
            profiles: Vec::new(),
            themes: BTreeMap::new(),
            keybindings: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct FontConfig {
    /// Font families in order of preference; the first one installed is used.
    pub family: Vec<String>,
    pub size: f32,
    /// Line height as a multiple of the font size.
    pub line_height: f32,
}

impl Default for FontConfig {
    fn default() -> Self {
        Self {
            family: vec![
                "JetBrains Mono".into(),
                "FiraCode Nerd Font Mono".into(),
                "Hack Nerd Font Mono".into(),
                "DejaVu Sans Mono".into(),
                "Liberation Mono".into(),
                "Noto Sans Mono".into(),
                "Ubuntu Mono".into(),
                "Cascadia Mono".into(),
                "Consolas".into(),
            ],
            size: 14.0,
            line_height: 1.25,
        }
    }
}

/// `[ui_font]`: the chrome's font. Only a family list, like `[font]`'s: the
/// first installed family wins, and an empty list (the default) leaves the
/// choice to Brindle's built-in list (`settings::resolve_ui_family`). It has
/// no size: the chrome's sizes are shadcn's, in rems.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct UiFontConfig {
    /// Font families in order of preference; the first one installed is used.
    pub family: Vec<String>,
}

/// What a right press does in a terminal when the program isn't using the
/// mouse (or shift is held).
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
pub enum RightClick {
    /// Nothing: the default.
    #[default]
    #[serde(rename = "none")]
    Nothing,
    /// Clear any selection and paste the clipboard; with no text on the
    /// clipboard, nothing is pasted and the selection stays.
    #[serde(rename = "paste")]
    Paste,
}

impl RightClick {
    /// `right_click`'s value for it, as the config file spells it.
    pub fn word(self) -> &'static str {
        match self {
            RightClick::Nothing => "none",
            RightClick::Paste => "paste",
        }
    }

    /// The value `word` names, read as a load reads `right_click`, or
    /// `None` for a word it refuses.
    pub fn from_word(word: &str) -> Option<Self> {
        use serde::de::IntoDeserializer as _;
        let word: serde::de::value::StrDeserializer<'_, serde::de::value::Error> = word.into_deserializer();
        Self::deserialize(word).ok()
    }
}

#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum CursorShapeConfig {
    #[default]
    Block,
    Beam,
    Underline,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CursorConfig {
    pub shape: CursorShapeConfig,
    pub blink: bool,
}

impl Default for CursorConfig {
    fn default() -> Self {
        Self { shape: CursorShapeConfig::Block, blink: true }
    }
}

/// When a notification is shown for a tab.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum NotifyWhen {
    /// Only when the tab isn't in front of the user: it is a background
    /// tab, or its window isn't focused.
    #[default]
    Unfocused,
    /// Also for the tab the user is looking at.
    Always,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct NotificationConfig {
    /// A bell (BEL) raises a notification.
    pub bell: bool,
    /// `OSC 9 ; message`.
    pub osc9: bool,
    /// `OSC 777 ; notify ; title ; body`.
    pub osc777: bool,
    /// kitty's `OSC 99`: notifications, their updates and closes, and the
    /// replies to its queries (off, Brindle answers none of them).
    pub osc99: bool,
    /// A program status record (OSC 7501) that becomes blocked, done or
    /// error.
    pub osc7501: bool,
    /// An agent zone whose status becomes blocked, done or error.
    pub agent: bool,
    pub when: NotifyWhen,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self {
            bell: true,
            osc9: true,
            osc777: true,
            osc99: true,
            osc7501: true,
            agent: true,
            when: NotifyWhen::Unfocused,
        }
    }
}

/// Whether a tool call row in an agent zone is open on its own (`[agent]`'s
/// `tool_details` and `tool_details_when_done`, OpenSpec change
/// `agents-tool-content` decision 10).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shown {
    Open,
    Closed,
}

impl Shown {
    /// The value `key` holds, or an error naming the key and both values
    /// (the config line's text for a bad one: TOML's own names neither).
    fn read<'de, D: serde::Deserializer<'de>>(key: &str, deserializer: D) -> Result<Self, D::Error> {
        let value = toml::Value::deserialize(deserializer)?;
        match value.as_str() {
            Some("open") => Ok(Self::Open),
            Some("closed") => Ok(Self::Closed),
            Some(_) | None => Err(serde::de::Error::custom(format!(
                "agent.{key} is `open` or `closed`, not {value}"
            ))),
        }
    }
}

fn tool_details<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Shown, D::Error> {
    Shown::read("tool_details", deserializer)
}

fn tool_details_when_done<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Shown, D::Error> {
    Shown::read("tool_details_when_done", deserializer)
}

/// `[agent]`: agent zones' settings, global to every agent profile.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct AgentConfig {
    /// A running tool call's row.
    #[serde(deserialize_with = "tool_details")]
    pub tool_details: Shown,
    /// A finished tool call's row.
    #[serde(deserialize_with = "tool_details_when_done")]
    pub tool_details_when_done: Shown,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self { tool_details: Shown::Open, tool_details_when_done: Shown::Closed }
    }
}

/// A remote Linux host or container endpoint.
///
/// `connect_at_launch` (either kind, default false) makes Brindle connect to
/// the endpoint at launch and keep that connection for as long as it runs.
/// It isn't part of the endpoint: [`RemoteTarget::endpoint`] drops it, and it
/// is serialized only when set, so the session state never records it.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum RemoteTarget {
    Ssh {
        host: String,
        #[serde(default)]
        user: Option<String>,
        #[serde(default)]
        port: Option<u16>,
        #[serde(default)]
        identity_file: Option<String>,
        #[serde(default)]
        ssh_args: Vec<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        connect_at_launch: bool,
    },
    Container {
        #[serde(default = "default_container_engine")]
        engine: String,
        container: String,
        #[serde(default)]
        user: Option<String>,
        #[serde(default, skip_serializing_if = "is_false")]
        connect_at_launch: bool,
    },
}

impl RemoteTarget {
    /// Whether Brindle connects to the endpoint at launch and keeps the
    /// connection (`remote-connections`' "Connecting at launch").
    pub fn connects_at_launch(&self) -> bool {
        match self {
            Self::Ssh { connect_at_launch, .. } | Self::Container { connect_at_launch, .. } => *connect_at_launch,
        }
    }

    /// The target as an endpoint, without `connect_at_launch`: what a tab
    /// on it keeps and the session state records.
    pub fn endpoint(&self) -> Self {
        let mut target = self.clone();
        match &mut target {
            Self::Ssh { connect_at_launch, .. } | Self::Container { connect_at_launch, .. } => *connect_at_launch = false,
        }
        target
    }
}

fn default_container_engine() -> String {
    "docker".into()
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// A profile's identity across a reload, where its index may name another
/// profile: profiles have no ids and their names may repeat, so it is the
/// `nth` (from 0) of the profiles named `name`, names matching ignoring case
/// as profile lookups do ([`Config::profile_key`], [`Config::find_profile`]).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProfileKey {
    pub name: String,
    pub nth: usize,
}

impl ProfileKey {
    /// The key as [`fmt::Display`] writes it (`1:Shell`), which a menu
    /// item's key carries; `None` for any other text.
    pub fn parse(text: &str) -> Option<Self> {
        let (nth, name) = text.split_once(':')?;
        Some(Self { name: name.to_string(), nth: nth.parse().ok()? })
    }

    /// The key of the `ix`th of `names`, a profile list in order: its name,
    /// and how many names before it match it. `None` past the last.
    pub fn of<'a>(names: impl IntoIterator<Item = &'a str> + Clone, ix: usize) -> Option<Self> {
        let name = names.clone().into_iter().nth(ix)?;
        let nth = names.into_iter().take(ix).filter(|n| n.eq_ignore_ascii_case(name)).count();
        Some(Self { name: name.to_string(), nth })
    }

    /// Where the profile this key names is among `names`: the key's `nth`
    /// of the names matching its own. `None` when there is none.
    pub fn find<'a>(&self, names: impl IntoIterator<Item = &'a str>) -> Option<usize> {
        names
            .into_iter()
            .enumerate()
            .filter(|(_, n)| n.eq_ignore_ascii_case(&self.name))
            .nth(self.nth)
            .map(|(ix, _)| ix)
    }
}

impl fmt::Display for ProfileKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.nth, self.name)
    }
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct Profile {
    /// What the profile opens (`type`, lowercase; `terminal` when absent).
    /// Read through [`Profile::kind`]; an agent profile always has a
    /// command ([`Config::parse`] refuses one without).
    /// `pub(crate)` only so other modules can still build a profile with
    /// `..Default::default()`; nothing outside this module sets it.
    #[serde(rename = "type")]
    pub(crate) kind: ProfileKind,
    #[serde(default)]
    pub remote: Option<RemoteTarget>,
    pub name: String,
    /// Program to run. Defaults to `$SHELL`, then `/bin/sh`; on Windows to
    /// `pwsh.exe` on `PATH`, then `powershell.exe`, then `%COMSPEC%`, then
    /// `cmd.exe`.
    pub command: Option<String>,
    pub args: Vec<String>,
    /// Working directory. `~` is expanded. Defaults to the home directory.
    pub cwd: Option<String>,
    pub env: BTreeMap<String, String>,
    /// Theme override for tabs opened with this profile.
    pub theme: Option<String>,
    /// Font size override.
    pub font_size: Option<f32>,
    /// The removed `tmux`, `tmux_session` and `tmux_args` keys, still
    /// accepted (whatever their values) so old configs load. Parsing reports
    /// that each is ignored.
    #[serde(rename = "tmux")]
    pub(crate) removed_tmux: Option<toml::Value>,
    #[serde(rename = "tmux_session")]
    pub(crate) removed_tmux_session: Option<toml::Value>,
    #[serde(rename = "tmux_args")]
    pub(crate) removed_tmux_args: Option<toml::Value>,
}

/// What kind of tab a profile opens: its `type` key.
#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ProfileKind {
    /// A terminal tab, running the profile's program.
    #[default]
    Terminal,
    /// An agent zone, running the profile's command as an ACP agent in the
    /// terminal server.
    Agent,
}

impl Profile {
    pub fn default_shell() -> Self {
        Self { name: "Shell".into(), ..Default::default() }
    }

    /// The kind of tab the profile opens, from its `type` key.
    pub fn kind(&self) -> ProfileKind {
        self.kind
    }

    /// Whether the profile names what its kind needs to run: a terminal
    /// profile always does (no command runs the default shell); an agent
    /// profile only with a command that isn't blank.
    fn runnable(&self) -> bool {
        match self.kind {
            ProfileKind::Terminal => true,
            ProfileKind::Agent => self.command.as_deref().is_some_and(|command| !command.trim().is_empty()),
        }
    }

    /// A short description of what the profile runs:
    /// `[<target> · ]<command>` for a terminal profile (`default shell`
    /// without one), `[<target> · ]agent · <command>` for an agent profile.
    pub fn description(&self) -> String {
        let target_prefix = match &self.remote {
            Some(RemoteTarget::Ssh { host, .. }) => format!("ssh:{host} · "),
            Some(RemoteTarget::Container { container, .. }) => format!("container:{container} · "),
            None => String::new(),
        };
        let base = match self.kind {
            ProfileKind::Terminal => self.command.clone().unwrap_or_else(|| "default shell".into()),
            ProfileKind::Agent => format!("agent · {}", self.command.as_deref().unwrap_or_default()),
        };
        format!("{target_prefix}{base}")
    }

    /// The profile's `cwd`, with a leading `~` expanded to `home` for a local
    /// profile. A remote profile's is the endpoint's to expand.
    pub fn working_directory(&self, home: Option<&Path>) -> Option<PathBuf> {
        self.cwd.as_deref().map(|path| {
            if self.remote.is_some() { PathBuf::from(path) } else { expand_tilde(path, home) }
        })
    }
}

/// `path` with a leading `~` or `~/` replaced by `home`; as written when
/// there is no home directory.
pub fn expand_tilde(path: &str, home: Option<&Path>) -> PathBuf {
    if path == "~" {
        return home.map_or_else(|| PathBuf::from(path), Path::to_path_buf);
    }
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = home
    {
        return home.join(rest);
    }
    PathBuf::from(path)
}

impl Config {
    pub fn path() -> PathBuf {
        if let Ok(path) = std::env::var("BRINDLE_CONFIG") {
            return PathBuf::from(path);
        }
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("brindle")
            .join("config.toml")
    }

    /// Loads the config, writing the default file if none exists. Errors in
    /// the file are reported and the defaults are used instead, so a typo
    /// never prevents the terminal from starting. The report is what the
    /// window's config line says (`configuration`).
    pub fn load_or_default() -> (Self, ConfigReport) {
        Self::load_or_default_at(&Self::path())
    }

    /// [`Config::load_or_default`] for the file at `path`.
    fn load_or_default_at(path: &Path) -> (Self, ConfigReport) {
        let mut findings = Vec::new();
        if !path.exists()
            && let Err(err) = write_default(path)
        {
            findings.push(ConfigFinding::DefaultNotWritten { path: path.to_path_buf(), error: format!("{err:#}") });
        }
        match Self::load(path) {
            Ok(Loaded { config, findings: found }) => {
                findings.extend(found);
                findings.extend(config.findings());
                (config, ConfigReport { error: None, findings })
            }
            Err(err) => {
                let message = format!("{err:#}");
                log::error!("{message}");
                let (config, clamped) = Self::default().normalized();
                findings.extend(clamped);
                (config, ConfigReport { error: Some(message), findings })
            }
        }
    }

    pub fn load(path: &Path) -> Result<Loaded> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(err) => return Err(err).context(format!("reading {}", path.display())),
        };
        Self::parse(&text).with_context(|| format!("invalid config {}", path.display()))
    }

    /// Parses `text`: the config, with what was clamped and the removed keys
    /// it still sets as findings. Unknown themes are [`Config::findings`].
    /// An agent profile without a command (absent, or only blanks) is an
    /// error naming the profile, as an invalid value is.
    pub fn parse(text: &str) -> Result<Loaded> {
        let config: Self = toml::from_str(text)?;
        let mut findings = config.removed_findings();
        let (config, clamped) = config.normalized();
        if let Some(profile) = config.profiles.iter().find(|profile| !profile.runnable()) {
            bail!("profile {:?}: an agent profile needs a command", profile.name);
        }
        findings.extend(clamped);
        Ok(Loaded { config, findings })
    }

    /// One finding for each removed key the file still sets: `restore_session`
    /// and `tmux_session_end` at the top, and each profile's `tmux`,
    /// `tmux_session` and `tmux_args` (naming the profile).
    fn removed_findings(&self) -> Vec<ConfigFinding> {
        const GONE: &str = "Brindle no longer has tmux support";
        const TMUX: &str = "Brindle no longer has tmux support; to run tmux in its tabs, set command = \"tmux\" and args = [\"new-session\", \"-A\", \"-s\", \"main\"]";
        let mut found = Vec::new();
        if self.removed_restore_session.is_some() {
            found.push(ConfigFinding::Removed {
                key: "restore_session".into(),
                profile: None,
                hint: Some("it is no longer a setting: Brindle always restores (--no-restore skips it once)"),
            });
        }
        if self.removed_tmux_session_end.is_some() {
            found.push(ConfigFinding::Removed { key: "tmux_session_end".into(), profile: None, hint: Some(GONE) });
        }
        for profile in &self.profiles {
            let keys = [
                ("tmux", &profile.removed_tmux, TMUX),
                ("tmux_session", &profile.removed_tmux_session, GONE),
                ("tmux_args", &profile.removed_tmux_args, GONE),
            ];
            for (key, value, hint) in keys {
                if value.is_some() {
                    found.push(ConfigFinding::Removed {
                        key: key.into(),
                        profile: Some(profile.name.clone()),
                        hint: Some(hint),
                    });
                }
            }
        }
        found
    }

    /// The unknown themes the config names: the global one, and each
    /// profile's. Their tabs use the default theme ([`Config::theme`]).
    pub fn findings(&self) -> Vec<ConfigFinding> {
        let known = |name: &str| self.themes.contains_key(name) || builtin_theme(name).is_some();
        let mut found = Vec::new();
        if !known(&self.theme) {
            found.push(ConfigFinding::UnknownTheme { name: self.theme.clone(), profile: None });
        }
        for profile in &self.profiles {
            if let Some(name) = profile.theme.as_deref().filter(|name| !known(name)) {
                found.push(ConfigFinding::UnknownTheme { name: name.into(), profile: Some(profile.name.clone()) });
            }
        }
        found
    }

    /// Ensures there is at least one profile and that names are usable, and
    /// clamps the font's size and line height, returning each clamp.
    fn normalized(mut self) -> (Self, Vec<ConfigFinding>) {
        if self.profiles.is_empty() {
            self.profiles.push(Profile::default_shell());
        }
        for (ix, profile) in self.profiles.iter_mut().enumerate() {
            if profile.name.trim().is_empty() {
                profile.name = format!("Profile {}", ix + 1);
            }
        }
        let mut clamped = Vec::new();
        let size = self.font.size.clamp(4.0, 96.0);
        if size != self.font.size {
            clamped.push(ConfigFinding::Clamped { key: "font.size", given: self.font.size, used: size });
        }
        let line_height = self.font.line_height.clamp(0.8, 3.0);
        if line_height != self.font.line_height {
            clamped.push(ConfigFinding::Clamped { key: "font.line_height", given: self.font.line_height, used: line_height });
        }
        self.font.size = size;
        self.font.line_height = line_height;
        (self, clamped)
    }

    pub fn default_profile_index(&self) -> usize {
        self.default_profile
            .as_ref()
            .and_then(|name| self.profile_index(name))
            .unwrap_or(0)
    }

    pub fn profile_index(&self, name: &str) -> Option<usize> {
        self.profiles.iter().position(|p| p.name.eq_ignore_ascii_case(name))
    }

    pub fn profile(&self, name: &str) -> Option<&Profile> {
        self.profile_index(name).map(|ix| &self.profiles[ix])
    }

    /// The key of profile `ix`: its name, and how many profiles before it
    /// have that name. `None` past the last profile.
    pub fn profile_key(&self, ix: usize) -> Option<ProfileKey> {
        ProfileKey::of(self.profiles.iter().map(|p| p.name.as_str()), ix)
    }

    /// Every profile's key, in configuration order.
    pub fn profile_keys(&self) -> Vec<ProfileKey> {
        (0..self.profiles.len()).filter_map(|ix| self.profile_key(ix)).collect()
    }

    /// The index of the profile `key` names in this config: the key's
    /// `nth` profile of its name. `None` when the config has none (it was
    /// renamed or removed).
    pub fn find_profile(&self, key: &ProfileKey) -> Option<usize> {
        key.find(self.profiles.iter().map(|p| p.name.as_str()))
    }

    pub fn profile_theme(&self, profile: &Profile) -> Theme {
        self.theme(profile.theme.as_deref())
    }

    /// The theme `name` (or the global one) names. Every name it is given
    /// comes from the config, whose unknown names are
    /// [`Config::findings`], so the default theme it falls back to is one
    /// the window reports.
    pub fn theme(&self, name: Option<&str>) -> Theme {
        let name = name.unwrap_or(&self.theme);
        if let Some(theme) = self.themes.get(name) {
            return theme.clone();
        }
        builtin_theme(name).unwrap_or_default()
    }
}

fn write_default(path: &Path) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }
    write_replacing(path, DEFAULT_CONFIG)?;
    Ok(())
}

/// Writes `text` to `path` in one step, so a reader (another Brindle, a
/// reload) finds the old file or the new one, never part of it: into
/// `<name>.<pid>.tmp` beside it (the pid keeps two processes' writes apart),
/// synced to disk, then renamed over it. The temp file is removed when a
/// step fails. `path` is replaced, not written through: a caller that keeps
/// a symlink resolves it first.
pub fn write_replacing(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write as _;

    let Some(name) = path.file_name() else {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, format!("{} names no file", path.display())));
    };
    let mut temp_name = name.to_os_string();
    temp_name.push(format!(".{}.tmp", std::process::id()));
    let temp = path.with_file_name(temp_name);
    let written = std::fs::File::create(&temp).and_then(|mut file| {
        file.write_all(text.as_bytes())?;
        file.sync_all()
    });
    let replaced = written.and_then(|()| std::fs::rename(&temp, path));
    if replaced.is_err() {
        std::fs::remove_file(&temp).ok();
    }
    replaced
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An empty directory of this test's own.
    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("brindle-config-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// The config `text` parses to, without its findings.
    fn parsed(text: &str) -> Result<Config> {
        Config::parse(text).map(|loaded| loaded.config)
    }

    /// The findings of parsing `text`, then of its config's themes.
    fn findings_of(text: &str) -> Vec<ConfigFinding> {
        let loaded = Config::parse(text).unwrap();
        let mut findings = loaded.findings;
        findings.extend(loaded.config.findings());
        findings
    }

    /// The names of the files in `dir`, sorted.
    fn files_in(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> =
            std::fs::read_dir(dir).unwrap().map(|entry| entry.unwrap().file_name().to_string_lossy().into()).collect();
        names.sort();
        names
    }

    #[test]
    fn a_replaced_file_has_the_new_text_and_no_temp_file_is_left() {
        let dir = temp_dir("replace");
        let path = dir.join("config.toml");
        write_replacing(&path, "theme = \"one\"\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "theme = \"one\"\n");
        write_replacing(&path, "theme = \"two\"\n").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "theme = \"two\"\n");
        assert_eq!(files_in(&dir), ["config.toml"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A rename that fails (the target is a directory) leaves the target as
    /// it was and no temp file.
    #[test]
    fn a_failed_replace_leaves_no_temp_file() {
        let dir = temp_dir("replace-fails");
        let path = dir.join("config.toml");
        std::fs::create_dir(&path).unwrap();
        assert!(write_replacing(&path, "theme = \"one\"\n").is_err());
        assert!(path.is_dir());
        assert_eq!(files_in(&dir), ["config.toml"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn write_default_creates_the_directory_and_the_default_config() {
        let dir = temp_dir("default");
        let path = dir.join("brindle").join("config.toml");
        write_default(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), DEFAULT_CONFIG);
        assert_eq!(files_in(&dir.join("brindle")), ["config.toml"]);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn default_config_file_parses() {
        let config = parsed(DEFAULT_CONFIG).unwrap();
        assert!(!config.profiles.is_empty());
        assert_eq!(config.profiles[config.default_profile_index()].name, "Shell");
    }

    #[test]
    fn ui_font_takes_a_family_list_and_nothing_else() {
        let config = parsed("[ui_font]\nfamily = [\"Inter\", \"Segoe UI\"]").unwrap();
        assert_eq!(config.ui_font.family, ["Inter", "Segoe UI"]);
        let err = parsed("[ui_font]\nsize = 12").unwrap_err();
        assert!(format!("{err:#}").contains("size"), "{err:#}");
        // Without the table the list is empty, so the built-in list applies.
        assert!(parsed("").unwrap().ui_font.family.is_empty());
        assert!(parsed(DEFAULT_CONFIG).unwrap().ui_font.family.is_empty());
    }

    #[test]
    fn empty_config_gets_a_shell_profile() {
        let config = parsed("").unwrap();
        assert_eq!(config.profiles.len(), 1);
        assert_eq!(config.profiles[0].name, "Shell");
        assert_eq!(config.theme, "brindle-dark");
    }

    /// Profiles named alike are told apart by how many came before, names
    /// matching ignoring case; a key finds its profile again after another
    /// is added before it under another name, and nothing once it is gone.
    #[test]
    fn profile_keys_tell_duplicate_names_apart() {
        let names = |names: &[&str]| {
            let profiles = names.iter().map(|name| Profile { name: (*name).into(), ..Default::default() }).collect();
            Config { profiles, ..Config::default() }
        };
        let key = |name: &str, nth| ProfileKey { name: name.into(), nth };
        let config = names(&["Shell", "Work", "shell", "Shell"]);
        assert_eq!(config.profile_keys(), [key("Shell", 0), key("Work", 0), key("shell", 1), key("Shell", 2)]);
        assert_eq!(config.profile_key(4), None);
        for (ix, key) in config.profile_keys().iter().enumerate() {
            assert_eq!(config.find_profile(key), Some(ix), "{key}");
        }
        assert_eq!(config.find_profile(&key("SHELL", 1)), Some(2), "ignoring case");
        assert_eq!(config.find_profile(&key("Shell", 3)), None);
        assert_eq!(config.find_profile(&key("Logs", 0)), None);

        // A reload: a profile is added first, and the second Shell is removed.
        let reloaded = names(&["Logs", "Shell", "Work", "Shell"]);
        assert_eq!(reloaded.find_profile(&key("Work", 0)), Some(2), "found by name, not place");
        assert_eq!(reloaded.find_profile(&key("Shell", 0)), Some(1));
        assert_eq!(reloaded.find_profile(&key("shell", 1)), Some(3));
        assert_eq!(reloaded.find_profile(&key("Shell", 2)), None, "gone");
    }

    /// A key as a menu item carries it, and back.
    #[test]
    fn a_profile_key_reads_back_its_text() {
        for key in [
            ProfileKey { name: "Shell".into(), nth: 0 },
            ProfileKey { name: "logs: prod".into(), nth: 2 },
            ProfileKey { name: String::new(), nth: 1 },
        ] {
            assert_eq!(ProfileKey::parse(&key.to_string()), Some(key.clone()), "{key}");
        }
        assert_eq!(ProfileKey { name: "Work".into(), nth: 1 }.to_string(), "1:Work");
        for text in ["Shell", "x:Shell", "-1:Shell", ""] {
            assert_eq!(ProfileKey::parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn profiles_and_overrides() {
        let config = parsed(
            r##"
            default_profile = "work"
            theme = "mine"

            [font]
            family = ["Iosevka"]
            size = 200

            [[profiles]]
            name = "Shell"

            [[profiles]]
            name = "Work"
            command = "/bin/zsh"
            args = ["-l"]
            cwd = "~/src"
            env = { FOO = "bar" }

            [themes.mine]
            foreground = "#ffffff"
            background = "#000000"
            "##,
        )
        .unwrap();
        assert_eq!(config.default_profile_index(), 1);
        let work = &config.profiles[1];
        assert_eq!(work.env.get("FOO").map(String::as_str), Some("bar"));
        assert_eq!(work.working_directory(Some(Path::new("/home/u"))), Some(PathBuf::from("/home/u/src")));
        assert_eq!(config.font.size, 96.0);
        let theme = config.theme(None);
        assert_eq!(theme.background.to_string(), "#000000");
    }

    #[test]
    fn a_tilde_without_a_home_stays_as_written() {
        let home = Some(Path::new("/home/u"));
        assert_eq!(expand_tilde("~", home), PathBuf::from("/home/u"));
        assert_eq!(expand_tilde("~/src", home), PathBuf::from("/home/u/src"));
        assert_eq!(expand_tilde("~", None), PathBuf::from("~"));
        assert_eq!(expand_tilde("~/src", None), PathBuf::from("~/src"));
        // Only a leading `~` or `~/` is the home directory.
        for as_written in ["~other/src", "/srv/~/x", "src"] {
            assert_eq!(expand_tilde(as_written, home), PathBuf::from(as_written));
        }
    }

    #[test]
    fn remote_profiles() {
        let config = parsed(
            r##"
            [[profiles]]
            name = "A8"
            remote = { type = "ssh", host = "A8", user = "ubuntu", port = 2222, ssh_args = ["-v"] }
            cwd = "~/src"

            [[profiles]]
            name = "DevContainer"
            remote = { type = "container", engine = "docker", container = "rust-box" }
            "##,
        )
        .unwrap();
        assert_eq!(config.profiles.len(), 2);
        match &config.profiles[0].remote {
            Some(RemoteTarget::Ssh { host, user, port, identity_file, ssh_args, connect_at_launch }) => {
                assert_eq!(host, "A8");
                assert_eq!(user.as_deref(), Some("ubuntu"));
                assert_eq!(*port, Some(2222));
                assert_eq!(identity_file, &None);
                assert_eq!(ssh_args, &["-v"]);
                assert!(!connect_at_launch);
            }
            _ => panic!("expected ssh remote"),
        }
        match &config.profiles[1].remote {
            Some(RemoteTarget::Container { engine, container, user, connect_at_launch }) => {
                assert_eq!(engine, "docker");
                assert_eq!(container, "rust-box");
                assert_eq!(user, &None);
                assert!(!connect_at_launch);
            }
            _ => panic!("expected container remote"),
        }
        assert_eq!(config.profiles[0].description(), "ssh:A8 · default shell");
        assert_eq!(config.profiles[1].description(), "container:rust-box · default shell");
        assert_eq!(config.profiles[0].working_directory(Some(Path::new("/home/u"))), Some(PathBuf::from("~/src")));
        assert!(parsed(
            "[[profiles]]\nname = \"bad\"\nremote = { type = \"ssh\", host = \"A8\", password = \"secret\" }"
        )
        .is_err());
    }

    /// 8.1: either kind takes `connect_at_launch`, false when absent; it
    /// isn't part of the endpoint.
    #[test]
    fn remote_targets_may_connect_at_launch() {
        let config = parsed(
            r##"
            [[profiles]]
            name = "A8"
            remote = { type = "ssh", host = "A8", connect_at_launch = true }

            [[profiles]]
            name = "devbox"
            remote = { type = "container", container = "devbox", connect_at_launch = true }

            [[profiles]]
            name = "B9"
            remote = { type = "ssh", host = "B9", connect_at_launch = false }

            [[profiles]]
            name = "box"
            remote = { type = "container", container = "box" }
            "##,
        )
        .unwrap();
        let eager: Vec<bool> =
            config.profiles.iter().map(|p| p.remote.as_ref().is_some_and(RemoteTarget::connects_at_launch)).collect();
        assert_eq!(eager, [true, true, false, false]);
        for profile in &config.profiles {
            let target = profile.remote.as_ref().unwrap();
            assert!(!target.endpoint().connects_at_launch(), "{}", profile.name);
        }
        let ssh = config.profiles[0].remote.as_ref().unwrap();
        assert_eq!(
            ssh.endpoint(),
            RemoteTarget::Ssh {
                host: "A8".into(),
                user: None,
                port: None,
                identity_file: None,
                ssh_args: Vec::new(),
                connect_at_launch: false,
            }
        );
        assert!(parsed(
            "[[profiles]]\nname = \"bad\"\nremote = { type = \"container\", container = \"c\", connect_at_launch = 1 }"
        )
        .is_err());
    }

    /// agents-profiles 6.10: the README's agent profile example loads as
    /// the agent profile it describes.
    #[test]
    fn the_readme_agent_example_parses() {
        let readme = include_str!("../README.md");
        let start = readme.find("\n### Agent tabs\n").expect("the agent section");
        let section = &readme[start..];
        let block = &section[section.find("```toml\n").expect("its example") + 8..];
        let toml = &block[..block.find("```").expect("the example's end")];
        let config = parsed(toml).unwrap();
        let [profile] = &config.profiles[..] else { panic!("one profile: {:?}", config.profiles) };
        assert_eq!(profile.kind(), ProfileKind::Agent);
        assert_eq!((profile.name.as_str(), profile.command.as_deref()), ("Agent", Some("my-acp-agent")));
        assert_eq!(profile.args, ["--acp"]);
        assert_eq!(profile.description(), "agent · my-acp-agent");
    }

    /// agents-profiles 7.2: the README's `[notifications]` example names
    /// every source's key, each at its default, and parses as the defaults.
    /// The destructuring fails to compile when a key is added, so the
    /// README is updated with it.
    #[test]
    fn the_readme_notifications_example_names_every_key() {
        let readme = include_str!("../README.md");
        let start = readme.find("```toml\n[notifications]\n").expect("the notifications example") + 8;
        let toml = &readme[start..start + readme[start..].find("```").expect("the example's end")];
        let config = parsed(toml).unwrap();
        assert_eq!(config.notifications, NotificationConfig::default());
        let NotificationConfig { bell: _, osc9: _, osc777: _, osc99: _, osc7501: _, agent: _, when: _ } = config.notifications;
        let keys: Vec<&str> = toml.lines().skip(1).filter_map(|line| line.split_once(" =").map(|(key, _)| key)).collect();
        assert_eq!(keys, ["bell", "osc9", "osc777", "osc99", "osc7501", "agent", "when"]);
    }

    /// 8.5: the config examples in the README's remote section, together,
    /// load as the profiles they describe.
    #[test]
    fn the_readme_remote_examples_parse() {
        let readme = include_str!("../README.md");
        let start = readme.find("\n## Remote hosts and containers\n").expect("the remote section");
        let section = &readme[start + 1..];
        let section = &section[..section.find("\n### Terminal server\n").expect("the section's end")];
        let mut toml = String::new();
        let mut in_block = false;
        for line in section.lines() {
            match (in_block, line) {
                (false, "```toml") => in_block = true,
                (true, "```") => in_block = false,
                (true, line) => {
                    toml.push_str(line);
                    toml.push('\n');
                }
                _ => {}
            }
        }
        let config = parsed(&toml).unwrap();
        let remotes: Vec<(&str, &str, bool)> = config
            .profiles
            .iter()
            .map(|p| {
                let target = p.remote.as_ref().unwrap_or_else(|| panic!("{} has no remote", p.name));
                let kind = match target {
                    RemoteTarget::Ssh { .. } => "ssh",
                    RemoteTarget::Container { .. } => "container",
                };
                (p.name.as_str(), kind, target.connects_at_launch())
            })
            .collect();
        assert_eq!(
            remotes,
            [
                ("A8", "ssh", false),
                ("A8 dev", "ssh", false),
                ("dev container", "container", false),
                ("dev container (podman)", "container", false),
                ("build host", "ssh", true),
                ("dev container (eager)", "container", true),
            ]
        );
    }

    #[test]
    fn the_removed_restore_session_key_still_loads() {
        assert!(findings_of("").is_empty());
        assert!(findings_of(DEFAULT_CONFIG).is_empty());
        assert!(!DEFAULT_CONFIG.contains("restore_session"));
        // Any old value loads, with no error for the banner, and a finding.
        for old in ["restore_session = true", "restore_session = false"] {
            let findings = findings_of(old);
            assert!(
                matches!(findings.as_slice(), [ConfigFinding::Removed { key, profile: None, .. }] if key == "restore_session"),
                "{old}: {findings:?}"
            );
        }
    }

    /// 1.1: `type` picks the profile's kind, `terminal` when absent.
    #[test]
    fn a_profile_type_picks_its_kind() {
        let config = parsed(
            r#"
            [[profiles]]
            name = "Shell"

            [[profiles]]
            name = "Fake"
            type = "agent"
            command = "agent-acp"
            args = ["--stdio"]

            [[profiles]]
            name = "Plain"
            type = "terminal"
            "#,
        )
        .unwrap();
        let kinds: Vec<ProfileKind> = config.profiles.iter().map(Profile::kind).collect();
        assert_eq!(kinds, [ProfileKind::Terminal, ProfileKind::Agent, ProfileKind::Terminal]);
        assert_eq!(config.profiles[1].command.as_deref(), Some("agent-acp"));
        assert_eq!(config.profiles[1].args, ["--stdio"]);
        assert_eq!(Profile::default().kind(), ProfileKind::Terminal);
        assert_eq!(Profile::default_shell().kind(), ProfileKind::Terminal);
    }

    /// 1.1: a `type` outside the two is an error naming both values.
    #[test]
    fn an_unknown_profile_type_names_both_kinds() {
        let error = parsed("[[profiles]]\nname = \"Chat\"\ntype = \"chat\"\ncommand = \"x\"").unwrap_err();
        let text = format!("{error:#}");
        assert!(text.contains("chat"), "{text}");
        assert!(text.contains("terminal") && text.contains("agent"), "{text}");
        // Case matters: the key is lowercase.
        assert!(parsed("[[profiles]]\nname = \"A\"\ntype = \"Agent\"\ncommand = \"x\"").is_err());
    }

    /// 1.1: an agent profile without a command (or with a blank one) is an
    /// error naming the profile; a terminal profile needs none.
    #[test]
    fn an_agent_profile_needs_a_command() {
        for text in [
            "[[profiles]]\nname = \"Agent\"\ntype = \"agent\"",
            "[[profiles]]\nname = \"Agent\"\ntype = \"agent\"\ncommand = \"  \"",
            "[[profiles]]\nname = \"Shell\"\n\n[[profiles]]\nname = \"Agent\"\ntype = \"agent\"\nargs = [\"--stdio\"]",
        ] {
            let error = parsed(text).unwrap_err();
            assert_eq!(format!("{error:#}"), "profile \"Agent\": an agent profile needs a command", "{text}");
        }
        assert!(parsed("[[profiles]]\nname = \"Shell\"\ntype = \"terminal\"").is_ok());
    }

    /// 1.1: the profile's `type` and its remote target's live apart: the
    /// target's `type` is inside its inline table.
    #[test]
    fn an_agent_profile_takes_a_remote_target() {
        let config = parsed(
            r#"
            [[profiles]]
            name = "A8 agent"
            type = "agent"
            command = "agent-acp"
            remote = { type = "ssh", host = "A8" }
            "#,
        )
        .unwrap();
        let profile = &config.profiles[0];
        assert_eq!(profile.kind(), ProfileKind::Agent);
        assert!(matches!(&profile.remote, Some(RemoteTarget::Ssh { host, .. }) if host == "A8"));
    }

    /// 1.2: an agent profile reads `[<target> · ]agent · <command>`; a
    /// terminal profile's description is unchanged.
    #[test]
    fn an_agent_profile_describes_its_command() {
        let config = parsed(
            r#"
            [[profiles]]
            name = "Local"
            type = "agent"
            command = "agent-acp"
            args = ["--stdio"]

            [[profiles]]
            name = "Remote"
            type = "agent"
            command = "agent-acp"
            remote = { type = "ssh", host = "A8" }

            [[profiles]]
            name = "Box"
            type = "agent"
            command = "agent-acp"
            remote = { type = "container", engine = "docker", container = "rust-box" }

            [[profiles]]
            name = "Shell"

            [[profiles]]
            name = "Top"
            command = "htop"
            remote = { type = "ssh", host = "A8" }
            "#,
        )
        .unwrap();
        let descriptions: Vec<String> = config.profiles.iter().map(Profile::description).collect();
        assert_eq!(
            descriptions,
            [
                "agent · agent-acp",
                "ssh:A8 · agent · agent-acp",
                "container:rust-box · agent · agent-acp",
                "default shell",
                "ssh:A8 · htop",
            ]
        );
    }

    #[test]
    fn the_removed_tmux_keys_still_load() {
        // Any old value loads, with no error for the banner, and one finding
        // per key; profile keys name their profile.
        let text = r#"
            tmux_session_end = 7

            [[profiles]]
            name = "work"
            tmux = "plain"
            tmux_session = ["not", "a", "name"]
            tmux_args = { odd = true }
            "#;
        let findings = findings_of(text);
        let lines: Vec<String> = findings.iter().map(ToString::to_string).collect();
        assert_eq!(lines.len(), 4, "{lines:?}");
        assert!(lines[0].starts_with("tmux_session_end is ignored"), "{lines:?}");
        for (line, key) in lines[1..].iter().zip(["tmux", "tmux_session", "tmux_args"]) {
            assert!(line.starts_with(&format!("profile \"work\": {key} is ignored")), "{line}");
        }
        assert!(lines[1].contains("command = \"tmux\""), "{}", lines[1]);
        // The profile runs the default shell, not tmux.
        let work = &parsed(text).unwrap().profiles[0];
        assert_eq!(work.command, None);
        assert_eq!(work.description(), "default shell");
        for old in ["tmux_session_end = \"ask\"", "tmux_session_end = \"shell\""] {
            assert_eq!(findings_of(old).len(), 1, "{old}");
        }
    }

    #[test]
    fn a_removed_key_is_a_finding() {
        let findings = findings_of("restore_session = true\n[[profiles]]\nname = \"w\"\ntmux = 1");
        let lines: Vec<String> = findings.iter().map(ToString::to_string).collect();
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[0].starts_with("restore_session is ignored: "), "{lines:?}");
        assert!(lines[1].starts_with("profile \"w\": tmux is ignored: "), "{lines:?}");
    }

    #[test]
    fn a_valid_config_has_no_findings() {
        assert!(findings_of("").is_empty());
        assert!(findings_of(DEFAULT_CONFIG).is_empty());
        assert!(findings_of("theme = \"mine\"\n[themes.mine]\nbackground = \"#000000\"").is_empty());
        assert!(findings_of("[font]\nsize = 4\nline_height = 3.0").is_empty());
        assert!(findings_of("[[profiles]]\nname = \"w\"\ntheme = \"tokyo-night\"").is_empty());
    }

    #[test]
    fn a_clamped_font_size_is_a_finding() {
        let loaded = Config::parse("[font]\nsize = 200").unwrap();
        assert_eq!(loaded.config.font.size, 96.0);
        assert_eq!(loaded.findings, [ConfigFinding::Clamped { key: "font.size", given: 200.0, used: 96.0 }]);
        assert_eq!(loaded.findings[0].to_string(), "font.size 200 is out of range, using 96");
        let loaded = Config::parse("[font]\nsize = 1").unwrap();
        assert_eq!(loaded.findings, [ConfigFinding::Clamped { key: "font.size", given: 1.0, used: 4.0 }]);
    }

    #[test]
    fn a_clamped_line_height_is_a_finding() {
        let loaded = Config::parse("[font]\nline_height = 9.5").unwrap();
        assert_eq!(loaded.config.font.line_height, 3.0);
        assert_eq!(loaded.findings, [ConfigFinding::Clamped { key: "font.line_height", given: 9.5, used: 3.0 }]);
        assert_eq!(loaded.findings[0].to_string(), "font.line_height 9.5 is out of range, using 3");
    }

    #[test]
    fn an_unknown_theme_is_a_finding() {
        let loaded = Config::parse("theme = \"tokyonight\"").unwrap();
        assert_eq!(loaded.config.findings(), [ConfigFinding::UnknownTheme { name: "tokyonight".into(), profile: None }]);
        assert_eq!(loaded.config.findings()[0].to_string(), "unknown theme \"tokyonight\", using brindle-dark");
    }

    #[test]
    fn an_unknown_profile_theme_names_the_profile() {
        let config = parsed("[[profiles]]\nname = \"Work\"\ntheme = \"gruvbox\"\n[[profiles]]\nname = \"Ok\"\ntheme = \"gruvbox-dark\"")
            .unwrap();
        let findings = config.findings();
        assert_eq!(findings, [ConfigFinding::UnknownTheme { name: "gruvbox".into(), profile: Some("Work".into()) }]);
        assert_eq!(findings[0].to_string(), "unknown theme \"gruvbox\" (profile Work), using brindle-dark");
    }

    #[test]
    fn each_finding_says_what_was_used() {
        let findings = [
            ConfigFinding::UnknownTheme { name: "x".into(), profile: None },
            ConfigFinding::Clamped { key: "font.size", given: 200.0, used: 96.0 },
            ConfigFinding::DefaultNotWritten { path: PathBuf::from("/c/config.toml"), error: "denied".into() },
        ];
        let lines: Vec<String> = findings.iter().map(ToString::to_string).collect();
        assert!(lines[0].ends_with("using brindle-dark"), "{}", lines[0]);
        assert!(lines[1].ends_with("using 96"), "{}", lines[1]);
        assert!(lines[2].contains("/c/config.toml") && lines[2].ends_with("denied"), "{}", lines[2]);
    }

    #[test]
    fn no_error_and_no_findings_is_no_line() {
        assert_eq!(config_line(None, &[], &[]), None);
    }

    #[test]
    fn findings_are_joined_after_an_error() {
        let findings = [
            ConfigFinding::Clamped { key: "font.size", given: 200.0, used: 96.0 },
            ConfigFinding::UnknownTheme { name: "x".into(), profile: None },
        ];
        assert_eq!(
            config_line(Some("bad key"), &findings, &[]).as_deref(),
            Some("Config error (using defaults): bad key; font.size 200 is out of range, using 96; unknown theme \"x\", using brindle-dark")
        );
        assert_eq!(
            config_line(None, &findings, &[]).as_deref(),
            Some("Config: font.size 200 is out of range, using 96; unknown theme \"x\", using brindle-dark")
        );
        assert_eq!(config_line(Some("bad key"), &[], &[]).as_deref(), Some("Config error (using defaults): bad key"));
    }

    #[test]
    fn notices_come_last() {
        let findings = [ConfigFinding::UnknownTheme { name: "x".into(), profile: None }];
        let notices = ["can't save".to_string()];
        assert_eq!(
            config_line(None, &findings, &notices).as_deref(),
            Some("Config: unknown theme \"x\", using brindle-dark; can't save")
        );
        assert_eq!(
            config_line(Some("e"), &findings, &notices).as_deref(),
            Some("Config error (using defaults): e; unknown theme \"x\", using brindle-dark; can't save")
        );
        // A notice alone isn't a config problem: no `Config:` prefix.
        assert_eq!(config_line(None, &[], &notices).as_deref(), Some("can't save"));
    }

    #[test]
    fn a_default_that_cant_be_written_is_a_finding() {
        let dir = temp_dir("default-unwritable");
        // The temp file `write_replacing` makes beside the config is a
        // directory, so the write fails while the file reads as missing
        // (a read-only directory would do, but not for root).
        let path = dir.join("config.toml");
        std::fs::create_dir(dir.join(format!("config.toml.{}.tmp", std::process::id()))).unwrap();
        let (config, report) = Config::load_or_default_at(&path);
        assert_eq!(report.error, None);
        assert_eq!(config.theme, "brindle-dark");
        assert!(
            matches!(report.findings.as_slice(), [ConfigFinding::DefaultNotWritten { path: p, .. }] if *p == path),
            "{:?}",
            report.findings
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn notification_settings() {
        let config = parsed("").unwrap();
        assert_eq!(config.notifications, NotificationConfig::default());
        assert!(config.notifications.bell && config.notifications.osc9 && config.notifications.osc777);
        assert!(config.notifications.osc7501 && config.notifications.osc99 && config.notifications.agent);
        let config = parsed("[notifications]\nagent = false").unwrap();
        assert!(!config.notifications.agent);
        assert!(config.notifications.osc7501 && config.notifications.bell);
        let config = parsed("[notifications]\nbell = false\nosc7501 = false\nwhen = \"always\"").unwrap();
        assert!(config.notifications.agent);
        assert!(!config.notifications.bell);
        assert!(config.notifications.osc9 && config.notifications.osc99);
        assert!(!config.notifications.osc7501);
        assert_eq!(config.notifications.when, NotifyWhen::Always);
        let config = parsed("[notifications]\nosc99 = false").unwrap();
        assert!(!config.notifications.osc99);
        assert!(config.notifications.osc9 && config.notifications.osc777 && config.notifications.osc7501);
        assert!(parsed("[notifications]\nwhen = \"sometimes\"").is_err());
        assert!(parsed("[notifications]\nsound = true").is_err());
    }

    // agents-tool-content 5.1: the `[agent]` table.

    #[test]
    fn agent_tool_details_default_open_while_running_and_closed_when_done() {
        let defaults = AgentConfig { tool_details: Shown::Open, tool_details_when_done: Shown::Closed };
        assert_eq!(AgentConfig::default(), defaults);
        assert_eq!(parsed("").unwrap().agent, defaults);
        assert_eq!(parsed("[agent]").unwrap().agent, defaults);
        assert_eq!(parsed(DEFAULT_CONFIG).unwrap().agent, defaults, "the shipped config says the defaults");
    }

    #[test]
    fn agent_tool_details_take_open_and_closed() {
        for (word, shown) in [("open", Shown::Open), ("closed", Shown::Closed)] {
            let config = parsed(&format!("[agent]\ntool_details = {word:?}")).unwrap();
            assert_eq!(config.agent.tool_details, shown);
            assert_eq!(config.agent.tool_details_when_done, Shown::Closed);
            let config = parsed(&format!("[agent]\ntool_details_when_done = {word:?}")).unwrap();
            assert_eq!(config.agent.tool_details_when_done, shown);
            assert_eq!(config.agent.tool_details, Shown::Open);
        }
    }

    /// A value that isn't one of the two names the key and both values,
    /// and loads the defaults, as every invalid config does.
    #[test]
    fn a_bad_tool_details_value_names_the_key_and_its_values() {
        for key in ["tool_details", "tool_details_when_done"] {
            let err = format!("{:#}", parsed(&format!("[agent]\n{key} = \"maybe\"")).unwrap_err());
            assert!(err.contains(&format!("agent.{key}")), "{err}");
            assert!(err.contains("maybe") && err.contains("`open`") && err.contains("`closed`"), "{err}");
        }
        let err = format!("{:#}", parsed("[agent]\ntool_details = true").unwrap_err());
        assert!(err.contains("agent.tool_details"), "{err}");
    }

    #[test]
    fn an_unknown_key_under_agent_is_an_error() {
        let err = format!("{:#}", parsed("[agent]\ntool_detail = \"open\"").unwrap_err());
        assert!(err.contains("tool_detail"), "{err}");
    }

    #[test]
    fn right_click_parses_none_and_paste() {
        assert_eq!(parsed("").unwrap().right_click, RightClick::Nothing);
        assert_eq!(parsed("right_click = \"none\"").unwrap().right_click, RightClick::Nothing);
        assert_eq!(parsed("right_click = \"paste\"").unwrap().right_click, RightClick::Paste);
    }

    /// Each value's word is the one a load reads back as it, and the words
    /// a load refuses are none.
    #[test]
    fn right_click_words_read_back_as_their_values() {
        for value in [RightClick::Nothing, RightClick::Paste] {
            assert_eq!(RightClick::from_word(value.word()), Some(value));
            let config = parsed(&format!("right_click = {:?}", value.word())).unwrap();
            assert_eq!(config.right_click, value);
        }
        assert_eq!(RightClick::from_word("menu"), None);
        assert_eq!(RightClick::from_word("Paste"), None);
    }

    #[test]
    fn an_unknown_right_click_value_is_a_config_error() {
        let err = format!("{:#}", parsed("right_click = \"menu\"").unwrap_err());
        assert!(err.contains("menu") && err.contains("none") && err.contains("paste"), "{err}");
    }

    #[test]
    fn unknown_keys_are_errors() {
        assert!(parsed("fontsize = 3").is_err());
        // Only the removed keys' exact names are tolerated.
        let err = parsed("tmux_sesion_end = \"ask\"").unwrap_err();
        assert!(format!("{err:#}").contains("tmux_sesion_end"), "{err:#}");
        assert!(parsed("[[profiles]]\nname = \"x\"\ntmux_sesion = \"x\"").is_err());
    }

    #[test]
    fn unknown_theme_falls_back() {
        let config = parsed("theme = \"nope\"").unwrap();
        let theme = config.theme(None);
        assert_eq!(theme.background, builtin_theme("brindle-dark").unwrap().background);
    }
}
