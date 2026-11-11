//! The settings dialog's Profiles section: the profile list, and an editor
//! for every profile setting. Saving writes one profile's table with
//! `config_edit`; theme and font size preview on the profile's open tabs.
//!
//! [`ProfilesState`] holds the state and logic and has no GPUI dependency;
//! the view half is in the parent module.

use std::path::Path;

use crate::config::{Profile, RemoteTarget, TmuxMode};
use crate::config_edit::{self, IMPLICIT_PROFILE, ProfileRef};

/// A profile in the list.
#[derive(Clone, Debug)]
pub struct ProfileEntry {
    /// The name it is listed (and looked up) by.
    pub name: String,
    pub at: ProfileRef,
    /// A profile that can't be read carries its error.
    pub profile: Result<Profile, String>,
}

/// A setting the profile editor edits, in the editor's (and tab) order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Name,
    Command,
    Args,
    Cwd,
    Env,
    Theme,
    FontSize,
    Tmux,
    Session,
    TmuxArgs,
    Default,
    Remote,
    SshHost,
    SshUser,
    SshPort,
    SshIdentity,
    SshArgs,
    Engine,
    Container,
    ContainerUser,
}

pub const FIELDS: [Field; 20] = [
    Field::Name,
    Field::Remote,
    Field::SshHost,
    Field::SshUser,
    Field::SshPort,
    Field::SshIdentity,
    Field::SshArgs,
    Field::Engine,
    Field::Container,
    Field::ContainerUser,
    Field::Command,
    Field::Args,
    Field::Cwd,
    Field::Env,
    Field::Theme,
    Field::FontSize,
    Field::Tmux,
    Field::Session,
    Field::TmuxArgs,
    Field::Default,
];

/// The fields edited as text, in [`Values::texts`] order.
pub const TEXT_FIELDS: [Field; 15] = [
    Field::Name,
    Field::Command,
    Field::Args,
    Field::Cwd,
    Field::Env,
    Field::FontSize,
    Field::Session,
    Field::TmuxArgs,
    Field::SshHost,
    Field::SshUser,
    Field::SshPort,
    Field::SshIdentity,
    Field::SshArgs,
    Field::Container,
    Field::ContainerUser,
];

/// What kind of endpoint a profile runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteKind {
    Local,
    Ssh,
    Container,
}

pub const REMOTE_KINDS: [RemoteKind; 3] = [RemoteKind::Local, RemoteKind::Ssh, RemoteKind::Container];

pub fn remote_label(kind: RemoteKind) -> &'static str {
    match kind {
        RemoteKind::Local => "none",
        RemoteKind::Ssh => "ssh",
        RemoteKind::Container => "container",
    }
}

pub const ENGINES: [&str; 2] = ["docker", "podman"];

impl Field {
    pub fn label(self) -> &'static str {
        match self {
            Field::Name => "Name",
            Field::Command => "Command",
            Field::Args => "Arguments",
            Field::Cwd => "Directory",
            Field::Env => "Environment",
            Field::Theme => "Theme",
            Field::FontSize => "Font size",
            Field::Tmux => "tmux",
            Field::Session => "Session",
            Field::TmuxArgs => "tmux arguments",
            Field::Default => "Default",
            Field::Remote => "Remote",
            Field::SshHost => "Host",
            Field::SshUser => "User",
            Field::SshPort => "Port",
            Field::SshIdentity => "Identity file",
            Field::SshArgs => "ssh arguments",
            Field::Engine => "Engine",
            Field::Container => "Container",
            Field::ContainerUser => "User",
        }
    }

    /// Placeholder text: what a blank field means.
    pub fn placeholder(self) -> &'static str {
        match self {
            Field::Command => "your shell",
            Field::Args => "shell words, e.g. -l",
            Field::Cwd => "default (home, or the current tab's directory)",
            Field::Env => "KEY=value …",
            Field::FontSize => "global size",
            Field::Session => "main",
            Field::TmuxArgs => "shell words, e.g. -L work",
            Field::SshHost => "hostname or ssh config alias",
            Field::SshUser | Field::ContainerUser => "default user",
            Field::SshPort => "22",
            Field::SshIdentity => "default key",
            Field::SshArgs => "shell words, e.g. -J bastion",
            Field::Container => "name or id",
            _ => "",
        }
    }

    fn ssh_only(self) -> bool {
        matches!(self, Field::SshHost | Field::SshUser | Field::SshPort | Field::SshIdentity | Field::SshArgs)
    }

    fn container_only(self) -> bool {
        matches!(self, Field::Engine | Field::Container | Field::ContainerUser)
    }

    pub fn text_index(self) -> Option<usize> {
        TEXT_FIELDS.iter().position(|f| *f == self)
    }

    fn tmux_only(self) -> bool {
        matches!(self, Field::Session | Field::TmuxArgs)
    }
}

pub const TMUX_MODES: [TmuxMode; 3] = [TmuxMode::None, TmuxMode::Plain, TmuxMode::Control];

pub fn tmux_label(mode: TmuxMode) -> &'static str {
    match mode {
        TmuxMode::None => "none",
        TmuxMode::Plain => "plain",
        TmuxMode::Control => "control",
    }
}

/// Words joined so `shlex::split` gives them back.
fn join_words(words: &[String]) -> String {
    shlex::try_join(words.iter().map(String::as_str)).unwrap_or_else(|_| words.join(" "))
}

fn quote(word: &str) -> String {
    shlex::try_quote(word).map(|q| q.into_owned()).unwrap_or_else(|_| word.to_string())
}

fn is_env_name(key: &str) -> bool {
    let mut chars = key.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn split_words(text: &str) -> Result<Vec<String>, String> {
    shlex::split(text).ok_or_else(|| "not valid shell words (an unclosed quote?)".to_string())
}

/// What the editor holds; compared with the starting values to tell
/// whether there are unsaved edits.
#[derive(Clone, Debug, PartialEq)]
struct Values {
    texts: [String; TEXT_FIELDS.len()],
    /// `None` follows the global theme.
    theme: Option<String>,
    tmux: TmuxMode,
    remote: RemoteKind,
    /// `docker` or `podman`.
    engine: String,
    default: bool,
}

impl Values {
    fn from_profile(p: &Profile, default: bool) -> Self {
        let env: Vec<String> = p.env.iter().map(|(k, v)| format!("{k}={}", quote(v))).collect();
        let mut texts = [
            p.name.clone(),
            p.command.clone().unwrap_or_default(),
            join_words(&p.args),
            p.cwd.clone().unwrap_or_default(),
            env.join(" "),
            p.font_size.map(|s| s.to_string()).unwrap_or_default(),
            p.tmux_session.clone().unwrap_or_default(),
            join_words(&p.tmux_args),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
            String::new(),
        ];
        let at = |field: Field| TEXT_FIELDS.iter().position(|f| *f == field).expect("text field");
        let mut engine = ENGINES[0].to_string();
        let kind = match &p.remote {
            None => RemoteKind::Local,
            Some(RemoteTarget::Ssh { host, user, port, identity_file, ssh_args }) => {
                texts[at(Field::SshHost)] = host.clone();
                texts[at(Field::SshUser)] = user.clone().unwrap_or_default();
                texts[at(Field::SshPort)] = port.map(|p| p.to_string()).unwrap_or_default();
                texts[at(Field::SshIdentity)] = identity_file.clone().unwrap_or_default();
                texts[at(Field::SshArgs)] = join_words(ssh_args);
                RemoteKind::Ssh
            }
            Some(RemoteTarget::Container { engine: e, container, user }) => {
                engine = e.clone();
                texts[at(Field::Container)] = container.clone();
                texts[at(Field::ContainerUser)] = user.clone().unwrap_or_default();
                RemoteKind::Container
            }
        };
        Self { texts, theme: p.theme.clone(), remote: kind, engine, tmux: p.tmux, default }
    }

    fn text(&self, field: Field) -> &str {
        field.text_index().map_or("", |ix| &self.texts[ix])
    }
}

/// The profile editor.
#[derive(Clone, Debug)]
pub struct ProfileEditor {
    /// The saved profile being edited; `None` for a new one.
    pub at: Option<ProfileRef>,
    /// Its listed name, for renames and the preview.
    pub original_name: Option<String>,
    /// List selection to go back to on discard.
    return_to: usize,
    initial: Values,
    values: Values,
    /// "Follow global" (`None`), then every theme by name.
    pub theme_choices: Vec<Option<String>>,
}

impl ProfileEditor {
    fn new(
        profile: &Profile,
        at: Option<ProfileRef>,
        default: bool,
        return_to: usize,
        themes: &[String],
    ) -> Self {
        let values = Values::from_profile(profile, default);
        let mut theme_choices: Vec<Option<String>> = std::iter::once(None).chain(themes.iter().cloned().map(Some)).collect();
        // A theme that no longer exists stays choosable, so opening and
        // saving doesn't change it.
        if let Some(theme) = &profile.theme
            && !themes.contains(theme)
        {
            theme_choices.push(Some(theme.clone()));
        }
        let original_name = at.as_ref().map(|_| profile.name.clone());
        Self {
            at,
            original_name,
            return_to,
            initial: values.clone(),
            values,
            theme_choices,
        }
    }

    pub fn text(&self, field: Field) -> &str {
        self.values.text(field)
    }

    pub fn set_text(&mut self, field: Field, text: &str) {
        if let Some(ix) = field.text_index() {
            self.values.texts[ix] = text.to_string();
        }
    }

    pub fn theme(&self) -> Option<&str> {
        self.values.theme.as_deref()
    }

    pub fn tmux(&self) -> TmuxMode {
        self.values.tmux
    }

    pub fn is_default(&self) -> bool {
        self.values.default
    }

    /// The current default can't be unset; another profile is made the
    /// default instead.
    pub fn default_locked(&self) -> bool {
        self.initial.default
    }

    pub fn dirty(&self) -> bool {
        self.values != self.initial
    }

    /// The fields shown: the tmux session and arguments only for tmux, and
    /// the target's fields only for its kind.
    pub fn visible_fields(&self) -> Vec<Field> {
        FIELDS
            .into_iter()
            .filter(|f| !f.tmux_only() || self.values.tmux != TmuxMode::None)
            .filter(|f| !f.ssh_only() || self.values.remote == RemoteKind::Ssh)
            .filter(|f| !f.container_only() || self.values.remote == RemoteKind::Container)
            .collect()
    }

    #[cfg(test)]
    pub fn remote_kind(&self) -> RemoteKind {
        self.values.remote
    }

    #[cfg(test)]
    pub fn engine(&self) -> &str {
        &self.values.engine
    }

    pub fn choose_remote(&mut self, kind: RemoteKind) {
        self.values.remote = kind;
    }

    /// How many options choice field `field` has (0 for other fields).
    #[cfg(test)]
    pub fn choice_count(&self, field: Field) -> usize {
        match field {
            Field::Theme => self.theme_choices.len(),
            Field::Tmux => TMUX_MODES.len(),
            Field::Remote => REMOTE_KINDS.len(),
            Field::Engine => ENGINES.len(),
            _ => 0,
        }
    }

    /// The option choice field `field` shows, as an index into its options
    /// (`theme_choices`, [`TMUX_MODES`], [`REMOTE_KINDS`], [`ENGINES`]).
    pub fn choice_index(&self, field: Field) -> Option<usize> {
        match field {
            Field::Theme => self.theme_choices.iter().position(|t| *t == self.values.theme),
            Field::Tmux => TMUX_MODES.iter().position(|m| *m == self.values.tmux),
            Field::Remote => REMOTE_KINDS.iter().position(|k| *k == self.values.remote),
            Field::Engine => ENGINES.iter().position(|e| *e == self.values.engine),
            _ => None,
        }
    }

    /// Chooses option `ix` of choice field `field`, as its select does. An
    /// index out of range changes nothing.
    pub fn choose(&mut self, field: Field, ix: usize) {
        match field {
            Field::Theme => {
                if let Some(theme) = self.theme_choices.get(ix) {
                    self.values.theme = theme.clone();
                }
            }
            Field::Tmux => {
                if let Some(&mode) = TMUX_MODES.get(ix) {
                    self.values.tmux = mode;
                }
            }
            Field::Remote => {
                if let Some(&kind) = REMOTE_KINDS.get(ix) {
                    self.choose_remote(kind);
                }
            }
            Field::Engine => {
                if let Some(engine) = ENGINES.get(ix) {
                    self.values.engine = engine.to_string();
                }
            }
            _ => {}
        }
    }

    pub fn toggle_default(&mut self) {
        if !self.default_locked() {
            self.values.default = !self.values.default;
        }
    }

    /// The profile the fields describe, or each invalid field's problem.
    /// `others` are the other profiles' names.
    pub fn result(&self, others: &[&str]) -> Result<Profile, Vec<(Field, String)>> {
        let mut errors = Vec::new();
        let text = |f: Field| self.values.text(f).trim();
        let optional = |f: Field| Some(text(f).to_string()).filter(|t| !t.is_empty());
        let words = |f: Field, errors: &mut Vec<(Field, String)>| match split_words(self.values.text(f)) {
            Ok(words) => words,
            Err(e) => {
                errors.push((f, e));
                Vec::new()
            }
        };

        let name = text(Field::Name);
        if name.is_empty() {
            errors.push((Field::Name, "the name can't be empty".into()));
        } else if others.iter().any(|o| o.eq_ignore_ascii_case(name)) {
            errors.push((Field::Name, format!("a profile named {name:?} already exists")));
        }
        let args = words(Field::Args, &mut errors);
        let tmux_args = if self.values.tmux == TmuxMode::None { Vec::new() } else { words(Field::TmuxArgs, &mut errors) };
        let mut env = std::collections::BTreeMap::new();
        for word in words(Field::Env, &mut errors) {
            match word.split_once('=') {
                Some((key, value)) if is_env_name(key) => _ = env.insert(key.to_string(), value.to_string()),
                Some((key, _)) => errors.push((Field::Env, format!("{key:?} isn't a valid variable name"))),
                None => errors.push((Field::Env, format!("{word:?} has no `=`"))),
            }
        }
        let font_size = match optional(Field::FontSize) {
            None => None,
            Some(t) => match t.parse::<f32>() {
                Ok(size) if (4.0..=96.0).contains(&size) => Some(size),
                _ => {
                    errors.push((Field::FontSize, "use a number from 4 to 96".into()));
                    None
                }
            },
        };
        let remote = match self.values.remote {
            RemoteKind::Local => None,
            RemoteKind::Ssh => {
                let host = text(Field::SshHost);
                if host.is_empty() {
                    errors.push((Field::SshHost, "the host can't be empty".into()));
                }
                let port = match optional(Field::SshPort) {
                    None => None,
                    Some(t) => match t.parse::<u16>() {
                        Ok(port) if port >= 1 => Some(port),
                        _ => {
                            errors.push((Field::SshPort, "use a number from 1 to 65535".into()));
                            None
                        }
                    },
                };
                Some(RemoteTarget::Ssh {
                    host: host.to_string(),
                    user: optional(Field::SshUser),
                    port,
                    identity_file: optional(Field::SshIdentity),
                    ssh_args: words(Field::SshArgs, &mut errors),
                })
            }
            RemoteKind::Container => {
                let container = text(Field::Container);
                if container.is_empty() {
                    errors.push((Field::Container, "the container can't be empty".into()));
                }
                Some(RemoteTarget::Container {
                    engine: self.values.engine.clone(),
                    container: container.to_string(),
                    user: optional(Field::ContainerUser),
                })
            }
        };
        if !errors.is_empty() {
            return Err(errors);
        }
        let tmux = self.values.tmux;
        Ok(Profile {
            name: name.to_string(),
            remote,
            command: optional(Field::Command),
            args,
            cwd: optional(Field::Cwd),
            env,
            theme: self.values.theme.clone(),
            font_size,
            tmux,
            tmux_session: optional(Field::Session).filter(|_| tmux != TmuxMode::None),
            tmux_args,
            ..Default::default()
        })
    }

    /// What the profile's open tabs should show now: the chosen theme, and
    /// the font size when it is valid (else the one it started with).
    fn preview(&self, base: &Profile) -> Profile {
        let mut profile = base.clone();
        profile.theme = self.values.theme.clone();
        let size = self.values.text(Field::FontSize).trim();
        if size.is_empty() {
            profile.font_size = None;
        } else if let Ok(size) = size.parse::<f32>()
            && (4.0..=96.0).contains(&size)
        {
            profile.font_size = Some(size);
        }
        profile
    }
}

#[derive(Clone, Debug)]
pub enum Mode {
    List,
    Editor(ProfileEditor),
    /// Asking whether to delete the selected profile.
    ConfirmDelete,
}

pub struct ProfilesState {
    pub entries: Vec<ProfileEntry>,
    /// `default_profile` from the file.
    pub default_name: Option<String>,
    pub selected: usize,
    pub mode: Mode,
    /// Why the list can't be edited (the file isn't valid TOML).
    pub read_only: Option<String>,
    /// Renames saved since the last [`Self::take_renames`].
    renames: Vec<(String, String)>,
}

/// What a profile operation did.
#[derive(Debug)]
pub enum Done {
    Nothing,
    /// The config file was written.
    Written,
}

impl ProfilesState {
    pub fn new(path: &Path, fallback: &[Profile], fallback_default: Option<String>) -> Self {
        let mut state = Self {
            entries: Vec::new(),
            default_name: None,
            selected: 0,
            mode: Mode::List,
            read_only: None,
            renames: Vec::new(),
        };
        state.reload(path, fallback, fallback_default);
        state.selected = state.default_index();
        state
    }

    /// Rereads the profiles from the file, or lists `fallback` read-only
    /// when it can't be parsed.
    pub fn reload(&mut self, path: &Path, fallback: &[Profile], fallback_default: Option<String>) {
        let doc = std::fs::read_to_string(path).map_err(|e| e.to_string()).and_then(|t| config_edit::parse(&t));
        let read = match doc {
            Ok(doc) => {
                self.read_only = None;
                self.default_name = doc.get("default_profile").and_then(|i| i.as_str()).map(str::to_string);
                config_edit::profiles(&doc)
            }
            Err(e) => {
                self.read_only = Some(e);
                self.default_name = fallback_default;
                fallback.iter().map(|p| (Some(p.name.clone()), Ok(p.clone()))).collect()
            }
        };
        self.entries = read
            .into_iter()
            .enumerate()
            .map(|(index, (raw, profile))| {
                // Unnamed profiles are listed as `Profile N`, as `Config` names them.
                let name = match (&profile, &raw) {
                    (Ok(p), _) if !p.name.trim().is_empty() => p.name.clone(),
                    (Err(_), Some(raw)) if !raw.trim().is_empty() => raw.clone(),
                    _ => format!("Profile {}", index + 1),
                };
                let profile = profile.map(|mut p| {
                    p.name = name.clone();
                    p
                });
                ProfileEntry { name, at: ProfileRef { index, name: raw }, profile }
            })
            .collect();
        if self.entries.is_empty() {
            self.entries.push(ProfileEntry {
                name: IMPLICIT_PROFILE.into(),
                at: ProfileRef { index: 0, name: Some(IMPLICIT_PROFILE.into()) },
                profile: Ok(Profile::default_shell()),
            });
        }
        self.selected = self.selected.min(self.entries.len() - 1);
    }

    /// The profile new tabs use: the one `default_profile` names, else the
    /// first (as `Config::default_profile_index`).
    pub fn default_index(&self) -> usize {
        self.default_name
            .as_ref()
            .and_then(|d| self.entries.iter().position(|e| e.name.eq_ignore_ascii_case(d)))
            .unwrap_or(0)
    }

    pub fn selected_entry(&self) -> Option<&ProfileEntry> {
        self.entries.get(self.selected)
    }

    pub fn take_renames(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.renames)
    }

    fn names_except(&self, except: Option<&str>) -> Vec<&str> {
        self.entries.iter().map(|e| e.name.as_str()).filter(|n| Some(*n) != except).collect()
    }

    /// The name and settings to preview on the edited profile's open tabs.
    pub fn preview(&self) -> Option<(String, Profile)> {
        let Mode::Editor(editor) = &self.mode else { return None };
        let name = editor.original_name.clone()?;
        let base = self.entries.iter().find(|e| e.name == name)?.profile.as_ref().ok()?;
        Some((name, editor.preview(base)))
    }

    pub fn move_selection(&mut self, forward: bool) {
        match &mut self.mode {
            Mode::List => {
                let n = self.entries.len();
                self.selected = if forward { (self.selected + 1) % n } else { (self.selected + n - 1) % n };
            }
            // The editor's fields are focus stops: the view moves focus.
            Mode::Editor(_) | Mode::ConfirmDelete => {}
        }
    }

    pub fn select(&mut self, ix: usize) {
        if matches!(self.mode, Mode::List) && ix < self.entries.len() {
            self.selected = ix;
        }
    }

    fn editable(&self) -> Result<(), String> {
        match &self.read_only {
            Some(e) => Err(e.clone()),
            None => Ok(()),
        }
    }

    fn open_editor(&mut self, profile: &Profile, at: Option<ProfileRef>, default: bool, themes: &[String]) {
        self.mode = Mode::Editor(ProfileEditor::new(profile, at, default, self.selected, themes));
    }

    /// Edit: the selected profile.
    pub fn start_edit(&mut self, themes: &[String]) -> Result<(), String> {
        self.editable()?;
        let entry = self.selected_entry().ok_or("no profile selected")?.clone();
        let profile = entry.profile.map_err(|e| format!("{:?} can't be read: {e}", entry.name))?;
        let default = self.selected == self.default_index();
        self.open_editor(&profile, Some(entry.at), default, themes);
        Ok(())
    }

    /// New: default settings under an unused `Profile N`.
    pub fn start_new(&mut self, themes: &[String]) -> Result<(), String> {
        self.editable()?;
        let taken = self.names_except(None);
        let name = (self.entries.len() + 1..)
            .map(|n| format!("Profile {n}"))
            .find(|n| !taken.iter().any(|t| t.eq_ignore_ascii_case(n)))
            .unwrap();
        self.open_editor(&Profile { name, ..Default::default() }, None, false, themes);
        Ok(())
    }

    /// Duplicate: the selected profile's settings as `<name> copy`.
    pub fn start_duplicate(&mut self, themes: &[String]) -> Result<(), String> {
        self.editable()?;
        let entry = self.selected_entry().ok_or("no profile selected")?.clone();
        let mut profile = entry.profile.map_err(|e| format!("{:?} can't be read: {e}", entry.name))?;
        let taken = self.names_except(None);
        let free = |n: &String| !taken.iter().any(|t| t.eq_ignore_ascii_case(n));
        let first = format!("{} copy", entry.name);
        profile.name = if free(&first) {
            first
        } else {
            (2..).map(|n| format!("{} copy {n}", entry.name)).find(free).unwrap()
        };
        self.open_editor(&profile, None, false, themes);
        Ok(())
    }

    pub fn editor(&self) -> Option<&ProfileEditor> {
        match &self.mode {
            Mode::Editor(editor) => Some(editor),
            _ => None,
        }
    }

    pub fn editor_mut(&mut self) -> Option<&mut ProfileEditor> {
        match &mut self.mode {
            Mode::Editor(editor) => Some(editor),
            _ => None,
        }
    }

    /// The editor's validation, with the other profiles' names.
    pub fn editor_result(&self) -> Option<Result<Profile, Vec<(Field, String)>>> {
        let editor = self.editor()?;
        Some(editor.result(&self.names_except(editor.original_name.as_deref())))
    }

    /// Save: writes the profile and returns to the list with it selected.
    pub fn save(&mut self, path: &Path, fallback: &[Profile]) -> Result<Done, String> {
        let Some(Ok(profile)) = self.editor_result() else {
            return Err("Fix the fields marked invalid first".into());
        };
        let editor = self.editor().expect("editing").clone();
        let old = editor.original_name.clone();
        let make_default = editor.is_default() && !editor.default_locked();
        config_edit::save(path, |doc| {
            config_edit::upsert_profile(doc, editor.at.as_ref(), &profile)?;
            if let Some(old) = old.as_deref().filter(|old| *old != profile.name) {
                config_edit::rename_default_profile(doc, old, &profile.name);
            }
            if make_default {
                config_edit::set_default_profile(doc, &profile.name);
            }
            Ok(())
        })?;
        if let Some(old) = old.filter(|old| *old != profile.name) {
            self.renames.push((old, profile.name.clone()));
        }
        self.mode = Mode::List;
        self.reload(path, fallback, None);
        self.selected = self.entries.iter().position(|e| e.name == profile.name).unwrap_or(0);
        Ok(Done::Written)
    }

    /// Discard: back to the list as it was.
    pub fn discard(&mut self) {
        if let Mode::Editor(editor) = &self.mode {
            self.selected = editor.return_to.min(self.entries.len() - 1);
        }
        self.mode = Mode::List;
    }

    /// Delete: asks for confirmation, or explains why the profile can't go.
    pub fn request_delete(&mut self) -> Result<(), String> {
        self.editable()?;
        if self.entries.len() <= 1 {
            return Err("The last profile can't be deleted".into());
        }
        self.mode = Mode::ConfirmDelete;
        Ok(())
    }

    /// The confirmation question.
    pub fn delete_question(&self) -> String {
        let name = self.selected_entry().map(|e| e.name.as_str()).unwrap_or("");
        if self.selected == self.default_index() && self.default_name.is_some() {
            format!("Delete {name:?}? It is the default profile; new tabs will use the first profile instead.")
        } else {
            format!("Delete {name:?}?")
        }
    }

    pub fn delete_selected(&mut self, path: &Path, fallback: &[Profile]) -> Result<Done, String> {
        self.mode = Mode::List;
        let entry = self.selected_entry().ok_or("no profile selected")?.clone();
        config_edit::save(path, |doc| config_edit::delete_profile(doc, &entry.at, &entry.name))?;
        self.reload(path, fallback, None);
        Ok(Done::Written)
    }

    /// alt-up / alt-down: moves the selected profile one place.
    pub fn move_selected(&mut self, up: bool, path: &Path, fallback: &[Profile]) -> Result<Done, String> {
        self.editable()?;
        if !matches!(self.mode, Mode::List) {
            return Ok(Done::Nothing);
        }
        let entry = self.selected_entry().ok_or("no profile selected")?.clone();
        let edge = if up { self.selected == 0 } else { self.selected + 1 >= self.entries.len() };
        if edge {
            return Ok(Done::Nothing);
        }
        let mut moved_to = entry.at.index;
        config_edit::save(path, |doc| {
            moved_to = config_edit::move_profile(doc, &entry.at, up)?;
            Ok(())
        })?;
        self.reload(path, fallback, None);
        self.selected = moved_to.min(self.entries.len() - 1);
        Ok(Done::Written)
    }

    /// Debug description for `--dump-screen-after`. `field` names the
    /// focused control, reported as the editor's `field=`.
    pub fn describe(&self, error: &Option<String>, field: &str) -> String {
        let section = "Profiles";
        match &self.mode {
            Mode::List => format!(
                "--- settings section={section} mode=list selected={:?} default={:?} profiles={:?} error={:?}\n",
                self.selected_entry().map(|e| e.name.as_str()).unwrap_or(""),
                self.entries.get(self.default_index()).map(|e| e.name.as_str()).unwrap_or(""),
                self.entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(),
                error
            ),
            Mode::Editor(editor) => {
                let invalid = match self.editor_result() {
                    Some(Err(errors)) => errors.iter().map(|(f, _)| f.label()).collect::<Vec<_>>(),
                    _ => Vec::new(),
                };
                format!(
                    "--- settings section={section} mode=editor name={:?} field={:?} theme={:?} tmux={:?} default={} invalid={:?} can_save={} error={:?}\n",
                    editor.text(Field::Name),
                    field,
                    editor.theme().unwrap_or("follow global"),
                    tmux_label(editor.tmux()),
                    editor.is_default(),
                    invalid,
                    invalid.is_empty(),
                    error
                )
            }
            Mode::ConfirmDelete => format!(
                "--- settings section={section} mode=confirm-delete profile={:?}\n",
                self.selected_entry().map(|e| e.name.as_str()).unwrap_or("")
            ),
        }
    }
}

// ---- view -------------------------------------------------------------------

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, FocusHandle, Focusable as _, InteractiveElement, IntoElement,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};

use super::{Btn, Choice, Section, SettingsDialog};
use crate::actions::{
    SettingsDuplicate, SettingsMoveDown, SettingsMoveUp, SettingsSectionFirst, SettingsSectionLast, SettingsSectionNext,
    SettingsSectionPrev,
};
use brindle_ui::{ActiveTheme as _, ButtonVariant, Checkbox, Input, Select, SelectEvent, TextInput, TextInputEvent};

/// The fields shown as a select rather than text.
const SELECT_FIELDS: [Field; 4] = [Field::Theme, Field::Tmux, Field::Remote, Field::Engine];

impl SettingsDialog {
    /// Creates the profile editor's inputs and selects when it opens and
    /// drops them when it closes. Focus follows in `place_focus`.
    pub(super) fn sync_profile_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let editing = self.model.section() == Section::Profiles && self.model.profiles.editor().is_some();
        if !editing {
            // A dropped select would leave an open menu's layer behind.
            for (_, select) in std::mem::take(&mut self.profile_selects) {
                select.update(cx, |s, cx| s.close(window, cx));
            }
            self.profile_inputs.clear();
            self._profile_subscriptions.clear();
            return;
        }
        if !self.profile_inputs.is_empty() {
            return;
        }
        let editor = self.model.profiles.editor().expect("editing").clone();
        for field in TEXT_FIELDS {
            let text = editor.text(field).to_string();
            let input = cx.new(|cx| TextInput::new(&text, cx).with_placeholder(field.placeholder()));
            let changed = cx.subscribe_in(&input, window, move |this, input, _: &TextInputEvent, _, cx| {
                let text = input.read(cx).text().to_string();
                if let Some(editor) = this.model.profiles.editor_mut() {
                    editor.set_text(field, &text);
                }
                this.model.error = None;
                this.preview(cx);
                cx.notify();
            });
            self._profile_subscriptions.push(changed);
            self.profile_inputs.push((field, input));
        }
        for field in SELECT_FIELDS {
            let labels = self.choice_labels(&editor, field);
            let selected = editor.choice_index(field);
            let select = cx.new(|cx| Select::new(labels, selected, cx));
            let changed = cx.subscribe_in(&select, window, move |this, _, event: &SelectEvent, window, cx| {
                let SelectEvent::Changed(ix) = *event;
                this.apply_choice(Choice::Profile(field), ix, window, cx);
            });
            self._profile_subscriptions.push(changed);
            self.profile_selects.push((field, select));
        }
    }

    /// The options of select field `field`, in the editor's order.
    fn choice_labels(&self, editor: &ProfileEditor, field: Field) -> Vec<SharedString> {
        match field {
            Field::Theme => {
                let known = |name: &str| self.model.entries.iter().any(|e| e.theme.is_ok() && e.name == name);
                editor
                    .theme_choices
                    .iter()
                    .map(|theme| match theme {
                        None => "follow global".into(),
                        Some(name) if known(name) => name.clone().into(),
                        Some(name) => format!("{name} (missing)").into(),
                    })
                    .collect()
            }
            Field::Tmux => TMUX_MODES.iter().map(|&m| tmux_label(m).into()).collect(),
            Field::Remote => REMOTE_KINDS.iter().map(|&k| remote_label(k).into()).collect(),
            Field::Engine => ENGINES.iter().map(|&e| e.into()).collect(),
            _ => Vec::new(),
        }
    }

    fn profile_input(&self, field: Field) -> Option<&Entity<TextInput>> {
        self.profile_inputs.iter().find(|(f, _)| *f == field).map(|(_, i)| i)
    }

    fn profile_select(&self, field: Field) -> Option<&Entity<Select>> {
        self.profile_selects.iter().find(|(f, _)| *f == field).map(|(_, s)| s)
    }

    /// The focus handle of a shown profile field, `None` for the default
    /// checkbox while it can't be changed (it takes no focus then).
    fn profile_field_handle(&self, editor: &ProfileEditor, field: Field, cx: &App) -> Option<FocusHandle> {
        match field {
            Field::Default => (!editor.default_locked()).then(|| self.default_handle.clone()),
            f if SELECT_FIELDS.contains(&f) => self.profile_select(f).map(|s| s.focus_handle(cx)),
            f => self.profile_input(f).map(|i| i.focus_handle(cx)),
        }
    }

    /// The profile editor's field stops: its shown fields, in order.
    pub(super) fn profile_field_stops(&self, cx: &App) -> Vec<FocusHandle> {
        let Some(editor) = self.model.profiles.editor() else { return Vec::new() };
        editor.visible_fields().into_iter().filter_map(|f| self.profile_field_handle(editor, f, cx)).collect()
    }

    /// Scrolls the profile editor's focused field into view.
    pub(super) fn scroll_to_focused_profile_field(&self, window: &Window, cx: &App) {
        let Some(editor) = self.model.profiles.editor() else { return };
        // The name sits above the scrolling body.
        let rows = editor.visible_fields();
        let focused = rows.iter().position(|&f| {
            self.profile_field_handle(editor, f, cx).is_some_and(|h| h.is_focused(window))
                || (SELECT_FIELDS.contains(&f) && self.profile_select(f).is_some_and(|s| s.read(cx).is_open()))
        });
        if let Some(ix) = focused.filter(|ix| *ix > 0) {
            self.profile_editor_scroll.scroll_to_item(ix - 1);
        }
    }

    // ---- actions ----

    pub(super) fn section_next(&mut self, _: &SettingsSectionNext, window: &mut Window, cx: &mut Context<Self>) {
        self.model.next_section(true);
        self.changed(window, cx);
    }

    pub(super) fn section_prev(&mut self, _: &SettingsSectionPrev, window: &mut Window, cx: &mut Context<Self>) {
        self.model.next_section(false);
        self.changed(window, cx);
    }

    pub(super) fn section_first(&mut self, _: &SettingsSectionFirst, window: &mut Window, cx: &mut Context<Self>) {
        self.model.edge_section(false);
        self.changed(window, cx);
    }

    pub(super) fn section_last(&mut self, _: &SettingsSectionLast, window: &mut Window, cx: &mut Context<Self>) {
        self.model.edge_section(true);
        self.changed(window, cx);
    }

    pub(super) fn duplicate(&mut self, _: &SettingsDuplicate, window: &mut Window, cx: &mut Context<Self>) {
        self.model.start_duplicate();
        self.changed(window, cx);
    }

    pub(super) fn move_up(&mut self, _: &SettingsMoveUp, window: &mut Window, cx: &mut Context<Self>) {
        let outcome = self.model.move_profile(true);
        self.outcome(outcome, window, cx);
    }

    pub(super) fn move_down(&mut self, _: &SettingsMoveDown, window: &mut Window, cx: &mut Context<Self>) {
        let outcome = self.model.move_profile(false);
        self.outcome(outcome, window, cx);
    }

    /// Runs `f` on the profile editor, then previews and redraws.
    pub(super) fn edit_profile(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut ProfileEditor),
    ) {
        if let Some(editor) = self.model.profiles.editor_mut() {
            f(editor);
            self.model.error = None;
        }
        self.changed(window, cx);
    }

    // ---- rendering ----

    pub(super) fn render_profiles(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        match &self.model.profiles.mode {
            Mode::Editor(editor) => {
                let editor = editor.clone();
                self.render_profile_editor(&editor, cx)
            }
            _ => self.render_profile_list(window, cx),
        }
    }

    fn render_profile_list(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let t = *cx.ui_theme();
        let fg = t.foreground;
        let muted = t.muted_foreground;
        let state = &self.model.profiles;
        let default = state.default_index();

        let rows = state
            .entries
            .iter()
            .enumerate()
            .map(|(ix, entry)| {
                let selected = ix == state.selected;
                let detail = match &entry.profile {
                    Ok(p) => p.description(),
                    Err(e) => e.clone(),
                };
                div()
                    .id(("profile", ix))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py_1p5()
                    .rounded_md()
                    .when(selected, |d| d.bg(t.accent))
                    .when(!selected, |d| d.hover(|d| d.bg(t.accent.opacity(0.5))))
                    .on_click(cx.listener(move |this, e: &gpui::ClickEvent, window, cx| {
                        this.model.select(ix);
                        if e.click_count() >= 2 {
                            this.model.start_edit();
                        }
                        this.changed(window, cx);
                    }))
                    .child(
                        div()
                            .w(px(20.0))
                            .flex_none()
                            .text_xs()
                            .text_color(muted)
                            .child(if ix < 9 { SharedString::from(format!("{}", ix + 1)) } else { SharedString::default() }),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_color(if entry.profile.is_ok() { fg } else { muted })
                                    .child(SharedString::from(entry.name.clone())),
                            )
                            .child(div().text_xs().text_color(muted).child(SharedString::from(detail))),
                    )
                    .when(ix == default, |d| d.child(div().flex_none().text_xs().text_color(t.primary).child("● default")))
            })
            .collect::<Vec<_>>();

        let confirming = matches!(state.mode, Mode::ConfirmDelete);
        let footer = if confirming {
            self.confirm_footer(state.delete_question(), cx)
        } else {
            let outline = ButtonVariant::Outline;
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .gap_2()
                .child(self.button(Btn::New, "New (n)", outline, cx, |this, window, cx| {
                    this.model.start_new();
                    this.changed(window, cx);
                }))
                .child(self.button(Btn::Duplicate, "Duplicate (c)", outline, cx, |this, window, cx| {
                    this.model.start_duplicate();
                    this.changed(window, cx);
                }))
                .child(self.button(Btn::Edit, "Edit (e)", outline, cx, |this, window, cx| {
                    this.model.start_edit();
                    this.changed(window, cx);
                }))
                .child(self.button(Btn::Delete, "Delete (d)", ButtonVariant::Destructive, cx, |this, window, cx| {
                    this.model.request_delete();
                    this.changed(window, cx);
                }))
                .child(self.button(Btn::Up, "↑", outline, cx, |this, window, cx| {
                    let outcome = this.model.move_profile(true);
                    this.outcome(outcome, window, cx);
                }))
                .child(self.button(Btn::Down, "↓", outline, cx, |this, window, cx| {
                    let outcome = this.model.move_profile(false);
                    this.outcome(outcome, window, cx);
                }))
                .child(div().flex_1())
                .child(self.button(Btn::Close, "Close", ButtonVariant::Default, cx, |_, _, cx| {
                    cx.emit(gpui::DismissEvent)
                }))
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap_2()
            .child(div().text_xs().text_color(muted).child(
                "ctrl-alt-N opens profile N; alt-up/alt-down reorder. Changes are saved as you make them.",
            ))
            .when_some(state.read_only.clone(), |d, e| {
                d.child(div().text_xs().text_color(t.destructive).child(SharedString::from(format!("Read-only: {e}"))))
            })
            .child(self.list_frame(
                div()
                    .id("profile-list")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.profile_list_scroll)
                    .children(rows),
                window,
                cx,
            ))
            .child(footer)
            .into_any_element()
    }

    fn render_profile_editor(&self, editor: &ProfileEditor, cx: &mut Context<Self>) -> AnyElement {
        let t = *cx.ui_theme();
        let muted = t.muted_foreground;
        let red = t.destructive;
        let errors = match self.model.profiles.editor_result() {
            Some(Err(errors)) => errors,
            _ => Vec::new(),
        };
        let error_for = |field: Field| errors.iter().find(|(f, _)| *f == field).map(|(_, e)| e.clone());
        let label = |text: &'static str| div().w(px(120.0)).flex_none().text_color(muted).child(text);

        let row = |this: &Self, field: Field, cx: &mut Context<Self>| -> AnyElement {
            let error = error_for(field);
            let control: AnyElement = match field {
                f if SELECT_FIELDS.contains(&f) => {
                    this.choice_slot(Choice::Profile(f)).flex_1().min_w_0().into_any_element()
                }
                Field::Default => this.render_default_toggle(editor, cx),
                _ => div()
                    .flex_1()
                    .min_w_0()
                    .children(this.profile_input(field).map(|input| Input::new(input).invalid(error.is_some())))
                    .into_any_element(),
            };
            div()
                .flex()
                .flex_col()
                .flex_none()
                .gap_0p5()
                .child(div().flex().items_center().gap_2().child(label(field.label())).child(control))
                .when_some(error, |d, e| {
                    d.child(div().pl(px(128.0)).text_xs().text_color(red).child(SharedString::from(e)))
                })
                .into_any_element()
        };

        let visible = editor.visible_fields();
        let name_row = row(self, Field::Name, cx);
        let rows: Vec<AnyElement> = visible.iter().skip(1).map(|&f| row(self, f, cx)).collect();
        let title = match &editor.original_name {
            Some(name) => format!("Edit {name:?}"),
            None => "New profile".to_string(),
        };
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap_2()
            .child(div().text_color(t.foreground).child(SharedString::from(title)))
            .child(name_row)
            .child(
                div()
                    .id("profile-fields")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .gap_1()
                    .overflow_y_scroll()
                    .track_scroll(&self.profile_editor_scroll)
                    .children(rows),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().min_w_0().text_xs().text_color(muted).child(
                        "Theme and font size show on this profile's tabs now; the rest applies to new tabs.",
                    ))
                    .child(self.editor_buttons(cx)),
            )
            .into_any_element()
    }

    /// The default checkbox and its note. The current default can't be
    /// unticked, so its checkbox is disabled.
    fn render_default_toggle(&self, editor: &ProfileEditor, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.ui_theme().muted_foreground;
        let locked = editor.default_locked();
        let note = if locked {
            "New tabs use this profile. Make another profile the default to change it."
        } else {
            "Use this profile for new tabs and windows"
        };
        let toggle = |this: &mut Self, window: &mut Window, cx: &mut Context<Self>| {
            this.edit_profile(window, cx, |e| e.toggle_default());
        };
        div()
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .gap_2()
            .child(
                Checkbox::new("profile-default", editor.is_default())
                    .disabled(locked)
                    .focus(&self.default_handle)
                    .on_click(cx.listener(move |this, _: &bool, window, cx| toggle(this, window, cx))),
            )
            .child(
                div()
                    .id("profile-default-note")
                    .text_xs()
                    .text_color(muted)
                    // The note is the checkbox's label: a click on it toggles too.
                    .when(!locked, |d| {
                        d.cursor_pointer().on_click(cx.listener(move |this, _, window, cx| {
                            window.focus(&this.default_handle);
                            toggle(this, window, cx)
                        }))
                    })
                    .child(note),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    const CONFIG: &str = r#"# test config
default_profile = "work"

# the shell
[[profiles]]
name = "Shell"

[[profiles]]
name = "tmux"
tmux = "control"
tmux_session = "main"

# my work profile
[[profiles]]
name = "Work"
cwd = "~/src"
env = { PAGER = "less -R" }
args = ["-c", "echo hi; exec bash"]
theme = "gone"
"#;

    const THEMES: &[&str] = &["brindle-dark", "tokyo-night", "mine"];

    struct Fixture {
        dir: std::path::PathBuf,
        path: std::path::PathBuf,
    }

    impl Fixture {
        fn new(name: &str, text: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("brindle-profiles-{}-{name}", std::process::id()));
            std::fs::remove_dir_all(&dir).ok();
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("config.toml");
            std::fs::write(&path, text).unwrap();
            Self { dir, path }
        }

        fn state(&self) -> ProfilesState {
            ProfilesState::new(&self.path, &[], None)
        }

        fn text(&self) -> String {
            std::fs::read_to_string(&self.path).unwrap()
        }

        fn config(&self) -> Config {
            Config::parse(&self.text()).unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.dir).ok();
        }
    }

    fn themes() -> Vec<String> {
        THEMES.iter().map(|t| t.to_string()).collect()
    }

    fn names(state: &ProfilesState) -> Vec<&str> {
        state.entries.iter().map(|e| e.name.as_str()).collect()
    }

    fn edit<'a>(state: &'a mut ProfilesState, name: &str) -> &'a mut ProfileEditor {
        state.select(state.entries.iter().position(|e| e.name == name).unwrap());
        state.start_edit(&themes()).unwrap();
        state.editor_mut().unwrap()
    }

    fn invalid(state: &ProfilesState) -> Vec<Field> {
        match state.editor_result().unwrap() {
            Ok(_) => Vec::new(),
            Err(errors) => errors.into_iter().map(|(f, _)| f).collect(),
        }
    }

    #[test]
    fn list_order_default_and_initial_selection() {
        let f = Fixture::new("list", CONFIG);
        let state = f.state();
        assert_eq!(names(&state), ["Shell", "tmux", "Work"]);
        assert_eq!(state.default_index(), 2);
        assert_eq!(state.selected, 2);
        assert!(state.describe(&None, "").contains("default=\"Work\""));
    }

    #[test]
    fn implicit_unnamed_unreadable_and_broken_files() {
        let f = Fixture::new("implicit", "theme = \"mine\"\n");
        let state = f.state();
        assert_eq!(names(&state), ["Shell"]);
        assert_eq!(state.default_index(), 0);
        // An unknown key elsewhere doesn't hide the profiles; a bad profile
        // is listed with its error and can't be edited.
        let f = Fixture::new("errors", "fontsize = 3\n[[profiles]]\nname = \"Work\"\n[[profiles]]\nfont_size = \"big\"\n[[profiles]]\ncwd = \"~\"\n");
        let mut state = f.state();
        assert_eq!(names(&state), ["Work", "Profile 2", "Profile 3"]);
        assert!(state.entries[1].profile.is_err());
        state.select(1);
        assert!(state.start_edit(&themes()).unwrap_err().contains("can't be read"));
        assert!(edit(&mut state, "Work").dirty() == false);
        // A file that isn't TOML lists the loaded profiles, read-only.
        let f = Fixture::new("broken", "[oops\n");
        let mut state = ProfilesState::new(&f.path, &[Profile { name: "Loaded".into(), ..Default::default() }], None);
        assert_eq!(names(&state), ["Loaded"]);
        assert!(state.start_edit(&themes()).unwrap_err().contains("not valid TOML"));
        assert!(state.start_new(&themes()).is_err());
    }

    #[test]
    fn fields_as_shell_words_round_trip() {
        let f = Fixture::new("words", CONFIG);
        let mut state = f.state();
        let editor = edit(&mut state, "Work");
        assert_eq!(editor.text(Field::Env), "PAGER='less -R'");
        assert_eq!(editor.text(Field::Args), "-c 'echo hi; exec bash'");
        assert_eq!(editor.theme(), Some("gone"));
        // A missing theme stays choosable, after the known ones.
        assert_eq!(editor.theme_choices.last().unwrap().as_deref(), Some("gone"));
        assert!(editor.is_default() && editor.default_locked());
        // Saving without changes keeps every value.
        state.save(&f.path, &[]).unwrap();
        let work = f.config().profiles[2].clone();
        assert_eq!(work.args, ["-c", "echo hi; exec bash"]);
        assert_eq!(work.env["PAGER"], "less -R");
        assert_eq!(work.theme.as_deref(), Some("gone"));
        assert!(f.text().contains("# my work profile\n[[profiles]]\nname = \"Work\""), "{}", f.text());
    }

    #[test]
    fn validation() {
        let f = Fixture::new("validation", CONFIG);
        let mut state = f.state();
        let editor = edit(&mut state, "Work");
        editor.set_text(Field::Name, "shell");
        assert_eq!(invalid(&state), [Field::Name]);
        let editor = state.editor_mut().unwrap();
        editor.set_text(Field::Name, "  ");
        editor.set_text(Field::Args, "-c \"echo hi");
        editor.set_text(Field::Env, "EDITOR=nvim verbose");
        editor.set_text(Field::FontSize, "200");
        assert_eq!(invalid(&state), [Field::Name, Field::Args, Field::Env, Field::FontSize]);
        let editor = state.editor_mut().unwrap();
        editor.set_text(Field::Name, "Work");
        editor.set_text(Field::Args, "");
        editor.set_text(Field::Env, "1X=a");
        editor.set_text(Field::FontSize, "x");
        assert_eq!(invalid(&state), [Field::Env, Field::FontSize]);
        let before = f.text();
        assert!(state.save(&f.path, &[]).is_err());
        assert_eq!(f.text(), before);
        // Keeping its own name, in another case, is fine.
        let editor = state.editor_mut().unwrap();
        editor.set_text(Field::Env, "");
        editor.set_text(Field::FontSize, "13.5");
        editor.set_text(Field::Name, "WORK");
        assert!(invalid(&state).is_empty());
    }

    #[test]
    fn tmux_fields_follow_the_mode() {
        let f = Fixture::new("tmux", CONFIG);
        let mut state = f.state();
        let editor = edit(&mut state, "Shell");
        assert!(!editor.visible_fields().contains(&Field::Session));
        editor.choose(Field::Tmux, 1);
        assert_eq!(editor.tmux(), TmuxMode::Plain);
        assert!(editor.visible_fields().contains(&Field::Session));
        editor.set_text(Field::Session, "dev");
        // Hidden again, the values are kept but not saved.
        editor.choose(Field::Tmux, 0);
        assert_eq!(editor.text(Field::Session), "dev");
        editor.choose(Field::Tmux, 2);
        assert_eq!(editor.tmux(), TmuxMode::Control);
        // After the tmux arguments comes the default checkbox.
        let visible = editor.visible_fields();
        let at = visible.iter().position(|f| *f == Field::TmuxArgs).unwrap();
        assert_eq!(visible[at + 1], Field::Default);
        editor.choose(Field::Tmux, 0);
        assert_eq!(editor.tmux(), TmuxMode::None);
        let visible = editor.visible_fields();
        let at = visible.iter().position(|f| *f == Field::Tmux).unwrap();
        assert_eq!(visible[at + 1], Field::Default);
        state.save(&f.path, &[]).unwrap();
        let shell = f.config().profiles[0].clone();
        assert_eq!((shell.tmux, shell.tmux_session), (TmuxMode::None, None));
    }

    #[test]
    fn ssh_target_is_written_and_read_back() {
        let f = Fixture::new("remote-ssh", CONFIG);
        let mut state = f.state();
        let editor = edit(&mut state, "Work");
        assert_eq!(editor.remote_kind(), RemoteKind::Local);
        assert!(!editor.visible_fields().contains(&Field::SshHost));
        editor.choose(Field::Remote, 1);
        assert_eq!(editor.remote_kind(), RemoteKind::Ssh);
        assert!(editor.visible_fields().contains(&Field::SshHost));
        assert!(!editor.visible_fields().contains(&Field::Container));
        editor.set_text(Field::SshHost, "A8");
        editor.set_text(Field::SshUser, "ubuntu");
        editor.set_text(Field::SshPort, "2222");
        editor.set_text(Field::SshArgs, "-J 'my bastion'");
        state.save(&f.path, &[]).unwrap();
        match &f.config().profiles[2].remote {
            Some(RemoteTarget::Ssh { host, user, port, identity_file, ssh_args }) => {
                assert_eq!((host.as_str(), user.as_deref(), *port), ("A8", Some("ubuntu"), Some(2222)));
                assert_eq!(identity_file, &None);
                assert_eq!(ssh_args, &["-J", "my bastion"]);
            }
            other => panic!("expected ssh, got {other:?}"),
        }
        // Opening it again shows the same values.
        let editor = edit(&mut state, "Work");
        assert_eq!(editor.remote_kind(), RemoteKind::Ssh);
        assert_eq!(editor.text(Field::SshPort), "2222");
        assert_eq!(editor.text(Field::SshArgs), "-J 'my bastion'");
    }

    #[test]
    fn container_target_and_removal() {
        let f = Fixture::new("remote-container", CONFIG);
        let mut state = f.state();
        let editor = edit(&mut state, "Work");
        editor.choose(Field::Remote, 2);
        assert_eq!(editor.remote_kind(), RemoteKind::Container);
        editor.set_text(Field::Container, "my-box");
        editor.choose(Field::Engine, 1);
        state.save(&f.path, &[]).unwrap();
        assert_eq!(
            f.config().profiles[2].remote,
            Some(RemoteTarget::Container { engine: "podman".into(), container: "my-box".into(), user: None })
        );
        let editor = edit(&mut state, "Work");
        assert_eq!((editor.remote_kind(), editor.engine()), (RemoteKind::Container, "podman"));
        editor.choose_remote(RemoteKind::Local);
        state.save(&f.path, &[]).unwrap();
        assert_eq!(f.config().profiles[2].remote, None);
        assert!(!f.text().contains("remote"));
    }

    #[test]
    fn remote_validation() {
        let f = Fixture::new("remote-validation", CONFIG);
        let mut state = f.state();
        let editor = edit(&mut state, "Work");
        editor.choose_remote(RemoteKind::Ssh);
        assert_eq!(invalid(&state), [Field::SshHost]);
        let editor = state.editor_mut().unwrap();
        editor.set_text(Field::SshHost, "A8");
        editor.set_text(Field::SshPort, "70000");
        assert_eq!(invalid(&state), [Field::SshPort]);
        let editor = state.editor_mut().unwrap();
        editor.set_text(Field::SshPort, "0");
        assert_eq!(invalid(&state), [Field::SshPort]);
        let editor = state.editor_mut().unwrap();
        editor.set_text(Field::SshPort, "");
        assert!(invalid(&state).is_empty());
        let editor = state.editor_mut().unwrap();
        editor.choose_remote(RemoteKind::Container);
        assert_eq!(invalid(&state), [Field::Container]);
    }

    /// A select's index is the option the editor shows, and choosing an
    /// index sets it.
    #[test]
    fn choice_indexes() {
        let f = Fixture::new("choices", CONFIG);
        let mut state = f.state();
        let editor = edit(&mut state, "Work");
        // Follow global, the three themes, then the missing `gone`.
        assert_eq!(editor.choice_count(Field::Theme), 5);
        assert_eq!(editor.choice_index(Field::Theme), Some(4));
        editor.choose(Field::Theme, 0);
        assert_eq!((editor.theme(), editor.choice_index(Field::Theme)), (None, Some(0)));
        editor.choose(Field::Theme, 4);
        assert_eq!(editor.theme(), Some("gone"));
        editor.choose(Field::Theme, 2);
        assert_eq!(editor.theme(), Some("tokyo-night"));
        // Out of range changes nothing.
        editor.choose(Field::Theme, 9);
        assert_eq!(editor.theme(), Some("tokyo-night"));
        for field in [Field::Tmux, Field::Remote, Field::Engine] {
            assert_eq!(editor.choice_index(field), Some(0), "{field:?}");
            for ix in 0..editor.choice_count(field) {
                editor.choose(field, ix);
                assert_eq!(editor.choice_index(field), Some(ix), "{field:?}");
            }
            editor.choose(field, 0);
        }
        // Choosing a remote kind shows its fields.
        editor.choose(Field::Remote, 1);
        assert_eq!(editor.remote_kind(), RemoteKind::Ssh);
        assert!(editor.visible_fields().contains(&Field::SshHost));
        // Text fields aren't choices.
        assert_eq!((editor.choice_count(Field::Cwd), editor.choice_index(Field::Cwd)), (0, None));
    }

    #[test]
    fn theme_and_font_size_preview() {
        let f = Fixture::new("preview", CONFIG);
        let mut state = f.state();
        assert!(state.preview().is_none());
        let editor = edit(&mut state, "Shell");
        // Shell follows the global theme (option 0); option 1 is brindle-dark.
        editor.choose(Field::Theme, 1);
        editor.set_text(Field::FontSize, "18");
        let (name, profile) = state.preview().unwrap();
        assert_eq!((name.as_str(), profile.theme.as_deref(), profile.font_size), ("Shell", Some("brindle-dark"), Some(18.0)));
        // An invalid size keeps the one it had.
        state.editor_mut().unwrap().set_text(Field::FontSize, "1");
        assert_eq!(state.preview().unwrap().1.font_size, None);
        state.discard();
        assert!(state.preview().is_none());
        assert_eq!(f.text(), CONFIG);
        // New profiles have no tabs to preview on.
        state.start_new(&themes()).unwrap();
        assert!(state.preview().is_none());
    }

    #[test]
    fn rename_the_default() {
        let f = Fixture::new("rename", CONFIG);
        let mut state = f.state();
        edit(&mut state, "Work").set_text(Field::Name, "Job");
        assert!(matches!(state.save(&f.path, &[]).unwrap(), Done::Written));
        let config = f.config();
        assert_eq!(config.default_profile.as_deref(), Some("Job"));
        assert_eq!(config.profiles[2].name, "Job");
        assert_eq!(state.take_renames(), [("Work".to_string(), "Job".to_string())]);
        assert!(state.take_renames().is_empty());
        assert_eq!(state.selected_entry().unwrap().name, "Job");
        assert!(matches!(state.mode, Mode::List));
    }

    #[test]
    fn make_default() {
        let f = Fixture::new("default", CONFIG);
        let mut state = f.state();
        let editor = edit(&mut state, "tmux");
        assert!(!editor.is_default());
        editor.toggle_default();
        state.save(&f.path, &[]).unwrap();
        assert_eq!(f.config().default_profile.as_deref(), Some("tmux"));
        // The default can't be unchecked.
        let editor = edit(&mut state, "tmux");
        editor.toggle_default();
        assert!(editor.is_default());
    }

    #[test]
    fn new_and_duplicate() {
        let f = Fixture::new("new", CONFIG);
        let mut state = f.state();
        state.start_new(&themes()).unwrap();
        assert_eq!(state.editor().unwrap().text(Field::Name), "Profile 4");
        state.discard();
        state.select(2);
        state.start_duplicate(&themes()).unwrap();
        let editor = state.editor().unwrap();
        assert_eq!(editor.text(Field::Name), "Work copy");
        assert!(!editor.is_default());
        assert_eq!(editor.text(Field::Cwd), "~/src");
        state.save(&f.path, &[]).unwrap();
        assert_eq!(names(&state), ["Shell", "tmux", "Work", "Work copy"]);
        assert_eq!(state.selected_entry().unwrap().name, "Work copy");
        assert_eq!(f.config().default_profile.as_deref(), Some("work"));
        state.select(2);
        state.start_duplicate(&themes()).unwrap();
        assert_eq!(state.editor().unwrap().text(Field::Name), "Work copy 2");
    }

    #[test]
    fn first_profile_on_an_empty_config() {
        let f = Fixture::new("empty", "# nothing\n");
        let mut state = f.state();
        state.start_new(&themes()).unwrap();
        state.editor_mut().unwrap().set_text(Field::Name, "Work");
        state.save(&f.path, &[]).unwrap();
        assert_eq!(names(&state), ["Shell", "Work"]);
    }

    #[test]
    fn deleting() {
        let f = Fixture::new("delete", CONFIG);
        let mut state = f.state();
        state.select(2);
        state.request_delete().unwrap();
        assert!(state.delete_question().contains("default profile"));
        state.delete_selected(&f.path, &[]).unwrap();
        assert_eq!(names(&state), ["Shell", "tmux"]);
        assert_eq!(f.config().default_profile, None);
        assert!(f.text().starts_with("# test config\n"));
        state.select(0);
        assert_eq!(state.delete_question(), "Delete \"Shell\"?");
        state.delete_selected(&f.path, &[]).unwrap();
        assert!(state.request_delete().unwrap_err().contains("last profile"));
        assert_eq!(names(&state), ["tmux"]);
    }

    #[test]
    fn moving() {
        let f = Fixture::new("move", CONFIG);
        let mut state = f.state();
        state.select(2);
        assert!(matches!(state.move_selected(true, &f.path, &[]).unwrap(), Done::Written));
        assert_eq!(names(&state), ["Shell", "Work", "tmux"]);
        assert_eq!(state.selected, 1);
        assert!(f.text().contains("# my work profile\n[[profiles]]\nname = \"Work\""));
        state.select(0);
        assert!(matches!(state.move_selected(true, &f.path, &[]).unwrap(), Done::Nothing));
        assert_eq!(f.config().profiles[1].name, "Work");
    }

    #[test]
    fn file_changed_underneath() {
        let f = Fixture::new("stale", CONFIG);
        let mut state = f.state();
        edit(&mut state, "Work").set_text(Field::Cwd, "~");
        std::fs::write(&f.path, CONFIG.replace("name = \"Work\"", "name = \"Other\"")).unwrap();
        let before = f.text();
        let err = state.save(&f.path, &[]).unwrap_err();
        assert!(err.contains("reopen the Profiles section"), "{err}");
        assert_eq!(f.text(), before);
        assert!(state.editor().is_some());
    }
}
