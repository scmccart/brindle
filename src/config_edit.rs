//! Edits to `config.toml` made from inside Brindle (the settings dialog).
//!
//! Every edit works on a `toml_edit` document, so the user's comments,
//! ordering and formatting survive; only the keys an edit concerns change.

use std::path::Path;

use toml_edit::{Array, DocumentMut, Item, Table, TableLike, Value};

use crate::config::DEFAULT_CONFIG;
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
}
