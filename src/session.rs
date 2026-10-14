//! Session restore: the windows and tabs recorded while Brindle runs, the
//! state file they are written to, and how a recorded tab is reopened.
//!
//! The state lives in `session.toml` under `$BRINDLE_STATE_DIR`, else
//! `$XDG_STATE_HOME/brindle` (`~/.local/state/brindle`). Only the process
//! holding `session.lock` reads or writes it, so a second Brindle never
//! duplicates the first one's tabs.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};

use crate::config::{Config, TmuxMode};
use crate::tmux::SessionKey;

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
    /// Index into `tabs`. A tmux entry means "that session's tab".
    #[serde(default)]
    pub active: usize,
    #[serde(default)]
    pub tabs: Vec<TabState>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum TabState {
    /// A local tab (including classic tmux): rerun the profile in `cwd`.
    Local {
        profile: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<PathBuf>,
    },
    /// A tmux control-mode session, attached once for all its windows.
    Tmux { profile: String, session: String },
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
        state.windows.retain(|w| !w.tabs.is_empty());
        for window in &mut state.windows {
            window.active = window.active.min(window.tabs.len() - 1);
        }
        Ok(state)
    }

    pub fn to_text(&self) -> String {
        toml::to_string(self).unwrap_or_else(|err| {
            log::error!("could not serialize session state: {err}");
            String::new()
        })
    }
}

/// Where the state file and lock live.
pub fn state_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("BRINDLE_STATE_DIR").filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    dirs::state_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join(".local/state")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("brindle")
}

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

static WRITE_GENERATION: AtomicU64 = AtomicU64::new(0);
static WRITE_LOCK: Mutex<u64> = Mutex::new(0);

/// A ticket ordering writes: a write is skipped if a newer one has already
/// landed, so a slow background write can't replace the final one on quit.
pub fn next_generation() -> u64 {
    WRITE_GENERATION.fetch_add(1, Ordering::SeqCst) + 1
}

/// Replaces `dir/session.toml` with `text` in one step (temp file + rename),
/// readable only by the user.
pub fn write_atomic(dir: &Path, text: &str, generation: u64) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};
    let mut last = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    if generation < *last {
        return Ok(());
    }
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    let tmp = dir.join(format!("{FILE_NAME}.tmp"));
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
    file.write_all(text.as_bytes())?;
    file.sync_all()?;
    std::fs::rename(&tmp, state_path(dir))?;
    *last = generation;
    Ok(())
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
        use std::os::fd::AsRawFd as _;
        use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};
        std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
        let file =
            std::fs::OpenOptions::new().write(true).create(true).truncate(false).mode(0o600).open(dir.join(LOCK_NAME))?;
        // SAFETY: flock only takes a valid descriptor, which `file` owns.
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(Lock { _file: file })
    }
}

// ---- restoring ------------------------------------------------------------------

/// What to open for a recorded tab, after the fallbacks.
#[derive(Debug, Clone, PartialEq)]
pub enum RestoredTab {
    /// Launch profile `profile` in `cwd`.
    Local { profile: usize, cwd: PathBuf },
    /// Attach profile `profile` (a control-mode profile) to `session`.
    Tmux { profile: usize, session: String },
}

/// Resolves a recorded tab against the current config: an unknown profile
/// becomes the default one, a vanished directory the profile's own (or
/// home), and the profile's current tmux mode decides how it opens.
pub fn resolve_tab(tab: &TabState, config: &Config, home: &Path) -> RestoredTab {
    let (name, cwd) = match tab {
        TabState::Local { profile, cwd } => (profile, cwd.as_deref()),
        TabState::Tmux { profile, .. } => (profile, None),
    };
    if config.profile_index(name).is_none() {
        log::warn!("session restore: no profile named {name:?}, using the default profile");
    }
    if let Some((profile, session)) = tmux_target(tab, config) {
        return RestoredTab::Tmux { profile, session };
    }
    let index = config.profile_index(name).unwrap_or_else(|| config.default_profile_index());
    let profile = &config.profiles[index];
    let cwd = cwd
        .filter(|d| d.is_dir())
        .map(Path::to_path_buf)
        .or_else(|| profile.working_directory().filter(|d| d.is_dir()))
        .unwrap_or_else(|| home.to_path_buf());
    RestoredTab::Local { profile: index, cwd }
}

/// The control-mode profile and session a recorded tab attaches, if its
/// profile (or the default one, for an unknown profile) is control mode.
fn tmux_target(tab: &TabState, config: &Config) -> Option<(usize, String)> {
    let (name, session) = match tab {
        TabState::Local { profile, .. } => (profile, None),
        TabState::Tmux { profile, session } => (profile, Some(session)),
    };
    let index = config.profile_index(name).unwrap_or_else(|| config.default_profile_index());
    let profile = &config.profiles[index];
    if profile.tmux != TmuxMode::Control {
        return None;
    }
    // Only a recorded session of this same profile keeps its name;
    // otherwise the profile's own session is used.
    let session = session
        .filter(|_| profile.name.eq_ignore_ascii_case(name))
        .cloned()
        .unwrap_or_else(|| profile.tmux_session_name().to_string());
    Some((index, session))
}

/// Keeps each tmux session in the first window (and at the first tab) that
/// records it, so restoring attaches it once. Windows left with nothing to
/// restore are dropped; `active` follows its entry, or resets to the first
/// when that entry was dropped.
pub fn dedupe_windows(windows: Vec<WindowState>, config: &Config) -> Vec<WindowState> {
    let mut seen = HashSet::new();
    windows
        .into_iter()
        .filter_map(|mut window| {
            let mut active = None;
            let mut tabs = Vec::new();
            for (ix, tab) in std::mem::take(&mut window.tabs).into_iter().enumerate() {
                if let Some((profile, session)) = tmux_target(&tab, config)
                    && !seen.insert(SessionKey::new(&config.profiles[profile], &session))
                {
                    continue;
                }
                if ix == window.active {
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
        })
        .collect()
}

// ---- recording --------------------------------------------------------------------

/// One tab as seen when taking a snapshot.
#[derive(Debug, Clone, PartialEq)]
pub enum SnapshotTab {
    Local { profile: String, cwd: Option<PathBuf> },
    /// A tab of a control-mode session; `key` identifies the session.
    Tmux { profile: String, session: String, key: u64 },
    /// Not recorded (message tabs).
    Skip,
}

/// Turns a window's tabs into recorded entries: one entry per tmux session
/// at its first tab, skipped tabs dropped. Returns the entries and the
/// entry the active tab maps to.
pub fn collapse(tabs: &[SnapshotTab], active: usize) -> (Vec<TabState>, usize) {
    let mut entries = Vec::new();
    let mut sessions: HashMap<u64, usize> = HashMap::new();
    let mut active_entry = None;
    for (ix, tab) in tabs.iter().enumerate() {
        let entry = match tab {
            SnapshotTab::Local { profile, cwd } => {
                entries.push(TabState::Local { profile: profile.clone(), cwd: cwd.clone() });
                Some(entries.len() - 1)
            }
            SnapshotTab::Tmux { profile, session, key } => Some(*sessions.entry(*key).or_insert_with(|| {
                entries.push(TabState::Tmux { profile: profile.clone(), session: session.clone() });
                entries.len() - 1
            })),
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
    last_written: Option<String>,
    lock: Option<Lock>,
    lock_failed: bool,
}

impl gpui::Global for SessionStore {}

impl SessionStore {
    /// Whether this process records the session: the setting is on and it
    /// owns the state directory (taking the lock on first use).
    pub fn is_owner(&mut self, enabled: bool) -> bool {
        if !enabled {
            return false;
        }
        if self.lock.is_none() && !self.lock_failed {
            match Lock::acquire(&state_dir()) {
                Ok(lock) => self.lock = Some(lock),
                Err(err) => {
                    self.lock_failed = true;
                    log::info!(
                        "another Brindle owns the session in {} ({err}); not restoring or recording",
                        state_dir().display()
                    );
                }
            }
        }
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

    /// The text to write, if it changed since the last write.
    pub fn pending_write(&mut self) -> Option<String> {
        if !self.touched || self.lock.is_none() {
            return None;
        }
        let text = self.state().to_text();
        if self.last_written.as_ref() == Some(&text) {
            return None;
        }
        self.last_written = Some(text.clone());
        Some(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("brindle-session-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        dir
    }

    fn sample() -> State {
        State::new(vec![
            WindowState {
                width: Some(1000.0),
                height: Some(660.0),
                maximized: false,
                active: 1,
                tabs: vec![
                    TabState::Local { profile: "Shell".into(), cwd: Some("/tmp".into()) },
                    TabState::Tmux { profile: "tmux".into(), session: "main".into() },
                    TabState::Local { profile: "Shell".into(), cwd: None },
                ],
            },
            WindowState { width: None, height: None, maximized: true, active: 0, tabs: vec![TabState::Local { profile: "x".into(), cwd: None }] },
        ])
    }

    #[test]
    fn round_trip() {
        let text = sample().to_text();
        assert!(text.contains("version = 1"), "{text}");
        assert!(text.contains("kind = \"tmux\""), "{text}");
        assert_eq!(State::parse(&text).unwrap(), sample());
    }

    #[test]
    fn rejects_bad_files() {
        assert!(State::parse("version = 2\n[[windows]]\n").unwrap_err().contains("version 2"));
        assert!(State::parse("this is not toml [").unwrap_err().contains("not a valid"));
        assert!(State::parse("[[windows]]\nactive = 0\n").unwrap_err().contains("no format version"));
        let unknown_kind = "version = 1\n[[windows]]\n[[windows.tabs]]\nkind = \"ssh\"\nprofile = \"x\"\n";
        assert!(State::parse(unknown_kind).is_err());
        assert!(State::parse("version = 1\n").unwrap().windows.is_empty());
        // Windows without tabs are dropped, then nothing is left.
        assert!(State::parse("version = 1\n[[windows]]\nactive = 0\n").unwrap().windows.is_empty());
    }

    #[test]
    fn active_is_clamped_and_unknown_fields_ignored() {
        let text = "version = 1\nfuture = true\n[[windows]]\nactive = 7\n[[windows.tabs]]\nkind = \"local\"\nprofile = \"a\"\n";
        let state = State::parse(text).unwrap();
        assert_eq!(state.windows[0].active, 0);
    }

    #[test]
    fn atomic_write_and_load() {
        let dir = tmp_dir("write");
        assert_eq!(load(&dir), Ok(None));
        write_atomic(&dir, &sample().to_text(), next_generation()).unwrap();
        assert_eq!(load(&dir), Ok(Some(sample())));
        assert!(!dir.join("session.toml.tmp").exists());
        use std::os::unix::fs::PermissionsExt as _;
        let mode = std::fs::metadata(state_path(&dir)).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        // An older write that lands late doesn't replace a newer one.
        let old = next_generation();
        let new = next_generation();
        write_atomic(&dir, "version = 1\n", new).unwrap();
        write_atomic(&dir, &sample().to_text(), old).unwrap();
        assert_eq!(std::fs::read_to_string(state_path(&dir)).unwrap(), "version = 1\n");
        // A state with no windows is a normal launch, not an error.
        write_atomic(&dir, "version = 1\nwindows = []\n", next_generation()).unwrap();
        assert_eq!(load(&dir), Ok(None));
        std::fs::write(state_path(&dir), "garbage").unwrap();
        assert!(load(&dir).unwrap_err().contains("ignoring session state"));
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn lock_is_exclusive() {
        let dir = tmp_dir("lock");
        let first = Lock::acquire(&dir).unwrap();
        assert!(Lock::acquire(&dir).is_err());
        drop(first);
        assert!(Lock::acquire(&dir).is_ok());
        std::fs::remove_dir_all(&dir).ok();
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
            [[profiles]]
            name = "tmux"
            tmux = "control"
            tmux_session = "work"
            [[profiles]]
            name = "classic"
            tmux = "plain"
            "#,
        )
        .unwrap()
    }

    #[test]
    fn resolve_fallbacks() {
        let config = config();
        let home = Path::new("/home/nobody");
        let local = |profile: &str, cwd: Option<&str>| TabState::Local { profile: profile.into(), cwd: cwd.map(PathBuf::from) };
        // Recorded directory kept.
        assert_eq!(resolve_tab(&local("Shell", Some("/tmp")), &config, home), RestoredTab::Local { profile: 0, cwd: "/tmp".into() });
        // Missing profile: the default one.
        assert_eq!(resolve_tab(&local("Gone", Some("/tmp")), &config, home), RestoredTab::Local { profile: 0, cwd: "/tmp".into() });
        // Deleted directory: home, or the profile's own directory.
        let gone = "/nonexistent/brindle-dir";
        assert_eq!(resolve_tab(&local("Shell", Some(gone)), &config, home), RestoredTab::Local { profile: 0, cwd: home.into() });
        assert_eq!(resolve_tab(&local("shell", None), &config, home), RestoredTab::Local { profile: 0, cwd: home.into() });
        assert_eq!(resolve_tab(&local("Pinned", Some(gone)), &config, home), RestoredTab::Local { profile: 1, cwd: "/".into() });
        // Classic tmux is a local tab.
        assert_eq!(resolve_tab(&local("classic", Some("/tmp")), &config, home), RestoredTab::Local { profile: 3, cwd: "/tmp".into() });
        // Control mode keeps the recorded (possibly renamed) session.
        let tmux = |profile: &str, session: &str| TabState::Tmux { profile: profile.into(), session: session.into() };
        assert_eq!(resolve_tab(&tmux("TMUX", "renamed"), &config, home), RestoredTab::Tmux { profile: 2, session: "renamed".into() });
        // A tmux entry whose profile isn't a control profile any more opens as a tab.
        assert_eq!(resolve_tab(&tmux("Shell", "x"), &config, home), RestoredTab::Local { profile: 0, cwd: home.into() });
        // A local entry whose profile is now control mode attaches its session.
        assert_eq!(resolve_tab(&local("tmux", Some("/tmp")), &config, home), RestoredTab::Tmux { profile: 2, session: "work".into() });
    }

    #[test]
    fn dedupe_keeps_each_session_once() {
        let config = config();
        let tmux = |session: &str| TabState::Tmux { profile: "tmux".into(), session: session.into() };
        let local = || TabState::Local { profile: "Shell".into(), cwd: None };
        let window = |active: usize, tabs: Vec<TabState>| WindowState { width: None, height: None, maximized: false, active, tabs };
        let windows = vec![
            window(0, vec![tmux("main")]),
            // A duplicate before the active tab: `active` follows its entry.
            window(2, vec![tmux("main"), local(), local()]),
            // Only a duplicate: the window goes.
            window(0, vec![tmux("main")]),
            // The active entry is the duplicate: the first tab becomes active.
            window(1, vec![local(), tmux("main"), tmux("other")]),
        ];
        let deduped = dedupe_windows(windows, &config);
        assert_eq!(
            deduped,
            vec![
                window(0, vec![tmux("main")]),
                window(1, vec![local(), local()]),
                window(0, vec![local(), tmux("other")]),
            ]
        );
    }

    #[test]
    fn dedupe_matches_profiles_on_the_same_server() {
        let mut config = config();
        // A second control profile for the same server and session.
        let mut alias = config.profiles[2].clone();
        alias.name = "alias".into();
        alias.tmux_session = Some("main".into());
        config.profiles.push(alias);
        let windows = vec![
            WindowState { width: None, height: None, maximized: false, active: 0, tabs: vec![TabState::Tmux { profile: "tmux".into(), session: "main".into() }] },
            WindowState { width: None, height: None, maximized: false, active: 0, tabs: vec![TabState::Tmux { profile: "alias".into(), session: "main".into() }] },
        ];
        assert_eq!(dedupe_windows(windows, &config).len(), 1);
    }

    #[test]
    fn collapse_sessions() {
        let local = |p: &str| SnapshotTab::Local { profile: p.into(), cwd: None };
        let tmux = |key| SnapshotTab::Tmux { profile: "tmux".into(), session: format!("s{key}"), key };
        let tabs = [local("a"), tmux(1), tmux(1), SnapshotTab::Skip, tmux(2), tmux(1), local("b")];
        let (entries, active) = collapse(&tabs, 5);
        assert_eq!(entries.len(), 4);
        assert_eq!(entries[1], TabState::Tmux { profile: "tmux".into(), session: "s1".into() });
        assert_eq!(entries[2], TabState::Tmux { profile: "tmux".into(), session: "s2".into() });
        assert_eq!(active, 1);
        assert_eq!(collapse(&tabs, 6).1, 3);
        // An active message tab maps to the first entry.
        assert_eq!(collapse(&tabs, 3).1, 0);
        assert_eq!(collapse(&[], 0), (vec![], 0));
    }

    fn window(tabs: usize) -> WindowState {
        WindowState {
            width: None,
            height: None,
            maximized: false,
            active: 0,
            tabs: (0..tabs).map(|i| TabState::Local { profile: format!("p{i}"), cwd: None }).collect(),
        }
    }

    #[test]
    fn store_rules() {
        let mut store = SessionStore::default();
        // Nothing recorded and no lock: nothing to write.
        assert_eq!(store.pending_write(), None);
        store.lock = Some(Lock::acquire(&tmp_dir("store")).unwrap());
        assert_eq!(store.pending_write(), None);
        store.record(20, 2, window(2));
        store.record(10, 1, window(1));
        let state = store.state();
        assert_eq!(state.windows.len(), 2);
        assert_eq!(state.windows[0].tabs.len(), 1, "ordered by creation");
        assert!(store.pending_write().is_some());
        assert_eq!(store.pending_write(), None, "unchanged");
        // A window closed while another remains is forgotten...
        store.prune(&[20]);
        assert_eq!(store.state().windows.len(), 1);
        // ...but the last one is kept when the app quits with it.
        store.prune(&[]);
        assert_eq!(store.state().windows.len(), 1);
        store.forget(20);
        assert!(store.state().windows.is_empty());
        assert!(store.pending_write().is_some(), "an emptied store is written");
        std::fs::remove_dir_all(tmp_dir("store")).ok();
    }

    #[test]
    fn forgetting_an_unrecorded_window_still_writes() {
        let dir = tmp_dir("forget");
        let mut store = SessionStore { lock: Some(Lock::acquire(&dir).unwrap()), ..Default::default() };
        store.forget(1);
        let text = store.pending_write().expect("written");
        assert!(State::parse(&text).unwrap().windows.is_empty(), "nothing to restore: {text}");
        std::fs::remove_dir_all(&dir).ok();
    }
}
