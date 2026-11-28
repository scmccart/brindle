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

// An agent's earlier conversations: the list, the plans and the palette's
// model (OpenSpec change `agents-session-list`).
pub mod conversations;

// Slash commands and session options: completion, the option commands,
// the options under the input and their palette (OpenSpec change
// `agents-commands`).
pub mod commands;

// Whether a close asks first, and what the confirmation says.
pub mod close;

// The fold the reader thread and the zone share.
pub mod shared;

// Markdown in agent messages and thoughts: the app's one door to the parser.
pub mod markdown;

// A tool call's detail, summary, copy text and open state.
pub mod tool;

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
use gpui::{App, Context, EventEmitter, Subscription, Task};
use parking_lot::Mutex;

use crate::FirstPrompt;
use crate::agent::login::{AgentCommand, TerminalLogin};
use crate::agent::model::{
    Absorbed, AfterLogin, CancelLogin, ChatModel, Fetch, First, KeyOutcome, LaunchChoice, Leaving, Link, ListDone,
    ListFor, Choose, LoginChoice, LoginZoneEnd, NotSent, Offer, OnChoosing, ProfileFound, Replied, Sent, Splice,
    Submit, Unqueue,
};
use crate::agent::commands::CompletionKey;
use crate::agent::spawn::AgentSpawnRequest;
use crate::agent::shared::AgentShared;
use crate::agent::tool::ToolPolicy;
use crate::agent::sink::{AgentEvent, Feed, Sink, Taker};
use crate::agent::spawn::{AgentRestartRequest, OpenedBy, agent_spawn_request, continuing_profile};
use crate::agent::transcript::{Conversation, EntryId};
use crate::config::{Config, Profile};
use crate::notify::osc::{Notice, Source};
use crate::remote::protocol::client::OptionValue;
use crate::remote::protocol::{RequestId, SessionId};
use crate::remote::{AgentStart, Host, SessionKey, Unsupported};
use crate::settings::Settings;
use crate::terminal::TerminalEvent;
use crate::terminal::pace::{self, Pace};
use crate::terminal::status::State;

/// The requests an agent zone sends its session: the terminal server's
/// [`crate::remote::AgentLink`], or a test's fake. Each returns the
/// request's id, which its answer comes back under, or why nothing was
/// sent ([`NotSent`], which the model decides on: [`ChatModel::not_sent`]).
pub trait Requests {
    fn prompt(&self, text: &str) -> Result<RequestId, NotSent>;
    fn resend(&self) -> Result<RequestId, NotSent>;
    fn unqueue(&self) -> Result<RequestId, NotSent>;
    fn answer(&self, entry: u64, option: Option<String>) -> Result<RequestId, NotSent>;
    /// `AGENT_CANCEL`, which has no answer.
    fn cancel(&self) -> Result<(), NotSent>;
    fn close(&self);
    fn detach_with(&self, layout: Option<serde_json::Value>);
    fn set_layout(&self, layout: Option<serde_json::Value>);
    fn id_and_key(&self) -> Option<(SessionId, Option<SessionKey>)>;
    /// Runs the session's ended agent again (`AGENT_RESTART`).
    fn restart(&self, request: AgentRestartRequest) -> Result<RequestId, NotSent>;
    /// Logs the agent in with its login method `method` (`AGENT_LOGIN`).
    fn login(&self, method: &str) -> Result<RequestId, NotSent>;
    /// Asks for the entries before `before`, within `max_bytes`
    /// (`AGENT_FETCH`): the page comes as a change before the reply. Never
    /// sent while the link is down ([`NotSent::LinkDown`]).
    fn fetch(&self, before: u64, max_bytes: u32) -> Result<RequestId, NotSent>;
    /// Asks for a page of the agent's conversations after `cursor`
    /// (`AGENT_LIST`): the page comes as [`AgentEvent::Conversations`] in
    /// place of a reply (OpenSpec change `agents-session-list`).
    fn list(&self, cursor: Option<String>) -> Result<RequestId, NotSent>;
    /// Ends a choosing agent's wait (`AGENT_CHOOSE`): continues
    /// `conversation`, or starts a new one with `None`.
    fn choose(&self, conversation: Option<String>) -> Result<RequestId, NotSent>;
    /// Sets the session option whose id is `option` to `value`
    /// (`AGENT_SET_OPTION`, OpenSpec change `agents-commands`): the new
    /// value comes as an `Options` event once the agent answers.
    fn set_option(&self, option: &str, value: OptionValue) -> Result<RequestId, NotSent>;
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
    fn prompt(&self, text: &str) -> Result<RequestId, NotSent> {
        crate::remote::AgentLink::prompt(self, text)
    }

    fn resend(&self) -> Result<RequestId, NotSent> {
        crate::remote::AgentLink::resend(self)
    }

    fn unqueue(&self) -> Result<RequestId, NotSent> {
        crate::remote::AgentLink::unqueue(self)
    }

    fn answer(&self, entry: u64, option: Option<String>) -> Result<RequestId, NotSent> {
        crate::remote::AgentLink::answer(self, entry, option)
    }

    fn cancel(&self) -> Result<(), NotSent> {
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

    fn restart(&self, request: AgentRestartRequest) -> Result<RequestId, NotSent> {
        crate::remote::AgentLink::restart(self, request)
    }

    fn login(&self, method: &str) -> Result<RequestId, NotSent> {
        crate::remote::AgentLink::login(self, method)
    }

    fn fetch(&self, before: u64, max_bytes: u32) -> Result<RequestId, NotSent> {
        crate::remote::AgentLink::fetch(self, before, max_bytes)
    }

    fn list(&self, cursor: Option<String>) -> Result<RequestId, NotSent> {
        crate::remote::AgentLink::list(self, cursor)
    }

    fn choose(&self, conversation: Option<String>) -> Result<RequestId, NotSent> {
        crate::remote::AgentLink::choose(self, conversation)
    }

    fn set_option(&self, option: &str, value: OptionValue) -> Result<RequestId, NotSent> {
        crate::remote::AgentLink::set_option(self, option, value)
    }
}

/// What an [`Agent`] asks of its view, beside the status and title events
/// it shares with terminals ([`TerminalEvent`]); everything else the view
/// reads from the model when the agent notifies.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentUpdate {
    /// The conversation's list changed, in order (`MessageScroller::splice`):
    /// its blocks, then the working row after them (OpenSpec change
    /// `agents-zone-focus-and-scroll`, decision 5).
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
    /// The completion list wrote into the input (OpenSpec change
    /// `agents-commands`): this text, the cursor at this byte.
    SetDraftAt(String, usize),
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

/// What an agent zone asks of its window about its conversations (OpenSpec
/// change `agents-session-list`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentAsk {
    /// Open the conversations palette on the zone (Conversations…, or a
    /// `-r` launch's agent now choosing).
    Conversations,
    /// Open the options palette on the zone (OpenSpec change
    /// `agents-commands`, flag 7): every option (Agent Options…), or option
    /// `option`'s choices (a press under the input, an option command
    /// alone).
    Options { option: Option<String> },
}

/// A set the server refused (OpenSpec change `agents-commands`, flag 11):
/// its words, for the window's notice line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptionRefused(pub String);

/// A list the palette asked for was answered: the page, or why not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentListed(pub Result<conversations::Page, String>);

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
    /// Hands the model `[agent]`'s tool policy on every config reload.
    _settings: Subscription,
}

impl EventEmitter<TerminalEvent> for Agent {}
impl EventEmitter<AgentUpdate> for Agent {}
impl EventEmitter<LoginZone> for Agent {}
impl EventEmitter<AgentAsk> for Agent {}
impl EventEmitter<AgentListed> for Agent {}
impl EventEmitter<OptionRefused> for Agent {}

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
        let model = ChatModel::new(profile.name.clone(), first);
        Self::spawned(model, profile, host, cwd, opened, |request| request, cx)
    }

    /// Starts an agent of `profile` that chooses (OpenSpec change
    /// `agents-session-list` decision 1): initialized, it waits for a
    /// conversation to be chosen, then does what a launch's `launch` asked
    /// (`-c`, `-r`) and sends its `first` prompt once ready. In `cwd` as
    /// [`Self::spawn`] says.
    pub fn spawn_choosing(
        profile: &Profile,
        host: &Host,
        cwd: Option<PathBuf>,
        opened: OpenedBy,
        first: Option<FirstPrompt>,
        launch: Option<LaunchChoice>,
        cx: &mut Context<Self>,
    ) -> Self {
        let home = dirs::home_dir();
        let directory = spawn::directory(profile, host, cwd.clone(), &opened, home.as_deref());
        let model = ChatModel::choosing(profile.name.clone(), first, launch, Self::dir_shown(host, directory.as_deref()));
        Self::spawned(model, profile, host, cwd, opened, AgentSpawnRequest::choosing, cx)
    }

    /// Starts an agent of `profile` on `host` in `cwd` continuing the listed
    /// conversation `id` there (a conversation chosen from a list another
    /// zone's agent gave, decision 5): `SPAWN_AGENT` with the conversation,
    /// as a continue in a new session.
    pub fn spawn_continuing(profile: &Profile, host: &Host, cwd: Option<PathBuf>, id: &str, cx: &mut Context<Self>) -> Self {
        let home = dirs::home_dir();
        let directory = spawn::directory(profile, host, cwd.clone(), &OpenedBy::Window, home.as_deref());
        let model = ChatModel::new(profile.name.clone(), None);
        let (id, dir) = (id.to_string(), directory.map(|dir| dir.to_string_lossy().into_owned()));
        let shape = move |request: AgentSpawnRequest| {
            // The spawn's own directory when this machine can't say it (the
            // home directory on another host).
            let dir = dir.or_else(|| request.cwd.clone()).unwrap_or_default();
            request.continuing_listed(&id, &dir)
        };
        Self::spawned(model, profile, host, cwd, OpenedBy::Window, shape, cx)
    }

    /// `directory` as a reason or a title shows it ([`conversations::shown_dir`]):
    /// under this machine's home with `~` only on this machine.
    pub fn dir_shown(host: &Host, directory: Option<&std::path::Path>) -> String {
        let home = dirs::home_dir().filter(|_| host.is_local());
        conversations::shown_dir(directory, home.as_deref())
    }

    #[allow(clippy::too_many_arguments, reason = "a start's model, profile, host and directory, and how its request is made")]
    fn spawned(
        model: ChatModel,
        profile: &Profile,
        host: &Host,
        cwd: Option<PathBuf>,
        opened: OpenedBy,
        shape: impl FnOnce(AgentSpawnRequest) -> AgentSpawnRequest + Send + 'static,
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
                shape(agent_spawn_request(
                    &profile,
                    &host,
                    cwd,
                    &opened,
                    std::env::vars_os(),
                    home.as_deref(),
                    crate::terminal::process::LOCAL_ENV_IGNORES_CASE,
                ))
            })
        };
        let connect = server_connect();
        let link = connect(host.clone(), AgentStart::Spawn(request), sink);
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
        let mut model = model;
        if let Some(settings) = cx.try_global::<Settings>() {
            model.set_policy(ToolPolicy::of(&settings.config.agent));
        }
        // A reload applies to the rows on screen at once (decision 10).
        let settings = cx.observe_global::<Settings>(|this: &mut Self, cx| {
            let policy = ToolPolicy::of(&Settings::get(cx).config.agent);
            let splices = this.model.set_policy(policy);
            this.splices(splices, cx);
        });
        Self {
            _settings: settings,
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
        match self.through(|requests| requests.restart(request)) {
            Ok(req) => {
                let absorbed = self.model.restarting(req);
                self.apply_absorbed(absorbed, cx);
            }
            Err(why) => {
                let replied = self.model.restart_not_sent(why);
                self.apply_replied(replied, cx);
            }
        }
        cx.notify();
    }

    /// Sends through the zone's session; [`NotSent::Gone`] when it has
    /// none (it never started).
    fn through<T>(&self, send: impl FnOnce(&dyn Requests) -> Result<T, NotSent>) -> Result<T, NotSent> {
        self.requests.as_deref().ok_or(NotSent::Gone).and_then(send)
    }

    /// A new session of `profile`'s agent continuing `conversation`, in its
    /// directory: the zone keeps its place and shows the old conversation
    /// until the new session's first events replace it, as an attach's
    /// replay does.
    fn respawn(&mut self, profile: &Profile, conversation: &Conversation, cx: &mut Context<Self>) {
        let conversation = conversation.clone();
        let directory = PathBuf::from(&conversation.cwd);
        // The server no longer has the zone's session: nothing to close.
        self.renew(profile, directory, Box::new(move |request| request.continuing(&conversation)), false, cx);
    }

    /// A new session of `profile`'s agent in `directory`, its request
    /// shaped by `shape`, in place of the zone's: what a continue in a new
    /// session, a stopped zone's New conversation and a conversation chosen
    /// into a stopped zone do. With `close_old` the zone's old session,
    /// which the server keeps for a stopped agent, is closed first.
    fn renew(
        &mut self,
        profile: &Profile,
        directory: PathBuf,
        shape: Box<dyn FnOnce(AgentSpawnRequest) -> AgentSpawnRequest + Send>,
        close_old: bool,
        cx: &mut Context<Self>,
    ) {
        if let (Held::Holding, Some(requests), true) = (self.held, &self.requests, close_old) {
            requests.close();
        }
        self.held = Held::Holding;
        let (sink, feed) = Sink::new();
        feed.shared.lock().start_over();
        let request = {
            let (profile, host, cwd) = (profile.clone(), self.host.clone(), directory.clone());
            // On the session's thread, as a new zone's start.
            Box::new(move || {
                let home = dirs::home_dir();
                shape(agent_spawn_request(
                    &profile,
                    &host,
                    Some(cwd),
                    &OpenedBy::Window,
                    std::env::vars_os(),
                    home.as_deref(),
                    crate::terminal::process::LOCAL_ENV_IGNORES_CASE,
                ))
            })
        };
        let link = (self.connect)(self.host.clone(), AgentStart::Spawn(request), sink);
        self.command = profile.command.clone().unwrap_or_default();
        self.model.set_command(agent_command(profile, &self.host));
        self.directory = Some(directory);
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

    /// Asks for a page of the agent's conversations after `cursor`, for
    /// `purpose` (OpenSpec change `agents-session-list`): the palette's
    /// answer comes as an [`AgentListed`] event, a launch's is acted on
    /// here. One that can't go is refused as the model says.
    pub fn list(&mut self, cursor: Option<String>, purpose: ListFor, cx: &mut Context<Self>) {
        self.send(Sent::List(purpose), |requests| requests.list(cursor), cx);
    }

    /// Ends a choosing agent's wait: continues `conversation`, or starts a
    /// new one with `None` (`AGENT_CHOOSE`). One that can't go now isn't
    /// sent, and the choosing screen says why ([`ChatModel::choose`]).
    pub fn choose(&mut self, conversation: Option<String>, cx: &mut Context<Self>) {
        match self.model.choose() {
            Choose::Send => match self.through(|requests| requests.choose(conversation)) {
                Ok(request) => self.model.chose(request),
                Err(why) => {
                    let replied = self.model.not_sent(Sent::Choose, why);
                    self.apply_replied(replied, cx);
                }
            },
            Choose::Refused => {}
        }
        cx.notify();
    }

    /// New conversation: a choosing agent starts one (`AGENT_CHOOSE` with
    /// none); a stopped zone's session is replaced by a new one in its
    /// directory, the old one closed. Nothing once the profile is gone.
    pub fn new_conversation(&mut self, cx: &mut Context<Self>) {
        if self.model.is_choosing() {
            self.choose(None, cx);
            return;
        }
        if self.held == Held::Left {
            return;
        }
        let (_, Some(profile)) = self.profile_found(&Settings::get(cx).config) else { return };
        let Some(directory) = self.zone_dir() else { return };
        self.renew(&profile, directory, Box::new(|request| request), true, cx);
        self.model.forget_conversation();
    }

    /// The zone's directory: where its session was started, else its
    /// conversation's.
    fn zone_dir(&self) -> Option<PathBuf> {
        self.directory.clone().or_else(|| self.model.conversation().map(|conversation| PathBuf::from(&conversation.cwd)))
    }

    /// A stopped zone takes the listed conversation `id` of its directory
    /// (a choice from a list another zone's agent gave): a new session
    /// continuing it replaces the zone's, the old one closed.
    pub fn continue_listed(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.held == Held::Left {
            return;
        }
        let (_, Some(profile)) = self.profile_found(&Settings::get(cx).config) else { return };
        let Some(directory) = self.zone_dir() else { return };
        let (id, cwd) = (id.to_string(), directory.to_string_lossy().into_owned());
        self.renew(&profile, directory, Box::new(move |request| request.continuing_listed(&id, &cwd)), true, cx);
        self.model.forget_conversation();
    }

    /// Conversations…: the window opens the conversations palette on the
    /// zone.
    pub fn ask_conversations(&mut self, cx: &mut Context<Self>) {
        cx.emit(AgentAsk::Conversations);
    }

    fn apply_listed(&mut self, done: ListDone, cx: &mut Context<Self>) {
        match done {
            ListDone::Palette(outcome) => cx.emit(AgentListed(outcome)),
            ListDone::Choose(id) => self.choose(Some(id), cx),
            ListDone::Nothing => {}
        }
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
            AgentEvent::Replay(false) => match self.model.replay_ended() {
                Some(absorbed) => absorbed,
                None => return,
            },
            AgentEvent::LinkDown => self.model.set_link(Link::Reconnecting),
            AgentEvent::SessionRunning => self.model.session_running(),
            AgentEvent::Exited(_) => self.model.ended(),
            AgentEvent::Lost(reason) => self.model.lost(reason),
            AgentEvent::Failed(failure) => self.model.not_started(failure),
            AgentEvent::Broken(reason) => self.model.broken(reason),
            AgentEvent::Reply { request, outcome } => {
                let replied = self.model.replied(request, outcome);
                self.apply_replied(replied, cx);
                return;
            }
            AgentEvent::Conversations { request, page } => {
                let done = self.model.listed(request, page);
                self.apply_listed(done, cx);
                return;
            }
            AgentEvent::Note(text) => {
                self.model.server_note(text);
                return;
            }
            AgentEvent::EndpointNote(text) => {
                self.model.set_endpoint_note(text);
                cx.notify();
                return;
            }
        };
        self.apply_absorbed(absorbed, cx);
    }

    /// Hands what the model decided on: the view's splices, the status and
    /// title events, the first prompt's send.
    fn apply_absorbed(&mut self, absorbed: Absorbed, cx: &mut Context<Self>) {
        let Absorbed { splices, status, notice, title_changed, queue_changed: _, first, choosing, replied } = absorbed;
        if let Some(replied) = replied {
            self.apply_replied(replied, cx);
        }
        match choosing {
            Some(OnChoosing::ListLatest) => self.list(None, ListFor::Latest, cx),
            Some(OnChoosing::OpenList) => cx.emit(AgentAsk::Conversations),
            None => {}
        }
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
            Some(First::Send(text)) => {
                let sent = self.model.sent_prompt(text.clone(), true);
                match self.through(|requests| requests.prompt(&text)) {
                    Ok(request) => self.model.sent(request, sent),
                    Err(why) => {
                        let replied = self.model.not_sent(sent, why);
                        self.apply_replied(replied, cx);
                    }
                }
            }
            Some(First::Draft(text) | First::Show(text)) => cx.emit(AgentUpdate::SetDraft(text)),
            Some(First::SendIfUnchanged(text)) => cx.emit(AgentUpdate::SendFirstIfUnchanged(text)),
            None => {}
        }
    }

    /// Sends the launch's first prompt, which waited for a login: the view
    /// found its input still holding exactly `text`.
    pub fn send_first(&mut self, text: &str, cx: &mut Context<Self>) {
        let text = text.to_string();
        let sent = self.model.sent_prompt(text.clone(), true);
        self.send(sent, |requests| requests.prompt(&text), cx);
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
                if let Err(why) = self.through(|requests| requests.cancel()) {
                    self.model.cancel_not_sent(why);
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
            Replied::Listed(done) => self.apply_listed(done, cx),
            Replied::OptionRefused(text) => cx.emit(OptionRefused(text)),
            Replied::Nothing => {}
        }
    }

    /// Sends what `send` sends, recording it as `sent`; one that couldn't
    /// go is refused as the model says.
    fn send(
        &mut self,
        sent: Sent,
        send: impl FnOnce(&dyn Requests) -> Result<RequestId, NotSent>,
        cx: &mut Context<Self>,
    ) {
        match self.through(send) {
            Ok(request) => self.model.sent(request, sent),
            Err(why) => {
                let replied = self.model.not_sent(sent, why);
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
            Submit::Send(text) => {
                let sent = self.model.sent_prompt(text.clone(), false);
                self.send(sent, |requests| requests.prompt(&text), cx);
            }
            Submit::Set { option, name, value } => {
                cx.emit(AgentUpdate::ClearDraft);
                self.set_option(&option, name, value, cx);
            }
            Submit::Choose { option } => {
                cx.emit(AgentUpdate::ClearDraft);
                self.ask_options(Some(option), cx);
            }
            Submit::Nothing(_) => cx.notify(),
        }
    }

    /// Asks the server to set option `option` (named `name`) to `value`
    /// (`AGENT_SET_OPTION`, OpenSpec change `agents-commands`): nothing
    /// shows as changed until the agent answers; a refusal goes to the
    /// window's notice line ([`OptionRefused`]).
    pub fn set_option(&mut self, option: &str, name: String, value: OptionValue, cx: &mut Context<Self>) {
        self.send(Sent::SetOption { name }, |requests| requests.set_option(option, value), cx);
    }

    /// The window's options palette on the zone: every option, or option
    /// `option`'s choices.
    pub fn ask_options(&mut self, option: Option<String>, cx: &mut Context<Self>) {
        cx.emit(AgentAsk::Options { option });
    }

    /// A press on option `id` under the prompt input: a toggle flips, a
    /// select's choices show in the window's palette.
    pub fn footer_pressed(&mut self, id: &str, cx: &mut Context<Self>) {
        match self.model.footer_flip(id) {
            Some((name, value)) => self.set_option(id, name, value, cx),
            None => self.ask_options(Some(id.to_string()), cx),
        }
    }

    /// `agent_next_mode`: the mode after the current one, wrapping; nothing
    /// without a mode option to step.
    pub fn next_mode(&mut self, cx: &mut Context<Self>) {
        if let Some((option, name, value)) = self.model.next_mode() {
            self.set_option(&option, name, value, cx);
        }
    }

    /// The prompt input's text or cursor changed: the completion list
    /// follows ([`ChatModel::typed`]).
    pub fn typed(&mut self, text: &str, cursor: usize, composing: bool, cx: &mut Context<Self>) {
        if self.model.typed(text, cursor, composing) {
            cx.notify();
        }
    }

    /// A key the completion list takes, or a press on its row `label`
    /// (`None`: a key).
    pub fn completion(&mut self, key: Option<CompletionKey>, label: Option<&str>, cx: &mut Context<Self>) {
        let outcome = match (key, label) {
            (Some(key), _) => self.model.completion_key(key),
            (None, Some(label)) => self.model.suggestion_pressed(label),
            (None, None) => None,
        };
        match outcome {
            Some(KeyOutcome::Redraw) | None => {}
            Some(KeyOutcome::Text(text, cursor)) => cx.emit(AgentUpdate::SetDraftAt(text, cursor)),
            Some(KeyOutcome::Set { option, name, value }) => {
                cx.emit(AgentUpdate::ClearDraft);
                self.set_option(&option, name, value, cx);
            }
        }
        cx.notify();
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
        if !self.model.cancel() {
            return;
        }
        if let Err(why) = self.through(|requests| requests.cancel()) {
            self.model.cancel_not_sent(why);
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

    /// The Earlier row is in view (`row_in_view`), and was in the frame
    /// before (`was_in_view`): asks for the page before the window when the
    /// model says so (OpenSpec change `agents-log-paging` decision 11).
    pub fn fetch_if_due(&mut self, row_in_view: bool, was_in_view: bool, cx: &mut Context<Self>) {
        if let Some(fetch) = self.model.fetch_due(row_in_view, was_in_view) {
            self.send_fetch(fetch, cx);
        }
    }

    /// A press on the Earlier row after a failed fetch: asks again.
    pub fn retry_fetch(&mut self, cx: &mut Context<Self>) {
        if let Some(fetch) = self.model.retry() {
            self.send_fetch(fetch, cx);
        }
    }

    fn send_fetch(&mut self, fetch: Fetch, cx: &mut Context<Self>) {
        let max_bytes = u32::try_from(fetch.max_bytes).unwrap_or(u32::MAX);
        match self.through(|requests| requests.fetch(fetch.before.0, max_bytes)) {
            Ok(request) => self.model.fetching(request),
            Err(why) => {
                // A fetch's refusal is its row's state: nothing to apply.
                let _nothing = self.model.not_sent(Sent::Fetch, why);
            }
        }
        cx.notify();
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

    /// A press on tool call `entry`'s row: open or close its detail.
    pub fn toggle_tool(&mut self, entry: EntryId, cx: &mut Context<Self>) {
        let splices = self.model.toggle_tool(entry);
        self.splices(splices, cx);
    }

    /// `toggle_agent_tool_details`.
    pub fn toggle_tool_details(&mut self, cx: &mut Context<Self>) {
        let splices = self.model.toggle_tool_details();
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
            match self.model.leave_on_quit() {
                Leaving::Close => requests.close(),
                Leaving::Detach => requests.detach_with(layout),
            }
        }
        self.held = Held::Left;
    }

    /// Where the agent a conversations palette lists through is
    /// ([`ChatModel::lister_state`]).
    pub fn lister_state(&self) -> model::ListerState {
        self.model.lister_state(&self.command, &self.host.name())
    }

    /// What the conversations plans know of the zone's agent: what it runs
    /// and the conversation it has.
    pub fn run_facts(&self) -> (conversations::ZoneRun, Option<String>) {
        (self.model.zone_run(), self.model.conversation().map(|conversation| conversation.id.clone()))
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
