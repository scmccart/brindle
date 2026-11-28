//! The agent's phase and the session's life: whether the agent may start,
//! its state as the zone shows it, the failure alert and the prompt input's
//! status.

use crate::agent::shared::StatusFrom;
use crate::agent::transcript::{AgentState, Slot, TurnState};

use super::facts::Rows;
use super::{AGENT_PROTOCOL, Absorbed, ChatModel, Due, RequestId, SessionHandle, running};
use super::ledger::{Sent, Taken};
use super::queue::{CHOOSING, EXITED, LOGGING_IN, NEEDS_LOGIN};

/// Why an agent session never started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartFailure {
    /// No terminal server could be reached or started: the cause.
    NoServer(String),
    /// The endpoint's terminal server speaks client protocols below 4.
    TooOld { endpoint: String },
    /// The server refused the spawn: the command and the server's error
    /// verbatim (on Windows a batch file's hint included).
    Refused { command: String, cause: String },
}

/// For logs and the client's error chain; the zone words it with
/// [`failure_text`].
impl std::fmt::Display for StartFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoServer(cause) => write!(f, "no terminal server: {cause}"),
            Self::TooOld { endpoint } => f.write_str(&too_old_text(endpoint)),
            Self::Refused { command, cause } => write!(f, "the agent {command} didn't start: {cause}"),
        }
    }
}

impl std::error::Error for StartFailure {}

/// Whether a connection that agreed on client `protocol` can start an
/// agent session on `endpoint`: only protocol 4 and up carry
/// `SPAWN_AGENT`, so an older server is refused before anything is sent.
pub fn agent_start(protocol: u32, endpoint: &str) -> Result<(), StartFailure> {
    if protocol >= AGENT_PROTOCOL {
        Ok(())
    } else {
        Err(StartFailure::TooOld { endpoint: endpoint.to_string() })
    }
}

/// The words for a server too old for agent sessions.
pub fn too_old_text(endpoint: &str) -> String {
    format!("the terminal server on {endpoint} is too old for agent sessions; it will update once its terminals have ended")
}

/// Where the zone's agent is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentPhase {
    /// The session is being started or attached; nothing folded yet.
    Connecting,
    /// The session never started.
    NotStarted(StartFailure),
    /// The agent's state, as the last `Agent` event folded says.
    Live(AgentState),
    /// The session can't be got back.
    Lost { reason: String },
    /// Brindle can't follow the session any more; it was detached.
    Broken { reason: String },
}

/// The connection to the session's endpoint, beside the phase: a reconnect
/// keeps the conversation shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Link {
    Up,
    Reconnecting,
}

/// Whether the session still runs on its server, beside the phase: it
/// ends on the session's exit (an agent that exited, or one hung up for
/// its ACP version, whose last state stays `Failed`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionLife {
    Open,
    Ended,
}

/// What a zone that can't take prompts says above its input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Alert {
    pub text: String,
    /// Drawn as a destructive alert (else the default one).
    pub destructive: bool,
    /// The agent's own words (its message, or the end of its standard
    /// error), shown under the text.
    pub detail: Option<String>,
    /// A command to show on a line of its own in the monospace font (a
    /// Windows batch file's `cmd /c …` form).
    pub mono: Option<String>,
}

/// What the alert names: the agent (its name, else its profile's), the
/// profile's command and the endpoint.
#[derive(Debug, Clone, Copy)]
pub struct Names<'a> {
    pub agent: &'a str,
    pub command: &'a str,
    pub endpoint: &'a str,
}

/// The `cmd /c …` form a Windows batch file's spawn error suggests, from
/// the server's words (`… run it as \`cmd /c npx …\``).
fn batch_form(cause: &str) -> Option<String> {
    let rest = &cause[cause.find("`cmd /c ")? + 1..];
    Some(rest[..rest.find('`')?].to_string())
}

/// The alert for `phase` and the session's `life`, or none while the zone
/// works (design.md decision 13): the action, then the cause. The two
/// `Failed` rows differ only by `life`: ended, it is a version mismatch's,
/// never worded as an exit. A login is done from the zone when `logins`
/// (a method Brindle can run is offered, OpenSpec change `agents-login`
/// decision 6), else through the agent's own command line.
pub fn failure_text(phase: &AgentPhase, life: SessionLife, names: Names<'_>, logins: bool) -> Option<Alert> {
    let alert = |text: String, destructive, detail: Option<&str>| Alert {
        text,
        destructive,
        detail: detail.map(str::to_string).filter(|d| !d.is_empty()),
        mono: None,
    };
    let agent = names.agent;
    match phase {
        AgentPhase::Connecting => None,
        AgentPhase::NotStarted(StartFailure::NoServer(cause)) => {
            Some(alert(format!("Brindle could not start the agent {}: {cause}", names.command), true, None))
        }
        AgentPhase::NotStarted(StartFailure::Refused { command, cause }) => Some(Alert {
            mono: batch_form(cause),
            ..alert(format!("Brindle could not start the agent {command}: {cause}"), true, None)
        }),
        AgentPhase::NotStarted(StartFailure::TooOld { endpoint }) => Some(alert(too_old_text(endpoint), true, None)),
        // A login running says so on its row, not in an alert.
        AgentPhase::Live(
            AgentState::Starting | AgentState::Ready { .. } | AgentState::LoggingIn { .. } | AgentState::Choosing { .. },
        ) => match life {
            SessionLife::Open => None,
            SessionLife::Ended => {
                Some(alert(format!("the agent session on {} has ended", names.endpoint), true, None))
            }
        },
        AgentPhase::Live(AgentState::LoginRequired { message, .. }) if logins => {
            Some(alert(format!("{agent} needs you to log in."), false, Some(message)))
        }
        AgentPhase::Live(AgentState::LoginRequired { message, .. }) => Some(alert(
            format!(
                "{agent} needs you to log in. Log in through the agent's own command line, then open the profile again."
            ),
            false,
            Some(message),
        )),
        AgentPhase::Live(AgentState::Failed { message }) => match life {
            SessionLife::Open => {
                Some(alert(format!("Brindle's terminal server failed handling {agent}: {message}"), true, None))
            }
            SessionLife::Ended => Some(alert(format!("{agent} can't be used: {message}"), true, None)),
        },
        AgentPhase::Live(AgentState::Exited { code, stderr }) => {
            let text = match code {
                Some(code) => format!("{agent} exited with status {code}"),
                None => format!("{agent} was ended by a signal"),
            };
            Some(alert(text, true, Some(stderr)))
        }
        AgentPhase::Live(AgentState::Lost { message }) => {
            Some(alert(format!("{agent} was lost with its terminal server's holder"), true, Some(message)))
        }
        AgentPhase::Lost { reason } => {
            Some(alert(format!("the agent session on {} was lost: {reason}", names.endpoint), true, None))
        }
        AgentPhase::Broken { reason } => Some(alert(
            "Brindle can't show this agent session any more; attach it again from Attach Session…".into(),
            true,
            Some(reason),
        )),
    }
}

/// Why the prompt input takes nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DisabledWhy {
    Connecting,
    NotStarted,
    Starting,
    /// The agent waits for a conversation to be chosen (OpenSpec change
    /// `agents-session-list`).
    Choosing,
    /// The agent needs a login, which the zone's login row offers.
    LoginRequired,
    /// The agent needs a login the zone can't run: it offers no method
    /// Brindle can (OpenSpec change `agents-login` decision 6).
    LoginElsewhere,
    LoggingIn,
    Failed,
    Exited,
    /// The agent was lost with the holder that ran it (`AgentState::Lost`).
    HolderLost,
    Lost,
    Broken,
    Reconnecting,
}

impl DisabledWhy {
    /// The prompt input's reason line.
    pub fn text(self) -> &'static str {
        match self {
            Self::Connecting => "connecting to the terminal server",
            Self::NotStarted => "the agent didn't start",
            Self::Starting => "the agent is starting",
            Self::Choosing => CHOOSING,
            Self::LoginRequired => NEEDS_LOGIN,
            Self::LoginElsewhere => {
                "the agent needs a login; log in through its own command line, then open the profile again"
            }
            Self::LoggingIn => LOGGING_IN,
            Self::Failed => "the agent can't be used",
            Self::Exited => EXITED,
            Self::HolderLost => HOLDER_LOST,
            Self::Lost => "the agent session was lost",
            Self::Broken => "Brindle can't show this agent session any more",
            Self::Reconnecting => "reconnecting to the terminal server",
        }
    }
}

pub(super) const HOLDER_LOST: &str = "the agent was lost with its holder";

/// What the prompt input's submit button is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptStatus {
    /// Send (disabled by the component while the text is blank).
    Ready,
    /// A prompt is on its way.
    Submitted,
    /// A turn runs: the button stops it.
    Streaming,
    Disabled(DisabledWhy),
}

impl PromptStatus {
    pub(super) fn word(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Submitted => "submitted",
            Self::Streaming => "streaming",
            Self::Disabled(_) => "disabled",
        }
    }
}

impl ChatModel {
    /// The session's start failed.
    pub fn not_started(&mut self, failure: StartFailure) -> Absorbed {
        let absorbed = self.life_event(|model| model.phase = AgentPhase::NotStarted(failure));
        self.nothing_will_replay(absorbed)
    }

    /// Nothing will replay the session now: a prompt whose answer was lost
    /// with the link can't be told taken or not.
    fn nothing_will_replay(&mut self, absorbed: Absorbed) -> Absorbed {
        let replied = self.uncertain.take().map(|uncertain| self.settle_uncertain(uncertain, Some(Taken::CantTell)));
        Absorbed { replied, ..absorbed }
    }

    /// The session ended on its server (`EXIT`): the agent state shown
    /// stays the last folded.
    pub fn ended(&mut self) -> Absorbed {
        self.life_event(|model| model.life = SessionLife::Ended)
    }

    /// The session can't be got back.
    pub fn lost(&mut self, reason: impl Into<String>) -> Absorbed {
        let reason = reason.into();
        let absorbed = self.life_event(|model| model.phase = AgentPhase::Lost { reason });
        self.nothing_will_replay(absorbed)
    }

    /// Brindle can't follow the session any more.
    pub fn broken(&mut self, reason: impl Into<String>) -> Absorbed {
        let reason = reason.into();
        let absorbed = self.life_event(|model| model.phase = AgentPhase::Broken { reason });
        self.nothing_will_replay(absorbed)
    }

    /// An `AGENT_RESTART` went out as `request` (an [`Offer::Restart`]): the
    /// zone says the agent is starting until the server's events say more,
    /// and shows the ended agent again if it is refused.
    pub fn restarting(&mut self, request: RequestId) -> Absorbed {
        let was = match &self.phase {
            AgentPhase::Live(state) => state.clone(),
            AgentPhase::Connecting | AgentPhase::NotStarted(_) | AgentPhase::Lost { .. } | AgentPhase::Broken { .. } => {
                return Absorbed::default();
            }
        };
        self.pending.insert(request, Sent::Restart { was });
        self.life_event(|model| {
            model.refusal = None;
            model.phase = AgentPhase::Live(AgentState::Starting);
        })
    }

    /// The zone continues its conversation in a new session (an
    /// [`Offer::Respawn`]): it connects again, keeping the conversation it
    /// shows until the new session's first events replace it
    /// ([`crate::agent::shared::AgentShared::start_over`]). The old
    /// session's requests, slot and uncertain prompt went with it, and so
    /// did the rows' and thoughts' toggles: the new log's entries take the
    /// same ids (decision 9). The new session isn't running until its
    /// thread says so.
    pub fn respawning(&mut self) -> Absorbed {
        let before = self.before();
        // What the updates marked is drawn first (OpenSpec change
        // `agents-absorb-cost`, decision 4).
        let mut splices = self.draw(Due::All);
        {
            let model = &mut *self;
            model.phase = AgentPhase::Connecting;
            model.life = SessionLife::Open;
            model.link = Link::Up;
            model.turn = TurnState::Idle;
            model.slot = Slot::Empty;
            model.pending.clear();
            model.refusal = None;
            model.handle = SessionHandle::Starting;
            model.uncertain = None;
            // A new conversation reuses the old one's ids (decision 9).
            model.toggled.clear();
            model.by_hand.clear();
            model.auto_open = None;
        }
        // The idle turn finishes the tool calls that ran, and the old
        // conversation's thoughts, still shown, are drawn as the policy
        // says, as its rows are; the working row's splice comes last.
        splices.extend(self.refresh_rows(Rows::All));
        splices.extend(self.update_thoughts(true));
        splices.extend(self.working());
        self.finish(before, splices, StatusFrom::Unchanged)
    }

    /// What the prompt input's button is, and whether it takes prompts.
    pub fn prompt_status(&self) -> PromptStatus {
        let disabled = |why| PromptStatus::Disabled(why);
        match &self.phase {
            AgentPhase::Connecting => disabled(DisabledWhy::Connecting),
            AgentPhase::NotStarted(_) => disabled(DisabledWhy::NotStarted),
            AgentPhase::Lost { .. } => disabled(DisabledWhy::Lost),
            AgentPhase::Broken { .. } => disabled(DisabledWhy::Broken),
            AgentPhase::Live(AgentState::Starting) => disabled(DisabledWhy::Starting),
            AgentPhase::Live(AgentState::Choosing { .. }) => disabled(DisabledWhy::Choosing),
            AgentPhase::Live(AgentState::LoggingIn { .. }) => disabled(DisabledWhy::LoggingIn),
            AgentPhase::Live(AgentState::LoginRequired { .. }) if self.terminal_login.is_some() => {
                disabled(DisabledWhy::LoggingIn)
            }
            AgentPhase::Live(AgentState::LoginRequired { .. }) if self.logins_usable() => {
                disabled(DisabledWhy::LoginRequired)
            }
            AgentPhase::Live(AgentState::LoginRequired { .. }) => disabled(DisabledWhy::LoginElsewhere),
            AgentPhase::Live(AgentState::Failed { .. }) => disabled(DisabledWhy::Failed),
            AgentPhase::Live(AgentState::Exited { .. }) => disabled(DisabledWhy::Exited),
            AgentPhase::Live(AgentState::Lost { .. }) => disabled(DisabledWhy::HolderLost),
            AgentPhase::Live(AgentState::Ready { .. }) => match (self.life, self.link) {
                (SessionLife::Ended, _) => disabled(DisabledWhy::Exited),
                (SessionLife::Open, Link::Reconnecting) => disabled(DisabledWhy::Reconnecting),
                // Ready, but the session isn't handed over yet (decision 2).
                (SessionLife::Open, Link::Up) if self.handle == SessionHandle::Starting => {
                    disabled(DisabledWhy::Connecting)
                }
                (SessionLife::Open, Link::Up) if self.in_flight() => PromptStatus::Submitted,
                (SessionLife::Open, Link::Up) if running(&self.turn) => PromptStatus::Streaming,
                (SessionLife::Open, Link::Up) => PromptStatus::Ready,
            },
        }
    }

    /// The alert for the zone's phase, if any ([`failure_text`]).
    pub fn alert(&self, command: &str, endpoint: &str) -> Option<Alert> {
        let agent = self.agent_label();
        failure_text(&self.phase, self.life, Names { agent: &agent, command, endpoint }, self.logins_usable())
    }
}

#[cfg(test)]
mod tests {
    use crate::agent::transcript::NoticeLevel;

    use super::super::harness::*;
    use super::super::*;
    use super::*;

    #[test]
    fn agent_sessions_need_client_protocol_4() {
        assert_eq!(agent_start(4, "this machine"), Ok(()));
        assert_eq!(agent_start(5, "this machine"), Ok(()));
        for old in [1, 2, 3] {
            assert_eq!(agent_start(old, "A8"), Err(StartFailure::TooOld { endpoint: "A8".into() }), "protocol {old}");
        }
        assert_eq!(
            too_old_text("A8"),
            "the terminal server on A8 is too old for agent sessions; it will update once its terminals have ended"
        );
    }

    /// Decision 13: a version mismatch's `Failed`, then the session's exit,
    /// stays `Live(Failed)` and is never worded as an exit.
    #[test]
    fn a_version_mismatch_stays_failed_after_the_session_ends() {
        let mut zone = Zone::new(None);
        let message = "the agent supports ACP versions 2; Brindle speaks 1";
        zone.feed(&[
            agent_state(AgentState::Failed { message: message.into() }),
            added(1, Entry::Notice { level: NoticeLevel::Warning, text: "stderr tail".into() }),
        ]);
        zone.model.ended();
        assert_eq!(zone.model.phase(), &AgentPhase::Live(AgentState::Failed { message: message.into() }));
        assert_eq!(zone.model.life(), SessionLife::Ended);
        let alert = zone.model.alert("fake-acp", "this machine").unwrap();
        assert_eq!(alert.text, format!("Fake can't be used: {message}"));
        assert!(!alert.text.contains("exit"));
        let dump = zone.model.describe(1, true, &found(&zone.model), &linux);
        assert!(dump.starts_with(&format!("--- agent zone=1 state=failed message={message:?} turn=idle")), "{dump}");
        assert!(dump.contains(" session=ended continue=none:not-ended\n"), "{dump}");
        assert_eq!(zone.model.prompt_status(), PromptStatus::Disabled(DisabledWhy::Failed));
    }

    /// Decision 13: each failure's alert, the action then the cause.
    #[test]
    fn each_failure_has_its_alert() {
        let names = Names { agent: "Fake Agent", command: "fake-acp", endpoint: "A8" };
        let text = |phase: AgentPhase, life| failure_text(&phase, life, names, false).map(|alert| alert.text);
        assert_eq!(text(AgentPhase::Connecting, SessionLife::Open), None);
        assert_eq!(text(AgentPhase::Live(AgentState::Ready { name: None, version: None }), SessionLife::Open), None);
        assert_eq!(text(AgentPhase::Live(AgentState::Starting), SessionLife::Open), None);
        assert_eq!(
            text(AgentPhase::NotStarted(StartFailure::NoServer("no helper".into())), SessionLife::Open).unwrap(),
            "Brindle could not start the agent fake-acp: no helper"
        );
        assert_eq!(
            text(AgentPhase::NotStarted(StartFailure::TooOld { endpoint: "A8".into() }), SessionLife::Open).unwrap(),
            too_old_text("A8")
        );
        assert_eq!(
            text(AgentPhase::Live(AgentState::Failed { message: "bad".into() }), SessionLife::Open).unwrap(),
            "Brindle's terminal server failed handling Fake Agent: bad"
        );
        let exited = failure_text(
            &AgentPhase::Live(AgentState::Exited { code: Some(3), stderr: "boom".into() }),
            SessionLife::Ended,
            names,
            false,
        )
        .unwrap();
        assert_eq!((exited.text.as_str(), exited.detail.as_deref()), ("Fake Agent exited with status 3", Some("boom")));
        assert_eq!(
            text(AgentPhase::Live(AgentState::Exited { code: None, stderr: String::new() }), SessionLife::Ended).unwrap(),
            "Fake Agent was ended by a signal"
        );
        assert_eq!(
            text(AgentPhase::Lost { reason: "the holder crashed".into() }, SessionLife::Open).unwrap(),
            "the agent session on A8 was lost: the holder crashed"
        );
        assert_eq!(
            text(AgentPhase::Broken { reason: "x".into() }, SessionLife::Open).unwrap(),
            "Brindle can't show this agent session any more; attach it again from Attach Session…"
        );
        assert_eq!(
            text(AgentPhase::Live(AgentState::Ready { name: None, version: None }), SessionLife::Ended).unwrap(),
            "the agent session on A8 has ended"
        );

        let cause = "npx is a batch file (C:/npx.cmd); run it as `cmd /c npx acp`";
        let refused = StartFailure::Refused { command: "npx".into(), cause: cause.into() };
        let alert = failure_text(&AgentPhase::NotStarted(refused), SessionLife::Open, names, false).unwrap();
        assert_eq!(alert.text, format!("Brindle could not start the agent npx: {cause}"));
        assert_eq!(alert.mono.as_deref(), Some("cmd /c npx acp"));
        assert!(alert.destructive);
        let plain = StartFailure::Refused { command: "fake".into(), cause: "not found".into() };
        assert_eq!(failure_text(&AgentPhase::NotStarted(plain), SessionLife::Open, names, false).unwrap().mono, None);

        // `agents-login` decision 6: the two login texts.
        let login = AgentPhase::Live(AgentState::LoginRequired { message: "run agent login".into(), methods: Vec::new() });
        let elsewhere = failure_text(&login, SessionLife::Open, names, false).unwrap();
        assert_eq!(
            elsewhere.text,
            "Fake Agent needs you to log in. Log in through the agent's own command line, then open the profile again."
        );
        assert_eq!(elsewhere.detail.as_deref(), Some("run agent login"));
        assert!(!elsewhere.destructive);
        let here = failure_text(&login, SessionLife::Open, names, true).unwrap();
        assert_eq!((here.text.as_str(), here.detail.as_deref()), ("Fake Agent needs you to log in.", Some("run agent login")));
        let logging_in = AgentPhase::Live(AgentState::LoggingIn { method: "g".into(), name: "Google".into() });
        assert_eq!(failure_text(&logging_in, SessionLife::Open, names, true), None);
    }
}
