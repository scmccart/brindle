//! A tab's session in a terminal server, this machine's or a remote
//! endpoint's ([`Host`]), as its terminal holds it from the moment the tab
//! opens.
//!
//! [`Session::start`] and [`Session::attach`] return at once: connecting
//! (starting a core if needed, or reconnecting), an upgrade's step-down,
//! saying what a spawn asks for (which may look at this machine's
//! directories) and the `SPAWN` or `ATTACH` itself all happen on a thread
//! of their own, so opening a tab never blocks the UI thread. Input and
//! resizes made meanwhile are kept, and go out in order once the core
//! reports the session. A failure goes to the sink, which shows it in the
//! tab. A tab closed while its request waits for a connection withdraws
//! the request.

use std::sync::Arc;

use anyhow::Result;
use parking_lot::Mutex;

use super::connection::{Abandon, Budget, Running, Sink};
use super::{AttachRequest, Connection, Foreground, Host, SessionSink, SpawnRequest};
use crate::agent::model::{NotSent, StartFailure};
use crate::agent::spawn::{AgentRestartRequest, AgentSpawnRequest};
use crate::remote::SessionKey;
use crate::remote::budget::{Delivery, Offer};
use crate::remote::protocol::{RequestId, SessionId};

/// How a tab let its session go.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Leaving {
    /// `CLOSE`: the program ends.
    End,
    /// `DETACH`: it keeps running, to be attached later.
    Keep,
}

enum Phase {
    /// The session isn't there yet; what the tab did meanwhile waits.
    Starting(Queued),
    /// The tab let it go before it was there: once it is, the session's
    /// thread ends or detaches it.
    Left(Leaving),
    Running(Running),
    /// It never started (the sink has said why), or the tab let it go.
    Gone,
}

/// What the tab asked of a session still starting, for it once it runs.
#[derive(Debug, Default, PartialEq, Eq)]
struct Queued {
    /// Input, in the order it was typed.
    input: Vec<Vec<u8>>,
    /// The last size asked for.
    resize: Option<Size>,
    /// The window to parent the session's console to (Windows).
    parent: Option<u64>,
}

/// One thing a starting session is handed once it runs.
#[derive(Debug, PartialEq, Eq)]
enum Op {
    Resize(Size),
    Reparent(u64),
    /// Input already counted against the session's budget.
    Input(Vec<u8>),
}

impl Queued {
    /// What the running session is handed, in order: its size, its window,
    /// then the input as typed.
    fn into_ops(self) -> Vec<Op> {
        let Self { input, resize, parent } = self;
        resize
            .map(Op::Resize)
            .into_iter()
            .chain(parent.map(Op::Reparent))
            .chain(input.into_iter().map(Op::Input))
            .collect()
    }

    /// How many bytes of input wait.
    fn held(&self) -> usize {
        self.input.iter().map(Vec::len).sum()
    }
}

/// What letting a session go leaves its tab to do.
enum Departure {
    /// It is still starting: its request is withdrawn, and the input it
    /// held (this many bytes) no longer waits.
    Withdraw { held: usize },
    /// It runs: it is closed or detached now.
    Now(Running),
    /// It is gone already.
    Nothing,
}

impl Phase {
    /// The tab lets the session go, `how`: a session still starting is
    /// left, one running is taken out to be ended now, and the last word
    /// of a tab that lets go twice is the one kept.
    fn leave(&mut self, how: Leaving) -> Departure {
        match std::mem::replace(self, Self::Gone) {
            Self::Starting(queued) => {
                *self = Self::Left(how);
                Departure::Withdraw { held: queued.held() }
            }
            Self::Left(_) => {
                *self = Self::Left(how);
                Departure::Withdraw { held: 0 }
            }
            Self::Running(running) => Departure::Now(running),
            Self::Gone => Departure::Nothing,
        }
    }
}

/// What the session's thread asks the core for.
enum Start {
    /// What to spawn, said on the session's thread once it is connected:
    /// saying it may mean looking at this machine (whether the directory
    /// to start in exists), which must not hold up the thread that opened
    /// the tab.
    Spawn(Box<dyn FnOnce() -> SpawnRequest + Send>),
    /// An agent session's start (`SPAWN_AGENT`), said on the session's
    /// thread as a spawn's is. The connection's writer checks the link's
    /// protocol first ([`agent_start`](crate::agent::model::agent_start)).
    SpawnAgent(Box<dyn FnOnce() -> AgentSpawnRequest + Send>),
    Attach(AttachRequest),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Size {
    cols: u16,
    rows: u16,
    cell_w: u16,
    cell_h: u16,
}

/// A tab's session in a terminal server.
pub struct Session {
    phase: Arc<Mutex<Phase>>,
    /// Lets go of the request while it waits.
    abandon: Arc<Abandon>,
    /// The session an attach asked for, which it stands for until it is
    /// answered: a tab recorded while its endpoint is offline keeps it.
    requested: Option<(SessionId, Option<SessionKey>)>,
    /// What bounds the input waiting to reach the session, across its
    /// phases: input held while it starts counts, and the [`Running`] it
    /// becomes counts against the same budget (`process R9`).
    budget: Budget,
}

impl Session {
    /// Starts a session on `host`, through the process's connection to it.
    /// `request` says what to spawn, and is called on the session's own
    /// thread, once it is connected: what it has to look at on this
    /// machine never holds up the caller.
    pub fn start(
        host: Host,
        request: impl FnOnce() -> SpawnRequest + Send + 'static,
        sink: Box<dyn SessionSink>,
    ) -> Self {
        Self::start_deciding(move || super::shared(&host), request, sink)
    }

    /// Starts a session for `request` in the connection `connect` gives,
    /// on a thread of its own: the tests' way in, with a connection of
    /// their own and a request that needs nothing looked at.
    #[cfg(test)]
    pub fn start_with(
        connect: impl FnOnce() -> Result<Arc<Connection>> + Send + 'static,
        request: SpawnRequest,
        sink: Box<dyn SessionSink>,
    ) -> Self {
        Self::start_deciding(connect, move || request, sink)
    }

    /// Starts a session in the connection `connect` gives, on a thread of
    /// its own, where `request` says what to spawn ([`Session::start`]).
    fn start_deciding(
        connect: impl FnOnce() -> Result<Arc<Connection>> + Send + 'static,
        request: impl FnOnce() -> SpawnRequest + Send + 'static,
        sink: Box<dyn SessionSink>,
    ) -> Self {
        Self::begin(connect, Start::Spawn(Box::new(request)), sink)
    }

    /// Starts an agent session on `host`, through the process's connection
    /// to it, as [`Session::start`] does a terminal's: `request` says what
    /// to run, on the session's own thread. Its transcript goes to `sink`.
    /// A failure to start reaches the sink as a
    /// [`StartFailure`] ([`SessionSink::failed`]'s error downcasts to it).
    pub fn start_agent(
        host: Host,
        request: impl FnOnce() -> AgentSpawnRequest + Send + 'static,
        sink: Box<dyn SessionSink>,
    ) -> Self {
        Self::begin(move || super::shared(&host), Start::SpawnAgent(Box::new(request)), sink)
    }

    /// [`Session::start_agent`] in the connection `connect` gives: the
    /// tests' way in.
    #[cfg(test)]
    pub fn start_agent_with(
        connect: impl FnOnce() -> Result<Arc<Connection>> + Send + 'static,
        request: AgentSpawnRequest,
        sink: Box<dyn SessionSink>,
    ) -> Self {
        Self::begin(connect, Start::SpawnAgent(Box::new(move || request)), sink)
    }

    /// Attaches a detached session on `host`, through the process's
    /// connection to it; its replay and then its live output go to `sink`.
    pub fn attach(host: Host, request: AttachRequest, sink: Box<dyn SessionSink>) -> Self {
        Self::attach_with(move || super::shared(&host), request, sink)
    }

    /// Attaches a session in the connection `connect` gives, on a thread of
    /// its own.
    pub fn attach_with(
        connect: impl FnOnce() -> Result<Arc<Connection>> + Send + 'static,
        request: AttachRequest,
        sink: Box<dyn SessionSink>,
    ) -> Self {
        Self::begin(connect, Start::Attach(request), sink)
    }

    fn begin(
        connect: impl FnOnce() -> Result<Arc<Connection>> + Send + 'static,
        start: Start,
        sink: Box<dyn SessionSink>,
    ) -> Self {
        let requested = match &start {
            Start::Attach(request) => Some((request.session, request.key.clone())),
            Start::Spawn(_) | Start::SpawnAgent(_) => None,
        };
        let phase = Arc::new(Mutex::new(Phase::Starting(Queued::default())));
        let abandon = Arc::new(Abandon::default());
        let budget = Budget::default();
        let sink: Sink = Arc::new(Mutex::new(sink));
        let (starting, abandoning, counting) = (phase.clone(), abandon.clone(), budget.clone());
        let failing = sink.clone();
        let spawned = std::thread::Builder::new()
            .name("server-spawn".into())
            .spawn(move || run(connect, start, sink, &starting, &abandoning, counting));
        if let Err(error) = spawned {
            *phase.lock() = Phase::Gone;
            failing.lock().failed(&anyhow::Error::from(error).context("starting a thread"));
        }
        Self { phase, abandon, requested, budget }
    }

    /// The program's pid, once the session exists. It is a process of the
    /// session's host, which may not be this machine.
    pub fn pid(&self) -> Option<u32> {
        match &*self.phase.lock() {
            Phase::Running(running) => Some(running.pid()),
            Phase::Starting(_) | Phase::Left(_) | Phase::Gone => None,
        }
    }

    /// The session's id and key, once it exists (no key from a core that
    /// predates keys). An attach reports the session it asked for until it
    /// is answered; only a pending spawn reports none.
    pub fn id_and_key(&self) -> Option<(SessionId, Option<SessionKey>)> {
        match &*self.phase.lock() {
            Phase::Running(running) => Some((running.id(), running.key().cloned())),
            Phase::Starting(_) | Phase::Left(_) => self.requested.clone(),
            Phase::Gone => None,
        }
    }

    /// What the core last reported of the session's foreground program
    /// (`PROCESS`, which only a relay connection asks for), while it runs.
    pub fn foreground(&self) -> Option<Foreground> {
        match &*self.phase.lock() {
            Phase::Running(running) => running.foreground(),
            Phase::Starting(_) | Phase::Left(_) | Phase::Gone => None,
        }
    }

    /// Input for the program: held while the session starts, sent once it
    /// runs, either way while its budget takes it. A session the tab let go,
    /// or that never started, discards it.
    pub fn input(&self, bytes: &[u8]) -> Delivery {
        match &mut *self.phase.lock() {
            Phase::Starting(queued) => {
                // Lock order: the phase, then the budget.
                let offer = self.budget.lock().offer(bytes.len());
                match offer {
                    Offer::Take => {
                        queued.input.push(bytes.to_vec());
                        Delivery::Queued
                    }
                    Offer::Refuse(refusal) => Delivery::Refused(refusal),
                }
            }
            Phase::Running(running) => running.input(bytes),
            Phase::Left(_) | Phase::Gone => Delivery::Discarded,
        }
    }

    /// How many bytes of the session's input wait.
    #[cfg(test)]
    fn waiting(&self) -> usize {
        self.budget.lock().waiting()
    }

    pub fn resize(&self, cols: u16, rows: u16, cell_w: u16, cell_h: u16) {
        match &mut *self.phase.lock() {
            Phase::Starting(queued) => queued.resize = Some(Size { cols, rows, cell_w, cell_h }),
            Phase::Running(running) => running.resize(cols, rows, cell_w, cell_h),
            Phase::Left(_) | Phase::Gone => {}
        }
    }

    /// Makes the window `hwnd` the owner of the session's console window
    /// (`REPARENT`), now or as soon as it starts or attaches, and again
    /// after each reconnect's replay. Only a Windows core takes it, so a
    /// remote session (on a relay connection) never sends it.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn reparent(&self, hwnd: u64) {
        match &mut *self.phase.lock() {
            Phase::Starting(queued) => queued.parent = Some(hwnd),
            Phase::Running(running) => running.reparent(hwnd),
            Phase::Left(_) | Phase::Gone => {}
        }
    }

    /// What an agent session's request is sent through: the running
    /// session's handle. While it starts nothing is sent
    /// ([`NotSent::Starting`]): agent requests aren't held, and the zone
    /// sends none until the session's thread says it runs
    /// ([`SessionSink::running`]). After it went, or when it never started,
    /// [`NotSent::Gone`].
    fn running<T>(&self, call: impl FnOnce(&Running) -> Result<T, NotSent>) -> Result<T, NotSent> {
        match &*self.phase.lock() {
            Phase::Running(running) => call(running),
            Phase::Starting(_) => Err(NotSent::Starting),
            Phase::Left(_) | Phase::Gone => Err(NotSent::Gone),
        }
    }

    /// Sends the agent a prompt (`AGENT_PROMPT`) while the session runs;
    /// the request's id, which its answer comes back under
    /// ([`SessionSink::reply`]), or why nothing was sent.
    pub fn prompt(&self, text: &str) -> Result<RequestId, NotSent> {
        self.running(|running| running.prompt(text))
    }

    /// Sends the held prompt again (`AGENT_RESEND`), as [`Session::prompt`].
    pub fn resend(&self) -> Result<RequestId, NotSent> {
        self.running(Running::resend)
    }

    /// Discards the queued prompt, waiting or held (`AGENT_UNQUEUE`), as
    /// [`Session::prompt`].
    pub fn unqueue(&self) -> Result<RequestId, NotSent> {
        self.running(Running::unqueue)
    }

    /// Runs the session's ended agent again to continue its conversation
    /// (`AGENT_RESTART`), as [`Session::prompt`].
    pub fn restart(&self, request: AgentRestartRequest) -> Result<RequestId, NotSent> {
        self.running(|running| running.restart(request))
    }

    /// Logs the agent in with its login method `method` (`AGENT_LOGIN`), as
    /// [`Session::prompt`]; [`Session::cancel`] cancels the login.
    pub fn login(&self, method: &str) -> Result<RequestId, NotSent> {
        self.running(|running| running.login(method))
    }

    /// Fetches the whole entries before `before` within `max_bytes`
    /// (`AGENT_FETCH`), as [`Session::prompt`]; its pages reach the sink
    /// before its reply. Not sent while the link is down
    /// ([`NotSent::LinkDown`]).
    pub fn fetch(&self, before: u64, max_bytes: u32) -> Result<RequestId, NotSent> {
        self.running(|running| running.fetch(before, max_bytes))
    }

    /// Asks for a page of the agent's conversations (`AGENT_LIST`), as
    /// [`Session::prompt`]; the page reaches the sink in place of a reply.
    pub fn list(&self, cursor: Option<String>) -> Result<RequestId, NotSent> {
        self.running(|running| running.list(cursor))
    }

    /// Ends a choosing agent's wait (`AGENT_CHOOSE`), as [`Session::prompt`].
    pub fn choose(&self, conversation: Option<String>) -> Result<RequestId, NotSent> {
        self.running(|running| running.choose(conversation))
    }

    /// Sets session option `option` to `value` (`AGENT_SET_OPTION`), as
    /// [`Session::prompt`].
    pub fn set_option(
        &self,
        option: &str,
        value: crate::remote::protocol::client::OptionValue,
    ) -> Result<RequestId, NotSent> {
        self.running(|running| running.set_option(option, value))
    }

    /// Answers the permission request in entry `entry` with `option`, or
    /// cancelled with none (`AGENT_PERMISSION`), as [`Session::prompt`].
    pub fn answer(&self, entry: u64, option: Option<String>) -> Result<RequestId, NotSent> {
        self.running(|running| running.answer(entry, option))
    }

    /// Cancels the running turn (`AGENT_CANCEL`) while the session runs,
    /// or says why it wasn't sent.
    pub fn cancel(&self) -> Result<(), NotSent> {
        self.running(Running::cancel)
    }

    /// Ends the session (`CLOSE`), or makes sure it ends as soon as it
    /// starts. A session that already exited sends nothing.
    pub fn close(&self) {
        self.leave(Leaving::End, None);
    }

    /// Leaves the session running with no client (`DETACH`), now or as soon
    /// as it starts, with `layout`, the layout of the tab it was in, which
    /// the core keeps for whichever client attaches it next. A session still
    /// starting has no key for a layout to name, and detaches without one. A
    /// session that already exited sends nothing.
    pub fn detach_with(&self, layout: Option<serde_json::Value>) {
        self.leave(Leaving::Keep, layout);
    }

    /// Keeps the core's copy of the session's layout current
    /// ([`Running::set_layout`]: only changes, only on client protocol 3).
    /// A session not running yet sends nothing; the next call catches it.
    pub fn set_layout(&self, layout: Option<serde_json::Value>) {
        if let Phase::Running(running) = &*self.phase.lock() {
            running.set_layout(layout);
        }
    }

    fn leave(&self, how: Leaving, layout: Option<serde_json::Value>) {
        // Held throughout: lock order, the phase then the budget.
        let mut phase = self.phase.lock();
        match phase.leave(how) {
            Departure::Withdraw { held } => {
                self.budget.lock().handled(held);
                // A request still waiting for the connection is withdrawn.
                self.abandon.abandon();
            }
            Departure::Now(running) => match how {
                Leaving::End => running.close(),
                Leaving::Keep => running.detach(layout),
            },
            Departure::Nothing => {}
        }
    }
}

/// Connects, spawns or attaches, and hands the session what was queued
/// meanwhile.
fn run(
    connect: impl FnOnce() -> Result<Arc<Connection>>,
    start: Start,
    sink: Sink,
    phase: &Mutex<Phase>,
    abandon: &Abandon,
    budget: Budget,
) {
    #[cfg(debug_assertions)]
    let delay = start_delay(std::env::var(crate::remote::SESSION_START_DELAY).ok().as_deref());
    let left = || matches!(&*phase.lock(), Phase::Left(_));
    let started = match connect() {
        // An agent zone tells a server it couldn't reach from one that
        // refused the spawn.
        Err(error) => match start {
            Start::SpawnAgent(_) => Err(anyhow::Error::new(StartFailure::NoServer(format!("{error:#}")))),
            Start::Spawn(_) | Start::Attach(_) => Err(error),
        },
        // A tab closed while connecting needs no session: a spawn doesn't
        // start one, and an attach leaves it detached.
        Ok(_) if left() => Err(anyhow::anyhow!("closed before it started")),
        Ok(connection) => match start {
            Start::Spawn(request) => connection.spawn(request(), sink.clone(), Some(abandon)),
            Start::SpawnAgent(request) => {
                let request = request();
                let command = request.command.clone();
                connection.spawn_agent(request, sink.clone(), Some(abandon)).map_err(|error| agent_refused(error, command))
            }
            Start::Attach(request) => connection.attach(request, sink.clone(), Some(abandon)),
        },
    };
    // The fault injector's delay, not a wait for a condition: the core has
    // answered and the reader goes on folding, but the session isn't
    // running yet (`BRINDLE_TEST_SESSION_START_DELAY`, debug builds only).
    #[cfg(debug_assertions)]
    if let (Ok(_), Some(delay)) = (&started, delay) {
        std::thread::sleep(delay);
    }
    let mut phase = phase.lock();
    let running = match started {
        Ok(running) => running.with_budget(budget),
        Err(_) if matches!(&*phase, Phase::Left(_)) => {
            *phase = Phase::Gone;
            return;
        }
        Err(error) => {
            *phase = Phase::Gone;
            drop(phase);
            sink.lock().failed(&error);
            return;
        }
    };
    // Under the lock, so later input waits for what was queued.
    let queued = match std::mem::replace(&mut *phase, Phase::Gone) {
        Phase::Starting(queued) => queued,
        Phase::Left(Leaving::End) => return running.close(),
        Phase::Left(Leaving::Keep) => return running.detach(None),
        Phase::Running(_) | Phase::Gone => unreachable!("only this thread leaves Starting"),
    };
    for op in queued.into_ops() {
        match op {
            Op::Resize(size) => running.resize(size.cols, size.rows, size.cell_w, size.cell_h),
            Op::Reparent(hwnd) => running.reparent(hwnd),
            // Counted when it was held: not offered again.
            Op::Input(bytes) => running.held_input(&bytes),
        }
    }
    *phase = Phase::Running(running);
    // The phase lock first, then the sink's, never both: the zone answers
    // this by sending through the running session (decision 2), and every
    // event the reader delivered before is queued before it.
    drop(phase);
    sink.lock().running();
}

/// `BRINDLE_TEST_SESSION_START_DELAY`'s value (debug builds only,
/// `process R20`): a whole number of milliseconds; anything else, or
/// nothing, delays nothing.
#[cfg(debug_assertions)]
fn start_delay(value: Option<&str>) -> Option<std::time::Duration> {
    let millis = value.filter(|millis| !millis.is_empty() && millis.bytes().all(|b| b.is_ascii_digit()))?;
    millis.parse().ok().map(std::time::Duration::from_millis)
}

/// An agent spawn's failure as its zone tells it: the gate's own refusal
/// (a server too old) as it is, anything else the server's (or the link's)
/// refusal of `command`, verbatim.
fn agent_refused(error: anyhow::Error, command: String) -> anyhow::Error {
    match error.downcast::<StartFailure>() {
        Ok(failure) => anyhow::Error::new(failure),
        Err(error) => anyhow::Error::new(StartFailure::Refused { command, cause: format!("{error:#}") }),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use super::*;
    use crate::agent::model::ReplyError;
    use crate::agent::sink::{AgentEvent, Feed, Sink as AgentSink};
    use crate::agent::transcript::{
        AgentState, Entry, EntryId, Event, HoldReason, PermissionOutcome, Slot, StopReason, Transcript, TurnState,
    };
    use crate::remote::local::sys::Listener;
    use crate::remote::local::{Attaching, Endpoint, Stream, connect};
    use crate::remote::protocol::Frame;
    use crate::remote::budget::Refusal;
    use crate::remote::protocol::client::{self, KillTarget, Spawned, ToClient, ToCore, Welcome};
    use crate::test_support::TempDir;
    use crate::test_support::wait::{limit, until};

    #[cfg(debug_assertions)]
    #[test]
    fn the_session_start_delay_switch_reads_whole_milliseconds() {
        assert_eq!(start_delay(Some("1500")), Some(Duration::from_millis(1500)));
        assert_eq!(start_delay(Some("0")), Some(Duration::ZERO));
        for value in [None, Some(""), Some("1.5"), Some("-3"), Some("2s"), Some("99999999999999999999999")] {
            assert_eq!(start_delay(value), None, "{value:?}");
        }
    }

    /// What the tab saw.
    struct Recorder(mpsc::Sender<String>);

    impl SessionSink for Recorder {
        fn output(&mut self, bytes: &[u8]) {
            self.0.send(format!("output {}", String::from_utf8_lossy(bytes))).ok();
        }
        fn note(&mut self, text: &str) {
            self.0.send(format!("note {text}")).ok();
        }
        fn replay(&mut self, active: bool) {
            self.0.send(format!("replay {active}")).ok();
        }
        fn exited(&mut self, code: Option<i32>) {
            self.0.send(format!("exited {code:?}")).ok();
        }
        fn lost(&mut self, reason: &str) {
            self.0.send(format!("lost {reason}")).ok();
        }
        fn failed(&mut self, error: &anyhow::Error) {
            self.0.send(format!("failed {error:#}")).ok();
        }
        fn foreground(&mut self) {
            self.0.send("foreground".into()).ok();
        }
        fn broken(&mut self, reason: &str) {
            self.0.send(format!("broken {reason}")).ok();
        }
    }

    fn request() -> SpawnRequest {
        SpawnRequest {
            command: Some("sh".into()),
            args: Vec::new(),
            cwd: None,
            env: BTreeMap::new(),
            cols: 80,
            rows: 24,
            cell_w: 8,
            cell_h: 16,
            scrollback: 0,
            policy: client::Policy::Detach,
            profile: Some("Shell".into()),
        }
    }

    /// A fresh directory for one test, removed when it ends.
    fn temp_dir(tag: &str) -> TempDir {
        TempDir::new(&format!("session-{tag}"))
    }

    /// A core that holds its `WELCOME` until the test sends on the gate it
    /// gives back, starts one session, and reports the messages it gets
    /// after `SPAWN`.
    fn slow_core(dir: &std::path::Path) -> (mpsc::Receiver<ToCore>, mpsc::Sender<()>) {
        let listener = Listener::bind(dir);
        let (tx, rx) = mpsc::channel();
        let (open, gate) = mpsc::channel::<()>();
        std::thread::spawn(move || {
            let mut stream = listener.accept();
            let ToCore::Hello(_) = ToCore::decode(&Frame::read_from(&mut stream).unwrap()).unwrap() else {
                panic!("not HELLO")
            };
            gate.recv().ok();
            let welcome = Welcome {
                protocol: 1,
                core_version: "0.0.0".into(),
                core_build: "test".into(),
                pid: std::process::id(),
                holder_protocols: vec![1],
            };
            ToClient::Welcome(welcome).encode().unwrap().write_to(&mut stream).unwrap();
            while let Ok(frame) = Frame::read_from(&mut stream) {
                let message = ToCore::decode(&frame).unwrap();
                if let ToCore::Spawn(spawn) = &message {
                    let spawned = Spawned { req: spawn.req, session: 7, pid: 4242, key: "k7".into() };
                    ToClient::Spawned(spawned).encode().unwrap().write_to(&mut stream).unwrap();
                    let output = client::Data { session: 7, bytes: b"$ ".to_vec() };
                    ToClient::Data(output).encode().unwrap().write_to(&mut stream).unwrap();
                }
                if tx.send(message).is_err() {
                    break;
                }
            }
        });
        (rx, open)
    }

    fn send(stream: &mut Stream, message: ToClient) {
        message.encode().unwrap().write_to(stream).unwrap();
    }

    /// A core that welcomes one client, then hands each message to `answer`
    /// (which may reply) and reports it.
    fn scripted_core(
        dir: &std::path::Path,
        answer: impl FnMut(&ToCore, &mut Stream) + Send + 'static,
    ) -> mpsc::Receiver<ToCore> {
        scripted_core_at(dir, 1, answer)
    }

    /// [`scripted_core`], agreeing on client protocol `protocol`.
    fn scripted_core_at(
        dir: &std::path::Path,
        protocol: u32,
        mut answer: impl FnMut(&ToCore, &mut Stream) + Send + 'static,
    ) -> mpsc::Receiver<ToCore> {
        let listener = Listener::bind(dir);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut stream = listener.accept();
            let ToCore::Hello(_) = ToCore::decode(&Frame::read_from(&mut stream).unwrap()).unwrap() else {
                panic!("not HELLO")
            };
            let welcome = Welcome {
                protocol,
                core_version: "0.0.0".into(),
                core_build: "test".into(),
                pid: std::process::id(),
                holder_protocols: vec![2],
            };
            send(&mut stream, ToClient::Welcome(welcome));
            while let Ok(frame) = Frame::read_from(&mut stream) {
                let message = ToCore::decode(&frame).unwrap();
                match &message {
                    // [`settled`]'s round trip, which it answers itself.
                    ToCore::Kill(kill) if kill.target == KillTarget::Session(SETTLE) => {
                        let message = "no such session".to_string();
                        send(&mut stream, ToClient::Error(client::ErrorMessage { req: Some(kill.req), session: None, message }));
                    }
                    _ => answer(&message, &mut stream),
                }
                if tx.send(message).is_err() {
                    break;
                }
            }
        });
        rx
    }

    /// The session [`settled`]'s `KILL` names, which a scripted core answers
    /// itself.
    const SETTLE: SessionId = SessionId::MAX;

    /// Two round trips through a scripted core, the positive marker for
    /// "nothing more was sent": after the first, the client has read all
    /// the core sent before its answer; after the second, the core has read
    /// all the client sent in reply to that. The messages the core got
    /// before the second are returned, the round trips' left out.
    fn settled(connection: &Connection, core: &mpsc::Receiver<ToCore>) -> Vec<ToCore> {
        let mut before = Vec::new();
        for _ in 0..2 {
            assert!(connection.kill(SETTLE).is_err(), "the round trip's KILL");
            loop {
                match recv(core) {
                    ToCore::Kill(kill) if kill.target == KillTarget::Session(SETTLE) => break,
                    other => before.push(other),
                }
            }
        }
        before
    }

    /// The connection a scripted core at `dir` serves, for sessions started
    /// on it and for [`settled`].
    fn connected(dir: &std::path::Path) -> Arc<Connection> {
        connector(dir)().unwrap()
    }

    /// Waits, within a scaled 10 s, until `done` holds.
    fn eventually(mut done: impl FnMut() -> bool) {
        until("the condition", Duration::from_secs(10), || done().then_some(()));
    }

    fn connector(dir: &std::path::Path) -> impl FnOnce() -> Result<Arc<Connection>> + Send + 'static {
        let endpoint = Endpoint::at(dir.join("unused"), dir.to_path_buf());
        move || {
            let connected = connect(&endpoint, "tok")?;
            Connection::start(endpoint, "tok", connected)
        }
    }

    fn attach_request(session: SessionId) -> AttachRequest {
        AttachRequest {
            session,
            key: SessionKey::from_wire(&format!("k{session}")),
            when: Attaching::Now { cols: 120, rows: 40 },
        }
    }

    /// Answers `ATTACH` of session 5 with a replay, then live output.
    fn replaying(message: &ToCore, stream: &mut Stream) {
        if let ToCore::Attach(attach) = message {
            let session = attach.session;
            send(stream, ToClient::ReplayStart(client::ReplayStart { req: Some(attach.req), session, pid: 99 }));
            send(stream, ToClient::Data(client::Data { session, bytes: b"screen".to_vec() }));
            send(stream, ToClient::ReplayEnd(client::SessionRef { session }));
            send(stream, ToClient::Data(client::Data { session, bytes: b"live".to_vec() }));
        }
    }

    fn recv(rx: &mpsc::Receiver<ToCore>) -> ToCore {
        rx.recv_timeout(limit(Duration::from_secs(10))).unwrap()
    }

    fn seen(rx: &mpsc::Receiver<String>) -> String {
        rx.recv_timeout(limit(Duration::from_secs(10))).unwrap()
    }

    /// `REPARENT` asked for while the session starts goes out once it has,
    /// and again after a replay (a reconnect's: the core may be a new one),
    /// but not for a session the tab closed.
    #[test]
    fn a_reparent_waits_for_the_session_and_follows_each_replay() {
        let dir = temp_dir("reparent");
        let mut replayed = false;
        let core = scripted_core(&dir, move |message, stream| match message {
            ToCore::Spawn(spawn) => {
                send(stream, ToClient::Spawned(Spawned { req: spawn.req, session: 7, pid: 4242, key: "k7".into() }));
            }
            ToCore::Reparent(_) if !replayed => {
                replayed = true;
                send(stream, ToClient::ReplayStart(client::ReplayStart { req: None, session: 7, pid: 4242 }));
                send(stream, ToClient::ReplayEnd(client::SessionRef { session: 7 }));
            }
            ToCore::Close(_) => send(stream, ToClient::ReplayEnd(client::SessionRef { session: 7 })),
            _ => {}
        });
        let (tx, tab) = mpsc::channel();
        let connection = connected(&dir);
        let starting = connection.clone();
        let session = Session::start_with(move || Ok(starting), request(), Box::new(Recorder(tx)));
        session.reparent(0x1234);
        assert!(matches!(recv(&core), ToCore::Spawn(_)));
        let reparent = client::Reparent { session: 7, hwnd: 0x1234 };
        assert_eq!(recv(&core), ToCore::Reparent(reparent.clone()));
        assert_eq!(seen(&tab), "replay true");
        assert_eq!(seen(&tab), "replay false");
        assert_eq!(recv(&core), ToCore::Reparent(reparent));
        // Once running it goes straight out.
        session.reparent(0x5678);
        assert_eq!(recv(&core), ToCore::Reparent(client::Reparent { session: 7, hwnd: 0x5678 }));
        session.close();
        assert!(matches!(recv(&core), ToCore::Close(_)));
        // The replay end after the close brings nothing more.
        session.reparent(0x9abc);
        assert_eq!(settled(&connection, &core), []);
    }

    /// `REPARENT` goes out again after a real reconnect: the core's
    /// connection drops, Brindle connects again with its token, and the
    /// (new) core's replay of the session is followed by the window it had.
    #[test]
    fn a_reparent_is_sent_again_after_the_link_drops_and_reconnects() {
        let dir = temp_dir("reparent-reconnect");
        let listener = Listener::bind(&dir);
        let (tx, core) = mpsc::channel();
        std::thread::spawn(move || {
            let welcome = |stream: &mut Stream| {
                let ToCore::Hello(_) = ToCore::decode(&Frame::read_from(stream).unwrap()).unwrap() else {
                    panic!("not HELLO")
                };
                let welcome = Welcome {
                    protocol: 1,
                    core_version: "0.0.0".into(),
                    core_build: "test".into(),
                    pid: std::process::id(),
                    holder_protocols: vec![2],
                };
                send(stream, ToClient::Welcome(welcome));
            };
            // The first core: starts session 7, takes its REPARENT, and goes.
            let mut stream = listener.accept();
            welcome(&mut stream);
            loop {
                let message = ToCore::decode(&Frame::read_from(&mut stream).unwrap()).unwrap();
                if let ToCore::Spawn(spawn) = &message {
                    send(&mut stream, ToClient::Spawned(Spawned { req: spawn.req, session: 7, pid: 4242, key: "k7".into() }));
                }
                let done = matches!(message, ToCore::Reparent(_));
                tx.send((1, message)).unwrap();
                if done {
                    break;
                }
            }
            drop(stream);
            // The next core: replays the session it still has when asked.
            let mut stream = listener.accept();
            welcome(&mut stream);
            while let Ok(frame) = Frame::read_from(&mut stream) {
                let message = ToCore::decode(&frame).unwrap();
                if let ToCore::List(list) = &message {
                    send(&mut stream, ToClient::ReplayStart(client::ReplayStart { req: None, session: 7, pid: 4242 }));
                    send(&mut stream, ToClient::ReplayEnd(client::SessionRef { session: 7 }));
                    let info = client::SessionInfo { session: 7, key: "k7".into(), attached: true, ..Default::default() };
                    send(&mut stream, ToClient::Sessions(client::Sessions { req: list.req, sessions: vec![info] }));
                }
                if tx.send((2, message)).is_err() {
                    break;
                }
            }
        });
        let (tab_tx, tab) = mpsc::channel();
        let session = Session::start_with(connector(&dir), request(), Box::new(Recorder(tab_tx)));
        session.reparent(0x1234);
        let next = || core.recv_timeout(Duration::from_secs(20)).unwrap();
        assert!(matches!(next(), (1, ToCore::Spawn(_))));
        let reparent = client::Reparent { session: 7, hwnd: 0x1234 };
        assert_eq!(next(), (1, ToCore::Reparent(reparent.clone())));
        assert!(matches!(next(), (2, ToCore::List(_))));
        assert_eq!(seen(&tab), "replay true");
        assert_eq!(seen(&tab), "replay false");
        assert_eq!(next(), (2, ToCore::Reparent(reparent)));
        session.close();
    }

    #[test]
    fn an_attach_gets_the_replay_then_live_output_and_queued_input() {
        let dir = temp_dir("attach");
        let core = scripted_core(&dir, replaying);
        let (tx, tab) = mpsc::channel();
        let connection = connected(&dir);
        let attaching = connection.clone();
        let session = Session::attach_with(move || Ok(attaching), attach_request(5), Box::new(Recorder(tx)));
        session.resize(100, 30, 9, 18);
        session.input(b"typed early");
        let ToCore::Attach(attach) = recv(&core) else { panic!("not ATTACH") };
        assert_eq!((attach.session, attach.cols, attach.rows), (5, 120, 40));
        // The sink was in place before the replay's first DATA.
        assert_eq!(seen(&tab), "replay true");
        assert_eq!(seen(&tab), "output screen");
        assert_eq!(seen(&tab), "replay false");
        assert_eq!(seen(&tab), "output live");
        let ToCore::Resize(resize) = recv(&core) else { panic!("not RESIZE") };
        assert_eq!((resize.session, resize.cols, resize.rows), (5, 100, 30));
        let ToCore::Data(data) = recv(&core) else { panic!("not DATA") };
        assert_eq!((data.session, data.bytes.as_slice()), (5, &b"typed early"[..]));
        assert_eq!(session.pid(), Some(99));
        assert_eq!(session.id_and_key(), Some((5, SessionKey::from_wire("k5"))));

        session.detach_with(None);
        assert!(matches!(recv(&core), ToCore::Detach(detach) if detach.session == 5));
        // Gone: neither again, nor a close.
        session.detach_with(None);
        session.close();
        session.input(b"late");
        assert_eq!(settled(&connection, &core), []);
    }

    /// A quit's detach carries its tab's layout to the core.
    #[test]
    fn a_detach_with_a_layout_sends_it() {
        let dir = temp_dir("detach-layout");
        let core = scripted_core(&dir, replaying);
        let (tx, _tab) = mpsc::channel();
        let session = Session::attach_with(connector(&dir), attach_request(5), Box::new(Recorder(tx)));
        assert!(matches!(recv(&core), ToCore::Attach(_)));
        let layout = serde_json::json!({"v": 1, "quit": "q", "index": 0, "zones": {"key": "k5", "active": true}});
        // Once the attach is answered: a session still starting has no key.
        eventually(|| session.pid().is_some());
        session.detach_with(Some(layout.clone()));
        loop {
            match recv(&core) {
                ToCore::Detach(detach) => {
                    assert_eq!((detach.session, detach.layout), (5, Some(layout)));
                    break;
                }
                ToCore::Resize(_) | ToCore::Data(_) => {}
                other => panic!("not DETACH: {other:?}"),
            }
        }
    }

    /// On client protocol 3 a layout goes out only when it changed; below 3
    /// none does (the quit's `DETACH` carries it).
    #[test]
    fn layouts_are_sent_once_per_change_and_only_on_protocol_3() {
        for protocol in [3, 2] {
            let dir = temp_dir(&format!("set-layout-{protocol}"));
            let core = scripted_core_at(&dir, protocol, replaying);
            let (tx, _tab) = mpsc::channel();
            let session = Session::attach_with(connector(&dir), attach_request(5), Box::new(Recorder(tx)));
            assert!(matches!(recv(&core), ToCore::Attach(_)));
            eventually(|| session.pid().is_some());
            let one = serde_json::json!({"v": 1, "origin": "o", "index": 0, "zones": {"key": "k5"}});
            let two = serde_json::json!({"v": 1, "origin": "o", "index": 1, "zones": {"key": "k5"}});
            for layout in [&one, &one, &two, &two] {
                session.set_layout(Some(layout.clone()));
            }
            // The detach marks the end of what the session sent.
            session.detach_with(None);
            let mut sent = Vec::new();
            loop {
                match recv(&core) {
                    ToCore::SetLayout(set) => sent.push(set.layout),
                    ToCore::Detach(_) => break,
                    _ => {}
                }
            }
            let expected = if protocol >= 3 { vec![Some(one), Some(two)] } else { Vec::new() };
            assert_eq!(sent, expected, "protocol {protocol}");
        }
    }

    #[test]
    fn a_refused_attach_goes_to_the_tab() {
        let dir = temp_dir("attach-refused");
        let _core = scripted_core(&dir, |message, stream| {
            if let ToCore::Attach(attach) = message {
                let message = format!("session {} is attached to another client", attach.session);
                let error = client::ErrorMessage { req: Some(attach.req), session: Some(attach.session), message };
                send(stream, ToClient::Error(error));
            }
        });
        let (tx, tab) = mpsc::channel();
        let session = Session::attach_with(connector(&dir), attach_request(3), Box::new(Recorder(tx)));
        assert_eq!(seen(&tab), "failed session 3 is attached to another client");
        assert_eq!(session.pid(), None);
    }

    #[test]
    fn a_tab_closed_while_attaching_leaves_the_session_detached() {
        let dir = temp_dir("attach-closed");
        let (release, wait) = mpsc::channel::<()>();
        let wait = Mutex::new(wait);
        let (got_tx, got) = mpsc::channel();
        let core = scripted_core(&dir, move |message, stream| {
            if matches!(message, ToCore::Attach(_)) {
                got_tx.send(()).ok();
                wait.lock().recv().ok();
            }
            replaying(message, stream);
        });
        let (tx, _tab) = mpsc::channel();
        let connection = connected(&dir);
        let attaching = connection.clone();
        let session = Session::attach_with(move || Ok(attaching), attach_request(5), Box::new(Recorder(tx)));
        got.recv_timeout(limit(Duration::from_secs(10))).unwrap();
        session.detach_with(None);
        release.send(()).unwrap();
        let ToCore::Attach(_) = recv(&core) else { panic!("not ATTACH") };
        assert!(matches!(recv(&core), ToCore::Detach(detach) if detach.session == 5));
        assert_eq!(settled(&connection, &core), [], "no CLOSE");
    }

    #[test]
    fn a_spawned_session_closes_or_detaches_once_and_not_after_its_exit() {
        let dir = temp_dir("leave");
        let core = scripted_core(&dir, |message, stream| {
            if let ToCore::Spawn(spawn) = message {
                let session = 10 + spawn.req as u32;
                let spawned = Spawned { req: spawn.req, session, pid: 4000 + spawn.req as u32, key: String::new() };
                send(stream, ToClient::Spawned(spawned));
                if spawn.cwd.as_deref() == Some("/exits") {
                    send(stream, ToClient::Exit(client::Exit { session, code: Some(0), lost: None }));
                }
            }
        });
        let connection = connector(&dir)().unwrap();
        let start = |cwd: &str| {
            let (tx, tab) = mpsc::channel();
            let mut spawn = request();
            spawn.cwd = Some(cwd.into());
            let connection = connection.clone();
            let session = Session::start_with(move || Ok(connection), spawn, Box::new(Recorder(tx)));
            let ToCore::Spawn(spawn) = recv(&core) else { panic!("not SPAWN") };
            (session, tab, 10 + spawn.req as u32)
        };

        let (ended, _tab, id) = start("/a");
        eventually(|| ended.pid().is_some());
        // This core predates keys: the session has none.
        assert_eq!(ended.id_and_key(), Some((id, None)));
        ended.close();
        ended.detach_with(None);
        assert!(matches!(recv(&core), ToCore::Close(close) if close.session == id));

        let (exited, tab, _) = start("/exits");
        assert_eq!(seen(&tab), "exited Some(0)");
        exited.close();
        exited.detach_with(None);
        assert_eq!(settled(&connection, &core), [], "nothing for an exited session");
    }

    /// What quitting does to a window's tabs: each terminal leaves its
    /// session explicitly (`Terminal::quit_with`: a profile tab's is detached,
    /// a `brindle terminal -e` one's closed), then the drop, when GPUI runs it, leaves it
    /// again, which sends nothing more. The flush before the exit returns
    /// once both are written.
    #[test]
    fn quitting_leaves_each_session_once_and_the_flush_writes_it() {
        let dir = temp_dir("quit");
        let core = scripted_core(&dir, |message, stream| {
            if let ToCore::Spawn(spawn) = message {
                let spawned = Spawned { req: spawn.req, session: 30 + spawn.req as u32, pid: 1, key: "k".into() };
                send(stream, ToClient::Spawned(spawned));
            }
        });
        let connection = connector(&dir)().unwrap();
        let start = || {
            let connection = connection.clone();
            let session = Session::start_with(move || Ok(connection), request(), Box::new(Recorder(mpsc::channel().0)));
            let ToCore::Spawn(spawn) = recv(&core) else { panic!("not SPAWN") };
            eventually(|| session.pid().is_some());
            (session, 30 + spawn.req as u32)
        };
        let (kept, kept_id) = start();
        let (ended, ended_id) = start();

        // The quit, then the drops.
        kept.detach_with(None);
        ended.close();
        kept.detach_with(None);
        ended.close();
        assert!(connection.flush(Instant::now() + Duration::from_secs(10)), "not written");
        assert!(matches!(recv(&core), ToCore::Detach(detach) if detach.session == kept_id));
        assert!(matches!(recv(&core), ToCore::Close(close) if close.session == ended_id));
        assert_eq!(settled(&connection, &core), [], "a session was left twice");
    }

    #[test]
    fn the_connection_lists_kills_and_ends_all_before_quitting() {
        let dir = temp_dir("manage");
        let core = scripted_core(&dir, |message, stream| match message {
            ToCore::List(list) => {
                let info = client::SessionInfo {
                    session: 4,
                    key: "k4".into(),
                    title: "vim".into(),
                    started: 1_700_000_000,
                    profile: Some("Shell".into()),
                    ..Default::default()
                };
                send(stream, ToClient::Sessions(client::Sessions { req: list.req, sessions: vec![info] }));
            }
            ToCore::Kill(kill) => match kill.target {
                client::KillTarget::Session(4) => send(stream, ToClient::Ok(client::Request { req: kill.req })),
                _ => {
                    let error = client::ErrorMessage { req: Some(kill.req), session: None, message: "no session 9".into() };
                    send(stream, ToClient::Error(error));
                }
            },
            ToCore::Spawn(spawn) => {
                let spawned = Spawned { req: spawn.req, session: 20 + spawn.req as u32, pid: 1, key: "k".into() };
                send(stream, ToClient::Spawned(spawned));
            }
            _ => {}
        });
        let connection = connector(&dir)().unwrap();
        let listed = connection.list().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!((listed[0].session, listed[0].key.as_str(), listed[0].title.as_str()), (4, "k4", "vim"));
        assert!(matches!(recv(&core), ToCore::List(_)));
        connection.kill(4).unwrap();
        assert!(matches!(recv(&core), ToCore::Kill(_)));
        assert_eq!(connection.kill(9).unwrap_err().to_string(), "no session 9");
        assert!(matches!(recv(&core), ToCore::Kill(_)));

        let sink: Sink = Arc::new(Mutex::new(Box::new(Recorder(mpsc::channel().0))));
        let running: Vec<_> = (0..2)
            .map(|_| {
                let running = connection.spawn(request(), sink.clone(), None).unwrap();
                assert!(matches!(recv(&core), ToCore::Spawn(_)));
                running
            })
            .collect();
        let ids: Vec<_> = running.iter().map(|r| r.id()).collect();
        // Each closes once, however often it's asked.
        for session in &running {
            session.close();
            session.close();
        }
        let mut closed: Vec<_> = (0..2)
            .map(|_| match recv(&core) {
                ToCore::Close(close) => close.session,
                other => panic!("not CLOSE: {other:?}"),
            })
            .collect();
        closed.sort_unstable();
        assert_eq!(closed, ids);
        assert_eq!(settled(&connection, &core), []);
    }

    #[test]
    fn a_slow_core_never_blocks_the_start_and_queued_input_arrives() {
        let dir = temp_dir("slow");
        let (core, welcome) = slow_core(&dir);
        let endpoint = Endpoint::at(dir.join("unused"), dir.to_path_buf());
        let (tx, seen) = mpsc::channel();
        let session = Session::start_with(
            move || {
                let connected = connect(&endpoint, "tok")?;
                Connection::start(endpoint, "tok", connected)
            },
            request(),
            Box::new(Recorder(tx)),
        );
        session.resize(100, 30, 9, 18);
        session.input(b"echo one\r");
        session.input(b"echo two\r");
        // The core holds its WELCOME until after all of that returned: the
        // start blocked on nothing, and nothing runs yet.
        assert_eq!(session.pid(), None);
        welcome.send(()).unwrap();

        let next = || recv(&core);
        let ToCore::Spawn(spawn) = next() else { panic!("not SPAWN") };
        assert_eq!(spawn.command.as_deref(), Some("sh"));
        let ToCore::Resize(resize) = next() else { panic!("not RESIZE") };
        assert_eq!((resize.session, resize.cols, resize.rows), (7, 100, 30));
        let ToCore::Data(one) = next() else { panic!("not DATA") };
        let ToCore::Data(two) = next() else { panic!("not DATA") };
        assert_eq!((one.session, one.bytes.as_slice()), (7, &b"echo one\r"[..]));
        assert_eq!(two.bytes, b"echo two\r");
        assert_eq!(session.pid(), Some(4242));
        assert_eq!(seen.recv_timeout(limit(Duration::from_secs(5))).unwrap(), "output $ ");

        // Running now: input goes straight out.
        session.input(b"x");
        let ToCore::Data(x) = next() else { panic!("not DATA") };
        assert_eq!(x.bytes, b"x");
        session.close();
        assert!(matches!(next(), ToCore::Close(close) if close.session == 7));
    }

    /// What a spawn asks for is said on the session's thread, once it is
    /// connected, and never on the thread that opened the tab: so whatever
    /// saying it looks at on this machine holds nothing else up.
    #[test]
    fn a_spawns_request_is_said_on_the_sessions_thread_once_it_is_connected() {
        let dir = temp_dir("request-thread");
        let (core, welcome) = slow_core(&dir);
        let endpoint = Endpoint::at(dir.join("unused"), dir.to_path_buf());
        let (tx, _seen) = mpsc::channel();
        let (said_on, said) = mpsc::channel();
        let session = Session::start_deciding(
            move || {
                let connected = connect(&endpoint, "tok")?;
                Connection::start(endpoint, "tok", connected)
            },
            move || {
                said_on.send(std::thread::current().id()).unwrap();
                SpawnRequest { cwd: Some("/checked/there".into()), ..request() }
            },
            Box::new(Recorder(tx)),
        );
        // The core holds its WELCOME, so the session isn't connected:
        // nothing was said yet, here or there.
        assert!(said.try_recv().is_err());
        welcome.send(()).unwrap();

        let ToCore::Spawn(spawn) = recv(&core) else { panic!("not SPAWN") };
        assert_eq!(spawn.cwd.as_deref(), Some("/checked/there"));
        let thread = said.recv_timeout(limit(Duration::from_secs(10))).unwrap();
        assert_ne!(thread, std::thread::current().id());
        session.close();
    }

    /// 6.1: a session keeps the last `PROCESS` for it, from the one the
    /// core sends right behind `SPAWNED`; only a new name tells the tab.
    /// `PROCESS` for another session is no one's, and an exit clears it.
    #[test]
    fn a_session_keeps_the_last_process_its_core_reported() {
        let dir = temp_dir("process");
        let process = |session: SessionId, name: &str, cwd: &str| {
            ToClient::Process(client::Process { session, name: Some(name.into()), cwd: Some(cwd.into()) })
        };
        let core = scripted_core(&dir, move |message, stream| match message {
            ToCore::Spawn(spawn) => {
                send(stream, ToClient::Spawned(Spawned { req: spawn.req, session: 7, pid: 4242, key: "k7".into() }));
                send(stream, process(7, "bash", "/home/u"));
                send(stream, process(8, "other", "/elsewhere"));
            }
            ToCore::Data(data) if data.bytes == b"cd /tmp\r" => send(stream, process(7, "bash", "/tmp")),
            ToCore::Data(data) if data.bytes == b"htop\r" => send(stream, process(7, "htop", "/tmp")),
            ToCore::Data(data) if data.bytes == b"exit\r" => {
                send(stream, ToClient::Exit(client::Exit { session: 7, code: Some(0), lost: None }))
            }
            _ => {}
        });
        let (tx, tab) = mpsc::channel();
        let session = Session::start_with(connector(&dir), request(), Box::new(Recorder(tx)));
        assert_eq!(session.foreground(), None, "nothing before the session runs");
        assert!(matches!(recv(&core), ToCore::Spawn(_)));
        assert_eq!(seen(&tab), "foreground");
        let reported = |name: &str, cwd: &str| Some(Foreground { name: Some(name.into()), cwd: Some(cwd.into()) });
        eventually(|| session.foreground() == reported("bash", "/home/u"));

        // A new directory with the same program changes no title.
        session.input(b"cd /tmp\r");
        recv(&core);
        eventually(|| session.foreground() == reported("bash", "/tmp"));
        session.input(b"htop\r");
        recv(&core);
        assert_eq!(seen(&tab), "foreground");
        assert_eq!(session.foreground(), reported("htop", "/tmp"));

        session.input(b"exit\r");
        recv(&core);
        assert_eq!(seen(&tab), "exited Some(0)");
        assert_eq!(session.foreground(), None);
    }

    /// A session starting on a core that answers `SPAWN` with session 7,
    /// held in `Starting` until `release` is sent to. The connection is
    /// the one it starts on, for [`settled`].
    fn held_start(tag: &str) -> (TempDir, Arc<Connection>, mpsc::Receiver<ToCore>, Session, mpsc::Sender<()>) {
        let dir = temp_dir(tag);
        let core = scripted_core(&dir, |message, stream| {
            if let ToCore::Spawn(spawn) = message {
                send(stream, ToClient::Spawned(Spawned { req: spawn.req, session: 7, pid: 4242, key: "k7".into() }));
            }
        });
        let (release, wait) = mpsc::channel::<()>();
        let connection = connected(&dir);
        let starting = connection.clone();
        let (tx, _tab) = mpsc::channel();
        let session = Session::start_with(
            move || {
                wait.recv().ok();
                Ok(starting)
            },
            request(),
            Box::new(Recorder(tx)),
        );
        (dir, connection, core, session, release)
    }

    /// The next `total` bytes of input the core gets, in order.
    fn input_received(core: &mpsc::Receiver<ToCore>, total: usize) -> Vec<u8> {
        let mut bytes = Vec::new();
        while bytes.len() < total {
            let ToCore::Data(data) = recv(core) else { panic!("not DATA") };
            bytes.extend(data.bytes);
        }
        bytes
    }

    /// 5.1: input held while the session starts is counted once: the
    /// handoff sends it without offering it again, so once it is written
    /// nothing waits.
    #[test]
    fn input_held_while_starting_counts_once_across_the_handoff() {
        let (_dir, _connection, core, session, release) = held_start("budget-handoff");
        let held = vec![b'a'; 3 << 20];
        assert_eq!(session.input(&held), Delivery::Queued);
        assert_eq!(session.waiting(), held.len());
        release.send(()).unwrap();
        assert!(matches!(recv(&core), ToCore::Spawn(_)));
        assert_eq!(input_received(&core, held.len()), held);
        eventually(|| session.waiting() == 0);
        session.close();
    }

    /// 5.1: input while the session starts is refused past the high mark,
    /// and only what was taken goes out once it has started.
    #[test]
    fn input_while_starting_is_refused_past_the_high_mark() {
        let (_dir, connection, core, session, release) = held_start("budget-starting");
        let held = vec![b'a'; 4 << 20];
        assert_eq!(session.input(&held), Delivery::Queued);
        assert_eq!(session.input(b"x"), Delivery::Refused(Refusal::First));
        assert_eq!(session.input(b"y"), Delivery::Refused(Refusal::Again));
        release.send(()).unwrap();
        assert!(matches!(recv(&core), ToCore::Spawn(_)));
        assert_eq!(input_received(&core, held.len()), held);
        assert_eq!(settled(&connection, &core), [], "nothing refused is written");
        session.close();
    }

    /// A running session is handed its size, then its window, then the
    /// input as typed; whatever wasn't asked for is left out.
    #[test]
    fn a_started_session_is_handed_its_size_then_its_window_then_its_input() {
        let size = Size { cols: 120, rows: 40, cell_w: 9, cell_h: 18 };
        let input = vec![b"one".to_vec(), b"two".to_vec()];
        let all = Queued { input: input.clone(), resize: Some(size), parent: Some(0x1234) };
        assert_eq!(all.held(), 6);
        assert_eq!(
            all.into_ops(),
            [Op::Resize(size), Op::Reparent(0x1234), Op::Input(b"one".to_vec()), Op::Input(b"two".to_vec())]
        );
        let input_only = Queued { input, ..Queued::default() };
        assert_eq!(input_only.into_ops(), [Op::Input(b"one".to_vec()), Op::Input(b"two".to_vec())]);
        assert_eq!(Queued { parent: Some(7), ..Queued::default() }.into_ops(), [Op::Reparent(7)]);
        assert_eq!(Queued::default().into_ops(), []);
    }

    /// A session let go while it starts is left as the tab said, its held
    /// input freed and its request withdrawn; letting go again changes how
    /// and frees nothing more; a session gone already needs nothing.
    #[test]
    fn a_session_let_go_while_starting_is_left_and_frees_its_input() {
        let mut phase = Phase::Starting(Queued { input: vec![vec![b'a'; 1024], b"x".to_vec()], ..Queued::default() });
        assert!(matches!(phase.leave(Leaving::Keep), Departure::Withdraw { held: 1025 }));
        assert!(matches!(phase, Phase::Left(Leaving::Keep)));
        assert!(matches!(phase.leave(Leaving::End), Departure::Withdraw { held: 0 }));
        assert!(matches!(phase, Phase::Left(Leaving::End)));

        let mut gone = Phase::Gone;
        assert!(matches!(gone.leave(Leaving::End), Departure::Nothing));
        assert!(matches!(gone, Phase::Gone));
    }

    /// What the tab did while its session started goes out once it has, in
    /// this order: the last size, the window, then the input in the order
    /// it was typed.
    #[test]
    fn what_was_queued_while_starting_goes_out_size_then_window_then_input() {
        let (_dir, connection, core, session, release) = held_start("handover-order");
        session.input(b"one");
        session.reparent(0x1234);
        session.resize(100, 30, 9, 18);
        session.input(b"two");
        session.resize(120, 40, 9, 18);
        release.send(()).unwrap();
        assert!(matches!(recv(&core), ToCore::Spawn(_)));
        let ToCore::Resize(resize) = recv(&core) else { panic!("not RESIZE") };
        assert_eq!((resize.session, resize.cols, resize.rows, resize.cell_w, resize.cell_h), (7, 120, 40, 9, 18));
        assert_eq!(recv(&core), ToCore::Reparent(client::Reparent { session: 7, hwnd: 0x1234 }));
        assert_eq!(input_received(&core, 6), b"onetwo");
        assert_eq!(settled(&connection, &core), []);
        session.close();
    }

    /// A tab closed while its session starts frees the input it held.
    #[test]
    fn input_held_by_a_tab_closed_while_starting_is_freed() {
        let (_dir, _connection, _core, session, release) = held_start("budget-left");
        assert_eq!(session.input(&[b'a'; 1024]), Delivery::Queued);
        session.close();
        assert_eq!(session.waiting(), 0);
        assert_eq!(session.input(b"late"), Delivery::Discarded);
        drop(release);
    }

    #[test]
    fn a_failure_goes_to_the_tab() {
        let (tx, seen) = mpsc::channel();
        let session = Session::start_with(
            || Err(anyhow::anyhow!("terminal server helper not found")),
            request(),
            Box::new(Recorder(tx)),
        );
        let seen = seen.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(seen, "failed terminal server helper not found");
        // Input to a session that never started goes nowhere.
        session.input(b"x");
        session.close();
        assert_eq!(session.pid(), None);
    }

    /// A tab closed while its attach waits for a relay that isn't up yet
    /// withdraws the attach: it never goes out, and the tab says nothing.
    #[test]
    fn a_tab_closed_while_its_attach_waits_for_a_relay_withdraws_it() {
        use crate::remote::local::connection::fake;

        let (dial, plan) = fake::dialer("A8");
        let connection = Connection::relay(dial, "tok", fake::quick()).unwrap();
        plan.next_dial();
        let (tx, tab) = mpsc::channel();
        let attaching = connection.clone();
        let session = Session::attach_with(move || Ok(attaching), attach_request(5), Box::new(Recorder(tx)));
        session.input(b"x");
        // The request reaches the connection before the tab closes.
        eventually(|| connection.requests_waiting() == 1);
        session.detach_with(None);
        let (link, far) = fake::link(&[], |message, out| {
            if let ToCore::List(list) = message {
                out.send(ToClient::Sessions(client::Sessions { req: list.req, sessions: Vec::new() }));
            }
        });
        plan.link(link);
        assert!(connection.list().unwrap().is_empty());
        assert!(matches!(far.next(), ToCore::List(_)), "no ATTACH before the LIST");
        // A round trip the far end answers itself: nothing went out before
        // its LIST, and the tab heard nothing before its answer.
        let round_trip = far.round_trip(&connection);
        assert!(
            matches!(far.next(), ToCore::List(list) if list.req == round_trip),
            "nothing before the round trip's LIST"
        );
        assert!(tab.try_recv().is_err(), "the closed tab says nothing");
        assert_eq!(session.pid(), None);
    }

    /// 7.1: until its attach is answered, a session reports the one it
    /// asked for, so a quit while its endpoint is offline records the zone
    /// again; a pending spawn reports none, and neither does an attach
    /// that failed.
    #[test]
    fn a_pending_attach_reports_the_requested_id_and_key() {
        let (release, wait) = mpsc::channel::<()>();
        let (tx, tab) = mpsc::channel();
        let session = Session::attach_with(
            move || {
                wait.recv().ok();
                anyhow::bail!("can't connect to A8: offline")
            },
            AttachRequest { when: Attaching::Recorded, ..attach_request(5) },
            Box::new(Recorder(tx)),
        );
        assert_eq!(session.id_and_key(), Some((5, SessionKey::from_wire("k5"))));
        assert_eq!(session.pid(), None);

        let (spawn_tx, _spawn_tab) = mpsc::channel();
        let (_hold, held) = mpsc::channel::<()>();
        let spawning = Session::start_with(
            move || {
                held.recv().ok();
                anyhow::bail!("unused")
            },
            request(),
            Box::new(Recorder(spawn_tx)),
        );
        assert_eq!(spawning.id_and_key(), None);

        release.send(()).unwrap();
        assert_eq!(seen(&tab), "failed can't connect to A8: offline");
        assert_eq!(session.id_and_key(), None);
    }

    #[test]
    fn a_tab_closed_while_connecting_starts_no_session() {
        let (tx, seen) = mpsc::channel();
        let (release, wait) = mpsc::channel::<()>();
        let (ran_tx, ran) = mpsc::channel();
        let session = Session::start_with(
            move || {
                wait.recv().ok();
                ran_tx.send(()).ok();
                anyhow::bail!("unused: the connection is never asked to spawn")
            },
            request(),
            Box::new(Recorder(tx)),
        );
        session.input(b"x");
        session.close();
        release.send(()).unwrap();
        ran.recv_timeout(limit(Duration::from_secs(5))).unwrap();
        // Closed tabs say nothing, even about a failure: the session's
        // thread drops the tab's sink having told it nothing.
        let heard = seen.recv_timeout(limit(Duration::from_secs(10)));
        assert_eq!(heard, Err(mpsc::RecvTimeoutError::Disconnected));
    }

    // Agent sessions (`agents-profiles` group 4), against scripted protocol
    // 4 cores. The core's own decisions (the queue slot, holds, resends)
    // are `remote-agent`'s `agents.rs` tests; these check what the client
    // sends, when, and where the answers and the transcript go.

    /// The agent session a scripted core starts.
    const AGENT: SessionId = 7;

    fn agent_request() -> AgentSpawnRequest {
        AgentSpawnRequest {
            command: "fake-agent".into(),
            args: vec!["--acp".into()],
            cwd: Some("/work".into()),
            env: BTreeMap::from([("AGENT_KEY".to_string(), "1".to_string())]),
            profile: "Claude".into(),
            conversation: None,
            start: None,
        }
    }

    fn transcript(stream: &mut Stream, events: &[Event]) {
        let events = events.iter().map(Event::to_value).collect();
        send(stream, ToClient::Transcript(client::Transcript { session: AGENT, events }));
    }

    fn values(stream: &mut Stream, events: Vec<serde_json::Value>) {
        send(stream, ToClient::Transcript(client::Transcript { session: AGENT, events }));
    }

    fn ok(stream: &mut Stream, req: RequestId) {
        send(stream, ToClient::Ok(client::Request { req }));
    }

    fn refuse(stream: &mut Stream, req: RequestId, message: &str) {
        let error = client::ErrorMessage { req: Some(req), session: Some(AGENT), message: message.into() };
        send(stream, ToClient::Error(error));
    }

    /// A scripted core's stand-in for one agent session's turn and queue
    /// slot, answering as the core does: a prompt with no turn running
    /// starts one ("hi" ends at once, "ask" waits on a permission request,
    /// any other runs until cancelled), one during a turn waits in the
    /// slot, and one more is refused; a cancel holds the waiting prompt; a
    /// resend sends the held prompt when idle and queues it during a turn.
    #[derive(Default)]
    struct MiniAgent {
        running: bool,
        slot: Slot,
        next: u64,
    }

    impl MiniAgent {
        fn added(&mut self, entry: Entry) -> Event {
            self.next += 1;
            Event::Added { id: EntryId(self.next), entry }
        }

        fn start_turn(&mut self, text: &str) -> Vec<Event> {
            self.running = true;
            let mut events = vec![self.added(Entry::User { text: text.into() }), Event::Turn { state: TurnState::Working }];
            match text {
                "hi" => {
                    events.push(self.added(Entry::Agent { text: "hello back".into() }));
                    events.push(Event::Turn { state: TurnState::Ended { stop: StopReason::EndTurn } });
                    self.running = false;
                }
                "ask" => {
                    let title = "Write a file".to_string();
                    let permission = Entry::Permission { call: Some("call_1".into()), title, options: Vec::new(), outcome: None };
                    events.push(self.added(permission));
                    events.push(Event::Turn { state: TurnState::Blocked });
                }
                _ => {}
            }
            events
        }

        fn set_slot(&mut self, slot: Slot) -> Event {
            self.slot = slot.clone();
            Event::Queue { slot }
        }

        fn answer(&mut self, message: &ToCore, stream: &mut Stream) {
            match message {
                ToCore::SpawnAgent(spawn) => {
                    send(stream, ToClient::Spawned(Spawned { req: spawn.req, session: AGENT, pid: 4242, key: "k7".into() }));
                    let ready = AgentState::Ready { name: Some("fake".into()), version: Some("1.0".into()) };
                    transcript(stream, &[Event::Agent { state: ready }]);
                }
                ToCore::AgentPrompt(prompt) if !self.running => {
                    ok(stream, prompt.req);
                    let events = self.start_turn(&prompt.text);
                    transcript(stream, &events);
                }
                ToCore::AgentPrompt(prompt) if self.slot == Slot::Empty => {
                    ok(stream, prompt.req);
                    let event = self.set_slot(Slot::Waiting { text: prompt.text.clone() });
                    transcript(stream, &[event]);
                }
                ToCore::AgentPrompt(prompt) => refuse(stream, prompt.req, "a prompt is already queued"),
                ToCore::AgentCancel(_) if self.running => {
                    self.running = false;
                    let mut events = vec![Event::Turn { state: TurnState::Ended { stop: StopReason::Cancelled } }];
                    if let Slot::Waiting { text } = self.slot.clone() {
                        events.push(self.set_slot(Slot::Held { text, reason: HoldReason::Cancelled }));
                    }
                    transcript(stream, &events);
                }
                ToCore::AgentResend(request) => match self.slot.clone() {
                    Slot::Held { text, .. } if self.running => {
                        ok(stream, request.req);
                        let event = self.set_slot(Slot::Waiting { text });
                        transcript(stream, &[event]);
                    }
                    Slot::Held { text, .. } => {
                        ok(stream, request.req);
                        let mut events = vec![self.set_slot(Slot::Empty)];
                        events.extend(self.start_turn(&text));
                        transcript(stream, &events);
                    }
                    Slot::Empty | Slot::Waiting { .. } => refuse(stream, request.req, "no prompt is held"),
                },
                ToCore::AgentUnqueue(request) => {
                    ok(stream, request.req);
                    let event = self.set_slot(Slot::Empty);
                    transcript(stream, &[event]);
                }
                ToCore::AgentPermission(answer) => {
                    ok(stream, answer.req);
                    let outcome = answer.option.clone().map_or(PermissionOutcome::Cancelled, |option| {
                        PermissionOutcome::Selected { option }
                    });
                    let answered = Event::Answered { id: EntryId(answer.entry), outcome };
                    transcript(stream, &[answered, Event::Turn { state: TurnState::Working }]);
                }
                _ => {}
            }
        }
    }

    /// Takes the feed's events until `done` holds for the shared fold;
    /// gives the life events (all but `Changed`) taken on the way.
    fn shows(feed: &mut Feed, what: &str, mut done: impl FnMut(&Transcript) -> bool) -> Vec<AgentEvent> {
        let mut life = Vec::new();
        until(what, Duration::from_secs(10), || {
            while let Ok(Some(event)) = feed.events.try_next() {
                feed.took([&event]);
                if event != AgentEvent::Changed {
                    life.push(event);
                }
            }
            done(feed.shared.lock().transcript()).then_some(())
        });
        life
    }

    /// The next life event (`Changed` skipped).
    fn next_life(feed: &mut Feed) -> AgentEvent {
        until("an agent life event", Duration::from_secs(10), || {
            while let Ok(Some(event)) = feed.events.try_next() {
                feed.took([&event]);
                if event != AgentEvent::Changed {
                    return Some(event);
                }
            }
            None
        })
    }

    /// The log's user and agent entries, as `user: …` and `agent: …`.
    fn says(log: &Transcript) -> Vec<String> {
        log.entries()
            .filter_map(|(_, entry)| match entry {
                Entry::User { text } => Some(format!("user: {text}")),
                Entry::Agent { text } => Some(format!("agent: {text}")),
                Entry::Thought { .. } | Entry::Tool { .. } | Entry::Permission { .. } | Entry::Notice { .. } => None,
            })
            .collect()
    }

    fn reply(request: RequestId) -> AgentEvent {
        AgentEvent::Reply { request, outcome: Ok(()) }
    }

    /// What a test drives: the session, its zone's end, the connection and
    /// what the core got.
    struct Agent {
        _dir: TempDir,
        session: Session,
        feed: Feed,
        connection: Arc<Connection>,
        core: mpsc::Receiver<ToCore>,
    }

    /// An agent session started on a scripted protocol 4 core answering
    /// as `answer` does, once it runs and its agent is ready. Checks the
    /// `SPAWN_AGENT` it sent.
    fn agent_session_with(
        tag: &str,
        fault: Option<Vec<u8>>,
        answer: impl FnMut(&ToCore, &mut Stream) + Send + 'static,
    ) -> Agent {
        let dir = temp_dir(tag);
        let core = scripted_core_at(&dir, 4, answer);
        let connection = connected(&dir);
        let starting = connection.clone();
        let (sink, mut feed) = AgentSink::with_fault(fault);
        let session = Session::start_agent_with(move || Ok(starting), agent_request(), Box::new(sink));
        let ToCore::SpawnAgent(spawn) = recv(&core) else { panic!("not SPAWN_AGENT") };
        let expected = client::SpawnAgent {
            req: spawn.req,
            command: "fake-agent".into(),
            args: vec!["--acp".into()],
            cwd: Some("/work".into()),
            env: BTreeMap::from([("AGENT_KEY".to_string(), "1".to_string())]),
            policy: client::Policy::Detach,
            profile: Some("Claude".into()),
            conversation: None,
            start: None,
        };
        assert_eq!(spawn, expected);
        let life = shows(&mut feed, "the agent ready", |log| matches!(log.agent(), AgentState::Ready { .. }));
        match life.as_slice() {
            [] => assert_eq!(next_life(&mut feed), AgentEvent::SessionRunning),
            [AgentEvent::SessionRunning] => {}
            other => panic!("unexpected life events: {other:?}"),
        }
        assert_eq!(session.id_and_key(), Some((AGENT, SessionKey::from_wire("k7"))));
        Agent { _dir: dir, session, feed, connection, core }
    }

    fn agent_session(tag: &str) -> Agent {
        let mut agent = MiniAgent::default();
        agent_session_with(tag, None, move |message, stream| agent.answer(message, stream))
    }

    /// 4.1 and 4.2: a protocol 4 core is sent `SPAWN_AGENT` with the
    /// request, and a prompt; the prompt's `OK` comes back under its id and
    /// the turn's transcript is folded.
    #[test]
    fn an_agent_session_starts_and_runs_a_turn() {
        let mut agent = agent_session("agent-turn");
        let req = agent.session.prompt("hi").unwrap();
        assert_eq!(recv(&agent.core), ToCore::AgentPrompt(client::AgentPrompt { req, session: AGENT, text: "hi".into() }));
        let ended = |log: &Transcript| *log.turn() == TurnState::Ended { stop: StopReason::EndTurn };
        assert_eq!(shows(&mut agent.feed, "the turn's end", ended), [reply(req)]);
        assert_eq!(says(agent.feed.shared.lock().transcript()), ["user: hi", "agent: hello back"]);
        agent.session.close();
        assert_eq!(recv(&agent.core), ToCore::Close(client::SessionRef { session: AGENT }));
    }

    /// `agents-session-recovery` 4.2: an ended agent's restart goes out as
    /// `AGENT_RESTART` on its session, with the restart's command,
    /// arguments and environment; its `OK` and a refusal come back under
    /// its id, and the continue's events are folded into the same log.
    #[test]
    fn a_restart_goes_out_as_agent_restart_and_its_answers_come_back() {
        let mut restarts = 0;
        let mut agent = agent_session_with("agent-restart", None, move |message, stream| match message {
            ToCore::SpawnAgent(spawn) => {
                send(stream, ToClient::Spawned(Spawned { req: spawn.req, session: AGENT, pid: 4242, key: "k7".into() }));
                let ready = AgentState::Ready { name: Some("fake".into()), version: None };
                transcript(stream, &[Event::Agent { state: ready }]);
            }
            // The agent exits on its first prompt; the core keeps the
            // session for a continue (no `EXIT`).
            ToCore::AgentPrompt(prompt) => {
                ok(stream, prompt.req);
                let exited = AgentState::Exited { code: Some(1), stderr: "boom".into() };
                transcript(stream, &[Event::Agent { state: exited }]);
            }
            ToCore::AgentRestart(restart) if restarts == 0 => {
                restarts += 1;
                ok(stream, restart.req);
                let notice = Entry::Notice {
                    level: crate::agent::transcript::NoticeLevel::Info,
                    text: "the agent was started again to continue the conversation".into(),
                };
                let ready = AgentState::Ready { name: Some("fake".into()), version: None };
                transcript(
                    stream,
                    &[
                        Event::Added { id: EntryId(1), entry: notice },
                        Event::Agent { state: AgentState::Starting },
                        Event::Agent { state: ready },
                    ],
                );
            }
            ToCore::AgentRestart(restart) => refuse(stream, restart.req, "session 7: the agent hasn't ended"),
            _ => {}
        });
        let prompt = agent.session.prompt("crash").unwrap();
        recv(&agent.core);
        let exited = |log: &Transcript| matches!(log.agent(), AgentState::Exited { .. });
        assert_eq!(shows(&mut agent.feed, "the agent exited", exited), [reply(prompt)]);
        let request = AgentRestartRequest {
            command: "fake-agent".into(),
            args: vec!["--second".into()],
            env: BTreeMap::from([("AGENT_KEY".to_string(), "2".to_string())]),
        };
        let req = agent.session.restart(request.clone()).unwrap();
        let sent = client::AgentRestart {
            req,
            session: AGENT,
            command: "fake-agent".into(),
            args: vec!["--second".into()],
            env: BTreeMap::from([("AGENT_KEY".to_string(), "2".to_string())]),
        };
        assert_eq!(recv(&agent.core), ToCore::AgentRestart(sent));
        let ready = |log: &Transcript| matches!(log.agent(), AgentState::Ready { .. }) && log.entries().count() == 1;
        assert_eq!(shows(&mut agent.feed, "the continued agent ready", ready), [reply(req)]);
        let again = agent.session.restart(request).unwrap();
        let refused = AgentEvent::Reply { request: again, outcome: Err("session 7: the agent hasn't ended".into()) };
        assert_eq!(next_life(&mut agent.feed), refused);
    }

    /// `agents-session-list` 4.1: a list goes out as `AGENT_LIST` with its
    /// cursor and its `AGENT_CONVERSATIONS` comes back as the zone's page;
    /// a choice goes out as `AGENT_CHOOSE`, its refusal under its id.
    #[test]
    fn a_list_and_a_choice_go_out_and_their_answers_come_back() {
        let mut agent = agent_session_with("agent-list", None, move |message, stream| match message {
            ToCore::SpawnAgent(spawn) => {
                send(stream, ToClient::Spawned(Spawned { req: spawn.req, session: AGENT, pid: 4242, key: "k7".into() }));
                let ready = AgentState::Ready { name: Some("fake".into()), version: None };
                transcript(stream, &[Event::Agent { state: ready }]);
            }
            ToCore::AgentList(list) => {
                let conversations = vec![client::ListedConversation { id: "s1".into(), title: None, updated: None }];
                let listed = client::AgentConversations { req: list.req, session: AGENT, conversations, next: None };
                send(stream, ToClient::AgentConversations(listed));
            }
            ToCore::AgentChoose(choose) => refuse(stream, choose.req, "session 7: the agent isn't choosing"),
            _ => {}
        });
        let req = agent.session.list(Some("c1".into())).unwrap();
        let sent = client::AgentList { req, session: AGENT, cursor: Some("c1".into()) };
        assert_eq!(recv(&agent.core), ToCore::AgentList(sent));
        let page = crate::agent::conversations::Page {
            conversations: vec![crate::agent::conversations::Listed { id: "s1".into(), title: None, updated: None }],
            next: None,
        };
        assert_eq!(next_life(&mut agent.feed), AgentEvent::Conversations { request: req, page });
        let chose = agent.session.choose(Some("s1".into())).unwrap();
        let sent = client::AgentChoose { req: chose, session: AGENT, conversation: Some("s1".into()) };
        assert_eq!(recv(&agent.core), ToCore::AgentChoose(sent));
        let refused = AgentEvent::Reply { request: chose, outcome: Err("session 7: the agent isn't choosing".into()) };
        assert_eq!(next_life(&mut agent.feed), refused);
    }

    /// `agents-commands` 4.1: a set goes out as `AGENT_SET_OPTION` naming
    /// the option and the value, its `OK` and its refusal coming back under
    /// its id.
    #[test]
    fn a_set_option_goes_out_and_its_answer_comes_back() {
        use client::OptionValue;
        let mut agent = agent_session_with("agent-set-option", None, move |message, stream| match message {
            ToCore::SpawnAgent(spawn) => {
                send(stream, ToClient::Spawned(Spawned { req: spawn.req, session: AGENT, pid: 4242, key: "k7".into() }));
                let ready = AgentState::Ready { name: Some("fake".into()), version: None };
                transcript(stream, &[Event::Agent { state: ready }]);
            }
            ToCore::AgentSetOption(set) if set.option == "mode" => ok(stream, set.req),
            ToCore::AgentSetOption(set) => refuse(stream, set.req, "session 7: the agent offers no option \"nope\""),
            _ => {}
        });
        let choice = OptionValue::Choice { value: "plan".into() };
        let req = agent.session.set_option("mode", choice.clone()).unwrap();
        let sent = client::AgentSetOption { req, session: AGENT, option: "mode".into(), value: choice };
        assert_eq!(recv(&agent.core), ToCore::AgentSetOption(sent));
        assert_eq!(next_life(&mut agent.feed), reply(req));
        let toggle = OptionValue::Toggle { on: true };
        let refused = agent.session.set_option("nope", toggle.clone()).unwrap();
        let sent = client::AgentSetOption { req: refused, session: AGENT, option: "nope".into(), value: toggle };
        assert_eq!(recv(&agent.core), ToCore::AgentSetOption(sent));
        let outcome = Err("session 7: the agent offers no option \"nope\"".into());
        assert_eq!(next_life(&mut agent.feed), AgentEvent::Reply { request: refused, outcome });
    }

    /// `agents-login` 7.1: a login choice goes out as `AGENT_LOGIN` on the
    /// session, naming the method; its `OK` and a refusal come back under
    /// its id, and the login's events are folded into the same log.
    #[test]
    fn a_login_goes_out_as_agent_login_and_its_answers_come_back() {
        let mut logins = 0;
        let mut agent = agent_session_with("agent-login", None, move |message, stream| match message {
            ToCore::SpawnAgent(spawn) => {
                // The scripted core takes the login whatever the agent's
                // state; the fold of `LoginRequired` is the transcript's.
                send(stream, ToClient::Spawned(Spawned { req: spawn.req, session: AGENT, pid: 4242, key: "k7".into() }));
                let ready = AgentState::Ready { name: Some("fake".into()), version: None };
                transcript(stream, &[Event::Agent { state: ready }]);
            }
            ToCore::AgentLogin(login) if logins == 0 => {
                logins += 1;
                ok(stream, login.req);
                let logging_in = AgentState::LoggingIn { method: login.method.clone(), name: "Log in with Google".into() };
                transcript(stream, &[Event::Agent { state: logging_in }]);
            }
            ToCore::AgentLogin(login) => refuse(stream, login.req, "session 7: the agent isn't waiting for a login"),
            _ => {}
        });
        let req = agent.session.login("google").unwrap();
        assert_eq!(
            recv(&agent.core),
            ToCore::AgentLogin(client::AgentLogin { req, session: AGENT, method: "google".into() })
        );
        let logging_in = |log: &Transcript| matches!(log.agent(), AgentState::LoggingIn { method, .. } if method == "google");
        assert_eq!(shows(&mut agent.feed, "the agent logging in", logging_in), [reply(req)]);
        let again = agent.session.login("google").unwrap();
        let refused =
            AgentEvent::Reply { request: again, outcome: Err("session 7: the agent isn't waiting for a login".into()) };
        assert_eq!(next_life(&mut agent.feed), refused);
    }

    /// `agents-session-recovery` 4.2: a continue in a new session is a
    /// `SPAWN_AGENT` naming the conversation, in its directory.
    #[test]
    fn a_continuing_start_sends_the_conversation() {
        let dir = temp_dir("agent-continuing");
        let core = scripted_core_at(&dir, 4, |message, stream| {
            if let ToCore::SpawnAgent(spawn) = message {
                send(stream, ToClient::Spawned(Spawned { req: spawn.req, session: AGENT, pid: 4242, key: "k7".into() }));
            }
        });
        let connection = connected(&dir);
        let starting = connection.clone();
        let conversation = crate::agent::transcript::Conversation {
            id: "sess-1".into(),
            cwd: "/home/u/src".into(),
            continuation: crate::agent::transcript::Continuation::Load,
            title: None,
        };
        let request = agent_request().continuing(&conversation);
        let (sink, _feed) = AgentSink::with_fault(None);
        let session = Session::start_agent_with(move || Ok(starting), request, Box::new(sink));
        let ToCore::SpawnAgent(spawn) = recv(&core) else { panic!("not SPAWN_AGENT") };
        assert_eq!(spawn.cwd.as_deref(), Some("/home/u/src"));
        assert_eq!(
            spawn.conversation,
            Some(client::AgentConversation { id: "sess-1".into(), cwd: "/home/u/src".into() })
        );
        assert_eq!((spawn.command.as_str(), spawn.profile.as_deref()), ("fake-agent", Some("Claude")));
        eventually(|| session.id_and_key() == Some((AGENT, SessionKey::from_wire("k7"))));
    }

    /// 4.2: a prompt during a turn waits in the slot and can be removed;
    /// one more while it waits is refused, the refusal coming back under
    /// that prompt's id.
    #[test]
    fn a_queued_prompt_is_removed_and_a_third_refused() {
        let mut agent = agent_session("agent-queue");
        let first = agent.session.prompt("long").unwrap();
        let working = |log: &Transcript| *log.turn() == TurnState::Working;
        assert_eq!(shows(&mut agent.feed, "the turn", working), [reply(first)]);
        let second = agent.session.prompt("next").unwrap();
        let waiting = |log: &Transcript| *log.slot() == Slot::Waiting { text: "next".into() };
        assert_eq!(shows(&mut agent.feed, "the queued prompt", waiting), [reply(second)]);
        let third = agent.session.prompt("third").unwrap();
        let refused = AgentEvent::Reply { request: third, outcome: Err("a prompt is already queued".into()) };
        assert_eq!(next_life(&mut agent.feed), refused);
        let removed = agent.session.unqueue().unwrap();
        let empty = |log: &Transcript| *log.slot() == Slot::Empty;
        assert_eq!(shows(&mut agent.feed, "the slot emptied", empty), [reply(removed)]);
        let sent: Vec<ToCore> = settled(&agent.connection, &agent.core);
        assert_eq!(
            sent,
            [
                ToCore::AgentPrompt(client::AgentPrompt { req: first, session: AGENT, text: "long".into() }),
                ToCore::AgentPrompt(client::AgentPrompt { req: second, session: AGENT, text: "next".into() }),
                ToCore::AgentPrompt(client::AgentPrompt { req: third, session: AGENT, text: "third".into() }),
                ToCore::AgentUnqueue(client::AgentRequest { req: removed, session: AGENT }),
            ]
        );
    }

    /// 4.2: a cancel (`AGENT_CANCEL`, which nothing answers) holds the
    /// queued prompt; a new prompt goes out beside the held one; a resend
    /// during that turn queues the held prompt, and one when idle sends
    /// it; a held prompt can be discarded.
    #[test]
    fn a_cancel_holds_the_queued_prompt_which_is_resent_or_discarded() {
        let mut agent = agent_session("agent-held");
        let held = |text: &str| Slot::Held { text: text.into(), reason: HoldReason::Cancelled };
        let first = agent.session.prompt("long").unwrap();
        let second = agent.session.prompt("next").unwrap();
        let waiting = |log: &Transcript| *log.slot() == Slot::Waiting { text: "next".into() };
        assert_eq!(shows(&mut agent.feed, "the queued prompt", waiting), [reply(first), reply(second)]);

        assert_eq!(agent.session.cancel(), Ok(()));
        let cancelled = |log: &Transcript| {
            *log.turn() == TurnState::Ended { stop: StopReason::Cancelled } && *log.slot() == held("next")
        };
        assert_eq!(shows(&mut agent.feed, "the prompt held", cancelled), []);

        // A new prompt beside the held one starts a turn; the held one stays.
        let beside = agent.session.prompt("beside").unwrap();
        let working = |log: &Transcript| *log.turn() == TurnState::Working;
        assert_eq!(shows(&mut agent.feed, "the new turn", working), [reply(beside)]);
        assert_eq!(*agent.feed.shared.lock().transcript().slot(), held("next"));

        // Sent again during a turn, it waits for that turn.
        let during = agent.session.resend().unwrap();
        assert_eq!(shows(&mut agent.feed, "the held prompt queued", waiting), [reply(during)]);

        // Held again, and sent again when idle: a turn of its own.
        assert_eq!(agent.session.cancel(), Ok(()));
        let held_again = |log: &Transcript| *log.slot() == held("next");
        shows(&mut agent.feed, "the prompt held again", held_again);
        let idle = agent.session.resend().unwrap();
        let resent = |log: &Transcript| *log.slot() == Slot::Empty && *log.turn() == TurnState::Working;
        assert_eq!(shows(&mut agent.feed, "the held prompt's turn", resent), [reply(idle)]);
        assert_eq!(says(agent.feed.shared.lock().transcript()), ["user: long", "user: beside", "user: next"]);

        // A held prompt discarded.
        let more = agent.session.prompt("more").unwrap();
        let queued = |log: &Transcript| *log.slot() == Slot::Waiting { text: "more".into() };
        assert_eq!(shows(&mut agent.feed, "another queued", queued), [reply(more)]);
        assert_eq!(agent.session.cancel(), Ok(()));
        shows(&mut agent.feed, "it held", |log| *log.slot() == held("more"));
        let discard = agent.session.unqueue().unwrap();
        assert_eq!(shows(&mut agent.feed, "the held prompt discarded", |log| *log.slot() == Slot::Empty), [
            reply(discard)
        ]);

        let sent = settled(&agent.connection, &agent.core);
        let cancels = sent.iter().filter(|message| **message == ToCore::AgentCancel(client::SessionRef { session: AGENT }));
        assert_eq!(cancels.count(), 3);
        let resends: Vec<&ToCore> = sent.iter().filter(|message| matches!(message, ToCore::AgentResend(_))).collect();
        assert_eq!(
            resends,
            [
                &ToCore::AgentResend(client::AgentRequest { req: during, session: AGENT }),
                &ToCore::AgentResend(client::AgentRequest { req: idle, session: AGENT }),
            ]
        );
    }

    /// 4.2: a permission answer names the entry and the option, and its
    /// `OK` comes back under its id.
    #[test]
    fn a_permission_answer_goes_out_and_is_folded() {
        let mut agent = agent_session("agent-permission");
        let asked = agent.session.prompt("ask").unwrap();
        let blocked = |log: &Transcript| *log.turn() == TurnState::Blocked;
        assert_eq!(shows(&mut agent.feed, "the permission request", blocked), [reply(asked)]);
        let entry = 2;
        let answer = agent.session.answer(entry, Some("allow-once".into())).unwrap();
        let answered = |log: &Transcript| {
            log.entries().any(|(id, entry)| {
                id == EntryId(2)
                    && matches!(entry, Entry::Permission { outcome: Some(PermissionOutcome::Selected { option }), .. } if option == "allow-once")
            })
        };
        assert_eq!(shows(&mut agent.feed, "the answer", answered), [reply(answer)]);
        let sent = settled(&agent.connection, &agent.core);
        let permission = client::AgentPermission { req: answer, session: AGENT, entry, option: Some("allow-once".into()) };
        assert_eq!(sent.last(), Some(&ToCore::AgentPermission(permission)));
    }

    /// 4.1 and 4.3 against a real core: a connection offering client
    /// protocol 4 (the offer passed to the dial; Brindle's own stays at 1–3)
    /// starts `brindle-fake-acp-agent`, runs a turn through it, and a panic
    /// in the sink (the fault switch's marker in the agent's answer) breaks
    /// the session, which is detached and still listed. Ignored only
    /// because it needs the built helpers:
    /// `BRINDLE_TEST_SERVER_HELPER=target/debug/brindle-remote
    /// BRINDLE_FAKE_ACP_AGENT=target/debug/brindle-fake-acp-agent
    /// cargo test -p brindle a_real_core_runs -- --ignored`.
    #[test]
    #[ignore = "needs BRINDLE_TEST_SERVER_HELPER and BRINDLE_FAKE_ACP_AGENT, built helpers"]
    fn a_real_core_runs_an_agent_session_and_a_broken_one_stays_listed() {
        let helper = std::path::PathBuf::from(std::env::var_os("BRINDLE_TEST_SERVER_HELPER").expect("the helper"));
        let fake = std::env::var("BRINDLE_FAKE_ACP_AGENT").expect("the fake agent");
        let temp = temp_dir("agent-real");
        let script = temp.join("agent.script");
        let chunk = |text: &str| {
            let update = serde_json::json!({"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text}});
            let message = serde_json::json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s1", "update": update}});
            format!("send {message}")
        };
        let lines = [
            "expect initialize".to_string(),
            r#"respond {"protocolVersion":1,"agentInfo":{"name":"fake","version":"1.0"}}"#.into(),
            "expect session/new".into(),
            r#"respond {"sessionId":"s1"}"#.into(),
            "expect session/prompt".into(),
            chunk("hello back"),
            r#"respond {"stopReason":"end_turn"}"#.into(),
            "expect session/prompt".into(),
            chunk("says BRINDLE-FAULT-REAL"),
            r#"respond {"stopReason":"end_turn"}"#.into(),
        ];
        std::fs::write(&script, lines.join("\n") + "\n").unwrap();

        // Left for the core to make: a Linux core refuses a directory that
        // isn't private.
        let dir = temp.join("server");
        let endpoint = Endpoint::at(helper.clone(), dir.clone());
        let connected = crate::remote::local::connect_offering(&endpoint, "agent-real", client::PROTOCOLS).unwrap();
        assert_eq!(connected.welcome.protocol, 4);
        let connection = Connection::start(endpoint, "agent-real", connected).unwrap();
        let starting = connection.clone();
        let request = AgentSpawnRequest {
            command: fake,
            args: vec![script.display().to_string()],
            cwd: None,
            env: BTreeMap::new(),
            profile: "Fake".into(),
            conversation: None,
            start: None,
        };
        let (sink, mut feed) = AgentSink::with_fault(Some(b"BRINDLE-FAULT-REAL".to_vec()));
        let session = Session::start_agent_with(move || Ok(starting), request, Box::new(sink));
        let ready = |log: &Transcript| matches!(log.agent(), AgentState::Ready { .. });
        shows(&mut feed, "the agent ready", ready);
        let (id, _) = until("the session running", Duration::from_secs(10), || session.id_and_key());

        let req = session.prompt("hi").unwrap();
        let ended = |log: &Transcript| says(log).len() == 2 && *log.turn() == TurnState::Ended { stop: StopReason::EndTurn };
        assert_eq!(shows(&mut feed, "the turn's end", ended), [reply(req)]);
        assert_eq!(says(feed.shared.lock().transcript()), ["user: hi", "agent: hello back"]);
        let listed = connection.list().unwrap();
        let info = listed.iter().find(|info| info.session == id).expect("listed");
        assert_eq!(info.profile.as_deref(), Some("Fake"));

        // The marker in the next answer panics the sink: broken, detached.
        let req = session.prompt("again").unwrap();
        let mut heard = Vec::new();
        let why = until("the session broken", Duration::from_secs(10), || {
            match feed.events.try_next() {
                Ok(Some(AgentEvent::Broken(why))) => return Some(why),
                Ok(Some(event)) => heard.push(event),
                Ok(None) | Err(_) => {}
            }
            None
        });
        assert!(why.contains(crate::terminal::SINK_PANIC), "{why}");
        assert!(heard.contains(&reply(req)), "{heard:?}");
        assert_eq!(session.prompt("more"), Err(NotSent::Gone), "no longer shown");
        let detached = || {
            let listed = connection.list().ok()?;
            listed.into_iter().find(|info| info.session == id).filter(|info| !info.attached)
        };
        until("the session listed, detached", Duration::from_secs(10), detached);

        let stop = crate::remote::hide_window(&mut std::process::Command::new(&helper))
            .arg("stop")
            .env("BRINDLE_SERVER_DIR", &dir)
            .output()
            .unwrap();
        assert!(stop.status.success(), "{stop:?}");
    }

    /// 4.2: a request written when the link drops is answered all the same,
    /// as unanswered (the core may have taken it), and the zone hears the
    /// link went down.
    #[test]
    fn an_agent_request_fails_with_the_link_and_the_zone_hears_it() {
        let mut agent = MiniAgent::default();
        let answer = move |message: &ToCore, stream: &mut Stream| match message {
            ToCore::AgentPrompt(_) => {}
            _ => agent.answer(message, stream),
        };
        let Agent { _dir, session, mut feed, connection, core } = agent_session_with("agent-link", None, answer);
        // The scripted core goes once its report channel is gone: after
        // reading the next message, which it never answers.
        drop(core);
        let req = session.prompt("unanswered").unwrap();
        let AgentEvent::Reply { request, outcome: Err(ReplyError::Unanswered(why)) } = next_life(&mut feed) else {
            panic!("not unanswered")
        };
        assert_eq!(request, req);
        assert!(why.starts_with("the terminal server went away"), "{why}");
        assert_eq!(next_life(&mut feed), AgentEvent::LinkDown);
        drop(connection);
    }

    /// 4.2, `agents-client-state-fixes` 4.3: nothing is sent for a session
    /// that doesn't run yet (still starting) or any more (gone).
    #[test]
    fn agent_requests_go_only_while_the_session_runs() {
        let (release, wait) = mpsc::channel::<()>();
        let (sink, mut feed) = AgentSink::with_fault(None);
        let session = Session::start_agent_with(
            move || {
                wait.recv().ok();
                anyhow::bail!("offline")
            },
            agent_request(),
            Box::new(sink),
        );
        let starting = Err(NotSent::Starting);
        assert_eq!(session.prompt("hi"), starting);
        assert_eq!((session.resend(), session.unqueue(), session.answer(1, None)), (starting, starting, starting));
        assert_eq!((session.fetch(1, 1024), session.list(None), session.choose(None)), (starting, starting, starting));
        assert_eq!(session.cancel(), Err(NotSent::Starting));
        release.send(()).unwrap();
        // A connection that fails is no server.
        assert_eq!(next_life(&mut feed), AgentEvent::Failed(StartFailure::NoServer("offline".into())));
        assert_eq!(session.prompt("hi"), Err(NotSent::Gone));
        assert_eq!(session.cancel(), Err(NotSent::Gone));
    }

    /// `agents-client-state-fixes` 4.3 (D-1): the session's thread says the
    /// session runs only once it does: when the zone takes
    /// `SessionRunning`, a prompt goes, though the agent's `Ready` came
    /// with `SPAWNED`, before the thread installed the session.
    #[test]
    fn the_running_event_comes_once_the_session_can_send() {
        let dir = temp_dir("agent-running");
        let mut mini = MiniAgent::default();
        let core = scripted_core_at(&dir, 4, move |message, stream| mini.answer(message, stream));
        let connection = connected(&dir);
        let starting = connection.clone();
        let (sink, mut feed) = AgentSink::with_fault(None);
        let session = Session::start_agent_with(move || Ok(starting), agent_request(), Box::new(sink));
        assert!(matches!(recv(&core), ToCore::SpawnAgent(_)));
        assert_eq!(next_life(&mut feed), AgentEvent::SessionRunning, "the only life event of a start");
        let req = session.prompt("hi").expect("sent once the session runs");
        assert_eq!(recv(&core), ToCore::AgentPrompt(client::AgentPrompt { req, session: AGENT, text: "hi".into() }));
        drop(connection);
    }

    /// `agents-client-state-fixes` 4.3: an attached session's
    /// `SessionRunning` comes after its replay's start, which the reader
    /// delivers before the attach's answer reaches the session's thread.
    #[test]
    fn an_attached_sessions_running_event_follows_its_replay_start() {
        let dir = temp_dir("agent-attach-running");
        let _core = scripted_core_at(&dir, 4, |message, stream| {
            if let ToCore::Attach(attach) = message {
                let start = client::ReplayStart { req: Some(attach.req), session: AGENT, pid: 4242 };
                send(stream, ToClient::ReplayStart(start));
                send(stream, ToClient::ReplayEnd(client::SessionRef { session: AGENT }));
            }
        });
        let connection = connected(&dir);
        let (sink, mut feed) = AgentSink::with_fault(None);
        let request = AttachRequest { session: AGENT, key: SessionKey::from_wire("k7"), when: Attaching::Now { cols: 0, rows: 0 } };
        let session = Session::attach_with(move || Ok(connection), request, Box::new(sink));
        let mut life = Vec::new();
        while !(life.contains(&AgentEvent::SessionRunning) && life.contains(&AgentEvent::Replay(false))) {
            life.push(next_life(&mut feed));
        }
        let at = |event: &AgentEvent| life.iter().position(|seen| seen == event).expect("taken");
        assert!(at(&AgentEvent::Replay(true)) < at(&AgentEvent::SessionRunning), "{life:?}");
        assert_eq!(life.len(), 3, "{life:?}");
        assert!(session.prompt("hi").is_ok());
    }

    /// 4.1: below client protocol 4 the gate refuses the spawn before it is
    /// sent, and the zone is told the server is too old.
    #[test]
    fn a_protocol_3_core_is_sent_no_agent_session() {
        let dir = temp_dir("agent-protocol-3");
        let core = scripted_core_at(&dir, 3, |_, _| {});
        let connection = connected(&dir);
        let starting = connection.clone();
        let (sink, mut feed) = AgentSink::with_fault(None);
        let session = Session::start_agent_with(move || Ok(starting), agent_request(), Box::new(sink));
        let AgentEvent::Failed(StartFailure::TooOld { endpoint }) = next_life(&mut feed) else {
            panic!("not refused as too old")
        };
        assert_eq!(endpoint, connection.name());
        assert_eq!(settled(&connection, &core), [], "no SPAWN_AGENT went out");
        assert_eq!(session.id_and_key(), None);
    }

    /// 4.3: an attached agent session's replay starts the fold over, and the
    /// live events after it follow.
    #[test]
    fn an_attached_agent_session_replays_its_transcript() {
        let dir = temp_dir("agent-replay");
        let core = scripted_core_at(&dir, 4, |message, stream| {
            if let ToCore::Attach(attach) = message {
                let start = client::ReplayStart { req: Some(attach.req), session: AGENT, pid: 4242 };
                send(stream, ToClient::ReplayStart(start));
                let said = Event::Added { id: EntryId(1), entry: Entry::Agent { text: "earlier".into() } };
                transcript(stream, &[said]);
                send(stream, ToClient::ReplayEnd(client::SessionRef { session: AGENT }));
                let live = Event::Added { id: EntryId(2), entry: Entry::Agent { text: "now".into() } };
                transcript(stream, &[live]);
            }
        });
        let connection = connected(&dir);
        let (sink, mut feed) = AgentSink::with_fault(None);
        let request = AttachRequest { session: AGENT, key: SessionKey::from_wire("k7"), when: Attaching::Now { cols: 0, rows: 0 } };
        let _session = Session::attach_with(move || Ok(connection), request, Box::new(sink));
        let both = |log: &Transcript| log.entries().count() == 2;
        let mut life = shows(&mut feed, "the replay and the live event", both);
        // Its place is `an_attached_sessions_running_event_follows_its_replay_start`'s.
        life.retain(|event| *event != AgentEvent::SessionRunning);
        assert_eq!(life, [AgentEvent::Replay(true), AgentEvent::Replay(false)]);
        assert_eq!(says(feed.shared.lock().transcript()), ["agent: earlier", "agent: now"]);
        let changes = feed.shared.lock().take_changes();
        assert!(changes.reset && changes.status_from == crate::agent::shared::StatusFrom::Replay, "{changes:?}");
        assert!(matches!(recv(&core), ToCore::Attach(_)));
    }

    /// `agents-log-paging` 4.1: an attached session's replay is its newest
    /// page; a fetch's `AGENT_PAGE` frames reach the sink in order and are
    /// folded in front, all before the fetch's reply.
    #[test]
    fn a_fetchs_frames_are_folded_in_front_before_its_reply() {
        let dir = temp_dir("agent-fetch");
        let added = |id: u64, text: &str| Event::Added { id: EntryId(id), entry: Entry::Agent { text: text.into() } };
        let core = scripted_core_at(&dir, 4, move |message, stream| match message {
            ToCore::Attach(attach) => {
                let start = client::ReplayStart { req: Some(attach.req), session: AGENT, pid: 4242 };
                send(stream, ToClient::ReplayStart(start));
                transcript(stream, &[Event::Window { from: EntryId(3), earlier: 2 }, added(3, "three"), added(4, "four")]);
                send(stream, ToClient::ReplayEnd(client::SessionRef { session: AGENT }));
            }
            ToCore::AgentFetch(fetch) => {
                assert_eq!((fetch.session, fetch.before, fetch.max_bytes), (AGENT, 3, 1024));
                let frame = |events: Vec<Event>, last| {
                    let events = events.iter().map(Event::to_value).collect();
                    ToClient::AgentPage(client::AgentPage { req: fetch.req, session: AGENT, events, last })
                };
                send(stream, frame(vec![added(1, "one")], None));
                send(stream, frame(vec![added(2, "two")], Some(client::PageEnd { more: false })));
            }
            _ => {}
        });
        let connection = connected(&dir);
        let (sink, mut feed) = AgentSink::with_fault(None);
        let request = AttachRequest { session: AGENT, key: SessionKey::from_wire("k7"), when: Attaching::Now { cols: 0, rows: 0 } };
        let session = Session::attach_with(move || Ok(connection), request, Box::new(sink));
        shows(&mut feed, "the page", |log| log.entries().count() == 2 && log.window().is_some());
        assert!(matches!(recv(&core), ToCore::Attach(_)));
        feed.shared.lock().take_changes();

        let req = session.fetch(3, 1024).expect("sent while shown");
        assert!(matches!(recv(&core), ToCore::AgentFetch(_)));
        // When the reply comes, the whole page is folded in front.
        let reply = until("the fetch's reply", Duration::from_secs(10), || {
            while let Ok(Some(event)) = feed.events.try_next() {
                feed.took([&event]);
                if let AgentEvent::Reply { request, outcome } = event {
                    return Some((request, outcome, says(feed.shared.lock().transcript())));
                }
            }
            None
        });
        assert_eq!(reply, (req, Ok(()), ["agent: one", "agent: two", "agent: three", "agent: four"].map(String::from).to_vec()));
        let shared = feed.shared.lock();
        assert!(shared.transcript().window().is_none());
        drop(shared);
        let changes = feed.shared.lock().take_changes();
        assert_eq!(changes.prepended, Some(2));
        assert_eq!(changes.status_from, crate::agent::shared::StatusFrom::Unchanged, "a page carries no status");
    }

    /// `agents-log-paging` 4.1: a fetch written when the link drops is
    /// answered with the link's failure.
    #[test]
    fn a_fetch_in_flight_fails_with_the_link() {
        let mut agent = MiniAgent::default();
        let Agent { _dir, session, mut feed, connection, core } =
            agent_session_with("agent-fetch-lost", None, move |message, stream| agent.answer(message, stream));
        // The core goes after reading the fetch, never answering it.
        drop(core);
        let req = session.fetch(1, 1024).unwrap();
        let AgentEvent::Reply { request, outcome: Err(ReplyError::Unanswered(why)) } = next_life(&mut feed) else {
            panic!("not unanswered")
        };
        assert_eq!(request, req);
        assert!(why.starts_with("the terminal server went away"), "{why}");
        assert_eq!(next_life(&mut feed), AgentEvent::LinkDown);
        drop(connection);
    }

    /// `agents-log-paging` 4.1, `agents-client-state-fixes` 4.2: a fetch
    /// while the link is down isn't sent, saying so at once, and no reply
    /// follows.
    #[test]
    fn a_fetch_while_the_link_is_down_fails_at_once() {
        let mut agent = MiniAgent::default();
        let Agent { _dir, session, mut feed, connection, core } =
            agent_session_with("agent-fetch-down", None, move |message, stream| agent.answer(message, stream));
        drop(core);
        // The core goes after reading the next message, never answering it.
        session.prompt("unanswered").unwrap();
        until("the link down", Duration::from_secs(10), || {
            while let Ok(Some(event)) = feed.events.try_next() {
                feed.took([&event]);
                if event == AgentEvent::LinkDown {
                    return Some(());
                }
            }
            None
        });
        assert_eq!(session.fetch(1, 1024), Err(NotSent::LinkDown), "the session is still shown");
        // The old synthetic reply was queued before `fetch` returned.
        let after: Vec<AgentEvent> = std::iter::from_fn(|| feed.events.try_next().ok().flatten())
            .filter(|event| matches!(event, AgentEvent::Reply { .. }))
            .collect();
        assert_eq!(after, [], "no reply follows");
        drop(connection);
    }

    /// 4.3: a transcript the sink can't follow (`bad`, sent in answer to a
    /// prompt) breaks the session: it is detached, never closed, and the
    /// zone is told why; nothing more is sent for it.
    fn broken_by(tag: &str, fault: Option<Vec<u8>>, bad: serde_json::Value) -> String {
        let mut agent = MiniAgent::default();
        let mut agent = agent_session_with(tag, fault, move |message, stream| match message {
            ToCore::AgentPrompt(prompt) if prompt.text == "break" => values(stream, vec![bad.clone()]),
            _ => agent.answer(message, stream),
        });
        agent.session.prompt("break").unwrap();
        assert!(matches!(recv(&agent.core), ToCore::AgentPrompt(_)));
        let AgentEvent::Broken(why) = next_life(&mut agent.feed) else { panic!("not broken") };
        assert_eq!(recv(&agent.core), ToCore::Detach(client::Detach { session: AGENT, layout: None }));
        assert_eq!(agent.session.prompt("again"), Err(NotSent::Gone), "no longer shown");
        assert_eq!(settled(&agent.connection, &agent.core), []);
        why
    }

    #[test]
    fn an_undecodable_event_breaks_the_agent_session() {
        let why = broken_by("agent-undecodable", None, serde_json::json!({"event": "no_such_kind"}));
        assert!(why.starts_with("an event from the terminal server couldn't be read"), "{why}");
    }

    #[test]
    fn an_event_that_does_not_fold_breaks_the_agent_session() {
        let bad = Event::Appended { id: EntryId(99), text: "more".into() }.to_value();
        let why = broken_by("agent-fold", None, bad);
        assert_eq!(why, "an event from the terminal server couldn't be applied: there is no entry 99");
    }

    /// The fault switch (`TestSwitch::SinkPanic`'s marker, given to the
    /// sink as the switch's variable gives it) panics the fold: the
    /// connection's guard catches it and breaks only that session.
    #[test]
    fn a_panic_in_the_agent_sink_breaks_the_agent_session() {
        let marker = b"BRINDLE-FAULT-7".to_vec();
        let bad = Event::Added { id: EntryId(1), entry: Entry::Agent { text: "says BRINDLE-FAULT-7".into() } }.to_value();
        let why = broken_by("agent-panic", Some(marker), bad);
        assert!(why.contains(crate::terminal::SINK_PANIC), "{why}");
    }
}
