use std::collections::BTreeMap;

use brindle_transcript::{
    ContentBudget, Entry, EntryId, Event, Exit, FileChange, FileOp, Location, MAX_LOCATIONS,
    NoticeLevel, Output as ToolOutputItem, SIGNAL_RESERVE, ToolItem, ToolKind, ToolStatus, chunks,
    clean, item_pieces,
};

use crate::acp::forms::{
    Content, ContentKind, OutputForm, Patch, ToolCall, ToolChange, ToolDetail, ToolItemForm,
};
use crate::acp::limits::MAX_TERMINAL_ID;
use crate::acp::{diff, plain};

use super::Step;
use super::text::{bound, cut_at, path_text, raw_text, text_item};

/// The most outputs a session still appends to (`agents-tool-content`
/// decision 5): past it the oldest is closed, its text kept.
pub const MAX_OPEN_OUTPUTS: usize = 64;

/// An output still appended to: its tool entry, its item there, and the
/// plain-text filter between pieces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenOutput {
    pub entry: EntryId,
    pub item: u32,
    pub filter: plain::State,
}

/// What a session knows of its agent's tool calls by the agent's ids: the
/// tool entries by call id and the outputs still appended to by terminal
/// id. The two are trimmed and reset together.
#[derive(Debug, Clone, PartialEq, Default)]
pub(in crate::acp) struct ToolBook {
    /// The tool entries by the agent's call id; trimmed entries leave it.
    pub(in crate::acp) tools: BTreeMap<String, EntryId>,
    /// The outputs still appended to, by the agent's terminal id
    /// (`agents-tool-content` decision 6); at most [`MAX_OPEN_OUTPUTS`].
    pub(in crate::acp) outputs: BTreeMap<String, OpenOutput>,
}

impl ToolBook {
    /// Forgets the calls and outputs whose entries are `through` or
    /// earlier, as the log's trim drops them.
    pub(super) fn forget_through(&mut self, through: EntryId) {
        self.tools.retain(|_, entry| *entry > through);
        self.outputs.retain(|_, open| open.entry > through);
    }
}

impl Step<'_> {
    /// The tool entry for the agent's call id, when the log still has it.
    pub(super) fn tool(&self, call: &str) -> Option<EntryId> {
        let id = *self.s.process.book.tools.get(call)?;
        self.s.transcript.entry(id).map(|_| id)
    }

    pub(super) fn tool_call(&mut self, call: ToolCall) {
        let id = bound(clean(&call.call));
        let title = bound(clean(&call.title));
        let entry = match self.tool(&id) {
            // An agent may send `tool_call` again for a call it already
            // named: its fields change the entry.
            Some(entry) => {
                self.change_tool(entry, Some(title), Some(call.kind), Some(call.status));
                entry
            }
            None => self.add_tool(id, title, call.kind, call.status),
        };
        self.tool_detail(entry, call.detail);
    }

    pub(super) fn tool_change(&mut self, change: ToolChange) {
        let id = bound(clean(&change.call));
        // A cleared field goes back to what a call with none has: its id
        // for a title, `other`, `pending`.
        let title = change
            .title
            .map(|title| bound(clean(&title)))
            .or_reset(|| id.clone());
        let kind = change.kind.or_reset(|| ToolKind::Other);
        let status = change.status.or_reset(|| ToolStatus::Pending);
        let entry = match self.tool(&id) {
            Some(entry) => {
                self.change_tool(entry, title, kind, status);
                entry
            }
            // An update for a call the log doesn't have adds it with the
            // fields it carries.
            None => {
                let title = title.unwrap_or_else(|| id.clone());
                self.add_tool(
                    id,
                    title,
                    kind.unwrap_or(ToolKind::Other),
                    status.unwrap_or(ToolStatus::Pending),
                )
            }
        };
        self.tool_detail(entry, change.detail);
    }

    pub(super) fn add_tool(
        &mut self,
        call: String,
        title: String,
        kind: ToolKind,
        status: ToolStatus,
    ) -> EntryId {
        let entry = self.add(Entry::tool(call.clone(), title, kind, status));
        self.s.process.book.tools.insert(call, entry);
        entry
    }

    /// Sends the fields that differ from the entry's, in whatever order
    /// they come (statuses can go backwards); nothing when none differ.
    fn change_tool(
        &mut self,
        entry: EntryId,
        title: Option<String>,
        kind: Option<ToolKind>,
        status: Option<ToolStatus>,
    ) {
        self.change_tool_fields(entry, title, kind, status, ToolFields::default());
    }

    fn change_tool_fields(
        &mut self,
        entry: EntryId,
        title: Option<String>,
        kind: Option<ToolKind>,
        status: Option<ToolStatus>,
        fields: ToolFields,
    ) {
        let Some(Entry::Tool {
            title: own_title,
            kind: own_kind,
            status: own_status,
            locations: own_locations,
            input: own_input,
            result: own_result,
            more: own_more,
            ..
        }) = self.s.transcript.entry(entry)
        else {
            return;
        };
        let title = title.filter(|title| title != own_title);
        let kind = kind.filter(|kind| kind != own_kind);
        let status = status.filter(|status| status != own_status);
        let locations = fields.locations.filter(|list| list != own_locations);
        let input = fields.input.filter(|input| input != own_input);
        let result = fields.result.filter(|result| result != own_result);
        let more = fields.more.filter(|more| more != own_more);
        if title.is_none()
            && kind.is_none()
            && status.is_none()
            && locations.is_none()
            && input.is_none()
            && result.is_none()
            && more.is_none()
        {
            return;
        }
        self.emit(Event::ToolChanged {
            id: entry,
            title,
            kind,
            status,
            locations,
            input,
            result,
            more,
        });
    }

    /// What a tool call carries beyond its row (`agents-tool-content`
    /// decisions 3 to 6), applied to `entry` with every bound before an
    /// event is made: its content, which replaces the entry's only when it
    /// differs; its locations and raw values; and its terminals' output.
    pub(super) fn tool_detail(&mut self, entry: EntryId, mut detail: ToolDetail) {
        // A terminal id past `MAX_TERMINAL_ID` is ignored: a cut one could
        // merge two terminals (`agents-event-bounds` decision 3).
        detail.outputs.retain(|form| match form {
            OutputForm::Opened { terminal, .. }
            | OutputForm::Appended { terminal, .. }
            | OutputForm::Exited { terminal, .. } => terminal.len() <= MAX_TERMINAL_ID,
        });
        let opened: BTreeMap<String, (Option<String>, Option<String>)> = detail
            .outputs
            .iter()
            .filter_map(|form| match form {
                OutputForm::Opened {
                    terminal,
                    cwd,
                    command,
                } => Some((
                    terminal.clone(),
                    (
                        cwd.as_deref().map(path_text),
                        command.as_ref().map(|command| bound(clean(command))),
                    ),
                )),
                OutputForm::Appended { .. } | OutputForm::Exited { .. } => None,
            })
            .collect();
        let mut fields = ToolFields::default();
        if let Patch::Set(items) = detail.content {
            fields.more = self.tool_content(entry, items, &opened, detail.unreadable);
        }
        fields.locations = detail.locations.value().map(|list| {
            list.iter()
                .take(MAX_LOCATIONS)
                .map(|location| Location {
                    path: path_text(&location.path),
                    line: location.line,
                })
                .collect()
        });
        fields.input = detail.input.value().map(|text| Some(raw_text(text)));
        fields.result = detail.result.value().map(|text| Some(raw_text(text)));
        self.change_tool_fields(entry, None, None, None, fields);
        for form in detail.outputs {
            match form {
                OutputForm::Opened {
                    terminal,
                    cwd,
                    command,
                } => {
                    let command = command.map(|command| bound(clean(&command)));
                    let cwd = cwd.as_deref().map(path_text);
                    match self.s.process.book.outputs.get(&terminal) {
                        None => {
                            let output = ToolOutputItem {
                                command,
                                cwd,
                                ..ToolOutputItem::default()
                            };
                            self.open_output(entry, terminal, output);
                        }
                        Some(open) => {
                            let (entry, item) = (open.entry, open.item);
                            self.describe_output(entry, item, cwd, command);
                        }
                    }
                }
                OutputForm::Appended { terminal, bytes } => {
                    if let Some(open) = self.output(entry, &terminal) {
                        let text = plain::feed(&mut open.filter, &bytes);
                        let (id, item) = (open.entry, open.item);
                        // In `TEXT_CHUNK` pieces, so every event fits a
                        // frame however much one delta carries; they fold
                        // as the whole text would.
                        for piece in chunks(&text).filter(|piece| !piece.is_empty()) {
                            self.emit(Event::ToolOutput {
                                id,
                                item,
                                text: piece.to_owned(),
                                exit: None,
                            });
                        }
                    }
                }
                OutputForm::Exited { terminal, exit } => {
                    if self.output(entry, &terminal).is_some()
                        && let Some(mut open) = self.s.process.book.outputs.remove(&terminal)
                    {
                        let text = plain::finish(&mut open.filter);
                        self.emit(Event::ToolOutput {
                            id: open.entry,
                            item: open.item,
                            text,
                            exit: exit.map(|exit| match exit {
                                Exit::Code { code } => Exit::Code { code },
                                Exit::Signal { name } => Exit::Signal {
                                    name: cut_at(clean(&name), SIGNAL_RESERVE).0,
                                },
                            }),
                        });
                    }
                }
            }
        }
    }

    /// The open output for `terminal`; one opens at the end of `entry`'s
    /// content when none is (Codex's file read sends output for a terminal
    /// nothing opened, flag 16). `None` when the entry has no room.
    fn output(&mut self, entry: EntryId, terminal: &str) -> Option<&mut OpenOutput> {
        if !self.s.process.book.outputs.contains_key(terminal) {
            self.open_output(entry, terminal.to_owned(), ToolOutputItem::default());
        }
        self.s.process.book.outputs.get_mut(terminal)
    }

    /// A terminal's description arriving after its output opened (a
    /// `[terminal]` item named it first): the output, when it is the
    /// entry's last item and has neither a command nor a directory, is
    /// sent again with them, its text kept.
    fn describe_output(
        &mut self,
        entry: EntryId,
        item: u32,
        cwd: Option<String>,
        command: Option<String>,
    ) {
        if cwd.is_none() && command.is_none() {
            return;
        }
        let Some(Entry::Tool { content, .. }) = self.s.transcript.entry(entry) else {
            return;
        };
        let last = content.len().checked_sub(1) == Some(item as usize);
        let Some(ToolItem::Output(output)) = content.get(item as usize) else {
            return;
        };
        if !last || output.cwd.is_some() || output.command.is_some() {
            return;
        }
        let output = ToolItem::Output(ToolOutputItem {
            cwd,
            command,
            ..output.clone()
        });
        // Only when the described output still fits the call's budget
        // beside the items before it (`agents-event-bounds` decision 3).
        if !ContentBudget::holding(&content[..item as usize]).fits(&output) {
            return;
        }
        self.emit(Event::ToolItems {
            id: entry,
            from: item,
            items: vec![output],
        });
    }

    /// Appends `output` to `entry`'s content and keeps it open for
    /// `terminal`, unless it doesn't fit the call's [`ContentBudget`]
    /// beside the entry's items ([`MAX_ITEMS`], [`CALL_LIMIT`]).
    fn open_output(&mut self, entry: EntryId, terminal: String, output: ToolOutputItem) {
        let Some(Entry::Tool { content, .. }) = self.s.transcript.entry(entry) else {
            return;
        };
        let at = content.len();
        let output = ToolItem::Output(output);
        if !ContentBudget::holding(content).fits(&output) {
            return;
        }
        let item = u32::try_from(at).unwrap_or(u32::MAX);
        self.emit(Event::ToolItems {
            id: entry,
            from: item,
            items: vec![output],
        });
        self.keep_open(
            terminal,
            OpenOutput {
                entry,
                item,
                filter: plain::State::default(),
            },
        );
    }

    /// Keeps an output open, closing the oldest past [`MAX_OPEN_OUTPUTS`].
    fn keep_open(&mut self, terminal: String, open: OpenOutput) {
        while self.s.process.book.outputs.len() >= MAX_OPEN_OUTPUTS {
            let oldest = self
                .s
                .process
                .book
                .outputs
                .iter()
                .min_by_key(|(_, open)| (open.entry, open.item))
                .map(|(terminal, _)| terminal.clone());
            let Some(oldest) = oldest else {
                break;
            };
            self.s.process.book.outputs.remove(&oldest);
        }
        self.s.process.book.outputs.insert(terminal, open);
    }

    /// Replaces `entry`'s content with `items`, bounded, when they differ,
    /// and returns how many items were left out past [`MAX_ITEMS`] and
    /// [`CALL_LIMIT`]. A terminal still named keeps its output as it is.
    fn tool_content(
        &mut self,
        entry: EntryId,
        items: Vec<ToolItemForm>,
        opened: &BTreeMap<String, (Option<String>, Option<String>)>,
        unreadable: u32,
    ) -> Option<u32> {
        let Some(Entry::Tool { content, title, .. }) = self.s.transcript.entry(entry) else {
            return None;
        };
        let (old, title) = (content.clone(), title.clone());
        let mut kept: Vec<ToolItem> = Vec::new();
        let mut terminals: Vec<(String, u32)> = Vec::new();
        let mut budget = ContentBudget::new();
        for form in items {
            let item = match form {
                ToolItemForm::Content(Content::Text(text)) => text_item(&text),
                ToolItemForm::Content(Content::Other(kind)) => text_item(placeholder(kind)),
                ToolItemForm::Unknown { kind } => {
                    text_item(&format!("[content Brindle doesn't show: {kind}]"))
                }
                ToolItemForm::RawDiff { path, old, new } => {
                    let diffed = diff::hunks(old.as_deref().unwrap_or_default(), &new);
                    ToolItem::Diff(FileChange {
                        op: match old {
                            None => FileOp::Add,
                            Some(_) => FileOp::Modify,
                        },
                        path: path_text(&path),
                        // Cleaned as `diff::hunks` keeps them.
                        hunks: diffed.hunks,
                        omitted: diffed.omitted,
                    })
                }
                ToolItemForm::Terminal { terminal } if terminal.len() > MAX_TERMINAL_ID => {
                    // Ignored, as its output is (`tool_detail`), and
                    // counted.
                    budget.leave_out();
                    continue;
                }
                ToolItemForm::Terminal { terminal } => {
                    let open = self
                        .s
                        .process
                        .book
                        .outputs
                        .get(&terminal)
                        .filter(|open| open.entry == entry)
                        .and_then(|open| old.get(open.item as usize));
                    let output = match open {
                        Some(item @ ToolItem::Output(_)) => item.clone(),
                        Some(ToolItem::Text { .. } | ToolItem::Diff(_)) | None => {
                            let (cwd, command) = opened.get(&terminal).cloned().unwrap_or_default();
                            ToolItem::Output(ToolOutputItem {
                                command,
                                cwd,
                                ..ToolOutputItem::default()
                            })
                        }
                    };
                    let at = u32::try_from(kept.len()).unwrap_or(u32::MAX);
                    terminals.push((terminal, at));
                    output
                }
            };
            if !budget.take(&item) {
                if let Some((_, at)) = terminals.last()
                    && *at as usize == kept.len()
                {
                    terminals.pop();
                }
                continue;
            }
            kept.push(item);
        }
        let more = budget.left_out();
        // The outputs of this entry the new list no longer names close.
        let named: Vec<&str> = terminals
            .iter()
            .map(|(terminal, _)| terminal.as_str())
            .collect();
        self.s
            .process
            .book
            .outputs
            .retain(|terminal, open| open.entry != entry || named.contains(&terminal.as_str()));
        let from = old
            .iter()
            .zip(&kept)
            .take_while(|(old, new)| old == new)
            .count();
        if from == kept.len() && from == old.len() {
            return Some(more);
        }
        // In the pieces a replay sends too (`item_pieces`); a list that
        // only shrinks is one empty piece.
        let changed = kept.get(from..).unwrap_or_default();
        let mut pieces = item_pieces(changed);
        if pieces.is_empty() {
            pieces.push(changed);
        }
        let mut at = from;
        for items in pieces {
            self.emit(Event::ToolItems {
                id: entry,
                from: u32::try_from(at).unwrap_or(u32::MAX),
                items: items.to_vec(),
            });
            at += items.len();
        }
        for (terminal, item) in terminals {
            match self.s.process.book.outputs.get_mut(&terminal) {
                Some(open) if open.entry == entry => open.item = item,
                Some(_) => {}
                None => self.keep_open(
                    terminal,
                    OpenOutput {
                        entry,
                        item,
                        filter: plain::State::default(),
                    },
                ),
            }
        }
        if unreadable > 0 {
            self.notice(
                NoticeLevel::Warning,
                format!("{unreadable} content items of tool call {title} couldn't be read"),
            );
        }
        Some(more)
    }
}

/// What a chunk that isn't text shows.
pub(super) fn placeholder(kind: ContentKind) -> &'static str {
    match kind {
        ContentKind::Image => "[image]",
        ContentKind::Audio => "[audio]",
        ContentKind::ResourceLink => "[link]",
        ContentKind::Resource => "[resource]",
        ContentKind::Unknown => "[content Brindle can't show]",
    }
}

/// A tool entry's fields beyond its row that an update changes; `None`
/// leaves one as it is.
#[derive(Default)]
struct ToolFields {
    locations: Option<Vec<Location>>,
    input: Option<Option<String>>,
    result: Option<Option<String>>,
    more: Option<u32>,
}

#[cfg(test)]
mod tests {
    use brindle_remote_protocol::serde_json::json;
    use brindle_transcript::{DiffLine, ITEM_LIMIT, MAX_ITEMS, RAW_LIMIT, TEXT_CHUNK};

    use crate::acp::dialect::Dialect;

    use super::super::harness::*;
    use super::super::*;
    use super::*;

    /// `agents-tool-content` decision 2: a version that can clear a tool
    /// call's field (v1 can't) puts it back to what a call with none has.
    #[test]
    fn a_cleared_tool_field_goes_back_to_what_a_call_with_none_has() {
        let mut agent = Agent::ready();
        agent.feed(&tool_call("c1", "Edit notes.txt", "in_progress"));
        let (entry, _) = agent.last_entry();
        let change = |title, kind, status| ToolChange {
            call: "c1".into(),
            title,
            kind,
            status,
            detail: forms::ToolDetail::default(),
        };
        let unknown = ToolChange {
            call: "c2".into(),
            ..change(Patch::Clear, Patch::Set(ToolKind::Read), Patch::Clear)
        };
        let mut step = Step {
            s: &mut agent.s,
            out: Output::default(),
            fault: None,
            now: agent.now,
            event_limit: MAX_EVENT,
        };
        step.tool_change(change(Patch::Keep, Patch::Keep, Patch::Keep));
        step.tool_change(change(Patch::Clear, Patch::Clear, Patch::Clear));
        // Cleared again, nothing differs.
        step.tool_change(change(Patch::Clear, Patch::Clear, Patch::Clear));
        // A call the log doesn't have, cleared, is added as one with none.
        step.tool_change(unknown);
        let events = step.out.events;
        assert_eq!(events.len(), 2, "{events:?}");
        assert_eq!(
            events[0],
            Event::ToolChanged {
                id: entry,
                title: Some("c1".into()),
                kind: Some(ToolKind::Other),
                status: Some(ToolStatus::Pending),
                locations: None,
                input: None,
                result: None,
                more: None,
            }
        );
        assert!(
            matches!(&events[1], Event::Added { entry: Entry::Tool { call, title, kind: ToolKind::Read, status: ToolStatus::Pending, .. }, .. } if call == "c2" && title == "c2"),
            "{events:?}"
        );
    }

    #[test]
    fn v1_never_clears_a_tool_field() {
        let update = |fields: Value| {
            let mut update = json!({"sessionUpdate": "tool_call_update", "toolCallId": "c1"});
            if let (Some(update), Some(fields)) = (update.as_object_mut(), fields.as_object()) {
                update.extend(fields.clone());
            }
            Dialect::newest().session_update(&json!({"sessionId": "s1", "update": update}))
        };
        for fields in [
            json!({}),
            json!({"title": null, "kind": null, "status": null}),
            json!({"title": "T", "kind": "read", "status": "completed"}),
        ] {
            let Ok(forms::SessionUpdate {
                update: Update::ToolChanged(change),
                ..
            }) = update(fields.clone())
            else {
                panic!("{fields}: didn't decode");
            };
            for cleared in [
                change.title == Patch::Clear,
                change.kind == Patch::Clear,
                change.status == Patch::Clear,
            ] {
                assert!(!cleared, "{fields}: {change:?}");
            }
        }
    }


    #[test]
    fn a_diff_becomes_a_file_change_and_a_resent_list_emits_nothing() {
        let mut agent = Agent::ready();
        agent.turn("edit");
        agent.feed(&tool_call("c1", "Edit", "in_progress"));
        let content = json!([diff_item(
            "/w/notes.txt",
            "a\nan apple a day\nb\n",
            "a\nan banana a day\nb\n"
        )]);
        let resend = || {
            update(
                json!({"sessionUpdate": "tool_call_update", "toolCallId": "c1", "content": content}),
            )
        };
        let out = agent.feed(&resend());
        assert_eq!(shape(&out.events), ["tool items"]);
        let out = agent.feed(&resend());
        assert!(out.events.is_empty(), "{:?}", out.events);
        let Ok::<[ToolItem; 1], _>([ToolItem::Diff(change)]) =
            content_of(&tool_entry(&agent, "c1")).try_into()
        else {
            panic!("not a diff");
        };
        assert_eq!(
            (change.op, change.path.as_str()),
            (FileOp::Modify, "/w/notes.txt")
        );
        assert_eq!(
            change.hunks[0].lines,
            [
                DiffLine::Context("a".into()),
                DiffLine::Removed("an apple a day".into()),
                DiffLine::Added("an banana a day".into()),
                DiffLine::Context("b".into()),
            ]
        );
        // No old text is a new file.
        agent.feed(&update(
            json!({"sessionUpdate": "tool_call_update", "toolCallId": "c1",
            "content": [{"type": "diff", "path": "/w/new.txt", "newText": "x\n"}]}),
        ));
        assert!(matches!(
            &content_of(&tool_entry(&agent, "c1"))[..],
            [ToolItem::Diff(FileChange {
                op: FileOp::Add,
                ..
            })]
        ));
    }

    #[test]
    fn a_permissions_tool_call_brings_its_content_but_never_its_title() {
        let mut agent = Agent::ready();
        agent.turn("edit");
        agent.feed(&tool_call("c1", "Edit notes.txt", "pending"));
        let request = json!({"jsonrpc": "2.0", "id": 0, "method": "session/request_permission",
            "params": {"sessionId": "s1", "options": [{"optionId": "a", "name": "Allow", "kind": "allow_once"}],
                "toolCall": {"toolCallId": "c1", "title": "Another title", "status": "completed",
                    "rawInput": {"path": "notes.txt"}, "rawOutput": "never",
                    "content": [diff_item("notes.txt", "apple\n", "banana\n")]}}});
        let out = agent.feed(&request.to_string());
        assert_eq!(
            shape(&out.events),
            [
                "tool items",
                "tool changed",
                "added permission",
                "turn blocked"
            ]
        );
        let Entry::Tool {
            title,
            status,
            input,
            result,
            content,
            ..
        } = tool_entry(&agent, "c1")
        else {
            panic!("not a tool");
        };
        assert_eq!(
            (title.as_str(), status),
            ("Edit notes.txt", ToolStatus::Pending)
        );
        assert_eq!(
            (input.as_deref(), result),
            (Some("{\n  \"path\": \"notes.txt\"\n}"), None)
        );
        assert_eq!(content.len(), 1);
        // The permission's row names the call's own title.
        assert!(
            matches!(agent.last_permission().1, Entry::Permission { title, .. } if title == "Another title")
        );
    }


    #[test]
    fn codexs_meta_output_ends_as_one_output_with_its_exit() {
        let mut agent = Agent::ready();
        agent.turn("pwd");
        agent.feed(&update(json!({"sessionUpdate": "tool_call", "toolCallId": "c1", "title": "pwd",
            "kind": "execute", "status": "in_progress", "content": [{"type": "terminal", "terminalId": "t1"}],
            "_meta": {"terminal_info": {"terminal_id": "t1", "cwd": "/home/user/scratch"}}})));
        agent.feed(&meta(
            "c1",
            json!({"terminal_output_delta": {"terminal_id": "t1", "data": "/home/user/"}}),
        ));
        let out = agent.feed(&meta(
            "c1",
            json!({
            "terminal_output_delta": {"terminal_id": "t1", "data": "\u{1b}[1mscratch\u{1b}[0m\n"},
            "terminal_exit": {"terminal_id": "t1", "exit_code": 0, "signal": null}}),
        ));
        assert_eq!(shape(&out.events), ["tool output", "tool output"]);
        assert_eq!(
            content_of(&tool_entry(&agent, "c1")),
            [ToolItem::Output(ToolOutputItem {
                cwd: Some("/home/user/scratch".into()),
                text: "/home/user/scratch\n".into(),
                exit: Some(Exit::Code { code: 0 }),
                ..ToolOutputItem::default()
            })]
        );
        assert!(agent.s.process.book.outputs.is_empty(), "closed at its exit");
        // Output for a terminal nothing opened opens one on its call (flag 16).
        agent.feed(&tool_call("c2", "Read", "in_progress"));
        agent.feed(&meta(
            "c2",
            json!({"terminal_output_delta": {"terminal_id": "t2", "data": "an apple a day\n"}}),
        ));
        assert!(matches!(&content_of(&tool_entry(&agent, "c2"))[..],
            [ToolItem::Output(ToolOutputItem { text, command: None, .. })] if text == "an apple a day\n"));
        // A resent `[terminal]` keeps the text.
        agent.feed(&update(
            json!({"sessionUpdate": "tool_call_update", "toolCallId": "c2",
            "content": [{"type": "terminal", "terminalId": "t2"}]}),
        ));
        assert!(matches!(&content_of(&tool_entry(&agent, "c2"))[..],
            [ToolItem::Output(ToolOutputItem { text, .. })] if text == "an apple a day\n"));
        // A restart closes the open outputs.
        assert!(agent.s.process.book.outputs.contains_key("t2"));
        let mut step = Step {
            s: &mut agent.s,
            out: Output::default(),
            fault: None,
            now: agent.now,
            event_limit: MAX_EVENT,
        };
        step.respawn(Start::New);
        assert!(agent.s.process.book.outputs.is_empty());
    }

    #[test]
    fn tool_content_is_bounded_before_any_event() {
        let mut agent = Agent::ready();
        agent.turn("big");
        let items: Vec<Value> = (0..40)
            .map(|n| json!({"type": "content", "content": {"type": "text", "text": format!("item {n}")}}))
            .collect();
        let big = "b".repeat(ITEM_LIMIT + 5);
        let mut content =
            vec![json!({"type": "content", "content": {"type": "text", "text": big}})];
        content.extend(items);
        content.push(json!({"type": "hologram"}));
        content.push(json!({"type": "content"}));
        let raw = "r".repeat(RAW_LIMIT * 2);
        let locations: Vec<Value> = (0..40).map(|n| json!({"path": format!("/f{n}")})).collect();
        let out = agent.feed(&update(
            json!({"sessionUpdate": "tool_call", "toolCallId": "c1", "title": "Big",
            "content": content, "rawInput": raw, "locations": locations}),
        ));
        let Entry::Tool {
            content,
            more,
            input,
            locations,
            ..
        } = tool_entry(&agent, "c1")
        else {
            panic!("not a tool");
        };
        assert_eq!(content.len(), MAX_ITEMS);
        assert!(
            matches!(&content[0], ToolItem::Text { text, cut: true } if text.len() == ITEM_LIMIT)
        );
        // 41 readable items and the unknown one, 32 kept.
        assert_eq!(more, 42 - 32);
        let input = input.expect("an input");
        assert!(input.len() <= RAW_LIMIT && input.ends_with('…'));
        assert_eq!(locations.len(), MAX_LOCATIONS);
        // The unreadable item is one notice.
        assert_eq!(
            agent.notices(),
            ["1 content items of tool call Big couldn't be read"]
        );
        for event in &out.events {
            assert!(event.to_value().to_string().len() < 1024 * 1024);
        }
    }

    /// One delta carrying 1 MiB of output (a frame's size) goes out in
    /// `TEXT_CHUNK` pieces, each event fitting a frame, and the output
    /// keeps what the whole text would: its newest lines within
    /// `ITEM_LIMIT`, the rest counted.
    #[test]
    fn a_delta_past_a_frame_is_sent_in_pieces() {
        let mut agent = Agent::ready();
        agent.turn("run");
        agent.feed(&update(json!({"sessionUpdate": "tool_call", "toolCallId": "c1", "title": "Run",
            "kind": "execute", "status": "in_progress", "content": [{"type": "terminal", "terminalId": "t1"}]})));
        let line = |n: usize| format!("test case {n:06} ... ok\n");
        let text: String = (0..(1024 * 1024 / line(0).len())).map(line).collect();
        let out = agent.feed(&meta(
            "c1",
            json!({"terminal_output_delta": {"terminal_id": "t1", "data": text}}),
        ));
        let outputs: Vec<&Event> = out
            .events
            .iter()
            .filter(|event| matches!(event, Event::ToolOutput { .. }))
            .collect();
        assert_eq!(outputs.len(), text.len().div_ceil(TEXT_CHUNK));
        for event in &out.events {
            assert!(event.to_value().to_string().len() < 128 * 1024);
        }
        let [ToolItem::Output(kept)] = &content_of(&tool_entry(&agent, "c1"))[..] else {
            panic!("not one output");
        };
        assert!(kept.text.len() <= ITEM_LIMIT && kept.text.starts_with("test case "));
        assert!(text.ends_with(&kept.text));
        assert_eq!(kept.omitted as usize + kept.text.len(), text.len());
    }

    #[test]
    fn a_trim_forgets_the_trimmed_entries_outputs() {
        let mut agent = Agent::ready();
        agent.turn("run");
        agent.feed(&meta(
            "c0",
            json!({"terminal_output_delta": {"terminal_id": "t0", "data": "x"}}),
        ));
        assert!(agent.s.process.book.outputs.contains_key("t0"));
        let mut step = Step {
            s: &mut agent.s,
            out: Output::default(),
            fault: None,
            now: agent.now,
            event_limit: MAX_EVENT,
        };
        let filler = "f".repeat(TEXT_CHUNK);
        while step.s.transcript.encoded_len() <= LOG_LIMIT {
            step.add_text(RunKind::Agent, &filler);
        }
        step.trim();
        assert!(agent.s.process.book.outputs.is_empty());
    }

    // --- tool calls -----------------------------------------------------

    #[test]
    fn a_tool_call_is_one_entry_its_updates_change_by_id() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.feed(&tool_call("call_1", "Write a file", "pending"));
        let out = agent.feed(&tool_update("call_1", "completed"));
        assert_eq!(
            out.events,
            [Event::ToolChanged {
                id: EntryId(2),
                title: None,
                kind: None,
                status: Some(ToolStatus::Completed),
                locations: None,
                input: None,
                result: None,
                more: None,
            }]
        );
        assert_eq!(
            agent.last_entry().1,
            Entry::tool(
                "call_1".into(),
                "Write a file".into(),
                ToolKind::Edit,
                ToolStatus::Completed
            )
        );
    }

    #[test]
    fn tool_statuses_apply_in_the_order_they_come_even_backwards() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.feed(&tool_call("call_1", "Write", "in_progress"));
        for status in ["pending", "in_progress", "failed"] {
            agent.feed(&tool_update("call_1", status));
        }
        assert!(matches!(
            agent.last_entry().1,
            Entry::Tool {
                status: ToolStatus::Failed,
                ..
            }
        ));
        let statuses: Vec<ToolStatus> = agent
            .events
            .iter()
            .filter_map(|event| match event {
                Event::ToolChanged { status, .. } => *status,
                Event::Added { .. }
                | Event::Appended { .. }
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
            })
            .collect();
        assert_eq!(
            statuses,
            [
                ToolStatus::Pending,
                ToolStatus::InProgress,
                ToolStatus::Failed
            ]
        );
    }

    #[test]
    fn a_tool_call_sent_again_changes_its_entry() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.feed(&tool_call("call_1", "Write", "pending"));
        let out = agent.feed(&tool_call("call_1", "Write hello.txt", "pending"));
        assert_eq!(shape(&out.events), ["tool changed"]);
        assert_eq!(agent.entries().len(), 2);
    }

    #[test]
    fn fields_that_didnt_change_send_nothing() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.feed(&tool_call("call_1", "Write", "pending"));
        assert!(
            agent
                .feed(&tool_call("call_1", "Write", "pending"))
                .events
                .is_empty()
        );
        assert!(
            agent
                .feed(&tool_update("call_1", "pending"))
                .events
                .is_empty()
        );
    }

    #[test]
    fn an_update_for_a_call_the_log_doesnt_have_adds_it() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.feed(&tool_update("call_9", "in_progress"));
        assert_eq!(
            agent.last_entry().1,
            Entry::tool(
                "call_9".into(),
                "call_9".into(),
                ToolKind::Other,
                ToolStatus::InProgress
            )
        );
    }

    // --- the tool book ----------------------------------------------------

    /// A book of calls and outputs at entries 1 to 4.
    fn book() -> ToolBook {
        let mut book = ToolBook::default();
        for n in 1..=4 {
            book.tools.insert(format!("c{n}"), EntryId(n));
            let open = OpenOutput {
                entry: EntryId(n),
                item: 0,
                filter: plain::State::default(),
            };
            book.outputs.insert(format!("t{n}"), open);
        }
        book
    }

    #[test]
    fn forget_through_keeps_only_later_calls_and_outputs() {
        let mut book = book();
        book.forget_through(EntryId(2));
        let calls: Vec<&str> = book.tools.keys().map(String::as_str).collect();
        let terminals: Vec<&str> = book.outputs.keys().map(String::as_str).collect();
        assert_eq!(calls, ["c3", "c4"]);
        assert_eq!(terminals, ["t3", "t4"]);
        book.forget_through(EntryId(4));
        assert_eq!(book, ToolBook::default());
    }
}
