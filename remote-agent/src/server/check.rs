//! `brindle-remote check --holder-protocols LIST`, an upgrading client's
//! first step: whether this binary's core speaks every H1 version in LIST
//! (comma- or space-separated), the versions of the running holders it
//! would take over. The client sends `STEP_DOWN` to the running core only
//! if it passes. A refusal is also written to the server directory's
//! `core.log`, beside the running core's own entries. A relay makes the same
//! check in-process ([`refusal`]).

use std::io::Write as _;

use brindle_remote_protocol::holder as h1;
use brindle_remote_protocol::version::VersionRange;

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
/// `wanted` ([`refusal_reason`]), also written to `core.log` as `who`'s;
/// `None` when it can.
pub fn refusal(who: &str, wanted: &[u32]) -> Option<String> {
    // The build (a hash of the whole binary) is read only for a refusal.
    if missing(wanted, h1::PROTOCOLS).is_empty() {
        return None;
    }
    let build = build_id::current().unwrap_or_else(|_| "unknown".into());
    let reason = refusal_reason(wanted, h1::PROTOCOLS, env!("CARGO_PKG_VERSION"), &build)?;
    log_refusal(who, &reason);
    Some(reason)
}

/// The versions in `wanted` that `supported` lacks, in `wanted`'s order.
fn missing(wanted: &[u32], supported: VersionRange) -> Vec<u32> {
    wanted
        .iter()
        .copied()
        .filter(|&version| !supported.contains(version))
        .collect()
}

/// Why a brindle-remote of `version` and `build`, whose core drives the H1
/// versions `supported`, can't take over holders that speak `wanted`:
/// `None` when it drives every one of them.
pub fn refusal_reason(
    wanted: &[u32],
    supported: VersionRange,
    version: &str,
    build: &str,
) -> Option<String> {
    let missing = missing(wanted, supported);
    if missing.is_empty() {
        return None;
    }
    let missing = missing
        .iter()
        .map(|version| format!("H{version}"))
        .collect::<Vec<_>>()
        .join(", ");
    Some(format!(
        "this brindle-remote (version {version}, build {build}) speaks holder protocol \
         H{supported}, not {missing}, which the running holders use"
    ))
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

    #[test]
    fn a_refusal_names_the_versions_it_lacks() {
        let supported = VersionRange::new(1, 2);
        let reason = |wanted: &[u32]| refusal_reason(wanted, supported, "0.9.0", "abc");
        assert_eq!(reason(&[]), None);
        assert_eq!(reason(&[2, 1]), None);
        assert_eq!(
            reason(&[3]).as_deref(),
            Some(
                "this brindle-remote (version 0.9.0, build abc) speaks holder protocol H1–2, \
                 not H3, which the running holders use"
            )
        );
        // Each one it lacks, in the order asked.
        assert_eq!(
            reason(&[5, 1, 0]).as_deref(),
            Some(
                "this brindle-remote (version 0.9.0, build abc) speaks holder protocol H1–2, \
                 not H5, H0, which the running holders use"
            )
        );
        let single = refusal_reason(&[2], VersionRange::new(1, 1), "1.0.0", "b1");
        assert_eq!(
            single.as_deref(),
            Some(
                "this brindle-remote (version 1.0.0, build b1) speaks holder protocol H1, \
                 not H2, which the running holders use"
            )
        );
    }
}
