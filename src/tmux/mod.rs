//! tmux control-mode integration.
//!
//! A [`TmuxSession`] runs `tmux -C` as a child process and mirrors its
//! session: every tmux window becomes a Brindle tab and every pane gets a
//! [`Terminal`] fed from `%output`. Keystrokes go back via `send-keys -H`.

pub mod layout;
pub mod protocol;

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::rc::Rc;

use futures::StreamExt as _;
use futures::channel::oneshot;
use gpui::{AppContext as _, Context, Entity, EventEmitter, Task};

use crate::config::{Config, Profile};
use crate::terminal::{GridSize, Terminal};
use layout::Layout;
use protocol::{Collector, Event, Notification, PaneId, WindowId};

#[derive(Debug, Clone, PartialEq)]
pub enum TmuxEvent {
    WindowAdded(WindowId),
    WindowClosed(WindowId),
    WindowActivated(WindowId),
    /// The session ended after it had been attached.
    Detached(String),
    /// tmux exited before the session ever attached.
    Failed(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// What a sent command's response should be used for.
#[derive(Debug)]
enum Pending {
    Ignore,
    ListWindows,
    PaneState(PaneId),
    CaptureAlternate(PaneId),
    Capture(PaneId),
    /// A line the user typed, followed by `display-message -p <token>`. Its
    /// commands each reply with their own block, so every block up to the
    /// token's belongs to it.
    UserCommand { token: String, first_error: Option<String>, reply: oneshot::Sender<Result<(), String>> },
}

/// The write side of the control connection, shared with pane input closures.
struct Io {
    writer: std::sync::mpsc::Sender<String>,
    pending: VecDeque<Pending>,
}

impl Io {
    fn send(&mut self, command: String, pending: Pending) {
        if self.write(command) {
            self.pending.push_back(pending);
        }
    }

    /// Writes a command line without expecting a reply of its own.
    fn write(&mut self, command: String) -> bool {
        log::debug!("tmux <- {command}");
        self.writer.send(command).is_ok()
    }
}

/// Matches one reply block to the pending queue and returns what it answers.
/// Blocks of an unfinished user command are absorbed (`None`); the user
/// command itself is returned when its sentinel block arrives.
fn match_reply(pending: &mut VecDeque<Pending>, ok: bool, body: &[Vec<u8>]) -> Option<Pending> {
    if let Some(Pending::UserCommand { token, first_error, .. }) = pending.front_mut() {
        if !ok {
            first_error.get_or_insert_with(|| join_body(body));
            return None;
        }
        if !matches!(body, [line] if line == token.as_bytes()) {
            return None;
        }
    }
    pending.pop_front()
}

fn join_body(body: &[Vec<u8>]) -> String {
    body.iter().map(|l| String::from_utf8_lossy(l)).collect::<Vec<_>>().join(" ")
}

pub struct TmuxWindow {
    pub id: WindowId,
    pub index: u32,
    pub name: String,
    pub layout: Option<Layout>,
    pub active_pane: Option<PaneId>,
    pub zoomed: bool,
}

const PANE_STATE_FORMAT: &str = "#{cursor_x} #{cursor_y} #{alternate_on} #{cursor_flag} \
    #{keypad_cursor_flag} #{keypad_flag} #{mouse_standard_flag} #{mouse_button_flag} \
    #{mouse_all_flag} #{mouse_sgr_flag} #{mouse_utf8_flag}";

/// Snapshot pieces collected while restoring a pane's existing contents.
#[derive(Default)]
struct Restore {
    state: Option<Vec<u32>>,
    alternate: Option<Vec<Vec<u8>>>,
}

pub struct Pane {
    pub terminal: Entity<Terminal>,
    /// Set until the initial `capture-pane` arrives; output is dropped
    /// meanwhile because the snapshot already contains it.
    restoring: Option<Restore>,
}

pub struct TmuxSession {
    pub profile: Profile,
    pub session_name: String,
    config: Config,
    io: Rc<RefCell<Io>>,
    pub windows: BTreeMap<WindowId, TmuxWindow>,
    pub panes: HashMap<PaneId, Pane>,
    pub active_window: Option<WindowId>,
    client_size: Option<(u16, u16)>,
    child: Option<Child>,
    /// Numbers the sentinels of user-typed commands.
    user_commands: u64,
    /// A window list has been received, i.e. the session really attached.
    attached: bool,
    detached: bool,
    _reader: Task<()>,
}

impl EventEmitter<TmuxEvent> for TmuxSession {}

/// The tmux command line for attaching to (or creating) `profile`'s session.
pub fn command_line(profile: &Profile, cwd: Option<&PathBuf>) -> (String, Vec<String>) {
    let program = profile.command.clone().unwrap_or_else(|| "tmux".into());
    let mut args = profile.tmux_args.clone();
    args.extend(["-C".into(), "new-session".into(), "-A".into(), "-s".into()]);
    args.push(profile.tmux_session_name().to_string());
    if let Some(dir) = cwd {
        args.push("-c".into());
        args.push(dir.to_string_lossy().into_owned());
    }
    (program, args)
}

impl TmuxSession {
    pub fn attach(profile: Profile, cwd: Option<PathBuf>, config: &Config, cx: &mut Context<Self>) -> Self {
        let session_name = profile.tmux_session_name().to_string();
        let (program, args) = command_line(&profile, cwd.as_ref().filter(|d| d.is_dir()));
        let (event_tx, event_rx) = futures::channel::mpsc::unbounded::<Option<Event>>();
        let (writer_tx, writer_rx) = std::sync::mpsc::channel::<String>();

        let mut command = Command::new(&program);
        command
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .envs(crate::terminal::child_env(&profile));
        let mut startup_error = None;
        let child = match command.spawn() {
            Ok(mut child) => {
                let stdout = child.stdout.take().unwrap();
                let mut stdin = child.stdin.take().unwrap();
                std::thread::Builder::new()
                    .name("tmux-reader".into())
                    .spawn(move || {
                        let mut reader = BufReader::with_capacity(1 << 16, stdout);
                        let mut collector = Collector::default();
                        let mut line = Vec::new();
                        loop {
                            line.clear();
                            match reader.read_until(b'\n', &mut line) {
                                Ok(0) | Err(_) => break,
                                Ok(_) => {}
                            }
                            let trimmed = line.strip_suffix(b"\n").unwrap_or(&line);
                            if let Some(event) = collector.feed_line(trimmed)
                                && event_tx.unbounded_send(Some(event)).is_err()
                            {
                                return;
                            }
                        }
                        event_tx.unbounded_send(None).ok();
                    })
                    .ok();
                std::thread::Builder::new()
                    .name("tmux-writer".into())
                    .spawn(move || {
                        while let Ok(command) = writer_rx.recv() {
                            let mut batch = command;
                            batch.push('\n');
                            // Coalesce queued commands into one write.
                            while let Ok(more) = writer_rx.try_recv() {
                                batch.push_str(&more);
                                batch.push('\n');
                            }
                            if stdin.write_all(batch.as_bytes()).and_then(|_| stdin.flush()).is_err() {
                                break;
                            }
                        }
                    })
                    .ok();
                Some(child)
            }
            Err(err) => {
                startup_error = Some(format!("could not run {program:?}: {err}"));
                None
            }
        };

        let reader = cx.spawn(async move |this, cx| {
            let mut events = event_rx.ready_chunks(4096);
            while let Some(batch) = events.next().await {
                let alive = this.update(cx, |this, cx| {
                    for event in batch {
                        match event {
                            Some(event) => this.handle_event(event, cx),
                            None => this.on_detached("tmux exited".into(), cx),
                        }
                    }
                });
                if alive.is_err() {
                    break;
                }
            }
        });

        let this = Self {
            profile,
            session_name,
            config: config.clone(),
            io: Rc::new(RefCell::new(Io { writer: writer_tx, pending: VecDeque::new() })),
            windows: BTreeMap::new(),
            panes: HashMap::new(),
            active_window: None,
            client_size: None,
            child,
            user_commands: 0,
            attached: false,
            detached: false,
            _reader: reader,
        };
        if let Some(err) = startup_error {
            log::error!("{err}");
            // Defer so the workspace has subscribed before we report.
            cx.spawn(async move |this, cx| {
                this.update(cx, |this, cx| this.on_detached(err, cx)).ok();
            })
            .detach();
        } else {
            this.list_windows();
        }
        this
    }

    fn send(&self, command: impl Into<String>) {
        self.io.borrow_mut().send(command.into(), Pending::Ignore);
    }

    fn list_windows(&self) {
        let format = "#{window_id}\t#{window_index}\t#{window_active}\t#{window_zoomed_flag}\t\
                      #{window_visible_layout}\t#{window_name}";
        self.io
            .borrow_mut()
            .send(format!("list-windows -F {}", protocol::quote(format)), Pending::ListWindows);
    }

    // ---- commands -----------------------------------------------------------------

    pub fn set_client_size(&mut self, cols: u16, rows: u16) {
        if cols < 2 || rows < 2 || self.client_size == Some((cols, rows)) || self.detached {
            return;
        }
        self.client_size = Some((cols, rows));
        self.send(format!("refresh-client -C {cols}x{rows}"));
    }

    pub fn select_window(&mut self, window: WindowId) {
        if self.active_window != Some(window) && self.windows.contains_key(&window) {
            self.active_window = Some(window);
            self.send(format!("select-window -t @{window}"));
        }
    }

    pub fn select_pane(&mut self, pane: PaneId, cx: &mut Context<Self>) {
        let Some(window) = self.window_of_pane(pane) else { return };
        let window = self.windows.get_mut(&window).unwrap();
        if window.active_pane != Some(pane) {
            window.active_pane = Some(pane);
            self.send(format!("select-pane -t %{pane}"));
            cx.notify();
        }
    }

    pub fn kill_window(&mut self, window: WindowId) {
        self.send(format!("kill-window -t @{window}"));
    }

    pub fn new_window(&mut self) {
        // Formats in -c expand against the client's current pane, so the new
        // window starts where the user is, like local tabs do.
        self.send(format!("new-window -c {}", protocol::quote("#{pane_current_path}")));
    }

    /// Runs `command` (a format with `{pane}` for the target) against the
    /// window's active pane.
    fn send_to_active_pane(&self, window: WindowId, command: impl Fn(PaneId) -> String) {
        if let Some(pane) = self.active_pane_of(window) {
            self.send(command(pane));
        }
    }

    pub fn split(&mut self, window: WindowId, horizontal: bool) {
        let flag = if horizontal { "-h" } else { "-v" };
        let dir = protocol::quote("#{pane_current_path}");
        self.send_to_active_pane(window, |p| format!("split-window {flag} -t %{p} -c {dir}"));
    }

    pub fn kill_pane(&mut self, window: WindowId) {
        self.send_to_active_pane(window, |p| format!("kill-pane -t %{p}"));
    }

    pub fn toggle_zoom(&mut self, window: WindowId) {
        self.send_to_active_pane(window, |p| format!("resize-pane -Z -t %{p}"));
    }

    pub fn focus_direction(&mut self, window: WindowId, direction: Direction) {
        let flag = match direction {
            Direction::Left => "-L",
            Direction::Right => "-R",
            Direction::Up => "-U",
            Direction::Down => "-D",
        };
        self.send_to_active_pane(window, |p| format!("select-pane {flag} -t %{p}"));
    }

    /// `name` is one of tmux's preset layouts, e.g. `tiled`.
    pub fn select_layout(&mut self, window: WindowId, name: &str) {
        self.send(format!("select-layout -t @{window} {name}"));
    }

    pub fn next_layout(&mut self, window: WindowId) {
        self.send(format!("next-layout -t @{window}"));
    }

    pub fn rotate(&mut self, window: WindowId) {
        self.send(format!("rotate-window -Z -t @{window}"));
    }

    /// Swaps the active pane with the previous (`up`) or next pane. Without
    /// `-d` the active pane moves along; without `-s` a marked pane is ignored.
    pub fn swap_pane(&mut self, window: WindowId, up: bool) {
        let flag = if up { "-U" } else { "-D" };
        self.send_to_active_pane(window, |p| format!("swap-pane {flag} -t %{p}"));
    }

    /// Moves the active pane into a new window, which becomes current.
    pub fn break_pane(&mut self, window: WindowId) {
        self.send_to_active_pane(window, |p| format!("break-pane -s %{p}"));
    }

    pub fn rename_window(&mut self, window: WindowId, name: &str) {
        if let Some(command) = rename_window_command(window, name) {
            self.send(command);
        }
    }

    /// Runs a command line the user typed. The result is tmux's first error,
    /// if any; output is discarded. New windows and panes start in the current
    /// pane's directory unless the line says otherwise.
    pub fn run_user_command(&mut self, line: &str) -> oneshot::Receiver<Result<(), String>> {
        let (reply, result) = oneshot::channel();
        let line = line.replace(['\r', '\n'], " ");
        if line.trim().is_empty() {
            reply.send(Err("Type a tmux command".into())).ok();
            return result;
        }
        self.user_commands += 1;
        let token = format!("brindle-sync-{}", self.user_commands);
        // Written back to back under one borrow so nothing else is queued
        // between the line and its sentinel.
        let mut io = self.io.borrow_mut();
        if io.write(protocol::with_start_dir(&line)) {
            io.send(
                format!("display-message -p {token}"),
                Pending::UserCommand { token, first_error: None, reply },
            );
        }
        result
    }

    pub fn detach(&mut self) {
        self.send("detach-client");
    }

    pub fn window(&self, window: WindowId) -> Option<&TmuxWindow> {
        self.windows.get(&window)
    }

    pub fn active_pane_of(&self, window: WindowId) -> Option<PaneId> {
        let window = self.windows.get(&window)?;
        window
            .active_pane
            .or_else(|| window.layout.as_ref()?.panes().first().map(|(id, _)| *id))
    }

    pub fn window_of_pane(&self, pane: PaneId) -> Option<WindowId> {
        self.windows.values().find_map(|w| {
            w.layout.as_ref()?.panes().iter().any(|(id, _)| *id == pane).then_some(w.id)
        })
    }

    pub fn pane_terminal(&self, pane: PaneId) -> Option<Entity<Terminal>> {
        self.panes.get(&pane).map(|p| p.terminal.clone())
    }

    // ---- incoming -----------------------------------------------------------------

    fn handle_event(&mut self, event: Event, cx: &mut Context<Self>) {
        match event {
            Event::Response { ours: false, .. } => {}
            Event::Response { ours: true, ok, body } => {
                let pending = match_reply(&mut self.io.borrow_mut().pending, ok, &body);
                match pending {
                    Some(Pending::ListWindows) if ok => self.on_window_list(body, cx),
                    Some(Pending::PaneState(pane)) => {
                        let state = body
                            .first()
                            .map(|l| {
                                String::from_utf8_lossy(l)
                                    .split(' ')
                                    .map(|f| f.trim().parse().unwrap_or(0))
                                    .collect()
                            })
                            .unwrap_or_default();
                        if let Some(restore) = self.panes.get_mut(&pane).and_then(|p| p.restoring.as_mut()) {
                            restore.state = Some(state);
                        }
                    }
                    Some(Pending::CaptureAlternate(pane)) => {
                        if let Some(restore) = self.panes.get_mut(&pane).and_then(|p| p.restoring.as_mut()) {
                            restore.alternate = ok.then_some(body);
                        }
                    }
                    Some(Pending::Capture(pane)) => self.finish_restore(pane, if ok { body } else { Vec::new() }, cx),
                    Some(Pending::UserCommand { first_error, reply, .. }) => {
                        reply.send(first_error.map_or(Ok(()), Err)).ok();
                    }
                    Some(Pending::ListWindows) | Some(Pending::Ignore) | None => {
                        if !ok {
                            log::warn!("tmux command failed: {}", join_body(&body));
                        }
                    }
                }
            }
            Event::Notification(notification) => self.on_notification(notification, cx),
        }
    }

    fn on_notification(&mut self, notification: Notification, cx: &mut Context<Self>) {
        match notification {
            Notification::Output { pane, data } => {
                if let Some(p) = self.panes.get(&pane)
                    && p.restoring.is_none()
                {
                    p.terminal.update(cx, |t, cx| t.feed(&data, cx));
                }
            }
            Notification::LayoutChange { window, layout, flags } => {
                if let Some(w) = self.windows.get_mut(&window) {
                    w.layout = layout::parse(&layout);
                    w.zoomed = flags.contains('Z');
                    self.sync_panes(cx);
                    cx.notify();
                } else {
                    self.list_windows();
                }
            }
            Notification::WindowAdd(_) | Notification::SessionChanged { .. } => self.list_windows(),
            Notification::WindowClose(window) => {
                if self.windows.remove(&window).is_some() {
                    self.sync_panes(cx);
                    cx.emit(TmuxEvent::WindowClosed(window));
                }
            }
            Notification::WindowRenamed { window, name } => {
                if let Some(w) = self.windows.get_mut(&window) {
                    w.name = name;
                    cx.notify();
                }
            }
            Notification::WindowPaneChanged { window, pane } => {
                if let Some(w) = self.windows.get_mut(&window) {
                    w.active_pane = Some(pane);
                    cx.notify();
                }
            }
            Notification::SessionWindowChanged { window } => {
                if self.windows.contains_key(&window) && self.active_window != Some(window) {
                    self.active_window = Some(window);
                    cx.emit(TmuxEvent::WindowActivated(window));
                }
            }
            Notification::SessionRenamed { name } => {
                self.session_name = name;
                cx.notify();
            }
            Notification::Exit(reason) => {
                self.on_detached(reason.unwrap_or_else(|| "detached".into()), cx)
            }
            _ => {}
        }
    }

    fn on_window_list(&mut self, body: Vec<Vec<u8>>, cx: &mut Context<Self>) {
        self.attached = true;
        let mut seen = Vec::new();
        let mut added = Vec::new();
        let mut active = None;
        for line in body {
            let line = String::from_utf8_lossy(&line);
            let fields: Vec<&str> = line.splitn(6, '\t').collect();
            let [id, index, is_active, zoomed, layout, name] = fields[..] else { continue };
            let Some(id) = id.strip_prefix('@').and_then(|s| s.parse().ok()) else { continue };
            seen.push(id);
            if is_active == "1" {
                active = Some(id);
            }
            let window = self.windows.entry(id).or_insert_with(|| {
                added.push(id);
                TmuxWindow { id, index: 0, name: String::new(), layout: None, active_pane: None, zoomed: false }
            });
            window.index = index.parse().unwrap_or(0);
            window.name = name.to_string();
            window.zoomed = zoomed == "1";
            window.layout = layout::parse(layout);
        }
        let closed: Vec<WindowId> = self.windows.keys().filter(|id| !seen.contains(id)).copied().collect();
        for id in &closed {
            self.windows.remove(id);
        }
        self.sync_panes(cx);

        added.sort_by_key(|id| self.windows[id].index);
        for id in closed {
            cx.emit(TmuxEvent::WindowClosed(id));
        }
        for id in added {
            cx.emit(TmuxEvent::WindowAdded(id));
        }
        if let Some(active) = active
            && self.active_window != Some(active)
        {
            self.active_window = Some(active);
            cx.emit(TmuxEvent::WindowActivated(active));
        }
        cx.notify();
    }

    /// Creates terminals for new panes, resizes existing ones to the
    /// layout, and drops panes that no longer exist.
    fn sync_panes(&mut self, cx: &mut Context<Self>) {
        let mut live = HashMap::new();
        for window in self.windows.values() {
            if let Some(layout) = &window.layout {
                for (id, rect) in layout.panes() {
                    live.insert(id, rect);
                }
            }
        }
        self.panes.retain(|id, _| live.contains_key(id));

        for (&id, rect) in &live {
            let size = GridSize { cols: rect.width.max(1), rows: rect.height.max(1), ..Default::default() };
            if let Some(pane) = self.panes.get(&id) {
                pane.terminal.update(cx, |t, _| t.resize(size));
                continue;
            }
            let io = self.io.clone();
            let input: Rc<dyn Fn(&[u8])> = Rc::new(move |bytes: &[u8]| {
                let mut io = io.borrow_mut();
                for command in protocol::send_keys_commands(id, bytes) {
                    io.send(command, Pending::Ignore);
                }
            });
            let theme = self.config.profile_theme(&self.profile);
            let config = &self.config;
            let terminal = cx.new(|cx| Terminal::remote(size, theme, config, input, cx));
            self.panes.insert(id, Pane { terminal, restoring: Some(Restore::default()) });

            // Snapshot the pane: state, alternate-saved screen, then the visible
            // screen plus history. Sent back to back so no output slips between.
            let mut io = self.io.borrow_mut();
            io.send(
                format!("display-message -p -t %{id} -F {}", protocol::quote(PANE_STATE_FORMAT)),
                Pending::PaneState(id),
            );
            io.send(format!("capture-pane -p -e -a -q -t %{id} -S -"), Pending::CaptureAlternate(id));
            io.send(format!("capture-pane -p -e -t %{id} -S -"), Pending::Capture(id));
        }
    }

    fn finish_restore(&mut self, pane: PaneId, screen: Vec<Vec<u8>>, cx: &mut Context<Self>) {
        let Some(p) = self.panes.get_mut(&pane) else { return };
        let Some(restore) = p.restoring.take() else { return };
        let bytes = restore_bytes(restore.state.as_deref().unwrap_or(&[]), restore.alternate.as_deref(), &screen);
        p.terminal.update(cx, |t, cx| t.feed(&bytes, cx));
    }

    fn on_detached(&mut self, reason: String, cx: &mut Context<Self>) {
        if self.detached {
            return;
        }
        self.detached = true;
        // Nothing more will be answered; dropping waiting user commands
        // closes their prompts.
        self.io.borrow_mut().pending.clear();
        for pane in self.panes.values() {
            pane.terminal.update(cx, |t, cx| t.mark_exited(cx));
        }
        cx.emit(if self.attached { TmuxEvent::Detached(reason) } else { TmuxEvent::Failed(reason) });
    }

    /// Picks up a reloaded config for panes created from now on.
    pub fn set_config(&mut self, profile: Profile, config: &Config) {
        self.profile = profile;
        self.config = config.clone();
    }
}

impl Drop for TmuxSession {
    fn drop(&mut self) {
        if let Some(mut child) = self.child.take() {
            // Detach politely; the server and its session keep running.
            self.io.borrow_mut().send("detach-client".into(), Pending::Ignore);
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_millis(500));
                child.kill().ok();
                child.wait().ok();
            });
        }
    }
}

/// The command renaming `window`, or `None` for an empty name.
fn rename_window_command(window: WindowId, name: &str) -> Option<String> {
    let name = name.trim();
    (!name.is_empty()).then(|| format!("rename-window -t @{window} {}", protocol::quote(name)))
}

fn join_lines(lines: &[Vec<u8>], out: &mut Vec<u8>) {
    for (i, line) in lines.iter().enumerate() {
        if i > 0 {
            out.extend_from_slice(b"\x1b[0m\r\n");
        }
        out.extend_from_slice(line);
    }
    out.extend_from_slice(b"\x1b[0m");
}

/// Escape sequences that recreate a pane from its `capture-pane` snapshot
/// and the modes reported by `display-message` (see `PANE_STATE_FORMAT`).
pub fn restore_bytes(state: &[u32], alternate: Option<&[Vec<u8>]>, screen: &[Vec<u8>]) -> Vec<u8> {
    let field = |i: usize| state.get(i).copied().unwrap_or(0);
    let (cursor_x, cursor_y, alternate_on) = (field(0), field(1), field(2) == 1);
    let mut out = Vec::new();
    if alternate_on {
        if let Some(normal) = alternate {
            join_lines(normal, &mut out);
        }
        out.extend_from_slice(b"\x1b[?1049h\x1b[H\x1b[2J");
    }
    join_lines(screen, &mut out);
    out.extend_from_slice(format!("\x1b[{};{}H", cursor_y + 1, cursor_x + 1).as_bytes());
    if state.len() > 3 && field(3) == 0 {
        out.extend_from_slice(b"\x1b[?25l");
    }
    let modes: [(usize, &[u8]); 7] = [
        (4, b"\x1b[?1h"),
        (5, b"\x1b="),
        (6, b"\x1b[?1000h"),
        (7, b"\x1b[?1002h"),
        (8, b"\x1b[?1003h"),
        (9, b"\x1b[?1006h"),
        (10, b"\x1b[?1005h"),
    ];
    for (ix, seq) in modes {
        if field(ix) == 1 {
            out.extend_from_slice(seq);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_line_for_profile() {
        let profile = Profile {
            name: "t".into(),
            tmux_session: Some("work".into()),
            tmux_args: vec!["-L".into(), "x".into()],
            ..Default::default()
        };
        let (program, args) = command_line(&profile, Some(&PathBuf::from("/tmp")));
        assert_eq!(program, "tmux");
        assert_eq!(args, ["-L", "x", "-C", "new-session", "-A", "-s", "work", "-c", "/tmp"]);
    }

    #[test]
    fn user_command_replies_up_to_its_sentinel() {
        let lines = |s: &str| vec![s.as_bytes().to_vec()];
        let user = |n: u32| {
            let (reply, result) = oneshot::channel();
            (Pending::UserCommand { token: format!("brindle-sync-{n}"), first_error: None, reply }, result)
        };
        let mut queue = VecDeque::new();
        let (first, _r1) = user(1);
        let (second, _r2) = user(2);
        let (third, _r3) = user(3);
        queue.extend([first, Pending::ListWindows, second, third, Pending::Ignore]);

        // One command, then its sentinel: Ok.
        assert!(match_reply(&mut queue, true, &[]).is_none());
        let done = match_reply(&mut queue, true, &lines("brindle-sync-1"));
        assert!(matches!(done, Some(Pending::UserCommand { first_error: None, .. })));
        // The next pending entry is matched normally.
        assert!(matches!(match_reply(&mut queue, true, &lines("@1\t0")), Some(Pending::ListWindows)));
        // An error, then the sentinel: Err with the first message.
        assert!(match_reply(&mut queue, false, &lines("unknown command: bogus")).is_none());
        let done = match_reply(&mut queue, true, &lines("brindle-sync-2"));
        assert!(
            matches!(done, Some(Pending::UserCommand { first_error: Some(ref e), .. }) if e == "unknown command: bogus")
        );
        // Two commands (`a ; b`) produce two blocks before the sentinel.
        assert!(match_reply(&mut queue, true, &[]).is_none());
        assert!(match_reply(&mut queue, true, &lines("output")).is_none());
        assert!(matches!(
            match_reply(&mut queue, true, &lines("brindle-sync-3")),
            Some(Pending::UserCommand { first_error: None, .. })
        ));
        assert!(matches!(match_reply(&mut queue, true, &[]), Some(Pending::Ignore)));
    }

    #[test]
    fn rename_window_quotes_and_skips_empty() {
        assert_eq!(rename_window_command(3, "build"), Some(r#"rename-window -t @3 "build""#.into()));
        assert_eq!(rename_window_command(3, r#"a "b" $c"#), Some(r#"rename-window -t @3 "a \"b\" \$c""#.into()));
        assert_eq!(rename_window_command(3, "  "), None);
    }

    #[test]
    fn restore_plain_screen() {
        let bytes = restore_bytes(&[3, 1, 0, 1, 0, 0, 0, 0, 0, 0, 0], None, &[b"one".to_vec(), b"two".to_vec()]);
        assert_eq!(bytes, b"one\x1b[0m\r\ntwo\x1b[0m\x1b[2;4H");
    }

    #[test]
    fn restore_alternate_screen_and_modes() {
        let bytes = restore_bytes(
            &[0, 0, 1, 0, 1, 0, 1, 1, 0, 1, 0],
            Some(&[b"shell".to_vec()]),
            &[b"vim".to_vec()],
        );
        let s = String::from_utf8(bytes).unwrap();
        assert!(s.starts_with("shell\x1b[0m\x1b[?1049h"));
        assert!(s.contains("vim"));
        for seq in ["\x1b[?25l", "\x1b[?1h", "\x1b[?1000h", "\x1b[?1002h", "\x1b[?1006h"] {
            assert!(s.contains(seq), "{seq:?}");
        }
        assert!(!s.contains("\x1b[?1003h"));
    }
}
