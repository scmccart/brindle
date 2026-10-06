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
    AnyElement, App, Context, EventEmitter, FocusHandle, Focusable, Hsla, InteractiveElement, IntoElement,
    KeyDownEvent, Keystroke, MouseButton, ParentElement, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px,
};

use crate::actions::{
    Paste, SettingsCancel, SettingsConfirm, SettingsDelete, SettingsDown, SettingsEdit, SettingsNew, SettingsNextField,
    SettingsPrevField, SettingsUp,
};
use crate::config_edit;
use crate::picker::TextField;
use crate::theme::{BUILTIN_THEMES, Color, Theme, builtin_theme};

const SECTIONS: &[&str] = &["Theme"];

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

const SLOTS: [Slot; 23] = [
    Slot::Foreground,
    Slot::Background,
    Slot::Cursor,
    Slot::CursorText,
    Slot::SelectionBackground,
    Slot::SelectionForeground,
    Slot::Accent,
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
    name: TextField,
    /// One field per [`SLOTS`] entry.
    colors: Vec<TextField>,
    /// 0 is the name; `n` is `colors[n - 1]`.
    focused: usize,
    /// The theme as of the last valid edit of every field.
    current: Theme,
}

const FIELD_COUNT: usize = 1 + SLOTS.len();

impl Editor {
    fn new(original: Option<String>, name: String, theme: &Theme, return_to: usize) -> Self {
        let current = theme.with_effective_colors();
        let colors = SLOTS
            .iter()
            .map(|slot| TextField::new(&slot.get(&current).map(|c| c.to_string()).unwrap_or_default()))
            .collect();
        Self { original, return_to, name: TextField::new(&name), colors, focused: 0, current }
    }

    /// Parses color field `ix`: `Ok(None)` is a cleared optional color.
    fn parse_color(&self, ix: usize) -> Result<Option<Color>, &'static str> {
        let text = self.colors[ix].text.trim();
        if text.is_empty() && SLOTS[ix].optional() {
            return Ok(None);
        }
        Color::parse(text).map(Some).ok_or("use #rrggbb or #rgb")
    }

    fn invalid_colors(&self) -> Vec<usize> {
        (0..SLOTS.len()).filter(|&ix| self.parse_color(ix).is_err()).collect()
    }

    fn name_result(&self, others: &[&str]) -> Result<String, String> {
        config_edit::validate_theme_name(&self.name.text, others.iter().copied())
    }

    /// Applies `f` to the focused field and updates the preview theme if the
    /// field now holds a valid color.
    fn edit_focused(&mut self, f: impl FnOnce(&mut TextField)) {
        match self.focused {
            0 => f(&mut self.name),
            n => {
                let ix = n - 1;
                f(&mut self.colors[ix]);
                if let Ok(color) = self.parse_color(ix) {
                    SLOTS[ix].set(&mut self.current, color);
                }
            }
        }
    }

    fn move_focus(&mut self, forward: bool) {
        self.focused = if forward { (self.focused + 1) % FIELD_COUNT } else { (self.focused + FIELD_COUNT - 1) % FIELD_COUNT };
    }

    fn focused_label(&self) -> &'static str {
        match self.focused {
            0 => "name",
            n => SLOTS[n - 1].label(),
        }
    }
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
}

impl Model {
    pub fn new(config_path: PathBuf, active: String, fallback: BTreeMap<String, Theme>) -> Self {
        let mut model =
            Self { config_path, entries: Vec::new(), active, selected: 0, mode: Mode::List, error: None, fallback };
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
            Mode::Editor(editor) => editor.move_focus(forward),
            Mode::ConfirmDelete => {}
        }
    }

    pub fn select(&mut self, ix: usize) {
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
        match &self.mode {
            Mode::List => self.apply(),
            Mode::Editor(_) => self.save_editor(),
            Mode::ConfirmDelete => self.delete_selected(),
        }
    }

    /// Escape: `true` when the whole dialog should close (cancel).
    pub fn cancel(&mut self) -> bool {
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

    /// Typing in the editor's focused field, or list shortcuts.
    pub fn key(&mut self, ks: &Keystroke) -> bool {
        match &mut self.mode {
            Mode::Editor(editor) => {
                if !TextField::is_edit_key(ks) {
                    return false;
                }
                editor.edit_focused(|f| f.apply_key(ks));
                self.error = None;
                true
            }
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

    pub fn paste(&mut self, text: &str) {
        if let Mode::Editor(editor) = &mut self.mode {
            editor.edit_focused(|f| f.paste(text));
        }
    }

    pub fn focus_field(&mut self, field: usize) {
        if let Mode::Editor(editor) = &mut self.mode {
            editor.focused = field.min(FIELD_COUNT - 1);
        }
    }

    pub fn clear_selection_foreground(&mut self) {
        if let Mode::Editor(editor) = &mut self.mode {
            let ix = SLOTS.iter().position(|s| *s == Slot::SelectionForeground).unwrap();
            editor.focused = ix + 1;
            editor.edit_focused(|f| f.text.clear());
        }
    }

    pub fn in_list(&self) -> bool {
        matches!(self.mode, Mode::List)
    }

    /// Debug description for `--dump-screen-after`.
    pub fn describe(&self) -> String {
        let section = SECTIONS[0];
        match &self.mode {
            Mode::List => format!(
                "--- settings section={section} mode=list selected={:?} active={:?} error={:?}\n",
                self.selected_entry().map(|e| e.name.as_str()).unwrap_or(""),
                self.active,
                self.error
            ),
            Mode::Editor(editor) => format!(
                "--- settings section={section} mode=editor name={:?} field={:?} invalid={:?} can_save={} error={:?}\n",
                editor.name.text,
                editor.focused_label(),
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
    /// The config file was written; reload it.
    ConfigWritten,
    /// Close the dialog, ending any preview.
    Close,
}

pub struct SettingsDialog {
    focus_handle: FocusHandle,
    model: Model,
    /// The dialog's own colors: the previewed theme.
    theme: Theme,
    list_scroll: ScrollHandle,
    editor_scroll: ScrollHandle,
}

impl EventEmitter<SettingsEvent> for SettingsDialog {}

impl Focusable for SettingsDialog {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl SettingsDialog {
    pub fn new(model: Model, theme: Theme, cx: &mut Context<Self>) -> Self {
        let theme = model.preview().unwrap_or(theme);
        let list_scroll = ScrollHandle::new();
        list_scroll.scroll_to_item(model.selected);
        Self { focus_handle: cx.focus_handle(), model, theme, list_scroll, editor_scroll: ScrollHandle::new() }
    }

    pub fn describe(&self) -> String {
        self.model.describe()
    }

    /// After any change: preview the theme the model now shows, and redraw.
    fn changed(&mut self, cx: &mut Context<Self>) {
        if let Some(preview) = self.model.preview()
            && preview != self.theme
        {
            self.theme = preview.clone();
            cx.emit(SettingsEvent::Preview(preview));
        }
        match &self.model.mode {
            Mode::List => self.list_scroll.scroll_to_item(self.model.selected),
            Mode::Editor(editor) => self.editor_scroll.scroll_to_item(editor.focused),
            Mode::ConfirmDelete => {}
        }
        cx.notify();
    }

    fn outcome(&mut self, outcome: Outcome, cx: &mut Context<Self>) {
        match outcome {
            Outcome::Nothing => self.changed(cx),
            Outcome::Written => {
                self.changed(cx);
                cx.emit(SettingsEvent::ConfigWritten);
            }
            Outcome::Applied => {
                cx.emit(SettingsEvent::ConfigWritten);
                cx.emit(SettingsEvent::Close);
            }
        }
    }

    fn up(&mut self, _: &SettingsUp, _: &mut Window, cx: &mut Context<Self>) {
        self.model.move_selection(false);
        self.changed(cx);
    }

    fn down(&mut self, _: &SettingsDown, _: &mut Window, cx: &mut Context<Self>) {
        self.model.move_selection(true);
        self.changed(cx);
    }

    fn next_field(&mut self, _: &SettingsNextField, _: &mut Window, cx: &mut Context<Self>) {
        if !self.model.in_list() {
            self.model.move_selection(true);
            self.changed(cx);
        }
    }

    fn prev_field(&mut self, _: &SettingsPrevField, _: &mut Window, cx: &mut Context<Self>) {
        if !self.model.in_list() {
            self.model.move_selection(false);
            self.changed(cx);
        }
    }

    fn confirm(&mut self, _: &SettingsConfirm, _: &mut Window, cx: &mut Context<Self>) {
        let outcome = self.model.confirm();
        self.outcome(outcome, cx);
    }

    fn cancel(&mut self, _: &SettingsCancel, _: &mut Window, cx: &mut Context<Self>) {
        if self.model.cancel() {
            cx.emit(SettingsEvent::Close);
        } else {
            self.changed(cx);
        }
    }

    fn new_theme(&mut self, _: &SettingsNew, _: &mut Window, cx: &mut Context<Self>) {
        self.model.start_new();
        self.changed(cx);
    }

    fn edit_theme(&mut self, _: &SettingsEdit, _: &mut Window, cx: &mut Context<Self>) {
        self.model.start_edit();
        self.changed(cx);
    }

    fn delete_theme(&mut self, _: &SettingsDelete, _: &mut Window, cx: &mut Context<Self>) {
        self.model.request_delete();
        self.changed(cx);
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.model.paste(&text);
            self.changed(cx);
        }
    }

    fn key_down(&mut self, e: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.model.key(&e.keystroke) {
            cx.stop_propagation();
            self.changed(cx);
        }
    }

    // ---- rendering ----

    fn button(
        &self,
        id: &'static str,
        label: impl Into<SharedString>,
        primary: bool,
        enabled: bool,
        cx: &mut Context<Self>,
        on_click: impl Fn(&mut Self, &mut Window, &mut Context<Self>) + 'static,
    ) -> impl IntoElement {
        let t = &self.theme;
        let panel = t.chrome_background();
        let accent = t.accent();
        div()
            .id(id)
            .px_3()
            .py_1()
            .rounded_md()
            .border_1()
            .border_color(t.chrome_border().hsla())
            .text_color(if enabled { t.foreground.hsla() } else { t.muted_foreground().hsla() })
            .when(primary && enabled, |d| d.bg(panel.mix(accent, 0.3).hsla()).border_color(accent.hsla()))
            .when(enabled, |d| {
                d.hover(|d| d.bg(panel.mix(t.foreground, 0.1).hsla()))
                    .on_click(cx.listener(move |this, _, window, cx| on_click(this, window, cx)))
            })
            .child(label.into())
    }

    fn swatch(&self, color: Option<Color>, size: f32) -> impl IntoElement {
        let t = &self.theme;
        div()
            .flex_none()
            .size(px(size))
            .rounded_sm()
            .border_1()
            .border_color(t.chrome_border().mix(t.foreground, 0.2).hsla())
            .when_some(color, |d, c| d.bg(c.hsla()))
    }

    fn render_list(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = &self.theme;
        let fg = t.foreground.hsla();
        let muted = t.muted_foreground().hsla();
        let panel = t.chrome_background();
        let accent = t.accent();
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
                .when(selected, |d| d.bg(panel.mix(accent, 0.22).hsla()))
                .when(readable, |d| {
                    d.hover(|d| d.bg(panel.mix(t.foreground, 0.08).hsla())).on_click(cx.listener(move |this, e: &gpui::ClickEvent, _, cx| {
                        this.model.select(ix);
                        if e.click_count() >= 2 {
                            let outcome = this.model.confirm();
                            this.outcome(outcome, cx);
                        } else {
                            this.changed(cx);
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
                Err(_) => row.child(div().w(px(84.0)).flex_none().text_xs().text_color(t.ansi[1].hsla()).child("unreadable")),
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
            .when(active, |d| d.child(div().flex_none().text_xs().text_color(accent.hsla()).child("● active")))
        })
        .collect::<Vec<_>>();

        let custom_selected = model.selected_entry().is_some_and(|e| e.custom);
        let confirming = matches!(model.mode, Mode::ConfirmDelete);
        let footer = if confirming {
            let name = model.selected_entry().map(|e| e.name.clone()).unwrap_or_default();
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(div().flex_1().text_color(fg).child(SharedString::from(format!("Delete {name:?}?"))))
                .child(self.button("delete-no", "Keep", false, true, cx, |this, _, cx| {
                    this.model.cancel();
                    this.changed(cx);
                }))
                .child(self.button("delete-yes", "Delete (enter)", true, true, cx, |this, _, cx| {
                    let outcome = this.model.confirm();
                    this.outcome(outcome, cx);
                }))
        } else {
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(self.button("new", "New (n)", false, true, cx, |this, _, cx| {
                    this.model.start_new();
                    this.changed(cx);
                }))
                .child(self.button("edit", "Edit (e)", false, true, cx, |this, _, cx| {
                    this.model.start_edit();
                    this.changed(cx);
                }))
                .child(self.button("delete", "Delete (d)", false, custom_selected, cx, |this, _, cx| {
                    this.model.request_delete();
                    this.changed(cx);
                }))
                .child(div().flex_1())
                .child(self.button("cancel", "Cancel", false, true, cx, |_, _, cx| cx.emit(SettingsEvent::Close)))
                .child(self.button("apply", "Apply", true, true, cx, |this, _, cx| {
                    let outcome = this.model.confirm();
                    this.outcome(outcome, cx);
                }))
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap_2()
            .child(div().text_xs().text_color(muted).child("Pick a theme to preview it; Apply makes it the default."))
            .child(
                div()
                    .id("theme-list")
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.list_scroll)
                    .children(rows),
            )
            .child(footer)
            .into_any_element()
    }

    fn render_field(&self, field: usize, text: &str, focused: bool, invalid: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        let border = if invalid {
            t.ansi[1].hsla()
        } else if focused {
            t.accent().hsla()
        } else {
            t.chrome_border().mix(t.foreground, 0.15).hsla()
        };
        div()
            .id(("field", field))
            .flex()
            .flex_1()
            .min_w_0()
            .items_center()
            .h(px(26.0))
            .px_2()
            .rounded_md()
            .border_1()
            .border_color(border)
            .bg(t.background.hsla())
            .text_color(t.foreground.hsla())
            .on_click(cx.listener(move |this, _, _, cx| {
                this.model.focus_field(field);
                this.changed(cx);
            }))
            .child(SharedString::from(text.to_string()))
            .when(focused, |d| d.child(div().w(px(1.5)).h(px(15.0)).bg(t.accent().hsla())))
    }

    fn render_editor(&self, editor: &Editor, cx: &mut Context<Self>) -> AnyElement {
        let t = &self.theme;
        let muted = t.muted_foreground().hsla();
        let red: Hsla = t.ansi[1].hsla();
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
                .child(self.swatch(color, 18.0))
                .child(self.render_field(field, &editor.colors[ix].text, editor.focused == field, invalid, cx))
                .when(slot.optional(), |d| {
                    d.child(self.button("clear-selection-fg", "Clear", false, color.is_some(), cx, |this, _, cx| {
                        this.model.clear_selection_foreground();
                        this.changed(cx);
                    }))
                })
        })
        .collect::<Vec<_>>();

        let title = match &editor.original {
            Some(name) => format!("Edit {name:?}"),
            None => "New theme".to_string(),
        };
        let can_save = self.model.editor_can_save();
        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap_2()
            .child(div().text_color(t.foreground.hsla()).child(SharedString::from(title)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(label("Name".into()))
                    .child(self.render_field(0, &editor.name.text, editor.focused == 0, name_error.is_some(), cx)),
            )
            .when_some(name_error, |d, e| d.child(div().text_xs().text_color(red).child(SharedString::from(e))))
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
                    .child(div().flex_1().text_xs().text_color(muted).child("tab moves between fields · #rrggbb or #rgb"))
                    .child(self.button("discard", "Discard", false, true, cx, |this, _, cx| {
                        this.model.cancel();
                        this.changed(cx);
                    }))
                    .child(self.button("save", "Save", true, can_save, cx, |this, _, cx| {
                        let outcome = this.model.confirm();
                        this.outcome(outcome, cx);
                    })),
            )
            .into_any_element()
    }
}

impl Render for SettingsDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme.clone();
        let content = match &self.model.mode {
            Mode::Editor(editor) => {
                let editor = editor.clone();
                self.render_editor(&editor, cx)
            }
            _ => self.render_list(cx),
        };
        let panel = t.chrome_background();
        let sections = SECTIONS.iter().enumerate().map(|(ix, name)| {
            div()
                .px_3()
                .py_1p5()
                .rounded_md()
                .when(ix == 0, |d| d.bg(panel.mix(t.accent(), 0.22).hsla()))
                .text_color(t.foreground.hsla())
                .child(*name)
        });
        let in_list = self.model.in_list();
        div()
            .key_context("SettingsDialog")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::next_field))
            .on_action(cx.listener(Self::prev_field))
            .on_action(cx.listener(Self::confirm))
            .on_action(cx.listener(Self::cancel))
            .on_action(cx.listener(Self::new_theme))
            .on_action(cx.listener(Self::edit_theme))
            .on_action(cx.listener(Self::delete_theme))
            .on_action(cx.listener(Self::paste))
            .on_key_down(cx.listener(Self::key_down))
            // A stray click outside shouldn't throw away an edit in progress.
            .when(in_list, |d| d.on_mouse_down_out(cx.listener(|_, _, _, cx| cx.emit(SettingsEvent::Close))))
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .w(px(680.0))
            .h(px(460.0))
            .flex()
            .flex_col()
            .bg(panel.hsla())
            .border_1()
            .border_color(t.chrome_border().hsla())
            .rounded_lg()
            .shadow_lg()
            .text_sm()
            .child(
                div()
                    .flex_none()
                    .px_4()
                    .py_2()
                    .border_b_1()
                    .border_color(t.chrome_border().hsla())
                    .text_color(t.foreground.hsla())
                    .child("Settings"),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_none()
                            .w(px(140.0))
                            .p_2()
                            .gap_1()
                            .border_r_1()
                            .border_color(t.chrome_border().hsla())
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
                                        .bg(t.ansi[1].mix(panel, 0.75).hsla())
                                        .text_color(t.foreground.hsla())
                                        .child(SharedString::from(e)),
                                )
                            }),
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

        fn model(&self, active: &str) -> Model {
            Model::new(self.path.clone(), active.into(), BTreeMap::new())
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

    fn type_text(model: &mut Model, text: &str) {
        for ch in text.chars() {
            let ks = Keystroke { key: ch.to_string(), key_char: Some(ch.to_string()), modifiers: Default::default() };
            assert!(model.key(&ks));
        }
    }

    fn editor(model: &Model) -> &Editor {
        match &model.mode {
            Mode::Editor(e) => e,
            _ => panic!("not editing: {}", model.describe()),
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
        let mut model = Model::new(f.path.clone(), "mine".into(), BTreeMap::from([("mine".into(), Theme::default())]));
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
        assert_eq!(e.name.text, "tokyo-night-custom");
        assert_eq!(e.original, None);
        assert_eq!(e.current.accent, Some(tokyo.ansi[4]));
        assert_eq!(e.current, tokyo.with_effective_colors());
        assert_eq!(model.preview(), Some(tokyo.with_effective_colors()));
        // Edit on a built-in copies it too.
        model.cancel();
        key(&mut model, "e");
        assert_eq!(editor(&model).name.text, "tokyo-night-custom");
    }

    #[test]
    fn color_edits_preview_and_validate() {
        let f = Fixture::new("colors", CONFIG);
        let mut model = f.model("brindle-dark");
        model.start_new();
        // Field 2 is the background.
        model.focus_field(2);
        for _ in 0..7 {
            key(&mut model, "backspace");
        }
        type_text(&mut model, "#000");
        assert_eq!(model.preview().unwrap().background, Color::hex(0));
        assert!(model.editor_can_save());
        // Field 3 is the cursor: an invalid value keeps the previous color.
        let cursor = model.preview().unwrap().cursor;
        model.move_selection(true);
        key(&mut model, "backspace");
        assert_eq!(editor(&model).focused_label(), "Cursor");
        assert_eq!(model.preview().unwrap().cursor, cursor);
        assert!(!model.editor_can_save());
        assert!(model.describe().contains("invalid=[\"Cursor\"]"), "{}", model.describe());
        type_text(&mut model, "x");
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
        model.focus_field(0);
        for _ in 0.."mine".len() {
            key(&mut model, "backspace");
        }
        type_text(&mut model, "spare");
        assert!(!model.editor_can_save());
        for _ in 0.."spare".len() {
            key(&mut model, "backspace");
        }
        type_text(&mut model, "tokyo-night");
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
        model.focus_field(0);
        for _ in 0.."mine".len() {
            key(&mut model, "backspace");
        }
        type_text(&mut model, "ours");
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
        model.focus_field(2);
        key(&mut model, "backspace");
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
        assert!(model.describe().contains("mode=confirm-delete theme=\"spare\""));
        assert!(!model.cancel());
        assert_eq!(f.text(), CONFIG);
        key(&mut model, "d");
        assert_eq!(model.confirm(), Outcome::Written);
        assert!(!f.text().contains("spare"));
        assert!(!names(&model).contains(&"spare"));
        assert_eq!(model.selected_entry().unwrap().name, "mine");
    }
}
