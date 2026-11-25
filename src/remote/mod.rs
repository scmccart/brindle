//! Remote Linux host and container support.
//!
//! A remote endpoint's sessions run in its own terminal server, which
//! Brindle reaches through a `brindle-remote relay` started over SSH or a
//! container's exec (`remote_command`): `client` deploys the helper and
//! launches the relay, and `local` keeps one connection per endpoint
//! ([`Host`]) through it.

// Only `local`, built on Linux and Windows, counts input against it.
#[cfg_attr(not(any(target_os = "linux", windows)), allow(dead_code))]
pub mod budget;
// Only `local`, built on Linux and Windows, launches relays.
#[cfg_attr(not(any(target_os = "linux", windows)), allow(dead_code))]
pub mod client;
mod host;
#[cfg_attr(not(any(target_os = "linux", windows)), allow(dead_code))]
pub mod hosts;
// Only `local`, built on Linux and Windows, reads keys and listings yet.
#[cfg_attr(not(any(target_os = "linux", windows)), allow(dead_code))]
mod identity;
#[cfg(any(target_os = "linux", windows))]
pub mod local;
pub use brindle_remote_protocol as protocol;
pub use host::Host;
pub use identity::{ListedSession, SessionKey};

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};

use crate::agent::sink::Sink as AgentSink;
use crate::agent::spawn::AgentSpawnRequest;
use crate::config::RemoteTarget;

/// What an agent zone's session starts from ([`agent_link`]).
pub enum AgentStart {
    /// A new agent session: what it runs, said on the session's own
    /// thread, as a terminal spawn's request is.
    Spawn(Box<dyn FnOnce() -> AgentSpawnRequest + Send>),
    /// A detached agent session, listed now (`recorded: false`: attached at
    /// once) or recorded by a restored zone (`recorded: true`: attached
    /// once the core's listing still has it).
    Attach { session: protocol::SessionId, key: Option<SessionKey>, recorded: bool },
}

/// An agent zone's session in a terminal server, as [`agent_link`] starts
/// it: on Linux and Windows the session in `remote::local`, which its
/// requests go through. Elsewhere none is ever made.
#[cfg(any(target_os = "linux", windows))]
pub struct AgentLink(local::Session);

/// Where there is no terminal server, no agent session is ever made.
#[cfg(not(any(target_os = "linux", windows)))]
pub enum AgentLink {}

#[cfg(any(target_os = "linux", windows))]
impl AgentLink {
    /// Sends a prompt while the session runs; the request's id, which its
    /// answer comes back under (`AgentEvent::Reply`), or `None`.
    pub fn prompt(&self, text: &str) -> Option<protocol::RequestId> {
        self.0.prompt(text)
    }

    /// Sends the held prompt again, as [`AgentLink::prompt`].
    pub fn resend(&self) -> Option<protocol::RequestId> {
        self.0.resend()
    }

    /// Discards the queued prompt, waiting or held, as [`AgentLink::prompt`].
    pub fn unqueue(&self) -> Option<protocol::RequestId> {
        self.0.unqueue()
    }

    /// Answers the permission request in entry `entry` (`None`: cancelled),
    /// as [`AgentLink::prompt`].
    pub fn answer(&self, entry: u64, option: Option<String>) -> Option<protocol::RequestId> {
        self.0.answer(entry, option)
    }

    /// Cancels the running turn; true when it was sent.
    pub fn cancel(&self) -> bool {
        self.0.cancel()
    }

    /// Ends the session (`CLOSE`): the core hangs the agent up.
    pub fn close(&self) {
        self.0.close();
    }

    /// Leaves the session running with no client (`DETACH`), with its
    /// tab's layout: a quit's.
    pub fn detach_with(&self, layout: Option<serde_json::Value>) {
        self.0.detach_with(layout);
    }

    /// Keeps the core's copy of the tab's layout current.
    pub fn set_layout(&self, layout: Option<serde_json::Value>) {
        self.0.set_layout(layout);
    }

    /// The session's id and key, once it exists (an attach's as asked).
    pub fn id_and_key(&self) -> Option<(protocol::SessionId, Option<SessionKey>)> {
        self.0.id_and_key()
    }
}

#[cfg(not(any(target_os = "linux", windows)))]
impl AgentLink {
    pub fn prompt(&self, _text: &str) -> Option<protocol::RequestId> {
        match *self {}
    }

    pub fn resend(&self) -> Option<protocol::RequestId> {
        match *self {}
    }

    pub fn unqueue(&self) -> Option<protocol::RequestId> {
        match *self {}
    }

    pub fn answer(&self, _entry: u64, _option: Option<String>) -> Option<protocol::RequestId> {
        match *self {}
    }

    pub fn cancel(&self) -> bool {
        match *self {}
    }

    pub fn close(&self) {
        match *self {}
    }

    pub fn detach_with(&self, _layout: Option<serde_json::Value>) {
        match *self {}
    }

    pub fn set_layout(&self, _layout: Option<serde_json::Value>) {
        match *self {}
    }

    pub fn id_and_key(&self) -> Option<(protocol::SessionId, Option<SessionKey>)> {
        match *self {}
    }
}

/// Why [`agent_link`] can't start an agent session here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unsupported {
    reason: &'static str,
}

impl std::fmt::Display for Unsupported {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.reason)
    }
}

impl std::error::Error for Unsupported {}

/// What [`agent_link`] answers where the terminal server can't run.
#[cfg_attr(any(target_os = "linux", windows), allow(dead_code))]
const NEEDS_THE_SERVER: Unsupported =
    Unsupported { reason: "agent sessions run in the terminal server, which needs Linux or Windows" };

/// Starts (or attaches) an agent zone's session on `host`'s terminal
/// server: the platform-neutral entry point `src/agent/` uses, so it names
/// no platform itself. `start` says what the session's thread asks the
/// core for, and `sink` gets what the session reports, on the connection's
/// reader thread. It returns at once: connecting, the protocol gate and
/// the spawn or attach happen on the session's own thread, and a failure
/// reaches the sink (`AgentEvent::Failed`).
#[cfg(any(target_os = "linux", windows))]
pub fn agent_link(host: Host, start: AgentStart, sink: AgentSink) -> Result<AgentLink, Unsupported> {
    let sink: Box<dyn local::SessionSink> = Box::new(sink);
    let session = match start {
        AgentStart::Spawn(request) => local::Session::start_agent(host, request, sink),
        AgentStart::Attach { session, key, recorded } => {
            // An agent session has no size: the listed 0×0 goes back, and
            // the core ignores it.
            let when = if recorded { local::Attaching::Recorded } else { local::Attaching::Now { cols: 0, rows: 0 } };
            local::Session::attach(host, local::AttachRequest { session, key, when }, sink)
        }
    };
    Ok(AgentLink(session))
}

/// [`agent_link`] where there is no terminal server: always refused.
#[cfg(not(any(target_os = "linux", windows)))]
pub fn agent_link(host: Host, start: AgentStart, sink: AgentSink) -> Result<AgentLink, Unsupported> {
    let _ = (host, start, sink);
    Err(NEEDS_THE_SERVER)
}

/// Keeps a process Brindle starts outside a tab from opening a console
/// window of its own: Brindle has no console to share, so a console program
/// would otherwise get one. Does nothing off Windows.
pub(crate) fn hide_window(command: &mut Command) -> &mut Command {
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(command, crate::win::CREATE_NO_WINDOW);
    command
}

/// The names the `brindle-remote` helper goes by beside Brindle: the Linux
/// archive's, then the one `scripts/install.sh` and cargo build.
const HELPER_NAMES: [&str; 2] = ["brindle-remote-linux-x86_64", "brindle-remote"];

/// Brindle's executable as it was started, then the file behind it when that
/// is elsewhere: helpers are looked for beside each in turn. WinGet starts
/// Brindle through a symlink in its `Links` directory, where `current_exe`
/// stays (the Start Menu shortcut wants that path), while the helpers it
/// unpacked sit beside the real `brindle.exe`.
fn exe_paths(exe: &Path) -> Vec<PathBuf> {
    let mut paths = vec![exe.to_path_buf()];
    // dunce drops the `\\?\` prefix `canonicalize` adds on Windows.
    if let Ok(resolved) = dunce::canonicalize(exe)
        && resolved != exe
    {
        paths.push(resolved);
    }
    paths
}

/// The first of `names` that is a file beside one of `exes` (see
/// `exe_paths`), trying every name beside each executable path in turn.
/// `is_file` is the probe (the shell passes `Path::is_file`).
fn beside_exe(
    exes: &[PathBuf],
    names: &[&str],
    is_file: &dyn Fn(&Path) -> bool,
) -> Option<PathBuf> {
    exes.iter()
        .flat_map(|exe| names.iter().map(move |name| exe.with_file_name(name)))
        .find(|path| is_file(path))
}

/// The `brindle-remote` helper, for remote tabs and for the local terminal
/// server alike: `BRINDLE_REMOTE_AGENT`, else beside Brindle's executable.
/// `None` when there is none; each caller says how to fix that.
pub(crate) fn find_helper() -> Result<Option<PathBuf>> {
    let exe = std::env::current_exe().context("locating Brindle executable")?;
    find_helper_in(
        std::env::var_os("BRINDLE_REMOTE_AGENT"),
        &exe_paths(&exe),
        &|path: &Path| path.is_file(),
    )
}

/// [`find_helper`]'s choice, from the variable's value, Brindle's executable
/// paths (`exe_paths`) and the probe for a file.
fn find_helper_in(
    variable: Option<OsString>,
    exes: &[PathBuf],
    is_file: &dyn Fn(&Path) -> bool,
) -> Result<Option<PathBuf>> {
    if let Some(path) = variable.filter(|path| !path.is_empty()) {
        let path = PathBuf::from(path);
        if is_file(&path) {
            return Ok(Some(path));
        }
        bail!(
            "BRINDLE_REMOTE_AGENT does not name a file: {}",
            path.display()
        );
    }
    Ok(beside_exe(exes, &HELPER_NAMES, is_file))
}

/// The helper the local terminal server runs: `BRINDLE_SERVER_HELPER`, then on
/// Linux `BRINDLE_REMOTE_AGENT`, then beside Brindle's executable. On Windows
/// `BRINDLE_REMOTE_AGENT` names the Linux helper for remote profiles, so it is
/// ignored and only `brindle-remote.exe` beside `brindle.exe` is looked for.
#[cfg_attr(not(any(target_os = "linux", windows)), allow(dead_code))]
pub(crate) fn find_server_helper() -> Result<Option<PathBuf>> {
    let exe = std::env::current_exe().context("locating Brindle executable")?;
    find_server_helper_in(
        std::env::var_os("BRINDLE_SERVER_HELPER"),
        std::env::var_os("BRINDLE_REMOTE_AGENT"),
        &exe_paths(&exe),
        cfg!(windows),
        &|path: &Path| path.is_file(),
    )
}

/// [`find_server_helper`]'s choice, from the variables' values, Brindle's
/// executable paths (`exe_paths`), the platform and the probe for a file.
fn find_server_helper_in(
    server_variable: Option<OsString>,
    remote_variable: Option<OsString>,
    exes: &[PathBuf],
    windows: bool,
    is_file: &dyn Fn(&Path) -> bool,
) -> Result<Option<PathBuf>> {
    if let Some(path) = server_variable.filter(|path| !path.is_empty()) {
        let path = PathBuf::from(path);
        if is_file(&path) {
            return Ok(Some(path));
        }
        bail!(
            "BRINDLE_SERVER_HELPER does not name a file: {}",
            path.display()
        );
    }
    if windows {
        return Ok(beside_exe(exes, &["brindle-remote.exe"], is_file));
    }
    find_helper_in(remote_variable, exes, is_file)
}

/// Waits for `child` to exit, killing it if `cancel` is set or `timeout`
/// passes first.
pub(crate) fn wait_process(
    mut child: Child,
    cancel: &AtomicBool,
    timeout: Duration,
) -> Result<ExitStatus> {
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

/// Builds a command whose stdio is the remote command's stdio.
pub fn remote_command(target: &RemoteTarget, program: &str, args: &[String]) -> Result<Command> {
    let mut command = match target {
        RemoteTarget::Ssh {
            host,
            user,
            port,
            identity_file,
            ssh_args,
            connect_at_launch: _,
        } => {
            let mut command = Command::new("ssh");
            command.args([
                "-T",
                "-o",
                "ConnectTimeout=15",
                "-o",
                "ServerAliveInterval=5",
                "-o",
                "ServerAliveCountMax=3",
            ]);
            command.args(ssh_args);
            if let Some(port) = port {
                command.args(["-p", &port.to_string()]);
            }
            if let Some(identity) = identity_file {
                command.args(["-i", identity]);
            }
            let destination = match user {
                Some(user) => format!("{user}@{host}"),
                None => host.clone(),
            };
            command.arg("--").arg(destination);
            let remote =
                shlex::try_join(std::iter::once(program).chain(args.iter().map(String::as_str)))
                    .context("remote command contains a NUL byte")?;
            command.arg(remote);
            command
        }
        RemoteTarget::Container {
            engine,
            container,
            user,
            connect_at_launch: _,
        } => {
            if engine != "docker" && engine != "podman" {
                bail!("unsupported container engine {engine:?}; use docker or podman");
            }
            let mut command = Command::new(engine);
            command.args(["exec", "-i"]);
            if let Some(user) = user {
                command.args(["--user", user]);
            }
            command.arg(container).arg(program).args(args);
            command
        }
    };
    hide_window(&mut command);
    Ok(command)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where the terminal server can't run, the seam item says so. (On
    /// Linux and Windows it starts a real session, which the client's own
    /// tests cover against scripted cores and a real helper.)
    #[test]
    fn agent_link_says_why_it_cannot_start_without_the_server() {
        assert_eq!(
            NEEDS_THE_SERVER.to_string(),
            "agent sessions run in the terminal server, which needs Linux or Windows"
        );
        #[cfg(not(any(target_os = "linux", windows)))]
        {
            let (sink, _feed) = AgentSink::with_fault(None);
            let start = AgentStart::Attach { session: 1, key: None, recorded: false };
            assert_eq!(agent_link(Host::Local, start, sink).err(), Some(NEEDS_THE_SERVER));
        }
    }

    #[test]
    fn ssh_command_uses_config_alias_and_quotes_remote_argv() {
        let target = RemoteTarget::Ssh {
            host: "A8".into(),
            user: Some("dev".into()),
            port: Some(2222),
            identity_file: Some("key file".into()),
            ssh_args: vec!["-o".into(), "BatchMode=yes".into()],
            connect_at_launch: false,
        };
        let command = remote_command(
            &target,
            "git",
            &["-C".into(), "work tree".into(), "status".into()],
        )
        .unwrap();
        let args: Vec<_> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "-T",
                "-o",
                "ConnectTimeout=15",
                "-o",
                "ServerAliveInterval=5",
                "-o",
                "ServerAliveCountMax=3",
                "-o",
                "BatchMode=yes",
                "-p",
                "2222",
                "-i",
                "key file",
                "--",
                "dev@A8",
                "git -C 'work tree' status",
            ]
        );
    }

    #[test]
    fn container_command_is_not_shell_interpreted() {
        let target = RemoteTarget::Container {
            engine: "docker".into(),
            container: "devbox".into(),
            user: Some("1000".into()),
            connect_at_launch: false,
        };
        let command =
            remote_command(&target, "/bin/sh", &["-lc".into(), "echo hi".into()]).unwrap();
        let args: Vec<_> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
            [
                "exec", "-i", "--user", "1000", "devbox", "/bin/sh", "-lc", "echo hi"
            ]
        );
    }

    /// A probe that answers yes for `files` alone.
    fn only(files: &[PathBuf]) -> impl Fn(&Path) -> bool + '_ {
        move |path| files.iter().any(|file| file == path)
    }

    #[test]
    fn the_helper_is_the_variable_or_beside_brindle() {
        let dir = PathBuf::from("/opt/brindle");
        let exes = [dir.join("brindle")];
        let find = |variable: Option<&str>, files: &[PathBuf]| {
            find_helper_in(variable.map(Into::into), &exes, &only(files))
        };
        let (helper, archive) = (
            dir.join("brindle-remote"),
            dir.join("brindle-remote-linux-x86_64"),
        );
        assert_eq!(find(None, &[]).unwrap(), None);
        assert_eq!(find(None, std::slice::from_ref(&helper)).unwrap(), Some(helper.clone()));
        // An empty variable counts as unset.
        assert_eq!(
            find(Some(""), std::slice::from_ref(&helper)).unwrap(),
            Some(helper.clone())
        );
        // The archive's name comes before the installed one.
        assert_eq!(
            find(None, &[helper.clone(), archive.clone()]).unwrap(),
            Some(archive)
        );
        // The variable's path is used when it is a file, and is an error naming
        // the variable when it isn't, whatever sits beside Brindle.
        let named = PathBuf::from("/elsewhere/helper");
        let error = find(Some("/elsewhere/helper"), std::slice::from_ref(&helper)).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("BRINDLE_REMOTE_AGENT does not name a file"),
            "{error}"
        );
        assert_eq!(
            find(Some("/elsewhere/helper"), &[named.clone(), helper]).unwrap(),
            Some(named)
        );
    }

    #[test]
    fn the_server_helper_order_holds_on_both_platforms() {
        let dir = PathBuf::from("/opt/brindle");
        let exes = [dir.join("brindle")];
        let (remote, server) = (
            PathBuf::from("/x/linux-helper"),
            PathBuf::from("/x/server-helper"),
        );
        let (helper, exe_helper) = (dir.join("brindle-remote"), dir.join("brindle-remote.exe"));
        let find = |s: Option<&Path>, r: Option<&Path>, windows, files: &[PathBuf]| {
            find_server_helper_in(
                s.map(Into::into),
                r.map(Into::into),
                &exes,
                windows,
                &only(files),
            )
        };
        for windows in [false, true] {
            assert_eq!(find(None, None, windows, &[]).unwrap(), None);
        }
        // Beside the exe: Linux names, and on Windows only brindle-remote.exe.
        assert_eq!(
            find(None, None, false, std::slice::from_ref(&helper)).unwrap(),
            Some(helper.clone())
        );
        assert_eq!(find(None, None, true, std::slice::from_ref(&helper)).unwrap(), None);
        let beside = [helper.clone(), exe_helper.clone()];
        assert_eq!(
            find(None, None, true, &beside).unwrap(),
            Some(exe_helper.clone())
        );
        // BRINDLE_REMOTE_AGENT: honoured on Linux, ignored on Windows.
        let with_remote = [helper.clone(), exe_helper.clone(), remote.clone()];
        assert_eq!(
            find(None, Some(&remote), false, &with_remote).unwrap(),
            Some(remote.clone())
        );
        assert_eq!(
            find(None, Some(&remote), true, &with_remote).unwrap(),
            Some(exe_helper.clone())
        );
        // BRINDLE_SERVER_HELPER wins on both; empty counts as unset.
        let all = [helper, exe_helper, remote.clone(), server.clone()];
        for windows in [false, true] {
            assert_eq!(
                find(Some(&server), Some(&remote), windows, &all).unwrap(),
                Some(server.clone())
            );
            assert!(
                find(Some(Path::new("")), None, windows, &all)
                    .unwrap()
                    .is_some()
            );
            let error =
                find(Some(Path::new("/x/missing")), Some(&remote), windows, &all).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("BRINDLE_SERVER_HELPER does not name a file"),
                "{error}"
            );
        }
    }

    /// A WinGet-style install: Brindle started through a link to `brindle`
    /// in another directory finds the helpers beside the real file, unless
    /// the link's directory has one too.
    #[test]
    fn helpers_are_found_beside_the_file_behind_a_link() {
        let (links, real) = (PathBuf::from("/links"), PathBuf::from("/real"));
        let exes = [links.join("brindle"), real.join("brindle")];
        let helper = |files: &[PathBuf]| find_helper_in(None, &exes, &only(files)).unwrap();
        let server = |windows, files: &[PathBuf]| {
            find_server_helper_in(None, None, &exes, windows, &only(files)).unwrap()
        };
        let names = [
            "brindle-remote-linux-x86_64",
            "brindle-remote",
            "brindle-remote.exe",
        ];
        let behind: Vec<PathBuf> = names.iter().map(|name| real.join(name)).collect();
        let linux = real.join("brindle-remote-linux-x86_64");
        assert_eq!(helper(&behind), Some(linux.clone()));
        assert_eq!(server(true, &behind), Some(real.join("brindle-remote.exe")));
        assert_eq!(server(false, &behind), Some(linux));
        // Beside the link comes first, whichever name it has there.
        let mut both = behind.clone();
        both.push(links.join("brindle-remote"));
        both.push(links.join("brindle-remote.exe"));
        assert_eq!(helper(&both), Some(links.join("brindle-remote")));
        assert_eq!(server(true, &both), Some(links.join("brindle-remote.exe")));
    }

    /// The shell's own resolution: the link as started, then the file behind
    /// it, and only the one path when the file is no link.
    #[cfg(unix)]
    #[test]
    fn the_exe_paths_are_the_link_then_the_file_behind_it() {
        let dir = crate::test_support::TempDir::new("exe-paths");
        let (real, links) = (dir.join("real"), dir.join("links"));
        std::fs::create_dir_all(&real).unwrap();
        std::fs::create_dir_all(&links).unwrap();
        std::fs::write(real.join("brindle"), b"").unwrap();
        let link = links.join("brindle");
        std::os::unix::fs::symlink(real.join("brindle"), &link).unwrap();
        let real = dunce::canonicalize(&real).unwrap();
        let paths = exe_paths(&link);
        let alone = exe_paths(&real.join("brindle"));
        assert_eq!(paths, [link, real.join("brindle")]);
        assert_eq!(alone, [real.join("brindle")]);
    }
}
