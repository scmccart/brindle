//! The settings dialog's Clipboard section: two switches, "Copy on select"
//! (`copy_on_select`) and "Right-click pastes" (`right_click = "paste"`),
//! written by Apply.
//!
//! The model half is GPUI-free and unit-tested: [`ClipboardState`] reads the
//! two values from the config file's text it is given, keeps what the
//! switches show and gives the edit to write. The view half, an
//! `impl SettingsDialog`, follows.

use crate::config::{DEFAULT_CONFIG, RightClick};
use crate::config_edit::{self, ClipboardChange, ClipboardValues, ConfigEdit, FileText};

use super::{Btn, Step};

/// A switch of the Clipboard section, in Tab order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClipboardField {
    CopyOnSelect,
    RightClick,
}

pub const CLIPBOARD_FIELDS: [ClipboardField; 2] = [ClipboardField::CopyOnSelect, ClipboardField::RightClick];

impl ClipboardField {
    /// The switch's label, which the dump's `field=` names too.
    pub fn label(self) -> &'static str {
        match self {
            ClipboardField::CopyOnSelect => "Copy on select",
            ClipboardField::RightClick => "Right-click pastes",
        }
    }

    /// What the section says beside the switch, if anything.
    pub fn hint(self) -> Option<&'static str> {
        match self {
            ClipboardField::CopyOnSelect => None,
            ClipboardField::RightClick => {
                Some("Pastes only when nothing is selected. Hold shift to paste where a program uses the mouse.")
            }
        }
    }
}

/// Whether `field`'s switch is on in `values`.
pub fn is_on(values: &ClipboardValues, field: ClipboardField) -> bool {
    match field {
        ClipboardField::CopyOnSelect => values.copy_on_select,
        ClipboardField::RightClick => values.right_click == RightClick::Paste,
    }
}

/// `values` with `field`'s switch flipped.
fn toggled(values: ClipboardValues, field: ClipboardField) -> ClipboardValues {
    match field {
        ClipboardField::CopyOnSelect => ClipboardValues { copy_on_select: !values.copy_on_select, ..values },
        ClipboardField::RightClick => {
            let right_click = match values.right_click {
                RightClick::Nothing => RightClick::Paste,
                RightClick::Paste => RightClick::Nothing,
            };
            ClipboardValues { right_click, ..values }
        }
    }
}

/// The Clipboard section's switches. Like the Fonts section it has no list
/// and no editor mode.
pub struct ClipboardState {
    /// What the file gives: the switches' starting values, what a reset
    /// goes back to and what Apply compares with.
    pub saved: ClipboardValues,
    /// What the switches show.
    pub current: ClipboardValues,
    /// Why nothing can be written (the file isn't valid TOML, or couldn't
    /// be read): the switches show the loaded config's values and can't be
    /// changed.
    pub read_only: Option<String>,
    /// Each key the file sets to a value of the wrong kind, which shows as
    /// its default: the section's error line.
    note: Option<String>,
    /// The values to show (read-only) when the file can't be read: the
    /// loaded config's.
    fallback: ClipboardValues,
}

impl ClipboardState {
    /// Reads the switches from `text`, the config file's (the default
    /// config when it doesn't exist), or from `fallback` (the loaded
    /// config's values), read-only, when the file can't be parsed.
    pub fn new(text: &FileText, fallback: ClipboardValues) -> Self {
        let mut state = Self { saved: fallback, current: fallback, read_only: None, note: None, fallback };
        state.reload(text);
        state
    }

    /// Rereads the file's text and resets both switches to it, dropping
    /// unapplied changes.
    pub fn reload(&mut self, text: &FileText) {
        let parsed = match text {
            Ok(text) => config_edit::parse(text.as_deref().unwrap_or(DEFAULT_CONFIG)),
            Err(e) => Err(e.clone()),
        };
        (self.saved, self.note, self.read_only) = match parsed {
            Ok(doc) => {
                let (values, notes) = config_edit::clipboard_values(&doc);
                (values, (!notes.is_empty()).then(|| notes.join("; ")), None)
            }
            Err(e) => (self.fallback, None, Some(e)),
        };
        self.reset();
    }

    /// Puts both switches back to the file's values.
    pub fn reset(&mut self) {
        self.current = self.saved;
    }

    /// Why a key of the file shows as its default, for the error line.
    pub fn note(&self) -> Option<&String> {
        self.note.as_ref()
    }

    /// Whether `field`'s switch is on.
    pub fn is_on(&self, field: ClipboardField) -> bool {
        is_on(&self.current, field)
    }

    /// Whether the switches can change: not while the section is
    /// read-only, when they are drawn disabled and take no focus.
    pub fn editable(&self) -> bool {
        self.read_only.is_none()
    }

    /// Flips `field`'s switch; nothing while the section is read-only.
    pub fn toggle(&mut self, field: ClipboardField) {
        if self.editable() {
            self.current = toggled(self.current, field);
        }
    }

    /// Apply can write while the file can be read.
    pub fn can_apply(&self) -> bool {
        self.editable()
    }

    /// Apply: the settings that differ from the file, and only those, to
    /// write (keeping its comments), or closing when none does.
    pub fn apply(&self) -> Result<Step, String> {
        if let Some(e) = &self.read_only {
            return Err(e.clone());
        }
        let (saved, current) = (self.saved, self.current);
        let changes = [
            (current.copy_on_select != saved.copy_on_select).then_some(ClipboardChange::CopyOnSelect(current.copy_on_select)),
            (current.right_click != saved.right_click).then_some(ClipboardChange::RightClick(current.right_click)),
        ];
        let changes: Vec<ClipboardChange> = changes.into_iter().flatten().collect();
        if changes.is_empty() {
            return Ok(Step::Close);
        }
        Ok(Step::Write(ConfigEdit::SetClipboard(changes)))
    }

    /// Whether button `b` can be pressed (and focused) in this section.
    pub fn enabled(&self, b: Btn) -> bool {
        match b {
            Btn::Apply => self.can_apply(),
            Btn::New
            | Btn::Duplicate
            | Btn::Edit
            | Btn::Delete
            | Btn::Up
            | Btn::Down
            | Btn::Cancel
            | Btn::Close
            | Btn::Keep
            | Btn::ConfirmDelete
            | Btn::Clear
            | Btn::Discard
            | Btn::Save => true,
        }
    }

    /// The section's `--- settings` line. `field` names the focused
    /// control.
    pub fn describe(&self, error: &Option<String>, field: &str) -> String {
        format!(
            "--- settings section=Clipboard mode=form copy_on_select={} right_click={} field={field:?} can_apply={} error={error:?}\n",
            self.current.copy_on_select,
            self.current.right_click.word(),
            self.can_apply(),
        )
    }
}

// ---- view -------------------------------------------------------------------

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, Context, FocusHandle, InteractiveElement as _, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement as _, Styled, Window, div, px,
};

use super::SettingsDialog;
use brindle_ui::{ActiveTheme as _, ButtonVariant, FIELD_HEIGHT, Switch};

impl SettingsDialog {
    fn clipboard_handle(&self, field: ClipboardField) -> Option<&FocusHandle> {
        self.clipboard_handles.iter().find(|(f, _)| *f == field).map(|(_, h)| h)
    }

    /// The Clipboard section's field stops, in order: its switches, unless
    /// they are read-only (disabled, so they take no focus).
    pub(super) fn clipboard_field_stops(&self) -> Vec<FocusHandle> {
        if !self.model.clipboard.editable() {
            return Vec::new();
        }
        CLIPBOARD_FIELDS.iter().filter_map(|&field| self.clipboard_handle(field).cloned()).collect()
    }

    /// The switch that has focus.
    pub(super) fn focused_clipboard_field(&self, window: &Window) -> Option<ClipboardField> {
        self.clipboard_handles.iter().find(|(_, h)| h.is_focused(window)).map(|(f, _)| *f)
    }

    /// A switch was flipped, by space or a click on it or its label.
    fn toggle_clipboard(&mut self, field: ClipboardField, window: &mut Window, cx: &mut Context<Self>) {
        self.model.toggle_clipboard(field);
        self.changed(window, cx);
    }

    // ---- rendering ----

    pub(super) fn render_clipboard(&self, cx: &mut Context<Self>) -> AnyElement {
        let t = *cx.ui_theme();
        let muted = t.muted_foreground;
        let state = &self.model.clipboard;
        let editable = state.editable();

        // The profile editor's field rows: the label in its column, then the
        // control, at least a text field tall.
        let rows = CLIPBOARD_FIELDS
            .iter()
            .enumerate()
            .filter_map(|(ix, &field)| {
                let handle = self.clipboard_handle(field)?.clone();
                let label = div()
                    .id(("clipboard-label", ix))
                    .w(px(150.0))
                    .flex_none()
                    .text_color(muted)
                    // The label is the switch's: a click on it toggles too.
                    .when(editable, |d| {
                        d.cursor_pointer().on_click(cx.listener(move |this, _, window, cx| {
                            window.focus(&handle);
                            this.toggle_clipboard(field, window, cx)
                        }))
                    })
                    .child(field.label());
                let switch = Switch::new(("clipboard-switch", ix), state.is_on(field))
                    .disabled(!editable)
                    .focus(self.clipboard_handle(field)?)
                    .on_click(cx.listener(move |this, _: &bool, window, cx| this.toggle_clipboard(field, window, cx)));
                Some(
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .gap_2()
                        .min_h(FIELD_HEIGHT)
                        .child(label)
                        .child(switch)
                        .children(
                            field.hint().map(|hint| div().flex_1().min_w_0().text_xs().text_color(muted).child(hint)),
                        ),
                )
            })
            .collect::<Vec<_>>();

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_h_0()
            .gap_2()
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child("Apply writes these to the config file, and open tabs follow at once."),
            )
            .when_some(state.read_only.clone(), |d, e| {
                d.child(div().text_xs().text_color(t.destructive).child(SharedString::from(format!("Read-only: {e}"))))
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
                        let step = this.model.confirm();
                        this.step(step, window, cx);
                    })),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn values(copy_on_select: bool, right_click: RightClick) -> ClipboardValues {
        ClipboardValues { copy_on_select, right_click }
    }

    /// The config file as text: the state reads it, and an apply's edit is
    /// written to it as the view writes it to the file.
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

        fn state(&self) -> ClipboardState {
            ClipboardState::new(&self.read(), ClipboardValues::default())
        }

        fn text(&self) -> String {
            self.text.clone().unwrap_or_default()
        }

        /// Apply, with what it asks for written.
        fn apply(&mut self, state: &ClipboardState) -> Result<Step, String> {
            let step = state.apply()?;
            if let Step::Write(edit) = &step {
                let (text, _) = config_edit::edited(self.text.as_deref(), |doc| edit.apply(doc))?;
                self.text = Some(text);
            }
            Ok(step)
        }
    }

    /// The spec's "Current values shown", values read past errors
    /// elsewhere in the file, and a missing file read as the default config.
    #[test]
    fn the_switches_show_the_files_values() {
        let state = Fixture::new("copy_on_select = true\n").state();
        assert!(state.is_on(ClipboardField::CopyOnSelect) && !state.is_on(ClipboardField::RightClick));
        assert_eq!((state.read_only.as_deref(), state.note()), (None, None));
        let state = Fixture::new("bogus = 1\ntheme = 7\nright_click = \"paste\"\n").state();
        assert_eq!(state.current, values(false, RightClick::Paste));
        assert!(state.can_apply());
        let state = Fixture { text: None }.state();
        assert_eq!(state.current, ClipboardValues::default());
        assert!(state.can_apply());
    }

    /// A key of the wrong kind shows as its default, and the note names it.
    #[test]
    fn a_wrongly_typed_key_is_noted() {
        let state = Fixture::new("copy_on_select = \"yes\"\nright_click = \"menu\"\n").state();
        assert_eq!(state.current, ClipboardValues::default());
        let note = state.note().unwrap();
        assert!(note.contains("copy_on_select isn't true or false") && note.contains("; right_click isn't"), "{note}");
        assert!(state.can_apply());
    }

    /// The spec's "Unparseable config": the loaded config's values,
    /// read-only; the switches don't change and Apply writes nothing and
    /// says why.
    #[test]
    fn an_unparseable_file_shows_the_loaded_values_read_only() {
        let mut f = Fixture::new("[oops\ncopy_on_select = false\n");
        let mut state = ClipboardState::new(&f.read(), values(true, RightClick::Paste));
        assert_eq!(state.current, values(true, RightClick::Paste));
        assert!(state.read_only.as_deref().unwrap().contains("not valid TOML"));
        state.toggle(ClipboardField::CopyOnSelect);
        assert_eq!(state.current, values(true, RightClick::Paste));
        assert!(!state.can_apply() && !state.enabled(Btn::Apply) && state.enabled(Btn::Cancel));
        assert!(f.apply(&state).unwrap_err().contains("not valid TOML"));
        assert_eq!(f.text(), "[oops\ncopy_on_select = false\n");
        // An unreadable file is read-only with the read's error.
        let state = ClipboardState::new(&Err("reading /c/config.toml: denied".into()), ClipboardValues::default());
        assert_eq!(state.read_only.as_deref(), Some("reading /c/config.toml: denied"));
        assert!(state.describe(&None, "").contains(" can_apply=false "));
    }

    /// Apply with nothing changed (or changed back) closes, writing
    /// nothing.
    #[test]
    fn apply_with_no_change_closes() {
        let mut f = Fixture::new("# mine\ncopy_on_select = true\n");
        let mut state = f.state();
        assert_eq!(f.apply(&state), Ok(Step::Close));
        state.toggle(ClipboardField::RightClick);
        state.toggle(ClipboardField::RightClick);
        assert_eq!(f.apply(&state), Ok(Step::Close));
        assert_eq!(f.text(), "# mine\ncopy_on_select = true\n");
    }

    /// Only the switches that differ from the file are written: the spec's
    /// "Unchanged switch" and "Turn on right-click paste".
    #[test]
    fn apply_writes_only_the_changed_fields() {
        let text = "# My config\ntheme = \"brindle-dark\" # dark\n\n[font]\nsize = 13\n";
        let mut f = Fixture::new(text);
        let mut state = f.state();
        state.toggle(ClipboardField::CopyOnSelect);
        assert_eq!(state.apply(), Ok(Step::Write(ConfigEdit::SetClipboard(vec![ClipboardChange::CopyOnSelect(true)]))));
        assert!(matches!(f.apply(&state), Ok(Step::Write(_))));
        assert_eq!(f.text(), text.replace("# dark\n", "# dark\ncopy_on_select = true\n"));
        assert!(!f.text().contains("right_click"));

        let mut state = f.state();
        state.toggle(ClipboardField::RightClick);
        assert_eq!(
            state.apply(),
            Ok(Step::Write(ConfigEdit::SetClipboard(vec![ClipboardChange::RightClick(RightClick::Paste)])))
        );
        f.apply(&state).unwrap();
        assert_eq!(f.text(), text.replace("# dark\n", "# dark\ncopy_on_select = true\nright_click = \"paste\"\n"));
        let config = crate::config::Config::parse(&f.text()).unwrap().config;
        assert!(config.copy_on_select && config.right_click == RightClick::Paste);

        // Both, turned off again, in the form's order.
        let mut state = f.state();
        state.toggle(ClipboardField::RightClick);
        state.toggle(ClipboardField::CopyOnSelect);
        assert_eq!(
            state.apply(),
            Ok(Step::Write(ConfigEdit::SetClipboard(vec![
                ClipboardChange::CopyOnSelect(false),
                ClipboardChange::RightClick(RightClick::Nothing),
            ])))
        );
    }

    /// A key of the wrong kind stays as the file has it unless its own
    /// switch changed.
    #[test]
    fn a_wrongly_typed_key_is_kept_unless_its_switch_changed() {
        let mut f = Fixture::new("copy_on_select = \"yes\"\n");
        let mut state = f.state();
        state.toggle(ClipboardField::RightClick);
        f.apply(&state).unwrap();
        assert_eq!(f.text(), "copy_on_select = \"yes\"\nright_click = \"paste\"\n");
        let mut state = f.state();
        state.toggle(ClipboardField::CopyOnSelect);
        f.apply(&state).unwrap();
        assert_eq!(f.text(), "copy_on_select = true\nright_click = \"paste\"\n");
        assert_eq!(f.state().note(), None);
    }

    /// A reset puts the file's values back; a reload rereads the file.
    #[test]
    fn reset_and_reload_drop_the_changes() {
        let mut f = Fixture::new("");
        let mut state = f.state();
        state.toggle(ClipboardField::CopyOnSelect);
        state.reset();
        assert_eq!(state.current, ClipboardValues::default());
        state.toggle(ClipboardField::CopyOnSelect);
        f.text = Some("right_click = \"paste\"\n".into());
        state.reload(&f.read());
        assert_eq!(state.current, values(false, RightClick::Paste));
        assert_eq!(state.saved, state.current);
    }

    #[test]
    fn the_dump_line_names_both_values() {
        let mut state = Fixture::new("").state();
        assert_eq!(
            state.describe(&None, "Copy on select"),
            "--- settings section=Clipboard mode=form copy_on_select=false right_click=none field=\"Copy on select\" can_apply=true error=None\n"
        );
        state.toggle(ClipboardField::CopyOnSelect);
        state.toggle(ClipboardField::RightClick);
        assert_eq!(
            state.describe(&Some("denied".into()), "Right-click pastes"),
            "--- settings section=Clipboard mode=form copy_on_select=true right_click=paste field=\"Right-click pastes\" can_apply=true error=Some(\"denied\")\n"
        );
    }

    #[test]
    fn fields_are_labelled_and_listed_in_tab_order() {
        assert_eq!(CLIPBOARD_FIELDS.map(ClipboardField::label), ["Copy on select", "Right-click pastes"]);
        assert_eq!(
            CLIPBOARD_FIELDS.map(ClipboardField::hint),
            [None, Some("Pastes only when nothing is selected. Hold shift to paste where a program uses the mouse.")]
        );
        let on = values(true, RightClick::Paste);
        assert!(CLIPBOARD_FIELDS.iter().all(|&f| is_on(&on, f)));
        assert!(CLIPBOARD_FIELDS.iter().all(|&f| !is_on(&toggled(on, f), f) && is_on(&toggled(toggled(on, f), f), f)));
    }
}
