//! Helper deployment for remote endpoints, the launch of their relays, and
//! the per-tab stdio transport of remote PTY sessions.
//!
//! **Deployment.** An endpoint keeps this build's Linux `brindle-remote`
//! cached by version and digest ([`Helper`]), so it is copied there once per
//! build:
//! - an SSH host in the user's `~/.cache/brindle/<version>-<digest>/`,
//!   uploaded with user-only permissions;
//! - a container in `/tmp/brindle-helper-<version>-<digest>/`, a directory
//!   of root's that every user of the container runs the helper from and
//!   none of them can write. Root makes it, takes the copied file over and
//!   removes other builds' directories. A directory that someone else made
//!   is refused and left alone. Nothing is removed when a connection ends.
//!
//! **Launch** ([`launch`]). A relay starts with one command, which is its
//! connection: a script that runs the cached helper's `relay`, or exits
//! [`NOT_CACHED`] when the endpoint has no helper it may run (in a
//! container: one that isn't root's own, or is a symbolic link). Only then
//! is the helper deployed, and the launch run once more.
//!
//! **The stdio transport** ([`connect`]) runs the same helper as
//! `--server`, one process per tab, until remote tabs move to the relay.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};
use futures::SinkExt as _;
use futures::channel::mpsc::Sender as EventSender;
use sha2::{Digest as _, Sha256};

use super::protocol::{
    DEFAULT_IDLE_LIMIT_SECS, ErrorMessage, Frame, HEARTBEAT_INTERVAL, MSG_DATA, MSG_ERROR,
    MSG_EXIT, MSG_PROCESS_INFO, MSG_SPAWN_RESPONSE, ProcessInfo, SpawnRequest, SpawnResponse,
};
use super::{hide_window, wait_process};
use crate::config::{Profile, RemoteTarget};
use crate::terminal::GridSize;

const REMOTE_DIR: &str = ".cache/brindle";
/// How much of the agent's stdout is read ahead: the preamble scan and every
/// frame read share one buffered reader.
const AGENT_READ_BUFFER: usize = 64 * 1024;

#[derive(Debug)]
pub enum Event {
    Output(Vec<u8>),
    ProcessInfo(ProcessInfo),
    Exited,
    /// Bootstrap or protocol failure: keep the diagnostic tab open, but it
    /// no longer has a live child.
    Failed,
}

enum Outgoing {
    Data(Vec<u8>),
    Resize(GridSize),
    QueryProcess,
}

const DATA_CHUNK_SIZE: usize = 32 * 1024;

impl Outgoing {
    fn into_frame(self) -> std::io::Result<Frame> {
        match self {
            Self::Data(bytes) => Frame::data(bytes),
            Self::Resize(size) => {
                Frame::resize(size.cols, size.rows, size.cell_width, size.cell_height)
            }
            Self::QueryProcess => Frame::query_process(),
        }
    }
}

/// A running remote PTY connection. Clones share one writer thread.
#[derive(Clone)]
pub struct Handle {
    tx: SyncSender<Outgoing>,
    cancel: Arc<AtomicBool>,
}

impl Handle {
    /// Queues `bytes` as data messages of at most [`DATA_CHUNK_SIZE`] each.
    pub fn input(&self, bytes: Vec<u8>) {
        if bytes.is_empty() {
            return;
        }
        if bytes.len() <= DATA_CHUNK_SIZE {
            self.tx.try_send(Outgoing::Data(bytes)).ok();
            return;
        }
        for chunk in bytes.chunks(DATA_CHUNK_SIZE) {
            if self.tx.try_send(Outgoing::Data(chunk.to_vec())).is_err() {
                break;
            }
        }
    }

    pub fn resize(&self, size: GridSize) {
        self.tx.try_send(Outgoing::Resize(size)).ok();
    }

    pub fn query_process(&self) {
        self.tx.try_send(Outgoing::QueryProcess).ok();
    }

    pub fn close(&self) {
        self.cancel.store(true, Ordering::Release);
    }
}

/// Starts bootstrap and connection work on a background thread, returning a
/// handle immediately so launching a remote tab never blocks the UI thread.
pub fn connect(
    profile: Profile,
    command_override: Option<(String, Vec<String>)>,
    cwd_override: Option<PathBuf>,
    size: GridSize,
    events: EventSender<Event>,
) -> Handle {
    connect_with(
        profile,
        command_override,
        cwd_override,
        size,
        events,
        Timing::default(),
    )
}

/// How the client and the helper keep each other honest about being alive.
#[derive(Clone, Copy)]
struct Timing {
    /// How often an idle client sends a heartbeat.
    heartbeat_interval: Duration,
    /// How long the helper may hear nothing before it ends the session.
    idle_limit_secs: u32,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            heartbeat_interval: HEARTBEAT_INTERVAL,
            idle_limit_secs: DEFAULT_IDLE_LIMIT_SECS,
        }
    }
}

fn connect_with(
    profile: Profile,
    command_override: Option<(String, Vec<String>)>,
    cwd_override: Option<PathBuf>,
    size: GridSize,
    mut events: EventSender<Event>,
    timing: Timing,
) -> Handle {
    let (tx, rx) = mpsc::sync_channel(128);
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    thread::Builder::new()
        .name("remote-connect".into())
        .spawn(move || {
            if let Err(err) = run_connection(
                &profile,
                command_override,
                cwd_override,
                size,
                rx,
                worker_cancel,
                &mut events,
                timing,
            ) {
                let text = format!("\n\x1b[31mBrindle remote connection failed:\x1b[0m\n{err:#}\n");
                send_event(&mut events, notice(&text));
                send_event(&mut events, Event::Failed);
            }
        })
        .ok();
    Handle { tx, cancel }
}

/// Text for the terminal itself to show. The grid needs CR LF, and ssh or a
/// remote shell supplies bare LFs, which would leave each line indented
/// under the end of the last.
fn notice(text: &str) -> Event {
    Event::Output(text.replace("\r\n", "\n").replace('\n', "\r\n").into_bytes())
}

fn send_event(events: &mut EventSender<Event>, event: Event) -> bool {
    futures::executor::block_on(events.send(event)).is_ok()
}

/// Sends queued input to the agent and, while idle, a heartbeat every
/// `heartbeat` so the agent can tell a quiet client from a vanished one.
///
/// This is the only loop that writes to the agent, so a heartbeat is due only
/// when nothing has been written for `heartbeat`: any frame already proves the
/// client is alive. A write that blocks because the agent is not reading (it
/// is busy forwarding a flood of output) delays the next heartbeat, but the
/// agent hears this write the moment it reads again, and it only checks for
/// silence when it is idle.
fn write_loop(
    outgoing: &mpsc::Receiver<Outgoing>,
    cancel: &AtomicBool,
    writer: &mut impl std::io::Write,
    heartbeat: Duration,
) -> Result<()> {
    let mut last_written = Instant::now();
    loop {
        if cancel.load(Ordering::Acquire) {
            // The transport may already be gone (the session ended on its
            // own, or the child was killed): a failed goodbye is no error.
            if let Ok(frame) = Frame::exit(None) {
                frame.write_to(writer).ok();
            }
            break;
        }
        match outgoing.recv_timeout(Duration::from_millis(100)) {
            Ok(message) => {
                message.into_frame()?.write_to(writer)?;
                last_written = Instant::now();
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if last_written.elapsed() >= heartbeat {
                    // After the session ended this fails; the caller ignores a
                    // writer error once the protocol saw a clean exit.
                    Frame::heartbeat().write_to(writer)?;
                    last_written = Instant::now();
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    Ok(())
}

fn run_connection(
    profile: &Profile,
    command_override: Option<(String, Vec<String>)>,
    cwd_override: Option<PathBuf>,
    size: GridSize,
    outgoing: mpsc::Receiver<Outgoing>,
    cancel: Arc<AtomicBool>,
    events: &mut EventSender<Event>,
    timing: Timing,
) -> Result<()> {
    let target = profile
        .remote
        .as_ref()
        .context("remote profile has no remote target")?;
    let helper = Helper::find()?;
    let commands = Commands { cancel: &cancel };
    let remote_agent = match target {
        RemoteTarget::Ssh { .. } => deploy_ssh(&commands, target, &helper)?,
        RemoteTarget::Container {
            engine, container, ..
        } => {
            deploy_container(&commands, engine, container, &helper)?;
            format!("{}/brindle-remote", helper.container_dir())
        }
    };
    let (command, args) = command_override.unwrap_or_else(|| {
        (
            profile.command.clone().unwrap_or_default(),
            profile.args.clone(),
        )
    });
    let request = SpawnRequest {
        command: (!command.is_empty()).then_some(command),
        args,
        cwd: cwd_override
            .or_else(|| profile.cwd.as_ref().map(PathBuf::from))
            .filter(|p| !p.as_os_str().is_empty())
            .map(|p| p.to_string_lossy().into_owned()),
        env: crate::terminal::child_env(profile).into_iter().collect(),
        cols: size.cols,
        rows: size.rows,
        cell_width: size.cell_width,
        cell_height: size.cell_height,
        idle_limit_secs: timing.idle_limit_secs,
    };
    let mut command = agent_command(target, &remote_agent)?;
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = ChildGuard::new(command.spawn().context("starting remote agent")?);
    let mut stdin = child.take_stdin()?;
    let mut stdout = BufReader::with_capacity(AGENT_READ_BUFFER, child.take_stdout()?);
    let stderr = child.take_stderr()?;
    Frame::spawn(&request)?.write_to(&mut stdin)?;
    let child = Arc::new(Mutex::new(child));
    let watched_child = child.clone();
    let watched_cancel = cancel.clone();
    let cancel_watcher = thread::Builder::new()
        .name("remote-cancel".into())
        .spawn(move || {
            while !watched_cancel.load(Ordering::Acquire) {
                thread::sleep(Duration::from_millis(50));
            }
            let mut child = watched_child
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            child.terminate();
        })?;

    let writer_cancel = cancel.clone();
    let heartbeat_interval = timing.heartbeat_interval;
    let writer = thread::spawn(move || {
        write_loop(&outgoing, &writer_cancel, &mut stdin, heartbeat_interval)
    });
    let stderr_reader = spawn_stderr_drain(stderr);

    let protocol_result = (|| -> Result<()> {
        super::protocol::skip_preamble(&mut stdout)
            .context("waiting for the remote agent's frame marker")?;
        let first = Frame::read_from(&mut stdout).context("reading remote agent handshake")?;
        if first.msg_type != MSG_SPAWN_RESPONSE {
            bail!(
                "remote agent sent message {} before spawn response",
                first.msg_type
            );
        }
        let response: SpawnResponse = first.decode_json()?;
        if !response.success {
            bail!(
                "remote agent could not spawn session: {}",
                response.error.unwrap_or_else(|| "unknown error".into())
            );
        }
        loop {
            let Some(frame) =
                Frame::read_from_or_eof(&mut stdout).context("reading remote agent frame")?
            else {
                bail!("connection lost: the remote agent closed the stream without reporting an exit");
            };
            match frame.msg_type {
                MSG_DATA => {
                    if frame.payload.len() > DATA_CHUNK_SIZE {
                        bail!(
                            "remote data frame is {} bytes; maximum is {DATA_CHUNK_SIZE}",
                            frame.payload.len()
                        );
                    }
                    if !send_event(events, Event::Output(frame.payload)) {
                        break;
                    }
                }
                MSG_PROCESS_INFO => {
                    let info: ProcessInfo = frame.decode_json()?;
                    if !send_event(events, Event::ProcessInfo(info)) {
                        break;
                    }
                }
                MSG_EXIT => break,
                MSG_ERROR => {
                    let error: ErrorMessage = frame.decode_json()?;
                    let text = format!("\n\x1b[31mRemote agent error:\x1b[0m {}\n", error.message);
                    if !send_event(events, notice(&text)) {
                        break;
                    }
                }
                _ => {}
            }
        }
        Ok(())
    })();

    cancel.store(true, Ordering::Release);
    cancel_watcher
        .join()
        .map_err(|_| anyhow::anyhow!("remote cancellation thread panicked"))?;
    let writer_result = writer
        .join()
        .map_err(|_| anyhow::anyhow!("remote writer thread panicked"))?;
    let status = child
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .wait_bounded(Duration::from_secs(2))?;
    let diagnostic = await_stderr(&stderr_reader);
    if let Err(err) = protocol_result {
        return Err(with_diagnostic(err, &diagnostic));
    }
    // The protocol saw a clean exit; a writer that lost the pipe afterwards
    // (typing into a finished session) is not a failure.
    if let Err(err) = writer_result {
        log::debug!("remote writer ended with an error after exit: {err:#}");
    }
    if !status.success() && !diagnostic.trim().is_empty() {
        send_event(events, notice(&format!("\n{diagnostic}\n")));
    }
    send_event(events, Event::Exited);
    Ok(())
}

const STDERR_DIAGNOSTIC_LIMIT: usize = 64 * 1024;
/// How long to wait for a stderr pipe to close after its process has ended.
/// A backgrounded ssh mux master (ControlPersist) can inherit the pipe and
/// keep it open indefinitely.
const STDERR_DRAIN_WAIT: Duration = Duration::from_secs(1);

/// Drains `stderr` on a thread and delivers the bounded text through a
/// channel, so the caller can stop waiting if the pipe never closes.
fn spawn_stderr_drain(stderr: impl Read + Send + 'static) -> mpsc::Receiver<String> {
    let (tx, rx) = mpsc::channel();
    // If the thread can't start, `tx` is dropped with the closure and the
    // receiver simply reports no output.
    thread::Builder::new()
        .name("remote-stderr".into())
        .spawn(move || {
            tx.send(drain_stderr(stderr)).ok();
        })
        .ok();
    rx
}

/// Takes what the drain has collected, giving up (and leaving the thread to
/// finish on its own) if the pipe stays open.
fn await_stderr(rx: &mpsc::Receiver<String>) -> String {
    rx.recv_timeout(STDERR_DRAIN_WAIT).unwrap_or_default()
}

fn with_diagnostic(err: anyhow::Error, diagnostic: &str) -> anyhow::Error {
    let diagnostic = diagnostic.trim();
    if diagnostic.is_empty() {
        err
    } else {
        anyhow::anyhow!("{err:#}\n{diagnostic}")
    }
}

fn drain_stderr(mut stderr: impl Read) -> String {
    let mut retained = Vec::new();
    let mut buffer = [0u8; 4096];
    loop {
        let Ok(read) = stderr.read(&mut buffer) else {
            break;
        };
        if read == 0 {
            break;
        }
        let keep = (STDERR_DIAGNOSTIC_LIMIT - retained.len()).min(read);
        retained.extend_from_slice(&buffer[..keep]);
    }
    String::from_utf8_lossy(&retained).into_owned()
}

struct ChildGuard(Option<Child>);

impl ChildGuard {
    fn new(child: Child) -> Self {
        Self(Some(child))
    }

    fn child_mut(&mut self) -> Result<&mut Child> {
        self.0
            .as_mut()
            .context("remote transport child was already reaped")
    }

    fn take_stdin(&mut self) -> Result<std::process::ChildStdin> {
        self.child_mut()?
            .stdin
            .take()
            .context("remote agent stdin unavailable")
    }

    fn take_stdout(&mut self) -> Result<std::process::ChildStdout> {
        self.child_mut()?
            .stdout
            .take()
            .context("remote agent stdout unavailable")
    }

    fn take_stderr(&mut self) -> Result<std::process::ChildStderr> {
        self.child_mut()?
            .stderr
            .take()
            .context("remote agent stderr unavailable")
    }

    fn terminate(&mut self) {
        if let Some(child) = &mut self.0 {
            child.kill().ok();
        }
    }

    fn wait_bounded(&mut self, timeout: Duration) -> Result<ExitStatus> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self
                .child_mut()?
                .try_wait()
                .context("waiting for remote agent transport")?
            {
                self.0.take();
                return Ok(status);
            }
            if Instant::now() >= deadline {
                self.terminate();
                let status = self
                    .child_mut()?
                    .wait()
                    .context("reaping remote agent transport")?;
                self.0.take();
                return Ok(status);
            }
            thread::sleep(Duration::from_millis(20));
        }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.terminate();
        if let Some(mut child) = self.0.take() {
            child.wait().ok();
        }
    }
}

fn agent_path() -> Result<PathBuf> {
    super::find_helper()?.context(
        "Linux remote helper not found beside Brindle; set BRINDLE_REMOTE_AGENT to a Linux brindle-remote binary",
    )
}

/// What a relay's launch exits with when the endpoint has no helper of this
/// build that it may run (design decision 8). A helper never exits with it,
/// so before the relay's frame marker it always means: deploy, then launch
/// again.
pub const NOT_CACHED: i32 = 97;
/// What a container's directory step exits with when the cache directory is
/// taken: something is there that isn't a directory of root's.
const TAKEN: i32 = 98;
/// How long each deployment command may take.
const DEPLOY_LIMIT: Duration = Duration::from_secs(30);

/// This build's Linux helper: the file on this machine, and the version and
/// digest that endpoints cache it by.
struct Helper {
    local: PathBuf,
    version: &'static str,
    digest: String,
}

impl Helper {
    /// The helper beside Brindle (or `BRINDLE_REMOTE_AGENT`), hashed as it
    /// is now: a rebuilt helper is another build.
    fn find() -> Result<Self> {
        let local = agent_path()?;
        let digest = file_sha256(&local)?;
        Ok(Self { local, version: env!("CARGO_PKG_VERSION"), digest })
    }

    /// Where an SSH host keeps it, with `$HOME` left for the host's shell.
    fn ssh_path(&self) -> String {
        format!("$HOME/{REMOTE_DIR}/{}-{}/brindle-remote", self.version, self.digest)
    }

    /// The directory a container keeps it in, for all of its users.
    fn container_dir(&self) -> String {
        format!("/tmp/brindle-helper-{}-{}", self.version, self.digest)
    }
}

/// A relay's launch ended, or was ended, before the relay's frame marker.
#[derive(Debug)]
pub struct Unstarted {
    /// The process's exit code; `None` when it was killed or didn't exit in
    /// time. A launch script's [`NOT_CACHED`] is one.
    pub code: Option<i32>,
    /// What the process wrote to stderr.
    pub said: String,
    /// The whole failure, as a tab shows it.
    pub message: String,
}

impl std::fmt::Display for Unstarted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Unstarted {}

/// Runs the deployment's commands: for real ([`Commands`]), or as a test
/// scripts them.
trait Run {
    /// Runs `command` to its end, with the file at `input` as its stdin when
    /// one is given. `what` names the command in errors.
    fn run(&self, command: Command, input: Option<&Path>, what: &str) -> Result<Ran>;
}

/// How a deployment command ended.
#[derive(Debug)]
struct Ran {
    /// Its exit code; `None` when it was killed.
    code: Option<i32>,
    stderr: String,
}

/// Runs each command for at most [`DEPLOY_LIMIT`], ending it early once
/// `cancel` is set.
struct Commands<'a> {
    cancel: &'a AtomicBool,
}

impl Run for Commands<'_> {
    fn run(&self, mut command: Command, input: Option<&Path>, what: &str) -> Result<Ran> {
        let input = match input {
            Some(path) => Some(
                File::open(path)
                    .with_context(|| format!("opening remote helper {}", path.display()))?,
            ),
            None => None,
        };
        command
            .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = command
            .spawn()
            .with_context(|| format!("starting {what}"))?;
        let stderr = child.stderr.take().map(spawn_stderr_drain);
        let sending = match (input, child.stdin.take()) {
            (Some(mut file), Some(mut stdin)) => {
                Some(thread::spawn(move || std::io::copy(&mut file, &mut stdin)))
            }
            _ => None,
        };
        let status =
            wait_process(child, self.cancel, DEPLOY_LIMIT).with_context(|| what.to_string())?;
        let sent = match sending {
            Some(sending) => sending
                .join()
                .map_err(|_| anyhow::anyhow!("{what}: the thread sending its input panicked"))?,
            None => Ok(0),
        };
        let stderr = stderr.map(|rx| await_stderr(&rx)).unwrap_or_default();
        // A command that failed says why itself. Under one that succeeded, a
        // pipe that broke is the failure.
        if status.success()
            && let Err(error) = sent
        {
            bail!("{what}: sending its input failed: {error}");
        }
        Ok(Ran { code: status.code(), stderr })
    }
}

fn hidden_command(program: &str) -> Command {
    let mut command = Command::new(program);
    hide_window(&mut command);
    command
}

/// "what failed", plus the command's stderr when it printed any.
fn bootstrap_failure(what: &str, stderr: &str) -> anyhow::Error {
    let stderr = stderr.trim();
    if stderr.is_empty() {
        anyhow::anyhow!("{what}")
    } else {
        anyhow::anyhow!("{what}: {stderr}")
    }
}

fn file_sha256(path: &Path) -> Result<String> {
    let mut file = File::open(path).with_context(|| format!("opening remote helper {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// What an SSH host's `sh` runs for a relay: the cached helper, when it is
/// there and says it is this version, else exit [`NOT_CACHED`]. It isn't a
/// login shell, whose startup files could print into the framed stream. The
/// path is a positional parameter, not a variable: a variable that the
/// user's environment already had would be changed for the relay, and the
/// relay's environment is its sessions'.
fn ssh_launch_script(helper: &Helper) -> String {
    format!(
        "set -- \"{path}\"; if test -x \"$1\" && test \"$(\"$1\" --version)\" = {version}; \
         then exec \"$1\" relay; fi; exit {NOT_CACHED}",
        path = helper.ssh_path(),
        version = helper.version,
    )
}

/// What an SSH host's `sh` runs to take the helper from its stdin: into a
/// user-only directory and through a temporary name, so the cached path only
/// ever holds a whole file.
fn ssh_upload_script(helper: &Helper) -> String {
    let remote = helper.ssh_path();
    format!(
        "umask 077; mkdir -p \"$HOME/{REMOTE_DIR}/{version}-{digest}\" && tmp=\"{remote}.tmp.$$\" \
         && cat > \"$tmp\" && chmod 700 \"$tmp\" && mv \"$tmp\" \"{remote}\"",
        version = helper.version,
        digest = helper.digest,
    )
}

/// What a container's `sh` runs for a relay, as the profile's user, with the
/// cache directory as `$1`: the helper in it, when the directory and the
/// file are root's own and neither is a symbolic link, else exit
/// [`NOT_CACHED`]. Only root can put anything in a directory of root's, or
/// (`/tmp` being sticky) put another in its place, so what passed the check
/// is what runs. Like the SSH launch it sets no variable.
const CONTAINER_LAUNCH: &str = concat!(
    r#"test -d "$1" && test ! -L "$1" && test "$(stat -c %u "$1")" = 0"#,
    r#" && test -f "$1/brindle-remote" && test ! -L "$1/brindle-remote" && test -x "$1/brindle-remote""#,
    r#" && test "$(stat -c %u "$1/brindle-remote")" = 0"#,
    r#" && exec "$1/brindle-remote" relay; exit 97"#,
);

/// Root's first deployment step in a container, with the cache directory as
/// `$1`: make it (mode 755), or keep one that is already a directory of
/// root's and not a symbolic link, with its mode put right. Anything else
/// there is someone else's: exit [`TAKEN`], touching nothing.
const CONTAINER_DIRECTORY: &str = concat!(
    r#"mkdir -m 755 "$1" 2>/dev/null && exit 0; "#,
    r#"if test -d "$1" && test ! -L "$1" && test "$(stat -c %u "$1")" = 0; then chmod 755 "$1"; exit; fi; "#,
    r#"if test -e "$1" || test -L "$1"; then exit 98; fi; "#,
    // Nothing is there: again, for mkdir's own message.
    r#"mkdir -m 755 "$1""#,
);

/// Root's last deployment step, with the cache directory as `$1` and the
/// copied file's name in it as `$2`: take the file over (`docker cp` can
/// keep the host file's owner), check that it is a regular file of root's,
/// and rename it into place. So the helper's path only ever holds a whole,
/// root-owned file, and a helper that is running is never written to. Then
/// remove the other builds' directories beside it, but only those that
/// plainly are directories of root's.
const CONTAINER_INSTALL: &str = concat!(
    r#"if test -f "$1/$2" && test ! -L "$1/$2" && chown 0:0 "$1/$2" && chmod 755 "$1/$2""#,
    r#" && test "$(stat -c %u "$1/$2")" = 0 && mv -f "$1/$2" "$1/brindle-remote"; "#,
    r#"then :; else rm -f "$1/$2"; exit 1; fi; "#,
    r#"for d in "${1%/*}"/brindle-helper-*; do "#,
    r#"test "$d" != "$1" && test -d "$d" && test ! -L "$d" && test "$(stat -c %u "$d")" = 0 && rm -rf "$d"; "#,
    r#"done; exit 0"#,
);

/// The command whose process is `target`'s relay, when the endpoint has this
/// build's helper; it exits [`NOT_CACHED`] when it hasn't. It is the warm
/// path's only connection: an SSH host checks the helper it has, and a
/// container (as the profile's user) that the helper is root's.
fn launch_command(target: &RemoteTarget, helper: &Helper) -> Result<Command> {
    match target {
        RemoteTarget::Ssh { .. } => {
            super::remote_command(target, "sh", &["-c".into(), ssh_launch_script(helper)])
        }
        RemoteTarget::Container { .. } => super::remote_command(
            target,
            "sh",
            &[
                "-c".into(),
                CONTAINER_LAUNCH.into(),
                "sh".into(),
                helper.container_dir(),
            ],
        ),
    }
}

/// Starts the relay at `target`, where `start` runs a launch command up to
/// the relay's frame marker (`local::relay::start`). The warm path is that
/// one command. When it exits [`NOT_CACHED`], the helper is deployed and the
/// launch run once more. Any other failure is the endpoint's or the
/// transport's own (ssh's 255 among them), which deploying wouldn't mend.
#[cfg_attr(not(any(target_os = "linux", windows)), allow(dead_code))]
pub(crate) fn launch<L>(
    target: &RemoteTarget,
    start: impl Fn(Command) -> std::result::Result<L, Unstarted>,
) -> Result<L> {
    let helper = Helper::find()?;
    // Nothing cancels a relay's launch: its commands end by their limits.
    let never = AtomicBool::new(false);
    launch_with(&Commands { cancel: &never }, target, &helper, start)
}

fn launch_with<L>(
    run: &impl Run,
    target: &RemoteTarget,
    helper: &Helper,
    start: impl Fn(Command) -> std::result::Result<L, Unstarted>,
) -> Result<L> {
    let first = match start(launch_command(target, helper)?) {
        Ok(link) => return Ok(link),
        Err(unstarted) => unstarted,
    };
    if first.code != Some(NOT_CACHED) {
        return Err(unlaunched(target, first));
    }
    match target {
        RemoteTarget::Ssh { .. } => upload_ssh(run, target, helper)?,
        RemoteTarget::Container {
            engine, container, ..
        } => deploy_container(run, engine, container, helper)?,
    }
    start(launch_command(target, helper)?).map_err(|again| match again.code {
        Some(NOT_CACHED) => undeployed(target, helper, &again),
        _ => unlaunched(target, again),
    })
}

/// A launch that failed for another reason than a missing helper.
fn unlaunched(target: &RemoteTarget, unstarted: Unstarted) -> anyhow::Error {
    match (target, unstarted.code) {
        // ssh itself failed (auth, host key, network), not the host's shell.
        (RemoteTarget::Ssh { .. }, Some(255)) => {
            bootstrap_failure("SSH connection failed", &unstarted.said)
        }
        _ => unstarted.into(),
    }
}

/// The endpoint still has no helper it may run, right after getting one.
fn undeployed(target: &RemoteTarget, helper: &Helper, again: &Unstarted) -> anyhow::Error {
    let what = match target {
        RemoteTarget::Ssh { .. } => format!(
            "the helper was uploaded, but running it on the host doesn't print version {}: \
             it must be this Brindle's helper, built for the host's Linux",
            helper.version
        ),
        RemoteTarget::Container { .. } => format!(
            "the helper was copied into the container, but the profile's user there doesn't \
             find {} and the helper in it to be root's own",
            helper.container_dir()
        ),
    };
    bootstrap_failure(&what, &again.said)
}

/// Uploads the helper to an SSH host's cache.
fn upload_ssh(run: &impl Run, target: &RemoteTarget, helper: &Helper) -> Result<()> {
    let command = super::remote_command(target, "sh", &["-c".into(), ssh_upload_script(helper)])?;
    let uploaded = run.run(command, Some(&helper.local), "SSH helper upload")?;
    if uploaded.code != Some(0) {
        return Err(bootstrap_failure("SSH helper upload failed", &uploaded.stderr));
    }
    Ok(())
}

/// `script` for root's `sh` in the container, with `args` as `$1` onwards.
fn container_root(engine: &str, container: &str, script: &str, args: &[&str]) -> Command {
    let mut command = hidden_command(engine);
    command
        .args(["exec", "--user", "0", container, "sh", "-c", script, "sh"])
        .args(args);
    command
}

/// A name that no other deployment gives its copy of the helper.
fn temporary_name() -> Result<String> {
    let mut random = [0u8; 8];
    getrandom::fill(&mut random).context("naming the container helper's copy")?;
    Ok(format!("brindle-remote.tmp-{}", hex(&random)))
}

/// Puts the helper in a container's cache, as root: the directory
/// ([`CONTAINER_DIRECTORY`]), the copy under a name of its own, then the
/// take-over, the rename into place and the removal of other builds
/// ([`CONTAINER_INSTALL`]). A directory that is taken is an error naming it,
/// and nothing more runs.
fn deploy_container(run: &impl Run, engine: &str, container: &str, helper: &Helper) -> Result<()> {
    let directory = helper.container_dir();
    let made = run.run(
        container_root(engine, container, CONTAINER_DIRECTORY, &[&directory]),
        None,
        "creating the container's helper directory",
    )?;
    match made.code {
        Some(0) => {}
        Some(TAKEN) => bail!(
            "{directory} exists in the container but isn't a directory owned by root, \
             so Brindle runs nothing from it and leaves it as it is; remove it there to connect"
        ),
        _ => return Err(bootstrap_failure("container helper directory failed", &made.stderr)),
    }
    let temporary = temporary_name()?;
    let mut copy = hidden_command(engine);
    copy.arg("cp")
        .arg(&helper.local)
        .arg(format!("{container}:{directory}/{temporary}"));
    let copied = run.run(copy, None, "copying container helper")?;
    if copied.code != Some(0) {
        return Err(bootstrap_failure("container helper upload failed", &copied.stderr));
    }
    let installed = run.run(
        container_root(engine, container, CONTAINER_INSTALL, &[&directory, &temporary]),
        None,
        "verifying container helper",
    )?;
    if installed.code != Some(0) {
        return Err(bootstrap_failure(
            "container helper is not a root-owned regular file",
            &installed.stderr,
        ));
    }
    Ok(())
}

/// Makes the helper available on an SSH host for the stdio agent, returning
/// its path there: a connection to check the cache, then the upload when it
/// has none. (A relay's launch checks for itself.)
fn deploy_ssh(run: &impl Run, target: &RemoteTarget, helper: &Helper) -> Result<String> {
    let remote = helper.ssh_path();
    let check = format!(
        "test -x \"{remote}\" && test \"$(\"{remote}\" --version)\" = {}",
        helper.version
    );
    let command = super::remote_command(target, "sh", &["-c".into(), check])?;
    let checked = run.run(command, None, "checking cached SSH remote helper")?;
    match checked.code {
        Some(0) => {}
        // ssh itself failed (auth, host key, network): uploading would too.
        Some(255) => return Err(bootstrap_failure("SSH connection failed", &checked.stderr)),
        _ => upload_ssh(run, target, helper)?,
    }
    Ok(remote)
}

/// The stdio agent's command, until task 6.4 removes it with the agent.
fn agent_command(target: &RemoteTarget, remote_agent: &str) -> Result<Command> {
    match target {
        RemoteTarget::Ssh { .. } => super::remote_command(
            target,
            "sh",
            // Not a login shell: a profile that prints would corrupt the
            // framed stdout stream.
            &["-c".into(), format!("exec \"{remote_agent}\" --server")],
        ),
        // The helper is the container's shared copy, which stays.
        RemoteTarget::Container { .. } => {
            super::remote_command(target, "env", &[remote_agent.into(), "--server".into()])
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::protocol::MSG_HEARTBEAT;
    use super::*;

    #[test]
    fn ssh_agent_launch_has_secret_free_argv() {
        let target = RemoteTarget::Ssh {
            host: "A8".into(),
            user: None,
            port: None,
            identity_file: None,
            ssh_args: Vec::new(),
        };
        let command = agent_command(&target, "$HOME/.cache/brindle/0.2.0/brindle-remote").unwrap();
        let args: Vec<_> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            &args[args.len() - 2..],
            [
                "A8",
                "sh -c 'exec \"$HOME/.cache/brindle/0.2.0/brindle-remote\" --server'",
            ]
        );
        assert!(!args.iter().any(|arg| arg.contains("request-json")));
    }

    // ---- the relay's launch and the deployment behind it ----

    use std::cell::RefCell;
    use std::collections::VecDeque;

    /// The cache directory of [`helper`] in a container.
    const CACHE: &str = "/tmp/brindle-helper-1.2.3-abc123";
    /// The SSH launch script for [`helper`].
    const SSH_LAUNCH: &str = r#"set -- "$HOME/.cache/brindle/1.2.3-abc123/brindle-remote"; if test -x "$1" && test "$("$1" --version)" = 1.2.3; then exec "$1" relay; fi; exit 97"#;
    /// The SSH upload script for [`helper`].
    const SSH_UPLOAD: &str = r#"umask 077; mkdir -p "$HOME/.cache/brindle/1.2.3-abc123" && tmp="$HOME/.cache/brindle/1.2.3-abc123/brindle-remote.tmp.$$" && cat > "$tmp" && chmod 700 "$tmp" && mv "$tmp" "$HOME/.cache/brindle/1.2.3-abc123/brindle-remote""#;

    fn helper() -> Helper {
        Helper {
            local: PathBuf::from("helpers/brindle-remote-linux"),
            version: "1.2.3",
            digest: "abc123".into(),
        }
    }

    fn ssh_host() -> RemoteTarget {
        RemoteTarget::Ssh {
            host: "A8".into(),
            user: Some("dev".into()),
            port: None,
            identity_file: None,
            ssh_args: Vec::new(),
        }
    }

    fn container_of(engine: &str, user: Option<&str>) -> RemoteTarget {
        RemoteTarget::Container {
            engine: engine.into(),
            container: "devbox".into(),
            user: user.map(Into::into),
        }
    }

    /// A command's program and arguments.
    fn line(command: &Command) -> Vec<String> {
        std::iter::once(command.get_program())
            .chain(command.get_args())
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect()
    }

    fn strings(words: &[&str]) -> Vec<String> {
        words.iter().map(|word| word.to_string()).collect()
    }

    /// `ssh` running `script` by the host's `sh`, for [`ssh_host`].
    fn ssh_line(script: &str) -> Vec<String> {
        let mut line = strings(&[
            "ssh",
            "-T",
            "-o",
            "ConnectTimeout=15",
            "-o",
            "ServerAliveInterval=5",
            "-o",
            "ServerAliveCountMax=3",
            "--",
            "dev@A8",
        ]);
        line.push(format!("sh -c '{script}'"));
        line
    }

    #[test]
    fn the_ssh_launch_runs_the_cached_relay_or_exits_97() {
        let command = launch_command(&ssh_host(), &helper()).unwrap();
        // One connection, and not a login shell.
        assert_eq!(line(&command), ssh_line(SSH_LAUNCH));
        assert_eq!(ssh_launch_script(&helper()), SSH_LAUNCH);
        assert!(SSH_LAUNCH.ends_with(&format!("; exit {NOT_CACHED}")));
        assert_eq!(ssh_upload_script(&helper()), SSH_UPLOAD);
    }

    #[test]
    fn a_container_launch_checks_the_cache_as_the_profiles_user() {
        let with_user = launch_command(&container_of("docker", Some("1000")), &helper()).unwrap();
        assert_eq!(
            line(&with_user),
            ["docker", "exec", "-i", "--user", "1000", "devbox", "sh", "-c", CONTAINER_LAUNCH, "sh", CACHE]
        );
        let without = launch_command(&container_of("podman", None), &helper()).unwrap();
        assert_eq!(
            line(&without),
            ["podman", "exec", "-i", "devbox", "sh", "-c", CONTAINER_LAUNCH, "sh", CACHE]
        );
        assert!(launch_command(&container_of("lxc", None), &helper()).is_err());
        assert_eq!(
            CONTAINER_LAUNCH,
            r#"test -d "$1" && test ! -L "$1" && test "$(stat -c %u "$1")" = 0 && test -f "$1/brindle-remote" && test ! -L "$1/brindle-remote" && test -x "$1/brindle-remote" && test "$(stat -c %u "$1/brindle-remote")" = 0 && exec "$1/brindle-remote" relay; exit 97"#
        );
        assert!(CONTAINER_LAUNCH.ends_with(&format!("; exit {NOT_CACHED}")));
    }

    #[test]
    fn roots_container_scripts_are_pinned() {
        assert_eq!(
            CONTAINER_DIRECTORY,
            r#"mkdir -m 755 "$1" 2>/dev/null && exit 0; if test -d "$1" && test ! -L "$1" && test "$(stat -c %u "$1")" = 0; then chmod 755 "$1"; exit; fi; if test -e "$1" || test -L "$1"; then exit 98; fi; mkdir -m 755 "$1""#
        );
        assert!(CONTAINER_DIRECTORY.contains(&format!("then exit {TAKEN}; fi")));
        assert_eq!(
            CONTAINER_INSTALL,
            r#"if test -f "$1/$2" && test ! -L "$1/$2" && chown 0:0 "$1/$2" && chmod 755 "$1/$2" && test "$(stat -c %u "$1/$2")" = 0 && mv -f "$1/$2" "$1/brindle-remote"; then :; else rm -f "$1/$2"; exit 1; fi; for d in "${1%/*}"/brindle-helper-*; do test "$d" != "$1" && test -d "$d" && test ! -L "$d" && test "$(stat -c %u "$d")" = 0 && rm -rf "$d"; done; exit 0"#
        );
    }

    /// An endpoint that runs nothing: it logs each launch and each
    /// deployment command, and ends each as the test scripted.
    #[derive(Default)]
    struct Scripted {
        log: RefCell<Vec<Step>>,
        launches: RefCell<VecDeque<std::result::Result<(), Unstarted>>>,
        runs: RefCell<VecDeque<(i32, &'static str)>>,
    }

    #[derive(Debug, PartialEq)]
    enum Step {
        Launch(Vec<String>),
        /// A deployment command and the file that is its stdin.
        Run(Vec<String>, Option<PathBuf>),
    }

    impl Scripted {
        /// How each launch ends, and each deployment command's exit code
        /// and stderr.
        fn new(
            launches: impl IntoIterator<Item = std::result::Result<(), Unstarted>>,
            runs: impl IntoIterator<Item = (i32, &'static str)>,
        ) -> Self {
            Self {
                log: RefCell::default(),
                launches: RefCell::new(launches.into_iter().collect()),
                runs: RefCell::new(runs.into_iter().collect()),
            }
        }

        fn launch(&self, target: &RemoteTarget) -> Result<()> {
            launch_with(self, target, &helper(), |command| {
                self.log.borrow_mut().push(Step::Launch(line(&command)));
                self.launches.borrow_mut().pop_front().expect("a launch the test didn't script")
            })
        }

        fn log(&self) -> Vec<Step> {
            self.log.take()
        }
    }

    impl Run for Scripted {
        fn run(&self, command: Command, input: Option<&Path>, _what: &str) -> Result<Ran> {
            self.log
                .borrow_mut()
                .push(Step::Run(line(&command), input.map(Path::to_path_buf)));
            let (code, stderr) = self.runs.borrow_mut().pop_front().expect("a command the test didn't script");
            Ok(Ran { code: Some(code), stderr: stderr.into() })
        }
    }

    /// A launch that exited with `code` before the relay's frame marker.
    fn exited(code: i32, said: &str) -> Unstarted {
        Unstarted {
            code: Some(code),
            said: said.into(),
            message: format!("the relay didn't start (exited with {code})\n{said}"),
        }
    }

    fn container_launch(user: Option<&str>) -> Step {
        let mut line = strings(&["docker", "exec", "-i"]);
        if let Some(user) = user {
            line.extend(strings(&["--user", user]));
        }
        line.extend(strings(&["devbox", "sh", "-c", CONTAINER_LAUNCH, "sh", CACHE]));
        Step::Launch(line)
    }

    fn root_step(script: &str, args: &[&str]) -> Vec<String> {
        let mut line = strings(&["docker", "exec", "--user", "0", "devbox", "sh", "-c", script, "sh"]);
        line.extend(strings(args));
        line
    }

    #[test]
    fn a_cached_helper_takes_one_command() {
        let host = Scripted::new([Ok(())], []);
        host.launch(&ssh_host()).unwrap();
        assert_eq!(host.log(), [Step::Launch(ssh_line(SSH_LAUNCH))]);

        let devbox = Scripted::new([Ok(())], []);
        devbox.launch(&container_of("docker", Some("dev"))).unwrap();
        assert_eq!(devbox.log(), [container_launch(Some("dev"))]);
    }

    #[test]
    fn a_host_without_the_helper_gets_it_uploaded_and_is_launched_again() {
        let host = Scripted::new([Err(exited(NOT_CACHED, "")), Ok(())], [(0, "")]);
        host.launch(&ssh_host()).unwrap();
        assert_eq!(
            host.log(),
            [
                Step::Launch(ssh_line(SSH_LAUNCH)),
                Step::Run(ssh_line(SSH_UPLOAD), Some(helper().local)),
                Step::Launch(ssh_line(SSH_LAUNCH)),
            ]
        );
    }

    #[test]
    fn a_container_without_the_helper_gets_it_from_root_and_is_launched_again() {
        let devbox = Scripted::new([Err(exited(NOT_CACHED, "")), Ok(())], [(0, ""), (0, ""), (0, "")]);
        devbox.launch(&container_of("docker", Some("dev"))).unwrap();
        let log = devbox.log();
        assert_eq!(log.len(), 5, "{log:?}");
        // The launch runs as the profile's user, the deployment as root.
        assert_eq!(log[0], container_launch(Some("dev")));
        assert_eq!(log[1], Step::Run(root_step(CONTAINER_DIRECTORY, &[CACHE]), None));
        let Step::Run(copy, None) = &log[2] else { panic!("{:?}", log[2]) };
        assert_eq!(copy[..3], ["docker", "cp", "helpers/brindle-remote-linux"]);
        // The copy has a name of its own, and is renamed into place.
        let temporary = copy[3]
            .strip_prefix(&format!("devbox:{CACHE}/"))
            .unwrap_or_else(|| panic!("{copy:?}"));
        let random = temporary.strip_prefix("brindle-remote.tmp-").unwrap_or_else(|| panic!("{temporary}"));
        assert!(random.len() == 16 && random.bytes().all(|b| b.is_ascii_hexdigit()), "{temporary}");
        assert_eq!(copy.len(), 4);
        assert_eq!(log[3], Step::Run(root_step(CONTAINER_INSTALL, &[CACHE, temporary]), None));
        assert_eq!(log[4], container_launch(Some("dev")));
        assert_ne!(temporary_name().unwrap(), temporary);
    }

    #[test]
    fn a_taken_container_directory_is_refused_and_nothing_more_runs() {
        let devbox = Scripted::new([Err(exited(NOT_CACHED, ""))], [(TAKEN, "")]);
        let error = devbox.launch(&container_of("docker", None)).unwrap_err().to_string();
        assert!(error.starts_with(&format!("{CACHE} exists in the container but isn't a directory owned by root")), "{error}");
        // No copy into it, nothing removed, and no second launch.
        assert_eq!(
            devbox.log(),
            [container_launch(None), Step::Run(root_step(CONTAINER_DIRECTORY, &[CACHE]), None)]
        );
    }

    #[test]
    fn an_ssh_failure_is_a_connection_error_and_not_a_missing_helper() {
        let said = "ssh: connect to host a8 port 22: Connection refused";
        let host = Scripted::new([Err(exited(255, said))], []);
        let error = host.launch(&ssh_host()).unwrap_err().to_string();
        assert_eq!(error, format!("SSH connection failed: {said}"));
        assert_eq!(host.log(), [Step::Launch(ssh_line(SSH_LAUNCH))]);

        // Any other exit is the launch's own failure, shown as it said it.
        let said = "Error response from daemon: No such container: devbox";
        let devbox = Scripted::new([Err(exited(1, said))], []);
        let error = devbox.launch(&container_of("docker", None)).unwrap_err();
        assert_eq!(error.downcast_ref::<Unstarted>().and_then(|unstarted| unstarted.code), Some(1));
        assert!(error.to_string().ends_with(said), "{error}");
        assert_eq!(devbox.log(), [container_launch(None)]);
        // A container's 255 is its own, too.
        let devbox = Scripted::new([Err(exited(255, "boom"))], []);
        let error = devbox.launch(&container_of("docker", None)).unwrap_err();
        assert!(error.downcast_ref::<Unstarted>().is_some(), "{error}");
    }

    #[test]
    fn a_helper_that_still_is_not_there_after_deploying_is_an_error() {
        let said = "sh: 1: /home/dev/.cache/brindle/1.2.3-abc123/brindle-remote: Exec format error";
        let host = Scripted::new([Err(exited(NOT_CACHED, "")), Err(exited(NOT_CACHED, said))], [(0, "")]);
        let error = host.launch(&ssh_host()).unwrap_err().to_string();
        assert!(error.starts_with("the helper was uploaded, but running it on the host doesn't print version 1.2.3"), "{error}");
        assert!(error.ends_with(said), "{error}");
        // Deployed once, launched twice.
        assert_eq!(host.log().len(), 3);

        let devbox = Scripted::new([Err(exited(NOT_CACHED, "")), Err(exited(NOT_CACHED, ""))], [(0, ""), (0, ""), (0, "")]);
        let error = devbox.launch(&container_of("docker", Some("dev"))).unwrap_err().to_string();
        assert!(error.starts_with("the helper was copied into the container"), "{error}");
        assert!(error.contains(CACHE), "{error}");
        assert_eq!(devbox.log().len(), 5);
    }

    #[test]
    fn a_failed_deployment_is_not_launched_again() {
        let host = Scripted::new([Err(exited(NOT_CACHED, ""))], [(1, "cat: write error: No space left on device\n")]);
        let error = host.launch(&ssh_host()).unwrap_err().to_string();
        assert_eq!(error, "SSH helper upload failed: cat: write error: No space left on device");
        assert_eq!(host.log().len(), 2);

        // Each of root's steps in a container.
        let steps: [(&[(i32, &str)], &str); 3] = [
            (&[(1, "mkdir: cannot create directory: Read-only file system")], "container helper directory failed: mkdir"),
            (&[(0, ""), (1, "no such container")], "container helper upload failed: no such container"),
            (&[(0, ""), (0, ""), (1, "")], "container helper is not a root-owned regular file"),
        ];
        for (runs, expected) in steps {
            let devbox = Scripted::new([Err(exited(NOT_CACHED, ""))], runs.iter().copied());
            let error = devbox.launch(&container_of("docker", None)).unwrap_err().to_string();
            assert!(error.starts_with(expected), "{error}");
            assert_eq!(devbox.log().len(), 1 + runs.len());
        }
    }

    #[test]
    fn the_stdio_agent_checks_an_ssh_cache_and_uses_the_containers_shared_helper() {
        // Cached: the check is all.
        let check = r#"test -x "$HOME/.cache/brindle/1.2.3-abc123/brindle-remote" && test "$("$HOME/.cache/brindle/1.2.3-abc123/brindle-remote" --version)" = 1.2.3"#;
        let host = Scripted::new([], [(0, "")]);
        let path = deploy_ssh(&host, &ssh_host(), &helper()).unwrap();
        assert_eq!(path, "$HOME/.cache/brindle/1.2.3-abc123/brindle-remote");
        assert_eq!(host.log(), [Step::Run(ssh_line(check), None)]);
        // Missing: uploaded.
        let host = Scripted::new([], [(1, ""), (0, "")]);
        deploy_ssh(&host, &ssh_host(), &helper()).unwrap();
        assert_eq!(
            host.log(),
            [Step::Run(ssh_line(check), None), Step::Run(ssh_line(SSH_UPLOAD), Some(helper().local))]
        );
        // ssh's own failure isn't a missing helper.
        let host = Scripted::new([], [(255, "Permission denied (publickey).")]);
        let error = deploy_ssh(&host, &ssh_host(), &helper()).unwrap_err().to_string();
        assert_eq!(error, "SSH connection failed: Permission denied (publickey).");
        assert_eq!(host.log().len(), 1);

        // The container's helper is shared, so the agent doesn't unlink it.
        let agent = format!("{CACHE}/brindle-remote");
        let command = agent_command(&container_of("docker", Some("dev")), &agent).unwrap();
        assert_eq!(
            line(&command),
            ["docker", "exec", "-i", "--user", "dev", "devbox", "env", agent.as_str(), "--server"]
        );
    }

    /// A command that copies its stdin to the file `to`, writes `said` to
    /// stderr and exits with `code`.
    fn copying(to: &Path, said: &str, code: i32) -> Command {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;

            let mut command = hidden_command("cmd");
            command.args(["/d", "/c"]).raw_arg(format!(
                "findstr \"^\" > \"{}\" & echo {said} 1>&2 & exit {code}",
                to.display()
            ));
            command
        }
        #[cfg(unix)]
        {
            let mut command = Command::new("sh");
            command
                .args(["-c", "cat > \"$1\"; echo \"$2\" >&2; exit \"$3\"", "sh"])
                .arg(to)
                .args([said, &code.to_string()]);
            command
        }
    }

    /// A command that waits a minute.
    fn waiting() -> Command {
        if cfg!(windows) {
            let mut command = hidden_command("powershell");
            command.args(["-NoProfile", "-NonInteractive", "-Command", "Start-Sleep 60"]);
            command
        } else {
            let mut command = Command::new("sleep");
            command.arg("60");
            command
        }
    }

    #[test]
    fn a_deployment_command_gets_its_input_and_says_how_it_ended() {
        let dir = std::env::temp_dir().join(format!("brindle-deploy-run-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let (input, copy) = (dir.join("helper"), dir.join("copy"));
        std::fs::write(&input, "the helper\n").unwrap();
        let never = AtomicBool::new(false);
        let commands = Commands { cancel: &never };

        // Its input is the file, and its exit code and stderr come back.
        let ran = commands.run(copying(&copy, "all of it", 0), Some(&input), "copying").unwrap();
        assert_eq!((ran.code, ran.stderr.trim()), (Some(0), "all of it"));
        assert_eq!(std::fs::read_to_string(&copy).unwrap().trim(), "the helper");
        let ran = commands.run(copying(&copy, "no space left", 3), Some(&input), "copying").unwrap();
        assert_eq!((ran.code, ran.stderr.trim()), (Some(3), "no space left"));
        // Without one, it reads nothing.
        let ran = commands.run(copying(&copy, "nothing", 0), None, "copying").unwrap();
        assert_eq!((ran.code, ran.stderr.trim()), (Some(0), "nothing"));
        assert_eq!(std::fs::read_to_string(&copy).unwrap(), "");

        // What can't run at all is an error naming it.
        let error = commands.run(Command::new("brindle-no-such-program"), None, "probing").unwrap_err();
        assert!(format!("{error:#}").starts_with("starting probing"), "{error:#}");
        let error = commands
            .run(copying(&copy, "unread", 0), Some(&dir.join("missing")), "copying")
            .unwrap_err();
        assert!(format!("{error:#}").starts_with("opening remote helper"), "{error:#}");

        // A cancelled command is ended at once, and doesn't count as done.
        let cancelled = AtomicBool::new(true);
        let started = Instant::now();
        let ran = Commands { cancel: &cancelled }.run(waiting(), None, "waiting").unwrap();
        assert_ne!(ran.code, Some(0));
        assert!(started.elapsed() < Duration::from_secs(30), "{:?}", started.elapsed());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The scripts as a real `sh` runs them, in a directory of the test's
    /// own. Ownership is the one thing faked, by a `stat` and a `chown`
    /// first on `PATH`, since tests don't run as root; Docker's opt-in
    /// scenarios below check the real thing. They run on Linux, which the
    /// scripts are for, and pass unrun elsewhere.
    mod scripts {
        use std::process::Output;

        use super::*;

        /// Files matching this aren't root's, to the fake `stat`.
        const THEIRS: &str = "*-theirs*";

        struct Sandbox(PathBuf);

        impl Sandbox {
            fn new(tag: &str) -> Option<Self> {
                if !cfg!(target_os = "linux") {
                    return None;
                }
                let root = std::env::temp_dir().join(format!("brindle-deploy-{tag}-{}", std::process::id()));
                std::fs::remove_dir_all(&root).ok();
                std::fs::create_dir_all(&root).unwrap();
                let sandbox = Self(root);
                // `stat -c %u FILE`: root's, unless FILE matches $THEIRS.
                sandbox.executable("bin/stat", "#!/bin/sh\ncase \"$3\" in $THEIRS) echo 1000;; *) echo 0;; esac\n");
                sandbox.executable("bin/chown", "#!/bin/sh\nexit 0\n");
                Some(sandbox)
            }

            fn path(&self, name: &str) -> PathBuf {
                self.0.join(name)
            }

            fn text(&self, name: &str) -> String {
                self.path(name).to_string_lossy().into_owned()
            }

            /// Runs `script` with `args` by `sh`, with the tools as they are.
            fn plain(&self, script: &str, args: &[&str]) -> String {
                let output = Command::new("sh")
                    .arg("-c")
                    .arg(script)
                    .arg("sh")
                    .args(args)
                    .stdin(Stdio::null())
                    .output()
                    .unwrap();
                assert!(output.status.success(), "{script} {args:?}: {output:?}");
                String::from_utf8_lossy(&output.stdout).into_owned()
            }

            /// Runs `script` with `args` by `sh` as an endpoint would, where
            /// files matching the pattern `theirs` aren't root's.
            fn sh(&self, script: &str, args: &[&str], theirs: &str) -> Output {
                let own = std::env::var_os("PATH").unwrap_or_default();
                let path = std::env::join_paths(
                    std::iter::once(self.path("bin")).chain(std::env::split_paths(&own)),
                )
                .unwrap();
                Command::new("sh")
                    .arg("-c")
                    .arg(script)
                    .arg("sh")
                    .args(args)
                    .env("PATH", path)
                    .env("THEIRS", theirs)
                    .env("HOME", self.path("home"))
                    // A variable of the user's that a script could clobber.
                    .env("h", "the user's")
                    .stdin(Stdio::null())
                    .output()
                    .unwrap()
            }

            /// Writes an executable file. The shell writes it, so that no
            /// child another test forks holds it open for writing while it
            /// runs (`ETXTBSY`).
            fn executable(&self, name: &str, contents: &str) {
                let write = r#"mkdir -p "${1%/*}" && printf %s "$2" > "$1" && chmod 755 "$1""#;
                self.plain(write, &[&self.text(name), contents]);
            }

            /// A helper that says `version`, and what it was run with.
            fn helper(&self, name: &str, version: &str) {
                let script = format!(
                    "#!/bin/sh\nif test \"$1\" = --version; then echo {version}; else echo \"ran $* h=$h\"; fi\n"
                );
                self.executable(name, &script);
            }

            fn symlink(&self, target: &str, link: &str) {
                self.plain(r#"ln -s "$1" "$2""#, &[&self.text(target), &self.text(link)]);
            }

            fn mode(&self, name: &str) -> String {
                self.plain(r#"stat -c %a "$1""#, &[&self.text(name)]).trim().to_string()
            }

            fn chmod(&self, name: &str, mode: &str) {
                self.plain(r#"chmod "$1" "$2""#, &[mode, &self.text(name)]);
            }

            /// The names in a directory.
            fn names(&self, directory: &str) -> Vec<String> {
                let mut names: Vec<String> = std::fs::read_dir(self.path(directory))
                    .unwrap()
                    .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                    .collect();
                names.sort();
                names
            }
        }

        impl Drop for Sandbox {
            fn drop(&mut self) {
                std::fs::remove_dir_all(&self.0).ok();
            }
        }

        /// A script's exit code and what it printed.
        fn out(output: &Output) -> (Option<i32>, String) {
            (output.status.code(), String::from_utf8_lossy(&output.stdout).into_owned())
        }

        /// A script that ran nothing and said the helper isn't cached.
        fn not_cached() -> (Option<i32>, String) {
            (Some(NOT_CACHED), String::new())
        }

        #[test]
        fn the_ssh_launch_script_runs_the_cached_helper_or_exits_97() {
            let Some(sandbox) = Sandbox::new("ssh-launch") else { return };
            let script = ssh_launch_script(&helper());
            let cached = "home/.cache/brindle/1.2.3-abc123/brindle-remote";
            // Nothing cached.
            assert_eq!(out(&sandbox.sh(&script, &[], THEIRS)), not_cached());
            // Cached: the relay runs, with the user's environment as it was.
            sandbox.helper(cached, "1.2.3");
            assert_eq!(out(&sandbox.sh(&script, &[], THEIRS)), (Some(0), "ran relay h=the user's\n".into()));
            // Not executable, or another version: not this build's helper.
            sandbox.chmod(cached, "644");
            assert_eq!(out(&sandbox.sh(&script, &[], THEIRS)), not_cached());
            sandbox.helper(cached, "1.2.2");
            assert_eq!(out(&sandbox.sh(&script, &[], THEIRS)), not_cached());
        }

        #[test]
        fn the_ssh_upload_script_caches_its_stdin_for_the_user_only() {
            let Some(sandbox) = Sandbox::new("ssh-upload") else { return };
            std::fs::write(sandbox.path("source"), "the helper").unwrap();
            let uploaded = Command::new("sh")
                .arg("-c")
                .arg(ssh_upload_script(&helper()))
                .env("HOME", sandbox.path("home"))
                .stdin(File::open(sandbox.path("source")).unwrap())
                .output()
                .unwrap();
            assert!(uploaded.status.success(), "{uploaded:?}");
            let directory = "home/.cache/brindle/1.2.3-abc123";
            let cached = format!("{directory}/brindle-remote");
            assert_eq!(std::fs::read_to_string(sandbox.path(&cached)).unwrap(), "the helper");
            assert_eq!((sandbox.mode(directory).as_str(), sandbox.mode(&cached).as_str()), ("700", "700"));
            // Nothing is left under a temporary name.
            assert_eq!(sandbox.names(directory), ["brindle-remote"]);
        }

        #[test]
        fn the_container_launch_script_runs_only_roots_helper() {
            let Some(sandbox) = Sandbox::new("container-launch") else { return };
            let cache = "brindle-helper-1.2.3-abc123";
            let helper = format!("{cache}/brindle-remote");
            let launch = |directory: &str, theirs: &str| {
                out(&sandbox.sh(CONTAINER_LAUNCH, &[&sandbox.text(directory)], theirs))
            };
            // No directory, then no helper in it.
            assert_eq!(launch(cache, THEIRS), not_cached());
            std::fs::create_dir(sandbox.path(cache)).unwrap();
            assert_eq!(launch(cache, THEIRS), not_cached());
            // Root's directory and root's helper.
            sandbox.helper(&helper, "1.2.3");
            assert_eq!(launch(cache, THEIRS), (Some(0), "ran relay h=the user's\n".into()));
            // A directory, or a helper, that isn't root's.
            assert_eq!(launch(cache, &sandbox.text(cache)), not_cached());
            assert_eq!(launch(cache, "*/brindle-remote"), not_cached());
            // A symbolic link to root's directory, or to root's helper.
            sandbox.symlink(cache, "brindle-helper-link");
            assert_eq!(launch("brindle-helper-link", THEIRS), not_cached());
            std::fs::create_dir(sandbox.path("brindle-helper-linked")).unwrap();
            sandbox.symlink(&helper, "brindle-helper-linked/brindle-remote");
            assert_eq!(launch("brindle-helper-linked", THEIRS), not_cached());
            // A helper that can't be run.
            sandbox.chmod(&helper, "644");
            assert_eq!(launch(cache, THEIRS), not_cached());
        }

        #[test]
        fn the_directory_script_makes_or_keeps_roots_directory_and_refuses_any_other() {
            let Some(sandbox) = Sandbox::new("container-directory") else { return };
            let run = |name: &str| sandbox.sh(CONTAINER_DIRECTORY, &[&sandbox.text(name)], THEIRS);
            let done = (Some(0), String::new());
            let taken = (Some(TAKEN), String::new());
            // Made.
            assert_eq!(out(&run("made")), done);
            assert_eq!(sandbox.mode("made"), "755");
            // Root's already: kept, with its mode put right.
            sandbox.chmod("made", "700");
            std::fs::write(sandbox.path("made/kept"), "").unwrap();
            assert_eq!(out(&run("made")), done);
            assert_eq!(sandbox.mode("made"), "755");
            assert_eq!(sandbox.names("made"), ["kept"]);
            // Someone else's directory: refused, and as it was.
            std::fs::create_dir(sandbox.path("dir-theirs")).unwrap();
            std::fs::write(sandbox.path("dir-theirs/canary"), "").unwrap();
            sandbox.chmod("dir-theirs", "700");
            assert_eq!(out(&run("dir-theirs")), taken);
            assert_eq!(sandbox.mode("dir-theirs"), "700");
            assert_eq!(sandbox.names("dir-theirs"), ["canary"]);
            // A symbolic link, to root's directory or to nothing.
            sandbox.symlink("made", "link");
            assert_eq!(out(&run("link")), taken);
            sandbox.symlink("nowhere", "dangling");
            assert_eq!(out(&run("dangling")), taken);
            assert!(!sandbox.path("nowhere").exists());
            // A file.
            std::fs::write(sandbox.path("file"), "theirs").unwrap();
            assert_eq!(out(&run("file")), taken);
            assert_eq!(std::fs::read_to_string(sandbox.path("file")).unwrap(), "theirs");
            // Nowhere to make it: mkdir's own failure, with its message.
            let failed = run("missing/made");
            assert!(!matches!(failed.status.code(), Some(0) | Some(TAKEN)), "{failed:?}");
            assert!(!failed.stderr.is_empty());
        }

        #[test]
        fn the_install_script_puts_the_helper_in_place_and_removes_roots_other_builds() {
            let Some(sandbox) = Sandbox::new("container-install") else { return };
            let cache = "brindle-helper-1.2.3-abc123";
            let helper = format!("{cache}/brindle-remote");
            let install = |copy: &str| out(&sandbox.sh(CONTAINER_INSTALL, &[&sandbox.text(cache), copy], THEIRS));
            let failed = (Some(1), String::new());
            std::fs::create_dir(sandbox.path(cache)).unwrap();
            std::fs::write(sandbox.path(&helper), "old").unwrap();
            // Beside it: root's other build, someone else's, a link, a file,
            // and directories that aren't a helper's.
            for other in ["brindle-helper-1.2.2-old", "brindle-helper-theirs", "elsewhere", "other"] {
                std::fs::create_dir(sandbox.path(other)).unwrap();
                std::fs::write(sandbox.path(other).join("kept"), "").unwrap();
            }
            sandbox.symlink("elsewhere", "brindle-helper-link");
            std::fs::write(sandbox.path("brindle-helper-file"), "").unwrap();
            let beside = sandbox.names("");

            // A copy that is missing, isn't root's or is a symbolic link is
            // removed, and nothing else changes.
            assert_eq!(install("brindle-remote.tmp-none"), failed);
            std::fs::write(sandbox.path(&format!("{cache}/brindle-remote.tmp-theirs")), "new").unwrap();
            assert_eq!(install("brindle-remote.tmp-theirs"), failed);
            sandbox.symlink("elsewhere/kept", &format!("{cache}/brindle-remote.tmp-link"));
            assert_eq!(install("brindle-remote.tmp-link"), failed);
            assert_eq!(sandbox.names(cache), ["brindle-remote"]);
            assert_eq!(std::fs::read_to_string(sandbox.path(&helper)).unwrap(), "old");
            assert_eq!(sandbox.names(""), beside);
            assert_eq!(sandbox.names("elsewhere"), ["kept"]);

            // The copy becomes the helper, and root's other build goes.
            std::fs::write(sandbox.path(&format!("{cache}/brindle-remote.tmp-1")), "new").unwrap();
            sandbox.chmod(&format!("{cache}/brindle-remote.tmp-1"), "600");
            assert_eq!(install("brindle-remote.tmp-1"), (Some(0), String::new()));
            assert_eq!(std::fs::read_to_string(sandbox.path(&helper)).unwrap(), "new");
            assert_eq!(sandbox.mode(&helper), "755");
            assert_eq!(sandbox.names(cache), ["brindle-remote"]);
            let left: Vec<String> = beside.into_iter().filter(|name| name != "brindle-helper-1.2.2-old").collect();
            assert_eq!(sandbox.names(""), left);
            for kept in ["brindle-helper-theirs", "elsewhere", "other"] {
                assert_eq!(sandbox.names(kept), ["kept"], "{kept}");
            }
            assert!(sandbox.path("brindle-helper-link").is_symlink());
        }
    }

    // ---- real-transport integration test (opt-in: BRINDLE_REMOTE_DOCKER_TEST=1) ----

    use futures::channel::mpsc::Receiver as EventReceiver;

    struct Container(String);

    impl Container {
        fn start(image: &str, tag: &str) -> std::result::Result<Self, String> {
            Self::start_with(image, tag, &[])
        }

        fn start_with(
            image: &str,
            tag: &str,
            extra: &[&str],
        ) -> std::result::Result<Self, String> {
            let name = format!("brindle-client-test-{}-{tag}", std::process::id());
            // Own the name first so a half-started container is still removed.
            let guard = Self(name.clone());
            let out = Command::new("docker")
                .args(["run", "-d", "--rm", "--name", &name])
                .args(extra)
                .args([image, "sleep", "600"])
                .output()
                .map_err(|e| format!("docker run: {e}"))?;
            if !out.status.success() {
                return Err(format!(
                    "docker run {image}: {}",
                    String::from_utf8_lossy(&out.stderr)
                ));
            }
            Ok(guard)
        }

        fn exec_sh(&self, script: &str) -> String {
            let out = Command::new("docker")
                .args(["exec", &self.0, "sh", "-c", script])
                .output()
                .expect("docker exec");
            String::from_utf8_lossy(&out.stdout).into_owned()
        }
    }

    impl Drop for Container {
        fn drop(&mut self) {
            Command::new("docker")
                .args(["rm", "-f", &self.0])
                .output()
                .ok();
        }
    }

    fn container_profile(container: &str) -> Profile {
        Profile {
            remote: Some(RemoteTarget::Container {
                engine: "docker".into(),
                container: container.into(),
                user: None,
            }),
            ..Profile::default_shell()
        }
    }

    fn test_size(cols: u16, rows: u16) -> GridSize {
        GridSize { cols, rows, cell_width: 8, cell_height: 16 }
    }

    #[derive(Debug, PartialEq)]
    enum End {
        Exited,
        Failed,
        Closed,
        /// Neither a terminal event nor the awaited output arrived in time.
        TimedOut,
        /// The awaited output arrived.
        Matched,
    }

    /// Reads events until `stop(output)` holds on the accumulated output, a
    /// terminal event arrives, or the timeout passes.
    fn pump(
        rx: &mut EventReceiver<Event>,
        output: &mut String,
        timeout: Duration,
        stop: impl Fn(&str) -> bool,
        infos: &mut Vec<ProcessInfo>,
    ) -> End {
        let deadline = Instant::now() + timeout;
        loop {
            if stop(output) {
                return End::Matched;
            }
            match rx.try_next() {
                Ok(Some(Event::Output(bytes))) => output.push_str(&String::from_utf8_lossy(&bytes)),
                Ok(Some(Event::ProcessInfo(info))) => infos.push(info),
                Ok(Some(Event::Exited)) => return End::Exited,
                Ok(Some(Event::Failed)) => return End::Failed,
                Ok(None) => return End::Closed,
                Err(_) => {
                    if Instant::now() >= deadline {
                        return End::TimedOut;
                    }
                    thread::sleep(Duration::from_millis(10));
                }
            }
        }
    }

    fn wait_for(rx: &mut EventReceiver<Event>, output: &mut String, needle: &str) -> bool {
        let needle = needle.to_string();
        let end = pump(rx, output, Duration::from_secs(20), |o| o.contains(&needle), &mut Vec::new());
        end == End::Matched
    }

    /// Runs to the terminal event; returns it with all output.
    fn run_to_end(rx: &mut EventReceiver<Event>, timeout: Duration) -> (End, String) {
        let mut output = String::new();
        let end = pump(rx, &mut output, timeout, |_| false, &mut Vec::new());
        (end, output)
    }

    fn connect_command(
        profile: Profile,
        command: &str,
        args: &[&str],
        size: GridSize,
    ) -> (Handle, EventReceiver<Event>) {
        let (tx, rx) = futures::channel::mpsc::channel(1024);
        let handle = connect(
            profile,
            Some((command.into(), args.iter().map(|a| a.to_string()).collect())),
            None,
            size,
            tx,
        );
        (handle, rx)
    }

    /// Checks that the container keeps the helper `BRINDLE_REMOTE_AGENT`
    /// names, in root's own directory, and nothing else of Brindle's: no
    /// other build, no copy left under a temporary name.
    fn scenario_one_helper_kept(c: &Container) -> std::result::Result<(), String> {
        let directory = Helper::find().map_err(|e| format!("{e:#}"))?.container_dir();
        let expected = format!("{directory} 0 755\n{directory}/brindle-remote 0 755\n");
        let kept = c.exec_sh("stat -c '%n %u %a' /tmp/brindle-* /tmp/brindle-*/* 2>&1");
        if kept == expected {
            Ok(())
        } else {
            Err(format!("kept {kept:?}, expected {expected:?}"))
        }
    }

    fn scenario_clean_exit(c: &Container, rounds: usize) -> std::result::Result<(), String> {
        for round in 0..rounds {
            let (_h, mut rx) = connect_command(
                container_profile(&c.0),
                "sh",
                &["-c", "printf READY; exit 0"],
                test_size(80, 24),
            );
            let (end, out) = run_to_end(&mut rx, Duration::from_secs(30));
            if end != End::Exited || !out.contains("READY") {
                return Err(format!("round {round}: end={end:?} output={out:?}"));
            }
        }
        Ok(())
    }

    fn scenario_missing_container() -> std::result::Result<(), String> {
        let (_h, mut rx) = connect_command(
            container_profile("brindle-no-such-container-xyz"),
            "sh",
            &[],
            test_size(80, 24),
        );
        let (end, out) = run_to_end(&mut rx, Duration::from_secs(30));
        if end != End::Failed || !out.to_lowercase().contains("no such container") {
            return Err(format!("end={end:?} output={out:?}"));
        }
        Ok(())
    }

    fn scenario_kill() -> std::result::Result<(), String> {
        let c = Container::start("ubuntu:24.04", "kill")?;
        let (h, mut rx) = connect_command(container_profile(&c.0), "sh", &[], test_size(80, 24));
        let mut out = String::new();
        h.input(b"echo up-$((1+1))\n".to_vec());
        if !wait_for(&mut rx, &mut out, "up-2\r\n") {
            return Err(format!("session never came up: {out:?}"));
        }
        Command::new("docker").args(["kill", &c.0]).output().ok();
        let (end, rest) = run_to_end(&mut rx, Duration::from_secs(30));
        out.push_str(&rest);
        if end != End::Failed || !out.contains("connection lost") {
            return Err(format!("end={end:?} output={out:?}"));
        }
        Ok(())
    }

    fn scenario_round_trip_resize_info(c: &Container) -> std::result::Result<(), String> {
        let (h, mut rx) = connect_command(container_profile(&c.0), "sh", &[], test_size(80, 24));
        let mut out = String::new();
        h.input(b"echo round-trip-$((1+1))\n".to_vec());
        if !wait_for(&mut rx, &mut out, "round-trip-2\r\n") {
            return Err(format!("no round trip: {out:?}"));
        }
        h.resize(test_size(120, 40));
        thread::sleep(Duration::from_millis(500));
        out.clear();
        h.input(b"stty size\n".to_vec());
        if !wait_for(&mut rx, &mut out, "40 120\r\n") {
            return Err(format!("resize not applied: {out:?}"));
        }
        h.query_process();
        let mut infos = Vec::new();
        let deadline = Instant::now() + Duration::from_secs(10);
        while infos.is_empty() && Instant::now() < deadline {
            pump(&mut rx, &mut out, Duration::from_millis(200), |_| false, &mut infos);
        }
        if !infos
            .iter()
            .any(|i| i.foreground_name.as_deref().is_some_and(|n| !n.is_empty()))
        {
            return Err(format!("no foreground name: {infos:?}"));
        }
        h.input(b"exit\n".to_vec());
        let (end, rest) = run_to_end(&mut rx, Duration::from_secs(20));
        if end != End::Exited {
            return Err(format!("end={end:?} output={rest:?}"));
        }
        Ok(())
    }

    fn scenario_bad_helper(c: &Container, dir: &Path) -> std::result::Result<(), String> {
        let bad = dir.join("not-a-helper");
        std::fs::write(&bad, b"this is not an executable\n").map_err(|e| e.to_string())?;
        unsafe { std::env::set_var("BRINDLE_REMOTE_AGENT", &bad) };
        let (_h, mut rx) = connect_command(container_profile(&c.0), "sh", &[], test_size(80, 24));
        let (end, out) = run_to_end(&mut rx, Duration::from_secs(30));
        if end != End::Failed || !out.contains("/brindle-remote: ") {
            return Err(format!("end={end:?} output={out:?}"));
        }
        Ok(())
    }

    #[test]
    fn docker_transport_scenarios() {
        if std::env::var_os("BRINDLE_REMOTE_DOCKER_TEST").is_none_or(|v| v != "1") {
            return;
        }
        let _turn = DOCKER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let started = Instant::now();
        let helper = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/x86_64-unknown-linux-musl/release/brindle-remote");
        assert!(
            helper.is_file(),
            "{} not found; build it with the docker command in README.md",
            helper.display()
        );
        unsafe { std::env::set_var("BRINDLE_REMOTE_AGENT", &helper) };
        // Under target/, not the system temp directory: a snap Docker's
        // `docker cp` can't see the host's /tmp.
        let scratch = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/tmp")
            .join(format!("brindle-client-test-{}", std::process::id()));
        std::fs::create_dir_all(&scratch).unwrap();
        let ubuntu = Container::start("ubuntu:24.04", "main").unwrap();
        let mut failures = Vec::new();
        let mut record = |name: &str, result: std::result::Result<(), String>| {
            eprintln!("scenario {name}: {}", if result.is_ok() { "ok" } else { "FAILED" });
            if let Err(e) = result {
                failures.push(format!("{name}: {e}"));
            }
        };
        record("a clean exit x20", scenario_clean_exit(&ubuntu, 20));
        record("b missing container", scenario_missing_container());
        record("c docker kill", scenario_kill());
        record("e/f round trip, resize, process info", scenario_round_trip_resize_info(&ubuntu));
        record("d the one helper is kept, root's own", scenario_one_helper_kept(&ubuntu));
        record("g bad helper", scenario_bad_helper(&ubuntu, &scratch));
        // Deploying another build removed the first one's directory.
        record("g the other build is all that is kept", scenario_one_helper_kept(&ubuntu));
        unsafe { std::env::set_var("BRINDLE_REMOTE_AGENT", &helper) };
        match Container::start("alpine", "alpine") {
            Ok(alpine) => {
                // busybox's sh, stat, mkdir and mv run the scripts here.
                record("h alpine run", scenario_clean_exit(&alpine, 1));
                record("h alpine keeps the helper, root's own", scenario_one_helper_kept(&alpine));
            }
            Err(e) => eprintln!("scenario h skipped: {e}"),
        }
        std::fs::remove_dir_all(&scratch).ok();
        assert!(failures.is_empty(), "failures:\n{}", failures.join("\n"));
        assert!(started.elapsed() < Duration::from_secs(120), "took {:?}", started.elapsed());
    }

    // ---- real OpenSSH + flood/Ctrl-C/orphan scenarios (same opt-in) ----

    /// The Docker tests change `BRINDLE_REMOTE_AGENT`, so they take turns.
    static DOCKER_TEST_LOCK: Mutex<()> = Mutex::new(());

    const FIXTURE_TAG: &str = "brindle-client-test/ssh-fixture:1";

    fn ssh_target(port: u16, key: &Path) -> RemoteTarget {
        // An empty config and a throwaway known_hosts, so the machine's own
        // ssh configuration cannot interfere (no portable null device).
        let config = key.with_file_name("ssh_config");
        std::fs::write(&config, "").unwrap();
        let config = config.to_string_lossy().into_owned();
        let known_hosts = key.with_file_name("known_hosts").to_string_lossy().into_owned();
        RemoteTarget::Ssh {
            host: "127.0.0.1".into(),
            user: Some("root".into()),
            port: Some(port),
            identity_file: Some(key.to_string_lossy().into_owned()),
            ssh_args: [
                "-F",
                &config,
                "-o",
                "BatchMode=yes",
                "-o",
                "IdentitiesOnly=yes",
                "-o",
                "StrictHostKeyChecking=no",
                "-o",
                &format!("UserKnownHostsFile={known_hosts}"),
                "-o",
                "LogLevel=ERROR",
            ]
            .into_iter()
            .map(String::from)
            .collect(),
        }
    }

    fn profile_for(target: &RemoteTarget) -> Profile {
        Profile { remote: Some(target.clone()), ..Profile::default_shell() }
    }

    fn generate_key(path: &Path) -> std::result::Result<(), String> {
        std::fs::remove_file(path).ok();
        std::fs::remove_file(path.with_extension("pub")).ok();
        let out = Command::new("ssh-keygen")
            .args(["-q", "-t", "ed25519", "-N", "", "-f"])
            .arg(path)
            .output()
            .map_err(|e| format!("ssh-keygen: {e}"))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!("ssh-keygen: {}", String::from_utf8_lossy(&out.stderr)))
        }
    }

    struct SshFixture {
        container: Container,
        target: RemoteTarget,
        scratch: PathBuf,
    }

    impl SshFixture {
        /// Starts the fixture in a container named with `tag`. Each test
        /// has a tag of its own, so one whose container is still being
        /// removed is never in the next one's way.
        fn start(scratch: &Path, tag: &str) -> std::result::Result<Self, String> {
            let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/remote");
            let build = Command::new("docker")
                .args(["build", "-q", "-t", FIXTURE_TAG])
                .arg(&fixture_dir)
                .output()
                .map_err(|e| format!("docker build: {e}"))?;
            if !build.status.success() {
                return Err(format!("docker build: {}", String::from_utf8_lossy(&build.stderr)));
            }
            let container = Container::start_with(FIXTURE_TAG, tag, &["-p", "127.0.0.1::22"])?;
            let key = scratch.join("id_test");
            generate_key(&key)?;
            let copy = Command::new("docker")
                .args(["cp"])
                .arg(key.with_extension("pub"))
                .arg(format!("{}:/root/.ssh/authorized_keys", container.0))
                .output()
                .map_err(|e| format!("docker cp: {e}"))?;
            if !copy.status.success() {
                return Err(format!("docker cp: {}", String::from_utf8_lossy(&copy.stderr)));
            }
            container.exec_sh(
                "chown root:root /root/.ssh/authorized_keys; chmod 600 /root/.ssh/authorized_keys; chmod 700 /root/.ssh",
            );
            let sshd = Command::new("docker")
                .args(["exec", "-d", &container.0, "/usr/sbin/sshd", "-D", "-e"])
                .output()
                .map_err(|e| format!("starting sshd: {e}"))?;
            if !sshd.status.success() {
                return Err(format!("starting sshd: {}", String::from_utf8_lossy(&sshd.stderr)));
            }
            let port_out = Command::new("docker")
                .args(["port", &container.0, "22/tcp"])
                .output()
                .map_err(|e| format!("docker port: {e}"))?;
            let text = String::from_utf8_lossy(&port_out.stdout).into_owned();
            let port: u16 = text
                .lines()
                .find_map(|line| line.trim().rsplit_once(':')?.1.parse().ok())
                .ok_or_else(|| format!("no published ssh port: {text:?}"))?;
            let target = ssh_target(port, &key);
            let fixture = Self { container, target, scratch: scratch.to_path_buf() };
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                let mut probe = super::super::remote_command(
                    &fixture.target,
                    "printf",
                    &["BRINDLE_SSH_READY".into()],
                )
                .map_err(|e| e.to_string())?;
                let out = probe.output().map_err(|e| format!("ssh probe: {e}"))?;
                if String::from_utf8_lossy(&out.stdout) == "BRINDLE_SSH_READY" {
                    return Ok(fixture);
                }
                if Instant::now() >= deadline {
                    return Err(format!(
                        "sshd never became ready: {}",
                        String::from_utf8_lossy(&out.stderr)
                    ));
                }
                thread::sleep(Duration::from_millis(300));
            }
        }
    }

    fn cache_stat(c: &Container) -> String {
        c.exec_sh("stat -c '%n %i %y' /root/.cache/brindle/*/brindle-remote 2>&1; ls -A /root/.cache/brindle/*/ 2>&1")
    }

    fn scenario_ssh_upload_then_reuse(fx: &SshFixture) -> std::result::Result<(), String> {
        let c = &fx.container;
        if !c.exec_sh("test -e /root/.cache/brindle && echo present").trim().is_empty() {
            return Err("cache dir existed before the first connect".into());
        }
        let mut stats = Vec::new();
        for round in 0..2 {
            let (_h, mut rx) = connect_command(
                profile_for(&fx.target),
                "sh",
                &["-c", &format!("printf READY{round}; exit 0")],
                test_size(80, 24),
            );
            let (end, out) = run_to_end(&mut rx, Duration::from_secs(60));
            if end != End::Exited || !out.contains(&format!("READY{round}")) {
                return Err(format!("round {round}: end={end:?} output={out:?}"));
            }
            stats.push(cache_stat(c));
            thread::sleep(Duration::from_millis(1200));
        }
        let version = env!("CARGO_PKG_VERSION");
        if !stats[0].contains(&format!(".cache/brindle/{version}-")) {
            return Err(format!("helper not cached under <version>-<digest>: {:?}", stats[0]));
        }
        if stats[0] != stats[1] {
            return Err(format!("helper was re-uploaded: {:?} vs {:?}", stats[0], stats[1]));
        }
        if stats[0].contains(".tmp") {
            return Err(format!("temporary upload file left behind: {:?}", stats[0]));
        }
        Ok(())
    }

    fn scenario_ssh_round_trip(fx: &SshFixture) -> std::result::Result<(), String> {
        let (h, mut rx) = connect_command(profile_for(&fx.target), "sh", &[], test_size(80, 24));
        let mut out = String::new();
        h.input(b"echo ssh-trip-$((20+22))\n".to_vec());
        if !wait_for(&mut rx, &mut out, "ssh-trip-42\r\n") {
            return Err(format!("no round trip: {out:?}"));
        }
        h.input(b"exit\n".to_vec());
        let (end, rest) = run_to_end(&mut rx, Duration::from_secs(20));
        if end != End::Exited {
            return Err(format!("end={end:?} output={rest:?}"));
        }
        Ok(())
    }

    fn scenario_ssh_wrong_key(fx: &SshFixture) -> std::result::Result<(), String> {
        let wrong = fx.scratch.join("id_wrong");
        generate_key(&wrong)?;
        let RemoteTarget::Ssh { port, .. } = &fx.target else { unreachable!() };
        let target = ssh_target(port.unwrap(), &wrong);
        let (_h, mut rx) = connect_command(profile_for(&target), "sh", &[], test_size(80, 24));
        let (end, out) = run_to_end(&mut rx, Duration::from_secs(60));
        std::fs::remove_file(&wrong).ok();
        std::fs::remove_file(wrong.with_extension("pub")).ok();
        if end != End::Failed || !out.contains("Permission denied") {
            return Err(format!("end={end:?} output={out:?}"));
        }
        Ok(())
    }

    fn noise_session(fx: &SshFixture) -> (End, String) {
        let (_h, mut rx) = connect_command(
            profile_for(&fx.target),
            "sh",
            &["-c", "printf READY-NOISE; exit 0"],
            test_size(80, 24),
        );
        run_to_end(&mut rx, Duration::from_secs(60))
    }

    fn scenario_ssh_profile_noise(fx: &SshFixture) -> std::result::Result<(), String> {
        let c = &fx.container;
        c.exec_sh("cp /root/.profile /root/.profile.orig; echo 'echo NOISE_PROFILE' >> /root/.profile");
        let (end, out) = noise_session(fx);
        c.exec_sh("mv /root/.profile.orig /root/.profile");
        if end != End::Exited || !out.contains("READY-NOISE") || out.contains("NOISE_PROFILE") {
            return Err(format!("end={end:?} output={out:?}"));
        }
        Ok(())
    }

    /// A noisy ~/.bashrc prints before the helper starts; the frame marker
    /// lets the client skip it, so the session works and the noise never
    /// reaches the terminal.
    fn scenario_ssh_bashrc_noise(fx: &SshFixture) -> std::result::Result<(), String> {
        let c = &fx.container;
        c.exec_sh(
            "cp /root/.bashrc /root/.bashrc.orig; printf 'echo NOISE_BASHRC\\n' | cat - /root/.bashrc.orig > /root/.bashrc",
        );
        let (end, out) = noise_session(fx);
        c.exec_sh("mv /root/.bashrc.orig /root/.bashrc");
        if end != End::Exited || !out.contains("READY-NOISE") || out.contains("NOISE_BASHRC") {
            return Err(format!("end={end:?} output={out:?}"));
        }
        Ok(())
    }

    /// A framed session driven by hand, so a test can kill its transport.
    struct RawAgent {
        child: std::process::Child,
        stdin: Option<std::process::ChildStdin>,
        _frames: mpsc::Receiver<Frame>,
        /// While set, the reader takes no frames, as a client that has
        /// stopped draining the transport.
        paused: Arc<AtomicBool>,
        /// Session output bytes received so far.
        received: Arc<std::sync::atomic::AtomicU64>,
    }

    impl RawAgent {
        fn start(
            target: &RemoteTarget,
            remote_path: &str,
            command: &str,
            args: &[&str],
            idle_limit_secs: u32,
        ) -> std::result::Result<Self, String> {
            let mut transport = super::super::remote_command(
                target,
                "sh",
                &["-c".into(), format!("exec \"{remote_path}\" --server")],
            )
            .map_err(|e| e.to_string())?;
            transport.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
            let mut child = transport.spawn().map_err(|e| format!("spawn transport: {e}"))?;
            let mut stdin = child.stdin.take().unwrap();
            let mut stdout = child.stdout.take().unwrap();
            let request = SpawnRequest {
                command: Some(command.into()),
                args: args.iter().map(|a| a.to_string()).collect(),
                cwd: None,
                env: crate::terminal::child_env(&Profile::default_shell()).into_iter().collect(),
                cols: 80,
                rows: 24,
                cell_width: 8,
                cell_height: 16,
                idle_limit_secs,
            };
            Frame::spawn(&request).and_then(|f| f.write_to(&mut stdin)).map_err(|e| e.to_string())?;
            let (tx, frames) = mpsc::channel();
            let paused = Arc::new(AtomicBool::new(false));
            let received = Arc::new(std::sync::atomic::AtomicU64::new(0));
            let (pause, counter) = (paused.clone(), received.clone());
            thread::spawn(move || {
                if super::super::protocol::skip_preamble(&mut stdout).is_err() {
                    return;
                }
                loop {
                    while pause.load(Ordering::Acquire) {
                        thread::sleep(Duration::from_millis(20));
                    }
                    let Ok(Some(frame)) = Frame::read_from_or_eof(&mut stdout) else {
                        break;
                    };
                    if frame.msg_type == MSG_DATA {
                        counter.fetch_add(frame.payload.len() as u64, Ordering::AcqRel);
                    }
                    if tx.send(frame).is_err() {
                        break;
                    }
                }
            });
            let first = frames
                .recv_timeout(Duration::from_secs(30))
                .map_err(|e| format!("no spawn response: {e}"))?;
            let response: SpawnResponse = first.decode_json().map_err(|e| e.to_string())?;
            if !response.success {
                return Err(format!("spawn failed: {:?}", response.error));
            }
            Ok(Self {
                child,
                stdin: Some(stdin),
                _frames: frames,
                paused,
                received,
            })
        }

        fn heartbeat(&mut self) {
            Frame::heartbeat().write_to(self.stdin.as_mut().unwrap()).ok();
        }

        fn input(&mut self, text: &str) {
            let frame = Frame::data(text.as_bytes().to_vec()).unwrap();
            frame.write_to(self.stdin.as_mut().unwrap()).ok();
        }

        /// Hard-kills the local transport process (ssh / docker exec).
        fn kill_transport(&mut self) {
            self.child.kill().ok();
            self.child.wait().ok();
        }
    }

    impl Drop for RawAgent {
        fn drop(&mut self) {
            self.child.kill().ok();
            self.child.wait().ok();
        }
    }

    fn count_procs(c: &Container, pattern: &str) -> usize {
        c.exec_sh(&format!(
            "for p in /proc/[0-9]*; do tr '\\0' ' ' < $p/cmdline 2>/dev/null; echo; done | grep -c '{pattern}'"
        ))
        .trim()
        .parse()
        .unwrap_or(usize::MAX)
    }

    fn list_procs(c: &Container, pattern: &str) -> String {
        c.exec_sh(&format!(
            "for p in /proc/[0-9]*; do echo \"$p $(tr '\\0' ' ' < $p/cmdline 2>/dev/null)\"; done | grep '{pattern}'"
        ))
    }

    /// Killing the transport must take the shell, its foreground job and its
    /// background job with it.
    fn scenario_orphans(
        c: &Container,
        target: &RemoteTarget,
        remote_path: &str,
        shell: &str,
        shell_args: &[&str],
    ) -> std::result::Result<(), String> {
        let mut agent = RawAgent::start(
            target,
            remote_path,
            shell,
            shell_args,
            DEFAULT_IDLE_LIMIT_SECS,
        )?;
        agent.input("sleep 4321 &\n");
        agent.input("sleep 4322\n");
        let deadline = Instant::now() + Duration::from_secs(20);
        while count_procs(c, "sleep 432[12]") != 2 {
            if Instant::now() >= deadline {
                return Err(format!("jobs never started: {}", list_procs(c, "sleep 432[12]")));
            }
            thread::sleep(Duration::from_millis(250));
        }
        agent.kill_transport();
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let left = count_procs(c, "sleep 432[12]");
            if left == 0 {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "{left} orphan(s) survived the transport: {}",
                    list_procs(c, "sleep 432[12]")
                ));
            }
            thread::sleep(Duration::from_millis(250));
        }
    }

    /// Drains events counting bytes, keeping only a short tail.
    fn drain_count(
        rx: &mut EventReceiver<Event>,
        timeout: Duration,
        stop: impl Fn(&str, u64) -> bool,
    ) -> (End, u64, String) {
        let deadline = Instant::now() + timeout;
        let mut total = 0u64;
        let mut tail = String::new();
        loop {
            match rx.try_next() {
                Ok(Some(Event::Output(bytes))) => {
                    total += bytes.len() as u64;
                    tail.push_str(&String::from_utf8_lossy(&bytes));
                    if tail.len() > 256 {
                        let mut cut = tail.len() - 128;
                        while !tail.is_char_boundary(cut) {
                            cut += 1;
                        }
                        tail.drain(..cut);
                    }
                    if stop(&tail, total) {
                        return (End::Matched, total, tail);
                    }
                }
                Ok(Some(Event::ProcessInfo(_))) => {}
                Ok(Some(Event::Exited)) => return (End::Exited, total, tail),
                Ok(Some(Event::Failed)) => return (End::Failed, total, tail),
                Ok(None) => return (End::Closed, total, tail),
                Err(_) => {
                    if Instant::now() >= deadline {
                        return (End::TimedOut, total, tail);
                    }
                    thread::sleep(Duration::from_millis(2));
                }
            }
        }
    }

    fn agent_rss(c: &Container) -> String {
        c.exec_sh("for p in /proc/[0-9]*; do if tr '\\0' ' ' < $p/cmdline 2>/dev/null | grep -q 'brindle-remote --server'; then grep -E 'VmHWM|VmRSS' $p/status | tr '\\n' ' '; fi; done")
    }

    fn scenario_flood(c: &Container, target: &RemoteTarget) -> std::result::Result<String, String> {
        let (h, mut rx) = connect_command(profile_for(target), "sh", &[], test_size(80, 24));
        let started = Instant::now();
        h.input(b"yes | head -c 200000000; echo FLOOD-DONE-$((1+2))\n".to_vec());
        let mut rss = String::new();
        let mut total = 0u64;
        let deadline = Instant::now() + Duration::from_secs(240);
        loop {
            let (end, bytes, tail) = drain_count(&mut rx, Duration::from_secs(1), |tail, _| {
                tail.contains("FLOOD-DONE-3\r\n")
            });
            total += bytes;
            if rss.is_empty() && total > 20_000_000 {
                rss = agent_rss(c);
            }
            match end {
                End::Matched => break,
                End::TimedOut if Instant::now() < deadline => {}
                other => {
                    return Err(format!("flood ended {other:?} after {total} bytes; tail={tail:?}"));
                }
            }
        }
        if total < 200_000_000 {
            return Err(format!("only {total} of 200000000 bytes arrived"));
        }
        h.input(b"exit\n".to_vec());
        Ok(format!("{total} bytes in {:?}; agent {}", started.elapsed(), rss.trim()))
    }

    fn scenario_ctrl_c_during_flood(target: &RemoteTarget) -> std::result::Result<(), String> {
        let (h, mut rx) = connect_command(profile_for(target), "sh", &[], test_size(80, 24));
        h.input(b"yes\n".to_vec());
        let (end, total, _) =
            drain_count(&mut rx, Duration::from_secs(30), |_, total| total > 5_000_000);
        if end != End::Matched {
            return Err(format!("flood never started: {end:?} {total}"));
        }
        h.input(vec![0x03]);
        h.input(b"echo CTRLC-OK-$((2+3))\n".to_vec());
        let (end, total, tail) = drain_count(&mut rx, Duration::from_secs(30), |tail, _| {
            tail.contains("CTRLC-OK-5\r\n")
        });
        if end != End::Matched {
            return Err(format!(
                "^C did not stop the flood: {end:?} after {total} more bytes; tail={tail:?}"
            ));
        }
        h.input(b"exit\n".to_vec());
        Ok(())
    }

    const IDLE_TEST_LIMIT_SECS: u32 = 3;
    /// Matches only the raw helper, not the shell that greps for it.
    const RAW_AGENT: &str = "raw-agen[t] --server";
    const IDLE_JOB: &str = "sleep 433[1]";

    fn wait_until(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if condition() {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(250));
        }
    }

    /// Starts a raw session with a short idle limit and a job running under
    /// its shell.
    fn start_idle_session(
        c: &Container,
        target: &RemoteTarget,
    ) -> std::result::Result<RawAgent, String> {
        let mut agent = RawAgent::start(target, "/tmp/raw-agent", "sh", &[], IDLE_TEST_LIMIT_SECS)?;
        agent.input("sleep 4331\n");
        if !wait_until(Duration::from_secs(20), || count_procs(c, IDLE_JOB) == 1) {
            return Err(format!("job never started: {}", list_procs(c, IDLE_JOB)));
        }
        Ok(agent)
    }

    fn wait_session_gone(c: &Container, within: Duration) -> std::result::Result<(), String> {
        if wait_until(within, || {
            count_procs(c, IDLE_JOB) == 0 && count_procs(c, RAW_AGENT) == 0
        }) {
            return Ok(());
        }
        Err(format!(
            "still running after {within:?}: {} {}",
            list_procs(c, RAW_AGENT),
            list_procs(c, IDLE_JOB)
        ))
    }

    /// A client that goes silent with its transport still open (a cut network
    /// leaves sshd holding the session) loses the helper, the shell and the
    /// job under it shortly after the idle limit.
    fn scenario_idle_client_is_dropped(
        c: &Container,
        target: &RemoteTarget,
    ) -> std::result::Result<(), String> {
        let _agent = start_idle_session(c, target)?;
        let started = Instant::now();
        wait_session_gone(c, Duration::from_secs(u64::from(IDLE_TEST_LIMIT_SECS) + 8))?;
        // The limit counts from the spawn, a little before `started`.
        if started.elapsed() < Duration::from_secs(1) {
            return Err(format!("ended too early: {:?}", started.elapsed()));
        }
        Ok(())
    }

    /// Heartbeats keep a silent session alive past the limit; once they stop
    /// the session goes.
    fn scenario_heartbeats_keep_the_session(
        c: &Container,
        target: &RemoteTarget,
    ) -> std::result::Result<(), String> {
        let mut agent = start_idle_session(c, target)?;
        for _ in 0..8 {
            agent.heartbeat();
            thread::sleep(Duration::from_secs(1));
        }
        if count_procs(c, IDLE_JOB) != 1 || count_procs(c, RAW_AGENT) != 1 {
            return Err(format!(
                "died despite heartbeats: {} {}",
                list_procs(c, RAW_AGENT),
                list_procs(c, IDLE_JOB)
            ));
        }
        wait_session_gone(c, Duration::from_secs(u64::from(IDLE_TEST_LIMIT_SECS) + 8))
    }

    /// While the reader is stalled under a flood, real input (not just
    /// heartbeats) fills the helper's event queue and leaves its stdin reader
    /// blocked mid-hand-over. That is not silence: the session survives, and
    /// Ctrl-C still stops the flood once the client reads again.
    fn scenario_input_during_stall_then_ctrl_c(
        c: &Container,
        target: &RemoteTarget,
    ) -> std::result::Result<(), String> {
        let mut agent = RawAgent::start(target, "/tmp/raw-agent", "sh", &[], IDLE_TEST_LIMIT_SECS)?;
        agent.input("yes\n");
        if !wait_until(Duration::from_secs(20), || {
            agent.received.load(Ordering::Acquire) > 100_000
        }) {
            return Err("the flood never started".into());
        }
        agent.paused.store(true, Ordering::Release);
        // More frames than the helper's event queue holds.
        for _ in 0..100 {
            agent.input("x");
        }
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(u64::from(IDLE_TEST_LIMIT_SECS) * 3) {
            agent.heartbeat();
            thread::sleep(Duration::from_secs(1));
            if count_procs(c, RAW_AGENT) != 1 {
                return Err(format!("helper gone after {:?} of stalled reading", started.elapsed()));
            }
        }
        agent.paused.store(false, Ordering::Release);
        agent.input("\u{3}");
        agent.input("echo CTRLC-OK-$((2+3))\n");
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut tail = String::new();
        while Instant::now() < deadline {
            let Ok(frame) = agent._frames.recv_timeout(Duration::from_millis(200)) else {
                continue;
            };
            if frame.msg_type == MSG_DATA {
                tail.push_str(&String::from_utf8_lossy(&frame.payload));
                if tail.len() > 256 {
                    let mut cut = tail.len() - 128;
                    while !tail.is_char_boundary(cut) {
                        cut += 1;
                    }
                    tail.drain(..cut);
                }
                if tail.contains("CTRLC-OK-5\r\n") {
                    return Ok(());
                }
            }
        }
        Err(format!(
            "^C did not stop the flood; helpers: {:?}; tail={tail:?}",
            list_procs(c, RAW_AGENT)
        ))
    }

    /// Output every 10 ms never leaves the helper's event queue quiet for a
    /// whole poll interval, yet a client that has gone silent (while still
    /// reading) must lose the session at the idle limit all the same.
    fn scenario_steady_output_does_not_hide_a_vanished_client(
        c: &Container,
        target: &RemoteTarget,
    ) -> std::result::Result<(), String> {
        let busy = "busyloo[p]";
        let _agent = RawAgent::start(
            target,
            "/tmp/raw-agent",
            "sh",
            &["-c", "while :; do echo x; sleep 0.01; done # busyloop"],
            IDLE_TEST_LIMIT_SECS,
        )?;
        if !wait_until(Duration::from_secs(20), || count_procs(c, busy) == 1) {
            return Err(format!("loop never started: {}", list_procs(c, busy)));
        }
        let limit = Duration::from_secs(u64::from(IDLE_TEST_LIMIT_SECS));
        if !wait_until(limit + Duration::from_secs(8), || {
            count_procs(c, busy) == 0 && count_procs(c, RAW_AGENT) == 0
        }) {
            return Err(format!(
                "still running: {} {}",
                list_procs(c, RAW_AGENT),
                list_procs(c, busy)
            ));
        }
        Ok(())
    }

    /// A healthy client that stops draining a flood for longer than the idle
    /// limit, while its heartbeats pile up unread behind the output, must not
    /// be dropped by the idle check: the helper's loop is blocked on the
    /// backed-up output, so silence there proves nothing. Once the client
    /// drains again, the session carries on.
    fn scenario_stalled_reader_under_flood(
        c: &Container,
        target: &RemoteTarget,
    ) -> std::result::Result<(), String> {
        let mut agent = RawAgent::start(target, "/tmp/raw-agent", "sh", &[], IDLE_TEST_LIMIT_SECS)?;
        agent.input("yes\n");
        // Let the pipeline fill, then stop reading.
        if !wait_until(Duration::from_secs(20), || {
            agent.received.load(Ordering::Acquire) > 100_000
        }) {
            return Err("the flood never started".into());
        }
        agent.paused.store(true, Ordering::Release);
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(u64::from(IDLE_TEST_LIMIT_SECS) * 3) {
            agent.heartbeat();
            thread::sleep(Duration::from_secs(1));
            if count_procs(c, RAW_AGENT) != 1 {
                return Err(format!("helper gone after {:?} of stalled reading", started.elapsed()));
            }
        }
        let before = agent.received.load(Ordering::Acquire);
        agent.paused.store(false, Ordering::Release);
        // The client is still alive, so it keeps its heartbeats going; with
        // the 3 s test limit, 3 s of silence here would be a vanished client.
        for _ in 0..3 {
            agent.heartbeat();
            thread::sleep(Duration::from_secs(1));
        }
        let gained = agent.received.load(Ordering::Acquire) - before;
        if gained < 1_000_000 || count_procs(c, RAW_AGENT) != 1 {
            return Err(format!(
                "output did not resume: {gained} bytes in 3 s; helpers: {:?}",
                list_procs(c, RAW_AGENT)
            ));
        }
        Ok(())
    }

    /// The real client keeps an idle session alive past the idle limit with
    /// its own heartbeats, and the limit it sends is the one the helper uses.
    fn scenario_client_heartbeats(target: &RemoteTarget) -> std::result::Result<(), String> {
        let timing = Timing {
            heartbeat_interval: Duration::from_secs(1),
            idle_limit_secs: IDLE_TEST_LIMIT_SECS,
        };
        let session = |timing| {
            let (tx, rx) = futures::channel::mpsc::channel(1024);
            let h = connect_with(
                profile_for(target),
                Some(("sh".into(), Vec::new())),
                None,
                test_size(80, 24),
                tx,
                timing,
            );
            (h, rx)
        };
        let (h, mut rx) = session(timing);
        let mut out = String::new();
        h.input(b"echo up-$((1+1))\n".to_vec());
        if !wait_for(&mut rx, &mut out, "up-2\r\n") {
            return Err(format!("session never came up: {out:?}"));
        }
        let end = pump(&mut rx, &mut out, Duration::from_secs(7), |_| false, &mut Vec::new());
        if end != End::TimedOut {
            return Err(format!("idle session ended: {end:?} {out:?}"));
        }
        h.input(b"echo still-$((2+2))\n".to_vec());
        if !wait_for(&mut rx, &mut out, "still-4\r\n") {
            return Err(format!("session died while idle: {out:?}"));
        }
        h.input(b"exit\n".to_vec());

        // With heartbeats slower than the limit, the same path loses the session.
        let (_h, mut rx) = session(Timing {
            heartbeat_interval: Duration::from_secs(60),
            ..timing
        });
        let (end, out) = run_to_end(&mut rx, Duration::from_secs(20));
        if end != End::Exited && end != End::Failed {
            return Err(format!("silent client kept its session: {end:?} {out:?}"));
        }
        Ok(())
    }

    /// Starts the profile's default session and reports what the shell sees.
    fn scenario_default_shell(
        target: &RemoteTarget,
        cwd: Option<&str>,
    ) -> std::result::Result<String, String> {
        let (tx, mut rx) = futures::channel::mpsc::channel(1024);
        let mut profile = profile_for(target);
        profile.cwd = cwd.map(String::from);
        let h = connect(profile, None, None, test_size(80, 24), tx);
        let mut out = String::new();
        h.input(
            b"echo S1=$0 S2=$TERM S3=$PWD E$((1+1))\n".to_vec(),
        );
        if !wait_for(&mut rx, &mut out, "E2\r\n") {
            return Err(format!("default shell never answered: {out:?}"));
        }
        h.input(b"exit\n".to_vec());
        let reply = out.rfind("S1=").map_or(out.as_str(), |at| &out[at..]);
        Ok(reply.lines().next().unwrap_or_default().to_string())
    }

    #[test]
    fn ssh_transport_scenarios() {
        if std::env::var_os("BRINDLE_REMOTE_DOCKER_TEST").is_none_or(|v| v != "1") {
            return;
        }
        let _turn = DOCKER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let helper = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/x86_64-unknown-linux-musl/release/brindle-remote");
        assert!(
            helper.is_file(),
            "{} not found; build it with the docker command in README.md",
            helper.display()
        );
        unsafe { std::env::set_var("BRINDLE_REMOTE_AGENT", &helper) };
        let scratch = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target/tmp")
            .join(format!("brindle-ssh-test-{}", std::process::id()));
        std::fs::create_dir_all(&scratch).unwrap();
        let fx = SshFixture::start(&scratch, "ssh").expect("ssh fixture");
        let mut failures = Vec::new();
        let mut record = |name: &str, result: std::result::Result<(), String>| {
            eprintln!("scenario {name}: {}", if result.is_ok() { "ok" } else { "FAILED" });
            if let Err(e) = result {
                failures.push(format!("{name}: {e}"));
            }
        };
        record("ssh upload then reuse", scenario_ssh_upload_then_reuse(&fx));
        record("ssh round trip and exit", scenario_ssh_round_trip(&fx));
        record("ssh wrong key", scenario_ssh_wrong_key(&fx));
        record("ssh ~/.profile noise", scenario_ssh_profile_noise(&fx));
        record("ssh ~/.bashrc noise", scenario_ssh_bashrc_noise(&fx));

        let digest = file_sha256(&helper).unwrap();
        let ssh_helper =
            format!("$HOME/{REMOTE_DIR}/{}-{digest}/brindle-remote", env!("CARGO_PKG_VERSION"));
        record(
            "ssh orphans (bash)",
            scenario_orphans(&fx.container, &fx.target, &ssh_helper, "bash", &["-i"]),
        );
        eprintln!(
            "OBSERVATION ssh orphans (dash, job control): {:?}",
            scenario_orphans(&fx.container, &fx.target, &ssh_helper, "sh", &["-i"])
        );
        match scenario_flood(&fx.container, &fx.target) {
            Ok(summary) => eprintln!("OBSERVATION ssh flood: {summary}"),
            Err(e) => record("ssh flood", Err(e)),
        }
        record("ssh ctrl-c during flood", scenario_ctrl_c_during_flood(&fx.target));
        std::fs::remove_dir_all(&scratch).ok();
        assert!(failures.is_empty(), "failures:\n{}", failures.join("\n"));
    }

    #[test]
    fn docker_session_behaviour_scenarios() {
        if std::env::var_os("BRINDLE_REMOTE_DOCKER_TEST").is_none_or(|v| v != "1") {
            return;
        }
        let _turn = DOCKER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
        let helper = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target/x86_64-unknown-linux-musl/release/brindle-remote");
        assert!(
            helper.is_file(),
            "{} not found; build it with the docker command in README.md",
            helper.display()
        );
        unsafe { std::env::set_var("BRINDLE_REMOTE_AGENT", &helper) };
        let c = Container::start("ubuntu:24.04", "behaviour").unwrap();
        let target = container_profile(&c.0).remote.unwrap();
        let mut failures = Vec::new();
        let mut record = |name: &str, result: std::result::Result<(), String>| {
            eprintln!("scenario {name}: {}", if result.is_ok() { "ok" } else { "FAILED" });
            if let Err(e) = result {
                failures.push(format!("{name}: {e}"));
            }
        };
        // The raw sessions run the helper from a path the test controls.
        let copy = Command::new("docker")
            .arg("cp")
            .arg(&helper)
            .arg(format!("{}:/tmp/raw-agent", c.0))
            .output()
            .unwrap();
        assert!(copy.status.success());
        c.exec_sh("chmod 755 /tmp/raw-agent");
        record(
            "docker orphans (bash)",
            scenario_orphans(&c, &target, "/tmp/raw-agent", "bash", &["-i"]),
        );
        eprintln!(
            "OBSERVATION docker orphans (dash, job control): {:?}",
            scenario_orphans(&c, &target, "/tmp/raw-agent", "sh", &["-i"])
        );
        match scenario_flood(&c, &target) {
            Ok(summary) => eprintln!("OBSERVATION docker flood: {summary}"),
            Err(e) => record("docker flood", Err(e)),
        }
        record("docker ctrl-c during flood", scenario_ctrl_c_during_flood(&target));
        record("idle client is dropped", scenario_idle_client_is_dropped(&c, &target));
        record(
            "heartbeats keep a silent session",
            scenario_heartbeats_keep_the_session(&c, &target),
        );
        record(
            "a stalled reader under flood is not dropped",
            scenario_stalled_reader_under_flood(&c, &target),
        );
        record(
            "input during a stall is not silence, and ^C works after",
            scenario_input_during_stall_then_ctrl_c(&c, &target),
        );
        record(
            "steady output does not hide a vanished client",
            scenario_steady_output_does_not_hide_a_vanished_client(&c, &target),
        );
        record("the client sends heartbeats", scenario_client_heartbeats(&target));
        match scenario_default_shell(&target, None) {
            Ok(line) => record(
                "default shell is the passwd login shell with a sane TERM",
                if line.starts_with("S1=-bash S2=xterm-256color") {
                    Ok(())
                } else {
                    Err(line)
                },
            ),
            Err(e) => record("default shell", Err(e)),
        }
        match scenario_default_shell(&target, Some("/no/such/dir")) {
            Ok(line) => record(
                "a missing cwd falls back to home",
                if line.contains(" S3=/root ") { Ok(()) } else { Err(line) },
            ),
            Err(e) => record("default shell, missing cwd", Err(e)),
        }
        assert!(failures.is_empty(), "failures:\n{}", failures.join("\n"));
    }

    /// The relay's launch and its deployment against real endpoints (same
    /// opt-in): what the relay transport runs for each connection.
    #[cfg(any(target_os = "linux", windows))]
    mod relay_launch {
        use super::*;
        use crate::remote::local::{Link, dial_relay, start_relay};
        use crate::remote::protocol::relay::{RelayHost, RelayMessage};

        type Scenario = std::result::Result<(), String>;

        /// What a launched relay says first, which is `RELAY_HOST` once it
        /// has reached its core. Letting go of the link ends the relay.
        fn host_of(link: Link) -> std::result::Result<RelayHost, String> {
            let Link { mut reader, wire } = link;
            let (tx, first) = mpsc::channel();
            thread::spawn(move || {
                tx.send(Frame::read_from(&mut reader)).ok();
            });
            let frame = first.recv_timeout(Duration::from_secs(60));
            drop(wire);
            let frame = frame
                .map_err(|_| "no frame from the relay within 60 s".to_string())?
                .map_err(|e| format!("reading the relay's first frame: {e}"))?;
            match RelayMessage::decode(&frame) {
                Ok(RelayMessage::Host(host)) if host.version == env!("CARGO_PKG_VERSION") => Ok(host),
                other => Err(format!("the relay's first frame was {other:?}")),
            }
        }

        /// Connects as the relay transport does: the production dial.
        fn connect(target: &RemoteTarget) -> std::result::Result<RelayHost, String> {
            let link = dial_relay("test".into(), target).launch().map_err(|e| format!("{e:#}"))?;
            host_of(link)
        }

        fn container_target(c: &Container, user: Option<&str>) -> RemoteTarget {
            RemoteTarget::Container {
                engine: "docker".into(),
                container: c.0.clone(),
                user: user.map(Into::into),
            }
        }

        /// The cached helper's inode and modification time.
        fn helper_file(c: &Container, path: &str) -> String {
            c.exec_sh(&format!("stat -c '%i %Y' {path} 2>&1"))
        }

        fn exec_as(c: &Container, user: &str, script: &str) -> String {
            let out = Command::new("docker")
                .args(["exec", "--user", user, &c.0, "sh", "-c", script])
                .output()
                .expect("docker exec");
            format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr))
        }

        /// The first connection to a container deploys the helper, as root
        /// and for every user; later ones, root's or another user's, run that
        /// same copy.
        fn scenario_container_helper_is_reused(c: &Container) -> Scenario {
            let directory = Helper::find().map_err(|e| format!("{e:#}"))?.container_dir();
            let path = format!("{directory}/brindle-remote");
            if !c.exec_sh("ls -d /tmp/brindle-* 2>/dev/null").is_empty() {
                return Err("a helper was there before the first connection".into());
            }
            let first = connect(&container_target(c, None))?;
            scenario_one_helper_kept(c)?;
            let copied = helper_file(c, &path);
            // Later than any second copy's time could equal.
            thread::sleep(Duration::from_millis(1200));
            let second = connect(&container_target(c, None))?;
            let other = connect(&container_target(c, Some("remote")))?;
            let after = helper_file(c, &path);
            if copied != after {
                return Err(format!("the helper was copied again: {copied:?} then {after:?}"));
            }
            scenario_one_helper_kept(c)?;
            if first.home != "/root" || second.home != "/root" || other.home != "/home/remote" {
                return Err(format!("homes: {:?} {:?} {:?}", first.home, second.home, other.home));
            }
            Ok(())
        }

        /// A directory another user of the container made where the helper
        /// would go is refused by name, and nothing in it runs or changes.
        fn scenario_taken_directory_is_refused(c: &Container) -> Scenario {
            let directory = Helper::find().map_err(|e| format!("{e:#}"))?.container_dir();
            let planted = exec_as(
                c,
                "remote",
                &format!(
                    "mkdir {directory} && printf '#!/bin/sh\\ntouch /tmp/canary-ran\\n' > {directory}/brindle-remote \
                     && chmod 755 {directory}/brindle-remote"
                ),
            );
            if !planted.is_empty() {
                return Err(format!("planting the directory: {planted}"));
            }
            let before = c.exec_sh(&format!("stat -c '%n %U %a %i %Y' {directory} {directory}/* 2>&1"));
            for user in [None, Some("remote")] {
                match connect(&container_target(c, user)) {
                    Ok(host) => return Err(format!("connected as {user:?}: {host:?}")),
                    Err(error) if error.contains(&format!("{directory} exists in the container")) => {}
                    Err(error) => return Err(format!("as {user:?}, the error doesn't name it: {error}")),
                }
            }
            let after = c.exec_sh(&format!("stat -c '%n %U %a %i %Y' {directory} {directory}/* 2>&1"));
            if before != after || !before.contains(" remote ") {
                return Err(format!("the directory changed: {before:?} then {after:?}"));
            }
            let ran = c.exec_sh("ls /tmp/canary-ran /tmp/brindle-helper-*/brindle-remote.tmp-* 2>/dev/null");
            if !ran.is_empty() {
                return Err(format!("something ran in it, or was copied into it: {ran}"));
            }
            Ok(())
        }

        /// A runner that counts what the real one runs.
        struct Counted<'a> {
            commands: Commands<'a>,
            ran: RefCell<Vec<Vec<String>>>,
        }

        impl Run for Counted<'_> {
            fn run(&self, command: Command, input: Option<&Path>, what: &str) -> Result<Ran> {
                self.ran.borrow_mut().push(line(&command));
                self.commands.run(command, input, what)
            }
        }

        /// Launches the relay with the real commands, returning each launch
        /// and each deployment command that was started. Every process of a
        /// launch is one of the two.
        fn counted_launch(
            target: &RemoteTarget,
        ) -> std::result::Result<(Vec<Vec<String>>, Vec<Vec<String>>), String> {
            let helper = Helper::find().map_err(|e| format!("{e:#}"))?;
            let never = AtomicBool::new(false);
            let counted = Counted { commands: Commands { cancel: &never }, ran: RefCell::default() };
            let launched = RefCell::new(Vec::new());
            let link = launch_with(&counted, target, &helper, |command| {
                launched.borrow_mut().push(line(&command));
                start_relay(command, Duration::from_secs(60))
            })
            .map_err(|e| format!("{e:#}"))?;
            host_of(link)?;
            Ok((launched.into_inner(), counted.ran.into_inner()))
        }

        /// A host without the helper takes three `ssh` processes (the launch
        /// that finds none, the upload, the launch); with it cached, the
        /// launch is the only one. Counted where the launch starts them.
        fn scenario_warm_ssh_launch_is_one_process(fx: &SshFixture) -> Scenario {
            let c = &fx.container;
            if !c.exec_sh("test -e /root/.cache/brindle && echo present").trim().is_empty() {
                return Err("a cache was there before the first connection".into());
            }
            let (launched, ran) = counted_launch(&fx.target)?;
            if launched.len() != 2 || ran.len() != 1 {
                return Err(format!("the first connection: launched {launched:?}, ran {ran:?}"));
            }
            // Everything in the build's directory, which is the helper alone.
            // (The host's server directory is beside it.)
            let cached = || {
                c.exec_sh(&format!(
                    "stat -c '%n %i %Y' /root/.cache/brindle/{}-*/* 2>&1",
                    env!("CARGO_PKG_VERSION")
                ))
            };
            let uploaded = cached();
            if uploaded.lines().count() != 1 || !uploaded.contains("/brindle-remote ") {
                return Err(format!("the cache holds {uploaded:?}"));
            }
            thread::sleep(Duration::from_millis(1200));
            let (launched, ran) = counted_launch(&fx.target)?;
            if launched.len() != 1 || launched[0][0] != "ssh" || !ran.is_empty() {
                return Err(format!("the warm connection: launched {launched:?}, ran {ran:?}"));
            }
            // And through the production dial, which uploads nothing either.
            connect(&fx.target)?;
            let after = cached();
            if uploaded != after {
                return Err(format!("the helper was uploaded again: {uploaded:?} then {after:?}"));
            }
            Ok(())
        }

        #[test]
        fn relay_launch_scenarios() {
            if std::env::var_os("BRINDLE_REMOTE_DOCKER_TEST").is_none_or(|v| v != "1") {
                return;
            }
            let _turn = DOCKER_TEST_LOCK.lock().unwrap_or_else(|p| p.into_inner());
            let helper = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("target/x86_64-unknown-linux-musl/release/brindle-remote");
            assert!(
                helper.is_file(),
                "{} not found; build it with the docker command in README.md",
                helper.display()
            );
            unsafe { std::env::set_var("BRINDLE_REMOTE_AGENT", &helper) };
            let scratch = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("target/tmp")
                .join(format!("brindle-relay-test-{}", std::process::id()));
            std::fs::create_dir_all(&scratch).unwrap();
            // The fixture's image has a second user, `remote`.
            let fx = SshFixture::start(&scratch, "relay-ssh").expect("ssh fixture");
            let reused = Container::start(FIXTURE_TAG, "reuse").unwrap();
            let taken = Container::start(FIXTURE_TAG, "taken").unwrap();
            let mut failures = Vec::new();
            let mut record = |name: &str, result: Scenario| {
                eprintln!("scenario {name}: {}", if result.is_ok() { "ok" } else { "FAILED" });
                if let Err(e) = result {
                    failures.push(format!("{name}: {e}"));
                }
            };
            record("a warm ssh launch is one ssh process", scenario_warm_ssh_launch_is_one_process(&fx));
            record("a container's helper is reused", scenario_container_helper_is_reused(&reused));
            record("a taken directory is refused", scenario_taken_directory_is_refused(&taken));
            std::fs::remove_dir_all(&scratch).ok();
            assert!(failures.is_empty(), "failures:\n{}", failures.join("\n"));
        }
    }

    fn frames_in(wire: &[u8]) -> Vec<Frame> {
        let mut cursor = std::io::Cursor::new(wire);
        let mut frames = Vec::new();
        while let Some(frame) = Frame::read_from_or_eof(&mut cursor).unwrap() {
            frames.push(frame);
        }
        frames
    }

    /// Runs `write_loop` until `stop_after` has passed, then cancels it.
    fn run_write_loop(
        rx: &mpsc::Receiver<Outgoing>,
        heartbeat: Duration,
        stop_after: Duration,
        writer: &mut impl std::io::Write,
    ) -> Result<()> {
        let cancel = Arc::new(AtomicBool::new(false));
        let stopper = cancel.clone();
        let timer = thread::spawn(move || {
            thread::sleep(stop_after);
            stopper.store(true, Ordering::Release);
        });
        let result = write_loop(rx, &cancel, writer, heartbeat);
        timer.join().unwrap();
        result
    }

    #[test]
    fn an_idle_writer_sends_heartbeats_and_then_the_goodbye() {
        let (_tx, rx) = mpsc::sync_channel::<Outgoing>(8);
        let mut wire = Vec::new();
        let heartbeat = Duration::from_millis(200);
        let started = Instant::now();
        run_write_loop(&rx, heartbeat, Duration::from_millis(900), &mut wire).unwrap();
        let elapsed = started.elapsed();
        let frames = frames_in(&wire);
        let beats = frames.iter().filter(|f| f.msg_type == MSG_HEARTBEAT).count();
        // Each heartbeat needs a full `heartbeat` of silence since the
        // previous write (or the start), so the run, however long the stop
        // timer actually slept, holds at most `elapsed / heartbeat` of them;
        // a writer beating on every 100 ms poll exceeds that. A stall can
        // delay any beat, so the floor is only that the idle writer sent one.
        let most = (elapsed.as_millis() / heartbeat.as_millis()) as usize;
        assert!(
            (1..=most).contains(&beats),
            "{beats} heartbeats in {elapsed:?} (at most {most}): {frames:?}"
        );
        assert_eq!(frames.last().unwrap().msg_type, MSG_EXIT);
        assert_eq!(
            frames.iter().filter(|f| f.msg_type == MSG_EXIT).count(),
            1,
            "{frames:?}"
        );
    }

    #[test]
    fn a_writer_that_just_sent_input_does_not_add_a_heartbeat() {
        let (tx, rx) = mpsc::sync_channel::<Outgoing>(8);
        let feeder = thread::spawn(move || {
            for _ in 0..8 {
                tx.send(Outgoing::Data(b"x".to_vec())).unwrap();
                thread::sleep(Duration::from_millis(100));
            }
            tx
        });
        let mut wire = Vec::new();
        run_write_loop(
            &rx,
            Duration::from_millis(500),
            Duration::from_millis(850),
            &mut wire,
        )
        .unwrap();
        let _tx = feeder.join().unwrap();
        let frames = frames_in(&wire);
        assert!(frames.iter().all(|f| f.msg_type != MSG_HEARTBEAT), "{frames:?}");
    }

    struct BrokenPipe;

    impl std::io::Write for BrokenPipe {
        fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
            Err(std::io::ErrorKind::BrokenPipe.into())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn a_failed_goodbye_is_not_an_error() {
        let (_tx, rx) = mpsc::sync_channel::<Outgoing>(8);
        run_write_loop(
            &rx,
            Duration::from_secs(60),
            Duration::from_millis(150),
            &mut BrokenPipe,
        )
        .unwrap();
    }

    #[test]
    fn a_failed_heartbeat_ends_the_writer_with_an_error_for_the_caller_to_ignore() {
        let (_tx, rx) = mpsc::sync_channel::<Outgoing>(8);
        let result = run_write_loop(
            &rx,
            Duration::from_millis(100),
            Duration::from_secs(1),
            &mut BrokenPipe,
        );
        assert!(result.is_err());
    }

    #[test]
    fn notices_use_crlf_for_every_line() {
        let Event::Output(bytes) = notice("one\ntwo\r\nthree\n") else {
            panic!("expected output");
        };
        assert_eq!(bytes, b"one\r\ntwo\r\nthree\r\n");
    }

    #[test]
    fn large_input_is_chunked_without_loss() {
        let bytes: Vec<u8> = (0..2 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
        let (tx, rx) = mpsc::sync_channel(128);
        let handle = Handle {
            tx,
            cancel: Arc::new(AtomicBool::new(false)),
        };
        handle.input(bytes.clone());
        drop(handle);
        let mut wire = Vec::new();
        for message in rx {
            message.into_frame().unwrap().write_to(&mut wire).unwrap();
        }
        let mut cursor = std::io::Cursor::new(wire);
        let mut decoded = Vec::new();
        let mut frames = 0;
        while let Some(frame) = Frame::read_from_or_eof(&mut cursor).unwrap() {
            assert_eq!(frame.msg_type, MSG_DATA);
            assert!(frame.payload.len() <= DATA_CHUNK_SIZE);
            decoded.extend(frame.payload);
            frames += 1;
        }
        assert!(frames > 1);
        assert_eq!(decoded, bytes);
    }
}
