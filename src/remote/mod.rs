//! Remote Linux host and container support.
//!
//! Terminal PTYs are served by the `brindle-remote` helper over a framed
//! stdio connection. tmux control clients use SSH/container-exec pipes
//! directly because tmux control mode already is a byte-stream protocol.

pub mod client;
pub mod protocol;

use std::collections::BTreeMap;
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail};

use crate::config::{Profile, RemoteTarget};

/// Builds a command whose stdio is the remote command's stdio.
pub fn remote_command(target: &RemoteTarget, program: &str, args: &[String]) -> Result<Command> {
    #[allow(unused_mut)] // Mutated below only on Windows for CREATE_NO_WINDOW.
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
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(
        &mut command,
        crate::win::CREATE_NO_WINDOW,
    );
    Ok(command)
}

pub struct TmuxProcess {
    pub command: Command,
    pub initial_input: Vec<u8>,
}

/// Builds the remote tmux control client process used by `TmuxSession`.
pub fn tmux_command(profile: &Profile, program: &str, args: &[String]) -> Result<TmuxProcess> {
    let target = profile
        .remote
        .as_ref()
        .context("remote profile has no target")?;
    let mut args = args.to_vec();
    let cwd = args
        .iter()
        .rposition(|arg| arg == "-c")
        .filter(|at| *at + 2 == args.len())
        .map(|at| {
            let cwd = args.remove(at + 1);
            args.remove(at);
            cwd
        });
    let mut argv = vec![program.to_string()];
    argv.extend(args);
    let command = shlex::try_join(argv.iter().map(String::as_str))
        .context("tmux command contains a NUL byte")?;
    let launch = match cwd.as_deref() {
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
    let mut command = remote_command(target, "sh", &["-lc".into(), script])?;
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
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
        let process = tmux_command(&profile, "tmux", &["-C".into()]).unwrap();
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
        assert!(tmux_command(&profile, "tmux", &["-C".into()]).is_err());
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
            &[
                "-C".into(),
                "new-session".into(),
                "-c".into(),
                "~/src tree".into(),
            ],
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
}
