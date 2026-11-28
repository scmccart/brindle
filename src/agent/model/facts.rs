//! What the zone knows of its entries beyond each one (OpenSpec change
//! `agents-absorb-cost`, decision 7): kept as copies are applied rather
//! than read again from every entry on every change, and the rows a change
//! can move.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use gpui::SharedString;

use crate::agent::tool;
use crate::agent::transcript::{Entry, EntryId};

use super::blocks::buttons;

/// A pending permission request: its buttons' option ids, in the order
/// they are drawn (reject kinds first), and the tool entry it shows (the
/// newest held with the call it names, before it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Pending {
    pub buttons: Vec<SharedString>,
    pub tool: Option<EntryId>,
}

/// The facts the rows, the focus stops and the notifications read.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Facts {
    /// The latest user message held: a tool call after it can be running.
    pub last_user: Option<EntryId>,
    /// The newest tool entry held for each call id.
    pub newest_tool: HashMap<String, EntryId>,
    /// The pending permission requests, in id order.
    pub pending: BTreeMap<EntryId, Pending>,
    /// Tool entries whose status is pending or in progress: the rows a
    /// turn's or a user message's change can move.
    pub unfinished: BTreeSet<EntryId>,
}

impl Facts {
    /// The facts of `entries`, in id order, read whole: on a reset or a
    /// prepended page, and what the tests compare the kept facts with.
    pub fn read<'a>(entries: impl IntoIterator<Item = &'a (EntryId, Entry)>) -> Self {
        let mut facts = Self::default();
        for (id, entry) in entries {
            facts.added(*id, entry);
        }
        facts
    }

    /// Entry `id`, newer than every held one, was added.
    pub fn added(&mut self, id: EntryId, entry: &Entry) {
        match entry {
            Entry::User { .. } => self.last_user = Some(id),
            Entry::Tool { call, .. } => {
                self.newest_tool.insert(call.clone(), id);
            }
            Entry::Permission { call, outcome: None, options, .. } => {
                let tool = call.as_ref().and_then(|call| self.newest_tool.get(call)).copied();
                self.pending.insert(id, Pending { buttons: buttons(options).into_iter().map(|b| b.id).collect(), tool });
            }
            Entry::Permission { outcome: Some(_), .. } | Entry::Agent { .. } | Entry::Thought { .. } | Entry::Notice { .. } => {}
        }
        self.status(id, entry);
    }

    /// Held entry `id` was replaced by `entry`: its call and kind stay, its
    /// status or outcome may not.
    pub fn replaced(&mut self, id: EntryId, entry: &Entry) {
        match entry {
            Entry::Permission { outcome: None, options, .. } => {
                let buttons: Vec<SharedString> = buttons(options).into_iter().map(|b| b.id).collect();
                match self.pending.get_mut(&id) {
                    Some(pending) => pending.buttons = buttons,
                    // An answer taken back never happens; read as new.
                    None => {
                        self.added_pending(id, entry);
                    }
                }
            }
            Entry::Permission { outcome: Some(_), .. } => {
                self.pending.remove(&id);
            }
            Entry::User { .. } | Entry::Agent { .. } | Entry::Thought { .. } | Entry::Tool { .. } | Entry::Notice { .. } => {}
        }
        self.status(id, entry);
    }

    fn added_pending(&mut self, id: EntryId, entry: &Entry) {
        let Entry::Permission { call, options, .. } = entry else { return };
        // The newest tool before it: one after it can't be shown.
        let tool = call.as_ref().and_then(|call| self.newest_tool.get(call)).copied().filter(|tool| *tool < id);
        self.pending.insert(id, Pending { buttons: buttons(options).into_iter().map(|b| b.id).collect(), tool });
    }

    fn status(&mut self, id: EntryId, entry: &Entry) {
        match entry {
            Entry::Tool { status, .. } if tool::unfinished(*status) => {
                self.unfinished.insert(id);
            }
            Entry::Tool { .. }
            | Entry::User { .. }
            | Entry::Agent { .. }
            | Entry::Thought { .. }
            | Entry::Permission { .. }
            | Entry::Notice { .. } => {
                self.unfinished.remove(&id);
            }
        }
    }

    /// The entries through `through` were trimmed: the rows whose shown
    /// tool or permission went with them.
    pub fn trimmed(&mut self, through: EntryId) -> BTreeSet<EntryId> {
        let mut moved = BTreeSet::new();
        if self.last_user.is_some_and(|user| user <= through) {
            self.last_user = None;
        }
        self.newest_tool.retain(|_, id| *id > through);
        self.unfinished.retain(|id| *id > through);
        let gone: Vec<EntryId> = self.pending.range(..=through).map(|(id, _)| *id).collect();
        for id in gone {
            if let Some(tool) = self.pending.remove(&id).and_then(|pending| pending.tool) {
                moved.insert(tool);
            }
        }
        for (id, pending) in &mut self.pending {
            if pending.tool.is_some_and(|tool| tool <= through) {
                pending.tool = None;
                moved.insert(*id);
            }
        }
        moved
    }

    /// Whether a pending permission shows tool entry `id`.
    pub fn shown_by_permission(&self, id: EntryId) -> bool {
        self.pending.values().any(|pending| pending.tool == Some(id))
    }

    /// The pending permissions that show tool entry `id`, in id order:
    /// several can name one call.
    pub fn showing(&self, id: EntryId) -> Vec<EntryId> {
        self.pending.iter().filter(|(_, pending)| pending.tool == Some(id)).map(|(permission, _)| *permission).collect()
    }
}

/// The rows a draw checks (decision 7): every one, or those a change can
/// have moved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Rows {
    All,
    Only(BTreeSet<EntryId>),
}

impl Default for Rows {
    fn default() -> Self {
        Self::Only(BTreeSet::new())
    }
}

impl Rows {
    pub fn add(&mut self, id: EntryId) {
        match self {
            Self::All => {}
            Self::Only(ids) => {
                ids.insert(id);
            }
        }
    }

    pub fn extend(&mut self, ids: impl IntoIterator<Item = EntryId>) {
        for id in ids {
            self.add(id);
        }
    }
}
