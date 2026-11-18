//! The detached terminal server sessions a launch attaches (Linux and
//! Windows): this machine's, listed once before any window opens, off the
//! UI thread ([`Listing`]), then taken by the restored zones whose recorded
//! session they are, and the rest opened as tabs of the first window,
//! oldest first. Each remote endpoint
//! the launch connects to does the same once it first answers, with the
//! tabs at the end ([`sweep`], design.md decision 10). Either way a session
//! whose owner may still come back is left alone ([`sweepable`], decision
//! 13).

use crate::config::{Config, Profile};
use crate::layout::LayoutTab;
use crate::remote::Host;
use crate::remote::protocol::client::SessionInfo;
use crate::session::ServerSession;

/// How long a session's owner may still come back for it: until then a
/// launch doesn't attach it unless a restored zone recorded it.
pub const OWNER_GRACE_SECS: u64 = 5 * 60;

/// Whether a launch attaches `info` as a tab of its own: detached, and with
/// no owner, or one gone for more than [`OWNER_GRACE_SECS`]. The rest stay
/// in the sessions palette.
pub fn sweepable(info: &SessionInfo) -> bool {
    !info.attached && (!info.owned || info.detached_secs > OWNER_GRACE_SECS)
}

/// The profile a session of this machine's server opens with: the one it
/// was started with, unless that is gone or a remote one; then the default.
pub fn local_profile(info: &SessionInfo, config: &Config) -> usize {
    info.profile
        .as_deref()
        .and_then(|name| config.profile_index(name))
        .filter(|&index| config.profiles[index].remote.is_none())
        .unwrap_or_else(|| config.default_profile_index())
}

/// The profile a session of `host`'s server opens with: the one it was
/// started with when that targets `host`, else the first profile that
/// does. `None` when none does.
pub fn remote_profile(info: &SessionInfo, host: &Host, config: &Config) -> Option<usize> {
    let on_host = |index: &usize| Host::of(&config.profiles[*index]) == *host;
    info.profile
        .as_deref()
        .and_then(|name| config.profile_index(name))
        .filter(on_host)
        .or_else(|| (0..config.profiles.len()).find(on_host))
}

/// The profile a detached session of `host`'s server opens a tab with, the
/// one rule for a launch's sweep and the sessions palette (design.md
/// decision 7). This machine's: [`local_profile`]. A remote endpoint's, the
/// first of: the profile it was started with, when that targets `host`; the
/// first profile that targets `host`; `first_tab_profile` (the name of the
/// profile of the first tab on `host`), when that profile exists; the
/// default profile. The tab runs on `host` whatever the profile targets.
pub fn session_profile(info: &SessionInfo, host: &Host, config: &Config, first_tab_profile: Option<&str>) -> usize {
    match host {
        Host::Local => local_profile(info, config),
        Host::Remote(_) => remote_profile(info, host, config)
            .or_else(|| first_tab_profile.and_then(|name| config.profile_index(name)))
            .unwrap_or_else(|| config.default_profile_index()),
    }
}

/// [`sweep`] grouped into tabs ([`crate::layout::group`]) and joined with
/// [`session_profile`]: the tabs `host`'s first listing opens at a launch,
/// each with the profile index its active session gives it.
pub fn swept_tabs(
    listed: Vec<SessionInfo>,
    skip: &[ServerSession],
    host: &Host,
    config: &Config,
    first_tab_profile: Option<&str>,
) -> Vec<(LayoutTab, usize)> {
    crate::layout::group(sweep(listed, skip))
        .into_iter()
        .map(|tab| {
            let index = session_profile(tab.active(), host, config, first_tab_profile);
            (tab, index)
        })
        .collect()
}

/// What a remote endpoint's first listing of the launch attaches as tabs:
/// the [`sweepable`] sessions not in `skip` (those restored into a zone, or
/// shown already), in start order.
pub fn sweep(listed: Vec<SessionInfo>, skip: &[ServerSession]) -> Vec<SessionInfo> {
    let mut sessions: Vec<SessionInfo> = listed
        .into_iter()
        .filter(sweepable)
        .filter(|info| !skip.iter().any(|s| s.id == info.session && s.key == info.key))
        .collect();
    sessions.sort_by_key(|s| (s.started, s.session));
    sessions
}

/// Whether `profile` connects at launch: its remote target's
/// `connect_at_launch`.
pub fn connects_at_launch(profile: &Profile) -> bool {
    profile.remote.as_ref().is_some_and(crate::config::RemoteTarget::connects_at_launch)
}

/// The endpoints of the profiles that connect at launch, each once, in
/// profile order: what the process keeps connected while it runs, launches
/// with `--no-restore` included. One endpoint is eager when any of its
/// profiles is.
pub fn eager_hosts(config: &Config) -> Vec<Host> {
    let mut hosts: Vec<Host> = Vec::new();
    for host in config.profiles.iter().filter(|p| connects_at_launch(p)).map(Host::of) {
        if !hosts.contains(&host) {
            hosts.push(host);
        }
    }
    hosts
}

/// The remote endpoints whose first listing a restoring launch sweeps: those
/// of its restored remote zones (`restored`), then those of the profiles
/// that connect at launch ([`eager_hosts`]), each once.
pub fn launch_hosts(restored: &[Host], config: &Config) -> Vec<Host> {
    let mut hosts: Vec<Host> = Vec::new();
    for host in restored.iter().cloned().chain(eager_hosts(config)) {
        if !host.is_local() && !hosts.contains(&host) {
            hosts.push(host);
        }
    }
    hosts
}

/// The remote endpoints that a window's tabs were on (`before`) and no
/// longer are (`after`), except the eager ones (`eager`, [`eager_hosts`]),
/// each once in `before`'s order. Once the last tab on such an endpoint
/// goes, the launch stops waiting for its first listing (design.md decision
/// 4), so the launch doesn't sweep it. This machine is never one.
pub fn endpoints_without_tabs(before: &[Host], after: &[Host], eager: &[Host]) -> Vec<Host> {
    let mut left: Vec<Host> = Vec::new();
    for host in before {
        if !host.is_local() && !after.contains(host) && !eager.contains(host) && !left.contains(host) {
            left.push(host.clone());
        }
    }
    left
}

/// A GPUI global: what is left to attach. Empty for `--no-restore`
/// launches, for a Brindle that doesn't own the session state, and where
/// no terminal server runs (macOS).
#[derive(Default)]
pub struct Detached {
    sessions: Vec<SessionInfo>,
}

impl gpui::Global for Detached {}

impl Detached {
    pub fn new(sessions: Vec<SessionInfo>) -> Self {
        Self { sessions }
    }

    /// The listed session a restored zone recorded: same id and same key
    /// (an empty key never matches, so a reused id is never taken for it).
    pub fn take(&mut self, recorded: &ServerSession) -> Option<SessionInfo> {
        let ix = self.position(recorded)?;
        Some(self.sessions.remove(ix))
    }

    /// Whether [`Detached::take`] would find `recorded`.
    pub fn has(&self, recorded: &ServerSession) -> bool {
        self.position(recorded).is_some()
    }

    fn position(&self, recorded: &ServerSession) -> Option<usize> {
        self.sessions.iter().position(|s| s.session == recorded.id && !s.key.is_empty() && s.key == recorded.key)
    }

    /// Everything not taken that a launch attaches ([`sweepable`]), in
    /// start order. A restored zone takes its own session whatever its
    /// owner, so the owner rule applies only here.
    pub fn take_rest(&mut self) -> Vec<SessionInfo> {
        let mut rest: Vec<SessionInfo> = std::mem::take(&mut self.sessions).into_iter().filter(sweepable).collect();
        rest.sort_by_key(|s| (s.started, s.session));
        rest
    }
}

/// What listing this machine's terminal server gave a launch, or a later
/// launch's `--attach` (design.md decision 3).
#[derive(Clone, Debug, PartialEq)]
pub enum Listing {
    /// Not asked: the launch neither restores nor attaches.
    Skipped,
    /// Every session, attached or not; empty when no server runs (none is
    /// started).
    Listed(Vec<SessionInfo>),
    /// The listing failed, for this cause (as `running_sessions` reports
    /// it): nothing tells a lost session from a live one.
    Failed(String),
}

impl Listing {
    /// The session `brindle --attach ID` opens, or the message its tab
    /// shows instead: [`attach_target`] over the listed sessions, else why
    /// there is no listing to look in.
    pub fn attach_target(&self, id: u32) -> Result<SessionInfo, String> {
        const UNLISTED: &str = "Brindle could not list the terminal server's sessions";
        match self {
            Listing::Listed(sessions) => attach_target(id, sessions),
            Listing::Failed(why) => Err(format!("{UNLISTED}: {why}")),
            Listing::Skipped => Err(UNLISTED.into()),
        }
    }
}

/// The session `brindle --attach ID` opens, from the launch's listing of
/// every session, or the message its tab shows instead.
pub fn attach_target(id: u32, sessions: &[SessionInfo]) -> Result<SessionInfo, String> {
    match sessions.iter().find(|s| s.session == id) {
        Some(s) if s.attached => Err(format!("session {id} is shown by another Brindle")),
        Some(s) => Ok(s.clone()),
        None => Err(format!("the terminal server has no session {id}")),
    }
}

/// What a session's tab shows until its title arrives, and in errors.
pub fn label(info: &SessionInfo) -> String {
    if !info.title.is_empty() {
        return info.title.clone();
    }
    let program = info.command.split_whitespace().next().unwrap_or_default();
    std::path::Path::new(program)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| format!("session {}", info.session))
}

/// How long ago `started` (unix seconds) was at `now`, shortly ("42s",
/// "5m", "3h", "2d"); `None` when the core didn't say.
pub fn age(started: u64, now: u64) -> Option<String> {
    if started == 0 {
        return None;
    }
    let secs = now.saturating_sub(started);
    Some(match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m", secs / 60),
        3600..86400 => format!("{}h", secs / 3600),
        _ => format!("{}d", secs / 86400),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ages_are_short() {
        assert_eq!(age(0, 100), None);
        assert_eq!(age(100, 142).as_deref(), Some("42s"));
        assert_eq!(age(100, 100 + 5 * 60 + 7).as_deref(), Some("5m"));
        assert_eq!(age(100, 100 + 3 * 3600).as_deref(), Some("3h"));
        assert_eq!(age(100, 100 + 2 * 86400 + 1).as_deref(), Some("2d"));
        // A clock that went back reads as just now.
        assert_eq!(age(200, 100).as_deref(), Some("0s"));
    }

    fn info(session: u32, key: &str, started: u64) -> SessionInfo {
        SessionInfo { session, key: key.into(), started, ..Default::default() }
    }

    #[test]
    fn a_zone_takes_only_its_own_session_and_the_rest_come_oldest_first() {
        let mut detached = Detached::new(vec![info(3, "c", 30), info(1, "a", 10), info(2, "", 5), info(4, "d", 10)]);
        // Same id, another key: a session that reused the id.
        assert!(detached.take(&ServerSession { id: 1, key: "old".into() }).is_none());
        // A core that predates keys: never matched.
        assert!(detached.take(&ServerSession { id: 2, key: String::new() }).is_none());
        assert_eq!(detached.take(&ServerSession { id: 3, key: "c".into() }).map(|s| s.session), Some(3));
        assert!(detached.take(&ServerSession { id: 3, key: "c".into() }).is_none(), "taken once");
        let rest: Vec<u32> = detached.take_rest().iter().map(|s| s.session).collect();
        assert_eq!(rest, [2, 1, 4]);
        assert!(detached.take_rest().is_empty());
    }

    fn owned(session: u32, key: &str, started: u64, detached_secs: u64) -> SessionInfo {
        SessionInfo { owned: true, detached_secs, ..info(session, key, started) }
    }

    /// 7.3: a session whose owner left it less than 5 minutes ago may be
    /// about to come back for it: the launch leaves it in the palette,
    /// though a restored zone still takes its own.
    #[test]
    fn the_rest_leaves_a_recent_owners_sessions_alone() {
        let mut detached = Detached::new(vec![
            owned(1, "a", 10, 60),
            owned(2, "b", 20, OWNER_GRACE_SECS + 1),
            info(3, "c", 5),
            owned(4, "d", 1, OWNER_GRACE_SECS),
            owned(5, "e", 2, 30),
        ]);
        assert_eq!(detached.take(&ServerSession { id: 5, key: "e".into() }).map(|s| s.session), Some(5));
        let rest: Vec<u32> = detached.take_rest().iter().map(|s| s.session).collect();
        assert_eq!(rest, [3, 2]);
    }

    /// 7.2: a remote endpoint's first listing attaches its detached
    /// sessions in start order, except those restored into zones (by id and
    /// key: a reused id is another session), those another client shows, and
    /// those a recent owner may come back for.
    #[test]
    fn a_sweep_takes_what_no_zone_restored_oldest_first_by_the_owner_rule() {
        let mut shown = info(6, "f", 1);
        shown.attached = true;
        let listed = vec![
            info(4, "d", 40),
            info(1, "a", 10),
            owned(2, "b", 20, 59),
            owned(3, "c", 30, 301),
            shown,
            info(5, "new", 5),
            info(7, "g", 10),
        ];
        let restored = [ServerSession { id: 1, key: "a".into() }, ServerSession { id: 5, key: "old".into() }];
        let swept: Vec<u32> = sweep(listed, &restored).iter().map(|s| s.session).collect();
        assert_eq!(swept, [5, 7, 3, 4]);
        assert!(sweep(Vec::new(), &restored).is_empty());
    }

    fn profiles() -> Config {
        Config::parse(
            "default_profile = \"Shell\"\n\
             [[profiles]]\nname = \"Shell\"\n\
             [[profiles]]\nname = \"htop on A8\"\ncommand = \"htop\"\nremote = { type = \"ssh\", host = \"A8\" }\n\
             [[profiles]]\nname = \"A8\"\nremote = { type = \"ssh\", host = \"A8\" }\n\
             [[profiles]]\nname = \"B9\"\nremote = { type = \"ssh\", host = \"B9\" }\n\
             [[profiles]]\nname = \"Work\"\n",
        )
        .unwrap()
    }

    fn started_with(profile: Option<&str>) -> SessionInfo {
        SessionInfo { profile: profile.map(Into::into), ..info(1, "k", 0) }
    }

    /// 7.2: a remote session opens with the profile it was started with
    /// when that targets its endpoint, else the first profile that does.
    #[test]
    fn a_remote_session_opens_with_a_profile_on_its_endpoint() {
        let config = profiles();
        let a8 = Host::of(&config.profiles[2]);
        assert_eq!(remote_profile(&started_with(Some("A8")), &a8, &config), Some(2));
        assert_eq!(remote_profile(&started_with(Some("a8")), &a8, &config), Some(2));
        // Started from another machine with a profile this one lacks, with
        // a local profile, with one on another host, or with none.
        for other in [Some("Laptop A8"), Some("Work"), Some("B9"), None] {
            assert_eq!(remote_profile(&started_with(other), &a8, &config), Some(1), "{other:?}");
        }
        // No profile targets the endpoint.
        let c3 = Host::Remote(crate::config::RemoteTarget::Ssh {
            host: "C3".into(),
            user: None,
            port: None,
            identity_file: None,
            ssh_args: Vec::new(),
            connect_at_launch: false,
        });
        assert_eq!(remote_profile(&started_with(Some("A8")), &c3, &config), None);
    }

    fn ssh(host: &str) -> Host {
        Host::Remote(crate::config::RemoteTarget::Ssh {
            host: host.into(),
            user: None,
            port: None,
            identity_file: None,
            ssh_args: Vec::new(),
            connect_at_launch: false,
        })
    }

    /// 6.1: one rule for a session's profile on an endpoint.
    #[test]
    fn a_session_on_an_endpoint_takes_the_first_profile_that_exists() {
        let config = profiles();
        let a8 = Host::of(&config.profiles[2]);
        // The profile it started with, when it targets the endpoint.
        assert_eq!(session_profile(&started_with(Some("A8")), &a8, &config, Some("Work")), 2);
        // Else the first profile that targets the endpoint, whatever the first tab's.
        assert_eq!(session_profile(&started_with(Some("B9")), &a8, &config, Some("Work")), 1);
        assert_eq!(session_profile(&started_with(None), &a8, &config, None), 1);
        // No profile targets the endpoint: the first tab's profile ...
        let c3 = ssh("C3");
        assert_eq!(session_profile(&started_with(Some("A8")), &c3, &config, Some("B9")), 3);
        assert_eq!(session_profile(&started_with(None), &c3, &config, Some("Work")), 4);
        // ... else the default, when that tab's profile is gone or there is no tab.
        assert_eq!(session_profile(&started_with(None), &c3, &config, Some("Gone")), 0);
        assert_eq!(session_profile(&started_with(None), &c3, &config, None), 0);
    }

    /// 6.1: a local session ignores the first tab's profile.
    #[test]
    fn a_local_session_is_unchanged_by_the_first_tab_profile() {
        let config = profiles();
        for first in [None, Some("B9"), Some("Gone")] {
            assert_eq!(session_profile(&started_with(Some("Work")), &Host::Local, &config, first), 4);
            assert_eq!(session_profile(&started_with(Some("A8")), &Host::Local, &config, first), 0);
            assert_eq!(session_profile(&started_with(None), &Host::Local, &config, first), 0);
        }
    }

    /// 6.1: the swept tabs keep the sweep's order and skips, each with the
    /// profile the one rule gives it.
    #[test]
    fn swept_tabs_keep_the_sweeps_order_with_each_sessions_profile() {
        let config = profiles();
        let a8 = Host::of(&config.profiles[2]);
        let with = |session, key: &str, started, profile: Option<&str>| SessionInfo {
            profile: profile.map(Into::into),
            ..info(session, key, started)
        };
        let listed = vec![
            with(3, "c", 30, Some("A8")),
            with(1, "a", 10, Some("Work")),
            with(2, "b", 20, None),
            with(4, "d", 5, Some("htop on A8")),
        ];
        let skip = [ServerSession { id: 4, key: "d".into() }];
        let tabs: Vec<(u32, usize)> = swept_tabs(listed.clone(), &skip, &a8, &config, Some("B9"))
            .into_iter()
            .map(|(tab, index)| (tab.active().session, index))
            .collect();
        assert_eq!(tabs, [(1, 1), (2, 1), (3, 2)]);
        // Where no profile targets the endpoint, the first tab's profile.
        let tabs: Vec<(u32, usize)> = swept_tabs(listed, &[], &ssh("C3"), &config, Some("B9"))
            .into_iter()
            .map(|(tab, index)| (tab.active().session, index))
            .collect();
        assert_eq!(tabs, [(4, 3), (1, 3), (2, 3), (3, 3)]);
        assert!(swept_tabs(Vec::new(), &[], &a8, &config, None).is_empty());
    }

    /// A remote endpoint's sessions detached with one tab's layout come
    /// back as that tab, with the profile its active session gives it; one
    /// a restored zone took is left out of it.
    #[test]
    fn swept_tabs_regroup_a_recorded_split() {
        let config = profiles();
        let a8 = Host::of(&config.profiles[2]);
        let layout = serde_json::json!({
            "v": 1, "origin": "q", "index": 0,
            "zones": {"split": "vertical", "first": {"key": "a"}, "second": {"split": "horizontal",
                "first": {"key": "b", "active": true}, "second": {"key": "c"}}},
        });
        let member = |session, key: &str, profile: &str| SessionInfo {
            profile: Some(profile.into()),
            layout: Some(layout.clone()),
            ..info(session, key, session.into())
        };
        let listed = vec![member(1, "a", "Work"), member(2, "b", "A8"), member(3, "c", "Work"), info(4, "d", 0)];
        let tabs = swept_tabs(listed, &[ServerSession { id: 3, key: "c".into() }], &a8, &config, None);
        let shapes: Vec<(Vec<u32>, u32, usize)> = tabs
            .iter()
            .map(|(tab, index)| (tab.zones.items().iter().map(|s| s.session).collect(), tab.active().session, *index))
            .collect();
        assert_eq!(shapes, [(vec![4], 4, 1), (vec![1, 2], 2, 2)]);
    }

    /// 7.3: a session of this machine's server opens with the profile it
    /// was started with, or the default one when that is gone or remote.
    #[test]
    fn a_local_session_opens_with_its_local_profile_or_the_default() {
        let config = profiles();
        assert_eq!(local_profile(&started_with(Some("Work")), &config), 4);
        assert_eq!(local_profile(&started_with(Some("A8")), &config), 0);
        assert_eq!(local_profile(&started_with(Some("Gone")), &config), 0);
        assert_eq!(local_profile(&started_with(None), &config), 0);
    }

    #[test]
    fn a_launch_connects_to_each_restored_remote_endpoint_once() {
        let config = profiles();
        let (a8, b9) = (Host::of(&config.profiles[1]), Host::of(&config.profiles[3]));
        let hosts = launch_hosts(&[a8.clone(), Host::Local, b9.clone(), Host::of(&config.profiles[2])], &config);
        assert_eq!(hosts, [a8, b9]);
        assert!(launch_hosts(&[], &config).is_empty(), "no profile connects at launch");
        assert!(eager_hosts(&config).is_empty());
    }

    /// 8.1: the profiles with `connect_at_launch` name the eager endpoints,
    /// each once in profile order (one is eager when any of its profiles
    /// is); a restoring launch sweeps them after the restored ones.
    #[test]
    fn profiles_that_connect_at_launch_make_their_endpoints_eager() {
        let config = Config::parse(
            "[[profiles]]\nname = \"Shell\"\n\
             [[profiles]]\nname = \"devbox\"\nremote = { type = \"container\", container = \"devbox\", connect_at_launch = true }\n\
             [[profiles]]\nname = \"htop on A8\"\ncommand = \"htop\"\nremote = { type = \"ssh\", host = \"A8\" }\n\
             [[profiles]]\nname = \"A8\"\nremote = { type = \"ssh\", host = \"A8\", connect_at_launch = true }\n\
             [[profiles]]\nname = \"B9\"\nremote = { type = \"ssh\", host = \"B9\" }\n\
             [[profiles]]\nname = \"devbox again\"\nremote = { type = \"container\", container = \"devbox\", connect_at_launch = true }\n",
        )
        .unwrap();
        let connects: Vec<bool> = config.profiles.iter().map(connects_at_launch).collect();
        assert_eq!(connects, [false, true, false, true, false, true]);
        let (devbox, a8, b9) = (Host::of(&config.profiles[1]), Host::of(&config.profiles[2]), Host::of(&config.profiles[4]));
        assert_eq!(eager_hosts(&config), [devbox.clone(), a8.clone()]);
        assert_eq!(launch_hosts(&[b9.clone(), a8.clone()], &config), [b9, a8, devbox]);
    }

    /// 3.2: the endpoints a close leaves with no tab are those the window
    /// had a tab on before and has none on after; an endpoint with another
    /// tab left, an eager endpoint and this machine are never named.
    #[test]
    fn only_endpoints_left_with_no_tab_that_arent_eager_are_named() {
        let config = profiles();
        let (a8, b9) = (Host::of(&config.profiles[2]), Host::of(&config.profiles[3]));
        let c3 = Host::Remote(crate::config::RemoteTarget::Ssh {
            host: "C3".into(),
            user: None,
            port: None,
            identity_file: None,
            ssh_args: Vec::new(),
            connect_at_launch: false,
        });
        let local = Host::Local;
        // The last tab on A8 went; B9 has another tab left.
        let before = [local.clone(), a8.clone(), b9.clone(), b9.clone()];
        assert_eq!(endpoints_without_tabs(&before, &[local.clone(), b9.clone()], &[]), std::slice::from_ref(&a8));
        // Two tabs on A8 before: the first close leaves one.
        assert!(endpoints_without_tabs(&[a8.clone(), a8.clone()], std::slice::from_ref(&a8), &[]).is_empty());
        assert_eq!(endpoints_without_tabs(&[a8.clone(), a8.clone()], &[], &[]), std::slice::from_ref(&a8));
        // An eager endpoint keeps its listing.
        assert!(endpoints_without_tabs(&[local.clone(), a8.clone()], std::slice::from_ref(&local), std::slice::from_ref(&a8)).is_empty());
        // This machine is never one, even with no tab left.
        assert!(endpoints_without_tabs(std::slice::from_ref(&local), &[], &[]).is_empty());
        // Several at once, in the window's order.
        let gone = endpoints_without_tabs(&[c3.clone(), local.clone(), a8.clone(), b9.clone()], &[local], &[b9]);
        assert_eq!(gone, [c3, a8]);
    }

    #[test]
    fn attach_finds_a_detached_session_or_says_why_not() {
        let mut shown = info(2, "b", 0);
        shown.attached = true;
        let sessions = [info(1, "a", 0), shown];
        assert_eq!(attach_target(1, &sessions).map(|s| s.key), Ok("a".into()));
        assert_eq!(attach_target(2, &sessions), Err("session 2 is shown by another Brindle".into()));
        assert_eq!(attach_target(99, &sessions), Err("the terminal server has no session 99".into()));
        assert_eq!(attach_target(1, &[]), Err("the terminal server has no session 1".into()));
    }

    #[test]
    fn listed_sessions_give_the_attach_target() {
        let mut shown = info(2, "b", 0);
        shown.attached = true;
        let listing = Listing::Listed(vec![info(1, "a", 0), shown]);
        assert_eq!(listing.attach_target(1).map(|s| s.key), Ok("a".into()), "a detached id");
        assert_eq!(listing.attach_target(2), Err("session 2 is shown by another Brindle".into()), "an attached id");
        assert_eq!(listing.attach_target(99), Err("the terminal server has no session 99".into()), "a missing id");
        assert_eq!(Listing::Listed(Vec::new()).attach_target(1), Err("the terminal server has no session 1".into()));
    }

    #[test]
    fn a_failed_listing_names_its_cause_for_attach() {
        assert_eq!(
            Listing::Failed("timed out".into()).attach_target(7),
            Err("Brindle could not list the terminal server's sessions: timed out".into())
        );
        assert_eq!(
            Listing::Skipped.attach_target(7),
            Err("Brindle could not list the terminal server's sessions".into()),
            "no listing, no cause"
        );
        for listing in [Listing::Failed("x".into()), Listing::Skipped] {
            let why = listing.attach_target(7).unwrap_err();
            assert!(!why.contains("has no session"), "never says the session doesn't exist: {why}");
        }
    }

    #[test]
    fn labels_prefer_the_title_then_the_program() {
        let mut s = info(5, "k", 0);
        assert_eq!(label(&s), "session 5");
        s.command = "/usr/bin/htop".into();
        assert_eq!(label(&s), "htop");
        s.command = "bash --norc --noprofile".into();
        assert_eq!(label(&s), "bash");
        s.title = "vim notes.md".into();
        assert_eq!(label(&s), "vim notes.md");
    }
}
