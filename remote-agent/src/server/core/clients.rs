//! The core's client connections: accepting with the peer check, the
//! `HELLO`/`WELCOME` handshake (with what the client asked for: a linger
//! for its `kill` sessions, process info), the takeover of a stale
//! connection with the same token, requests, and the per-client queues
//! whose length pauses the client's sessions.

use std::io;
use std::time::{Duration, Instant};

use brindle_remote_protocol::client::{self, ErrorMessage, Hello, ToClient, ToCore, Welcome};
use brindle_remote_protocol::{RequestId, SessionId};

use super::{CLIENT_QUEUE_HIGH, CLIENT_QUEUE_LOW, Core, MAX_LINGER, Source, log};
use crate::server::conn::Conn;
use crate::server::event_loop::{EventLoop, Interest, Token};

/// A connection must send `HELLO` within this long.
const HELLO_LIMIT: Duration = Duration::from_secs(5);
/// At most this many connections may wait to say `HELLO`.
const MAX_PENDING: usize = 64;
/// Once this much is queued to a client, [`Core::send_client`] sends what
/// the socket takes right away, instead of at the loop's next turn: one
/// turn can read and feed a megabyte of a holder's output, and the client
/// would otherwise get none of it until the core had fed all of it.
const EAGER_FLUSH: usize = 64 * 1024;
/// How long a refused client gets to read its `ERROR`.
const REFUSAL_FLUSH_LIMIT: Duration = Duration::from_secs(1);

/// A connection that hasn't sent `HELLO` yet.
pub(super) struct Pending {
    conn: Conn,
    since: Instant,
}

pub(super) struct Client {
    pub(super) conn: Conn,
    pub(super) interest: Interest,
    /// The `client_token` from its `HELLO`.
    pub(super) token: String,
    /// Its queue passed [`CLIENT_QUEUE_HIGH`], so its sessions are paused.
    pub(super) over: bool,
    /// How long its `kill` sessions wait for it once its connection ends:
    /// its `HELLO`'s `linger_secs`, at most [`MAX_LINGER`].
    linger: Duration,
    /// It speaks protocol 2 and asked for `PROCESS`.
    pub(super) process_info: bool,
}

impl<L: EventLoop> Core<L> {
    pub(super) fn accept(&mut self) {
        loop {
            // The peer check comes before reading anything.
            let Some(listener) = &self.listener else {
                return;
            };
            let stream = match listener.accept() {
                Ok(Some(stream)) => stream,
                Ok(None) => return,
                Err(error) if error.kind() == io::ErrorKind::PermissionDenied => {
                    log!("{error}");
                    continue;
                }
                Err(error) => {
                    log!("accept failed: {error}");
                    return;
                }
            };
            if self.pending.len() >= MAX_PENDING {
                log!("too many connections waiting to say HELLO; refusing one");
                continue;
            }
            let conn = match Conn::new(stream) {
                Ok(conn) => conn,
                Err(error) => {
                    log!("can't use a connection: {error}");
                    continue;
                }
            };
            let token = self.token();
            if let Err(error) = self.ev.register(&conn.source(), token, Interest::READABLE) {
                log!("can't watch a connection: {error}");
                continue;
            }
            self.sources.insert(token, Source::Pending);
            self.pending.insert(
                token,
                Pending {
                    conn,
                    since: Instant::now(),
                },
            );
        }
    }

    pub(super) fn expire_pending(&mut self, now: Instant) {
        let expired: Vec<Token> = self
            .pending
            .iter()
            .filter(|(_, pending)| now.duration_since(pending.since) >= HELLO_LIMIT)
            .map(|(&token, _)| token)
            .collect();
        for token in expired {
            self.pending.remove(&token);
            self.drop_source(token);
        }
    }

    pub(super) fn read_pending(&mut self, token: Token) {
        let Some(pending) = self.pending.get_mut(&token) else {
            return;
        };
        let eof = pending.conn.read_available().unwrap_or(true);
        match pending.conn.next_frame() {
            Ok(Some(frame)) => {
                let mut conn = self.pending.remove(&token).expect("pending").conn;
                match ToCore::decode(&frame) {
                    Ok(ToCore::Hello(hello)) => self.welcome(token, conn, hello),
                    _ => {
                        queue(
                            &mut conn,
                            error(None, None, "the first message must be HELLO"),
                        );
                        let _ = conn.flush_for(REFUSAL_FLUSH_LIMIT);
                        self.drop_source(token);
                    }
                }
            }
            Ok(None) if !eof => {}
            Ok(None) | Err(_) => {
                self.pending.remove(&token);
                self.drop_source(token);
            }
        }
    }

    /// Answers a `HELLO`: `WELCOME` in the highest common version, or an
    /// `ERROR` naming both ranges and the end of the connection.
    fn welcome(&mut self, token: Token, mut conn: Conn, hello: Hello) {
        let protocol = match client::answer_hello(&hello, client::PROTOCOLS) {
            Ok(protocol) => protocol,
            Err(refusal) => {
                log!(
                    "refused a client {:?}: {}",
                    hello.client_version,
                    refusal.message
                );
                queue(&mut conn, ToClient::Error(refusal));
                let _ = conn.flush_for(REFUSAL_FLUSH_LIMIT);
                self.drop_source(token);
                return;
            }
        };
        let mut holder_protocols: Vec<u32> = self
            .holders
            .values()
            .filter_map(|holder| holder.protocol())
            .collect();
        holder_protocols.sort_unstable();
        holder_protocols.dedup();
        queue(
            &mut conn,
            ToClient::Welcome(Welcome {
                protocol,
                core_version: env!("CARGO_PKG_VERSION").into(),
                core_build: self.build.clone(),
                pid: std::process::id(),
                holder_protocols,
            }),
        );
        let linger = Duration::from_secs(hello.linger_secs.into()).min(MAX_LINGER);
        let process_info = protocol >= 2 && hello.process_info;
        log!(
            "client {token} connected ({:?}, protocol {protocol}{}{})",
            hello.client_version,
            if linger.is_zero() {
                String::new()
            } else {
                format!(", linger {}s", linger.as_secs())
            },
            if process_info { ", process info" } else { "" }
        );
        self.take_over(token, &hello.client_token);
        self.sources.insert(token, Source::Client);
        self.clients.insert(
            token,
            Client {
                conn,
                interest: Interest::READABLE,
                token: hello.client_token,
                over: false,
                linger,
                process_info,
            },
        );
        self.reattach(token);
        self.process_client_frames(token);
    }

    /// A `HELLO` with the token of a client that is still connected: that
    /// connection is stale (one that stopped delivering without closing, as
    /// a dropped network link leaves it), and `new` takes its place. It is
    /// closed without its sessions' close policies applying: they keep
    /// their owner, so [`Core::reattach`] gives each to `new` with its
    /// replay. Its requests still waiting fail: spawns not yet sent to a
    /// holder get their `ERROR` (if the old connection still takes it), and
    /// sessions still starting are hung up ([`Core::client_replaced`]). An
    /// empty token owns nothing, so it replaces nothing.
    fn take_over(&mut self, new: Token, client_token: &str) {
        if client_token.is_empty() {
            return;
        }
        let stale: Vec<Token> = self
            .clients
            .iter()
            .filter(|(_, client)| client.token == client_token)
            .map(|(&token, _)| token)
            .collect();
        for old in stale {
            log!(
                "client {new} has the token of client {old}, which it replaces; {} session(s) go to the new one",
                self.sessions_of(old).len()
            );
            let (failed, kept) = std::mem::take(&mut self.waiting)
                .into_iter()
                .partition(|waiting| waiting.client == old);
            self.waiting = kept;
            for waiting in failed {
                self.send_client(
                    old,
                    error(
                        Some(waiting.spawn.req),
                        None,
                        "this connection was replaced by a newer one with its token",
                    ),
                );
            }
            if let Some(mut stale) = self.clients.remove(&old) {
                // Whatever the socket takes now; nothing waits for the rest.
                let _ = stale.conn.flush();
            }
            self.drop_source(old);
            self.client_replaced(old);
        }
    }

    pub(super) fn read_client(&mut self, token: Token) {
        let Some(client) = self.clients.get_mut(&token) else {
            return;
        };
        // What was read before a failure still counts.
        let ended = match client.conn.read_available() {
            Ok(false) => None,
            Ok(true) => Some("the connection closed".to_string()),
            Err(error) => Some(format!("read failed: {error}")),
        };
        self.process_client_frames(token);
        if let Some(reason) = ended {
            self.drop_client(token, &reason);
        }
    }

    fn process_client_frames(&mut self, token: Token) {
        loop {
            let Some(client) = self.clients.get_mut(&token) else {
                return;
            };
            let frame = match client.conn.next_frame() {
                Ok(Some(frame)) => frame,
                Ok(None) => return,
                Err(error) => return self.drop_client(token, &format!("bad frame: {error}")),
            };
            match ToCore::decode(&frame) {
                Ok(message) => self.handle_client(token, message),
                Err(decode) => self.send_client(token, error(None, None, decode.to_string())),
            }
        }
    }

    fn handle_client(&mut self, client: Token, message: ToCore) {
        match message {
            ToCore::Hello(_) => self.send_client(
                client,
                error(None, None, "this connection already sent HELLO"),
            ),
            ToCore::Spawn(spawn) => self.client_spawn(client, spawn),
            ToCore::Data(data) => self.client_input(client, data),
            ToCore::Resize(resize) => self.client_resize(client, resize),
            ToCore::Close(target) => self.client_close(client, target.session),
            ToCore::Reparent(reparent) => self.client_reparent(client, reparent),
            ToCore::Attach(attach) => self.client_attach(client, attach),
            ToCore::Detach(target) => self.client_detach(client, target.session),
            ToCore::List(request) => self.client_list(client, request.req),
            ToCore::Kill(kill) => self.client_kill(client, kill),
            ToCore::Status(request) => self.client_status(client, request.req),
            ToCore::Stop(request) => self.client_stop(client, request.req),
            ToCore::StepDown(request) => self.client_step_down(client, request.req),
        }
    }

    /// Queues a message for a client. Past [`CLIENT_QUEUE_HIGH`] the client's
    /// sessions are paused in their holders.
    pub(super) fn send_client(&mut self, client: Token, message: ToClient) {
        let Some(entry) = self.clients.get_mut(&client) else {
            return;
        };
        queue(&mut entry.conn, message);
        if entry.conn.queued() >= EAGER_FLUSH {
            // A failure shows again in the loop's own flush, which drops
            // the client there.
            let _ = entry.conn.flush();
        }
        if !entry.over && entry.conn.queued() > CLIENT_QUEUE_HIGH {
            entry.over = true;
            self.pause_client_sessions(client, true);
        }
    }

    /// Sends what the socket takes; back under [`CLIENT_QUEUE_LOW`], the
    /// client's sessions are resumed.
    pub(super) fn flush_client(&mut self, client: Token) {
        let Some(entry) = self.clients.get_mut(&client) else {
            return;
        };
        if let Err(error) = entry.conn.flush() {
            return self.drop_client(client, &format!("write failed: {error}"));
        }
        if entry.over && entry.conn.queued() < CLIENT_QUEUE_LOW {
            entry.over = false;
            self.pause_client_sessions(client, false);
        }
    }

    /// The client is gone, and its sessions' close policies apply.
    pub(super) fn drop_client(&mut self, client: Token, reason: &str) {
        let Some(entry) = self.clients.remove(&client) else {
            return;
        };
        self.drop_source(client);
        self.waiting.retain(|waiting| waiting.client != client);
        log!(
            "client {client} left ({reason}) with {} session(s)",
            self.sessions_of(client).len()
        );
        self.client_gone(client, entry.linger);
    }

    pub(super) fn client_token(&self, client: Token) -> Option<&str> {
        self.clients.get(&client).map(|entry| entry.token.as_str())
    }

    pub(super) fn client_is_over(&self, client: Token) -> bool {
        self.clients.get(&client).is_some_and(|entry| entry.over)
    }
}

pub(super) fn error(
    req: Option<RequestId>,
    session: Option<SessionId>,
    message: impl Into<String>,
) -> ToClient {
    ToClient::Error(ErrorMessage {
        req,
        session,
        message: message.into(),
    })
}

fn queue(conn: &mut Conn, message: ToClient) {
    match message.encode() {
        Ok(frame) => conn.queue(&frame),
        Err(error) => log!("can't encode a message: {error}"),
    }
}
