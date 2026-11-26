//! The `Agent` entity over a real sink and fake requests (task 5.1).

use super::test_support::{Call, Zone, added, agent, app, ready, settle, user};
use super::*;
use crate::agent::model::{AgentPhase, UNSENT};
use crate::agent::transcript::{AgentState, Event, Slot, TurnState};

fn drained<T>(rx: &mut futures::channel::mpsc::UnboundedReceiver<T>) -> Vec<T> {
    std::iter::from_fn(|| rx.try_next().ok().flatten()).collect()
}

/// Events fed through the sink reach the model, in order; the entity says
/// what changed (splices, the status with its notice, the title) and the
/// first prompt goes once, on the first live `Ready`.
#[test]
fn events_through_the_sink_reach_the_model_and_the_first_prompt_goes_once() {
    let mut cx = app();
    let mut zone = Zone::new(Some("hello there"), &mut cx);
    let mut terminal_events = cx.events::<TerminalEvent, _>(&zone.agent);
    let mut updates = cx.events::<AgentUpdate, _>(&zone.agent);

    zone.feed(&[Event::Agent { state: AgentState::Starting }]);
    settle(&cx);
    assert!(zone.take_calls().is_empty(), "not sent while starting");
    assert_eq!(drained(&mut terminal_events), [TerminalEvent::Status(None)], "live: idle");
    zone.agent.read_with(&cx, |agent, _| {
        assert_eq!(agent.model().phase(), &AgentPhase::Live(AgentState::Starting));
        assert_eq!(agent.title(), "Fake");
    });

    zone.feed(&[ready()]);
    settle(&cx);
    assert_eq!(zone.take_calls(), [Call::Prompt("hello there".into())]);
    assert_eq!(
        drained(&mut terminal_events),
        [TerminalEvent::Status(None), TerminalEvent::TitleChanged],
        "ready, and named by the agent"
    );
    zone.agent.read_with(&cx, |agent, _| {
        assert_eq!(agent.title(), "Fake Agent");
        assert_eq!(agent.status(), Some(State::Idle));
        assert_eq!(agent.model().prompt_status(), model::PromptStatus::Submitted, "the first prompt is in flight");
    });

    // The turn: the user's entry and the agent's reply are folded and
    // drawn; the reply to the first prompt changes no draft.
    zone.sink.reply(1, Ok(()));
    zone.feed(&[added(1, user("hello there")), Event::Turn { state: TurnState::Working }, added(2, agent("Hi!"))]);
    settle(&cx);
    let splices = drained(&mut updates);
    assert!(
        splices.iter().all(|update| matches!(update, AgentUpdate::Splices(_))),
        "only the list changes: {splices:?}"
    );
    assert_eq!(drained(&mut terminal_events), [TerminalEvent::Status(None)], "working");
    zone.feed(&[Event::Turn { state: TurnState::Ended { stop: crate::agent::transcript::StopReason::EndTurn } }]);
    settle(&cx);
    let events = drained(&mut terminal_events);
    let [TerminalEvent::Status(Some(notice))] = events.as_slice() else { panic!("a done notice: {events:?}") };
    assert_eq!((notice.source, notice.title.as_str(), notice.body.as_str()), (Source::Agent, "Fake Agent", "Done"));
    zone.agent.read_with(&cx, |agent, _| {
        assert_eq!(agent.model().blocks().len(), 2);
        assert_eq!(agent.status(), Some(State::Done));
    });

    // A replay (a reattach) sends nothing again.
    zone.sink.replay(true);
    zone.feed(&[ready(), added(1, user("hello there")), added(2, agent("Hi!"))]);
    zone.sink.replay(false);
    settle(&cx);
    assert!(zone.take_calls().is_empty(), "the first prompt went once");
    assert_eq!(zone.agent.read_with(&cx, |agent, _| agent.model().blocks().len()), 2);
}

/// A prompt goes through the model's decision; its answer clears the draft
/// or shows the refusal; a request that couldn't go is refused locally.
#[test]
fn prompts_go_through_the_model_and_their_answers_come_back() {
    let mut cx = app();
    let mut zone = Zone::new(None, &mut cx);
    let mut updates = cx.events::<AgentUpdate, _>(&zone.agent);
    zone.feed(&[ready()]);
    settle(&cx);
    zone.agent.update(&mut cx, |agent, cx| agent.prompt("  ", false, cx));
    zone.agent.update(&mut cx, |agent, cx| agent.prompt("go", true, cx));
    assert!(zone.take_calls().is_empty(), "blank and composing send nothing");
    zone.agent.update(&mut cx, |agent, cx| agent.prompt("go", false, cx));
    assert_eq!(zone.take_calls(), [Call::Prompt("go".into())]);
    zone.sink.reply(1, Ok(()));
    settle(&cx);
    assert_eq!(drained(&mut updates), [AgentUpdate::ClearDraft]);

    zone.agent.update(&mut cx, |agent, cx| agent.prompt("again", false, cx));
    assert_eq!(zone.take_calls(), [Call::Prompt("again".into())]);
    zone.sink.reply(2, Err("the agent isn't ready".into()));
    settle(&cx);
    assert!(drained(&mut updates).is_empty(), "a refused prompt keeps its draft");
    zone.agent.read_with(&cx, |agent, _| assert_eq!(agent.model().refusal(), Some("the agent isn't ready")));

    zone.down.set(true);
    zone.agent.update(&mut cx, |agent, cx| agent.prompt("lost", false, cx));
    zone.agent.read_with(&cx, |agent, _| assert_eq!(agent.model().refusal(), Some(UNSENT)));
    zone.down.set(false);

    // Cancel, resend, unqueue and answer go only when the model has
    // something for them.
    zone.agent.update(&mut cx, |agent, cx| {
        agent.cancel(cx);
        agent.resend(cx);
        agent.unqueue(cx);
        agent.answer(EntryId(9), "allow-once", cx);
    });
    assert_eq!(zone.take_calls(), [Call::Prompt("lost".into())], "nothing else to act on");
    zone.feed(&[Event::Turn { state: TurnState::Working }, Event::Queue { slot: Slot::Waiting { text: "next".into() } }]);
    settle(&cx);
    zone.agent.update(&mut cx, |agent, cx| {
        agent.cancel(cx);
        agent.unqueue(cx);
    });
    assert_eq!(zone.take_calls(), [Call::Cancel, Call::Unqueue]);
}

/// A first prompt the agent never got ready for becomes the draft.
#[test]
fn a_first_prompt_becomes_the_draft_when_the_agent_fails_to_start() {
    let mut cx = app();
    let mut zone = Zone::new(Some("do it"), &mut cx);
    let mut updates = cx.events::<AgentUpdate, _>(&zone.agent);
    zone.sink.failed(&anyhow::anyhow!("no helper"));
    settle(&cx);
    assert_eq!(drained(&mut updates), [AgentUpdate::SetDraft("do it".into())]);
    assert!(zone.take_calls().is_empty());
    zone.agent.read_with(&cx, |agent, _| {
        assert!(matches!(agent.model().phase(), AgentPhase::NotStarted(_)));
        assert_eq!(agent.server_session(), None, "a session that never started isn't kept");
    });
}

/// The link going down and coming back, the exit, closing and quitting.
#[test]
fn life_events_and_leaving_the_session() {
    let mut cx = app();
    let mut zone = Zone::new(None, &mut cx);
    zone.feed(&[ready()]);
    zone.sink.link_down();
    settle(&cx);
    zone.agent.read_with(&cx, |agent, _| {
        assert_eq!(agent.model().link(), Link::Reconnecting);
        assert_eq!(agent.server_session(), Some((7, None)));
    });
    zone.sink.replay(true);
    zone.feed(&[ready()]);
    zone.sink.replay(false);
    settle(&cx);
    zone.agent.read_with(&cx, |agent, _| assert_eq!(agent.model().link(), Link::Up));

    zone.agent.read_with(&cx, |agent, _| agent.set_layout(Some(serde_json::json!({"zones": 1}))));
    zone.agent.update(&mut cx, |agent, _| {
        agent.quit_with(None);
        agent.close();
    });
    assert_eq!(
        zone.take_calls(),
        [Call::Layout(Some(serde_json::json!({"zones": 1}))), Call::Detach(None)],
        "a quit detaches once; nothing more goes"
    );
    zone.agent.read_with(&cx, |agent, _| assert_eq!(agent.server_session(), None));

    let mut ended = Zone::new(None, &mut cx);
    ended.feed(&[ready()]);
    ended.sink.exited(Some(0));
    settle(&cx);
    ended.agent.read_with(&cx, |agent, _| assert_eq!(agent.server_session(), None, "an ended session isn't kept"));
    ended.agent.update(&mut cx, |agent, _| agent.close());
    assert_eq!(ended.take_calls(), [Call::Close]);
}
