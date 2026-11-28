use std::fmt;

use crate::output::output_fields_len;
use crate::{
    AgentState, Conversation, Entry, EntryId, Event, Kept, SessionOption, SlashCommand, Slot,
    StopReason, TEXT_CHUNK, ToolItem, Transcript, Trim, TurnState, Window,
};

/// An event [`Transcript::apply`] couldn't apply. The log is unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FoldError {
    /// An `Added` whose id isn't the next one.
    OutOfOrder { id: EntryId, expected: EntryId },
    /// An event for an entry the log doesn't have (never added, or trimmed).
    UnknownEntry { id: EntryId },
    /// An `Appended` for an entry with no text to grow.
    NotAppendable { id: EntryId },
    /// A `ToolChanged`, `ToolItems` or `ToolOutput` for an entry that isn't
    /// a tool call.
    NotATool { id: EntryId },
    /// A `ToolItems` whose `from` is past the end of the content.
    ItemsOutOfRange { id: EntryId, from: u32, len: u32 },
    /// A `ToolOutput` for an item that is missing or isn't an output.
    NotAnOutput { id: EntryId, item: u32 },
    /// An `Answered` for an entry that isn't a permission request.
    NotAPermission { id: EntryId },
    /// An `Answered` for a permission request that already has an outcome.
    AlreadyAnswered { id: EntryId },
    /// A `Trimmed` past the last entry of a log that has entries.
    TrimPastEnd { through: EntryId, next: EntryId },
    /// A `Trimmed` that doesn't go past the last one.
    TrimBehind { through: EntryId, trimmed: EntryId },
    /// A `Window` on a transcript that already holds entries.
    WindowAfterEntries,
    /// A `Window` that starts before the next entry's id.
    WindowBehind { from: EntryId, next: EntryId },
    /// A `Window` with no earlier entries: a page that holds every entry
    /// has no window.
    WindowEmpty { from: EntryId },
    /// A fetched page that doesn't end right before the held entries.
    PageGap { last: EntryId, from: EntryId },
    /// A fetched page whose entries and `more` don't fit the window: `more`
    /// with no entries, no `more` while earlier entries would remain, or
    /// more entries than the window has.
    WindowMismatch {
        earlier: u64,
        fetched: u64,
        more: bool,
    },
    /// A fetched page with entries for a transcript that holds every entry.
    NoWindow,
}

impl fmt::Display for FoldError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OutOfOrder { id, expected } => {
                write!(f, "entry {id} was added where entry {expected} was next")
            }
            Self::UnknownEntry { id } => write!(f, "there is no entry {id}"),
            Self::NotAppendable { id } => write!(f, "entry {id} has no text to append to"),
            Self::NotATool { id } => write!(f, "entry {id} isn't a tool call"),
            Self::ItemsOutOfRange { id, from, len } => write!(
                f,
                "tool call {id}'s content was replaced from item {from}, but it has {len}"
            ),
            Self::NotAnOutput { id, item } => {
                write!(f, "tool call {id} has no output item {item}")
            }
            Self::NotAPermission { id } => write!(f, "entry {id} isn't a permission request"),
            Self::AlreadyAnswered { id } => {
                write!(f, "permission request {id} was already answered")
            }
            Self::TrimPastEnd { through, next } => {
                write!(
                    f,
                    "trimmed through entry {through}, but the next entry is {next}"
                )
            }
            Self::TrimBehind { through, trimmed } => write!(
                f,
                "trimmed through entry {through}, but entries through {trimmed} were already"
            ),
            Self::WindowAfterEntries => {
                f.write_str("a page's window came after entries were already held")
            }
            Self::WindowBehind { from, next } => write!(
                f,
                "a page's window starts at entry {from}, but the next entry is {next}"
            ),
            Self::WindowEmpty { from } => {
                write!(f, "a page's window at entry {from} has no earlier entries")
            }
            Self::PageGap { last, from } => write!(
                f,
                "a fetched page ends at entry {last}, but the held entries start at {from}"
            ),
            Self::WindowMismatch {
                earlier,
                fetched,
                more,
            } => {
                let more = if *more {
                    "more to fetch"
                } else {
                    "nothing more to fetch"
                };
                write!(
                    f,
                    "a fetched page of {fetched} entries with {more} doesn't fit the {earlier} earlier entries"
                )
            }
            Self::NoWindow => {
                f.write_str("a fetched page came for a transcript that holds every entry")
            }
        }
    }
}

impl std::error::Error for FoldError {}

/// What the session list says an agent session is doing, in program
/// status's names (design decision 15).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Activity {
    Idle,
    Working,
    Blocked,
    Done,
    Error,
}

impl Transcript {
    /// A new session's log: no entries, the slot empty, no turn, the agent
    /// starting.
    pub fn new() -> Self {
        Self::default()
    }

    /// Applies one event, or says why it can't and leaves the log as it was.
    pub fn apply(&mut self, event: &Event) -> Result<(), FoldError> {
        if let Some(id) = entry_change(event)
            && self.window.is_some_and(|window| id < window.from)
        {
            // Not held: a later fetch brings the entry as it is then.
            return Ok(());
        }
        match event {
            Event::Added { id, entry } => {
                if *id != self.next {
                    return Err(FoldError::OutOfOrder {
                        id: *id,
                        expected: self.next,
                    });
                }
                let mut entry = entry.clone();
                entry.bound_tool();
                let size = entry.measure();
                self.entries.push_back(Kept {
                    id: *id,
                    entry,
                    size,
                });
                self.size += size;
                self.next = id.next();
            }
            Event::Appended { id, text } => {
                let kept = self.kept_mut(*id)?;
                let Some(own) = kept.entry.text_mut() else {
                    return Err(FoldError::NotAppendable { id: *id });
                };
                own.push_str(text);
                let grown = escaped_len(text);
                kept.size += grown;
                self.size += grown;
            }
            Event::ToolChanged {
                id,
                title,
                kind,
                status,
                locations,
                input,
                result,
                more,
            } => {
                let kept = self.kept_mut(*id)?;
                let Entry::Tool {
                    title: own_title,
                    kind: own_kind,
                    status: own_status,
                    locations: own_locations,
                    input: own_input,
                    result: own_result,
                    more: own_more,
                    ..
                } = &mut kept.entry
                else {
                    return Err(FoldError::NotATool { id: *id });
                };
                if let Some(title) = title {
                    own_title.clone_from(title);
                }
                if let Some(kind) = kind {
                    *own_kind = *kind;
                }
                if let Some(status) = status {
                    *own_status = *status;
                }
                if let Some(locations) = locations {
                    own_locations.clone_from(locations);
                }
                if let Some(input) = input {
                    own_input.clone_from(input);
                }
                if let Some(result) = result {
                    own_result.clone_from(result);
                }
                if let Some(more) = more {
                    *own_more = *more;
                }
                kept.entry.bound_tool();
                self.resize_entry(*id);
            }
            Event::ToolItems { id, from, items } => {
                let kept = self.kept_mut(*id)?;
                let Entry::Tool { content, .. } = &mut kept.entry else {
                    return Err(FoldError::NotATool { id: *id });
                };
                let len = u32::try_from(content.len()).unwrap_or(u32::MAX);
                if *from > len {
                    return Err(FoldError::ItemsOutOfRange {
                        id: *id,
                        from: *from,
                        len,
                    });
                }
                content.truncate(*from as usize);
                content.extend(items.iter().cloned());
                kept.entry.bound_tool();
                self.resize_entry(*id);
            }
            Event::ToolOutput {
                id,
                item,
                text,
                exit,
            } => {
                let kept = self.kept_mut(*id)?;
                let Entry::Tool { content, .. } = &mut kept.entry else {
                    return Err(FoldError::NotATool { id: *id });
                };
                let Some(ToolItem::Output(output)) = content.get_mut(*item as usize) else {
                    return Err(FoldError::NotAnOutput { id: *id, item: *item });
                };
                let before = output_fields_len(output);
                let dropped = output.append(text);
                if let Some(exit) = exit {
                    let mut exit = exit.clone();
                    exit.bound();
                    output.exit = Some(exit);
                }
                let after = output_fields_len(output);
                // Grown by the text and shrunk by what was dropped, with no
                // re-encode per piece; `omitted`, `partial` and `exit` are
                // measured alone.
                let old = kept.size;
                let size = (old + escaped_len(text) + after).saturating_sub(dropped + before);
                kept.size = size;
                self.size = self.size.saturating_sub(old) + size;
            }
            Event::Answered { id, outcome } => {
                let kept = self.kept_mut(*id)?;
                let Entry::Permission {
                    outcome: own_outcome,
                    ..
                } = &mut kept.entry
                else {
                    return Err(FoldError::NotAPermission { id: *id });
                };
                if own_outcome.is_some() {
                    return Err(FoldError::AlreadyAnswered { id: *id });
                }
                *own_outcome = Some(outcome.clone());
                self.resize_entry(*id);
            }
            Event::Queue { slot } => self.slot = slot.clone(),
            Event::Turn { state } => self.turn = state.clone(),
            Event::Agent { state } => self.agent = state.clone(),
            Event::Conversation { conversation } => {
                self.conversation = Some(conversation.clone());
            }
            Event::Trimmed { through, dropped } => self.trim(*through, *dropped)?,
            Event::Commands { commands } => self.commands.clone_from(commands),
            Event::Options { options } => self.options.clone_from(options),
            Event::Window { from, earlier } => {
                if !self.entries.is_empty() {
                    return Err(FoldError::WindowAfterEntries);
                }
                if *from < self.next {
                    return Err(FoldError::WindowBehind {
                        from: *from,
                        next: self.next,
                    });
                }
                if *earlier == 0 {
                    return Err(FoldError::WindowEmpty { from: *from });
                }
                self.next = *from;
                self.window = Some(Window {
                    from: *from,
                    earlier: *earlier,
                });
            }
        }
        Ok(())
    }

    fn trim(&mut self, through: EntryId, dropped: u64) -> Result<(), FoldError> {
        self.trim_entries(through, dropped)?;
        if let Some(window) = self.window {
            // Below the window, the trim takes entries the server kept and
            // this copy never held: fewer remain to fetch. At or past it,
            // the held entries were dropped through it, so nothing earlier
            // is left.
            let earlier = if through < window.from {
                window
                    .earlier
                    .min(window.from.0.saturating_sub(through.0).saturating_sub(1))
            } else {
                0
            };
            self.window = (earlier > 0).then_some(Window {
                from: window.from,
                earlier,
            });
        }
        Ok(())
    }

    fn trim_entries(&mut self, through: EntryId, dropped: u64) -> Result<(), FoldError> {
        if let Some(trim) = self.trimmed
            && through <= trim.through
        {
            return Err(FoldError::TrimBehind {
                through,
                trimmed: trim.through,
            });
        }
        if through >= self.next && !self.entries.is_empty() {
            return Err(FoldError::TrimPastEnd {
                through,
                next: self.next,
            });
        }
        while let Some(front) = self.entries.front()
            && front.id <= through
        {
            self.size = self.size.saturating_sub(front.size);
            self.entries.pop_front();
        }
        if through >= self.next {
            self.next = through.next();
        }
        let before = self.trimmed.map_or(0, |trim| trim.dropped);
        self.trimmed = Some(Trim {
            through,
            dropped: before.saturating_add(dropped),
        });
        Ok(())
    }

    fn index(&self, id: EntryId) -> Result<usize, FoldError> {
        self.entries
            .binary_search_by_key(&id, |kept| kept.id)
            .map_err(|_| FoldError::UnknownEntry { id })
    }

    fn kept_mut(&mut self, id: EntryId) -> Result<&mut Kept, FoldError> {
        let index = self.index(id)?;
        self.entries
            .get_mut(index)
            .ok_or(FoldError::UnknownEntry { id })
    }

    /// Measures an entry again after a change that isn't an append.
    fn resize_entry(&mut self, id: EntryId) {
        if let Ok(index) = self.index(id)
            && let Some(kept) = self.entries.get_mut(index)
        {
            let size = kept.entry.measure();
            self.size = self.size.saturating_sub(kept.size) + size;
            kept.size = size;
        }
    }

    /// The queue slot.
    pub fn slot(&self) -> &Slot {
        &self.slot
    }

    pub fn turn(&self) -> &TurnState {
        &self.turn
    }

    pub fn agent(&self) -> &AgentState {
        &self.agent
    }

    /// The agent's conversation, once it gave one; it stays after the
    /// agent ends.
    pub fn conversation(&self) -> Option<&Conversation> {
        self.conversation.as_ref()
    }

    /// The agent's slash commands, as it last listed them; empty while the
    /// agent offers none or isn't running.
    pub fn commands(&self) -> &[SlashCommand] {
        &self.commands
    }

    /// The session's options, as the agent last listed them; empty while
    /// the agent offers none or isn't running.
    pub fn options(&self) -> &[SessionOption] {
        &self.options
    }

    /// The kept entries, oldest first.
    pub fn entries(&self) -> impl Iterator<Item = (EntryId, &Entry)> {
        self.entries.iter().map(|kept| (kept.id, &kept.entry))
    }

    pub fn entry(&self, id: EntryId) -> Option<&Entry> {
        let index = self.index(id).ok()?;
        self.entries.get(index).map(|kept| &kept.entry)
    }

    /// The id the next entry gets.
    pub fn next_id(&self) -> EntryId {
        self.next
    }

    /// How many entries were trimmed in all.
    pub fn dropped(&self) -> u64 {
        self.trimmed.map_or(0, |trim| trim.dropped)
    }

    /// The kept entries' encoded length, each with its [`ENTRY_FRAMING`]:
    /// what the log's bound, a trim and a page's budget count.
    pub fn encoded_len(&self) -> usize {
        self.size
    }

    /// What the session list shows for this log (design decision 15). The
    /// slot doesn't change it.
    pub fn activity(&self) -> Activity {
        match &self.agent {
            AgentState::Starting | AgentState::LoggingIn { .. } => Activity::Working,
            // Nothing runs on the user's behalf while a conversation is
            // chosen (`agents-session-list`'s "A choosing zone waits
            // quietly").
            AgentState::Choosing { .. } => Activity::Idle,
            AgentState::LoginRequired { .. }
            | AgentState::Failed { .. }
            | AgentState::Exited { .. }
            | AgentState::Lost { .. } => Activity::Error,
            AgentState::Ready { .. } => match &self.turn {
                TurnState::Working | TurnState::Cancelling => Activity::Working,
                TurnState::Blocked => Activity::Blocked,
                TurnState::Idle => Activity::Idle,
                TurnState::Ended { stop } => match stop {
                    StopReason::EndTurn => Activity::Done,
                    StopReason::Cancelled => Activity::Idle,
                    StopReason::MaxTokens
                    | StopReason::MaxTurnRequests
                    | StopReason::Refusal
                    | StopReason::Other { .. } => Activity::Error,
                },
                TurnState::Failed { .. } => Activity::Error,
            },
        }
    }

    /// Drops the oldest entries whole until the kept ones' encoded length is
    /// at most `limit`, always keeping the newest, and returns the `Trimmed`
    /// it applied for them; `None` when nothing was dropped.
    pub fn trim_to(&mut self, limit: usize) -> Option<Event> {
        let mut size = self.size;
        let mut count: u64 = 0;
        let mut through = None;
        for kept in self
            .entries
            .iter()
            .take(self.entries.len().saturating_sub(1))
        {
            if size <= limit {
                break;
            }
            size = size.saturating_sub(kept.size);
            count += 1;
            through = Some(kept.id);
        }
        let event = Event::Trimmed {
            through: through?,
            dropped: count,
        };
        self.apply(&event).ok().map(|()| event)
    }
}

/// The entry a change to an existing entry names: the events a paged copy
/// ignores below its window.
pub(crate) fn entry_change(event: &Event) -> Option<EntryId> {
    match event {
        Event::Appended { id, .. }
        | Event::ToolChanged { id, .. }
        | Event::ToolItems { id, .. }
        | Event::ToolOutput { id, .. }
        | Event::Answered { id, .. } => Some(*id),
        Event::Added { .. }
        | Event::Queue { .. }
        | Event::Turn { .. }
        | Event::Agent { .. }
        | Event::Conversation { .. }
        | Event::Trimmed { .. }
        | Event::Commands { .. }
        | Event::Options { .. }
        | Event::Window { .. } => None,
    }
}

/// `text` in pieces of at most [`TEXT_CHUNK`] bytes, cut at character
/// boundaries; one empty piece for empty text.
pub fn chunks(text: &str) -> impl Iterator<Item = &str> {
    let mut rest = Some(text);
    std::iter::from_fn(move || {
        let left = rest?;
        if left.len() <= TEXT_CHUNK {
            rest = None;
            return Some(left);
        }
        let mut cut = TEXT_CHUNK;
        while !left.is_char_boundary(cut) {
            cut -= 1;
        }
        let (piece, after) = left.split_at(cut);
        rest = Some(after);
        Some(piece)
    })
}

/// How many bytes `text` takes inside a JSON string, as serde_json escapes
/// it: `"` and `\` take two, the control characters two (`\b`, `\t`, `\n`,
/// `\f`, `\r`) or six (`\u00XX`). Cleaned text ([`clean`]) takes at most
/// twice its bytes.
pub fn escaped_len(text: &str) -> usize {
    text.bytes()
        .map(|byte| match byte {
            b'"' | b'\\' | 0x08 | b'\t' | b'\n' | 0x0c | b'\r' => 2,
            0x00..=0x1f => 6,
            _ => 1,
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fuzz::XorShift;
    use crate::harness::*;
    use crate::*;

    #[test]
    fn chunks_append_to_one_entry() {
        let transcript = fold(&[
            added(1, agent("Hel")),
            Event::Appended {
                id: EntryId(1),
                text: "lo".into(),
            },
            Event::Appended {
                id: EntryId(1),
                text: "!".into(),
            },
        ])
        .unwrap();
        let entries: Vec<_> = transcript.entries().collect();
        assert_eq!(entries, [(EntryId(1), &agent("Hello!"))]);
        assert_eq!(transcript.next_id(), EntryId(2));
    }

    #[test]
    fn a_tool_changes_by_id_and_only_in_the_fields_given() {
        let mut transcript = fold(&[added(1, tool("call_1")), added(2, tool("call_2"))]).unwrap();
        transcript
            .apply(&changed(2, None, None, Some(ToolStatus::Completed)))
            .unwrap();
        transcript
            .apply(&changed(1, Some("Edit notes.md"), Some(ToolKind::Edit), None))
            .unwrap();
        assert_eq!(
            transcript.entry(EntryId(1)),
            Some(&Entry::tool(
                "call_1".into(),
                "Edit notes.md".into(),
                ToolKind::Edit,
                ToolStatus::Pending,
            ))
        );
        assert_eq!(
            transcript.entry(EntryId(2)),
            Some(&Entry::tool(
                "call_2".into(),
                "Read notes.md".into(),
                ToolKind::Read,
                ToolStatus::Completed,
            ))
        );
    }

    #[test]
    fn an_answer_goes_only_on_a_permission_entry() {
        let outcome = PermissionOutcome::Selected {
            option: "allow-once".into(),
        };
        let mut transcript = fold(&[added(1, agent("hi")), added(2, permission())]).unwrap();
        let before = transcript.clone();
        assert_eq!(
            transcript.apply(&Event::Answered {
                id: EntryId(1),
                outcome: outcome.clone(),
            }),
            Err(FoldError::NotAPermission { id: EntryId(1) })
        );
        assert_eq!(transcript, before, "a refused event changes nothing");
        transcript
            .apply(&Event::Answered {
                id: EntryId(2),
                outcome: outcome.clone(),
            })
            .unwrap();
        let Some(Entry::Permission { outcome: kept, .. }) = transcript.entry(EntryId(2)) else {
            panic!("not a permission entry");
        };
        assert_eq!(kept, &Some(outcome));
    }

    #[test]
    fn queue_sets_the_slot_from_any_slot_and_never_fails() {
        let slots = [
            Slot::Empty,
            Slot::Waiting {
                text: "next".into(),
            },
            Slot::Held {
                text: "held".into(),
                reason: HoldReason::Stopped {
                    stop: StopReason::MaxTokens,
                },
            },
        ];
        for from in &slots {
            for to in &slots {
                let mut transcript = Transcript::new();
                transcript
                    .apply(&Event::Queue { slot: from.clone() })
                    .unwrap();
                transcript
                    .apply(&Event::Queue { slot: to.clone() })
                    .unwrap();
                assert_eq!(transcript.slot(), to);
            }
        }
    }

    #[test]
    fn every_fold_error_is_returned_and_changes_nothing() {
        let base = fold(&[
            added(1, agent("a")),
            added(2, tool("call_1")),
            added(3, permission()),
            Event::Answered {
                id: EntryId(3),
                outcome: PermissionOutcome::Cancelled,
            },
            added(
                4,
                Entry::Notice {
                    level: NoticeLevel::Info,
                    text: "n".into(),
                },
            ),
            Event::Trimmed {
                through: EntryId(1),
                dropped: 1,
            },
        ])
        .unwrap();
        let cases = [
            (
                added(9, agent("x")),
                FoldError::OutOfOrder {
                    id: EntryId(9),
                    expected: EntryId(5),
                },
            ),
            (
                added(4, agent("x")),
                FoldError::OutOfOrder {
                    id: EntryId(4),
                    expected: EntryId(5),
                },
            ),
            (
                Event::Appended {
                    id: EntryId(1),
                    text: "trimmed".into(),
                },
                FoldError::UnknownEntry { id: EntryId(1) },
            ),
            (
                Event::Appended {
                    id: EntryId(7),
                    text: "never added".into(),
                },
                FoldError::UnknownEntry { id: EntryId(7) },
            ),
            (
                Event::Appended {
                    id: EntryId(2),
                    text: "x".into(),
                },
                FoldError::NotAppendable { id: EntryId(2) },
            ),
            (
                Event::Appended {
                    id: EntryId(4),
                    text: "x".into(),
                },
                FoldError::NotAppendable { id: EntryId(4) },
            ),
            (
                changed(3, None, None, Some(ToolStatus::Failed)),
                FoldError::NotATool { id: EntryId(3) },
            ),
            (
                Event::Answered {
                    id: EntryId(2),
                    outcome: PermissionOutcome::Cancelled,
                },
                FoldError::NotAPermission { id: EntryId(2) },
            ),
            (
                Event::Answered {
                    id: EntryId(3),
                    outcome: PermissionOutcome::Cancelled,
                },
                FoldError::AlreadyAnswered { id: EntryId(3) },
            ),
            (
                Event::Trimmed {
                    through: EntryId(5),
                    dropped: 4,
                },
                FoldError::TrimPastEnd {
                    through: EntryId(5),
                    next: EntryId(5),
                },
            ),
            (
                Event::Trimmed {
                    through: EntryId(1),
                    dropped: 1,
                },
                FoldError::TrimBehind {
                    through: EntryId(1),
                    trimmed: EntryId(1),
                },
            ),
        ];
        for (event, error) in cases {
            let mut transcript = base.clone();
            assert_eq!(transcript.apply(&event), Err(error.clone()), "{event:?}");
            assert_eq!(transcript, base, "{event:?} changed the log");
            assert!(!error.to_string().is_empty());
        }
    }

    #[test]
    fn activity_follows_the_table_for_every_row() {
        let failed = || TurnState::Failed {
            message: "m".into(),
        };
        let rows = [
            (AgentState::Starting, TurnState::Idle, Activity::Working),
            (AgentState::Choosing { message: None }, TurnState::Idle, Activity::Idle),
            (ready(), TurnState::Working, Activity::Working),
            (ready(), TurnState::Cancelling, Activity::Working),
            (ready(), TurnState::Blocked, Activity::Blocked),
            (ready(), ended(StopReason::EndTurn), Activity::Done),
            (ready(), TurnState::Idle, Activity::Idle),
            (ready(), ended(StopReason::Cancelled), Activity::Idle),
            (ready(), ended(StopReason::MaxTokens), Activity::Error),
            (ready(), ended(StopReason::MaxTurnRequests), Activity::Error),
            (ready(), ended(StopReason::Refusal), Activity::Error),
            (
                ready(),
                ended(StopReason::Other {
                    name: "later".into(),
                }),
                Activity::Error,
            ),
            (ready(), failed(), Activity::Error),
            (
                AgentState::LoginRequired {
                    message: "log in".into(),
                    methods: Vec::new(),
                },
                failed(),
                Activity::Error,
            ),
            (
                AgentState::LoginRequired {
                    message: "log in".into(),
                    methods: vec![agent_method("chat-gpt", "ChatGPT")],
                },
                TurnState::Idle,
                Activity::Error,
            ),
            (
                AgentState::LoggingIn {
                    method: "chat-gpt".into(),
                    name: "ChatGPT".into(),
                },
                TurnState::Idle,
                Activity::Working,
            ),
            (
                AgentState::LoggingIn {
                    method: "chat-gpt".into(),
                    name: "ChatGPT".into(),
                },
                failed(),
                Activity::Working,
            ),
            (
                AgentState::Failed {
                    message: "m".into(),
                },
                TurnState::Working,
                Activity::Error,
            ),
            (
                AgentState::Exited {
                    code: Some(1),
                    stderr: String::new(),
                },
                ended(StopReason::EndTurn),
                Activity::Error,
            ),
            (
                AgentState::Lost {
                    message: "its holder ended".into(),
                },
                TurnState::Failed {
                    message: "the agent exited".into(),
                },
                Activity::Error,
            ),
        ];
        for (agent, turn, activity) in rows {
            let transcript = with(agent.clone(), turn.clone());
            assert_eq!(transcript.activity(), activity, "{agent:?}, {turn:?}");
        }
    }

    #[test]
    fn the_slot_doesnt_change_the_activity() {
        for slot in [
            Slot::Waiting { text: "w".into() },
            Slot::Held {
                text: "h".into(),
                reason: HoldReason::Cancelled,
            },
        ] {
            let mut transcript = with(ready(), ended(StopReason::EndTurn));
            transcript.apply(&Event::Queue { slot }).unwrap();
            assert_eq!(transcript.activity(), Activity::Done);
        }
    }

    #[test]
    fn trimming_keeps_the_newest_entries_and_its_event_drops_the_same_ones() {
        let events: Vec<Event> = (1..=10)
            .map(|id| added(id, agent(&"x".repeat(100))))
            .collect();
        let mut trimmed = fold(&events).unwrap();
        let mut copy = trimmed.clone();
        let one = trimmed
            .entries()
            .next()
            .map(|(_, entry)| entry.measure())
            .unwrap();
        let event = trimmed.trim_to(one * 3).expect("over the limit");
        assert_eq!(
            event,
            Event::Trimmed {
                through: EntryId(7),
                dropped: 7
            }
        );
        let kept: Vec<EntryId> = trimmed.entries().map(|(id, _)| id).collect();
        assert_eq!(kept, [EntryId(8), EntryId(9), EntryId(10)]);
        assert!(trimmed.encoded_len() <= one * 3);
        assert_eq!(trimmed.dropped(), 7);
        copy.apply(&event).unwrap();
        assert_eq!(copy, trimmed);
        // Within the limit, nothing is dropped.
        assert_eq!(trimmed.trim_to(one * 3), None);
        // The newest entry stays even when it alone is over the limit.
        let event = trimmed.trim_to(0).unwrap();
        assert_eq!(
            event,
            Event::Trimmed {
                through: EntryId(9),
                dropped: 2
            }
        );
        assert_eq!(trimmed.entries().count(), 1);
        assert_eq!(trimmed.dropped(), 9);
        assert_eq!(trimmed.trim_to(0), None);
        assert_eq!(fold(&trimmed.replay()), Ok(trimmed));
    }

    #[test]
    fn the_encoded_length_follows_appends_and_changes() {
        let mut transcript = fold(&[
            added(1, agent("a\"b")),
            Event::Appended {
                id: EntryId(1),
                text: "\n\u{1}\\é".into(),
            },
            added(2, tool("call_1")),
            changed(
                2,
                Some("a longer title than before"),
                None,
                Some(ToolStatus::InProgress),
            ),
            added(3, permission()),
        ])
        .unwrap();
        transcript
            .apply(&Event::Answered {
                id: EntryId(3),
                outcome: PermissionOutcome::Cancelled,
            })
            .unwrap();
        assert_eq!(transcript.encoded_len(), measured(&transcript));
    }

    #[test]
    fn the_login_states_fold_and_replay_as_the_agents_state() {
        let required = AgentState::LoginRequired {
            message: "log in".into(),
            methods: vec![agent_method("chat-gpt", "ChatGPT"), terminal_method()],
        };
        let logging_in = AgentState::LoggingIn {
            method: "chat-gpt".into(),
            name: "ChatGPT".into(),
        };
        let mut transcript = with(required.clone(), TurnState::Idle);
        assert_eq!(transcript.agent(), &required);
        assert_eq!(fold(&transcript.replay()), Ok(transcript.clone()));
        transcript
            .apply(&Event::Agent {
                state: logging_in.clone(),
            })
            .unwrap();
        assert_eq!(transcript.agent(), &logging_in);
        assert_eq!(transcript.replay().last(), Some(&Event::Agent { state: logging_in }));
        assert_eq!(fold(&transcript.replay()), Ok(transcript));
    }

    #[test]
    fn tool_items_replace_from_their_index_and_refuse_past_the_end() {
        let mut transcript = fold(&[added(1, tool("c1")), added(2, agent("a"))]).unwrap();
        transcript
            .apply(&items(1, 0, vec![text_item("a"), text_item("b")]))
            .unwrap();
        assert_eq!(content_of(&transcript, 1), [text_item("a"), text_item("b")]);
        // Appended at the length.
        transcript.apply(&items(1, 2, vec![diff_item()])).unwrap();
        assert_eq!(content_of(&transcript, 1).len(), 3);
        // Replaced from 1.
        transcript.apply(&items(1, 1, vec![text_item("c")])).unwrap();
        assert_eq!(content_of(&transcript, 1), [text_item("a"), text_item("c")]);
        assert_eq!(transcript.encoded_len(), measured(&transcript));
        // Cleared.
        transcript.apply(&items(1, 0, Vec::new())).unwrap();
        assert_eq!(content_of(&transcript, 1), []);
        assert_eq!(transcript.encoded_len(), measured(&transcript));

        transcript.apply(&items(1, 0, vec![output_item(None)])).unwrap();
        let base = transcript.clone();
        for (event, error) in [
            (
                items(1, 2, vec![text_item("x")]),
                FoldError::ItemsOutOfRange {
                    id: EntryId(1),
                    from: 2,
                    len: 1,
                },
            ),
            (
                items(2, 0, Vec::new()),
                FoldError::NotATool { id: EntryId(2) },
            ),
            (
                output(2, 0, "x", None),
                FoldError::NotATool { id: EntryId(2) },
            ),
            (
                output(1, 1, "x", None),
                FoldError::NotAnOutput {
                    id: EntryId(1),
                    item: 1,
                },
            ),
            (items(9, 0, Vec::new()), FoldError::UnknownEntry { id: EntryId(9) }),
        ] {
            let mut copy = base.clone();
            assert_eq!(copy.apply(&event), Err(error.clone()), "{event:?}");
            assert_eq!(copy, base, "{event:?} changed the log");
            assert!(!error.to_string().is_empty());
        }
        let mut copy = fold(&[added(1, tool("c1"))]).unwrap();
        copy.apply(&items(1, 0, vec![text_item("t")])).unwrap();
        let base = copy.clone();
        assert_eq!(
            copy.apply(&output(1, 0, "x", None)),
            Err(FoldError::NotAnOutput {
                id: EntryId(1),
                item: 0
            })
        );
        assert_eq!(copy, base);
    }

    #[test]
    fn tool_changed_sets_clears_and_keeps_the_new_fields() {
        let mut transcript = fold(&[added(1, tool("c1"))]).unwrap();
        let location = Location {
            path: "/p".into(),
            line: None,
        };
        transcript
            .apply(&Event::ToolChanged {
                id: EntryId(1),
                title: None,
                kind: None,
                status: None,
                locations: Some(vec![location.clone()]),
                input: Some(Some("in".into())),
                result: Some(Some("out".into())),
                more: Some(4),
            })
            .unwrap();
        transcript
            .apply(&Event::ToolChanged {
                id: EntryId(1),
                title: None,
                kind: None,
                status: None,
                locations: None,
                input: Some(None),
                result: None,
                more: None,
            })
            .unwrap();
        let Some(Entry::Tool {
            locations,
            input,
            result,
            more,
            ..
        }) = transcript.entry(EntryId(1))
        else {
            panic!()
        };
        assert_eq!(
            (locations, input, result, more),
            (&vec![location], &None, &Some("out".into()), &4)
        );
        assert_eq!(transcript.encoded_len(), measured(&transcript));
    }

    #[test]
    fn the_log_trims_large_tool_entries_whole() {
        let big = ToolItem::Text {
            text: "t".repeat(ITEM_LIMIT),
            cut: true,
        };
        let mut events = Vec::new();
        for id in 1..=40u64 {
            events.push(added(id, tool("c1")));
            events.push(items(id, 0, vec![big.clone(), big.clone(), big.clone()]));
        }
        let mut transcript = fold(&events).unwrap();
        let entry = transcript.encoded_len() / 40;
        assert!(entry > 3 * ITEM_LIMIT);
        let trimmed = transcript.trim_to(12 * entry).unwrap();
        assert_eq!(
            trimmed,
            Event::Trimmed {
                through: EntryId(28),
                dropped: 28
            }
        );
        assert_eq!(transcript.entries().count(), 12);
        assert_eq!(transcript.encoded_len(), measured(&transcript));
        assert_eq!(fold(&transcript.replay()), Ok(transcript));
    }

    #[test]
    fn commands_and_options_replace_whole_and_an_empty_list_clears() {
        let mut transcript = Transcript::new();
        let set = |transcript: &mut Transcript, event: Event| transcript.apply(&event).unwrap();
        set(
            &mut transcript,
            Event::Commands {
                commands: vec![compact(), compact()],
            },
        );
        set(
            &mut transcript,
            Event::Commands {
                commands: vec![compact()],
            },
        );
        assert_eq!(transcript.commands(), [compact()]);
        set(
            &mut transcript,
            Event::Options {
                options: vec![mode_option("ask"), fast_option()],
            },
        );
        set(
            &mut transcript,
            Event::Options {
                options: vec![mode_option("code")],
            },
        );
        assert_eq!(transcript.options(), [mode_option("code")]);
        set(&mut transcript, Event::Commands { commands: vec![] });
        set(&mut transcript, Event::Options { options: vec![] });
        assert!(transcript.commands().is_empty() && transcript.options().is_empty());
        assert_eq!(transcript, Transcript::new());
    }

    // The fold's bounds (OpenSpec change `agents-event-bounds`).

    /// A fold of a tool call update carrying 40 items and a 100 KiB raw
    /// input holds at most 32 items and a raw input of at most 16 KiB
    /// (B-7).
    #[test]
    fn a_fold_of_a_tool_update_past_the_bounds_holds_them() {
        let mut transcript = fold(&[added(1, tool("call_1"))]).unwrap();
        let raw = "x".repeat(100 * 1024);
        transcript
            .apply(&Event::ToolChanged {
                id: EntryId(1),
                title: None,
                kind: None,
                status: None,
                locations: None,
                input: Some(Some(raw)),
                result: None,
                more: None,
            })
            .unwrap();
        let forty: Vec<ToolItem> = (0..40).map(|n| text_item(&format!("item {n}"))).collect();
        transcript.apply(&items(1, 0, forty)).unwrap();
        let Some(Entry::Tool {
            content,
            more,
            input,
            ..
        }) = transcript.entry(EntryId(1))
        else {
            panic!("no tool entry");
        };
        assert!(content.len() <= MAX_ITEMS, "{} items", content.len());
        assert!(*more >= 8, "more {more}");
        let input = input.as_deref().unwrap_or_default();
        assert!(input.len() <= RAW_LIMIT, "a raw input of {}", input.len());
        assert!(input.ends_with('…'));
    }

    /// A fold of grouped pieces equals a fold of one item an event, which is
    /// how a format 2 checkpoint written before replays its tool entries.
    #[test]
    fn a_fold_of_grouped_pieces_equals_a_fold_of_one_item_an_event() {
        let mut rng = XorShift::new(0xeb_0003);
        for _ in 0..300 {
            let content: Vec<ToolItem> =
                (0..rng.below(12)).map(|_| crate::fuzz::tool_item(&mut rng)).collect();
            let entry = tool_with("call_1", content, None);
            let log = fold(&[added(1, entry)]).unwrap();
            let Some(Entry::Tool { content, .. }) = log.entry(EntryId(1)) else {
                unreachable!()
            };
            let mut one_each = vec![added(1, tool("call_1"))];
            for (at, item) in content.iter().enumerate() {
                one_each.push(items(1, at as u32, vec![item.clone()]));
            }
            let mut header = fold(&one_each).unwrap();
            // The header's `more` is the entry's, as the replay's `Added`
            // carries it.
            if let Some(Entry::Tool { more, .. }) = log.entry(EntryId(1)) {
                header.apply(&Event::ToolChanged {
                    id: EntryId(1),
                    title: None,
                    kind: None,
                    status: None,
                    locations: None,
                    input: None,
                    result: None,
                    more: Some(*more),
                })
                .unwrap();
            }
            assert_eq!(fold(&log.replay()).unwrap(), log);
            assert_eq!(header.entry(EntryId(1)), log.entry(EntryId(1)));
        }
    }
}
