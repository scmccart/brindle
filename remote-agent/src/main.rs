#[path = "../../src/remote/agent.rs"]
mod agent;
#[path = "../../src/remote/protocol.rs"]
#[allow(dead_code)]
mod protocol;

use std::env;
use std::process::ExitCode;

use protocol::{SessionKind, SpawnRequest};

const USAGE: &str = "usage: brindle-remote --version\n       brindle-remote pty --request-json <SpawnRequest JSON>\n       brindle-remote tmux --request-json <SpawnRequest JSON>\n       brindle-remote --server";

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
    let Some(mode) = args.next() else {
        return Err(usage().into());
    };

    match mode.as_str() {
        "--version" => {
            if args.next().is_some() {
                return Err("--version does not accept arguments".into());
            }
            println!("{}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        "--help" | "-h" => {
            if args.next().is_some() {
                return Err("--help does not accept arguments".into());
            }
            println!("{}", usage());
            Ok(())
        }
        "--server" | "server" => {
            if args.next().is_some() {
                return Err("--server does not accept arguments".into());
            }
            agent::serve_server_stdio().map_err(|error| error.to_string())
        }
        "pty" => run_direct_mode(SessionKind::Pty, args),
        "tmux" => run_direct_mode(SessionKind::Tmux, args),
        other => Err(format!("unknown mode {other:?}\n{}", usage())),
    }
}

fn run_direct_mode(
    expected_kind: SessionKind,
    mut args: impl Iterator<Item = String>,
) -> Result<(), String> {
    let Some(argument) = args.next() else {
        return Err(format!(
            "{} mode requires --request-json <json>",
            mode_name(expected_kind)
        ));
    };
    let json = if argument == "--request-json" {
        args.next()
            .ok_or_else(|| "--request-json requires a JSON value".to_owned())?
    } else if let Some(json) = argument.strip_prefix("--request-json=") {
        json.to_owned()
    } else {
        return Err(format!(
            "unexpected argument {argument:?}; expected --request-json"
        ));
    };
    if args.next().is_some() {
        return Err("unexpected arguments after the spawn request".into());
    }

    let request: SpawnRequest = serde_json::from_str(&json)
        .map_err(|error| format!("invalid SpawnRequest JSON: {error}"))?;
    if request.kind != expected_kind {
        return Err(format!(
            "{} mode requires request kind {:?}, received {:?}",
            mode_name(expected_kind),
            expected_kind,
            request.kind
        ));
    }
    agent::serve_stdio(request).map_err(|error| error.to_string())
}

fn mode_name(kind: SessionKind) -> &'static str {
    match kind {
        SessionKind::Pty => "pty",
        SessionKind::Tmux => "tmux",
    }
}

fn usage() -> &'static str {
    USAGE
}
