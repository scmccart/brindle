//! The settings dialog's Profiles section: the profile list, and an editor
//! for every profile setting. Saving writes one profile's table with
//! `config_edit`; theme and font size preview on the profile's open tabs.
//!
//! [`ProfilesState`] holds the state and logic and has no GPUI dependency:
//! it reads the profiles from the config file's text it is given and
//! returns the [`ConfigEdit`] to write, never touching the file. The view
//! half, an `impl SettingsDialog`, follows it.

use std::path::Path;

use crate::config::{Profile, ProfileKey, RemoteTarget};
use crate::config_edit::{self, ConfigEdit, DefaultChange, FileText, IMPLICIT_PROFILE, ProfileRef};

use super::{ListMode, Step};

/// A profile in the list.
#[derive(Clone, Debug)]
pub struct ProfileEntry {
    /// The name it is listed (and looked up) by.
    pub name: String,
    /// Its `name` key as the file has it, which a write checks is still
    /// there: `None` when it has none (or a blank one).
    raw: Option<String>,
    /// A profile that can't be read carries its error.
    pub profile: Result<Profile, String>,
}

/// A row of the profile list, as the view draws it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProfileRow {
    pub name: String,
    /// The profile's description, or why it can't be read.
    pub detail: String,
    pub readable: bool,
    /// The digit of its ctrl-alt-N shortcut, for the first nine.
    pub hint: Option<String>,
    pub selected: bool,
    pub default: bool,
}

/// What the profile editor edits: a new profile, or the saved one with
/// this key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    New,
    Saved(ProfileKey),
}

impl Target {
    /// The saved profile's listed name.
    pub fn name(&self) -> Option<&str> {
        match self {
            Target::New => None,
            Target::Saved(key) => Some(&key.name),
        }
    }
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
    ConnectAtLaunch,
}

pub const FIELDS: [Field; 18] = [
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
    Field::ConnectAtLaunch,
    Field::Command,
    Field::Args,
    Field::Cwd,
    Field::Env,
    Field::Theme,
    Field::FontSize,
    Field::Default,
];

/// The fields edited as text, in [`Values::texts`] order.
pub const TEXT_FIELDS: [Field; 13] = [
    Field::Name,
    Field::Command,
    Field::Args,
    Field::Cwd,
    Field::Env,
    Field::FontSize,
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
            Field::ConnectAtLaunch => "Connect at launch",
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
            Field::SshHost => "hostname or ssh config alias",
            Field::SshUser | Field::ContainerUser => "default user",
            Field::SshPort => "22",
            Field::SshIdentity => "default key",
            Field::SshArgs => "shell words, e.g. -J bastion",
            Field::Container => "name or id",
            Field::Name | Field::Theme | Field::Default | Field::Remote | Field::Engine | Field::ConnectAtLaunch => "",
        }
    }

    fn ssh_only(self) -> bool {
        matches!(self, Field::SshHost | Field::SshUser | Field::SshPort | Field::SshIdentity | Field::SshArgs)
    }

    fn container_only(self) -> bool {
        matches!(self, Field::Engine | Field::Container | Field::ContainerUser)
    }

    /// Shown for both kinds of remote target, hidden for a local profile.
    fn remote_only(self) -> bool {
        matches!(self, Field::ConnectAtLaunch)
    }

    pub fn text_index(self) -> Option<usize> {
        TEXT_FIELDS.iter().position(|f| *f == self)
    }
}

/// A word from the file that can't be written as a shell word (one with a
/// NUL character), so the field showing it can't give it back.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unquotable {
    word: String,
}

impl std::fmt::Display for Unquotable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?} can't be written as a shell word", self.word)
    }
}

/// Words joined so `shlex::split` gives them back, or the first word that
/// can't be.
fn join_words(words: &[String]) -> Result<String, Unquotable> {
    words.iter().map(|word| quote(word)).collect::<Result<Vec<_>, _>>().map(|quoted| quoted.join(" "))
}

fn quote(word: &str) -> Result<String, Unquotable> {
    shlex::try_quote(word).map(|q| q.into_owned()).map_err(|_| Unquotable { word: word.to_string() })
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
    remote: RemoteKind,
    /// `docker` or `podman`.
    engine: String,
    /// The target's `connect_at_launch`, kept as it was: a profile saved
    /// from the editor keeps it, whichever kind of endpoint it is on now.
    connect_at_launch: bool,
    default: bool,
}

impl Values {
    /// The values `p` shows, and the fields whose words can't be written as
    /// shell words: those show the words joined plainly, and can't be saved
    /// as they are (see [`ProfileEditor::result`]).
    fn from_profile(p: &Profile, default: bool) -> (Self, Vec<(Field, Unquotable)>) {
        let mut unquotable = Vec::new();
        let mut shown = |field: Field, words: Result<String, Unquotable>, plain: String| match words {
            Ok(words) => words,
            Err(reason) => {
                unquotable.push((field, reason));
                plain
            }
        };
        let env = p
            .env
            .iter()
            .map(|(k, v)| quote(v).map(|v| format!("{k}={v}")))
            .collect::<Result<Vec<_>, _>>()
            .map(|env| env.join(" "));
        let env_plain = p.env.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join(" ");
        let args = shown(Field::Args, join_words(&p.args), p.args.join(" "));
        let env = shown(Field::Env, env, env_plain);
        let mut texts = [
            p.name.clone(),
            p.command.clone().unwrap_or_default(),
            args,
            p.cwd.clone().unwrap_or_default(),
            env,
            p.font_size.map(|s| s.to_string()).unwrap_or_default(),
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
        let connect_at_launch = p.remote.as_ref().is_some_and(RemoteTarget::connects_at_launch);
        let kind = match &p.remote {
            None => RemoteKind::Local,
            Some(RemoteTarget::Ssh { host, user, port, identity_file, ssh_args, connect_at_launch: _ }) => {
                texts[at(Field::SshHost)] = host.clone();
                texts[at(Field::SshUser)] = user.clone().unwrap_or_default();
                texts[at(Field::SshPort)] = port.map(|p| p.to_string()).unwrap_or_default();
                texts[at(Field::SshIdentity)] = identity_file.clone().unwrap_or_default();
                texts[at(Field::SshArgs)] = shown(Field::SshArgs, join_words(ssh_args), ssh_args.join(" "));
                RemoteKind::Ssh
            }
            Some(RemoteTarget::Container { engine: e, container, user, connect_at_launch: _ }) => {
                engine = e.clone();
                texts[at(Field::Container)] = container.clone();
                texts[at(Field::ContainerUser)] = user.clone().unwrap_or_default();
                RemoteKind::Container
            }
        };
        (Self { texts, theme: p.theme.clone(), remote: kind, engine, connect_at_launch, default }, unquotable)
    }

    fn text(&self, field: Field) -> &str {
        field.text_index().map_or("", |ix| &self.texts[ix])
    }
}

/// The profile editor.
#[derive(Clone, Debug)]
pub struct ProfileEditor {
    /// The profile being edited; its key's name is the listed name, for
    /// renames and the preview.
    pub target: Target,
    /// The list selection to go back to on discard.
    return_to: Option<ProfileKey>,
    initial: Values,
    values: Values,
    /// The fields whose words from the file can't be written as shell
    /// words, until the field is edited. Each blocks saving, since the text
    /// it shows isn't what the file has.
    unquotable: Vec<(Field, Unquotable)>,
    /// "Follow global" (`None`), then every theme by name.
    pub theme_choices: Vec<Option<String>>,
}

impl ProfileEditor {
    fn new(profile: &Profile, target: Target, default: bool, return_to: Option<ProfileKey>, themes: &[String]) -> Self {
        let (values, unquotable) = Values::from_profile(profile, default);
        let mut theme_choices: Vec<Option<String>> = std::iter::once(None).chain(themes.iter().cloned().map(Some)).collect();
        // A theme that no longer exists stays choosable, so opening and
        // saving doesn't change it.
        if let Some(theme) = &profile.theme
            && !themes.contains(theme)
        {
            theme_choices.push(Some(theme.clone()));
        }
        Self { target, return_to, initial: values.clone(), values, unquotable, theme_choices }
    }

    /// The editor's heading.
    pub fn title(&self) -> String {
        match self.target.name() {
            Some(name) => format!("Edit {name:?}"),
            None => "New profile".to_string(),
        }
    }

    /// The note beside the default checkbox: the current default can't be
    /// unticked.
    pub fn default_note(&self) -> &'static str {
        if self.default_locked() {
            "New tabs use this profile. Make another profile the default to change it."
        } else {
            "Use this profile for new tabs and windows"
        }
    }

    /// The theme select's labels: "follow global", then each theme, marked
    /// missing unless it is one of the `known` themes.
    pub fn theme_labels(&self, known: &[String]) -> Vec<String> {
        self.theme_choices
            .iter()
            .map(|theme| match theme {
                None => "follow global".to_string(),
                Some(name) if known.contains(name) => name.clone(),
                Some(name) => format!("{name} (missing)"),
            })
            .collect()
    }

    /// The options of select field `field`, in the editor's order, with the
    /// `known` themes; none for a field that isn't a select.
    pub fn choice_labels(&self, field: Field, known: &[String]) -> Vec<String> {
        match field {
            Field::Theme => self.theme_labels(known),
            Field::Remote => REMOTE_KINDS.iter().map(|&k| remote_label(k).to_string()).collect(),
            Field::Engine => ENGINES.iter().map(|e| e.to_string()).collect(),
            Field::Name
            | Field::Command
            | Field::Args
            | Field::Cwd
            | Field::Env
            | Field::FontSize
            | Field::Default
            | Field::SshHost
            | Field::SshUser
            | Field::SshPort
            | Field::SshIdentity
            | Field::SshArgs
            | Field::Container
            | Field::ContainerUser
            | Field::ConnectAtLaunch => Vec::new(),
        }
    }

    pub fn text(&self, field: Field) -> &str {
        self.values.text(field)
    }

    /// Sets a text field. Editing a field whose words couldn't be written
    /// as shell words replaces them, so it is valid again.
    pub fn set_text(&mut self, field: Field, text: &str) {
        if let Some(ix) = field.text_index() {
            if self.values.texts[ix] != text {
                self.unquotable.retain(|(unquotable, _)| *unquotable != field);
            }
            self.values.texts[ix] = text.to_string();
        }
    }

    pub fn theme(&self) -> Option<&str> {
        self.values.theme.as_deref()
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

    /// The fields shown: the target's fields only for its kind.
    pub fn visible_fields(&self) -> Vec<Field> {
        FIELDS
            .into_iter()
            .filter(|f| !f.ssh_only() || self.values.remote == RemoteKind::Ssh)
            .filter(|f| !f.container_only() || self.values.remote == RemoteKind::Container)
            .filter(|f| !f.remote_only() || self.values.remote != RemoteKind::Local)
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
            Field::Remote => REMOTE_KINDS.len(),
            Field::Engine => ENGINES.len(),
            _ => 0,
        }
    }

    /// The option choice field `field` shows, as an index into its options
    /// (`theme_choices`, [`REMOTE_KINDS`], [`ENGINES`]).
    #[allow(
        clippy::wildcard_enum_match_arm,
        reason = "railway R3 exception: `Field` has 17 variants and only the three select fields have options"
    )]
    pub fn choice_index(&self, field: Field) -> Option<usize> {
        match field {
            Field::Theme => self.theme_choices.iter().position(|t| *t == self.values.theme),
            Field::Remote => REMOTE_KINDS.iter().position(|k| *k == self.values.remote),
            Field::Engine => ENGINES.iter().position(|e| *e == self.values.engine),
            _ => None,
        }
    }

    /// Chooses option `ix` of choice field `field`, as its select does. An
    /// index out of range changes nothing.
    #[allow(
        clippy::wildcard_enum_match_arm,
        reason = "railway R3 exception: `Field` has 17 variants and only the three select fields have options"
    )]
    pub fn choose(&mut self, field: Field, ix: usize) {
        match field {
            Field::Theme => {
                if let Some(theme) = self.theme_choices.get(ix) {
                    self.values.theme = theme.clone();
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

    /// The "Connect at launch" switch. It is kept, not shown, while the
    /// profile is local, so choosing a remote kind again finds it as it was.
    pub fn connect_at_launch(&self) -> bool {
        self.values.connect_at_launch
    }

    pub fn toggle_connect_at_launch(&mut self) {
        self.values.connect_at_launch = !self.values.connect_at_launch;
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
        let words = |f: Field, errors: &mut Vec<(Field, String)>| {
            if let Some((_, reason)) = self.unquotable.iter().find(|(unquotable, _)| *unquotable == f) {
                errors.push((f, reason.to_string()));
                return Vec::new();
            }
            match split_words(self.values.text(f)) {
                Ok(words) => words,
                Err(e) => {
                    errors.push((f, e));
                    Vec::new()
                }
            }
        };

        let name = text(Field::Name);
        if name.is_empty() {
            errors.push((Field::Name, "the name can't be empty".into()));
        } else if others.iter().any(|o| o.eq_ignore_ascii_case(name)) {
            errors.push((Field::Name, format!("a profile named {name:?} already exists")));
        }
        let args = words(Field::Args, &mut errors);
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
                    connect_at_launch: self.values.connect_at_launch,
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
                    connect_at_launch: self.values.connect_at_launch,
                })
            }
        };
        if !errors.is_empty() {
            return Err(errors);
        }
        Ok(Profile {
            name: name.to_string(),
            remote,
            command: optional(Field::Command),
            args,
            cwd: optional(Field::Cwd),
            env,
            theme: self.values.theme.clone(),
            font_size,
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
#[allow(
    clippy::large_enum_variant,
    reason = "one value, the Profiles section's mode, held by the open dialog; boxing the editor adds an indirection to every field read"
)]
pub enum Mode {
    List,
    Editor(ProfileEditor),
    /// Asking whether to delete the selected profile.
    ConfirmDelete,
}

/// The Profiles section's state: the list, its selection by
/// [`ProfileKey`], and the editor or delete question when one shows.
pub struct ProfilesState {
    entries: Vec<ProfileEntry>,
    /// `default_profile` from the file.
    default_name: Option<String>,
    /// The selected profile; always one of the list's once it is read.
    selected: Option<ProfileKey>,
    mode: Mode,
    /// Why the list can't be edited (the file isn't valid TOML).
    pub read_only: Option<String>,
    /// Renames saved since the last [`Self::take_renames`].
    renames: Vec<(String, String)>,
    /// The profiles to list (read-only) when the file can't be read: the
    /// config in effect's.
    fallback: Vec<Profile>,
}

impl ProfilesState {
    /// The list read from `text` (the file at `path`), with the default
    /// profile selected.
    pub fn new(path: &Path, text: &FileText, fallback: Vec<Profile>, fallback_default: Option<String>) -> Self {
        let mut state = Self {
            entries: Vec::new(),
            default_name: None,
            selected: None,
            mode: Mode::List,
            read_only: None,
            renames: Vec::new(),
            fallback,
        };
        state.reload(path, text, fallback_default);
        state.select_index(state.default_index());
        state
    }

    /// Rereads the profiles from the file's text, or lists the fallback
    /// read-only (with `fallback_default` the default) when it can't be
    /// read: the message names the file and the cause. A file that doesn't
    /// exist reads as an empty one. The selection is kept by its key, or
    /// else by its place.
    pub fn reload(&mut self, path: &Path, text: &FileText, fallback_default: Option<String>) {
        let previous = self.selected_index().unwrap_or(0);
        let read = config_edit::document_in(path, text).and_then(|doc| {
            let read = config_edit::profiles(&doc).map_err(|shape| shape.describe(path))?;
            Ok((doc, read))
        });
        let read = match read {
            Ok((doc, read)) => {
                self.read_only = None;
                self.default_name = doc.get("default_profile").and_then(|i| i.as_str()).map(str::to_string);
                read
            }
            Err(e) => {
                self.read_only = Some(e);
                self.default_name = fallback_default;
                let named = |p: &Profile| Some(p.name.clone()).filter(|name| !name.trim().is_empty());
                self.fallback.iter().map(|p| (named(p), Ok(p.clone()))).collect()
            }
        };
        self.entries = read
            .into_iter()
            .enumerate()
            .map(|(index, (raw, profile))| {
                // Unnamed profiles are listed as `Profile N`, as `Config` names them.
                let name = raw.clone().unwrap_or_else(|| format!("Profile {}", index + 1));
                let profile = profile.map(|mut p| {
                    p.name = name.clone();
                    p
                });
                ProfileEntry { name, raw, profile }
            })
            .collect();
        if self.entries.is_empty() {
            self.entries.push(ProfileEntry {
                name: IMPLICIT_PROFILE.into(),
                raw: Some(IMPLICIT_PROFILE.into()),
                profile: Ok(Profile::default_shell()),
            });
        }
        self.reselect(previous);
    }

    fn names(&self) -> impl Iterator<Item = &str> + Clone {
        self.entries.iter().map(|e| e.name.as_str())
    }

    /// Keeps the selection when its key is still listed, or else selects
    /// the profile at `fallback` (the last when the list is shorter).
    fn reselect(&mut self, fallback: usize) {
        if self.selected_index().is_none() {
            self.select_index(fallback.min(self.entries.len() - 1));
        }
    }

    fn select_index(&mut self, ix: usize) {
        if let Some(key) = ProfileKey::of(self.names(), ix) {
            self.selected = Some(key);
        }
    }

    /// Where the selected profile is in the list.
    pub fn selected_index(&self) -> Option<usize> {
        self.selected.as_ref()?.find(self.names())
    }

    /// The listed profiles.
    #[cfg(test)]
    pub fn entries(&self) -> &[ProfileEntry] {
        &self.entries
    }

    pub fn list_mode(&self) -> ListMode {
        match self.mode {
            Mode::List => ListMode::List,
            Mode::Editor(_) => ListMode::Editor,
            Mode::ConfirmDelete => ListMode::ConfirmDelete,
        }
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
        self.entries.get(self.selected_index()?)
    }

    /// The list's rows, in order.
    pub fn rows(&self) -> Vec<ProfileRow> {
        let (selected, default) = (self.selected_index(), self.default_index());
        self.entries
            .iter()
            .enumerate()
            .map(|(ix, entry)| ProfileRow {
                name: entry.name.clone(),
                detail: match &entry.profile {
                    Ok(p) => p.description(),
                    Err(e) => e.clone(),
                },
                readable: entry.profile.is_ok(),
                hint: (ix < 9).then(|| (ix + 1).to_string()),
                selected: selected == Some(ix),
                default: ix == default,
            })
            .collect()
    }

    pub fn take_renames(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.renames)
    }

    fn names_except(&self, except: Option<&str>) -> Vec<&str> {
        self.names().filter(|n| Some(*n) != except).collect()
    }

    /// Where the profile `key` names is in the file, checked by the write:
    /// built only for that write, never kept.
    fn profile_ref(&self, key: &ProfileKey) -> Result<ProfileRef, String> {
        let index = key.find(self.names()).ok_or(config_edit::PROFILES_CHANGED)?;
        Ok(ProfileRef { index, name: self.entries[index].raw.clone() })
    }

    /// The name and settings to preview on the edited profile's open tabs.
    pub fn preview(&self) -> Option<(String, Profile)> {
        let Mode::Editor(editor) = &self.mode else { return None };
        let Target::Saved(key) = &editor.target else { return None };
        let base = self.entries.get(key.find(self.names())?)?.profile.as_ref().ok()?;
        Some((key.name.clone(), editor.preview(base)))
    }

    pub fn move_selection(&mut self, forward: bool) {
        match self.mode {
            Mode::List => {
                let n = self.entries.len();
                let ix = self.selected_index().unwrap_or(0);
                self.select_index(if forward { (ix + 1) % n } else { (ix + n - 1) % n });
            }
            // The editor's fields are focus stops: the view moves focus.
            Mode::Editor(_) | Mode::ConfirmDelete => {}
        }
    }

    pub fn select(&mut self, ix: usize) {
        if matches!(self.mode, Mode::List) && ix < self.entries.len() {
            self.select_index(ix);
        }
    }

    /// Home and end: the first profile, or the `last`.
    pub fn edge(&self, last: bool) -> Option<usize> {
        if last { self.entries.len().checked_sub(1) } else { Some(0) }
    }

    fn editable(&self) -> Result<(), String> {
        match &self.read_only {
            Some(e) => Err(e.clone()),
            None => Ok(()),
        }
    }

    fn open_editor(&mut self, profile: &Profile, target: Target, default: bool, themes: &[String]) {
        self.mode = Mode::Editor(ProfileEditor::new(profile, target, default, self.selected.clone(), themes));
    }

    /// Edit: the selected profile.
    pub fn start_edit(&mut self, themes: &[String]) -> Result<(), String> {
        self.editable()?;
        let entry = self.selected_entry().ok_or("no profile selected")?.clone();
        let key = self.selected.clone().ok_or("no profile selected")?;
        let profile = entry.profile.map_err(|e| format!("{:?} can't be read: {e}", entry.name))?;
        let default = self.selected_index() == Some(self.default_index());
        self.open_editor(&profile, Target::Saved(key), default, themes);
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
        self.open_editor(&Profile { name, ..Default::default() }, Target::New, false, themes);
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
        self.open_editor(&profile, Target::New, false, themes);
        Ok(())
    }

    pub fn editor(&self) -> Option<&ProfileEditor> {
        match &self.mode {
            Mode::Editor(editor) => Some(editor),
            Mode::List | Mode::ConfirmDelete => None,
        }
    }

    pub fn editor_mut(&mut self) -> Option<&mut ProfileEditor> {
        match &mut self.mode {
            Mode::Editor(editor) => Some(editor),
            Mode::List | Mode::ConfirmDelete => None,
        }
    }

    /// The editor's validation, with the other profiles' names.
    pub fn editor_result(&self) -> Option<Result<Profile, Vec<(Field, String)>>> {
        let editor = self.editor()?;
        Some(editor.result(&self.names_except(editor.target.name())))
    }

    /// Save: the profile's edit, made the default or following a rename of
    /// the default. The editor stays open until it is written
    /// ([`Self::written`]).
    pub fn save(&self) -> Result<Step, String> {
        let Some(Ok(profile)) = self.editor_result() else {
            return Err("Fix the fields marked invalid first".into());
        };
        let editor = self.editor().expect("editing");
        let at = match &editor.target {
            Target::New => None,
            Target::Saved(key) => Some(self.profile_ref(key)?),
        };
        let renamed = editor.target.name().filter(|old| *old != profile.name);
        let default = match (editor.is_default() && !editor.default_locked(), renamed) {
            (true, _) => DefaultChange::Set(profile.name.clone()),
            (false, Some(old)) => DefaultChange::Rename { old: old.to_string(), new: profile.name.clone() },
            (false, None) => DefaultChange::Keep,
        };
        Ok(Step::Write(ConfigEdit::SaveProfile { at, profile, default }))
    }

    /// Discard: back to the list as it was.
    pub fn discard(&mut self) {
        if let Mode::Editor(editor) = &self.mode {
            let fallback = self.selected_index().unwrap_or(0);
            self.selected = editor.return_to.clone();
            self.reselect(fallback);
        }
        self.mode = Mode::List;
    }

    /// Escape: closes in the list, discards in the editor, keeps the
    /// profile when asked about deleting it.
    pub fn cancel(&mut self) -> Step {
        match self.mode {
            Mode::List => Step::Close,
            Mode::Editor(_) => {
                self.discard();
                Step::Nothing
            }
            Mode::ConfirmDelete => {
                self.mode = Mode::List;
                Step::Nothing
            }
        }
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
        if self.selected_index() == Some(self.default_index()) && self.default_name.is_some() {
            format!("Delete {name:?}? It is the default profile; new tabs will use the first profile instead.")
        } else {
            format!("Delete {name:?}?")
        }
    }

    /// Delete, confirmed: back to the list, deleting the selected profile.
    pub fn delete_selected(&mut self) -> Result<Step, String> {
        self.mode = Mode::List;
        let entry = self.selected_entry().ok_or("no profile selected")?;
        let key = self.selected.as_ref().ok_or("no profile selected")?;
        Ok(Step::Write(ConfigEdit::DeleteProfile { at: self.profile_ref(key)?, name: entry.name.clone() }))
    }

    /// alt-up / alt-down: moves the selected profile one place, unless it
    /// is at that end.
    pub fn move_selected(&self, up: bool) -> Result<Step, String> {
        self.editable()?;
        if !matches!(self.mode, Mode::List) {
            return Ok(Step::Nothing);
        }
        let key = self.selected.as_ref().ok_or("no profile selected")?;
        let at = self.profile_ref(key)?;
        let edge = if up { at.index == 0 } else { at.index + 1 >= self.entries.len() };
        if edge {
            return Ok(Step::Nothing);
        }
        Ok(Step::Write(ConfigEdit::MoveProfile { at, up }))
    }

    /// The edit this section asked for was written (`applied` is what it
    /// gave), and the file now reads `text`: back to the list, with the
    /// saved profile selected, the one in a deleted profile's place, or the
    /// moved one.
    pub fn written(&mut self, edit: &ConfigEdit, applied: config_edit::Applied, path: &Path, text: &FileText) {
        match edit {
            ConfigEdit::SaveProfile { profile, .. } => {
                let old = self.editor().and_then(|e| e.target.name()).map(str::to_string);
                if let Some(old) = old.filter(|old| *old != profile.name) {
                    self.renames.push((old, profile.name.clone()));
                }
                self.mode = Mode::List;
                self.reload(path, text, None);
                self.select_index(self.entries.iter().position(|e| e.name == profile.name).unwrap_or(0));
            }
            ConfigEdit::DeleteProfile { .. } => self.reload(path, text, None),
            ConfigEdit::MoveProfile { at, .. } => {
                let moved_to = match applied {
                    config_edit::Applied::MovedTo(ix) => ix,
                    config_edit::Applied::Done => at.index,
                };
                self.reload(path, text, None);
                self.select_index(moved_to.min(self.entries.len() - 1));
            }
            ConfigEdit::SetGlobalTheme { .. }
            | ConfigEdit::SaveTheme { .. }
            | ConfigEdit::DeleteTheme { .. }
            | ConfigEdit::SetFonts(_) => {}
        }
    }

    /// Whether button `b` can be pressed (and focused) in this section.
    pub fn enabled(&self, b: super::Btn) -> bool {
        use super::Btn;
        let editable = self.read_only.is_none();
        let readable = self.selected_entry().is_some_and(|e| e.profile.is_ok());
        let (ix, len) = (self.selected_index().unwrap_or(0), self.entries.len());
        match b {
            Btn::New => editable,
            Btn::Duplicate | Btn::Edit => editable && readable,
            Btn::Delete => editable && len > 1,
            Btn::Up => editable && ix > 0,
            Btn::Down => editable && ix + 1 < len,
            Btn::Save => matches!(self.editor_result(), Some(Ok(_))),
            Btn::Cancel
            | Btn::Apply
            | Btn::Close
            | Btn::Keep
            | Btn::ConfirmDelete
            | Btn::Clear
            | Btn::Discard => true,
        }
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
                self.names().collect::<Vec<_>>(),
                error
            ),
            Mode::Editor(editor) => {
                let invalid = match self.editor_result() {
                    Some(Err(errors)) => errors.iter().map(|(f, _)| f.label()).collect::<Vec<_>>(),
                    Some(Ok(_)) | None => Vec::new(),
                };
                format!(
                    "--- settings section={section} mode=editor name={:?} field={:?} theme={:?} default={} remote={} connect_at_launch={} invalid={:?} can_save={} error={:?}\n",
                    editor.text(Field::Name),
                    field,
                    editor.theme().unwrap_or("follow global"),
                    editor.is_default(),
                    remote_label(editor.values.remote),
                    // Not shown for a local profile.
                    if editor.values.remote == RemoteKind::Local { "-".to_string() } else { editor.connect_at_launch().to_string() },
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

use super::{Btn, ListOp, Section, SettingsDialog};
use crate::actions::{
    SettingsDuplicate, SettingsMoveDown, SettingsMoveUp, SettingsSectionFirst, SettingsSectionLast, SettingsSectionNext,
    SettingsSectionPrev,
};
use brindle_ui::{
    ActiveTheme as _, ButtonVariant, Checkbox, FIELD_HEIGHT, Input, Select, SelectEvent, SelectOption, Switch, TextInput,
    TextInputEvent, option_keyed,
};

/// The fields shown as a select rather than text.
const SELECT_FIELDS: [Field; 3] = [Field::Theme, Field::Remote, Field::Engine];

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
        let known = self.model.themes.theme_names();
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
            // Each option is keyed by its index (labels can repeat), and the
            // chosen key is found again at it.
            let options: Vec<SelectOption> = editor
                .choice_labels(field, &known)
                .into_iter()
                .enumerate()
                .map(|(ix, label)| SelectOption::new(label).key(ix.to_string()))
                .collect();
            let selected = editor.choice_index(field);
            let select = cx.new(|cx| Select::new(options, selected, cx));
            let changed = cx.subscribe_in(&select, window, move |this, select, event: &SelectEvent, window, cx| {
                let SelectEvent::Changed(key) = event;
                if let Some(ix) = option_keyed(select.read(cx).options(), key) {
                    this.apply_choice(field, ix, window, cx);
                }
            });
            self._profile_subscriptions.push(changed);
            self.profile_selects.push((field, select));
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
    #[allow(
        clippy::wildcard_enum_match_arm,
        reason = "railway R3 exception: every field but the two switches and the selects is a text input"
    )]
    fn profile_field_handle(&self, editor: &ProfileEditor, field: Field, cx: &App) -> Option<FocusHandle> {
        match field {
            Field::Default => (!editor.default_locked()).then(|| self.default_handle.clone()),
            Field::ConnectAtLaunch => Some(self.connect_handle.clone()),
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
        self.switch_section(|model, text| model.next_section(true, text), window, cx);
    }

    pub(super) fn section_prev(&mut self, _: &SettingsSectionPrev, window: &mut Window, cx: &mut Context<Self>) {
        self.switch_section(|model, text| model.next_section(false, text), window, cx);
    }

    pub(super) fn section_first(&mut self, _: &SettingsSectionFirst, window: &mut Window, cx: &mut Context<Self>) {
        self.switch_section(|model, text| model.edge_section(false, text), window, cx);
    }

    pub(super) fn section_last(&mut self, _: &SettingsSectionLast, window: &mut Window, cx: &mut Context<Self>) {
        self.switch_section(|model, text| model.edge_section(true, text), window, cx);
    }

    pub(super) fn duplicate(&mut self, _: &SettingsDuplicate, window: &mut Window, cx: &mut Context<Self>) {
        self.list_op(ListOp::Duplicate, window, cx);
    }

    pub(super) fn move_up(&mut self, _: &SettingsMoveUp, window: &mut Window, cx: &mut Context<Self>) {
        let step = self.model.move_profile(true);
        self.step(step, window, cx);
    }

    pub(super) fn move_down(&mut self, _: &SettingsMoveDown, window: &mut Window, cx: &mut Context<Self>) {
        let step = self.model.move_profile(false);
        self.step(step, window, cx);
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
        match self.model.profiles.editor().cloned() {
            Some(editor) => self.render_profile_editor(&editor, cx),
            None => self.render_profile_list(window, cx),
        }
    }

    fn render_profile_list(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let t = *cx.ui_theme();
        let fg = t.foreground;
        let muted = t.muted_foreground;
        let state = &self.model.profiles;

        let rows = state
            .rows()
            .into_iter()
            .enumerate()
            .map(|(ix, row)| {
                div()
                    .id(("profile", ix))
                    .flex()
                    .flex_none()
                    .items_center()
                    .gap_3()
                    .px_3()
                    .py_1p5()
                    .rounded_md()
                    .when(row.selected, |d| d.bg(t.accent))
                    .when(!row.selected, |d| d.hover(|d| d.bg(t.accent.opacity(0.5))))
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
                            .child(row.hint.map(SharedString::from).unwrap_or_default()),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(div().text_color(if row.readable { fg } else { muted }).child(SharedString::from(row.name)))
                            .child(div().text_xs().text_color(muted).child(SharedString::from(row.detail))),
                    )
                    .when(row.default, |d| d.child(div().flex_none().text_xs().text_color(t.primary).child("● default")))
            })
            .collect::<Vec<_>>();

        let confirming = state.list_mode() == ListMode::ConfirmDelete;
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
                    this.list_op(ListOp::New, window, cx)
                }))
                .child(self.button(Btn::Duplicate, "Duplicate (c)", outline, cx, |this, window, cx| {
                    this.list_op(ListOp::Duplicate, window, cx)
                }))
                .child(self.button(Btn::Edit, "Edit (e)", outline, cx, |this, window, cx| {
                    this.list_op(ListOp::Edit, window, cx)
                }))
                .child(self.button(Btn::Delete, "Delete (d)", ButtonVariant::Destructive, cx, |this, window, cx| {
                    this.list_op(ListOp::Delete, window, cx)
                }))
                .child(self.button(Btn::Up, "↑", outline, cx, |this, window, cx| {
                    let step = this.model.move_profile(true);
                    this.step(step, window, cx);
                }))
                .child(self.button(Btn::Down, "↓", outline, cx, |this, window, cx| {
                    let step = this.model.move_profile(false);
                    this.step(step, window, cx);
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
            Some(Ok(_)) | None => Vec::new(),
        };
        let error_for = |field: Field| errors.iter().find(|(f, _)| *f == field).map(|(_, e)| e.clone());
        let label = |text: &'static str| div().w(px(120.0)).flex_none().text_color(muted).child(text);

        let row = |this: &Self, field: Field, cx: &mut Context<Self>| -> AnyElement {
            let error = error_for(field);
            let control: AnyElement = match field {
                Field::Theme | Field::Remote | Field::Engine => {
                    this.choice_slot(field).flex_1().min_w_0().into_any_element()
                }
                Field::Default => this.render_default_toggle(editor, cx),
                Field::ConnectAtLaunch => this.render_connect_toggle(editor, cx),
                Field::Name
                | Field::Command
                | Field::Args
                | Field::Cwd
                | Field::Env
                | Field::FontSize
                | Field::SshHost
                | Field::SshUser
                | Field::SshPort
                | Field::SshIdentity
                | Field::SshArgs
                | Field::Container
                | Field::ContainerUser => div()
                    .flex_1()
                    .min_w_0()
                    .children(this.profile_input(field).map(|input| Input::new(input).invalid(error.is_some())))
                    .into_any_element(),
            };
            // The control line is at least a text field tall with its control
            // centred; an error sits under it at its natural height.
            div()
                .flex()
                .flex_col()
                .flex_none()
                .gap_0p5()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap_2()
                        .min_h(FIELD_HEIGHT)
                        .child(label(field.label()))
                        .child(control),
                )
                .when_some(error, |d, e| {
                    d.child(div().pl(px(128.0)).text_xs().text_color(red).child(SharedString::from(e)))
                })
                .into_any_element()
        };

        let visible = editor.visible_fields();
        let name_row = row(self, Field::Name, cx);
        let rows: Vec<AnyElement> = visible.iter().skip(1).map(|&f| row(self, f, cx)).collect();
        let title = editor.title();
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
                        "Theme and font size show on this profile's tabs now; command, arguments, directory and environment apply to new tabs only.",
                    ))
                    .child(self.editor_buttons(cx)),
            )
            .into_any_element()
    }

    /// The "Connect at launch" switch and its note.
    fn render_connect_toggle(&self, editor: &ProfileEditor, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.ui_theme().muted_foreground;
        let toggle = |this: &mut Self, window: &mut Window, cx: &mut Context<Self>| {
            this.edit_profile(window, cx, |e| e.toggle_connect_at_launch());
        };
        div()
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .gap_2()
            .child(
                Switch::new("profile-connect-at-launch", editor.connect_at_launch())
                    .focus(&self.connect_handle)
                    .on_click(cx.listener(move |this, _: &bool, window, cx| toggle(this, window, cx))),
            )
            .child(
                div()
                    .id("profile-connect-at-launch-note")
                    .text_xs()
                    .text_color(muted)
                    // The note is the switch's label: a click on it toggles too.
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        window.focus(&this.connect_handle);
                        toggle(this, window, cx)
                    }))
                    .child("Connect when Brindle starts, not on first use"),
            )
            .into_any_element()
    }

    /// The default checkbox and its note. The current default can't be
    /// unticked, so its checkbox is disabled.
    fn render_default_toggle(&self, editor: &ProfileEditor, cx: &mut Context<Self>) -> AnyElement {
        let muted = cx.ui_theme().muted_foreground;
        let locked = editor.default_locked();
        let note = editor.default_note();
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
name = "htop"
command = "htop"

# my work profile
[[profiles]]
name = "Work"
cwd = "~/src"
env = { PAGER = "less -R" }
args = ["-c", "echo hi; exec bash"]
theme = "gone"
"#;

    const THEMES: &[&str] = &["brindle-dark", "tokyo-night", "mine"];

    fn path() -> &'static Path {
        Path::new("/c/config.toml")
    }

    /// The config file as text: the state reads it, and each edit the state
    /// asks for is written to it as the view writes it to the file, then
    /// reported back with `written`.
    struct Fixture {
        text: Option<String>,
    }

    impl Fixture {
        fn new(text: &str) -> Self {
            Self { text: Some(text.to_string()) }
        }

        fn read(&self) -> FileText {
            Ok(self.text.clone())
        }

        fn state(&self) -> ProfilesState {
            ProfilesState::new(path(), &self.read(), Vec::new(), None)
        }

        fn text(&self) -> String {
            self.text.clone().unwrap_or_default()
        }

        fn config(&self) -> Config {
            Config::parse(&self.text()).unwrap().config
        }

        /// Takes what an operation returned: writes its edit and reports
        /// it. Whether it wrote, or why not.
        fn take(&mut self, state: &mut ProfilesState, step: Result<Step, String>) -> Result<bool, String> {
            match step? {
                Step::Nothing => Ok(false),
                Step::Close => panic!("the dialog closed"),
                Step::Write(edit) => {
                    let (text, applied) = config_edit::edited(self.text.as_deref(), |doc| edit.apply(doc))?;
                    self.text = Some(text);
                    state.written(&edit, applied, path(), &self.read());
                    Ok(true)
                }
            }
        }

        fn save(&mut self, state: &mut ProfilesState) -> Result<bool, String> {
            let step = state.save();
            self.take(state, step)
        }

        fn delete(&mut self, state: &mut ProfilesState) -> Result<bool, String> {
            let step = state.delete_selected();
            self.take(state, step)
        }

        fn move_selected(&mut self, state: &mut ProfilesState, up: bool) -> Result<bool, String> {
            let step = state.move_selected(up);
            self.take(state, step)
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
        let f = Fixture::new(CONFIG);
        let state = f.state();
        assert_eq!(names(&state), ["Shell", "htop", "Work"]);
        assert_eq!(state.default_index(), 2);
        assert_eq!(state.selected_index(), Some(2));
        assert!(state.describe(&None, "").contains("default=\"Work\""));
    }

    #[test]
    fn implicit_unnamed_unreadable_and_broken_files() {
        let f = Fixture::new("theme = \"mine\"\n");
        let state = f.state();
        assert_eq!(names(&state), ["Shell"]);
        assert_eq!(state.default_index(), 0);
        // An unknown key elsewhere doesn't hide the profiles; a bad profile
        // is listed with its error and can't be edited.
        let f = Fixture::new("fontsize = 3\n[[profiles]]\nname = \"Work\"\n[[profiles]]\nfont_size = \"big\"\n[[profiles]]\ncwd = \"~\"\n");
        let mut state = f.state();
        assert_eq!(names(&state), ["Work", "Profile 2", "Profile 3"]);
        assert!(state.entries[1].profile.is_err());
        state.select(1);
        assert!(state.start_edit(&themes()).unwrap_err().contains("can't be read"));
        assert!(!edit(&mut state, "Work").dirty());
        // A file that isn't TOML lists the loaded profiles, read-only.
        let f = Fixture::new("[oops\n");
        let mut state =
            ProfilesState::new(path(), &f.read(), vec![Profile { name: "Loaded".into(), ..Default::default() }], None);
        assert_eq!(names(&state), ["Loaded"]);
        assert!(state.start_edit(&themes()).unwrap_err().contains("isn't valid TOML"));
        assert!(state.start_new(&themes()).is_err());
    }

    /// What the read-only text says, and that a missing file isn't one.
    #[test]
    fn the_read_only_text_names_the_file_and_the_cause() {
        let f = Fixture::new("profiles = 3\n");
        let state = f.state();
        let why = state.read_only.clone().unwrap();
        assert!(why.starts_with("profiles in ") && why.ends_with("config.toml is not an array"), "{why}");

        // A file that can't be read at all.
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        let state = ProfilesState::new(path(), &config_edit::text_from(path(), Err(denied)), Vec::new(), None);
        let why = state.read_only.clone().unwrap();
        assert!(why.starts_with("reading /c/config.toml: "), "{why}");

        // No file reads as an empty document: the implicit profile, editable.
        let mut state = Fixture { text: None }.state();
        assert_eq!(state.read_only, None);
        assert_eq!(names(&state), [IMPLICIT_PROFILE]);
        assert!(state.start_edit(&themes()).is_ok());
    }

    #[test]
    fn a_word_with_nul_is_unquotable() {
        assert_eq!(join_words(&["-c".into(), "a b".into()]).unwrap(), "-c 'a b'");
        let nul = "a\0b".to_string();
        assert_eq!(quote(&nul), Err(Unquotable { word: nul.clone() }));
        assert_eq!(join_words(&["-c".into(), nul.clone()]), Err(Unquotable { word: nul }));
        assert!(quote("a\0b").unwrap_err().to_string().contains("can't be written as a shell word"));
    }

    /// Arguments from the file that can't be written as shell words are
    /// marked invalid, saying so, and block Save until the field is edited:
    /// the text shown wouldn't be what the file has.
    #[test]
    fn an_unquotable_argument_blocks_save() {
        let mut f = Fixture::new(
            "[[profiles]]\nname = \"Odd\"\nargs = [\"-c\", \"a\\u0000b\"]\nenv = { K = \"x\\u0000y\" }\n",
        );
        let mut state = f.state();
        edit(&mut state, "Odd");
        assert_eq!(invalid(&state), [Field::Args, Field::Env]);
        let Err(errors) = state.editor_result().unwrap() else { panic!("saving must be refused") };
        assert!(errors.iter().all(|(_, why)| why.contains("can't be written as a shell word")), "{errors:?}");
        // The file is untouched by a refused save.
        let before = f.text();
        assert!(f.save(&mut state).is_err());
        assert_eq!(f.text(), before);
        // Editing the field replaces the words, so it is valid again.
        let editor = state.editor_mut().unwrap();
        editor.set_text(Field::Args, "-c ok");
        assert_eq!(invalid(&state), [Field::Env]);
        let editor = state.editor_mut().unwrap();
        editor.set_text(Field::Env, "K=v");
        assert_eq!(invalid(&state), []);
    }

    #[test]
    fn fields_as_shell_words_round_trip() {
        let mut f = Fixture::new(CONFIG);
        let mut state = f.state();
        let editor = edit(&mut state, "Work");
        assert_eq!(editor.text(Field::Env), "PAGER='less -R'");
        assert_eq!(editor.text(Field::Args), "-c 'echo hi; exec bash'");
        assert_eq!(editor.theme(), Some("gone"));
        // A missing theme stays choosable, after the known ones.
        assert_eq!(editor.theme_choices.last().unwrap().as_deref(), Some("gone"));
        assert!(editor.is_default() && editor.default_locked());
        // Saving without changes keeps every value.
        f.save(&mut state).unwrap();
        let work = f.config().profiles[2].clone();
        assert_eq!(work.args, ["-c", "echo hi; exec bash"]);
        assert_eq!(work.env["PAGER"], "less -R");
        assert_eq!(work.theme.as_deref(), Some("gone"));
        assert!(f.text().contains("# my work profile\n[[profiles]]\nname = \"Work\""), "{}", f.text());
    }

    #[test]
    fn validation() {
        let mut f = Fixture::new(CONFIG);
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
        assert!(f.save(&mut state).is_err());
        assert_eq!(f.text(), before);
        // Keeping its own name, in another case, is fine.
        let editor = state.editor_mut().unwrap();
        editor.set_text(Field::Env, "");
        editor.set_text(Field::FontSize, "13.5");
        editor.set_text(Field::Name, "WORK");
        assert!(invalid(&state).is_empty());
    }

    /// A local profile's fields end with the font size, then the default
    /// checkbox.
    #[test]
    fn local_fields_end_with_font_size_then_default() {
        let f = Fixture::new(CONFIG);
        let mut state = f.state();
        let editor = edit(&mut state, "Shell");
        let visible = editor.visible_fields();
        assert_eq!(visible[visible.len() - 2..], [Field::FontSize, Field::Default]);
    }

    #[test]
    fn ssh_target_is_written_and_read_back() {
        let mut f = Fixture::new(CONFIG);
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
        f.save(&mut state).unwrap();
        match &f.config().profiles[2].remote {
            Some(RemoteTarget::Ssh { host, user, port, identity_file, ssh_args, connect_at_launch }) => {
                assert_eq!((host.as_str(), user.as_deref(), *port), ("A8", Some("ubuntu"), Some(2222)));
                assert!(!connect_at_launch);
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

    /// 8.1: editing a profile that connects at launch, through either kind
    /// of target, keeps `connect_at_launch` when its switch isn't touched,
    /// and one that doesn't still doesn't.
    #[test]
    fn saving_an_edit_keeps_connect_at_launch() {
        let text = "[[profiles]]\nname = \"Shell\"\n\
                    [[profiles]]\nname = \"A8\"\nremote = { type = \"ssh\", host = \"A8\", connect_at_launch = true }\n\
                    [[profiles]]\nname = \"box\"\nremote = { type = \"container\", container = \"box\", connect_at_launch = true }\n\
                    [[profiles]]\nname = \"B9\"\nremote = { type = \"ssh\", host = \"B9\" }\n";
        let mut f = Fixture::new(text);
        let mut state = f.state();
        for name in ["A8", "box", "B9"] {
            edit(&mut state, name).set_text(Field::Cwd, "/srv");
            f.save(&mut state).unwrap();
        }
        // The kind changed too: SSH to a container, the setting kept.
        let editor = edit(&mut state, "A8");
        editor.choose_remote(RemoteKind::Container);
        editor.set_text(Field::Container, "a8box");
        f.save(&mut state).unwrap();
        let config = f.config();
        let eager: Vec<(&str, bool, Option<&str>)> = config.profiles[1..]
            .iter()
            .map(|p| (p.name.as_str(), p.remote.as_ref().unwrap().connects_at_launch(), p.cwd.as_deref()))
            .collect();
        assert_eq!(eager, [("A8", true, Some("/srv")), ("box", true, Some("/srv")), ("B9", false, Some("/srv"))]);
        assert!(matches!(config.profiles[1].remote, Some(RemoteTarget::Container { .. })));
        assert_eq!(f.text().matches("connect_at_launch = true").count(), 2, "{}", f.text());
        assert!(!f.text().contains("connect_at_launch = false"), "{}", f.text());
    }

    /// 8.4: the "Connect at launch" switch is shown for `ssh` and
    /// `container`, hidden for `none`, and is a stop (the editor's stops are
    /// its visible fields in order) after the kind's own fields.
    #[test]
    fn connect_at_launch_switch_shows_per_kind() {
        let f = Fixture::new(CONFIG);
        let mut state = f.state();
        let editor = edit(&mut state, "Work");
        assert_eq!(editor.remote_kind(), RemoteKind::Local);
        assert!(!editor.visible_fields().contains(&Field::ConnectAtLaunch));
        editor.choose_remote(RemoteKind::Ssh);
        let visible = editor.visible_fields();
        let at = visible.iter().position(|f| *f == Field::ConnectAtLaunch).expect("shown for ssh");
        assert_eq!(visible[at - 1], Field::SshArgs, "after the last ssh field");
        assert_eq!(visible[at + 1], Field::Command, "before the profile's own fields");
        editor.choose_remote(RemoteKind::Container);
        let visible = editor.visible_fields();
        let at = visible.iter().position(|f| *f == Field::ConnectAtLaunch).expect("shown for container");
        assert_eq!(visible[at - 1], Field::ContainerUser, "after the last container field");
        assert_eq!(visible.iter().filter(|f| **f == Field::ConnectAtLaunch).count(), 1);
        editor.choose_remote(RemoteKind::Local);
        assert!(!editor.visible_fields().contains(&Field::ConnectAtLaunch));
    }

    /// 8.4, the spec's scenario: an SSH host `A8` with the switch on saves
    /// `connect_at_launch = true`; off writes no key; a toggle makes the
    /// editor dirty and survives a trip through `none`.
    #[test]
    fn connect_at_launch_switch_is_written_only_when_on() {
        let mut f = Fixture::new(CONFIG);
        let mut state = f.state();
        let editor = edit(&mut state, "Work");
        editor.choose_remote(RemoteKind::Ssh);
        editor.set_text(Field::SshHost, "A8");
        assert!(!editor.connect_at_launch());
        f.save(&mut state).unwrap();
        assert!(!f.text().contains("connect_at_launch"), "{}", f.text());

        let editor = edit(&mut state, "Work");
        assert!(!editor.dirty());
        editor.toggle_connect_at_launch();
        assert!(editor.connect_at_launch() && editor.dirty());
        // Through none and back, the switch is as it was left.
        editor.choose_remote(RemoteKind::Local);
        editor.choose_remote(RemoteKind::Ssh);
        assert!(editor.connect_at_launch());
        f.save(&mut state).unwrap();
        assert!(f.text().contains(r#"remote = { type = "ssh", host = "A8", connect_at_launch = true }"#), "{}", f.text());
        assert_eq!(f.config().profiles[2].remote.as_ref().map(RemoteTarget::connects_at_launch), Some(true));

        // Opened again it shows on, and turning it off removes the key.
        let editor = edit(&mut state, "Work");
        assert!(editor.connect_at_launch() && !editor.dirty());
        editor.toggle_connect_at_launch();
        f.save(&mut state).unwrap();
        assert!(!f.text().contains("connect_at_launch"), "{}", f.text());
        assert!(f.text().contains(r#"remote = { type = "ssh", host = "A8" }"#), "{}", f.text());

        // The container kind writes it the same way, and `none` drops it
        // with the rest of the target.
        let editor = edit(&mut state, "Work");
        editor.choose_remote(RemoteKind::Container);
        editor.set_text(Field::Container, "box");
        editor.toggle_connect_at_launch();
        f.save(&mut state).unwrap();
        assert!(f.text().contains("connect_at_launch = true"), "{}", f.text());
        let editor = edit(&mut state, "Work");
        editor.choose_remote(RemoteKind::Local);
        f.save(&mut state).unwrap();
        assert!(!f.text().contains("remote") && !f.text().contains("connect_at_launch"), "{}", f.text());
    }

    #[test]
    fn connect_at_launch_shows_in_the_dump() {
        let f = Fixture::new(CONFIG);
        let mut state = f.state();
        edit(&mut state, "Work");
        let line = state.describe(&None, "Connect at launch");
        assert!(line.contains("remote=none connect_at_launch=-"), "{line}");
        let editor = state.editor_mut().unwrap();
        editor.choose_remote(RemoteKind::Ssh);
        editor.toggle_connect_at_launch();
        let line = state.describe(&None, "Connect at launch");
        assert!(line.contains("remote=ssh connect_at_launch=true"), "{line}");
    }

    #[test]
    fn container_target_and_removal() {
        let mut f = Fixture::new(CONFIG);
        let mut state = f.state();
        let editor = edit(&mut state, "Work");
        editor.choose(Field::Remote, 2);
        assert_eq!(editor.remote_kind(), RemoteKind::Container);
        editor.set_text(Field::Container, "my-box");
        editor.choose(Field::Engine, 1);
        f.save(&mut state).unwrap();
        assert_eq!(
            f.config().profiles[2].remote,
            Some(RemoteTarget::Container {
                engine: "podman".into(),
                container: "my-box".into(),
                user: None,
                connect_at_launch: false
            })
        );
        let editor = edit(&mut state, "Work");
        assert_eq!((editor.remote_kind(), editor.engine()), (RemoteKind::Container, "podman"));
        editor.choose_remote(RemoteKind::Local);
        f.save(&mut state).unwrap();
        assert_eq!(f.config().profiles[2].remote, None);
        assert!(!f.text().contains("remote"));
    }

    #[test]
    fn remote_validation() {
        let f = Fixture::new(CONFIG);
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
        let f = Fixture::new(CONFIG);
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
        for field in [Field::Remote, Field::Engine] {
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
        let f = Fixture::new(CONFIG);
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
        let mut f = Fixture::new(CONFIG);
        let mut state = f.state();
        edit(&mut state, "Work").set_text(Field::Name, "Job");
        assert_eq!(f.save(&mut state), Ok(true));
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
        let mut f = Fixture::new(CONFIG);
        let mut state = f.state();
        let editor = edit(&mut state, "htop");
        assert!(!editor.is_default());
        editor.toggle_default();
        f.save(&mut state).unwrap();
        assert_eq!(f.config().default_profile.as_deref(), Some("htop"));
        // The default can't be unchecked.
        let editor = edit(&mut state, "htop");
        editor.toggle_default();
        assert!(editor.is_default());
    }

    #[test]
    fn new_and_duplicate() {
        let mut f = Fixture::new(CONFIG);
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
        f.save(&mut state).unwrap();
        assert_eq!(names(&state), ["Shell", "htop", "Work", "Work copy"]);
        assert_eq!(state.selected_entry().unwrap().name, "Work copy");
        assert_eq!(f.config().default_profile.as_deref(), Some("work"));
        state.select(2);
        state.start_duplicate(&themes()).unwrap();
        assert_eq!(state.editor().unwrap().text(Field::Name), "Work copy 2");
    }

    #[test]
    fn first_profile_on_an_empty_config() {
        let mut f = Fixture::new("# nothing\n");
        let mut state = f.state();
        state.start_new(&themes()).unwrap();
        state.editor_mut().unwrap().set_text(Field::Name, "Work");
        f.save(&mut state).unwrap();
        assert_eq!(names(&state), ["Shell", "Work"]);
    }

    #[test]
    fn deleting() {
        let mut f = Fixture::new(CONFIG);
        let mut state = f.state();
        state.select(2);
        state.request_delete().unwrap();
        assert!(state.delete_question().contains("default profile"));
        f.delete(&mut state).unwrap();
        assert_eq!(names(&state), ["Shell", "htop"]);
        assert_eq!(f.config().default_profile, None);
        assert!(f.text().starts_with("# test config\n"));
        state.select(0);
        assert_eq!(state.delete_question(), "Delete \"Shell\"?");
        f.delete(&mut state).unwrap();
        assert!(state.request_delete().unwrap_err().contains("last profile"));
        assert_eq!(names(&state), ["htop"]);
    }

    #[test]
    fn moving() {
        let mut f = Fixture::new(CONFIG);
        let mut state = f.state();
        state.select(2);
        assert_eq!(f.move_selected(&mut state, true), Ok(true));
        assert_eq!(names(&state), ["Shell", "Work", "htop"]);
        assert_eq!(state.selected_index(), Some(1));
        assert!(f.text().contains("# my work profile\n[[profiles]]\nname = \"Work\""));
        state.select(0);
        assert_eq!(f.move_selected(&mut state, true), Ok(false));
        assert_eq!(f.config().profiles[1].name, "Work");
    }

    /// Up and down step through the list and wrap; in the editor and the
    /// delete question they leave the selection alone.
    #[test]
    fn moving_the_selection_wraps_in_the_list_only() {
        let f = Fixture::new(CONFIG);
        let mut state = f.state();
        assert_eq!(state.selected_entry().unwrap().name, "Work");
        state.move_selection(true);
        assert_eq!(state.selected_entry().unwrap().name, "Shell");
        state.move_selection(false);
        assert_eq!(state.selected_entry().unwrap().name, "Work");
        state.move_selection(false);
        assert_eq!(state.selected_entry().unwrap().name, "htop");
        state.start_edit(&themes()).unwrap();
        state.move_selection(true);
        state.discard();
        assert_eq!(state.selected_entry().unwrap().name, "htop");
        state.request_delete().unwrap();
        state.move_selection(true);
        assert_eq!(state.selected_entry().unwrap().name, "htop");
    }

    #[test]
    fn the_delete_question_is_in_the_dump() {
        let f = Fixture::new(CONFIG);
        let mut state = f.state();
        state.select(1);
        state.request_delete().unwrap();
        assert_eq!(state.describe(&None, ""), "--- settings section=Profiles mode=confirm-delete profile=\"htop\"\n");
    }

    #[test]
    fn file_changed_underneath() {
        let mut f = Fixture::new(CONFIG);
        let mut state = f.state();
        edit(&mut state, "Work").set_text(Field::Cwd, "~");
        f.text = Some(CONFIG.replace("name = \"Work\"", "name = \"Other\""));
        let before = f.text();
        let err = f.save(&mut state).unwrap_err();
        assert!(err.contains("reopen the Profiles section"), "{err}");
        assert_eq!(f.text(), before);
        assert!(state.editor().is_some());
    }

    /// Each operation returns the edit to write, with the profile it acts
    /// on as the list has it, and touches nothing.
    #[test]
    fn operations_return_their_edits() {
        let f = Fixture::new(CONFIG);
        let mut state = f.state();
        let work = ProfileRef { index: 2, name: Some("Work".into()) };
        assert_eq!(state.move_selected(true), Ok(Step::Write(ConfigEdit::MoveProfile { at: work.clone(), up: true })));
        assert_eq!(state.move_selected(false), Ok(Step::Nothing), "at the end");
        state.request_delete().unwrap();
        assert_eq!(
            state.delete_selected(),
            Ok(Step::Write(ConfigEdit::DeleteProfile { at: work.clone(), name: "Work".into() }))
        );
        assert_eq!(state.list_mode(), ListMode::List);
        // A rename of the default renames `default_profile` with it.
        edit(&mut state, "Work").set_text(Field::Name, "Job");
        let Ok(Step::Write(ConfigEdit::SaveProfile { at, profile, default })) = state.save() else { panic!("a save") };
        assert_eq!((at, profile.name.as_str()), (Some(work), "Job"));
        assert_eq!(default, DefaultChange::Rename { old: "Work".into(), new: "Job".into() });
        state.discard();
        // Another made the default; a new one is added.
        edit(&mut state, "htop").toggle_default();
        let Ok(Step::Write(ConfigEdit::SaveProfile { at, default, .. })) = state.save() else { panic!("a save") };
        assert_eq!((at, default), (Some(ProfileRef { index: 1, name: Some("htop".into()) }), DefaultChange::Set("htop".into())));
        state.discard();
        state.start_new(&themes()).unwrap();
        let Ok(Step::Write(ConfigEdit::SaveProfile { at, profile, default })) = state.save() else { panic!("a save") };
        assert_eq!((at, profile.name.as_str(), default), (None, "Profile 4", DefaultChange::Keep));
        // A profile renamed that isn't the default.
        state.discard();
        edit(&mut state, "Shell").set_text(Field::Name, "Main");
        let Ok(Step::Write(ConfigEdit::SaveProfile { default, .. })) = state.save() else { panic!("a save") };
        assert_eq!(default, DefaultChange::Rename { old: "Shell".into(), new: "Main".into() });
        assert_eq!(f.text(), CONFIG);
    }

    /// A reload keeps the selection by its key wherever the profile moved,
    /// and a profile that is gone gives way to the one in its place.
    #[test]
    fn a_reload_keeps_the_selection_by_key() {
        let mut f = Fixture::new(CONFIG);
        let mut state = f.state();
        assert_eq!(state.selected_entry().unwrap().name, "Work");
        // Moved to the front, elsewhere; the key matches names in any case.
        f.text = Some(
            "default_profile = \"work\"\n[[profiles]]\nname = \"WORK\"\n[[profiles]]\nname = \"Shell\"\n[[profiles]]\nname = \"htop\"\n"
                .into(),
        );
        state.reload(path(), &f.read(), None);
        assert_eq!((state.selected_index(), state.selected_entry().unwrap().name.as_str()), (Some(0), "WORK"));
        // Gone: the profile now at its place.
        f.text = Some("[[profiles]]\nname = \"Shell\"\n[[profiles]]\nname = \"htop\"\n".into());
        state.reload(path(), &f.read(), None);
        assert_eq!(state.selected_entry().unwrap().name, "Shell");
        state.select(1);
        // Gone, and the list shorter than its place: the last.
        f.text = Some("[[profiles]]\nname = \"Shell\"\n".into());
        state.reload(path(), &f.read(), None);
        assert_eq!(state.selected_entry().unwrap().name, "Shell");
        // A deleted profile's place, after the write.
        let mut f = Fixture::new(CONFIG);
        let mut state = f.state();
        state.select(1);
        f.delete(&mut state).unwrap();
        assert_eq!(state.selected_entry().unwrap().name, "Work");
        // An edit discarded goes back to the profile it was opened on.
        edit(&mut state, "Shell");
        f.text = Some(CONFIG.replace("[[profiles]]\nname = \"Shell\"\n", ""));
        let text = f.read();
        state.reload(path(), &text, None);
        state.discard();
        assert_eq!(state.selected_entry().unwrap().name, "htop", "Shell went: the one in its place");
    }

    /// The list's rows: name, detail, shortcut digit, selection and the
    /// default, as the view draws them.
    #[test]
    fn rows_mark_the_selection_and_the_default() {
        let f = Fixture::new(&format!("{CONFIG}[[profiles]]\nfont_size = \"big\"\n"));
        let mut state = f.state();
        state.select(1);
        let rows = state.rows();
        assert_eq!(rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["Shell", "htop", "Work", "Profile 4"]);
        assert_eq!(rows.iter().map(|r| (r.selected, r.default)).collect::<Vec<_>>(), [
            (false, false),
            (true, false),
            (false, true),
            (false, false)
        ]);
        assert_eq!(rows[1].hint.as_deref(), Some("2"));
        assert_eq!(rows[1].detail, state.entries[1].profile.as_ref().unwrap().description());
        assert!(!rows[3].readable && !rows[3].detail.is_empty());
    }

    /// The editor's title, default note and theme labels.
    #[test]
    fn the_editors_texts() {
        let f = Fixture::new(CONFIG);
        let mut state = f.state();
        let editor = edit(&mut state, "Work");
        assert_eq!(editor.title(), "Edit \"Work\"");
        assert!(editor.default_note().starts_with("New tabs use this profile."));
        assert_eq!(editor.theme_labels(&themes()), ["follow global", "brindle-dark", "tokyo-night", "mine", "gone (missing)"]);
        assert_eq!(editor.choice_labels(Field::Theme, &themes()), editor.theme_labels(&themes()));
        assert_eq!(editor.choice_labels(Field::Remote, &themes()), ["none", "ssh", "container"]);
        assert!(editor.choice_labels(Field::Cwd, &themes()).is_empty());
        state.discard();
        let editor = edit(&mut state, "htop");
        assert_eq!(editor.default_note(), "Use this profile for new tabs and windows");
        state.discard();
        state.start_new(&themes()).unwrap();
        assert_eq!(state.editor().unwrap().title(), "New profile");
    }

    /// Which buttons the list and the editor can press.
    #[test]
    fn buttons_follow_the_selection_and_the_file() {
        let f = Fixture::new(&format!("{CONFIG}[[profiles]]\nfont_size = \"big\"\n"));
        let mut state = f.state();
        state.select(0);
        assert!(state.enabled(Btn::Edit) && state.enabled(Btn::Down) && !state.enabled(Btn::Up));
        state.select(3);
        assert!(!state.enabled(Btn::Edit) && !state.enabled(Btn::Duplicate), "unreadable");
        assert!(state.enabled(Btn::Delete) && state.enabled(Btn::Up) && !state.enabled(Btn::Down));
        let single = Fixture::new("[[profiles]]\nname = \"Only\"\n").state();
        assert!(!single.enabled(Btn::Delete));
    }
}
