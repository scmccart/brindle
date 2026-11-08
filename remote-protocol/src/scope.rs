//! Starting a terminal server role in its own systemd scope, shared by
//! Brindle (which starts the core) and `brindle-remote` (a core starts
//! holders, a holder restarts the core).
//!
//! A desktop launches Brindle in an app scope with `KillMode=control-group`,
//! so stopping that scope (logging out, or `systemctl --user stop`) would
//! kill a server started inside it, double fork or not. Running the role
//! under `systemd-run --user --scope` moves it into a scope of its own, as
//! tmux does with its `tmux-spawn-*.scope` units.

use std::ffi::{OsStr, OsString};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// Set to `1` to start roles without a scope (tests, the e2e suite).
pub const NO_SCOPE_ENV: &str = "BRINDLE_NO_SCOPE";

/// How a role gets started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    /// Through this `systemd-run`.
    Scope(PathBuf),
    /// Directly, for this reason (worth one log line).
    Plain(&'static str),
}

/// Decides from the environment's values: `BRINDLE_NO_SCOPE`,
/// `XDG_RUNTIME_DIR`, `PATH`, and whether a path exists or is an executable
/// file (the parameters let tests decide without touching the system).
pub fn decide(
    no_scope: Option<&OsStr>,
    runtime_dir: Option<&OsStr>,
    path: Option<&OsStr>,
    exists: impl Fn(&Path) -> bool,
    executable: impl Fn(&Path) -> bool,
) -> Decision {
    if no_scope == Some(OsStr::new("1")) {
        return Decision::Plain("BRINDLE_NO_SCOPE=1");
    }
    let Some(runtime) = runtime_dir.filter(|dir| !dir.is_empty()) else {
        return Decision::Plain("XDG_RUNTIME_DIR is unset");
    };
    if !exists(&Path::new(runtime).join("systemd/private")) {
        return Decision::Plain("no systemd user manager");
    }
    let found = path.and_then(|path| {
        std::env::split_paths(path)
            .filter(|dir| dir.is_absolute())
            .map(|dir| dir.join("systemd-run"))
            .find(|candidate| executable(candidate))
    });
    match found {
        Some(program) => Decision::Scope(program),
        None => Decision::Plain("systemd-run isn't on PATH"),
    }
}

/// [`decide`] for this process.
pub fn current() -> Decision {
    decide(
        std::env::var_os(NO_SCOPE_ENV).as_deref(),
        std::env::var_os("XDG_RUNTIME_DIR").as_deref(),
        std::env::var_os("PATH").as_deref(),
        |path| path.exists(),
        |path| {
            path.metadata()
                .is_ok_and(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        },
    )
}

/// The command that runs `program args` for `role` (`serve` or `hold`):
/// through `systemd-run` in a `brindle-<role>-<random>` scope for
/// [`Decision::Scope`], else directly.
///
/// `systemd-run --scope` execs the program itself, so a `/proc/self/exe`
/// program would name `systemd-run`: callers pass `/proc/<own pid>/exe`
/// instead, which still names their binary after an install replaced it.
pub fn command(decision: &Decision, role: &str, program: &OsStr, args: &[OsString]) -> Command {
    match decision {
        Decision::Plain(_) => {
            let mut command = Command::new(program);
            command.args(args);
            command
        }
        Decision::Scope(systemd_run) => {
            let mut command = Command::new(systemd_run);
            command
                .args(["--user", "--scope", "--collect", "--quiet"])
                .arg(format!("--unit={}", unit_name(role)))
                .arg("--")
                .arg(program)
                .args(args);
            command
        }
    }
}

/// `brindle-<role>-<pid><nanoseconds>` in hex, unique enough for units that
/// are collected once their processes are gone.
fn unit_name(role: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|time| time.subsec_nanos())
        .unwrap_or_default();
    format!("brindle-{role}-{:x}{nanos:08x}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(value: &str) -> Option<&OsStr> {
        Some(OsStr::new(value))
    }

    fn decide_with(no_scope: Option<&OsStr>, runtime: Option<&OsStr>, path: Option<&OsStr>) -> Decision {
        decide(
            no_scope,
            runtime,
            path,
            |path| path == Path::new("/run/user/1000/systemd/private"),
            |path| path == Path::new("/usr/bin/systemd-run"),
        )
    }

    #[test]
    fn a_user_manager_and_systemd_run_give_a_scope() {
        assert_eq!(
            decide_with(None, os("/run/user/1000"), os("/home/u/bin:/usr/bin")),
            Decision::Scope(PathBuf::from("/usr/bin/systemd-run"))
        );
        assert_eq!(
            decide_with(os("0"), os("/run/user/1000"), os("/usr/bin")),
            Decision::Scope(PathBuf::from("/usr/bin/systemd-run"))
        );
    }

    #[test]
    fn anything_missing_starts_the_role_directly() {
        let full = (os("/run/user/1000"), os("/usr/bin"));
        assert_eq!(decide_with(os("1"), full.0, full.1), Decision::Plain("BRINDLE_NO_SCOPE=1"));
        assert_eq!(decide_with(None, None, full.1), Decision::Plain("XDG_RUNTIME_DIR is unset"));
        assert_eq!(decide_with(None, os(""), full.1), Decision::Plain("XDG_RUNTIME_DIR is unset"));
        assert_eq!(
            decide_with(None, os("/run/user/1001"), full.1),
            Decision::Plain("no systemd user manager")
        );
        assert_eq!(
            decide_with(None, full.0, os("/bin:usr/bin")),
            Decision::Plain("systemd-run isn't on PATH")
        );
        assert_eq!(decide_with(None, full.0, None), Decision::Plain("systemd-run isn't on PATH"));
    }

    #[test]
    fn a_scoped_command_wraps_the_program() {
        let args = [OsString::from("serve"), OsString::from("--ready-fd"), OsString::from("3")];
        let scoped = command(
            &Decision::Scope(PathBuf::from("/usr/bin/systemd-run")),
            "serve",
            OsStr::new("/opt/brindle-remote"),
            &args,
        );
        assert_eq!(scoped.get_program(), "/usr/bin/systemd-run");
        let got: Vec<_> = scoped.get_args().map(|arg| arg.to_string_lossy().into_owned()).collect();
        assert_eq!(&got[..4], ["--user", "--scope", "--collect", "--quiet"]);
        assert!(got[4].starts_with("--unit=brindle-serve-"), "{}", got[4]);
        assert_eq!(&got[5..], ["--", "/opt/brindle-remote", "serve", "--ready-fd", "3"]);

        let plain = command(&Decision::Plain("test"), "hold", OsStr::new("/opt/x"), &args);
        assert_eq!(plain.get_program(), "/opt/x");
        assert_eq!(plain.get_args().collect::<Vec<_>>(), ["serve", "--ready-fd", "3"]);
    }
}
