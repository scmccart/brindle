//! `Menu`: the content shadcn's DropdownMenu and ContextMenu share (Radix's
//! Menu), and the pieces its two openers, [`ContextMenu`] and
//! [`DropdownMenu`], are built from.
//!
//! A menu lists [`MenuEntry`]s: items, each with a label, an optional
//! description line under it (the crate's addition), an optional shortcut
//! drawn as [`Kbd`] keycaps and a disabled state, and separators between
//! groups. It is the view of a non-modal [`Overlay`] that blocks the
//! pointer outside it, as a select's menu: an outside press only closes
//! it. It emits [`MenuEvent::Selected`] with the entry's index (separators
//! counted) when an item is chosen; every other way out (Escape, an
//! outside press) is the overlay's `DismissEvent`. Focus goes back to what
//! had it when the menu opened, either way.
//!
//! Keys, as Radix's menu: the highlight starts on nothing. Down (also
//! ctrl-n), Home and PageUp highlight the first enabled item from there,
//! Up (ctrl-p), End and PageDown the last; after that up and down move over
//! the enabled items without wrapping ([`MenuNav`], on [`ListNav`]), home and
//! PageUp go to the first and end and PageDown to the last. Typing
//! highlights the next enabled item whose label starts with the typed text
//! ([`Typeahead`], labels only). Enter and space choose the highlighted
//! item; space continues a typeahead search instead while one is under way.
//! Tab does nothing (the menu lists no focus stops).
//!
//! Pointer, as Radix's menu with the [`Select`] threshold: moving over an
//! enabled item highlights it, and over anything else in the menu (a
//! disabled item, a separator, the padding) or out of it clears the
//! highlight. A primary release on an enabled item chooses it, wherever the
//! press came (a click, or a press elsewhere dragged onto it). The release
//! that ends the press which opened the menu ([`MenuPress`]: a primary
//! press on a dropdown's trigger, a secondary press for a context menu) is
//! ignored when the pointer moved no more than [`DRAG_THRESHOLD`] since the
//! press; moved further, it chooses the enabled item it lands on, so press,
//! drag and release is one gesture ([`menu_item_release`]). A release
//! anywhere else does nothing.
//!
//! [`ContextMenu`]: crate::components::context_menu::ContextMenu
//! [`DropdownMenu`]: crate::components::dropdown_menu::DropdownMenu
//! [`Select`]: crate::components::select::Select
//! [`DRAG_THRESHOLD`]: crate::components::select::DRAG_THRESHOLD

use std::time::Instant;

use gpui::{
    AnyElement, App, Bounds, Context, DismissEvent, Div, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    ParentElement, Pixels, Point, Render, ScrollHandle, SharedString, StatefulInteractiveElement, Styled,
    Subscription, WeakEntity, Window, div, prelude::FluentBuilder, px,
};

use crate::actions::{ListConfirm, ListDown, ListFirst, ListLast, ListUp};
use crate::components::DISABLED_OPACITY;
use crate::components::kbd::Kbd;
use crate::components::select::{MenuRelease, ReleaseOver, TypedKey, moved_off, typed_key};
use crate::primitives::focus_scope::FocusStops;
use crate::primitives::focus_visible::{Modality, set_modality};
use crate::primitives::nav::{ListNav, Typeahead};
use crate::primitives::overlay::Overlay;
use crate::primitives::placement::{Align, Anchored, EDGE_MARGIN, SIDE_OFFSET, Side, available_height};
use crate::style::{MENU_MIN_WIDTH, MenuStyle, menu_style};
use crate::theme::ActiveTheme;

// ---- pure --------------------------------------------------------------------

/// One item of a menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuItem {
    /// What the item says. Typeahead and the item's name use this alone.
    pub label: SharedString,
    /// A second line under the label, in the muted foreground.
    pub description: Option<SharedString>,
    /// The keys that do the same elsewhere, as the keymap prints them
    /// (`ctrl-shift-t`), drawn as keycaps after the label.
    pub shortcut: Option<SharedString>,
    /// A disabled item is drawn muted, skipped by the keys and never chosen.
    pub disabled: bool,
}

impl MenuItem {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self { label: label.into(), description: None, shortcut: None, disabled: false }
    }

    pub fn description(mut self, description: impl Into<SharedString>) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn shortcut(mut self, keys: impl Into<SharedString>) -> Self {
        self.shortcut = Some(keys.into());
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

/// A row of a menu: an item, or a separator between two groups of them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuEntry {
    Item(MenuItem),
    /// A one-pixel rule in the border colour. It is never highlighted.
    Separator,
}

impl MenuEntry {
    pub fn item(&self) -> Option<&MenuItem> {
        match self {
            MenuEntry::Item(item) => Some(item),
            MenuEntry::Separator => None,
        }
    }

    /// Whether the entry can be highlighted and chosen: an item that isn't
    /// disabled.
    pub fn is_enabled(&self) -> bool {
        self.item().is_some_and(|item| !item.disabled)
    }

    /// The item's label, or nothing for a separator.
    pub fn label(&self) -> &str {
        self.item().map_or("", |item| item.label.as_ref())
    }
}

impl From<MenuItem> for MenuEntry {
    fn from(item: MenuItem) -> Self {
        MenuEntry::Item(item)
    }
}

impl From<&str> for MenuEntry {
    fn from(label: &str) -> Self {
        MenuItem::new(label.to_string()).into()
    }
}

impl From<String> for MenuEntry {
    fn from(label: String) -> Self {
        MenuItem::new(label).into()
    }
}

impl From<SharedString> for MenuEntry {
    fn from(label: SharedString) -> Self {
        MenuItem::new(label).into()
    }
}

/// What a menu tells its owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuEvent {
    /// The entry at this index (separators counted) was chosen. The menu
    /// has closed and given focus back before the owner hears of it.
    Selected(usize),
}

/// An open menu's highlight and typeahead, over its entries. Pure: the
/// [`Menu`] holds one and passes its entries in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuNav {
    list: ListNav,
    typeahead: Typeahead,
}

impl MenuNav {
    /// The state of a menu of `count` entries that just opened: nothing is
    /// highlighted, and the highlight doesn't wrap (Radix's `loop` is off
    /// by default).
    pub fn new(count: usize) -> Self {
        Self { list: ListNav::new(count, false), typeahead: Typeahead::new() }
    }

    pub fn highlighted(&self) -> Option<usize> {
        self.list.highlight
    }

    /// Highlights `ix` (or nothing), as the pointer does. Returns whether
    /// that changed anything.
    pub fn set_highlight(&mut self, ix: Option<usize>) -> bool {
        let changed = self.list.highlight != ix;
        self.list.highlight = ix;
        changed
    }

    /// The next enabled item, or the first from nothing. Stays at the end.
    pub fn down(&mut self, entries: &[MenuEntry]) -> Option<usize> {
        self.list.next(enabled(entries))
    }

    /// The previous enabled item, or the last from nothing. Stays at the
    /// start.
    pub fn up(&mut self, entries: &[MenuEntry]) -> Option<usize> {
        self.list.prev(enabled(entries))
    }

    pub fn first(&mut self, entries: &[MenuEntry]) -> Option<usize> {
        self.list.first(enabled(entries))
    }

    pub fn last(&mut self, entries: &[MenuEntry]) -> Option<usize> {
        self.list.last(enabled(entries))
    }

    /// A character typed at `now`: it extends or starts the query, and the
    /// highlight moves to the next enabled item whose label starts with it
    /// (`Typeahead::find`; descriptions and shortcuts don't count). Returns
    /// the new highlight when there was a match.
    pub fn type_char(&mut self, ch: char, now: Instant, entries: &[MenuEntry]) -> Option<usize> {
        self.typeahead.push(ch, now);
        let labels: Vec<&str> = entries.iter().map(MenuEntry::label).collect();
        let found = self.typeahead.find(&labels, self.list.highlight, enabled(entries))?;
        self.list.highlight = Some(found);
        Some(found)
    }

    /// Whether a character typed at `now` extends the query, so a space
    /// continues the search instead of choosing.
    pub fn is_searching(&self, now: Instant) -> bool {
        self.typeahead.is_searching(now)
    }
}

/// A move of the highlight: one of `MenuNav`'s steps.
type Step = fn(&mut MenuNav, &[MenuEntry]) -> Option<usize>;

fn enabled(entries: &[MenuEntry]) -> impl Fn(usize) -> bool + '_ {
    move |ix| entries.get(ix).is_some_and(MenuEntry::is_enabled)
}

/// The pointer press that opened a menu: a primary press on a dropdown's
/// trigger, or a secondary press for a context menu. The menu keeps it
/// until that button's release, which ends the gesture.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MenuPress {
    pub button: MouseButton,
    pub position: Point<Pixels>,
}

/// What the release of `button` at `at`, `over` part of the window, does to
/// an open menu, given the press that opened it if that hasn't been
/// released yet (`press`).
///
/// - The release of the opening press, no more than [`DRAG_THRESHOLD`] from
///   where it began, does nothing, wherever it lands.
/// - Moved further, it chooses the enabled item it lands on, and does
///   nothing anywhere else. Radix's menu does the same (an item chooses on
///   any pointer-up that didn't follow a press on it), with no threshold.
/// - Any other primary release chooses the enabled item it lands on (a
///   click, or a press in the menu dragged onto an item).
/// - Any other release of another button does nothing.
///
/// A release never closes the menu without a choice; only Escape and an
/// outside press do that.
///
/// [`DRAG_THRESHOLD`]: crate::components::select::DRAG_THRESHOLD
pub fn menu_item_release(
    press: Option<MenuPress>,
    button: MouseButton,
    at: Point<Pixels>,
    over: ReleaseOver,
) -> MenuRelease {
    let gesture = press.filter(|press| press.button == button);
    let counts = match gesture {
        Some(press) => moved_off(press.position, at),
        None => button == MouseButton::Left,
    };
    match over {
        ReleaseOver::Option(ix) if counts => MenuRelease::Pick(ix),
        _ => MenuRelease::Stay,
    }
}

/// Where a menu goes: against a trigger's bounds on a side, start-aligned,
/// `offset` away. Its height is capped at the room that leaves
/// ([`available_height`]), so a long menu scrolls.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MenuAnchor {
    /// In window coordinates.
    pub trigger: Bounds<Pixels>,
    pub side: Side,
    pub offset: Pixels,
}

impl MenuAnchor {
    /// A context menu's: its top-left corner at `point`. A zero-size trigger
    /// there with the menu on its right, start-aligned, no offset, which
    /// is Radix's `side="right" align="start"` (with `sideOffset` 0): near
    /// the window's right edge it flips to the left of the point, and near
    /// the bottom it is shifted up into the window.
    pub fn at(point: Point<Pixels>) -> Self {
        Self { trigger: Bounds { origin: point, size: Default::default() }, side: Side::Right, offset: px(0.) }
    }

    /// A dropdown's: below `trigger`, start-aligned, shadcn's 4px away,
    /// flipping above when it doesn't fit.
    pub fn below(trigger: Bounds<Pixels>) -> Self {
        Self { trigger, side: Side::Bottom, offset: SIDE_OFFSET }
    }

    /// The tallest the menu may be in a window of `viewport`.
    pub fn max_height(&self, viewport: gpui::Size<Pixels>) -> Pixels {
        available_height(self.trigger, viewport, self.side, self.offset, EDGE_MARGIN)
    }

    /// `panel` placed against the trigger.
    pub fn place(self, panel: AnyElement) -> AnyElement {
        Anchored::new(self.trigger)
            .side(self.side)
            .align(Align::Start)
            .offset(self.offset)
            .child(panel)
            .into_any_element()
    }
}

// ---- the view ----------------------------------------------------------------

/// An open menu: the view an opener's overlay shows. Its owner reads it to
/// show what is open (`entries`, `highlighted`); the openers create it.
pub struct Menu {
    entries: Vec<MenuEntry>,
    nav: MenuNav,
    focus: FocusHandle,
    scroll: ScrollHandle,
    pub(crate) anchor: MenuAnchor,
    /// The press that opened the menu, until its release.
    press: Option<MenuPress>,
}

impl EventEmitter<MenuEvent> for Menu {}
impl EventEmitter<DismissEvent> for Menu {}

impl Focusable for Menu {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// No stops: the overlay focuses the menu itself, and Tab does nothing in
/// it, as Radix's menu prevents Tab.
impl FocusStops for Menu {
    fn stops(&self, _: &App) -> Vec<FocusHandle> {
        Vec::new()
    }
}

impl Menu {
    /// A menu of `entries` placed by `anchor`, highlighting nothing.
    /// `press` is the pointer press that opened it, if one did; its
    /// release ends the gesture. The openers create menus; an app that
    /// keeps its own overlay slot for menus creates them here, opens them
    /// in a [`menu_overlay`] and draws them with [`render_menu`].
    pub fn new(
        entries: Vec<MenuEntry>,
        anchor: MenuAnchor,
        press: Option<MenuPress>,
        cx: &mut Context<Self>,
    ) -> Self {
        let nav = MenuNav::new(entries.len());
        Self { entries, nav, focus: cx.focus_handle(), scroll: ScrollHandle::new(), anchor, press }
    }

    pub fn entries(&self) -> &[MenuEntry] {
        &self.entries
    }

    /// The highlighted entry, or `None` (as when the menu opens).
    pub fn highlighted(&self) -> Option<usize> {
        self.nav.highlighted()
    }

    /// The panel's painted bounds in window coordinates, from the last
    /// frame; `None` before it painted.
    pub fn bounds(&self) -> Option<Bounds<Pixels>> {
        let bounds = self.scroll.bounds();
        (bounds.size.width > px(0.)).then_some(bounds)
    }

    /// Where entry `ix` was painted in the last frame, in window
    /// coordinates, scrolled as the panel shows it (so a row scrolled out
    /// of view lies outside [`Menu::bounds`]).
    pub fn entry_bounds(&self, ix: usize) -> Option<Bounds<Pixels>> {
        let bounds = self.scroll.bounds_for_item(ix)?;
        Some(Bounds { origin: bounds.origin + self.scroll.offset(), size: bounds.size })
    }

    /// The entry under `at`, if any.
    fn entry_at(&self, at: Point<Pixels>) -> Option<usize> {
        (0..self.entries.len()).find(|&ix| self.entry_bounds(ix).is_some_and(|bounds| bounds.contains(&at)))
    }

    /// The enabled item under `at`, if any.
    fn item_at(&self, at: Point<Pixels>) -> Option<usize> {
        self.entry_at(at).filter(|&ix| self.entries[ix].is_enabled())
    }

    /// Chooses entry `ix` if it is an enabled item.
    fn select(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.entries.get(ix).is_some_and(MenuEntry::is_enabled) {
            cx.emit(MenuEvent::Selected(ix));
        }
    }

    fn select_highlighted(&mut self, cx: &mut Context<Self>) {
        if let Some(ix) = self.nav.highlighted() {
            self.select(ix, cx);
        }
    }

    fn set_highlight(&mut self, ix: Option<usize>, cx: &mut Context<Self>) {
        if self.nav.set_highlight(ix) {
            cx.notify();
        }
    }

    /// Moves the highlight with `step` and scrolls it into view.
    fn move_highlight(&mut self, step: Step, cx: &mut Context<Self>) {
        if let Some(ix) = step(&mut self.nav, &self.entries) {
            self.scroll.scroll_to_item(ix);
        }
        cx.notify();
    }

    fn up(&mut self, _: &ListUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_highlight(MenuNav::up, cx);
    }

    fn down(&mut self, _: &ListDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_highlight(MenuNav::down, cx);
    }

    fn first(&mut self, _: &ListFirst, _: &mut Window, cx: &mut Context<Self>) {
        self.move_highlight(MenuNav::first, cx);
    }

    fn last(&mut self, _: &ListLast, _: &mut Window, cx: &mut Context<Self>) {
        self.move_highlight(MenuNav::last, cx);
    }

    fn confirm(&mut self, _: &ListConfirm, _: &mut Window, cx: &mut Context<Self>) {
        self.select_highlighted(cx);
    }

    /// PageUp and PageDown (Radix's), typeahead and space. A key taken here
    /// is stopped, which keeps it from the keystroke observer that records
    /// keyboard input, so it is recorded here.
    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let now = Instant::now();
        let keystroke = &event.keystroke;
        let page: Option<Step> = match keystroke.key.as_str() {
            "pageup" if !keystroke.modifiers.modified() => Some(MenuNav::first),
            "pagedown" if !keystroke.modifiers.modified() => Some(MenuNav::last),
            _ => None,
        };
        if let Some(step) = page {
            set_modality(window, Modality::Keyboard, cx);
            self.move_highlight(step, cx);
            cx.stop_propagation();
            return;
        }
        let searching = self.nav.is_searching(now);
        match typed_key(&keystroke.key, keystroke.key_char.as_deref(), keystroke.modifiers, searching) {
            TypedKey::Char(ch) => {
                set_modality(window, Modality::Keyboard, cx);
                if let Some(ix) = self.nav.type_char(ch, now, &self.entries) {
                    self.scroll.scroll_to_item(ix);
                    cx.notify();
                }
                cx.stop_propagation();
            }
            TypedKey::Space => {
                set_modality(window, Modality::Keyboard, cx);
                self.select_highlighted(cx);
                cx.stop_propagation();
            }
            TypedKey::Ignore => {}
        }
    }

    /// A release of `button` at `at`, in the menu (`inside`) or outside it:
    /// ends the opening gesture when it is that press's button, and chooses
    /// or not by [`menu_item_release`].
    fn release(&mut self, button: MouseButton, at: Point<Pixels>, inside: bool, cx: &mut Context<Self>) {
        let press = self.press;
        if press.is_some_and(|press| press.button == button) {
            self.press = None;
        }
        let over = match self.item_at(at) {
            Some(ix) if inside => ReleaseOver::Option(ix),
            _ if inside => ReleaseOver::Menu,
            _ => ReleaseOver::Outside,
        };
        if let MenuRelease::Pick(ix) = menu_item_release(press, button, at, over) {
            self.select(ix, cx);
        }
    }

    fn released(
        this: &WeakEntity<Self>,
        button: MouseButton,
        inside: bool,
    ) -> impl Fn(&MouseUpEvent, &mut Window, &mut App) + 'static {
        let this = this.clone();
        move |event, _, cx| {
            let _ = this.update(cx, |menu, cx| menu.release(button, event.position, inside, cx));
        }
    }
}

/// An item's row: shadcn's `flex items-center gap-2 rounded-sm px-2 py-1.5
/// text-sm`, inset by the panel's `p-1` (as a margin, so separators span
/// the panel), the label over its description, and the shortcut at the
/// end.
fn item_row(item: &MenuItem, highlighted: bool, style: &MenuStyle) -> Div {
    div()
        .flex_none()
        .mx_1()
        .flex()
        .items_center()
        .gap_2()
        .px_2()
        .py_1p5()
        .rounded(style.item_radius)
        .text_size(style.text_size)
        .whitespace_nowrap()
        .when(highlighted, |d| d.bg(style.highlight).text_color(style.highlight_foreground))
        .when(item.disabled, |d| d.opacity(DISABLED_OPACITY))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(div().truncate().child(item.label.clone()))
                .when_some(item.description.clone(), |d, description| {
                    d.child(
                        div().truncate().text_size(style.description_size).text_color(style.muted).child(description),
                    )
                }),
        )
        // `ml-auto`, with room before the keys.
        .when_some(item.shortcut.clone(), |d, keys| d.child(Kbd::new(keys).ml_auto().pl_4()))
}

impl Render for Menu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let style = menu_style(cx.ui_theme());
        let viewport = window.viewport_size();
        let max_width = (viewport.width - EDGE_MARGIN * 2.).max(px(0.));
        let min_width = MENU_MIN_WIDTH.to_pixels(window.rem_size()).min(max_width);
        let highlight = self.nav.highlighted();
        let this = cx.entity().downgrade();
        // Every entry, separators included, is a direct child of the
        // scroller, which `scroll_to_item` and `entry_bounds` index.
        let rows: Vec<AnyElement> = self
            .entries
            .iter()
            .enumerate()
            .map(|(ix, entry)| match entry {
                // `-mx-1 my-1 h-px bg-border`: the panel's width.
                MenuEntry::Separator => div().flex_none().my_1().h(px(1.)).bg(style.separator).into_any_element(),
                MenuEntry::Item(item) => item_row(item, highlight == Some(ix), &style).into_any_element(),
            })
            .collect();

        div()
            .track_focus(&self.focus)
            .key_context("UiList")
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::first))
            .on_action(cx.listener(Self::last))
            .on_action(cx.listener(Self::confirm))
            .on_key_down(cx.listener(Self::key_down))
            .child(
                // DropdownMenuContent / ContextMenuContent: `min-w-[8rem]
                // max-h-(available height) overflow-y-auto rounded-md border
                // bg-popover text-popover-foreground p-1 shadow-md`.
                div()
                    .id("ui-menu")
                    .flex()
                    .flex_col()
                    .min_w(min_width)
                    .max_w(max_width)
                    .max_h(self.anchor.max_height(viewport))
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .py_1()
                    .rounded(style.radius)
                    .border_1()
                    .border_color(style.border)
                    .bg(style.background)
                    .text_color(style.foreground)
                    .shadow_md()
                    // A press in the menu is pointer input, and starts anew:
                    // the opening press is over.
                    .on_any_mouse_down(cx.listener(|this, _: &MouseDownEvent, window, cx| {
                        this.press = None;
                        set_modality(window, Modality::Pointer, cx);
                    }))
                    // The pointer highlights the enabled item under it, and
                    // nothing over anything else.
                    .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, _, cx| {
                        let ix = this.item_at(event.position);
                        this.set_highlight(ix, cx);
                    }))
                    .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                        if !*hovered {
                            this.set_highlight(None, cx);
                        }
                    }))
                    .on_mouse_up(MouseButton::Left, Self::released(&this, MouseButton::Left, true))
                    .on_mouse_up(MouseButton::Right, Self::released(&this, MouseButton::Right, true))
                    // Releases outside: the layer blocks the pointer out
                    // there, so nothing beneath sees them.
                    .on_mouse_up_out(MouseButton::Left, Self::released(&this, MouseButton::Left, false))
                    .on_mouse_up_out(MouseButton::Right, Self::released(&this, MouseButton::Right, false))
                    .children(rows),
            )
    }
}

// ---- the openers' shared parts --------------------------------------------------

/// A fresh overlay slot for a menu: non-modal, blocking the pointer outside
/// the menu (Radix's menus are modal by default, which disables outside
/// pointer events, and the select's menu does the same here).
pub fn menu_overlay() -> Overlay<Menu> {
    Overlay::non_modal().block_outside_pointer()
}

/// Subscribes an opener to `menu`'s choices: the opener's overlay (found
/// with `overlay`) closes first, so focus is back where it was, and the
/// opener then emits the same [`MenuEvent`] to its owner.
pub(crate) fn forward_selection<O: EventEmitter<MenuEvent> + 'static>(
    menu: &Entity<Menu>,
    overlay: fn(&mut O) -> &mut Overlay<Menu>,
    window: &mut Window,
    cx: &mut Context<O>,
) -> Subscription {
    cx.subscribe_in(menu, window, move |opener, menu, event: &MenuEvent, window, cx| {
        let slot = overlay(opener);
        if slot.view() != Some(menu) {
            return;
        }
        slot.close(window, cx);
        cx.emit(*event);
        cx.notify();
    })
}

/// The open menu of `overlay`, placed against its anchor, or `None`.
pub fn render_menu(overlay: &Overlay<Menu>, window: &mut Window, cx: &mut App) -> Option<AnyElement> {
    let anchor = overlay.view()?.read(cx).anchor;
    overlay.render_with(move |panel| anchor.place(panel), window, cx)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{point, size};
    use std::time::Duration;

    /// Brindle's tab menu, with Move Tab Left disabled (the first tab).
    fn tab_menu() -> Vec<MenuEntry> {
        vec![
            MenuItem::new("Rename Tab").shortcut("ctrl-shift-r").into(),
            MenuItem::new("Move Tab Left").disabled(true).into(),
            MenuItem::new("Move Tab Right").into(),
            MenuEntry::Separator,
            MenuItem::new("Close Tab").shortcut("ctrl-shift-w").into(),
        ]
    }

    /// A profile menu, whose descriptions must not count for typeahead.
    fn profiles() -> Vec<MenuEntry> {
        vec![
            MenuItem::new("bash").description("the default shell").into(),
            MenuItem::new("fish").description("a friendly shell").into(),
            MenuItem::new("toolbox").description("toolbox container · dev").into(),
            MenuItem::new("zsh").description("not installed").disabled(true).into(),
            MenuEntry::Separator,
            MenuItem::new("Settings…").into(),
        ]
    }

    /// The spec's scenarios: the highlight starts on nothing, the first
    /// down highlights the first enabled item, the second down passes the
    /// disabled item, the third the separator, and the ends stop.
    #[test]
    fn down_and_up_skip_disabled_items_and_separators_without_wrapping() {
        let entries = tab_menu();
        let mut nav = MenuNav::new(entries.len());
        assert_eq!(nav.highlighted(), None);
        assert_eq!(nav.down(&entries), Some(0));
        assert_eq!(nav.down(&entries), Some(2));
        assert_eq!(nav.down(&entries), Some(4));
        assert_eq!(nav.down(&entries), Some(4), "down at the end wrapped");
        assert_eq!(nav.up(&entries), Some(2));
        assert_eq!(nav.up(&entries), Some(0));
        assert_eq!(nav.up(&entries), Some(0), "up at the start wrapped");
        // From nothing, up goes to the last enabled item, as Radix's
        // ArrowUp on the content does.
        let mut nav = MenuNav::new(entries.len());
        assert_eq!(nav.up(&entries), Some(4));
    }

    #[test]
    fn home_and_end_go_to_the_first_and_last_enabled_items() {
        let mut entries = tab_menu();
        let mut nav = MenuNav::new(entries.len());
        assert_eq!(nav.last(&entries), Some(4));
        assert_eq!(nav.first(&entries), Some(0));
        // With the first item disabled, home skips it; from nothing too.
        entries[0] = MenuItem::new("Rename Tab").disabled(true).into();
        assert_eq!(nav.first(&entries), Some(2));
        let mut nav = MenuNav::new(entries.len());
        assert_eq!(nav.first(&entries), Some(2));
    }

    #[test]
    fn nothing_to_highlight() {
        let entries = vec![MenuEntry::Separator, MenuItem::new("Off").disabled(true).into()];
        let mut nav = MenuNav::new(entries.len());
        assert_eq!(nav.down(&entries), None);
        assert_eq!(nav.up(&entries), None);
        assert_eq!(nav.first(&entries), None);
        assert_eq!(nav.type_char('o', Instant::now(), &entries), None);
        let mut empty = MenuNav::new(0);
        assert_eq!(empty.down(&[]), None);
    }

    /// The spec's scenario: `t` highlights `toolbox`, whose label starts with
    /// it, not `bash`, whose description does. A disabled match is passed,
    /// and a repeated letter cycles.
    #[test]
    fn typeahead_matches_labels_only() {
        let entries = profiles();
        let t0 = Instant::now();
        let mut nav = MenuNav::new(entries.len());
        assert_eq!(nav.type_char('t', t0, &entries), Some(2));
        assert_eq!(nav.highlighted(), Some(2));
        // Two seconds on, a new query: `s` passes zsh (disabled) for Settings….
        assert_eq!(nav.type_char('s', t0 + Duration::from_secs(2), &entries), Some(5));
        // `f`, then `i` within the second: fish.
        assert_eq!(nav.type_char('f', t0 + Duration::from_secs(4), &entries), Some(1));
        assert!(nav.is_searching(t0 + Duration::from_millis(4500)));
        assert_eq!(nav.type_char('i', t0 + Duration::from_millis(4500), &entries), Some(1));
        // No label starts with the query: the highlight stays.
        assert_eq!(nav.type_char('q', t0 + Duration::from_secs(8), &entries), None);
        assert_eq!(nav.highlighted(), Some(1));
        // Case doesn't matter.
        assert_eq!(nav.type_char('B', t0 + Duration::from_secs(10), &entries), Some(0));
    }

    #[test]
    fn the_pointer_sets_the_highlight() {
        let mut nav = MenuNav::new(3);
        assert!(nav.set_highlight(Some(1)));
        assert!(!nav.set_highlight(Some(1)));
        assert!(nav.set_highlight(None));
        assert_eq!(nav.highlighted(), None);
    }

    /// The release table: the opening press's release within 10px does
    /// nothing; further, it chooses an item it lands on; any other primary
    /// release on an item chooses it; nothing else chooses or closes.
    #[test]
    fn releases_follow_the_gesture_rule() {
        use MenuRelease::{Pick, Stay};
        use ReleaseOver::{Menu, Option, Outside};
        let at = |x: f32, y: f32| point(px(x), px(y));
        let right = Some(MenuPress { button: MouseButton::Right, position: at(200., 40.) });
        let left = Some(MenuPress { button: MouseButton::Left, position: at(200., 40.) });
        for press in [right, left] {
            let button = press.unwrap().button;
            // Released where it began, or within 10px, even over an item.
            for over in [Option(0), Menu, Outside] {
                assert_eq!(menu_item_release(press, button, at(200., 40.), over), Stay, "{button:?} {over:?}");
                assert_eq!(menu_item_release(press, button, at(208., 48.), over), Stay, "{button:?} {over:?}");
            }
            // Dragged onto an item: one gesture.
            assert_eq!(menu_item_release(press, button, at(220., 100.), Option(2)), Pick(2));
            // Dragged elsewhere: nothing, and the menu stays open.
            assert_eq!(menu_item_release(press, button, at(220., 100.), Menu), Stay);
            assert_eq!(menu_item_release(press, button, at(400., 300.), Outside), Stay);
        }
        // A click once the gesture is over: primary chooses, secondary doesn't.
        assert_eq!(menu_item_release(None, MouseButton::Left, at(210., 45.), Option(0)), Pick(0));
        assert_eq!(menu_item_release(None, MouseButton::Left, at(210., 45.), Menu), Stay);
        assert_eq!(menu_item_release(None, MouseButton::Right, at(210., 45.), Option(0)), Stay);
        // A primary release while a secondary gesture is pending is a click.
        assert_eq!(menu_item_release(right, MouseButton::Left, at(201., 41.), Option(0)), Pick(0));
        assert_eq!(menu_item_release(None, MouseButton::Middle, at(210., 45.), Option(0)), Stay);
    }

    #[test]
    fn entries_from_labels_and_items() {
        let entry: MenuEntry = "Close Tab".into();
        assert_eq!(entry, MenuEntry::Item(MenuItem::new("Close Tab")));
        assert!(entry.is_enabled());
        assert_eq!(entry.label(), "Close Tab");
        assert!(!MenuEntry::Separator.is_enabled());
        assert_eq!(MenuEntry::Separator.label(), "");
        assert_eq!(MenuEntry::Separator.item(), None);
        let item = MenuItem::new("toolbox").description("toolbox container · dev").shortcut("ctrl-alt-3");
        assert_eq!(item.description.as_ref().map(|s| s.as_ref()), Some("toolbox container · dev"));
        assert_eq!(item.shortcut.as_ref().map(|s| s.as_ref()), Some("ctrl-alt-3"));
        let entry: MenuEntry = item.disabled(true).into();
        assert!(!entry.is_enabled());
        let shared: MenuEntry = SharedString::from("fish").into();
        assert_eq!(shared.label(), "fish");
    }

    /// A context menu goes on the right of a zero-size trigger at the
    /// point, with no offset; a dropdown below its trigger, 4px away. A
    /// long one is capped at the room beside its anchor.
    #[test]
    fn anchors() {
        let context = MenuAnchor::at(point(px(200.), px(40.)));
        assert_eq!(context.trigger, Bounds { origin: point(px(200.), px(40.)), size: size(px(0.), px(0.)) });
        assert_eq!((context.side, context.offset), (Side::Right, px(0.)));
        let viewport = size(px(1000.), px(700.));
        assert_eq!(context.max_height(viewport), px(684.));
        let trigger = Bounds { origin: point(px(20.), px(100.)), size: size(px(80.), px(36.)) };
        let dropdown = MenuAnchor::below(trigger);
        assert_eq!((dropdown.side, dropdown.offset), (Side::Bottom, SIDE_OFFSET));
        // Below: 700 - 8 - (136 + 4); above: 100 - 4 - 8.
        assert_eq!(dropdown.max_height(viewport), px(552.));
    }
}
