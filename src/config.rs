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
            theme: "brindle-dark".into(),
            scrollback_lines: 10_000,
            padding: 6.0,
            cursor: CursorConfig::default(),
            copy_on_select: false,
            osc52_paste: false,
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
            ],
            size: 14.0,
            line_height: 1.25,
        }
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

/// How a profile uses tmux.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "kebab-case")]
pub enum TmuxMode {
    /// Not a tmux profile.
    #[default]
    None,
    /// Run tmux normally inside the terminal (`tmux new -A -s <session>`).
    Plain,
    /// Attach in control mode (`tmux -CC`): tmux windows become native tabs
    /// and panes are drawn natively.
    Control,
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Profile {
    pub name: String,
    /// Program to run. Defaults to `$SHELL`, then `/bin/sh`.
    pub command: Option<String>,
    pub args: Vec<String>,
    /// Working directory. `~` is expanded. Defaults to the home directory.
    pub cwd: Option<String>,
    pub env: BTreeMap<String, String>,
    /// Theme override for tabs opened with this profile.
    pub theme: Option<String>,
    /// Font size override.
    pub font_size: Option<f32>,
    pub tmux: TmuxMode,
    /// tmux session name to create or attach to (tmux profiles only).
    pub tmux_session: Option<String>,
    /// Extra arguments passed to tmux before the command, e.g. `["-L", "work"]`.
    pub tmux_args: Vec<String>,
}

impl Profile {
    pub fn default_shell() -> Self {
        Self { name: "Shell".into(), ..Default::default() }
    }

    pub fn working_directory(&self) -> Option<PathBuf> {
        self.cwd.as_deref().map(expand_tilde)
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
        Ok(config.normalized())
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

    pub fn theme(&self, name: Option<&str>) -> Theme {
        let name = name.unwrap_or(&self.theme);
        if let Some(theme) = self.themes.get(name) {
            return theme.clone();
        }
        builtin_theme(name).unwrap_or_else(|| {
            log::warn!("unknown theme {name:?}, using brindle-dark");
            builtin_theme("brindle-dark").unwrap()
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
        assert!(config.profiles.iter().any(|p| p.tmux == TmuxMode::Control));
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
            tmux = "control"
            tmux_session = "work"

            [themes.mine]
            foreground = "#ffffff"
            background = "#000000"
            "##,
        )
        .unwrap();
        assert_eq!(config.default_profile_index(), 1);
        let work = &config.profiles[1];
        assert_eq!(work.tmux, TmuxMode::Control);
        assert_eq!(work.env.get("FOO").map(String::as_str), Some("bar"));
        assert!(work.working_directory().unwrap().ends_with("src"));
        assert_eq!(config.font.size, 96.0);
        let theme = config.theme(None);
        assert_eq!(theme.background.to_string(), "#000000");
    }

    #[test]
    fn unknown_keys_are_errors() {
        assert!(Config::parse("fontsize = 3").is_err());
    }

    #[test]
    fn unknown_theme_falls_back() {
        let config = Config::parse("theme = \"nope\"").unwrap();
        let theme = config.theme(None);
        assert_eq!(theme.background, builtin_theme("brindle-dark").unwrap().background);
    }
}
