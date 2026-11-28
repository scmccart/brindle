//! An absorb's two halves and the draw that follows (OpenSpec change
//! `agents-absorb-cost`, decisions 2–6): [`ChatModel::copy_changes`] runs
//! under `AgentShared`'s lock and copies only what changed;
//! [`ChatModel::absorb`] applies the copy with the lock released, deciding
//! and marking but rendering nothing; [`ChatModel::draw`] renders each
//! marked entry once and splices the list, at the end of the update.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use brindle_ui::{Inline, RichBlock};

use crate::agent::markdown;
use crate::agent::shared::{Changes, Dirty};
use crate::agent::tool::{self, DetailPart, RowState};
use crate::agent::transcript::{
    Activity, AgentState, Conversation, Entry, EntryId, Exit, SessionOption, Slot, SlashCommand, ToolItem, Transcript,
    TurnState, Window,
};

use super::blocks::{Block, BlockBody, ShownPart, Splice, render_entry, working_splice};
use super::facts::{Facts, Rows};
use super::ledger::Sent;
use super::{Absorbed, AgentPhase, ChatModel};

/// What the first half copied under the lock: the changes as taken, the
/// entries that changed, and the session's values when they may have
/// moved (decision 2).
#[derive(Debug, Clone, PartialEq)]
pub struct ChangeCopy {
    changes: Changes,
    /// Copies of the entries that changed, in id order.
    entries: Vec<(EntryId, Copied)>,
    /// The turn, the agent, the conversation and the activity, when
    /// `reset`, `turn_or_agent` or `added_from` says they may have moved.
    session: Option<SessionCopy>,
    /// The slot, when `reset` or `slot_changed`.
    slot: Option<Slot>,
    /// The transcript's window, as every absorb follows it.
    window: Option<Window>,
    /// The slash commands and the session's options, when `reset` or
    /// `lists_changed`.
    lists: Option<(Vec<SlashCommand>, Vec<SessionOption>)>,
}

/// One changed entry's copy (decision 2): exactly one of these.
#[derive(Debug, Clone, PartialEq)]
pub enum Copied {
    /// An entry the zone doesn't hold: added, on a fetched page, or any
    /// entry on a reset.
    New(Entry),
    /// A message's text from `from_byte` on, the zone's text being that
    /// long.
    Tail { from_byte: usize, text: String },
    /// Output item `item`'s new bytes and how its command ended (decision
    /// 6).
    Output { item: usize, text: String, exit: Option<Exit> },
    /// A tool call's items from `from` on, and how many its bound left out
    /// (the fold raises `more` with the items it drops).
    Items { from: usize, items: Vec<ToolItem>, more: u32 },
    /// Anything else: the entry again.
    Whole(Entry),
}

/// The session's values an absorb copies.
#[derive(Debug, Clone, PartialEq)]
pub struct SessionCopy {
    turn: TurnState,
    activity: Activity,
    agent: AgentState,
    conversation: Option<Conversation>,
}

/// How a drawn entry changed since it was drawn (decision 4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Mark {
    /// Held, with no blocks yet (added or prepended).
    New,
    /// Drawn, and its text, output or items grew since: the frame gate
    /// may leave it for the next draw.
    Grown,
    /// Drawn, and replaced or otherwise changed: drawn with the next draw.
    Changed,
}

impl Mark {
    fn and(self, later: Self) -> Self {
        match (self, later) {
            (Self::New, _) | (_, Self::New) => Self::New,
            (Self::Changed, _) | (_, Self::Changed) => Self::Changed,
            (Self::Grown, Self::Grown) => Self::Grown,
        }
    }
}

/// Which marks a draw takes (decision 5): every one, or all but the
/// grown ones (the frame gate's draw within a frame of the last).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Due {
    All,
    Settled,
}

/// An entry whose only change since it was drawn is output appended to
/// one item: its drawn blocks show that item with a dropped-start mark
/// before its lines when `had_omitted`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct GrownOutput {
    item: usize,
    had_omitted: bool,
}

/// What the zone hasn't drawn yet (`absorb` marks, `draw` clears).
#[derive(Debug, Clone, Default)]
pub(super) struct Undrawn {
    /// A reset: every block is drawn again, as one splice of the whole
    /// list.
    all: bool,
    entries: BTreeMap<EntryId, Mark>,
    /// Tool text items whose source changed: rendered by the draw.
    texts: BTreeSet<(EntryId, usize)>,
    /// Entries whose only change is one output's delta (decision 6).
    outputs: BTreeMap<EntryId, GrownOutput>,
    /// The rows the draw checks (decision 7).
    rows: Rows,
    /// Every thought's openness is checked, not only the auto-open ones.
    thoughts: bool,
}

impl Undrawn {
    fn mark(&mut self, id: EntryId, mark: Mark) {
        let mark = self.entries.get(&id).map_or(mark, |held| held.and(mark));
        self.entries.insert(id, mark);
    }

    fn trim(&mut self, through: EntryId) {
        self.entries.retain(|id, _| *id > through);
        self.texts.retain(|(id, _)| *id > through);
        self.outputs.retain(|id, _| *id > through);
        if let Rows::Only(ids) = &mut self.rows {
            ids.retain(|id| *id > through);
        }
    }
}

/// The text items of a tool entry's content, by index.
fn text_indices(content: &[ToolItem]) -> impl Iterator<Item = usize> + '_ {
    content.iter().enumerate().filter_map(|(item, content)| match content {
        ToolItem::Text { .. } => Some(item),
        ToolItem::Diff(_) | ToolItem::Output(_) => None,
    })
}

fn text_of(item: &ToolItem) -> Option<&str> {
    match item {
        ToolItem::Text { text, .. } => Some(text),
        ToolItem::Diff(_) | ToolItem::Output(_) => None,
    }
}

/// Tool entry `id`'s items from `from` on going from `old` to `new` (both
/// counted from `from`): the text items whose source differs, or that have
/// no render, are due to be rendered (decision 8: the old item is in hand
/// here, so no source is kept beside a render), and the renders of items
/// that are no longer text are forgotten.
fn texts_due(
    texts: &mut BTreeMap<(EntryId, usize), super::blocks::Rich>,
    id: EntryId,
    old: &[ToolItem],
    new: &[ToolItem],
    from: usize,
) -> Vec<(EntryId, usize)> {
    let held: Vec<usize> = texts.range((id, from)..=(id, usize::MAX)).map(|((_, item), _)| *item).collect();
    for item in held {
        if new.get(item - from).and_then(text_of).is_none() {
            texts.remove(&(id, item));
        }
    }
    (0..new.len())
        .filter(|at| {
            let now = new.get(*at).and_then(text_of);
            now.is_some() && (now != old.get(*at).and_then(text_of) || !texts.contains_key(&(id, from + at)))
        })
        .map(|at| (id, from + at))
        .collect()
}

/// The new bytes of output item `item` (decision 6): with the zone's copy
/// at `(omitted₀, len₀)` and the transcript's at `(omitted₁, len₁)`, the
/// last `d = (omitted₁ + len₁) − (omitted₀ + len₀)` bytes of the
/// transcript's text, when none of them was dropped; else `None`.
pub(super) fn output_delta(own: &crate::agent::transcript::Output, now: &crate::agent::transcript::Output) -> Option<String> {
    let (text, own_text) = (now.text.as_str(), own.text.as_str());
    let total = now.omitted.checked_add(text.len() as u64)?;
    let held = own.omitted.checked_add(own_text.len() as u64)?;
    let delta = usize::try_from(total.checked_sub(held)?).ok()?;
    let at = text.len().checked_sub(delta)?;
    text.get(at..).map(str::to_string)
}

impl ChatModel {
    /// The first half (decision 2), run under `AgentShared`'s lock: copies
    /// of what changed, sized by the zone's own copy. Reads the model and
    /// writes nothing.
    pub fn copy_changes(&self, transcript: &Transcript, changes: Changes) -> ChangeCopy {
        let mut entries = Vec::new();
        if changes.reset {
            entries.extend(transcript.entries().map(|(id, entry)| (id, Copied::New(entry.clone()))));
        } else {
            if changes.prepended.is_some_and(|added| added > 0) {
                let held = self.entries.first().map(|(id, _)| *id).or(changes.added_from);
                entries.extend(
                    transcript
                        .entries()
                        .take_while(|(id, _)| held.is_none_or(|held| *id < held))
                        .map(|(id, entry)| (id, Copied::New(entry.clone()))),
                );
            }
            for (id, dirty) in &changes.changed {
                if let Some(copied) = self.copy_entry(transcript, *id, *dirty) {
                    entries.push((*id, copied));
                }
            }
            if let Some(from) = changes.added_from {
                let last = self.entries.last().map(|(id, _)| *id);
                entries.extend(
                    transcript
                        .entries_from(from)
                        .filter(|(id, _)| last.is_none_or(|last| *id > last))
                        .map(|(id, entry)| (id, Copied::New(entry.clone()))),
                );
            }
        }
        let session = (changes.reset || changes.turn_or_agent || changes.added_from.is_some()).then(|| SessionCopy {
            turn: transcript.turn().clone(),
            activity: transcript.activity(),
            agent: transcript.agent().clone(),
            conversation: transcript.conversation().cloned(),
        });
        let slot = (changes.reset || changes.slot_changed).then(|| transcript.slot().clone());
        let lists = (changes.reset || changes.lists_changed)
            .then(|| (transcript.commands().to_vec(), transcript.options().to_vec()));
        ChangeCopy { changes, entries, session, slot, window: transcript.window(), lists }
    }

    /// Held entry `id`'s copy after `dirty`: the least the zone needs, or
    /// the entry whole when a part can't be taken.
    fn copy_entry(&self, transcript: &Transcript, id: EntryId, dirty: Dirty) -> Option<Copied> {
        let source = transcript.entry(id)?;
        let own = self.held(id)?;
        let whole = || Copied::Whole(source.clone());
        Some(match (dirty, own, source) {
            (
                Dirty::Appended { from_byte },
                Entry::User { text: own } | Entry::Agent { text: own } | Entry::Thought { text: own },
                Entry::User { text } | Entry::Agent { text } | Entry::Thought { text },
            ) if own.len() == from_byte && text.is_char_boundary(from_byte) => {
                Copied::Tail { from_byte, text: text[from_byte..].to_string() }
            }
            (
                Dirty::Output { item },
                Entry::Tool { content: own, .. },
                Entry::Tool { content, more, .. },
            ) => match (own.get(item), content.get(item)) {
                (Some(ToolItem::Output(own)), Some(ToolItem::Output(now))) => match output_delta(own, now) {
                    Some(text) => Copied::Output { item, text, exit: now.exit.clone() },
                    // The new bytes pushed some of themselves out: the item
                    // again, with the ones after it.
                    None => Copied::Items { from: item, items: content[item..].to_vec(), more: *more },
                },
                (Some(_), Some(_)) if own.len() == content.len() => {
                    Copied::Items { from: item, items: content[item..].to_vec(), more: *more }
                }
                (Some(_) | None, Some(_) | None) => whole(),
            },
            (Dirty::ToolItems { from }, Entry::Tool { content: own, .. }, Entry::Tool { content, more, .. })
                if from <= own.len() && from <= content.len() =>
            {
                Copied::Items { from, items: content[from..].to_vec(), more: *more }
            }
            (Dirty::Appended { .. } | Dirty::ToolItems { .. } | Dirty::Output { .. } | Dirty::Replaced, _, _) => whole(),
        })
    }

    /// The second half (decision 3), with the lock released: applies the
    /// copy, keeps the facts, marks what changed and decides what the
    /// change means. Renders nothing; a trim's splice comes back at once,
    /// the rest with the next [`draw`](Self::draw).
    pub fn absorb(&mut self, taken: ChangeCopy) -> Absorbed {
        let before = self.before();
        let ChangeCopy { changes, entries, session, slot, window, lists } = taken;
        let mut splices = Vec::new();
        let last_user = self.facts.last_user;
        if changes.reset {
            // A fetch in flight belongs to the copy that is gone.
            self.pending.retain(|_, sent| *sent != Sent::Fetch);
            self.entries.clear();
            for (id, copied) in entries {
                match copied {
                    Copied::New(entry) => self.entries.push((id, entry)),
                    // Never built for a reset.
                    Copied::Tail { .. } | Copied::Output { .. } | Copied::Items { .. } | Copied::Whole(_) => {}
                }
            }
            self.auto_open = None;
            self.rendered.clear();
            self.texts.clear();
            self.facts = Facts::read(&self.entries);
            let texts = self
                .entries
                .iter()
                .flat_map(|(id, entry)| match entry {
                    Entry::Tool { content, .. } => text_indices(content).map(|item| (*id, item)).collect(),
                    Entry::User { .. }
                    | Entry::Agent { .. }
                    | Entry::Thought { .. }
                    | Entry::Permission { .. }
                    | Entry::Notice { .. } => Vec::new(),
                })
                .collect();
            self.undrawn = Undrawn { all: true, texts, rows: Rows::All, thoughts: true, ..Undrawn::default() };
        } else {
            if let Some(through) = changes.trimmed_through {
                splices.extend(self.trim(through));
            }
            for (id, copied) in entries {
                if changes.trimmed_through.is_some_and(|through| id <= through) {
                    continue;
                }
                self.apply_copy(id, copied);
            }
            if changes.prepended.is_some_and(|added| added > 0) {
                self.facts = Facts::read(&self.entries);
                self.undrawn.rows = Rows::All;
            }
        }
        if let Some(SessionCopy { turn, activity, agent, conversation }) = session {
            if turn != self.turn {
                self.undrawn.rows.extend(self.facts.unfinished.iter().copied());
            }
            self.turn = turn;
            if activity != self.activity {
                self.seen = false;
            }
            self.activity = activity;
            match &self.phase {
                AgentPhase::Connecting | AgentPhase::Live(_) => self.phase = AgentPhase::Live(agent),
                AgentPhase::NotStarted(_) | AgentPhase::Lost { .. } | AgentPhase::Broken { .. } => {}
            }
            if let AgentPhase::Live(AgentState::Ready { name: Some(name), .. }) = &self.phase {
                self.agent_name = Some(name.clone());
            }
            if let Some(conversation) = conversation {
                self.conversation = Some(conversation);
            }
        }
        if self.facts.last_user != last_user {
            self.undrawn.rows.extend(self.facts.unfinished.iter().copied());
        }
        if let Some(slot) = slot {
            self.slot = slot;
        }
        if let Some((commands, options)) = lists {
            self.commands = commands;
            self.options = options;
            self.refresh_completion();
        }
        self.follow_window(window, changes.reset, changes.prepended.is_some());
        // The reconnect's replay says whether a prompt whose answer was
        // lost with the link was taken (decision 5).
        let replied =
            if changes.reset { self.uncertain.take().map(|uncertain| self.settle_uncertain(uncertain, None)) } else { None };
        Absorbed { replied, ..self.finish(before, splices, changes.status_from) }
    }

    /// Drops the entries through `through` and everything kept for them;
    /// gives the blocks' splice.
    fn trim(&mut self, through: EntryId) -> Option<Splice> {
        self.entries.retain(|(id, _)| *id > through);
        self.rendered.retain(|id, _| *id > through);
        self.texts.retain(|(id, _), _| *id > through);
        self.by_hand.retain(|id, _| *id > through);
        self.toggled.retain(|id, _| *id > through);
        self.undrawn.trim(through);
        let moved = self.facts.trimmed(through);
        self.undrawn.rows.extend(moved);
        let gone = self.blocks.partition_point(|block| block.entry <= through);
        self.blocks.drain(..gone);
        (gone > 0).then_some(Splice { range: 0..gone, count: 0 })
    }

    /// Applies one entry's copy, marking it and the rows it can move.
    fn apply_copy(&mut self, id: EntryId, copied: Copied) {
        let at = self.entries.binary_search_by_key(&id, |(own, _)| *own);
        let index = match (copied, at) {
            (Copied::New(entry), Err(at)) => {
                if let Entry::Tool { content, .. } = &entry {
                    self.undrawn.texts.extend(text_indices(content).map(|item| (id, item)));
                }
                if at == self.entries.len() {
                    // Newer than every held entry; a prepended page's facts
                    // are read again whole.
                    self.facts.added(id, &entry);
                }
                if let Some(tool) = self.facts.pending.get(&id).and_then(|pending| pending.tool) {
                    self.undrawn.rows.add(tool);
                }
                self.entries.insert(at, (id, entry));
                self.undrawn.outputs.remove(&id);
                self.undrawn.mark(id, Mark::New);
                self.undrawn.rows.add(id);
                return;
            }
            // Already held: a page and a live add can overlap.
            (Copied::New(_), Ok(_)) => return,
            (copied, Ok(index)) => (copied, index),
            (Copied::Tail { .. } | Copied::Output { .. } | Copied::Items { .. } | Copied::Whole(_), Err(_)) => return,
        };
        let (copied, index) = index;
        let own = &mut self.entries[index].1;
        let mark = match (copied, own) {
            (
                Copied::Tail { from_byte, text },
                Entry::User { text: own } | Entry::Agent { text: own } | Entry::Thought { text: own },
            ) if own.len() == from_byte => {
                own.push_str(&text);
                self.undrawn.outputs.remove(&id);
                Mark::Grown
            }
            (Copied::Output { item, text, exit }, Entry::Tool { content, .. }) => {
                let Some(ToolItem::Output(output)) = content.get_mut(item) else { return };
                let grown = GrownOutput { item, had_omitted: output.omitted > 0 };
                output.append(&text);
                let ended = output.exit != exit;
                output.exit = exit;
                let undrawn = self.undrawn.entries.get(&id).copied();
                let alone = match undrawn {
                    None => true,
                    Some(Mark::Grown) => self.undrawn.outputs.get(&id).is_some_and(|held| held.item == item),
                    Some(Mark::New | Mark::Changed) => false,
                };
                match (alone && !ended, undrawn) {
                    (true, None) => {
                        self.undrawn.outputs.insert(id, grown);
                    }
                    (true, Some(_)) => {}
                    (false, _) => {
                        self.undrawn.outputs.remove(&id);
                    }
                }
                Mark::Grown
            }
            (Copied::Items { from, items, more }, Entry::Tool { content, more: own_more, .. }) if from <= content.len() => {
                let due = texts_due(&mut self.texts, id, &content[from..], &items, from);
                self.undrawn.texts.extend(due);
                content.truncate(from);
                content.extend(items);
                *own_more = more;
                self.undrawn.outputs.remove(&id);
                Mark::Grown
            }
            (Copied::Whole(entry), own) => {
                let old_tool = self.facts.pending.get(&id).and_then(|pending| pending.tool);
                if let (Entry::Tool { content: old, .. }, Entry::Tool { content: new, .. }) = (&*own, &entry) {
                    let due = texts_due(&mut self.texts, id, old, new, 0);
                    self.undrawn.texts.extend(due);
                } else if let Entry::Tool { content, .. } = &entry {
                    self.undrawn.texts.extend(text_indices(content).map(|item| (id, item)));
                }
                self.facts.replaced(id, &entry);
                *own = entry;
                let new_tool = self.facts.pending.get(&id).and_then(|pending| pending.tool);
                self.undrawn.rows.extend(old_tool.into_iter().chain(new_tool));
                self.undrawn.outputs.remove(&id);
                Mark::Changed
            }
            // A part that doesn't fit the zone's entry: the first half
            // never builds one, since it sized it by this copy.
            (Copied::Tail { .. } | Copied::Output { .. } | Copied::Items { .. } | Copied::New(_), _) => return,
        };
        self.undrawn.mark(id, mark);
        self.undrawn.rows.add(id);
    }

    /// Draws what the update marked (decision 4): each marked message or
    /// thought rendered once and each changed tool text item, spliced in
    /// id order, then the thoughts' openness, the rows a change can have
    /// moved, and last the working row's splice. `Due::Settled` leaves the
    /// grown entries for the next draw (the frame gate, decision 5).
    pub fn draw(&mut self, due: Due) -> Vec<Splice> {
        let mut undrawn = std::mem::take(&mut self.undrawn);
        if due == Due::Settled && !undrawn.all {
            let (grown, rest): (BTreeMap<EntryId, Mark>, BTreeMap<EntryId, Mark>) =
                undrawn.entries.into_iter().partition(|(_, mark)| *mark == Mark::Grown);
            undrawn.entries = rest;
            let (texts, now): (BTreeSet<_>, BTreeSet<_>) =
                undrawn.texts.into_iter().partition(|(id, _)| grown.contains_key(id));
            undrawn.texts = now;
            self.undrawn.outputs = undrawn.outputs.iter().filter(|(id, _)| grown.contains_key(id)).map(|(k, v)| (*k, *v)).collect();
            self.undrawn.texts = texts;
            self.undrawn.entries = grown;
        }
        let mut splices = Vec::new();
        for (id, item) in std::mem::take(&mut undrawn.texts) {
            self.render_text(id, item);
        }
        if undrawn.all {
            for index in 0..self.entries.len() {
                let id = self.entries[index].0;
                self.stats.rendered_entry(&self.entries[index].1);
                let render = render_entry(&self.entries[index].1);
                self.keep_render(id, render);
            }
            let old = self.blocks.len();
            self.blocks = self.entries.iter().flat_map(|(id, entry)| self.draw_entry(*id, entry, false)).collect();
            splices.push(Splice { range: 0..old, count: self.blocks.len() });
        } else {
            let mut run: Option<(usize, Vec<Block>)> = None;
            for (id, mark) in std::mem::take(&mut undrawn.entries) {
                let Ok(index) = self.entries.binary_search_by_key(&id, |(own, _)| *own) else { continue };
                self.render_marked(id, index, mark);
                match mark {
                    Mark::New => {
                        let entry = &self.entries[index].1;
                        let fresh = self.draw_entry(id, entry, self.thought_open(id));
                        let at = self.blocks.partition_point(|block| block.entry < id);
                        if let Some((start, blocks)) = &mut run
                            && *start == at
                        {
                            blocks.extend(fresh);
                            continue;
                        }
                        splices.extend(self.flush(run.take()));
                        let at = self.blocks.partition_point(|block| block.entry < id);
                        run = Some((at, fresh));
                    }
                    Mark::Grown | Mark::Changed => {
                        splices.extend(self.flush(run.take()));
                        let output = undrawn.outputs.get(&id).copied();
                        splices.extend(self.redraw_changed(id, index, mark, output));
                    }
                }
            }
            splices.extend(self.flush(run));
        }
        splices.extend(self.update_thoughts(undrawn.thoughts));
        splices.extend(self.refresh_rows(undrawn.rows));
        splices.extend(self.working());
        splices
    }

    /// The working row's insert or removal, after the blocks' splices, at
    /// the count they left (OpenSpec change `agents-zone-focus-and-scroll`,
    /// decision 5): what changed since the list last had it.
    pub(super) fn working(&mut self) -> Option<Splice> {
        let now = self.working_row().is_some();
        let splice = working_splice(self.drawn_working, now, self.blocks.len());
        self.drawn_working = now;
        splice
    }

    /// Inserts a run of new entries' blocks.
    fn flush(&mut self, run: Option<(usize, Vec<Block>)>) -> Option<Splice> {
        let (at, blocks) = run?;
        let count = blocks.len();
        self.blocks.splice(at..at, blocks);
        (count > 0).then_some(Splice { range: at..at, count })
    }

    /// Renders marked entry `id`'s Markdown, keeping the parts that didn't
    /// change of one drawn before.
    fn render_marked(&mut self, id: EntryId, index: usize, mark: Mark) {
        self.stats.rendered_entry(&self.entries[index].1);
        let render = render_entry(&self.entries[index].1);
        let render = match mark {
            Mark::New => render,
            Mark::Grown | Mark::Changed => render.map(|(fresh, panicked)| {
                let old = self.rendered.get(&id).map_or(&[][..], Vec::as_slice);
                let same = old.iter().zip(&fresh).take_while(|(old, fresh)| old == fresh).count();
                let mut parts = old[..same].to_vec();
                parts.extend(fresh.into_iter().skip(same));
                (parts, panicked)
            }),
        };
        self.keep_render(id, render);
    }

    /// Renders tool entry `id`'s text item `item`, or forgets its render
    /// when it is no longer text. The renderer runs under `catch_unwind` as
    /// for messages: a panic shows the source plain.
    fn render_text(&mut self, id: EntryId, item: usize) {
        let source = match self.held(id) {
            Some(Entry::Tool { content, .. }) => content.get(item).and_then(text_of).map(str::to_string),
            Some(Entry::User { .. } | Entry::Agent { .. } | Entry::Thought { .. } | Entry::Permission { .. } | Entry::Notice { .. })
            | None => None,
        };
        let Some(source) = source else {
            self.texts.remove(&(id, item));
            return;
        };
        self.stats.rendered(&source);
        let blocks: Arc<[Arc<RichBlock>]> = match std::panic::catch_unwind(|| markdown::render(&source)) {
            Ok(parts) => parts.into_iter().map(|part| part.block).collect(),
            Err(_) => {
                if !self.render_failed {
                    self.render_failed = true;
                    log::warn!("the Markdown renderer panicked on agent entry {id}; showing its text plain");
                }
                Arc::from([Arc::new(RichBlock::Paragraph(Inline::plain(source)))])
            }
        };
        self.texts.insert((id, item), blocks);
    }

    /// Redraws drawn entry `id` after a change, from the first block that
    /// differs.
    fn redraw_changed(&mut self, id: EntryId, index: usize, mark: Mark, output: Option<GrownOutput>) -> Vec<Splice> {
        let range = self.entry_blocks(id);
        let open = self.thought_open(id);
        let entry = &self.entries[index].1;
        let (range, fresh) = match entry {
            // A grown user message's earlier parts don't change: only its
            // last and new ones are drawn again.
            Entry::User { text } if mark == Mark::Grown && !range.is_empty() => {
                let last = &self.blocks[range.end - 1];
                (range.end - 1..range.end, super::blocks::user_blocks(id, text, last.part, last.start))
            }
            // An agent message's items from the first that differs: a
            // changed part, or the end of its bubble moving.
            Entry::Agent { .. } => {
                let fresh = self.draw_entry(id, entry, open);
                let same = self.blocks[range.clone()].iter().zip(&fresh).take_while(|(old, new)| old == new).count();
                if same == range.len() && same == fresh.len() {
                    return Vec::new();
                }
                (range.start + same..range.end, fresh[same..].to_vec())
            }
            // A tool call's parts that changed, and the pending permission
            // showing its detail; a permission's row.
            Entry::Tool { .. } | Entry::Permission { .. } => {
                let mut splices: Vec<Splice> = match output {
                    Some(grown) => self.redraw_output(id, grown),
                    None => self.redraw(id),
                }
                .into_iter()
                .collect();
                for permission in self.facts.showing(id) {
                    splices.extend(match output {
                        Some(grown) => self.redraw_permission_output(permission, id, grown),
                        None => self.redraw(permission),
                    });
                }
                return splices;
            }
            Entry::User { .. } | Entry::Thought { .. } | Entry::Notice { .. } => (range, self.draw_entry(id, entry, open)),
        };
        let count = fresh.len();
        self.blocks.splice(range.clone(), fresh);
        vec![Splice { range, count }]
    }

    /// Output item `grown.item` of tool entry `id`'s parts as they are now,
    /// as drawn detail.
    fn output_shown(&self, id: EntryId, item: usize) -> Option<Vec<ShownPart>> {
        let Some(Entry::Tool { content, .. }) = self.held(id) else { return None };
        let Some(ToolItem::Output(output)) = content.get(item) else { return None };
        Some(
            tool::output_parts(item, output)
                .into_iter()
                .map(|part| ShownPart { entry: id, part, text: Arc::from([]) })
                .collect(),
        )
    }

    /// Where output `grown.item`'s parts are among `parts`: from its
    /// dropped-start mark, when it was drawn with one, through its last
    /// line.
    fn output_span(parts: &[&ShownPart], grown: GrownOutput) -> Option<std::ops::Range<usize>> {
        let is_item = |part: &&ShownPart| matches!(&part.part, DetailPart::Output { item, .. } if *item == grown.item);
        let first = parts.iter().position(is_item)?;
        let last = parts.iter().rposition(is_item)? + 1;
        let start = if grown.had_omitted {
            let mark = first.checked_sub(1)?;
            match &parts[mark].part {
                DetailPart::Mark { .. } => mark,
                DetailPart::Text { .. }
                | DetailPart::Diff { .. }
                | DetailPart::Output { .. }
                | DetailPart::Locations { .. }
                | DetailPart::Raw { .. } => return None,
            }
        } else {
            first
        };
        Some(start..last)
    }

    /// Redraws one output's parts of open tool row `id` (decision 6), the
    /// entry's other items' parts kept; the whole row when it isn't drawn
    /// open or the parts can't be found.
    fn redraw_output(&mut self, id: EntryId, grown: GrownOutput) -> Option<Splice> {
        let range = self.entry_blocks(id);
        let drawn_open =
            matches!(self.blocks.get(range.start).map(|block| &block.body), Some(BlockBody::Tool { state: RowState::Open, .. }));
        let open_now = self.held(id).is_some_and(|entry| self.tool_state(id, entry) == RowState::Open);
        if !(drawn_open && open_now) {
            return self.redraw(id);
        }
        let Some(parts) = self.output_shown(id, grown.item) else { return self.redraw(id) };
        let blocks = &self.blocks[range.clone()];
        let shown: Vec<&ShownPart> = blocks
            .iter()
            .filter_map(|block| match &block.body {
                BlockBody::ToolDetail(part) => Some(part),
                BlockBody::UserText { .. }
                | BlockBody::AgentPart { .. }
                | BlockBody::Thought { .. }
                | BlockBody::Tool { .. }
                | BlockBody::Permission { .. }
                | BlockBody::Notice { .. } => None,
            })
            .collect();
        let Some(span) = Self::output_span(&shown, grown) else { return self.redraw(id) };
        // The row's line is the entry's first block; its detail follows.
        let span = span.start + 1..span.end + 1;
        let mut fresh: Vec<Block> = blocks[..span.start].to_vec();
        fresh.extend(parts.into_iter().map(|part| Block { entry: id, part: 0, start: 0, last: false, body: BlockBody::ToolDetail(part) }));
        fresh.extend(blocks[span.end..].iter().cloned());
        let count = fresh.len();
        for (part, block) in fresh.iter_mut().enumerate() {
            block.part = part;
            block.last = part + 1 == count;
        }
        self.splice_entry(range, fresh)
    }

    /// Redraws the detail permission `permission` shows of tool entry
    /// `tool` with one output's parts again.
    fn redraw_permission_output(&mut self, permission: EntryId, tool: EntryId, grown: GrownOutput) -> Option<Splice> {
        let range = self.entry_blocks(permission);
        let Some(BlockBody::Permission { detail, shows: Some(shows), outcome: None, .. }) =
            self.blocks.get(range.start).map(|block| &block.body)
        else {
            return self.redraw(permission);
        };
        if *shows != tool || range.len() != 1 {
            return self.redraw(permission);
        }
        let Some(parts) = self.output_shown(tool, grown.item) else { return self.redraw(permission) };
        let shown: Vec<&ShownPart> = detail.iter().collect();
        let Some(span) = Self::output_span(&shown, grown) else { return self.redraw(permission) };
        let mut fresh: Vec<ShownPart> = detail[..span.start].to_vec();
        fresh.extend(parts);
        fresh.extend(detail[span.end..].iter().cloned());
        if fresh[..] == detail[..] {
            return None;
        }
        let block = &mut self.blocks[range.start];
        if let BlockBody::Permission { detail, .. } = &mut block.body {
            *detail = fresh.into();
        }
        Some(Splice { range, count: 1 })
    }
}

#[cfg(test)]
impl ChangeCopy {
    /// The entries' copies, as the tests read them.
    pub(super) fn copied(&self) -> &[(EntryId, Copied)] {
        &self.entries
    }
}

#[cfg(test)]
mod tests {
    use super::super::harness::*;
    use super::super::*;
    use super::*;
    use crate::agent::shared::Changes;
    use brindle_transcript::ITEM_LIMIT;

    use crate::agent::transcript::{Event, Output, PermissionOutcome, StopReason, ToolStatus};

    fn appended(id: u64, text: &str) -> Event {
        Event::Appended { id: EntryId(id), text: text.into() }
    }

    fn out(id: u64, item: u32, text: &str) -> Event {
        Event::ToolOutput { id: EntryId(id), item, text: text.into(), exit: None }
    }

    fn items(id: u64, from: u32, items: Vec<ToolItem>) -> Event {
        Event::ToolItems { id: EntryId(id), from, items }
    }

    fn text_item(text: &str) -> ToolItem {
        ToolItem::Text { text: text.into(), cut: false }
    }

    fn output_item(text: &str) -> ToolItem {
        ToolItem::Output(Output { command: Some("seq".into()), text: text.into(), ..Output::default() })
    }

    fn lines(from: usize, count: usize) -> String {
        (from..from + count).map(|n| format!("line {n}\n")).collect()
    }

    /// Folds `events` into the zone's shared transcript, with no absorb.
    fn fold(zone: &mut Zone, events: &[Event]) {
        let values: Vec<_> = events.iter().map(Event::to_value).collect();
        zone.shared.apply(&values).unwrap();
    }

    /// What a zone that took the whole transcript at once draws: the
    /// blocks the incremental absorbs must have reached.
    fn drawn_whole(zone: &Zone) -> Vec<Block> {
        let mut fresh = ChatModel::new("Fake", None);
        fresh.session_running();
        fresh.set_policy(zone.model.policy);
        let reset = Changes { reset: true, ..Changes::default() };
        let taken = fresh.copy_changes(zone.shared.transcript(), reset);
        fresh.absorb(taken);
        fresh.draw(Due::All);
        fresh.blocks().to_vec()
    }

    /// The zone's copy of the entries is the transcript's, its blocks are
    /// what a whole draw gives, and its facts what reading them gives.
    fn in_step_whole(zone: &Zone) {
        let held: Vec<(EntryId, Entry)> = zone.shared.transcript().entries().map(|(id, e)| (id, e.clone())).collect();
        assert_eq!(zone.model.entries, held, "the zone's copy");
        assert_eq!(zone.model.blocks(), drawn_whole(zone).as_slice(), "the blocks");
        assert_eq!(zone.model.facts, Facts::read(&zone.model.entries), "the facts");
        assert_drawn_afresh(&zone.model);
    }

    /// A running tool call whose row is open, with a text item, an output
    /// and a diff.
    fn running_tool(zone: &mut Zone, output: &str) {
        zone.feed(&[
            added(1, user("go")),
            turn(TurnState::Working),
            added(2, in_progress("c1", vec![text_item("**t**"), output_item(output), diff_item("/p/main.rs")])),
        ]);
        assert_eq!(row(&zone.model, 2).0, RowState::Open);
    }

    #[test]
    fn the_copy_taken_under_the_lock_holds_only_what_changed() {
        let mut zone = Zone::ready();
        zone.feed(&[
            added(1, user("hi")),
            turn(TurnState::Working),
            added(2, agent("Hel")),
            added(3, in_progress("c1", vec![text_item("a"), text_item("b"), output_item("1\n")])),
        ]);
        fold(&mut zone, &[appended(2, "lo")]);
        let copy = zone.copy();
        assert_eq!(copy.copied(), [(EntryId(2), Copied::Tail { from_byte: 3, text: "lo".into() })]);
        zone.model.absorb(copy);

        fold(&mut zone, &[out(3, 2, "2\n")]);
        let copy = zone.copy();
        assert_eq!(copy.copied(), [(EntryId(3), Copied::Output { item: 2, text: "2\n".into(), exit: None })]);
        zone.model.absorb(copy);

        fold(&mut zone, &[items(3, 2, vec![output_item("x\n")])]);
        let copy = zone.copy();
        assert_eq!(copy.copied(), [(EntryId(3), Copied::Items { from: 2, items: vec![output_item("x\n")], more: 0 })]);
        zone.model.absorb(copy);

        // A tail the zone's text doesn't end where: the entry whole.
        if let Entry::Agent { text } = &mut zone.model.entries[1].1 {
            text.push('!');
        }
        fold(&mut zone, &[appended(2, " you")]);
        let copy = zone.copy();
        assert_eq!(copy.copied(), [(EntryId(2), Copied::Whole(agent("Hello you")))]);
        zone.model.absorb(copy);

        // A reset copies each held entry as new; an added one is new too.
        let log = zone.shared.transcript().replay();
        zone.shared.replay_start();
        fold(&mut zone, &log);
        zone.shared.replay_end();
        let copy = zone.copy();
        let ids: Vec<(EntryId, bool)> =
            copy.copied().iter().map(|(id, copied)| (*id, matches!(copied, Copied::New(_)))).collect();
        assert_eq!(ids, [(EntryId(1), true), (EntryId(2), true), (EntryId(3), true)]);
        zone.model.absorb(copy);
        fold(&mut zone, &[added(4, agent("more"))]);
        assert_eq!(zone.copy().copied(), [(EntryId(4), Copied::New(agent("more")))]);
    }

    #[test]
    fn absorb_renders_nothing_and_draw_renders_each_marked_message_once() {
        let mut zone = Zone::ready();
        zone.feed(&[added(1, user("hi")), added(2, agent("a")), added(3, thought("t"))]);
        let before = zone.model.render_stats();
        for event in [appended(2, "b"), appended(3, "u"), appended(2, "c")] {
            fold(&mut zone, &[event]);
            zone.absorb_undrawn();
        }
        fold(&mut zone, &[added(4, agent("new"))]);
        zone.absorb_undrawn();
        assert_eq!(zone.model.render_stats(), before, "the absorbs rendered nothing");
        zone.model.draw(Due::All);
        assert_eq!(zone.model.render_stats().renders - before.renders, 3, "each marked message once");
        in_step(&zone.model);
        in_step_whole(&zone);
        assert_eq!(zone.model.draw(Due::All), [], "nothing left to draw");
    }

    #[test]
    fn a_reset_is_drawn_as_one_splice_of_the_whole_list() {
        let mut zone = Zone::ready();
        zone.feed(&[added(1, user("hi")), added(2, agent("one\n\ntwo"))]);
        let old = zone.model.blocks().len();
        let absorbed = zone.replay(&[ready(), added(1, user("hi")), added(2, agent("one")), added(3, agent("three"))]);
        assert_eq!(absorbed.splices, [Splice { range: 0..old, count: zone.model.blocks().len() }]);
        in_step_whole(&zone);
    }

    #[test]
    fn a_prepended_page_is_drawn_as_one_insert_at_the_start() {
        let server = long_log(40);
        let mut zone = paged(&server);
        let shown = zone.model.blocks().len();
        let fetch = zone.model.fetch_due(true, false).expect("a fetch while the row shows");
        zone.model.fetching(7);
        let before = zone.model.render_stats().renders;
        let absorbed = answer_fetch(&mut zone, &server, fetch, 7);
        let added = zone.model.blocks().len() - shown;
        assert_eq!(absorbed.splices, [Splice { range: 0..0, count: added }]);
        let first = zone.model.entries.first().map(|(id, _)| *id).unwrap();
        let fetched = zone.model.entries.iter().take_while(|(id, _)| *id < EntryId(first.0 + 100)).count();
        assert!(fetched > 0);
        let markdown = zone
            .model
            .entries
            .iter()
            .filter(|(id, entry)| {
                *id < zone.shared.transcript().entries().nth(fetched).map_or(EntryId(u64::MAX), |(id, _)| id)
                    && matches!(entry, Entry::Agent { .. } | Entry::Thought { .. })
            })
            .count() as u64;
        assert!(zone.model.render_stats().renders - before <= markdown, "each fetched message rendered once");
        in_step(&zone.model);
        assert_eq!(zone.model.facts, Facts::read(&zone.model.entries));
    }

    #[test]
    fn a_toggle_draws_marked_entries_first() {
        let mut zone = Zone::ready();
        zone.feed(&[added(1, user("hi")), added(2, thought("a"))]);
        let range = zone.model.entry_blocks(EntryId(2));
        fold(&mut zone, &[appended(2, "b")]);
        zone.absorb_undrawn();
        let splices = zone.model.toggle_thought(EntryId(2));
        assert_eq!(splices, [Splice { range: range.clone(), count: 1 }, Splice { range: range.clone(), count: 1 }]);
        let BlockBody::Thought { open, .. } = &zone.model.blocks()[range.start].body else { panic!() };
        assert!(*open);
        assert_eq!(texts(&zone.model)[1], "ab");
        assert_drawn_afresh(&zone.model);
    }

    #[test]
    fn the_working_rows_splice_is_the_last_splice_a_draw_returns() {
        let mut zone = Zone::ready();
        zone.feed(&[added(1, user("hi"))]);
        let absorbed = zone.feed(&[turn(TurnState::Working), added(2, agent("x")), added(3, agent("y"))]);
        let len = zone.model.blocks().len();
        assert_eq!(absorbed.splices.last(), Some(&Splice { range: len..len, count: 1 }));
        assert!(absorbed.splices[..absorbed.splices.len() - 1].iter().all(|splice| splice.range.end <= len));
        let absorbed = zone.feed(&[appended(3, "z"), turn(TurnState::Ended { stop: StopReason::EndTurn })]);
        let len = zone.model.blocks().len();
        assert_eq!(absorbed.splices.last(), Some(&Splice { range: len..len + 1, count: 0 }));
        assert_eq!(zone.model.item_count(), len);
    }

    /// Decision 6: an output's deltas appended to the zone's copy give the
    /// transcript's item, over split points of a 100 KiB output in 1, 2 and
    /// 3 deltas (past `ITEM_LIMIT`), `omitted` and `partial` included.
    #[test]
    fn an_output_delta_appended_to_the_zones_copy_equals_the_transcripts() {
        let text: String = (0..12_000).map(|n| if n % 7 == 0 { format!("{}\n", "y".repeat(n % 300)) } else { format!("l{n}\n") }).collect();
        let text = &text[..text.floor_char_boundary(100 * 1024)];
        assert!(text.len() > ITEM_LIMIT);
        let folded = |pieces: &[&str]| -> Vec<Output> {
            let mut log = Transcript::new();
            log.apply(&added(1, in_progress("c1", vec![ToolItem::Output(Output::default())]))).unwrap();
            pieces
                .iter()
                .map(|piece| {
                    log.apply(&out(1, 0, piece)).unwrap();
                    let Some(Entry::Tool { content, .. }) = log.entry(EntryId(1)) else { panic!() };
                    let ToolItem::Output(output) = &content[0] else { panic!() };
                    output.clone()
                })
                .collect()
        };
        let check = |pieces: &[&str]| {
            let states = folded(pieces);
            let mut own = Output::default();
            for (piece, now) in pieces.iter().zip(&states) {
                match output_delta(&own, now) {
                    Some(delta) => {
                        own.append(&delta);
                        assert_eq!(&own, now, "after {} bytes", piece.len());
                    }
                    // The piece pushed some of itself out: the zone copies
                    // the item instead.
                    None => {
                        assert!(piece.len() > now.text.as_str().len());
                        own = now.clone();
                    }
                }
            }
        };
        let cuts: Vec<usize> = (0..text.len()).step_by(997).chain([1, ITEM_LIMIT - 1, ITEM_LIMIT, ITEM_LIMIT + 1]).filter(|at| text.is_char_boundary(*at)).collect();
        check(&[text]);
        for &at in &cuts {
            check(&[&text[..at], &text[at..]]);
            let second = (at + ITEM_LIMIT / 3).min(text.len());
            if text.is_char_boundary(second) {
                check(&[&text[..at], &text[at..second], &text[second..]]);
            }
        }
    }

    #[test]
    fn an_output_delta_redraws_only_its_last_part() {
        let mut zone = Zone::ready();
        running_tool(&mut zone, &lines(0, 120));
        let range = zone.model.entry_blocks(EntryId(2));
        let before = zone.model.render_stats();
        let absorbed = zone.feed(&[out(2, 1, "more\n")]);
        // The row, the text part, then three output parts and the diff.
        let last_output = range.start + 4;
        assert_eq!(absorbed.splices, [Splice { range: last_output..last_output + 1, count: 1 }]);
        assert_eq!(zone.model.render_stats(), before, "the text item isn't rendered again");
        in_step_whole(&zone);
    }

    #[test]
    fn an_output_whose_front_was_dropped_redraws_all_its_parts_and_no_other_items() {
        let mut zone = Zone::ready();
        let line = format!("{}\n", "x".repeat(99));
        running_tool(&mut zone, &line.repeat(ITEM_LIMIT / 100 - 5));
        let range = zone.model.entry_blocks(EntryId(2));
        let before = zone.model.render_stats();
        let diff_before = zone.model.blocks()[range.end - 1].body.clone();
        let absorbed = zone.feed(&[out(2, 1, &line.repeat(20))]);
        let Entry::Tool { content, .. } = zone.model.held(EntryId(2)).unwrap() else { panic!() };
        let ToolItem::Output(output) = &content[1] else { panic!() };
        assert!(output.omitted > 0, "the front was dropped");
        // Neither the row's line nor the text part is spliced; the diff
        // after is the same part, renumbered when the output's count of
        // parts moved (a block's `part` is its place in the entry).
        let range_now = zone.model.entry_blocks(EntryId(2));
        for splice in &absorbed.splices {
            assert!(splice.range.start >= range.start + 2, "{splice:?}");
        }
        assert_eq!(zone.model.blocks()[range_now.end - 1].body, diff_before);
        assert_eq!(zone.model.render_stats(), before);
        in_step_whole(&zone);
    }

    #[test]
    fn the_detail_drawn_from_deltas_equals_the_detail_drawn_whole() {
        let mut state: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = move |n: u64| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state % n
        };
        for _ in 0..20 {
            let mut zone = Zone::ready();
            running_tool(&mut zone, "");
            let mut line = 0;
            for _ in 0..30 {
                let count = next(80) as usize;
                let mut text = lines(line, count);
                line += count;
                if next(4) == 0 {
                    text.push_str(&"z".repeat(next(9000) as usize));
                }
                zone.feed(&[out(2, 1, &text)]);
                in_step_whole(&zone);
            }
        }
    }

    #[test]
    fn a_pending_permission_showing_the_tool_is_redrawn_the_same_way() {
        let mut zone = Zone::ready();
        running_tool(&mut zone, &lines(0, 10));
        let Entry::Permission { title, options, outcome, .. } = permission("Run it") else { unreachable!() };
        zone.feed(&[added(3, Entry::Permission { call: Some("c1".into()), title, options, outcome })]);
        assert_eq!(row(&zone.model, 2).0, RowState::WithPermission);
        let permission = zone.model.entry_blocks(EntryId(3));
        let absorbed = zone.feed(&[out(2, 1, "eleven\n")]);
        assert_eq!(absorbed.splices, [Splice { range: permission, count: 1 }]);
        in_step_whole(&zone);
    }

    /// Generated conversations (decision 7): users, tools across turns,
    /// permissions added and answered, outputs, trims and resets.
    fn generated(seed: u64, steps: usize, mut check: impl FnMut(&Zone)) {
        let mut state = seed;
        let mut next = move |n: u64| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state % n
        };
        let mut zone = Zone::ready();
        let mut id = 1;
        let mut tools: Vec<u64> = Vec::new();
        let mut pending: Vec<u64> = Vec::new();
        for _ in 0..steps {
            let first = zone.shared.transcript().entries().next().map_or(id, |(first, _)| first.0);
            let event = match next(11) {
                0 => Some(added(id, user("go"))),
                1 => Some(added(id, agent("ok"))),
                2 => {
                    let status = [ToolStatus::Pending, ToolStatus::InProgress, ToolStatus::Completed][next(3) as usize];
                    tools.push(id);
                    Some(added(id, tool_entry(&format!("c{}", next(3)), status, vec![output_item("a\n")])))
                }
                3 => tools.iter().filter(|tool| **tool >= first).nth(next(4) as usize).map(|tool| tool_completed(*tool)),
                4 => {
                    let Entry::Permission { title, options, outcome, .. } = permission("May I?") else { unreachable!() };
                    pending.push(id);
                    Some(added(id, Entry::Permission { call: Some(format!("c{}", next(3))), title, options, outcome }))
                }
                5 => {
                    let held: Vec<u64> = pending.iter().copied().filter(|p| *p >= first).collect();
                    held.get(next(3) as usize).map(|p| {
                        pending.retain(|own| own != p);
                        Event::Answered { id: EntryId(*p), outcome: PermissionOutcome::Cancelled }
                    })
                }
                6 => Some(turn(match next(3) {
                    0 => TurnState::Working,
                    1 => TurnState::Idle,
                    _ => TurnState::Ended { stop: StopReason::EndTurn },
                })),
                7 => tools.iter().filter(|tool| **tool >= first).nth(next(3) as usize).map(|tool| out(*tool, 0, "b\n")),
                8 if id > first + 3 => {
                    let through = first + next(id - first - 1);
                    let dropped = through - first + 1;
                    Some(Event::Trimmed { through: EntryId(through), dropped })
                }
                9 => {
                    let log = zone.shared.transcript().replay();
                    zone.replay(&log);
                    check(&zone);
                    None
                }
                _ => Some(added(id, thought("hmm"))),
            };
            let Some(event) = event else { continue };
            if matches!(event, Event::Added { .. }) {
                id += 1;
            }
            zone.feed(&[event]);
            check(&zone);
        }
    }

    #[test]
    fn facts_kept_on_change_equal_facts_read_again() {
        for seed in 1..=60 {
            generated(seed * 0x9e37_79b9, 60, |zone| assert_eq!(zone.model.facts, Facts::read(&zone.model.entries)));
        }
    }

    #[test]
    fn rows_refreshed_by_cause_equal_rows_refreshed_whole() {
        for seed in 1..=60 {
            generated(seed * 0x51_7cc1, 60, in_step_whole);
        }
    }

    #[test]
    fn pending_permissions_come_from_the_set_in_id_order() {
        let mut zone = Zone::ready();
        let ids = |zone: &Zone| zone.model.pending_permissions().into_iter().map(|(id, _)| id.0).collect::<Vec<_>>();
        zone.feed(&[added(1, user("go")), added(2, permission("One")), added(3, permission("Two"))]);
        assert_eq!(ids(&zone), [2, 3]);
        assert_eq!(zone.model.pending_permissions()[0].1, ["reject-once", "allow-once"], "reject first");
        fold(&mut zone, &[Event::Answered { id: EntryId(2), outcome: PermissionOutcome::Cancelled }, added(4, permission("Three"))]);
        zone.absorb_undrawn();
        assert_eq!(ids(&zone), [3, 4], "from the absorb, before the draw");
        assert!(zone.model.answer(EntryId(4), "allow-once") && !zone.model.answer(EntryId(2), "allow-once"));
        zone.model.draw(Due::All);
        assert_eq!(zone.model.permission_block(EntryId(4)), Some(zone.model.entry_blocks(EntryId(4)).start));
        assert_eq!(zone.model.permission_block(EntryId(2)), None);
    }

    #[test]
    fn a_tool_text_item_is_rendered_again_only_when_its_text_changed() {
        let mut zone = Zone::ready();
        zone.feed(&[added(1, user("go")), turn(TurnState::Working), added(2, in_progress("c1", vec![text_item("a"), text_item("b")]))]);
        let before = zone.model.render_stats().renders;
        zone.feed(&[items(2, 1, vec![text_item("b")])]);
        assert_eq!(zone.model.render_stats().renders, before, "item 1 unchanged");
        zone.feed(&[items(2, 1, vec![text_item("c")])]);
        assert_eq!(zone.model.render_stats().renders, before + 1, "item 1 changed, rendered once");
        assert_eq!(zone.model.texts.len(), 2);
        in_step_whole(&zone);
        zone.feed(&[items(2, 1, vec![output_item("x\n")])]);
        assert_eq!(zone.model.texts.keys().copied().collect::<Vec<_>>(), [(EntryId(2), 0)], "no longer text: forgotten");
    }
}
