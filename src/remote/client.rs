//! Client transport and helper deployment for remote PTY sessions.

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
    let local_agent = agent_path()?;
    let deployment = match target {
        RemoteTarget::Ssh { .. } => deploy_ssh(target, &local_agent, &cancel)?,
        RemoteTarget::Container {
            engine, container, ..
        } => deploy_container(engine, container, &local_agent, &cancel)?,
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
    let mut command = agent_command(target, &deployment.path)?;
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
    // Clean up first: closing the last tab quits Brindle, which would end
    // this thread before it removed the container's helper.
    drop(deployment);
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
    if let Some(path) = std::env::var_os("BRINDLE_REMOTE_AGENT") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
        bail!(
            "BRINDLE_REMOTE_AGENT does not name a file: {}",
            path.display()
        );
    }
    let exe = std::env::current_exe().context("locating Brindle executable")?;
    for name in ["brindle-remote-linux-x86_64", "brindle-remote"] {
        let path = exe.with_file_name(name);
        if path.is_file() {
            return Ok(path);
        }
    }
    bail!(
        "Linux remote helper not found beside Brindle; set BRINDLE_REMOTE_AGENT to a Linux brindle-remote binary"
    )
}

/// Where the helper runs from on the endpoint, and what to remove after.
struct Deployment {
    path: String,
    cleanup: Option<ContainerCleanup>,
}

/// A container's private helper directory, removed when the session ends.
struct ContainerCleanup {
    engine: String,
    container: String,
    directory: String,
}

impl Drop for Deployment {
    fn drop(&mut self) {
        if let Some(cleanup) = &self.cleanup {
            let never_cancel = AtomicBool::new(false);
            container_exec_root(
                &cleanup.engine,
                &cleanup.container,
                &["rm", "-rf", &cleanup.directory],
                &never_cancel,
                "removing container helper",
                "container helper removal failed",
            )
            .ok();
        }
    }
}

fn hidden_command(program: &str) -> Command {
    let mut command = Command::new(program);
    hide_window(&mut command);
    command
}

/// Runs `args` as root inside the container, failing with `failure` (and the
/// command's stderr) when it exits unsuccessfully.
fn container_exec_root(
    engine: &str,
    container: &str,
    args: &[&str],
    cancel: &AtomicBool,
    what: &str,
    failure: &str,
) -> Result<()> {
    let mut command = hidden_command(engine);
    command.args(["exec", "--user", "0", container]).args(args);
    let (status, stderr) = run_status(command, cancel, what)?;
    if !status.success() {
        return Err(bootstrap_failure(failure, &stderr));
    }
    Ok(())
}

/// Runs a bootstrap command, returning its status and bounded stderr.
fn run_status(
    mut command: Command,
    cancel: &AtomicBool,
    what: &str,
) -> Result<(ExitStatus, String)> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .with_context(|| format!("starting {what}"))?;
    let drain = child.stderr.take().map(spawn_stderr_drain);
    let status =
        wait_process(child, cancel, Duration::from_secs(30)).with_context(|| what.to_string())?;
    let stderr = drain.map(|rx| await_stderr(&rx)).unwrap_or_default();
    Ok((status, stderr))
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

/// Makes the helper available on an SSH host, uploading it unless the cache
/// (keyed by version and digest) already has it.
fn deploy_ssh(target: &RemoteTarget, local: &Path, cancel: &AtomicBool) -> Result<Deployment> {
    let version = env!("CARGO_PKG_VERSION");
    let digest = file_sha256(local)?;
    let remote = format!("$HOME/{REMOTE_DIR}/{version}-{digest}/brindle-remote");
    let check = format!("test -x \"{remote}\" && test \"$(\"{remote}\" --version)\" = {version}");
    let command = super::remote_command(target, "sh", &["-c".into(), check])?;
    let (status, stderr) = run_status(command, cancel, "checking cached SSH remote helper")?;
    if status.success() {
        return Ok(Deployment {
            path: remote,
            cleanup: None,
        });
    }
    if status.code() == Some(255) {
        // ssh itself failed (auth, host key, network): uploading would too.
        return Err(bootstrap_failure("SSH connection failed", &stderr));
    }
    let script = format!(
        "umask 077; mkdir -p \"$HOME/{REMOTE_DIR}/{version}-{digest}\" && tmp=\"{remote}.tmp.$$\" && cat > \"$tmp\" && chmod 700 \"$tmp\" && mv \"$tmp\" \"{remote}\""
    );
    let mut upload = super::remote_command(target, "sh", &["-c".into(), script])?;
    upload
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = upload.spawn().context("starting SSH helper upload")?;
    let upload_stderr = child.stderr.take().map(spawn_stderr_drain);
    let mut input = child.stdin.take().context("SSH upload stdin unavailable")?;
    let mut file = File::open(local)
        .with_context(|| format!("opening remote helper {}", local.display()))?;
    let copy = thread::spawn(move || std::io::copy(&mut file, &mut input));
    let status = wait_process(child, cancel, Duration::from_secs(30))?;
    let copied = copy
        .join()
        .map_err(|_| anyhow::anyhow!("SSH upload thread panicked"))??;
    if !status.success() {
        let stderr = upload_stderr
            .map(|rx| await_stderr(&rx))
            .unwrap_or_default();
        return Err(bootstrap_failure(
            &format!("SSH helper upload failed after {copied} bytes"),
            &stderr,
        ));
    }
    Ok(Deployment {
        path: remote,
        cleanup: None,
    })
}

/// Copies the helper into a fresh private directory in the container; the
/// returned [`Deployment`] removes it when dropped.
fn deploy_container(
    engine: &str,
    container: &str,
    local: &Path,
    cancel: &AtomicBool,
) -> Result<Deployment> {
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).context("generating container helper path")?;
    let directory = format!("/tmp/brindle-{}", hex(&random));
    let deployment = Deployment {
        path: format!("{directory}/brindle-remote"),
        cleanup: Some(ContainerCleanup {
            engine: engine.to_owned(),
            container: container.to_owned(),
            directory: directory.clone(),
        }),
    };
    container_exec_root(
        engine,
        container,
        &["mkdir", "-m", "755", &directory],
        cancel,
        "creating private container helper directory",
        "container helper directory failed",
    )?;
    let mut command = hidden_command(engine);
    command
        .args(["cp"])
        .arg(local)
        .arg(format!("{container}:{}", deployment.path));
    let (status, stderr) = run_status(command, cancel, "copying container helper")?;
    if !status.success() {
        return Err(bootstrap_failure("container helper upload failed", &stderr));
    }
    container_exec_root(
        engine,
        container,
        &[
            "sh",
            "-c",
            // `docker cp` can keep the host file's owner, depending on the
            // daemon, so root takes the regular file over before checking.
            "test -f \"$1\" && test ! -L \"$1\" && chown 0:0 \"$1\" && chmod 755 \"$1\" && test \"$(stat -c %u \"$1\")\" = 0",
            "sh",
            &deployment.path,
        ],
        cancel,
        "verifying container helper",
        "container helper is not a root-owned regular file",
    )?;
    Ok(deployment)
}

fn agent_command(target: &RemoteTarget, remote_agent: &str) -> Result<Command> {
    match target {
        RemoteTarget::Ssh { .. } => super::remote_command(
            target,
            "sh",
            // Not a login shell: a profile that prints would corrupt the
            // framed stdout stream.
            &["-c".into(), format!("exec \"{remote_agent}\" --server")],
        ),
        RemoteTarget::Container { .. } => super::remote_command(
            target,
            "env",
            &[
                "BRINDLE_REMOTE_UNLINK=1".into(),
                remote_agent.into(),
                "--server".into(),
            ],
        ),
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

    fn wait_helper_dir_gone(container: &Container) -> bool {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let count = container.exec_sh("ls -d /tmp/brindle-* 2>/dev/null | wc -l");
            if count.trim() == "0" {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            thread::sleep(Duration::from_millis(250));
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
        record(
            "d helper dir removed",
            if wait_helper_dir_gone(&ubuntu) {
                Ok(())
            } else {
                Err(format!("still there: {}", ubuntu.exec_sh("ls -d /tmp/brindle-*")))
            },
        );
        record("g bad helper", scenario_bad_helper(&ubuntu, &scratch));
        unsafe { std::env::set_var("BRINDLE_REMOTE_AGENT", &helper) };
        match Container::start("alpine", "alpine") {
            Ok(alpine) => {
                record("h alpine run", scenario_clean_exit(&alpine, 1));
                record(
                    "h alpine cleanup",
                    if wait_helper_dir_gone(&alpine) {
                        Ok(())
                    } else {
                        Err(format!("still there: {}", alpine.exec_sh("ls -d /tmp/brindle-*")))
                    },
                );
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
        fn start(scratch: &Path) -> std::result::Result<Self, String> {
            let fixture_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/remote");
            let build = Command::new("docker")
                .args(["build", "-q", "-t", FIXTURE_TAG])
                .arg(&fixture_dir)
                .output()
                .map_err(|e| format!("docker build: {e}"))?;
            if !build.status.success() {
                return Err(format!("docker build: {}", String::from_utf8_lossy(&build.stderr)));
            }
            let container = Container::start_with(FIXTURE_TAG, "ssh", &["-p", "127.0.0.1::22"])?;
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
        let fx = SshFixture::start(&scratch).expect("ssh fixture");
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
        run_write_loop(
            &rx,
            Duration::from_millis(200),
            Duration::from_millis(900),
            &mut wire,
        )
        .unwrap();
        let frames = frames_in(&wire);
        let beats = frames.iter().filter(|f| f.msg_type == MSG_HEARTBEAT).count();
        assert!((2..=5).contains(&beats), "{beats} heartbeats in {frames:?}");
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
