//! A Brindle window: the tab strip (which doubles as the title bar), the
//! active tab's content, and the palettes.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;

use futures::channel::oneshot;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Action, AnyElement, App, AppContext as _, BorrowAppContext as _, Context, CursorStyle, Decorations, Entity, FocusHandle,
    Focusable, Hsla, InteractiveElement, IntoElement, KeyDownEvent, Modifiers, MouseButton, ParentElement, Pixels, Point,
    Render, ResizeEdge, SharedString, Size, StatefulInteractiveElement, Styled, Subscription,
    Task, Window, WindowBounds, WindowControlArea, canvas, div, point, px, rems,
};
use gpui::EntityId;

use crate::actions::*;
use crate::config::{Config, Profile, TmuxMode, TmuxSessionEnd};
use crate::notify::osc::Source;
use crate::notify::{self, Notifier};
use crate::picker::{Palette, PaletteEvent, PaletteRow, PromptRequest, SubmitFn, ready};
use brindle_ui::{
    ActiveTheme as _, Button, ButtonSize, ButtonVariant, Icon, Modality, Overlay as UiOverlay, TabActivate, TabList,
    TabMove, TabTrigger, TabVariant, TabWidth, TextInput, Tooltip, TooltipFor as _, button_style,
};
use crate::remote::protocol::client::SessionInfo;
use crate::session::{self, RestoredTab, RestoredZoneNode, SessionStore, SnapshotTab, WindowState, ZoneNodeState};
use crate::settings::Settings;
use crate::settings_dialog::{self, SettingsDialog, SettingsEvent};
use crate::terminal::process::Snapshot;
use crate::terminal::status::State;
use crate::terminal::{Terminal, TerminalEvent};
use crate::terminal_view::{GridLayout, PointerStep, TerminalView};
use crate::theme::Theme;
use crate::tmux::protocol::WindowId;
use crate::tmux::{AttachSpec, SessionKey, TmuxEvent, TmuxSession};
use crate::tmux_view::TmuxWindowView;
use crate::zone_view::ZoneTreeView;

const TAB_BAR_HEIGHT: f32 = 38.0;
const RESIZE_BORDER: f32 = 6.0;

pub enum TabContent {
    Zones(Entity<ZoneTreeView>),
    Tmux { session: Entity<TmuxSession>, view: Entity<TmuxWindowView> },
}

/// What a tab was opened as, for session restore.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabOrigin {
    /// A program in a PTY (a profile, a command override, classic tmux).
    Local,
    /// A tmux control-mode window.
    Tmux,
    /// A message, such as an attach failure; never recorded.
    Message,
}

pub struct Tab {
    pub id: u64,
    pub profile: Profile,
    pub content: TabContent,
    pub origin: TabOrigin,
    pub bell: bool,
    /// The most pressing program status (OSC 7501) in the tab's terminal,
    /// or in any pane of its tmux window; see [`Workspace::refresh_status`].
    pub status: Option<State>,
    /// A user-chosen label (`Rename Tab`), shown instead of the automatic
    /// title until changed or cleared.
    manual_title: Option<String>,
    /// Cached displayed title so rendering never probes /proc; see [`Tab::refresh_title`].
    title: SharedString,
    _subscriptions: Vec<Subscription>,
}

impl Tab {
    fn new(
        profile: Profile,
        content: TabContent,
        origin: TabOrigin,
        subscriptions: Vec<Subscription>,
        cx: &mut App,
    ) -> Self {
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let mut tab = Self {
            id: NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            profile,
            content,
            origin,
            bell: false,
            status: None,
            manual_title: None,
            title: SharedString::default(),
            _subscriptions: subscriptions,
        };
        tab.refresh_title(&mut None, cx);
        tab
    }

    /// Recomputes the title: the program's OSC title, else the foreground
    /// process name, else the profile name. On Windows a title the console
    /// built from a program's path counts as none (`cmdline::is_console_title`).
    /// Returns whether it changed.
    /// `snapshot` is the processes running, for the foreground name, taken
    /// only when a title needs it (a program's own title, which can change
    /// thousands of times a second, never does).
    fn refresh_title(&mut self, snapshot: &mut Option<Snapshot>, cx: &mut App) -> bool {
        let zone_changed = match &self.content {
            TabContent::Zones(view) => view.update(cx, |v, cx| v.refresh_titles(snapshot, cx)),
            TabContent::Tmux { .. } => false,
        };
        let mut displayed = std::mem::take(&mut self.title);
        let changed = refresh_displayed_title(&mut displayed, self.manual_title.as_deref(), || {
            self.automatic_title(snapshot, cx)
        });
        self.title = displayed;
        changed || zone_changed
    }

    fn automatic_title(&self, snapshot: &mut Option<Snapshot>, cx: &App) -> String {
        match &self.content {
            TabContent::Zones(view) => view.read(cx).automatic_title(snapshot, cx),
            TabContent::Tmux { view, .. } => view.read(cx).title(cx),
        }
    }

    fn set_manual_title(&mut self, text: &str, snapshot: &mut Option<Snapshot>, cx: &mut App) -> bool {
        self.manual_title = parse_manual_title(text);
        self.refresh_title(snapshot, cx)
    }

    /// Whether this tab shows `session` (and, if given, its `window`).
    fn shows_tmux(&self, session: &Entity<TmuxSession>, window: Option<WindowId>, cx: &App) -> bool {
        matches!(&self.content, TabContent::Tmux { session: s, view }
            if s == session && window.is_none_or(|w| view.read(cx).window_id == w))
    }

    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match &self.content {
            TabContent::Zones(view) => view.read(cx).focus_handle(cx),
            TabContent::Tmux { view, .. } => view.read(cx).focus_handle(cx),
        }
    }

    fn theme(&self, cx: &App) -> Theme {
        match &self.content {
            TabContent::Zones(view) => view.read(cx).theme.clone(),
            TabContent::Tmux { view, .. } => view.read(cx).theme.clone(),
        }
    }

    /// The tab's terminal background and foreground, which its face in the
    /// strip takes while it is active, so it joins the terminal below.
    fn terminal_colors(&self, cx: &App) -> (Hsla, Hsla) {
        let theme = match &self.content {
            TabContent::Zones(view) => &view.read(cx).theme,
            TabContent::Tmux { view, .. } => &view.read(cx).theme,
        };
        (theme.background.hsla(), theme.foreground.hsla())
    }

    fn active_terminal(&self, cx: &App) -> Option<Entity<Terminal>> {
        match &self.content {
            TabContent::Zones(view) => view.read(cx).active_terminal(cx),
            TabContent::Tmux { view, .. } => view.read(cx).active_terminal(cx),
        }
    }

    /// The view of the terminal that has (or last had) focus in this tab.
    fn active_view(&self, cx: &App) -> Option<Entity<TerminalView>> {
        match &self.content {
            TabContent::Zones(view) => view.read(cx).active_view(),
            TabContent::Tmux { view, .. } => view.read(cx).active_view(),
        }
    }

    /// Where the active terminal was last painted.
    fn active_grid(&self, cx: &App) -> Option<GridLayout> {
        self.active_view(cx)?.read(cx).layout.get()
    }

    fn is_tmux(&self) -> bool {
        matches!(self.content, TabContent::Tmux { .. })
    }

    /// The most pressing program status across the tab's terminals.
    fn status_summary(&self, cx: &App) -> Option<State> {
        match &self.content {
            TabContent::Zones(view) => view.read(cx).status_summary(cx),
            TabContent::Tmux { view, .. } => view.read(cx).status_summary(cx),
        }
    }
}

/// What to run in a new tab.
pub struct LaunchRequest {
    pub profile: usize,
    pub command: Option<(String, Vec<String>)>,
    pub cwd: Option<PathBuf>,
}

impl LaunchRequest {
    pub fn profile(profile: usize) -> Self {
        Self { profile, command: None, cwd: None }
    }
}

/// What a launch opens: the window's first content, or a tab a later
/// request adds.
pub enum Launch {
    Request(LaunchRequest),
    /// A window recorded by session restore.
    Restore(WindowState),
    /// `brindle --attach ID`: that detached session, or why it can't be.
    Attach { session: u32, target: Result<SessionInfo, String> },
}

/// A row of the palette that asks about a session another Brindle shows.
#[derive(Clone, Debug, PartialEq, Eq)]
enum OpenChoice {
    NewSession,
    Profile(usize),
}

enum PaletteKind {
    NewTab,
    /// The command palette: its actions by row, and the tab that had focus
    /// when it opened, which they run in.
    Commands { actions: Vec<Box<dyn Action>>, tab_focus: FocusHandle },
    Prompt,
    /// Asks what to do after a session that was all the window showed
    /// ended: row 0 closes the window, row n opens profile n - 1.
    SessionEnded { session: Entity<TmuxSession> },
    /// Asks what to open instead of attaching a session another Brindle
    /// shows: `profile` was asked for.
    SessionOpenElsewhere { choices: Vec<OpenChoice>, profile: usize },
    /// The terminal server's detached sessions, by row: enter attaches one
    /// into a new tab, `picker_kill` ends it.
    Sessions { sessions: Vec<SessionInfo> },
}

/// Kills `kill` (if given), then lists the server's detached sessions,
/// through the process's shared connection. Blocks: off the UI thread.
fn detached_now(kill: Option<u32>) -> anyhow::Result<Vec<SessionInfo>> {
    #[cfg(target_os = "linux")]
    {
        let connection = crate::remote::local::shared()?;
        if let Some(id) = kill
            && let Err(why) = connection.kill(id)
        {
            log::warn!("killing session {id}: {why:#}");
        }
        // A killed session stays listed until its program has exited.
        Ok(connection.list()?.into_iter().filter(|s| !s.attached && Some(s.session) != kill).collect())
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = kill;
        Ok(Vec::new())
    }
}

/// The sessions palette's rows: newest first, each with its directory and
/// age at `now` (unix seconds).
fn session_rows(sessions: &mut [SessionInfo], now: u64) -> Vec<PaletteRow> {
    sessions.sort_by_key(|s| std::cmp::Reverse((s.started, s.session)));
    sessions
        .iter()
        .map(|s| {
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
            }
        })
        .collect()
}

/// The sessions palette's empty message for `count` detached sessions: it
/// says there are none only when there are none; a filter matching none of
/// them gets the command list's own "No results found.".
fn sessions_empty_message(count: usize) -> Option<SharedString> {
    (count == 0).then(|| "No detached sessions".into())
}

/// What a [`PaletteKind`] is, without its payload (entities and focus
/// handles), so the rules that depend only on that are testable for every
/// kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PaletteTag {
    NewTab,
    Commands,
    Prompt,
    SessionEnded,
    SessionOpenElsewhere,
    Sessions,
}

impl PaletteTag {
    #[cfg(test)]
    const ALL: [PaletteTag; 6] = [
        PaletteTag::NewTab,
        PaletteTag::Commands,
        PaletteTag::Prompt,
        PaletteTag::SessionEnded,
        PaletteTag::SessionOpenElsewhere,
        PaletteTag::Sessions,
    ];

    fn of(kind: &PaletteKind) -> Self {
        match kind {
            PaletteKind::NewTab => PaletteTag::NewTab,
            PaletteKind::Commands { .. } => PaletteTag::Commands,
            PaletteKind::Prompt => PaletteTag::Prompt,
            PaletteKind::SessionEnded { .. } => PaletteTag::SessionEnded,
            PaletteKind::SessionOpenElsewhere { .. } => PaletteTag::SessionOpenElsewhere,
            PaletteKind::Sessions { .. } => PaletteTag::Sessions,
        }
    }
}

/// Whether a palette of this kind, opened on its own, ignores a press
/// outside it: those that ask a question the user must answer (after a
/// session ended or opened elsewhere) close only on escape, their close
/// button or a choice. A prompt opened on its own is a plain palette; one
/// that follows a question takes its place in the same slot and keeps its
/// flag (`open_prompt`).
fn ignores_outside_press(tag: PaletteTag) -> bool {
    match tag {
        PaletteTag::SessionEnded | PaletteTag::SessionOpenElsewhere => true,
        PaletteTag::NewTab | PaletteTag::Commands | PaletteTag::Prompt | PaletteTag::Sessions => false,
    }
}

pub struct Workspace {
    tabs: Vec<Tab>,
    active: usize,
    /// The palettes' modal slot. At most one overlay, a palette or the
    /// settings dialog, is open at a time.
    palette: UiOverlay<Palette>,
    /// What the open palette is for, and the subscription to its events.
    palette_kind: Option<PaletteKind>,
    _palette_subscription: Option<Subscription>,
    /// The settings dialog's modal slot. A press on its backdrop does
    /// nothing, so a stray click never throws away a preview or an edit.
    settings: UiOverlay<SettingsDialog>,
    _settings_subscriptions: Vec<Subscription>,
    /// A theme being previewed by the settings dialog, shown on the tabs
    /// that follow the global theme.
    theme_preview: Option<Theme>,
    /// The profile the settings dialog is editing (by its saved name) and
    /// the settings its tabs preview: theme and font size.
    profile_preview: Option<(String, Profile)>,
    focus_handle: FocusHandle,
    /// The tab strip's focus, as a tab list: one keyboard stop, reached
    /// only through `FocusTabStrip` (a press on a tab leaves focus in its
    /// terminal). Focus rests on the active tab; arrows, home and end
    /// switch tabs, and enter or escape give focus back to the terminal.
    strip_focus: FocusHandle,
    /// tmux control-mode sessions attached in this window.
    tmux_sessions: Vec<(Entity<TmuxSession>, Subscription)>,
    last_title: SharedString,
    _title_refresh: Task<()>,
    /// This window's id and creation order, for session restore.
    window_id: u64,
    order: u64,
    /// Restored tmux sessions whose first `WindowActivated` must not take
    /// the active tab (another tab was active when they were recorded).
    quiet_sessions: HashSet<EntityId>,
    /// Restored tmux sessions without tabs yet: their first tab goes after
    /// this tab (`None`: at the front).
    anchors: HashMap<EntityId, Option<u64>>,
    /// Sessions attached from the "session ended" palette, keyed by the new
    /// session, with the ended session whose tabs stay until it shows up.
    replacing: HashMap<EntityId, Entity<TmuxSession>>,
    /// Sessions created from a prompt, which waits to hear whether tmux
    /// attached them.
    creating: HashMap<EntityId, oneshot::Sender<Result<(), String>>>,
    /// The window's size as recorded for restore: the size it opened
    /// with, until the user resizes it (see [`recorded_size`]).
    windowed_size: std::cell::Cell<Size<Pixels>>,
}

impl Workspace {
    pub fn new(launch: Launch, window: &mut Window, cx: &mut Context<Self>) -> Self {
        static NEXT_ORDER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let mut this = Self {
            tabs: Vec::new(),
            active: 0,
            palette: UiOverlay::modal(),
            palette_kind: None,
            _palette_subscription: None,
            settings: UiOverlay::modal().ignore_outside_press(),
            _settings_subscriptions: Vec::new(),
            theme_preview: None,
            profile_preview: None,
            focus_handle: cx.focus_handle(),
            strip_focus: cx.focus_handle(),
            tmux_sessions: Vec::new(),
            last_title: SharedString::default(),
            _title_refresh: Task::ready(()),
            window_id: window.window_handle().window_id().as_u64(),
            order: NEXT_ORDER.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            quiet_sessions: HashSet::new(),
            anchors: HashMap::new(),
            replacing: HashMap::new(),
            creating: HashMap::new(),
            windowed_size: std::cell::Cell::new(window.bounds().size),
        };
        this._title_refresh = cx.spawn_in(window, async move |this, cx| {
            // Foreground processes change without any event (e.g. `vim`
            // starting), so poll their names gently. The same poll records
            // the window for session restore (directories have no events).
            loop {
                cx.background_executor().timer(std::time::Duration::from_secs(1)).await;
                let alive = this.update_in(cx, |this, window, cx| {
                    let mut changed = false;
                    // At most one look at the processes for every tab.
                    let mut snapshot = None;
                    for tab in &mut this.tabs {
                        changed |= tab.refresh_title(&mut snapshot, cx);
                    }
                    if changed {
                        cx.notify();
                    }
                    this.record_session(window, cx);
                });
                if alive.is_err() {
                    break;
                }
            }
        });
        this.open(launch, window, cx);
        this
    }

    /// Opens what a launch asks for in this window.
    pub fn open(&mut self, launch: Launch, window: &mut Window, cx: &mut Context<Self>) {
        match launch {
            Launch::Request(request) => self.launch(request, window, cx),
            Launch::Restore(state) => self.restore(state, window, cx),
            Launch::Attach { session, target } => self.launch_attach(session, target, window, cx),
        }
    }

    /// The tab `brindle --attach` opens: the session attached, or a message
    /// naming it and why not.
    fn launch_attach(&mut self, session: u32, target: Result<SessionInfo, String>, window: &mut Window, cx: &mut Context<Self>) {
        match target {
            Ok(info) => self.attach_detached_here(info, window, cx),
            Err(why) => {
                let config = Self::settings(cx).clone();
                let profile = config.profiles[config.default_profile_index()].clone();
                let text = format!("\x1b[31mBrindle could not attach session {session}:\x1b[0m\r\n{why}\r\n");
                let theme = config.profile_theme(&profile);
                let terminal = cx.new(|cx| Terminal::message(&text, theme, &config, cx));
                self.insert_terminal_tab(profile, terminal, true, window, cx);
            }
        }
    }

    /// Opens a recorded window's tabs. Local tabs open in order; each tmux
    /// session is attached once and its windows arrive later, placed after
    /// the tab recorded before it. The recorded active tab is activated if
    /// it is local; for a tmux session, tmux's current window decides.
    fn restore(&mut self, state: WindowState, window: &mut Window, cx: &mut Context<Self>) {
        let config = Self::settings(cx).clone();
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let mut previous: Option<u64> = None;
        let mut active_tab = None;
        for (ix, entry) in state.tabs.iter().enumerate() {
            match session::resolve_tab(entry, &config, &home) {
                RestoredTab::Local { profile, cwd, title, zones, session } => {
                    let found = match zones {
                        Some(_) => None,
                        None => Self::take_detached(session.as_ref(), cx),
                    };
                    match (zones, found) {
                        (Some(zones), _) => self.restore_local_zones(profile, zones, window, cx),
                        #[cfg(target_os = "linux")]
                        (None, Some(info)) if config.profiles[profile].tmux == TmuxMode::None => {
                            let profile = config.profiles[profile].clone();
                            let terminal = Self::attached_terminal(&info, &profile, &config, cx);
                            self.insert_terminal_tab(profile, terminal, true, window, cx);
                        }
                        _ => self.launch(LaunchRequest { profile, command: None, cwd: Some(cwd) }, window, cx),
                    }
                    let Some(tab) = self.tabs.get_mut(self.active) else { continue };
                    if let Some(title) = title {
                        tab.set_manual_title(&title, &mut None, cx);
                    }
                    previous = Some(tab.id);
                    if ix == state.active {
                        active_tab = Some(tab.id);
                    }
                }
                RestoredTab::Tmux { profile, session } => {
                    let mut profile = config.profiles[profile].clone();
                    profile.tmux_session = Some(session);
                    if let Some(session) = self.attach_tmux(profile, None, window, cx) {
                        let id = session.entity_id();
                        self.anchors.insert(id, previous);
                        if ix != state.active {
                            self.quiet_sessions.insert(id);
                        }
                    }
                }
            }
        }
        if let Some(ix) = active_tab.and_then(|id| self.tabs.iter().position(|t| t.id == id)) {
            self.activate(ix, window, cx);
        }
    }

    fn restore_local_zones(
        &mut self,
        profile_ix: usize,
        zones: RestoredZoneNode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let config = Self::settings(cx).clone();
        let Some(profile) = config.profiles.get(profile_ix).cloned() else { return };
        let command = match profile.tmux {
            TmuxMode::Plain => {
                let mut args = profile.tmux_args.clone();
                args.extend([
                    "new-session".into(),
                    "-A".into(),
                    "-s".into(),
                    profile.tmux_session_name().to_string(),
                ]);
                Some(("tmux".into(), args))
            }
            _ => None,
        };
        let (tree, leaves, active_id) = zones.into_tree();
        let terminals: Vec<_> = leaves
            .into_iter()
            .map(|(id, cwd, session)| {
                let terminal = Self::restored_terminal(&profile, command.clone(), cwd, session.as_ref(), &config, cx);
                (id, terminal)
            })
            .collect();
        let font_size = profile.font_size;
        let theme = self.tab_theme(&profile, &config);
        let p_clone = profile.clone();
        let view = cx.new(|cx| {
            ZoneTreeView::from_restored(p_clone, theme, font_size, tree, terminals, active_id, window, cx)
        });
        let subscriptions = Self::zone_tab_subscriptions(&view, window, cx);
        let tab = Tab::new(profile, TabContent::Zones(view), TabOrigin::Local, subscriptions, cx);
        self.insert_tab(tab, None, window, cx);
    }

    /// This window as session restore records it.
    pub fn snapshot(&self, window: &Window, cx: &App) -> WindowState {
        // A session tmux hasn't named yet can't be reattached by name.
        let session_tab = |session: &Entity<TmuxSession>| {
            let s = session.read(cx);
            match &s.session_name {
                Some(name) => SnapshotTab::Tmux {
                    profile: s.profile.name.clone(),
                    session: name.clone(),
                    key: session.entity_id().as_u64(),
                },
                None => SnapshotTab::Skip,
            }
        };
        // Sessions still attaching have no tabs yet; record them where
        // their tabs will go, so a quit meanwhile doesn't lose them.
        let pending: Vec<&Entity<TmuxSession>> = self
            .tmux_sessions
            .iter()
            .map(|(s, _)| s)
            .filter(|s| !self.tabs.iter().any(|t| t.shows_tmux(s, None, cx)))
            .collect();
        let pending_after = |anchor: Option<u64>| -> Vec<SnapshotTab> {
            pending
                .iter()
                .filter(|s| self.anchors.get(&s.entity_id()) == Some(&anchor))
                .map(|s| session_tab(s))
                .collect()
        };
        let mut tabs: Vec<SnapshotTab> = pending_after(None);
        let mut active = None;
        for (ix, tab) in self.tabs.iter().enumerate() {
            if ix == self.active {
                active = Some(tabs.len());
            }
            tabs.push(match (&tab.origin, &tab.content) {
                (TabOrigin::Message, _) => SnapshotTab::Skip,
                (_, TabContent::Tmux { session, .. }) => session_tab(session),
                (_, TabContent::Zones(view)) => {
                    let z = view.read(cx);
                    let zones = ZoneNodeState::from_tree(z.tree(), z.active_id(), &mut |id| {
                        (z.zone_working_directory(id, cx), z.zone_server_session(id, cx))
                    });
                    // A single zone's session is the tab's.
                    let session = zones.is_none().then(|| z.zone_server_session(z.active_id(), cx)).flatten();
                    SnapshotTab::Local {
                        profile: tab.profile.name.clone(),
                        remote: tab.profile.remote.clone(),
                        cwd: z.active_terminal(cx).and_then(|t| t.read(cx).working_directory()),
                        title: tab.manual_title.clone(),
                        zones,
                        session,
                    }
                }
            });
            tabs.extend(pending_after(Some(tab.id)));
        }
        tabs.extend(pending.iter().filter(|s| !self.anchors.contains_key(&s.entity_id())).map(|s| session_tab(s)));
        let (tabs, active) = session::collapse(&tabs, active.unwrap_or(0));
        let maximized = matches!(window.window_bounds(), WindowBounds::Maximized(_));
        if !maximized {
            let outer = window.window_bounds().get_bounds().size;
            let inner = window.inner_window_bounds().get_bounds().size;
            self.windowed_size.set(recorded_size(self.windowed_size.get(), outer, inner));
        }
        let size = self.windowed_size.get();
        WindowState {
            width: Some(f32::from(size.width).round()),
            height: Some(f32::from(size.height).round()),
            maximized,
            active,
            tabs,
        }
    }

    /// Stores this window's snapshot, when session restore is on and this
    /// process owns the session.
    pub fn record_session(&self, window: &Window, cx: &mut Context<Self>) {
        if !cx.update_global(|store: &mut SessionStore, _| store.is_owner()) {
            return;
        }
        let state = self.snapshot(window, cx);
        let (id, order) = (self.window_id, self.order);
        cx.update_global(|store: &mut SessionStore, _| store.record(id, order, state));
    }

    fn settings(cx: &App) -> &Config {
        &Settings::get(cx).config
    }

    pub fn active_theme(&self, cx: &App) -> Theme {
        self.tabs
            .get(self.active)
            .map(|t| t.theme(cx))
            .unwrap_or_else(|| Self::settings(cx).theme(None))
    }

    /// The active directory only has meaning to a profile on the same host
    /// or container. In particular, never pass a Windows path to remote tmux.
    fn active_cwd_for(&self, profile: &Profile, cx: &App) -> Option<PathBuf> {
        let tab = self.tabs.get(self.active)?;
        if tab.profile.remote != profile.remote {
            return None;
        }
        tab.active_terminal(cx)?.read(cx).working_directory()
    }

    /// Opens a tab for the request. tmux control-mode profiles attach a
    /// session whose windows arrive as tabs asynchronously.
    pub fn launch(&mut self, request: LaunchRequest, window: &mut Window, cx: &mut Context<Self>) {
        let config = Self::settings(cx).clone();
        let Some(profile) = config.profiles.get(request.profile).cloned() else {
            log::warn!("no profile #{}", request.profile + 1);
            return;
        };
        let requested_cwd = request.cwd.filter(|cwd| {
            if profile.remote.is_none() {
                return true;
            }
            let cwd = cwd.to_string_lossy();
            cwd.starts_with('/') || cwd == "~" || cwd.starts_with("~/")
        });

        if profile.tmux == TmuxMode::Control && request.command.is_none() {
            let cwd = requested_cwd.or_else(|| profile.working_directory());
            self.attach_tmux(profile, cwd, window, cx);
            return;
        }

        let command = request.command.or_else(|| match profile.tmux {
            TmuxMode::Plain => {
                let mut args = profile.tmux_args.clone();
                args.extend([
                    "new-session".into(),
                    "-A".into(),
                    "-s".into(),
                    profile.tmux_session_name().to_string(),
                ]);
                Some(("tmux".into(), args))
            }
            _ => None,
        });

        let cwd = requested_cwd.or_else(|| {
            // New tabs follow the current directory only on the same endpoint,
            // unless the destination profile pins one.
            if profile.cwd.is_none() { self.active_cwd_for(&profile, cx) } else { None }
        });

        let terminal = cx.new(|cx| Terminal::spawn(&profile, command, cwd, &config, cx));
        let font_size = profile.font_size;
        let theme = self.tab_theme(&profile, &config);
        let p_clone = profile.clone();
        let view = cx.new(|cx| ZoneTreeView::new(p_clone, theme, font_size, terminal, window, cx));
        let subscriptions = Self::zone_tab_subscriptions(&view, window, cx);
        let tab = Tab::new(profile, TabContent::Zones(view), TabOrigin::Local, subscriptions, cx);
        self.insert_tab(tab, None, window, cx);
    }

    /// A local tab showing `terminal` alone, inserted after the active tab,
    /// and activated when `activate`.
    fn insert_terminal_tab(
        &mut self,
        profile: Profile,
        terminal: Entity<Terminal>,
        activate: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> u64 {
        let config = Self::settings(cx).clone();
        let theme = self.tab_theme(&profile, &config);
        let font_size = profile.font_size;
        let p_clone = profile.clone();
        let view = cx.new(|cx| ZoneTreeView::new(p_clone, theme, font_size, terminal, window, cx));
        let subscriptions = Self::zone_tab_subscriptions(&view, window, cx);
        let tab = Tab::new(profile, TabContent::Zones(view), TabOrigin::Local, subscriptions, cx);
        let id = tab.id;
        if activate {
            self.insert_tab(tab, None, window, cx);
        } else {
            let at = self.tabs.len();
            self.insert_tab_inactive(tab, Some(at), cx);
        }
        id
    }

    /// The detached session a restored zone recorded, when the launch's
    /// listing still has it ([`crate::reattach::Detached`]).
    fn take_detached(recorded: Option<&session::ServerSession>, cx: &mut App) -> Option<SessionInfo> {
        let recorded = recorded?;
        cx.try_global::<crate::reattach::Detached>()?;
        cx.update_global(|detached: &mut crate::reattach::Detached, _| detached.take(recorded))
    }

    /// A terminal attached to the detached session `info`, closing as
    /// `profile` says.
    #[cfg(target_os = "linux")]
    fn attached_terminal(info: &SessionInfo, profile: &Profile, config: &Config, cx: &mut Context<Self>) -> Entity<Terminal> {
        // At the session's own size, so attaching resizes nothing; a core
        // that predates sizes in `LIST` reports 0.
        let default = crate::terminal::GridSize::default();
        let (cols, rows) = if info.cols > 0 && info.rows > 0 { (info.cols, info.rows) } else { (default.cols, default.rows) };
        let request = crate::remote::local::AttachRequest { session: info.session, key: info.key.clone(), cols, rows };
        let what = crate::reattach::label(info);
        cx.new(|cx| Terminal::attach(request, what, profile, config, cx))
    }

    /// A restored zone's terminal: its recorded session attached when the
    /// launch found it still detached, else `profile` spawned in `cwd`.
    fn restored_terminal(
        profile: &Profile,
        command: Option<(String, Vec<String>)>,
        cwd: PathBuf,
        recorded: Option<&session::ServerSession>,
        config: &Config,
        cx: &mut Context<Self>,
    ) -> Entity<Terminal> {
        // Only a profile's own program runs in a reattachable session.
        let found = if command.is_none() { Self::take_detached(recorded, cx) } else { None };
        #[cfg(target_os = "linux")]
        if let Some(info) = found {
            return Self::attached_terminal(&info, profile, config, cx);
        }
        #[cfg(not(target_os = "linux"))]
        let _ = found;
        cx.new(|cx| Terminal::spawn(profile, command, Some(cwd), config, cx))
    }

    /// Opens a tab for each of `sessions` (detached sessions nothing
    /// restored took), after the others and in this order, each with the
    /// profile its meta names (the default one when it's gone). The active
    /// tab stays the same.
    pub fn attach_detached(&mut self, sessions: Vec<SessionInfo>, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(target_os = "linux")]
        {
            let config = Self::settings(cx).clone();
            for info in sessions {
                let index = info
                    .profile
                    .as_deref()
                    .and_then(|name| config.profile_index(name))
                    .unwrap_or_else(|| config.default_profile_index());
                let profile = config.profiles[index].clone();
                let terminal = Self::attached_terminal(&info, &profile, &config, cx);
                let activate = self.tabs.is_empty();
                self.insert_terminal_tab(profile, terminal, activate, window, cx);
            }
        }
        #[cfg(not(target_os = "linux"))]
        let _ = (sessions, window, cx);
    }

    /// Inserts `tab` at `index` (default: after the active tab) and activates it.
    fn insert_tab(&mut self, tab: Tab, index: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let index = self.insert_tab_inactive(tab, index, cx);
        self.activate(index, window, cx);
    }

    /// Inserts `tab` at `index` without activating it; the active tab stays
    /// the same. Used for tmux windows, which come to the front only when
    /// tmux makes them current.
    fn insert_tab_inactive(&mut self, tab: Tab, index: Option<usize>, cx: &mut Context<Self>) -> usize {
        let index = index.unwrap_or(if self.tabs.is_empty() { 0 } else { self.active + 1 }).min(self.tabs.len());
        if !self.tabs.is_empty() && index <= self.active {
            self.active += 1;
        }
        if self.theme_preview.is_some() || self.profile_preview.is_some() {
            let profile = self.effective_profile(&tab.profile);
            let theme = self.tab_theme(profile, Self::settings(cx));
            set_tab_theme(&tab, theme, profile.font_size, cx);
        }
        self.tabs.insert(index, tab);
        cx.notify();
        index
    }

    /// Attaches `profile`'s session, or focuses it if the window already
    /// shows it. Returns the newly attached session.
    fn attach_tmux(
        &mut self,
        profile: Profile,
        cwd: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<TmuxSession>> {
        if let Some(session) = SessionKey::for_profile(&profile).and_then(|key| self.find_session(&key, cx)) {
            self.show_session(&session, window, cx);
            return None;
        }
        Some(self.create_session(profile, AttachSpec::Named, cwd, None, window, cx))
    }

    /// Starts a control client for `profile`'s server as `spec` says.
    /// `reply` hears whether tmux attached it (its first tab arrived) or
    /// refused, with tmux's message.
    fn create_session(
        &mut self,
        profile: Profile,
        spec: AttachSpec,
        cwd: Option<PathBuf>,
        reply: Option<oneshot::Sender<Result<(), String>>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<TmuxSession> {
        let config = Self::settings(cx).clone();
        let cwd = cwd.or_else(|| self.active_cwd_for(&profile, cx));
        let session = cx.new(|cx| TmuxSession::attach(profile, spec, cwd, &config, cx));
        let subscription = cx.subscribe_in(&session, window, |this, session, event, window, cx| {
            this.on_tmux_event(session.clone(), event, window, cx)
        });
        self.tmux_sessions.push((session.clone(), subscription));
        if let Some(reply) = reply {
            self.creating.insert(session.entity_id(), reply);
        }
        session
    }

    /// The window's session for `key`, if it shows one.
    fn find_session(&self, key: &SessionKey, cx: &App) -> Option<Entity<TmuxSession>> {
        self.tmux_sessions.iter().map(|(s, _)| s).find(|s| s.read(cx).key().as_ref() == Some(key)).cloned()
    }

    /// The sessions the window shows.
    fn open_sessions(&self, cx: &App) -> Vec<SessionKey> {
        self.tmux_sessions.iter().filter_map(|(s, _)| s.read(cx).key()).collect()
    }

    /// Activates `session`'s tab for tmux's current window, else its first.
    fn show_session(&mut self, session: &Entity<TmuxSession>, window: &mut Window, cx: &mut Context<Self>) {
        let current = session.read(cx).active_window;
        let ix = current
            .and_then(|w| self.tabs.iter().position(|t| t.shows_tmux(session, Some(w), cx)))
            .or_else(|| self.tabs.iter().position(|t| t.shows_tmux(session, None, cx)));
        if let Some(ix) = ix {
            self.activate(ix, window, cx);
        }
    }

    /// Opens profile `ix` in a new tab, or activates its tmux session's tab
    /// when the window already shows it.
    fn open_profile(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(profile) = Self::settings(cx).profiles.get(ix).cloned() else { return };
        match SessionKey::for_profile(&profile).and_then(|key| self.find_session(&key, cx)) {
            Some(session) => self.show_session(&session, window, cx),
            None => self.launch(LaunchRequest::profile(ix), window, cx),
        }
    }

    /// Asks what to open instead of profile `profile`'s session `name`,
    /// which another Brindle shows.
    fn ask_session_open(&mut self, profile: usize, name: &str, window: &mut Window, cx: &mut Context<Self>) {
        let open = self.open_sessions(cx);
        let (rows, choices) = session_open_rows(&Self::settings(cx).profiles, profile, &open).into_iter().unzip();
        let placeholder = format!("tmux session \"{name}\" is open in another Brindle");
        let palette = cx.new(|cx| Palette::list(rows, placeholder, cx));
        self.show_palette(palette, PaletteKind::SessionOpenElsewhere { choices, profile }, window, cx);
    }

    /// Asks for a name, then creates that tmux session on `profile`'s
    /// server, in the window.
    fn ask_new_session(&mut self, profile: Profile, window: &mut Window, cx: &mut Context<Self>) {
        let handle = window.window_handle().downcast::<Workspace>();
        let cwd = self.active_cwd_for(&profile, cx);
        let submit: SubmitFn = Rc::new(move |name, cx| {
            let name = Some(name.trim().to_string()).filter(|n| !n.is_empty());
            let (reply, result) = oneshot::channel();
            let (profile, cwd) = (profile.clone(), cwd.clone());
            // The palette is updating the window: attach once it's done.
            cx.defer(move |cx| {
                if let Some(handle) = handle {
                    handle
                        .update(cx, |ws, window, cx| {
                            ws.create_session(profile, AttachSpec::New(name), cwd, Some(reply), window, cx);
                        })
                        .ok();
                }
            });
            result
        });
        let label = "New tmux session name (empty: tmux picks one)".into();
        self.open_prompt(PromptRequest { label, initial: String::new(), submit }, window, cx);
    }

    fn tmux_new_session(&mut self, _: &TmuxNewSession, window: &mut Window, cx: &mut Context<Self>) {
        let config = Self::settings(cx);
        let tab_profile = self.tabs.get(self.active).and_then(|t| match &t.content {
            TabContent::Tmux { session, .. } => Some(session.read(cx).profile.clone()),
            TabContent::Zones(_) => None,
        });
        let default = &config.profiles[config.default_profile_index()];
        let profile = tab_profile
            .or_else(|| (default.tmux == TmuxMode::Control).then(|| default.clone()))
            .or_else(|| config.profiles.iter().find(|p| p.tmux == TmuxMode::Control).cloned());
        if let Some(profile) = profile {
            self.ask_new_session(profile, window, cx);
        }
    }

    fn on_tmux_event(
        &mut self,
        session: Entity<TmuxSession>,
        event: &TmuxEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            TmuxEvent::WindowAdded(window_id) => {
                let window_id = *window_id;
                if self.tabs.iter().any(|t| t.shows_tmux(&session, Some(window_id), cx)) {
                    return;
                }
                let profile = session.read(cx).profile.clone();
                let theme = self.tab_theme(&profile, Self::settings(cx));
                let view = cx.new(|cx| TmuxWindowView::new(session.clone(), window_id, theme, profile.font_size, window, cx));
                let subscriptions = vec![
                    cx.observe(&view, |this, _, cx| {
                        // Window renames arrive through the view.
                        for tab in this.tabs.iter_mut().filter(|t| t.is_tmux()) {
                            tab.refresh_title(&mut None, cx);
                        }
                        cx.notify();
                    }),
                    cx.subscribe_in(&view, window, |this, _, request: &PromptRequest, window, cx| {
                        this.open_prompt(request.clone(), window, cx)
                    }),
                    // Bells, notifications and status from the window's panes.
                    cx.subscribe_in(&view, window, |this, view, event: &TerminalEvent, window, cx| {
                        let tab = this.tabs.iter().position(|t| {
                            matches!(&t.content, TabContent::Tmux { view: v, .. } if v == view)
                        });
                        if let Some(ix) = tab {
                            this.on_attention(ix, event, window, cx);
                        }
                    }),
                ];
                let tab = Tab::new(
                    profile,
                    TabContent::Tmux { session: session.clone(), view },
                    TabOrigin::Tmux,
                    subscriptions,
                    cx,
                );
                // Keep a session's tabs together, in tmux's window order. A
                // restored session's first tab goes where it was recorded.
                let anchor = self.anchors.remove(&session.entity_id());
                let index = match self.tabs.iter().rposition(|t| t.shows_tmux(&session, None, cx)) {
                    Some(last) => Some(last + 1),
                    None => match anchor {
                        Some(Some(id)) => self.tabs.iter().position(|t| t.id == id).map(|i| i + 1),
                        Some(None) => Some(0),
                        None => None,
                    },
                };
                self.insert_tab_inactive(tab, index, cx);
                self.finish_replacing(&session, window, cx);
                if let Some(reply) = self.creating.remove(&session.entity_id()) {
                    reply.send(Ok(())).ok();
                }
            }
            TmuxEvent::WindowClosed(window_id) => {
                // The window's last tab, for the session's last window: the
                // session is ending, and `Detached` decides what follows.
                if self.tabs.len() == 1 && self.tabs[0].shows_tmux(&session, Some(*window_id), cx) {
                    return;
                }
                if let Some(ix) = self.tabs.iter().position(|t| t.shows_tmux(&session, Some(*window_id), cx)) {
                    self.remove_tab(ix, window, cx);
                }
            }
            TmuxEvent::WindowActivated(window_id) => {
                // A restored session that wasn't the active tab stays in the
                // background once; nothing is sent to tmux either way.
                if self.quiet_sessions.remove(&session.entity_id()) {
                    return;
                }
                // Always activate: tabs are added without activation, so the
                // current window's tab may already be `active` yet unfocused.
                if let Some(ix) = self.tabs.iter().position(|t| t.shows_tmux(&session, Some(*window_id), cx)) {
                    self.activate(ix, window, cx);
                }
            }
            TmuxEvent::Failed(reason) => {
                self.tmux_sessions.retain(|(s, _)| *s != session);
                self.forget_restore(&session);
                // A prompt that created the session shows tmux's message;
                // if it has been dismissed, a tab explains instead.
                if let Some(reply) = self.creating.remove(&session.entity_id())
                    && reply.send(Err(reason.clone())).is_ok()
                {
                    self.finish_replacing(&session, window, cx);
                    return;
                }
                let profile = session.read(cx).profile.clone();
                let text = format!(
                    "\x1b[1mtmux ({}) ended before attaching:\x1b[0m {reason}\r\n\r\n\
                     Is tmux installed and on PATH? Profile settings: tmux_session, tmux_args, command.\r\n",
                    profile.name
                );
                self.open_message_tab(profile, &text, window, cx);
                self.finish_replacing(&session, window, cx);
            }
            TmuxEvent::ShownByAnotherBrindle(name) => {
                self.tmux_sessions.retain(|(s, _)| *s != session);
                self.forget_restore(&session);
                self.creating.remove(&session.entity_id());
                // An ended session it was to replace stays until the user
                // picks something (see `orphaned_sessions`).
                self.replacing.remove(&session.entity_id());
                let profile = session.read(cx).profile.name.clone();
                match Self::settings(cx).profiles.iter().position(|p| p.name == profile) {
                    Some(ix) => {
                        self.ask_session_open(ix, &name, window, cx)
                    }
                    None => self.close_orphans(window, cx),
                }
            }
            TmuxEvent::Detached(reason) => {
                log::info!("tmux session detached: {reason}");
                self.creating.remove(&session.entity_id());
                self.finish_replacing(&session, window, cx);
                if self.tabs.is_empty() {
                    return;
                }
                self.tmux_sessions.retain(|(s, _)| *s != session);
                self.forget_restore(&session);
                if self.tabs.iter().all(|t| t.shows_tmux(&session, None, cx)) {
                    // Nothing else would be left: the setting decides.
                    match Self::settings(cx).tmux_session_end {
                        TmuxSessionEnd::CloseWindow => {}
                        TmuxSessionEnd::NewTab => {
                            if let Some(profile) = Self::fallback_profile(Self::settings(cx)) {
                                self.launch(LaunchRequest::profile(profile), window, cx);
                            }
                        }
                        TmuxSessionEnd::Ask => {
                            // The ended tabs stay until the user chooses.
                            self.ask_session_ended(session, window, cx);
                            return;
                        }
                    }
                }
                self.remove_session_tabs(&session, window, cx);
            }
        }
    }

    /// The profile a window falls back to when tmux leaves it empty: the
    /// default, or the first non-tmux profile when the default would attach
    /// a control-mode session again.
    fn fallback_profile(config: &Config) -> Option<usize> {
        let default = config.default_profile_index();
        if config.profiles[default].tmux == TmuxMode::Control {
            config.profiles.iter().position(|p| p.tmux == TmuxMode::None)
        } else {
            Some(default)
        }
    }

    /// Closes every tab of `session`. Removing the last tab closes the
    /// window, as for any tab.
    fn remove_session_tabs(&mut self, session: &Entity<TmuxSession>, window: &mut Window, cx: &mut Context<Self>) {
        while let Some(ix) = self.tabs.iter().position(|t| t.shows_tmux(session, None, cx)) {
            self.remove_tab(ix, window, cx);
            if self.tabs.is_empty() {
                return;
            }
        }
    }

    fn ask_session_ended(&mut self, session: Entity<TmuxSession>, window: &mut Window, cx: &mut Context<Self>) {
        let rows = session_ended_rows(&Self::settings(cx).profiles);
        let placeholder = format!("tmux session \"{}\" ended", session.read(cx).session_name.as_deref().unwrap_or("?"));
        let palette = cx.new(|cx| Palette::list(rows, placeholder, cx));
        self.show_palette(palette, PaletteKind::SessionEnded { session }, window, cx);
    }

    /// `session` showed up (or failed): the ended session it replaces goes.
    fn finish_replacing(&mut self, session: &Entity<TmuxSession>, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ended) = self.replacing.remove(&session.entity_id()) {
            self.remove_session_tabs(&ended, window, cx);
        }
    }

    fn forget_restore(&mut self, session: &Entity<TmuxSession>) {
        self.quiet_sessions.remove(&session.entity_id());
        self.anchors.remove(&session.entity_id());
    }

    /// Sessions whose tabs are shown but which are no longer attached, and
    /// that no attaching session waits to replace: a session that ended,
    /// whose replacement another Brindle turned out to show.
    fn orphaned_sessions(&self) -> Vec<Entity<TmuxSession>> {
        let mut orphans: Vec<Entity<TmuxSession>> = Vec::new();
        for tab in &self.tabs {
            if let TabContent::Tmux { session, .. } = &tab.content
                && !self.tmux_sessions.iter().any(|(s, _)| s == session)
                && !self.replacing.values().any(|ended| ended == session)
                && !orphans.contains(session)
            {
                orphans.push(session.clone());
            }
        }
        orphans
    }

    /// Closes the tabs of [`orphaned_sessions`], then the window if nothing
    /// is left in it or on its way (a second Brindle's window that asked
    /// about a session, and was dismissed).
    fn close_orphans(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        for session in self.orphaned_sessions() {
            self.remove_session_tabs(&session, window, cx);
            if self.tabs.is_empty() {
                return;
            }
        }
        if self.tabs.is_empty() && self.tmux_sessions.is_empty() {
            let id = self.window_id;
            cx.update_global(|store: &mut SessionStore, _| store.forget(id));
            window.remove_window();
        }
    }

    /// Opens profile `ix` in a tab, in place of any [`orphaned_sessions`]:
    /// their tabs close once it shows up.
    fn launch_replacing_orphans(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let orphan = self.orphaned_sessions().into_iter().next();
        let profile = Self::settings(cx).profiles.get(ix).cloned();
        match profile {
            Some(profile) if profile.tmux == TmuxMode::Control => {
                if let Some(new) = self.attach_tmux(profile, None, window, cx) {
                    if let Some(orphan) = orphan {
                        self.replacing.insert(new.entity_id(), orphan);
                    }
                    return;
                }
            }
            Some(_) => self.launch(LaunchRequest::profile(ix), window, cx),
            None => {}
        }
        self.close_orphans(window, cx);
    }

    fn zone_tab_subscriptions(
        view: &Entity<ZoneTreeView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Subscription> {
        vec![cx.subscribe_in(view, window, |this, view, event, window, cx| {
            this.on_zone_tree_event(view, event, window, cx)
        })]
    }

    fn on_zone_tree_event(
        &mut self,
        view: &Entity<ZoneTreeView>,
        event: &TerminalEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.tabs.iter().position(|t| {
            matches!(&t.content, TabContent::Zones(v) if v == view)
        }) else {
            return;
        };
        match event {
            TerminalEvent::Exited => self.remove_tab(ix, window, cx),
            TerminalEvent::Bell | TerminalEvent::Notify(_) | TerminalEvent::Status(_) => {
                self.on_attention(ix, event, window, cx)
            }
            TerminalEvent::TitleChanged => {
                if self.tabs[ix].refresh_title(&mut None, cx) {
                    cx.notify();
                }
            }
        }
    }

    /// A bell, a notification request or a program status change from tab
    /// `ix`, local or tmux: marks the tab (the bell dot, the status dot),
    /// and raises a desktop notification if the tab isn't in front of the
    /// user (or the settings say always).
    fn on_attention(&mut self, ix: usize, event: &TerminalEvent, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        let visible = ix == self.active && window.is_window_active();
        let (source, title, body) = match event {
            TerminalEvent::Bell => {
                if !visible {
                    self.tabs[ix].bell = true;
                    cx.notify();
                }
                (Source::Bell, "", "")
            }
            TerminalEvent::Notify(notice) => (notice.source, notice.title.as_str(), notice.body.as_str()),
            TerminalEvent::Status(notice) => {
                self.refresh_status(ix, cx);
                let Some(notice) = notice else { return };
                (notice.source, notice.title.as_str(), notice.body.as_str())
            }
            _ => return,
        };
        if !notify::policy::wanted(&Self::settings(cx).notifications, source, visible) {
            return;
        }
        let Some(handle) = window.window_handle().downcast::<Workspace>() else { return };
        let tab = &self.tabs[ix];
        let (summary, body) = notify::policy::layout(source, title, body, &tab.title);
        let request = notify::Request { source, summary, body, window: handle, tab: tab.id };
        Notifier::send(request, cx);
    }

    /// Recomputes tab `ix`'s status dot from its terminals' records. Unlike
    /// the bell dot it isn't cleared by activating the tab: it goes when the
    /// records do (a new report, a new prompt, the user typing).
    fn refresh_status(&mut self, ix: usize, cx: &mut Context<Self>) {
        let status = self.tabs[ix].status_summary(cx);
        if self.tabs[ix].status != status {
            self.tabs[ix].status = status;
            cx.notify();
        }
    }

    /// Activates the tab with this id, if it is still open (a notification
    /// for it was clicked).
    pub fn show_tab(&mut self, tab_id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.tabs.iter().position(|t| t.id == tab_id) {
            self.activate(ix, window, cx);
        }
    }

    /// Makes tab `index` the active one and, unless an overlay is open,
    /// gives its terminal focus.
    fn activate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if !self.select(index, cx) {
            return;
        }
        if !self.has_overlay() {
            self.focus_active_tab(window, cx);
        }
    }

    /// Shows tab `index` (clamped) without moving focus: its bell clears and
    /// a tmux window becomes its session's current one. False with no tabs.
    fn select(&mut self, index: usize, cx: &mut Context<Self>) -> bool {
        if self.tabs.is_empty() {
            return false;
        }
        self.active = index.min(self.tabs.len() - 1);
        let tab = &mut self.tabs[self.active];
        tab.bell = false;
        if let TabContent::Tmux { session, view } = &tab.content {
            let window_id = view.read(cx).window_id;
            session.update(cx, |s, _| s.select_window(window_id));
        }
        cx.notify();
        true
    }

    /// Focuses the active tab's terminal (the tab's own focus handle).
    fn focus_active_tab(&self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get(self.active) {
            window.focus(&tab.focus_handle(cx));
        }
    }

    /// The strip asked to activate a tab. A pointer press activates it and
    /// puts focus in its terminal, as a key or a click elsewhere would; an
    /// arrow, home or end while the strip has focus only shows the tab, so
    /// focus stays on the strip, resting on the new active tab.
    fn activate_from_strip(&mut self, request: &TabActivate, window: &mut Window, cx: &mut Context<Self>) {
        match request.by {
            Modality::Pointer => self.activate(request.index, window, cx),
            Modality::Keyboard => {
                self.select(request.index, cx);
            }
        }
    }

    /// `FocusTabStrip`: focus moves to the strip, on the active tab. Nothing
    /// happens while an overlay holds focus, or with no tabs.
    fn focus_tab_strip(&mut self, _: &FocusTabStrip, window: &mut Window, cx: &mut Context<Self>) {
        if self.has_overlay() || self.tabs.is_empty() {
            return;
        }
        window.focus(&self.strip_focus);
        cx.notify();
    }

    /// A key while the strip has focus, after the tab list took its own
    /// (arrows, home, end): enter and escape return focus to the active
    /// tab's terminal. Anything else goes on to the window's bindings.
    fn strip_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if !self.strip_focus.is_focused(window) || !returns_from_strip(&event.keystroke.key, event.keystroke.modifiers) {
            return;
        }
        cx.stop_propagation();
        self.focus_active_tab(window, cx);
        cx.notify();
    }

    /// Removes tab `ix`, ending its terminals' programs.
    fn remove_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(ix);
        if let TabContent::Zones(view) = &tab.content {
            view.update(cx, |v, cx| v.shutdown_all(cx));
        }
        if self.tabs.is_empty() {
            // Nothing to restore for a window whose tabs all closed.
            let id = self.window_id;
            cx.update_global(|store: &mut SessionStore, _| store.forget(id));
            window.remove_window();
            return;
        }
        if self.active > ix || self.active >= self.tabs.len() {
            self.active = self.active.saturating_sub(1);
        }
        self.activate(self.active, window, cx);
    }

    // ---- actions ----------------------------------------------------------------

    fn open_message_tab(&mut self, profile: Profile, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let config = Self::settings(cx).clone();
        let theme = config.profile_theme(&profile);
        let terminal = cx.new(|cx| Terminal::message(text, theme.clone(), &config, cx));
        let p_clone = profile.clone();
        let view = cx.new(|cx| ZoneTreeView::new(p_clone, theme, profile.font_size, terminal, window, cx));
        let tab = Tab::new(profile, TabContent::Zones(view), TabOrigin::Message, Vec::new(), cx);
        self.insert_tab(tab, None, window, cx);
    }

    fn new_tab(&mut self, _: &NewTab, window: &mut Window, cx: &mut Context<Self>) {
        // In a tmux tab, a new tab is a new tmux window in the same session.
        if let Some(tab) = self.tabs.get(self.active)
            && let TabContent::Tmux { session, .. } = &tab.content
        {
            session.update(cx, |s, _| s.new_window());
            return;
        }
        let profile = Self::settings(cx).default_profile_index();
        self.open_profile(profile, window, cx);
    }

    fn new_tab_with_profile(&mut self, action: &NewTabWithProfile, window: &mut Window, cx: &mut Context<Self>) {
        self.open_profile(action.0, window, cx);
    }

    fn close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        self.close_tab_at(self.active, window, cx);
    }

    fn close_tab_at(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(ix) else { return };
        if let TabContent::Tmux { session, view } = &tab.content
            && !session.read(cx).is_detached()
        {
            // Closing a tmux tab closes that tmux window; tmux tells us when
            // it is gone and the tab is removed then.
            let window_id = view.read(cx).window_id;
            session.update(cx, |s, _| s.kill_window(window_id));
            return;
        }
        self.remove_tab(ix, window, cx);
    }

    fn next_tab(&mut self, _: &NextTab, window: &mut Window, cx: &mut Context<Self>) {
        if !self.tabs.is_empty() {
            self.activate((self.active + 1) % self.tabs.len(), window, cx);
        }
    }

    fn prev_tab(&mut self, _: &PrevTab, window: &mut Window, cx: &mut Context<Self>) {
        if !self.tabs.is_empty() {
            self.activate((self.active + self.tabs.len() - 1) % self.tabs.len(), window, cx);
        }
    }

    fn activate_tab(&mut self, action: &ActivateTab, window: &mut Window, cx: &mut Context<Self>) {
        let ix = if action.0 == usize::MAX { self.tabs.len().saturating_sub(1) } else { action.0 };
        if ix < self.tabs.len() {
            self.activate(ix, window, cx);
        }
    }

    fn move_tab(&mut self, delta: isize, cx: &mut Context<Self>) {
        let len = self.tabs.len() as isize;
        if len < 2 {
            return;
        }
        let to = (self.active as isize + delta).rem_euclid(len) as usize;
        let tab = self.tabs.remove(self.active);
        self.tabs.insert(to, tab);
        self.active = to;
        cx.notify();
    }

    fn move_tab_left(&mut self, _: &MoveTabLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.move_tab(-1, cx);
    }

    fn move_tab_right(&mut self, _: &MoveTabRight, _: &mut Window, cx: &mut Context<Self>) {
        self.move_tab(1, cx);
    }

    fn rename_tab(&mut self, _: &RenameTab, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(self.active) else { return };
        let tab_id = tab.id;
        let initial = tab.title.to_string();
        let this = cx.entity().downgrade();
        let submit: SubmitFn = Rc::new(move |text, cx| {
            this.update(cx, |this, cx| {
                if let Some(tab) = this.tabs.iter_mut().find(|t| t.id == tab_id)
                    && tab.set_manual_title(&text, &mut None, cx)
                {
                    cx.notify();
                }
            })
            .ok();
            ready(Ok(()))
        });
        let label = "Rename tab".into();
        self.open_prompt(PromptRequest { label, initial, submit }, window, cx);
    }

    /// A tab dragged onto another: the tab at `from` takes `to`'s place
    /// (after it when it came from the left, before it from the right), and
    /// the active tab stays active wherever it ends up.
    fn move_tab_to(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        let Some(active_id) = self.tabs.get(self.active).map(|t| t.id) else { return };
        if !reorder(&mut self.tabs, from, to) {
            return;
        }
        self.active = self.tabs.iter().position(|t| t.id == active_id).unwrap_or(0);
        cx.notify();
    }

    /// Closing the window quits Brindle: its terminals' programs keep running.
    fn close_window(&mut self, _: &CloseWindow, window: &mut Window, _: &mut Context<Self>) {
        window.remove_window();
    }

    fn open_profile_picker(&mut self, _: &OpenProfilePicker, window: &mut Window, cx: &mut Context<Self>) {
        if self.close_palette_toggling(|k| matches!(k, PaletteKind::NewTab), window, cx) {
            return;
        }
        let tab_focus = self.tabs.get(self.active).map(|t| t.focus_handle(cx));
        let rows = Self::settings(cx)
            .profiles
            .iter()
            .enumerate()
            .map(|(ix, profile)| {
                // Picking a session the window shows activates its tab; say so.
                let shown = SessionKey::for_profile(profile).and_then(|key| self.find_session(&key, cx));
                let state = if shown.is_some() { " · open in this window" } else { "" };
                PaletteRow {
                    label: profile.name.clone().into(),
                    detail: Some(format!("{}{state}", profile.description()).into()),
                    shortcut: tab_focus.as_ref().and_then(|f| shortcut(&NewTabWithProfile(ix), f, window)),
                }
            })
            .collect();
        let palette = cx.new(|cx| Palette::list(rows, "Open profile…", cx));
        self.show_palette(palette, PaletteKind::NewTab, window, cx);
    }

    fn open_command_palette(&mut self, _: &OpenCommandPalette, window: &mut Window, cx: &mut Context<Self>) {
        if self.close_palette_toggling(|k| matches!(k, PaletteKind::Commands { .. }), window, cx) {
            return;
        }
        let Some(tab_focus) = self.tabs.get(self.active).map(|t| t.focus_handle(cx)) else { return };
        // Snapshot what the tab can do while it still has focus.
        window.focus(&tab_focus);
        let types: std::collections::HashSet<std::any::TypeId> =
            window.available_actions(cx).iter().map(|a| a.as_any().type_id()).collect();
        let commands =
            palette_commands(|a| types.contains(&a.as_any().type_id()) || window.is_action_available(a, cx));
        let rows = commands
            .iter()
            .map(|(label, action)| PaletteRow {
                label: (*label).into(),
                detail: None,
                shortcut: shortcut(&**action, &tab_focus, window),
            })
            .collect();
        let actions = commands.into_iter().map(|(_, a)| a).collect();
        let palette = cx.new(|cx| Palette::list(rows, "Run a command…", cx));
        self.show_palette(palette, PaletteKind::Commands { actions, tab_focus }, window, cx);
    }

    /// Asks for a line of text. An open palette becomes the prompt in
    /// place (a list followed by a prompt keeps one overlay layer and the
    /// focus it gives back, and the slot's outside-press rule: the prompt
    /// that follows a question ignores outside presses as the question
    /// did); otherwise one opens, replacing any overlay.
    fn open_prompt(&mut self, request: PromptRequest, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(palette) = self.palette.view().cloned() {
            palette.update(cx, |palette, cx| palette.show_prompt(request, window, cx));
            self.palette_kind = Some(PaletteKind::Prompt);
            cx.notify();
            return;
        }
        let palette = cx.new(|cx| Palette::prompt(request, cx));
        self.show_palette(palette, PaletteKind::Prompt, window, cx);
    }

    /// Closes any open palette. Returns true if it was of the kind `is`
    /// matches, so that opening a palette again toggles it.
    fn close_palette_toggling(
        &mut self,
        is: impl Fn(&PaletteKind) -> bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.has_overlay() {
            return false;
        }
        let same = self.palette_kind.as_ref().is_some_and(is);
        self.dismiss_overlay(window, cx);
        same
    }

    /// Opens `palette` in the palette slot, replacing any overlay. The one
    /// open goes first and gives focus back (to the tab), so that is the
    /// focus the palette records and gives back when it closes.
    fn show_palette(
        &mut self,
        palette: Entity<Palette>,
        kind: PaletteKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_overlay(window, cx);
        let subscription = cx.subscribe_in(&palette, window, |this, _, event, window, cx| match event {
            PaletteEvent::Confirmed(ix) => this.palette_confirmed(*ix, window, cx),
            PaletteEvent::Kill(ix) => this.palette_kill(*ix, window, cx),
            PaletteEvent::Dismissed => this.palette_dismissed(window, cx),
        });
        self.palette.set_ignore_outside_press(ignores_outside_press(PaletteTag::of(&kind)));
        self.palette_kind = Some(kind);
        self._palette_subscription = Some(subscription);
        self.palette.open(palette, |this| &mut this.palette, window, cx);
        cx.notify();
    }

    /// Escape (or a click outside) on a palette. After a session ended,
    /// that closes the window. After asking about another Brindle's session
    /// (or a prompt that followed), tabs nothing replaces close, and so does
    /// a window left without tabs.
    fn palette_dismissed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (ended, orphans) = match &self.palette_kind {
            Some(PaletteKind::SessionEnded { session }) => (Some(session.clone()), false),
            Some(PaletteKind::SessionOpenElsewhere { .. } | PaletteKind::Prompt) => (None, true),
            _ => (None, false),
        };
        self.dismiss_overlay(window, cx);
        if let Some(session) = ended {
            self.remove_session_tabs(&session, window, cx);
        }
        if orphans {
            self.close_orphans(window, cx);
        }
    }

    fn palette_confirmed(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(kind) = self.palette_kind.take() else { return };
        match kind {
            PaletteKind::NewTab => {
                self.dismiss_overlay(window, cx);
                self.open_profile(ix, window, cx);
            }
            PaletteKind::SessionOpenElsewhere { choices, profile } => match choices.get(ix) {
                // The question turns into the prompt for the session's
                // name, in place.
                Some(OpenChoice::NewSession) => match Self::settings(cx).profiles.get(profile).cloned() {
                    Some(profile) => self.ask_new_session(profile, window, cx),
                    None => self.dismiss_overlay(window, cx),
                },
                Some(&OpenChoice::Profile(other)) => {
                    self.dismiss_overlay(window, cx);
                    self.launch_replacing_orphans(other, window, cx);
                }
                None => self.dismiss_overlay(window, cx),
            },
            PaletteKind::Commands { actions, tab_focus } => {
                // Run it from the tab, exactly as its keybinding would: the
                // palette closes first (giving focus back to the tab), since
                // the dispatch is deferred and follows focus. A prompt the
                // command asks for opens a palette of its own.
                self.close_overlay(window, cx);
                window.focus(&tab_focus);
                cx.notify();
                if let Some(action) = actions.into_iter().nth(ix) {
                    window.dispatch_action(action, cx);
                }
            }
            // A prompt has no rows to confirm.
            PaletteKind::Prompt => self.palette_kind = Some(PaletteKind::Prompt),
            PaletteKind::Sessions { sessions } => {
                self.dismiss_overlay(window, cx);
                if let Some(info) = sessions.into_iter().nth(ix) {
                    self.attach_detached_here(info, window, cx);
                }
            }
            PaletteKind::SessionEnded { session } => {
                self.dismiss_overlay(window, cx);
                let profile = ix.checked_sub(1).and_then(|p| Self::settings(cx).profiles.get(p).cloned());
                match profile {
                    // A control-mode session's tabs arrive later; keep the
                    // window open with the ended tabs until they do.
                    Some(profile) if profile.tmux == TmuxMode::Control => {
                        if let Some(new) = self.attach_tmux(profile, None, window, cx) {
                            self.replacing.insert(new.entity_id(), session);
                            return;
                        }
                    }
                    Some(_) => self.launch(LaunchRequest::profile(ix - 1), window, cx),
                    None => {}
                }
                self.remove_session_tabs(&session, window, cx);
            }
        }
    }

    /// "Attach Session…": lists the detached sessions off the UI thread,
    /// then shows them.
    fn attach_session(&mut self, _: &AttachSession, window: &mut Window, cx: &mut Context<Self>) {
        if self.close_palette_toggling(|k| matches!(k, PaletteKind::Sessions { .. }), window, cx) {
            return;
        }
        self.refresh_sessions_palette(None, window, cx);
    }

    /// Kills `kill` first, if given, then lists the detached sessions and
    /// shows them in the sessions palette. After a kill, only a sessions
    /// palette still open is refreshed: one closed meanwhile stays closed.
    fn refresh_sessions_palette(&mut self, kill: Option<u32>, window: &mut Window, cx: &mut Context<Self>) {
        let listing = cx.background_executor().spawn(async move { detached_now(kill) });
        cx.spawn_in(window, async move |this, cx| {
            let listed = listing.await;
            this.update_in(cx, |this, window, cx| {
                let open = matches!(&this.palette_kind, Some(PaletteKind::Sessions { .. }));
                if kill.is_some() && !open {
                    return;
                }
                let sessions = listed.unwrap_or_else(|why| {
                    log::warn!("listing sessions: {why:#}");
                    Vec::new()
                });
                this.show_sessions_palette(sessions, window, cx);
            })
            .ok();
        })
        .detach();
    }

    fn show_sessions_palette(&mut self, mut sessions: Vec<SessionInfo>, window: &mut Window, cx: &mut Context<Self>) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let rows = session_rows(&mut sessions, now);
        let empty = sessions_empty_message(sessions.len());
        // An open sessions palette takes the new list in place, keeping its
        // filter text.
        if let Some(PaletteKind::Sessions { sessions: listed }) = &mut self.palette_kind
            && let Some(palette) = self.palette.view().cloned()
        {
            *listed = sessions;
            palette.update(cx, |palette, cx| palette.set_rows(rows, empty, cx));
            cx.notify();
            return;
        }
        let palette = cx.new(|cx| Palette::list_with_empty(rows, "Attach a detached session", empty, cx));
        self.show_palette(palette, PaletteKind::Sessions { sessions }, window, cx);
    }

    /// `picker_kill` in the sessions palette: ends that session, and shows
    /// the list again.
    fn palette_kill(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(PaletteKind::Sessions { sessions }) = &self.palette_kind else {
            return;
        };
        let Some(id) = sessions.get(ix).map(|s| s.session) else { return };
        // Its row goes at once, so enter can't attach it meanwhile.
        let rest = sessions.iter().filter(|s| s.session != id).cloned().collect();
        self.show_sessions_palette(rest, window, cx);
        self.refresh_sessions_palette(Some(id), window, cx);
    }

    /// Attaches a detached session into a new tab after the active one.
    fn attach_detached_here(&mut self, info: SessionInfo, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(target_os = "linux")]
        {
            let config = Self::settings(cx).clone();
            let index = info
                .profile
                .as_deref()
                .and_then(|name| config.profile_index(name))
                .unwrap_or_else(|| config.default_profile_index());
            let profile = config.profiles[index].clone();
            let terminal = Self::attached_terminal(&info, &profile, &config, cx);
            self.insert_terminal_tab(profile, terminal, true, window, cx);
        }
        #[cfg(not(target_os = "linux"))]
        let _ = (info, window, cx);
    }

    fn open_settings(&mut self, _: &OpenSettings, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings.is_open() {
            self.dismiss_overlay(window, cx);
            return;
        }
        let config = Self::settings(cx);
        let model = settings_dialog::Model::new(
            Config::path(),
            config.theme.clone(),
            config.themes.clone(),
            config.profiles.clone(),
            config.default_profile.clone(),
            config.tmux_session_end,
        );
        let theme = config.theme(None);
        let dialog = cx.new(|cx| SettingsDialog::new(model, theme, window, cx));
        let subscription = cx.subscribe_in(&dialog, window, |this, _, event, _, cx| match event {
            SettingsEvent::Preview(theme) => this.set_theme_preview(Some(theme.clone()), cx),
            // Deferred by GPUI, so the reload sees this window idle.
            SettingsEvent::ProfilePreview(preview) => this.set_profile_preview(preview.clone(), cx),
            // Deferred, so the reload sees this window idle (as ReloadConfig).
            SettingsEvent::ConfigWritten(renames) => {
                let renames = renames.clone();
                cx.defer(move |cx| crate::reload_config_renaming(cx, &renames));
            }
        });
        // The slot closes itself on the dialog's `DismissEvent`; this ends
        // the previews, as closing it any other way does.
        let dismissed = cx.subscribe_in(&dialog, window, |this, _, _: &gpui::DismissEvent, _, cx| this.end_previews(cx));
        // A palette goes first, with focus back on the tab, so that is the
        // focus the dialog records and puts back when it closes (the
        // palette's input is gone by then).
        if self.palette.is_open() {
            self.dismiss_overlay(window, cx);
        }
        self._settings_subscriptions = vec![subscription, dismissed];
        self.settings.open(dialog, |this| &mut this.settings, window, cx);
        cx.notify();
    }

    fn has_overlay(&self) -> bool {
        self.palette.is_open() || self.settings.is_open()
    }

    /// Closes whatever overlay is open and gives focus back to the tab.
    fn dismiss_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_overlay(window, cx);
        if let Some(tab) = self.tabs.get(self.active) {
            window.focus(&tab.focus_handle(cx));
        }
        cx.notify();
    }

    /// Drops the open overlay. Closing the settings dialog this way cancels
    /// it, ending any preview.
    fn close_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.palette.close(window, cx);
        self.palette_kind = None;
        self._palette_subscription = None;
        self.settings.close(window, cx);
        self._settings_subscriptions.clear();
        self.end_previews(cx);
    }

    /// Stops the settings dialog's theme and profile previews.
    fn end_previews(&mut self, cx: &mut Context<Self>) {
        if self.theme_preview.is_some() {
            self.set_theme_preview(None, cx);
        }
        if self.profile_preview.is_some() {
            self.set_profile_preview(None, cx);
        }
    }

    fn change_font_size(&mut self, delta: Option<f32>, cx: &mut Context<Self>) {
        cx.update_global::<Settings, _>(|settings, _| match delta {
            Some(d) => settings.zoom = (settings.zoom + d).clamp(-10.0, 60.0),
            None => settings.zoom = 0.0,
        });
        cx.refresh_windows();
    }

    fn increase_font_size(&mut self, _: &IncreaseFontSize, _: &mut Window, cx: &mut Context<Self>) {
        self.change_font_size(Some(1.0), cx);
    }

    fn decrease_font_size(&mut self, _: &DecreaseFontSize, _: &mut Window, cx: &mut Context<Self>) {
        self.change_font_size(Some(-1.0), cx);
    }

    fn reset_font_size(&mut self, _: &ResetFontSize, _: &mut Window, cx: &mut Context<Self>) {
        self.change_font_size(None, cx);
    }

    fn open_config(&mut self, _: &OpenConfig, window: &mut Window, cx: &mut Context<Self>) {
        let command = editor_command(&Config::path().to_string_lossy());
        let profile = Self::settings(cx).default_profile_index();
        self.launch(LaunchRequest { profile, command: Some(command), cwd: None }, window, cx);
    }

    /// The theme a tab with `profile` shows: the preview for tabs that
    /// follow the global theme, else the profile's theme.
    fn tab_theme(&self, profile: &Profile, config: &Config) -> Theme {
        match (&self.theme_preview, &profile.theme) {
            (Some(preview), None) => preview.clone(),
            _ => config.profile_theme(profile),
        }
    }

    /// A tab's profile, or the settings dialog's preview of it.
    fn effective_profile<'a>(&'a self, profile: &'a Profile) -> &'a Profile {
        match &self.profile_preview {
            Some((name, preview)) if name.eq_ignore_ascii_case(&profile.name) => preview,
            _ => profile,
        }
    }

    /// Re-themes every tab (and sets its font size) from the config and the
    /// previews.
    fn apply_tab_themes(&mut self, cx: &mut Context<Self>) {
        let config = Self::settings(cx).clone();
        for tab in &self.tabs {
            let profile = self.effective_profile(&tab.profile);
            set_tab_theme(tab, self.tab_theme(profile, &config), profile.font_size, cx);
        }
    }

    /// Shows the edited profile's theme and font size on its tabs, or, with
    /// `None`, puts their configured ones back. As with the theme preview,
    /// tmux isn't told.
    pub fn set_profile_preview(&mut self, preview: Option<(String, Profile)>, cx: &mut Context<Self>) {
        self.profile_preview = preview;
        self.apply_tab_themes(cx);
        cx.notify();
    }

    /// Shows `preview` on the tabs that follow the global theme, or, with
    /// `None`, puts their configured theme back. tmux isn't told: color
    /// reports are only sent on a real config change.
    pub fn set_theme_preview(&mut self, preview: Option<Theme>, cx: &mut Context<Self>) {
        match &preview {
            Some(theme) => Settings::set_ui_theme(theme, cx),
            None => Settings::apply_ui_theme(cx),
        }
        self.theme_preview = preview;
        self.apply_tab_themes(cx);
        cx.notify();
    }

    /// Applies a reloaded config to running terminals. `renames` (old, new)
    /// are profiles renamed in the settings dialog: their tabs follow the
    /// new name.
    pub fn apply_config(&mut self, renames: &[(String, String)], cx: &mut Context<Self>) {
        let config = Self::settings(cx).clone();
        for tab in &mut self.tabs {
            if let Some(new) = renamed(&tab.profile.name, renames) {
                tab.profile.name = new.to_string();
            }
        }
        for (session, _) in &self.tmux_sessions {
            session.update(cx, |s, _| {
                if let Some(new) = renamed(&s.profile.name, renames) {
                    s.profile.name = new.to_string();
                }
            });
        }
        for tab in &mut self.tabs {
            if let Some(p) = config.profile(&tab.profile.name) {
                let remote = tab.profile.remote.clone();
                tab.profile = p.clone();
                tab.profile.remote = remote;
            }
        }
        let mut snapshot = None;
        for tab in &mut self.tabs {
            tab.refresh_title(&mut snapshot, cx);
        }
        self.apply_tab_themes(cx);
        for (session, _) in &self.tmux_sessions {
            session.update(cx, |s, cx| {
                let profile = config.profile(&s.profile.name).cloned().unwrap_or_else(|| s.profile.clone());
                s.set_config(profile, &config, cx);
            });
        }
        cx.notify();
    }

    /// The text input that has keyboard focus, if any: the overlay's, or
    /// the find bar's in the active terminal.
    fn focused_input(&self, window: &Window, cx: &App) -> Option<Entity<TextInput>> {
        if let Some(palette) = self.palette.view() {
            let input = palette.read(cx).input(cx);
            return input.read(cx).is_focused(window).then_some(input);
        }
        if self.settings.is_open() {
            return self.settings.view()?.read(cx).focused_input(window, cx);
        }
        let input = self.tabs.get(self.active)?.active_view(cx)?.read(cx).find_input()?;
        input.read(cx).is_focused(window).then_some(input)
    }

    /// Debug hook for `--send`: types into the focused text input, or else
    /// the active tab.
    pub fn send_to_active(&mut self, bytes: &[u8], window: &mut Window, cx: &mut Context<Self>) {
        if let Some(input) = self.focused_input(window, cx) {
            let text: String = String::from_utf8_lossy(bytes).chars().filter(|c| !c.is_control()).collect();
            input.update(cx, |input, cx| input.insert(&text, cx));
            return;
        }
        if let Some(terminal) = self.tabs.get(self.active).and_then(|t| t.active_terminal(cx)) {
            let bytes = bytes.to_vec();
            terminal.update(cx, |t, cx| t.input(bytes, cx));
        }
    }

    /// Debug hook for `--hover` / `--click`: mouse events for a cell of the
    /// active terminal, through its own handlers.
    pub fn debug_pointer(&mut self, step: &PointerStep, window: &mut Window, cx: &mut Context<Self>) {
        match self.tabs.get(self.active).and_then(|t| t.active_view(cx)) {
            Some(view) => view.update(cx, |view, cx| view.debug_pointer(step, window, cx)),
            None => log::error!("--hover/--click: no active terminal"),
        }
    }

    /// Debug hook for `--find`: searches the active terminal for `query`.
    pub fn find_in_active(&mut self, query: &str, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(view) = self.tabs.get(self.active).and_then(|t| t.active_view(cx)) {
            view.update(cx, |view, cx| view.find_text(query, window, cx));
        }
    }

    /// The active terminal's visible screen text, for `--wait-for`.
    pub fn active_screen_text(&self, cx: &App) -> Option<String> {
        Some(self.tabs.get(self.active)?.active_terminal(cx)?.read(cx).screen_text())
    }

    /// Debug hook for `--dump-screen-after`.
    pub fn dump_active_screen(&self, window: &Window, cx: &App) -> String {
        let scale = window.scale_factor();
        let mut out = String::new();
        for (ix, tab) in self.tabs.iter().enumerate() {
            out.push_str(&format!(
                "--- tab {} {:?}{}\n",
                ix + 1,
                tab.title,
                if ix == self.active { " (active)" } else { "" }
            ));
        }
        if let Some(terminal) = self.tabs.get(self.active).and_then(|t| t.active_terminal(cx)) {
            let terminal = terminal.read(cx);
            let size = terminal.size();
            out.push_str(&format!("--- screen {}x{} mode {:?}\n", size.cols, size.rows, terminal.mode()));
            if let Some(grid) = self.tabs.get(self.active).and_then(|t| t.active_grid(cx)) {
                out.push_str(&grid.describe(scale));
            }
            let view = self.tabs.get(self.active).and_then(|t| t.active_view(cx));
            if let Some(link) = view.and_then(|v| v.read(cx).hovered_link.clone()) {
                out.push_str(&format!("--- link {}\n", link.uri));
            }
            if let Some(text) = terminal.selection_text() {
                out.push_str(&format!("--- selection {text:?}\n"));
            }
            out.push_str(&terminal.status().describe());
            if let Some(TabContent::Zones(zones)) = self.tabs.get(self.active).map(|t| &t.content) {
                out.push_str(&zones.read(cx).describe_zones());
            }
            out.push_str(&terminal.screen_text());
        }
        if let Some(find) = self.tabs.get(self.active).and_then(|t| t.active_view(cx)).and_then(|v| v.read(cx).describe_find(cx)) {
            out.push_str(&find);
        }
        if let Some(palette) = self.palette.view() {
            out.push_str(&palette.read(cx).describe(cx));
        }
        if let Some(dialog) = self.settings.view() {
            out.push_str(&dialog.read(cx).describe(window, cx));
        }
        if let Some(input) = self.focused_input(window, cx) {
            out.push_str(&format!("--- input {}\n", input.read(cx).describe()));
        }
        out.push_str(&format!("--- focus {}\n", self.describe_focus(window, cx)));
        out
    }

    /// Where keyboard focus is, for `--dump-screen-after`: `terminal` (the
    /// active tab's active terminal), `palette` (inside an open palette),
    /// `settings stop=NAME` (inside the settings dialog, on the control
    /// `NAME`), `tabs` (the tab strip, on the active tab), `none`, or
    /// `other`; then the window's `modality=keyboard|pointer`, which
    /// decides whether the focused control shows its ring.
    fn describe_focus(&self, window: &Window, cx: &App) -> String {
        let terminal = self.tabs.get(self.active).and_then(|t| t.active_view(cx));
        let place = if terminal.is_some_and(|v| v.read(cx).focus_handle.is_focused(window)) {
            "terminal".to_string()
        } else if self.palette.view().is_some_and(|p| p.read(cx).focus_handle(cx).contains_focused(window, cx)) {
            "palette".to_string()
        } else if let Some(dialog) =
            self.settings.view().filter(|d| d.read(cx).focus_handle(cx).contains_focused(window, cx))
        {
            format!("settings stop={:?}", dialog.read(cx).focused_label(window, cx))
        } else if self.strip_focus.is_focused(window) {
            "tabs".to_string()
        } else if window.focused(cx).is_none() {
            "none".to_string()
        } else {
            "other".to_string()
        };
        let modality = match brindle_ui::modality(window, cx) {
            brindle_ui::Modality::Keyboard => "keyboard",
            brindle_ui::Modality::Pointer => "pointer",
        };
        format!("{place} modality={modality}")
    }

    // ---- rendering --------------------------------------------------------------

    /// The tab strip, which is also the window's title bar: a `TabList` in
    /// the `Attached` variant, the new-tab and profile buttons, the caption
    /// (empty space) and, with client decorations, the window controls.
    ///
    /// It is drawn in the window's chrome tokens (`cx.ui_theme()`, from the
    /// global theme), whatever the active tab's profile theme; only the
    /// active tab's face takes its terminal's background and foreground,
    /// so it joins the terminal below. The strip is one keyboard stop
    /// (`strip_focus`), reached through `FocusTabStrip`: the list takes the
    /// arrows, home and end, and the wrapper around it enter and escape. A
    /// press on a tab activates it and leaves focus in its terminal.
    fn render_tab_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = *cx.ui_theme();
        let decorations = window.window_decorations();
        // Windows always shows Brindle's own caption (see `window_control_area`
        // below); GPUI reports server decorations there.
        let csd = matches!(decorations, Decorations::Client { .. }) || cfg!(windows);

        // Status dots take the global theme's ANSI colours (the previewed
        // theme's while the settings dialog shows one), not the tab's;
        // working is the muted foreground.
        let global = match &self.theme_preview {
            Some(theme) => theme.clone(),
            None => Self::settings(cx).theme(None),
        };
        let status_color = |state: State| match state {
            State::Blocked => global.ansi[3].hsla(),
            State::Error => global.ansi[1].hsla(),
            State::Done => global.ansi[2].hsla(),
            State::Working | State::Idle => tokens.muted_foreground,
        };
        let dot = |color: Hsla| div().flex_none().size(px(6.0)).rounded_full().bg(color);

        // A tooltip naming a control and the key bound to its action in this
        // window, looked up when the tooltip is shown (a rebound key shows
        // correctly, and a render pays only for the closure).
        let focus = self.focus_handle.clone();
        let tip = move |label: &'static str, action: Box<dyn Action>| {
            let (focus, action): (FocusHandle, Rc<dyn Action>) = (focus.clone(), Rc::from(action));
            move |window: &mut Window, _: &mut App| Tooltip::new(label).shortcut_opt(shortcut(&*action, &focus, window))
        };
        let close_tip = tip("Close tab", Box::new(CloseTab));

        let triggers: Vec<TabTrigger> = self
            .tabs
            .iter()
            .enumerate()
            .map(|(ix, tab)| {
                let mut trigger =
                    TabTrigger::new(("tab", tab.id as usize), tab.title.clone()).closable(true).close_tooltip(close_tip.clone());
                if ix == self.active {
                    let (surface, text) = tab.terminal_colors(cx);
                    trigger = trigger.surface(surface).text_color(text);
                }
                if tab.is_tmux() {
                    trigger = trigger.leading(Icon::AppWindow.size(rems(0.875)).color(tokens.primary));
                }
                if let Some(state) = tab.status {
                    trigger = trigger.adornment(dot(status_color(state)));
                }
                if tab.bell {
                    trigger = trigger.adornment(dot(tokens.primary));
                }
                trigger
            })
            .collect();
        let list = TabList::new("tabs")
            .variant(TabVariant::Attached)
            .active(self.active)
            .children(triggers)
            .focus(&self.strip_focus)
            .trigger_width(TabWidth::new(rems(5.), rems(15.)))
            .on_activate(cx.listener(|this, request: &TabActivate, window, cx| this.activate_from_strip(request, window, cx)))
            .on_close(cx.listener(|this, ix: &usize, window, cx| this.close_tab_at(*ix, window, cx)))
            .on_move(cx.listener(|this, request: &TabMove, _, cx| this.move_tab_to(request.from, request.to, cx)))
            .flex_shrink()
            .min_w_0()
            .overflow_hidden();

        // A press on a strip button stops here: it doesn't start a window
        // move (the strip's own press handler, below) and moves focus
        // nowhere (a `Button` without a handle lets a press bubble to the
        // workspace, which tracks focus).
        let inert = |id: &'static str, control: AnyElement| {
            div()
                .id(id)
                .flex_none()
                .on_mouse_down(MouseButton::Left, |_, window, cx| {
                    window.prevent_default();
                    cx.stop_propagation();
                })
                .child(control)
        };
        let button = |id: &'static str, icon: Icon| {
            Button::icon_only(id, icon)
                .variant(ButtonVariant::Ghost)
                .size(ButtonSize::IconXs)
                .text_color(tokens.muted_foreground)
        };
        let new_tab = button("new-tab", Icon::Plus)
            .tooltip_with(tip("New tab", Box::new(NewTab)))
            .on_click(cx.listener(|this, _, window, cx| this.new_tab(&NewTab, window, cx)));
        let profiles = button("profiles", Icon::ChevronDown)
            .tooltip_with(tip("New tab from profile…", Box::new(OpenProfilePicker)))
            .on_click(cx.listener(|this, _, window, cx| this.open_profile_picker(&OpenProfilePicker, window, cx)));

        // The caption buttons stay hand-built faces, in the ghost icon
        // button's size and hover colours, drawing icons rather than text
        // glyphs (which fall back to whatever font the platform picks). On
        // Windows the marked areas are native caption buttons: Windows acts
        // on them (and shows Snap Layouts on maximize), but only on a press
        // Brindle leaves unhandled, so they get no handlers there.
        let caption = button_style(ButtonVariant::Ghost, ButtonSize::IconXs, &tokens);
        let face = move |id: &'static str, icon: Icon| {
            div()
                .id(id)
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .h(caption.height)
                .w(caption.width.unwrap_or(caption.height))
                .rounded(caption.radius)
                .text_color(tokens.muted_foreground)
                .cursor(CursorStyle::Arrow)
                .hover(move |mut s| {
                    s = s.text_color(caption.hover_foreground);
                    if let Some(background) = caption.hover_background {
                        s = s.bg(background);
                    }
                    s
                })
                .child(icon.size(caption.icon_size))
        };
        let control = |id, icon, area, action: fn(&mut Window, &mut App)| {
            if cfg!(windows) {
                face(id, icon).window_control_area(area)
            } else {
                face(id, icon)
                    .on_mouse_down(MouseButton::Left, |_, window, cx| {
                        window.prevent_default();
                        cx.stop_propagation();
                    })
                    .on_click(move |_, window, cx| action(window, cx))
            }
        };
        let window_controls = csd.then(|| {
            let controls = window.window_controls();
            let maximize = if window.is_maximized() { Icon::Copy } else { Icon::Square };
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .pr_2()
                .when(controls.minimize, |d| {
                    d.child(control("minimize", Icon::Minus, WindowControlArea::Min, |window, _| window.minimize_window()))
                })
                .when(controls.maximize, |d| {
                    d.child(control("maximize", maximize, WindowControlArea::Max, |window, _| window.zoom_window()))
                })
                .child(control("close-window", Icon::X, WindowControlArea::Close, |window, cx| {
                    window.dispatch_action(Box::new(CloseWindow), cx)
                }))
        });

        div()
            .id("tab-bar")
            .flex()
            .flex_none()
            .items_end()
            .h(px(TAB_BAR_HEIGHT))
            .pl_2()
            .gap_0p5()
            .bg(tokens.background)
            .border_b_1()
            .border_color(tokens.border)
            // On Windows, Windows acts on caption presses only when Brindle
            // leaves them unhandled, and GPUI counts a press as handled when
            // it moves focus, which it does for any focusable element under
            // the mouse. The strip occludes the workspace, so a press here
            // focuses nothing and Windows drags, double-clicks, shows the
            // system menu and resizes from the top edge.
            .when(cfg!(windows), |d| d.occlude())
            // Elsewhere, empty space in the strip acts as the title bar.
            .when(csd && !cfg!(windows), |d| {
                d.on_mouse_down(MouseButton::Left, |e, window, _| {
                    if e.click_count >= 2 {
                        window.zoom_window();
                    } else {
                        window.start_window_move();
                    }
                })
                .on_mouse_down(MouseButton::Right, |e, window, _| window.show_window_menu(e.position))
            })
            .child(
                div()
                    .flex()
                    .flex_shrink()
                    .min_w_0()
                    .items_end()
                    // Enter and escape while the strip has focus go back to
                    // the terminal; the list has already taken its own keys.
                    .on_key_down(cx.listener(Self::strip_key_down))
                    .child(list),
            )
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .h(px(TAB_BAR_HEIGHT - 4.0))
                    .gap_0p5()
                    .pl_1()
                    .child(inert("new-tab-press", new_tab.into_any_element()))
                    .child(inert("profiles-press", profiles.into_any_element())),
            )
            // Windows hit-tests this as the caption: drag, double-click,
            // the system menu and Snap. Only empty space is marked, since a
            // marked area wins over everything inside it.
            .child(div().id("caption").flex_1().h_full().when(cfg!(windows), |d| d.window_control_area(WindowControlArea::Drag)))
            .children(window_controls.map(|c| div().flex().h_full().items_center().child(c)))
    }

    fn render_content(&self) -> AnyElement {
        match self.tabs.get(self.active).map(|t| &t.content) {
            Some(TabContent::Zones(view)) => view.clone().into_any_element(),
            Some(TabContent::Tmux { view, .. }) => view.clone().into_any_element(),
            None => div().into_any_element(),
        }
    }
}


/// Moves `items[from]` to `to`'s place: after it when it came from the
/// left (the items between move left), before it when it came from the
/// right, which is where a tab dropped on another lands. False, moving
/// nothing, for a drop on itself or an index out of range.
fn reorder<T>(items: &mut Vec<T>, from: usize, to: usize) -> bool {
    if from == to || from >= items.len() || to >= items.len() {
        return false;
    }
    let item = items.remove(from);
    items.insert(to, item);
    true
}

/// Whether a key pressed while the tab strip has focus sends focus back to
/// the active tab's terminal: enter or escape with no modifier held. The
/// tab list itself takes the arrows, home and end.
fn returns_from_strip(key: &str, modifiers: Modifiers) -> bool {
    !modifiers.modified() && matches!(key, "enter" | "escape")
}

/// Trims a submitted `Rename Tab` value: a non-empty string sets a manual
/// title, and an empty one clears it.
fn parse_manual_title(text: &str) -> Option<String> {
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// Automatic title for a local tab: the program's OSC title (ignoring empty
/// strings and Windows' default console path titles), else the foreground
/// process name, else the profile name.
pub(crate) fn terminal_automatic_title(
    osc_title: Option<&str>,
    profile_name: &str,
    foreground_process: impl FnOnce() -> Option<String>,
) -> String {
    let own = osc_title.filter(|t| {
        !t.trim().is_empty() && !(cfg!(windows) && crate::terminal::cmdline::is_console_title(t))
    });
    match own {
        Some(title) => title.to_string(),
        None => foreground_process().unwrap_or_else(|| profile_name.to_string()),
    }
}

/// Updates `displayed` from `manual_title` when set, else from `automatic_title`.
/// Returns whether `displayed` changed.
fn refresh_displayed_title(
    displayed: &mut SharedString,
    manual_title: Option<&str>,
    automatic_title: impl FnOnce() -> String,
) -> bool {
    let title: SharedString = match manual_title {
        Some(manual) => manual.to_string().into(),
        None => automatic_title().into(),
    };
    let changed = title != *displayed;
    *displayed = title;
    changed
}

/// The new name for a profile called `name`, if it was renamed (names
/// match ignoring case, as profile lookups do).
fn renamed<'a>(name: &str, renames: &'a [(String, String)]) -> Option<&'a str> {
    renames.iter().find(|(old, _)| old.eq_ignore_ascii_case(name)).map(|(_, new)| new.as_str())
}

/// Gives a tab's terminals `theme`, keeping its font size.
fn set_tab_theme(tab: &Tab, theme: Theme, font_size: Option<f32>, cx: &mut App) {
    match &tab.content {
        TabContent::Zones(view) => {
            let profile = tab.profile.clone();
            view.update(cx, |view, cx| view.set_profile(profile, theme, font_size, cx))
        }
        TabContent::Tmux { view, .. } => view.update(cx, |view, cx| view.set_profile(theme, font_size, cx)),
    }
}

/// The size to record for a window last recorded (or opened) at `known`,
/// whose platform bounds are now `outer`, or `inner` without the client
/// inset. Whether the window manager has grown the window by the inset yet
/// varies (X11 only gets `_GTK_FRAME_EXTENTS`), so either can be off by
/// the inset on each side: within that, `known` stands. A bigger change is
/// the user resizing, and then the inner size is what they see.
fn recorded_size(known: Size<Pixels>, outer: Size<Pixels>, inner: Size<Pixels>) -> Size<Pixels> {
    let slack = 2.0 * RESIZE_BORDER + 1.0;
    let near = |a: Size<Pixels>| {
        (f32::from(a.width) - f32::from(known.width)).abs() <= slack
            && (f32::from(a.height) - f32::from(known.height)).abs() <= slack
    };
    if near(outer) && near(inner) { known } else { inner }
}

/// The keys that run `action` in the element `focus`, as the palettes show them.
/// The rows of the palette shown after a session that was all the window
/// showed ended: "Close window", then a new tab with each profile.
/// Rows for a window whose only session ended. A control-mode profile
/// whose session another window shows goes there instead of a new tab.
fn session_ended_rows(profiles: &[Profile]) -> Vec<PaletteRow> {
    std::iter::once(PaletteRow { label: "Close window".into(), detail: None, shortcut: None })
        .chain(profiles.iter().map(|profile| PaletteRow {
            label: format!("New tab: {}", profile.name).into(),
            detail: Some(profile.description().into()),
            shortcut: None,
        }))
        .collect()
}

/// Rows for a profile whose session another Brindle shows: make a new
/// session instead, or open another profile. Profiles whose session the
/// window shows (`open`) are left out.
fn session_open_rows(profiles: &[Profile], picked: usize, open: &[SessionKey]) -> Vec<(PaletteRow, OpenChoice)> {
    let row = |label: String, detail: String| PaletteRow { label: label.into(), detail: Some(detail.into()), shortcut: None };
    let mut rows =
        vec![(row("New tmux session…".into(), format!("with profile {}", profiles[picked].name)), OpenChoice::NewSession)];
    rows.extend(
        profiles
            .iter()
            .enumerate()
            .filter(|&(ix, profile)| {
                ix != picked && SessionKey::for_profile(profile).is_none_or(|key| !open.contains(&key))
            })
            .map(|(ix, profile)| (row(format!("New tab: {}", profile.name), profile.description()), OpenChoice::Profile(ix))),
    );
    rows
}

pub(crate) fn shortcut(action: &dyn Action, focus: &FocusHandle, window: &Window) -> Option<SharedString> {
    let binding = window.highest_precedence_binding_for_action_in(action, focus)?;
    Some(binding.keystrokes().iter().map(|k| k.unparse()).collect::<Vec<_>>().join(" ").into())
}

/// The command that opens `path` in `$VISUAL`, else `$EDITOR`, else nano,
/// through the shell so values like `code -w` work.
#[cfg(unix)]
fn editor_command(path: &str) -> (String, Vec<String>) {
    let editor = std::env::var("VISUAL").or_else(|_| std::env::var("EDITOR")).unwrap_or_else(|_| "nano".into());
    ("/bin/sh".into(), vec!["-c".into(), format!("{editor} {}", shell_quote(path))])
}

/// The command that opens `path` in `%VISUAL%`, else `%EDITOR%`, else
/// Notepad, through cmd so values like `code -w` work.
#[cfg(windows)]
fn editor_command(path: &str) -> (String, Vec<String>) {
    let (visual, editor) = (std::env::var("VISUAL").ok(), std::env::var("EDITOR").ok());
    crate::terminal::cmdline::windows_editor_command(visual.as_deref(), editor.as_deref(), path)
}

#[cfg(unix)]
fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn resize_edge(pos: Point<Pixels>, inset: Pixels, size: Size<Pixels>) -> Option<ResizeEdge> {
    let near_left = pos.x < inset;
    let near_right = pos.x > size.width - inset;
    let near_top = pos.y < inset;
    let near_bottom = pos.y > size.height - inset;
    Some(match (near_top, near_bottom, near_left, near_right) {
        (true, _, true, _) => ResizeEdge::TopLeft,
        (true, _, _, true) => ResizeEdge::TopRight,
        (_, true, true, _) => ResizeEdge::BottomLeft,
        (_, true, _, true) => ResizeEdge::BottomRight,
        (true, _, _, _) => ResizeEdge::Top,
        (_, true, _, _) => ResizeEdge::Bottom,
        (_, _, true, _) => ResizeEdge::Left,
        (_, _, _, true) => ResizeEdge::Right,
        _ => return None,
    })
}

impl Focusable for Workspace {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let title = self.tabs.get(self.active).map(|t| t.title.clone()).unwrap_or_default();
        if title != self.last_title {
            window.set_window_title(&title);
            self.last_title = title;
        }

        let theme = self.active_theme(cx);
        let decorations = window.window_decorations();
        let inset = px(RESIZE_BORDER);
        let rounding = px(8.0);
        let config_error = Settings::get(cx).config_error.clone();

        // While the settings dialog previews a theme, only the overlay
        // toggles act, so nothing (such as a new tab) changes underneath it.
        let settings_open = self.settings.is_open();
        let body = div()
            .id("workspace")
            .key_context("Workspace")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::open_profile_picker))
            .on_action(cx.listener(Self::open_command_palette))
            .on_action(cx.listener(Self::open_settings))
            .when(Settings::get(cx).config.profiles.iter().any(|p| p.tmux == TmuxMode::Control), |d| {
                d.on_action(cx.listener(Self::tmux_new_session))
            })
            // Consumes keys bound to `Swallow` (ctrl-shift-d outside tmux tabs).
            .on_action(cx.listener(|_, _: &Swallow, _, _| {}))
            .when(!settings_open, |d| {
                d.on_action(cx.listener(Self::new_tab))
                    .on_action(cx.listener(Self::new_tab_with_profile))
                    .on_action(cx.listener(Self::close_tab))
                    .on_action(cx.listener(Self::next_tab))
                    .on_action(cx.listener(Self::prev_tab))
                    .on_action(cx.listener(Self::activate_tab))
                    .on_action(cx.listener(Self::move_tab_left))
                    .on_action(cx.listener(Self::move_tab_right))
                    .on_action(cx.listener(Self::rename_tab))
                    .on_action(cx.listener(Self::focus_tab_strip))
                    .on_action(cx.listener(Self::close_window))
                    .on_action(cx.listener(Self::attach_session))
                    .on_action(cx.listener(Self::increase_font_size))
                    .on_action(cx.listener(Self::decrease_font_size))
                    .on_action(cx.listener(Self::reset_font_size))
                    .on_action(cx.listener(Self::open_config))
            })
            .relative()
            .flex()
            .flex_col()
            .size_full()
            .overflow_hidden()
            .bg(theme.background.hsla())
            .font_family(Settings::get(cx).ui_font_family.clone())
            .cursor(CursorStyle::Arrow)
            .when_some(match decorations {
                Decorations::Client { tiling } => Some(tiling),
                Decorations::Server => None,
            }, |d, tiling| {
                d.border_color(cx.ui_theme().border)
                    .when(!(tiling.top || tiling.left), |d| d.rounded_tl(rounding))
                    .when(!(tiling.top || tiling.right), |d| d.rounded_tr(rounding))
                    .when(!(tiling.bottom || tiling.left), |d| d.rounded_bl(rounding))
                    .when(!(tiling.bottom || tiling.right), |d| d.rounded_br(rounding))
                    .when(!tiling.top, |d| d.border_t_1())
                    .when(!tiling.bottom, |d| d.border_b_1())
                    .when(!tiling.left, |d| d.border_l_1())
                    .when(!tiling.right, |d| d.border_r_1())
                    .when(!tiling.is_tiled(), |d| {
                        d.shadow(vec![gpui::BoxShadow {
                            color: gpui::black().opacity(0.35),
                            blur_radius: inset / 2.,
                            spread_radius: px(0.),
                            offset: point(px(0.0), px(0.0)),
                        }])
                    })
            })
            .on_mouse_move(|_, _, cx| cx.stop_propagation())
            .child(self.render_tab_bar(window, cx))
            .when_some(config_error, |d, err| {
                d.child(
                    div()
                        .flex_none()
                        .px_3()
                        .py_1()
                        .text_xs()
                        .bg(theme.ansi[1].mix(theme.background, 0.6).hsla())
                        .text_color(theme.foreground.hsla())
                        .child(SharedString::from(format!("Config error (using defaults): {err}"))),
                )
            })
            .child(div().flex_1().min_h_0().child(self.render_content()))
            // Last in the tree, so a modal backdrop paints above the rest
            // (one of the two is open at most).
            .children(self.palette.render(window, cx))
            .children(self.settings.render(window, cx));

        // With client-side decorations we draw a transparent margin around the
        // window for the shadow and the resize handles.
        match decorations {
            Decorations::Server => body.into_any_element(),
            Decorations::Client { tiling } => {
                window.set_client_inset(inset);
                div()
                    .id("window-backdrop")
                    .size_full()
                    .when(!tiling.top, |d| d.pt(inset))
                    .when(!tiling.bottom, |d| d.pb(inset))
                    .when(!tiling.left, |d| d.pl(inset))
                    .when(!tiling.right, |d| d.pr(inset))
                    .child(
                        canvas(
                            |_, window, _| {
                                window.insert_hitbox(
                                    gpui::Bounds::new(point(px(0.0), px(0.0)), window.window_bounds().get_bounds().size),
                                    gpui::HitboxBehavior::Normal,
                                )
                            },
                            move |_, hitbox, window, _| {
                                let mouse = window.mouse_position();
                                let size = window.window_bounds().get_bounds().size;
                                let Some(edge) = resize_edge(mouse, inset, size) else { return };
                                window.set_cursor_style(
                                    match edge {
                                        ResizeEdge::Top | ResizeEdge::Bottom => CursorStyle::ResizeUpDown,
                                        ResizeEdge::Left | ResizeEdge::Right => CursorStyle::ResizeLeftRight,
                                        ResizeEdge::TopLeft | ResizeEdge::BottomRight => CursorStyle::ResizeUpLeftDownRight,
                                        ResizeEdge::TopRight | ResizeEdge::BottomLeft => CursorStyle::ResizeUpRightDownLeft,
                                    },
                                    &hitbox,
                                );
                            },
                        )
                        .size_full()
                        .absolute(),
                    )
                    .on_mouse_move(|_, window, _| window.refresh())
                    .on_mouse_down(MouseButton::Left, move |e, window, _| {
                        let size = window.window_bounds().get_bounds().size;
                        if let Some(edge) = resize_edge(e.position, inset, size) {
                            window.start_window_resize(edge);
                        }
                    })
                    .child(body)
                    .into_any_element()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::size;

    /// The click-outside rule per palette (pointer presses can't be posted
    /// on Linux, so this stands in for an e2e case): questions ignore a
    /// press outside, plain lists and a prompt opened on its own close on
    /// it. The prompt that follows a question replaces it in place and
    /// keeps the slot's flag (`open_prompt` doesn't set it). Every kind is
    /// covered through its tag; `PaletteTag::of` is an exhaustive match, so
    /// a new kind needs a decision.
    #[test]
    fn questions_ignore_outside_presses() {
        let sticky: Vec<PaletteTag> = PaletteTag::ALL.into_iter().filter(|&tag| ignores_outside_press(tag)).collect();
        assert_eq!(sticky, [PaletteTag::SessionEnded, PaletteTag::SessionOpenElsewhere]);
        // The kinds that build without an app map to their tags.
        let elsewhere = PaletteKind::SessionOpenElsewhere { choices: Vec::new(), profile: 0 };
        assert_eq!(PaletteTag::of(&elsewhere), PaletteTag::SessionOpenElsewhere);
        assert_eq!(PaletteTag::of(&PaletteKind::Prompt), PaletteTag::Prompt);
        assert_eq!(PaletteTag::of(&PaletteKind::NewTab), PaletteTag::NewTab);
        assert_eq!(PaletteTag::of(&PaletteKind::Sessions { sessions: Vec::new() }), PaletteTag::Sessions);
    }

    /// The spec's drag scenario: the first tab dropped on the third lands in
    /// the third place; one dropped from the right lands before its target;
    /// a drop on itself or out of range moves nothing.
    #[test]
    fn reorder_puts_the_moved_item_in_the_targets_place() {
        let mut items = vec!['a', 'b', 'c', 'd'];
        assert!(reorder(&mut items, 0, 2));
        assert_eq!(items, ['b', 'c', 'a', 'd']);
        assert!(reorder(&mut items, 3, 1));
        assert_eq!(items, ['b', 'd', 'c', 'a']);
        assert!(!reorder(&mut items, 2, 2));
        assert!(!reorder(&mut items, 4, 0));
        assert!(!reorder(&mut items, 0, 4));
        assert_eq!(items, ['b', 'd', 'c', 'a']);
    }

    /// Enter and escape alone leave the strip; the list's own keys, tab
    /// (the focus scope's) and modified keys don't.
    #[test]
    fn enter_and_escape_leave_the_strip() {
        let none = Modifiers::default();
        assert!(returns_from_strip("enter", none));
        assert!(returns_from_strip("escape", none));
        for key in ["left", "right", "home", "end", "pageup", "pagedown", "tab", "space", "a"] {
            assert!(!returns_from_strip(key, none), "{key}");
        }
        assert!(!returns_from_strip("enter", Modifiers::shift()));
        assert!(!returns_from_strip("escape", Modifiers::control()));
    }

    /// The sessions palette says "No detached sessions" only when there
    /// are none; a filter that matches none of several gets the command
    /// list's own empty message.
    #[test]
    fn sessions_palette_says_none_only_when_there_are_none() {
        assert_eq!(sessions_empty_message(0).as_ref().map(|m| m.as_ref()), Some("No detached sessions"));
        assert_eq!(sessions_empty_message(1), None);
        assert_eq!(sessions_empty_message(3), None);
    }

    #[test]
    fn session_ended_rows_offer_closing_first_then_each_profile() {
        let config = Config::parse(
            "[[profiles]]\nname = \"tmux\"\ntmux = \"control\"\n[[profiles]]\nname = \"Shell\"\n",
        )
        .unwrap();
        let rows = session_ended_rows(&config.profiles);
        let labels: Vec<&str> = rows.iter().map(|r| r.label.as_ref()).collect();
        assert_eq!(labels, ["Close window", "New tab: tmux", "New tab: Shell"]);
        assert!(rows[0].detail.is_none());
        assert_eq!(rows[1].detail.as_ref().map(|d| d.to_string()), Some(config.profiles[0].description().to_string()));
    }

    fn open_rows_config() -> Config {
        Config::parse(
            "[[profiles]]\nname = \"tmux\"\ntmux = \"control\"\n\
             [[profiles]]\nname = \"Shell\"\n\
             [[profiles]]\nname = \"work\"\ntmux = \"control\"\ntmux_session = \"work\"\n\
             [[profiles]]\nname = \"classic\"\ntmux = \"plain\"\n",
        )
        .unwrap()
    }

    #[test]
    fn session_open_rows_offer_a_new_session_then_other_profiles() {
        let config = open_rows_config();
        let rows = session_open_rows(&config.profiles, 0, &[]);
        let labels: Vec<&str> = rows.iter().map(|(r, _)| r.label.as_ref()).collect();
        assert_eq!(labels, ["New tmux session…", "New tab: Shell", "New tab: work", "New tab: classic"]);
        let choices: Vec<OpenChoice> = rows.into_iter().map(|(_, c)| c).collect();
        assert_eq!(choices, [OpenChoice::NewSession, OpenChoice::Profile(1), OpenChoice::Profile(2), OpenChoice::Profile(3)]);
    }

    #[test]
    fn session_open_rows_skip_sessions_the_window_shows() {
        let config = open_rows_config();
        let open = [SessionKey::for_profile(&config.profiles[2]).unwrap()];
        let rows = session_open_rows(&config.profiles, 0, &open);
        let labels: Vec<&str> = rows.iter().map(|(r, _)| r.label.as_ref()).collect();
        assert_eq!(labels, ["New tmux session…", "New tab: Shell", "New tab: classic"]);
    }

    #[test]
    fn resize_edges() {
        let s = size(px(800.0), px(600.0));
        let i = px(6.0);
        assert_eq!(resize_edge(point(px(2.0), px(2.0)), i, s), Some(ResizeEdge::TopLeft));
        assert_eq!(resize_edge(point(px(400.0), px(598.0)), i, s), Some(ResizeEdge::Bottom));
        assert_eq!(resize_edge(point(px(799.0), px(300.0)), i, s), Some(ResizeEdge::Right));
        assert_eq!(resize_edge(point(px(400.0), px(300.0)), i, s), None);
    }

    #[test]
    fn recorded_sizes() {
        let known = size(px(1000.0), px(660.0));
        // Before and after the window manager adds the inset.
        assert_eq!(recorded_size(known, known, size(px(988.0), px(648.0))), known);
        assert_eq!(recorded_size(known, size(px(1012.0), px(672.0)), known), known);
        // Resized by the user.
        let inner = size(px(1188.0), px(748.0));
        assert_eq!(recorded_size(known, size(px(1200.0), px(760.0)), inner), inner);
    }

    #[cfg(unix)]
    #[test]
    fn quoting() {
        assert_eq!(shell_quote("/a b/c'd"), "'/a b/c'\\''d'");
    }

    #[test]
    fn renamed_profiles_match_ignoring_case() {
        let renames = vec![("Work".to_string(), "Job".to_string())];
        assert_eq!(renamed("Work", &renames), Some("Job"));
        assert_eq!(renamed("work", &renames), Some("Job"));
        assert_eq!(renamed("Shell", &renames), None);
        assert_eq!(renamed("Work", &[]), None);
    }

    #[test]
    fn title_refresh_automatic_manual_and_clear() {
        let mut displayed = SharedString::default();
        let mut manual = None;

        // Unset manual title: automatic title precedence (OSC -> foreground -> profile).
        assert!(refresh_displayed_title(&mut displayed, manual.as_deref(), || {
            terminal_automatic_title(None, "Shell", || None)
        }));
        assert_eq!(displayed.as_ref(), "Shell");
        assert!(!refresh_displayed_title(&mut displayed, manual.as_deref(), || {
            terminal_automatic_title(None, "Shell", || None)
        }));

        assert!(refresh_displayed_title(&mut displayed, manual.as_deref(), || {
            terminal_automatic_title(None, "Shell", || Some("htop".into()))
        }));
        assert_eq!(displayed.as_ref(), "htop");

        assert!(refresh_displayed_title(&mut displayed, manual.as_deref(), || {
            terminal_automatic_title(Some("vim: Cargo.toml"), "Shell", || Some("htop".into()))
        }));
        assert_eq!(displayed.as_ref(), "vim: Cargo.toml");

        // Manual title trims input and overrides OSC, foreground, profile and tmux changes.
        manual = parse_manual_title("  prod logs  ");
        assert_eq!(manual.as_deref(), Some("prod logs"));
        assert!(refresh_displayed_title(&mut displayed, manual.as_deref(), || "ignored".into()));
        assert_eq!(displayed.as_ref(), "prod logs");

        let mut probed = false;
        assert!(!refresh_displayed_title(&mut displayed, manual.as_deref(), || {
            probed = true;
            terminal_automatic_title(Some("new-osc"), "RenamedProfile", || Some("python".into()))
        }));
        assert!(!probed, "automatic source is not even queried while manual title is set");
        assert_eq!(displayed.as_ref(), "prod logs");
        assert!(!refresh_displayed_title(&mut displayed, manual.as_deref(), || "tmux-window-2".into()));
        assert_eq!(displayed.as_ref(), "prod logs");

        // Clearing with empty/whitespace input resumes automatic title calculation.
        manual = parse_manual_title("   \t ");
        assert_eq!(manual, None);
        assert!(refresh_displayed_title(&mut displayed, manual.as_deref(), || {
            terminal_automatic_title(None, "Shell", || Some("bash".into()))
        }));
        assert_eq!(displayed.as_ref(), "bash");
    }
}
