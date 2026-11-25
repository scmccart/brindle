//! Agent zones: an ACP agent session in the terminal server, shown as a
//! conversation instead of a terminal (`agent-zones`).
//!
//! [`transcript`] is Brindle's one door to `brindle-transcript`: every other
//! file reaches the transcript's types through it.
//!
//! The data flows from the connection's reader thread, where
//! [`sink::Sink`] folds each `TRANSCRIPT` frame into the [`shared`] fold,
//! to the [`Agent`] entity on the UI thread, which drains the sink's gate,
//! copies what changed into its [`model::ChatModel`] and sends what the
//! model decides, to the [`view::AgentView`], which draws the model and
//! never takes the fold's lock (`process R14`).

pub mod transcript;

// What an agent session's start asks for.
pub mod spawn;

// Whether a close asks first, and what the confirmation says.
pub mod close;

// The fold the reader thread and the zone share.
pub mod shared;

// Markdown in agent messages and thoughts: the app's one door to the parser.
pub mod markdown;

// The chat model and the protocol gate.
pub mod model;

// What a login method offers, and what a terminal login runs.
pub mod login;

// The sink the connection's reader folds an agent session's transcript
// through, and its zone's end, which the `Agent` entity drains.
pub mod sink;

// The zone's view.
pub mod view;

use std::path::PathBuf;
use std::sync::Arc;

use futures::StreamExt as _;
use gpui::{App, Context, EventEmitter, Task};
use parking_lot::Mutex;

use crate::FirstPrompt;
use crate::agent::login::{AgentCommand, TerminalLogin};
use crate::agent::model::{
    Absorbed, AfterLogin, CancelLogin, ChatModel, First, Link, LoginChoice, LoginZoneEnd, Offer, ProfileFound, Replied,
    Sent, Splice, Submit, Unqueue,
};
use crate::agent::shared::AgentShared;
use crate::agent::sink::{AgentEvent, Feed, Sink, Taker};
use crate::agent::spawn::{AgentRestartRequest, OpenedBy, agent_spawn_request, continuing_profile};
use crate::agent::transcript::{Conversation, EntryId};
use crate::config::{Config, Profile};
use crate::notify::osc::{Notice, Source};
use crate::remote::protocol::{RequestId, SessionId};
use crate::remote::{AgentStart, Host, SessionKey, Unsupported};
use crate::settings::Settings;
use crate::terminal::TerminalEvent;
use crate::terminal::pace::{self, Pace};
use crate::terminal::status::State;

/// The requests an agent zone sends its session: the terminal server's
/// [`crate::remote::AgentLink`], or a test's fake. Each returns the
/// request's id, which its answer comes back under, or `None` when
/// nothing was sent.
pub trait Requests {
    fn prompt(&self, text: &str) -> Option<RequestId>;
    fn resend(&self) -> Option<RequestId>;
    fn unqueue(&self) -> Option<RequestId>;
    fn answer(&self, entry: u64, option: Option<String>) -> Option<RequestId>;
    /// True when the cancel was sent.
    fn cancel(&self) -> bool;
    fn close(&self);
    fn detach_with(&self, layout: Option<serde_json::Value>);
    fn set_layout(&self, layout: Option<serde_json::Value>);
    fn id_and_key(&self) -> Option<(SessionId, Option<SessionKey>)>;
    /// Runs the session's ended agent again (`AGENT_RESTART`).
    fn restart(&self, request: AgentRestartRequest) -> Option<RequestId>;
    /// Logs the agent in with its login method `method` (`AGENT_LOGIN`).
    fn login(&self, method: &str) -> Option<RequestId>;
}

/// Opens an agent zone's session on a host (`remote::agent_link`), or a
/// test's fake: how a zone starts, attaches, and continues its conversation
/// in a new session.
pub(crate) type Connect = Box<dyn Fn(Host, AgentStart, Sink) -> Result<Box<dyn Requests>, Unsupported>>;

/// The terminal server's [`Connect`].
fn server_connect() -> Connect {
    Box::new(|host, start, sink| {
        crate::remote::agent_link(host, start, sink).map(|link| Box::new(link) as Box<dyn Requests>)
    })
}

impl Requests for crate::remote::AgentLink {
    fn prompt(&self, text: &str) -> Option<RequestId> {
        crate::remote::AgentLink::prompt(self, text)
    }

    fn resend(&self) -> Option<RequestId> {
        crate::remote::AgentLink::resend(self)
    }

    fn unqueue(&self) -> Option<RequestId> {
        crate::remote::AgentLink::unqueue(self)
    }

    fn answer(&self, entry: u64, option: Option<String>) -> Option<RequestId> {
        crate::remote::AgentLink::answer(self, entry, option)
    }

    fn cancel(&self) -> bool {
        crate::remote::AgentLink::cancel(self)
    }

    fn close(&self) {
        crate::remote::AgentLink::close(self);
    }

    fn detach_with(&self, layout: Option<serde_json::Value>) {
        crate::remote::AgentLink::detach_with(self, layout);
    }

    fn set_layout(&self, layout: Option<serde_json::Value>) {
        crate::remote::AgentLink::set_layout(self, layout);
    }

    fn id_and_key(&self) -> Option<(SessionId, Option<SessionKey>)> {
        crate::remote::AgentLink::id_and_key(self)
    }

    fn restart(&self, request: AgentRestartRequest) -> Option<RequestId> {
        crate::remote::AgentLink::restart(self, request)
    }

    fn login(&self, method: &str) -> Option<RequestId> {
        crate::remote::AgentLink::login(self, method)
    }
}

/// What an [`Agent`] asks of its view, beside the status and title events
/// it shares with terminals ([`TerminalEvent`]); everything else the view
/// reads from the model when the agent notifies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentUpdate {
    /// The conversation's blocks changed, in order (`MessageScroller::splice`).
    Splices(Vec<Splice>),
    /// A prompt was taken: the input empties.
    ClearDraft,
    /// Text the user didn't type goes into the input: a first prompt that
    /// wasn't sent (design.md decision 3), or one shown while the agent
    /// needs a login. Never anything from the slot.
    SetDraft(String),
    /// The first prompt shown while the agent needed a login can go now:
    /// the view sends it ([`Agent::send_first`]) only if the input still
    /// holds exactly this (OpenSpec change `agents-login` decision 10).
    SendFirstIfUnchanged(String),
}

/// What an agent zone asks of its tab for a terminal login (OpenSpec
/// change `agents-login`, design decision 9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginZone {
    /// Open a terminal zone beside the agent zone running this.
    Open(TerminalLogin),
    /// Close the login zone (Cancel login).
    Close,
}

/// `profile`'s command on `host`, which a terminal login runs again: its
/// variable names ignore case where this machine's do, on this machine.
fn agent_command(profile: &Profile, host: &Host) -> AgentCommand {
    AgentCommand {
        program: profile.command.clone().unwrap_or_default(),
        args: profile.args.clone(),
        ignore_case: host.is_local() && crate::terminal::process::LOCAL_ENV_IGNORES_CASE,
    }
}

/// Whether the zone has let its session go (closed it or detached it on a
/// quit): once is enough.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Held {
    Holding,
    Left,
}

/// An agent zone's session and its model: the entity that drains the
/// sink, decides through [`ChatModel`] and sends the requests.
pub struct Agent {
    model: ChatModel,
    /// The fold the reader thread writes, read only while absorbing a
    /// change (a leaf lock), never while rendering.
    shared: Arc<Mutex<AgentShared>>,
    requests: Option<Box<dyn Requests>>,
    /// The profile's command, for the alerts.
    command: String,
    host: Host,
    directory: Option<PathBuf>,
    held: Held,
    /// The profile the session was started with, when it isn't the zone's
    /// (a zone attached under a substitute profile): a continue looks this
    /// one up. `None`: the zone's own, following a rename.
    session_profile: Option<String>,
    connect: Connect,
    _events: Task<()>,
}

impl EventEmitter<TerminalEvent> for Agent {}
impl EventEmitter<AgentUpdate> for Agent {}
impl EventEmitter<LoginZone> for Agent {}

impl Agent {
    /// Starts an agent of `profile` in a new session of `host`'s terminal
    /// server, in `cwd` (`-d`) or the profile's directory, else as
    /// `opened` says ([`spawn::directory`]), with a launch's `first`
    /// prompt. Returns at once; a session that can't start shows why.
    pub fn spawn(
        profile: &Profile,
        host: &Host,
        cwd: Option<PathBuf>,
        opened: OpenedBy,
        first: Option<FirstPrompt>,
        cx: &mut Context<Self>,
    ) -> Self {
        let home = dirs::home_dir();
        let directory = spawn::directory(profile, host, cwd.clone(), &opened, home.as_deref());
        let (sink, feed) = Sink::new();
        let request = {
            let (profile, host) = (profile.clone(), host.clone());
            // Built on the session's thread, as a terminal's spawn request
            // is: it reads Brindle's environment.
            Box::new(move || {
                let home = dirs::home_dir();
                agent_spawn_request(
                    &profile,
                    &host,
                    cwd,
                    &opened,
                    std::env::vars_os(),
                    home.as_deref(),
                    crate::terminal::process::LOCAL_ENV_IGNORES_CASE,
                )
            })
        };
        let connect = server_connect();
        let link = connect(host.clone(), AgentStart::Spawn(request), sink);
        let model = ChatModel::new(profile.name.clone(), first);
        Self::linked(model, feed, link, profile, host, directory, connect, cx)
    }

    /// Attaches the detached agent session `session` (with `key`) of
    /// `host`'s terminal server, started in `directory` with the profile
    /// `started_with` (as its listing says; `None`: unknown, taken as
    /// `profile`): listed now (`recorded: false`) or recorded by a restored
    /// zone (`true`). Its replay shows the conversation; an attached zone
    /// has no first prompt.
    #[allow(clippy::too_many_arguments, reason = "the session's identity and where it was listed, each a value the caller holds")]
    pub fn attach(
        profile: &Profile,
        host: &Host,
        session: SessionId,
        key: Option<SessionKey>,
        recorded: bool,
        directory: Option<PathBuf>,
        started_with: Option<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        let (sink, feed) = Sink::new();
        let connect = server_connect();
        let link = connect(host.clone(), AgentStart::Attach { session, key, recorded }, sink);
        let model = ChatModel::new(profile.name.clone(), None);
        let mut agent = Self::linked(model, feed, link, profile, host, directory, connect, cx);
        agent.session_profile = started_with.filter(|name| !name.eq_ignore_ascii_case(&profile.name));
        agent
    }

    #[allow(clippy::too_many_arguments, reason = "what spawn and attach each hold, handed on once")]
    fn linked(
        mut model: ChatModel,
        feed: Feed,
        link: Result<Box<dyn Requests>, Unsupported>,
        profile: &Profile,
        host: &Host,
        directory: Option<PathBuf>,
        connect: Connect,
        cx: &mut Context<Self>,
    ) -> Self {
        let requests = match link {
            Ok(link) => Some(link),
            Err(unsupported) => {
                model.not_started(model::StartFailure::NoServer(unsupported.to_string()));
                None
            }
        };
        model.set_command(agent_command(profile, host));
        let command = profile.command.clone().unwrap_or_default();
        Self::from_parts(model, feed, requests, command, host.clone(), directory, connect, cx)
    }

    /// An agent over `feed` and `requests` as they are, continuing in new
    /// sessions through `connect`: what [`Self::spawn`] and
    /// [`Self::attach`] build on, and what a test gives a fake sink, fake
    /// requests and a fake connect.
    #[allow(clippy::too_many_arguments, reason = "the zone's parts, each a value the caller holds")]
    pub(crate) fn from_parts(
        model: ChatModel,
        feed: Feed,
        requests: Option<Box<dyn Requests>>,
        command: String,
        host: Host,
        directory: Option<PathBuf>,
        connect: Connect,
        cx: &mut Context<Self>,
    ) -> Self {
        let (shared, taker, events) = feed.split();
        Self {
            model,
            shared,
            requests,
            command,
            host,
            directory,
            held: Held::Holding,
            session_profile: None,
            connect,
            _events: Self::drain(events, taker, cx),
        }
    }

    /// As [`Self::attach`] records a session started with another profile.
    #[cfg(test)]
    pub(crate) fn set_started_with(&mut self, name: &str) {
        self.session_profile = Some(name.to_string());
    }

    /// The name of the profile a continue runs: the one the session was
    /// started with.
    fn session_profile_name(&self) -> String {
        self.session_profile.clone().unwrap_or_else(|| self.model.profile_name().to_string())
    }

    /// What continuing the conversation would do now, with the config's
    /// profiles as they are (`agent_continue`, the Continue conversation
    /// button, the dump).
    pub fn offer(&self, cx: &App) -> Offer {
        self.model.offer(&self.profile_found(&Settings::get(cx).config).0)
    }

    /// Whether the session's agent profile is in `config`, and the profile.
    fn profile_found(&self, config: &Config) -> (ProfileFound, Option<Profile>) {
        let name = self.session_profile_name();
        match continuing_profile(&name, config) {
            Some(profile) => (ProfileFound::Agent, Some(profile.clone())),
            None => (ProfileFound::Gone { name }, None),
        }
    }

    /// Continues the conversation as [`Self::offer`] says: restarts the
    /// agent on the session, or starts a new session continuing it.
    /// Nothing when there is no offer, or once the zone let its session go.
    pub fn continue_conversation(&mut self, cx: &mut Context<Self>) {
        if self.held == Held::Left {
            return;
        }
        let (found, profile) = self.profile_found(&Settings::get(cx).config);
        let (offer, Some(profile)) = (self.model.offer(&found), profile) else { return };
        match offer {
            Offer::Restart => self.restart(&profile, cx),
            Offer::Respawn { conversation } => self.respawn(&profile, &conversation, cx),
            Offer::None(_) => {}
        }
    }

    /// `AGENT_RESTART` with `profile`'s command, arguments and environment,
    /// built as a new zone's are; the zone says the agent is starting.
    fn restart(&mut self, profile: &Profile, cx: &mut Context<Self>) {
        let home = dirs::home_dir();
        let request = agent_spawn_request(
            profile,
            &self.host,
            None,
            &OpenedBy::Window,
            std::env::vars_os(),
            home.as_deref(),
            crate::terminal::process::LOCAL_ENV_IGNORES_CASE,
        )
        .restart();
        self.command = request.command.clone();
        match self.requests.as_ref().and_then(|requests| requests.restart(request)) {
            Some(req) => {
                let absorbed = self.model.restarting(req);
                self.apply_absorbed(absorbed, cx);
            }
            None => self.model.server_note(model::UNSENT),
        }
        cx.notify();
    }

    /// A new session of `profile`'s agent continuing `conversation`, in its
    /// directory: the zone keeps its place and shows the old conversation
    /// until the new session's first events replace it, as an attach's
    /// replay does.
    fn respawn(&mut self, profile: &Profile, conversation: &Conversation, cx: &mut Context<Self>) {
        let (sink, feed) = Sink::new();
        feed.shared.lock().start_over();
        let request = {
            let (profile, host, conversation) = (profile.clone(), self.host.clone(), conversation.clone());
            // On the session's thread, as a new zone's start.
            Box::new(move || {
                let home = dirs::home_dir();
                agent_spawn_request(
                    &profile,
                    &host,
                    None,
                    &OpenedBy::Window,
                    std::env::vars_os(),
                    home.as_deref(),
                    crate::terminal::process::LOCAL_ENV_IGNORES_CASE,
                )
                .continuing(&conversation)
            })
        };
        let link = (self.connect)(self.host.clone(), AgentStart::Spawn(request), sink);
        self.command = profile.command.clone().unwrap_or_default();
        self.model.set_command(agent_command(profile, &self.host));
        self.directory = Some(PathBuf::from(&conversation.cwd));
        let (shared, taker, events) = feed.split();
        self.shared = shared;
        self._events = Self::drain(events, taker, cx);
        let absorbed = self.model.respawning();
        self.apply_absorbed(absorbed, cx);
        self.requests = match link {
            Ok(requests) => Some(requests),
            Err(unsupported) => {
                let absorbed = self.model.not_started(model::StartFailure::NoServer(unsupported.to_string()));
                self.apply_absorbed(absorbed, cx);
                None
            }
        };
        cx.notify();
    }

    /// Drains the sink's events in chunks, paced ([`pace`]), and applies
    /// them in order, telling the gate what was taken first, as a
    /// terminal's loop does.
    fn drain(
        events: futures::channel::mpsc::UnboundedReceiver<AgentEvent>,
        taker: Taker,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        cx.spawn(async move |this, cx| {
            let mut rx = events.ready_chunks(256);
            let mut pace = Pace::default();
            while let Some(chunk) = rx.next().await {
                pace.wait(cx).await;
                let alive = this.update(cx, |this, cx| {
                    pace::drain(&mut rx, chunk, |chunk| {
                        if let Some(recovered) = taker.took(chunk.iter()) {
                            // Every agent event is merged or a life event,
                            // which the gate never drops.
                            log::warn!("an agent zone's gate reports {} dropped events", recovered.dropped);
                        }
                        for event in chunk {
                            this.apply(event, cx);
                        }
                    });
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
                pace.updated();
            }
        })
    }

    /// Acts on one event from the sink.
    fn apply(&mut self, event: AgentEvent, cx: &mut Context<Self>) {
        let absorbed = match event {
            AgentEvent::Changed => {
                // The fold is the reader thread's: the zone copies what
                // changed and lets go.
                let mut shared = self.shared.lock();
                let changes = shared.take_changes();
                if changes.is_empty() {
                    return;
                }
                self.model.absorb(shared.transcript(), changes)
            }
            AgentEvent::Replay(true) => return,
            AgentEvent::Replay(false) => match self.model.link() {
                Link::Reconnecting => self.model.set_link(Link::Up),
                Link::Up => return,
            },
            AgentEvent::LinkDown => self.model.set_link(Link::Reconnecting),
            AgentEvent::Exited(_) => self.model.ended(),
            AgentEvent::Lost(reason) => self.model.lost(reason),
            AgentEvent::Failed(failure) => self.model.not_started(failure),
            AgentEvent::Broken(reason) => self.model.broken(reason),
            AgentEvent::Reply { request, outcome } => {
                let replied = self.model.replied(request, outcome);
                self.apply_replied(replied, cx);
                return;
            }
            AgentEvent::Note(text) => {
                self.model.server_note(text);
                return;
            }
        };
        self.apply_absorbed(absorbed, cx);
    }

    /// Hands what the model decided on: the view's splices, the status and
    /// title events, the first prompt's send.
    fn apply_absorbed(&mut self, absorbed: Absorbed, cx: &mut Context<Self>) {
        let Absorbed { splices, status, notice, title_changed, queue_changed: _, first } = absorbed;
        if !splices.is_empty() {
            cx.emit(AgentUpdate::Splices(splices));
        }
        if status.is_some() {
            let notice = notice.map(|notice| Notice { source: Source::Agent, title: notice.title, body: notice.body });
            cx.emit(TerminalEvent::Status(notice));
        }
        if title_changed {
            cx.emit(TerminalEvent::TitleChanged);
        }
        match first {
            Some(First::Send(text)) => match self.requests.as_ref().and_then(|requests| requests.prompt(&text)) {
                Some(request) => self.model.sent(request, Sent::FirstPrompt(text)),
                None => {
                    let replied = self.model.unsent(Sent::FirstPrompt(text));
                    self.apply_replied(replied, cx);
                }
            },
            Some(First::Draft(text) | First::Show(text)) => cx.emit(AgentUpdate::SetDraft(text)),
            Some(First::SendIfUnchanged(text)) => cx.emit(AgentUpdate::SendFirstIfUnchanged(text)),
            None => {}
        }
    }

    /// Sends the launch's first prompt, which waited for a login: the view
    /// found its input still holding exactly `text`.
    pub fn send_first(&mut self, text: &str, cx: &mut Context<Self>) {
        let text = text.to_string();
        self.send(Sent::FirstPrompt(text.clone()), |requests| requests.prompt(&text), cx);
    }

    /// The login row's method `method` was chosen: `AGENT_LOGIN` for an
    /// agent method, a login zone beside for a terminal one; nothing when
    /// the model says so.
    pub fn choose_login(&mut self, method: &str, cx: &mut Context<Self>) {
        match self.model.choose_login(method) {
            LoginChoice::Agent(method) => self.send(Sent::Login, |requests| requests.login(&method), cx),
            LoginChoice::Terminal(plan) => {
                cx.emit(LoginZone::Open(plan));
                cx.notify();
            }
            LoginChoice::Nothing => {}
        }
    }

    /// Cancel login: `AGENT_CANCEL` for the agent's login, the login
    /// zone's close for a terminal login.
    pub fn cancel_login(&mut self, cx: &mut Context<Self>) {
        match self.model.cancel_login() {
            CancelLogin::Send => {
                if !self.requests.as_ref().is_some_and(|requests| requests.cancel()) {
                    self.model.server_note(model::UNSENT);
                }
            }
            CancelLogin::CloseZone => cx.emit(LoginZone::Close),
            CancelLogin::Nothing => return,
        }
        cx.notify();
    }

    /// The terminal login's zone ended as `end`: a login that succeeded
    /// starts the agent again with the profile's command, arguments and
    /// environment (`AGENT_RESTART`), keeping its conversation.
    pub fn login_zone_ended(&mut self, end: LoginZoneEnd, cx: &mut Context<Self>) {
        match self.model.login_zone_ended(end) {
            AfterLogin::Restart => match self.profile_found(&Settings::get(cx).config) {
                (_, Some(profile)) => self.restart(&profile, cx),
                (ProfileFound::Gone { name }, None) => {
                    self.model.server_note(model::NoOffer::ProfileGone { name }.text());
                }
                (ProfileFound::Agent, None) => {}
            },
            AfterLogin::Nothing => {}
        }
        cx.notify();
    }

    fn apply_replied(&mut self, replied: Replied, cx: &mut Context<Self>) {
        match replied {
            Replied::ClearDraft => cx.emit(AgentUpdate::ClearDraft),
            Replied::SetDraft(text) => cx.emit(AgentUpdate::SetDraft(text)),
            Replied::Nothing => {}
        }
    }

    /// Sends what `send` sends, recording it as `sent`; one that couldn't
    /// go is refused as the model says.
    fn send(&mut self, sent: Sent, send: impl FnOnce(&dyn Requests) -> Option<RequestId>, cx: &mut Context<Self>) {
        match self.requests.as_deref().and_then(send) {
            Some(request) => self.model.sent(request, sent),
            None => {
                let replied = self.model.unsent(sent);
                self.apply_replied(replied, cx);
            }
        }
        cx.notify();
    }

    pub fn model(&self) -> &ChatModel {
        &self.model
    }

    /// Enter in the prompt input (or `agent_submit`) with `draft`,
    /// `composing` while an input method composes: sent when the model
    /// says so (during a turn the server queues it), else kept with the
    /// reason shown.
    pub fn prompt(&mut self, draft: &str, composing: bool, cx: &mut Context<Self>) {
        match self.model.submit(draft, composing) {
            Submit::Send(text) => self.send(Sent::Prompt, |requests| requests.prompt(&text), cx),
            Submit::Nothing(_) => cx.notify(),
        }
    }

    /// The held prompt's send again (`agent_resend`).
    pub fn resend(&mut self, cx: &mut Context<Self>) {
        if self.model.resend() {
            self.send(Sent::Resend, |requests| requests.resend(), cx);
        }
    }

    /// The queue row's remove, or a held prompt's discard
    /// (`agent_unqueue`): after the session ended only the row goes.
    pub fn unqueue(&mut self, cx: &mut Context<Self>) {
        match self.model.unqueue() {
            Unqueue::Send => self.send(Sent::Unqueue, |requests| requests.unqueue(), cx),
            Unqueue::Cleared => cx.notify(),
            Unqueue::Nothing => {}
        }
    }

    /// Stops the running turn (escape, the stop button, `agent_cancel`).
    pub fn cancel(&mut self, cx: &mut Context<Self>) {
        if self.model.cancel() && !self.requests.as_ref().is_some_and(|requests| requests.cancel()) {
            self.model.server_note(model::UNSENT);
            cx.notify();
        }
    }

    /// Chooses `option` for the permission request in `entry`.
    pub fn answer(&mut self, entry: EntryId, option: &str, cx: &mut Context<Self>) {
        if self.model.answer(entry, option) {
            let option = option.to_string();
            self.send(Sent::Answer(entry), |requests| requests.answer(entry.0, Some(option)), cx);
        }
    }

    /// The user typed into the prompt input: a `done` or `error` showing
    /// is seen.
    pub fn seen(&mut self, cx: &mut Context<Self>) {
        if self.model.seen().is_some() {
            cx.emit(TerminalEvent::Status(None));
            cx.notify();
        }
    }

    /// A thought's trigger.
    pub fn toggle_thought(&mut self, entry: EntryId, cx: &mut Context<Self>) {
        let splices = self.model.toggle_thought(entry);
        self.splices(splices, cx);
    }

    /// `toggle_agent_thoughts`.
    pub fn toggle_thoughts(&mut self, cx: &mut Context<Self>) {
        let splices = self.model.toggle_thoughts();
        self.splices(splices, cx);
    }

    fn splices(&mut self, splices: Vec<Splice>, cx: &mut Context<Self>) {
        if !splices.is_empty() {
            cx.emit(AgentUpdate::Splices(splices));
            cx.notify();
        }
    }

    /// A restore's note (the recorded profile is gone), shown above the
    /// conversation.
    pub fn note(&mut self, text: &str, cx: &mut Context<Self>) {
        self.model.set_note(text);
        cx.notify();
    }

    /// The status the zone shows (design.md decision 10).
    pub fn status(&self) -> Option<State> {
        self.model.shown_status()
    }

    /// The zone's title: the agent's name, else the profile's.
    pub fn title(&self) -> String {
        self.model.title()
    }

    /// The tab's profile is now called `name` (renamed in the settings
    /// dialog): a title that is the profile's follows it.
    pub fn set_profile_name(&mut self, name: &str, cx: &mut Context<Self>) {
        let before = self.model.title();
        self.model.set_profile_name(name);
        if self.model.title() != before {
            cx.emit(TerminalEvent::TitleChanged);
            cx.notify();
        }
    }

    /// Where the session was started: the spawn's directory, or the one
    /// its listing gave.
    pub fn directory(&self) -> Option<&PathBuf> {
        self.directory.as_ref()
    }

    /// The profile's command, the endpoint's name: what the alerts name.
    pub fn names(&self) -> (&str, String) {
        (&self.command, self.host.name())
    }

    /// The session (id and key) a quit leaves for the next launch, while
    /// the zone holds one that is open ([`ChatModel::keeps_session`]).
    pub fn server_session(&self) -> Option<(SessionId, Option<SessionKey>)> {
        match self.held {
            Held::Holding if self.model.keeps_session() => self.requests.as_ref()?.id_and_key(),
            Held::Holding | Held::Left => None,
        }
    }

    /// The user closed the zone: the session ends (`CLOSE`). Once is
    /// enough.
    pub fn close(&mut self) {
        if let (Held::Holding, Some(requests)) = (self.held, &self.requests) {
            requests.close();
        }
        self.held = Held::Left;
    }

    /// Brindle is quitting: the session is left running, detached with its
    /// tab's `layout` (every agent session is kept: there is no agent
    /// `-e`). Done explicitly, never in a drop (`process R16`).
    pub fn quit_with(&mut self, layout: Option<serde_json::Value>) {
        if let (Held::Holding, Some(requests)) = (self.held, &self.requests) {
            requests.detach_with(layout);
        }
        self.held = Held::Left;
    }

    /// Keeps the core's copy of the tab's layout current while the session
    /// is open.
    pub fn set_layout(&self, layout: Option<serde_json::Value>) {
        if let (Held::Holding, true, Some(requests)) = (self.held, self.model.keeps_session(), &self.requests) {
            requests.set_layout(layout);
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support;

#[cfg(test)]
mod tests;
