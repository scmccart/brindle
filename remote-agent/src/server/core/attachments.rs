//! The attachment store's decisions (OpenSpec change
//! `agents-prompt-context`, design decisions 1, 2 and 4): where the cache
//! and an agent session's attachments directory are, what an upload,
//! a prompt, a discard, an adoption and a session's end write and
//! remove, and which store directories a sweep removes.
//!
//! Pure (`fcis R1`–`R3`): the core's shell (`agents.rs`) gathers the
//! environment, the clock, what a directory lists and what a session's log
//! references, calls [`plan`] or [`sweep`], and runs the [`Step`]s through
//! `server/platform/`'s store items.
//!
//! A session's store is `<attachments directory>/<store name>`, its texts
//! the files `1`, `2`, … in it and its keep time the [`MARKER`] file written
//! when the session ends. What a session holds of it (the directory, the
//! name, the next id) is its [`Store`], checkpointed with it.

use std::collections::{BTreeMap, BTreeSet};

use brindle_transcript::{ATTACHMENT_TEXT, AttachmentId};

use crate::acp::reduce::Store;
use crate::acp::reduce::attach::{absolute, join};
pub use crate::server::platform::Listed;

/// The most kept text one session's store holds.
pub const STORE_LIMIT: u64 = 16 * 1024 * 1024;
/// How many days a store stays after its session ends, by default.
pub const DEFAULT_KEEP_DAYS: u32 = 14;
/// The longest keep time a spawn may ask for.
pub const MAX_KEEP_DAYS: u32 = 3650;
/// The file in a store directory that holds when it may be removed.
pub const MARKER: &str = "expires";
/// The attachments directory's name in the cache directory.
pub const ATTACHMENTS: &str = "attachments";
const DAY: u64 = 24 * 60 * 60;

/// The cache directory (decision 1): `BRINDLE_CACHE_DIR`, then on Windows
/// `%LOCALAPPDATA%\brindle\cache`, elsewhere `$XDG_CACHE_HOME/brindle` (an
/// absolute one) and then `~/.cache/brindle`. `var` reads the core's
/// environment (empty counts as unset); `home` is its home.
pub fn cache_dir(var: impl Fn(&str) -> Option<String>, home: Option<&str>, windows: bool) -> Option<String> {
    let set = |name: &str| var(name).filter(|value| !value.is_empty());
    if let Some(dir) = set("BRINDLE_CACHE_DIR") {
        return Some(dir);
    }
    if windows {
        return set("LOCALAPPDATA").map(|base| join(&join(&base, "brindle", true), "cache", true));
    }
    if let Some(xdg) = set("XDG_CACHE_HOME").filter(|xdg| absolute(xdg, false)) {
        return Some(join(&xdg, "brindle", false));
    }
    home.filter(|home| !home.is_empty())
        .map(|home| join(&join(home, ".cache", false), "brindle", false))
}

/// The attachments directory a session keeps its store in: the spawn's
/// (`~` and `~/…` with the agent's home), else the cache directory's
/// [`ATTACHMENTS`]. `Err` says why it has none; a relative one is kept as
/// given and refuses attaches ([`plan`]).
pub fn attachments_root(
    configured: Option<&str>,
    cache: Option<&str>,
    home: Option<&str>,
    windows: bool,
) -> Result<String, String> {
    match configured.filter(|dir| !dir.is_empty()) {
        Some(dir) => {
            let tilde = dir == "~" || dir.starts_with("~/") || (windows && dir.starts_with("~\\"));
            match (tilde, home) {
                (true, Some(home)) if dir.len() > 2 => Ok(join(home, &dir[2..], windows)),
                (true, Some(home)) => Ok(home.to_owned()),
                (true, None) => Err(format!("the attachments directory {dir:?} names a home this host doesn't have")),
                (false, _) => Ok(dir.to_owned()),
            }
        }
        None => cache
            .map(|cache| join(cache, ATTACHMENTS, windows))
            .ok_or_else(|| "this host has no cache directory (no HOME)".to_owned()),
    }
}

/// A spawn's keep time, bounded: none asked is [`DEFAULT_KEEP_DAYS`].
pub fn keep_days(asked: Option<u32>) -> u32 {
    asked.map_or(DEFAULT_KEEP_DAYS, |days| days.min(MAX_KEEP_DAYS))
}

/// A store's name: the day it was made (UTC, from `now`, seconds since
/// the epoch) and 64 random bits, `YYYYMMDD-` and 16 hex digits.
pub fn store_name(now: u64, random: u64) -> String {
    let (year, month, day) = civil(now / DAY);
    format!("{year:04}{month:02}{day:02}-{random:016x}")
}

/// Whether `name` is a store's ([`store_name`]): the only directories a
/// sweep looks at, so it never removes anything else in a directory a
/// config names.
pub fn is_store_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    bytes.len() == 25
        && bytes[..8].iter().all(u8::is_ascii_digit)
        && bytes[8] == b'-'
        && bytes[9..].iter().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

/// The day `days` after 1970-01-01 (Howard Hinnant's `civil_from_days`).
fn civil(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    (year, month, day)
}

/// A marker's text: the time, in seconds since the epoch, after which the
/// store may be removed.
pub fn marker_text(until: u64) -> String {
    format!("{until}\n")
}

/// A marker read back; `None` when it isn't one.
pub fn parse_marker(text: &str) -> Option<u64> {
    text.trim().parse().ok()
}

/// An upload a connection holds until a prompt takes it (decision 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Upload {
    pub session: u32,
    pub id: AttachmentId,
    pub name: String,
    pub bytes: u64,
}

/// Every connection's untaken uploads, by connection `C` and the number the
/// client gave each.
#[derive(Debug)]
pub struct Uploads<C> {
    held: BTreeMap<(C, u64), Upload>,
}

impl<C> Default for Uploads<C> {
    fn default() -> Self {
        Self { held: BTreeMap::new() }
    }
}

/// Why an upload or a prompt's uploads are refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UploadRefusal {
    /// The number is already one of the connection's untaken uploads.
    Taken { upload: u64 },
    /// The connection holds no such upload (for this session).
    Missing { upload: u64 },
}

impl std::fmt::Display for UploadRefusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Taken { upload } => write!(f, "upload {upload} is already on this connection"),
            Self::Missing { upload } => write!(f, "this connection holds no upload {upload}"),
        }
    }
}

impl<C: Ord + Copy> Uploads<C> {
    /// Whether `client` holds upload number `upload`.
    pub fn refusal(&self, client: C, upload: u64) -> Option<UploadRefusal> {
        self.held
            .contains_key(&(client, upload))
            .then_some(UploadRefusal::Taken { upload })
    }

    pub fn add(&mut self, client: C, upload: u64, held: Upload) {
        self.held.insert((client, upload), held);
    }

    /// The uploads a prompt of `session` names, in its order, taken from
    /// `client`: all of them, or none when one is missing (or another
    /// session's). A number named twice is missing the second time.
    pub fn take(&mut self, client: C, session: u32, uploads: &[u64]) -> Result<Vec<Upload>, UploadRefusal> {
        let mut seen = BTreeSet::new();
        for upload in uploads {
            let held = self.held.get(&(client, *upload)).filter(|held| held.session == session);
            if held.is_none() || !seen.insert(*upload) {
                return Err(UploadRefusal::Missing { upload: *upload });
            }
        }
        Ok(uploads
            .iter()
            .filter_map(|upload| self.held.remove(&(client, *upload)))
            .collect())
    }

    /// What `client` held, now that it closed or detached.
    pub fn drop_client(&mut self, client: C) -> Vec<Upload> {
        let (gone, kept) = std::mem::take(&mut self.held)
            .into_iter()
            .partition(|((holder, _), _)| *holder == client);
        self.held = kept;
        gone.into_values().collect()
    }

    /// What was uploaded for `session`, now that it ended.
    pub fn drop_session(&mut self, session: u32) -> Vec<Upload> {
        let (gone, kept) = std::mem::take(&mut self.held)
            .into_iter()
            .partition(|(_, held)| held.session == session);
        self.held = kept;
        gone.into_values().collect()
    }

    /// The ids of `session`'s untaken uploads: still referenced.
    pub fn of_session(&self, session: u32) -> impl Iterator<Item = AttachmentId> + '_ {
        self.held
            .values()
            .filter(move |held| held.session == session)
            .map(|held| held.id)
    }
}

/// What [`plan`] is given of one session's store.
#[derive(Debug, Clone, Copy)]
pub struct StoreState<'a> {
    pub store: &'a Store,
    /// Whether the shell found the store's directory, and the attachments
    /// directory it is in, private to the user (`Err` the reason); checked
    /// at an attach only.
    pub private: Result<(), &'a str>,
    /// The texts the store holds, with their sizes.
    pub kept: &'a BTreeMap<AttachmentId, u64>,
    /// The texts the session's log, its queue slot and its untaken uploads
    /// name.
    pub referenced: &'a BTreeSet<AttachmentId>,
}

/// What happened to a session's store.
#[derive(Debug, Clone, Copy)]
pub enum StoreEvent<'a> {
    /// `AGENT_ATTACH`, given the id the store's [`Store::take_id`] gave.
    Attach { id: AttachmentId, bytes: u64 },
    /// What the session references changed: a prompt took uploads or was
    /// refused, a queued or held prompt was discarded or sent, log entries
    /// were trimmed, a connection closed. Unreferenced texts go.
    Changed,
    /// An adopting core listed the store's directory: its texts are known
    /// again, and those nothing references (uploads no prompt took before
    /// the restart) go.
    Adopted { listing: &'a [Listed] },
    /// The session's record is dropped (never an agent's exit, decision 4):
    /// the marker says when the store may go, or it goes at once with 0
    /// days.
    Ended,
}

/// What the shell does to the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// Make the attachments directory and the store's, private.
    MakeDirs { root: String, dir: String },
    /// Write the upload's text, once, read-only.
    Write { id: AttachmentId, file: String },
    /// A kept text, with its size, known again after an adoption.
    Know { id: AttachmentId, bytes: u64 },
    Remove { id: AttachmentId, file: String },
    /// Write the marker with this text.
    Mark { file: String, text: String },
    RemoveDir { dir: String },
}

/// Why an attach is refused (decision 11), as the client is told.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    TooLong { bytes: u64 },
    StoreFull { held: u64 },
    /// The session has no store directory, or it isn't absolute.
    NoStore { why: String },
    DirectoryNotPrivate { path: String, why: String },
}

impl std::fmt::Display for Refusal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TooLong { bytes } => write!(
                f,
                "the attached text is {bytes} bytes, over the limit of {ATTACHMENT_TEXT}"
            ),
            Self::StoreFull { held } => write!(
                f,
                "the conversation's attachments hold {held} bytes, and the limit is {STORE_LIMIT}"
            ),
            Self::NoStore { why } => write!(f, "attached text can't be kept: {why}"),
            Self::DirectoryNotPrivate { path, why } => {
                write!(f, "the attachments directory {path} isn't private: {why}")
            }
        }
    }
}

/// What `event` does to the store `state` describes, at `now` (seconds
/// since the epoch); an attach may be refused.
pub fn plan(state: StoreState<'_>, event: StoreEvent<'_>, now: u64) -> Result<Vec<Step>, Refusal> {
    let store = state.store;
    match event {
        StoreEvent::Attach { id, bytes } => {
            if bytes > ATTACHMENT_TEXT as u64 {
                return Err(Refusal::TooLong { bytes });
            }
            let held: u64 = state.kept.values().sum();
            if held + bytes > STORE_LIMIT {
                return Err(Refusal::StoreFull { held });
            }
            if store.root.is_empty() {
                return Err(Refusal::NoStore {
                    why: "the session has no attachments directory".to_owned(),
                });
            }
            if !absolute(&store.root, store.windows) {
                return Err(Refusal::NoStore {
                    why: format!("the attachments directory {} isn't absolute", store.root),
                });
            }
            let (Some(dir), Some(file)) = (store.dir(), store.file(id)) else {
                return Err(Refusal::NoStore {
                    why: "the session's store has no name".to_owned(),
                });
            };
            if let Err(why) = state.private {
                return Err(Refusal::DirectoryNotPrivate {
                    path: store.root.clone(),
                    why: why.to_owned(),
                });
            }
            Ok(vec![
                Step::MakeDirs {
                    root: store.root.clone(),
                    dir,
                },
                Step::Write { id, file },
            ])
        }
        StoreEvent::Changed => Ok(unreferenced(store, state.kept.keys().copied(), state.referenced)),
        StoreEvent::Adopted { listing } => {
            let ids: Vec<(AttachmentId, u64)> = listing
                .iter()
                .filter(|listed| !listed.dir)
                .filter_map(|listed| file_id(&listed.name).map(|id| (id, listed.bytes)))
                .collect();
            let mut steps: Vec<Step> = ids
                .iter()
                .filter(|(id, _)| state.referenced.contains(id))
                .map(|(id, bytes)| Step::Know { id: *id, bytes: *bytes })
                .collect();
            steps.extend(unreferenced(store, ids.iter().map(|(id, _)| *id), state.referenced));
            Ok(steps)
        }
        StoreEvent::Ended => Ok(match store.dir() {
            None => Vec::new(),
            Some(dir) if store.keep_days == 0 => vec![Step::RemoveDir { dir }],
            Some(dir) => vec![Step::Mark {
                file: join(&dir, MARKER, store.windows),
                text: marker_text(now + u64::from(store.keep_days) * DAY),
            }],
        }),
    }
}

/// The removals of `kept` texts nothing references.
fn unreferenced(
    store: &Store,
    kept: impl Iterator<Item = AttachmentId>,
    referenced: &BTreeSet<AttachmentId>,
) -> Vec<Step> {
    kept.filter(|id| !referenced.contains(id))
        .filter_map(|id| store.file(id).map(|file| Step::Remove { id, file }))
        .collect()
}

/// The id a store's file is named by: a decimal number from 1, written
/// as [`Store::file`] names it.
fn file_id(name: &str) -> Option<AttachmentId> {
    let id: u64 = name.parse().ok()?;
    (id > 0 && id.to_string() == name).then_some(AttachmentId(id))
}

/// A store directory a sweep found in an attachments directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// Its path.
    pub dir: String,
    /// Its marker's time, when it has a readable one.
    pub marker: Option<u64>,
    /// When it, or the newest file in it, was last written.
    pub newest: Option<u64>,
}

/// The store directories of `found` a sweep at `now` removes (decision 4):
/// each whose marker's time has passed, and each with no marker (its core
/// died before its session ended) whose newest file is older than
/// [`DEFAULT_KEEP_DAYS`]; never a live session's.
pub fn sweep(found: &[Found], live: &BTreeSet<String>, now: u64) -> Vec<Step> {
    found
        .iter()
        .filter(|found| !live.contains(&found.dir))
        .filter(|found| {
            let due = found
                .marker
                .or_else(|| found.newest.map(|newest| newest + u64::from(DEFAULT_KEEP_DAYS) * DAY));
            due.is_some_and(|due| due <= now)
        })
        .map(|found| Step::RemoveDir { dir: found.dir.clone() })
        .collect()
}

/// When to sweep which attachments directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SweepAt<'a> {
    /// A core's start: the default attachments directory.
    CoreStart { default: Option<&'a str> },
    /// A session starting or ending: its attachments directory, which a
    /// config may have named.
    Session { root: &'a str },
}

/// The attachments directory a sweep at `at` lists, if any.
pub fn sweep_root(at: SweepAt<'_>) -> Option<&str> {
    match at {
        SweepAt::CoreStart { default } => default,
        SweepAt::Session { root } => (!root.is_empty()).then_some(root),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let pairs: BTreeMap<String, String> = pairs.iter().map(|(k, v)| ((*k).into(), (*v).into())).collect();
        move |name| pairs.get(name).cloned()
    }

    fn store() -> Store {
        Store {
            root: "/c/attachments".into(),
            name: "20261128-00000000000000ab".into(),
            next: 1,
            keep_days: 14,
            home: Some("/home/u".into()),
            windows: false,
        }
    }

    const DIR: &str = "/c/attachments/20261128-00000000000000ab";

    fn file(id: u64) -> String {
        format!("{DIR}/{id}")
    }

    fn state<'a>(
        store: &'a Store,
        kept: &'a BTreeMap<AttachmentId, u64>,
        referenced: &'a BTreeSet<AttachmentId>,
    ) -> StoreState<'a> {
        StoreState {
            store,
            private: Ok(()),
            kept,
            referenced,
        }
    }

    fn ids(ids: &[u64]) -> BTreeSet<AttachmentId> {
        ids.iter().map(|id| AttachmentId(*id)).collect()
    }

    fn kept(pairs: &[(u64, u64)]) -> BTreeMap<AttachmentId, u64> {
        pairs.iter().map(|(id, bytes)| (AttachmentId(*id), *bytes)).collect()
    }

    // --- where -----------------------------------------------------------

    #[test]
    fn the_cache_directory_follows_each_rule_on_each_platform() {
        let home = Some("/home/u");
        assert_eq!(cache_dir(env(&[("BRINDLE_CACHE_DIR", "/t/c")]), home, false).as_deref(), Some("/t/c"));
        assert_eq!(
            cache_dir(env(&[("BRINDLE_CACHE_DIR", "C:\\t"), ("LOCALAPPDATA", "C:\\L")]), None, true).as_deref(),
            Some("C:\\t")
        );
        assert_eq!(cache_dir(env(&[("XDG_CACHE_HOME", "/x")]), home, false).as_deref(), Some("/x/brindle"));
        // A relative XDG_CACHE_HOME is ignored, as the spec says.
        assert_eq!(cache_dir(env(&[("XDG_CACHE_HOME", "x")]), home, false).as_deref(), Some("/home/u/.cache/brindle"));
        assert_eq!(cache_dir(env(&[("BRINDLE_CACHE_DIR", "")]), home, false).as_deref(), Some("/home/u/.cache/brindle"));
        assert_eq!(cache_dir(env(&[]), None, false), None);
        assert_eq!(
            cache_dir(env(&[("LOCALAPPDATA", "C:\\Users\\u\\AppData\\Local"), ("XDG_CACHE_HOME", "/x")]), home, true).as_deref(),
            Some("C:\\Users\\u\\AppData\\Local\\brindle\\cache")
        );
        assert_eq!(cache_dir(env(&[]), Some("C:\\Users\\u"), true), None);
    }

    #[test]
    fn a_sessions_attachments_directory_is_its_spawns_else_the_caches() {
        let cache = Some("/home/u/.cache/brindle");
        let home = Some("/home/u");
        assert_eq!(attachments_root(None, cache, home, false), Ok("/home/u/.cache/brindle/attachments".into()));
        assert_eq!(attachments_root(Some(""), cache, home, false), Ok("/home/u/.cache/brindle/attachments".into()));
        assert_eq!(attachments_root(Some("~/att"), cache, home, false), Ok("/home/u/att".into()));
        assert_eq!(attachments_root(Some("/srv/att"), cache, home, false), Ok("/srv/att".into()));
        // A relative one is kept, and refuses attaches.
        assert_eq!(attachments_root(Some("att"), cache, home, false), Ok("att".into()));
        assert!(attachments_root(Some("~/att"), cache, None, false).is_err());
        assert!(attachments_root(None, None, home, false).is_err());
        assert_eq!(
            attachments_root(Some("~\\att"), None, Some("C:\\Users\\u"), true),
            Ok("C:\\Users\\u\\att".into())
        );
        assert_eq!(
            attachments_root(None, Some("C:\\L\\brindle\\cache"), None, true),
            Ok("C:\\L\\brindle\\cache\\attachments".into())
        );
    }

    #[test]
    fn keep_days_default_to_14_and_are_bounded() {
        assert_eq!(keep_days(None), 14);
        assert_eq!(keep_days(Some(0)), 0);
        assert_eq!(keep_days(Some(3650)), 3650);
        assert_eq!(keep_days(Some(9999)), 3650);
    }

    #[test]
    fn a_store_is_named_by_its_day_and_random_bits() {
        // 2026-11-28T12:00:00Z.
        let name = store_name(1_795_867_200, 0xab);
        assert_eq!(name, "20261128-00000000000000ab");
        assert!(is_store_name(&name));
        assert_eq!(store_name(0, u64::MAX), "19700101-ffffffffffffffff");
        // 2000-02-29 (a leap day) and 2100-03-01.
        assert_eq!(&store_name(951_782_400, 0)[..8], "20000229");
        assert_eq!(&store_name(4_107_542_400, 0)[..8], "21000301");
        for other in ["20261128-00000000000000AB", "2026112800000000000000ab0", "x", "20261128-0000000000000ab", "expires", "1"] {
            assert!(!is_store_name(other), "{other}");
        }
    }

    #[test]
    fn a_marker_round_trips() {
        assert_eq!(parse_marker(&marker_text(42)), Some(42));
        assert_eq!(parse_marker("soon"), None);
    }

    // --- uploads ---------------------------------------------------------

    fn upload(session: u32, id: u64) -> Upload {
        Upload {
            session,
            id: AttachmentId(id),
            name: format!("u{id}"),
            bytes: 3,
        }
    }

    #[test]
    fn a_prompt_takes_all_of_its_uploads_or_none() {
        let mut uploads = Uploads::default();
        uploads.add(1, 1, upload(5, 1));
        uploads.add(1, 2, upload(5, 2));
        uploads.add(2, 1, upload(5, 3));
        assert_eq!(uploads.refusal(1, 1), Some(UploadRefusal::Taken { upload: 1 }));
        assert_eq!(UploadRefusal::Taken { upload: 1 }.to_string(), "upload 1 is already on this connection");
        assert_eq!(uploads.refusal(1, 3), None);
        // One missing: none is taken.
        assert_eq!(uploads.take(1, 5, &[1, 7]), Err(UploadRefusal::Missing { upload: 7 }));
        assert_eq!(UploadRefusal::Missing { upload: 7 }.to_string(), "this connection holds no upload 7");
        // Another session's, or one named twice, is missing too.
        assert_eq!(uploads.take(1, 6, &[1]), Err(UploadRefusal::Missing { upload: 1 }));
        assert_eq!(uploads.take(1, 5, &[1, 1]), Err(UploadRefusal::Missing { upload: 1 }));
        assert_eq!(uploads.of_session(5).count(), 3);
        // In the prompt's order.
        assert_eq!(uploads.take(1, 5, &[2, 1]), Ok(vec![upload(5, 2), upload(5, 1)]));
        assert_eq!(uploads.take(1, 5, &[1]), Err(UploadRefusal::Missing { upload: 1 }));
        assert_eq!(uploads.of_session(5).collect::<Vec<_>>(), [AttachmentId(3)]);
    }

    #[test]
    fn a_closing_connection_or_ending_session_gives_up_its_uploads() {
        let mut uploads = Uploads::default();
        uploads.add(1, 1, upload(5, 1));
        uploads.add(2, 1, upload(5, 2));
        uploads.add(2, 2, upload(6, 3));
        assert_eq!(uploads.drop_client(2), [upload(5, 2), upload(6, 3)]);
        assert_eq!(uploads.drop_client(2), []);
        assert_eq!(uploads.drop_session(5), [upload(5, 1)]);
        assert_eq!(uploads.of_session(5).count(), 0);
    }

    // --- plan ------------------------------------------------------------

    #[test]
    fn an_attach_makes_the_directories_and_writes_its_file() {
        let (store, kept, referenced) = (store(), kept(&[]), ids(&[]));
        assert_eq!(
            plan(state(&store, &kept, &referenced), StoreEvent::Attach { id: AttachmentId(1), bytes: 3 }, 0),
            Ok(vec![
                Step::MakeDirs {
                    root: "/c/attachments".into(),
                    dir: DIR.into()
                },
                Step::Write {
                    id: AttachmentId(1),
                    file: file(1)
                },
            ])
        );
    }

    #[test]
    fn an_attach_over_the_limit_or_past_a_full_store_is_refused() {
        let (store, referenced) = (store(), ids(&[]));
        let none = kept(&[]);
        let limit = ATTACHMENT_TEXT as u64;
        let attach = |bytes| StoreEvent::Attach { id: AttachmentId(1), bytes };
        assert!(plan(state(&store, &none, &referenced), attach(limit), 0).is_ok());
        let refused = plan(state(&store, &none, &referenced), attach(limit + 1), 0);
        assert_eq!(refused, Err(Refusal::TooLong { bytes: limit + 1 }));
        assert_eq!(
            Refusal::TooLong { bytes: 300_000 }.to_string(),
            "the attached text is 300000 bytes, over the limit of 262144"
        );
        let full = kept(&[(1, STORE_LIMIT - 10)]);
        assert!(plan(state(&store, &full, &referenced), attach(10), 0).is_ok());
        assert_eq!(
            plan(state(&store, &full, &referenced), attach(11), 0),
            Err(Refusal::StoreFull { held: STORE_LIMIT - 10 })
        );
    }

    #[test]
    fn a_relative_missing_or_unprivate_directory_refuses_attaches() {
        let (kept, referenced) = (kept(&[]), ids(&[]));
        let attach = StoreEvent::Attach { id: AttachmentId(1), bytes: 1 };
        let relative = Store { root: "att".into(), ..store() };
        assert!(matches!(
            plan(state(&relative, &kept, &referenced), attach, 0),
            Err(Refusal::NoStore { why }) if why.contains("isn't absolute")
        ));
        let none = Store::default();
        assert!(matches!(plan(state(&none, &kept, &referenced), attach, 0), Err(Refusal::NoStore { .. })));
        let store = store();
        let open = StoreState {
            private: Err("its mode is 0755; it must be 0700"),
            ..state(&store, &kept, &referenced)
        };
        let refused = plan(open, attach, 0);
        assert_eq!(
            refused.map_err(|refusal| refusal.to_string()),
            Err("the attachments directory /c/attachments isn't private: its mode is 0755; it must be 0700".into())
        );
    }

    #[test]
    fn a_change_removes_the_texts_nothing_references() {
        let store = store();
        // A prompt took 1 and 2 and was refused; 3 is a queued prompt's; 4
        // an entry's that was trimmed.
        let kept = kept(&[(1, 1), (2, 1), (3, 1), (4, 1)]);
        let referenced = ids(&[3]);
        assert_eq!(
            plan(state(&store, &kept, &referenced), StoreEvent::Changed, 0),
            Ok(vec![
                Step::Remove { id: AttachmentId(1), file: file(1) },
                Step::Remove { id: AttachmentId(2), file: file(2) },
                Step::Remove { id: AttachmentId(4), file: file(4) },
            ])
        );
        let all = ids(&[1, 2, 3, 4]);
        assert_eq!(plan(state(&store, &kept, &all), StoreEvent::Changed, 0), Ok(vec![]));
    }

    #[test]
    fn an_adoption_knows_the_referenced_texts_and_removes_the_orphans() {
        let store = store();
        let (none, referenced) = (kept(&[]), ids(&[2]));
        let listed = |name: &str, dir: bool| Listed {
            name: name.into(),
            dir,
            bytes: 5,
            modified: Some(1),
        };
        let listing = [listed("1", false), listed("2", false), listed("expires", false), listed("02", false), listed("3", true)];
        assert_eq!(
            plan(state(&store, &none, &referenced), StoreEvent::Adopted { listing: &listing }, 0),
            Ok(vec![
                Step::Know { id: AttachmentId(2), bytes: 5 },
                Step::Remove { id: AttachmentId(1), file: file(1) },
            ])
        );
    }

    #[test]
    fn a_sessions_end_writes_its_marker_or_removes_it_with_0_days() {
        let (kept, referenced) = (kept(&[(1, 1)]), ids(&[1]));
        let store = store();
        assert_eq!(
            plan(state(&store, &kept, &referenced), StoreEvent::Ended, 100),
            Ok(vec![Step::Mark {
                file: format!("{DIR}/expires"),
                text: format!("{}\n", 100 + 14 * DAY)
            }])
        );
        let now = Store { keep_days: 0, ..store.clone() };
        assert_eq!(
            plan(state(&now, &kept, &referenced), StoreEvent::Ended, 100),
            Ok(vec![Step::RemoveDir { dir: DIR.into() }])
        );
        let none = Store::default();
        assert_eq!(plan(state(&none, &kept, &referenced), StoreEvent::Ended, 100), Ok(vec![]));
    }

    /// Decision 4 (refresh, task 2.6): an agent's exit isn't an event of
    /// the store's; only the session's record being dropped (`Ended`) is.
    /// So an exit, with 0 days too, leaves the store as it is: the shell
    /// plans nothing for it, and a `Changed` after it removes nothing the
    /// log still references.
    #[test]
    fn an_agents_exit_writes_no_marker_and_removes_nothing() {
        let store = Store { keep_days: 0, ..store() };
        let (kept, referenced) = (kept(&[(1, 1)]), ids(&[1]));
        assert_eq!(plan(state(&store, &kept, &referenced), StoreEvent::Changed, 100), Ok(vec![]));
        assert_eq!(
            plan(state(&store, &kept, &referenced), StoreEvent::Ended, 100),
            Ok(vec![Step::RemoveDir { dir: DIR.into() }])
        );
    }

    // --- the sweep -------------------------------------------------------

    #[test]
    fn a_sweep_removes_what_is_past_its_time_and_never_a_live_sessions() {
        let now = 100 * DAY;
        let found = |dir: &str, marker, newest| Found {
            dir: dir.into(),
            marker,
            newest,
        };
        let dirs = [
            found("/a/past", Some(now - 1), Some(now)),
            found("/a/at", Some(now), None),
            found("/a/before", Some(now + 1), Some(0)),
            found("/a/old-unmarked", None, Some(now - 15 * DAY)),
            found("/a/new-unmarked", None, Some(now - 13 * DAY)),
            found("/a/unknown", None, None),
            found("/a/live", None, Some(0)),
        ];
        let live = BTreeSet::from(["/a/live".to_owned()]);
        let removed: Vec<_> = sweep(&dirs, &live, now)
            .into_iter()
            .map(|step| match step {
                Step::RemoveDir { dir } => dir,
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(removed, ["/a/past", "/a/at", "/a/old-unmarked"]);
    }

    #[test]
    fn a_core_sweeps_the_default_directory_and_a_session_its_own() {
        assert_eq!(sweep_root(SweepAt::CoreStart { default: Some("/c/attachments") }), Some("/c/attachments"));
        assert_eq!(sweep_root(SweepAt::CoreStart { default: None }), None);
        assert_eq!(sweep_root(SweepAt::Session { root: "/srv/att" }), Some("/srv/att"));
        assert_eq!(sweep_root(SweepAt::Session { root: "" }), None);
    }
}
