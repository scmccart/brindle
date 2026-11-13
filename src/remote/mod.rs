//! Remote Linux host and container support.
//!
//! Terminal PTYs are served by the `brindle-remote` helper over a framed
//! stdio connection. tmux control clients use SSH/container-exec pipes
//! directly because tmux control mode already is a byte-stream protocol.

pub mod client;
// Nothing on Windows uses the client until local tabs run in the server
// there (`Backend::Server`, task 7.3 of `terminal-server-windows`).
#[cfg(any(target_os = "linux", windows))]
#[cfg_attr(windows, allow(dead_code, unused_imports))]
pub mod local;
pub use brindle_remote_protocol as protocol;

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};

use crate::config::{Profile, RemoteTarget};

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

/// The `brindle-remote` helper, for remote tabs and for the local terminal
/// server alike: `BRINDLE_REMOTE_AGENT`, else beside Brindle's executable.
/// `None` when there is none; each caller says how to fix that.
pub(crate) fn find_helper() -> Result<Option<PathBuf>> {
    let exe = std::env::current_exe().context("locating Brindle executable")?;
    find_helper_in(std::env::var_os("BRINDLE_REMOTE_AGENT"), &exe)
}

fn find_helper_in(variable: Option<OsString>, exe: &Path) -> Result<Option<PathBuf>> {
    if let Some(path) = variable.filter(|path| !path.is_empty()) {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(Some(path));
        }
        bail!("BRINDLE_REMOTE_AGENT does not name a file: {}", path.display());
    }
    Ok(HELPER_NAMES.iter().map(|name| exe.with_file_name(name)).find(|path| path.is_file()))
}

/// The helper the local terminal server runs: `BRINDLE_SERVER_HELPER`, then on
/// Linux `BRINDLE_REMOTE_AGENT`, then beside Brindle's executable. On Windows
/// `BRINDLE_REMOTE_AGENT` names the Linux helper for remote profiles, so it is
/// ignored and only `brindle-remote.exe` beside `brindle.exe` is looked for.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn find_server_helper() -> Result<Option<PathBuf>> {
    let exe = std::env::current_exe().context("locating Brindle executable")?;
    find_server_helper_in(
        std::env::var_os("BRINDLE_SERVER_HELPER"),
        std::env::var_os("BRINDLE_REMOTE_AGENT"),
        &exe,
        cfg!(windows),
    )
}

fn find_server_helper_in(
    server_variable: Option<OsString>,
    remote_variable: Option<OsString>,
    exe: &Path,
    windows: bool,
) -> Result<Option<PathBuf>> {
    if let Some(path) = server_variable.filter(|path| !path.is_empty()) {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(Some(path));
        }
        bail!("BRINDLE_SERVER_HELPER does not name a file: {}", path.display());
    }
    if windows {
        let path = exe.with_file_name("brindle-remote.exe");
        return Ok(path.is_file().then_some(path));
    }
    find_helper_in(remote_variable, exe)
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

/// How long [`detach_client`] may take, connecting included.
const DETACH_LIMIT: Duration = Duration::from_secs(10);

/// The command that detaches the tmux client `client` on the profile's
/// endpoint: `tmux_program <tmux_options> detach-client -t <client>`.
pub fn detach_command(
    profile: &Profile,
    tmux_program: &str,
    tmux_options: &[String],
    client: &str,
) -> Result<Command> {
    let mut target = profile
        .remote
        .clone()
        .context("remote profile has no target")?;
    if let RemoteTarget::Ssh { ssh_args, .. } = &mut target {
        // No password prompt on a connection nobody is watching; the first
        // value of an option wins in ssh.
        ssh_args.splice(0..0, ["-o".to_string(), "BatchMode=yes".to_string()]);
    }
    let mut args = tmux_options.to_vec();
    args.extend(["detach-client".to_string(), "-t".to_string(), client.to_string()]);
    remote_command(&target, tmux_program, &args)
}

/// Runs `command` to completion in at most [`DETACH_LIMIT`], silently. Meant
/// for a background thread: the connection it needs may be gone.
pub fn detach_client(mut command: Command) {
    use std::process::Stdio;
    command.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    match command.spawn() {
        Ok(child) => {
            let never = AtomicBool::new(false);
            if let Err(error) = wait_process(child, &never, DETACH_LIMIT) {
                log::debug!("detaching the dropped tmux client: {error:#}");
            }
        }
        Err(error) => log::debug!("detaching the dropped tmux client: {error}"),
    }
}

/// A tmux process ready to spawn, and what to write to its stdin first.
/// The caller sets its stdio.
pub struct TmuxProcess {
    pub command: Command,
    pub initial_input: Vec<u8>,
}

/// Builds the remote tmux process used by `TmuxSession`, starting in `cwd`
/// on the endpoint when given (`~` is expanded there, not here).
pub fn tmux_command(
    profile: &Profile,
    program: &str,
    args: &[String],
    cwd: Option<&str>,
) -> Result<TmuxProcess> {
    let target = profile
        .remote
        .as_ref()
        .context("remote profile has no target")?;
    let mut argv = vec![program.to_string()];
    argv.extend(args.iter().cloned());
    let command = shlex::try_join(argv.iter().map(String::as_str))
        .context("tmux command contains a NUL byte")?;
    let launch = match cwd {
        Some("~") => format!("cd \"$HOME\" && exec {command}"),
        Some(cwd) if cwd.starts_with("~/") => {
            let rest = shlex::try_quote(&cwd[2..]).context("tmux directory contains a NUL byte")?;
            format!("cd \"$HOME\"/{rest} && exec {command}")
        }
        Some(cwd) => {
            let cwd = shlex::try_quote(cwd).context("tmux directory contains a NUL byte")?;
            format!("cd {cwd} && exec {command}")
        }
        None => format!("exec {command}"),
    };
    let script =
        format!("IFS= read -r brindle_env || exit 1; eval \"$brindle_env\" || exit 1; {launch}");
    let command = remote_command(target, "sh", &["-lc".into(), script])?;
    let environment: BTreeMap<_, _> = crate::terminal::child_env(profile).into_iter().collect();
    let mut setup = String::new();
    for (key, value) in environment {
        if !valid_env_name(&key) {
            bail!("invalid environment variable name {key:?}");
        }
        if value.contains(['\n', '\r']) {
            bail!("remote tmux environment variable {key:?} contains a newline");
        }
        let value = shlex::try_quote(&value).context("environment value contains a NUL byte")?;
        setup.push_str("export ");
        setup.push_str(&key);
        setup.push('=');
        setup.push_str(&value);
        setup.push(';');
    }
    setup.push('\n');
    Ok(TmuxProcess {
        command,
        initial_input: setup.into_bytes(),
    })
}

fn valid_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|ch| ch.is_ascii_alphabetic() || ch == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_command_uses_config_alias_and_quotes_remote_argv() {
        let target = RemoteTarget::Ssh {
            host: "A8".into(),
            user: Some("dev".into()),
            port: Some(2222),
            identity_file: Some("key file".into()),
            ssh_args: vec!["-o".into(), "BatchMode=yes".into()],
        };
        let command = remote_command(
            &target,
            "tmux",
            &["-L".into(), "work socket".into(), "-C".into()],
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
                "tmux -L 'work socket' -C",
            ]
        );
    }

    #[test]
    fn container_command_is_not_shell_interpreted() {
        let target = RemoteTarget::Container {
            engine: "docker".into(),
            container: "devbox".into(),
            user: Some("1000".into()),
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

    #[test]
    fn remote_tmux_applies_profile_environment_without_interpreting_values() {
        let profile = Profile {
            remote: Some(RemoteTarget::Container {
                engine: "docker".into(),
                container: "devbox".into(),
                user: None,
            }),
            env: [("TOKEN".into(), "a b".into())].into(),
            ..Profile::default_shell()
        };
        let process = tmux_command(&profile, "tmux", &["-C".into()], None).unwrap();
        let args: Vec<_> = process
            .command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(&args[..5], ["exec", "-i", "devbox", "sh", "-lc"]);
        assert!(
            args[5].contains("eval \"$brindle_env\" || exit 1; exec tmux -C"),
            "{}",
            args[5]
        );
        assert!(!args.iter().any(|arg| arg.contains("TOKEN")));
        let prelude = String::from_utf8(process.initial_input).unwrap();
        assert!(prelude.contains("export TOKEN='a b';"), "{prelude}");
        assert!(prelude.contains("export TERM=xterm-256color;"), "{prelude}");
    }

    #[test]
    fn remote_tmux_rejects_multiline_environment_values() {
        let profile = Profile {
            remote: Some(RemoteTarget::Container {
                engine: "docker".into(),
                container: "devbox".into(),
                user: None,
            }),
            env: [("TOKEN".into(), "first\nsecond".into())].into(),
            ..Profile::default_shell()
        };
        assert!(tmux_command(&profile, "tmux", &["-C".into()], None).is_err());
    }

    #[test]
    fn remote_tmux_expands_tilde_cwd_on_the_endpoint() {
        let profile = Profile {
            remote: Some(RemoteTarget::Ssh {
                host: "A8".into(),
                user: None,
                port: None,
                identity_file: None,
                ssh_args: Vec::new(),
            }),
            ..Profile::default_shell()
        };
        let process = tmux_command(
            &profile,
            "tmux",
            &["-C".into(), "new-session".into()],
            Some("~/src tree"),
        )
        .unwrap();
        let args: Vec<_> = process
            .command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(args.iter().any(|arg| arg == "A8"));
        let script = args.last().unwrap();
        assert!(script.contains("cd \"$HOME\"/"), "{script}");
        assert!(script.contains("src tree"), "{script}");
        assert!(script.contains("exec tmux -C new-session"), "{script}");
        assert!(!script.contains(" -c "));
    }

    #[test]
    fn the_helper_is_the_variable_or_beside_brindle() {
        let dir = std::env::temp_dir().join(format!("brindle-find-helper-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("brindle");
        assert_eq!(find_helper_in(None, &exe).unwrap(), None);
        std::fs::write(dir.join("brindle-remote"), b"").unwrap();
        assert_eq!(find_helper_in(None, &exe).unwrap(), Some(dir.join("brindle-remote")));
        assert_eq!(find_helper_in(Some("".into()), &exe).unwrap(), Some(dir.join("brindle-remote")));
        std::fs::write(dir.join("brindle-remote-linux-x86_64"), b"").unwrap();
        assert_eq!(find_helper_in(None, &exe).unwrap(), Some(dir.join("brindle-remote-linux-x86_64")));
        let named = dir.join("elsewhere");
        let error = find_helper_in(Some(named.clone().into()), &exe).unwrap_err();
        assert!(error.to_string().contains("does not name a file"), "{error}");
        std::fs::write(&named, b"").unwrap();
        assert_eq!(find_helper_in(Some(named.clone().into()), &exe).unwrap(), Some(named));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_server_helper_order_holds_on_both_platforms() {
        let dir = std::env::temp_dir().join(format!("brindle-server-helper-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("brindle");
        let remote = dir.join("linux-helper");
        let server = dir.join("server-helper");
        let find = |s: Option<&Path>, r: Option<&Path>, windows| {
            find_server_helper_in(s.map(Into::into), r.map(Into::into), &exe, windows)
        };
        for windows in [false, true] {
            assert_eq!(find(None, None, windows).unwrap(), None);
        }
        // Beside the exe: Linux names, or brindle-remote.exe on Windows.
        std::fs::write(dir.join("brindle-remote"), b"").unwrap();
        assert_eq!(find(None, None, false).unwrap(), Some(dir.join("brindle-remote")));
        assert_eq!(find(None, None, true).unwrap(), None);
        std::fs::write(dir.join("brindle-remote.exe"), b"").unwrap();
        assert_eq!(find(None, None, true).unwrap(), Some(dir.join("brindle-remote.exe")));
        // BRINDLE_REMOTE_AGENT: honoured on Linux, ignored on Windows.
        std::fs::write(&remote, b"").unwrap();
        assert_eq!(find(None, Some(&remote), false).unwrap(), Some(remote.clone()));
        assert_eq!(find(None, Some(&remote), true).unwrap(), Some(dir.join("brindle-remote.exe")));
        // BRINDLE_SERVER_HELPER wins on both; empty counts as unset.
        std::fs::write(&server, b"").unwrap();
        for windows in [false, true] {
            assert_eq!(find(Some(&server), Some(&remote), windows).unwrap(), Some(server.clone()));
            assert_eq!(
                find(Some(Path::new("")), None, windows).unwrap().is_some(),
                true
            );
            let missing = dir.join("missing");
            let error = find(Some(&missing), Some(&remote), windows).unwrap_err();
            assert!(
                error.to_string().contains("BRINDLE_SERVER_HELPER does not name a file"),
                "{error}"
            );
        }
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn detach_command_names_the_client_and_never_prompts() {
        let profile = Profile {
            remote: Some(RemoteTarget::Ssh {
                host: "A8".into(),
                user: None,
                port: None,
                identity_file: None,
                ssh_args: vec![],
            }),
            ..Profile::default()
        };
        let command =
            detach_command(&profile, "tmux", &["-L".into(), "work".into()], "client-42").unwrap();
        let args: Vec<_> = command.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        assert!(args.windows(2).any(|w| w == ["-o", "BatchMode=yes"]), "{args:?}");
        assert_eq!(args.last().unwrap(), "tmux -L work detach-client -t client-42");
        assert!(detach_command(&Profile::default(), "tmux", &[], "c").is_err());
    }
}
