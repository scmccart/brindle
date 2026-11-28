use brindle_remote_protocol::serde_json::Value;
use brindle_transcript::{
    AgentState, HoldReason, LoginKind, LoginMethod, NoticeLevel, TurnState, clean,
};

use crate::acp::envelope::RpcError;
use crate::acp::forms::{self, Request};
use crate::acp::rpc::Purpose;

use super::lifecycle::{Ending, Greeting, Phase, Start};
use super::text::bound;
use super::{Moment, Refused, Step};

/// An agent waiting for a login (`agents-login` decision 4): what its
/// `initialize` said, for the later ready state and conversation, the
/// agent's last login message, and how the session goes on once a login
/// succeeds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Login {
    pub greeting: Greeting,
    pub message: String,
    pub then: AfterLogin,
}

/// How a session goes on after a login.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AfterLogin {
    /// A start request answered `-32000`: it is sent again (`session/new`,
    /// or the continue's `session/resume` or `session/load`).
    Start(Start),
    /// A prompt answered `-32000`: the agent is ready again in the ACP
    /// session the prompt was sent in, and nothing is sent on its own.
    Ready { session: String },
}

impl Step<'_> {
    /// Keeps the login methods `initialize` advertised, their text cleaned,
    /// with one warning notice when any were left out or cut
    /// (`agents-login` decision 3). Brindle always advertises
    /// `auth.terminal`.
    pub(super) fn keep_methods(&mut self, read: forms::Methods) {
        self.s.methods = read
            .methods
            .into_iter()
            .map(|method| LoginMethod {
                name: clean(&method.name),
                description: method.description.map(|text| clean(&text)),
                ..method
            })
            .collect();
        if let Some(text) = methods_notice(read.dropped, read.cut) {
            self.notice(NoticeLevel::Warning, text);
        }
    }

    /// `-32000` to a start request (decision 17): the agent waits for a
    /// login, after which `then` goes on.
    pub(super) fn login_required(&mut self, error: &RpcError, greeting: Greeting, then: AfterLogin) {
        self.needs_login(Login {
            greeting,
            message: bound(clean(&error.message)),
            then,
        });
        self.mark(Moment::Started);
    }

    /// The agent needs a login: the phase and the agent state, with its
    /// methods (`agents-login` decision 4). The caller marks the moment.
    pub(super) fn needs_login(&mut self, login: Login) {
        let message = login.message.clone();
        self.s.phase = Phase::LoginRequired(login);
        self.agent(AgentState::LoginRequired {
            message,
            methods: self.s.methods.clone(),
        });
    }

    /// `authenticate`'s answer (`agents-login` decision 4): a result, of
    /// whatever shape, is a login, and the session goes on (`Starting`
    /// first); an error needs a login again, with its message.
    pub(super) fn authenticated(&mut self, result: Result<&Value, &RpcError>) {
        let Phase::LoggingIn { login, .. } = &self.s.phase else {
            return;
        };
        let login = login.clone();
        match result {
            Ok(_) => {
                self.agent(AgentState::Starting);
                self.mark(Moment::Login);
                match login.then {
                    AfterLogin::Start(start) => self.start(start, login.greeting),
                    AfterLogin::Ready { session } => {
                        self.s.phase = Phase::Ready { session };
                        self.agent(AgentState::Ready {
                            name: login.greeting.name,
                            version: login.greeting.version,
                        });
                    }
                }
            }
            Err(error) => {
                self.needs_login(Login {
                    message: bound(clean(&error.message)),
                    ..login
                });
                self.mark(Moment::Login);
            }
        }
    }

    /// Decision 17: the turn fails, a waiting prompt is held, then the
    /// agent needs a login (the queue's change first, as decision 10's
    /// table says), after which it is ready again in the same ACP session.
    /// Neither prompt is ever sent again on its own (`agents-login`
    /// decision 5).
    pub(super) fn prompt_needs_login(&mut self, error: &RpcError) {
        let message = bound(clean(&error.message));
        // A prompt is out only while ready.
        let Phase::Ready { session } = &self.s.phase else {
            return self.end_turn(Ending::Failed(message));
        };
        let login = Login {
            greeting: self.greeting_now(),
            message: message.clone(),
            then: AfterLogin::Ready {
                session: session.clone(),
            },
        };
        self.s.process.run = None;
        self.hold_waiting(HoldReason::LoginRequired);
        self.turn(TurnState::Failed { message });
        self.needs_login(login);
        self.mark(Moment::TurnEnded);
    }

    /// `AGENT_LOGIN` (`agents-login` decision 4): with the agent waiting
    /// for a login, `authenticate` with one of its agent methods goes out,
    /// checkpointed first, and the agent is logging in. A terminal method
    /// is the client's to run; anything else is refused and changes
    /// nothing.
    pub(super) fn login(&mut self, method: String) -> Result<(), Refused> {
        let login = match &self.s.phase {
            Phase::LoginRequired(login) => login.clone(),
            Phase::LoggingIn { .. } => return Err(Refused::AlreadyLoggingIn),
            Phase::Spawning { .. }
            | Phase::Initializing { .. }
            | Phase::Creating { .. }
            | Phase::Resuming { .. }
            | Phase::Loading { .. }
            | Phase::Ready { .. }
            | Phase::Choosing { .. }
            | Phase::Restarting { .. }
            | Phase::Refused
            | Phase::Broken
            | Phase::Exited
            | Phase::Lost => return Err(Refused::NoLoginNeeded),
        };
        let Some(chosen) = self.s.methods.iter().find(|offered| offered.id == method) else {
            return Err(Refused::NoSuchMethod { method });
        };
        let name = match &chosen.kind {
            LoginKind::Agent => chosen.name.clone(),
            LoginKind::Terminal { .. } => return Err(Refused::RunByClient { method }),
        };
        let line = self.s.rpc.authenticate(&method);
        self.s.phase = Phase::LoggingIn {
            login,
            method: method.clone(),
        };
        self.agent(AgentState::LoggingIn { method, name });
        self.out.stdin.extend(line);
        self.mark(Moment::BeforeRequest);
        Ok(())
    }

    /// `AGENT_CANCEL` while logging in: `$/cancel_request` for the
    /// `authenticate`, whose answer is then ignored, and the agent needs a
    /// login again, with its earlier message.
    pub(super) fn cancel_login(&mut self, login: Login) {
        if let Some(id) = self.s.rpc.awaited(Purpose::Authenticate) {
            self.s.rpc.forget_id(id);
            self.out
                .stdin
                .extend(self.s.rpc.dialect().encode(Request::Withdraw { id }));
        }
        self.needs_login(login);
        self.mark(Moment::Login);
    }
}

/// The warning when login methods were left out or their descriptions cut
/// (`agents-login` decision 3), or `None` when neither.
fn methods_notice(dropped: usize, cut: usize) -> Option<String> {
    let left_out = match dropped {
        0 => None,
        1 => Some("1 login method from the agent was left out".to_owned()),
        n => Some(format!("{n} login methods from the agent were left out")),
    };
    let cut = match cut {
        0 => None,
        1 => Some("1 login method's description was cut".to_owned()),
        n => Some(format!("{n} login methods' descriptions were cut")),
    };
    match (left_out, cut) {
        (Some(left_out), Some(cut)) => Some(format!("{left_out}, and {cut}")),
        (Some(text), None) | (None, Some(text)) => Some(text),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use brindle_remote_protocol::serde_json::json;
    use brindle_transcript::{Activity, Continuation};

    use crate::acp::forms::AUTH_REQUIRED;

    use super::super::harness::*;
    use super::super::lifecycle::RESTARTED_AFTER_LOGIN;
    use super::super::*;
    use super::*;


    #[test]
    fn the_methods_are_kept_from_initialize_cleaned_in_the_agents_order() {
        let agent = Agent::needing_login();
        assert_eq!(agent.s.methods, methods());
        assert!(agent.notices().is_empty(), "{:?}", agent.notices());
    }

    #[test]
    fn methods_left_out_or_cut_add_one_warning() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        let long = "d".repeat(forms::MAX_LOGIN_DESCRIPTION + 1);
        let out = agent.feed(&result(
            1,
            json!({"protocolVersion": 1, "authMethods": [
                {"id": "a", "name": "A", "description": long},
                {"id": "env", "name": "Env", "type": "env_var"},
                {"name": "no id"},
            ]}),
        ));
        assert_eq!(
            agent.notices(),
            ["2 login methods from the agent were left out, and 1 login method's description was cut"]
        );
        assert!(matches!(
            out.events.last(),
            Some(Event::Added { entry: Entry::Notice { level: NoticeLevel::Warning, .. }, .. })
        ));
        assert_eq!(agent.s.methods.len(), 1);
        assert_eq!(methods_notice(0, 0), None);
        assert_eq!(
            methods_notice(1, 0).as_deref(),
            Some("1 login method from the agent was left out")
        );
        assert_eq!(
            methods_notice(0, 2).as_deref(),
            Some("2 login methods' descriptions were cut")
        );
    }

    #[test]
    fn session_new_answered_32000_needs_a_login_offering_the_methods() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        agent.feed(&initialized_with_methods(1, both()));
        let out = agent.feed(&error(2, AUTH_REQUIRED, "Authentication required"));
        assert_eq!(*agent.agent_state(), login_required("Authentication required"));
        assert_eq!(out.checkpoint, Some(Moment::Started));
        assert!(!out.hang_up && out.stdin.is_empty());
        assert_eq!(
            *agent.s.phase(),
            Phase::LoginRequired(Login {
                greeting: Greeting {
                    name: Some("fake".into()),
                    version: Some("2.0".into()),
                    continuation: Continuation::ResumeAndLoad,
                },
                message: "Authentication required".into(),
                then: AfterLogin::Start(Start::New),
            })
        );
    }

    #[test]
    fn a_prompt_answered_32000_needs_a_login_in_its_session_offering_the_methods() {
        let mut agent = Agent::ready_with_methods();
        let id = agent.turn("one");
        agent.prompt("two");
        let out = agent.feed(&error(id, AUTH_REQUIRED, "expired"));
        assert_eq!(
            shape(&out.events),
            ["queue held", "turn failed", "agent login required"]
        );
        assert_eq!(*agent.agent_state(), login_required("expired"));
        assert_eq!(out.checkpoint, Some(Moment::TurnEnded));
        let Phase::LoginRequired(login) = agent.s.phase() else {
            panic!("{:?}", agent.s.phase());
        };
        assert_eq!(
            login.then,
            AfterLogin::Ready {
                session: "s1".into()
            }
        );
        assert_eq!(login.greeting.name.as_deref(), Some("fake"));
        assert_eq!(login.greeting.continuation, Continuation::ResumeAndLoad);
    }

    #[test]
    fn a_login_with_an_agent_method_sends_authenticate_and_is_logging_in() {
        let mut agent = Agent::needing_login();
        let out = agent.login("oauth-personal");
        assert_eq!(out.reply, Some(Ok(())));
        assert_eq!(shape(&out.events), ["agent logging in"]);
        assert_eq!(*agent.agent_state(), logging_in());
        assert_eq!(agent.activity(), Activity::Working);
        assert_eq!(out.checkpoint, Some(Moment::BeforeRequest));
        assert_eq!(
            String::from_utf8_lossy(&out.stdin),
            "{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"authenticate\",\"params\":{\"methodId\":\"oauth-personal\"}}\n"
        );
        assert!(matches!(
            agent.s.phase(),
            Phase::LoggingIn { method, .. } if method == "oauth-personal"
        ));
        assert_eq!(agent.s.rpc().awaited(Purpose::Authenticate), Some(3));
    }

    #[test]
    fn a_login_with_a_terminal_or_unknown_method_is_refused_and_changes_nothing() {
        for (method, refused) in [
            (
                "cli",
                Refused::RunByClient {
                    method: "cli".into(),
                },
            ),
            (
                "nope",
                Refused::NoSuchMethod {
                    method: "nope".into(),
                },
            ),
        ] {
            let mut agent = Agent::needing_login();
            let before = agent.s.clone();
            let out = agent.login(method);
            assert_eq!(out.reply, Some(Err(refused)));
            assert!(out.events.is_empty() && out.stdin.is_empty());
            assert_eq!(out.checkpoint, None);
            assert_eq!(agent.s, before);
        }
    }

    #[test]
    fn a_login_while_none_is_needed_or_one_runs_is_refused() {
        let mut ready = Agent::ready_with_methods();
        assert_eq!(
            ready.login("oauth-personal").reply,
            Some(Err(Refused::NoLoginNeeded))
        );
        assert!(!ready.sent_methods().contains(&"authenticate"));
        let mut starting = Agent::new();
        assert_eq!(
            starting.login("oauth-personal").reply,
            Some(Err(Refused::NoLoginNeeded))
        );
        let mut agent = Agent::needing_login();
        agent.login("oauth-personal");
        let out = agent.login("api-key");
        assert_eq!(out.reply, Some(Err(Refused::AlreadyLoggingIn)));
        assert!(out.stdin.is_empty());
    }

    #[test]
    fn a_login_at_the_start_answered_starts_the_session() {
        let mut agent = Agent::needing_login();
        agent.login("oauth-personal");
        let out = agent.feed(&result(3, json!({})));
        assert_eq!(shape(&out.events), ["agent starting"]);
        assert_eq!(out.checkpoint, Some(Moment::BeforeRequest));
        assert_eq!(agent.last("session/new")["id"], json!(4));
        assert!(matches!(agent.s.phase(), Phase::Creating { .. }));
        agent.feed(&result(4, json!({"sessionId": "s1"})));
        assert_eq!(
            *agent.agent_state(),
            AgentState::Ready {
                name: Some("fake".into()),
                version: Some("2.0".into())
            }
        );
        assert_eq!(agent.turn("hi"), 5);
    }

    #[test]
    fn session_new_answering_32000_again_after_a_login_needs_one_again() {
        let mut agent = Agent::needing_login();
        agent.login("oauth-personal");
        agent.feed(&result(3, json!(null)));
        let out = agent.feed(&error(4, AUTH_REQUIRED, "still not"));
        assert_eq!(*agent.agent_state(), login_required("still not"));
        assert_eq!(out.checkpoint, Some(Moment::Started));
        assert!(matches!(
            agent.s.phase(),
            Phase::LoginRequired(Login { then: AfterLogin::Start(Start::New), .. })
        ));
        assert_eq!(agent.login("api-key").reply, Some(Ok(())));
    }

    #[test]
    fn a_login_after_a_prompts_demand_is_ready_in_the_same_session_and_sends_nothing() {
        let mut agent = Agent::ready_with_methods();
        let id = agent.turn("one");
        agent.feed(&error(id, AUTH_REQUIRED, "expired"));
        agent.login("api-key");
        let auth = agent.authenticate_id();
        let n = agent.sent.len();
        let out = agent.feed(&result(auth, json!({})));
        assert_eq!(shape(&out.events), ["agent starting", "agent ready"]);
        assert_eq!(out.checkpoint, Some(Moment::Login));
        assert!(out.stdin.is_empty());
        assert_eq!(agent.sent.len(), n);
        assert_eq!(
            *agent.s.phase(),
            Phase::Ready {
                session: "s1".into()
            }
        );
        assert_eq!(
            *agent.agent_state(),
            AgentState::Ready {
                name: Some("fake".into()),
                version: Some("2.0".into())
            }
        );
        // The next prompt goes to the same session.
        agent.turn("two");
        assert_eq!(agent.last("session/prompt")["params"]["sessionId"], json!("s1"));
    }

    /// `agents-login` decision 5: neither the failed prompt nor the held
    /// one is sent on its own; the held one goes again only by a resend,
    /// refused while logging in and taken once ready.
    #[test]
    fn a_failed_prompt_is_never_sent_again_and_a_held_one_only_by_a_resend_once_ready() {
        let mut agent = Agent::ready_with_methods();
        let id = agent.turn("one");
        agent.prompt("two");
        agent.feed(&error(id, AUTH_REQUIRED, "expired"));
        assert_eq!(
            agent.cmd(Command::Resend).reply,
            Some(Err(Refused::LoginRequired))
        );
        agent.login("oauth-personal");
        assert_eq!(
            agent.cmd(Command::Resend).reply,
            Some(Err(Refused::LoggingIn))
        );
        assert_eq!(agent.prompt("three").reply, Some(Err(Refused::LoggingIn)));
        let auth = agent.authenticate_id();
        agent.feed(&result(auth, json!({})));
        let prompts = |agent: &Agent| {
            agent
                .sent
                .iter()
                .filter(|line| line["method"] == "session/prompt")
                .map(|line| line["params"]["prompt"][0]["text"].clone())
                .collect::<Vec<_>>()
        };
        assert_eq!(prompts(&agent), [json!("one")]);
        assert_eq!(*agent.slot(), held("two", HoldReason::LoginRequired));
        assert_eq!(agent.cmd(Command::Resend).reply, Some(Ok(())));
        assert_eq!(prompts(&agent), [json!("one"), json!("two")]);
    }

    #[test]
    fn while_a_login_is_needed_or_runs_unqueue_still_discards() {
        for running in [false, true] {
            let mut agent = Agent::ready_with_methods();
            let id = agent.turn("one");
            agent.prompt("two");
            agent.feed(&error(id, AUTH_REQUIRED, "expired"));
            if running {
                agent.login("oauth-personal");
            }
            assert_eq!(agent.cmd(Command::Unqueue).reply, Some(Ok(())));
            assert_eq!(*agent.slot(), Slot::Empty);
        }
    }

    #[test]
    fn a_failed_login_needs_one_again_with_its_message() {
        let mut agent = Agent::needing_login();
        agent.login("oauth-personal");
        let out = agent.feed(&error(3, -32603, "the browser was closed"));
        assert_eq!(*agent.agent_state(), login_required("the browser was closed"));
        assert_eq!(out.checkpoint, Some(Moment::Login));
        assert!(out.stdin.is_empty() && !out.hang_up);
        // `-32000` from authenticate itself is just an error.
        agent.login("api-key");
        agent.feed(&error(4, AUTH_REQUIRED, "bad key"));
        assert_eq!(*agent.agent_state(), login_required("bad key"));
        assert!(matches!(
            agent.s.phase(),
            Phase::LoginRequired(Login { then: AfterLogin::Start(Start::New), .. })
        ));
    }

    #[test]
    fn a_cancelled_login_sends_cancel_request_and_ignores_the_late_answer() {
        let mut agent = Agent::needing_login();
        agent.login("oauth-personal");
        let out = agent.cmd(Command::Cancel);
        assert_eq!(out.reply, None);
        assert_eq!(
            String::from_utf8_lossy(&out.stdin),
            "{\"jsonrpc\":\"2.0\",\"method\":\"$/cancel_request\",\"params\":{\"requestId\":3}}\n"
        );
        assert_eq!(*agent.agent_state(), login_required("Authentication required"));
        assert_eq!(out.checkpoint, Some(Moment::Login));
        assert_eq!(agent.s.rpc().awaited(Purpose::Authenticate), None);
        // The agent's late answer, a success or an error, is ignored.
        for late in [result(3, json!({})), error(3, -32800, "cancelled")] {
            let out = agent.feed(&late);
            assert!(out.events.is_empty() && out.stdin.is_empty(), "{out:?}");
        }
        // No session/new beyond the one answered -32000.
        let news = agent.sent_methods().iter().filter(|m| **m == "session/new").count();
        assert_eq!(news, 1);
        // And a new login can be chosen.
        assert_eq!(agent.login("api-key").reply, Some(Ok(())));
        assert_eq!(agent.authenticate_id(), 4);
    }

    #[test]
    fn a_cancel_while_a_login_is_needed_does_nothing() {
        let mut agent = Agent::needing_login();
        let out = agent.cmd(Command::Cancel);
        assert_eq!(out, Output::default());
    }

    #[test]
    fn the_agent_exiting_while_logging_in_exits_and_forgets_the_login() {
        let mut agent = Agent::needing_login();
        agent.login("oauth-personal");
        let out = agent.step(Input::Exited {
            code: Some(1),
            stderr: b"",
        });
        assert_eq!(shape(&out.events), ["agent exited"]);
        assert_eq!(*agent.s.phase(), Phase::Exited);
        assert_eq!(agent.s.rpc().awaited(Purpose::Authenticate), None);
        assert_eq!(
            agent.login("oauth-personal").reply,
            Some(Err(Refused::NoLoginNeeded))
        );
    }

    #[test]
    fn output_lost_while_logging_in_needs_the_login_again() {
        let mut agent = Agent::needing_login();
        agent.login("oauth-personal");
        let out = agent.step(Input::OutputLost);
        assert_eq!(
            *agent.agent_state(),
            login_required("the login's answer was lost while the terminal server restarted")
        );
        assert_eq!(out.checkpoint, Some(Moment::Login));
        assert_eq!(agent.s.rpc().awaited(Purpose::Authenticate), None);
        assert!(matches!(
            agent.s.phase(),
            Phase::LoginRequired(Login { then: AfterLogin::Start(Start::New), .. })
        ));
        // The answer, should it still come, matches nothing.
        assert!(agent.feed(&result(3, json!({}))).events.is_empty());
        // Lost output while a login is only needed leaves it needed.
        let mut needed = Agent::needing_login();
        needed.step(Input::OutputLost);
        assert_eq!(*needed.agent_state(), login_required("Authentication required"));
    }

    #[test]
    fn the_login_refusals_say_why() {
        for (refused, text) in [
            (Refused::LoginRequired, "the agent needs a login"),
            (Refused::NoLoginNeeded, "the agent isn't waiting for a login"),
            (Refused::AlreadyLoggingIn, "a login is already running"),
            (
                Refused::NoSuchMethod {
                    method: "x".into(),
                },
                "the agent offers no login method \"x\"",
            ),
            (
                Refused::RunByClient {
                    method: "x".into(),
                },
                "login method \"x\" runs in a terminal, which Brindle opens",
            ),
            (Refused::LoggingIn, "the agent is logging in"),
        ] {
            assert_eq!(refused.to_string(), text);
        }
    }

    // agents-login 5.3: a restart while a login is needed.

    #[test]
    fn a_restart_at_a_starts_login_demand_hangs_up_then_sends_session_new_again() {
        let mut agent = Agent::needing_login();
        let out = agent.step(Input::Restart);
        assert_eq!(out.reply, Some(Ok(())));
        assert!(out.hang_up, "the running agent is hung up");
        assert!(out.stdin.is_empty());
        assert_eq!(out.checkpoint, None);
        assert_eq!(shape(&out.events), ["added notice", "agent starting"]);
        assert_eq!(
            agent.notices().last().map(String::as_str),
            Some(RESTARTED_AFTER_LOGIN)
        );
        assert_eq!(*agent.s.phase(), Phase::Restarting { start: Start::New });
        assert!(!agent.s.continuable());
        // Nothing is read from the agent being hung up, and nothing taken.
        let n = agent.events.len();
        agent.feed(&said("late words"));
        assert_eq!(agent.events.len(), n);
        assert_eq!(agent.prompt("hi").reply, Some(Err(Refused::NotReady)));
        assert_eq!(
            agent.login("oauth-personal").reply,
            Some(Err(Refused::NoLoginNeeded))
        );

        // Its exit: the core steps the restart again, and the new agent
        // starts as a new conversation, the request ids going on.
        let out = agent.step(Input::Restart);
        assert_eq!(out.reply, Some(Ok(())));
        assert!(out.events.is_empty() && !out.hang_up);
        assert_eq!(*agent.s.phase(), Phase::Spawning { start: Start::New });
        agent.step(Input::Spawned);
        let init = agent.last("initialize");
        assert_eq!(init["id"], json!(3));
        assert_eq!(
            init["params"]["clientCapabilities"],
            json!({"auth": {"terminal": true}, "session": {"configOptions": {"boolean": {}}}})
        );
        agent.feed(&initialized_with_methods(3, both()));
        assert_eq!(agent.last("session/new")["id"], json!(4));
        let out = agent.feed(&result(4, json!({"sessionId": "s2"})));
        assert_eq!(shape(&out.events), ["conversation", "agent ready"]);
        assert_eq!(
            *agent.s.phase(),
            Phase::Ready {
                session: "s2".into()
            }
        );
    }

    #[test]
    fn a_restart_at_a_prompts_login_demand_takes_up_its_session_keeping_the_log() {
        let mut agent = Agent::ready_with_methods();
        let id = agent.turn("one");
        agent.prompt("two");
        agent.feed(&error(id, AUTH_REQUIRED, "expired"));
        let log = agent.entries().len();
        assert_eq!(agent.step(Input::Restart).reply, Some(Ok(())));
        assert_eq!(
            *agent.s.phase(),
            Phase::Restarting {
                start: Start::Continue {
                    conversation: conversation_s1(Continuation::ResumeAndLoad),
                    log: Log::Kept,
                }
            }
        );
        let id = agent.restart();
        agent.feed(&initialized(id, resume_only()));
        let resume = agent.last("session/resume");
        assert_eq!(resume["params"]["sessionId"], json!("s1"));
        assert_eq!(resume["params"]["cwd"], json!("/home/u/src"));
        let out = agent.feed(&result(resume["id"].as_u64().expect("an id"), json!({})));
        assert_eq!(shape(&out.events), ["conversation", "agent ready"]);
        // The log (with its notice) and the held prompt are kept, and the
        // held prompt goes again only by a resend.
        assert_eq!(agent.entries().len(), log + 1);
        assert_eq!(
            *agent.s.transcript().slot(),
            Slot::Held {
                text: "two".into(),
                reason: HoldReason::LoginRequired
            }
        );
        assert_eq!(agent.cmd(Command::Resend).reply, Some(Ok(())));
        assert_eq!(agent.last("session/prompt")["params"]["sessionId"], json!("s1"));
    }

    #[test]
    fn a_restart_at_a_prompts_demand_is_refused_when_the_agent_cant_continue() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        agent.feed(&initialized_with_methods(1, json!({})));
        agent.feed(&result(2, json!({"sessionId": "s1"})));
        let id = agent.turn("one");
        agent.feed(&error(id, AUTH_REQUIRED, "expired"));
        let phase = agent.s.phase().clone();
        let out = agent.step(Input::Restart);
        assert_eq!(out.reply, Some(Err(Refused::CannotContinue)));
        assert!(!out.hang_up && out.events.is_empty());
        assert_eq!(*agent.s.phase(), phase);
    }

    #[test]
    fn a_restart_while_logging_in_or_ready_is_refused() {
        let mut agent = Agent::needing_login();
        agent.login("oauth-personal");
        assert_eq!(
            agent.step(Input::Restart).reply,
            Some(Err(Refused::NotEnded))
        );
        let mut ready = Agent::ready_with_methods();
        assert_eq!(
            ready.step(Input::Restart).reply,
            Some(Err(Refused::NotEnded))
        );
    }

    /// A restart dropped before the agent's exit (the session closed, its
    /// client gone): the exit is stepped, and the agent ends as any does.
    #[test]
    fn an_exit_stepped_while_restarting_ends_the_agent() {
        let mut agent = Agent::ready_with_methods();
        let id = agent.turn("one");
        agent.feed(&error(id, AUTH_REQUIRED, "expired"));
        agent.step(Input::Restart);
        agent.exit();
        assert_eq!(*agent.s.phase(), Phase::Exited);
        assert!(matches!(
            agent.agent_state(),
            AgentState::Exited { code: Some(1), .. }
        ));
        assert!(agent.s.continuable(), "its conversation can be continued");
    }
}
