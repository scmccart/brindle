//! A local tab's session, as its terminal holds it from the moment the tab
//! opens.
//!
//! [`Session::start`] and [`Session::attach`] return at once: connecting
//! (starting a core if needed, or reconnecting), an upgrade's step-down and
//! the `SPAWN` or `ATTACH` itself all happen on a thread of their own, so
//! opening a tab never blocks the UI thread. Input and resizes made
//! meanwhile are kept, and go out in order once the core reports the
//! session. A failure goes to the sink, which shows it in the tab.

use std::sync::Arc;

use anyhow::Result;
use parking_lot::Mutex;

use super::connection::{Running, Sink};
use super::{AttachRequest, Connection, SessionSink, SpawnRequest};
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
    /// The session isn't there yet.
    Starting {
        input: Vec<Vec<u8>>,
        resize: Option<Size>,
        /// The window to parent the session's console to (Windows).
        parent: Option<u64>,
        left: Option<Leaving>,
    },
    Running(Running),
    /// It never started (the sink has said why), or the tab let it go.
    Gone,
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

/// A local tab's session in the terminal server.
pub struct Session {
    phase: Arc<Mutex<Phase>>,
}

impl Session {
    /// Starts a session in the process's shared connection.
    pub fn start(request: SpawnRequest, sink: Box<dyn SessionSink>) -> Self {
        Self::start_with(super::shared, request, sink)
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

    /// Attaches a detached session in the process's shared connection; its
    /// replay and then its live output go to `sink`.
    pub fn attach(request: AttachRequest, sink: Box<dyn SessionSink>) -> Self {
        Self::attach_with(super::shared, request, sink)
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
        let phase = Arc::new(Mutex::new(Phase::Starting { input: Vec::new(), resize: None, parent: None, left: None }));
        let sink: Sink = Arc::new(Mutex::new(sink));
        let starting = phase.clone();
        let failing = sink.clone();
        let spawned = std::thread::Builder::new()
            .name("server-spawn".into())
            .spawn(move || run(connect, start, sink, &starting));
        if let Err(error) = spawned {
            *phase.lock() = Phase::Gone;
            failing.lock().failed(&anyhow::Error::from(error).context("starting a thread"));
        }
        Self { phase }
    }

    /// The program's pid, once the session exists.
    pub fn pid(&self) -> Option<u32> {
        match &*self.phase.lock() {
            Phase::Running(running) => Some(running.pid()),
            _ => None,
        }
    }

    /// The session's id and key, once it exists (the key is empty from a
    /// core that predates keys).
    pub fn id_and_key(&self) -> Option<(SessionId, String)> {
        match &*self.phase.lock() {
            Phase::Running(running) => Some((running.id(), running.key().to_string())),
            _ => None,
        }
    }

    pub fn input(&self, bytes: &[u8]) {
        match &mut *self.phase.lock() {
            Phase::Starting { input, left: None, .. } => input.push(bytes.to_vec()),
            Phase::Running(running) => running.input(bytes),
            Phase::Starting { .. } | Phase::Gone => {}
        }
    }

    pub fn resize(&self, cols: u16, rows: u16, cell_w: u16, cell_h: u16) {
        match &mut *self.phase.lock() {
            Phase::Starting { resize, .. } => *resize = Some(Size { cols, rows, cell_w, cell_h }),
            Phase::Running(running) => running.resize(cols, rows, cell_w, cell_h),
            Phase::Gone => {}
        }
    }

    /// Makes the window `hwnd` the owner of the session's console window
    /// (`REPARENT`), now or as soon as it starts or attaches, and again
    /// after each reconnect's replay. Only a Windows core takes it.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn reparent(&self, hwnd: u64) {
        match &mut *self.phase.lock() {
            Phase::Starting { parent, .. } => *parent = Some(hwnd),
            Phase::Running(running) => running.reparent(hwnd),
            Phase::Gone => {}
        }
    }

    /// Ends the session (`CLOSE`), or makes sure it ends as soon as it
    /// starts. A session that already exited sends nothing.
    pub fn close(&self) {
        self.leave(Leaving::End);
    }

    /// Leaves the session running with no client (`DETACH`), now or as soon
    /// as it starts. A session that already exited sends nothing.
    pub fn detach(&self) {
        self.leave(Leaving::Keep);
    }

    fn leave(&self, how: Leaving) {
        let mut phase = self.phase.lock();
        match &mut *phase {
            Phase::Starting { left, input, .. } => {
                *left = Some(how);
                input.clear();
            }
            Phase::Running(running) => {
                match how {
                    Leaving::End => running.close(),
                    Leaving::Keep => running.detach(),
                }
                *phase = Phase::Gone;
            }
            Phase::Gone => {}
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
) {
    let left = || matches!(&*phase.lock(), Phase::Starting { left: Some(_), .. });
    let started = connect().and_then(|connection| {
        // A tab closed while connecting needs no session: a spawn doesn't
        // start one, and an attach leaves it detached.
        if left() {
            anyhow::bail!("closed before it started");
        }
        match start {
            Start::Spawn(request) => connection.spawn(request, sink.clone()),
            Start::Attach(request) => connection.attach(request, sink.clone()),
        }
    });
    let mut phase = phase.lock();
    let running = match started {
        Ok(running) => running,
        Err(_) if matches!(&*phase, Phase::Starting { left: Some(_), .. }) => {
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
    let Phase::Starting { input, resize, parent, left } = std::mem::replace(&mut *phase, Phase::Gone) else {
        unreachable!("only this thread leaves Starting");
    };
    match left {
        Some(Leaving::End) => return running.close(),
        Some(Leaving::Keep) => return running.detach(),
        None => {}
    }
    if let Some(size) = resize {
        running.resize(size.cols, size.rows, size.cell_w, size.cell_h);
    }
    if let Some(hwnd) = parent {
        running.reparent(hwnd);
    }
    for bytes in input {
        running.input(&bytes);
    }
    *phase = Phase::Running(running);
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use super::*;
    use crate::remote::local::sys::Listener;
    use crate::remote::local::{Endpoint, Stream, connect};
    use crate::remote::protocol::Frame;
    use crate::remote::protocol::client::{self, Spawned, ToClient, ToCore, Welcome};

    /// What the tab saw.
    struct Recorder(mpsc::Sender<String>);

    impl SessionSink for Recorder {
        fn output(&mut self, bytes: &[u8]) {
            self.0.send(format!("output {}", String::from_utf8_lossy(bytes))).ok();
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
    }

    fn request() -> SpawnRequest {
        SpawnRequest {
            command: "sh".into(),
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

    fn temp_dir(tag: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!("brindle-session-{tag}-{}", std::process::id()));
        std::fs::remove_dir_all(&path).ok();
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    /// A core that waits `delay` before its `WELCOME`, starts one session,
    /// and reports the messages it gets after `SPAWN`.
    fn slow_core(dir: &std::path::Path, delay: Duration) -> mpsc::Receiver<ToCore> {
        let listener = Listener::bind(dir);
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut stream = listener.accept();
            let ToCore::Hello(_) = ToCore::decode(&Frame::read_from(&mut stream).unwrap()).unwrap() else {
                panic!("not HELLO")
            };
            std::thread::sleep(delay);
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
        rx
    }

    fn send(stream: &mut Stream, message: ToClient) {
        message.encode().unwrap().write_to(stream).unwrap();
    }

    /// A core that welcomes one client, then hands each message to `answer`
    /// (which may reply) and reports it.
    fn scripted_core(
        dir: &std::path::Path,
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
                protocol: 1,
                core_version: "0.0.0".into(),
                core_build: "test".into(),
                pid: std::process::id(),
                holder_protocols: vec![2],
            };
            send(&mut stream, ToClient::Welcome(welcome));
            while let Ok(frame) = Frame::read_from(&mut stream) {
                let message = ToCore::decode(&frame).unwrap();
                answer(&message, &mut stream);
                if tx.send(message).is_err() {
                    break;
                }
            }
        });
        rx
    }

    fn connector(dir: &std::path::Path) -> impl FnOnce() -> Result<Arc<Connection>> + Send + 'static {
        let endpoint = Endpoint::at(dir.join("unused"), dir.to_path_buf());
        move || {
            let connected = connect(&endpoint, "tok")?;
            Connection::start(endpoint, "tok", connected)
        }
    }

    fn attach_request(session: SessionId) -> AttachRequest {
        AttachRequest { session, key: format!("k{session}"), cols: 120, rows: 40 }
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
        rx.recv_timeout(Duration::from_secs(10)).unwrap()
    }

    fn seen(rx: &mpsc::Receiver<String>) -> String {
        rx.recv_timeout(Duration::from_secs(10)).unwrap()
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
        let session = Session::start_with(connector(&dir), request(), Box::new(Recorder(tx)));
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
        assert!(core.recv_timeout(Duration::from_millis(300)).is_err());
    }

    #[test]
    fn an_attach_gets_the_replay_then_live_output_and_queued_input() {
        let dir = temp_dir("attach");
        let core = scripted_core(&dir, replaying);
        let (tx, tab) = mpsc::channel();
        let session = Session::attach_with(connector(&dir), attach_request(5), Box::new(Recorder(tx)));
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
        assert_eq!(session.id_and_key(), Some((5, "k5".to_string())));

        session.detach();
        assert!(matches!(recv(&core), ToCore::Detach(detach) if detach.session == 5));
        // Gone: neither again, nor a close.
        session.detach();
        session.close();
        session.input(b"late");
        assert!(core.recv_timeout(Duration::from_millis(300)).is_err());
        std::fs::remove_dir_all(&dir).ok();
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
        std::fs::remove_dir_all(&dir).ok();
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
        let session = Session::attach_with(connector(&dir), attach_request(5), Box::new(Recorder(tx)));
        got.recv_timeout(Duration::from_secs(10)).unwrap();
        session.detach();
        release.send(()).unwrap();
        let ToCore::Attach(_) = recv(&core) else { panic!("not ATTACH") };
        assert!(matches!(recv(&core), ToCore::Detach(detach) if detach.session == 5));
        assert!(core.recv_timeout(Duration::from_millis(300)).is_err(), "no CLOSE");
        std::fs::remove_dir_all(&dir).ok();
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
        while ended.pid().is_none() {
            std::thread::sleep(Duration::from_millis(10));
        }
        ended.close();
        ended.detach();
        assert!(matches!(recv(&core), ToCore::Close(close) if close.session == id));

        let (exited, tab, _) = start("/exits");
        assert_eq!(seen(&tab), "exited Some(0)");
        exited.close();
        exited.detach();
        assert!(core.recv_timeout(Duration::from_millis(300)).is_err(), "nothing for an exited session");
        std::fs::remove_dir_all(&dir).ok();
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
                let running = connection.spawn(request(), sink.clone()).unwrap();
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
        assert!(core.recv_timeout(Duration::from_millis(300)).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_slow_core_never_blocks_the_start_and_queued_input_arrives() {
        let dir = temp_dir("slow");
        let core = slow_core(&dir, Duration::from_millis(800));
        let endpoint = Endpoint::at(dir.join("unused"), dir.clone());
        let (tx, seen) = mpsc::channel();
        let started = Instant::now();
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
        assert!(started.elapsed() < Duration::from_millis(200), "{:?}", started.elapsed());
        assert_eq!(session.pid(), None);

        let next = || core.recv_timeout(Duration::from_secs(10)).unwrap();
        let ToCore::Spawn(spawn) = next() else { panic!("not SPAWN") };
        assert_eq!(spawn.command.as_deref(), Some("sh"));
        let ToCore::Resize(resize) = next() else { panic!("not RESIZE") };
        assert_eq!((resize.session, resize.cols, resize.rows), (7, 100, 30));
        let ToCore::Data(one) = next() else { panic!("not DATA") };
        let ToCore::Data(two) = next() else { panic!("not DATA") };
        assert_eq!((one.session, one.bytes.as_slice()), (7, &b"echo one\r"[..]));
        assert_eq!(two.bytes, b"echo two\r");
        assert!(started.elapsed() >= Duration::from_millis(800));
        assert_eq!(session.pid(), Some(4242));
        assert_eq!(seen.recv_timeout(Duration::from_secs(5)).unwrap(), "output $ ");

        // Running now: input goes straight out.
        session.input(b"x");
        let ToCore::Data(x) = next() else { panic!("not DATA") };
        assert_eq!(x.bytes, b"x");
        session.close();
        assert!(matches!(next(), ToCore::Close(close) if close.session == 7));
        std::fs::remove_dir_all(&dir).ok();
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
        ran.recv_timeout(Duration::from_secs(5)).unwrap();
        // Closed tabs say nothing, even about a failure.
        assert!(seen.recv_timeout(Duration::from_millis(300)).is_err());
    }
}
