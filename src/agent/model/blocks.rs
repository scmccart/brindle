//! The zone's blocks: each entry drawn as its parts, the splices an absorb
//! makes, the tool rows and their details, the thoughts' openness, and the
//! copy actions' text.

use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Arc;

use brindle_ui::{Inline, RichBlock};
use gpui::SharedString;

use crate::agent::markdown::{self, Part};
use crate::agent::shared::Dirty;
use crate::agent::tool::{self, DetailPart, RowState, Shown, ToolPolicy};
use crate::agent::transcript::{
    Entry, EntryId, NoticeLevel, PermissionKind, PermissionOption, PermissionOutcome, ToolItem, ToolKind, ToolStatus,
    Transcript,
};

use super::{BLOCK_LIMIT, ChatModel, running};

/// One of a permission request's buttons, keyed by its entry and `id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionButton {
    pub id: SharedString,
    pub name: SharedString,
    pub kind: PermissionKind,
}

/// What a block draws.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BlockBody {
    /// A part of a user message, plain ([`parts`]).
    UserText { text: SharedString },
    /// One top-level Markdown block of an agent message.
    AgentPart { block: Arc<RichBlock> },
    /// A thought, whole: all its Markdown blocks.
    Thought { blocks: Arc<[Arc<RichBlock>]>, open: bool },
    /// A tool call's line: its summary ([`tool::summary`]) and how its row
    /// is drawn ([`tool::row_state`]). An open row's detail follows as
    /// [`BlockBody::ToolDetail`] blocks of the same entry.
    Tool { kind: ToolKind, title: SharedString, status: ToolStatus, summary: Option<SharedString>, state: RowState },
    /// One part of an open tool call's detail.
    ToolDetail(ShownPart),
    /// The buttons, reject kinds first, then allow kinds, each group in
    /// the agent's order. While pending, `detail` holds the parts of the
    /// tool entry `shows` (the one its call names), drawn inside the row
    /// above the buttons (OpenSpec change `agents-tool-content` decision
    /// 10, flag 25).
    Permission {
        title: SharedString,
        options: Vec<PermissionButton>,
        outcome: Option<PermissionOutcome>,
        detail: Arc<[ShownPart]>,
        shows: Option<EntryId>,
    },
    Notice { level: NoticeLevel, text: SharedString },
}

/// Rendered Markdown, shared between the cache and the blocks.
pub type Rich = Arc<[Arc<RichBlock>]>;

/// A text item's Markdown source and its rendering.
pub(super) type RenderedText = (SharedString, Rich);

/// A detail part as drawn: the part, and a text item's rendered Markdown
/// (empty for the other parts).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShownPart {
    pub entry: EntryId,
    pub part: DetailPart,
    pub text: Rich,
}

/// What a tool entry's or a permission's blocks show beyond the entry
/// itself: a tool row's state and, when open, its detail; a permission's
/// tool entry and that entry's detail.
#[derive(Debug, Clone, Default)]
pub(super) struct ToolDraw {
    state: Option<RowState>,
    detail: Vec<ShownPart>,
    shows: Option<EntryId>,
}

/// One item of the conversation's list: an entry, or one part of a message
/// (`part` counts from 0; `start` is its first byte in the entry's text;
/// `last` is the entry's last part, where its bubble ends).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub entry: EntryId,
    pub part: usize,
    pub start: usize,
    pub last: bool,
    pub body: BlockBody,
}

/// A change to the conversation's list, as the view's list state takes
/// it: the items in `range` replaced by `count` new ones.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Splice {
    pub range: Range<usize>,
    pub count: usize,
}

/// The list's change for the working row (OpenSpec change
/// `agents-zone-attention`, decision 2), the item after the model's `len`
/// blocks: inserted when it appears (`before` it didn't show, `after` it
/// does), removed when it goes, nothing otherwise. `ChatModel::finish`
/// puts it after the blocks' splices, which all fall within the blocks
/// (`agents-zone-focus-and-scroll`, decision 5).
pub(super) fn working_splice(before: bool, after: bool, len: usize) -> Option<Splice> {
    match (before, after) {
        (false, true) => Some(Splice { range: len..len, count: 1 }),
        (true, false) => Some(Splice { range: len..len + 1, count: 0 }),
        (false, false) | (true, true) => None,
    }
}

/// The byte ranges of `text` its parts cover, from `from` (the start of a
/// part) on: a part ends at a blank line, or, once it is past
/// [`BLOCK_LIMIT`], at the next line break. Line breaks between parts
/// belong to none, so a part never starts with one. A cut point depends
/// only on the text before it, so text appended to a message changes only
/// its last part and adds new ones. A message with no text but line breaks
/// is one empty part.
pub fn parts(text: &str, from: usize) -> Vec<Range<usize>> {
    let from = from.min(text.len());
    let mut parts = Vec::new();
    let mut start = from;
    loop {
        start += text[start..].bytes().take_while(|b| *b == b'\n').count();
        if start >= text.len() {
            break;
        }
        let rest = &text[start..];
        let paragraph = rest.find("\n\n").map(|p| start + p);
        let long = if rest.len() > BLOCK_LIMIT {
            let mut cut = start + BLOCK_LIMIT;
            while !text.is_char_boundary(cut) {
                cut += 1;
            }
            text[cut..].find('\n').map(|q| cut + q)
        } else {
            None
        };
        let end = match (paragraph, long) {
            (Some(p), Some(q)) => p.min(q),
            (Some(cut), None) | (None, Some(cut)) => cut,
            (None, None) => text.len(),
        };
        parts.push(start..end);
        start = end;
    }
    if parts.is_empty() {
        parts.push(from..from);
    }
    parts
}

fn is_allow(kind: PermissionKind) -> bool {
    match kind {
        PermissionKind::AllowOnce | PermissionKind::AllowAlways => true,
        PermissionKind::RejectOnce | PermissionKind::RejectAlways => false,
    }
}

/// A permission request's buttons: reject kinds first, then allow kinds,
/// each in the agent's order (OpenSpec change `agents-zone-attention`,
/// decision 4), so the keyboard reaches the safe answer first.
pub(super) fn buttons(options: &[PermissionOption]) -> Vec<PermissionButton> {
    let reject = options.iter().filter(|o| !is_allow(o.kind));
    let allow = options.iter().filter(|o| is_allow(o.kind));
    reject
        .chain(allow)
        .map(|o| PermissionButton { id: o.id.clone().into(), name: o.name.clone().into(), kind: o.kind })
        .collect()
}

/// The blocks of user message `id`, from its part `part` starting at byte
/// `from` of its text (0 and 0 for the whole message).
fn user_blocks(id: EntryId, text: &str, part: usize, from: usize) -> Vec<Block> {
    let ranges = parts(text, from);
    let count = ranges.len();
    ranges
        .into_iter()
        .enumerate()
        .map(|(i, range)| Block {
            entry: id,
            part: part + i,
            start: range.start,
            last: i + 1 == count,
            body: BlockBody::UserText { text: text.get(range).unwrap_or_default().to_string().into() },
        })
        .collect()
}

/// The blocks an entry is drawn as: one per part of a user message
/// ([`parts`]) or per rendered part of an agent message (`rendered`), else
/// one; a thought holds its `rendered` blocks and is drawn open when
/// `thought_open`.
pub(super) fn blocks_of(id: EntryId, entry: &Entry, rendered: &[Part], thought_open: bool, draw: ToolDraw) -> Vec<Block> {
    let one = |body| vec![Block { entry: id, part: 0, start: 0, last: true, body }];
    match entry {
        Entry::User { text } => user_blocks(id, text, 0, 0),
        Entry::Agent { .. } => rendered
            .iter()
            .enumerate()
            .map(|(i, part)| Block {
                entry: id,
                part: i,
                start: part.range.start,
                last: i + 1 == rendered.len(),
                body: BlockBody::AgentPart { block: part.block.clone() },
            })
            .collect(),
        Entry::Thought { .. } => {
            let blocks = rendered.iter().map(|part| part.block.clone()).collect();
            one(BlockBody::Thought { blocks, open: thought_open })
        }
        Entry::Tool { title, kind, status, .. } => {
            let state = draw.state.unwrap_or(RowState::Nothing);
            let detail = match state {
                RowState::Open => draw.detail,
                RowState::Closed | RowState::WithPermission | RowState::Nothing => Vec::new(),
            };
            let row = BlockBody::Tool {
                kind: *kind,
                title: title.clone().into(),
                status: *status,
                summary: tool::summary(entry),
                state,
            };
            let count = detail.len() + 1;
            std::iter::once(row)
                .chain(detail.into_iter().map(BlockBody::ToolDetail))
                .enumerate()
                .map(|(part, body)| Block { entry: id, part, start: 0, last: part + 1 == count, body })
                .collect()
        }
        Entry::Permission { title, options, outcome, .. } => one(BlockBody::Permission {
            title: title.clone().into(),
            options: buttons(options),
            outcome: outcome.clone(),
            detail: if outcome.is_none() { draw.detail.into() } else { Arc::from([]) },
            shows: if outcome.is_none() { draw.shows } else { None },
        }),
        Entry::Notice { level, text } => one(BlockBody::Notice { level: *level, text: text.clone().into() }),
    }
}

/// An agent message's or thought's text rendered ([`markdown::render`]),
/// and whether the renderer panicked, when its text is shown as one plain
/// paragraph instead; `None` for the entries that aren't Markdown.
pub(super) fn render_entry(entry: &Entry) -> Option<(Vec<Part>, bool)> {
    let text = match entry {
        Entry::Agent { text } | Entry::Thought { text } => text,
        Entry::User { .. } | Entry::Tool { .. } | Entry::Permission { .. } | Entry::Notice { .. } => return None,
    };
    Some(match std::panic::catch_unwind(|| markdown::render(text)) {
        Ok(parts) => (parts, false),
        Err(_) => {
            let block = RichBlock::Paragraph(Inline::plain(text.clone()));
            (vec![Part { range: 0..text.len(), block: Arc::new(block) }], true)
        }
    })
}

/// A tool entry's text items, by index, with their Markdown sources; none
/// for the other entries.
pub(super) fn text_items(entry: &Entry) -> Vec<(usize, SharedString)> {
    let Entry::Tool { content, .. } = entry else { return Vec::new() };
    content
        .iter()
        .enumerate()
        .filter_map(|(item, content)| match content {
            ToolItem::Text { text, .. } => Some((item, SharedString::from(text.clone()))),
            ToolItem::Diff(_) | ToolItem::Output(_) => None,
        })
        .collect()
}

impl ChatModel {
    /// The conversation list's items: the blocks, then the working row
    /// while it shows (OpenSpec change `agents-zone-focus-and-scroll`,
    /// decision 5).
    pub fn item_count(&self) -> usize {
        self.blocks.len() + usize::from(self.working_row().is_some())
    }

    /// Copies the entries a fetched page put in front of the held ones,
    /// renders their Markdown once and draws them, with each thought open
    /// as the user left it; gives the splice at the list's start.
    pub(super) fn prepend_entries(&mut self, transcript: &Transcript, added_from: Option<EntryId>) -> Option<Splice> {
        let held = self.entries.first().map(|(id, _)| *id).or(added_from);
        let fresh: Vec<(EntryId, Entry)> = transcript
            .entries()
            .take_while(|(id, _)| held.is_none_or(|held| *id < held))
            .map(|(id, entry)| (id, entry.clone()))
            .collect();
        if fresh.is_empty() {
            return None;
        }
        for (id, entry) in &fresh {
            self.stats.rendered_entry(entry);
            self.keep_render(*id, render_entry(entry));
            self.keep_texts(*id, text_items(entry));
        }
        let blocks: Vec<Block> =
            fresh.iter().flat_map(|(id, entry)| self.draw(*id, entry, self.thought_open(*id))).collect();
        let count = blocks.len();
        self.blocks.splice(0..0, blocks);
        self.entries.splice(0..0, fresh);
        Some(Splice { range: 0..0, count })
    }

    /// Copies entry `id` again (its tail only when text was appended) and
    /// redraws its blocks from the first that changed.
    pub(super) fn update_entry(&mut self, transcript: &Transcript, id: EntryId, dirty: Dirty) -> Vec<Splice> {
        let Some(source) = transcript.entry(id) else { return Vec::new() };
        let Ok(index) = self.entries.binary_search_by_key(&id, |(own, _)| *own) else { return Vec::new() };
        let own = &mut self.entries[index].1;
        let appended = match (dirty, &mut *own, source) {
            (
                Dirty::Appended { from_byte },
                Entry::User { text: own } | Entry::Agent { text: own } | Entry::Thought { text: own },
                Entry::User { text } | Entry::Agent { text } | Entry::Thought { text },
            ) if own.len() == from_byte && text.is_char_boundary(from_byte) => {
                own.push_str(&text[from_byte..]);
                true
            }
            (Dirty::Appended { .. } | Dirty::ToolItems { .. } | Dirty::Output { .. } | Dirty::Replaced, _, _) => false,
        };
        if !appended {
            *own = source.clone();
        }
        // A tool call's text items are rendered again only where their
        // source changed (decision 10's cache by entry and item).
        let items = text_items(&self.entries[index].1);
        self.keep_texts(id, items);
        // An agent message or thought is rendered again whole; the parts
        // that didn't change keep their cached values.
        self.stats.rendered_entry(&self.entries[index].1);
        let render = render_entry(&self.entries[index].1).map(|(fresh, panicked)| {
            let old = self.rendered.get(&id).map_or(&[][..], Vec::as_slice);
            let same = old.iter().zip(&fresh).take_while(|(old, fresh)| old == fresh).count();
            let mut parts = old[..same].to_vec();
            parts.extend(fresh.into_iter().skip(same));
            (parts, panicked)
        });
        self.keep_render(id, render);
        let range = self.entry_blocks(id);
        let open = self.thought_open(id);
        let entry = &self.entries[index].1;
        let (range, fresh) = match entry {
            // A user message's earlier parts don't change: only its last
            // and new ones are drawn again.
            Entry::User { text } if !range.is_empty() => {
                let last = &self.blocks[range.end - 1];
                (range.end - 1..range.end, user_blocks(id, text, last.part, last.start))
            }
            // An agent message's items from the first that differs: a
            // changed part, or the end of its bubble moving.
            Entry::Agent { .. } => {
                let fresh = self.draw(id, entry, open);
                let same = self.blocks[range.clone()].iter().zip(&fresh).take_while(|(old, new)| old == new).count();
                if same == range.len() && same == fresh.len() {
                    return Vec::new();
                }
                (range.start + same..range.end, fresh[same..].to_vec())
            }
            // A tool call's parts that changed, and the pending permission
            // showing its detail; a permission's row.
            Entry::Tool { .. } | Entry::Permission { .. } => {
                let mut splices: Vec<Splice> = self.redraw(id).into_iter().collect();
                let permission = self.permission_tools.iter().find(|(_, tool)| **tool == id).map(|(perm, _)| *perm);
                splices.extend(permission.and_then(|permission| self.redraw(permission)));
                return splices;
            }
            Entry::User { .. } | Entry::Thought { .. } | Entry::Notice { .. } => (range, self.draw(id, entry, open)),
        };
        let count = fresh.len();
        self.blocks.splice(range.clone(), fresh);
        vec![Splice { range, count }]
    }

    /// Draws entry `id` again, splicing only the blocks that differ (a
    /// common start and end kept): an open row's growing output changes
    /// its last part alone.
    fn redraw(&mut self, id: EntryId) -> Option<Splice> {
        let index = self.entries.binary_search_by_key(&id, |(own, _)| *own).ok()?;
        let entry = &self.entries[index].1;
        let fresh = self.draw(id, entry, self.thought_open(id));
        let range = self.entry_blocks(id);
        let old = &self.blocks[range.clone()];
        let head = old.iter().zip(&fresh).take_while(|(old, new)| old == new).count();
        if head == old.len() && head == fresh.len() {
            return None;
        }
        let room = old.len().min(fresh.len()) - head;
        let tail = old.iter().rev().zip(fresh.iter().rev()).take(room).take_while(|(old, new)| old == new).count();
        let range = range.start + head..range.end - tail;
        let new: Vec<Block> = fresh[head..fresh.len() - tail].to_vec();
        let count = new.len();
        self.blocks.splice(range.clone(), new);
        Some(Splice { range, count })
    }

    /// Renders tool entry `id`'s text `items` (index and source) whose
    /// source changed, and forgets its other items' renders. The renderer
    /// runs under `catch_unwind` as for messages: a panic shows the source
    /// plain.
    pub(super) fn keep_texts(&mut self, id: EntryId, items: Vec<(usize, SharedString)>) {
        let held: Vec<(EntryId, usize)> = self.texts.range((id, 0)..=(id, usize::MAX)).map(|(key, _)| *key).collect();
        for key in held {
            if !items.iter().any(|(item, _)| *item == key.1) {
                self.texts.remove(&key);
            }
        }
        for (item, source) in items {
            if self.texts.get(&(id, item)).is_some_and(|(held, _)| *held == source) {
                continue;
            }
            self.stats.rendered(&source);
            let blocks: Arc<[Arc<RichBlock>]> = match std::panic::catch_unwind(|| markdown::render(&source)) {
                Ok(parts) => parts.into_iter().map(|part| part.block).collect(),
                Err(_) => {
                    if !self.render_failed {
                        self.render_failed = true;
                        log::warn!("the Markdown renderer panicked on agent entry {id}; showing its text plain");
                    }
                    Arc::from([Arc::new(RichBlock::Paragraph(Inline::plain(source.to_string())))])
                }
            };
            self.texts.insert((id, item), (source, blocks));
        }
    }

    /// The latest user message, and each pending permission's tool entry,
    /// read again from the held entries.
    pub(super) fn refresh_facts(&mut self) {
        self.last_user =
            self.entries.iter().rev().find(|(_, entry)| matches!(entry, Entry::User { .. })).map(|(id, _)| *id);
        let mut newest: std::collections::HashMap<&str, EntryId> = std::collections::HashMap::new();
        let mut tools = BTreeMap::new();
        for (id, entry) in &self.entries {
            match entry {
                Entry::Tool { call, .. } => {
                    newest.insert(call.as_str(), *id);
                }
                Entry::Permission { call: Some(call), outcome: None, .. } => {
                    if let Some(tool) = newest.get(call.as_str()) {
                        tools.insert(*id, *tool);
                    }
                }
                Entry::Permission { .. }
                | Entry::User { .. }
                | Entry::Agent { .. }
                | Entry::Thought { .. }
                | Entry::Notice { .. } => {}
            }
        }
        self.permission_tools = tools;
    }

    /// Whether tool call `id` runs (decision 10): its status is pending or
    /// in progress, the turn runs, and it comes after the latest user
    /// message held (or none is held: the turn's prompt is above the page).
    fn tool_running(&self, id: EntryId, status: ToolStatus) -> bool {
        tool::unfinished(status) && running(&self.turn) && self.last_user.is_none_or(|user| id > user)
    }

    /// How tool entry `id`'s row is drawn now.
    pub(super) fn tool_state(&self, id: EntryId, entry: &Entry) -> RowState {
        let Entry::Tool { status, .. } = entry else { return RowState::Nothing };
        let pending = self.permission_tools.values().any(|tool| *tool == id);
        tool::row_state(entry, self.by_hand.get(&id).copied(), self.tool_running(id, *status), pending, self.policy)
    }

    /// Tool entry `id`'s detail parts as drawn, its text items rendered.
    fn shown_parts(&self, id: EntryId, entry: &Entry) -> Vec<ShownPart> {
        tool::detail(entry)
            .into_iter()
            .map(|part| {
                let text = match &part {
                    DetailPart::Text { item, .. } => {
                        self.texts.get(&(id, *item)).map_or_else(|| Arc::from([]), |(_, blocks)| blocks.clone())
                    }
                    DetailPart::Diff { .. }
                    | DetailPart::Output { .. }
                    | DetailPart::Locations { .. }
                    | DetailPart::Raw { .. }
                    | DetailPart::Mark { .. } => Arc::from([]),
                };
                ShownPart { entry: id, part, text }
            })
            .collect()
    }

    /// What tool entry or permission `id` shows beyond itself.
    fn tool_draw(&self, id: EntryId, entry: &Entry) -> ToolDraw {
        match entry {
            Entry::Tool { .. } => {
                let state = self.tool_state(id, entry);
                let detail = match state {
                    RowState::Open => self.shown_parts(id, entry),
                    RowState::Closed | RowState::WithPermission | RowState::Nothing => Vec::new(),
                };
                ToolDraw { state: Some(state), detail, shows: None }
            }
            Entry::Permission { .. } => {
                let Some(tool) = self.permission_tools.get(&id).copied() else { return ToolDraw::default() };
                let Some(tool_entry) = self.held(tool) else { return ToolDraw::default() };
                ToolDraw { state: None, detail: self.shown_parts(tool, tool_entry), shows: Some(tool) }
            }
            Entry::User { .. } | Entry::Agent { .. } | Entry::Thought { .. } | Entry::Notice { .. } => {
                ToolDraw::default()
            }
        }
    }

    /// Held entry `id`.
    fn held(&self, id: EntryId) -> Option<&Entry> {
        let index = self.entries.binary_search_by_key(&id, |(own, _)| *own).ok()?;
        Some(&self.entries[index].1)
    }

    /// Redraws every tool row whose state isn't the one drawn, and every
    /// permission whose shown tool entry changed (a turn's end, a user
    /// message, a permission answered or added, the policy, a toggle).
    pub(super) fn refresh_rows(&mut self) -> Vec<Splice> {
        self.refresh_facts();
        let stale: Vec<EntryId> = self
            .entries
            .iter()
            .filter(|(id, entry)| {
                let range = self.entry_blocks(*id);
                match (entry, self.blocks.get(range.start).map(|block| &block.body)) {
                    (Entry::Tool { .. }, Some(BlockBody::Tool { state, .. })) => *state != self.tool_state(*id, entry),
                    (Entry::Permission { outcome: None, .. }, Some(BlockBody::Permission { shows, .. })) => {
                        *shows != self.permission_tools.get(id).copied()
                    }
                    (
                        Entry::Tool { .. }
                        | Entry::Permission { .. }
                        | Entry::User { .. }
                        | Entry::Agent { .. }
                        | Entry::Thought { .. }
                        | Entry::Notice { .. },
                        _,
                    ) => false,
                }
            })
            .map(|(id, _)| *id)
            .collect();
        stale.into_iter().filter_map(|id| self.redraw(id)).collect()
    }

    /// The policy from the settings (`[agent]`): the rows it changes, and
    /// only those, are redrawn.
    pub fn set_policy(&mut self, policy: ToolPolicy) -> Vec<Splice> {
        if self.policy == policy {
            return Vec::new();
        }
        self.policy = policy;
        self.refresh_rows()
    }

    /// A press on tool row `id`'s line: opens or closes it, as the user's
    /// choice; nothing for a row with nothing to open, or whose detail a
    /// pending permission shows.
    pub fn toggle_tool(&mut self, id: EntryId) -> Vec<Splice> {
        let Some(entry) = self.held(id) else { return Vec::new() };
        let shown = match self.tool_state(id, entry) {
            RowState::Open => Shown::Closed,
            RowState::Closed => Shown::Open,
            RowState::WithPermission | RowState::Nothing => return Vec::new(),
        };
        self.by_hand.insert(id, shown);
        self.redraw(id).into_iter().collect()
    }

    /// `toggle_agent_tool_details`: opens every tool row with a detail when
    /// any is closed, else closes them all, each as the user's choice.
    pub fn toggle_tool_details(&mut self) -> Vec<Splice> {
        let rows: Vec<(EntryId, RowState)> = self
            .entries
            .iter()
            .filter(|(_, entry)| tool::has_detail(entry))
            .map(|(id, entry)| {
                // As the row would be without a pending permission.
                let state = match entry {
                    Entry::Tool { status, .. } => tool::row_state(
                        entry,
                        self.by_hand.get(id).copied(),
                        self.tool_running(*id, *status),
                        false,
                        self.policy,
                    ),
                    Entry::User { .. }
                    | Entry::Agent { .. }
                    | Entry::Thought { .. }
                    | Entry::Permission { .. }
                    | Entry::Notice { .. } => RowState::Nothing,
                };
                (*id, state)
            })
            .collect();
        let open = rows.iter().any(|(_, state)| *state == RowState::Closed);
        for (id, _) in &rows {
            self.by_hand.insert(*id, if open { Shown::Open } else { Shown::Closed });
        }
        rows.into_iter().filter_map(|(id, _)| self.redraw(id)).collect()
    }

    /// What content item `item` of tool entry `entry` copies.
    pub fn tool_copy_text(&self, entry: EntryId, item: usize) -> Option<String> {
        match self.held(entry)? {
            Entry::Tool { content, .. } => content.get(item).map(tool::copy_text),
            Entry::User { .. } | Entry::Agent { .. } | Entry::Thought { .. } | Entry::Permission { .. } | Entry::Notice { .. } => {
                None
            }
        }
    }

    /// Keeps entry `id`'s rendered parts (`None`: it has none), logging the
    /// renderer's first panic.
    pub(super) fn keep_render(&mut self, id: EntryId, render: Option<(Vec<Part>, bool)>) {
        match render {
            Some((parts, panicked)) => {
                if panicked && !self.render_failed {
                    self.render_failed = true;
                    log::warn!("the Markdown renderer panicked on agent entry {id}; showing its text plain");
                }
                self.rendered.insert(id, parts);
            }
            None => {
                self.rendered.remove(&id);
            }
        }
    }

    /// The blocks entry `id` is drawn as, from its cached parts.
    pub(super) fn draw(&self, id: EntryId, entry: &Entry, thought_open: bool) -> Vec<Block> {
        let draw = self.tool_draw(id, entry);
        blocks_of(id, entry, self.rendered.get(&id).map_or(&[][..], Vec::as_slice), thought_open, draw)
    }

    /// The blocks of entry `id`.
    pub(super) fn entry_blocks(&self, id: EntryId) -> Range<usize> {
        self.blocks.partition_point(|b| b.entry < id)..self.blocks.partition_point(|b| b.entry <= id)
    }

    pub(super) fn thought_open(&self, id: EntryId) -> bool {
        self.toggled.get(&id).copied().unwrap_or(self.auto_open == Some(id))
    }

    /// The thought open on its own: the latest entry, while a turn runs.
    fn wanted_auto_open(&self) -> Option<EntryId> {
        match self.entries.last() {
            Some((id, Entry::Thought { .. })) if running(&self.turn) => Some(*id),
            Some(_) | None => None,
        }
    }

    /// Redraws the thoughts whose openness changed (`all`: every one).
    pub(super) fn update_thoughts(&mut self, all: bool) -> Vec<Splice> {
        let before = self.auto_open;
        self.auto_open = self.wanted_auto_open();
        let ids: Vec<EntryId> = if all {
            self.entries.iter().filter(|(_, e)| matches!(e, Entry::Thought { .. })).map(|(id, _)| *id).collect()
        } else {
            [before, self.auto_open].into_iter().flatten().collect()
        };
        ids.into_iter().filter_map(|id| self.redraw_thought(id)).collect()
    }

    /// Draws thought `id` open or closed as [`Self::thought_open`] says,
    /// when that isn't how it is drawn.
    fn redraw_thought(&mut self, id: EntryId) -> Option<Splice> {
        let open = self.thought_open(id);
        let range = self.entry_blocks(id);
        let block = self.blocks.get_mut(range.start).filter(|_| range.len() == 1)?;
        let BlockBody::Thought { open: drawn, .. } = &mut block.body else { return None };
        if *drawn == open {
            return None;
        }
        *drawn = open;
        Some(Splice { range, count: 1 })
    }

    /// The thought trigger: opens or closes thought `id`.
    pub fn toggle_thought(&mut self, id: EntryId) -> Vec<Splice> {
        let open = !self.thought_open(id);
        self.toggled.insert(id, open);
        self.redraw_thought(id).into_iter().collect()
    }

    /// `toggle_agent_thoughts`: opens every thought when one is closed,
    /// else closes them all.
    pub fn toggle_thoughts(&mut self) -> Vec<Splice> {
        let ids: Vec<EntryId> =
            self.entries.iter().filter(|(_, e)| matches!(e, Entry::Thought { .. })).map(|(id, _)| *id).collect();
        let open = ids.iter().any(|id| !self.thought_open(*id));
        ids.into_iter()
            .filter_map(|id| {
                self.toggled.insert(id, open);
                self.redraw_thought(id)
            })
            .collect()
    }

    /// The block a pending permission request's buttons are drawn in, or
    /// `None` when `entry` is no pending request (OpenSpec change
    /// `agents-zone-focus-and-scroll` decision 4).
    pub fn permission_block(&self, entry: EntryId) -> Option<usize> {
        self.blocks.iter().position(|block| {
            block.entry == entry
                && match &block.body {
                    BlockBody::Permission { outcome, .. } => outcome.is_none(),
                    BlockBody::UserText { .. }
                    | BlockBody::AgentPart { .. }
                    | BlockBody::Thought { .. }
                    | BlockBody::Tool { .. }
                    | BlockBody::ToolDetail(_)
                    | BlockBody::Notice { .. } => false,
                }
        })
    }

    /// `copy_last_agent_message`: the latest agent message's text.
    pub fn last_agent_message(&self) -> Option<&str> {
        self.entries.iter().rev().find_map(|(_, entry)| match entry {
            Entry::Agent { text } => Some(text.as_str()),
            Entry::User { .. } | Entry::Thought { .. } | Entry::Tool { .. } | Entry::Permission { .. } | Entry::Notice { .. } => {
                None
            }
        })
    }

    /// `copy_last_code_block`: the last code block's text, at any depth,
    /// of the latest agent message that has one (thoughts don't count).
    pub fn last_code_block(&self) -> Option<&str> {
        self.entries.iter().rev().filter(|(_, entry)| matches!(entry, Entry::Agent { .. })).find_map(|(id, _)| {
            let parts = self.rendered.get(id)?;
            parts.iter().rev().find_map(|part| markdown::last_code(&part.block)).map(|text| text.as_ref())
        })
    }

    /// An entry's whole text, for a message's copy button.
    pub fn message_text(&self, id: EntryId) -> Option<&str> {
        let index = self.entries.binary_search_by_key(&id, |(own, _)| *own).ok()?;
        match &self.entries[index].1 {
            Entry::Agent { text } | Entry::User { text } => Some(text),
            Entry::Thought { .. } | Entry::Tool { .. } | Entry::Permission { .. } | Entry::Notice { .. } => None,
        }
    }

    /// The text each copy of the conversation the zone holds keeps, in
    /// bytes (OpenSpec change `agents-absorb-cost`, decision 8's table).
    #[cfg(test)]
    pub(super) fn held_text(&self) -> HeldText {
        let rich = |blocks: &[Arc<RichBlock>]| blocks.iter().map(|block| markdown::shown_text(block).len()).sum::<usize>();
        let mut held = HeldText::default();
        for (_, entry) in &self.entries {
            held.entries += match entry {
                Entry::User { text } | Entry::Agent { text } | Entry::Thought { text } => text.len(),
                Entry::Tool { content, .. } => content
                    .iter()
                    .map(|item| match item {
                        ToolItem::Text { text, .. } => text.len(),
                        ToolItem::Output(output) => output.text.len(),
                        ToolItem::Diff(_) => 0,
                    })
                    .sum(),
                Entry::Permission { .. } | Entry::Notice { .. } => 0,
            };
        }
        held.rendered = self.rendered.values().flatten().map(|part| markdown::shown_text(&part.block).len()).sum();
        held.text_renders = self.texts.values().map(|(_, blocks)| rich(blocks)).sum();
        held.text_sources = self.texts.values().map(|(source, _)| source.len()).sum();
        let detail_source = |part: &ShownPart| match &part.part {
            DetailPart::Text { source, .. } => source.len(),
            DetailPart::Diff { .. }
            | DetailPart::Output { .. }
            | DetailPart::Locations { .. }
            | DetailPart::Raw { .. }
            | DetailPart::Mark { .. } => 0,
        };
        for block in &self.blocks {
            match &block.body {
                BlockBody::UserText { text } => held.user_parts += text.len(),
                BlockBody::ToolDetail(part) => held.detail_sources += detail_source(part),
                BlockBody::Permission { detail, .. } => held.detail_sources += detail.iter().map(detail_source).sum::<usize>(),
                BlockBody::AgentPart { .. } | BlockBody::Thought { .. } | BlockBody::Tool { .. } | BlockBody::Notice { .. } => {}
            }
        }
        held
    }
}

/// What [`ChatModel::held_text`] counts: each copy's bytes of text.
#[cfg(test)]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct HeldText {
    /// The zone's copy of the entries (`ChatModel.entries`).
    pub entries: usize,
    /// The shown text of the rendered agent messages and thoughts.
    pub rendered: usize,
    /// User messages' parts, as their blocks hold them.
    pub user_parts: usize,
    /// The shown text of tool text items' renders.
    pub text_renders: usize,
    /// Tool text items' sources kept beside their renders.
    pub text_sources: usize,
    /// Tool text items' sources in drawn detail parts.
    pub detail_sources: usize,
}

#[cfg(test)]
mod tests {
    use crate::agent::transcript::{Event, Exit, FileOp, StopReason};

    use super::super::harness::*;
    use super::super::*;
    use super::*;

    /// Decision 8: a message splits at blank lines and, past 4 KiB, at the
    /// next line break; appending changes only the last part.
    #[test]
    fn a_message_splits_at_blank_lines_and_past_4_kib() {
        let text = "one\nline\n\n\ntwo\n\nthree";
        let cut: Vec<&str> = parts(text, 0).into_iter().map(|r| &text[r]).collect();
        assert_eq!(cut, ["one\nline", "two", "three"]);
        let empty = Range { start: 0, end: 0 };
        assert_eq!(parts("", 0), vec![empty.clone()]);
        assert_eq!(parts("\n\n", 0), vec![empty]);

        let long = format!("{}\nrest\nmore", "x".repeat(BLOCK_LIMIT + 10));
        let cut: Vec<&str> = parts(&long, 0).into_iter().map(|r| &long[r]).collect();
        assert_eq!(cut, ["x".repeat(BLOCK_LIMIT + 10).as_str(), "rest\nmore"]);
        let short = format!("{}\nrest", "x".repeat(BLOCK_LIMIT - 10));
        assert_eq!(parts(&short, 0).len(), 1, "under the limit a line break doesn't cut");

        // Every prefix's parts but its last are the whole text's.
        let whole = format!("a\n\nb{}\nc\n\n\nd", "é".repeat(BLOCK_LIMIT));
        let full = parts(&whole, 0);
        for end in (0..=whole.len()).filter(|end| whole.is_char_boundary(*end)) {
            let prefix = parts(&whole[..end], 0);
            assert_eq!(prefix[..prefix.len() - 1], full[..prefix.len() - 1], "prefix of {end} bytes");
        }
    }

    /// Decision 8: each entry kind's block, a permission's reject kinds
    /// first, keyed by entry and option id.
    #[test]
    fn each_entry_is_drawn_as_its_blocks() {
        let blocks = drawn(EntryId(5), &permission("Run cargo test?"), false);
        let [Block { entry: EntryId(5), part: 0, last: true, body: BlockBody::Permission { options, outcome: None, .. }, .. }] =
            blocks.as_slice()
        else {
            panic!("{blocks:?}")
        };
        let ids: Vec<&str> = options.iter().map(|o| o.id.as_ref()).collect();
        assert_eq!(ids, ["reject-once", "allow-once"]);

        let message = drawn(EntryId(2), &agent("a\n\nb"), false);
        assert_eq!(message.iter().map(|b| (b.part, b.start, b.last)).collect::<Vec<_>>(), [(0, 0, false), (1, 3, true)]);
        assert!(matches!(&message[0].body, BlockBody::AgentPart { .. }));
        let message = drawn(EntryId(2), &user("a\n\nb"), false);
        assert!(matches!(&message[1].body, BlockBody::UserText { text } if text.as_ref() == "b"));
        assert_eq!(drawn(EntryId(1), &user(""), false).len(), 1, "an empty message is one block");
        assert_eq!(drawn(EntryId(1), &agent(""), false).len(), 1, "an empty message is one block");
        let tool = Entry::tool("c".into(), "Read".into(), ToolKind::Read, ToolStatus::Pending);
        assert!(matches!(drawn(EntryId(3), &tool, false)[0].body, BlockBody::Tool { kind: ToolKind::Read, .. }));
        assert!(matches!(drawn(EntryId(4), &thought("t"), true)[0].body, BlockBody::Thought { open: true, .. }));
    }

    /// Decision 8: new entries splice in at the end, a streaming message
    /// redraws only its last block, a trim drops blocks and a replay starts
    /// the list over.
    #[test]
    fn absorbing_changes_splices_only_what_changed() {
        let mut zone = Zone::ready();
        let absorbed = zone.feed(&[added(1, user("hi")), added(2, agent("Hel"))]);
        assert_eq!(absorbed.splices, [Splice { range: 0..0, count: 2 }]);

        let absorbed = zone.feed(&[Event::Appended { id: EntryId(2), text: "lo\n\nthere".into() }]);
        assert_eq!(absorbed.splices, [Splice { range: 1..2, count: 2 }]);
        assert_eq!(texts(&zone.model), ["hi", "Hello", "there"]);
        let absorbed = zone.feed(&[Event::Appended { id: EntryId(2), text: " you".into() }]);
        assert_eq!(absorbed.splices, [Splice { range: 2..3, count: 1 }], "only the last part");
        assert_eq!(texts(&zone.model), ["hi", "Hello", "there you"]);

        let absorbed = zone.feed(&[
            added(3, permission("Run?")),
            Event::Answered { id: EntryId(3), outcome: PermissionOutcome::Cancelled },
        ]);
        assert_eq!(absorbed.splices, [Splice { range: 3..3, count: 1 }], "a new entry's change is in its copy");
        let absorbed = zone.feed(&[Event::Trimmed { through: EntryId(1), dropped: 1 }]);
        assert_eq!(absorbed.splices, [Splice { range: 0..1, count: 0 }]);
        assert_eq!(texts(&zone.model), ["Hello", "there you", "Run?"]);

        let absorbed = zone.replay(&[added(1, user("again")), ready()]);
        assert_eq!(absorbed.splices, [Splice { range: 0..3, count: 1 }]);
        assert_eq!(texts(&zone.model), ["again"]);
    }

    /// Decision 8: a thought is open while it is the latest entry and the
    /// turn runs, closed once another follows, unless the user toggled it.
    #[test]
    fn thoughts_open_while_latest_and_streaming() {
        let mut zone = Zone::ready();
        zone.feed(&[turn(TurnState::Working), added(1, thought("hmm"))]);
        assert!(matches!(zone.model.blocks()[0].body, BlockBody::Thought { open: true, .. }));
        let absorbed = zone.feed(&[added(2, agent("answer"))]);
        assert!(matches!(zone.model.blocks()[0].body, BlockBody::Thought { open: false, .. }));
        assert!(absorbed.splices.contains(&Splice { range: 0..1, count: 1 }));

        assert_eq!(zone.model.toggle_thought(EntryId(1)), [Splice { range: 0..1, count: 1 }]);
        assert!(matches!(zone.model.blocks()[0].body, BlockBody::Thought { open: true, .. }));
        zone.feed(&[added(3, thought("more"))]);
        assert!(matches!(zone.model.blocks()[2].body, BlockBody::Thought { open: true, .. }));
        zone.feed(&[turn(TurnState::Ended { stop: StopReason::EndTurn })]);
        assert!(matches!(zone.model.blocks()[2].body, BlockBody::Thought { open: false, .. }), "closed once the turn ends");
        assert!(matches!(zone.model.blocks()[0].body, BlockBody::Thought { open: true, .. }), "toggled stays");

        assert_eq!(zone.model.toggle_thoughts().len(), 1, "one was closed: all open");
        assert_eq!(zone.model.toggle_thoughts().len(), 2, "all open: all close");
        assert!(zone.model.blocks().iter().all(|b| !matches!(b.body, BlockBody::Thought { open: true, .. })));

        let mut idle = Zone::ready();
        idle.feed(&[added(1, thought("hmm"))]);
        assert!(matches!(idle.model.blocks()[0].body, BlockBody::Thought { open: false, .. }), "no turn running");
    }

    /// OpenSpec change `agents-zone-focus-and-scroll` decision 4: a pending
    /// request names the block its buttons are drawn in, and none once
    /// answered.
    #[test]
    fn a_pending_permission_names_its_block() {
        let mut zone = Zone::ready();
        zone.feed(&[
            turn(TurnState::Working),
            added(1, Entry::User { text: "go".into() }),
            added(2, thought("hmm")),
            added(3, permission("Run cargo test?")),
        ]);
        let ix = zone.model.permission_block(EntryId(3)).expect("pending");
        assert_eq!(zone.model.blocks()[ix].entry, EntryId(3));
        assert!(matches!(zone.model.blocks()[ix].body, BlockBody::Permission { outcome: None, .. }));
        assert_eq!(zone.model.permission_block(EntryId(2)), None, "a thought isn't a request");
        assert_eq!(zone.model.permission_block(EntryId(9)), None, "no such entry");
        zone.feed(&[Event::Answered { id: EntryId(3), outcome: PermissionOutcome::Selected { option: "allow-once".into() } }]);
        assert_eq!(zone.model.permission_block(EntryId(3)), None, "answered");
    }

    /// `agents-zone-attention` decision 2: the working row is inserted after
    /// the blocks when it appears, removed from there when it goes, and left
    /// alone otherwise.
    #[test]
    fn the_working_rows_splice_inserts_removes_or_leaves_it() {
        assert_eq!(working_splice(false, true, 4), Some(Splice { range: 4..4, count: 1 }));
        assert_eq!(working_splice(true, false, 4), Some(Splice { range: 4..5, count: 0 }));
        assert_eq!(working_splice(false, true, 0), Some(Splice { range: 0..0, count: 1 }));
        assert_eq!(working_splice(true, true, 4), None, "its text may change, its place doesn't");
        assert_eq!(working_splice(false, false, 4), None);
    }

    /// OpenSpec change `agents-zone-focus-and-scroll` decision 5: the
    /// working row's insert or removal ends the absorb's splices, at the
    /// block count as the absorb left it; a change of its words alone
    /// splices nothing; the list's items are the blocks and the row.
    #[test]
    fn the_working_rows_splice_follows_the_blocks_splices() {
        let mut zone = Zone::ready();
        assert_eq!(zone.model.item_count(), 0);
        let absorbed = zone.feed(&[added(1, user("hi")), turn(TurnState::Working)]);
        assert_eq!(absorbed.splices, [Splice { range: 0..0, count: 1 }, Splice { range: 1..1, count: 1 }]);
        assert_eq!(zone.model.item_count(), 2, "the block and the row");
        let tool = Entry::tool("c1".into(), "Read main.rs".into(), ToolKind::Read, ToolStatus::InProgress);
        let absorbed = zone.feed(&[added(2, tool)]);
        assert_eq!(absorbed.splices, [Splice { range: 1..1, count: 1 }], "the row's words changed, its place didn't");
        assert_eq!(zone.model.working_row(), Some(WorkingRow::Tool { title: "Read main.rs".into() }));
        assert_eq!(zone.model.item_count(), 3);
        let absorbed = zone.model.ended();
        assert_eq!(absorbed.splices, [Splice { range: 2..3, count: 0 }], "the row goes from after the blocks");
        assert_eq!(zone.model.item_count(), 2);
    }

    /// The copy actions read the zone's own copy.
    #[test]
    fn the_copy_actions_read_the_zones_copy() {
        let mut zone = Zone::ready();
        assert_eq!(zone.model.last_agent_message(), None);
        zone.feed(&[added(1, user("hi")), added(2, agent("one")), added(3, agent("two")), added(4, thought("t"))]);
        assert_eq!(zone.model.last_agent_message(), Some("two"));
        assert_eq!(zone.model.message_text(EntryId(1)), Some("hi"));
        assert_eq!(zone.model.message_text(EntryId(4)), None);
        assert!(!zone.model.cancel(), "no turn");
        zone.feed(&[turn(TurnState::Working)]);
        assert!(zone.model.cancel());
        zone.feed(&[turn(TurnState::Cancelling)]);
        assert!(!zone.model.cancel(), "already cancelling");
    }

    fn appended(id: u64, text: &str) -> Event {
        Event::Appended { id: EntryId(id), text: text.into() }
    }

    const STREAMED: &str = "Intro with [a link](https://example.com) and **bold**.\n\n\
                            ```rust\nfn main() {\n    println!(\"hi\");\n}\n```\n\
                            - one\n- [x] two\n\n> quoted\n\n| a | b |\n| - | - |\n| 1 | 2 |\n\nSee [d] — é.\n\n[d]: https://d.example\n";

    /// Decision 5: whatever the chunks, a streamed message ends with the
    /// parts of its whole text.
    #[test]
    fn streaming_at_every_split_point_ends_with_the_whole_texts_parts() {
        let whole = STREAMED;
        for split in (1..whole.len()).filter(|at| whole.is_char_boundary(*at)) {
            let mut zone = Zone::ready();
            zone.feed(&[added(1, user("hi")), added(2, agent(&whole[..split]))]);
            zone.feed(&[appended(2, &whole[split..])]);
            assert_eq!(parts_of(&zone.model, 2), markdown::render(whole), "split at {split}");
            in_step(&zone.model);
        }
        // Character by character.
        let mut zone = Zone::ready();
        zone.feed(&[added(1, agent(""))]);
        for c in whole.chars() {
            zone.feed(&[appended(1, &c.to_string())]);
        }
        in_step(&zone.model);
        assert_eq!(parts_of(&zone.model, 1), markdown::render(whole));
    }

    /// Decision 5: an unclosed fence is code to the end; its closing fence
    /// and the text after it add a part, the parts above unchanged.
    #[test]
    fn an_unclosed_fence_is_code_and_its_close_adds_a_part() {
        let mut zone = Zone::ready();
        zone.feed(&[added(1, agent("Intro\n\n```rust\nfn a() {}\n"))]);
        let absorbed = zone.feed(&[appended(1, "fn b() {}\n")]);
        assert_eq!(texts(&zone.model), ["Intro", "fn a() {}\nfn b() {}\n"]);
        assert_eq!(markdown::kind_word(&parts_of(&zone.model, 1)[1].block), "code:rust");
        assert_eq!(absorbed.splices, [Splice { range: 1..2, count: 1 }], "the paragraph above stays");
        let intro = parts_of(&zone.model, 1)[0].block.clone();
        let absorbed = zone.feed(&[appended(1, "```\nAfter")]);
        assert_eq!(texts(&zone.model), ["Intro", "fn a() {}\nfn b() {}\n", "After"]);
        assert_eq!(absorbed.splices, [Splice { range: 1..2, count: 2 }], "the code's bubble no longer ends there");
        assert!(Arc::ptr_eq(&intro, &parts_of(&zone.model, 1)[0].block));
        in_step(&zone.model);
    }

    /// Decision 5: a reference definition that arrives late changes the
    /// earlier part that uses it, which is spliced from there.
    #[test]
    fn a_late_reference_definition_splices_from_the_part_it_changes() {
        let mut zone = Zone::ready();
        zone.feed(&[added(1, user("q")), added(2, agent("Lead.\n\nSee [the docs][d].\n\n"))]);
        assert_eq!(texts(&zone.model), ["q", "Lead.", "See [the docs][d]."]);
        let absorbed = zone.feed(&[appended(2, "[d]: https://example.com")]);
        assert_eq!(texts(&zone.model), ["q", "Lead.", "See the docs."], "the definition isn't shown");
        assert_eq!(absorbed.splices, [Splice { range: 2..3, count: 1 }]);
        let RichBlock::Paragraph(inline) = &*parts_of(&zone.model, 2)[1].block else { panic!() };
        assert_eq!(inline.links()[0].target.as_ref(), "https://example.com");
        in_step(&zone.model);
    }

    /// User messages keep step 1's plain parts, cut at blank lines and past
    /// 4 KiB, and only their last part is drawn again.
    #[test]
    fn user_messages_split_as_before() {
        let mut zone = Zone::ready();
        zone.feed(&[added(1, user("**a**\n\nb"))]);
        assert_eq!(texts(&zone.model), ["**a**", "b"], "plain, asterisks kept");
        assert!(zone.model.rendered.is_empty());
        let absorbed = zone.feed(&[appended(1, "c\n\nd")]);
        assert_eq!(texts(&zone.model), ["**a**", "bc", "d"]);
        assert_eq!(absorbed.splices, [Splice { range: 1..2, count: 2 }]);
        let long = format!("{}\nrest", "x".repeat(BLOCK_LIMIT + 10));
        zone.feed(&[added(2, user(&long))]);
        assert_eq!(zone.model.blocks().len(), 5);
    }

    /// Decision 5: a thought is one item holding all its blocks, rendered
    /// again whole as it grows.
    #[test]
    fn a_thought_renders_as_one_block() {
        let mut zone = Zone::ready();
        zone.feed(&[turn(TurnState::Working), added(1, thought("Plan:\n\n- a"))]);
        let absorbed = zone.feed(&[appended(1, "\n- b\n\nDone.")]);
        assert_eq!(absorbed.splices, [Splice { range: 0..1, count: 1 }]);
        let [Block { body: BlockBody::Thought { blocks, open: true }, .. }] = zone.model.blocks() else {
            panic!("{:?}", zone.model.blocks())
        };
        let kinds: Vec<String> = blocks.iter().map(|block| markdown::kind_word(block)).collect();
        assert_eq!(kinds, ["paragraph", "list", "paragraph"]);
        assert_eq!(markdown::shown_text(&blocks[1]), "- a\n- b");
        in_step(&zone.model);
    }

    /// Decision 5: trims and resets keep the cache in step with the
    /// entries.
    #[test]
    fn trims_and_resets_keep_the_cache_in_step() {
        let mut zone = Zone::ready();
        zone.feed(&[added(1, agent("one")), added(2, thought("t")), added(3, user("u")), added(4, agent("four"))]);
        in_step(&zone.model);
        zone.feed(&[Event::Trimmed { through: EntryId(2), dropped: 2 }]);
        assert_eq!(zone.model.rendered.keys().copied().collect::<Vec<_>>(), [EntryId(4)]);
        in_step(&zone.model);
        zone.replay(&[added(1, agent("again *x*")), added(2, thought("hm")), ready()]);
        assert_eq!(zone.model.rendered.keys().copied().collect::<Vec<_>>(), [EntryId(1), EntryId(2)]);
        assert_eq!(texts(&zone.model), ["again x", "hm"]);
        in_step(&zone.model);
    }

    /// Decision 4: a renderer panic shows the source as one plain
    /// paragraph (the walk can't panic, so the guard is checked directly).
    #[test]
    fn the_renderer_runs_guarded() {
        let (parts, panicked) = render_entry(&agent("*a*")).unwrap();
        assert!(!panicked);
        assert_eq!(parts, markdown::render("*a*"));
        assert_eq!(render_entry(&user("*a*")), None);
        let mut model = ChatModel::new("Fake", None);
        let plain = vec![Part { range: 0..3, block: Arc::new(RichBlock::Paragraph(Inline::plain("*a*"))) }];
        model.keep_render(EntryId(1), Some((plain.clone(), true)));
        assert!(model.render_failed);
        assert_eq!(model.rendered[&EntryId(1)], plain);
    }

    /// A Markdown message of at least 256 KiB: prose, lists and a long
    /// fence.
    fn long_markdown() -> String {
        let mut text = String::new();
        let mut section = 0;
        while text.len() < 256 * 1024 {
            section += 1;
            text.push_str(&format!(
                "## Section {section}\n\nSome prose with **bold**, a [link](https://example.com/{section}) and `code`,\n\
                 wrapped onto a second line.\n\n- item one\n- item two\n  - nested {section}\n\n"
            ));
            if section == 20 {
                text.push_str("```rust\n");
                for line in 0..2_000 {
                    text.push_str(&format!("let x{line} = {line};\n"));
                }
                text.push_str("```\n\n");
            }
        }
        text
    }

    /// Decision 6's guard (`testing R8`: the rule, not a time): streaming a
    /// 256 KiB message in 64-byte appends ends with the whole text's parts;
    /// each absorb splices from the first item whose part changed (or
    /// whose bubble's end moved) and touches no other entry; and the parts
    /// it didn't change, and every other entry's, keep their cached values.
    #[test]
    fn streaming_a_long_message_re_renders_only_what_changed() {
        let whole = long_markdown();
        let mut zone = Zone::ready();
        zone.feed(&[added(1, thought("before")), added(2, agent("")), added(3, user("after"))]);
        let thought_before = parts_of(&zone.model, 1);
        let mut at = 0;
        while at < whole.len() {
            let mut end = (at + 64).min(whole.len());
            while !whole.is_char_boundary(end) {
                end += 1;
            }
            let old = parts_of(&zone.model, 2);
            let items = zone.model.entry_blocks(EntryId(2));
            let absorbed = zone.feed(&[appended(2, &whole[at..end])]);
            let new = parts_of(&zone.model, 2);
            // The first item that must change: a part that differs, or the
            // last one when the count changed (its bubble's end moved).
            let differs = old.iter().zip(&new).take_while(|(old, new)| old == new).count();
            let first = if old.len() == new.len() { differs } else { differs.min(old.len() - 1) };
            match absorbed.splices.as_slice() {
                [] => assert_eq!(old, new, "nothing changed at {at}"),
                [splice] => {
                    assert_eq!(splice.range.start, items.start + first, "at {at}");
                    assert_eq!(splice.range.end, items.end, "at {at}: only this entry's items");
                    assert_eq!(splice.count, new.len() - first, "at {at}");
                }
                more => panic!("at {at}: {more:?}"),
            }
            for (old, new) in old.iter().zip(&new).take(differs) {
                assert!(Arc::ptr_eq(&old.block, &new.block), "at {at}: an unchanged part was replaced");
            }
            at = end;
        }
        assert_eq!(parts_of(&zone.model, 2), markdown::render(&whole));
        let after = parts_of(&zone.model, 1);
        assert!(thought_before.iter().zip(&after).all(|(a, b)| Arc::ptr_eq(&a.block, &b.block)), "another entry kept");

        // An absorb that changes no block leaves the cache and the blocks
        // alone: its one splice is the working row's, after them.
        let before = parts_of(&zone.model, 2);
        let absorbed = zone.feed(&[appended(2, ""), turn(TurnState::Working)]);
        let len = zone.model.blocks().len();
        assert_eq!(absorbed.splices, [Splice { range: len..len, count: 1 }]);
        let kept = parts_of(&zone.model, 2);
        assert!(before.len() == kept.len() && before.iter().zip(&kept).all(|(a, b)| Arc::ptr_eq(&a.block, &b.block)));
        in_step(&zone.model);
    }

    /// `copy_last_code_block`: the last code block of the latest agent
    /// message that has one, nested ones included, thoughts skipped.
    #[test]
    fn the_last_code_block() {
        let mut zone = Zone::ready();
        assert_eq!(zone.model.last_code_block(), None);
        zone.feed(&[added(1, agent("no code")), added(2, user("```\nuser\n```"))]);
        assert_eq!(zone.model.last_code_block(), None, "no agent code");
        zone.feed(&[added(3, agent("```\na\n```\n\n```\nb\n```"))]);
        assert_eq!(zone.model.last_code_block(), Some("b\n"));
        zone.feed(&[added(4, agent("- item\n\n  ```\n  nested\n  ```")), added(5, thought("```\nthought\n```"))]);
        assert_eq!(zone.model.last_code_block(), Some("nested\n"), "a thought doesn't count");
        zone.feed(&[added(6, agent("plain again"))]);
        assert_eq!(zone.model.last_code_block(), Some("nested\n"), "the latest message that has one");
    }

    // agents-tool-content 5.3–5.7: tool call details.

    use crate::agent::tool::DiffRow;

    use crate::agent::transcript::{FileChange, Location, Output};

    fn output_item(command: Option<&str>, text: &str, exit: Option<Exit>) -> ToolItem {
        ToolItem::Output(Output { command: command.map(str::to_string), text: text.into(), exit, ..Output::default() })
    }

    fn items(id: u64, from: u32, items: Vec<ToolItem>) -> Event {
        Event::ToolItems { id: EntryId(id), from, items }
    }

    fn out(id: u64, item: u32, text: &str, exit: Option<Exit>) -> Event {
        Event::ToolOutput { id: EntryId(id), item, text: text.into(), exit }
    }

    fn named_permission(call: &str) -> Entry {
        let Entry::Permission { title, options, outcome, .. } = permission("Write a file") else { unreachable!() };
        Entry::Permission { call: Some(call.into()), title, options, outcome }
    }

    /// The detail parts permission `id` shows.
    fn shown_with(model: &ChatModel, id: u64) -> usize {
        let range = model.entry_blocks(EntryId(id));
        let BlockBody::Permission { detail, .. } = &model.blocks()[range.start].body else { panic!() };
        detail.len()
    }

    /// Decision 10: a running call's row is open, a finished one closed,
    /// and the detail is a block per part after the row's.
    #[test]
    fn a_tool_row_opens_while_its_call_runs_and_closes_when_it_finishes() {
        let mut zone = Zone::ready();
        zone.feed(&[added(1, user("go")), turn(TurnState::Working)]);
        let absorbed = zone.feed(&[added(2, in_progress("c1", vec![diff_item("/p/main.rs")]))]);
        assert_eq!(absorbed.splices, [Splice { range: 1..1, count: 2 }], "the row and its diff");
        assert_eq!(row(&zone.model, 2), (RowState::Open, 2));
        let BlockBody::Tool { summary, .. } = &zone.model.blocks()[1].body else { panic!() };
        assert_eq!(*summary, Some("+1 \u{2212}1".into()));
        let BlockBody::ToolDetail(ShownPart { part: DetailPart::Diff { rows, head: Some(head), .. }, .. }) =
            &zone.model.blocks()[2].body
        else {
            panic!("{:?}", zone.model.blocks())
        };
        assert_eq!(head.path.as_ref(), "/p/main.rs");
        assert_eq!(rows[1], DiffRow {
            sign: tool::Sign::Removed,
            old: Some(11),
            new: None,
            text: "    old();".into(),
            hunk_start: false
        });
        assert!(zone.model.blocks()[2].last && !zone.model.blocks()[1].last);

        let absorbed = zone.feed(&[tool_completed(2)]);
        assert_eq!(absorbed.splices, [Splice { range: 1..3, count: 1 }]);
        assert_eq!(row(&zone.model, 2), (RowState::Closed, 1));
        assert_drawn_afresh(&zone.model);

        // One that arrives finished is closed; one left running by a
        // stopped turn closes with it.
        zone.feed(&[added(3, tool_entry("c2", ToolStatus::Completed, vec![diff_item("/p/a.rs")]))]);
        assert_eq!(row(&zone.model, 3), (RowState::Closed, 1));
        zone.feed(&[added(4, in_progress("c3", vec![diff_item("/p/b.rs")]))]);
        assert_eq!(row(&zone.model, 4), (RowState::Open, 2));
        let absorbed = zone.feed(&[turn(TurnState::Ended { stop: StopReason::Cancelled })]);
        assert_eq!(row(&zone.model, 4), (RowState::Closed, 1));
        assert!(absorbed.splices.contains(&Splice { range: 3..5, count: 1 }), "{:?}", absorbed.splices);
        assert_drawn_afresh(&zone.model);

        // Nothing to open: none.
        zone.feed(&[turn(TurnState::Working), added(5, in_progress("c4", Vec::new()))]);
        assert_eq!(row(&zone.model, 5), (RowState::Nothing, 1));
        // A call before the latest user message no longer runs.
        zone.feed(&[added(6, in_progress("c5", vec![diff_item("/p/c.rs")]))]);
        assert_eq!(row(&zone.model, 6), (RowState::Open, 2));
        zone.feed(&[added(7, user("next"))]);
        assert_eq!(row(&zone.model, 6), (RowState::Closed, 1));
        assert_drawn_afresh(&zone.model);
    }

    /// A choice by hand outlives completion and a policy change; a policy
    /// change splices only the rows it changes.
    #[test]
    fn a_choice_by_hand_outlives_completion_and_the_policy() {
        let mut zone = Zone::ready();
        zone.feed(&[added(1, user("go")), turn(TurnState::Working)]);
        zone.feed(&[added(2, in_progress("c1", vec![diff_item("/p/a.rs")]))]);
        zone.feed(&[added(3, in_progress("c2", vec![diff_item("/p/b.rs")]))]);
        assert_eq!(zone.model.toggle_tool(EntryId(2)), [Splice { range: 1..3, count: 1 }], "closed by hand");
        assert_eq!(zone.model.toggle_tool(EntryId(2)), [Splice { range: 1..2, count: 2 }], "opened again");
        zone.feed(&[tool_completed(2), tool_completed(3)]);
        assert_eq!(row(&zone.model, 2), (RowState::Open, 2), "still open: the user opened it");
        assert_eq!(row(&zone.model, 3), (RowState::Closed, 1));

        let all_open = ToolPolicy { running: Shown::Open, done: Shown::Open };
        let splices = zone.model.set_policy(all_open);
        assert_eq!(splices, [Splice { range: 3..4, count: 2 }], "only the row that changes");
        assert_eq!(zone.model.set_policy(all_open), [], "no change, no splice");
        let splices = zone.model.set_policy(ToolPolicy { running: Shown::Closed, done: Shown::Closed });
        assert_eq!(splices, [Splice { range: 3..5, count: 1 }]);
        assert_eq!(row(&zone.model, 2), (RowState::Open, 2), "a choice by hand outlives the policy");
        assert_eq!(zone.model.toggle_tool(EntryId(9)), [], "no such entry");
        assert_drawn_afresh(&zone.model);

        // Toggle Tool Details: one closed, so all open; then all close.
        let splices = zone.model.toggle_tool_details();
        assert_eq!(splices.len(), 1);
        assert_eq!((row(&zone.model, 2).0, row(&zone.model, 3).0), (RowState::Open, RowState::Open));
        zone.model.toggle_tool_details();
        assert_eq!((row(&zone.model, 2).0, row(&zone.model, 3).0), (RowState::Closed, RowState::Closed));
        assert_drawn_afresh(&zone.model);
    }

    /// A call that runs with the zone holding no user message (its turn's
    /// prompt above the page) is running (flag 7); every call of a replay's
    /// earlier turns, and of a fetched page, is finished.
    #[test]
    fn replays_and_pages_draw_earlier_calls_closed_and_a_running_one_open() {
        let mut server = Transcript::new();
        let mut events = vec![ready(), added(1, user("first")), turn(TurnState::Working)];
        events.push(added(2, tool_entry("c1", ToolStatus::InProgress, vec![diff_item("/p/a.rs")])));
        events.push(turn(TurnState::Ended { stop: StopReason::EndTurn }));
        events.push(added(3, user("second")));
        events.push(turn(TurnState::Working));
        events.push(added(4, in_progress("c2", vec![diff_item("/p/b.rs")])));
        for n in 0..20 {
            events.push(added(n + 5, agent(&"y".repeat(20 * 1024))));
        }
        events.push(added(25, in_progress("c3", vec![diff_item("/p/c.rs")])));
        for event in &events {
            server.apply(event).unwrap();
        }
        let mut whole = Zone::new(None);
        whole.replay(&server.replay());
        assert_eq!(row(&whole.model, 2).0, RowState::Closed, "left in progress by an ended turn");
        assert_eq!(row(&whole.model, 4).0, RowState::Open, "the running turn's");
        assert_eq!(row(&whole.model, 25).0, RowState::Open);

        let mut zone = paged(&server);
        assert!(zone.model.held(EntryId(3)).is_none(), "the prompt is above the page");
        assert_eq!(row(&zone.model, 25).0, RowState::Open, "running with no user message held");
        let mut request = 1;
        while let Some(fetch) = zone.model.fetch_due(true, true) {
            zone.model.fetching(request);
            let absorbed = answer_fetch(&mut zone, &server, fetch, request);
            assert_eq!(absorbed.notice, None);
            request += 1;
        }
        assert_eq!(row(&zone.model, 2).0, RowState::Closed);
        assert_eq!(row(&zone.model, 4).0, RowState::Open, "after the second prompt, its turn running");
        assert_eq!(row(&zone.model, 25).0, RowState::Open);
        assert_drawn_afresh(&zone.model);
    }

    /// An open row's growing output splices only its last part; a replaced
    /// content list redraws from the first part that differs; a trim drops
    /// an open entry's parts.
    #[test]
    fn an_open_rows_changes_splice_only_the_parts_that_change() {
        let mut zone = Zone::ready();
        zone.feed(&[added(1, user("go")), turn(TurnState::Working)]);
        zone.feed(&[added(2, in_progress("c1", vec![diff_item("/p/a.rs"), output_item(Some("cargo test"), "", None)]))]);
        assert_eq!(row(&zone.model, 2), (RowState::Open, 3));
        let absorbed = zone.feed(&[out(2, 1, "line\n", None)]);
        assert_eq!(absorbed.splices, [Splice { range: 3..4, count: 1 }], "the output's part alone");
        let many: String = (0..60).map(|n| format!("{n}\n")).collect();
        let absorbed = zone.feed(&[out(2, 1, &many, None)]);
        assert_eq!(absorbed.splices, [Splice { range: 3..4, count: 2 }], "a part full, a new one after it");
        let absorbed = zone.feed(&[out(2, 1, "more\n", None)]);
        assert_eq!(absorbed.splices, [Splice { range: 4..5, count: 1 }], "only the last part");
        let absorbed = zone.feed(&[out(2, 1, "", Some(Exit::Code { code: 1 }))]);
        assert!(absorbed.splices.iter().all(|s| s.range.start >= 1), "{:?}", absorbed.splices);
        let BlockBody::Tool { summary, .. } = &zone.model.blocks()[1].body else { panic!() };
        assert_eq!(*summary, Some("+1 \u{2212}1 · exit 1".into()));
        assert_drawn_afresh(&zone.model);

        // The list replaced from item 1: the line's summary loses the exit,
        // and the parts from the row's on are drawn again.
        let absorbed = zone.feed(&[items(2, 1, vec![ToolItem::Text { text: "**done**".into(), cut: false }])]);
        assert_eq!(absorbed.splices, [Splice { range: 1..5, count: 3 }]);
        // Replaced again from item 1 with the summary unchanged: the row
        // and the diff's part stay.
        let absorbed = zone.feed(&[items(2, 1, vec![ToolItem::Text { text: "**more**".into(), cut: false }])]);
        assert_eq!(absorbed.splices, [Splice { range: 3..4, count: 1 }]);
        let BlockBody::ToolDetail(ShownPart { part: DetailPart::Text { .. }, text, .. }) = &zone.model.blocks()[3].body
        else {
            panic!()
        };
        assert_eq!(text.len(), 1, "rendered");
        assert_drawn_afresh(&zone.model);

        // A trim that drops the open entry drops its parts.
        zone.feed(&[added(3, agent("after"))]);
        let absorbed = zone.feed(&[Event::Trimmed { through: EntryId(2), dropped: 2 }]);
        assert_eq!(absorbed.splices, [Splice { range: 0..4, count: 0 }]);
        assert_eq!(zone.model.blocks().len(), 1);
        assert!(zone.model.texts.is_empty() && zone.model.by_hand.is_empty());
    }

    /// Decision 10: a pending permission shows its call's detail, its row
    /// shows the line alone; answered, the detail goes back to the row.
    #[test]
    fn a_pending_permission_shows_its_calls_detail() {
        for policy in [ToolPolicy::default(), ToolPolicy { running: Shown::Closed, done: Shown::Closed }] {
            let mut zone = Zone::ready();
            zone.model.set_policy(policy);
            zone.feed(&[added(1, user("go")), turn(TurnState::Working)]);
            zone.feed(&[added(2, in_progress("c1", vec![diff_item("/p/main.rs")]))]);
            zone.feed(&[turn(TurnState::Blocked), added(3, named_permission("c1"))]);
            assert_eq!(row(&zone.model, 2), (RowState::WithPermission, 1), "{policy:?}");
            assert_eq!(shown_with(&zone.model, 3), 1);
            let dump = zone.model.describe(1, true, &found(&zone.model), &linux);
            assert!(dump.contains("status=in_progress open=permission items=1 summary=\"+1 \u{2212}1\" \"Edit c1\""), "{dump}");
            assert!(dump.contains("outcome=pending detail=1 \"Write a file\""), "{dump}");
            assert_drawn_afresh(&zone.model);

            // Content that arrives after the permission row shows with it.
            zone.feed(&[items(2, 1, vec![diff_item("/p/lib.rs")])]);
            assert_eq!(shown_with(&zone.model, 3), 2);
            assert_drawn_afresh(&zone.model);

            zone.feed(&[
                Event::Answered { id: EntryId(3), outcome: PermissionOutcome::Selected { option: "allow-once".into() } },
                turn(TurnState::Working),
            ]);
            assert_eq!(shown_with(&zone.model, 3), 0, "answered: no detail");
            let want = if policy.running == Shown::Open { (RowState::Open, 3) } else { (RowState::Closed, 1) };
            assert_eq!(row(&zone.model, 2), want);
            let dump = zone.model.describe(1, true, &found(&zone.model), &linux);
            assert!(dump.contains("outcome=selected:allow-once \"Write a file\""), "{dump}");
            assert_drawn_afresh(&zone.model);
        }

        // Cancelled: no detail either.
        let mut zone = Zone::ready();
        zone.feed(&[turn(TurnState::Working), added(1, in_progress("c1", vec![diff_item("/p/a.rs")]))]);
        zone.feed(&[added(2, named_permission("c1"))]);
        zone.feed(&[Event::Answered { id: EntryId(2), outcome: PermissionOutcome::Cancelled }]);
        assert_eq!(shown_with(&zone.model, 2), 0);
        assert_eq!(row(&zone.model, 1).0, RowState::Open);

        // No call, or a call the model doesn't hold: as before.
        let mut zone = Zone::ready();
        let mut no_call = named_permission("c1");
        if let Entry::Permission { call, .. } = &mut no_call {
            *call = None;
        }
        zone.feed(&[added(1, in_progress("c1", vec![diff_item("/p/a.rs")])), added(2, no_call)]);
        zone.feed(&[added(3, named_permission("elsewhere"))]);
        assert_eq!((shown_with(&zone.model, 2), shown_with(&zone.model, 3)), (0, 0));
        let dump = zone.model.describe(1, true, &found(&zone.model), &linux);
        assert!(dump.contains("outcome=pending \"Write a file\"\n"), "{dump}");
    }

    /// Decision 15: a restored page whose first entry is the permission's
    /// tool call shows its detail in the permission.
    #[test]
    fn a_restored_page_starting_at_the_permissions_call_shows_its_detail() {
        let mut server = Transcript::new();
        let mut events = vec![ready(), added(1, user("go")), turn(TurnState::Working)];
        events.push(added(2, in_progress("c1", vec![diff_item("/p/main.rs")])));
        for n in 0..20 {
            events.push(added(n + 3, agent(&"y".repeat(20 * 1024))));
        }
        events.push(turn(TurnState::Blocked));
        events.push(added(23, named_permission("c1")));
        for event in &events {
            server.apply(event).unwrap();
        }
        let zone = paged(&server);
        assert_eq!(zone.model.entries.first().map(|(id, _)| *id), Some(EntryId(2)));
        assert_eq!(row(&zone.model, 2).0, RowState::WithPermission);
        assert_eq!(shown_with(&zone.model, 23), 1);
    }

    /// Decision 12: the tool line's `open=`, `items=`, `more=` and
    /// `summary=` (flags 31 and 33, each only with one), and its content's
    /// lines, escaped onto one line each.
    #[test]
    fn the_dump_lists_a_tool_calls_content() {
        let mut zone = Zone::ready();
        let mut entry = tool_entry("c1", ToolStatus::Completed, vec![
            diff_item("/p/main.rs"),
            output_item(Some("cargo test"), "ok\n", Some(Exit::Code { code: 0 })),
            output_item(None, "/p\n", Some(Exit::Signal { name: "SIGKILL".into() })),
            ToolItem::Text { text: "a \"quote\"\nb".into(), cut: true },
            ToolItem::Diff(FileChange { op: FileOp::Move { from: "/p/old".into() }, path: "/p/new".into(), hunks: vec![], omitted: 3 }),
        ]);
        if let Entry::Tool { locations, input, more, .. } = &mut entry {
            locations.push(Location { path: "/p/main.rs".into(), line: Some(10) });
            locations.push(Location { path: "/p/lib.rs".into(), line: None });
            *input = Some("{\"x\":1}".into());
            *more = 8;
        }
        zone.feed(&[added(1, entry)]);
        let dump = zone.model.describe(1, true, &found(&zone.model), &linux);
        let lines: Vec<&str> = dump.lines().filter(|line| line.contains("entry=1") || line.starts_with("--- entry 1")).collect();
        assert_eq!(lines, [
            "--- entry 1 tool call=\"c1\" kind=edit status=completed open=no items=5 more=8 summary=\"+1 \u{2212}1 · SIGKILL\" \"Edit c1\"",
            "--- item entry=1 index=0 kind=diff op=modify path=\"/p/main.rs\" added=1 removed=1 omitted=0",
            "--- hunk entry=1 index=0 old=10 new=10 \" fn main() {\\n-    old();\\n+    new();\\n }\"",
            "--- item entry=1 index=1 kind=output command=\"cargo test\" exit=0 omitted=0 \"ok\\n\"",
            "--- item entry=1 index=2 kind=output exit=signal:SIGKILL omitted=0 \"/p\\n\"",
            "--- item entry=1 index=3 kind=text cut=yes \"a \\\"quote\\\"\\nb\"",
            "--- item entry=1 index=4 kind=diff op=move from=\"/p/old\" path=\"/p/new\" added=0 removed=0 omitted=3",
            "--- location entry=1 path=\"/p/main.rs\" line=10",
            "--- location entry=1 path=\"/p/lib.rs\"",
        ], "no raw line beside content");
        assert!(zone.model.wait_text(&found(&zone.model), &linux).contains("kind=diff"));

        // Raw values show with no content items; an empty call is `none`.
        let mut zone = Zone::ready();
        let mut raw = tool_entry("c2", ToolStatus::Pending, Vec::new());
        if let Entry::Tool { input, result, .. } = &mut raw {
            *input = Some("*** Begin Patch".into());
            *result = Some("done".into());
        }
        zone.feed(&[added(1, raw), added(2, tool_entry("c3", ToolStatus::Pending, Vec::new()))]);
        let dump = zone.model.describe(1, true, &found(&zone.model), &linux);
        assert!(dump.contains("--- raw entry=1 which=input \"*** Begin Patch\"\n--- raw entry=1 which=result \"done\"\n"));
        assert!(dump.contains("status=pending open=none items=0 \"Edit c3\""), "{dump}");
    }

    #[test]
    fn copy_text_reads_the_held_entry() {
        let mut zone = Zone::ready();
        zone.feed(&[added(1, tool_entry("c1", ToolStatus::Completed, vec![output_item(None, "ok\n", None)]))]);
        assert_eq!(zone.model.tool_copy_text(EntryId(1), 0).as_deref(), Some("ok\n"));
        assert_eq!(zone.model.tool_copy_text(EntryId(1), 1), None);
        assert_eq!(zone.model.tool_copy_text(EntryId(2), 0), None);
    }
}
