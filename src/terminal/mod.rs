//! The terminal model: an alacritty `Term` grid plus the backend feeding it.
//!
//! The backends:
//! - `Server` (Linux and Windows): a session of a terminal server, on this
//!   machine or at a remote endpoint (its [`Host`], through
//!   [`crate::remote::local`]), whose output the connection's reader thread
//!   parses. Every tab with a program uses it. What assumes the program
//!   runs here (its pid, its directory checked on disk, its console window)
//!   is for [`Host::Local`] only; a remote session's program is what its
//!   host reports.
//! - `Remote`: a read-only message (an error shown in a tab), fed in with
//!   [`Terminal::feed`]; it has no program, so input goes nowhere.

pub mod cmdline;
pub mod events;
pub mod keys;
pub mod links;
pub mod mouse;
pub mod pace;
pub mod process;
pub mod search;
#[cfg(any(target_os = "linux", windows))]
mod server;
// Moved to `brindle-term-core`, which the server core shares.
pub use brindle_term_core::emulator::MIN_COLS;
pub use brindle_term_core::{shell_integration, status};

use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event as AlacEvent, EventListener, WindowSize};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point as GridPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{self, ClipboardType, Term, TermMode};
use alacritty_terminal::vte::ansi::{self, NamedColor, Rgb};
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use futures::StreamExt as _;
use gpui::{ClipboardItem, Context, EventEmitter, Task};
use parking_lot::Mutex;

use self::events::{EventGate, Merge, Recovered};
use self::pace::Pace;
use self::shell_integration::{Marks, Parser};

use crate::config::{Config, CursorShapeConfig, Profile};
use crate::latency::Latency;
use crate::notify::osc::{Found, Notice};
use crate::remote::Host;
use crate::theme::{Color, Theme};

/// What reaches the UI thread from the emulator and output scanner.
pub enum ChannelEvent {
    Alac(AlacEvent),
    /// A sequence the scanner found in the output (on the thread reading it).
    #[cfg_attr(not(any(target_os = "linux", windows)), allow(dead_code))]
    Osc(Found),
    /// A replay of a server session starts (`true`) or ends. In between,
    /// the state updates but no attention is raised.
    #[cfg_attr(not(any(target_os = "linux", windows)), allow(dead_code))]
    Replay(bool),
    /// The server session can't be got back; the reason is on the screen.
    #[cfg_attr(not(any(target_os = "linux", windows)), allow(dead_code))]
    Lost,
    /// The session never started or attached; the reason is on the screen.
    #[cfg_attr(not(any(target_os = "linux", windows)), allow(dead_code))]
    Failed,
    /// A remote session's host reported another foreground program, which
    /// a title may show.
    #[cfg_attr(not(any(target_os = "linux", windows)), allow(dead_code))]
    Foreground,
    /// A merged event's marker: its latest value, if it has one, is in the
    /// gate's slot ([`events::EventGate`]).
    Merged(Merge),
    /// Brindle's emulator failed on the session's output, saying this: the
    /// session was let go, detached, and the tab can't show it any more.
    #[cfg_attr(not(any(target_os = "linux", windows)), allow(dead_code))]
    Broken(String),
    /// Input was refused, the first time since input was last taken: too
    /// much of the tab's earlier input still waits to reach its session.
    #[cfg_attr(not(any(target_os = "linux", windows)), allow(dead_code))]
    InputRefused,
}

/// Where a terminal's events go: through its [`EventGate`] onto the
/// channel its event task drains, on whichever thread produced them (the
/// connection's reader while it parses, or the UI thread).
#[derive(Clone)]
pub struct Listener {
    tx: UnboundedSender<ChannelEvent>,
    output: Arc<OutputMarker>,
    /// A leaf lock: taken last, with the parser and term locks held when
    /// alacritty sends mid-parse, and nothing taken under it.
    gate: Arc<Mutex<EventGate>>,
}

impl Listener {
    /// A listener, and the channel its events arrive on.
    fn channel() -> (Self, UnboundedReceiver<ChannelEvent>) {
        let (tx, rx) = unbounded();
        let listener = Self { tx, output: Arc::default(), gate: Arc::default() };
        (listener, rx)
    }

    /// Queues `event` for the UI thread as the gate says: a merged one's
    /// marker only when none waits, a side one dropped past the gate's
    /// marks. The channel is unbounded; the gate is what bounds it.
    pub fn send(&self, event: ChannelEvent) {
        let mut gate = self.gate.lock();
        if let Some(event) = gate.pass(event) {
            // Under the gate's lock, so the channel's order is the gate's.
            self.tx.unbounded_send(event).ok();
        }
    }
}

impl EventListener for Listener {
    fn send_event(&self, event: AlacEvent) {
        if matches!(event, AlacEvent::Wakeup) {
            // On the thread that parsed new output, right after it did.
            self.output.advance(Instant::now());
        }
        self.send(ChannelEvent::Alac(event));
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

impl GridSize {
    /// This size with at least [`MIN_COLS`] columns, the narrowest grid the
    /// client and the server's emulators make. Where a size comes from
    /// outside the term (a layout, a listing), it goes through here before
    /// it is used.
    pub fn clamped(self) -> Self {
        Self { cols: self.cols.max(MIN_COLS), ..self }
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
    /// A session of `host`'s terminal server.
    #[cfg(any(target_os = "linux", windows))]
    Server { session: crate::remote::local::Session, closing: Closing, host: Host },
    /// A read-only message with no program behind it.
    Remote,
}

/// What happens to a terminal's program, local or remote, when Brindle
/// goes away without closing it (quit, a crash, the desktop stopping
/// Brindle). The user closing the terminal always ends it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Closing {
    /// It keeps running, detached, and the next launch attaches it.
    Keeps,
    /// It ends: a command given instead of the profile's program (`-e`),
    /// which is never restored or reattached.
    Ends,
}

impl Closing {
    pub fn new(command_override: bool) -> Self {
        if command_override { Self::Ends } else { Self::Keeps }
    }

    #[cfg(any(target_os = "linux", windows))]
    fn policy(self) -> crate::remote::protocol::client::Policy {
        match self {
            Self::Ends => crate::remote::protocol::client::Policy::Kill,
            Self::Keeps => crate::remote::protocol::client::Policy::Detach,
        }
    }
}

pub struct Terminal {
    term: Arc<FairMutex<Term<Listener>>>,
    /// What `term` was made with, for the one that replaces it when
    /// Brindle's emulator fails ([`Terminal::broken`]).
    term_config: term::Config,
    backend: Backend,
    /// The term's listener: the UI thread's own events (a refused input's
    /// line) go through it too.
    listener: Listener,
    output: Arc<OutputMarker>,
    /// The vte processor and OSC 133 scanner. Locked before `term`; a server
    /// session's reader thread uses it too (see `server.rs`).
    parser: Arc<Mutex<Parser>>,
    /// Shell-integration marks. Locked after `term`.
    marks: Arc<Mutex<Marks>>,
    size: GridSize,
    title: Option<String>,
    exited: bool,
    /// It shows a detached session it attached ([`Terminal::attach`]).
    attaching: bool,
    /// Its session never started or attached; it shows why.
    failed: bool,
    /// Between a server replay's start and end: events that would raise
    /// attention are dropped ([`quiet`]).
    replaying: bool,
    /// Program status records (OSC 7501).
    status: status::Records,
    pub theme: Theme,
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
    /// Drains the terminal's events in chunks, paced ([`pace`]), and applies
    /// them in order. The gate learns once per chunk what was taken, before
    /// its markers read their slots, so a value put after that has a marker
    /// of its own.
    fn spawn_events(rx: UnboundedReceiver<ChannelEvent>, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            let mut rx = rx.ready_chunks(1024);
            let mut pace = Pace::default();
            while let Some(chunk) = rx.next().await {
                pace.wait(cx).await;
                let alive = this.update(cx, |this, cx| {
                    pace::drain(&mut rx, chunk, |chunk| {
                        let recovered = this.listener.gate.lock().took(chunk.iter().map(events::kind_of));
                        for event in chunk {
                            this.apply(event, cx);
                        }
                        if let Some(recovered) = recovered {
                            this.recovered(recovered, cx);
                        }
                    });
                    cx.notify();
                });
                if alive.is_err() {
                    break;
                }
                pace.updated();
            }
        })
    }

    /// Acts on one event from the channel.
    fn apply(&mut self, event: ChannelEvent, cx: &mut Context<Self>) {
        match event {
            ChannelEvent::Alac(event) => self.handle_event(event, cx),
            ChannelEvent::Osc(found) => self.handle_found(found, cx),
            ChannelEvent::Merged(merge) => self.merged(merge, cx),
            // Never queued as itself: the gate turns it into its marker.
            ChannelEvent::Foreground => self.merged(Merge::Foreground, cx),
            ChannelEvent::Replay(active) => self.replaying = active,
            ChannelEvent::Lost => self.mark_lost(cx),
            ChannelEvent::Failed => {
                self.failed = true;
                self.mark_lost(cx);
            }
            ChannelEvent::Broken(reason) => self.broken(&reason, cx),
            ChannelEvent::InputRefused => self.show(INPUT_REFUSED, cx),
        }
    }

    /// A merged event's marker: its slot's latest value, if any.
    fn merged(&mut self, merge: Merge, cx: &mut Context<Self>) {
        match merge {
            Merge::Wakeup => self.handle_event(AlacEvent::Wakeup, cx),
            Merge::Title => {
                let title = self.listener.gate.lock().take_title();
                if let Some(title) = title {
                    self.set_title(title, cx);
                }
            }
            Merge::Foreground => cx.emit(TerminalEvent::TitleChanged),
            Merge::Clipboard(ty) => {
                let text = self.listener.gate.lock().take_clipboard(ty);
                if let Some(text) = text {
                    self.store_clipboard(ty, text, cx);
                }
            }
        }
    }

    /// The gate dropped side events while the UI thread was behind, and is
    /// back under its marks. A dropped status report leaves the records
    /// stale, so they start over and the tab's dot is recomputed.
    fn recovered(&mut self, recovered: Recovered, cx: &mut Context<Self>) {
        let Recovered { dropped, statuses } = recovered;
        log::warn!(
            "a terminal's events came faster than the window took them: {dropped} were dropped{}",
            if statuses { ", status reports among them, so its status starts over" } else { "" }
        );
        if statuses {
            self.status.reset();
            cx.emit(TerminalEvent::Status(None));
        }
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "private, built only by the constructors below from values each already has (the term, the listener and its event channel, parser, marks and backend); a parameter struct would be built just to be unpacked here"
    )]
    fn new(
        term: Arc<FairMutex<Term<Listener>>>,
        listener: Listener,
        rx: UnboundedReceiver<ChannelEvent>,
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
            term_config: term_config(config),
            backend,
            output: listener.output.clone(),
            listener,
            parser,
            marks,
            size,
            title: None,
            exited: false,
            attaching: false,
            failed: false,
            replaying: false,
            status: status::Records::default(),
            theme,
            copy_on_select: config.copy_on_select,
            last_activity: Instant::now(),
            sync_flush: None,
        }
    }

    /// Starts `profile`'s program (or `command_override`) in a new session
    /// of `host`'s terminal server: the host of the tab it opens in, which
    /// stays its own whatever the profile targets later. It returns at
    /// once; if the session can't start, the terminal shows why.
    pub fn spawn(
        profile: &Profile,
        host: &Host,
        command_override: Option<(String, Vec<String>)>,
        cwd_override: Option<std::path::PathBuf>,
        config: &Config,
        cx: &mut Context<Self>,
    ) -> Self {
        let theme = config.profile_theme(profile);
        #[cfg(any(target_os = "linux", windows))]
        {
            // A command given instead of the profile's program (`-e`) is
            // never restored or reattached, so it ends with its tab and with
            // Brindle. Everything else keeps running when Brindle goes away
            // without closing it.
            let closing = Closing::new(command_override.is_some());
            let (request, doing) = spawn_request(profile, host, command_override, cwd_override, config);
            Self::start_session(host, request, doing, closing, theme, config, cx)
        }
        #[cfg(not(any(target_os = "linux", windows)))]
        {
            let _ = (host, command_override, cwd_override);
            log::error!("failed to start a terminal: terminals run in the terminal server, which needs Linux or Windows");
            let text = "\x1b[31mBrindle could not start a terminal:\x1b[0m\r\n\
                        terminals run in the terminal server, which needs Linux or Windows\r\n";
            Self::message(text, theme, config, cx)
        }
    }

    /// A terminal whose session `request` starts on `host`, in the
    /// background ([`crate::remote::local::Session::start`]).
    #[cfg(any(target_os = "linux", windows))]
    #[allow(
        clippy::too_many_arguments,
        reason = "private, called by `spawn` with what it already holds; a parameter struct would be a one-use type"
    )]
    fn start_session(
        host: &Host,
        request: crate::remote::local::SpawnRequest,
        doing: String,
        closing: Closing,
        theme: Theme,
        config: &Config,
        cx: &mut Context<Self>,
    ) -> Self {
        let (listener, rx) = Listener::channel();
        let size = GridSize::default();
        let term = Arc::new(FairMutex::new(Term::new(term_config(config), &size, listener.clone())));
        let parser = Arc::new(Mutex::new(parser_for(host)));
        let marks = Arc::new(Mutex::new(Marks::default()));
        let sink = server::Sink::new(term.clone(), parser.clone(), marks.clone(), listener.clone(), doing);
        let session = crate::remote::local::Session::start(host.clone(), request, Box::new(sink));
        let backend = Backend::Server { session, closing, host: host.clone() };
        Self::new(term, listener, rx, backend, parser, marks, size, theme, config, cx)
    }

    /// Attaches a detached session of `host`'s terminal server: its replay,
    /// quiet (no bell or notification), then its live output. It closes as
    /// a spawned profile terminal does. Returns at once, like a spawn; a
    /// refused attach shows in the terminal, which then only shows that
    /// message ([`Terminal::shows_only_message`]). A session a restored
    /// zone recorded (`Attaching::Recorded`) is attached once its endpoint's
    /// listing confirms it, at the size listed; meanwhile a remote one says
    /// it is connecting, until the replay's `ESC c` clears the line.
    #[cfg(any(target_os = "linux", windows))]
    pub fn attach(
        host: &Host,
        request: crate::remote::local::AttachRequest,
        what: String,
        profile: &Profile,
        config: &Config,
        cx: &mut Context<Self>,
    ) -> Self {
        let (listener, rx) = Listener::channel();
        // The session's size: the replay is rendered at it, and the first
        // layout resizes the session only if the window's grid differs. A
        // recorded session's comes with its listing.
        let (size, recorded) = match request.when {
            crate::remote::local::Attaching::Now { cols, rows } => {
                (GridSize { cols, rows, ..GridSize::default() }, false)
            }
            crate::remote::local::Attaching::Recorded => (GridSize::default(), true),
        };
        let term = Arc::new(FairMutex::new(Term::new(term_config(config), &size, listener.clone())));
        let parser = Arc::new(Mutex::new(parser_for(host)));
        let marks = Arc::new(Mutex::new(Marks::default()));
        if !host.is_local() {
            // Before the session's thread starts, so before any replay.
            let line = format!("\x1b[2mconnecting to {}…\x1b[22m\r\n", host.name());
            parser.lock().advance(&mut *term.lock_unfair(), &marks, line.as_bytes());
        }
        let doing = if recorded {
            format!("restore session {} on {}", request.session, host.name())
        } else {
            format!("attach session {} ({what})", request.session)
        };
        let mut sink = server::Sink::new(term.clone(), parser.clone(), marks.clone(), listener.clone(), doing);
        if !recorded {
            sink = sink.replayed_at(size);
        }
        let session = crate::remote::local::Session::attach(host.clone(), request, Box::new(sink));
        let backend = Backend::Server { session, closing: Closing::Keeps, host: host.clone() };
        let theme = config.profile_theme(profile);
        let mut this = Self::new(term, listener, rx, backend, parser, marks, size, theme, config, cx);
        this.attaching = true;
        this
    }

    /// A read-only terminal showing `text` (used to report errors in a tab).
    pub fn message(text: &str, theme: Theme, config: &Config, cx: &mut Context<Self>) -> Self {
        let (listener, rx) = Listener::channel();
        let size = GridSize::default();
        let term = Arc::new(FairMutex::new(Term::new(term_config(config), &size, listener.clone())));
        let parser = Arc::new(Mutex::new(Parser::new(process::LOCAL_PATH_STYLE)));
        let marks = Arc::new(Mutex::new(Marks::default()));
        let mut this = Self::new(term, listener, rx, Backend::Remote, parser, marks, size, theme, config, cx);
        this.feed(text.as_bytes(), cx);
        this
    }

    /// Feeds a message terminal its text. A server session's output is
    /// parsed on its connection's reader thread instead, so it is ignored
    /// here.
    pub fn feed(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        #[cfg(any(target_os = "linux", windows))]
        if let Backend::Server { .. } = self.backend {
            return;
        }
        {
            let mut parser = self.parser.lock();
            let mut term = self.term.lock_unfair();
            parser.advance(&mut *term, &self.marks, bytes);
            self.output.advance(Instant::now());
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

    /// The server session can't be got back (its line is on the screen): the
    /// terminal has ended, but the tab stays open on its last screen, so no
    /// `Exited`.
    fn mark_lost(&mut self, cx: &mut Context<Self>) {
        self.replaying = false;
        if !self.exited {
            self.exited = true;
            if self.status.end_activity() {
                cx.emit(TerminalEvent::Status(None));
            }
        }
    }

    /// The program status records (OSC 7501).
    pub fn status(&self) -> &status::Records {
        &self.status
    }

    /// Emits an event the output caused, quietly during a replay.
    fn emit_from_output(&mut self, event: TerminalEvent, cx: &mut Context<Self>) {
        let event = if self.replaying { quiet(event) } else { Some(event) };
        if let Some(event) = event {
            cx.emit(event);
        }
    }

    /// Acts on a sequence the scanner found in the output, in stream order.
    fn handle_found(&mut self, found: Found, cx: &mut Context<Self>) {
        let changed = match found {
            Found::Notice(notice) => {
                self.emit_from_output(TerminalEvent::Notify(notice), cx);
                return;
            }
            Found::Status(status::Message::Query) => {
                self.write(status::QUERY_REPLY);
                return;
            }
            Found::Status(message) => {
                let update = self.status.apply(&message);
                if update.changed {
                    self.emit_from_output(TerminalEvent::Status(update.notice), cx);
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

    #[allow(
        clippy::wildcard_enum_match_arm,
        reason = "foreign enum: alacritty_terminal::event::Event, of which the rest need no action here"
    )]
    fn handle_event(&mut self, event: AlacEvent, cx: &mut Context<Self>) {
        match event {
            AlacEvent::Wakeup => {
                self.last_activity = Instant::now();
                Latency::output(cx.entity_id(), self.output.generation(), self.output.at(), cx);
                self.schedule_sync_flush(cx);
            }
            AlacEvent::Title(title) => self.set_title(Some(title), cx),
            AlacEvent::ResetTitle => self.set_title(None, cx),
            AlacEvent::ClipboardStore(kind, text) => self.store_clipboard(kind, text, cx),
            // Only sent when the config allows OSC 52 reads (see `term_config`).
            AlacEvent::ClipboardLoad(kind, format) => {
                let item = match kind {
                    ClipboardType::Clipboard => cx.read_from_clipboard(),
                    ClipboardType::Selection => crate::primary::read_primary(cx),
                };
                let text = item.and_then(|i| i.text()).unwrap_or_default();
                self.write(format(&text).into_bytes());
            }
            AlacEvent::ColorRequest(index, format) => {
                let set = self.term.lock().colors()[index];
                self.write(format(set.unwrap_or_else(|| self.default_color(index))).into_bytes());
            }
            AlacEvent::PtyWrite(text) => self.write(text.into_bytes()),
            AlacEvent::TextAreaSizeRequest(format) => self.write(format(self.size.into()).into_bytes()),
            AlacEvent::Bell => self.emit_from_output(TerminalEvent::Bell, cx),
            AlacEvent::Exit | AlacEvent::ChildExit(_) => self.mark_exited(cx),
            // Never queued: the gate ignores them (`events::kind_of`).
            AlacEvent::MouseCursorDirty | AlacEvent::CursorBlinkingChange => {}
        }
    }

    /// A program's clipboard write (OSC 52).
    fn store_clipboard(&mut self, kind: ClipboardType, text: String, cx: &mut Context<Self>) {
        let item = ClipboardItem::new_string(text);
        match kind {
            ClipboardType::Clipboard => cx.write_to_clipboard(item),
            ClipboardType::Selection => crate::primary::write_primary(cx, item),
        }
    }

    /// Writes Brindle's own `text` into the terminal as output is written,
    /// under parser → term → marks, so it lands between the program's reads.
    fn show(&mut self, text: &str, cx: &mut Context<Self>) {
        {
            let mut parser = self.parser.lock();
            let mut term = self.term.lock_unfair();
            parser.advance(&mut *term, &self.marks, text.as_bytes());
        }
        cx.notify();
    }

    /// Brindle's emulator failed on the session's output, which the
    /// connection let go, detached: the program runs on, but this tab
    /// can't show it. A panic in the middle of a `Term` update can leave
    /// its grid inconsistent, and painting it could end Brindle, so the
    /// emulator's state is replaced, not kept as a lost tab's last screen
    /// is. The tab stays open, saying why and how to get the session back,
    /// and counts as ended: nothing records it, and closing it sends
    /// nothing.
    fn broken(&mut self, reason: &str, cx: &mut Context<Self>) {
        let (fresh_parser, fresh_term) =
            (self.fresh_parser(), Term::new(self.term_config.clone(), &self.size, self.listener.clone()));
        {
            let mut parser = self.parser.lock();
            let mut term = self.term.lock_unfair();
            *parser = fresh_parser;
            *term = fresh_term;
            *self.marks.lock() = Marks::default();
        }
        self.status.reset();
        cx.emit(TerminalEvent::Status(None));
        self.show(&broken_line(reason), cx);
        self.mark_lost(cx);
    }

    /// A parser like the one the terminal was made with.
    fn fresh_parser(&self) -> Parser {
        match &self.backend {
            #[cfg(any(target_os = "linux", windows))]
            Backend::Server { host, .. } => parser_for(host),
            Backend::Remote => Parser::new(process::LOCAL_PATH_STYLE),
        }
    }

    /// Theme color for an xterm color index (0-255 palette, then the named
    /// colors alacritty uses for foreground/background/cursor).
    pub fn default_color(&self, index: usize) -> Rgb {
        palette_color(&self.theme, index).into()
    }

    /// Writes raw bytes to the program: input, and the replies the emulator
    /// generates (device, color and size reports), all counted against the
    /// session's input budget. The first refusal of a crossing puts a line
    /// in the tab (through the event task, as this has no context); later
    /// ones add nothing. A message terminal has no program, so they go
    /// nowhere.
    pub fn write(&self, bytes: impl Into<Cow<'static, [u8]>>) {
        let bytes = bytes.into();
        if bytes.is_empty() {
            return;
        }
        match &self.backend {
            #[cfg(any(target_os = "linux", windows))]
            Backend::Server { session, .. } => {
                use crate::remote::budget::{Delivery, Refusal};
                match session.input(&bytes) {
                    Delivery::Refused(Refusal::First) => self.listener.send(ChannelEvent::InputRefused),
                    Delivery::Queued | Delivery::Refused(Refusal::Again) | Delivery::Discarded => {}
                }
            }
            Backend::Remote => {}
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
        if size.cols == 0 || size.rows == 0 {
            return;
        }
        let size = size.clamped();
        if size == self.size {
            return;
        }
        self.size = size;
        let mut term = self.term.lock();
        self.marks.lock().resize(&mut *term, size);
        drop(term);
        match &self.backend {
            #[cfg(any(target_os = "linux", windows))]
            Backend::Server { session, .. } => {
                session.resize(size.cols, size.rows, size.cell_width, size.cell_height)
            }
            Backend::Remote => {}
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
    /// titles. A session on this machine is looked up by its pid, in
    /// `snapshot` (the title tick's look at all processes, taken the first
    /// time a terminal needs it); a remote one's is what its host last
    /// reported, since its pid is another machine's.
    pub fn foreground_process_name(&self, snapshot: &mut Option<process::Snapshot>) -> Option<String> {
        match &self.backend {
            #[cfg(any(target_os = "linux", windows))]
            Backend::Server { session, host: Host::Local, .. } => {
                let pid = session.pid()?;
                process::PtyProcess::new(pid).foreground_name(snapshot.get_or_insert_with(process::Snapshot::take))
            }
            #[cfg(any(target_os = "linux", windows))]
            Backend::Server { session, .. } => session.foreground().and_then(|foreground| foreground.name),
            Backend::Remote => None,
        }
    }

    /// Parents the tab's console window to `window`, so dialogs that console
    /// programs open (credential prompts, for one) belong to Brindle's
    /// window. Tabs never move between windows, so once is enough: the
    /// session sends `REPARENT` once it has started or attached, and again
    /// after each reconnect's replay, since a new core doesn't know the
    /// window. Only a session on this machine has a console here: a remote
    /// one's host is Linux, whose core refuses `REPARENT`.
    #[cfg(windows)]
    pub fn parent_console_to(&self, window: &gpui::Window) {
        let Backend::Server { session, host: Host::Local, .. } = &self.backend else { return };
        let Some(hwnd) = crate::win::hwnd(window) else { return };
        session.reparent(hwnd.0 as usize as u64);
    }

    /// Current working directory of the foreground process (falls back to
    /// the shell). On Windows, where no other process's directory can be
    /// read, it is the directory the program last reported (OSC 7 or 9;9),
    /// if it still exists. A remote session's is a path on its host, never
    /// checked here: the directory the program reported (OSC 7), else the
    /// one its host last reported.
    pub fn working_directory(&self) -> Option<std::path::PathBuf> {
        match &self.backend {
            #[cfg(windows)]
            Backend::Server { host: Host::Local, .. } => {
                self.marks.lock().reported_dir().filter(|dir| dir.is_dir()).map(Into::into)
            }
            #[cfg(target_os = "linux")]
            Backend::Server { session, host: Host::Local, .. } => {
                session.pid().and_then(|pid| process::PtyProcess::new(pid).working_directory())
            }
            #[cfg(any(target_os = "linux", windows))]
            Backend::Server { session, .. } => self
                .marks
                .lock()
                .reported_dir()
                .map(Into::into)
                .or_else(|| session.foreground()?.cwd.map(Into::into)),
            Backend::Remote => None,
        }
    }

    /// The visible screen as plain text (used by `--dump-screen` and tests).
    pub fn screen_text(&self) -> String {
        screen_text(&self.term.lock())
    }

    /// The user closed the terminal: its program ends. Once is enough:
    /// later calls, and the drop, do nothing.
    pub fn close(&mut self) {
        self.leave(true, None);
    }

    /// Brindle is quitting: the program is left as the terminal's
    /// [`Closing`] says (a profile tab's session is detached, an `-e`
    /// one's ends), as the drop would. It is done explicitly because the
    /// drop can't be relied on: on Windows GPUI keeps the focused
    /// terminal's view alive past the process's exit. Once is enough:
    /// later calls, and the drop, do nothing. A kept session is detached
    /// with `layout`, the layout of the tab it was in (`crate::layout`).
    pub fn quit_with(&mut self, layout: Option<serde_json::Value>) {
        self.leave(false, layout);
    }

    /// Ends the terminal's connection to its program: `close` ends the
    /// program; otherwise a server session's program is left as its
    /// [`Closing`] says. A message terminal has no program.
    /// Keeps the layout of the tab this terminal is in current on its
    /// server session (`crate::layout`), as it changes; a message terminal
    /// has none.
    pub fn set_layout(&self, layout: Option<serde_json::Value>) {
        match &self.backend {
            #[cfg(any(target_os = "linux", windows))]
            Backend::Server { session, closing: Closing::Keeps, .. } if !self.exited => session.set_layout(layout),
            _ => {
                let _ = layout;
            }
        }
    }

    fn leave(&mut self, close: bool, layout: Option<serde_json::Value>) {
        match &self.backend {
            #[cfg(any(target_os = "linux", windows))]
            Backend::Server { session, closing, .. } => match (close, closing) {
                (false, Closing::Keeps) => session.detach_with(layout),
                _ => session.close(),
            },
            Backend::Remote => {}
        }
        let _ = close;
        #[cfg(not(any(target_os = "linux", windows)))]
        let _ = layout;
    }

    /// The terminal server session (id and key, `None` from a core that
    /// predates keys) this terminal shows, once it exists and while its
    /// program runs; an attach reports the session it asked for until it is
    /// answered.
    #[cfg(any(target_os = "linux", windows))]
    pub fn server_session(
        &self,
    ) -> Option<(crate::remote::protocol::SessionId, Option<crate::remote::SessionKey>)> {
        match &self.backend {
            Backend::Server { session, .. } if !self.exited => session.id_and_key(),
            Backend::Server { .. } | Backend::Remote => None,
        }
    }

    /// An attach that failed (refused, or its session gone): the terminal
    /// only says why, and session restore doesn't record it.
    pub fn shows_only_message(&self) -> bool {
        self.attaching && self.failed
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        self.leave(false, None);
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

/// The color for an xterm palette index or one of the named colors
/// alacritty uses after it (foreground, background, cursor, dim colors).
fn palette_color(t: &Theme, index: usize) -> Color {
    let (fg, bg) = (t.foreground, t.background);
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

/// What a server replay may emit. A replay recreates a session's state, so
/// it may change status dots and titles, but it raises no attention: no
/// bell, no notification, and no status notice.
fn quiet(event: TerminalEvent) -> Option<TerminalEvent> {
    match event {
        TerminalEvent::Bell | TerminalEvent::Notify(_) => None,
        TerminalEvent::Status(Some(_)) => Some(TerminalEvent::Status(None)),
        other @ (TerminalEvent::TitleChanged | TerminalEvent::Exited | TerminalEvent::Status(_)) => Some(other),
    }
}

/// How far dim text and dim palette colors fade towards the background.
pub const DIM_FACTOR: f32 = 0.4;

/// What a tab says, once per crossing of its input budget, when input is
/// refused ([`crate::remote::budget`]).
const INPUT_REFUSED: &str =
    "\r\n\x1b[2minput not sent: 4 MiB of earlier input is still waiting to reach this session\x1b[22m\r\n";

/// What a tab whose emulator failed says in place of its screen.
fn broken_line(reason: &str) -> String {
    format!(
        "\x1b[31mBrindle can't show this session any more:\x1b[0m {}\r\n\
         It keeps running, detached: attach it again from Attach Session…\r\n",
        reason.replace('\n', "\r\n")
    )
}

/// What a new terminal of `profile` on `host` asks that host's server to
/// run, and what starting it is called in an error. A remote host's
/// session is left to it: its login shell when the profile names no
/// command, a directory as given (its holder expands `~` and falls back to
/// home), and its own environment under the profile's
/// ([`SpawnRequest::remote`](crate::remote::local::SpawnRequest::remote)).
/// This machine's is resolved here: the program, a directory that exists,
/// and Brindle's own environment under the profile's. Which one depends on
/// `host` alone, not on what the profile targets.
#[cfg(any(target_os = "linux", windows))]
pub(crate) fn spawn_request(
    profile: &Profile,
    host: &Host,
    command_override: Option<(String, Vec<String>)>,
    cwd_override: Option<std::path::PathBuf>,
    config: &Config,
) -> (crate::remote::local::SpawnRequest, String) {
    let policy = Closing::new(command_override.is_some()).policy();
    let size = GridSize::default();
    let scrollback = u32::try_from(config.scrollback_lines).unwrap_or(u32::MAX);
    if !host.is_local() {
        let request =
            crate::remote::local::SpawnRequest::remote(profile, command_override, cwd_override, size, scrollback, policy);
        let doing = match request.command.as_str() {
            "" => format!("open a shell on {}", host.name()),
            command => format!("start {command:?} on {}", host.name()),
        };
        return (request, doing);
    }
    // The program is always resolved here (on Windows from `PATH` and
    // `%COMSPEC%`), so `SPAWN` never leaves it to the server.
    let (program, args) = command_override.unwrap_or_else(|| match &profile.command {
        Some(program) => (program.clone(), profile.args.clone()),
        None => default_program(&profile.args),
    });
    let home = dirs::home_dir();
    let working_directory = cwd_override
        .or_else(|| profile.working_directory(home.as_deref()))
        .or(home)
        .filter(|dir| dir.is_dir());
    let doing = format!("start {program:?}");
    let request = crate::remote::local::SpawnRequest {
        command: program,
        args,
        cwd: working_directory.map(|dir| dir.to_string_lossy().into_owned()),
        env: server_env(std::env::vars_os(), profile),
        cols: size.cols,
        rows: size.rows,
        cell_w: size.cell_width,
        cell_h: size.cell_height,
        scrollback,
        policy,
        profile: Some(profile.name.clone()),
    };
    (request, doing)
}

/// The parser for a terminal on `host`, reading directory reports in the
/// host's path style ([`Host::path_style`]).
#[cfg(any(target_os = "linux", windows))]
fn parser_for(host: &Host) -> Parser {
    Parser::new(host.path_style(process::LOCAL_PATH_STYLE))
}

/// The program a profile with no `command` runs, with the profile's `args`:
/// `$SHELL`, then `/bin/sh` (`cmdline::unix_default_program`).
#[cfg(unix)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn default_program(args: &[String]) -> (String, Vec<String>) {
    cmdline::unix_default_program(std::env::var("SHELL").ok(), args)
}

/// The program a profile with no `command` runs, with the profile's `args`
/// after its own: PowerShell 7, Windows PowerShell, `%COMSPEC%` or `cmd.exe`.
#[cfg(windows)]
fn default_program(args: &[String]) -> (String, Vec<String>) {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let comspec = std::env::var("COMSPEC").ok();
    let (program, mut own) = cmdline::windows_default_program(
        std::env::split_paths(&path),
        comspec.as_deref(),
        |path: &std::path::Path| path.is_file(),
    );
    own.extend_from_slice(args);
    (program, own)
}

/// Environment for programs started by Brindle.
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

/// The whole environment of a local server session's program: `vars`
/// (Brindle's own environment) with [`child_env`] applied over it. The
/// server adds nothing, so this is what the program sees. Variables that
/// aren't UTF-8 can't travel in `SPAWN` and are left out, as are the
/// startup-notification tokens that were for Brindle's own launch.
///
/// On Windows names are compared case-insensitively, as Windows does: a
/// profile's `path` replaces Brindle's `Path`, under the profile's spelling.
#[cfg_attr(not(any(target_os = "linux", windows)), allow(dead_code))]
pub fn server_env(
    vars: impl IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
    profile: &Profile,
) -> std::collections::BTreeMap<String, String> {
    merged_env(vars, child_env(profile), cfg!(windows))
}

/// `vars` with `set` over them; see [`server_env`].
fn merged_env(
    vars: impl IntoIterator<Item = (std::ffi::OsString, std::ffi::OsString)>,
    set: HashMap<String, String>,
    ignore_case: bool,
) -> std::collections::BTreeMap<String, String> {
    let mut env: std::collections::BTreeMap<String, String> = vars
        .into_iter()
        .filter_map(|(key, value)| Some((key.into_string().ok()?, value.into_string().ok()?)))
        .collect();
    for key in ["XDG_ACTIVATION_TOKEN", "DESKTOP_STARTUP_ID"] {
        env.remove(key);
    }
    if ignore_case {
        return cmdline::merge_environment(env.into_iter().collect(), set.into_iter().collect()).into_iter().collect();
    }
    env.extend(set);
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

/// Sends focus in/out reports when the app asked for them (mode 1004).
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
    fn a_grid_size_narrower_than_min_cols_is_clamped_and_its_rows_kept() {
        let narrow = GridSize { cols: 1, rows: 7, cell_width: 9, cell_height: 18 };
        assert_eq!(narrow.clamped(), GridSize { cols: MIN_COLS, rows: 7, cell_width: 9, cell_height: 18 });
        let wide = GridSize { cols: 120, ..narrow };
        assert_eq!(wide.clamped(), wide);
    }

    /// 6.1/6.2: what a new terminal runs follows the host it opens on, not
    /// what its profile targets. A remote host's session is left to it (the
    /// directory unchecked here); this machine's is resolved here, a
    /// directory that isn't here dropped. A profile tab detaches, `-e` kills.
    #[cfg(any(target_os = "linux", windows))]
    #[test]
    fn a_spawn_request_follows_its_host() {
        use crate::config::RemoteTarget;
        use crate::remote::protocol::client::Policy;

        let a8 = RemoteTarget::Ssh { host: "A8".into(), user: None, port: None, identity_file: None, ssh_args: Vec::new(), connect_at_launch: false };
        let profile = Profile {
            name: "htop".into(),
            command: Some("htop".into()),
            cwd: Some("/var/log/brindle-nowhere".into()),
            remote: Some(a8.clone()),
            ..Default::default()
        };
        let config = Config::default();

        let (remote, doing) = spawn_request(&profile, &Host::Remote(a8.clone()), None, None, &config);
        assert_eq!((remote.command.as_str(), doing.as_str()), ("htop", "start \"htop\" on A8"));
        assert_eq!(remote.cwd.as_deref(), Some("/var/log/brindle-nowhere"));
        assert_eq!(remote.policy, Policy::Detach);

        let (local, doing) = spawn_request(&profile, &Host::Local, None, None, &config);
        assert_eq!((local.command.as_str(), doing.as_str()), ("htop", "start \"htop\""));
        assert_eq!(local.cwd, None);
        assert_eq!(local.policy, Policy::Detach);

        let shell = Profile { command: None, ..profile.clone() };
        let (_, doing) = spawn_request(&shell, &Host::Remote(a8.clone()), None, None, &config);
        assert_eq!(doing, "open a shell on A8");

        let (command, _) = spawn_request(&profile, &Host::Remote(a8), Some(("top".into(), vec!["-b".into()])), None, &config);
        assert_eq!((command.command.as_str(), command.args.as_slice()), ("top", ["-b".to_string()].as_slice()));
        assert_eq!(command.policy, Policy::Kill);
    }

    /// What reaches the event task through a listener: its waiting merged
    /// events once each, the latest title in the gate's slot.
    #[test]
    fn a_listener_queues_wakeups_and_titles_once_until_they_are_taken() {
        let (listener, mut rx) = Listener::channel();
        listener.send_event(AlacEvent::Wakeup);
        listener.send_event(AlacEvent::Title("one".into()));
        listener.send_event(AlacEvent::Bell);
        listener.send_event(AlacEvent::Wakeup);
        listener.send_event(AlacEvent::Title("two".into()));
        listener.send(ChannelEvent::Replay(true));
        let queued: Vec<ChannelEvent> = std::iter::from_fn(|| rx.try_next().ok().flatten()).collect();
        assert!(matches!(
            queued[..],
            [
                ChannelEvent::Merged(Merge::Wakeup),
                ChannelEvent::Merged(Merge::Title),
                ChannelEvent::Alac(AlacEvent::Bell),
                ChannelEvent::Replay(true)
            ]
        ));
        let mut gate = listener.gate.lock();
        assert_eq!(gate.took(queued.iter().map(events::kind_of)), None);
        assert_eq!(gate.take_title(), Some(Some("two".to_string())));
    }

    #[test]
    fn palette_colors_come_from_the_theme() {
        let t = Theme::default();
        assert_eq!(palette_color(&t, NamedColor::Foreground as usize), t.foreground);
        assert_eq!(palette_color(&t, NamedColor::Background as usize), t.background);
        assert_eq!(
            palette_color(&t, NamedColor::DimForeground as usize),
            t.foreground.mix(t.background, DIM_FACTOR)
        );
        assert_eq!(palette_color(&t, 1), t.ansi[1]);
        assert_eq!(palette_color(&t, NamedColor::Cursor as usize), t.cursor);
    }

    #[test]
    fn only_output_advances_the_marker() {
        let (listener, _rx) = Listener::channel();
        let marker = listener.output.clone();
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
    fn profile_terminals_outlive_brindle_and_commands_end_with_it() {
        assert_eq!(Closing::new(false), Closing::Keeps);
        // `-e` commands are never restored.
        assert_eq!(Closing::new(true), Closing::Ends);
        #[cfg(any(target_os = "linux", windows))]
        {
            use crate::remote::protocol::client::Policy;
            assert_eq!(Closing::Keeps.policy(), Policy::Detach);
            assert_eq!(Closing::Ends.policy(), Policy::Kill);
        }
    }

    #[test]
    fn a_replay_raises_no_attention_but_keeps_its_state() {
        use crate::notify::osc::Source;
        let notice = Notice { source: Source::Osc9, title: "t".into(), body: "b".into() };
        assert_eq!(quiet(TerminalEvent::Bell), None);
        assert_eq!(quiet(TerminalEvent::Notify(notice.clone())), None);
        assert_eq!(quiet(TerminalEvent::Status(Some(notice))), Some(TerminalEvent::Status(None)));
        assert_eq!(quiet(TerminalEvent::Status(None)), Some(TerminalEvent::Status(None)));
        assert_eq!(quiet(TerminalEvent::TitleChanged), Some(TerminalEvent::TitleChanged));
        assert_eq!(quiet(TerminalEvent::Exited), Some(TerminalEvent::Exited));
    }

    #[cfg(unix)]
    #[test]
    fn server_sessions_get_brindles_environment_with_the_profiles_over_it() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt as _;
        let profile = Profile {
            env: [("FOO".into(), "profile".into()), ("EXTRA".into(), "1".into())].into(),
            ..Profile::default_shell()
        };
        let own = [
            ("PATH", "/usr/bin"),
            ("FOO", "brindle"),
            ("TERM", "dumb"),
            ("XDG_ACTIVATION_TOKEN", "for-brindle"),
            ("DESKTOP_STARTUP_ID", "for-brindle"),
        ]
        .map(|(k, v)| (OsString::from(k), OsString::from(v)));
        let not_utf8 = (OsString::from("RAW"), OsString::from_vec(vec![0xff, 0xfe]));
        let env = server_env(own.into_iter().chain([not_utf8]), &profile);
        assert_eq!(env.get("PATH").map(String::as_str), Some("/usr/bin"));
        // The profile wins over Brindle's own, and Brindle's defaults apply.
        assert_eq!(env.get("FOO").map(String::as_str), Some("profile"));
        assert_eq!(env.get("EXTRA").map(String::as_str), Some("1"));
        assert_eq!(env.get("TERM").map(String::as_str), Some("xterm-256color"));
        assert_eq!(env.get("TERM_PROGRAM").map(String::as_str), Some("Brindle"));
        assert!(!env.contains_key("XDG_ACTIVATION_TOKEN") && !env.contains_key("DESKTOP_STARTUP_ID"));
        assert!(!env.contains_key("RAW"));
        // Without profile entries, Brindle's own value is what the program sees.
        let env = server_env([(OsString::from("FOO"), OsString::from("2"))], &Profile::default_shell());
        assert_eq!(env.get("FOO").map(String::as_str), Some("2"));
    }

    /// Windows compares names case-insensitively, so a profile's `path`
    /// replaces Brindle's `Path` instead of joining it.
    #[test]
    fn windows_sessions_get_one_value_per_name_whatever_its_case() {
        use std::ffi::OsString;
        let own = [("Path", "C:\\Windows"), ("TEMP", "C:\\t"), ("term", "dumb")]
            .map(|(key, value)| (OsString::from(key), OsString::from(value)));
        let set: HashMap<String, String> =
            [("path".to_string(), "C:\\tools".to_string()), ("TERM".to_string(), "xterm-256color".to_string())].into();
        let env = merged_env(own.clone(), set.clone(), true);
        let pairs: Vec<(&str, &str)> = env.iter().map(|(key, value)| (key.as_str(), value.as_str())).collect();
        assert_eq!(pairs, [("TEMP", "C:\\t"), ("TERM", "xterm-256color"), ("path", "C:\\tools")]);
        // Elsewhere they are different variables.
        assert_eq!(merged_env(own, set, false).len(), 5);
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
