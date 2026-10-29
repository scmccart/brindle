//! The terminal model: an alacritty `Term` grid plus the backend feeding it.
//!
//! Two backends exist:
//! - `Pty`: a local child process on a pseudo-terminal, driven by
//!   alacritty's I/O thread.
//! - `Remote`: bytes delivered by someone else (a tmux control-mode pane).
//!   Output is fed in with [`Terminal::feed`] and input goes to a callback.

// Used by the Windows PTY; tested everywhere.
#[cfg_attr(not(windows), allow(dead_code))]
pub mod cmdline;
#[cfg(windows)]
mod conpty;
pub mod keys;
pub mod links;
pub mod mouse;
pub mod process;
mod pty;
pub mod shell_integration;
mod scan;
pub mod search;
pub mod status;

use std::borrow::Cow;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event as AlacEvent, EventListener, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, EventLoopSender, Msg};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point as GridPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{self, ClipboardType, Term, TermMode};
#[cfg(unix)]
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{self, NamedColor, Rgb};
use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedSender, unbounded};
use gpui::{ClipboardItem, Context, EventEmitter, Task};
use parking_lot::Mutex;

use self::shell_integration::{Marks, Parser};

use crate::config::{Config, CursorShapeConfig, Profile};
use crate::latency::Latency;
use crate::notify::osc::{Found, Notice, Scanner};
use crate::theme::{Color, Theme};

/// What reaches the UI thread from the emulator and the output scanner.
pub enum ChannelEvent {
    Alac(AlacEvent),
    /// A sequence the scanner found in PTY output (on alacritty's I/O
    /// thread).
    Osc(Found),
}

#[derive(Clone)]
pub struct Listener(UnboundedSender<ChannelEvent>, Arc<OutputMarker>);

impl EventListener for Listener {
    fn send_event(&self, event: AlacEvent) {
        if matches!(event, AlacEvent::Wakeup) {
            // On alacritty's I/O thread, right after it parsed new output.
            self.1.advance(Instant::now());
        }
        self.0.unbounded_send(ChannelEvent::Alac(event)).ok();
    }
}

/// Counts output reaching a terminal, and when the latest arrived. Latency
/// probes compare it with the generation a paint showed (see
/// [`crate::latency`]).
#[derive(Default)]
pub struct OutputMarker {
    generation: AtomicU64,
    /// Nanoseconds after [`epoch`].
    at: AtomicU64,
}

fn epoch() -> Instant {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    *EPOCH.get_or_init(Instant::now)
}

impl OutputMarker {
    fn advance(&self, now: Instant) {
        let nanos = now.saturating_duration_since(epoch()).as_nanos() as u64;
        self.at.store(nanos, Ordering::Relaxed);
        self.generation.fetch_add(1, Ordering::Release);
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::Acquire)
    }

    /// When the latest output arrived.
    pub fn at(&self) -> Instant {
        epoch() + Duration::from_nanos(self.at.load(Ordering::Relaxed))
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
    /// A program asked for a desktop notification (OSC 9 / OSC 777).
    Notify(Notice),
    /// The child process exited (PTY) or the pane closed (remote).
    Exited,
    /// The program status records (OSC 7501) changed. The notice is set
    /// when a record now needs the user: blocked, done or error.
    Status(Option<Notice>),
}

enum Backend {
    Pty {
        sender: EventLoopSender,
        process: process::PtyProcess,
        #[cfg(windows)]
        console: Arc<conpty::Pseudoconsole>,
    },
    Remote {
        scanner: Scanner,
        input: Rc<dyn Fn(&[u8])>,
        palette: Option<PaletteHandler>,
    },
}

/// Answers a remote terminal's palette query (OSC 4): given the index and
/// the formatted reply, it decides whether to send it.
pub type PaletteHandler = Rc<dyn Fn(usize, Vec<u8>)>;

pub struct Terminal {
    term: Arc<FairMutex<Term<Listener>>>,
    backend: Backend,
    output: Arc<OutputMarker>,
    /// The vte processor and OSC 133 scanner. Locked before `term`; the PTY
    /// reader thread uses it too (see `pty.rs`).
    parser: Arc<Mutex<Parser>>,
    /// Shell-integration marks. Locked after `term`.
    marks: Arc<Mutex<Marks>>,
    size: GridSize,
    title: Option<String>,
    exited: bool,
    /// Program status records (OSC 7501).
    status: status::Records,
    pub theme: Theme,
    /// Replace the theme's default colors (set from tmux pane styles).
    pub default_colors: DefaultColors,
    copy_on_select: bool,
    /// Updated whenever there is output; drives the cursor blink reset.
    pub last_activity: Instant,
    /// Pending flush of a synchronized update (mode 2026).
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
        rx: futures::channel::mpsc::UnboundedReceiver<ChannelEvent>,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        cx.spawn(async move |this, cx| {
            let mut rx = rx.ready_chunks(1024);
            while let Some(events) = rx.next().await {
                let alive = this.update(cx, |this, cx| {
                    for event in events {
                        match event {
                            ChannelEvent::Alac(event) => this.handle_event(event, cx),
                            ChannelEvent::Osc(found) => this.handle_found(found, cx),
                        }
                    }
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
            }
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn new(
        term: Arc<FairMutex<Term<Listener>>>,
        output: Arc<OutputMarker>,
        rx: futures::channel::mpsc::UnboundedReceiver<ChannelEvent>,
        backend: Backend,
        parser: Arc<Mutex<Parser>>,
        marks: Arc<Mutex<Marks>>,
        size: GridSize,
        theme: Theme,
        config: &Config,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            _events: Self::spawn_events(rx, cx),
            term,
            backend,
            output,
            parser,
            marks,
            size,
            title: None,
            exited: false,
            status: status::Records::default(),
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
        let (program, args) = command_override.unwrap_or_else(|| match &profile.command {
            Some(program) => (program.clone(), profile.args.clone()),
            None => default_program(&profile.args),
        });

        let working_directory = cwd_override
            .or_else(|| profile.working_directory())
            .or_else(dirs::home_dir)
            .filter(|dir| dir.is_dir());

        #[cfg(unix)]
        let options = tty::Options {
            shell: Some(tty::Shell::new(program.clone(), args)),
            working_directory,
            drain_on_exit: true,
            env: child_env(profile),
        };
        #[cfg(windows)]
        let command = conpty::Command {
            line: cmdline::launch_line(&program, &args),
            cwd: working_directory,
            env: child_env(profile).into_iter().collect(),
        };

        let (tx, rx) = unbounded();
        let output = Arc::new(OutputMarker::default());
        let listener = Listener(tx.clone(), output.clone());
        let size = GridSize::default();
        let term = Arc::new(FairMutex::new(Term::new(term_config(config), &size, listener.clone())));
        let parser = Arc::new(Mutex::new(Parser::new()));
        let marks = Arc::new(Mutex::new(Marks::default()));
        #[cfg(unix)]
        let spawned = (|| -> anyhow::Result<Backend> {
            let pty = tty::new(&options, size.into(), 0)?;
            let child_pid = pty.child().id();
            let master_fd = std::os::fd::AsRawFd::as_raw_fd(pty.file());
            let pty = pty::MarkedPty::new(pty, term.clone(), parser.clone(), marks.clone(), listener.clone(), tx)?;
            let event_loop = EventLoop::new(term.clone(), listener, pty, true, false)?;
            let sender = event_loop.channel();
            event_loop.spawn();
            Ok(Backend::Pty { sender, process: process::PtyProcess::new(child_pid, master_fd) })
        })();
        #[cfg(windows)]
        let spawned = (|| -> anyhow::Result<Backend> {
            let spawned = conpty::spawn(&command, size.into())?;
            let (child_pid, console) = (spawned.pid, spawned.console.clone());
            let pty = pty::MarkedPty::new(spawned, term.clone(), parser.clone(), marks.clone(), listener.clone(), tx);
            let event_loop = EventLoop::new(term.clone(), listener, pty, true, false)?;
            let sender = event_loop.channel();
            event_loop.spawn();
            Ok(Backend::Pty { sender, process: process::PtyProcess::new(child_pid), console })
        })();

        let theme = config.profile_theme(profile);
        match spawned {
            Ok(backend) => Self::new(term, output, rx, backend, parser, marks, size, theme, config, cx),
            Err(err) => {
                log::error!("failed to launch {program:?}: {err:#}");
                let text = format!("\x1b[31mBrindle could not start {program:?}:\x1b[0m\r\n{err:#}\r\n");
                Self::message(&text, theme, config, cx)
            }
        }
    }

    /// A terminal whose output is supplied via [`Terminal::feed`] and whose
    /// input is handed to `input`. Palette queries go to `palette`, if any.
    pub fn remote(
        size: GridSize,
        theme: Theme,
        config: &Config,
        input: Rc<dyn Fn(&[u8])>,
        palette: Option<PaletteHandler>,
        cx: &mut Context<Self>,
    ) -> Self {
        let (tx, rx) = unbounded();
        let output = Arc::new(OutputMarker::default());
        let term = Arc::new(FairMutex::new(Term::new(term_config(config), &size, Listener(tx, output.clone()))));
        let backend = Backend::Remote { scanner: Scanner::new(), input, palette };
        let parser = Arc::new(Mutex::new(Parser::new()));
        let marks = Arc::new(Mutex::new(Marks::default()));
        Self::new(term, output, rx, backend, parser, marks, size, theme, config, cx)
    }

    /// A read-only terminal showing `text` (used to report errors in a tab).
    pub fn message(text: &str, theme: Theme, config: &Config, cx: &mut Context<Self>) -> Self {
        let mut this = Self::remote(GridSize::default(), theme, config, Rc::new(|_| {}), None, cx);
        this.feed(text.as_bytes(), cx);
        this
    }

    pub fn is_remote(&self) -> bool {
        matches!(self.backend, Backend::Remote { .. })
    }

    /// Feeds output into a remote terminal.
    pub fn feed(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        let Backend::Remote { scanner, .. } = &mut self.backend else {
            return;
        };
        let found = scanner.scan(bytes);
        {
            let mut parser = self.parser.lock();
            let mut term = self.term.lock_unfair();
            parser.advance(&mut *term, &self.marks, bytes);
            self.output.advance(Instant::now());
        }
        for item in found {
            self.handle_found(item, cx);
        }
        self.last_activity = Instant::now();
        Latency::output(cx.entity_id(), self.output.generation(), self.output.at(), cx);
        self.schedule_sync_flush(cx);
        cx.notify();
    }

    /// A synchronized update (mode 2026) may be buffering output; make sure
    /// it is flushed once its timeout passes even if no more output arrives.
    fn schedule_sync_flush(&mut self, cx: &mut Context<Self>) {
        if self.sync_flush.is_some() {
            return;
        }
        let Some(deadline) = self.parser.lock().sync_deadline() else { return };
        let wait = deadline.saturating_duration_since(Instant::now());
        self.sync_flush = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(wait).await;
            this.update(cx, |this, cx| this.flush_sync(cx)).ok();
        }));
    }

    fn flush_sync(&mut self, cx: &mut Context<Self>) {
        self.sync_flush = None;
        let pending = {
            let mut parser = self.parser.lock();
            match parser.sync_deadline() {
                Some(t) if t <= Instant::now() => {
                    parser.stop_sync(&mut *self.term.lock_unfair(), &self.marks);
                    cx.notify();
                    false
                }
                Some(_) => true,
                None => false,
            }
        };
        if pending {
            self.schedule_sync_flush(cx);
        }
    }

    /// How much output has arrived; see [`OutputMarker`].
    pub fn output_marker(&self) -> &OutputMarker {
        &self.output
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
            if self.status.end_activity() {
                cx.emit(TerminalEvent::Status(None));
            }
            cx.emit(TerminalEvent::Exited);
        }
    }

    /// The program status records (OSC 7501).
    pub fn status(&self) -> &status::Records {
        &self.status
    }

    /// Acts on a sequence the scanner found in the output, in stream order.
    fn handle_found(&mut self, found: Found, cx: &mut Context<Self>) {
        let changed = match found {
            Found::Notice(notice) => {
                cx.emit(TerminalEvent::Notify(notice));
                return;
            }
            Found::Status(status::Message::Query) => {
                // tmux doesn't answer this one, so a pane's reply goes
                // through its input like the palette replies.
                self.write(status::QUERY_REPLY);
                return;
            }
            Found::Status(message) => {
                let update = self.status.apply(&message);
                if update.changed {
                    cx.emit(TerminalEvent::Status(update.notice));
                }
                return;
            }
            Found::PromptStart => self.status.end_activity(),
            Found::Reset => self.status.reset(),
        };
        if changed {
            cx.emit(TerminalEvent::Status(None));
        }
    }

    fn handle_event(&mut self, event: AlacEvent, cx: &mut Context<Self>) {
        match event {
            AlacEvent::Wakeup => {
                self.last_activity = Instant::now();
                Latency::output(cx.entity_id(), self.output.generation(), self.output.at(), cx);
                self.schedule_sync_flush(cx);
            }
            AlacEvent::Title(title) => self.set_title(Some(title), cx),
            AlacEvent::ResetTitle => self.set_title(None, cx),
            AlacEvent::ClipboardStore(kind, text) => {
                let item = ClipboardItem::new_string(text);
                match kind {
                    ClipboardType::Clipboard => cx.write_to_clipboard(item),
                    ClipboardType::Selection => crate::primary::write_primary(cx, item),
                }
            }
            // Only sent when the config allows OSC 52 reads (see `term_config`).
            AlacEvent::ClipboardLoad(kind, format) => {
                let item = match kind {
                    ClipboardType::Clipboard => cx.read_from_clipboard(),
                    ClipboardType::Selection => crate::primary::read_primary(cx),
                };
                let text = item.and_then(|i| i.text()).unwrap_or_default();
                self.reply(format(&text).into_bytes());
            }
            AlacEvent::ColorRequest(index, format) => {
                let set = self.term.lock().colors()[index];
                let reply = format(set.unwrap_or_else(|| self.default_color(index))).into_bytes();
                // A palette index the program set is in tmux's pane palette
                // too, and tmux answers it.
                match &self.backend {
                    Backend::Remote { palette: Some(palette), .. } if index < 256 && set.is_none() => {
                        palette(index, reply)
                    }
                    _ => self.reply(reply),
                }
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
    /// forwarding them would inject garbage into the pane's input. Palette
    /// queries, which tmux leaves to us, go to the pane's `PaletteHandler`.
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
    /// selection, like every other terminal. The user has now seen the
    /// program's `done` and `error` status records.
    pub fn input(&mut self, bytes: impl Into<Cow<'static, [u8]>>, cx: &mut Context<Self>) {
        if self.status.seen() {
            cx.emit(TerminalEvent::Status(None));
        }
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
        let mut term = self.term.lock();
        self.marks.lock().resize(&mut *term, size);
        drop(term);
        if let Backend::Pty { sender, .. } = &self.backend {
            sender.send(Msg::Resize(size.into())).ok();
        }
    }

    pub fn scroll(&mut self, scroll: Scroll, cx: &mut Context<Self>) {
        self.term.lock().scroll_display(scroll);
        cx.notify();
    }

    pub fn clear_scrollback(&mut self, cx: &mut Context<Self>) {
        let mut term = self.term.lock();
        term.grid_mut().clear_history();
        self.marks.lock().prune(&term);
        cx.notify();
    }

    // ---- shell integration ----------------------------------------------------

    /// The shell-integration marks; lock only after (or without) the term.
    pub fn marks(&self) -> &Arc<Mutex<Marks>> {
        &self.marks
    }

    /// Scrolls the view so the previous (or next) prompt is its top line.
    /// Returns `false`, doing nothing, when the terminal has no prompt marks.
    pub fn jump_to_prompt(&mut self, previous: bool, cx: &mut Context<Self>) -> bool {
        let mut term = self.term.lock();
        let offset = term.grid().display_offset() as i64;
        let target = {
            let marks = self.marks.lock();
            if marks.prompt_lines().next().is_none() {
                return false;
            }
            marks.jump_target(-offset, previous)
        };
        let history = term.grid().history_size() as i64;
        let new_offset = match target {
            Some(line) => (-line).clamp(0, history),
            None if previous => offset,
            None => 0,
        };
        if new_offset != offset {
            term.scroll_display(Scroll::Delta((new_offset - offset) as i32));
            cx.notify();
        }
        true
    }

    /// Selects the last command's output. Returns `false` when there is none.
    pub fn select_last_output(&mut self, cx: &mut Context<Self>) -> bool {
        let mut term = self.term.lock();
        let Some((start, end)) = self.marks.lock().last_output(&term) else { return false };
        let mut selection = Selection::new(SelectionType::Simple, start, Side::Left);
        selection.update(end, Side::Right);
        term.selection = Some(selection);
        drop(term);
        self.finish_selection(cx);
        cx.notify();
        true
    }

    /// The last command's output as text, leaving the selection alone.
    pub fn last_output_text(&self) -> Option<String> {
        let term = self.term.lock();
        let (start, end) = self.marks.lock().last_output(&term)?;
        Some(term.bounds_to_string(start, end)).filter(|s| !s.is_empty())
    }

    // ---- selection ----------------------------------------------------------

    /// Converts a viewport cell to a grid point (accounting for scrollback).
    pub fn viewport_to_grid(&self, col: usize, line: usize) -> GridPoint {
        let offset = self.term.lock().grid().display_offset();
        term::viewport_to_point(offset, GridPoint::new(line, Column(col)))
    }

    /// The link at a viewport cell, if any (see [`link_at`]).
    pub fn link_at_viewport(&self, col: usize, line: usize) -> Option<Link> {
        let term = self.term.lock();
        let point = term::viewport_to_point(term.grid().display_offset(), GridPoint::new(line, Column(col)));
        link_at(&*term, point)
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
            crate::primary::write_primary(cx, ClipboardItem::new_string(text.clone()));
            if self.copy_on_select {
                cx.write_to_clipboard(ClipboardItem::new_string(text));
            }
        }
    }

    // ---- process info -------------------------------------------------------

    /// Name of the foreground process on the PTY (e.g. `vim`), for tab
    /// titles. `snapshot` is the title tick's look at all processes.
    pub fn foreground_process_name(&self, snapshot: &process::Snapshot) -> Option<String> {
        let Backend::Pty { process, .. } = &self.backend else {
            return None;
        };
        process.foreground_name(snapshot)
    }

    /// Parents the tab's console window to `window`, so dialogs that console
    /// programs open (credential prompts, for one) belong to Brindle's
    /// window. Local tabs never move between windows, so once is enough.
    #[cfg(windows)]
    pub fn parent_console_to(&self, window: &gpui::Window) {
        let Backend::Pty { console, .. } = &self.backend else {
            return;
        };
        if let Some(hwnd) = crate::win::hwnd(window) {
            console.reparent(hwnd);
        }
    }

    /// Current working directory of the foreground process (falls back to
    /// the shell). On Windows, where no other process's directory can be
    /// read, it is the directory the program last reported (OSC 7 or 9;9),
    /// if it still exists.
    pub fn working_directory(&self) -> Option<std::path::PathBuf> {
        let Backend::Pty { process, .. } = &self.backend else {
            return None;
        };
        if cfg!(windows) {
            return self.marks.lock().reported_dir().filter(|dir| dir.is_dir()).map(Into::into);
        }
        process.working_directory()
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

/// A link found in a terminal: its URI and the cells it covers, as
/// inclusive column ranges per row in grid coordinates (scrollback lines are
/// negative), so it stays on its text while the view scrolls.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    pub uri: String,
    pub spans: Vec<(Line, Column, Column)>,
}

impl Link {
    pub fn contains(&self, point: GridPoint) -> bool {
        self.spans.iter().any(|&(line, first, last)| line == point.line && (first..=last).contains(&point.column))
    }
}

/// How many wrapped rows above and below the pointer are joined when looking
/// for a URL, so a pathological line can't stall the UI.
const MAX_WRAPPED_ROWS: i32 = 32;

/// The link at `point`: the cell's OSC 8 hyperlink if it has one, otherwise
/// a URL detected in the logical line (rows joined where the terminal
/// wrapped them).
pub fn link_at<T>(term: &Term<T>, point: GridPoint) -> Option<Link> {
    use term::cell::Flags;
    let grid = term.grid();
    let cols = grid.columns();
    if point.line < grid.topmost_line() || point.line > grid.bottommost_line() || point.column.0 >= cols {
        return None;
    }
    let mut point = point;
    // The spacer after a wide character belongs to that character.
    if grid[point].flags.contains(Flags::WIDE_CHAR_SPACER) && point.column.0 > 0 {
        point.column -= 1;
    }

    // OSC 8: every visible cell carrying the same hyperlink.
    if let Some(hyperlink) = grid[point].hyperlink() {
        let offset = grid.display_offset() as i32;
        let mut spans = Vec::new();
        for row in 0..grid.screen_lines() as i32 {
            let line = Line(row - offset);
            let mut first = None;
            for col in 0..=cols {
                let linked = col < cols && grid[line][Column(col)].hyperlink().as_ref() == Some(&hyperlink);
                match (linked, first) {
                    (true, None) => first = Some(col),
                    (false, Some(start)) => {
                        spans.push((line, Column(start), Column(col - 1)));
                        first = None;
                    }
                    _ => {}
                }
            }
        }
        return Some(Link { uri: hyperlink.uri().to_string(), spans });
    }

    // Detection over the logical line around `point`.
    let wraps = |line: Line| grid[line][Column(cols - 1)].flags.contains(Flags::WRAPLINE);
    let mut top = point.line;
    while top > grid.topmost_line() && point.line.0 - top.0 < MAX_WRAPPED_ROWS && wraps(Line(top.0 - 1)) {
        top = Line(top.0 - 1);
    }
    let mut bottom = point.line;
    while bottom < grid.bottommost_line() && bottom.0 - point.line.0 < MAX_WRAPPED_ROWS && wraps(bottom) {
        bottom = Line(bottom.0 + 1);
    }
    let mut chars = Vec::new();
    let mut cells = Vec::new();
    let mut at = None;
    for line in top.0..=bottom.0 {
        let row = &grid[Line(line)];
        for col in 0..cols {
            let cell = &row[Column(col)];
            if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                continue;
            }
            let here = GridPoint::new(Line(line), Column(col));
            if here == point {
                at = Some(chars.len());
            }
            chars.push(cell.c);
            let width = if cell.flags.contains(Flags::WIDE_CHAR) { 2 } else { 1 };
            cells.push((here, width));
        }
    }
    let range = links::find_url(&chars, at?)?;
    let uri: String = chars[range.clone()].iter().collect();
    let mut spans: Vec<(Line, Column, Column)> = Vec::new();
    for &(p, width) in &cells[range] {
        let last = Column(p.column.0 + width - 1);
        match spans.last_mut() {
            Some((line, _, end)) if *line == p.line => *end = last,
            _ => spans.push((p.line, p.column, last)),
        }
    }
    Some(Link { uri, spans })
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

/// The program a profile with no `command` runs, with the profile's `args`:
/// `$SHELL`, then `/bin/sh`.
#[cfg(unix)]
fn default_program(args: &[String]) -> (String, Vec<String>) {
    let program = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    (program, args.to_vec())
}

/// The program a profile with no `command` runs, with the profile's `args`
/// after its own: PowerShell 7, Windows PowerShell, `%COMSPEC%` or `cmd.exe`.
#[cfg(windows)]
fn default_program(args: &[String]) -> (String, Vec<String>) {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let comspec = std::env::var("COMSPEC").ok();
    let (program, mut own) = cmdline::windows_default_program(std::env::split_paths(&path), comspec.as_deref());
    own.extend_from_slice(args);
    (program, own)
}

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
    use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};

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
    fn only_output_advances_the_marker() {
        let (tx, _rx) = unbounded();
        let marker = Arc::new(OutputMarker::default());
        let listener = Listener(tx, marker.clone());
        let before = Instant::now();
        listener.send_event(AlacEvent::Bell);
        listener.send_event(AlacEvent::Title("t".into()));
        assert_eq!(marker.generation(), 0);
        listener.send_event(AlacEvent::Wakeup);
        listener.send_event(AlacEvent::Wakeup);
        assert_eq!(marker.generation(), 2);
        assert!(marker.at() >= before && marker.at() <= Instant::now());
    }

    #[test]
    fn paste_normalizes_newlines() {
        assert_eq!(paste_bytes("a\nb\r\nc", false), b"a\rb\rc");
    }

    #[test]
    fn bracketed_paste_wraps_and_sanitizes() {
        assert_eq!(paste_bytes("x\x1b[201~y", true), b"\x1b[200~x[201~y\x1b[201~");
    }

    fn term_with(cols: usize, rows: usize, bytes: &[u8]) -> Term<alacritty_terminal::event::VoidListener> {
        let size = GridSize { cols: cols as u16, rows: rows as u16, ..Default::default() };
        let mut term = Term::new(term::Config::default(), &size, alacritty_terminal::event::VoidListener);
        let mut processor: Processor<StdSyncHandler> = Processor::new();
        processor.advance(&mut term, bytes);
        term
    }

    fn at(line: i32, col: usize) -> GridPoint {
        GridPoint::new(Line(line), Column(col))
    }

    #[test]
    fn detected_link_on_one_row() {
        let term = term_with(40, 3, b"see https://example.com/a. ok");
        let link = link_at(&term, at(0, 10)).unwrap();
        assert_eq!(link.uri, "https://example.com/a");
        assert_eq!(link.spans, vec![(Line(0), Column(4), Column(24))]);
        assert!(link.contains(at(0, 4)) && !link.contains(at(0, 25)));
        assert_eq!(link_at(&term, at(0, 1)), None);
        assert_eq!(link_at(&term, at(0, 25)), None);
    }

    #[test]
    fn wrapped_link_is_joined() {
        // A 28-character URL from column 3 of a 20-column terminal wraps onto row 1.
        let term = term_with(20, 4, b"xx https://example.com/abcdefgh end");
        for point in [at(0, 5), at(1, 3)] {
            let link = link_at(&term, point).unwrap();
            assert_eq!(link.uri, "https://example.com/abcdefgh");
            assert_eq!(link.spans, vec![(Line(0), Column(3), Column(19)), (Line(1), Column(0), Column(10))]);
        }
    }

    #[test]
    fn hard_line_breaks_are_not_joined() {
        let term = term_with(40, 3, b"https://example.com/a\r\nbc");
        assert_eq!(link_at(&term, at(0, 3)).unwrap().uri, "https://example.com/a");
        assert_eq!(link_at(&term, at(1, 0)), None);
    }

    #[test]
    fn wide_characters_keep_columns_right() {
        let term = term_with(40, 2, "漢字 https://example.com/漢 x".as_bytes());
        let link = link_at(&term, at(0, 8)).unwrap();
        assert_eq!(link.uri, "https://example.com/漢");
        // 漢字 and a space take columns 0..=4; the URL's last char is wide.
        assert_eq!(link.spans, vec![(Line(0), Column(5), Column(26))]);
        // The spacer half of the wide char belongs to the link too.
        assert_eq!(link_at(&term, at(0, 26)).unwrap().uri, "https://example.com/漢");
    }

    #[test]
    fn osc8_link_wins_and_covers_its_cells() {
        let term = term_with(
            40,
            3,
            b"\x1b]8;id=a;https://a.example\x1b\\https://b.example\x1b]8;;\x1b\\ x\r\n\x1b]8;id=a;https://a.example\x1b\\more\x1b]8;;\x1b\\",
        );
        let link = link_at(&term, at(0, 3)).unwrap();
        assert_eq!(link.uri, "https://a.example");
        assert_eq!(link.spans, vec![(Line(0), Column(0), Column(16)), (Line(1), Column(0), Column(3))]);
        assert_eq!(link_at(&term, at(0, 18)), None);
    }

    #[test]
    fn osc8_links_without_ids_are_separate() {
        let term = term_with(
            40,
            2,
            b"\x1b]8;;https://a.example\x1b\\one\x1b]8;;\x1b\\ \x1b]8;;https://a.example\x1b\\two\x1b]8;;\x1b\\",
        );
        let link = link_at(&term, at(0, 1)).unwrap();
        assert_eq!(link.spans, vec![(Line(0), Column(0), Column(2))]);
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
