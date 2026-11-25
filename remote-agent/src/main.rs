// railway R3, types R2: no `_ =>` on one of Brindle's own enums. Non-test
// code only; `docs/architecture/README.md` lists the exceptions.
#![cfg_attr(not(test), warn(clippy::wildcard_enum_match_arm))]

#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "the core's agent sessions (task group 7 of agents-sessions) call it; until then only its tests do"
    )
)]
mod acp;
mod conpty;
#[cfg(target_os = "linux")]
mod liveness;
#[cfg(any(target_os = "linux", windows))]
mod pty;
#[cfg(any(target_os = "linux", windows))]
mod server;
#[cfg(test)]
mod test_support;

use std::env;
use std::process::ExitCode;

#[cfg(any(target_os = "linux", windows))]
const USAGE: &str = server::cli::USAGE;
#[cfg(not(any(target_os = "linux", windows)))]
const USAGE: &str = "usage: brindle-remote --version";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("brindle-remote: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args().skip(1);
    let mode = args.next();
    let rest: Vec<String> = args.collect();
    let has_extra = !rest.is_empty();

    match mode.as_deref() {
        Some("--version") => {
            if has_extra {
                return Err("--version does not accept arguments".into());
            }
            println!("{}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Some("serve") => serve(&rest),
        Some("hold") => hold(&rest),
        Some("relay") => relay(&rest),
        // The trampoline that starts a role detached, for the role's starter.
        #[cfg(windows)]
        Some("launch") => server::daemon::launch(&rest),
        Some(command) => manage(command, &rest).unwrap_or_else(|| Err(USAGE.into())),
        None => Err(USAGE.into()),
    }
}

#[cfg(any(target_os = "linux", windows))]
fn serve(args: &[String]) -> Result<(), String> {
    server::core::run(args)
}

#[cfg(any(target_os = "linux", windows))]
fn hold(args: &[String]) -> Result<(), String> {
    server::holder::run(args)
}

#[cfg(target_os = "linux")]
fn relay(args: &[String]) -> Result<(), String> {
    server::relay::run(args)
}

/// Remote hosts are Linux: Brindle on Windows runs a Linux host's relay.
#[cfg(not(target_os = "linux"))]
fn relay(_args: &[String]) -> Result<(), String> {
    Err("relay runs only on Linux".into())
}

/// `check`, `list`, `kill`, `status` and `stop`; `None` for anything else.
#[cfg(any(target_os = "linux", windows))]
fn manage(command: &str, args: &[String]) -> Option<Result<(), String>> {
    server::cli::run(command, args)
}

#[cfg(not(any(target_os = "linux", windows)))]
fn serve(_args: &[String]) -> Result<(), String> {
    Err("the terminal server runs only on Linux and Windows".into())
}

#[cfg(not(any(target_os = "linux", windows)))]
fn hold(_args: &[String]) -> Result<(), String> {
    Err("the terminal server runs only on Linux and Windows".into())
}

#[cfg(not(any(target_os = "linux", windows)))]
fn manage(command: &str, _args: &[String]) -> Option<Result<(), String>> {
    matches!(command, "check" | "list" | "kill" | "status" | "stop")
        .then(|| Err("the terminal server runs only on Linux and Windows".into()))
}
