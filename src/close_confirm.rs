//! The close confirmation (`agent-zones`' "Closing an agent mid-turn asks
//! first", design.md decision 12): a modal `Dialog` naming each busy agent
//! a close would end, with Cancel (focused when it opens, so an enter
//! pressed by reflex keeps the agent) and Close (destructive).
//!
//! What asks and what it names is [`crate::agent::close`]'s, pure; this view
//! only draws the words and reports the choice. Escape and a press on the
//! backdrop cancel through the overlay's `Dismiss`, as Cancel does; Close
//! emits [`Confirmed`], and the workspace then closes what was asked.

use gpui::{
    App, Context, DismissEvent, EventEmitter, FocusHandle, Focusable, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Styled as _, Window, div,
};

use brindle_ui::{Button, ButtonVariant, Dialog, FocusStops};

use crate::agent::close::CloseText;

/// The user chose Close.
pub struct Confirmed;

/// Which of the dialog's two buttons has focus, for the dump's focus line
/// (`--- focus confirm stop=cancel|close`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfirmStop {
    Cancel,
    Close,
}

impl ConfirmStop {
    pub fn word(self) -> &'static str {
        match self {
            Self::Cancel => "cancel",
            Self::Close => "close",
        }
    }
}

/// The dialog's view, held in the workspace's modal overlay slot.
pub struct CloseConfirm {
    text: CloseText,
    focus: FocusHandle,
    cancel: FocusHandle,
    close: FocusHandle,
}

impl EventEmitter<DismissEvent> for CloseConfirm {}
impl EventEmitter<Confirmed> for CloseConfirm {}

impl Focusable for CloseConfirm {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl FocusStops for CloseConfirm {
    /// Cancel, then Close: tab and shift-tab move between them.
    fn stops(&self, _: &App) -> Vec<FocusHandle> {
        vec![self.cancel.clone(), self.close.clone()]
    }

    fn initial_focus(&self, _: &App) -> Option<FocusHandle> {
        Some(self.cancel.clone())
    }
}

impl CloseConfirm {
    pub fn new(text: CloseText, cx: &mut Context<Self>) -> Self {
        Self { text, focus: cx.focus_handle(), cancel: cx.focus_handle(), close: cx.focus_handle() }
    }

    /// The button with keyboard focus, if one has it.
    pub fn focused_stop(&self, window: &Window) -> Option<ConfirmStop> {
        if self.cancel.is_focused(window) {
            Some(ConfirmStop::Cancel)
        } else if self.close.is_focused(window) {
            Some(ConfirmStop::Close)
        } else {
            None
        }
    }

    /// Whether focus is anywhere in the dialog.
    pub fn contains_focused(&self, window: &Window, cx: &App) -> bool {
        self.focus.contains_focused(window, cx)
    }
}

impl Render for CloseConfirm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let lines = self.text.lines.iter().map(|line| div().text_sm().child(line.clone()));
        div().track_focus(&self.focus).child(
            Dialog::new()
                .title(self.text.title.clone())
                .child(div().flex().flex_col().gap_1().children(lines))
                .footer(
                    Button::new("close-confirm-cancel", "Cancel")
                        .variant(ButtonVariant::Outline)
                        .focus(&self.cancel)
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(DismissEvent))),
                )
                .footer(
                    Button::new("close-confirm-close", "Close")
                        .variant(ButtonVariant::Destructive)
                        .focus(&self.close)
                        .on_click(cx.listener(|_, _, _, cx| cx.emit(Confirmed))),
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
    use crate::agent::close::{AgentBusy, BusyAgent, close_text};
    use crate::zone_view::ZoneId;

    /// A dialog open in a modal overlay slot of a test window, as the
    /// workspace keeps it, with what it reported.
    struct Host {
        slot: brindle_ui::Overlay<CloseConfirm>,
    }

    impl Render for Host {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().children(self.slot.render(window, cx))
        }
    }

    struct Opened {
        host: gpui::Entity<Host>,
        dialog: gpui::Entity<CloseConfirm>,
        cx: VisualTestContext,
        confirmed: Rc<Cell<u32>>,
        dismissed: Rc<Cell<u32>>,
        _subscriptions: Vec<gpui::Subscription>,
        _app: TestAppContext,
    }

    fn open() -> Opened {
        let mut app: TestAppContext = crate::agent::test_support::app();
        let (host, cx) = app.add_window_view(|_, _| Host { slot: brindle_ui::Overlay::modal() });
        let mut cx = cx.clone();
        let busy = [BusyAgent { zone: ZoneId(1), name: "Fake Agent".into(), busy: AgentBusy::Working }];
        let dialog = cx.update(|_, cx| cx.new(|cx| CloseConfirm::new(close_text(&busy), cx)));
        let (confirmed, dismissed) = (Rc::new(Cell::new(0)), Rc::new(Cell::new(0)));
        let subscriptions = cx.update(|_, cx| {
            let (confirmed, dismissed) = (confirmed.clone(), dismissed.clone());
            vec![
                cx.subscribe(&dialog, move |_, _: &Confirmed, _| confirmed.set(confirmed.get() + 1)),
                cx.subscribe(&dialog, move |_, _: &DismissEvent, _| dismissed.set(dismissed.get() + 1)),
            ]
        });
        host.update_in(&mut cx, |host, window, cx| host.slot.open(dialog.clone(), |host| &mut host.slot, window, cx));
        cx.run_until_parked();
        Opened { host, dialog, cx, confirmed, dismissed, _subscriptions: subscriptions, _app: app }
    }

    fn stop(opened: &mut Opened) -> Option<ConfirmStop> {
        let dialog = opened.dialog.clone();
        opened.cx.update(|window, cx| dialog.read(cx).focused_stop(window))
    }

    /// Cancel has focus when the dialog opens; enter there cancels, and
    /// nothing is confirmed.
    #[test]
    fn cancel_is_focused_and_enter_cancels() {
        let mut opened = open();
        assert_eq!(stop(&mut opened), Some(ConfirmStop::Cancel));
        opened.cx.simulate_keystrokes("enter");
        assert_eq!((opened.confirmed.get(), opened.dismissed.get()), (0, 1));
        let open = opened.host.read_with(&opened.cx, |host, _| host.slot.is_open());
        assert!(!open, "the slot closed");
    }

    /// Tab moves to Close; enter there confirms.
    #[test]
    fn tab_then_enter_confirms() {
        let mut opened = open();
        opened.cx.simulate_keystrokes("tab");
        assert_eq!(stop(&mut opened), Some(ConfirmStop::Close));
        opened.cx.simulate_keystrokes("enter");
        assert_eq!((opened.confirmed.get(), opened.dismissed.get()), (1, 0));
    }

    /// Escape cancels.
    #[test]
    fn escape_cancels() {
        let mut opened = open();
        opened.cx.simulate_keystrokes("escape");
        assert_eq!(opened.confirmed.get(), 0);
        let open = opened.host.read_with(&opened.cx, |host, _| host.slot.is_open());
        assert!(!open, "escape closed it");
    }
}
