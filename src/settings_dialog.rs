//! The settings dialog: an overlay over the window with a list of sections.
//! The only section so far is "Theme": pick a theme with a live preview on
//! the tabs behind the dialog, and create, edit, rename or delete custom
//! themes. Changes are written to the config file with `config_edit`.
//!
//! [`Model`] holds the state and all the logic and has no GPUI dependency,
//! so it is unit-tested; [`SettingsDialog`] is the view around it.

use std::collections::BTreeMap;
use std::path::PathBuf;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, ClickEvent, Context, CursorStyle, DismissEvent, Entity, EventEmitter,
    FocusHandle, Focusable, InteractiveElement, IntoElement, KeyDownEvent, Keystroke, MouseButton, ParentElement, Render,
    ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div, px,
};

use brindle_ui::{
    ActiveTheme as _, Button, ButtonSize, ButtonVariant, Dialog, Dismiss, FocusRing as _, FocusStops, Input, Select,
    TextInput, TextInputEvent, Tokens,
};

use crate::actions::{
    SettingsConfirm, SettingsDelete, SettingsDown, SettingsEdit, SettingsFirst, SettingsLast, SettingsNew, SettingsUp,
};
use crate::config_edit;
use crate::theme::{BUILTIN_THEMES, Color, Theme, builtin_theme};

mod profiles;

use crate::config::Profile;
use profiles::ProfilesState;

/// The dialog's sections, in the order they are listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    General,
    Profiles,
    Theme,
}

const SECTIONS: [Section; 3] = [Section::General, Section::Profiles, Section::Theme];

impl Section {
    fn label(self) -> &'static str {
        match self {
            Section::General => "General",
            Section::Profiles => "Profiles",
            Section::Theme => "Theme",
        }
    }
}

// ---- model ------------------------------------------------------------------

/// A theme in the list.
#[derive(Clone, Debug)]
pub struct ThemeEntry {
    pub name: String,
    pub custom: bool,
    /// A custom theme that can't be read carries its error.
    pub theme: Result<Theme, String>,
}

/// A color a theme editor field edits.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Slot {
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

const SLOTS: [Slot; 27] = [
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
    fn label(self) -> &'static str {
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
    fn optional(self) -> bool {
        self == Slot::SelectionForeground
    }

    fn get(self, theme: &Theme) -> Option<Color> {
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
            (_, None) => {}
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

/// The custom-theme editor.
#[derive(Clone, Debug)]
struct Editor {
    /// The saved custom theme being edited; `None` for a new one.
    original: Option<String>,
    /// List selection to go back to on discard.
    return_to: usize,
    name: String,
    /// One field per [`SLOTS`] entry.
    colors: Vec<String>,
    /// The theme as of the last valid edit of every field.
    current: Theme,
    /// The fields as the editor opened, to tell unsaved edits.
    initial: Vec<String>,
}

const FIELD_COUNT: usize = 1 + SLOTS.len();

impl Editor {
    fn new(original: Option<String>, name: String, theme: &Theme, return_to: usize) -> Self {
        let current = theme.with_effective_colors();
        let colors: Vec<String> = SLOTS
            .iter()
            .map(|slot| slot.get(&current).map(|c| c.to_string()).unwrap_or_default())
            .collect();
        let initial = std::iter::once(name.clone()).chain(colors.iter().cloned()).collect();
        Self { original, return_to, name, colors, current, initial }
    }

    /// Parses color field `ix`: `Ok(None)` is a cleared optional color.
    fn parse_color(&self, ix: usize) -> Result<Option<Color>, &'static str> {
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

    /// Sets field `field` (0 is the name) and updates the preview theme if
    /// it now holds a valid color.
    fn set_field(&mut self, field: usize, text: &str) {
        match field {
            0 => self.name = text.to_string(),
            n if n < FIELD_COUNT => {
                let ix = n - 1;
                self.colors[ix] = text.to_string();
                if let Ok(color) = self.parse_color(ix) {
                    SLOTS[ix].set(&mut self.current, color);
                }
            }
            _ => {}
        }
    }

    fn dirty(&self) -> bool {
        (0..FIELD_COUNT).any(|field| self.field(field) != self.initial[field])
    }

    fn field(&self, field: usize) -> &str {
        match field {
            0 => &self.name,
            n => &self.colors[n - 1],
        }
    }

}

/// A theme editor field's name in the `--dump-screen-after` line: `name`
/// for field 0, else its color's label.
fn field_label(field: usize) -> &'static str {
    match field {
        0 => "name",
        n => SLOTS.get(n - 1).map_or("", |slot| slot.label()),
    }
}

/// The selection foreground's field (its color slot plus one for the name).
fn selection_foreground_field() -> usize {
    SLOTS.iter().position(|s| *s == Slot::SelectionForeground).unwrap() + 1
}

/// What kind of view a section shows: its list (General counts as one), an
/// editor, or the delete confirmation. A change of section or kind moves
/// focus to the new view's first control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeKind {
    List,
    Editor,
    ConfirmDelete,
}

#[derive(Clone, Debug)]
enum Mode {
    List,
    Editor(Editor),
    /// Asking whether to delete the selected custom theme.
    ConfirmDelete,
}

/// What the view should tell the workspace after an operation.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Nothing,
    /// The config file changed; reload it but keep the dialog open.
    Written,
    /// The config file changed and the dialog is done.
    Applied,
    /// The dialog is done; nothing to write.
    Close,
}

pub struct Model {
    config_path: PathBuf,
    entries: Vec<ThemeEntry>,
    /// The global theme's name when the dialog opened.
    active: String,
    selected: usize,
    mode: Mode,
    /// The last failure to show (save errors, refused deletes).
    error: Option<String>,
    /// Custom themes to list when the file can't be parsed.
    fallback: BTreeMap<String, Theme>,
    section: Section,
    profiles: ProfilesState,
    /// Profiles to list (read-only) when the file can't be parsed.
    fallback_profiles: Vec<Profile>,
}

impl Model {
    pub fn new(
        config_path: PathBuf,
        active: String,
        fallback: BTreeMap<String, Theme>,
        fallback_profiles: Vec<Profile>,
        fallback_default: Option<String>,
    ) -> Self {
        let profiles = ProfilesState::new(&config_path, &fallback_profiles, fallback_default);
        let mut model = Self {
            config_path,
            entries: Vec::new(),
            active,
            selected: 0,
            mode: Mode::List,
            error: None,
            fallback,
            section: Section::General,
            profiles,
            fallback_profiles,
        };
        model.reload_entries();
        let active = model.active.clone();
        model.selected = model.selectable_index(&active).or_else(|| model.selectable_index("brindle-dark")).unwrap_or(0);
        model
    }

    /// Rereads the custom themes from the config file.
    fn reload_entries(&mut self) {
        let customs = match std::fs::read_to_string(&self.config_path).map_err(|e| e.to_string()).and_then(|t| config_edit::parse(&t)) {
            Ok(doc) => config_edit::custom_themes(&doc),
            Err(_) => self.fallback.iter().map(|(n, t)| (n.clone(), Ok(t.clone()))).collect(),
        };
        self.entries = BUILTIN_THEMES
            .iter()
            .map(|name| ThemeEntry { name: name.to_string(), custom: false, theme: Ok(builtin_theme(name).unwrap()) })
            .chain(customs.into_iter().map(|(name, theme)| ThemeEntry { name, custom: true, theme }))
            .collect();
    }

    /// The index of the readable entry `name` (custom themes shadow
    /// built-ins of the same name, as in the config).
    fn selectable_index(&self, name: &str) -> Option<usize> {
        let matches = |e: &ThemeEntry| e.name == name && e.theme.is_ok();
        self.entries.iter().rposition(matches)
    }

    fn custom_names(&self) -> Vec<&str> {
        self.entries.iter().filter(|e| e.custom).map(|e| e.name.as_str()).collect()
    }

    pub fn selected_entry(&self) -> Option<&ThemeEntry> {
        self.entries.get(self.selected)
    }

    /// The theme the tabs should show right now.
    pub fn preview(&self) -> Option<Theme> {
        match &self.mode {
            Mode::Editor(editor) => Some(editor.current.clone()),
            _ => self.selected_entry()?.theme.as_ref().ok().cloned(),
        }
    }

    pub fn move_selection(&mut self, forward: bool) {
        if self.section == Section::General {
            return;
        }
        if self.section == Section::Profiles {
            self.profiles.move_selection(forward);
            self.error = None;
            return;
        }
        match &mut self.mode {
            Mode::List => {
                let count = self.entries.len();
                let mut ix = self.selected;
                for _ in 0..count {
                    ix = if forward { (ix + 1) % count } else { (ix + count - 1) % count };
                    if self.entries[ix].theme.is_ok() {
                        self.selected = ix;
                        self.error = None;
                        return;
                    }
                }
            }
            // The editor's fields are focus stops: the view moves focus.
            Mode::Editor(_) | Mode::ConfirmDelete => {}
        }
    }

    /// Home and end in a list: selects its first entry, or its `last`
    /// (themes skip the unreadable ones, as up and down do).
    pub fn select_edge(&mut self, last: bool) {
        match self.section {
            Section::General => {}
            Section::Profiles => {
                if let Some(ix) = if last { self.profiles.entries.len().checked_sub(1) } else { Some(0) } {
                    self.profiles.select(ix);
                    self.error = None;
                }
            }
            Section::Theme => {
                let readable = |ix: &usize| self.entries[*ix].theme.is_ok();
                let mut ixs = 0..self.entries.len();
                let found = if last { ixs.rfind(readable) } else { ixs.find(readable) };
                if let Some(ix) = found {
                    self.select(ix);
                }
            }
        }
    }

    pub fn select(&mut self, ix: usize) {
        if self.section == Section::General {
            return;
        }
        if self.section == Section::Profiles {
            self.profiles.select(ix);
            self.error = None;
            return;
        }
        if matches!(self.mode, Mode::List) && self.entries.get(ix).is_some_and(|e| e.theme.is_ok()) {
            self.selected = ix;
            self.error = None;
        }
    }

    fn save(&mut self, edit: impl FnOnce(&mut toml_edit::DocumentMut) -> Result<(), String>) -> Result<(), ()> {
        match config_edit::save(&self.config_path, edit) {
            Ok(()) => {
                self.error = None;
                Ok(())
            }
            Err(e) => {
                self.error = Some(e);
                Err(())
            }
        }
    }

    /// Enter: apply in the list, save in the editor, delete when confirming.
    pub fn confirm(&mut self) -> Outcome {
        if self.section == Section::General {
            return Outcome::Close;
        }
        if self.section == Section::Profiles {
            let result = match &self.profiles.mode {
                profiles::Mode::List => {
                    let themes = self.theme_names();
                    self.profiles.start_edit(&themes).map(|()| profiles::Done::Nothing)
                }
                profiles::Mode::Editor(_) => self.profiles.save(&self.config_path, &self.fallback_profiles),
                profiles::Mode::ConfirmDelete => self.profiles.delete_selected(&self.config_path, &self.fallback_profiles),
            };
            return self.profiles_done(result);
        }
        match &self.mode {
            Mode::List => self.apply(),
            Mode::Editor(_) => self.save_editor(),
            Mode::ConfirmDelete => self.delete_selected(),
        }
    }

    /// Escape: `true` when the whole dialog should close (cancel).
    pub fn cancel(&mut self) -> bool {
        if self.section == Section::General {
            return true;
        }
        if self.section == Section::Profiles {
            self.error = None;
            return match self.profiles.mode {
                profiles::Mode::List => true,
                profiles::Mode::Editor(_) => {
                    self.profiles.discard();
                    false
                }
                profiles::Mode::ConfirmDelete => {
                    self.profiles.mode = profiles::Mode::List;
                    false
                }
            };
        }
        match std::mem::replace(&mut self.mode, Mode::List) {
            Mode::List => true,
            Mode::Editor(editor) => {
                self.selected = editor.return_to;
                self.error = None;
                false
            }
            Mode::ConfirmDelete => false,
        }
    }

    fn apply(&mut self) -> Outcome {
        let Some(entry) = self.selected_entry().filter(|e| e.theme.is_ok()) else { return Outcome::Nothing };
        let name = entry.name.clone();
        match self.save(|doc| {
            config_edit::set_global_theme(doc, &name);
            Ok(())
        }) {
            Ok(()) => Outcome::Applied,
            Err(()) => Outcome::Nothing,
        }
    }

    /// New: a copy of the selected theme under an unused name.
    pub fn start_new(&mut self) {
        if self.section == Section::General {
            return;
        }
        if self.section == Section::Profiles {
            if matches!(self.profiles.mode, profiles::Mode::List) {
                let themes = self.theme_names();
                let result = self.profiles.start_new(&themes).map(|()| profiles::Done::Nothing);
                self.profiles_done(result);
            }
            return;
        }
        if !matches!(self.mode, Mode::List) {
            return;
        }
        let Some(ThemeEntry { name, theme: Ok(theme), .. }) = self.selected_entry().cloned() else { return };
        let name = config_edit::unused_name(&name, self.custom_names());
        self.mode = Mode::Editor(Editor::new(None, name, &theme, self.selected));
        self.error = None;
    }

    /// Edit: a custom theme in place; a built-in is copied first.
    pub fn start_edit(&mut self) {
        if self.section == Section::General {
            return;
        }
        if self.section == Section::Profiles {
            if matches!(self.profiles.mode, profiles::Mode::List) {
                let themes = self.theme_names();
                let result = self.profiles.start_edit(&themes).map(|()| profiles::Done::Nothing);
                self.profiles_done(result);
            }
            return;
        }
        if !matches!(self.mode, Mode::List) {
            return;
        }
        match self.selected_entry().cloned() {
            Some(ThemeEntry { name, custom: true, theme: Ok(theme) }) => {
                self.mode = Mode::Editor(Editor::new(Some(name.clone()), name, &theme, self.selected));
                self.error = None;
            }
            Some(ThemeEntry { custom: false, .. }) => self.start_new(),
            _ => {}
        }
    }

    pub fn editor_can_save(&self) -> bool {
        let Mode::Editor(editor) = &self.mode else { return false };
        editor.invalid_colors().is_empty() && editor.name_result(&self.other_names(editor)).is_ok()
    }

    fn other_names(&self, editor: &Editor) -> Vec<&str> {
        self.custom_names().into_iter().filter(|n| Some(*n) != editor.original.as_deref()).collect()
    }

    fn save_editor(&mut self) -> Outcome {
        let Mode::Editor(editor) = &self.mode else { return Outcome::Nothing };
        let name = match editor.name_result(&self.other_names(editor)) {
            Ok(name) => name,
            Err(e) => {
                self.error = Some(e);
                return Outcome::Nothing;
            }
        };
        if !editor.invalid_colors().is_empty() {
            self.error = Some("Fix the colors marked invalid first".into());
            return Outcome::Nothing;
        }
        let theme = editor.current.clone();
        let original = editor.original.clone();
        let saved = self.save(|doc| {
            if let Some(old) = original.as_deref().filter(|old| *old != name) {
                config_edit::rename_theme(doc, old, &name)?;
            }
            config_edit::upsert_theme(doc, &name, &theme);
            Ok(())
        });
        if saved.is_err() {
            return Outcome::Nothing;
        }
        if original.as_deref() == Some(self.active.as_str()) {
            // A rename carries the global theme along.
            self.active = name.clone();
        }
        self.mode = Mode::List;
        self.reload_entries();
        self.selected = self.selectable_index(&name).unwrap_or(0);
        Outcome::Written
    }

    /// Delete: asks for confirmation, or explains why the theme can't go.
    pub fn request_delete(&mut self) {
        if self.section == Section::General {
            return;
        }
        if self.section == Section::Profiles {
            if matches!(self.profiles.mode, profiles::Mode::List) {
                let result = self.profiles.request_delete().map(|()| profiles::Done::Nothing);
                self.profiles_done(result);
            }
            return;
        }
        if !matches!(self.mode, Mode::List) {
            return;
        }
        let Some(entry) = self.selected_entry() else { return };
        if !entry.custom {
            self.error = Some(format!("{:?} is a built-in theme and can't be deleted", entry.name));
            return;
        }
        let name = entry.name.clone();
        let refs = std::fs::read_to_string(&self.config_path)
            .map_err(|e| e.to_string())
            .and_then(|t| config_edit::parse(&t))
            .map(|mut doc| config_edit::theme_references(&mut doc, &name));
        match refs {
            Ok(refs) if refs.is_empty() => {
                self.mode = Mode::ConfirmDelete;
                self.error = None;
            }
            Ok(refs) => self.error = Some(format!("{name:?} is in use by {}", refs.join(", "))),
            Err(e) => self.error = Some(e),
        }
    }

    fn delete_selected(&mut self) -> Outcome {
        let Some(name) = self.selected_entry().map(|e| e.name.clone()) else { return Outcome::Nothing };
        self.mode = Mode::List;
        if self.save(|doc| config_edit::delete_theme(doc, &name)).is_err() {
            return Outcome::Nothing;
        }
        let previous = self.selected;
        self.reload_entries();
        // Stay near where the theme was.
        self.selected = previous.min(self.entries.len().saturating_sub(1));
        if self.entries[self.selected].theme.is_err() {
            let active = self.active.clone();
            self.selected = self.selectable_index(&active).unwrap_or(0);
        }
        Outcome::Written
    }

    /// List shortcuts. In the editor, keys belong to the text inputs.
    pub fn key(&mut self, ks: &Keystroke) -> bool {
        if self.section == Section::General {
            return false;
        }
        if self.section == Section::Profiles {
            let plain = !(ks.modifiers.control || ks.modifiers.alt || ks.modifiers.platform);
            if !plain || !matches!(self.profiles.mode, profiles::Mode::List) {
                return false;
            }
            match ks.key.as_str() {
                "n" => self.start_new(),
                "e" => self.start_edit(),
                "c" => self.start_duplicate(),
                "delete" | "d" => self.request_delete(),
                _ => return false,
            }
            return true;
        }
        match &mut self.mode {
            Mode::Editor(_) => false,
            Mode::List if ks.modifiers.control || ks.modifiers.alt || ks.modifiers.platform => false,
            Mode::List => {
                match ks.key.as_str() {
                    "n" => self.start_new(),
                    "e" => self.start_edit(),
                    "delete" | "d" => self.request_delete(),
                    _ => return false,
                }
                true
            }
            Mode::ConfirmDelete => false,
        }
    }

    /// A field's text changed in its input.
    pub fn set_field(&mut self, field: usize, text: &str) {
        if let Mode::Editor(editor) = &mut self.mode {
            editor.set_field(field, text);
            self.error = None;
        }
    }

    /// The editor's field texts (name first), when editing.
    pub fn editor_fields(&self) -> Option<Vec<String>> {
        let Mode::Editor(editor) = &self.mode else { return None };
        Some((0..FIELD_COUNT).map(|field| editor.field(field).to_string()).collect())
    }

    /// Clears the selection foreground; returns its field.
    pub fn clear_selection_foreground(&mut self) -> Option<usize> {
        let Mode::Editor(editor) = &mut self.mode else { return None };
        let field = selection_foreground_field();
        editor.set_field(field, "");
        Some(field)
    }

    /// Whether the editor's selection foreground holds a color, so Clear
    /// has something to clear.
    fn selection_foreground_set(&self) -> bool {
        match &self.mode {
            Mode::Editor(editor) => Slot::SelectionForeground.get(&editor.current).is_some(),
            _ => false,
        }
    }

    #[cfg(test)]
    pub fn in_list(&self) -> bool {
        self.mode_kind() == ModeKind::List
    }

    /// What the current section shows.
    pub fn mode_kind(&self) -> ModeKind {
        match self.section {
            Section::General => ModeKind::List,
            Section::Theme => match self.mode {
                Mode::List => ModeKind::List,
                Mode::Editor(_) => ModeKind::Editor,
                Mode::ConfirmDelete => ModeKind::ConfirmDelete,
            },
            Section::Profiles => match self.profiles.mode {
                profiles::Mode::List => ModeKind::List,
                profiles::Mode::Editor(_) => ModeKind::Editor,
                profiles::Mode::ConfirmDelete => ModeKind::ConfirmDelete,
            },
        }
    }

    pub fn section(&self) -> Section {
        self.section
    }

    /// Switches section, refused while an editor has unsaved edits. Leaving
    /// Theme drops an unapplied preview: its selection goes back to the
    /// active theme.
    pub fn select_section(&mut self, section: Section) -> bool {
        if section == self.section {
            return true;
        }
        let dirty = match self.section {
            Section::General => false,
            Section::Theme => matches!(&self.mode, Mode::Editor(e) if e.dirty()),
            Section::Profiles => self.profiles.editor().is_some_and(|e| e.dirty()),
        };
        if dirty {
            self.error = Some("Save or discard the edits first".into());
            return false;
        }
        match self.section {
            Section::General => {}
            Section::Theme => {
                self.mode = Mode::List;
                let active = self.active.clone();
                self.selected = self.selectable_index(&active).or_else(|| self.selectable_index("brindle-dark")).unwrap_or(0);
            }
            Section::Profiles => {
                if !matches!(self.profiles.mode, profiles::Mode::List) {
                    self.profiles.discard();
                }
            }
        }
        if section == Section::Profiles {
            // Pick up edits made elsewhere since the dialog opened.
            self.profiles.reload(&self.config_path, &self.fallback_profiles, None);
        }
        self.section = section;
        self.error = None;
        true
    }

    /// Cycles through the sections (ctrl-tab).
    /// Home and end in the section list: the first or the `last` section.
    pub fn edge_section(&mut self, last: bool) -> bool {
        self.select_section(if last { SECTIONS[SECTIONS.len() - 1] } else { SECTIONS[0] })
    }

    pub fn next_section(&mut self, forward: bool) -> bool {
        let ix = SECTIONS.iter().position(|s| *s == self.section).unwrap_or(0);
        let n = SECTIONS.len();
        self.select_section(SECTIONS[if forward { (ix + 1) % n } else { (ix + n - 1) % n }])
    }

    /// Every theme a profile can choose: built-ins, then custom themes.
    fn theme_names(&self) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for entry in self.entries.iter().filter(|e| e.theme.is_ok()) {
            if !names.contains(&entry.name) {
                names.push(entry.name.clone());
            }
        }
        names
    }

    fn profiles_done(&mut self, result: Result<profiles::Done, String>) -> Outcome {
        match result {
            Ok(profiles::Done::Written) => {
                self.error = None;
                Outcome::Written
            }
            Ok(profiles::Done::Nothing) => {
                self.error = None;
                Outcome::Nothing
            }
            Err(e) => {
                self.error = Some(e);
                Outcome::Nothing
            }
        }
    }

    /// Duplicate (profiles only).
    pub fn start_duplicate(&mut self) {
        if self.section == Section::Profiles && matches!(self.profiles.mode, profiles::Mode::List) {
            let themes = self.theme_names();
            let result = self.profiles.start_duplicate(&themes).map(|()| profiles::Done::Nothing);
            self.profiles_done(result);
        }
    }

    /// alt-up / alt-down in the profile list.
    pub fn move_profile(&mut self, up: bool) -> Outcome {
        if self.section != Section::Profiles {
            return Outcome::Nothing;
        }
        let result = self.profiles.move_selected(up, &self.config_path, &self.fallback_profiles);
        self.profiles_done(result)
    }

    /// The edited profile's name and the settings its tabs should preview.
    pub fn profile_preview(&self) -> Option<(String, Profile)> {
        if self.section != Section::Profiles {
            return None;
        }
        self.profiles.preview()
    }

    /// Profile renames saved since the last call, for the reload.
    pub fn take_renames(&mut self) -> Vec<(String, String)> {
        self.profiles.take_renames()
    }

    /// Debug description for `--dump-screen-after`. `field` names the
    /// focused control, which an editor reports as its `field=`.
    pub fn describe(&self, field: &str) -> String {
        if self.section == Section::General {
            return format!("--- settings section=General error={:?}\n", self.error);
        }
        if self.section == Section::Profiles {
            return self.profiles.describe(&self.error, field);
        }
        let section = "Theme";
        match &self.mode {
            Mode::List => format!(
                "--- settings section={section} mode=list selected={:?} active={:?} error={:?}\n",
                self.selected_entry().map(|e| e.name.as_str()).unwrap_or(""),
                self.active,
                self.error
            ),
            Mode::Editor(editor) => format!(
                "--- settings section={section} mode=editor name={:?} field={:?} invalid={:?} can_save={} error={:?}\n",
                editor.name,
                field,
                editor.invalid_colors().iter().map(|&ix| SLOTS[ix].label()).collect::<Vec<_>>(),
                self.editor_can_save(),
                self.error
            ),
            Mode::ConfirmDelete => format!(
                "--- settings section={section} mode=confirm-delete theme={:?}\n",
                self.selected_entry().map(|e| e.name.as_str()).unwrap_or("")
            ),
        }
    }
}

// ---- view -------------------------------------------------------------------

pub enum SettingsEvent {
    /// Show this theme on the tabs that follow the global theme.
    Preview(Theme),
    /// Show this profile's theme and font size on its tabs (by its saved
    /// name), or stop.
    ProfilePreview(Option<(String, Profile)>),
    /// The config file was written; reload it, with these profile renames
    /// (old, new) applied to open tabs.
    ConfigWritten(Vec<(String, String)>),
}

/// The dialog's buttons, each with its own focus handle. A button the
/// theme and profile views share (New, Save, …) shares its handle too, since
/// only one of them shows at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Btn {
    New,
    Duplicate,
    Edit,
    Delete,
    Up,
    Down,
    Cancel,
    Apply,
    Close,
    Keep,
    ConfirmDelete,
    Clear,
    Discard,
    Save,
}

const BUTTONS: [Btn; 14] = [
    Btn::New,
    Btn::Duplicate,
    Btn::Edit,
    Btn::Delete,
    Btn::Up,
    Btn::Down,
    Btn::Cancel,
    Btn::Apply,
    Btn::Close,
    Btn::Keep,
    Btn::ConfirmDelete,
    Btn::Clear,
    Btn::Discard,
    Btn::Save,
];

impl Btn {
    /// The element id, unique among the buttons of one view.
    fn id(self) -> &'static str {
        match self {
            Btn::New => "new",
            Btn::Duplicate => "duplicate",
            Btn::Edit => "edit",
            Btn::Delete => "delete",
            Btn::Up => "move-up",
            Btn::Down => "move-down",
            Btn::Cancel => "cancel",
            Btn::Apply => "apply",
            Btn::Close => "close",
            Btn::Keep => "delete-no",
            Btn::ConfirmDelete => "delete-yes",
            Btn::Clear => "clear-selection-fg",
            Btn::Discard => "discard",
            Btn::Save => "save",
        }
    }

    /// What a focused button reports as the editor's `field=` in the dump.
    fn label(self) -> &'static str {
        match self {
            Btn::New => "New",
            Btn::Duplicate => "Duplicate",
            Btn::Edit => "Edit",
            Btn::Delete => "Delete",
            Btn::Up => "Move up",
            Btn::Down => "Move down",
            Btn::Cancel => "Cancel",
            Btn::Apply => "Apply",
            Btn::Close => "Close",
            Btn::Keep => "Keep",
            Btn::ConfirmDelete => "Delete",
            Btn::Clear => "Clear",
            Btn::Discard => "Discard",
            Btn::Save => "Save",
        }
    }
}

/// The footer buttons of each view, in Tab order (left to right).
fn footer_buttons(section: Section, kind: ModeKind) -> &'static [Btn] {
    match (section, kind) {
        (Section::General, _) => &[Btn::Close],
        (_, ModeKind::ConfirmDelete) => &[Btn::Keep, Btn::ConfirmDelete],
        (_, ModeKind::Editor) => &[Btn::Discard, Btn::Save],
        (Section::Theme, ModeKind::List) => &[Btn::New, Btn::Edit, Btn::Delete, Btn::Cancel, Btn::Apply],
        (Section::Profiles, ModeKind::List) => {
            &[Btn::New, Btn::Duplicate, Btn::Edit, Btn::Delete, Btn::Up, Btn::Down, Btn::Close]
        }
    }
}

/// What enter does when the focused control is `on_button` (a button or
/// the dialog's close `X`) or not: a button takes enter itself, on its
/// key-down, so the dialog's binding lets the key through to it instead of
/// running the section's confirm. gpui matches the binding first, so
/// without this a focused Discard would save.
fn enter_confirms(on_button: bool) -> bool {
    !on_button
}

/// A choice field: one of the profile editor's selects.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Choice {
    Profile(profiles::Field),
}

pub struct SettingsDialog {
    focus_handle: FocusHandle,
    /// The close `X` in the corner.
    close_handle: FocusHandle,
    /// The theme list or the profile list, whichever is shown: one Tab
    /// stop, whose keys move the selection.
    list_handle: FocusHandle,
    /// The section list: the first Tab stop, whose keys switch sections.
    sections_handle: FocusHandle,
    buttons: Vec<(Btn, FocusHandle)>,
    model: Model,
    /// The theme the tabs last previewed, to tell when it changes.
    theme: Theme,
    list_scroll: ScrollHandle,
    editor_scroll: ScrollHandle,
    /// The editor's inputs, one per field (name first); empty in the list.
    fields: Vec<Entity<TextInput>>,
    _field_subscriptions: Vec<Subscription>,
    /// What the last profile preview showed: name, theme and font size.
    profile_previewed: Option<(String, Option<String>, Option<f32>)>,
    /// The profile editor's text inputs and selects; empty unless it is
    /// open.
    profile_inputs: Vec<(profiles::Field, Entity<TextInput>)>,
    profile_selects: Vec<(profiles::Field, Entity<Select>)>,
    _profile_subscriptions: Vec<Subscription>,
    /// The profile editor's default checkbox.
    default_handle: FocusHandle,
    profile_list_scroll: ScrollHandle,
    profile_editor_scroll: ScrollHandle,
    /// The section and kind of view focus was last placed for.
    placed_for: (Section, ModeKind),
    /// The focused handle an editor last scrolled into view.
    scrolled_to: Option<FocusHandle>,
}

impl EventEmitter<SettingsEvent> for SettingsDialog {}
impl EventEmitter<DismissEvent> for SettingsDialog {}

impl Focusable for SettingsDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl FocusStops for SettingsDialog {
    /// The section list (one stop), then the view's fields (or its list,
    /// one stop), then its footer buttons, then the close `X`.
    fn stops(&self, cx: &App) -> Vec<FocusHandle> {
        let mut stops = vec![self.sections_handle.clone()];
        let (section, kind) = (self.model.section(), self.model.mode_kind());
        match (section, kind) {
            (Section::General, _) => {}
            (Section::Theme | Section::Profiles, ModeKind::List) => stops.push(self.list_handle.clone()),
            (Section::Theme, ModeKind::Editor) => {
                for (field, input) in self.fields.iter().enumerate() {
                    stops.push(input.focus_handle(cx));
                    if field == selection_foreground_field() && self.enabled(Btn::Clear) {
                        stops.push(self.button_handle(Btn::Clear));
                    }
                }
            }
            (Section::Profiles, ModeKind::Editor) => stops.extend(self.profile_field_stops(cx)),
            _ => {}
        }
        stops.extend(
            footer_buttons(section, kind).iter().filter(|b| self.enabled(**b)).map(|b| self.button_handle(*b)),
        );
        stops.push(self.close_handle.clone());
        stops
    }
}

impl SettingsDialog {
    pub fn new(model: Model, theme: Theme, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let theme = model.preview().unwrap_or(theme);
        let list_scroll = ScrollHandle::new();
        list_scroll.scroll_to_item(model.selected);
        let placed_for = (model.section(), model.mode_kind());
        Self {
            focus_handle: cx.focus_handle(),
            close_handle: cx.focus_handle(),
            list_handle: cx.focus_handle(),
            sections_handle: cx.focus_handle(),
            buttons: BUTTONS.iter().map(|&b| (b, cx.focus_handle())).collect(),
            model,
            theme,
            list_scroll,
            editor_scroll: ScrollHandle::new(),
            fields: Vec::new(),
            _field_subscriptions: Vec::new(),
            profile_previewed: None,
            profile_inputs: Vec::new(),
            profile_selects: Vec::new(),
            _profile_subscriptions: Vec::new(),
            default_handle: cx.focus_handle(),
            profile_list_scroll: ScrollHandle::new(),
            profile_editor_scroll: ScrollHandle::new(),
            placed_for,
            scrolled_to: None,
        }
    }

    /// The dialog's `--dump-screen-after` line, with the focused control
    /// as an editor's `field=`.
    pub fn describe(&self, window: &Window, cx: &App) -> String {
        self.model.describe(self.focused_label(window, cx))
    }

    /// The editor input that has keyboard focus.
    pub fn focused_input(&self, window: &Window, cx: &App) -> Option<Entity<TextInput>> {
        let mut inputs = self.fields.iter().chain(self.profile_inputs.iter().map(|(_, i)| i));
        inputs.find(|f| f.read(cx).is_focused(window)).cloned()
    }

    /// The name of the focused control: an editor field's label, a
    /// button's, `list` for a list, `close` for the `X`, or empty.
    pub fn focused_label(&self, window: &Window, cx: &App) -> &'static str {
        if self.sections_handle.is_focused(window) {
            return "sections";
        }
        if self.list_handle.is_focused(window) {
            return "list";
        }
        if let Some(field) = self.fields.iter().position(|f| f.read(cx).is_focused(window)) {
            return field_label(field);
        }
        if let Some((field, _)) = self.profile_inputs.iter().find(|(_, i)| i.read(cx).is_focused(window)) {
            return field.label();
        }
        // A select whose menu is open still counts as the focused field.
        let select_focused = |s: &Entity<Select>| {
            let s = s.read(cx);
            s.is_open() || s.focus_handle(cx).is_focused(window)
        };
        if let Some((field, _)) = self.profile_selects.iter().find(|(_, s)| select_focused(s)) {
            return field.label();
        }
        if self.default_handle.is_focused(window) {
            return profiles::Field::Default.label();
        }
        if let Some((b, _)) = self.buttons.iter().find(|(_, h)| h.is_focused(window)) {
            return b.label();
        }
        if self.close_handle.is_focused(window) {
            return "close";
        }
        ""
    }

    fn button_handle(&self, b: Btn) -> FocusHandle {
        self.buttons.iter().find(|(btn, _)| *btn == b).map(|(_, h)| h.clone()).expect("every button has a handle")
    }

    /// Whether a button or the close `X` has focus.
    fn button_focused(&self, window: &Window) -> bool {
        self.close_handle.is_focused(window) || self.buttons.iter().any(|(_, h)| h.is_focused(window))
    }

    /// Whether button `b` can be pressed (and focused) in the current view.
    fn enabled(&self, b: Btn) -> bool {
        let model = &self.model;
        let profiles = &model.profiles;
        let editable = profiles.read_only.is_none();
        let profile_ok = profiles.selected_entry().is_some_and(|e| e.profile.is_ok());
        match (model.section(), b) {
            (Section::Theme, Btn::Delete) => model.selected_entry().is_some_and(|e| e.custom),
            (Section::Theme, Btn::Clear) => model.selection_foreground_set(),
            (Section::Theme, Btn::Save) => model.editor_can_save(),
            (Section::Profiles, Btn::New) => editable,
            (Section::Profiles, Btn::Duplicate | Btn::Edit) => editable && profile_ok,
            (Section::Profiles, Btn::Delete) => editable && profiles.entries.len() > 1,
            (Section::Profiles, Btn::Up) => editable && profiles.selected > 0,
            (Section::Profiles, Btn::Down) => editable && profiles.selected + 1 < profiles.entries.len(),
            (Section::Profiles, Btn::Save) => matches!(profiles.editor_result(), Some(Ok(_))),
            _ => true,
        }
    }

    /// The editor's field stops, in order; the first takes focus when the
    /// editor opens. Empty outside an editor.
    fn field_stops(&self, cx: &App) -> Vec<FocusHandle> {
        match (self.model.section(), self.model.mode_kind()) {
            (Section::Theme, ModeKind::Editor) => self.fields.iter().map(|f| f.focus_handle(cx)).collect(),
            (Section::Profiles, ModeKind::Editor) => self.profile_field_stops(cx),
            _ => Vec::new(),
        }
    }

    /// Where focus goes when a view opens: the section list (General has
    /// no controls), a list, an editor's name, or (asking to confirm a
    /// delete) the dialog itself.
    fn default_focus(&self, cx: &App) -> FocusHandle {
        match (self.model.section(), self.model.mode_kind()) {
            (Section::General, _) => self.sections_handle.clone(),
            (_, ModeKind::List) => self.list_handle.clone(),
            _ => self.field_stops(cx).into_iter().next().unwrap_or_else(|| self.focus_handle.clone()),
        }
    }

    /// Whether a select's menu is open; it holds focus while it is.
    fn menu_open(&self, cx: &App) -> bool {
        self.profile_selects.iter().any(|(_, s)| s.read(cx).is_open())
    }

    /// Moves focus to the new view's first control when the section or the
    /// kind of view changed, and off a control the view no longer shows
    /// (or shows disabled).
    fn place_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = (self.model.section(), self.model.mode_kind());
        let moved = view != self.placed_for;
        self.placed_for = view;
        if !moved && self.menu_open(cx) {
            return;
        }
        // On the section list, focus stays while the sections change under
        // it, by key or click: the user is still choosing a section.
        if self.sections_handle.is_focused(window) {
            return;
        }
        let stops = self.stops(cx);
        let valid = window.focused(cx).is_some_and(|f| f == self.focus_handle || stops.contains(&f));
        if moved || !valid {
            window.focus(&self.default_focus(cx));
        }
    }

    /// After any change: preview the theme the model now shows, keep the
    /// editor's inputs, the selects and focus in step with the model, and
    /// redraw.
    fn changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.preview(cx);
        self.sync_fields(window, cx);
        self.sync_selects(cx);
        self.place_focus(window, cx);
        match self.model.section() {
            Section::General => {}
            Section::Theme => {
                if matches!(self.model.mode, Mode::List) {
                    self.list_scroll.scroll_to_item(self.model.selected);
                }
            }
            Section::Profiles => {
                if matches!(self.model.profiles.mode, profiles::Mode::List) {
                    self.profile_list_scroll.scroll_to_item(self.model.profiles.selected);
                }
            }
        }
        cx.notify();
    }

    /// Shows the model's values in the selects: a refused write leaves the
    /// model as it was, and a reload or a newly opened editor sets them.
    fn sync_selects(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.model.profiles.editor() else { return };
        for (field, select) in &self.profile_selects {
            let current = editor.choice_index(*field);
            if select.read(cx).selected() != current {
                select.update(cx, |s, cx| s.set_selected(current, cx));
            }
        }
    }

    /// The select behind a choice field.
    fn choice_select(&self, choice: Choice) -> Option<Entity<Select>> {
        match choice {
            Choice::Profile(field) => self.profile_selects.iter().find(|(f, _)| *f == field).map(|(_, s)| s.clone()),
        }
    }

    /// Option `ix` of a choice field was chosen: change the edited profile.
    fn apply_choice(&mut self, choice: Choice, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        match choice {
            Choice::Profile(field) => self.edit_profile(window, cx, |e| e.choose(field, ix)),
        }
    }

    /// The slot a choice field's select is laid out in. It adds no keys: a
    /// choice changes only through the select's menu (`SelectEvent::Changed`).
    fn choice_slot(&self, choice: Choice) -> gpui::Div {
        div().children(self.choice_select(choice))
    }

    /// Scrolls an editor's focused field into view when focus moved, by
    /// Tab, a click or the keys.
    fn follow_focus(&mut self, window: &Window, cx: &App) {
        let focused = window.focused(cx);
        if focused == self.scrolled_to {
            return;
        }
        if let Some(field) = self.fields.iter().position(|f| f.read(cx).is_focused(window)) {
            // The name sits above the scrolling color rows.
            if field > 0 {
                self.editor_scroll.scroll_to_item(field - 1);
            }
        }
        self.scroll_to_focused_profile_field(window, cx);
        self.scrolled_to = focused;
    }

    fn preview(&mut self, cx: &mut Context<Self>) {
        if let Some(preview) = self.model.preview()
            && preview != self.theme
        {
            self.theme = preview.clone();
            cx.emit(SettingsEvent::Preview(preview));
        }
        let profile = self.model.profile_preview();
        let shown = profile.as_ref().map(|(name, p)| (name.clone(), p.theme.clone(), p.font_size));
        if shown != self.profile_previewed {
            self.profile_previewed = shown;
            cx.emit(SettingsEvent::ProfilePreview(profile));
        }
    }

    /// Creates the inputs when the editor opens and drops them when it
    /// closes. Focus follows in `place_focus`.
    fn sync_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_profile_fields(window, cx);
        let Some(texts) = self.model.editor_fields() else {
            self.fields.clear();
            self._field_subscriptions.clear();
            return;
        };
        if self.fields.is_empty() {
            for (field, text) in texts.iter().enumerate() {
                let input = cx.new(|cx| TextInput::new(text, cx));
                let changed = cx.subscribe_in(&input, window, move |this, input, _: &TextInputEvent, _, cx| {
                    let text = input.read(cx).text().to_string();
                    this.model.set_field(field, &text);
                    this.preview(cx);
                    cx.notify();
                });
                self._field_subscriptions.push(changed);
                self.fields.push(input);
            }
        }
    }

    fn outcome(&mut self, outcome: Outcome, window: &mut Window, cx: &mut Context<Self>) {
        match outcome {
            Outcome::Nothing => self.changed(window, cx),
            Outcome::Written => {
                // The preview ends (the editor closed) before the reload.
                self.changed(window, cx);
                cx.emit(SettingsEvent::ConfigWritten(self.model.take_renames()));
            }
            Outcome::Applied => {
                cx.emit(SettingsEvent::ConfigWritten(self.model.take_renames()));
                cx.emit(DismissEvent);
            }
            Outcome::Close => cx.emit(DismissEvent),
        }
    }

    /// Up and down move a list's selection. In an editor they do nothing
    /// (tab moves between fields), and a focused select takes them first,
    /// to open its menu.
    fn up(&mut self, _: &SettingsUp, window: &mut Window, cx: &mut Context<Self>) {
        self.move_up_down(false, window, cx);
    }

    fn down(&mut self, _: &SettingsDown, window: &mut Window, cx: &mut Context<Self>) {
        self.move_up_down(true, window, cx);
    }

    /// Home and end in a list.
    fn first(&mut self, _: &SettingsFirst, window: &mut Window, cx: &mut Context<Self>) {
        self.model.select_edge(false);
        self.changed(window, cx);
    }

    fn last(&mut self, _: &SettingsLast, window: &mut Window, cx: &mut Context<Self>) {
        self.model.select_edge(true);
        self.changed(window, cx);
    }

    fn move_up_down(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.mode_kind() == ModeKind::Editor {
            // Nothing to move: the key goes on, unhandled.
            cx.propagate();
            return;
        }
        self.model.move_selection(forward);
        self.changed(window, cx);
    }

    /// Enter: the section's confirm (apply, edit, save, delete, close), but
    /// not on a focused button, which takes the key itself.
    fn confirm(&mut self, _: &SettingsConfirm, window: &mut Window, cx: &mut Context<Self>) {
        // On the section list, the focused section is already the shown
        // one (activation follows focus), so enter has nothing to do.
        if self.sections_handle.is_focused(window) {
            return;
        }
        if !enter_confirms(self.button_focused(window)) {
            // On to the button's own key-down handler.
            cx.propagate();
            return;
        }
        let outcome = self.model.confirm();
        self.outcome(outcome, window, cx);
    }

    /// Escape (through the overlay's `Dismiss`, after an open select menu
    /// took its own) or the close `X`: closes in a list and in General,
    /// discards in an editor. Handling `Dismiss` here keeps the dialog open
    /// unless the model says it is done.
    fn dismiss(&mut self, _: &Dismiss, window: &mut Window, cx: &mut Context<Self>) {
        self.request_cancel(window, cx);
    }

    /// Escape or the close `X`: closes in a list and in General, discards in
    /// an editor.
    fn request_cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.cancel() {
            cx.emit(DismissEvent);
        } else {
            self.changed(window, cx);
        }
    }

    fn new_theme(&mut self, _: &SettingsNew, window: &mut Window, cx: &mut Context<Self>) {
        self.model.start_new();
        self.changed(window, cx);
    }

    fn edit_theme(&mut self, _: &SettingsEdit, window: &mut Window, cx: &mut Context<Self>) {
        self.model.start_edit();
        self.changed(window, cx);
    }

    fn delete_theme(&mut self, _: &SettingsDelete, window: &mut Window, cx: &mut Context<Self>) {
        self.model.request_delete();
        self.changed(window, cx);
    }

    fn key_down(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.key(&e.keystroke) {
            cx.stop_propagation();
            self.changed(window, cx);
        }
    }

    // ---- rendering ----

    /// The General section has no settings left; only its Close button.
    fn render_general(&self, cx: &mut Context<Self>) -> AnyElement {
        let body = div().flex_1();
        let footer = div()
            .flex()
            .items_center()
            .child(div().flex_1())
            .child(self.button(Btn::Close, "Close", ButtonVariant::Default, cx, |_, _, cx| cx.emit(DismissEvent)));
        div().flex().flex_col().flex_1().min_h_0().gap_2().child(body).child(footer).into_any_element()
    }

    /// The frame around the theme or profile list: one Tab stop
    /// (`list_handle`) in the `SettingsList` key context, whose keys move
    /// the selection, with the focus ring when it got focus from the
    /// keyboard. A press in it focuses it, as a pointer press on a control
    /// does. The ring is on this frame, not on the scrolling list inside,
    /// which would clip it.
    fn list_frame(&self, list: impl IntoElement, window: &Window, cx: &App) -> gpui::Div {
        let ring = brindle_ui::focus_ring(cx.ui_theme());
        let shown = brindle_ui::shows_ring(self.list_handle.is_focused(window), brindle_ui::modality(window, cx), false);
        let handle = self.list_handle.clone();
        div()
            .key_context("SettingsList")
            .track_focus(&self.list_handle)
            .on_mouse_down(MouseButton::Left, move |_, window, cx| brindle_ui::focus_from_pointer(&handle, window, cx))
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .rounded_md()
            .focus_ring(shown, ring, window)
            .child(list)
    }

    /// A footer button: small, focusable through its own handle, disabled
    /// when [`Self::enabled`] says so.
    fn button(
        &self,
        b: Btn,
        label: impl Into<SharedString>,
        variant: ButtonVariant,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> Button {
        Button::new(b.id(), label)
            .variant(variant)
            .size(ButtonSize::Sm)
            .disabled(!self.enabled(b))
            .focus(&self.button_handle(b))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| on_click(this, window, cx)))
    }

    fn swatch(color: Option<Color>, size: f32, t: &Tokens) -> impl IntoElement {
        div()
            .flex_none()
            .size(px(size))
            .rounded_sm()
            .border_1()
            .border_color(t.input)
            .when_some(color, |d, c| d.bg(c.hsla()))
    }

    fn render_list(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let t = *cx.ui_theme();
        let fg = t.foreground;
        let muted = t.muted_foreground;
        let model = &self.model;

        let rows = model.entries.iter().enumerate().map(|(ix, entry)| {
            let selected = ix == model.selected;
            let active = entry.name == model.active && model.selectable_index(&model.active) == Some(ix);
            let readable = entry.theme.is_ok();
            let mut row = div()
                .id(("theme", ix))
                .flex()
                .flex_none()
                .items_center()
                .gap_3()
                .px_3()
                .py_1p5()
                .rounded_md()
                .when(selected, |d| d.bg(t.accent))
                .when(readable, |d| {
                    d.when(!selected, |d| d.hover(|d| d.bg(t.accent.opacity(0.5)))).on_click(cx.listener(move |this, e: &gpui::ClickEvent, window, cx| {
                        this.model.select(ix);
                        if e.click_count() >= 2 {
                            let outcome = this.model.confirm();
                            this.outcome(outcome, window, cx);
                        } else {
                            this.changed(window, cx);
                        }
                    }))
                });
            row = match &entry.theme {
                Ok(theme) => row.child(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap_0p5()
                        .p_1()
                        .rounded_sm()
                        .bg(theme.background.hsla())
                        .child(div().px_1().text_xs().text_color(theme.foreground.hsla()).child("Aa"))
                        .children((1..7).map(|i| div().size(px(8.0)).rounded_full().bg(theme.ansi[i].hsla()))),
                ),
                Err(_) => row.child(div().w(px(84.0)).flex_none().text_xs().text_color(t.destructive).child("unreadable")),
            };
            row.child(
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .child(div().text_color(if readable { fg } else { muted }).child(SharedString::from(entry.name.clone())))
                    .when_some(entry.theme.as_ref().err(), |d, err| {
                        d.child(div().text_xs().text_color(muted).child(SharedString::from(err.clone())))
                    }),
            )
            .when(entry.custom, |d| d.child(div().flex_none().text_xs().text_color(muted).child("custom")))
            .when(active, |d| d.child(div().flex_none().text_xs().text_color(t.primary).child("● active")))
        })
        .collect::<Vec<_>>();

        let confirming = matches!(model.mode, Mode::ConfirmDelete);
        let footer = if confirming {
            let name = model.selected_entry().map(|e| e.name.clone()).unwrap_or_default();
            self.confirm_footer(format!("Delete {name:?}?"), cx)
        } else {
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(self.button(Btn::New, "New (n)", ButtonVariant::Outline, cx, |this, window, cx| {
                    this.model.start_new();
                    this.changed(window, cx);
                }))
                .child(self.button(Btn::Edit, "Edit (e)", ButtonVariant::Outline, cx, |this, window, cx| {
                    this.model.start_edit();
                    this.changed(window, cx);
                }))
                .child(self.button(Btn::Delete, "Delete (d)", ButtonVariant::Destructive, cx, |this, window, cx| {
                    this.model.request_delete();
                    this.changed(window, cx);
                }))
                .child(div().flex_1())
                .child(self.button(Btn::Cancel, "Cancel", ButtonVariant::Outline, cx, |_, _, cx| cx.emit(DismissEvent)))
                .child(self.button(Btn::Apply, "Apply", ButtonVariant::Default, cx, |this, window, cx| {
                    let outcome = this.model.confirm();
                    this.outcome(outcome, window, cx);
                }))
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap_2()
            .child(div().text_xs().text_color(muted).child("Pick a theme to preview it; Apply makes it the default."))
            .child(self.list_frame(
                div()
                    .id("theme-list")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.list_scroll)
                    .children(rows),
                window,
                cx,
            ))
            .child(footer)
            .into_any_element()
    }

    /// The confirmation footer of a delete: the question, Keep, and Delete.
    pub(super) fn confirm_footer(&self, question: String, cx: &mut Context<Self>) -> gpui::Div {
        let fg = cx.ui_theme().foreground;
        div()
            .flex()
            .items_center()
            .gap_2()
            .child(div().flex_1().min_w_0().text_color(fg).child(SharedString::from(question)))
            .child(self.button(Btn::Keep, "Keep", ButtonVariant::Outline, cx, |this, window, cx| {
                this.model.cancel();
                this.changed(window, cx);
            }))
            .child(self.button(Btn::ConfirmDelete, "Delete (enter)", ButtonVariant::Destructive, cx, |this, window, cx| {
                let outcome = this.model.confirm();
                this.outcome(outcome, window, cx);
            }))
    }

    /// Editor field `field`'s input, framed, red when `invalid`.
    fn render_field(&self, field: usize, invalid: bool) -> impl IntoElement {
        div()
            .flex_1()
            .min_w_0()
            .children(self.fields.get(field).map(|input| Input::new(input).invalid(invalid)))
    }

    fn render_editor(&self, editor: &Editor, cx: &mut Context<Self>) -> AnyElement {
        let t = *cx.ui_theme();
        let muted = t.muted_foreground;
        let name_error = editor.name_result(&self.model.other_names(editor)).err();

        let label = |text: SharedString| div().w(px(150.0)).flex_none().text_color(muted).child(text);
        let rows = SLOTS.iter().enumerate().map(|(ix, slot)| {
            let field = ix + 1;
            let invalid = editor.parse_color(ix).is_err();
            let color = slot.get(&editor.current);
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .child(label(slot.label().into()))
                .child(Self::swatch(color, 18.0, &t))
                .child(self.render_field(field, invalid))
                .when(slot.optional(), |d| {
                    d.child(self.button(Btn::Clear, "Clear", ButtonVariant::Outline, cx, |this, window, cx| {
                        if let Some(field) = this.model.clear_selection_foreground()
                            && let Some(input) = this.fields.get(field).cloned()
                        {
                            input.update(cx, |input, cx| input.set_text("", cx));
                            // Clear is disabled now: focus its field.
                            window.focus(&input.focus_handle(cx));
                        }
                        this.changed(window, cx);
                    }))
                })
        })
        .collect::<Vec<_>>();

        let title = match &editor.original {
            Some(name) => format!("Edit {name:?}"),
            None => "New theme".to_string(),
        };
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap_2()
            .child(div().text_color(t.foreground).child(SharedString::from(title)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(label("Name".into()))
                    .child(self.render_field(0, name_error.is_some())),
            )
            .when_some(name_error, |d, e| d.child(div().text_xs().text_color(t.destructive).child(SharedString::from(e))))
            .child(
                div()
                    .id("editor-fields")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .gap_1()
                    .overflow_y_scroll()
                    .track_scroll(&self.editor_scroll)
                    .children(rows),
            )
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1().min_w_0().text_xs().text_color(muted).child("tab moves between fields · #rrggbb or #rgb"))
                    .child(self.editor_buttons(cx)),
            )
            .into_any_element()
    }

    /// An editor's Discard and Save.
    pub(super) fn editor_buttons(&self, cx: &mut Context<Self>) -> gpui::Div {
        div()
            .flex()
            .flex_none()
            .gap_2()
            .child(self.button(Btn::Discard, "Discard", ButtonVariant::Outline, cx, |this, window, cx| {
                this.model.cancel();
                this.changed(window, cx);
            }))
            .child(self.button(Btn::Save, "Save", ButtonVariant::Default, cx, |this, window, cx| {
                let outcome = this.model.confirm();
                this.outcome(outcome, window, cx);
            }))
    }
}

impl Render for SettingsDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Focus moves by Tab, clicks and keys; scroll an editor after it.
        self.follow_focus(window, cx);
        let t = *cx.ui_theme();
        let content = match (self.model.section(), &self.model.mode) {
            (Section::General, _) => self.render_general(cx),
            (Section::Profiles, _) => self.render_profiles(window, cx),
            (Section::Theme, Mode::Editor(editor)) => {
                let editor = editor.clone();
                self.render_editor(&editor, cx)
            }
            (Section::Theme, _) => self.render_list(window, cx),
        };
        let current = self.model.section();
        // The section list is a vertical tab list: one Tab stop whose
        // focused tab is the shown section, ringed while focus came from
        // the keyboard.
        let ring = brindle_ui::focus_ring(&t);
        let ring_shown =
            brindle_ui::shows_ring(self.sections_handle.is_focused(window), brindle_ui::modality(window, cx), false);
        let sections = SECTIONS.iter().map(|&section| {
            div()
                .id(section.label())
                .relative()
                .px_3()
                .py_1p5()
                .rounded_md()
                .when(section == current, |d| d.bg(t.accent).text_color(t.accent_foreground))
                .when(section != current, |d| d.text_color(t.foreground).hover(|d| d.bg(t.accent.opacity(0.5))))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.model.select_section(section);
                    this.changed(window, cx);
                }))
                .focus_ring(ring_shown && section == current, ring, window)
                .child(section.label())
        });
        let sections_handle = self.sections_handle.clone();
        div()
            .key_context("SettingsDialog")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::first))
            .on_action(cx.listener(Self::last))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::dismiss))
            .on_action(cx.listener(Self::new_theme))
            .on_action(cx.listener(Self::edit_theme))
            .on_action(cx.listener(Self::delete_theme))
            .on_action(cx.listener(Self::section_next))
            .on_action(cx.listener(Self::section_prev))
            .on_action(cx.listener(Self::section_first))
            .on_action(cx.listener(Self::section_last))
            .on_action(cx.listener(Self::duplicate))
            .on_action(cx.listener(Self::move_up))
            .on_action(cx.listener(Self::move_down))
            .on_key_down(cx.listener(Self::key_down))
            // The overlay's backdrop already blocks the pointer and ignores
            // presses outside; the cursor stays an arrow over the panel.
            .cursor(CursorStyle::Arrow)
            .text_sm()
            .child(
                Dialog::new()
                    .title("Settings")
                    .w(px(760.0))
                    .h(px(460.0))
                    .p_4()
                    .gap_3()
                    .close_focus(&self.close_handle)
                    .on_close(cx.listener(|this, _: &ClickEvent, window, cx| this.request_cancel(window, cx)))
                    .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .key_context("SettingsSections")
                            .track_focus(&self.sections_handle)
                            // A press on a section focuses the list, as a
                            // press on a tab focuses it, without a ring.
                            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                                brindle_ui::focus_from_pointer(&sections_handle, window, cx)
                            })
                            .flex()
                            .flex_col()
                            .flex_none()
                            .w(px(140.0))
                            .p_2()
                            .gap_1()
                            .border_r_1()
                            .border_color(t.border)
                            .children(sections),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .p_3()
                            .gap_2()
                            .child(content)
                            .when_some(self.model.error.clone(), |d, e| {
                                d.child(
                                    div()
                                        .flex_none()
                                        .px_2()
                                        .py_1()
                                        .rounded_md()
                                        .text_xs()
                                        .bg(t.destructive.opacity(0.25))
                                        .text_color(t.foreground)
                                        .child(SharedString::from(e)),
                                )
                            }),
                    ),
            ),
            )
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

    struct Fixture {
        dir: PathBuf,
        path: PathBuf,
    }

    impl Fixture {
        fn new(name: &str, text: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("brindle-settings-{}-{name}", std::process::id()));
            std::fs::remove_dir_all(&dir).ok();
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("config.toml");
            std::fs::write(&path, text).unwrap();
            Self { dir, path }
        }

        /// A model on the Theme section, where most tests start.
        fn model(&self, active: &str) -> Model {
            let mut model = self.general(active);
            model.select_section(Section::Theme);
            model
        }

        /// A model as the dialog opens: on the General section.
        fn general(&self, active: &str) -> Model {
            Model::new(self.path.clone(), active.into(), BTreeMap::new(), Vec::new(), None)
        }

        fn text(&self) -> String {
            std::fs::read_to_string(&self.path).unwrap()
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.dir).ok();
        }
    }

    fn names(model: &Model) -> Vec<&str> {
        model.entries.iter().map(|e| e.name.as_str()).collect()
    }

    fn key(model: &mut Model, keys: &str) {
        for k in keys.split(' ') {
            assert!(model.key(&Keystroke::parse(k).unwrap()), "{k} not handled");
        }
    }

    /// Edits field `field` as its input would: the model gets the new text.
    fn type_into(model: &mut Model, field: usize, text: &str) {
        model.set_field(field, text);
    }

    fn editor(model: &Model) -> &Editor {
        match &model.mode {
            Mode::Editor(e) => e,
            _ => panic!("not editing: {}", model.describe("")),
        }
    }

    #[test]
    fn list_order_and_initial_selection() {
        let f = Fixture::new("order", CONFIG);
        let model = f.model("mine");
        assert_eq!(
            names(&model),
            ["brindle-dark", "brindle-light", "tokyo-night", "gruvbox-dark", "solarized-dark", "broken", "mine", "spare"]
        );
        assert_eq!(model.selected_entry().unwrap().name, "mine");
        assert!(model.entries[5].theme.is_err());
        // Unknown or unreadable active themes fall back to brindle-dark.
        assert_eq!(f.model("broken").selected_entry().unwrap().name, "brindle-dark");
        assert_eq!(f.model("nope").selected_entry().unwrap().name, "brindle-dark");
    }

    #[test]
    fn moving_skips_unreadable_themes() {
        let f = Fixture::new("skip", CONFIG);
        let mut model = f.model("solarized-dark");
        model.move_selection(true);
        assert_eq!(model.selected_entry().unwrap().name, "mine");
        model.move_selection(false);
        assert_eq!(model.selected_entry().unwrap().name, "solarized-dark");
        model.select(5);
        assert_eq!(model.selected_entry().unwrap().name, "solarized-dark");
        assert_eq!(model.preview(), builtin_theme("solarized-dark"));
    }

    /// The section list's keys: down and up step and wrap (Radix's `loop`),
    /// home and end go to the ends.
    #[test]
    fn section_keys_step_wrap_and_reach_the_ends() {
        let f = Fixture::new("section-keys", CONFIG);
        let mut model = f.general("mine");
        assert!(model.next_section(true));
        assert_eq!(model.section(), Section::Profiles);
        assert!(model.next_section(true));
        assert_eq!(model.section(), Section::Theme);
        assert!(model.next_section(true));
        assert_eq!(model.section(), Section::General);
        assert!(model.edge_section(true));
        assert_eq!(model.section(), Section::Theme);
        assert!(model.edge_section(false));
        assert_eq!(model.section(), Section::General);
    }

    #[test]
    fn home_and_end_select_the_lists_edges() {
        let f = Fixture::new("edges", CONFIG);
        let mut model = f.model("mine");
        model.select_edge(true);
        assert_eq!(model.selected_entry().unwrap().name, "spare");
        model.select_edge(false);
        assert_eq!(model.selected_entry().unwrap().name, "brindle-dark");
        // In General there is no list: nothing changes.
        let mut general = f.general("mine");
        general.select_edge(true);
        assert_eq!(general.section(), Section::General);
    }

    #[test]
    fn apply_writes_the_global_theme() {
        let f = Fixture::new("apply", CONFIG);
        let mut model = f.model("mine");
        model.move_selection(true);
        assert_eq!(model.confirm(), Outcome::Applied);
        assert_eq!(f.text(), CONFIG.replace("theme = \"mine\"", "theme = \"spare\""));
    }

    #[test]
    fn apply_on_a_malformed_file_keeps_the_dialog() {
        let f = Fixture::new("malformed", "theme = \"mine\"\n[oops\n");
        let mut model = Model::new(
            f.path.clone(),
            "mine".into(),
            BTreeMap::from([("mine".into(), Theme::default())]),
            Vec::new(),
            None,
        );
        model.select_section(Section::Theme);
        // The fallback themes are still listed.
        assert_eq!(model.selected_entry().unwrap().name, "mine");
        model.move_selection(false);
        assert_eq!(model.confirm(), Outcome::Nothing);
        assert!(model.error.as_deref().unwrap().contains("not valid TOML"));
        assert!(model.in_list());
        assert_eq!(model.selected_entry().unwrap().name, "solarized-dark");
        assert_eq!(f.text(), "theme = \"mine\"\n[oops\n");
    }

    #[test]
    fn cancel_closes_from_the_list() {
        let f = Fixture::new("cancel", CONFIG);
        let mut model = f.model("mine");
        model.move_selection(true);
        assert!(model.cancel());
        assert_eq!(f.text(), CONFIG);
    }

    #[test]
    fn new_copies_the_selected_theme() {
        let f = Fixture::new("new", CONFIG);
        let mut model = f.model("tokyo-night");
        key(&mut model, "n");
        let tokyo = builtin_theme("tokyo-night").unwrap();
        let e = editor(&model);
        assert_eq!(e.name, "tokyo-night-custom");
        assert_eq!(e.original, None);
        assert_eq!(e.current.accent, Some(tokyo.ansi[4]));
        assert_eq!(e.current, tokyo.with_effective_colors());
        assert_eq!(model.preview(), Some(tokyo.with_effective_colors()));
        // Edit on a built-in copies it too.
        model.cancel();
        key(&mut model, "e");
        assert_eq!(editor(&model).name, "tokyo-night-custom");
    }

    #[test]
    fn color_edits_preview_and_validate() {
        let f = Fixture::new("colors", CONFIG);
        let mut model = f.model("brindle-dark");
        model.start_new();
        // Field 2 is the background.
        type_into(&mut model, 2, "#000");
        assert_eq!(model.preview().unwrap().background, Color::hex(0));
        assert!(model.editor_can_save());
        // Field 3 is the cursor: an invalid value keeps the previous color.
        let cursor = model.preview().unwrap().cursor;
        assert_eq!(field_label(3), "Cursor");
        let text = model.editor_fields().unwrap()[3].clone();
        model.set_field(3, &text[..text.len() - 1]);
        assert_eq!(model.preview().unwrap().cursor, cursor);
        assert!(!model.editor_can_save());
        let line = model.describe("Cursor");
        assert!(line.contains("field=\"Cursor\" invalid=[\"Cursor\"]"), "{line}");
        model.set_field(3, &format!("{}x", &text[..text.len() - 1]));
        assert_eq!(model.confirm(), Outcome::Nothing);
        assert!(model.error.is_some());
        assert_eq!(f.text(), CONFIG);
    }

    #[test]
    fn name_rules() {
        let f = Fixture::new("names", CONFIG);
        let mut model = f.model("mine");
        model.start_edit();
        // Keeping its own name is fine.
        assert!(model.editor_can_save());
        type_into(&mut model, 0, "spare");
        assert!(!model.editor_can_save());
        type_into(&mut model, 0, "tokyo-night");
        assert!(!model.editor_can_save());
        assert_eq!(model.confirm(), Outcome::Nothing);
        assert!(model.error.as_deref().unwrap().contains("built-in"));
    }

    #[test]
    fn saving_a_new_theme_keeps_the_global_theme() {
        let f = Fixture::new("save-new", CONFIG);
        let mut model = f.model("mine");
        model.move_selection(true); // spare
        model.start_new();
        assert_eq!(model.confirm(), Outcome::Written);
        assert!(model.in_list());
        assert_eq!(model.selected_entry().unwrap().name, "spare-custom");
        let config = crate::config::Config::parse(&f.text().replace("background = \"nope\"", "")).unwrap();
        assert_eq!(config.theme, "mine");
        assert_eq!(config.themes["spare-custom"].background, Color::hex(0x111111));
        assert_eq!(model.preview().unwrap().background, Color::hex(0x111111));
    }

    #[test]
    fn renaming_the_active_theme() {
        let f = Fixture::new("rename", &CONFIG.replace("name = \"Shell\"", "name = \"Shell\"\ntheme = \"mine\""));
        let mut model = f.model("mine");
        model.start_edit();
        type_into(&mut model, 0, "ours");
        assert_eq!(model.confirm(), Outcome::Written);
        let text = f.text();
        assert!(text.contains("theme = \"ours\"\n\n[[profiles]]"), "{text}");
        assert!(text.contains("name = \"Shell\"\ntheme = \"ours\""), "{text}");
        assert!(text.contains("[themes.ours]") && !text.contains("mine"), "{text}");
        assert_eq!(model.active, "ours");
        assert_eq!(model.selected_entry().unwrap().name, "ours");
    }

    #[test]
    fn discard_restores_the_selection() {
        let f = Fixture::new("discard", CONFIG);
        let mut model = f.model("mine");
        model.move_selection(true);
        model.start_edit();
        type_into(&mut model, 2, "#1");
        assert!(!model.cancel());
        assert!(model.in_list());
        assert_eq!(model.selected_entry().unwrap().name, "spare");
        assert_eq!(model.preview().unwrap().background, Color::hex(0x111111));
        assert_eq!(f.text(), CONFIG);
    }

    #[test]
    fn clearing_the_selection_foreground() {
        let f = Fixture::new("clear", CONFIG);
        let mut model = f.model("solarized-dark");
        model.start_new();
        assert!(editor(&model).current.selection_foreground.is_some());
        model.clear_selection_foreground();
        assert_eq!(model.preview().unwrap().selection_foreground, None);
        assert!(model.editor_can_save());
        assert_eq!(model.confirm(), Outcome::Written);
        assert!(!f.text().contains("selection_foreground"));
    }

    #[test]
    fn editor_fields_and_keys() {
        let f = Fixture::new("fields", CONFIG);
        let mut model = f.model("mine");
        assert_eq!(model.editor_fields(), None);
        model.start_edit();
        let fields = model.editor_fields().unwrap();
        assert_eq!(fields.len(), FIELD_COUNT);
        assert_eq!(fields[0], "mine");
        assert_eq!(fields[2], "#000000");
        assert_eq!((field_label(0), field_label(1)), ("name", "Foreground"));
        // Typing belongs to the inputs; list shortcuts are off, and up and
        // down leave the editor (the view moves focus between fields).
        assert!(!model.key(&Keystroke::parse("n").unwrap()));
        model.move_selection(true);
        assert_eq!(model.mode_kind(), ModeKind::Editor);
        type_into(&mut model, 6, "#abcdef");
        assert!(model.selection_foreground_set());
        assert_eq!(model.clear_selection_foreground(), Some(6));
        assert_eq!(selection_foreground_field(), 6);
        assert_eq!(model.editor_fields().unwrap()[6], "");
        assert!(!model.selection_foreground_set());
    }

    /// Escape (the overlay's dismiss request) per section and mode: it
    /// closes the dialog from a list and from General, and otherwise leaves
    /// the editor or the confirmation for the list, keeping the dialog.
    #[test]
    fn escape_outcome_per_mode() {
        // A second profile, so one can be asked about deleting.
        let config = CONFIG.replace("name = \"Shell\"\n", "name = \"Shell\"\n\n[[profiles]]\nname = \"Work\"\n");
        let f = Fixture::new("escape", &config);
        let mut model = f.general("mine");
        assert_eq!(model.mode_kind(), ModeKind::List);
        assert!(model.cancel(), "General closes");

        model.select_section(Section::Theme);
        assert!(model.cancel(), "the theme list closes");
        model.start_edit();
        type_into(&mut model, 2, "#123456");
        assert_eq!(model.mode_kind(), ModeKind::Editor);
        assert!(!model.cancel(), "the theme editor discards");
        assert_eq!(model.mode_kind(), ModeKind::List);
        model.select(7);
        key(&mut model, "d");
        assert_eq!(model.mode_kind(), ModeKind::ConfirmDelete);
        assert!(!model.cancel(), "the theme delete question goes back to the list");
        assert_eq!(model.mode_kind(), ModeKind::List);

        model.select_section(Section::Profiles);
        assert!(model.cancel(), "the profile list closes");
        key(&mut model, "e");
        assert_eq!(model.mode_kind(), ModeKind::Editor);
        assert!(!model.cancel(), "the profile editor discards");
        assert_eq!(model.mode_kind(), ModeKind::List);
        key(&mut model, "d");
        assert_eq!(model.mode_kind(), ModeKind::ConfirmDelete);
        assert!(!model.cancel(), "the profile delete question goes back to the list");
        assert_eq!(model.mode_kind(), ModeKind::List);
        // Nothing was written.
        assert_eq!(f.text(), config);
    }

    /// Enter on a focused button (or the close `X`) is the button's: the
    /// dialog's confirm doesn't run for it.
    #[test]
    fn enter_on_a_button_is_the_buttons() {
        assert!(enter_confirms(false));
        assert!(!enter_confirms(true));
    }

    /// Each view's buttons, in Tab order, and the dump label of each.
    #[test]
    fn footer_buttons_per_view() {
        assert_eq!(footer_buttons(Section::General, ModeKind::List), [Btn::Close]);
        assert_eq!(
            footer_buttons(Section::Theme, ModeKind::List),
            [Btn::New, Btn::Edit, Btn::Delete, Btn::Cancel, Btn::Apply]
        );
        for section in [Section::Theme, Section::Profiles] {
            assert_eq!(footer_buttons(section, ModeKind::Editor), [Btn::Discard, Btn::Save]);
            assert_eq!(footer_buttons(section, ModeKind::ConfirmDelete), [Btn::Keep, Btn::ConfirmDelete]);
        }
        assert_eq!(footer_buttons(Section::Profiles, ModeKind::List).last(), Some(&Btn::Close));
        // Ids are unique, so the shared handles never meet in one view.
        let ids: std::collections::HashSet<_> = BUTTONS.iter().map(|b| b.id()).collect();
        assert_eq!(ids.len(), BUTTONS.len());
        assert_eq!(Btn::Discard.label(), "Discard");
    }

    #[test]
    fn switching_sections() {
        let f = Fixture::new("sections", CONFIG);
        let mut model = f.model("mine");
        assert_eq!(model.section(), Section::Theme);
        // An unapplied preview is dropped on leaving Theme.
        model.move_selection(true);
        assert_eq!(model.selected_entry().unwrap().name, "spare");
        assert!(model.next_section(false));
        assert_eq!(model.section(), Section::Profiles);
        assert_eq!(model.selected_entry().unwrap().name, "mine");
        assert!(model.describe("").starts_with("--- settings section=Profiles mode=list selected=\"Shell\""), "{}", model.describe(""));
        // Escape in the profile list closes the dialog.
        assert!(model.cancel());
        // A dirty profile editor holds the section; a clean one doesn't.
        key(&mut model, "e");
        assert!(!model.in_list());
        assert!(model.select_section(Section::Theme));
        assert!(model.in_list());
        model.select_section(Section::Profiles);
        key(&mut model, "e");
        model.profiles.editor_mut().unwrap().set_text(profiles::Field::Cwd, "~/src");
        assert!(!model.select_section(Section::Theme));
        assert_eq!(model.section(), Section::Profiles);
        assert!(model.error.as_deref().unwrap().contains("Save or discard"));
        // The same for the theme editor.
        model.cancel();
        model.select_section(Section::Theme);
        model.start_edit();
        type_into(&mut model, 2, "#123456");
        assert!(!model.next_section(true));
        assert_eq!(f.text(), CONFIG);
    }

    #[test]
    fn opens_on_general_and_sections_wrap() {
        let f = Fixture::new("general-order", CONFIG);
        let mut model = f.general("mine");
        assert_eq!(model.section(), Section::General);
        assert_eq!(SECTIONS.map(Section::label), ["General", "Profiles", "Theme"]);
        assert!(model.in_list());
        assert!(model.next_section(true));
        assert_eq!(model.section(), Section::Profiles);
        assert!(model.next_section(true));
        assert_eq!(model.section(), Section::Theme);
        // The theme list shows the active theme.
        assert_eq!(model.selected_entry().unwrap().name, "mine");
        assert!(model.next_section(true));
        assert_eq!(model.section(), Section::General);
        assert!(model.next_section(false));
        assert_eq!(model.section(), Section::Theme);
        // In General, up/down and list keys do nothing; enter and escape close.
        model.select_section(Section::General);
        model.move_selection(true);
        assert!(!model.key(&Keystroke::parse("n").unwrap()));
        assert_eq!(model.confirm(), Outcome::Close);
        assert!(model.cancel());
        assert_eq!(f.text(), CONFIG);
    }

    #[test]
    fn general_has_no_settings() {
        let f = Fixture::new("general-write", CONFIG);
        let model = f.general("mine");
        assert_eq!(model.describe(""), "--- settings section=General error=None\n");
        assert_eq!(f.text(), CONFIG);
    }

    #[test]
    fn copies_save_the_derived_search_colors() {
        let f = Fixture::new("search-colors", CONFIG);
        let mut model = f.model("tokyo-night");
        model.start_new();
        let tokyo = builtin_theme("tokyo-night").unwrap();
        assert_eq!(editor(&model).current.search_focused_match_background, Some(tokyo.ansi[3]));
        assert_eq!(model.confirm(), Outcome::Written);
        let text = f.text();
        let (match_bg, match_fg) = tokyo.search_match();
        for (key, color) in [
            ("search_match_background", match_bg),
            ("search_match_foreground", match_fg),
            ("search_focused_match_background", tokyo.ansi[3]),
            ("search_focused_match_foreground", tokyo.background),
        ] {
            assert!(text.contains(&format!("{key} = \"{color}\"")), "{key} missing in:\n{text}");
        }
    }

    #[test]
    fn deleting_themes() {
        let f = Fixture::new("delete", CONFIG);
        let mut model = f.model("mine");
        // In use as the global theme.
        key(&mut model, "d");
        assert!(model.in_list());
        assert!(model.error.as_deref().unwrap().contains("the global theme"));
        // Built-ins can't be deleted.
        model.select(0);
        key(&mut model, "d");
        assert!(model.error.as_deref().unwrap().contains("built-in"));
        // An unused one goes after confirming.
        model.select(7);
        key(&mut model, "delete");
        assert!(model.describe("").contains("mode=confirm-delete theme=\"spare\""));
        assert!(!model.cancel());
        assert_eq!(f.text(), CONFIG);
        key(&mut model, "d");
        assert_eq!(model.confirm(), Outcome::Written);
        assert!(!f.text().contains("spare"));
        assert!(!names(&model).contains(&"spare"));
        assert_eq!(model.selected_entry().unwrap().name, "mine");
    }
}
