//! A tool call's detail in an agent zone (OpenSpec change
//! `agents-tool-content`, design decision 10): what an open row shows
//! below its line, as parts the conversation's list lays out one at a
//! time; the summary on its line; what its copy buttons copy; and whether
//! the row is open.
//!
//! Pure: values in, values out (`fcis R1`). The model ([`super::model`])
//! renders a text item's Markdown and draws the parts as blocks.

use std::sync::Arc;

use gpui::SharedString;

use crate::agent::transcript::{DiffLine, Entry, Exit, FileChange, FileOp, Output, ToolItem, ToolStatus};
use crate::config::AgentConfig;
pub use crate::config::Shown;

/// The most lines of a diff or an output one part holds, so the list lays
/// out only the parts on screen however long the item.
pub const DETAIL_LINES: usize = 50;

/// How tool call rows open on their own: `[agent]`'s `tool_details` for a
/// running call, `tool_details_when_done` for a finished one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ToolPolicy {
    pub running: Shown,
    pub done: Shown,
}

impl Default for ToolPolicy {
    fn default() -> Self {
        Self { running: Shown::Open, done: Shown::Closed }
    }
}

impl ToolPolicy {
    /// The policy `[agent]` sets.
    pub fn of(config: &AgentConfig) -> Self {
        Self { running: config.tool_details, done: config.tool_details_when_done }
    }
}

/// How a tool call's row is drawn (the dump's `open=`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowState {
    /// The line, then the detail.
    Open,
    /// The line alone, with a chevron to open it.
    Closed,
    /// The line alone: a pending permission shows the detail.
    WithPermission,
    /// The line alone: there is nothing to open.
    Nothing,
}

impl RowState {
    /// The dump's word.
    pub fn word(self) -> &'static str {
        match self {
            Self::Open => "yes",
            Self::Closed => "no",
            Self::WithPermission => "permission",
            Self::Nothing => "none",
        }
    }

    /// What the row's chevron shows: none while there is nothing the user
    /// can open or close.
    pub fn open(self) -> Option<bool> {
        match self {
            Self::Open => Some(true),
            Self::Closed => Some(false),
            Self::WithPermission | Self::Nothing => None,
        }
    }
}

/// How a row of a call with a detail is drawn, in this order: with a
/// pending permission that names it; as the user left it; else as the
/// policy says for a running or a finished call.
pub fn row_state(
    entry: &Entry,
    by_hand: Option<Shown>,
    running: bool,
    pending_permission: bool,
    policy: ToolPolicy,
) -> RowState {
    if !has_detail(entry) {
        return RowState::Nothing;
    }
    if pending_permission {
        return RowState::WithPermission;
    }
    let shown = by_hand.unwrap_or(if running { policy.running } else { policy.done });
    match shown {
        Shown::Open => RowState::Open,
        Shown::Closed => RowState::Closed,
    }
}

/// Whether a tool call's status still runs (the turn decides the rest).
pub fn unfinished(status: ToolStatus) -> bool {
    match status {
        ToolStatus::Pending | ToolStatus::InProgress => true,
        ToolStatus::Completed | ToolStatus::Failed => false,
    }
}

/// Whether a tool entry has anything to open: content, a count of items
/// left out, locations, or a raw value.
pub fn has_detail(entry: &Entry) -> bool {
    match entry {
        Entry::Tool { content, more, locations, input, result, .. } => {
            !content.is_empty() || *more > 0 || !locations.is_empty() || input.is_some() || result.is_some()
        }
        Entry::User { .. } | Entry::Agent { .. } | Entry::Thought { .. } | Entry::Permission { .. } | Entry::Notice { .. } => {
            false
        }
    }
}

/// A diff line's sign, and one line of a diff as drawn: its sign, its line
/// in the old and the new text, its text, and whether it starts a hunk
/// after another (a border line above it). Plain values: the parts hold
/// what the zone's [`brindle_ui::Diff`] draws, so drawing reads them as
/// they are (`process R14`).
pub use brindle_ui::{DiffRowData as DiffRow, DiffSign as Sign};

/// Whether `part`'s frame goes on into `next`, the block after it: a later
/// run of the same diff's or output's lines. A part with a header starts
/// its item's frame.
pub fn continues_into(part: &DetailPart, next: Option<&DetailPart>) -> bool {
    match (part, next) {
        (DetailPart::Diff { item, .. }, Some(DetailPart::Diff { item: next, head: None, .. }))
        | (DetailPart::Output { item, .. }, Some(DetailPart::Output { item: next, head: None, .. })) => item == next,
        (
            DetailPart::Text { .. }
            | DetailPart::Diff { .. }
            | DetailPart::Output { .. }
            | DetailPart::Locations { .. }
            | DetailPart::Raw { .. }
            | DetailPart::Mark { .. },
            _,
        ) => false,
    }
}

/// A diff's header: its operation's word, its path and, for a move or
/// copy, the path it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffHead {
    pub op: SharedString,
    pub path: SharedString,
    pub from: Option<SharedString>,
}

/// An output's header: its command line, when the agent gave one, and how
/// it ended, once it did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputHead {
    pub command: Option<SharedString>,
    pub exit: Option<SharedString>,
    /// Its code block's label: the command (else `output`), then how it
    /// ended (`cargo test · exit 1`).
    pub label: SharedString,
}

impl OutputHead {
    pub fn new(command: Option<SharedString>, exit: Option<SharedString>) -> Self {
        let what = command.clone().unwrap_or_else(|| "output".into());
        let label = match &exit {
            Some(exit) => format!("{what} \u{b7} {exit}").into(),
            None => what,
        };
        Self { command, exit, label }
    }
}

/// Which raw value a part shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Raw {
    Input,
    Result,
}

impl Raw {
    pub fn word(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Result => "result",
        }
    }
}

/// One part of a tool call's detail, a list item of its own. A diff's or
/// an output's first part carries its header; the parts after it continue
/// its lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DetailPart {
    /// Text item `item`, its Markdown rendered by the model; its source
    /// stays in the entry (OpenSpec change `agents-absorb-cost`, decision 8).
    Text { item: usize },
    /// Up to [`DETAIL_LINES`] lines of diff item `item`.
    Diff { item: usize, head: Option<DiffHead>, rows: Arc<[DiffRow]> },
    /// Up to [`DETAIL_LINES`] lines of output item `item`, whole lines with
    /// their line breaks.
    Output { item: usize, head: Option<OutputHead>, text: SharedString },
    /// The files the call works on, one line each (`path:line`).
    Locations { lines: Arc<[SharedString]> },
    /// A raw value, as text.
    Raw { which: Raw, text: SharedString },
    /// What was left out, in words.
    Mark { text: SharedString },
}

/// A file operation's word on a diff's header.
pub fn op_word(op: &FileOp) -> SharedString {
    match op {
        FileOp::Add => "Added".into(),
        FileOp::Delete => "Deleted".into(),
        FileOp::Modify => "Modified".into(),
        FileOp::Move { .. } => "Moved".into(),
        FileOp::Copy { .. } => "Copied".into(),
        FileOp::Other { name } => name.clone().into(),
    }
}

/// How a command ended, in words: `exit N`, or the signal's name.
pub fn exit_word(exit: &Exit) -> String {
    match exit {
        Exit::Code { code } => format!("exit {code}"),
        Exit::Signal { name } => name.clone(),
    }
}

fn plural(count: u64, one: &str, many: &str) -> String {
    if count == 1 { format!("1 {one}") } else { format!("{count} {many}") }
}

/// A tool call's detail, in order: its content items (a text item whole, a
/// diff's or an output's lines [`DETAIL_LINES`] at a time, each with the
/// marks for what it left out), the count of items left out, then its
/// locations, then its raw input and result only when it has no content
/// items. Empty for an entry with nothing to open.
pub fn detail(entry: &Entry) -> Vec<DetailPart> {
    let Entry::Tool { content, more, locations, input, result, .. } = entry else { return Vec::new() };
    let mut parts = Vec::new();
    for (item, content) in content.iter().enumerate() {
        match content {
            ToolItem::Text { cut, .. } => {
                parts.push(DetailPart::Text { item });
                if *cut {
                    parts.push(DetailPart::Mark { text: "The rest of this text isn't kept".into() });
                }
            }
            ToolItem::Diff(change) => diff_parts(item, change, &mut parts),
            ToolItem::Output(output) => parts.extend(output_parts(item, output)),
        }
    }
    if *more > 0 {
        let text = format!("{} not shown", plural(u64::from(*more), "more item", "more items"));
        parts.push(DetailPart::Mark { text: text.into() });
    }
    if !locations.is_empty() {
        let lines = locations
            .iter()
            .map(|location| match location.line {
                Some(line) => SharedString::from(format!("{}:{line}", location.path)),
                None => SharedString::from(location.path.clone()),
            })
            .collect();
        parts.push(DetailPart::Locations { lines });
    }
    if content.is_empty() {
        for (which, value) in [(Raw::Input, input), (Raw::Result, result)] {
            if let Some(text) = value {
                parts.push(DetailPart::Raw { which, text: text.clone().into() });
            }
        }
    }
    parts
}

/// A diff's rows with their line numbers, hunk by hunk.
pub fn diff_rows(change: &FileChange) -> Vec<DiffRow> {
    let mut rows = Vec::new();
    for (index, hunk) in change.hunks.iter().enumerate() {
        // Counted with `checked_add`: a side past `u32::MAX` goes
        // unnumbered (`agents-event-bounds` decision 7), whatever the hunk.
        let (mut old, mut new) = (Some(hunk.old_start), Some(hunk.new_start));
        let next = |at: Option<u32>| at.and_then(|at| at.checked_add(1));
        for (at, line) in hunk.lines.iter().enumerate() {
            let (sign, old_line, new_line) = match line {
                DiffLine::Context(_) => {
                    let numbers = (Sign::Context, old, new);
                    old = next(old);
                    new = next(new);
                    numbers
                }
                DiffLine::Removed(_) => {
                    let numbers = (Sign::Removed, old, None);
                    old = next(old);
                    numbers
                }
                DiffLine::Added(_) => {
                    let numbers = (Sign::Added, None, new);
                    new = next(new);
                    numbers
                }
            };
            rows.push(DiffRow {
                sign,
                old: old_line,
                new: new_line,
                text: line.text().to_string().into(),
                hunk_start: index > 0 && at == 0,
            });
        }
    }
    rows
}

fn diff_parts(item: usize, change: &FileChange, parts: &mut Vec<DetailPart>) {
    let from = match &change.op {
        FileOp::Move { from } | FileOp::Copy { from } => Some(SharedString::from(from.clone())),
        FileOp::Add | FileOp::Delete | FileOp::Modify | FileOp::Other { .. } => None,
    };
    let mut head = Some(DiffHead { op: op_word(&change.op), path: change.path.clone().into(), from });
    let rows = diff_rows(change);
    let mut chunks = rows.chunks(DETAIL_LINES).peekable();
    if chunks.peek().is_none() {
        parts.push(DetailPart::Diff { item, head: head.take(), rows: Arc::from([]) });
    }
    for chunk in chunks {
        parts.push(DetailPart::Diff { item, head: head.take(), rows: chunk.into() });
    }
    if change.omitted > 0 {
        let text = format!("{} not shown", plural(u64::from(change.omitted), "more line", "more lines"));
        parts.push(DetailPart::Mark { text: text.into() });
    }
}

/// Output item `item`'s parts, as [`detail`] holds them: the mark for its
/// dropped start, then its lines [`DETAIL_LINES`] at a time, the first
/// with its header. Built on their own, so a delta redraws one output
/// alone (OpenSpec change `agents-absorb-cost`, decision 6).
pub fn output_parts(item: usize, output: &Output) -> Vec<DetailPart> {
    let mut parts = Vec::new();
    if output.omitted > 0 {
        let mut text = format!("The first {} of output aren't kept", plural(output.omitted, "byte", "bytes"));
        if output.partial {
            text.push_str(", nor the start of its first line");
        }
        parts.push(DetailPart::Mark { text: text.into() });
    }
    let mut head = Some(OutputHead::new(
        output.command.clone().map(SharedString::from),
        output.exit.as_ref().map(|exit| exit_word(exit).into()),
    ));
    let text = output.text.as_str();
    let mut start = 0;
    loop {
        let end = text[start..]
            .match_indices('\n')
            .nth(DETAIL_LINES - 1)
            .map_or(text.len(), |(at, _)| start + at + 1);
        parts.push(DetailPart::Output { item, head: head.take(), text: text[start..end].to_string().into() });
        start = end;
        if start >= text.len() {
            break;
        }
    }
    parts
}

/// The summary on a tool call's line: the lines added and removed over
/// its diffs (`+A −R`), then how its last output that ended did; `None`
/// with neither.
pub fn summary(entry: &Entry) -> Option<SharedString> {
    let Entry::Tool { content, .. } = entry else { return None };
    let (mut added, mut removed, mut diffs) = (0usize, 0usize, false);
    let mut exit = None;
    for item in content {
        match item {
            ToolItem::Diff(change) => {
                diffs = true;
                for line in change.hunks.iter().flat_map(|hunk| &hunk.lines) {
                    match line {
                        DiffLine::Added(_) => added += 1,
                        DiffLine::Removed(_) => removed += 1,
                        DiffLine::Context(_) => {}
                    }
                }
            }
            ToolItem::Output(Output { exit: Some(ended), .. }) => exit = Some(exit_word(ended)),
            ToolItem::Output(_) | ToolItem::Text { .. } => {}
        }
    }
    let words: Vec<String> = diffs.then(|| format!("+{added} \u{2212}{removed}")).into_iter().chain(exit).collect();
    (!words.is_empty()).then(|| words.join(" · ").into())
}

/// What a content item's copy button copies: a diff as unified diff text,
/// an output as its text, a text item as its source.
pub fn copy_text(item: &ToolItem) -> String {
    match item {
        ToolItem::Text { text, .. } => text.clone(),
        ToolItem::Output(output) => output.text.to_string(),
        ToolItem::Diff(change) => {
            let (old, new) = match &change.op {
                FileOp::Add => ("/dev/null".to_string(), change.path.clone()),
                FileOp::Delete => (change.path.clone(), "/dev/null".to_string()),
                FileOp::Move { from } | FileOp::Copy { from } => (from.clone(), change.path.clone()),
                FileOp::Modify | FileOp::Other { .. } => (change.path.clone(), change.path.clone()),
            };
            let mut out = format!("--- {old}\n+++ {new}\n");
            for hunk in &change.hunks {
                let count = |keep: fn(&DiffLine) -> bool| hunk.lines.iter().filter(|line| keep(line)).count();
                let olds = count(|line| !matches!(line, DiffLine::Added(_)));
                let news = count(|line| !matches!(line, DiffLine::Removed(_)));
                out.push_str(&format!("@@ -{},{olds} +{},{news} @@\n", hunk.old_start, hunk.new_start));
                for line in &hunk.lines {
                    let sign = match line {
                        DiffLine::Context(_) => ' ',
                        DiffLine::Added(_) => '+',
                        DiffLine::Removed(_) => '-',
                    };
                    out.push(sign);
                    out.push_str(line.text());
                    out.push('\n');
                }
            }
            out
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::transcript::{Hunk, Location, ToolKind};

    fn tool(content: Vec<ToolItem>) -> Entry {
        let Entry::Tool { call, title, kind, status, .. } =
            Entry::tool("c1".into(), "Edit".into(), ToolKind::Edit, ToolStatus::InProgress)
        else {
            unreachable!()
        };
        Entry::Tool { call, title, kind, status, content, more: 0, locations: Vec::new(), input: None, result: None }
    }

    fn with(entry: Entry, change: impl FnOnce(&mut Entry)) -> Entry {
        let mut entry = entry;
        change(&mut entry);
        entry
    }

    fn change(op: FileOp, hunks: Vec<Hunk>) -> FileChange {
        FileChange { op, path: "/p/main.rs".into(), hunks, omitted: 0 }
    }

    fn hunk(old: u32, new: u32, lines: &[&str]) -> Hunk {
        let lines = lines
            .iter()
            .map(|line| match line.split_at(1) {
                ("+", text) => DiffLine::Added(text.into()),
                ("-", text) => DiffLine::Removed(text.into()),
                (_, text) => DiffLine::Context(text.into()),
            })
            .collect();
        Hunk { old_start: old, new_start: new, lines }
    }

    fn output(text: &str, exit: Option<Exit>) -> ToolItem {
        ToolItem::Output(Output { text: text.into(), exit, ..Output::default() })
    }

    fn main_rs() -> ToolItem {
        ToolItem::Diff(change(FileOp::Modify, vec![hunk(10, 12, &[" fn main() {", "-    old();", "+    new();", " }"])]))
    }

    #[test]
    fn a_row_state_follows_the_permission_the_hand_then_the_policy() {
        let empty = tool(Vec::new());
        let full = tool(vec![main_rs()]);
        let policies = [
            ToolPolicy::default(),
            ToolPolicy { running: Shown::Closed, done: Shown::Open },
            ToolPolicy { running: Shown::Open, done: Shown::Open },
            ToolPolicy { running: Shown::Closed, done: Shown::Closed },
        ];
        let hands = [None, Some(Shown::Open), Some(Shown::Closed)];
        for policy in policies {
            for by_hand in hands {
                for running in [false, true] {
                    for pending in [false, true] {
                        let state = row_state(&empty, by_hand, running, pending, policy);
                        assert_eq!(state, RowState::Nothing, "no detail, nothing to open");
                        let state = row_state(&full, by_hand, running, pending, policy);
                        let want = if pending {
                            RowState::WithPermission
                        } else {
                            match by_hand.unwrap_or(if running { policy.running } else { policy.done }) {
                                Shown::Open => RowState::Open,
                                Shown::Closed => RowState::Closed,
                            }
                        };
                        assert_eq!(state, want, "{policy:?} {by_hand:?} running={running} pending={pending}");
                    }
                }
            }
        }
        // The defaults: open while running, closed once done.
        assert_eq!(row_state(&full, None, true, false, ToolPolicy::default()), RowState::Open);
        assert_eq!(row_state(&full, None, false, false, ToolPolicy::default()), RowState::Closed);
        assert_eq!(RowState::WithPermission.word(), "permission");
        assert_eq!(RowState::Nothing.open(), None);
        assert_eq!(RowState::Closed.open(), Some(false));
    }

    #[test]
    fn anything_but_content_also_has_a_detail() {
        let base = tool(Vec::new());
        assert!(!has_detail(&base));
        assert!(has_detail(&with(base.clone(), |e| if let Entry::Tool { more, .. } = e { *more = 1 })));
        assert!(has_detail(&with(base.clone(), |e| {
            if let Entry::Tool { locations, .. } = e {
                locations.push(Location { path: "/p".into(), line: None });
            }
        })));
        assert!(has_detail(&with(base.clone(), |e| if let Entry::Tool { input, .. } = e { *input = Some("{}".into()) })));
        assert!(has_detail(&with(base, |e| if let Entry::Tool { result, .. } = e { *result = Some("ok".into()) })));
        assert!(!has_detail(&Entry::User { text: "hi".into() }));
    }

    #[test]
    fn a_detail_is_items_then_locations_then_raw_values_only_without_items() {
        let raw = |entry: Entry| {
            with(entry, |e| {
                if let Entry::Tool { locations, input, result, .. } = e {
                    locations.push(Location { path: "/p/main.rs".into(), line: Some(10) });
                    locations.push(Location { path: "/p/lib.rs".into(), line: None });
                    *input = Some("{\"patch\":1}".into());
                    *result = Some("done".into());
                }
            })
        };
        let parts = detail(&raw(tool(vec![ToolItem::Text { text: "**hi**".into(), cut: false }, main_rs()])));
        assert_eq!(parts[0], DetailPart::Text { item: 0 });
        assert!(matches!(&parts[1], DetailPart::Diff { item: 1, head: Some(_), .. }));
        let DetailPart::Locations { lines } = &parts[2] else { panic!("{parts:?}") };
        assert_eq!(lines.iter().map(|l| l.as_ref()).collect::<Vec<_>>(), ["/p/main.rs:10", "/p/lib.rs"]);
        assert_eq!(parts.len(), 3, "no raw values beside content: {parts:?}");

        let parts = detail(&raw(tool(Vec::new())));
        assert!(matches!(&parts[0], DetailPart::Locations { .. }));
        assert_eq!(parts[1], DetailPart::Raw { which: Raw::Input, text: "{\"patch\":1}".into() });
        assert_eq!(parts[2], DetailPart::Raw { which: Raw::Result, text: "done".into() });
        assert!(detail(&tool(Vec::new())).is_empty());
        assert!(detail(&Entry::Agent { text: "x".into() }).is_empty());
    }

    /// OpenSpec change `agents-absorb-cost` decision 6: one output's parts
    /// are what the whole detail holds for it, so a delta redraws them
    /// alone.
    #[test]
    fn an_outputs_parts_are_built_on_their_own() {
        let long: String = (0..(DETAIL_LINES * 2 + 5)).map(|n| format!("line {n}\n")).collect();
        let dropped = ToolItem::Output(Output { text: "tail\n".into(), omitted: 10, partial: true, ..Output::default() });
        let entry = tool(vec![
            ToolItem::Text { text: "a".into(), cut: true },
            output(&long, Some(Exit::Code { code: 0 })),
            main_rs(),
            dropped,
        ]);
        let whole = detail(&entry);
        let Entry::Tool { content, .. } = &entry else { unreachable!() };
        for (item, content) in content.iter().enumerate() {
            let ToolItem::Output(output) = content else { continue };
            let own = output_parts(item, output);
            let at = whole
                .iter()
                .position(|part| matches!(part, DetailPart::Output { item: own, .. } if *own == item))
                .expect("its parts")
                - usize::from(output.omitted > 0);
            assert_eq!(whole[at..at + own.len()], own[..], "item {item}");
        }
    }

    /// Decision 8: a text item's part names its item; the source stays in
    /// the entry.
    #[test]
    fn a_text_parts_detail_holds_no_source() {
        let parts = detail(&tool(vec![ToolItem::Text { text: "**hi**".into(), cut: false }]));
        assert_eq!(parts, [DetailPart::Text { item: 0 }]);
    }

    #[test]
    fn a_diffs_rows_carry_signs_line_numbers_and_hunk_starts() {
        let two = change(FileOp::Modify, vec![hunk(10, 12, &[" a", "-b", "+c", " d"]), hunk(40, 42, &["+e"])]);
        let rows = diff_rows(&two);
        type Shown<'a> = (Sign, Option<u32>, Option<u32>, &'a str, bool);
        let shown: Vec<Shown> = rows.iter().map(|r| (r.sign, r.old, r.new, r.text.as_ref(), r.hunk_start)).collect();
        assert_eq!(
            shown,
            [
                (Sign::Context, Some(10), Some(12), "a", false),
                (Sign::Removed, Some(11), None, "b", false),
                (Sign::Added, None, Some(13), "c", false),
                (Sign::Context, Some(12), Some(14), "d", false),
                (Sign::Added, None, Some(42), "e", true),
            ]
        );
    }

    /// A long diff or output comes in parts of [`DETAIL_LINES`] lines, the
    /// first with the header; marks say what was left out.
    #[test]
    fn long_items_come_in_parts_with_their_marks() {
        let lines: Vec<String> = (0..120).map(|n| format!("+line {n}")).collect();
        let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
        let mut long = change(FileOp::Add, vec![hunk(0, 1, &refs)]);
        long.omitted = 400;
        let parts = detail(&tool(vec![ToolItem::Diff(long)]));
        let sizes: Vec<(bool, usize)> = parts
            .iter()
            .filter_map(|p| match p {
                DetailPart::Diff { head, rows, .. } => Some((head.is_some(), rows.len())),
                _ => None,
            })
            .collect();
        assert_eq!(sizes, [(true, 50), (false, 50), (false, 20)]);
        assert_eq!(parts.last(), Some(&DetailPart::Mark { text: "400 more lines not shown".into() }));
        // One frame across the runs, ending before the mark.
        let joins: Vec<bool> = (0..parts.len()).map(|ix| continues_into(&parts[ix], parts.get(ix + 1))).collect();
        assert_eq!(joins, [true, true, false, false]);
        // Two diffs one after the other are two frames.
        let two = detail(&tool(vec![main_rs(), main_rs()]));
        assert!(!continues_into(&two[0], two.get(1)));

        let text: String = (0..120).map(|n| format!("{n}\n")).collect();
        let mut cut = Output { text: text.clone().into(), exit: Some(Exit::Code { code: 2 }), ..Output::default() };
        cut.omitted = 1000;
        cut.partial = true;
        let parts = detail(&tool(vec![ToolItem::Output(cut)]));
        assert_eq!(
            parts[0],
            DetailPart::Mark { text: "The first 1000 bytes of output aren't kept, nor the start of its first line".into() }
        );
        let texts: Vec<&str> = parts[1..]
            .iter()
            .map(|p| match p {
                DetailPart::Output { text, .. } => text.as_ref(),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(texts.concat(), text, "the parts hold the text whole");
        assert_eq!(texts.iter().map(|t| t.lines().count()).collect::<Vec<_>>(), [50, 50, 20]);
        assert!(continues_into(&parts[1], parts.get(2)) && !continues_into(&parts[3], parts.get(4)));
        assert!(!continues_into(&parts[0], parts.get(1)), "a mark joins nothing");
        let DetailPart::Output { head: Some(head), .. } = &parts[1] else { panic!("{parts:?}") };
        assert_eq!(head.exit, Some("exit 2".into()));
        assert_eq!(head.label.as_ref(), "output \u{b7} exit 2");
        assert_eq!(OutputHead::new(Some("cargo test".into()), None).label.as_ref(), "cargo test");

        // An empty output, still running, is one part.
        let parts = detail(&tool(vec![output("", None)]));
        assert_eq!(parts.len(), 1);
        let parts = detail(&with(tool(vec![main_rs()]), |e| if let Entry::Tool { more, .. } = e { *more = 3 }));
        assert_eq!(parts.last(), Some(&DetailPart::Mark { text: "3 more items not shown".into() }));
        let parts = detail(&tool(vec![ToolItem::Text { text: "x".into(), cut: true }]));
        assert!(matches!(&parts[1], DetailPart::Mark { .. }));
    }

    #[test]
    fn a_move_with_no_hunks_is_its_header_alone() {
        let moved = FileChange { op: FileOp::Move { from: "/p/old.rs".into() }, path: "/p/new.rs".into(), hunks: vec![], omitted: 0 };
        let parts = detail(&tool(vec![ToolItem::Diff(moved.clone())]));
        let [DetailPart::Diff { item: 0, head: Some(head), rows }] = parts.as_slice() else { panic!("{parts:?}") };
        assert_eq!((head.op.as_ref(), head.path.as_ref()), ("Moved", "/p/new.rs"));
        assert_eq!(head.from, Some("/p/old.rs".into()));
        assert!(rows.is_empty());
        assert_eq!(summary(&tool(vec![ToolItem::Diff(moved)])), Some("+0 \u{2212}0".into()));
    }

    #[test]
    fn a_summary_counts_lines_and_names_the_last_exit() {
        assert_eq!(summary(&tool(Vec::new())), None);
        assert_eq!(summary(&tool(vec![main_rs()])), Some("+1 \u{2212}1".into()));
        assert_eq!(summary(&tool(vec![output("x", None)])), None, "still running");
        let items = vec![
            main_rs(),
            output("a", Some(Exit::Code { code: 0 })),
            output("b", Some(Exit::Code { code: 1 })),
            output("c", None),
        ];
        assert_eq!(summary(&tool(items)), Some("+1 \u{2212}1 · exit 1".into()));
        let killed = vec![output("a", Some(Exit::Signal { name: "SIGKILL".into() }))];
        assert_eq!(summary(&tool(killed)), Some("SIGKILL".into()));
    }

    #[test]
    fn copy_text_is_a_unified_diff_the_output_or_the_source() {
        let two = change(
            FileOp::Modify,
            vec![hunk(10, 10, &[" fn main() {", "-    old();", "+    new();", " }"]), hunk(40, 40, &["+    more();"])],
        );
        assert_eq!(
            copy_text(&ToolItem::Diff(two)),
            "--- /p/main.rs\n+++ /p/main.rs\n@@ -10,3 +10,3 @@\n fn main() {\n-    old();\n+    new();\n }\n@@ -40,0 +40,1 @@\n+    more();\n"
        );
        let added = change(FileOp::Add, vec![hunk(0, 1, &["+hi"])]);
        assert!(copy_text(&ToolItem::Diff(added)).starts_with("--- /dev/null\n+++ /p/main.rs\n"));
        assert_eq!(copy_text(&output("ok\n", None)), "ok\n");
        assert_eq!(copy_text(&ToolItem::Text { text: "**hi**".into(), cut: false }), "**hi**");
    }

    /// The rows of a hunk starting at `u32::MAX` with two context lines
    /// are drawn without a panic, the second line unnumbered (D-8, OpenSpec
    /// change `agents-event-bounds`).
    #[test]
    fn a_hunk_at_the_last_line_number_draws_without_a_panic() {
        let rows = diff_rows(&change(FileOp::Modify, vec![hunk(u32::MAX, u32::MAX, &[" a", " b"])]));
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].old, rows[0].new), (Some(u32::MAX), Some(u32::MAX)));
        assert_eq!((rows[1].old, rows[1].new), (None, None));
    }
}
