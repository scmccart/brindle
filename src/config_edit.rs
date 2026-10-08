//! Edits to `config.toml` made from inside Brindle (the settings dialog).
//!
//! Every edit works on a `toml_edit` document, so the user's comments,
//! ordering and formatting survive; only the keys an edit concerns change.

use std::path::Path;

use toml_edit::{Array, ArrayOfTables, DocumentMut, InlineTable, Item, Table, TableLike, Value};

use crate::config::{DEFAULT_CONFIG, Profile, TmuxMode};
use crate::theme::{BUILTIN_THEMES, Theme};

pub fn parse(text: &str) -> Result<DocumentMut, String> {
    text.parse::<DocumentMut>().map_err(|e| format!("the config file is not valid TOML; fix it first.\n{e}"))
}

/// Reads the config at `path` (the default config if it doesn't exist),
/// applies `edit` and writes it back in one step. Nothing is written if the
/// file isn't valid TOML or `edit` fails.
pub fn save(path: &Path, edit: impl FnOnce(&mut DocumentMut) -> Result<(), String>) -> Result<(), String> {
    // Write through symlinks (dotfile managers) rather than replacing them.
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => DEFAULT_CONFIG.to_string(),
        Err(e) => return Err(format!("reading {}: {e}", path.display())),
    };
    let mut doc = parse(&text)?;
    edit(&mut doc)?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    }
    let mut tmp = path.clone().into_os_string();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    std::fs::write(&tmp, doc.to_string()).map_err(|e| format!("writing {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| {
        std::fs::remove_file(&tmp).ok();
        format!("replacing {}: {e}", path.display())
    })
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

/// The `[themes.*]` tables, each read on its own so one bad theme (or an
/// error elsewhere in the file) doesn't hide the others. Sorted by name.
pub fn custom_themes(doc: &DocumentMut) -> Vec<(String, Result<Theme, String>)> {
    // The document is valid TOML, so the plain parser accepts it too.
    let Ok(root) = toml::from_str::<toml::Table>(&doc.to_string()) else { return Vec::new() };
    let Some(toml::Value::Table(themes)) = root.get("themes") else { return Vec::new() };
    // toml::Table is a BTreeMap by default, so this is already sorted.
    themes
        .iter()
        .map(|(name, value)| (name.clone(), value.clone().try_into::<Theme>().map_err(|e| e.message().to_string())))
        .collect()
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
/// position in `profiles` and its `name` key (`None` when it has none).
/// Profiles have no ids, so every edit checks both before writing.
#[derive(Clone, Debug, PartialEq)]
pub struct ProfileRef {
    pub index: usize,
    pub name: Option<String>,
}

/// The name used when the config defines no profiles (see `Config::normalized`).
pub const IMPLICIT_PROFILE: &str = "Shell";

/// The `profiles` entries in file order, each with its raw `name` key (for
/// a [`ProfileRef`]) and read on its own, so one bad profile (or an error
/// elsewhere in the file) doesn't hide the others.
pub fn profiles(doc: &DocumentMut) -> Vec<(Option<String>, Result<Profile, String>)> {
    let Ok(root) = toml::from_str::<toml::Table>(&doc.to_string()) else { return Vec::new() };
    let Some(toml::Value::Array(entries)) = root.get("profiles") else { return Vec::new() };
    entries
        .iter()
        .map(|v| {
            let name = v.get("name").and_then(|n| n.as_str()).map(str::to_string);
            (name, v.clone().try_into::<Profile>().map_err(|e| e.message().to_string()))
        })
        .collect()
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

/// The profile at `at`, after checking it is still the one the dialog read.
fn profile_at<'a>(doc: &'a mut DocumentMut, at: &ProfileRef) -> Result<&'a mut dyn TableLike, String> {
    const CHANGED: &str = "The config file changed since the profiles were read; reopen the Profiles section.";
    let table = profiles_mut(doc).and_then(|p| p.entry(at.index)).ok_or(CHANGED)?;
    let name = table.get("name").and_then(|i| i.as_str()).map(str::to_string);
    if name != at.name {
        return Err(CHANGED.into());
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
    let tmux = match p.tmux {
        TmuxMode::None => None,
        TmuxMode::Plain => Some(Value::from("plain")),
        TmuxMode::Control => Some(Value::from("control")),
    };
    let is_tmux = tmux.is_some();
    set_or_remove(table, "tmux", tmux);
    set_or_remove(table, "tmux_session", p.tmux_session.as_deref().filter(|_| is_tmux).map(Value::from));
    set_or_remove(table, "tmux_args", if is_tmux { strings(&p.tmux_args) } else { None });
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
        crate::config::Config::parse(&doc.to_string()).unwrap_or_else(|e| panic!("{e:#}\n{doc}"))
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
        let themes = custom_themes(&doc);
        let names: Vec<&str> = themes.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(names, ["broken", "mine"]);
        assert!(themes[0].1.as_ref().unwrap_err().contains("invalid color"));
        assert_eq!(themes[1].1.as_ref().unwrap().background, Color::hex(0));
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

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("brindle-config-edit-{}-{name}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn save_refuses_malformed_files() {
        let dir = temp_dir("malformed");
        let path = dir.join("config.toml");
        let text = "theme = \"x\"\n[broken\n";
        std::fs::write(&path, text).unwrap();
        let err = save(&path, |doc| Ok(set_global_theme(doc, "y"))).unwrap_err();
        assert!(err.contains("not valid TOML"), "{err}");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
        assert!(!dir.join("config.toml.tmp").exists());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn save_starts_from_the_default_config() {
        let dir = temp_dir("missing");
        let path = dir.join("sub/config.toml");
        save(&path, |doc| Ok(set_global_theme(doc, "gruvbox-dark"))).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, DEFAULT_CONFIG.replace("theme = \"brindle-dark\"", "theme = \"gruvbox-dark\""));
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn save_writes_through_symlinks() {
        let dir = temp_dir("symlink");
        let target = dir.join("real.toml");
        let link = dir.join("config.toml");
        std::fs::write(&target, "theme = \"a\"\n").unwrap();
        std::os::unix::fs::symlink(&target, &link).unwrap();
        save(&link, |doc| Ok(set_global_theme(doc, "b"))).unwrap();
        assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(std::fs::read_to_string(&target).unwrap(), "theme = \"b\"\n");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn failed_edit_writes_nothing() {
        let dir = temp_dir("failed-edit");
        let path = dir.join("config.toml");
        std::fs::write(&path, SAMPLE).unwrap();
        assert!(save(&path, |doc| delete_theme(doc, "mine")).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), SAMPLE);
        std::fs::remove_dir_all(dir).ok();
    }

    // ---- profiles ----

    fn shell_ref() -> ProfileRef {
        ProfileRef { index: 0, name: Some("Shell".into()) }
    }

    fn names_of(doc: &DocumentMut) -> Vec<String> {
        profiles(doc).into_iter().map(|(_, p)| p.map(|p| p.name).unwrap_or_else(|e| format!("error: {e}"))).collect()
    }

    #[test]
    fn profiles_are_read_one_by_one() {
        let doc = parse("fontsize = 3\n[[profiles]]\nname = \"A\"\n[[profiles]]\nname = 3\n[[profiles]]\nname = \"C\"\ntmux = \"control\"\n").unwrap();
        let read = profiles(&doc);
        assert_eq!(read.len(), 3);
        assert_eq!(read[0].0.as_deref(), Some("A"));
        assert_eq!(read[0].1.as_ref().unwrap().name, "A");
        assert_eq!(read[1].0, None);
        assert!(read[1].1.is_err());
        assert_eq!(read[2].1.as_ref().unwrap().tmux, TmuxMode::Control);
        let inline = parse("profiles = [{ name = \"X\" }, { name = \"Y\", args = [\"-l\"] }]\n").unwrap();
        assert_eq!(names_of(&inline), ["X", "Y"]);
        assert!(profiles(&parse("").unwrap()).is_empty());
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
    fn tmux_settings_only_for_tmux_profiles() {
        let mut doc = parse("[[profiles]]\nname = \"T\"\ntmux = \"plain\"\ntmux_session = \"s\"\ntmux_args = [\"-L\", \"x\"]\n").unwrap();
        let mut p = reparse(&doc).profiles[0].clone();
        p.tmux = TmuxMode::None;
        upsert_profile(&mut doc, Some(&ProfileRef { index: 0, name: Some("T".into()) }), &p).unwrap();
        assert_eq!(doc.to_string(), "[[profiles]]\nname = \"T\"\n");
    }

    #[test]
    fn new_profiles_go_last() {
        let mut doc = doc();
        let p = Profile { name: "New".into(), tmux: TmuxMode::Control, ..Default::default() };
        upsert_profile(&mut doc, None, &p).unwrap();
        assert_eq!(names_of(&doc), ["Shell", "Work", "New"]);
        let text = doc.to_string();
        assert!(text.contains("[[profiles]]\nname = \"New\"\ntmux = \"control\"\n"), "{text}");
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
        let mut doc = parse(DEFAULT_CONFIG).unwrap();
        let tmux = ProfileRef { index: 1, name: Some("tmux".into()) };
        assert_eq!(move_profile(&mut doc, &tmux, true).unwrap(), 0);
        assert_eq!(names_of(&doc), ["tmux", "Shell", "tmux (classic)"]);
        move_profile(&mut doc, &ProfileRef { index: 0, name: Some("tmux".into()) }, false).unwrap();
        assert_eq!(doc.to_string(), DEFAULT_CONFIG);
    }
}
