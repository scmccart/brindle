//! `AgentView` in a test window, over an `Agent` fed through a real sink
//! with fake requests (tasks 5.1, 5.2 and 5.3).

use std::time::Duration;

use gpui::{EntityInputHandler as _, FocusHandle, VisualTestContext};

use super::*;
use crate::agent::test_support::{Call, Zone, added, app, ready, settle, user};
use crate::agent::transcript::{AgentState, Entry, Event, HoldReason, PermissionOption, Slot, StopReason, TurnState};

struct Fixture {
    zone: Zone,
    view: Entity<AgentView>,
    cx: &'static mut VisualTestContext,
}

impl Fixture {
    fn new() -> Self {
        let mut app = app();
        let zone = Zone::new(None, &mut app);
        let agent = zone.agent.clone();
        let (view, cx) = app.add_window_view(|window, cx| AgentView::new(agent, Theme::default(), None, window, cx));
        let cx: &'static mut VisualTestContext = Box::leak(Box::new(cx.clone()));
        let mut this = Self { zone, view, cx };
        this.focus_prompt();
        this
    }

    fn feed(&mut self, events: &[Event]) {
        self.zone.feed(events);
        self.settle();
    }

    fn settle(&mut self) {
        settle(self.cx);
    }

    fn prompt_handle(&mut self) -> FocusHandle {
        self.view.read_with(self.cx, |view, cx| view.focus_handle(cx))
    }

    fn focus(&mut self, handle: &FocusHandle) {
        self.cx.update(|window, _| window.focus(handle));
        self.cx.run_until_parked();
    }

    fn focus_prompt(&mut self) {
        let prompt = self.prompt_handle();
        self.focus(&prompt);
    }

    fn focused(&mut self) -> Option<FocusHandle> {
        self.cx.update(|window, cx| window.focused(cx))
    }

    fn row_stops(&mut self) -> Vec<FocusHandle> {
        self.view.read_with(self.cx, |view, cx| view.prompt.read(cx).row_stops())
    }

    fn stop_button(&mut self) -> Option<FocusHandle> {
        self.view.read_with(self.cx, |view, cx| view.prompt.read(cx).stop_button())
    }

    fn draft(&mut self) -> String {
        self.view.read_with(self.cx, |view, cx| view.prompt.read(cx).text(cx).to_string())
    }

    fn reason(&mut self) -> Option<SharedString> {
        self.view.read_with(self.cx, |view, cx| view.prompt.read(cx).reason().cloned())
    }

    fn queued(&mut self) -> Option<Queued> {
        self.view.read_with(self.cx, |view, cx| view.prompt.read(cx).queued().cloned())
    }

    /// Presses tab `times` times from where focus is, noting each stop.
    fn tabs(&mut self, times: usize) -> Vec<FocusHandle> {
        (0..times)
            .map(|_| {
                self.cx.simulate_keystrokes("tab");
                self.focused().expect("a stop has focus")
            })
            .collect()
    }

    fn calls(&self) -> Vec<Call> {
        self.zone.take_calls()
    }
}

fn working() -> Event {
    Event::Turn { state: TurnState::Working }
}

fn ended(stop: StopReason) -> Event {
    Event::Turn { state: TurnState::Ended { stop } }
}

fn slot(slot: Slot) -> Event {
    Event::Queue { slot }
}

fn waiting() -> Slot {
    Slot::Waiting { text: "next".into() }
}

fn held() -> Slot {
    Slot::Held { text: "next".into(), reason: HoldReason::Cancelled }
}

fn permission() -> Entry {
    let option = |id: &str, name: &str, kind| PermissionOption { id: id.into(), name: name.into(), kind };
    Entry::Permission {
        call: Some("call_1".into()),
        title: "Run cargo test?".into(),
        options: vec![
            option("reject-once", "Reject", PermissionKind::RejectOnce),
            option("allow-once", "Allow once", PermissionKind::AllowOnce),
        ],
        outcome: None,
    }
}

/// Tab walks the prompt, the waiting row's remove, the pending permission's
/// buttons (allow first) and the stop button, and wraps; with a held row
/// and no turn, the prompt, send again and discard.
#[test]
fn tab_walks_the_prompt_the_row_the_permissions_and_stop() {
    let mut f = Fixture::new();
    f.feed(&[ready(), added(1, user("hi")), working(), slot(waiting()), added(2, permission())]);
    let prompt = f.prompt_handle();
    let remove = f.row_stops();
    assert_eq!(remove.len(), 1);
    let (allow, reject) = f.view.read_with(f.cx, |view, _| {
        let handle = |option: &str| view.permission_handle(EntryId(2), option).cloned().expect("pending");
        (handle("allow-once"), handle("reject-once"))
    });
    let stop = f.stop_button().expect("a turn runs");
    assert_eq!(f.tabs(5), [remove[0].clone(), allow, reject, stop, prompt.clone()]);
    f.cx.simulate_keystrokes("shift-tab");
    assert_eq!(f.focused(), f.stop_button(), "shift-tab goes back");

    // The permission answered and the turn over: its buttons and the stop
    // button go, and the focus on stop returns to the prompt.
    f.feed(&[
        Event::Answered {
            id: EntryId(2),
            outcome: crate::agent::transcript::PermissionOutcome::Selected { option: "allow-once".into() },
        },
        ended(StopReason::Cancelled),
        slot(held()),
    ]);
    assert_eq!(f.focused(), Some(prompt.clone()), "the stop that had focus went");
    let row = f.row_stops();
    assert_eq!(row.len(), 2, "send again and discard");
    assert_eq!(f.tabs(3), [row[0].clone(), row[1].clone(), prompt]);
}

/// Enter during a turn with the slot empty sends (the server queues it);
/// with the slot taken it keeps the text with the reason.
#[test]
fn enter_during_a_turn_queues_and_with_the_slot_taken_keeps_the_text() {
    let mut f = Fixture::new();
    f.feed(&[ready(), working()]);
    f.cx.simulate_input("next");
    f.cx.simulate_keystrokes("enter");
    assert_eq!(f.calls(), [Call::Prompt("next".into())]);
    f.zone.sink.reply(1, Ok(()));
    f.feed(&[slot(waiting())]);
    assert_eq!(f.draft(), "", "the taken prompt left the input");
    assert_eq!(f.queued(), Some(Queued::Waiting { text: "next".into() }));

    f.cx.simulate_input("third");
    f.cx.simulate_keystrokes("enter");
    assert!(f.calls().is_empty(), "the slot is taken");
    assert_eq!(f.draft(), "third");
    assert_eq!(f.reason().as_ref().map(SharedString::as_ref), Some(crate::agent::model::ALREADY_QUEUED));
}

/// With no turn running, a held prompt doesn't block a new one: enter
/// sends it and the held one stays.
#[test]
fn enter_beside_a_held_prompt_with_no_turn_sends() {
    let mut f = Fixture::new();
    f.feed(&[ready(), working(), slot(waiting()), ended(StopReason::Cancelled), slot(held())]);
    f.cx.simulate_input("another");
    f.cx.simulate_keystrokes("enter");
    assert_eq!(f.calls(), [Call::Prompt("another".into())]);
    assert!(matches!(f.queued(), Some(Queued::Held { .. })));
}

/// Enter while an input method composes sends nothing.
#[test]
fn enter_while_composing_sends_nothing() {
    let mut f = Fixture::new();
    f.feed(&[ready()]);
    f.cx.simulate_input("ab");
    let area = f.view.read_with(f.cx, |view, cx| view.prompt.read(cx).area().clone());
    f.cx.update(|window, cx| area.update(cx, |area, cx| area.replace_and_mark_text_in_range(None, "e", None, window, cx)));
    f.cx.simulate_keystrokes("enter");
    assert!(f.calls().is_empty());
    // `agent_submit` sees the composition too.
    f.cx.dispatch_action(AgentSubmit);
    assert!(f.calls().is_empty());
}

/// Escape stops the running turn, from the prompt (its stop) and from a
/// permission button (the zone's); with no turn it sends nothing.
#[test]
fn escape_cancels_the_turn() {
    let mut f = Fixture::new();
    f.feed(&[ready()]);
    f.cx.simulate_keystrokes("escape");
    assert!(f.calls().is_empty(), "no turn to stop");
    f.feed(&[working(), added(1, permission())]);
    f.cx.simulate_keystrokes("escape");
    assert_eq!(f.calls(), [Call::Cancel]);
    let allow = f.view.read_with(f.cx, |view, _| view.permission_handle(EntryId(1), "allow-once").cloned()).unwrap();
    f.focus(&allow);
    f.cx.simulate_keystrokes("escape");
    assert_eq!(f.calls(), [Call::Cancel]);
    f.cx.simulate_keystrokes("enter");
    assert_eq!(f.calls(), [Call::Answer(1, Some("allow-once".into()))], "enter on a permission button answers it");
}

/// After the session ends, send again is unavailable and discard clears
/// the row here, sending nothing; focus on discard returns to the prompt.
#[test]
fn after_an_exit_send_again_is_unavailable_and_discard_clears_the_row() {
    let mut f = Fixture::new();
    f.feed(&[ready(), working(), slot(waiting()), ended(StopReason::Cancelled), slot(held())]);
    assert_eq!(f.row_stops().len(), 2);
    f.feed(&[Event::Agent { state: AgentState::Exited { code: Some(1), stderr: "bye".into() } }]);
    f.zone.sink.exited(Some(1));
    f.settle();
    let Some(Queued::Held { resend: UiResend::Unavailable(why), .. }) = f.queued() else { panic!("held") };
    assert_eq!(why.as_ref(), "the agent has exited");
    let row = f.row_stops();
    assert_eq!(row.len(), 1, "discard only");
    f.focus(&row[0]);
    f.cx.simulate_keystrokes("space");
    f.settle();
    assert_eq!(f.queued(), None);
    assert!(f.calls().is_empty(), "nothing goes to an ended session");
    assert_eq!(f.focused(), Some(f.prompt_handle()), "the discard button went, and its focus with it");
    // The alert says it exited.
    assert!(f.cx.debug_bounds("agent-alert").is_some());
}

/// A refused spawn with a Windows batch file's hint shows its `cmd /c`
/// form on a monospace line of its own; the reconnecting banner shows
/// while the link is down.
#[test]
fn the_alert_shows_a_batch_files_form_and_the_banner_the_reconnect() {
    let mut f = Fixture::new();
    assert!(f.cx.debug_bounds("agent-alert-mono").is_none());
    let failure = crate::agent::model::StartFailure::Refused {
        command: "npx".into(),
        cause: "npx is a batch file: run it as `cmd /c npx agent`".into(),
    };
    f.zone.sink.failed(&anyhow::Error::new(failure));
    f.settle();
    assert!(f.cx.debug_bounds("agent-alert-mono").is_some());

    let mut g = Fixture::new();
    g.feed(&[ready()]);
    g.zone.sink.link_down();
    g.settle();
    assert!(g.cx.debug_bounds("agent-banner").is_some());
}

/// The slot changing never touches the draft.
#[test]
fn the_draft_is_unchanged_when_a_prompt_becomes_held() {
    let mut f = Fixture::new();
    f.feed(&[ready(), working(), slot(waiting())]);
    f.cx.simulate_input("typing");
    f.feed(&[ended(StopReason::MaxTokens), slot(Slot::Held { text: "next".into(), reason: HoldReason::Stopped { stop: StopReason::MaxTokens } })]);
    assert_eq!(f.draft(), "typing");
    assert!(matches!(f.queued(), Some(Queued::Held { .. })));
}

/// The palette's actions: remove and send again act on the slot, the copy
/// actions put text on the clipboard, toggle opens the thoughts.
#[test]
fn the_zones_actions_act_on_the_agent() {
    let mut f = Fixture::new();
    f.feed(&[
        ready(),
        added(1, user("hi")),
        added(2, Entry::Thought { text: "hmm".into() }),
        added(3, crate::agent::test_support::agent("Hello!")),
        ended(StopReason::Cancelled),
        slot(held()),
    ]);
    f.cx.dispatch_action(CopyLastAgentMessage);
    assert_eq!(f.cx.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("Hello!"));
    f.cx.dispatch_action(CopyQueuedPrompt);
    assert_eq!(f.cx.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("next"));
    f.cx.dispatch_action(AgentResend);
    f.cx.dispatch_action(AgentUnqueue);
    assert_eq!(f.calls(), [Call::Resend, Call::Unqueue]);
    let open = |f: &mut Fixture| {
        f.zone.agent.read_with(f.cx, |agent, _| {
            agent.model().blocks().iter().any(|b| matches!(b.body, BlockBody::Thought { open: true, .. }))
        })
    };
    assert!(!open(&mut f));
    f.cx.dispatch_action(ToggleAgentThoughts);
    assert!(open(&mut f));
    f.cx.dispatch_action(AgentScrollToBottom);
    assert_eq!(f.view.read_with(f.cx, |view, cx| view.scroller.read(cx).follow()), brindle_ui::Follow::AtEnd);
}

/// Typing into the prompt input marks a `done` as seen; a draft the agent
/// sets doesn't.
#[test]
fn typing_marks_done_as_seen() {
    let mut f = Fixture::new();
    f.feed(&[ready(), working(), ended(StopReason::EndTurn)]);
    let status = |f: &mut Fixture| f.zone.agent.read_with(f.cx, |agent, _| agent.status());
    assert_eq!(status(&mut f), Some(crate::terminal::status::State::Done));
    f.cx.simulate_input("x");
    assert_eq!(status(&mut f), None);
}

/// 5.1: the view draws from the model alone. While another thread holds
/// the shared fold's lock, a frame is drawn; were `render` to take the
/// lock, the frame would wait for it and the holder would time out.
#[test]
fn render_takes_no_lock() {
    let mut f = Fixture::new();
    f.feed(&[ready(), added(1, user("hi")), added(2, crate::agent::test_support::agent("Hello!")), working()]);
    let shared = f.zone.shared.clone();
    let (locked_tx, locked_rx) = std::sync::mpsc::channel();
    let (drawn_tx, drawn_rx) = std::sync::mpsc::channel::<()>();
    let holder = std::thread::spawn(move || {
        let _guard = shared.lock();
        locked_tx.send(()).unwrap();
        // Bounds a failing run: a render that waited on the lock gets it
        // only after this gives up.
        drawn_rx.recv_timeout(Duration::from_secs(10)).is_ok()
    });
    locked_rx.recv().unwrap();
    let view = f.view.clone();
    f.cx.update(|window, cx| {
        view.update(cx, |_, cx| cx.notify());
        window.draw(cx).clear();
    });
    drawn_tx.send(()).unwrap();
    assert!(holder.join().unwrap(), "the frame was drawn while the lock was held");
    assert!(f.cx.debug_bounds("prompt-area").is_some());
}

/// 5.3: the profile's font size, else the terminal size in effect; the
/// drawn prompt grows with it.
#[test]
fn set_profile_changes_the_text_size() {
    let f = Fixture::new();
    let configured = f.cx.update(|_, cx| Settings::get(cx).font_size(None));
    assert_eq!(f.view.read_with(f.cx, |view, cx| view.text_size(cx)), configured);
    let before = f.cx.debug_bounds("prompt-area").expect("drawn").size.height;
    let view = f.view.clone();
    let big = configured + gpui::px(12.0);
    f.cx.update(|_, cx| view.update(cx, |view, cx| view.set_profile(Theme::default(), Some(f32::from(big)), cx)));
    f.cx.run_until_parked();
    assert_eq!(f.view.read_with(f.cx, |view, cx| view.text_size(cx)), big);
    let after = f.cx.debug_bounds("prompt-area").expect("drawn").size.height;
    assert!(after > before, "the prompt is drawn larger: {before:?} then {after:?}");
    f.cx.update(|_, cx| view.update(cx, |view, cx| view.set_profile(Theme::default(), None, cx)));
    assert_eq!(f.view.read_with(f.cx, |view, cx| view.text_size(cx)), configured);
}

#[test]
fn the_prompt_state_follows_the_model() {
    use crate::agent::model::DisabledWhy;
    let model = ChatModel::new("Fake", None);
    let (status, queued, reason) = prompt_state(&model);
    assert_eq!((status, queued), (UiStatus::Disabled, None));
    assert_eq!(reason.as_ref().map(SharedString::as_ref), Some(DisabledWhy::Connecting.text()));
    assert_eq!(bubble_part(0, true), BubblePart::Whole);
    assert_eq!(bubble_part(0, false), BubblePart::First);
    assert_eq!(bubble_part(1, false), BubblePart::Middle);
    assert_eq!(bubble_part(2, true), BubblePart::Last);
    assert_eq!(queued_of(&QueueRow::Waiting { text: "a".into() }), Some(Queued::Waiting { text: "a".into() }));
    assert_eq!(tool_status(ToolStatus::InProgress), UiToolStatus::Running);
    assert_eq!(tool_icon(ToolKind::Execute), Icon::Terminal);
}
