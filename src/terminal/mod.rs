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
mod dir_check;
pub mod events;
pub mod keys;
pub mod links;
pub mod mouse;
pub mod pace;
pub mod process;
pub mod search;
pub mod tab_note;
#[cfg(any(target_os = "linux", windows))]
mod server;
/// The fault switch the server sink reads (`server`'s `inject_fault`), named
/// here so `--list-fault-switches` and the reader share one constant.
pub const SINK_PANIC: &str = "BRINDLE_TEST_SINK_PANIC";
// Moved to `brindle-term-core`, which the server core shares.
pub use brindle_term_core::emulator::MIN_COLS;
pub use brindle_term_core::{context, progress, shell_integration, status};

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
use self::context::{Badge, Contexts};
use self::pace::Pace;
use self::progress::Progress;
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
    /// This size within the limits every grid keeps, on the client and in
    /// the server's emulators alike (`emulator::clamped`): [`MIN_COLS`] to
    /// 2048 columns and 1 to 1024 rows. Where a size comes from outside the
    /// term (a layout, a listing), it goes through here before a grid is
    /// made or resized with it.
    pub fn clamped(self) -> Self {
        let (cols, rows) = brindle_term_core::emulator::clamped(self.cols, self.rows);
        Self { cols, rows, ..self }
    }

    /// The grid a session listed at `size` is attached at: its own size,
    /// so attaching resizes nothing, or the default grid when its listing
    /// gave none (`ListedSession::size`: a core that predates sizes, or a
    /// size outside the grid limits).
    pub fn listed(size: Option<(u16, u16)>) -> Self {
        size.map_or_else(Self::default, |(cols, rows)| Self { cols, rows, ..Self::default() }.clamped())
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
    /// Whether its program runs, ended, or never started; moved on only by
    /// [`Life::after`].
    life: Life,
    /// Whether it started its session or attached a detached one
    /// ([`Terminal::attach`]).
    origin: Origin,
    /// Notices ([`Terminal::note`]) due before the attached session's first
    /// replay ends, which would wipe them: `Some` until then, written when
    /// the replay ends, or after the line that says the attach failed or
    /// the session was lost.
    held_notes: Option<Vec<String>>,
    /// Between a server replay's start and end: events that would raise
    /// attention are dropped ([`quiet`]).
    replaying: bool,
    /// Program status records (OSC 7501).
    status: status::Records,
    /// The program's progress (OSC 9;4), until a clear report, a new
    /// prompt, a reset or the program's end.
    progress: Option<Progress>,
    /// The program's open contexts (OSC 3008), until they end, the program
    /// ends, or a replay begins (which rebuilds them).
    contexts: Contexts,
    /// `contexts`' notable context, recomputed whenever `contexts` changes,
    /// so the chrome never computes it while drawing.
    badge: Option<Badge>,
    /// Whether the directory the program reports exists, where that is the
    /// terminal's directory ([`Terminal::follows_reported_dir`]). It is
    /// refreshed when output brings another report, by a check off the UI
    /// thread ([`Terminal::follow_reported_dir`]).
    dir_check: dir_check::DirCheck,
    pub theme: Theme,
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
            ChannelEvent::Replay(active) => {
                self.replaying = active;
                // A replay's `ESC c` ends no context, and it carries the
                // session's open ones, so the old stack goes first.
                if active && self.clear_contexts() {
                    cx.emit(TerminalEvent::Status(None));
                }
                if !active {
                    self.release_notes(cx);
                }
            }
            ChannelEvent::Lost => {
                self.mark_lost(cx);
                self.release_notes(cx);
            }
            ChannelEvent::Failed => {
                self.mark_failed(cx);
                self.release_notes(cx);
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
    /// back under its marks. A dropped status or progress report leaves the
    /// records or the progress stale, so both start over and the tab's dot
    /// and bar are recomputed.
    fn recovered(&mut self, recovered: Recovered, cx: &mut Context<Self>) {
        let Recovered { dropped, statuses } = recovered;
        log::warn!(
            "a terminal's events came faster than the window took them: {dropped} were dropped{}",
            if statuses { ", status reports among them, so its status starts over" } else { "" }
        );
        if statuses {
            self.status.reset();
            self.progress = None;
            self.clear_contexts();
            cx.emit(TerminalEvent::Status(None));
        }
    }

    /// Drops every context and the badge; true when there were any.
    fn clear_contexts(&mut self) -> bool {
        self.badge = None;
        self.contexts.clear()
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
            life: Life::Running,
            origin: Origin::Spawned,
            held_notes: None,
            replaying: false,
            status: status::Records::default(),
            progress: None,
            contexts: Contexts::default(),
            badge: None,
            dir_check: dir_check::DirCheck::default(),
            theme,
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
            let (request, doing) =
                spawn_request(profile, host, command_override, cwd_override, config, dirs::home_dir());
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
    /// background ([`crate::remote::local::Session::start`]). The directory
    /// of a request for this machine is checked there, on the thread that
    /// starts the session and not on this one, which a path that takes long
    /// to answer would hold up ([`with_checked_dir`]). A remote host's is a
    /// path on that host, which the host resolves.
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
        let session = match host {
            Host::Local => crate::remote::local::Session::start(
                host.clone(),
                move || {
                    let is_dir = request.cwd.as_deref().is_some_and(|dir| std::path::Path::new(dir).is_dir());
                    with_checked_dir(request, is_dir)
                },
                Box::new(sink),
            ),
            Host::Remote(_) => crate::remote::local::Session::start(host.clone(), move || request, Box::new(sink)),
        };
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
        let (size, recorded) = attach_size(request.when);
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
        this.origin = Origin::Attached;
        this.held_notes = Some(Vec::new());
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

    /// The program exited: the terminal has ended, which its tab hears once.
    pub fn mark_exited(&mut self, cx: &mut Context<Self>) {
        if self.end(LifeEvent::Exited, cx) {
            cx.emit(TerminalEvent::Exited);
        }
    }

    /// The server session can't be got back, or Brindle can't show it any
    /// more (its line is on the screen): the terminal has ended, but the tab
    /// stays open on its last screen, so no `Exited`.
    fn mark_lost(&mut self, cx: &mut Context<Self>) {
        self.replaying = false;
        self.end(LifeEvent::Lost, cx);
    }

    /// The session never started or attached (its line is on the screen):
    /// as [`Terminal::mark_lost`], and an attached terminal then shows only
    /// that line ([`Terminal::shows_only_message`]).
    fn mark_failed(&mut self, cx: &mut Context<Self>) {
        self.replaying = false;
        self.end(LifeEvent::Failed, cx);
    }

    /// Moves the terminal's life on by `event`. True when that ended a
    /// running terminal, whose status activity, progress and contexts then
    /// end.
    fn end(&mut self, event: LifeEvent, cx: &mut Context<Self>) -> bool {
        let was = self.life;
        self.life = was.after(event);
        let ended = was == Life::Running;
        if ended {
            let activity = self.status.end_activity();
            let progress = self.progress.take().is_some();
            let contexts = self.clear_contexts();
            if activity || progress || contexts {
                cx.emit(TerminalEvent::Status(None));
            }
        }
        ended
    }

    /// The program status records (OSC 7501).
    pub fn status(&self) -> &status::Records {
        &self.status
    }

    /// The program's progress (OSC 9;4), if it reported one.
    pub fn progress(&self) -> Option<Progress> {
        self.progress
    }

    /// The program's open contexts (OSC 3008), outermost first.
    pub fn contexts(&self) -> &Contexts {
        &self.contexts
    }

    /// The badge of the program's notable context, if any
    /// ([`Contexts::notable`]).
    pub fn badge(&self) -> Option<&Badge> {
        self.badge.as_ref()
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
        // The same rule the server's emulator follows, so a replay agrees.
        let progress = found.progress_after(self.progress);
        let progress_changed = progress != self.progress;
        self.progress = progress;
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
            Found::Progress(_) => false,
            // The same stack the server's emulator keeps, so a replay
            // agrees; resets and prompts leave it.
            Found::Context(message) => {
                let changed = self.contexts.apply(message);
                if changed {
                    self.badge = self.contexts.notable();
                }
                changed
            }
            Found::PromptStart => self.status.end_activity(),
            Found::Reset => self.status.reset(),
        };
        if changed || progress_changed {
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
                // Output is what brings a directory report.
                self.follow_reported_dir(cx);
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
    /// Shows `text` as one dim line (`tab_note`): a failure that belongs to
    /// this tab and doesn't end its session. While an attached session's
    /// first replay is still to come, it is held and written after it.
    pub fn note(&mut self, text: &str, cx: &mut Context<Self>) {
        match &mut self.held_notes {
            Some(held) => held.push(text.to_string()),
            None => self.show(&tab_note::line(text), cx),
        }
    }

    /// Writes the notices [`Terminal::note`] held, after what the terminal
    /// shows now.
    fn release_notes(&mut self, cx: &mut Context<Self>) {
        for text in self.held_notes.take().unwrap_or_default() {
            self.show(&tab_note::line(&text), cx);
        }
    }

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
        // The marks' directory report went with them.
        self.follow_reported_dir(cx);
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
    pub fn select_last_output(&mut self, to_clipboard: bool, cx: &mut Context<Self>) -> bool {
        let mut term = self.term.lock();
        let Some((start, end)) = self.marks.lock().last_output(&term) else { return false };
        let mut selection = Selection::new(SelectionType::Simple, start, Side::Left);
        selection.update(end, Side::Right);
        term.selection = Some(selection);
        drop(term);
        self.finish_selection(to_clipboard, cx);
        cx.notify();
        true
    }

    /// The last command's output as text, leaving the selection alone.
    pub fn last_output_text(&self) -> Option<String> {
        let term = self.term.lock();
        let (start, end) = self.marks.lock().last_output(&term)?;
        Some(term.bounds_to_string(start, end)).filter(|s| !s.is_empty())
    }

    /// The last command's line as text: the line its shell reported (OSC
    /// 633 `E`), else what is between its `B` and `C` marks
    /// ([`Marks::last_command_line`]).
    pub fn last_command_line_text(&self) -> Option<String> {
        let term = self.term.lock();
        let source = self.marks.lock().last_command_line(&term)?;
        source.text(&term)
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

    /// Called when a mouse selection is finished: the text goes to the
    /// primary selection, and to the clipboard when `to_clipboard` (the
    /// `copy_on_select` setting in effect now) says so.
    pub fn finish_selection(&mut self, to_clipboard: bool, cx: &mut Context<Self>) {
        if let Some(text) = self.selection_text() {
            crate::primary::write_primary(cx, ClipboardItem::new_string(text.clone()));
            if to_clipboard {
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
    /// if it existed when it was checked: this only reads what the check,
    /// run off the UI thread, found (`dir_check`), so it gives none while
    /// the check is still out. A remote session's is a path on its host,
    /// never checked here: the directory the program reported (OSC 7),
    /// else the one its host last reported.
    pub fn working_directory(&self) -> Option<std::path::PathBuf> {
        match &self.backend {
            #[cfg(any(target_os = "linux", windows))]
            Backend::Server { session, host: Host::Local, .. } => match process::LOCAL_DIRECTORY {
                process::LocalDirectory::Reported => self.dir_check.existing().map(Into::into),
                process::LocalDirectory::Process => {
                    session.pid().and_then(|pid| process::PtyProcess::new(pid).working_directory())
                }
            },
            #[cfg(any(target_os = "linux", windows))]
            Backend::Server { session, host: Host::Remote(_), .. } => self
                .marks
                .lock()
                .reported_dir()
                .map(Into::into)
                .or_else(|| session.foreground()?.cwd.map(Into::into)),
            Backend::Remote => None,
        }
    }

    /// Whether the terminal's directory is the one its program reports,
    /// checked on this machine: a session here, where the system gives no
    /// process's directory ([`process::LOCAL_DIRECTORY`]). A remote
    /// session's reported directory is a path on its host, never checked
    /// here, and a message terminal has no program.
    fn follows_reported_dir(&self) -> bool {
        match &self.backend {
            #[cfg(any(target_os = "linux", windows))]
            Backend::Server { host: Host::Local, .. } => {
                process::LOCAL_DIRECTORY == process::LocalDirectory::Reported
            }
            #[cfg(any(target_os = "linux", windows))]
            Backend::Server { host: Host::Remote(_), .. } => false,
            Backend::Remote => false,
        }
    }

    /// Tells the directory check which directory the program reports now,
    /// and starts the check it asks for, if any ([`dir_check::DirCheck`]).
    fn follow_reported_dir(&mut self, cx: &mut Context<Self>) {
        if !self.follows_reported_dir() {
            return;
        }
        let to_check = self.dir_check.report(self.marks.lock().reported_dir());
        if let Some(dir) = to_check {
            self.check_dir(dir, cx);
        }
    }

    /// Finds out whether `dir` is a directory on the background executor,
    /// where a path that takes long to answer holds nothing up, and gives
    /// the directory check the answer; it may then ask for the next one.
    fn check_dir(&mut self, dir: std::path::PathBuf, cx: &mut Context<Self>) {
        let probed = dir.clone();
        let is_dir = cx.background_executor().spawn(async move { probed.is_dir() });
        cx.spawn(async move |this, cx| {
            let is_dir = is_dir.await;
            this.update(cx, |this, cx| {
                if let Some(next) = this.dir_check.finished(&dir, is_dir) {
                    this.check_dir(next, cx);
                }
            })
            .ok();
        })
        .detach();
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

    /// Keeps the layout of the tab this terminal is in current on its
    /// server session (`crate::layout`), as it changes, while its program
    /// runs; a message terminal has none.
    pub fn set_layout(&self, layout: Option<serde_json::Value>) {
        match &self.backend {
            #[cfg(any(target_os = "linux", windows))]
            Backend::Server { session, closing: Closing::Keeps, .. } if self.life == Life::Running => {
                session.set_layout(layout)
            }
            // Its program ended, or ends with Brindle: nothing restores it.
            #[cfg(any(target_os = "linux", windows))]
            Backend::Server { closing: Closing::Keeps | Closing::Ends, .. } => {}
            Backend::Remote => drop(layout),
        }
    }

    /// Ends the terminal's connection to its program: `close` ends the
    /// program; otherwise a server session's program is left as its
    /// [`Closing`] says. A message terminal has no program.
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
            Backend::Server { session, .. } if self.life == Life::Running => session.id_and_key(),
            Backend::Server { .. } | Backend::Remote => None,
        }
    }

    /// An attach that failed (refused, or its session gone): the terminal
    /// only says why, and session restore doesn't record it.
    pub fn shows_only_message(&self) -> bool {
        self.origin.shows_only_message(self.life)
    }
}

/// Where a terminal's program is in its life (`types R1`): one value, so a
/// terminal can't be both failed and running.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Life {
    /// Its program runs, or its session is still starting or attaching.
    Running,
    /// Its program exited, or its session was lost or can't be shown.
    Ended,
    /// Its session never started or attached; the terminal says why.
    Failed,
}

/// What ends a terminal's life.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LifeEvent {
    /// Its program exited.
    Exited,
    /// Its session can't be got back, or Brindle's emulator broke on it.
    Lost,
    /// Its session never started or attached.
    Failed,
}

impl Life {
    /// The life after `event`: any of them ends a running terminal, a
    /// failure is final, and a session that failed after it ended failed.
    fn after(self, event: LifeEvent) -> Self {
        match self {
            Self::Failed => Self::Failed,
            Self::Running | Self::Ended => match event {
                LifeEvent::Exited | LifeEvent::Lost => Self::Ended,
                LifeEvent::Failed => Self::Failed,
            },
        }
    }
}

/// How a terminal came by its session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Origin {
    /// It started it (or shows a message, with no session).
    Spawned,
    /// It attached a detached one.
    Attached,
}

impl Origin {
    /// Whether a terminal of this origin, in `life`, only says why it has
    /// no session: an attach that failed, which restore doesn't record.
    fn shows_only_message(self, life: Life) -> bool {
        self == Self::Attached && life == Life::Failed
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
/// This machine's is resolved here: the program, the directory it is to
/// start in (the asked one, else the profile's with `~` as `home`, else
/// `home`), and Brindle's own environment under the profile's. Which one
/// depends on `host` alone, not on what the profile targets.
///
/// This machine's directory is named, not checked: whether it exists is
/// found out on the thread that starts the session, and
/// [`with_checked_dir`] then says what is sent.
#[cfg(any(target_os = "linux", windows))]
pub(crate) fn spawn_request(
    profile: &Profile,
    host: &Host,
    command_override: Option<(String, Vec<String>)>,
    cwd_override: Option<std::path::PathBuf>,
    config: &Config,
    home: Option<std::path::PathBuf>,
) -> (crate::remote::local::SpawnRequest, String) {
    let policy = Closing::new(command_override.is_some()).policy();
    let size = GridSize::default();
    let scrollback = u32::try_from(config.scrollback_lines).unwrap_or(u32::MAX);
    if !host.is_local() {
        let request =
            crate::remote::local::SpawnRequest::remote(profile, command_override, cwd_override, size, scrollback, policy);
        let doing = match &request.command {
            None => format!("open a shell on {}", host.name()),
            Some(command) => format!("start {command:?} on {}", host.name()),
        };
        return (request, doing);
    }
    // The program is always resolved here (on Windows from `PATH` and
    // `%COMSPEC%`), so `SPAWN` never leaves it to the server.
    let (program, args) = command_override.unwrap_or_else(|| match &profile.command {
        Some(program) => (program.clone(), profile.args.clone()),
        None => default_program(&profile.args),
    });
    let working_directory = cwd_override.or_else(|| profile.working_directory(home.as_deref())).or(home);
    let doing = format!("start {program:?}");
    let request = crate::remote::local::SpawnRequest {
        // A profile's empty `command` is none.
        command: Some(program).filter(|program| !program.is_empty()),
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

/// A request for this machine ([`spawn_request`]) as it is sent, once its
/// directory has been looked at: `is_dir` is whether `request.cwd` is a
/// directory here. One that isn't is left out, and the program starts in
/// its server's default directory; no other directory is tried in its
/// place.
#[cfg(any(target_os = "linux", windows))]
pub(crate) fn with_checked_dir(
    mut request: crate::remote::local::SpawnRequest,
    is_dir: bool,
) -> crate::remote::local::SpawnRequest {
    if !is_dir {
        request.cwd = None;
    }
    request
}

/// The grid an attach starts with, and whether its session's size is still
/// to come with a listing (a recorded session's). A session just listed is
/// attached at its own size, so its replay, rendered at that size, lands
/// as it was drawn and the first layout resizes the session only if the
/// window's grid differs. The size is the caller's (`GridSize::listed`),
/// taken within the limits every grid keeps whatever it says.
#[cfg(any(target_os = "linux", windows))]
fn attach_size(when: crate::remote::local::Attaching) -> (GridSize, bool) {
    match when {
        crate::remote::local::Attaching::Now { cols, rows } => {
            (GridSize { cols, rows, ..GridSize::default() }.clamped(), false)
        }
        crate::remote::local::Attaching::Recorded => (GridSize::default(), true),
    }
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

/// Resolves a cell color against the term's dynamic palette (the colors
/// the program set, OSC 4, 10, 11 and 12) and then `theme`, the terminal's
/// own ([`Terminal::default_color`]).
pub fn resolve_color(color: ansi::Color, colors: &term::color::Colors, theme: &Theme) -> Rgb {
    match color {
        ansi::Color::Spec(rgb) => rgb,
        ansi::Color::Named(named) => {
            colors[named as usize].unwrap_or_else(|| palette_color(theme, named as usize).into())
        }
        ansi::Color::Indexed(i) => {
            colors[i as usize].unwrap_or_else(|| palette_color(theme, i as usize).into())
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
    fn a_grid_size_is_clamped_to_the_grid_limits_and_its_cells_kept() {
        let narrow = GridSize { cols: 1, rows: 7, cell_width: 9, cell_height: 18 };
        assert_eq!(narrow.clamped(), GridSize { cols: MIN_COLS, rows: 7, cell_width: 9, cell_height: 18 });
        let wide = GridSize { cols: 120, ..narrow };
        assert_eq!(wide.clamped(), wide);
        let huge = GridSize { cols: u16::MAX, rows: u16::MAX, ..narrow };
        assert_eq!(huge.clamped(), GridSize { cols: 2048, rows: 1024, cell_width: 9, cell_height: 18 });
        let largest = GridSize { cols: 2048, rows: 1024, ..narrow };
        assert_eq!(largest.clamped(), largest);
        assert_eq!(GridSize { rows: 0, ..wide }.clamped(), GridSize { rows: 1, ..wide });
    }

    /// A session an endpoint lists at 65535×65535 has no usable size: its
    /// tab starts at the default grid, as a session listed with no size
    /// does. One listed within the limits is attached at its own size.
    #[cfg(any(target_os = "linux", windows))]
    #[test]
    fn a_session_listed_past_the_grid_limits_attaches_at_the_default_size() {
        use crate::remote::ListedSession;
        use crate::remote::local::Attaching;
        use crate::remote::protocol::client::SessionInfo;

        // As `Workspace::attached_terminal` asks for the attach.
        let attach = |cols, rows| {
            let listed = ListedSession::of(&SessionInfo { session: 3, cols, rows, ..Default::default() });
            let size = GridSize::listed(listed.size);
            attach_size(Attaching::Now { cols: size.cols, rows: size.rows })
        };
        let default = GridSize::default();
        assert_eq!(attach(65535, 65535), (default, false));
        assert_eq!(attach(2049, 24), (default, false));
        assert_eq!(attach(80, 1025), (default, false));
        // A core that predates sizes lists none.
        assert_eq!(attach(0, 0), (default, false));
        assert_eq!(attach(120, 40), (GridSize { cols: 120, rows: 40, ..default }, false));
        assert_eq!(attach(2048, 1024), (GridSize { cols: 2048, rows: 1024, ..default }, false));
    }

    /// Whatever size an attach is asked at, its grid stays within the
    /// limits; a recorded session's size comes later, with its listing.
    #[cfg(any(target_os = "linux", windows))]
    #[test]
    fn an_attach_never_starts_past_the_grid_limits() {
        use crate::remote::local::Attaching;

        let default = GridSize::default();
        assert_eq!(
            attach_size(Attaching::Now { cols: 65535, rows: 65535 }),
            (GridSize { cols: 2048, rows: 1024, ..default }, false)
        );
        assert_eq!(attach_size(Attaching::Now { cols: 1, rows: 5 }), (GridSize { cols: MIN_COLS, rows: 5, ..default }, false));
        assert_eq!(attach_size(Attaching::Recorded), (default, true));
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

        use std::path::{Path, PathBuf};

        let a8 = RemoteTarget::Ssh { host: "A8".into(), user: None, port: None, identity_file: None, ssh_args: Vec::new(), connect_at_launch: false };
        let profile = Profile {
            name: "htop".into(),
            command: Some("htop".into()),
            cwd: Some("/srv/logs".into()),
            remote: Some(a8.clone()),
            ..Default::default()
        };
        let config = Config::default();
        let remote_host = Host::Remote(a8);
        // The request as it is sent: this machine's with its directory
        // checked against `here`, the directories this machine has, as the
        // thread that starts the session checks it; a remote host's as it
        // is.
        let request = |profile: &Profile, host: &Host, command, cwd, here: &[&str]| {
            let home = Some(PathBuf::from("/home/me"));
            let (request, doing) = spawn_request(profile, host, command, cwd, &config, home);
            let request = match host {
                Host::Local => {
                    // As paths: Windows joins with `\`.
                    let is_dir =
                        request.cwd.as_deref().is_some_and(|dir| here.iter().any(|here| Path::new(dir) == Path::new(here)));
                    with_checked_dir(request, is_dir)
                }
                Host::Remote(_) => request,
            };
            (request, doing)
        };

        let (remote, doing) = request(&profile, &remote_host, None, None, &[]);
        assert_eq!((remote.command.as_deref(), doing.as_str()), (Some("htop"), "start \"htop\" on A8"));
        assert_eq!(remote.cwd.as_deref(), Some("/srv/logs"));
        assert_eq!(remote.policy, Policy::Detach);

        let (local, doing) = request(&profile, &Host::Local, None, None, &["/home/me"]);
        assert_eq!((local.command.as_deref(), doing.as_str()), (Some("htop"), "start \"htop\""));
        assert_eq!(local.cwd, None, "the profile's directory isn't here, and home isn't tried");
        assert_eq!(local.policy, Policy::Detach);
        let (local, _) = request(&profile, &Host::Local, None, None, &["/srv/logs"]);
        assert_eq!(local.cwd.as_deref(), Some("/srv/logs"));
        let (split, _) = request(&profile, &Host::Local, None, Some("/srv/old".into()), &["/srv/old", "/srv/logs"]);
        assert_eq!(split.cwd.as_deref(), Some("/srv/old"), "the asked directory wins");

        // A local profile's `~` is this machine's home, and with no
        // directory it starts in home, if there is one.
        let local_profile = Profile { cwd: Some("~/src".into()), remote: None, ..profile.clone() };
        let (local, _) = request(&local_profile, &Host::Local, None, None, &["/home/me/src"]);
        assert_eq!(local.cwd.as_deref().map(Path::new), Some(Path::new("/home/me/src")));
        let no_dir = Profile { cwd: None, ..local_profile };
        assert_eq!(request(&no_dir, &Host::Local, None, None, &["/home/me"]).0.cwd.as_deref(), Some("/home/me"));
        assert_eq!(request(&no_dir, &Host::Local, None, None, &[]).0.cwd, None);

        let shell = Profile { command: None, ..profile.clone() };
        let (_, doing) = request(&shell, &remote_host, None, None, &[]);
        assert_eq!(doing, "open a shell on A8");
        let empty = Profile { command: Some(String::new()), ..profile.clone() };
        let (empty, doing) = request(&empty, &remote_host, None, None, &[]);
        assert_eq!((empty.command, doing.as_str()), (None, "open a shell on A8"));

        let (command, _) = request(&profile, &remote_host, Some(("top".into(), vec!["-b".into()])), None, &[]);
        assert_eq!((command.command.as_deref(), command.args.as_slice()), (Some("top"), ["-b".to_string()].as_slice()));
        assert_eq!(command.policy, Policy::Kill);
    }

    /// A request for this machine names the directory it is to start in
    /// without looking at it. What the thread that starts the session then
    /// finds decides whether it is sent: a directory that isn't there is
    /// left out, and nothing else of the request changes.
    #[cfg(any(target_os = "linux", windows))]
    #[test]
    fn a_local_requests_directory_is_named_unchecked_and_sent_only_if_it_is_there() {
        use std::path::PathBuf;

        let profile =
            Profile { name: "sh".into(), command: Some("sh".into()), cwd: Some("/srv/logs".into()), ..Default::default() };
        let request = |cwd: Option<&str>| {
            let home = Some(PathBuf::from("/home/me"));
            spawn_request(&profile, &Host::Local, None, cwd.map(PathBuf::from), &Config::default(), home).0
        };
        // The asked directory, else the profile's: neither is looked at.
        assert_eq!(request(Some("/gone/away")).cwd.as_deref(), Some("/gone/away"));
        assert_eq!(request(None).cwd.as_deref(), Some("/srv/logs"));

        let there = with_checked_dir(request(Some("/gone/away")), true);
        assert_eq!(there.cwd.as_deref(), Some("/gone/away"));
        let gone = with_checked_dir(request(Some("/gone/away")), false);
        assert_eq!(gone.cwd, None, "the profile's directory isn't tried in its place");
        assert_eq!((gone.command, gone.args, gone.profile), (there.command, there.args, there.profile));
        // A request with no directory stays one.
        let none = Profile { cwd: None, ..profile.clone() };
        let (bare, _) = spawn_request(&none, &Host::Local, None, None, &Config::default(), None);
        assert_eq!(bare.cwd, None);
        assert_eq!(with_checked_dir(bare, false).cwd, None);
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

    /// A cell's colour: an RGB one as it is; a named or indexed one from
    /// the palette entries the program set, else from the theme.
    #[test]
    fn cell_colors_take_the_programs_palette_then_the_theme() {
        let theme = crate::theme::builtin_theme("tokyo-night").unwrap();
        let resolve = |color: ansi::Color, colors: &term::color::Colors| resolve_color(color, colors, &theme);
        let mut colors = term::color::Colors::default();
        let spec = Rgb { r: 1, g: 2, b: 3 };
        assert_eq!(resolve(ansi::Color::Spec(spec), &colors), spec);
        assert_eq!(resolve(ansi::Color::Named(NamedColor::Red), &colors), Rgb::from(theme.ansi[1]));
        assert_eq!(resolve(ansi::Color::Named(NamedColor::Background), &colors), Rgb::from(theme.background));
        assert_eq!(resolve(ansi::Color::Named(NamedColor::Cursor), &colors), Rgb::from(theme.cursor));
        assert_eq!(resolve(ansi::Color::Indexed(4), &colors), Rgb::from(theme.ansi[4]));
        assert_eq!(resolve(ansi::Color::Indexed(196), &colors), Rgb::from(theme.indexed(196)));

        // What the program set (OSC 4, 11) wins over the theme.
        let (red, background) = (Rgb { r: 9, g: 8, b: 7 }, Rgb { r: 6, g: 5, b: 4 });
        colors[NamedColor::Red] = Some(red);
        colors[NamedColor::Background] = Some(background);
        colors[196] = Some(red);
        assert_eq!(resolve(ansi::Color::Named(NamedColor::Red), &colors), red);
        assert_eq!(resolve(ansi::Color::Indexed(1), &colors), red, "index 1 is red");
        assert_eq!(resolve(ansi::Color::Named(NamedColor::Background), &colors), background);
        assert_eq!(resolve(ansi::Color::Indexed(196), &colors), red);
        // The entries it didn't set stay the theme's, and RGB stays RGB.
        assert_eq!(resolve(ansi::Color::Indexed(4), &colors), Rgb::from(theme.ansi[4]));
        assert_eq!(resolve(ansi::Color::Named(NamedColor::Cursor), &colors), Rgb::from(theme.cursor));
        assert_eq!(resolve(ansi::Color::Spec(spec), &colors), spec);
    }

    /// The emulator's settings come from the config: OSC 52 reads only
    /// when `osc52_paste` allows them, the scrollback, and the cursor's
    /// shape and blink.
    #[test]
    fn the_emulator_takes_osc52_scrollback_and_the_cursor_from_the_config() {
        let defaults = term_config(&Config::default());
        assert_eq!(defaults.osc52, term::Osc52::OnlyCopy);
        assert_eq!(defaults.scrolling_history, 10_000);
        assert_eq!(defaults.default_cursor_style.shape, ansi::CursorShape::Block);
        assert!(defaults.default_cursor_style.blinking);

        let mut config = Config::default();
        config.osc52_paste = true;
        config.scrollback_lines = 0;
        config.cursor = crate::config::CursorConfig { shape: CursorShapeConfig::Beam, blink: false };
        let set = term_config(&config);
        assert_eq!(set.osc52, term::Osc52::CopyPaste);
        assert_eq!(set.scrolling_history, 0);
        assert_eq!(set.default_cursor_style.shape, ansi::CursorShape::Beam);
        assert!(!set.default_cursor_style.blinking);

        config.cursor.shape = CursorShapeConfig::Underline;
        assert_eq!(term_config(&config).default_cursor_style.shape, ansi::CursorShape::Underline);
    }

    /// A terminal's parser reads directory reports in its host's path
    /// style: this machine's for a local session, POSIX for a remote
    /// endpoint's. So OSC 7's `/C:/src` is `C:\src`, and OSC 9;9's `C:\src`
    /// a directory at all, only for a Windows session here.
    #[cfg(any(target_os = "linux", windows))]
    #[test]
    fn a_terminals_parser_reads_directories_in_its_hosts_path_style() {
        use crate::config::RemoteTarget;
        use shell_integration::PathStyle;

        let reported = |host: &Host, report: &[u8]| {
            let mut parser = parser_for(host);
            let size = GridSize { cols: 20, rows: 2, ..Default::default() };
            let mut term = Term::new(term::Config::default(), &size, alacritty_terminal::event::VoidListener);
            let marks = Mutex::new(Marks::default());
            parser.advance(&mut term, &marks, report);
            marks.lock().reported_dir().map(|dir| dir.to_string_lossy().into_owned())
        };
        let osc7: &[u8] = b"\x1b]7;file://host/C:/src\x1b\\";
        let osc9: &[u8] = b"\x1b]9;9;C:\\src\x1b\\";

        let remote = Host::Remote(RemoteTarget::Ssh {
            host: "A8".into(),
            user: None,
            port: None,
            identity_file: None,
            ssh_args: Vec::new(),
            connect_at_launch: false,
        });
        assert_eq!(reported(&remote, osc7).as_deref(), Some("/C:/src"));
        assert_eq!(reported(&remote, osc9), None);

        let (osc7_here, osc9_here) = match process::LOCAL_PATH_STYLE {
            PathStyle::Posix => (Some("/C:/src"), None),
            PathStyle::Windows => (Some(r"C:\src"), Some(r"C:\src")),
        };
        assert_eq!(reported(&Host::Local, osc7).as_deref(), osc7_here);
        assert_eq!(reported(&Host::Local, osc9).as_deref(), osc9_here);
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

    /// Any event ends a running terminal: an exit or a loss as ended, a
    /// failure as failed. An ended terminal stays ended, unless its session
    /// then failed, and a failure is final.
    #[test]
    fn a_terminals_life_moves_on_by_what_ended_it() {
        use LifeEvent::{Exited, Failed, Lost};
        let table = [
            (Life::Running, Exited, Life::Ended),
            (Life::Running, Lost, Life::Ended),
            (Life::Running, Failed, Life::Failed),
            (Life::Ended, Exited, Life::Ended),
            (Life::Ended, Lost, Life::Ended),
            (Life::Ended, Failed, Life::Failed),
            (Life::Failed, Exited, Life::Failed),
            (Life::Failed, Lost, Life::Failed),
            (Life::Failed, Failed, Life::Failed),
        ];
        for (life, event, after) in table {
            assert_eq!(life.after(event), after, "{life:?} after {event:?}");
        }
    }

    /// Only an attach that failed counts as showing nothing but its
    /// message, whatever else ended.
    #[test]
    fn only_a_failed_attach_shows_only_its_message() {
        let table = [
            (Origin::Spawned, Life::Running, false),
            (Origin::Spawned, Life::Ended, false),
            (Origin::Spawned, Life::Failed, false),
            (Origin::Attached, Life::Running, false),
            (Origin::Attached, Life::Ended, false),
            (Origin::Attached, Life::Failed, true),
        ];
        for (origin, life, only) in table {
            assert_eq!(origin.shows_only_message(life), only, "{origin:?} {life:?}");
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
