//! An agent's earlier conversations (OpenSpec change `agents-session-list`):
//! what a list answered ([`Page`]), the pure plans the window decides by
//! (which agent lists, which directory, where a choice opens, which is the
//! latest), the times a row shows, and the conversations palette's model.
//!
//! Pure: every function takes the facts and `now` as arguments; the
//! workspace gathers them, calls these and applies what they say
//! (`fcis R1`, `R2`).

use std::path::{Path, PathBuf};

use crate::agent::spawn::{self, OpenedBy};
use crate::config::{Profile, ProfileKey};
use crate::picker::PaletteRow;
use crate::remote::Host;
use crate::remote::protocol::client::{AgentConversations, ListedConversation};
use crate::workspace::TabId;
use crate::zone_view::{ZoneId, split_target};

/// One conversation an agent listed: its id, and its title and last
/// activity when the agent gave them (`updated` as the agent's text).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Listed {
    pub id: String,
    pub title: Option<String>,
    pub updated: Option<String>,
}

/// A page of conversations, in the agent's order, and the cursor for the
/// next page when the agent has more.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub conversations: Vec<Listed>,
    pub next: Option<String>,
}

impl Page {
    /// The page `AGENT_CONVERSATIONS` carries.
    pub fn of(listed: AgentConversations) -> Self {
        let conversations = listed
            .conversations
            .into_iter()
            .map(|ListedConversation { id, title, updated }| Listed { id, title, updated })
            .collect();
        Self { conversations, next: listed.next }
    }
}

/// A zone of the window: its tab and its id in that tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ZoneAt {
    pub tab: TabId,
    pub zone: ZoneId,
}

/// What a zone runs, as the plans see it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneRun {
    Terminal,
    /// An agent that is ready: it can list, and it runs.
    Ready,
    /// An agent waiting for a conversation to be chosen.
    Choosing,
    /// An agent that exited, was lost or never started: nothing runs.
    Stopped,
    /// An agent that connects, starts, logs in, failed in its turn's way
    /// or can't be followed: it may run, and can't list.
    Busy,
}

impl ZoneRun {
    /// Whether no agent runs in the zone, so a choice can open in it.
    fn idle_agent(self) -> bool {
        match self {
            Self::Choosing | Self::Stopped => true,
            Self::Terminal | Self::Ready | Self::Busy => false,
        }
    }
}

/// What the plans know of a zone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ZoneFacts {
    pub at: ZoneAt,
    /// Its profile's name; an agent zone's, or a terminal's.
    pub profile: String,
    pub host: Host,
    /// Its directory: an agent session's, or a terminal's working
    /// directory.
    pub dir: Option<PathBuf>,
    pub run: ZoneRun,
    /// The conversation the zone's agent has, by id.
    pub conversation: Option<String>,
    /// How recently it had focus: larger is more recent.
    pub recency: u64,
}

impl ZoneFacts {
    fn of_profile(&self, profile: &str, host: &Host) -> bool {
        self.run != ZoneRun::Terminal && self.profile.eq_ignore_ascii_case(profile) && self.host == *host
    }
}

/// Whether two directories are one, compared without a trailing separator
/// and nothing else (as the server's filter compares them, flag 9).
pub fn same_dir(a: Option<&Path>, b: Option<&Path>) -> bool {
    let trim = |dir: &Path| dir.to_string_lossy().trim_end_matches(['/', '\\']).to_string();
    match (a, b) {
        (Some(a), Some(b)) => trim(a) == trim(b),
        (None, None) => true,
        (Some(_), None) | (None, Some(_)) => false,
    }
}

/// Who answers a list (design decision 3).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lister {
    /// That zone's agent, through its own session.
    Zone(ZoneAt),
    /// An agent started to list, choosing (`SPAWN_AGENT` with `start`).
    Chooser,
}

/// Who lists `profile`'s conversations on `host` in `dir` for the pane
/// `from`: a zone of that profile on that host in that directory whose
/// agent is ready, `from` itself when it is one (or when it is choosing: its
/// agent is that profile's, waiting in that directory), else the one that
/// had focus last; otherwise an agent started for it.
pub fn lister_plan(profile: &str, host: &Host, dir: Option<&Path>, from: ZoneAt, zones: &[ZoneFacts]) -> Lister {
    let lists = |zone: &ZoneFacts| zone.of_profile(profile, host) && same_dir(zone.dir.as_deref(), dir);
    if let Some(zone) = zones.iter().find(|zone| zone.at == from)
        && lists(zone)
        && matches!(zone.run, ZoneRun::Ready | ZoneRun::Choosing)
    {
        return Lister::Zone(from);
    }
    zones
        .iter()
        .filter(|zone| lists(zone) && zone.run == ZoneRun::Ready)
        .max_by_key(|zone| zone.recency)
        .map_or(Lister::Chooser, |zone| Lister::Zone(zone.at))
}

/// The pane a list is asked from: its profile, host and directory.
#[derive(Debug, Clone, Copy)]
pub struct Pane<'a> {
    pub profile: &'a Profile,
    pub host: &'a Host,
    pub dir: Option<&'a Path>,
}

/// The host and directory a list of `profile`'s conversations is for, from
/// `pane` (design decision 4): where a new agent zone of `profile` split
/// from the pane would run and start ([`split_target`], then
/// [`spawn::directory`]), so the list and the split can't drift. `home` is
/// this machine's home directory.
pub fn list_dir(pane: Pane<'_>, profile: &Profile, home: Option<&Path>) -> (Host, Option<PathBuf>) {
    let (host, cwd) = split_target(pane.profile, pane.host, pane.dir.map(Path::to_path_buf), profile);
    let dir = spawn::directory(profile, &host, cwd, &OpenedBy::Window, home);
    (host, dir)
}

/// `dir` as a palette title or a reason shows it: under this machine's
/// `home` with `~`, else as it is; `~` when there is none (the home
/// directory on the endpoint).
pub fn shown_dir(dir: Option<&Path>, home: Option<&Path>) -> String {
    let Some(dir) = dir else { return "~".into() };
    if let Some(rest) = home.and_then(|home| dir.strip_prefix(home).ok()) {
        let rest = rest.to_string_lossy();
        return if rest.is_empty() { "~".into() } else { format!("~/{}", rest.replace('\\', "/")) };
    }
    dir.to_string_lossy().into_owned()
}

/// Whether a zone shows the conversation `id` of `profile` on `host` with
/// an agent that may run: the row is marked "open" and choosing it shows
/// the zone.
fn showing<'a>(id: &str, profile: &str, host: &Host, zones: &'a [ZoneFacts]) -> Option<&'a ZoneFacts> {
    zones.iter().find(|zone| {
        zone.of_profile(profile, host)
            && zone.conversation.as_deref() == Some(id)
            && matches!(zone.run, ZoneRun::Ready | ZoneRun::Busy)
    })
}

/// Whether the row of conversation `id` is marked "open".
pub fn open_mark(id: &str, profile: &str, host: &Host, zones: &[ZoneFacts]) -> bool {
    showing(id, profile, host, zones).is_some()
}

/// Where a chosen conversation opens (design decision 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Open {
    /// A zone already shows it: that zone's tab becomes active and it takes
    /// focus.
    Show(ZoneAt),
    /// The pane, an agent zone of that profile with no agent running,
    /// takes it.
    Here(ZoneAt),
    /// Ask whether to split the pane or open a new tab.
    Ask,
}

/// Where `profile`'s conversation `id` on `host` opens, chosen from the
/// pane `from` (the user's rule): shown where a zone has it; in the pane
/// when it is an agent zone of the profile with no agent running (stopped,
/// or choosing); else asked.
pub fn open_plan(id: &str, profile: &str, host: &Host, from: ZoneAt, zones: &[ZoneFacts]) -> Open {
    if let Some(zone) = showing(id, profile, host, zones) {
        return Open::Show(zone.at);
    }
    match zones.iter().find(|zone| zone.at == from) {
        Some(zone) if zone.of_profile(profile, host) && zone.run.idle_agent() => Open::Here(from),
        Some(_) | None => Open::Ask,
    }
}

/// The order `agent_continue_conversation` asks which of the config's
/// `agents` in (design decision 7): the config's, the `active` zone's
/// profile first when it is one of them.
pub fn ask_order(mut agents: Vec<ProfileKey>, active: Option<&ProfileKey>) -> Vec<ProfileKey> {
    if let Some(at) = active.and_then(|active| agents.iter().position(|key| key == active)) {
        let key = agents.remove(at);
        agents.insert(0, key);
    }
    agents
}

/// What `-c` continues (design decision 8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Latest {
    Continue(String),
    None,
}

/// The latest of `conversations`: the one with the newest `updated` that
/// parses, else the first listed (Antigravity doesn't list newest first,
/// decision 15); none when the list is empty.
pub fn latest_plan(conversations: &[Listed]) -> Latest {
    let newest = conversations
        .iter()
        .enumerate()
        .filter_map(|(ix, listed)| Some((rfc3339(listed.updated.as_deref()?)?, ix, listed)))
        // The newest; of equal times, the one listed first.
        .max_by(|(a, ai, _), (b, bi, _)| a.cmp(b).then(bi.cmp(ai)))
        .map(|(_, _, listed)| listed);
    match newest.or(conversations.first()) {
        Some(listed) => Latest::Continue(listed.id.clone()),
        None => Latest::None,
    }
}

/// Days since 1970-01-01 of the civil date `y-m-d` (Howard Hinnant's
/// `days_from_civil`).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// The civil date of `days` since 1970-01-01.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// The Unix time of an RFC 3339 timestamp (`2026-11-26T20:35:14Z`, with or
/// without fractional seconds, `Z` or an offset), to the second; `None`
/// for anything else (flag 6).
pub fn rfc3339(text: &str) -> Option<i64> {
    let digits = |s: &str| -> Option<i64> { s.bytes().all(|b| b.is_ascii_digit()).then(|| s.parse().ok())? };
    let (date, rest) = text.split_once(['T', 't'])?;
    let mut date_parts = date.splitn(3, '-');
    let (y, m, d) = (date_parts.next()?, date_parts.next()?, date_parts.next()?);
    if y.len() != 4 || m.len() != 2 || d.len() != 2 {
        return None;
    }
    let (y, m, d) = (digits(y)?, digits(m)?, digits(d)?);
    if !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let zone_at = rest.find(['Z', 'z', '+', '-'])?;
    let (time, zone) = rest.split_at(zone_at);
    let time = time.split_once('.').map_or(time, |(whole, fraction)| {
        if !fraction.is_empty() && fraction.bytes().all(|b| b.is_ascii_digit()) { whole } else { "" }
    });
    let mut time_parts = time.splitn(3, ':');
    let (hh, mm, ss) = (time_parts.next()?, time_parts.next()?, time_parts.next()?);
    if hh.len() != 2 || mm.len() != 2 || ss.len() != 2 {
        return None;
    }
    let (hh, mm, ss) = (digits(hh)?, digits(mm)?, digits(ss)?);
    if hh > 23 || mm > 59 || ss > 60 {
        return None;
    }
    let offset = match zone {
        "Z" | "z" => 0,
        _ => {
            let sign = if zone.starts_with('-') { -1 } else { 1 };
            let (oh, om) = zone.get(1..)?.split_once(':')?;
            if oh.len() != 2 || om.len() != 2 {
                return None;
            }
            sign * (digits(oh)? * 3600 + digits(om)? * 60)
        }
    };
    Some(days_from_civil(y, m, d) * 86_400 + hh * 3600 + mm * 60 + ss - offset)
}

/// When a conversation was last active, as its row shows it, at Unix time
/// `now`: "just now", "5 minutes ago", "2 hours ago", "yesterday" (the
/// calendar day before, in UTC), else its date; `None` when `updated`
/// doesn't parse.
pub fn ago(now: i64, updated: &str) -> Option<String> {
    let then = rfc3339(updated)?;
    let secs = now - then;
    let plural = |n: i64, unit: &str| if n == 1 { format!("1 {unit} ago") } else { format!("{n} {unit}s ago") };
    Some(if secs < 60 {
        "just now".into()
    } else if secs < 3600 {
        plural(secs / 60, "minute")
    } else if secs < 86_400 {
        plural(secs / 3600, "hour")
    } else if then.div_euclid(86_400) == now.div_euclid(86_400) - 1 {
        "yesterday".into()
    } else {
        let (y, m, d) = civil_from_days(then.div_euclid(86_400));
        format!("{y}-{m:02}-{d:02}")
    })
}

/// How many conversations the palette holds at most.
pub const HELD_LIMIT: usize = 1024;

/// The palette's list, as far as it got.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListState {
    /// The agent started to list isn't choosing yet.
    Starting,
    Listing,
    Listed,
    /// Nothing was listed, saying why (the agent can't list, needs a login,
    /// didn't start, or answered with an error).
    Failed(String),
}

/// The last row, after the conversations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum More {
    None,
    /// The agent has more, after this cursor.
    Available(String),
    Loading(String),
    /// Asking for more failed; the row asks again.
    Failed { cursor: String, why: String },
    /// The palette holds [`HELD_LIMIT`]: the row says so.
    Capped,
}

/// Where the question opens a chosen conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    SplitRight,
    SplitDown,
    NewTab,
}

impl Place {
    const ALL: [Place; 3] = [Place::SplitRight, Place::SplitDown, Place::NewTab];

    fn label(self) -> &'static str {
        match self {
            Self::SplitRight => "Split Right",
            Self::SplitDown => "Split Down",
            Self::NewTab => "New Tab",
        }
    }

    fn splits(self) -> bool {
        match self {
            Self::SplitRight | Self::SplitDown => true,
            Self::NewTab => false,
        }
    }
}

/// What the palette shows: the list, or the question of where to open
/// `chosen`, its splits disabled when the pane is too small (`splits`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stage {
    List,
    Ask { chosen: Listed, splits: bool },
}

/// What a confirmed row asks for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Pick {
    Conversation(Listed),
    /// The next page, after this cursor.
    More(String),
    /// Open the chosen conversation there.
    Place(Listed, Place),
    Nothing,
}

/// The conversations palette's model: `profile`'s conversations in `dir`
/// (as shown), its list's state, the conversations held, its last row, the
/// ids a zone shows, and its stage.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConversationsModel {
    profile: String,
    dir: String,
    state: ListState,
    conversations: Vec<Listed>,
    more: More,
    open: Vec<String>,
    stage: Stage,
}

impl ConversationsModel {
    /// A palette for `profile`'s conversations in `dir`, listing at once
    /// (`starting: false`) or once an agent started for it is choosing.
    pub fn new(profile: impl Into<String>, dir: impl Into<String>, starting: bool) -> Self {
        Self {
            profile: profile.into(),
            dir: dir.into(),
            state: if starting { ListState::Starting } else { ListState::Listing },
            conversations: Vec::new(),
            more: More::None,
            open: Vec::new(),
            stage: Stage::List,
        }
    }

    /// The palette's title (its filter's placeholder).
    pub fn title(&self) -> String {
        format!("{} conversations in {}", self.profile, self.dir)
    }

    #[cfg(test)]
    pub fn stage(&self) -> &Stage {
        &self.stage
    }

    pub fn state(&self) -> &ListState {
        &self.state
    }

    /// The agent started for the list is choosing: the first page is asked
    /// for.
    pub fn listing(&mut self) {
        if self.state == ListState::Starting {
            self.state = ListState::Listing;
        }
    }

    /// A page came: its conversations go below those held, up to
    /// [`HELD_LIMIT`].
    pub fn page(&mut self, page: Page) {
        let room = HELD_LIMIT.saturating_sub(self.conversations.len());
        let over = page.conversations.len() > room;
        self.conversations.extend(page.conversations.into_iter().take(room));
        let full = self.conversations.len() >= HELD_LIMIT;
        self.more = match page.next {
            Some(_) if full => More::Capped,
            None if over => More::Capped,
            Some(cursor) => More::Available(cursor),
            None => More::None,
        };
        self.state = ListState::Listed;
    }

    /// The list, or asking for more, failed with `why`: more keeps the
    /// rows shown.
    pub fn failed(&mut self, why: impl Into<String>) {
        let why = why.into();
        match (&self.state, &self.more) {
            (ListState::Listed, More::Loading(cursor)) => self.more = More::Failed { cursor: cursor.clone(), why },
            _ => self.state = ListState::Failed(why),
        }
    }

    /// The ids of the conversations a zone shows now.
    pub fn set_open(&mut self, open: Vec<String>) {
        self.open = open;
    }

    /// The question of where to open `chosen`.
    pub fn ask(&mut self, chosen: Listed, splits: bool) {
        self.stage = Stage::Ask { chosen, splits };
    }

    /// Escape at the question: the list again. False when the list shows.
    pub fn back(&mut self) -> bool {
        let asking = matches!(self.stage, Stage::Ask { .. });
        self.stage = Stage::List;
        asking
    }

    /// The rows, at Unix time `now`.
    pub fn rows(&self, now: i64) -> Vec<PaletteRow> {
        let row = |label: String, detail: Option<String>, disabled: bool| PaletteRow {
            label: label.into(),
            detail: detail.map(Into::into),
            shortcut: None,
            disabled,
        };
        match &self.stage {
            Stage::Ask { splits, .. } => {
                Place::ALL.iter().map(|place| row(place.label().into(), None, place.splits() && !splits)).collect()
            }
            Stage::List => {
                if self.state != ListState::Listed {
                    return Vec::new();
                }
                let mut rows: Vec<PaletteRow> = self
                    .conversations
                    .iter()
                    .map(|listed| {
                        let time = listed.updated.as_deref().and_then(|updated| ago(now, updated));
                        let open = self.open.contains(&listed.id).then(|| "open".to_string());
                        let detail: Vec<String> = time.into_iter().chain(open).collect();
                        row(conversation_label(listed), (!detail.is_empty()).then(|| detail.join(" · ")), false)
                    })
                    .collect();
                rows.extend(match &self.more {
                    More::None => None,
                    More::Available(_) => Some(row("More…".into(), None, false)),
                    More::Loading(_) => Some(row("Listing more…".into(), None, true)),
                    More::Failed { why, .. } => Some(row("More…".into(), Some(why.clone()), false)),
                    More::Capped => {
                        Some(row(format!("Only the first {} conversations are shown", grouped(HELD_LIMIT)), None, true))
                    }
                });
                rows
            }
        }
    }

    /// What the palette says when it lists no rows: the agent starting,
    /// listing, no conversation in the directory, or the failure.
    pub fn empty_text(&self) -> Option<String> {
        match (&self.stage, &self.state) {
            (Stage::Ask { .. }, _) => None,
            (Stage::List, ListState::Starting) => Some(format!("starting {}…", self.profile)),
            (Stage::List, ListState::Listing) => Some("listing…".into()),
            (Stage::List, ListState::Listed) => Some(format!("no earlier conversation in {}", self.dir)),
            (Stage::List, ListState::Failed(why)) => Some(why.clone()),
        }
    }

    /// What confirming row `ix` asks for. "More…" marks the page loading.
    pub fn confirm(&mut self, ix: usize) -> Pick {
        match &self.stage {
            Stage::Ask { chosen, splits } => match Place::ALL.get(ix) {
                Some(place) if !place.splits() || *splits => Pick::Place(chosen.clone(), *place),
                Some(_) | None => Pick::Nothing,
            },
            Stage::List => {
                if let Some(listed) = self.conversations.get(ix) {
                    return Pick::Conversation(listed.clone());
                }
                if ix != self.conversations.len() {
                    return Pick::Nothing;
                }
                match std::mem::replace(&mut self.more, More::None) {
                    More::Available(cursor) | More::Failed { cursor, .. } => {
                        self.more = More::Loading(cursor.clone());
                        Pick::More(cursor)
                    }
                    more @ (More::None | More::Loading(_) | More::Capped) => {
                        self.more = more;
                        Pick::Nothing
                    }
                }
            }
        }
    }

    /// The dump's line for the list (design decision 10); none at the
    /// question, which prints as a palette list.
    pub fn describe(&self) -> String {
        if matches!(self.stage, Stage::Ask { .. }) {
            return String::new();
        }
        let state = match &self.state {
            ListState::Starting => "starting",
            ListState::Listing => "listing",
            ListState::Listed if self.conversations.is_empty() => "empty",
            ListState::Listed => "listed",
            ListState::Failed(_) => "failed",
        };
        let more = if page_has_more(&self.more) { "yes" } else { "no" };
        format!("--- palette conversations profile={:?} dir={:?} state={state} more={more}\n", self.profile, self.dir)
    }
}

/// Whether the agent has a page the palette hasn't shown.
fn page_has_more(more: &More) -> bool {
    match more {
        More::Available(_) | More::Loading(_) | More::Failed { .. } => true,
        More::None | More::Capped => false,
    }
}

/// `n` with a comma between thousands (`1,024`).
fn grouped(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (ix, ch) in digits.chars().enumerate() {
        if ix > 0 && (digits.len() - ix).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// A row's label: the title, else "Untitled" and the start of the id.
pub fn conversation_label(listed: &Listed) -> String {
    match listed.title.as_deref().filter(|title| !title.trim().is_empty()) {
        Some(title) => title.to_string(),
        None => format!("Untitled {}", listed.id.chars().take(6).collect::<String>()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    fn listed(id: &str, title: Option<&str>, updated: Option<&str>) -> Listed {
        Listed { id: id.into(), title: title.map(Into::into), updated: updated.map(Into::into) }
    }

    fn at(tab: u64, zone: u64) -> ZoneAt {
        ZoneAt { tab: TabId(tab), zone: ZoneId(zone) }
    }

    fn facts(at: ZoneAt, profile: &str, dir: &str, run: ZoneRun) -> ZoneFacts {
        ZoneFacts {
            at,
            profile: profile.into(),
            host: Host::Local,
            dir: Some(PathBuf::from(dir)),
            run,
            conversation: None,
            recency: 0,
        }
    }

    fn profiles(text: &str) -> Vec<Profile> {
        Config::parse(text).unwrap().config.profiles
    }

    const TWO: &str = "[[profiles]]\nname = \"Shell\"\n\n[[profiles]]\nname = \"Claude\"\ntype = \"agent\"\ncommand = \"claude-acp\"\n\n[[profiles]]\nname = \"Far\"\ntype = \"agent\"\ncommand = \"agy\"\nremote = { type = \"ssh\", host = \"A8\" }\n";

    /// The zone the list was asked from lists when it is ready (or
    /// choosing) in the directory wanted; else the ready zone of the
    /// profile there that had focus last; a zone in another directory, of
    /// another profile or busy never does; with none, an agent is started.
    #[test]
    fn a_ready_zone_of_the_profile_in_the_directory_lists() {
        let from = at(1, 1);
        let here = facts(from, "Claude", "/src/app", ZoneRun::Ready);
        let host = Host::Local;
        let dir = Some(Path::new("/src/app/"));
        assert_eq!(lister_plan("claude", &host, dir, from, std::slice::from_ref(&here)), Lister::Zone(from));
        let choosing = ZoneFacts { run: ZoneRun::Choosing, ..here.clone() };
        assert_eq!(lister_plan("Claude", &host, dir, from, std::slice::from_ref(&choosing)), Lister::Zone(from));
        let elsewhere = facts(at(2, 3), "Claude", "/src/other", ZoneRun::Ready);
        assert_eq!(lister_plan("Claude", &host, dir, at(1, 9), &[elsewhere]), Lister::Chooser, "another directory");
        let older = ZoneFacts { recency: 1, ..facts(at(2, 1), "Claude", "/src/app", ZoneRun::Ready) };
        let newer = ZoneFacts { recency: 5, ..facts(at(3, 1), "Claude", "/src/app", ZoneRun::Ready) };
        let busy = ZoneFacts { recency: 9, ..facts(at(4, 1), "Claude", "/src/app", ZoneRun::Busy) };
        let shell = facts(at(5, 1), "Shell", "/src/app", ZoneRun::Terminal);
        let other = ZoneFacts { recency: 9, ..facts(at(6, 1), "Codex", "/src/app", ZoneRun::Ready) };
        let zones = [older, newer, busy, shell, other, ZoneFacts { at: at(7, 1), ..choosing }];
        assert_eq!(lister_plan("Claude", &host, dir, at(5, 1), &zones), Lister::Zone(at(3, 1)));
        let remote = Host::of(&profiles(TWO)[2]);
        assert_eq!(lister_plan("Claude", &remote, dir, at(5, 1), &zones), Lister::Chooser, "another host");
    }

    /// The directory listed is the split rule's: an agent zone's own; a
    /// terminal's working directory on the profile's host; else the
    /// profile's directory, else home there (none on a remote endpoint).
    #[test]
    fn the_directory_listed_is_the_one_a_split_would_start_in() {
        let profiles = profiles(TWO);
        let (shell, claude, far) = (&profiles[0], &profiles[1], &profiles[2]);
        let home = Path::new("/home/u");
        let local = Host::Local;
        let api = Path::new("/home/u/src/api");
        let from_shell = Pane { profile: shell, host: &local, dir: Some(api) };
        assert_eq!(list_dir(from_shell, claude, Some(home)), (Host::Local, Some(api.to_path_buf())));
        let from_agent = Pane { profile: claude, host: &local, dir: Some(Path::new("/srv/x")) };
        assert_eq!(list_dir(from_agent, claude, Some(home)).1, Some(PathBuf::from("/srv/x")));
        let (host, dir) = list_dir(from_shell, far, Some(home));
        assert_eq!((host.is_local(), dir), (false, None), "a remote profile's home there");
        let unknown = Pane { profile: shell, host: &local, dir: None };
        assert_eq!(list_dir(unknown, claude, Some(home)).1, Some(home.to_path_buf()), "home here");
        let with_cwd = &profiles_with_cwd();
        assert_eq!(list_dir(Pane { dir: None, ..from_shell }, with_cwd, Some(home)).1, Some(PathBuf::from("/srv/proj")));
        let far_pane = Pane { profile: far, host: &Host::of(far), dir: Some(Path::new("/remote/dir")) };
        assert_eq!(list_dir(far_pane, far, Some(home)).1, Some(PathBuf::from("/remote/dir")), "the zone's own");
    }

    fn profiles_with_cwd() -> Profile {
        profiles("[[profiles]]\nname = \"Proj\"\ntype = \"agent\"\ncommand = \"x\"\ncwd = \"/srv/proj\"\n").remove(0)
    }

    /// Decision 7: the question lists the config's agent profiles in order,
    /// the active zone's first when it is one of them.
    #[test]
    fn the_profile_question_puts_the_active_zones_first() {
        let key = |name: &str, nth| ProfileKey { name: name.into(), nth };
        let agents = vec![key("Claude", 0), key("Codex", 0), key("Claude", 1)];
        assert_eq!(ask_order(agents.clone(), None), agents);
        assert_eq!(ask_order(agents.clone(), Some(&key("Claude", 1))), [key("Claude", 1), key("Claude", 0), key("Codex", 0)]);
        assert_eq!(ask_order(agents.clone(), Some(&key("Shell", 0))), agents, "a terminal's profile isn't asked");
    }

    /// A directory under home shows with `~`; others as they are; none is
    /// the endpoint's home.
    #[test]
    fn a_directory_shows_under_home_with_a_tilde() {
        let home = Some(Path::new("/home/u"));
        assert_eq!(shown_dir(Some(Path::new("/home/u/src/brindle")), home), "~/src/brindle");
        assert_eq!(shown_dir(Some(Path::new("/home/u")), home), "~");
        assert_eq!(shown_dir(Some(Path::new("/srv")), home), "/srv");
        assert_eq!(shown_dir(None, home), "~");
        assert!(same_dir(Some(Path::new("/a/b/")), Some(Path::new("/a/b"))));
        assert!(!same_dir(Some(Path::new("/a/b")), Some(Path::new("/a/B"))));
    }

    /// A conversation a zone shows opens there; a stopped or choosing zone
    /// of the profile takes it; a running agent or a shell asks.
    #[test]
    fn a_choice_opens_where_the_users_rule_says() {
        let host = Host::Local;
        let shown = ZoneFacts { conversation: Some("s7".into()), ..facts(at(2, 4), "Claude", "/a", ZoneRun::Ready) };
        let stopped = ZoneFacts { conversation: Some("s1".into()), ..facts(at(1, 1), "Claude", "/a", ZoneRun::Stopped) };
        let choosing = facts(at(1, 2), "Claude", "/a", ZoneRun::Choosing);
        let running = ZoneFacts { conversation: Some("s2".into()), ..facts(at(1, 3), "Claude", "/a", ZoneRun::Ready) };
        let shell = facts(at(1, 5), "Shell", "/a", ZoneRun::Terminal);
        let codex = facts(at(1, 6), "Codex", "/a", ZoneRun::Stopped);
        let zones = [shown, stopped, choosing, running, shell, codex];
        assert_eq!(open_plan("s7", "Claude", &host, at(1, 5), &zones), Open::Show(at(2, 4)), "already open");
        assert!(open_mark("s7", "claude", &host, &zones));
        assert_eq!(open_plan("s9", "Claude", &host, at(1, 1), &zones), Open::Here(at(1, 1)), "a stopped zone");
        assert_eq!(open_plan("s1", "Claude", &host, at(1, 1), &zones), Open::Here(at(1, 1)), "its own, stopped");
        assert!(!open_mark("s1", "Claude", &host, &zones), "a stopped zone's isn't open");
        assert_eq!(open_plan("s9", "Claude", &host, at(1, 2), &zones), Open::Here(at(1, 2)), "a choosing zone");
        assert_eq!(open_plan("s9", "Claude", &host, at(1, 3), &zones), Open::Ask, "a running agent");
        assert_eq!(open_plan("s9", "Claude", &host, at(1, 5), &zones), Open::Ask, "a shell");
        assert_eq!(open_plan("s9", "Claude", &host, at(1, 6), &zones), Open::Ask, "another profile's zone");
    }

    /// The latest is the newest `updated`, whatever the order; the first
    /// listed when none parses; none for an empty list.
    #[test]
    fn the_latest_is_the_newest_time_else_the_first() {
        assert_eq!(latest_plan(&[]), Latest::None);
        let antigravity = [
            listed("old", None, Some("2026-11-20T10:00:00Z")),
            listed("new", None, Some("2026-11-26T20:35:37Z")),
            listed("mid", None, Some("2026-11-25T00:00:00.500+01:00")),
        ];
        assert_eq!(latest_plan(&antigravity), Latest::Continue("new".into()));
        let unparsed = [listed("a", None, Some("yesterday")), listed("b", None, None)];
        assert_eq!(latest_plan(&unparsed), Latest::Continue("a".into()));
        let tie = [listed("x", None, Some("2026-11-26T00:00:00Z")), listed("y", None, Some("2026-11-26T00:00:00Z"))];
        assert_eq!(latest_plan(&tie), Latest::Continue("x".into()));
    }

    /// RFC 3339 with fractions and offsets reads; anything else doesn't.
    #[test]
    fn rfc3339_times_read_to_the_second() {
        assert_eq!(rfc3339("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(rfc3339("2026-11-26T20:35:14.000Z"), rfc3339("2026-11-26T20:35:14Z"));
        assert_eq!(rfc3339("2026-11-26T21:35:14+01:00"), rfc3339("2026-11-26T20:35:14Z"));
        assert_eq!(rfc3339("2000-03-01T00:00:00Z"), Some(951_868_800));
        for bad in ["", "2026-11-26", "2026-11-26T20:35Z", "2026-13-01T00:00:00Z", "26-11-26T00:00:00Z", "2026-11-26T20:35:14", "2026-11-26T20:35:14.Z"] {
            assert_eq!(rfc3339(bad), None, "{bad:?}");
        }
    }

    /// Minutes, hours, yesterday, a date, and nothing for text that doesn't
    /// parse.
    #[test]
    fn ago_reads_as_a_row_shows_it() {
        let now = rfc3339("2026-11-26T12:00:00Z").unwrap();
        assert_eq!(ago(now, "2026-11-26T11:59:30Z").as_deref(), Some("just now"));
        assert_eq!(ago(now, "2026-11-26T11:55:00Z").as_deref(), Some("5 minutes ago"));
        assert_eq!(ago(now, "2026-11-26T11:59:00Z").as_deref(), Some("1 minute ago"));
        assert_eq!(ago(now, "2026-11-26T10:00:00Z").as_deref(), Some("2 hours ago"));
        assert_eq!(ago(now, "2026-11-25T08:00:00Z").as_deref(), Some("yesterday"));
        assert_eq!(ago(now, "2026-11-03T08:00:00Z").as_deref(), Some("2026-11-03"));
        assert_eq!(ago(now, "2026-11-27T08:00:00Z").as_deref(), Some("just now"), "a clock ahead");
        assert_eq!(ago(now, "soon"), None);
    }

    fn page(ids: &[&str], next: Option<&str>) -> Page {
        Page { conversations: ids.iter().map(|id| listed(id, Some(id), None)).collect(), next: next.map(Into::into) }
    }

    fn labels(rows: &[PaletteRow]) -> Vec<String> {
        rows.iter()
            .map(|row| match &row.detail {
                Some(detail) => format!("{}  {detail}{}", row.label, if row.disabled { " [off]" } else { "" }),
                None => format!("{}{}", row.label, if row.disabled { " [off]" } else { "" }),
            })
            .collect()
    }

    /// Each state says itself, in the palette and the dump's line.
    #[test]
    fn the_palettes_states_and_its_dump_line() {
        let mut model = ConversationsModel::new("Claude", "~/src/brindle", true);
        assert_eq!(model.title(), "Claude conversations in ~/src/brindle");
        assert_eq!(model.empty_text().as_deref(), Some("starting Claude…"));
        assert!(model.describe().contains("profile=\"Claude\" dir=\"~/src/brindle\" state=starting more=no"));
        model.listing();
        assert_eq!(model.empty_text().as_deref(), Some("listing…"));
        assert!(model.describe().contains("state=listing"));
        model.page(page(&[], None));
        assert_eq!(model.empty_text().as_deref(), Some("no earlier conversation in ~/src/brindle"));
        assert!(model.describe().contains("state=empty more=no"));
        assert!(model.rows(0).is_empty());
        let mut failed = ConversationsModel::new("Claude", "~", false);
        failed.failed("the agent can't list conversations");
        assert_eq!(failed.empty_text().as_deref(), Some("the agent can't list conversations"));
        assert!(failed.describe().contains("state=failed"));
        let quoted = ConversationsModel::new("A \"b\"", "~/x", false);
        assert!(quoted.describe().contains(r#"profile="A \"b\"""#), "escaped: {}", quoted.describe());
    }

    /// Rows show the title (else Untitled and the id's start), the time and
    /// "open"; "More…" asks for the next page, which goes below; a failed
    /// more keeps the rows.
    #[test]
    fn rows_more_and_a_failed_more() {
        let now = rfc3339("2026-11-26T12:00:00Z").unwrap();
        let mut model = ConversationsModel::new("Claude", "~", false);
        model.page(Page {
            conversations: vec![
                listed("a1", Some("Fix the upload test"), Some("2026-11-26T10:00:00Z")),
                listed("7f3a9c0d", None, Some("2026-11-25T09:00:00Z")),
                listed("c3", Some(" "), Some("never")),
            ],
            next: Some("p2".into()),
        });
        model.set_open(vec!["7f3a9c0d".into()]);
        assert_eq!(
            labels(&model.rows(now)),
            ["Fix the upload test  2 hours ago", "Untitled 7f3a9c  yesterday · open", "Untitled c3", "More…"]
        );
        assert!(model.describe().contains("state=listed more=yes"));
        assert_eq!(model.confirm(1), Pick::Conversation(listed("7f3a9c0d", None, Some("2026-11-25T09:00:00Z"))));
        assert_eq!(model.confirm(3), Pick::More("p2".into()));
        assert_eq!(labels(&model.rows(now))[3], "Listing more… [off]");
        assert_eq!(model.confirm(3), Pick::Nothing, "already loading");
        model.failed("the agent's error");
        assert_eq!(labels(&model.rows(now))[3], "More…  the agent's error");
        assert_eq!(model.confirm(3), Pick::More("p2".into()), "asks again");
        model.page(page(&["d4"], None));
        assert_eq!(labels(&model.rows(now))[3..], ["d4".to_string()]);
        assert!(model.describe().contains("more=no"));
        assert_eq!(model.confirm(9), Pick::Nothing);
    }

    /// At most 1,024 are held; past that the last row says so.
    #[test]
    fn the_palette_holds_at_most_1024() {
        let mut model = ConversationsModel::new("Claude", "~", false);
        let ids: Vec<String> = (0..1000).map(|n| format!("c{n}")).collect();
        let first: Vec<&str> = ids.iter().map(String::as_str).collect();
        model.page(page(&first, Some("next")));
        assert_eq!(model.rows(0).len(), 1001);
        assert_eq!(model.confirm(1000), Pick::More("next".into()));
        model.page(page(&first[..100], Some("again")));
        let rows = model.rows(0);
        assert_eq!(rows.len(), HELD_LIMIT + 1);
        assert_eq!(labels(&rows[HELD_LIMIT..]), ["Only the first 1,024 conversations are shown [off]"]);
        assert!(model.describe().contains("more=no"));
        assert_eq!(model.confirm(HELD_LIMIT), Pick::Nothing);
    }

    /// The question's rows, a split disabled when the pane is too small,
    /// escape going back to the list.
    #[test]
    fn the_question_and_its_disabled_splits() {
        let mut model = ConversationsModel::new("Claude", "~", false);
        model.page(page(&["a"], None));
        let chosen = listed("a", Some("a"), None);
        model.ask(chosen.clone(), true);
        assert_eq!(labels(&model.rows(0)), ["Split Right", "Split Down", "New Tab"]);
        assert_eq!(model.describe(), "", "the question prints as a palette list");
        assert_eq!(model.empty_text(), None);
        assert_eq!(model.confirm(0), Pick::Place(chosen.clone(), Place::SplitRight));
        model.ask(chosen.clone(), false);
        assert_eq!(labels(&model.rows(0)), ["Split Right [off]", "Split Down [off]", "New Tab"]);
        assert_eq!(model.confirm(1), Pick::Nothing);
        assert_eq!(model.confirm(2), Pick::Place(chosen, Place::NewTab));
        assert!(model.back());
        assert_eq!(model.stage(), &Stage::List);
        assert!(!model.back(), "escape at the list closes it");
    }
}
