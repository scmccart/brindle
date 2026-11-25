//! The link confirmation (`links`' "Confirming links an agent wrote",
//! design.md decision 7 of `agents-markdown`): a modal `Dialog` showing the
//! whole link an agent wrote, with Cancel (focused when it opens, so an
//! enter pressed by reflex opens nothing) and Open.
//!
//! Which links ask is [`crate::agent::markdown::link_action`]'s, pure; this
//! view only draws the link and reports the choice. Escape and a press on
//! the backdrop cancel through the overlay's `Dismiss`, as Cancel does;
//! Open emits [`OpenLink`], and the workspace then opens the link.

use gpui::{
    App, Context, DismissEvent, EventEmitter, FocusHandle, Focusable, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Window, div,
};

use brindle_ui::{ActiveTheme as _, Button, ButtonVariant, Dialog, FocusStops};

use crate::settings::Settings;

/// The dialog's title.
pub const TITLE: &str = "Open this link?";

/// What the dialog says under its title.
pub const DESCRIPTION: &str = "An agent wrote this link. Brindle opens only http and https links without asking.";

/// The user chose Open.
pub struct OpenLink;

/// Which of the dialog's two buttons has focus, for the dump's focus line
/// (`--- focus confirm stop=cancel|open`). Its own type, not the close
/// confirmation's (`types R12`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkStop {
    Cancel,
    Open,
}

impl LinkStop {
    pub fn word(self) -> &'static str {
        match self {
            Self::Cancel => "cancel",
            Self::Open => "open",
        }
    }
}

/// The dump's line for the open confirmation: the link escaped onto one
/// line, as entry lines are.
pub fn describe(uri: &str) -> String {
    format!("--- confirm link {uri:?}\n")
}

/// The dialog's view, held in the workspace's modal overlay slot.
pub struct LinkConfirm {
    uri: SharedString,
    focus: FocusHandle,
    cancel: FocusHandle,
    open: FocusHandle,
}

impl EventEmitter<DismissEvent> for LinkConfirm {}
impl EventEmitter<OpenLink> for LinkConfirm {}

impl Focusable for LinkConfirm {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl FocusStops for LinkConfirm {
    /// Cancel, then Open: tab and shift-tab move between them.
    fn stops(&self, _: &App) -> Vec<FocusHandle> {
        vec![self.cancel.clone(), self.open.clone()]
    }

    fn initial_focus(&self, _: &App) -> Option<FocusHandle> {
        Some(self.cancel.clone())
    }
}

impl LinkConfirm {
    pub fn new(uri: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self { uri: uri.into(), focus: cx.focus_handle(), cancel: cx.focus_handle(), open: cx.focus_handle() }
    }

    #[cfg(test)]
    pub fn uri(&self) -> &str {
        &self.uri
    }

    /// The button with keyboard focus, if one has it.
    pub fn focused_stop(&self, window: &Window) -> Option<LinkStop> {
        if self.cancel.is_focused(window) {
            Some(LinkStop::Cancel)
        } else if self.open.is_focused(window) {
            Some(LinkStop::Open)
        } else {
            None
        }
    }

    /// Whether focus is anywhere in the dialog.
    pub fn contains_focused(&self, window: &Window, cx: &App) -> bool {
        self.focus.contains_focused(window, cx)
    }
}

impl Render for LinkConfirm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mono = Settings::get(cx).font().family;
        let muted = cx.ui_theme().muted;
        // The whole link in the terminal's family, wrapped, never cut.
        let link = div()
            .debug_selector(|| "link-confirm-uri".into())
            .text_sm()
            .font_family(mono)
            .whitespace_normal()
            .px_2()
            .py_1()
            .rounded_md()
            .bg(muted)
            .child(self.uri.clone());
        div().track_focus(&self.focus).child(
            Dialog::new()
                .title(TITLE)
                .description(DESCRIPTION)
                .child(link)
                .footer(
                    Button::new("link-confirm-cancel", "Cancel")
                        .variant(ButtonVariant::Outline)
                        .focus(&self.cancel)
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(DismissEvent))),
                )
                .footer(
                    Button::new("link-confirm-open", "Open")
                        .variant(ButtonVariant::Default)
                        .focus(&self.open)
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(OpenLink))),
                ),
        )
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use gpui::{AppContext as _, TestAppContext, VisualTestContext};

    use super::*;

    #[test]
    fn the_dump_line_escapes_the_link_and_the_stops_have_words() {
        assert_eq!(describe("mailto:dev@example.com"), "--- confirm link \"mailto:dev@example.com\"\n");
        assert_eq!(describe("ssh://a\"b\nc"), "--- confirm link \"ssh://a\\\"b\\nc\"\n");
        assert_eq!((LinkStop::Cancel.word(), LinkStop::Open.word()), ("cancel", "open"));
    }

    /// A dialog open in a modal overlay slot of a test window, as the
    /// workspace keeps it.
    struct Host {
        slot: brindle_ui::Overlay<LinkConfirm>,
    }

    impl Render for Host {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().children(self.slot.render(window, cx))
        }
    }

    struct Opened {
        host: gpui::Entity<Host>,
        dialog: gpui::Entity<LinkConfirm>,
        cx: VisualTestContext,
        opened: Rc<Cell<u32>>,
        dismissed: Rc<Cell<u32>>,
        _subscriptions: Vec<gpui::Subscription>,
        _app: TestAppContext,
    }

    fn open() -> Opened {
        let mut app: TestAppContext = crate::agent::test_support::app();
        let (host, cx) = app.add_window_view(|_, _| Host { slot: brindle_ui::Overlay::modal() });
        let mut cx = cx.clone();
        let dialog = cx.update(|_, cx| cx.new(|cx| LinkConfirm::new("mailto:dev@example.com", cx)));
        let (opened, dismissed) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
        let subscriptions = cx.update(|_, cx| {
            let (opened, dismissed) = (opened.clone(), dismissed.clone());
            vec![
                cx.subscribe(&dialog, move |_, _: &OpenLink, _| opened.set(opened.get() + 1)),
                cx.subscribe(&dialog, move |_, _: &DismissEvent, _| dismissed.set(dismissed.get() + 1)),
            ]
        });
        host.update_in(&mut cx, |host, window, cx| host.slot.open(dialog.clone(), |host| &mut host.slot, window, cx));
        cx.run_until_parked();
        Opened { host, dialog, cx, opened, dismissed, _subscriptions: subscriptions, _app: app }
    }

    fn stop(opened: &mut Opened) -> Option<LinkStop> {
        let dialog = opened.dialog.clone();
        opened.cx.update(|window, cx| dialog.read(cx).focused_stop(window))
    }

    /// Cancel has focus when the dialog opens, and it shows the whole
    /// link; enter there cancels, and nothing opens.
    #[test]
    fn cancel_is_focused_and_enter_opens_nothing() {
        let mut opened = open();
        assert_eq!(stop(&mut opened), Some(LinkStop::Cancel));
        assert!(opened.cx.debug_bounds("link-confirm-uri").is_some(), "the link is shown");
        assert_eq!(opened.dialog.read_with(&opened.cx, |dialog, _| dialog.uri().to_string()), "mailto:dev@example.com");
        opened.cx.simulate_keystrokes("enter");
        assert_eq!((opened.opened.get(), opened.dismissed.get()), (0, 1));
        assert!(!opened.host.read_with(&opened.cx, |host, _| host.slot.is_open()), "the slot closed");
    }

    /// Tab moves to Open; enter there opens.
    #[test]
    fn tab_then_enter_opens() {
        let mut opened = open();
        opened.cx.simulate_keystrokes("tab");
        assert_eq!(stop(&mut opened), Some(LinkStop::Open));
        opened.cx.simulate_keystrokes("enter");
        assert_eq!((opened.opened.get(), opened.dismissed.get()), (1, 0));
    }

    /// Escape cancels.
    #[test]
    fn escape_cancels() {
        let mut opened = open();
        opened.cx.simulate_keystrokes("escape");
        assert_eq!(opened.opened.get(), 0);
        assert!(!opened.host.read_with(&opened.cx, |host, _| host.slot.is_open()), "escape closed it");
    }
}
