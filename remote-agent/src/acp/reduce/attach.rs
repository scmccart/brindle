//! A prompt's attachments in the reducer (OpenSpec change
//! `agents-prompt-context`, design decisions 5 and 6): where the session
//! keeps attached text, how a link's path is resolved, and the parts a
//! prompt is sent as.
//!
//! The store's own decisions (ids, bounds, what to remove and when) are the
//! core's (`server/core/attachments.rs`); here is what the session holds of
//! it, checkpointed with the session ([`Store`]), and the kept texts the
//! core's shell read for the prompts the session may send
//! ([`AgentSession::texts`](super::AgentSession)), which a checkpoint never
//! holds.

use std::collections::BTreeMap;

use brindle_transcript::{Attachment, AttachmentId, MAX_ATTACHMENT_NAME, MAX_PATH, clean};

use super::super::forms::{PromptPart, PromptSupport};
use super::super::uri;
use super::Refused;

/// The largest kept text sent inside a prompt, to an agent that takes
/// embedded context (decision 2); a larger one is a link to its file.
pub const EMBED_LIMIT: u64 = 64 * 1024;

/// Where the session keeps attached text on the agent's host, and what a
/// link's path is resolved with (decision 1, 5). Small and bounded, so
/// every checkpoint record holds it whole (`agents-checkpoint-deltas`
/// decision 4); a checkpoint from before it reads as the default, a
/// session with no store.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Store {
    /// The attachments directory (absolute), with the store's directory
    /// `name` in it; empty while the session has none.
    pub root: String,
    /// The store's directory name, made once by the core's shell from the
    /// date and random bytes.
    pub name: String,
    /// The next attachment id, from 1, never reused.
    pub next: u64,
    /// How many days the store stays after the session ends.
    pub keep_days: u32,
    /// The home a link's `~` stands for, on the agent's host.
    pub home: Option<String>,
    /// Whether the agent's host reads paths as Windows does.
    pub windows: bool,
}

impl Store {
    /// The session's store directory, once it has one.
    pub fn dir(&self) -> Option<String> {
        (!self.root.is_empty() && !self.name.is_empty()).then(|| join(&self.root, &self.name, self.windows))
    }

    /// The file that keeps text `id`.
    pub fn file(&self, id: AttachmentId) -> Option<String> {
        self.dir().map(|dir| join(&dir, &id.0.to_string(), self.windows))
    }

    /// Takes the next id.
    pub fn take_id(&mut self) -> AttachmentId {
        let id = AttachmentId(self.next.max(1));
        self.next = id.0 + 1;
        id
    }
}

/// `name` in directory `dir`.
pub fn join(dir: &str, name: &str, windows: bool) -> String {
    let separator = if windows { '\\' } else { '/' };
    let dir = dir.trim_end_matches(|c| c == separator || (windows && c == '/'));
    format!("{dir}{separator}{name}")
}

/// Whether `path` is absolute on the agent's host.
pub fn absolute(path: &str, windows: bool) -> bool {
    if !windows {
        return path.starts_with('/');
    }
    let bytes = path.as_bytes();
    path.starts_with("\\\\")
        || path.starts_with("//")
        || (bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':' && matches!(bytes[2], b'\\' | b'/'))
}

/// A link's path as the agent is given it (decision 5): `~` and `~/…`
/// with the session's home, a relative path against the session's
/// directory, an absolute one as it is. `~` with no home stays as given.
pub fn resolve(path: &str, cwd: &str, home: Option<&str>, windows: bool) -> String {
    let tilde = path == "~" || path.starts_with("~/") || (windows && path.starts_with("~\\"));
    if tilde {
        return match home {
            Some(home) if path.len() > 2 => join(home, &path[2..], windows),
            Some(home) => home.to_owned(),
            None => path.to_owned(),
        };
    }
    if absolute(path, windows) {
        path.to_owned()
    } else {
        join(cwd, path, windows)
    }
}

/// A prompt part the reducer builds and hands to the dialect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Part {
    Text(String),
    Link { uri: String, name: String },
    Embedded { uri: String, name: String, text: String },
}

impl Part {
    pub fn form(&self) -> PromptPart<'_> {
        match self {
            Self::Text(text) => PromptPart::Text(text),
            Self::Link { uri, name } => PromptPart::Link { uri, name },
            Self::Embedded { uri, name, text } => PromptPart::Embedded { uri, name, text },
        }
    }
}

/// A prompt's parts (decision 6) and the names of the kept texts left out,
/// which were missing: the text first, then each attachment in order; a
/// link by its resolved path's URI; a kept text embedded when the agent
/// takes it and it is at most [`EMBED_LIMIT`], else a link to its file.
pub fn parts(
    text: &str,
    attachments: &[Attachment],
    texts: &BTreeMap<AttachmentId, String>,
    store: &Store,
    cwd: &str,
    support: PromptSupport,
) -> (Vec<Part>, Vec<String>) {
    let mut parts = vec![Part::Text(text.to_owned())];
    let mut missing = Vec::new();
    for attachment in attachments {
        match attachment {
            Attachment::Link { path, name } => {
                let path = resolve(path, cwd, store.home.as_deref(), store.windows);
                parts.push(Part::Link {
                    uri: uri::file(&path, store.windows),
                    name: name.clone(),
                });
            }
            Attachment::Text { id, name, bytes } => {
                let (Some(file), Some(kept)) = (store.file(*id), texts.get(id)) else {
                    missing.push(name.clone());
                    continue;
                };
                let uri = uri::file(&file, store.windows);
                let part = if support.embedded && *bytes <= EMBED_LIMIT {
                    Part::Embedded {
                        uri,
                        name: name.clone(),
                        text: kept.clone(),
                    }
                } else {
                    Part::Link { uri, name: name.clone() }
                };
                parts.push(part);
            }
        }
    }
    (parts, missing)
}

/// A prompt's attachments cleaned as its text is (decision 2): each name
/// cleaned and cut to [`MAX_ATTACHMENT_NAME`] bytes, a link's path cleaned
/// and refused past [`MAX_PATH`], since a cut path would name another file.
pub fn cleaned(attachments: Vec<Attachment>) -> Result<Vec<Attachment>, Refused> {
    attachments
        .into_iter()
        .map(|attachment| match attachment {
            Attachment::Link { path, name } => {
                let path = clean(&path);
                if path.len() > MAX_PATH {
                    return Err(Refused::PathTooLong { len: path.len() });
                }
                Ok(Attachment::Link { path, name: name_of(&name) })
            }
            Attachment::Text { id, name, bytes } => Ok(Attachment::Text {
                id,
                name: name_of(&name),
                bytes,
            }),
        })
        .collect()
}

/// `name` cleaned and cut at a character boundary to
/// [`MAX_ATTACHMENT_NAME`] bytes.
pub fn name_of(name: &str) -> String {
    let mut name = clean(name);
    if name.len() > MAX_ATTACHMENT_NAME {
        let mut end = MAX_ATTACHMENT_NAME;
        while !name.is_char_boundary(end) {
            end -= 1;
        }
        name.truncate(end);
    }
    name
}

/// The kept texts `attachments` name.
pub fn kept_ids(attachments: &[Attachment]) -> impl Iterator<Item = AttachmentId> + '_ {
    attachments.iter().filter_map(Attachment::kept)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        Store {
            root: "/home/u/.cache/brindle/attachments".into(),
            name: "20261128-ab12".into(),
            next: 1,
            keep_days: 14,
            home: Some("/home/u".into()),
            windows: false,
        }
    }

    fn text(id: u64, name: &str, bytes: u64) -> Attachment {
        Attachment::Text {
            id: AttachmentId(id),
            name: name.into(),
            bytes,
        }
    }

    fn link(path: &str) -> Attachment {
        Attachment::Link {
            path: path.into(),
            name: "n".into(),
        }
    }

    const EMBEDS: PromptSupport = PromptSupport {
        embedded: true,
        images: false,
    };

    #[test]
    fn a_links_path_is_resolved_with_home_and_the_sessions_directory() {
        let home = Some("/home/u");
        assert_eq!(resolve("~", "/w", home, false), "/home/u");
        assert_eq!(resolve("~/a.rs", "/w", home, false), "/home/u/a.rs");
        assert_eq!(resolve("src/a.rs", "/w", home, false), "/w/src/a.rs");
        assert_eq!(resolve("src/a.rs", "/w/", home, false), "/w/src/a.rs");
        assert_eq!(resolve("/etc/hosts", "/w", home, false), "/etc/hosts");
        assert_eq!(resolve("~/a", "/w", None, false), "~/a");
        // `~user` is a relative name here, as the agent would read it.
        assert_eq!(resolve("~bob/a", "/w", home, false), "/w/~bob/a");
        let home = Some("C:\\Users\\u");
        assert_eq!(resolve("~\\a.rs", "C:\\w", home, true), "C:\\Users\\u\\a.rs");
        assert_eq!(resolve("~/a.rs", "C:\\w", home, true), "C:\\Users\\u\\a.rs");
        assert_eq!(resolve("src\\a.rs", "C:\\w\\", home, true), "C:\\w\\src\\a.rs");
        assert_eq!(resolve("D:\\a.rs", "C:\\w", home, true), "D:\\a.rs");
        assert_eq!(resolve("d:/a.rs", "C:\\w", home, true), "d:/a.rs");
        assert_eq!(resolve("\\\\srv\\s\\a", "C:\\w", home, true), "\\\\srv\\s\\a");
        // A drive-relative path isn't absolute.
        assert_eq!(resolve("D:a.rs", "C:\\w", home, true), "C:\\w\\D:a.rs");
    }

    #[test]
    fn a_stores_files_are_named_by_id_in_its_directory() {
        let mut store = store();
        assert_eq!(store.dir().as_deref(), Some("/home/u/.cache/brindle/attachments/20261128-ab12"));
        assert_eq!(store.take_id(), AttachmentId(1));
        assert_eq!(store.take_id(), AttachmentId(2));
        assert_eq!(store.file(AttachmentId(2)).as_deref(), Some("/home/u/.cache/brindle/attachments/20261128-ab12/2"));
        // An id is never 0, even from a store from before ids.
        let mut old = Store::default();
        assert_eq!(old.take_id(), AttachmentId(1));
        assert_eq!(old.dir(), None);
        assert_eq!(old.file(AttachmentId(1)), None);
        let windows = Store {
            root: "C:\\Users\\u\\AppData\\Local\\brindle\\cache\\attachments\\".into(),
            windows: true,
            ..store
        };
        assert_eq!(
            windows.file(AttachmentId(3)).as_deref(),
            Some("C:\\Users\\u\\AppData\\Local\\brindle\\cache\\attachments\\20261128-ab12\\3")
        );
    }

    #[test]
    fn a_prompts_parts_are_its_text_then_each_attachment_in_order() {
        let texts = BTreeMap::from([(AttachmentId(1), "ok\n".to_owned()), (AttachmentId(2), "x".repeat(70_000))]);
        let attachments = [text(1, "cargo test", 3), link("src/a b.rs"), text(2, "big", 70_000)];
        let (parts, missing) = parts("why?", &attachments, &texts, &store(), "/w", EMBEDS);
        let dir = "file:///home/u/.cache/brindle/attachments/20261128-ab12";
        assert_eq!(
            parts,
            [
                Part::Text("why?".into()),
                Part::Embedded {
                    uri: format!("{dir}/1"),
                    name: "cargo test".into(),
                    text: "ok\n".into()
                },
                Part::Link {
                    uri: "file:///w/src/a%20b.rs".into(),
                    name: "n".into()
                },
                Part::Link {
                    uri: format!("{dir}/2"),
                    name: "big".into()
                },
            ]
        );
        assert!(missing.is_empty());
    }

    #[test]
    fn a_text_of_the_embed_limit_is_embedded_and_one_byte_more_is_linked() {
        let at = "a".repeat(EMBED_LIMIT as usize);
        let over = "a".repeat(EMBED_LIMIT as usize + 1);
        let texts = BTreeMap::from([(AttachmentId(1), at), (AttachmentId(2), over)]);
        let attachments = [text(1, "at", EMBED_LIMIT), text(2, "over", EMBED_LIMIT + 1)];
        let (parts, _) = parts("t", &attachments, &texts, &store(), "/w", EMBEDS);
        assert!(matches!(parts[1], Part::Embedded { .. }));
        assert!(matches!(parts[2], Part::Link { .. }));
    }

    #[test]
    fn an_agent_without_embedded_context_gets_a_link_to_each_kept_text() {
        let texts = BTreeMap::from([(AttachmentId(1), "ok".to_owned())]);
        let (parts, _) = parts("t", &[text(1, "small", 2)], &texts, &store(), "/w", PromptSupport::default());
        assert_eq!(
            parts[1],
            Part::Link {
                uri: "file:///home/u/.cache/brindle/attachments/20261128-ab12/1".into(),
                name: "small".into()
            }
        );
    }

    #[test]
    fn a_missing_text_is_left_out_by_name() {
        let attachments = [text(1, "gone", 2), link("a")];
        let (parts, missing) = parts("t", &attachments, &BTreeMap::new(), &store(), "/w", EMBEDS);
        assert_eq!(parts.len(), 2);
        assert_eq!(missing, ["gone"]);
        // And with no store at all.
        let texts = BTreeMap::from([(AttachmentId(1), "ok".to_owned())]);
        let (parts, missing) = super::parts("t", &attachments, &texts, &Store::default(), "/w", EMBEDS);
        assert_eq!((parts.len(), missing.as_slice()), (2, &["gone".to_owned()][..]));
    }

    // --- through the reducer -------------------------------------------

    use brindle_remote_protocol::serde_json::{Value, json};
    use brindle_transcript::{Entry, HoldReason, NoticeLevel, Slot, StopReason};

    use super::super::harness::{Agent, result};
    use super::super::{Command, Input, Refused};

    /// Ready in `s1` with `capabilities`, its store set at the spawn as the
    /// core's shell sets it.
    fn ready_with(capabilities: Value) -> Agent {
        let mut agent = Agent::in_dir("/w");
        agent.s.store = store();
        agent.step(Input::Spawned);
        agent.feed(&result(1, json!({"protocolVersion": 1, "agentCapabilities": capabilities})));
        agent.feed(&result(2, json!({"sessionId": "s1"})));
        agent
    }

    fn embeds() -> Agent {
        ready_with(json!({"promptCapabilities": {"embeddedContext": true}}))
    }

    fn say(agent: &mut Agent, text: &str, attachments: Vec<Attachment>) -> Option<Result<(), Refused>> {
        agent
            .cmd(Command::Prompt {
                text: text.into(),
                attachments,
            })
            .reply
    }

    fn kept(agent: &mut Agent, id: u64, text: &str) -> Attachment {
        agent.s.texts.insert(AttachmentId(id), text.into());
        self::text(id, &format!("t{id}"), text.len() as u64)
    }

    #[test]
    fn a_prompt_is_sent_with_its_parts_and_logged_with_its_references() {
        let mut agent = embeds();
        let small = kept(&mut agent, 1, "ok\n");
        assert_eq!(say(&mut agent, "why?", vec![small.clone(), link("~/a.rs")]), Some(Ok(())));
        let prompt = &agent.last("session/prompt")["params"]["prompt"];
        assert_eq!(
            *prompt,
            json!([
                {"type": "text", "text": "why?"},
                {"type": "resource", "resource": {"uri": "file:///home/u/.cache/brindle/attachments/20261128-ab12/1", "mimeType": "text/plain", "text": "ok\n"}},
                {"type": "resource_link", "uri": "file:///home/u/a.rs", "name": "n"},
            ])
        );
        // The log keeps the path as given.
        assert_eq!(
            agent.entries(),
            [Entry::User {
                text: "why?".into(),
                attachments: vec![small, link("~/a.rs")]
            }]
        );
    }

    #[test]
    fn an_agent_without_embedded_context_is_sent_a_link_to_a_small_text() {
        let mut agent = ready_with(json!({}));
        let small = kept(&mut agent, 1, "ok");
        say(&mut agent, "t", vec![small]);
        assert_eq!(
            agent.last("session/prompt")["params"]["prompt"][1],
            json!({"type": "resource_link", "uri": "file:///home/u/.cache/brindle/attachments/20261128-ab12/1", "name": "t1"})
        );
    }

    #[test]
    fn a_ninth_attachment_is_refused_before_anything_else() {
        let mut agent = Agent::new();
        let out = agent.cmd(Command::Prompt {
            text: "t".into(),
            attachments: vec![link("a"); 9],
        });
        assert_eq!(out.reply, Some(Err(Refused::TooManyAttachments { count: 9 })));
        assert_eq!(
            Refused::TooManyAttachments { count: 9 }.to_string(),
            "a prompt takes at most 8 attachments, not 9"
        );
        let mut agent = embeds();
        assert_eq!(say(&mut agent, "t", vec![link("a"); 8]), Some(Ok(())));
    }

    #[test]
    fn names_are_cleaned_and_cut_and_a_long_path_is_refused() {
        let mut agent = embeds();
        let named = Attachment::Link {
            path: "a\u{1b}b".into(),
            name: format!("x\u{7}{}", "é".repeat(200)),
        };
        assert_eq!(say(&mut agent, "t", vec![named]), Some(Ok(())));
        let Some(Entry::User { attachments, .. }) = agent.entries().pop() else {
            panic!("no user entry");
        };
        let Attachment::Link { path, name } = &attachments[0] else {
            panic!("not a link");
        };
        assert_eq!(path, "a\u{fffd}b");
        assert!(name.starts_with("x\u{fffd}") && name.len() <= MAX_ATTACHMENT_NAME, "{name}");
        let long = Attachment::Link {
            path: "/".repeat(MAX_PATH + 1),
            name: "n".into(),
        };
        let mut agent = embeds();
        assert_eq!(say(&mut agent, "t", vec![long]), Some(Err(Refused::PathTooLong { len: MAX_PATH + 1 })));
        assert!(agent.entries().is_empty());
    }

    #[test]
    fn a_queued_prompt_keeps_its_attachments_waiting_held_and_sent_again() {
        let mut agent = embeds();
        let id = agent.turn("one");
        let small = kept(&mut agent, 1, "ok");
        assert_eq!(say(&mut agent, "two", vec![small.clone(), link("b")]), Some(Ok(())));
        assert_eq!(*agent.slot(), Slot::Waiting { text: "two".into(), attachments: vec![small.clone(), link("b")] });
        // Cancelled: held with them.
        agent.cmd(Command::Cancel);
        agent.end(id, "cancelled");
        assert_eq!(
            *agent.slot(),
            Slot::Held {
                text: "two".into(),
                reason: HoldReason::Cancelled,
                attachments: vec![small.clone(), link("b")]
            }
        );
        // Sent again: the parts are built then.
        assert_eq!(agent.cmd(Command::Resend).reply, Some(Ok(())));
        let prompt = &agent.last("session/prompt")["params"]["prompt"];
        assert_eq!(prompt[0]["text"], "two");
        assert_eq!(prompt[1]["type"], "resource");
        assert_eq!(prompt[2]["uri"], "file:///w/b");
        assert!(matches!(agent.entries().last(), Some(Entry::User { attachments, .. }) if attachments.len() == 2));
    }

    #[test]
    fn a_waiting_prompt_goes_with_its_attachments_when_the_turn_ends() {
        let mut agent = embeds();
        let id = agent.turn("one");
        let small = kept(&mut agent, 1, "ok");
        say(&mut agent, "two", vec![small]);
        agent.end(id, "end_turn");
        assert_eq!(*agent.slot(), Slot::Empty);
        let prompt = &agent.last("session/prompt")["params"]["prompt"];
        assert_eq!((prompt[0]["text"].clone(), prompt[1]["resource"]["text"].clone()), (json!("two"), json!("ok")));
        let _ = StopReason::EndTurn;
    }

    #[test]
    fn a_discarded_prompt_takes_its_attachments_with_it() {
        let mut agent = embeds();
        agent.turn("one");
        let small = kept(&mut agent, 1, "ok");
        say(&mut agent, "two", vec![small]);
        assert_eq!(agent.cmd(Command::Unqueue).reply, Some(Ok(())));
        assert_eq!(*agent.slot(), Slot::Empty);
        assert!(agent.s.slot_texts().is_empty());
    }

    #[test]
    fn a_missing_text_is_left_out_with_a_warning_naming_it() {
        let mut agent = embeds();
        let gone = text(4, "cargo test", 10);
        assert_eq!(say(&mut agent, "why?", vec![gone.clone()]), Some(Ok(())));
        assert_eq!(agent.last("session/prompt")["params"]["prompt"].as_array().map(Vec::len), Some(1));
        // The entry keeps its reference; the notice says it went without.
        let entries = agent.entries();
        assert!(matches!(&entries[0], Entry::User { attachments, .. } if *attachments == [gone]));
        assert!(matches!(
            &entries[1],
            Entry::Notice { level: NoticeLevel::Warning, text } if text.contains("\"cargo test\" is no longer kept")
        ));
    }

    #[test]
    fn a_session_start_names_the_store_to_an_agent_that_takes_more_directories() {
        let dir = "/home/u/.cache/brindle/attachments/20261128-ab12";
        let agent = ready_with(json!({"sessionCapabilities": {"additionalDirectories": {}}}));
        assert_eq!(agent.last("session/new")["params"]["additionalDirectories"], json!([dir]));
        // Not to one that doesn't, nor without a store.
        let agent = embeds();
        assert!(agent.last("session/new")["params"].get("additionalDirectories").is_none());
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        agent.feed(&result(1, json!({"protocolVersion": 1, "agentCapabilities": {"sessionCapabilities": {"additionalDirectories": {}}}})));
        assert!(agent.last("session/new")["params"].get("additionalDirectories").is_none());
    }

    #[test]
    fn a_windows_hosts_parts_have_windows_uris() {
        let store = Store {
            root: "C:\\c".into(),
            name: "s".into(),
            home: Some("C:\\Users\\u".into()),
            windows: true,
            ..store()
        };
        let texts = BTreeMap::from([(AttachmentId(1), "ok".to_owned())]);
        let (parts, _) = parts("t", &[text(1, "o", 2), link("~\\a.rs")], &texts, &store, "C:\\w", EMBEDS);
        assert_eq!(
            parts[1..],
            [
                Part::Embedded {
                    uri: "file:///C:/c/s/1".into(),
                    name: "o".into(),
                    text: "ok".into()
                },
                Part::Link {
                    uri: "file:///C:/Users/u/a.rs".into(),
                    name: "n".into()
                },
            ]
        );
    }
}
