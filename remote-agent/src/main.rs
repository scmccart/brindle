mod agent;
#[cfg(target_os = "linux")]
mod pty;
#[cfg(target_os = "linux")]
mod server;

use std::env;
use std::process::ExitCode;

const USAGE: &str =
    "usage: brindle-remote --version | --server | serve [--ready-fd N] | hold [--ready-fd N]";

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
        _ => Err(USAGE.into()),
    }
}

#[cfg(target_os = "linux")]
fn serve(args: &[String]) -> Result<(), String> {
    server::core::run(args)
}

#[cfg(target_os = "linux")]
fn hold(args: &[String]) -> Result<(), String> {
    server::holder::run(args)
}

#[cfg(not(target_os = "linux"))]
fn serve(_args: &[String]) -> Result<(), String> {
    Err("the terminal server runs only on Linux".into())
}

#[cfg(not(target_os = "linux"))]
fn hold(_args: &[String]) -> Result<(), String> {
    Err("the terminal server runs only on Linux".into())
}
