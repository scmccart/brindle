//! The freedesktop notification service (`org.freedesktop.Notifications`)
//! over the session bus, run on GPUI's background executor.
//!
//! The worker connects on the first notification, so a session that never
//! notifies never touches D-Bus. If the bus or the service is missing it
//! logs once, drops notifications, and tries again at most every 30 s.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use futures::StreamExt as _;
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender};
use gpui::BackgroundExecutor;
use zbus::zvariant::Value;
use zbus::{Connection, MatchRule, MessageStream};

use super::kitty::{Button, Caps, Expiry, Urgency};
use super::{Command, Hints, Reply};

const DEST: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
const IFACE: &str = "org.freedesktop.Notifications";
const RETRY: Duration = Duration::from_secs(30);

/// What the service can do before it has been asked: urgency and expiry
/// always, and buttons until `GetCapabilities` says there are no actions
/// (as kitty assumes), since a query is answered at once.
pub const CAPS: Caps = Caps { hints: true, buttons: true };

/// The actions a notification is sent with: the default action, which a
/// click on the notification invokes, then a key and label per button
/// (keys `"1"`, `"2"`, …). None at all when the server shows no actions.
fn actions(server_actions: bool, buttons: &[String]) -> Vec<String> {
    if !server_actions {
        return Vec::new();
    }
    let mut actions = vec!["default".to_string(), "Show".to_string()];
    for (index, label) in buttons.iter().enumerate() {
        actions.push(Button::key(index));
        actions.push(label.clone());
    }
    actions
}

/// The `urgency` hint's byte.
fn urgency_byte(urgency: Urgency) -> u8 {
    match urgency {
        Urgency::Low => 0,
        Urgency::Normal => 1,
        Urgency::Critical => 2,
    }
}

/// `Notify`'s `expire_timeout`: -1 the server's default, 0 never, else
/// milliseconds (at most `i32::MAX`).
fn expire_timeout(expiry: Expiry) -> i32 {
    match expiry {
        Expiry::Default => -1,
        Expiry::Never => 0,
        Expiry::After(after) => i32::try_from(after.as_millis()).unwrap_or(i32::MAX).max(1),
    }
}

struct Service {
    connection: Connection,
    actions: bool,
    markup: bool,
}

pub async fn run(mut commands: UnboundedReceiver<Command>, replies: UnboundedSender<Reply>, executor: BackgroundExecutor) {
    let mut service: Option<Service> = None;
    let mut retry_at: Option<Instant> = None;
    let mut warned = false;
    while let Some(command) = commands.next().await {
        if service.is_none() && retry_at.is_none_or(|t| Instant::now() >= t) {
            match connect(&replies, &executor).await {
                Ok(s) => {
                    service = Some(s);
                    retry_at = None;
                    warned = false;
                }
                Err(err) => {
                    if !warned {
                        log::warn!("desktop notifications are unavailable: {err:#}");
                        warned = true;
                    }
                    retry_at = Some(Instant::now() + RETRY);
                }
            }
        }
        let Some(s) = &service else {
            log::debug!("dropping a notification: no notification service");
            continue;
        };
        match command {
            Command::Notify { key, replaces, summary, body, hints, buttons } => {
                match notify(s, replaces, &summary, &body, hints, &buttons).await {
                    Ok(id) => {
                        log::debug!("notification {id} shown");
                        replies.unbounded_send(Reply::Shown { key, id }).ok();
                    }
                    Err(err) => {
                        log::warn!("sending a desktop notification failed: {err:#}");
                        // Reconnect next time, in case the service restarted.
                        service = None;
                    }
                }
            }
            Command::Close { id } => {
                let closed = s.connection.call_method(Some(DEST), PATH, Some(IFACE), "CloseNotification", &(id,)).await;
                if let Err(err) = closed {
                    log::debug!("closing notification {id}: {err:#}");
                }
            }
        }
    }
}

async fn connect(replies: &UnboundedSender<Reply>, executor: &BackgroundExecutor) -> zbus::Result<Service> {
    let connection = Connection::session().await?;
    let reply = connection.call_method(Some(DEST), PATH, Some(IFACE), "GetCapabilities", &()).await?;
    let capabilities: Vec<String> = reply.body().deserialize()?;
    log::debug!("notification server capabilities: {capabilities:?}");
    let has = |name: &str| capabilities.iter().any(|c| c == name);

    let rule = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .interface(IFACE)?
        .path(PATH)?
        .build();
    let stream = MessageStream::for_match_rule(rule, &connection, None).await?;
    executor.spawn(listen(stream, replies.clone())).detach();
    let actions = has("actions");
    replies.unbounded_send(Reply::Capabilities { buttons: actions }).ok();
    Ok(Service { connection, actions, markup: has("body-markup") })
}

/// Forwards the server's signals about our notifications.
async fn listen(mut stream: MessageStream, replies: UnboundedSender<Reply>) {
    while let Some(message) = stream.next().await {
        let Ok(message) = message else { continue };
        let header = message.header();
        let reply = match header.member().map(|m| m.as_str()) {
            Some("ActionInvoked") => {
                message.body().deserialize::<(u32, String)>().ok().map(|(id, action)| Reply::Action { id, action })
            }
            Some("ActivationToken") => {
                message.body().deserialize::<(u32, String)>().ok().map(|(id, token)| Reply::Token { id, token })
            }
            Some("NotificationClosed") => {
                message.body().deserialize::<(u32, u32)>().ok().map(|(id, _reason)| Reply::Closed { id })
            }
            _ => None,
        };
        if let Some(reply) = reply
            && replies.unbounded_send(reply).is_err()
        {
            break;
        }
    }
}

async fn notify(
    service: &Service,
    replaces: u32,
    summary: &str,
    body: &str,
    hints: Hints,
    buttons: &[String],
) -> zbus::Result<u32> {
    let body = if service.markup { super::escape_markup(body) } else { body.to_string() };
    let actions = actions(service.actions, buttons);
    let mut map: HashMap<&str, Value> = HashMap::new();
    map.insert("desktop-entry", Value::from("brindle"));
    map.insert("urgency", Value::from(urgency_byte(hints.urgency)));
    if hints.silent {
        map.insert("suppress-sound", Value::from(true));
    }
    let reply = service
        .connection
        .call_method(
            Some(DEST),
            PATH,
            Some(IFACE),
            "Notify",
            &("Brindle", replaces, "brindle", summary, body.as_str(), actions, map, expire_timeout(hints.expiry)),
        )
        .await?;
    reply.body().deserialize::<u32>()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_action_list() {
        assert!(actions(false, &["Allow".into()]).is_empty(), "no actions on the server: none at all");
        assert_eq!(actions(true, &[]), ["default", "Show"]);
        assert_eq!(actions(true, &["Allow".into(), "Deny".into()]), ["default", "Show", "1", "Allow", "2", "Deny"]);
    }

    #[test]
    fn hints() {
        assert_eq!(
            [Urgency::Low, Urgency::Normal, Urgency::Critical].map(urgency_byte),
            [0, 1, 2]
        );
        assert_eq!(expire_timeout(Expiry::Default), -1);
        assert_eq!(expire_timeout(Expiry::Never), 0);
        assert_eq!(expire_timeout(Expiry::After(Duration::from_millis(1500))), 1500);
        assert_eq!(expire_timeout(Expiry::After(Duration::from_secs(u64::from(u32::MAX)))), i32::MAX);
        // Under a millisecond isn't "never".
        assert_eq!(expire_timeout(Expiry::After(Duration::from_micros(10))), 1);
        assert_eq!(Hints::default(), Hints { urgency: Urgency::Normal, expiry: Expiry::Default, silent: false });
    }
}
