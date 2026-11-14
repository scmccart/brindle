//! Remote Linux host and container support.
//!
//! Terminal PTYs are served by the `brindle-remote` helper over a framed
//! stdio connection, deployed and reached through SSH or a container's exec
//! (`remote_command`).

pub mod client;
#[cfg(any(target_os = "linux", windows))]
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

/// The first of `names` that is a file beside `exe` (see `exe_paths`).
fn beside_exe(exe: &Path, names: &[&str]) -> Option<PathBuf> {
    exe_paths(exe)
        .into_iter()
        .flat_map(|exe| names.iter().map(move |name| exe.with_file_name(name)))
        .find(|path| path.is_file())
}

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
    Ok(beside_exe(exe, &HELPER_NAMES))
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
        return Ok(beside_exe(exe, &["brindle-remote.exe"]));
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

    /// A WinGet-style install: Brindle started through a link to `brindle`
    /// in another directory finds the helpers beside the real file, unless
    /// the link's directory has one too.
    #[test]
    fn helpers_are_found_beside_the_file_behind_a_link() {
        let dir = std::env::temp_dir().join(format!("brindle-helper-link-{}", std::process::id()));
        let (real, links) = (dir.join("real"), dir.join("links"));
        std::fs::create_dir_all(&real).unwrap();
        std::fs::create_dir_all(&links).unwrap();
        std::fs::write(real.join("brindle"), b"").unwrap();
        let link = links.join("brindle");
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(real.join("brindle"), &link);
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_file(real.join("brindle"), &link);
        if let Err(error) = made {
            // Windows creates symlinks only elevated or in developer mode.
            eprintln!("skipped: can't create a symlink here: {error}");
            std::fs::remove_dir_all(&dir).ok();
            return;
        }
        let real = dunce::canonicalize(&real).unwrap();
        assert_eq!(find_helper_in(None, &link).unwrap(), None);
        for name in ["brindle-remote-linux-x86_64", "brindle-remote", "brindle-remote.exe"] {
            std::fs::write(real.join(name), b"").unwrap();
        }
        let linux = real.join("brindle-remote-linux-x86_64");
        assert_eq!(find_helper_in(None, &link).unwrap(), Some(linux.clone()));
        let server = |windows| find_server_helper_in(None, None, &link, windows).unwrap();
        assert_eq!(server(true), Some(real.join("brindle-remote.exe")));
        assert_eq!(server(false), Some(linux));
        // Beside the link comes first.
        std::fs::write(links.join("brindle-remote.exe"), b"").unwrap();
        assert_eq!(server(true), Some(links.join("brindle-remote.exe")));
        std::fs::remove_dir_all(&dir).ok();
    }
}
