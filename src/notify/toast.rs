//! Windows toast notifications, shown under Brindle's AppUserModelID by a
//! worker thread.
//!
//! The worker registers the AppUserModelID for the current user first, so
//! toasts work without a Start Menu shortcut (a `cargo install`, a dev
//! build). Each toast's tag is Brindle's notification id, so a bell's toast
//! replaces the tab's previous one. A click reports the default action. A
//! toast that times out stays in Action Center and can still be clicked, so
//! only one the user dismissed counts as closed. When Windows or the user
//! has turned Brindle's notifications off, requests are dropped with one
//! warning.

use std::collections::VecDeque;
use std::sync::mpsc;

use futures::channel::mpsc::UnboundedSender;
use windows::Data::Xml::Dom::XmlDocument;
use windows::Foundation::TypedEventHandler;
use windows::UI::Notifications::{
    NotificationSetting, ToastDismissalReason, ToastDismissedEventArgs, ToastNotification, ToastNotificationManager,
    ToastNotifier,
};
use windows::Win32::System::Com::{COINIT_MULTITHREADED, CoInitializeEx};
use windows::core::{HSTRING, IInspectable};

use super::{Command, Reply, toast_xml};
use crate::win::APP_ID;

/// Toasts kept alive (their click handlers with them), oldest dropped
/// first; matches what [`super::Notifier`] remembers.
const KEEP: usize = super::MAX_REMEMBERED;

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
    // SAFETY: initializes COM for this thread, which lives as long as the
    // process.
    unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.ok().ok();
    if let Err(err) = crate::win::register_app_id(super::icon_path().as_deref()) {
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
        let Command::Notify { key, replaces, summary, body } = command;
        if !matches!(notifier.Setting(), Ok(NotificationSetting::Enabled)) {
            if !std::mem::replace(&mut warned, true) {
                log::warn!("notifications for Brindle are turned off in Windows; dropping them");
            }
            continue;
        }
        warned = false;
        let id = if replaces != 0 {
            replaces
        } else {
            next_id += 1;
            next_id - 1
        };
        match show(&notifier, id, &summary, &body, &replies) {
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
}

fn show(
    notifier: &ToastNotifier,
    id: u32,
    summary: &str,
    body: &str,
    replies: &UnboundedSender<Reply>,
) -> windows::core::Result<ToastNotification> {
    let content = XmlDocument::new()?;
    content.LoadXml(&HSTRING::from(toast_xml(summary, body)))?;
    let toast = ToastNotification::CreateToastNotification(&content)?;
    toast.SetTag(&HSTRING::from(id.to_string()))?;
    toast.SetGroup(&HSTRING::from("brindle"))?;
    let clicks = replies.clone();
    toast.Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(move |_, _| {
        clicks.unbounded_send(Reply::Action { id, action: "default".into() }).ok();
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
