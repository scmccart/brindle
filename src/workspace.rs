//! A Brindle window: the tab strip (which doubles as the title bar), the
//! active tab's content, and the palettes.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;

use futures::channel::oneshot;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Action, AnyElement, App, AppContext as _, BorrowAppContext as _, Context, CursorStyle, Decorations, Entity, FocusHandle,
    Focusable, InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels, Point,
    Render, ResizeEdge, SharedString, Size, StatefulInteractiveElement, Styled, Subscription,
    Task, Window, WindowBounds, WindowHandle, canvas, div, point, px,
};
use gpui::EntityId;

use crate::actions::*;
use crate::config::{Config, Profile, TmuxMode, TmuxSessionEnd};
use crate::notify::osc::Source;
use crate::notify::{self, Notifier};
use crate::picker::{Palette, PaletteEvent, PaletteRow, PromptRequest, SubmitFn};
use crate::text_input::TextInput;
use crate::session::{self, RestoredTab, SessionStore, SnapshotTab, WindowState};
use crate::settings::Settings;
use crate::settings_dialog::{self, SettingsDialog, SettingsEvent};
use crate::terminal::{Terminal, TerminalEvent};
use crate::terminal_view::{GridLayout, PointerStep, TerminalView};
use crate::theme::Theme;
use crate::tmux::protocol::WindowId;
use crate::tmux::{AttachSpec, SessionKey, TmuxEvent, TmuxSession, has_session_command};
use crate::tmux_view::TmuxWindowView;

const TAB_BAR_HEIGHT: f32 = 38.0;
const RESIZE_BORDER: f32 = 6.0;

pub enum TabContent {
    Terminal(Entity<TerminalView>),
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
    /// Cached so rendering never probes /proc; see [`Tab::refresh_title`].
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
            title: SharedString::default(),
            _subscriptions: subscriptions,
        };
        tab.refresh_title(cx);
        tab
    }

    /// Recomputes the title: the program's OSC title, else the foreground
    /// process name, else the profile name. Returns whether it changed.
    fn refresh_title(&mut self, cx: &App) -> bool {
        let title: SharedString = match &self.content {
            TabContent::Terminal(view) => {
                let terminal = view.read(cx).terminal.read(cx);
                match terminal.title().filter(|t| !t.trim().is_empty()) {
                    Some(title) => title.to_string(),
                    None => terminal.foreground_process_name().unwrap_or_else(|| self.profile.name.clone()),
                }
            }
            TabContent::Tmux { view, .. } => view.read(cx).title(cx),
        }
        .into();
        let changed = title != self.title;
        self.title = title;
        changed
    }

    /// Whether this tab shows `session` (and, if given, its `window`).
    fn shows_tmux(&self, session: &Entity<TmuxSession>, window: Option<WindowId>, cx: &App) -> bool {
        matches!(&self.content, TabContent::Tmux { session: s, view }
            if s == session && window.is_none_or(|w| view.read(cx).window_id == w))
    }

    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match &self.content {
            TabContent::Terminal(view) => view.read(cx).focus_handle.clone(),
            TabContent::Tmux { view, .. } => view.read(cx).focus_handle(cx),
        }
    }

    fn theme(&self, cx: &App) -> Theme {
        match &self.content {
            TabContent::Terminal(view) => view.read(cx).terminal.read(cx).theme.clone(),
            TabContent::Tmux { view, .. } => view.read(cx).theme.clone(),
        }
    }

    fn active_terminal(&self, cx: &App) -> Option<Entity<Terminal>> {
        match &self.content {
            TabContent::Terminal(view) => Some(view.read(cx).terminal.clone()),
            TabContent::Tmux { view, .. } => view.read(cx).active_terminal(cx),
        }
    }

    /// The view of the terminal that has (or last had) focus in this tab.
    fn active_view(&self, cx: &App) -> Option<Entity<TerminalView>> {
        match &self.content {
            TabContent::Terminal(view) => Some(view.clone()),
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

/// What a new window opens with.
pub enum Launch {
    Request(LaunchRequest),
    /// A window recorded by session restore.
    Restore(WindowState),
    /// A new tmux session (named by tmux when `name` is `None`).
    NewSession { profile: Profile, name: Option<String>, cwd: Option<PathBuf> },
}

/// Where an action opens a profile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Tab,
    Window,
}

/// Where a tmux session is shown.
#[derive(Clone)]
enum SessionLocation {
    Here(Entity<TmuxSession>),
    Elsewhere(WindowHandle<Workspace>, Entity<TmuxSession>),
}

/// A row of the palette that asks about a session open elsewhere.
#[derive(Clone, Debug, PartialEq, Eq)]
enum OpenChoice {
    GoTo,
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
    /// Asks what to open instead of attaching a session another window
    /// shows: `profile` was asked for, as `target`.
    SessionOpenElsewhere { choices: Vec<OpenChoice>, profile: usize, location: SessionLocation, target: Target },
}

/// What is shown over the tabs; at most one at a time.
enum Overlay {
    Palette(OpenPalette),
    Settings { dialog: Entity<SettingsDialog>, _subscription: Subscription },
}

struct OpenPalette {
    palette: Entity<Palette>,
    kind: PaletteKind,
    _subscription: Subscription,
}

#[derive(Clone)]
struct DraggedTab {
    id: u64,
    title: SharedString,
    theme: Theme,
}

impl Render for DraggedTab {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let t = &self.theme;
        div()
            .px_3()
            .py_1()
            .rounded_md()
            .bg(t.background.hsla())
            .border_1()
            .border_color(t.accent().hsla())
            .text_sm()
            .text_color(t.foreground.hsla())
            .child(self.title.clone())
    }
}

pub struct Workspace {
    tabs: Vec<Tab>,
    active: usize,
    overlay: Option<Overlay>,
    /// A theme being previewed by the settings dialog, shown on the tabs
    /// that follow the global theme.
    theme_preview: Option<Theme>,
    /// The profile the settings dialog is editing (by its saved name) and
    /// the settings its tabs preview: theme and font size.
    profile_preview: Option<(String, Profile)>,
    focus_handle: FocusHandle,
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
            overlay: None,
            theme_preview: None,
            profile_preview: None,
            focus_handle: cx.focus_handle(),
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
                    for tab in &mut this.tabs {
                        changed |= tab.refresh_title(cx);
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
        match launch {
            Launch::Request(request) => this.launch(request, window, cx),
            Launch::Restore(state) => this.restore(state, window, cx),
            Launch::NewSession { profile, name, cwd } => {
                this.create_session(profile, AttachSpec::New(name), cwd, None, window, cx);
            }
        }
        this
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
                RestoredTab::Local { profile, cwd } => {
                    self.launch(LaunchRequest { profile, command: None, cwd: Some(cwd) }, window, cx);
                    let Some(tab) = self.tabs.get(self.active) else { continue };
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
                (_, TabContent::Terminal(view)) => SnapshotTab::Local {
                    profile: tab.profile.name.clone(),
                    cwd: view.read(cx).terminal.read(cx).working_directory(),
                },
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
        let enabled = Self::settings(cx).restore_session;
        if !cx.update_global(|store: &mut SessionStore, _| store.is_owner(enabled)) {
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

    fn active_cwd(&self, cx: &App) -> Option<PathBuf> {
        let tab = self.tabs.get(self.active)?;
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

        if profile.tmux == TmuxMode::Control && request.command.is_none() {
            self.attach_tmux(profile, request.cwd, window, cx);
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

        let cwd = request.cwd.or_else(|| {
            // New tabs follow the current directory unless the profile pins one.
            if profile.cwd.is_none() { self.active_cwd(cx) } else { None }
        });

        let terminal = cx.new(|cx| Terminal::spawn_pty(&profile, command, cwd, &config, cx));
        let font_size = profile.font_size;
        let view = cx.new(|cx| TerminalView::new(terminal.clone(), font_size, window, cx));
        let subscriptions = vec![cx.subscribe_in(&terminal, window, |this, terminal, event, window, cx| {
            this.on_terminal_event(terminal, event, window, cx)
        })];
        let tab = Tab::new(profile, TabContent::Terminal(view), TabOrigin::Local, subscriptions, cx);
        self.insert_tab(tab, None, window, cx);
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

    /// Attaches `profile`'s session, or focuses it if this window already
    /// shows it. A session another window shows is not attached again
    /// (callers that may hit that ask first, see `open_profile`). Returns
    /// the newly attached session.
    fn attach_tmux(
        &mut self,
        profile: Profile,
        cwd: Option<PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<TmuxSession>> {
        if let Some(key) = SessionKey::for_profile(&profile) {
            match self.find_session(&key, window, cx) {
                Some(SessionLocation::Here(session)) => {
                    self.show_session(&session, window, cx);
                    return None;
                }
                Some(SessionLocation::Elsewhere(..)) => {
                    log::info!("tmux session {:?} is shown in another window; not attaching it again", profile.tmux_session_name());
                    return None;
                }
                None => {}
            }
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
        let cwd = cwd.or_else(|| self.active_cwd(cx));
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

    /// Where `key`'s session is shown: this window first, then the others.
    fn find_session(&self, key: &SessionKey, window: &Window, cx: &App) -> Option<SessionLocation> {
        let shows = |sessions: &[(Entity<TmuxSession>, Subscription)]| {
            sessions.iter().map(|(s, _)| s).find(|s| s.read(cx).key().as_ref() == Some(key)).cloned()
        };
        if let Some(session) = shows(&self.tmux_sessions) {
            return Some(SessionLocation::Here(session));
        }
        self.other_workspaces(window, cx).into_iter().find_map(|handle| {
            // Another window's root is readable unless it is mid-update,
            // which only this window can be.
            let session = shows(&handle.read(cx).ok()?.tmux_sessions)?;
            Some(SessionLocation::Elsewhere(handle, session))
        })
    }

    /// The sessions every window shows.
    fn open_sessions(&self, window: &Window, cx: &App) -> Vec<SessionKey> {
        let keys = |sessions: &[(Entity<TmuxSession>, Subscription)]| -> Vec<SessionKey> {
            sessions.iter().filter_map(|(s, _)| s.read(cx).key()).collect()
        };
        let mut open = keys(&self.tmux_sessions);
        for handle in self.other_workspaces(window, cx) {
            if let Ok(ws) = handle.read(cx) {
                open.extend(keys(&ws.tmux_sessions));
            }
        }
        open
    }

    fn other_workspaces(&self, window: &Window, cx: &App) -> Vec<WindowHandle<Workspace>> {
        let own = window.window_handle().window_id();
        cx.windows().into_iter().filter(|w| w.window_id() != own).filter_map(|w| w.downcast::<Workspace>()).collect()
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

    /// Shows the session where it is, bringing its window up if that's
    /// another one.
    fn go_to_session(&mut self, location: SessionLocation, window: &mut Window, cx: &mut Context<Self>) {
        match location {
            SessionLocation::Here(session) => self.show_session(&session, window, cx),
            SessionLocation::Elsewhere(handle, session) => {
                // Asked for while this window still has focus.
                let token = notify::activation::request_token(window);
                cx.defer(move |cx| {
                    handle
                        .update(cx, |ws, window, cx| {
                            ws.show_session(&session, window, cx);
                            notify::activation::raise(window, token.as_deref());
                        })
                        .ok();
                });
            }
        }
    }

    /// Opens profile `ix` as `target` says, unless that would attach a
    /// session another window shows (or, for a new window, any window):
    /// then asks what to open instead.
    fn open_profile(&mut self, ix: usize, target: Target, window: &mut Window, cx: &mut Context<Self>) {
        let Some(profile) = Self::settings(cx).profiles.get(ix).cloned() else { return };
        let location = SessionKey::for_profile(&profile).and_then(|key| self.find_session(&key, window, cx));
        match (location, target) {
            (None, Target::Tab) => self.launch(LaunchRequest::profile(ix), window, cx),
            (None, Target::Window) => {
                crate::open_window(Launch::Request(LaunchRequest::profile(ix)), cx);
            }
            (Some(SessionLocation::Here(session)), Target::Tab) => self.show_session(&session, window, cx),
            (Some(location), target) => self.ask_session_open(ix, location, target, window, cx),
        }
    }

    fn ask_session_open(
        &mut self,
        profile: usize,
        location: SessionLocation,
        target: Target,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (SessionLocation::Here(session) | SessionLocation::Elsewhere(_, session)) = &location;
        let name = session.read(cx).session_name.clone().unwrap_or_default();
        let open = self.open_sessions(window, cx);
        let (rows, choices) = session_open_rows(&Self::settings(cx).profiles, profile, &name, &open, target)
            .into_iter()
            .unzip();
        let placeholder = match location {
            SessionLocation::Here(_) => format!("tmux session \"{name}\" is open in this window"),
            SessionLocation::Elsewhere(..) => format!("tmux session \"{name}\" is open in another window"),
        };
        let theme = self.active_theme(cx);
        let palette = cx.new(|cx| Palette::list(rows, placeholder, theme, cx));
        let kind = PaletteKind::SessionOpenElsewhere { choices, profile, location, target };
        self.show_palette(palette, kind, window, cx);
    }

    /// Asks for a name, then creates that tmux session on `profile`'s
    /// server, here or in a new window.
    fn ask_new_session(&mut self, profile: Profile, target: Target, window: &mut Window, cx: &mut Context<Self>) {
        let handle = window.window_handle().downcast::<Workspace>();
        let cwd = self.active_cwd(cx);
        let submit: SubmitFn = Rc::new(move |name, cx| {
            let name = Some(name.trim().to_string()).filter(|n| !n.is_empty());
            let (reply, result) = oneshot::channel();
            let (profile, cwd) = (profile.clone(), cwd.clone());
            match target {
                // The palette is updating this window: attach once it's done.
                Target::Tab => cx.defer(move |cx| {
                    if let Some(handle) = handle {
                        handle
                            .update(cx, |ws, window, cx| {
                                ws.create_session(profile, AttachSpec::New(name), cwd, Some(reply), window, cx);
                            })
                            .ok();
                    }
                }),
                // Check the name first, so a taken one is reported here
                // rather than in a window that opens only to fail.
                Target::Window => cx
                    .spawn(async move |cx| {
                        if let Some(name) = &name {
                            let (program, args) = has_session_command(&profile, name);
                            let env = crate::terminal::child_env(&profile);
                            let taken = cx
                                .background_executor()
                                .spawn(async move {
                                    std::process::Command::new(program)
                                        .args(args)
                                        .envs(env)
                                        .stdin(std::process::Stdio::null())
                                        .stdout(std::process::Stdio::null())
                                        .stderr(std::process::Stdio::null())
                                        .status()
                                        .is_ok_and(|s| s.success())
                                })
                                .await;
                            if taken {
                                reply.send(Err(format!("duplicate session: {name}"))).ok();
                                return;
                            }
                        }
                        cx.update(|cx| crate::open_window(Launch::NewSession { profile, name, cwd }, cx)).ok();
                        reply.send(Ok(())).ok();
                    })
                    .detach(),
            }
            result
        });
        let label = "New tmux session name (empty: tmux picks one)".into();
        self.open_prompt(PromptRequest { label, initial: String::new(), submit }, window, cx);
    }

    fn tmux_new_session(&mut self, _: &TmuxNewSession, window: &mut Window, cx: &mut Context<Self>) {
        let config = Self::settings(cx);
        let tab_profile = self.tabs.get(self.active).and_then(|t| match &t.content {
            TabContent::Tmux { session, .. } => Some(session.read(cx).profile.clone()),
            TabContent::Terminal(_) => None,
        });
        let default = &config.profiles[config.default_profile_index()];
        let profile = tab_profile
            .or_else(|| (default.tmux == TmuxMode::Control).then(|| default.clone()))
            .or_else(|| config.profiles.iter().find(|p| p.tmux == TmuxMode::Control).cloned());
        if let Some(profile) = profile {
            self.ask_new_session(profile, Target::Tab, window, cx);
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
                            tab.refresh_title(cx);
                        }
                        cx.notify();
                    }),
                    cx.subscribe_in(&view, window, |this, _, request: &PromptRequest, window, cx| {
                        this.open_prompt(request.clone(), window, cx)
                    }),
                    // Bells and notifications from the window's panes.
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
        let open = self.open_sessions(window, cx);
        let rows = session_ended_rows(&Self::settings(cx).profiles, &open);
        let placeholder = format!("tmux session \"{}\" ended", session.read(cx).session_name.as_deref().unwrap_or("?"));
        let theme = self.active_theme(cx);
        let palette = cx.new(|cx| Palette::list(rows, placeholder, theme, cx).modal());
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

    fn on_terminal_event(
        &mut self,
        terminal: &Entity<Terminal>,
        event: &TerminalEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.tabs.iter().position(|t| {
            matches!(&t.content, TabContent::Terminal(view) if view.read(cx).terminal == *terminal)
        }) else {
            return;
        };
        match event {
            TerminalEvent::Exited => self.remove_tab(ix, window, cx),
            TerminalEvent::Bell | TerminalEvent::Notify(_) => self.on_attention(ix, event, window, cx),
            TerminalEvent::TitleChanged => {
                if self.tabs[ix].refresh_title(cx) {
                    cx.notify();
                }
            }
        }
    }

    /// A bell or a notification request from tab `ix`, local or tmux: marks
    /// the tab for a bell, and raises a desktop notification if the tab
    /// isn't in front of the user (or the settings say always).
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

    /// Activates the tab with this id, if it is still open (a notification
    /// for it was clicked).
    pub fn show_tab(&mut self, tab_id: u64, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ix) = self.tabs.iter().position(|t| t.id == tab_id) {
            self.activate(ix, window, cx);
        }
    }

    fn activate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.tabs.is_empty() {
            return;
        }
        self.active = index.min(self.tabs.len() - 1);
        let tab = &mut self.tabs[self.active];
        tab.bell = false;
        if let TabContent::Tmux { session, view } = &tab.content {
            let window_id = view.read(cx).window_id;
            session.update(cx, |s, _| s.select_window(window_id));
        }
        let focus = tab.focus_handle(cx);
        if self.overlay.is_none() {
            window.focus(&focus);
        }
        cx.notify();
    }

    fn remove_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(ix);
        if let TabContent::Terminal(view) = &tab.content {
            view.read(cx).terminal.clone().update(cx, |t, _| t.shutdown());
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
        let terminal = cx.new(|cx| Terminal::message(text, theme, &config, cx));
        let view = cx.new(|cx| TerminalView::new(terminal, profile.font_size, window, cx));
        let tab = Tab::new(profile, TabContent::Terminal(view), TabOrigin::Message, Vec::new(), cx);
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
        self.open_profile(profile, Target::Tab, window, cx);
    }

    fn new_tab_with_profile(&mut self, action: &NewTabWithProfile, window: &mut Window, cx: &mut Context<Self>) {
        self.open_profile(action.0, Target::Tab, window, cx);
    }

    fn new_window(&mut self, _: &NewWindow, window: &mut Window, cx: &mut Context<Self>) {
        let profile = Self::settings(cx).default_profile_index();
        self.open_profile(profile, Target::Window, window, cx);
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

    fn drop_tab(&mut self, dragged: &DraggedTab, target: usize, cx: &mut Context<Self>) {
        let Some(from) = self.tabs.iter().position(|t| t.id == dragged.id) else { return };
        let active_id = self.tabs[self.active].id;
        let tab = self.tabs.remove(from);
        let to = target.min(self.tabs.len());
        self.tabs.insert(to, tab);
        self.active = self.tabs.iter().position(|t| t.id == active_id).unwrap_or(0);
        cx.notify();
    }

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
                // Picking a session another window shows goes there; say so.
                let shown = SessionKey::for_profile(profile).and_then(|key| self.find_session(&key, window, cx));
                let state = match shown {
                    Some(SessionLocation::Here(_)) => " · open in this window",
                    Some(SessionLocation::Elsewhere(..)) => " · in another window (switches)",
                    None => "",
                };
                PaletteRow {
                    label: profile.name.clone().into(),
                    detail: Some(format!("{}{state}", profile.description()).into()),
                    shortcut: tab_focus.as_ref().and_then(|f| shortcut(&NewTabWithProfile(ix), f, window)),
                }
            })
            .collect();
        let theme = self.active_theme(cx);
        let palette = cx.new(|cx| Palette::list(rows, "Open profile…", theme, cx));
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
        let theme = self.active_theme(cx);
        let palette = cx.new(|cx| Palette::list(rows, "Run a command…", theme, cx));
        self.show_palette(palette, PaletteKind::Commands { actions, tab_focus }, window, cx);
    }

    /// Asks for a line of text, replacing any open palette.
    fn open_prompt(&mut self, request: PromptRequest, window: &mut Window, cx: &mut Context<Self>) {
        let theme = self.active_theme(cx);
        let palette = cx.new(|cx| Palette::prompt(request, theme, cx));
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
        if self.overlay.is_none() {
            return false;
        }
        let same = matches!(&self.overlay, Some(Overlay::Palette(open)) if is(&open.kind));
        self.dismiss_overlay(window, cx);
        same
    }

    fn show_palette(
        &mut self,
        palette: Entity<Palette>,
        kind: PaletteKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let subscription = cx.subscribe_in(&palette, window, |this, _, event, window, cx| match event {
            PaletteEvent::Confirmed(ix) => this.palette_confirmed(*ix, window, cx),
            PaletteEvent::Dismissed => this.palette_dismissed(window, cx),
        });
        self.close_overlay(cx);
        window.focus(&palette.focus_handle(cx));
        self.overlay = Some(Overlay::Palette(OpenPalette { palette, kind, _subscription: subscription }));
        cx.notify();
    }

    /// Escape (or a click outside) on a palette. After a session ended,
    /// that closes the window.
    fn palette_dismissed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ended = match &self.overlay {
            Some(Overlay::Palette(OpenPalette { kind: PaletteKind::SessionEnded { session }, .. })) => {
                Some(session.clone())
            }
            _ => None,
        };
        self.dismiss_overlay(window, cx);
        if let Some(session) = ended {
            self.remove_session_tabs(&session, window, cx);
        }
    }

    fn palette_confirmed(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(Overlay::Palette(open)) = self.overlay.take() else { return };
        match open.kind {
            PaletteKind::NewTab => {
                self.dismiss_overlay(window, cx);
                // The row said the session is open in another window, so
                // going there is what was picked; nothing to ask.
                let profile = Self::settings(cx).profiles.get(ix).cloned();
                let location = profile
                    .and_then(|p| SessionKey::for_profile(&p))
                    .and_then(|key| self.find_session(&key, window, cx));
                match location {
                    Some(location @ SessionLocation::Elsewhere(..)) => self.go_to_session(location, window, cx),
                    _ => self.open_profile(ix, Target::Tab, window, cx),
                }
            }
            PaletteKind::SessionOpenElsewhere { choices, profile, location, target } => {
                self.dismiss_overlay(window, cx);
                match choices.get(ix) {
                    Some(OpenChoice::GoTo) => self.go_to_session(location, window, cx),
                    Some(OpenChoice::NewSession) => {
                        if let Some(profile) = Self::settings(cx).profiles.get(profile).cloned() {
                            self.ask_new_session(profile, target, window, cx);
                        }
                    }
                    Some(&OpenChoice::Profile(other)) => match target {
                        Target::Tab => self.launch(LaunchRequest::profile(other), window, cx),
                        Target::Window => {
                            crate::open_window(Launch::Request(LaunchRequest::profile(other)), cx);
                        }
                    },
                    None => {}
                }
            }
            PaletteKind::Commands { actions, tab_focus } => {
                // Run it from the tab, exactly as its keybinding would.
                window.focus(&tab_focus);
                cx.notify();
                if let Some(action) = actions.into_iter().nth(ix) {
                    window.dispatch_action(action, cx);
                }
            }
            // A prompt has no rows to confirm.
            PaletteKind::Prompt => {}
            PaletteKind::SessionEnded { session } => {
                self.dismiss_overlay(window, cx);
                let profile = ix.checked_sub(1).and_then(|p| Self::settings(cx).profiles.get(p).cloned());
                match profile {
                    // A control-mode session's tabs arrive later; keep the
                    // window open with the ended tabs until they do.
                    Some(profile) if profile.tmux == TmuxMode::Control => {
                        let location = SessionKey::for_profile(&profile).and_then(|key| self.find_session(&key, window, cx));
                        if let Some(location @ SessionLocation::Elsewhere(..)) = location {
                            // "Go to <session>": this window has nothing left.
                            self.go_to_session(location, window, cx);
                        } else if let Some(new) = self.attach_tmux(profile, None, window, cx) {
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

    fn open_settings(&mut self, _: &OpenSettings, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.overlay, Some(Overlay::Settings { .. })) {
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
        let dialog = cx.new(|cx| SettingsDialog::new(model, theme, cx));
        let subscription = cx.subscribe_in(&dialog, window, |this, _, event, window, cx| match event {
            SettingsEvent::Preview(theme) => this.set_theme_preview(Some(theme.clone()), cx),
            // Deferred by GPUI, so the reload sees this window idle.
            SettingsEvent::ProfilePreview(preview) => this.set_profile_preview(preview.clone(), cx),
            // Deferred, so the reload sees this window idle (as ReloadConfig).
            SettingsEvent::ConfigWritten(renames) => {
                let renames = renames.clone();
                cx.defer(move |cx| crate::reload_config_renaming(cx, &renames));
            }
            SettingsEvent::Close => this.dismiss_overlay(window, cx),
        });
        self.close_overlay(cx);
        window.focus(&dialog.focus_handle(cx));
        self.overlay = Some(Overlay::Settings { dialog, _subscription: subscription });
        cx.notify();
    }

    /// Closes whatever overlay is open and gives focus back to the tab.
    fn dismiss_overlay(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_overlay(cx);
        if let Some(tab) = self.tabs.get(self.active) {
            window.focus(&tab.focus_handle(cx));
        }
        cx.notify();
    }

    /// Drops the open overlay. Closing the settings dialog this way cancels
    /// it, ending any preview.
    fn close_overlay(&mut self, cx: &mut Context<Self>) {
        self.overlay = None;
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
        let path = Config::path();
        let editor = std::env::var("VISUAL")
            .or_else(|_| std::env::var("EDITOR"))
            .unwrap_or_else(|_| "nano".into());
        // Run through the shell so EDITOR values like "code -w" work.
        let command = format!("{editor} {}", shell_quote(&path.to_string_lossy()));
        let profile = Self::settings(cx).default_profile_index();
        self.launch(
            LaunchRequest { profile, command: Some(("/bin/sh".into(), vec!["-c".into(), command])), cwd: None },
            window,
            cx,
        );
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
                tab.profile = p.clone();
            }
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
        match &self.overlay {
            Some(Overlay::Palette(open)) => {
                let input = open.palette.read(cx).input();
                input.read(cx).is_focused(window).then(|| input.clone())
            }
            Some(Overlay::Settings { dialog, .. }) => dialog.read(cx).focused_input(window, cx),
            None => {
                let input = self.tabs.get(self.active)?.active_view(cx)?.read(cx).find_input()?;
                input.read(cx).is_focused(window).then_some(input)
            }
        }
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

    /// Debug hook for `--dump-screen-after`.
    /// When this window was opened relative to the others.
    pub fn creation_order(&self) -> u64 {
        self.order
    }

    pub fn tab_titles(&self) -> Vec<String> {
        self.tabs.iter().map(|t| t.title.to_string()).collect()
    }

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
            out.push_str(&terminal.screen_text());
        }
        if let Some(find) = self.tabs.get(self.active).and_then(|t| t.active_view(cx)).and_then(|v| v.read(cx).describe_find(cx)) {
            out.push_str(&find);
        }
        match &self.overlay {
            Some(Overlay::Palette(open)) => out.push_str(&open.palette.read(cx).describe(cx)),
            Some(Overlay::Settings { dialog, .. }) => out.push_str(&dialog.read(cx).describe()),
            None => {}
        }
        if let Some(input) = self.focused_input(window, cx) {
            out.push_str(&format!("--- input {}\n", input.read(cx).describe()));
        }
        out
    }

    // ---- rendering --------------------------------------------------------------

    fn render_tab_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.active_theme(cx);
        let t = &theme;
        let chrome = t.chrome_background();
        let fg = t.foreground.hsla();
        let muted = t.muted_foreground().hsla();
        let accent = t.accent().hsla();
        let border = t.chrome_border().hsla();
        let decorations = window.window_decorations();
        let csd = matches!(decorations, Decorations::Client { .. });

        let tabs = self.tabs.iter().enumerate().map(|(ix, tab)| {
            let active = ix == self.active;
            let title = tab.title.clone();
            let bell = tab.bell;
            let tmux = tab.is_tmux();
            let dragged = DraggedTab { id: tab.id, title: title.clone(), theme: theme.clone() };
            div()
                .id(("tab", tab.id as usize))
                .group("tab")
                .relative()
                .flex()
                .flex_shrink()
                .min_w(px(80.0))
                .max_w(px(240.0))
                .h(px(TAB_BAR_HEIGHT - 8.0))
                .items_center()
                .gap_1p5()
                .pl_3()
                .pr_1()
                .rounded_t_md()
                .text_sm()
                .cursor(CursorStyle::Arrow)
                .when(active, |d| d.bg(t.background.hsla()).text_color(fg))
                .when(!active, |d| d.text_color(muted).hover(|d| d.bg(t.hover_background().hsla())))
                .on_mouse_down(MouseButton::Left, cx.listener(move |this, _, window, cx| {
                    this.activate(ix, window, cx);
                    cx.stop_propagation();
                }))
                .on_mouse_down(MouseButton::Middle, cx.listener(move |this, _, window, cx| {
                    this.close_tab_at(ix, window, cx);
                    cx.stop_propagation();
                }))
                .on_drag(dragged, |dragged, _, _, cx| cx.new(|_| dragged.clone()))
                .drag_over::<DraggedTab>(move |style, _, _, _| style.border_l_2().border_color(accent))
                .on_drop(cx.listener(move |this, dragged: &DraggedTab, _, cx| this.drop_tab(dragged, ix, cx)))
                .when(active, |d| {
                    d.child(div().absolute().top_0().left_0().right_0().h(px(2.0)).rounded_t_md().bg(accent))
                })
                .when(tmux, |d| {
                    d.child(div().text_xs().text_color(accent).child("⧉"))
                })
                .child(
                    div()
                        .flex_1()
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .text_ellipsis()
                        .child(title),
                )
                .when(bell, |d| d.child(div().size(px(6.0)).rounded_full().bg(accent)))
                .child(
                    div()
                        .id(("close", ix))
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .size(px(20.0))
                        .rounded_sm()
                        .text_color(muted)
                        .when(!active, |d| d.invisible().group_hover("tab", |d| d.visible()))
                        .hover(|d| d.bg(t.hover_background().mix(t.foreground, 0.1).hsla()).text_color(fg))
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_click(cx.listener(move |this, _, window, cx| this.close_tab_at(ix, window, cx)))
                        .child("×"),
                )
        });

        let button = |id: &'static str, label: &'static str| {
            div()
                .id(id)
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .size(px(26.0))
                .rounded_md()
                .text_color(muted)
                .cursor(CursorStyle::Arrow)
                .hover(|d| d.bg(t.hover_background().hsla()).text_color(fg))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(label)
        };

        let window_controls = csd.then(|| {
            let controls = window.window_controls();
            div()
                .flex()
                .flex_none()
                .items_center()
                .gap_1()
                .pr_2()
                .when(controls.minimize, |d| {
                    d.child(button("minimize", "–").on_click(|_, window, _| window.minimize_window()))
                })
                .when(controls.maximize, |d| {
                    d.child(button("maximize", if window.is_maximized() { "❐" } else { "□" }).on_click(|_, window, _| window.zoom_window()))
                })
                .child(button("close-window", "×").on_click(|_, window, _| window.remove_window()))
        });

        div()
            .id("tab-bar")
            .flex()
            .flex_none()
            .items_end()
            .h(px(TAB_BAR_HEIGHT))
            .pl_2()
            .gap_0p5()
            .bg(chrome.hsla())
            .border_b_1()
            .border_color(border)
            // Empty space in the strip acts as the title bar.
            .when(csd, |d| {
                d.on_mouse_down(MouseButton::Left, |e, window, _| {
                    if e.click_count >= 2 {
                        window.zoom_window();
                    } else {
                        window.start_window_move();
                    }
                })
                .on_mouse_down(MouseButton::Right, |e, window, _| window.show_window_menu(e.position))
            })
            .child(div().flex().flex_shrink().min_w_0().items_end().gap_0p5().overflow_hidden().children(tabs))
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .h(px(TAB_BAR_HEIGHT - 4.0))
                    .gap_0p5()
                    .pl_1()
                    .child(button("new-tab", "+").on_click(cx.listener(|this, _, window, cx| {
                        this.new_tab(&NewTab, window, cx)
                    })))
                    .child(button("profiles", "⌄").on_click(cx.listener(|this, _, window, cx| {
                        this.open_profile_picker(&OpenProfilePicker, window, cx)
                    }))),
            )
            .child(div().flex_1().h_full())
            .children(window_controls.map(|c| div().flex().h_full().items_center().child(c)))
    }

    fn render_overlay(&self) -> Option<AnyElement> {
        let overlay = match self.overlay.as_ref()? {
            Overlay::Palette(open) => open.palette.clone().into_any_element(),
            Overlay::Settings { dialog, .. } => dialog.clone().into_any_element(),
        };
        Some(
            div()
                .absolute()
                .top(px(TAB_BAR_HEIGHT + 8.0))
                .left_0()
                .right_0()
                .flex()
                .justify_center()
                .child(overlay)
                .into_any_element(),
        )
    }

    fn render_content(&self) -> AnyElement {
        match self.tabs.get(self.active).map(|t| &t.content) {
            Some(TabContent::Terminal(view)) => view.clone().into_any_element(),
            Some(TabContent::Tmux { view, .. }) => view.clone().into_any_element(),
            None => div().into_any_element(),
        }
    }
}

/// Gives a tab's terminals `theme`, keeping its font size.
/// The new name for a profile called `name`, if it was renamed (names
/// match ignoring case, as profile lookups do).
fn renamed<'a>(name: &str, renames: &'a [(String, String)]) -> Option<&'a str> {
    renames.iter().find(|(old, _)| old.eq_ignore_ascii_case(name)).map(|(_, new)| new.as_str())
}

fn set_tab_theme(tab: &Tab, theme: Theme, font_size: Option<f32>, cx: &mut App) {
    match &tab.content {
        TabContent::Terminal(view) => view.update(cx, |view, cx| {
            view.font_size_override = font_size;
            view.terminal.update(cx, |t, _| t.theme = theme)
        }),
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
fn session_ended_rows(profiles: &[Profile], open: &[SessionKey]) -> Vec<PaletteRow> {
    std::iter::once(PaletteRow { label: "Close window".into(), detail: None, shortcut: None })
        .chain(profiles.iter().map(|profile| {
            let shown = SessionKey::for_profile(profile).is_some_and(|key| open.contains(&key));
            let label = if shown {
                format!("Go to {}", profile.tmux_session_name())
            } else {
                format!("New tab: {}", profile.name)
            };
            PaletteRow { label: label.into(), detail: Some(profile.description().into()), shortcut: None }
        }))
        .collect()
}

/// Rows for a profile whose session another window shows: go there, make
/// a new session instead, or open another profile. Profiles that would ask
/// the same question again are left out.
fn session_open_rows(
    profiles: &[Profile],
    picked: usize,
    session: &str,
    open: &[SessionKey],
    target: Target,
) -> Vec<(PaletteRow, OpenChoice)> {
    let row = |label: String, detail: String| PaletteRow { label: label.into(), detail: Some(detail.into()), shortcut: None };
    let verb = match target {
        Target::Tab => "New tab",
        Target::Window => "New window",
    };
    let mut rows = vec![
        (PaletteRow { label: format!("Go to {session}").into(), detail: None, shortcut: None }, OpenChoice::GoTo),
        (row("New tmux session…".into(), format!("with profile {}", profiles[picked].name)), OpenChoice::NewSession),
    ];
    rows.extend(
        profiles
            .iter()
            .enumerate()
            .filter(|&(ix, profile)| {
                ix != picked && SessionKey::for_profile(profile).is_none_or(|key| !open.contains(&key))
            })
            .map(|(ix, profile)| (row(format!("{verb}: {}", profile.name), profile.description()), OpenChoice::Profile(ix))),
    );
    rows
}

fn shortcut(action: &dyn Action, focus: &FocusHandle, window: &Window) -> Option<SharedString> {
    let binding = window.highest_precedence_binding_for_action_in(action, focus)?;
    Some(binding.keystrokes().iter().map(|k| k.unparse()).collect::<Vec<_>>().join(" ").into())
}

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
        let settings_open = matches!(self.overlay, Some(Overlay::Settings { .. }));
        let body = div()
            .id("workspace")
            .key_context("Workspace")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::open_profile_picker))
            .on_action(cx.listener(Self::open_command_palette))
            .on_action(cx.listener(Self::open_settings))
            .on_action(cx.listener(Self::new_window))
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
                    .on_action(cx.listener(Self::close_window))
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
                d.border_color(theme.chrome_border().hsla())
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
            .when_some(self.render_overlay(), |d, overlay| d.child(overlay));

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

    #[test]
    fn session_ended_rows_offer_closing_first_then_each_profile() {
        let config = Config::parse(
            "[[profiles]]\nname = \"tmux\"\ntmux = \"control\"\n[[profiles]]\nname = \"Shell\"\n",
        )
        .unwrap();
        let rows = session_ended_rows(&config.profiles, &[]);
        let labels: Vec<&str> = rows.iter().map(|r| r.label.as_ref()).collect();
        assert_eq!(labels, ["Close window", "New tab: tmux", "New tab: Shell"]);
        assert!(rows[0].detail.is_none());
        assert_eq!(rows[1].detail.as_ref().map(|d| d.to_string()), Some(config.profiles[0].description().to_string()));
    }

    #[test]
    fn session_ended_rows_go_to_a_session_shown_elsewhere() {
        let config = Config::parse(
            "[[profiles]]\nname = \"tmux\"\ntmux = \"control\"\n[[profiles]]\nname = \"Shell\"\n",
        )
        .unwrap();
        let open = [SessionKey::for_profile(&config.profiles[0]).unwrap()];
        let rows = session_ended_rows(&config.profiles, &open);
        let labels: Vec<&str> = rows.iter().map(|r| r.label.as_ref()).collect();
        assert_eq!(labels, ["Close window", "Go to main", "New tab: Shell"]);
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
    fn session_open_rows_offer_going_there_first() {
        let config = open_rows_config();
        let open = [SessionKey::for_profile(&config.profiles[0]).unwrap()];
        let rows = session_open_rows(&config.profiles, 0, "main", &open, Target::Tab);
        let labels: Vec<&str> = rows.iter().map(|(r, _)| r.label.as_ref()).collect();
        assert_eq!(labels, ["Go to main", "New tmux session…", "New tab: Shell", "New tab: work", "New tab: classic"]);
        let choices: Vec<OpenChoice> = rows.into_iter().map(|(_, c)| c).collect();
        assert_eq!(
            choices,
            [OpenChoice::GoTo, OpenChoice::NewSession, OpenChoice::Profile(1), OpenChoice::Profile(2), OpenChoice::Profile(3)]
        );
    }

    #[test]
    fn session_open_rows_for_a_new_window_skip_open_sessions() {
        let config = open_rows_config();
        let open = [
            SessionKey::for_profile(&config.profiles[0]).unwrap(),
            SessionKey::for_profile(&config.profiles[2]).unwrap(),
        ];
        let rows = session_open_rows(&config.profiles, 0, "main", &open, Target::Window);
        let labels: Vec<&str> = rows.iter().map(|(r, _)| r.label.as_ref()).collect();
        assert_eq!(labels, ["Go to main", "New tmux session…", "New window: Shell", "New window: classic"]);
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
}
