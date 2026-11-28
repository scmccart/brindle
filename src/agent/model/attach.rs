//! A prompt's attachments in the zone (OpenSpec change
//! `agents-prompt-context`, decisions 7, 8 and 9): the draft the next prompt
//! takes, what sending it sends, files dropped on the zone, and the
//! conversation's rows of a user message's kept texts.
//!
//! The draft is the zone's alone: nothing of it reaches the server until
//! the prompt is sent, and it isn't in the session state file.

use std::path::PathBuf;

use crate::agent::transcript::{ATTACHMENT_TEXT, Attachment, EntryId, MAX_ATTACHMENTS, clean};
use crate::remote::protocol::client::PromptAttachment;

use super::change::Mark;
use super::{ChatModel, RequestId};

/// One attachment waiting in the draft: a link to a file on the agent's
/// host, or a text Brindle holds until the prompt goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Draft {
    Link { path: String, name: String },
    Text { name: String, text: String },
}

impl Draft {
    /// A link to `path`, as typed, named by its last part.
    pub fn link(path: &str) -> Self {
        let path = clean(path);
        Self::Link { name: file_name(&path).to_string(), path }
    }

    /// `text`, named `name`, both cleaned as a prompt is (flag 6), so its
    /// size is what the server is sent.
    pub fn text(name: &str, text: &str) -> Self {
        Self::Text { name: clean(name), text: clean(text) }
    }

    pub fn name(&self) -> &str {
        match self {
            Self::Link { name, .. } | Self::Text { name, .. } => name,
        }
    }

    /// What the prompt input's row says beside the name: a link's path, a
    /// text's size.
    pub fn detail(&self) -> String {
        match self {
            Self::Link { path, .. } => path.clone(),
            Self::Text { text, .. } => size(text.len() as u64),
        }
    }

    /// Whether `attachment`, from the server's log or slot, is this one
    /// (decision 13's `taken`): a link by its path and name, a kept text by
    /// its name and size, since its id is the server's.
    pub fn same(&self, attachment: &Attachment) -> bool {
        match (self, attachment) {
            (Self::Link { path, name }, Attachment::Link { path: kept, name: kept_name }) => {
                path == kept && name == kept_name
            }
            (Self::Text { name, text }, Attachment::Text { name: kept, bytes, .. }) => {
                name == kept && text.len() as u64 == *bytes
            }
            (Self::Link { .. }, Attachment::Text { .. }) | (Self::Text { .. }, Attachment::Link { .. }) => false,
        }
    }
}

/// Whether `drafts` are `attachments`, one for one in order.
pub fn same_attachments(drafts: &[Draft], attachments: &[Attachment]) -> bool {
    drafts.len() == attachments.len() && drafts.iter().zip(attachments).all(|(draft, kept)| draft.same(kept))
}

/// The last part of `path`, split at `/` and at `\` (a Windows host's
/// path), or the path itself when that is empty.
fn file_name(path: &str) -> &str {
    let trimmed = path.trim_end_matches(['/', '\\']);
    match trimmed.rsplit(['/', '\\']).next() {
        Some(name) if !name.is_empty() => name,
        Some(_) | None => path,
    }
}

/// `bytes` for a person: `412 bytes`, `2.1 KiB`, `1 MiB`.
pub fn size(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * 1024;
    let scaled = |unit: u64, name: &str| {
        let tenths = (bytes * 10 + unit / 2) / unit;
        if tenths.is_multiple_of(10) { format!("{} {name}", tenths / 10) } else { format!("{}.{} {name}", tenths / 10, tenths % 10) }
    };
    if bytes < KIB {
        format!("{bytes} bytes")
    } else if bytes < MIB {
        scaled(KIB, "KiB")
    } else {
        scaled(MIB, "MiB")
    }
}

/// Why an attachment wasn't added to the draft: said on the window's
/// notice line, the draft unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AttachRefusal {
    /// The draft holds [`MAX_ATTACHMENTS`] already.
    TooMany,
    /// A text over [`ATTACHMENT_TEXT`], never shortened.
    TooLong { bytes: u64, limit: u64 },
}

impl AttachRefusal {
    /// The notice line's words, `what` naming the text (`the output`, a
    /// file's name).
    pub fn text(&self, what: &str) -> String {
        match self {
            Self::TooMany => format!("a prompt takes at most {MAX_ATTACHMENTS} attachments"),
            Self::TooLong { bytes, limit } => {
                format!("{what} is {}, and an attachment takes at most {}", size(*bytes), size(*limit))
            }
        }
    }
}

/// What sending a prompt with its draft sends, in order (decision 7): an
/// `AGENT_ATTACH` per text, then the `AGENT_PROMPT` naming them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendPlan {
    pub text: String,
    /// Each text to attach, as its name and text, in the draft's order.
    pub uploads: Vec<(String, String)>,
    /// The prompt's attachments, a text by its place in `uploads`.
    pub parts: Vec<PlanPart>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlanPart {
    Link { path: String, name: String },
    Upload(usize),
}

/// The plan for prompt `text` with `draft`.
pub fn send_plan(text: &str, draft: &[Draft]) -> SendPlan {
    let mut uploads = Vec::new();
    let parts = draft
        .iter()
        .map(|item| match item {
            Draft::Link { path, name } => PlanPart::Link { path: path.clone(), name: name.clone() },
            Draft::Text { name, text } => {
                uploads.push((name.clone(), text.clone()));
                PlanPart::Upload(uploads.len() - 1)
            }
        })
        .collect();
    SendPlan { text: text.to_string(), uploads, parts }
}

impl SendPlan {
    /// The prompt's attachments, once each text went as the upload of the
    /// same place in `numbers` (its `AGENT_ATTACH`'s request id).
    pub fn attachments(&self, numbers: &[RequestId]) -> Vec<PromptAttachment> {
        self.parts
            .iter()
            .filter_map(|part| match part {
                PlanPart::Link { path, name } => Some(PromptAttachment::Link { path: path.clone(), name: name.clone() }),
                PlanPart::Upload(index) => numbers.get(*index).map(|upload| PromptAttachment::Text { upload: *upload }),
            })
            .collect()
    }
}

/// A kept text's row in a user message (decision 9): closed, its text on
/// its way, shown, or gone from the server, saying why.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TextState {
    Closed,
    Loading,
    Shown(String),
    Gone(String),
}

impl TextState {
    /// The dump's name for it.
    pub fn name(&self) -> &'static str {
        match self {
            Self::Closed => "closed",
            Self::Loading => "loading",
            Self::Shown(_) => "shown",
            Self::Gone(_) => "gone",
        }
    }
}

/// The words a row says once the server can't read its text.
pub const NO_LONGER_KEPT: &str = "the text is no longer kept";

/// A path dropped on the zone, and whether it is a directory (which the
/// shell looked up).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dropped {
    pub path: PathBuf,
    pub dir: bool,
}

/// What to do with one dropped path ([`drop_plan`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropStep {
    /// Attach this at once.
    Attach(Draft),
    /// Read the file on a background task, at most [`ATTACHMENT_TEXT`] + 1
    /// bytes, and pass it to [`dropped_text`].
    Read { path: PathBuf, name: String },
    /// Refused: the file's name and why.
    Refuse { name: String, why: DropRefusal },
}

/// Why a dropped file wasn't attached.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DropRefusal {
    Directory,
    /// Not UTF-8 text, for an agent on another host.
    NotText,
    TooLong { bytes: u64, limit: u64 },
    Unreadable(String),
}

impl DropRefusal {
    /// The notice line's words for file `name`.
    pub fn text(&self, name: &str) -> String {
        match self {
            Self::Directory => format!("{name} is a directory, which can't be attached"),
            Self::NotText => format!("{name} isn't text and can't be sent to an agent on that host"),
            Self::TooLong { bytes, limit } => AttachRefusal::TooLong { bytes: *bytes, limit: *limit }.text(name),
            Self::Unreadable(why) => format!("{name} can't be read: {why}"),
        }
    }
}

/// What dropping `paths` does (decision 8): for an agent on this machine
/// (`local`) each file is a link to its path; for one on another host or
/// in a container each file is read and sent as text. A directory is
/// refused either way.
pub fn drop_plan(paths: &[Dropped], local: bool) -> Vec<DropStep> {
    paths
        .iter()
        .map(|dropped| {
            let shown = dropped.path.to_string_lossy();
            let name = file_name(&shown).to_string();
            if dropped.dir {
                DropStep::Refuse { name, why: DropRefusal::Directory }
            } else if local {
                DropStep::Attach(Draft::link(&shown))
            } else {
                DropStep::Read { path: dropped.path.clone(), name }
            }
        })
        .collect()
}

/// The text attachment a dropped file's read makes: `read` its first bytes
/// (at most [`ATTACHMENT_TEXT`] + 1) or why it couldn't be, `size` its
/// length on disk. Refused when it is longer than an attachment takes or
/// isn't UTF-8.
pub fn dropped_text(name: &str, size: u64, read: Result<Vec<u8>, String>) -> Result<Draft, DropRefusal> {
    let bytes = read.map_err(DropRefusal::Unreadable)?;
    let limit = ATTACHMENT_TEXT as u64;
    if size > limit || bytes.len() as u64 > limit {
        return Err(DropRefusal::TooLong { bytes: size.max(bytes.len() as u64), limit });
    }
    let text = String::from_utf8(bytes).map_err(|_| DropRefusal::NotText)?;
    Ok(Draft::text(name, &text))
}

/// What opening or closing a row asks of the shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowPress {
    /// Read the kept text `id` (`AGENT_READ_ATTACHMENT`).
    Read(u64),
    Nothing,
}

/// A draft attachment's identity in the zone (`types R3`): the prompt
/// input's remove mark names it, so a press removes the item it was drawn
/// for even after the list changed. Never reused while the zone lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DraftKey(pub u64);

/// An attachment of the next prompt, with its key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attached {
    pub key: DraftKey,
    pub draft: Draft,
}

impl ChatModel {
    /// The attachments the next prompt takes, with their keys.
    pub fn attachments(&self) -> &[Attached] {
        &self.attached
    }

    /// The attachments the next prompt takes.
    pub fn drafts(&self) -> Vec<Draft> {
        self.attached.iter().map(|attached| attached.draft.clone()).collect()
    }

    /// Adds `draft` to the next prompt's attachments, or says why not.
    pub fn attach(&mut self, draft: Draft) -> Result<(), AttachRefusal> {
        if self.attached.len() >= MAX_ATTACHMENTS {
            return Err(AttachRefusal::TooMany);
        }
        if let Draft::Text { text, .. } = &draft
            && text.len() > ATTACHMENT_TEXT
        {
            return Err(AttachRefusal::TooLong { bytes: text.len() as u64, limit: ATTACHMENT_TEXT as u64 });
        }
        self.next_draft += 1;
        self.attached.push(Attached { key: DraftKey(self.next_draft), draft });
        Ok(())
    }

    /// Backspace in the prompt input: with the cursor `at_start` and
    /// nothing `selected` it removes the last attachment (true), else the
    /// input handles it.
    pub fn backspace_at_start(&mut self, at_start: bool, selected: bool) -> bool {
        at_start && !selected && self.attached.pop().is_some()
    }

    /// The remove mark of the attachment keyed `key`; nothing once it is
    /// gone.
    pub fn remove_attachment(&mut self, key: DraftKey) {
        self.attached.retain(|attached| attached.key != key);
    }

    /// Clear Attachments.
    pub fn clear_attachments(&mut self) {
        self.attached.clear();
    }

    /// The plan for sending prompt `text` with the draft (decision 7); a
    /// new sequence, whose refusals are its own.
    pub fn send_plan(&mut self, text: &str) -> SendPlan {
        self.attach_refused = false;
        send_plan(text, &self.drafts())
    }

    /// The row of user entry `entry`'s attachment `index`, as it is drawn.
    pub fn row_state(&self, entry: EntryId, index: usize) -> &TextState {
        self.rows.get(&(entry, index)).unwrap_or(&TextState::Closed)
    }

    /// A press on the row of user entry `entry`'s kept text `index`: a
    /// closed (or gone) row asks for its text and shows it loading; an
    /// open or loading one closes, dropping its text.
    pub fn press_row(&mut self, entry: EntryId, index: usize) -> RowPress {
        let Some(id) = self.kept_id(entry, index) else {
            return RowPress::Nothing;
        };
        self.undrawn.mark(entry, Mark::Changed);
        match self.row_state(entry, index) {
            TextState::Closed | TextState::Gone(_) => {
                self.rows.insert((entry, index), TextState::Loading);
                RowPress::Read(id)
            }
            TextState::Loading | TextState::Shown(_) => {
                self.rows.remove(&(entry, index));
                RowPress::Nothing
            }
        }
    }

    /// A read that couldn't go: its row closes again.
    pub(super) fn row_closed(&mut self, entry: EntryId, index: usize) {
        if self.rows.remove(&(entry, index)).is_some() {
            self.undrawn.mark(entry, Mark::Changed);
        }
    }

    /// The kept text's id of user entry `entry`'s attachment `index`.
    fn kept_id(&self, entry: EntryId, index: usize) -> Option<u64> {
        match self.held(entry)?.attachments().get(index)? {
            Attachment::Text { id, .. } => Some(id.0),
            Attachment::Link { .. } => None,
        }
    }

    /// A read's answer for the row it was sent for: shown while the row
    /// still waits for it; dropped when the row was closed meanwhile.
    pub(super) fn row_answered(&mut self, entry: EntryId, index: usize, outcome: Result<String, String>) {
        let Some(state) = self.rows.get_mut(&(entry, index)) else {
            return;
        };
        if *state != TextState::Loading {
            return;
        }
        *state = match outcome {
            Ok(text) => TextState::Shown(text),
            Err(_) => TextState::Gone(NO_LONGER_KEPT.into()),
        };
        self.undrawn.mark(entry, Mark::Changed);
    }

    /// Whether `request` is a read on its way.
    pub fn reads(&self, request: RequestId) -> bool {
        matches!(self.pending.get(&request), Some(super::Sent::Read { .. }))
    }

    /// `ATTACHMENT` answering the read `request`.
    pub fn attachment_read(&mut self, request: RequestId, text: String) {
        if let Some(super::Sent::Read { entry, index }) = self.pending.remove(&request) {
            self.row_answered(entry, index, Ok(text));
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::agent::transcript::AttachmentId;

    use super::super::harness::*;
    use super::super::{BlockBody, Due, Replied, ReplyError, Sent};
    use super::*;

    #[test]
    fn sizes_read_as_bytes_kib_and_mib() {
        assert_eq!(size(412), "412 bytes");
        assert_eq!(size(2150), "2.1 KiB");
        assert_eq!(size(12 * 1024), "12 KiB");
        assert_eq!(size(256 * 1024), "256 KiB");
        assert_eq!(size(1024 * 1024), "1 MiB");
    }

    #[test]
    fn a_link_is_named_by_its_last_part_on_either_kind_of_host() {
        assert_eq!(Draft::link("src/main.rs"), Draft::Link { path: "src/main.rs".into(), name: "main.rs".into() });
        assert_eq!(Draft::link("C:\\w\\a.rs").name(), "a.rs");
        assert_eq!(Draft::link("docs/").name(), "docs");
        assert_eq!(Draft::link("/").name(), "/");
        assert_eq!(Draft::text("out\u{1b}", "a\u{7}b\n"), Draft::Text { name: "out\u{fffd}".into(), text: "a\u{fffd}b\n".into() });
    }

    /// Task 4.2: the count at its edge, the length at its edge (never
    /// shortened, both numbers said), and a refusal leaving the draft.
    #[test]
    fn attaching_refuses_a_ninth_and_a_text_over_the_limit() {
        let mut zone = Zone::ready();
        for n in 0..MAX_ATTACHMENTS {
            assert_eq!(zone.model.attach(Draft::link(&format!("f{n}"))), Ok(()));
        }
        assert_eq!(zone.model.attach(Draft::link("f8")), Err(AttachRefusal::TooMany));
        assert_eq!(AttachRefusal::TooMany.text("f8"), "a prompt takes at most 8 attachments");
        assert_eq!(zone.model.attachments().len(), 8);

        let mut zone = Zone::ready();
        let whole = "x".repeat(ATTACHMENT_TEXT);
        assert_eq!(zone.model.attach(Draft::text("whole", &whole)), Ok(()));
        let over = "x".repeat(ATTACHMENT_TEXT + 1);
        let refused = zone.model.attach(Draft::text("over", &over));
        assert_eq!(refused, Err(AttachRefusal::TooLong { bytes: 262_145, limit: 262_144 }));
        let words = AttachRefusal::TooLong { bytes: 1024 * 1024, limit: 262_144 }.text("the output");
        assert_eq!(words, "the output is 1 MiB, and an attachment takes at most 256 KiB");
        assert_eq!(zone.model.drafts(), [Draft::text("whole", &whole)]);
    }

    /// Task 4.2: backspace at the very start with nothing selected removes
    /// the last attachment; with a selection or mid-text it doesn't.
    #[test]
    fn backspace_at_the_start_removes_the_last_attachment() {
        let mut zone = Zone::ready();
        zone.model.attach(Draft::link("a")).unwrap();
        zone.model.attach(Draft::link("b")).unwrap();
        assert!(!zone.model.backspace_at_start(false, false), "mid-text");
        assert!(!zone.model.backspace_at_start(true, true), "a selection");
        assert_eq!(zone.model.attachments().len(), 2);
        assert!(zone.model.backspace_at_start(true, false));
        assert_eq!(zone.model.drafts(), [Draft::link("a")]);
        assert!(zone.model.backspace_at_start(true, false));
        assert!(!zone.model.backspace_at_start(true, false), "none left: the input's");
    }

    /// Task 6.1: a remove mark names its item by key, so a press removes
    /// the item it was drawn for even after the list changed, and a key
    /// once removed removes nothing.
    #[test]
    fn a_remove_mark_removes_its_own_item_by_key() {
        let mut zone = Zone::ready();
        for name in ["a", "b", "c"] {
            zone.model.attach(Draft::link(name)).unwrap();
        }
        let keys: Vec<DraftKey> = zone.model.attachments().iter().map(|attached| attached.key).collect();
        zone.model.backspace_at_start(true, false);
        zone.model.remove_attachment(keys[1]);
        assert_eq!(zone.model.drafts(), [Draft::link("a")]);
        zone.model.remove_attachment(keys[1]);
        zone.model.remove_attachment(keys[2]);
        assert_eq!(zone.model.drafts(), [Draft::link("a")]);
        zone.model.attach(Draft::link("d")).unwrap();
        assert!(!keys.contains(&zone.model.attachments()[1].key), "a new key");
        zone.model.attach(Draft::link("c")).unwrap();
        zone.model.clear_attachments();
        assert!(zone.model.attachments().is_empty());
    }

    /// Task 4.2: the plan attaches each text in the draft's order, then
    /// prompts with every attachment in its place.
    #[test]
    fn the_send_plan_attaches_each_text_then_prompts_naming_them() {
        let draft = [Draft::text("one", "1\n"), Draft::link("src/a.rs"), Draft::text("two", "2\n")];
        let plan = send_plan("why?", &draft);
        assert_eq!(plan.uploads, [("one".to_string(), "1\n".to_string()), ("two".to_string(), "2\n".to_string())]);
        assert_eq!(
            plan.attachments(&[41, 42]),
            [
                PromptAttachment::Text { upload: 41 },
                PromptAttachment::Link { path: "src/a.rs".into(), name: "a.rs".into() },
                PromptAttachment::Text { upload: 42 },
            ]
        );
        assert_eq!(send_plan("hi", &[]).attachments(&[]), []);
    }

    /// Task 4.2: an accepted prompt clears its text and draft; a refusal
    /// anywhere in the sequence keeps both, the attach's reason shown
    /// rather than the prompt's that follows it.
    #[test]
    fn a_refusal_in_the_sequence_keeps_the_text_and_the_draft() {
        let mut zone = Zone::ready();
        zone.model.attach(Draft::text("out", "ok\n")).unwrap();
        zone.model.sent(1, Sent::Attach);
        zone.model.sent(2, zone.model.sent_prompt("why?", false));
        assert_eq!(zone.model.replied(1, Err(ReplyError::Failed("the store is full".into()))), Replied::Nothing);
        let replied = zone.model.replied(2, Err(ReplyError::Failed("this connection holds no upload 1".into())));
        assert_eq!(replied, Replied::Nothing);
        assert_eq!(zone.model.refusal(), Some("the store is full"));
        assert_eq!(zone.model.attachments().len(), 1);

        zone.model.sent(3, Sent::Attach);
        zone.model.sent(4, zone.model.sent_prompt("why?", false));
        assert_eq!(zone.model.replied(3, Ok(())), Replied::Nothing);
        assert_eq!(zone.model.replied(4, Ok(())), Replied::ClearDraft);
        assert_eq!(zone.model.refusal(), None);
        assert!(zone.model.attachments().is_empty());
    }

    /// Task 4.3: a local agent's drops are links, a remote one's are read;
    /// a directory is refused either way.
    #[test]
    fn a_drop_links_for_a_local_agent_and_reads_for_a_remote_one() {
        let dropped = [
            Dropped { path: PathBuf::from("/home/u/a.rs"), dir: false },
            Dropped { path: PathBuf::from("/home/u/src"), dir: true },
        ];
        assert_eq!(
            drop_plan(&dropped, true),
            [
                DropStep::Attach(Draft::Link { path: "/home/u/a.rs".into(), name: "a.rs".into() }),
                DropStep::Refuse { name: "src".into(), why: DropRefusal::Directory },
            ]
        );
        assert_eq!(
            drop_plan(&dropped, false),
            [
                DropStep::Read { path: PathBuf::from("/home/u/a.rs"), name: "a.rs".into() },
                DropStep::Refuse { name: "src".into(), why: DropRefusal::Directory },
            ]
        );
        assert_eq!(DropRefusal::Directory.text("src"), "src is a directory, which can't be attached");
    }

    /// Task 4.3: a read file is text when it is UTF-8 and within the limit;
    /// an empty file is an empty text.
    #[test]
    fn a_dropped_file_is_text_when_it_is_utf8_within_the_limit() {
        assert_eq!(dropped_text("notes.md", 3, Ok(b"hi\n".to_vec())), Ok(Draft::text("notes.md", "hi\n")));
        assert_eq!(dropped_text("empty", 0, Ok(Vec::new())), Ok(Draft::text("empty", "")));
        assert_eq!(dropped_text("logo.png", 4, Ok(vec![0x89, b'P', 0xff, 0xfe])), Err(DropRefusal::NotText));
        assert_eq!(
            DropRefusal::NotText.text("logo.png"),
            "logo.png isn't text and can't be sent to an agent on that host"
        );
        let limit = ATTACHMENT_TEXT as u64;
        let whole = vec![b'x'; ATTACHMENT_TEXT];
        assert!(dropped_text("whole", limit, Ok(whole)).is_ok());
        let over = vec![b'x'; ATTACHMENT_TEXT + 1];
        let refused = dropped_text("big.log", 300 * 1024, Ok(over));
        assert_eq!(refused, Err(DropRefusal::TooLong { bytes: 300 * 1024, limit }));
        assert_eq!(
            DropRefusal::TooLong { bytes: 300 * 1024, limit }.text("big.log"),
            "big.log is 300 KiB, and an attachment takes at most 256 KiB"
        );
        assert_eq!(
            dropped_text("gone", 0, Err("permission denied".into())),
            Err(DropRefusal::Unreadable("permission denied".into()))
        );
    }

    fn kept(id: u64, name: &str, bytes: u64) -> Attachment {
        Attachment::Text { id: AttachmentId(id), name: name.into(), bytes }
    }

    /// Task 4.2 (design decision 13): a prompt lost with the link is taken
    /// only when its attachments match too, a link by path and name, a
    /// kept text by name and size.
    #[test]
    fn a_lost_prompt_is_taken_only_with_the_same_attachments() {
        use super::super::ledger::{PromptSent, Taken, taken};
        use crate::agent::transcript::Slot;
        let sent = PromptSent {
            text: "why?".into(),
            attachments: vec![Draft::text("cargo test", "ok\n"), Draft::link("src/a.rs")],
            after: None,
        };
        let link = Attachment::Link { path: "src/a.rs".into(), name: "a.rs".into() };
        let logged = |attachments: Vec<Attachment>| vec![(EntryId(1), user_with("why?", attachments))];
        let same = logged(vec![kept(1, "cargo test", 3), link.clone()]);
        assert_eq!(taken(&sent, &same, None, &Slot::Empty), Taken::Yes);
        assert_eq!(taken(&sent, &logged(vec![kept(1, "cargo test", 3)]), None, &Slot::Empty), Taken::No, "one fewer");
        let other = logged(vec![kept(1, "cargo test", 4), link.clone()]);
        assert_eq!(taken(&sent, &other, None, &Slot::Empty), Taken::No, "another size");
        assert_eq!(taken(&sent, &logged(Vec::new()), None, &Slot::Empty), Taken::No, "the text alone");
        let waiting = Slot::Waiting { text: "why?".into(), attachments: vec![kept(1, "cargo test", 3), link] };
        assert_eq!(taken(&sent, &[], None, &waiting), Taken::Yes);
    }

    /// What the zone draws of entry 1, as (part, last, what).
    fn drawn(zone: &Zone) -> Vec<(usize, bool, String)> {
        zone.model
            .blocks()
            .iter()
            .filter(|block| block.entry == EntryId(1))
            .map(|block| {
                let what = match &block.body {
                    BlockBody::UserText { text } => format!("text {text}"),
                    BlockBody::Attached { index, attachment, state } => {
                        format!("attached {index} {} {}", attachment.name(), state.name())
                    }
                    other => format!("{other:?}"),
                };
                (block.part, block.last, what)
            })
            .collect()
    }

    /// Tasks 4.1 and 4.4: a user message with attachments folds with its
    /// references and draws them in its bubble under its text, the last
    /// one ending it; an opened row is drawn again; a replay draws the
    /// same message again with its rows closed only when they were.
    #[test]
    fn a_user_message_draws_its_attachments_under_its_text() {
        let mut zone = Zone::ready();
        let link = Attachment::Link { path: "src/a.rs".into(), name: "a.rs".into() };
        let message = user_with("why?", vec![kept(5, "cargo test", 12), link]);
        zone.feed(&[added(1, message.clone())]);
        let closed = [
            (0, false, "text why?".to_string()),
            (1, false, "attached 0 cargo test closed".to_string()),
            (2, true, "attached 1 a.rs closed".to_string()),
        ];
        assert_eq!(drawn(&zone), closed);

        zone.model.press_row(EntryId(1), 0);
        zone.model.sent(3, Sent::Read { entry: EntryId(1), index: 0 });
        let splices = zone.model.draw(Due::All);
        assert!(!splices.is_empty(), "the row is drawn again");
        assert_eq!(drawn(&zone)[1].2, "attached 0 cargo test loading");
        zone.model.attachment_read(3, "ok\n".into());
        zone.model.draw(Due::All);
        assert_eq!(drawn(&zone)[1].2, "attached 0 cargo test shown");

        zone.replay(&[ready(), added(1, message)]);
        assert_eq!(drawn(&zone)[1].2, "attached 0 cargo test shown", "the same conversation: the row stays open");
        assert_eq!(drawn(&zone).len(), 3);
    }

    /// Task 4.4: a row opens loading and asks for its text, shows it when
    /// it comes, says when it is gone, and drops it on closing; an answer
    /// after the row closed is dropped.
    #[test]
    fn a_rows_text_is_asked_for_shown_and_dropped_on_closing() {
        let mut zone = Zone::ready();
        let link = Attachment::Link { path: "src/a.rs".into(), name: "a.rs".into() };
        zone.feed(&[added(1, user_with("why?", vec![kept(5, "cargo test", 12), link]))]);
        let entry = EntryId(1);
        assert_eq!(*zone.model.row_state(entry, 0), TextState::Closed);
        assert_eq!(zone.model.press_row(entry, 1), RowPress::Nothing, "a link has no text to open");
        assert_eq!(zone.model.press_row(entry, 0), RowPress::Read(5));
        zone.model.sent(9, Sent::Read { entry, index: 0 });
        assert_eq!(*zone.model.row_state(entry, 0), TextState::Loading);
        zone.model.attachment_read(9, "ok\n".into());
        assert_eq!(*zone.model.row_state(entry, 0), TextState::Shown("ok\n".into()));
        assert_eq!(zone.model.press_row(entry, 0), RowPress::Nothing);
        assert_eq!(*zone.model.row_state(entry, 0), TextState::Closed, "closed, its text dropped");

        // Closed before the answer: the answer is dropped.
        assert_eq!(zone.model.press_row(entry, 0), RowPress::Read(5));
        zone.model.sent(10, Sent::Read { entry, index: 0 });
        zone.model.press_row(entry, 0);
        zone.model.attachment_read(10, "late\n".into());
        assert_eq!(*zone.model.row_state(entry, 0), TextState::Closed);

        // Gone from the server.
        zone.model.press_row(entry, 0);
        zone.model.sent(11, Sent::Read { entry, index: 0 });
        zone.model.replied(11, Err(ReplyError::Failed("session 8: attachment 5 is no longer kept".into())));
        assert_eq!(*zone.model.row_state(entry, 0), TextState::Gone(NO_LONGER_KEPT.into()));
        assert_eq!(zone.model.press_row(entry, 0), RowPress::Read(5), "a gone row asks again");
    }
}
