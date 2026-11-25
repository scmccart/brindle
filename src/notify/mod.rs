//! Desktop notifications: finding them in terminal output ([`osc`]),
//! deciding whether to show them ([`policy`]), and showing them.
//!
//! [`Notifier`] is a GPUI global. It rate-limits, then hands each
//! notification to a sink: on Linux a D-Bus worker on the background
//! executor (`dbus`), on Windows a toast worker thread (`toast`), or, when
//! `BRINDLE_NOTIFY_LOG` is set, a file that tests read, appended by a
//! `notify-log` writer thread that quitting waits for ([`LogWriter`]). It
//! remembers which window and tab each notification came from, so a click
//! can show that tab, and which of kitty's OSC 99 notifications are open
//! ([`kitty::Open`]), so a program can update, close and list them and hear
//! of clicks and closes.

#[cfg(target_os = "linux")]
pub mod activation;
#[cfg(target_os = "linux")]
mod dbus;
pub mod kitty;
pub mod osc;
pub mod policy;
mod toast;

/// Raising a window on Windows for a notification click: restore it if
/// minimized and bring it to the front, or, when Windows refuses that,
/// flash its taskbar button. (GPUI's own activation fakes a key press to
/// get the foreground, which Brindle doesn't do.)
#[cfg(windows)]
pub mod activation {
    use gpui::{App, Window};
    use windows::Win32::UI::WindowsAndMessaging::{
        FLASHW_ALL, FLASHW_TIMERNOFG, FLASHWINFO, FlashWindowEx, IsIconic, SW_RESTORE, SetForegroundWindow, ShowWindow,
    };

    /// Nothing to prepare: Windows raises with no token.
    pub fn prepare(_window: &Window, _cx: &mut App) {}

    pub fn raise(window: &mut Window, _token: Option<&str>, _cx: &mut App) {
        let Some(hwnd) = crate::win::hwnd(window) else { return };
        // SAFETY: plain window-manager calls on Brindle's own window.
        unsafe {
            if IsIconic(hwnd).as_bool() {
                ShowWindow(hwnd, SW_RESTORE).ok().ok();
            }
            if !SetForegroundWindow(hwnd).as_bool() {
                log::debug!("Windows kept the foreground; flashing the window instead");
                let info = FLASHWINFO {
                    cbSize: std::mem::size_of::<FLASHWINFO>() as u32,
                    hwnd,
                    dwFlags: FLASHW_ALL | FLASHW_TIMERNOFG,
                    uCount: 0,
                    dwTimeout: 0,
                };
                FlashWindowEx(&info).ok().ok();
            }
        }
    }
}

/// Raising a window elsewhere: plain activation, with no tokens.
#[cfg(not(any(target_os = "linux", windows)))]
pub mod activation {
    use gpui::{App, Window};

    /// Nothing to prepare: no tokens here.
    pub fn prepare(_window: &Window, _cx: &mut App) {}

    pub fn raise(window: &mut Window, _token: Option<&str>, _cx: &mut App) {
        window.activate_window();
    }
}

use std::collections::{HashMap, VecDeque};
use std::io::Write as _;
use std::path::PathBuf;
use std::time::Instant;

#[cfg(any(target_os = "linux", windows))]
use futures::StreamExt as _;
#[cfg(target_os = "linux")]
use futures::channel::mpsc::UnboundedSender;
#[cfg(any(target_os = "linux", windows))]
use futures::channel::mpsc::unbounded;
#[cfg(target_os = "linux")]
use gpui::AppContext as _;
use gpui::{App, BorrowAppContext as _, EntityId, Global, WeakEntity, WindowHandle};

use self::kitty::{Arrival, Caps, Closing, Expiry, KittyId, Program, Replace, Sound, Urgency};
use self::osc::Source;
use self::policy::RateLimiter;
use crate::terminal::Terminal;
use crate::workspace::Workspace;

/// Environment variable naming a file that receives notifications instead
/// of the desktop (for tests).
pub const LOG_ENV: &str = "BRINDLE_NOTIFY_LOG";

/// How many clicked-on-able notifications are remembered.
const MAX_REMEMBERED: usize = 256;

/// What the platform's sink can show until it says otherwise, for the
/// OSC 99 query's reply. The log sink stands in for the platform's.
#[cfg(target_os = "linux")]
const PLATFORM_CAPS: Caps = dbus::CAPS;
#[cfg(windows)]
const PLATFORM_CAPS: Caps = toast::CAPS;
#[cfg(not(any(target_os = "linux", windows)))]
const PLATFORM_CAPS: Caps = Caps { hints: false, buttons: false };

/// A notification to show for a tab.
pub struct Request {
    pub source: Source,
    pub summary: String,
    pub body: String,
    pub window: WindowHandle<Workspace>,
    pub tab: u64,
    /// What an OSC 99 notification asks of clicks, closes and the sink.
    pub program: Option<ProgramNotice>,
}

/// An OSC 99 notification's program part: the terminal its reports go to,
/// and what it asked for.
#[derive(Clone, Debug, PartialEq)]
pub struct ProgramNotice {
    pub terminal: WeakEntity<Terminal>,
    pub program: Program,
}

#[derive(Clone)]
struct Target {
    window: WindowHandle<Workspace>,
    tab: u64,
    source: Source,
    program: Option<ProgramNotice>,
}

/// What a notification asks of the platform beyond its text. Only OSC 99
/// sets anything but the defaults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(not(any(target_os = "linux", windows)), allow(dead_code))]
pub(crate) struct Hints {
    /// Toasts have no urgency or expiry.
    #[cfg_attr(windows, allow(dead_code))]
    pub urgency: Urgency,
    #[cfg_attr(windows, allow(dead_code))]
    pub expiry: Expiry,
    pub silent: bool,
}

impl Default for Hints {
    fn default() -> Self {
        Self { urgency: Urgency::Normal, expiry: Expiry::Default, silent: false }
    }
}

/// What [`Notifier`] asks a desktop sink to do.
#[cfg_attr(not(any(target_os = "linux", windows)), allow(dead_code))]
pub(crate) enum Command {
    Notify {
        /// Ours, echoed in [`Reply::Shown`].
        key: u64,
        /// The sink's id of a notification this one replaces, or 0.
        replaces: u32,
        summary: String,
        body: String,
        hints: Hints,
        /// Button labels, whose actions are keyed `"1"`, `"2"`, … (empty
        /// where the sink shows no actions).
        buttons: Vec<String>,
    },
    /// Close the notification the sink showed as `id`.
    Close { id: u32 },
}

/// What a desktop sink reports back.
#[cfg_attr(not(any(target_os = "linux", windows)), allow(dead_code))]
pub(crate) enum Reply {
    Shown {
        key: u64,
        id: u32,
    },
    /// `action` is `"default"` for the notification itself, or a button's
    /// key.
    Action {
        id: u32,
        action: String,
    },
    /// Wayland's activation token, for raising the window on a click.
    #[cfg_attr(windows, allow(dead_code))]
    Token {
        id: u32,
        token: String,
    },
    Closed {
        id: u32,
    },
    /// Whether the notification service shows buttons (actions).
    #[cfg_attr(windows, allow(dead_code))]
    Capabilities {
        buttons: bool,
    },
}

/// What [`Notifier`] does once it is done with its own state: show a tab,
/// write a report to a terminal.
#[derive(Default)]
struct Effects {
    show: Option<(Target, Option<String>)>,
    write: Option<(WeakEntity<Terminal>, String)>,
}

/// What the `notify-log` thread is asked to do, in order.
enum LogCommand {
    /// Append this line to the file.
    Line(String),
    /// Answer once every line sent before has been written.
    Flush(futures::channel::oneshot::Sender<()>),
}

/// The `BRINDLE_NOTIFY_LOG` file's writer: a `notify-log` thread that
/// appends each line, so the UI thread never touches the file. Its queue is
/// bounded by the notification rate limits (`policy.rs`, `process R9`).
/// When the thread couldn't start, lines are dropped (warned once, at
/// [`LogWriter::start`]).
#[derive(Clone)]
struct LogWriter(std::sync::mpsc::Sender<LogCommand>);

impl LogWriter {
    fn start(path: PathBuf) -> Self {
        let (commands, queued) = std::sync::mpsc::channel();
        let shown = path.display().to_string();
        let spawned = std::thread::Builder::new().name("notify-log".into()).spawn(move || {
            for command in queued {
                match command {
                    LogCommand::Line(line) => append(&path, &line),
                    LogCommand::Flush(done) => {
                        done.send(()).ok();
                    }
                }
            }
        });
        // A thread that didn't start dropped the receiving end with it, so
        // sends fail and lines are dropped.
        if let Err(err) = spawned {
            log::warn!("no notify-log thread ({err}): not writing notifications to {shown}");
        }
        LogWriter(commands)
    }

    fn write(&self, line: String) {
        self.0.send(LogCommand::Line(line)).ok();
    }

    /// Resolves once every line written before it is in the file, or at
    /// once when the thread is gone.
    fn flush(&self) -> impl std::future::Future<Output = ()> + use<> {
        let (done, flushed) = futures::channel::oneshot::channel();
        // A closed channel drops `done` with the command: `flushed` ends.
        self.0.send(LogCommand::Flush(done)).ok();
        async move {
            flushed.await.ok();
        }
    }
}

/// Appends `line` to the file at `path`, creating it.
fn append(path: &std::path::Path, line: &str) {
    let written = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .and_then(|mut file| file.write_all(line.as_bytes()));
    if let Err(err) = written {
        log::warn!("writing {}: {err}", path.display());
    }
}

enum Sink {
    Log { writer: LogWriter, next_id: u32 },
    #[cfg(target_os = "linux")]
    Bus(UnboundedSender<Command>),
    #[cfg(windows)]
    Toast(std::sync::mpsc::Sender<Command>),
    /// No desktop sink on this platform: requests are dropped, with one
    /// warning.
    #[cfg(not(any(target_os = "linux", windows)))]
    Off { warned: bool },
}

pub struct Notifier {
    sink: Sink,
    limiter: RateLimiter,
    next_key: u64,
    /// Sent, waiting for the server's id.
    pending: HashMap<u64, Target>,
    /// Shown, by the server's id, oldest first in `order`.
    shown: HashMap<u32, Target>,
    order: VecDeque<u32>,
    /// Each tab's latest bell notification, which its next one replaces.
    bell_ids: HashMap<u64, u32>,
    /// Activation tokens, which arrive just before the click itself.
    tokens: HashMap<u32, String>,
    /// What the sink can show, for the OSC 99 query's reply.
    caps: Caps,
    /// The open OSC 99 notifications with ids, by terminal.
    open: kitty::Open<EntityId>,
}

impl Global for Notifier {}

impl Notifier {
    /// Installs the global and starts its sink: the log file's writer
    /// thread, which a quit waits for, or the platform's sink (on Linux a
    /// D-Bus worker, which connects on first use).
    pub fn init(cx: &mut App) {
        let sink = match std::env::var_os(LOG_ENV).filter(|p| !p.is_empty()) {
            Some(path) => {
                let writer = LogWriter::start(PathBuf::from(path));
                // Every line sent before the quit is in the file by the
                // time Brindle exits, `--dump-screen-after`'s included
                // (`process R16`: in the quit path, not a drop).
                let flusher = writer.clone();
                cx.on_app_quit(move |_| flusher.flush()).detach();
                Sink::Log { writer, next_id: 1 }
            }
            #[cfg(windows)]
            None => {
                let (reply_tx, mut replies) = unbounded();
                let commands = toast::spawn(reply_tx);
                cx.spawn(async move |cx| {
                    while let Some(reply) = replies.next().await {
                        if cx.update(|cx| Self::on_reply(reply, cx)).is_err() {
                            break;
                        }
                    }
                })
                .detach();
                Sink::Toast(commands)
            }
            #[cfg(not(any(target_os = "linux", windows)))]
            None => Sink::Off { warned: false },
            #[cfg(target_os = "linux")]
            None => {
                let (commands, command_rx) = unbounded();
                let (reply_tx, mut replies) = unbounded();
                let executor = cx.background_executor().clone();
                cx.background_spawn(dbus::run(command_rx, reply_tx, executor)).detach();
                cx.spawn(async move |cx| {
                    while let Some(reply) = replies.next().await {
                        if cx.update(|cx| Self::on_reply(reply, cx)).is_err() {
                            break;
                        }
                    }
                })
                .detach();
                Sink::Bus(commands)
            }
        };
        cx.set_global(Notifier {
            sink,
            limiter: RateLimiter::new(),
            next_key: 1,
            pending: HashMap::new(),
            shown: HashMap::new(),
            order: VecDeque::new(),
            bell_ids: HashMap::new(),
            tokens: HashMap::new(),
            caps: PLATFORM_CAPS,
            open: kitty::Open::default(),
        });
    }

    /// What OSC 99 can do here, for the query's reply: the sink's latest
    /// word on it.
    pub fn caps(cx: &App) -> Caps {
        cx.try_global::<Notifier>().map_or(PLATFORM_CAPS, |this| this.caps)
    }

    /// The program in `terminal` asks for the ids of its open OSC 99
    /// notifications: the reply, oldest first, is written back to it.
    pub fn answer_alive(terminal: &WeakEntity<Terminal>, id: &KittyId, cx: &mut App) {
        let open = cx.try_global::<Notifier>().map_or_else(Vec::new, |this| this.open.alive(terminal.entity_id()));
        Self::report(terminal, kitty::reply_alive(id, &open), cx);
    }

    /// The program in `terminal` closes its notification `id`: closes it,
    /// and writes the close report back when it asked for one.
    pub fn close_program(terminal: &WeakEntity<Terminal>, id: &KittyId, cx: &mut App) {
        if !cx.has_global::<Notifier>() {
            return;
        }
        let report = cx.update_global::<Notifier, _>(|this, _| this.close_request(terminal.entity_id(), id));
        if let Some(report) = report {
            Self::report(terminal, report, cx);
        }
    }

    /// Writes `text` to `terminal`'s program, unless the terminal closed.
    fn report(terminal: &WeakEntity<Terminal>, text: String, cx: &mut App) {
        if terminal.update(cx, |terminal, _| terminal.write(text.into_bytes())).is_err() {
            log::debug!("an OSC 99 reply found its terminal closed");
        }
    }

    /// Shows `request` unless a rate limit says no. Never blocks on D-Bus.
    pub fn send(request: Request, cx: &mut App) {
        if !cx.has_global::<Notifier>() {
            return;
        }
        cx.update_global::<Notifier, _>(|this, _| this.send_request(request, Instant::now()));
    }

    fn send_request(&mut self, request: Request, now: Instant) {
        if !self.limiter.allow(request.tab, request.source, now) {
            log::debug!("rate limit: dropping a {} notification", request.source.name());
            return;
        }
        let key = self.next_key;
        self.next_key += 1;
        // An OSC 99 update replaces its open notification; one sent before
        // the first had an id closes the first once it has.
        let replace = match (&request.program, request.source) {
            (Some(notice), _) => notice.program.id.as_ref().map_or(Replace::Nothing, |id| {
                self.open.send(notice.terminal.entity_id(), id, key)
            }),
            (None, Source::Bell) => self.bell_ids.get(&request.tab).map_or(Replace::Nothing, |&id| Replace::Id(id)),
            (None, Source::Osc9 | Source::Osc777 | Source::Osc99 | Source::Osc7501 | Source::Agent) => {
                Replace::Nothing
            }
        };
        let replaces = match replace {
            Replace::Id(id) => id,
            Replace::Nothing => 0,
        };
        let (hints, buttons) = match &request.program {
            Some(notice) => {
                let program = &notice.program;
                let hints = Hints { urgency: program.urgency, expiry: program.expiry, silent: program.sound == Sound::Silent };
                let buttons = if self.caps.buttons {
                    program.buttons.iter().map(|label| label.as_str().to_string()).collect()
                } else {
                    Vec::new()
                };
                (hints, buttons)
            }
            None => (Hints::default(), Vec::new()),
        };
        let target = Target { window: request.window, tab: request.tab, source: request.source, program: request.program };
        self.pending.insert(key, target);
        match &mut self.sink {
            Sink::Log { writer, next_id } => {
                let line = match &self.pending[&key].program {
                    Some(notice) => {
                        let flags = LogFlags { replaces: replaces != 0, caps: self.caps };
                        log_line_99(&request.summary, &request.body, &notice.program, flags)
                    }
                    None => log_line(request.source, &request.summary, &request.body),
                };
                writer.write(line);
                let id = if replaces != 0 { replaces } else { std::mem::replace(next_id, *next_id + 1) };
                // The log shows at once, so nothing waits for an id.
                self.on_shown(key, id);
            }
            #[cfg(target_os = "linux")]
            Sink::Bus(commands) => {
                let command = Command::Notify { key, replaces, summary: request.summary, body: request.body, hints, buttons };
                commands.unbounded_send(command).ok();
            }
            #[cfg(windows)]
            Sink::Toast(commands) => {
                let command = Command::Notify { key, replaces, summary: request.summary, body: request.body, hints, buttons };
                commands.send(command).ok();
            }
            #[cfg(not(any(target_os = "linux", windows)))]
            Sink::Off { warned } => {
                let _ = (hints, buttons);
                self.pending.remove(&key);
                if !std::mem::replace(warned, true) {
                    log::warn!("desktop notifications aren't supported on this platform yet; dropping them");
                }
            }
        }
    }

    /// Hands `command` to the desktop sink. The log has nothing to close.
    fn command(&mut self, command: Command) {
        match &mut self.sink {
            Sink::Log { .. } => {}
            #[cfg(target_os = "linux")]
            Sink::Bus(commands) => {
                commands.unbounded_send(command).ok();
            }
            #[cfg(windows)]
            Sink::Toast(commands) => {
                commands.send(command).ok();
            }
            #[cfg(not(any(target_os = "linux", windows)))]
            Sink::Off { .. } => drop(command),
        }
    }

    /// The sink showed `key` as `id`: remember it for clicks, or close it
    /// when an update or the program's close came first.
    fn on_shown(&mut self, key: u64, id: u32) {
        let target = self.pending.remove(&key);
        match self.open.shown(key, id) {
            Arrival::Keep => {
                if let Some(target) = target {
                    self.remember(id, target);
                }
            }
            Arrival::Close => self.command(Command::Close { id }),
        }
    }

    /// The program closed `id` in `terminal`; the close report to write,
    /// if it asked for one.
    fn close_request(&mut self, terminal: EntityId, id: &KittyId) -> Option<String> {
        let target = match self.open.close(terminal, id) {
            Closing::Unknown => return None,
            Closing::Now(sink) => {
                self.order.retain(|&i| i != sink);
                let target = self.shown.get(&sink).cloned();
                self.forget(sink);
                self.command(Command::Close { id: sink });
                target
            }
            // Its sink id hasn't come yet: the table closes it then.
            Closing::WhenShown(key) => self.pending.remove(&key),
        };
        if let Sink::Log { writer, .. } = &self.sink {
            writer.write(log_close_line(id));
        }
        kitty::on_close(target.as_ref().and_then(|t| t.program.as_ref()).map(|n| &n.program))
    }

    fn remember(&mut self, id: u32, target: Target) {
        if target.source == Source::Bell {
            self.bell_ids.insert(target.tab, id);
        }
        if self.shown.insert(id, target).is_none() {
            self.order.push_back(id);
        }
        while self.order.len() > MAX_REMEMBERED {
            if let Some(old) = self.order.pop_front() {
                self.forget(old);
            }
        }
    }

    fn forget(&mut self, id: u32) {
        if let Some(target) = self.shown.remove(&id)
            && self.bell_ids.get(&target.tab) == Some(&id)
        {
            self.bell_ids.remove(&target.tab);
        }
        self.tokens.remove(&id);
    }

    fn on_reply(reply: Reply, cx: &mut App) {
        let effects = cx.update_global::<Notifier, _>(|this, _| this.apply_reply(reply));
        if let Some((terminal, text)) = effects.write {
            Self::report(&terminal, text, cx);
        }
        if let Some((target, token)) = effects.show {
            Self::show(target, token, cx);
        }
    }

    fn apply_reply(&mut self, reply: Reply) -> Effects {
        match reply {
            Reply::Shown { key, id } => {
                self.on_shown(key, id);
                Effects::default()
            }
            Reply::Token { id, token } => {
                if self.shown.contains_key(&id) {
                    self.tokens.insert(id, token);
                }
                Effects::default()
            }
            Reply::Action { id, action } => {
                let token = self.tokens.remove(&id);
                let Some(target) = self.shown.get(&id).cloned() else { return Effects::default() };
                let notice = target.program.as_ref();
                let activation = kitty::activation(notice.map(|n| &n.program), &action);
                let write = activation.write.zip(notice).map(|(text, n)| (n.terminal.clone(), text));
                let show = activation.raise.then_some((target, token));
                Effects { show, write }
            }
            Reply::Closed { id } => {
                self.order.retain(|&i| i != id);
                let target = self.shown.get(&id).cloned();
                self.forget(id);
                self.open.closed(id);
                let notice = target.and_then(|t| t.program);
                let write = notice.and_then(|n| kitty::on_close(Some(&n.program)).map(|text| (n.terminal, text)));
                Effects { show: None, write }
            }
            Reply::Capabilities { buttons } => {
                self.caps.buttons = buttons;
                Effects::default()
            }
        }
    }

    /// The user clicked a notification: show its tab and raise its window.
    fn show(target: Target, token: Option<String>, cx: &mut App) {
        let shown = target.window.update(cx, |workspace, window, cx| {
            workspace.show_tab(target.tab, window, cx);
            activation::raise(window, token.as_deref(), cx);
        });
        if shown.is_err() {
            log::debug!("a notification was clicked after its window closed");
        }
    }
}

/// A notification body escaped for servers with `body-markup`, which read
/// it as a small subset of HTML (the D-Bus sink, `dbus`).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn escape_markup(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// The PNG toasts show for Brindle (the AUMID's `IconUri`), written to
/// `%LOCALAPPDATA%rindle` when the file there differs.
#[cfg(windows)]
fn icon_path() -> Option<PathBuf> {
    const ICON: &[u8] = include_bytes!("../../assets/windows/brindle-256.png");
    let dir = dirs::data_local_dir()?.join("brindle");
    let path = dir.join("brindle-256.png");
    if std::fs::read(&path).is_ok_and(|on_disk| on_disk == ICON) {
        return Some(path);
    }
    match std::fs::create_dir_all(&dir).and_then(|()| crate::win::write_replacing(&path, ICON)) {
        Ok(()) => Some(path),
        Err(err) => {
            log::warn!("writing the notification icon to {}: {err}", path.display());
            None
        }
    }
}

/// A field of a test log line, kept on its line and in its column.
fn log_field(text: &str) -> String {
    text.replace(['\t', '\n', '\r'], " ")
}

/// One line of the test log: source, title and body, separated by tabs.
pub fn log_line(source: Source, summary: &str, body: &str) -> String {
    format!("{}\t{}\t{}\n", source.name(), log_field(summary), log_field(body))
}

/// What an OSC 99 log line says beyond the notification itself.
#[derive(Clone, Copy, Debug)]
struct LogFlags {
    /// It replaced an open notification.
    replaces: bool,
    /// What the platform's sink would show: buttons and hints are logged
    /// only where they would be sent.
    caps: Caps,
}

/// The test log's line for an OSC 99 notification: [`log_line`]'s, then
/// tab-separated flags in a fixed order, each only when it applies: `i=ID`,
/// `replaces`, `buttons=A|B`, `silent`, `urgency=low|critical`,
/// `expiry=never|MS`.
fn log_line_99(summary: &str, body: &str, program: &Program, flags: LogFlags) -> String {
    let mut line = format!("{}\t{}\t{}", Source::Osc99.name(), log_field(summary), log_field(body));
    if let Some(id) = &program.id {
        line.push_str(&format!("\ti={}", id.as_str()));
    }
    if flags.replaces {
        line.push_str("\treplaces");
    }
    if flags.caps.buttons && !program.buttons.is_empty() {
        let labels: Vec<String> = program.buttons.iter().map(|label| log_field(label.as_str())).collect();
        line.push_str(&format!("\tbuttons={}", labels.join("|")));
    }
    if program.sound == Sound::Silent {
        line.push_str("\tsilent");
    }
    if flags.caps.hints {
        match program.urgency {
            Urgency::Low => line.push_str("\turgency=low"),
            Urgency::Normal => {}
            Urgency::Critical => line.push_str("\turgency=critical"),
        }
        match program.expiry {
            Expiry::Default => {}
            Expiry::Never => line.push_str("\texpiry=never"),
            Expiry::After(after) => line.push_str(&format!("\texpiry={}", after.as_millis())),
        }
    }
    line.push('\n');
    line
}

/// The test log's line for a program's close of its notification `id`.
fn log_close_line(id: &KittyId) -> String {
    format!("close\t{}\ti={}\n", Source::Osc99.name(), id.as_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn program(id: Option<&str>) -> Program {
        Program {
            id: id.map(|id| KittyId::parse(id.as_bytes()).unwrap()),
            focus: true,
            report: false,
            close_report: false,
            urgency: Urgency::Normal,
            expiry: Expiry::Default,
            sound: Sound::Default,
            buttons: Vec::new(),
        }
    }

    #[test]
    fn osc99_log_lines() {
        const ALL: Caps = Caps { hints: true, buttons: true };
        let plain = LogFlags { replaces: false, caps: ALL };
        assert_eq!(log_line_99("Hi — sh", "", &program(None), plain), "osc99\tHi — sh\t\n");
        assert_eq!(
            log_line_99("Failed — sh", "b", &program(Some("job")), LogFlags { replaces: true, caps: ALL }),
            "osc99\tFailed — sh\tb\ti=job\treplaces\n"
        );
        let full = Program {
            buttons: ["Allow", "Deny"].iter().map(|l| kitty::Label::new(l).unwrap()).collect(),
            sound: Sound::Silent,
            urgency: Urgency::Critical,
            expiry: Expiry::After(Duration::from_millis(1500)),
            ..program(Some("p"))
        };
        assert_eq!(
            log_line_99("t", "b", &full, plain),
            "osc99\tt\tb\ti=p\tbuttons=Allow|Deny\tsilent\turgency=critical\texpiry=1500\n"
        );
        // What the sink wouldn't send isn't logged.
        let toast = LogFlags { replaces: false, caps: Caps { hints: false, buttons: true } };
        assert_eq!(log_line_99("t", "b", &full, toast), "osc99\tt\tb\ti=p\tbuttons=Allow|Deny\tsilent\n");
        let no_actions = LogFlags { replaces: false, caps: Caps { hints: true, buttons: false } };
        let quiet = Program { urgency: Urgency::Low, expiry: Expiry::Never, ..full };
        assert_eq!(log_line_99("t", "b", &quiet, no_actions), "osc99\tt\tb\ti=p\tsilent\turgency=low\texpiry=never\n");
        assert_eq!(log_close_line(&KittyId::parse(b"n1").unwrap()), "close\tosc99\ti=n1\n");
    }

    #[test]
    fn the_log_writer_appends_in_order_and_a_flush_waits_for_it() {
        struct Removed(PathBuf);
        impl Drop for Removed {
            fn drop(&mut self) {
                std::fs::remove_dir_all(&self.0).ok();
            }
        }
        let dir = std::env::temp_dir().join(format!("brindle-notify-log-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        let _removed = Removed(dir.clone());
        let path = dir.join("notify.log");
        let writer = LogWriter::start(path.clone());
        writer.write(log_line(Source::Osc9, "sh", "one"));
        writer.write(log_line(Source::Osc777, "t — sh", "two"));
        futures::executor::block_on(writer.flush());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "osc9\tsh\tone\nosc777\tt — sh\ttwo\n");
    }

    #[test]
    fn a_flush_ends_when_the_log_thread_is_gone() {
        let (commands, queued) = std::sync::mpsc::channel();
        drop(queued);
        futures::executor::block_on(LogWriter(commands).flush());
    }

    #[test]
    fn markup_is_escaped() {
        assert_eq!(escape_markup("a < b && c > d"), "a &lt; b &amp;&amp; c &gt; d");
        assert_eq!(escape_markup("<b>x</b>"), "&lt;b&gt;x&lt;/b&gt;");
    }

    #[test]
    fn log_lines() {
        assert_eq!(log_line(Source::Osc9, "make", "done"), "osc9\tmake\tdone\n");
        assert_eq!(log_line(Source::Osc777, "a\tb", "c\nd"), "osc777\ta b\tc d\n");
        assert_eq!(log_line(Source::Bell, "bash", "Bell"), "bell\tbash\tBell\n");
    }
}
