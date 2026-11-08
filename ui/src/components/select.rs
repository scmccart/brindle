//! `Select`: shadcn's select, a trigger that opens a menu of options.
//!
//! An entity the owner creates with its options and keeps, renders where
//! the trigger goes and subscribes to: [`SelectEvent::Changed`] is emitted
//! only when the selected option actually changes ([`Selection::pick`]).
//! The trigger shows the selected option's label, or the placeholder, and
//! a chevron; it fills its parent's width. Activating it (a click, enter or
//! space), or pressing up or down (also ctrl-p/ctrl-n) while it has focus,
//! opens the menu below it with the selection highlighted; home and end do
//! nothing while it is closed. Typing while the menu is closed moves the
//! selection by typeahead ([`Selection::type_char`]); while it is open, the
//! highlight.
//!
//! The menu is a non-modal [`Overlay`] placed with [`Anchored`]: below the
//! trigger, start-aligned, at least as wide as it, flipping above when it
//! doesn't fit. It is deferred, so it paints above the window from wherever
//! the select is, a dialog's body included. Its highlight doesn't wrap, moves
//! over enabled options only, follows the pointer and is scrolled into view.
//! Enter, space or a click on an option picks it and closes the menu; Escape
//! and an outside press close it without a change. Focus goes back to the
//! trigger either way, through the overlay.
//!
//! gpui's keyboard click fires on enter and space *key-up*, for whatever has
//! focus then, with no pairing to the key-down. A pick on enter key-down in
//! the menu closes it and puts focus back on the trigger before that key-up
//! arrives, which would open the menu again if the trigger acted on keyboard
//! clicks. So, as every control of the crate (`components`' module doc),
//! the trigger acts on mouse clicks only, and enter and space open it on
//! key-down: enter through the `ListConfirm` action (the trigger is in the
//! `UiList` context) and space through `on_key_down`
//! ([`trigger_activation`], [`typed_key`]).
//!
//! [`Overlay`]: crate::primitives::overlay::Overlay
//! [`Anchored`]: crate::primitives::placement::Anchored

use std::time::Instant;

use gpui::{
    App, AppContext, Bounds, ClickEvent, Context, DismissEvent, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, KeyDownEvent, Modifiers, MouseButton, MouseDownEvent, MouseMoveEvent,
    ParentElement, Pixels, Render, ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Subscription, Window,
    div, prelude::FluentBuilder, px, rems,
};

use crate::actions::{ListConfirm, ListDown, ListFirst, ListLast, ListUp};
use crate::components::popover::{TriggerAction, follows_trigger, trigger_action};
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
    /// A primary mouse click.
    MouseClick,
    /// gpui's keyboard click, on enter or space key-up.
    KeyboardClick,
    /// Enter or a bare space on key-down.
    Key,
}

/// What activating the trigger does when the menu is `open` now and the
/// trigger was `pressed_open` by the press behind a mouse click
/// (`trigger_action`, as for a popover).
///
/// A keyboard click does nothing: it fires on key-up for whatever has focus
/// then, so the one that follows a pick in the menu (enter key-down, which
/// closes the menu and focuses the trigger) would open the menu again. Enter
/// and space act on key-down instead.
pub fn trigger_activation(open: bool, activation: Activation, pressed_open: bool) -> TriggerAction {
    match activation {
        Activation::KeyboardClick => TriggerAction::Stay,
        Activation::MouseClick => trigger_action(open, true, pressed_open),
        Activation::Key => trigger_action(open, false, pressed_open),
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
    /// Whether the menu was open when the last pointer press on the trigger
    /// came (`trigger_action`).
    pressed_open: bool,
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
            menu: Overlay::non_modal(),
            trigger: None,
            pressed_open: false,
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
        if self.disabled || self.menu.is_open() {
            return;
        }
        let options = self.options.clone();
        let selected = self.selection.selected();
        let highlight = initial_highlight(selected, options.len(), |ix| !options[ix].disabled);
        let min_width = self.trigger.map_or(px(0.), |bounds| bounds.size.width);
        let menu = cx.new(|cx| SelectMenu::new(options, selected, highlight, min_width, cx));
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
        let pressed_open = std::mem::take(&mut self.pressed_open);
        match trigger_activation(self.menu.is_open(), activation, pressed_open) {
            TriggerAction::Open => self.open(window, cx),
            TriggerAction::Close => self.close(window, cx),
            TriggerAction::Stay => {}
        }
    }

    fn click(&mut self, event: &ClickEvent, window: &mut Window, cx: &mut Context<Self>) {
        let activation = match event {
            ClickEvent::Mouse(_) => Activation::MouseClick,
            ClickEvent::Keyboard(_) => Activation::KeyboardClick,
        };
        self.activate(activation, window, cx);
    }

    fn mouse_down(&mut self, _: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        // Bubbles after the open menu's outside-press handler asked it to
        // dismiss, and before that dismissal applies.
        self.pressed_open = self.menu.is_open();
        focus_from_pointer(&self.focus, window, cx);
    }

    fn open_on(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open(window, cx);
    }

    fn up(&mut self, _: &ListUp, window: &mut Window, cx: &mut Context<Self>) {
        self.open_on(window, cx);
    }

    fn down(&mut self, _: &ListDown, window: &mut Window, cx: &mut Context<Self>) {
        self.open_on(window, cx);
    }

    /// Enter on the closed trigger, on key-down.
    fn confirm(&mut self, _: &ListConfirm, window: &mut Window, cx: &mut Context<Self>) {
        self.activate(Activation::Key, window, cx);
    }

    /// Home and end do nothing on the closed trigger, as in Radix. They are
    /// taken so that a list around the select doesn't move instead.
    fn first(&mut self, _: &ListFirst, _: &mut Window, _: &mut Context<Self>) {}

    fn last(&mut self, _: &ListLast, _: &mut Window, _: &mut Context<Self>) {}

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
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
                    // Mouse clicks only; see the module doc.
                    .on_click(cx.listener(Self::click))
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
        cx: &mut Context<Self>,
    ) -> Self {
        let mut nav = ListNav::new(options.len(), false);
        nav.highlight = highlight;
        let scroll = ScrollHandle::new();
        if let Some(ix) = highlight {
            scroll.scroll_to_item(ix);
        }
        Self { options, selected, nav, typeahead: Typeahead::new(), focus: cx.focus_handle(), scroll, min_width }
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
                            .on_click(cx.listener(move |_, _, _, cx| cx.emit(MenuEvent::Picked(ix))))
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

    /// The trigger acts on mouse clicks and key-downs; the keyboard click
    /// that follows a key-down does nothing, so a pick on enter doesn't
    /// reopen the menu.
    #[test]
    fn the_trigger_ignores_keyboard_clicks() {
        use TriggerAction::{Close, Open, Stay};
        for pressed_open in [false, true] {
            for open in [false, true] {
                assert_eq!(trigger_activation(open, Activation::KeyboardClick, pressed_open), Stay);
            }
        }
        // A mouse click toggles, except one whose press closed the menu.
        assert_eq!(trigger_activation(false, Activation::MouseClick, false), Open);
        assert_eq!(trigger_activation(false, Activation::MouseClick, true), Stay);
        assert_eq!(trigger_activation(true, Activation::MouseClick, false), Close);
        // Enter or space on key-down opens, and a stale press record
        // counts for nothing.
        assert_eq!(trigger_activation(false, Activation::Key, false), Open);
        assert_eq!(trigger_activation(false, Activation::Key, true), Open);
        assert_eq!(trigger_activation(true, Activation::Key, false), Close);
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
