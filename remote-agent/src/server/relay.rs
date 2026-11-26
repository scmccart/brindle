//! `brindle-remote relay`: a client of its host's core that pipes the core's
//! protocol to its stdio, so a Brindle on another machine reaches the server
//! over any byte stream (`ssh`, a container engine's `exec`). Linux only.
//!
//! In order:
//! 1. **Marker:** stdout gets `FRAME_STREAM_MARKER` and from then on only
//!    frames. A login shell may print before the relay runs; the client skips
//!    that (`skip_preamble`). With `--login-environment` (the SSH launch
//!    passes it, a container's doesn't), the relay then starts reading the
//!    user's login shell's environment on a thread of its own: the shell
//!    ([`login_env::invocation`], `$SHELL -l -i -c`) runs in a session of its
//!    own with stdin and stdout on `/dev/null`, stderr on a pipe whose last
//!    4 KiB are kept for the reason, and file descriptor 3 on the dump pipe;
//!    it `exec`s this binary's hidden `login-env` role, which writes its
//!    environment there ([`login_env::encode_dump`]). Nothing the startup
//!    files print reaches stdout. The dump is read up to its terminator, for
//!    at most [`login_env::LIMIT`] (`BRINDLE_TEST_RELAY_LOGIN_ENV_MS` for
//!    tests), after which the shell's process group gets `SIGKILL`; a thread
//!    reaps the shell either way.
//! 2. **Core:** the server directory is resolved as a local client resolves
//!    it (created if missing), and the relay connects to the core, starting
//!    one from its own binary (`/proc/<pid>/exe serve`) when none listens. The
//!    core goes into a systemd scope of its own only when the user lingers
//!    (`/var/lib/systemd/linger/<user>`) and a user manager is reachable:
//!    without lingering, the user manager stops after the last login and
//!    takes its scopes with it. Otherwise the core starts plain, in the
//!    login's session (which outlives the login unless the host sets
//!    `KillUserProcesses=yes`), with `BRINDLE_NO_SCOPE=1` so its holders do
//!    too, and `core.log` gets one line saying so and why. The line adds that
//!    the sessions may not outlive the login only when a user manager is
//!    reachable (the user doesn't linger, or `systemd-run` is missing): a
//!    host with no user manager, a container for one, or `BRINDLE_NO_SCOPE=1`
//!    gets the plain line, since no scope was possible to ask for.
//! 3. **Upgrade:** on a connection of its own (token `relay-<pid>-upgrade`,
//!    which owns nothing), the relay reads `WELCOME` and compares its build
//!    with the core's by the rules a local client uses
//!    (`upgrade::is_newer`; the core's file time through
//!    `/proc/<core pid>/exe`). When it is newer and drives every running
//!    holder's H1 version ([`check::refusal`]), it sends `STEP_DOWN`, waits
//!    for the core to exit, starts `serve` as in step 2 and connects again,
//!    waiting for the holders' own core if its own won't start. A refused
//!    check or step-down leaves the running core, which it relays to.
//! 4. **Host:** before it opens the client's connection to the core, the
//!    relay waits for the login environment it was asked for, so that wait
//!    isn't spent from the core's `HELLO` limit; a failure gets one
//!    `core.log` line naming it, never the environment's values. Then
//!    `RELAY_HOST` (version, build, its own environment with the passwd
//!    basics ([`login_env::with_basics`]), the login shell from `SHELL` else
//!    passwd, home, server directory, core pid, and the login environment
//!    or why it wasn't read), or `RELAY_ERROR` naming the problem and the
//!    core's log, after which it exits. The client's connection to the core
//!    is open by then, so the client's `HELLO` must follow within the core's
//!    limit (5 s).
//! 5. **Pipe:** whole frames are copied both ways by their headers. The
//!    client's relay frames (`RELAY_HEARTBEAT`) are dropped, so no core ever
//!    sees one. The relay ends when either side closes.
//!
//! **Liveness:** the stdin reader thread stamps a [`Liveness`] on every frame
//! it reads, heartbeats included, and is busy while handing one to the core;
//! the main thread ends the relay after [`IDLE_LIMIT`] without one. A
//! heartbeat thread sends `RELAY_HEARTBEAT` after [`HEARTBEAT_AFTER`]
//! without a write, under the writer lock that the core→stdout copier takes
//! for each whole frame, so heartbeats fall between frames. A client that
//! stops reading blocks only the copier and the heartbeat thread, never the
//! reader or the watch, so a slow reader isn't silence. For tests,
//! `BRINDLE_TEST_RELAY_HEARTBEAT_MS` and `BRINDLE_TEST_RELAY_IDLE_MS` replace
//! the two timings.
//!
//! **`login-env`** ([`login_env_role`]), hidden from the usage text, is the
//! role the login shell `exec`s: it writes its environment to file
//! descriptor 3 and exits.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fmt::Display;
use std::fs::{self, File};
use std::io::{self, BufReader, Read as _, Write as _};
use std::os::fd::{AsFd as _, AsRawFd as _, FromRawFd as _, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use brindle_remote_protocol::client::{self, Hello, Request, ToClient, ToCore, Welcome};
use brindle_remote_protocol::ready::ReadyRead;
use brindle_remote_protocol::relay::{self, LoginEnv, RelayError, RelayHost, RelayMessage};
use brindle_remote_protocol::upgrade::{self, Build};
use brindle_remote_protocol::{FRAME_STREAM_MARKER, Frame, HEARTBEAT_INTERVAL, scope, server_dir};

use super::login_env::{self, DumpRead, LoginEnvFailure};
use super::{build_id, check, daemon, dir};
use crate::liveness::Liveness;

/// The relay sends a heartbeat after this long without writing to its client.
pub const HEARTBEAT_AFTER: Duration = HEARTBEAT_INTERVAL;
/// The relay ends after this long without a frame from its client.
pub const IDLE_LIMIT: Duration = Duration::from_secs(60);
const HEARTBEAT_ENV: &str = "BRINDLE_TEST_RELAY_HEARTBEAT_MS";
const IDLE_ENV: &str = "BRINDLE_TEST_RELAY_IDLE_MS";
const LOGIN_ENV_ENV: &str = "BRINDLE_TEST_RELAY_LOGIN_ENV_MS";
/// The argument that asks the relay for the login shell's environment.
pub const LOGIN_ENVIRONMENT_ARG: &str = "--login-environment";
/// The file descriptor the `login-env` role writes its dump to.
const DUMP_FD: RawFd = 3;
/// How long a started core may take to say it is ready.
const START_LIMIT: Duration = Duration::from_secs(5);
/// How long the relay keeps trying a core it started: one that lost a start
/// race says it is ready once the winner listens.
const CONNECT_LIMIT: Duration = Duration::from_secs(5);
/// After a step-down, how long the relay waits for a core: its own, or the
/// one the holders start when its own won't.
const UPGRADE_CONNECT_LIMIT: Duration = Duration::from_secs(15);
const HELLO_LIMIT: Duration = Duration::from_secs(10);
/// How long a stepping-down core may take to checkpoint and answer.
const STEP_DOWN_LIMIT: Duration = Duration::from_secs(60);
/// How long a stepped-down core may take to exit. Until it has, a new `serve`
/// can't take the lock, and would count the exiting core as the winner.
const EXIT_LIMIT: Duration = Duration::from_secs(10);
/// Where systemd keeps a file per user whose user manager outlives their
/// logins (`loginctl enable-linger`).
const LINGER_DIR: &str = "/var/lib/systemd/linger";

pub fn run(args: &[String]) -> Result<(), String> {
    let asked = login_asked(args)?;
    let timings = Timings::from_env();
    let output = Arc::new(Output::stdout().map_err(|error| format!("relay: no stdout: {error}"))?);
    output
        .raw(FRAME_STREAM_MARKER)
        .map_err(|error| format!("relay: can't write to the client: {error}"))?;
    let env = login_env::with_basics(own_env(), passwd().as_ref());
    let capture = asked.then(|| Capture::start(&env, timings.login));
    let reached = match reach(capture) {
        Ok(reached) => reached,
        Err(message) => {
            let error = RelayMessage::Error(RelayError {
                message: message.clone(),
            });
            let _ = output.message(&error);
            return Err(message);
        }
    };
    output
        .message(&RelayMessage::Host(reached.host(env)))
        .map_err(|error| format!("relay: can't write to the client: {error}"))?;
    pipe(reached, output, timings)
}

/// Whether the relay's arguments ask for the login shell's environment:
/// none, or [`LOGIN_ENVIRONMENT_ARG`] alone; anything else is a usage error.
fn login_asked(args: &[String]) -> Result<bool, String> {
    match args {
        [] => Ok(false),
        [arg] if arg == LOGIN_ENVIRONMENT_ARG => Ok(true),
        _ => Err(format!("usage: brindle-remote relay [{LOGIN_ENVIRONMENT_ARG}]")),
    }
}

/// The relay's own environment, without the variables that aren't UTF-8.
fn own_env() -> BTreeMap<String, String> {
    std::env::vars_os()
        .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
        .collect()
}

/// The current user's passwd entry, for [`login_env::with_basics`].
fn passwd() -> Option<login_env::Passwd> {
    crate::pty::passwd_entry().map(|entry| login_env::Passwd {
        name: entry.name.to_string_lossy().into_owned(),
        home: entry.home.to_string_lossy().into_owned(),
        shell: entry.shell.to_string_lossy().into_owned(),
    })
}

/// `brindle-remote login-env`: writes this process's environment to file
/// descriptor 3 ([`login_env::encode_dump`]) and exits. The login shell
/// `exec`s it; it takes no arguments, and fails when fd 3 isn't open.
pub fn login_env_role(args: &[String]) -> Result<(), String> {
    use std::os::unix::ffi::OsStringExt as _;

    if !args.is_empty() {
        return Err(format!("usage: brindle-remote {}", login_env::ROLE));
    }
    // SAFETY: `F_GETFD` only asks whether the descriptor is open.
    if unsafe { libc::fcntl(DUMP_FD, libc::F_GETFD) } == -1 {
        return Err(format!(
            "{}: file descriptor {DUMP_FD} isn't open: {}",
            login_env::ROLE,
            io::Error::last_os_error()
        ));
    }
    // SAFETY: the descriptor is open (above), and this process owns it
    // from here: nothing else in it uses fd 3.
    let mut out = unsafe { File::from_raw_fd(DUMP_FD) };
    let dump = login_env::encode_dump(
        std::env::vars_os().map(|(name, value)| (name.into_vec(), value.into_vec())),
    );
    out.write_all(&dump)
        .map_err(|error| format!("{}: can't write the environment: {error}", login_env::ROLE))
}

/// The login shell's environment being read on its own thread.
struct Capture(mpsc::Receiver<Result<BTreeMap<String, String>, LoginEnvFailure>>);

impl Capture {
    /// Starts reading it, the shell started from `env`, for at most `limit`.
    fn start(env: &BTreeMap<String, String>, limit: Duration) -> Self {
        let (done, outcome) = mpsc::channel();
        let env = env.clone();
        thread::spawn(move || {
            let _ = done.send(capture(&env, limit));
        });
        Self(outcome)
    }

    /// Waits for the outcome, which [`capture`] bounds by its limit.
    fn join(self) -> Result<BTreeMap<String, String>, LoginEnvFailure> {
        self.0.recv().unwrap_or_else(|_| {
            Err(LoginEnvFailure::Exited {
                status: login_env::exit_text(None, None),
                stderr: "the relay's reading thread stopped".into(),
            })
        })
    }
}

/// Runs the login shell (`env`'s `SHELL`, which the passwd basics filled)
/// and reads its environment, killing its process group after `limit`.
fn capture(
    env: &BTreeMap<String, String>,
    limit: Duration,
) -> Result<BTreeMap<String, String>, LoginEnvFailure> {
    let deadline = Instant::now() + limit;
    let shell = env.get("SHELL").cloned().unwrap_or_default();
    let spawn_failed = |error: String| LoginEnvFailure::Spawn {
        shell: shell.clone(),
        error,
    };
    let helper = std::env::current_exe()
        .map_err(|error| spawn_failed(format!("can't find this helper: {error}")))?;
    let helper = helper
        .to_str()
        .ok_or_else(|| spawn_failed(format!("this helper's path {} isn't UTF-8", helper.display())))?;
    let command = login_env::invocation(&shell, helper)?;
    let started = LoginShell::spawn(&command, env).map_err(|error| spawn_failed(error.to_string()))?;
    started.read(deadline, limit, env)
}

/// A login shell started by [`capture`], before its dump is read.
struct LoginShell {
    /// Its process id, which is its process group's (it has a session of
    /// its own).
    pid: u32,
    dump: File,
    stderr: File,
    /// Readable, then at its end, once the shell has exited and been
    /// reaped: a thread waits for it.
    exited: File,
    status: mpsc::Receiver<io::Result<std::process::ExitStatus>>,
}

impl LoginShell {
    fn spawn(command: &[String], env: &BTreeMap<String, String>) -> io::Result<Self> {
        let (program, args) = command
            .split_first()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "no program"))?;
        let (dump_read, dump_write) = os_pipe()?;
        let mut shell = std::process::Command::new(program);
        shell
            .args(args)
            .envs(env)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        if let Some(home) = env.get("HOME").filter(|home| Path::new(home).is_dir()) {
            shell.current_dir(home);
        }
        let dump_fd = dump_write.as_raw_fd();
        // Asked here, since the child may make only async-signal-safe calls.
        let last_signal = libc::SIGRTMAX();
        // SAFETY: the closure makes only async-signal-safe calls.
        unsafe {
            shell.pre_exec(move || {
                crate::pty::default_signals(last_signal)?;
                // Its own session and process group, with no controlling
                // terminal, so a timeout can end the whole group.
                if libc::setsid() == -1 {
                    return Err(io::Error::last_os_error());
                }
                // The dump pipe on fd 3, open across the exec (`dup2`
                // leaves the copy without close-on-exec).
                let kept = if dump_fd == DUMP_FD {
                    libc::fcntl(DUMP_FD, libc::F_SETFD, 0)
                } else {
                    libc::dup2(dump_fd, DUMP_FD)
                };
                if kept == -1 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = shell.spawn()?;
        drop(dump_write);
        let stderr = child
            .stderr
            .take()
            .map(OwnedFd::from)
            .ok_or_else(|| io::Error::other("no stderr pipe"))?;
        let (exited, exited_write) = os_pipe()?;
        for fd in [&dump_read, &stderr, &exited] {
            set_nonblocking(fd)?;
        }
        let pid = child.id();
        let (reaped, status) = mpsc::channel();
        thread::spawn(move || {
            let _ = reaped.send(child.wait());
            drop(exited_write);
        });
        Ok(Self {
            pid,
            dump: File::from(dump_read),
            stderr: File::from(stderr),
            exited: File::from(exited),
            status,
        })
    }

    /// Reads the dump up to its terminator, until `deadline`.
    fn read(
        mut self,
        deadline: Instant,
        limit: Duration,
        own: &BTreeMap<String, String>,
    ) -> Result<BTreeMap<String, String>, LoginEnvFailure> {
        let mut dump = Vec::new();
        let mut tail = Vec::new();
        let (mut dump_open, mut stderr_open) = (true, true);
        loop {
            let now = Instant::now();
            if now >= deadline {
                self.kill();
                return Err(LoginEnvFailure::TimedOut { after: limit });
            }
            let wait = (deadline - now).as_millis().saturating_add(1).min(i32::MAX as u128) as i32;
            let fd = |file: &File, open: bool| if open { file.as_raw_fd() } else { -1 };
            let mut fds = [
                pollfd(fd(&self.dump, dump_open)),
                pollfd(fd(&self.stderr, stderr_open)),
                pollfd(self.exited.as_raw_fd()),
            ];
            // SAFETY: `fds` is a valid array of its length.
            if unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, wait) } == -1 {
                let error = io::Error::last_os_error();
                if error.kind() == io::ErrorKind::Interrupted {
                    continue;
                }
                self.kill();
                return Err(LoginEnvFailure::Exited {
                    status: login_env::exit_text(None, None),
                    stderr: format!("the relay couldn't wait for it: {error}"),
                });
            }
            if fds[1].revents != 0 {
                stderr_open = drain(&mut self.stderr, |bytes| login_env::keep_tail(&mut tail, bytes));
            }
            if fds[0].revents != 0 {
                dump_open = drain(&mut self.dump, |bytes| dump.extend_from_slice(bytes));
                match login_env::parse_dump(&dump) {
                    DumpRead::Complete(vars) => return Ok(login_env::captured(own, vars)),
                    DumpRead::TooLarge => {
                        self.kill();
                        return Err(LoginEnvFailure::TooLarge);
                    }
                    DumpRead::Incomplete => {}
                }
            }
            if fds[2].revents != 0 {
                // It exited: what it wrote is in the pipes already.
                drain(&mut self.dump, |bytes| dump.extend_from_slice(bytes));
                drain(&mut self.stderr, |bytes| login_env::keep_tail(&mut tail, bytes));
                let status = match self.status.recv() {
                    Ok(Ok(status)) => {
                        use std::os::unix::process::ExitStatusExt as _;
                        login_env::exit_text(status.code(), status.signal())
                    }
                    Ok(Err(_)) | Err(_) => login_env::exit_text(None, None),
                };
                return login_env::after_exit(&dump, status, &tail)
                    .map(|vars| login_env::captured(own, vars));
            }
        }
    }

    /// Ends the shell's process group. What double-forked out of it keeps
    /// running, as after an SSH login.
    fn kill(&self) {
        if let Ok(pid) = libc::pid_t::try_from(self.pid) {
            // SAFETY: a signal to the group this shell leads.
            unsafe { libc::kill(-pid, libc::SIGKILL) };
        }
    }
}

fn pollfd(fd: RawFd) -> libc::pollfd {
    libc::pollfd {
        fd,
        events: libc::POLLIN,
        revents: 0,
    }
}

/// Reads what `file` (non-blocking) has into `take`; whether it is still
/// open (no end or error seen).
fn drain(file: &mut File, mut take: impl FnMut(&[u8])) -> bool {
    let mut buffer = [0u8; 16 * 1024];
    loop {
        match file.read(&mut buffer) {
            Ok(0) => return false,
            Ok(read) => take(&buffer[..read]),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return true,
            Err(_) => return false,
        }
    }
}

/// A pipe's ends, both close-on-exec: `(read, write)`.
fn os_pipe() -> io::Result<(OwnedFd, OwnedFd)> {
    let mut fds = [-1; 2];
    // SAFETY: `fds` has room for the two fds `pipe2` writes.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: both fds are new and owned by nothing else.
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

fn set_nonblocking(fd: &OwnedFd) -> io::Result<()> {
    // SAFETY: flag queries and changes on an open fd.
    unsafe {
        let flags = libc::fcntl(fd.as_raw_fd(), libc::F_GETFL);
        if flags == -1 || libc::fcntl(fd.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) == -1 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// The timings, as tests may shorten them.
#[derive(Clone, Copy)]
struct Timings {
    heartbeat: Duration,
    idle: Duration,
    /// How long the login shell's environment may take.
    login: Duration,
}

impl Timings {
    fn from_env() -> Self {
        let millis = |name: &str| {
            std::env::var(name)
                .ok()?
                .parse::<u64>()
                .ok()
                .filter(|&ms| ms > 0)
                .map(Duration::from_millis)
        };
        Self {
            heartbeat: millis(HEARTBEAT_ENV).unwrap_or(HEARTBEAT_AFTER),
            idle: millis(IDLE_ENV).unwrap_or(IDLE_LIMIT),
            login: millis(LOGIN_ENV_ENV).unwrap_or(login_env::LIMIT),
        }
    }
}

/// Lines in the server directory's `core.log`, beside the core's own.
struct Log(PathBuf);

impl Log {
    fn line(&self, text: impl Display) {
        if let Ok(mut log) = daemon::open_log(&self.0) {
            let _ = writeln!(
                log,
                "{} relay[{}]: {text}",
                daemon::timestamp(),
                std::process::id()
            );
        }
    }
}

/// The relay's stdout: the client's frames, one writer at a time.
struct Output(Mutex<Writer>);

struct Writer {
    out: File,
    /// When the client was last written to, for the heartbeats.
    last_write: Instant,
}

impl Writer {
    fn frame(&mut self, frame: &Frame) -> io::Result<()> {
        frame.write_to(&mut self.out)?;
        self.last_write = Instant::now();
        Ok(())
    }
}

impl Output {
    fn stdout() -> io::Result<Self> {
        let out = File::from(io::stdout().as_fd().try_clone_to_owned()?);
        Ok(Self(Mutex::new(Writer {
            out,
            last_write: Instant::now(),
        })))
    }

    fn lock(&self) -> MutexGuard<'_, Writer> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn raw(&self, bytes: &[u8]) -> io::Result<()> {
        let mut writer = self.lock();
        writer.out.write_all(bytes)?;
        writer.last_write = Instant::now();
        Ok(())
    }

    fn frame(&self, frame: &Frame) -> io::Result<()> {
        self.lock().frame(frame)
    }

    fn message(&self, message: &RelayMessage) -> io::Result<()> {
        self.frame(&message.encode()?)
    }
}

/// The core, reached.
struct Reached {
    dir: PathBuf,
    build: Result<String, String>,
    log: Log,
    /// The client's connection to the core, before its `HELLO`.
    core: UnixStream,
    /// 0 when the core shares no protocol version with the relay.
    core_pid: u32,
    /// The login shell's environment, when it was asked for.
    login: Option<LoginEnv>,
}

impl Reached {
    /// The host frame, with `env`, the relay's own environment with the
    /// passwd basics.
    fn host(&self, env: BTreeMap<String, String>) -> RelayHost {
        let passwd_shell = crate::pty::passwd_entry()
            .map(|entry| entry.shell.to_string_lossy().into_owned());
        // No shell is the wire's empty field (`types R9`).
        let shell = login_shell(env.get("SHELL").cloned(), passwd_shell).unwrap_or_default();
        let home = crate::pty::home_dir(&BTreeMap::new())
            .map(|home| home.to_string_lossy().into_owned())
            .unwrap_or_default();
        RelayHost {
            version: env!("CARGO_PKG_VERSION").into(),
            build: host_build(&self.build).to_string(),
            env,
            shell,
            home,
            server_dir: self.dir.display().to_string(),
            core_pid: self.core_pid,
            login: self.login.clone(),
        }
    }
}

/// What `RELAY_HOST` says of a capture, and the `core.log` line for a
/// failed one, which names the reason and never a value.
fn login_outcome(
    outcome: Result<BTreeMap<String, String>, LoginEnvFailure>,
) -> (LoginEnv, Option<String>) {
    match outcome {
        Ok(env) => (LoginEnv::Captured(env), None),
        Err(failure) => {
            let text = failure.text();
            let line = format!("the login shell's environment wasn't read: {text}");
            (LoginEnv::Failed(text), Some(line))
        }
    }
}

/// The user's login shell, which `RELAY_HOST` carries: `SHELL` when it is
/// set and not empty, else the passwd database's (`docker exec` sets no
/// `SHELL`); `None` when neither names one.
fn login_shell(shell_var: Option<String>, passwd_shell: Option<String>) -> Option<String> {
    shell_var.filter(|shell| !shell.is_empty()).or(passwd_shell)
}

/// The build id `RELAY_HOST` carries: this binary's, or `""`, the wire's
/// "unknown" (`process R5`), when it couldn't be read.
fn host_build(build: &Result<String, String>) -> &str {
    build.as_deref().unwrap_or_default()
}

/// Resolves the server directory, reaches the core (starting or upgrading
/// it), waits for the login environment's `capture` when there is one, and
/// opens the client's connection; or says what went wrong, with the core's
/// log when there is a directory for it.
fn reach(capture: Option<Capture>) -> Result<Reached, String> {
    let dir = dir::from_env()
        .map_err(|error| format!("terminal server unreachable: {error}"))?
        .path()
        .to_path_buf();
    let server = Server {
        socket: dir.join(server_dir::CORE_SOCKET),
        log: Log(dir.join(server_dir::CORE_LOG)),
        build: build_id::current().map_err(|error| error.to_string()),
        dir,
    };
    if let Err(error) = &server.build {
        server.log.line(format_args!(
            "can't read its own build id, so it offers no upgrade and tells the client none: {error}"
        ));
    }
    let failure = |problem: String| {
        format!(
            "terminal server unreachable: {problem}; its log is {}",
            server.log.0.display()
        )
    };
    let probe = server.probe(false).map_err(failure)?;
    let probe = server.offer_upgrade(probe).map_err(failure)?;
    // Before the client's connection opens, so the wait isn't spent from the
    // core's limit for its `HELLO`.
    let login = capture.map(|capture| {
        let (login, line) = login_outcome(capture.join());
        if let Some(line) = line {
            server.log.line(line);
        }
        login
    });
    // The probe stays open until then, so a core with nothing else to do
    // doesn't exit in between.
    let core = server.connect().map_err(failure)?;
    let core_pid = probe.welcome.as_ref().map_or(0, |welcome| welcome.pid);
    drop(probe);
    Ok(Reached {
        dir: server.dir,
        build: server.build,
        log: server.log,
        core,
        core_pid,
        login,
    })
}

struct Server {
    dir: PathBuf,
    socket: PathBuf,
    log: Log,
    /// This binary's build id, or why it can't be read (then no upgrade is
    /// offered, and the log says so once).
    build: Result<String, String>,
}

/// A connection of the relay's own to the core, past `HELLO`.
struct Probe {
    stream: UnixStream,
    /// `None` when the core shares no protocol version with the relay: the
    /// client's own `HELLO` gets the core's reason.
    welcome: Option<Welcome>,
}

impl Server {
    fn connect(&self) -> Result<UnixStream, String> {
        UnixStream::connect(&self.socket)
            .map_err(|error| format!("can't connect to {}: {error}", self.socket.display()))
    }

    /// Connects and says `HELLO`, starting a core when none listens. After
    /// an upgrade (`after_upgrade`) a core that won't start isn't the end:
    /// the holders start one of their own, so it keeps trying for
    /// [`UPGRADE_CONNECT_LIMIT`].
    fn probe(&self, after_upgrade: bool) -> Result<Probe, String> {
        let mut deadline = after_upgrade.then(|| Instant::now() + UPGRADE_CONNECT_LIMIT);
        let mut start_error = None;
        let mut started = false;
        loop {
            let waiting = deadline.is_some_and(|deadline| Instant::now() < deadline);
            match UnixStream::connect(&self.socket) {
                Ok(stream) => match hello(stream) {
                    Ok(probe) => return Ok(probe),
                    // A core still exiting, or one starting.
                    Err(_) if waiting => {}
                    Err(error) => return Err(error),
                },
                Err(error) if no_listener(&error) => {
                    if !started {
                        started = true;
                        if let Err(error) = self.start_core() {
                            if !after_upgrade {
                                return Err(error);
                            }
                            self.log
                                .line(format_args!("the upgraded core didn't start: {error}"));
                            start_error = Some(error);
                        }
                        deadline.get_or_insert_with(|| Instant::now() + CONNECT_LIMIT);
                        continue;
                    }
                    if !waiting {
                        return Err(start_error.unwrap_or_else(|| {
                            format!("no core listens on {}", self.socket.display())
                        }));
                    }
                }
                Err(error) => {
                    return Err(format!(
                        "can't connect to {}: {error}",
                        self.socket.display()
                    ));
                }
            }
            thread::sleep(Duration::from_millis(50));
        }
    }

    /// Starts a core from this binary, in a scope only when the user lingers
    /// ([`decide_scope`]), and waits for it to be ready.
    fn start_core(&self) -> Result<(), String> {
        let user = crate::pty::user_name();
        let lingers = lingers(user.as_deref(), |path| path.exists());
        let ScopeChoice { decision, note } = decide_scope(scope::current_outcome(), lingers);
        let mut env: Vec<(&str, &OsStr)> = vec![("BRINDLE_SERVER_DIR", self.dir.as_os_str())];
        if let Some(note) = note {
            self.log.line(format_args!("{note}"));
            env.push((scope::NO_SCOPE_ENV, OsStr::new("1")));
        }
        let started = daemon::start_role_as(
            &decision,
            "serve",
            &daemon::role_args("serve"),
            &env,
            |command| {
                // The relay's stdout is its client's: nothing else writes
                // there.
                command.stdout(Stdio::null()).stderr(Stdio::piped());
            },
        )
        .map_err(|error| format!("can't start the core: {error}"))?;
        let (mut child, mut ready, program) = (started.child, started.ready, started.program);
        let ready = ready.wait(START_LIMIT);
        if matches!(ready, ReadyRead::Pending) {
            // A starter that hasn't detached the core by now won't.
            let _ = child.kill();
        }
        let output = child.wait_with_output();
        // What the pipe says beside what the starter said, if anything.
        let pipe = match ready {
            ReadyRead::Listening => return Ok(()),
            ReadyRead::Ended => None,
            ReadyRead::Pending => Some(format!("no ready signal within {} s", START_LIMIT.as_secs())),
            ReadyRead::Failed(error) => Some(format!("reading the ready pipe failed: {error}")),
        };
        let said = output
            .as_ref()
            .map(|output| String::from_utf8_lossy(&output.stderr).trim().to_string())
            .unwrap_or_default();
        let reason = match (said.as_str(), pipe, &output) {
            ("", Some(pipe), _) => pipe,
            ("", None, Ok(output)) => format!("it exited ({})", output.status),
            ("", None, Err(error)) => error.to_string(),
            (said, None, _) => said.to_string(),
            (said, Some(pipe), _) => format!("{said}; {pipe}"),
        };
        Err(format!(
            "the core didn't start ({} serve): {reason}",
            program.to_string_lossy()
        ))
    }

    /// Replaces the core behind `probe` with this binary's when it is newer
    /// (see the module docs), and returns a probe of the core to relay to.
    fn offer_upgrade(&self, probe: Probe) -> Result<Probe, String> {
        let (Some(welcome), Ok(build)) = (&probe.welcome, &self.build) else {
            return Ok(probe);
        };
        let own = Build {
            version: env!("CARGO_PKG_VERSION").into(),
            build: build.clone(),
            mtime: modified("/proc/self/exe"),
        };
        let core = Build {
            version: welcome.core_version.clone(),
            build: welcome.core_build.clone(),
            mtime: modified(format!("/proc/{}/exe", welcome.pid)),
        };
        if !upgrade::is_newer(&own, &core) {
            return Ok(probe);
        }
        self.log.line(format_args!(
            "upgrading the core (pid {}) from {} ({}) to {} ({})",
            welcome.pid, core.version, core.build, own.version, own.build
        ));
        // A refusal is in the log already.
        if check::refusal("relay", &welcome.holder_protocols).is_some() {
            return Ok(probe);
        }
        let old = welcome.pid;
        let Probe {
            mut stream,
            welcome,
        } = probe;
        if let Err(error) = step_down(&mut stream) {
            self.log
                .line(format_args!("keeping the running core: {error}"));
            return Ok(Probe { stream, welcome });
        }
        drop(stream);
        if !wait_gone(old, EXIT_LIMIT) {
            self.log.line(format_args!(
                "the core (pid {old}) still runs {} s after stepping down",
                EXIT_LIMIT.as_secs()
            ));
        }
        self.probe(true)
    }
}

fn no_listener(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    )
}

fn modified(path: impl AsRef<Path>) -> Option<std::time::SystemTime> {
    fs::metadata(path)
        .and_then(|metadata| metadata.modified())
        .ok()
}

/// Says `HELLO` with a token that owns nothing, and reads the answer.
#[allow(
    clippy::wildcard_enum_match_arm,
    reason = "railway R3 exception: any answer to HELLO but `Welcome` or `Error` is reported as a failure"
)]
fn hello(mut stream: UnixStream) -> Result<Probe, String> {
    let talk = |error: io::Error| format!("can't talk to the core: {error}");
    stream.set_read_timeout(Some(HELLO_LIMIT)).map_err(talk)?;
    let hello = ToCore::Hello(Hello {
        min: client::PROTOCOLS.min,
        max: client::PROTOCOLS.max,
        client_version: format!("brindle-remote relay {}", env!("CARGO_PKG_VERSION")),
        client_token: format!("relay-{}-upgrade", std::process::id()),
        ..Default::default()
    });
    hello
        .encode()
        .and_then(|frame| frame.write_to(&mut stream))
        .map_err(talk)?;
    let frame = match Frame::read_from_or_eof(&mut stream) {
        Ok(Some(frame)) => frame,
        Ok(None) => return Err("the core closed the connection before answering".into()),
        Err(error) if timed_out(&error) => {
            return Err(format!(
                "the core didn't answer within {} s",
                HELLO_LIMIT.as_secs()
            ));
        }
        Err(error) => return Err(talk(error)),
    };
    stream.set_read_timeout(None).map_err(talk)?;
    match ToClient::decode(&frame).map_err(talk)? {
        ToClient::Welcome(welcome) => Ok(Probe {
            stream,
            welcome: Some(welcome),
        }),
        ToClient::Error(_) => Ok(Probe {
            stream,
            welcome: None,
        }),
        other => Err(format!("the core answered HELLO with {other:?}")),
    }
}

fn timed_out(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
    )
}

/// Sends `STEP_DOWN` and waits for `OK`, then for the core to close.
fn step_down(stream: &mut UnixStream) -> Result<(), String> {
    let req = 1;
    let talk = |error: io::Error| format!("can't talk to the core: {error}");
    stream
        .set_read_timeout(Some(STEP_DOWN_LIMIT))
        .map_err(talk)?;
    ToCore::StepDown(Request { req })
        .encode()
        .and_then(|frame| frame.write_to(stream))
        .map_err(talk)?;
    loop {
        match Frame::read_from_or_eof(stream) {
            Ok(Some(frame)) => match ToClient::decode(&frame) {
                Ok(ToClient::Ok(ok)) if ok.req == req => break,
                Ok(ToClient::Error(error)) if error.req == Some(req) => {
                    return Err(format!("STEP_DOWN refused: {}", error.message));
                }
                _ => {}
            },
            Ok(None) => return Ok(()),
            Err(error) if timed_out(&error) => {
                return Err(format!(
                    "no answer to STEP_DOWN within {} s",
                    STEP_DOWN_LIMIT.as_secs()
                ));
            }
            Err(error) => return Err(talk(error)),
        }
    }
    // The core closes its clients as it exits.
    let _ = stream.read_to_end(&mut Vec::new());
    Ok(())
}

/// Waits up to `limit` for `pid` to be gone (or a zombie, which a container
/// whose init doesn't reap leaves behind); whether it is.
fn wait_gone(pid: u32, limit: Duration) -> bool {
    let deadline = Instant::now() + limit;
    loop {
        if !runs(pid) {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// Whether `pid` runs: its `/proc` entry exists and isn't a zombie's.
fn runs(pid: u32) -> bool {
    fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|stat| stat_runs(&stat))
}

/// Whether a `/proc/<pid>/stat` line is a running process's: its state,
/// the first field after the command's last `)`, is there and isn't a
/// zombie's (`Z`) or a dead one's (`X`).
fn stat_runs(stat: &str) -> bool {
    let state = stat
        .rfind(')')
        .and_then(|end| stat[end + 1..].trim_start().chars().next());
    !matches!(state, None | Some('Z' | 'X'))
}

/// Where a relay starts a core, and the line for `core.log` when that is
/// without a scope.
#[derive(Debug, PartialEq, Eq)]
struct ScopeChoice {
    decision: scope::Decision,
    /// `Some` exactly when `decision` is [`scope::Decision::Plain`].
    note: Option<String>,
}

/// Where a relay starts a core: as `current` (this process's environment)
/// decides, except that a scope needs the user to linger, since otherwise the
/// user manager stops after the last login and takes its scopes with it.
///
/// The note says the core starts without a scope and why. It adds that the
/// sessions may not outlive the login only when a user manager is reachable,
/// the one case where a scope was possible (the user doesn't linger, or
/// `systemd-run` is missing). With no user manager (a container, a host
/// without systemd) or `BRINDLE_NO_SCOPE=1`, nothing could have been done
/// about it, so the note stays plain.
fn decide_scope(current: scope::Outcome, lingers: bool) -> ScopeChoice {
    let scope::Outcome {
        decision,
        user_manager,
    } = current;
    let decision = match decision {
        scope::Decision::Scope(_) if !lingers => scope::Decision::Plain("the user doesn't linger"),
        scope::Decision::Scope(_) | scope::Decision::Plain(_) => decision,
    };
    let note = match &decision {
        scope::Decision::Scope(_) => None,
        scope::Decision::Plain(reason) if user_manager => Some(format!(
            "starting the core without a scope of its own ({reason}); its sessions may not \
             outlive the login if the host sets KillUserProcesses=yes (`loginctl enable-linger` \
             keeps them)"
        )),
        scope::Decision::Plain(reason) => Some(format!(
            "starting the core without a scope of its own ({reason})"
        )),
    };
    ScopeChoice { decision, note }
}

/// Whether `user` lingers: `/var/lib/systemd/linger/<user>` exists.
fn lingers(user: Option<&OsStr>, exists: impl Fn(&Path) -> bool) -> bool {
    user.filter(|user| {
        let name = user.as_encoded_bytes();
        !name.is_empty() && !name.contains(&b'/') && name != b"." && name != b".."
    })
    .is_some_and(|user| exists(&Path::new(LINGER_DIR).join(user)))
}

/// Why the pipe ended.
enum End {
    /// The client closed the relay's stdin, or its stdout.
    ClientClosed,
    /// The core closed the connection.
    CoreClosed,
    Failed(String),
}

/// Copies frames both ways until either side closes or the client has been
/// silent for the idle limit (see the module docs).
fn pipe(reached: Reached, output: Arc<Output>, timings: Timings) -> Result<(), String> {
    let liveness = Arc::new(Liveness::new(Instant::now()));
    let (ended, ends) = mpsc::channel();
    let input = io::stdin()
        .as_fd()
        .try_clone_to_owned()
        .map(File::from)
        .map_err(|error| format!("relay: no stdin: {error}"))?;
    let to_core = reached
        .core
        .try_clone()
        .map_err(|error| format!("relay: {error}"))?;
    {
        let (liveness, ended) = (liveness.clone(), ended.clone());
        thread::spawn(move || forward_input(input, to_core, &liveness, &ended));
    }
    {
        let (output, ended) = (output.clone(), ended.clone());
        let core = reached.core;
        thread::spawn(move || forward_output(core, &output, &ended));
    }
    thread::spawn(move || heartbeats(&output, timings.heartbeat, &ended));
    let tick = (timings.heartbeat.min(timings.idle) / 4)
        .clamp(Duration::from_millis(10), Duration::from_secs(1));
    loop {
        match ends.recv_timeout(tick) {
            Ok(End::ClientClosed | End::CoreClosed) | Err(RecvTimeoutError::Disconnected) => {
                return Ok(());
            }
            Ok(End::Failed(message)) => return Err(message),
            Err(RecvTimeoutError::Timeout) => {
                if liveness.expired(Instant::now(), timings.idle) {
                    let message = format!(
                        "relay: nothing from the client for {} ms; ending the relay",
                        timings.idle.as_millis()
                    );
                    reached.log.line(&message);
                    return Err(message);
                }
            }
        }
    }
}

/// The stdin reader: frames from the client to the core, stamping each.
fn forward_input(input: File, mut core: UnixStream, liveness: &Liveness, ended: &Sender<End>) {
    let mut input = BufReader::with_capacity(64 * 1024, input);
    let end = loop {
        match Frame::read_from_or_eof(&mut input) {
            Ok(Some(frame)) => {
                liveness.stamp(Instant::now());
                // The link's own, proof of life only: never the core's.
                if relay::is_relay_type(frame.msg_type) {
                    continue;
                }
                liveness.set_busy(true);
                let sent = frame.write_to(&mut core);
                liveness.finish_handover(Instant::now());
                if sent.is_err() {
                    break End::CoreClosed;
                }
            }
            Ok(None) => break End::ClientClosed,
            Err(error) => break End::Failed(format!("relay: bad input from the client: {error}")),
        }
    };
    let _ = ended.send(end);
}

/// The core→stdout copier: whole frames, under the writer lock.
fn forward_output(core: UnixStream, output: &Output, ended: &Sender<End>) {
    let mut core = BufReader::with_capacity(64 * 1024, core);
    let end = loop {
        match Frame::read_from_or_eof(&mut core) {
            Ok(Some(frame)) => {
                if output.frame(&frame).is_err() {
                    break End::ClientClosed;
                }
            }
            Ok(None) => break End::CoreClosed,
            Err(error) => break End::Failed(format!("relay: bad output from the core: {error}")),
        }
    };
    let _ = ended.send(end);
}

/// Sends a heartbeat whenever the client has been written nothing for
/// `every`. It may wait for the lock, or block writing, as long as the
/// client takes nothing: it measures nothing.
fn heartbeats(output: &Output, every: Duration, ended: &Sender<End>) {
    let Ok(heartbeat) = RelayMessage::Heartbeat.encode() else {
        return;
    };
    loop {
        let due = output.lock().last_write + every;
        let now = Instant::now();
        if due > now {
            thread::sleep(due - now);
            continue;
        }
        let mut writer = output.lock();
        if writer.last_write.elapsed() >= every && writer.frame(&heartbeat).is_err() {
            let _ = ended.send(End::ClientClosed);
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_relay_takes_only_the_login_environment_argument() {
        let args = |items: &[&str]| items.iter().map(|item| item.to_string()).collect::<Vec<_>>();
        assert_eq!(login_asked(&args(&[])), Ok(false));
        assert_eq!(login_asked(&args(&["--login-environment"])), Ok(true));
        for bad in [
            &["--login-environment", "--login-environment"][..],
            &["--login"],
            &["x"],
            &["--login-environment", "x"],
        ] {
            let error = login_asked(&args(bad)).unwrap_err();
            assert_eq!(error, "usage: brindle-remote relay [--login-environment]");
        }
    }

    /// A capture's outcome is what the host frame says; a failure's log
    /// line names its reason and carries no value.
    #[test]
    fn a_failed_capture_is_reported_and_logged_by_its_reason() {
        let env = BTreeMap::from([("SECRET".to_string(), "hunter2".to_string())]);
        assert_eq!(
            login_outcome(Ok(env.clone())),
            (LoginEnv::Captured(env), None)
        );
        let (login, line) = login_outcome(Err(LoginEnvFailure::TimedOut {
            after: Duration::from_secs(10),
        }));
        assert_eq!(login, LoginEnv::Failed("it took longer than 10 s".into()));
        assert_eq!(
            line.as_deref(),
            Some("the login shell's environment wasn't read: it took longer than 10 s")
        );
    }

    #[test]
    fn an_unreadable_build_id_is_sent_as_unknown() {
        let unreadable = Err("can't read /proc/self/exe: permission denied".to_string());
        assert_eq!(host_build(&unreadable), "");
        assert_eq!(host_build(&Ok("4f2a91".to_string())), "4f2a91");
    }

    /// A `SHELL` that says something wins; else the passwd database's
    /// shell, as under `docker exec`, which sets none; else none.
    #[test]
    fn the_login_shell_is_shell_then_the_passwd_database() {
        let some = |shell: &str| Some(shell.to_string());
        assert_eq!(login_shell(some("/bin/zsh"), some("/bin/bash")), some("/bin/zsh"));
        assert_eq!(login_shell(some("/bin/zsh"), None), some("/bin/zsh"));
        assert_eq!(login_shell(some(""), some("/bin/bash")), some("/bin/bash"));
        assert_eq!(login_shell(None, some("/bin/bash")), some("/bin/bash"));
        assert_eq!(login_shell(some(""), None), None);
        assert_eq!(login_shell(None, None), None);
        // An empty passwd shell is what the database says.
        assert_eq!(login_shell(None, some("")), some(""));
    }

    /// A process runs while its stat line has a state after the command's
    /// last `)` that isn't a zombie's or a dead one's.
    #[test]
    fn a_stat_line_runs_unless_its_state_is_missing_zombie_or_dead() {
        assert!(stat_runs("4242 (serve) R 1 4242 4242 0 -1"));
        assert!(stat_runs("4242 (serve) S 1 4242 4242 0 -1\n"));
        assert!(stat_runs("4242 (serve) D 1"));
        assert!(!stat_runs("4242 (serve) Z 1 4242 4242 0 -1"));
        assert!(!stat_runs("4242 (serve) X 1"));
        // The command may hold `)` and spaces: the last `)` ends it.
        assert!(stat_runs("4242 (a) Z (b) S 1"));
        assert!(!stat_runs("4242 (a) S (b) Z 1"));
        // No state to read.
        assert!(!stat_runs("4242 serve S 1"));
        assert!(!stat_runs("4242 (serve)"));
        assert!(!stat_runs("4242 (serve)   \n"));
        assert!(!stat_runs(""));
    }

    /// A socket that isn't there, or that nothing listens on, is no core.
    #[test]
    fn no_listener_is_a_missing_or_refusing_socket() {
        for kind in [io::ErrorKind::NotFound, io::ErrorKind::ConnectionRefused] {
            assert!(no_listener(&io::Error::from(kind)), "{kind:?}");
        }
        for kind in [
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::TimedOut,
            io::ErrorKind::ConnectionReset,
            io::ErrorKind::Other,
        ] {
            assert!(!no_listener(&io::Error::from(kind)), "{kind:?}");
        }
    }

    /// A read timeout surfaces as `WouldBlock` or `TimedOut`, whichever
    /// the platform gives.
    #[test]
    fn timed_out_is_either_kind_a_read_timeout_gives() {
        for kind in [io::ErrorKind::WouldBlock, io::ErrorKind::TimedOut] {
            assert!(timed_out(&io::Error::from(kind)), "{kind:?}");
        }
        for kind in [
            io::ErrorKind::Interrupted,
            io::ErrorKind::UnexpectedEof,
            io::ErrorKind::ConnectionRefused,
            io::ErrorKind::NotFound,
        ] {
            assert!(!timed_out(&io::Error::from(kind)), "{kind:?}");
        }
    }

    fn outcome(decision: scope::Decision, user_manager: bool) -> scope::Outcome {
        scope::Outcome {
            decision,
            user_manager,
        }
    }

    #[test]
    fn the_login_warning_is_logged_only_where_a_user_manager_could_have_given_a_scope() {
        let scoped = || scope::Decision::Scope(PathBuf::from("/usr/bin/systemd-run"));
        const WARNING: &str = "may not outlive the login";

        // A scope for a lingering user: nothing to say.
        let choice = decide_scope(outcome(scoped(), true), true);
        assert_eq!(choice, ScopeChoice { decision: scoped(), note: None });

        // A user manager but no linger: plain, and the warning.
        let choice = decide_scope(outcome(scoped(), true), false);
        assert_eq!(choice.decision, scope::Decision::Plain("the user doesn't linger"));
        let note = choice.note.expect("a note");
        assert!(note.contains("without a scope of its own (the user doesn't linger)"), "{note}");
        assert!(note.contains(WARNING) && note.contains("enable-linger"), "{note}");

        // A user manager but no systemd-run: plain, and the warning, whatever
        // the linger file says.
        for lingers in [true, false] {
            let missing = scope::Decision::Plain("systemd-run isn't on PATH");
            let choice = decide_scope(outcome(missing.clone(), true), lingers);
            assert_eq!(choice.decision, missing);
            let note = choice.note.expect("a note");
            assert!(note.contains("systemd-run isn't on PATH") && note.contains(WARNING), "{note}");
        }

        // No user manager (a container): plain with its reason, no warning,
        // whatever the linger file says.
        for lingers in [true, false] {
            for reason in ["no systemd user manager", "XDG_RUNTIME_DIR is unset"] {
                let plain = scope::Decision::Plain(reason);
                let choice = decide_scope(outcome(plain.clone(), false), lingers);
                assert_eq!(choice.decision, plain);
                let note = choice.note.expect("a note");
                assert!(note.contains("without a scope of its own"), "{note}");
                assert!(note.contains(reason) && !note.contains(WARNING), "{note}");
            }
        }

        // BRINDLE_NO_SCOPE=1: the same, no warning.
        let off = scope::Decision::Plain("BRINDLE_NO_SCOPE=1");
        let choice = decide_scope(outcome(off.clone(), false), true);
        assert_eq!(choice.decision, off);
        let note = choice.note.expect("a note");
        assert!(note.contains("BRINDLE_NO_SCOPE=1") && !note.contains(WARNING), "{note}");
    }

    #[test]
    fn a_user_lingers_when_systemd_has_their_file() {
        let exists = |path: &Path| path == Path::new("/var/lib/systemd/linger/sam");
        assert!(lingers(Some(OsStr::new("sam")), exists));
        assert!(!lingers(Some(OsStr::new("kim")), exists));
        assert!(!lingers(None, exists));
        // Names that would point elsewhere never count.
        let anything = |_: &Path| true;
        for name in ["", ".", "..", "a/b", "../sam"] {
            assert!(!lingers(Some(OsStr::new(name)), anything), "{name:?}");
        }
        assert!(lingers(Some(OsStr::new("sam")), anything));
    }

    #[test]
    fn test_timings_replace_the_defaults_only_when_valid() {
        // Read from this process's environment, which tests share: only the
        // defaults are checked here; the relay tests set the variables.
        if std::env::var_os(HEARTBEAT_ENV).is_none() && std::env::var_os(IDLE_ENV).is_none() {
            let timings = Timings::from_env();
            assert_eq!(
                (timings.heartbeat, timings.idle),
                (HEARTBEAT_AFTER, IDLE_LIMIT)
            );
        }
        assert!(IDLE_LIMIT >= 3 * HEARTBEAT_AFTER);
    }

    #[test]
    fn a_zombie_or_a_missing_pid_doesnt_run() {
        assert!(runs(std::process::id()));
        assert!(!runs(u32::MAX));
        assert!(wait_gone(u32::MAX, Duration::ZERO));
        assert!(!wait_gone(std::process::id(), Duration::from_millis(30)));
    }
}
