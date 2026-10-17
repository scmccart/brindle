//! Desktop notifications: finding them in terminal output ([`osc`]),
//! deciding whether to show them ([`policy`]), and showing them.
//!
//! [`Notifier`] is a GPUI global. It rate-limits, then hands each
//! notification to a sink: on Linux a D-Bus worker on the background
//! executor (`dbus`), on Windows a toast worker thread (`toast`), or, when
//! `BRINDLE_NOTIFY_LOG` is set, a file that tests read. It remembers which window and tab each notification came
//! from, so a click can show that tab.

#[cfg(target_os = "linux")]
pub mod activation;
#[cfg(target_os = "linux")]
mod dbus;
pub mod osc;
pub mod policy;
#[cfg(windows)]
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

    pub fn raise(window: &mut Window, _token: Option<&str>) {
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

    pub fn request_token(_window: &Window) -> Option<String> {
        None
    }

    pub fn track_input(_window: &Window, _cx: &mut App) {}
}

/// Raising a window elsewhere: plain activation, with no tokens.
#[cfg(not(any(target_os = "linux", windows)))]
pub mod activation {
    use gpui::{App, Window};

    pub fn raise(window: &mut Window, _token: Option<&str>) {
        window.activate_window();
    }

    pub fn request_token(_window: &Window) -> Option<String> {
        None
    }

    pub fn track_input(_window: &Window, _cx: &mut App) {}
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
use gpui::{App, BorrowAppContext as _, Global, WindowHandle};

use self::osc::Source;
use self::policy::RateLimiter;
use crate::workspace::Workspace;

/// Environment variable naming a file that receives notifications instead
/// of the desktop (for tests).
pub const LOG_ENV: &str = "BRINDLE_NOTIFY_LOG";

/// How many clicked-on-able notifications are remembered.
const MAX_REMEMBERED: usize = 256;

/// A notification to show for a tab.
pub struct Request {
    pub source: Source,
    pub summary: String,
    pub body: String,
    pub window: WindowHandle<Workspace>,
    pub tab: u64,
}

#[derive(Clone, Copy)]
struct Target {
    window: WindowHandle<Workspace>,
    tab: u64,
    source: Source,
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
    },
}

/// What a desktop sink reports back.
#[cfg_attr(not(any(target_os = "linux", windows)), allow(dead_code))]
pub(crate) enum Reply {
    Shown { key: u64, id: u32 },
    Action { id: u32, action: String },
    /// Wayland's activation token, for raising the window on a click.
    #[cfg_attr(windows, allow(dead_code))]
    Token { id: u32, token: String },
    Closed { id: u32 },
}

enum Sink {
    Log { path: PathBuf, next_id: u32 },
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
}

impl Global for Notifier {}

impl Notifier {
    /// Installs the global and, unless notifications go to a log file,
    /// starts the platform's sink (on Linux a D-Bus worker, which connects
    /// on first use).
    pub fn init(cx: &mut App) {
        let sink = match std::env::var_os(LOG_ENV).filter(|p| !p.is_empty()) {
            Some(path) => Sink::Log { path: PathBuf::from(path), next_id: 1 },
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
        });
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
        let target = Target { window: request.window, tab: request.tab, source: request.source };
        let replaces = match request.source {
            Source::Bell => self.bell_ids.get(&request.tab).copied().unwrap_or(0),
            _ => 0,
        };
        match &mut self.sink {
            Sink::Log { path, next_id } => {
                // Written right away: `--dump-screen-after` may quit next.
                let line = log_line(request.source, &request.summary, &request.body);
                let written = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&*path)
                    .and_then(|mut file| file.write_all(line.as_bytes()));
                if let Err(err) = written {
                    log::warn!("writing {}: {err}", path.display());
                }
                let id = if replaces != 0 { replaces } else { std::mem::replace(next_id, *next_id + 1) };
                self.remember(id, target);
            }
            #[cfg(target_os = "linux")]
            Sink::Bus(commands) => {
                let key = self.next_key;
                self.next_key += 1;
                self.pending.insert(key, target);
                let command = Command::Notify { key, replaces, summary: request.summary, body: request.body };
                commands.unbounded_send(command).ok();
            }
            #[cfg(windows)]
            Sink::Toast(commands) => {
                let key = self.next_key;
                self.next_key += 1;
                self.pending.insert(key, target);
                let command = Command::Notify { key, replaces, summary: request.summary, body: request.body };
                commands.send(command).ok();
            }
            #[cfg(not(any(target_os = "linux", windows)))]
            Sink::Off { warned } => {
                if !std::mem::replace(warned, true) {
                    log::warn!("desktop notifications aren't supported on this platform yet; dropping them");
                }
            }
        }
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
        let click = cx.update_global::<Notifier, _>(|this, _| match reply {
            Reply::Shown { key, id } => {
                if let Some(target) = this.pending.remove(&key) {
                    this.remember(id, target);
                }
                None
            }
            Reply::Token { id, token } => {
                if this.shown.contains_key(&id) {
                    this.tokens.insert(id, token);
                }
                None
            }
            Reply::Action { id, action } => {
                let token = this.tokens.remove(&id);
                let target = this.shown.get(&id).copied();
                (action == "default").then_some(target.map(|t| (t, token))).flatten()
            }
            Reply::Closed { id } => {
                this.order.retain(|&i| i != id);
                this.forget(id);
                None
            }
        });
        if let Some((target, token)) = click {
            Self::show(target, token, cx);
        }
    }

    /// The user clicked a notification: show its tab and raise its window.
    fn show(target: Target, token: Option<String>, cx: &mut App) {
        let shown = target.window.update(cx, |workspace, window, cx| {
            workspace.show_tab(target.tab, window, cx);
            activation::raise(window, token.as_deref());
        });
        if shown.is_err() {
            log::debug!("a notification was clicked after its window closed");
        }
    }
}

/// A `ToastGeneric` toast with `title` and `body`, escaped so markup in them
/// stays text.
#[cfg_attr(not(windows), allow(dead_code))]
fn toast_xml(title: &str, body: &str) -> String {
    let escape = |text: &str| {
        text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
    };
    format!(
        "<toast><visual><binding template=\"ToastGeneric\"><text>{}</text><text>{}</text></binding></visual></toast>",
        escape(title),
        escape(body)
    )
}

/// The PNG toasts show for Brindle, once one is written out.
#[cfg(windows)]
fn icon_path() -> Option<PathBuf> {
    None
}

/// One line of the test log: source, title and body, separated by tabs.
pub fn log_line(source: Source, summary: &str, body: &str) -> String {
    let field = |s: &str| s.replace(['\t', '\n', '\r'], " ");
    format!("{}\t{}\t{}\n", source.name(), field(summary), field(body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toast_markup_stays_text() {
        let xml = toast_xml("make <all> & \"go\"", "it's </text><text>done");
        assert_eq!(
            xml,
            "<toast><visual><binding template=\"ToastGeneric\"><text>make &lt;all&gt; &amp; &quot;go&quot;</text>\
             <text>it&apos;s &lt;/text&gt;&lt;text&gt;done</text></binding></visual></toast>"
        );
    }

    #[test]
    fn log_lines() {
        assert_eq!(log_line(Source::Osc9, "make", "done"), "osc9\tmake\tdone\n");
        assert_eq!(log_line(Source::Osc777, "a\tb", "c\nd"), "osc777\ta b\tc d\n");
        assert_eq!(log_line(Source::Bell, "bash", "Bell"), "bell\tbash\tBell\n");
    }
}
