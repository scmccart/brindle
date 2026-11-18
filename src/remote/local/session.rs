//! A tab's session in a terminal server, this machine's or a remote
//! endpoint's ([`Host`]), as its terminal holds it from the moment the tab
//! opens.
//!
//! [`Session::start`] and [`Session::attach`] return at once: connecting
//! (starting a core if needed, or reconnecting), an upgrade's step-down and
//! the `SPAWN` or `ATTACH` itself all happen on a thread of their own, so
//! opening a tab never blocks the UI thread. Input and resizes made
//! meanwhile are kept, and go out in order once the core reports the
//! session. A failure goes to the sink, which shows it in the tab. A tab
//! closed while its request waits for a connection withdraws the request.

use std::sync::Arc;

use anyhow::Result;
use parking_lot::Mutex;

use super::connection::{Abandon, Budget, Running, Sink};
use super::{AttachRequest, Connection, Foreground, Host, SessionSink, SpawnRequest};
use crate::remote::SessionKey;
use crate::remote::budget::{Delivery, Offer};
use crate::remote::protocol::SessionId;

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
    Spawn(SpawnRequest),
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
    pub fn start(host: Host, request: SpawnRequest, sink: Box<dyn SessionSink>) -> Self {
        Self::start_with(move || super::shared(&host), request, sink)
    }

    /// Starts a session in the connection `connect` gives, on a thread of
    /// its own.
    pub fn start_with(
        connect: impl FnOnce() -> Result<Arc<Connection>> + Send + 'static,
        request: SpawnRequest,
        sink: Box<dyn SessionSink>,
    ) -> Self {
        Self::begin(connect, Start::Spawn(request), sink)
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
            Start::Spawn(_) => None,
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
    let left = || matches!(&*phase.lock(), Phase::Left(_));
    let started = connect().and_then(|connection| {
        // A tab closed while connecting needs no session: a spawn doesn't
        // start one, and an attach leaves it detached.
        if left() {
            anyhow::bail!("closed before it started");
        }
        match start {
            Start::Spawn(request) => connection.spawn(request, sink.clone(), Some(abandon)),
            Start::Attach(request) => connection.attach(request, sink.clone(), Some(abandon)),
        }
    });
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
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use super::*;
    use crate::remote::local::sys::Listener;
    use crate::remote::local::{Attaching, Endpoint, Stream, connect};
    use crate::remote::protocol::Frame;
    use crate::remote::budget::Refusal;
    use crate::remote::protocol::client::{self, KillTarget, Spawned, ToClient, ToCore, Welcome};
    use crate::test_support::TempDir;
    use crate::test_support::wait::{limit, until};

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
    /// an `-e` one's closed), then the drop, when GPUI runs it, leaves it
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
        far.settle.store(true, std::sync::atomic::Ordering::Release);
        assert!(connection.list().unwrap().is_empty());
        assert!(matches!(far.next(), ToCore::List(_)), "nothing before the round trip's LIST");
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
}
