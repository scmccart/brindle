mod agent;

use std::env;
use std::process::ExitCode;

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
    let has_extra = args.next().is_some();

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
        _ => Err(USAGE.into()),
    }
}
