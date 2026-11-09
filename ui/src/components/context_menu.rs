//! `ContextMenu`: shadcn's ContextMenu, a [`Menu`] opened at the pointer by
//! a secondary press on its target.
//!
//! The owner keeps the entity, renders it anywhere in its tree (it takes
//! no room; the menu is drawn above the window) and subscribes to
//! [`MenuEvent`]. A target opens it from its secondary press with
//! [`ContextMenuTarget::on_context_menu`], calling
//! [`ContextMenu::open_on_press`] with the entries for what was pressed;
//! [`ContextMenu::open`] opens it at a point from code or a key.
//!
//! The menu goes with its top-left corner at the pointer: against a
//! zero-size trigger there, on its right and start-aligned (Radix's
//! `side="right" align="start"`), so near the window's right edge it flips
//! to the left of the pointer and near the bottom it is shifted up into the
//! window ([`MenuAnchor::at`]). It blocks the pointer outside it, so an
//! outside press only closes it, and gives focus back on closing to what
//! had it when it opened.
//!
//! The secondary press that opened it is the start of a gesture: its
//! release within 10px does nothing, and further away it chooses the item
//! it lands on ([`menu_item_release`]).
//!
//! [`menu_item_release`]: crate::components::menu::menu_item_release

use gpui::{
    App, AppContext, Context, Entity, EventEmitter, InteractiveElement, IntoElement, MouseButton, MouseDownEvent, ParentElement,
    Pixels, Point, Render, Styled, Subscription, Window, div,
};

use crate::components::menu::{
    Menu, MenuAnchor, MenuEntry, MenuEvent, MenuPress, forward_selection, menu_overlay, render_menu,
};
use crate::primitives::focus_visible::{Modality, set_modality};
use crate::primitives::overlay::Overlay;

/// A context menu. One serves any number of targets: each open replaces
/// what was open.
pub struct ContextMenu {
    overlay: Overlay<Menu>,
    _selection: Option<Subscription>,
}

impl EventEmitter<MenuEvent> for ContextMenu {}

impl Default for ContextMenu {
    fn default() -> Self {
        Self::new()
    }
}

impl ContextMenu {
    pub fn new() -> Self {
        Self { overlay: menu_overlay(), _selection: None }
    }

    pub fn is_open(&self) -> bool {
        self.overlay.is_open()
    }

    /// The open menu, to read its entries and highlight.
    pub fn menu(&self) -> Option<&Entity<Menu>> {
        self.overlay.view()
    }

    /// Opens a menu of `entries` with its corner at `at` (window
    /// coordinates), highlighting nothing, and focuses it. A menu already
    /// open closes first. For a key or a call from code: no pointer
    /// gesture is under way.
    pub fn open(&mut self, at: Point<Pixels>, entries: Vec<MenuEntry>, window: &mut Window, cx: &mut Context<Self>) {
        self.open_from(at, entries, None, window, cx);
    }

    /// Opens a menu of `entries` at the pointer from the press `event` (a
    /// secondary press on a target), whose release then ends the gesture.
    pub fn open_on_press(
        &mut self,
        event: &MouseDownEvent,
        entries: Vec<MenuEntry>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let press = MenuPress { button: event.button, position: event.position };
        self.open_from(event.position, entries, Some(press), window, cx);
    }

    fn open_from(
        &mut self,
        at: Point<Pixels>,
        entries: Vec<MenuEntry>,
        press: Option<MenuPress>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.overlay.close(window, cx);
        let menu = cx.new(|cx| Menu::new(entries, MenuAnchor::at(at), press, cx));
        self._selection = Some(forward_selection(&menu, |this: &mut Self| &mut this.overlay, window, cx));
        self.overlay.open(menu, |this: &mut Self| &mut this.overlay, window, cx);
    }

    /// Closes the menu without a choice; focus goes back.
    pub fn close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.overlay.is_open() {
            self.overlay.close(window, cx);
            cx.notify();
        }
    }
}

impl Render for ContextMenu {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Out of flow, so it takes no room (or gap) where it is rendered.
        div().absolute().children(render_menu(&self.overlay, window, cx))
    }
}

/// Makes an element a context menu's target: `open` runs on a secondary
/// press on it, and is expected to call [`ContextMenu::open_on_press`].
pub trait ContextMenuTarget: InteractiveElement + Sized {
    /// The press is pointer input. It goes no further: an ancestor that
    /// tracks focus doesn't take focus from the menu that just opened, and
    /// an ancestor's own secondary-press handler (a window menu) doesn't
    /// run.
    fn on_context_menu(self, open: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static) -> Self {
        self.on_mouse_down(MouseButton::Right, move |event, window, cx| {
            set_modality(window, Modality::Pointer, cx);
            open(event, window, cx);
            window.prevent_default();
            cx.stop_propagation();
        })
    }
}

impl<E: InteractiveElement + Sized> ContextMenuTarget for E {}
