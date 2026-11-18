//! The palette: the view behind every palette Brindle shows (new tab from a
//! profile, the command palette, the sessions palette) and the one-line
//! prompt (`Rename Tab`).
//!
//! `Workspace` shows it in a modal `brindle_ui::Overlay`. Its list mode is a
//! `brindle_ui::Command` in `Dialog` chrome, as shadcn's `CommandDialog`
//! puts one (no padding, no visible header); its prompt mode is an `Input`
//! under the prompt's label, which is the dialog's title.
//!
//! Every way of closing reaches the owner as [`PaletteEvent::Dismissed`]:
//! escape and an outside press arrive as `Dismiss` (the command list's own
//! `Cancelled` in list mode, this view's handler in prompt mode). The view
//! never emits `DismissEvent`, so the overlay never closes behind the
//! owner's back.
//!
//! The sessions palette's state and rows are built here too, purely
//! ([`SessionsModel`], [`session_palette`]): grouped by endpoint, with a row
//! that does nothing for each endpoint still being listed, still connecting
//! or offline.

use std::rc::Rc;

use gpui::{
    App, AppContext as _, ClickEvent, Context, CursorStyle, DismissEvent, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString, Styled, Subscription, Window, div,
};

use crate::actions::PickerKill;
use crate::config::Config;
use crate::remote::Host;
use crate::remote::protocol::client::SessionInfo;
use brindle_ui::{
    Command, CommandEvent, CommandGroup, CommandItem, Dialog, Dismiss, FocusStops, Input, ListConfirm, TextInput,
};

pub enum PaletteEvent {
    /// A row was chosen; the index is into the rows the palette was built
    /// with, across its groups in order.
    Confirmed(usize),
    /// `picker_kill` on a row (only the sessions palette acts on it).
    Kill(usize),
    Dismissed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteRow {
    pub label: SharedString,
    pub detail: Option<SharedString>,
    pub shortcut: Option<SharedString>,
    /// Listed muted, and can't be highlighted or chosen.
    pub disabled: bool,
}

impl PaletteRow {
    fn item(&self) -> CommandItem {
        CommandItem {
            label: self.label.clone(),
            detail: self.detail.clone(),
            shortcut: self.shortcut.clone(),
            disabled: self.disabled,
            checked: false,
        }
    }
}

/// Rows under a heading, or none. A group after the first has a separator
/// above it, which the command list shows only while the groups on both
/// sides list rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteGroup {
    pub heading: Option<SharedString>,
    pub rows: Vec<PaletteRow>,
}

impl PaletteGroup {
    pub fn unnamed(rows: Vec<PaletteRow>) -> Self {
        Self { heading: None, rows }
    }

    pub fn named(heading: impl Into<SharedString>, rows: Vec<PaletteRow>) -> Self {
        Self { heading: Some(heading.into()), rows }
    }
}

/// The command list's groups.
fn command_groups(groups: &[PaletteGroup]) -> Vec<CommandGroup> {
    groups
        .iter()
        .enumerate()
        .map(|(ix, group)| CommandGroup {
            heading: group.heading.clone(),
            items: group.rows.iter().map(PaletteRow::item).collect(),
            separator: ix > 0,
        })
        .collect()
}

/// Runs a prompt's text; the palette closes once it has.
pub type SubmitFn = Rc<dyn Fn(String, &mut App)>;

/// A request to ask the user for one line of text.
#[derive(Clone)]
pub struct PromptRequest {
    pub label: SharedString,
    pub initial: String,
    pub submit: SubmitFn,
}

struct Prompt {
    label: SharedString,
    input: Entity<TextInput>,
    submit: SubmitFn,
}

struct List {
    groups: Vec<PaletteGroup>,
    command: Entity<Command>,
}

impl List {
    /// The row at `ix` across the groups, as the command list counts.
    fn row(&self, ix: usize) -> Option<&PaletteRow> {
        self.groups.iter().flat_map(|group| &group.rows).nth(ix)
    }
}

enum Mode {
    List(List),
    Prompt(Prompt),
}

pub struct Palette {
    /// The view's own handle, tracked on its root: the overlay keeps it for
    /// the view's whole life, while the input with focus is the mode's.
    focus: FocusHandle,
    mode: Mode,
    _subscription: Option<Subscription>,
}

impl EventEmitter<PaletteEvent> for Palette {}
/// Required of an overlay's view; never emitted (see the module doc).
impl EventEmitter<DismissEvent> for Palette {}

impl Focusable for Palette {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

/// One stop, the active mode's input, so Tab stays on it.
impl FocusStops for Palette {
    fn stops(&self, cx: &App) -> Vec<FocusHandle> {
        vec![self.input(cx).focus_handle(cx)]
    }
}

impl Palette {
    /// A filterable list of `rows`, with `placeholder` in the empty filter.
    pub fn list(rows: Vec<PaletteRow>, placeholder: impl Into<SharedString>, cx: &mut Context<Self>) -> Self {
        Self::list_grouped(vec![PaletteGroup::unnamed(rows)], placeholder, None, cx)
    }

    /// [`Palette::list`] of rows in `groups`, with `empty` as the message
    /// shown when nothing is listed, instead of the command list's own.
    pub fn list_grouped(
        groups: Vec<PaletteGroup>,
        placeholder: impl Into<SharedString>,
        empty: Option<SharedString>,
        cx: &mut Context<Self>,
    ) -> Self {
        let placeholder = placeholder.into();
        let command = cx.new(|cx| {
            let mut command = Command::new(command_groups(&groups), cx).with_placeholder(placeholder, cx);
            command.set_empty_message(empty, cx);
            command
        });
        let subscription = cx.subscribe(&command, |_, _, event: &CommandEvent, cx| match *event {
            CommandEvent::Confirmed(ix) => cx.emit(PaletteEvent::Confirmed(ix)),
            CommandEvent::Cancelled => cx.emit(PaletteEvent::Dismissed),
        });
        Self { focus: cx.focus_handle(), mode: Mode::List(List { groups, command }), _subscription: Some(subscription) }
    }

    pub fn prompt(request: PromptRequest, cx: &mut Context<Self>) -> Self {
        // The pre-filled value is selected, so typing replaces it.
        let input = cx.new(|cx| {
            let mut input = TextInput::new(&request.initial, cx);
            input.select_all(cx);
            input
        });
        let prompt = Prompt { label: request.label, input, submit: request.submit };
        Self { focus: cx.focus_handle(), mode: Mode::Prompt(prompt), _subscription: None }
    }

    /// Replaces a list's groups and its empty message (`None`: the command
    /// list's own), keeping its filter text. `keep` names the row to keep
    /// highlighted, by its index across the new groups (the list then also
    /// keeps its scroll offset); with `None` the command list highlights
    /// the first match again.
    pub fn set_groups(
        &mut self,
        groups: Vec<PaletteGroup>,
        empty: Option<SharedString>,
        keep: Option<usize>,
        cx: &mut Context<Self>,
    ) {
        if let Mode::List(list) = &mut self.mode {
            list.command.update(cx, |command, cx| {
                command.set_empty_message(empty, cx);
                command.set_groups(command_groups(&groups), keep, cx);
            });
            list.groups = groups;
        }
    }

    /// A list's highlighted row, by its index across the groups.
    pub fn highlighted(&self, cx: &App) -> Option<usize> {
        match &self.mode {
            Mode::List(list) => list.command.read(cx).highlighted(),
            Mode::Prompt(_) => None,
        }
    }

    /// The filter or prompt input.
    pub fn input(&self, cx: &App) -> Entity<TextInput> {
        match &self.mode {
            Mode::List(list) => list.command.read(cx).filter_input().clone(),
            Mode::Prompt(prompt) => prompt.input.clone(),
        }
    }

    /// Debug description for `--dump-screen-after`.
    pub fn describe(&self, cx: &App) -> String {
        match &self.mode {
            Mode::List(list) => {
                let command = list.command.read(cx);
                let highlighted = command
                    .highlighted()
                    .and_then(|item| command.rows().matches().iter().position(|&ix| ix == item))
                    .map_or_else(|| "none".to_string(), |at| at.to_string());
                let mut out = format!("--- palette list query={:?} highlighted={highlighted}\n", command.query(cx));
                for &ix in command.rows().matches() {
                    let Some(row) = list.row(ix) else { continue };
                    out.push_str(&row.label);
                    if let Some(detail) = &row.detail {
                        out.push_str(&format!("  {detail}"));
                    }
                    if let Some(shortcut) = &row.shortcut {
                        out.push_str(&format!("  [{shortcut}]"));
                    }
                    out.push('\n');
                }
                out
            }
            Mode::Prompt(p) => format!("--- palette prompt {:?} text={:?}\n", p.label, p.input.read(cx).text()),
        }
    }

    /// `picker_kill`: the highlighted row of a list.
    fn kill(&mut self, _: &PickerKill, _: &mut Window, cx: &mut Context<Self>) {
        if let Mode::List(list) = &self.mode
            && let Some(ix) = list.command.read(cx).highlighted()
        {
            cx.emit(PaletteEvent::Kill(ix));
        }
    }

    /// Enter in a prompt submits it, as enter in a one-field form does. A
    /// list confirms through its command list, which is nearer the focus.
    fn submit(&mut self, _: &ListConfirm, _: &mut Window, cx: &mut Context<Self>) {
        let Mode::Prompt(prompt) = &self.mode else { return };
        let text = prompt.input.read(cx).text().to_string();
        (prompt.submit.clone())(text, cx);
        cx.emit(PaletteEvent::Dismissed);
    }

    /// Escape or an outside press (a list's arrives as the command list's
    /// `Cancelled` instead).
    fn dismiss(&mut self, _: &Dismiss, _: &mut Window, cx: &mut Context<Self>) {
        cx.emit(PaletteEvent::Dismissed);
    }

    fn render_prompt(prompt: &Prompt, cx: &mut Context<Self>) -> Dialog {
        Dialog::new()
            .title(prompt.label.clone())
            .on_close(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(PaletteEvent::Dismissed)))
            .child(Input::new(&prompt.input))
    }
}

impl Render for Palette {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let panel = match &self.mode {
            // CommandDialog: DialogContent with overflow-hidden p-0 around
            // the command list, its header for screen readers only and no
            // close button.
            Mode::List(list) => Dialog::new().p_0().gap_0().overflow_hidden().child(list.command.clone()),
            Mode::Prompt(prompt) => Self::render_prompt(prompt, cx),
        };
        div()
            .key_context("Palette")
            .track_focus(&self.focus)
            .on_action(cx.listener(Self::kill))
            .on_action(cx.listener(Self::submit))
            .on_action(cx.listener(Self::dismiss))
            .cursor(CursorStyle::Arrow)
            .child(panel)
    }
}

// ---- the sessions palette ----------------------------------------------------

/// How an endpoint answered the sessions palette's listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Listing {
    /// Its sessions (attached ones are left out).
    Sessions(Vec<SessionInfo>),
    /// Brindle is still connecting to it.
    Connecting,
    /// It can't be reached, or didn't answer, and why.
    Offline(String),
    /// It answered, but listing its sessions failed, and why: this machine,
    /// whose server isn't offline when it can't be listed. It may have
    /// sessions, so it never adds the note that nothing is detached.
    Unlisted(String),
    /// It hasn't answered yet: [`SessionsModel`] starts every endpoint
    /// here, and [`session_palette`] lists it as `listing sessions…`.
    Pending,
}

/// What a sessions palette row stands for, by row across the groups.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(
    clippy::large_enum_variant,
    reason = "one row per detached session in an open palette, a handful at most; boxing `SessionInfo` adds an allocation per row"
)]
pub enum SessionRow {
    /// A detached session of `host`'s core: enter attaches it, and
    /// `picker_kill` ends it, both on `host`'s connection.
    Session { host: Host, info: SessionInfo },
    /// A row that does nothing: `host` being listed, connecting, offline or
    /// unlisted.
    Endpoint(Host),
    /// A row that does nothing: a session on `host` that the palette
    /// couldn't end, and why, until `host` answers again.
    KillFailed(Host),
    /// A row that does nothing: the note that nothing is detached.
    Note,
}

/// What stays the same about a sessions palette row when the rows around it
/// change: how the model finds the highlighted row again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowKey {
    /// A session: its endpoint, id and key (ids restart in a fresh core).
    Session { host: Host, id: u32, key: String },
    /// An endpoint's row.
    Endpoint(Host),
    /// An endpoint's failed kill.
    KillFailed(Host),
    /// The note.
    Note,
}

impl SessionRow {
    /// The endpoint whose connection the row's actions use, and its session.
    pub fn session(&self) -> Option<(&Host, &SessionInfo)> {
        match self {
            Self::Session { host, info } => Some((host, info)),
            Self::Endpoint(_) | Self::KillFailed(_) | Self::Note => None,
        }
    }

    pub fn key(&self) -> RowKey {
        match self {
            Self::Session { host, info } => RowKey::Session { host: host.clone(), id: info.session, key: info.key.clone() },
            Self::Endpoint(host) => RowKey::Endpoint(host.clone()),
            Self::KillFailed(host) => RowKey::KillFailed(host.clone()),
            Self::Note => RowKey::Note,
        }
    }
}

/// A sessions palette: its groups, what each row stands for, and its empty
/// message.
#[derive(Debug, PartialEq)]
pub struct SessionPalette {
    pub groups: Vec<PaletteGroup>,
    pub rows: Vec<SessionRow>,
    pub empty: Option<SharedString>,
}

/// What the sessions palette says when nothing is detached anywhere.
pub const NO_DETACHED: &str = "No detached sessions";

/// How long the sessions palette waits for every endpoint to answer before
/// it opens with the answers it has, listing the others as pending.
pub const OPEN_GRACE: std::time::Duration = std::time::Duration::from_millis(250);

/// The sessions palette for what each endpoint `listed` (this machine's
/// listing among them), without the sessions in `killed` (endpoint, id and
/// key: they stay listed until their programs have exited), at `now` (unix
/// seconds).
///
/// Endpoints come in this order: this machine, then remote endpoints in the
/// order of the first profile that targets each, then those no profile
/// targets (a tab's, after a reload), by name. Each one's detached sessions
/// are newest first, under its name when any remote endpoint lists one
/// ("This machine" over the local ones); with only this machine's, there is
/// no heading. An endpoint still being listed, still connecting or offline
/// gets one row that does nothing, saying so, and so does one that couldn't
/// be listed (unlisted). A failed kill on an endpoint (`kill_failed`: the
/// endpoint and why) is a row that does nothing after the endpoint's. When
/// nothing is detached anywhere, no endpoint is still being listed and
/// every endpoint could be listed, the palette says so: as its empty
/// message, or, when there is a row, as a note row after every endpoint's
/// (the command list shows its empty message only when it has no rows).
pub fn session_palette(
    listed: &[(Host, Listing)],
    killed: &[(Host, u32, String)],
    kill_failed: &[(Host, String)],
    config: &Config,
    now: u64,
) -> SessionPalette {
    let rank = |host: &Host| match host {
        Host::Local => (0, String::new()),
        Host::Remote(_) => match config.profiles.iter().position(|profile| Host::of(profile) == *host) {
            Some(ix) => (ix + 1, String::new()),
            None => (usize::MAX, host.name()),
        },
    };
    let mut endpoints: Vec<&(Host, Listing)> = listed.iter().collect();
    endpoints.sort_by_cached_key(|(host, _)| rank(host));
    let detached = |host: &Host, sessions: &[SessionInfo]| -> Vec<SessionInfo> {
        let mut sessions: Vec<SessionInfo> = sessions
            .iter()
            .filter(|s| !s.attached && !killed.iter().any(|(on, id, key)| on == host && *id == s.session && *key == s.key))
            .cloned()
            .collect();
        sessions.sort_by_key(|s| std::cmp::Reverse((s.started, s.session)));
        sessions
    };
    // Each endpoint's detached sessions, or its inert row.
    let parts: Vec<(&Host, Result<Vec<SessionInfo>, PaletteRow>)> = endpoints
        .into_iter()
        .map(|(host, listing)| {
            let part = match listing {
                Listing::Sessions(sessions) => Ok(detached(host, sessions)),
                Listing::Pending => Err(inert(host.name(), Some("listing sessions…".into()))),
                Listing::Connecting => Err(inert(host.name(), Some("connecting…".into()))),
                Listing::Offline(why) => {
                    let why = why.lines().next().unwrap_or_default();
                    Err(inert(host.name(), Some(format!("offline: {why}"))))
                }
                Listing::Unlisted(why) => {
                    let why = why.lines().next().unwrap_or_default();
                    Err(inert(host.name(), Some(format!("could not list sessions: {why}"))))
                }
            };
            (host, part)
        })
        .collect();
    let lists = |part: &Result<Vec<SessionInfo>, PaletteRow>| part.as_ref().is_ok_and(|sessions| !sessions.is_empty());
    let headings = parts.iter().any(|(host, part)| !host.is_local() && lists(part));
    let any_session = parts.iter().any(|(_, part)| lists(part));

    // Not while an endpoint is still being listed (its sessions may be
    // about to appear), nor when one couldn't be listed (it may have some).
    let unknown = listed.iter().any(|(_, listing)| match listing {
        Listing::Pending | Listing::Unlisted(_) => true,
        Listing::Sessions(_) | Listing::Connecting | Listing::Offline(_) => false,
    });

    let (mut groups, mut rows) = (Vec::new(), Vec::new());
    for (host, part) in parts {
        match part {
            Ok(sessions) if sessions.is_empty() => {}
            Ok(sessions) => {
                let shown = sessions.iter().map(|s| session_row(s, now)).collect();
                groups.push(match (headings, host) {
                    (false, _) => PaletteGroup::unnamed(shown),
                    (true, Host::Local) => PaletteGroup::named("This machine", shown),
                    (true, Host::Remote(_)) => PaletteGroup::named(host.name(), shown),
                });
                rows.extend(sessions.into_iter().map(|info| SessionRow::Session { host: host.clone(), info }));
            }
            Err(row) => {
                groups.push(PaletteGroup::unnamed(vec![row]));
                rows.push(SessionRow::Endpoint(host.clone()));
            }
        }
        for (_, why) in kill_failed.iter().filter(|(on, _)| on == host) {
            let why = why.lines().next().unwrap_or_default();
            groups.push(PaletteGroup::unnamed(vec![inert(host.name(), Some(why.to_string()))]));
            rows.push(SessionRow::KillFailed(host.clone()));
        }
    }
    // After every endpoint's row.
    if !any_session && !rows.is_empty() && !unknown {
        groups.push(PaletteGroup::unnamed(vec![inert(NO_DETACHED.into(), None)]));
        rows.push(SessionRow::Note);
    }
    let empty = rows.is_empty().then(|| NO_DETACHED.into());
    if groups.is_empty() {
        // The command list's one group, as every palette has.
        groups.push(PaletteGroup::unnamed(Vec::new()));
    }
    SessionPalette { groups, rows, empty }
}

/// What [`SessionsModel::view`] gives the palette: its groups and empty
/// message, and the row to keep highlighted (an index across the groups).
#[derive(Debug, PartialEq)]
pub struct SessionsView {
    pub groups: Vec<PaletteGroup>,
    pub empty: Option<SharedString>,
    pub keep: Option<usize>,
}

/// An endpoint's place in a [`SessionsModel`].
#[derive(Debug)]
struct Endpoint {
    host: Host,
    listing: Listing,
    /// The round `listing` is from; 0 for none yet.
    answered: u64,
    /// A kill on it that failed, and why, until its next answer.
    kill_failed: Option<String>,
}

/// What ending a session from the palette gave: its id, and `Err` with why
/// it couldn't be ended.
pub type KillOutcome = (u32, Result<(), String>);

/// The sessions palette's state, apart from GPUI: what each endpoint
/// listed or that it hasn't answered yet, the round of listings they
/// belong to, the sessions killed from it and the highlighted row.
///
/// A round is one set of listings, started when the palette opens, when an
/// endpoint's state changes or after a kill. An answer applies one at a time
/// and only to the current round (an earlier round's is dropped). A new
/// round keeps what each endpoint last listed on display until its answer
/// arrives, so rows don't flicker back to `listing sessions…`; an endpoint
/// that has never answered shows that.
#[derive(Debug)]
pub struct SessionsModel {
    round: u64,
    endpoints: Vec<Endpoint>,
    killed: Vec<(Host, u32, String)>,
    /// The highlighted row, and where it was in `rows`.
    highlighted: Option<(RowKey, usize)>,
    /// The rows of the last [`SessionsModel::view`].
    rows: Vec<SessionRow>,
}

impl SessionsModel {
    /// A model at `round` for `hosts` in order, none of which has answered.
    pub fn new(round: u64, hosts: impl IntoIterator<Item = Host>) -> Self {
        let mut model = Self { round, endpoints: Vec::new(), killed: Vec::new(), highlighted: None, rows: Vec::new() };
        model.start_round(round, hosts);
        model
    }

    pub fn round(&self) -> u64 {
        self.round
    }

    /// Starts `round` for `hosts`, in order. An endpoint already known keeps
    /// its listing until it answers again.
    pub fn start_round(&mut self, round: u64, hosts: impl IntoIterator<Item = Host>) {
        self.round = round;
        let mut known = std::mem::take(&mut self.endpoints);
        self.endpoints = hosts
            .into_iter()
            .map(|host| match known.iter().position(|endpoint| endpoint.host == host) {
                Some(at) => known.swap_remove(at),
                None => Endpoint { host, listing: Listing::Pending, answered: 0, kill_failed: None },
            })
            .collect();
    }

    /// `host`'s answer in `round`, with the outcome of the kill made before
    /// it, if any. Returns whether it applied: an answer from an endpoint
    /// the round doesn't have is dropped, and so is one from another round,
    /// except for a failed kill.
    ///
    /// A failed kill brings the session's row back and adds a row saying
    /// why, which stays until the endpoint's next answer. It counts in any
    /// round, since its row went at once: the kill's answer may be of a
    /// round that another started meanwhile.
    pub fn answer(&mut self, round: u64, host: &Host, listing: Listing, kill: Option<KillOutcome>) -> bool {
        let Some(endpoint) = self.endpoints.iter_mut().find(|endpoint| endpoint.host == *host) else {
            return false;
        };
        let failed = match kill {
            Some((id, Err(why))) => {
                self.killed.retain(|(on, killed, _)| !(on == host && *killed == id));
                Some(format!("could not end session {id}: {why}"))
            }
            Some((_, Ok(()))) | None => None,
        };
        if round != self.round {
            let applied = failed.is_some();
            if failed.is_some() {
                endpoint.kill_failed = failed;
            }
            return applied;
        }
        endpoint.listing = listing;
        endpoint.answered = round;
        endpoint.kill_failed = failed;
        true
    }

    /// Whether every endpoint has answered in the current round: the palette
    /// opens at once when it has, and otherwise after [`OPEN_GRACE`].
    pub fn complete(&self) -> bool {
        self.endpoints.iter().all(|endpoint| endpoint.answered == self.round)
    }

    /// Removes a session's row from now on: it stays gone while its program
    /// exits and the listings catch up.
    pub fn kill(&mut self, host: Host, id: u32, key: String) {
        self.killed.push((host, id, key));
    }

    /// What the palette's list highlights now, as an index into the rows of
    /// the last view (`None`: nothing, as for a list with no enabled row).
    /// The next view keeps that row highlighted.
    pub fn highlight(&mut self, item: Option<usize>) {
        self.highlighted = item.and_then(|at| Some((self.rows.get(at)?.key(), at)));
    }

    /// The rows of the last view, by index across the groups.
    pub fn rows(&self) -> &[SessionRow] {
        &self.rows
    }

    /// The palette for the endpoints' listings so far at `now` (unix
    /// seconds), keeping the highlight where the user left it.
    ///
    /// The highlighted row stays highlighted while it is listed. A row that
    /// is gone (a kill) passes the highlight to the row now at its place,
    /// else to the last row, skipping rows that do nothing.
    pub fn view(&mut self, config: &Config, now: u64) -> SessionsView {
        let listed: Vec<(Host, Listing)> =
            self.endpoints.iter().map(|endpoint| (endpoint.host.clone(), endpoint.listing.clone())).collect();
        let kill_failed: Vec<(Host, String)> = self
            .endpoints
            .iter()
            .filter_map(|endpoint| Some((endpoint.host.clone(), endpoint.kill_failed.clone()?)))
            .collect();
        let shown = session_palette(&listed, &self.killed, &kill_failed, config, now);
        let enabled = |at: &usize| matches!(shown.rows[*at], SessionRow::Session { .. });
        let keep = self.highlighted.as_ref().and_then(|(key, place)| {
            shown.rows.iter().position(|row| row.key() == *key).or_else(|| {
                (*place..shown.rows.len()).find(enabled).or_else(|| (0..shown.rows.len().min(*place)).rev().find(enabled))
            })
        });
        self.highlighted = keep.map(|at| (shown.rows[at].key(), at));
        self.rows = shown.rows;
        SessionsView { groups: shown.groups, empty: shown.empty, keep }
    }
}

/// A row that does nothing.
fn inert(label: String, detail: Option<String>) -> PaletteRow {
    PaletteRow { label: label.into(), detail: detail.map(Into::into), shortcut: None, disabled: true }
}

/// A session's row: its title (else its program), with its directory and
/// age at `now` (unix seconds).
fn session_row(s: &SessionInfo, now: u64) -> PaletteRow {
    let age = crate::reattach::age(s.started, now);
    let detail = match (&s.cwd, age) {
        (Some(cwd), Some(age)) => format!("{cwd} · {age} ago"),
        (Some(cwd), None) => cwd.clone(),
        (None, Some(age)) => format!("{age} ago"),
        (None, None) => String::new(),
    };
    PaletteRow {
        label: crate::reattach::label(s).into(),
        detail: (!detail.is_empty()).then(|| detail.into()),
        shortcut: None,
        disabled: false,
    }
}

#[cfg(test)]
mod tests {
    use brindle_ui::CommandRows;

    use super::*;
    use crate::config::{Profile, RemoteTarget};

    fn ssh(host: &str) -> RemoteTarget {
        RemoteTarget::Ssh {
            host: host.into(),
            user: None,
            port: None,
            identity_file: None,
            ssh_args: Vec::new(),
            connect_at_launch: false,
        }
    }

    fn profile(name: &str, remote: Option<RemoteTarget>) -> Profile {
        Profile { name: name.into(), remote, ..Default::default() }
    }

    /// B9's profile comes before A8's, so B9 is listed first.
    fn config() -> Config {
        let mut config = Config::default();
        config.profiles = vec![
            profile("Shell", None),
            profile("B9", Some(ssh("B9"))),
            profile("htop on A8", Some(ssh("A8"))),
            profile("A8", Some(ssh("A8"))),
        ];
        config
    }

    fn info(session: u32, title: &str, started: u64) -> SessionInfo {
        SessionInfo { session, key: format!("{title}-{session}"), title: title.into(), started, ..Default::default() }
    }

    fn labels(palette: &SessionPalette) -> Vec<(Option<&str>, Vec<&str>)> {
        palette
            .groups
            .iter()
            .map(|group| (text(&group.heading), group.rows.iter().map(|row| row.label.as_ref()).collect()))
            .collect()
    }

    fn text(text: &Option<SharedString>) -> Option<&str> {
        text.as_ref().map(|text| text.as_ref())
    }

    fn a8() -> Host {
        Host::Remote(ssh("A8"))
    }

    fn b9() -> Host {
        Host::Remote(ssh("B9"))
    }

    /// 8.3: this machine's sessions first, then each endpoint's in profile
    /// order (one no profile targets last), each newest first under its
    /// name; attached ones are left out. A session id may be on two
    /// endpoints: each row keeps its own, which its actions target.
    #[test]
    fn sessions_are_grouped_by_endpoint_in_profile_order() {
        let stray = Host::Remote(ssh("C7"));
        let mut attached = info(3, "attached", 300);
        attached.attached = true;
        let listed = [
            (a8(), Listing::Sessions(vec![info(1, "build", 100), info(2, "logs", 200)])),
            (stray, Listing::Sessions(vec![info(1, "stray", 100)])),
            (Host::Local, Listing::Sessions(vec![info(1, "shell", 100), attached, info(2, "vim", 200)])),
            (b9(), Listing::Sessions(vec![info(7, "top", 100)])),
        ];
        let palette = session_palette(&listed, &[], &[], &config(), 400);
        assert_eq!(
            labels(&palette),
            [
                (Some("This machine"), vec!["vim", "shell"]),
                (Some("B9"), vec!["top"]),
                (Some("A8"), vec!["logs", "build"]),
                (Some("C7"), vec!["stray"]),
            ]
        );
        assert_eq!(palette.empty, None);
        let targets: Vec<(String, u32)> = palette
            .rows
            .iter()
            .map(|row| row.session().map(|(host, info)| (host.name(), info.session)).expect("a session's row"))
            .collect();
        let expected = [("this machine", 2), ("this machine", 1), ("B9", 7), ("A8", 2), ("A8", 1), ("C7", 1)];
        assert_eq!(targets, expected.map(|(host, id)| (host.to_string(), id)));
        assert_eq!(text(&palette.groups[1].rows[0].detail), Some("5m ago"));
        let rows = CommandRows::build(&command_groups(&palette.groups), "", None);
        assert_eq!(rows.highlighted_item(), Some(0), "the newest local session");
    }

    /// With this machine's sessions alone the palette is one group with no
    /// heading, as before remote endpoints were listed; with none anywhere
    /// it says so.
    #[test]
    fn this_machine_alone_has_no_heading_and_none_says_so() {
        let listed = [(Host::Local, Listing::Sessions(vec![info(1, "shell", 0)])), (a8(), Listing::Sessions(Vec::new()))];
        let palette = session_palette(&listed, &[], &[], &config(), 0);
        assert_eq!(labels(&palette), [(None, vec!["shell"])]);
        assert_eq!(palette.groups[0].rows[0].detail, None, "no directory and no start time");
        assert_eq!(palette.empty, None, "a filter matching none gets the command list's own message");

        let palette = session_palette(&[(Host::Local, Listing::Sessions(Vec::new()))], &[], &[], &config(), 0);
        assert!(palette.rows.is_empty());
        assert_eq!(text(&palette.empty), Some(NO_DETACHED));
        assert_eq!(command_groups(&palette.groups).len(), 1, "the command list's one group");
    }

    /// An endpoint still connecting or offline is one row that does
    /// nothing: muted and never highlighted, so enter on it opens nothing.
    /// With no session anywhere an inert row says so, after the endpoints'.
    #[test]
    fn connecting_and_offline_endpoints_are_rows_that_do_nothing() {
        let listed = [
            (Host::Local, Listing::Sessions(Vec::new())),
            (a8(), Listing::Offline("ssh: connect to host A8 port 22: Connection refused\nmore".into())),
            (b9(), Listing::Connecting),
        ];
        let palette = session_palette(&listed, &[], &[], &config(), 0);
        assert_eq!(labels(&palette), [(None, vec!["B9"]), (None, vec!["A8"]), (None, vec![NO_DETACHED])]);
        assert_eq!(text(&palette.groups[0].rows[0].detail), Some("connecting…"));
        assert_eq!(
            text(&palette.groups[1].rows[0].detail),
            Some("offline: ssh: connect to host A8 port 22: Connection refused")
        );
        assert_eq!(palette.rows, [SessionRow::Endpoint(b9()), SessionRow::Endpoint(a8()), SessionRow::Note]);
        assert!(palette.rows.iter().all(|row| row.session().is_none()), "no row's actions reach a connection");
        assert_eq!(palette.empty, None, "the command list has rows, so its message wouldn't show");
        assert!(palette.groups.iter().flat_map(|group| &group.rows).all(|row| row.disabled));
        let rows = CommandRows::build(&command_groups(&palette.groups), "", None);
        assert_eq!(rows.highlighted_item(), None, "nothing to confirm");

        // Beside a session the inert row is the endpoint's alone, and the
        // highlight starts on the session.
        let listed = [(a8(), Listing::Sessions(vec![info(4, "build", 0)])), (b9(), Listing::Offline("refused".into()))];
        let palette = session_palette(&listed, &[], &[], &config(), 0);
        assert_eq!(labels(&palette), [(None, vec!["B9"]), (Some("A8"), vec!["build"])]);
        assert_eq!(palette.rows[0], SessionRow::Endpoint(b9()));
        let rows = CommandRows::build(&command_groups(&palette.groups), "", None);
        assert_eq!(rows.highlighted_item(), Some(1));
        assert_eq!(palette.rows[1].session().map(|(host, info)| (host.clone(), info.session)), Some((a8(), 4)));
    }

    /// An offline row shows its reason's first line, and the reasons a
    /// failed launch builds put the cause there, the other lines below
    /// (design decision 1).
    #[test]
    fn an_offline_row_shows_the_cause_of_a_multi_line_reason() {
        let reasons = [
            // ssh 255: the error, then its warning.
            (
                "SSH connection failed: ssh: connect to host A8 port 22: Connection refused\nWarning: Permanently added 'a8'.",
                "offline: SSH connection failed: ssh: connect to host A8 port 22: Connection refused",
            ),
            // A container's launch: the cause, the exit, then the stream's problem.
            (
                "the relay didn't start: Error response from daemon: container abc is not running (docker exited with 1)\n\
                 the stream ended before the frame marker",
                "offline: the relay didn't start: Error response from daemon: container abc is not running (docker exited with 1)",
            ),
        ];
        for (reason, row) in reasons {
            let listed = [(a8(), Listing::Offline(reason.into()))];
            let palette = session_palette(&listed, &[], &[], &config(), 0);
            let offline = palette.groups.iter().flat_map(|group| &group.rows).find(|row| row.label.as_ref() == "A8").unwrap();
            assert_eq!(text(&offline.detail), Some(row), "{reason}");
        }
    }

    /// A killed session's row goes at once, from its endpoint only: the
    /// same key elsewhere stays.
    #[test]
    fn a_killed_session_goes_from_its_endpoint_only() {
        let listed = [
            (Host::Local, Listing::Sessions(vec![info(1, "shell", 0)])),
            (a8(), Listing::Sessions(vec![info(1, "shell", 0), info(2, "build", 0)])),
        ];
        let killed = [(a8(), 1, "shell-1".to_string())];
        let palette = session_palette(&listed, &killed, &[], &config(), 0);
        assert_eq!(labels(&palette), [(Some("This machine"), vec!["shell"]), (Some("A8"), vec!["build"])]);
        let killed = [(a8(), 1, "shell-1".to_string()), (a8(), 2, "build-2".to_string())];
        assert_eq!(labels(&session_palette(&listed, &killed, &[], &config(), 0)), [(None, vec!["shell"])]);
    }

    /// An endpoint still being listed is a row that does nothing, in its
    /// place, and the note that nothing is detached waits for it.
    #[test]
    fn a_pending_endpoint_is_a_row_and_holds_back_the_note() {
        let listed = [(Host::Local, Listing::Sessions(Vec::new())), (a8(), Listing::Pending)];
        let palette = session_palette(&listed, &[], &[], &config(), 0);
        assert_eq!(labels(&palette), [(None, vec!["A8"])]);
        assert_eq!(text(&palette.groups[0].rows[0].detail), Some("listing sessions…"));
        assert_eq!(palette.rows, [SessionRow::Endpoint(a8())]);
        assert_eq!(palette.empty, None, "the command list has a row");
        let rows = CommandRows::build(&command_groups(&palette.groups), "", None);
        assert_eq!(rows.highlighted_item(), None, "a row that does nothing can't be chosen");

        // Another endpoint's sessions list beside it.
        let listed = [(b9(), Listing::Sessions(vec![info(1, "top", 0)])), (a8(), Listing::Pending)];
        let palette = session_palette(&listed, &[], &[], &config(), 0);
        assert_eq!(labels(&palette), [(Some("B9"), vec!["top"]), (None, vec!["A8"])]);

        // Once it has answered with nothing, the note comes last.
        let listed = [(Host::Local, Listing::Sessions(Vec::new())), (a8(), Listing::Sessions(Vec::new()))];
        let palette = session_palette(&listed, &[], &[], &config(), 0);
        assert!(palette.rows.is_empty() && text(&palette.empty) == Some(NO_DETACHED), "no row to put a note after");
    }

    fn hosts() -> [Host; 3] {
        [Host::Local, b9(), a8()]
    }

    /// A model of this machine, B9 and A8 at `round`, that has each of
    /// `answers` (this machine's, B9's and A8's, `None` for none yet).
    fn model(round: u64, answers: [Option<Listing>; 3]) -> SessionsModel {
        let mut model = SessionsModel::new(round, hosts());
        for (host, answer) in hosts().into_iter().zip(answers) {
            if let Some(listing) = answer {
                assert!(model.answer(round, &host, listing, None));
            }
        }
        model
    }

    fn sessions(titles: &[&str]) -> Option<Listing> {
        Some(Listing::Sessions(titles.iter().zip(1..).map(|(title, id)| info(id, title, 100 - id as u64)).collect()))
    }

    /// The labels of `model`'s rows, in order, by `view`.
    fn shown(model: &mut SessionsModel) -> (Vec<String>, Option<usize>) {
        let view = model.view(&config(), 200);
        let labels = view.groups.iter().flat_map(|g| &g.rows).map(|row| row.label.to_string()).collect();
        (labels, view.keep)
    }

    /// An answer from an earlier round is dropped, one from an endpoint the
    /// round doesn't have too; one from the current round applies.
    #[test]
    fn an_answer_from_an_earlier_round_is_dropped() {
        let mut model = SessionsModel::new(1, hosts());
        model.start_round(2, hosts());
        assert!(!model.answer(1, &Host::Local, Listing::Sessions(vec![info(1, "old", 0)]), None));
        assert!(!model.answer(2, &Host::Remote(ssh("C7")), Listing::Connecting, None));
        assert!(!model.complete());
        assert!(model.answer(2, &Host::Local, Listing::Sessions(vec![info(1, "new", 0)]), None));
        assert_eq!(shown(&mut model).0, ["new", "B9", "A8"]);
    }

    /// 5.4: a palette whose endpoints all answer within the grace opens
    /// complete; otherwise it opens with a row for each endpoint still out,
    /// and no note.
    #[test]
    fn the_palette_is_complete_once_every_endpoint_answered() {
        let mut model = model(1, [sessions(&[]), None, None]);
        assert!(!model.complete(), "two endpoints are out");
        assert_eq!(shown(&mut model).0, ["B9", "A8"], "a row each, and no note");
        assert!(model.answer(1, &b9(), Listing::Connecting, None));
        assert!(!model.complete());
        assert_eq!(shown(&mut model).0, ["B9", "A8"], "A8 is still out: no note");
        assert!(model.answer(1, &a8(), Listing::Offline("refused".into()), None));
        assert!(model.complete());
        assert_eq!(shown(&mut model).0, ["B9", "A8", NO_DETACHED], "all answered: the note comes last");

        let mut model = model_with_all_answered();
        assert!(model.complete());
        assert_eq!(shown(&mut model).0, ["shell", "top"].map(String::from));
    }

    fn model_with_all_answered() -> SessionsModel {
        model(1, [sessions(&["shell"]), sessions(&["top"]), sessions(&[])])
    }

    /// A new round keeps what an endpoint last listed on display until it
    /// answers again, and is complete only once every endpoint has.
    #[test]
    fn a_new_round_keeps_what_was_listed_until_the_answer() {
        let mut model = model_with_all_answered();
        model.start_round(2, hosts());
        assert!(!model.complete());
        assert_eq!(shown(&mut model).0, ["shell", "top"], "no flicker back to pending");
        for host in hosts() {
            assert!(model.answer(2, &host, Listing::Sessions(Vec::new()), None));
        }
        assert!(model.complete());
        assert_eq!(shown(&mut model).0, [] as [&str; 0]);
    }

    /// The highlighted session stays highlighted when an answer adds rows
    /// above it.
    #[test]
    fn the_highlighted_session_stays_when_an_answer_adds_rows_above() {
        let mut model = model(1, [sessions(&["a", "b", "c"]), None, sessions(&["x"])]);
        let (labels, keep) = shown(&mut model);
        assert_eq!(labels, ["a", "b", "c", "B9", "x"].map(String::from));
        assert_eq!(keep, None, "nothing was highlighted yet: the list's first");
        // The user moved to "b" (item 1), then B9 answers with sessions above A8's.
        model.highlight(Some(1));
        assert!(model.answer(1, &b9(), Listing::Sessions(vec![info(7, "top", 500), info(8, "vim", 400)]), None));
        let (labels, keep) = shown(&mut model);
        assert_eq!(labels, ["a", "b", "c", "top", "vim", "x"].map(String::from));
        assert_eq!(keep, Some(1));
        // Rows now come in above it: a session is added to this machine's.
        model.highlight(keep);
        assert!(model.answer(1, &Host::Local, Listing::Sessions(vec![info(9, "new", 900), info(1, "a", 99), info(2, "b", 98)]), None));
        let (labels, keep) = shown(&mut model);
        assert_eq!(labels, ["new", "a", "b", "top", "vim", "x"].map(String::from));
        assert_eq!(keep, Some(2), "still \"b\"");
        // A highlighted endpoint-less palette (nothing enabled) keeps nothing.
        model.highlight(None);
        assert_eq!(shown(&mut model).1, None);
    }

    /// A killed row passes its highlight to the row now at its place, else
    /// to the last, and the rows that do nothing are skipped.
    #[test]
    fn a_killed_row_passes_its_highlight_to_the_next_row_else_the_last() {
        let mut model = model(1, [sessions(&["a", "b", "c"]), None, None]);
        model.answer(1, &b9(), Listing::Connecting, None);
        model.answer(1, &a8(), Listing::Connecting, None);
        assert_eq!(shown(&mut model).0, ["a", "b", "c", "B9", "A8"].map(String::from));

        model.highlight(Some(1));
        shown(&mut model);
        model.kill(Host::Local, 2, "b-2".into());
        let (labels, keep) = shown(&mut model);
        assert_eq!(labels, ["a", "c", "B9", "A8"].map(String::from));
        assert_eq!(keep, Some(1), "\"c\" took the place");

        // The last session: the endpoint rows after it do nothing, so the
        // highlight goes back to the last session.
        model.kill(Host::Local, 3, "c-3".into());
        let (labels, keep) = shown(&mut model);
        assert_eq!(labels, ["a", "B9", "A8"].map(String::from));
        assert_eq!(keep, Some(0));

        // Nothing left to highlight: the endpoints' rows and the note do
        // nothing.
        model.kill(Host::Local, 1, "a-1".into());
        let (labels, keep) = shown(&mut model);
        assert_eq!(labels, ["B9", "A8", NO_DETACHED].map(String::from));
        assert_eq!(keep, None);
        assert_eq!(model.rows(), [SessionRow::Endpoint(b9()), SessionRow::Endpoint(a8()), SessionRow::Note]);
    }

    /// The note's place: after every endpoint's row, and only when nothing
    /// is listed anywhere and nothing is pending.
    #[test]
    fn the_note_comes_after_every_endpoint_and_only_once_nothing_is_pending() {
        let mut model = model(1, [sessions(&[]), None, None]);
        assert!(!shown(&mut model).0.contains(&NO_DETACHED.to_string()), "two are pending");
        model.answer(1, &b9(), Listing::Offline("down".into()), None);
        assert!(!shown(&mut model).0.contains(&NO_DETACHED.to_string()), "A8 is pending");
        model.answer(1, &a8(), Listing::Connecting, None);
        let (labels, keep) = shown(&mut model);
        assert_eq!(labels, ["B9", "A8", NO_DETACHED].map(String::from));
        assert_eq!(keep, None);
        assert_eq!(model.rows().last(), Some(&SessionRow::Note));
        // A session anywhere replaces it.
        model.start_round(2, hosts());
        model.answer(2, &a8(), Listing::Sessions(vec![info(1, "build", 0)]), None);
        assert!(!shown(&mut model).0.contains(&NO_DETACHED.to_string()));
    }

    /// This machine answered but couldn't be listed: a row that does
    /// nothing, with the first line of why, in this machine's place.
    #[test]
    fn an_unlisted_endpoint_is_a_disabled_row_with_its_cause() {
        let why = "connecting to the terminal server: brindle-remote is missing\nmore";
        let listed = [(a8(), Listing::Sessions(vec![info(4, "build", 0)])), (Host::Local, Listing::Unlisted(why.into()))];
        let palette = session_palette(&listed, &[], &[], &config(), 0);
        assert_eq!(labels(&palette), [(None, vec!["this machine"]), (Some("A8"), vec!["build"])]);
        let row = &palette.groups[0].rows[0];
        assert_eq!(
            text(&row.detail),
            Some("could not list sessions: connecting to the terminal server: brindle-remote is missing")
        );
        assert!(row.disabled);
        assert_eq!(palette.rows[0], SessionRow::Endpoint(Host::Local));
        let rows = CommandRows::build(&command_groups(&palette.groups), "", None);
        assert_eq!(rows.highlighted_item(), Some(1), "the session, not the failure");
    }

    /// A failure to list isn't an answer that nothing is detached: no note,
    /// alone or beside endpoints that list nothing.
    #[test]
    fn unlisted_does_not_add_the_no_sessions_note() {
        let alone = [(Host::Local, Listing::Unlisted("refused".into()))];
        let palette = session_palette(&alone, &[], &[], &config(), 0);
        assert_eq!(labels(&palette), [(None, vec!["this machine"])]);
        assert_eq!(palette.rows, [SessionRow::Endpoint(Host::Local)]);
        assert_eq!(palette.empty, None);

        let beside = [
            (Host::Local, Listing::Unlisted("refused".into())),
            (a8(), Listing::Sessions(Vec::new())),
            (b9(), Listing::Offline("down".into())),
        ];
        let palette = session_palette(&beside, &[], &[], &config(), 0);
        assert_eq!(labels(&palette), [(None, vec!["this machine"]), (None, vec!["B9"])]);
        assert!(!palette.rows.contains(&SessionRow::Note));
    }

    /// The details of `model`'s rows, in order, by `view`.
    fn details(model: &mut SessionsModel) -> Vec<Option<String>> {
        let view = model.view(&config(), 200);
        view.groups.iter().flat_map(|g| &g.rows).map(|row| row.detail.as_ref().map(ToString::to_string)).collect()
    }

    /// A kill that fails gives its session's row back, and a row after the
    /// endpoint's says why. A failed kill counts in any round.
    #[test]
    fn a_failed_kill_brings_the_row_back_and_adds_its_note() {
        let mut model = model(1, [sessions(&["a", "b", "c"]), sessions(&[]), sessions(&[])]);
        model.kill(Host::Local, 2, "b-2".into());
        assert_eq!(shown(&mut model).0, ["a", "c"]);

        model.start_round(2, hosts());
        let failed = Some((2, Err("the session is still starting".to_string())));
        assert!(model.answer(2, &Host::Local, sessions(&["a", "b", "c"]).unwrap(), failed));
        assert_eq!(shown(&mut model).0, ["a", "b", "c", "this machine"], "no note: sessions are listed");
        assert_eq!(model.rows().last(), Some(&SessionRow::KillFailed(Host::Local)));
        assert_eq!(details(&mut model).last(), Some(&Some("could not end session 2: the session is still starting".into())));

        // A successful kill changes nothing, and neither does a stale round's.
        let mut model = model_with_all_answered();
        model.kill(Host::Local, 1, "shell-1".into());
        model.start_round(2, hosts());
        assert!(!model.answer(1, &Host::Local, sessions(&["shell"]).unwrap(), Some((1, Ok(())))));
        // One that failed counts though the answer is of an older round.
        assert!(model.answer(1, &Host::Local, sessions(&["shell"]).unwrap(), Some((1, Err("refused".into())))));
        assert_eq!(shown(&mut model).0, ["shell", "this machine", "top"]);
        assert!(!model.complete(), "the stale listing didn't count as this round's");
    }

    /// The failed kill's row stays through other endpoints' answers and
    /// goes at its own endpoint's next.
    #[test]
    fn the_note_goes_at_the_endpoints_next_answer() {
        let mut model = model(1, [sessions(&["a"]), sessions(&[]), sessions(&[])]);
        model.kill(Host::Local, 1, "a-1".into());
        model.start_round(2, hosts());
        assert!(model.answer(2, &Host::Local, sessions(&["a"]).unwrap(), Some((1, Err("refused".into())))));
        assert!(model.answer(2, &a8(), Listing::Sessions(Vec::new()), None));
        assert_eq!(shown(&mut model).0, ["a", "this machine"]);

        model.start_round(3, hosts());
        assert_eq!(shown(&mut model).0, ["a", "this machine"], "until this machine answers again");
        assert!(model.answer(3, &Host::Local, sessions(&["a"]).unwrap(), None));
        assert_eq!(shown(&mut model).0, ["a"]);
    }

    /// The highlight follows the rows' identities through a failed kill: a
    /// session highlighted below the killed one stays highlighted when the
    /// killed one comes back above it.
    #[test]
    fn the_highlighted_row_stays_through_a_failed_kill() {
        let mut model = model(1, [sessions(&["a", "b", "c"]), sessions(&[]), sessions(&[])]);
        shown(&mut model);
        model.highlight(Some(2));
        model.kill(Host::Local, 2, "b-2".into());
        let (labels, keep) = shown(&mut model);
        assert_eq!((labels, keep), (vec!["a".to_string(), "c".to_string()], Some(1)), "still \"c\"");

        model.highlight(keep);
        model.start_round(2, hosts());
        assert!(model.answer(2, &Host::Local, sessions(&["a", "b", "c"]).unwrap(), Some((2, Err("refused".into())))));
        let (labels, keep) = shown(&mut model);
        assert_eq!(labels, ["a", "b", "c", "this machine"]);
        assert_eq!(keep, Some(2), "still \"c\"");
    }
}
