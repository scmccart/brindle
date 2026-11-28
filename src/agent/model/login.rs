//! The login row (OpenSpec change `agents-login`): the methods offered, a
//! choice, Cancel login and a terminal login zone's end.

use gpui::SharedString;

use crate::agent::login::{self, MethodOffer, TerminalLogin};
use crate::agent::transcript::AgentState;

use super::ChatModel;
use super::ledger::Sent;
use super::phase::{AgentPhase, SessionLife};

/// The login row above the prompt input (OpenSpec change `agents-login`,
/// design decision 6), from the agent's phase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginRow {
    None,
    /// The agent's message, then one choice per method, in its order.
    Choose { message: SharedString, methods: Vec<MethodChoice> },
    /// A login runs: the agent's (`AgentState::LoggingIn`), or a terminal
    /// login's program in the zone beside (`terminal`).
    Running { method: SharedString, name: SharedString, terminal: bool },
}

/// One of the login row's choices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MethodChoice {
    pub id: SharedString,
    pub name: SharedString,
    pub description: Option<SharedString>,
    pub offer: MethodOffer,
}

/// What choosing a login method does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoginChoice {
    /// Send `AGENT_LOGIN` for this method.
    Agent(String),
    /// Open a terminal zone running this beside the agent zone.
    Terminal(TerminalLogin),
    /// Nothing: no such usable method, a login request on its way, or
    /// nothing can be sent now.
    Nothing,
}

/// What Cancel login does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancelLogin {
    /// Send `AGENT_CANCEL`: the agent's login stops.
    Send,
    /// Close the terminal login's zone.
    CloseZone,
    Nothing,
}

/// How a terminal login's zone went.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginZoneEnd {
    /// Its program exited, with its status (`None`: ended by a signal).
    Exited { code: Option<i32> },
    /// The user closed it: a cancel.
    Closed,
}

/// What a terminal login's end asks of the zone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterLogin {
    /// It succeeded: start the agent again (`AGENT_RESTART`).
    Restart,
    Nothing,
}

impl ChatModel {
    /// Whether the agent needs a login, or is logging in, on an open
    /// session.
    pub(super) fn needs_login(&self) -> bool {
        let phase = match &self.phase {
            AgentPhase::Live(AgentState::LoginRequired { .. } | AgentState::LoggingIn { .. }) => true,
            AgentPhase::Live(
                AgentState::Starting
                | AgentState::Ready { .. }
                | AgentState::Failed { .. }
                | AgentState::Exited { .. }
                | AgentState::Lost { .. }
                | AgentState::Choosing { .. },
            )
            | AgentPhase::Connecting
            | AgentPhase::NotStarted(_)
            | AgentPhase::Lost { .. }
            | AgentPhase::Broken { .. } => false,
        };
        phase && self.life == SessionLife::Open
    }

    /// The login row for the agent's phase (decision 6): the methods in
    /// the agent's order while it needs a login and offers some, a running
    /// login's method while one runs.
    pub fn login_row(&self) -> LoginRow {
        if !self.needs_login() {
            return LoginRow::None;
        }
        if let Some((id, name)) = &self.terminal_login {
            return LoginRow::Running { method: id.clone().into(), name: name.clone().into(), terminal: true };
        }
        match &self.phase {
            AgentPhase::Live(AgentState::LoggingIn { method, name }) => {
                LoginRow::Running { method: method.clone().into(), name: name.clone().into(), terminal: false }
            }
            AgentPhase::Live(AgentState::LoginRequired { message, methods }) if !methods.is_empty() => LoginRow::Choose {
                message: message.clone().into(),
                methods: methods
                    .iter()
                    .map(|method| MethodChoice {
                        id: method.id.clone().into(),
                        name: method.name.clone().into(),
                        description: method.description.clone().map(SharedString::from),
                        offer: login::offer(&self.command, method),
                    })
                    .collect(),
            },
            AgentPhase::Live(
                AgentState::LoginRequired { .. }
                | AgentState::Starting
                | AgentState::Ready { .. }
                | AgentState::Failed { .. }
                | AgentState::Exited { .. }
                | AgentState::Lost { .. }
                | AgentState::Choosing { .. },
            )
            | AgentPhase::Connecting
            | AgentPhase::NotStarted(_)
            | AgentPhase::Lost { .. }
            | AgentPhase::Broken { .. } => LoginRow::None,
        }
    }

    /// Whether the agent needs a login it offers a method for that Brindle
    /// can run.
    pub(super) fn logins_usable(&self) -> bool {
        match &self.phase {
            AgentPhase::Live(AgentState::LoginRequired { methods, .. }) => {
                methods.iter().any(|method| login::offer(&self.command, method).usable())
            }
            AgentPhase::Live(
                AgentState::LoggingIn { .. }
                | AgentState::Starting
                | AgentState::Ready { .. }
                | AgentState::Failed { .. }
                | AgentState::Exited { .. }
                | AgentState::Lost { .. }
                | AgentState::Choosing { .. },
            )
            | AgentPhase::Connecting
            | AgentPhase::NotStarted(_)
            | AgentPhase::Lost { .. }
            | AgentPhase::Broken { .. } => false,
        }
    }

    /// Whether a login runs: the agent's, or a terminal login's program.
    pub(super) fn logging_in(&self) -> bool {
        self.terminal_login.is_some() || matches!(self.phase, AgentPhase::Live(AgentState::LoggingIn { .. }))
    }

    /// What beside the login row says: a refused `AGENT_LOGIN`, or how a
    /// terminal login's program ended.
    pub fn login_note(&self) -> Option<&str> {
        self.login_note.as_deref()
    }

    /// The login row's method `id` was chosen: what to do, decided once (a
    /// press while a login request is on its way does nothing).
    pub fn choose_login(&mut self, id: &str) -> LoginChoice {
        let LoginRow::Choose { methods, .. } = self.login_row() else { return LoginChoice::Nothing };
        let Some(choice) = methods.into_iter().find(|choice| choice.id.as_ref() == id) else {
            return LoginChoice::Nothing;
        };
        let asked = self.pending.values().any(|sent| *sent == Sent::Login);
        if asked || !self.can_send() {
            return LoginChoice::Nothing;
        }
        match choice.offer {
            MethodOffer::Agent => {
                self.login_note = None;
                LoginChoice::Agent(id.to_string())
            }
            MethodOffer::Terminal(plan) => {
                self.login_note = None;
                self.terminal_login = Some((id.to_string(), choice.name.to_string()));
                LoginChoice::Terminal(plan)
            }
            MethodOffer::Withheld(_) => LoginChoice::Nothing,
        }
    }

    /// Cancel login: the agent's login is cancelled, a terminal login's
    /// zone closed.
    pub fn cancel_login(&mut self) -> CancelLogin {
        if self.terminal_login.take().is_some() {
            return CancelLogin::CloseZone;
        }
        match &self.phase {
            AgentPhase::Live(AgentState::LoggingIn { .. }) if self.can_send() => CancelLogin::Send,
            AgentPhase::Live(_)
            | AgentPhase::Connecting
            | AgentPhase::NotStarted(_)
            | AgentPhase::Lost { .. }
            | AgentPhase::Broken { .. } => CancelLogin::Nothing,
        }
    }

    /// The terminal login's zone ended as `end`: its program's status 0
    /// starts the agent again; another status, or a signal, shows the
    /// methods again saying so; a close is a cancel.
    pub fn login_zone_ended(&mut self, end: LoginZoneEnd) -> AfterLogin {
        if self.terminal_login.take().is_none() || !self.needs_login() {
            return AfterLogin::Nothing;
        }
        match end {
            LoginZoneEnd::Exited { code: Some(0) } => return AfterLogin::Restart,
            LoginZoneEnd::Exited { code: Some(code) } => {
                self.login_note = Some(format!("the login program exited with status {code}"));
            }
            LoginZoneEnd::Exited { code: None } => {
                self.login_note = Some("the login program was ended by a signal".into());
            }
            LoginZoneEnd::Closed => {}
        }
        AfterLogin::Nothing
    }
}

#[cfg(test)]
mod tests {
    use crate::agent::close::AgentBusy;
    use crate::agent::transcript::{Event, HoldReason};

    use super::super::harness::*;
    use super::super::*;
    use super::*;

    /// Decision 9 and `agents-sessions` decision 17: a login demanded by a
    /// prompt.
    #[test]
    fn a_login_demanded_by_a_prompt() {
        let mut zone = Zone::ready();
        zone.feed(&[added(1, user("hi")), turn(TurnState::Working), added(2, agent("Sure")), queue(waiting())]);
        let message = "Authentication required";
        let absorbed = zone.feed(&[
            queue(held(HoldReason::LoginRequired)),
            turn(TurnState::Failed { message: message.into() }),
            agent_state(AgentState::LoginRequired { message: message.into(), methods: Vec::new() }),
        ]);
        assert_eq!(absorbed.status, Some(Some(State::Error)));
        assert_eq!(zone.model.turn(), &TurnState::Failed { message: message.into() });
        let QueueRow::Held { why, resend, .. } = zone.model.queue() else { panic!("{:?}", zone.model.queue()) };
        assert_eq!(why.as_ref(), "the agent needs a login");
        assert_eq!(resend, &Resend::Unavailable("the agent needs a login".into()));
        assert!(!zone.model.resend());
        assert_eq!(zone.model.unqueue(), Unqueue::Send, "discard still works");
        // No method offered: the login is the agent's own command line's.
        assert_eq!(zone.model.prompt_status(), PromptStatus::Disabled(DisabledWhy::LoginElsewhere));
        assert_eq!(
            DisabledWhy::LoginElsewhere.text(),
            "the agent needs a login; log in through its own command line, then open the profile again"
        );
        assert_eq!(DisabledWhy::LoginRequired.text(), "the agent needs a login");
        assert_eq!(DisabledWhy::LoggingIn.text(), "the agent is logging in");
        assert_eq!(zone.model.login_row(), LoginRow::None);
        assert_eq!(texts(&zone.model), ["hi", "Sure"], "earlier entries kept");
        let alert = zone.model.alert("fake-acp", "this machine").unwrap();
        assert_eq!(
            alert.text,
            "Fake Agent needs you to log in. Log in through the agent's own command line, then open the profile again."
        );
        assert_eq!((alert.destructive, alert.detail.as_deref()), (false, Some(message)));
        assert!(zone.model.describe(1, true, &found(&zone.model), &linux).contains("--- queued held reason=login-required resend=no \"next\"\n"));
    }

    // OpenSpec change `agents-login`, sections 6.1–6.3.

    fn agent_method(id: &str, name: &str, description: Option<&str>) -> crate::agent::transcript::LoginMethod {
        crate::agent::transcript::LoginMethod {
            id: id.into(),
            name: name.into(),
            description: description.map(str::to_string),
            kind: crate::agent::transcript::LoginKind::Agent,
        }
    }

    fn terminal_method(id: &str, name: &str, env: &[(&str, &str)]) -> crate::agent::transcript::LoginMethod {
        crate::agent::transcript::LoginMethod {
            id: id.into(),
            name: name.into(),
            description: None,
            kind: crate::agent::transcript::LoginKind::Terminal {
                args: vec!["login".into()],
                env: env.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect(),
            },
        }
    }

    fn two_methods() -> Vec<crate::agent::transcript::LoginMethod> {
        vec![agent_method("google", "Log in with Google", None), agent_method("api-key", "API Key", Some("Use an API key"))]
    }

    fn login_required(methods: Vec<crate::agent::transcript::LoginMethod>) -> Event {
        agent_state(AgentState::LoginRequired { message: "run agent login".into(), methods })
    }

    fn logging_in(method: &str, name: &str) -> Event {
        agent_state(AgentState::LoggingIn { method: method.into(), name: name.into() })
    }

    /// Decision 6: one choice per method in the agent's order, with its
    /// description, nothing chosen; a choice sends `AGENT_LOGIN` once, and
    /// a press while it is on its way does nothing; a refusal is noted
    /// beside the row; Cancel login sends `AGENT_CANCEL` while logging in.
    #[test]
    fn the_login_row_offers_the_methods_in_order_and_chooses_once() {
        let mut zone = Zone::new(None);
        let absorbed = zone.feed(&[agent_state(AgentState::Starting), login_required(two_methods())]);
        assert_eq!(absorbed.status, Some(Some(State::Error)));
        let LoginRow::Choose { message, methods } = zone.model.login_row() else { panic!("{:?}", zone.model.login_row()) };
        assert_eq!(message.as_ref(), "run agent login");
        let shown: Vec<_> =
            methods.iter().map(|c| (c.id.to_string(), c.name.to_string(), c.description.as_ref().map(|d| d.to_string()))).collect();
        assert_eq!(
            shown,
            [
                ("google".to_string(), "Log in with Google".to_string(), None),
                ("api-key".to_string(), "API Key".to_string(), Some("Use an API key".to_string())),
            ]
        );
        assert!(methods.iter().all(|c| c.offer == MethodOffer::Agent));
        assert_eq!(zone.model.prompt_status(), PromptStatus::Disabled(DisabledWhy::LoginRequired));
        assert_eq!(zone.model.alert("fake-acp", "here").unwrap().text, "Fake needs you to log in.");
        assert_eq!(zone.model.close_busy(), None, "an error doesn't ask");

        assert_eq!(zone.model.choose_login("nope"), LoginChoice::Nothing);
        assert_eq!(zone.model.choose_login("google"), LoginChoice::Agent("google".into()));
        zone.model.sent(4, Sent::Login);
        assert_eq!(zone.model.choose_login("api-key"), LoginChoice::Nothing, "a login request is on its way");
        assert_eq!(zone.model.replied(4, Err("the agent offers no login method \"google\"".into())), Replied::Nothing);
        assert_eq!(zone.model.login_note(), Some("the agent offers no login method \"google\""));
        let dump = zone.model.wait_text(&found(&zone.model), &linux);
        assert!(
            dump.contains(
                "--- login choice method=\"google\" kind=agent \"Log in with Google\"\n--- login choice method=\"api-key\" kind=agent \"API Key\" description=\"Use an API key\" refusal=\"the agent offers no login method \\\"google\\\"\"\n"
            ),
            "{dump}"
        );
        assert_eq!(zone.model.choose_login("google"), LoginChoice::Agent("google".into()), "chosen again");
        assert_eq!(zone.model.login_note(), None, "a new choice clears the note");
        zone.model.sent(5, Sent::Login);
        zone.model.replied(5, Ok(()));

        let absorbed = zone.feed(&[logging_in("google", "Log in with Google")]);
        assert_eq!(absorbed.status, Some(Some(State::Working)), "logging in is working");
        assert_eq!(
            zone.model.login_row(),
            LoginRow::Running { method: "google".into(), name: "Log in with Google".into(), terminal: false }
        );
        assert_eq!(zone.model.prompt_status(), PromptStatus::Disabled(DisabledWhy::LoggingIn));
        assert_eq!(zone.model.alert("fake-acp", "here"), None);
        assert_eq!(zone.model.close_busy(), None, "a login running never asks");
        assert!(!zone.model.cancel(), "the turn's stop has no turn to stop");
        let dump = zone.model.wait_text(&found(&zone.model), &linux);
        assert!(dump.contains("state=logging-in "), "{dump}");
        assert!(dump.contains("--- login running method=\"google\" \"Log in with Google\"\n"), "{dump}");
        assert_eq!(zone.model.cancel_login(), CancelLogin::Send);
        zone.model.set_link(Link::Reconnecting);
        assert_eq!(zone.model.cancel_login(), CancelLogin::Nothing, "nothing is sent while reconnecting");
        zone.model.set_link(Link::Up);

        zone.feed(&[agent_state(AgentState::Starting), ready()]);
        assert_eq!(zone.model.login_row(), LoginRow::None);
        assert_eq!(zone.model.prompt_status(), PromptStatus::Ready);
        assert_eq!(zone.model.cancel_login(), CancelLogin::Nothing);
        assert_eq!(zone.model.choose_login("google"), LoginChoice::Nothing);
        assert!(!zone.model.wait_text(&found(&zone.model), &linux).contains("--- login"));
    }

    /// Decision 7: a busy agent still asks before closing, a logging-in
    /// one doesn't.
    #[test]
    fn closing_asks_only_for_a_busy_agent_that_isnt_logging_in() {
        let mut zone = Zone::ready();
        zone.feed(&[turn(TurnState::Working)]);
        assert_eq!(zone.model.close_busy(), Some(AgentBusy::Working));
        zone.feed(&[turn(TurnState::Blocked)]);
        assert_eq!(zone.model.close_busy(), Some(AgentBusy::Blocked));
        let mut zone = Zone::new(None);
        zone.feed(&[agent_state(AgentState::Starting)]);
        assert_eq!(zone.model.close_busy(), Some(AgentBusy::Working), "starting is working");
        zone.feed(&[login_required(two_methods()), logging_in("google", "Log in with Google")]);
        assert_eq!(zone.model.shown_status(), Some(State::Working));
        assert_eq!(zone.model.close_busy(), None);
    }

    /// Decision 9: a terminal method's choice shows what it runs and
    /// leaves out; choosing it runs the plan beside, Cancel login closes
    /// its zone, its status 0 asks for the restart, another status shows
    /// the methods again with it.
    #[test]
    fn a_terminal_login_runs_beside_and_its_exit_decides() {
        let mut zone = Zone::new(None);
        zone.model.set_command(AgentCommand { program: "fake-acp".into(), args: vec!["--acp".into()], ignore_case: false });
        let methods = vec![terminal_method("cli", "Log in from the terminal", &[("LOGIN_MODE", "device"), ("PATH", "/tmp/evil")])];
        zone.feed(&[login_required(methods.clone())]);
        let dump = zone.model.wait_text(&found(&zone.model), &linux);
        assert!(
            dump.contains("--- login choice method=\"cli\" kind=terminal \"Log in from the terminal\" args=\"login\" env=\"LOGIN_MODE=device\" dropped=\"PATH\"\n"),
            "{dump}"
        );
        let LoginChoice::Terminal(plan) = zone.model.choose_login("cli") else { panic!("a terminal login") };
        assert_eq!((plan.command.as_str(), plan.args.as_slice()), ("fake-acp", &["--acp".to_string(), "login".to_string()][..]));
        assert_eq!(zone.model.login_row(), LoginRow::Running { method: "cli".into(), name: "Log in from the terminal".into(), terminal: true });
        assert_eq!(zone.model.prompt_status(), PromptStatus::Disabled(DisabledWhy::LoggingIn));
        assert_eq!(zone.model.close_busy(), None);
        assert_eq!(zone.model.choose_login("cli"), LoginChoice::Nothing, "one login at a time");

        assert_eq!(zone.model.cancel_login(), CancelLogin::CloseZone);
        assert!(matches!(zone.model.login_row(), LoginRow::Choose { .. }), "the methods again");
        assert_eq!(zone.model.login_zone_ended(LoginZoneEnd::Closed), AfterLogin::Nothing, "the close it asked for");

        assert!(matches!(zone.model.choose_login("cli"), LoginChoice::Terminal(_)));
        assert_eq!(zone.model.login_zone_ended(LoginZoneEnd::Exited { code: Some(1) }), AfterLogin::Nothing);
        assert_eq!(zone.model.login_note(), Some("the login program exited with status 1"));
        assert!(zone.model.wait_text(&found(&zone.model), &linux).contains(" refusal=\"the login program exited with status 1\"\n"));
        assert!(matches!(zone.model.choose_login("cli"), LoginChoice::Terminal(_)));
        assert_eq!(zone.model.login_zone_ended(LoginZoneEnd::Exited { code: None }), AfterLogin::Nothing);
        assert_eq!(zone.model.login_note(), Some("the login program was ended by a signal"));
        assert!(matches!(zone.model.choose_login("cli"), LoginChoice::Terminal(_)));
        assert_eq!(zone.model.login_zone_ended(LoginZoneEnd::Exited { code: Some(0) }), AfterLogin::Restart);
        assert_eq!(zone.model.login_zone_ended(LoginZoneEnd::Exited { code: Some(0) }), AfterLogin::Nothing, "once");

        // The restart shows the agent starting; the agent moving on drops
        // a login zone the zone still thought ran.
        zone.model.restarting(9);
        assert_eq!(zone.model.phase(), &AgentPhase::Live(AgentState::Starting));
        zone.feed(&[login_required(methods)]);
        assert!(matches!(zone.model.choose_login("cli"), LoginChoice::Terminal(_)));
        zone.feed(&[agent_state(AgentState::Exited { code: Some(1), stderr: String::new() })]);
        assert_eq!(zone.model.login_row(), LoginRow::None);
        assert_eq!(zone.model.login_zone_ended(LoginZoneEnd::Exited { code: Some(0) }), AfterLogin::Nothing);
    }

    /// Decision 9: a profile whose command re-parses its arguments shows
    /// its terminal method withheld, which can't be chosen, and the zone
    /// says the login is the agent's own command line's.
    #[test]
    fn a_withheld_method_is_shown_but_not_offered() {
        let mut zone = Zone::new(Some("hi"));
        zone.model.set_command(AgentCommand { program: "cmd".into(), args: vec!["/c".into(), "npx".into()], ignore_case: true });
        let absorbed = zone.feed(&[login_required(vec![terminal_method("cli", "Log in from the terminal", &[])])]);
        assert_eq!(absorbed.first, Some(First::Draft("hi".into())), "no usable method: a draft");
        let LoginRow::Choose { methods, .. } = zone.model.login_row() else { panic!("a row") };
        assert!(matches!(&methods[0].offer, MethodOffer::Withheld(w) if w.why == "the login can't be run through `cmd /c`"));
        assert_eq!(zone.model.choose_login("cli"), LoginChoice::Nothing);
        assert_eq!(zone.model.prompt_status(), PromptStatus::Disabled(DisabledWhy::LoginElsewhere));
        let dump = zone.model.wait_text(&found(&zone.model), &linux);
        assert!(dump.contains(" withheld=\"the login can't be run through `cmd /c`\"\n"), "{dump}");
    }

    /// Decision 10: the first prompt is shown while the agent needs a
    /// login it can run, kept through a failed and a cancelled login and
    /// while logging in, and sent once, if unchanged, on the first live
    /// `Ready`; never from a replay.
    #[test]
    fn the_first_prompt_waits_for_a_login_and_goes_once() {
        let mut zone = Zone::new(Some("hi"));
        let absorbed = zone.feed(&[agent_state(AgentState::Starting), login_required(two_methods())]);
        assert_eq!(absorbed.first, Some(First::Show("hi".into())));
        assert_eq!(zone.model.first_shown(), Some("hi"));
        assert!(zone.model.wait_text(&found(&zone.model), &linux).contains("--- prompt status=disabled first=\"hi\"\n"));
        // A failed login: back to the methods, still waiting, not shown again.
        assert_eq!(zone.feed(&[logging_in("google", "Google")]).first, None);
        let failed = AgentState::LoginRequired { message: "browser login timed out".into(), methods: two_methods() };
        assert_eq!(zone.feed(&[agent_state(failed)]).first, None);
        // A cancelled one, the same.
        assert_eq!(zone.feed(&[logging_in("google", "Google"), login_required(two_methods())]).first, None);
        assert_eq!(zone.model.first_shown(), Some("hi"));
        // Logged in: starting keeps it, ready sends it if unchanged.
        assert_eq!(zone.feed(&[logging_in("google", "Google"), agent_state(AgentState::Starting)]).first, None);
        assert_eq!(zone.feed(&[ready()]).first, Some(First::SendIfUnchanged("hi".into())));
        assert_eq!(zone.model.first_shown(), None);
        assert_eq!(zone.feed(&[login_required(two_methods()), ready()]).first, None, "sent once");

        // A login at start shown, then the agent fails: a plain draft.
        for end in [
            AgentState::Failed { message: "x".into() },
            AgentState::Exited { code: Some(1), stderr: String::new() },
            AgentState::Lost { message: "its holder ended".into() },
        ] {
            let mut zone = Zone::new(Some("hi"));
            assert_eq!(zone.feed(&[login_required(two_methods())]).first, Some(First::Show("hi".into())));
            assert_eq!(zone.feed(&[agent_state(end.clone())]).first, Some(First::Draft("hi".into())), "{end:?}");
        }
        let mut zone = Zone::new(Some("hi"));
        zone.feed(&[login_required(two_methods())]);
        assert_eq!(zone.model.lost("gone").first, Some(First::Draft("hi".into())));
        let mut zone = Zone::new(Some("hi"));
        zone.feed(&[login_required(two_methods())]);
        assert_eq!(zone.model.ended().first, Some(First::Draft("hi".into())));
        // Ready from a replay never sends it.
        let mut zone = Zone::new(Some("hi"));
        zone.feed(&[login_required(two_methods())]);
        assert_eq!(zone.replay(&[ready()]).first, Some(First::Draft("hi".into())));
        // No method at all: a draft, as before.
        let mut zone = Zone::new(Some("hi"));
        assert_eq!(zone.feed(&[login_required(Vec::new())]).first, Some(First::Draft("hi".into())));
        // No login: sent as ever.
        let mut zone = Zone::new(Some("hi"));
        assert_eq!(zone.feed(&[ready()]).first, Some(First::Send("hi".into())));
    }
}
