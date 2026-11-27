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
use crate::agent::model::ChatModel;
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

/// Requests that record each call and answer with the next id, or with
/// nothing while `down`.
pub struct FakeRequests {
    calls: Calls,
    next: Cell<RequestId>,
    pub down: Rc<Cell<bool>>,
    /// The session's id.
    session: SessionId,
}

impl FakeRequests {
    fn record(&self, call: Call) -> Option<RequestId> {
        self.calls.borrow_mut().push(call);
        if self.down.get() {
            return None;
        }
        let id = self.next.get();
        self.next.set(id + 1);
        Some(id)
    }
}

impl Requests for FakeRequests {
    fn prompt(&self, text: &str) -> Option<RequestId> {
        self.record(Call::Prompt(text.to_string()))
    }

    fn resend(&self) -> Option<RequestId> {
        self.record(Call::Resend)
    }

    fn unqueue(&self) -> Option<RequestId> {
        self.record(Call::Unqueue)
    }

    fn answer(&self, entry: u64, option: Option<String>) -> Option<RequestId> {
        self.record(Call::Answer(entry, option))
    }

    fn cancel(&self) -> bool {
        self.record(Call::Cancel).is_some()
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

    fn restart(&self, request: AgentRestartRequest) -> Option<RequestId> {
        self.record(Call::Restart(request))
    }

    fn login(&self, method: &str) -> Option<RequestId> {
        self.record(Call::Login(method.to_string()))
    }

    fn fetch(&self, before: u64, max_bytes: u32) -> Option<RequestId> {
        self.record(Call::Fetch(before, max_bytes))
    }

    fn list(&self, cursor: Option<String>) -> Option<RequestId> {
        self.record(Call::List(cursor))
    }

    fn choose(&self, conversation: Option<String>) -> Option<RequestId> {
        self.record(Call::Choose(conversation))
    }

    fn set_option(&self, option: &str, value: OptionValue) -> Option<RequestId> {
        self.record(Call::SetOption(option.to_string(), value))
    }
}

/// A [`Connect`] that records each start and gives fake requests for a
/// new session ([`FakeRequests`], session 8, sharing the zone's calls), or
/// `refuse`s it.
fn fake_connect(calls: Calls, down: Rc<Cell<bool>>, starts: Starts, sinks: Sinks) -> Connect {
    Box::new(move |_host, start, sink| {
        let start = match start {
            AgentStart::Spawn(request) => Start::Spawn(request()),
            AgentStart::Attach { session, .. } => Start::Attach(session),
        };
        starts.borrow_mut().push(start);
        sinks.borrow_mut().push(sink);
        let requests = FakeRequests { calls: calls.clone(), next: Cell::new(101), down: down.clone(), session: 8 };
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
    pub down: Rc<Cell<bool>>,
    pub shared: Arc<Mutex<AgentShared>>,
    /// What the zone's connect started (a continue in a new session), and
    /// the new sessions' sinks.
    pub starts: Starts,
    pub sinks: Sinks,
}

impl Zone {
    pub fn new(first: Option<&str>, cx: &mut TestAppContext) -> Self {
        let (sink, feed) = Sink::with_fault(None);
        let shared = feed.shared.clone();
        let calls = Calls::default();
        let down = Rc::new(Cell::new(false));
        let requests = FakeRequests { calls: calls.clone(), next: Cell::new(1), down: down.clone(), session: 7 };
        let (starts, sinks) = (Starts::default(), Sinks::default());
        let connect = fake_connect(calls.clone(), down.clone(), starts.clone(), sinks.clone());
        let first = first.map(|text| FirstPrompt::new(text.to_string()).unwrap());
        let agent = cx.new(|cx| {
            let model = ChatModel::new("Fake", first);
            let requests = Some(Box::new(requests) as Box<dyn Requests>);
            Agent::from_parts(model, feed, requests, "fake-acp".into(), Host::Local, None, connect, cx)
        });
        Self { sink, agent, calls, down, shared, starts, sinks }
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
