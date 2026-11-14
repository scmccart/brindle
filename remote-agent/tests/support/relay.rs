//! A `brindle-remote relay` for tests: its stdin and stdout are one end of a
//! socket pair, and the test reads and writes the other end, as `ssh` would.

use std::fs::File;
use std::io;
use std::net::Shutdown;
use std::os::fd::OwnedFd;
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use brindle_remote_protocol::client;
use brindle_remote_protocol::relay::{RelayHost, RelayMessage};
use brindle_remote_protocol::{Frame, skip_preamble};

use super::{TIMEOUT, TestClient};

/// The relay's idle limit in the liveness tests...
pub const IDLE: Duration = Duration::from_millis(1500);
/// ...and its heartbeat interval.
pub const HEARTBEAT: Duration = Duration::from_millis(300);
/// The variables that set them.
pub const SHORT_TIMINGS: [(&str, &str); 2] = [
    ("BRINDLE_TEST_RELAY_IDLE_MS", "1500"),
    ("BRINDLE_TEST_RELAY_HEARTBEAT_MS", "300"),
];

/// A running relay. Dropping it kills it.
pub struct Relay {
    child: Child,
    /// The test's end of the link, until a client takes it.
    stream: Option<UnixStream>,
}

impl Relay {
    /// Starts `bin relay` for the server directory `dir`, without a scope,
    /// with `env` on top. A core it starts inherits its environment, so the
    /// holders of that core start only `--version` as a core of their own.
    /// Its stderr goes to a log in `dir`.
    pub fn start(bin: &Path, dir: &Path, env: &[(&str, &str)]) -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let n = NEXT.fetch_add(1, Ordering::Relaxed);
        let (ours, theirs) = UnixStream::pair().unwrap();
        let stderr = File::create(dir.join(format!("relay-{n}.log"))).unwrap();
        let mut command = Command::new(bin);
        command
            .arg("relay")
            .env("BRINDLE_SERVER_DIR", dir)
            .env("BRINDLE_NO_SCOPE", "1")
            .env("BRINDLE_TEST_HOLDER_CORE_ARGS", "--version")
            .envs(env.iter().copied())
            .stdin(Stdio::from(OwnedFd::from(theirs.try_clone().unwrap())))
            .stdout(Stdio::from(OwnedFd::from(theirs)))
            .stderr(Stdio::from(stderr));
        // A freshly written binary can be briefly busy: another test
        // thread's fork may still hold the writer's fd until it execs.
        let mut attempts = 0;
        let child = loop {
            match command.spawn() {
                Err(error)
                    if error.kind() == io::ErrorKind::ExecutableFileBusy && attempts < 100 =>
                {
                    attempts += 1;
                    thread::sleep(Duration::from_millis(20));
                }
                result => break result.unwrap(),
            }
        };
        // Only the relay holds its end of the link now.
        drop(command);
        Self {
            child,
            stream: Some(ours),
        }
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// The relay's first frame after its marker: `RELAY_HOST` or
    /// `RELAY_ERROR`. Nothing may come before the marker.
    pub fn first(&mut self) -> RelayMessage {
        let stream = self.stream.as_mut().expect("a client has the link");
        stream.set_read_timeout(Some(TIMEOUT)).unwrap();
        let noise = skip_preamble(stream).unwrap();
        assert!(
            noise.is_empty(),
            "the relay printed {noise:?} before its marker"
        );
        let frame = Frame::read_from(stream).unwrap();
        RelayMessage::decode(&frame).unwrap()
    }

    /// The relay's `RELAY_HOST`.
    pub fn host(&mut self) -> RelayHost {
        match self.first() {
            RelayMessage::Host(host) => host,
            other => panic!("expected RELAY_HOST, got {other:?}"),
        }
    }

    /// A client over the link (after [`Relay::host`]), past its `HELLO`
    /// with `token`.
    pub fn client(&mut self, token: &str) -> TestClient {
        self.client_with(token, |_| {})
    }

    /// [`Relay::client`] with the `HELLO` as `edit` changes it.
    pub fn client_with(
        &mut self,
        token: &str,
        edit: impl FnOnce(&mut client::Hello),
    ) -> TestClient {
        let stream = self.stream.take().expect("one client per relay");
        stream.set_read_timeout(None).unwrap();
        TestClient::over(stream).greet(token, edit)
    }

    /// Waits up to `limit` for the relay to exit, and returns how it did.
    pub fn exits_within(&mut self, limit: Duration) -> Option<ExitStatus> {
        let deadline = Instant::now() + limit;
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                return Some(status);
            }
            if Instant::now() >= deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    pub fn running(&mut self) -> bool {
        self.child.try_wait().unwrap().is_none()
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Ends the relay's stdin, as a client closing the link does, keeping the
/// other direction open.
pub fn hang_up(client: &TestClient) {
    client.stream.shutdown(Shutdown::Write).unwrap();
}
