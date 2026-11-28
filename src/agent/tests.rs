//! The `Agent` entity over a real sink and fake requests (task 5.1).

use super::test_support::{Call, FAKE_PROFILE, Start, Zone, added, agent, app, ready, set_config, settle, user};
use super::*;
use crate::agent::model::{AgentPhase, NoOffer, NotSent, UNSENT};
use crate::agent::transcript::{AgentState, Continuation, Entry, Event, NoticeLevel, Slot, TurnState};

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
        [TerminalEvent::Status(None)],
        "ready; the agent's own name doesn't retitle the zone"
    );
    zone.agent.read_with(&cx, |agent, _| {
        assert_eq!(agent.title(), "Fake");
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
    // The working row's insert, after the blocks' (OpenSpec change
    // `agents-zone-focus-and-scroll`, decision 5).
    let blocks = zone.agent.read_with(&cx, |agent, _| agent.model().blocks().len());
    let last = splices
        .iter()
        .rev()
        .find_map(|update| if let AgentUpdate::Splices(splices) = update { splices.last().cloned() } else { None });
    assert_eq!(last, Some(model::Splice { range: blocks..blocks, count: 1 }), "the working row, last");
    assert_eq!(zone.agent.read_with(&cx, |agent, _| agent.model().working_row()), Some(model::WorkingRow::Working));
    assert_eq!(drained(&mut terminal_events), [TerminalEvent::Status(None)], "working");
    zone.feed(&[Event::Turn { state: TurnState::Ended { stop: crate::agent::transcript::StopReason::EndTurn } }]);
    settle(&cx);
    let events = drained(&mut terminal_events);
    let [TerminalEvent::Status(Some(notice))] = events.as_slice() else { panic!("a done notice: {events:?}") };
    assert_eq!((notice.source, notice.title.as_str(), notice.body.as_str()), (Source::Agent, "Fake", "Done"));
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

    zone.refuse.set(Some(NotSent::Gone));
    zone.agent.update(&mut cx, |agent, cx| agent.prompt("lost", false, cx));
    zone.agent.read_with(&cx, |agent, _| assert_eq!(agent.model().refusal(), Some(UNSENT)));
    zone.refuse.set(None);

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

/// Once a quit has left the session, nothing more goes through it: no
/// prompt, layout, close or second detach.
#[test]
fn a_quit_leaves_and_sends_nothing_more() {
    let mut cx = app();
    let mut zone = Zone::new(None, &mut cx);
    zone.feed(&[ready()]);
    settle(&cx);
    let layout = Some(serde_json::json!({"zones": 1}));
    zone.agent.update(&mut cx, |agent, cx| {
        agent.quit_with(layout.clone());
        agent.prompt("after the quit", false, cx);
        agent.set_layout(layout.clone());
        agent.close();
        agent.quit_with(None);
    });
    assert_eq!(zone.take_calls(), [Call::Detach(layout)], "one detach, then nothing");
    zone.agent.read_with(&cx, |agent, _| assert_eq!(agent.server_session(), None));
}

/// A zone whose start failed has no session to send to: its input is
/// disabled as not started, a prompt sends nothing, and so does leaving.
#[test]
fn a_failed_start_refuses_a_prompt_as_today() {
    let mut cx = app();
    let mut zone = Zone::no_server(&mut cx);
    zone.sink.failed(&anyhow::anyhow!("the terminal server is too old"));
    settle(&cx);
    zone.agent.update(&mut cx, |agent, cx| agent.prompt("hello", false, cx));
    zone.agent.read_with(&cx, |agent, _| {
        assert_eq!(agent.model().prompt_status(), model::PromptStatus::Disabled(model::DisabledWhy::NotStarted));
        assert_eq!(agent.server_session(), None);
    });
    zone.agent.update(&mut cx, |agent, _| {
        agent.set_layout(None);
        agent.close();
    });
    assert!(zone.take_calls().is_empty());
}

// `agents-session-recovery` 4.3: continuing a conversation.

fn conversation(continuation: Continuation) -> Event {
    let conversation = Conversation { id: "sess-1".into(), cwd: "/home/u/src".into(), continuation, title: None };
    Event::Conversation { conversation }
}

fn exited() -> Event {
    Event::Agent { state: AgentState::Exited { code: Some(1), stderr: "boom".into() } }
}

fn notice(text: &str) -> Entry {
    Entry::Notice { level: NoticeLevel::Info, text: text.into() }
}

/// A zone of the `Fake` agent profile whose agent ran one turn, with a
/// conversation `continuation` offers, then exited.
fn ended_zone(continuation: Continuation, cx: &mut gpui::TestAppContext) -> Zone {
    set_config(FAKE_PROFILE, cx);
    let mut zone = Zone::new(None, cx);
    zone.feed(&[conversation(continuation), ready(), added(1, user("hi")), added(2, agent("Hello!")), exited()]);
    settle(cx);
    zone
}

/// `Restart`: `AGENT_RESTART` with the profile's command, arguments and
/// environment (as a new zone's: no terminal variables); the zone says the
/// agent is starting, then the continue's events show the agent ready on
/// the same session, the earlier turn kept.
#[test]
fn a_restart_sends_agent_restart_and_the_agent_comes_back_on_the_same_session() {
    let mut cx = app();
    let mut zone = ended_zone(Continuation::Resume, &mut cx);
    zone.agent.read_with(&cx, |agent, cx| assert_eq!(agent.offer(cx), Offer::Restart));
    zone.agent.update(&mut cx, |agent, cx| agent.continue_conversation(cx));
    let calls = zone.take_calls();
    let [Call::Restart(request)] = calls.as_slice() else { panic!("one restart: {calls:?}") };
    assert_eq!((request.command.as_str(), request.args.as_slice()), ("fake-acp", &["--acp".to_string()][..]));
    for name in crate::agent::spawn::TERMINAL_ONLY {
        assert!(!request.env.contains_key(name), "{name}");
    }
    assert!(zone.take_starts().is_empty(), "the same session");
    zone.agent.read_with(&cx, |agent, _| {
        assert_eq!(agent.model().phase(), &AgentPhase::Live(AgentState::Starting));
    });

    zone.sink.reply(1, Ok(()));
    zone.feed(&[
        added(3, notice("the agent was started again to continue the conversation")),
        Event::Agent { state: AgentState::Starting },
        conversation(Continuation::Resume),
        ready(),
    ]);
    settle(&cx);
    zone.agent.read_with(&cx, |agent, _| {
        assert!(matches!(agent.model().phase(), AgentPhase::Live(AgentState::Ready { .. })));
        assert_eq!(agent.model().blocks().len(), 3, "the earlier turn and the notice");
        assert_eq!(agent.server_session(), Some((7, None)));
    });
    zone.agent.update(&mut cx, |agent, cx| agent.prompt("again", false, cx));
    assert_eq!(zone.take_calls(), [Call::Prompt("again".into())]);
}

/// A refused restart shows the refusal and the ended agent again.
#[test]
fn a_refused_restart_shows_why() {
    let mut cx = app();
    let mut zone = ended_zone(Continuation::Resume, &mut cx);
    zone.agent.update(&mut cx, |agent, cx| agent.continue_conversation(cx));
    assert!(matches!(zone.take_calls().as_slice(), [Call::Restart(_)]));
    zone.sink.reply(1, Err("session 7: the agent can't continue this conversation".into()));
    settle(&cx);
    zone.agent.read_with(&cx, |agent, cx| {
        assert!(matches!(agent.model().phase(), AgentPhase::Live(AgentState::Exited { .. })));
        assert_eq!(agent.model().refusal(), Some("session 7: the agent can't continue this conversation"));
        assert_eq!(agent.offer(cx), Offer::Restart, "offered again");
    });
    // One that couldn't go says so.
    zone.refuse.set(Some(NotSent::Gone));
    zone.agent.update(&mut cx, |agent, cx| agent.continue_conversation(cx));
    zone.agent.read_with(&cx, |agent, _| assert_eq!(agent.model().refusal(), Some(UNSENT)));
}

/// `Respawn`: the server lost the session, so a new one continues the
/// conversation (`SPAWN_AGENT` naming it, in its directory); the zone
/// keeps its place and shows the old conversation until the new session's
/// events replace it, then sends through the new session.
#[test]
fn a_respawn_starts_a_new_session_continuing_the_conversation() {
    let mut cx = app();
    let mut zone = ended_zone(Continuation::Load, &mut cx);
    zone.sink.lost("the terminal server no longer has session 7");
    settle(&cx);
    zone.agent.read_with(&cx, |agent, cx| {
        assert!(matches!(agent.offer(cx), Offer::Respawn { .. }));
        assert_eq!(agent.server_session(), None);
    });
    zone.agent.update(&mut cx, |agent, cx| agent.continue_conversation(cx));
    let starts = zone.take_starts();
    let [Start::Spawn(request)] = starts.as_slice() else { panic!("one spawn: {starts:?}") };
    assert_eq!(
        request.conversation,
        Some(crate::remote::protocol::client::AgentConversation { id: "sess-1".into(), cwd: "/home/u/src".into() })
    );
    assert_eq!((request.cwd.as_deref(), request.command.as_str()), (Some("/home/u/src"), "fake-acp"));
    zone.agent.read_with(&cx, |agent, _| {
        assert_eq!(agent.model().phase(), &AgentPhase::Connecting);
        assert_eq!(agent.model().blocks().len(), 2, "the old conversation, until the new one comes");
        assert_eq!(agent.directory().map(|dir| dir.to_string_lossy().into_owned()).as_deref(), Some("/home/u/src"));
    });

    zone.feed_new(&[
        added(1, notice("this continues an earlier conversation; what follows is the agent's replay of it")),
        added(2, user("hi")),
        added(3, agent("Hello!")),
        conversation(Continuation::Load),
        ready(),
    ]);
    settle(&cx);
    zone.agent.read_with(&cx, |agent, _| {
        assert!(matches!(agent.model().phase(), AgentPhase::Live(AgentState::Ready { .. })));
        assert_eq!(agent.model().blocks().len(), 3, "the new session's log");
        assert_eq!(agent.server_session(), Some((8, None)), "the new session");
    });
    zone.agent.update(&mut cx, |agent, cx| agent.prompt("next", false, cx));
    assert_eq!(zone.take_calls(), [Call::Prompt("next".into())]);
}

/// `agents-session-list` 5.4: a stopped zone's New conversation closes its
/// session and starts a new one in its directory with no conversation;
/// a listed conversation chosen into it starts one continuing that; a
/// choosing zone's New conversation is `AGENT_CHOOSE` with none, and
/// Conversations… asks the window.
#[test]
fn a_stopped_zone_starts_anew_or_takes_a_listed_conversation() {
    let mut cx = app();
    let zone = ended_zone(Continuation::Resume, &mut cx);
    zone.agent.update(&mut cx, |agent, cx| agent.new_conversation(cx));
    assert_eq!(zone.take_calls(), [Call::Close]);
    let starts = zone.take_starts();
    let [Start::Spawn(request)] = starts.as_slice() else { panic!("one spawn: {starts:?}") };
    assert_eq!((request.conversation.as_ref(), request.start), (None, None));
    assert_eq!(request.cwd.as_deref(), Some("/home/u/src"), "the conversation's directory");
    zone.agent.read_with(&cx, |agent, _| {
        assert_eq!(agent.model().phase(), &AgentPhase::Connecting);
        assert_eq!(agent.model().conversation(), None, "the old conversation is forgotten");
    });

    let listed = ended_zone(Continuation::Resume, &mut cx);
    listed.agent.update(&mut cx, |agent, cx| agent.continue_listed("other", cx));
    assert_eq!(listed.take_calls(), [Call::Close]);
    let starts = listed.take_starts();
    let [Start::Spawn(request)] = starts.as_slice() else { panic!("one spawn: {starts:?}") };
    let conversation = request.conversation.as_ref().map(|c| (c.id.as_str(), c.cwd.as_str()));
    assert_eq!(conversation, Some(("other", "/home/u/src")));

    let mut choosing = Zone::new(None, &mut cx);
    choosing.feed(&[Event::Agent { state: AgentState::Choosing { message: None } }]);
    settle(&cx);
    let asked = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let _subscription = cx.update(|cx| {
        let asked = asked.clone();
        cx.subscribe(&choosing.agent, move |_, event: &AgentAsk, _| asked.borrow_mut().push(event.clone()))
    });
    choosing.agent.update(&mut cx, |agent, cx| agent.ask_conversations(cx));
    settle(&cx);
    assert_eq!(asked.borrow().as_slice(), [AgentAsk::Conversations]);
    choosing.agent.update(&mut cx, |agent, cx| agent.new_conversation(cx));
    assert_eq!(choosing.take_calls(), [Call::Choose(None)]);
    assert!(choosing.take_starts().is_empty(), "the same session");
}

/// No offer, nothing sent: an agent that can't continue, a profile that
/// is gone, a session started with another profile than the zone's.
#[test]
fn nothing_is_continued_without_an_offer() {
    let mut cx = app();
    let zone = ended_zone(Continuation::Neither, &mut cx);
    zone.agent.read_with(&cx, |agent, cx| assert_eq!(agent.offer(cx), Offer::None(NoOffer::NotOffered)));
    zone.agent.update(&mut cx, |agent, cx| agent.continue_conversation(cx));
    assert!(zone.take_calls().is_empty());

    let gone = ended_zone(Continuation::Resume, &mut cx);
    set_config("[[profiles]]\nname = \"Shell\"\n", &mut cx);
    gone.agent.read_with(&cx, |agent, cx| {
        assert_eq!(agent.offer(cx), Offer::None(NoOffer::ProfileGone { name: "Fake".into() }));
    });
    gone.agent.update(&mut cx, |agent, cx| agent.continue_conversation(cx));
    assert!(gone.take_calls().is_empty() && gone.take_starts().is_empty());

    let substitute = ended_zone(Continuation::Resume, &mut cx);
    substitute.agent.update(&mut cx, |agent, _| agent.set_started_with("Old"));
    substitute.agent.read_with(&cx, |agent, cx| {
        assert_eq!(agent.offer(cx), Offer::None(NoOffer::ProfileGone { name: "Old".into() }));
    });
    substitute.agent.update(&mut cx, |agent, cx| agent.continue_conversation(cx));
    assert!(substitute.take_calls().is_empty());

    // A renamed profile is still the zone's.
    let renamed = ended_zone(Continuation::Resume, &mut cx);
    set_config(&FAKE_PROFILE.replace("\"Fake\"", "\"Renamed\""), &mut cx);
    renamed.agent.update(&mut cx, |agent, cx| agent.set_profile_name("Renamed", cx));
    renamed.agent.read_with(&cx, |agent, cx| assert_eq!(agent.offer(cx), Offer::Restart));
}

/// A closed zone continues nothing.
#[test]
fn a_closed_zone_continues_nothing() {
    let mut cx = app();
    let zone = ended_zone(Continuation::Resume, &mut cx);
    zone.agent.update(&mut cx, |agent, cx| {
        agent.close();
        agent.continue_conversation(cx);
    });
    assert_eq!(zone.take_calls(), [Call::Close]);
}

// OpenSpec change `agents-login`, task 7.1.

fn methods() -> Vec<crate::agent::transcript::LoginMethod> {
    use crate::agent::transcript::{LoginKind, LoginMethod};
    vec![
        LoginMethod { id: "google".into(), name: "Log in with Google".into(), description: None, kind: LoginKind::Agent },
        LoginMethod {
            id: "cli".into(),
            name: "Log in from the terminal".into(),
            description: None,
            kind: LoginKind::Terminal { args: vec!["--login".into()], env: [("LOGIN_MODE".to_string(), "device".to_string())].into() },
        },
    ]
}

fn login_required() -> Event {
    Event::Agent { state: AgentState::LoginRequired { message: "Authentication required".into(), methods: methods() } }
}

/// An agent method's choice sends `AGENT_LOGIN` once; its refusal shows
/// beside the row; Cancel login while logging in sends `AGENT_CANCEL`.
#[test]
fn a_login_choice_sends_agent_login_and_cancel_login_cancels() {
    let mut cx = app();
    let mut zone = Zone::new(None, &mut cx);
    zone.feed(&[login_required()]);
    settle(&cx);
    zone.agent.update(&mut cx, |agent, cx| {
        agent.choose_login("google", cx);
        agent.choose_login("google", cx);
    });
    assert_eq!(zone.take_calls(), [Call::Login("google".into())], "once while it is on its way");
    zone.sink.reply(1, Err("session 7: a login is already running".into()));
    settle(&cx);
    zone.agent.read_with(&cx, |agent, _| assert_eq!(agent.model().login_note(), Some("session 7: a login is already running")));

    zone.agent.update(&mut cx, |agent, cx| agent.choose_login("google", cx));
    assert_eq!(zone.take_calls(), [Call::Login("google".into())]);
    zone.sink.reply(2, Ok(()));
    zone.feed(&[Event::Agent { state: AgentState::LoggingIn { method: "google".into(), name: "Log in with Google".into() } }]);
    settle(&cx);
    zone.agent.update(&mut cx, |agent, cx| agent.cancel_login(cx));
    assert_eq!(zone.take_calls(), [Call::Cancel]);
    zone.refuse.set(Some(NotSent::Gone));
    zone.agent.update(&mut cx, |agent, cx| agent.cancel_login(cx));
    zone.agent.read_with(&cx, |agent, _| assert_eq!(agent.model().refusal(), Some(UNSENT)));
}

/// A terminal method's choice asks the tree for a login zone; Cancel
/// login asks it to close; the zone's status 0 restarts the agent
/// (`AGENT_RESTART` with the profile's command), another status sends
/// nothing.
#[test]
fn a_terminal_login_opens_a_zone_and_its_success_restarts_the_agent() {
    let mut cx = app();
    set_config(FAKE_PROFILE, &mut cx);
    let mut zone = Zone::new(None, &mut cx);
    let mut login_zones = cx.events::<LoginZone, _>(&zone.agent);
    zone.feed(&[login_required()]);
    settle(&cx);
    zone.agent.update(&mut cx, |agent, cx| agent.choose_login("cli", cx));
    let opened = drained(&mut login_zones);
    let [LoginZone::Open(plan)] = opened.as_slice() else { panic!("a login zone: {opened:?}") };
    assert_eq!(plan.appended, ["--login"]);
    assert_eq!(plan.env.get("LOGIN_MODE").map(String::as_str), Some("device"));
    assert!(zone.take_calls().is_empty(), "nothing goes to the server");
    zone.agent.update(&mut cx, |agent, cx| agent.cancel_login(cx));
    assert!(matches!(drained(&mut login_zones).as_slice(), [LoginZone::Close]));
    zone.agent.update(&mut cx, |agent, cx| agent.login_zone_ended(model::LoginZoneEnd::Closed, cx));

    zone.agent.update(&mut cx, |agent, cx| agent.choose_login("cli", cx));
    drained(&mut login_zones);
    zone.agent.update(&mut cx, |agent, cx| agent.login_zone_ended(model::LoginZoneEnd::Exited { code: Some(2) }, cx));
    assert!(zone.take_calls().is_empty());
    zone.agent.update(&mut cx, |agent, cx| {
        agent.choose_login("cli", cx);
        agent.login_zone_ended(model::LoginZoneEnd::Exited { code: Some(0) }, cx);
    });
    let calls = zone.take_calls();
    let [Call::Restart(request)] = calls.as_slice() else { panic!("one restart: {calls:?}") };
    assert_eq!((request.command.as_str(), request.args.as_slice()), ("fake-acp", &["--acp".to_string()][..]));
    zone.agent.read_with(&cx, |agent, _| assert_eq!(agent.model().phase(), &AgentPhase::Live(AgentState::Starting)));
}

/// The first prompt shown while the agent needs a login goes once the
/// agent is ready, through the view's check that it is unchanged.
#[test]
fn a_first_prompt_after_a_login_is_sent_if_unchanged() {
    let mut cx = app();
    let mut zone = Zone::new(Some("hello there"), &mut cx);
    let mut updates = cx.events::<AgentUpdate, _>(&zone.agent);
    zone.feed(&[Event::Agent { state: AgentState::Starting }, login_required()]);
    settle(&cx);
    assert!(drained(&mut updates).contains(&AgentUpdate::SetDraft("hello there".into())));
    zone.feed(&[Event::Agent { state: AgentState::Starting }, ready()]);
    settle(&cx);
    assert!(drained(&mut updates).contains(&AgentUpdate::SendFirstIfUnchanged("hello there".into())));
    assert!(zone.take_calls().is_empty(), "the view decides");
    zone.agent.update(&mut cx, |agent, cx| agent.send_first("hello there", cx));
    assert_eq!(zone.take_calls(), [Call::Prompt("hello there".into())]);
}

/// D-1 (`agents-client-state-fixes` task 4.5): a launch's first prompt
/// whose agent is ready before the session's thread handed over the
/// session waits for it, refusing nothing, and goes once it runs, once.
#[test]
fn a_first_prompt_waits_for_the_session_to_run_and_goes_once() {
    let mut cx = app();
    let mut zone = Zone::starting(Some("hello there"), &mut cx);
    let mut updates = cx.events::<AgentUpdate, _>(&zone.agent);
    zone.feed(&[ready()]);
    settle(&cx);
    assert!(zone.take_calls().is_empty(), "not sent before the session runs");
    zone.agent.read_with(&cx, |agent, _| {
        assert_eq!(agent.model().refusal(), None);
        assert_eq!(agent.model().prompt_status(), model::PromptStatus::Disabled(model::DisabledWhy::Connecting));
    });

    zone.sink.running();
    settle(&cx);
    assert_eq!(zone.take_calls(), [Call::Prompt("hello there".into())]);
    zone.feed(&[added(1, user("hello there")), ready()]);
    settle(&cx);
    assert!(zone.take_calls().is_empty(), "sent once");
    let drafts = drained(&mut updates).into_iter().filter(|update| matches!(update, AgentUpdate::SetDraft(_)));
    assert_eq!(drafts.count(), 0, "never the draft");
    zone.agent.read_with(&cx, |agent, _| assert_eq!(agent.model().refusal(), None));
}

/// `agents-client-state-fixes` task 4.5: enter before the session runs
/// sends nothing and keeps the text, with no refusal; a send that meets
/// a session still being set up (`NotSent::Starting`) is the same.
#[test]
fn enter_before_the_session_runs_keeps_the_text_without_a_refusal() {
    let mut cx = app();
    let mut zone = Zone::starting(None, &mut cx);
    let mut updates = cx.events::<AgentUpdate, _>(&zone.agent);
    zone.feed(&[ready()]);
    settle(&cx);
    zone.agent.update(&mut cx, |agent, cx| agent.prompt("early", false, cx));
    assert!(zone.take_calls().is_empty(), "the model holds it");
    zone.agent.read_with(&cx, |agent, _| assert_eq!(agent.model().refusal(), None));

    zone.sink.running();
    settle(&cx);
    zone.refuse.set(Some(NotSent::Starting));
    zone.agent.update(&mut cx, |agent, cx| agent.prompt("raced", false, cx));
    assert_eq!(zone.take_calls(), [Call::Prompt("raced".into())]);
    settle(&cx);
    assert!(drained(&mut updates).is_empty(), "the input keeps its text");
    zone.agent.read_with(&cx, |agent, _| {
        assert_eq!(agent.model().refusal(), None);
        assert_eq!(agent.model().prompt_status(), model::PromptStatus::Ready, "nothing waits for an answer");
    });
}

/// D-11 (`agents-client-state-fixes` task 1.8): a conversation chosen
/// while the link is down isn't sent, and the choosing screen says why.
#[test]
fn a_choice_while_reconnecting_says_why() {
    let mut cx = app();
    let mut zone = Zone::new(None, &mut cx);
    zone.feed(&[Event::Agent { state: AgentState::Choosing { message: None } }]);
    zone.sink.link_down();
    settle(&cx);
    zone.agent.update(&mut cx, |agent, cx| agent.choose(Some("c1".into()), cx));
    assert!(!zone.take_calls().contains(&Call::Choose(Some("c1".into()))), "not sent");
    zone.agent.read_with(&cx, |agent, _| {
        let reason = agent.model().choosing_reason();
        assert!(reason.contains("connection to the terminal server"), "{reason}");
    });
}

/// What the entity emitted, in order, across the event types the test
/// watches.
fn emitted(zone: &Zone, cx: &mut gpui::TestAppContext) -> (std::rc::Rc<std::cell::RefCell<Vec<String>>>, [gpui::Subscription; 2]) {
    let log = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let subscriptions = cx.update(|cx| {
        let (statuses, updates) = (log.clone(), log.clone());
        [
            cx.subscribe(&zone.agent, move |_, event: &TerminalEvent, _| {
                if let TerminalEvent::Status(_) = event {
                    statuses.borrow_mut().push("status".to_string());
                }
            }),
            cx.subscribe(&zone.agent, move |_, event: &AgentUpdate, _| {
                updates.borrow_mut().push(match event {
                    AgentUpdate::Splices(_) => "splices".to_string(),
                    AgentUpdate::ClearDraft => "clear-draft".to_string(),
                    other => format!("{other:?}"),
                })
            }),
        ]
    });
    (log, subscriptions)
}

/// OpenSpec change `agents-absorb-cost` decision 4: a drained update's
/// statuses and draft changes come in event order, and its splices once,
/// after them. (The sink's gate folds every change it holds into one
/// pending `Changed`, so a chunk holds one; the `Reply` after it is the
/// update's second event.)
#[test]
fn an_update_emits_its_statuses_in_order_and_its_splices_once_after_them() {
    let mut cx = app();
    let mut zone = Zone::new(None, &mut cx);
    zone.feed(&[ready()]);
    settle(&cx);
    zone.agent.update(&mut cx, |agent, cx| agent.prompt("go", false, cx));
    assert_eq!(zone.take_calls(), [Call::Prompt("go".into())]);
    let (log, _subscriptions) = emitted(&zone, &mut cx);
    zone.feed(&[added(1, user("go")), Event::Turn { state: TurnState::Working }, added(2, agent("Hi"))]);
    zone.sink.reply(1, Ok(()));
    zone.feed(&[Event::Appended { id: EntryId(2), text: " there".into() }]);
    settle(&cx);
    assert_eq!(log.borrow().as_slice(), ["status", "clear-draft", "splices"]);
    zone.agent.read_with(&cx, |agent, _| assert_eq!(agent.model().message_text(EntryId(2)), Some("Hi there")));
}

/// Decision 2: `AgentShared`'s lock is free whenever the zone renders, so
/// the reader thread never waits on a render.
#[test]
fn the_lock_is_free_while_the_zone_renders() {
    let mut cx = app();
    let mut zone = Zone::new(None, &mut cx);
    let free = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let (shared, seen) = (zone.shared.clone(), free.clone());
    crate::agent::model::RENDER_PROBE.with(|probe| {
        *probe.borrow_mut() = Some(Box::new(move || seen.borrow_mut().push(shared.try_lock().is_some())));
    });
    zone.feed(&[ready(), added(1, user("go")), added(2, agent("**Hi**"))]);
    settle(&cx);
    zone.feed(&[Event::Appended { id: EntryId(2), text: " there".into() }]);
    settle(&cx);
    crate::agent::model::RENDER_PROBE.with(|probe| *probe.borrow_mut() = None);
    let free = free.borrow();
    assert!(free.len() >= 2, "the zone rendered: {free:?}");
    assert!(free.iter().all(|free| *free), "the lock was held across a render: {free:?}");
}

/// Decision 5's frame gate: a message growing in updates less than a
/// frame apart is rendered once per frame, its last text drawn when the
/// frame ends; one that grows after a quiet frame is drawn at once.
#[test]
fn a_growing_message_is_drawn_once_per_frame() {
    let mut cx = app();
    let mut zone = Zone::new(None, &mut cx);
    zone.feed(&[ready(), added(1, user("go")), added(2, agent("a"))]);
    settle(&cx);
    let entity = zone.agent.clone();
    let renders = |cx: &gpui::TestAppContext| entity.read_with(cx, |agent, _| agent.model().render_stats().renders);
    let blocks = |cx: &gpui::TestAppContext| entity.read_with(cx, |agent, _| agent.model().blocks().to_vec());
    let executor = cx.executor();

    // A frame after the last draw: drawn at once.
    let before = renders(&cx);
    zone.feed(&[Event::Appended { id: EntryId(2), text: " b".into() }]);
    executor.run_until_parked();
    assert_eq!(renders(&cx), before + 1, "after a quiet frame");

    // Within the frame: left for its end.
    let drawn = blocks(&cx);
    for text in [" c", " d"] {
        zone.feed(&[Event::Appended { id: EntryId(2), text: text.into() }]);
        executor.run_until_parked();
    }
    assert_eq!(renders(&cx), before + 1, "within the frame");
    assert_eq!(blocks(&cx), drawn, "drawn as it was");
    executor.advance_clock(crate::terminal::pace::FRAME);
    executor.run_until_parked();
    assert_eq!(renders(&cx), before + 2, "once, at the frame's end");
    zone.agent.read_with(&cx, |agent, _| {
        let model = agent.model();
        assert_ne!(model.blocks(), drawn.as_slice());
        assert_eq!(model.message_text(EntryId(2)), Some("a b c d"));
    });
}
