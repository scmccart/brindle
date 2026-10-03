//! A Brindle window: the tab strip (which doubles as the title bar), the
//! active tab's content, and the profile picker.

use std::path::PathBuf;

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, AppContext as _, BorrowAppContext as _, Context, CursorStyle, Decorations, Entity, FocusHandle,
    Focusable, Hsla, InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels, Point,
    Render, ResizeEdge, SharedString, Size, StatefulInteractiveElement, Styled, Subscription,
    Window, canvas, div, point, px,
};

use crate::actions::*;
use crate::config::{Config, Profile, TmuxMode};
use crate::picker::{PickerEvent, ProfilePicker};
use crate::settings::Settings;
use crate::terminal::{Terminal, TerminalEvent};
use crate::terminal_view::TerminalView;
use crate::theme::Theme;
use crate::tmux::{TmuxEvent, TmuxSession};
use crate::tmux_view::TmuxWindowView;

const TAB_BAR_HEIGHT: f32 = 38.0;
const RESIZE_BORDER: f32 = 6.0;

pub enum TabContent {
    Terminal(Entity<TerminalView>),
    Tmux { session: Entity<TmuxSession>, view: Entity<TmuxWindowView> },
}

pub struct Tab {
    pub id: u64,
    pub profile: Profile,
    pub content: TabContent,
    pub bell: bool,
    _subscriptions: Vec<Subscription>,
}

impl Tab {
    fn title(&self, cx: &App) -> String {
        match &self.content {
            TabContent::Terminal(view) => {
                let terminal = view.read(cx).terminal.read(cx);
                if let Some(title) = terminal.title().filter(|t| !t.trim().is_empty()) {
                    return title.to_string();
                }
                terminal
                    .foreground_process_name()
                    .unwrap_or_else(|| self.profile.name.clone())
            }
            TabContent::Tmux { view, .. } => view.read(cx).title(cx),
        }
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
    next_tab_id: u64,
    picker: Option<(Entity<ProfilePicker>, Subscription)>,
    focus_handle: FocusHandle,
    /// tmux control-mode sessions attached in this window.
    tmux_sessions: Vec<(Entity<TmuxSession>, Subscription)>,
    last_title: String,
}

impl Workspace {
    pub fn new(launch: LaunchRequest, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut this = Self {
            tabs: Vec::new(),
            active: 0,
            next_tab_id: 1,
            picker: None,
            focus_handle: cx.focus_handle(),
            tmux_sessions: Vec::new(),
            last_title: String::new(),
        };
        this.launch(launch, window, cx);
        this
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
                    profile.tmux_session.clone().unwrap_or_else(|| "main".into()),
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
        let id = self.next_id();
        let subscriptions = vec![cx.subscribe_in(&terminal, window, move |this, _, event, window, cx| {
            this.on_terminal_event(id, event, window, cx)
        })];
        let tab = Tab { id, profile, content: TabContent::Terminal(view), bell: false, _subscriptions: subscriptions };
        let index = if self.tabs.is_empty() { 0 } else { self.active + 1 };
        self.tabs.insert(index, tab);
        self.activate(index, window, cx);
    }

    fn attach_tmux(&mut self, profile: Profile, cwd: Option<PathBuf>, window: &mut Window, cx: &mut Context<Self>) {
        let config = Self::settings(cx).clone();
        let session_name = profile.tmux_session.clone().unwrap_or_else(|| "main".into());
        if let Some((session, _)) = self
            .tmux_sessions
            .iter()
            .find(|(s, _)| s.read(cx).profile.name == profile.name && s.read(cx).session_name == session_name)
        {
            // Already attached: just focus its first tab.
            let session = session.clone();
            if let Some(ix) = self.tabs.iter().position(|t| matches!(&t.content, TabContent::Tmux { session: s, .. } if *s == session)) {
                self.activate(ix, window, cx);
            }
            return;
        }
        let cwd = cwd.or_else(|| self.active_cwd(cx));
        let session = cx.new(|cx| TmuxSession::attach(profile.clone(), cwd, &config, cx));
        let subscription = cx.subscribe_in(&session, window, |this, session, event, window, cx| {
            this.on_tmux_event(session.clone(), event, window, cx)
        });
        self.tmux_sessions.push((session, subscription));
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
                if self.tabs.iter().any(|t| matches!(&t.content, TabContent::Tmux { session: s, view } if *s == session && view.read(cx).window_id == window_id)) {
                    return;
                }
                let profile = session.read(cx).profile.clone();
                let theme = Self::settings(cx).theme(profile.theme.as_deref());
                let view = cx.new(|cx| TmuxWindowView::new(session.clone(), window_id, theme, profile.font_size, window, cx));
                let id = self.next_id();
                let subscriptions = vec![cx.observe(&view, |_, _, cx| cx.notify())];
                let tab = Tab {
                    id,
                    profile,
                    content: TabContent::Tmux { session: session.clone(), view },
                    bell: false,
                    _subscriptions: subscriptions,
                };
                // Keep a session's tabs together, in tmux's window order.
                let index = self
                    .tabs
                    .iter()
                    .rposition(|t| matches!(&t.content, TabContent::Tmux { session: s, .. } if *s == session))
                    .map(|i| i + 1)
                    .unwrap_or(if self.tabs.is_empty() { 0 } else { self.active + 1 });
                self.tabs.insert(index, tab);
                self.activate(index, window, cx);
            }
            TmuxEvent::WindowClosed(window_id) => {
                let window_id = *window_id;
                if let Some(ix) = self.tabs.iter().position(|t| matches!(&t.content, TabContent::Tmux { session: s, view } if *s == session && view.read(cx).window_id == window_id)) {
                    self.remove_tab(ix, window, cx);
                }
            }
            TmuxEvent::WindowActivated(window_id) => {
                let window_id = *window_id;
                if let Some(ix) = self.tabs.iter().position(|t| matches!(&t.content, TabContent::Tmux { session: s, view } if *s == session && view.read(cx).window_id == window_id)) {
                    if ix != self.active {
                        self.activate(ix, window, cx);
                    }
                }
            }
            TmuxEvent::Detached(reason) => {
                log::info!("tmux session detached: {reason}");
                self.tmux_sessions.retain(|(s, _)| *s != session);
                let had_tabs = self
                    .tabs
                    .iter()
                    .any(|t| matches!(&t.content, TabContent::Tmux { session: s, .. } if *s == session));
                if !had_tabs {
                    // It never got as far as a window: say why in a tab.
                    let profile = session.read(cx).profile.clone();
                    let text = format!(
                        "\x1b[1mtmux ({}) ended before attaching:\x1b[0m {reason}\r\n\r\n\
                         Is tmux installed and on PATH? Profile settings: tmux_session, tmux_args, command.\r\n",
                        profile.name
                    );
                    self.open_message_tab(profile, &text, window, cx);
                    return;
                }
                let mut ix = 0;
                while ix < self.tabs.len() {
                    if matches!(&self.tabs[ix].content, TabContent::Tmux { session: s, .. } if *s == session) {
                        self.remove_tab(ix, window, cx);
                    } else {
                        ix += 1;
                    }
                }
                if self.tabs.is_empty() {
                    // Nothing else to show: fall back to a plain shell so the
                    // window doesn't vanish when tmux exits.
                    let profile = Self::settings(cx).default_profile_index();
                    let profile = if Self::settings(cx).profiles[profile].tmux == TmuxMode::Control {
                        Self::settings(cx).profiles.iter().position(|p| p.tmux == TmuxMode::None)
                    } else {
                        Some(profile)
                    };
                    match profile {
                        Some(profile) => self.launch(LaunchRequest { profile, command: None, cwd: None }, window, cx),
                        None => window.remove_window(),
                    }
                }
            }
        }
    }

    fn next_id(&mut self) -> u64 {
        let id = self.next_tab_id;
        self.next_tab_id += 1;
        id
    }

    fn on_terminal_event(&mut self, tab_id: u64, event: &TerminalEvent, window: &mut Window, cx: &mut Context<Self>) {
        let Some(ix) = self.tabs.iter().position(|t| t.id == tab_id) else { return };
        match event {
            TerminalEvent::Exited => self.remove_tab(ix, window, cx),
            TerminalEvent::Bell => {
                if ix != self.active || !window.is_window_active() {
                    self.tabs[ix].bell = true;
                }
                cx.notify();
            }
            TerminalEvent::TitleChanged => cx.notify(),
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
        if self.picker.is_none() {
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
        let theme = config.theme(profile.theme.as_deref());
        let terminal = cx.new(|cx| {
            let mut t = Terminal::remote(crate::terminal::GridSize::default(), theme, &config, std::rc::Rc::new(|_| {}), cx);
            t.feed(text.as_bytes(), cx);
            t
        });
        let view = cx.new(|cx| TerminalView::new(terminal, profile.font_size, window, cx));
        let id = self.next_id();
        let tab = Tab { id, profile, content: TabContent::Terminal(view), bell: false, _subscriptions: Vec::new() };
        let index = if self.tabs.is_empty() { 0 } else { self.active + 1 };
        self.tabs.insert(index, tab);
        self.activate(index, window, cx);
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
        self.launch(LaunchRequest { profile, command: None, cwd: None }, window, cx);
    }

    fn new_tab_with_profile(&mut self, action: &NewTabWithProfile, window: &mut Window, cx: &mut Context<Self>) {
        if action.0 < Self::settings(cx).profiles.len() {
            self.launch(LaunchRequest { profile: action.0, command: None, cwd: None }, window, cx);
        }
    }

    fn close_tab(&mut self, _: &CloseTab, window: &mut Window, cx: &mut Context<Self>) {
        self.close_tab_at(self.active, window, cx);
    }

    fn close_tab_at(&mut self, ix: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(tab) = self.tabs.get(ix) else { return };
        if let TabContent::Tmux { session, view } = &tab.content {
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
        if self.picker.is_some() {
            self.dismiss_picker(window, cx);
            return;
        }
        let theme = self.active_theme(cx);
        let picker = cx.new(|cx| ProfilePicker::new(theme, cx));
        let subscription = cx.subscribe_in(&picker, window, |this, _, event, window, cx| match event {
            PickerEvent::Confirmed(ix) => {
                let ix = *ix;
                this.dismiss_picker(window, cx);
                this.launch(LaunchRequest { profile: ix, command: None, cwd: None }, window, cx);
            }
            PickerEvent::Dismissed => this.dismiss_picker(window, cx),
        });
        window.focus(&picker.focus_handle(cx));
        self.picker = Some((picker, subscription));
        cx.notify();
    }

    fn dismiss_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.picker = None;
        if let Some(tab) = self.tabs.get(self.active) {
            window.focus(&tab.focus_handle(cx));
        }
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

    fn tmux_detach(&mut self, _: &TmuxDetach, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(tab) = self.tabs.get(self.active)
            && let TabContent::Tmux { session, .. } = &tab.content
        {
            session.update(cx, |s, _| s.detach());
        }
    }

    /// Applies a reloaded config to running terminals.
    pub fn apply_config(&mut self, cx: &mut Context<Self>) {
        let config = Self::settings(cx).clone();
        for tab in &mut self.tabs {
            if let Some(p) = config.profiles.iter().find(|p| p.name == tab.profile.name) {
                tab.profile = p.clone();
            }
            let theme = config.theme(tab.profile.theme.as_deref());
            match &tab.content {
                TabContent::Terminal(view) => {
                    let font_size = tab.profile.font_size;
                    view.update(cx, |view, cx| {
                        view.font_size_override = font_size;
                        view.terminal.update(cx, |t, _| t.theme = theme)
                    });
                }
                TabContent::Tmux { view, .. } => view.update(cx, |view, cx| view.set_theme(theme, cx)),
            }
        }
        cx.notify();
    }

    /// Debug hook for `--send`: types into the active tab.
    pub fn send_to_active(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        if let Some(terminal) = self.tabs.get(self.active).and_then(|t| t.active_terminal(cx)) {
            let bytes = bytes.to_vec();
            terminal.update(cx, |t, cx| t.input(bytes, cx));
        }
    }

    /// Debug hook for `--dump-screen-after`.
    pub fn dump_active_screen(&self, cx: &App) -> String {
        let mut out = String::new();
        for (ix, tab) in self.tabs.iter().enumerate() {
            out.push_str(&format!(
                "--- tab {} {:?}{}\n",
                ix + 1,
                tab.title(cx),
                if ix == self.active { " (active)" } else { "" }
            ));
        }
        if let Some(terminal) = self.tabs.get(self.active).and_then(|t| t.active_terminal(cx)) {
            let terminal = terminal.read(cx);
            let size = terminal.size();
            out.push_str(&format!("--- screen {}x{}\n", size.cols, size.rows));
            out.push_str(&terminal.screen_text());
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
            let title: SharedString = tab.title(cx).into();
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

    fn render_content(&self) -> AnyElement {
        match self.tabs.get(self.active).map(|t| &t.content) {
            Some(TabContent::Terminal(view)) => view.clone().into_any_element(),
            Some(TabContent::Tmux { view, .. }) => view.clone().into_any_element(),
            None => div().into_any_element(),
        }
    }
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
        let title = self.tabs.get(self.active).map(|t| t.title(cx)).unwrap_or_default();
        if title != self.last_title {
            window.set_window_title(&title);
            self.last_title = title;
        }

        let theme = self.active_theme(cx);
        let decorations = window.window_decorations();
        let inset = px(RESIZE_BORDER);
        let rounding = px(8.0);
        let config_error = Settings::get(cx).config_error.clone();

        let body = div()
            .id("workspace")
            .key_context("Workspace")
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::new_tab))
            .on_action(cx.listener(Self::new_tab_with_profile))
            .on_action(cx.listener(Self::close_tab))
            .on_action(cx.listener(Self::next_tab))
            .on_action(cx.listener(Self::prev_tab))
            .on_action(cx.listener(Self::activate_tab))
            .on_action(cx.listener(Self::move_tab_left))
            .on_action(cx.listener(Self::move_tab_right))
            .on_action(cx.listener(Self::close_window))
            .on_action(cx.listener(Self::open_profile_picker))
            .on_action(cx.listener(Self::increase_font_size))
            .on_action(cx.listener(Self::decrease_font_size))
            .on_action(cx.listener(Self::reset_font_size))
            .on_action(cx.listener(Self::open_config))
            .on_action(cx.listener(Self::tmux_detach))
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
                            color: Hsla { h: 0., s: 0., l: 0., a: 0.35 },
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
            .when_some(self.picker.as_ref(), |d, (picker, _)| {
                d.child(
                    div()
                        .absolute()
                        .top(px(TAB_BAR_HEIGHT + 8.0))
                        .left_0()
                        .right_0()
                        .flex()
                        .justify_center()
                        .child(picker.clone()),
                )
            });

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
    fn resize_edges() {
        let s = size(px(800.0), px(600.0));
        let i = px(6.0);
        assert_eq!(resize_edge(point(px(2.0), px(2.0)), i, s), Some(ResizeEdge::TopLeft));
        assert_eq!(resize_edge(point(px(400.0), px(598.0)), i, s), Some(ResizeEdge::Bottom));
        assert_eq!(resize_edge(point(px(799.0), px(300.0)), i, s), Some(ResizeEdge::Right));
        assert_eq!(resize_edge(point(px(400.0), px(300.0)), i, s), None);
    }

    #[test]
    fn quoting() {
        assert_eq!(shell_quote("/a b/c'd"), "'/a b/c'\\''d'");
    }
}
