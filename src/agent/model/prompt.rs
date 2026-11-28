//! The prompt input: what a submit does, and the launch's first prompt.

use crate::FirstPrompt;
use crate::agent::commands::{self, Local};
use crate::agent::shared::StatusFrom;
use crate::agent::transcript::{AgentState, Slot};
use crate::remote::protocol::client::OptionValue;

use super::{ChatModel, SessionHandle};
use super::phase::{AgentPhase, DisabledWhy, Link, PromptStatus, SessionLife};

/// Why enter sends nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// An IME composition is open: enter commits it.
    Composing,
    Blank,
    NotReady(DisabledWhy),
    /// A prompt is on its way.
    InFlight,
    /// A turn runs and the slot holds a prompt.
    AlreadyQueued,
    /// An option command named none of its option's choices (OpenSpec
    /// change `agents-commands`): the text stays, with the reason.
    NoSuchChoice,
}

/// The words for a prompt kept because one is already queued.
pub const ALREADY_QUEUED: &str = "a prompt is already queued";

/// The refusal shown when a request couldn't be sent at all
/// ([`ChatModel::not_sent`]).
pub const UNSENT: &str = "the agent session isn't connected; nothing was sent";

/// The launch's first prompt (decision 3).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum FirstState {
    None,
    /// Waiting for the agent and the session.
    Waiting(FirstPrompt),
    /// Shown in the read-only input while the agent needs a login.
    Shown(FirstPrompt),
}

/// What enter does with the prompt input's text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Submit {
    /// Send this as a prompt (the server sends it, or queues it during a
    /// turn); the input keeps it until the server accepts it.
    Send(String),
    /// An option command (OpenSpec change `agents-commands`, decision 6):
    /// ask the server to set option `option` (named `name`) to `value`,
    /// the input emptied, nothing sent to the agent or queued.
    Set { option: String, name: String, value: OptionValue },
    /// An option command alone: show option `option`'s choices, the input
    /// emptied.
    Choose { option: String },
    Nothing(Why),
}

/// What a launch's first prompt does now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum First {
    /// The agent is ready: send it, once.
    Send(String),
    /// The agent needs a login the zone can run: the read-only prompt
    /// input shows it while it waits (OpenSpec change `agents-login`
    /// decision 10).
    Show(String),
    /// The agent is ready after it was shown: send it, once, only if the
    /// prompt input still holds exactly it.
    SendIfUnchanged(String),
    /// It can't be sent: it becomes the prompt input's draft, unsent.
    Draft(String),
}

impl ChatModel {
    /// The launch's first prompt: sent on the first live `Ready`, never
    /// from a replay; left as the draft on a failure, or when a replay is
    /// what shows the agent ready. While the agent needs a login the zone
    /// can run, it is shown in the read-only input and kept, through the
    /// login, a failed or cancelled one included, and sent once the agent
    /// is ready if the input still holds it (OpenSpec change
    /// `agents-login` decision 10). It waits for the running session too,
    /// and for a link that is up (OpenSpec change
    /// `agents-client-state-fixes`, decision 3).
    pub(super) fn settle_first(&mut self, status_from: StatusFrom) -> Option<First> {
        let (first, shown) = match &self.first {
            FirstState::None => return None,
            FirstState::Waiting(first) => (first.clone(), false),
            FirstState::Shown(first) => (first.clone(), true),
        };
        let text = first.text().to_string();
        let now = match (&self.phase, self.life) {
            (_, SessionLife::Ended) => First::Draft(text),
            (AgentPhase::Live(AgentState::Ready { .. }), SessionLife::Open) => {
                match (status_from, self.link, self.handle) {
                    (StatusFrom::Replay, _, _) => First::Draft(text),
                    (StatusFrom::Unchanged | StatusFrom::Live, Link::Reconnecting, _)
                    | (StatusFrom::Unchanged | StatusFrom::Live, Link::Up, SessionHandle::Starting) => return None,
                    (StatusFrom::Unchanged | StatusFrom::Live, Link::Up, SessionHandle::Running) if shown => {
                        First::SendIfUnchanged(text)
                    }
                    (StatusFrom::Unchanged | StatusFrom::Live, Link::Up, SessionHandle::Running) => First::Send(text),
                }
            }
            (
                AgentPhase::Connecting
                | AgentPhase::Live(AgentState::Starting | AgentState::LoggingIn { .. } | AgentState::Choosing { .. }),
                SessionLife::Open,
            ) => return None,
            (AgentPhase::Live(AgentState::LoginRequired { .. }), SessionLife::Open) if self.logins_usable() => {
                if shown {
                    return None;
                }
                self.first = FirstState::Shown(first);
                return Some(First::Show(text));
            }
            (
                AgentPhase::NotStarted(_)
                | AgentPhase::Live(
                    AgentState::LoginRequired { .. }
                    | AgentState::Failed { .. }
                    | AgentState::Exited { .. }
                    | AgentState::Lost { .. },
                )
                | AgentPhase::Lost { .. }
                | AgentPhase::Broken { .. },
                SessionLife::Open,
            ) => First::Draft(text),
        };
        self.first = FirstState::None;
        Some(now)
    }

    /// The launch's first prompt while the read-only input shows it,
    /// waiting for a login (the dump's `first=`).
    pub fn first_shown(&self) -> Option<&str> {
        match &self.first {
            FirstState::Shown(first) => Some(first.text()),
            FirstState::None | FirstState::Waiting(_) => None,
        }
    }

    /// Enter in the prompt input with `draft`, `composing` when an IME
    /// composition is open. During a turn with the slot empty it sends (the
    /// server queues it); while a turn runs with the slot taken it keeps
    /// the text with the reason; with no turn, a held prompt doesn't block.
    pub fn submit(&mut self, draft: &str, composing: bool) -> Submit {
        // An option command is Brindle's (OpenSpec change `agents-commands`,
        // decision 6): carried out during a turn too, never queued.
        if !composing && let Some(local) = self.local(draft) {
            return match local {
                Local::Set { option, name, value } => {
                    self.refusal = None;
                    Submit::Set { option, name, value }
                }
                Local::Choose { option } => {
                    self.refusal = None;
                    Submit::Choose { option }
                }
                Local::NoSuchChoice { .. } => {
                    self.refusal = local.reason();
                    Submit::Nothing(Why::NoSuchChoice)
                }
            };
        }
        let why = if composing {
            Some(Why::Composing)
        } else if draft.trim().is_empty() {
            Some(Why::Blank)
        } else {
            match self.prompt_status() {
                PromptStatus::Disabled(why) => Some(Why::NotReady(why)),
                PromptStatus::Submitted => Some(Why::InFlight),
                PromptStatus::Streaming => match &self.slot {
                    Slot::Empty => None,
                    Slot::Waiting { .. } | Slot::Held { .. } => Some(Why::AlreadyQueued),
                },
                PromptStatus::Ready => None,
            }
        };
        match why {
            Some(why) => {
                if why == Why::AlreadyQueued {
                    self.refusal = Some(ALREADY_QUEUED.into());
                }
                Submit::Nothing(why)
            }
            None => {
                self.refusal = None;
                // A command goes without the whitespace typed before it.
                Submit::Send(commands::command_text(draft).unwrap_or(draft).to_string())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::agent::transcript::HoldReason;

    use super::super::harness::*;
    use super::super::*;
    use super::*;

    /// Decision 3: the first prompt is sent once, on the first live `Ready`.
    #[test]
    fn the_first_prompt_is_sent_once_on_the_first_live_ready() {
        let mut zone = Zone::new(Some("run the tests"));
        assert_eq!(zone.feed(&[agent_state(AgentState::Starting)]).first, None, "not yet");
        assert_eq!(zone.feed(&[ready()]).first, Some(First::Send("run the tests".into())));
        assert_eq!(zone.feed(&[ready()]).first, None, "once");
        assert_eq!(zone.model.ended().first, None);

        let mut zone = Zone::new(Some("run the tests"));
        zone.model.set_link(Link::Reconnecting);
        assert_eq!(zone.feed(&[ready()]).first, None, "held back while reconnecting");
        assert_eq!(zone.model.set_link(Link::Up).first, Some(First::Send("run the tests".into())));

        let mut zone = Zone::new(None);
        assert_eq!(zone.feed(&[ready()]).first, None, "a restored or attached zone has none");
    }

    /// Decision 3: a replay never sends it; every failure leaves it as the
    /// draft.
    #[test]
    fn the_first_prompt_becomes_the_draft_on_a_failure_or_a_replay() {
        let draft = Some(First::Draft("run the tests".into()));
        let mut zone = Zone::new(Some("run the tests"));
        assert_eq!(zone.replay(&[ready()]).first, draft, "never sent from a replay");

        let failed = [
            AgentState::LoginRequired { message: "log in".into(), methods: Vec::new() },
            AgentState::Failed { message: "bad".into() },
            AgentState::Exited { code: Some(1), stderr: String::new() },
        ];
        for state in failed {
            let mut zone = Zone::new(Some("run the tests"));
            assert_eq!(zone.feed(&[agent_state(state.clone())]).first, draft, "{state:?}");
            assert_eq!(zone.feed(&[ready()]).first, None, "and never sent later");
        }
        let mut zone = Zone::new(Some("run the tests"));
        assert_eq!(zone.model.not_started(StartFailure::NoServer("no helper".into())).first, draft);
        let mut zone = Zone::new(Some("run the tests"));
        assert_eq!(zone.model.lost("gone").first, draft);
        let mut zone = Zone::new(Some("run the tests"));
        assert_eq!(zone.model.broken("bad event").first, draft);
        let mut zone = Zone::new(Some("run the tests"));
        zone.feed(&[agent_state(AgentState::Starting)]);
        assert_eq!(zone.model.ended().first, draft);

        // A refused send leaves it as the draft, with the refusal.
        let mut zone = Zone::new(Some("run the tests"));
        zone.feed(&[ready()]);
        zone.model.sent(7, zone.model.sent_prompt("run the tests", true));
        assert_eq!(zone.model.prompt_status(), PromptStatus::Submitted);
        assert_eq!(zone.model.replied(7, Err("the agent isn't ready".into())), Replied::SetDraft("run the tests".into()));
        assert_eq!(zone.model.refusal(), Some("the agent isn't ready"));
    }

    /// Decision 9: a new prompt beside a held one is sent with no turn and
    /// kept during one; a waiting one blocks it during a turn too.
    #[test]
    fn a_new_prompt_beside_the_slot() {
        let mut zone = Zone::ready();
        zone.feed(&[queue(held(HoldReason::Cancelled))]);
        assert_eq!(zone.model.submit("again", false), Submit::Send("again".into()), "no turn: held doesn't block");
        zone.feed(&[turn(TurnState::Working)]);
        assert_eq!(zone.model.prompt_status(), PromptStatus::Streaming);
        assert_eq!(zone.model.submit("again", false), Submit::Nothing(Why::AlreadyQueued));
        assert_eq!(zone.model.refusal(), Some(ALREADY_QUEUED));
        zone.feed(&[queue(waiting())]);
        assert_eq!(zone.model.submit("again", false), Submit::Nothing(Why::AlreadyQueued));
        zone.feed(&[queue(Slot::Empty)]);
        assert_eq!(zone.model.submit("again", false), Submit::Send("again".into()), "the server queues it");
        assert_eq!(zone.model.refusal(), None);

        assert_eq!(zone.model.submit("again", true), Submit::Nothing(Why::Composing));
        assert_eq!(zone.model.submit(" \n", false), Submit::Nothing(Why::Blank));
        zone.model.sent(1, zone.model.sent_prompt("again", false));
        assert_eq!(zone.model.submit("again", false), Submit::Nothing(Why::InFlight));
        assert_eq!(zone.model.replied(1, Ok(())), Replied::ClearDraft);
        zone.model.sent(2, zone.model.sent_prompt("again", false));
        assert_eq!(zone.model.replied(2, Err("no".into())), Replied::Nothing, "a refused prompt keeps the draft");
        assert_eq!(zone.model.refusal(), Some("no"));
        assert_eq!(zone.model.replied(2, Ok(())), Replied::Nothing, "answered once");
        zone.model.set_link(Link::Reconnecting);
        assert_eq!(zone.model.submit("again", false), Submit::Nothing(Why::NotReady(DisabledWhy::Reconnecting)));
        assert_eq!(Zone::new(None).model.submit("hi", false), Submit::Nothing(Why::NotReady(DisabledWhy::Connecting)));
    }

    /// Decision 9: no slot change touches the prompt input's draft.
    #[test]
    fn no_slot_change_alters_the_draft() {
        let mut zone = Zone::new(Some("first"));
        assert_eq!(zone.feed(&[ready()]).first, Some(First::Send("first".into())));
        let mut slots = vec![Slot::Empty, waiting()];
        slots.extend(EVERY_HOLD.iter().cloned().map(held));
        for slot in slots.iter().chain(slots.iter().rev()) {
            let absorbed = zone.feed(&[queue(slot.clone())]);
            assert_eq!(absorbed.first, None, "{slot:?}");
            assert!(absorbed.splices.is_empty() && absorbed.status.is_none(), "{slot:?}");
        }
        zone.model.sent(3, Sent::Unqueue);
        zone.model.sent(4, Sent::Resend);
        assert_eq!(zone.model.replied(3, Ok(())), Replied::Nothing);
        assert_eq!(zone.model.replied(4, Err("no prompt is held".into())), Replied::Nothing);
    }
}
