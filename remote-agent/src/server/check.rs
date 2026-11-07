//! `brindle-remote check --holder-protocols LIST`, an upgrading client's
//! first step: whether this binary's core speaks every H1 version in LIST
//! (comma- or space-separated), the versions of the running holders it
//! would take over. The client sends `STEP_DOWN` to the running core only
//! if it passes. A refusal is also written to the server directory's
//! `core.log`, beside the running core's own entries.

use std::io::Write as _;

use brindle_remote_protocol::holder as h1;

use super::{build_id, daemon, dir};

pub fn run(args: &[String]) -> Result<(), String> {
    let list = match args {
        [flag, list] if flag == "--holder-protocols" => list,
        _ => return Err("usage: brindle-remote check --holder-protocols LIST".into()),
    };
    let wanted = parse_protocols(list)?;
    let missing: Vec<u32> = wanted
        .iter()
        .copied()
        .filter(|&version| !h1::PROTOCOLS.contains(version))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    let build = build_id::current().unwrap_or_else(|_| "unknown".into());
    let missing = missing
        .iter()
        .map(|version| format!("H{version}"))
        .collect::<Vec<_>>()
        .join(", ");
    let reason = format!(
        "this brindle-remote (version {}, build {build}) speaks holder protocol H{}, \
         not {missing}, which the running holders use",
        env!("CARGO_PKG_VERSION"),
        h1::PROTOCOLS
    );
    log_refusal(&reason);
    Err(format!("upgrade refused: {reason}"))
}

fn parse_protocols(list: &str) -> Result<Vec<u32>, String> {
    list.split([',', ' '])
        .filter(|item| !item.is_empty())
        .map(|item| {
            item.trim_start_matches(['H', 'h'])
                .parse()
                .map_err(|_| format!("{item:?} isn't a holder protocol version"))
        })
        .collect()
}

/// Appends a refused upgrade to `core.log`, if the server directory exists.
fn log_refusal(reason: &str) {
    let Ok(dir) = dir::resolve_from_env() else {
        return;
    };
    if !dir.is_dir() {
        return;
    }
    if let Ok(mut log) = daemon::open_log(&dir.join("core.log")) {
        let _ = writeln!(
            log,
            "{} check[{}]: upgrade refused: {reason}",
            daemon::timestamp(),
            std::process::id()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holder_protocol_lists() {
        assert_eq!(parse_protocols("1"), Ok(vec![1]));
        assert_eq!(parse_protocols("1,2 H3"), Ok(vec![1, 2, 3]));
        assert_eq!(parse_protocols(""), Ok(vec![]));
        assert!(parse_protocols("one").is_err());
    }

    #[test]
    fn this_binary_drives_its_own_holder_protocols() {
        assert_eq!(run(&["--holder-protocols".into(), "1".into()]), Ok(()));
        assert!(run(&[]).is_err());
    }
}
