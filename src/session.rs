//! Session restore: the windows and tabs recorded while Brindle runs, the
//! state file they are written to, and how a recorded tab is reopened.
//!
//! The state lives in `session.toml` under `$BRINDLE_STATE_DIR`, else
//! `$XDG_STATE_HOME/brindle` (`~/.local/state/brindle`), or
//! `%LOCALAPPDATA%\brindle` on Windows; with none of them, nowhere. Only
//! the process holding `session.lock` reads or writes it, so a second
//! Brindle never duplicates the first one's tabs.

use std::collections::HashMap;
use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::config::{Config, Profile};
use crate::remote::Host;
use crate::zone_view::{SplitAxis, ZoneId, ZoneTree};

/// The state file format this build reads and writes.
pub const VERSION: u32 = 1;
const FILE_NAME: &str = "session.toml";
const LOCK_NAME: &str = "session.lock";
/// The smallest window a restore opens (the window's minimum size).
pub const MIN_SIZE: (f32, f32) = (360.0, 200.0);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct State {
    pub version: u32,
    #[serde(default)]
    pub windows: Vec<WindowState>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowState {
    /// Logical pixels of the window's (restore) size.
    #[serde(default)]
    pub width: Option<f32>,
    #[serde(default)]
    pub height: Option<f32>,
    #[serde(default)]
    pub maximized: bool,
    /// Index into `tabs`.
    #[serde(default)]
    pub active: usize,
    #[serde(default)]
    pub tabs: Vec<TabState>,
}

/// The terminal server session a zone showed, on its tab's endpoint: its
/// id, and the key that tells it from a later session that reused the id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerSession {
    pub id: u32,
    pub key: String,
}

/// A node in a local tab's recorded binary zone split tree.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ZoneNodeState {
    /// For a leaf: its working directory. Omitted for a split.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    /// For a leaf: its terminal server session, attached again on restore
    /// when it is still detached.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session: Option<ServerSession>,
    /// True on the leaf that had focus when recorded.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub active: bool,
    /// For a split: `horizontal` or `vertical`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub split: Option<SplitAxis>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub first: Option<Box<ZoneNodeState>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub second: Option<Box<ZoneNodeState>>,
}

impl ZoneNodeState {
    pub fn leaf(cwd: Option<PathBuf>, active: bool) -> Self {
        Self { cwd, session: None, active, split: None, first: None, second: None }
    }

    pub fn split(axis: SplitAxis, first: ZoneNodeState, second: ZoneNodeState) -> Self {
        Self {
            cwd: None,
            session: None,
            active: false,
            split: Some(axis),
            first: Some(Box::new(first)),
            second: Some(Box::new(second)),
        }
    }

    /// Builds a `ZoneNodeState` from a `ZoneTree`, returning `None` for a
    /// single-zone tree (which is recorded in the top-level `cwd` and
    /// `session` alone). `leaves` gives each zone's directory and session;
    /// a zone it leaves out isn't recorded, as in [`record_zones`].
    pub fn from_tree(tree: &ZoneTree, active: ZoneId, leaves: &HashMap<ZoneId, ZoneRecord>) -> Option<Self> {
        match tree {
            ZoneTree::Leaf(_) => None,
            ZoneTree::Split { .. } => Self::record_node(tree, active, leaves),
        }
    }

    /// `tree` as recorded, without the leaves `leaves` leaves out: a split's
    /// other side takes a left-out leaf's place. `None` when no leaf is
    /// recorded.
    fn record_node(tree: &ZoneTree, active: ZoneId, leaves: &HashMap<ZoneId, ZoneRecord>) -> Option<Self> {
        match tree {
            ZoneTree::Leaf(id) => {
                let ZoneRecord { cwd, session } = leaves.get(id)?.clone();
                Some(Self { session, ..Self::leaf(cwd, *id == active) })
            }
            ZoneTree::Split { axis, first, second } => {
                match (Self::record_node(first, active, leaves), Self::record_node(second, active, leaves)) {
                    (Some(first), Some(second)) => Some(Self::split(*axis, first, second)),
                    (one, other) => one.or(other),
                }
            }
        }
    }

    /// Whether this leaf, or any leaf under this split, recorded a session.
    fn has_session(&self) -> bool {
        self.session.is_some()
            || self.first.as_ref().is_some_and(|n| n.has_session())
            || self.second.as_ref().is_some_and(|n| n.has_session())
    }

    /// The active leaf's directory.
    fn active_cwd(&self) -> Option<PathBuf> {
        match (&self.first, &self.second) {
            (Some(first), Some(second)) => first.active_cwd().or_else(|| second.active_cwd()),
            _ => self.active.then(|| self.cwd.clone()).flatten(),
        }
    }
}

/// What session restore records of one zone, which its view gathers: its
/// directory and its terminal server session.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ZoneRecord {
    pub cwd: Option<PathBuf>,
    pub session: Option<ServerSession>,
}

/// What a tab records of its zones ([`record_zones`]).
#[derive(Debug, Clone, PartialEq)]
pub struct RecordedZones {
    /// The active zone's directory.
    pub cwd: Option<PathBuf>,
    /// The split tree, when two or more zones are recorded.
    pub zones: Option<ZoneNodeState>,
    /// A single recorded zone's session (a split's are in `zones`).
    pub session: Option<ServerSession>,
}

/// What a tab records of its zone `tree`: each zone's directory and session
/// from `leaves`, which has no entry for a zone that only shows a message
/// (an attach that failed, design.md decision 10). Such a zone isn't
/// recorded: a split's other side takes its place, the first zone left is
/// active when the active one went, and a tab with no zone left isn't
/// recorded (`None`).
pub fn record_zones(tree: &ZoneTree, active: ZoneId, leaves: &HashMap<ZoneId, ZoneRecord>) -> Option<RecordedZones> {
    let mut node = ZoneNodeState::record_node(tree, active, leaves)?;
    if node.split.is_none() {
        return Some(RecordedZones { cwd: node.cwd, zones: None, session: node.session });
    }
    if !node.has_active() {
        node.activate_first();
    }
    Some(RecordedZones { cwd: node.active_cwd(), zones: Some(node), session: None })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
#[allow(
    clippy::large_enum_variant,
    reason = "one value per tab, read from or written to the state file once; boxing `Local`'s fields changes nothing the file shows"
)]
pub enum TabState {
    /// A local tab: rerun the profile in `cwd` (or
    /// `zones` when split into 2+ zones).
    Local {
        profile: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        remote: Option<crate::config::RemoteTarget>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<PathBuf>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        zones: Option<ZoneNodeState>,
        /// A single-zone tab's terminal server session (a split tab's are
        /// in `zones`).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session: Option<ServerSession>,
    },
    /// An entry of a kind this build doesn't restore, such as the tmux
    /// sessions (`kind = "tmux"`) older versions recorded: parsing drops it,
    /// so the other entries still restore, and snapshots never write it.
    #[serde(other)]
    Skipped,
}

impl WindowState {
    /// Drops the entries this build doesn't restore; `active` follows its
    /// entry, or the entry before it when that one was dropped.
    fn drop_skipped(&mut self) {
        let mut active = None;
        let mut tabs = Vec::new();
        for (ix, tab) in std::mem::take(&mut self.tabs).into_iter().enumerate() {
            if tab == TabState::Skipped {
                continue;
            }
            if ix <= self.active {
                active = Some(tabs.len());
            }
            tabs.push(tab);
        }
        self.tabs = tabs;
        self.active = active.unwrap_or(0);
    }
}

impl State {
    pub fn new(windows: Vec<WindowState>) -> Self {
        State { version: VERSION, windows }
    }

    /// Parses a state file. Unknown versions and malformed data are errors,
    /// each with a reason to log. Windows without tabs are dropped, so the
    /// result may have no windows (the last window was closed).
    pub fn parse(text: &str) -> Result<State, String> {
        #[derive(Deserialize)]
        struct Version {
            version: Option<u32>,
        }
        let version: Version = toml::from_str(text).map_err(|e| format!("not a valid state file: {}", e.message()))?;
        match version.version {
            Some(VERSION) => {}
            Some(other) => return Err(format!("unknown format version {other} (this Brindle reads {VERSION})")),
            None => return Err("no format version".into()),
        }
        let mut state: State = toml::from_str(text).map_err(|e| format!("not a valid state file: {}", e.message()))?;
        for window in &mut state.windows {
            window.drop_skipped();
        }
        state.windows.retain(|w| !w.tabs.is_empty());
        for window in &mut state.windows {
            window.active = window.active.min(window.tabs.len() - 1);
        }
        Ok(state)
    }

    /// The state's file text, or why it can't be serialized (a directory
    /// whose name isn't valid UTF-8, say).
    pub fn to_text(&self) -> Result<String, toml::ser::Error> {
        toml::to_string(self)
    }
}

/// What [`state_dir`] decides from, read from the environment by
/// [`StateEnv::read`].
#[derive(Debug, Default, Clone, PartialEq)]
pub struct StateEnv {
    /// `BRINDLE_STATE_DIR`, when set and not empty.
    pub named: Option<PathBuf>,
    /// The platform's directory for state: `$XDG_STATE_HOME` (absolute) on
    /// Linux, `%LOCALAPPDATA%` on Windows.
    pub platform: Option<PathBuf>,
    /// The home directory (`$HOME`), whose `.local/state` stands in for
    /// `$XDG_STATE_HOME`; none on Windows, which always has the platform's.
    pub home: Option<PathBuf>,
}

impl StateEnv {
    /// The environment's values. On Linux both come from the environment
    /// itself, never the password database, so a Brindle started without
    /// `HOME` has no state directory rather than one it wasn't told about.
    pub fn read() -> Self {
        let set = |name: &str| std::env::var_os(name).filter(|value| !value.is_empty()).map(PathBuf::from);
        let named = set("BRINDLE_STATE_DIR");
        if cfg!(windows) {
            return StateEnv { named, platform: dirs::data_local_dir(), home: None };
        }
        StateEnv { named, platform: set("XDG_STATE_HOME").filter(|dir| dir.is_absolute()), home: set("HOME") }
    }
}

/// Where the state file and lock live: `BRINDLE_STATE_DIR`, else `brindle`
/// in the platform's directory for state, else in the home directory's
/// `.local/state`. `None` when there is none of them: Brindle then runs on
/// its own and saves nothing ([`NO_STATE_DIR`]).
pub fn state_dir(env: &StateEnv) -> Option<PathBuf> {
    if let Some(dir) = &env.named {
        return Some(dir.clone());
    }
    let base = env.platform.clone().or_else(|| env.home.as_ref().map(|home| home.join(".local/state")))?;
    Some(base.join("brindle"))
}

/// The window's notice when there is no state directory.
pub const NO_STATE_DIR: &str = "Brindle can't save its session: no state directory (set BRINDLE_STATE_DIR)";

pub fn state_path(dir: &Path) -> PathBuf {
    dir.join(FILE_NAME)
}

/// Reads the state in `dir`. A missing file is `Ok(None)`; anything else
/// that can't be restored is an error naming the file.
pub fn load(dir: &Path) -> Result<Option<State>, String> {
    let path = state_path(dir);
    let text = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(format!("ignoring session state {}: {err}", path.display())),
    };
    match State::parse(&text) {
        // Nothing was open at the end (the last tab closed): a normal launch.
        Ok(state) if state.windows.is_empty() => Ok(None),
        Ok(state) => Ok(Some(state)),
        Err(why) => Err(format!("ignoring session state {}: {why}", path.display())),
    }
}

/// One state directory's `session.toml`, written in ticket order: a write
/// is skipped when a newer ticket has landed, so a slow background write
/// can't replace the final one on quit. The order is this value's own, so
/// two directories' writes never skip each other's.
#[derive(Debug)]
pub struct StateFile {
    dir: PathBuf,
    issued: AtomicU64,
    /// The ticket of the last write that landed; `None` before the first.
    landed: Mutex<Option<u64>>,
}

impl StateFile {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir, issued: AtomicU64::new(0), landed: Mutex::new(None) }
    }

    /// The next ticket, newer than every one issued before it.
    pub fn ticket(&self) -> u64 {
        self.issued.fetch_add(1, Ordering::SeqCst)
    }

    /// Replaces the file with `text` in one step (temp file + rename),
    /// readable only by the user, unless a write with a newer ticket than
    /// `ticket` has landed already.
    pub fn write(&self, text: &str, ticket: u64) -> std::io::Result<()> {
        let mut landed = self.landed.lock().unwrap_or_else(|e| e.into_inner());
        if landed.is_some_and(|landed| ticket < landed) {
            return Ok(());
        }
        private_dir().create(&self.dir)?;
        let tmp = self.dir.join(format!("{FILE_NAME}.tmp"));
        let mut file = private_file().truncate(true).open(&tmp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        std::fs::rename(&tmp, self.path())?;
        *landed = Some(ticket);
        Ok(())
    }

    /// The state file's path, for messages.
    pub fn path(&self) -> PathBuf {
        state_path(&self.dir)
    }
}

/// Exclusive ownership of the state directory, held while this lives.
#[derive(Debug)]
pub struct Lock {
    _file: File,
}

impl Lock {
    /// Takes the lock without waiting. Fails if another process (or another
    /// open of the lock file) holds it.
    pub fn acquire(dir: &Path) -> std::io::Result<Lock> {
        private_dir().create(dir)?;
        let file = private_file().truncate(false).open(dir.join(LOCK_NAME))?;
        // flock(LOCK_EX | LOCK_NB) on Unix, LockFileEx on Windows.
        file.try_lock().map_err(|err| match err {
            std::fs::TryLockError::Error(err) => err,
            std::fs::TryLockError::WouldBlock => std::io::ErrorKind::WouldBlock.into(),
        })?;
        Ok(Lock { _file: file })
    }
}

/// A builder for the state directory: readable only by the user on Unix.
/// On Windows, `%LOCALAPPDATA%` is already per-user.
fn private_dir() -> std::fs::DirBuilder {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder
}

/// Options for writing a state file, created readable only by the user on
/// Unix.
fn private_file() -> std::fs::OpenOptions {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options
}

// ---- restoring ------------------------------------------------------------------

/// A resolved zone node after applying working-directory fallbacks.
#[derive(Debug, Clone, PartialEq)]
pub enum RestoredZoneNode {
    Leaf { cwd: PathBuf, active: bool, session: Option<ServerSession> },
    Split {
        axis: SplitAxis,
        first: Box<RestoredZoneNode>,
        second: Box<RestoredZoneNode>,
    },
}

/// The leaves of a restored split: each zone with its directory and its
/// recorded session.
pub type RestoredLeaves = Vec<(ZoneId, PathBuf, Option<ServerSession>)>;

impl RestoredZoneNode {
    /// Converts this resolved node into a `(ZoneTree, leaves, active_id)`:
    /// each leaf with its directory and its recorded session.
    pub fn into_tree(self) -> (ZoneTree, RestoredLeaves, ZoneId) {
        let mut next_id = 1u64;
        let mut leaves = Vec::new();
        let mut active_id = None;
        let tree = Self::build_tree(self, &mut next_id, &mut leaves, &mut active_id);
        let active = active_id.or_else(|| leaves.first().map(|(id, ..)| *id)).unwrap_or(ZoneId(1));
        (tree, leaves, active)
    }

    fn build_tree(
        node: RestoredZoneNode,
        next_id: &mut u64,
        leaves: &mut Vec<(ZoneId, PathBuf, Option<ServerSession>)>,
        active_id: &mut Option<ZoneId>,
    ) -> ZoneTree {
        match node {
            RestoredZoneNode::Leaf { cwd, active, session } => {
                let id = ZoneId(*next_id);
                *next_id += 1;
                if active && active_id.is_none() {
                    *active_id = Some(id);
                }
                leaves.push((id, cwd, session));
                ZoneTree::Leaf(id)
            }
            RestoredZoneNode::Split { axis, first, second } => ZoneTree::Split {
                axis,
                first: Box::new(Self::build_tree(*first, next_id, leaves, active_id)),
                second: Box::new(Self::build_tree(*second, next_id, leaves, active_id)),
            },
        }
    }
}

/// What to open for a recorded tab, after the fallbacks.
#[derive(Debug, Clone, PartialEq)]
pub enum RestoredTab {
    /// Launch profile `profile` in `cwd` (or `zones` when split), on `host`.
    Local {
        profile: usize,
        /// Where the tab's zones run: the endpoint they recorded sessions
        /// on, else the profile's.
        host: Host,
        cwd: PathBuf,
        title: Option<String>,
        zones: Option<RestoredZoneNode>,
        /// A single-zone tab's recorded session, attached when still detached.
        session: Option<ServerSession>,
        /// The recorded profile's name when the config has no such profile
        /// any more (the tab took the default one), for the shell to report.
        missing_profile: Option<String>,
    },
}

/// A recorded directory on `host`, else `profile`'s own when the profile
/// runs there, else home (a remote endpoint's default, the empty path).
/// `is_dir` is the probe for a local directory (the shell passes
/// `Path::is_dir`).
fn resolve_cwd(cwd: Option<&Path>, host: &Host, profile: &Profile, home: &Path, is_dir: &dyn Fn(&Path) -> bool) -> PathBuf {
    // The profile's directory is a path on its own endpoint.
    let own = (Host::of(profile) == *host).then_some(profile);
    if !host.is_local() {
        return cwd
            .map(Path::to_path_buf)
            .or_else(|| own.and_then(|p| p.cwd.as_deref()).map(PathBuf::from))
            .unwrap_or_default();
    }
    cwd.filter(|d| is_dir(d))
        .map(Path::to_path_buf)
        .or_else(|| own.and_then(|p| p.working_directory(Some(home))).filter(|d| is_dir(d)))
        .unwrap_or_else(|| home.to_path_buf())
}

fn resolve_zone_node(
    node: &ZoneNodeState,
    host: &Host,
    profile: &Profile,
    home: &Path,
    is_dir: &dyn Fn(&Path) -> bool,
) -> RestoredZoneNode {
    if let (Some(axis), Some(first), Some(second)) = (node.split, node.first.as_deref(), node.second.as_deref()) {
        RestoredZoneNode::Split {
            axis,
            first: Box::new(resolve_zone_node(first, host, profile, home, is_dir)),
            second: Box::new(resolve_zone_node(second, host, profile, home, is_dir)),
        }
    } else {
        RestoredZoneNode::Leaf {
            cwd: resolve_cwd(node.cwd.as_deref(), host, profile, home, is_dir),
            active: node.active,
            session: node.session.clone(),
        }
    }
}

/// Resolves a recorded tab against the current config: an unknown profile
/// becomes the default one (and is named in `missing_profile`), and a
/// vanished directory the profile's own (or `home`). `is_dir` is the probe
/// for whether a local directory is still there. `None` for an entry that
/// isn't restored (parsing drops those).
///
/// A tab whose zones recorded terminal server sessions stays on the
/// endpoint it recorded, whatever its profile targets now, so they can be
/// attached there; its zones without a session start there too, as a
/// split of the tab would. A tab with no session record follows its
/// profile, and keeps its directories only when that is still the same
/// endpoint (the "Remote endpoint identity" requirement).
pub fn resolve_tab(
    tab: &TabState,
    config: &Config,
    home: &Path,
    is_dir: &dyn Fn(&Path) -> bool,
) -> Option<RestoredTab> {
    let (name, recorded_remote, cwd, title, zones, session) = match tab {
        TabState::Local { profile, remote, cwd, title, zones, session } => (
            profile,
            remote.as_ref(),
            cwd.as_deref(),
            title.as_deref().and_then(|t| {
                let trimmed = t.trim();
                (!trimmed.is_empty()).then(|| trimmed.to_string())
            }),
            zones.as_ref(),
            session.as_ref(),
        ),
        TabState::Skipped => return None,
    };
    let found = config.profile_index(name);
    let missing_profile = found.is_none().then(|| name.clone());
    let index = found.unwrap_or_else(|| config.default_profile_index());
    let profile = &config.profiles[index];
    let recorded = recorded_remote.cloned().map_or(Host::Local, Host::Remote);
    let has_session = session.is_some() || zones.is_some_and(ZoneNodeState::has_session);
    let host = if has_session { recorded.clone() } else { Host::of(profile) };
    let same_endpoint = host == recorded;
    let cwd = resolve_cwd(cwd.filter(|_| same_endpoint), &host, profile, home, is_dir);
    let zones = zones
        .filter(|_| same_endpoint)
        .map(|z| resolve_zone_node(z, &host, profile, home, is_dir))
        .filter(|z| matches!(z, RestoredZoneNode::Split { .. }));
    // Only with a record, which keeps the tab on its endpoint.
    let session = session.cloned();
    Some(RestoredTab::Local { profile: index, host, cwd, title, zones, session, missing_profile })
}

/// The recorded windows as the one window Brindle has: every window's tabs
/// in order, with the first window's size and active tab. `None` when no
/// window has tabs.
pub fn merge_windows(windows: Vec<WindowState>) -> Option<WindowState> {
    let mut windows = windows.into_iter().filter(|w| !w.tabs.is_empty());
    let mut merged = windows.next()?;
    for window in windows {
        merged.tabs.extend(window.tabs);
    }
    Some(merged)
}

/// `window` without the local zones whose recorded terminal server session
/// is gone (`alive` says which are left, from this machine's server): a
/// split's other side takes a dropped zone's place, and a tab with no zone
/// left is dropped. Zones with no record stay, and so do a remote tab's,
/// whose sessions are its endpoint's. `None` when no tab is left.
pub fn drop_lost_zones(mut window: WindowState, alive: impl Fn(&ServerSession) -> bool) -> Option<WindowState> {
    let lost = |session: &Option<ServerSession>| session.as_ref().is_some_and(|s| !alive(s));
    let mut active = None;
    let mut tabs = Vec::new();
    for (ix, tab) in std::mem::take(&mut window.tabs).into_iter().enumerate() {
        let tab = match tab {
            tab @ TabState::Local { remote: Some(_), .. } => tab,
            TabState::Local { profile, remote, cwd, title, zones: Some(zones), session } => {
                match prune_zones(zones, &lost) {
                    None => continue,
                    // One zone left: recorded as a single-zone tab is.
                    Some(leaf) if leaf.split.is_none() => TabState::Local {
                        profile,
                        remote,
                        cwd: leaf.cwd.or(cwd),
                        title,
                        zones: None,
                        session: leaf.session,
                    },
                    Some(mut zones) => {
                        if !zones.has_active() {
                            zones.activate_first();
                        }
                        TabState::Local { profile, remote, cwd, title, zones: Some(zones), session }
                    }
                }
            }
            TabState::Local { zones: None, ref session, .. } if lost(session) => continue,
            tab @ (TabState::Local { .. } | TabState::Skipped) => tab,
        };
        if ix <= window.active {
            active = Some(tabs.len());
        }
        tabs.push(tab);
    }
    if tabs.is_empty() {
        return None;
    }
    window.tabs = tabs;
    window.active = active.unwrap_or(0);
    Some(window)
}

/// `node` without its lost leaves; `None` when none is left.
fn prune_zones(node: ZoneNodeState, lost: &impl Fn(&Option<ServerSession>) -> bool) -> Option<ZoneNodeState> {
    match (node.first, node.second) {
        (Some(first), Some(second)) => match (prune_zones(*first, lost), prune_zones(*second, lost)) {
            (Some(first), Some(second)) => Some(ZoneNodeState { first: Some(Box::new(first)), second: Some(Box::new(second)), ..node }),
            (one, other) => one.or(other),
        },
        _ if lost(&node.session) => None,
        _ => Some(ZoneNodeState { first: None, second: None, ..node }),
    }
}

impl ZoneNodeState {
    fn has_active(&self) -> bool {
        self.active
            || self.first.as_ref().is_some_and(|n| n.has_active())
            || self.second.as_ref().is_some_and(|n| n.has_active())
    }

    fn activate_first(&mut self) {
        match &mut self.first {
            Some(first) => first.activate_first(),
            None => self.active = true,
        }
    }
}

// ---- recording --------------------------------------------------------------------

/// One tab as seen when taking a snapshot.
#[derive(Debug, Clone, PartialEq)]
#[allow(
    clippy::large_enum_variant,
    reason = "one value per tab, built when the state is recorded and dropped after it is written"
)]
pub enum SnapshotTab {
    Local {
        profile: String,
        remote: Option<crate::config::RemoteTarget>,
        cwd: Option<PathBuf>,
        title: Option<String>,
        zones: Option<ZoneNodeState>,
        session: Option<ServerSession>,
    },
    /// Not recorded (message tabs).
    Skip,
}

/// Turns a window's tabs into recorded entries, skipped tabs dropped.
/// Returns the entries and the entry the active tab maps to.
pub fn collapse(tabs: &[SnapshotTab], active: usize) -> (Vec<TabState>, usize) {
    let mut entries = Vec::new();
    let mut active_entry = None;
    for (ix, tab) in tabs.iter().enumerate() {
        let entry = match tab {
            SnapshotTab::Local { profile, remote, cwd, title, zones, session } => {
                entries.push(TabState::Local {
                    profile: profile.clone(),
                    remote: remote.clone(),
                    cwd: cwd.clone(),
                    title: title.clone(),
                    zones: zones.clone(),
                    session: session.clone(),
                });
                Some(entries.len() - 1)
            }
            SnapshotTab::Skip => None,
        };
        if ix == active {
            active_entry = entry;
        }
    }
    let active = active_entry.unwrap_or(0).min(entries.len().saturating_sub(1));
    (entries, active)
}

struct Entry {
    window: u64,
    /// Creation order, which is the order windows are restored in.
    order: u64,
    state: WindowState,
}

/// The recorded windows of this process (a GPUI global).
#[derive(Default)]
pub struct SessionStore {
    entries: Vec<Entry>,
    /// Something was recorded or forgotten: until then there is nothing to
    /// write (an empty store must not replace a file not yet restored from).
    touched: bool,
    /// The text the state file last received from this process: set only by
    /// [`next_write`] when it returns `Write`, and kept through failures, so
    /// the next state that serializes is written when it differs from this.
    last_written: Option<String>,
    /// The serialization failure last reported, cleared by any state that
    /// serializes, so a failure that repeats every check is reported once.
    unserializable: Option<toml::ser::Error>,
    lock: Option<Lock>,
}

/// What the state file needs ([`SessionStore::pending_write`]).
#[derive(Debug, PartialEq)]
pub enum PendingWrite {
    /// Nothing to write: this process doesn't own the file, nothing was
    /// recorded yet, or the state's text is what the file last received.
    Nothing,
    /// Replace the file with this text.
    Write(String),
    /// The state can't be serialized. The file stays as it was; report this.
    Unserializable(toml::ser::Error),
    /// The same failure as the last call, already reported.
    StillUnserializable,
}

/// Decides what the file needs from the state's serialization. A failure
/// leaves `last_written` alone and is returned as `Unserializable` once per
/// run of equal failures; a success clears it.
fn next_write(
    last_written: &mut Option<String>,
    reported: &mut Option<toml::ser::Error>,
    text: Result<String, toml::ser::Error>,
) -> PendingWrite {
    match text {
        Ok(text) => {
            *reported = None;
            if last_written.as_ref() == Some(&text) {
                return PendingWrite::Nothing;
            }
            *last_written = Some(text.clone());
            PendingWrite::Write(text)
        }
        Err(error) => {
            if reported.as_ref() == Some(&error) {
                return PendingWrite::StillUnserializable;
            }
            *reported = Some(error.clone());
            PendingWrite::Unserializable(error)
        }
    }
}

impl gpui::Global for SessionStore {}

impl SessionStore {
    /// A store holding `lock`, taken at launch; without it, this process
    /// neither restores nor records.
    pub fn new(lock: Option<Lock>) -> Self {
        Self { lock, ..Self::default() }
    }

    /// Whether this process records and restores the session: it took the
    /// state directory's lock at launch (the instance).
    pub fn is_owner(&self) -> bool {
        self.lock.is_some()
    }

    pub fn record(&mut self, window: u64, order: u64, state: WindowState) {
        self.touched = true;
        match self.entries.iter_mut().find(|e| e.window == window) {
            Some(entry) => entry.state = state,
            None => self.entries.push(Entry { window, order, state }),
        }
    }

    /// Drops a window, e.g. because its last tab closed.
    pub fn forget(&mut self, window: u64) {
        self.entries.retain(|e| e.window != window);
        // Even if the window was never recorded (its tabs closed within the
        // first second), the file it was restored from is now out of date.
        self.touched = true;
    }

    /// Drops windows that closed, unless none are left: then the app is
    /// quitting with them, and they are kept.
    pub fn prune(&mut self, live: &[u64]) {
        if live.is_empty() {
            return;
        }
        let before = self.entries.len();
        self.entries.retain(|e| live.contains(&e.window));
        self.touched |= self.entries.len() != before;
    }

    pub fn state(&self) -> State {
        let mut entries: Vec<&Entry> = self.entries.iter().filter(|e| !e.state.tabs.is_empty()).collect();
        entries.sort_by_key(|e| e.order);
        State::new(entries.into_iter().map(|e| e.state.clone()).collect())
    }

    /// What to do with the state file now: write the text if it changed
    /// since the last write, or report that the state can't be serialized.
    pub fn pending_write(&mut self) -> PendingWrite {
        if !self.touched || self.lock.is_none() {
            return PendingWrite::Nothing;
        }
        let text = self.state().to_text();
        next_write(&mut self.last_written, &mut self.unserializable, text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fresh directory for one test, removed when it ends.
    fn tmp_dir(name: &str) -> crate::test_support::TempDir {
        crate::test_support::TempDir::new(&format!("session-{name}"))
    }

    fn sample() -> State {
        State::new(vec![
            WindowState {
                width: Some(1000.0),
                height: Some(660.0),
                maximized: false,
                active: 1,
                tabs: vec![
                    TabState::Local { profile: "Shell".into(), remote: None, cwd: Some("/tmp".into()), title: Some("server".into()), zones: None, session: None },
                    TabState::Local { profile: "Shell".into(), remote: None, cwd: None, title: None, zones: None, session: None },
                ],
            },
            WindowState { width: None, height: None, maximized: true, active: 0, tabs: vec![TabState::Local { profile: "x".into(), remote: None, cwd: None, title: None, zones: None, session: None }] },
        ])
    }

    #[test]
    fn no_home_and_no_override_is_none() {
        assert_eq!(state_dir(&StateEnv::default()), None);
    }

    #[test]
    fn the_override_wins() {
        let env = StateEnv {
            named: Some("/tmp/s".into()),
            platform: Some("/state".into()),
            home: Some("/home/u".into()),
        };
        assert_eq!(state_dir(&env), Some(PathBuf::from("/tmp/s")));
        let env = StateEnv { named: None, ..env };
        assert_eq!(state_dir(&env), Some(PathBuf::from("/state/brindle")), "the platform's directory next");
        let env = StateEnv { platform: None, ..env };
        assert_eq!(state_dir(&env), Some(PathBuf::from("/home/u/.local/state/brindle")), "then the home's");
    }

    /// Every combination of the three: the override, else `brindle` in the
    /// platform's directory, else in the home's `.local/state`, else none.
    #[test]
    fn the_state_dir_is_the_first_of_the_override_the_platform_and_the_home() {
        let named = || Some(PathBuf::from("/tmp/s"));
        let platform = || Some(PathBuf::from("/state"));
        let home = || Some(PathBuf::from("/home/u"));
        let cases = [
            (named(), platform(), home(), Some("/tmp/s")),
            (named(), platform(), None, Some("/tmp/s")),
            (named(), None, home(), Some("/tmp/s")),
            (named(), None, None, Some("/tmp/s")),
            (None, platform(), home(), Some("/state/brindle")),
            (None, platform(), None, Some("/state/brindle")),
            (None, None, home(), Some("/home/u/.local/state/brindle")),
            (None, None, None, None),
        ];
        for (named, platform, home, expected) in cases {
            let env = StateEnv { named, platform, home };
            assert_eq!(state_dir(&env), expected.map(PathBuf::from), "{env:?}");
        }
    }

    #[test]
    fn round_trip() {
        let text = sample().to_text().expect("serializes");
        assert!(text.contains("version = 1"), "{text}");
        assert!(text.contains("kind = \"local\""), "{text}");
        assert!(text.contains("title = \"server\""), "{text}");
        assert_eq!(text.matches("title = ").count(), 1, "{text}");
        assert_eq!(State::parse(&text).unwrap(), sample());
    }

    #[test]
    fn rejects_bad_files() {
        assert!(State::parse("version = 2\n[[windows]]\n").unwrap_err().contains("version 2"));
        assert!(State::parse("this is not toml [").unwrap_err().contains("not a valid"));
        assert!(State::parse("[[windows]]\nactive = 0\n").unwrap_err().contains("no format version"));
        // A local entry missing its profile is malformed.
        assert!(State::parse("version = 1\n[[windows]]\n[[windows.tabs]]\nkind = \"local\"\n").is_err());
        assert!(State::parse("version = 1\n").unwrap().windows.is_empty());
        // Windows without tabs are dropped, then nothing is left.
        assert!(State::parse("version = 1\n[[windows]]\nactive = 0\n").unwrap().windows.is_empty());
    }

    #[test]
    fn active_is_clamped_and_unknown_fields_ignored() {
        let text = "version = 1\nfuture = true\n[[windows]]\nactive = 7\n[[windows.tabs]]\nkind = \"local\"\nprofile = \"a\"\n";
        let state = State::parse(text).unwrap();
        assert_eq!(state.windows[0].active, 0);
        assert_eq!(state.windows[0].tabs, vec![TabState::Local { profile: "a".into(), remote: None, cwd: None, title: None, zones: None, session: None }]);
    }

    #[test]
    fn atomic_write_and_load() {
        let dir = tmp_dir("write");
        let file = StateFile::new(dir.to_path_buf());
        assert_eq!(file.path(), state_path(&dir));
        assert_eq!(load(&dir), Ok(None));
        file.write(&sample().to_text().expect("serializes"), file.ticket()).unwrap();
        assert_eq!(load(&dir), Ok(Some(sample())));
        assert!(!dir.join("session.toml.tmp").exists());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(state_path(&dir)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600);
        }
        // An older write that lands late doesn't replace a newer one.
        let old = file.ticket();
        let new = file.ticket();
        file.write("version = 1\n", new).unwrap();
        file.write(&sample().to_text().expect("serializes"), old).unwrap();
        assert_eq!(std::fs::read_to_string(state_path(&dir)).unwrap(), "version = 1\n");
        // A state with no windows is a normal launch, not an error.
        file.write("version = 1\nwindows = []\n", file.ticket()).unwrap();
        assert_eq!(load(&dir), Ok(None));
        std::fs::write(state_path(&dir), "garbage").unwrap();
        assert!(load(&dir).unwrap_err().contains("ignoring session state"));
    }

    /// Tickets order the writes of one directory only: a newer write to
    /// another directory doesn't make this one's older ticket stale.
    #[test]
    fn a_newer_write_elsewhere_does_not_skip_this_directorys_write() {
        let (a, b) = (tmp_dir("ordered-a"), tmp_dir("ordered-b"));
        let (file_a, file_b) = (StateFile::new(a.to_path_buf()), StateFile::new(b.to_path_buf()));
        let first = file_a.ticket();
        // `b`'s landed ticket is a higher number than `first`.
        file_b.ticket();
        file_b.write("version = 1\n", file_b.ticket()).unwrap();
        file_a.write(&sample().to_text().expect("serializes"), first).unwrap();
        assert_eq!(load(&a), Ok(Some(sample())));
    }

    #[test]
    fn lock_is_exclusive() {
        let dir = tmp_dir("lock");
        let first = Lock::acquire(&dir).unwrap();
        assert!(Lock::acquire(&dir).is_err());
        drop(first);
        // A child forked by another test in this binary briefly shares the lock's
        // open file description (until its own exec), so the retake can fail for
        // a few milliseconds after the drop. Retry for a bounded time.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let retaken = loop {
            match Lock::acquire(&dir) {
                Ok(lock) => break Ok(lock),
                Err(e) if std::time::Instant::now() >= deadline => break Err(e),
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(10)),
            }
        };
        assert!(retaken.is_ok(), "lock not retaken within 5 s: {:?}", retaken.err());
        drop(retaken);
    }

    fn config() -> Config {
        Config::parse(
            r#"
            default_profile = "Shell"
            [[profiles]]
            name = "Shell"
            [[profiles]]
            name = "Pinned"
            cwd = "/"
            "#,
        )
        .unwrap().config
    }

    /// A probe that answers yes for `dirs` alone: the directories that exist.
    fn only<'a>(dirs: &'a [&'a str]) -> impl Fn(&Path) -> bool + 'a {
        move |dir| dirs.iter().any(|d| Path::new(d) == dir)
    }

    /// No directory exists: every lookup falls back.
    fn none_exist(_: &Path) -> bool {
        false
    }

    fn local_tab(profile: &str, cwd: Option<&str>) -> TabState {
        TabState::Local { profile: profile.into(), remote: None, cwd: cwd.map(PathBuf::from), title: None, zones: None, session: None }
    }

    #[test]
    fn resolve_fallbacks() {
        let config = config();
        let home = Path::new("/home/nobody");
        let work = "/srv/work";
        let existing = [work, "/"];
        let is_dir = only(&existing);
        let resolve = |tab: &TabState| resolve_tab(tab, &config, home, &is_dir);
        let restored = |profile, cwd: &str, title: Option<&str>| {
            Some(RestoredTab::Local {
                profile,
                host: Host::Local,
                cwd: cwd.into(),
                title: title.map(Into::into),
                zones: None,
                session: None,
                missing_profile: None,
            })
        };
        // Recorded directory kept.
        assert_eq!(resolve(&local_tab("Shell", Some(work))), restored(0, work, None));
        // Deleted directory: home, or the profile's own directory.
        let gone = "/srv/gone";
        assert_eq!(resolve(&local_tab("Shell", Some(gone))), restored(0, "/home/nobody", None));
        assert_eq!(resolve(&local_tab("shell", None)), restored(0, "/home/nobody", None));
        assert_eq!(resolve(&local_tab("Pinned", Some(gone))), restored(1, "/", None));
        // The profile's own directory gone too: home.
        assert_eq!(resolve_tab(&local_tab("Pinned", Some(gone)), &config, home, &none_exist), restored(1, "/home/nobody", None));
        // Recorded manual title is preserved for local tabs (and trimmed/ignored if empty).
        assert_eq!(
            resolve(&TabState::Local { profile: "Shell".into(), remote: None, cwd: Some(work.into()), title: Some("server".into()), zones: None, session: None }),
            restored(0, work, Some("server"))
        );
        assert_eq!(
            resolve(&TabState::Local { profile: "Shell".into(), remote: None, cwd: Some(work.into()), title: Some("  ".into()), zones: None, session: None }),
            restored(0, work, None)
        );
        assert_eq!(resolve(&TabState::Skipped), None);
    }

    #[test]
    fn a_missing_profile_is_named_on_the_restored_tab() {
        let config = config();
        let home = Path::new("/home/nobody");
        let Some(RestoredTab::Local { profile, cwd, missing_profile, .. }) =
            resolve_tab(&local_tab("Gone", Some("/srv/work")), &config, home, &only(&["/srv/work"]))
        else {
            panic!("not local")
        };
        // The default profile takes its place, in the recorded directory.
        assert_eq!((profile, cwd, missing_profile), (0, PathBuf::from("/srv/work"), Some("Gone".to_string())));
        // A profile found under another case isn't missing.
        let Some(RestoredTab::Local { profile, missing_profile, .. }) = resolve_tab(&local_tab("pinned", None), &config, home, &none_exist)
        else {
            panic!("not local")
        };
        assert_eq!((profile, missing_profile), (1, None));
    }

    #[test]
    fn old_tmux_entries_are_skipped() {
        let local = |profile: &str| TabState::Local { profile: profile.into(), remote: None, cwd: None, title: None, zones: None, session: None };
        // A tmux entry between two local tabs: the two local tabs restore,
        // in order, and the active tab follows its entry.
        let text = "version = 1\n[[windows]]\nactive = 2\n\
                    [[windows.tabs]]\nkind = \"local\"\nprofile = \"a\"\n\
                    [[windows.tabs]]\nkind = \"tmux\"\nprofile = \"tmux\"\nsession = \"work\"\n\
                    [[windows.tabs]]\nkind = \"local\"\nprofile = \"b\"\n";
        let state = State::parse(text).unwrap();
        assert_eq!(state.windows.len(), 1);
        assert_eq!(state.windows[0].tabs, vec![local("a"), local("b")]);
        assert_eq!(state.windows[0].active, 1);
        // The active entry was the tmux one: the tab before it is active.
        let state = State::parse(&text.replace("active = 2", "active = 1")).unwrap();
        assert_eq!(state.windows[0].active, 0);
        // Saving it again writes no tmux entry.
        let saved = state.to_text().expect("serializes");
        assert!(!saved.contains("tmux"), "{saved}");

        // Only tmux entries: nothing to restore, as with no state file.
        let only = "version = 1\n[[windows]]\n[[windows.tabs]]\nkind = \"tmux\"\nprofile = \"tmux\"\nsession = \"main\"\n\
                    [[windows]]\n[[windows.tabs]]\nkind = \"tmux\"\nprofile = \"work\"\nsession = \"work\"\n";
        assert!(State::parse(only).unwrap().windows.is_empty());
        let dir = tmp_dir("only-tmux");
        let file = StateFile::new(dir.to_path_buf());
        file.write(only, file.ticket()).unwrap();
        assert_eq!(load(&dir), Ok(None));
    }

    #[test]
    fn a_tabs_server_session_round_trips_and_old_files_still_load() {
        let session = ServerSession { id: 3, key: "9f2c".into() };
        let tab = TabState::Local {
            profile: "Shell".into(),
            remote: None,
            cwd: Some("/tmp".into()),
            title: None,
            zones: None,
            session: Some(session.clone()),
        };
        let state = State::new(vec![WindowState { width: None, height: None, maximized: false, active: 0, tabs: vec![tab] }]);
        let text = state.to_text().expect("serializes");
        assert!(text.contains("key = \"9f2c\""), "{text}");
        assert!(text.contains("version = 1"), "{text}");
        assert_eq!(State::parse(&text).unwrap(), state);
        // Written before sessions were recorded: no session, same version.
        let old = "version = 1\n[[windows]]\n[[windows.tabs]]\nkind = \"local\"\nprofile = \"Shell\"\n";
        let TabState::Local { session: none, .. } = &State::parse(old).unwrap().windows[0].tabs[0] else {
            panic!("not a local tab")
        };
        assert_eq!(none, &None);

        // Restoring keeps it for a local profile, and (7.1) for a remote
        // one, on the endpoint it was recorded on.
        let config = Config::parse(
            "[[profiles]]\nname = \"Shell\"\n[[profiles]]\nname = \"R\"\nremote = { type = \"ssh\", host = \"h\" }\n",
        )
        .unwrap().config;
        let home = Path::new("/home/u");
        let Some(RestoredTab::Local { session: kept, host, .. }) =
            resolve_tab(&state.windows[0].tabs[0], &config, home, &none_exist)
        else {
            panic!("not local")
        };
        assert_eq!((kept, host), (Some(session.clone()), Host::Local));
        let target = config.profiles[1].remote.clone().unwrap();
        let remote = TabState::Local {
            profile: "R".into(),
            remote: Some(target.clone()),
            cwd: None,
            title: None,
            zones: None,
            session: Some(session.clone()),
        };
        let Some(RestoredTab::Local { session: kept, host, .. }) = resolve_tab(&remote, &config, home, &none_exist) else {
            panic!("not local")
        };
        assert_eq!((kept, host), (Some(session), Host::Remote(target)));
    }

    fn ssh(host: &str) -> crate::config::RemoteTarget {
        crate::config::RemoteTarget::Ssh { host: host.into(), user: None, port: None, identity_file: None, ssh_args: Vec::new(), connect_at_launch: false }
    }

    /// 7.1: a remote zone that recorded a session is attached on the host
    /// it recorded, whatever the profile targets now (another host, this
    /// machine, or a profile that is gone), with its zones and directories,
    /// and its unrecorded zones start there too. Without a record, the tab
    /// follows its profile.
    #[test]
    fn a_recorded_remote_session_is_restored_on_its_recorded_host() {
        let home = Path::new("/local/home");
        let s = |id: u32| Some(ServerSession { id, key: format!("k{id}") });
        let leaf = |cwd: &str, session, active| ZoneNodeState { session, ..ZoneNodeState::leaf(Some(cwd.into()), active) };
        let split = ZoneNodeState::split(SplitAxis::Vertical, leaf("/srv/a", s(3), true), leaf("/srv/b", None, false));
        let tab = |profile: &str, zones: Option<ZoneNodeState>, session| TabState::Local {
            profile: profile.into(),
            remote: Some(ssh("old-host")),
            cwd: Some("/srv/a".into()),
            title: None,
            zones,
            session,
        };
        let moved = Config::parse(
            "default_profile = \"Local\"\n[[profiles]]\nname = \"Local\"\n\
             [[profiles]]\nname = \"Remote\"\ncwd = \"/home/new\"\nremote = { type = \"ssh\", host = \"new-host\" }\n",
        )
        .unwrap().config;
        let old = Host::Remote(ssh("old-host"));

        // A single zone: the profile now points to new-host.
        let Some(RestoredTab::Local { profile, host, cwd, session, .. }) =
            resolve_tab(&tab("Remote", None, s(7)), &moved, home, &none_exist)
        else {
            panic!("not local")
        };
        assert_eq!((profile, host, cwd, session), (1, old.clone(), PathBuf::from("/srv/a"), s(7)));

        // A split, under a profile that is gone (the default is local).
        let Some(RestoredTab::Local { profile, host, zones, .. }) =
            resolve_tab(&tab("Gone", Some(split.clone()), None), &moved, home, &none_exist)
        else {
            panic!("not local")
        };
        assert_eq!((profile, host), (0, old));
        let (_, leaves, _) = zones.expect("the split is kept").into_tree();
        let leaves: Vec<_> = leaves.into_iter().map(|(_, cwd, session)| (cwd, session)).collect();
        assert_eq!(leaves, [(PathBuf::from("/srv/a"), s(3)), (PathBuf::from("/srv/b"), None)]);

        // No record: the profile's endpoint, its directory, no zones.
        let unrecorded = ZoneNodeState::split(SplitAxis::Vertical, leaf("/srv/a", None, true), leaf("/srv/b", None, false));
        let Some(RestoredTab::Local { host, cwd, zones, session, .. }) =
            resolve_tab(&tab("Remote", Some(unrecorded), None), &moved, home, &none_exist)
        else {
            panic!("not local")
        };
        assert_eq!((host, cwd, zones, session), (Host::Remote(ssh("new-host")), PathBuf::from("/home/new"), None, None));
    }

    /// 7.1: a zone that only shows a message (its attach failed) isn't
    /// recorded: a split's other side takes its place, and a tab with no
    /// zone left isn't recorded at all.
    #[test]
    fn a_zone_that_only_shows_a_message_isnt_recorded() {
        let (z1, z2, z3) = (ZoneId(1), ZoneId(2), ZoneId(3));
        let mut tree = ZoneTree::new(z1);
        tree.split(z1, SplitAxis::Horizontal, z2);
        tree.split(z2, SplitAxis::Vertical, z3);
        let session = |id: u32| Some(ServerSession { id, key: format!("k{id}") });
        let record = |n: u64, session| (ZoneId(n), ZoneRecord { cwd: Some(PathBuf::from(format!("/d{n}"))), session });
        // Zone 2, the active one, failed to attach: it has no entry.
        let leaves: HashMap<ZoneId, ZoneRecord> = [record(1, session(1)), record(3, session(3))].into();
        let recorded = record_zones(&tree, z2, &leaves).unwrap();
        let first = ZoneNodeState { session: session(1), ..ZoneNodeState::leaf(Some("/d1".into()), true) };
        let third = ZoneNodeState { session: session(3), ..ZoneNodeState::leaf(Some("/d3".into()), false) };
        assert_eq!(recorded.zones, Some(ZoneNodeState::split(SplitAxis::Horizontal, first, third)));
        assert_eq!((recorded.cwd, recorded.session), (Some("/d1".into()), None));

        // One zone left: recorded as a single-zone tab, with its session.
        let mut two = ZoneTree::new(z1);
        two.split(z1, SplitAxis::Vertical, z2);
        let recorded = record_zones(&two, z1, &leaves).unwrap();
        assert_eq!(recorded, RecordedZones { cwd: Some("/d1".into()), zones: None, session: session(1) });

        // Every zone failed: nothing is recorded.
        assert_eq!(record_zones(&ZoneTree::new(z2), z2, &leaves), None);
        assert_eq!(record_zones(&two, z1, &HashMap::new()), None);
        // Nothing left out: the same as `from_tree`.
        let every: HashMap<ZoneId, ZoneRecord> = [record(1, None), record(2, None), record(3, None)].into();
        let all = record_zones(&tree, z3, &every).unwrap();
        assert_eq!(all.zones, ZoneNodeState::from_tree(&tree, z3, &every));
        assert_eq!(all.cwd, Some("/d3".into()));
    }

    #[test]
    fn remote_restore_drops_cwd_from_another_endpoint() {
        let config = Config::parse(
            "default_profile = \"Remote\"\n[[profiles]]\nname = \"Remote\"\ncwd = \"/home/current\"\nremote = { type = \"ssh\", host = \"new-host\" }\n"
        )
        .unwrap().config;
        let tab = TabState::Local {
            profile: "Remote".into(),
            remote: Some(ssh("old-host")),
            cwd: Some("/secret/on/old-host".into()),
            title: None,
            zones: None,
            session: None,
        };
        assert_eq!(
            resolve_tab(&tab, &config, Path::new("/local/home"), &none_exist),
            Some(RestoredTab::Local {
                profile: 0,
                host: Host::Remote(ssh("new-host")),
                cwd: "/home/current".into(),
                title: None,
                zones: None,
                session: None,
                missing_profile: None,
            })
        );
    }

    #[test]
    fn merging_windows_keeps_every_tab_and_the_first_windows_layout() {
        let tab = |p: &str| TabState::Local { profile: p.into(), remote: None, cwd: None, title: None, zones: None, session: None };
        let window = |width, active, tabs| WindowState { width: Some(width), height: None, maximized: false, active, tabs };
        let merged = merge_windows(vec![
            window(10.0, 0, vec![]),
            window(20.0, 1, vec![tab("a"), tab("b")]),
            window(30.0, 0, vec![tab("c")]),
        ]);
        assert_eq!(merged, Some(window(20.0, 1, vec![tab("a"), tab("b"), tab("c")])));
        assert_eq!(merge_windows(vec![window(10.0, 0, vec![])]), None);
        assert_eq!(merge_windows(vec![]), None);
    }

    #[test]
    fn lost_zones_are_dropped() {
        let s = |id: u32| Some(ServerSession { id, key: format!("k{id}") });
        let leaf = |id: u32, active: bool| ZoneNodeState { session: s(id), ..ZoneNodeState::leaf(Some(format!("/d{id}").into()), active) };
        let single = |id: u32| TabState::Local { profile: "p".into(), remote: None, cwd: None, title: None, zones: None, session: s(id) };
        let split = |zones| TabState::Local { profile: "p".into(), remote: None, cwd: None, title: None, zones: Some(zones), session: None };
        let unrecorded = TabState::Local { profile: "p".into(), remote: None, cwd: None, title: None, zones: None, session: None };
        let window = |active, tabs| WindowState { width: None, height: None, maximized: false, active, tabs };
        // Sessions 1, 4 and 6 are gone.
        let alive = |session: &ServerSession| ![1, 4, 6].contains(&session.id);

        let three = ZoneNodeState::split(SplitAxis::Horizontal, leaf(2, false), ZoneNodeState::split(SplitAxis::Vertical, leaf(4, true), leaf(5, false)));
        let pruned = drop_lost_zones(
            window(2, vec![single(1), unrecorded.clone(), split(three), split(ZoneNodeState::split(SplitAxis::Vertical, leaf(6, true), leaf(7, false)))]),
            alive,
        )
        .unwrap();
        assert_eq!(pruned.active, 1, "the active tab follows its entry");
        assert_eq!(pruned.tabs[0], unrecorded);
        // The lost zone's sibling takes its place; the active zone is gone,
        // so the first left becomes active.
        let mut first = leaf(2, false);
        first.active = true;
        assert_eq!(pruned.tabs[1], split(ZoneNodeState::split(SplitAxis::Horizontal, first, leaf(5, false))));
        // One zone left: a single-zone tab with that zone's directory.
        assert_eq!(
            pruned.tabs[2],
            TabState::Local { profile: "p".into(), remote: None, cwd: Some("/d7".into()), title: None, zones: None, session: s(7) }
        );

        // The active tab dropped: the one before it becomes active.
        assert_eq!(drop_lost_zones(window(1, vec![single(2), single(1), single(3)]), alive).unwrap().active, 0);
        // Nothing left.
        assert_eq!(drop_lost_zones(window(0, vec![single(1), split(ZoneNodeState::split(SplitAxis::Vertical, leaf(4, true), leaf(6, false)))]), alive), None);

        // A remote tab's sessions are its endpoint's, which this machine's
        // listing doesn't have: its zones stay, recorded or not.
        let remote = Some(crate::config::RemoteTarget::Ssh { host: "A8".into(), user: None, port: None, identity_file: None, ssh_args: Vec::new(), connect_at_launch: false });
        let on_a8 = |tab: TabState| match tab {
            TabState::Local { profile, cwd, title, zones, session, .. } => TabState::Local { profile, remote: remote.clone(), cwd, title, zones, session },
            other => other,
        };
        let tabs = vec![on_a8(single(1)), on_a8(split(ZoneNodeState::split(SplitAxis::Vertical, leaf(4, true), leaf(7, false))))];
        assert_eq!(drop_lost_zones(window(1, tabs.clone()), alive), Some(window(1, tabs)));
    }

    #[test]
    fn collapse_drops_skipped_tabs() {
        let local = |p: &str| SnapshotTab::Local { profile: p.into(), remote: None, cwd: None, title: None, zones: None, session: None };
        let tabs = [local("a"), SnapshotTab::Skip, local("b")];
        let (entries, active) = collapse(&tabs, 2);
        assert_eq!(entries.len(), 2);
        assert_eq!(active, 1);
        // An active message tab maps to the first entry.
        assert_eq!(collapse(&tabs, 1).1, 0);
        assert_eq!(collapse(&[], 0), (vec![], 0));
        let titled = SnapshotTab::Local { profile: "a".into(), remote: None, cwd: None, title: Some("server".into()), zones: None, session: None };
        assert_eq!(
            collapse(&[titled], 0).0,
            vec![TabState::Local { profile: "a".into(), remote: None, cwd: None, title: Some("server".into()), zones: None, session: None }]
        );
    }

    fn window(tabs: usize) -> WindowState {
        WindowState {
            width: None,
            height: None,
            maximized: false,
            active: 0,
            tabs: (0..tabs).map(|i| TabState::Local { profile: format!("p{i}"), remote: None, cwd: None, title: None, zones: None, session: None }).collect(),
        }
    }

    #[test]
    fn store_rules() {
        let mut store = SessionStore::default();
        // Nothing recorded and no lock: nothing to write.
        assert_eq!(store.pending_write(), PendingWrite::Nothing);
        let dir = tmp_dir("store");
        store.lock = Some(Lock::acquire(&dir).unwrap());
        assert_eq!(store.pending_write(), PendingWrite::Nothing);
        store.record(20, 2, window(2));
        store.record(10, 1, window(1));
        let state = store.state();
        assert_eq!(state.windows.len(), 2);
        assert_eq!(state.windows[0].tabs.len(), 1, "ordered by creation");
        assert!(matches!(store.pending_write(), PendingWrite::Write(_)));
        assert_eq!(store.pending_write(), PendingWrite::Nothing, "unchanged");
        // A window closed while another remains is forgotten...
        store.prune(&[20]);
        assert_eq!(store.state().windows.len(), 1);
        // ...but the last one is kept when the app quits with it.
        store.prune(&[]);
        assert_eq!(store.state().windows.len(), 1);
        store.forget(20);
        assert!(store.state().windows.is_empty());
        assert!(matches!(store.pending_write(), PendingWrite::Write(_)), "an emptied store is written");
    }

    #[test]
    fn forgetting_an_unrecorded_window_still_writes() {
        let dir = tmp_dir("forget");
        let mut store = SessionStore { lock: Some(Lock::acquire(&dir).unwrap()), ..Default::default() };
        store.forget(1);
        let PendingWrite::Write(text) = store.pending_write() else { panic!("written") };
        assert!(State::parse(&text).unwrap().windows.is_empty(), "nothing to restore: {text}");
    }

    fn failure(message: &str) -> toml::ser::Error {
        <toml::ser::Error as serde::ser::Error>::custom(message)
    }

    #[test]
    fn an_unserializable_state_is_reported_once_and_never_written() {
        let mut last = Some("kept".to_string());
        let mut reported = None;
        assert_eq!(next_write(&mut last, &mut reported, Err(failure("bad"))), PendingWrite::Unserializable(failure("bad")));
        assert_eq!(last.as_deref(), Some("kept"));
        assert_eq!(next_write(&mut last, &mut reported, Err(failure("bad"))), PendingWrite::StillUnserializable);
        assert_eq!(last.as_deref(), Some("kept"));
    }

    #[test]
    fn a_different_failure_is_reported_again() {
        let (mut last, mut reported) = (None, None);
        next_write(&mut last, &mut reported, Err(failure("a")));
        assert_eq!(next_write(&mut last, &mut reported, Err(failure("b"))), PendingWrite::Unserializable(failure("b")));
    }

    #[test]
    fn the_text_last_written_survives_a_failure() {
        let (mut last, mut reported) = (None, None);
        assert_eq!(next_write(&mut last, &mut reported, Ok("a".into())), PendingWrite::Write("a".into()));
        next_write(&mut last, &mut reported, Err(failure("bad")));
        assert_eq!(next_write(&mut last, &mut reported, Ok("a".into())), PendingWrite::Nothing);
        assert_eq!(next_write(&mut last, &mut reported, Ok("b".into())), PendingWrite::Write("b".into()));
    }

    #[test]
    fn a_failure_after_a_success_is_reported_again() {
        let (mut last, mut reported) = (None, None);
        next_write(&mut last, &mut reported, Err(failure("bad")));
        next_write(&mut last, &mut reported, Ok("a".into()));
        assert_eq!(next_write(&mut last, &mut reported, Err(failure("bad"))), PendingWrite::Unserializable(failure("bad")));
    }

    /// A directory whose name isn't valid UTF-8, which serde refuses to
    /// serialize as a path.
    fn unserializable_dir() -> PathBuf {
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt as _;
            PathBuf::from(std::ffi::OsStr::from_bytes(b"/tmp/\xff"))
        }
        #[cfg(windows)]
        {
            use std::os::windows::ffi::OsStringExt as _;
            PathBuf::from(std::ffi::OsString::from_wide(&[0xD800]))
        }
    }

    #[test]
    fn a_directory_that_isnt_utf8_leaves_the_file_alone() {
        let dir = tmp_dir("non-utf8");
        let mut store = SessionStore { lock: Some(Lock::acquire(&dir).unwrap()), ..Default::default() };
        let with_cwd = |cwd: PathBuf| {
            let mut w = window(1);
            if let TabState::Local { cwd: slot, .. } = &mut w.tabs[0] {
                *slot = Some(cwd);
            }
            w
        };
        store.record(1, 1, with_cwd(unserializable_dir()));
        assert!(matches!(store.pending_write(), PendingWrite::Unserializable(_)));
        assert_eq!(store.pending_write(), PendingWrite::StillUnserializable);
        store.record(1, 1, with_cwd("/tmp".into()));
        let PendingWrite::Write(text) = store.pending_write() else { panic!("written once it serializes") };
        let state = State::parse(&text).unwrap();
        assert!(matches!(&state.windows[0].tabs[0], TabState::Local { cwd: Some(c), .. } if c == Path::new("/tmp")), "{text}");
    }

    #[test]
    fn zone_tree_round_trip_and_fallbacks() {
        let z1 = ZoneId(1);
        let z2 = ZoneId(2);
        let z3 = ZoneId(3);

        let record = |id, cwd: &str, session| (id, ZoneRecord { cwd: Some(cwd.into()), session });

        // Single-zone tree produces None so session.toml is unchanged.
        let single = ZoneTree::new(z1);
        assert_eq!(ZoneNodeState::from_tree(&single, z1, &[record(z1, "/srv", None)].into()), None);

        // 2-zone and nested 3-zone trees round-trip through TOML.
        let mut two = ZoneTree::new(z1);
        two.split(z1, SplitAxis::Horizontal, z2);
        let leaves = [record(z1, "/a", Some(ServerSession { id: 1, key: "k1".into() })), record(z2, "/b", None)];
        let two_state = ZoneNodeState::from_tree(&two, z2, &leaves.into()).unwrap();

        let mut three = two.clone();
        three.split(z2, SplitAxis::Vertical, z3);
        let leaves = [
            record(z1, "/a", None),
            record(z2, "/b", Some(ServerSession { id: 7, key: "k7".into() })),
            record(z3, "/c", None),
        ];
        let three_state = ZoneNodeState::from_tree(&three, z3, &leaves.into()).unwrap();

        let state = State::new(vec![WindowState {
            width: Some(800.0),
            height: Some(600.0),
            maximized: false,
            active: 1,
            tabs: vec![
                TabState::Local {
                    profile: "Shell".into(),
                    remote: None,
                    cwd: Some("/b".into()),
                    title: None,
                    zones: Some(two_state.clone()),
                    session: None,
                },
                TabState::Local {
                    profile: "Shell".into(),
                    remote: None,
                    cwd: Some("/c".into()),
                    title: Some("work".into()),
                    zones: Some(three_state.clone()),
                    session: None,
                },
            ],
        }]);
        let text = state.to_text().expect("serializes");
        assert_eq!(State::parse(&text).unwrap(), state);

        // Resolving a zone tree applies directory fallbacks per zone and reconstructs ZoneTree.
        let config = config();
        let home = Path::new("/home/nobody");
        let tmp = PathBuf::from("/srv/work");
        let gone = PathBuf::from("/srv/gone");
        let mixed = ZoneNodeState::split(
            SplitAxis::Horizontal,
            ZoneNodeState::leaf(Some(tmp.clone()), false),
            ZoneNodeState::split(
                SplitAxis::Vertical,
                ZoneNodeState::leaf(Some(gone), false),
                ZoneNodeState::leaf(None, true),
            ),
        );
        let tab = TabState::Local {
            profile: "Shell".into(),
            remote: None,
            cwd: Some(tmp.clone()),
            title: None,
            zones: Some(mixed),
            session: None,
        };
        let Some(RestoredTab::Local { profile, cwd, title, zones, .. }) =
            resolve_tab(&tab, &config, home, &only(&["/srv/work"]))
        else {
            panic!("expected RestoredTab::Local");
        };
        assert_eq!(profile, 0);
        assert_eq!(cwd, tmp);
        assert_eq!(title, None);
        let (restored_tree, leaves, active_id) = zones.unwrap().into_tree();
        assert_eq!(restored_tree, three);
        assert_eq!(active_id, z3);
        assert_eq!(
            leaves,
            vec![
                (z1, tmp, None),
                (z2, home.to_path_buf(), None),
                (z3, home.to_path_buf(), None),
            ]
        );
    }
}
