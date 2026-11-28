//! The queue row: the slot's waiting or held prompt, why it is held, and
//! what remove and send-again do.

use gpui::SharedString;

use crate::agent::transcript::{AgentState, HoldReason, Slot, StopReason, TurnState};

use super::{ChatModel, SessionHandle};
use super::phase::{AgentPhase, HOLDER_LOST, Link, SessionLife};

/// Whether a value may be resent from the queue row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Resend {
    Available,
    /// Disabled, with the tooltip saying why.
    Unavailable(SharedString),
}

/// The row above the prompt input: the queue slot as the zone shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QueueRow {
    None,
    /// Queued: sent when the running turn ends normally.
    Waiting { text: SharedString },
    /// Not sent, with why in words, and whether send-again works.
    Held { text: SharedString, reason: HoldReason, why: SharedString, resend: Resend },
}

/// The name a stop reason has in the dump and the hold's words.
pub fn stop_word(stop: &StopReason) -> &str {
    match stop {
        StopReason::EndTurn => "end_turn",
        StopReason::MaxTokens => "max_tokens",
        StopReason::MaxTurnRequests => "max_turn_requests",
        StopReason::Refusal => "refusal",
        StopReason::Cancelled => "cancelled",
        StopReason::Other { name } => name,
    }
}

/// Why a prompt is held, in words ("not sent: …").
pub fn hold_words(reason: &HoldReason) -> String {
    match reason {
        HoldReason::Cancelled => "you stopped the turn".into(),
        HoldReason::Failed => "the turn failed".into(),
        HoldReason::Stopped { stop } => format!("the turn ended: {}", stop_word(stop)),
        HoldReason::LoginRequired => "the agent needs a login".into(),
        HoldReason::AgentStopped => "the agent stopped".into(),
        HoldReason::OutputLost => "output was lost while the terminal server restarted".into(),
    }
}

/// The dump's word for a hold reason.
pub(super) fn hold_word(reason: &HoldReason) -> String {
    match reason {
        HoldReason::Cancelled => "cancelled".into(),
        HoldReason::Failed => "failed".into(),
        HoldReason::Stopped { stop } => format!("stopped:{}", stop_word(stop)),
        HoldReason::LoginRequired => "login-required".into(),
        HoldReason::AgentStopped => "agent-stopped".into(),
        HoldReason::OutputLost => "output-lost".into(),
    }
}

pub(super) const EXITED: &str = "the agent has exited";

pub(super) const NEEDS_LOGIN: &str = "the agent needs a login";

pub(super) const LOGGING_IN: &str = "the agent is logging in";

/// A choosing agent's reason line (`agents-session-list` decision 6).
pub(super) const CHOOSING: &str = "choose a conversation, or start a new one";

/// A core refusal's words without the `session N: ` it starts with, for a
/// list's failure, which the palette shows in place of rows.
pub(super) fn without_session(why: &str) -> &str {
    why.strip_prefix("session ")
        .and_then(|rest| rest.split_once(": "))
        .filter(|(id, _)| !id.is_empty() && id.bytes().all(|b| b.is_ascii_digit()))
        .map_or(why, |(_, words)| words)
}

/// Whether a held prompt can be sent again, for the phase and the
/// session's life: only to a ready agent whose session is open.
fn resend_of(phase: &AgentPhase, life: SessionLife) -> Resend {
    let why = match life {
        SessionLife::Ended => EXITED,
        SessionLife::Open => match phase {
            AgentPhase::Live(AgentState::Ready { .. }) => return Resend::Available,
            AgentPhase::Live(AgentState::Starting) => "the agent is starting",
            AgentPhase::Live(AgentState::Choosing { .. }) => CHOOSING,
            AgentPhase::Live(AgentState::LoggingIn { .. }) => LOGGING_IN,
            AgentPhase::Live(AgentState::LoginRequired { .. }) => NEEDS_LOGIN,
            AgentPhase::Live(AgentState::Failed { .. }) => "the agent failed",
            AgentPhase::Live(AgentState::Exited { .. }) => EXITED,
            AgentPhase::Live(AgentState::Lost { .. }) => HOLDER_LOST,
            AgentPhase::Connecting | AgentPhase::NotStarted(_) => "the agent isn't ready",
            AgentPhase::Lost { .. } => "the agent session was lost",
            AgentPhase::Broken { .. } => "Brindle can't show this agent session any more",
        },
    };
    Resend::Unavailable(why.into())
}

/// The queue row for `slot` in `phase` with the session's `life`.
pub fn queue_row(slot: &Slot, phase: &AgentPhase, life: SessionLife) -> QueueRow {
    match slot {
        Slot::Empty => QueueRow::None,
        Slot::Waiting { text, .. } => QueueRow::Waiting { text: text.clone().into() },
        Slot::Held { text, reason, .. } => QueueRow::Held {
            text: text.clone().into(),
            reason: reason.clone(),
            why: hold_words(reason).into(),
            resend: resend_of(phase, life),
        },
    }
}

/// What the queue row's remove (or discard) does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unqueue {
    /// Send `AGENT_UNQUEUE`; the row goes when the server's `Queue` comes.
    Send,
    /// The session has ended or was lost: the row was cleared here, and
    /// nothing is sent.
    Cleared,
    /// Nothing to remove, or nothing can be sent now.
    Nothing,
}

impl ChatModel {
    /// Whether anything can be sent to the session now.
    pub(super) fn can_send(&self) -> bool {
        let live = match &self.phase {
            AgentPhase::Live(_) => true,
            AgentPhase::Connecting | AgentPhase::NotStarted(_) | AgentPhase::Lost { .. } | AgentPhase::Broken { .. } => false,
        };
        live && self.life == SessionLife::Open && self.link == Link::Up && self.handle == SessionHandle::Running
    }

    /// The queue row's remove, or a held prompt's discard
    /// (`agent_unqueue`). After the session ended or was lost the row is
    /// only cleared here.
    pub fn unqueue(&mut self) -> Unqueue {
        match &self.slot {
            Slot::Empty => return Unqueue::Nothing,
            Slot::Waiting { .. } | Slot::Held { .. } => {}
        }
        let gone = self.life == SessionLife::Ended || matches!(self.phase, AgentPhase::Lost { .. });
        if gone {
            self.slot = Slot::Empty;
            self.queue = QueueRow::None;
            Unqueue::Cleared
        } else if self.can_send() {
            Unqueue::Send
        } else {
            Unqueue::Nothing
        }
    }

    /// Whether the held prompt can be sent again now (`agent_resend`).
    pub fn resend(&self) -> bool {
        matches!(&self.queue, QueueRow::Held { resend: Resend::Available, .. }) && self.can_send()
    }

    /// Whether a stop (escape, the stop button, `agent_cancel`) has a turn
    /// to stop.
    pub fn cancel(&self) -> bool {
        let turn = match &self.turn {
            TurnState::Working | TurnState::Blocked => true,
            TurnState::Idle | TurnState::Cancelling | TurnState::Ended { .. } | TurnState::Failed { .. } => false,
        };
        turn && self.can_send()
    }

    /// `copy_queued_prompt`: the slot's text, waiting or held.
    pub fn queued_text(&self) -> Option<&str> {
        match &self.slot {
            Slot::Empty => None,
            Slot::Waiting { text, .. } | Slot::Held { text, .. } => Some(text),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::harness::*;
    use super::super::*;
    use super::*;

    /// Decisions 8 and 9: every slot and hold reason with each phase, and
    /// the session's life.
    #[test]
    fn every_slot_and_hold_reason_with_each_phase() {
        let words = [
            "you stopped the turn",
            "the turn failed",
            "the turn ended: max_tokens",
            "the turn ended: max_turn_requests",
            "the turn ended: refusal",
            "the turn ended: cancelled",
            "the agent needs a login",
            "the agent stopped",
            "output was lost while the terminal server restarted",
        ];
        for (reason, words) in EVERY_HOLD.iter().zip(words) {
            assert_eq!(hold_words(reason), words);
        }
        for phase in every_phase() {
            for life in [SessionLife::Open, SessionLife::Ended] {
                assert_eq!(queue_row(&Slot::Empty, &phase, life), QueueRow::None);
                assert_eq!(queue_row(&waiting(), &phase, life), QueueRow::Waiting { text: "next".into() });
                for reason in &EVERY_HOLD {
                    let QueueRow::Held { text, reason: shown, why, resend } = queue_row(&held(reason.clone()), &phase, life)
                    else {
                        panic!("{phase:?} {reason:?}")
                    };
                    assert_eq!((text.as_ref(), &shown, why.to_string()), ("next", reason, hold_words(reason)));
                    let ready = matches!(phase, AgentPhase::Live(AgentState::Ready { .. }));
                    let expected = match (ready, life) {
                        (true, SessionLife::Open) => Resend::Available,
                        (_, SessionLife::Ended) => Resend::Unavailable("the agent has exited".into()),
                        (false, SessionLife::Open) => resend_of(&phase, life),
                    };
                    assert_eq!(resend, expected, "{phase:?} {life:?}");
                    assert_ne!(resend == Resend::Available, !ready || life == SessionLife::Ended);
                }
            }
        }
        let login = AgentPhase::Live(AgentState::LoginRequired { message: "log in".into(), methods: Vec::new() });
        assert_eq!(resend_of(&login, SessionLife::Open), Resend::Unavailable("the agent needs a login".into()));
        let logging_in = AgentPhase::Live(AgentState::LoggingIn { method: "g".into(), name: "Google".into() });
        assert_eq!(resend_of(&logging_in, SessionLife::Open), Resend::Unavailable("the agent is logging in".into()));
        assert_eq!(resend_of(&AgentPhase::Live(AgentState::Starting), SessionLife::Open), Resend::Unavailable("the agent is starting".into()));
        assert_eq!(
            resend_of(&AgentPhase::Live(AgentState::Failed { message: "x".into() }), SessionLife::Open),
            Resend::Unavailable("the agent failed".into())
        );
    }

    /// Decision 9: unqueue and resend send requests while the session is
    /// open, and after it ended (or was lost) only clear the row here.
    #[test]
    fn unqueue_and_resend() {
        let mut zone = Zone::ready();
        assert_eq!(zone.model.unqueue(), Unqueue::Nothing);
        assert!(!zone.model.resend());
        zone.feed(&[queue(waiting())]);
        assert_eq!(zone.model.unqueue(), Unqueue::Send);
        assert!(!zone.model.resend(), "only a held prompt is sent again");
        let absorbed = zone.feed(&[queue(held(HoldReason::Failed))]);
        assert!(absorbed.queue_changed);
        assert!(zone.model.resend());
        assert_eq!(zone.model.unqueue(), Unqueue::Send);
        assert_eq!(zone.model.queued_text(), Some("next"));

        zone.feed(&[agent_state(AgentState::Exited { code: Some(0), stderr: String::new() })]);
        zone.model.ended();
        let QueueRow::Held { resend, .. } = zone.model.queue() else { panic!("{:?}", zone.model.queue()) };
        assert_eq!(resend, &Resend::Unavailable("the agent has exited".into()), "the zone keeps its copy");
        assert!(!zone.model.resend());
        assert_eq!(zone.model.unqueue(), Unqueue::Cleared);
        assert_eq!(zone.model.queue(), &QueueRow::None);
        assert_eq!(zone.model.queued_text(), None);

        let mut zone = Zone::ready();
        zone.feed(&[queue(held(HoldReason::AgentStopped))]);
        zone.model.lost("gone");
        assert_eq!(zone.model.unqueue(), Unqueue::Cleared);
        let mut zone = Zone::ready();
        zone.feed(&[queue(waiting())]);
        zone.model.set_link(Link::Reconnecting);
        assert_eq!(zone.model.unqueue(), Unqueue::Nothing, "nothing is sent while reconnecting");
    }
}
