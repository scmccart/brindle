//! How the process's remote endpoints stand (design.md decision 11): the
//! [`Hosts`] GPUI global, which the sessions palette shows.
//!
//! It holds each remote endpoint the process has a connection to, and each
//! eager one (a profile connects to it at launch), whether connected or not:
//! connecting, connected, or offline with its last error. Each relay
//! connection reports its own changes ([`LinkState`]) from its threads, and
//! the UI thread applies them in the order they come ([`Hosts::apply`]),
//! so nothing polls the connections. Reports carry the connection's id: a
//! connection that replaced an ended one counts, and the ended one's late
//! reports don't.

use crate::remote::Host;

/// A relay connection's state, as it reports it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkState {
    /// Its first link is starting.
    Connecting,
    /// A link is up.
    Connected,
    /// No link is up, and why: an attempt failed, or the link was lost. It
    /// stays offline through later attempts until a link is up.
    Offline(String),
    /// The connection ended for good, and why: nothing used it (its unwritten
    /// `CLOSE`s given up, if any), or the endpoint's server refused this
    /// Brindle and no profile connects to it at launch. Its last report.
    Ended(String),
}

/// What the sessions palette says of an endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostState {
    Connecting,
    Connected,
    /// It can't be reached, and why.
    Offline(String),
}

#[derive(Debug)]
struct Entry {
    host: Host,
    state: HostState,
    /// The connection whose report this is; `None` before any report, and
    /// once that connection ended (an eager endpoint's entry stays).
    conn: Option<u64>,
    /// The latest connection that reported, ended or not: an older one's
    /// reports are stale.
    newest: u64,
    /// A profile connects to it at launch.
    eager: bool,
}

/// The remote endpoints' states, in the order they were first known.
#[derive(Debug, Default)]
pub struct Hosts {
    entries: Vec<Entry>,
}

impl gpui::Global for Hosts {}

impl Hosts {
    /// Makes `eager` the endpoints that profiles connect to at launch. One
    /// not known yet is listed as connecting, before its connection
    /// reports; one no longer eager whose connection has ended is
    /// forgotten (one still connected stays until its connection ends).
    /// Says whether anything changed.
    pub fn want(&mut self, eager: &[Host]) -> bool {
        let mut changed = false;
        for entry in &mut self.entries {
            let wanted = eager.contains(&entry.host);
            changed |= entry.eager != wanted;
            entry.eager = wanted;
        }
        let before = self.entries.len();
        self.entries.retain(|entry| entry.eager || entry.conn.is_some());
        changed |= self.entries.len() != before;
        for host in eager.iter().filter(|host| !host.is_local()) {
            if !self.entries.iter().any(|entry| entry.host == *host) {
                self.entries.push(Entry { host: host.clone(), state: HostState::Connecting, conn: None, newest: 0, eager: true });
                changed = true;
            }
        }
        changed
    }

    /// Applies connection `conn`'s report on `host`. A connection that
    /// ended takes a host no profile keeps out, and leaves an eager one
    /// offline with why. Says whether what the palette shows changed.
    pub fn apply(&mut self, host: &Host, conn: u64, state: LinkState) -> bool {
        let Some(ix) = self.entries.iter().position(|entry| entry.host == *host) else {
            let state = match state {
                LinkState::Ended(_) => return false,
                LinkState::Connecting => HostState::Connecting,
                LinkState::Connected => HostState::Connected,
                LinkState::Offline(why) => HostState::Offline(why),
            };
            self.entries.push(Entry { host: host.clone(), state, conn: Some(conn), newest: conn, eager: false });
            return true;
        };
        let entry = &mut self.entries[ix];
        if conn < entry.newest {
            return false;
        }
        entry.newest = conn;
        let state = match state {
            LinkState::Connecting => HostState::Connecting,
            LinkState::Connected => HostState::Connected,
            LinkState::Offline(why) => HostState::Offline(why),
            LinkState::Ended(_) if !entry.eager => {
                self.entries.remove(ix);
                return true;
            }
            LinkState::Ended(why) => {
                entry.conn = None;
                let offline = HostState::Offline(why);
                return std::mem::replace(&mut entry.state, offline.clone()) != offline;
            }
        };
        entry.conn = Some(conn);
        std::mem::replace(&mut entry.state, state.clone()) != state
    }

    /// Applies a batch of reports ([`Hosts::apply`]), each `(host, conn,
    /// state)`, in order. Says whether any changed what the palette shows.
    pub fn apply_all(&mut self, batch: impl IntoIterator<Item = (Host, u64, LinkState)>) -> bool {
        // `|`, not `||`: every report applies, whatever the earlier ones did.
        batch.into_iter().fold(false, |changed, (host, conn, state)| self.apply(&host, conn, state) | changed)
    }

    /// Each endpoint and its state, in the order they were first known.
    pub fn states(&self) -> Vec<(Host, HostState)> {
        self.entries.iter().map(|entry| (entry.host.clone(), entry.state.clone())).collect()
    }

    #[cfg(test)]
    pub fn state(&self, host: &Host) -> Option<&HostState> {
        self.entries.iter().find(|entry| entry.host == *host).map(|entry| &entry.state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::RemoteTarget;

    fn ssh(host: &str) -> Host {
        Host::Remote(RemoteTarget::Ssh {
            host: host.into(),
            user: None,
            port: None,
            identity_file: None,
            ssh_args: Vec::new(),
            connect_at_launch: false,
        })
    }

    fn offline(why: &str) -> HostState {
        HostState::Offline(why.into())
    }

    /// A connection's reports, in turn: connecting, connected, offline with
    /// its error (each new error replacing the last), connected again. One
    /// that says nothing new changes nothing.
    #[test]
    fn a_hosts_state_follows_its_connections_reports() {
        let (mut hosts, a8) = (Hosts::default(), ssh("A8"));
        assert!(hosts.apply(&a8, 1, LinkState::Connecting));
        assert_eq!(hosts.state(&a8), Some(&HostState::Connecting));
        assert!(hosts.apply(&a8, 1, LinkState::Connected));
        assert!(!hosts.apply(&a8, 1, LinkState::Connected));
        assert!(hosts.apply(&a8, 1, LinkState::Offline("connection lost".into())));
        assert!(hosts.apply(&a8, 1, LinkState::Offline("ssh: Connection refused".into())));
        assert!(!hosts.apply(&a8, 1, LinkState::Offline("ssh: Connection refused".into())));
        assert_eq!(hosts.state(&a8), Some(&offline("ssh: Connection refused")));
        assert!(hosts.apply(&a8, 1, LinkState::Connected));
        assert_eq!(hosts.states(), [(a8, HostState::Connected)]);
    }

    /// A connection that ends takes its host out, unless a profile connects
    /// to it at launch: then it stays, offline with why. A replaced
    /// connection's late reports change nothing.
    #[test]
    fn an_ended_connection_leaves_only_an_eager_host() {
        let (mut hosts, a8, b9) = (Hosts::default(), ssh("A8"), ssh("B9"));
        assert!(hosts.want(std::slice::from_ref(&b9)));
        assert_eq!(hosts.states(), [(b9.clone(), HostState::Connecting)], "listed before it reports");
        hosts.apply(&a8, 1, LinkState::Connected);
        hosts.apply(&b9, 2, LinkState::Connected);
        assert!(hosts.apply(&a8, 1, LinkState::Ended("the connection was no longer used".into())));
        assert_eq!(hosts.state(&a8), None);
        assert!(!hosts.apply(&a8, 1, LinkState::Ended("again".into())), "an unknown host's end");
        assert!(hosts.apply(&b9, 2, LinkState::Ended("refused".into())));
        assert_eq!(hosts.state(&b9), Some(&offline("refused")));

        // A new connection to B9 replaces the ended one; the old one is
        // stale even when it reports late.
        assert!(hosts.apply(&b9, 5, LinkState::Connecting));
        assert!(!hosts.apply(&b9, 2, LinkState::Connected));
        assert!(!hosts.apply(&b9, 2, LinkState::Ended("late".into())));
        assert_eq!(hosts.state(&b9), Some(&HostState::Connecting));
        hosts.apply(&a8, 4, LinkState::Connecting);
        hosts.apply(&a8, 3, LinkState::Ended("the old one".into()));
        assert_eq!(hosts.state(&a8), Some(&HostState::Connecting), "a replaced connection's end");
    }

    /// 3.4: a batch of reports applies in order, every one of them, and
    /// says whether any changed what the palette shows.
    #[test]
    fn a_batch_applies_in_order_and_says_whether_anything_changed() {
        let (mut hosts, a8, b9) = (Hosts::default(), ssh("A8"), ssh("B9"));
        let batch = vec![
            (a8.clone(), 1, LinkState::Connecting),
            (a8.clone(), 1, LinkState::Connected),
            (b9.clone(), 2, LinkState::Offline("ssh: Connection refused".into())),
        ];
        assert!(hosts.apply_all(batch));
        assert_eq!(hosts.states(), [(a8.clone(), HostState::Connected), (b9.clone(), offline("ssh: Connection refused"))]);
        // Nothing new: no change.
        assert!(!hosts.apply_all(vec![(a8.clone(), 1, LinkState::Connected)]));
        assert!(!hosts.apply_all(Vec::new()));
        // A change after reports that change nothing still counts, and the
        // last report wins.
        let batch = vec![
            (a8.clone(), 1, LinkState::Connected),
            (b9.clone(), 2, LinkState::Connected),
            (b9.clone(), 2, LinkState::Offline("connection lost: ssh exited with 255".into())),
        ];
        assert!(hosts.apply_all(batch));
        assert_eq!(hosts.state(&b9), Some(&offline("connection lost: ssh exited with 255")));
        // A change before reports that change nothing does too.
        let batch = vec![(a8.clone(), 1, LinkState::Ended("the connection was no longer used".into())), (b9, 2, LinkState::Offline("connection lost: ssh exited with 255".into()))];
        assert!(hosts.apply_all(batch));
        assert_eq!(hosts.state(&a8), None);
    }

    /// A reload that turns connecting at launch off forgets an eager host
    /// whose connection is gone, and leaves one still connected until its
    /// connection ends. Turning it on lists a host at once.
    #[test]
    fn want_follows_the_profiles_that_connect_at_launch() {
        let (mut hosts, a8, b9) = (Hosts::default(), ssh("A8"), ssh("B9"));
        assert!(hosts.want(&[a8.clone(), b9.clone(), Host::Local]));
        assert!(!hosts.want(&[a8.clone(), b9.clone()]));
        assert_eq!(hosts.states().len(), 2, "this machine isn't a remote endpoint");
        hosts.apply(&a8, 1, LinkState::Connected);
        hosts.apply(&b9, 2, LinkState::Ended("refused".into()));
        assert!(hosts.want(&[]));
        assert_eq!(hosts.states(), [(a8.clone(), HostState::Connected)]);
        assert!(hosts.apply(&a8, 1, LinkState::Ended("the connection was no longer used".into())));
        assert!(hosts.states().is_empty());
        assert!(hosts.want(std::slice::from_ref(&a8)));
        assert_eq!(hosts.states(), [(a8, HostState::Connecting)]);
    }

    /// A connection that couldn't be made is reported offline under an id
    /// of its own, taken after the earlier connection's: that one's late
    /// reports don't hide it, and a connection made later replaces it.
    #[test]
    fn hosts_orders_an_offline_report_after_an_earlier_connection() {
        let (mut hosts, a8) = (Hosts::default(), ssh("A8"));
        hosts.want(std::slice::from_ref(&a8));
        hosts.apply(&a8, 1, LinkState::Connecting);
        let why = "connecting to A8: can't start a thread: out of memory";
        assert!(hosts.apply(&a8, 2, LinkState::Offline(why.into())));
        assert!(!hosts.apply(&a8, 1, LinkState::Connected), "the earlier connection's late report");
        assert_eq!(hosts.state(&a8), Some(&offline(why)));
        assert!(hosts.apply(&a8, 3, LinkState::Connected));
        assert_eq!(hosts.state(&a8), Some(&HostState::Connected));
    }
}
