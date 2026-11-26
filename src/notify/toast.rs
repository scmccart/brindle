//! Windows toast notifications, shown under Brindle's AppUserModelID by a
//! worker thread.
//!
//! The worker registers the AppUserModelID for the current user first, so
//! toasts work without a Start Menu shortcut (a `cargo install`, a dev
//! build). Each toast's tag is Brindle's notification id, so a bell's toast
//! replaces the tab's previous one and an OSC 99 close removes it. A click
//! reports the default action, a button press its number; an activated
//! toast leaves Action Center, so the click is followed by a close. A toast
//! that times out stays in Action Center and can still be clicked, so only
//! one the user dismissed counts as closed. When Windows or the user has
//! turned Brindle's notifications off, Windows drops them and the worker
//! logs one warning. It still shows every toast: Windows keeps an app's
//! notification settings only once it has shown one of its toasts, so a
//! worker that skipped them while `Setting()` said off could leave Brindle
//! listed as off in Settings with a toggle that never stuck.
//!
//! The toast's XML and the reading of its activation arguments are pure
//! and built everywhere, so they are tested on Linux too; the worker is
//! Windows' alone.

use super::kitty::{Button, Caps};

/// What toasts can do, for the OSC 99 query's reply: buttons always, no
/// urgency or expiry.
#[cfg_attr(not(windows), allow(dead_code))]
pub const CAPS: Caps = Caps { hints: false, buttons: true };

/// The text lines a toast shows a `body` in: one per line, at most two
/// (`ToastGeneric` shows a title and two text lines), a third and later
/// joining the second with a space (OpenSpec change
/// `agents-zone-attention`, decision 6). A body with no line break is one
/// line, as an OSC source's always is.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn body_lines(body: &str) -> Vec<String> {
    let mut lines = body.split('\n').map(|line| line.trim_end_matches('\r'));
    let first = lines.next().unwrap_or_default().to_string();
    let rest: Vec<&str> = lines.collect();
    if rest.is_empty() { vec![first] } else { vec![first, rest.join(" ")] }
}

/// A `ToastGeneric` toast with `title` and `body` (a `<text>` per line,
/// [`body_lines`]), a button per label (its activation arguments
/// `button=N`) and, when `silent`, no sound, escaped so markup in them
/// stays text.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn xml(title: &str, body: &str, buttons: &[String], silent: bool) -> String {
    let escape = |text: &str| {
        text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
    };
    let texts: String = body_lines(body).iter().map(|line| format!("<text>{}</text>", escape(line))).collect();
    let mut xml = format!(
        "<toast><visual><binding template=\"ToastGeneric\"><text>{}</text>{texts}</binding></visual>",
        escape(title)
    );
    if !buttons.is_empty() {
        xml.push_str("<actions>");
        for (index, label) in buttons.iter().enumerate() {
            xml.push_str(&format!(
                "<action content=\"{}\" arguments=\"button={}\" activationType=\"foreground\"/>",
                escape(label),
                Button::key(index)
            ));
        }
        xml.push_str("</actions>");
    }
    if silent {
        xml.push_str("<audio silent=\"true\"/>");
    }
    xml.push_str("</toast>");
    xml
}

/// The action key a toast's activation `arguments` stand for: a button's
/// number for `button=N`, else `"default"` (the toast itself, whose
/// arguments are empty).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn action_key(arguments: &str) -> String {
    match arguments.strip_prefix("button=").and_then(Button::from_key) {
        Some(button) => button.number().to_string(),
        None => "default".to_string(),
    }
}

#[cfg(windows)]
pub use worker::spawn;

#[cfg(windows)]
mod worker {
    use std::collections::VecDeque;
    use std::sync::mpsc;

    use futures::channel::mpsc::UnboundedSender;
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::Foundation::TypedEventHandler;
    use windows::UI::Notifications::{
        NotificationSetting, ToastActivatedEventArgs, ToastDismissalReason, ToastDismissedEventArgs,
        ToastNotification, ToastNotificationManager, ToastNotifier,
    };
    use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
    use windows::core::{HSTRING, IInspectable, Interface as _};

    use super::super::{Command, Hints, Reply};
    use crate::win::APP_ID;

    /// Toasts kept alive (their click handlers with them), oldest dropped
    /// first; matches what [`super::super::Notifier`] remembers.
    const KEEP: usize = super::super::MAX_REMEMBERED;

    /// The group every Brindle toast is in.
    const GROUP: &str = "brindle";

    /// Starts the worker. Commands go to the returned sender.
    pub fn spawn(replies: UnboundedSender<Reply>) -> mpsc::Sender<Command> {
        let (commands, received) = mpsc::channel();
        let started = std::thread::Builder::new().name("toasts".into()).spawn(move || run(received, replies));
        if let Err(err) = started {
            log::warn!("could not start the notification thread: {err}");
        }
        commands
    }

    fn run(commands: mpsc::Receiver<Command>, replies: UnboundedSender<Reply>) {
        // SAFETY: initializes COM for this thread, which lives as long as
        // the process.
        unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok().ok();
        if let Err(err) = crate::win::register_app_id(super::super::icon_path().as_deref()) {
            log::warn!("registering Brindle for notifications: {err}");
        }
        let notifier = match ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(APP_ID)) {
            Ok(notifier) => notifier,
            Err(err) => {
                log::warn!("notifications are unavailable: {err}");
                return;
            }
        };
        let mut next_id = 1;
        let mut shown: VecDeque<(u32, ToastNotification)> = VecDeque::new();
        let mut warned = false;
        for command in commands {
            match command {
                Command::Notify { key, replaces, summary, body, hints, buttons } => {
                    // Only for the log: Windows drops a toast it won't show
                    // itself.
                    if matches!(notifier.Setting(), Ok(NotificationSetting::Enabled)) {
                        warned = false;
                    } else if !std::mem::replace(&mut warned, true) {
                        log::warn!("notifications for Brindle are turned off in Windows; it will drop them");
                    }
                    let id = if replaces != 0 {
                        replaces
                    } else {
                        next_id += 1;
                        next_id - 1
                    };
                    match show(&notifier, id, &summary, &body, hints, &buttons, &replies) {
                        Ok(toast) => {
                            shown.retain(|(old, _)| *old != id);
                            shown.push_back((id, toast));
                            while shown.len() > KEEP {
                                shown.pop_front();
                            }
                            replies.unbounded_send(Reply::Shown { key, id }).ok();
                        }
                        Err(err) => log::warn!("showing a notification: {err}"),
                    }
                }
                Command::Close { id } => {
                    shown.retain(|(old, _)| *old != id);
                    if let Err(err) = close(id) {
                        log::debug!("closing notification {id}: {err}");
                    }
                }
            }
        }
    }

    /// Removes toast `id` from the screen and from Action Center.
    fn close(id: u32) -> windows::core::Result<()> {
        ToastNotificationManager::History()?.RemoveGroupedTagWithId(
            &HSTRING::from(id.to_string()),
            &HSTRING::from(GROUP),
            &HSTRING::from(APP_ID),
        )
    }

    fn show(
        notifier: &ToastNotifier,
        id: u32,
        summary: &str,
        body: &str,
        hints: Hints,
        buttons: &[String],
        replies: &UnboundedSender<Reply>,
    ) -> windows::core::Result<ToastNotification> {
        // Urgency and expiry have no toast counterpart.
        let content = XmlDocument::new()?;
        content.LoadXml(&HSTRING::from(super::xml(summary, body, buttons, hints.silent)))?;
        let toast = ToastNotification::CreateToastNotification(&content)?;
        toast.SetTag(&HSTRING::from(id.to_string()))?;
        toast.SetGroup(&HSTRING::from(GROUP))?;
        let clicks = replies.clone();
        toast.Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(move |_, args| {
            let arguments = args
                .ok()
                .ok()
                .and_then(|args| args.cast::<ToastActivatedEventArgs>().ok())
                .and_then(|args| args.Arguments().ok())
                .map(|arguments| arguments.to_string())
                .unwrap_or_default();
            clicks.unbounded_send(Reply::Action { id, action: super::action_key(&arguments) }).ok();
            // An activated toast leaves Action Center: it is closed.
            clicks.unbounded_send(Reply::Closed { id }).ok();
            Ok(())
        }))?;
        let closes = replies.clone();
        toast.Dismissed(&TypedEventHandler::<ToastNotification, ToastDismissedEventArgs>::new(move |_, args| {
            if args.ok()?.Reason()? == ToastDismissalReason::UserCanceled {
                closes.unbounded_send(Reply::Closed { id }).ok();
            }
            Ok(())
        }))?;
        notifier.Show(&toast)?;
        Ok(toast)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toast_markup_stays_text() {
        assert_eq!(
            xml("make <all> & \"go\"", "it's </text><text>done", &[], false),
            "<toast><visual><binding template=\"ToastGeneric\"><text>make &lt;all&gt; &amp; &quot;go&quot;</text>\
             <text>it&apos;s &lt;/text&gt;&lt;text&gt;done</text></binding></visual></toast>"
        );
    }

    /// The spec's "A Windows toast": a permission's two body lines are the
    /// toast's two text lines; a third joins the second; an OSC body, with
    /// no line break, stays one.
    #[test]
    fn a_body_line_is_a_text_line_at_most_two() {
        let binding = |texts: &str| {
            format!("<toast><visual><binding template=\"ToastGeneric\"><text>Fake needs permission</text>{texts}</binding></visual></toast>")
        };
        assert_eq!(
            xml("Fake needs permission", "Command: cargo test", &[], false),
            binding("<text>Command: cargo test</text>")
        );
        assert_eq!(
            xml("Fake needs permission", "Command: cargo test\nReject · Allow once", &[], false),
            binding("<text>Command: cargo test</text><text>Reject · Allow once</text>")
        );
        assert_eq!(
            xml("Fake needs permission", "Command: cargo test\nReject · Allow once\nmore", &[], false),
            binding("<text>Command: cargo test</text><text>Reject · Allow once more</text>")
        );
        assert_eq!(body_lines("build done"), ["build done"]);
        assert_eq!(body_lines(""), [""]);
        assert_eq!(body_lines("a\r\nb"), ["a", "b"]);
    }

    #[test]
    fn buttons_and_silence() {
        let buttons = ["Allow".to_string(), "<Deny> \"now\"".to_string()];
        assert_eq!(
            xml("t", "b", &buttons, true),
            "<toast><visual><binding template=\"ToastGeneric\"><text>t</text><text>b</text></binding></visual>\
             <actions><action content=\"Allow\" arguments=\"button=1\" activationType=\"foreground\"/>\
             <action content=\"&lt;Deny&gt; &quot;now&quot;\" arguments=\"button=2\" activationType=\"foreground\"/>\
             </actions><audio silent=\"true\"/></toast>"
        );
        assert!(xml("t", "b", &[], true).ends_with("</visual><audio silent=\"true\"/></toast>"));
    }

    #[test]
    fn activation_arguments() {
        assert_eq!(action_key(""), "default");
        assert_eq!(action_key("button=1"), "1");
        assert_eq!(action_key("button=4"), "4");
        for other in ["button=5", "button=", "button=x", "launch", "button=1 "] {
            assert_eq!(action_key(other), "default", "{other:?}");
        }
    }
}
