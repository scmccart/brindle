//! `brindle-remote check --holder-protocols LIST`, an upgrading client's
//! first step: whether this binary's core speaks every H1 version in LIST
//! (comma- or space-separated), the versions of the running holders it
//! would take over. The client sends `STEP_DOWN` to the running core only
//! if it passes. A refusal is also written to the server directory's
//! `core.log`, beside the running core's own entries. A relay makes the same
//! check in-process ([`refusal`]).

use std::io::Write as _;

use brindle_remote_protocol::holder as h1;

use super::{build_id, daemon, dir};

pub fn run(args: &[String]) -> Result<(), String> {
    let list = match args {
        [flag, list] if flag == "--holder-protocols" => list,
        _ => return Err("usage: brindle-remote check --holder-protocols LIST".into()),
    };
    let wanted = parse_protocols(list)?;
    match refusal("check", &wanted) {
        None => Ok(()),
        Some(reason) => Err(format!("upgrade refused: {reason}")),
    }
}

/// Why this binary can't take over holders that speak the H1 versions
/// `wanted`, also written to `core.log` as `who`'s; `None` when it can.
pub fn refusal(who: &str, wanted: &[u32]) -> Option<String> {
    let missing: Vec<u32> = wanted
        .iter()
        .copied()
        .filter(|&version| !h1::PROTOCOLS.contains(version))
        .collect();
    if missing.is_empty() {
        return None;
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
    log_refusal(who, &reason);
    Some(reason)
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
fn log_refusal(who: &str, reason: &str) {
    let Ok(dir) = dir::resolve_from_env() else {
        return;
    };
    if !dir.is_dir() {
        return;
    }
    if let Ok(mut log) = daemon::open_log(&dir.join("core.log")) {
        let _ = writeln!(
            log,
            "{} {who}[{}]: upgrade refused: {reason}",
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
        let own: Vec<u32> = (h1::PROTOCOLS.min..=h1::PROTOCOLS.max).collect();
        assert_eq!(refusal("test", &own), None);
        assert_eq!(refusal("test", &[]), None);
    }
}
