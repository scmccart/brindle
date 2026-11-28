//! What asks for the user's attention: the shown status and the seen
//! rule, the working row, permissions, notices and the banner.

use gpui::SharedString;

use crate::agent::close::AgentBusy;
use crate::agent::transcript::{
    AgentState, Entry, EntryId, PermissionOption, ToolKind, ToolStatus, TurnState, status_of,
};
use crate::terminal::status::State;

use super::ChatModel;
use super::blocks::{BlockBody, PermissionButton, buttons};
use super::phase::{AgentPhase, Link, SessionLife};
use super::queue::stop_word;

/// What the working row at the end of the conversation says while a turn
/// runs (decision 2): it isn't an entry and is never copied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkingRow {
    /// `Working…`.
    Working,
    /// The title of the turn's newest tool call in progress.
    Tool { title: SharedString },
    /// `Stopping…`: a cancel is on its way.
    Stopping,
}

impl WorkingRow {
    /// The row's words.
    pub fn text(&self) -> SharedString {
        match self {
            Self::Working => "Working…".into(),
            Self::Tool { title } => title.clone(),
            Self::Stopping => "Stopping…".into(),
        }
    }
}

/// The word a permission notification puts before the request's title for
/// its tool call's kind (decision 6); none for `other`.
pub fn kind_word(kind: ToolKind) -> Option<&'static str> {
    match kind {
        ToolKind::Read => Some("Read"),
        ToolKind::Edit => Some("Edit"),
        ToolKind::Delete => Some("Delete"),
        ToolKind::Move => Some("Move"),
        ToolKind::Search => Some("Search"),
        ToolKind::Execute => Some("Command"),
        ToolKind::Think => Some("Think"),
        ToolKind::Fetch => Some("Fetch"),
        ToolKind::SwitchMode => Some("Mode"),
        ToolKind::Other => None,
    }
}

/// An agent notification's title (decision 6): the profile's name, then
/// ` needs permission` for a request (`state` blocked), then
/// ` — <zone title>` when the zone's title isn't the profile's name.
pub fn notice_title(profile: &str, zone_title: &str, state: State) -> String {
    let mut title = profile.to_string();
    match state {
        State::Blocked => title.push_str(" needs permission"),
        State::Done | State::Error | State::Idle | State::Working => {}
    }
    if !zone_title.trim().is_empty() && zone_title.trim() != profile.trim() {
        title.push_str(" — ");
        title.push_str(zone_title);
    }
    title
}

/// A permission notification's body (decision 6): the request's title,
/// after its tool call's kind word and a colon when `kind` has one; then,
/// on a second line, the options' names in the row's order joined by
/// ` · `, ending ` · +N more` for the `more` other requests waiting.
pub fn permission_body(title: &str, kind: Option<ToolKind>, options: &[PermissionButton], more: usize) -> String {
    let first = match kind.and_then(kind_word) {
        Some(word) => format!("{word}: {title}"),
        None => title.to_string(),
    };
    let mut second: Vec<String> = options.iter().map(|o| o.name.to_string()).collect();
    if more > 0 {
        second.push(format!("+{more} more"));
    }
    if second.is_empty() { first } else { format!("{first}\n{}", second.join(" · ")) }
}

/// A desktop notification an agent zone raises (decision 10).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentNotice {
    pub title: String,
    pub body: String,
}

impl ChatModel {
    /// The status the zone shows (decision 10): its activity's, while the
    /// phase is live; none outside it; a `done` or `error` the user has
    /// seen by typing is dropped until the activity changes.
    pub fn shown_status(&self) -> Option<State> {
        match &self.phase {
            AgentPhase::Live(_) => {}
            AgentPhase::Connecting | AgentPhase::NotStarted(_) | AgentPhase::Lost { .. } | AgentPhase::Broken { .. } => {
                return None;
            }
        }
        let state = status_of(self.activity);
        match state {
            State::Done | State::Error if self.seen => None,
            State::Done | State::Error | State::Idle | State::Working | State::Blocked => Some(state),
        }
    }

    /// The user typed into the prompt input: a `done` or `error` showing
    /// is seen. The new shown status, when it changed.
    pub fn seen(&mut self) -> Option<Option<State>> {
        let before = self.shown_status();
        match status_of(self.activity) {
            State::Done | State::Error => self.seen = true,
            State::Idle | State::Working | State::Blocked => {}
        }
        let after = self.shown_status();
        (after != before).then_some(after)
    }

    /// The working row at the end of the conversation (decision 2): none
    /// unless the agent is ready on an open session; while the turn works,
    /// the title of its newest tool call in progress (after the latest
    /// user message), else `Working…`; `Stopping…` while a cancel is on
    /// its way; none while it waits on a permission or once it is over.
    pub fn working_row(&self) -> Option<WorkingRow> {
        match (&self.phase, self.life) {
            (AgentPhase::Live(AgentState::Ready { .. }), SessionLife::Open) => {}
            (
                AgentPhase::Live(
                    AgentState::Starting
                    | AgentState::LoginRequired { .. }
                    | AgentState::LoggingIn { .. }
                    | AgentState::Failed { .. }
                    | AgentState::Exited { .. }
                    | AgentState::Lost { .. }
                    | AgentState::Choosing { .. },
                )
                | AgentPhase::Connecting
                | AgentPhase::NotStarted(_)
                | AgentPhase::Lost { .. }
                | AgentPhase::Broken { .. },
                _,
            )
            | (AgentPhase::Live(AgentState::Ready { .. }), SessionLife::Ended) => return None,
        }
        match &self.turn {
            TurnState::Working => Some(self.tool_in_progress().map_or(WorkingRow::Working, |title| WorkingRow::Tool {
                title: title.to_string().into(),
            })),
            TurnState::Cancelling => Some(WorkingRow::Stopping),
            TurnState::Blocked | TurnState::Idle | TurnState::Ended { .. } | TurnState::Failed { .. } => None,
        }
    }

    /// The title of the newest tool call in progress after the latest user
    /// message.
    fn tool_in_progress(&self) -> Option<&str> {
        for (_, entry) in self.entries.iter().rev() {
            match entry {
                Entry::User { .. } => return None,
                Entry::Tool { title, status: ToolStatus::InProgress, .. } => return Some(title),
                Entry::Tool { .. }
                | Entry::Agent { .. }
                | Entry::Thought { .. }
                | Entry::Permission { .. }
                | Entry::Notice { .. } => {}
            }
        }
        None
    }

    /// Whether closing the zone asks first ([`crate::agent::close`]): a
    /// busy agent does, but a login running doesn't count, since choosing
    /// again redoes all it holds (decision 7).
    pub fn close_busy(&self) -> Option<AgentBusy> {
        if self.logging_in() { None } else { AgentBusy::of(self.shown_status()) }
    }

    /// Whether choosing `option` answers permission request `entry`: it
    /// must be pending and offer that option.
    pub fn answer(&self, entry: EntryId, option: &str) -> bool {
        let pending = self.entries.binary_search_by_key(&entry, |(id, _)| *id).ok().is_some_and(|index| {
            match &self.entries[index].1 {
                Entry::Permission { options, outcome: None, .. } => options.iter().any(|o| o.id == option),
                Entry::Permission { outcome: Some(_), .. }
                | Entry::User { .. }
                | Entry::Agent { .. }
                | Entry::Thought { .. }
                | Entry::Tool { .. }
                | Entry::Notice { .. } => false,
            }
        });
        pending && self.can_send()
    }

    /// The pending permission requests' buttons, in order, for the zone's
    /// focus stops.
    pub fn pending_permissions(&self) -> Vec<(EntryId, Vec<SharedString>)> {
        self.blocks
            .iter()
            .filter_map(|block| match &block.body {
                BlockBody::Permission { options, outcome: None, .. } => {
                    Some((block.entry, options.iter().map(|o| o.id.clone()).collect()))
                }
                BlockBody::Permission { outcome: Some(_), .. }
                | BlockBody::UserText { .. }
                | BlockBody::AgentPart { .. }
                | BlockBody::Thought { .. }
                | BlockBody::Tool { .. }
                | BlockBody::ToolDetail(_)
                | BlockBody::Notice { .. } => None,
            })
            .collect()
    }

    /// The reconnecting banner over the conversation.
    pub fn banner(&self, endpoint: &str) -> Option<String> {
        match self.link {
            Link::Up => None,
            Link::Reconnecting => Some(format!("reconnecting to {endpoint}…")),
        }
    }

    /// The words an `error` notification gives.
    fn error_text(&self) -> String {
        match &self.phase {
            AgentPhase::Live(
                AgentState::LoginRequired { message, .. }
                | AgentState::Failed { message }
                | AgentState::Lost { message },
            ) => message.clone(),
            AgentPhase::Live(AgentState::Exited { code: Some(code), .. }) => format!("exited with status {code}"),
            AgentPhase::Live(AgentState::Exited { code: None, .. }) => "ended by a signal".into(),
            AgentPhase::Live(
                AgentState::Starting | AgentState::Ready { .. } | AgentState::LoggingIn { .. } | AgentState::Choosing { .. },
            ) => match &self.turn
            {
                TurnState::Failed { message } => message.clone(),
                TurnState::Ended { stop } => format!("the turn ended: {}", stop_word(stop)),
                TurnState::Idle | TurnState::Working | TurnState::Blocked | TurnState::Cancelling => "error".into(),
            },
            AgentPhase::Connecting | AgentPhase::NotStarted(_) | AgentPhase::Lost { .. } | AgentPhase::Broken { .. } => {
                "error".into()
            }
        }
    }

    /// A permission notification's body (decision 6): the latest pending
    /// request's ([`permission_body`]), with its tool call's kind when the
    /// zone still holds that call, and how many others wait; `Waiting for
    /// permission` when none is pending.
    fn pending_body(&self) -> String {
        let pending: Vec<(&Option<String>, &str, &[PermissionOption])> = self
            .entries
            .iter()
            .filter_map(|(_, entry)| match entry {
                Entry::Permission { call, title, options, outcome: None } => Some((call, title.as_str(), options.as_slice())),
                Entry::Permission { outcome: Some(_), .. }
                | Entry::User { .. }
                | Entry::Agent { .. }
                | Entry::Thought { .. }
                | Entry::Tool { .. }
                | Entry::Notice { .. } => None,
            })
            .collect();
        let Some((call, title, options)) = pending.last() else { return "Waiting for permission".into() };
        let kind = call.as_deref().and_then(|call| self.tool_kind(call));
        permission_body(title, kind, &buttons(options), pending.len() - 1)
    }

    /// The kind of tool call `call`, while the zone holds its entry.
    fn tool_kind(&self, call: &str) -> Option<ToolKind> {
        self.entries.iter().rev().find_map(|(_, entry)| match entry {
            Entry::Tool { call: own, kind, .. } if own == call => Some(*kind),
            Entry::Tool { .. }
            | Entry::User { .. }
            | Entry::Agent { .. }
            | Entry::Thought { .. }
            | Entry::Permission { .. }
            | Entry::Notice { .. } => None,
        })
    }

    /// The notification for a shown status going from `prev` to `next`:
    /// on becoming `blocked`, `done` or `error` from another, never while
    /// replaying. Titled with [`notice_title`] (the profile, what it asks,
    /// the zone's title when it differs); the body is the permission's
    /// ([`permission_body`]), `Done`, or the failure's words.
    pub fn notice(&self, prev: Option<State>, next: Option<State>, replaying: bool) -> Option<AgentNotice> {
        if replaying || prev == next {
            return None;
        }
        let state = next?;
        let body = match state {
            State::Blocked => self.pending_body(),
            State::Done => "Done".to_string(),
            State::Error => self.error_text(),
            State::Idle | State::Working => return None,
        };
        Some(AgentNotice { title: notice_title(&self.profile, &self.title(), state), body })
    }
}

#[cfg(test)]
mod tests {
    use crate::agent::transcript::{Event, PermissionKind, PermissionOutcome, StopReason};

    use super::super::harness::*;
    use super::super::*;
    use super::*;

    /// `agents-zone-attention` decision 4 (the spec's "Rejects come
    /// first"): the buttons are reject kinds first, then allow kinds, each
    /// in the agent's order; the dump's entry line keeps the agent's order.
    #[test]
    fn a_permissions_rejects_come_first() {
        let entry = Entry::Permission {
            call: None,
            title: "Run cargo test?".into(),
            options: vec![
                option("allow-always", "Always allow", PermissionKind::AllowAlways),
                option("allow-once", "Allow once", PermissionKind::AllowOnce),
                option("reject-once", "Reject", PermissionKind::RejectOnce),
                option("reject-always", "Never", PermissionKind::RejectAlways),
            ],
            outcome: None,
        };
        let blocks = drawn(EntryId(1), &entry, false);
        let [Block { body: BlockBody::Permission { options, .. }, .. }] = blocks.as_slice() else { panic!("{blocks:?}") };
        let names: Vec<&str> = options.iter().map(|o| o.name.as_ref()).collect();
        assert_eq!(names, ["Reject", "Never", "Always allow", "Allow once"]);
        let model = ChatModel::new("Fake", None);
        assert_eq!(
            model.entry_line(EntryId(1), &entry),
            "permission options=allow-always:\"Always allow\"|allow-once:\"Allow once\"|reject-once:\"Reject\"|reject-always:\"Never\" outcome=pending \"Run cargo test?\""
        );
    }

    /// Decision 8: an answer names a pending request's option; an answered
    /// request takes none.
    #[test]
    fn permissions_are_answered_once() {
        let mut zone = Zone::ready();
        zone.feed(&[turn(TurnState::Blocked), added(1, permission("Run cargo test?"))]);
        assert!(zone.model.answer(EntryId(1), "allow-once"));
        assert!(!zone.model.answer(EntryId(1), "allow-always"), "not offered");
        assert!(!zone.model.answer(EntryId(9), "allow-once"), "no such entry");
        assert_eq!(zone.model.pending_permissions(), [(EntryId(1), vec!["reject-once".into(), "allow-once".into()])]);
        zone.model.sent(1, Sent::Answer(EntryId(1)));
        assert_eq!(zone.model.replied(1, Ok(())), Replied::Nothing, "the outcome comes as an event");
        zone.feed(&[Event::Answered { id: EntryId(1), outcome: PermissionOutcome::Selected { option: "allow-once".into() } }]);
        assert!(!zone.model.answer(EntryId(1), "allow-once"), "answered");
        assert!(zone.model.pending_permissions().is_empty());
        assert!(zone.model.describe(1, true, &found(&zone.model), &linux).contains("outcome=selected:allow-once"));
    }

    /// Decision 10: the shown status is the activity's while live, with
    /// `done` and `error` dropped once the user types, until it changes.
    #[test]
    fn the_shown_status_and_the_seen_rule() {
        let mut zone = Zone::new(None);
        assert_eq!(zone.model.shown_status(), None, "connecting");
        assert_eq!(zone.feed(&[agent_state(AgentState::Starting)]).status, Some(Some(State::Working)));
        zone.feed(&[ready()]);
        assert_eq!(zone.model.shown_status(), Some(State::Idle));
        assert_eq!(zone.model.seen(), None, "idle isn't dropped");
        zone.feed(&[turn(TurnState::Working)]);
        let absorbed = zone.feed(&[turn(TurnState::Ended { stop: StopReason::EndTurn })]);
        assert_eq!(absorbed.status, Some(Some(State::Done)));
        assert_eq!(zone.model.seen(), Some(None));
        assert_eq!(zone.model.seen(), None);
        assert_eq!(zone.feed(&[added(1, agent("more"))]).status, None, "the activity didn't change");
        assert_eq!(zone.feed(&[turn(TurnState::Working)]).status, Some(Some(State::Working)));
        assert_eq!(zone.feed(&[turn(TurnState::Ended { stop: StopReason::EndTurn })]).status, Some(Some(State::Done)));
        assert_eq!(zone.model.lost("gone").status, Some(None), "no status outside live");
    }

    /// Decision 10: notices on becoming blocked, done or error, never from
    /// a replay, titled with the profile's name, what it asks and the
    /// zone's title when it differs (`agents-zone-attention` decision 6).
    #[test]
    fn notices_on_blocked_done_and_error() {
        let mut zone = Zone::ready();
        zone.feed(&[turn(TurnState::Working)]);
        let absorbed = zone.feed(&[turn(TurnState::Blocked), added(1, permission("Run cargo test?"))]);
        // The profile's name, not the agent's own; the call isn't held, so
        // no kind word.
        assert_eq!(
            absorbed.notice,
            Some(AgentNotice { title: "Fake needs permission".into(), body: "Run cargo test?\nReject · Allow once".into() })
        );
        zone.feed(&[titled(Some("Hello.txt file creation"))]);
        let absorbed = zone.feed(&[turn(TurnState::Ended { stop: StopReason::EndTurn })]);
        assert_eq!(
            absorbed.notice,
            Some(AgentNotice { title: "Fake — Hello.txt file creation".into(), body: "Done".into() })
        );
        let absorbed = zone.feed(&[turn(TurnState::Failed { message: "rate limited".into() })]);
        assert_eq!(
            absorbed.notice,
            Some(AgentNotice { title: "Fake — Hello.txt file creation".into(), body: "rate limited".into() })
        );
        assert_eq!(zone.feed(&[turn(TurnState::Working)]).notice, None, "working raises none");

        let mut unnamed = Zone::new(None);
        let absorbed = unnamed.feed(&[agent_state(AgentState::Failed { message: "bad".into() })]);
        assert_eq!(absorbed.notice, Some(AgentNotice { title: "Fake".into(), body: "bad".into() }));
        assert_eq!(unnamed.model.notice(Some(State::Working), Some(State::Error), true), None, "replaying");

        let mut replayed = Zone::new(None);
        let absorbed = replayed.replay(&[ready(), turn(TurnState::Ended { stop: StopReason::EndTurn })]);
        assert_eq!(absorbed.status, Some(Some(State::Done)));
        assert_eq!(absorbed.notice, None, "a replay raises nothing");
    }

    /// `agents-zone-attention` decision 2: the working row for every turn,
    /// a tool call in progress after or before the latest user message, a
    /// completed one, a cancel, and the phases and an ended session that
    /// show none.
    #[test]
    fn the_working_row_follows_the_turn() {
        let tool = |id: u64, call: &str, title: &str, status| {
            added(id, Entry::tool(call.into(), title.into(), ToolKind::Read, status))
        };
        // The row after `feed`, and whether the feed's last splice is the
        // row's (at the block count): `Some(true)` inserted, `Some(false)`
        // removed.
        let fed = |zone: &mut Zone, events: &[Event]| {
            let absorbed = zone.feed(events);
            let len = zone.model.blocks().len();
            let splice = match absorbed.splices.last() {
                Some(Splice { range, count: 1 }) if *range == (len..len) => Some(true),
                Some(Splice { range, count: 0 }) if *range == (len..len + 1) => Some(false),
                Some(_) | None => None,
            };
            (zone.model.working_row(), splice)
        };
        let mut zone = Zone::ready();
        assert_eq!(zone.model.working_row(), None, "idle");
        assert_eq!(fed(&mut zone, &[added(1, user("hi")), turn(TurnState::Working)]), (Some(WorkingRow::Working), Some(true)));
        assert_eq!(WorkingRow::Working.text(), "Working…");
        let row = fed(&mut zone, &[tool(2, "c1", "Read main.rs", ToolStatus::InProgress)]);
        assert_eq!(row, (Some(WorkingRow::Tool { title: "Read main.rs".into() }), None), "its words, not its place");
        assert_eq!(zone.model.working_row().unwrap().text(), "Read main.rs");
        assert_eq!(fed(&mut zone, &[tool_completed(2)]).0, Some(WorkingRow::Working), "a completed call");
        // The newest of two in progress.
        zone.feed(&[tool(3, "c2", "First", ToolStatus::InProgress), tool(4, "c3", "Second", ToolStatus::InProgress)]);
        assert_eq!(zone.model.working_row(), Some(WorkingRow::Tool { title: "Second".into() }));
        assert_eq!(fed(&mut zone, &[turn(TurnState::Blocked)]), (None, Some(false)), "a permission waits");
        let row = fed(&mut zone, &[turn(TurnState::Working)]);
        assert_eq!(row, (Some(WorkingRow::Tool { title: "Second".into() }), Some(true)));
        assert_eq!(fed(&mut zone, &[turn(TurnState::Cancelling)]), (Some(WorkingRow::Stopping), None));
        assert_eq!(WorkingRow::Stopping.text(), "Stopping…");
        let row = fed(&mut zone, &[turn(TurnState::Ended { stop: StopReason::Cancelled })]);
        assert_eq!(row, (None, Some(false)), "the turn ended");
        assert_eq!(fed(&mut zone, &[turn(TurnState::Ended { stop: StopReason::EndTurn })]), (None, None), "unchanged");

        // A call in progress before the latest user message isn't this
        // turn's.
        let mut next = Zone::ready();
        next.feed(&[tool(1, "c1", "Old", ToolStatus::InProgress), added(2, user("again")), turn(TurnState::Working)]);
        assert_eq!(next.model.working_row(), Some(WorkingRow::Working));
        assert_eq!(fed(&mut next, &[turn(TurnState::Failed { message: "x".into() })]), (None, Some(false)), "failed");
        next.feed(&[turn(TurnState::Idle)]);
        assert_eq!(next.model.working_row(), None);

        // Working, but the session ended or the agent isn't ready.
        let mut ended = Zone::ready();
        ended.feed(&[turn(TurnState::Working)]);
        assert_eq!(ended.model.ended().splices, [Splice { range: 0..1, count: 0 }], "an ended session");
        assert_eq!(ended.model.working_row(), None);
        for phase in every_phase() {
            let mut model = ChatModel::new("Fake", None);
            model.turn = TurnState::Working;
            let ready = matches!(phase, AgentPhase::Live(AgentState::Ready { .. }));
            model.phase = phase.clone();
            assert_eq!(model.working_row().is_some(), ready, "{phase:?}");
        }
    }

    /// `agents-zone-attention` decision 6: the words of a notification's
    /// title and a permission's body.
    #[test]
    fn notification_words() {
        assert_eq!(notice_title("Fake", "Fake", State::Blocked), "Fake needs permission");
        assert_eq!(
            notice_title("Fake", "Hello.txt file creation", State::Blocked),
            "Fake needs permission — Hello.txt file creation"
        );
        assert_eq!(notice_title("Fake", "Fake", State::Done), "Fake");
        assert_eq!(notice_title("Fake", "Hello.txt file creation", State::Done), "Fake — Hello.txt file creation");
        assert_eq!(notice_title("Fake", "Build", State::Error), "Fake — Build");
        assert_eq!(notice_title("Fake", " ", State::Error), "Fake");

        let words: Vec<Option<&str>> = [
            ToolKind::Read,
            ToolKind::Edit,
            ToolKind::Delete,
            ToolKind::Move,
            ToolKind::Search,
            ToolKind::Execute,
            ToolKind::Think,
            ToolKind::Fetch,
            ToolKind::SwitchMode,
            ToolKind::Other,
        ]
        .into_iter()
        .map(kind_word)
        .collect();
        assert_eq!(
            words,
            [
                Some("Read"),
                Some("Edit"),
                Some("Delete"),
                Some("Move"),
                Some("Search"),
                Some("Command"),
                Some("Think"),
                Some("Fetch"),
                Some("Mode"),
                None
            ]
        );

        let row = |names: &[&str]| -> Vec<PermissionButton> {
            names
                .iter()
                .map(|name| PermissionButton {
                    id: name.to_string().into(),
                    name: name.to_string().into(),
                    kind: PermissionKind::AllowOnce,
                })
                .collect()
        };
        assert_eq!(
            permission_body("cargo test", Some(ToolKind::Execute), &row(&["Reject", "Allow once"]), 0),
            "Command: cargo test\nReject · Allow once"
        );
        assert_eq!(permission_body("Use the network?", None, &row(&["Deny", "Allow"]), 0), "Use the network?\nDeny · Allow");
        assert_eq!(permission_body("x", Some(ToolKind::Other), &row(&["Reject"]), 0), "x\nReject");
        assert_eq!(
            permission_body("cargo test", Some(ToolKind::Execute), &row(&["Reject", "Allow once"]), 1),
            "Command: cargo test\nReject · Allow once · +1 more"
        );
        assert_eq!(permission_body("x", None, &[], 0), "x");
        assert_eq!(permission_body("x", None, &[], 2), "x\n+2 more");
    }

    /// The spec's "A command" and "More requests waiting": the body names
    /// the held tool call's kind and the row's options, and the requests
    /// waiting beside the latest; a trimmed call is worded without a kind.
    #[test]
    fn a_permission_notice_gives_the_context() {
        let tool = |call: &str, kind| {
            added(2, Entry::tool(call.into(), "cargo test".into(), kind, ToolStatus::Pending))
        };
        let asking = |id: u64, call: &str, title: &str| {
            added(
                id,
                Entry::Permission {
                    call: Some(call.into()),
                    title: title.into(),
                    options: vec![
                        option("allow-once", "Allow once", PermissionKind::AllowOnce),
                        option("reject-once", "Reject", PermissionKind::RejectOnce),
                    ],
                    outcome: None,
                },
            )
        };
        let mut zone = Zone::ready();
        zone.feed(&[added(1, user("hi")), turn(TurnState::Working), tool("call_2", ToolKind::Execute)]);
        let absorbed = zone.feed(&[turn(TurnState::Blocked), asking(3, "call_2", "cargo test")]);
        assert_eq!(absorbed.notice.unwrap().body, "Command: cargo test\nReject · Allow once");

        let mut two = Zone::ready();
        two.feed(&[added(1, user("hi")), turn(TurnState::Working), tool("call_2", ToolKind::Edit)]);
        let absorbed =
            two.feed(&[turn(TurnState::Blocked), asking(3, "call_2", "Write a.txt"), asking(4, "call_9", "Run it")]);
        // The later names a call the zone doesn't hold: no kind word.
        assert_eq!(absorbed.notice.unwrap().body, "Run it\nReject · Allow once · +1 more");

        let mut none = Zone::ready();
        none.feed(&[turn(TurnState::Working)]);
        let absorbed = none.feed(&[turn(TurnState::Blocked)]);
        assert_eq!(absorbed.notice.unwrap().body, "Waiting for permission");
    }
}
