//! The palette: the view behind every palette Brindle shows (new tab from a
//! profile, the command palette, the sessions palette) and the one-line
//! prompt (`Rename Tab`).
//!
//! `Workspace` shows it in a modal `brindle_ui::Overlay`. Its list mode is a
//! `brindle_ui::Command` in `Dialog` chrome, as shadcn's `CommandDialog`
//! puts one (no padding, no visible header); its prompt mode is an `Input`
//! under the prompt's label, which is the dialog's title.
//!
//! Every way of closing reaches the owner as [`PaletteEvent::Dismissed`]:
//! escape and an outside press arrive as `Dismiss` (the command list's own
//! `Cancelled` in list mode, this view's handler in prompt mode). The view
//! never emits `DismissEvent`, so the overlay never closes behind the
//! owner's back.

use std::rc::Rc;

use gpui::{
    App, AppContext as _, ClickEvent, Context, CursorStyle, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString, Styled, Subscription, Window, div,
};

use crate::actions::PickerKill;
use brindle_ui::{
    Command, CommandEvent, CommandGroup, CommandItem, Dialog, Dismiss, FocusStops, Input, ListConfirm, TextInput,
};

pub enum PaletteEvent {
    /// A row was chosen; the index is into the rows the palette was built with.
    Confirmed(usize),
    /// `picker_kill` on a row (only the sessions palette acts on it).
    Kill(usize),
    Dismissed,
}

pub struct PaletteRow {
    pub label: SharedString,
    pub detail: Option<SharedString>,
    pub shortcut: Option<SharedString>,
}

impl PaletteRow {
    fn item(&self) -> CommandItem {
        CommandItem { label: self.label.clone(), detail: self.detail.clone(), shortcut: self.shortcut.clone(), disabled: false }
    }
}

/// The command list's one group: the rows, with no heading.
fn groups(rows: &[PaletteRow]) -> Vec<CommandGroup> {
    vec![CommandGroup::unnamed(rows.iter().map(PaletteRow::item))]
}

/// Runs a prompt's text; the palette closes once it has.
pub type SubmitFn = Rc<dyn Fn(String, &mut App)>;

/// A request to ask the user for one line of text.
#[derive(Clone)]
pub struct PromptRequest {
    pub label: SharedString,
    pub initial: String,
    pub submit: SubmitFn,
}

struct Prompt {
    label: SharedString,
    input: Entity<TextInput>,
    submit: SubmitFn,
}

struct List {
    rows: Vec<PaletteRow>,
    command: Entity<Command>,
}

enum Mode {
    List(List),
    Prompt(Prompt),
}

pub struct Palette {
    /// The view's own handle, tracked on its root: the overlay keeps it for
    /// the view's whole life, while the input with focus is the mode's.
    focus: FocusHandle,
    mode: Mode,
    _subscription: Option<Subscription>,
}

impl EventEmitter<PaletteEvent> for Palette {}
/// Required of an overlay's view; never emitted (see the module doc).
impl EventEmitter<DismissEvent> for Palette {}

impl Focusable for Palette {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// One stop, the active mode's input, so Tab stays on it.
impl FocusStops for Palette {
    fn stops(&self, cx: &App) -> Vec<FocusHandle> {
        vec![self.input(cx).focus_handle(cx)]
    }
}

impl Palette {
    /// A filterable list of `rows`, with `placeholder` in the empty filter.
    pub fn list(rows: Vec<PaletteRow>, placeholder: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self::list_with_empty(rows, placeholder, None, cx)
    }

    /// [`Palette::list`] with `empty` as the message shown when nothing is
    /// listed, instead of the command list's own.
    pub fn list_with_empty(
        rows: Vec<PaletteRow>,
        placeholder: impl Into<SharedString>,
        empty: Option<SharedString>,
        cx: &mut Context<Self>,
    ) -> Self {
        let placeholder = placeholder.into();
        let command = cx.new(|cx| {
            let mut command = Command::new(groups(&rows), cx).with_placeholder(placeholder, cx);
            command.set_empty_message(empty, cx);
            command
        });
        let subscription = cx.subscribe(&command, |_, _, event: &CommandEvent, cx| match *event {
            CommandEvent::Confirmed(ix) => cx.emit(PaletteEvent::Confirmed(ix)),
            CommandEvent::Cancelled => cx.emit(PaletteEvent::Dismissed),
        });
        Self { focus: cx.focus_handle(), mode: Mode::List(List { rows, command }), _subscription: Some(subscription) }
    }

    pub fn prompt(request: PromptRequest, cx: &mut Context<Self>) -> Self {
        // The pre-filled value is selected, so typing replaces it.
        let input = cx.new(|cx| {
            let mut input = TextInput::new(&request.initial, cx);
            input.select_all(cx);
            input
        });
        let prompt = Prompt { label: request.label, input, submit: request.submit };
        Self { focus: cx.focus_handle(), mode: Mode::Prompt(prompt), _subscription: None }
    }

    /// Replaces a list's rows and its empty message (`None`: the command
    /// list's own), keeping its filter text. The command list highlights
    /// the first match again.
    pub fn set_rows(&mut self, rows: Vec<PaletteRow>, empty: Option<SharedString>, cx: &mut Context<Self>) {
        if let Mode::List(list) = &mut self.mode {
            list.command.update(cx, |command, cx| {
                command.set_empty_message(empty, cx);
                command.set_groups(groups(&rows), cx);
            });
            list.rows = rows;
        }
    }

    /// The filter or prompt input.
    pub fn input(&self, cx: &App) -> Entity<TextInput> {
        match &self.mode {
            Mode::List(list) => list.command.read(cx).filter_input().clone(),
            Mode::Prompt(prompt) => prompt.input.clone(),
        }
    }

    /// Debug description for `--dump-screen-after`.
    pub fn describe(&self, cx: &App) -> String {
        match &self.mode {
            Mode::List(list) => {
                let command = list.command.read(cx);
                let mut out = format!("--- palette list query={:?}\n", command.query(cx));
                for &ix in command.rows().matches() {
                    let Some(row) = list.rows.get(ix) else { continue };
                    out.push_str(&row.label);
                    if let Some(detail) = &row.detail {
                        out.push_str(&format!("  {detail}"));
                    }
                    if let Some(shortcut) = &row.shortcut {
                        out.push_str(&format!("  [{shortcut}]"));
                    }
                    out.push('\n');
                }
                out
            }
            Mode::Prompt(p) => format!("--- palette prompt {:?} text={:?}\n", p.label, p.input.read(cx).text()),
        }
    }

    /// `picker_kill`: the highlighted row of a list.
    fn kill(&mut self, _: &PickerKill, _: &mut Window, cx: &mut Context<Self>) {
        if let Mode::List(list) = &self.mode
            && let Some(ix) = list.command.read(cx).highlighted()
        {
            cx.emit(PaletteEvent::Kill(ix));
        }
    }

    /// Enter in a prompt submits it, as enter in a one-field form does. A
    /// list confirms through its command list, which is nearer the focus.
    fn submit(&mut self, _: &ListConfirm, _: &mut Window, cx: &mut Context<Self>) {
        let Mode::Prompt(prompt) = &self.mode else { return };
        let text = prompt.input.read(cx).text().to_string();
        (prompt.submit.clone())(text, cx);
        cx.emit(PaletteEvent::Dismissed);
    }

    /// Escape or an outside press (a list's arrives as the command list's
    /// `Cancelled` instead).
    fn dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(PaletteEvent::Dismissed);
    }

    fn render_prompt(prompt: &Prompt, cx: &mut Context<Self>) -> Dialog {
        Dialog::new()
            .title(prompt.label.clone())
            .on_close(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(PaletteEvent::Dismissed)))
            .child(Input::new(&prompt.input))
    }
}

impl Render for Palette {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panel = match &self.mode {
            // CommandDialog: DialogContent with overflow-hidden p-0 around
            // the command list, its header for screen readers only and no
            // close button.
            Mode::List(list) => Dialog::new().p_0().gap_0().overflow_hidden().child(list.command.clone()),
            Mode::Prompt(prompt) => Self::render_prompt(prompt, cx),
        };
        div()
            .key_context("Palette")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::kill))
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::dismiss))
            .cursor(CursorStyle::Arrow)
            .child(panel)
    }
}
