//! The terminal model: an alacritty `Term` grid plus the backend feeding it.
//!
//! Two backends exist:
//! - `Pty`: a local child process on a pseudo-terminal, driven by
//!   alacritty's I/O thread.
//! - `Remote`: bytes delivered by someone else (a tmux control-mode pane).
//!   Output is fed in with [`Terminal::feed`] and input goes to a callback.

pub mod keys;
pub mod mouse;

use std::borrow::Cow;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use alacritty_terminal::event::{Event as AlacEvent, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point as GridPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{self, ClipboardType, Term, TermMode};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{self, NamedColor, Processor, Rgb, StdSyncHandler};
use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui::{ClipboardItem, Context, EventEmitter, Task};

use crate::config::{Config, CursorShapeConfig, Profile};
use crate::theme::{Color, Theme};

#[derive(Clone)]
pub struct Listener(UnboundedSender<AlacEvent>);

impl EventListener for Listener {
    fn send_event(&self, event: AlacEvent) {
        self.0.unbounded_send(event).ok();
    }
}

/// Grid dimensions in cells, plus the cell size in pixels (reported to apps
/// through `TIOCGWINSZ`, used by image protocols and some TUIs).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridSize {
    pub cols: u16,
    pub rows: u16,
    pub cell_width: u16,
    pub cell_height: u16,
}

impl Default for GridSize {
    fn default() -> Self {
        Self { cols: 80, rows: 24, cell_width: 8, cell_height: 16 }
    }
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.rows as usize
    }
    fn screen_lines(&self) -> usize {
        self.rows as usize
    }
    fn columns(&self) -> usize {
        self.cols as usize
    }
}

impl From<GridSize> for WindowSize {
    fn from(s: GridSize) -> Self {
        WindowSize {
            num_lines: s.rows,
            num_cols: s.cols,
            cell_width: s.cell_width,
            cell_height: s.cell_height,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum TerminalEvent {
    TitleChanged,
    Bell,
    /// The child process exited (PTY) or the pane closed (remote).
    Exited,
}

enum Backend {
    Pty {
        sender: EventLoopSender,
        child_pid: u32,
        master_fd: i32,
    },
    Remote {
        processor: Box<Processor<StdSyncHandler>>,
        input: Rc<dyn Fn(&[u8])>,
    },
}

pub struct Terminal {
    term: Arc<FairMutex<Term<Listener>>>,
    backend: Backend,
    size: GridSize,
    title: Option<String>,
    exited: bool,
    pub theme: Theme,
    /// Replace the theme's default colors (set from tmux pane styles).
    pub default_colors: DefaultColors,
    copy_on_select: bool,
    /// Updated whenever there is output; drives the cursor blink reset.
    pub last_activity: Instant,
    /// Pending flush of a synchronized update (mode 2026) on remote terminals.
    sync_flush: Option<Task<()>>,
    _events: Task<()>,
}

impl EventEmitter<TerminalEvent> for Terminal {}

fn term_config(config: &Config) -> term::Config {
    term::Config {
        scrolling_history: config.scrollback_lines,
        osc52: if config.osc52_paste {
            term::Osc52::CopyPaste
        } else {
            term::Osc52::OnlyCopy
        },
        default_cursor_style: ansi::CursorStyle {
            shape: match config.cursor.shape {
                CursorShapeConfig::Block => ansi::CursorShape::Block,
                CursorShapeConfig::Beam => ansi::CursorShape::Beam,
                CursorShapeConfig::Underline => ansi::CursorShape::Underline,
            },
            blinking: config.cursor.blink,
        },
        ..Default::default()
    }
}

impl Terminal {
    fn spawn_events(
        rx: futures::channel::mpsc::UnboundedReceiver<AlacEvent>,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        cx.spawn(async move |this, cx| {
            let mut rx = rx.ready_chunks(1024);
            while let Some(events) = rx.next().await {
                let alive = this.update(cx, |this, cx| {
                    for event in events {
                        this.handle_event(event, cx);
                    }
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
            }
        })
    }

    fn new(
        term: Arc<FairMutex<Term<Listener>>>,
        rx: futures::channel::mpsc::UnboundedReceiver<AlacEvent>,
        backend: Backend,
        size: GridSize,
        theme: Theme,
        config: &Config,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            _events: Self::spawn_events(rx, cx),
            term,
            backend,
            size,
            title: None,
            exited: false,
            theme,
            default_colors: DefaultColors::default(),
            copy_on_select: config.copy_on_select,
            last_activity: Instant::now(),
            sync_flush: None,
        }
    }

    /// Spawns `profile`'s program on a new PTY. If that fails, the terminal
    /// shows the error instead so the tab explains what went wrong.
    pub fn spawn_pty(
        profile: &Profile,
        command_override: Option<(String, Vec<String>)>,
        cwd_override: Option<std::path::PathBuf>,
        config: &Config,
        cx: &mut Context<Self>,
    ) -> Self {
        let (program, args) = command_override.unwrap_or_else(|| {
            let program = profile
                .command
                .clone()
                .or_else(|| std::env::var("SHELL").ok())
                .unwrap_or_else(|| "/bin/sh".into());
            (program, profile.args.clone())
        });

        let working_directory = cwd_override
            .or_else(|| profile.working_directory())
            .or_else(dirs::home_dir)
            .filter(|dir| dir.is_dir());

        let options = tty::Options {
            shell: Some(tty::Shell::new(program.clone(), args)),
            working_directory,
            drain_on_exit: true,
            env: child_env(profile),
        };

        let (tx, rx) = unbounded();
        let listener = Listener(tx);
        let size = GridSize::default();
        let term = Arc::new(FairMutex::new(Term::new(term_config(config), &size, listener.clone())));
        let spawned = (|| -> anyhow::Result<Backend> {
            let pty = tty::new(&options, size.into(), 0)?;
            let child_pid = pty.child().id();
            let master_fd = std::os::fd::AsRawFd::as_raw_fd(pty.file());
            let event_loop = EventLoop::new(term.clone(), listener, pty, true, false)?;
            let sender = event_loop.channel();
            event_loop.spawn();
            Ok(Backend::Pty { sender, child_pid, master_fd })
        })();

        let theme = config.profile_theme(profile);
        match spawned {
            Ok(backend) => Self::new(term, rx, backend, size, theme, config, cx),
            Err(err) => {
                log::error!("failed to launch {program:?}: {err:#}");
                let text = format!("\x1b[31mBrindle could not start {program:?}:\x1b[0m\r\n{err:#}\r\n");
                Self::message(&text, theme, config, cx)
            }
        }
    }

    /// A terminal whose output is supplied via [`Terminal::feed`] and whose
    /// input is handed to `input`.
    pub fn remote(
        size: GridSize,
        theme: Theme,
        config: &Config,
        input: Rc<dyn Fn(&[u8])>,
        cx: &mut Context<Self>,
    ) -> Self {
        let (tx, rx) = unbounded();
        let term = Arc::new(FairMutex::new(Term::new(term_config(config), &size, Listener(tx))));
        let backend = Backend::Remote { processor: Box::new(Processor::new()), input };
        Self::new(term, rx, backend, size, theme, config, cx)
    }

    /// A read-only terminal showing `text` (used to report errors in a tab).
    pub fn message(text: &str, theme: Theme, config: &Config, cx: &mut Context<Self>) -> Self {
        let mut this = Self::remote(GridSize::default(), theme, config, Rc::new(|_| {}), cx);
        this.feed(text.as_bytes(), cx);
        this
    }

    pub fn is_remote(&self) -> bool {
        matches!(self.backend, Backend::Remote { .. })
    }

    /// Feeds output into a remote terminal.
    pub fn feed(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        let Backend::Remote { processor, .. } = &mut self.backend else {
            return;
        };
        {
            let mut term = self.term.lock();
            processor.advance(&mut *term, bytes);
        }
        self.last_activity = Instant::now();
        // A synchronized update (mode 2026) may be buffering; make sure it is
        // flushed once its timeout passes even if no more output arrives.
        if let Some(deadline) = processor.sync_timeout().sync_timeout()
            && self.sync_flush.is_none()
        {
            let wait = deadline.saturating_duration_since(Instant::now());
            self.sync_flush = Some(cx.spawn(async move |this, cx| {
                cx.background_executor().timer(wait).await;
                this.update(cx, |this, cx| this.flush_sync(cx)).ok();
            }));
        }
        cx.notify();
    }

    fn flush_sync(&mut self, cx: &mut Context<Self>) {
        self.sync_flush = None;
        if let Backend::Remote { processor, .. } = &mut self.backend
            && processor.sync_timeout().sync_timeout().is_some_and(|t| t <= Instant::now())
        {
            processor.stop_sync(&mut *self.term.lock());
            cx.notify();
        }
    }

    pub fn term(&self) -> &Arc<FairMutex<Term<Listener>>> {
        &self.term
    }

    pub fn mode(&self) -> TermMode {
        *self.term.lock().mode()
    }

    pub fn size(&self) -> GridSize {
        self.size
    }

    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    pub fn set_title(&mut self, title: Option<String>, cx: &mut Context<Self>) {
        if self.title != title {
            self.title = title;
            cx.emit(TerminalEvent::TitleChanged);
        }
    }

    pub fn mark_exited(&mut self, cx: &mut Context<Self>) {
        if !self.exited {
            self.exited = true;
            cx.emit(TerminalEvent::Exited);
        }
    }

    fn handle_event(&mut self, event: AlacEvent, cx: &mut Context<Self>) {
        match event {
            AlacEvent::Wakeup => self.last_activity = Instant::now(),
            AlacEvent::Title(title) => self.set_title(Some(title), cx),
            AlacEvent::ResetTitle => self.set_title(None, cx),
            AlacEvent::ClipboardStore(kind, text) => {
                let item = ClipboardItem::new_string(text);
                match kind {
                    ClipboardType::Clipboard => cx.write_to_clipboard(item),
                    ClipboardType::Selection => cx.write_to_primary(item),
                }
            }
            // Only sent when the config allows OSC 52 reads (see `term_config`).
            AlacEvent::ClipboardLoad(kind, format) => {
                let item = match kind {
                    ClipboardType::Clipboard => cx.read_from_clipboard(),
                    ClipboardType::Selection => cx.read_from_primary(),
                };
                let text = item.and_then(|i| i.text()).unwrap_or_default();
                self.reply(format(&text).into_bytes());
            }
            AlacEvent::ColorRequest(index, format) => {
                let color = self.term.lock().colors()[index].unwrap_or_else(|| self.default_color(index));
                self.reply(format(color).into_bytes());
            }
            AlacEvent::PtyWrite(text) => self.reply(text.into_bytes()),
            AlacEvent::TextAreaSizeRequest(format) => self.reply(format(self.size.into()).into_bytes()),
            AlacEvent::Bell => cx.emit(TerminalEvent::Bell),
            AlacEvent::Exit | AlacEvent::ChildExit(_) => self.mark_exited(cx),
            _ => {}
        }
    }

    /// Theme color for an xterm color index (0-255 palette, then the named
    /// colors alacritty uses for foreground/background/cursor).
    pub fn default_color(&self, index: usize) -> Rgb {
        palette_color(&self.theme, self.default_colors, index).into()
    }

    /// Sends a reply the emulator generated (device/color/size reports).
    /// Remote panes drop these: tmux answers such queries itself, and
    /// forwarding them would inject garbage into the pane's input.
    fn reply(&self, bytes: Vec<u8>) {
        if !self.is_remote() {
            self.write(bytes);
        }
    }

    /// Writes raw bytes to the program.
    pub fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        let bytes = bytes.into();
        if bytes.is_empty() {
            return;
        }
        match &self.backend {
            Backend::Pty { sender, .. } => {
                sender.send(Msg::Input(bytes)).ok();
            }
            Backend::Remote { input, .. } => input(&bytes),
        }
    }

    /// Writes user input: snaps the view back to the prompt and drops the
    /// selection, like every other terminal.
    pub fn input(&mut self, bytes: impl Into<Cow<'static, [u8]>>, cx: &mut Context<Self>) {
        {
            let mut term = self.term.lock();
            if term.grid().display_offset() != 0 {
                term.scroll_display(Scroll::Bottom);
                cx.notify();
            }
            if term.selection.take().is_some() {
                cx.notify();
            }
        }
        self.write(bytes);
    }

    pub fn paste(&mut self, text: &str, cx: &mut Context<Self>) {
        let bracketed = self.mode().contains(TermMode::BRACKETED_PASTE);
        let bytes = paste_bytes(text, bracketed);
        self.input(bytes, cx);
    }

    pub fn resize(&mut self, size: GridSize) {
        if size == self.size || size.cols == 0 || size.rows == 0 {
            return;
        }
        self.size = size;
        self.term.lock().resize(size);
        if let Backend::Pty { sender, .. } = &self.backend {
            sender.send(Msg::Resize(size.into())).ok();
        }
    }

    pub fn scroll(&mut self, scroll: Scroll, cx: &mut Context<Self>) {
        self.term.lock().scroll_display(scroll);
        cx.notify();
    }

    pub fn clear_scrollback(&mut self, cx: &mut Context<Self>) {
        self.term.lock().grid_mut().clear_history();
        cx.notify();
    }

    // ---- selection ----------------------------------------------------------

    /// Converts a viewport cell to a grid point (accounting for scrollback).
    pub fn viewport_to_grid(&self, col: usize, line: usize) -> GridPoint {
        let offset = self.term.lock().grid().display_offset();
        term::viewport_to_point(offset, GridPoint::new(line, Column(col)))
    }

    pub fn start_selection(&mut self, col: usize, line: usize, side: Side, ty: SelectionType, cx: &mut Context<Self>) {
        let point = self.viewport_to_grid(col, line);
        let mut term = self.term.lock();
        let mut selection = Selection::new(ty, point, side);
        if ty != SelectionType::Simple {
            // Semantic and line selections select their unit immediately.
            selection.include_all();
        }
        term.selection = Some(selection);
        drop(term);
        cx.notify();
    }

    pub fn update_selection(&mut self, col: usize, line: usize, side: Side, cx: &mut Context<Self>) {
        let point = self.viewport_to_grid(col, line);
        let mut term = self.term.lock();
        if let Some(selection) = term.selection.as_mut() {
            selection.update(point, side);
            cx.notify();
        }
    }

    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        let mut term = self.term.lock();
        let top = term.grid().topmost_line();
        let bottom = term.grid().bottommost_line();
        let last_col = term.grid().last_column();
        let mut selection = Selection::new(SelectionType::Simple, GridPoint::new(top, Column(0)), Side::Left);
        selection.update(GridPoint::new(bottom, last_col), Side::Right);
        term.selection = Some(selection);
        drop(term);
        cx.notify();
    }

    pub fn selection_text(&self) -> Option<String> {
        self.term.lock().selection_to_string().filter(|s| !s.is_empty())
    }

    /// Called when a mouse selection is finished.
    pub fn finish_selection(&mut self, cx: &mut Context<Self>) {
        if let Some(text) = self.selection_text() {
            cx.write_to_primary(ClipboardItem::new_string(text.clone()));
            if self.copy_on_select {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
        }
    }

    // ---- process info -------------------------------------------------------

    /// Name of the foreground process on the PTY (e.g. `vim`), for tab titles.
    pub fn foreground_process_name(&self) -> Option<String> {
        let pid = self.foreground_pid()?;
        let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
        Some(comm.trim().to_string())
    }

    fn foreground_pid(&self) -> Option<i32> {
        let Backend::Pty { master_fd, .. } = &self.backend else {
            return None;
        };
        // SAFETY: the fd stays open for as long as the event loop owns the PTY,
        // and tcgetpgrp has no memory-safety preconditions.
        let pgrp = unsafe { libc::tcgetpgrp(*master_fd) };
        (pgrp > 0).then_some(pgrp)
    }

    /// Current working directory of the foreground process (falls back to the shell).
    pub fn working_directory(&self) -> Option<std::path::PathBuf> {
        let Backend::Pty { child_pid, .. } = &self.backend else {
            return None;
        };
        let pid = self.foreground_pid().map(|p| p as u32).unwrap_or(*child_pid);
        std::fs::read_link(format!("/proc/{pid}/cwd"))
            .or_else(|_| std::fs::read_link(format!("/proc/{child_pid}/cwd")))
            .ok()
    }

    /// The visible screen as plain text (used by `--dump-screen` and tests).
    pub fn screen_text(&self) -> String {
        screen_text(&self.term.lock())
    }

    pub fn shutdown(&mut self) {
        if let Backend::Pty { sender, .. } = &self.backend {
            sender.send(Msg::Shutdown).ok();
        }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub fn screen_text<T>(term: &Term<T>) -> String {
    let grid = term.grid();
    let mut out = String::new();
    for line in 0..grid.screen_lines() {
        let row = &grid[Line(line as i32 - grid.display_offset() as i32)];
        let mut text: String = (0..grid.columns())
            .filter(|&c| {
                !row[Column(c)].flags.contains(term::cell::Flags::WIDE_CHAR_SPACER)
            })
            .map(|c| row[Column(c)].c)
            .collect();
        text.truncate(text.trim_end().len());
        out.push_str(&text);
        out.push('\n');
    }
    out
}

/// Default foreground and background that replace the theme's for one
/// terminal; `None` keeps the theme's.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DefaultColors {
    pub foreground: Option<Color>,
    pub background: Option<Color>,
}

/// The color for an xterm palette index or one of the named colors
/// alacritty uses after it (foreground, background, cursor, dim colors).
fn palette_color(t: &Theme, defaults: DefaultColors, index: usize) -> Color {
    let fg = defaults.foreground.unwrap_or(t.foreground);
    let bg = defaults.background.unwrap_or(t.background);
    match index {
        0..=255 => t.indexed(index as u8),
        i if i == NamedColor::Foreground as usize => fg,
        i if i == NamedColor::Background as usize => bg,
        i if i == NamedColor::Cursor as usize => t.cursor,
        i if i == NamedColor::BrightForeground as usize => fg,
        i if i == NamedColor::DimForeground as usize => fg.mix(bg, DIM_FACTOR),
        i if (NamedColor::DimBlack as usize..=NamedColor::DimWhite as usize).contains(&i) => {
            t.ansi[i - NamedColor::DimBlack as usize].mix(bg, DIM_FACTOR)
        }
        _ => fg,
    }
}

/// How far dim text and dim palette colors fade towards the background.
pub const DIM_FACTOR: f32 = 0.4;

/// Environment for programs started by Brindle (shells and tmux alike).
pub fn child_env(profile: &Profile) -> HashMap<String, String> {
    let mut env: HashMap<String, String> = [
        ("TERM", "xterm-256color"),
        ("COLORTERM", "truecolor"),
        ("TERM_PROGRAM", "Brindle"),
        ("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION")),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_string(), v.to_string()))
    .collect();
    env.extend(profile.env.iter().map(|(k, v)| (k.clone(), v.clone())));
    env
}

/// Prepares clipboard text for the PTY.
pub fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    if bracketed {
        // Strip anything that could terminate the paste early.
        let clean = text.replace('\x1b', "");
        let mut out = b"\x1b[200~".to_vec();
        out.extend_from_slice(clean.as_bytes());
        out.extend_from_slice(b"\x1b[201~");
        out
    } else {
        text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
    }
}

/// Resolves a cell color against the term's dynamic palette and the theme.
pub fn resolve_color(color: ansi::Color, colors: &term::color::Colors, terminal: &Terminal) -> Rgb {
    match color {
        ansi::Color::Spec(rgb) => rgb,
        ansi::Color::Named(named) => {
            colors[named as usize].unwrap_or_else(|| terminal.default_color(named as usize))
        }
        ansi::Color::Indexed(i) => {
            colors[i as usize].unwrap_or_else(|| terminal.default_color(i as usize))
        }
    }
}

/// Sends focus in/out reports when the app asked for them (tmux uses this
/// for `focus-events on`).
pub fn report_focus(terminal: &Terminal, focused: bool) {
    if let Some(seq) = mouse::focus_report(focused, terminal.mode()) {
        terminal.write(seq);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_colors_replace_the_theme() {
        let t = Theme::default();
        let none = DefaultColors::default();
        let fg = NamedColor::Foreground as usize;
        let bg = NamedColor::Background as usize;
        assert_eq!(palette_color(&t, none, fg), t.foreground);
        assert_eq!(palette_color(&t, none, bg), t.background);
        let blue = Color::hex(0x0000ff);
        let tinted = DefaultColors { foreground: Some(blue), background: None };
        assert_eq!(palette_color(&t, tinted, fg), blue);
        assert_eq!(palette_color(&t, tinted, bg), t.background);
        assert_eq!(palette_color(&t, tinted, NamedColor::DimForeground as usize), blue.mix(t.background, DIM_FACTOR));
        // Palette entries and the cursor keep the theme's colors.
        assert_eq!(palette_color(&t, tinted, 1), t.ansi[1]);
        assert_eq!(palette_color(&t, tinted, NamedColor::Cursor as usize), t.cursor);
    }

    #[test]
    fn paste_normalizes_newlines() {
        assert_eq!(paste_bytes("a\nb\r\nc", false), b"a\rb\rc");
    }

    #[test]
    fn bracketed_paste_wraps_and_sanitizes() {
        assert_eq!(paste_bytes("x\x1b[201~y", true), b"\x1b[200~x[201~y\x1b[201~");
    }

    #[test]
    fn screen_text_of_processed_output() {
        let size = GridSize { cols: 10, rows: 3, ..Default::default() };
        let mut term = Term::new(term::Config::default(), &size, alacritty_terminal::event::VoidListener);
        let mut processor: Processor<StdSyncHandler> = Processor::new();
        processor.advance(&mut term, b"hello\r\n\x1b[31mred\x1b[0m");
        assert_eq!(screen_text(&term), "hello\nred\n\n");
    }
}
