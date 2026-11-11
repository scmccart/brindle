//! Remote Linux host and container support.
//!
//! Terminal PTYs are served by the `brindle-remote` helper over a framed
//! stdio connection, deployed and reached through SSH or a container's exec
//! (`remote_command`).

pub mod client;
#[cfg(target_os = "linux")]
pub mod local;
pub use brindle_remote_protocol as protocol;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context as _, Result, bail};

use crate::config::RemoteTarget;

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
}
