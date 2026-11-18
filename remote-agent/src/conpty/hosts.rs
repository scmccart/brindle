//! Which console host is a session's, and which processes a hang-up ends.
//!
//! Windows has no process group to signal: the holder ends what is left on
//! the session's console host (`conpty::console_host`). Pids are reused,
//! so a host is the session's only when it started before the session's
//! root process, and a process is ended only when it is on that host and
//! started no earlier than it.
//!
//! Pure, so it is tested on every platform. The Windows `pty` takes the
//! snapshot, opens the processes, reads their hosts and start times, and
//! terminates them.

/// A process in a snapshot of the running ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Listed {
    pub pid: u32,
    /// The process that created it.
    pub parent: u32,
}

/// The session's console host, as it was found: its pid, and when it
/// started (in 100 ns units since 1601).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostSeen {
    pub pid: u32,
    pub started: u64,
}

/// Whether the host the root process names is the session's: it still
/// runs, both start times are known, and it started before the root. A
/// process that took the host's pid later started after the root had
/// connected to it.
pub fn is_session_host(exited: bool, host_started: Option<u64>, root_started: Option<u64>) -> bool {
    match (exited, host_started, root_started) {
        (false, Some(host), Some(root)) => host < root,
        _ => false,
    }
}

/// Whether `host` is a console host this process (`own`) created. A root
/// that moved to another console names that one's.
pub fn is_ours(snapshot: &[Listed], host: u32, own: u32) -> bool {
    snapshot
        .iter()
        .any(|entry| entry.pid == host && entry.parent == own)
}

/// The processes that may be on the host: every one in the snapshot but
/// this process and the host itself, in the snapshot's order.
pub fn candidates(snapshot: &[Listed], host: u32, own: u32) -> Vec<u32> {
    snapshot
        .iter()
        .map(|entry| entry.pid)
        .filter(|&pid| pid != own && pid != host)
        .collect()
}

/// Whether a candidate that is `on` a console host and `started` then is
/// ended with the session: it is on `host`, and started no earlier than it.
/// A process that was on an earlier host with this pid started before it.
pub fn ends(on: Option<u32>, started: Option<u64>, host: &HostSeen) -> bool {
    on == Some(host.pid) && started.is_some_and(|started| started >= host.started)
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWN: u32 = 100;
    const HOST: u32 = 200;

    fn listed(pid: u32, parent: u32) -> Listed {
        Listed { pid, parent }
    }

    #[test]
    fn a_host_that_started_after_the_root_is_not_the_sessions() {
        assert!(is_session_host(false, Some(10), Some(20)));
        // It took a pid the session's host had, after the root connected.
        assert!(!is_session_host(false, Some(20), Some(10)));
        assert!(!is_session_host(false, Some(20), Some(20)));
    }

    #[test]
    fn an_exited_host_or_an_unknown_start_is_not_the_sessions() {
        assert!(!is_session_host(true, Some(10), Some(20)));
        assert!(!is_session_host(false, None, Some(20)));
        assert!(!is_session_host(false, Some(10), None));
        assert!(!is_session_host(false, None, None));
    }

    #[test]
    fn a_host_is_ours_only_when_this_process_is_its_parent() {
        let snapshot = [listed(OWN, 1), listed(HOST, OWN), listed(300, HOST)];
        assert!(is_ours(&snapshot, HOST, OWN));
        // A root that moved to another console names that one's host.
        let theirs = [listed(OWN, 1), listed(HOST, 50), listed(300, HOST)];
        assert!(!is_ours(&theirs, HOST, OWN));
        // A host that is gone from the snapshot.
        assert!(!is_ours(&[listed(OWN, 1)], HOST, OWN));
        assert!(!is_ours(&[], HOST, OWN));
    }

    #[test]
    fn candidates_leave_out_this_process_and_the_host() {
        let snapshot = [
            listed(4, 0),
            listed(OWN, 1),
            listed(HOST, OWN),
            listed(300, HOST),
            listed(301, 300),
        ];
        assert_eq!(candidates(&snapshot, HOST, OWN), [4, 300, 301]);
        assert_eq!(candidates(&[], HOST, OWN), Vec::<u32>::new());
    }

    #[test]
    fn a_candidate_ends_only_on_the_host_and_started_after_it() {
        let host = HostSeen {
            pid: HOST,
            started: 50,
        };
        assert!(ends(Some(HOST), Some(60), &host));
        assert!(ends(Some(HOST), Some(50), &host));
        // On an earlier host that had this pid.
        assert!(!ends(Some(HOST), Some(40), &host));
        // On another console, on none, or of an unknown start.
        assert!(!ends(Some(201), Some(60), &host));
        assert!(!ends(None, Some(60), &host));
        assert!(!ends(Some(HOST), None, &host));
    }
}
