//! `TabList` and `TabTrigger`: shadcn's Tabs (Radix's tab list and
//! triggers), with what a window's tab strip adds.
//!
//! A `RenderOnce` builder, controlled by its owner: the owner keeps the
//! triggers and which one is active, passes them in on each render, and is
//! asked through handlers to activate ([`TabActivate`]), close (the index)
//! or move ([`TabMove`]) a trigger. `TabsContent` isn't built: what the
//! active trigger shows is the owner's.
//!
//! Keyboard, as Radix's roving focus with automatic activation: the list is
//! one keyboard stop, focused through the `FocusHandle` the owner gives it,
//! and focus rests on the active trigger, which shows the ring. While it
//! has focus, left and right ask the owner to activate the previous and
//! next enabled trigger, wrapping at the ends, and home or PageUp, end or
//! PageDown the first and last ([`tab_key`], [`tab_target`]); disabled
//! triggers are skipped, and a modified key is left alone. Focus stays on
//! the list, so once the owner activates the trigger it has focus. Enter,
//! space and Escape are left to the owner (a tab strip sends focus back to
//! its terminal on them).
//!
//! Pointer: a primary press on an enabled trigger asks the owner to
//! activate it and goes no further. Unlike Radix it focuses nothing, and no
//! press anywhere on the list moves focus: the owner decides where focus
//! goes (a tab strip focuses the tab's terminal). The additions a tab strip
//! needs, each listed in the crate's README:
//! - a close button (`closable`): a ghost icon button at the trigger's end,
//!   shown on the active trigger and on a hovered one, whose click asks the
//!   owner to close the trigger and doesn't activate it;
//! - a middle press on a closable trigger asks the owner to close it;
//! - leading and trailing adornments (an icon, status dots), and a tooltip
//!   on the trigger and on its close button;
//! - a minimum and maximum width ([`TabWidth`]) with the label cut by an
//!   ellipsis;
//! - with `on_move`, a trigger can be dragged onto another, which asks the
//!   owner to move it there; a bar on the side where it will land marks the
//!   target while dragging ([`drop_side`]);
//! - the `Attached` variant, whose active trigger takes its owner's
//!   surface and text colours ([`TabTrigger::surface`]).
//!
//! A disabled trigger is drawn at half opacity and takes no part: no
//! activation, close, drag or drop, and no tooltip.

use std::rc::Rc;

use gpui::{
    AnyElement, App, AppContext, Bounds, Context, ElementId, FocusHandle, Hsla, InteractiveElement, IntoElement,
    KeyDownEvent, Modifiers, MouseButton, MouseDownEvent, ParentElement, Pixels, Refineable, Render, RenderOnce,
    SharedString, StatefulInteractiveElement, StyleRefinement, Styled, Window, div, prelude::FluentBuilder, px,
};

use crate::components::button::Button;
use crate::components::tooltip::{Tooltip, TooltipFor, TooltipHandler, attach};
use crate::components::{DISABLED_OPACITY, FocusRing};
use crate::icon::Icon;
use crate::primitives::focus_visible::{Modality, modality, set_modality, shows_ring};
use crate::primitives::nav::ListNav;
use crate::style::{
    ButtonSize, ButtonVariant, TAB_DRAG_MAX_WIDTH, TAB_LINE_WIDTH, TabColors, TabVariant, TabWidth, tab_drag_style,
    tab_list_style, tab_style,
};
use crate::theme::ActiveTheme;

/// Where a key moves focus in a tab list: Radix's roving-focus intents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabKey {
    Prev,
    Next,
    First,
    Last,
}

/// The intent of a key-down on a focused, horizontal, left-to-right tab
/// list, as Radix's roving focus maps it: left and right, home and PageUp,
/// end and PageDown. Up and down mean nothing in a horizontal list, and a
/// key held with ctrl, alt, shift or the platform key is left alone. A
/// held key repeats.
pub fn tab_key(key: &str, modifiers: Modifiers) -> Option<TabKey> {
    if modifiers.modified() {
        return None;
    }
    Some(match key {
        "left" => TabKey::Prev,
        "right" => TabKey::Next,
        "home" | "pageup" => TabKey::First,
        "end" | "pagedown" => TabKey::Last,
        _ => return None,
    })
}

/// The trigger `key` moves to from `current` among `count` triggers, of
/// which `enabled` says which can be reached: the previous or next enabled
/// one, coming round past the ends, or the first or last enabled one.
/// `None` when no trigger is enabled; `current` itself when it is the only
/// one.
pub fn tab_target(key: TabKey, current: Option<usize>, count: usize, enabled: impl Fn(usize) -> bool) -> Option<usize> {
    let mut nav = ListNav { highlight: current.filter(|ix| *ix < count), count, wrap: true };
    let target = match key {
        TabKey::Prev => nav.prev(&enabled),
        TabKey::Next => nav.next(&enabled),
        TabKey::First => nav.first(&enabled),
        TabKey::Last => nav.last(&enabled),
    };
    target.filter(|ix| enabled(*ix))
}

/// Which side of the target trigger a dragged one lands on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DropSide {
    Left,
    Right,
}

/// Where the trigger at `from`, dropped on the one at `to`, ends up: moved
/// to `to`'s place, which is after it when it came from the left (the
/// triggers between move left) and before it when it came from the right.
/// `None` for a drop on itself, which moves nothing.
pub fn drop_side(from: usize, to: usize) -> Option<DropSide> {
    match from.cmp(&to) {
        std::cmp::Ordering::Less => Some(DropSide::Right),
        std::cmp::Ordering::Greater => Some(DropSide::Left),
        std::cmp::Ordering::Equal => None,
    }
}

/// The owner is asked to make the trigger at `index` the active one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TabActivate {
    pub index: usize,
    /// How the user asked: a primary press (`Pointer`), or an arrow, home
    /// or end while the list had focus (`Keyboard`). A tab strip focuses
    /// the tab's content after a press and leaves focus on the list after
    /// a key.
    pub by: Modality,
}

/// The owner is asked to move the trigger at `from` to `to`'s place.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TabMove {
    pub from: usize,
    pub to: usize,
}

/// What a dragged trigger carries, and its preview while dragging: the
/// trigger's label in a small panel. Drops are taken only by triggers of
/// the list it came from.
#[derive(Clone, Debug)]
pub struct DraggedTab {
    /// The list's element id.
    pub list: ElementId,
    /// Where the trigger was when the drag began.
    pub index: usize,
    pub label: SharedString,
}

impl Render for DraggedTab {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let style = tab_drag_style(cx.ui_theme());
        div()
            .max_w(TAB_DRAG_MAX_WIDTH)
            .px(gpui::rems(0.75))
            .py(gpui::rems(0.25))
            .rounded(style.radius)
            .border_1()
            .border_color(style.border)
            .bg(style.background)
            .text_color(style.foreground)
            .text_size(gpui::rems(0.875))
            // gpui lays the dragged view out at its min-content size, where
            // wrapping text would break at every word: the label's unwrapped
            // copy sizes it and the cut text is laid over it.
            .child(trigger_label(self.label.clone()))
    }
}

/// One trigger of a [`TabList`]. It renders only inside the list.
pub struct TabTrigger {
    id: ElementId,
    label: SharedString,
    leading: Vec<AnyElement>,
    adornments: Vec<AnyElement>,
    disabled: bool,
    closable: bool,
    colors: TabColors,
    tooltip: Option<TooltipHandler>,
    close_tooltip: Option<TooltipHandler>,
}

impl TabTrigger {
    /// A trigger showing `label`. `id` is its element id, unique in the
    /// list and kept by the trigger when it moves (the tab's own id, not
    /// its index), since gpui keys a trigger's hover, press and drag by it.
    pub fn new(id: impl Into<ElementId>, label: impl Into<SharedString>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            leading: Vec::new(),
            adornments: Vec::new(),
            disabled: false,
            closable: false,
            colors: TabColors::default(),
            tooltip: None,
            close_tooltip: None,
        }
    }

    /// Something before the label: an icon, drawn in the trigger's text
    /// colour.
    pub fn leading(mut self, element: impl IntoElement) -> Self {
        self.leading.push(element.into_any_element());
        self
    }

    /// Something after the label, before the close button: a status dot, a
    /// bell. Adornments keep their size when the label is cut.
    pub fn adornment(mut self, element: impl IntoElement) -> Self {
        self.adornments.push(element.into_any_element());
        self
    }

    /// A disabled trigger is drawn muted, is skipped by the keys and takes
    /// no press, drag or drop.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// A closable trigger has a close button and closes on a middle press,
    /// both through the list's `on_close`.
    pub fn closable(mut self, closable: bool) -> Self {
        self.closable = closable;
        self
    }

    /// The surface the trigger is drawn on while it is the active trigger
    /// of an attached list (a terminal's background), instead of the
    /// `background` token. Other variants and an inactive trigger ignore it.
    pub fn surface(mut self, color: impl Into<Hsla>) -> Self {
        self.colors.surface = Some(color.into());
        self
    }

    /// The text colour while it is the active trigger of an attached list,
    /// instead of the `foreground` token.
    pub fn text_color(mut self, color: impl Into<Hsla>) -> Self {
        self.colors.text = Some(color.into());
        self
    }

    /// The close button's tooltip, worked out when it is shown (a shortcut
    /// from the keymap then).
    pub fn close_tooltip(mut self, build: impl Fn(&mut Window, &mut App) -> Tooltip + 'static) -> Self {
        self.close_tooltip = Some(Box::new(build));
        self
    }

    pub fn label(&self) -> &SharedString {
        &self.label
    }
}

/// The trigger's tooltip shows only while it is enabled.
impl TooltipFor for TabTrigger {
    fn tooltip_with(mut self, build: impl Fn(&mut Window, &mut App) -> Tooltip + 'static) -> Self {
        self.tooltip = Some(Box::new(build));
        self
    }
}

type ActivateHandler = Rc<dyn Fn(&TabActivate, &mut Window, &mut App) + 'static>;
type CloseHandler = Rc<dyn Fn(&usize, &mut Window, &mut App) + 'static>;
type MoveHandler = Rc<dyn Fn(&TabMove, &mut Window, &mut App) + 'static>;
type BoundsHandler = Box<dyn Fn(&[Bounds<Pixels>], &mut Window, &mut App) + 'static>;

/// A tab list. Build it with [`TabList::new`], give it its triggers, the
/// active index and the handlers, refine its layout through `Styled`, and
/// render it.
#[derive(IntoElement)]
pub struct TabList {
    id: ElementId,
    variant: TabVariant,
    active: Option<usize>,
    triggers: Vec<TabTrigger>,
    focus: Option<FocusHandle>,
    width: Option<TabWidth>,
    on_activate: Option<ActivateHandler>,
    on_close: Option<CloseHandler>,
    on_move: Option<MoveHandler>,
    on_bounds: Option<BoundsHandler>,
    style: StyleRefinement,
}

impl TabList {
    /// A list with no triggers. `id` names it among its siblings, and
    /// tells its drags from another list's.
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            variant: TabVariant::default(),
            active: None,
            triggers: Vec::new(),
            focus: None,
            width: None,
            on_activate: None,
            on_close: None,
            on_move: None,
            on_bounds: None,
            style: StyleRefinement::default(),
        }
    }

    pub fn variant(mut self, variant: TabVariant) -> Self {
        self.variant = variant;
        self
    }

    /// The index of the active trigger.
    pub fn active(mut self, index: usize) -> Self {
        self.active = Some(index);
        self
    }

    pub fn child(mut self, trigger: TabTrigger) -> Self {
        self.triggers.push(trigger);
        self
    }

    pub fn children(mut self, triggers: impl IntoIterator<Item = TabTrigger>) -> Self {
        self.triggers.extend(triggers);
        self
    }

    /// The handle the list is focused through: one keyboard stop, its
    /// focus resting on the active trigger. Without one the list takes no
    /// keys.
    pub fn focus(mut self, handle: &FocusHandle) -> Self {
        self.focus = Some(handle.clone());
        self
    }

    /// Each trigger's width limits. Without them a trigger is as wide as
    /// its content.
    pub fn trigger_width(mut self, width: TabWidth) -> Self {
        self.width = Some(width);
        self
    }

    /// Asked to activate a trigger: on a primary press on it, and on the
    /// keys while the list has focus.
    pub fn on_activate(mut self, handler: impl Fn(&TabActivate, &mut Window, &mut App) + 'static) -> Self {
        self.on_activate = Some(Rc::new(handler));
        self
    }

    /// Asked to close a closable trigger, by its index: on a click on its
    /// close button and on a middle press on it.
    pub fn on_close(mut self, handler: impl Fn(&usize, &mut Window, &mut App) + 'static) -> Self {
        self.on_close = Some(Rc::new(handler));
        self
    }

    /// Makes the enabled triggers draggable onto each other, and asks to
    /// move one when it is dropped on another.
    pub fn on_move(mut self, handler: impl Fn(&TabMove, &mut Window, &mut App) + 'static) -> Self {
        self.on_move = Some(Rc::new(handler));
        self
    }

    /// Told where each trigger was laid out, in window coordinates and in
    /// trigger order, on every frame (during prepaint, so it must not
    /// notify): for placing something beside a trigger, a menu say.
    pub fn on_trigger_bounds(mut self, handler: impl Fn(&[Bounds<Pixels>], &mut Window, &mut App) + 'static) -> Self {
        self.on_bounds = Some(Box::new(handler));
        self
    }
}

impl Styled for TabList {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

/// What every trigger of one list shares while it renders.
struct ListContext {
    id: ElementId,
    variant: TabVariant,
    active: Option<usize>,
    ring_shown: bool,
    width: Option<TabWidth>,
    primary: Hsla,
    on_activate: Option<ActivateHandler>,
    on_close: Option<CloseHandler>,
    on_move: Option<MoveHandler>,
}

/// The group a trigger's close button looks to for the hover that shows it.
const TRIGGER_GROUP: &str = "brindle-ui-tab-trigger";

impl RenderOnce for TabList {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let tokens = *cx.ui_theme();
        let list_style = tab_list_style(self.variant, &tokens);
        let count = self.triggers.len();
        let active = self.active.filter(|ix| *ix < count);
        let focused = self.focus.as_ref().is_some_and(|handle| handle.is_focused(window));
        let context = ListContext {
            id: self.id.clone(),
            variant: self.variant,
            active,
            ring_shown: shows_ring(focused, modality(window, cx), false),
            width: self.width,
            primary: tokens.primary,
            on_activate: self.on_activate.clone(),
            on_close: self.on_close,
            on_move: self.on_move,
        };
        let enabled: Rc<[bool]> = self.triggers.iter().map(|trigger| !trigger.disabled).collect();
        let triggers: Vec<AnyElement> = self
            .triggers
            .into_iter()
            .enumerate()
            .map(|(ix, trigger)| render_trigger(trigger, ix, &context, window, cx))
            .collect();
        let on_activate = self.on_activate;
        let mut list = div()
            .flex()
            .flex_none()
            .gap(list_style.gap)
            .pt(list_style.padding.top)
            .pr(list_style.padding.right)
            .pb(list_style.padding.bottom)
            .pl(list_style.padding.left)
            .rounded(list_style.radius)
            .when(list_style.align_bottom, |d| d.items_end())
            .when(!list_style.align_bottom, |d| d.items_center())
            .when_some(list_style.height, |d, height| d.h(height))
            .when_some(list_style.background, |d, background| d.bg(background))
            .when_some(self.on_bounds, |d, on_bounds| {
                d.on_children_prepainted(move |bounds, window, cx| on_bounds(&bounds, window, cx))
            })
            .id(self.id)
            // A press anywhere on the list (its gaps and padding included)
            // moves focus nowhere: not to the list, which tracks focus, nor
            // to an ancestor that does. It still bubbles, so empty space in
            // a tab strip keeps acting as the title bar.
            .on_any_mouse_down(|_, window, _| window.prevent_default())
            .when_some(self.focus, |d, handle| {
                d.track_focus(&handle).on_key_down(move |event: &KeyDownEvent, window, cx| {
                    let Some(key) = tab_key(&event.keystroke.key, event.keystroke.modifiers) else { return };
                    // The observer that records keyboard input doesn't run
                    // for a stopped key.
                    set_modality(window, Modality::Keyboard, cx);
                    cx.stop_propagation();
                    let target = tab_target(key, active, enabled.len(), |ix| enabled[ix]);
                    if let (Some(index), Some(on_activate)) = (target, on_activate.as_ref())
                        && Some(index) != active
                    {
                        on_activate(&TabActivate { index, by: Modality::Keyboard }, window, cx);
                    }
                })
            })
            .children(triggers);
        // The owner's layout refinements win over the component's.
        list.style().refine(&self.style);
        list
    }
}

/// One trigger, as the list draws it at `ix`.
fn render_trigger(trigger: TabTrigger, ix: usize, list: &ListContext, window: &mut Window, cx: &mut App) -> AnyElement {
    let tokens = *cx.ui_theme();
    let is_active = list.active == Some(ix);
    let style = tab_style(list.variant, is_active, trigger.colors, &tokens);
    let disabled = trigger.disabled;
    let ring_shown = list.ring_shown && is_active;
    let on_activate = list.on_activate.clone().filter(|_| !disabled);
    let on_close = list.on_close.clone().filter(|_| trigger.closable && !disabled);
    let on_move = list.on_move.clone().filter(|_| !disabled);
    let closes = on_close.is_some();
    let border_width = if style.border.is_some() { px(1.) } else { px(0.) };

    let label = trigger_label(trigger.label.clone());
    let close = on_close.clone().map(|on_close| {
        let button = Button::icon_only("close", Icon::X)
            .variant(ButtonVariant::Ghost)
            .size(ButtonSize::IconXs)
            .text_color(style.foreground)
            .on_click(move |_, window, cx| on_close(&ix, window, cx));
        let button = match trigger.close_tooltip {
            Some(build) => button.tooltip_with(build),
            None => button,
        };
        div()
            .flex()
            .flex_none()
            // Shown on the active trigger and on a hovered one; hidden, it
            // keeps its room and takes no press (the trigger gets it).
            .when(!is_active, |d| d.invisible().group_hover(TRIGGER_GROUP, |s| s.visible()))
            // The press stops here: it doesn't activate the trigger or
            // start a drag, and moves focus nowhere (the button has no
            // handle, and the list around it tracks focus).
            .on_mouse_down(MouseButton::Left, |_, window, cx| {
                window.prevent_default();
                cx.stop_propagation();
            })
            .child(button)
    });

    let mut element = div()
        .id(trigger.id)
        .group(TRIGGER_GROUP)
        .relative()
        .flex()
        .items_center()
        .gap(style.gap)
        .pl(style.padding_left)
        .pr(if closes { style.padding_right_with_close } else { style.padding_right })
        .text_size(style.text_size)
        .font_weight(style.font_weight)
        .whitespace_nowrap()
        .text_color(style.foreground)
        .when_some(style.height, |d, height| d.h(height))
        .when(style.height.is_none(), |d| d.h_full())
        .when_some(list.width, |d, width| d.flex_shrink().min_w(width.min()).max_w(width.max()))
        .when(list.width.is_none(), |d| d.flex_none())
        .when_some(style.background, |d, background| d.bg(background))
        .when_some(style.border, |d, border| d.border_1().border_color(border))
        .when(ring_shown, |d| d.when_some(style.focus_border, |d, border| d.border_color(border)))
        .when(style.shadow, |d| d.shadow_sm())
        .when(disabled, |d| d.opacity(DISABLED_OPACITY))
        .when(!disabled && !is_active, |d| {
            let (hover_background, hover_foreground) = (style.hover_background, style.hover_foreground);
            d.hover(move |mut s| {
                s = s.text_color(hover_foreground);
                if let Some(background) = hover_background {
                    s = s.bg(background);
                }
                s
            })
        })
        .on_any_mouse_down(move |event: &MouseDownEvent, window, cx| {
            // No press on a trigger moves focus; the owner decides.
            window.prevent_default();
            match event.button {
                MouseButton::Left if !disabled => {
                    // It goes no further: a tab strip's own press (a window
                    // move) mustn't start under a tab.
                    cx.stop_propagation();
                    set_modality(window, Modality::Pointer, cx);
                    if let Some(on_activate) = &on_activate {
                        on_activate(&TabActivate { index: ix, by: Modality::Pointer }, window, cx);
                    }
                }
                MouseButton::Middle => {
                    if let Some(on_close) = &on_close {
                        cx.stop_propagation();
                        on_close(&ix, window, cx);
                    }
                }
                _ => {}
            }
        })
        .when_some(style.top_line, |d, line| {
            // Over the border, where there is one, as the edge of the trigger.
            let mut top = div()
                .absolute()
                .top(-border_width)
                .left(-border_width)
                .right(-border_width)
                .h(TAB_LINE_WIDTH)
                .bg(line);
            if style.rounded_top_only {
                top = top.rounded_t(style.radius);
            }
            d.child(top)
        })
        .children(trigger.leading)
        .child(label)
        .children(trigger.adornments)
        .children(close);
    element = if style.rounded_top_only { element.rounded_t(style.radius) } else { element.rounded(style.radius) };
    if let Some(on_move) = on_move {
        let value = DraggedTab { list: list.id.clone(), index: ix, label: trigger.label };
        let (over_list, drop_list, primary) = (list.id.clone(), list.id.clone(), list.primary);
        element = element
            .on_drag(value, |value, _, _, cx| cx.new(|_| value.clone()))
            .drag_over::<DraggedTab>(move |style, dragged, _, _| {
                if dragged.list != over_list {
                    return style;
                }
                drop_bar(style, drop_side(dragged.index, ix), primary)
            })
            .can_drop(move |value, _, _| {
                value.downcast_ref::<DraggedTab>().is_some_and(|dragged| dragged.list == drop_list && dragged.index != ix)
            })
            .on_drop(move |dragged: &DraggedTab, window, cx| {
                on_move(&TabMove { from: dragged.index, to: ix }, window, cx);
            });
    }
    let element = attach(element, trigger.tooltip.filter(|_| !disabled));
    element.focus_ring(ring_shown, style.ring, window).into_any_element()
}

/// The drag-over style that marks where a dropped trigger lands: a primary
/// bar on that side, and no other border meanwhile.
fn drop_bar(mut style: StyleRefinement, side: Option<DropSide>, color: Hsla) -> StyleRefinement {
    let Some(side) = side else { return style };
    let (left, right) = match side {
        DropSide::Left => (TAB_LINE_WIDTH, px(0.)),
        DropSide::Right => (px(0.), TAB_LINE_WIDTH),
    };
    let widths = &mut style.border_widths;
    widths.top = Some(px(0.).into());
    widths.bottom = Some(px(0.).into());
    widths.left = Some(left.into());
    widths.right = Some(right.into());
    style.border_color = Some(color);
    style
}

/// One line of text cut by an ellipsis at the width layout gives it.
/// gpui's `truncate()` (no wrapping) cuts nothing in a flex item: its text
/// is measured first at its natural width, and without a wrap width that
/// first measure is kept. A one-line clamp wraps at the width it's given,
/// so the text is measured again there and cut with the ellipsis.
fn one_line_ellipsis<E: Styled>(element: E) -> E {
    element.whitespace_normal().line_clamp(1).text_ellipsis()
}

/// A trigger's label: as wide as the whole text when there is room, and
/// cut with an ellipsis when there isn't. An invisible unwrapped copy sizes
/// it, so a long label holds its trigger at exactly the maximum width; the
/// cut text alone would measure a few pixels short of it (the ellipsis
/// replaces whole glyphs), and the trigger would change width as its text
/// did. The text shown is laid over that copy at the label's final width.
fn trigger_label(text: SharedString) -> gpui::Div {
    div()
        .relative()
        .flex_1()
        .min_w_0()
        .overflow_hidden()
        .child(div().invisible().whitespace_nowrap().child(text.clone()))
        .child(div().absolute().inset_0().child(text).map(one_line_ellipsis))
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: fn(usize) -> bool = |_| true;

    #[test]
    fn keys_map_as_radixs_roving_focus() {
        let none = Modifiers::default();
        for (key, expected) in [
            ("left", Some(TabKey::Prev)),
            ("right", Some(TabKey::Next)),
            ("home", Some(TabKey::First)),
            ("pageup", Some(TabKey::First)),
            ("end", Some(TabKey::Last)),
            ("pagedown", Some(TabKey::Last)),
            // A horizontal list: up and down mean nothing.
            ("up", None),
            ("down", None),
            // Left to the owner.
            ("enter", None),
            ("space", None),
            ("escape", None),
            ("tab", None),
            ("a", None),
        ] {
            assert_eq!(tab_key(key, none), expected, "{key}");
        }
        for modifiers in [Modifiers::shift(), Modifiers::control(), Modifiers::alt(), Modifiers { platform: true, ..none }] {
            assert_eq!(tab_key("right", modifiers), None, "{modifiers:?}");
            assert_eq!(tab_key("home", modifiers), None, "{modifiers:?}");
        }
    }

    /// The spec's scenarios: right from the second of four activates the
    /// third; right on the last comes round to the first; a disabled next
    /// trigger is skipped.
    #[test]
    fn arrows_move_over_enabled_triggers_and_wrap() {
        assert_eq!(tab_target(TabKey::Next, Some(1), 4, ALL), Some(2));
        assert_eq!(tab_target(TabKey::Next, Some(3), 4, ALL), Some(0));
        assert_eq!(tab_target(TabKey::Prev, Some(0), 4, ALL), Some(3));
        assert_eq!(tab_target(TabKey::Prev, Some(2), 4, ALL), Some(1));
        let third_disabled = |ix| ix != 2;
        assert_eq!(tab_target(TabKey::Next, Some(1), 4, third_disabled), Some(3));
        assert_eq!(tab_target(TabKey::Prev, Some(3), 4, third_disabled), Some(1));
        // Round the end, past a disabled first trigger.
        assert_eq!(tab_target(TabKey::Next, Some(3), 4, |ix| ix != 0), Some(1));
    }

    #[test]
    fn home_and_end_go_to_the_enabled_ends() {
        assert_eq!(tab_target(TabKey::First, Some(2), 4, ALL), Some(0));
        assert_eq!(tab_target(TabKey::Last, Some(1), 4, ALL), Some(3));
        let ends_disabled = |ix| ix != 0 && ix != 3;
        assert_eq!(tab_target(TabKey::First, Some(2), 4, ends_disabled), Some(1));
        assert_eq!(tab_target(TabKey::Last, Some(1), 4, ends_disabled), Some(2));
    }

    #[test]
    fn nowhere_to_go() {
        // The only enabled trigger stays.
        assert_eq!(tab_target(TabKey::Next, Some(1), 3, |ix| ix == 1), Some(1));
        assert_eq!(tab_target(TabKey::Prev, Some(1), 3, |ix| ix == 1), Some(1));
        // None enabled, or none at all.
        assert_eq!(tab_target(TabKey::Next, Some(1), 3, |_| false), None);
        assert_eq!(tab_target(TabKey::First, None, 0, ALL), None);
        // No active trigger (or one out of range): next goes to the first,
        // previous to the last.
        assert_eq!(tab_target(TabKey::Next, None, 3, ALL), Some(0));
        assert_eq!(tab_target(TabKey::Prev, Some(7), 3, ALL), Some(2));
        // A disabled active trigger is left like any other.
        assert_eq!(tab_target(TabKey::Next, Some(1), 3, |ix| ix != 1), Some(2));
    }

    /// The spec's scenario: the first trigger dropped on the third lands in
    /// the third place, after it, so the bar is on its right.
    #[test]
    fn a_dropped_trigger_lands_on_the_side_it_came_past() {
        assert_eq!(drop_side(0, 2), Some(DropSide::Right));
        assert_eq!(drop_side(3, 1), Some(DropSide::Left));
        assert_eq!(drop_side(2, 2), None);
        let bar = drop_bar(StyleRefinement::default(), Some(DropSide::Right), gpui::red());
        assert_eq!(bar.border_widths.right, Some(TAB_LINE_WIDTH.into()));
        assert_eq!(bar.border_widths.left, Some(px(0.).into()));
        assert_eq!(bar.border_color, Some(gpui::red()));
        assert_eq!(drop_bar(StyleRefinement::default(), None, gpui::red()), StyleRefinement::default());
    }

    /// What a key on a focused list does, the way gpui gets there: none of
    /// the list's keys may be taken by the crate's bindings around it (a
    /// bound key never reaches the list's `on_key_down`).
    #[test]
    fn list_keys_reach_the_list() {
        use crate::actions::default_bindings;
        use gpui::{KeyContext, Keymap, Keystroke};
        let keymap = Keymap::new(default_bindings());
        for contexts in [&[][..], &["UiFocusScope"], &["UiLayer", "UiFocusScope"]] {
            let contexts: Vec<KeyContext> = contexts.iter().map(|c| KeyContext::parse(c).unwrap()).collect();
            for key in ["left", "right", "home", "end", "pageup", "pagedown"] {
                let press = Keystroke { modifiers: Modifiers::default(), key: key.into(), key_char: None };
                let (bindings, pending) = keymap.bindings_for_input(&[press], &contexts);
                assert!(!pending && bindings.is_empty(), "{key} is bound around the list in {contexts:?}");
                assert!(tab_key(key, Modifiers::default()).is_some());
            }
        }
    }

    #[test]
    fn a_trigger_starts_enabled_unclosable_and_without_colours() {
        let trigger = TabTrigger::new(("tab", 7usize), "bash");
        assert!(!trigger.disabled && !trigger.closable);
        assert_eq!(trigger.colors, TabColors::default());
        assert!(trigger.tooltip.is_none() && trigger.close_tooltip.is_none());
        let trigger = trigger
            .surface(gpui::rgb(0x1a1b26))
            .text_color(gpui::rgb(0xc0caf5))
            .closable(true)
            .tooltip_for("bash", None)
            .close_tooltip(|_, _| Tooltip::new("Close tab"));
        assert_eq!(trigger.colors.surface, Some(gpui::rgb(0x1a1b26).into()));
        assert!(trigger.closable && trigger.tooltip.is_some() && trigger.close_tooltip.is_some());
        let list = TabList::new("tabs").child(trigger).active(0);
        assert_eq!((list.active, list.triggers.len(), list.variant), (Some(0), 1, TabVariant::Default));
    }
}
