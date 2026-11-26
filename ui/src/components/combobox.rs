//! `Combobox`: shadcn's combobox, a popover holding a [`Command`]: a trigger
//! showing the chosen option that opens a searchable list of the options.
//!
//! An entity the owner creates with its options, in [`CommandGroup`]s, and
//! keeps, renders where the trigger goes and subscribes to:
//! [`ComboboxEvent::Changed`] carries the chosen option's [`ItemKey`] and is
//! emitted only when the choice changes ([`Selection::pick`]). The choice is
//! kept by that key, so options replaced under it keep it where an option
//! still has it. The trigger looks like a
//! [`Select`]'s and fills its parent's width: the chosen option's label, or
//! the placeholder, and a chevron. A primary press on it, enter or space, or
//! up or down while it has focus opens the panel below it
//! ([`closed_trigger_key`]); a character typed on the closed trigger opens
//! it with that character as the search.
//!
//! The panel is shadcn's `<PopoverContent className="p-0">` around a
//! `<Command>`, at the trigger's width or wider ([`panel_width`]), and is
//! placed only once the trigger has been measured (until then it waits a
//! frame). Its
//! search input has focus; the chosen option is highlighted, scrolled into
//! view and checked ([`checked_groups`]). Typing filters the options, enter
//! or a click chooses the highlighted one and closes the panel, and escape
//! or a press outside it closes it without a change; focus goes back to the
//! trigger either way, through the overlay.
//!
//! The panel is a non-modal [`Overlay`] placed with [`Anchored`], like a
//! select's menu: deferred, so it paints above a dialog's body, flipping
//! above the trigger when it doesn't fit below, and blocking the pointer
//! outside it while open ([`OverlayKind::NonModal`] with `blocks_outside`),
//! so an outside press only closes it. As on a select, the trigger has no
//! click handler: enter and space open it on key-down (see `select`'s
//! module doc).
//!
//! [`Select`]: crate::components::select::Select
//! [`Overlay`]: crate::primitives::overlay::Overlay
//! [`Anchored`]: crate::primitives::placement::Anchored

use gpui::{
    App, AppContext, Bounds, Context, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable, InteractiveElement,
    IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, ParentElement, Pixels, Render, SharedString,
    Styled, Subscription, Window, div, prelude::FluentBuilder, rems,
};

use crate::actions::{ListConfirm, ListDown, ListFirst, ListLast, ListUp};
use crate::components::command::{Command, CommandEvent, CommandGroup, CommandItem, item_keyed};
use crate::components::popover::{PopoverContent, TriggerAction, follows_trigger};
use crate::components::select::{ListKey, Selection, TypedKey, closed_trigger_key, typed_key};
use crate::components::{DISABLED_OPACITY, FocusRing, ItemKey, inert_press};
use crate::icon::Icon;
use crate::primitives::focus_scope::FocusStops;
use crate::primitives::focus_visible::{Modality, focus_from_pointer, modality, set_modality, shows_ring};
use crate::primitives::overlay::{Overlay, OverlayKind};
use crate::primitives::placement::{Align, Anchored, Side, anchor_trigger};
use crate::style::{MUTED_ICON_OPACITY, SELECT_MENU_MIN_WIDTH, focus_ring, select_style};
use crate::theme::ActiveTheme;

// ---- pure --------------------------------------------------------------------

/// The option at `ix` across `groups`, in declaration order.
pub fn option_at(groups: &[CommandGroup], ix: usize) -> Option<&CommandItem> {
    groups.iter().flat_map(|group| &group.items).nth(ix)
}

/// The key of the option at `ix` across `groups`: how an index the owner
/// gives, valid only in the groups of that call, becomes the choice kept.
pub fn key_at(groups: &[CommandGroup], ix: Option<usize>) -> Option<ItemKey> {
    option_at(groups, ix?).map(|item| item.key.clone())
}

/// `groups` with the option at `selected` checked, as shadcn's combobox
/// shows the chosen value with a check, and every other option not.
pub fn checked_groups(groups: &[CommandGroup], selected: Option<usize>) -> Vec<CommandGroup> {
    let mut ix = 0;
    groups
        .iter()
        .map(|group| {
            let mut group = group.clone();
            for item in &mut group.items {
                item.checked = Some(ix) == selected;
                ix += 1;
            }
            group
        })
        .collect()
}

/// The panel's width: the trigger's, but at least `min` (a select menu's
/// least width), so a narrow trigger still gets a usable search. A
/// trigger not measured yet (`None`) gives `min`.
pub fn panel_width(trigger: Option<Pixels>, min: Pixels) -> Pixels {
    trigger.map_or(min, |width| width.max(min))
}

/// What a key typed on the closed trigger does: a character opens the
/// panel searching for it, a bare space opens it, anything else nothing
/// (the arrows and enter come as list actions, [`closed_trigger_key`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TriggerKey {
    Search(char),
    Open,
    Ignore,
}

pub fn trigger_key(typed: TypedKey) -> TriggerKey {
    match typed {
        TypedKey::Char(ch) => TriggerKey::Search(ch),
        TypedKey::Space => TriggerKey::Open,
        TypedKey::Ignore => TriggerKey::Ignore,
    }
}

// ---- entity ------------------------------------------------------------------

/// What a combobox tells its owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ComboboxEvent {
    /// The chosen option changed to the one with this key.
    Changed(ItemKey),
}

/// The default search placeholder.
const SEARCH_PLACEHOLDER: &str = "Search…";

/// A combobox. The owner keeps the entity, renders it where the trigger goes
/// and subscribes to [`ComboboxEvent`].
pub struct Combobox {
    groups: Vec<CommandGroup>,
    selection: Selection,
    placeholder: SharedString,
    search_placeholder: SharedString,
    empty_message: Option<SharedString>,
    disabled: bool,
    /// The trigger's handle, the combobox's own.
    focus: FocusHandle,
    panel: Overlay<ComboboxPanel>,
    /// The trigger's bounds, in window coordinates, from the last frame.
    trigger: Option<Bounds<Pixels>>,
    /// The open panel's picks.
    _picks: Option<Subscription>,
}

impl EventEmitter<ComboboxEvent> for Combobox {}

impl Focusable for Combobox {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl Combobox {
    /// A combobox over the options in `groups`, showing option `selected`
    /// of them, kept by its key from then on (`None`, or an index out of
    /// range, shows the placeholder).
    pub fn new(groups: Vec<CommandGroup>, selected: Option<usize>, cx: &mut Context<Self>) -> Self {
        let selected = key_at(&groups, selected);
        Self {
            groups,
            selection: Selection::new(selected),
            placeholder: SharedString::default(),
            search_placeholder: SEARCH_PLACEHOLDER.into(),
            empty_message: None,
            disabled: false,
            focus: cx.focus_handle(),
            panel: Overlay::new(OverlayKind::NonModal { blocks_outside: true }),
            trigger: None,
            _picks: None,
        }
    }

    /// What the trigger shows while nothing is chosen.
    pub fn with_placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// The search input's placeholder.
    pub fn with_search_placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.search_placeholder = placeholder.into();
        self
    }

    /// The message shown when no option matches the search (shadcn's
    /// `CommandEmpty`; the command list's own by default).
    pub fn with_empty_message(mut self, message: impl Into<SharedString>) -> Self {
        self.empty_message = Some(message.into());
        self
    }

    pub fn groups(&self) -> &[CommandGroup] {
        &self.groups
    }

    /// The option at `ix` across the groups.
    pub fn option(&self, ix: usize) -> Option<&CommandItem> {
        option_at(&self.groups, ix)
    }

    /// The chosen option's index across the groups of now.
    pub fn selected(&self) -> Option<usize> {
        item_keyed(&self.groups, self.selection.selected()?)
    }

    /// The chosen option's key.
    pub fn selected_key(&self) -> Option<&ItemKey> {
        self.selection.selected()
    }

    /// The chosen option's label, or `None` while the placeholder shows.
    pub fn selected_label(&self) -> Option<&SharedString> {
        self.selected().and_then(|ix| self.option(ix)).map(|item| &item.label)
    }

    /// Chooses option `selected` of the groups of now, from the owner (an
    /// index out of range clears the choice). No change is reported.
    pub fn set_selected(&mut self, selected: Option<usize>, cx: &mut Context<Self>) {
        self.selection.set(key_at(&self.groups, selected));
        self.sync_panel(cx);
        cx.notify();
    }

    /// Replaces the options, with option `selected` of them the choice. An
    /// open panel lists the new options, keeping its search and the
    /// highlighted option (by its key). No change is reported.
    pub fn set_groups(&mut self, groups: Vec<CommandGroup>, selected: Option<usize>, cx: &mut Context<Self>) {
        self.groups = groups;
        self.selection.set(key_at(&self.groups, selected));
        self.sync_panel(cx);
        cx.notify();
    }

    /// Gives an open panel the current options and their check, keeping
    /// its highlighted option by its key.
    fn sync_panel(&mut self, cx: &mut Context<Self>) {
        let Some(command) = self.command(cx) else { return };
        let groups = checked_groups(&self.groups, self.selected());
        command.update(cx, |command, cx| {
            let keep = command.highlighted().and_then(|ix| command.item(ix)).map(|item| item.key.clone());
            command.set_groups(groups, keep, cx);
        });
    }

    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    /// A disabled combobox is drawn muted, opens nothing and takes no
    /// focus. An open panel closes.
    pub fn set_disabled(&mut self, disabled: bool, window: &mut Window, cx: &mut Context<Self>) {
        if disabled {
            self.close(window, cx);
        }
        self.disabled = disabled;
        cx.notify();
    }

    pub fn is_open(&self) -> bool {
        self.panel.is_open()
    }

    /// The open panel's command list, for an owner that reads its search
    /// or highlight; `None` while closed.
    pub fn command(&self, cx: &App) -> Option<Entity<Command>> {
        self.panel.view().map(|panel| panel.read(cx).command.clone())
    }

    /// The open panel's highlighted option, by its index across the groups.
    pub fn highlighted(&self, cx: &App) -> Option<usize> {
        self.command(cx).and_then(|command| command.read(cx).highlighted())
    }

    /// Opens the panel with the chosen option highlighted and the search
    /// focused. Nothing happens when the combobox is disabled or already
    /// open.
    pub fn open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.open_with(None, window, cx);
    }

    /// [`Combobox::open`], searching for `query` when given (the first match
    /// is highlighted then).
    fn open_with(&mut self, query: Option<&str>, window: &mut Window, cx: &mut Context<Self>) {
        if self.disabled || self.panel.is_open() {
            return;
        }
        let selected = self.selection.selected().cloned();
        let groups = checked_groups(&self.groups, self.selected());
        let (placeholder, empty) = (self.search_placeholder.clone(), self.empty_message.clone());
        let query = query.map(str::to_string);
        let command = cx.new(|cx| {
            let mut command = Command::new(groups.clone(), cx).with_placeholder(placeholder, cx);
            if let Some(empty) = empty {
                command = command.with_empty_message(empty);
            }
            match &query {
                Some(query) => command.set_query(query, cx),
                // The chosen option highlighted and scrolled into view.
                None => command.set_groups(groups, selected, cx),
            }
            command
        });
        let width = panel_width(
            self.trigger.map(|bounds| bounds.size.width),
            SELECT_MENU_MIN_WIDTH.to_pixels(window.rem_size()),
        );
        let panel = cx.new(|cx| ComboboxPanel::new(command, width, cx));
        self._picks = Some(cx.subscribe_in(&panel, window, |this, _, event: &PanelEvent, window, cx| {
            let PanelEvent::Picked(key) = event;
            // Close first, so focus is back on the trigger when the owner
            // hears of the change.
            this.panel.close(window, cx);
            this.choose(key, cx);
        }));
        self.panel.open(panel, |combobox| &mut combobox.panel, window, cx);
    }

    /// Closes the panel without changing the choice; focus returns to the
    /// trigger.
    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.panel.is_open() {
            self.panel.close(window, cx);
            cx.notify();
        }
    }

    /// Chooses the option keyed `key` as the user does, telling the owner
    /// when it changed.
    fn choose(&mut self, key: &ItemKey, cx: &mut Context<Self>) {
        if let Some(key) = self.selection.pick(key) {
            cx.emit(ComboboxEvent::Changed(key));
        }
        cx.notify();
    }

    /// A primary press on the trigger focuses it (as pointer input) and
    /// opens the panel at once, as a select does.
    fn mouse_down(&mut self, _: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        focus_from_pointer(&self.focus, window, cx);
        self.open(window, cx);
    }

    /// A list action on the closed trigger, as on a select's.
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

    /// Space opens the panel, and a typed character opens it searching for
    /// that character. A key taken here is stopped, which keeps it from
    /// the keystroke observer that records keyboard input, so it is
    /// recorded here.
    fn key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        let typed = typed_key(&keystroke.key, keystroke.key_char.as_deref(), keystroke.modifiers, false);
        match trigger_key(typed) {
            TriggerKey::Search(ch) => {
                set_modality(window, Modality::Keyboard, cx);
                self.open_with(Some(&ch.to_string()), window, cx);
                cx.stop_propagation();
            }
            TriggerKey::Open => {
                set_modality(window, Modality::Keyboard, cx);
                self.open(window, cx);
                cx.stop_propagation();
            }
            TriggerKey::Ignore => {}
        }
    }
}

impl Render for Combobox {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = &cx.ui_theme();
        let style = select_style(t);
        let (ring, ring_color) = (t.ring, focus_ring(t));
        let disabled = self.disabled;
        let focused = !disabled && self.focus.is_focused(window);
        let ring_shown = shows_ring(focused, modality(window, cx), false);
        let label = self.selected_label().cloned();
        let text_color = if label.is_some() { style.foreground } else { style.placeholder };
        let text = label.unwrap_or_else(|| self.placeholder.clone());

        // shadcn's combobox trigger is an outline button with
        // justify-between; drawn as SelectTrigger (flex items-center
        // justify-between gap-2 rounded-md border border-input px-3 py-2
        // text-sm whitespace-nowrap h-9, the chevron at opacity-50), so a
        // combobox and a select side by side look alike.
        let trigger = div()
            .id("ui-combobox-trigger")
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

        // The trigger's bounds each frame, for placing and sizing the panel.
        // Recorded without a notify, since this runs while the frame is
        // drawn; a trigger that moved under an open panel asks for one more
        // frame.
        let this = cx.entity();
        let record = move |bounds, window: &mut Window, cx: &mut App| {
            let follow = this.update(cx, |combobox, _| {
                let follow = follows_trigger(combobox.trigger, bounds, combobox.panel.is_open());
                combobox.trigger = Some(bounds);
                follow
            });
            if follow {
                window.request_animation_frame();
            }
        };

        // Placed only beside a measured trigger: an open panel whose
        // trigger has no bounds yet waits for the frame that records them,
        // which asks for the next (`follows_trigger`).
        let panel = self.trigger.and_then(|at| {
            self.panel.render_with(
                move |panel| Anchored::new(at).side(Side::Bottom).align(Align::Start).child(panel).into_any_element(),
                window,
                cx,
            )
        });

        anchor_trigger(trigger, record).w_full().children(panel)
    }
}

// ---- panel -------------------------------------------------------------------

/// What the panel tells its combobox: the picked option's key.
#[derive(Clone, Debug, PartialEq, Eq)]
enum PanelEvent {
    Picked(ItemKey),
}

/// The open panel, the overlay's view: the command list in a popover's
/// panel. Its one stop is the search input, so Tab stays there.
struct ComboboxPanel {
    command: Entity<Command>,
    width: Pixels,
    _events: Subscription,
}

impl EventEmitter<PanelEvent> for ComboboxPanel {}
impl EventEmitter<DismissEvent> for ComboboxPanel {}

impl Focusable for ComboboxPanel {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.command.focus_handle(cx)
    }
}

impl FocusStops for ComboboxPanel {
    fn stops(&self, cx: &App) -> Vec<FocusHandle> {
        vec![self.command.focus_handle(cx)]
    }
}

impl ComboboxPanel {
    fn new(command: Entity<Command>, width: Pixels, cx: &mut Context<Self>) -> Self {
        let events = cx.subscribe(&command, |_, command, event: &CommandEvent, cx| match *event {
            // The index is the command list's, valid in this update only.
            CommandEvent::Confirmed(ix) => {
                if let Some(key) = command.read(cx).item(ix).map(|item| item.key.clone()) {
                    cx.emit(PanelEvent::Picked(key));
                }
            }
            // Escape, or an outside press (which asks the focused search
            // to dismiss): close without a change.
            CommandEvent::Cancelled => cx.emit(DismissEvent),
        });
        Self { command, width, _events: events }
    }
}

impl Render for ComboboxPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        // <PopoverContent className="w-[--trigger-width] p-0"><Command>…
        PopoverContent::new().p_0().w(self.width).child(self.command.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{Modifiers, px};

    fn groups() -> Vec<CommandGroup> {
        vec![
            CommandGroup::new("Monospace", ["Cascadia Mono", "Consolas"]),
            CommandGroup::new("Other", ["Arial", "Segoe UI"]),
        ]
    }

    #[test]
    fn options_are_counted_across_the_groups() {
        let groups = groups();
        assert_eq!(option_at(&groups, 0).map(|i| i.label.as_ref()), Some("Cascadia Mono"));
        assert_eq!(option_at(&groups, 2).map(|i| i.label.as_ref()), Some("Arial"));
        assert_eq!(option_at(&groups, 4), None);
        assert_eq!(item_keyed(&groups, &ItemKey::from("Segoe UI")), Some(3));
        assert_eq!(item_keyed(&groups, &ItemKey::from("Inter")), None);
    }

    /// b39 #3, #4: an index the owner gives becomes the key kept, and that
    /// key finds the option again in groups that replaced the old ones.
    #[test]
    fn the_choice_is_kept_by_key() {
        let groups = groups();
        assert_eq!(key_at(&groups, Some(2)), Some(ItemKey::from("Arial")));
        assert_eq!(key_at(&groups, Some(4)), None);
        assert_eq!(key_at(&groups, None), None);
        // A family list rebuilt with one more monospace family: Arial moved.
        let rebuilt = vec![
            CommandGroup::new("Monospace", ["Cascadia Mono", "Consolas", "Fira Code"]),
            CommandGroup::new("Other", ["Arial", "Segoe UI"]),
        ];
        assert_eq!(item_keyed(&rebuilt, &ItemKey::from("Arial")), Some(3));
        // Keys of the owner's own are found by key, not label.
        let keyed = vec![CommandGroup::unnamed([
            CommandItem::new("Default").key("default"),
            CommandItem::new("Default").key("family:Default"),
        ])];
        assert_eq!(key_at(&keyed, Some(1)), Some(ItemKey::from("family:Default")));
        assert_eq!(item_keyed(&keyed, &ItemKey::from("family:Default")), Some(1));
    }

    /// The chosen option, and only it, is checked; nothing is without one.
    #[test]
    fn only_the_chosen_option_is_checked() {
        let checked = checked_groups(&groups(), Some(2));
        let flags: Vec<bool> = checked.iter().flat_map(|g| g.items.iter().map(|i| i.checked)).collect();
        assert_eq!(flags, [false, false, true, false]);
        // Headings and labels are kept.
        assert_eq!(checked[1].heading.as_ref().map(|h| h.as_ref()), Some("Other"));
        let none = checked_groups(&checked, None);
        assert!(none.iter().all(|g| g.items.iter().all(|i| !i.checked)));
    }

    /// b39 #5: a trigger not measured yet gives the least width.
    #[test]
    fn the_panel_is_at_least_as_wide_as_the_trigger() {
        assert_eq!(panel_width(Some(px(300.)), px(128.)), px(300.));
        assert_eq!(panel_width(Some(px(90.)), px(128.)), px(128.));
        assert_eq!(panel_width(None, px(128.)), px(128.));
    }

    /// A character on the closed trigger searches for it; a space opens;
    /// a modified key or a control key does nothing.
    #[test]
    fn typing_on_the_trigger_opens_the_search() {
        let none = Modifiers::default();
        let ctrl = Modifiers { control: true, ..Modifiers::default() };
        let key = |key: &str, ch: Option<&str>, modifiers| trigger_key(typed_key(key, ch, modifiers, false));
        assert_eq!(key("c", Some("c"), none), TriggerKey::Search('c'));
        assert_eq!(key("space", None, none), TriggerKey::Open);
        assert_eq!(key("space", Some(" "), none), TriggerKey::Open);
        assert_eq!(key("c", None, ctrl), TriggerKey::Ignore);
        assert_eq!(key("escape", None, none), TriggerKey::Ignore);
        assert_eq!(key("tab", Some("\t"), none), TriggerKey::Ignore);
    }
}
