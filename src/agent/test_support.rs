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
use crate::agent::transcript::{AgentState, Entry, EntryId, Event};
use crate::agent::{Agent, Requests};
use crate::config::{Config, ConfigReport};
use crate::remote::Host;
use crate::remote::SessionKey;
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
}

pub type Calls = Rc<RefCell<Vec<Call>>>;

/// Requests that record each call and answer with the next id, or with
/// nothing while `down`.
pub struct FakeRequests {
    calls: Calls,
    next: Cell<RequestId>,
    pub down: Rc<Cell<bool>>,
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
        Some((7, None))
    }
}

/// An agent zone's entity over a real sink, with fake requests.
pub struct Zone {
    pub sink: Sink,
    pub agent: Entity<Agent>,
    pub calls: Calls,
    pub down: Rc<Cell<bool>>,
    pub shared: Arc<Mutex<AgentShared>>,
}

impl Zone {
    pub fn new(first: Option<&str>, cx: &mut TestAppContext) -> Self {
        let (sink, feed) = Sink::with_fault(None);
        let shared = feed.shared.clone();
        let calls = Calls::default();
        let down = Rc::new(Cell::new(false));
        let requests = FakeRequests { calls: calls.clone(), next: Cell::new(1), down: down.clone() };
        let first = first.map(|text| FirstPrompt::new(text.to_string()).unwrap());
        let agent = cx.new(|cx| {
            let model = ChatModel::new("Fake", first);
            Agent::from_parts(model, feed, Some(Box::new(requests)), "fake-acp".into(), Host::Local, None, cx)
        });
        Self { sink, agent, calls, down, shared }
    }

    /// Folds `events` on this thread, as the reader thread does.
    pub fn feed(&mut self, events: &[Event]) {
        let values = events.iter().map(Event::to_value).collect();
        self.sink.transcript(values).unwrap();
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
