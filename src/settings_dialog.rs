//! The settings dialog: an overlay over the window with a list of sections.
//! Its sections are "Profiles" (list, edit, reorder and delete profiles),
//! "Theme" (pick a theme with a live preview on the tabs behind the dialog,
//! and create, edit, rename or delete custom themes) and "Fonts" (the
//! terminal font's family, size and line height and the chrome's family,
//! previewed as they change). Changes are written to the config file with
//! `config_edit`.
//!
//! [`Model`] holds the state and all the logic and has no GPUI dependency,
//! so it is unit-tested. Each section is a model of its own
//! ([`ThemesState`], [`ProfilesState`], [`FontsState`]), and none of them
//! touches the file: they are given its text, and answer an operation with
//! a [`Step`], which may be a [`ConfigEdit`] to write. [`SettingsDialog`]
//! is the view around the model: it reads the file, writes the edits and
//! reports each write back with [`Model::written`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, ClickEvent, Context, CursorStyle, DismissEvent, Entity, EventEmitter,
    FocusHandle, Focusable, InteractiveElement, IntoElement, KeyDownEvent, Keystroke, MouseButton, ParentElement, Render,
    ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div, px,
};

use brindle_ui::{
    ActiveTheme as _, Button, ButtonSize, ButtonVariant, Combobox, Dialog, Dismiss, FocusRing as _, FocusStops, Input,
    Select, TextInput, TextInputEvent, Tokens,
};

use crate::actions::{
    SettingsConfirm, SettingsDelete, SettingsDown, SettingsEdit, SettingsFirst, SettingsLast, SettingsNew, SettingsUp,
};
use crate::config_edit::{self, Applied, ConfigEdit, FileText};
use crate::theme::{Color, Theme};

mod fonts;
mod profiles;
mod themes;

use crate::config::{FontConfig, Profile, UiFontConfig};
use crate::settings::FontPreview;
use fonts::FontsState;
use profiles::ProfilesState;
use themes::{Editor, FieldId, SLOTS, ThemesState};

/// The dialog's sections, in the order they are listed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Section {
    Profiles,
    Theme,
    Fonts,
}

const SECTIONS: [Section; 3] = [Section::Profiles, Section::Theme, Section::Fonts];

impl Section {
    fn label(self) -> &'static str {
        match self {
            Section::Profiles => "Profiles",
            Section::Theme => "Theme",
            Section::Fonts => "Fonts",
        }
    }
}

// ---- model ------------------------------------------------------------------

/// What a section with a list shows: the list, an editor, or the delete
/// confirmation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListMode {
    List,
    Editor,
    ConfirmDelete,
}

/// What the dialog shows: a section, and its list's mode for the two that
/// have one. The Fonts section is a form, always.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    Profiles(ListMode),
    Theme(ListMode),
    Fonts,
}

impl View {
    pub fn section(self) -> Section {
        match self {
            View::Profiles(_) => Section::Profiles,
            View::Theme(_) => Section::Theme,
            View::Fonts => Section::Fonts,
        }
    }

    pub fn kind(self) -> ModeKind {
        match self {
            View::Profiles(mode) | View::Theme(mode) => match mode {
                ListMode::List => ModeKind::List,
                ListMode::Editor => ModeKind::Editor,
                ListMode::ConfirmDelete => ModeKind::ConfirmDelete,
            },
            View::Fonts => ModeKind::Form,
        }
    }
}

/// What kind of view a section shows: its list, an editor, the delete
/// confirmation, or a form (Fonts: fields always editable, applied
/// together). A change of section or kind moves focus to the new view's
/// first control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModeKind {
    List,
    Editor,
    ConfirmDelete,
    Form,
}

/// What the view does after an operation.
#[derive(Clone, Debug, PartialEq)]
#[allow(
    clippy::large_enum_variant,
    reason = "returned and taken at once by the view, never stored; boxing the edit adds an indirection for nothing"
)]
pub enum Step {
    /// Show the model as it is now.
    Nothing,
    /// Write this edit to the config file, then report it with
    /// [`Model::written`].
    Write(ConfigEdit),
    /// Close the dialog.
    Close,
}

/// A section's answer to an operation, which [`Model`] takes as its step
/// and its error line.
#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::large_enum_variant, reason = "a step on its way to the view, as `Step` is")]
pub enum Reply {
    /// Nothing happened: the error line stays as it is.
    Quiet,
    /// Done, or this step to take: the error line clears.
    Done(Step),
    /// Refused, saying why in the error line.
    Refused(String),
}

impl From<Result<Step, String>> for Reply {
    fn from(result: Result<Step, String>) -> Self {
        match result {
            Ok(step) => Reply::Done(step),
            Err(e) => Reply::Refused(e),
        }
    }
}

/// What a list key (or button) asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListOp {
    New,
    Edit,
    /// Profiles only.
    Duplicate,
    Delete,
}

/// What the view should tell the workspace after a write.
#[derive(Debug, PartialEq)]
pub enum Outcome {
    Nothing,
    /// The config file changed; reload it but keep the dialog open.
    Written,
    /// The config file changed and the dialog is done.
    Applied,
}

/// Where focus goes after the model changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FocusMove {
    Stay,
    /// To the view's first control.
    Default,
}

/// What the view finds about focus when the model changed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FocusFacts {
    /// A select's menu or a family list is open; it holds focus while it is.
    pub menu_open: bool,
    /// The section list has focus.
    pub on_sections: bool,
    /// Focus is on the dialog or one of the view's stops.
    pub valid: bool,
}

/// Where focus goes when the dialog shows `view`, focus having last been
/// placed for `placed_for`: to the new view's first control when the view
/// changed, and off a control the view no longer shows (or shows
/// disabled). An open menu keeps it while the view is the same, and so
/// does the section list while the sections change under it (by key or
/// click: the user is still choosing one), though a view that opens in the
/// same section (an editor, by a list key) takes focus as usual.
pub fn focus_target(placed_for: View, view: View, facts: FocusFacts) -> FocusMove {
    let moved = view != placed_for;
    if !moved && facts.menu_open {
        return FocusMove::Stay;
    }
    let section_changed = view.section() != placed_for.section();
    if facts.on_sections && (section_changed || !moved) {
        return FocusMove::Stay;
    }
    if moved || !facts.valid { FocusMove::Default } else { FocusMove::Stay }
}

pub struct Model {
    /// The config file, which messages name.
    config_path: PathBuf,
    section: Section,
    themes: ThemesState,
    profiles: ProfilesState,
    fonts: FontsState,
    /// The last failure to show (save errors, refused deletes).
    error: Option<String>,
}

impl Model {
    /// The dialog's model, from the config file at `config_path` as it
    /// reads (`text`). The `fallback`s are the config in effect's, shown
    /// read-only when the file can't be read; `active` is its global theme.
    pub fn new(
        config_path: PathBuf,
        text: &FileText,
        active: String,
        fallback: BTreeMap<String, Theme>,
        fallback_profiles: Vec<Profile>,
        fallback_default: Option<String>,
    ) -> Self {
        let themes = ThemesState::new(&config_path, text, active, fallback);
        let profiles = ProfilesState::new(&config_path, text, fallback_profiles, fallback_default);
        let fonts = FontsState::new(text, (FontConfig::default(), UiFontConfig::default()), Vec::new());
        Self { config_path, section: Section::Profiles, themes, profiles, fonts, error: None }
    }

    /// The Fonts section's fonts, from `text`: `fallback`, the loaded
    /// config's, shown read-only when the file can't be parsed, and the
    /// `installed` families (GPUI's names), which decide what a family
    /// field shows and lists.
    pub fn with_fonts(mut self, text: &FileText, fallback: (FontConfig, UiFontConfig), installed: Vec<String>) -> Self {
        self.fonts = FontsState::new(text, fallback, installed);
        self
    }

    /// The config file the dialog edits.
    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    /// Takes a section's reply: the step it asks for, with the error line
    /// cleared, set or left as it is.
    fn reply(&mut self, reply: Reply) -> Step {
        match reply {
            Reply::Quiet => Step::Nothing,
            Reply::Done(step) => {
                self.error = None;
                step
            }
            Reply::Refused(e) => {
                self.error = Some(e);
                Step::Nothing
            }
        }
    }

    /// The theme the tabs should show right now.
    pub fn preview(&self) -> Option<Theme> {
        self.themes.preview()
    }

    pub fn move_selection(&mut self, forward: bool) {
        match self.section {
            // The form's fields are focus stops: the view moves focus.
            Section::Fonts => {}
            Section::Profiles => {
                self.profiles.move_selection(forward);
                self.error = None;
            }
            Section::Theme => {
                let reply = self.themes.move_selection(forward);
                self.reply(reply);
            }
        }
    }

    /// Home and end in a list: selects its first entry, or its `last`
    /// (themes skip the unreadable ones, as up and down do).
    pub fn select_edge(&mut self, last: bool) {
        match self.section {
            Section::Profiles => {
                if let Some(ix) = self.profiles.edge(last) {
                    self.profiles.select(ix);
                    self.error = None;
                }
            }
            Section::Theme => {
                let reply = self.themes.select_edge(last);
                self.reply(reply);
            }
            Section::Fonts => {}
        }
    }

    pub fn select(&mut self, ix: usize) {
        match self.section {
            Section::Fonts => {}
            Section::Profiles => {
                self.profiles.select(ix);
                self.error = None;
            }
            Section::Theme => {
                let reply = self.themes.select(ix);
                self.reply(reply);
            }
        }
    }

    /// Enter: apply in the theme list, edit in the profile list, save in an
    /// editor, delete when confirming, apply the Fonts form.
    pub fn confirm(&mut self) -> Step {
        let reply = match self.section {
            Section::Fonts => self.fonts.apply().into(),
            Section::Profiles => match self.profiles.list_mode() {
                ListMode::List => {
                    let themes = self.themes.theme_names();
                    self.profiles.start_edit(&themes).map(|()| Step::Nothing).into()
                }
                ListMode::Editor => self.profiles.save().into(),
                ListMode::ConfirmDelete => self.profiles.delete_selected().into(),
            },
            Section::Theme => self.themes.confirm(),
        };
        self.reply(reply)
    }

    /// Escape: closes the dialog from a list or the form; otherwise leaves
    /// the editor or the confirmation for the list.
    pub fn cancel(&mut self) -> Step {
        match self.section {
            Section::Fonts => Step::Close,
            Section::Profiles => {
                self.error = None;
                self.profiles.cancel()
            }
            Section::Theme => {
                let reply = self.themes.cancel();
                self.reply(reply)
            }
        }
    }

    /// New: a copy of the selected theme, or a new profile.
    pub fn start_new(&mut self) {
        match self.section {
            Section::Fonts => {}
            Section::Profiles => {
                if self.profiles.list_mode() == ListMode::List {
                    let themes = self.themes.theme_names();
                    let reply = self.profiles.start_new(&themes).map(|()| Step::Nothing).into();
                    self.reply(reply);
                }
            }
            Section::Theme => {
                let reply = self.themes.start_new();
                self.reply(reply);
            }
        }
    }

    /// Edit: the selected theme (a built-in is copied first) or profile.
    pub fn start_edit(&mut self) {
        match self.section {
            Section::Fonts => {}
            Section::Profiles => {
                if self.profiles.list_mode() == ListMode::List {
                    let themes = self.themes.theme_names();
                    let reply = self.profiles.start_edit(&themes).map(|()| Step::Nothing).into();
                    self.reply(reply);
                }
            }
            Section::Theme => {
                let reply = self.themes.start_edit();
                self.reply(reply);
            }
        }
    }

    /// Duplicate (profiles only).
    pub fn start_duplicate(&mut self) {
        if self.section == Section::Profiles && self.profiles.list_mode() == ListMode::List {
            let themes = self.themes.theme_names();
            let reply = self.profiles.start_duplicate(&themes).map(|()| Step::Nothing).into();
            self.reply(reply);
        }
    }

    /// Delete: asks for confirmation, or explains why the theme or profile
    /// can't go. `text` is the config file as it reads now, which tells
    /// what still refers to a theme.
    pub fn request_delete(&mut self, text: &FileText) {
        match self.section {
            Section::Fonts => {}
            Section::Profiles => {
                if self.profiles.list_mode() == ListMode::List {
                    let reply = self.profiles.request_delete().map(|()| Step::Nothing).into();
                    self.reply(reply);
                }
            }
            Section::Theme => {
                let reply = self.themes.request_delete(text);
                self.reply(reply);
            }
        }
    }

    /// The list shortcut `ks` is, in a list. In an editor and the Fonts
    /// form, keys belong to the fields.
    pub fn list_key(&self, ks: &Keystroke) -> Option<ListOp> {
        let plain = !(ks.modifiers.control || ks.modifiers.alt || ks.modifiers.platform);
        if !plain || self.mode_kind() != ModeKind::List {
            return None;
        }
        match ks.key.as_str() {
            "n" => Some(ListOp::New),
            "e" => Some(ListOp::Edit),
            "c" if self.section == Section::Profiles => Some(ListOp::Duplicate),
            "delete" | "d" => Some(ListOp::Delete),
            _ => None,
        }
    }

    /// A theme editor field's text changed in its input.
    pub fn set_field(&mut self, field: FieldId, text: &str) {
        let reply = self.themes.set_field(field, text);
        self.reply(reply);
    }

    /// The theme editor's fields and their texts, in Tab order, when
    /// editing.
    pub fn editor_fields(&self) -> Option<Vec<(FieldId, String)>> {
        self.themes.editor_fields()
    }

    /// Clears the theme editor's selection foreground; returns its field.
    pub fn clear_selection_foreground(&mut self) -> Option<FieldId> {
        self.themes.clear_selection_foreground()
    }

    #[cfg(test)]
    pub fn in_list(&self) -> bool {
        self.mode_kind() == ModeKind::List
    }

    /// What the dialog shows.
    pub fn view(&self) -> View {
        match self.section {
            Section::Theme => View::Theme(self.themes.list_mode()),
            Section::Profiles => View::Profiles(self.profiles.list_mode()),
            Section::Fonts => View::Fonts,
        }
    }

    /// What the current section shows.
    pub fn mode_kind(&self) -> ModeKind {
        self.view().kind()
    }

    pub fn section(&self) -> Section {
        self.section
    }

    /// Switches section, refused while an editor has unsaved edits. Leaving
    /// Theme drops an unapplied preview: its selection goes back to the
    /// active theme. Leaving Fonts drops its unapplied changes; it has no
    /// editor, so it never holds the section. Profiles and Fonts are reread
    /// from `text`, the file as it reads now, to pick up edits made
    /// elsewhere since the dialog opened.
    pub fn select_section(&mut self, section: Section, text: &FileText) -> bool {
        if section == self.section {
            return true;
        }
        let dirty = match self.section {
            Section::Theme => self.themes.dirty(),
            Section::Profiles => self.profiles.editor().is_some_and(|e| e.dirty()),
            Section::Fonts => false,
        };
        if dirty {
            self.error = Some("Save or discard the edits first".into());
            return false;
        }
        match self.section {
            Section::Theme => self.themes.leave(),
            Section::Profiles => {
                if self.profiles.list_mode() != ListMode::List {
                    self.profiles.discard();
                }
            }
            Section::Fonts => self.fonts.reset(),
        }
        match section {
            Section::Profiles => self.profiles.reload(&self.config_path, text, None),
            Section::Fonts => self.fonts.reload(text),
            Section::Theme => {}
        }
        self.section = section;
        self.error = (section == Section::Theme).then(|| self.themes.note().cloned()).flatten();
        true
    }

    /// Home and end in the section list: the first or the `last` section.
    pub fn edge_section(&mut self, last: bool, text: &FileText) -> bool {
        self.select_section(if last { SECTIONS[SECTIONS.len() - 1] } else { SECTIONS[0] }, text)
    }

    /// Cycles through the sections (ctrl-tab).
    pub fn next_section(&mut self, forward: bool, text: &FileText) -> bool {
        let ix = SECTIONS.iter().position(|s| *s == self.section).unwrap_or(0);
        let n = SECTIONS.len();
        self.select_section(SECTIONS[if forward { (ix + 1) % n } else { (ix + n - 1) % n }], text)
    }

    /// Every theme a profile can choose: built-ins, then custom themes.
    #[cfg(test)]
    fn theme_names(&self) -> Vec<String> {
        self.themes.theme_names()
    }

    /// alt-up / alt-down in the profile list.
    pub fn move_profile(&mut self, up: bool) -> Step {
        if self.section != Section::Profiles {
            return Step::Nothing;
        }
        let reply = self.profiles.move_selected(up).into();
        self.reply(reply)
    }

    /// The edit a step asked for was written, giving `result`, and the file
    /// now reads `text`: the section that asked rereads it, and the view
    /// learns what to tell the workspace. A failed write keeps the section
    /// as it was, with the error.
    pub fn written(&mut self, edit: &ConfigEdit, result: Result<Applied, String>, text: &FileText) -> Outcome {
        let applied = match result {
            Ok(applied) => applied,
            Err(e) => {
                self.error = Some(e);
                return Outcome::Nothing;
            }
        };
        self.error = None;
        match edit {
            ConfigEdit::SetGlobalTheme { .. } | ConfigEdit::SetFonts(_) => Outcome::Applied,
            ConfigEdit::SaveTheme { .. } | ConfigEdit::DeleteTheme { .. } => {
                self.themes.written(edit, &self.config_path, text);
                if self.section == Section::Theme {
                    self.error = self.themes.note().cloned();
                }
                Outcome::Written
            }
            ConfigEdit::SaveProfile { .. } | ConfigEdit::DeleteProfile { .. } | ConfigEdit::MoveProfile { .. } => {
                self.profiles.written(edit, applied, &self.config_path, text);
                Outcome::Written
            }
        }
    }

    /// Whether button `b` can be pressed (and focused) in the current view.
    pub fn enabled(&self, b: Btn) -> bool {
        match self.section {
            Section::Theme => self.themes.enabled(b),
            Section::Profiles => self.profiles.enabled(b),
            Section::Fonts => self.fonts.enabled(b),
        }
    }

    /// The theme list's row marked active: the global theme's.
    pub fn active_row(&self) -> Option<usize> {
        self.themes.active_row()
    }

    /// The fonts the window should preview: the Fonts section's valid,
    /// unapplied changes, while it is shown.
    pub fn font_preview(&self) -> Option<FontPreview> {
        if self.section != Section::Fonts {
            return None;
        }
        self.fonts.preview()
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

    /// [`Model::describe_open`] with no family list open.
    #[cfg(test)]
    pub fn describe(&self, field: &str) -> String {
        self.describe_open(Some(field), None)
    }

    /// Debug description for `--dump-screen-after`. `field` names the
    /// focused control, which an editor reports as its `field=` (`""` for
    /// none); `open` is an open family list's field, search and highlighted
    /// option, for the Fonts section's line.
    pub fn describe_open(&self, field: Option<&str>, open: Option<(fonts::FontField, String, Option<String>)>) -> String {
        let field = field.unwrap_or("");
        match self.section {
            Section::Fonts => self.fonts.describe(&self.error, field, open),
            Section::Profiles => self.profiles.describe(&self.error, field),
            Section::Theme => self.themes.describe(&self.error, field),
        }
    }
}

// ---- view -------------------------------------------------------------------

#[allow(
    clippy::large_enum_variant,
    reason = "emitted one at a time to the workspace and consumed at once; never stored"
)]
pub enum SettingsEvent {
    /// Show this theme on the tabs that follow the global theme.
    Preview(Theme),
    /// Show this profile's theme and font size on its tabs (by its saved
    /// name), or stop.
    ProfilePreview(Option<(String, Profile)>),
    /// The config file was written; reload it, with these profile renames
    /// (old, new) applied to open tabs.
    ConfigWritten(Vec<(String, String)>),
    /// Show these fonts in the window instead of the config's, or stop.
    FontPreview(Option<FontPreview>),
}

/// The dialog's buttons, each with its own focus handle. A button the
/// theme and profile views share (New, Save, …) shares its handle too, since
/// only one of them shows at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Btn {
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
fn footer_buttons(view: View) -> &'static [Btn] {
    match view {
        View::Fonts => &[Btn::Cancel, Btn::Apply],
        View::Theme(ListMode::ConfirmDelete) | View::Profiles(ListMode::ConfirmDelete) => {
            &[Btn::Keep, Btn::ConfirmDelete]
        }
        View::Theme(ListMode::Editor) | View::Profiles(ListMode::Editor) => &[Btn::Discard, Btn::Save],
        View::Theme(ListMode::List) => &[Btn::New, Btn::Edit, Btn::Delete, Btn::Cancel, Btn::Apply],
        View::Profiles(ListMode::List) => {
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
    /// The theme editor's inputs, one per field (name first); empty in the
    /// list.
    fields: Vec<(FieldId, Entity<TextInput>)>,
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
    connect_handle: FocusHandle,
    profile_list_scroll: ScrollHandle,
    profile_editor_scroll: ScrollHandle,
    /// The Fonts section's text inputs and family comboboxes; empty unless
    /// it is shown.
    font_inputs: Vec<(fonts::FontField, Entity<TextInput>)>,
    font_choices: Vec<(fonts::FontField, Entity<Combobox>)>,
    _font_subscriptions: Vec<Subscription>,
    /// What the last font preview showed, to tell when it changes.
    font_previewed: Option<FontPreview>,
    /// The view focus was last placed for.
    placed_for: View,
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
        let view = self.model.view();
        match view {
            View::Theme(ListMode::List) | View::Profiles(ListMode::List) => stops.push(self.list_handle.clone()),
            View::Theme(ListMode::Editor) => {
                for (field, input) in &self.fields {
                    stops.push(input.focus_handle(cx));
                    if *field == FieldId::selection_foreground() && self.model.enabled(Btn::Clear) {
                        stops.push(self.button_handle(Btn::Clear));
                    }
                }
            }
            View::Profiles(ListMode::Editor) => stops.extend(self.profile_field_stops(cx)),
            View::Fonts => stops.extend(self.font_field_stops(cx)),
            View::Theme(ListMode::ConfirmDelete) | View::Profiles(ListMode::ConfirmDelete) => {}
        }
        stops.extend(footer_buttons(view).iter().filter(|b| self.model.enabled(**b)).map(|b| self.button_handle(*b)));
        stops.push(self.close_handle.clone());
        stops
    }

    /// The dialog opens with its section's first stop focused (the profile
    /// list), not the section list, as a list or an editor that opens later
    /// takes focus on its first stop.
    fn initial_focus(&self, cx: &App) -> Option<FocusHandle> {
        Some(self.default_focus(cx))
    }
}

impl SettingsDialog {
    pub fn new(model: Model, theme: Theme, _window: &mut Window, cx: &mut Context<Self>) -> Self {
        let theme = model.preview().unwrap_or(theme);
        let list_scroll = ScrollHandle::new();
        if let Some(ix) = model.themes.selected_index() {
            list_scroll.scroll_to_item(ix);
        }
        let placed_for = model.view();
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
            connect_handle: cx.focus_handle(),
            profile_list_scroll: ScrollHandle::new(),
            profile_editor_scroll: ScrollHandle::new(),
            font_inputs: Vec::new(),
            font_choices: Vec::new(),
            _font_subscriptions: Vec::new(),
            font_previewed: None,
            placed_for,
            scrolled_to: None,
        }
    }

    /// The dialog's `--dump-screen-after` line, with the focused control
    /// as an editor's `field=`, and an open family list's search.
    pub fn describe(&self, window: &Window, cx: &App) -> String {
        self.model.describe_open(self.focused_label(window, cx), self.open_font_list(cx))
    }

    /// The editor input that has keyboard focus, or a Fonts field's (an
    /// open family list's search included).
    pub fn focused_input(&self, window: &Window, cx: &App) -> Option<Entity<TextInput>> {
        let mut inputs = self.fields.iter().map(|(_, i)| i).chain(self.profile_inputs.iter().map(|(_, i)| i));
        inputs.find(|f| f.read(cx).is_focused(window)).cloned().or_else(|| self.focused_font_input(window, cx))
    }

    /// The name of the focused control: an editor field's label, a
    /// button's, `list` for a list, `close` for the `X`; `None` for none.
    pub fn focused_label(&self, window: &Window, cx: &App) -> Option<&'static str> {
        if self.sections_handle.is_focused(window) {
            return Some("sections");
        }
        if self.list_handle.is_focused(window) {
            return Some("list");
        }
        if let Some((field, _)) = self.fields.iter().find(|(_, f)| f.read(cx).is_focused(window)) {
            return themes::field_label(*field);
        }
        if let Some((field, _)) = self.profile_inputs.iter().find(|(_, i)| i.read(cx).is_focused(window)) {
            return Some(field.label());
        }
        // A select whose menu is open still counts as the focused field.
        let select_focused = |s: &Entity<Select>| {
            let s = s.read(cx);
            s.is_open() || s.focus_handle(cx).is_focused(window)
        };
        if let Some((field, _)) = self.profile_selects.iter().find(|(_, s)| select_focused(s)) {
            return Some(field.label());
        }
        if let Some(field) = self.focused_font_field(window, cx) {
            return Some(field.label());
        }
        if self.default_handle.is_focused(window) {
            return Some(profiles::Field::Default.label());
        }
        if self.connect_handle.is_focused(window) {
            return Some(profiles::Field::ConnectAtLaunch.label());
        }
        if let Some((b, _)) = self.buttons.iter().find(|(_, h)| h.is_focused(window)) {
            return Some(b.label());
        }
        if self.close_handle.is_focused(window) {
            return Some("close");
        }
        None
    }

    fn button_handle(&self, b: Btn) -> FocusHandle {
        self.buttons.iter().find(|(btn, _)| *btn == b).map(|(_, h)| h.clone()).expect("every button has a handle")
    }

    /// Whether a button or the close `X` has focus.
    fn button_focused(&self, window: &Window) -> bool {
        self.close_handle.is_focused(window) || self.buttons.iter().any(|(_, h)| h.is_focused(window))
    }

    /// The theme editor's input for `field`.
    fn theme_input(&self, field: FieldId) -> Option<&Entity<TextInput>> {
        self.fields.iter().find(|(f, _)| *f == field).map(|(_, i)| i)
    }

    /// The editor's field stops, in order; the first takes focus when the
    /// editor opens. Empty outside an editor.
    fn field_stops(&self, cx: &App) -> Vec<FocusHandle> {
        match self.model.view() {
            View::Theme(ListMode::Editor) => self.fields.iter().map(|(_, f)| f.focus_handle(cx)).collect(),
            View::Profiles(ListMode::Editor) => self.profile_field_stops(cx),
            View::Fonts => self.font_field_stops(cx),
            View::Theme(ListMode::List | ListMode::ConfirmDelete)
            | View::Profiles(ListMode::List | ListMode::ConfirmDelete) => Vec::new(),
        }
    }

    /// Where focus goes when a view opens: a list, an editor's name, the
    /// Fonts section's first field, or (asking to confirm a delete) the
    /// dialog itself.
    fn default_focus(&self, cx: &App) -> FocusHandle {
        match self.model.mode_kind() {
            ModeKind::List => self.list_handle.clone(),
            ModeKind::Editor | ModeKind::ConfirmDelete | ModeKind::Form => {
                self.field_stops(cx).into_iter().next().unwrap_or_else(|| self.focus_handle.clone())
            }
        }
    }

    /// Whether a select's menu or a family list is open; it holds focus
    /// while it is.
    fn menu_open(&self, cx: &App) -> bool {
        self.profile_selects.iter().any(|(_, s)| s.read(cx).is_open()) || self.font_list_open(cx)
    }

    /// Moves focus where [`focus_target`] says, once the model changed.
    fn place_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let view = self.model.view();
        let placed_for = std::mem::replace(&mut self.placed_for, view);
        let stops = self.stops(cx);
        let facts = FocusFacts {
            menu_open: self.menu_open(cx),
            on_sections: self.sections_handle.is_focused(window),
            valid: window.focused(cx).is_some_and(|f| f == self.focus_handle || stops.contains(&f)),
        };
        match focus_target(placed_for, view, facts) {
            FocusMove::Stay => {}
            FocusMove::Default => window.focus(&self.default_focus(cx)),
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
        match self.model.view() {
            View::Theme(ListMode::List) => {
                if let Some(ix) = self.model.themes.selected_index() {
                    self.list_scroll.scroll_to_item(ix);
                }
            }
            View::Profiles(ListMode::List) => {
                if let Some(ix) = self.model.profiles.selected_index() {
                    self.profile_list_scroll.scroll_to_item(ix);
                }
            }
            View::Theme(ListMode::Editor | ListMode::ConfirmDelete)
            | View::Profiles(ListMode::Editor | ListMode::ConfirmDelete)
            | View::Fonts => {}
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

    /// The select behind a profile choice field.
    fn choice_select(&self, field: profiles::Field) -> Option<Entity<Select>> {
        self.profile_selects.iter().find(|(f, _)| *f == field).map(|(_, s)| s.clone())
    }

    /// Option `ix` of a profile choice field was chosen: change the edited
    /// profile.
    fn apply_choice(&mut self, field: profiles::Field, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_profile(window, cx, |e| e.choose(field, ix));
    }

    /// The slot a choice field's select is laid out in. It adds no keys: a
    /// choice changes only through the select's menu (`SelectEvent::Changed`).
    fn choice_slot(&self, field: profiles::Field) -> gpui::Div {
        div().children(self.choice_select(field))
    }

    /// Scrolls an editor's focused field into view when focus moved, by
    /// Tab, a click or the keys.
    fn follow_focus(&mut self, window: &Window, cx: &App) {
        let focused = window.focused(cx);
        if focused == self.scrolled_to {
            return;
        }
        // The name sits above the scrolling color rows.
        if let Some((FieldId::Color(ix), _)) = self.fields.iter().find(|(_, f)| f.read(cx).is_focused(window)) {
            self.editor_scroll.scroll_to_item(*ix);
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
        let fonts = self.model.font_preview();
        if fonts != self.font_previewed {
            self.font_previewed = fonts.clone();
            cx.emit(SettingsEvent::FontPreview(fonts));
        }
    }

    /// Creates the inputs when the editor opens and drops them when it
    /// closes. Focus follows in `place_focus`.
    fn sync_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.sync_profile_fields(window, cx);
        self.sync_font_fields(window, cx);
        let Some(texts) = self.model.editor_fields() else {
            self.fields.clear();
            self._field_subscriptions.clear();
            return;
        };
        if self.fields.is_empty() {
            for (field, text) in texts {
                let input = cx.new(|cx| TextInput::new(&text, cx));
                let changed = cx.subscribe_in(&input, window, move |this, input, _: &TextInputEvent, _, cx| {
                    let text = input.read(cx).text().to_string();
                    this.model.set_field(field, &text);
                    this.preview(cx);
                    cx.notify();
                });
                self._field_subscriptions.push(changed);
                self.fields.push((field, input));
            }
        }
    }

    /// Takes the step an operation returned: shows the model, writes its
    /// edit and reports it back, or closes.
    fn step(&mut self, step: Step, window: &mut Window, cx: &mut Context<Self>) {
        match step {
            Step::Nothing => self.changed(window, cx),
            Step::Write(edit) => {
                let path = self.model.config_path().to_path_buf();
                let result = config_edit::save(&path, |doc| edit.apply(doc));
                let text = config_edit::read_text(&path);
                let outcome = self.model.written(&edit, result, &text);
                self.outcome(outcome, window, cx);
            }
            Step::Close => cx.emit(DismissEvent),
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
        }
    }

    /// Switches section through `switch`, with the config file as it reads
    /// now.
    pub(super) fn switch_section(
        &mut self,
        switch: impl FnOnce(&mut Model, &FileText) -> bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = config_edit::read_text(self.model.config_path());
        switch(&mut self.model, &text);
        self.changed(window, cx);
    }

    /// A list key or button: New, Edit, Duplicate or Delete. A delete
    /// reads the file, to tell what still refers to a theme.
    pub(super) fn list_op(&mut self, op: ListOp, window: &mut Window, cx: &mut Context<Self>) {
        match op {
            ListOp::New => self.model.start_new(),
            ListOp::Edit => self.model.start_edit(),
            ListOp::Duplicate => self.model.start_duplicate(),
            ListOp::Delete => {
                let text = config_edit::read_text(self.model.config_path());
                self.model.request_delete(&text);
            }
        }
        self.changed(window, cx);
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
        if matches!(self.model.mode_kind(), ModeKind::Editor | ModeKind::Form) {
            // Nothing to move: the key goes on, unhandled.
            cx.propagate();
            return;
        }
        self.model.move_selection(forward);
        self.changed(window, cx);
    }

    /// Enter: the section's confirm (apply, edit, save, delete), but
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
        let step = self.model.confirm();
        self.step(step, window, cx);
    }

    /// Escape (through the overlay's `Dismiss`, after an open select menu
    /// took its own) or the close `X`: closes in a list, discards in an
    /// editor. Handling `Dismiss` here keeps the dialog open
    /// unless the model says it is done.
    fn dismiss(&mut self, _: &Dismiss, window: &mut Window, cx: &mut Context<Self>) {
        self.request_cancel(window, cx);
    }

    /// Escape or the close `X`: closes in a list, discards in an editor.
    fn request_cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let step = self.model.cancel();
        self.step(step, window, cx);
    }

    fn new_theme(&mut self, _: &SettingsNew, window: &mut Window, cx: &mut Context<Self>) {
        self.list_op(ListOp::New, window, cx);
    }

    fn edit_theme(&mut self, _: &SettingsEdit, window: &mut Window, cx: &mut Context<Self>) {
        self.list_op(ListOp::Edit, window, cx);
    }

    fn delete_theme(&mut self, _: &SettingsDelete, window: &mut Window, cx: &mut Context<Self>) {
        self.list_op(ListOp::Delete, window, cx);
    }

    fn key_down(&mut self, e: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(op) = self.model.list_key(&e.keystroke) {
            cx.stop_propagation();
            self.list_op(op, window, cx);
        }
    }

    // ---- rendering ----

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
    /// when [`Model::enabled`] says so.
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
            .disabled(!self.model.enabled(b))
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
        let themes = &self.model.themes;
        let (selected_row, active_row) = (themes.selected_index(), self.model.active_row());

        let rows = themes.entries().iter().enumerate().map(|(ix, entry)| {
            let selected = selected_row == Some(ix);
            let active = active_row == Some(ix);
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
                            let step = this.model.confirm();
                            this.step(step, window, cx);
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

        let confirming = themes.list_mode() == ListMode::ConfirmDelete;
        let footer = if confirming {
            let name = themes.selected_entry().map(|e| e.name.clone()).unwrap_or_default();
            self.confirm_footer(format!("Delete {name:?}?"), cx)
        } else {
            div()
                .flex()
                .items_center()
                .gap_2()
                .child(self.button(Btn::New, "New (n)", ButtonVariant::Outline, cx, |this, window, cx| {
                    this.list_op(ListOp::New, window, cx)
                }))
                .child(self.button(Btn::Edit, "Edit (e)", ButtonVariant::Outline, cx, |this, window, cx| {
                    this.list_op(ListOp::Edit, window, cx)
                }))
                .child(self.button(Btn::Delete, "Delete (d)", ButtonVariant::Destructive, cx, |this, window, cx| {
                    this.list_op(ListOp::Delete, window, cx)
                }))
                .child(div().flex_1())
                .child(self.button(Btn::Cancel, "Cancel", ButtonVariant::Outline, cx, |_, _, cx| cx.emit(DismissEvent)))
                .child(self.button(Btn::Apply, "Apply", ButtonVariant::Default, cx, |this, window, cx| {
                    let step = this.model.confirm();
                    this.step(step, window, cx);
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
                let step = this.model.cancel();
                this.step(step, window, cx);
            }))
            .child(self.button(Btn::ConfirmDelete, "Delete (enter)", ButtonVariant::Destructive, cx, |this, window, cx| {
                let step = this.model.confirm();
                this.step(step, window, cx);
            }))
    }

    /// Theme editor field `field`'s input, framed, red when `invalid`.
    fn render_field(&self, field: FieldId, invalid: bool) -> impl IntoElement {
        div()
            .flex_1()
            .min_w_0()
            .children(self.theme_input(field).map(|input| Input::new(input).invalid(invalid)))
    }

    fn render_editor(&self, editor: &Editor, cx: &mut Context<Self>) -> AnyElement {
        let t = *cx.ui_theme();
        let muted = t.muted_foreground;
        let name_error = self.model.themes.name_error(editor);

        let label = |text: SharedString| div().w(px(150.0)).flex_none().text_color(muted).child(text);
        let rows = SLOTS.iter().enumerate().map(|(ix, slot)| {
            let invalid = editor.parse_color(ix).is_err();
            let color = slot.get(&editor.current);
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_2()
                .child(label(slot.label().into()))
                .child(Self::swatch(color, 18.0, &t))
                .child(self.render_field(FieldId::Color(ix), invalid))
                .when(slot.optional(), |d| {
                    d.child(self.button(Btn::Clear, "Clear", ButtonVariant::Outline, cx, |this, window, cx| {
                        if let Some(field) = this.model.clear_selection_foreground()
                            && let Some(input) = this.theme_input(field).cloned()
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

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap_2()
            .child(div().text_color(t.foreground).child(SharedString::from(editor.title())))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(label("Name".into()))
                    .child(self.render_field(FieldId::Name, name_error.is_some())),
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
                let step = this.model.cancel();
                this.step(step, window, cx);
            }))
            .child(self.button(Btn::Save, "Save", ButtonVariant::Default, cx, |this, window, cx| {
                let step = this.model.confirm();
                this.step(step, window, cx);
            }))
    }
}

impl Render for SettingsDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Focus moves by Tab, clicks and keys; scroll an editor after it.
        self.follow_focus(window, cx);
        let t = *cx.ui_theme();
        let content = match self.model.section() {
            Section::Profiles => self.render_profiles(window, cx),
            Section::Theme => match self.model.themes.editor().cloned() {
                Some(editor) => self.render_editor(&editor, cx),
                None => self.render_list(window, cx),
            },
            Section::Fonts => self.render_fonts(cx),
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
                    this.switch_section(|model, text| model.select_section(section, text), window, cx);
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
    use crate::theme::{BUILTIN_THEMES, builtin_theme};

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

    fn path() -> PathBuf {
        PathBuf::from("/c/config.toml")
    }

    /// The config file as the view reads and writes it, held as text: the
    /// tests take each step as the view does, with no file.
    struct File {
        text: Option<String>,
    }

    impl File {
        fn new(text: &str) -> Self {
            Self { text: Some(text.to_string()) }
        }

        fn read(&self) -> FileText {
            Ok(self.text.clone())
        }

        fn text(&self) -> String {
            self.text.clone().unwrap_or_default()
        }

        /// A model as the dialog opens: on the Profiles section.
        fn opened(&self, active: &str) -> Model {
            Model::new(path(), &self.read(), active.into(), BTreeMap::new(), Vec::new(), None)
        }

        /// A model on the Theme section, where the theme tests start.
        fn model(&self, active: &str) -> Model {
            let mut model = self.opened(active);
            self.section(&mut model, Section::Theme);
            model
        }

        fn section(&self, model: &mut Model, section: Section) -> bool {
            model.select_section(section, &self.read())
        }

        fn next(&self, model: &mut Model, forward: bool) -> bool {
            model.next_section(forward, &self.read())
        }

        fn edge(&self, model: &mut Model, last: bool) -> bool {
            model.edge_section(last, &self.read())
        }

        /// Takes `step` as the view does: writes its edit, then reports it.
        /// A step that closes the dialog isn't one to take here: the tests
        /// that expect one check the step itself.
        fn take(&mut self, model: &mut Model, step: Step) -> Outcome {
            match step {
                Step::Nothing => Outcome::Nothing,
                Step::Close => panic!("the dialog closed"),
                Step::Write(edit) => {
                    let result = config_edit::edited(self.text.as_deref(), |doc| edit.apply(doc));
                    let result = result.map(|(text, applied)| {
                        self.text = Some(text);
                        applied
                    });
                    model.written(&edit, result, &self.read())
                }
            }
        }

        fn confirm(&mut self, model: &mut Model) -> Outcome {
            let step = model.confirm();
            self.take(model, step)
        }

        fn move_profile(&mut self, model: &mut Model, up: bool) -> Outcome {
            let step = model.move_profile(up);
            self.take(model, step)
        }

        /// Presses each of `keys`, which must be list shortcuts, as the
        /// view runs them.
        fn key(&self, model: &mut Model, keys: &str) {
            for k in keys.split(' ') {
                let op = model.list_key(&Keystroke::parse(k).unwrap());
                match op.unwrap_or_else(|| panic!("{k} not handled")) {
                    ListOp::New => model.start_new(),
                    ListOp::Edit => model.start_edit(),
                    ListOp::Duplicate => model.start_duplicate(),
                    ListOp::Delete => model.request_delete(&self.read()),
                }
            }
        }
    }

    /// A second profile after Shell, for the profile operations.
    fn two_profiles() -> String {
        CONFIG.replace("name = \"Shell\"\n", "name = \"Shell\"\n\n[[profiles]]\nname = \"Work\"\n")
    }

    fn selected_theme(model: &Model) -> &str {
        &model.themes.selected_entry().unwrap().name
    }

    fn names(model: &Model) -> Vec<&str> {
        model.themes.entries().iter().map(|e| e.name.as_str()).collect()
    }

    fn editor(model: &Model) -> &Editor {
        model.themes.editor().unwrap_or_else(|| panic!("not editing: {}", model.describe("")))
    }

    /// The section list's keys: down and up step and wrap (Radix's `loop`),
    /// home and end go to the ends.
    #[test]
    fn section_keys_step_wrap_and_reach_the_ends() {
        let f = File::new(CONFIG);
        let mut model = f.opened("mine");
        assert_eq!(model.section(), Section::Profiles);
        assert!(f.next(&mut model, true));
        assert_eq!(model.section(), Section::Theme);
        assert!(f.next(&mut model, true));
        assert_eq!(model.section(), Section::Fonts);
        assert!(f.next(&mut model, true));
        assert_eq!(model.section(), Section::Profiles);
        assert!(f.next(&mut model, false));
        assert_eq!(model.section(), Section::Fonts);
        assert!(f.edge(&mut model, false));
        assert_eq!(model.section(), Section::Profiles);
        assert!(f.edge(&mut model, true));
        assert_eq!(model.section(), Section::Fonts);
    }

    #[test]
    fn apply_writes_the_global_theme() {
        let mut f = File::new(CONFIG);
        let mut model = f.model("mine");
        model.move_selection(true);
        assert_eq!(f.confirm(&mut model), Outcome::Applied);
        assert_eq!(f.text(), CONFIG.replace("theme = \"mine\"", "theme = \"spare\""));
    }

    /// The Theme section's error line shows the note once the section is
    /// selected, and drops it on leaving.
    #[test]
    fn the_theme_section_shows_why_its_themes_couldnt_be_read() {
        let f = File::new("themes = 3\n");
        let mut model = f.opened("brindle-dark");
        assert_eq!(model.error, None, "the Profiles section is shown first");
        f.section(&mut model, Section::Theme);
        let error = model.error.clone().unwrap();
        assert!(error.starts_with("themes in ") && error.ends_with(" is not a table"), "{error}");
        assert!(model.describe_open(None, None).contains("is not a table"));
        f.section(&mut model, Section::Profiles);
        assert_eq!(model.error, None);
    }

    #[test]
    fn apply_on_a_malformed_file_keeps_the_dialog() {
        let mut f = File::new("theme = \"mine\"\n[oops\n");
        let mut model = Model::new(
            path(),
            &f.read(),
            "mine".into(),
            BTreeMap::from([("mine".into(), Theme::default())]),
            Vec::new(),
            None,
        );
        f.section(&mut model, Section::Theme);
        // The fallback themes are still listed.
        assert_eq!(selected_theme(&model), "mine");
        model.move_selection(false);
        assert_eq!(f.confirm(&mut model), Outcome::Nothing);
        assert!(model.error.as_deref().unwrap().contains("not valid TOML"));
        assert!(model.in_list());
        assert_eq!(selected_theme(&model), "solarized-dark");
        assert_eq!(f.text(), "theme = \"mine\"\n[oops\n");
    }

    #[test]
    fn cancel_closes_from_the_list() {
        let f = File::new(CONFIG);
        let mut model = f.model("mine");
        model.move_selection(true);
        assert_eq!(model.cancel(), Step::Close);
        assert_eq!(f.text(), CONFIG);
    }

    /// A theme saved and a theme deleted are written, and the error line
    /// shows the section's note again, if it has one.
    #[test]
    fn theme_writes_reread_the_list() {
        let mut f = File::new(CONFIG);
        let mut model = f.model("mine");
        model.start_edit();
        model.set_field(FieldId::Name, "ours");
        assert_eq!(f.confirm(&mut model), Outcome::Written);
        assert!(model.in_list());
        assert_eq!(selected_theme(&model), "ours");
        assert!(f.text().contains("theme = \"ours\""), "{}", f.text());
        // A refused write keeps the editor, with the error.
        model.start_edit();
        let step = model.confirm();
        let Step::Write(edit) = step else { panic!("a save") };
        assert_eq!(model.written(&edit, Err("writing: denied".into()), &f.read()), Outcome::Nothing);
        assert_eq!((model.mode_kind(), model.error.as_deref()), (ModeKind::Editor, Some("writing: denied")));
    }

    /// Escape (the overlay's dismiss request) per section and mode: it
    /// closes the dialog from a list, and otherwise leaves the editor or the
    /// confirmation for the list, keeping the dialog.
    #[test]
    fn escape_outcome_per_mode() {
        // A second profile, so one can be asked about deleting.
        let config = two_profiles();
        let f = File::new(&config);
        let mut model = f.opened("mine");
        assert_eq!(model.mode_kind(), ModeKind::List);

        f.section(&mut model, Section::Theme);
        assert_eq!(model.cancel(), Step::Close, "the theme list closes");
        model.start_edit();
        model.set_field(FieldId::Color(1), "#123456");
        assert_eq!(model.mode_kind(), ModeKind::Editor);
        assert_eq!(model.cancel(), Step::Nothing, "the theme editor discards");
        assert_eq!(model.mode_kind(), ModeKind::List);
        model.select(7);
        f.key(&mut model, "d");
        assert_eq!(model.mode_kind(), ModeKind::ConfirmDelete);
        assert_eq!(model.cancel(), Step::Nothing, "the theme delete question goes back to the list");
        assert_eq!(model.mode_kind(), ModeKind::List);

        f.section(&mut model, Section::Profiles);
        assert_eq!(model.cancel(), Step::Close, "the profile list closes");
        f.key(&mut model, "e");
        assert_eq!(model.mode_kind(), ModeKind::Editor);
        assert_eq!(model.cancel(), Step::Nothing, "the profile editor discards");
        assert_eq!(model.mode_kind(), ModeKind::List);
        f.key(&mut model, "d");
        assert_eq!(model.mode_kind(), ModeKind::ConfirmDelete);
        assert_eq!(model.cancel(), Step::Nothing, "the profile delete question goes back to the list");
        assert_eq!(model.mode_kind(), ModeKind::List);

        f.section(&mut model, Section::Fonts);
        assert_eq!(model.cancel(), Step::Close, "the Fonts form closes");
        // Nothing was written.
        assert_eq!(f.text(), config);
    }

    /// The list shortcuts, per section and mode: Duplicate is the profile
    /// list's only; the editor and the form keep their keys, as do
    /// modified keys.
    #[test]
    fn list_keys_per_view() {
        let f = File::new(&two_profiles());
        let mut model = f.opened("mine");
        let key = |model: &Model, k: &str| model.list_key(&Keystroke::parse(k).unwrap());
        assert_eq!(key(&model, "n"), Some(ListOp::New));
        assert_eq!(key(&model, "e"), Some(ListOp::Edit));
        assert_eq!(key(&model, "c"), Some(ListOp::Duplicate));
        assert_eq!(key(&model, "d"), Some(ListOp::Delete));
        assert_eq!(key(&model, "delete"), Some(ListOp::Delete));
        assert_eq!(key(&model, "ctrl-n"), None);
        assert_eq!(key(&model, "x"), None);
        f.section(&mut model, Section::Theme);
        assert_eq!(key(&model, "c"), None);
        assert_eq!(key(&model, "alt-e"), None);
        assert_eq!(key(&model, "e"), Some(ListOp::Edit));
        model.start_edit();
        assert_eq!(key(&model, "n"), None, "typing belongs to the editor's inputs");
        model.cancel();
        f.section(&mut model, Section::Fonts);
        assert_eq!(key(&model, "n"), None);
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
        assert_eq!(
            footer_buttons(View::Theme(ListMode::List)),
            [Btn::New, Btn::Edit, Btn::Delete, Btn::Cancel, Btn::Apply]
        );
        for view in [View::Theme, View::Profiles] {
            assert_eq!(footer_buttons(view(ListMode::Editor)), [Btn::Discard, Btn::Save]);
            assert_eq!(footer_buttons(view(ListMode::ConfirmDelete)), [Btn::Keep, Btn::ConfirmDelete]);
        }
        assert_eq!(footer_buttons(View::Profiles(ListMode::List)).last(), Some(&Btn::Close));
        assert_eq!(footer_buttons(View::Fonts), [Btn::Cancel, Btn::Apply]);
        // Ids are unique, so the shared handles never meet in one view.
        let ids: std::collections::HashSet<_> = BUTTONS.iter().map(|b| b.id()).collect();
        assert_eq!(ids.len(), BUTTONS.len());
        assert_eq!(Btn::Discard.label(), "Discard");
    }

    /// Which buttons can be pressed, per section: the theme list's Delete
    /// only on a custom theme, Clear and Save as the editor allows, the
    /// profile list's by the selection and the file, and the Fonts form's
    /// Apply while its fields are valid.
    #[test]
    fn buttons_are_enabled_per_view() {
        let mut f = File::new(&two_profiles());
        let mut model = f.opened("mine");
        // Shell, the first of two, is selected.
        assert!(model.enabled(Btn::New) && model.enabled(Btn::Edit) && model.enabled(Btn::Delete));
        assert!(!model.enabled(Btn::Up) && model.enabled(Btn::Down));
        model.select(1);
        assert!(model.enabled(Btn::Up) && !model.enabled(Btn::Down));
        model.start_edit();
        assert!(model.enabled(Btn::Save));
        model.profiles.editor_mut().unwrap().set_text(profiles::Field::Name, "");
        assert!(!model.enabled(Btn::Save));
        model.cancel();
        let read_only = File::new("[oops\n").opened("mine");
        for b in [Btn::New, Btn::Duplicate, Btn::Edit, Btn::Delete, Btn::Up, Btn::Down] {
            assert!(!read_only.enabled(b), "{b:?}");
        }
        assert!(read_only.enabled(Btn::Close));

        f.section(&mut model, Section::Theme);
        assert!(model.enabled(Btn::Delete), "mine is custom");
        model.select(0);
        assert!(!model.enabled(Btn::Delete), "a built-in");
        model.start_new();
        assert!(model.enabled(Btn::Save) && !model.enabled(Btn::Clear));
        model.set_field(FieldId::selection_foreground(), "#abcdef");
        assert!(model.enabled(Btn::Clear));
        model.cancel();

        f.section(&mut model, Section::Fonts);
        assert!(model.enabled(Btn::Apply) && model.enabled(Btn::Cancel));
        model.fonts.set_text(fonts::FontField::Size, "200");
        assert!(!model.enabled(Btn::Apply));
        assert_eq!(f.confirm(&mut model), Outcome::Nothing);
    }

    #[test]
    fn the_active_row_is_the_global_themes() {
        let f = File::new(CONFIG);
        let model = f.model("mine");
        let active = model.active_row().unwrap();
        assert_eq!(model.themes.entries()[active].name, "mine");
        assert_eq!(f.model("broken").active_row(), None, "an unreadable theme isn't marked");
        assert_eq!(f.model("nope").active_row(), None);
    }

    /// Focus moves to the new view's first control when the view changes,
    /// and off a control that is gone; an open menu holds it, and so does
    /// the section list while the sections change under it.
    #[test]
    fn focus_follows_the_view() {
        let facts = |menu_open, on_sections, valid| FocusFacts { menu_open, on_sections, valid };
        let list = View::Theme(ListMode::List);
        let editor = View::Theme(ListMode::Editor);
        let profiles = View::Profiles(ListMode::List);
        assert_eq!(focus_target(list, list, facts(false, false, true)), FocusMove::Stay);
        assert_eq!(focus_target(list, list, facts(false, false, false)), FocusMove::Default, "off a stop that went");
        assert_eq!(focus_target(list, editor, facts(false, false, true)), FocusMove::Default, "an editor opened");
        assert_eq!(focus_target(list, list, facts(true, false, false)), FocusMove::Stay, "an open menu holds it");
        assert_eq!(focus_target(list, editor, facts(true, false, true)), FocusMove::Default, "unless the view changed");
        // On the section list.
        assert_eq!(focus_target(list, profiles, facts(false, true, true)), FocusMove::Stay, "choosing a section");
        assert_eq!(focus_target(list, list, facts(false, true, false)), FocusMove::Stay);
        assert_eq!(focus_target(list, editor, facts(false, true, true)), FocusMove::Default, "a list key opened an editor");
        assert_eq!(View::Fonts.kind(), ModeKind::Form);
        assert_eq!((editor.section(), editor.kind()), (Section::Theme, ModeKind::Editor));
    }

    #[test]
    fn switching_sections() {
        let f = File::new(CONFIG);
        let mut model = f.model("mine");
        assert_eq!(model.section(), Section::Theme);
        // An unapplied preview is dropped on leaving Theme.
        model.move_selection(true);
        assert_eq!(selected_theme(&model), "spare");
        assert!(f.next(&mut model, false));
        assert_eq!(model.section(), Section::Profiles);
        assert_eq!(selected_theme(&model), "mine");
        assert!(model.describe("").starts_with("--- settings section=Profiles mode=list selected=\"Shell\""), "{}", model.describe(""));
        // Escape in the profile list closes the dialog.
        assert_eq!(model.cancel(), Step::Close);
        // A dirty profile editor holds the section; a clean one doesn't.
        f.key(&mut model, "e");
        assert!(!model.in_list());
        assert!(f.section(&mut model, Section::Theme));
        assert!(model.in_list());
        f.section(&mut model, Section::Profiles);
        f.key(&mut model, "e");
        model.profiles.editor_mut().unwrap().set_text(profiles::Field::Cwd, "~/src");
        assert!(!f.section(&mut model, Section::Theme));
        assert_eq!(model.section(), Section::Profiles);
        assert!(model.error.as_deref().unwrap().contains("Save or discard"));
        // The same for the theme editor.
        model.cancel();
        f.section(&mut model, Section::Theme);
        model.start_edit();
        model.set_field(FieldId::Color(1), "#123456");
        assert!(!f.next(&mut model, true));
        assert_eq!(f.text(), CONFIG);
    }

    /// Entering the Profiles section rereads the file, and keeps the
    /// selected profile by its key; one that is gone gives way to the one
    /// in its place.
    #[test]
    fn entering_profiles_keeps_the_selection_by_key() {
        let mut f = File::new(&two_profiles());
        let mut model = f.opened("mine");
        model.select(1);
        f.section(&mut model, Section::Theme);
        // A profile added before Work, elsewhere.
        f.text = Some(two_profiles().replace("name = \"Shell\"\n", "name = \"Shell\"\n\n[[profiles]]\nname = \"New\"\n"));
        f.section(&mut model, Section::Profiles);
        assert_eq!(model.profiles.selected_entry().unwrap().name, "Work");
        assert_eq!(model.profiles.selected_index(), Some(2));
        // Work renamed elsewhere: the one in its place.
        f.section(&mut model, Section::Theme);
        f.text = Some(f.text().replace("name = \"Work\"", "name = \"Job\""));
        f.section(&mut model, Section::Profiles);
        assert_eq!(model.profiles.selected_entry().unwrap().name, "Job");
        // Gone, and the list shorter: the last.
        f.section(&mut model, Section::Theme);
        f.text = Some(CONFIG.to_string());
        f.section(&mut model, Section::Profiles);
        assert_eq!(model.profiles.selected_entry().unwrap().name, "Shell");
    }

    #[test]
    fn opens_on_profiles_and_sections_wrap() {
        let f = File::new(CONFIG);
        let mut model = f.opened("mine");
        assert_eq!(model.section(), Section::Profiles);
        assert_eq!(SECTIONS.map(Section::label), ["Profiles", "Theme", "Fonts"]);
        assert!(model.in_list());
        assert!(f.next(&mut model, true));
        assert_eq!(model.section(), Section::Theme);
        // The theme list shows the active theme.
        assert_eq!(selected_theme(&model), "mine");
        assert!(f.next(&mut model, true));
        assert_eq!(model.section(), Section::Fonts);
        assert_eq!(model.mode_kind(), ModeKind::Form);
        assert!(f.next(&mut model, true));
        assert_eq!(model.section(), Section::Profiles);
        assert!(f.next(&mut model, false));
        assert_eq!(model.section(), Section::Fonts);
        assert_eq!(f.text(), CONFIG);
    }

    fn fonts_model(f: &File) -> Model {
        let installed = ["Cascadia Mono", "Consolas", "Inter"].map(String::from).to_vec();
        let mut model =
            f.opened("mine").with_fonts(&f.read(), (FontConfig::default(), UiFontConfig::default()), installed);
        assert!(f.section(&mut model, Section::Fonts));
        model
    }

    /// The spec's "Leave Fonts unapplied": the preview ends and the fields
    /// go back to the file's values.
    #[test]
    fn leaving_fonts_drops_the_changes() {
        let f = File::new("[font]\nsize = 13\n");
        let mut model = fonts_model(&f);
        assert_eq!(model.font_preview(), None);
        model.fonts.set_text(fonts::FontField::Size, "18");
        assert_eq!(model.font_preview(), Some(FontPreview { size: Some(18.0), ..Default::default() }));
        assert!(model.describe("Size").starts_with("--- settings section=Fonts mode=form "), "{}", model.describe(""));
        // Nothing holds the section.
        assert!(f.section(&mut model, Section::Theme));
        assert_eq!(model.font_preview(), None);
        assert!(model.in_list());
        assert!(f.section(&mut model, Section::Fonts));
        assert_eq!(model.fonts.size, "13");
        assert_eq!(model.font_preview(), None);
        assert_eq!(f.text(), "[font]\nsize = 13\n");
        // Escape closes the dialog; the view ends the preview.
        assert_eq!(model.cancel(), Step::Close);
    }

    /// Fonts are reread from the file each time the section is shown.
    #[test]
    fn entering_fonts_rereads_the_file() {
        let mut f = File::new("[font]\nsize = 13\n");
        let mut model = fonts_model(&f);
        f.section(&mut model, Section::Theme);
        f.text = Some("[font]\nsize = 15\n[ui_font]\nfamily = [\"Inter\"]\n".into());
        f.section(&mut model, Section::Fonts);
        assert_eq!(model.fonts.size, "15");
        assert_eq!(model.fonts.ui, crate::settings::FamilyChoice::Family("Inter".into()));
    }

    /// Apply writes and closes; Apply with nothing changed closes without
    /// writing; an invalid field keeps the dialog with the error.
    #[test]
    fn applying_fonts() {
        let mut f = File::new("# mine\n[font]\nsize = 13\n");
        let mut model = fonts_model(&f);
        assert_eq!(model.confirm(), Step::Close);
        assert_eq!(f.text(), "# mine\n[font]\nsize = 13\n");
        model.fonts.set_text(fonts::FontField::Size, "200");
        assert_eq!(f.confirm(&mut model), Outcome::Nothing);
        assert!(model.error.is_some());
        assert_eq!(f.text(), "# mine\n[font]\nsize = 13\n");
        model.fonts.set_text(fonts::FontField::Size, "20");
        assert_eq!(f.confirm(&mut model), Outcome::Applied);
        assert_eq!(model.error, None);
        assert_eq!(f.text(), "# mine\n[font]\nsize = 20\n");
        // The list keys do nothing in the form.
        let before = model.describe("");
        model.move_selection(true);
        model.select_edge(true);
        assert_eq!(model.list_key(&Keystroke::parse("n").unwrap()), None);
        model.start_new();
        model.start_edit();
        model.request_delete(&f.read());
        assert_eq!(model.describe(""), before);
        assert_eq!(model.mode_kind(), ModeKind::Form);
    }

    #[test]
    fn duplicating_opens_a_copy_only_in_the_profile_list() {
        let f = File::new(&two_profiles());
        let mut model = f.opened("mine");
        model.start_duplicate();
        assert_eq!(model.mode_kind(), ModeKind::Editor);
        assert_eq!(model.profiles.editor().unwrap().text(profiles::Field::Name), "Shell copy");
        // Not from the editor, nor from another section.
        model.start_duplicate();
        assert_eq!(model.profiles.editor().unwrap().text(profiles::Field::Name), "Shell copy");
        model.cancel();
        f.section(&mut model, Section::Theme);
        model.start_duplicate();
        assert!(model.in_list());
        assert_eq!(f.text(), two_profiles());
    }

    #[test]
    fn moving_a_profile_writes_it_one_place_on() {
        let mut f = File::new(&two_profiles());
        let mut model = f.opened("mine");
        // Shell, the first, is selected: up has nowhere to go.
        assert_eq!(model.move_profile(true), Step::Nothing);
        assert_eq!(
            model.move_profile(false),
            Step::Write(ConfigEdit::MoveProfile {
                at: config_edit::ProfileRef { index: 0, name: Some("Shell".into()) },
                up: false
            })
        );
        assert_eq!(f.move_profile(&mut model, false), Outcome::Written);
        assert_eq!(model.profiles.selected_entry().unwrap().name, "Shell");
        let config = crate::config::Config::parse(&f.text().replace("background = \"nope\"", "")).unwrap().config;
        assert_eq!(config.profiles.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Work", "Shell"]);
        // Only in the Profiles section.
        f.section(&mut model, Section::Theme);
        assert_eq!(model.move_profile(true), Step::Nothing);
    }

    /// The themes a profile can choose: the readable ones, built-ins first,
    /// each once.
    #[test]
    fn theme_names_are_the_readable_themes() {
        let f = File::new(CONFIG);
        let model = f.opened("mine");
        let builtins: Vec<String> = BUILTIN_THEMES.iter().map(|t| t.to_string()).collect();
        assert_eq!(model.theme_names(), [builtins.clone(), vec!["mine".to_string(), "spare".to_string()]].concat());
        // A custom theme named like a built-in is listed once.
        let f = File::new("[themes.tokyo-night]\nbackground = \"#000000\"\n");
        assert_eq!(f.opened("mine").theme_names(), builtins);
    }

    #[test]
    fn renames_are_taken_once() {
        let mut f = File::new(&two_profiles());
        let mut model = f.opened("mine");
        assert!(model.take_renames().is_empty());
        f.key(&mut model, "e");
        model.profiles.editor_mut().unwrap().set_text(profiles::Field::Name, "Main");
        assert_eq!(f.confirm(&mut model), Outcome::Written);
        assert_eq!(model.take_renames(), [("Shell".to_string(), "Main".to_string())]);
        assert!(model.take_renames().is_empty());
    }

    /// The profile preview shows only while the Profiles section edits a
    /// saved profile.
    #[test]
    fn the_profile_preview_is_the_profile_sections() {
        let f = File::new(&two_profiles());
        let mut model = f.opened("mine");
        assert!(model.profile_preview().is_none());
        f.key(&mut model, "e");
        model.profiles.editor_mut().unwrap().set_text(profiles::Field::FontSize, "18");
        let (name, profile) = model.profile_preview().unwrap();
        assert_eq!((name.as_str(), profile.font_size), ("Shell", Some(18.0)));
        model.cancel();
        assert!(model.profile_preview().is_none());
        f.section(&mut model, Section::Theme);
        assert!(model.profile_preview().is_none());
    }

    /// What each write tells the view: a theme or profile edit keeps the
    /// dialog, the global theme and the fonts close it.
    #[test]
    fn writes_keep_or_close_the_dialog() {
        let mut f = File::new(&two_profiles());
        let mut model = f.opened("mine");
        f.key(&mut model, "d");
        assert_eq!(f.confirm(&mut model), Outcome::Written, "a profile deleted");
        assert_eq!(model.profiles.entries().len(), 1);
        f.section(&mut model, Section::Theme);
        model.select(7);
        f.key(&mut model, "d");
        assert_eq!(f.confirm(&mut model), Outcome::Written, "a theme deleted");
        assert_eq!(selected_theme(&model), "mine", "the one in its place, the last readable");
        model.select(0);
        assert_eq!(f.confirm(&mut model), Outcome::Applied, "the global theme set");
        f.section(&mut model, Section::Fonts);
        model.fonts.set_text(fonts::FontField::Size, "20");
        assert_eq!(f.confirm(&mut model), Outcome::Applied, "the fonts set");
        assert!(f.text().contains("theme = \"brindle-dark\"") && f.text().contains("size = 20"), "{}", f.text());
    }

    #[test]
    fn list_order_and_initial_selection() {
        let f = File::new(CONFIG);
        let model = f.model("mine");
        assert_eq!(
            names(&model),
            ["brindle-dark", "brindle-light", "tokyo-night", "gruvbox-dark", "solarized-dark", "broken", "mine", "spare"]
        );
        assert_eq!(selected_theme(&model), "mine");
        assert!(model.themes.entries()[5].theme.is_err());
        // Unknown or unreadable active themes fall back to brindle-dark.
        assert_eq!(selected_theme(&f.model("broken")), "brindle-dark");
        assert_eq!(selected_theme(&f.model("nope")), "brindle-dark");
    }

    #[test]
    fn moving_skips_unreadable_themes() {
        let f = File::new(CONFIG);
        let mut model = f.model("solarized-dark");
        model.move_selection(true);
        assert_eq!(selected_theme(&model), "mine");
        model.move_selection(false);
        assert_eq!(selected_theme(&model), "solarized-dark");
        model.select(5);
        assert_eq!(selected_theme(&model), "solarized-dark");
        assert_eq!(model.preview(), builtin_theme("solarized-dark"));
    }

    #[test]
    fn home_and_end_select_the_lists_edges() {
        let f = File::new(CONFIG);
        let mut model = f.model("mine");
        model.select_edge(true);
        assert_eq!(selected_theme(&model), "spare");
        model.select_edge(false);
        assert_eq!(selected_theme(&model), "brindle-dark");
    }

    #[test]
    fn new_copies_the_selected_theme() {
        let f = File::new(CONFIG);
        let mut model = f.model("tokyo-night");
        f.key(&mut model, "n");
        let tokyo = builtin_theme("tokyo-night").unwrap();
        let e = editor(&model);
        assert_eq!(e.name, "tokyo-night-custom");
        assert_eq!(e.original, None);
        assert_eq!(e.current.accent, Some(tokyo.ansi[4]));
        assert_eq!(e.current, tokyo.with_effective_colors());
        assert_eq!(model.preview(), Some(tokyo.with_effective_colors()));
        // Edit on a built-in doesn't copy it: New does.
        model.cancel();
        f.key(&mut model, "e");
        assert!(model.in_list());
    }

    #[test]
    fn color_edits_preview_and_validate() {
        let mut f = File::new(CONFIG);
        let mut model = f.model("brindle-dark");
        model.start_new();
        // Color 1 is the background.
        model.set_field(FieldId::Color(1), "#000");
        assert_eq!(model.preview().unwrap().background, Color::hex(0));
        assert!(model.themes.editor_can_save());
        // Color 2 is the cursor: an invalid value keeps the previous color.
        let cursor = model.preview().unwrap().cursor;
        assert_eq!(themes::field_label(FieldId::Color(2)), Some("Cursor"));
        let (field, text) = model.editor_fields().unwrap()[3].clone();
        assert_eq!(field, FieldId::Color(2));
        model.set_field(field, &text[..text.len() - 1]);
        assert_eq!(model.preview().unwrap().cursor, cursor);
        assert!(!model.themes.editor_can_save());
        let line = model.describe("Cursor");
        assert!(line.contains("field=\"Cursor\" invalid=[\"Cursor\"]"), "{line}");
        model.set_field(field, &format!("{}x", &text[..text.len() - 1]));
        assert_eq!(f.confirm(&mut model), Outcome::Nothing);
        assert!(model.error.is_some());
        assert_eq!(f.text(), CONFIG);
    }

    #[test]
    fn name_rules() {
        let mut f = File::new(CONFIG);
        let mut model = f.model("mine");
        model.start_edit();
        // Keeping its own name is fine.
        assert!(model.themes.editor_can_save());
        model.set_field(FieldId::Name, "spare");
        assert!(!model.themes.editor_can_save());
        model.set_field(FieldId::Name, "tokyo-night");
        assert!(!model.themes.editor_can_save());
        assert_eq!(f.confirm(&mut model), Outcome::Nothing);
        assert!(model.error.as_deref().unwrap().contains("built-in"));
    }

    #[test]
    fn saving_a_new_theme_keeps_the_global_theme() {
        let mut f = File::new(CONFIG);
        let mut model = f.model("mine");
        model.move_selection(true); // spare
        model.start_new();
        assert_eq!(f.confirm(&mut model), Outcome::Written);
        assert!(model.in_list());
        assert_eq!(selected_theme(&model), "spare-custom");
        let config = crate::config::Config::parse(&f.text().replace("background = \"nope\"", "")).unwrap().config;
        assert_eq!(config.theme, "mine");
        assert_eq!(config.themes["spare-custom"].background, Color::hex(0x111111));
        assert_eq!(model.preview().unwrap().background, Color::hex(0x111111));
    }

    #[test]
    fn renaming_the_active_theme() {
        let mut f = File::new(&CONFIG.replace("name = \"Shell\"", "name = \"Shell\"\ntheme = \"mine\""));
        let mut model = f.model("mine");
        model.start_edit();
        model.set_field(FieldId::Name, "ours");
        assert_eq!(f.confirm(&mut model), Outcome::Written);
        let text = f.text();
        assert!(text.contains("theme = \"ours\"\n\n[[profiles]]"), "{text}");
        assert!(text.contains("name = \"Shell\"\ntheme = \"ours\""), "{text}");
        assert!(text.contains("[themes.ours]") && !text.contains("mine"), "{text}");
        assert_eq!(model.themes.active(), "ours");
        assert_eq!(selected_theme(&model), "ours");
    }

    #[test]
    fn discard_restores_the_selection() {
        let f = File::new(CONFIG);
        let mut model = f.model("mine");
        model.move_selection(true);
        model.start_edit();
        model.set_field(FieldId::Color(1), "#1");
        assert_eq!(model.cancel(), Step::Nothing);
        assert!(model.in_list());
        assert_eq!(selected_theme(&model), "spare");
        assert_eq!(model.preview().unwrap().background, Color::hex(0x111111));
        assert_eq!(f.text(), CONFIG);
    }

    #[test]
    fn clearing_the_selection_foreground() {
        let mut f = File::new(CONFIG);
        let mut model = f.model("solarized-dark");
        model.start_new();
        assert!(editor(&model).current.selection_foreground.is_some());
        model.clear_selection_foreground();
        assert_eq!(model.preview().unwrap().selection_foreground, None);
        assert!(model.themes.editor_can_save());
        assert_eq!(f.confirm(&mut model), Outcome::Written);
        assert!(!f.text().contains("selection_foreground"));
    }

    #[test]
    fn editor_fields_and_keys() {
        let f = File::new(CONFIG);
        let mut model = f.model("mine");
        assert_eq!(model.editor_fields(), None);
        model.start_edit();
        let fields = model.editor_fields().unwrap();
        assert_eq!(fields.len(), 1 + SLOTS.len());
        assert_eq!(fields[0], (FieldId::Name, "mine".to_string()));
        assert_eq!(fields[2], (FieldId::Color(1), "#000000".to_string()));
        assert_eq!(
            (themes::field_label(FieldId::Name), themes::field_label(FieldId::Color(0))),
            (Some("name"), Some("Foreground"))
        );
        assert_eq!(themes::field_label(FieldId::Color(SLOTS.len())), None);
        // Typing belongs to the inputs; list shortcuts are off, and up and
        // down leave the editor (the view moves focus between fields).
        assert_eq!(model.list_key(&Keystroke::parse("n").unwrap()), None);
        model.move_selection(true);
        assert_eq!(model.mode_kind(), ModeKind::Editor);
        model.set_field(FieldId::Color(5), "#abcdef");
        assert!(model.enabled(Btn::Clear));
        assert_eq!(model.clear_selection_foreground(), Some(FieldId::Color(5)));
        assert_eq!(FieldId::selection_foreground(), FieldId::Color(5));
        assert_eq!(model.editor_fields().unwrap()[6], (FieldId::Color(5), String::new()));
        assert!(!model.enabled(Btn::Clear));
        // The dump names the focused field, and nothing for none.
        assert!(model.describe_open(Some("Cursor"), None).contains("field=\"Cursor\""));
        assert!(model.describe_open(None, None).contains("field=\"\""));
    }

    #[test]
    fn copies_save_the_derived_search_colors() {
        let mut f = File::new(CONFIG);
        let mut model = f.model("tokyo-night");
        model.start_new();
        let tokyo = builtin_theme("tokyo-night").unwrap();
        assert_eq!(editor(&model).current.search_focused_match_background, Some(tokyo.ansi[3]));
        assert_eq!(f.confirm(&mut model), Outcome::Written);
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
        let mut f = File::new(CONFIG);
        let mut model = f.model("mine");
        // In use as the global theme.
        f.key(&mut model, "d");
        assert!(model.in_list());
        assert!(model.error.as_deref().unwrap().contains("the global theme"));
        // Built-ins can't be deleted.
        model.select(0);
        f.key(&mut model, "d");
        assert!(model.error.as_deref().unwrap().contains("built in and can't be deleted"));
        // An unused one goes after confirming.
        model.select(7);
        f.key(&mut model, "delete");
        assert!(model.describe("").contains("mode=confirm-delete theme=\"spare\""));
        assert_eq!(model.cancel(), Step::Nothing);
        assert_eq!(f.text(), CONFIG);
        f.key(&mut model, "d");
        assert_eq!(f.confirm(&mut model), Outcome::Written);
        assert!(!f.text().contains("spare"));
        assert!(!names(&model).contains(&"spare"));
        assert_eq!(selected_theme(&model), "mine");
    }
}
