mod agent;
mod conpty;
#[cfg(any(target_os = "linux", windows))]
mod pty;
#[cfg(any(target_os = "linux", windows))]
mod server;

use std::env;
use std::process::ExitCode;

#[cfg(any(target_os = "linux", windows))]
const USAGE: &str = server::cli::USAGE;
#[cfg(not(any(target_os = "linux", windows)))]
const USAGE: &str = "usage: brindle-remote --version | --server";

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
    if env::var_os("BRINDLE_REMOTE_UNLINK").is_some()
        && let Ok(exe) = env::current_exe()
    {
        std::fs::remove_file(exe).ok();
    }
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
        Some("--server") => {
            if has_extra {
                return Err("--server does not accept arguments".into());
            }
            agent::serve_stdio().map_err(|error| error.to_string())
        }
        Some("serve") => serve(&rest),
        Some("hold") => hold(&rest),
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

/// `check`, `list`, `kill`, `status` and `stop`; `None` for anything else.
#[cfg(any(target_os = "linux", windows))]
fn manage(command: &str, args: &[String]) -> Option<Result<(), String>> {
    server::cli::run(command, args)
}

#[cfg(not(any(target_os = "linux", windows)))]
fn serve(_args: &[String]) -> Result<(), String> {
    Err("the terminal server runs only on Linux".into())
}

#[cfg(not(any(target_os = "linux", windows)))]
fn hold(_args: &[String]) -> Result<(), String> {
    Err("the terminal server runs only on Linux".into())
}

#[cfg(not(any(target_os = "linux", windows)))]
fn manage(command: &str, _args: &[String]) -> Option<Result<(), String>> {
    matches!(command, "check" | "list" | "kill" | "status" | "stop")
        .then(|| Err("the terminal server runs only on Linux".into()))
}
