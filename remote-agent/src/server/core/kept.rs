//! The core's attachment store (OpenSpec change `agents-prompt-context`,
//! design decisions 1–4 and 6): the shell around
//! [`attachments::plan`](super::attachments::plan). It gathers a session's
//! store, what its log and queue slot reference and what its directory
//! lists, calls the plan, and runs the steps through the platform's store
//! items (`fcis R1`, `R9`).
//!
//! - **`AGENT_ATTACH`** cleans the text and its name, checks the store's
//!   directories are private (making them), writes the text once and keeps
//!   the upload on its connection until a prompt takes it.
//! - **`AGENT_PROMPT`** with attachments takes its uploads (all or none),
//!   reads the kept texts it names into the session's
//!   [`AgentSession::texts`] and steps the prompt; whatever the step says,
//!   the store then drops what nothing references, so a refused prompt
//!   leaves no file.
//! - **A change** of what a session references (a step with a `Queue` or
//!   `Trimmed` event, a connection closing or detaching with untaken
//!   uploads) removes the unreferenced texts.
//! - **`AGENT_READ_ATTACHMENT`** reads a text the log or slot names.
//! - **Adoption** lists the store again and reads the queued prompt's texts
//!   before the holder's tail can send it.
//! - **Ends** ([`Core::reconcile_stores`], each turn): a session's record
//!   gone writes its marker (or removes the store with 0 days); a new one
//!   is known. Each sweeps its attachments directory, and the default one
//!   is swept once clients are first accepted, after the startup adoption.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use brindle_remote_protocol::SessionId;
use brindle_remote_protocol::client::{self, AttachmentText, PromptAttachment, ToClient};
use brindle_transcript::{ATTACHMENT_TEXT, Attachment, AttachmentId, MAX_ATTACHMENTS, clean};

use super::agents::AgentCommand;
use super::attachments::{self, Found, Refusal, Step, StoreEvent, StoreState, SweepAt, Upload};
use super::clients::error;
use super::{Core, log};
use crate::acp::reduce::attach::{absolute, join, name_of};
use crate::acp::reduce::{AgentSession, Command, Refused, Store};
use crate::server::event_loop::{EventLoop, Token};
use crate::server::platform::{store, sys};

/// What the core keeps of one agent session's store beside its
/// [`AgentSession`]: the store as the session had it (for its end, after
/// the session is gone) and the texts in it, with their sizes.
pub(super) struct Kept {
    store: Store,
    files: BTreeMap<AttachmentId, u64>,
}

/// The longest marker read back.
const MARKER_LIMIT: u64 = 64;

/// Seconds since the epoch, now.
fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// A new agent session's store, from its `SPAWN_AGENT`: its attachments
/// directory (the spawn's, `~` the agent's home, else the core's cache
/// directory's), a fresh name, ids from 1 and its keep time.
pub(super) fn spawn_store(spawn: &client::SpawnAgent) -> Store {
    let home = store::agent_home(&spawn.env);
    let own_home = store::agent_home(&BTreeMap::new());
    let cache = attachments::cache_dir(|name| std::env::var(name).ok(), own_home.as_deref(), store::WINDOWS);
    let root = attachments::attachments_root(
        spawn.attachments_dir.as_deref(),
        cache.as_deref(),
        home.as_deref(),
        store::WINDOWS,
    )
    .unwrap_or_else(|why| {
        log!("an agent session keeps no attached text: {why}");
        String::new()
    });
    let random = sys::random_u64().unwrap_or_else(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos() as u64)
    });
    Store {
        root,
        name: attachments::store_name(now_secs(), random),
        next: 1,
        keep_days: attachments::keep_days(spawn.attachments_keep_days),
        home,
        windows: store::WINDOWS,
    }
}

/// Whether `store`'s directories are private to the user, made so when
/// they can be: `Ok` too when there is nothing to check (no absolute
/// directory), which the plan refuses on its own.
fn private(store: &Store) -> Result<(), String> {
    let Some(dir) = store.dir().filter(|_| absolute(&store.root, store.windows)) else {
        return Ok(());
    };
    store::make_private(Path::new(&store.root))
        .and_then(|()| store::make_private(Path::new(&dir)))
        .map_err(|error| error.to_string())
}

impl<L: EventLoop> Core<L> {
    /// The running agent session `id`, if it is one.
    fn acp(&mut self, id: SessionId) -> Option<&mut AgentSession> {
        self.sessions.get_mut(&id)?.agent()?.running()
    }

    /// What session `id` references: its log's and slot's kept texts and
    /// its untaken uploads.
    fn referenced(&mut self, id: SessionId) -> BTreeSet<AttachmentId> {
        let mut referenced = self.acp(id).map(|acp| acp.referenced()).unwrap_or_default();
        referenced.extend(self.uploads.of_session(id));
        referenced
    }

    /// `AGENT_ATTACH` from `client`: the text kept for the session, as its
    /// upload `upload`, answered `OK`; or the `ERROR` saying why not.
    pub(super) fn client_agent_attach(&mut self, client: Token, attach: client::AgentAttach) {
        let client::AgentAttach {
            req,
            session: id,
            upload,
            name,
            text,
        } = attach;
        if self.attached_session(client, id, Some(req)).is_none() {
            return;
        }
        let refuse = |core: &mut Self, why: String| {
            core.send_client(client, error(Some(req), Some(id), format!("session {id}: {why}")));
        };
        let Some(store) = self.acp(id).map(|acp| acp.store().clone()) else {
            return refuse(self, Refused::NotReady.to_string());
        };
        if let Some(refusal) = self.uploads.refusal(client, upload) {
            return refuse(self, refusal.to_string());
        }
        let (text, name) = (clean(&text), name_of(&name));
        let bytes = text.len() as u64;
        let attached = AttachmentId(store.next.max(1));
        let private = private(&store);
        let none = BTreeMap::new();
        let files = self.kept.get(&id).map_or(&none, |kept| &kept.files);
        let state = StoreState {
            store: &store,
            private: private.as_ref().map(|_| ()).map_err(String::as_str),
            kept: files,
            referenced: &BTreeSet::new(),
        };
        let steps = match attachments::plan(state, StoreEvent::Attach { id: attached, bytes }, now_secs()) {
            Ok(steps) => steps,
            Err(refusal) => return refuse(self, refusal.to_string()),
        };
        for step in steps {
            let done = match step {
                Step::MakeDirs { root, dir } => store::make_private(Path::new(&root))
                    .and_then(|()| store::make_private(Path::new(&dir)))
                    .map_err(|error| Refusal::DirectoryNotPrivate { path: root, why: error.to_string() }.to_string()),
                Step::Write { file, .. } => store::write_once(Path::new(&file), text.as_bytes())
                    .map_err(|error| format!("can't keep the attached text: {error}")),
                Step::Know { .. } | Step::Remove { .. } | Step::Mark { .. } | Step::RemoveDir { .. } => Ok(()),
            };
            if let Err(why) = done {
                return refuse(self, why);
            }
        }
        if let Some(acp) = self.acp(id) {
            acp.store_mut().take_id();
        }
        self.kept
            .entry(id)
            .or_insert_with(|| Kept {
                store: store.clone(),
                files: BTreeMap::new(),
            })
            .files
            .insert(attached, bytes);
        self.uploads.add(
            client,
            upload,
            Upload {
                session: id,
                id: attached,
                name,
                bytes,
            },
        );
        self.send_client(client, ToClient::Ok(client::Request { req }));
    }

    /// `AGENT_PROMPT` from `client`: its uploads taken (all or none) and
    /// their texts read, then stepped as any command; the store then drops
    /// what nothing references, a refused prompt's uploads included.
    pub(super) fn client_agent_prompt(&mut self, client: Token, prompt: client::AgentPrompt) {
        let client::AgentPrompt {
            req,
            session: id,
            text,
            attachments,
        } = prompt;
        let command = |attachments| AgentCommand {
            req: Some(req),
            session: id,
            command: Command::Prompt { text, attachments },
        };
        if attachments.is_empty() {
            return self.client_agent_command(client, command(Vec::new()));
        }
        if self.attached_session(client, id, Some(req)).is_none() {
            return;
        }
        if attachments.len() > MAX_ATTACHMENTS {
            let refused = Refused::TooManyAttachments {
                count: attachments.len(),
            };
            return self.send_client(client, error(Some(req), Some(id), format!("session {id}: {refused}")));
        }
        let uploads: Vec<u64> = attachments
            .iter()
            .filter_map(|attachment| match attachment {
                PromptAttachment::Text { upload } => Some(*upload),
                PromptAttachment::Link { .. } => None,
            })
            .collect();
        let mut taken = match self.uploads.take(client, id, &uploads) {
            Ok(taken) => taken.into_iter(),
            Err(refusal) => {
                self.send_client(client, error(Some(req), Some(id), format!("session {id}: {refusal}")));
                return self.store_changed(id);
            }
        };
        let attachments: Vec<Attachment> = attachments
            .into_iter()
            .filter_map(|attachment| match attachment {
                PromptAttachment::Link { path, name } => Some(Attachment::Link { path, name }),
                PromptAttachment::Text { .. } => taken.next().map(|upload| Attachment::Text {
                    id: upload.id,
                    name: upload.name,
                    bytes: upload.bytes,
                }),
            })
            .collect();
        let ids: Vec<AttachmentId> = attachments.iter().filter_map(Attachment::kept).collect();
        self.read_texts(id, &ids);
        self.client_agent_command(client, command(attachments));
        self.store_changed(id);
    }

    /// Reads the kept texts `ids` into session `id`'s
    /// [`AgentSession::texts`]; one that can't be read stays out, and the
    /// prompt goes without it, with a notice.
    fn read_texts(&mut self, id: SessionId, ids: &[AttachmentId]) {
        let Some(acp) = self.acp(id) else {
            return;
        };
        for attached in ids {
            if acp.texts.contains_key(attached) {
                continue;
            }
            let Some(file) = acp.store().file(*attached) else {
                continue;
            };
            match store::read(Path::new(&file), ATTACHMENT_TEXT as u64) {
                Ok(bytes) => {
                    acp.texts.insert(*attached, String::from_utf8_lossy(&bytes).into_owned());
                }
                Err(error) => log!("session {id}: can't read its attached text {}: {error}", attached.0),
            }
        }
    }

    /// What session `id` references changed: the texts nothing references
    /// are removed, and the read texts kept are its slot's.
    pub(super) fn store_changed(&mut self, id: SessionId) {
        let referenced = self.referenced(id);
        let Some(acp) = self.acp(id) else {
            return;
        };
        let store = acp.store().clone();
        let slot: BTreeSet<AttachmentId> = acp.slot_texts().into_iter().collect();
        acp.texts.retain(|attached, _| slot.contains(attached));
        let Some(kept) = self.kept.get(&id) else {
            return;
        };
        let state = StoreState {
            store: &store,
            private: Ok(()),
            kept: &kept.files,
            referenced: &referenced,
        };
        if let Ok(steps) = attachments::plan(state, StoreEvent::Changed, now_secs()) {
            self.run_steps(id, steps);
        }
    }

    /// Runs the removals, markers and knowledge `steps` say for session
    /// `id`'s store (an attach writes its own).
    fn run_steps(&mut self, id: SessionId, steps: Vec<Step>) {
        for step in steps {
            match step {
                Step::Know { id: attached, bytes } => {
                    if let Some(kept) = self.kept.get_mut(&id) {
                        kept.files.insert(attached, bytes);
                    }
                }
                Step::Remove { id: attached, file } => {
                    if let Err(error) = store::remove(Path::new(&file)) {
                        log!("session {id}: can't remove its attached text {}: {error}", attached.0);
                    }
                    if let Some(kept) = self.kept.get_mut(&id) {
                        kept.files.remove(&attached);
                    }
                }
                Step::Mark { file, text } => {
                    if let Err(error) = store::write_marker(Path::new(&file), &text) {
                        log!("session {id}: can't mark its attachments' keep time in {file}: {error}");
                    }
                }
                Step::RemoveDir { dir } => {
                    if let Err(error) = store::remove_dir(Path::new(&dir)) {
                        log!("session {id}: can't remove its attachments {dir}: {error}");
                    }
                }
                Step::MakeDirs { .. } | Step::Write { .. } => {}
            }
        }
    }

    /// `client`'s untaken uploads (of `session` alone, when given) go: it
    /// closed, was replaced or detached the session.
    pub(super) fn drop_uploads(&mut self, client: Token, session: Option<SessionId>) {
        let gone = self.uploads.drop_client(client, session);
        let sessions: BTreeSet<SessionId> = gone.iter().map(|upload| upload.session).collect();
        for id in sessions {
            self.store_changed(id);
        }
    }

    /// `AGENT_READ_ATTACHMENT` from `client`: a text the session's log or
    /// slot names, as `ATTACHMENT`; or the `ERROR` saying why not.
    pub(super) fn client_agent_read_attachment(&mut self, client: Token, read: client::AgentReadAttachment) {
        let client::AgentReadAttachment { req, session: id, id: number } = read;
        if self.attached_session(client, id, Some(req)).is_none() {
            return;
        }
        let attached = AttachmentId(number);
        let file = self
            .acp(id)
            .filter(|acp| acp.referenced().contains(&attached))
            .and_then(|acp| acp.store().file(attached));
        let Some(file) = file else {
            let message = format!("session {id}: the session keeps no attachment {number}");
            return self.send_client(client, error(Some(req), Some(id), message));
        };
        match store::read(Path::new(&file), ATTACHMENT_TEXT as u64) {
            Ok(bytes) => {
                let text = String::from_utf8_lossy(&bytes).into_owned();
                let answer = AttachmentText { req, session: id, id: number, text };
                self.send_client(client, ToClient::Attachment(answer));
            }
            Err(failure) => {
                log!("session {id}: can't read its attached text {number}: {failure}");
                let message = format!("session {id}: attachment {number} is no longer kept");
                self.send_client(client, error(Some(req), Some(id), message));
            }
        }
    }

    /// Agent session `id` was adopted: its store is known again from what
    /// its directory lists (removing what nothing references), and its
    /// queued prompt's texts are read.
    pub(super) fn store_adopted(&mut self, id: SessionId) {
        let referenced = self.referenced(id);
        let Some(acp) = self.acp(id) else {
            return;
        };
        let store = acp.store().clone();
        let slot = acp.slot_texts();
        let listing = match store.dir() {
            Some(dir) => store::list(Path::new(&dir)).unwrap_or_else(|error| {
                log!("session {id}: can't list its attachments {dir}: {error}");
                Vec::new()
            }),
            None => Vec::new(),
        };
        self.kept.insert(
            id,
            Kept {
                store: store.clone(),
                files: BTreeMap::new(),
            },
        );
        let state = StoreState {
            store: &store,
            private: Ok(()),
            kept: &BTreeMap::new(),
            referenced: &referenced,
        };
        if let Ok(steps) = attachments::plan(state, StoreEvent::Adopted { listing: &listing }, now_secs()) {
            self.run_steps(id, steps);
        }
        self.read_texts(id, &slot);
    }

    /// Each turn: an agent session the store doesn't know yet (a new
    /// spawn) is known, and its attachments directory swept; one whose
    /// record is gone ends (its marker, or its store removed with 0 days),
    /// and its directory is swept. The default directory is swept once,
    /// when clients are first accepted.
    pub(super) fn reconcile_stores(&mut self) {
        let fresh: Vec<(SessionId, Store)> = self
            .sessions
            .iter_mut()
            .filter(|(id, _)| !self.kept.contains_key(id))
            .filter_map(|(id, session)| Some((*id, session.agent()?.running()?.store().clone())))
            .collect();
        for (id, store) in fresh {
            let root = store.root.clone();
            self.kept.insert(id, Kept { store, files: BTreeMap::new() });
            self.sweep(SweepAt::Session { root: &root });
        }
        let gone: Vec<SessionId> = self
            .kept
            .keys()
            .filter(|id| !self.sessions.contains_key(id))
            .copied()
            .collect();
        for id in gone {
            let Some(kept) = self.kept.remove(&id) else {
                continue;
            };
            self.uploads.drop_session(id);
            let state = StoreState {
                store: &kept.store,
                private: Ok(()),
                kept: &kept.files,
                referenced: &BTreeSet::new(),
            };
            if let Ok(steps) = attachments::plan(state, StoreEvent::Ended, now_secs()) {
                self.run_steps(id, steps);
            }
            self.sweep(SweepAt::Session { root: &kept.store.root });
        }
        if !self.swept && self.lifecycle.accepting() {
            self.swept = true;
            let home = store::agent_home(&BTreeMap::new());
            let cache = attachments::cache_dir(|name| std::env::var(name).ok(), home.as_deref(), store::WINDOWS);
            let default = cache.map(|cache| join(&cache, attachments::ATTACHMENTS, store::WINDOWS));
            self.sweep(SweepAt::CoreStart { default: default.as_deref() });
        }
    }

    /// Removes the store directories in the attachments directory a sweep
    /// `at` lists whose time has passed, never a live session's
    /// ([`attachments::sweep`]).
    fn sweep(&mut self, at: SweepAt<'_>) {
        let Some(root) = attachments::sweep_root(at).filter(|root| absolute(root, store::WINDOWS)) else {
            return;
        };
        let Ok(listing) = store::list(Path::new(root)) else {
            return;
        };
        let found: Vec<Found> = listing
            .iter()
            .filter(|listed| listed.dir && attachments::is_store_name(&listed.name))
            .map(|listed| {
                let dir = join(root, &listed.name, store::WINDOWS);
                let inside = store::list(Path::new(&dir)).unwrap_or_default();
                let marker = inside
                    .iter()
                    .find(|file| file.name == attachments::MARKER && !file.dir)
                    .and_then(|_| store::read(Path::new(&join(&dir, attachments::MARKER, store::WINDOWS)), MARKER_LIMIT).ok())
                    .and_then(|bytes| attachments::parse_marker(&String::from_utf8_lossy(&bytes)));
                let newest = inside.iter().filter_map(|file| file.modified).chain(listed.modified).max();
                Found { dir, marker, newest }
            })
            .collect();
        let live: BTreeSet<String> = self.kept.values().filter_map(|kept| kept.store.dir()).collect();
        for step in attachments::sweep(&found, &live, now_secs()) {
            if let Step::RemoveDir { dir } = step {
                match store::remove_dir(Path::new(&dir)) {
                    Ok(()) => log!("removed the attachments {dir}: their keep time passed"),
                    Err(error) => log!("can't remove the attachments {dir}: {error}"),
                }
            }
        }
    }
}
