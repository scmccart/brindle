//! The relay transport: a remote endpoint's terminal server, reached
//! through `brindle-remote relay` at the end of an `ssh` or container
//! `exec` process (design decision 2).
//!
//! - **[`Dial`]** starts a relay and reads up to its frame marker. The
//!   production one ([`dial_with`]) runs the endpoint's launch command, which is
//!   the cached helper's `relay` or an exit with 97; on that [`Unstarted`]
//!   the helper is deployed and the launch run again
//!   ([`client::launch`](crate::remote::client::launch)).
//! - **The process** ([`start`]): its stdin and stdout are the link, and its
//!   stderr is kept for errors. A thread waits for it with a blocking
//!   `wait()`, and ending the link kills it by its pid only until that
//!   `wait()` has returned.
//! - **[`Timings`]:** heartbeats, the dead-link limit, the bootstrap limit,
//!   the reconnect backoff, how long an unused connection is kept and how
//!   long an unwritten `CLOSE` keeps it trying, which tests shorten.
//! - **The program's environment** ([`relay_env`], [`SpawnRequest::remote`]):
//!   a remote `SPAWN` carries the relay's environment (`RELAY_HOST`, a fresh
//!   login's) with the profile's ([`child_env`]) over it.
//!
//! [`child_env`]: crate::terminal::child_env

use std::collections::BTreeMap;
use std::io::{self, Read};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Duration;

use anyhow::Result;
use parking_lot::{Condvar, Mutex};

use super::link::{Link, Wire};
use super::{SpawnRequest, sys};
use crate::config::{Profile, RemoteTarget};
use crate::remote::client::{self, HelperLookup};
pub use crate::remote::client::Unstarted;
use crate::remote::protocol::client::Policy;
use crate::remote::protocol::{Frame, HEARTBEAT_INTERVAL, skip_preamble};
use crate::terminal::GridSize;

/// How long a relay's process may take to write its frame marker: an SSH
/// connect (`ConnectTimeout=15`) and the login's startup files.
const MARKER_LIMIT: Duration = Duration::from_secs(60);
/// How much of a relay process's stderr is kept for its errors.
const STDERR_KEPT: usize = 16 * 1024;
/// How long to wait for a relay process that ended its stdout to exit, for
/// its status.
const EXIT_WAIT: Duration = Duration::from_secs(2);

/// The relay link's timings (design decision 2).
#[derive(Clone, Copy, Debug)]
pub struct Timings {
    /// A heartbeat goes out after this long without a write.
    pub heartbeat: Duration,
    /// A link that has read nothing for this long is dead.
    pub dead: Duration,
    /// How long a link may take from its frame marker to `WELCOME`. The
    /// relay sends nothing in between while it may step a core down
    /// (checkpointing up to 60 s, its exit 10 s, the next core 15 s), so
    /// this is longer than the dead-link limit.
    pub bootstrap: Duration,
    /// The first pause before reconnecting, doubled after each failure...
    pub first_retry: Duration,
    /// ...up to this.
    pub retry_cap: Duration,
    /// The cap for an eager connection with no terminals.
    pub eager_cap: Duration,
    /// How long a connection that isn't eager is kept once nothing uses it,
    /// so closing a tab and opening another reuses it.
    pub idle: Duration,
    /// How long, with no link up, a `CLOSE` not written yet keeps the
    /// connection in use after it was queued; then it is given up, and its
    /// program is left running (design decision 4).
    pub close_limit: Duration,
}

impl Default for Timings {
    fn default() -> Self {
        Self {
            heartbeat: HEARTBEAT_INTERVAL,
            dead: Duration::from_secs(30),
            bootstrap: Duration::from_secs(120),
            first_retry: Duration::from_secs(1),
            retry_cap: Duration::from_secs(30),
            eager_cap: Duration::from_secs(60),
            idle: Duration::from_secs(5),
            close_limit: Duration::from_secs(5 * 60),
        }
    }
}

/// The pause before the next attempt after `failures` failed ones in a row
/// (none after a link was lost): `first`, `first`, 2×`first`, 4×`first`…,
/// at most `cap`.
pub fn backoff(failures: u32, first: Duration, cap: Duration) -> Duration {
    let doublings = failures.saturating_sub(1).min(20);
    first.saturating_mul(1 << doublings).min(cap)
}

/// Starts relays at one endpoint.
pub trait Dial: Send + Sync {
    /// The endpoint, as messages name it ([`super::Host::name`]).
    fn name(&self) -> String;
    /// Starts a relay and reads up to its frame marker, so the link's
    /// reader starts at the relay's first frame. An error is what a new
    /// tab on the endpoint shows.
    fn launch(&self) -> Result<Link>;
}

/// The relay at one SSH host or in one container, started by the endpoint's
/// launch command with the helper deployed when the endpoint has none
/// ([`client::launch`]).
struct Deployed {
    name: String,
    target: RemoteTarget,
    /// The helper deployed, looked up at each launch.
    helper: HelperLookup,
}

impl Dial for Deployed {
    fn name(&self) -> String {
        self.name.clone()
    }

    fn launch(&self) -> Result<Link> {
        client::launch(&self.target, &self.helper, |command| start(command, MARKER_LIMIT))
    }
}

/// The relay at `target`: this build's `brindle-remote relay` from the
/// endpoint's cache, deployed there first when it is missing. The helper
/// deployed is the one `helper` finds: production's is
/// [`client::default_helper`], and tests pass their own.
pub fn dial_with(name: String, target: &RemoteTarget, helper: HelperLookup) -> Arc<dyn Dial> {
    Arc::new(Deployed { name, target: target.clone(), helper })
}

/// Runs `command` as a relay's process and reads its stdout up to the frame
/// marker, for at most `limit` (then the process is killed). Its stdin and
/// stdout are the link. A process whose stdout ends before the marker is
/// [`Unstarted`], with the code it exited with.
pub fn start(mut command: Command, limit: Duration) -> Result<Link, Unstarted> {
    let program = command.get_program().to_string_lossy().into_owned();
    let unstarted = |message: String| Unstarted { code: None, said: String::new(), message };
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| unstarted(format!("can't run {program}: {error}")))?;
    let (Some(stdin), Some(mut stdout), Some(stderr)) = (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        child.kill().ok();
        child.wait().ok();
        return Err(unstarted(format!("{program} has no stdio")));
    };
    let process = Process::watch(child);
    let kept = keep_stderr(stderr);

    // Killed if the marker doesn't come in time, which ends the read below.
    let (marked, marking) = mpsc::channel::<()>();
    let late = Arc::new(AtomicBool::new(false));
    {
        let (process, late) = (process.clone(), late.clone());
        std::thread::Builder::new()
            .name("relay-marker".into())
            .spawn(move || {
                if let Err(mpsc::RecvTimeoutError::Timeout) = marking.recv_timeout(limit) {
                    late.store(true, Ordering::Release);
                    process.kill();
                }
            })
            .map_err(|error| unstarted(format!("starting a thread: {error}")))?;
    }
    let preamble = skip_preamble(&mut stdout);
    drop(marked);
    if let Err(error) = preamble {
        let late = late.load(Ordering::Acquire);
        // A process whose output ended is on its way out, and its exit code
        // says why (a launch script's 97), so it gets a moment to exit by
        // itself. Too much noise leaves it running, and a late one has been
        // killed already.
        let ending = error.kind() == io::ErrorKind::UnexpectedEof && !late;
        let status = ending.then(|| process.wait_for(EXIT_WAIT)).flatten().or_else(|| {
            process.kill();
            process.wait_for(EXIT_WAIT)
        });
        let problem = if late { format!("no frame marker within {} s", limit.as_secs()) } else { error.to_string() };
        let said = kept.text(EXIT_WAIT);
        let exit = status.map(|status| format!("{program} {}", exit_text(status)));
        let message = unstarted_message(&problem, exit.as_deref(), &said);
        return Err(Unstarted { code: status.and_then(|status| status.code()), said, message });
    }
    let wire = Arc::new(ChildWire { stdin: Mutex::new(Some(stdin)), process, stderr: kept, program });
    Ok(Link { reader: Box::new(stdout), wire })
}

/// What a tab shows for a relay that didn't start. When the process wrote to
/// stderr, the first line names its cause (the last line it wrote,
/// [`client::split_cause`]) and how the process ended; the other stderr lines
/// and then the stream's `problem` follow. With nothing on stderr the problem
/// is the first line.
fn unstarted_message(problem: &str, exit: Option<&str>, said: &str) -> String {
    let ending = exit.map(|exit| format!(" ({exit})")).unwrap_or_default();
    let Some((cause, rest)) = client::split_cause(said) else {
        return format!("the relay didn't start: {problem}{ending}");
    };
    let mut message = format!("the relay didn't start: {cause}{ending}");
    if !rest.is_empty() {
        message.push('\n');
        message.push_str(&rest);
    }
    message.push('\n');
    message.push_str(problem);
    message
}

fn exit_text(status: ExitStatus) -> String {
    match status.code() {
        Some(code) => format!("exited with {code}"),
        None => format!("ended: {status}"),
    }
}

/// A relay's process, waited for on a thread of its own.
struct Process {
    pid: u32,
    state: Mutex<Exit>,
    exited: Condvar,
}

#[derive(Default)]
struct Exit {
    /// Set once `wait()` returned, before the `Child` is dropped. On
    /// Windows its open handle keeps the pid its own until then. On Linux
    /// `wait()` reaps it before returning, so a kill between the two could
    /// reach a process that took the pid in that instant; nothing else
    /// reaps it, so the window is that short.
    status: Option<ExitStatus>,
}

impl Process {
    fn watch(mut child: Child) -> Arc<Self> {
        let process = Arc::new(Self { pid: child.id(), state: Mutex::new(Exit::default()), exited: Condvar::new() });
        let watching = process.clone();
        let started = std::thread::Builder::new().name("relay-wait".into()).spawn(move || {
            let status = child.wait();
            {
                let mut state = watching.state.lock();
                // A failed wait leaves nothing to kill either.
                state.status = Some(status.unwrap_or_else(|_| failed_status()));
            }
            watching.exited.notify_all();
            drop(child);
        });
        if started.is_err() {
            // Nothing will reap it: end it now and count it as exited.
            sys::kill(process.pid);
            process.state.lock().status = Some(failed_status());
        }
        process
    }

    /// Ends the process, unless it has already exited.
    fn kill(&self) {
        let state = self.state.lock();
        if state.status.is_none() {
            sys::kill(self.pid);
        }
    }

    /// Its exit status, waiting up to `limit` for it.
    fn wait_for(&self, limit: Duration) -> Option<ExitStatus> {
        let mut state = self.state.lock();
        if state.status.is_none() {
            self.exited.wait_for(&mut state, limit);
        }
        state.status
    }
}

/// An exit status for a process whose own couldn't be read.
fn failed_status() -> ExitStatus {
    #[cfg(unix)]
    {
        std::os::unix::process::ExitStatusExt::from_raw(1 << 8)
    }
    #[cfg(windows)]
    {
        std::os::windows::process::ExitStatusExt::from_raw(1)
    }
}

/// A relay process's stderr: its first [`STDERR_KEPT`] bytes are kept, and
/// the rest is read and dropped so the process never blocks on it.
#[derive(Clone, Default)]
struct Stderr(Arc<(Mutex<Said>, Condvar)>);

#[derive(Default)]
struct Said {
    bytes: Vec<u8>,
    /// The pipe ended.
    done: bool,
}

impl Stderr {
    /// What it said, once the pipe ended or `limit` passed: a process that
    /// left a child behind (an ssh `ControlPersist` master) may hold the
    /// pipe open.
    fn text(&self, limit: Duration) -> String {
        let (said, ended) = &*self.0;
        let mut said = said.lock();
        if !said.done {
            ended.wait_for(&mut said, limit);
        }
        String::from_utf8_lossy(&said.bytes).trim().to_string()
    }
}

fn keep_stderr(mut stderr: impl Read + Send + 'static) -> Stderr {
    let kept = Stderr::default();
    let keeping = kept.clone();
    let started = std::thread::Builder::new().name("relay-stderr".into()).spawn(move || {
        let (said, ended) = &*keeping.0;
        let mut buffer = [0u8; 4096];
        loop {
            match stderr.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    let mut said = said.lock();
                    let room = STDERR_KEPT.saturating_sub(said.bytes.len());
                    said.bytes.extend_from_slice(&buffer[..read.min(room)]);
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
        said.lock().done = true;
        ended.notify_all();
    });
    if started.is_err() {
        kept.0.0.lock().done = true;
    }
    kept
}

/// The writing half of a relay process's link.
struct ChildWire {
    stdin: Mutex<Option<ChildStdin>>,
    process: Arc<Process>,
    stderr: Stderr,
    program: String,
}

impl Wire for ChildWire {
    fn send(&self, frame: &Frame) -> io::Result<()> {
        match &mut *self.stdin.lock() {
            Some(stdin) => frame.write_to(stdin),
            None => Err(io::ErrorKind::BrokenPipe.into()),
        }
    }

    /// Kills the process, which ends its stdout and any write blocked on
    /// its stdin. The stdin lock isn't taken: such a write holds it.
    fn end(&self) {
        self.process.kill();
    }

    fn why(&self) -> String {
        let status = self.process.wait_for(EXIT_WAIT);
        let said = self.stderr.text(EXIT_WAIT);
        let exit = status.map(|status| format!("{} {}", self.program, exit_text(status)));
        match client::split_cause(&said) {
            // A process that exited 0 and wrote nothing has nothing to add
            // to a stream that ended.
            None if status.is_some_and(|status| status.success()) => String::new(),
            None => exit.unwrap_or_default(),
            // The cause first, as in a launch failure's message.
            Some((cause, rest)) => {
                let mut why = match exit {
                    Some(exit) => format!("{exit}: {cause}"),
                    None => cause.to_string(),
                };
                if !rest.is_empty() {
                    why.push('\n');
                    why.push_str(&rest);
                }
                why
            }
        }
    }
}

impl Drop for ChildWire {
    /// A link that is let go of ends its process.
    fn drop(&mut self) {
        self.process.kill();
    }
}

/// A remote session's environment: `host`'s (`RELAY_HOST`'s: a fresh
/// login's on the endpoint) with `own` over it. Names are compared as
/// given, since endpoints are Linux whatever this machine is.
pub fn relay_env(host: &BTreeMap<String, String>, own: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    let mut env = host.clone();
    env.extend(own.iter().map(|(name, value)| (name.clone(), value.clone())));
    env
}

impl SpawnRequest {
    /// What a remote profile's session runs, for a relay connection: the
    /// profile's command, or none (the endpoint's login shell); its
    /// directory as given, or none (the holder expands `~` and falls back to
    /// home for a directory that isn't there); and [`child_env`] as the
    /// environment, which the connection puts over the endpoint's.
    ///
    /// [`child_env`]: crate::terminal::child_env
    pub fn remote(
        profile: &Profile,
        command: Option<(String, Vec<String>)>,
        cwd: Option<PathBuf>,
        size: GridSize,
        scrollback: u32,
        policy: Policy,
    ) -> Self {
        let (command, args) = command.unwrap_or_else(|| (profile.command.clone().unwrap_or_default(), profile.args.clone()));
        let cwd = cwd
            .map(|dir| dir.to_string_lossy().into_owned())
            .or_else(|| profile.cwd.clone())
            .filter(|dir| !dir.is_empty());
        Self {
            command,
            args,
            cwd,
            env: crate::terminal::child_env(profile).into_iter().collect(),
            cols: size.cols,
            rows: size.rows,
            cell_w: size.cell_width,
            cell_h: size.cell_height,
            scrollback,
            policy,
            profile: Some(profile.name.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    #[test]
    fn the_backoff_doubles_from_the_first_pause_to_its_cap() {
        let second = Duration::from_secs(1);
        let cap = Duration::from_secs(30);
        let pauses: Vec<u64> = (0..9).map(|failures| backoff(failures, second, cap).as_secs()).collect();
        assert_eq!(pauses, [1, 1, 2, 4, 8, 16, 30, 30, 30]);
        assert_eq!(backoff(u32::MAX, second, cap), cap);
        assert_eq!(backoff(9, second, Duration::from_secs(60)).as_secs(), 60);
    }

    #[test]
    fn the_endpoints_environment_is_under_the_profiles() {
        let host = BTreeMap::from([
            ("LANG".to_string(), "en_GB.UTF-8".to_string()),
            ("HOME".to_string(), "/home/ubuntu".to_string()),
            ("TERM".to_string(), "dumb".to_string()),
            ("path".to_string(), "/lower".to_string()),
        ]);
        let profile = Profile {
            name: "A8".into(),
            env: BTreeMap::from([("FOO".to_string(), "1".to_string()), ("PATH".to_string(), "/usr/bin".to_string())]),
            ..Default::default()
        };
        let own: BTreeMap<String, String> = crate::terminal::child_env(&profile).into_iter().collect();
        let env = relay_env(&host, &own);
        assert_eq!(env["LANG"], "en_GB.UTF-8");
        assert_eq!(env["HOME"], "/home/ubuntu");
        assert_eq!(env["FOO"], "1");
        // Brindle's terminal variables win over a tty-less login's.
        assert_eq!(env["TERM"], "xterm-256color");
        assert_eq!(env["COLORTERM"], "truecolor");
        assert_eq!(env["TERM_PROGRAM"], "Brindle");
        // Linux names: `PATH` doesn't replace `path`, even on Windows.
        assert_eq!((env["PATH"].as_str(), env["path"].as_str()), ("/usr/bin", "/lower"));
        // Nothing of Brindle's own environment.
        let keys: Vec<&str> = env.keys().map(String::as_str).collect();
        assert_eq!(keys, ["COLORTERM", "FOO", "HOME", "LANG", "PATH", "TERM", "TERM_PROGRAM", "TERM_PROGRAM_VERSION", "path"]);
    }

    fn profile() -> Profile {
        Profile {
            name: "A8".into(),
            remote: Some(RemoteTarget::Ssh {
                host: "A8".into(),
                user: None,
                port: None,
                identity_file: None,
                ssh_args: Vec::new(),
                connect_at_launch: false,
            }),
            env: BTreeMap::from([("FOO".to_string(), "1".to_string())]),
            ..Default::default()
        }
    }

    #[test]
    fn a_remote_request_leaves_the_shell_and_directory_to_the_endpoint() {
        let size = GridSize::default();
        let request = SpawnRequest::remote(&profile(), None, None, size, 1000, Policy::Detach);
        // No command: the holder runs the login shell.
        assert_eq!(request.command, "");
        assert!(request.args.is_empty());
        assert_eq!(request.cwd, None);
        assert_eq!(request.env, crate::terminal::child_env(&profile()).into_iter().collect());
        assert_eq!((request.cols, request.rows, request.cell_w, request.cell_h), (80, 24, 8, 16));
        assert_eq!((request.scrollback, request.policy), (1000, Policy::Detach));
        assert_eq!(request.profile.as_deref(), Some("A8"));

        // The profile's directory goes as written: `~` is the endpoint's.
        let mut with_dir = profile();
        with_dir.cwd = Some("~/src".into());
        with_dir.command = Some("htop".into());
        with_dir.args = vec!["-d".into(), "10".into()];
        let request = SpawnRequest::remote(&with_dir, None, None, size, 0, Policy::Detach);
        assert_eq!((request.command.as_str(), request.args.as_slice()), ("htop", &["-d".to_string(), "10".to_string()][..]));
        assert_eq!(request.cwd.as_deref(), Some("~/src"));

        // A split's directory wins, unchecked: it needn't exist here.
        let missing = PathBuf::from("/srv/old");
        let command = Some(("tail".to_string(), vec!["-f".to_string(), "syslog".to_string()]));
        let request = SpawnRequest::remote(&with_dir, command, Some(missing), size, 0, Policy::Kill);
        assert_eq!(request.cwd.as_deref(), Some("/srv/old"));
        assert_eq!(request.command, "tail");
        assert_eq!(request.policy, Policy::Kill);
    }

    /// A command that writes `text` to stderr and exits with `code`.
    fn failing(text: &str, code: i32) -> Command {
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;

            let mut command = Command::new("cmd");
            command.args(["/d", "/c"]).raw_arg(format!("echo {text} 1>&2 & exit {code}"));
            crate::remote::hide_window(&mut command);
            command
        }
        #[cfg(unix)]
        {
            let mut command = Command::new("sh");
            command.args(["-c", &format!("echo {text} >&2; exit {code}")]);
            command
        }
    }

    /// A command that writes `noise`, the frame marker and `after` to its
    /// stdout, then waits a minute, as one process: killing it ends the
    /// pipe.
    fn marking(noise: &str, after: &str) -> Command {
        #[cfg(windows)]
        {
            let script = format!(
                "$o = [Console]::OpenStandardOutput(); \
                 $b = [Text.Encoding]::ASCII.GetBytes(\"{noise}`0BRINDLE-FRAMES1`n{after}\"); \
                 $o.Write($b, 0, $b.Length); $o.Flush(); Start-Sleep 60"
            );
            let mut command = Command::new("powershell");
            command.args(["-NoProfile", "-NonInteractive", "-Command", &script]);
            crate::remote::hide_window(&mut command);
            command
        }
        #[cfg(unix)]
        {
            let mut command = Command::new("sh");
            command.args(["-c", &format!("printf '{noise}\\000BRINDLE-FRAMES1\\n{after}'; exec sleep 60")]);
            command
        }
    }

    /// A command that writes nothing and waits a minute.
    fn silent() -> Command {
        #[cfg(windows)]
        {
            let mut command = Command::new("powershell");
            command.args(["-NoProfile", "-NonInteractive", "-Command", "Start-Sleep 60"]);
            crate::remote::hide_window(&mut command);
            command
        }
        #[cfg(unix)]
        {
            let mut command = Command::new("sleep");
            command.arg("60");
            command
        }
    }

    #[test]
    fn a_relay_that_exits_before_its_marker_says_why_with_its_code() {
        let error = start(failing("helper not cached", 97), Duration::from_secs(20)).err().expect("an error");
        assert_eq!(error.code, Some(97));
        assert!(error.message.starts_with("the relay didn't start"), "{}", error.message);
        assert!(error.message.contains("exited with 97"), "{}", error.message);
        assert!(error.message.contains("helper not cached"), "{}", error.message);
        // As an anyhow error, 5.3's launch finds the code.
        let error = anyhow::Error::from(error);
        assert_eq!(error.downcast_ref::<Unstarted>().and_then(|e| e.code), Some(97));
        let missing = start(Command::new("brindle-no-such-program"), Duration::from_secs(5)).err().expect("an error");
        assert!(missing.message.starts_with("can't run brindle-no-such-program"), "{}", missing.message);
    }

    /// The first line of a tab's message names the cause (design decision
    /// 1); everything else the process said, and the stream's problem, stays
    /// below it.
    #[test]
    fn a_launch_failures_first_line_is_the_last_thing_the_process_said() {
        let problem = "the stream ended before the frame marker";
        let exit = Some("docker exited with 1");
        // docker: one line.
        let docker = "Error response from daemon: container abc is not running";
        assert_eq!(
            unstarted_message(problem, exit, docker),
            format!("the relay didn't start: {docker} (docker exited with 1)\n{problem}")
        );
        // ssh: its warning, then its error.
        let ssh = "Warning: Permanently added 'a8' (ED25519).\nssh: connect to host a8 port 22: Connection refused\n";
        assert_eq!(
            unstarted_message(problem, Some("ssh exited with 255"), ssh),
            format!(
                "the relay didn't start: ssh: connect to host a8 port 22: Connection refused \
                 (ssh exited with 255)\nWarning: Permanently added 'a8' (ED25519).\n{problem}"
            )
        );
        // Nothing on stderr: today's text.
        assert_eq!(
            unstarted_message(problem, exit, ""),
            format!("the relay didn't start: {problem} (docker exited with 1)")
        );
        assert_eq!(unstarted_message("no frame marker within 60 s", None, "  \n"), "the relay didn't start: no frame marker within 60 s");
    }

    /// A process's `why`: its exit, then the cause, then the other lines; and
    /// nothing for a process that exited cleanly with nothing to say.
    #[cfg(unix)]
    #[test]
    fn a_link_that_ends_says_how_its_process_ended() {
        // A process that writes its marker, runs `script` and exits `code`.
        let why = |script: &str, code: i32| {
            let mut command = Command::new("sh");
            command.args(["-c", &format!("printf '\\000BRINDLE-FRAMES1\\n'; {script}{}exit {code}", if script.is_empty() { "" } else { "; " })]);
            let mut link = start(command, Duration::from_secs(20)).unwrap_or_else(|e| panic!("{e}"));
            let mut rest = Vec::new();
            link.reader.read_to_end(&mut rest).ok();
            link.wire.why()
        };
        assert_eq!(why("", 0), "");
        assert_eq!(why("", 3), "sh exited with 3");
        assert_eq!(why("echo ssh: boom >&2", 255), "sh exited with 255: ssh: boom");
        assert_eq!(why("echo warning >&2; echo ssh: boom >&2", 255), "sh exited with 255: ssh: boom\nwarning");
    }

    /// A launch whose output ends before it has exited isn't killed for it:
    /// its code is what tells a missing helper (97) from a failure.
    #[test]
    fn a_relay_that_ends_its_output_before_exiting_still_says_its_code() {
        if !cfg!(unix) {
            // No shell here that can close its own stdout.
            return;
        }
        let mut command = Command::new("sh");
        command.args(["-c", "exec >&-; sleep 0.3; echo not cached >&2; exit 97"]);
        let error = start(command, Duration::from_secs(20)).err().expect("an error");
        assert_eq!(error.code, Some(97), "{}", error.message);
        assert_eq!(error.said, "not cached");
    }

    #[test]
    fn a_relay_that_never_marks_is_killed_at_the_limit() {
        let started = Instant::now();
        let error = start(silent(), Duration::from_millis(1500)).err().expect("an error");
        assert!(started.elapsed() >= Duration::from_millis(1500));
        assert!(started.elapsed() < Duration::from_secs(30), "{:?}", started.elapsed());
        assert!(error.message.contains("no frame marker within 1 s"), "{}", error.message);
        if cfg!(unix) {
            assert_eq!(error.code, None, "killed, not exited");
        }
    }

    #[test]
    fn ending_a_relay_link_kills_its_process_and_ends_the_read() {
        let mut link = start(marking("motd\\n", "F"), Duration::from_secs(30)).unwrap_or_else(|e| panic!("{e}"));
        // The reader starts right after the marker.
        let mut first = [0u8; 1];
        link.reader.read_exact(&mut first).unwrap();
        assert_eq!(&first, b"F");
        let (tx, read) = mpsc::channel();
        let mut reader = link.reader;
        std::thread::spawn(move || {
            let mut rest = Vec::new();
            tx.send(reader.read_to_end(&mut rest).map(|_| rest.len()).map_err(|e| e.kind())).ok();
        });
        // The silent relay keeps the read open until the end: what ends it
        // is the end (nothing read, or a broken pipe), not an error before.
        link.wire.end();
        let ended = read.recv_timeout(crate::test_support::wait::limit(Duration::from_secs(10))).expect("the read ended");
        assert!(matches!(ended, Ok(0) | Err(io::ErrorKind::BrokenPipe)), "{ended:?}");
        // It was killed: it says how, and writes fail.
        assert!(!link.wire.why().is_empty());
        let frame = Frame::new(1, b"x".to_vec()).unwrap();
        let failing = || link.wire.send(&frame).is_err();
        assert!(crate::test_support::wait::eventually(Duration::from_secs(10), failing), "writes still succeed");
        // Ending it again kills nothing.
        link.wire.end();
    }
}
