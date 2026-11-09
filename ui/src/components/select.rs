//! `Select`: shadcn's select, a trigger that opens a menu of options.
//!
//! An entity the owner creates with its options and keeps, renders where
//! the trigger goes and subscribes to: [`SelectEvent::Changed`] is emitted
//! only when the selected option actually changes ([`Selection::pick`]).
//! The trigger shows the selected option's label, or the placeholder, and
//! a chevron; it fills its parent's width. A primary press on it, enter or
//! space, or up or down (also ctrl-p/ctrl-n) while it has focus, opens the
//! menu below it with the selection highlighted; home and end do nothing
//! while it is closed ([`closed_trigger_key`]). Typing while the menu is
//! closed moves the selection by typeahead ([`Selection::type_char`]);
//! while it is open, the highlight.
//!
//! The menu is a non-modal [`Overlay`] placed with [`Anchored`]: below the
//! trigger, start-aligned, at least as wide as it, flipping above when it
//! doesn't fit. It is deferred, so it paints above the window from wherever
//! the select is, a dialog's body included. Its highlight doesn't wrap, moves
//! over enabled options only, follows the pointer and is scrolled into view.
//! Enter or space picks the highlighted option and a primary *release* over
//! an option picks that one, closing the menu; Escape and an outside press
//! close it without a change. Focus goes back to the trigger either way,
//! through the overlay.
//!
//! The pointer follows Radix. The trigger opens on the press, so a press on
//! the trigger, a drag onto an option and a release picks it in one gesture;
//! a release that hardly moved ([`moved_off`]) leaves the menu open, and one
//! that moved and lands outside the menu, the trigger included, closes it
//! ([`menu_release`]). While the menu is open its layer blocks the pointer
//! outside it ([`Overlay::block_outside_pointer`]), so an outside press only
//! closes the menu and reaches nothing beneath, the trigger included.
//!
//! The trigger has no click handler, so gpui's keyboard click (on enter and
//! space *key-up*, for whatever has focus then) never reaches it: a pick on
//! enter key-down in the menu puts focus back on the trigger before that
//! key-up, which must not open the menu again. Enter and space open it on
//! key-down instead: enter through the `ListConfirm` action (the trigger is
//! in the `UiList` context) and space through `on_key_down`
//! ([`trigger_activation`], [`typed_key`]).
//!
//! [`Overlay::block_outside_pointer`]: crate::primitives::overlay::Overlay::block_outside_pointer
//! [`Overlay`]: crate::primitives::overlay::Overlay
//! [`Anchored`]: crate::primitives::placement::Anchored

use std::time::Instant;

use gpui::{
    App, AppContext, Bounds, Context, DismissEvent, EventEmitter, FocusHandle, Focusable, InteractiveElement,
    IntoElement, KeyDownEvent, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement,
    Pixels, Point, Render, ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Subscription, Window, div,
    prelude::FluentBuilder, px, rems,
};

use crate::actions::{ListConfirm, ListDown, ListFirst, ListLast, ListUp};
use crate::components::popover::{TriggerAction, follows_trigger};
use crate::components::{DISABLED_OPACITY, FocusRing, inert_press};
use crate::icon::Icon;
use crate::primitives::focus_scope::FocusStops;
use crate::primitives::focus_visible::{Modality, focus_from_pointer, modality, set_modality, shows_ring};
use crate::primitives::nav::{ListNav, Typeahead};
use crate::primitives::overlay::Overlay;
use crate::primitives::placement::{Align, Anchored, Side, anchor_trigger};
use crate::style::{
    MUTED_ICON_OPACITY, SELECT_MENU_MAX_HEIGHT, SELECT_MENU_MIN_WIDTH, SELECT_TICK_SIZE, focus_ring, select_style,
};
use crate::theme::ActiveTheme;

// ---- pure --------------------------------------------------------------------

/// One option of a select.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectOption {
    pub label: SharedString,
    /// A disabled option is drawn muted and can't be highlighted or chosen.
    pub disabled: bool,
}

impl SelectOption {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self { label: label.into(), disabled: false }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }
}

impl From<&str> for SelectOption {
    fn from(label: &str) -> Self {
        Self::new(label.to_string())
    }
}

impl From<String> for SelectOption {
    fn from(label: String) -> Self {
        Self::new(label)
    }
}

impl From<SharedString> for SelectOption {
    fn from(label: SharedString) -> Self {
        Self::new(label)
    }
}

/// A select's selection, and the typeahead that moves it while the menu is
/// closed. Pure: the entity holds one and reports what it returns.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selection {
    selected: Option<usize>,
    typeahead: Typeahead,
}

impl Selection {
    pub fn new(selected: Option<usize>) -> Self {
        Self { selected, typeahead: Typeahead::new() }
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// Sets the selection from the owner. No change is reported.
    pub fn set(&mut self, selected: Option<usize>) {
        self.selected = selected;
    }

    /// Picks option `ix`, as a click or enter in the menu does. Returns the
    /// new selection when it differs from the current one, which is when the
    /// owner is told; picking the selected option again returns `None`.
    pub fn pick(&mut self, ix: usize) -> Option<usize> {
        if self.selected == Some(ix) {
            return None;
        }
        self.selected = Some(ix);
        Some(ix)
    }

    /// A character typed at `now` while the menu is closed: it extends or
    /// starts the typeahead query, and the selection moves to the first
    /// enabled option from the selection on whose label starts with it
    /// (`Typeahead::find`). Returns the new selection when it changed.
    pub fn type_char<S: AsRef<str>>(
        &mut self,
        ch: char,
        now: Instant,
        labels: &[S],
        enabled: impl Fn(usize) -> bool,
    ) -> Option<usize> {
        self.typeahead.push(ch, now);
        let found = self.typeahead.find(labels, self.selected, enabled)?;
        self.pick(found)
    }

    /// Whether a character typed at `now` extends the query, so a space
    /// continues the search instead of opening the menu.
    pub fn is_searching(&self, now: Instant) -> bool {
        self.typeahead.is_searching(now)
    }

    /// Forgets the query, as when the menu opens.
    pub fn clear_typeahead(&mut self) {
        self.typeahead.clear();
    }
}

/// Where an opening menu's highlight starts: on the selected option, else
/// on the first enabled one, else nowhere.
pub fn initial_highlight(selected: Option<usize>, count: usize, enabled: impl Fn(usize) -> bool) -> Option<usize> {
    selected.or_else(|| (0..count).find(|&ix| enabled(ix)))
}

/// What a key typed into a select or its menu means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TypedKey {
    /// A printable character for the typeahead query, a space included
    /// while a search is under way.
    Char(char),
    /// A bare space outside a search: it activates (opens the menu, or
    /// picks the highlighted option).
    Space,
    /// Anything else: control keys, modified keys and keys with no
    /// character. The list actions cover the arrows, home, end and enter.
    Ignore,
}

/// Reads a keystroke (`key`, its `key_char` and `modifiers`) for a select,
/// when a typeahead search is `searching`. Windows reports a space as the
/// key `space` with no character, so the key is checked before the
/// character. A key with ctrl, alt, the platform or the function modifier
/// is never typed; shift is, for capitals.
pub fn typed_key(key: &str, key_char: Option<&str>, modifiers: Modifiers, searching: bool) -> TypedKey {
    if modifiers.control || modifiers.alt || modifiers.platform || modifiers.function {
        return TypedKey::Ignore;
    }
    if key == "space" || key_char == Some(" ") {
        return if searching { TypedKey::Char(' ') } else { TypedKey::Space };
    }
    let mut chars = key_char.unwrap_or_default().chars();
    match (chars.next(), chars.next()) {
        (Some(ch), None) if !ch.is_control() => TypedKey::Char(ch),
        _ => TypedKey::Ignore,
    }
}

/// How a select's trigger was activated.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Activation {
    /// A primary pointer press, as Radix opens a mouse trigger on
    /// pointer-down (not on the click that follows).
    Press,
    /// Enter or a bare space on key-down.
    Key,
}

/// What activating the trigger does when the menu is `open` now. A press
/// or a key opens a closed menu. A key on an open menu closes it (the open
/// menu has focus, so this is for completeness); a press can't reach the
/// trigger while the menu is open (its layer blocks the pointer outside
/// it), and does nothing if it does.
pub fn trigger_activation(open: bool, activation: Activation) -> TriggerAction {
    match (open, activation) {
        (false, _) => TriggerAction::Open,
        (true, Activation::Key) => TriggerAction::Close,
        (true, Activation::Press) => TriggerAction::Stay,
    }
}

/// How far, in either axis, the pointer may move between the press that
/// opened the menu and its release before the release counts as a drag:
/// Radix's 10px.
pub const DRAG_THRESHOLD: Pixels = px(10.);

/// Whether the pointer moved from the trigger press at `press` to the
/// release at `release` by more than [`DRAG_THRESHOLD`] in either axis.
pub fn moved_off(press: Point<Pixels>, release: Point<Pixels>) -> bool {
    (release.x - press.x).abs() > DRAG_THRESHOLD || (release.y - press.y).abs() > DRAG_THRESHOLD
}

/// Where a primary release over an open menu lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReleaseOver {
    /// An enabled option.
    Option(usize),
    /// The menu, but no enabled option (its padding, a disabled option).
    Menu,
    /// Outside the menu, the select's own trigger included.
    Outside,
}

/// What a release does to an open menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuRelease {
    Pick(usize),
    Stay,
    /// Close without a change.
    Close,
}

/// What a primary release `over` part of the window does to the open
/// menu. `gesture` is `Some(moved)` when the release ends the press that
/// opened the menu, `moved` from [`moved_off`]; `None` for any other
/// release (a click in a menu already open).
///
/// As in Radix's `SelectContent` pointer-up handler: the press that opened
/// the menu, released within [`DRAG_THRESHOLD`] of where it began, does
/// nothing (wherever it lands); moved further, it picks an option it is
/// released on, and closes the menu when released outside it, over the
/// trigger as anywhere else. Any later release on an enabled option picks
/// it.
pub fn menu_release(gesture: Option<bool>, over: ReleaseOver) -> MenuRelease {
    match (gesture, over) {
        (Some(false), _) => MenuRelease::Stay,
        (_, ReleaseOver::Option(ix)) => MenuRelease::Pick(ix),
        (Some(true), ReleaseOver::Outside) => MenuRelease::Close,
        _ => MenuRelease::Stay,
    }
}

/// A list action reaching the trigger (it is in the `UiList` context).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListKey {
    Up,
    Down,
    First,
    Last,
    Confirm,
}

/// What a list action does on the trigger, which has focus only while the
/// menu is closed (the open menu takes it). As in Radix: up and down open
/// the menu, enter activates (opens) it on key-down, and home and end do
/// nothing. Space and typeahead come through `on_key_down` ([`typed_key`]);
/// left and right are bound to nothing and typed as nothing, so they do
/// nothing either.
pub fn closed_trigger_key(key: ListKey) -> TriggerAction {
    match key {
        ListKey::Up | ListKey::Down => TriggerAction::Open,
        ListKey::Confirm => trigger_activation(false, Activation::Key),
        ListKey::First | ListKey::Last => TriggerAction::Stay,
    }
}

// ---- entity ------------------------------------------------------------------

/// What a select tells its owner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectEvent {
    /// The selected option changed to this index.
    Changed(usize),
}

/// A select. The owner keeps the entity, renders it where the trigger goes
/// and subscribes to [`SelectEvent`].
pub struct Select {
    options: Vec<SelectOption>,
    selection: Selection,
    placeholder: SharedString,
    disabled: bool,
    /// The trigger's handle, the select's own.
    focus: FocusHandle,
    menu: Overlay<SelectMenu>,
    /// The trigger's bounds, in window coordinates, from the last frame.
    trigger: Option<Bounds<Pixels>>,
    /// The open menu's picks.
    _picks: Option<Subscription>,
}

impl EventEmitter<SelectEvent> for Select {}

impl Focusable for Select {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Select {
    /// A select over `options` showing `selected` (`None`, or an index out
    /// of range, shows the placeholder).
    pub fn new(
        options: impl IntoIterator<Item = impl Into<SelectOption>>,
        selected: Option<usize>,
        cx: &mut Context<Self>,
    ) -> Self {
        let options: Vec<SelectOption> = options.into_iter().map(Into::into).collect();
        let selected = selected.filter(|&ix| ix < options.len());
        Self {
            options,
            selection: Selection::new(selected),
            placeholder: SharedString::default(),
            disabled: false,
            focus: cx.focus_handle(),
            menu: Overlay::non_modal().block_outside_pointer(),
            trigger: None,
            _picks: None,
        }
    }

    /// What the trigger shows while nothing is selected.
    pub fn with_placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    pub fn options(&self) -> &[SelectOption] {
        &self.options
    }

    /// Replaces the options. An open menu closes, and a selection past the
    /// new end is cleared. No change is reported.
    pub fn set_options(
        &mut self,
        options: impl IntoIterator<Item = impl Into<SelectOption>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close(window, cx);
        self.options = options.into_iter().map(Into::into).collect();
        let selected = self.selection.selected().filter(|&ix| ix < self.options.len());
        self.selection.set(selected);
        cx.notify();
    }

    pub fn selected(&self) -> Option<usize> {
        self.selection.selected()
    }

    /// The selected option's label, or `None` while the placeholder shows.
    pub fn selected_label(&self) -> Option<&SharedString> {
        self.selection.selected().and_then(|ix| self.options.get(ix)).map(|option| &option.label)
    }

    /// Sets the selection from the owner (an index out of range clears
    /// it). No change is reported.
    pub fn set_selected(&mut self, selected: Option<usize>, cx: &mut Context<Self>) {
        self.selection.set(selected.filter(|&ix| ix < self.options.len()));
        cx.notify();
    }

    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// A disabled select is drawn muted, opens nothing and takes no focus.
    /// An open menu closes.
    pub fn set_disabled(&mut self, disabled: bool, window: &mut Window, cx: &mut Context<Self>) {
        if disabled {
            self.close(window, cx);
        }
        self.disabled = disabled;
        cx.notify();
    }

    pub fn is_open(&self) -> bool {
        self.menu.is_open()
    }

    /// The open menu's highlighted option, or `None` while the menu is
    /// closed (or highlights nothing).
    pub fn highlighted(&self, cx: &App) -> Option<usize> {
        self.menu.view().and_then(|menu| menu.read(cx).nav.highlight)
    }

    /// Opens the menu with the highlight on the selection (else the first
    /// enabled option) and gives it focus. Nothing happens when the select
    /// is disabled or the menu is already open.
    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_from(None, window, cx);
    }

    /// [`Select::open`], from a pointer press on the trigger at `press` when
    /// given: the menu then reads the release that ends that press as the
    /// end of a press-drag-release gesture ([`menu_release`]).
    fn open_from(&mut self, press: Option<Point<Pixels>>, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled || self.menu.is_open() {
            return;
        }
        let options = self.options.clone();
        let selected = self.selection.selected();
        let highlight = initial_highlight(selected, options.len(), |ix| !options[ix].disabled);
        let min_width = self.trigger.map_or(px(0.), |bounds| bounds.size.width);
        let menu = cx.new(|cx| SelectMenu::new(options, selected, highlight, min_width, press, cx));
        self._picks = Some(cx.subscribe_in(&menu, window, |this, _, event: &MenuEvent, window, cx| {
            let MenuEvent::Picked(ix) = *event;
            // Close first, so focus is back on the trigger when the owner
            // hears of the change.
            this.menu.close(window, cx);
            this.choose(ix, cx);
        }));
        self.selection.clear_typeahead();
        self.menu.open(menu, |select| &mut select.menu, window, cx);
    }

    /// Closes the menu without changing the selection; focus returns to
    /// the trigger.
    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.menu.is_open() {
            self.menu.close(window, cx);
            cx.notify();
        }
    }

    /// Selects `ix` as the user does, telling the owner when it changed.
    fn choose(&mut self, ix: usize, cx: &mut Context<Self>) {
        if let Some(ix) = self.selection.pick(ix) {
            cx.emit(SelectEvent::Changed(ix));
        }
        cx.notify();
    }

    fn activate(&mut self, activation: Activation, window: &mut Window, cx: &mut Context<Self>) {
        match trigger_activation(self.menu.is_open(), activation) {
            TriggerAction::Open => self.open(window, cx),
            TriggerAction::Close => self.close(window, cx),
            TriggerAction::Stay => {}
        }
    }

    /// A primary press on the trigger focuses it (as pointer input) and
    /// opens the menu at once, as Radix does on pointer-down; the release
    /// is the menu's to read.
    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        focus_from_pointer(&self.focus, window, cx);
        if trigger_activation(self.menu.is_open(), Activation::Press) == TriggerAction::Open {
            self.open_from(Some(event.position), window, cx);
        }
    }

    /// A list action on the trigger, by [`closed_trigger_key`]. Home and end
    /// are taken even though they do nothing, so that a list around the
    /// select doesn't move instead.
    fn list_key(&mut self, key: ListKey, window: &mut Window, cx: &mut Context<Self>) {
        match closed_trigger_key(key) {
            TriggerAction::Open => self.open(window, cx),
            TriggerAction::Close => self.close(window, cx),
            TriggerAction::Stay => {}
        }
    }

    fn up(&mut self, _: &ListUp, window: &mut Window, cx: &mut Context<Self>) {
        self.list_key(ListKey::Up, window, cx);
    }

    fn down(&mut self, _: &ListDown, window: &mut Window, cx: &mut Context<Self>) {
        self.list_key(ListKey::Down, window, cx);
    }

    /// Enter on the closed trigger, on key-down.
    fn confirm(&mut self, _: &ListConfirm, window: &mut Window, cx: &mut Context<Self>) {
        self.list_key(ListKey::Confirm, window, cx);
    }

    fn first(&mut self, _: &ListFirst, window: &mut Window, cx: &mut Context<Self>) {
        self.list_key(ListKey::First, window, cx);
    }

    fn last(&mut self, _: &ListLast, window: &mut Window, cx: &mut Context<Self>) {
        self.list_key(ListKey::Last, window, cx);
    }

    /// Typeahead while the menu is closed, and space to open it. A key
    /// taken here is stopped, which keeps it from the keystroke observer
    /// that records keyboard input, so it is recorded here.
    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let now = Instant::now();
        let keystroke = &event.keystroke;
        let searching = self.selection.is_searching(now);
        match typed_key(&keystroke.key, keystroke.key_char.as_deref(), keystroke.modifiers, searching) {
            TypedKey::Char(ch) => {
                set_modality(window, Modality::Keyboard, cx);
                let options = &self.options;
                let labels: Vec<&str> = options.iter().map(|option| option.label.as_ref()).collect();
                if let Some(ix) = self.selection.type_char(ch, now, &labels, |ix| !options[ix].disabled) {
                    cx.emit(SelectEvent::Changed(ix));
                    cx.notify();
                }
                cx.stop_propagation();
            }
            TypedKey::Space => {
                set_modality(window, Modality::Keyboard, cx);
                self.activate(Activation::Key, window, cx);
                cx.stop_propagation();
            }
            TypedKey::Ignore => {}
        }
    }
}

impl Render for Select {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = cx.ui_theme();
        let style = select_style(t);
        let (ring, ring_color) = (t.ring, focus_ring(t));
        let disabled = self.disabled;
        let focused = !disabled && self.focus.is_focused(window);
        let ring_shown = shows_ring(focused, modality(window, cx), false);
        let label = self.selected_label().cloned();
        let text_color = if label.is_some() { style.foreground } else { style.placeholder };
        let text = label.unwrap_or_else(|| self.placeholder.clone());

        // SelectTrigger: flex items-center justify-between gap-2 rounded-md
        // border border-input px-3 py-2 text-sm whitespace-nowrap h-9,
        // data-[placeholder]:text-muted-foreground, a size-4 chevron at
        // opacity-50. It fills its parent's width.
        let trigger = div()
            .id("ui-select-trigger")
            .flex()
            .w_full()
            .items_center()
            .justify_between()
            .gap_2()
            .h_9()
            .px_3()
            .py_2()
            .rounded(style.radius)
            .border_1()
            .border_color(style.border)
            .text_size(rems(0.875))
            .whitespace_nowrap()
            .text_color(text_color)
            .when(ring_shown, |d| d.border_color(ring))
            .focus_ring(ring_shown, ring_color, window)
            .when(disabled, |d| inert_press(d.opacity(DISABLED_OPACITY).cursor_not_allowed()))
            .when(!disabled, |d| {
                d.cursor_pointer()
                    .key_context("UiList")
                    .track_focus(&self.focus)
                    // The press opens; no click handler (see the module doc).
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
                    .on_action(cx.listener(Self::up))
                    .on_action(cx.listener(Self::down))
                    .on_action(cx.listener(Self::first))
                    .on_action(cx.listener(Self::last))
                    .on_action(cx.listener(Self::confirm))
                    .on_key_down(cx.listener(Self::key_down))
            })
            .child(div().flex_1().min_w_0().truncate().child(text))
            .child(div().flex_none().opacity(MUTED_ICON_OPACITY).child(Icon::ChevronDown.size(rems(1.))));

        // The trigger's bounds each frame, for placing the menu. Recorded
        // without a notify, since this runs while the frame is drawn; a
        // trigger that moved under an open menu asks for one more frame.
        let this = cx.entity();
        let record = move |bounds, window: &mut Window, cx: &mut App| {
            let follow = this.update(cx, |select, _| {
                let follow = follows_trigger(select.trigger, bounds, select.menu.is_open());
                select.trigger = Some(bounds);
                follow
            });
            if follow {
                window.request_animation_frame();
            }
        };

        let at = self.trigger.unwrap_or_default();
        let menu = self.menu.render_with(
            move |panel| Anchored::new(at).side(Side::Bottom).align(Align::Start).child(panel).into_any_element(),
            window,
            cx,
        );

        anchor_trigger(trigger, record).w_full().children(menu)
    }
}

// ---- menu --------------------------------------------------------------------

/// What the menu tells its select.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MenuEvent {
    Picked(usize),
}

/// The open menu: the overlay's view. It lists no focus stops, so Tab does
/// nothing while it is open, and the overlay focuses the view itself.
struct SelectMenu {
    options: Vec<SelectOption>,
    selected: Option<usize>,
    nav: ListNav,
    typeahead: Typeahead,
    focus: FocusHandle,
    scroll: ScrollHandle,
    /// The trigger's width, which the menu is at least as wide as.
    min_width: Pixels,
    /// Where the trigger press that opened the menu came, until a release
    /// ends it or another press in the menu starts anew.
    gesture: Option<Point<Pixels>>,
}

impl EventEmitter<MenuEvent> for SelectMenu {}
impl EventEmitter<DismissEvent> for SelectMenu {}

impl Focusable for SelectMenu {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl FocusStops for SelectMenu {
    fn stops(&self, _: &App) -> Vec<FocusHandle> {
        Vec::new()
    }
}

impl SelectMenu {
    fn new(
        options: Vec<SelectOption>,
        selected: Option<usize>,
        highlight: Option<usize>,
        min_width: Pixels,
        gesture: Option<Point<Pixels>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut nav = ListNav::new(options.len(), false);
        nav.highlight = highlight;
        let scroll = ScrollHandle::new();
        if let Some(ix) = highlight {
            scroll.scroll_to_item(ix);
        }
        Self {
            options,
            selected,
            nav,
            typeahead: Typeahead::new(),
            focus: cx.focus_handle(),
            scroll,
            min_width,
            gesture,
        }
    }

    /// A primary release `over` part of the window at `at`: ends the
    /// opening gesture, if any, and picks, stays or closes ([`menu_release`]).
    fn release(&mut self, over: ReleaseOver, at: Point<Pixels>, cx: &mut Context<Self>) {
        let gesture = self.gesture.take().map(|press| moved_off(press, at));
        match menu_release(gesture, over) {
            MenuRelease::Pick(ix) => {
                if !self.options[ix].disabled {
                    cx.emit(MenuEvent::Picked(ix));
                }
            }
            MenuRelease::Close => cx.emit(DismissEvent),
            MenuRelease::Stay => {}
        }
    }

    /// Moves the highlight with `step` and scrolls it into view.
    fn move_highlight(
        &mut self,
        step: impl FnOnce(&mut ListNav, &dyn Fn(usize) -> bool) -> Option<usize>,
        cx: &mut Context<Self>,
    ) {
        let options = &self.options;
        let enabled = |ix: usize| !options[ix].disabled;
        if let Some(ix) = step(&mut self.nav, &enabled) {
            self.scroll.scroll_to_item(ix);
        }
        cx.notify();
    }

    fn up(&mut self, _: &ListUp, _: &mut Window, cx: &mut Context<Self>) {
        self.move_highlight(|nav, enabled| nav.prev(enabled), cx);
    }

    fn down(&mut self, _: &ListDown, _: &mut Window, cx: &mut Context<Self>) {
        self.move_highlight(|nav, enabled| nav.next(enabled), cx);
    }

    fn first(&mut self, _: &ListFirst, _: &mut Window, cx: &mut Context<Self>) {
        self.move_highlight(|nav, enabled| nav.first(enabled), cx);
    }

    fn last(&mut self, _: &ListLast, _: &mut Window, cx: &mut Context<Self>) {
        self.move_highlight(|nav, enabled| nav.last(enabled), cx);
    }

    /// Picks the highlighted option, if it is enabled.
    fn pick_highlight(&mut self, cx: &mut Context<Self>) {
        if let Some(ix) = self.nav.highlight
            && !self.options[ix].disabled
        {
            cx.emit(MenuEvent::Picked(ix));
        }
    }

    fn confirm(&mut self, _: &ListConfirm, _: &mut Window, cx: &mut Context<Self>) {
        self.pick_highlight(cx);
    }

    /// Hovering an enabled option highlights it.
    fn hover(&mut self, ix: usize, cx: &mut Context<Self>) {
        if self.nav.highlight != Some(ix) {
            self.nav.highlight = Some(ix);
            cx.notify();
        }
    }

    /// Typeahead over the options, and space to pick. A key taken here is
    /// stopped, so it is recorded as keyboard input here (the trigger a
    /// pick refocuses shows its ring).
    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let now = Instant::now();
        let keystroke = &event.keystroke;
        let searching = self.typeahead.is_searching(now);
        match typed_key(&keystroke.key, keystroke.key_char.as_deref(), keystroke.modifiers, searching) {
            TypedKey::Char(ch) => {
                set_modality(window, Modality::Keyboard, cx);
                self.typeahead.push(ch, now);
                let options = &self.options;
                let labels: Vec<&str> = options.iter().map(|option| option.label.as_ref()).collect();
                if let Some(ix) = self.typeahead.find(&labels, self.nav.highlight, |ix| !options[ix].disabled) {
                    self.nav.highlight = Some(ix);
                    self.scroll.scroll_to_item(ix);
                    cx.notify();
                }
                cx.stop_propagation();
            }
            TypedKey::Space => {
                set_modality(window, Modality::Keyboard, cx);
                self.pick_highlight(cx);
                cx.stop_propagation();
            }
            TypedKey::Ignore => {}
        }
    }
}

impl Render for SelectMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let style = select_style(cx.ui_theme());
        let highlight = self.nav.highlight;
        let selected = self.selected;
        // SelectItem: relative flex items-center gap-2 rounded-sm py-1.5
        // pr-8 pl-2 text-sm, focus:bg-accent focus:text-accent-foreground,
        // data-[disabled]:opacity-50, the tick absolute right-2 size-3.5.
        let rows: Vec<_> = self
            .options
            .iter()
            .enumerate()
            .map(|(ix, option)| {
                div()
                    .id(("ui-select-option", ix))
                    .relative()
                    .flex()
                    .items_center()
                    .gap_2()
                    .pl_2()
                    .pr_8()
                    .py_1p5()
                    .rounded(style.option_radius)
                    .text_size(rems(0.875))
                    .whitespace_nowrap()
                    .when(highlight == Some(ix), |d| d.bg(style.highlight).text_color(style.highlight_foreground))
                    .when(option.disabled, |d| d.opacity(DISABLED_OPACITY))
                    .when(!option.disabled, |d| {
                        d.on_mouse_move(cx.listener(move |this, _: &MouseMoveEvent, _, cx| this.hover(ix, cx)))
                            // A press is pointer input, so the trigger the
                            // pick gives focus back to shows no ring.
                            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                                set_modality(window, Modality::Pointer, cx)
                            })
                            // A release picks, wherever the press came
                            // (the trigger's included: one gesture).
                            .on_mouse_up(
                                MouseButton::Left,
                                cx.listener(move |this, event: &MouseUpEvent, _, cx| {
                                    this.release(ReleaseOver::Option(ix), event.position, cx)
                                }),
                            )
                    })
                    .child(div().flex_1().min_w_0().truncate().child(option.label.clone()))
                    .when(selected == Some(ix), |d| {
                        d.child(div().absolute().right_2().flex().items_center().child(Icon::Check.size(SELECT_TICK_SIZE)))
                    })
            })
            .collect();
        let min_width = self.min_width.max(SELECT_MENU_MIN_WIDTH.to_pixels(window.rem_size()));

        // SelectContent: min-w-[8rem] rounded-md border bg-popover
        // text-popover-foreground shadow-md, the viewport p-1. The options
        // are the scroller's direct children, which `scroll_to_item` indexes.
        div()
            .track_focus(&self.focus)
            .key_context("UiList")
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::first))
            .on_action(cx.listener(Self::last))
            .on_action(cx.listener(Self::confirm))
            .on_key_down(cx.listener(Self::key_down))
            // A press in the menu starts anew: the opening press is over.
            .on_mouse_down(MouseButton::Left, cx.listener(|this, _: &MouseDownEvent, _, _| this.gesture = None))
            // A release outside the menu, the trigger included. The layer
            // blocks the pointer out there, so nothing beneath sees it.
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, event: &MouseUpEvent, _, cx| {
                    this.release(ReleaseOver::Outside, event.position, cx)
                }),
            )
            .child(
                div()
                    .id("ui-select-menu")
                    .flex()
                    .flex_col()
                    .min_w(min_width)
                    .max_h(SELECT_MENU_MAX_HEIGHT)
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .p_1()
                    .rounded(style.radius)
                    .border_1()
                    .border_color(style.menu_border)
                    .bg(style.menu_background)
                    .text_color(style.menu_foreground)
                    .shadow_md()
                    .children(rows),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const ALL: fn(usize) -> bool = |_| true;

    /// The owner is told only when the selection actually changes.
    #[test]
    fn a_pick_is_a_change_only_when_it_differs() {
        // The spec's scenarios: bash shown, fish picked; bash shown, bash picked.
        let mut selection = Selection::new(Some(0));
        assert_eq!(selection.pick(1), Some(1));
        assert_eq!(selection.selected(), Some(1));
        assert_eq!(selection.pick(1), None);
        assert_eq!(selection.selected(), Some(1));
        // From the placeholder, any pick is a change.
        let mut none = Selection::new(None);
        assert_eq!(none.pick(0), Some(0));
        // The owner's own set reports nothing and is the new baseline.
        none.set(Some(2));
        assert_eq!(none.pick(2), None);
        none.set(None);
        assert_eq!(none.pick(2), Some(2));
    }

    /// Typing while the menu is closed moves the selection as the open
    /// menu's typeahead moves the highlight, from the selection.
    #[test]
    fn typeahead_moves_the_selection_while_closed() {
        let labels = ["bash", "fish", "foot"];
        let t0 = Instant::now();
        let mut selection = Selection::new(Some(0));
        assert!(!selection.is_searching(t0));
        // `f` from bash: fish, a change.
        assert_eq!(selection.type_char('f', t0, &labels, ALL), Some(1));
        assert!(selection.is_searching(t0 + Duration::from_millis(500)));
        // `f` again, two seconds later: foot (a one-character query cycles).
        assert_eq!(selection.type_char('f', t0 + Duration::from_secs(2), &labels, ALL), Some(2));
        // `o` within the second: `fo`, still foot, so no change.
        assert_eq!(selection.type_char('o', t0 + Duration::from_millis(2500), &labels, ALL), None);
        assert_eq!(selection.selected(), Some(2));
        // Nothing starts with `z`: the selection stays and nothing is reported.
        assert_eq!(selection.type_char('z', t0 + Duration::from_secs(5), &labels, ALL), None);
        assert_eq!(selection.selected(), Some(2));
        // Case-insensitive, and a disabled match is skipped.
        assert_eq!(selection.type_char('B', t0 + Duration::from_secs(8), &labels, ALL), Some(0));
        assert_eq!(selection.type_char('f', t0 + Duration::from_secs(11), &labels, |ix| ix != 1), Some(2));
        // Opening the menu forgets the query.
        selection.clear_typeahead();
        assert!(!selection.is_searching(t0 + Duration::from_secs(11)));
    }

    #[test]
    fn the_menu_opens_on_the_selection_or_the_first_enabled_option() {
        assert_eq!(initial_highlight(Some(2), 3, ALL), Some(2));
        assert_eq!(initial_highlight(None, 3, ALL), Some(0));
        assert_eq!(initial_highlight(None, 3, |ix| ix != 0), Some(1));
        assert_eq!(initial_highlight(None, 3, |_| false), None);
        assert_eq!(initial_highlight(None, 0, ALL), None);
        // A selected option is highlighted even when disabled since.
        assert_eq!(initial_highlight(Some(1), 3, |ix| ix != 1), Some(1));
    }

    /// Letters and a space in a search type; a bare space activates;
    /// modified keys and control keys don't type.
    #[test]
    fn typed_keys() {
        let none = Modifiers::default();
        let shift = Modifiers { shift: true, ..Modifiers::default() };
        let ctrl = Modifiers { control: true, ..Modifiers::default() };
        let alt = Modifiers { alt: true, ..Modifiers::default() };
        assert_eq!(typed_key("f", Some("f"), none, false), TypedKey::Char('f'));
        assert_eq!(typed_key("f", Some("F"), shift, false), TypedKey::Char('F'));
        assert_eq!(typed_key("é", Some("é"), none, true), TypedKey::Char('é'));
        // Space: as Windows reports it (no character) and as X11 does.
        assert_eq!(typed_key("space", None, none, false), TypedKey::Space);
        assert_eq!(typed_key("space", Some(" "), none, false), TypedKey::Space);
        assert_eq!(typed_key("space", None, none, true), TypedKey::Char(' '));
        assert_eq!(typed_key("space", Some(" "), none, true), TypedKey::Char(' '));
        // Modified keys are shortcuts, not characters.
        assert_eq!(typed_key("f", None, ctrl, false), TypedKey::Ignore);
        assert_eq!(typed_key("f", Some("f"), alt, false), TypedKey::Ignore);
        assert_eq!(typed_key("space", None, ctrl, false), TypedKey::Ignore);
        // Keys without a character, and control characters.
        assert_eq!(typed_key("down", None, none, false), TypedKey::Ignore);
        assert_eq!(typed_key("enter", Some("\n"), none, false), TypedKey::Ignore);
        assert_eq!(typed_key("tab", Some("\t"), none, true), TypedKey::Ignore);
        assert_eq!(typed_key("escape", None, none, true), TypedKey::Ignore);
        // A composed string is not one character.
        assert_eq!(typed_key("a", Some("ab"), none, false), TypedKey::Ignore);
    }

    /// The trigger acts on a press and on key-downs (it has no click
    /// handler, so gpui's keyboard click on key-up never reaches it).
    #[test]
    fn the_trigger_opens_on_a_press_or_a_key() {
        use TriggerAction::{Close, Open, Stay};
        assert_eq!(trigger_activation(false, Activation::Press), Open);
        assert_eq!(trigger_activation(false, Activation::Key), Open);
        assert_eq!(trigger_activation(true, Activation::Key), Close);
        // The open menu's layer blocks presses on the trigger; one that
        // got through does nothing.
        assert_eq!(trigger_activation(true, Activation::Press), Stay);
    }

    /// Radix's 10px: a release that moved no further than that, in both
    /// axes, ends the opening press where it began.
    #[test]
    fn a_release_moved_off_past_ten_pixels() {
        let at = |x: f32, y: f32| Point { x: px(x), y: px(y) };
        assert!(!moved_off(at(100., 100.), at(100., 100.)));
        assert!(!moved_off(at(100., 100.), at(110., 90.)));
        assert!(moved_off(at(100., 100.), at(111., 100.)));
        assert!(moved_off(at(100., 100.), at(100., 89.)));
    }

    /// The pointer table, Radix's: one gesture (press on the trigger, drag
    /// onto an option, release) picks; released within 10px of the press,
    /// the menu stays, over the trigger or anywhere; moved further and
    /// released outside the menu (the trigger included), it closes. A later
    /// click in an open menu picks an option and does nothing else.
    #[test]
    fn menu_releases_follow_radix() {
        use MenuRelease::{Close, Pick, Stay};
        use ReleaseOver::{Menu, Option, Outside};
        // The opening press, released without moving: nothing, wherever
        // (the trigger it was pressed on is outside the menu).
        for over in [Option(2), Menu, Outside] {
            assert_eq!(menu_release(Some(false), over), Stay, "{over:?}");
        }
        // Dragged, then released: over the trigger is outside too.
        assert_eq!(menu_release(Some(true), Option(2)), Pick(2));
        assert_eq!(menu_release(Some(true), Menu), Stay);
        assert_eq!(menu_release(Some(true), Outside), Close);
        // Any other release: an option picks, nothing else acts.
        assert_eq!(menu_release(None, Option(0)), Pick(0));
        for over in [Menu, Outside] {
            assert_eq!(menu_release(None, over), Stay, "{over:?}");
        }
    }

    /// What a key press does on the closed trigger, the way gpui gets there:
    /// the crate's bindings in `contexts` (the trigger's `UiList` last) pick
    /// the list action [`closed_trigger_key`] answers, and an unbound key
    /// goes to the trigger's `on_key_down` ([`typed_key`]: a bare space
    /// activates on key-down, anything else typed is typeahead, which the
    /// table's keys never are).
    fn on_closed_trigger(contexts: &[&str], key: &str, key_char: Option<&str>) -> TriggerAction {
        use crate::actions::default_bindings;
        use gpui::{Action, KeyContext, Keymap, Keystroke};
        let keymap = Keymap::new(default_bindings());
        let contexts: Vec<KeyContext> = contexts.iter().map(|c| KeyContext::parse(c).unwrap()).collect();
        let press = Keystroke { modifiers: Modifiers::default(), key: key.into(), key_char: key_char.map(Into::into) };
        let (bindings, pending) = keymap.bindings_for_input(&[press], &contexts);
        assert!(!pending, "{key} starts a longer binding");
        if let Some(binding) = bindings.first() {
            let action = binding.action();
            let keys: [(&dyn Action, ListKey); 5] = [
                (&ListUp, ListKey::Up),
                (&ListDown, ListKey::Down),
                (&ListFirst, ListKey::First),
                (&ListLast, ListKey::Last),
                (&ListConfirm, ListKey::Confirm),
            ];
            let Some((_, list_key)) = keys.iter().find(|(a, _)| action.partial_eq(*a)) else {
                panic!("{key} is bound to {} on the trigger", action.name());
            };
            return closed_trigger_key(*list_key);
        }
        match typed_key(key, key_char, Modifiers::default(), false) {
            TypedKey::Space => trigger_activation(false, Activation::Key),
            TypedKey::Char(ch) => panic!("{key} types {ch:?}"),
            TypedKey::Ignore => TriggerAction::Stay,
        }
    }

    /// The closed trigger's keys, pinned to Radix's: enter, space, up and
    /// down open the menu (on the selection, `initial_highlight`); left,
    /// right, home and end do nothing. On its own and inside a dialog's
    /// layer and focus scope, whose bindings must not take any of them.
    #[test]
    fn closed_trigger_keys_match_radix() {
        use TriggerAction::{Open, Stay};
        let table = [
            ("enter", Some("\n"), Open),
            ("space", Some(" "), Open),
            ("space", None, Open),
            ("up", None, Open),
            ("down", None, Open),
            ("left", None, Stay),
            ("right", None, Stay),
            ("home", None, Stay),
            ("end", None, Stay),
        ];
        for contexts in [&["UiList"][..], &["UiLayer", "UiFocusScope", "UiList"]] {
            for (key, key_char, expected) in table {
                assert_eq!(on_closed_trigger(contexts, key, key_char), expected, "{key} ({key_char:?}) in {contexts:?}");
            }
        }
    }

    #[test]
    fn options_from_labels() {
        let option: SelectOption = "bash".into();
        assert_eq!(option, SelectOption::new("bash"));
        assert!(!option.disabled);
        assert!(SelectOption::new("fish").disabled(true).disabled);
        let shared: SelectOption = SharedString::from("foot").into();
        assert_eq!(shared.label.as_ref(), "foot");
    }
}
