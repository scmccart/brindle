//! Continuing a conversation (OpenSpec change `agents-session-recovery`):
//! what Continue conversation would do, and whether the zone keeps its
//! session.

use crate::agent::transcript::{AgentState, Conversation};

use super::ChatModel;
use super::phase::{AgentPhase, SessionLife};

/// Whether the zone's agent profile can run a continue: found by the name
/// the session was started with, as an agent profile ([`crate::agent::spawn::continuing_profile`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProfileFound {
    Agent,
    /// No agent profile has the session's profile name now: deleted,
    /// renamed away, made a terminal profile, or the zone was attached
    /// under a substitute (`session-restore`'s "Agent profile gone").
    Gone { name: String },
}

/// What continuing the zone's conversation would do (OpenSpec change
/// `agents-session-recovery`, design decision 8).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Offer {
    /// `AGENT_RESTART` on the zone's session, which the server keeps.
    Restart,
    /// A new session continuing `conversation` (`SPAWN_AGENT` with it): the
    /// server no longer has the zone's.
    Respawn { conversation: Conversation },
    None(NoOffer),
}

/// Why there is nothing to continue.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoOffer {
    /// The agent runs, starts or failed, or the zone can't follow it.
    NotEnded,
    /// The agent never gave an ACP session.
    NoConversation,
    /// The agent offered neither `session/load` nor `session/resume`.
    NotOffered,
    /// The zone's agent profile is gone ([`ProfileFound::Gone`]).
    ProfileGone { name: String },
}

impl NoOffer {
    /// The prompt input's reason line for an ended agent with no offer.
    pub fn text(&self) -> String {
        match self {
            Self::NotEnded => "the agent hasn't ended".into(),
            Self::NoConversation => "the agent never started a conversation".into(),
            Self::NotOffered => "the agent can't continue conversations".into(),
            Self::ProfileGone { name } => format!("the profile {name} is gone"),
        }
    }

    /// The dump's word (`continue=none:WORD`).
    pub fn word(&self) -> &'static str {
        match self {
            Self::NotEnded => "not-ended",
            Self::NoConversation => "no-conversation",
            Self::NotOffered => "not-offered",
            Self::ProfileGone { .. } => "profile-gone",
        }
    }
}

impl Offer {
    /// The dump's `continue=` value: `restart`, `respawn` or `none:REASON`.
    pub fn word(&self) -> String {
        match self {
            Self::Restart => "restart".into(),
            Self::Respawn { .. } => "respawn".into(),
            Self::None(why) => format!("none:{}", why.word()),
        }
    }
}

/// What continuing the conversation would do for an agent in `phase` whose
/// session is `life`, with the zone's last known `conversation` and its
/// `profile`. An agent that exited or was lost with its holder, on a session
/// the server keeps, restarts there; a session the server no longer has
/// (lost, ended with `EXIT`, or a continue in a new session that didn't
/// start) continues in a new one. Either needs a conversation the agent
/// can take up again and the profile. Nothing else is offered: a running,
/// starting or failed agent (a refused continue ends `Failed`, which isn't
/// offered again), a zone not yet connected or broken.
pub fn continue_offer(
    phase: &AgentPhase,
    life: SessionLife,
    conversation: Option<&Conversation>,
    profile: &ProfileFound,
) -> Offer {
    let respawn = match (phase, life) {
        (AgentPhase::Live(AgentState::Exited { .. } | AgentState::Lost { .. }), SessionLife::Open) => false,
        (AgentPhase::Live(AgentState::Exited { .. } | AgentState::Lost { .. }), SessionLife::Ended)
        | (AgentPhase::Lost { .. } | AgentPhase::NotStarted(_), SessionLife::Open | SessionLife::Ended) => true,
        (
            AgentPhase::Connecting
            | AgentPhase::Broken { .. }
            | AgentPhase::Live(
                AgentState::Starting
                | AgentState::Ready { .. }
                | AgentState::LoginRequired { .. }
                | AgentState::LoggingIn { .. }
                | AgentState::Failed { .. }
                | AgentState::Choosing { .. },
            ),
            SessionLife::Open | SessionLife::Ended,
        ) => return Offer::None(NoOffer::NotEnded),
    };
    let Some(conversation) = conversation else { return Offer::None(NoOffer::NoConversation) };
    if !conversation.continuation.continuable() {
        return Offer::None(NoOffer::NotOffered);
    }
    match profile {
        ProfileFound::Gone { name } => Offer::None(NoOffer::ProfileGone { name: name.clone() }),
        ProfileFound::Agent if respawn => Offer::Respawn { conversation: conversation.clone() },
        ProfileFound::Agent => Offer::Restart,
    }
}

impl ChatModel {
    /// The agent's conversation as the zone last knew it.
    pub fn conversation(&self) -> Option<&Conversation> {
        self.conversation.as_ref()
    }

    /// What continuing the conversation would do now, with the zone's
    /// agent profile as `profile` ([`continue_offer`]).
    pub fn offer(&self, profile: &ProfileFound) -> Offer {
        continue_offer(&self.phase, self.life, self.conversation.as_ref(), profile)
    }

    /// Whether the zone's session is one a quit leaves for the next launch
    /// to attach (design.md decision 5's `zone_records`): not once it has
    /// ended, nor when it never started, was lost or can't be followed, nor
    /// while its agent has exited or was lost with its holder, though the
    /// server keeps that session for a continue (`agents-session-recovery`
    /// decision 10: the quit's `DETACH` ends it).
    pub fn keeps_session(&self) -> bool {
        let phase = match &self.phase {
            AgentPhase::Connecting
            | AgentPhase::Live(
                AgentState::Starting
                | AgentState::Ready { .. }
                | AgentState::LoginRequired { .. }
                | AgentState::LoggingIn { .. }
                | AgentState::Failed { .. },
            ) => !self.choosing.started,
            // Nothing to bring back: the session isn't recorded, and a quit
            // ends it (OpenSpec change `agents-session-list`).
            AgentPhase::Live(AgentState::Choosing { .. }) => false,
            AgentPhase::Live(AgentState::Exited { .. } | AgentState::Lost { .. })
            | AgentPhase::NotStarted(_)
            | AgentPhase::Lost { .. }
            | AgentPhase::Broken { .. } => false,
        };
        phase && self.life == SessionLife::Open
    }
}

#[cfg(test)]
mod tests {
    use crate::agent::shared::AgentShared;
    use crate::agent::transcript::{Event, HoldReason, NoticeLevel};

    use super::super::harness::*;
    use super::super::*;
    use super::*;

    /// Only a session that is open (or still connecting) is left for the
    /// next launch.
    #[test]
    fn a_zone_keeps_its_session_only_while_it_is_open() {
        let mut zone = Zone::new(None);
        assert!(zone.model.keeps_session(), "connecting");
        zone.feed(&[ready()]);
        assert!(zone.model.keeps_session());
        zone.model.ended();
        assert!(!zone.model.keeps_session(), "ended");
        let mut lost = Zone::ready();
        lost.model.lost("gone");
        assert!(!lost.model.keeps_session());
        let mut broken = Zone::ready();
        broken.model.broken("bad event");
        assert!(!broken.model.keeps_session());
        let mut never = Zone::new(None);
        never.model.not_started(StartFailure::NoServer("no helper".into()));
        assert!(!never.model.keeps_session());
    }

    fn exited() -> AgentState {
        AgentState::Exited { code: Some(1), stderr: "boom".into() }
    }

    fn holder_lost() -> AgentState {
        AgentState::Lost { message: "its holder ended".into() }
    }

    /// 4.1: the offer over every phase, the session's life, the
    /// conversation (none, not continuable, each continuable kind) and the
    /// profile (found, or gone: deleted, or a substitute's, which the
    /// entity looks up by the session's own name and finds gone).
    #[test]
    fn the_offer_for_every_phase_life_conversation_and_profile() {
        use crate::agent::transcript::Continuation;
        let gone = ProfileFound::Gone { name: "Fake".into() };
        let conversations = [
            None,
            Some(conversation(Continuation::Neither)),
            Some(conversation(Continuation::ResumeAndLoad)),
            Some(conversation(Continuation::Resume)),
            Some(conversation(Continuation::Load)),
        ];
        for phase in every_phase() {
            for life in [SessionLife::Open, SessionLife::Ended] {
                for conversation in &conversations {
                    for profile in [ProfileFound::Agent, gone.clone()] {
                        let offer = continue_offer(&phase, life, conversation.as_ref(), &profile);
                        let ended = match &phase {
                            AgentPhase::Live(AgentState::Exited { .. } | AgentState::Lost { .. })
                            | AgentPhase::Lost { .. }
                            | AgentPhase::NotStarted(_) => true,
                            AgentPhase::Connecting
                            | AgentPhase::Broken { .. }
                            | AgentPhase::Live(
                                AgentState::Starting
                                | AgentState::Ready { .. }
                                | AgentState::LoginRequired { .. }
                                | AgentState::LoggingIn { .. }
                                | AgentState::Failed { .. }
                                | AgentState::Choosing { .. },
                            ) => false,
                        };
                        let kept = matches!(phase, AgentPhase::Live(_)) && life == SessionLife::Open;
                        let expected = match (ended, conversation, &profile) {
                            (false, _, _) => Offer::None(NoOffer::NotEnded),
                            (true, None, _) => Offer::None(NoOffer::NoConversation),
                            (true, Some(c), _) if !c.continuation.continuable() => Offer::None(NoOffer::NotOffered),
                            (true, Some(_), ProfileFound::Gone { name }) => {
                                Offer::None(NoOffer::ProfileGone { name: name.clone() })
                            }
                            (true, Some(_), ProfileFound::Agent) if kept => Offer::Restart,
                            (true, Some(c), ProfileFound::Agent) => Offer::Respawn { conversation: c.clone() },
                        };
                        assert_eq!(offer, expected, "{phase:?} {life:?} {conversation:?} {profile:?}");
                    }
                }
            }
        }
        // The rows the design names, spelled out.
        let resume = conversation(Continuation::Resume);
        let exited = AgentPhase::Live(exited());
        assert_eq!(continue_offer(&exited, SessionLife::Open, Some(&resume), &ProfileFound::Agent), Offer::Restart);
        let lost = AgentPhase::Live(holder_lost());
        assert_eq!(continue_offer(&lost, SessionLife::Open, Some(&resume), &ProfileFound::Agent), Offer::Restart);
        let gone_session = AgentPhase::Lost { reason: "the terminal server no longer has it".into() };
        assert_eq!(
            continue_offer(&gone_session, SessionLife::Open, Some(&resume), &ProfileFound::Agent),
            Offer::Respawn { conversation: resume.clone() }
        );
        assert_eq!(
            continue_offer(&exited, SessionLife::Ended, Some(&resume), &ProfileFound::Agent),
            Offer::Respawn { conversation: resume.clone() },
            "ended by a kill: the server no longer has it"
        );
        let failed = AgentPhase::Live(AgentState::Failed { message: "the agent couldn't continue".into() });
        assert_eq!(
            continue_offer(&failed, SessionLife::Ended, Some(&resume), &ProfileFound::Agent),
            Offer::None(NoOffer::NotEnded),
            "a refused continue isn't offered again"
        );
    }

    /// The reasons' words and the dump's.
    #[test]
    fn the_offers_words() {
        assert_eq!(NoOffer::NotOffered.text(), "the agent can't continue conversations");
        assert_eq!(NoOffer::NoConversation.text(), "the agent never started a conversation");
        assert_eq!(NoOffer::ProfileGone { name: "Fake".into() }.text(), "the profile Fake is gone");
        assert_eq!(NoOffer::NotEnded.text(), "the agent hasn't ended");
        assert_eq!(Offer::Restart.word(), "restart");
        let respawn = Offer::Respawn { conversation: conversation(crate::agent::transcript::Continuation::Load) };
        assert_eq!(respawn.word(), "respawn");
        let words: Vec<String> = [
            NoOffer::NotEnded,
            NoOffer::NoConversation,
            NoOffer::NotOffered,
            NoOffer::ProfileGone { name: "Fake".into() },
        ]
        .into_iter()
        .map(|why| Offer::None(why).word())
        .collect();
        assert_eq!(words, ["none:not-ended", "none:no-conversation", "none:not-offered", "none:profile-gone"]);
    }

    /// An agent lost with its holder has its own reason, alert and state
    /// word; an exited one keeps its session open (no `EXIT` came), and
    /// neither is a session a quit leaves for the next launch.
    #[test]
    fn an_ended_agent_on_a_kept_session() {
        let mut zone = Zone::ready();
        zone.feed(&[conversation_event(crate::agent::transcript::Continuation::Resume)]);
        assert!(zone.model.keeps_session());
        zone.feed(&[agent_state(holder_lost())]);
        assert_eq!(zone.model.life(), SessionLife::Open);
        assert_eq!(zone.model.prompt_status(), PromptStatus::Disabled(DisabledWhy::HolderLost));
        assert_eq!(DisabledWhy::HolderLost.text(), "the agent was lost with its holder");
        let alert = zone.model.alert("fake-acp", "this machine").unwrap();
        assert_eq!(alert.text, "Fake Agent was lost with its terminal server's holder");
        assert_eq!(alert.detail.as_deref(), Some("its holder ended"));
        let wait = zone.model.wait_text(&found(&zone.model), &linux);
        assert!(wait.starts_with("--- agent state=holder-lost "), "{wait}");
        assert!(wait.contains(" session=open continue=restart\n"), "{wait}");
        let gone = zone.model.offer(&ProfileFound::Gone { name: "Fake".into() });
        assert!(zone.model.describe(1, true, &gone, &linux).contains(" continue=none:profile-gone\n"));
        assert!(!zone.model.keeps_session(), "not recorded while the agent is lost");
        assert_eq!(zone.model.offer(&ProfileFound::Agent), Offer::Restart);
        zone.feed(&[agent_state(exited())]);
        assert_eq!(zone.model.prompt_status(), PromptStatus::Disabled(DisabledWhy::Exited));
        assert!(!zone.model.keeps_session(), "not recorded while the agent has exited");
        assert_eq!(zone.model.conversation().map(|c| c.id.as_str()), Some("sess-1"), "kept through the exit");
    }

    /// 4.1: a held prompt's send-again is unavailable while the agent has
    /// exited and while the restart starts it, and available once the
    /// continued agent is ready, on the same open session.
    #[test]
    fn send_again_comes_back_after_a_restarts_ready_and_not_before() {
        let mut zone = Zone::ready();
        zone.feed(&[
            conversation_event(crate::agent::transcript::Continuation::Resume),
            turn(TurnState::Working),
            queue(waiting()),
        ]);
        zone.feed(&[
            queue(held(HoldReason::AgentStopped)),
            turn(TurnState::Failed { message: "the agent exited".into() }),
            agent_state(exited()),
        ]);
        let unavailable = |model: &ChatModel| match model.queue() {
            QueueRow::Held { resend: Resend::Unavailable(why), .. } => Some(why.to_string()),
            QueueRow::Held { resend: Resend::Available, .. } | QueueRow::Waiting { .. } | QueueRow::None => None,
        };
        assert_eq!(unavailable(&zone.model).as_deref(), Some("the agent has exited"));
        assert!(!zone.model.resend());

        zone.model.restarting(9);
        assert_eq!(zone.model.phase(), &AgentPhase::Live(AgentState::Starting));
        assert_eq!(unavailable(&zone.model).as_deref(), Some("the agent is starting"));
        assert_eq!(zone.model.replied(9, Ok(())), Replied::Nothing);
        let notice = Entry::Notice {
            level: NoticeLevel::Info,
            text: "the agent was started again to continue the conversation".into(),
        };
        zone.feed(&[added(1, notice), agent_state(AgentState::Starting)]);
        assert_eq!(unavailable(&zone.model).as_deref(), Some("the agent is starting"));
        assert!(!zone.model.resend(), "not before the continued agent is ready");
        zone.feed(&[conversation_event(crate::agent::transcript::Continuation::Resume), ready()]);
        assert!(matches!(zone.model.queue(), QueueRow::Held { resend: Resend::Available, .. }));
        assert!(zone.model.resend(), "once it is ready");
        assert_eq!(zone.model.life(), SessionLife::Open);
        assert!(zone.model.keeps_session());
    }

    /// A refused restart shows the ended agent again, with the refusal;
    /// one whose events already moved the agent on leaves it there.
    #[test]
    fn a_refused_restart_shows_the_ended_agent_again() {
        let mut zone = Zone::ready();
        zone.feed(&[agent_state(exited())]);
        zone.model.restarting(4);
        assert_eq!(zone.model.prompt_status(), PromptStatus::Disabled(DisabledWhy::Starting));
        assert_eq!(zone.model.replied(4, Err("session 7: the agent hasn't ended".into())), Replied::Nothing);
        assert_eq!(zone.model.phase(), &AgentPhase::Live(exited()));
        assert_eq!(zone.model.refusal(), Some("session 7: the agent hasn't ended"));

        zone.model.restarting(5);
        assert_eq!(zone.model.refusal(), None, "a new try clears it");
        zone.feed(&[ready()]);
        zone.model.replied(5, Err("late".into()));
        assert!(matches!(zone.model.phase(), AgentPhase::Live(AgentState::Ready { .. })));

        let mut lost = Zone::ready();
        lost.model.lost("gone");
        assert_eq!(lost.model.restarting(6), Absorbed::default(), "nothing to restart outside a live phase");
    }

    /// A continue in a new session reconnects keeping the conversation
    /// shown, until the new session's first events replace it; the old
    /// conversation is kept until the new one names its own.
    #[test]
    fn a_respawn_keeps_the_old_conversation_until_the_new_session_replaces_it() {
        use crate::agent::transcript::Continuation;
        let mut zone = Zone::ready();
        zone.feed(&[conversation_event(Continuation::Load), added(1, user("hi")), added(2, agent("Hello!"))]);
        zone.feed(&[queue(held(HoldReason::AgentStopped)), agent_state(exited())]);
        zone.model.ended();
        zone.model.lost("the terminal server no longer has the session");
        let Offer::Respawn { conversation: kept } = zone.model.offer(&ProfileFound::Agent) else { panic!("respawn") };
        assert_eq!(kept.id, "sess-1");

        zone.model.respawning();
        assert_eq!(zone.model.phase(), &AgentPhase::Connecting);
        assert_eq!((zone.model.life(), zone.model.link()), (SessionLife::Open, Link::Up));
        assert_eq!(zone.model.queue(), &QueueRow::None, "the old session's slot went with it");
        assert_eq!(texts(&zone.model), ["hi", "Hello!"], "shown until the new session's events");
        zone.model.session_running();

        let mut fresh = AgentShared::new();
        fresh.start_over();
        fresh.apply(&[agent_state(AgentState::Starting).to_value()]).unwrap();
        let changes = fresh.take_changes();
        let taken = zone.model.copy_changes(fresh.transcript(), changes);
        zone.model.absorb(taken);
        assert_eq!(zone.model.draw(Due::All), [Splice { range: 0..2, count: 0 }]);
        assert_eq!(zone.model.phase(), &AgentPhase::Live(AgentState::Starting));
        assert_eq!(zone.model.conversation().map(|c| c.id.as_str()), Some("sess-1"), "known until replaced");

        let replay = Entry::Notice {
            level: NoticeLevel::Info,
            text: "this continues an earlier conversation; what follows is the agent's replay of it".into(),
        };
        let values: Vec<_> = [
            added(1, replay),
            added(2, user("hi")),
            added(3, agent("Hello!")),
            Event::Conversation {
                conversation: conversation(Continuation::Resume),
            },
            ready(),
        ]
        .iter()
        .map(Event::to_value)
        .collect();
        fresh.apply(&values).unwrap();
        let changes = fresh.take_changes();
        let taken = zone.model.copy_changes(fresh.transcript(), changes);
        zone.model.absorb(taken);
        zone.model.draw(Due::All);
        assert_eq!(texts(&zone.model).len(), 3);
        assert_eq!(zone.model.conversation().map(|c| c.continuation), Some(Continuation::Resume), "the new one's");
        assert_eq!(zone.model.prompt_status(), PromptStatus::Ready);
    }
}
