//! Client transport and helper deployment for remote PTY sessions.

use std::fs::File;
use std::io::{Read, Write};
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
    ErrorMessage, Frame, MSG_DATA, MSG_ERROR, MSG_EXIT, MSG_PROCESS_INFO, MSG_SPAWN_RESPONSE,
    ProcessInfo, SessionKind, SpawnRequest, SpawnResponse,
};
use crate::config::{Profile, RemoteTarget};
use crate::terminal::GridSize;

const REMOTE_DIR: &str = ".cache/brindle";

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

fn write_data_frames(writer: &mut impl Write, channel_id: u32, bytes: &[u8]) -> Result<()> {
    for chunk in bytes.chunks(DATA_CHUNK_SIZE) {
        Frame::data(channel_id, chunk)?.write_to(writer)?;
    }
    Ok(())
}

/// A running remote PTY connection. Clones share one writer thread.
#[derive(Clone)]
pub struct Handle {
    tx: SyncSender<Outgoing>,
    cancel: Arc<AtomicBool>,
}

impl Handle {
    pub fn input(&self, bytes: Vec<u8>) {
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
    mut events: EventSender<Event>,
) -> Handle {
    let channel_id = 1;
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
                channel_id,
                rx,
                worker_cancel,
                &mut events,
            ) {
                let text =
                    format!("\r\n\x1b[31mBrindle remote connection failed:\x1b[0m\r\n{err:#}\r\n");
                send_event(&mut events, Event::Output(text.into_bytes()));
                send_event(&mut events, Event::Failed);
            }
        })
        .ok();
    Handle { tx, cancel }
}

fn send_event(events: &mut EventSender<Event>, event: Event) -> bool {
    futures::executor::block_on(events.send(event)).is_ok()
}

fn run_connection(
    profile: &Profile,
    command_override: Option<(String, Vec<String>)>,
    cwd_override: Option<PathBuf>,
    size: GridSize,
    channel_id: u32,
    outgoing: mpsc::Receiver<Outgoing>,
    cancel: Arc<AtomicBool>,
    events: &mut EventSender<Event>,
) -> Result<()> {
    let target = profile
        .remote
        .as_ref()
        .context("remote profile has no remote target")?;
    let local_agent = agent_path()?;
    let deployment = deploy_agent(target, &local_agent, &cancel)?;
    let (command, args) = command_override.unwrap_or_else(|| {
        (
            profile.command.clone().unwrap_or_default(),
            profile.args.clone(),
        )
    });
    let request = SpawnRequest {
        channel_id,
        kind: SessionKind::Pty,
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
    };
    let mut command = agent_command(target, deployment.path())?;
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(
        &mut command,
        crate::win::CREATE_NO_WINDOW,
    );
    let mut child = ChildGuard::new(command.spawn().context("starting remote agent")?);
    let mut stdin = child.take_stdin()?;
    let mut stdout = child.take_stdout()?;
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
    let writer = thread::spawn(move || -> Result<()> {
        loop {
            if writer_cancel.load(Ordering::Acquire) {
                // The transport may already be gone (the session ended on its
                // own, or the child was killed): a failed goodbye is no error.
                if let Ok(frame) = Frame::exit(channel_id, None) {
                    frame.write_to(&mut stdin).ok();
                }
                break;
            }
            let message = match outgoing.recv_timeout(Duration::from_millis(100)) {
                Ok(message) => message,
                Err(mpsc::RecvTimeoutError::Timeout) => continue,
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            };
            let frame = match message {
                Outgoing::Data(bytes) => {
                    write_data_frames(&mut stdin, channel_id, &bytes)?;
                    continue;
                }
                Outgoing::Resize(size) => Frame::resize(
                    channel_id,
                    size.cols,
                    size.rows,
                    size.cell_width,
                    size.cell_height,
                )?,
                Outgoing::QueryProcess => Frame::query_process(channel_id)?,
            };
            frame.write_to(&mut stdin)?;
        }
        Ok(())
    });
    let stderr_reader = spawn_stderr_drain(stderr);

    let protocol_result = (|| -> Result<()> {
        super::protocol::skip_preamble(&mut stdout)
            .context("waiting for the remote agent's frame marker")?;
        let first = Frame::read_from(&mut stdout).context("reading remote agent handshake")?;
        if first.channel_id != super::protocol::CHANNEL_CONTROL
            || first.msg_type != MSG_SPAWN_RESPONSE
        {
            bail!(
                "remote agent sent channel {} message {} before spawn response",
                first.channel_id,
                first.msg_type
            );
        }
        let response: SpawnResponse = first.decode_json()?;
        if response.channel_id != channel_id {
            bail!(
                "remote agent spawn response is for channel {} instead of {channel_id}",
                response.channel_id
            );
        }
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
            if frame.channel_id != channel_id {
                continue;
            }
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
                    if !send_event(
                        events,
                        Event::Output(
                            format!(
                                "\r\n\x1b[31mRemote agent error:\x1b[0m {}\r\n",
                                error.message
                            )
                            .into_bytes(),
                        ),
                    ) {
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
        send_event(
            events,
            Event::Output(format!("\r\n{diagnostic}\r\n").into_bytes()),
        );
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

struct Deployment {
    path: String,
    cleanup: Option<(String, String, String)>,
}

impl Deployment {
    fn path(&self) -> &str {
        &self.path
    }
}

impl Drop for Deployment {
    fn drop(&mut self) {
        if let Some((engine, container, directory)) = &self.cleanup {
            let mut command = hidden_command(engine);
            command.args([
                "exec",
                "--user",
                "0",
                container,
                "rm",
                "-rf",
                directory,
            ]);
            let never_cancel = AtomicBool::new(false);
            run_status(command, &never_cancel, "removing container helper").ok();
        }
    }
}

fn hidden_command(program: &str) -> Command {
    #[allow(unused_mut)] // Mutated below only on Windows for CREATE_NO_WINDOW.
    let mut command = Command::new(program);
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(
        &mut command,
        crate::win::CREATE_NO_WINDOW,
    );
    command
}

fn wait_process(mut child: Child, cancel: &AtomicBool, timeout: Duration) -> Result<ExitStatus> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().context("waiting for bootstrap process")? {
            return Ok(status);
        }
        if cancel.load(Ordering::Acquire) || Instant::now() >= deadline {
            child.kill().ok();
            return child.wait().context("reaping bootstrap process");
        }
        thread::sleep(Duration::from_millis(20));
    }
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
    Ok(hasher.finalize().iter().map(|byte| format!("{byte:02x}")).collect())
}

fn deploy_agent(target: &RemoteTarget, local: &Path, cancel: &AtomicBool) -> Result<Deployment> {
    let version = env!("CARGO_PKG_VERSION");
    let digest = file_sha256(local)?;
    match target {
        RemoteTarget::Ssh { .. } => {
            let remote = format!("$HOME/{REMOTE_DIR}/{version}-{digest}/brindle-remote");
            let check = format!("test -x \"{remote}\" && test \"$(\"{remote}\" --version)\" = {version}");
            let command = super::remote_command(target, "sh", &["-lc".into(), check])?;
            let (status, stderr) =
                run_status(command, cancel, "checking cached SSH remote helper")?;
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
            let mut upload = super::remote_command(target, "sh", &["-lc".into(), script])?;
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
        RemoteTarget::Container {
            engine, container, ..
        } => {
            let mut random = [0u8; 16];
            getrandom::fill(&mut random).context("generating container helper path")?;
            let nonce: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
            let directory = format!("/tmp/brindle-{nonce}");
            let remote = format!("{directory}/brindle-remote");
            let deployment = Deployment {
                path: remote,
                cleanup: Some((engine.clone(), container.clone(), directory.clone())),
            };
            let mut command = hidden_command(engine);
            command.args([
                "exec", "--user", "0", container, "mkdir", "-m", "755", &directory,
            ]);
            let (status, stderr) = run_status(
                command,
                cancel,
                "creating private container helper directory",
            )?;
            if !status.success() {
                return Err(bootstrap_failure(
                    "container helper directory failed",
                    &stderr,
                ));
            }
            let destination = format!("{container}:{}", deployment.path());
            let mut command = hidden_command(engine);
            command.args(["cp"]).arg(local).arg(&destination);
            let (status, stderr) = run_status(command, cancel, "copying container helper")?;
            if !status.success() {
                return Err(bootstrap_failure("container helper upload failed", &stderr));
            }
            let mut command = hidden_command(engine);
            command.args([
                "exec",
                "--user",
                "0",
                container,
                "sh",
                "-c",
                "test -f \"$1\" && test \"$(stat -c %u \"$1\")\" = 0",
                "sh",
                deployment.path(),
            ]);
            let (status, stderr) = run_status(command, cancel, "verifying container helper")?;
            if !status.success() {
                return Err(bootstrap_failure(
                    "container helper is not a regular file",
                    &stderr,
                ));
            }
            let mut command = hidden_command(engine);
            command.args([
                "exec",
                "--user",
                "0",
                container,
                "chmod",
                "755",
                deployment.path(),
            ]);
            let (status, stderr) =
                run_status(command, cancel, "marking container helper executable")?;
            if !status.success() {
                return Err(bootstrap_failure("container helper chmod failed", &stderr));
            }
            Ok(deployment)
        }
    }
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
        let scratch = std::env::temp_dir().join(format!("brindle-client-test-{}", std::process::id()));
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
    }

    impl RawAgent {
        fn start(
            target: &RemoteTarget,
            remote_path: &str,
            command: &str,
            args: &[&str],
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
                channel_id: 1,
                kind: SessionKind::Pty,
                command: Some(command.into()),
                args: args.iter().map(|a| a.to_string()).collect(),
                cwd: None,
                env: crate::terminal::child_env(&Profile::default_shell()).into_iter().collect(),
                cols: 80,
                rows: 24,
                cell_width: 8,
                cell_height: 16,
            };
            Frame::spawn(&request).and_then(|f| f.write_to(&mut stdin)).map_err(|e| e.to_string())?;
            let (tx, frames) = mpsc::channel();
            thread::spawn(move || {
                if super::super::protocol::skip_preamble(&mut stdout).is_err() {
                    return;
                }
                while let Ok(Some(frame)) = Frame::read_from_or_eof(&mut stdout) {
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
            Ok(Self { child, stdin: Some(stdin), _frames: frames })
        }

        fn input(&mut self, text: &str) {
            let frame = Frame::data(1, text.as_bytes()).unwrap();
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
        let mut agent = RawAgent::start(target, remote_path, shell, shell_args)?;
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
        let scratch = std::env::temp_dir().join(format!("brindle-ssh-test-{}", std::process::id()));
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

    #[test]
    fn large_input_is_chunked_without_loss() {
        let bytes: Vec<u8> = (0..2 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
        let mut wire = Vec::new();
        write_data_frames(&mut wire, 7, &bytes).unwrap();
        let mut cursor = std::io::Cursor::new(wire);
        let mut decoded = Vec::new();
        let mut frames = 0;
        while let Some(frame) = Frame::read_from_or_eof(&mut cursor).unwrap() {
            assert_eq!(frame.channel_id, 7);
            assert_eq!(frame.msg_type, MSG_DATA);
            assert!(frame.payload.len() <= DATA_CHUNK_SIZE);
            decoded.extend(frame.payload);
            frames += 1;
        }
        assert!(frames > 1);
        assert_eq!(decoded, bytes);
    }
}
