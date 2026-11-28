//! The requests the zone sent and their answers: what a reply or a
//! request that wasn't sent does, and whether a prompt lost with the link
//! was taken.

use crate::FirstPrompt;
use crate::agent::transcript::{AgentState, Entry, EntryId, Slot, Window};

use super::{ChatModel, RequestId};
use super::attach::{Draft, same_attachments};
use super::choosing::{LaunchChoice, ListDone, ListFor};
use super::paging::Paging;
use super::phase::AgentPhase;
use super::prompt::{FirstState, UNSENT};
use super::queue::{queue_row, without_session};

/// Why a list couldn't go while the zone's session is still being set up.
pub const STARTING: &str = "the agent session is still starting";

/// What a fetch, or another request that never waits for the next link,
/// fails with while the link is down.
pub const LINK_DOWN: &str = "the connection to the terminal server is down";

/// A prompt whose answer was lost with the link, missing from the
/// reconnect's replay.
pub const PROMPT_NOT_TAKEN: &str = "the connection to the terminal server dropped before the prompt arrived; nothing was sent";

/// A prompt whose answer was lost with the link, when the replay can't tell
/// whether it arrived.
pub const PROMPT_MAYBE_TAKEN: &str =
    "the connection to the terminal server dropped as the prompt was sent; it may have reached the agent";

/// Why a request wasn't sent (OpenSpec change `agents-client-state-fixes`,
/// decision 1): what the model does with it is [`ChatModel::not_sent`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotSent {
    /// The session isn't handed to the zone yet: nothing was sent, and the
    /// zone can send once it runs.
    Starting,
    /// The zone let it go, it never started, or the connection no longer
    /// shows it (exited, lost, broken): nothing was sent, and nothing will
    /// be.
    Gone,
    /// The link is down, for a request that never waits for the next link
    /// (a fetch).
    LinkDown,
}

/// How a request the zone sent failed (decision 5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplyError {
    /// The core refused it, or it never went out: nothing happened.
    Failed(String),
    /// It was written on a link that went down before its answer: the core
    /// may have taken it.
    Unanswered(String),
}

/// The tests' refusals: the core's.
#[cfg(test)]
impl From<&str> for ReplyError {
    fn from(why: &str) -> Self {
        Self::Failed(why.to_string())
    }
}

impl ReplyError {
    /// What it says.
    pub fn text(&self) -> &str {
        match self {
            Self::Failed(why) | Self::Unanswered(why) => why,
        }
    }
}

/// A prompt as the zone sent it: its text, and the newest entry the zone
/// held then, which the replay is searched after when its answer is lost
/// (decision 5).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PromptSent {
    pub text: String,
    /// Its attachments (OpenSpec change `agents-prompt-context`, decision
    /// 13): [`taken`] compares them as well as the text.
    pub attachments: Vec<Draft>,
    pub after: Option<EntryId>,
}

/// A prompt whose answer was lost with the link, waiting for the
/// reconnect's replay to say whether the core took it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Uncertain {
    sent: PromptSent,
    /// The launch's first prompt, which was never in the input.
    first: bool,
}

/// Whether the core took a prompt whose answer was lost ([`taken`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Taken {
    Yes,
    No,
    /// The replay's page doesn't reach back far enough to say.
    CantTell,
}

/// Whether the core took `prompt`, from the reconnect's replay: the slot
/// holds its cleaned text and attachments (queued behind a turn), or a user
/// entry after the one the zone held when it was sent does; it can't tell
/// when the replay's window starts past that entry's successor; else it
/// didn't.
pub fn taken(prompt: &PromptSent, entries: &[(EntryId, Entry)], window: Option<Window>, slot: &Slot) -> Taken {
    let kept = crate::agent::transcript::clean(&prompt.text);
    let queued = match slot {
        Slot::Waiting { text, attachments } | Slot::Held { text, attachments, .. } => {
            *text == kept && same_attachments(&prompt.attachments, attachments)
        }
        Slot::Empty => false,
    };
    let logged = entries.iter().any(|(id, entry)| {
        prompt.after.is_none_or(|after| *id > after)
            && matches!(entry, Entry::User { text, attachments } if *text == kept && same_attachments(&prompt.attachments, attachments))
    });
    if queued || logged {
        return Taken::Yes;
    }
    let cut = window.is_some_and(|window| match prompt.after {
        Some(after) => window.from.0 > after.0.saturating_add(1),
        None => window.earlier > 0,
    });
    if cut { Taken::CantTell } else { Taken::No }
}

/// A request the zone sent, kept until its reply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sent {
    /// A typed prompt ([`ChatModel::sent_prompt`]).
    Prompt(PromptSent),
    /// The launch's first prompt, put back as the draft if refused.
    FirstPrompt(PromptSent),
    Resend,
    Unqueue,
    Answer(EntryId),
    /// `AGENT_RESTART`, sent while the agent was `was`: a refusal shows it
    /// again.
    Restart { was: AgentState },
    /// `AGENT_LOGIN`: a refusal is shown beside the login row.
    Login,
    /// `AGENT_FETCH` (OpenSpec change `agents-log-paging`): its page comes
    /// as a change before the reply; a refusal shows on the Earlier row.
    Fetch,
    /// `AGENT_LIST` (OpenSpec change `agents-session-list`), for what it
    /// says: its page comes in place of the reply.
    List(ListFor),
    /// `AGENT_CHOOSE`: a refusal is shown beside the prompt input.
    Choose,
    /// `AGENT_SET_OPTION` (OpenSpec change `agents-commands`) for the
    /// option named `name`: a refusal goes to the window's notice line.
    SetOption { name: String },
    /// `AGENT_ATTACH` of a text the prompt sent after it names (OpenSpec
    /// change `agents-prompt-context`): a refusal is shown beside the
    /// prompt input, in place of the prompt's own that follows.
    Attach,
    /// `AGENT_READ_ATTACHMENT` for user entry `entry`'s attachment `index`:
    /// its text comes in place of the reply; a refusal says the row's text
    /// is no longer kept.
    Read { entry: EntryId, index: usize },
}

/// What a reply does to the prompt input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Replied {
    /// A prompt was accepted: the input empties.
    ClearDraft,
    /// The first prompt was refused: it becomes the draft.
    SetDraft(String),
    /// A list was answered (OpenSpec change `agents-session-list`).
    Listed(ListDone),
    /// A set was refused (OpenSpec change `agents-commands`, flag 11):
    /// this text goes to the window's notice line, and nothing changes.
    OptionRefused(String),
    /// The input is left as it is.
    Nothing,
}

impl ChatModel {
    /// Settles a prompt whose answer was lost with the link: as
    /// `decided`, else from the entries, window and slot the zone now
    /// holds ([`taken`]).
    pub(super) fn settle_uncertain(&mut self, uncertain: Uncertain, decided: Option<Taken>) -> Replied {
        let Uncertain { sent, first } = uncertain;
        let taken = decided.unwrap_or_else(|| taken(&sent, &self.entries, self.window, &self.slot));
        let draft = |text: String| if first { Replied::SetDraft(text) } else { Replied::Nothing };
        match taken {
            Taken::Yes => {
                self.refusal = None;
                if first {
                    Replied::Nothing
                } else {
                    self.attached.clear();
                    Replied::ClearDraft
                }
            }
            Taken::No => {
                self.refusal = Some(PROMPT_NOT_TAKEN.into());
                draft(sent.text)
            }
            Taken::CantTell => {
                self.refusal = Some(PROMPT_MAYBE_TAKEN.into());
                draft(sent.text)
            }
        }
    }

    /// A prompt is on its way, or its answer was lost with the link and the
    /// replay hasn't said whether it arrived.
    pub(super) fn in_flight(&self) -> bool {
        self.uncertain.is_some() || self.pending.values().any(|sent| matches!(sent, Sent::Prompt(_) | Sent::FirstPrompt(_)))
    }

    /// A request went out as `request`.
    pub fn sent(&mut self, request: RequestId, sent: Sent) {
        self.pending.insert(request, sent);
    }

    /// What the ledger records for prompt `text` (the launch's `first`, or
    /// a typed one): the newest entry the zone holds now, which the replay
    /// is searched after if its answer is lost (decision 5).
    pub fn sent_prompt(&self, text: impl Into<String>, first: bool) -> Sent {
        // A launch's first prompt has none: `-p` takes no attachments.
        let attachments = if first { Vec::new() } else { self.drafts() };
        let sent = PromptSent { text: text.into(), attachments, after: self.entries.last().map(|(id, _)| *id) };
        if first { Sent::FirstPrompt(sent) } else { Sent::Prompt(sent) }
    }

    /// The server answered `request`: a prompt accepted empties the input;
    /// a refusal is shown beside it, and a refused first prompt becomes
    /// the draft. A prompt whose answer was lost with the link waits for
    /// the reconnect's replay ([`Absorbed::replied`]).
    pub fn replied(&mut self, request: RequestId, outcome: Result<(), ReplyError>) -> Replied {
        let Some(sent) = self.pending.remove(&request) else { return Replied::Nothing };
        if sent == Sent::Fetch {
            self.fetched(outcome);
            return Replied::Nothing;
        }
        self.answered(sent, outcome)
    }

    /// A request couldn't go, for `why` (decision 1's table): one that
    /// meets a session still being set up is held, never refused; one whose
    /// session is gone is refused with [`UNSENT`], a first prompt becoming
    /// the draft; a fetch with the link down fails with [`LINK_DOWN`].
    pub fn not_sent(&mut self, sent: Sent, why: NotSent) -> Replied {
        let text = match why {
            NotSent::Starting => return self.held_back(sent),
            NotSent::Gone => UNSENT,
            NotSent::LinkDown => LINK_DOWN,
        };
        match sent {
            Sent::Fetch => {
                if let Some(count) = self.paging_count() {
                    self.paging = Paging::Failed { count, why: text.into() };
                }
                Replied::Nothing
            }
            // A set that couldn't go is said beside the input, as any
            // request's (`agents-commands` decision 9); the server's
            // refusals go to the notice line.
            Sent::SetOption { .. } | Sent::Attach => {
                self.refusal = Some(text.into());
                Replied::Nothing
            }
            // The row closes again, with why beside the input.
            Sent::Read { entry, index } => {
                self.row_closed(entry, index);
                self.refusal = Some(text.into());
                Replied::Nothing
            }
            Sent::Prompt(_)
            | Sent::FirstPrompt(_)
            | Sent::Resend
            | Sent::Unqueue
            | Sent::Answer(_)
            | Sent::Restart { .. }
            | Sent::Login
            | Sent::List(_)
            | Sent::Choose => self.answered(sent, Err(ReplyError::Failed(text.into()))),
        }
    }

    /// A request that met a session still being set up: nothing recorded
    /// and nothing refused. A first prompt waits again, and a launch's list
    /// is asked again once the session runs; the palette is told the agent
    /// is still starting.
    fn held_back(&mut self, sent: Sent) -> Replied {
        match sent {
            Sent::FirstPrompt(PromptSent { text, .. }) => match FirstPrompt::new(text.clone()) {
                Ok(first) => {
                    self.first = FirstState::Waiting(first);
                    Replied::Nothing
                }
                // Never: it was one. Kept as the draft rather than lost.
                Err(_) => Replied::SetDraft(text),
            },
            Sent::List(ListFor::Palette) => Replied::Listed(ListDone::Palette(Err(STARTING.into()))),
            Sent::List(ListFor::Latest) => {
                self.choosing.launch = Some(LaunchChoice::Last);
                Replied::Nothing
            }
            Sent::Read { entry, index } => {
                self.row_closed(entry, index);
                Replied::Nothing
            }
            Sent::Prompt(_)
            | Sent::Resend
            | Sent::Unqueue
            | Sent::Answer(_)
            | Sent::Restart { .. }
            | Sent::Login
            | Sent::Fetch
            | Sent::Choose
            | Sent::SetOption { .. }
            | Sent::Attach => Replied::Nothing,
        }
    }

    /// An `AGENT_RESTART` that couldn't go, for `why`: as [`Self::not_sent`]
    /// says for a restart. Nothing was recorded for it ([`Self::restarting`]
    /// comes only once one went), so the agent shown stays as it is.
    pub fn restart_not_sent(&mut self, why: NotSent) -> Replied {
        let was = match &self.phase {
            AgentPhase::Live(state) => state.clone(),
            AgentPhase::Connecting | AgentPhase::NotStarted(_) | AgentPhase::Lost { .. } | AgentPhase::Broken { .. } => {
                AgentState::Starting
            }
        };
        self.not_sent(Sent::Restart { was }, why)
    }

    /// A cancel, or Cancel login, that couldn't go, for `why` (decision 1's
    /// table): nothing while the session is still being set up; once it is
    /// gone, said beside the prompt input.
    pub fn cancel_not_sent(&mut self, why: NotSent) {
        match why {
            NotSent::Starting => {}
            NotSent::Gone => self.server_note(UNSENT),
            NotSent::LinkDown => self.server_note(LINK_DOWN),
        }
    }

    /// The terminal server's word about the session that answers no request
    /// (a refused cancel): shown beside the prompt input, as a refusal.
    pub fn server_note(&mut self, text: impl Into<String>) {
        self.refusal = Some(text.into());
    }

    fn answered(&mut self, sent: Sent, outcome: Result<(), ReplyError>) -> Replied {
        // A prompt whose answer was lost: the reconnect's replay says.
        let outcome = match (sent, outcome) {
            (Sent::Prompt(sent), Err(ReplyError::Unanswered(_))) => {
                self.uncertain = Some(Uncertain { sent, first: false });
                return Replied::Nothing;
            }
            (Sent::FirstPrompt(sent), Err(ReplyError::Unanswered(_))) => {
                self.uncertain = Some(Uncertain { sent, first: true });
                return Replied::Nothing;
            }
            (sent, outcome) => (sent, outcome.map_err(|why| why.text().to_string())),
        };
        match outcome {
            (Sent::Prompt(_), Ok(())) => {
                self.refusal = None;
                self.attach_refused = false;
                self.attached.clear();
                Replied::ClearDraft
            }
            // The prompt after it says so, or is refused for it.
            (Sent::Attach, Ok(())) => Replied::Nothing,
            (Sent::Attach, Err(why)) => {
                self.refusal = Some(why);
                self.attach_refused = true;
                Replied::Nothing
            }
            // Its text comes as `ATTACHMENT`; an `OK` answers none.
            (Sent::Read { .. }, Ok(())) => Replied::Nothing,
            (Sent::Read { entry, index }, Err(why)) => {
                self.row_answered(entry, index, Err(why));
                Replied::Nothing
            }
            // Refused for an attach before it: that one's reason stays.
            (Sent::Prompt(_), Err(_)) if self.attach_refused => {
                self.attach_refused = false;
                Replied::Nothing
            }
            (
                Sent::FirstPrompt(_) | Sent::Resend | Sent::Unqueue | Sent::Answer(_) | Sent::Restart { .. } | Sent::Login,
                Ok(()),
            ) => Replied::Nothing,
            // Answered through `fetched`, which knows the request.
            (Sent::Fetch, Ok(()) | Err(_)) => Replied::Nothing,
            // A list's page comes as `listed`: an `OK` answers none.
            (Sent::List(_) | Sent::Choose, Ok(())) => Replied::Nothing,
            // The new value comes as the agent's `Options`, never before.
            (Sent::SetOption { .. }, Ok(())) => Replied::Nothing,
            (Sent::SetOption { name }, Err(why)) => {
                Replied::OptionRefused(format!("{name} wasn't changed: {}", without_session(&why)))
            }
            (Sent::List(purpose), Err(why)) => Replied::Listed(self.list_refused(purpose, why)),
            (Sent::Choose, Err(why)) => {
                self.refusal = Some(why);
                Replied::Nothing
            }
            (Sent::Login, Err(why)) => {
                self.login_note = Some(why);
                Replied::Nothing
            }
            (Sent::FirstPrompt(PromptSent { text, .. }), Err(why)) => {
                self.refusal = Some(why);
                Replied::SetDraft(text)
            }
            // Refused, nothing started: the ended agent shows again, unless
            // the server's events moved it on meanwhile.
            (Sent::Restart { was }, Err(why)) => {
                if self.phase == AgentPhase::Live(AgentState::Starting) {
                    self.phase = AgentPhase::Live(was);
                    self.queue = queue_row(&self.slot, &self.phase, self.life);
                }
                self.refusal = Some(why);
                Replied::Nothing
            }
            (Sent::Prompt(_) | Sent::Resend | Sent::Unqueue | Sent::Answer(_), Err(why)) => {
                self.refusal = Some(why);
                Replied::Nothing
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::agent::shared::AgentShared;
    use crate::agent::tool::RowState;
    use crate::agent::transcript::{Event, PAGE_BYTES, StopReason};
    use super::super::prompt::Why;

    use super::super::harness::*;
    use super::super::*;
    use super::*;

    /// A request that couldn't go because its session is gone is answered
    /// as a refusal: a prompt's draft stays, a first prompt becomes the
    /// draft; a server's note is shown the same way.
    #[test]
    fn an_unsent_request_is_refused_and_a_first_prompt_becomes_the_draft() {
        let mut zone = Zone::ready();
        let prompt = zone.model.sent_prompt("go", false);
        assert_eq!(zone.model.not_sent(prompt, NotSent::Gone), Replied::Nothing);
        assert_eq!(zone.model.refusal(), Some(UNSENT));
        let first = zone.model.sent_prompt("go", true);
        assert_eq!(zone.model.not_sent(first, NotSent::Gone), Replied::SetDraft("go".into()));
        assert_eq!(zone.model.not_sent(Sent::Resend, NotSent::Gone), Replied::Nothing);
        zone.model.server_note("the terminal server: no turn is running");
        assert_eq!(zone.model.refusal(), Some("the terminal server: no turn is running"));
        assert_eq!(zone.model.prompt_status(), PromptStatus::Ready, "nothing is in flight");
    }

    // --- OpenSpec change `agents-client-state-fixes` ---------------------

    /// D-4: a prompt whose answer was lost with the link, found in the
    /// reconnect's replay, was taken: no refusal, and the input empties.
    #[test]
    fn a_prompt_lost_with_the_link_and_found_in_the_replay_clears_its_draft() {
        let mut zone = Zone::ready();
        zone.feed(&[added(1, user("hi")), added(2, agent("Hello!"))]);
        zone.model.sent(7, zone.model.sent_prompt("next", false));
        let replied = zone.model.replied(7, Err(ReplyError::Unanswered("the terminal server went away (the relay ended)".into())));
        assert_eq!((replied, zone.model.refusal()), (Replied::Nothing, None), "not refused yet");
        zone.model.set_link(Link::Reconnecting);
        zone.model.set_link(Link::Up);
        let absorbed = zone.replay(&[ready(), added(1, user("hi")), added(2, agent("Hello!")), added(3, user("next"))]);
        assert_eq!(zone.model.refusal(), None);
        assert_eq!(absorbed.replied, Some(Replied::ClearDraft));
        assert_eq!(zone.model.prompt_status(), PromptStatus::Ready, "settled");
    }

    /// D-6: a new conversation in the zone forgets the old rows' and
    /// thoughts' toggles: the new log's entries at the same ids are drawn
    /// as the policy says.
    #[test]
    fn a_new_conversation_forgets_the_old_rows_and_thoughts() {
        let mut zone = Zone::ready();
        zone.feed(&[added(1, user("go")), turn(TurnState::Working), added(2, thought("hmm"))]);
        assert!(zone.model.thought_open(EntryId(2)), "the streaming thought opens on its own");
        zone.model.toggle_thought(EntryId(2));
        assert!(!zone.model.thought_open(EntryId(2)), "closed by hand");
        zone.feed(&[added(3, in_progress("c1", vec![diff_item("/p/a.rs")])), tool_completed(3)]);
        zone.feed(&[turn(TurnState::Ended { stop: StopReason::EndTurn })]);
        assert_eq!(row(&zone.model, 3).0, RowState::Closed, "finished: closed by policy");
        zone.model.toggle_tool(EntryId(3));
        assert_eq!(row(&zone.model, 3).0, RowState::Open, "opened by hand");

        zone.model.respawning();
        zone.shared = AgentShared::new();
        zone.shared.start_over();
        zone.feed(&[ready(), added(1, user("again")), turn(TurnState::Working), added(2, thought("thinking"))]);
        assert!(zone.model.thought_open(EntryId(2)), "the new streaming thought is open");
        zone.feed(&[added(3, in_progress("c9", vec![diff_item("/p/b.rs")])), tool_completed(3)]);
        assert_eq!(row(&zone.model, 3).0, RowState::Closed, "the new call is closed by policy");
        assert_drawn_afresh(&zone.model);
    }

    /// D-7: a respawn that doesn't start leaves no fetch loading.
    #[test]
    fn a_respawn_that_doesnt_start_leaves_no_fetch_loading() {
        let server = long_log(40);
        let mut zone = paged(&server);
        zone.model.fetching(5);
        assert!(matches!(zone.model.earlier(), Earlier::Loading { .. }));
        zone.model.respawning();
        zone.model.not_started(StartFailure::NoServer("no helper".into()));
        assert!(!matches!(zone.model.earlier(), Earlier::Loading { .. }), "{:?}", zone.model.earlier());
    }

    /// D-10: in one batch of changes, a replay of an idle ready agent, then
    /// a live permission: its notice is raised.
    #[test]
    fn a_live_permission_after_a_replay_raises_its_notice() {
        let mut zone = Zone::ready();
        zone.model.set_link(Link::Reconnecting);
        zone.shared.replay_start();
        let replayed = [ready(), added(1, user("go")), added(2, agent("done")), turn(TurnState::Ended { stop: StopReason::EndTurn })];
        zone.shared.apply(&replayed.iter().map(Event::to_value).collect::<Vec<_>>()).unwrap();
        zone.shared.replay_end();
        let live = [added(3, user("more")), turn(TurnState::Working), added(4, permission("Run ls")), turn(TurnState::Blocked)];
        zone.shared.apply(&live.iter().map(Event::to_value).collect::<Vec<_>>()).unwrap();
        zone.model.set_link(Link::Up);
        let absorbed = zone.absorb();
        assert_eq!(absorbed.status, Some(Some(State::Blocked)));
        assert!(absorbed.notice.is_some(), "the live permission is announced");
    }

    /// D-10: a first prompt, a replay showing the agent starting, then a
    /// live `Ready` in the same batch: the prompt is sent.
    #[test]
    fn a_live_ready_after_a_replay_sends_the_first_prompt() {
        let mut zone = Zone::new(Some("run the tests"));
        zone.shared.replay_start();
        zone.shared.apply(&[agent_state(AgentState::Starting).to_value()]).unwrap();
        zone.shared.replay_end();
        zone.shared.apply(&[ready().to_value()]).unwrap();
        assert_eq!(zone.absorb().first, Some(First::Send("run the tests".into())));
    }

    // Section 3: the session handle (decisions 1 and 2).

    /// D-1: the agent is ready before the session's thread hands over the
    /// session; the first prompt goes only once it runs.
    #[test]
    fn a_first_prompt_waits_for_the_session_to_run() {
        let mut zone = Zone::starting(Some("run the tests"));
        assert_eq!(zone.feed(&[ready()]).first, None, "the session isn't running yet");
        assert_eq!(zone.model.first_shown(), None);
        assert_eq!(zone.model.session_running().first, Some(First::Send("run the tests".into())));
        assert_eq!(zone.feed(&[ready()]).first, None, "once");
    }

    #[test]
    fn the_input_says_connecting_until_the_session_runs() {
        let mut zone = Zone::starting(None);
        zone.feed(&[ready()]);
        assert_eq!(zone.model.prompt_status(), PromptStatus::Disabled(DisabledWhy::Connecting));
        assert_eq!(zone.model.submit("hi", false), Submit::Nothing(Why::NotReady(DisabledWhy::Connecting)));
        assert!(!zone.model.can_choose());
        zone.model.session_running();
        assert_eq!(zone.model.prompt_status(), PromptStatus::Ready);
        assert_eq!(zone.model.submit("hi", false), Submit::Send("hi".into()));
        zone.model.respawning();
        zone.feed(&[ready()]);
        assert_eq!(zone.model.prompt_status(), PromptStatus::Disabled(DisabledWhy::Connecting), "a respawn starts again");
    }

    /// Decision 1's table, one row per cell.
    #[test]
    fn each_request_that_isnt_sent_does_what_its_case_says() {
        // Starting: nothing is refused.
        let mut zone = Zone::starting(Some("go"));
        let prompt = zone.model.sent_prompt("hi", false);
        assert_eq!(zone.model.not_sent(prompt, NotSent::Starting), Replied::Nothing);
        assert_eq!(zone.model.refusal(), None);
        zone.feed(&[ready()]);
        zone.model.session_running();
        let first = zone.model.sent_prompt("go", true);
        assert_eq!(zone.model.not_sent(first, NotSent::Starting), Replied::Nothing, "it waits again");
        assert_eq!(zone.model.refusal(), None);
        assert_eq!(zone.feed(&[ready()]).first, Some(First::Send("go".into())), "and goes on the next change");
        assert_eq!(
            zone.model.not_sent(Sent::List(ListFor::Palette), NotSent::Starting),
            Replied::Listed(ListDone::Palette(Err(STARTING.into())))
        );
        for sent in [Sent::Resend, Sent::Unqueue, Sent::Answer(EntryId(1)), Sent::Login, Sent::Choose, Sent::SetOption { name: "Model".into() }] {
            assert_eq!(zone.model.not_sent(sent.clone(), NotSent::Starting), Replied::Nothing, "{sent:?}");
            assert_eq!(zone.model.refusal(), None, "{sent:?}");
        }
        let mut last =
            Zone { shared: AgentShared::new(), model: ChatModel::choosing("Fake", None, Some(LaunchChoice::Last), "~/src/app") };
        last.model.session_running();
        assert_eq!(last.feed(&[choosing(None)]).choosing, Some(OnChoosing::ListLatest));
        assert_eq!(last.model.not_sent(Sent::List(ListFor::Latest), NotSent::Starting), Replied::Nothing);
        assert_eq!(last.feed(&[choosing(None)]).choosing, Some(OnChoosing::ListLatest), "asked again");
        let server = long_log(40);
        let mut paged_zone = paged(&server);
        let before = paged_zone.model.earlier();
        assert_eq!(paged_zone.model.not_sent(Sent::Fetch, NotSent::Starting), Replied::Nothing);
        assert_eq!(paged_zone.model.earlier(), before, "a fetch waits for the row");

        // Gone: refused with UNSENT, a first prompt becoming the draft.
        let mut gone = Zone::ready();
        let first = gone.model.sent_prompt("go", true);
        assert_eq!(gone.model.not_sent(first, NotSent::Gone), Replied::SetDraft("go".into()));
        assert_eq!(gone.model.refusal(), Some(UNSENT));
        let Earlier::Available { count } = before else { panic!("{before:?}") };
        paged_zone.model.not_sent(Sent::Fetch, NotSent::Gone);
        assert_eq!(paged_zone.model.earlier(), Earlier::Failed { count, why: UNSENT.into() });

        // LinkDown: a fetch fails with it, and the rest are refused with it.
        let mut down = paged(&server);
        down.model.not_sent(Sent::Fetch, NotSent::LinkDown);
        assert_eq!(down.model.earlier(), Earlier::Failed { count, why: LINK_DOWN.into() });
        let prompt = down.model.sent_prompt("hi", false);
        assert_eq!(down.model.not_sent(prompt, NotSent::LinkDown), Replied::Nothing);
        assert_eq!(down.model.refusal(), Some(LINK_DOWN));
    }

    // Section 3: a prompt whose answer was lost (decision 5).

    /// Sends prompt `text` from `zone` and loses its answer with the link,
    /// which then comes back.
    fn lose_prompt(zone: &mut Zone, text: &str, first: bool) {
        zone.model.sent(7, zone.model.sent_prompt(text, first));
        let replied = zone.model.replied(7, Err(ReplyError::Unanswered("the terminal server went away".into())));
        assert_eq!((replied, zone.model.refusal()), (Replied::Nothing, None), "not refused yet");
        assert_eq!(zone.model.prompt_status(), PromptStatus::Submitted, "still in flight");
        zone.model.set_link(Link::Reconnecting);
        zone.model.set_link(Link::Up);
    }

    #[test]
    fn a_prompt_lost_with_the_link_and_queued_in_the_slot_clears_its_draft() {
        let mut zone = Zone::ready();
        zone.feed(&[added(1, user("hi")), turn(TurnState::Working)]);
        lose_prompt(&mut zone, "next", false);
        let absorbed = zone.replay(&[ready(), added(1, user("hi")), turn(TurnState::Working), queue(Slot::waiting("next"))]);
        assert_eq!((absorbed.replied, zone.model.refusal()), (Some(Replied::ClearDraft), None));
    }

    #[test]
    fn a_prompt_lost_with_the_link_and_missing_from_the_replay_is_refused_then() {
        let mut zone = Zone::ready();
        zone.feed(&[added(1, user("hi")), added(2, agent("Hello!"))]);
        lose_prompt(&mut zone, "next", false);
        let absorbed = zone.replay(&[ready(), added(1, user("hi")), added(2, agent("Hello!"))]);
        assert_eq!((absorbed.replied, zone.model.refusal()), (Some(Replied::Nothing), Some(PROMPT_NOT_TAKEN)));
        assert_eq!(zone.model.prompt_status(), PromptStatus::Ready, "settled");
        assert_eq!(zone.feed(&[added(3, agent("more"))]).replied, None, "once");
    }

    /// The user's entry, wherever it went, would be before the replayed
    /// page: the zone can't tell.
    #[test]
    fn a_prompt_lost_with_the_link_past_the_replayed_page_may_have_been_sent() {
        let mut server = long_log(40);
        let mut zone = paged(&server);
        lose_prompt(&mut zone, "next", false);
        for id in 42..=81 {
            server.apply(&added(id, agent(&format!("**{id}** {}", "y".repeat(20 * 1024))))).unwrap();
        }
        let absorbed = zone.replay(&server.page(PAGE_BYTES));
        assert!(zone.shared.transcript().window().is_some_and(|window| window.from.0 > 42));
        assert_eq!((absorbed.replied, zone.model.refusal()), (Some(Replied::Nothing), Some(PROMPT_MAYBE_TAKEN)));
    }

    /// The reducer cleans a prompt's text before logging it; the search
    /// matches the cleaned text.
    #[test]
    fn a_cleaned_prompt_is_found_in_the_replay() {
        let mut zone = Zone::ready();
        lose_prompt(&mut zone, "ring \u{7} me", false);
        let absorbed = zone.replay(&[ready(), added(1, user(&crate::agent::transcript::clean("ring \u{7} me")))]);
        assert_eq!((absorbed.replied, zone.model.refusal()), (Some(Replied::ClearDraft), None));
    }

    #[test]
    fn a_first_prompt_lost_with_the_link_becomes_the_draft_only_when_not_taken() {
        let mut taken = Zone::new(Some("go"));
        assert_eq!(taken.feed(&[ready()]).first, Some(First::Send("go".into())));
        lose_prompt(&mut taken, "go", true);
        let absorbed = taken.replay(&[ready(), added(1, user("go"))]);
        assert_eq!((absorbed.replied, taken.model.refusal()), (Some(Replied::Nothing), None));

        let mut lost = Zone::new(Some("go"));
        lost.feed(&[ready()]);
        lose_prompt(&mut lost, "go", true);
        let absorbed = lost.replay(&[ready()]);
        assert_eq!((absorbed.replied, lost.model.refusal()), (Some(Replied::SetDraft("go".into())), Some(PROMPT_NOT_TAKEN)));

        // Nothing will replay: the zone can't tell, and the text is kept.
        let mut ended = Zone::new(Some("go"));
        ended.feed(&[ready()]);
        lose_prompt(&mut ended, "go", true);
        let absorbed = ended.model.lost("the holder went away");
        assert_eq!((absorbed.replied, ended.model.refusal()), (Some(Replied::SetDraft("go".into())), Some(PROMPT_MAYBE_TAKEN)));
    }

    #[test]
    fn a_failed_reply_is_still_a_refusal_at_once() {
        let mut zone = Zone::ready();
        zone.model.sent(7, zone.model.sent_prompt("next", false));
        assert_eq!(zone.model.replied(7, Err(ReplyError::Failed("the agent isn't ready".into()))), Replied::Nothing);
        assert_eq!(zone.model.refusal(), Some("the agent isn't ready"));
        assert_eq!(zone.model.prompt_status(), PromptStatus::Ready, "nothing waits");
        assert_eq!(zone.replay(&[ready()]).replied, None);
        zone.model.sent(8, zone.model.sent_prompt("go", true));
        assert_eq!(zone.model.replied(8, Err(ReplyError::Failed("no".into()))), Replied::SetDraft("go".into()));
    }
}
