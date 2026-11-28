//! The choosing screen and the conversation lists (OpenSpec change
//! `agents-session-list`): the zone's choices, a launch's list and a choice.

use crate::FirstPrompt;
use crate::agent::conversations::{Latest, Page, ZoneRun, latest_plan};
use crate::agent::transcript::AgentState;

use super::{ChatModel, RequestId, SessionHandle};
use super::ledger::{STARTING, Sent};
use super::paging::no_earlier_text;
use super::phase::{AgentPhase, Link, SessionLife};
use super::queue::{CHOOSING, without_session};
use super::recovery::{NoOffer, Offer};

/// A choice made while the link is down.
pub const CHOICE_LINK_DOWN: &str = "the connection to the terminal server is down; the choice wasn't sent";

/// A choice made while another is on its way.
pub const CHOICE_PENDING: &str = "a choice is already on its way";

/// A choice made when the agent isn't choosing.
pub const NOT_CHOOSING: &str = "the agent isn't choosing a conversation";

/// What a choice does now ([`ChatModel::choose`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Choose {
    /// Send `AGENT_CHOOSE`.
    Send,
    /// Nothing is sent: the choosing screen says why.
    Refused,
}

/// What a list the zone's agent answers is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListFor {
    /// The conversations palette: the page or the refusal is handed to it.
    Palette,
    /// A `-c` launch: the latest is chosen ([`latest_plan`]).
    Latest,
}

/// What a launch asks of its choosing zone (design decision 8).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchChoice {
    /// `-c`: the latest conversation, else the choosing screen.
    Last,
    /// `-r`: the conversations palette.
    List,
}

/// What the zone does, once, when its launch's agent is first choosing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnChoosing {
    /// Ask for the first page, for the latest (`-c`).
    ListLatest,
    /// Open the conversations palette on the zone (`-r`).
    OpenList,
}

/// What a list's answer asks of the zone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListDone {
    /// The palette's page, or why there is none: handed to the palette.
    Palette(Result<Page, String>),
    /// The latest conversation, to choose (`AGENT_CHOOSE`).
    Choose(String),
    Nothing,
}

/// What a quit does to a zone's session ([`ChatModel::leave_on_quit`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Leaving {
    /// The session ends.
    Close,
    /// The session is left running, detached with its tab's layout.
    Detach,
}

/// Where the agent a conversations palette lists through is
/// ([`ChatModel::lister_state`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ListerState {
    Starting,
    /// Choosing, or ready: it can be asked for a list.
    Lists,
    /// It can't list, saying why.
    Failed(String),
}

/// A button of a pane that has no conversation running (design decision
/// 6), each a keyboard stop on the reason line: Continue conversation,
/// Conversations…, New conversation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneChoice {
    Continue,
    Conversations,
    New,
}

impl ZoneChoice {
    /// The button's label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Continue => "Continue conversation",
            Self::Conversations => "Conversations…",
            Self::New => "New conversation",
        }
    }

    /// The stop's name in the dump's focus line, and the prompt line's
    /// `actions=`.
    pub fn word(self) -> &'static str {
        match self {
            Self::Continue => "continue",
            Self::Conversations => "conversations",
            Self::New => "new",
        }
    }
}

/// A zone's facts about choosing a conversation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Choosing {
    /// The session was started to choose, and no conversation has started
    /// in it yet: session restore leaves it out, and a quit ends it.
    pub(super) started: bool,
    /// What the launch asked for, until the agent is first choosing.
    pub(super) launch: Option<LaunchChoice>,
    /// The directory as shown, for the reasons that name it.
    dir: String,
    /// The reason a list left: none to continue, or its refusal.
    pub(super) note: Option<String>,
    /// A list was refused because the agent can't list.
    unlisted: bool,
}

impl ChatModel {
    /// A zone whose session starts choosing (OpenSpec change
    /// `agents-session-list`), in the directory shown as `dir`, for a
    /// launch's `launch` choice and `first` prompt.
    pub fn choosing(
        profile: impl Into<String>,
        first: Option<FirstPrompt>,
        launch: Option<LaunchChoice>,
        dir: impl Into<String>,
    ) -> Self {
        let mut model = Self::new(profile, first);
        model.choosing = Choosing { started: true, launch, dir: dir.into(), note: None, unlisted: false };
        model
    }

    /// The zone's session was replaced by one starting a new conversation,
    /// or another listed one: the conversation it knew is no longer its
    /// own, so nothing continues it or titles the zone.
    pub fn forget_conversation(&mut self) {
        self.conversation = None;
    }

    /// Whether the agent is choosing on an open session.
    pub fn is_choosing(&self) -> bool {
        matches!(self.phase, AgentPhase::Live(AgentState::Choosing { .. })) && self.life == SessionLife::Open
    }

    /// Whether the session was started to choose and no conversation has
    /// started in it.
    pub fn started_choosing(&self) -> bool {
        self.choosing.started
    }

    /// What a quit does to the zone's session: a session still choosing
    /// has nothing to bring back, so it ends (OpenSpec change
    /// `agents-session-list` decision 12); any other is left detached.
    pub fn leave_on_quit(&self) -> Leaving {
        if self.started_choosing() || self.is_choosing() { Leaving::Close } else { Leaving::Detach }
    }

    /// The reason a choosing zone shows: the agent's message for a refused
    /// choice, else a list's (none to continue, a refusal), else the
    /// screen's own.
    pub fn choosing_reason(&self) -> String {
        match &self.phase {
            AgentPhase::Live(AgentState::Choosing { message: Some(message) }) => message.clone(),
            AgentPhase::Live(_) | AgentPhase::Connecting | AgentPhase::NotStarted(_) | AgentPhase::Lost { .. } | AgentPhase::Broken { .. } => {
                self.choosing.note.clone().unwrap_or_else(|| CHOOSING.into())
            }
        }
    }

    /// The pane's buttons (design decision 6) for the continue `offer`: a
    /// choosing agent's Conversations… (unless it can't list) and New
    /// conversation; a stopped agent's Continue conversation when it can,
    /// then Conversations… and New conversation; none while an agent
    /// runs, or once the zone's profile is gone.
    pub fn zone_choices(&self, offer: &Offer) -> Vec<ZoneChoice> {
        let list = |unlisted: bool| if unlisted { vec![ZoneChoice::New] } else { vec![ZoneChoice::Conversations, ZoneChoice::New] };
        if self.is_choosing() {
            return list(self.choosing.unlisted);
        }
        match offer {
            Offer::Restart | Offer::Respawn { .. } => [vec![ZoneChoice::Continue], list(self.choosing.unlisted)].concat(),
            // An agent that never started keeps its own reason, alone.
            Offer::None(NoOffer::NoConversation) if matches!(self.phase, AgentPhase::NotStarted(_)) => Vec::new(),
            Offer::None(NoOffer::NoConversation | NoOffer::NotOffered) => list(self.choosing.unlisted),
            Offer::None(NoOffer::ProfileGone { .. }) => Vec::new(),
            // A choosing session that ended is stopped too.
            Offer::None(NoOffer::NotEnded) if matches!(self.phase, AgentPhase::Live(AgentState::Choosing { .. })) => {
                list(self.choosing.unlisted)
            }
            Offer::None(NoOffer::NotEnded) => Vec::new(),
        }
    }

    /// Whether `AGENT_CHOOSE` can go now: the agent is choosing on a link
    /// that is up, with no choice on its way ([`Self::choose`] decides).
    #[cfg(test)]
    pub fn can_choose(&self) -> bool {
        self.is_choosing()
            && self.link == Link::Up
            && self.handle == SessionHandle::Running
            && !self.pending.values().any(|sent| *sent == Sent::Choose)
    }

    /// A conversation was chosen (decision 10): send it when it can go;
    /// otherwise the choosing screen says why, as its reason.
    pub fn choose(&mut self) -> Choose {
        let why = if !self.is_choosing() {
            NOT_CHOOSING
        } else if self.link == Link::Reconnecting {
            CHOICE_LINK_DOWN
        } else if self.pending.values().any(|sent| *sent == Sent::Choose) {
            CHOICE_PENDING
        } else if self.handle == SessionHandle::Starting {
            STARTING
        } else {
            self.choosing.note = None;
            return Choose::Send;
        };
        self.choosing.note = Some(why.into());
        Choose::Refused
    }

    /// An `AGENT_CHOOSE` went out as `request`: the reason a list left goes.
    pub fn chose(&mut self, request: RequestId) {
        self.choosing.note = None;
        self.refusal = None;
        self.pending.insert(request, Sent::Choose);
    }

    /// Whether a list can be asked for now: the agent is ready or choosing
    /// on a link that is up.
    #[cfg(test)]
    pub fn can_list(&self) -> bool {
        let live = matches!(self.phase, AgentPhase::Live(AgentState::Ready { .. } | AgentState::Choosing { .. }));
        live && self.life == SessionLife::Open && self.link == Link::Up
    }

    /// `AGENT_CONVERSATIONS` answered the list `request` with `page`.
    pub fn listed(&mut self, request: RequestId, page: Page) -> ListDone {
        match self.pending.remove(&request) {
            Some(Sent::List(ListFor::Palette)) => ListDone::Palette(Ok(page)),
            Some(Sent::List(ListFor::Latest)) => match latest_plan(&page.conversations) {
                Latest::Continue(id) => ListDone::Choose(id),
                Latest::None => {
                    self.choosing.note = Some(no_earlier_text(&self.choosing.dir));
                    ListDone::Nothing
                }
            },
            Some(other) => {
                // Not a list: kept for its own reply.
                self.pending.insert(request, other);
                ListDone::Nothing
            }
            None => ListDone::Nothing,
        }
    }

    /// A list for `purpose` was refused with `why` (or couldn't go): the
    /// palette's is handed on; a launch's becomes the screen's reason. A
    /// list the agent can't answer at all marks it.
    pub(super) fn list_refused(&mut self, purpose: ListFor, why: String) -> ListDone {
        let why = without_session(&why).to_owned();
        if why == crate::remote::protocol::client::LIST_NOT_OFFERED {
            self.choosing.unlisted = true;
        }
        match purpose {
            ListFor::Palette => ListDone::Palette(Err(why)),
            ListFor::Latest => {
                self.choosing.note = Some(why);
                ListDone::Nothing
            }
        }
    }

    /// What the conversations plans see the zone's agent do (OpenSpec
    /// change `agents-session-list` decisions 3 and 5).
    pub fn zone_run(&self) -> ZoneRun {
        match &self.phase {
            AgentPhase::Live(AgentState::Ready { .. }) => match (self.life, self.link) {
                (SessionLife::Open, Link::Up) => ZoneRun::Ready,
                (SessionLife::Open, Link::Reconnecting) => ZoneRun::Busy,
                (SessionLife::Ended, _) => ZoneRun::Stopped,
            },
            AgentPhase::Live(AgentState::Choosing { .. }) => ZoneRun::Choosing,
            AgentPhase::NotStarted(_)
            | AgentPhase::Lost { .. }
            | AgentPhase::Broken { .. }
            | AgentPhase::Live(AgentState::Exited { .. } | AgentState::Lost { .. }) => ZoneRun::Stopped,
            AgentPhase::Connecting
            | AgentPhase::Live(
                AgentState::Starting
                | AgentState::LoginRequired { .. }
                | AgentState::LoggingIn { .. }
                | AgentState::Failed { .. },
            ) => ZoneRun::Busy,
        }
    }

    /// Where an agent the conversations palette lists through is: starting,
    /// able to list, or unable to, and why (decision 3: a login needed, or
    /// the failure's text). `command` and `endpoint` are what the alert
    /// names.
    pub fn lister_state(&self, command: &str, endpoint: &str) -> ListerState {
        match &self.phase {
            AgentPhase::Connecting | AgentPhase::Live(AgentState::Starting) => ListerState::Starting,
            AgentPhase::Live(AgentState::Choosing { .. } | AgentState::Ready { .. }) if self.life == SessionLife::Open => {
                ListerState::Lists
            }
            AgentPhase::Live(AgentState::LoginRequired { .. } | AgentState::LoggingIn { .. }) => {
                ListerState::Failed(format!("{} needs a login: open the profile to log in", self.profile))
            }
            AgentPhase::Live(AgentState::Choosing { .. } | AgentState::Ready { .. })
            | AgentPhase::NotStarted(_)
            | AgentPhase::Lost { .. }
            | AgentPhase::Broken { .. }
            | AgentPhase::Live(AgentState::Failed { .. } | AgentState::Exited { .. } | AgentState::Lost { .. }) => {
                let text = self.alert(command, endpoint).map(|alert| alert.text);
                ListerState::Failed(text.unwrap_or_else(|| format!("{} can't list conversations", self.profile)))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::agent::shared::AgentShared;
    use super::super::prompt::Why;

    use super::super::harness::*;
    use super::super::*;
    use super::*;

    /// A zone started to choose, for a launch's `launch`, in `~/src/app`.
    fn chooser(launch: Option<LaunchChoice>) -> Zone {
        let mut zone = Zone { shared: AgentShared::new(), model: ChatModel::choosing("Fake", None, launch, "~/src/app") };
        zone.model.session_running();
        zone
    }

    fn page(ids: &[(&str, Option<&str>)]) -> Page {
        let conversations = ids
            .iter()
            .map(|(id, updated)| crate::agent::conversations::Listed {
                id: (*id).into(),
                title: None,
                updated: updated.map(Into::into),
            })
            .collect();
        Page { conversations, next: None }
    }

    /// Decision 6's table: each row's reason and buttons; a gone profile
    /// offers none; a running agent none.
    #[test]
    fn the_choosing_screens_table() {
        use crate::agent::transcript::Continuation;
        use ZoneChoice::{Continue, Conversations, New};
        let mut zone = chooser(None);
        zone.feed(&[choosing(None)]);
        let offer = found(&zone.model);
        assert_eq!(offer, Offer::None(NoOffer::NotEnded));
        assert_eq!(zone.model.zone_choices(&offer), [Conversations, New]);
        assert_eq!(zone.model.choosing_reason(), CHOOSING);
        assert_eq!(zone.model.prompt_status(), PromptStatus::Disabled(DisabledWhy::Choosing));
        // A refused choice's message is the reason.
        zone.feed(&[choosing(Some("no such session"))]);
        assert_eq!(zone.model.choosing_reason(), "no such session");
        // The agent can't list: New conversation alone, with its reason.
        let request = 5;
        zone.model.sent(request, Sent::List(ListFor::Latest));
        let replied = zone.model.replied(request, Err(crate::remote::protocol::client::LIST_NOT_OFFERED.into()));
        assert_eq!(replied, Replied::Listed(ListDone::Nothing));
        zone.feed(&[choosing(None)]);
        assert_eq!(zone.model.zone_choices(&found(&zone.model)), [New]);
        assert_eq!(zone.model.choosing_reason(), "the agent can't list conversations");

        // Exited, can continue; can't continue; the profile gone.
        let mut ended = Zone::new(None);
        ended.feed(&[conversation_event(Continuation::Resume), ready(), agent_state(AgentState::Exited { code: Some(1), stderr: String::new() })]);
        assert_eq!(ended.model.zone_choices(&found(&ended.model)), [Continue, Conversations, New]);
        let gone = ended.model.offer(&ProfileFound::Gone { name: "Fake".into() });
        assert_eq!(ended.model.zone_choices(&gone), []);
        let mut neither = Zone::new(None);
        neither.feed(&[conversation_event(Continuation::Neither), ready(), agent_state(AgentState::Lost { message: "gone".into() })]);
        assert_eq!(neither.model.zone_choices(&found(&neither.model)), [Conversations, New]);
        assert_eq!(Zone::ready().model.zone_choices(&found(&Zone::ready().model)), []);
    }

    /// While choosing: no working row, no status asking for attention, no
    /// prompt sent, and `state=choosing` with its reason in the dump; the
    /// session isn't kept for restore until a conversation starts.
    #[test]
    fn a_choosing_zone_waits_quietly() {
        let mut zone = chooser(None);
        assert!(!zone.model.keeps_session(), "started to choose");
        zone.feed(&[agent_state(AgentState::Starting), choosing(None)]);
        assert_eq!(zone.model.working_row(), None);
        assert!(zone.model.shown_status().is_none_or(|state| state == State::Idle));
        assert_eq!(zone.model.submit("hi", false), Submit::Nothing(Why::NotReady(DisabledWhy::Choosing)));
        assert!(!zone.model.keeps_session());
        let dump = zone.model.describe(1, true, &found(&zone.model), &|_| unreachable!());
        assert!(dump.contains("state=choosing"), "{dump}");
        assert!(dump.contains(&format!("--- prompt status=disabled reason={CHOOSING:?} actions=conversations,new\n")), "{dump}");
        zone.feed(&[ready()]);
        assert!(zone.model.keeps_session(), "a conversation started");
        assert!(!zone.model.started_choosing());
    }

    /// `-c`: the first time the agent chooses, the zone lists for the
    /// latest, once; the newest is chosen, or the screen says there is none
    /// in the directory. `-r` opens the list; nothing from a replay.
    #[test]
    fn a_launch_lists_for_the_latest_or_opens_the_list() {
        let mut last = chooser(Some(LaunchChoice::Last));
        assert_eq!(last.feed(&[agent_state(AgentState::Starting)]).choosing, None);
        assert_eq!(last.feed(&[choosing(None)]).choosing, Some(OnChoosing::ListLatest));
        assert_eq!(last.feed(&[choosing(None)]).choosing, None, "once");
        assert!(last.model.can_list());
        last.model.sent(3, Sent::List(ListFor::Latest));
        let newest = page(&[("old", Some("2026-11-01T00:00:00Z")), ("new", Some("2026-11-02T00:00:00Z"))]);
        assert_eq!(last.model.listed(3, newest), ListDone::Choose("new".into()));
        last.model.sent(4, Sent::List(ListFor::Latest));
        assert_eq!(last.model.listed(4, page(&[])), ListDone::Nothing);
        assert_eq!(last.model.choosing_reason(), "no earlier conversation in ~/src/app");
        assert_eq!(last.model.listed(4, page(&[])), ListDone::Nothing, "answered already");

        let mut list = chooser(Some(LaunchChoice::List));
        assert_eq!(list.replay(&[choosing(None)]).choosing, None, "not from a replay");
        assert_eq!(list.feed(&[choosing(None)]).choosing, Some(OnChoosing::OpenList));
    }

    /// What the conversations plans see of a zone, and whether a palette
    /// can list through it.
    #[test]
    fn a_zones_run_and_its_listers_state() {
        let mut zone = chooser(None);
        assert_eq!((zone.model.zone_run(), zone.model.lister_state("f", "A8")), (ZoneRun::Busy, ListerState::Starting));
        zone.feed(&[choosing(None)]);
        assert_eq!((zone.model.zone_run(), zone.model.lister_state("f", "A8")), (ZoneRun::Choosing, ListerState::Lists));
        let ready = Zone::ready();
        assert_eq!((ready.model.zone_run(), ready.model.lister_state("f", "A8")), (ZoneRun::Ready, ListerState::Lists));
        let mut login = chooser(None);
        login.feed(&[agent_state(AgentState::LoginRequired { message: "log in".into(), methods: Vec::new() })]);
        assert_eq!(login.model.zone_run(), ZoneRun::Busy);
        assert_eq!(
            login.model.lister_state("f", "A8"),
            ListerState::Failed("Fake needs a login: open the profile to log in".into())
        );
        let mut exited = chooser(None);
        exited.feed(&[agent_state(AgentState::Exited { code: Some(2), stderr: String::new() })]);
        assert_eq!(exited.model.zone_run(), ZoneRun::Stopped);
        assert_eq!(exited.model.lister_state("f", "A8"), ListerState::Failed("Fake exited with status 2".into()));
    }

    /// A list's refusal loses the core's `session N: `, and the agent that
    /// can't list is marked so from it.
    #[test]
    fn a_lists_refusal_keeps_the_agents_words() {
        assert_eq!(without_session("session 12: the index is locked"), "the index is locked");
        assert_eq!(without_session("session x: kept"), "session x: kept");
        assert_eq!(without_session("session : kept"), "session : kept");
        assert_eq!(without_session("no prefix: kept"), "no prefix: kept");
        let mut zone = chooser(None);
        zone.feed(&[choosing(None)]);
        zone.model.sent(7, Sent::List(ListFor::Palette));
        let why = format!("session 3: {}", crate::remote::protocol::client::LIST_NOT_OFFERED);
        let replied = zone.model.replied(7, Err(ReplyError::Failed(why)));
        let words = crate::remote::protocol::client::LIST_NOT_OFFERED;
        assert_eq!(replied, Replied::Listed(ListDone::Palette(Err(words.into()))));
        // Marked: New conversation alone.
        assert_eq!(zone.model.zone_choices(&found(&zone.model)), [ZoneChoice::New]);
    }

    /// The palette's list is handed on, page or refusal; a choice can go
    /// once while choosing, and its refusal shows.
    #[test]
    fn a_palettes_list_and_a_choice() {
        let mut zone = chooser(None);
        zone.feed(&[choosing(None)]);
        zone.model.sent(7, Sent::List(ListFor::Palette));
        assert_eq!(zone.model.listed(7, page(&[("a", None)])), ListDone::Palette(Ok(page(&[("a", None)]))));
        zone.model.sent(8, Sent::List(ListFor::Palette));
        assert_eq!(zone.model.replied(8, Err("the agent's error".into())), Replied::Listed(ListDone::Palette(Err("the agent's error".into()))));
        assert!(zone.model.can_choose());
        zone.model.chose(9);
        assert!(!zone.model.can_choose(), "one choice on its way");
        assert_eq!(zone.model.replied(9, Err("the agent isn't choosing".into())), Replied::Nothing);
        assert_eq!(zone.model.refusal(), Some("the agent isn't choosing"));
        assert!(zone.model.can_choose());
        assert!(!Zone::ready().model.can_choose(), "a ready agent doesn't choose");
        assert!(Zone::ready().model.can_list());
    }

    /// A launch's `-c` list waits for the running session too.
    #[test]
    fn a_launchs_list_waits_for_the_session_to_run() {
        let mut last =
            Zone { shared: AgentShared::new(), model: ChatModel::choosing("Fake", None, Some(LaunchChoice::Last), "~/src/app") };
        assert_eq!(last.feed(&[choosing(None)]).choosing, None, "not before the session runs");
        assert_eq!(last.model.session_running().choosing, Some(OnChoosing::ListLatest));
        assert_eq!(last.feed(&[choosing(None)]).choosing, None, "once");
    }

    // Section 3: a choice (decision 10).

    #[test]
    fn a_choice_that_cant_go_says_why() {
        let mut ready = Zone::ready();
        assert_eq!(ready.model.choose(), Choose::Refused, "not choosing");

        let mut zone = chooser(None);
        zone.feed(&[choosing(None)]);
        zone.model.set_link(Link::Reconnecting);
        assert_eq!(zone.model.choose(), Choose::Refused);
        assert_eq!(zone.model.choosing_reason(), CHOICE_LINK_DOWN);
        zone.model.set_link(Link::Up);
        assert_eq!(zone.model.choose(), Choose::Send);
        assert_eq!(zone.model.choosing_reason(), CHOOSING, "the reason goes");
        zone.model.chose(9);
        assert_eq!(zone.model.choose(), Choose::Refused);
        assert_eq!(zone.model.choosing_reason(), CHOICE_PENDING);

        let mut starting =
            Zone { shared: AgentShared::new(), model: ChatModel::choosing("Fake", None, None, "~/src/app") };
        starting.feed(&[choosing(None)]);
        assert_eq!(starting.model.choose(), Choose::Refused);
        assert_eq!(starting.model.choosing_reason(), STARTING);
        starting.model.session_running();
        assert_eq!(starting.model.choose(), Choose::Send);
    }

    /// `agents-session-list` decision 12: a quit ends a session started to
    /// choose, and one whose agent is choosing again; any other is left
    /// detached.
    #[test]
    fn a_zone_still_choosing_closes_its_session_on_quit_and_others_detach() {
        let mut zone = chooser(None);
        assert_eq!(zone.model.leave_on_quit(), Leaving::Close, "started to choose");
        zone.feed(&[agent_state(AgentState::Starting), choosing(None)]);
        assert_eq!(zone.model.leave_on_quit(), Leaving::Close, "still choosing");
        zone.feed(&[ready()]);
        assert_eq!(zone.model.leave_on_quit(), Leaving::Detach, "a conversation started");
        zone.feed(&[choosing(None)]);
        assert_eq!(zone.model.leave_on_quit(), Leaving::Close, "choosing again");
        assert_eq!(Zone::ready().model.leave_on_quit(), Leaving::Detach);
    }
}
