//! The settings dialog's Theme section: the theme list (every built-in,
//! then the file's custom themes), and an editor for a custom theme.
//!
//! [`ThemesState`] holds the state and every rule about it and has no GPUI
//! dependency: it reads the themes from the config file's text it is given
//! and returns the [`ConfigEdit`] to write, never touching the file. The
//! view half is in the parent module.

use std::collections::BTreeMap;
use std::path::Path;

use crate::config_edit::{self, ConfigEdit, FileText};
use crate::theme::{BUILTIN_THEMES, Color, Theme, builtin_theme};

use super::{Btn, ListMode, Reply, Step};

/// A theme in the list.
#[derive(Clone, Debug)]
pub struct ThemeEntry {
    pub name: String,
    pub custom: bool,
    /// A custom theme that can't be read carries its error.
    pub theme: Result<Theme, String>,
}

impl ThemeEntry {
    /// Where the theme comes from, as the dump and an accessible row name
    /// it: `built-in` (the lock in the list) or `custom` (the file's).
    pub fn tag(&self) -> &'static str {
        if self.custom { "custom" } else { "built-in" }
    }
}

/// A theme's identity in the list across a reread: its name, and whether
/// it is the file's (a custom theme may share a built-in's name, and
/// shadows it).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThemeKey {
    pub name: String,
    pub custom: bool,
}

impl ThemeKey {
    fn of(entry: &ThemeEntry) -> Self {
        Self { name: entry.name.clone(), custom: entry.custom }
    }

    fn names(&self, entry: &ThemeEntry) -> bool {
        entry.name == self.name && entry.custom == self.custom
    }
}

/// The Theme section's list: every built-in theme, then the file's custom
/// themes in name order. When the file (`path`, read as `read`) can't be
/// read as themes, the custom themes are the `fallback` (the config in
/// effect's), and the message says why: it names the file and the cause.
pub fn theme_entries(
    path: &Path,
    read: Result<toml_edit::DocumentMut, String>,
    fallback: &BTreeMap<String, Theme>,
) -> (Vec<ThemeEntry>, Option<String>) {
    let (customs, note) = match read.and_then(|doc| config_edit::custom_themes(&doc).map_err(|shape| shape.describe(path))) {
        Ok(customs) => (customs, None),
        Err(note) => (fallback.iter().map(|(name, theme)| (name.clone(), Ok(theme.clone()))).collect(), Some(note)),
    };
    let entries = BUILTIN_THEMES
        .iter()
        .map(|name| ThemeEntry {
            name: name.to_string(),
            custom: false,
            theme: Ok(builtin_theme(name).expect("every name in BUILTIN_THEMES is a built-in theme")),
        })
        .chain(customs.into_iter().map(|(name, theme)| ThemeEntry { name, custom: true, theme }))
        .collect();
    (entries, note)
}

/// A color a theme editor field edits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Slot {
    Foreground,
    Background,
    Cursor,
    CursorText,
    SelectionBackground,
    SelectionForeground,
    Accent,
    SearchMatchBackground,
    SearchMatchForeground,
    SearchFocusedBackground,
    SearchFocusedForeground,
    Ansi(usize),
}

const ANSI_NAMES: [&str; 16] = [
    "Black",
    "Red",
    "Green",
    "Yellow",
    "Blue",
    "Magenta",
    "Cyan",
    "White",
    "Bright black",
    "Bright red",
    "Bright green",
    "Bright yellow",
    "Bright blue",
    "Bright magenta",
    "Bright cyan",
    "Bright white",
];

pub const SLOTS: [Slot; 27] = [
    Slot::Foreground,
    Slot::Background,
    Slot::Cursor,
    Slot::CursorText,
    Slot::SelectionBackground,
    Slot::SelectionForeground,
    Slot::Accent,
    Slot::SearchMatchBackground,
    Slot::SearchMatchForeground,
    Slot::SearchFocusedBackground,
    Slot::SearchFocusedForeground,
    Slot::Ansi(0),
    Slot::Ansi(1),
    Slot::Ansi(2),
    Slot::Ansi(3),
    Slot::Ansi(4),
    Slot::Ansi(5),
    Slot::Ansi(6),
    Slot::Ansi(7),
    Slot::Ansi(8),
    Slot::Ansi(9),
    Slot::Ansi(10),
    Slot::Ansi(11),
    Slot::Ansi(12),
    Slot::Ansi(13),
    Slot::Ansi(14),
    Slot::Ansi(15),
];

impl Slot {
    pub fn label(self) -> &'static str {
        match self {
            Slot::Foreground => "Foreground",
            Slot::Background => "Background",
            Slot::Cursor => "Cursor",
            Slot::CursorText => "Cursor text",
            Slot::SelectionBackground => "Selection background",
            Slot::SelectionForeground => "Selection foreground",
            Slot::Accent => "Accent",
            Slot::SearchMatchBackground => "Search match",
            Slot::SearchMatchForeground => "Search match text",
            Slot::SearchFocusedBackground => "Current match",
            Slot::SearchFocusedForeground => "Current match text",
            Slot::Ansi(i) => ANSI_NAMES[i],
        }
    }

    /// Only the selection foreground may be left empty ("keep each cell's
    /// own color"); see design decision 4.
    pub fn optional(self) -> bool {
        self == Slot::SelectionForeground
    }

    pub fn get(self, theme: &Theme) -> Option<Color> {
        match self {
            Slot::Foreground => Some(theme.foreground),
            Slot::Background => Some(theme.background),
            Slot::Cursor => Some(theme.cursor),
            Slot::CursorText => theme.cursor_text,
            Slot::SelectionBackground => Some(theme.selection_background),
            Slot::SelectionForeground => theme.selection_foreground,
            Slot::Accent => theme.accent,
            Slot::SearchMatchBackground => theme.search_match_background,
            Slot::SearchMatchForeground => theme.search_match_foreground,
            Slot::SearchFocusedBackground => theme.search_focused_match_background,
            Slot::SearchFocusedForeground => theme.search_focused_match_foreground,
            Slot::Ansi(i) => Some(theme.ansi[i]),
        }
    }

    fn set(self, theme: &mut Theme, color: Option<Color>) {
        match (self, color) {
            (Slot::SelectionForeground, c) => theme.selection_foreground = c,
            (
                Slot::Foreground
                | Slot::Background
                | Slot::Cursor
                | Slot::CursorText
                | Slot::SelectionBackground
                | Slot::Accent
                | Slot::SearchMatchBackground
                | Slot::SearchMatchForeground
                | Slot::SearchFocusedBackground
                | Slot::SearchFocusedForeground
                | Slot::Ansi(_),
                None,
            ) => {}
            (Slot::Foreground, Some(c)) => theme.foreground = c,
            (Slot::Background, Some(c)) => theme.background = c,
            (Slot::Cursor, Some(c)) => theme.cursor = c,
            (Slot::CursorText, Some(c)) => theme.cursor_text = Some(c),
            (Slot::SelectionBackground, Some(c)) => theme.selection_background = c,
            (Slot::Accent, Some(c)) => theme.accent = Some(c),
            (Slot::SearchMatchBackground, Some(c)) => theme.search_match_background = Some(c),
            (Slot::SearchMatchForeground, Some(c)) => theme.search_match_foreground = Some(c),
            (Slot::SearchFocusedBackground, Some(c)) => theme.search_focused_match_background = Some(c),
            (Slot::SearchFocusedForeground, Some(c)) => theme.search_focused_match_foreground = Some(c),
            (Slot::Ansi(i), Some(c)) => theme.ansi[i] = c,
        }
    }
}

/// A field of the theme editor: the name, or the color of [`SLOTS`]`[i]`.
/// It owns the one conversion between the editor's fields, in Tab order,
/// and the color slots.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FieldId {
    Name,
    Color(usize),
}

impl FieldId {
    /// Every field, in Tab order: the name, then each color.
    pub fn all() -> impl Iterator<Item = FieldId> {
        std::iter::once(FieldId::Name).chain((0..SLOTS.len()).map(FieldId::Color))
    }

    /// The color's slot; `None` for the name, or past the last slot.
    pub fn slot(self) -> Option<Slot> {
        match self {
            FieldId::Name => None,
            FieldId::Color(ix) => SLOTS.get(ix).copied(),
        }
    }

    /// The selection foreground's field, which Clear empties.
    pub fn selection_foreground() -> FieldId {
        let ix = SLOTS.iter().position(|s| *s == Slot::SelectionForeground);
        FieldId::Color(ix.expect("SLOTS has the selection foreground"))
    }
}

/// A theme editor field's name in the `--dump-screen-after` line: `name`
/// for the name, else its color's label; `None` past the last slot.
pub fn field_label(field: FieldId) -> Option<&'static str> {
    match field {
        FieldId::Name => Some("name"),
        FieldId::Color(_) => field.slot().map(Slot::label),
    }
}

/// The custom-theme editor.
#[derive(Clone, Debug)]
pub struct Editor {
    /// The saved custom theme being edited; `None` for a new one.
    pub original: Option<String>,
    /// The list selection to go back to on discard.
    return_to: Option<ThemeKey>,
    pub name: String,
    /// One field per [`SLOTS`] entry.
    colors: Vec<String>,
    /// The theme as of the last valid edit of every field.
    pub current: Theme,
    /// The name and colors as the editor opened, to tell unsaved edits.
    initial: (String, Vec<String>),
}

impl Editor {
    fn new(original: Option<String>, name: String, theme: &Theme, return_to: Option<ThemeKey>) -> Self {
        let current = theme.with_effective_colors();
        let colors: Vec<String> = SLOTS
            .iter()
            .map(|slot| slot.get(&current).map(|c| c.to_string()).unwrap_or_default())
            .collect();
        let initial = (name.clone(), colors.clone());
        Self { original, return_to, name, colors, current, initial }
    }

    /// Parses the color of slot `ix`: `Ok(None)` is a cleared optional
    /// color.
    pub fn parse_color(&self, ix: usize) -> Result<Option<Color>, &'static str> {
        let text = self.colors[ix].trim();
        if text.is_empty() && SLOTS[ix].optional() {
            return Ok(None);
        }
        Color::parse(text).map(Some).ok_or("use #rrggbb or #rgb")
    }

    fn invalid_colors(&self) -> Vec<usize> {
        (0..SLOTS.len()).filter(|&ix| self.parse_color(ix).is_err()).collect()
    }

    fn name_result(&self, others: &[&str]) -> Result<String, String> {
        config_edit::validate_theme_name(&self.name, others.iter().copied())
    }

    /// Sets a field and updates the preview theme if it now holds a valid
    /// color. A color past the last slot changes nothing.
    fn set_field(&mut self, field: FieldId, text: &str) {
        match field {
            FieldId::Name => self.name = text.to_string(),
            FieldId::Color(ix) => {
                let Some(slot) = field.slot() else { return };
                self.colors[ix] = text.to_string();
                if let Ok(color) = self.parse_color(ix) {
                    slot.set(&mut self.current, color);
                }
            }
        }
    }

    fn dirty(&self) -> bool {
        self.name != self.initial.0 || self.colors != self.initial.1
    }

    /// A field's text.
    pub fn field(&self, field: FieldId) -> &str {
        match field {
            FieldId::Name => &self.name,
            FieldId::Color(ix) => self.colors.get(ix).map_or("", String::as_str),
        }
    }

    /// The editor's heading.
    pub fn title(&self) -> String {
        match &self.original {
            Some(name) => format!("Edit {name:?}"),
            None => "New theme".to_string(),
        }
    }
}

#[derive(Clone, Debug)]
#[allow(
    clippy::large_enum_variant,
    reason = "one value, the Theme section's mode, held by the open dialog; boxing the editor adds an indirection to every field read"
)]
enum Mode {
    List,
    Editor(Editor),
    /// Asking whether to delete the selected custom theme.
    ConfirmDelete,
}

/// The Theme section's state: the list, its selection by [`ThemeKey`], the
/// active theme, and the editor or delete question when one shows.
pub struct ThemesState {
    entries: Vec<ThemeEntry>,
    /// The global theme's name when the dialog opened (renamed with it).
    active: String,
    selected: Option<ThemeKey>,
    mode: Mode,
    /// Why the file's themes couldn't be read, when they couldn't: the
    /// section's error line says it whenever it has nothing newer.
    note: Option<String>,
    /// Custom themes to list when the file can't be read.
    fallback: BTreeMap<String, Theme>,
}

impl ThemesState {
    /// The list read from `text` (the file at `path`), with the `active`
    /// theme selected, or brindle-dark when it can't be.
    pub fn new(path: &Path, text: &FileText, active: String, fallback: BTreeMap<String, Theme>) -> Self {
        let mut state = Self {
            entries: Vec::new(),
            active,
            selected: None,
            mode: Mode::List,
            note: None,
            fallback,
        };
        state.reload(path, text);
        state.select_active();
        state
    }

    /// Rereads the custom themes from the file's text. When it can't be
    /// read as themes, the list has the config in effect's, and [`Self::note`]
    /// says why.
    fn reload(&mut self, path: &Path, text: &FileText) {
        let (entries, note) = theme_entries(path, config_edit::document_in(path, text), &self.fallback);
        self.entries = entries;
        self.note = note;
    }

    pub fn note(&self) -> Option<&String> {
        self.note.as_ref()
    }

    pub fn entries(&self) -> &[ThemeEntry] {
        &self.entries
    }

    #[cfg(test)]
    pub fn active(&self) -> &str {
        &self.active
    }

    pub fn list_mode(&self) -> ListMode {
        match self.mode {
            Mode::List => ListMode::List,
            Mode::Editor(_) => ListMode::Editor,
            Mode::ConfirmDelete => ListMode::ConfirmDelete,
        }
    }

    pub fn editor(&self) -> Option<&Editor> {
        match &self.mode {
            Mode::Editor(editor) => Some(editor),
            Mode::List | Mode::ConfirmDelete => None,
        }
    }

    fn select_index(&mut self, ix: usize) {
        if let Some(entry) = self.entries.get(ix) {
            self.selected = Some(ThemeKey::of(entry));
        }
    }

    /// Selects the active theme, else brindle-dark, else the first.
    fn select_active(&mut self) {
        let ix = self.selectable_index(&self.active).or_else(|| self.selectable_index("brindle-dark")).unwrap_or(0);
        self.select_index(ix);
    }

    /// The index of the readable entry `name` (custom themes shadow
    /// built-ins of the same name, as in the config).
    fn selectable_index(&self, name: &str) -> Option<usize> {
        let matches = |e: &ThemeEntry| e.name == name && e.theme.is_ok();
        self.entries.iter().rposition(matches)
    }

    /// Where the selected theme is in the list.
    pub fn selected_index(&self) -> Option<usize> {
        let selected = self.selected.as_ref()?;
        self.entries.iter().position(|e| selected.names(e))
    }

    pub fn selected_entry(&self) -> Option<&ThemeEntry> {
        self.entries.get(self.selected_index()?)
    }

    /// The row marked active: the readable theme the global theme names.
    pub fn active_row(&self) -> Option<usize> {
        self.selectable_index(&self.active)
    }

    fn custom_names(&self) -> Vec<&str> {
        self.entries.iter().filter(|e| e.custom).map(|e| e.name.as_str()).collect()
    }

    /// Every theme a profile can choose: built-ins, then custom themes,
    /// each readable one once.
    pub fn theme_names(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for entry in self.entries.iter().filter(|e| e.theme.is_ok()) {
            if !names.contains(&entry.name) {
                names.push(entry.name.clone());
            }
        }
        names
    }

    /// The theme the tabs should show right now.
    pub fn preview(&self) -> Option<Theme> {
        match &self.mode {
            Mode::Editor(editor) => Some(editor.current.clone()),
            Mode::List | Mode::ConfirmDelete => self.selected_entry()?.theme.as_ref().ok().cloned(),
        }
    }

    /// Up and down in the list: the next readable theme, wrapping. Quiet
    /// when nothing moved (the editor's fields are focus stops: the view
    /// moves focus).
    pub fn move_selection(&mut self, forward: bool) -> Reply {
        match &self.mode {
            Mode::List => {
                let count = self.entries.len();
                let mut ix = self.selected_index().unwrap_or(0);
                for _ in 0..count {
                    ix = if forward { (ix + 1) % count } else { (ix + count - 1) % count };
                    if self.entries[ix].theme.is_ok() {
                        self.select_index(ix);
                        return Reply::Done(Step::Nothing);
                    }
                }
                Reply::Quiet
            }
            Mode::Editor(_) | Mode::ConfirmDelete => Reply::Quiet,
        }
    }

    /// Home and end: the first or `last` readable theme.
    pub fn select_edge(&mut self, last: bool) -> Reply {
        let readable = |ix: &usize| self.entries[*ix].theme.is_ok();
        let mut ixs = 0..self.entries.len();
        let found = if last { ixs.rfind(readable) } else { ixs.find(readable) };
        match found {
            Some(ix) => self.select(ix),
            None => Reply::Quiet,
        }
    }

    /// A click on row `ix`: selects it in the list when it is readable.
    pub fn select(&mut self, ix: usize) -> Reply {
        if matches!(self.mode, Mode::List) && self.entries.get(ix).is_some_and(|e| e.theme.is_ok()) {
            self.select_index(ix);
            return Reply::Done(Step::Nothing);
        }
        Reply::Quiet
    }

    /// Leaving the section: the list again, on the active theme, dropping
    /// an unapplied preview.
    pub fn leave(&mut self) {
        self.mode = Mode::List;
        self.select_active();
    }

    /// Whether the editor holds unsaved edits.
    pub fn dirty(&self) -> bool {
        matches!(&self.mode, Mode::Editor(e) if e.dirty())
    }

    /// Enter: apply in the list, save in the editor, delete when
    /// confirming.
    pub fn confirm(&mut self) -> Reply {
        match &self.mode {
            Mode::List => match self.selected_entry().filter(|e| e.theme.is_ok()) {
                Some(entry) => Reply::Done(Step::Write(ConfigEdit::SetGlobalTheme { name: entry.name.clone() })),
                None => Reply::Quiet,
            },
            Mode::Editor(editor) => self.save_editor(editor),
            Mode::ConfirmDelete => {
                let Some(name) = self.selected_entry().map(|e| e.name.clone()) else { return Reply::Quiet };
                self.mode = Mode::List;
                Reply::Done(Step::Write(ConfigEdit::DeleteTheme { name }))
            }
        }
    }

    fn save_editor(&self, editor: &Editor) -> Reply {
        let name = match editor.name_result(&self.other_names(editor)) {
            Ok(name) => name,
            Err(e) => return Reply::Refused(e),
        };
        if !editor.invalid_colors().is_empty() {
            return Reply::Refused("Fix the colors marked invalid first".into());
        }
        let edit = ConfigEdit::SaveTheme { original: editor.original.clone(), name, theme: editor.current.clone() };
        Reply::Done(Step::Write(edit))
    }

    /// The edit this section asked for was written, and the file now reads
    /// `text`: back to the list, on the saved theme or near the deleted one.
    pub fn written(&mut self, edit: &ConfigEdit, path: &Path, text: &FileText) {
        match edit {
            ConfigEdit::SaveTheme { original, name, .. } => {
                if original.as_deref() == Some(self.active.as_str()) {
                    // A rename carries the global theme along.
                    self.active = name.clone();
                }
                self.mode = Mode::List;
                self.reload(path, text);
                let ix = self.selectable_index(name).unwrap_or(0);
                self.select_index(ix);
            }
            ConfigEdit::DeleteTheme { .. } => {
                let previous = self.selected_index().unwrap_or(0);
                self.reload(path, text);
                // Stay near where the theme was.
                let mut ix = previous.min(self.entries.len().saturating_sub(1));
                if self.entries[ix].theme.is_err() {
                    ix = self.selectable_index(&self.active).unwrap_or(0);
                }
                self.select_index(ix);
            }
            ConfigEdit::SetGlobalTheme { .. }
            | ConfigEdit::SaveProfile { .. }
            | ConfigEdit::DeleteProfile { .. }
            | ConfigEdit::MoveProfile { .. }
            | ConfigEdit::SetFonts(_) => {}
        }
    }

    /// Escape: closes in the list, discards in the editor, keeps the theme
    /// when asked about deleting it.
    pub fn cancel(&mut self) -> Reply {
        match std::mem::replace(&mut self.mode, Mode::List) {
            Mode::List => Reply::Done(Step::Close),
            Mode::Editor(editor) => {
                self.selected = editor.return_to;
                Reply::Done(Step::Nothing)
            }
            Mode::ConfirmDelete => Reply::Quiet,
        }
    }

    /// New: a copy of the selected theme under an unused name.
    pub fn start_new(&mut self) -> Reply {
        if !matches!(self.mode, Mode::List) {
            return Reply::Quiet;
        }
        let Some(ThemeEntry { name, theme: Ok(theme), .. }) = self.selected_entry().cloned() else { return Reply::Quiet };
        let name = config_edit::unused_name(&name, self.custom_names());
        self.mode = Mode::Editor(Editor::new(None, name, &theme, self.selected.clone()));
        Reply::Done(Step::Nothing)
    }

    /// Edit: a custom theme in place. A built-in is read-only: the refusal
    /// says New makes an editable copy.
    pub fn start_edit(&mut self) -> Reply {
        if !matches!(self.mode, Mode::List) {
            return Reply::Quiet;
        }
        match self.selected_entry().cloned() {
            Some(ThemeEntry { name, custom: true, theme: Ok(theme) }) => {
                self.mode = Mode::Editor(Editor::new(Some(name.clone()), name, &theme, self.selected.clone()));
                Reply::Done(Step::Nothing)
            }
            Some(ThemeEntry { name, custom: false, .. }) => {
                Reply::Refused(format!("{name:?} is built in: New makes an editable copy"))
            }
            Some(ThemeEntry { custom: true, theme: Err(_), .. }) | None => Reply::Quiet,
        }
    }

    pub fn editor_can_save(&self) -> bool {
        let Mode::Editor(editor) = &self.mode else { return false };
        editor.invalid_colors().is_empty() && editor.name_result(&self.other_names(editor)).is_ok()
    }

    /// The other custom themes' names, which the edited one can't take.
    fn other_names(&self, editor: &Editor) -> Vec<&str> {
        self.custom_names().into_iter().filter(|n| Some(*n) != editor.original.as_deref()).collect()
    }

    /// The editor's name error, if its name can't be saved.
    pub fn name_error(&self, editor: &Editor) -> Option<String> {
        editor.name_result(&self.other_names(editor)).err()
    }

    /// Delete: asks for confirmation, or says why the theme can't go. The
    /// file's `text` tells what still refers to it.
    pub fn request_delete(&mut self, text: &FileText) -> Reply {
        if !matches!(self.mode, Mode::List) {
            return Reply::Quiet;
        }
        let Some(entry) = self.selected_entry() else { return Reply::Quiet };
        if !entry.custom {
            return Reply::Refused(format!("{:?} is built in and can't be deleted", entry.name));
        }
        let name = entry.name.clone();
        let refs = text
            .clone()
            .and_then(|text| config_edit::parse(text.as_deref().unwrap_or("")))
            .map(|mut doc| config_edit::theme_references(&mut doc, &name));
        match refs {
            Ok(refs) if refs.is_empty() => {
                self.mode = Mode::ConfirmDelete;
                Reply::Done(Step::Nothing)
            }
            Ok(refs) => Reply::Refused(format!("{name:?} is in use by {}", refs.join(", "))),
            Err(e) => Reply::Refused(e),
        }
    }

    /// A field's text changed in its input.
    pub fn set_field(&mut self, field: FieldId, text: &str) -> Reply {
        match &mut self.mode {
            Mode::Editor(editor) => {
                editor.set_field(field, text);
                Reply::Done(Step::Nothing)
            }
            Mode::List | Mode::ConfirmDelete => Reply::Quiet,
        }
    }

    /// The editor's fields and their texts, in Tab order, when editing.
    pub fn editor_fields(&self) -> Option<Vec<(FieldId, String)>> {
        let editor = self.editor()?;
        Some(FieldId::all().map(|field| (field, editor.field(field).to_string())).collect())
    }

    /// Clears the selection foreground; returns its field.
    pub fn clear_selection_foreground(&mut self) -> Option<FieldId> {
        let Mode::Editor(editor) = &mut self.mode else { return None };
        let field = FieldId::selection_foreground();
        editor.set_field(field, "");
        Some(field)
    }

    /// Whether the editor's selection foreground holds a color, so Clear
    /// has something to clear.
    pub fn selection_foreground_set(&self) -> bool {
        self.editor().is_some_and(|editor| Slot::SelectionForeground.get(&editor.current).is_some())
    }

    /// Whether button `b` can be pressed (and focused) in this section.
    pub fn enabled(&self, b: Btn) -> bool {
        match b {
            Btn::Delete => self.selected_entry().is_some_and(|e| e.custom),
            Btn::Edit => self.selected_entry().is_some_and(|e| e.custom && e.theme.is_ok()),
            Btn::Clear => self.selection_foreground_set(),
            Btn::Save => self.editor_can_save(),
            Btn::New
            | Btn::Duplicate
            | Btn::Up
            | Btn::Down
            | Btn::Cancel
            | Btn::Apply
            | Btn::Close
            | Btn::Keep
            | Btn::ConfirmDelete
            | Btn::Discard => true,
        }
    }

    /// The section's `--- settings` line. `field` names the focused
    /// control, reported as the editor's `field=`.
    pub fn describe(&self, error: &Option<String>, field: &str) -> String {
        let section = "Theme";
        let selected = self.selected_entry().map(|e| e.name.as_str()).unwrap_or("");
        match &self.mode {
            Mode::List => format!(
                "--- settings section={section} mode=list selected={selected:?} active={:?} error={error:?} origin={}\n",
                self.active,
                self.selected_entry().map_or("none", ThemeEntry::tag),
            ),
            Mode::Editor(editor) => format!(
                "--- settings section={section} mode=editor name={:?} field={field:?} invalid={:?} can_save={} error={error:?}\n",
                editor.name,
                editor.invalid_colors().iter().map(|&ix| SLOTS[ix].label()).collect::<Vec<_>>(),
                self.editor_can_save(),
            ),
            Mode::ConfirmDelete => format!("--- settings section={section} mode=confirm-delete theme={selected:?}\n"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r##"# test config
theme = "mine"

[[profiles]]
name = "Shell"

[themes.mine]
background = "#000000"

[themes.spare]
background = "#111111"

[themes.broken]
background = "nope"
"##;

    fn path() -> &'static Path {
        Path::new("/c/config.toml")
    }

    fn state(text: &str, active: &str) -> ThemesState {
        ThemesState::new(path(), &Ok(Some(text.to_string())), active.into(), BTreeMap::new())
    }

    fn selected(state: &ThemesState) -> &str {
        &state.selected_entry().unwrap().name
    }

    fn written(edit: &str) -> Step {
        Step::Write(ConfigEdit::SetGlobalTheme { name: edit.into() })
    }

    /// A file that can't be read as themes lists the config in effect's
    /// themes, and the message names the file and the cause.
    #[test]
    fn theme_entries_name_the_path_and_cause() {
        let path = path();
        let fallback = BTreeMap::from([("mine".to_string(), Theme::default())]);
        let customs = |entries: &[ThemeEntry]| -> Vec<String> {
            entries.iter().filter(|e| e.custom).map(|e| e.name.clone()).collect()
        };
        let document = |read| config_edit::document_in(path, &config_edit::text_from(path, read));

        let (entries, note) = theme_entries(path, config_edit::parse("themes = 3\n"), &fallback);
        assert_eq!(note.as_deref(), Some("themes in /c/config.toml is not a table"));
        assert_eq!(customs(&entries), ["mine"]);
        assert_eq!(entries.len(), BUILTIN_THEMES.len() + 1, "the built-ins are listed too");

        let denied = std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        let (entries, note) = theme_entries(path, document(Err(denied)), &BTreeMap::new());
        assert!(note.as_deref().unwrap().starts_with("reading /c/config.toml: "), "{note:?}");
        assert!(customs(&entries).is_empty());

        let (entries, note) = theme_entries(path, document(Ok("[oops\n".into())), &fallback);
        assert!(note.as_deref().unwrap().starts_with("/c/config.toml isn't valid TOML: "), "{note:?}");
        assert_eq!(customs(&entries), ["mine"]);

        // A file that doesn't exist, or that holds its own themes, has no note.
        let missing = std::io::Error::from(std::io::ErrorKind::NotFound);
        let (entries, note) = theme_entries(path, document(Err(missing)), &fallback);
        assert_eq!((note, customs(&entries)), (None, Vec::<String>::new()));
        let (entries, note) =
            theme_entries(path, config_edit::parse("[themes.own]\nbackground = \"#000000\"\n"), &fallback);
        assert_eq!((note, customs(&entries)), (None, vec!["own".to_string()]));
    }

    /// Each operation that writes returns its edit, and the file is left to
    /// the view.
    #[test]
    fn operations_return_their_edits() {
        let mut themes = state(CONFIG, "mine");
        themes.move_selection(true);
        assert_eq!(themes.confirm(), Reply::Done(written("spare")));
        // The editor saves a copy, renamed or not.
        themes.start_edit();
        themes.set_field(FieldId::Name, "ours");
        let Reply::Done(Step::Write(ConfigEdit::SaveTheme { original, name, theme })) = themes.confirm() else {
            panic!("a save")
        };
        assert_eq!((original.as_deref(), name.as_str()), (Some("spare"), "ours"));
        assert_eq!(theme.background, Color::hex(0x111111));
        assert!(matches!(themes.mode, Mode::Editor(_)), "the editor stays open until the write is done");
        themes.cancel();
        // The delete question deletes the selected theme.
        let text = Ok(Some(CONFIG.to_string()));
        assert_eq!(themes.request_delete(&text), Reply::Done(Step::Nothing));
        assert_eq!(themes.confirm(), Reply::Done(Step::Write(ConfigEdit::DeleteTheme { name: "spare".into() })));
        assert_eq!(themes.list_mode(), ListMode::List);
    }

    #[test]
    fn refusals_say_why() {
        let mut themes = state(CONFIG, "mine");
        let text = Ok(Some(CONFIG.to_string()));
        // In use, built in, or the file can't be read.
        assert!(matches!(themes.request_delete(&text), Reply::Refused(e) if e.contains("the global theme")));
        themes.select(0);
        assert!(matches!(themes.request_delete(&text), Reply::Refused(e) if e.contains("built in and can't be deleted")));
        themes.select(7);
        let unreadable = Err("reading /c/config.toml: denied".to_string());
        assert_eq!(themes.request_delete(&unreadable), Reply::Refused("reading /c/config.toml: denied".into()));
        assert!(matches!(themes.request_delete(&Ok(Some("[oops\n".into()))), Reply::Refused(e) if e.contains("not valid TOML")));
        // A file that isn't there refers to nothing.
        assert_eq!(themes.request_delete(&Ok(None)), Reply::Done(Step::Nothing));
        themes.cancel();
        // The editor refuses a taken name and invalid colors.
        themes.start_edit();
        themes.set_field(FieldId::Name, "mine");
        assert!(matches!(themes.confirm(), Reply::Refused(e) if e.contains("already exists")));
        themes.set_field(FieldId::Name, "spare");
        themes.set_field(FieldId::Color(1), "#12");
        assert_eq!(themes.confirm(), Reply::Refused("Fix the colors marked invalid first".into()));
    }

    /// The selection is kept by name, a built-in apart from a custom theme
    /// of the same name.
    #[test]
    fn the_selection_is_kept_by_name() {
        let shadow = "[themes.tokyo-night]\nbackground = \"#000000\"\n";
        let mut themes = state(shadow, "tokyo-night");
        // The active theme is the custom one, which shadows the built-in.
        assert_eq!(themes.selected_index(), Some(BUILTIN_THEMES.len()));
        let builtin = BUILTIN_THEMES.iter().position(|t| *t == "tokyo-night").unwrap();
        themes.select(builtin);
        assert_eq!(themes.selected_index(), Some(builtin));
        assert_eq!(themes.preview(), builtin_theme("tokyo-night"));
        assert_eq!(themes.active_row(), Some(BUILTIN_THEMES.len()));
    }

    /// After a save the saved theme is selected; after a delete the one in
    /// its place, or the active theme when that one can't be read.
    #[test]
    fn writes_reread_the_list() {
        let mut themes = state(CONFIG, "mine");
        themes.start_edit();
        themes.set_field(FieldId::Name, "ours");
        let Reply::Done(Step::Write(edit)) = themes.confirm() else { panic!("a save") };
        let renamed = CONFIG.replace("mine", "ours");
        themes.written(&edit, path(), &Ok(Some(renamed.clone())));
        assert_eq!((themes.list_mode(), selected(&themes), themes.active()), (ListMode::List, "ours", "ours"));

        // Deleting spare, the last row, selects ours before it.
        themes.select_edge(true);
        assert_eq!(selected(&themes), "spare");
        themes.request_delete(&Ok(Some(renamed.clone())));
        let Reply::Done(Step::Write(edit)) = themes.confirm() else { panic!("a delete") };
        let without = renamed.replace("[themes.spare]\nbackground = \"#111111\"\n", "");
        themes.written(&edit, path(), &Ok(Some(without.clone())));
        assert_eq!(selected(&themes), "ours");
        // Deleting a theme that leaves an unreadable one in its place goes
        // to the active theme.
        let mut themes = state(&format!("{without}[themes.zz]\nbackground = \"#222222\"\n"), "brindle-dark");
        themes.select_edge(true);
        themes.request_delete(&Ok(Some(without.clone())));
        let Reply::Done(Step::Write(edit)) = themes.confirm() else { panic!("a delete") };
        // broken is now last, where zz was.
        themes.written(&edit, path(), &Ok(Some(without.replace("[themes.broken]", "[themes.zbroken]"))));
        assert_eq!(selected(&themes), "brindle-dark");
    }

    #[test]
    fn field_ids_own_the_slot_conversion() {
        let fields: Vec<FieldId> = FieldId::all().collect();
        assert_eq!(fields.len(), 1 + SLOTS.len());
        assert_eq!(fields[0], FieldId::Name);
        assert_eq!(fields[1], FieldId::Color(0));
        assert_eq!(FieldId::Color(1).slot(), Some(Slot::Background));
        assert_eq!(FieldId::Name.slot(), None);
        assert_eq!(FieldId::Color(SLOTS.len()).slot(), None);
        assert_eq!(FieldId::selection_foreground(), FieldId::Color(5));
        assert_eq!(field_label(FieldId::Name), Some("name"));
        assert_eq!(field_label(FieldId::Color(2)), Some("Cursor"));
        assert_eq!(field_label(FieldId::Color(SLOTS.len())), None);
        // A field past the last slot changes nothing.
        let mut themes = state(CONFIG, "mine");
        themes.start_edit();
        let before = themes.editor_fields();
        themes.set_field(FieldId::Color(SLOTS.len()), "#123456");
        assert_eq!(themes.editor_fields(), before);
    }

    #[test]
    fn the_editors_title_names_the_saved_theme() {
        let mut themes = state(CONFIG, "mine");
        themes.start_edit();
        assert_eq!(themes.editor().unwrap().title(), "Edit \"mine\"");
        themes.cancel();
        themes.select(0);
        themes.start_new();
        assert_eq!(themes.editor().unwrap().title(), "New theme");
    }

    #[test]
    fn buttons_follow_the_selection_and_the_editor() {
        let mut themes = state(CONFIG, "mine");
        assert!(themes.enabled(Btn::Delete), "mine is custom");
        themes.select(0);
        assert!(!themes.enabled(Btn::Delete), "built-ins can't be deleted");
        assert!(!themes.enabled(Btn::Clear) && !themes.enabled(Btn::Save));
        themes.start_new();
        assert!(themes.enabled(Btn::Save));
        themes.set_field(FieldId::selection_foreground(), "#abcdef");
        assert!(themes.enabled(Btn::Clear));
        themes.clear_selection_foreground();
        assert!(!themes.enabled(Btn::Clear));
        themes.set_field(FieldId::Color(0), "nope");
        assert!(!themes.enabled(Btn::Save));
        for b in [Btn::New, Btn::Cancel, Btn::Apply, Btn::Discard, Btn::Keep, Btn::ConfirmDelete] {
            assert!(themes.enabled(b), "{b:?}");
        }
    }

    #[test]
    fn every_theme_is_tagged_built_in_or_custom() {
        let themes = state(CONFIG, "mine");
        for entry in themes.entries() {
            assert_eq!(entry.tag(), if entry.custom { "custom" } else { "built-in" }, "{}", entry.name);
        }
        let tags: Vec<&str> = themes.entries().iter().map(ThemeEntry::tag).collect();
        assert_eq!(tags.iter().filter(|t| **t == "built-in").count(), BUILTIN_THEMES.len());
        assert_eq!(&tags[BUILTIN_THEMES.len()..], ["custom"; 3]);
        let mut themes = themes;
        themes.select(0);
        assert!(themes.describe(&None, "").ends_with("origin=built-in\n"));
        themes.select(BUILTIN_THEMES.len() + 1); // mine
        assert!(themes.describe(&None, "").ends_with("origin=custom\n"));
    }

    #[test]
    fn edit_is_unavailable_on_a_built_in() {
        let mut themes = state(CONFIG, "mine");
        themes.select(0);
        assert!(!themes.enabled(Btn::Edit));
        themes.select(BUILTIN_THEMES.len() + 1);
        assert_eq!(themes.selected_entry().unwrap().name, "mine");
        assert!(themes.enabled(Btn::Edit), "mine is a custom theme");
        // A custom theme that can't be read can't be selected, so Edit
        // stays where it was.
        let broken = themes.entries().iter().position(|e| e.theme.is_err()).unwrap();
        assert_eq!(themes.select(broken), Reply::Quiet);
        assert!(themes.enabled(Btn::Edit));
    }

    #[test]
    fn the_edit_key_on_a_built_in_says_new_copies() {
        let mut themes = state(CONFIG, "mine");
        themes.select(0);
        let name = themes.selected_entry().unwrap().name.clone();
        assert_eq!(
            themes.start_edit(),
            Reply::Refused(format!("{name:?} is built in: New makes an editable copy"))
        );
        assert_eq!(themes.list_mode(), ListMode::List);
        assert!(themes.editor().is_none());
    }

    #[test]
    fn the_delete_key_on_a_built_in_says_why() {
        let mut themes = state(CONFIG, "mine");
        themes.select(0);
        let name = themes.selected_entry().unwrap().name.clone();
        let text = Ok(Some(CONFIG.to_string()));
        assert_eq!(themes.request_delete(&text), Reply::Refused(format!("{name:?} is built in and can't be deleted")));
        assert_eq!(themes.list_mode(), ListMode::List);
    }
}
