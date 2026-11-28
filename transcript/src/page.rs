use crate::{Entry, EntryId, Event, FoldError, Slot, Transcript, chunks, item_pieces};

/// The most a page or a fetched page holds, in entries' encoded length
/// (OpenSpec change `agents-log-paging`, design decision 3).
pub const PAGE_BYTES: usize = 256 * 1024;

/// What a paged transcript doesn't hold: the entries before `from`, of
/// which `earlier` (never 0) are kept on the server and can be fetched.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Window {
    pub from: EntryId,
    pub earlier: u64,
}

/// A fetch's answer ([`Transcript::fetch`]): the entries' events, in id
/// order, and whether kept entries remain before the first of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    pub events: Vec<Event>,
    pub more: bool,
}

impl Transcript {
    /// What this copy doesn't hold, when it was paged; the server's log
    /// never has a window.
    pub fn window(&self) -> Option<Window> {
        self.window
    }

    /// The events that rebuild this log in a fresh one: `Trimmed` when
    /// entries were dropped, `Window` when this copy holds only a suffix,
    /// each entry's `Added` with its text split into `Appended`s of at most
    /// [`TEXT_CHUNK`] bytes and a permission's outcome as its `Answered`,
    /// the slot's `Queue` when it isn't empty, the `Conversation` when there
    /// is one, then `Turn` and `Agent`, then `Commands` and `Options` when
    /// each isn't empty.
    pub fn replay(&self) -> Vec<Event> {
        self.replay_from(0)
    }

    /// [`Transcript::replay`] cut at a start entry (design decision 3): the
    /// newest whole entries whose encoded lengths add up to at most
    /// `budget`, always the newest one, reaching back to the oldest pending
    /// permission and to the tool call each pending permission names (the
    /// newest tool entry with that call before it; OpenSpec change
    /// `agents-tool-content`, design decision 15); then a `Window` naming
    /// how many kept entries precede them. Everything that isn't an entry
    /// (the slot, the conversation, the turn, the agent's state, its
    /// commands and options) is in every page, outside the budget. A log that fits the budget pages as its `replay()`.
    pub fn page(&self, budget: usize) -> Vec<Event> {
        let mut start = self.entries.len() - self.newest_within(self.entries.len(), budget);
        for (index, kept) in self.entries.iter().enumerate() {
            let Entry::Permission {
                outcome: None,
                call,
                ..
            } = &kept.entry
            else {
                continue;
            };
            start = start.min(index);
            if let Some(call) = call
                && let Some(tool) = self.entries.range(..index).rposition(|kept| {
                    matches!(&kept.entry, Entry::Tool { call: own, .. } if own == call)
                })
            {
                start = start.min(tool);
            }
        }
        self.replay_from(start)
    }

    /// The newest whole kept entries with ids below `before` whose encoded
    /// lengths add up to at most `budget` (at least one when any exists),
    /// each as it stands now, in id order; `more` says whether kept
    /// entries remain before the first of them. A fetch carries entries
    /// only: the state after them is already the client's.
    pub fn fetch(&self, before: EntryId, budget: usize) -> Fetched {
        let end = self.entries.partition_point(|kept| kept.id < before);
        let start = end - self.newest_within(end, budget);
        let mut events = Vec::new();
        for kept in self.entries.range(start..end) {
            replay_entry(kept.id, &kept.entry, &mut events);
        }
        Fetched {
            events,
            more: start > 0 || self.window.is_some(),
        }
    }

    /// How many of the kept entries before index `end`, newest first, fit
    /// in `budget`; at least one when there is one.
    fn newest_within(&self, end: usize, budget: usize) -> usize {
        let mut total = 0usize;
        let mut taken = 0usize;
        for kept in self.entries.range(..end).rev() {
            total = total.saturating_add(kept.size);
            if taken > 0 && total > budget {
                break;
            }
            taken += 1;
        }
        taken
    }

    /// Folds a fetched page in front of the held entries (design decision
    /// 4): its entries must end right before the window's `from`; they go
    /// in front, the window's count drops by theirs, and the window goes
    /// when `more` is false. An empty page with `more` false on a copy
    /// with no window is the answer to a fetch a trim overtook, and changes
    /// nothing. A refused page changes nothing.
    pub fn prepend(&mut self, events: &[Event], more: bool) -> Result<(), FoldError> {
        let Some(first) = events.iter().find_map(|event| match event {
            Event::Added { id, .. } => Some(*id),
            Event::Appended { .. }
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
        }) else {
            return match self.window {
                None if !more => Ok(()),
                window => Err(FoldError::WindowMismatch {
                    earlier: window.map_or(0, |window| window.earlier),
                    fetched: 0,
                    more,
                }),
            };
        };
        let Some(window) = self.window else {
            return Err(FoldError::NoWindow);
        };
        let mut page = Self {
            next: first,
            ..Self::default()
        };
        for event in events {
            page.apply(event)?;
        }
        let last = page.entries.back().map_or(first, |kept| kept.id);
        if page.window.is_some() || last.next() != window.from {
            return Err(FoldError::PageGap {
                last,
                from: window.from,
            });
        }
        let fetched = page.entries.len() as u64;
        let earlier = window.earlier.checked_sub(fetched);
        let earlier = match (earlier, more) {
            (Some(earlier), true) if earlier > 0 => Some(earlier),
            (Some(0), false) => None,
            (Some(_) | None, true | false) => {
                return Err(FoldError::WindowMismatch {
                    earlier: window.earlier,
                    fetched,
                    more,
                });
            }
        };
        let from = page.entries.front().map_or(window.from, |kept| kept.id);
        self.size += page.size;
        while let Some(kept) = page.entries.pop_back() {
            self.entries.push_front(kept);
        }
        self.window = earlier.map(|earlier| Window { from, earlier });
        Ok(())
    }

    /// This log with the entries before `from` taken out into a window:
    /// what a page from `from` folds to (design decision 4's first law).
    #[cfg(test)]
    pub(crate) fn suffix(&self, from: EntryId) -> Self {
        let mut suffix = self.clone();
        let mut taken = 0u64;
        while let Some(front) = suffix.entries.front()
            && front.id < from
        {
            suffix.size -= front.size;
            suffix.entries.pop_front();
            taken += 1;
        }
        let earlier = taken + self.window.map_or(0, |window| window.earlier);
        if earlier > 0 {
            let from = suffix.entries.front().map_or(suffix.next, |kept| kept.id);
            suffix.window = Some(Window { from, earlier });
        }
        suffix
    }

    /// [`Transcript::replay`] as a value that serializes as the JSON array
    /// `replay()` encodes to, written event by event (OpenSpec change
    /// `agents-checkpoint-deltas`, decision 8): one entry's events are
    /// made at a time, so the whole log is never copied to be encoded.
    pub fn replay_ref(&self) -> ReplayRef<'_> {
        ReplayRef { transcript: self }
    }

    pub(crate) fn replay_from(&self, start: usize) -> Vec<Event> {
        let mut events = Vec::with_capacity(self.entries.len() - start + 6);
        let collected: Result<(), std::convert::Infallible> = self.replay_each(start, &mut |event| {
            events.push(event);
            Ok(())
        });
        match collected {
            Ok(()) => events,
        }
    }

    /// Hands each event of the replay from entry index `start` to `emit`,
    /// in order, stopping at its first error.
    fn replay_each<E>(
        &self,
        start: usize,
        emit: &mut dyn FnMut(Event) -> Result<(), E>,
    ) -> Result<(), E> {
        if let Some(trim) = self.trimmed {
            emit(Event::Trimmed {
                through: trim.through,
                dropped: trim.dropped,
            })?;
        }
        let earlier = start as u64 + self.window.map_or(0, |window| window.earlier);
        if earlier > 0 {
            let from = self.entries.get(start).map_or(self.next, |kept| kept.id);
            emit(Event::Window { from, earlier })?;
        }
        let mut scratch = Vec::new();
        for kept in self.entries.range(start..) {
            replay_entry(kept.id, &kept.entry, &mut scratch);
            for event in scratch.drain(..) {
                emit(event)?;
            }
        }
        match &self.slot {
            Slot::Empty => {}
            slot @ (Slot::Waiting { .. } | Slot::Held { .. }) => {
                emit(Event::Queue { slot: slot.clone() })?;
            }
        }
        if let Some(conversation) = &self.conversation {
            emit(Event::Conversation {
                conversation: conversation.clone(),
            })?;
        }
        emit(Event::Turn {
            state: self.turn.clone(),
        })?;
        emit(Event::Agent {
            state: self.agent.clone(),
        })?;
        if !self.commands.is_empty() {
            emit(Event::Commands {
                commands: self.commands.clone(),
            })?;
        }
        if !self.options.is_empty() {
            emit(Event::Options {
                options: self.options.clone(),
            })?;
        }
        Ok(())
    }
}

/// A log's replay to be serialized ([`Transcript::replay_ref`]): the JSON
/// array of [`Transcript::replay`]'s events, byte for byte, written into
/// whatever serializer the caller gives it.
#[derive(Clone, Copy)]
pub struct ReplayRef<'a> {
    transcript: &'a Transcript,
}

impl serde::Serialize for ReplayRef<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeSeq;
        let mut seq = serializer.serialize_seq(None)?;
        self.transcript
            .replay_each(0, &mut |event| seq.serialize_element(&event))?;
        seq.end()
    }
}

/// An entry's `Added`, its text's further `Appended`s and its outcome's
/// `Answered`. Each is one of the live events' forms, so each is within
/// the producer's `MAX_EVENT` (the server's `acp::limits`, invariant I1):
/// text in pieces of at most [`TEXT_CHUNK`], a tool call's header apart
/// from its items, grouped by [`item_pieces`].
fn replay_entry(id: EntryId, entry: &Entry, events: &mut Vec<Event>) {
    match entry {
        Entry::User { text, attachments } => replay_text(
            id,
            text,
            |text| Entry::User {
                text,
                attachments: attachments.clone(),
            },
            events,
        ),
        Entry::Agent { text } => replay_text(id, text, |text| Entry::Agent { text }, events),
        Entry::Thought { text } => replay_text(id, text, |text| Entry::Thought { text }, events),
        Entry::Permission {
            call,
            title,
            options,
            outcome,
        } => {
            events.push(Event::Added {
                id,
                entry: Entry::Permission {
                    call: call.clone(),
                    title: title.clone(),
                    options: options.clone(),
                    outcome: None,
                },
            });
            if let Some(outcome) = outcome {
                events.push(Event::Answered {
                    id,
                    outcome: outcome.clone(),
                });
            }
        }
        Entry::Tool {
            call,
            title,
            kind,
            status,
            content,
            more,
            locations,
            input,
            result,
        } => {
            // The entry without its content, then its items in the pieces
            // the live split sends (`agents-event-bounds` decision 2), each
            // event within `MAX_EVENT` (`acp::limits` I1). A fold of either
            // form, or of one item an event as before, is the same entry.
            events.push(Event::Added {
                id,
                entry: Entry::Tool {
                    call: call.clone(),
                    title: title.clone(),
                    kind: *kind,
                    status: *status,
                    content: Vec::new(),
                    more: *more,
                    locations: locations.clone(),
                    input: input.clone(),
                    result: result.clone(),
                },
            });
            let mut from = 0usize;
            for piece in item_pieces(content) {
                events.push(Event::ToolItems {
                    id,
                    from: u32::try_from(from).unwrap_or(u32::MAX),
                    items: piece.to_vec(),
                });
                from += piece.len();
            }
        }
        Entry::Notice { .. } => events.push(Event::Added {
            id,
            entry: entry.clone(),
        }),
    }
}

/// A text entry's `Added`, made by `entry` from its first piece, and an
/// `Appended` for each piece after it.
fn replay_text(
    id: EntryId,
    text: &str,
    entry: impl FnOnce(String) -> Entry,
    events: &mut Vec<Event>,
) {
    let mut pieces = chunks(text);
    let first = pieces.next().unwrap_or_default().to_string();
    events.push(Event::Added {
        id,
        entry: entry(first),
    });
    events.extend(pieces.map(|piece| Event::Appended {
        id,
        text: piece.to_string(),
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fold::entry_change;
    use crate::fuzz::{XorShift, random_transcript};
    use crate::harness::*;
    use crate::*;

    /// `agents-checkpoint-deltas` decision 8: the replay streamed from the
    /// log (`replay_ref`) encodes byte for byte as the collected one, over
    /// random logs, trimmed ones and windowed ones.
    #[test]
    fn replaying_by_reference_encodes_the_same_bytes_as_replay() {
        let mut rng = XorShift::new(0x7a5c_0de1);
        let same = |transcript: &Transcript| {
            assert_eq!(
                serde_json::to_vec(&transcript.replay_ref()).unwrap(),
                serde_json::to_vec(&transcript.replay()).unwrap()
            );
        };
        let (mut trimmed, mut windowed) = (0, 0);
        for _ in 0..500 {
            let transcript = random_transcript(&mut rng);
            same(&transcript);
            let mut cut = transcript.clone();
            if cut.trim_to(cut.encoded_len() / 2).is_some() {
                trimmed += 1;
                same(&cut);
            }
            if let Some(from) = transcript.entries().nth(1).map(|(id, _)| id) {
                windowed += 1;
                same(&transcript.suffix(from));
            }
        }
        assert!(trimmed > 50 && windowed > 50, "{trimmed} trimmed, {windowed} windowed");
        same(&Transcript::new());
    }

    #[test]
    fn a_replay_folds_back_to_the_same_log() {
        let mut rng = XorShift::new(0x7a5c_0001);
        let mut slots = [0usize; 3];
        let mut conversations = [0usize; 3];
        let mut lost = 0usize;
        for _ in 0..400 {
            let transcript = random_transcript(&mut rng);
            slots[match transcript.slot() {
                Slot::Empty => 0,
                Slot::Waiting { .. } => 1,
                Slot::Held { .. } => 2,
            }] += 1;
            conversations[match transcript.conversation() {
                None => 0,
                Some(Conversation { title: None, .. }) => 1,
                Some(Conversation { title: Some(_), .. }) => 2,
            }] += 1;
            lost += usize::from(matches!(transcript.agent(), AgentState::Lost { .. }));
            let replay = transcript.replay();
            for event in &replay {
                if let Event::Added { entry, .. } = event
                    && let Entry::User { text, .. } | Entry::Agent { text } | Entry::Thought { text } =
                        entry
                {
                    assert!(text.len() <= TEXT_CHUNK);
                }
                if let Event::Appended { text, .. } = event {
                    assert!(text.len() <= TEXT_CHUNK && !text.is_empty());
                }
            }
            assert_eq!(fold(&replay), Ok(transcript.clone()));
            // And through the wire's encoding.
            let decoded: Vec<Event> = replay
                .iter()
                .map(|event| Event::from_value(&event.to_value()).unwrap())
                .collect();
            assert_eq!(decoded, replay);
        }
        assert!(slots.iter().all(|&n| n > 0), "slots generated: {slots:?}");
        assert!(
            conversations.iter().all(|&n| n > 0),
            "logs without a conversation, with an untitled one and with a titled one: \
             {conversations:?}"
        );
        assert!(lost > 0, "no log ended with the agent lost");
    }

    #[test]
    fn a_titled_conversation_replaces_an_untitled_one_and_a_later_one_clears_it() {
        let untitled = conversation("s1", Continuation::Resume);
        let titled = Conversation {
            title: Some("Hello.txt file creation".into()),
            ..untitled.clone()
        };
        let mut transcript = Transcript::new();
        for (next, title) in [
            (&untitled, None),
            (&titled, Some("Hello.txt file creation")),
            (&untitled, None),
        ] {
            transcript
                .apply(&Event::Conversation {
                    conversation: next.clone(),
                })
                .unwrap();
            assert_eq!(
                transcript.conversation().and_then(|c| c.title.as_deref()),
                title
            );
        }
        // A replay carries the title.
        transcript
            .apply(&Event::Conversation {
                conversation: titled.clone(),
            })
            .unwrap();
        assert!(transcript.replay().contains(&Event::Conversation {
            conversation: titled
        }));
        assert_eq!(fold(&transcript.replay()), Ok(transcript));
    }

    #[test]
    fn a_conversation_event_sets_the_conversation_and_a_later_one_replaces_it() {
        let mut transcript = Transcript::new();
        assert_eq!(transcript.conversation(), None);
        let first = conversation("s1", Continuation::ResumeAndLoad);
        transcript
            .apply(&Event::Conversation {
                conversation: first.clone(),
            })
            .unwrap();
        assert_eq!(transcript.conversation(), Some(&first));
        // Any agent state may follow, and the conversation stays.
        for state in [
            ready(),
            AgentState::Exited {
                code: Some(1),
                stderr: String::new(),
            },
            AgentState::Lost {
                message: "its holder ended".into(),
            },
            AgentState::Starting,
        ] {
            transcript.apply(&Event::Agent { state }).unwrap();
            assert_eq!(transcript.conversation(), Some(&first));
        }
        let second = conversation("s1", Continuation::Load);
        transcript
            .apply(&Event::Conversation {
                conversation: second.clone(),
            })
            .unwrap();
        assert_eq!(transcript.conversation(), Some(&second));
        assert_eq!(transcript.next_id(), EntryId::FIRST, "no entry was added");
    }

    #[test]
    fn a_replay_carries_the_conversation_right_before_the_turn_and_the_agent() {
        let held = Slot::held("next", HoldReason::AgentStopped);
        let kept = conversation("s1", Continuation::Resume);
        let lost = AgentState::Lost {
            message: "its holder ended".into(),
        };
        let transcript = fold(&[
            Event::Conversation {
                conversation: kept.clone(),
            },
            added(1, agent("hi")),
            Event::Queue { slot: held.clone() },
            Event::Agent {
                state: lost.clone(),
            },
        ])
        .unwrap();
        assert_eq!(
            transcript.replay(),
            [
                added(1, agent("hi")),
                Event::Queue { slot: held },
                Event::Conversation { conversation: kept },
                Event::Turn {
                    state: TurnState::Idle
                },
                Event::Agent { state: lost },
            ]
        );
        assert_eq!(fold(&transcript.replay()), Ok(transcript));
    }

    #[test]
    fn long_text_replays_in_chunks_cut_at_character_boundaries() {
        // Three-byte characters, so a cut at exactly 64 KiB would split one.
        let text = "€".repeat(TEXT_CHUNK / 3 * 2 + 5);
        let transcript = fold(&[added(1, agent(&text))]).unwrap();
        let replay = transcript.replay();
        let appended = replay
            .iter()
            .filter(|event| matches!(event, Event::Appended { .. }))
            .count();
        assert_eq!(appended, 2);
        assert_eq!(fold(&replay), Ok(transcript));
        let pieces: Vec<&str> = chunks(&text).collect();
        assert_eq!(pieces[0].len(), TEXT_CHUNK - 1, "cut before the character");
        assert_eq!(pieces.concat(), text);
        assert!(pieces.iter().all(|piece| piece.len() <= TEXT_CHUNK));
        assert_eq!(chunks("").collect::<Vec<_>>(), [""]);
    }

    #[test]
    fn a_replay_holds_the_trim_the_entries_the_slot_the_turn_and_the_agent() {
        let held = Slot::held("again", HoldReason::Failed);
        let transcript = fold(&[
            added(1, agent("old")),
            added(2, permission()),
            Event::Answered {
                id: EntryId(2),
                outcome: PermissionOutcome::Selected {
                    option: "reject-once".into(),
                },
            },
            Event::Trimmed {
                through: EntryId(1),
                dropped: 1,
            },
            Event::Queue { slot: held.clone() },
            Event::Turn {
                state: TurnState::Failed {
                    message: "rate limited".into(),
                },
            },
        ])
        .unwrap();
        let Entry::Permission {
            call,
            title,
            options,
            ..
        } = permission()
        else {
            unreachable!()
        };
        assert_eq!(
            transcript.replay(),
            [
                Event::Trimmed {
                    through: EntryId(1),
                    dropped: 1
                },
                added(
                    2,
                    Entry::Permission {
                        call,
                        title,
                        options,
                        outcome: None
                    }
                ),
                Event::Answered {
                    id: EntryId(2),
                    outcome: PermissionOutcome::Selected {
                        option: "reject-once".into()
                    }
                },
                Event::Queue { slot: held },
                Event::Turn {
                    state: TurnState::Failed {
                        message: "rate limited".into()
                    }
                },
                Event::Agent {
                    state: AgentState::Starting
                },
            ]
        );
        // An empty slot isn't replayed.
        assert!(
            !Transcript::new()
                .replay()
                .iter()
                .any(|event| matches!(event, Event::Queue { .. }))
        );
    }

    #[test]
    fn a_window_goes_only_on_an_empty_transcript_at_or_past_the_next_id() {
        let mut copy = fold(&[window(5, 4)]).unwrap();
        assert_eq!(copy.next_id(), EntryId(5));
        assert_eq!(
            copy.window(),
            Some(Window {
                from: EntryId(5),
                earlier: 4
            })
        );
        copy.apply(&added(5, agent("hi"))).unwrap();
        let before = copy.clone();
        assert_eq!(
            copy.apply(&window(9, 1)),
            Err(FoldError::WindowAfterEntries)
        );
        assert_eq!(copy, before, "a refused event changes nothing");

        let mut trimmed = fold(&[Event::Trimmed {
            through: EntryId(6),
            dropped: 6,
        }])
        .unwrap();
        let before = trimmed.clone();
        assert_eq!(
            trimmed.apply(&window(5, 1)),
            Err(FoldError::WindowBehind {
                from: EntryId(5),
                next: EntryId(7)
            })
        );
        assert_eq!(trimmed, before, "a refused event changes nothing");
        trimmed.apply(&window(7, 3)).unwrap();
        // A window with nothing earlier is refused.
        let mut empty = Transcript::new();
        assert_eq!(
            empty.apply(&window(3, 0)),
            Err(FoldError::WindowEmpty { from: EntryId(3) })
        );
        assert_eq!(empty, Transcript::new());
    }

    #[test]
    fn changes_below_the_window_are_ignored_and_the_rest_fold_as_before() {
        let mut copy = paged();
        let before = copy.clone();
        for event in [
            appended(2, "more"),
            changed(1, None, None, Some(ToolStatus::Completed)),
            Event::Answered {
                id: EntryId(4),
                outcome: PermissionOutcome::Cancelled,
            },
        ] {
            assert_eq!(copy.apply(&event), Ok(()), "{event:?}");
            assert_eq!(copy, before, "{event:?} changed nothing");
        }
        // An entry from the window on is held.
        copy.apply(&appended(5, "!")).unwrap();
        assert_eq!(copy.entry(EntryId(5)), Some(&agent(&format!("{}!", "x".repeat(10)))));
        // An `Added` must still be the next id.
        assert_eq!(
            copy.apply(&added(10, agent("late"))),
            Err(FoldError::OutOfOrder {
                id: EntryId(10),
                expected: EntryId(9)
            })
        );
        // State events aren't entries: unchanged.
        copy.apply(&Event::Queue {
            slot: Slot::waiting("next"),
        })
        .unwrap();
        copy.apply(&Event::Turn { state: TurnState::Working }).unwrap();
        copy.apply(&Event::Conversation {
            conversation: conversation("s1", Continuation::Resume),
        })
        .unwrap();
        assert_eq!(copy.turn(), &TurnState::Working);
        assert!(copy.window().is_some());
    }

    #[test]
    fn a_trim_below_the_window_lowers_its_count_and_one_at_or_past_it_drops_it() {
        let mut copy = paged();
        copy.apply(&Event::Trimmed {
            through: EntryId(2),
            dropped: 2,
        })
        .unwrap();
        assert_eq!(
            copy.window(),
            Some(Window {
                from: EntryId(5),
                earlier: 2
            })
        );
        assert_eq!(copy.entries().count(), 4, "no held entry was dropped");
        copy.apply(&Event::Trimmed {
            through: EntryId(4),
            dropped: 2,
        })
        .unwrap();
        assert_eq!(copy.window(), None, "nothing earlier is left");
        assert_eq!(copy.dropped(), 4);

        let mut copy = paged();
        copy.apply(&Event::Trimmed {
            through: EntryId(6),
            dropped: 6,
        })
        .unwrap();
        assert_eq!(copy.window(), None);
        assert_eq!(
            copy.entries().map(|(id, _)| id).collect::<Vec<_>>(),
            [EntryId(7), EntryId(8)]
        );
    }

    #[test]
    fn a_log_that_fits_pages_as_its_replay() {
        let server = long(8, 10);
        assert_eq!(server.page(PAGE_BYTES), server.replay());
        assert_eq!(server.page(server.encoded_len()), server.replay());
    }

    #[test]
    fn a_long_log_pages_its_newest_whole_entries_within_the_budget_then_its_state() {
        let mut server = long(10, 100);
        let held = Slot::held("again", HoldReason::Failed);
        let titled = Conversation {
            title: Some("Hello.txt file creation".into()),
            ..conversation("s1", Continuation::Resume)
        };
        for event in [
            Event::Trimmed {
                through: EntryId(2),
                dropped: 2,
            },
            Event::Queue { slot: held.clone() },
            Event::Conversation {
                conversation: titled.clone(),
            },
            Event::Turn {
                state: TurnState::Working,
            },
        ] {
            server.apply(&event).unwrap();
        }
        let size = server.entries.back().unwrap().size;
        let page = server.page(size * 3);
        assert_eq!(
            page,
            [
                Event::Trimmed {
                    through: EntryId(2),
                    dropped: 2
                },
                window(8, 5),
                added(8, agent(&"x".repeat(100))),
                added(9, agent(&"x".repeat(100))),
                added(10, agent(&"x".repeat(100))),
                Event::Queue { slot: held },
                Event::Conversation {
                    conversation: titled
                },
                Event::Turn {
                    state: TurnState::Working
                },
                Event::Agent { state: ready() },
            ]
        );
        assert_eq!(fold(&page), Ok(server.suffix(EntryId(8))));
        // A budget one byte short takes one entry fewer.
        assert_eq!(fold(&server.page(size * 3 - 1)).unwrap().entries().count(), 2);
    }

    #[test]
    fn a_budget_of_nothing_gives_the_newest_entry_and_a_huge_one_goes_alone() {
        let server = long(4, 10);
        let page = fold(&server.page(0)).unwrap();
        assert_eq!(
            page.entries().map(|(id, _)| id).collect::<Vec<_>>(),
            [EntryId(4)]
        );
        let mut server = long(4, 10);
        server.apply(&added(5, agent(&"€".repeat(350_000)))).unwrap();
        let page = server.page(PAGE_BYTES);
        let copy = fold(&page).unwrap();
        assert_eq!(
            copy.entries().map(|(id, _)| id).collect::<Vec<_>>(),
            [EntryId(5)]
        );
        assert_eq!(copy.window().map(|window| window.earlier), Some(4));
        assert!(page.iter().filter(|event| matches!(event, Event::Appended { .. })).count() > 10);
    }

    #[test]
    fn a_pending_permission_far_back_pulls_the_page_down_to_it() {
        let mut server = long(2, 10);
        server.apply(&added(3, permission())).unwrap();
        for id in 4..40 {
            server.apply(&added(id, agent(&"y".repeat(64 * 1024)))).unwrap();
        }
        let copy = fold(&server.page(PAGE_BYTES)).unwrap();
        assert_eq!(
            copy.window(),
            Some(Window {
                from: EntryId(3),
                earlier: 2
            })
        );
        assert_eq!(copy, server.suffix(EntryId(3)));
    }

    #[test]
    fn a_page_carries_a_login_demand_with_its_methods() {
        let mut server = long(20, 100);
        let demand = AgentState::LoginRequired {
            message: "log in first".into(),
            methods: vec![agent_method("oauth", "Log in"), terminal_method()],
        };
        server
            .apply(&Event::Agent {
                state: demand.clone(),
            })
            .unwrap();
        let page = server.page(300);
        assert_eq!(page.last(), Some(&Event::Agent { state: demand }));
        assert!(matches!(page[0], Event::Window { .. }));
    }

    #[test]
    fn a_fetch_takes_whole_entries_below_before_as_they_stand_now() {
        let mut server = long(3, 10);
        server.apply(&added(4, tool("call_1"))).unwrap();
        server.apply(&added(5, permission())).unwrap();
        for id in 6..=8 {
            server.apply(&added(id, agent("z"))).unwrap();
        }
        server
            .apply(&changed(4, None, None, Some(ToolStatus::Completed)))
            .unwrap();
        server
            .apply(&Event::Answered {
                id: EntryId(5),
                outcome: PermissionOutcome::Cancelled,
            })
            .unwrap();
        let fetched = server.fetch(EntryId(6), PAGE_BYTES);
        assert!(!fetched.more);
        let mut expected = Vec::new();
        for (id, entry) in server.entries().take(5) {
            replay_entry(id, entry, &mut expected);
        }
        assert_eq!(fetched.events, expected);
        assert!(
            fetched.events.iter().all(|event| entry_change(event).is_some()
                || matches!(event, Event::Added { .. })),
            "entries only"
        );
        // A budget that takes two: more remain.
        let size = server.entries.get(4).unwrap().size + server.entries.get(3).unwrap().size;
        let fetched = server.fetch(EntryId(6), size);
        assert!(fetched.more);
        assert_eq!(fold_page(&fetched.events).first(), Some(&EntryId(4)));
        // At or below the first kept entry: nothing.
        assert_eq!(
            server.fetch(EntryId(1), PAGE_BYTES),
            Fetched {
                events: Vec::new(),
                more: false
            }
        );
        assert_eq!(server.fetch(EntryId(2), 0).events, [added(1, agent(&"x".repeat(10)))]);
    }

    /// The ids of a fetched page's `Added`s.
    fn fold_page(events: &[Event]) -> Vec<EntryId> {
        events
            .iter()
            .filter_map(|event| match event {
                Event::Added { id, .. } => Some(*id),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_page_folded_in_front_lowers_the_count_and_the_last_drops_the_window() {
        let server = long(8, 10);
        let mut copy = paged();
        let first = server.fetch(EntryId(5), server.entries[0].size * 2);
        assert!(first.more);
        copy.prepend(&first.events, first.more).unwrap();
        assert_eq!(
            copy.window(),
            Some(Window {
                from: EntryId(3),
                earlier: 2
            })
        );
        assert_eq!(copy, server.suffix(EntryId(3)));
        let rest = server.fetch(EntryId(3), PAGE_BYTES);
        assert!(!rest.more);
        copy.prepend(&rest.events, rest.more).unwrap();
        assert_eq!(copy, server);
    }

    #[test]
    fn an_empty_page_without_more_on_a_copy_with_no_window_changes_nothing() {
        let mut copy = long(3, 10);
        let before = copy.clone();
        assert_eq!(copy.prepend(&[], false), Ok(()));
        assert_eq!(copy, before);
    }

    #[test]
    fn a_page_that_doesnt_fit_the_window_is_refused_and_changes_nothing() {
        let server = long(8, 10);
        let gap = server.fetch(EntryId(4), PAGE_BYTES);
        let whole = server.fetch(EntryId(5), PAGE_BYTES);
        let part = server.fetch(EntryId(5), 0);
        let cases: [(Transcript, &[Event], bool, FoldError); 5] = [
            (
                paged(),
                &gap.events,
                false,
                FoldError::PageGap {
                    last: EntryId(3),
                    from: EntryId(5),
                },
            ),
            (long(3, 10), &whole.events, false, FoldError::NoWindow),
            (
                paged(),
                &part.events,
                false,
                FoldError::WindowMismatch {
                    earlier: 4,
                    fetched: 1,
                    more: false,
                },
            ),
            (
                paged(),
                &[],
                true,
                FoldError::WindowMismatch {
                    earlier: 4,
                    fetched: 0,
                    more: true,
                },
            ),
            (
                paged(),
                &whole.events,
                true,
                FoldError::WindowMismatch {
                    earlier: 4,
                    fetched: 4,
                    more: true,
                },
            ),
        ];
        for (mut copy, events, more, error) in cases {
            let before = copy.clone();
            assert_eq!(copy.prepend(events, more), Err(error.clone()));
            assert_eq!(copy, before, "{error}");
            assert!(!error.to_string().is_empty());
        }
        // An empty page without more while earlier entries remain.
        let mut copy = paged();
        assert!(matches!(
            copy.prepend(&[], false),
            Err(FoldError::WindowMismatch { fetched: 0, .. })
        ));
    }

    #[test]
    fn a_paged_copy_replays_with_its_window() {
        let copy = paged();
        assert_eq!(fold(&copy.replay()), Ok(copy));
    }

    #[test]
    fn the_new_fold_errors_read_as_what_went_wrong() {
        for (error, text) in [
            (
                FoldError::WindowAfterEntries,
                "a page's window came after entries were already held",
            ),
            (
                FoldError::WindowBehind {
                    from: EntryId(3),
                    next: EntryId(5),
                },
                "a page's window starts at entry 3, but the next entry is 5",
            ),
            (
                FoldError::PageGap {
                    last: EntryId(3),
                    from: EntryId(5),
                },
                "a fetched page ends at entry 3, but the held entries start at 5",
            ),
            (
                FoldError::WindowMismatch {
                    earlier: 4,
                    fetched: 1,
                    more: false,
                },
                "a fetched page of 1 entries with nothing more to fetch doesn't fit the 4 earlier entries",
            ),
            (
                FoldError::WindowEmpty { from: EntryId(3) },
                "a page's window at entry 3 has no earlier entries",
            ),
            (
                FoldError::NoWindow,
                "a fetched page came for a transcript that holds every entry",
            ),
        ] {
            assert_eq!(error.to_string(), text);
        }
    }

    #[test]
    fn a_tool_entry_replays_without_content_then_its_items_grouped() {
        let entry = tool_with(
            "c1",
            vec![text_item("a"), diff_item(), output_item(Some("ls"))],
            Some("in"),
        );
        let transcript = fold(&[added(1, entry)]).unwrap();
        let replay = transcript.replay();
        // The two small items in one piece; an output, which counts at
        // its full `ITEM_LIMIT`, alone (`agents-event-bounds` decision 2).
        assert_eq!(
            replay[..3],
            [
                added(1, tool_with("c1", Vec::new(), Some("in"))),
                items(1, 0, vec![text_item("a"), diff_item()]),
                items(1, 2, vec![output_item(Some("ls"))]),
            ]
        );
        assert_eq!(fold(&replay), Ok(transcript));
    }

    #[test]
    fn a_paged_copy_ignores_tool_content_below_its_window() {
        let mut server = fold(&[added(1, tool("c1")), items(1, 0, vec![output_item(None)])])
            .unwrap();
        for id in 2..=8 {
            server.apply(&added(id, agent(&"x".repeat(10)))).unwrap();
        }
        let mut copy = fold(&server.replay_from(4)).unwrap();
        let before = copy.clone();
        for event in [items(1, 0, vec![text_item("t")]), output(1, 0, "x", None)] {
            assert_eq!(copy.apply(&event), Ok(()), "{event:?}");
            assert_eq!(copy, before, "{event:?} changed nothing");
        }
    }

    /// A log of a tool call `call`, entries of `filler` bytes, then a
    /// permission for `call`, answered or not.
    fn permission_after(call: &str, filler: u64, outcome: Option<PermissionOutcome>) -> Transcript {
        let mut events = vec![
            added(1, agent("before")),
            added(2, tool_with(call, vec![diff_item()], None)),
        ];
        let chunk = 64 * 1024;
        let count = filler / chunk as u64;
        for id in 3..3 + count {
            events.push(added(id, agent(&"y".repeat(chunk))));
        }
        // `permission()` names call_1.
        let asked = 3 + count;
        events.push(added(asked, permission()));
        if let Some(outcome) = outcome {
            events.push(Event::Answered {
                id: EntryId(asked),
                outcome,
            });
        }
        events.push(added(asked + 1, agent("after")));
        fold(&events).unwrap()
    }

    #[test]
    fn a_waiting_permissions_tool_call_far_back_starts_the_page() {
        let server = permission_after("call_1", 2 * 1024 * 1024, None);
        let copy = fold(&server.page(PAGE_BYTES)).unwrap();
        assert_eq!(copy.window().map(|window| window.from), Some(EntryId(2)));
        assert_eq!(copy, server.suffix(EntryId(2)));
        assert_eq!(content_of(&copy, 2), [diff_item()]);
    }

    #[test]
    fn an_answered_permissions_tool_call_doesnt_widen_the_page() {
        let server = permission_after(
            "call_1",
            2 * 1024 * 1024,
            Some(PermissionOutcome::Cancelled),
        );
        let plain = fold(&server.page(PAGE_BYTES)).unwrap();
        assert!(plain.window().unwrap().from > EntryId(30));
    }

    #[test]
    fn a_permission_naming_a_call_the_log_doesnt_hold_pages_as_before() {
        let server = permission_after("call_9", 2 * 1024 * 1024, None);
        let copy = fold(&server.page(PAGE_BYTES)).unwrap();
        // The budget's newest entries, the permission among them, as before:
        // not the tool call of another id.
        let permission = server
            .entries()
            .find(|(_, entry)| matches!(entry, Entry::Permission { .. }))
            .map(|(id, _)| id)
            .unwrap();
        let from = copy.window().unwrap().from;
        assert!(from > EntryId(30) && from <= permission, "{from}");
    }

    #[test]
    fn a_replay_and_a_page_carry_both_lists_after_the_agent_and_a_fetch_neither() {
        let mut server = long(10, 100);
        assert!(
            !server
                .replay()
                .iter()
                .any(|event| matches!(event, Event::Commands { .. } | Event::Options { .. })),
            "empty lists are left out"
        );
        let commands = Event::Commands {
            commands: vec![compact()],
        };
        let options = Event::Options {
            options: vec![mode_option("ask"), fast_option()],
        };
        server.apply(&commands).unwrap();
        server.apply(&options).unwrap();
        let replay = server.replay();
        let tail = &replay[replay.len() - 3..];
        assert!(matches!(tail[0], Event::Agent { .. }));
        assert_eq!(tail[1..], [commands.clone(), options.clone()]);
        assert_eq!(fold(&replay), Ok(server.clone()));
        // A page holds them whatever its budget, after the agent.
        let page = server.page(150);
        assert_eq!(page[page.len() - 2..], [commands, options]);
        let copy = fold(&page).unwrap();
        assert!(copy.window().is_some());
        assert_eq!(copy, server.suffix(copy.window().unwrap().from));
        // A fetch brings entries only.
        let fetched = server.fetch(copy.window().unwrap().from, 150);
        assert!(
            fetched
                .events
                .iter()
                .all(|event| matches!(event, Event::Added { .. } | Event::Appended { .. }))
        );
    }

    /// A replay of a tool entry of 32 small items sends one `ToolItems`; a
    /// larger item goes alone.
    #[test]
    fn a_replay_groups_a_tool_entrys_items_by_the_live_split() {
        let small: Vec<ToolItem> = (0..MAX_ITEMS).map(|n| text_item(&format!("{n}"))).collect();
        let log = fold(&[added(1, tool_with("call_1", small.clone(), None))]).unwrap();
        let replay = log.replay();
        let pieces: Vec<&Event> = replay
            .iter()
            .filter(|event| matches!(event, Event::ToolItems { .. }))
            .collect();
        assert_eq!(pieces, [&items(1, 0, small)]);
        let mixed = vec![
            text_item("a"),
            ToolItem::Output(Output::default()),
            text_item(&"b".repeat(TEXT_CHUNK - 10)),
            text_item(&"c".repeat(20)),
        ];
        let groups: Vec<usize> = item_pieces(&mixed).iter().map(|piece| piece.len()).collect();
        assert_eq!(groups, [1, 1, 1, 1]);
        let fits = vec![text_item(&"a".repeat(100)), text_item(&"b".repeat(TEXT_CHUNK - 100))];
        assert_eq!(item_pieces(&fits).len(), 1);
    }
}
