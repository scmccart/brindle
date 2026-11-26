//! The core's client connections: accepting with the peer check, the
//! `HELLO`/`WELCOME` handshake (with what the client asked for: a linger
//! for its `kill` sessions, process info), the takeover of a stale
//! connection with the same token, requests, and the per-client queues
//! whose length pauses the client's sessions.

use std::io;
use std::time::{Duration, Instant};

use brindle_remote_protocol::client::{self, ErrorMessage, Hello, ToClient, ToCore, Welcome};
use brindle_remote_protocol::{RequestId, SessionId};

use super::agents::AgentCommand;
use super::policy::{self, ClientOptions, Mark};
use super::{Core, DEAD_CLIENT_READ_LIMIT, Source, log};
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
/// How long a refused client gets to read its `ERROR` and the end
/// ([`refuse`]).
const REFUSAL_CLOSE_LIMIT: Duration = Duration::from_secs(1);

/// A connection that hasn't sent `HELLO` yet.
pub(super) struct Pending {
    conn: Conn,
    since: Instant,
}

/// A client's `client_token`, which its sessions' owner is matched by
/// ([`policy::returning`]): never empty (`types R6`, `R7`). It is decoded
/// once, from a `HELLO` or a holder's stored meta.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(super) struct ClientToken(String);

impl ClientToken {
    /// The token a `HELLO` or a stored meta names: an empty one names none,
    /// and owns nothing.
    pub(super) fn from_wire(token: &str) -> Option<Self> {
        if token.is_empty() {
            None
        } else {
            Some(Self(token.to_string()))
        }
    }

    pub(super) fn as_str(&self) -> &str {
        &self.0
    }
}

pub(super) struct Client {
    pub(super) conn: Conn,
    pub(super) interest: Interest,
    /// The `client_token` from its `HELLO`; `None` when it named none.
    pub(super) token: Option<ClientToken>,
    /// Its queue passed [`CLIENT_QUEUE_HIGH`](super::CLIENT_QUEUE_HIGH), so
    /// its sessions are paused ([`policy::queue_mark`]).
    pub(super) over: bool,
    /// How long its `kill` sessions wait for it once its connection ends:
    /// its `HELLO`'s `linger_secs`, at most
    /// [`MAX_LINGER`](super::MAX_LINGER) ([`policy::client_options`]).
    linger: Duration,
    /// It speaks protocol 2 and asked for `PROCESS`.
    pub(super) process_info: bool,
    /// The client protocol its `HELLO` negotiated.
    pub(super) protocol: u32,
    /// A write to it failed: what it sent is being handled before it is
    /// dropped ([`Core::drop_failed_client`]), and nothing more is sent.
    failed: bool,
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
                        queue_or_log(
                            &mut conn,
                            &error(None, None, "the first message must be HELLO"),
                        );
                        refuse(&mut conn);
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
                queue_or_log(&mut conn, &ToClient::Error(refusal));
                refuse(&mut conn);
                self.drop_source(token);
                return;
            }
        };
        let holder_protocols =
            policy::holder_protocols(self.holders.values().map(|holder| holder.protocol()));
        queue_or_log(
            &mut conn,
            &ToClient::Welcome(Welcome {
                protocol,
                core_version: env!("CARGO_PKG_VERSION").into(),
                core_build: self.build.clone(),
                pid: std::process::id(),
                holder_protocols,
            }),
        );
        let ClientOptions {
            linger,
            process_info,
        } = policy::client_options(protocol, &hello);
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
        let client_token = ClientToken::from_wire(&hello.client_token);
        if let Some(client_token) = &client_token {
            self.take_over(token, client_token);
        }
        self.sources.insert(token, Source::Client);
        self.clients.insert(
            token,
            Client {
                conn,
                interest: Interest::READABLE,
                token: client_token,
                over: false,
                linger,
                process_info,
                protocol,
                failed: false,
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
    /// sessions still starting are hung up ([`Core::client_replaced`]). A
    /// `HELLO` with no token owns nothing, so it replaces nothing.
    fn take_over(&mut self, new: Token, client_token: &ClientToken) {
        let stale: Vec<Token> = self
            .clients
            .iter()
            .filter(|(_, client)| client.token.as_ref() == Some(client_token))
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
                        Some(waiting.spawn.req()),
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
            self.client_replaced(old, Instant::now());
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
            ToCore::SetLayout(set) => self.client_set_layout(client, set),
            ToCore::Attach(attach) => self.client_attach(client, attach),
            ToCore::Detach(detach) => self.client_detach(client, detach),
            ToCore::List(request) => self.client_list(client, request.req),
            ToCore::Kill(kill) => self.client_kill(client, kill),
            ToCore::Status(request) => self.client_status(client, request.req),
            ToCore::Stop(request) => self.client_stop(client, request.req),
            ToCore::StepDown(request) => self.client_step_down(client, request.req),
            // Client protocol 4's agent sessions (`agents.rs`); each is
            // refused below 4.
            ToCore::SpawnAgent(spawn) => self.client_spawn_agent(client, spawn),
            ToCore::AgentPrompt(prompt) => {
                self.client_agent_command(client, AgentCommand::prompt(prompt))
            }
            ToCore::AgentCancel(target) => {
                self.client_agent_command(client, AgentCommand::cancel(target))
            }
            ToCore::AgentPermission(answer) => {
                self.client_agent_command(client, AgentCommand::answer(answer))
            }
            ToCore::AgentUnqueue(request) => {
                self.client_agent_command(client, AgentCommand::unqueue(request))
            }
            ToCore::AgentResend(request) => {
                self.client_agent_command(client, AgentCommand::resend(request))
            }
        }
    }

    /// Queues a message for a client. Past
    /// [`CLIENT_QUEUE_HIGH`](super::CLIENT_QUEUE_HIGH) the client's sessions
    /// are paused in their holders. A client whose write failed is sent
    /// nothing: it is gone.
    pub(super) fn send_client(&mut self, client: Token, message: ToClient) {
        let Some(entry) = self.clients.get_mut(&client) else {
            return;
        };
        if entry.failed {
            return;
        }
        if let Err(failure) = queue(&mut entry.conn, &message) {
            // An `ERROR` for the same request and session in its place.
            let replacement = unsendable(&message, &failure);
            log!("client {client}: {}", unsendable_text(&message, &failure));
            if let Err(failure) = queue(&mut entry.conn, &replacement) {
                log!("client {client}: {}", unsendable_text(&replacement, &failure));
            }
        }
        if entry.conn.queued() >= EAGER_FLUSH {
            // A failure shows again in the loop's own flush, which drops
            // the client there.
            let _ = entry.conn.flush();
        }
        match policy::queue_mark(entry.over, entry.conn.queued()) {
            Some(Mark::Pause) => {
                entry.over = true;
                self.pause_client_sessions(client, true);
            }
            // Resuming waits for the loop's flush (`flush_client`).
            Some(Mark::Resume) | None => {}
        }
    }

    /// Sends what the socket takes; back under
    /// [`CLIENT_QUEUE_LOW`](super::CLIENT_QUEUE_LOW), the client's sessions
    /// are resumed. A failed write drops the client once what it sent is
    /// handled ([`Core::drop_failed_client`]).
    pub(super) fn flush_client(&mut self, client: Token) {
        let Some(entry) = self.clients.get_mut(&client) else {
            return;
        };
        if let Err(error) = entry.conn.flush() {
            return self.drop_failed_client(client, &format!("write failed: {error}"));
        }
        match policy::queue_mark(entry.over, entry.conn.queued()) {
            Some(Mark::Resume) => {
                entry.over = false;
                self.pause_client_sessions(client, false);
            }
            // A flush only shrinks its queue.
            Some(Mark::Pause) | None => {}
        }
    }

    /// A write to the client failed, so it is gone, but what it sent before
    /// it went still counts, as in [`Core::read_client`]: a client that
    /// detaches its sessions and then closes (Brindle quitting) may be found
    /// gone by a write before the core has read its `DETACH`es. All of it is
    /// read, up to [`DEAD_CLIENT_READ_LIMIT`] rather than one turn's budget,
    /// since this connection has no later turn, and its frames are handled
    /// before the close policies apply. What would answer them is dropped.
    fn drop_failed_client(&mut self, client: Token, reason: &str) {
        let Some(entry) = self.clients.get_mut(&client) else {
            return;
        };
        entry.failed = true;
        // A read that fails after some bytes keeps them.
        let _ = entry.conn.read_up_to(DEAD_CLIENT_READ_LIMIT);
        self.process_client_frames(client);
        self.drop_client(client, reason);
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
        self.client_gone(client, entry.linger, Instant::now());
    }

    /// The token of a connected client, if its `HELLO` named one.
    pub(super) fn client_token(&self, client: Token) -> Option<&ClientToken> {
        self.clients
            .get(&client)
            .and_then(|entry| entry.token.as_ref())
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

/// Queues `message` on the client's connection, or returns why it can't be
/// encoded (a payload over the frame limit, say).
fn queue(conn: &mut Conn, message: &ToClient) -> io::Result<()> {
    conn.queue(&message.encode()?);
    Ok(())
}

/// Queues a message for a connection still in its handshake, which has no
/// request or session to answer for; one that can't be encoded is logged.
fn queue_or_log(conn: &mut Conn, message: &ToClient) {
    if let Err(error) = queue(conn, message) {
        log!("{}", unsendable_text(message, &error));
    }
}

/// The `ERROR` a client gets in place of `message`, which `failure` kept
/// from being encoded: for the same request and session, and always small.
pub(super) fn unsendable(message: &ToClient, failure: &io::Error) -> ToClient {
    let (req, session) = match message {
        ToClient::Welcome(_) => (None, None),
        ToClient::Error(m) => (m.req, m.session),
        ToClient::Spawned(m) => (Some(m.req), Some(m.session)),
        ToClient::Data(m) => (None, Some(m.session)),
        ToClient::Exit(m) => (None, Some(m.session)),
        ToClient::ReplayStart(m) => (m.req, Some(m.session)),
        ToClient::ReplayEnd(m) => (None, Some(m.session)),
        ToClient::Sessions(m) => (Some(m.req), None),
        ToClient::ServerStatus(m) => (Some(m.req), None),
        ToClient::Ok(m) => (Some(m.req), None),
        ToClient::Process(m) => (None, Some(m.session)),
        ToClient::Transcript(m) => (None, Some(m.session)),
    };
    error(req, session, unsendable_text(message, failure))
}

fn unsendable_text(message: &ToClient, failure: &io::Error) -> String {
    let what = match message {
        ToClient::Welcome(_) => "WELCOME",
        ToClient::Error(_) => "error",
        ToClient::Spawned(_) => "answer to the spawn",
        ToClient::Data(_) => "output",
        ToClient::Exit(_) => "exit",
        ToClient::ReplayStart(_) => "replay",
        ToClient::ReplayEnd(_) => "replay's end",
        ToClient::Sessions(_) => "session list",
        ToClient::ServerStatus(_) => "status",
        ToClient::Ok(_) => "answer",
        ToClient::Process(_) => "process info",
        ToClient::Transcript(_) => "transcript",
    };
    format!("the terminal server couldn't send its {what}: {failure}")
}

/// Closes a refused connection gracefully, within [`REFUSAL_CLOSE_LIMIT`]:
/// the client reads its `ERROR`, then the end, even with bytes of its own
/// still unread, which closing at once would turn into a reset
/// (`process R10`).
fn refuse(conn: &mut Conn) {
    if let Err(error) = conn.close_for(REFUSAL_CLOSE_LIMIT) {
        log!("a refused client didn't close in time: {error}");
    }
}

#[cfg(test)]
mod tests {
    use brindle_remote_protocol::MAX_FRAME_PAYLOAD_SIZE;
    use brindle_remote_protocol::client::{Data, SessionInfo, Sessions};

    use super::*;

    #[test]
    fn an_unencodable_reply_is_replaced_by_an_error_for_its_request() {
        let reply = ToClient::Sessions(Sessions {
            req: 9,
            sessions: vec![SessionInfo {
                command: "x".repeat(MAX_FRAME_PAYLOAD_SIZE),
                ..SessionInfo::default()
            }],
        });
        let failure = reply.encode().unwrap_err();
        let replacement = unsendable(&reply, &failure);
        assert_eq!(
            replacement,
            error(
                Some(9),
                None,
                format!("the terminal server couldn't send its session list: {failure}")
            )
        );
        assert!(replacement.encode().is_ok());

        // A session's message is replaced for that session.
        let output = ToClient::Data(Data {
            session: 3,
            bytes: vec![b'x'; MAX_FRAME_PAYLOAD_SIZE + 1],
        });
        let failure = output.encode().unwrap_err();
        let ToClient::Error(replacement) = unsendable(&output, &failure) else {
            panic!("not an ERROR");
        };
        assert_eq!((replacement.req, replacement.session), (None, Some(3)));
    }

    #[test]
    fn an_empty_client_token_names_no_owner() {
        assert_eq!(ClientToken::from_wire(""), None);
        let token = ClientToken::from_wire("brindle-1").unwrap();
        assert_eq!(token.as_str(), "brindle-1");
        // Anything else is a token, as it is.
        assert_eq!(ClientToken::from_wire(" ").as_ref().map(ClientToken::as_str), Some(" "));
    }
}
