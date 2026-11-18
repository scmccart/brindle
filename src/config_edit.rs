//! Edits to `config.toml` made from inside Brindle (the settings dialog).
//!
//! Every edit works on a `toml_edit` document, so the user's comments,
//! ordering and formatting survive; only the keys an edit concerns change.

use std::path::Path;

use toml_edit::{Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, TableLike, Value};

use crate::config::{DEFAULT_CONFIG, Profile, RemoteTarget, write_replacing};
use crate::theme::{BUILTIN_THEMES, Theme};

pub fn parse(text: &str) -> Result<DocumentMut, String> {
    text.parse::<DocumentMut>().map_err(|e| format!("the config file is not valid TOML; fix it first.\n{e}"))
}

/// Reads the config at `path` (the default config if it doesn't exist),
/// applies `edit` and writes it back in one step, giving what `edit` gave.
/// Nothing is written if the file isn't valid TOML or `edit` fails.
pub fn save<T>(path: &Path, edit: impl FnOnce(&mut DocumentMut) -> Result<T, String>) -> Result<T, String> {
    // Write through symlinks (dotfile managers) rather than replacing them.
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("reading {}: {e}", path.display())),
    };
    let (text, out) = edited(text.as_deref(), edit)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    }
    write_replacing(&path, &text).map_err(|e| format!("writing {}: {e}", path.display()))?;
    Ok(out)
}

/// What [`save`] writes: `text` (the default config when there is no
/// file) with `edit` applied, and what `edit` gave. Refused, as `save` is,
/// when the text isn't valid TOML or `edit` fails.
pub fn edited<T>(
    text: Option<&str>,
    edit: impl FnOnce(&mut DocumentMut) -> Result<T, String>,
) -> Result<(String, T), String> {
    let mut doc = parse(text.unwrap_or(DEFAULT_CONFIG))?;
    let out = edit(&mut doc)?;
    Ok((doc.to_string(), out))
}

/// The config file as the settings dialog read it: its text, `None` when
/// there is no file, or why it couldn't be read (`reading <path>: <cause>`).
/// Each section reads a missing file its own way: the theme and profile
/// lists as an empty file, the Fonts section as the default config, which
/// is what a write starts from.
pub type FileText = Result<Option<String>, String>;

/// Reads the config file at `path` for the settings dialog.
pub fn read_text(path: &Path) -> FileText {
    text_from(path, std::fs::read_to_string(path))
}

/// [`read_text`] for what reading `path` gave.
pub fn text_from(path: &Path, read: std::io::Result<String>) -> FileText {
    match read {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(format!("reading {}: {e}", path.display())),
    }
}

/// A change to the config file the settings dialog asks for. Its sections
/// decide what to write and return one; the view writes it with [`save`]
/// and [`ConfigEdit::apply`].
#[derive(Clone, Debug, PartialEq)]
#[allow(
    clippy::large_enum_variant,
    reason = "one edit at a time, made and written in one handler; boxing the profile adds an indirection for nothing"
)]
pub enum ConfigEdit {
    /// The top-level `theme`.
    SetGlobalTheme { name: String },
    /// `[themes.<name>]`, renamed from `original` (and every reference to
    /// it) first when that is another name.
    SaveTheme { original: Option<String>, name: String, theme: Theme },
    DeleteTheme { name: String },
    /// The profile at `at`, or a new one last, with the default changed.
    SaveProfile { at: Option<ProfileRef>, profile: Profile, default: DefaultChange },
    /// The profile at `at`, listed as `name`.
    DeleteProfile { at: ProfileRef, name: String },
    /// The profile at `at` one place up or down.
    MoveProfile { at: ProfileRef, up: bool },
    /// The Fonts section's changed fields, in this order.
    SetFonts(Vec<FontChange>),
}

/// What saving a profile does to `default_profile`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DefaultChange {
    Keep,
    /// The profile was renamed: a `default_profile` naming `old` names
    /// `new`.
    Rename { old: String, new: String },
    /// The profile is made the default.
    Set(String),
}

/// A Fonts field written by Apply.
#[derive(Clone, Debug, PartialEq)]
pub enum FontChange {
    /// `[font].family`: the family first, or `None` for Brindle's list.
    TerminalFamily(Option<String>),
    /// `[font].size`.
    Size(f32),
    /// `[font].line_height`.
    LineHeight(f32),
    /// `[ui_font].family`: the family first, or `None` for Brindle's list.
    UiFamily(Option<String>),
}

/// What applying an edit gave.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Applied {
    Done,
    /// A moved profile's new position.
    MovedTo(usize),
}

impl ConfigEdit {
    /// Makes the edit in `doc`, through the functions below.
    pub fn apply(&self, doc: &mut DocumentMut) -> Result<Applied, String> {
        match self {
            ConfigEdit::SetGlobalTheme { name } => set_global_theme(doc, name),
            ConfigEdit::SaveTheme { original, name, theme } => {
                if let Some(old) = original.as_deref().filter(|old| old != name) {
                    rename_theme(doc, old, name)?;
                }
                upsert_theme(doc, name, theme);
            }
            ConfigEdit::DeleteTheme { name } => delete_theme(doc, name)?,
            ConfigEdit::SaveProfile { at, profile, default } => {
                upsert_profile(doc, at.as_ref(), profile)?;
                match default {
                    DefaultChange::Keep => {}
                    DefaultChange::Rename { old, new } => rename_default_profile(doc, old, new),
                    DefaultChange::Set(name) => set_default_profile(doc, name),
                }
            }
            ConfigEdit::DeleteProfile { at, name } => delete_profile(doc, at, name)?,
            ConfigEdit::MoveProfile { at, up } => return move_profile(doc, at, *up).map(Applied::MovedTo),
            ConfigEdit::SetFonts(changes) => {
                for change in changes {
                    match change {
                        FontChange::TerminalFamily(family) => set_font_family(doc, FONT_TABLE, family.as_deref()),
                        FontChange::Size(size) => set_font_number(doc, "size", *size),
                        FontChange::LineHeight(height) => set_font_number(doc, "line_height", *height),
                        FontChange::UiFamily(family) => set_font_family(doc, UI_FONT_TABLE, family.as_deref()),
                    }
                }
            }
        }
        Ok(Applied::Done)
    }
}

/// Sets the top-level `theme`.
pub fn set_global_theme(doc: &mut DocumentMut, name: &str) {
    set_value(doc.as_table_mut(), "theme", Value::from(name));
}

/// Writes `[themes.<name>]` with every color of `theme`. An existing table
/// keeps its place and comments.
pub fn upsert_theme(doc: &mut DocumentMut, name: &str, theme: &Theme) {
    let themes = themes_table_mut(doc);
    if !themes.get(name).is_some_and(|i| i.is_table_like()) {
        themes.insert(name, Item::Table(Table::new()));
    }
    let table = themes.get_mut(name).and_then(|i| i.as_table_like_mut()).expect("theme table");
    let color = |c: crate::theme::Color| Value::from(c.to_string());
    set_value(table, "foreground", color(theme.foreground));
    set_value(table, "background", color(theme.background));
    set_value(table, "cursor", color(theme.cursor));
    set_value(table, "cursor_text", color(theme.cursor_text.unwrap_or(theme.background)));
    set_value(table, "selection_background", color(theme.selection_background));
    match theme.selection_foreground {
        Some(c) => set_value(table, "selection_foreground", color(c)),
        None => _ = table.remove("selection_foreground"),
    }
    set_value(table, "accent", color(theme.accent()));
    let (match_bg, match_fg) = theme.search_match();
    let (focused_bg, focused_fg) = theme.search_focused_match();
    set_value(table, "search_match_background", color(match_bg));
    set_value(table, "search_match_foreground", color(match_fg));
    set_value(table, "search_focused_match_background", color(focused_bg));
    set_value(table, "search_focused_match_foreground", color(focused_fg));
    let mut ansi = Array::new();
    for (ix, c) in theme.ansi.iter().enumerate() {
        let mut v = color(*c);
        // Normal colors on the first line, bright ones on the second.
        let prefix = match ix {
            0 => "",
            8 => "\n         ",
            _ => " ",
        };
        v.decor_mut().set_prefix(prefix);
        ansi.push_formatted(v);
    }
    set_value(table, "ansi", Value::Array(ansi));
}

/// Renames `[themes.<old>]` and every reference to it (the global `theme`
/// and profiles' `theme`).
pub fn rename_theme(doc: &mut DocumentMut, old: &str, new: &str) -> Result<(), String> {
    let themes = themes_table_mut(doc);
    if themes.contains_key(new) {
        return Err(format!("a theme named {new:?} already exists"));
    }
    let Some(item) = themes.remove(old) else {
        return Err(format!("no custom theme named {old:?}"));
    };
    themes.insert(new, item);
    for_each_reference_slot(doc, |table, _| {
        if table.get("theme").and_then(|i| i.as_str()) == Some(old) {
            set_value(table, "theme", Value::from(new));
        }
    });
    Ok(())
}

/// Removes `[themes.<name>]`, refusing while anything refers to it.
pub fn delete_theme(doc: &mut DocumentMut, name: &str) -> Result<(), String> {
    let refs = theme_references(doc, name);
    if !refs.is_empty() {
        return Err(format!("{name:?} is in use by {}", refs.join(", ")));
    }
    match themes_table_mut(doc).remove(name) {
        Some(_) => Ok(()),
        None => Err(format!("no custom theme named {name:?}")),
    }
}

/// What refers to the theme `name`: "the global theme" and/or "profile X".
pub fn theme_references(doc: &mut DocumentMut, name: &str) -> Vec<String> {
    let mut refs = Vec::new();
    for_each_reference_slot(doc, |table, what| {
        if table.get("theme").and_then(|i| i.as_str()) == Some(name) {
            refs.push(what);
        }
    });
    refs
}

/// Each custom theme by name, with the error of one that can't be read.
pub type CustomThemes = Vec<(String, Result<Theme, String>)>;

/// The `[themes.*]` tables, each read on its own so one bad theme (or an
/// error elsewhere in the file) doesn't hide the others. Sorted by name.
///
/// A missing `themes` key is no themes; one that isn't a table is a
/// [`Shape`] error, as is a document the plain parser refuses.
pub fn custom_themes(doc: &DocumentMut) -> Result<CustomThemes, Shape> {
    let root = root_table(doc)?;
    let themes = match root.get("themes") {
        None => return Ok(Vec::new()),
        Some(toml::Value::Table(themes)) => themes,
        Some(_) => return Err(Shape::NotATable("themes")),
    };
    // toml::Table is a BTreeMap by default, so this is already sorted.
    Ok(themes
        .iter()
        .map(|(name, value)| (name.clone(), value.clone().try_into::<Theme>().map_err(|e| e.message().to_string())))
        .collect())
}

/// Why a section of the config file can't be read as the settings dialog
/// needs: the file's own shape isn't what the section edits.
#[derive(Debug, PartialEq, Eq)]
pub enum Shape {
    /// The plain TOML parser refuses the document.
    Unparsable(String),
    /// The named key is set, but not to a table.
    NotATable(&'static str),
    /// The named key is set, but not to an array.
    NotAnArray(&'static str),
}

impl Shape {
    /// The message for a section that read the file at `path`.
    pub fn describe(&self, path: &Path) -> String {
        match self {
            Self::Unparsable(why) => format!("{} isn't valid TOML: {why}", path.display()),
            Self::NotATable(key) => format!("{key} in {} is not a table", path.display()),
            Self::NotAnArray(key) => format!("{key} in {} is not an array", path.display()),
        }
    }
}

fn root_table(doc: &DocumentMut) -> Result<toml::Table, Shape> {
    toml::from_str::<toml::Table>(&doc.to_string()).map_err(|e| Shape::Unparsable(e.message().to_string()))
}

/// The config file at `path`, read as `text`, as a document for a section
/// of the settings dialog. A file that doesn't exist reads as an empty one,
/// as `Config::load` treats it; any other failure is a message naming what
/// was being done, the file and the cause.
pub fn document_in(path: &Path, text: &FileText) -> Result<DocumentMut, String> {
    match text {
        Ok(Some(text)) => text.parse::<DocumentMut>().map_err(|e| Shape::Unparsable(e.to_string()).describe(path)),
        Ok(None) => Ok(DocumentMut::new()),
        Err(e) => Err(e.clone()),
    }
}

/// Checks a custom theme name. `others` are the other custom themes' names.
/// Returns the trimmed name.
pub fn validate_theme_name<'a>(name: &str, others: impl IntoIterator<Item = &'a str>) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("The name can't be empty".into());
    }
    if BUILTIN_THEMES.contains(&name) {
        return Err(format!("{name:?} is a built-in theme"));
    }
    if others.into_iter().any(|o| o == name) {
        return Err(format!("A theme named {name:?} already exists"));
    }
    Ok(name.to_string())
}

/// A name for a copy of `base` that isn't in `taken`: `base-custom`, then
/// `base-custom-2`, …
pub fn unused_name<'a>(base: &str, taken: impl IntoIterator<Item = &'a str> + Clone) -> String {
    let stem = base.trim_end_matches(|c: char| c.is_ascii_digit() || c == '-');
    let stem = stem.strip_suffix("-custom").unwrap_or(base);
    let free = |n: &str| !BUILTIN_THEMES.contains(&n) && !taken.clone().into_iter().any(|t| t == n);
    let first = format!("{stem}-custom");
    if free(&first) {
        return first;
    }
    (2..).map(|n| format!("{stem}-custom-{n}")).find(|n| free(n)).unwrap()
}

fn themes_table_mut(doc: &mut DocumentMut) -> &mut dyn TableLike {
    if !doc.get("themes").is_some_and(|i| i.is_table_like()) {
        let mut table = Table::new();
        table.set_implicit(true);
        doc.insert("themes", Item::Table(table));
    }
    doc["themes"].as_table_like_mut().expect("themes table")
}

// ---- profiles ---------------------------------------------------------------

/// Where a profile is in the file, as it was when the dialog read it: its
/// position in `profiles` and its `name` key (`None` when it has none, or
/// a blank one). Profiles have no ids, so every edit checks both before
/// writing.
#[derive(Clone, Debug, PartialEq)]
pub struct ProfileRef {
    pub index: usize,
    pub name: Option<String>,
}

/// The name used when the config defines no profiles (see `Config::normalized`).
pub const IMPLICIT_PROFILE: &str = "Shell";

/// Each profile entry's raw `name` key and the profile read from it, or
/// the error that made it unreadable.
pub type ProfileReads = Vec<(Option<String>, Result<Profile, String>)>;

/// The `profiles` entries in file order, each with its raw `name` key (for
/// a [`ProfileRef`]; `None` when it is missing, not a string or blank) and
/// read on its own, so one bad profile (or an error elsewhere in the file)
/// doesn't hide the others.
///
/// A missing `profiles` key is no profiles; one that isn't an array is a
/// [`Shape`] error, as is a document the plain parser refuses.
pub fn profiles(doc: &DocumentMut) -> Result<ProfileReads, Shape> {
    let root = root_table(doc)?;
    let entries = match root.get("profiles") {
        None => return Ok(Vec::new()),
        Some(toml::Value::Array(entries)) => entries,
        Some(_) => return Err(Shape::NotAnArray("profiles")),
    };
    Ok(entries
        .iter()
        .map(|v| {
            let name = named(v.get("name").and_then(|n| n.as_str()));
            (name, v.clone().try_into::<Profile>().map_err(|e| e.message().to_string()))
        })
        .collect())
}

/// A profile's `name` key as a [`ProfileRef`] holds it: a blank one is
/// none.
fn named(name: Option<&str>) -> Option<String> {
    name.filter(|name| !name.trim().is_empty()).map(str::to_string)
}

/// The `profiles` array in either form.
enum Profiles<'a> {
    Tables(&'a mut ArrayOfTables),
    Inline(&'a mut Array),
}

impl<'a> Profiles<'a> {
    fn len(&self) -> usize {
        match self {
            Profiles::Tables(a) => a.len(),
            Profiles::Inline(a) => a.len(),
        }
    }

    fn entry(self, ix: usize) -> Option<&'a mut dyn TableLike> {
        match self {
            Profiles::Tables(a) => a.get_mut(ix).map(|t| t as &mut dyn TableLike),
            Profiles::Inline(a) => a.get_mut(ix).and_then(|v| v.as_inline_table_mut()).map(|t| t as &mut dyn TableLike),
        }
    }
}

#[allow(
    clippy::wildcard_enum_match_arm,
    reason = "foreign enum: toml_edit::Item, of which only the two profile shapes count"
)]
fn profiles_mut(doc: &mut DocumentMut) -> Option<Profiles<'_>> {
    match doc.get_mut("profiles")? {
        Item::ArrayOfTables(a) => Some(Profiles::Tables(a)),
        Item::Value(Value::Array(a)) => Some(Profiles::Inline(a)),
        _ => None,
    }
}

fn profile_count(doc: &mut DocumentMut) -> usize {
    profiles_mut(doc).map_or(0, |p| p.len())
}

/// Why an edit of a profile was refused: the file no longer has it where
/// the dialog read it.
pub const PROFILES_CHANGED: &str = "The config file changed since the profiles were read; reopen the Profiles section.";

/// The profile at `at`, after checking it is still the one the dialog read.
fn profile_at<'a>(doc: &'a mut DocumentMut, at: &ProfileRef) -> Result<&'a mut dyn TableLike, String> {
    let table = profiles_mut(doc).and_then(|p| p.entry(at.index)).ok_or(PROFILES_CHANGED)?;
    if named(table.get("name").and_then(|i| i.as_str())) != at.name {
        return Err(PROFILES_CHANGED.into());
    }
    Ok(table)
}

/// A config with no profiles gets the implicit "Shell" profile written out,
/// so it stays listed once another profile exists.
pub fn materialize_implicit(doc: &mut DocumentMut) {
    if profile_count(doc) > 0 {
        return;
    }
    let mut table = Table::new();
    table.insert("name", Item::Value(Value::from(IMPLICIT_PROFILE)));
    let mut array = ArrayOfTables::new();
    array.push(table);
    doc.insert("profiles", Item::ArrayOfTables(array));
}

/// Writes `profile` over the profile at `at`, or appends it when `at` is
/// `None`. Blank settings are removed; other keys keep their formatting.
pub fn upsert_profile(doc: &mut DocumentMut, at: Option<&ProfileRef>, profile: &Profile) -> Result<(), String> {
    materialize_implicit(doc);
    let table = match at {
        Some(at) => profile_at(doc, at)?,
        None => {
            let Some(mut profiles) = profiles_mut(doc) else { return Err("`profiles` is not an array".into()) };
            let ix = profiles.len();
            match &mut profiles {
                Profiles::Tables(a) => a.push(Table::new()),
                Profiles::Inline(a) => a.push(InlineTable::new()),
            }
            profiles.entry(ix).expect("new profile")
        }
    };
    write_profile(table, profile);
    Ok(())
}

fn write_profile(table: &mut dyn TableLike, p: &Profile) {
    fn set_or_remove(table: &mut dyn TableLike, key: &str, value: Option<Value>) {
        match value {
            Some(v) => set_value(table, key, v),
            None => _ = table.remove(key),
        }
    }
    fn strings(items: &[String]) -> Option<Value> {
        (!items.is_empty()).then(|| Value::Array(items.iter().map(String::as_str).collect()))
    }
    set_value(table, "name", Value::from(p.name.as_str()));
    if let Some(remote) = &p.remote {
        let mut target = InlineTable::new();
        match remote {
            RemoteTarget::Ssh { host, user, port, identity_file, ssh_args, connect_at_launch: _ } => {
                target.insert("type", Value::from("ssh"));
                target.insert("host", Value::from(host.as_str()));
                if let Some(user) = user { target.insert("user", Value::from(user.as_str())); }
                if let Some(port) = port { target.insert("port", Value::from(i64::from(*port))); }
                if let Some(identity) = identity_file { target.insert("identity_file", Value::from(identity.as_str())); }
                if let Some(args) = strings(ssh_args) { target.insert("ssh_args", args); }
            }
            RemoteTarget::Container { engine, container, user, connect_at_launch: _ } => {
                target.insert("type", Value::from("container"));
                target.insert("engine", Value::from(engine.as_str()));
                target.insert("container", Value::from(container.as_str()));
                if let Some(user) = user { target.insert("user", Value::from(user.as_str())); }
            }
        }
        // Written only when on: off is the default.
        if remote.connects_at_launch() {
            target.insert("connect_at_launch", Value::from(true));
        }
        target.fmt();
        set_value(table, "remote", Value::InlineTable(target));
    } else {
        table.remove("remote");
    }
    set_or_remove(table, "command", p.command.as_deref().map(Value::from));
    set_or_remove(table, "args", strings(&p.args));
    set_or_remove(table, "cwd", p.cwd.as_deref().map(Value::from));
    let env = (!p.env.is_empty()).then(|| {
        let mut t = InlineTable::new();
        for (k, v) in &p.env {
            t.insert(k, Value::from(v.as_str()));
        }
        t.fmt();
        Value::InlineTable(t)
    });
    set_or_remove(table, "env", env);
    set_or_remove(table, "theme", p.theme.as_deref().map(Value::from));
    // Through the shortest decimal, so 13.3 isn't written as 13.300000190734863.
    let font_size = p.font_size.and_then(|s| s.to_string().parse::<f64>().ok()).map(Value::from);
    set_or_remove(table, "font_size", font_size);
    // The removed tmux settings: saving a profile cleans them out of it.
    for key in ["tmux", "tmux_session", "tmux_args"] {
        set_or_remove(table, key, None);
    }
}

/// Rewrites `default_profile` when it names `old` (ignoring case, as
/// lookups do).
pub fn rename_default_profile(doc: &mut DocumentMut, old: &str, new: &str) {
    if doc.get("default_profile").and_then(|i| i.as_str()).is_some_and(|d| d.eq_ignore_ascii_case(old)) {
        set_value(doc.as_table_mut(), "default_profile", Value::from(new));
    }
}

pub fn set_default_profile(doc: &mut DocumentMut, name: &str) {
    set_value(doc.as_table_mut(), "default_profile", Value::from(name));
}

/// Removes the profile at `at` (`name` is its listed name). The last
/// profile can't go; removing the default removes `default_profile`.
pub fn delete_profile(doc: &mut DocumentMut, at: &ProfileRef, name: &str) -> Result<(), String> {
    if profile_count(doc) <= 1 {
        return Err("The last profile can't be deleted".into());
    }
    profile_at(doc, at)?;
    match profiles_mut(doc) {
        Some(Profiles::Tables(a)) => a.remove(at.index),
        Some(Profiles::Inline(a)) => _ = a.remove(at.index),
        None => {}
    }
    if doc.get("default_profile").and_then(|i| i.as_str()).is_some_and(|d| d.eq_ignore_ascii_case(name)) {
        remove_top_level(doc, "default_profile");
    }
    Ok(())
}

/// Removes a top-level key, handing the comments above it to the next
/// top-level key (or the top of the file), so they aren't lost with it.
fn remove_top_level(doc: &mut DocumentMut, key: &str) {
    let root = doc.as_table_mut();
    let prefix = root.key(key).and_then(|k| k.leaf_decor().prefix()).and_then(|p| p.as_str()).unwrap_or("").to_string();
    let keys: Vec<String> = root.iter().map(|(k, _)| k.to_string()).collect();
    let next = keys.iter().skip_while(|k| *k != key).nth(1).filter(|k| root.get(k).is_some_and(|i| i.is_value())).cloned();
    root.remove(key);
    if prefix.is_empty() {
        return;
    }
    let decor = match next.and_then(|next| root.key_mut(&next)) {
        Some(mut k) => {
            let old = k.leaf_decor().prefix().and_then(|p| p.as_str()).unwrap_or("").to_string();
            k.leaf_decor_mut().set_prefix(format!("{prefix}{old}"));
            return;
        }
        None => root.decor_mut(),
    };
    let old = decor.prefix().and_then(|p| p.as_str()).unwrap_or("").to_string();
    decor.set_prefix(format!("{prefix}{old}"));
}

/// Swaps the profile at `at` with its neighbour above (`up`) or below.
/// Returns its new index. Each profile keeps its comments; the places in
/// the file keep theirs.
pub fn move_profile(doc: &mut DocumentMut, at: &ProfileRef, up: bool) -> Result<usize, String> {
    profile_at(doc, at)?;
    let count = profile_count(doc);
    let (i, j) = match up {
        true if at.index > 0 => (at.index - 1, at.index),
        false if at.index + 1 < count => (at.index, at.index + 1),
        _ => return Ok(at.index),
    };
    match profiles_mut(doc) {
        Some(Profiles::Tables(a)) => {
            let (first, second) = (a.get(i).cloned().unwrap(), a.get(j).cloned().unwrap());
            // Tables are written in document position order, so the
            // positions stay where they are and the contents swap.
            let (pi, pj) = (first.position(), second.position());
            let place = |mut t: Table, pos: Option<usize>| {
                if let Some(pos) = pos {
                    t.set_position(pos);
                }
                t
            };
            *a.get_mut(i).unwrap() = place(second, pi);
            *a.get_mut(j).unwrap() = place(first, pj);
        }
        Some(Profiles::Inline(a)) => {
            let first = a.get(i).cloned().unwrap();
            let second = a.get(j).cloned().unwrap();
            let (di, dj) = (first.decor().clone(), second.decor().clone());
            let place = |mut v: Value, decor: toml_edit::Decor| {
                *v.decor_mut() = decor;
                v
            };
            *a.get_mut(i).unwrap() = place(second, di);
            *a.get_mut(j).unwrap() = place(first, dj);
        }
        None => {}
    }
    Ok(if up { i } else { j })
}

/// Calls `f` with each table that may hold a `theme` key (the root, then
/// each profile) and a description of it for messages.
fn for_each_reference_slot(doc: &mut DocumentMut, mut f: impl FnMut(&mut dyn TableLike, String)) {
    f(doc.as_table_mut(), "the global theme".to_string());
    match doc.get_mut("profiles") {
        Some(Item::ArrayOfTables(array)) => {
            for table in array.iter_mut() {
                let what = profile_name(table);
                f(table, what);
            }
        }
        Some(Item::Value(Value::Array(array))) => {
            for value in array.iter_mut() {
                if let Value::InlineTable(table) = value {
                    let what = profile_name(table);
                    f(table, what);
                }
            }
        }
        _ => {}
    }
}

fn profile_name(table: &dyn TableLike) -> String {
    match table.get("name").and_then(|i| i.as_str()) {
        Some(name) => format!("profile {name:?}"),
        None => "a profile".to_string(),
    }
}

// ---- fonts --------------------------------------------------------------------

/// The terminal font's table.
pub const FONT_TABLE: &str = "font";
/// The chrome font's table.
pub const UI_FONT_TABLE: &str = "ui_font";

/// `[<table>]` (`font` or `ui_font`), created when missing. A new
/// `[ui_font]` is placed right after `[font]`.
fn font_table_mut<'a>(doc: &'a mut DocumentMut, table: &str) -> &'a mut dyn TableLike {
    if !doc.get(table).is_some_and(|i| i.is_table_like()) {
        let mut new = Table::new();
        if table == UI_FONT_TABLE
            && let Some(position) = doc.get(FONT_TABLE).and_then(|i| i.as_table()).and_then(|t| t.position())
        {
            // Tables print in position order, and one of equal position
            // after the tables before it in the document.
            new.set_position(position);
        }
        doc.insert(table, Item::Table(new));
    }
    doc[table].as_table_like_mut().expect("font table")
}

/// Puts `family` first in `[<table>].family`, keeping the list's other
/// entries after it (minus case-insensitive copies of `family`) as
/// fallbacks, or, with `None` ("Default"), removes `family` so Brindle's
/// built-in list applies. A table left empty goes too, unless it carries
/// comments.
pub fn set_font_family(doc: &mut DocumentMut, table: &str, family: Option<&str>) {
    let Some(family) = family else {
        let Some(t) = doc.get_mut(table).and_then(|i| i.as_table_like_mut()) else { return };
        let Some(left) = remove_keeping_comments(t, "family") else { return };
        let empty = t.is_empty();
        let Some(t) = doc.get_mut(table).and_then(|i| i.as_table_mut()) else {
            // Dotted keys or an inline table: no header to keep comments on.
            if empty {
                doc.remove(table);
            }
            return;
        };
        if !empty && left.is_empty() {
            return;
        }
        // Comments with no key after them to go to are kept above the
        // header; an empty table without comments goes.
        let header = t.decor().prefix().and_then(|p| p.as_str()).unwrap_or("").to_string();
        if !empty || header.contains('#') || !left.is_empty() {
            t.decor_mut().set_prefix(format!("{header}{left}"));
        } else {
            doc.remove(table);
        }
        return;
    };
    let t = font_table_mut(doc, table);
    let mut list = Array::new();
    list.push(family);
    for other in t.get("family").and_then(|i| i.as_array()).into_iter().flat_map(|a| a.iter()) {
        if let Some(other) = other.as_str().filter(|o| !o.eq_ignore_ascii_case(family)) {
            list.push(other);
        }
    }
    set_value(t, "family", Value::Array(list));
}

/// Sets `[font].<key>` (`size` or `line_height`), whole numbers without a
/// fraction.
pub fn set_font_number(doc: &mut DocumentMut, key: &str, value: f32) {
    let value = if value.fract() == 0.0 && value.abs() < 1e9 {
        Value::from(value as i64)
    } else {
        // The f32's shortest form, not its f64 widening (1.3, not 1.2999…).
        Value::from(value.to_string().parse::<f64>().unwrap_or(value as f64))
    };
    set_value(font_table_mut(doc, FONT_TABLE), key, value);
}

/// Removes `key` from `table`, handing the comments above it to the next
/// key in the table. Returns the comments no key took (empty when there
/// were none, or a key took them), or `None` if `key` wasn't there.
fn remove_keeping_comments(table: &mut dyn TableLike, key: &str) -> Option<String> {
    let prefix = table.key(key)?.leaf_decor().prefix().and_then(|p| p.as_str()).unwrap_or("").to_string();
    let keys: Vec<String> = table.iter().map(|(k, _)| k.to_string()).collect();
    let next = keys.iter().skip_while(|k| *k != key).nth(1).cloned();
    table.remove(key);
    let comments: String = prefix.lines().filter(|l| l.trim_start().starts_with('#')).map(|l| format!("{l}\n")).collect();
    if comments.is_empty() {
        return Some(comments);
    }
    match next.and_then(|next| table.key_mut(&next)) {
        Some(mut next) => {
            let old = next.leaf_decor().prefix().and_then(|p| p.as_str()).unwrap_or("").to_string();
            next.leaf_decor_mut().set_prefix(format!("{comments}{old}"));
            Some(String::new())
        }
        None => Some(comments),
    }
}

/// Sets `key` to `value`, keeping the old value's surrounding whitespace and
/// trailing comment.
fn set_value(table: &mut dyn TableLike, key: &str, mut value: Value) {
    if let Some(old) = table.get(key).and_then(|i| i.as_value()) {
        *value.decor_mut() = old.decor().clone();
    } else {
        value.decor_mut().clear();
    }
    match table.get_mut(key) {
        Some(item) => *item = Item::Value(value),
        None => _ = table.insert(key, Item::Value(value)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Color, builtin_theme};

    const SAMPLE: &str = r##"# My config
default_profile = "Shell"

# Pick a theme.
theme = "mine" # the one I like

[[profiles]]
name = "Shell"

[[profiles]]
name = "Work"
theme = "mine"

# A dark one.
[themes.mine]
background = "#000000" # pure black

[themes.broken]
background = "nope"

# ---- keys ----
[keybindings]
"ctrl-shift-x" = "none"
"##;

    fn doc() -> DocumentMut {
        parse(SAMPLE).unwrap()
    }

    fn reparse(doc: &DocumentMut) -> crate::config::Config {
        crate::config::Config::parse(&doc.to_string()).unwrap_or_else(|e| panic!("{e:#}\n{doc}")).config
    }

    #[test]
    fn global_theme_keeps_comments() {
        let mut doc = doc();
        set_global_theme(&mut doc, "tokyo-night");
        let text = doc.to_string();
        assert_eq!(text, SAMPLE.replace(r#"theme = "mine" # the one"#, r#"theme = "tokyo-night" # the one"#));
    }

    #[test]
    fn global_theme_added_when_missing() {
        let mut doc = parse("# hi\n[keybindings]\n").unwrap();
        set_global_theme(&mut doc, "gruvbox-dark");
        // The comment belongs to the table header, so the key goes above it.
        assert_eq!(doc.to_string(), "theme = \"gruvbox-dark\"\n# hi\n[keybindings]\n");
    }

    #[test]
    fn upsert_existing_theme_keeps_place_and_comments() {
        let mut doc = doc();
        let theme = builtin_theme("tokyo-night").unwrap();
        upsert_theme(&mut doc, "mine", &theme);
        let text = doc.to_string();
        assert!(text.contains("# A dark one.\n[themes.mine]\nbackground = \"#1a1b26\" # pure black\n"), "{text}");
        // Still before [themes.broken] and the keybindings, which are untouched.
        let mine = text.find("[themes.mine]").unwrap();
        assert!(mine < text.find("[themes.broken]").unwrap());
        assert!(text.ends_with("# ---- keys ----\n[keybindings]\n\"ctrl-shift-x\" = \"none\"\n"), "{text}");
        assert!(text.contains("ansi = [\"#15161e\", \"#f7768e\""), "{text}");
        assert!(text.contains("\"#a9b1d6\",\n         \"#414868\""), "{text}");
        let mut doc = doc.clone();
        themes_table_mut(&mut doc).remove("broken");
        let config = reparse(&doc);
        assert_eq!(config.themes["mine"], theme.with_effective_colors());
    }

    #[test]
    fn upsert_new_theme_goes_after_the_last_theme() {
        let mut doc = doc();
        let theme = builtin_theme("gruvbox-dark").unwrap();
        upsert_theme(&mut doc, "new one", &theme);
        let text = doc.to_string();
        let new = text.find("[themes.\"new one\"]").unwrap_or_else(|| panic!("{text}"));
        assert!(new > text.find("[themes.broken]").unwrap());
        assert!(new < text.find("[keybindings]").unwrap());
    }

    #[test]
    fn upsert_into_a_file_without_themes() {
        let mut doc = parse("theme = \"x\"\n\n[keybindings]\n").unwrap();
        let theme = builtin_theme("brindle-light").unwrap();
        upsert_theme(&mut doc, "light", &theme);
        let config = reparse(&doc);
        assert_eq!(config.themes["light"], theme.with_effective_colors());
        assert!(!doc.to_string().contains("[themes]\n"), "{doc}");
    }

    #[test]
    fn cleared_selection_foreground_is_omitted() {
        let mut doc = doc();
        let mut theme = builtin_theme("solarized-dark").unwrap();
        upsert_theme(&mut doc, "sol", &theme);
        assert!(doc.to_string().contains("selection_foreground"));
        theme.selection_foreground = None;
        upsert_theme(&mut doc, "sol", &theme);
        assert!(!doc.to_string().contains("selection_foreground"));
    }

    #[test]
    fn rename_updates_references() {
        let mut doc = doc();
        rename_theme(&mut doc, "mine", "ours").unwrap();
        let text = doc.to_string();
        assert!(!text.contains("mine"), "{text}");
        assert!(text.contains("theme = \"ours\" # the one I like"));
        assert!(text.contains("name = \"Work\"\ntheme = \"ours\""));
        assert!(text.contains("# A dark one.\n[themes.ours]\nbackground = \"#000000\" # pure black"), "{text}");
        assert!(rename_theme(&mut doc, "ours", "broken").is_err());
        assert!(rename_theme(&mut doc, "nothing", "x").is_err());
    }

    #[test]
    fn references_and_delete() {
        let mut doc = doc();
        assert_eq!(theme_references(&mut doc, "mine"), ["the global theme", "profile \"Work\""]);
        assert!(theme_references(&mut doc, "broken").is_empty());
        let err = delete_theme(&mut doc, "mine").unwrap_err();
        assert!(err.contains("profile \"Work\""), "{err}");
        assert!(doc.to_string().contains("[themes.mine]"));
        delete_theme(&mut doc, "broken").unwrap();
        assert!(!doc.to_string().contains("broken"));
        assert!(delete_theme(&mut doc, "broken").is_err());
    }

    #[test]
    fn custom_themes_are_read_one_by_one() {
        // An unknown top-level key makes the whole Config invalid, but the
        // themes are still listed.
        let doc = parse(&format!("fontsize = 3\n{SAMPLE}")).unwrap();
        let themes = custom_themes(&doc).unwrap();
        let names: Vec<&str> = themes.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["broken", "mine"]);
        assert!(themes[0].1.as_ref().unwrap_err().contains("invalid color"));
        assert_eq!(themes[1].1.as_ref().unwrap().background, Color::hex(0));
    }

    #[test]
    fn themes_that_are_not_a_table_are_a_shape_error() {
        let doc = parse("themes = 3\n").unwrap();
        assert_eq!(custom_themes(&doc).unwrap_err(), Shape::NotATable("themes"));
        let doc = parse("themes = [\"a\"]\n").unwrap();
        assert_eq!(custom_themes(&doc).unwrap_err(), Shape::NotATable("themes"));
        assert_eq!(
            Shape::NotATable("themes").describe(Path::new("/c/config.toml")),
            "themes in /c/config.toml is not a table"
        );
    }

    #[test]
    fn profiles_that_are_not_an_array_are_a_shape_error() {
        let doc = parse("profiles = \"x\"\n").unwrap();
        assert_eq!(profiles(&doc).unwrap_err(), Shape::NotAnArray("profiles"));
        let doc = parse("[profiles]\nname = \"x\"\n").unwrap();
        assert_eq!(profiles(&doc).unwrap_err(), Shape::NotAnArray("profiles"));
        assert_eq!(
            Shape::NotAnArray("profiles").describe(Path::new("/c/config.toml")),
            "profiles in /c/config.toml is not an array"
        );
    }

    #[test]
    fn a_missing_key_is_empty() {
        let doc = parse("theme = \"x\"\n").unwrap();
        assert!(custom_themes(&doc).unwrap().is_empty());
        assert!(profiles(&doc).unwrap().is_empty());
        // A file that doesn't exist reads as an empty document.
        let missing = std::io::Error::from(std::io::ErrorKind::NotFound);
        let doc = document_in(Path::new("/c/config.toml"), &text_from(Path::new("/c/config.toml"), Err(missing))).unwrap();
        assert!(profiles(&doc).unwrap().is_empty());
    }

    #[test]
    fn a_document_that_cant_be_read_names_the_path_and_cause() {
        let path = Path::new("/c/config.toml");
        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        let err = document_in(path, &text_from(path, Err(denied))).unwrap_err();
        assert!(err.starts_with("reading /c/config.toml: ") && err.contains("ermission denied"), "{err}");
        let err = document_in(path, &text_from(path, Ok("[broken\n".into()))).unwrap_err();
        assert!(err.starts_with("/c/config.toml isn't valid TOML: "), "{err}");
    }

    #[test]
    fn names() {
        assert_eq!(validate_theme_name("  mine ", ["other"]).unwrap(), "mine");
        assert!(validate_theme_name("   ", []).is_err());
        assert!(validate_theme_name("tokyo-night", []).unwrap_err().contains("built-in"));
        assert!(validate_theme_name("other", ["other"]).unwrap_err().contains("already exists"));
        assert_eq!(unused_name("tokyo-night", ["x"]), "tokyo-night-custom");
        assert_eq!(unused_name("tokyo-night", ["tokyo-night-custom"]), "tokyo-night-custom-2");
        assert_eq!(unused_name("mine-custom-2", ["mine-custom", "mine-custom-2"]), "mine-custom-3");
    }

    #[test]
    fn copies_keep_effective_colors() {
        let tokyo = builtin_theme("tokyo-night").unwrap();
        let copy = Theme { accent: None, cursor_text: None, ..tokyo.clone() }.with_effective_colors();
        assert_eq!(copy.accent, Some(tokyo.ansi[4]));
        assert_ne!(copy.accent, builtin_theme("brindle-dark").unwrap().accent);
        assert_eq!(copy.cursor_text, Some(tokyo.background));
    }

    /// A fresh directory for one test, removed when it ends.
    fn temp_dir(name: &str) -> crate::test_support::TempDir {
        crate::test_support::TempDir::new(&format!("config-edit-{name}"))
    }

    #[test]
    fn save_refuses_malformed_files() {
        let dir = temp_dir("malformed-save");
        let path = dir.join("config.toml");
        let text = "theme = \"x\"\n[broken\n";
        std::fs::write(&path, text).unwrap();
        let err = save(&path, |doc| {
            set_global_theme(doc, "y");
            Ok(())
        })
        .unwrap_err();
        assert!(err.contains("not valid TOML"), "{err}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        assert!(!dir.join("config.toml.tmp").exists());
    }

    #[test]
    fn save_starts_from_the_default_config() {
        let dir = temp_dir("missing");
        let path = dir.join("sub/config.toml");
        save(&path, |doc| {
            set_global_theme(doc, "gruvbox-dark");
            Ok(())
        })
        .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, DEFAULT_CONFIG.replace("theme = \"brindle-dark\"", "theme = \"gruvbox-dark\""));
    }

    #[test]
    fn edits_refused_on_malformed_file() {
        let dir = temp_dir("malformed-edit");
        let path = dir.join("config.toml");
        let text = "theme = \"x\n";
        std::fs::write(&path, text).unwrap();
        let err = save(&path, |doc| {
            set_global_theme(doc, "y");
            Ok(())
        })
        .unwrap_err();
        assert!(err.contains("not valid TOML"), "{err}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }

    // Windows needs Developer Mode or elevation to create symlinks.
    #[cfg(unix)]
    #[test]
    fn save_writes_through_symlinks() {
        let dir = temp_dir("symlink");
        let target = dir.join("real.toml");
        let link = dir.join("config.toml");
        std::fs::write(&target, "theme = \"a\"\n").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        save(&link, |doc| {
            set_global_theme(doc, "b");
            Ok(())
        })
        .unwrap();
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "theme = \"b\"\n");
    }

    #[test]
    fn failed_edit_writes_nothing() {
        let dir = temp_dir("failed-edit");
        let path = dir.join("config.toml");
        std::fs::write(&path, SAMPLE).unwrap();
        assert!(save(&path, |doc| delete_theme(doc, "mine")).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), SAMPLE);
    }

    // ---- profiles ----

    fn shell_ref() -> ProfileRef {
        ProfileRef { index: 0, name: Some("Shell".into()) }
    }

    fn names_of(doc: &DocumentMut) -> Vec<String> {
        profiles(doc).unwrap().into_iter().map(|(_, p)| p.map(|p| p.name).unwrap_or_else(|e| format!("error: {e}"))).collect()
    }

    #[test]
    fn profiles_are_read_one_by_one() {
        let doc = parse("fontsize = 3\n[[profiles]]\nname = \"A\"\n[[profiles]]\nname = 3\n[[profiles]]\nname = \"C\"\ntmux = \"control\"\n").unwrap();
        let read = profiles(&doc).unwrap();
        assert_eq!(read.len(), 3);
        assert_eq!(read[0].0.as_deref(), Some("A"));
        assert_eq!(read[0].1.as_ref().unwrap().name, "A");
        assert_eq!(read[1].0, None);
        assert!(read[1].1.is_err());
        // A removed tmux key doesn't make the profile unreadable.
        assert_eq!(read[2].1.as_ref().unwrap().name, "C");
        let inline = parse("profiles = [{ name = \"X\" }, { name = \"Y\", args = [\"-l\"] }]\n").unwrap();
        assert_eq!(names_of(&inline), ["X", "Y"]);
        assert!(profiles(&parse("").unwrap()).unwrap().is_empty());
    }

    /// A blank `name` key is no name, in the read and in the check before
    /// a write.
    #[test]
    fn a_blank_profile_name_is_none() {
        let mut doc = parse("[[profiles]]\nname = \"  \"\ncwd = \"~\"\n[[profiles]]\nname = \"B\"\n").unwrap();
        let read = profiles(&doc).unwrap();
        assert_eq!((read[0].0.as_deref(), read[1].0.as_deref()), (None, Some("B")));
        let blank = ProfileRef { index: 0, name: None };
        let profile = Profile { name: "A".into(), ..Default::default() };
        upsert_profile(&mut doc, Some(&blank), &profile).unwrap();
        assert_eq!(names_of(&doc), ["A", "B"]);
        let stale = ProfileRef { index: 0, name: None };
        assert_eq!(upsert_profile(&mut doc, Some(&stale), &profile), Err(PROFILES_CHANGED.to_string()));
    }

    #[test]
    fn document_in_reads_a_missing_file_as_empty() {
        let path = Path::new("/c/config.toml");
        assert_eq!(document_in(path, &Ok(None)).unwrap().to_string(), "");
        assert_eq!(document_in(path, &Ok(Some("a = 1\n".into()))).unwrap().to_string(), "a = 1\n");
        let broken = document_in(path, &Ok(Some("[oops\n".into()))).unwrap_err();
        assert!(broken.starts_with("/c/config.toml isn't valid TOML: "), "{broken}");
        assert_eq!(document_in(path, &Err("reading /c/config.toml: denied".into())).unwrap_err(), "reading /c/config.toml: denied");
    }

    #[test]
    fn upsert_keeps_comments_and_removes_blanks() {
        let mut doc = parse("# top\n\n# the shell\n[[profiles]]\nname = \"Shell\"\ncwd = \"~\" # home\ntheme = \"mine\"\n\n[[profiles]]\nname = \"Work\"\n").unwrap();
        let mut p = Profile { name: "Shell".into(), cwd: Some("~/src".into()), font_size: Some(13.3), ..Default::default() };
        p.args = vec!["-l".into()];
        p.env.insert("PAGER".into(), "less -R".into());
        upsert_profile(&mut doc, Some(&shell_ref()), &p).unwrap();
        let text = doc.to_string();
        assert!(text.starts_with("# top\n\n# the shell\n[[profiles]]\nname = \"Shell\"\ncwd = \"~/src\" # home\n"), "{text}");
        assert!(!text.contains("theme"), "{text}");
        assert!(text.contains("font_size = 13.3\n"), "{text}");
        assert!(text.contains("args = [\"-l\"]\n"), "{text}");
        assert!(text.contains("env = { PAGER = \"less -R\" }\n"), "{text}");
        let config = reparse(&doc);
        assert_eq!(config.profiles[0].cwd.as_deref(), Some("~/src"));
        assert_eq!(config.profiles[1].name, "Work");
    }

    #[test]
    fn new_profile_writes_remote_target() {
        let mut doc = parse("[[profiles]]\nname = \"Shell\"\n").unwrap();
        let profile = Profile {
            name: "Remote copy".into(),
            remote: Some(RemoteTarget::Ssh {
                host: "A8".into(),
                user: Some("dev".into()),
                port: Some(2222),
                identity_file: Some("~/.ssh/work".into()),
                ssh_args: vec!["-o".into(), "BatchMode=yes".into()],
                connect_at_launch: false,
            }),
            ..Default::default()
        };
        upsert_profile(&mut doc, None, &profile).unwrap();
        assert_eq!(reparse(&doc).profiles[1].remote, profile.remote);
    }

    /// 8.1: `connect_at_launch` on either kind of target, absent, false or
    /// true in the file, reads back as written when the profile is saved
    /// again, and is written only when true.
    #[test]
    fn connect_at_launch_round_trips_and_is_written_only_when_on() {
        let kinds = [
            ("ssh", "{ type = \"ssh\", host = \"A8\"", "type = \"ssh\", host = \"A8\""),
            ("container", "{ type = \"container\", container = \"devbox\"", "container = \"devbox\""),
        ];
        for (kind, open, written) in kinds {
            for (setting, on) in [("", false), (", connect_at_launch = false", false), (", connect_at_launch = true", true)] {
                let text = format!("[[profiles]]\nname = \"Shell\"\n[[profiles]]\nname = \"R\"\nremote = {open}{setting} }}\n");
                let case = format!("{kind}{setting}");
                let mut doc = parse(&text).unwrap();
                let read = reparse(&doc).profiles[1].clone();
                let target = read.remote.clone().expect("a remote target");
                assert_eq!(target.connects_at_launch(), on, "{case}");
                upsert_profile(&mut doc, Some(&ProfileRef { index: 1, name: Some("R".into()) }), &read).unwrap();
                let saved = doc.to_string();
                assert!(saved.contains(written), "{case}: {saved}");
                assert_eq!(saved.contains("connect_at_launch = true"), on, "{case}: {saved}");
                assert!(!saved.contains("connect_at_launch = false"), "{case}: {saved}");
                assert_eq!(reparse(&doc).profiles[1].remote, Some(target), "{case}");
            }
        }
        // A new profile too, and turning it off removes the key.
        let mut doc = parse("[[profiles]]\nname = \"Shell\"\n").unwrap();
        let mut profile = Profile {
            name: "devbox".into(),
            remote: Some(RemoteTarget::Container {
                engine: "podman".into(),
                container: "devbox".into(),
                user: None,
                connect_at_launch: true,
            }),
            ..Default::default()
        };
        upsert_profile(&mut doc, None, &profile).unwrap();
        assert_eq!(reparse(&doc).profiles[1].remote, profile.remote);
        let at = ProfileRef { index: 1, name: Some("devbox".into()) };
        profile.remote = profile.remote.as_ref().map(RemoteTarget::endpoint);
        upsert_profile(&mut doc, Some(&at), &profile).unwrap();
        assert!(!doc.to_string().contains("connect_at_launch"), "{doc}");
        assert_eq!(reparse(&doc).profiles[1].remote, profile.remote);
    }

    #[test]
    fn saving_a_profile_removes_the_old_tmux_keys() {
        let text = "# top\ntmux_session_end = \"ask\" # old\n\n# the work profile\n[[profiles]]\nname = \"T\" # mine\n\
                    tmux = \"plain\"\ntmux_session = \"s\"\ncommand = \"/bin/zsh\" # zsh\ntmux_args = [\"-L\", \"x\"]\n\n\
                    [[profiles]]\nname = \"U\"\ntmux = \"control\"\n";
        let mut doc = parse(text).unwrap();
        let p = reparse(&doc).profiles[0].clone();
        upsert_profile(&mut doc, Some(&ProfileRef { index: 0, name: Some("T".into()) }), &p).unwrap();
        // Only the saved profile is cleaned; the top-level key is left alone.
        assert_eq!(
            doc.to_string(),
            "# top\ntmux_session_end = \"ask\" # old\n\n# the work profile\n[[profiles]]\nname = \"T\" # mine\n\
             command = \"/bin/zsh\" # zsh\n\n[[profiles]]\nname = \"U\"\ntmux = \"control\"\n"
        );
        // The inline form too.
        let mut inline = parse("profiles = [{ name = \"X\", tmux = \"plain\", tmux_args = [] }]\n").unwrap();
        upsert_profile(&mut inline, Some(&ProfileRef { index: 0, name: Some("X".into()) }), &Profile { name: "X".into(), ..Default::default() }).unwrap();
        assert!(!inline.to_string().contains("tmux"), "{inline}");
        assert_eq!(names_of(&inline), ["X"]);
    }

    #[test]
    fn writes_leave_the_old_tmux_session_end_alone() {
        let text = "theme = \"a\"\ntmux_session_end = \"new_tab\" # old\n";
        let mut doc = parse(text).unwrap();
        set_global_theme(&mut doc, "b");
        assert_eq!(doc.to_string(), "theme = \"b\"\ntmux_session_end = \"new_tab\" # old\n");
        upsert_profile(&mut doc, None, &Profile { name: "Work".into(), ..Default::default() }).unwrap();
        assert!(doc.to_string().contains("tmux_session_end = \"new_tab\" # old\n"), "{doc}");
    }

    #[test]
    fn new_profiles_go_last() {
        let mut doc = doc();
        // Nothing writes the removed tmux settings, whatever the profile holds.
        let p = Profile { name: "New".into(), removed_tmux: Some("control".into()), ..Default::default() };
        upsert_profile(&mut doc, None, &p).unwrap();
        assert_eq!(names_of(&doc), ["Shell", "Work", "New"]);
        let text = doc.to_string();
        assert!(text.contains("[[profiles]]\nname = \"New\"\n"), "{text}");
        assert!(!text.contains("tmux"), "{text}");
        assert!(text.find("name = \"New\"").unwrap() < text.find("[themes.mine]").unwrap(), "{text}");
        // The inline form too.
        let mut inline = parse("profiles = [{ name = \"X\" }]\n").unwrap();
        upsert_profile(&mut inline, None, &p).unwrap();
        assert_eq!(names_of(&inline), ["X", "New"]);
    }

    #[test]
    fn first_profile_on_an_empty_config_keeps_shell() {
        let mut doc = parse("# nothing yet\ntheme = \"mine\"\n").unwrap();
        upsert_profile(&mut doc, None, &Profile { name: "Work".into(), ..Default::default() }).unwrap();
        assert_eq!(names_of(&doc), ["Shell", "Work"]);
        // Editing the implicit Shell writes it out.
        let mut doc = parse("").unwrap();
        let p = Profile { name: "Shell".into(), cwd: Some("~".into()), ..Default::default() };
        upsert_profile(&mut doc, Some(&shell_ref()), &p).unwrap();
        assert_eq!(doc.to_string(), "[[profiles]]\nname = \"Shell\"\ncwd = \"~\"\n");
    }

    #[test]
    fn stale_references_are_refused() {
        let mut doc = doc();
        let p = Profile { name: "X".into(), ..Default::default() };
        let stale = ProfileRef { index: 1, name: Some("Shell".into()) };
        let before = doc.to_string();
        assert!(upsert_profile(&mut doc, Some(&stale), &p).unwrap_err().contains("reopen"));
        assert!(upsert_profile(&mut doc, Some(&ProfileRef { index: 5, name: None }), &p).is_err());
        assert!(delete_profile(&mut doc, &stale, "Shell").is_err());
        assert!(move_profile(&mut doc, &stale, true).is_err());
        assert_eq!(doc.to_string(), before);
        // A profile without a name is matched by its missing name.
        let mut doc = parse("[[profiles]]\ncwd = \"~\"\n").unwrap();
        upsert_profile(&mut doc, Some(&ProfileRef { index: 0, name: None }), &Profile { name: "Profile 1".into(), ..Default::default() }).unwrap();
        assert_eq!(names_of(&doc), ["Profile 1"]);
    }

    #[test]
    fn renaming_updates_the_default() {
        let mut doc = parse("default_profile = \"work\" # mine\n[[profiles]]\nname = \"Work\"\n").unwrap();
        rename_default_profile(&mut doc, "Work", "Job");
        assert_eq!(doc.to_string(), "default_profile = \"Job\" # mine\n[[profiles]]\nname = \"Work\"\n");
        rename_default_profile(&mut doc, "Other", "X");
        assert!(doc.to_string().contains("\"Job\""));
        set_default_profile(&mut doc, "Work");
        assert!(doc.to_string().starts_with("default_profile = \"Work\" # mine\n"));
    }

    #[test]
    fn deleting_profiles() {
        let mut doc = doc();
        // Shell is the default (SAMPLE says "Shell").
        delete_profile(&mut doc, &shell_ref(), "Shell").unwrap();
        assert_eq!(names_of(&doc), ["Work"]);
        assert!(doc.get("default_profile").is_none());
        assert!(doc.to_string().contains("# Pick a theme."));
        assert!(doc.to_string().starts_with("# My config\n"), "{doc}");
        let mut last = parse("# top\ndefault_profile = \"b\"\n[[profiles]]\nname = \"a\"\n[[profiles]]\nname = \"b\"\n").unwrap();
        delete_profile(&mut last, &ProfileRef { index: 1, name: Some("b".into()) }, "b").unwrap();
        assert_eq!(last.to_string(), "# top\n[[profiles]]\nname = \"a\"\n");
        let err = delete_profile(&mut doc, &ProfileRef { index: 0, name: Some("Work".into()) }, "Work").unwrap_err();
        assert!(err.contains("last profile"), "{err}");
        assert!(delete_profile(&mut parse("").unwrap(), &shell_ref(), "Shell").is_err());
        let mut inline = parse("profiles = [{ name = \"X\" }, { name = \"Y\" }]\n").unwrap();
        delete_profile(&mut inline, &ProfileRef { index: 0, name: Some("X".into()) }, "X").unwrap();
        assert_eq!(names_of(&inline), ["Y"]);
    }

    #[test]
    fn moving_profiles_keeps_their_comments() {
        let mut doc = parse("[[profiles]]\nname = \"A\"\n\n# about B\n[[profiles]]\nname = \"B\"\n\n[keybindings]\n").unwrap();
        let ix = move_profile(&mut doc, &ProfileRef { index: 1, name: Some("B".into()) }, true).unwrap();
        assert_eq!(ix, 0);
        assert_eq!(doc.to_string(), "\n# about B\n[[profiles]]\nname = \"B\"\n[[profiles]]\nname = \"A\"\n\n[keybindings]\n");
        // At the ends nothing moves.
        assert_eq!(move_profile(&mut doc, &ProfileRef { index: 0, name: Some("B".into()) }, true).unwrap(), 0);
        let mut inline = parse("profiles = [{ name = \"X\" }, { name = \"Y\" }]\n").unwrap();
        move_profile(&mut inline, &ProfileRef { index: 0, name: Some("X".into()) }, false).unwrap();
        assert_eq!(inline.to_string(), "profiles = [{ name = \"Y\" }, { name = \"X\" }]\n");
    }

    #[test]
    fn moving_up_and_back_round_trips_the_default_config() {
        // The default config has one profile: add a second after it.
        let text = DEFAULT_CONFIG.replace("# font_size = 13\n", "# font_size = 13\n\n[[profiles]]\nname = \"Work\"\n");
        let mut doc = parse(&text).unwrap();
        let work = ProfileRef { index: 1, name: Some("Work".into()) };
        assert_eq!(move_profile(&mut doc, &work, true).unwrap(), 0);
        assert_eq!(names_of(&doc), ["Work", "Shell"]);
        move_profile(&mut doc, &ProfileRef { index: 0, name: Some("Work".into()) }, false).unwrap();
        assert_eq!(doc.to_string(), text);
    }

    #[test]
    fn choosing_a_ui_font_adds_the_table_after_font_and_keeps_comments() {
        let mut doc = parse(DEFAULT_CONFIG).unwrap();
        set_font_family(&mut doc, UI_FONT_TABLE, Some("Inter"));
        let text = doc.to_string();
        let config = reparse(&doc);
        assert_eq!(config.ui_font.family, ["Inter"]);
        assert_eq!(config.font, crate::config::Config::parse(DEFAULT_CONFIG).unwrap().config.font);
        assert!(text.contains("line_height = 1.25\n\n[ui_font]\nfamily = [\"Inter\"]\n"), "{text}");
        // Every comment of the default config is still there.
        for line in DEFAULT_CONFIG.lines().filter(|l| l.starts_with('#')) {
            assert!(text.contains(line), "lost {line:?}");
        }
        // A config without [font] gets [ui_font] at its end.
        let mut doc = parse(SAMPLE).unwrap();
        set_font_family(&mut doc, UI_FONT_TABLE, Some("Inter"));
        assert_eq!(doc.to_string(), format!("{SAMPLE}\n[ui_font]\nfamily = [\"Inter\"]\n"));
    }

    #[test]
    fn choosing_a_listed_fallback_moves_it_to_the_front() {
        let mut doc = parse("[font]\n# Mine.\nfamily = [\"JetBrains Mono\", \"Cascadia Mono\", \"Consolas\"] # in order\nsize = 13\n").unwrap();
        set_font_family(&mut doc, FONT_TABLE, Some("consolas"));
        assert_eq!(
            doc.to_string(),
            "[font]\n# Mine.\nfamily = [\"consolas\", \"JetBrains Mono\", \"Cascadia Mono\"] # in order\nsize = 13\n"
        );
        set_font_family(&mut doc, FONT_TABLE, Some("Cascadia Mono"));
        assert_eq!(reparse(&doc).font.family, ["Cascadia Mono", "consolas", "JetBrains Mono"]);
        // A family not in the list goes in front of all of it.
        let mut doc = parse("[font]\nfamily = [\"JetBrains Mono\", \"Cascadia Mono\"]\n").unwrap();
        set_font_family(&mut doc, FONT_TABLE, Some("Consolas"));
        assert_eq!(reparse(&doc).font.family, ["Consolas", "JetBrains Mono", "Cascadia Mono"]);
    }

    #[test]
    fn default_removes_family_and_an_emptied_table() {
        // The comment above `family` goes to the next key.
        let mut doc = parse(DEFAULT_CONFIG).unwrap();
        set_font_family(&mut doc, FONT_TABLE, None);
        let text = doc.to_string();
        assert!(text.contains("[font]\n# First installed family wins.\nsize = 14\nline_height = 1.25\n"), "{text}");
        assert_eq!(reparse(&doc).font.family, crate::config::FontConfig::default().family);

        // An emptied table goes, with nothing else changed.
        let mut doc = parse(SAMPLE).unwrap();
        set_font_family(&mut doc, UI_FONT_TABLE, Some("Inter"));
        set_font_family(&mut doc, UI_FONT_TABLE, None);
        assert_eq!(doc.to_string(), SAMPLE);
        // Nothing to remove: nothing changes.
        set_font_family(&mut doc, UI_FONT_TABLE, None);
        assert_eq!(doc.to_string(), SAMPLE);

        // A table with comments stays, empty, and keeps them.
        let mut doc = parse("theme = \"x\"\n\n# My chrome font.\n[ui_font]\nfamily = [\"Inter\"]\n").unwrap();
        set_font_family(&mut doc, UI_FONT_TABLE, None);
        assert_eq!(doc.to_string(), "theme = \"x\"\n\n# My chrome font.\n[ui_font]\n");
        // A comment above the last key, with none after it to take it, goes
        // above the header.
        let mut doc = parse("[font]\nsize = 13\n# Mine.\nfamily = [\"Iosevka\"]\n").unwrap();
        set_font_family(&mut doc, FONT_TABLE, None);
        assert_eq!(doc.to_string(), "# Mine.\n[font]\nsize = 13\n");
        // Dotted keys work too.
        let mut doc = parse("ui_font.family = [\"Inter\"]\nfont.size = 13\n").unwrap();
        set_font_family(&mut doc, UI_FONT_TABLE, None);
        assert_eq!(doc.to_string(), "font.size = 13\n");
    }

    #[test]
    fn font_numbers_keep_comments_and_drop_needless_fractions() {
        let mut doc = parse(DEFAULT_CONFIG).unwrap();
        set_font_number(&mut doc, "size", 20.0);
        set_font_number(&mut doc, "line_height", 1.3);
        let text = doc.to_string();
        assert!(text.contains("size = 20\nline_height = 1.3\n"), "{text}");
        assert_eq!(text, DEFAULT_CONFIG.replace("size = 14\nline_height = 1.25\n", "size = 20\nline_height = 1.3\n"));
        let config = reparse(&doc);
        assert_eq!((config.font.size, config.font.line_height), (20.0, 1.3));
        // A trailing comment stays; a missing table is made.
        let mut doc = parse("[font]\nsize = 13 # big enough\n").unwrap();
        set_font_number(&mut doc, "size", 12.5);
        assert_eq!(doc.to_string(), "[font]\nsize = 12.5 # big enough\n");
        let mut doc = parse("theme = \"x\"\n").unwrap();
        set_font_number(&mut doc, "line_height", 1.0);
        assert_eq!(doc.to_string(), "theme = \"x\"\n\n[font]\nline_height = 1\n");
    }

    // ---- ConfigEdit ----

    /// `edit` applied to SAMPLE, and the text it leaves.
    fn applied(edit: &ConfigEdit) -> (Result<Applied, String>, String) {
        let mut doc = doc();
        let result = edit.apply(&mut doc);
        (result, doc.to_string())
    }

    #[test]
    fn edit_sets_the_global_theme() {
        let (result, text) = applied(&ConfigEdit::SetGlobalTheme { name: "tokyo-night".into() });
        assert_eq!(result, Ok(Applied::Done));
        assert_eq!(text, SAMPLE.replace(r#"theme = "mine" # the one"#, r#"theme = "tokyo-night" # the one"#));
    }

    #[test]
    fn edit_saves_a_theme_renaming_it_first() {
        let theme = builtin_theme("gruvbox-dark").unwrap();
        // Under its own name: written in place.
        let (result, text) =
            applied(&ConfigEdit::SaveTheme { original: Some("mine".into()), name: "mine".into(), theme: theme.clone() });
        assert_eq!(result, Ok(Applied::Done));
        assert!(text.contains("# A dark one.\n[themes.mine]\nbackground = \"#282828\" # pure black\n"), "{text}");
        // Renamed: every reference follows.
        let (result, text) =
            applied(&ConfigEdit::SaveTheme { original: Some("mine".into()), name: "ours".into(), theme: theme.clone() });
        assert_eq!(result, Ok(Applied::Done));
        assert!(!text.contains("mine") && text.contains("[themes.ours]"), "{text}");
        assert!(text.contains("name = \"Work\"\ntheme = \"ours\""), "{text}");
        // A new theme, and a rename onto a taken name refused.
        let (_, text) = applied(&ConfigEdit::SaveTheme { original: None, name: "new".into(), theme: theme.clone() });
        assert!(text.contains("[themes.new]") && text.contains("[themes.mine]"), "{text}");
        let refused = ConfigEdit::SaveTheme { original: Some("mine".into()), name: "broken".into(), theme };
        assert!(applied(&refused).0.unwrap_err().contains("already exists"));
    }

    #[test]
    fn edit_deletes_a_theme_only_when_unused() {
        let (result, text) = applied(&ConfigEdit::DeleteTheme { name: "broken".into() });
        assert_eq!(result, Ok(Applied::Done));
        assert!(!text.contains("broken"), "{text}");
        let (result, _) = applied(&ConfigEdit::DeleteTheme { name: "mine".into() });
        assert!(result.unwrap_err().contains("in use by"));
    }

    #[test]
    fn edit_saves_a_profile_and_changes_the_default() {
        let work = ProfileRef { index: 1, name: Some("Work".into()) };
        let profile = Profile { name: "Job".into(), cwd: Some("~".into()), ..Default::default() };
        // Renamed: the default follows only when it named the old name.
        let rename = DefaultChange::Rename { old: "Shell".into(), new: "Job".into() };
        let edit = ConfigEdit::SaveProfile { at: Some(shell_ref()), profile: profile.clone(), default: rename };
        let (result, text) = applied(&edit);
        assert_eq!(result, Ok(Applied::Done));
        assert!(text.starts_with("# My config\ndefault_profile = \"Job\"\n"), "{text}");
        let rename = DefaultChange::Rename { old: "Work".into(), new: "Job".into() };
        let (_, text) = applied(&ConfigEdit::SaveProfile { at: Some(work.clone()), profile: profile.clone(), default: rename });
        assert!(text.starts_with("# My config\ndefault_profile = \"Shell\"\n"), "{text}");
        assert!(text.contains("[[profiles]]\nname = \"Job\"\ncwd = \"~\"\n"), "{text}");
        // Made the default; kept; new.
        let set = DefaultChange::Set("Job".into());
        let (_, text) = applied(&ConfigEdit::SaveProfile { at: Some(work.clone()), profile: profile.clone(), default: set });
        assert!(text.starts_with("# My config\ndefault_profile = \"Job\"\n"), "{text}");
        let keep = ConfigEdit::SaveProfile { at: None, profile, default: DefaultChange::Keep };
        let (result, text) = applied(&keep);
        assert_eq!(result, Ok(Applied::Done));
        assert!(text.starts_with("# My config\ndefault_profile = \"Shell\"\n"), "{text}");
        let mut doc = doc();
        keep.apply(&mut doc).unwrap();
        assert_eq!(names_of(&doc), ["Shell", "Work", "Job"]);
        // A stale reference is refused.
        let stale = ConfigEdit::SaveProfile {
            at: Some(ProfileRef { index: 1, name: Some("Shell".into()) }),
            profile: Profile { name: "X".into(), ..Default::default() },
            default: DefaultChange::Keep,
        };
        assert!(applied(&stale).0.unwrap_err().contains("reopen"));
    }

    #[test]
    fn edit_deletes_a_profile() {
        let edit = ConfigEdit::DeleteProfile { at: ProfileRef { index: 1, name: Some("Work".into()) }, name: "Work".into() };
        let mut doc = doc();
        assert_eq!(edit.apply(&mut doc), Ok(Applied::Done));
        assert_eq!(names_of(&doc), ["Shell"]);
        // The last one can't go.
        assert!(edit.apply(&mut parse("[[profiles]]\nname = \"Work\"\n").unwrap()).unwrap_err().contains("last profile"));
    }

    #[test]
    fn edit_moves_a_profile_and_says_where() {
        let mut doc = doc();
        let up = ConfigEdit::MoveProfile { at: ProfileRef { index: 1, name: Some("Work".into()) }, up: true };
        assert_eq!(up.apply(&mut doc), Ok(Applied::MovedTo(0)));
        assert_eq!(names_of(&doc), ["Work", "Shell"]);
        let edge = ConfigEdit::MoveProfile { at: ProfileRef { index: 0, name: Some("Work".into()) }, up: true };
        assert_eq!(edge.apply(&mut doc), Ok(Applied::MovedTo(0)));
    }

    #[test]
    fn edit_sets_the_fonts_in_order() {
        let mut doc = parse(DEFAULT_CONFIG).unwrap();
        let edit = ConfigEdit::SetFonts(vec![
            FontChange::TerminalFamily(Some("Consolas".into())),
            FontChange::Size(20.0),
            FontChange::LineHeight(1.3),
            FontChange::UiFamily(Some("Inter".into())),
        ]);
        assert_eq!(edit.apply(&mut doc), Ok(Applied::Done));
        let config = reparse(&doc);
        assert_eq!(config.font.family[0], "Consolas");
        assert_eq!((config.font.size, config.font.line_height), (20.0, 1.3));
        assert_eq!(config.ui_font.family, ["Inter"]);
        // "Default" removes the lists.
        let back = ConfigEdit::SetFonts(vec![FontChange::TerminalFamily(None), FontChange::UiFamily(None)]);
        back.apply(&mut doc).unwrap();
        let config = reparse(&doc);
        assert_eq!(config.font.family, crate::config::FontConfig::default().family);
        assert!(config.ui_font.family.is_empty());
    }

    /// What `save` writes, without a file: the default config when there is
    /// none, and nothing for malformed text or a failed edit.
    #[test]
    fn edited_text_is_what_save_writes() {
        let (text, ()) = edited(None, |doc| {
            set_global_theme(doc, "gruvbox-dark");
            Ok(())
        })
        .unwrap();
        assert_eq!(text, DEFAULT_CONFIG.replace("theme = \"brindle-dark\"", "theme = \"gruvbox-dark\""));
        let (text, moved) = edited(Some(SAMPLE), |doc| move_profile(doc, &shell_ref(), false)).unwrap();
        assert_eq!(moved, 1);
        assert_eq!(names_of(&parse(&text).unwrap()), ["Work", "Shell"]);
        assert!(edited(Some("[oops\n"), |_| Ok(())).unwrap_err().contains("not valid TOML"));
        assert!(edited(Some(SAMPLE), |doc| delete_theme(doc, "mine")).is_err());
    }

    #[test]
    fn file_text_reads_a_missing_file_as_none() {
        let path = Path::new("/c/config.toml");
        assert_eq!(text_from(path, Ok("x".into())), Ok(Some("x".to_string())));
        assert_eq!(text_from(path, Err(std::io::Error::from(std::io::ErrorKind::NotFound))), Ok(None));
        let denied = text_from(path, Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))).unwrap_err();
        assert!(denied.starts_with("reading /c/config.toml: "), "{denied}");
    }
}
