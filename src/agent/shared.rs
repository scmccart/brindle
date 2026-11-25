//! The fold an agent session's reader thread and its zone share (design.md
//! decision 8): the session's [`Transcript`], folded from the events the
//! terminal server sends, and what changed since the zone last looked
//! ([`Changes`]). It sits behind a mutex the sink and the entity share, a
//! leaf lock: nothing is taken under it. The zone copies what changed
//! into its model and never reads the lock while drawing (`process R14`).
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
    /// Something else changed (a tool call's fields, a permission's
    /// outcome): the zone copies the entry again.
    Replaced,
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
    /// A replay started: the transcript was rebuilt from nothing, and the
    /// zone starts over from it.
    pub reset: bool,
    /// Some of these changes were folded during a replay: what they hold
    /// already happened, and raises nothing.
    pub from_replay: bool,
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

    pub fn replaying(&self) -> bool {
        self.replaying
    }

    /// `REPLAY_START`: the replay rebuilds the transcript from nothing.
    pub fn replay_start(&mut self) {
        self.transcript = Transcript::new();
        self.replaying = true;
        self.changes = Changes { reset: true, from_replay: true, ..Changes::default() };
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
            | Event::Answered { .. }
            | Event::Queue { .. }
            | Event::Turn { .. }
            | Event::Agent { .. }
            | Event::Trimmed { .. } => None,
        };
        self.transcript
            .apply(event)
            .map_err(|why| Broken(format!("an event from the terminal server couldn't be applied: {why}")))?;
        let changes = &mut self.changes;
        changes.from_replay |= self.replaying;
        match event {
            Event::Added { id, .. } => changes.added(*id),
            Event::Appended { id, .. } => changes.appended(*id, before.unwrap_or(0)),
            Event::ToolChanged { id, .. } | Event::Answered { id, .. } => changes.replaced(*id),
            Event::Queue { .. } => changes.slot_changed = true,
            Event::Turn { .. } | Event::Agent { .. } => changes.turn_or_agent = true,
            Event::Trimmed { through, .. } => changes.trimmed(*through),
        }
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
        AgentState, HoldReason, NoticeLevel, PermissionKind, PermissionOption, PermissionOutcome, Slot, StopReason,
        ToolKind, ToolStatus, TurnState,
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
        Entry::Tool { call: "call_1".into(), title: "Read main.rs".into(), kind: ToolKind::Read, status: ToolStatus::Pending }
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
            Event::ToolChanged { id: EntryId(4), title: None, kind: None, status: Some(ToolStatus::Completed) },
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
        assert!(!changes.from_replay);
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
            assert!(changes.reset && changes.from_replay && changes.slot_changed && changes.turn_or_agent);
            assert_eq!(changes.added_from, Some(EntryId(1)));
            assert_eq!(shared.transcript(), &source, "the copy is the server's log");
            assert_eq!(shared.transcript().slot(), &slot);
            shared.apply(&values(&[Event::Turn { state: TurnState::Working }])).unwrap();
            let live = shared.take_changes();
            assert!(!live.from_replay && !live.reset && live.turn_or_agent);
        }
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
}
