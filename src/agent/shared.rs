//! The fold an agent session's reader thread and its zone share (design.md
//! decision 8): the session's [`Transcript`], folded from the events the
//! terminal server sends, and what changed since the zone last looked
//! ([`Changes`]). It sits behind a mutex the sink and the entity share, a
//! leaf lock: nothing is taken under it. The UI thread holds it only to
//! take the changes and copy what changed (`ChatModel::copy_changes`,
//! OpenSpec change `agents-absorb-cost` decision 2), never across a
//! render: the zone applies the copy and renders after letting go, and
//! `render` and `prepaint` never take it (`process R14`).
//!
//! Pure: values in, values out.

use std::collections::BTreeMap;
use std::fmt;

use serde_json::Value;

use crate::agent::transcript::{Entry, EntryId, Event, Transcript};

/// How an entry the zone already shows changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dirty {
    /// Text was appended from this byte of the entry's text on: the zone
    /// copies only the new tail.
    Appended { from_byte: usize },
    /// A tool call's content items from index `from` on were replaced
    /// (`ToolItems`, OpenSpec change `agents-tool-content` decision 10):
    /// the zone draws those items' parts again.
    ToolItems { from: usize },
    /// Output was appended to a tool call's item `item` (`ToolOutput`):
    /// the zone draws that output's last part again.
    Output { item: usize },
    /// Something else changed (a tool call's fields, a permission's
    /// outcome): the zone copies the entry again.
    Replaced,
}

impl Dirty {
    /// Two changes to one tool entry as one: the items from the first
    /// that either touched, or a replacement when either replaced it.
    fn and(self, later: Self) -> Self {
        match (self, later) {
            (Self::Output { item: a }, Self::Output { item: b }) if a == b => Self::Output { item: a },
            (
                Self::ToolItems { from: a } | Self::Output { item: a },
                Self::ToolItems { from: b } | Self::Output { item: b },
            ) => Self::ToolItems { from: a.min(b) },
            (Self::Replaced | Self::Appended { .. } | Self::ToolItems { .. } | Self::Output { .. }, _) => {
                Self::Replaced
            }
        }
    }
}

/// What changed in the transcript since [`AgentShared::take_changes`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Changes {
    /// The first entry added since; every entry from it on is new.
    pub added_from: Option<EntryId>,
    /// Entries before `added_from` that changed.
    pub changed: BTreeMap<EntryId, Dirty>,
    /// The entries through this one were trimmed.
    pub trimmed_through: Option<EntryId>,
    /// A `Queue` event set the slot (read it whole with
    /// [`Transcript::slot`]).
    pub slot_changed: bool,
    /// A `Turn` or `Agent` event came.
    pub turn_or_agent: bool,
    /// A `Commands` or `Options` event came (OpenSpec change
    /// `agents-commands`): read both lists whole with
    /// [`Transcript::commands`] and [`Transcript::options`].
    pub lists_changed: bool,
    /// A replay started: the transcript was rebuilt from nothing, and the
    /// zone starts over from it.
    pub reset: bool,
    /// Where the newest `Turn` or `Agent` event in these changes was folded
    /// (OpenSpec change `agents-client-state-fixes`, decision 8): one from
    /// a replay already happened and raises nothing, while live ones after
    /// it in the same changes do.
    pub status_from: StatusFrom,
    /// Fetched entries were folded in front of the held ones
    /// ([`AgentShared::prepend`], OpenSpec change `agents-log-paging`):
    /// this many, every one before the entries the zone already shows.
    pub prepended: Option<usize>,
}

/// Where the newest `Turn` or `Agent` event of a batch of changes was
/// folded.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum StatusFrom {
    /// None came.
    #[default]
    Unchanged,
    Replay,
    Live,
}

impl Changes {
    /// Whether nothing changed.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    fn added(&mut self, id: EntryId) {
        self.added_from.get_or_insert(id);
    }

    fn is_new(&self, id: EntryId) -> bool {
        self.added_from.is_some_and(|from| id >= from)
    }

    fn appended(&mut self, id: EntryId, from_byte: usize) {
        if self.is_new(id) {
            return;
        }
        self.changed.entry(id).or_insert(Dirty::Appended { from_byte });
    }

    fn replaced(&mut self, id: EntryId) {
        if !self.is_new(id) {
            self.changed.insert(id, Dirty::Replaced);
        }
    }

    /// A tool entry's items changed (`ToolItems` or `ToolOutput`), merged
    /// with what changed in it before.
    fn tool_content(&mut self, id: EntryId, dirty: Dirty) {
        if self.is_new(id) {
            return;
        }
        let merged = self.changed.get(&id).map_or(dirty, |before| before.and(dirty));
        self.changed.insert(id, merged);
    }

    fn trimmed(&mut self, through: EntryId) {
        self.trimmed_through = Some(self.trimmed_through.map_or(through, |before| before.max(through)));
        self.changed.retain(|id, _| *id > through);
    }
}

/// Why the zone can't follow its session any more: an event that didn't
/// decode or didn't fold. The session is detached as broken.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Broken(pub String);

impl fmt::Display for Broken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The transcript the reader folds and the zone reads.
#[derive(Debug, Clone, Default)]
pub struct AgentShared {
    transcript: Transcript,
    replaying: bool,
    changes: Changes,
}

impl AgentShared {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn transcript(&self) -> &Transcript {
        &self.transcript
    }

    #[cfg(test)]
    pub fn replaying(&self) -> bool {
        self.replaying
    }

    /// `REPLAY_START`: the replay rebuilds the transcript from nothing.
    pub fn replay_start(&mut self) {
        self.transcript = Transcript::new();
        self.replaying = true;
        // A replay that brings no status event is still all from a replay.
        self.changes = Changes { reset: true, status_from: StatusFrom::Replay, ..Changes::default() };
    }

    /// A fold for a new session that takes the zone's place (a continue in
    /// a new session): its first change starts the zone over from it, as a
    /// replay's does, but its events are live.
    pub fn start_over(&mut self) {
        self.changes.reset = true;
    }

    /// `REPLAY_END`: events from here on are live.
    pub fn replay_end(&mut self) {
        self.replaying = false;
    }

    /// Decodes and folds one `TRANSCRIPT` frame's events, in order. An event
    /// that doesn't decode ([`Event::from_value`]) or fold
    /// ([`Transcript::apply`]) stops it: the session is [`Broken`], and the
    /// events before it stay folded (the zone is left as it was shown).
    pub fn apply(&mut self, values: &[Value]) -> Result<(), Broken> {
        for value in values {
            let event = Event::from_value(value)
                .map_err(|why| Broken(format!("an event from the terminal server couldn't be read: {why}")))?;
            self.fold(&event)?;
        }
        Ok(())
    }

    fn fold(&mut self, event: &Event) -> Result<(), Broken> {
        let before = match event {
            Event::Appended { id, .. } => self.transcript.entry(*id).map(text_len),
            Event::Added { .. }
            | Event::ToolChanged { .. }
            | Event::ToolItems { .. }
            | Event::ToolOutput { .. }
            | Event::Answered { .. }
            | Event::Queue { .. }
            | Event::Turn { .. }
            | Event::Agent { .. }
            | Event::Conversation { .. }
            | Event::Trimmed { .. }
            | Event::Commands { .. }
            | Event::Options { .. }
            | Event::Window { .. } => None,
        };
        self.transcript
            .apply(event)
            .map_err(|why| Broken(format!("an event from the terminal server couldn't be applied: {why}")))?;
        let changes = &mut self.changes;
        if let Event::Turn { .. } | Event::Agent { .. } = event {
            changes.status_from = if self.replaying { StatusFrom::Replay } else { StatusFrom::Live };
        }
        // A paged copy ignores a change to an entry below its window: there
        // is nothing the zone shows to redraw.
        let held = |id: &EntryId| self.transcript.entry(*id).is_some();
        match event {
            Event::Appended { id, .. }
            | Event::ToolChanged { id, .. }
            | Event::ToolItems { id, .. }
            | Event::ToolOutput { id, .. }
            | Event::Answered { id, .. }
                if !held(id) => {}
            Event::Added { id, .. } => changes.added(*id),
            Event::Appended { id, .. } => changes.appended(*id, before.unwrap_or(0)),
            Event::ToolItems { id, from, .. } => changes.tool_content(*id, Dirty::ToolItems { from: *from as usize }),
            Event::ToolOutput { id, item, .. } => changes.tool_content(*id, Dirty::Output { item: *item as usize }),
            Event::ToolChanged { id, .. } | Event::Answered { id, .. } => changes.replaced(*id),
            Event::Queue { .. } => changes.slot_changed = true,
            // The conversation decides whether a continue is offered.
            Event::Turn { .. } | Event::Agent { .. } | Event::Conversation { .. } => changes.turn_or_agent = true,
            // The agent's commands and options (`agents-commands`).
            Event::Commands { .. } | Event::Options { .. } => changes.lists_changed = true,
            Event::Trimmed { through, .. } => changes.trimmed(*through),
            // Only in a replay, after its reset: the zone reads the window
            // from the transcript on every absorb.
            Event::Window { .. } => {}
        }
        Ok(())
    }

    /// Decodes a fetched page's events and folds them in front of the held
    /// entries ([`Transcript::prepend`]); `more` says whether entries are
    /// still earlier. Recorded as `prepended`, which says they already
    /// happened: a page carries entries only, so it leaves `status_from`
    /// alone. One that doesn't decode or fit
    /// leaves the transcript as it was, and the session is [`Broken`].
    pub fn prepend(&mut self, values: &[Value], more: bool) -> Result<(), Broken> {
        let events = values
            .iter()
            .map(Event::from_value)
            .collect::<Result<Vec<Event>, _>>()
            .map_err(|why| Broken(format!("an event from the terminal server couldn't be read: {why}")))?;
        let before = self.transcript.entries().count();
        self.transcript
            .prepend(&events, more)
            .map_err(|why| Broken(format!("a page from the terminal server couldn't be applied: {why}")))?;
        let added = self.transcript.entries().count() - before;
        self.changes.prepended = Some(self.changes.prepended.unwrap_or(0) + added);
        Ok(())
    }

    /// What changed since the last call, handed over.
    pub fn take_changes(&mut self) -> Changes {
        std::mem::take(&mut self.changes)
    }
}

/// The length of an entry's growing text, in bytes; 0 for the kinds
/// without one.
fn text_len(entry: &Entry) -> usize {
    match entry {
        Entry::User { text } | Entry::Agent { text } | Entry::Thought { text } => text.len(),
        Entry::Tool { .. } | Entry::Permission { .. } | Entry::Notice { .. } => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::transcript::{
        AgentState, HoldReason, NoticeLevel, Output, PermissionKind, PermissionOption, PermissionOutcome, Slot,
        StopReason, ToolItem, ToolKind, ToolStatus, TurnState,
    };

    fn values(events: &[Event]) -> Vec<Value> {
        events.iter().map(Event::to_value).collect()
    }

    fn added(id: u64, entry: Entry) -> Event {
        Event::Added { id: EntryId(id), entry }
    }

    fn agent(text: &str) -> Entry {
        Entry::Agent { text: text.into() }
    }

    fn permission() -> Entry {
        Entry::Permission {
            call: Some("call_2".into()),
            title: "Run cargo test?".into(),
            options: vec![PermissionOption { id: "allow-once".into(), name: "Allow once".into(), kind: PermissionKind::AllowOnce }],
            outcome: None,
        }
    }

    fn tool() -> Entry {
        Entry::tool("call_1".into(), "Read main.rs".into(), ToolKind::Read, ToolStatus::Pending)
    }

    /// Every event kind folds, and each says what changed.
    #[test]
    fn every_event_kind_folds_and_says_what_changed() {
        let mut shared = AgentShared::new();
        let first = [
            added(1, Entry::User { text: "hi".into() }),
            added(2, agent("Hel")),
            added(3, Entry::Thought { text: "hmm".into() }),
            added(4, tool()),
            added(5, permission()),
            added(6, Entry::Notice { level: NoticeLevel::Warning, text: "1 line was not an ACP message".into() }),
        ];
        shared.apply(&values(&first)).unwrap();
        let changes = shared.take_changes();
        assert_eq!(changes, Changes { added_from: Some(EntryId(1)), ..Changes::default() });
        assert!(shared.take_changes().is_empty(), "taken");

        let next = [
            Event::Appended { id: EntryId(2), text: "lo".into() },
            Event::Appended { id: EntryId(2), text: "!".into() },
            Event::ToolChanged {
                id: EntryId(4),
                title: None,
                kind: None,
                status: Some(ToolStatus::Completed),
                locations: None,
                input: None,
                result: None,
                more: None,
            },
            Event::Answered { id: EntryId(5), outcome: PermissionOutcome::Selected { option: "allow-once".into() } },
            Event::Queue { slot: Slot::Waiting { text: "next".into() } },
            Event::Turn { state: TurnState::Working },
            Event::Agent { state: AgentState::Ready { name: Some("Fake Agent".into()), version: None } },
            added(7, agent("new")),
            Event::Appended { id: EntryId(7), text: " text".into() },
        ];
        shared.apply(&values(&next)).unwrap();
        let changes = shared.take_changes();
        assert_eq!(
            changes,
            Changes {
                added_from: Some(EntryId(7)),
                changed: BTreeMap::from([
                    (EntryId(2), Dirty::Appended { from_byte: 3 }),
                    (EntryId(4), Dirty::Replaced),
                    (EntryId(5), Dirty::Replaced),
                ]),
                slot_changed: true,
                turn_or_agent: true,
                status_from: StatusFrom::Live,
                ..Changes::default()
            },
            "an appended entry keeps its first tail; a new entry's changes aren't listed"
        );
        assert_eq!(shared.transcript().entry(EntryId(2)), Some(&agent("Hello!")));
        assert_eq!(shared.transcript().entry(EntryId(7)), Some(&agent("new text")));

        shared.apply(&values(&[Event::Appended { id: EntryId(6), text: "x".into() }])).unwrap_err();
        let trim = [
            Event::Appended { id: EntryId(2), text: "?".into() },
            Event::Appended { id: EntryId(7), text: "!".into() },
            Event::Trimmed { through: EntryId(3), dropped: 3 },
        ];
        shared.apply(&values(&trim)).unwrap();
        let changes = shared.take_changes();
        assert_eq!(changes.trimmed_through, Some(EntryId(3)));
        assert_eq!(changes.changed, BTreeMap::from([(EntryId(7), Dirty::Appended { from_byte: 8 })]), "trimmed ones go");
        assert_eq!(changes.status_from, StatusFrom::Unchanged);
    }

    /// A replay starts over: it holds a waiting prompt or a held one (its
    /// `Queue` is in the replay), and everything it folds is marked as
    /// from a replay; live events after it aren't.
    #[test]
    fn a_replay_starts_over_and_carries_the_slot() {
        for slot in [
            Slot::Waiting { text: "next".into() },
            Slot::Held { text: "next".into(), reason: HoldReason::Stopped { stop: StopReason::MaxTokens } },
        ] {
            let mut source = Transcript::new();
            for event in [
                added(1, Entry::User { text: "hi".into() }),
                added(2, agent("Hello!")),
                Event::Queue { slot: slot.clone() },
                Event::Turn { state: TurnState::Ended { stop: StopReason::EndTurn } },
                Event::Agent { state: AgentState::Ready { name: None, version: None } },
            ] {
                source.apply(&event).unwrap();
            }
            let mut shared = AgentShared::new();
            shared.apply(&values(&[added(1, agent("stale"))])).unwrap();
            shared.take_changes();
            shared.replay_start();
            assert!(shared.replaying());
            shared.apply(&values(&source.replay())).unwrap();
            shared.replay_end();
            let changes = shared.take_changes();
            assert!(changes.reset && changes.slot_changed && changes.turn_or_agent);
            assert_eq!(changes.status_from, StatusFrom::Replay);
            assert_eq!(changes.added_from, Some(EntryId(1)));
            assert_eq!(shared.transcript(), &source, "the copy is the server's log");
            assert_eq!(shared.transcript().slot(), &slot);
            shared.apply(&values(&[Event::Turn { state: TurnState::Working }])).unwrap();
            let live = shared.take_changes();
            assert!(!live.reset && live.turn_or_agent);
            assert_eq!(live.status_from, StatusFrom::Live);
        }
    }

    /// D-10: `status_from` follows the newest `Turn` or `Agent` event of a
    /// batch, so live ones after a replay in the same batch count as live,
    /// and entries alone change nothing.
    #[test]
    fn the_newest_status_event_says_where_it_was_folded() {
        let mut shared = AgentShared::new();
        shared.apply(&values(&[added(1, agent("hi"))])).unwrap();
        assert_eq!(shared.take_changes().status_from, StatusFrom::Unchanged, "entries alone");
        shared.replay_start();
        shared.apply(&values(&[Event::Turn { state: TurnState::Idle }])).unwrap();
        shared.replay_end();
        // A replay starts the log over, so its first entry is 1 again.
        shared.apply(&values(&[added(1, agent("live")), Event::Agent { state: AgentState::Ready { name: None, version: None } }])).unwrap();
        assert_eq!(shared.take_changes().status_from, StatusFrom::Live, "the live one came last");
        shared.replay_start();
        shared.apply(&values(&[Event::Turn { state: TurnState::Idle }])).unwrap();
        shared.replay_end();
        shared.apply(&values(&[added(1, agent("live"))])).unwrap();
        assert_eq!(shared.take_changes().status_from, StatusFrom::Replay, "a live entry doesn't count");
    }

    /// A value `from_value` rejects and an event the fold rejects each
    /// break the session, saying why; what came before stays folded.
    #[test]
    fn an_event_that_doesnt_decode_or_fold_breaks_the_session() {
        let mut shared = AgentShared::new();
        let bad = [added(1, agent("kept")).to_value(), serde_json::json!({"event": "teleported"})];
        let Broken(why) = shared.apply(&bad).unwrap_err();
        assert!(why.starts_with("an event from the terminal server couldn't be read: "), "{why}");
        assert_eq!(shared.transcript().entry(EntryId(1)), Some(&agent("kept")));
        let Broken(why) = shared.apply(&[serde_json::json!("not an object")]).unwrap_err();
        assert!(why.contains("a transcript event is a JSON object"), "{why}");

        let Broken(why) = shared.apply(&values(&[added(5, agent("out of order"))])).unwrap_err();
        assert_eq!(why, "an event from the terminal server couldn't be applied: entry 5 was added where entry 2 was next");
        let Broken(why) = shared.apply(&values(&[Event::Appended { id: EntryId(9), text: "x".into() }])).unwrap_err();
        assert!(why.ends_with("there is no entry 9"), "{why}");
        assert_eq!(Broken("x".into()).to_string(), "x");
    }

    // agents-tool-content 5.2: tool content's changes.

    fn output(text: &str) -> ToolItem {
        ToolItem::Output(Output { text: text.into(), ..Output::default() })
    }

    fn items(id: u64, from: u32, items: Vec<ToolItem>) -> Event {
        Event::ToolItems { id: EntryId(id), from, items }
    }

    fn out(id: u64, item: u32, text: &str) -> Event {
        Event::ToolOutput { id: EntryId(id), item, text: text.into(), exit: None }
    }

    /// Each tool content event folds and is recorded as what it changed;
    /// changes to one entry merge, and a new entry's aren't listed.
    #[test]
    fn tool_content_events_record_the_items_they_change() {
        let mut shared = AgentShared::new();
        shared.apply(&values(&[added(1, tool()), added(2, tool())])).unwrap();
        shared.apply(&values(&[items(1, 0, vec![output("a\n"), output("b\n")])])).unwrap();
        shared.take_changes();

        let table: Vec<(Vec<Event>, Dirty)> = vec![
            (vec![out(1, 1, "more\n")], Dirty::Output { item: 1 }),
            (vec![out(1, 1, "x"), out(1, 1, "y\n")], Dirty::Output { item: 1 }),
            (vec![out(1, 1, "x"), out(1, 0, "y\n")], Dirty::ToolItems { from: 0 }),
            (vec![items(1, 1, vec![output("c\n")])], Dirty::ToolItems { from: 1 }),
            (vec![out(1, 1, "x"), items(1, 2, vec![output("d\n")])], Dirty::ToolItems { from: 1 }),
            (vec![items(1, 3, vec![]), out(1, 0, "z\n")], Dirty::ToolItems { from: 0 }),
            (
                vec![
                    out(1, 0, "z\n"),
                    Event::ToolChanged {
                        id: EntryId(1),
                        title: Some("Run".into()),
                        kind: None,
                        status: None,
                        locations: None,
                        input: None,
                        result: None,
                        more: None,
                    },
                    out(1, 0, "w\n"),
                ],
                Dirty::Replaced,
            ),
        ];
        for (events, dirty) in table {
            shared.apply(&values(&events)).unwrap();
            let changes = shared.take_changes();
            assert_eq!(changes.changed, BTreeMap::from([(EntryId(1), dirty)]), "{events:?}");
        }
        let Some(Entry::Tool { content, .. }) = shared.transcript().entry(EntryId(1)) else { panic!("a tool") };
        assert!(!content.is_empty());

        // Content for a tool entry added in the same changes isn't listed.
        shared.apply(&values(&[added(3, tool()), items(3, 0, vec![output("new\n")]), out(3, 0, "more\n")])).unwrap();
        let changes = shared.take_changes();
        assert_eq!(changes.added_from, Some(EntryId(3)));
        assert!(changes.changed.is_empty(), "{:?}", changes.changed);
        // A trim drops the trimmed entries' changes.
        shared.apply(&values(&[items(2, 0, vec![output("x\n")]), out(1, 0, "a\n")])).unwrap();
        shared.apply(&values(&[Event::Trimmed { through: EntryId(1), dropped: 1 }])).unwrap();
        let changes = shared.take_changes();
        assert_eq!(changes.changed, BTreeMap::from([(EntryId(2), Dirty::ToolItems { from: 0 })]));
    }

    /// A replay and a prepended page carry tool content whole: the copy is
    /// the server's log, and nothing is listed as changed.
    #[test]
    fn a_replay_and_a_page_carry_tool_content() {
        let mut source = Transcript::new();
        for event in [
            added(1, Entry::User { text: "go".into() }),
            added(2, tool()),
            items(2, 0, vec![ToolItem::Text { text: "**done**".into(), cut: false }, output("one\n")]),
            out(2, 1, "two\n"),
            added(3, agent("ok")),
        ] {
            source.apply(&event).unwrap();
        }
        let mut shared = AgentShared::new();
        shared.replay_start();
        shared.apply(&values(&source.replay())).unwrap();
        shared.replay_end();
        let changes = shared.take_changes();
        assert!(changes.reset && changes.changed.is_empty());
        assert_eq!(shared.transcript(), &source);

        // A paged copy of the last entry, then the rest fetched in front.
        let mut shared = AgentShared::new();
        shared.replay_start();
        shared.apply(&values(&source.page(1))).unwrap();
        shared.replay_end();
        shared.take_changes();
        let fetched = source.fetch(EntryId(3), usize::MAX);
        shared.prepend(&values(&fetched.events), fetched.more).unwrap();
        let changes = shared.take_changes();
        assert_eq!(changes.prepended, Some(2));
        assert!(changes.changed.is_empty());
        assert_eq!(changes.status_from, StatusFrom::Unchanged, "a page carries no status");
        assert_eq!(shared.transcript().entry(EntryId(2)), source.entry(EntryId(2)));
    }

    fn commands() -> Event {
        let compact = crate::agent::transcript::SlashCommand {
            name: "compact".into(),
            description: "Free up context".into(),
            hint: Some("<instructions>".into()),
        };
        Event::Commands { commands: vec![compact] }
    }

    fn options() -> Event {
        use crate::agent::transcript::{OptionCategory, OptionChoice, OptionControl, SessionOption};
        let choice = |value: &str| OptionChoice { value: value.into(), name: value.into(), description: None, group: None };
        let mode = SessionOption {
            id: "mode".into(),
            name: "Mode".into(),
            description: None,
            category: OptionCategory::Mode,
            kind: OptionControl::Select { current: "auto".into(), choices: vec![choice("auto"), choice("plan")] },
        };
        Event::Options { options: vec![mode] }
    }

    /// `agents-commands` 4.1: each list event folds and says so, replacing
    /// the last; a replay holding both carries both.
    #[test]
    fn the_commands_and_the_options_fold_and_replay() {
        let mut shared = AgentShared::new();
        shared.apply(&values(&[commands()])).unwrap();
        assert_eq!(shared.take_changes(), Changes { lists_changed: true, ..Changes::default() });
        assert_eq!(shared.transcript().commands().len(), 1);
        shared.apply(&values(&[options()])).unwrap();
        assert_eq!(shared.take_changes(), Changes { lists_changed: true, ..Changes::default() });
        assert_eq!(shared.transcript().options().len(), 1);
        shared.apply(&values(&[Event::Commands { commands: vec![] }])).unwrap();
        assert!(shared.take_changes().lists_changed);
        assert!(shared.transcript().commands().is_empty(), "replaced whole");

        let mut source = Transcript::new();
        for event in [
            added(1, Entry::User { text: "hi".into() }),
            Event::Agent { state: AgentState::Ready { name: None, version: None } },
            commands(),
            options(),
        ] {
            source.apply(&event).unwrap();
        }
        let mut shared = AgentShared::new();
        shared.replay_start();
        shared.apply(&values(&source.replay())).unwrap();
        shared.replay_end();
        let changes = shared.take_changes();
        assert!(changes.reset && changes.lists_changed);
        assert_eq!(shared.transcript(), &source);
        assert_eq!((shared.transcript().commands().len(), shared.transcript().options().len()), (1, 1));
    }

    /// Tool content for an entry below the window changes nothing the zone
    /// holds, and records nothing.
    #[test]
    fn tool_content_below_the_window_records_nothing() {
        let mut source = Transcript::new();
        for event in [added(1, tool()), items(1, 0, vec![output("a\n")]), added(2, agent("later"))] {
            source.apply(&event).unwrap();
        }
        let mut shared = AgentShared::new();
        shared.replay_start();
        shared.apply(&values(&source.page(1))).unwrap();
        shared.replay_end();
        shared.take_changes();
        assert!(shared.transcript().entry(EntryId(1)).is_none(), "below the window");
        shared.apply(&values(&[out(1, 0, "b\n"), items(1, 1, vec![output("c\n")])])).unwrap();
        let changes = shared.take_changes();
        assert!(changes.is_empty(), "{changes:?}");
    }

    // agents-log-paging 4.2: a fetched page folded in front.

    #[test]
    fn a_prepended_page_is_recorded_as_from_a_replay_and_a_bad_one_changes_nothing() {
        let mut shared = AgentShared::new();
        shared.replay_start();
        let window = Event::Window { from: EntryId(3), earlier: 2 };
        shared.apply(&values(&[window, added(3, agent("three"))])).unwrap();
        shared.replay_end();
        shared.take_changes();

        // Entry 2 alone leaves entry 1 earlier: `more` must say so.
        let refused = shared.prepend(&values(&[added(2, agent("two"))]), false).unwrap_err();
        assert!(refused.0.contains("couldn't be applied"), "{refused}");
        assert_eq!(shared.transcript().entries().count(), 1);
        assert!(shared.take_changes().is_empty());
        // Not an event at all.
        let refused = shared.prepend(&[serde_json::json!({"event": "nope"})], false).unwrap_err();
        assert!(refused.0.contains("couldn't be read"), "{refused}");

        shared.prepend(&values(&[added(2, agent("two"))]), true).unwrap();
        shared.prepend(&values(&[added(1, agent("one"))]), false).unwrap();
        let changes = shared.take_changes();
        assert_eq!(changes.prepended, Some(2));
        assert_eq!(changes.status_from, StatusFrom::Unchanged, "a page carries no status");
        assert_eq!(changes.added_from, None, "nothing new after the shown entries");
        assert!(shared.transcript().window().is_none());
        assert_eq!(shared.transcript().entries().count(), 3);
    }
}
