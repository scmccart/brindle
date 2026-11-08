//! `Command`: shadcn's command, a filterable list of items under group
//! headings, for palettes.
//!
//! An entity the owner creates with its groups and keeps, renders inline, in
//! a dialog or in a popover, and subscribes to: [`CommandEvent::Confirmed`]
//! carries the chosen item's index in declaration order across the groups
//! (`Command::item` finds it again), and [`CommandEvent::Cancelled`] reports
//! Escape. A filter [`TextInput`] behind an [`Icon::Search`] sits above the
//! list and has the focus; typing filters the items by [`filter`]'s
//! case-insensitive subsequence match, keeping their order, hiding groups
//! left with no items and showing a message when nothing matches. Each
//! change to the filter highlights the first match ([`CommandRows`]). A
//! group can take a separator above it ([`CommandGroup::separator`]),
//! shadcn's `CommandSeparator`: a line across the list, shown only while
//! the groups on both sides of it list items.
//!
//! The list is a `gpui::list`, since headings and items differ in height
//! and palettes can be long. Separators are rows too, but have no rank, so
//! the highlight passes over them. Up and down (also ctrl-p/ctrl-n) move the
//! highlight over the enabled matches and wrap at the ends; home and end go
//! to the text input's own bindings, which are nearer the focus. The pointer
//! highlights the item under it, and the highlight is scrolled into view
//! (`ListState::scroll_to_reveal_item` on the row [`CommandRows::row_of`]
//! gives). Enter or a click confirms the highlighted item.
//!
//! Escape is bound to `Dismiss` in the `UiCommand` context, nearer the focus
//! than an overlay's `UiLayer` binding, so the command list alone sees it and
//! emits `Cancelled`; a hosting `ManagedView` emits `DismissEvent` in
//! response, and an inline list still reports it. An outside press on a
//! hosting overlay dispatches `Dismiss` to the focused input and arrives the
//! same way.
//!
//! [`filter`]: crate::filter::filter

use gpui::{
    AnyElement, App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement,
    IntoElement, ListAlignment, ListSizingBehavior, ListState, MouseButton, MouseMoveEvent, ParentElement, Pixels,
    Render, SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div, list, prelude::FluentBuilder,
    px, rems,
};

use crate::actions::{Dismiss, ListConfirm, ListDown, ListFirst, ListLast, ListUp};
use crate::components::{DISABLED_OPACITY, inert_press};
use crate::filter::filter;
use crate::icon::Icon;
use crate::primitives::focus_visible::{Modality, set_modality};
use crate::primitives::nav::ListNav;
use crate::style::{
    COMMAND_LIST_MAX_HEIGHT, COMMAND_SEPARATOR_HEIGHT, MUTED_ICON_OPACITY, command_row_insets, command_style,
};
use crate::text_input::{TextInput, TextInputEvent};
use crate::theme::ActiveTheme;

// ---- pure --------------------------------------------------------------------

/// One item of a command list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandItem {
    pub label: SharedString,
    /// A line under the label, in the muted colour.
    pub detail: Option<SharedString>,
    /// Shown at the right, in the muted colour.
    pub shortcut: Option<SharedString>,
    /// A disabled item is listed muted and can't be highlighted or
    /// confirmed.
    pub disabled: bool,
}

impl CommandItem {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self { label: label.into(), detail: None, shortcut: None, disabled: false }
    }

    pub fn detail(mut self, detail: impl Into<SharedString>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn shortcut(mut self, shortcut: impl Into<SharedString>) -> Self {
        self.shortcut = Some(shortcut.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl From<&str> for CommandItem {
    fn from(label: &str) -> Self {
        Self::new(label.to_string())
    }
}

impl From<String> for CommandItem {
    fn from(label: String) -> Self {
        Self::new(label)
    }
}

impl From<SharedString> for CommandItem {
    fn from(label: SharedString) -> Self {
        Self::new(label)
    }
}

/// A group of items, under a heading or none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandGroup {
    pub heading: Option<SharedString>,
    pub items: Vec<CommandItem>,
    /// A separator above the group, between it and the group before:
    /// shadcn's `<CommandSeparator />` written before this
    /// `<CommandGroup>`. It shows only while both groups list items, so it
    /// does nothing on the first group.
    pub separator: bool,
}

impl CommandGroup {
    /// A group under `heading`.
    pub fn new(heading: impl Into<SharedString>, items: impl IntoIterator<Item = impl Into<CommandItem>>) -> Self {
        Self { heading: Some(heading.into()), items: items.into_iter().map(Into::into).collect(), separator: false }
    }

    /// A group with no heading.
    pub fn unnamed(items: impl IntoIterator<Item = impl Into<CommandItem>>) -> Self {
        Self { heading: None, items: items.into_iter().map(Into::into).collect(), separator: false }
    }

    /// Whether a separator sits above the group.
    pub fn separator(mut self, separator: bool) -> Self {
        self.separator = separator;
        self
    }
}

/// One row of the filtered list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandRow {
    /// The heading of the group at this index among the groups.
    Heading(usize),
    /// A matching item: its index across the groups, and its rank among
    /// the matches, which the highlight counts in.
    Item { item: usize, rank: usize },
    /// The separator above the group at this index among the groups. It
    /// belongs to no group and has no rank.
    Separator(usize),
}

/// The filtered list: its rows, the matching items and the highlight. Pure,
/// so the entity rebuilds it on each change to the filter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandRows {
    rows: Vec<CommandRow>,
    /// The group each row belongs to, by row; none for a separator.
    row_groups: Vec<Option<usize>>,
    /// The matching items' indices across the groups, by rank.
    matches: Vec<usize>,
    /// Whether each match is enabled, by rank.
    enabled: Vec<bool>,
    /// The highlight, as a rank. It wraps.
    nav: ListNav,
}

impl Default for CommandRows {
    fn default() -> Self {
        Self::build(&[], "")
    }
}

impl CommandRows {
    /// Filters `groups` by `query` (`filter`'s subsequence match, in order).
    /// A group with no matching item is left out, heading and all; a group
    /// with no heading lists its items alone. A group's separator is listed
    /// above it only when the group just before it lists items too, so a
    /// separator is never first, last or next to another. The first enabled
    /// match is highlighted.
    pub fn build(groups: &[CommandGroup], query: &str) -> Self {
        let mut rows = Vec::new();
        let mut row_groups = Vec::new();
        let mut matches = Vec::new();
        let mut enabled = Vec::new();
        let mut first = 0;
        // Whether the group before the current one listed items.
        let mut previous_listed = false;
        for (group_ix, group) in groups.iter().enumerate() {
            let hits = filter(query, group.items.iter().map(|item| item.label.as_ref()));
            let listed = !hits.is_empty();
            if listed {
                if group.separator && previous_listed {
                    rows.push(CommandRow::Separator(group_ix));
                    row_groups.push(None);
                }
                if group.heading.is_some() {
                    rows.push(CommandRow::Heading(group_ix));
                    row_groups.push(Some(group_ix));
                }
                for hit in hits {
                    let item = first + hit;
                    rows.push(CommandRow::Item { item, rank: matches.len() });
                    row_groups.push(Some(group_ix));
                    matches.push(item);
                    enabled.push(!group.items[hit].disabled);
                }
            }
            previous_listed = listed;
            first += group.items.len();
        }
        let mut nav = ListNav::new(matches.len(), true);
        nav.first(|rank| enabled[rank]);
        Self { rows, row_groups, matches, enabled, nav }
    }

    pub fn rows(&self) -> &[CommandRow] {
        &self.rows
    }

    /// How many rows the list shows, headings and separators included.
    pub fn len(&self) -> usize {
        self.rows.len()
    }

    /// Whether nothing matches, when the list shows its empty message and
    /// no heading.
    pub fn is_empty(&self) -> bool {
        self.matches.is_empty()
    }

    /// Whether `row` is the first and whether it is the last listed row of
    /// its group, which take the group's padding above and below it. None
    /// for a separator, which belongs to no group, and past the end.
    pub fn group_edges(&self, row: usize) -> Option<(bool, bool)> {
        let group = (*self.row_groups.get(row)?)?;
        let first = row == 0 || self.row_groups[row - 1] != Some(group);
        let last = self.row_groups.get(row + 1).is_none_or(|&next| next != Some(group));
        Some((first, last))
    }

    /// The matching items' indices across the groups, by rank.
    pub fn matches(&self) -> &[usize] {
        &self.matches
    }

    /// The highlight's rank among the matches.
    pub fn highlight(&self) -> Option<usize> {
        self.nav.highlight
    }

    /// The highlighted item's index across the groups, if it is enabled.
    pub fn highlighted_item(&self) -> Option<usize> {
        let rank = self.nav.highlight.filter(|&rank| self.enabled[rank])?;
        Some(self.matches[rank])
    }

    /// The row of the match ranked `rank`: what to scroll into view.
    pub fn row_of(&self, rank: usize) -> Option<usize> {
        self.rows.iter().position(|row| matches!(row, CommandRow::Item { rank: r, .. } if *r == rank))
    }

    /// The highlighted row, to scroll into view.
    pub fn highlighted_row(&self) -> Option<usize> {
        self.row_of(self.nav.highlight?)
    }

    /// Highlights the match ranked `rank`, as the pointer does. Returns
    /// whether that changed anything.
    pub fn set_highlight(&mut self, rank: usize) -> bool {
        if rank >= self.matches.len() || !self.enabled[rank] || self.nav.highlight == Some(rank) {
            return false;
        }
        self.nav.highlight = Some(rank);
        true
    }

    /// Moves the highlight with `step`; returns the row to scroll into view.
    fn step(&mut self, step: impl FnOnce(&mut ListNav, &dyn Fn(usize) -> bool) -> Option<usize>) -> Option<usize> {
        let enabled = &self.enabled;
        let rank = step(&mut self.nav, &|rank| enabled[rank])?;
        self.row_of(rank)
    }

    /// Highlights the previous enabled match, round the top.
    pub fn up(&mut self) -> Option<usize> {
        self.step(|nav, enabled| nav.prev(enabled))
    }

    /// Highlights the next enabled match, round the bottom.
    pub fn down(&mut self) -> Option<usize> {
        self.step(|nav, enabled| nav.next(enabled))
    }

    pub fn first(&mut self) -> Option<usize> {
        self.step(|nav, enabled| nav.first(enabled))
    }

    pub fn last(&mut self) -> Option<usize> {
        self.step(|nav, enabled| nav.last(enabled))
    }
}

// ---- entity ------------------------------------------------------------------

/// What a command list tells its owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CommandEvent {
    /// Enter, or a click, on the item with this index across the groups.
    Confirmed(usize),
    /// Escape.
    Cancelled,
}

/// The default placeholder, shadcn's.
const PLACEHOLDER: &str = "Type a command or search…";

/// The default empty message, shadcn's.
const EMPTY_MESSAGE: &str = "No results found.";

/// How far past the visible rows the list measures, so the first frame of
/// an opening list (measured within this before it has bounds) is as tall
/// as its cap at the 16px root size.
const LIST_OVERDRAW: Pixels = px(600.);

/// A command list. The owner keeps the entity, renders it and subscribes to
/// [`CommandEvent`]. Focus is the filter input's.
pub struct Command {
    groups: Vec<CommandGroup>,
    filter: Entity<TextInput>,
    rows: CommandRows,
    list: ListState,
    empty_message: SharedString,
    _filter_changes: Subscription,
}

impl EventEmitter<CommandEvent> for Command {}

impl Focusable for Command {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.filter.read(cx).focus_handle(cx)
    }
}

impl Command {
    pub fn new(groups: Vec<CommandGroup>, cx: &mut Context<Self>) -> Self {
        let filter = cx.new(|cx| TextInput::new("", cx).with_placeholder(PLACEHOLDER));
        let subscription = cx.subscribe(&filter, |this, _, _: &TextInputEvent, cx| this.refilter(cx));
        let rows = CommandRows::build(&groups, "");
        let list = ListState::new(rows.len(), ListAlignment::Top, LIST_OVERDRAW).measure_all();
        Self { groups, filter, rows, list, empty_message: EMPTY_MESSAGE.into(), _filter_changes: subscription }
    }

    /// The filter input's placeholder.
    pub fn with_placeholder(self, placeholder: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        let placeholder = placeholder.into();
        self.filter.update(cx, |input, cx| input.set_placeholder(placeholder, cx));
        self
    }

    /// The message shown when nothing matches.
    pub fn with_empty_message(mut self, message: impl Into<SharedString>) -> Self {
        self.empty_message = message.into();
        self
    }

    pub fn groups(&self) -> &[CommandGroup] {
        &self.groups
    }

    /// The item at `ix` across the groups, in declaration order: what
    /// [`CommandEvent::Confirmed`] names.
    pub fn item(&self, ix: usize) -> Option<&CommandItem> {
        self.groups.iter().flat_map(|group| &group.items).nth(ix)
    }

    /// Replaces the groups and filters them by the current query.
    pub fn set_groups(&mut self, groups: Vec<CommandGroup>, cx: &mut Context<Self>) {
        self.groups = groups;
        self.refilter(cx);
    }

    /// The filter input, for an owner that reads its text or feeds it.
    pub fn filter_input(&self) -> &Entity<TextInput> {
        &self.filter
    }

    pub fn query(&self, cx: &App) -> String {
        self.filter.read(cx).text().to_string()
    }

    /// Sets the filter from the owner, as typing would.
    pub fn set_query(&mut self, query: &str, cx: &mut Context<Self>) {
        self.filter.update(cx, |input, cx| input.set_text(query, cx));
        self.refilter(cx);
    }

    /// The filtered rows and the highlight.
    pub fn rows(&self) -> &CommandRows {
        &self.rows
    }

    /// The highlighted item's index across the groups.
    pub fn highlighted(&self) -> Option<usize> {
        self.rows.highlighted_item()
    }

    /// Rebuilds the rows for the current query: the first match is
    /// highlighted and the list starts at the top.
    fn refilter(&mut self, cx: &mut Context<Self>) {
        let query = self.filter.read(cx).text().to_string();
        self.rows = CommandRows::build(&self.groups, &query);
        self.list.reset(self.rows.len());
        cx.notify();
    }

    fn reveal(&mut self, row: Option<usize>, cx: &mut Context<Self>) {
        if let Some(row) = row {
            self.list.scroll_to_reveal_item(row);
        }
        cx.notify();
    }

    fn up(&mut self, _: &ListUp, _: &mut Window, cx: &mut Context<Self>) {
        let row = self.rows.up();
        self.reveal(row, cx);
    }

    fn down(&mut self, _: &ListDown, _: &mut Window, cx: &mut Context<Self>) {
        let row = self.rows.down();
        self.reveal(row, cx);
    }

    fn first(&mut self, _: &ListFirst, _: &mut Window, cx: &mut Context<Self>) {
        let row = self.rows.first();
        self.reveal(row, cx);
    }

    fn last(&mut self, _: &ListLast, _: &mut Window, cx: &mut Context<Self>) {
        let row = self.rows.last();
        self.reveal(row, cx);
    }

    fn confirm(&mut self, _: &ListConfirm, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(item) = self.rows.highlighted_item() {
            cx.emit(CommandEvent::Confirmed(item));
        }
    }

    fn cancel(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(CommandEvent::Cancelled);
    }

    /// The pointer over the match ranked `rank` highlights it.
    fn hover(&mut self, rank: usize, cx: &mut Context<Self>) {
        if self.rows.set_highlight(rank) {
            cx.notify();
        }
    }

    /// One row of the list, built when the list lays it out, inside its
    /// group's padding ([`command_row_insets`]). gpui's list lays each row
    /// out on its own at the list's width and ignores the list's horizontal
    /// padding, so the row brings the group's, and fills the width between
    /// (a flex row laid out alone would shrink to its content).
    ///
    /// A separator is in no group and takes no padding: CommandSeparator's
    /// `-mx-1 h-px bg-border` spans the list past the groups' `p-1`, and the
    /// groups beside it bring the space above and below.
    fn render_row(&mut self, row: usize, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        if let Some(CommandRow::Separator(_)) = self.rows.rows().get(row) {
            let style = command_style(cx.ui_theme());
            return div().w_full().h(COMMAND_SEPARATOR_HEIGHT).bg(style.separator).into_any_element();
        }
        let Some((first, last)) = self.rows.group_edges(row) else {
            // The list asks for a row the filter has since removed.
            return div().into_any_element();
        };
        let insets = command_row_insets(first, last);
        div()
            .w_full()
            .px(insets.x)
            .pt(insets.top)
            .pb(insets.bottom)
            .child(self.render_row_content(row, window, cx))
            .into_any_element()
    }

    /// A row's heading or item, filling the group's inner width.
    fn render_row_content(&mut self, row: usize, _: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let style = command_style(cx.ui_theme());
        match self.rows.rows().get(row) {
            // `render_row` draws separators.
            None | Some(CommandRow::Separator(_)) => div().into_any_element(),
            // CommandGroup's heading: px-2 py-1.5 text-xs font-medium
            // text-muted-foreground.
            Some(CommandRow::Heading(group)) => div()
                .w_full()
                .px_2()
                .py_1p5()
                .text_size(rems(0.75))
                .font_weight(FontWeight::MEDIUM)
                .text_color(style.muted)
                .whitespace_nowrap()
                .truncate()
                .children(self.groups.get(*group).and_then(|group| group.heading.clone()))
                .into_any_element(),
            // CommandItem: flex items-center gap-2 rounded-sm px-2 py-1.5
            // text-sm, data-[selected=true]:bg-accent
            // data-[selected=true]:text-accent-foreground,
            // data-[disabled=true]:opacity-50; CommandShortcut ml-auto
            // text-xs text-muted-foreground.
            Some(CommandRow::Item { item: item_ix, rank }) => {
                let (item_ix, rank) = (*item_ix, *rank);
                let Some(item) = self.item(item_ix) else {
                    return div().into_any_element();
                };
                let highlighted = self.rows.highlight() == Some(rank);
                let disabled = item.disabled;
                let (label, detail, shortcut) = (item.label.clone(), item.detail.clone(), item.shortcut.clone());
                div()
                    .id(("ui-command-item", item_ix))
                    .w_full()
                    .flex()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1p5()
                    .rounded(style.item_radius)
                    .text_size(rems(0.875))
                    .whitespace_nowrap()
                    .when(highlighted, |d| d.bg(style.highlight).text_color(style.highlight_foreground))
                    // A press keeps focus in the filter, rather than letting
                    // it bubble to an ancestor that tracks focus.
                    .when(disabled, |d| inert_press(d.opacity(DISABLED_OPACITY)))
                    .when(!disabled, |d| {
                        d.on_mouse_move(cx.listener(move |this, _: &MouseMoveEvent, _, cx| this.hover(rank, cx)))
                            // A press is pointer input, so whatever a host
                            // gives focus back to after the confirmation
                            // shows no ring.
                            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                                set_modality(window, Modality::Pointer, cx);
                                window.prevent_default();
                            })
                            .on_click(cx.listener(move |_, _, _, cx| cx.emit(CommandEvent::Confirmed(item_ix))))
                    })
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_w_0()
                            .child(div().truncate().child(label))
                            .children(detail.map(|detail| {
                                div().text_size(rems(0.75)).text_color(style.muted).truncate().child(detail)
                            })),
                    )
                    .children(shortcut.map(|shortcut| {
                        div().flex_none().ml_auto().text_size(rems(0.75)).text_color(style.muted).child(shortcut)
                    }))
                    .into_any_element()
            }
        }
    }
}

impl Render for Command {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let style = command_style(cx.ui_theme());
        let empty = self.rows.is_empty();
        // CommandList: max-h-[300px] overflow-y-auto. Each row carries its
        // group's p-1 (`render_row`). The list sizes itself to its rows up
        // to the cap.
        let rows = (!empty).then(|| {
            list(self.list.clone(), cx.processor(Self::render_row))
                .w_full()
                .with_sizing_behavior(ListSizingBehavior::Infer)
                .max_h(COMMAND_LIST_MAX_HEIGHT)
        });

        // Command: bg-popover text-popover-foreground flex flex-col
        // overflow-hidden rounded-md. CommandInput: flex h-9 items-center
        // gap-2 border-b px-3, a size-4 search glass at opacity-50, the
        // input text-sm. CommandEmpty: py-6 text-center text-sm.
        div()
            .key_context("UiCommand")
            .on_action(cx.listener(Self::cancel))
            .flex()
            .flex_col()
            .w_full()
            .overflow_hidden()
            .rounded(style.radius)
            .bg(style.background)
            .text_color(style.foreground)
            .child(
                div()
                    .key_context("UiList")
                    .on_action(cx.listener(Self::up))
                    .on_action(cx.listener(Self::down))
                    .on_action(cx.listener(Self::first))
                    .on_action(cx.listener(Self::last))
                    .on_action(cx.listener(Self::confirm))
                    .flex()
                    .flex_col()
                    .w_full()
                    .child(
                        div()
                            .flex()
                            .h_9()
                            .items_center()
                            .gap_2()
                            .px_3()
                            .border_b_1()
                            .border_color(style.border)
                            .child(div().flex_none().opacity(MUTED_ICON_OPACITY).child(Icon::Search.size(rems(1.))))
                            .child(div().flex_1().min_w_0().text_size(rems(0.875)).child(self.filter.clone())),
                    )
                    .children(rows)
                    .when(empty, |d| {
                        d.child(
                            div()
                                .py_6()
                                .text_center()
                                .text_size(rems(0.875))
                                .child(self.empty_message.clone()),
                        )
                    }),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use CommandRow::{Heading, Item, Separator};

    fn groups() -> Vec<CommandGroup> {
        vec![
            CommandGroup::new("Tabs", ["New Tab", "Close Tab"]),
            CommandGroup::new("Panes", ["Split Right", "Split Down"]),
            CommandGroup::unnamed(["Reload Config"]),
        ]
    }

    /// The spec's scenario: `spr` leaves `Split Right` alone, highlighted.
    #[test]
    fn filtering_hides_empty_groups_and_highlights_the_first_match() {
        let rows = CommandRows::build(&groups(), "spr");
        // The Tabs group and the unnamed one have no match, so no heading
        // and no item of theirs is listed.
        assert_eq!(rows.rows(), [Heading(1), Item { item: 2, rank: 0 }]);
        assert_eq!(rows.matches(), [2]);
        assert_eq!(rows.highlight(), Some(0));
        assert_eq!(rows.highlighted_item(), Some(2));
        assert_eq!(rows.highlighted_row(), Some(1));
        assert!(!rows.is_empty());
        assert_eq!(rows.len(), 2);
        // The plain scenario, with no headings at all.
        let plain = [CommandGroup::unnamed(["Split Right", "Close Tab"])];
        let rows = CommandRows::build(&plain, "spr");
        assert_eq!(rows.rows(), [Item { item: 0, rank: 0 }]);
        assert_eq!(rows.highlighted_item(), Some(0));
    }

    /// No filter lists everything in order, headings where groups have
    /// them, with the first item highlighted.
    #[test]
    fn an_empty_query_lists_everything() {
        let rows = CommandRows::build(&groups(), "");
        assert_eq!(
            rows.rows(),
            [
                Heading(0),
                Item { item: 0, rank: 0 },
                Item { item: 1, rank: 1 },
                Heading(1),
                Item { item: 2, rank: 2 },
                Item { item: 3, rank: 3 },
                Item { item: 4, rank: 4 },
            ]
        );
        assert_eq!(rows.matches(), [0, 1, 2, 3, 4]);
        assert_eq!(rows.highlight(), Some(0));
        // A match in several groups keeps each group's heading.
        let rows = CommandRows::build(&groups(), "ta");
        assert_eq!(rows.rows(), [Heading(0), Item { item: 0, rank: 0 }, Item { item: 1, rank: 1 }]);
    }

    /// Each row knows whether it opens or closes its group, headings
    /// counted, so the group's padding goes above its first row and below
    /// its last; a filtered group's edges are its listed rows.
    #[test]
    fn rows_know_their_groups_edges() {
        let rows = CommandRows::build(&groups(), "");
        // Heading(0), New Tab, Close Tab, Heading(1), Split Right, Split Down, Reload Config.
        let edges: Vec<_> = (0..rows.len()).map(|row| rows.group_edges(row)).collect();
        assert_eq!(
            edges,
            [
                Some((true, false)),
                Some((false, false)),
                Some((false, true)),
                Some((true, false)),
                Some((false, false)),
                Some((false, true)),
                // The unnamed group's one item is both.
                Some((true, true)),
            ]
        );
        assert_eq!(rows.group_edges(7), None);
        // Filtered: Tabs keeps its heading and Close Tab, which is now last.
        let rows = CommandRows::build(&groups(), "clo");
        assert_eq!(rows.rows(), [Heading(0), Item { item: 1, rank: 0 }]);
        assert_eq!(rows.group_edges(0), Some((true, false)));
        assert_eq!(rows.group_edges(1), Some((false, true)));
        assert_eq!(CommandRows::default().group_edges(0), None);
    }

    /// The spec's scenario: nothing matches, so no heading shows and the
    /// empty message does.
    #[test]
    fn nothing_matches() {
        let rows = CommandRows::build(&groups(), "zzz");
        assert!(rows.is_empty());
        assert!(rows.rows().is_empty());
        assert_eq!(rows.highlight(), None);
        assert_eq!(rows.highlighted_item(), None);
        assert_eq!(rows.highlighted_row(), None);
        // Moving goes nowhere.
        let mut rows = rows;
        assert_eq!(rows.down(), None);
        assert_eq!(rows.up(), None);
        assert_eq!(CommandRows::default(), CommandRows::build(&[], ""));
    }

    /// The spec's scenario: down from the last item highlights the first.
    /// Each move returns the row to scroll into view, headings counted.
    #[test]
    fn navigation_wraps_over_the_matches_and_names_their_rows() {
        let mut rows = CommandRows::build(&groups(), "");
        assert_eq!(rows.last(), Some(6));
        assert_eq!(rows.highlighted_item(), Some(4));
        assert_eq!(rows.down(), Some(1));
        assert_eq!(rows.highlight(), Some(0));
        assert_eq!(rows.up(), Some(6));
        assert_eq!(rows.up(), Some(5));
        assert_eq!(rows.highlighted_item(), Some(3));
        // Over a heading: item 1 (row 2) down to item 2 (row 4).
        assert_eq!(rows.first(), Some(1));
        assert_eq!(rows.down(), Some(2));
        assert_eq!(rows.down(), Some(4));
        assert_eq!(rows.row_of(2), Some(4));
        assert_eq!(rows.row_of(9), None);
    }

    /// The pointer highlights an enabled match, and reports a change only
    /// when it is one; disabled items are skipped by the keys and the
    /// pointer, and are never confirmed.
    #[test]
    fn disabled_items_are_skipped() {
        let groups = [CommandGroup::unnamed([
            CommandItem::new("Attach").disabled(true),
            CommandItem::new("Detach"),
            CommandItem::new("Kill").disabled(true),
            CommandItem::new("Rename"),
        ])];
        let mut rows = CommandRows::build(&groups, "");
        // The first enabled match is highlighted, not the first match.
        assert_eq!(rows.highlight(), Some(1));
        assert_eq!(rows.down(), Some(3));
        assert_eq!(rows.down(), Some(1));
        assert_eq!(rows.last(), Some(3));
        assert_eq!(rows.first(), Some(1));
        assert!(rows.set_highlight(3));
        assert!(!rows.set_highlight(3));
        assert!(!rows.set_highlight(2));
        assert!(!rows.set_highlight(7));
        assert_eq!(rows.highlighted_item(), Some(3));
        // All disabled: nothing is highlighted.
        let all = [CommandGroup::unnamed([CommandItem::new("Attach").disabled(true)])];
        let rows = CommandRows::build(&all, "");
        assert_eq!(rows.highlight(), None);
        assert!(!rows.is_empty());
    }

    /// `groups()` with a separator above Panes and above the unnamed group,
    /// as shadcn writes `<CommandSeparator />` between groups.
    fn separated() -> Vec<CommandGroup> {
        let mut groups = groups();
        groups[1].separator = true;
        groups[2].separator = true;
        groups
    }

    /// The spec's scenario: with no filter, a separator sits between Tabs'
    /// last row and Panes' heading, and another above the unnamed group.
    #[test]
    fn separators_list_between_groups() {
        let rows = CommandRows::build(&separated(), "");
        assert_eq!(
            rows.rows(),
            [
                Heading(0),
                Item { item: 0, rank: 0 },
                Item { item: 1, rank: 1 },
                Separator(1),
                Heading(1),
                Item { item: 2, rank: 2 },
                Item { item: 3, rank: 3 },
                Separator(2),
                Item { item: 4, rank: 4 },
            ]
        );
        // Separators are rows, not matches.
        assert_eq!(rows.matches(), [0, 1, 2, 3, 4]);
        assert_eq!(rows.len(), 9);
        // A separator belongs to no group, so the groups beside it keep
        // their padding at that edge: Close Tab is Tabs' last row, the Panes
        // heading its first.
        let edges: Vec<_> = (0..rows.len()).map(|row| rows.group_edges(row)).collect();
        assert_eq!(
            edges,
            [
                Some((true, false)),
                Some((false, false)),
                Some((false, true)),
                None,
                Some((true, false)),
                Some((false, false)),
                Some((false, true)),
                None,
                Some((true, true)),
            ]
        );
        // Without the flags, nothing changes from before.
        assert!(!CommandRows::build(&groups(), "").rows().iter().any(|row| matches!(row, Separator(_))));
    }

    /// The spec's scenario: a separator shows only while the groups on both
    /// sides of it list items, so a filter that empties either hides it.
    #[test]
    fn separators_hide_beside_an_empty_group() {
        // Panes only: Tabs above its separator is gone, so the Panes heading
        // is first, and the unnamed group below the other is gone too.
        let rows = CommandRows::build(&separated(), "spr");
        assert_eq!(rows.rows(), [Heading(1), Item { item: 2, rank: 0 }]);
        assert_eq!(rows.highlighted_row(), Some(1));
        // Tabs and the unnamed group, but not Panes between them: both
        // separators are beside Panes, so neither shows.
        let rows = CommandRows::build(&separated(), "e");
        assert_eq!(
            rows.rows(),
            [Heading(0), Item { item: 0, rank: 0 }, Item { item: 1, rank: 1 }, Item { item: 4, rank: 2 }]
        );
        // Tabs and Panes: one separator, and none last.
        let rows = CommandRows::build(&separated(), "t");
        assert_eq!(
            rows.rows(),
            [
                Heading(0),
                Item { item: 0, rank: 0 },
                Item { item: 1, rank: 1 },
                Separator(1),
                Heading(1),
                Item { item: 2, rank: 2 },
                Item { item: 3, rank: 3 },
            ]
        );
        // The unnamed group alone: no separator first.
        let rows = CommandRows::build(&separated(), "rel");
        assert_eq!(rows.rows(), [Item { item: 4, rank: 0 }]);
        // Nothing matches: nothing, separators included, is listed.
        assert!(CommandRows::build(&separated(), "zzz").rows().is_empty());
        // A separator on the first group has nothing above it.
        let first = [CommandGroup::new("Tabs", ["New Tab"]).separator(true)];
        assert_eq!(CommandRows::build(&first, "").rows(), [Heading(0), Item { item: 0, rank: 0 }]);
        // A group with no items lists nothing, and neither does the
        // separator above it or the one below.
        let empty = [
            CommandGroup::new("Tabs", ["New Tab"]),
            CommandGroup::new("None", Vec::<CommandItem>::new()).separator(true),
            CommandGroup::unnamed(["Reload Config"]).separator(true),
        ];
        assert_eq!(
            CommandRows::build(&empty, "").rows(),
            [Heading(0), Item { item: 0, rank: 0 }, Item { item: 1, rank: 1 }]
        );
    }

    /// Over many filters of a list where every group has a separator, a
    /// separator is never first or last, never beside another, and always
    /// stands between rows of the group before it and the group it is
    /// above.
    #[test]
    fn separators_are_never_first_last_or_doubled() {
        let mut groups = vec![
            CommandGroup::new("Tabs", ["New Tab", "Close Tab"]),
            CommandGroup::new("Panes", ["Split Right", "Split Down"]),
            CommandGroup::unnamed(["Reload Config"]),
            CommandGroup::new("Sessions", ["Attach", "Detach"]),
            CommandGroup::new("Empty", Vec::<CommandItem>::new()),
            CommandGroup::new("Window", ["Close Window"]),
        ];
        for group in &mut groups {
            group.separator = true;
        }
        let group_of = |rows: &CommandRows, row: usize| match rows.rows()[row] {
            Heading(group) => Some(group),
            Item { item, .. } => {
                let mut first = 0;
                groups.iter().position(|group| {
                    first += group.items.len();
                    item < first
                })
            }
            Separator(_) => None,
        };
        let queries = ["", "e", "t", "s", "spr", "rel", "a", "c", "d", "w", "o", "zzz", "ta", "cl"];
        for query in queries {
            let rows = CommandRows::build(&groups, query);
            let list = rows.rows();
            for (row, kind) in list.iter().enumerate() {
                let Separator(group) = *kind else { continue };
                assert!(row > 0 && row + 1 < list.len(), "{query:?}: a separator at the end, {list:?}");
                assert_eq!(group_of(&rows, row - 1), Some(group - 1), "{query:?}: {list:?}");
                assert_eq!(group_of(&rows, row + 1), Some(group), "{query:?}: {list:?}");
            }
            // Every pair of adjacent listed groups has its separator.
            let separators = list.iter().filter(|row| matches!(row, Separator(_))).count();
            let listed: Vec<_> = (0..list.len()).filter_map(|row| group_of(&rows, row)).collect();
            let adjacent = listed.windows(2).filter(|pair| pair[1] == pair[0] + 1).count();
            assert_eq!(separators, adjacent, "{query:?}: {list:?}");
        }
    }

    /// The spec's scenario: up and down pass over separators, which the
    /// highlight never lands on and `row_of` never names.
    #[test]
    fn navigation_passes_over_separators() {
        let mut rows = CommandRows::build(&separated(), "");
        assert_eq!(rows.highlighted_row(), Some(1));
        // Close Tab (row 2), then over the separator and Panes' heading to
        // Split Right (row 5).
        assert_eq!(rows.down(), Some(2));
        assert_eq!(rows.down(), Some(5));
        assert_eq!(rows.highlighted_item(), Some(2));
        assert_eq!(rows.up(), Some(2));
        // Split Down (row 6), then over the second separator to Reload
        // Config (row 8), and round to New Tab.
        assert_eq!(rows.down(), Some(5));
        assert_eq!(rows.down(), Some(6));
        assert_eq!(rows.down(), Some(8));
        assert_eq!(rows.highlighted_item(), Some(4));
        assert_eq!(rows.down(), Some(1));
        assert_eq!(rows.up(), Some(8));
        assert_eq!(rows.last(), Some(8));
        assert_eq!(rows.first(), Some(1));
        for rank in 0..rows.matches().len() {
            let row = rows.row_of(rank).expect("every match has a row");
            assert!(matches!(rows.rows()[row], Item { rank: r, .. } if r == rank), "rank {rank} names row {row}");
        }
        // Disabled items beside a separator are skipped too.
        let groups = [
            CommandGroup::new("Tabs", [CommandItem::new("New Tab"), CommandItem::new("Close Tab").disabled(true)]),
            CommandGroup::new("Panes", [CommandItem::new("Split Right").disabled(true), CommandItem::new("Split Down")])
                .separator(true),
        ];
        let mut rows = CommandRows::build(&groups, "");
        assert_eq!(rows.rows()[3], Separator(1));
        assert_eq!(rows.down(), Some(6));
        assert_eq!(rows.highlighted_item(), Some(3));
        assert_eq!(rows.down(), Some(1));
    }

    #[test]
    fn items_and_groups_from_labels() {
        let item: CommandItem = "Close Tab".into();
        assert_eq!(item, CommandItem::new("Close Tab"));
        let item = CommandItem::new("Split Right").detail("tmux").shortcut("ctrl-shift-r");
        assert_eq!(item.detail.as_ref().map(|s| s.as_ref()), Some("tmux"));
        assert_eq!(item.shortcut.as_ref().map(|s| s.as_ref()), Some("ctrl-shift-r"));
        assert!(!item.disabled);
        let group = CommandGroup::new("Panes", [item.clone()]);
        assert_eq!(group.heading.as_ref().map(|s| s.as_ref()), Some("Panes"));
        assert_eq!(group.items, [item]);
        assert_eq!(CommandGroup::unnamed(["a"]).heading, None);
        // No separator unless asked for.
        assert!(!group.separator);
        assert!(CommandGroup::unnamed(["a"]).separator(true).separator);
    }
}
