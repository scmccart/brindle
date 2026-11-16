//! User configuration: fonts, themes, profiles and keybindings.
//!
//! Loaded from `$XDG_CONFIG_HOME/brindle/config.toml`. A commented default
//! file is written on first launch.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde::Deserialize;

use crate::theme::{Theme, builtin_theme};

pub const DEFAULT_CONFIG: &str = include_str!("../assets/default-config.toml");

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
    /// Allow programs to read the clipboard via OSC 52 (writing is always allowed).
    pub osc52_paste: bool,
    /// The removed `restore_session` key, still accepted so old configs
    /// load: restoring is always on now. Parsing logs that it's ignored.
    #[serde(rename = "restore_session")]
    removed_restore_session: Option<serde::de::IgnoredAny>,
    /// Draw a red bar beside prompts whose command failed (needs shell
    /// integration; see `shell-integration/`).
    pub mark_failed_commands: bool,
    /// The removed `tmux_session_end` key, still accepted (whatever its
    /// value) so old configs load. Parsing logs that it's ignored.
    #[serde(rename = "tmux_session_end")]
    removed_tmux_session_end: Option<toml::Value>,
    /// Desktop notifications from bells and OSC 9/777.
    pub notifications: NotificationConfig,
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
            osc52_paste: false,
            removed_restore_session: None,
            mark_failed_commands: true,
            removed_tmux_session_end: None,
            notifications: NotificationConfig::default(),
            profiles: Vec::new(),
            themes: BTreeMap::new(),
            keybindings: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
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
    /// A program status record (OSC 7501) that becomes blocked, done or
    /// error.
    pub osc7501: bool,
    pub when: NotifyWhen,
}

impl Default for NotificationConfig {
    fn default() -> Self {
        Self { bell: true, osc9: true, osc777: true, osc7501: true, when: NotifyWhen::Unfocused }
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

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Profile {
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
    /// accepted (whatever their values) so old configs load. Parsing logs
    /// that each is ignored.
    #[serde(rename = "tmux")]
    pub(crate) removed_tmux: Option<toml::Value>,
    #[serde(rename = "tmux_session")]
    pub(crate) removed_tmux_session: Option<toml::Value>,
    #[serde(rename = "tmux_args")]
    pub(crate) removed_tmux_args: Option<toml::Value>,
}

impl Profile {
    pub fn default_shell() -> Self {
        Self { name: "Shell".into(), ..Default::default() }
    }

    /// A short description of what the profile runs.
    pub fn description(&self) -> String {
        let target_prefix = match &self.remote {
            Some(RemoteTarget::Ssh { host, .. }) => format!("ssh:{host} · "),
            Some(RemoteTarget::Container { container, .. }) => format!("container:{container} · "),
            None => String::new(),
        };
        let base = self.command.clone().unwrap_or_else(|| "default shell".into());
        format!("{target_prefix}{base}")
    }

    pub fn working_directory(&self) -> Option<PathBuf> {
        self.cwd.as_deref().map(|path| {
            if self.remote.is_some() { PathBuf::from(path) } else { expand_tilde(path) }
        })
    }
}

pub fn expand_tilde(path: &str) -> PathBuf {
    if path == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(path));
    }
    if let Some(rest) = path.strip_prefix("~/")
        && let Some(home) = dirs::home_dir()
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
    /// the file are logged and the defaults are used instead, so a typo never
    /// prevents the terminal from starting.
    pub fn load_or_default() -> (Self, Option<String>) {
        let path = Self::path();
        if !path.exists() {
            if let Err(err) = write_default(&path) {
                log::warn!("could not write default config to {}: {err:#}", path.display());
            }
        }
        match Self::load(&path) {
            Ok(config) => (config, None),
            Err(err) => {
                let message = format!("{err:#}");
                log::error!("{message}");
                (Self::default().normalized(), Some(message))
            }
        }
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(err) => return Err(err).context(format!("reading {}", path.display())),
        };
        Self::parse(&text).with_context(|| format!("invalid config {}", path.display()))
    }

    pub fn parse(text: &str) -> Result<Self> {
        let config: Self = toml::from_str(text)?;
        if config.has_removed_restore_session() {
            log::warn!("restore_session is no longer a setting and is ignored: Brindle always restores (--no-restore skips it once)");
        }
        for warning in config.removed_tmux_warnings() {
            log::warn!("{warning}");
        }
        Ok(config.normalized())
    }

    /// Whether the file still sets the removed `restore_session` key.
    pub fn has_removed_restore_session(&self) -> bool {
        self.removed_restore_session.is_some()
    }

    /// One warning for each removed tmux key the file still sets: the
    /// top-level `tmux_session_end`, and each profile's `tmux`,
    /// `tmux_session` and `tmux_args` (naming the profile).
    fn removed_tmux_warnings(&self) -> Vec<String> {
        const GONE: &str = "Brindle no longer has tmux support";
        let mut warnings = Vec::new();
        if self.removed_tmux_session_end.is_some() {
            warnings.push(format!("tmux_session_end is ignored: {GONE}"));
        }
        for profile in &self.profiles {
            let keys = [
                ("tmux", &profile.removed_tmux),
                ("tmux_session", &profile.removed_tmux_session),
                ("tmux_args", &profile.removed_tmux_args),
            ];
            for (key, value) in keys {
                if value.is_none() {
                    continue;
                }
                let mut warning = format!("profile {:?}: {key} is ignored: {GONE}", profile.name);
                if key == "tmux" {
                    warning.push_str(
                        "; to run tmux in its tabs, set command = \"tmux\" and args = [\"new-session\", \"-A\", \"-s\", \"main\"]",
                    );
                }
                warnings.push(warning);
            }
        }
        warnings
    }

    /// Ensures there is at least one profile and that names are usable.
    fn normalized(mut self) -> Self {
        if self.profiles.is_empty() {
            self.profiles.push(Profile::default_shell());
        }
        for (ix, profile) in self.profiles.iter_mut().enumerate() {
            if profile.name.trim().is_empty() {
                profile.name = format!("Profile {}", ix + 1);
            }
        }
        self.font.size = self.font.size.clamp(4.0, 96.0);
        self.font.line_height = self.font.line_height.clamp(0.8, 3.0);
        self
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

    pub fn profile_theme(&self, profile: &Profile) -> Theme {
        self.theme(profile.theme.as_deref())
    }

    pub fn theme(&self, name: Option<&str>) -> Theme {
        let name = name.unwrap_or(&self.theme);
        if let Some(theme) = self.themes.get(name) {
            return theme.clone();
        }
        builtin_theme(name).unwrap_or_else(|| {
            log::warn!("unknown theme {name:?}, using brindle-dark");
            Theme::default()
        })
    }
}

fn write_default(path: &Path) -> Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, DEFAULT_CONFIG)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_file_parses() {
        let config = Config::parse(DEFAULT_CONFIG).unwrap();
        assert!(!config.profiles.is_empty());
        assert_eq!(config.profiles[config.default_profile_index()].name, "Shell");
    }

    #[test]
    fn ui_font_takes_a_family_list_and_nothing_else() {
        let config = Config::parse("[ui_font]\nfamily = [\"Inter\", \"Segoe UI\"]").unwrap();
        assert_eq!(config.ui_font.family, ["Inter", "Segoe UI"]);
        let err = Config::parse("[ui_font]\nsize = 12").unwrap_err();
        assert!(format!("{err:#}").contains("size"), "{err:#}");
        // Without the table the list is empty, so the built-in list applies.
        assert!(Config::parse("").unwrap().ui_font.family.is_empty());
        assert!(Config::parse(DEFAULT_CONFIG).unwrap().ui_font.family.is_empty());
    }

    #[test]
    fn empty_config_gets_a_shell_profile() {
        let config = Config::parse("").unwrap();
        assert_eq!(config.profiles.len(), 1);
        assert_eq!(config.profiles[0].name, "Shell");
        assert_eq!(config.theme, "brindle-dark");
    }

    #[test]
    fn profiles_and_overrides() {
        let config = Config::parse(
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
        assert!(work.working_directory().unwrap().ends_with("src"));
        assert_eq!(config.font.size, 96.0);
        let theme = config.theme(None);
        assert_eq!(theme.background.to_string(), "#000000");
    }

    #[test]
    fn remote_profiles() {
        let config = Config::parse(
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
        assert_eq!(config.profiles[0].working_directory(), Some(PathBuf::from("~/src")));
        assert!(Config::parse(
            "[[profiles]]\nname = \"bad\"\nremote = { type = \"ssh\", host = \"A8\", password = \"secret\" }"
        )
        .is_err());
    }

    /// 8.1: either kind takes `connect_at_launch`, false when absent; it
    /// isn't part of the endpoint.
    #[test]
    fn remote_targets_may_connect_at_launch() {
        let config = Config::parse(
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
        assert!(Config::parse(
            "[[profiles]]\nname = \"bad\"\nremote = { type = \"container\", container = \"c\", connect_at_launch = 1 }"
        )
        .is_err());
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
        let config = Config::parse(&toml).unwrap();
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
        assert!(!Config::parse("").unwrap().has_removed_restore_session());
        assert!(!Config::parse(DEFAULT_CONFIG).unwrap().has_removed_restore_session());
        assert!(!DEFAULT_CONFIG.contains("restore_session"));
        // Any old value loads, with no error for the banner.
        for old in ["restore_session = true", "restore_session = false"] {
            assert!(Config::parse(old).unwrap().has_removed_restore_session(), "{old}");
        }
    }

    #[test]
    fn the_removed_tmux_keys_still_load() {
        assert!(Config::parse("").unwrap().removed_tmux_warnings().is_empty());
        // Any old value loads, with no error for the banner, and one warning
        // per key; profile keys name their profile.
        let config = Config::parse(
            r#"
            tmux_session_end = 7

            [[profiles]]
            name = "work"
            tmux = "plain"
            tmux_session = ["not", "a", "name"]
            tmux_args = { odd = true }
            "#,
        )
        .unwrap();
        let warnings = config.removed_tmux_warnings();
        assert_eq!(warnings.len(), 4, "{warnings:?}");
        assert!(warnings[0].starts_with("tmux_session_end is ignored"), "{warnings:?}");
        for (warning, key) in warnings[1..].iter().zip(["tmux", "tmux_session", "tmux_args"]) {
            assert!(warning.starts_with(&format!("profile \"work\": {key} is ignored")), "{warning}");
        }
        assert!(warnings[1].contains("command = \"tmux\""), "{}", warnings[1]);
        // The profile runs the default shell, not tmux.
        let work = &config.profiles[0];
        assert_eq!(work.command, None);
        assert_eq!(work.description(), "default shell");
        for old in ["tmux_session_end = \"ask\"", "tmux_session_end = \"shell\""] {
            assert_eq!(Config::parse(old).unwrap().removed_tmux_warnings().len(), 1, "{old}");
        }
    }

    #[test]
    fn notification_settings() {
        let config = Config::parse("").unwrap();
        assert_eq!(config.notifications, NotificationConfig::default());
        assert!(config.notifications.bell && config.notifications.osc9 && config.notifications.osc777);
        assert!(config.notifications.osc7501);
        let config = Config::parse("[notifications]\nbell = false\nosc7501 = false\nwhen = \"always\"").unwrap();
        assert!(!config.notifications.bell);
        assert!(config.notifications.osc9);
        assert!(!config.notifications.osc7501);
        assert_eq!(config.notifications.when, NotifyWhen::Always);
        assert!(Config::parse("[notifications]\nwhen = \"sometimes\"").is_err());
        assert!(Config::parse("[notifications]\nsound = true").is_err());
    }

    #[test]
    fn unknown_keys_are_errors() {
        assert!(Config::parse("fontsize = 3").is_err());
        // Only the removed keys' exact names are tolerated.
        let err = Config::parse("tmux_sesion_end = \"ask\"").unwrap_err();
        assert!(format!("{err:#}").contains("tmux_sesion_end"), "{err:#}");
        assert!(Config::parse("[[profiles]]\nname = \"x\"\ntmux_sesion = \"x\"").is_err());
    }

    #[test]
    fn unknown_theme_falls_back() {
        let config = Config::parse("theme = \"nope\"").unwrap();
        let theme = config.theme(None);
        assert_eq!(theme.background, builtin_theme("brindle-dark").unwrap().background);
    }
}
