use brindle_remote_protocol::RequestId;
use brindle_remote_protocol::client::ListedConversation;
use brindle_remote_protocol::serde_json::Value;
use brindle_transcript::clean;

use crate::acp::envelope::RpcError;
use crate::acp::forms::{AUTH_REQUIRED, Listing};

use super::lifecycle::{Phase, Start};
use super::login::AfterLogin;
use super::text::session_title;
use super::{Moment, Refused, Step};

/// The most conversations a list's answer gives (`agents-session-list`
/// decision 2); the rest are dropped, the agent's cursor kept.
pub const MAX_LISTED: usize = 256;
/// The longest listed conversation id kept, in bytes: one longer is left
/// out, since a cut id would name another conversation.
pub const MAX_LISTED_ID: usize = 1024;
/// The longest last-active time kept, in bytes: a longer one is left out.
pub const MAX_LISTED_TIME: usize = 64;
/// The longest cursor taken or passed on, in bytes.
pub const MAX_CURSOR: usize = 1024;

/// A page of the agent's conversations for a client's list, bounded and
/// cleaned, in the agent's order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Page {
    pub conversations: Vec<ListedConversation>,
    pub next: Option<String>,
    /// The agent's entries in this directory left out as unreadable, for
    /// the core's log.
    pub unreadable: usize,
}

/// The answer to a client's list request `req`: a page, or why there is
/// none, as the `ERROR`'s text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListAnswer {
    pub req: RequestId,
    pub result: Result<Page, String>,
}

impl Step<'_> {
    /// A list out when the agent stops being read is never answered: its
    /// client is told `why` (`agents-session-list` decision 11).
    pub(super) fn drop_list(&mut self, why: String) {
        if let Some(req) = self.s.rpc.forget_list() {
            self.out.listed = Some(ListAnswer {
                req,
                result: Err(why),
            });
        }
    }

    /// `AGENT_LIST` (decision 2): taken while the agent is ready or
    /// choosing and offers a list, one at a time; the answer comes as
    /// [`Output::listed`].
    pub(super) fn list(&mut self, req: RequestId, cursor: Option<String>) -> Result<(), Refused> {
        match &self.s.phase {
            Phase::Ready { .. } | Phase::Choosing { .. } => {}
            Phase::Spawning { .. }
            | Phase::Initializing { .. }
            | Phase::Creating { .. }
            | Phase::Resuming { .. }
            | Phase::Loading { .. }
            | Phase::LoginRequired(_)
            | Phase::LoggingIn { .. }
            | Phase::Restarting { .. }
            | Phase::Refused
            | Phase::Broken
            | Phase::Exited
            | Phase::Lost => return Err(self.readiness().err().unwrap_or(Refused::NotReady)),
        }
        if !self.s.lists {
            return Err(Refused::NotOffered);
        }
        if self.s.rpc.listing().is_some() {
            return Err(Refused::ListBusy);
        }
        if let Some(cursor) = cursor.as_ref().filter(|cursor| cursor.len() > MAX_CURSOR) {
            return Err(Refused::TooLongToList {
                what: "cursor",
                len: cursor.len(),
            });
        }
        let line = self.s.rpc.list(req, &self.s.cwd, cursor.as_deref());
        self.out.stdin.extend(line);
        // The list itself isn't kept (flag 4), but the id it took is: a
        // restored session never sends another request under it, so the
        // agent's late answer is to no request.
        self.mark(Moment::BeforeRequest);
        Ok(())
    }

    /// The agent's answer to the list for `req`: the session's directory's
    /// conversations, bounded; or its error, a login it needs included,
    /// which a choosing or ready agent then waits for as for any request.
    pub(super) fn listed(&mut self, req: RequestId, result: Result<&Value, &RpcError>) {
        let result = match result {
            Ok(value) => match self.s.rpc.dialect().list_result(value) {
                Ok(listing) => Ok(page(listing, &self.s.cwd)),
                Err(_) => Err("the agent's answer to the list isn't one Brindle can read".to_owned()),
            },
            // A choosing agent waits for the login, then chooses again
            // (decision 1); a ready one keeps its conversation, and its
            // next prompt finds the login needed as any prompt would.
            Err(error) if error.code == AUTH_REQUIRED => {
                if let Phase::Choosing { greeting } = &self.s.phase {
                    let greeting = greeting.clone();
                    self.login_required(error, greeting, AfterLogin::Start(Start::Choose));
                }
                Err(Refused::LoginRequired.to_string())
            }
            Err(error) => Err(format!(
                "the agent couldn't list its conversations: {}",
                clean(&error.message)
            )),
        };
        self.out.listed = Some(ListAnswer { req, result });
    }
}

/// The page a client gets of the agent's `listing` (`agents-session-list`
/// decision 2): only the conversations in `cwd` (an agent may list every
/// directory's, flag 9), each with a usable id; at most [`MAX_LISTED`] in
/// the agent's order; titles as [`session_title`] keeps them; a time past
/// [`MAX_LISTED_TIME`] bytes, or holding a control character, left out;
/// and the agent's cursor kept only within [`MAX_CURSOR`] bytes.
pub fn page(listing: Listing, cwd: &str) -> Page {
    let here = same_dir(cwd);
    let mut page = Page {
        next: listing
            .next
            .filter(|cursor| !cursor.is_empty() && cursor.len() <= MAX_CURSOR),
        unreadable: listing.unreadable,
        ..Page::default()
    };
    for listed in listing.conversations {
        if same_dir(&listed.cwd) != here {
            continue;
        }
        if listed.id.is_empty() || listed.id.len() > MAX_LISTED_ID || listed.id.chars().any(char::is_control) {
            page.unreadable += 1;
            continue;
        }
        if page.conversations.len() == MAX_LISTED {
            break;
        }
        page.conversations.push(ListedConversation {
            id: listed.id,
            title: listed.title.as_deref().and_then(session_title),
            updated: listed.updated.filter(|time| {
                !time.is_empty() && time.len() <= MAX_LISTED_TIME && !time.chars().any(char::is_control)
            }),
        });
    }
    page
}

/// A directory as compared for a list: without a trailing separator, so
/// `/w/` is `/w` (the root stays `/`).
fn same_dir(dir: &str) -> &str {
    let trimmed = dir.trim_end_matches(['/', '\\']);
    if trimmed.is_empty() { &dir[..dir.len().min(1)] } else { trimmed }
}

#[cfg(test)]
mod tests {
    use brindle_remote_protocol::serde_json::json;
    use brindle_transcript::Activity;

    use super::super::harness::*;
    use super::super::lifecycle::{CANT_CONTINUE, NOT_REPLAYED};
    use super::super::text::MAX_SESSION_TITLE;
    use super::super::*;
    use super::*;

    // --- choosing and listing (agents-session-list) ----------------------

    use crate::acp::forms::Listed;


    #[test]
    fn a_choosing_agent_waits_after_initialize_and_refuses_prompts() {
        let mut agent = Agent::new();
        agent.s = AgentSession::choosing("/home/u/src".into(), "0.3.0".into());
        agent.step(Input::Spawned);
        let out = agent.feed(&initialized(1, listing_offered()));
        assert!(out.stdin.is_empty(), "nothing after initialize");
        assert_eq!(out.checkpoint, Some(Moment::Started));
        assert!(matches!(agent.s.phase(), Phase::Choosing { .. }));
        assert_eq!(*agent.agent_state(), AgentState::Choosing { message: None });
        assert_eq!(agent.activity(), Activity::Idle);
        assert_eq!(agent.prompt("hi").reply, Some(Err(Refused::NotReady)));
        assert_eq!(agent.sent_methods(), ["initialize"]);
    }

    #[test]
    fn a_chosen_conversation_continues_by_what_the_agent_offers_into_the_fresh_log() {
        // Offering both, a fresh log loads it, its replay shown.
        let mut agent = Agent::choosing_offering(listing_offered());
        assert_eq!(agent.choose(Some("c7")).reply, Some(Ok(())));
        assert_eq!(*agent.agent_state(), AgentState::Starting);
        let load = agent.last("session/load").clone();
        assert_eq!(load["params"]["sessionId"], "c7");
        assert_eq!(load["params"]["cwd"], "/home/u/src");
        let id = load["id"].as_u64().unwrap();
        agent.feed(&result(id, json!({})));
        assert_eq!(*agent.s.phase(), Phase::Ready { session: "c7".into() });
        assert_eq!(agent.s.transcript.conversation().map(|c| c.id.as_str()), Some("c7"));
        assert!(!agent.s.chose, "a continue done is no longer a choice");
        // Resume-only agents resume it, with the notice that nothing shows.
        let mut resuming = Agent::choosing_offering(json!({"sessionCapabilities": {"resume": {}, "list": {}}}));
        resuming.choose(Some("c8"));
        let resume = resuming.last("session/resume").clone();
        assert_eq!(resume["params"]["sessionId"], "c8");
        resuming.feed(&result(resume["id"].as_u64().unwrap(), json!({})));
        assert_eq!(resuming.notices(), [NOT_REPLAYED]);
    }

    #[test]
    fn a_choice_of_none_starts_a_new_conversation() {
        let mut agent = Agent::choosing_offering(listing_offered());
        assert_eq!(agent.choose(None).reply, Some(Ok(())));
        let id = agent.last("session/new")["id"].as_u64().unwrap();
        agent.feed(&result(id, json!({"sessionId": "n1"})));
        assert_eq!(*agent.s.phase(), Phase::Ready { session: "n1".into() });
    }

    #[test]
    fn a_refused_choice_returns_to_choosing_with_the_agents_reason() {
        let mut agent = Agent::choosing_offering(listing_offered());
        agent.choose(Some("gone"));
        let id = agent.last("session/load")["id"].as_u64().unwrap();
        let out = agent.feed(&error(id, -32002, "no such session"));
        assert!(!out.hang_up, "the agent stays to choose again");
        assert_eq!(
            *agent.agent_state(),
            AgentState::Choosing {
                message: Some("the agent couldn't continue the conversation: no such session".into())
            }
        );
        assert!(matches!(agent.s.phase(), Phase::Choosing { .. }));
        // A second choice works.
        assert_eq!(agent.choose(None).reply, Some(Ok(())));
        // An agent offering neither way back says so, and waits too.
        let mut neither = Agent::choosing_offering(json!({"sessionCapabilities": {"list": {}}}));
        assert_eq!(neither.choose(Some("c1")).reply, Some(Ok(())));
        assert_eq!(
            *neither.agent_state(),
            AgentState::Choosing {
                message: Some(CANT_CONTINUE.into())
            }
        );
    }

    #[test]
    fn a_choice_is_refused_unless_the_agent_is_choosing() {
        let mut ready = Agent::ready_offering(listing_offered());
        assert_eq!(ready.choose(None).reply, Some(Err(Refused::NotChoosing)));
        let mut agent = Agent::choosing_offering(listing_offered());
        let long = "x".repeat(MAX_LISTED_ID + 1);
        assert_eq!(
            agent.choose(Some(&long)).reply,
            Some(Err(Refused::TooLongToList {
                what: "conversation id",
                len: MAX_LISTED_ID + 1
            }))
        );
        agent.choose(None);
        assert_eq!(agent.choose(None).reply, Some(Err(Refused::NotChoosing)));
    }

    #[test]
    fn a_list_asks_for_the_sessions_directory_and_answers_with_its_conversations_only() {
        let mut agent = Agent::choosing_offering(listing_offered());
        let out = agent.list(9, Some("p1"));
        assert_eq!(out.reply, Some(Ok(())));
        // Its id is kept before it goes, though the list isn't (flag 4).
        assert_eq!(out.checkpoint, Some(Moment::BeforeRequest));
        let list = agent.last("session/list").clone();
        assert_eq!(list["params"], json!({"cwd": "/home/u/src", "cursor": "p1"}));
        let out = agent.feed(&result(
            list["id"].as_u64().unwrap(),
            json!({"sessions": [
                {"sessionId": "a", "cwd": "/home/u/src", "title": "Fix\nthe   build", "updatedAt": "2026-11-26T18:05:00Z"},
                {"sessionId": "b", "cwd": "/elsewhere", "title": "Not here"},
                {"sessionId": "c", "cwd": "/home/u/src/", "updatedAt": "x".repeat(MAX_LISTED_TIME + 1)},
                {"sessionId": "", "cwd": "/home/u/src"}
            ], "nextCursor": "p2"}),
        ));
        let page = listed_ok(&out);
        assert_eq!(out.listed.as_ref().map(|answer| answer.req), Some(9));
        assert_eq!(
            page.conversations,
            [
                ListedConversation {
                    id: "a".into(),
                    title: Some("Fix the build".into()),
                    updated: Some("2026-11-26T18:05:00Z".into())
                },
                ListedConversation {
                    id: "c".into(),
                    title: None,
                    updated: None
                },
            ]
        );
        assert_eq!(page.next.as_deref(), Some("p2"));
        assert_eq!(page.unreadable, 1);
        // The answer changes nothing in the log.
        assert!(out.events.is_empty());
        assert!(matches!(agent.s.phase(), Phase::Choosing { .. }));
    }

    #[test]
    fn a_list_is_refused_when_not_offered_busy_too_long_or_not_ready() {
        let mut agent = Agent::choosing_offering(both());
        assert_eq!(agent.list(1, None).reply, Some(Err(Refused::NotOffered)));
        let mut agent = Agent::choosing_offering(listing_offered());
        assert_eq!(agent.list(1, None).reply, Some(Ok(())));
        assert_eq!(agent.list(2, None).reply, Some(Err(Refused::ListBusy)));
        let mut ready = Agent::ready_offering(listing_offered());
        let long = "c".repeat(MAX_CURSOR + 1);
        assert_eq!(
            ready.list(3, Some(&long)).reply,
            Some(Err(Refused::TooLongToList {
                what: "cursor",
                len: MAX_CURSOR + 1
            }))
        );
        // A ready agent lists too, during a turn included (flag 8).
        ready.turn("one");
        assert_eq!(ready.list(4, None).reply, Some(Ok(())));
        let mut starting = Agent::new();
        starting.step(Input::Spawned);
        assert_eq!(starting.list(5, None).reply, Some(Err(Refused::NotReady)));
    }

    #[test]
    fn a_list_error_is_answered_with_the_agents_words() {
        let mut agent = Agent::choosing_offering(listing_offered());
        agent.list(9, None);
        let id = agent.last("session/list")["id"].as_u64().unwrap();
        let out = agent.feed(&error(id, -32603, "index unavailable"));
        assert_eq!(
            out.listed,
            Some(ListAnswer {
                req: 9,
                result: Err("the agent couldn't list its conversations: index unavailable".into())
            })
        );
        assert!(matches!(agent.s.phase(), Phase::Choosing { .. }));
        // An answer Brindle can't read.
        agent.list(10, None);
        let id = agent.last("session/list")["id"].as_u64().unwrap();
        let out = agent.feed(&result(id, json!({"sessions": "none"})));
        assert!(matches!(out.listed, Some(ListAnswer { req: 10, result: Err(_) })));
    }

    #[test]
    fn a_login_a_list_needs_returns_to_choosing_once_done() {
        let mut agent = Agent::new();
        agent.s = AgentSession::choosing("/home/u/src".into(), "0.3.0".into());
        agent.step(Input::Spawned);
        agent.feed(&initialized_with_methods(1, listing_offered()));
        agent.list(9, None);
        let id = agent.last("session/list")["id"].as_u64().unwrap();
        let out = agent.feed(&error(id, AUTH_REQUIRED, "log in first"));
        assert_eq!(
            out.listed,
            Some(ListAnswer {
                req: 9,
                result: Err(Refused::LoginRequired.to_string())
            })
        );
        assert_eq!(*agent.agent_state(), login_required("log in first"));
        agent.login("oauth-personal");
        let id = agent.authenticate_id();
        agent.feed(&result(id, json!({})));
        assert!(matches!(agent.s.phase(), Phase::Choosing { .. }));
        assert_eq!(*agent.agent_state(), AgentState::Choosing { message: None });
        // A ready agent's list keeps its conversation.
        let mut ready = Agent::ready_with_methods();
        ready.s.lists = true;
        ready.list(3, None);
        let id = ready.last("session/list")["id"].as_u64().unwrap();
        ready.feed(&error(id, AUTH_REQUIRED, "log in first"));
        assert_eq!(*ready.s.phase(), Phase::Ready { session: "s1".into() });
    }

    #[test]
    fn a_list_out_when_the_agent_ends_is_answered_with_why() {
        let mut agent = Agent::choosing_offering(listing_offered());
        agent.list(9, None);
        let out = agent.exit();
        assert_eq!(
            out.listed,
            Some(ListAnswer {
                req: 9,
                result: Err(Refused::Exited.to_string())
            })
        );
        assert!(agent.s.rpc.listing().is_none());
    }

    #[test]
    fn a_page_is_bounded_and_keeps_the_agents_order_and_cursor() {
        let conversations = (0..MAX_LISTED + 10)
            .map(|n| Listed {
                id: format!("s{n}"),
                cwd: "/w".into(),
                title: Some(format!("t{n}")),
                updated: None,
            })
            .chain([Listed {
                id: "x".repeat(MAX_LISTED_ID + 1),
                cwd: "/w".into(),
                title: None,
                updated: None,
            }])
            .collect();
        let listing = Listing {
            conversations,
            next: Some("more".into()),
            unreadable: 2,
        };
        let page = page(listing, "/w");
        assert_eq!(page.conversations.len(), MAX_LISTED);
        assert_eq!(page.conversations[0].id, "s0");
        assert_eq!(page.conversations[MAX_LISTED - 1].id, format!("s{}", MAX_LISTED - 1));
        assert_eq!(page.next.as_deref(), Some("more"));
        let long_cursor = Listing {
            next: Some("c".repeat(MAX_CURSOR + 1)),
            ..Listing::default()
        };
        assert_eq!(super::page(long_cursor, "/w").next, None);
        let long_title = Listing {
            conversations: vec![Listed {
                id: "a".into(),
                cwd: "/".into(),
                title: Some("t".repeat(MAX_SESSION_TITLE * 2)),
                updated: None,
            }],
            ..Listing::default()
        };
        let title = super::page(long_title, "/").conversations[0].title.clone().unwrap();
        assert_eq!(title.chars().count(), MAX_SESSION_TITLE);
        assert!(title.ends_with('…'));
    }
}
