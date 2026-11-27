//! `AgentView` in a test window, over an `Agent` fed through a real sink
//! with fake requests (tasks 5.1, 5.2 and 5.3).

use std::time::Duration;

use gpui::{AnyElement, EntityInputHandler as _, FocusHandle, VisualTestContext, px};

use super::*;
use crate::agent::test_support::{Call, FAKE_PROFILE, Zone, added, app, ready, set_config, settle, user};
use crate::agent::transcript::{
    AgentState, Continuation, Conversation, Entry, Event, HoldReason, PermissionOption, Slot, StopReason, TurnState,
};

struct Fixture {
    zone: Zone,
    view: Entity<AgentView>,
    cx: &'static mut VisualTestContext,
}

impl Fixture {
    fn new() -> Self {
        Self::with_config(None)
    }

    /// A fixture whose app's config is `config` (`None`: the default).
    fn with_config(config: Option<&str>) -> Self {
        let mut app = app();
        if let Some(config) = config {
            set_config(config, &mut app);
        }
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

/// `agents-zone-attention` decision 4: an allow option is destructive, a
/// reject an outline. (The working row's splice is the model's now,
/// `model.rs`'s `the_working_rows_splice_inserts_removes_or_leaves_it`.)
#[test]
fn the_permission_variants() {
    assert_eq!(permission_variant(PermissionKind::AllowOnce), ButtonVariant::Destructive);
    assert_eq!(permission_variant(PermissionKind::AllowAlways), ButtonVariant::Destructive);
    assert_eq!(permission_variant(PermissionKind::RejectOnce), ButtonVariant::Outline);
    assert_eq!(permission_variant(PermissionKind::RejectAlways), ButtonVariant::Outline);
}

/// `agents-zone-attention` decisions 1 and 2: the working row is the
/// scroller's last item after a prompt, follows a tool call in progress,
/// hides while a permission waits (which is drawn destructive), reads
/// `Stopping…` while a cancel is on its way and goes at the end, the view
/// wanting the motion clock exactly while it shows.
#[test]
fn the_working_row_ends_the_conversation_while_the_turn_works() {
    let mut f = Fixture::new();
    let shown = |f: &mut Fixture| {
        f.view.read_with(f.cx, |view, cx| {
            let model = view.agent.read(cx).model();
            let (blocks, working) = (model.blocks().len(), model.working_row());
            let items = view.scroller.read(cx).item_count();
            assert!(items == blocks || items == blocks + 1, "{items} items for {blocks} blocks");
            assert_eq!(items == blocks + 1, working.is_some(), "the row is the last item exactly while it shows");
            working.map(|row| row.text().to_string())
        })
    };
    let running = |f: &mut Fixture| f.cx.update(|_, cx| Motion::running(cx));
    f.feed(&[ready()]);
    assert_eq!(shown(&mut f), None);
    assert!(!running(&mut f));
    f.feed(&[added(1, user("hi")), working()]);
    assert_eq!(shown(&mut f).as_deref(), Some("Working…"));
    assert!(running(&mut f), "the view wants the clock");
    f.cx.run_until_parked();
    assert!(f.cx.debug_bounds("agent-working").is_some(), "the row is drawn");
    let tool = |status| {
        Entry::tool("call_1".into(), "Read main.rs".into(), crate::agent::transcript::ToolKind::Read, status)
    };
    f.feed(&[added(2, tool(crate::agent::transcript::ToolStatus::InProgress))]);
    assert_eq!(shown(&mut f).as_deref(), Some("Read main.rs"));
    f.feed(&[Event::Turn { state: TurnState::Blocked }, added(3, permission())]);
    assert_eq!(shown(&mut f), None, "the permission row is the call to action");
    assert!(!running(&mut f));
    f.cx.run_until_parked();
    assert!(f.cx.debug_bounds("confirmation-destructive").is_some(), "a pending request is drawn as a risk");
    f.feed(&[
        Event::Answered {
            id: EntryId(3),
            outcome: crate::agent::transcript::PermissionOutcome::Selected { option: "reject-once".into() },
        },
        working(),
    ]);
    assert_eq!(shown(&mut f).as_deref(), Some("Read main.rs"), "back once answered");
    f.cx.run_until_parked();
    assert!(f.cx.debug_bounds("confirmation-destructive").is_none(), "answered, it is plain");
    f.feed(&[Event::Turn { state: TurnState::Cancelling }]);
    assert_eq!(shown(&mut f).as_deref(), Some("Stopping…"));
    f.feed(&[ended(StopReason::Cancelled)]);
    assert_eq!(shown(&mut f), None, "gone at the end");
    assert!(!running(&mut f), "the view stopped wanting the clock");

    // A view released while its row shows doesn't keep wanting the clock.
    f.feed(&[added(4, user("again")), working()]);
    let agent = f.zone.agent.clone();
    let second = f.cx.update(|window, cx| cx.new(|cx| AgentView::new(agent, Theme::default(), None, window, cx)));
    let id = second.entity_id();
    assert!(f.cx.update(|_, cx| Motion::wants(id, cx)), "a new view of a working zone wants it");
    drop(second);
    f.cx.update(|_, _| {});
    f.cx.run_until_parked();
    assert!(!f.cx.update(|_, cx| Motion::wants(id, cx)), "released, it no longer wants the clock");
    assert!(running(&mut f), "the first view still does");
}

/// OpenSpec change `agents-zone-focus-and-scroll` decision 5 (E-6): one
/// paced chunk that ends the session while the working row shows, then adds
/// a notice, leaves the scroller with exactly the model's items, the notice
/// drawn and the row gone.
#[test]
fn an_exit_and_a_notice_in_one_chunk_leave_no_empty_item() {
    let mut f = Fixture::new();
    f.feed(&[ready(), added(1, user("hi")), working()]);
    let counts = |f: &mut Fixture| {
        f.view.read_with(f.cx, |view, cx| (view.scroller.read(cx).item_count(), view.agent.read(cx).model().item_count()))
    };
    assert_eq!(counts(&mut f), (2, 2), "the block and the row");
    // Both reach the agent's loop before it runs: one chunk.
    f.zone.sink.exited(Some(0));
    f.zone.feed(&[added(2, Entry::Notice { level: NoticeLevel::Warning, text: "the agent exited".into() })]);
    f.settle();
    assert_eq!(counts(&mut f), (2, 2), "the block and the notice, no stale row");
    f.cx.run_until_parked();
    assert!(f.cx.debug_bounds("agent-working").is_none(), "the row is gone");
}

/// The agent's order put allow first: the row turns it round.
fn permission_allow_first() -> Entry {
    let option = |id: &str, name: &str, kind| PermissionOption { id: id.into(), name: name.into(), kind };
    Entry::Permission {
        call: Some("call_1".into()),
        title: "Run cargo test?".into(),
        options: vec![
            option("allow-once", "Allow once", PermissionKind::AllowOnce),
            option("reject-once", "Reject", PermissionKind::RejectOnce),
        ],
        outcome: None,
    }
}

/// Tab walks the prompt, the waiting row's remove, the pending permission's
/// buttons (reject first, `agents-zone-attention` decision 4) and the stop
/// button, and wraps; with a held row and no turn, the prompt, send again
/// and discard.
#[test]
fn tab_walks_the_prompt_the_row_the_permissions_and_stop() {
    let mut f = Fixture::new();
    f.feed(&[ready(), added(1, user("hi")), working(), slot(waiting()), added(2, permission_allow_first())]);
    let prompt = f.prompt_handle();
    let remove = f.row_stops();
    assert_eq!(remove.len(), 1);
    let (allow, reject) = f.view.read_with(f.cx, |view, _| {
        let handle = |option: &str| view.permission_handle(EntryId(2), option).cloned().expect("pending");
        (handle("allow-once"), handle("reject-once"))
    });
    let stop = f.stop_button().expect("a turn runs");
    assert_eq!(f.tabs(5), [remove[0].clone(), reject, allow, stop, prompt.clone()]);
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

/// The focus line names each stop (task 6.7): the prompt, the waiting
/// row's remove, the permission's buttons by entry and option, stop; then
/// a held row's send again and discard.
#[test]
fn the_focus_line_names_each_stop() {
    let mut f = Fixture::new();
    f.feed(&[ready(), added(1, user("hi")), working(), slot(waiting()), added(2, permission())]);
    let mut names = Vec::new();
    for _ in 0..5 {
        let view = f.view.clone();
        names.push(f.cx.update(|window, cx| view.read(cx).focused_stop(window, cx)).unwrap_or_default());
        f.cx.simulate_keystrokes("tab");
    }
    assert_eq!(names, ["prompt", "queued:remove", "permission:2:reject-once", "permission:2:allow-once", "stop"]);
    f.focus_prompt();
    f.feed(&[
        Event::Answered {
            id: EntryId(2),
            outcome: crate::agent::transcript::PermissionOutcome::Selected { option: "allow-once".into() },
        },
        ended(StopReason::Cancelled),
        slot(held()),
    ]);
    let mut names = Vec::new();
    for _ in 0..3 {
        f.cx.simulate_keystrokes("tab");
        let view = f.view.clone();
        names.push(f.cx.update(|window, cx| view.read(cx).focused_stop(window, cx)).unwrap_or_default());
    }
    assert_eq!(names, ["queued:send-again", "queued:discard", "prompt"]);
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
    // A finished call's row, closed, opens on Toggle Tool Details.
    let mut tool = crate::agent::transcript::Entry::tool(
        "c1".into(),
        "Run".into(),
        crate::agent::transcript::ToolKind::Execute,
        crate::agent::transcript::ToolStatus::Completed,
    );
    if let crate::agent::transcript::Entry::Tool { input, .. } = &mut tool {
        *input = Some("ls".into());
    }
    f.feed(&[added(4, tool)]);
    let row = |f: &mut Fixture| {
        f.zone.agent.read_with(f.cx, |agent, _| {
            agent.model().blocks().iter().find_map(|b| match &b.body {
                BlockBody::Tool { state, .. } => Some(*state),
                _ => None,
            })
        })
    };
    assert_eq!(row(&mut f), Some(crate::agent::tool::RowState::Closed));
    f.cx.dispatch_action(crate::actions::ToggleAgentToolDetails);
    assert_eq!(row(&mut f), Some(crate::agent::tool::RowState::Open));
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
    let state = prompt_state(&model, &Offer::None(NoOffer::NotEnded));
    assert_eq!((state.status, state.queued.clone(), state.action()), (UiStatus::Disabled, None, None));
    assert_eq!(state.reason.as_ref().map(SharedString::as_ref), Some(DisabledWhy::Connecting.text()));
    assert_eq!(bubble_part(0, true), BubblePart::Whole);
    assert_eq!(bubble_part(0, false), BubblePart::First);
    assert_eq!(bubble_part(1, false), BubblePart::Middle);
    assert_eq!(bubble_part(2, true), BubblePart::Last);
    assert_eq!(queued_of(&QueueRow::Waiting { text: "a".into() }), Some(Queued::Waiting { text: "a".into() }));
    assert_eq!(tool_status(ToolStatus::InProgress), UiToolStatus::Running);
    assert_eq!(tool_icon(ToolKind::Execute), Icon::Terminal);
}

// `agents-session-recovery` 5.1: the Continue conversation button.

fn conversation(continuation: Continuation) -> Event {
    let conversation = Conversation { id: "sess-1".into(), cwd: "/home/u/src".into(), continuation, title: None };
    Event::Conversation { conversation }
}

fn exited() -> Event {
    Event::Agent { state: AgentState::Exited { code: Some(1), stderr: "boom".into() } }
}

/// A view of the `Fake` agent profile's zone (in a config that has it, or
/// not) whose agent ran one turn with a conversation `continuation`
/// offers, then exited.
fn ended_view(continuation: Continuation, config: Option<&str>) -> Fixture {
    let mut f = Fixture::with_config(config);
    f.feed(&[conversation(continuation), ready(), added(1, user("hi")), exited()]);
    f
}

fn action(f: &mut Fixture) -> Option<String> {
    f.view.read_with(f.cx, |view, cx| view.prompt.read(cx).action().map(|label| label.to_string()))
}

fn reason_text(f: &mut Fixture) -> Option<String> {
    f.reason().map(|reason| reason.to_string())
}

fn action_stop(f: &mut Fixture) -> Option<FocusHandle> {
    f.view.read_with(f.cx, |view, cx| view.prompt.read(cx).action_stop())
}

/// What the reason line shows for each offer: the button after the
/// exit's reason (or after a refused continue's words); no button and why
/// not for an ended agent that can't continue; a running or never-started
/// agent's own reason.
#[test]
fn the_reason_line_follows_the_offer() {
    use crate::agent::model::{DisabledWhy, StartFailure};
    let mut f = ended_view(Continuation::Resume, Some(FAKE_PROFILE));
    let text = |state: &PromptState| state.reason.as_ref().map(|r| r.to_string());
    let shown = |f: &mut Fixture, offer: Offer| f.zone.agent.read_with(f.cx, |agent, _| prompt_state(agent.model(), &offer));
    let state = shown(&mut f, Offer::Restart);
    assert_eq!(state.status, UiStatus::Disabled);
    assert_eq!((text(&state).as_deref(), state.action()), (Some(DisabledWhy::Exited.text()), Some(CONTINUE.into())));
    let all = [ZoneChoice::Continue, ZoneChoice::Conversations, ZoneChoice::New];
    assert_eq!(state.actions, all);
    for (why, says, actions) in [
        (NoOffer::NotOffered, "the agent can't continue conversations", &all[1..]),
        (NoOffer::NoConversation, "the agent never started a conversation", &all[1..]),
        (NoOffer::ProfileGone { name: "Fake".into() }, "the profile Fake is gone", &[][..]),
    ] {
        let state = shown(&mut f, Offer::None(why));
        assert_eq!((text(&state).as_deref(), state.actions.as_slice()), (Some(says), actions));
    }
    let mut g = Fixture::new();
    g.feed(&[ready()]);
    let state = shown(&mut g, Offer::None(NoOffer::NotEnded));
    assert_eq!((state.reason.clone(), state.action()), (None, None));
    let mut h = Fixture::new();
    h.zone.sink.failed(&anyhow::Error::new(StartFailure::NoServer("no helper".into())));
    h.settle();
    let state = shown(&mut h, Offer::None(NoOffer::NoConversation));
    assert_eq!(text(&state).as_deref(), Some(DisabledWhy::NotStarted.text()), "a never-started agent keeps its reason");
}

/// An exited agent that can continue shows the button, one Tab from the
/// prompt (`continue` on the focus line); enter on it sends the restart,
/// the button goes with its focus, and a refusal brings it back after the
/// refusal's words. `agent_continue` does the same.
#[test]
fn the_continue_button_and_agent_continue_restart_the_agent() {
    let mut f = ended_view(Continuation::Resume, Some(FAKE_PROFILE));
    assert_eq!(action(&mut f).as_deref(), Some(CONTINUE));
    let stop = action_stop(&mut f).expect("a stop");
    assert_eq!(f.tabs(1), [stop]);
    let view = f.view.clone();
    assert_eq!(f.cx.update(|window, cx| view.read(cx).focused_stop(window, cx)).as_deref(), Some("continue"));
    f.cx.simulate_keystrokes("enter");
    f.settle();
    assert!(matches!(f.calls().as_slice(), [Call::Restart(_)]));
    assert_eq!(action(&mut f), None, "starting, nothing to continue");
    assert_eq!(f.focused(), Some(f.prompt_handle()), "the button went, and its focus with it");

    f.zone.sink.reply(1, Err("session 7: the server is stopping; try again".into()));
    f.settle();
    assert_eq!(action(&mut f).as_deref(), Some(CONTINUE), "offered again");
    assert_eq!(reason_text(&mut f).as_deref(), Some("session 7: the server is stopping; try again"));
    f.cx.dispatch_action(AgentContinue);
    f.settle();
    assert!(matches!(f.calls().as_slice(), [Call::Restart(_)]));
}

/// No button where nothing can be continued: an agent that offered
/// neither load nor resume, and a profile gone from the config, which a
/// config change takes away from a zone already showing the button.
#[test]
fn no_continue_button_when_it_isnt_offered() {
    let mut f = ended_view(Continuation::Neither, Some(FAKE_PROFILE));
    // Conversations… and New conversation stay (`agents-session-list`).
    assert_eq!(action(&mut f).as_deref(), Some("Conversations…"));
    assert_eq!(reason_text(&mut f).as_deref(), Some("the agent can't continue conversations"));
    let stops = f.view.read_with(f.cx, |view, cx| view.prompt.read(cx).action_stops());
    assert_eq!(f.tabs(1), [stops[0].clone()], "Tab finds Conversations…, not a continue");
    f.cx.dispatch_action(AgentContinue);
    assert!(f.calls().is_empty());

    let mut g = ended_view(Continuation::Resume, Some(FAKE_PROFILE));
    let stop = action_stop(&mut g).expect("offered");
    g.focus(&stop);
    let mut app = g.cx.clone();
    set_config("[[profiles]]\nname = \"Shell\"\n", &mut app);
    g.settle();
    assert_eq!(action(&mut g), None);
    assert_eq!(reason_text(&mut g).as_deref(), Some("the profile Fake is gone"));
    assert_eq!(g.focused(), Some(g.prompt_handle()), "the button's focus went back to the prompt");
}

/// The plan of a Linux host called `here`, as the dump's tests use.
fn linux(uri: &str) -> Plan {
    links::plan(uri, "here", false)
}

/// 4.3: a press acts only with the terminal's gesture (ctrl, not alt);
/// then http and https open, other schemes the terminal opens ask, and
/// refusals say why on the config line.
#[test]
fn a_link_press_acts_only_with_ctrl() {
    let ctrl = Modifiers::control();
    let ctrl_alt = Modifiers { control: true, alt: true, ..Modifiers::default() };
    let shift = Modifiers::shift();
    assert_eq!(link_press("https://example.com", &Modifiers::none(), linux), None, "a plain click does nothing");
    assert_eq!(link_press("https://example.com", &ctrl_alt, linux), None);
    assert_eq!(link_press("https://example.com", &shift, linux), None);
    assert_eq!(link_press("https://example.com", &ctrl, linux), Some(LinkAction::Open));
    assert_eq!(link_press("mailto:a@example.com", &ctrl, linux), Some(LinkAction::Confirm));
    assert_eq!(
        link_press("javascript:void", &ctrl, linux),
        Some(LinkAction::Refuse(links::Refusal::Scheme("javascript".into())))
    );
    assert_eq!(link_step("https://example.com", LinkAction::Open), LinkStep::Open);
    assert_eq!(link_step("mailto:a@example.com", LinkAction::Confirm), LinkStep::Confirm);
    assert_eq!(
        link_step("javascript:void", LinkAction::Refuse(links::Refusal::Scheme("javascript".into()))),
        LinkStep::Fail("couldn't open javascript:void: its scheme (javascript) isn't one Brindle opens".into())
    );
    assert_eq!(
        link_step("file://otherhost/share/x", LinkAction::Refuse(links::Refusal::OtherHost("otherhost".into()))),
        LinkStep::Fail("couldn't open file://otherhost/share/x: it names another host (otherhost)".into())
    );
}

/// 4.1: a message's parts are one bubble, so only its outer edges are
/// padded; every other block is padded on both.
#[test]
fn only_a_messages_outer_edges_are_padded() {
    use crate::agent::markdown::render;
    let part = |part: usize, last: bool| Block {
        entry: EntryId(1),
        part,
        start: 0,
        last,
        body: BlockBody::AgentPart { block: render("a").remove(0).block },
    };
    assert_eq!(item_padding(&part(0, true)), (true, true));
    assert_eq!(item_padding(&part(0, false)), (true, false));
    assert_eq!(item_padding(&part(1, false)), (false, false));
    assert_eq!(item_padding(&part(2, true)), (false, true));
    let user = Block { body: BlockBody::UserText { text: "a".into() }, ..part(1, false) };
    assert_eq!(item_padding(&user), (false, false));
    let notice = Block { body: BlockBody::Notice { level: NoticeLevel::Info, text: "n".into() }, ..part(1, false) };
    assert_eq!(item_padding(&notice), (true, true));
}

/// 4.1 and 4.2: an agent message's code block is drawn as one, its copy
/// button writes its text to the clipboard, and Copy Last Code Block
/// copies the last one; with no code it leaves the clipboard alone.
#[test]
fn code_blocks_copy_from_their_button_and_the_palette() {
    let mut f = Fixture::new();
    f.feed(&[ready(), added(1, crate::agent::test_support::agent("No code here."))]);
    f.cx.write_to_clipboard(ClipboardItem::new_string("before".into()));
    f.cx.dispatch_action(CopyLastCodeBlock);
    assert_eq!(f.cx.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("before"), "untouched");
    f.feed(&[added(2, crate::agent::test_support::agent("Try:\n\n```rust\nfn a() {}\n```\n\n```\n  b\n```"))]);
    assert!(f.cx.debug_bounds("code-block-code").is_some(), "the code is drawn as a code block");
    f.cx.dispatch_action(CopyLastCodeBlock);
    assert_eq!(f.cx.read_from_clipboard().and_then(|item| item.text()).as_deref(), Some("  b\n"));
    let copy = f.cx.debug_bounds("code-block-copy").expect("a copy button");
    f.cx.write_to_clipboard(ClipboardItem::new_string("before".into()));
    f.cx.simulate_click(copy.center(), Modifiers::none());
    let copied = f.cx.read_from_clipboard().and_then(|item| item.text());
    assert!(matches!(copied.as_deref(), Some("fn a() {}\n" | "  b\n")), "a block's text exactly: {copied:?}");
}

/// 4.3: a link that asks emits `ConfirmLink` with the prompt to give focus
/// back to, a refused one `LinkFailed` with the terminal's words, and a
/// press without ctrl neither.
#[test]
fn link_presses_ask_or_fail_through_events() {
    use std::cell::RefCell;
    use std::rc::Rc;
    let mut f = Fixture::new();
    let asked = Rc::new(RefCell::new(Vec::<(String, FocusHandle)>::new()));
    let failed = Rc::new(RefCell::new(Vec::<String>::new()));
    let view = f.view.clone();
    let _subscriptions = f.cx.update(|_, cx| {
        let (asked, failed) = (asked.clone(), failed.clone());
        [
            cx.subscribe(&view, move |_, confirm: &ConfirmLink, _| {
                asked.borrow_mut().push((confirm.uri.clone(), confirm.back.clone()));
            }),
            cx.subscribe(&view, move |_, LinkFailed(text): &LinkFailed, _| failed.borrow_mut().push(text.clone())),
        ]
    });
    let press = |f: &mut Fixture, target: &str, modifiers: Modifiers| {
        let view = f.view.clone();
        f.cx.update(|_, cx| view.update(cx, |view, cx| view.press_link(target, modifiers, cx)));
        f.cx.run_until_parked();
    };
    press(&mut f, "mailto:dev@example.com", Modifiers::none());
    press(&mut f, "javascript:void", Modifiers::none());
    assert!(asked.borrow().is_empty() && failed.borrow().is_empty(), "a plain click does nothing");
    press(&mut f, "mailto:dev@example.com", Modifiers::control());
    let prompt = f.prompt_handle();
    assert_eq!(asked.borrow().iter().map(|(uri, back)| (uri.as_str(), back == &prompt)).collect::<Vec<_>>(), [(
        "mailto:dev@example.com",
        true
    )]);
    press(&mut f, "javascript:void", Modifiers::control());
    assert_eq!(*failed.borrow(), ["couldn't open javascript:void: its scheme (javascript) isn't one Brindle opens"]);
}

// OpenSpec change `agents-login`, task 7.2.

fn login_methods() -> Vec<crate::agent::transcript::LoginMethod> {
    use crate::agent::transcript::{LoginKind, LoginMethod};
    vec![
        LoginMethod { id: "google".into(), name: "Log in with Google".into(), description: None, kind: LoginKind::Agent },
        LoginMethod {
            id: "api-key".into(),
            name: "API Key".into(),
            description: Some("Use an API key".into()),
            kind: LoginKind::Agent,
        },
    ]
}

fn focused_name(f: &mut Fixture) -> String {
    let view = f.view.clone();
    f.cx.update(|window, cx| view.read(cx).focused_stop(window, cx)).unwrap_or_default()
}

/// The login row arrives with nothing focused; tab walks its methods in
/// the agent's order after the prompt, each named `login:ID`; enter on one
/// sends `AGENT_LOGIN` for it; while it runs, Cancel login is the one
/// stop (`login:cancel`) and sends `AGENT_CANCEL`; once the agent is
/// ready the row goes and focus returns to the prompt.
#[test]
fn the_login_row_walks_its_methods_and_cancel_login() {
    let mut f = Fixture::new();
    f.feed(&[Event::Agent { state: AgentState::LoginRequired { message: "log in".into(), methods: login_methods() } }]);
    let prompt = f.prompt_handle();
    assert_eq!(f.focused(), Some(prompt.clone()), "nothing is chosen for the user");
    let (google, key) = f.view.read_with(f.cx, |view, _| {
        (view.login_handle("google").cloned().expect("a stop"), view.login_handle("api-key").cloned().expect("a stop"))
    });
    assert_eq!(f.tabs(3), [google.clone(), key, prompt.clone()]);
    let mut names = Vec::new();
    for _ in 0..3 {
        f.cx.simulate_keystrokes("tab");
        names.push(focused_name(&mut f));
    }
    assert_eq!(names, ["login:google", "login:api-key", "prompt"]);

    f.focus(&google);
    f.cx.simulate_keystrokes("enter");
    f.settle();
    assert_eq!(f.calls(), [Call::Login("google".into())]);
    f.zone.sink.reply(1, Ok(()));
    f.feed(&[Event::Agent { state: AgentState::LoggingIn { method: "google".into(), name: "Log in with Google".into() } }]);
    assert_eq!(f.focused(), Some(prompt.clone()), "the method that had focus went");
    let cancel = f.view.read_with(f.cx, |view, _| view.cancel_login_handle().clone());
    assert_eq!(f.tabs(2), [cancel.clone(), prompt.clone()]);
    f.focus(&cancel);
    assert_eq!(focused_name(&mut f), "login:cancel");
    f.cx.simulate_keystrokes("enter");
    f.settle();
    assert_eq!(f.calls(), [Call::Cancel]);

    f.feed(&[Event::Agent { state: AgentState::Starting }, ready()]);
    assert_eq!(f.focused(), Some(prompt), "Cancel login went with the row");
    f.view.read_with(f.cx, |view, _| assert_eq!(view.login, crate::agent::model::LoginRow::None));
}

/// The row's words: its title while choosing and while one runs, and each
/// method's description with what a terminal login runs, or why it is
/// withheld.
#[test]
fn the_login_rows_words() {
    use crate::agent::login::{MethodOffer, TerminalLogin, Withheld};
    use crate::agent::model::{LoginRow, MethodChoice};
    let words = |text: Option<SharedString>| text.map(|text| text.to_string());
    assert_eq!(login_title(&LoginRow::None), None);
    let choose = LoginRow::Choose { message: "x".into(), methods: Vec::new() };
    assert_eq!(words(login_title(&choose)).as_deref(), Some("Log in to continue"));
    let running = LoginRow::Running { method: "g".into(), name: "Google".into(), terminal: false };
    assert_eq!(words(login_title(&running)).as_deref(), Some("Logging in with Google…"));
    let choice = |description: Option<&str>, offer| MethodChoice {
        id: "m".into(),
        name: "M".into(),
        description: description.map(|text| SharedString::from(text.to_string())),
        offer,
    };
    assert_eq!(method_description(&choice(None, MethodOffer::Agent)), None);
    assert_eq!(words(method_description(&choice(Some("Use `gh`"), MethodOffer::Agent))).as_deref(), Some("Use `gh`"));
    let plan = TerminalLogin {
        command: "agent".into(),
        args: vec!["--login".into()],
        appended: vec!["--login".into()],
        env: [("LOGIN_MODE".to_string(), "device".to_string())].into(),
        dropped: vec!["PATH".into()],
    };
    assert_eq!(
        words(method_description(&choice(Some("In a terminal"), MethodOffer::Terminal(plan)))).as_deref(),
        Some("In a terminal · Runs with --login; sets LOGIN_MODE=device; not applied: PATH")
    );
    let withheld = MethodOffer::Withheld(Withheld { why: "the login can't be run through `ssh`".into() });
    assert_eq!(words(method_description(&choice(None, withheld))).as_deref(), Some("the login can't be run through `ssh`"));
}

/// An agent view in a zone's card, as `zone_view.rs` draws one: alone, or
/// as the first of a horizontal split's two zones.
struct SplitHost {
    view: Entity<AgentView>,
    split: bool,
}

impl Render for SplitHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let card = |id: usize, child: AnyElement| {
            brindle_ui::Card::new(("zone", id)).size_full().min_w_0().min_h_0().child(
                div().id(("agent-zone", id)).size_full().min_w_0().min_h_0().child(child),
            )
        };
        let zone = card(1, self.view.clone().into_any_element());
        let root = div().w(px(800.)).h(px(500.)).flex().flex_col();
        if !self.split {
            return root.child(zone);
        }
        let half = |child: AnyElement| div().flex_1().min_w_0().min_h_0().size_full().child(child);
        root.child(
            div()
                .flex()
                .flex_row()
                .gap(px(crate::zone_view::ZONE_GAP))
                .size_full()
                .min_w_0()
                .min_h_0()
                .child(half(zone.into_any_element()))
                .child(half(card(2, div().into_any_element()).into_any_element())),
        )
    }
}

/// The spec's "Connecting after a split" in a view test: a connecting
/// agent zone's reason line reads across the zone at its full width and,
/// in the next frame, at half of it: at least as wide as `connecting`
/// (0.6em a character in gpui's test text system) and at most two of the
/// lines it takes at full width.
#[test]
fn the_connecting_line_reads_across_a_split_zone() {
    let mut app = app();
    let zone = Zone::new(None, &mut app);
    let agent = zone.agent.clone();
    let (host, cx) = app.add_window_view(|window, cx| {
        let view = cx.new(|cx| AgentView::new(agent, Theme::default(), None, window, cx));
        SplitHost { view, split: false }
    });
    settle(cx);
    let view = host.read_with(cx, |host, _| host.view.clone());
    assert_eq!(
        view.read_with(cx, |view, cx| view.prompt.read(cx).reason().map(|reason| reason.to_string())).as_deref(),
        Some("connecting to the terminal server")
    );
    let size = f32::from(view.read_with(cx, |view, cx| view.text_size(cx)));
    let connecting = px("connecting".len() as f32 * 0.6 * 0.75 * size);
    let full = cx.debug_bounds("prompt-reason").expect("drawn");
    assert!(full.size.height < px(2. * 0.75 * size), "at full width the reason isn't one line: {full:?}");
    assert!(full.size.width >= connecting, "{full:?}");
    cx.update(|_, cx| {
        host.update(cx, |host, cx| {
            host.split = true;
            cx.notify();
        })
    });
    settle(cx);
    let half = cx.debug_bounds("prompt-reason").expect("drawn");
    assert!(half.right() < px(400.), "the reason isn't in the split's first zone: {half:?}");
    assert!(half.size.width >= connecting, "after the split: {half:?}");
    assert!(half.size.height <= full.size.height * 2., "after the split: {half:?}, at full width {full:?}");
}

/// OpenSpec change `agents-log-paging` task 5.5: a zone attached with a
/// page shows the Earlier row above its first block; scrolled into view it
/// asks for the page before the window, and while that is on its way, with
/// no agent working, the view wants the motion clock for the row's loader;
/// the page goes in front and the clock stops.
#[test]
fn the_earlier_row_fetches_in_view_and_spins_until_the_page_comes() {
    use crate::agent::transcript::{PAGE_BYTES, Transcript};
    let mut f = Fixture::new();
    let mut server = Transcript::new();
    let mut events = vec![ready(), added(1, user("go"))];
    for n in 0..40 {
        events.push(added(n + 2, Entry::Agent { text: format!("{n} {}", "x".repeat(20 * 1024)) }));
    }
    events.push(ended(StopReason::EndTurn));
    for event in &events {
        server.apply(event).unwrap();
    }
    f.zone.sink.replay(true);
    f.zone.sink.transcript(server.page(PAGE_BYTES).iter().map(Event::to_value).collect()).unwrap();
    f.zone.sink.replay(false);
    f.settle();
    let window = f.zone.shared.lock().transcript().window().expect("800 KiB is paged");
    let (from, count) = (window.from, window.earlier);
    let earlier = |f: &mut Fixture| f.view.read_with(f.cx, |view, _| view.earlier.clone());
    let running = |f: &mut Fixture| f.cx.update(|_, cx| Motion::running(cx));
    assert_eq!(earlier(&mut f), Earlier::Available { count });
    assert!(!running(&mut f));
    let fetches = |f: &mut Fixture| f.zone.take_calls().into_iter().filter(|call| matches!(call, Call::Fetch(..))).count();
    assert_eq!(fetches(&mut f), 0, "the row isn't in view at the end");

    // Paged up to the row, the zone asks for the page before the window.
    for _ in 0..40 {
        if matches!(earlier(&mut f), Earlier::Loading { .. }) {
            break;
        }
        f.view.update(f.cx, |view, cx| view.page(PageDirection::Up, cx));
        f.cx.run_until_parked();
    }
    assert!(matches!(earlier(&mut f), Earlier::Loading { .. }), "no fetch: {:?}", earlier(&mut f));
    let calls = f.zone.calls.borrow().clone();
    assert_eq!(calls.last(), Some(&Call::Fetch(from.0, PAGE_BYTES as u32)), "{calls:?}");
    let request = f.zone.last_request();
    assert!(running(&mut f), "the loader turns while the page is on its way");
    assert!(f.cx.debug_bounds("agent-earlier").is_some(), "the row is drawn");

    // The page, then its reply: in front, and the clock stops.
    let blocks = |f: &mut Fixture| f.view.read_with(f.cx, |view, cx| view.agent.read(cx).model().blocks().len());
    let before = blocks(&mut f);
    let fetched = server.fetch(from, PAGE_BYTES);
    f.zone.sink.page(request, fetched.events.iter().map(Event::to_value).collect(), Some(fetched.more)).unwrap();
    f.zone.sink.reply(request, Ok(()));
    f.settle();
    assert!(blocks(&mut f) > before, "the page went in front");
    assert!(matches!(earlier(&mut f), Earlier::Available { count: left } if left < count) || earlier(&mut f) == Earlier::None);
    assert!(!running(&mut f), "no agent works and nothing loads");
}

// OpenSpec change `agents-zone-focus-and-scroll`, task 7.1.

/// A turn running over twenty long agent messages, taller than the window.
fn long_turn(f: &mut Fixture) {
    let mut events = vec![ready(), added(1, user("go")), working()];
    for n in 0..20u64 {
        events.push(added(n + 2, crate::agent::test_support::agent(&format!("{n} {}", "word ".repeat(600)))));
    }
    f.feed(&events);
}

/// Where the last frame drew item `ix`, if it did, and the scroller's view.
fn drawn(f: &mut Fixture, ix: usize) -> (Option<gpui::Bounds<Pixels>>, gpui::Bounds<Pixels>) {
    let viewport = f.cx.debug_bounds("message-scroller").expect("the scroller is drawn");
    (f.view.read_with(f.cx, |view, cx| view.scroller.read(cx).item_bounds(ix)), viewport)
}

fn permission_block(f: &mut Fixture, entry: u64) -> usize {
    f.view.read_with(f.cx, |view, cx| view.agent.read(cx).model().permission_block(EntryId(entry))).expect("pending")
}

fn page_up(f: &mut Fixture) {
    f.view.update(f.cx, |view, cx| view.page(PageDirection::Up, cx));
    f.settle();
}

/// The spec's "Tab to a permission below the view": away from the end, a
/// request arrives below the view; Tab from the prompt focuses its first
/// button and brings the block wholly into view, and enter answers it.
#[test]
fn tab_to_a_permission_below_the_view_reveals_it_and_enter_answers_it() {
    let mut f = Fixture::new();
    long_turn(&mut f);
    for _ in 0..3 {
        page_up(&mut f);
    }
    f.feed(&[added(22, permission())]);
    let block = permission_block(&mut f, 22);
    assert_eq!(drawn(&mut f, block).0, None, "below the view");
    let reject = f.view.read_with(f.cx, |view, _| view.permission_handle(EntryId(22), "reject-once").cloned()).unwrap();
    f.cx.simulate_keystrokes("tab");
    f.settle();
    assert_eq!(f.focused(), Some(reject));
    let (bounds, viewport) = drawn(&mut f, block);
    let bounds = bounds.expect("revealed");
    assert!(
        bounds.top() >= viewport.top() - px(0.5) && bounds.bottom() <= viewport.bottom() + px(0.5),
        "{bounds:?} wholly in {viewport:?}"
    );
    f.calls();
    f.cx.simulate_keystrokes("enter");
    assert_eq!(f.calls(), [Call::Answer(22, Some("reject-once".into()))]);
}

/// The spec's "A permission already in view": Tab to it moves nothing.
#[test]
fn a_permission_in_view_is_not_scrolled_to() {
    let mut f = Fixture::new();
    long_turn(&mut f);
    f.feed(&[added(22, permission())]);
    let block = permission_block(&mut f, 22);
    let before = drawn(&mut f, block).0.expect("in view at the end");
    f.cx.simulate_keystrokes("tab");
    f.settle();
    assert_eq!(drawn(&mut f, block).0, Some(before));
    assert_eq!(f.view.read_with(f.cx, |view, cx| view.scroller.read(cx).follow()), brindle_ui::Follow::AtEnd);
}

/// The spec's "A focused permission scrolled out of view": focus goes back
/// to the prompt and the request stays pending.
#[test]
fn scrolling_a_focused_permission_out_of_view_gives_focus_to_the_prompt() {
    let mut f = Fixture::new();
    long_turn(&mut f);
    f.feed(&[added(22, permission())]);
    let block = permission_block(&mut f, 22);
    let prompt = f.prompt_handle();
    f.cx.simulate_keystrokes("tab");
    assert_ne!(f.focused(), Some(prompt.clone()), "on the permission");
    for _ in 0..3 {
        page_up(&mut f);
    }
    assert_eq!(drawn(&mut f, block).0, None, "out of view");
    assert_eq!(f.focused(), Some(prompt));
    assert!(!f.calls().iter().any(|call| matches!(call, Call::Answer(..))), "nothing answered");
    assert_eq!(permission_block(&mut f, 22), block, "still pending");
}

// OpenSpec change `agents-zone-focus-and-scroll`, task 6.1.

/// What an item's height depends on besides its text and the width: the
/// size and the two families, and nothing else.
#[test]
fn a_measure_differs_on_size_or_either_family_only() {
    let measure = Measure { size: px(14.), ui_family: "Inter".into(), code_family: "Mono".into() };
    assert_eq!(measure.clone(), Measure { size: px(14.), ui_family: "Inter".into(), code_family: "Mono".into() });
    assert_ne!(measure, Measure { size: px(15.), ..measure.clone() });
    assert_ne!(measure, Measure { ui_family: "Other".into(), ..measure.clone() });
    assert_ne!(measure, Measure { code_family: "Other".into(), ..measure.clone() });
}

/// A view in an 800×500 zone (`SplitHost`, unsplit), its conversation ten
/// long single-paragraph agent messages, each one tall list item.
fn tall_view() -> (Zone, Entity<AgentView>, &'static mut VisualTestContext) {
    let mut app = app();
    let mut zone = Zone::new(None, &mut app);
    let agent = zone.agent.clone();
    let (host, cx) = app.add_window_view(|window, cx| {
        let view = cx.new(|cx| AgentView::new(agent, Theme::default(), None, window, cx));
        SplitHost { view, split: false }
    });
    let cx: &'static mut VisualTestContext = Box::leak(Box::new(cx.clone()));
    let mut events = vec![ready(), added(1, user("go"))];
    for n in 0..10u64 {
        let text = format!("{n} {}", "word ".repeat(600));
        events.push(added(n + 2, crate::agent::test_support::agent(&text)));
    }
    events.push(ended(StopReason::EndTurn));
    zone.feed(&events);
    settle(cx);
    let view = host.read_with(cx, |host, _| host.view.clone());
    (zone, view, cx)
}

/// Pages up from the end until an item's middle is at the view's top.
fn away_in_an_item(view: &Entity<AgentView>, cx: &mut VisualTestContext) -> gpui::ListOffset {
    for _ in 0..10 {
        view.update(cx, |view, cx| view.page(PageDirection::Up, cx));
        cx.run_until_parked();
        let top = view.read_with(cx, |view, cx| view.scroller.read(cx).scroll_top());
        if top.offset_in_item > px(0.) && top.item_ix > 2 {
            return top;
        }
    }
    panic!("no item's middle reached the top");
}

/// The item at the view's top: its index, where it is drawn and how far
/// into it the view starts, as a fraction of its height.
fn top_item(view: &Entity<AgentView>, cx: &mut VisualTestContext) -> (usize, gpui::Bounds<Pixels>, f32) {
    let viewport = cx.debug_bounds("message-scroller").expect("the scroller is drawn");
    view.read_with(cx, |view, cx| {
        let scroller = view.scroller.read(cx);
        let ix = scroller.scroll_top().item_ix;
        let bounds = scroller.item_bounds(ix).expect("the top item is drawn");
        (ix, bounds, f32::from(viewport.top() - bounds.top()) / f32::from(bounds.size.height))
    })
}

/// The spec's "A config reload while reading back": a theme change at the
/// same size leaves the view where it is.
#[test]
fn a_reload_with_the_same_size_leaves_the_view_where_it_is() {
    let (_zone, view, cx) = tall_view();
    let before = away_in_an_item(&view, cx);
    let dark = crate::theme::builtin_theme("brindle-dark").expect("a built-in");
    view.update(cx, |view, cx| view.set_profile(dark, None, cx));
    cx.run_until_parked();
    let after = view.read_with(cx, |view, cx| view.scroller.read(cx).scroll_top());
    assert_eq!((after.item_ix, after.offset_in_item), (before.item_ix, before.offset_in_item));
}

/// The spec's "A zoom while reading back": the item at the top stays at
/// the same fraction of its new height, and a page up moves the view by a
/// page through items above measured at the new size, not the old.
#[test]
fn a_zoom_remeasures_and_keeps_the_top_entry() {
    let (_zone, view, cx) = tall_view();
    away_in_an_item(&view, cx);
    let (ix, _, fraction) = top_item(&view, cx);
    cx.update(|_, cx| gpui::BorrowAppContext::update_global::<Settings, _>(cx, |settings, _| settings.zoom += 6.));
    cx.run_until_parked();
    let (after, bounds, zoomed) = top_item(&view, cx);
    assert_eq!(after, ix, "the same entry at the top");
    assert!((zoomed - fraction).abs() < 0.01, "at {zoomed} of its height, {fraction} before the zoom");
    let viewport = cx.debug_bounds("message-scroller").expect("drawn");
    let page = f32::from(viewport.size.height) - 24.;
    view.update(cx, |view, cx| view.page(PageDirection::Up, cx));
    cx.run_until_parked();
    let moved = view.read_with(cx, |view, cx| view.scroller.read(cx).item_bounds(ix)).expect("still in view");
    assert!(
        (f32::from(moved.top() - bounds.top()) - page).abs() < 1.,
        "a page up moved the entry from {:?} to {:?}, not by {page}",
        bounds.top(),
        moved.top()
    );
}

/// The spec's "Reading a code block while it streams": away with a growing
/// agent part's middle at the top, more lines arrive and its top stays.
#[test]
fn a_streaming_fence_at_the_top_keeps_its_lines() {
    let mut app = app();
    let mut zone = Zone::new(None, &mut app);
    let agent = zone.agent.clone();
    let (host, cx) = app.add_window_view(|window, cx| {
        let view = cx.new(|cx| AgentView::new(agent, Theme::default(), None, window, cx));
        SplitHost { view, split: false }
    });
    let cx: &'static mut VisualTestContext = Box::leak(Box::new(cx.clone()));
    let lines = |from: usize, to: usize| (from..to).map(|n| format!("line {n}\n")).collect::<String>();
    zone.feed(&[
        ready(),
        added(1, user("go")),
        working(),
        added(2, crate::agent::test_support::agent(&format!("```\n{}", lines(0, 80)))),
    ]);
    settle(cx);
    let view = host.read_with(cx, |host, _| host.view.clone());
    // Up into the fence, the conversation's last block.
    let fence = view.read_with(cx, |view, cx| view.agent.read(cx).model().blocks().len() - 1);
    let mut top = None;
    for _ in 0..10 {
        view.update(cx, |view, cx| view.page(PageDirection::Up, cx));
        cx.run_until_parked();
        let at = view.read_with(cx, |view, cx| view.scroller.read(cx).scroll_top());
        if at.item_ix == fence && at.offset_in_item > px(0.) {
            top = Some(at);
            break;
        }
    }
    assert!(top.is_some(), "the fence's middle never reached the top");
    let (ix, before, _) = top_item(&view, cx);
    zone.feed(&[Event::Appended { id: EntryId(2), text: lines(80, 120) }]);
    settle(cx);
    let (after_ix, after, _) = top_item(&view, cx);
    assert_eq!((after_ix, after.top()), (ix, before.top()), "the fence's lines in view stayed put");
    assert!(after.size.height > before.size.height, "it grew");
}

/// A view that its host can stop drawing, as a hidden tab's zone.
struct HideHost {
    view: Entity<AgentView>,
    shown: bool,
}

impl Render for HideHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let root = div().w(px(800.)).h(px(500.)).flex().flex_col();
        if self.shown { root.child(self.view.clone()) } else { root }
    }
}

/// The spec's "A page that arrives in a hidden tab" (E-4): a short page
/// shows the Earlier row in view at the end, so a fetch is asked; the view
/// stops being drawn, the page arrives, and no other page is asked until
/// a frame shows the row in view again.
#[test]
fn a_page_that_arrives_in_a_hidden_zone_asks_for_no_other() {
    use crate::agent::transcript::{PAGE_BYTES, Transcript};
    let mut app = app();
    let mut zone = Zone::new(None, &mut app);
    let agent = zone.agent.clone();
    let (host, cx) = app.add_window_view(|window, cx| {
        let view = cx.new(|cx| AgentView::new(agent, Theme::default(), None, window, cx));
        HideHost { view, shown: true }
    });
    let cx: &'static mut VisualTestContext = Box::leak(Box::new(cx.clone()));
    // Thoughts, each closed once another follows: a page of them is a few
    // short rows, so the Earlier row is in view at the end.
    let mut server = Transcript::new();
    let mut events = vec![ready(), added(1, user("go"))];
    for n in 0..40u64 {
        events.push(added(n + 2, Entry::Thought { text: format!("{n} {}", "x".repeat(100 * 1024)) }));
    }
    events.push(ended(StopReason::EndTurn));
    for event in &events {
        server.apply(event).unwrap();
    }
    zone.sink.replay(true);
    zone.sink.transcript(server.page(PAGE_BYTES).iter().map(Event::to_value).collect()).unwrap();
    zone.sink.replay(false);
    settle(cx);
    let fetches = |zone: &Zone| zone.calls.borrow().iter().filter(|call| matches!(call, Call::Fetch(..))).count();
    assert_eq!(fetches(&zone), 1, "the row in view at the end asks: {:?}", zone.calls.borrow());
    let request = zone.last_request();

    let host = host.clone();
    let set_shown = move |shown: bool, cx: &mut VisualTestContext| {
        cx.update(|_, cx| {
            host.update(cx, |host, cx| {
                host.shown = shown;
                cx.notify();
            })
        });
        cx.run_until_parked();
    };
    set_shown(false, cx);
    let window = zone.shared.lock().transcript().window().expect("paged");
    let fetched = server.fetch(window.from, PAGE_BYTES);
    zone.sink.page(request, fetched.events.iter().map(Event::to_value).collect(), Some(fetched.more)).unwrap();
    zone.sink.reply(request, Ok(()));
    settle(cx);
    assert_eq!(fetches(&zone), 1, "a hidden zone asks for no other page: {:?}", zone.calls.borrow());

    set_shown(true, cx);
    settle(cx);
    assert_eq!(fetches(&zone), 2, "shown with the row in view, it asks again");
}
