//! A Brindle window: the tab strip (which doubles as the title bar), the
//! active tab's content, and the palettes.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    Action, AnyElement, App, AppContext as _, BorrowAppContext as _, Bounds, Context, CursorStyle, Decorations, Entity,
    FocusHandle, Focusable, Hsla, InteractiveElement, IntoElement, KeyDownEvent, Modifiers, MouseButton, MouseDownEvent,
    ParentElement, Pixels, Point,
    Render, ResizeEdge, SharedString, Size, StatefulInteractiveElement, Styled, Subscription,
    Task, Window, WindowBounds, WindowControlArea, canvas, div, point, px, rems,
};

use crate::actions::*;
use crate::config::{Config, Profile};
use crate::notify::osc::Source;
use crate::notify::{self, Notifier};
use crate::picker::{Listing, Palette, PaletteEvent, PaletteRow, PromptRequest, SessionRow, SessionsModel, SubmitFn};
use brindle_ui::{
    ActiveFont as _, ActiveTheme as _, Button, ButtonSize, ButtonVariant, ContextMenu, DropdownMenu, Icon, MenuEntry, MenuEvent, MenuItem,
    LEGIBLE_ICON_STROKE, Modality, Overlay as UiOverlay, TabActivate, TabContextMenu, TabList, TabMove, TabTrigger,
    TabWidth, TextInput, Tooltip, TooltipFor as _, button_style,
};
use crate::remote::Host;
use crate::remote::hosts::{HostState, Hosts};
use crate::remote::protocol::client::SessionInfo;
use crate::session::{self, RestoredTab, RestoredZoneNode, SessionStore, SnapshotTab, WindowState};
use crate::settings::{FontPreview, Settings};
use crate::settings_dialog::{self, SettingsDialog, SettingsEvent};
use crate::terminal::process::Snapshot;
use crate::terminal::status::State;
use crate::terminal::{Terminal, TerminalEvent};
use crate::terminal_view::{GridLayout, PointerStep, TerminalView};
use crate::theme::Theme;
use crate::zone_view::{ZONE_GAP, ZoneMenuRequest, ZoneTreeView};

/// The tab strip's height in logical pixels: the pill tab list's
/// (`TAB_LIST_HEIGHT`, shadcn's `h-9`, at the default 16px rem) plus the
/// `ZONE_GAP` margin above it, which matches the one between it and the
/// cards (and the one at its left). The strip's `IconSm` (2rem) buttons fit
/// inside the list's height. It has no border under it: the strip's
/// background runs on into the margin round the cards below.
pub(crate) const TAB_BAR_HEIGHT: f32 = brindle_ui::style::TAB_LIST_HEIGHT.0 * 16.0 + ZONE_GAP;
/// The room between two tab faces, in logical pixels.
const TAB_FACE_GAP: f32 = 4.0;
const RESIZE_BORDER: f32 = 6.0;

/// What a tab was opened as, for session restore.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabOrigin {
    /// A program in a PTY (a profile or a command override).
    Local,
    /// A message, such as an attach failure; never recorded.
    Message,
}

pub struct Tab {
    pub id: u64,
    /// The tab's profile, as the config has it now.
    pub profile: Profile,
    /// The endpoint the tab opened on: where its zones run, and where new
    /// ones start, whatever its profile targets now. It never changes.
    pub host: Host,
    /// The tab's zones: one terminal, or a split tree of them.
    pub zones: Entity<ZoneTreeView>,
    pub origin: TabOrigin,
    pub bell: bool,
    /// The most pressing program status (OSC 7501) across the tab's
    /// terminals; see [`Workspace::refresh_status`].
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
        host: Host,
        zones: Entity<ZoneTreeView>,
        origin: TabOrigin,
        subscriptions: Vec<Subscription>,
        cx: &mut App,
    ) -> Self {
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let mut tab = Self {
            id: NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            profile,
            host,
            zones,
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
        let zone_changed = self.zones.update(cx, |v, cx| v.refresh_titles(snapshot, cx));
        let mut displayed = std::mem::take(&mut self.title);
        let changed = refresh_displayed_title(&mut displayed, self.manual_title.as_deref(), || {
            self.automatic_title(snapshot, cx)
        });
        self.title = displayed;
        changed || zone_changed
    }

    fn automatic_title(&self, snapshot: &mut Option<Snapshot>, cx: &App) -> String {
        self.zones.read(cx).automatic_title(snapshot, cx)
    }

    fn set_manual_title(&mut self, text: &str, snapshot: &mut Option<Snapshot>, cx: &mut App) -> bool {
        self.manual_title = parse_manual_title(text);
        self.refresh_title(snapshot, cx)
    }

    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.zones.read(cx).focus_handle(cx)
    }

    fn theme(&self, cx: &App) -> Theme {
        self.zones.read(cx).theme.clone()
    }

    /// The colours of the tab's face in the strip, active or not: its
    /// terminal's background (the face's surface) and foreground (its
    /// text, icons and close button).
    fn face_colors(&self, cx: &App) -> (Hsla, Hsla) {
        let theme = &self.zones.read(cx).theme;
        (theme.background.hsla(), theme.foreground.hsla())
    }

    fn active_terminal(&self, cx: &App) -> Option<Entity<Terminal>> {
        self.zones.read(cx).active_terminal(cx)
    }

    /// The view of the terminal that has (or last had) focus in this tab.
    fn active_view(&self, cx: &App) -> Option<Entity<TerminalView>> {
        self.zones.read(cx).active_view()
    }

    /// Where the active terminal was last painted.
    fn active_grid(&self, cx: &App) -> Option<GridLayout> {
        self.active_view(cx)?.read(cx).layout.get()
    }

    /// The most pressing program status across the tab's terminals.
    fn status_summary(&self, cx: &App) -> Option<State> {
        self.zones.read(cx).status_summary(cx)
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

enum PaletteKind {
    NewTab,
    /// The command palette: its actions by row, and the tab that had focus
    /// when it opened, which they run in.
    Commands { actions: Vec<Box<dyn Action>>, tab_focus: FocusHandle },
    Prompt,
    /// The detached sessions of this machine's terminal server and of each
    /// remote endpoint's: the model that holds what each endpoint listed,
    /// the sessions killed from it and the highlighted row, and knows what
    /// each row stands for (enter attaches a session into a new tab,
    /// `picker_kill` ends it, both on its endpoint's connection).
    Sessions { model: SessionsModel },
}

/// How long the sessions palette waits for a remote endpoint's listing
/// before it counts the endpoint as offline.
#[cfg(any(target_os = "linux", windows))]
const LIST_LIMIT: std::time::Duration = std::time::Duration::from_secs(2);

/// Unix seconds now.
fn unix_now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// The endpoints the sessions palette lists, with how each stands: this
/// machine first, then the [`Hosts`] endpoints (`states`) in their order.
fn palette_endpoints(states: Vec<(Host, HostState)>) -> Vec<(Host, HostState)> {
    let mut endpoints = vec![(Host::Local, HostState::Connected)];
    endpoints.extend(states.into_iter().filter(|(host, _)| !host.is_local()));
    endpoints
}

/// What the sessions palette lists for `host`, `state` being how the
/// endpoint stands and `listed` what asking it gave (`Err`: why that
/// failed; `None`: it wasn't asked, which is how a connecting or offline
/// endpoint is). A connecting or offline endpoint says so; this machine's
/// failure is logged and lists nothing; any other endpoint that can't
/// answer is offline. Sessions attached to a client are left out.
fn listing_from(host: &Host, state: &HostState, listed: Option<Result<Vec<SessionInfo>, String>>) -> Listing {
    match (host, state) {
        (Host::Remote(_), HostState::Connecting) => return Listing::Connecting,
        (Host::Remote(_), HostState::Offline(why)) => return Listing::Offline(why.clone()),
        _ => {}
    }
    match listed.unwrap_or_else(|| Err("no listing was made".into())) {
        Ok(sessions) => Listing::Sessions(sessions.into_iter().filter(|s| !s.attached).collect()),
        Err(why) if host.is_local() => {
            log::warn!("listing sessions: {why}");
            Listing::Sessions(Vec::new())
        }
        Err(why) => Listing::Offline(why),
    }
}

/// Kills session `kill` (if given), then lists `host`'s detached sessions
/// for the sessions palette, `state` being how the endpoint stands. Blocks:
/// off the UI thread. This machine's server is listed through the shared
/// connection, as ever. A remote endpoint is listed only when connected,
/// on the connection the process has (none is made), within [`LIST_LIMIT`];
/// [`listing_from`] decides what that gives.
fn listing_of(host: &Host, state: &HostState, kill: Option<u32>) -> Listing {
    let listed = match (host, state) {
        (Host::Remote(_), HostState::Connecting | HostState::Offline(_)) => None,
        _ => Some(list_sessions(host, kill)),
    };
    listing_from(host, state, listed)
}

/// Kills session `kill` (if given) on `host`'s connection, then lists its
/// sessions.
fn list_sessions(host: &Host, kill: Option<u32>) -> Result<Vec<SessionInfo>, String> {
    #[cfg(any(target_os = "linux", windows))]
    {
        use crate::remote::local;

        let connection = match host {
            Host::Local => local::shared(host),
            Host::Remote(_) => local::existing(host).ok_or_else(|| anyhow::anyhow!("the connection ended")),
        };
        let limit = (!host.is_local()).then_some(LIST_LIMIT);
        connection
            .and_then(|connection| {
                if let Some(id) = kill
                    && let Err(why) = connection.kill_within(id, limit)
                {
                    log::warn!("killing session {id} on {}: {why:#}", host.name());
                }
                connection.list_within(limit)
            })
            .map_err(|why| format!("{why:#}"))
    }
    #[cfg(not(any(target_os = "linux", windows)))]
    {
        let _ = (host, kill);
        Ok(Vec::new())
    }
}


pub struct Workspace {
    tabs: Vec<Tab>,
    active: usize,
    /// The palettes' modal slot. At most one overlay, a palette, the
    /// settings dialog or the profile menu, is open at a time.
    palette: UiOverlay<Palette>,
    /// What the open palette is for, and the subscription to its events.
    palette_kind: Option<PaletteKind>,
    /// The sessions palette's latest round of listings: an older round's
    /// answers are dropped.
    sessions_listing: u64,
    /// The sessions palette that isn't open yet: it collects the endpoints'
    /// answers until all are in or `OPEN_GRACE` has passed.
    sessions_opening: Option<SessionsModel>,
    _palette_subscription: Option<Subscription>,
    /// The settings dialog's modal slot. A press on its backdrop does
    /// nothing, so a stray click never throws away a preview or an edit.
    settings: UiOverlay<SettingsDialog>,
    _settings_subscriptions: Vec<Subscription>,
    /// The strip's `⌄` button and its profile menu, the third overlay: at
    /// most one of the palette, the settings dialog and this menu is open.
    /// Its items are rebuilt each time it opens (`refresh_profile_menu`),
    /// so they follow config reloads and keymap changes.
    profile_menu: Entity<DropdownMenu>,
    _profile_menu_subscription: Subscription,
    /// The tab and zone header menus' slot, the fourth overlay (one of the
    /// four is open at most). It opens on the tab or zone it is for, which
    /// is active and focused by then, so its items run their actions there.
    context_menu: Entity<ContextMenu>,
    /// The open context menu's entries and what its items run.
    context_menu_items: Option<ActionMenu>,
    /// The tab (by id) whose menu is open in `context_menu`; `None` for a
    /// zone header's menu. Its items act on that tab, not the active one.
    context_menu_tab: Option<u64>,
    _context_menu_subscription: Subscription,
    /// Where the strip's tabs were laid out in the last frame, in window
    /// coordinates, for placing a tab menu opened from a key.
    tab_bounds: Rc<RefCell<Vec<Bounds<Pixels>>>>,
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
    last_title: SharedString,
    _title_refresh: Task<()>,
    /// This window's id and creation order, for session restore.
    window_id: u64,
    order: u64,
    /// The window's size as recorded for restore: the size it opened
    /// with, until the user resizes it (see [`recorded_size`]).
    windowed_size: std::cell::Cell<Size<Pixels>>,
    /// The remote endpoints of the restored tabs, each once, and the
    /// sessions restored zones recorded there: a launch's sweep of an
    /// endpoint leaves those to their zones ([`Workspace::attach_swept`]).
    restored_hosts: Vec<Host>,
    restored_sessions: Vec<(Host, session::ServerSession)>,
    /// Brindle is quitting, and the terminals have left their sessions
    /// ([`Workspace::quit`]): nothing is recorded any more, since the zones
    /// no longer know their sessions.
    quitting: bool,
}

impl Workspace {
    pub fn new(launch: Launch, window: &mut Window, cx: &mut Context<Self>) -> Self {
        static NEXT_ORDER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let focus_handle = cx.focus_handle();
        let profile_menu = cx.new(|_| DropdownMenu::new(profile_menu_trigger(focus_handle.clone()), Vec::<MenuEntry>::new()));
        // The menu has closed and given focus back to the tab by now.
        let profile_menu_subscription = cx.subscribe_in(&profile_menu, window, |this, _, event: &MenuEvent, window, cx| {
            let MenuEvent::Selected(ix) = *event;
            this.open_profile(ix, window, cx);
        });
        let context_menu = cx.new(|_| ContextMenu::new());
        let context_menu_subscription =
            cx.subscribe_in(&context_menu, window, |this, _, event: &MenuEvent, window, cx| {
                let MenuEvent::Selected(ix) = *event;
                this.context_menu_selected(ix, window, cx);
            });
        let mut this = Self {
            tabs: Vec::new(),
            active: 0,
            palette: UiOverlay::modal(),
            palette_kind: None,
            sessions_listing: 0,
            sessions_opening: None,
            _palette_subscription: None,
            settings: UiOverlay::modal().ignore_outside_press(),
            _settings_subscriptions: Vec::new(),
            profile_menu,
            _profile_menu_subscription: profile_menu_subscription,
            context_menu,
            context_menu_items: None,
            context_menu_tab: None,
            _context_menu_subscription: context_menu_subscription,
            tab_bounds: Rc::default(),
            theme_preview: None,
            profile_preview: None,
            focus_handle,
            strip_focus: cx.focus_handle(),
            last_title: SharedString::default(),
            _title_refresh: Task::ready(()),
            window_id: window.window_handle().window_id().as_u64(),
            order: NEXT_ORDER.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            windowed_size: std::cell::Cell::new(window.bounds().size),
            restored_hosts: Vec::new(),
            restored_sessions: Vec::new(),
            quitting: false,
        };
        // A close the window manager or Windows asks for (its close button,
        // alt-F4, the taskbar) quits too.
        let workspace = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            workspace.update(cx, |ws, cx| ws.quit(window, cx)).ok();
            true
        });
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
            Ok(info) => self.attach_detached_here(Host::Local, info, window, cx),
            Err(why) => {
                // A message tab: never recorded for session restore.
                let config = Self::settings(cx);
                let profile = config.profiles[config.default_profile_index()].clone();
                let text = format!("\x1b[31mBrindle could not attach session {session}:\x1b[0m\r\n{why}\r\n");
                self.open_message_tab(profile, &text, window, cx);
            }
        }
    }

    /// Opens a recorded window's tabs in order, and activates the recorded
    /// active tab. Zones that recorded a session get it back: this
    /// machine's from the launch's listing, a remote endpoint's once that
    /// endpoint answers (the window never waits for it).
    fn restore(&mut self, state: WindowState, window: &mut Window, cx: &mut Context<Self>) {
        let config = Self::settings(cx).clone();
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"));
        let mut active_tab = None;
        for (ix, entry) in state.tabs.iter().enumerate() {
            match session::resolve_tab(entry, &config, &home) {
                None => {}
                Some(RestoredTab::Local { profile, host, cwd, title, zones, session }) => {
                    if !host.is_local() && !self.restored_hosts.contains(&host) {
                        self.restored_hosts.push(host.clone());
                    }
                    match (zones, session) {
                        (Some(zones), _) => self.restore_zones(profile, host, zones, window, cx),
                        (None, Some(recorded)) => {
                            let profile = config.profiles[profile].clone();
                            let terminal = self.restored_terminal(&profile, &host, cwd, Some(&recorded), &config, cx);
                            self.insert_terminal_tab(profile, host, terminal, true, window, cx);
                        }
                        // No record: the tab follows its profile.
                        (None, None) => self.launch(LaunchRequest { profile, command: None, cwd: Some(cwd) }, window, cx),
                    }
                    let Some(tab) = self.tabs.get_mut(self.active) else { continue };
                    if let Some(title) = title {
                        tab.set_manual_title(&title, &mut None, cx);
                    }
                    if ix == state.active {
                        active_tab = Some(tab.id);
                    }
                }
            }
        }
        if let Some(ix) = active_tab.and_then(|id| self.tabs.iter().position(|t| t.id == id)) {
            self.activate(ix, window, cx);
        }
    }

    /// A restored split tab on `host`: the endpoint its zones recorded
    /// sessions on, else its profile's (`session::resolve_tab`).
    fn restore_zones(
        &mut self,
        profile_ix: usize,
        host: Host,
        zones: RestoredZoneNode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let config = Self::settings(cx).clone();
        let Some(profile) = config.profiles.get(profile_ix).cloned() else { return };
        let (tree, leaves, active_id) = zones.into_tree();
        let terminals: Vec<_> = leaves
            .into_iter()
            .map(|(id, cwd, session)| {
                let terminal = self.restored_terminal(&profile, &host, cwd, session.as_ref(), &config, cx);
                (id, terminal)
            })
            .collect();
        let font_size = profile.font_size;
        let theme = self.tab_theme(&profile, &config);
        let p_clone = profile.clone();
        let tree_host = host.clone();
        let view = cx.new(|cx| {
            ZoneTreeView::from_restored(p_clone, tree_host, theme, font_size, tree, terminals, active_id, window, cx)
        });
        let subscriptions = Self::zone_tab_subscriptions(&view, window, cx);
        let tab = Tab::new(profile, host, view, TabOrigin::Local, subscriptions, cx);
        self.insert_tab(tab, None, window, cx);
    }

    /// This window as session restore records it.
    pub fn snapshot(&self, window: &Window, cx: &App) -> WindowState {
        let mut tabs: Vec<SnapshotTab> = Vec::new();
        let mut active = None;
        for (ix, tab) in self.tabs.iter().enumerate() {
            if ix == self.active {
                active = Some(tabs.len());
            }
            tabs.push(match tab.origin {
                TabOrigin::Message => SnapshotTab::Skip,
                TabOrigin::Local => {
                    let z = tab.zones.read(cx);
                    // Zones that only show a message (a failed attach) aren't
                    // recorded, and neither is a tab of nothing else.
                    match session::record_zones(z.tree(), z.active_id(), &mut |id| z.zone_record(id, cx)) {
                        None => SnapshotTab::Skip,
                        Some(recorded) => SnapshotTab::Local {
                            profile: tab.profile.name.clone(),
                            // Where it runs, which its profile may no longer name.
                            remote: tab.host.target().cloned(),
                            cwd: recorded.cwd,
                            title: tab.manual_title.clone(),
                            zones: recorded.zones,
                            // A single zone's session is the tab's.
                            session: recorded.session,
                        },
                    }
                }
            });
        }
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
        if self.quitting || !cx.update_global(|store: &mut SessionStore, _| store.is_owner()) {
            return;
        }
        let state = self.snapshot(window, cx);
        let (id, order) = (self.window_id, self.order);
        cx.update_global(|store: &mut SessionStore, _| store.record(id, order, state));
    }

    /// Brindle is quitting, or this window is closing, which quits it:
    /// records the window while its zones still know their sessions, then
    /// has every tab's terminals leave their programs as their `Closing`
    /// says ([`Terminal::quit_with`]: profile tabs' sessions are detached, `-e`
    /// ones end). Their drops would do the same, but GPUI doesn't always
    /// run them before the process exits: on Windows the focused
    /// terminal's view outlives the exit, held by the input handler of a
    /// window whose destruction never runs. Once is enough.
    pub fn quit(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.quitting {
            return;
        }
        self.record_session(window, cx);
        self.quitting = true;
        // Each tab's kept sessions are detached with its layout.
        for (index, tab) in self.tabs.iter().enumerate() {
            let title = tab.manual_title.clone();
            tab.zones.update(cx, |zones, cx| zones.quit_all(index, title.as_deref(), cx));
        }
    }

    /// Keeps each local tab's layout current on its sessions' servers
    /// (`ZoneTreeView::send_layout`, which sends only changes): called once a
    /// second with the session recorder, so a change reaches the server
    /// within about a second, and a Brindle that crashes leaves its splits.
    pub fn send_layouts(&self, cx: &App) {
        if self.quitting {
            return;
        }
        for (index, tab) in self.tabs.iter().enumerate() {
            if tab.origin == TabOrigin::Local {
                tab.zones.read(cx).send_layout(index, tab.manual_title.as_deref(), cx);
            }
        }
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
    /// or container as the active tab ([`same_endpoint`]).
    fn active_cwd_for(&self, profile: &Profile, cx: &App) -> Option<PathBuf> {
        let tab = self.tabs.get(self.active)?;
        if !same_endpoint(&tab.host, profile) {
            return None;
        }
        tab.active_terminal(cx)?.read(cx).working_directory()
    }

    /// Opens a tab for the request.
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
        let command = request.command;

        let cwd = requested_cwd.or_else(|| {
            // New tabs follow the current directory only on the same endpoint,
            // unless the destination profile pins one.
            if profile.cwd.is_none() { self.active_cwd_for(&profile, cx) } else { None }
        });

        // The tab is on the profile's endpoint from now on.
        let host = Host::of(&profile);
        let terminal = cx.new(|cx| Terminal::spawn(&profile, &host, command, cwd, &config, cx));
        let font_size = profile.font_size;
        let theme = self.tab_theme(&profile, &config);
        let p_clone = profile.clone();
        let tree_host = host.clone();
        let view = cx.new(|cx| ZoneTreeView::new(p_clone, tree_host, theme, font_size, terminal, window, cx));
        let subscriptions = Self::zone_tab_subscriptions(&view, window, cx);
        let tab = Tab::new(profile, host, view, TabOrigin::Local, subscriptions, cx);
        self.insert_tab(tab, None, window, cx);
    }

    /// A tab on `host` showing `terminal` alone: inserted after the active
    /// tab and activated when `activate`, else at the end. Attached and
    /// restored sessions open this way: `host` is the one they run on.
    fn insert_terminal_tab(
        &mut self,
        profile: Profile,
        host: Host,
        terminal: Entity<Terminal>,
        activate: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> u64 {
        let config = Self::settings(cx).clone();
        let theme = self.tab_theme(&profile, &config);
        let font_size = profile.font_size;
        let p_clone = profile.clone();
        let tree_host = host.clone();
        let view = cx.new(|cx| ZoneTreeView::new(p_clone, tree_host, theme, font_size, terminal, window, cx));
        let subscriptions = Self::zone_tab_subscriptions(&view, window, cx);
        let tab = Tab::new(profile, host, view, TabOrigin::Local, subscriptions, cx);
        let id = tab.id;
        if activate {
            self.insert_tab(tab, None, window, cx);
        } else {
            let at = self.tabs.len();
            self.insert_tab_inactive(tab, Some(at), cx);
        }
        id
    }

    /// Opens a tab a sweep grouped from `host`'s detached sessions
    /// ([`crate::layout::group`]) at the end, with `profile`: one session as
    /// [`Workspace::insert_terminal_tab`] does, several in their recorded
    /// split with its active zone focused, either way with the recorded
    /// title. It is activated only when it is the window's first tab.
    #[cfg(any(target_os = "linux", windows))]
    fn insert_layout_tab(
        &mut self,
        profile: Profile,
        host: Host,
        tab: crate::layout::LayoutTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let config = Self::settings(cx).clone();
        let activate = self.tabs.is_empty();
        let title = tab.title.clone();
        let id = match tab.into_single() {
            Ok(info) => {
                let terminal = Self::attached_terminal(&host, &info, &profile, &config, cx);
                self.insert_terminal_tab(profile, host, terminal, activate, window, cx)
            }
            Err(tab) => {
                let (tree, leaves, active) = tab.into_tree();
                let terminals: Vec<_> = leaves
                    .into_iter()
                    .map(|(id, info)| (id, Self::attached_terminal(&host, &info, &profile, &config, cx)))
                    .collect();
                let theme = self.tab_theme(&profile, &config);
                let font_size = profile.font_size;
                let (p_clone, tree_host) = (profile.clone(), host.clone());
                let view = cx.new(|cx| {
                    ZoneTreeView::from_restored(p_clone, tree_host, theme, font_size, tree, terminals, active, window, cx)
                });
                let subscriptions = Self::zone_tab_subscriptions(&view, window, cx);
                let tab = Tab::new(profile, host, view, TabOrigin::Local, subscriptions, cx);
                let id = tab.id;
                if activate {
                    self.insert_tab(tab, None, window, cx);
                } else {
                    let at = self.tabs.len();
                    self.insert_tab_inactive(tab, Some(at), cx);
                }
                id
            }
        };
        if let Some(title) = title
            && let Some(tab) = self.tabs.iter_mut().find(|t| t.id == id)
        {
            tab.set_manual_title(&title, &mut None, cx);
        }
    }

    /// The detached session a restored zone recorded, when the launch's
    /// listing still has it ([`crate::reattach::Detached`]).
    fn take_detached(recorded: Option<&session::ServerSession>, cx: &mut App) -> Option<SessionInfo> {
        let recorded = recorded?;
        cx.try_global::<crate::reattach::Detached>()?;
        cx.update_global(|detached: &mut crate::reattach::Detached, _| detached.take(recorded))
    }

    /// A terminal attached to the detached session `info` of `host`'s
    /// server, closing as `profile` says.
    #[cfg(any(target_os = "linux", windows))]
    fn attached_terminal(
        host: &Host,
        info: &SessionInfo,
        profile: &Profile,
        config: &Config,
        cx: &mut Context<Self>,
    ) -> Entity<Terminal> {
        // At the session's own size, so attaching resizes nothing; a core
        // that predates sizes in `LIST` reports 0.
        let default = crate::terminal::GridSize::default();
        let (cols, rows) = if info.cols > 0 && info.rows > 0 { (info.cols, info.rows) } else { (default.cols, default.rows) };
        let request =
            crate::remote::local::AttachRequest { session: info.session, key: info.key.clone(), cols, rows, recorded: false };
        let what = crate::reattach::label(info);
        cx.new(|cx| Terminal::attach(host, request, what, profile, config, cx))
    }

    /// A restored zone's terminal on `host`. Its recorded session is
    /// attached: on this machine when the launch's listing found it still
    /// detached; on a remote endpoint once its listing there confirms it,
    /// which the zone waits for, saying it is connecting. Otherwise
    /// `profile` is spawned in `cwd`.
    fn restored_terminal(
        &mut self,
        profile: &Profile,
        host: &Host,
        cwd: PathBuf,
        recorded: Option<&session::ServerSession>,
        config: &Config,
        cx: &mut Context<Self>,
    ) -> Entity<Terminal> {
        #[cfg(any(target_os = "linux", windows))]
        match recorded {
            Some(recorded) if !host.is_local() => {
                self.restored_sessions.push((host.clone(), recorded.clone()));
                let request = crate::remote::local::AttachRequest {
                    session: recorded.id,
                    key: recorded.key.clone(),
                    cols: 0,
                    rows: 0,
                    recorded: true,
                };
                let what = host.name();
                return cx.new(|cx| Terminal::attach(host, request, what, profile, config, cx));
            }
            Some(recorded) => {
                if let Some(info) = Self::take_detached(Some(recorded), cx) {
                    return Self::attached_terminal(host, &info, profile, config, cx);
                }
            }
            None => {}
        }
        #[cfg(not(any(target_os = "linux", windows)))]
        let _ = recorded;
        cx.new(|cx| Terminal::spawn(profile, host, None, Some(cwd), config, cx))
    }

    /// Opens `sessions` (detached sessions of this machine nothing restored
    /// took) as tabs after the others: those detached with one tab's layout
    /// together in its split, the rest one per tab, in the order
    /// [`crate::layout::group`] gives. Each tab takes the profile its active
    /// session was started with, or the default one when that is gone or
    /// remote. The active tab stays the same.
    pub fn attach_detached(&mut self, sessions: Vec<SessionInfo>, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(any(target_os = "linux", windows))]
        {
            let config = Self::settings(cx).clone();
            for tab in crate::layout::group(sessions) {
                let profile = config.profiles[crate::reattach::local_profile(tab.active(), &config)].clone();
                // The launch listed this machine's sessions.
                self.insert_layout_tab(profile, Host::Local, tab, window, cx);
            }
        }
        #[cfg(not(any(target_os = "linux", windows)))]
        let _ = (sessions, window, cx);
    }

    /// The remote endpoints of the restored tabs, each once.
    pub fn restored_hosts(&self) -> &[Host] {
        &self.restored_hosts
    }

    /// Opens a tab at the end for each session `host`'s first listing of the
    /// launch has that the launch attaches ([`crate::reattach::swept_tabs`]):
    /// not restored into a zone, not shown here already, and with no owner
    /// that may come back. Each takes the profile
    /// [`crate::reattach::session_profile`] gives it, on `host` whatever that
    /// profile targets. The active tab stays the same, and the replays raise
    /// no bells or notifications.
    #[cfg(any(target_os = "linux", windows))]
    pub fn attach_swept(&mut self, host: Host, listed: Vec<SessionInfo>, window: &mut Window, cx: &mut Context<Self>) {
        let config = Self::settings(cx).clone();
        let mut skip: Vec<session::ServerSession> =
            self.restored_sessions.iter().filter(|(on, _)| *on == host).map(|(_, s)| s.clone()).collect();
        skip.extend(self.sessions_on(&host, cx));
        let first = self.profile_on(&host);
        let tabs = crate::reattach::swept_tabs(listed, &skip, &host, &config, first.as_deref());
        if tabs.is_empty() {
            return;
        }
        log::info!("attaching {} detached session tab(s) on {}", tabs.len(), host.name());
        for (tab, index) in tabs {
            let profile = config.profiles[index].clone();
            self.insert_layout_tab(profile, host.clone(), tab, window, cx);
        }
    }

    /// The name of the profile of the first tab on `host`, the one lookup
    /// [`crate::reattach::session_profile`] needs from the window.
    #[cfg(any(target_os = "linux", windows))]
    fn profile_on(&self, host: &Host) -> Option<String> {
        self.tabs.iter().find(|tab| tab.host == *host).map(|tab| tab.profile.name.clone())
    }

    /// The sessions this window's tabs on `host` show (or are attaching).
    #[cfg(any(target_os = "linux", windows))]
    fn sessions_on(&self, host: &Host, cx: &App) -> Vec<session::ServerSession> {
        self.tabs
            .iter()
            .filter(|tab| tab.host == *host)
            .flat_map(|tab| {
                let zones = tab.zones.read(cx);
                zones.tree().leaves().into_iter().filter_map(|id| zones.zone_server_session(id, cx)).collect::<Vec<_>>()
            })
            .collect()
    }

    /// Inserts `tab` at `index` (default: after the active tab) and activates it.
    fn insert_tab(&mut self, tab: Tab, index: Option<usize>, window: &mut Window, cx: &mut Context<Self>) {
        let index = self.insert_tab_inactive(tab, index, cx);
        self.activate(index, window, cx);
    }

    /// Inserts `tab` at `index` without activating it; the active tab stays
    /// the same.
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

    /// Opens profile `ix` in a new tab.
    fn open_profile(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.launch(LaunchRequest::profile(ix), window, cx);
    }

    fn zone_tab_subscriptions(
        view: &Entity<ZoneTreeView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<Subscription> {
        vec![
            cx.subscribe_in(view, window, |this, view, event, window, cx| this.on_zone_tree_event(view, event, window, cx)),
            cx.subscribe_in(view, window, |this, view, request: &ZoneMenuRequest, window, cx| {
                this.open_zone_menu(view, request, window, cx)
            }),
        ]
    }

    /// A zone asked for its header menu (it has focus by now). The menu
    /// opens only in the active tab, whose zones are the ones shown.
    fn open_zone_menu(
        &mut self,
        view: &Entity<ZoneTreeView>,
        request: &ZoneMenuRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let shown = self.tabs.get(self.active).is_some_and(|t| &t.zones == view);
        if !shown {
            return;
        }
        let menu = ActionMenu::new(request.rows.clone(), |action| tab_shortcut(action, cx));
        self.open_context_menu(menu, request.at, request.press.as_ref(), window, cx);
    }

    fn on_zone_tree_event(
        &mut self,
        view: &Entity<ZoneTreeView>,
        event: &TerminalEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(ix) = self.tabs.iter().position(|t| &t.zones == view) else {
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
    /// `ix`: marks the tab (the bell dot, the status dot),
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
        if !self.has_overlay(cx) {
            self.focus_active_tab(window, cx);
        }
    }

    /// Shows tab `index` (clamped) without moving focus: its bell clears.
    /// False with no tabs.
    fn select(&mut self, index: usize, cx: &mut Context<Self>) -> bool {
        if self.tabs.is_empty() {
            return false;
        }
        self.active = index.min(self.tabs.len() - 1);
        self.tabs[self.active].bell = false;
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
        if self.has_overlay(cx) || self.tabs.is_empty() {
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

    /// Removes tab `ix`, ending its terminals' programs. When it was the
    /// last tab on a remote endpoint that no profile connects to at launch,
    /// the launch stops waiting for that endpoint's first listing
    /// ([`crate::remote::local::stop_listing`]), so it doesn't sweep it.
    fn remove_tab(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        if ix >= self.tabs.len() {
            return;
        }
        #[cfg(any(target_os = "linux", windows))]
        let before: Vec<Host> = self.tabs.iter().map(|tab| tab.host.clone()).collect();
        let tab = self.tabs.remove(ix);
        tab.zones.update(cx, |v, cx| v.shutdown_all(cx));
        #[cfg(any(target_os = "linux", windows))]
        {
            let after: Vec<Host> = self.tabs.iter().map(|tab| tab.host.clone()).collect();
            let eager = crate::reattach::eager_hosts(Self::settings(cx));
            for host in crate::reattach::endpoints_without_tabs(&before, &after, &eager) {
                crate::remote::local::stop_listing(&host);
            }
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
        let host = Host::of(&profile);
        let tree_host = host.clone();
        let view = cx.new(|cx| ZoneTreeView::new(p_clone, tree_host, theme, profile.font_size, terminal, window, cx));
        let tab = Tab::new(profile, host, view, TabOrigin::Message, Vec::new(), cx);
        self.insert_tab(tab, None, window, cx);
    }

    fn new_tab(&mut self, _: &NewTab, window: &mut Window, cx: &mut Context<Self>) {
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

    /// Moves tab `ix` one place (`delta` -1 or 1), without wrapping, as the
    /// tab menu does; the active tab stays active wherever it ends up.
    fn move_tab_at(&mut self, ix: usize, delta: isize, cx: &mut Context<Self>) {
        let Some(to) = ix.checked_add_signed(delta) else { return };
        self.move_tab_to(ix, to, cx);
    }

    fn rename_tab(&mut self, _: &RenameTab, window: &mut Window, cx: &mut Context<Self>) {
        self.rename_tab_at(self.active, window, cx);
    }

    /// Asks for a new title for tab `ix`, which it gets whatever moves meanwhile.
    fn rename_tab_at(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(ix) else { return };
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
        });
        let label = "Rename tab".into();
        self.open_prompt(PromptRequest { label, initial, submit }, window, cx);
    }

    /// A tab dragged onto another: the tab at `from` takes `to`'s place
    /// (after it when it came from the left, before it from the right), and
    /// the active tab stays active wherever it ends up.
    fn move_tab_to(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        if let Some(active) = reorder_keeping(&mut self.tabs, self.active, from, to, |t| t.id) {
            self.active = active;
            cx.notify();
        }
    }

    /// Closing the window quits Brindle: its terminals' programs keep running.
    fn close_window(&mut self, _: &CloseWindow, window: &mut Window, cx: &mut Context<Self>) {
        self.quit(window, cx);
        window.remove_window();
    }

    fn open_profile_picker(&mut self, _: &OpenProfilePicker, window: &mut Window, cx: &mut Context<Self>) {
        if self.close_palette_toggling(|k| matches!(k, PaletteKind::NewTab), window, cx) {
            return;
        }
        let rows = self.profile_rows(window, cx);
        let palette = cx.new(|cx| Palette::list(rows, "Open profile…", cx));
        self.show_palette(palette, PaletteKind::NewTab, window, cx);
    }

    /// The profiles as the picker and the profile menu list them
    /// ([`profile_rows`]), with the keys that open each in the active tab.
    fn profile_rows(&self, window: &Window, cx: &App) -> Vec<PaletteRow> {
        let focus = self.tabs.get(self.active).map_or_else(|| self.focus_handle.clone(), |t| t.focus_handle(cx));
        profile_rows(&Self::settings(cx).profiles, |ix| shortcut(&NewTabWithProfile(ix), &focus, window))
    }

    /// Gives the profile menu the picker's rows as they are now, for its
    /// next opening.
    fn refresh_profile_menu(&mut self, window: &Window, cx: &mut Context<Self>) {
        let entries = profile_menu_entries(self.profile_rows(window, cx));
        self.profile_menu.update(cx, |menu, _| menu.set_entries(entries));
    }

    /// `OpenProfileMenu`: opens the `⌄` button's menu below it, as a press
    /// on the button does, or closes it when it is open. Another overlay
    /// closes first, so the menu gives focus back to the tab.
    fn open_profile_menu(&mut self, _: &OpenProfileMenu, window: &mut Window, cx: &mut Context<Self>) {
        let was_open = self.profile_menu.read(cx).is_open();
        if self.has_overlay(cx) {
            self.dismiss_overlay(window, cx);
        }
        if was_open {
            return;
        }
        self.refresh_profile_menu(window, cx);
        self.profile_menu.update(cx, |menu, cx| menu.open(window, cx));
        cx.notify();
    }

    /// `OpenTabMenu`: the active tab's menu, below its face in the strip
    /// (below the strip's start before the strip was first drawn).
    fn open_tab_menu(&mut self, _: &OpenTabMenu, window: &mut Window, cx: &mut Context<Self>) {
        let face = self.tab_bounds.borrow().get(self.active).copied();
        let at = face.map_or_else(|| point(px(0.), px(TAB_BAR_HEIGHT)), |face| face.bottom_left());
        self.open_tab_menu_at(self.active, at, None, window, cx);
    }

    /// Tab `ix`'s menu, at `at` (window coordinates), from the secondary
    /// `press` on its face when one asked. The tab isn't activated: focus
    /// stays with the active tab, and the items act on tab `ix` by its id
    /// (`context_menu_selected`).
    fn open_tab_menu_at(
        &mut self,
        ix: usize,
        at: Point<Pixels>,
        press: Option<&MouseDownEvent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if ix >= self.tabs.len() {
            return;
        }
        let menu = ActionMenu::new(tab_menu_rows(ix, self.tabs.len()), |action| tab_shortcut(action, cx));
        self.open_context_menu(menu, at, press, window, cx);
        self.context_menu_tab = Some(self.tabs[ix].id);
    }

    /// Opens `menu` in the context menu slot at `at`, or at the pointer of
    /// `press`, replacing any overlay. Focus goes to the active tab first:
    /// the menu gives it back there on closing, and a chosen item's action
    /// runs from there.
    fn open_context_menu(
        &mut self,
        menu: ActionMenu,
        at: Point<Pixels>,
        press: Option<&MouseDownEvent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_overlay(window, cx);
        self.focus_active_tab(window, cx);
        let entries = menu.entries.clone();
        self.context_menu_items = Some(menu);
        self.context_menu.update(cx, |context_menu, cx| match press {
            Some(event) => context_menu.open_on_press(event, entries, window, cx),
            None => context_menu.open(at, entries, window, cx),
        });
        cx.notify();
    }

    /// An item of the tab or zone menu was chosen. The menu has closed and
    /// given focus back to the active tab. A tab menu's item acts on the
    /// menu's tab (gone by now: nothing); a zone menu's action runs from the
    /// focused zone, exactly as its keybinding would (as the command palette
    /// runs one).
    fn context_menu_selected(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let tab = self.context_menu_tab.take();
        let Some(action) = self.context_menu_items.take().and_then(|menu| menu.action(ix).map(|a| a.boxed_clone())) else {
            return;
        };
        self.focus_active_tab(window, cx);
        cx.notify();
        if let Some(item) = tab.and_then(|_| TabMenuItem::of(&*action)) {
            let Some(ix) = tab.and_then(|id| self.tabs.iter().position(|t| t.id == id)) else { return };
            match item {
                TabMenuItem::Rename => self.rename_tab_at(ix, window, cx),
                TabMenuItem::MoveLeft => self.move_tab_at(ix, -1, cx),
                TabMenuItem::MoveRight => self.move_tab_at(ix, 1, cx),
                TabMenuItem::Close => self.close_tab_at(ix, window, cx),
            }
            return;
        }
        window.dispatch_action(action, cx);
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
                disabled: false,
            })
            .collect();
        let actions = commands.into_iter().map(|(_, a)| a).collect();
        let palette = cx.new(|cx| Palette::list(rows, "Run a command…", cx));
        self.show_palette(palette, PaletteKind::Commands { actions, tab_focus }, window, cx);
    }

    /// Asks for a line of text, replacing any overlay.
    fn open_prompt(&mut self, request: PromptRequest, window: &mut Window, cx: &mut Context<Self>) {
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
        if !self.has_overlay(cx) {
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
        self.palette_kind = Some(kind);
        self._palette_subscription = Some(subscription);
        self.palette.open(palette, |this| &mut this.palette, window, cx);
        cx.notify();
    }

    /// Escape (or a click outside) on a palette.
    fn palette_dismissed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.dismiss_overlay(window, cx);
    }

    fn palette_confirmed(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(kind) = self.palette_kind.take() else { return };
        match kind {
            PaletteKind::NewTab => {
                self.dismiss_overlay(window, cx);
                self.open_profile(ix, window, cx);
            }
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
            PaletteKind::Sessions { model } => {
                let target =
                    model.rows().get(ix).and_then(SessionRow::session).map(|(host, info)| (host.clone(), info.clone()));
                match target {
                    Some((host, info)) => {
                        self.dismiss_overlay(window, cx);
                        self.attach_detached_here(host, info, window, cx);
                    }
                    // A row that does nothing (the command list confirms
                    // none of them anyway).
                    None => self.palette_kind = Some(PaletteKind::Sessions { model }),
                }
            }
        }
    }

    /// "Attach Session…": lists the detached sessions off the UI thread,
    /// then shows them.
    fn attach_session(&mut self, _: &AttachSession, window: &mut Window, cx: &mut Context<Self>) {
        if self.close_palette_toggling(|k| matches!(k, PaletteKind::Sessions { .. }), window, cx) {
            return;
        }
        self.refresh_sessions_palette(None, true, window, cx);
    }

    /// The remote endpoints' states changed (the [`Hosts`] global): an open
    /// sessions palette lists them again.
    pub fn hosts_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(self.palette_kind, Some(PaletteKind::Sessions { .. })) {
            self.refresh_sessions_palette(None, false, window, cx);
        }
    }

    /// Starts a round of listings: kills `kill` first, if given (an endpoint
    /// and a session id), then lists the detached sessions of this machine
    /// and of each remote endpoint the [`Hosts`] global holds
    /// ([`palette_endpoints`]), each on a background thread. Each endpoint's
    /// answer is applied as it arrives ([`Workspace::sessions_answered`]),
    /// and only if it is of the latest round.
    ///
    /// With `opening`, the sessions palette doesn't exist yet. It opens
    /// when every endpoint has answered, or after `OPEN_GRACE`
    /// ([`crate::picker::OPEN_GRACE`], about 250 ms), whichever comes first:
    /// the usual palette opens complete, and a slow endpoint doesn't keep it
    /// closed, but is listed as `listing sessions…` until it answers. Without
    /// `opening`, the answers go to the open sessions palette's model, which
    /// keeps the highlighted row where it was; a palette closed meanwhile
    /// stays closed.
    fn refresh_sessions_palette(&mut self, kill: Option<(Host, u32)>, opening: bool, window: &mut Window, cx: &mut Context<Self>) {
        let endpoints = palette_endpoints(cx.try_global::<Hosts>().map(Hosts::states).unwrap_or_default());
        self.sessions_listing += 1;
        let round = self.sessions_listing;
        let hosts = endpoints.iter().map(|(host, _)| host.clone());
        if opening {
            self.sessions_opening = Some(SessionsModel::new(round, hosts));
        } else if let Some(PaletteKind::Sessions { model }) = &mut self.palette_kind {
            model.start_round(round, hosts);
        } else {
            return;
        }
        for (host, state) in endpoints {
            let kill = kill.as_ref().filter(|(on, _)| *on == host).map(|(_, id)| *id);
            let listing = {
                let host = host.clone();
                cx.background_executor().spawn(async move { listing_of(&host, &state, kill) })
            };
            cx.spawn_in(window, async move |this, cx| {
                let listed = listing.await;
                this.update_in(cx, |this, window, cx| this.sessions_answered(round, host, listed, window, cx)).ok();
            })
            .detach();
        }
        if opening {
            cx.spawn_in(window, async move |this, cx| {
                cx.background_executor().timer(crate::picker::OPEN_GRACE).await;
                this.update_in(cx, |this, window, cx| {
                    if this.sessions_opening.as_ref().is_some_and(|model| model.round() == round) {
                        this.open_sessions_palette(window, cx);
                    }
                })
                .ok();
            })
            .detach();
        }
    }

    /// `host`'s listing in `round` has arrived: into the open sessions
    /// palette, which shows it at once, or into the one still opening, which
    /// opens if that was the last answer it waited for.
    fn sessions_answered(&mut self, round: u64, host: Host, listed: Listing, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(PaletteKind::Sessions { model }) = &mut self.palette_kind {
            if model.answer(round, &host, listed) {
                self.show_sessions_palette(cx);
            }
        } else if let Some(model) = &mut self.sessions_opening
            && model.answer(round, &host, listed)
            && model.complete()
        {
            self.open_sessions_palette(window, cx);
        }
    }

    /// Opens the sessions palette with what the endpoints have listed so far.
    fn open_sessions_palette(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut model) = self.sessions_opening.take() else { return };
        let view = model.view(Self::settings(cx), unix_now());
        let palette = cx.new(|cx| Palette::list_grouped(view.groups, "Attach a detached session", view.empty, cx));
        self.show_palette(palette, PaletteKind::Sessions { model }, window, cx);
    }

    /// Shows the open sessions palette's model in place: its filter text
    /// stays, and so does the highlighted row (by identity, as the user left
    /// it in the list).
    fn show_sessions_palette(&mut self, cx: &mut Context<Self>) {
        let Some(PaletteKind::Sessions { model }) = &mut self.palette_kind else { return };
        let Some(palette) = self.palette.view().cloned() else { return };
        model.highlight(palette.read(cx).highlighted(cx));
        let view = model.view(Self::settings(cx), unix_now());
        palette.update(cx, |palette, cx| palette.set_groups(view.groups, view.empty, view.keep, cx));
        cx.notify();
    }

    /// `picker_kill` in the sessions palette: ends that session on its
    /// endpoint, and shows the list again.
    fn palette_kill(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(PaletteKind::Sessions { model }) = &mut self.palette_kind else {
            return;
        };
        let Some((host, info)) = model.rows().get(ix).and_then(SessionRow::session) else { return };
        let (host, id, key) = (host.clone(), info.session, info.key.clone());
        model.kill(host.clone(), id, key);
        // Its row goes at once, so enter can't attach it meanwhile, and the
        // highlight passes to the row that takes its place.
        self.show_sessions_palette(cx);
        self.refresh_sessions_palette(Some((host, id)), false, window, cx);
    }

    /// Attaches a detached session of `host` into a new tab after the
    /// active one, on `host`, with the profile
    /// [`crate::reattach::session_profile`] gives it.
    fn attach_detached_here(&mut self, host: Host, info: SessionInfo, window: &mut Window, cx: &mut Context<Self>) {
        #[cfg(any(target_os = "linux", windows))]
        {
            let config = Self::settings(cx).clone();
            let first = self.profile_on(&host);
            let index = crate::reattach::session_profile(&info, &host, &config, first.as_deref());
            let profile = config.profiles[index].clone();
            let terminal = Self::attached_terminal(&host, &info, &profile, &config, cx);
            self.insert_terminal_tab(profile, host, terminal, true, window, cx);
        }
        #[cfg(not(any(target_os = "linux", windows)))]
        let _ = (host, info, window, cx);
    }

    fn open_settings(&mut self, _: &OpenSettings, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings.is_open() {
            self.dismiss_overlay(window, cx);
            return;
        }
        let settings = Settings::get(cx);
        let config = &settings.config;
        let model = settings_dialog::Model::new(
            Config::path(),
            config.theme.clone(),
            config.themes.clone(),
            config.profiles.clone(),
            config.default_profile.clone(),
        )
        .with_fonts((config.font.clone(), config.ui_font.clone()), settings.installed_fonts().to_vec());
        let theme = config.theme(None);
        let dialog = cx.new(|cx| SettingsDialog::new(model, theme, window, cx));
        let subscription = cx.subscribe_in(&dialog, window, |this, _, event, _, cx| match event {
            SettingsEvent::Preview(theme) => this.set_theme_preview(Some(theme.clone()), cx),
            // Deferred by GPUI, so the reload sees this window idle.
            SettingsEvent::ProfilePreview(preview) => this.set_profile_preview(preview.clone(), cx),
            SettingsEvent::FontPreview(preview) => this.set_font_preview(preview.clone(), cx),
            // Deferred, so the reload sees this window idle (as ReloadConfig).
            SettingsEvent::ConfigWritten(renames) => {
                let renames = renames.clone();
                cx.defer(move |cx| crate::reload_config_renaming(cx, &renames));
            }
        });
        // The slot closes itself on the dialog's `DismissEvent`; this ends
        // the previews, as closing it any other way does.
        let dismissed = cx.subscribe_in(&dialog, window, |this, _, _: &gpui::DismissEvent, _, cx| this.end_previews(cx));
        // A palette or the profile menu goes first, with focus back on the
        // tab, so that is the focus the dialog records and puts back when it
        // closes (the palette's input is gone by then).
        if self.has_overlay(cx) {
            self.dismiss_overlay(window, cx);
        }
        self._settings_subscriptions = vec![subscription, dismissed];
        self.settings.open(dialog, |this| &mut this.settings, window, cx);
        cx.notify();
    }

    /// Whether a palette, the settings dialog, the profile menu or a tab or
    /// zone menu is open.
    fn has_overlay(&self, cx: &App) -> bool {
        self.palette.is_open()
            || self.settings.is_open()
            || self.profile_menu.read(cx).is_open()
            || self.context_menu.read(cx).is_open()
    }

    /// The open menu: the profile menu, or a tab or zone menu.
    fn open_menu(&self, cx: &App) -> Option<Entity<brindle_ui::Menu>> {
        self.profile_menu.read(cx).menu().or_else(|| self.context_menu.read(cx).menu()).cloned()
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
        // A sessions palette still collecting answers isn't opened over
        // whatever replaces this overlay.
        self.sessions_opening = None;
        self._palette_subscription = None;
        self.settings.close(window, cx);
        self._settings_subscriptions.clear();
        self.profile_menu.update(cx, |menu, cx| menu.close(window, cx));
        self.context_menu.update(cx, |menu, cx| menu.close(window, cx));
        self.context_menu_tab = None;
        self.end_previews(cx);
    }

    /// Stops the settings dialog's theme, profile and font previews.
    fn end_previews(&mut self, cx: &mut Context<Self>) {
        if self.theme_preview.is_some() {
            self.set_theme_preview(None, cx);
        }
        if self.profile_preview.is_some() {
            self.set_profile_preview(None, cx);
        }
        if Settings::get(cx).font_preview().is_some() {
            self.set_font_preview(None, cx);
        }
    }

    /// Shows the settings dialog's fonts in place of the config's (or, with
    /// `None`, the config's again): the terminal font's family, size and
    /// line height on every tab, whose grids re-fit, and the UI font on the
    /// chrome. A config reload replaces `Settings`, which ends it too.
    pub fn set_font_preview(&mut self, preview: Option<FontPreview>, cx: &mut Context<Self>) {
        cx.update_global::<Settings, _>(|settings, _| settings.set_font_preview(preview));
        Settings::apply_ui_font(cx);
        cx.refresh_windows();
        cx.notify();
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
    /// `None`, puts their configured ones back.
    pub fn set_profile_preview(&mut self, preview: Option<(String, Profile)>, cx: &mut Context<Self>) {
        self.profile_preview = preview;
        self.apply_tab_themes(cx);
        cx.notify();
    }

    /// Shows `preview` on the tabs that follow the global theme, or, with
    /// `None`, puts their configured theme back.
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
        // A tab whose profile now targets another endpoint stays on its
        // own host (`Tab::host`): its splits start there.
        for tab in &mut self.tabs {
            if let Some(profile) = reloaded_profile(&tab.profile, &config) {
                tab.profile = profile;
            }
        }
        let mut snapshot = None;
        for tab in &mut self.tabs {
            tab.refresh_title(&mut snapshot, cx);
        }
        self.apply_tab_themes(cx);
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
            out.push_str(&describe_fonts(cx));
            let view = self.tabs.get(self.active).and_then(|t| t.active_view(cx));
            if let Some(link) = view.and_then(|v| v.read(cx).hovered_link.clone()) {
                out.push_str(&format!("--- link {}\n", link.uri));
            }
            if let Some(text) = terminal.selection_text() {
                out.push_str(&format!("--- selection {text:?}\n"));
            }
            out.push_str(&terminal.status().describe());
            if let Some(tab) = self.tabs.get(self.active) {
                out.push_str(&tab.zones.read(cx).describe_zones());
            }
            out.push_str(&terminal.screen_text());
        } else {
            out.push_str(&describe_fonts(cx));
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
        if let Some(menu) = self.open_menu(cx) {
            let menu = menu.read(cx);
            out.push_str(&describe_menu(menu.entries(), menu.highlighted()));
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
    /// `NAME`), `tabs` (the tab strip, on the active tab), `menu` (an open
    /// menu: the profile menu, or a tab or zone menu), `none`, or
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
        } else if self.open_menu(cx).is_some_and(|m| m.read(cx).focus_handle(cx).contains_focused(window, cx)) {
            "menu".to_string()
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

    /// The tab strip, which is also the window's title bar: a `TabList`
    /// (shadcn's pill), the new-tab and profile buttons, the caption (empty
    /// space) and, with client decorations, the window controls. Nothing in
    /// it acts on the active tab: split and find are on their keys, the tab
    /// menu and the palette.
    ///
    /// It is drawn in the window's chrome tokens (`cx.ui_theme()`, from the
    /// global theme), whatever the tabs' profile themes; only each tab's
    /// face, active or not, takes its terminal's background and foreground
    /// (its icons and close button too), the active one raised. Its buttons
    /// are `IconSm` with 16px icons at a legible stroke. The strip is one
    /// keyboard stop (`strip_focus`), reached through `FocusTabStrip`: the
    /// list takes the arrows, home and end, and the wrapper around it enter
    /// and escape. A press on a tab activates it and leaves focus in its
    /// terminal.
    fn render_tab_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tokens = *cx.ui_theme();
        let decorations = window.window_decorations();
        // Windows always shows Brindle's own caption (see `window_control_area`
        // below); GPUI reports server decorations there.
        let csd = matches!(decorations, Decorations::Client { .. }) || cfg!(windows);

        // Status dots take the global theme's ANSI colours (the previewed
        // theme's while the settings dialog shows one), not the tab's;
        // working is the muted foreground.
        let global = Settings::chrome_theme(cx).clone();
        let status_color = |state: State| global.status_color(state, tokens.muted_foreground);
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
            .map(|tab| {
                let (surface, text) = tab.face_colors(cx);
                let mut trigger = TabTrigger::new(("tab", tab.id as usize), tab.title.clone())
                    .closable(true)
                    .close_tooltip(close_tip.clone())
                    .surface(surface)
                    .text_color(text);
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
            .active(self.active)
            .children(triggers)
            .focus(&self.strip_focus)
            .trigger_width(TabWidth::new(rems(5.), rems(15.)))
            .on_activate(cx.listener(|this, request: &TabActivate, window, cx| this.activate_from_strip(request, window, cx)))
            .on_close(cx.listener(|this, ix: &usize, window, cx| this.close_tab_at(*ix, window, cx)))
            .on_move(cx.listener(|this, request: &TabMove, _, cx| this.move_tab_to(request.from, request.to, cx)))
            // A right-click on a tab opens its menu at the pointer; the press
            // goes no further, so the strip's window menu doesn't open too.
            // (On Windows a tab isn't a caption area: only the empty
            // `caption` element is, so the system menu doesn't open either.)
            .on_context_menu(cx.listener(|this, request: &TabContextMenu, window, cx| {
                this.open_tab_menu_at(request.index, request.event.position, Some(&request.event), window, cx)
            }))
            .on_trigger_bounds({
                let tab_bounds = self.tab_bounds.clone();
                move |bounds, _, _| {
                    let mut recorded = tab_bounds.borrow_mut();
                    recorded.clear();
                    recorded.extend_from_slice(bounds);
                }
            })
            .flex_shrink()
            .min_w_0()
            .overflow_hidden()
            // No pill behind the faces, and a little room between them; the
            // list's left padding goes too, so the first face lines up with
            // the card below. (Documented in `ui/README.md`.)
            .bg(gpui::transparent_black())
            .gap(px(TAB_FACE_GAP))
            .pl_0();

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
        let new_tab = strip_button("new-tab", Icon::Plus, tokens.muted_foreground)
            .tooltip_with(tip("New tab", Box::new(NewTab)))
            .on_click(cx.listener(|this, _, window, cx| this.new_tab(&NewTab, window, cx)));
        // `⌄` is the profile menu's trigger: the dropdown opens on the
        // press. The capture phase runs before the dropdown's own handler,
        // so the items it opens with are the picker's rows as they are now.
        // While the menu is open its cover takes the press, which closes it.
        let profiles = inert("profiles-press", self.profile_menu.clone().into_any_element()).capture_any_mouse_down(
            cx.listener(|this, event: &gpui::MouseDownEvent, window, cx| {
                if event.button == MouseButton::Left {
                    this.refresh_profile_menu(window, cx);
                }
            }),
        );

        // The caption buttons stay hand-built faces, in the strip buttons'
        // ghost `IconSm` size and hover colours, drawing icons at the same
        // legible stroke rather than text glyphs (which fall back to
        // whatever font the platform picks). On Windows the marked areas are
        // native caption buttons: Windows acts on them (and shows Snap
        // Layouts on maximize), but only on a press Brindle leaves
        // unhandled, so they get no handlers there.
        let caption = button_style(ButtonVariant::Ghost, ButtonSize::IconSm, &tokens);
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
                .child(icon.size(caption.icon_size).min_stroke(LEGIBLE_ICON_STROKE))
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
            .pt(px(ZONE_GAP))
            .pl(px(ZONE_GAP))
            .gap_0p5()
            .bg(tokens.background)
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
                    .h_full()
                    .gap_0p5()
                    .pl_1()
                    .child(inert("new-tab-press", new_tab.into_any_element()))
                    .child(profiles),
            )
            // Windows hit-tests this as the caption: drag, double-click,
            // the system menu and Snap. Only empty space is marked, since a
            // marked area wins over everything inside it.
            .child(div().id("caption").flex_1().h_full().when(cfg!(windows), |d| d.window_control_area(WindowControlArea::Drag)))
            .children(window_controls.map(|c| div().flex().h_full().items_center().child(c)))
    }

    fn render_content(&self) -> AnyElement {
        match self.tabs.get(self.active) {
            Some(tab) => tab.zones.clone().into_any_element(),
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

/// [`reorder`], following the item at `active` by its id: its new index, or
/// `None` when nothing moved.
fn reorder_keeping<T>(items: &mut Vec<T>, active: usize, from: usize, to: usize, id: impl Fn(&T) -> u64) -> Option<usize> {
    let active_id = id(items.get(active)?);
    if !reorder(items, from, to) {
        return None;
    }
    Some(items.iter().position(|item| id(item) == active_id).unwrap_or(0))
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

/// Whether a directory from a terminal on `host` means anything to a new
/// tab of `profile`: only on the same endpoint.
fn same_endpoint(host: &Host, profile: &Profile) -> bool {
    *host == Host::of(profile)
}

/// A tab's profile once the config is reloaded: the config's profile of
/// that name, whatever it targets now; `None` when the config has none,
/// and the tab keeps its own.
fn reloaded_profile(current: &Profile, config: &Config) -> Option<Profile> {
    config.profile(&current.name).cloned()
}

/// Gives a tab's terminals `theme`, keeping its font size.
fn set_tab_theme(tab: &Tab, theme: Theme, font_size: Option<f32>, cx: &mut App) {
    let profile = tab.profile.clone();
    tab.zones.update(cx, |view, cx| view.set_profile(profile, theme, font_size, cx))
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

/// The profiles as the profile picker and the profile menu list them, in
/// configuration order: each one's name, what it runs and the keys bound to
/// opening it (`shortcut(ix)`, the keymap's ctrl-alt-N for the first nine).
fn profile_rows(profiles: &[Profile], shortcut: impl Fn(usize) -> Option<SharedString>) -> Vec<PaletteRow> {
    profiles
        .iter()
        .enumerate()
        .map(|(ix, profile)| PaletteRow {
            label: profile.name.clone().into(),
            detail: Some(profile.description().into()),
            shortcut: shortcut(ix),
            disabled: false,
        })
        .collect()
}

/// The profile menu's items: the picker's rows, the description under the
/// name and the keys at the right. There are no separators, so an item's
/// index is its profile's.
fn profile_menu_entries(rows: Vec<PaletteRow>) -> Vec<MenuEntry> {
    rows.into_iter()
        .map(|row| MenuItem { label: row.label, description: row.detail, shortcut: row.shortcut, disabled: false }.into())
        .collect()
}

/// The menu of tab `ix` of `count`: Rename Tab, Move Tab Left (disabled on
/// the first tab), Move Tab Right (disabled on the last), a separator and
/// Close Tab.
/// What a tab menu item does to the menu's tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum TabMenuItem {
    Rename,
    MoveLeft,
    MoveRight,
    Close,
}

impl TabMenuItem {
    /// The item for one of [`tab_menu_rows`]' actions; `None` for any other.
    fn of(action: &dyn Action) -> Option<Self> {
        let any = action.as_any();
        if any.is::<RenameTab>() {
            Some(Self::Rename)
        } else if any.is::<MoveTabLeft>() {
            Some(Self::MoveLeft)
        } else if any.is::<MoveTabRight>() {
            Some(Self::MoveRight)
        } else if any.is::<CloseTab>() {
            Some(Self::Close)
        } else {
            None
        }
    }
}

fn tab_menu_rows(ix: usize, count: usize) -> Vec<ActionRow> {
    vec![
        ActionRow::item(RenameTab, true),
        ActionRow::item(MoveTabLeft, ix > 0),
        ActionRow::item(MoveTabRight, ix + 1 < count),
        ActionRow::Separator,
        ActionRow::item(CloseTab, true),
    ]
}

/// One of the strip's buttons (new tab, the profile chevron): a ghost
/// `IconSm` button in `color` whose 16px icon is stroked at least
/// [`LEGIBLE_ICON_STROKE`] wide (the spec's "Strip icons are legible").
fn strip_button(id: &'static str, icon: Icon, color: Hsla) -> Button {
    Button::icon_only(id, icon)
        .variant(ButtonVariant::Ghost)
        .size(ButtonSize::IconSm)
        .icon_min_stroke(LEGIBLE_ICON_STROKE)
        .text_color(color)
}

/// The keys that run `action` in a terminal of a tab, as the keymap
/// resolves them in that terminal's contexts (`Workspace`, then `ZoneTree`,
/// then `Terminal`, where user bindings go). Unlike [`shortcut`] it needs
/// no drawn frame, so a background tab that a right-click just activated
/// shows its keys.
fn tab_shortcut(action: &dyn Action, cx: &App) -> Option<SharedString> {
    let keymap = cx.key_bindings();
    let keymap = keymap.borrow();
    tab_keys(&keymap, action)
}

/// [`tab_shortcut`] in `keymap`.
fn tab_keys(keymap: &gpui::Keymap, action: &dyn Action) -> Option<SharedString> {
    let contexts: Vec<gpui::KeyContext> = ["Workspace", "ZoneTree", "Terminal"]
        .into_iter()
        .filter_map(|name| gpui::KeyContext::parse(name).ok())
        .collect();
    keymap
        .bindings_for_action(action)
        .rev()
        .find(|binding| {
            // The binding counts only when its keys reach it there, rather
            // than another binding of the same keys.
            let (found, _) = keymap.bindings_for_input(binding.keystrokes(), &contexts);
            found.first().is_some_and(|first| first.action().partial_eq(binding.action()))
        })
        .map(keys_text)
}

/// The strip's `⌄` button, as the profile menu's trigger builds it each
/// frame: a strip button with no `on_click` (the dropdown takes the press)
/// and no focus handle (a press moves focus nowhere), whose tooltip names
/// the picker's key, looked up in `focus` when it is shown.
fn profile_menu_trigger(focus: FocusHandle) -> impl Fn(bool, &mut Window, &mut App) -> Button + 'static {
    move |_open, _, cx| {
        let focus = focus.clone();
        strip_button("profiles", Icon::ChevronDown, cx.ui_theme().muted_foreground)
            .tooltip_with(move |window, _| {
                Tooltip::new("New tab from profile…").shortcut_opt(shortcut(&OpenProfilePicker, &focus, window))
            })
    }
}

/// The fonts in effect, preview included, for `--dump-screen-after`:
/// `--- fonts terminal="…" ui="…"`, the families GPUI is asked for.
fn describe_fonts(cx: &App) -> String {
    let settings = Settings::get(cx);
    format!("--- fonts terminal={:?} ui={:?}\n", settings.font().family.to_string(), settings.ui_font_family().to_string())
}

/// An open menu, for `--dump-screen-after`: `--- menu items=A|[B]|-|C
/// highlighted=N`, with disabled items in brackets and separators as `-`;
/// `N` is the highlighted entry's index from 0, separators counted (as
/// `MenuEvent::Selected` counts), or `none`. Then each item on a line of
/// its own as the palette prints its rows: `label  description  [keys]`.
fn describe_menu(entries: &[MenuEntry], highlighted: Option<usize>) -> String {
    let items: Vec<String> = entries
        .iter()
        .map(|entry| match entry.item() {
            None => "-".to_string(),
            Some(item) if item.disabled => format!("[{}]", item.label),
            Some(item) => item.label.to_string(),
        })
        .collect();
    let highlighted = highlighted.map_or_else(|| "none".to_string(), |ix| ix.to_string());
    let mut out = format!("--- menu items={} highlighted={highlighted}\n", items.join("|"));
    for item in entries.iter().filter_map(MenuEntry::item) {
        out.push_str(&item.label);
        if let Some(description) = &item.description {
            out.push_str(&format!("  {description}"));
        }
        if let Some(keys) = &item.shortcut {
            out.push_str(&format!("  [{keys}]"));
        }
        out.push('\n');
    }
    out
}

/// The keys that run `action` in the element `focus`, as the palettes show them.
pub(crate) fn shortcut(action: &dyn Action, focus: &FocusHandle, window: &Window) -> Option<SharedString> {
    let binding = window.highest_precedence_binding_for_action_in(action, focus)?;
    Some(keys_text(&binding))
}

/// A binding's keystrokes as the keymap prints them (`ctrl-alt-1`), with a
/// space between strokes.
fn keys_text(binding: &gpui::KeyBinding) -> SharedString {
    binding.keystrokes().iter().map(|k| k.unparse()).collect::<Vec<_>>().join(" ").into()
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
            .on_action(cx.listener(Self::open_profile_menu))
            .on_action(cx.listener(Self::open_command_palette))
            .on_action(cx.listener(Self::open_settings))
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
                    .on_action(cx.listener(Self::open_tab_menu))
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
            .font_family(cx.ui_font())
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
            // (one of the two is open at most; the profile menu is drawn
            // deferred from the strip, and only while neither is open).
            .children(self.palette.render(window, cx))
            .children(self.settings.render(window, cx))
            // The tab and zone menus: drawn deferred, taking no room here.
            .child(self.context_menu.clone());

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

    fn ssh_host(host: &str) -> Host {
        Host::Remote(crate::config::RemoteTarget::Ssh {
            host: host.into(),
            user: None,
            port: None,
            identity_file: None,
            ssh_args: Vec::new(),
            connect_at_launch: false,
        })
    }

    /// 5.3: the sessions palette lists this machine first, then the
    /// `Hosts` endpoints in their order, each with its state.
    #[test]
    fn palette_endpoints_list_this_machine_first_then_the_hosts_in_order() {
        assert_eq!(palette_endpoints(Vec::new()), [(Host::Local, HostState::Connected)]);
        let states = vec![
            (ssh_host("B9"), HostState::Connecting),
            (ssh_host("A8"), HostState::Offline("refused".into())),
            (ssh_host("C7"), HostState::Connected),
        ];
        assert_eq!(
            palette_endpoints(states.clone()),
            [
                (Host::Local, HostState::Connected),
                (ssh_host("B9"), HostState::Connecting),
                (ssh_host("A8"), HostState::Offline("refused".into())),
                (ssh_host("C7"), HostState::Connected),
            ]
        );
        // This machine is never listed twice.
        let mut again = states;
        again.insert(0, (Host::Local, HostState::Connected));
        assert_eq!(palette_endpoints(again).iter().filter(|(host, _)| host.is_local()).count(), 1);
    }

    /// 5.3: what the palette lists for an endpoint, from its state and what
    /// asking it gave, without the I/O.
    #[test]
    fn listing_from_decides_what_the_palette_lists() {
        let (a8, session) = (ssh_host("A8"), |id, attached| SessionInfo { session: id, attached, ..Default::default() });
        let sessions = || Ok(vec![session(1, false), session(2, true), session(3, false)]);
        let kept = |listing: Listing| match listing {
            Listing::Sessions(sessions) => sessions.iter().map(|s| s.session).collect::<Vec<_>>(),
            other => panic!("{other:?}"),
        };

        // Attached sessions are left out, on this machine and elsewhere.
        assert_eq!(kept(listing_from(&Host::Local, &HostState::Connected, Some(sessions()))), [1, 3]);
        assert_eq!(kept(listing_from(&a8, &HostState::Connected, Some(sessions()))), [1, 3]);

        // A connecting or offline endpoint says so, whatever was asked.
        assert_eq!(listing_from(&a8, &HostState::Connecting, None), Listing::Connecting);
        assert_eq!(listing_from(&a8, &HostState::Connecting, Some(sessions())), Listing::Connecting);
        assert_eq!(
            listing_from(&a8, &HostState::Offline("refused".into()), None),
            Listing::Offline("refused".into())
        );

        // A failure to list: this machine lists nothing, any other endpoint is offline.
        assert_eq!(listing_from(&Host::Local, &HostState::Connected, Some(Err("boom".into()))), Listing::Sessions(Vec::new()));
        assert_eq!(
            listing_from(&a8, &HostState::Connected, Some(Err("the connection ended".into()))),
            Listing::Offline("the connection ended".into())
        );
        assert!(matches!(listing_from(&a8, &HostState::Connected, None), Listing::Offline(_)), "connected but never asked");
    }

    /// 6.2: a tab stays on the host it was made on. Once its profile moves
    /// to another endpoint, a reload gives the tab the moved profile, but a
    /// split (the profile on the tab's host) still starts on the old host,
    /// and the tab's directories mean nothing to a new tab of the profile.
    #[cfg(any(target_os = "linux", windows))]
    #[test]
    fn a_tab_keeps_its_host_when_its_profile_moves() {
        use crate::config::RemoteTarget;

        let ssh = |host: &str| RemoteTarget::Ssh {
            host: host.into(),
            user: None,
            port: None,
            identity_file: None,
            ssh_args: Vec::new(),
            connect_at_launch: false,
        };
        let before = Profile {
            name: "logs".into(),
            command: Some("htop".into()),
            remote: Some(ssh("old-host")),
            ..Default::default()
        };
        let tab_host = Host::of(&before);
        let moved = Profile { remote: Some(ssh("new-host")), ..before.clone() };
        let mut config = Config::default();
        config.profiles = vec![moved.clone()];

        let reloaded = reloaded_profile(&before, &config).expect("the profile is still there");
        assert_eq!(reloaded.remote, moved.remote);
        let split = |profile: &Profile, host: &Host| {
            crate::terminal::spawn_request(profile, host, None, Some("/var/log/brindle-nowhere".into()), &config)
        };
        let (request, doing) = split(&reloaded, &tab_host);
        assert_eq!(doing, "start \"htop\" on old-host");
        assert_eq!(request.cwd.as_deref(), Some("/var/log/brindle-nowhere"), "the old host's directory, unchecked");
        assert!(same_endpoint(&tab_host, &before));
        assert!(!same_endpoint(&tab_host, &reloaded));

        // A local tab whose profile now names a remote still splits here.
        let (request, doing) = split(&reloaded, &Host::Local);
        assert_eq!(doing, "start \"htop\"");
        assert_eq!(request.cwd, None, "no such directory on this machine");
        assert!(!same_endpoint(&Host::Local, &reloaded));

        // A profile the config dropped leaves the tab its own.
        assert!(reloaded_profile(&before, &Config::default()).is_none());
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

    /// The keys `shortcut` shows for `action` in a local tab's terminal with
    /// the default keymap: the last binding that matches there and isn't
    /// shadowed, as gpui's `highest_precedence_binding_for_action_in` finds it.
    fn default_keys(keymap: &gpui::Keymap, action: &dyn Action) -> Option<SharedString> {
        let contexts: Vec<gpui::KeyContext> =
            ["Workspace", "ZoneTree", "Terminal"].iter().map(|c| gpui::KeyContext::parse(c).unwrap()).collect();
        keymap
            .bindings_for_action(action)
            .rev()
            .find(|binding| {
                let (found, _) = keymap.bindings_for_input(binding.keystrokes(), &contexts);
                found.first().is_some_and(|f| f.action().partial_eq(binding.action()))
            })
            .map(keys_text)
    }

    fn text(s: &Option<SharedString>) -> Option<&str> {
        s.as_ref().map(|s| s.as_ref())
    }

    /// The picker's rows and the menu's items: names in configuration
    /// order, what each runs and ctrl-alt-N from the keymap.
    #[test]
    fn profile_rows_and_menu_items_match_the_picker() {
        let config = Config::parse(
            "[[profiles]]\nname = \"Shell\"\ncommand = \"bash\"\n\
             [[profiles]]\nname = \"htop\"\ncommand = \"htop\"\n[[profiles]]\nname = \"work\"\n",
        )
        .unwrap();
        let keymap = gpui::Keymap::new(crate::actions::default_bindings());
        let build = || profile_rows(&config.profiles, |ix| default_keys(&keymap, &NewTabWithProfile(ix)));
        let rows = build();
        let labels: Vec<&str> = rows.iter().map(|r| r.label.as_ref()).collect();
        assert_eq!(labels, ["Shell", "htop", "work"]);
        assert_eq!(text(&rows[0].detail), Some(config.profiles[0].description().as_str()));
        assert_eq!(text(&rows[1].detail), Some("htop"));
        assert_eq!(text(&rows[2].detail), Some("default shell"));
        let keys: Vec<Option<&str>> = rows.iter().map(|r| text(&r.shortcut)).collect();
        assert_eq!(keys, [Some("ctrl-alt-1"), Some("ctrl-alt-2"), Some("ctrl-alt-3")]);

        // One item per row, in the same order, with nothing between them, so
        // `MenuEvent::Selected(ix)` is profile `ix`.
        let entries = profile_menu_entries(build());
        assert_eq!(entries.len(), rows.len());
        for (entry, row) in entries.iter().zip(&rows) {
            let item = entry.item().expect("a separator in the profile menu");
            assert_eq!(item.label, row.label);
            assert_eq!(item.description, row.detail);
            assert_eq!(item.shortcut, row.shortcut);
            assert!(!item.disabled);
        }
    }

    /// ctrl-alt-1 … ctrl-alt-9 are bound to the first nine profiles; a tenth
    /// and eleventh are listed with no keys.
    #[test]
    fn only_the_first_nine_profiles_have_keys() {
        let toml: String = (1..=11).map(|n| format!("[[profiles]]\nname = \"p{n}\"\ncommand = \"bash\"\n")).collect();
        let config = Config::parse(&toml).unwrap();
        let keymap = gpui::Keymap::new(crate::actions::default_bindings());
        let entries = profile_menu_entries(profile_rows(&config.profiles, |ix| {
            default_keys(&keymap, &NewTabWithProfile(ix))
        }));
        assert_eq!(entries.len(), 11);
        for (ix, entry) in entries.iter().enumerate() {
            let item = entry.item().unwrap();
            assert_eq!(item.label.as_ref(), format!("p{}", ix + 1));
            let want = (ix < 9).then(|| format!("ctrl-alt-{}", ix + 1));
            assert_eq!(text(&item.shortcut), want.as_deref(), "profile {}", ix + 1);
        }
    }

    #[test]
    fn menus_dump_their_items_and_highlight() {
        let entries = vec![
            MenuItem::new("Shell").description("bash").shortcut("ctrl-alt-1").into(),
            MenuItem::new("Off").disabled(true).into(),
            MenuEntry::Separator,
            MenuItem::new("htop").into(),
        ];
        assert_eq!(
            describe_menu(&entries, None),
            "--- menu items=Shell|[Off]|-|htop highlighted=none\nShell  bash  [ctrl-alt-1]\nOff\nhtop\n"
        );
        assert!(describe_menu(&entries, Some(3)).starts_with("--- menu items=Shell|[Off]|-|htop highlighted=3\n"));
    }

    /// The first line `--- menu` prints for the menu of tab `ix` of `count`,
    /// with the default keys.
    fn tab_menu_line(ix: usize, count: usize) -> String {
        let keymap = gpui::Keymap::new(default_bindings());
        let menu = ActionMenu::new(tab_menu_rows(ix, count), |action| tab_keys(&keymap, action));
        describe_menu(&menu.entries, None)
    }

    /// Each tab menu row's action maps to the item that acts on the menu's
    /// tab; a zone menu's action to none, so it is dispatched.
    #[test]
    fn tab_menu_items_act_on_the_menus_tab() {
        let items: Vec<_> = tab_menu_rows(1, 3)
            .iter()
            .filter_map(|row| match row {
                ActionRow::Item { action, .. } => Some(TabMenuItem::of(&**action)),
                ActionRow::Separator => None,
            })
            .collect();
        use TabMenuItem::*;
        assert_eq!(items, [Some(Rename), Some(MoveLeft), Some(MoveRight), Some(Close)]);
        assert_eq!(TabMenuItem::of(&crate::actions::SplitZoneRight), None);
    }

    /// Moving a background tab from its menu keeps the active tab active,
    /// and moving the active one moves it with it; nothing moves past an end.
    #[test]
    fn a_menu_move_keeps_the_active_tab() {
        let moved = |active: usize, from: usize, to: usize| {
            let mut ids = vec![10u64, 11, 12];
            let active = reorder_keeping(&mut ids, active, from, to, |id| *id);
            (ids, active)
        };
        // The third tab moves left while the first is active.
        assert_eq!(moved(0, 2, 1), (vec![10, 12, 11], Some(0)));
        // The second moves left past the active first one.
        assert_eq!(moved(0, 1, 0), (vec![11, 10, 12], Some(1)));
        // The active tab itself moves right.
        assert_eq!(moved(1, 1, 2), (vec![10, 12, 11], Some(2)));
        // Past the end: nothing.
        assert_eq!(moved(0, 2, 3), (vec![10, 11, 12], None));
    }

    /// The spec's tab menu: Move Tab Left disabled on the first tab, Move
    /// Tab Right on the last (both on a lone tab), and each item's default
    /// key. The menu of the third of four tabs has both moves enabled.
    #[test]
    fn tab_menu_items() {
        assert_eq!(
            tab_menu_line(0, 3),
            "--- menu items=Rename Tab|[Move Tab Left]|Move Tab Right|-|Close Tab highlighted=none\n\
             Rename Tab\nMove Tab Left  [ctrl-shift-pageup]\nMove Tab Right  [ctrl-shift-pagedown]\n\
             Close Tab  [ctrl-shift-w]\n"
        );
        assert!(tab_menu_line(2, 4).starts_with("--- menu items=Rename Tab|Move Tab Left|Move Tab Right|-|Close Tab "));
        assert!(tab_menu_line(2, 3).starts_with("--- menu items=Rename Tab|Move Tab Left|[Move Tab Right]|-|Close Tab "));
        assert!(tab_menu_line(0, 1).starts_with("--- menu items=Rename Tab|[Move Tab Left]|[Move Tab Right]|-|Close Tab "));
        let menu = ActionMenu::new(tab_menu_rows(0, 2), |_| None);
        assert!(menu.action(0).is_some_and(|a| a.partial_eq(&RenameTab)));
        assert!(menu.action(1).is_none(), "Move Tab Left on the first tab runs nothing");
        assert!(menu.action(4).is_some_and(|a| a.partial_eq(&CloseTab)));
    }

    /// A tab menu's keys come from the keymap for the tab's own contexts,
    /// so a binding in the `Terminal` context (where user bindings go) shows,
    /// a binding only another context has doesn't, and a key that a later
    /// binding takes for something else isn't shown for the earlier one.
    #[test]
    fn tab_menu_keys_follow_the_tab_contexts() {
        let mut bindings = default_bindings();
        bindings.push(gpui::KeyBinding::new("f2", RenameTab, Some("Terminal")));
        bindings.push(gpui::KeyBinding::new("f3", MoveTabRight, Some("SettingsDialog")));
        bindings.push(gpui::KeyBinding::new("ctrl-shift-w", NewTab, Some("Terminal")));
        let keymap = gpui::Keymap::new(bindings);
        assert_eq!(tab_keys(&keymap, &RenameTab), Some("f2".into()));
        assert_eq!(tab_keys(&keymap, &MoveTabRight), Some("ctrl-shift-pagedown".into()));
        assert_eq!(tab_keys(&keymap, &CloseTab), None);
        assert_eq!(tab_keys(&keymap, &MoveTabLeft), Some("ctrl-shift-pageup".into()));
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

        // Manual title trims input and overrides OSC, foreground and profile changes.
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
        assert!(!refresh_displayed_title(&mut displayed, manual.as_deref(), || "other-title".into()));
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
