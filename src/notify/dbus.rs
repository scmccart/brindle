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

use super::{Command, Reply};

const DEST: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
const IFACE: &str = "org.freedesktop.Notifications";
const RETRY: Duration = Duration::from_secs(30);

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
            Command::Notify { key, replaces, summary, body } => match notify(s, replaces, &summary, &body).await {
                Ok(id) => {
                    log::debug!("notification {id} shown");
                    replies.unbounded_send(Reply::Shown { key, id }).ok();
                }
                Err(err) => {
                    log::warn!("sending a desktop notification failed: {err:#}");
                    // Reconnect next time, in case the service restarted.
                    service = None;
                }
            },
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
    Ok(Service { connection, actions: has("actions"), markup: has("body-markup") })
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

async fn notify(service: &Service, replaces: u32, summary: &str, body: &str) -> zbus::Result<u32> {
    let body = if service.markup { escape_markup(body) } else { body.to_string() };
    // The default action is what a click on the notification invokes.
    let actions: Vec<&str> = if service.actions { vec!["default", "Show"] } else { Vec::new() };
    let mut hints: HashMap<&str, Value> = HashMap::new();
    hints.insert("desktop-entry", Value::from("brindle"));
    hints.insert("urgency", Value::from(1u8));
    let reply = service
        .connection
        .call_method(
            Some(DEST),
            PATH,
            Some(IFACE),
            "Notify",
            &("Brindle", replaces, "brindle", summary, body.as_str(), actions, hints, -1i32),
        )
        .await?;
    reply.body().deserialize::<u32>()
}

/// Servers with `body-markup` read the body as a small subset of HTML.
pub fn escape_markup(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn markup_is_escaped() {
        assert_eq!(escape_markup("a < b && c > d"), "a &lt; b &amp;&amp; c &gt; d");
        assert_eq!(escape_markup("<b>x</b>"), "&lt;b&gt;x&lt;/b&gt;");
    }
}
