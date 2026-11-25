//! The core's agent sessions (OpenSpec change `agents-sessions`, design
//! decisions 4, 9, 10, 11, 13 and 15): an ACP agent run by a holder on
//! plain pipes, whose ACP client the core is. The session's state is an
//! [`AgentSession`] from `acp::reduce`; this shell only gathers what
//! happens to it, calls [`reduce::step`] and applies the [`Output`]
//! (`fcis R1`, `R9`). Everything else (ownership, close policies,
//! hang-ups, layouts, replays' ordering) is the terminal sessions' code in
//! `sessions.rs`, which this module's [`Agent`] sits in as the session's
//! [`Content::Agent`](super::sessions::Content).
//!
//! - **Starting:** `SPAWN_AGENT` (client protocol 4 only) is refused with
//!   an empty command; its directory is resolved to the absolute one the
//!   agent's `session/new` gets ([`platform::agent_directory`]), which is
//!   the spawn's and the listed `cwd`; it goes to a holder that speaks H1
//!   3 as `SPAWN_PIPES`. Its `SPAWNED` reaches the client before any of
//!   its events; then `initialize` goes out ([`Input::Spawned`]).
//! - **Stepping:** the agent's stdout, the holder's exit and stderr tail,
//!   the attached client's commands and the reducer's own wake-ups
//!   ([`Output::wake`]) each step the reducer, under `catch_unwind`: a
//!   panic is that session's ACP handling failing ([`Input::Fault`]), and
//!   the agent runs on until the session is closed (`process R3`).
//!   `BRINDLE_TEST_ACP_PANIC` (debug builds) injects one.
//! - **Applying:** the events go to the attached client as `TRANSCRIPT`
//!   (unless it waits for its replay, which has them), then a checkpoint
//!   when [`policy::agent_checkpoint_due`] says so, then the bytes for the
//!   agent's stdin as `INPUT`, then the hang-up: so a prompt's checkpoint
//!   is in the holder's connection before its bytes.
//! - **Checkpoints** are `acp::checkpoint`'s encoding; `covers` is the
//!   stdout fed since the last one ([`crate::acp::lines::Lines::covers`]).
//! - **Adopting:** the holder's checkpoint is kept until its tail comes;
//!   then [`checkpoint::adopt`] rebuilds the session and the tail is fed
//!   as stdout. A session that exited while no core ran gets its exit once
//!   its stderr tail follows the tail.
//! - **Ending** (change `agents-session-recovery`, decisions 5 and 6): an
//!   agent that exits, or whose holder is lost, steps `Input::Exited` or
//!   `Input::Lost`; when it can continue its conversation
//!   ([`Agent::continuable`]) and a client has the session or may return,
//!   [`policy::end_fate`] keeps it as `Life::AgentEnded`, in this core's
//!   memory only (no program, no holder, no `EXIT`). A `DETACH`, `CLOSE`,
//!   `KILL` or `STOP` ends it (`EXIT { code, lost: None }`), as does its
//!   client's leaving unless it lingers ([`policy::ended_gone_fate`]).
//! - **Continuing:** `AGENT_RESTART` from the attached client
//!   ([`Core::client_agent_restart`]), refused as
//!   [`policy::restart_refusal`] says or while the core stops, steps
//!   `Input::Restart`, updates the listed command and queues a
//!   `SpawnRequest::Restart`: a `SPAWN_PIPES` under the same session id
//!   in the conversation's `cwd`, on any H1 3 holder of the core's build,
//!   answered `OK` at once and never `SPAWNED`. A `SPAWN_FAILED` (or a
//!   restart abandoned by a hang-up, `STOP` or a takeover) steps
//!   `Exited { None, message }` and returns it to ended; a holder lost
//!   before its `SPAWNED` places it again. A `SPAWN_AGENT` naming a
//!   `conversation` starts as [`Agent::continuing`] in its `cwd`. Either
//!   is checkpointed right after its `SPAWN_PIPES`, so a core killed
//!   before `SPAWNED` still continues it. The reducer chooses resume or
//!   load and mutes or shows the replay.
//! - **Listing:** [`Agent::info`] maps the log's [`Activity`] onto the
//!   wire's [`AgentActivity`] with an exhaustive `match` ([`activity`]).

use std::panic::{self, AssertUnwindSafe};
use std::sync::OnceLock;
use std::time::Instant;

use brindle_remote_protocol::client::{self, AgentActivity, AgentInfo, ToClient};
use brindle_remote_protocol::holder::{self as h1, ToHolder};
use brindle_remote_protocol::serde_json::Value;
use brindle_remote_protocol::{RequestId, SessionId};
use brindle_transcript::{Activity, AgentState, Continuation, Conversation, EntryId, Event};

use super::clients::error;
use super::holders::WaitingSpawn;
use super::policy::{self, AgentCheckpoint, AgentRequest};
use super::sessions::{Life, SpawnRequest};
use super::{Core, log};
use crate::acp::checkpoint;
use crate::acp::reduce::{self, AgentSession, Command, Input, Output, Refused};
use crate::server::event_loop::{EventLoop, Token};
use crate::server::fault;
use crate::server::platform;

/// The version an agent's `initialize` names as the client's: the core's.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Where an agent session's ACP state is.
enum Acp {
    /// Adopted from a holder, whose checkpoint came (`checkpoint`, empty
    /// when the core never stored one) and whose tail hasn't yet.
    Adopting { checkpoint: Vec<u8> },
    /// The conversation, stepped by [`reduce::step`].
    Running(Box<AgentSession>),
}

/// An agent session's state in the core.
pub(super) struct Agent {
    acp: Acp,
    /// [`Lines::covers`](crate::acp::lines::Lines::covers) at the last
    /// checkpoint: what the next one covers starts there.
    checkpointed: u64,
    /// The holder's `STDERR_TAIL`, until its `EXITED`.
    stderr: Option<Vec<u8>>,
    /// When the reducer asked to be stepped with [`Input::Tick`].
    wake: Option<Instant>,
    /// Its checkpoint came out too large to store, so its handling failed
    /// and it isn't checkpointed again (the bounds make this unreachable).
    unstorable: bool,
}

impl Agent {
    /// A new agent session working in `cwd` (absolute).
    pub(super) fn new(cwd: String) -> Self {
        Self::with(Acp::Running(Box::new(AgentSession::new(cwd, VERSION.to_owned()))))
    }

    /// A new agent session continuing `conversation` (a `SPAWN_AGENT` that
    /// names one), into a fresh log, in the conversation's directory.
    pub(super) fn continuing(conversation: &client::AgentConversation) -> Self {
        let conversation = Conversation {
            id: conversation.id.clone(),
            cwd: conversation.cwd.clone(),
            // What the new agent offers decides; the reducer reads that.
            continuation: Continuation::Neither,
        };
        Self::with(Acp::Running(Box::new(AgentSession::continuing(
            conversation,
            VERSION.to_owned(),
        ))))
    }

    /// Its agent is started again ([`Input::Restart`]): the new holder
    /// session's stdout counts from 0, and no stderr tail is kept.
    pub(super) fn restarted(&mut self) {
        self.checkpointed = 0;
        self.stderr = None;
        self.wake = None;
    }

    /// Whether its agent ended and its conversation can be continued
    /// ([`AgentSession::continuable`]).
    pub(super) fn continuable(&self) -> bool {
        match &self.acp {
            Acp::Adopting { .. } => false,
            Acp::Running(acp) => acp.continuable(),
        }
    }

    /// An agent session adopted from a holder, waiting for its checkpoint
    /// and tail.
    pub(super) fn adopting() -> Self {
        Self::with(Acp::Adopting {
            checkpoint: Vec::new(),
        })
    }

    fn with(acp: Acp) -> Self {
        Self {
            acp,
            checkpointed: 0,
            stderr: None,
            wake: None,
            unstorable: false,
        }
    }

    /// The holder's stored checkpoint, while adopting.
    pub(super) fn checkpoint_read(&mut self, bytes: &[u8]) {
        if let Acp::Adopting { checkpoint } = &mut self.acp {
            *checkpoint = bytes.to_vec();
        }
    }

    /// What `LIST` says of it: its activity, and the name its agent gave
    /// in `initialize` while it is ready. One still being adopted is
    /// `working`, as a starting agent is.
    pub(super) fn info(&self) -> AgentInfo {
        match &self.acp {
            Acp::Adopting { .. } => AgentInfo {
                activity: AgentActivity::Working,
                name: None,
            },
            Acp::Running(acp) => {
                let transcript = acp.transcript();
                let name = match transcript.agent() {
                    AgentState::Ready { name, .. } => name.clone(),
                    AgentState::Starting
                    | AgentState::LoginRequired { .. }
                    | AgentState::LoggingIn { .. }
                    | AgentState::Failed { .. }
                    | AgentState::Exited { .. }
                    | AgentState::Lost { .. } => None,
                };
                AgentInfo {
                    activity: activity(transcript.activity()),
                    name,
                }
            }
        }
    }

    /// The events that rebuild its log, encoded for `TRANSCRIPT`.
    pub(super) fn replay(&self) -> Vec<Value> {
        match &self.acp {
            Acp::Adopting { .. } => Vec::new(),
            Acp::Running(acp) => acp.transcript().replay().iter().map(Event::to_value).collect(),
        }
    }
}

/// The listing's activity for the log's (decision 15): the same five
/// names, one to one.
pub(super) fn activity(activity: Activity) -> AgentActivity {
    match activity {
        Activity::Idle => AgentActivity::Idle,
        Activity::Working => AgentActivity::Working,
        Activity::Blocked => AgentActivity::Blocked,
        Activity::Done => AgentActivity::Done,
        Activity::Error => AgentActivity::Error,
    }
}

/// [`reduce::step`] under `catch_unwind`: a panic fails only this
/// session's ACP handling, as [`Input::Fault`] with the panic's text. A
/// panic while failing it too leaves the session as it is, logged.
fn guarded_step(id: SessionId, acp: &mut AgentSession, input: Input<'_>, now: Instant) -> Output {
    let stepped = panic::catch_unwind(AssertUnwindSafe(|| {
        inject_fault(&input);
        reduce::step(acp, input, now)
    }));
    let reason = match stepped {
        Ok(output) => return output,
        Err(payload) => fault::describe(&*payload),
    };
    log!("session {id}: ACP handling panicked ({reason}); its agent runs on, refusing prompts");
    let failed = panic::catch_unwind(AssertUnwindSafe(|| {
        reduce::step(acp, Input::Fault { reason }, now)
    }));
    failed.unwrap_or_else(|payload| {
        log!(
            "session {id}: failing its ACP handling panicked too ({})",
            fault::describe(&*payload)
        );
        Output::default()
    })
}

/// Panics when the agent's stdout or a prompt holds
/// `BRINDLE_TEST_ACP_PANIC`'s marker (debug builds only).
fn inject_fault(input: &Input<'_>) {
    static MARKER: OnceLock<Option<Vec<u8>>> = OnceLock::new();
    let Some(marker) = fault::marker(fault::ACP_PANIC, &MARKER) else {
        return;
    };
    let bytes: &[u8] = match input {
        Input::Stdout(bytes) => bytes,
        Input::Command(Command::Prompt { text }) => text.as_bytes(),
        Input::Spawned
        | Input::OutputLost
        | Input::Exited { .. }
        | Input::Lost { .. }
        | Input::Restart
        | Input::Command(
            Command::Unqueue
            | Command::Resend
            | Command::Cancel
            | Command::Answer { .. }
            | Command::Login { .. },
        )
        | Input::Fault { .. }
        | Input::Tick => return,
    };
    if fault::holds(bytes, marker) {
        panic!("{} matched", fault::ACP_PANIC);
    }
}

/// A protocol 4 client's command for an agent session.
pub(super) struct AgentCommand {
    pub(super) request: AgentRequest,
    /// The request its `OK` or `ERROR` answers; `None` for `AGENT_CANCEL`.
    pub(super) req: Option<RequestId>,
    pub(super) session: SessionId,
    pub(super) command: Command,
}

impl AgentCommand {
    pub(super) fn prompt(prompt: client::AgentPrompt) -> Self {
        Self {
            request: AgentRequest::Prompt,
            req: Some(prompt.req),
            session: prompt.session,
            command: Command::Prompt { text: prompt.text },
        }
    }

    pub(super) fn cancel(target: client::SessionRef) -> Self {
        Self {
            request: AgentRequest::Cancel,
            req: None,
            session: target.session,
            command: Command::Cancel,
        }
    }

    pub(super) fn answer(answer: client::AgentPermission) -> Self {
        Self {
            request: AgentRequest::Permission,
            req: Some(answer.req),
            session: answer.session,
            command: Command::Answer {
                entry: EntryId(answer.entry),
                option: answer.option,
            },
        }
    }

    pub(super) fn unqueue(request: client::AgentRequest) -> Self {
        Self {
            request: AgentRequest::Unqueue,
            req: Some(request.req),
            session: request.session,
            command: Command::Unqueue,
        }
    }

    pub(super) fn resend(request: client::AgentRequest) -> Self {
        Self {
            request: AgentRequest::Resend,
            req: Some(request.req),
            session: request.session,
            command: Command::Resend,
        }
    }

    /// `AGENT_LOGIN` (OpenSpec change `agents-login`).
    pub(super) fn login(login: client::AgentLogin) -> Self {
        Self {
            request: AgentRequest::Login,
            req: Some(login.req),
            session: login.session,
            command: Command::Login {
                method: login.method,
            },
        }
    }
}

impl<L: EventLoop> Core<L> {
    /// `SPAWN_AGENT`: refused below protocol 4 and with no command; else
    /// queued for a holder that runs pipe sessions, in the absolute
    /// directory its agent will work in. One that continues a conversation
    /// works in the conversation's directory, which ACP ties it to.
    pub(super) fn client_spawn_agent(&mut self, client: Token, spawn: client::SpawnAgent) {
        let protocol = self.clients.get(&client).map_or(0, |entry| entry.protocol);
        let refusal = policy::agent_request_refusal(protocol, AgentRequest::Spawn)
            .map(|refusal| refusal.to_string())
            .or_else(|| policy::agent_spawn_refusal(&spawn.command).map(str::to_owned));
        if let Some(refusal) = refusal {
            return self.send_client(client, error(Some(spawn.req), None, refusal));
        }
        let cwd = match &spawn.conversation {
            Some(conversation) => conversation.cwd.clone(),
            None => platform::agent_directory(spawn.cwd.as_deref(), &spawn.env),
        };
        self.queue_spawn(client, SpawnRequest::Agent { spawn, cwd });
    }

    /// `AGENT_RESTART` (change `agents-session-recovery`, decision 5): an
    /// ended agent session the client has attached is started again with
    /// the command given, in its conversation's directory, under its own
    /// id, key and meta (the command updated), on any holder of the
    /// core's build. Refused below protocol 4, for a session the client
    /// hasn't attached, while the core stops or steps down, and as
    /// [`policy::restart_refusal`] says; the reducer's own refusal
    /// ([`Input::Restart`]) comes last. `OK` once the spawn is queued; what
    /// follows comes as transcript events.
    pub(super) fn client_agent_restart(&mut self, client: Token, restart: client::AgentRestart) {
        let (req, id) = (restart.req, restart.session);
        let protocol = self.clients.get(&client).map_or(0, |entry| entry.protocol);
        if let Some(refusal) = policy::agent_request_refusal(protocol, AgentRequest::Restart) {
            return self.send_client(client, error(Some(req), Some(id), refusal.to_string()));
        }
        let stopping = self.stopping.is_some() || self.stepping_down.is_some();
        let Some(session) = self.attached_session(client, id, Some(req)) else {
            return;
        };
        let life = session.life.clone();
        let Some(agent) = session.agent() else {
            let message = format!("session {id} isn't an agent session");
            return self.send_client(client, error(Some(req), Some(id), message));
        };
        let continuable = agent.continuable();
        let cwd = match &agent.acp {
            Acp::Running(acp) => acp.transcript().conversation().map(|c| c.cwd.clone()),
            Acp::Adopting { .. } => None,
        };
        let refusal = if stopping {
            Some("the server is stopping; try again".to_owned())
        } else {
            policy::restart_refusal(&life, continuable, &restart.command).map(|r| r.to_string())
        };
        if let Some(refusal) = refusal {
            let message = format!("session {id}: {refusal}");
            return self.send_client(client, error(Some(req), Some(id), message));
        }
        let Some(cwd) = cwd else {
            let message = format!("session {id}: {}", Refused::CannotContinue);
            return self.send_client(client, error(Some(req), Some(id), message));
        };
        match self.agent_step(id, Input::Restart) {
            Some(Ok(())) => {}
            Some(Err(refused)) => {
                let message = format!("session {id}: {refused}");
                return self.send_client(client, error(Some(req), Some(id), message));
            }
            None => {
                let message = format!("session {id}: the terminal server's ACP handling failed");
                return self.send_client(client, error(Some(req), Some(id), message));
            }
        }
        log!(
            "session {id}: its agent starts again to continue its conversation ({:?})",
            restart.command
        );
        let command = (restart.command.clone(), restart.args.clone());
        let spawn = SpawnRequest::Restart { restart, cwd };
        self.start_again(id, &command.0, &command.1, spawn.clone());
        self.send_client(client, ToClient::Ok(client::Request { req }));
        self.waiting.push(WaitingSpawn { client, id, spawn });
        self.place_waiting();
    }

    /// A protocol 4 client's command for an agent session it has attached:
    /// stepped, its events sent, then its reply (`OK` or an `ERROR` naming
    /// the session; none for a cancel the reducer took).
    pub(super) fn client_agent_command(&mut self, client: Token, command: AgentCommand) {
        let AgentCommand {
            request,
            req,
            session: id,
            command,
        } = command;
        let protocol = self.clients.get(&client).map_or(0, |entry| entry.protocol);
        if let Some(refusal) = policy::agent_request_refusal(protocol, request) {
            return self.send_client(client, error(req, Some(id), refusal.to_string()));
        }
        let Some(session) = self.attached_session(client, id, req) else {
            return;
        };
        let running = match session.agent().map(|agent| &agent.acp) {
            None => Err(format!("session {id} isn't an agent session")),
            Some(Acp::Adopting { .. }) => Err(format!("session {id}: {}", Refused::NotReady)),
            Some(Acp::Running(_)) => Ok(()),
        };
        if let Err(message) = running {
            return self.send_client(client, error(req, Some(id), message));
        }
        match self.agent_step(id, Input::Command(command)) {
            Some(Ok(())) => {
                if let Some(req) = req {
                    self.send_client(client, ToClient::Ok(client::Request { req }));
                }
            }
            Some(Err(refused)) => {
                self.send_client(client, error(req, Some(id), format!("session {id}: {refused}")));
            }
            // The step panicked: the command wasn't taken, and the log
            // says the handling failed.
            None => {
                let message = format!("session {id}: the terminal server's ACP handling failed");
                self.send_client(client, error(req, Some(id), message));
            }
        }
    }

    /// Steps agent session `id` with `input` and applies what it says;
    /// gives the reply to a client's command. Nothing for a session that
    /// isn't a running agent's.
    pub(super) fn agent_step(
        &mut self,
        id: SessionId,
        input: Input<'_>,
    ) -> Option<Result<(), Refused>> {
        let now = Instant::now();
        let agent = self.sessions.get_mut(&id)?.agent()?;
        let Acp::Running(acp) = &mut agent.acp else {
            return None;
        };
        let output = guarded_step(id, acp, input, now);
        self.agent_apply(id, output)
    }

    /// Applies a step's output, in its order: the events to the attached
    /// client (unless a replay it waits for has them), a checkpoint when
    /// due, the agent's stdin, the hang-up. Returns the reply.
    fn agent_apply(&mut self, id: SessionId, output: Output) -> Option<Result<(), Refused>> {
        let Output {
            events,
            stdin,
            reply,
            checkpoint,
            hang_up,
            wake,
        } = output;
        let session = self.sessions.get_mut(&id)?;
        let client = session
            .ownership
            .client()
            .filter(|_| session.pending_replay.is_none());
        let live = session.life.is_live();
        let holder = session.holder;
        let agent = session.agent()?;
        agent.wake = wake;
        let since = match &agent.acp {
            Acp::Running(acp) => acp.lines().covers().saturating_sub(agent.checkpointed),
            Acp::Adopting { .. } => 0,
        };
        if let Some(client) = client
            && !events.is_empty()
        {
            self.send_transcript(client, id, events.iter().map(Event::to_value).collect());
        }
        if policy::agent_checkpoint_due(AgentCheckpoint::Stepped(checkpoint), since) {
            self.checkpoint(id);
        }
        // Nothing reaches an agent being hung up, or gone.
        if !stdin.is_empty() && live {
            self.send_holder(
                holder,
                ToHolder::Input(h1::Data {
                    session: id,
                    bytes: stdin,
                }),
            );
        }
        if hang_up {
            self.hang_up(id);
        }
        reply
    }

    /// Sends `events` of session `id` to `client` as `TRANSCRIPT` frames of
    /// whole events. One that can't fit a frame (the log's bounds rule it
    /// out) is reported to the client and logged.
    pub(super) fn send_transcript(&mut self, client: Token, id: SessionId, events: Vec<Value>) {
        match client::pack_transcript(id, events) {
            Ok(messages) => {
                for message in messages {
                    self.send_client(client, ToClient::Transcript(message));
                }
            }
            Err(failure) => {
                log!("session {id}: can't send its transcript: {failure}");
                let message = format!("the terminal server couldn't send its transcript: {failure}");
                self.send_client(client, error(None, Some(id), message));
            }
        }
    }

    /// Checkpoints agent session `id` (its liveness already checked by
    /// [`Core::checkpoint`]): its encoded state, covering the stdout fed
    /// since the last one. A state too large to store fails its handling.
    pub(super) fn agent_checkpoint(&mut self, id: SessionId) {
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        let holder = session.holder;
        let Some(agent) = session.agent() else {
            return;
        };
        let Acp::Running(acp) = &agent.acp else {
            return;
        };
        if agent.unstorable {
            return;
        }
        match checkpoint::encode(acp) {
            Ok(bytes) => {
                let covers = acp.lines().covers();
                let since = covers.saturating_sub(agent.checkpointed);
                agent.checkpointed = covers;
                for message in ToHolder::checkpoint(id, &bytes, since) {
                    self.send_holder(holder, message);
                }
            }
            Err(too_large) => {
                log!("session {id}: {too_large}; failing its ACP handling");
                agent.unstorable = true;
                self.agent_step(
                    id,
                    Input::Fault {
                        reason: too_large.to_string(),
                    },
                );
            }
        }
    }

    /// The holder's `STDERR_TAIL`: kept for the `EXITED` that follows; a
    /// session adopted as exited (it ended while no core ran) has its exit
    /// stepped now, since its tail is all that was still to come.
    pub(super) fn agent_stderr(&mut self, holder: Token, id: SessionId, bytes: Vec<u8>) {
        let Some(session) = self.sessions.get_mut(&id) else {
            return log!("holder {holder}: a stderr tail for session {id}, which it doesn't have");
        };
        if session.holder != holder {
            return log!("holder {holder} reported session {id}, which isn't its");
        }
        let exited = match session.life {
            Life::ExitedAwaitingOwner { code, .. } => Some(code),
            Life::Starting { .. }
            | Life::Running { .. }
            | Life::HungUp { .. }
            | Life::AgentEnded { .. } => None,
        };
        let Some(agent) = session.agent() else {
            return log!("holder {holder}: a stderr tail for terminal session {id}; ignored");
        };
        agent.stderr = Some(bytes);
        if let Some(code) = exited {
            self.agent_exited(id, code);
        }
    }

    /// Agent session `id`'s program exited (or was lost with its holder):
    /// its exit is stepped with the stderr tail that came before, so the
    /// log has it before the session's `EXIT`. Nothing for a terminal.
    pub(super) fn agent_exited(&mut self, id: SessionId, code: Option<i32>) {
        let Some(agent) = self.sessions.get_mut(&id).and_then(|session| session.agent()) else {
            return;
        };
        let stderr = agent.stderr.take().unwrap_or_default();
        self.agent_step(
            id,
            Input::Exited {
                code,
                stderr: &stderr,
            },
        );
    }

    /// An adopted agent session's tail came after its checkpoint: the
    /// session is rebuilt ([`checkpoint::adopt`], under `catch_unwind`),
    /// what that did is applied, and the tail is fed as stdout.
    pub(super) fn agent_adopted(&mut self, id: SessionId, truncated: bool, tail: &[u8]) {
        let now = Instant::now();
        let Some(session) = self.sessions.get_mut(&id) else {
            return;
        };
        let cwd = session.meta.cwd.clone().unwrap_or_default();
        let Some(agent) = session.agent() else {
            return;
        };
        let Acp::Adopting { checkpoint: stored } = &agent.acp else {
            return;
        };
        let adopted = panic::catch_unwind(AssertUnwindSafe(|| {
            checkpoint::adopt(stored, truncated, &cwd, VERSION, now)
        }));
        let (acp, output) = match adopted {
            Ok(adopted) => {
                if let Err(why) = &adopted.restored {
                    log!("session {id}: {}: {why}", checkpoint::NOT_RESTORED);
                }
                (adopted.session, adopted.output)
            }
            Err(payload) => {
                let reason = fault::describe(&*payload);
                log!("session {id}: adopting its conversation panicked ({reason})");
                let mut acp = AgentSession::new(cwd, VERSION.to_owned());
                let output = guarded_step(id, &mut acp, Input::Fault { reason }, now);
                (acp, output)
            }
        };
        if truncated {
            log!(
                "session {id}: its stdout was truncated; adopted from its checkpoint and the last {} bytes",
                tail.len()
            );
        }
        agent.checkpointed = acp.lines().covers();
        agent.acp = Acp::Running(Box::new(acp));
        self.agent_apply(id, output);
        if !tail.is_empty() {
            self.agent_step(id, Input::Stdout(tail));
        }
    }

    /// Steps every agent session whose wake-up is due at `now`.
    pub(super) fn tick_agents(&mut self, now: Instant) {
        let due: Vec<SessionId> = self
            .sessions
            .iter_mut()
            .filter_map(|(&id, session)| {
                let agent = session.agent()?;
                agent.wake.is_some_and(|wake| wake <= now).then_some(id)
            })
            .collect();
        for id in due {
            self.agent_step(id, Input::Tick);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_activity_is_listed_by_its_own_name() {
        let pairs = [
            (Activity::Idle, AgentActivity::Idle),
            (Activity::Working, AgentActivity::Working),
            (Activity::Blocked, AgentActivity::Blocked),
            (Activity::Done, AgentActivity::Done),
            (Activity::Error, AgentActivity::Error),
        ];
        for (log, listed) in pairs {
            assert_eq!(activity(log), listed);
        }
    }

    #[test]
    fn a_new_agent_is_listed_working_and_replays_its_starting_state() {
        let agent = Agent::new("/home/u".into());
        assert_eq!(
            agent.info(),
            AgentInfo {
                activity: AgentActivity::Working,
                name: None
            }
        );
        let events: Vec<Event> = agent
            .replay()
            .iter()
            .map(|value| Event::from_value(value).unwrap())
            .collect();
        assert!(
            events.iter().any(|event| matches!(event, Event::Agent { state: AgentState::Starting })),
            "{events:?}"
        );

        let adopting = Agent::adopting();
        assert_eq!(adopting.info().activity, AgentActivity::Working);
        assert!(adopting.replay().is_empty());
    }

    #[test]
    fn a_fault_fails_the_agent_with_its_reason_and_leaves_it_running() {
        let mut acp = AgentSession::new("/home/u".into(), "test".into());
        let now = Instant::now();
        let output = guarded_step(1, &mut acp, Input::Spawned, now);
        assert!(!output.stdin.is_empty(), "initialize goes out");
        // What `guarded_step` steps when the reducer panics: the agent
        // fails with the panic's text.
        let caught = panic::catch_unwind(|| panic!("boom")).unwrap_err();
        let reason = fault::describe(&*caught);
        let output = reduce::step(&mut acp, Input::Fault { reason }, now);
        assert!(matches!(
            acp.transcript().agent(),
            AgentState::Failed { message } if message.contains("boom")
        ));
        assert!(!output.hang_up, "the agent runs on");
    }
}
