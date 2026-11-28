//! What the agent zone's `TestAppContext` tests share: an app set up as
//! Brindle sets itself up, fake requests that record what the zone sends,
//! and an agent fed through a real sink.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use gpui::{AppContext as _, Entity, TestAppContext};
use parking_lot::Mutex;

use crate::FirstPrompt;
use crate::agent::model::{ChatModel, NotSent};
use crate::agent::shared::AgentShared;
use crate::agent::sink::Sink;
use crate::agent::spawn::{AgentRestartRequest, AgentSpawnRequest};
use crate::agent::transcript::{AgentState, Entry, EntryId, Event};
use crate::agent::{Agent, Connect, Requests};
use crate::config::{Config, ConfigReport};
use crate::remote::{AgentStart, Host};
use crate::remote::SessionKey;
use crate::remote::protocol::client::OptionValue;
use crate::remote::protocol::{RequestId, SessionId};
use crate::settings::Settings;

/// A test app with Brindle's bindings, the `brindle-ui` setup, the light
/// tokens and the default settings.
pub fn app() -> TestAppContext {
    let cx = TestAppContext::single();
    cx.update(|cx| {
        brindle_ui::init(cx);
        cx.bind_keys(crate::actions::default_bindings());
        brindle_ui::UiTheme::set(brindle_ui::Tokens::neutral_light(), cx);
        cx.set_global(Settings::new(Config::default(), ConfigReport::default(), Vec::new()));
    });
    cx
}

/// Makes the app's config `text`.
pub fn set_config(text: &str, cx: &mut TestAppContext) {
    let config = Config::parse(text).unwrap().config;
    cx.update(|cx| cx.set_global(Settings::new(config, ConfigReport::default(), Vec::new())));
}

/// A config whose agent profile `Fake` runs `fake-acp --acp`.
pub const FAKE_PROFILE: &str = "[[profiles]]\nname = \"Shell\"\n\n[[profiles]]\nname = \"Fake\"\ntype = \"agent\"\ncommand = \"fake-acp\"\nargs = [\"--acp\"]\n";

/// What the zone sent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Call {
    Prompt(String),
    Resend,
    Unqueue,
    Answer(u64, Option<String>),
    Cancel,
    Close,
    Detach(Option<serde_json::Value>),
    Layout(Option<serde_json::Value>),
    Restart(AgentRestartRequest),
    Login(String),
    Fetch(u64, u32),
    List(Option<String>),
    Choose(Option<String>),
    SetOption(String, OptionValue),
}

pub type Calls = Rc<RefCell<Vec<Call>>>;

/// Requests that record each call and answer with the next id, or, while
/// `refuse` holds a reason, with that.
pub struct FakeRequests {
    calls: Calls,
    next: Rc<Cell<RequestId>>,
    pub refuse: Rc<Cell<Option<NotSent>>>,
    /// The session's id.
    session: SessionId,
}

impl FakeRequests {
    fn record(&self, call: Call) -> Result<RequestId, NotSent> {
        self.calls.borrow_mut().push(call);
        if let Some(why) = self.refuse.get() {
            return Err(why);
        }
        let id = self.next.get();
        self.next.set(id + 1);
        Ok(id)
    }
}

impl Requests for FakeRequests {
    fn prompt(&self, text: &str) -> Result<RequestId, NotSent> {
        self.record(Call::Prompt(text.to_string()))
    }

    fn resend(&self) -> Result<RequestId, NotSent> {
        self.record(Call::Resend)
    }

    fn unqueue(&self) -> Result<RequestId, NotSent> {
        self.record(Call::Unqueue)
    }

    fn answer(&self, entry: u64, option: Option<String>) -> Result<RequestId, NotSent> {
        self.record(Call::Answer(entry, option))
    }

    fn cancel(&self) -> Result<(), NotSent> {
        self.record(Call::Cancel).map(|_| ())
    }

    fn close(&self) {
        self.calls.borrow_mut().push(Call::Close);
    }

    fn detach_with(&self, layout: Option<serde_json::Value>) {
        self.calls.borrow_mut().push(Call::Detach(layout));
    }

    fn set_layout(&self, layout: Option<serde_json::Value>) {
        self.calls.borrow_mut().push(Call::Layout(layout));
    }

    fn id_and_key(&self) -> Option<(SessionId, Option<SessionKey>)> {
        Some((self.session, None))
    }

    fn restart(&self, request: AgentRestartRequest) -> Result<RequestId, NotSent> {
        self.record(Call::Restart(request))
    }

    fn login(&self, method: &str) -> Result<RequestId, NotSent> {
        self.record(Call::Login(method.to_string()))
    }

    fn fetch(&self, before: u64, max_bytes: u32) -> Result<RequestId, NotSent> {
        self.record(Call::Fetch(before, max_bytes))
    }

    fn list(&self, cursor: Option<String>) -> Result<RequestId, NotSent> {
        self.record(Call::List(cursor))
    }

    fn choose(&self, conversation: Option<String>) -> Result<RequestId, NotSent> {
        self.record(Call::Choose(conversation))
    }

    fn set_option(&self, option: &str, value: OptionValue) -> Result<RequestId, NotSent> {
        self.record(Call::SetOption(option.to_string(), value))
    }
}

/// A [`Connect`] that records each start and gives fake requests for a
/// new session ([`FakeRequests`], session 8, sharing the zone's calls and
/// its `refuse`). The fake session runs at once: its sink is told so
/// before it is kept.
fn fake_connect(calls: Calls, refuse: Rc<Cell<Option<NotSent>>>, starts: Starts, sinks: Sinks) -> Connect {
    Box::new(move |_host, start, mut sink| {
        let start = match start {
            AgentStart::Spawn(request) => Start::Spawn(request()),
            AgentStart::Attach { session, .. } => Start::Attach(session),
        };
        starts.borrow_mut().push(start);
        sink.running();
        sinks.borrow_mut().push(sink);
        let requests =
            FakeRequests { calls: calls.clone(), next: Rc::new(Cell::new(101)), refuse: refuse.clone(), session: 8 };
        Ok(Box::new(requests) as Box<dyn Requests>)
    })
}

/// What the fake connect was asked to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Start {
    Spawn(AgentSpawnRequest),
    Attach(SessionId),
}

pub type Starts = Rc<RefCell<Vec<Start>>>;
pub type Sinks = Rc<RefCell<Vec<Sink>>>;

/// An agent zone's entity over a real sink, with fake requests.
pub struct Zone {
    pub sink: Sink,
    pub agent: Entity<Agent>,
    pub calls: Calls,
    /// What the fake requests answer instead of an id, while set.
    pub refuse: Rc<Cell<Option<NotSent>>>,
    pub shared: Arc<Mutex<AgentShared>>,
    /// What the zone's connect started (a continue in a new session), and
    /// the new sessions' sinks.
    pub starts: Starts,
    pub sinks: Sinks,
    /// The next id the fake requests answer with.
    next: Rc<Cell<RequestId>>,
}

impl Zone {
    /// A zone whose session runs: its sink was told, as the session's
    /// thread tells it once the session is set up.
    pub fn new(first: Option<&str>, cx: &mut TestAppContext) -> Self {
        let mut zone = Self::starting(first, cx);
        zone.sink.running();
        zone
    }

    /// A zone whose session's thread hasn't handed it the running session
    /// yet: nothing told its sink `running`.
    pub fn starting(first: Option<&str>, cx: &mut TestAppContext) -> Self {
        Self::build(first, true, cx)
    }

    /// A zone whose start failed: it has no requests to send through, as
    /// when the server is too old.
    pub fn no_server(cx: &mut TestAppContext) -> Self {
        Self::build(None, false, cx)
    }

    fn build(first: Option<&str>, server: bool, cx: &mut TestAppContext) -> Self {
        let (sink, feed) = Sink::with_fault(None);
        let shared = feed.shared.clone();
        let calls = Calls::default();
        let refuse = Rc::new(Cell::new(None));
        let next = Rc::new(Cell::new(1));
        let requests = FakeRequests { calls: calls.clone(), next: next.clone(), refuse: refuse.clone(), session: 7 };
        let (starts, sinks) = (Starts::default(), Sinks::default());
        let connect = fake_connect(calls.clone(), refuse.clone(), starts.clone(), sinks.clone());
        let first = first.map(|text| FirstPrompt::new(text.to_string()).unwrap());
        let agent = cx.new(|cx| {
            let model = ChatModel::new("Fake", first);
            let requests = server.then(|| Box::new(requests) as Box<dyn Requests>);
            Agent::from_parts(model, feed, requests, "fake-acp".into(), Host::Local, None, connect, cx)
        });
        Self { sink, agent, calls, refuse, shared, starts, sinks, next }
    }

    /// The id the zone's fake requests answered their last request with.
    pub fn last_request(&self) -> RequestId {
        self.next.get() - 1
    }

    /// Folds `events` on this thread, as the reader thread does.
    pub fn feed(&mut self, events: &[Event]) {
        let values = events.iter().map(Event::to_value).collect();
        self.sink.transcript(values).unwrap();
    }

    /// Folds `events` into the newest session the zone's connect started.
    pub fn feed_new(&mut self, events: &[Event]) {
        let values = events.iter().map(Event::to_value).collect();
        self.sinks.borrow_mut().last_mut().expect("a new session").transcript(values).unwrap();
    }

    pub fn take_starts(&self) -> Vec<Start> {
        std::mem::take(&mut *self.starts.borrow_mut())
    }

    pub fn take_calls(&self) -> Vec<Call> {
        std::mem::take(&mut *self.calls.borrow_mut())
    }
}

/// Lets the agent's loop take everything queued, past its pacing.
pub fn settle(cx: &TestAppContext) {
    let executor = cx.executor();
    executor.run_until_parked();
    executor.advance_clock(Duration::from_millis(20));
    executor.run_until_parked();
}

pub fn ready() -> Event {
    Event::Agent { state: AgentState::Ready { name: Some("Fake Agent".into()), version: None } }
}

pub fn added(id: u64, entry: Entry) -> Event {
    Event::Added { id: EntryId(id), entry }
}

pub fn user(text: &str) -> Entry {
    Entry::User { text: text.into() }
}

pub fn agent(text: &str) -> Entry {
    Entry::Agent { text: text.into() }
}
