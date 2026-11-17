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

/// What [`decide`] found: where a role starts, and whether a systemd user
/// manager is reachable. A caller that starts a role without a scope uses
/// the flag to tell a host that could have given one from a host with no
/// systemd (a container).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    pub decision: Decision,
    /// Whether `$XDG_RUNTIME_DIR/systemd/private` exists. Always false when
    /// `BRINDLE_NO_SCOPE=1` switched scopes off, since nothing was looked for.
    pub user_manager: bool,
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
) -> Outcome {
    let plain = |reason, user_manager| Outcome {
        decision: Decision::Plain(reason),
        user_manager,
    };
    if no_scope == Some(OsStr::new("1")) {
        return plain("BRINDLE_NO_SCOPE=1", false);
    }
    let Some(runtime) = runtime_dir.filter(|dir| !dir.is_empty()) else {
        return plain("XDG_RUNTIME_DIR is unset", false);
    };
    if !exists(&Path::new(runtime).join("systemd/private")) {
        return plain("no systemd user manager", false);
    }
    let found = path.and_then(|path| {
        std::env::split_paths(path)
            .filter(|dir| dir.is_absolute())
            .map(|dir| dir.join("systemd-run"))
            .find(|candidate| executable(candidate))
    });
    match found {
        Some(program) => Outcome {
            decision: Decision::Scope(program),
            user_manager: true,
        },
        None => plain("systemd-run isn't on PATH", true),
    }
}

/// [`decide`] for this process.
pub fn current_outcome() -> Outcome {
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

/// Where a role starts in this process: [`current_outcome`]'s decision.
pub fn current() -> Decision {
    current_outcome().decision
}

/// The command that runs `program args`: through `systemd-run` in the scope
/// `unit` (a [`unit_name`]) for [`Decision::Scope`], else directly.
///
/// `systemd-run --scope` execs the program itself, so a `/proc/self/exe`
/// program would name `systemd-run`: callers pass `/proc/<own pid>/exe`
/// instead, which still names their binary after an install replaced it.
pub fn command(decision: &Decision, unit: &str, program: &OsStr, args: &[OsString]) -> Command {
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
                .arg(format!("--unit={unit}"))
                .arg("--")
                .arg(program)
                .args(args);
            command
        }
    }
}

/// `brindle-<role>-<pid><nanoseconds>` in hex for `role` (`serve` or
/// `hold`), unique enough for units that are collected once their processes
/// are gone. `subsec_nanos` is the clock's nanoseconds within its second.
pub fn unit_name(role: &str, pid: u32, subsec_nanos: u32) -> String {
    format!("brindle-{role}-{pid:x}{subsec_nanos:08x}")
}

/// [`unit_name`] for this process now: reads the clock and the pid.
pub fn new_unit_name(role: &str) -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|time| time.subsec_nanos())
        .unwrap_or_default();
    unit_name(role, std::process::id(), nanos)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn os(value: &str) -> Option<&OsStr> {
        Some(OsStr::new(value))
    }

    fn outcome_with(no_scope: Option<&OsStr>, runtime: Option<&OsStr>, path: Option<&OsStr>) -> Outcome {
        decide(
            no_scope,
            runtime,
            path,
            |path| path == Path::new("/run/user/1000/systemd/private"),
            |path| path == Path::new("/usr/bin/systemd-run"),
        )
    }

    fn decide_with(no_scope: Option<&OsStr>, runtime: Option<&OsStr>, path: Option<&OsStr>) -> Decision {
        outcome_with(no_scope, runtime, path).decision
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
    fn the_outcome_says_whether_a_user_manager_was_found() {
        let full = (os("/run/user/1000"), os("/usr/bin"));
        let manager = |no_scope, runtime, path| outcome_with(no_scope, runtime, path).user_manager;
        // A manager with systemd-run: a scope.
        assert!(manager(None, full.0, full.1));
        // A manager whose systemd-run is missing: found, still plain.
        assert!(manager(None, full.0, os("/bin")));
        assert!(manager(None, full.0, None));
        // No manager (a container), or no runtime directory to look in.
        assert!(!manager(None, os("/run/user/1001"), full.1));
        assert!(!manager(None, None, full.1));
        assert!(!manager(None, os(""), full.1));
        // Switched off: nothing is looked for.
        assert!(!manager(os("1"), full.0, full.1));
    }

    #[test]
    fn unit_names_are_the_role_pid_and_nanoseconds_in_hex() {
        assert_eq!(unit_name("serve", 0x4d2, 0x2a), "brindle-serve-4d20000002a");
        assert_eq!(unit_name("hold", 0x1, 0x3b9a_c9ff), "brindle-hold-13b9ac9ff");
    }

    #[test]
    fn a_scoped_command_wraps_the_program() {
        let args = [OsString::from("serve"), OsString::from("--ready-fd"), OsString::from("3")];
        let scoped = command(
            &Decision::Scope(PathBuf::from("/usr/bin/systemd-run")),
            "brindle-serve-4d20000002a",
            OsStr::new("/opt/brindle-remote"),
            &args,
        );
        assert_eq!(scoped.get_program(), "/usr/bin/systemd-run");
        let got: Vec<_> = scoped.get_args().map(|arg| arg.to_string_lossy().into_owned()).collect();
        assert_eq!(&got[..4], ["--user", "--scope", "--collect", "--quiet"]);
        assert_eq!(got[4], "--unit=brindle-serve-4d20000002a");
        assert_eq!(&got[5..], ["--", "/opt/brindle-remote", "serve", "--ready-fd", "3"]);

        let plain = command(&Decision::Plain("test"), "brindle-hold-1", OsStr::new("/opt/x"), &args);
        assert_eq!(plain.get_program(), "/opt/x");
        assert_eq!(plain.get_args().collect::<Vec<_>>(), ["serve", "--ready-fd", "3"]);
    }
}
