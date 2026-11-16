//! The settings dialog's Fonts section: the terminal font's family, size and
//! line height, and the chrome's family, previewed as they change and
//! written by Apply.
//!
//! The model half is GPUI-free and unit-tested: [`FontsState`] reads the four
//! values from the config file, keeps the fields as text and choices,
//! validates them and gives the preview and the write; [`family_names`] and
//! [`family_groups`] build the family lists. The monospace probe behind the
//! terminal font's grouping ([`monospace_families`]) and the view half, an
//! `impl SettingsDialog`, follow.

use std::collections::HashSet;
use std::ops::RangeInclusive;
use std::path::Path;
use std::sync::Arc;

use toml_edit::{DocumentMut, Item};

use crate::config::{DEFAULT_CONFIG, FontConfig, UiFontConfig};
use crate::config_edit::{self, FONT_TABLE, UI_FONT_TABLE};
use crate::settings::{FamilyChoice, FontPreview, first_installed};

/// A field of the Fonts section, in Tab order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FontField {
    Terminal,
    Size,
    LineHeight,
    Ui,
}

pub const FONT_FIELDS: [FontField; 4] = [FontField::Terminal, FontField::Size, FontField::LineHeight, FontField::Ui];

impl FontField {
    pub fn label(self) -> &'static str {
        match self {
            FontField::Terminal => "Terminal font",
            FontField::Size => "Size",
            FontField::LineHeight => "Line height",
            FontField::Ui => "UI font",
        }
    }

    /// Whether the field is a family choice (a combobox) rather than text.
    pub fn is_family(self) -> bool {
        matches!(self, FontField::Terminal | FontField::Ui)
    }
}

/// The label of the choice that leaves a family list to Brindle's own.
pub const DEFAULT_FAMILY: &str = "Default";

/// The terminal font sizes the config takes (`Config::normalized`).
pub const SIZE_RANGE: RangeInclusive<f32> = 4.0..=96.0;
/// The line heights the config takes.
pub const LINE_HEIGHT_RANGE: RangeInclusive<f32> = 0.8..=3.0;

/// How a family choice is shown.
pub fn choice_label(choice: &FamilyChoice) -> &str {
    match choice {
        FamilyChoice::Default => DEFAULT_FAMILY,
        FamilyChoice::Family(name) => name,
    }
}

/// A number as a field shows it: `13`, `1.25`.
pub fn number_text(value: f32) -> String {
    format!("{value}")
}

fn parse_in(text: &str, range: &RangeInclusive<f32>, message: &str) -> Result<f32, String> {
    match text.trim().parse::<f32>() {
        Ok(value) if value.is_finite() && range.contains(&value) => Ok(value),
        _ => Err(message.to_string()),
    }
}

pub fn parse_size(text: &str) -> Result<f32, String> {
    parse_in(text, &SIZE_RANGE, "use a number from 4 to 96")
}

pub fn parse_line_height(text: &str) -> Result<f32, String> {
    parse_in(text, &LINE_HEIGHT_RANGE, "use a number from 0.8 to 3.0")
}

// ---- reading -----------------------------------------------------------------

/// The config's four font values as the section shows them.
#[derive(Clone, Debug, PartialEq)]
pub struct FontValues {
    pub terminal: FamilyChoice,
    pub size: f32,
    pub line_height: f32,
    pub ui: FamilyChoice,
}

impl FontValues {
    /// A family list shows as its first installed family, or "Default"
    /// when none is installed.
    pub fn new(font: &FontConfig, ui_font: &UiFontConfig, installed: &[String]) -> Self {
        let choice = |list: &[String]| match first_installed(list, installed) {
            Some(name) => FamilyChoice::Family(name),
            None => FamilyChoice::Default,
        };
        Self {
            terminal: choice(&font.family),
            size: font.size,
            line_height: font.line_height,
            ui: choice(&ui_font.family),
        }
    }
}

/// `[font]` and `[ui_font]` from the file, read on their own so that a
/// mistake elsewhere in it doesn't hide them. A key that is missing or
/// can't be read takes its default, and numbers are clamped to the
/// config's ranges, as a load does.
pub fn fonts_in(doc: &DocumentMut) -> (FontConfig, UiFontConfig) {
    let defaults = FontConfig::default();
    let table = |name: &str| doc.get(name).and_then(|item| item.as_table_like());
    let strings = |item: Option<&Item>| -> Option<Vec<String>> {
        let array = item?.as_array()?;
        Some(array.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
    };
    let number = |item: Option<&Item>| -> Option<f32> {
        let value = item?.as_value()?;
        let number = value.as_float().map(|f| f as f32).or_else(|| value.as_integer().map(|i| i as f32))?;
        number.is_finite().then_some(number)
    };
    let font = table(FONT_TABLE);
    let ui_font = table(UI_FONT_TABLE);
    let font = FontConfig {
        family: font.and_then(|t| strings(t.get("family"))).unwrap_or(defaults.family),
        size: font
            .and_then(|t| number(t.get("size")))
            .unwrap_or(defaults.size)
            .clamp(*SIZE_RANGE.start(), *SIZE_RANGE.end()),
        line_height: font
            .and_then(|t| number(t.get("line_height")))
            .unwrap_or(defaults.line_height)
            .clamp(*LINE_HEIGHT_RANGE.start(), *LINE_HEIGHT_RANGE.end()),
    };
    let ui_font = UiFontConfig { family: ui_font.and_then(|t| strings(t.get("family"))).unwrap_or_default() };
    (font, ui_font)
}

// ---- the state ---------------------------------------------------------------

/// What an apply did.
#[derive(Debug, PartialEq)]
pub enum Done {
    /// Nothing was changed, so nothing was written.
    Nothing,
    /// The config file was written.
    Written,
}

/// The Fonts section's fields. They are always editable: the section has
/// no list and no editor mode.
pub struct FontsState {
    /// What the file gives: the fields' starting values, what a reset goes
    /// back to and what Apply compares with.
    pub saved: FontValues,
    pub terminal: FamilyChoice,
    pub size: String,
    pub line_height: String,
    pub ui: FamilyChoice,
    /// Why nothing can be written (the file isn't valid TOML).
    pub read_only: Option<String>,
    /// The installed families, as GPUI lists them.
    installed: Vec<String>,
}

impl FontsState {
    /// Reads the fields from the file at `path` (the default config when
    /// it doesn't exist), or from `fallback` (the loaded config's fonts),
    /// read-only, when the file can't be parsed.
    pub fn new(path: &Path, fallback: &(FontConfig, UiFontConfig), installed: Vec<String>) -> Self {
        let saved = FontValues::new(&fallback.0, &fallback.1, &installed);
        let mut state = Self {
            terminal: saved.terminal.clone(),
            size: number_text(saved.size),
            line_height: number_text(saved.line_height),
            ui: saved.ui.clone(),
            saved,
            read_only: None,
            installed,
        };
        state.reload(path, fallback);
        state
    }

    /// Rereads the file and resets every field to it, dropping unapplied
    /// changes.
    pub fn reload(&mut self, path: &Path, fallback: &(FontConfig, UiFontConfig)) {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => Ok(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(DEFAULT_CONFIG.to_string()),
            Err(e) => Err(format!("reading {}: {e}", path.display())),
        };
        let (font, ui_font) = match text.and_then(|text| config_edit::parse(&text)) {
            Ok(doc) => {
                self.read_only = None;
                fonts_in(&doc)
            }
            Err(e) => {
                self.read_only = Some(e);
                fallback.clone()
            }
        };
        self.saved = FontValues::new(&font, &ui_font, &self.installed);
        self.reset();
    }

    /// Puts every field back to the file's value.
    pub fn reset(&mut self) {
        self.terminal = self.saved.terminal.clone();
        self.size = number_text(self.saved.size);
        self.line_height = number_text(self.saved.line_height);
        self.ui = self.saved.ui.clone();
    }

    pub fn installed(&self) -> &[String] {
        &self.installed
    }

    /// A text field's text.
    pub fn text(&self, field: FontField) -> &str {
        match field {
            FontField::Size => &self.size,
            FontField::LineHeight => &self.line_height,
            FontField::Terminal | FontField::Ui => "",
        }
    }

    pub fn set_text(&mut self, field: FontField, text: &str) {
        match field {
            FontField::Size => self.size = text.to_string(),
            FontField::LineHeight => self.line_height = text.to_string(),
            FontField::Terminal | FontField::Ui => {}
        }
    }

    /// A family field's choice.
    pub fn choice(&self, field: FontField) -> &FamilyChoice {
        match field {
            FontField::Ui => &self.ui,
            _ => &self.terminal,
        }
    }

    pub fn choose(&mut self, field: FontField, choice: FamilyChoice) {
        match field {
            FontField::Terminal => self.terminal = choice,
            FontField::Ui => self.ui = choice,
            FontField::Size | FontField::LineHeight => {}
        }
    }

    /// The fields marked invalid, with why.
    pub fn errors(&self) -> Vec<(FontField, String)> {
        let mut errors = Vec::new();
        if let Err(e) = parse_size(&self.size) {
            errors.push((FontField::Size, e));
        }
        if let Err(e) = parse_line_height(&self.line_height) {
            errors.push((FontField::LineHeight, e));
        }
        errors
    }

    /// Apply is enabled while no field is invalid.
    pub fn can_apply(&self) -> bool {
        self.errors().is_empty()
    }

    /// The valid fields that differ from the file, or `None` when none does.
    pub fn preview(&self) -> Option<FontPreview> {
        let family = |choice: &FamilyChoice, saved: &FamilyChoice| (choice != saved).then(|| choice.clone());
        let number = |parsed: Result<f32, String>, saved: f32| parsed.ok().filter(|value| *value != saved);
        let preview = FontPreview {
            terminal_family: family(&self.terminal, &self.saved.terminal),
            size: number(parse_size(&self.size), self.saved.size),
            line_height: number(parse_line_height(&self.line_height), self.saved.line_height),
            ui_family: family(&self.ui, &self.saved.ui),
        };
        (preview != FontPreview::default()).then_some(preview)
    }

    /// Apply: writes the fields that differ from the file, and only those,
    /// keeping its comments. A chosen family goes to the front of its
    /// table's `family` list; "Default" removes the list.
    pub fn apply(&mut self, path: &Path) -> Result<Done, String> {
        if let Some(e) = &self.read_only {
            return Err(e.clone());
        }
        if !self.can_apply() {
            return Err("Fix the fields marked invalid first".into());
        }
        let Some(changes) = self.preview() else { return Ok(Done::Nothing) };
        let name = |choice: &FamilyChoice| match choice {
            FamilyChoice::Default => None,
            FamilyChoice::Family(name) => Some(name.clone()),
        };
        config_edit::save(path, |doc| {
            if let Some(choice) = &changes.terminal_family {
                config_edit::set_font_family(doc, FONT_TABLE, name(choice).as_deref());
            }
            if let Some(size) = changes.size {
                config_edit::set_font_number(doc, "size", size);
            }
            if let Some(line_height) = changes.line_height {
                config_edit::set_font_number(doc, "line_height", line_height);
            }
            if let Some(choice) = &changes.ui_family {
                config_edit::set_font_family(doc, UI_FONT_TABLE, name(choice).as_deref());
            }
            Ok(())
        })?;
        Ok(Done::Written)
    }

    /// The section's `--- settings` line. `field` names the focused
    /// control; `open` is an open family list's field, search and
    /// highlighted option.
    pub fn describe(&self, error: &Option<String>, field: &str, open: Option<(FontField, String, Option<String>)>) -> String {
        let invalid: Vec<&str> = self.errors().iter().map(|(f, _)| f.label()).collect();
        let open = match open {
            Some((field, query, highlighted)) => {
                format!(" open={:?} query={query:?} highlighted={:?}", field.label(), highlighted.unwrap_or_default())
            }
            None => String::new(),
        };
        format!(
            "--- settings section=Fonts mode=form terminal={:?} size={:?} line_height={:?} ui={:?} field={field:?} invalid={invalid:?} can_apply={} error={error:?}{open}\n",
            choice_label(&self.terminal),
            self.size,
            self.line_height,
            choice_label(&self.ui),
            self.can_apply(),
        )
    }
}

// ---- family lists ------------------------------------------------------------

/// The families a choice lists, from GPUI's installed names: without GPUI's
/// own (`.SystemUIFont`, `.ZedMono`) and Windows' vertical-writing variants
/// (`@MS Gothic`), each once (case-insensitively), sorted case-insensitively.
pub fn family_names(installed: &[String]) -> Vec<String> {
    let mut names: Vec<String> = installed
        .iter()
        .filter(|name| !name.trim().is_empty() && !name.starts_with('.') && !name.starts_with('@'))
        .cloned()
        .collect();
    names.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()).then_with(|| a.cmp(b)));
    names.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
    names
}

/// One group of a family list: under a heading or none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FamilyGroup {
    pub heading: Option<&'static str>,
    pub choices: Vec<FamilyChoice>,
}

/// A family field's options: "Default" first, then `names`. For the
/// terminal font, once the probe has found the `monospace` families, they
/// come first under "Monospace" and the others follow under "Other";
/// until then, and for the UI font, the families are one list.
pub fn family_groups(field: FontField, names: &[String], monospace: Option<&HashSet<String>>) -> Vec<FamilyGroup> {
    let default = FamilyGroup { heading: None, choices: vec![FamilyChoice::Default] };
    let families = |names: &mut dyn Iterator<Item = &String>| names.map(|n| FamilyChoice::Family(n.clone())).collect();
    match (field, monospace) {
        (FontField::Terminal, Some(mono)) => {
            let (fixed, other): (Vec<&String>, Vec<&String>) = names.iter().partition(|n| mono.contains(*n));
            vec![
                default,
                FamilyGroup { heading: Some("Monospace"), choices: families(&mut fixed.into_iter()) },
                FamilyGroup { heading: Some("Other"), choices: families(&mut other.into_iter()) },
            ]
        }
        _ => vec![default, FamilyGroup { heading: None, choices: families(&mut names.iter()) }],
    }
}

/// The options of `groups` in order, which is how a choice's index in its
/// list is counted.
pub fn family_options(groups: &[FamilyGroup]) -> Vec<FamilyChoice> {
    groups.iter().flat_map(|group| group.choices.iter().cloned()).collect()
}

/// Where `choice` is among `options` (a family matched case-insensitively).
pub fn option_index(options: &[FamilyChoice], choice: &FamilyChoice) -> Option<usize> {
    options.iter().position(|option| match (option, choice) {
        (FamilyChoice::Family(a), FamilyChoice::Family(b)) => a.eq_ignore_ascii_case(b),
        (a, b) => a == b,
    })
}

// ---- the monospace probe -----------------------------------------------------

/// The monospace families of this process: probed once, the first time the
/// Fonts section opens, on the background executor, since it loads every
/// family. `None` until the probe is done.
#[derive(Default)]
pub struct MonospaceProbe {
    started: bool,
    families: Option<Arc<HashSet<String>>>,
}

impl gpui::Global for MonospaceProbe {}

/// The monospace families among `names` once the probe is done, starting
/// it the first time. Observe [`MonospaceProbe`] to hear when it is.
pub fn monospace_families(names: &[String], cx: &mut gpui::App) -> Option<Arc<HashSet<String>>> {
    let probe = cx.default_global::<MonospaceProbe>();
    if probe.started {
        return probe.families.clone();
    }
    probe.started = true;
    let names = names.to_vec();
    let text_system = cx.text_system().clone();
    cx.spawn(async move |cx| {
        let families = cx
            .background_executor()
            .spawn(async move { names.into_iter().filter(|name| is_monospace(&text_system, name)).collect() })
            .await;
        cx.update(|cx| cx.update_global::<MonospaceProbe, _>(|probe, _| probe.families = Some(Arc::new(families))))
            .ok();
    })
    .detach();
    None
}

/// Whether `family` is monospace: resolved at 16px, `i` and `M` advance
/// alike. A family GPUI can't load counts as proportional (it resolves to
/// a fallback, which may be monospace itself, so the resolved font must be
/// the family's own), as does a family without the two glyphs.
fn is_monospace(text_system: &gpui::TextSystem, family: &str) -> bool {
    let probe = std::panic::AssertUnwindSafe(|| {
        let id = text_system.resolve_font(&gpui::font(family.to_string()));
        let own = text_system.get_font_for_id(id).is_some_and(|font| font.family.eq_ignore_ascii_case(family));
        if !own {
            return false;
        }
        let size = gpui::px(16.);
        match (text_system.advance(id, size, 'i'), text_system.advance(id, size, 'M')) {
            (Ok(i), Ok(m)) => (f32::from(i.width) - f32::from(m.width)).abs() < 0.01,
            _ => false,
        }
    });
    // `resolve_font` panics when not even GPUI's fallbacks load.
    std::panic::catch_unwind(probe).unwrap_or(false)
}

// ---- view -------------------------------------------------------------------

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, BorrowAppContext as _, Context, Entity, FocusHandle, Focusable as _, IntoElement,
    ParentElement, SharedString, Styled, Window, div, px,
};

use super::{Btn, Section, SettingsDialog};
use brindle_ui::{
    ActiveTheme as _, ButtonVariant, Combobox, ComboboxEvent, CommandGroup, CommandItem, FIELD_HEIGHT, Input, TextInput,
    TextInputEvent,
};

/// The fields typed as text.
const TEXT_FIELDS: [FontField; 2] = [FontField::Size, FontField::LineHeight];
/// The family fields, each a combobox.
const FAMILY_FIELDS: [FontField; 2] = [FontField::Terminal, FontField::Ui];

/// A family list's groups as a combobox lists them.
fn command_groups(groups: &[FamilyGroup]) -> Vec<CommandGroup> {
    groups
        .iter()
        .map(|group| {
            let items = group.choices.iter().map(|choice| CommandItem::new(choice_label(choice).to_string()));
            match group.heading {
                Some(heading) => CommandGroup::new(heading, items),
                None => CommandGroup::unnamed(items),
            }
        })
        .collect()
}

impl SettingsDialog {
    /// Creates the Fonts section's inputs and comboboxes when it is shown
    /// and drops them when it is left. Focus follows in `place_focus`.
    pub(super) fn sync_font_fields(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.model.section() != Section::Fonts {
            // A dropped combobox would leave an open panel's layer behind.
            for (_, combobox) in std::mem::take(&mut self.font_choices) {
                combobox.update(cx, |c, cx| c.close(window, cx));
            }
            self.font_inputs.clear();
            self._font_subscriptions.clear();
            return;
        }
        if !self.font_inputs.is_empty() {
            return;
        }
        if self.font_names.is_empty() {
            self.font_names = family_names(self.model.fonts.installed());
        }
        let monospace = monospace_families(&self.font_names, cx);
        if monospace.is_none() {
            // The terminal font's list regroups when the probe is done.
            let probed = cx.observe_global::<MonospaceProbe>(|this, cx| this.regroup_terminal_families(cx));
            self._font_subscriptions.push(probed);
        }
        for field in TEXT_FIELDS {
            let text = self.model.fonts.text(field).to_string();
            let input = cx.new(|cx| TextInput::new(&text, cx));
            let changed = cx.subscribe_in(&input, window, move |this, input, _: &TextInputEvent, _, cx| {
                let text = input.read(cx).text().to_string();
                this.model.fonts.set_text(field, &text);
                this.model.error = None;
                this.preview(cx);
                cx.notify();
            });
            self._font_subscriptions.push(changed);
            self.font_inputs.push((field, input));
        }
        for field in FAMILY_FIELDS {
            let groups = family_groups(field, &self.font_names, monospace.as_deref());
            let selected = option_index(&family_options(&groups), self.model.fonts.choice(field));
            let combobox = cx.new(|cx| {
                Combobox::new(command_groups(&groups), selected, cx)
                    .with_placeholder("Choose a font")
                    .with_search_placeholder("Search fonts…")
                    .with_empty_message("No font found.")
            });
            let changed = cx.subscribe_in(&combobox, window, move |this, combobox, event: &ComboboxEvent, _, cx| {
                let ComboboxEvent::Changed(ix) = *event;
                // Option 0 is "Default"; the others are families, by name.
                let choice = match ix {
                    0 => FamilyChoice::Default,
                    _ => match combobox.read(cx).option(ix) {
                        Some(item) => FamilyChoice::Family(item.label.to_string()),
                        None => return,
                    },
                };
                this.model.fonts.choose(field, choice);
                this.model.error = None;
                this.preview(cx);
                cx.notify();
            });
            self._font_subscriptions.push(changed);
            self.font_choices.push((field, combobox));
        }
    }

    /// Moves the monospace families to the top of the terminal font's list
    /// once the probe has found them, keeping the choice by its name.
    fn regroup_terminal_families(&mut self, cx: &mut Context<Self>) {
        let Some(monospace) = cx.try_global::<MonospaceProbe>().and_then(|p| p.families.clone()) else { return };
        let Some(combobox) = self.font_combobox(FontField::Terminal).cloned() else { return };
        let groups = family_groups(FontField::Terminal, &self.font_names, Some(&monospace));
        let selected = option_index(&family_options(&groups), self.model.fonts.choice(FontField::Terminal));
        combobox.update(cx, |c, cx| c.set_groups(command_groups(&groups), selected, cx));
    }

    fn font_input(&self, field: FontField) -> Option<&Entity<TextInput>> {
        self.font_inputs.iter().find(|(f, _)| *f == field).map(|(_, i)| i)
    }

    fn font_combobox(&self, field: FontField) -> Option<&Entity<Combobox>> {
        self.font_choices.iter().find(|(f, _)| *f == field).map(|(_, c)| c)
    }

    /// The Fonts section's field stops, in order.
    pub(super) fn font_field_stops(&self, cx: &App) -> Vec<FocusHandle> {
        FONT_FIELDS
            .iter()
            .filter_map(|&field| match field.is_family() {
                true => self.font_combobox(field).map(|c| c.focus_handle(cx)),
                false => self.font_input(field).map(|i| i.focus_handle(cx)),
            })
            .collect()
    }

    /// The text input of a focused font field: a size, or an open family
    /// list's search.
    pub(super) fn focused_font_input(&self, window: &Window, cx: &App) -> Option<Entity<TextInput>> {
        let searches = self
            .font_choices
            .iter()
            .filter_map(|(_, c)| c.read(cx).command(cx))
            .map(|command| command.read(cx).filter_input().clone());
        let mut inputs = self.font_inputs.iter().map(|(_, i)| i.clone()).chain(searches);
        inputs.find(|i| i.read(cx).is_focused(window))
    }

    /// The focused font field: a text field, or a combobox whose trigger has
    /// focus or whose list is open.
    pub(super) fn focused_font_field(&self, window: &Window, cx: &App) -> Option<FontField> {
        if let Some((field, _)) = self.font_inputs.iter().find(|(_, i)| i.read(cx).is_focused(window)) {
            return Some(*field);
        }
        let focused = |c: &Entity<Combobox>| c.read(cx).is_open() || c.focus_handle(cx).is_focused(window);
        self.font_choices.iter().find(|(_, c)| focused(c)).map(|(f, _)| *f)
    }

    /// Whether a family list is open; it holds focus while it is.
    pub(super) fn font_list_open(&self, cx: &App) -> bool {
        self.font_choices.iter().any(|(_, c)| c.read(cx).is_open())
    }

    /// The open family list's field, search and highlighted option, for the
    /// dump.
    pub(super) fn open_font_list(&self, cx: &App) -> Option<(FontField, String, Option<String>)> {
        let (field, combobox) = self.font_choices.iter().find(|(_, c)| c.read(cx).is_open())?;
        let combobox = combobox.read(cx);
        let query = combobox.command(cx).map(|command| command.read(cx).query(cx)).unwrap_or_default();
        let highlighted = combobox.highlighted(cx).and_then(|ix| combobox.option(ix)).map(|item| item.label.to_string());
        Some((*field, query, highlighted))
    }

    // ---- rendering ----

    pub(super) fn render_fonts(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = *cx.ui_theme();
        let muted = t.muted_foreground;
        let red = t.destructive;
        let fonts = &self.model.fonts;
        let errors = fonts.errors();
        let label = |text: &'static str| div().w(px(120.0)).flex_none().text_color(muted).child(text);

        // The profile editor's field rows: the control line at least a text
        // field tall with its control centred, an error under it at its
        // natural height.
        let rows = FONT_FIELDS.iter().map(|&field| {
            let error = errors.iter().find(|(f, _)| *f == field).map(|(_, e)| e.clone());
            let control = div().flex_1().min_w_0();
            let control = match field.is_family() {
                true => control.children(self.font_combobox(field).cloned()),
                false => control.children(self.font_input(field).map(|i| Input::new(i).invalid(error.is_some()))),
            };
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
        });

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap_2()
            .child(div().text_xs().text_color(muted).child(
                "Changes show in this window as you make them; Apply writes them to the config file.",
            ))
            .when_some(fonts.read_only.clone(), |d, e| {
                d.child(div().text_xs().text_color(red).child(SharedString::from(format!("Read-only: {e}"))))
            })
            .child(div().flex().flex_col().flex_1().min_h_0().gap_1().children(rows))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(div().flex_1())
                    .child(self.button(Btn::Cancel, "Cancel", ButtonVariant::Outline, cx, |_, _, cx| {
                        cx.emit(gpui::DismissEvent)
                    }))
                    .child(self.button(Btn::Apply, "Apply", ButtonVariant::Default, cx, |this, window, cx| {
                        let outcome = this.model.confirm();
                        this.outcome(outcome, window, cx);
                    })),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    const INSTALLED: [&str; 6] = ["Cascadia Mono", "Consolas", "Arial", "Segoe UI", "Inter", ".SystemUIFont"];

    fn installed() -> Vec<String> {
        INSTALLED.iter().map(|s| s.to_string()).collect()
    }

    fn fallback() -> (FontConfig, UiFontConfig) {
        (FontConfig::default(), UiFontConfig::default())
    }

    struct Fixture {
        dir: PathBuf,
        path: PathBuf,
    }

    impl Fixture {
        fn new(name: &str, text: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("brindle-fonts-{}-{name}", std::process::id()));
            std::fs::remove_dir_all(&dir).ok();
            std::fs::create_dir_all(&dir).unwrap();
            let path = dir.join("config.toml");
            std::fs::write(&path, text).unwrap();
            Self { dir, path }
        }

        fn state(&self) -> FontsState {
            FontsState::new(&self.path, &fallback(), installed())
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

    fn family(name: &str) -> FamilyChoice {
        FamilyChoice::Family(name.into())
    }

    /// The spec's "Current fonts shown".
    #[test]
    fn current_fonts_are_shown() {
        let f = Fixture::new("current", "# mine\n[font]\nfamily = [\"Nonexistent\", \"cascadia mono\"]\nsize = 13\n");
        let state = f.state();
        assert_eq!(state.terminal, family("Cascadia Mono"));
        assert_eq!((state.size.as_str(), state.line_height.as_str()), ("13", "1.25"));
        assert_eq!(state.ui, FamilyChoice::Default);
        assert!(state.read_only.is_none());
        assert_eq!(state.preview(), None);
        // Nothing installed from the list: "Default".
        let f = Fixture::new("none", "[font]\nfamily = [\"Nonexistent\"]\n[ui_font]\nfamily = [\"Inter\"]\n");
        let state = f.state();
        assert_eq!(state.terminal, FamilyChoice::Default);
        assert_eq!(state.ui, family("Inter"));
    }

    /// Values are read from the file even when another part of it has an
    /// error, and out-of-range numbers show as the load clamps them.
    #[test]
    fn fonts_are_read_past_errors_elsewhere() {
        let f = Fixture::new("elsewhere", "bogus_key = 1\ntheme = 7\n[font]\nsize = 200\nline_height = \"tall\"\n");
        let state = f.state();
        assert!(state.read_only.is_none());
        assert_eq!((state.size.as_str(), state.line_height.as_str()), ("96", "1.25"));
        // A missing file reads as the default config, which can be applied.
        let f = Fixture::new("missing", "");
        std::fs::remove_file(&f.path).unwrap();
        let state = f.state();
        assert!(state.read_only.is_none());
        assert_eq!(state.terminal, family("Cascadia Mono"));
        assert_eq!(state.size, "14");
    }

    /// The spec's "Size out of range": marked invalid with the range, Apply
    /// disabled, and the invalid value not previewed.
    #[test]
    fn a_size_out_of_range_is_invalid() {
        let f = Fixture::new("range", "[font]\nsize = 13\n");
        let mut state = f.state();
        state.set_text(FontField::Size, "200");
        let errors = state.errors();
        assert_eq!(errors.len(), 1);
        assert_eq!(errors[0].0, FontField::Size);
        assert!(errors[0].1.contains("4 to 96"), "{}", errors[0].1);
        assert!(!state.can_apply());
        assert_eq!(state.preview(), None);
        assert!(state.apply(&f.path).is_err());
        assert_eq!(f.text(), "[font]\nsize = 13\n");
        for bad in ["", "abc", "3.9", "NaN", "inf"] {
            state.set_text(FontField::Size, bad);
            assert!(!state.can_apply(), "{bad:?}");
        }
        state.set_text(FontField::Size, " 18 ");
        state.set_text(FontField::LineHeight, "3.5");
        let errors = state.errors();
        assert_eq!(errors.iter().map(|(f, _)| *f).collect::<Vec<_>>(), [FontField::LineHeight]);
        assert!(errors[0].1.contains("0.8 to 3.0"));
        // The valid change is still previewed.
        assert_eq!(state.preview(), Some(FontPreview { size: Some(18.0), ..Default::default() }));
    }

    /// Only the fields that differ from the file are previewed.
    #[test]
    fn the_preview_holds_the_changed_fields() {
        let f = Fixture::new("preview", "[font]\nsize = 13\n");
        let mut state = f.state();
        state.choose(FontField::Ui, family("Inter"));
        state.set_text(FontField::Size, "18");
        assert_eq!(
            state.preview(),
            Some(FontPreview { size: Some(18.0), ui_family: Some(family("Inter")), ..Default::default() })
        );
        // Back to the file's values: nothing to preview.
        state.choose(FontField::Ui, FamilyChoice::Default);
        state.set_text(FontField::Size, "13.0");
        assert_eq!(state.preview(), None);
        state.choose(FontField::Terminal, FamilyChoice::Default);
        state.set_text(FontField::LineHeight, "1.5");
        assert_eq!(
            state.preview(),
            Some(FontPreview {
                terminal_family: Some(FamilyChoice::Default),
                line_height: Some(1.5),
                ..Default::default()
            })
        );
        // A reset drops the changes.
        state.reset();
        assert_eq!(state.preview(), None);
        assert_eq!(state.terminal, family("Cascadia Mono"));
    }

    /// The spec's "Choose a UI font": `[ui_font]` is added, comments kept.
    #[test]
    fn choosing_a_ui_font_writes_ui_font() {
        let text = "# My config\ntheme = \"brindle-dark\" # dark\n\n[font]\nsize = 13\n";
        let f = Fixture::new("ui", text);
        let mut state = f.state();
        state.choose(FontField::Ui, family("Inter"));
        assert_eq!(state.apply(&f.path), Ok(Done::Written));
        assert_eq!(f.text(), format!("{text}\n[ui_font]\nfamily = [\"Inter\"]\n"));
        let config = crate::config::Config::parse(&f.text()).unwrap();
        assert_eq!(config.ui_font.family, ["Inter"]);
        assert_eq!(config.font.size, 13.0);
    }

    /// The spec's "Choose a listed fallback", and unchanged fields writing
    /// nothing.
    #[test]
    fn a_chosen_family_moves_to_the_front() {
        let text = "[font]\nfamily = [\"JetBrains Mono\", \"Cascadia Mono\"]\nsize = 13.0 # mine\n";
        let f = Fixture::new("fallback", text);
        let mut state = f.state();
        assert_eq!(state.terminal, family("Cascadia Mono"));
        state.choose(FontField::Terminal, family("Consolas"));
        assert_eq!(state.apply(&f.path), Ok(Done::Written));
        // The size wasn't changed, so it is still written as it was.
        assert_eq!(f.text(), "[font]\nfamily = [\"Consolas\", \"JetBrains Mono\", \"Cascadia Mono\"]\nsize = 13.0 # mine\n");
        // "Default" removes the list.
        let mut state = f.state();
        state.choose(FontField::Terminal, FamilyChoice::Default);
        state.set_text(FontField::Size, "20");
        assert_eq!(state.apply(&f.path), Ok(Done::Written));
        assert_eq!(f.text(), "[font]\nsize = 20 # mine\n");
        // Nothing changed: nothing written.
        let mut state = f.state();
        assert_eq!(state.apply(&f.path), Ok(Done::Nothing));
        assert_eq!(f.text(), "[font]\nsize = 20 # mine\n");
    }

    /// A file that isn't valid TOML: the loaded config's fonts, read-only,
    /// and Apply writes nothing and says why.
    #[test]
    fn an_unparseable_file_is_read_only() {
        let f = Fixture::new("broken", "[font\nsize = 13\n");
        let fallback = (FontConfig { size: 15.0, ..FontConfig::default() }, UiFontConfig { family: vec!["Arial".into()] });
        let mut state = FontsState::new(&f.path, &fallback, installed());
        assert!(state.read_only.is_some());
        assert_eq!((state.size.as_str(), &state.ui), ("15", &family("Arial")));
        state.set_text(FontField::Size, "18");
        let err = state.apply(&f.path).unwrap_err();
        assert!(err.contains("not valid TOML"), "{err}");
        assert_eq!(f.text(), "[font\nsize = 13\n");
    }

    #[test]
    fn the_dump_line_names_every_field() {
        let f = Fixture::new("dump", "[font]\nsize = 13\n");
        let mut state = f.state();
        state.set_text(FontField::Size, "200");
        assert_eq!(
            state.describe(&None, "Size", None),
            "--- settings section=Fonts mode=form terminal=\"Cascadia Mono\" size=\"200\" line_height=\"1.25\" ui=\"Default\" field=\"Size\" invalid=[\"Size\"] can_apply=false error=None\n"
        );
        let open = Some((FontField::Ui, "in".to_string(), Some("Inter".to_string())));
        assert!(state.describe(&None, "UI font", open).ends_with(" open=\"UI font\" query=\"in\" highlighted=\"Inter\"\n"));
    }

    #[test]
    fn family_names_drop_gpui_names_and_vertical_variants() {
        let installed: Vec<String> =
            ["Segoe UI", ".SystemUIFont", "@MS Gothic", "arial", "Arial", "MS Gothic", "consolas", ".ZedMono", " "]
                .iter()
                .map(|s| s.to_string())
                .collect();
        assert_eq!(family_names(&installed), ["Arial", "consolas", "MS Gothic", "Segoe UI"]);
    }

    /// Before the probe, one alphabetical list after "Default"; after it,
    /// monospace families first under their heading. The UI font is never
    /// grouped.
    #[test]
    fn monospace_families_come_first_once_probed() {
        let names = family_names(&installed());
        assert_eq!(names, ["Arial", "Cascadia Mono", "Consolas", "Inter", "Segoe UI"]);
        let plain = family_groups(FontField::Terminal, &names, None);
        assert_eq!(plain.len(), 2);
        assert_eq!(plain[0].choices, [FamilyChoice::Default]);
        assert_eq!(plain[1].heading, None);
        assert_eq!(plain[1].choices.len(), 5);
        let mono: HashSet<String> = ["Cascadia Mono", "Consolas"].iter().map(|s| s.to_string()).collect();
        let grouped = family_groups(FontField::Terminal, &names, Some(&mono));
        assert_eq!(grouped.iter().map(|g| g.heading).collect::<Vec<_>>(), [None, Some("Monospace"), Some("Other")]);
        assert_eq!(grouped[1].choices, [family("Cascadia Mono"), family("Consolas")]);
        assert_eq!(grouped[2].choices, [family("Arial"), family("Inter"), family("Segoe UI")]);
        assert_eq!(family_groups(FontField::Ui, &names, Some(&mono)), family_groups(FontField::Ui, &names, None));

        // Choices are found by name, so a regrouping keeps what is chosen.
        let options = family_options(&grouped);
        assert_eq!(option_index(&options, &family("consolas")), Some(2));
        assert_eq!(option_index(&options, &FamilyChoice::Default), Some(0));
        assert_eq!(option_index(&family_options(&plain), &family("Consolas")), Some(3));
        assert_eq!(option_index(&options, &family("Missing")), None);
    }
}
