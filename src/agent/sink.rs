//! Where an agent session's transcript lands on the connection's reader
//! thread (design.md decision 7): [`Sink`] folds each `TRANSCRIPT` frame
//! into the [`AgentShared`] it shares with its zone, and tells the zone
//! through its [`Feed`]'s channel.
//!
//! What it sends passes an [`EventGate`], as a terminal's events do
//! (`process R9`): a fold sends one merged [`AgentEvent::Changed`], of which
//! at most one waits, since the zone takes every change from the shared
//! fold at once; the life events (a replay's start and end, the link going
//! down, the session's thread handing over the running session, the exit,
//! the loss, the failure to start, the break, a request's answer, a note)
//! are always queued, and are bounded by the session's one lifetime and the
//! requests its zone sent.
//!
//! The connection calls it through [`crate::remote::local`]'s
//! `SessionSink`, under `Connection::deliver`'s `catch_unwind`: an event it
//! can't decode or fold, or a panic, breaks the session (it is detached,
//! still running), as an emulator failure breaks a terminal's.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use parking_lot::Mutex;
use serde_json::Value;

use crate::agent::conversations::Page;
use crate::agent::model::{ReplyError, StartFailure};
use crate::remote::protocol::client::AgentConversations;
use crate::agent::shared::AgentShared;
use crate::remote::protocol::RequestId;
use crate::terminal::events::{Admit, EventGate, Kind, Merge, Recovered};

/// What an agent session's sink tells its zone, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentEvent {
    /// The shared fold changed: the zone takes its changes
    /// ([`AgentShared::take_changes`]). Merged: at most one waits.
    Changed,
    /// A replay starts (`true`: the fold starts over) or ends. One that
    /// ends after the link went down means it is back up.
    Replay(bool),
    /// The link to the endpoint's core went down; a replay (or the
    /// session's loss) follows.
    LinkDown,
    /// The session ended on its server (`EXIT`).
    Exited(Option<i32>),
    /// The session can't be got back, saying why.
    Lost(String),
    /// The session never started.
    Failed(StartFailure),
    /// Brindle can't follow the session any more, saying why: it was
    /// detached, and nothing more comes.
    Broken(String),
    /// The session's thread handed the zone the running session: requests
    /// can go now (OpenSpec change `agents-client-state-fixes`, decision 2).
    /// Every event the reader delivered before it is queued before it.
    SessionRunning,
    /// The core answered the zone's request `request`, or it failed:
    /// refused or never sent ([`ReplyError::Failed`]), or lost with the
    /// link after it was written ([`ReplyError::Unanswered`]).
    Reply { request: RequestId, outcome: Result<(), ReplyError> },
    /// A page of the agent's conversations answering the list `request`
    /// (`AGENT_CONVERSATIONS`, OpenSpec change `agents-session-list`), in
    /// place of its reply.
    Conversations { request: RequestId, page: Page },
    /// The core's word about the session that answers no request (a
    /// refused cancel, a holder's fault report).
    Note(String),
    /// The agent got the endpoint's SSH environment because the login
    /// shell's wasn't read: an endpoint note above the conversation.
    EndpointNote(String),
}

/// What `event` is to the gate: [`AgentEvent::Changed`] merged, every
/// other a life event, always queued.
pub fn kind_of(event: &AgentEvent) -> Kind {
    match event {
        AgentEvent::Changed => Kind::Merged(Merge::Wakeup),
        AgentEvent::Replay(_)
        | AgentEvent::SessionRunning
        | AgentEvent::LinkDown
        | AgentEvent::Exited(_)
        | AgentEvent::Lost(_)
        | AgentEvent::Failed(_)
        | AgentEvent::Broken(_)
        | AgentEvent::Reply { .. }
        | AgentEvent::Conversations { .. }
        | AgentEvent::Note(_)
        | AgentEvent::EndpointNote(_) => Kind::Life,
    }
}

/// The zone's end: the shared fold, the gate and the events.
pub struct Feed {
    pub shared: Arc<Mutex<AgentShared>>,
    pub events: UnboundedReceiver<AgentEvent>,
    gate: Arc<Mutex<EventGate>>,
}

impl Feed {
    /// The zone took `taken` off [`Feed::events`]: they no longer wait. A
    /// merged `Changed` can be queued again.
    #[cfg(test)]
    pub fn took<'a>(&self, taken: impl IntoIterator<Item = &'a AgentEvent>) -> Option<Recovered> {
        self.gate.lock().took(taken.into_iter().map(kind_of))
    }

    /// The shared fold, the gate's end and the events apart, so the zone's
    /// loop can own the receiver while it tells the gate what it took.
    pub fn split(self) -> (Arc<Mutex<AgentShared>>, Taker, UnboundedReceiver<AgentEvent>) {
        (self.shared, Taker { gate: self.gate }, self.events)
    }
}

/// The gate's end of a [`Feed`] split from its events ([`Feed::split`]).
pub struct Taker {
    gate: Arc<Mutex<EventGate>>,
}

impl Taker {
    /// As [`Feed::took`].
    pub fn took<'a>(&self, taken: impl IntoIterator<Item = &'a AgentEvent>) -> Option<Recovered> {
        self.gate.lock().took(taken.into_iter().map(kind_of))
    }
}

/// An agent session's sink, which the connection's reader thread calls.
pub struct Sink {
    shared: Arc<Mutex<AgentShared>>,
    /// A leaf lock, as a terminal's gate: nothing is taken under it.
    gate: Arc<Mutex<EventGate>>,
    tx: UnboundedSender<AgentEvent>,
    /// The debug-only fault switch's marker (`BRINDLE_TEST_SINK_PANIC`,
    /// `process R20`): an event whose JSON holds it panics the fold.
    fault: Option<Vec<u8>>,
    /// A fetch's `AGENT_PAGE` frames by request, until its last frame
    /// (OpenSpec change `agents-log-paging`).
    pages: BTreeMap<RequestId, Vec<Value>>,
    /// Fetches whose frames a replay's start cut off: their remaining
    /// frames are dropped, since the fold they were for is gone.
    dropping: BTreeSet<RequestId>,
}

impl Sink {
    /// A sink and the zone's end of it, with the fault switch as the
    /// environment sets it (only in a debug build). The `Agent` entity
    /// makes its sink here; the tests give the marker instead.
    pub fn new() -> (Self, Feed) {
        Self::with_fault(fault_marker())
    }

    /// A sink whose fold panics on an event holding `fault`, as the fault
    /// switch makes it.
    pub fn with_fault(fault: Option<Vec<u8>>) -> (Self, Feed) {
        let shared = Arc::new(Mutex::new(AgentShared::new()));
        let gate = Arc::new(Mutex::new(EventGate::default()));
        let (tx, events) = unbounded();
        let sink = Self {
            shared: shared.clone(),
            gate: gate.clone(),
            tx,
            fault,
            pages: BTreeMap::new(),
            dropping: BTreeSet::new(),
        };
        (sink, Feed { shared, events, gate })
    }

    /// Queues `event` as the gate says. Under the gate's lock, so the
    /// channel's order is the gate's.
    fn send(&self, event: AgentEvent) {
        let mut gate = self.gate.lock();
        match gate.admit(kind_of(&event)) {
            Admit::Queue => {
                self.tx.unbounded_send(event).ok();
            }
            Admit::Merged | Admit::Dropped | Admit::Ignored => {}
        }
    }

    /// Folds one `TRANSCRIPT` frame's events. `Err` says why one couldn't
    /// be decoded or folded; those before it stay folded, and the zone is
    /// told of them.
    pub fn transcript(&mut self, events: Vec<Value>) -> Result<(), String> {
        let folded = {
            let mut shared = self.shared.lock();
            self.inject_fault(&events);
            shared.apply(&events)
        };
        self.send(AgentEvent::Changed);
        folded.map_err(|broken| broken.to_string())
    }

    /// Panics, with the fold's lock held as a real failure would, when the
    /// fault switch is on and an event holds its marker.
    fn inject_fault(&self, events: &[Value]) {
        let Some(marker) = self.fault.as_deref().filter(|marker| !marker.is_empty()) else { return };
        for event in events {
            let text = event.to_string();
            if text.as_bytes().windows(marker.len()).any(|window| window == marker) {
                panic!("{}: a transcript event held its marker", crate::terminal::SINK_PANIC);
            }
        }
    }

    /// One `AGENT_PAGE` frame of fetch `request`: kept until its last
    /// (`last` is `Some(more)`), then the whole page is folded in front
    /// ([`AgentShared::prepend`]) and the zone told. `Err` says why it
    /// couldn't be folded. Frames of a fetch a replay cut off are dropped.
    pub fn page(&mut self, request: RequestId, events: Vec<Value>, last: Option<bool>) -> Result<(), String> {
        if self.dropping.contains(&request) {
            if last.is_some() {
                self.dropping.remove(&request);
            }
            return Ok(());
        }
        self.pages.entry(request).or_default().extend(events);
        let Some(more) = last else { return Ok(()) };
        let page = self.pages.remove(&request).unwrap_or_default();
        let folded = {
            let mut shared = self.shared.lock();
            self.inject_fault(&page);
            shared.prepend(&page, more)
        };
        self.send(AgentEvent::Changed);
        folded.map_err(|broken| broken.to_string())
    }

    /// A replay starts (the fold starts over) or ends.
    pub fn replay(&mut self, active: bool) {
        if active {
            let cut = std::mem::take(&mut self.pages);
            self.dropping.extend(cut.into_keys());
        }
        {
            let mut shared = self.shared.lock();
            if active {
                shared.replay_start();
            } else {
                shared.replay_end();
            }
        }
        self.send(AgentEvent::Replay(active));
        if active {
            self.send(AgentEvent::Changed);
        }
    }

    /// The core's answer to request `request`. A fetch that failed (with
    /// the link) leaves no frames behind.
    pub fn reply(&mut self, request: RequestId, outcome: Result<(), ReplyError>) {
        self.pages.remove(&request);
        self.dropping.remove(&request);
        self.send(AgentEvent::Reply { request, outcome });
    }

    /// The session's thread handed over the running session.
    pub fn running(&mut self) {
        self.send(AgentEvent::SessionRunning);
    }

    /// `AGENT_CONVERSATIONS` answering the list `request`: the page goes to
    /// the zone in place of the request's reply.
    pub fn conversations(&mut self, request: RequestId, listed: AgentConversations) {
        self.send(AgentEvent::Conversations { request, page: Page::of(listed) });
    }

    pub fn link_down(&mut self) {
        self.send(AgentEvent::LinkDown);
    }

    pub fn exited(&mut self, code: Option<i32>) {
        self.send(AgentEvent::Exited(code));
    }

    pub fn lost(&mut self, reason: &str) {
        self.send(AgentEvent::Lost(reason.to_string()));
    }

    /// The session never started: the gate's or the client's
    /// [`StartFailure`] when the error is one, else no server.
    pub fn failed(&mut self, error: &anyhow::Error) {
        let failure = error
            .downcast_ref::<StartFailure>()
            .cloned()
            .unwrap_or_else(|| StartFailure::NoServer(format!("{error:#}")));
        log::warn!("an agent session didn't start: {failure}");
        self.send(AgentEvent::Failed(failure));
    }

    pub fn broken(&mut self, reason: &str) {
        self.send(AgentEvent::Broken(reason.to_string()));
    }

    pub fn note(&mut self, text: &str) {
        self.send(AgentEvent::Note(text.to_string()));
    }

    pub fn endpoint_note(&mut self, text: &str) {
        self.send(AgentEvent::EndpointNote(text.to_string()));
    }
}

/// The fault switch's marker, read when a sink is made: only a debug build
/// has the switch.
#[cfg(debug_assertions)]
fn fault_marker() -> Option<Vec<u8>> {
    std::env::var_os(crate::terminal::SINK_PANIC).filter(|value| !value.is_empty()).map(|value| value.into_encoded_bytes())
}

/// A release build has no fault switch.
#[cfg(not(debug_assertions))]
fn fault_marker() -> Option<Vec<u8>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::transcript::{Entry, EntryId, Event};

    fn taken(feed: &mut Feed) -> Vec<AgentEvent> {
        let events: Vec<AgentEvent> = std::iter::from_fn(|| feed.events.try_next().ok().flatten()).collect();
        feed.took(&events);
        events
    }

    fn added(id: u64, text: &str) -> Value {
        Event::Added { id: EntryId(id), entry: Entry::Agent { text: text.into() } }.to_value()
    }

    /// 4.4: a burst of folds leaves one `Changed` queued, whose changes
    /// hold the whole burst; once it is taken, the next fold queues one
    /// again. Life events are queued each time, in order.
    #[test]
    fn a_burst_leaves_one_changed_queued() {
        let (mut sink, mut feed) = Sink::with_fault(None);
        for id in 1..=50 {
            sink.transcript(vec![added(id, "chunk")]).unwrap();
        }
        sink.reply(7, Ok(()));
        sink.reply(8, Err("a prompt is already queued".into()));
        assert_eq!(
            taken(&mut feed),
            [
                AgentEvent::Changed,
                AgentEvent::Reply { request: 7, outcome: Ok(()) },
                AgentEvent::Reply { request: 8, outcome: Err("a prompt is already queued".into()) },
            ]
        );
        let changes = feed.shared.lock().take_changes();
        assert_eq!(changes.added_from, Some(EntryId(1)));
        assert_eq!(feed.shared.lock().transcript().entries().count(), 50);

        sink.transcript(vec![added(51, "more")]).unwrap();
        sink.transcript(vec![added(52, "more")]).unwrap();
        assert_eq!(taken(&mut feed), [AgentEvent::Changed]);
    }

    /// A replay starts the fold over and is told around its events; the
    /// life events of a session's end come as they happen.
    #[test]
    fn a_replay_and_the_life_events_reach_the_zone_in_order() {
        let (mut sink, mut feed) = Sink::with_fault(None);
        sink.transcript(vec![added(1, "before")]).unwrap();
        taken(&mut feed);
        feed.shared.lock().take_changes();
        sink.replay(true);
        sink.transcript(vec![added(1, "replayed")]).unwrap();
        sink.replay(false);
        sink.link_down();
        sink.exited(Some(3));
        sink.lost("gone");
        sink.broken("why");
        sink.note("the terminal server: no turn is running");
        assert_eq!(
            taken(&mut feed),
            [
                AgentEvent::Replay(true),
                AgentEvent::Changed,
                AgentEvent::Replay(false),
                AgentEvent::LinkDown,
                AgentEvent::Exited(Some(3)),
                AgentEvent::Lost("gone".into()),
                AgentEvent::Broken("why".into()),
                AgentEvent::Note("the terminal server: no turn is running".into()),
            ]
        );
        let changes = feed.shared.lock().take_changes();
        assert!(changes.reset && changes.status_from == crate::agent::shared::StatusFrom::Replay, "{changes:?}");
        let texts: Vec<String> = feed
            .shared
            .lock()
            .transcript()
            .entries()
            .map(|(_, entry)| format!("{entry:?}"))
            .collect();
        assert_eq!(texts.len(), 1);
        assert!(texts[0].contains("replayed"), "{texts:?}");
    }

    /// `agents-client-state-fixes` 4.4: the session's thread's
    /// `SessionRunning` is a life event, never merged with a pending
    /// `Changed` or dropped, queued after what was delivered before it; a
    /// reply keeps whether it was refused or lost with the link.
    #[test]
    fn the_running_event_is_a_life_event_queued_in_order() {
        assert_eq!(kind_of(&AgentEvent::SessionRunning), Kind::Life);
        let (mut sink, mut feed) = Sink::with_fault(None);
        sink.transcript(vec![added(1, "ready")]).unwrap();
        sink.running();
        sink.transcript(vec![added(2, "after")]).unwrap();
        sink.reply(3, Err(ReplyError::Unanswered("the terminal server went away".into())));
        sink.reply(4, Err(ReplyError::Failed("refused".into())));
        assert_eq!(
            taken(&mut feed),
            [
                AgentEvent::Changed,
                AgentEvent::SessionRunning,
                AgentEvent::Reply {
                    request: 3,
                    outcome: Err(ReplyError::Unanswered("the terminal server went away".into()))
                },
                AgentEvent::Reply { request: 4, outcome: Err(ReplyError::Failed("refused".into())) },
            ],
            "the second fold merged into the waiting `Changed`"
        );
    }

    /// `agents-session-list` 4.1: a list's answer reaches the zone as its
    /// page, in the server's order, with its cursor.
    #[test]
    fn a_lists_answer_reaches_the_zone_as_a_page() {
        use crate::agent::conversations::{Listed, Page};
        use crate::remote::protocol::client::{AgentConversations, ListedConversation};
        let (mut sink, mut feed) = Sink::with_fault(None);
        let listed = AgentConversations {
            req: 4,
            session: 7,
            conversations: vec![
                ListedConversation { id: "b".into(), title: Some("Fix".into()), updated: None },
                ListedConversation { id: "a".into(), title: None, updated: Some("2026-11-01T00:00:00Z".into()) },
            ],
            next: Some("c2".into()),
        };
        sink.conversations(4, listed);
        let page = Page {
            conversations: vec![
                Listed { id: "b".into(), title: Some("Fix".into()), updated: None },
                Listed { id: "a".into(), title: None, updated: Some("2026-11-01T00:00:00Z".into()) },
            ],
            next: Some("c2".into()),
        };
        assert_eq!(taken(&mut feed), [AgentEvent::Conversations { request: 4, page }]);
    }

    /// An event that doesn't decode is refused with why; the ones before it
    /// stay folded, and the zone is told of them.
    #[test]
    fn an_event_that_does_not_decode_is_refused_with_why() {
        let (mut sink, mut feed) = Sink::with_fault(None);
        let refused = sink.transcript(vec![added(1, "kept"), serde_json::json!({"kind": "no such kind"})]);
        let why = refused.unwrap_err();
        assert!(why.starts_with("an event from the terminal server couldn't be read"), "{why}");
        assert_eq!(taken(&mut feed), [AgentEvent::Changed]);
        assert_eq!(feed.shared.lock().transcript().entries().count(), 1);
    }

    /// A failure to start reaches the zone as the client's typed failure,
    /// else as no server.
    #[test]
    fn a_failure_to_start_keeps_its_kind() {
        let (mut sink, mut feed) = Sink::with_fault(None);
        sink.failed(&anyhow::Error::new(StartFailure::TooOld { endpoint: "A8".into() }));
        sink.failed(&anyhow::anyhow!("terminal server helper not found"));
        assert_eq!(
            taken(&mut feed),
            [
                AgentEvent::Failed(StartFailure::TooOld { endpoint: "A8".into() }),
                AgentEvent::Failed(StartFailure::NoServer("terminal server helper not found".into())),
            ]
        );
    }

    /// The fault switch panics the fold on an event holding its marker, and
    /// on nothing else.
    #[test]
    fn the_fault_switch_panics_on_its_marker_only() {
        let (mut sink, _feed) = Sink::with_fault(Some(b"BRINDLE-FAULT".to_vec()));
        sink.transcript(vec![added(1, "fine")]).unwrap();
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            sink.transcript(vec![added(2, "says BRINDLE-FAULT here")])
        }));
        assert!(panicked.is_err());
    }

    // agents-log-paging 4.2: a fetch's pages.

    fn window(from: u64, earlier: u64) -> Value {
        Event::Window { from: EntryId(from), earlier }.to_value()
    }

    /// A sink whose fold holds entries 3 and 4 behind a window over 2.
    fn paged() -> (Sink, Feed) {
        let (mut sink, mut feed) = Sink::with_fault(None);
        sink.replay(true);
        sink.transcript(vec![window(3, 2), added(3, "three"), added(4, "four")]).unwrap();
        sink.replay(false);
        taken(&mut feed);
        feed.shared.lock().take_changes();
        (sink, feed)
    }

    fn texts(feed: &Feed) -> Vec<String> {
        feed.shared
            .lock()
            .transcript()
            .entries()
            .map(|(id, entry)| match entry {
                Entry::Agent { text } => format!("{}:{text}", id.0),
                other => format!("{}:{other:?}", id.0),
            })
            .collect()
    }

    #[test]
    fn a_two_frame_page_is_folded_once_at_its_last_frame() {
        let (mut sink, mut feed) = paged();
        sink.page(9, vec![added(1, "one")], None).unwrap();
        assert_eq!(taken(&mut feed), [], "nothing is folded before the last frame");
        assert_eq!(texts(&feed), ["3:three", "4:four"]);
        sink.page(9, vec![added(2, "two")], Some(false)).unwrap();
        assert_eq!(taken(&mut feed), [AgentEvent::Changed]);
        assert_eq!(texts(&feed), ["1:one", "2:two", "3:three", "4:four"]);
        let changes = feed.shared.lock().take_changes();
        assert_eq!((changes.prepended, changes.status_from), (Some(2), crate::agent::shared::StatusFrom::Unchanged));
        assert!(feed.shared.lock().transcript().window().is_none());
    }

    #[test]
    fn a_page_with_a_gap_breaks_the_session_and_changes_nothing() {
        let (mut sink, feed) = paged();
        let refused = sink.page(9, vec![added(1, "one")], Some(false)).unwrap_err();
        assert!(refused.contains("a page from the terminal server couldn't be applied"), "{refused}");
        assert_eq!(texts(&feed), ["3:three", "4:four"]);
    }

    #[test]
    fn an_empty_page_after_a_trim_took_the_window_folds_as_nothing() {
        let (mut sink, mut feed) = paged();
        // A trim through entry 2 takes the whole window.
        let trimmed = Event::Trimmed { through: EntryId(2), dropped: 2 }.to_value();
        sink.transcript(vec![trimmed]).unwrap();
        assert!(feed.shared.lock().transcript().window().is_none());
        taken(&mut feed);
        feed.shared.lock().take_changes();
        sink.page(9, Vec::new(), Some(false)).unwrap();
        assert_eq!(texts(&feed), ["3:three", "4:four"]);
        assert_eq!(feed.shared.lock().take_changes().prepended, Some(0));
    }

    #[test]
    fn a_page_cut_off_by_a_replay_is_dropped() {
        let (mut sink, mut feed) = paged();
        sink.page(9, vec![added(1, "one")], None).unwrap();
        // A reconnect replays the newest page again.
        sink.replay(true);
        sink.transcript(vec![window(3, 2), added(3, "three"), added(4, "four")]).unwrap();
        sink.replay(false);
        taken(&mut feed);
        sink.page(9, vec![added(2, "two")], Some(false)).unwrap();
        assert_eq!(texts(&feed), ["3:three", "4:four"], "the cut page is dropped");
        // A later fetch folds as usual.
        sink.page(10, vec![added(1, "one"), added(2, "two")], Some(false)).unwrap();
        assert_eq!(texts(&feed), ["1:one", "2:two", "3:three", "4:four"]);
    }
}
