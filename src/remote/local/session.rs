//! A local tab's session, as its terminal holds it from the moment the tab
//! opens.
//!
//! [`Session::start`] returns at once: connecting (starting a core if
//! needed, or reconnecting), an upgrade's step-down and the `SPAWN` itself
//! all happen on a thread of their own, so opening a tab never blocks the UI
//! thread. Input and resizes made meanwhile are kept, and go out in order
//! once the core reports the session. A failure goes to the sink, which
//! shows it in the tab.

use std::sync::Arc;

use anyhow::Result;
use parking_lot::Mutex;

use super::connection::{Running, Sink};
use super::{Connection, SessionSink, SpawnRequest};

enum Phase {
    /// The session isn't there yet.
    Starting {
        input: Vec<Vec<u8>>,
        resize: Option<Size>,
        closed: bool,
    },
    Running(Running),
    /// It never started; the sink has said why.
    Failed,
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
        let phase = Arc::new(Mutex::new(Phase::Starting { input: Vec::new(), resize: None, closed: false }));
        let sink: Sink = Arc::new(Mutex::new(sink));
        let starting = phase.clone();
        let failing = sink.clone();
        let spawned = std::thread::Builder::new()
            .name("server-spawn".into())
            .spawn(move || run(connect, request, sink, &starting));
        if let Err(error) = spawned {
            *phase.lock() = Phase::Failed;
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

    pub fn input(&self, bytes: &[u8]) {
        match &mut *self.phase.lock() {
            Phase::Starting { input, closed: false, .. } => input.push(bytes.to_vec()),
            Phase::Running(running) => running.input(bytes),
            Phase::Starting { .. } | Phase::Failed => {}
        }
    }

    pub fn resize(&self, cols: u16, rows: u16, cell_w: u16, cell_h: u16) {
        match &mut *self.phase.lock() {
            Phase::Starting { resize, .. } => *resize = Some(Size { cols, rows, cell_w, cell_h }),
            Phase::Running(running) => running.resize(cols, rows, cell_w, cell_h),
            Phase::Failed => {}
        }
    }

    /// Ends the session, or makes sure it ends as soon as it starts.
    pub fn close(&self) {
        match &mut *self.phase.lock() {
            Phase::Starting { closed, input, .. } => {
                *closed = true;
                input.clear();
            }
            Phase::Running(running) => running.close(),
            Phase::Failed => {}
        }
    }
}

/// Connects, spawns, and hands the session what was queued meanwhile.
fn run(
    connect: impl FnOnce() -> Result<Arc<Connection>>,
    request: SpawnRequest,
    sink: Sink,
    phase: &Mutex<Phase>,
) {
    let closed = || matches!(&*phase.lock(), Phase::Starting { closed: true, .. });
    let started = connect().and_then(|connection| {
        // A tab closed while connecting needs no session.
        if closed() {
            anyhow::bail!("closed before it started");
        }
        connection.spawn(request, sink.clone())
    });
    let mut phase = phase.lock();
    let running = match started {
        Ok(running) => running,
        Err(_) if matches!(&*phase, Phase::Starting { closed: true, .. }) => {
            *phase = Phase::Failed;
            return;
        }
        Err(error) => {
            *phase = Phase::Failed;
            drop(phase);
            sink.lock().failed(&error);
            return;
        }
    };
    // Under the lock, so later input waits for what was queued.
    let Phase::Starting { input, resize, closed } = std::mem::replace(&mut *phase, Phase::Failed) else {
        unreachable!("only this thread leaves Starting");
    };
    if closed {
        running.close();
        return;
    }
    if let Some(size) = resize {
        running.resize(size.cols, size.rows, size.cell_w, size.cell_h);
    }
    for bytes in input {
        running.input(&bytes);
    }
    *phase = Phase::Running(running);
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::os::unix::net::UnixListener;
    use std::path::PathBuf;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use super::*;
    use crate::remote::local::{Endpoint, connect};
    use crate::remote::protocol::Frame;
    use crate::remote::protocol::client::{self, Spawned, ToClient, ToCore, Welcome};

    /// What the tab saw.
    struct Recorder(mpsc::Sender<String>);

    impl SessionSink for Recorder {
        fn output(&mut self, bytes: &[u8]) {
            self.0.send(format!("output {}", String::from_utf8_lossy(bytes))).ok();
        }
        fn replay(&mut self, _: bool) {}
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
        let listener = UnixListener::bind(dir.join("core.sock")).unwrap();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
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
                    let spawned = Spawned { req: spawn.req, session: 7, pid: 4242 };
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

    #[test]
    fn a_slow_core_never_blocks_the_start_and_queued_input_arrives() {
        let dir = temp_dir("slow");
        let core = slow_core(&dir, Duration::from_millis(800));
        let endpoint = Endpoint { helper: dir.join("unused"), dir: dir.clone() };
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

        let mut next = || core.recv_timeout(Duration::from_secs(10)).unwrap();
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
