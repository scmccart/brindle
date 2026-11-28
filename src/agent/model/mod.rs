//! The chat model of an agent zone (design.md decision 8): what the zone
//! shows and decides, on the UI thread, from the transcript the reader
//! thread folds ([`crate::agent::shared`]) and the session's life events.
//!
//! GPUI-free: plain values in, plain values out (`fcis R1`, `R5`); the
//! blocks hold `SharedString`s so the view draws them without copying.
//! The view owns the prompt input's text: nothing here writes it, apart
//! from a launch's first prompt left as a draft when it can't be sent
//! ([`First::Draft`]) and a refused send's text put back
//! ([`Replied::SetDraft`]); nothing from the queue slot ever reaches it.

use std::collections::BTreeMap;

use crate::FirstPrompt;
use crate::agent::commands::Completion;
use crate::agent::login::AgentCommand;
use crate::agent::markdown::Part;
use crate::agent::shared::{Changes, StatusFrom};
use crate::agent::tool::{Shown, ToolPolicy};
use crate::agent::transcript::{
    Activity, AgentState, Conversation, Entry, EntryId, SessionOption, Slot, SlashCommand, Transcript, TurnState, Window,
};
use crate::terminal::status::State;

use attention::AgentNotice;
use blocks::{RenderedText, render_entry, text_items, working_splice};
use choosing::Choosing;
use ledger::Uncertain;
use paging::Paging;
use phase::SessionLife;
use prompt::FirstState;
use queue::queue_row;

mod attention;
mod blocks;
mod choosing;
mod dump;
mod ledger;
mod login;
mod options;
mod paging;
mod phase;
mod prompt;
mod queue;
mod recovery;
#[cfg(test)]
mod harness;

pub use attention::WorkingRow;
pub use blocks::{Block, BlockBody, ShownPart, Splice};
pub use choosing::{Choose, LaunchChoice, Leaving, ListDone, ListFor, ListerState, OnChoosing, ZoneChoice};
pub use ledger::{LINK_DOWN, NotSent, Replied, ReplyError, Sent};
pub use login::{AfterLogin, CancelLogin, LoginChoice, LoginRow, LoginZoneEnd, MethodChoice};
pub use options::KeyOutcome;
pub use paging::{Earlier, Fetch};
pub use phase::{AgentPhase, Link, PromptStatus, StartFailure, agent_start};
pub use prompt::{First, Submit};
#[cfg(test)]
pub use phase::DisabledWhy;
#[cfg(test)]
pub use prompt::{ALREADY_QUEUED, UNSENT};
pub use queue::{QueueRow, Resend, Unqueue};
pub use recovery::{NoOffer, Offer, ProfileFound};

/// The client protocol version that carries agent sessions.
pub const AGENT_PROTOCOL: u32 = 4;

/// A request's id on the session's connection (the client protocol's
/// `req`), which its `OK` or `ERROR` names.
pub type RequestId = u64;

/// How long a user message's block grows before it is cut at the next
/// line break, so a long pasted prompt lays out in pieces. Agent messages
/// are cut at their top-level Markdown blocks instead.
pub const BLOCK_LIMIT: usize = 4 * 1024;

/// Whether the session's thread has handed the zone the running session
/// (decision 2): until it has, nothing can be sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionHandle {
    Starting,
    Running,
}

/// What a change to the model asks of the view.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Absorbed {
    /// For the conversation's list, in order.
    pub splices: Vec<Splice>,
    /// The shown status, when it changed.
    pub status: Option<Option<State>>,
    pub notice: Option<AgentNotice>,
    pub title_changed: bool,
    pub queue_changed: bool,
    /// The launch's first prompt, when it is sent or left as a draft now.
    pub first: Option<First>,
    /// The launch's agent is choosing for the first time: what its launch
    /// asked for (OpenSpec change `agents-session-list`).
    pub choosing: Option<OnChoosing>,
    /// A prompt whose answer was lost with the link, settled now
    /// (OpenSpec change `agents-client-state-fixes`, decision 5).
    pub replied: Option<Replied>,
}

/// How much Markdown the zone rendered (OpenSpec change
/// `agents-absorb-cost`, decision 4): each render of an agent message, a
/// thought or a tool's text item, and the bytes each was given.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RenderStats {
    pub renders: u64,
    pub bytes: u64,
}

impl RenderStats {
    /// One render of `source`.
    fn rendered(&mut self, source: &str) {
        self.renders += 1;
        self.bytes += source.len() as u64;
    }

    /// One render of `entry`, when it is Markdown.
    fn rendered_entry(&mut self, entry: &Entry) {
        match entry {
            Entry::Agent { text } | Entry::Thought { text } => self.rendered(text),
            Entry::User { .. } | Entry::Tool { .. } | Entry::Permission { .. } | Entry::Notice { .. } => {}
        }
    }
}

/// What a change compares against.
struct Before {
    status: Option<State>,
    title: String,
    queue: QueueRow,
    /// Whether the working row showed.
    working: bool,
}

/// An agent zone's model.
#[derive(Debug, Clone)]
pub struct ChatModel {
    /// The profile's name, the title until the agent names the
    /// conversation.
    profile: String,
    /// The agent's own name, from the last `Ready` that gave one: the
    /// failure texts' (`agent_label`), never the title.
    agent_name: Option<String>,
    phase: AgentPhase,
    link: Link,
    life: SessionLife,
    turn: TurnState,
    activity: Activity,
    slot: Slot,
    queue: QueueRow,
    /// The zone's copy of the kept entries, in id order.
    entries: Vec<(EntryId, Entry)>,
    /// The rendered parts of the agent messages and thoughts in `entries`
    /// (`types R10`): refreshed when one is appended to or replaced,
    /// rebuilt on a reset, dropped with a trimmed entry.
    rendered: BTreeMap<EntryId, Vec<Part>>,
    /// The renderer panicked once (logged once).
    render_failed: bool,
    /// How much the zone rendered, logged when it is left.
    stats: RenderStats,
    blocks: Vec<Block>,
    /// Thoughts the user opened or closed.
    toggled: BTreeMap<EntryId, bool>,
    /// The thought drawn open on its own: the latest entry while a turn
    /// runs.
    auto_open: Option<EntryId>,
    first: FirstState,
    /// The requests on their way, by id: the only record of a fetch in
    /// flight ([`Self::earlier`]).
    pending: BTreeMap<RequestId, Sent>,
    /// Whether the session's thread handed over the running session
    /// ([`Self::session_running`]); a respawn starts it over.
    handle: SessionHandle,
    /// A prompt whose answer was lost with the link, until the reconnect's
    /// replay settles it.
    uncertain: Option<Uncertain>,
    refusal: Option<String>,
    /// The user typed after a `done` or `error` arrived.
    seen: bool,
    /// A restore's note, shown above the conversation.
    note: Option<String>,
    /// The endpoint's note, shown below a restore's: the agent got the SSH
    /// session's environment because the login shell's wasn't read.
    endpoint_note: Option<String>,
    /// The agent's conversation as the zone last knew it: the transcript's,
    /// kept while a new session's transcript has none yet.
    conversation: Option<Conversation>,
    /// The agent profile's command, which a terminal login runs again.
    command: AgentCommand,
    /// What the zone says beside the login row: a refused `AGENT_LOGIN`,
    /// or how a terminal login's program ended.
    login_note: Option<String>,
    /// A terminal login running in the zone beside: its method's id and
    /// name.
    terminal_login: Option<(String, String)>,
    /// The transcript's window: the zone holds the entries from its
    /// `from` on.
    window: Option<Window>,
    /// The entries before the window, as last read from it.
    paging: Paging,
    /// How tool call rows open on their own (`[agent]`, decision 10).
    policy: ToolPolicy,
    /// Tool call rows the user opened or closed.
    by_hand: BTreeMap<EntryId, Shown>,
    /// The rendered Markdown of tool calls' text items, by entry and item,
    /// with the source it was rendered from.
    texts: BTreeMap<(EntryId, usize), RenderedText>,
    /// The latest user message held: a tool call after it can be running.
    last_user: Option<EntryId>,
    /// Each pending permission's tool entry, when the zone holds it (the
    /// newest with the call the permission names, before it).
    permission_tools: BTreeMap<EntryId, EntryId>,
    /// What the zone knows of choosing (OpenSpec change
    /// `agents-session-list`).
    choosing: Choosing,
    /// The agent's slash commands and the session's options, as the
    /// transcript last had them (OpenSpec change `agents-commands`).
    commands: Vec<SlashCommand>,
    options: Vec<SessionOption>,
    /// The prompt input's text and cursor as last typed, which the
    /// completion list follows.
    draft: (String, usize),
    /// The completion list while it shows.
    completion: Option<Completion>,
    /// Escape closed the list: it stays closed until the text stops being
    /// a command word.
    dismissed: bool,
}

fn running(turn: &TurnState) -> bool {
    match turn {
        TurnState::Working | TurnState::Blocked | TurnState::Cancelling => true,
        TurnState::Idle | TurnState::Ended { .. } | TurnState::Failed { .. } => false,
    }
}

impl ChatModel {
    /// A zone of `profile`'s agent, with a launch's `first` prompt (only a
    /// launch has one: a restored or attached zone never does).
    pub fn new(profile: impl Into<String>, first: Option<FirstPrompt>) -> Self {
        Self {
            profile: profile.into(),
            agent_name: None,
            phase: AgentPhase::Connecting,
            link: Link::Up,
            life: SessionLife::Open,
            turn: TurnState::Idle,
            activity: Activity::Idle,
            slot: Slot::Empty,
            queue: QueueRow::None,
            entries: Vec::new(),
            rendered: BTreeMap::new(),
            render_failed: false,
            stats: RenderStats::default(),
            blocks: Vec::new(),
            toggled: BTreeMap::new(),
            auto_open: None,
            first: first.map_or(FirstState::None, FirstState::Waiting),
            pending: BTreeMap::new(),
            handle: SessionHandle::Starting,
            uncertain: None,
            refusal: None,
            seen: false,
            note: None,
            endpoint_note: None,
            conversation: None,
            command: AgentCommand::default(),
            login_note: None,
            terminal_login: None,
            window: None,
            paging: Paging::None,
            policy: ToolPolicy::default(),
            by_hand: BTreeMap::new(),
            texts: BTreeMap::new(),
            last_user: None,
            permission_tools: BTreeMap::new(),
            choosing: Choosing::default(),
            commands: Vec::new(),
            options: Vec::new(),
            draft: (String::new(), 0),
            completion: None,
            dismissed: false,
        }
    }

    /// The agent profile's command, as the session was started with it.
    pub fn set_command(&mut self, command: AgentCommand) {
        self.command = command;
    }

    /// A restore's note (a recorded profile gone from the config).
    pub fn set_note(&mut self, note: impl Into<String>) {
        self.note = Some(note.into());
    }

    /// The profile was renamed.
    pub fn set_profile_name(&mut self, profile: impl Into<String>) {
        self.profile = profile.into();
    }

    /// The zone's profile's name.
    pub fn profile_name(&self) -> &str {
        &self.profile
    }

    pub fn phase(&self) -> &AgentPhase {
        &self.phase
    }

    /// The link, which only tests read: the entity's replay end goes through
    /// [`replay_ended`](Self::replay_ended).
    #[cfg(test)]
    pub fn link(&self) -> Link {
        self.link
    }

    #[cfg(test)]
    pub fn life(&self) -> SessionLife {
        self.life
    }

    #[cfg(test)]
    pub fn turn(&self) -> &TurnState {
        &self.turn
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    pub fn queue(&self) -> &QueueRow {
        &self.queue
    }

    /// A restore's note alone; the view shows [`ChatModel::notes`].
    #[cfg(test)]
    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// The agent's start fell back to the endpoint's SSH environment, as
    /// `text` says. The latest start's word replaces an earlier one's.
    pub fn set_endpoint_note(&mut self, text: impl Into<String>) {
        self.endpoint_note = Some(text.into());
    }

    /// The notes shown above the conversation, one line each: a restore's,
    /// then the endpoint's.
    pub fn notes(&self) -> impl Iterator<Item = &str> {
        self.note.as_deref().into_iter().chain(self.endpoint_note.as_deref())
    }

    /// The refusal shown beside the prompt input.
    pub fn refusal(&self) -> Option<&str> {
        self.refusal.as_deref()
    }

    /// The zone's title (OpenSpec change `agents-session-title`, decision
    /// 5): the title the agent gave the conversation the zone last knew,
    /// else the profile's name. The agent's own name never titles it.
    pub fn title(&self) -> String {
        self.conversation
            .as_ref()
            .and_then(|conversation| conversation.title.clone())
            .unwrap_or_else(|| self.profile.clone())
    }

    /// The agent's name for the failure texts' `{agent}`: its name from the
    /// last `Ready` that gave one (kept after it exits or fails), else the
    /// profile's.
    pub fn agent_label(&self) -> String {
        self.agent_name.clone().unwrap_or_else(|| self.profile.clone())
    }

    /// How much the zone has rendered so far.
    #[cfg(test)]
    pub fn render_stats(&self) -> RenderStats {
        self.stats
    }

    fn before(&self) -> Before {
        Before { status: self.shown_status(), title: self.title(), queue: self.queue.clone(), working: self.working_row().is_some() }
    }

    /// Folds what changed in the shared transcript into the zone's copy.
    pub fn absorb(&mut self, transcript: &Transcript, changes: Changes) -> Absorbed {
        let before = self.before();
        let mut splices = Vec::new();
        if changes.reset {
            // A fetch in flight belongs to the copy that is gone.
            self.pending.retain(|_, sent| *sent != Sent::Fetch);
            self.entries = transcript.entries().map(|(id, entry)| (id, entry.clone())).collect();
            self.auto_open = None;
            self.rendered.clear();
            self.texts.clear();
            let renders: Vec<_> =
                self.entries.iter().map(|(id, entry)| (*id, render_entry(entry), text_items(entry))).collect();
            for (_, entry) in &self.entries {
                self.stats.rendered_entry(entry);
            }
            for (id, render, items) in renders {
                self.keep_render(id, render);
                self.keep_texts(id, items);
            }
            // The turn decides which tool calls run, so it is read first.
            self.turn = transcript.turn().clone();
            self.refresh_facts();
            let old = self.blocks.len();
            self.blocks = self.entries.iter().flat_map(|(id, entry)| self.draw(*id, entry, false)).collect();
            splices.push(Splice { range: 0..old, count: self.blocks.len() });
        } else {
            if changes.prepended.is_some_and(|added| added > 0) {
                splices.extend(self.prepend_entries(transcript, changes.added_from));
            }
            if let Some(through) = changes.trimmed_through {
                self.entries.retain(|(id, _)| *id > through);
                self.rendered.retain(|id, _| *id > through);
                self.texts.retain(|(id, _), _| *id > through);
                self.by_hand.retain(|id, _| *id > through);
                self.toggled.retain(|id, _| *id > through);
                let gone = self.blocks.partition_point(|block| block.entry <= through);
                self.blocks.drain(..gone);
                if gone > 0 {
                    splices.push(Splice { range: 0..gone, count: 0 });
                }
            }
            for (id, dirty) in changes.changed {
                splices.extend(self.update_entry(transcript, id, dirty));
            }
            if let Some(from) = changes.added_from {
                let at = self.blocks.len();
                for (id, entry) in transcript.entries().filter(|(id, _)| *id >= from) {
                    if self.entries.last().is_some_and(|(last, _)| *last >= id) {
                        continue;
                    }
                    self.entries.push((id, entry.clone()));
                    if matches!(entry, Entry::User { .. }) {
                        self.last_user = Some(id);
                    }
                    self.stats.rendered_entry(entry);
                    self.keep_render(id, render_entry(entry));
                    self.keep_texts(id, text_items(entry));
                    let fresh = self.draw(id, entry, false);
                    self.blocks.extend(fresh);
                }
                if self.blocks.len() > at {
                    splices.push(Splice { range: at..at, count: self.blocks.len() - at });
                }
            }
        }
        if changes.reset || changes.turn_or_agent || changes.added_from.is_some() {
            self.turn = transcript.turn().clone();
            let activity = transcript.activity();
            if activity != self.activity {
                self.seen = false;
            }
            self.activity = activity;
            match &self.phase {
                AgentPhase::Connecting | AgentPhase::Live(_) => self.phase = AgentPhase::Live(transcript.agent().clone()),
                AgentPhase::NotStarted(_) | AgentPhase::Lost { .. } | AgentPhase::Broken { .. } => {}
            }
            if let AgentPhase::Live(AgentState::Ready { name: Some(name), .. }) = &self.phase {
                self.agent_name = Some(name.clone());
            }
            if let Some(conversation) = transcript.conversation() {
                self.conversation = Some(conversation.clone());
            }
        }
        if changes.reset || changes.slot_changed {
            self.slot = transcript.slot().clone();
        }
        if changes.reset || changes.lists_changed {
            self.commands = transcript.commands().to_vec();
            self.options = transcript.options().to_vec();
            self.refresh_completion();
        }
        splices.extend(self.update_thoughts(changes.reset));
        splices.extend(self.refresh_rows());
        self.follow_window(transcript.window(), changes.reset, changes.prepended.is_some());
        // The reconnect's replay says whether a prompt whose answer was
        // lost with the link was taken (decision 5).
        let replied = if changes.reset { self.uncertain.take().map(|uncertain| self.settle_uncertain(uncertain, None)) } else { None };
        Absorbed { replied, ..self.finish(before, splices, changes.status_from) }
    }

    /// The session's thread handed over the running session
    /// (`AgentEvent::SessionRunning`, decision 2): requests can go, and a
    /// first prompt or a launch's list waiting for it goes now.
    pub fn session_running(&mut self) -> Absorbed {
        self.life_event(|model| model.handle = SessionHandle::Running)
    }

    /// The link to the endpoint went down or came back.
    pub fn set_link(&mut self, link: Link) -> Absorbed {
        self.life_event(|model| model.link = link)
    }

    /// A replay ended: it ends a reconnect, bringing the link back up; a
    /// replay on a link already up (an attach's) changes nothing.
    pub fn replay_ended(&mut self) -> Option<Absorbed> {
        match self.link {
            Link::Reconnecting => Some(self.set_link(Link::Up)),
            Link::Up => None,
        }
    }

    fn life_event(&mut self, change: impl FnOnce(&mut Self)) -> Absorbed {
        let before = self.before();
        change(self);
        // A tool row's state can follow the phase and the session's life.
        let splices = self.refresh_rows();
        self.finish(before, splices, StatusFrom::Unchanged)
    }

    /// What every change ends with, `status_from` saying where the newest
    /// turn or agent-state event of the change was folded (decision 8): one
    /// from a replay raises nothing, sends no first prompt and starts no
    /// launch's choice.
    fn finish(&mut self, before: Before, mut splices: Vec<Splice>, status_from: StatusFrom) -> Absorbed {
        let replayed = status_from == StatusFrom::Replay;
        let running = self.handle == SessionHandle::Running;
        self.queue = queue_row(&self.slot, &self.phase, self.life);
        // The note beside the login row goes with it; a terminal login
        // whose agent moved on (it exited, the session ended) is forgotten.
        if !self.needs_login() {
            self.login_note = None;
            self.terminal_login = None;
        }
        let status = self.shown_status();
        let changed = status != before.status;
        // The working row is the list's item after the blocks: its insert or
        // removal comes after the blocks' splices, at the count they left
        // (OpenSpec change `agents-zone-focus-and-scroll`, decision 5).
        splices.extend(working_splice(before.working, self.working_row().is_some(), self.blocks.len()));
        // A conversation started: the session is an ordinary one now.
        if matches!(self.phase, AgentPhase::Live(AgentState::Ready { .. })) {
            self.choosing.started = false;
            self.choosing.note = None;
        }
        let choosing = match self.choosing.launch {
            // A launch's list waits for the session too (decision 2).
            Some(launch) if self.is_choosing() && !replayed && running => {
                self.choosing.launch = None;
                Some(match launch {
                    LaunchChoice::Last => OnChoosing::ListLatest,
                    LaunchChoice::List => OnChoosing::OpenList,
                })
            }
            Some(_) | None => None,
        };
        Absorbed {
            choosing,
            splices,
            status: changed.then_some(status),
            notice: if changed { self.notice(before.status, status, replayed) } else { None },
            title_changed: self.title() != before.title,
            queue_changed: self.queue != before.queue,
            first: self.settle_first(status_from),
            replied: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::harness::*;
    use super::*;

    /// `agents-session-title` decision 5: the title is the conversation's
    /// title, else the profile's name; the agent's own name labels only
    /// the failure texts.
    #[test]
    fn the_title_is_the_conversations_else_the_profiles() {
        use crate::agent::transcript::Continuation;
        let mut zone = Zone::new(None);
        assert_eq!(zone.model.title(), "Fake");
        assert!(!zone.feed(&[conversation_event(Continuation::Resume), ready()]).title_changed);
        assert_eq!(zone.model.title(), "Fake", "the agent's own name doesn't title the zone");
        assert_eq!(zone.model.agent_label(), "Fake Agent");
        let absorbed = zone.feed(&[titled(Some("Hello.txt file creation"))]);
        assert!(absorbed.title_changed);
        assert_eq!(zone.model.title(), "Hello.txt file creation");
        assert!(!zone.feed(&[titled(Some("Hello.txt file creation"))]).title_changed, "the same title");
        // An exit keeps the conversation, and with it the title.
        zone.feed(&[agent_state(AgentState::Exited { code: Some(0), stderr: String::new() })]);
        assert_eq!(zone.model.title(), "Hello.txt file creation");
        assert_eq!(zone.model.agent_label(), "Fake Agent");
        let absorbed = zone.feed(&[titled(None)]);
        assert!(absorbed.title_changed);
        assert_eq!(zone.model.title(), "Fake", "a cleared title brings the profile's name back");

        // A profile rename retitles an untitled zone, and not a titled one.
        let mut renamed = Zone::ready();
        renamed.model.set_profile_name("Renamed");
        assert_eq!(renamed.model.title(), "Renamed");
        assert_eq!(renamed.model.agent_label(), "Fake Agent");
        renamed.feed(&[titled(Some("Named"))]);
        renamed.model.set_profile_name("Again");
        assert_eq!(renamed.model.title(), "Named");
        let unnamed = Zone::new(None);
        assert_eq!(unnamed.model.agent_label(), "Fake", "no Ready yet: the profile's");
    }

    /// Decision 5: a respawn into a new server session shows the old
    /// conversation's title until the new session's (untitled)
    /// conversation arrives, then the profile's name.
    #[test]
    fn a_respawn_falls_back_to_the_profiles_name_once_its_conversation_arrives() {
        use crate::agent::transcript::Continuation;
        let mut zone = Zone::new(None);
        zone.feed(&[conversation_event(Continuation::Resume), ready()]);
        zone.feed(&[titled(Some("Old"))]);
        zone.feed(&[agent_state(AgentState::Exited { code: Some(1), stderr: String::new() })]);
        // The new session's log, replayed from the start: no conversation yet.
        let absorbed = zone.replay(&[agent_state(AgentState::Starting)]);
        assert!(!absorbed.title_changed);
        assert_eq!(zone.model.title(), "Old", "kept while the new log has no conversation");
        let absorbed = zone.feed(&[conversation_event(Continuation::Resume), ready()]);
        assert!(absorbed.title_changed);
        assert_eq!(zone.model.title(), "Fake");
    }

    /// OpenSpec change `agents-absorb-cost` decision 5's guard: the frames
    /// one drained update takes render a growing message once, however
    /// many of them appended to it.
    #[test]
    #[ignore = "agents-absorb-cost 5.4"]
    fn a_batch_of_appends_absorbed_in_one_update_renders_the_message_once() {
        use crate::agent::transcript::Event;
        let mut zone = Zone::ready();
        zone.feed(&[added(1, user("hi")), added(2, agent("Hel"))]);
        let before = zone.model.render_stats().renders;
        let append = |text: &str| [Event::Appended { id: EntryId(2), text: text.into() }];
        zone.feed_batch(&[&append("lo"), &append(" the"), &append("re")]);
        assert_eq!(zone.model.render_stats().renders - before, 1);
        assert_eq!(zone.model.message_text(EntryId(2)), Some("Hello there"));
    }

    /// Decision 8's table on a fixed conversation: a tool's text item is
    /// held once beside its entry and once rendered, however its row is
    /// drawn.
    #[test]
    #[ignore = "agents-absorb-cost 5.7"]
    fn held_text_counts_the_copies() {
        use crate::agent::tool::RowState;
        use crate::agent::transcript::ToolItem;
        let mut zone = Zone::ready();
        let tool = in_progress("c1", vec![ToolItem::Text { text: "abcdefgh".repeat(512), cut: false }]);
        zone.feed(&[
            added(1, user(&"u".repeat(2048))),
            turn(TurnState::Working),
            added(2, agent(&"a".repeat(10 * 1024))),
            added(3, tool),
        ]);
        assert_eq!(row(&zone.model, 3).0, RowState::Open);
        let held = zone.model.held_text();
        assert_eq!(held.entries, 2048 + 10 * 1024 + 4096, "{held:?}");
        assert_eq!((held.user_parts, held.rendered, held.text_renders), (2048, 10 * 1024, 4096), "{held:?}");
        assert_eq!((held.text_sources, held.detail_sources), (0, 0), "the tool text held once beside its entry: {held:?}");
    }

    /// The end of a replay brings a reconnecting link back up, and does
    /// nothing on a link that is up.
    #[test]
    fn a_replay_ends_a_reconnect_and_nothing_else() {
        let mut zone = Zone::ready();
        assert_eq!(zone.model.replay_ended(), None, "an attach's replay");
        assert_eq!(zone.model.link(), Link::Up);
        zone.model.set_link(Link::Reconnecting);
        assert!(zone.model.replay_ended().is_some(), "a reconnect's replay");
        assert_eq!(zone.model.link(), Link::Up);
        assert_eq!(zone.model.replay_ended(), None);
    }
}
