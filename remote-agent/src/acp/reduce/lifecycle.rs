use brindle_remote_protocol::serde_json::Value;
use brindle_transcript::{
    AgentState, Continuation, Conversation, Event, HoldReason, NoticeLevel, PermissionOutcome,
    StopReason, TurnState, clean,
};

use crate::acp::dialect::{self, Dialect, Unsupported};
use crate::acp::envelope::RpcError;
use crate::acp::forms::{AUTH_REQUIRED, Continued, Offered, UNSUPPORTED_VERSION};
use crate::acp::limits::MAX_SESSION_ID;
use crate::acp::lines::Lines;

use super::list::MAX_LISTED_ID;
use super::login::{AfterLogin, Login};
use super::options::Compare;
use super::text::{agent_name, bound};
use super::{Moment, Refused, Step, muted_notice};

/// Where the agent's ACP conversation is: what the transcript's
/// [`AgentState`] can't say (which start request is out, the ACP session
/// id, why the agent failed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Phase {
    /// The holder hasn't said the agent started; what it starts.
    Spawning { start: Start },
    /// `initialize` is sent and not answered.
    Initializing { start: Start },
    /// `session/new` is sent and not answered.
    Creating { greeting: Greeting },
    /// `session/resume` of conversation `conversation` is sent and not
    /// answered (OpenSpec change `agents-session-recovery`, decision 2).
    /// `log` says whether the log holds the conversation's earlier
    /// messages: a fresh one gets a notice that it doesn't.
    Resuming {
        greeting: Greeting,
        conversation: String,
        log: Log,
    },
    /// `session/load` of conversation `conversation` is sent and not
    /// answered: the agent replays it first, muted or shown.
    Loading {
        greeting: Greeting,
        conversation: String,
        replay: Replay,
    },
    /// Takes prompts in the ACP session `session`.
    Ready { session: String },
    /// Initialized with [`Start::Choose`]: no ACP session yet, and nothing
    /// is sent until a client chooses a conversation to continue, or a new
    /// one (OpenSpec change `agents-session-list`, decision 1). Lists are
    /// taken.
    Choosing { greeting: Greeting },
    /// The agent answered `-32000` (decision 17): it waits for a client to
    /// choose one of its login methods (OpenSpec change `agents-login`,
    /// decision 4), and nothing else is sent.
    LoginRequired(Login),
    /// `authenticate` with login method `method` is out
    /// ([`Purpose::Authenticate`] in [`Rpc::outgoing`]), awaited however
    /// long it takes.
    LoggingIn { login: Login, method: String },
    /// `AGENT_RESTART` while the agent needed a login (`agents-login`
    /// decision 8): the agent is hung up, nothing more is read from it, and
    /// once it has exited the core steps [`Input::Restart`] again to spawn
    /// it anew with `start`. Should its exit be stepped instead (the
    /// restart was dropped: the session closed, or its client left), it
    /// ends as any agent does.
    Restarting { start: Start },
    /// The start failed (another ACP version, or an error or unreadable
    /// answer to `initialize`, `session/new`, `session/resume` or
    /// `session/load`, or a continue the agent offers no way to make;
    /// decision 4): the agent is hung up, and its exit leaves the failed
    /// state, adding its stderr as a notice.
    Refused,
    /// The core's handling of this agent failed (decision 13), or its
    /// checkpoint couldn't be restored (decision 11): its lines are
    /// ignored, and it runs until the session is closed.
    Broken,
    /// The agent's process ended.
    Exited,
    /// The agent's holder ended, and the agent with it.
    Lost,
}

/// What a spawned agent starts: a new conversation, or one it continues
/// (decisions 2 and 4 of `agents-session-recovery`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Start {
    /// `session/new` in the session's directory.
    New,
    /// `session/resume` or `session/load` of `conversation`, whichever the
    /// new agent offers, into the log kept from its earlier agent or into a
    /// fresh one.
    Continue {
        conversation: Conversation,
        log: Log,
    },
    /// Nothing after `initialize`: the session waits in
    /// [`Phase::Choosing`] (`agents-session-list`).
    Choose,
}

/// Whether a continued conversation's log is the session's own, kept from
/// its earlier agent (`AGENT_RESTART`), or a fresh one in a new session
/// whose server no longer had it (`SPAWN_AGENT` with a conversation).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Log {
    Kept,
    Fresh,
}

/// What a `session/load`'s replay does to the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Replay {
    /// A kept log already holds it: each update is counted, not applied.
    Muted { updates: u64 },
    /// A fresh log takes it as ordinary updates.
    Shown,
}

/// What `initialize` said that the agent's ready state and conversation
/// carry: its name and version, and what it offers for continuing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Greeting {
    pub name: Option<String>,
    pub version: Option<String>,
    pub continuation: Continuation,
}

/// How a continue goes on once the new agent has answered `initialize`
/// (decision 2's table), from what that agent offers, not the earlier one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choice {
    /// `session/resume`.
    Resume,
    /// `session/load`, its replay muted or shown.
    Load { replay: Replay },
    /// Neither is offered: the continue fails.
    Neither,
}

/// Decision 2's table: a kept log resumes when it can and otherwise loads
/// muted; a fresh log loads (shown) when it can and otherwise resumes.
pub fn choose(log: Log, offered: Continuation) -> Choice {
    match (log, offered) {
        (Log::Kept, Continuation::ResumeAndLoad | Continuation::Resume)
        | (Log::Fresh, Continuation::Resume) => Choice::Resume,
        (Log::Kept, Continuation::Load) => Choice::Load {
            replay: Replay::Muted { updates: 0 },
        },
        (Log::Fresh, Continuation::ResumeAndLoad | Continuation::Load) => Choice::Load {
            replay: Replay::Shown,
        },
        (Log::Kept | Log::Fresh, Continuation::Neither) => Choice::Neither,
    }
}

/// The notice an `AGENT_RESTART` adds before the agent starts again.
pub const RESTARTED: &str = "the agent was started again to continue the conversation";

/// The notice an `AGENT_RESTART` of an agent that needed a login adds
/// (`agents-login` decision 8).
pub const RESTARTED_AFTER_LOGIN: &str = "the agent is started again after its login";

/// The notice a fresh log's shown replay starts with.
pub const REPLAY_SHOWN: &str =
    "this continues an earlier conversation; what follows is the agent's replay of it";

/// The notice a fresh log gets when the agent could only resume.
pub const NOT_REPLAYED: &str =
    "conversation continued; the agent doesn't replay its earlier messages, so they aren't shown";

/// The failure when the continued agent offers neither way to continue.
pub const CANT_CONTINUE: &str =
    "the agent can't continue conversations: it offers neither session/load nor session/resume";

/// A turn's end.
pub(super) enum Ending {
    Stop(StopReason),
    Failed(String),
}

impl Ending {
    pub(super) fn state(&self) -> TurnState {
        match self {
            Self::Stop(stop) => TurnState::Ended { stop: stop.clone() },
            Self::Failed(message) => TurnState::Failed {
                message: message.clone(),
            },
        }
    }
}

/// Whether the agent process whose requests are closed still runs
/// (`Step::close_process`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Closing {
    /// It is gone or hung up: nothing more can be written to it.
    Gone,
    /// It runs on, but this session can no longer serve it (`fail`).
    LeftRunning,
}

impl Step<'_> {
    // --- holder events --------------------------------------------------

    pub(super) fn spawned(&mut self) {
        let Phase::Spawning { start } = &self.s.phase else {
            return;
        };
        let start = start.clone();
        let line = self.s.rpc.initialize(&self.s.version);
        self.s.phase = Phase::Initializing { start };
        self.out.stdin.extend(line);
        self.mark(Moment::BeforeRequest);
    }

    pub(super) fn exited(&mut self, code: Option<i32>, stderr: &[u8]) {
        let stderr = bound(clean(&String::from_utf8_lossy(stderr)));
        self.end(Phase::Exited, AgentState::Exited { code, stderr });
    }

    pub(super) fn lost(&mut self, message: &str) {
        self.end(
            Phase::Lost,
            AgentState::Lost {
                message: bound(clean(message)),
            },
        );
    }

    /// The agent's process ended (`phase` says how): its waiting permission
    /// requests are recorded cancelled, a waiting prompt is held, a running
    /// turn fails, and the agent takes `state`. After a refused start the
    /// failure stays the agent's state, and an exit's stderr is a notice.
    /// Nothing more happens once it has ended.
    fn end(&mut self, phase: Phase, state: AgentState) {
        if self.ended() {
            return;
        }
        let refused = self.s.phase == Phase::Refused;
        let lost = phase == Phase::Lost;
        self.s.phase = phase;
        self.clear_lists();
        // A login out when the agent ended is never answered, nor is a
        // list, whose client is told.
        let why = if lost { Refused::Lost } else { Refused::Exited };
        self.close_process(why.to_string(), Closing::Gone);
        if refused {
            // Decision 4: the version message stays the last agent state.
            match &state {
                AgentState::Exited { stderr, .. } if !stderr.trim().is_empty() => {
                    let text = format!("the agent's standard error:\n{stderr}");
                    self.notice(NoticeLevel::Warning, text);
                }
                AgentState::Exited { .. }
                | AgentState::Lost { .. }
                | AgentState::Starting
                | AgentState::Ready { .. }
                | AgentState::LoginRequired { .. }
                | AgentState::LoggingIn { .. }
                | AgentState::Failed { .. }
                | AgentState::Choosing { .. } => {}
            }
        } else {
            self.hold_waiting(HoldReason::AgentStopped);
            if self.running() {
                self.s.run = None;
                self.turn(TurnState::Failed {
                    message: "the agent exited".to_owned(),
                });
            }
            self.agent(state);
        }
        self.mark(Moment::Stopped);
    }

    /// `AGENT_RESTART` (decision 4): an ended agent whose conversation can
    /// be continued is spawned again into this log. The old agent's
    /// requests are dead with it, its stdout's line is gone (the new
    /// agent's holder session counts from 0), and the request ids go on.
    /// No checkpoint: the session has no holder until the spawn is placed,
    /// and [`Input::Spawned`] checkpoints before `initialize`.
    pub(super) fn restart(&mut self) -> Result<(), Refused> {
        match &self.s.phase {
            Phase::LoginRequired(login) => {
                let login = login.clone();
                return self.restart_for_login(login);
            }
            Phase::Restarting { start } => {
                let start = start.clone();
                self.respawn(start);
                return Ok(());
            }
            Phase::Spawning { .. }
            | Phase::Initializing { .. }
            | Phase::Creating { .. }
            | Phase::Resuming { .. }
            | Phase::Loading { .. }
            | Phase::Ready { .. }
            | Phase::Choosing { .. }
            | Phase::LoggingIn { .. }
            | Phase::Refused
            | Phase::Broken
            | Phase::Exited
            | Phase::Lost => {}
        }
        if !self.ended() {
            return Err(Refused::NotEnded);
        }
        if !self.s.continuable() {
            return Err(Refused::CannotContinue);
        }
        let Some(conversation) = self.s.transcript.conversation().cloned() else {
            return Err(Refused::CannotContinue);
        };
        self.respawn(Start::Continue {
            conversation,
            log: Log::Kept,
        });
        self.notice(NoticeLevel::Info, RESTARTED.to_owned());
        self.agent(AgentState::Starting);
        Ok(())
    }

    /// The agent is to be spawned again with `start`: the old agent's
    /// requests are dead with it, its stdout's line is gone (the new
    /// agent's holder session counts from 0), and the request ids go on.
    pub(super) fn respawn(&mut self, start: Start) {
        // Normally a no-op: `end` or `restart_for_login` closed the old
        // process. No path resets `Rpc` without closing it.
        self.close_process(Refused::Restarted.to_string(), Closing::Gone);
        self.s.lines = Lines::new();
        self.s.run = None;
        // The old agent's calls and terminals are gone with it; their
        // entries and text stay.
        self.s.book.clear();
        self.s.bad = None;
        self.s.phase = Phase::Spawning { start };
    }

    /// `AGENT_RESTART` while the agent needs a login (`agents-login`
    /// decision 8, after a terminal login): the running agent is hung up,
    /// and the start it is to make again is kept. A start's demand makes
    /// that start again (`session/new`, or the continue); a prompt's takes
    /// up its ACP session as a kept log's continue, refused when the agent
    /// offered no way to continue it. The log, the slot and the methods
    /// stay; the agent is starting from here on.
    fn restart_for_login(&mut self, login: Login) -> Result<(), Refused> {
        let start = match login.then {
            AfterLogin::Start(start) => start,
            AfterLogin::Ready { session } => {
                if !login.greeting.continuation.continuable() {
                    return Err(Refused::CannotContinue);
                }
                Start::Continue {
                    conversation: Conversation {
                        id: session,
                        cwd: self.s.cwd.clone(),
                        continuation: login.greeting.continuation,
                        // `ready` takes the title from the log.
                        title: None,
                    },
                    log: Log::Kept,
                }
            }
        };
        self.s.phase = Phase::Restarting { start };
        // The turn is already failed (`prompt_needs_login`), or there is
        // none (a start's demand): nothing to unblock.
        self.close_process(Refused::Restarted.to_string(), Closing::Gone);
        self.clear_lists();
        self.notice(NoticeLevel::Info, RESTARTED_AFTER_LOGIN.to_owned());
        self.agent(AgentState::Starting);
        self.out.hang_up = true;
        Ok(())
    }

    /// The start failed (decision 4): the agent fails with `message`, is
    /// sent nothing more and is hung up.
    pub(super) fn refuse(&mut self, message: String) {
        self.s.phase = Phase::Refused;
        self.close_process(message.clone(), Closing::Gone);
        // A permission asked during a muted load left the turn blocked.
        self.unblock();
        self.clear_lists();
        self.hold_waiting(HoldReason::AgentStopped);
        self.agent(AgentState::Failed {
            message: bound(message),
        });
        self.out.hang_up = true;
        self.mark(Moment::Stopped);
    }

    /// This session is done with its agent process (OpenSpec change
    /// `agents-process-lifecycle`, decision 1): a list out to it is
    /// answered `why`, every request awaited is forgotten, and each
    /// permission request waiting is recorded `cancelled`, and answered so
    /// too when the process runs on. The ids keep counting. The turn is
    /// each caller's to decide.
    fn close_process(&mut self, why: String, closing: Closing) {
        self.drop_list(why);
        for (entry, answer) in self.s.rpc.close() {
            self.emit(Event::Answered {
                id: entry,
                outcome: PermissionOutcome::Cancelled,
            });
            match closing {
                Closing::Gone => {}
                Closing::LeftRunning => self.out.stdin.extend(answer),
            }
        }
    }

    /// The core's handling failed (decision 13): the agent fails, a waiting
    /// prompt is held, and the agent is left running.
    pub(super) fn fail(&mut self, reason: &str) {
        self.fault = None;
        if self.ended() {
            return;
        }
        // The agent runs on, and a request of its could never get a choice
        // now: it is answered `cancelled` (`process R19`).
        self.close_process(
            "the terminal server's ACP handling failed".to_owned(),
            Closing::LeftRunning,
        );
        self.s.phase = Phase::Broken;
        self.clear_lists();
        self.s.run = None;
        self.hold_waiting(HoldReason::AgentStopped);
        if self.running() {
            self.turn(TurnState::Failed {
                message: "ACP handling failed".to_owned(),
            });
        }
        self.agent(AgentState::Failed {
            message: bound(format!("ACP handling failed: {}", clean(reason))),
        });
        self.mark(Moment::Stopped);
    }

    pub(super) fn initialized(&mut self, result: Result<&Value, &RpcError>) {
        let Phase::Initializing { start } = &self.s.phase else {
            return;
        };
        let start = start.clone();
        // `initialize` asked for the newest version; the answer names the
        // one the session speaks.
        let newest = Dialect::newest().version();
        match result {
            Ok(result) => match dialect::spoken(result).map(Dialect::from_version) {
                None => self.refuse(
                    "the agent's answer to initialize isn't one Brindle can read".to_owned(),
                ),
                Some(Err(Unsupported { version })) => self.refuse(format!(
                    "the agent speaks ACP version {version}; Brindle speaks {newest}"
                )),
                Some(Ok(dialect)) => self.speaking(dialect, start, result),
            },
            Err(error) => {
                let message = match Dialect::newest().supported_versions(error) {
                    Some(versions) => format!(
                        "the agent supports ACP version{} {}; Brindle speaks {newest}",
                        if versions.len() == 1 { "" } else { "s" },
                        versions
                            .iter()
                            .map(u16::to_string)
                            .collect::<Vec<_>>()
                            .join(", ")
                    ),
                    None if error.code == UNSUPPORTED_VERSION => {
                        format!("the agent supports no ACP version Brindle speaks ({newest})")
                    }
                    None => format!("the agent couldn't start: {}", clean(&error.message)),
                };
                self.refuse(message);
            }
        }
    }

    /// `initialize`'s answer, read in the version it names.
    fn speaking(&mut self, dialect: Dialect, start: Start, result: &Value) {
        self.s.rpc.speak(dialect);
        match dialect.initialize_result(result) {
            Ok(initialized) => {
                let (name, version) = initialized.agent.map_or((None, None), |agent| {
                    (Some(agent_name(&agent)), Some(bound(clean(&agent.version))))
                });
                let greeting = Greeting {
                    name,
                    version,
                    continuation: initialized.continuation,
                };
                self.s.lists = initialized.lists;
                self.keep_methods(dialect.auth_methods(result, true));
                self.start(start, greeting);
            }
            Err(_) => self
                .refuse("the agent's answer to initialize isn't one Brindle can read".to_owned()),
        }
    }

    /// After `initialize`: `session/new` for a new conversation, else
    /// `session/resume` or `session/load` by decision 2's table (a shown
    /// replay's notice first), or a refusal when neither is offered. No
    /// continue falls back to `session/new`.
    pub(super) fn start(&mut self, start: Start, greeting: Greeting) {
        let (conversation, log) = match start {
            Start::New => {
                let line = self.s.rpc.new_session(&self.s.cwd);
                self.s.phase = Phase::Creating { greeting };
                self.out.stdin.extend(line);
                self.mark(Moment::BeforeRequest);
                return;
            }
            Start::Continue { conversation, log } => (conversation, log),
            Start::Choose => return self.wait_for_choice(greeting, None),
        };
        let line = match choose(log, greeting.continuation) {
            Choice::Resume => {
                let line = self.s.rpc.resume(&conversation.id, &conversation.cwd);
                self.s.phase = Phase::Resuming {
                    greeting,
                    conversation: conversation.id,
                    log,
                };
                line
            }
            Choice::Load { replay } => {
                if replay == Replay::Shown {
                    self.notice(NoticeLevel::Info, REPLAY_SHOWN.to_owned());
                }
                let line = self.s.rpc.load(&conversation.id, &conversation.cwd);
                self.s.phase = Phase::Loading {
                    greeting,
                    conversation: conversation.id,
                    replay,
                };
                line
            }
            Choice::Neither if self.s.chose => {
                return self.wait_for_choice(greeting, Some(CANT_CONTINUE.to_owned()));
            }
            Choice::Neither => return self.refuse(CANT_CONTINUE.to_owned()),
        };
        self.out.stdin.extend(line);
        self.mark(Moment::BeforeRequest);
    }

    /// A continue the agent refused: a chosen one returns to choosing with
    /// the reason (`agents-session-list` decision 11, flag 5), any other
    /// fails the agent.
    fn continue_refused(&mut self, greeting: Greeting, message: String) {
        if self.s.chose {
            self.wait_for_choice(greeting, Some(message));
        } else {
            self.refuse(message);
        }
    }

    /// The agent waits for a choice (`agents-session-list` decision 1):
    /// after `initialize`, after a login that interrupted a list, or after
    /// the agent refused the conversation chosen, with its `message`.
    fn wait_for_choice(&mut self, greeting: Greeting, message: Option<String>) {
        self.s.chose = false;
        self.s.phase = Phase::Choosing { greeting };
        self.agent(AgentState::Choosing {
            message: message.map(bound),
        });
        self.mark(Moment::Started);
    }

    /// `AGENT_CHOOSE` (decision 1): in [`Phase::Choosing`] only, a
    /// conversation continues as a fresh log's does, by what the agent
    /// offers, or a new one starts.
    pub(super) fn choose_conversation(&mut self, conversation: Option<String>) -> Result<(), Refused> {
        let Phase::Choosing { greeting } = &self.s.phase else {
            return Err(Refused::NotChoosing);
        };
        let greeting = greeting.clone();
        let start = match conversation {
            Some(id) if id.len() > MAX_LISTED_ID => {
                return Err(Refused::TooLongToList {
                    what: "conversation id",
                    len: id.len(),
                });
            }
            Some(id) => {
                self.s.chose = true;
                Start::Continue {
                    conversation: Conversation {
                        id,
                        cwd: self.s.cwd.clone(),
                        continuation: greeting.continuation,
                        title: None,
                    },
                    log: Log::Fresh,
                }
            }
            None => Start::New,
        };
        self.agent(AgentState::Starting);
        self.start(start, greeting);
        Ok(())
    }

    pub(super) fn created(&mut self, result: Result<&Value, &RpcError>) {
        let Phase::Creating { greeting } = &self.s.phase else {
            return;
        };
        let greeting = greeting.clone();
        match result {
            Ok(result) => match self.s.rpc.dialect().new_session_result(result) {
                // Every later request sends the id back, and the
                // conversation names it: one past `MAX_SESSION_ID` can't be
                // cut, so the start fails (`agents-event-bounds` decision 3).
                Ok(created) if created.session_id.is_empty()
                    || created.session_id.len() > MAX_SESSION_ID
                    || created.session_id.chars().any(char::is_control) =>
                {
                    self.refuse(
                        "the agent's session id is longer than 1 KiB, or isn't one Brindle can send back"
                            .to_owned(),
                    );
                }
                Ok(created) => self.ready(created.session_id, greeting, None, created.offered),
                Err(_) => self.refuse(
                    "the agent's answer to session/new isn't one Brindle can read".to_owned(),
                ),
            },
            Err(error) if error.code == AUTH_REQUIRED => {
                self.login_required(error, greeting, AfterLogin::Start(Start::New));
            }
            Err(error) => self.refuse(format!(
                "the agent couldn't start a session: {}",
                clean(&error.message)
            )),
        }
    }

    /// `session/resume`'s or `session/load`'s answer (decision 4 of
    /// `agents-session-recovery`): the conversation continues in the same
    /// ACP session, with the notice its way calls for; `-32000` needs a
    /// login; any other error, or an answer Brindle can't read, fails it.
    pub(super) fn continued(&mut self, result: Result<&Value, &RpcError>) {
        let (greeting, conversation, log, notice, how) = match &self.s.phase {
            Phase::Resuming {
                greeting,
                conversation,
                log,
            } => {
                let notice = match log {
                    Log::Kept => None,
                    Log::Fresh => Some((NoticeLevel::Warning, NOT_REPLAYED.to_owned())),
                };
                (greeting, conversation, *log, notice, Continued::Resume)
            }
            Phase::Loading {
                greeting,
                conversation,
                replay,
            } => {
                let (notice, log) = match replay {
                    Replay::Muted { updates } => {
                        (Some((NoticeLevel::Info, muted_notice(*updates))), Log::Kept)
                    }
                    Replay::Shown => (None, Log::Fresh),
                };
                (greeting, conversation, log, notice, Continued::Load)
            }
            Phase::Spawning { .. }
            | Phase::Initializing { .. }
            | Phase::Creating { .. }
            | Phase::Ready { .. }
            | Phase::Choosing { .. }
            | Phase::LoginRequired(_)
            | Phase::LoggingIn { .. }
            | Phase::Restarting { .. }
            | Phase::Refused
            | Phase::Broken
            | Phase::Exited
            | Phase::Lost => return,
        };
        let (greeting, conversation) = (greeting.clone(), conversation.clone());
        let offered = match result {
            Ok(result) => self.s.rpc.dialect().continued_result(how, result).ok(),
            // After a login the same continue is sent again (the session's
            // directory is the conversation's).
            Err(error) if error.code == AUTH_REQUIRED => {
                let start = Start::Continue {
                    conversation: Conversation {
                        id: conversation,
                        cwd: self.s.cwd.clone(),
                        continuation: greeting.continuation,
                        // `ready` takes the title from the log.
                        title: None,
                    },
                    log,
                };
                return self.login_required(error, greeting, AfterLogin::Start(start));
            }
            Err(error) => {
                let message = format!(
                    "the agent couldn't continue the conversation: {}",
                    clean(&error.message)
                );
                return self.continue_refused(greeting, message);
            }
        };
        let Some(offered) = offered else {
            let method = self.s.rpc.dialect().continue_method(how);
            let message = format!(
                "the agent couldn't continue the conversation: its answer to {method} isn't one Brindle can read"
            );
            return self.continue_refused(greeting, message);
        };
        self.s.chose = false;
        // The replay's last chunk is no run a live chunk extends.
        self.s.run = None;
        self.ready(conversation, greeting, notice, offered);
    }

    /// The agent takes prompts in ACP session `session`: the conversation,
    /// then the options its start `offered` (`agents-commands`, decision
    /// 12) with any mode notice, then `notice` when there is one, then the
    /// ready state.
    fn ready(
        &mut self,
        session: String,
        greeting: Greeting,
        notice: Option<(NoticeLevel, String)>,
        offered: Offered,
    ) {
        // The same ACP session keeps its title (a login's restart, a
        // continue by resume or load); a new one has none yet.
        let title = self
            .s
            .transcript
            .conversation()
            .filter(|conversation| conversation.id == session)
            .and_then(|conversation| conversation.title.clone());
        self.emit(Event::Conversation {
            conversation: Conversation {
                id: session.clone(),
                cwd: self.s.cwd.clone(),
                continuation: greeting.continuation,
                title,
            },
        });
        // The start's options are the ones that stand, whole; a mode is
        // compared with the one last in effect.
        self.offer_options(offered, Compare::Kept);
        if let Some((level, text)) = notice {
            self.notice(level, text);
        }
        self.s.phase = Phase::Ready { session };
        self.agent(AgentState::Ready {
            name: greeting.name,
            version: greeting.version,
        });
        self.mark(Moment::Started);
    }

    /// What `initialize` said, as the log has it once the agent is ready:
    /// its name and version, and what its conversation offers.
    pub(super) fn greeting_now(&self) -> Greeting {
        let (name, version) = match self.s.transcript.agent() {
            AgentState::Ready { name, version } => (name.clone(), version.clone()),
            AgentState::Starting
            | AgentState::LoginRequired { .. }
            | AgentState::LoggingIn { .. }
            | AgentState::Failed { .. }
            | AgentState::Exited { .. }
            | AgentState::Lost { .. }
            | AgentState::Choosing { .. } => (None, None),
        };
        let continuation = self
            .s
            .transcript
            .conversation()
            .map_or(Continuation::Neither, |conversation| {
                conversation.continuation
            });
        Greeting {
            name,
            version,
            continuation,
        }
    }
}

#[cfg(test)]
mod tests {
    use brindle_remote_protocol::serde_json::{self, json};
    use brindle_transcript::Activity;

    use super::super::harness::*;
    use super::super::*;
    use super::*;

    // --- the start sequence (decision 4) --------------------------------

    #[test]
    fn spawning_sends_initialize_for_version_1_advertising_auth_terminal_and_boolean_options() {
        let mut agent = Agent::new();
        let out = agent.step(Input::Spawned);
        assert_eq!(
            out.stdin,
            b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":1,\"clientCapabilities\":{\"auth\":{\"terminal\":true},\"session\":{\"configOptions\":{\"boolean\":{}}}},\"clientInfo\":{\"name\":\"brindle\",\"version\":\"0.3.0\"}}}\n"
        );
        assert_eq!(out.checkpoint, Some(Moment::BeforeRequest));
        assert_eq!(*agent.s.phase(), Phase::Initializing { start: Start::New });
        assert!(agent.step(Input::Spawned).stdin.is_empty(), "spawned twice");
    }

    #[test]
    fn an_answered_initialize_sends_session_new_in_the_absolute_directory() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        let out = agent.feed(&result(1, json!({"protocolVersion": 1})));
        assert_eq!(
            out.stdin,
            b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"session/new\",\"params\":{\"cwd\":\"/home/u/src\",\"mcpServers\":[]}}\n"
        );
        assert_eq!(out.checkpoint, Some(Moment::BeforeRequest));
        assert_eq!(agent.s.cwd(), "/home/u/src");
    }

    #[test]
    fn an_answered_session_new_makes_the_agent_ready_with_the_name_initialize_gave() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        agent.feed(&result(1, json!({"protocolVersion": 1, "agentInfo": {"name": "fake", "title": "Fake", "version": "1.0"}})));
        let out = agent.feed(&result(2, json!({"sessionId": "s1"})));
        assert_eq!(
            *agent.agent_state(),
            AgentState::Ready {
                // The readable title (`agents-session-title` decision 4).
                name: Some("Fake".into()),
                version: Some("1.0".into())
            }
        );
        assert_eq!(out.checkpoint, Some(Moment::Started));
    }

    #[test]
    fn a_32022_with_its_supported_versions_fails_the_agent_naming_them() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        let out = agent.feed(&fixture_line("initialize-32022", 0));
        assert_eq!(
            *agent.agent_state(),
            AgentState::Failed {
                message: "the agent supports ACP version 2; Brindle speaks 1".into()
            }
        );
        assert!(out.hang_up);
        assert!(out.stdin.is_empty(), "no session/new");
        assert_eq!(*agent.s.phase(), Phase::Refused);
    }

    #[test]
    fn several_supported_versions_are_all_named() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        agent.feed(&json!({"jsonrpc": "2.0", "id": 1, "error": {"code": -32022, "message": "no", "data": {"requested": 1, "supported": [2, 3]}}}).to_string());
        assert_eq!(
            *agent.agent_state(),
            AgentState::Failed {
                message: "the agent supports ACP versions 2, 3; Brindle speaks 1".into()
            }
        );
    }

    #[test]
    fn a_32022_without_well_formed_data_fails_naming_brindles_version() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        let out = agent.feed(&fixture_line("initialize-32022", 1));
        assert_eq!(
            *agent.agent_state(),
            AgentState::Failed {
                message: "the agent supports no ACP version Brindle speaks (1)".into()
            }
        );
        assert!(out.hang_up);
    }

    #[test]
    fn a_success_with_another_protocol_version_fails_the_agent_naming_it() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        let out = agent.feed(&fixture_line("initialize-v2-reply", 0));
        assert_eq!(
            *agent.agent_state(),
            AgentState::Failed {
                message: "the agent speaks ACP version 2; Brindle speaks 1".into()
            }
        );
        assert!(out.hang_up);
        assert!(!agent.sent_methods().contains(&"session/new"));
    }

    #[test]
    fn another_initialize_error_fails_the_agent_with_its_message() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        let out = agent.feed(&error(1, -32603, "internal error"));
        assert_eq!(
            *agent.agent_state(),
            AgentState::Failed {
                message: "the agent couldn't start: internal error".into()
            }
        );
        assert!(out.hang_up);
    }

    #[test]
    fn an_unreadable_initialize_answer_fails_the_agent() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        let out = agent.feed(&result(1, json!({"nothing": true})));
        assert!(matches!(agent.agent_state(), AgentState::Failed { .. }));
        assert!(out.hang_up);
    }

    #[test]
    fn a_refused_agent_is_sent_nothing_more() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        agent.feed(&fixture_line("initialize-v2-reply", 0));
        let out =
            agent.feed(r#"{"jsonrpc":"2.0","id":9,"method":"fs/read_text_file","params":{}}"#);
        assert!(out.stdin.is_empty());
        assert!(agent.feed("not json").events.is_empty());
    }

    #[test]
    fn a_refused_agents_exit_leaves_the_failed_state_and_adds_its_stderr_as_a_notice() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        agent.feed(&fixture_line("initialize-32022", 0));
        let out = agent.step(Input::Exited {
            code: Some(1),
            stderr: b"only v2 here\n",
        });
        assert_eq!(shape(&out.events), ["added notice"]);
        assert_eq!(
            agent.notices(),
            ["the agent's standard error:\nonly v2 here\n"]
        );
        assert_eq!(
            *agent.agent_state(),
            AgentState::Failed {
                message: "the agent supports ACP version 2; Brindle speaks 1".into()
            }
        );
        let last_agent = agent.events.iter().rev().find_map(|event| match event {
            Event::Agent { state } => Some(state.clone()),
            Event::Added { .. }
            | Event::Appended { .. }
            | Event::ToolChanged { .. }
            | Event::ToolItems { .. }
            | Event::ToolOutput { .. }
            | Event::Answered { .. }
            | Event::Queue { .. }
            | Event::Turn { .. }
            | Event::Conversation { .. }
            | Event::Trimmed { .. }
            | Event::Commands { .. }
            | Event::Options { .. }
            | Event::Window { .. } => None,
        });
        assert!(matches!(last_agent, Some(AgentState::Failed { .. })));
    }

    #[test]
    fn a_refused_agents_exit_with_empty_stderr_adds_nothing() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        agent.feed(&fixture_line("initialize-32022", 0));
        let out = agent.step(Input::Exited {
            code: Some(1),
            stderr: b"",
        });
        assert!(out.events.is_empty());
    }

    #[test]
    fn auth_required_from_session_new_needs_a_login_and_keeps_the_agent_running() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        agent.feed(&result(1, json!({"protocolVersion": 1})));
        let out = agent.feed(&error(2, -32000, "run agent login"));
        assert_eq!(
            *agent.agent_state(),
            AgentState::LoginRequired {
                message: "run agent login".into(),
                methods: Vec::new(),
            }
        );
        assert!(!out.hang_up);
        assert_eq!(agent.activity(), Activity::Error);
        assert_eq!(agent.prompt("hi").reply, Some(Err(Refused::LoginRequired)));
        assert!(!agent.sent_methods().contains(&"authenticate"));
    }

    #[test]
    fn another_session_new_error_fails_the_start() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        agent.feed(&result(1, json!({"protocolVersion": 1})));
        let out = agent.feed(&error(2, -32603, "no such directory"));
        assert_eq!(
            *agent.agent_state(),
            AgentState::Failed {
                message: "the agent couldn't start a session: no such directory".into()
            }
        );
        assert!(out.hang_up);
    }

    #[test]
    fn a_prompt_before_ready_is_refused_and_nothing_is_queued() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        let out = agent.prompt("hi");
        assert_eq!(out.reply, Some(Err(Refused::NotReady)));
        assert_eq!(
            out.reply
                .as_ref()
                .map(|r| r.as_ref().map_err(ToString::to_string)),
            Some(Err("the agent isn't ready".into()))
        );
        assert!(out.events.is_empty());
        assert_eq!(*agent.slot(), Slot::Empty);
    }

    // --- exits and faults -----------------------------------------------

    #[test]
    fn an_exit_reports_its_status_and_the_end_of_its_stderr() {
        let mut agent = Agent::ready();
        let out = agent.step(Input::Exited {
            code: Some(1),
            stderr: b"loading\nfatal: no model\n",
        });
        assert_eq!(
            *agent.agent_state(),
            AgentState::Exited {
                code: Some(1),
                stderr: "loading\nfatal: no model\n".into()
            }
        );
        assert_eq!(out.checkpoint, Some(Moment::Stopped));
        assert!(
            agent
                .step(Input::Exited {
                    code: None,
                    stderr: b""
                })
                .events
                .is_empty()
        );
    }

    #[test]
    fn an_exit_mid_turn_fails_the_turn_and_cancels_the_waiting_permissions() {
        let mut agent = Agent::ready();
        agent.turn("one");
        let entry = agent.ask(json!(0), &ALLOW_REJECT);
        let out = agent.step(Input::Exited {
            code: None,
            stderr: b"\x1b[31mkilled\xff",
        });
        assert!(out.stdin.is_empty(), "nothing is written to a gone agent");
        assert_eq!(
            shape(&out.events),
            ["answered", "turn failed", "agent exited"]
        );
        assert!(matches!(
            agent.s.transcript.entry(entry),
            Some(Entry::Permission {
                outcome: Some(PermissionOutcome::Cancelled),
                ..
            })
        ));
        assert_eq!(
            *agent.agent_state(),
            AgentState::Exited {
                code: None,
                stderr: "\u{fffd}[31mkilled\u{fffd}".into()
            }
        );
    }

    #[test]
    fn a_handling_fault_fails_the_agent_and_leaves_it_running_with_its_lines_ignored() {
        let mut agent = Agent::ready();
        let out = agent.step(Input::Fault {
            reason: "panicked at reduce.rs".into(),
        });
        assert!(!out.hang_up);
        assert_eq!(
            *agent.agent_state(),
            AgentState::Failed {
                message: "ACP handling failed: panicked at reduce.rs".into()
            }
        );
        assert!(agent.feed(&said("x")).events.is_empty());
        assert!(
            agent
                .feed(r#"{"jsonrpc":"2.0","id":4,"method":"fs/x"}"#)
                .stdin
                .is_empty()
        );
        let before = agent.s.lines().covers();
        agent.feed("more");
        assert_eq!(
            agent.s.lines().covers(),
            before + 5,
            "the bytes are still counted"
        );
    }

    #[test]
    fn an_exit_after_a_handling_fault_still_reports_the_exit() {
        let mut agent = Agent::ready();
        agent.step(Input::Fault {
            reason: "boom".into(),
        });
        agent.step(Input::Exited {
            code: Some(0),
            stderr: b"",
        });
        assert!(matches!(agent.agent_state(), AgentState::Exited { .. }));
    }

    #[test]
    fn a_fold_error_from_the_reducers_own_events_fails_the_handling() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.feed(&tool_call("call_1", "Write", "pending"));
        // A run pointing at the tool entry: the next chunk appends to it.
        agent.s.run = Some(Run {
            kind: RunKind::Agent,
            entry: EntryId(2),
        });
        let out = agent.feed(&said("x"));
        assert_eq!(shape(&out.events), ["turn failed", "agent failed"]);
        assert!(
            matches!(agent.agent_state(), AgentState::Failed { message } if message.contains("has no text to append to"))
        );
    }


    #[test]
    fn the_start_choice_follows_decision_2s_table() {
        let muted = Choice::Load {
            replay: Replay::Muted { updates: 0 },
        };
        let shown = Choice::Load {
            replay: Replay::Shown,
        };
        for (log, offered, choice) in [
            (Log::Kept, Continuation::ResumeAndLoad, Choice::Resume),
            (Log::Kept, Continuation::Resume, Choice::Resume),
            (Log::Kept, Continuation::Load, muted),
            (Log::Kept, Continuation::Neither, Choice::Neither),
            (Log::Fresh, Continuation::ResumeAndLoad, shown),
            (Log::Fresh, Continuation::Load, shown),
            (Log::Fresh, Continuation::Resume, Choice::Resume),
            (Log::Fresh, Continuation::Neither, Choice::Neither),
        ] {
            assert_eq!(choose(log, offered), choice, "{log:?} {offered:?}");
        }
    }

    // 2.1: the conversation is named.

    #[test]
    fn a_new_session_names_its_conversation_before_the_ready_state() {
        let mut agent = Agent::in_dir("/home/u/src");
        agent.step(Input::Spawned);
        agent.feed(&initialized(1, both()));
        let out = agent.feed(&result(2, json!({"sessionId": "s1"})));
        assert_eq!(shape(&out.events), ["conversation", "agent ready"]);
        assert_eq!(
            out.events[0],
            Event::Conversation {
                conversation: conversation_s1(Continuation::ResumeAndLoad)
            }
        );
        assert_eq!(out.checkpoint, Some(Moment::Started));
        // What the agent offered is what the conversation says.
        for (capabilities, continuation) in [
            (load_only(), Continuation::Load),
            (resume_only(), Continuation::Resume),
            (json!({}), Continuation::Neither),
        ] {
            let agent = Agent::ready_offering(capabilities);
            assert_eq!(
                agent.s.transcript().conversation(),
                Some(&conversation_s1(continuation))
            );
        }
    }

    #[test]
    fn an_exit_leaves_the_conversation_in_the_log_and_its_replay() {
        let agent = Agent::ended();
        assert!(matches!(agent.agent_state(), AgentState::Exited { .. }));
        let conversation = conversation_s1(Continuation::ResumeAndLoad);
        assert_eq!(agent.s.transcript().conversation(), Some(&conversation));
        assert!(agent.s.transcript().replay().contains(&Event::Conversation { conversation }));
        assert!(agent.s.continuable());
    }

    #[test]
    fn only_an_ended_agent_with_a_conversation_it_offered_to_continue_is_continuable() {
        assert!(!Agent::new().s.continuable(), "no conversation yet");
        assert!(!Agent::ready_offering(both()).s.continuable(), "still running");
        let mut neither = Agent::ready_offering(json!({}));
        neither.exit();
        assert!(!neither.s.continuable(), "neither offered");
        let mut never = Agent::new();
        never.step(Input::Spawned);
        never.feed(&initialized(1, both()));
        never.exit();
        assert!(!never.s.continuable(), "no ACP session was given");
        let mut lost = Agent::ready_offering(load_only());
        lost.step(Input::Lost {
            message: "its holder ended",
        });
        assert!(lost.s.continuable());
    }

    // 2.2: the start choice after the new agent's `initialize`.

    #[test]
    fn a_kept_log_resumes_when_the_new_agent_offers_resume() {
        let mut agent = Agent::ended();
        let before = agent.sent.len();
        let id = agent.restart();
        let out = agent.feed(&initialized(id, both()));
        assert_eq!(
            agent.sent_since(before).last(),
            Some(&json!({"jsonrpc": "2.0", "id": id + 1, "method": "session/resume",
                "params": {"sessionId": "s1", "cwd": "/home/u/src", "mcpServers": []}}))
        );
        assert_eq!(out.checkpoint, Some(Moment::BeforeRequest));
        assert!(matches!(agent.s.phase(), Phase::Resuming { log: Log::Kept, .. }));
        let out = agent.feed(&result(id + 1, json!({})));
        assert_eq!(shape(&out.events), ["conversation", "agent ready"]);
        assert_eq!(out.checkpoint, Some(Moment::Started));
        assert_eq!(
            *agent.agent_state(),
            AgentState::Ready {
                name: Some("fake".into()),
                version: Some("2.0".into())
            }
        );
        assert_eq!(*agent.s.phase(), Phase::Ready { session: "s1".into() });
        let methods: Vec<&str> = agent
            .sent_since(before)
            .iter()
            .filter_map(|line| line["method"].as_str())
            .collect();
        assert_eq!(methods, ["initialize", "session/resume"], "no session/new");
    }

    #[test]
    fn a_kept_log_loads_muted_when_the_new_agent_offers_only_load() {
        let mut agent = Agent::ended();
        let id = agent.restart();
        agent.feed(&initialized(id, load_only()));
        assert_eq!(
            *agent.last("session/load"),
            json!({"jsonrpc": "2.0", "id": id + 1, "method": "session/load",
                "params": {"sessionId": "s1", "cwd": "/home/u/src", "mcpServers": []}})
        );
        assert!(matches!(
            agent.s.phase(),
            Phase::Loading {
                replay: Replay::Muted { updates: 0 },
                ..
            }
        ));
        assert!(!agent.sent_methods().contains(&"session/resume"));
    }

    #[test]
    fn a_fresh_log_loads_and_shows_the_replay_after_its_notice() {
        let mut agent = Agent::in_dir("/ignored");
        agent.s = AgentSession::continuing(
            conversation_s1(Continuation::Load),
            "0.3.0".into(),
        );
        assert_eq!(agent.s.cwd(), "/home/u/src", "the conversation's directory");
        agent.step(Input::Spawned);
        let out = agent.feed(&initialized(1, both()));
        assert_eq!(shape(&out.events), ["added notice"]);
        assert_eq!(agent.notices(), [REPLAY_SHOWN]);
        assert_eq!(agent.last("session/load")["params"]["sessionId"], "s1");
        assert!(!agent.sent_methods().contains(&"session/new"));
    }

    #[test]
    fn a_fresh_log_resumes_with_a_warning_when_the_agent_offers_only_resume() {
        let mut agent = Agent::new();
        agent.s = AgentSession::continuing(conversation_s1(Continuation::Resume), "0.3.0".into());
        agent.step(Input::Spawned);
        agent.feed(&initialized(1, resume_only()));
        assert_eq!(agent.last("session/resume")["params"]["sessionId"], "s1");
        let out = agent.feed(&result(2, json!({})));
        assert_eq!(
            shape(&out.events),
            ["conversation", "added notice", "agent ready"]
        );
        assert!(matches!(
            agent.last_entry().1,
            Entry::Notice { level: NoticeLevel::Warning, text } if text == NOT_REPLAYED
        ));
        assert_eq!(
            agent.s.transcript().conversation(),
            Some(&conversation_s1(Continuation::Resume))
        );
    }

    #[test]
    fn a_new_agent_offering_neither_is_refused_and_sent_nothing_more() {
        let mut agent = Agent::ended();
        let before = agent.sent.len();
        let id = agent.restart();
        let out = agent.feed(&initialized(id, json!({})));
        assert!(out.hang_up);
        assert_eq!(
            *agent.agent_state(),
            AgentState::Failed {
                message: CANT_CONTINUE.into()
            }
        );
        assert_eq!(agent.sent_since(before).len(), 1, "only initialize");
        agent.exit();
        assert!(matches!(agent.agent_state(), AgentState::Failed { .. }));
        assert!(!agent.s.continuable(), "the session ends as after a mismatch");
        assert_eq!(
            agent.step(Input::Restart).reply,
            Some(Err(Refused::CannotContinue))
        );
    }

    // 2.3: the replay, muted or shown.

    #[test]
    fn a_muted_replay_of_7_updates_leaves_the_log_as_it_was_plus_one_notice() {
        let mut agent = Agent::ended();
        let id = agent.restart();
        let kept = agent.entries();
        let next = agent.s.transcript().next_id();
        agent.feed(&initialized(id, load_only()));
        for line in [
            chunk("user_message_chunk", "first"),
            said("Done."),
            chunk("agent_thought_chunk", "hmm"),
            tool_call("call_9", "Edit", "completed"),
            tool_update("call_9", "completed"),
            update(json!({"sessionUpdate": "plan", "entries": []})),
            // One Brindle can't read still counts.
            update(json!({"sessionUpdate": "agent_message_chunk"})),
        ] {
            assert!(agent.feed(&line).events.is_empty(), "{line}");
        }
        // Another conversation's update isn't counted.
        agent.feed(
            &json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s2",
                "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "x"}}}})
            .to_string(),
        );
        assert_eq!(agent.entries(), kept);
        let out = agent.feed(&result(id + 1, json!(null)));
        assert_eq!(
            shape(&out.events),
            ["conversation", "added notice", "agent ready"]
        );
        let mut expected = kept;
        expected.push(Entry::Notice {
            level: NoticeLevel::Info,
            text: "conversation continued; the agent's replay of 7 updates wasn't shown".into(),
        });
        assert_eq!(agent.entries(), expected);
        assert_eq!(agent.last_entry().0, next);
        assert_eq!(*agent.turn_state(), ended(StopReason::EndTurn), "no turn");
        // A live chunk afterwards is a new entry, not the old run's.
        agent.feed(&said("Back."));
        assert!(matches!(agent.last_entry().1, Entry::Agent { text } if text == "Back."));
    }

    #[test]
    fn a_muted_load_applies_no_tool_content() {
        let mut agent = Agent::ended();
        let id = agent.restart();
        let kept = agent.entries();
        agent.feed(&initialized(id, load_only()));
        for line in [
            update(
                json!({"sessionUpdate": "tool_call", "toolCallId": "call_9", "title": "Edit",
                "content": [{"type": "diff", "path": "/a", "oldText": "1\n", "newText": "2\n"}],
                "rawInput": "x", "locations": [{"path": "/a"}]}),
            ),
            meta(
                "call_9",
                json!({"terminal_output_delta": {"terminal_id": "t9", "data": "out\n"}}),
            ),
        ] {
            assert!(agent.feed(&line).events.is_empty(), "{line}");
        }
        assert_eq!(agent.entries(), kept);
        assert!(agent.s.book.outputs.is_empty());
    }

    // The title across restarts (agents-session-title decision 3).

    /// [`Agent::ended`], its conversation titled `Old` before the exit.
    fn ended_titled() -> Agent {
        let mut agent = Agent::ready_offering(both());
        let id = agent.turn("first");
        agent.feed(&titled(json!("Old")));
        agent.feed(&said("Done."));
        agent.end(id, "end_turn");
        agent.exit();
        assert_eq!(title_of(&agent), Some("Old"));
        agent
    }

    #[test]
    fn a_resume_of_the_same_session_keeps_its_title() {
        let mut agent = ended_titled();
        let id = agent.restart();
        agent.feed(&initialized(id, both()));
        let out = agent.feed(&result(id + 1, json!({})));
        assert_eq!(shape(&out.events), ["conversation", "agent ready"]);
        assert!(matches!(
            &out.events[0],
            Event::Conversation { conversation } if conversation.title.as_deref() == Some("Old")
        ));
        assert_eq!(title_of(&agent), Some("Old"));
    }

    #[test]
    fn a_muted_load_applies_a_replayed_title_without_counting_it() {
        let mut agent = ended_titled();
        let id = agent.restart();
        agent.feed(&initialized(id, load_only()));
        assert!(agent.feed(&said("Done.")).events.is_empty());
        let out = agent.feed(&titled(json!("New")));
        assert_eq!(shape(&out.events), ["conversation"]);
        let out = agent.feed(&result(id + 1, json!(null)));
        assert_eq!(
            shape(&out.events),
            ["conversation", "added notice", "agent ready"]
        );
        assert_eq!(
            agent.notices().last().map(String::as_str),
            Some("conversation continued; the agent's replay of 1 update wasn't shown")
        );
        assert_eq!(title_of(&agent), Some("New"));
    }

    #[test]
    fn a_login_restart_of_the_same_session_keeps_its_title() {
        let mut agent = Agent::ready_with_methods();
        let id = agent.turn("one");
        agent.feed(&titled(json!("Hello.txt file creation")));
        agent.feed(&error(id, AUTH_REQUIRED, "expired"));
        assert_eq!(agent.step(Input::Restart).reply, Some(Ok(())));
        let id = agent.restart();
        agent.feed(&initialized(id, resume_only()));
        let resume = agent.last("session/resume")["id"].as_u64().expect("an id");
        agent.feed(&result(resume, json!({})));
        assert!(matches!(agent.s.phase(), Phase::Ready { .. }));
        assert_eq!(title_of(&agent), Some("Hello.txt file creation"));
    }

    #[test]
    fn a_login_by_a_prompts_demand_keeps_the_title() {
        let mut agent = Agent::ready_with_methods();
        let id = agent.turn("one");
        agent.feed(&titled(json!("Named")));
        agent.feed(&error(id, AUTH_REQUIRED, "expired"));
        agent.login("oauth-personal");
        agent.feed(&result(agent.authenticate_id(), json!({})));
        assert!(matches!(agent.s.phase(), Phase::Ready { .. }));
        assert_eq!(title_of(&agent), Some("Named"));
    }

    #[test]
    fn a_new_server_session_starts_untitled_until_the_agent_names_it() {
        let mut agent = Agent::new();
        agent.s = AgentSession::continuing(conversation_s1(Continuation::Load), "0.3.0".into());
        agent.step(Input::Spawned);
        agent.feed(&initialized(1, both()));
        // The shown replay's title names no conversation the log has yet.
        assert_eq!(agent.feed(&titled(json!("Before"))), Output::default());
        agent.feed(&result(2, json!(null)));
        assert!(matches!(agent.s.phase(), Phase::Ready { .. }));
        assert_eq!(title_of(&agent), None);
        agent.feed(&titled(json!("After")));
        assert_eq!(title_of(&agent), Some("After"));
    }

    #[test]
    fn a_muted_count_of_one_update_is_singular() {
        assert_eq!(
            muted_notice(1),
            "conversation continued; the agent's replay of 1 update wasn't shown"
        );
    }

    #[test]
    fn a_shown_replay_of_a_user_and_an_agent_message_gives_two_entries() {
        let mut agent = Agent::new();
        agent.s = AgentSession::continuing(conversation_s1(Continuation::Load), "0.3.0".into());
        agent.step(Input::Spawned);
        agent.feed(&initialized(1, both()));
        agent.feed(&chunk("user_message_chunk", "hi"));
        agent.feed(&said("hel"));
        agent.feed(&said("lo"));
        let out = agent.feed(&result(2, json!({})));
        assert_eq!(shape(&out.events), ["conversation", "agent ready"]);
        assert_eq!(
            agent.entries(),
            [
                Entry::Notice {
                    level: NoticeLevel::Info,
                    text: REPLAY_SHOWN.into()
                },
                Entry::User { text: "hi".into() },
                Entry::Agent {
                    text: "hello".into()
                },
            ]
        );
        assert_eq!(*agent.turn_state(), TurnState::Idle);
        assert_eq!(agent.activity(), Activity::Idle);
        agent.feed(&said("again"));
        assert_eq!(agent.entries().len(), 4, "the replay's run ended");
    }

    #[test]
    fn a_permission_request_during_a_muted_load_is_logged_and_answerable() {
        let mut agent = Agent::ended();
        let id = agent.restart();
        agent.feed(&initialized(id, load_only()));
        let entry = agent.ask(json!(0), &ALLOW_REJECT);
        assert!(matches!(
            agent.last_entry().1,
            Entry::Permission { outcome: None, .. }
        ));
        agent.feed(&result(id + 1, json!({})));
        assert_eq!(
            agent
                .cmd(Command::Answer {
                    entry,
                    option: Some("allow-once".into())
                })
                .reply,
            Some(Ok(()))
        );
        assert_eq!(agent.sent.last().map(|line| &line["id"]), Some(&json!(0)));
    }

    // 2.4: the answers to session/resume and session/load.

    #[test]
    fn a_continue_answered_32000_needs_a_login_and_keeps_the_agent() {
        for (capabilities, method, continuation) in [
            (both(), "session/resume", Continuation::ResumeAndLoad),
            (load_only(), "session/load", Continuation::Load),
        ] {
            let mut agent = Agent::ended();
            let id = agent.restart();
            agent.feed(&initialized(id, capabilities));
            let out = agent.feed(&error(id + 1, AUTH_REQUIRED, "log in first"));
            assert!(!out.hang_up);
            assert_eq!(
                *agent.agent_state(),
                AgentState::LoginRequired {
                    message: "log in first".into(),
                    methods: Vec::new(),
                }
            );
            // After a login, the same continue of the same conversation
            // into the kept log.
            let Phase::LoginRequired(login) = agent.s.phase() else {
                panic!("{:?}", agent.s.phase());
            };
            assert_eq!(
                login.then,
                AfterLogin::Start(Start::Continue {
                    conversation: conversation_s1(continuation),
                    log: Log::Kept,
                })
            );
            assert_eq!(agent.last(method)["params"]["sessionId"], json!("s1"));
        }
    }

    #[test]
    fn after_a_login_a_continue_is_sent_again_and_continues() {
        let mut agent = Agent::ended();
        let id = agent.restart();
        agent.feed(&initialized_with_methods(id, both()));
        agent.feed(&error(id + 1, AUTH_REQUIRED, "log in first"));
        assert_eq!(agent.login("oauth-personal").reply, Some(Ok(())));
        let n = agent.sent.len();
        agent.feed(&result(id + 2, json!({})));
        let methods: Vec<&Value> = agent.sent_since(n).iter().map(|l| &l["method"]).collect();
        assert_eq!(methods, [&json!("session/resume")]);
        agent.feed(&result(id + 3, json!({})));
        assert_eq!(
            *agent.s.phase(),
            Phase::Ready {
                session: "s1".into()
            }
        );
        assert_eq!(agent.notices().last().map(String::as_str), Some(RESTARTED));
    }

    #[test]
    fn a_continue_answered_with_another_error_fails_and_hangs_up() {
        let mut agent = Agent::ended();
        let id = agent.restart();
        agent.feed(&initialized(id, both()));
        let out = agent.feed(&error(id + 1, -32603, "no such session"));
        assert!(out.hang_up);
        let failed = AgentState::Failed {
            message: "the agent couldn't continue the conversation: no such session".into(),
        };
        assert_eq!(*agent.agent_state(), failed);
        assert_eq!(out.checkpoint, Some(Moment::Stopped));
        agent.step(Input::Exited {
            code: Some(0),
            stderr: b"bye\n",
        });
        assert_eq!(*agent.agent_state(), failed, "its exit leaves the failure");
        assert!(!agent.s.continuable());
    }

    #[test]
    fn an_unreadable_continue_answer_fails_naming_the_request() {
        for (capabilities, method) in [(both(), "session/resume"), (load_only(), "session/load")] {
            let mut agent = Agent::ended();
            let id = agent.restart();
            agent.feed(&initialized(id, capabilities));
            let out = agent.feed(&result(id + 1, json!("done")));
            assert!(out.hang_up);
            assert_eq!(
                *agent.agent_state(),
                AgentState::Failed {
                    message: format!(
                        "the agent couldn't continue the conversation: its answer to {method} isn't one Brindle can read"
                    )
                }
            );
        }
    }

    #[test]
    fn output_lost_mid_continue_fails_it() {
        let mut agent = Agent::ended();
        let id = agent.restart();
        agent.feed(&initialized(id, both()));
        let out = agent.step(Input::OutputLost);
        assert!(out.hang_up);
        assert_eq!(
            *agent.agent_state(),
            AgentState::Failed {
                message: "the agent couldn't continue the conversation: output was lost while the terminal server restarted".into()
            }
        );
    }

    // 2.5: restart and a lost holder.

    #[test]
    fn a_restart_adds_its_notice_starts_the_agent_and_asks_for_no_checkpoint() {
        let mut agent = Agent::ended();
        let out = agent.step(Input::Restart);
        assert_eq!(out.reply, Some(Ok(())));
        assert_eq!(shape(&out.events), ["added notice", "agent starting"]);
        assert_eq!(agent.notices().last().map(String::as_str), Some(RESTARTED));
        assert_eq!(out.checkpoint, None);
        assert!(out.stdin.is_empty() && !out.hang_up);
        assert!(matches!(
            agent.s.phase(),
            Phase::Spawning {
                start: Start::Continue { log: Log::Kept, .. }
            }
        ));
        assert_eq!(agent.s.lines().covers(), 0, "the new holder session's");
        let out = agent.step(Input::Spawned);
        assert_eq!(out.checkpoint, Some(Moment::BeforeRequest));
    }

    #[test]
    fn ids_continue_after_a_restart() {
        let mut agent = Agent::ended();
        let next_request = agent.s.rpc().next_id();
        let last_entry = agent.last_entry().0;
        let id = agent.restart();
        assert_eq!(id, next_request, "request ids go on");
        agent.feed(&initialized(id, both()));
        agent.feed(&result(id + 1, json!({})));
        agent.turn("again");
        let (user, entry) = agent
            .s
            .transcript()
            .entries()
            .find(|(_, entry)| matches!(entry, Entry::User { text } if text == "again"))
            .map(|(id, entry)| (id, entry.clone()))
            .expect("the prompt's entry");
        assert!(matches!(entry, Entry::User { .. }));
        // The restart's notice took the next id, the prompt the one after.
        assert_eq!(user, EntryId(last_entry.0 + 2));
        assert_eq!(agent.prompt_id(), id + 2);
    }

    #[test]
    fn the_turn_in_flight_at_an_exit_fails_and_is_never_sent_to_the_new_agent() {
        let mut agent = Agent::ready_offering(both());
        agent.turn("fix it");
        agent.exit();
        assert_eq!(
            *agent.turn_state(),
            TurnState::Failed {
                message: "the agent exited".into()
            }
        );
        let before = agent.sent.len();
        let id = agent.restart();
        agent.feed(&initialized(id, both()));
        let out = agent.feed(&result(id + 1, json!({})));
        assert!(out.stdin.is_empty(), "nothing sent on its own");
        let after = serde_json::to_string(agent.sent_since(before)).expect("JSON");
        assert!(!after.contains("fix it"), "{after}");
        assert!(matches!(agent.turn_state(), TurnState::Failed { .. }));
    }

    #[test]
    fn a_held_prompt_can_be_sent_again_once_the_continued_agent_is_ready_and_not_before() {
        let mut agent = Agent::ready_offering(both());
        agent.turn("first");
        agent.prompt("next");
        agent.exit();
        assert_eq!(*agent.slot(), held("next", HoldReason::AgentStopped));
        let before = agent.sent.len();
        let id = agent.restart();
        assert_eq!(
            agent.cmd(Command::Resend).reply,
            Some(Err(Refused::NotReady))
        );
        agent.feed(&initialized(id, both()));
        assert_eq!(
            agent.cmd(Command::Resend).reply,
            Some(Err(Refused::NotReady))
        );
        assert_eq!(*agent.slot(), held("next", HoldReason::AgentStopped));
        let out = agent.feed(&result(id + 1, json!({})));
        assert!(out.stdin.is_empty(), "not sent on its own");
        let methods: Vec<&str> = agent
            .sent_since(before)
            .iter()
            .filter_map(|line| line["method"].as_str())
            .collect();
        assert_eq!(methods, ["initialize", "session/resume"]);
        assert_eq!(agent.cmd(Command::Resend).reply, Some(Ok(())));
        assert_eq!(agent.last("session/prompt")["params"]["prompt"][0]["text"], "next");
        assert_eq!(*agent.slot(), Slot::Empty);
    }

    #[test]
    fn a_permission_cancelled_at_the_exit_stays_cancelled_and_is_never_answered_again() {
        let mut agent = Agent::ready_offering(both());
        agent.turn("first");
        let entry = agent.ask(json!(5), &ALLOW_REJECT);
        agent.exit();
        let cancelled = |agent: &Agent| {
            matches!(
                agent.s.transcript().entry(entry),
                Some(Entry::Permission {
                    outcome: Some(PermissionOutcome::Cancelled),
                    ..
                })
            )
        };
        assert!(cancelled(&agent));
        let before = agent.sent.len();
        let id = agent.restart();
        agent.feed(&initialized(id, both()));
        agent.feed(&result(id + 1, json!({})));
        assert!(cancelled(&agent));
        assert!(agent.s.rpc().permissions().is_empty());
        assert!(
            agent.sent_since(before).iter().all(|line| line.get("result").is_none()),
            "no answer to the old request"
        );
        assert_eq!(
            agent
                .cmd(Command::Answer {
                    entry,
                    option: Some("allow-once".into())
                })
                .reply,
            Some(Err(Refused::AlreadyAnswered { entry }))
        );
    }

    #[test]
    fn a_restart_is_refused_unless_the_agent_ended_with_a_conversation_to_continue() {
        let mut running = Agent::ready_offering(both());
        let out = running.step(Input::Restart);
        assert_eq!(out.reply, Some(Err(Refused::NotEnded)));
        assert!(out.events.is_empty() && out.stdin.is_empty());
        let mut starting = Agent::ended();
        starting.restart();
        assert_eq!(
            starting.step(Input::Restart).reply,
            Some(Err(Refused::NotEnded))
        );
        let mut neither = Agent::ready_offering(json!({}));
        neither.exit();
        assert_eq!(
            neither.step(Input::Restart).reply,
            Some(Err(Refused::CannotContinue))
        );
        let mut refused = Agent::new();
        refused.step(Input::Spawned);
        refused.feed(&result(1, json!({"protocolVersion": 2})));
        refused.exit();
        assert_eq!(
            refused.step(Input::Restart).reply,
            Some(Err(Refused::CannotContinue))
        );
        for (refusal, text) in [
            (Refused::NotEnded, "the agent hasn't ended"),
            (
                Refused::CannotContinue,
                "the agent can't continue this conversation",
            ),
            (Refused::Lost, "the agent was lost with its holder"),
        ] {
            assert_eq!(refusal.to_string(), text);
        }
    }

    #[test]
    fn a_restart_that_couldnt_be_placed_returns_to_ended_and_can_be_tried_again() {
        let mut agent = Agent::ended();
        assert_eq!(agent.step(Input::Restart).reply, Some(Ok(())));
        agent.step(Input::Exited {
            code: None,
            stderr: b"no such file: fake-agent",
        });
        assert_eq!(
            *agent.agent_state(),
            AgentState::Exited {
                code: None,
                stderr: "no such file: fake-agent".into()
            }
        );
        assert!(agent.s.continuable());
        agent.restart();
    }

    #[test]
    fn a_lost_holder_ends_the_agent_as_an_exit_does_but_lost() {
        let mut agent = Agent::ready_offering(load_only());
        agent.turn("first");
        let entry = agent.ask(json!(1), &ALLOW_REJECT);
        agent.prompt("next");
        let out = agent.step(Input::Lost {
            message: "its holder ended",
        });
        assert_eq!(
            shape(&out.events),
            ["answered", "queue held", "turn failed", "agent lost"]
        );
        assert!(matches!(
            agent.s.transcript().entry(entry),
            Some(Entry::Permission {
                outcome: Some(PermissionOutcome::Cancelled),
                ..
            })
        ));
        assert_eq!(*agent.slot(), held("next", HoldReason::AgentStopped));
        assert_eq!(
            *agent.turn_state(),
            TurnState::Failed {
                message: "the agent exited".into()
            }
        );
        assert_eq!(
            *agent.agent_state(),
            AgentState::Lost {
                message: "its holder ended".into()
            }
        );
        assert_eq!(*agent.s.phase(), Phase::Lost);
        assert_eq!(out.checkpoint, Some(Moment::Stopped));
        assert_eq!(agent.activity(), Activity::Error);
        assert_eq!(agent.prompt("x").reply, Some(Err(Refused::Lost)));
        // Nothing more ends it.
        assert!(agent.exit().events.is_empty());
        assert!(agent
            .step(Input::Lost {
                message: "again"
            })
            .events
            .is_empty());
        let id = agent.restart();
        agent.feed(&initialized(id, load_only()));
        assert_eq!(agent.last("session/load")["params"]["sessionId"], "s1");
    }

    // --- closing what a process left open (agents-process-lifecycle) -----

    /// A-3: the restart after a terminal login hangs the old agent up, so
    /// the permission it asked is closed before the new one is shown
    /// starting, and no answer to it ever reaches either agent.
    #[test]
    fn a_permission_waiting_at_a_login_restart_is_recorded_cancelled_before_the_agent_starts() {
        let mut agent = Agent::ready_with_methods();
        let id = agent.turn("one");
        agent.feed(&said("Working"));
        let entry = agent.ask(json!("p7"), &ALLOW_REJECT);
        agent.feed(&error(id, AUTH_REQUIRED, "expired"));
        let out = agent.step(Input::Restart);
        assert_eq!(out.reply, Some(Ok(())));
        assert_eq!(shape(&out.events), ["answered", "added notice", "agent starting"]);
        assert_eq!(
            out.events.first(),
            Some(&Event::Answered {
                id: entry,
                outcome: PermissionOutcome::Cancelled
            })
        );
        assert!(out.stdin.is_empty(), "nothing goes to an agent being hung up");
        assert!(out.hang_up);

        // The new agent takes up the session, is sent nothing for the
        // request, and a late answer to it is refused naming it.
        let sent = agent.sent.len();
        let id = agent.restart();
        agent.feed(&initialized(id, resume_only()));
        let resume = agent.last("session/resume")["id"].as_u64().expect("an id");
        agent.feed(&result(resume, json!({})));
        assert_eq!(*agent.s.phase(), Phase::Ready { session: "s1".into() });
        assert_eq!(
            agent
                .cmd(Command::Answer {
                    entry,
                    option: Some("allow-once".into())
                })
                .reply,
            Some(Err(Refused::AlreadyAnswered { entry }))
        );
        assert!(
            agent.sent_since(sent).iter().all(|line| line["id"] != json!("p7")),
            "{:?}",
            agent.sent_since(sent)
        );
    }

    /// A-3: a list out to the agent being hung up for its login restart is
    /// refused there, and the next goes to the new agent.
    #[test]
    fn a_list_out_at_a_login_restart_is_refused_saying_the_agent_is_started_again() {
        let mut agent = Agent::ready_with_methods();
        agent.s.lists = true;
        let id = agent.turn("one");
        assert_eq!(agent.list(3, None).reply, Some(Ok(())));
        agent.feed(&error(id, AUTH_REQUIRED, "expired"));
        let out = agent.step(Input::Restart);
        assert_eq!(out.reply, Some(Ok(())));
        assert_eq!(
            out.listed,
            Some(ListAnswer {
                req: 3,
                result: Err("the agent is started again after its login".into())
            })
        );
        assert!(agent.s.rpc.listing().is_none());
        let id = agent.restart();
        agent.feed(&initialized(id, json!({"sessionCapabilities": {"resume": {}, "list": {}}})));
        let resume = agent.last("session/resume")["id"].as_u64().expect("an id");
        agent.feed(&result(resume, json!({})));
        assert_eq!(agent.list(4, None).reply, Some(Ok(())));
    }

    /// Observation 6: a step that faults sends nothing it asked for; the
    /// prompt here is never written, though its id was taken.
    #[test]
    fn a_fold_error_writes_nothing_the_faulted_step_asked_for() {
        let mut agent = Agent::ready();
        agent.event_limit = 1024;
        let out = agent.prompt(&"x".repeat(4096));
        assert!(
            matches!(agent.agent_state(), AgentState::Failed { .. }),
            "{:?}",
            agent.agent_state()
        );
        assert!(out.stdin.is_empty(), "{} bytes written", out.stdin.len());
        assert!(!agent.sent_methods().contains(&"session/prompt"));
    }

    /// A-3: after a handling fault the agent runs on, and no choice can
    /// come for what it asked; it is answered `cancelled`, as the log says.
    #[test]
    fn a_handling_fault_cancels_and_answers_the_waiting_permissions() {
        let mut agent = Agent::ready();
        agent.turn("one");
        let entry = agent.ask(json!("p1"), &ALLOW_REJECT);
        let sent = agent.sent.len();
        let out = agent.step(Input::Fault {
            reason: "panicked at reduce.rs".into(),
        });
        assert!(!out.hang_up);
        assert_eq!(shape(&out.events), ["answered", "turn failed", "agent failed"]);
        assert_eq!(
            agent.s.transcript.entry(entry).and_then(|entry| match entry {
                Entry::Permission { outcome, .. } => outcome.clone(),
                _ => None,
            }),
            Some(PermissionOutcome::Cancelled)
        );
        let answers = agent.sent_since(sent);
        assert_eq!(answers.len(), 1, "{answers:?}");
        assert_eq!(answers[0]["id"], json!("p1"));
        assert_eq!(answers[0]["result"]["outcome"]["outcome"], json!("cancelled"));
    }

    /// A-3: a load the new agent refuses closes a permission it asked
    /// during the muted replay, and the turn it blocked is no longer
    /// blocked.
    #[test]
    fn a_refused_load_cancels_a_permission_asked_during_it_and_unblocks_the_turn() {
        let mut agent = Agent::ended();
        let id = agent.restart();
        agent.feed(&initialized(id, load_only()));
        let entry = agent.ask(json!("p9"), &ALLOW_REJECT);
        assert_eq!(*agent.turn_state(), TurnState::Blocked);
        let sent = agent.sent.len();
        let out = agent.feed(&error(id + 1, -32603, "no such session"));
        assert!(out.hang_up);
        assert_eq!(shape(&out.events)[..2], ["answered", "turn idle"]);
        assert!(matches!(
            agent.s.transcript.entry(entry),
            Some(Entry::Permission {
                outcome: Some(PermissionOutcome::Cancelled),
                ..
            })
        ));
        assert!(agent.sent_since(sent).is_empty(), "nothing goes to a refused agent");
        assert!(matches!(agent.agent_state(), AgentState::Failed { .. }));
    }

    /// A-3: the exit closed what the agent had out; the restart after it
    /// has nothing left to close.
    #[test]
    fn a_restart_after_an_exit_answers_nothing_already_closed() {
        let mut agent = Agent::ready_offering(both());
        agent.turn("one");
        agent.ask(json!("p2"), &ALLOW_REJECT);
        let out = agent.exit();
        assert_eq!(shape(&out.events), ["answered", "turn failed", "agent exited"]);
        let out = agent.step(Input::Restart);
        assert_eq!(out.reply, Some(Ok(())));
        assert_eq!(shape(&out.events), ["added notice", "agent starting"]);
        let sent = agent.sent.len();
        agent.step(Input::Spawned);
        assert_eq!(
            agent
                .sent_since(sent)
                .iter()
                .filter_map(|line| line["method"].as_str())
                .collect::<Vec<_>>(),
            ["initialize"]
        );
    }

    /// C-4: a session has ended once its exit or loss is stepped, and not
    /// before, nor once its agent is started again.
    #[test]
    fn a_session_has_ended_once_its_exit_or_loss_is_stepped() {
        let mut agent = Agent::ready_offering(both());
        assert!(!agent.s.has_ended());
        agent.exit();
        assert!(agent.s.has_ended());
        agent.step(Input::Restart);
        assert!(!agent.s.has_ended(), "started again");

        let mut lost = Agent::ready();
        lost.step(Input::Lost { message: "its holder ended" });
        assert!(lost.s.has_ended());
        // A failed handling isn't an end: the agent runs on.
        let mut failed = Agent::ready();
        failed.step(Input::Fault {
            reason: "boom".into(),
        });
        assert!(!failed.s.has_ended());
    }

    /// A respawn keeps nothing of the old process: its line in progress,
    /// its chunk run, its bad-line run, and its tool calls and outputs.
    #[test]
    fn a_respawn_leaves_no_state_of_the_old_process() {
        let mut agent = Agent::ready();
        agent.step(Input::Stdout(b"{\"jsonrpc\": \"2.0\", \"method\": \"half"));
        agent.s.run = Some(Run {
            kind: RunKind::Agent,
            entry: EntryId(1),
        });
        agent.s.bad = Some(BadRun {
            lines: 3,
            last: agent.now,
        });
        agent.s.book.tools.insert("c1".into(), EntryId(1));
        let open = OpenOutput {
            entry: EntryId(1),
            item: 0,
            filter: crate::acp::plain::State::default(),
        };
        agent.s.book.outputs.insert("t1".into(), open);
        assert_ne!(agent.s.lines, Lines::new(), "a line in progress");
        let mut step = Step {
            s: &mut agent.s,
            out: Output::default(),
            fault: None,
            now: agent.now,
            event_limit: MAX_EVENT,
        };
        step.respawn(Start::New);
        assert_eq!(agent.s.lines, Lines::new());
        assert_eq!(agent.s.run, None);
        assert_eq!(agent.s.bad, None);
        assert_eq!(agent.s.book, ToolBook::default());
    }
}
