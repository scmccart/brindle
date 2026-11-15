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
//! The sessions palette's rows are built here too, purely
//! ([`session_palette`]): grouped by endpoint, with a row that does nothing
//! for each endpoint still connecting or offline.

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
    /// list's own), keeping its filter text. The command list highlights
    /// the first match again.
    pub fn set_groups(&mut self, groups: Vec<PaletteGroup>, empty: Option<SharedString>, cx: &mut Context<Self>) {
        if let Mode::List(list) = &mut self.mode {
            list.command.update(cx, |command, cx| {
                command.set_empty_message(empty, cx);
                command.set_groups(command_groups(&groups), cx);
            });
            list.groups = groups;
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
                let mut out = format!("--- palette list query={:?}\n", command.query(cx));
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
}

/// What a sessions palette row stands for, by row across the groups.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionRow {
    /// A detached session of `host`'s core: enter attaches it, and
    /// `picker_kill` ends it, both on `host`'s connection.
    Session { host: Host, info: SessionInfo },
    /// A row that does nothing: an endpoint connecting or offline, or the
    /// note that nothing is detached.
    Inert,
}

impl SessionRow {
    /// The endpoint whose connection the row's actions use, and its session.
    pub fn session(&self) -> Option<(&Host, &SessionInfo)> {
        match self {
            Self::Session { host, info } => Some((host, info)),
            Self::Inert => None,
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
/// no heading. An endpoint still connecting or offline gets one row that
/// does nothing, saying so. When nothing is detached, the palette says so:
/// as its empty message, or as an inert first row above the endpoints'
/// rows (the command list shows its empty message only when it has no
/// rows).
pub fn session_palette(
    listed: &[(Host, Listing)],
    killed: &[(Host, u32, String)],
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
                Listing::Connecting => Err(inert(host.name(), Some("connecting…".into()))),
                Listing::Offline(why) => {
                    let why = why.lines().next().unwrap_or_default();
                    Err(inert(host.name(), Some(format!("offline: {why}"))))
                }
            };
            (host, part)
        })
        .collect();
    let lists = |part: &Result<Vec<SessionInfo>, PaletteRow>| part.as_ref().is_ok_and(|sessions| !sessions.is_empty());
    let headings = parts.iter().any(|(host, part)| !host.is_local() && lists(part));
    let any_session = parts.iter().any(|(_, part)| lists(part));

    let (mut groups, mut rows) = (Vec::new(), Vec::new());
    if !any_session && parts.iter().any(|(_, part)| part.is_err()) {
        groups.push(PaletteGroup::unnamed(vec![inert(NO_DETACHED.into(), None)]));
        rows.push(SessionRow::Inert);
    }
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
                rows.push(SessionRow::Inert);
            }
        }
    }
    let empty = rows.is_empty().then(|| NO_DETACHED.into());
    if groups.is_empty() {
        // The command list's one group, as every palette has.
        groups.push(PaletteGroup::unnamed(Vec::new()));
    }
    SessionPalette { groups, rows, empty }
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
        let palette = session_palette(&listed, &[], &config(), 400);
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
        let rows = CommandRows::build(&command_groups(&palette.groups), "");
        assert_eq!(rows.highlighted_item(), Some(0), "the newest local session");
    }

    /// With this machine's sessions alone the palette is one group with no
    /// heading, as before remote endpoints were listed; with none anywhere
    /// it says so.
    #[test]
    fn this_machine_alone_has_no_heading_and_none_says_so() {
        let listed = [(Host::Local, Listing::Sessions(vec![info(1, "shell", 0)])), (a8(), Listing::Sessions(Vec::new()))];
        let palette = session_palette(&listed, &[], &config(), 0);
        assert_eq!(labels(&palette), [(None, vec!["shell"])]);
        assert_eq!(palette.groups[0].rows[0].detail, None, "no directory and no start time");
        assert_eq!(palette.empty, None, "a filter matching none gets the command list's own message");

        let palette = session_palette(&[(Host::Local, Listing::Sessions(Vec::new()))], &[], &config(), 0);
        assert!(palette.rows.is_empty());
        assert_eq!(text(&palette.empty), Some(NO_DETACHED));
        assert_eq!(command_groups(&palette.groups).len(), 1, "the command list's one group");
    }

    /// An endpoint still connecting or offline is one row that does
    /// nothing: muted and never highlighted, so enter on it opens nothing.
    /// With no session anywhere an inert row says so first.
    #[test]
    fn connecting_and_offline_endpoints_are_rows_that_do_nothing() {
        let listed = [
            (Host::Local, Listing::Sessions(Vec::new())),
            (a8(), Listing::Offline("ssh: connect to host A8 port 22: Connection refused\nmore".into())),
            (b9(), Listing::Connecting),
        ];
        let palette = session_palette(&listed, &[], &config(), 0);
        assert_eq!(labels(&palette), [(None, vec![NO_DETACHED]), (None, vec!["B9"]), (None, vec!["A8"])]);
        assert_eq!(text(&palette.groups[1].rows[0].detail), Some("connecting…"));
        assert_eq!(
            text(&palette.groups[2].rows[0].detail),
            Some("offline: ssh: connect to host A8 port 22: Connection refused")
        );
        assert_eq!(palette.rows, [SessionRow::Inert, SessionRow::Inert, SessionRow::Inert]);
        assert!(palette.rows.iter().all(|row| row.session().is_none()), "no row's actions reach a connection");
        assert_eq!(palette.empty, None, "the command list has rows, so its message wouldn't show");
        assert!(palette.groups.iter().flat_map(|group| &group.rows).all(|row| row.disabled));
        let rows = CommandRows::build(&command_groups(&palette.groups), "");
        assert_eq!(rows.highlighted_item(), None, "nothing to confirm");

        // Beside a session the inert row is the endpoint's alone, and the
        // highlight starts on the session.
        let listed = [(a8(), Listing::Sessions(vec![info(4, "build", 0)])), (b9(), Listing::Offline("refused".into()))];
        let palette = session_palette(&listed, &[], &config(), 0);
        assert_eq!(labels(&palette), [(None, vec!["B9"]), (Some("A8"), vec!["build"])]);
        assert_eq!(palette.rows[0], SessionRow::Inert);
        let rows = CommandRows::build(&command_groups(&palette.groups), "");
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
            let palette = session_palette(&listed, &[], &config(), 0);
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
        let palette = session_palette(&listed, &killed, &config(), 0);
        assert_eq!(labels(&palette), [(Some("This machine"), vec!["shell"]), (Some("A8"), vec!["build"])]);
        let killed = [(a8(), 1, "shell-1".to_string()), (a8(), 2, "build-2".to_string())];
        assert_eq!(labels(&session_palette(&listed, &killed, &config(), 0)), [(None, vec!["shell"])]);
    }
}
