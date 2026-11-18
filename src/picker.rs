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
//!
//! The sessions palette's state and rows are built purely, in [`sessions`]
//! ([`SessionsModel`], [`sessions::session_palette`]).

mod sessions;

use std::rc::Rc;

use gpui::{
    App, AppContext as _, ClickEvent, Context, CursorStyle, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString, Styled, Subscription, Window, div,
};

pub use sessions::{KillOutcome, Listing, OPEN_GRACE, SessionRef, SessionRow, SessionsModel};

use crate::actions::PickerKill;
use brindle_ui::{
    Command, CommandEvent, CommandGroup, CommandItem, Dialog, Dismiss, FocusStops, Input, ItemKey, ListConfirm,
    TextInput,
};

pub enum PaletteEvent {
    /// A row was chosen; the index is into the rows the palette was built
    /// with, across its groups in order.
    Confirmed(usize),
    /// `picker_kill` on a row (only the sessions palette acts on it).
    Kill(usize),
    Dismissed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteRow {
    pub label: SharedString,
    pub detail: Option<SharedString>,
    pub shortcut: Option<SharedString>,
    /// Listed muted, and can't be highlighted or chosen.
    pub disabled: bool,
}

impl PaletteRow {
    /// The row as the command list's item, keyed `key`.
    fn item(&self, key: ItemKey) -> CommandItem {
        CommandItem {
            key,
            label: self.label.clone(),
            detail: self.detail.clone(),
            shortcut: self.shortcut.clone(),
            disabled: self.disabled,
            checked: false,
        }
    }
}

/// The key of the row at `ix` across a palette's groups. Rows are keyed by
/// position for now (labels repeat: two sessions of one program), so a
/// kept row is the one at the index the caller computed against the new
/// groups, as before keys.
fn row_key(ix: usize) -> ItemKey {
    ItemKey::new(ix.to_string())
}

/// Rows under a heading, or none. A group after the first has a separator
/// above it, which the command list shows only while the groups on both
/// sides list rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteGroup {
    pub heading: Option<SharedString>,
    pub rows: Vec<PaletteRow>,
}

impl PaletteGroup {
    pub fn unnamed(rows: Vec<PaletteRow>) -> Self {
        Self { heading: None, rows }
    }

    pub fn named(heading: impl Into<SharedString>, rows: Vec<PaletteRow>) -> Self {
        Self { heading: Some(heading.into()), rows }
    }
}

/// The command list's groups, each row keyed by its index across them
/// ([`row_key`]).
fn command_groups(groups: &[PaletteGroup]) -> Vec<CommandGroup> {
    let mut row = 0;
    groups
        .iter()
        .enumerate()
        .map(|(ix, group)| CommandGroup {
            heading: group.heading.clone(),
            items: group
                .rows
                .iter()
                .map(|r| {
                    let item = r.item(row_key(row));
                    row += 1;
                    item
                })
                .collect(),
            separator: ix > 0,
        })
        .collect()
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
    groups: Vec<PaletteGroup>,
    command: Entity<Command>,
}

impl List {
    /// The row at `ix` across the groups, as the command list counts.
    fn row(&self, ix: usize) -> Option<&PaletteRow> {
        self.groups.iter().flat_map(|group| &group.rows).nth(ix)
    }
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
        Self::list_grouped(vec![PaletteGroup::unnamed(rows)], placeholder, None, cx)
    }

    /// [`Palette::list`] of rows in `groups`, with `empty` as the message
    /// shown when nothing is listed, instead of the command list's own.
    pub fn list_grouped(
        groups: Vec<PaletteGroup>,
        placeholder: impl Into<SharedString>,
        empty: Option<SharedString>,
        cx: &mut Context<Self>,
    ) -> Self {
        let placeholder = placeholder.into();
        let command = cx.new(|cx| {
            let mut command = Command::new(command_groups(&groups), cx).with_placeholder(placeholder, cx);
            command.set_empty_message(empty, cx);
            command
        });
        let subscription = cx.subscribe(&command, |_, _, event: &CommandEvent, cx| match *event {
            CommandEvent::Confirmed(ix) => cx.emit(PaletteEvent::Confirmed(ix)),
            CommandEvent::Cancelled => cx.emit(PaletteEvent::Dismissed),
        });
        Self { focus: cx.focus_handle(), mode: Mode::List(List { groups, command }), _subscription: Some(subscription) }
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

    /// Replaces a list's groups and its empty message (`None`: the command
    /// list's own), keeping its filter text. `keep` names the row to keep
    /// highlighted, by its index across the new groups (the list then also
    /// keeps its scroll offset); with `None` the command list highlights
    /// the first match again.
    pub fn set_groups(
        &mut self,
        groups: Vec<PaletteGroup>,
        empty: Option<SharedString>,
        keep: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        if let Mode::List(list) = &mut self.mode {
            list.command.update(cx, |command, cx| {
                command.set_empty_message(empty, cx);
                command.set_groups(command_groups(&groups), keep.map(row_key), cx);
            });
            list.groups = groups;
        }
    }

    /// A list's highlighted row, by its index across the groups.
    pub fn highlighted(&self, cx: &App) -> Option<usize> {
        match &self.mode {
            Mode::List(list) => list.command.read(cx).highlighted(),
            Mode::Prompt(_) => None,
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
                let highlighted = command
                    .highlighted()
                    .and_then(|item| command.rows().matches().iter().position(|&ix| ix == item))
                    .map_or_else(|| "none".to_string(), |at| at.to_string());
                let mut out = format!("--- palette list query={:?} highlighted={highlighted}\n", command.query(cx));
                for &ix in command.rows().matches() {
                    let Some(row) = list.row(ix) else { continue };
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Rows are keyed by their index across the groups, so a row kept by
    /// index is the one the command list keeps, even when labels repeat.
    #[test]
    fn rows_are_keyed_by_their_index_across_the_groups() {
        let row = |label: &str| PaletteRow { label: label.to_string().into(), detail: None, shortcut: None, disabled: false };
        let groups = [
            PaletteGroup::named("This machine", vec![row("bash"), row("bash")]),
            PaletteGroup::named("A8", vec![row("bash")]),
        ];
        let command = command_groups(&groups);
        let keys: Vec<&str> = command.iter().flat_map(|g| &g.items).map(|item| item.key.as_str()).collect();
        assert_eq!(keys, ["0", "1", "2"]);
        for ix in 0..3 {
            assert_eq!(brindle_ui::item_keyed(&command, &row_key(ix)), Some(ix));
        }
    }
}
