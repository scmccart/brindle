use brindle_transcript::{ITEM_LIMIT, MAX_PATH, RAW_LIMIT, TEXT_CHUNK, ToolItem, clean};

use crate::acp::forms;
use crate::acp::limits::MAX_PERMISSION_ID;

/// The longest conversation title kept, in characters, `…` included
/// (`agents-session-title` decision 3): a tab's title, read as characters,
/// and well under [`TEXT_CHUNK`] however long the agent's title.
pub const MAX_SESSION_TITLE: usize = 256;

/// The agent's readable name (`agents-session-title` decision 4):
/// `agentInfo.title` when it is given and not blank, else `agentInfo.name`
/// (which the npm adapters fill with their package's name), cleaned and
/// bounded.
pub(super) fn agent_name(agent: &forms::AgentInfo) -> String {
    agent
        .title
        .as_deref()
        .map(|title| bound(clean(title)))
        .filter(|title| !title.trim().is_empty())
        .unwrap_or_else(|| bound(clean(&agent.name)))
}

/// A conversation's title as kept (`agents-session-title` decision 3):
/// control characters dropped, each run of whitespace (line breaks
/// included) one space, trimmed, and past [`MAX_SESSION_TITLE`] characters
/// cut to one fewer and ended with `…`. A title with nothing left is none.
pub fn session_title(raw: &str) -> Option<String> {
    let mut title = String::with_capacity(raw.len().min(4 * MAX_SESSION_TITLE));
    let mut chars = 0usize;
    let mut gap = false;
    for c in raw.chars() {
        if c.is_whitespace() {
            gap = chars > 0;
            continue;
        }
        if c.is_control() {
            continue;
        }
        if gap {
            title.push(' ');
            chars += 1;
            gap = false;
        }
        title.push(c);
        chars += 1;
        if chars > MAX_SESSION_TITLE {
            break;
        }
    }
    if chars > MAX_SESSION_TITLE {
        let cut = title
            .char_indices()
            .nth(MAX_SESSION_TITLE - 1)
            .map_or(title.len(), |(at, _)| at);
        title.truncate(cut);
        title.truncate(title.trim_end().len());
        title.push('…');
    }
    (!title.is_empty()).then_some(title)
}

/// `text` cut to at most [`TEXT_CHUNK`] bytes at a character boundary, for
/// a field that doesn't grow.
pub(super) fn bound(mut text: String) -> String {
    if text.len() > TEXT_CHUNK {
        let mut cut = TEXT_CHUNK;
        while !text.is_char_boundary(cut) {
            cut -= 1;
        }
        text.truncate(cut);
    }
    text
}

/// `text` cut to at most `limit` bytes at a character boundary, and
/// whether it was cut.
pub(super) fn cut_at(mut text: String, limit: usize) -> (String, bool) {
    if text.len() <= limit {
        return (text, false);
    }
    let mut cut = limit;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    text.truncate(cut);
    (text, true)
}

/// A text content item, cleaned, its end dropped past [`ITEM_LIMIT`].
pub(super) fn text_item(text: &str) -> ToolItem {
    let (text, cut) = cut_at(clean(text), ITEM_LIMIT);
    ToolItem::Text { text, cut }
}

/// A raw input or result, cleaned, its end dropped past [`RAW_LIMIT`] with
/// `…`. Cut before it is cleaned, so a long one costs only its bound
/// (`agents-event-bounds` decision 5); cleaning can lengthen the kept
/// part, which is cut again.
pub(super) fn raw_text(text: &str) -> String {
    let ellipsis = "…";
    let mut prefix = RAW_LIMIT.min(text.len());
    while !text.is_char_boundary(prefix) {
        prefix -= 1;
    }
    let was_cut = prefix < text.len();
    match cut_at(clean(&text[..prefix]), RAW_LIMIT) {
        (text, false) if !was_cut => text,
        (text, _) => {
            let (mut text, _) = cut_at(text, RAW_LIMIT - ellipsis.len());
            text.push_str(ellipsis);
            text
        }
    }
}

/// Whether an id from the agent can be kept and sent back as it came: not
/// empty, at most [`MAX_PERMISSION_ID`] bytes, and no control character
/// (so it encodes to at most twice its bytes, as cleaned text does).
pub(super) fn sendable_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_PERMISSION_ID && !id.chars().any(char::is_control)
}

/// A path or a directory, cleaned and cut at [`MAX_PATH`]
/// (`agents-event-bounds` decision 3).
pub(super) fn path_text(path: &str) -> String {
    cut_at(clean(path), MAX_PATH).0
}

#[cfg(test)]
mod tests {
    use brindle_remote_protocol::serde_json::json;

    use super::super::harness::*;
    use super::super::*;
    use super::*;


    #[test]
    fn a_session_title_is_one_line_bounded_and_none_when_blank() {
        assert_eq!(
            session_title("Hello.txt file creation").as_deref(),
            Some("Hello.txt file creation")
        );
        assert_eq!(
            session_title("  Fix\tthe\r\n\n build  \u{7}now\u{1b}[31m ").as_deref(),
            Some("Fix the build now[31m")
        );
        assert_eq!(
            session_title("a\u{0}b \u{85} c").as_deref(),
            Some("ab c"),
            "a control character is dropped; NEL is whitespace"
        );
        for blank in ["", "   ", "\n\t\r", "\u{7}\u{1b}", " \u{0} "] {
            assert_eq!(session_title(blank), None, "{blank:?}");
        }
        let exact = "x".repeat(MAX_SESSION_TITLE);
        assert_eq!(session_title(&exact).as_deref(), Some(exact.as_str()));
        let long: String = (0..40).map(|n| format!("word{n:03}\n")).collect();
        assert_eq!(long.chars().count(), 320);
        let kept = session_title(&format!("{long}{}", "y".repeat(80))).expect("a title");
        assert_eq!(kept.chars().count(), MAX_SESSION_TITLE);
        assert!(kept.ends_with('…') && !kept.contains('\n'), "{kept}");
        let long = "z".repeat(400);
        let kept = session_title(&long).expect("a title");
        assert_eq!(kept, format!("{}…", "z".repeat(MAX_SESSION_TITLE - 1)));
        // A cut inside multi-byte characters lands on a boundary.
        let wide = "é".repeat(300);
        let kept = session_title(&wide).expect("a title");
        assert_eq!(kept, format!("{}…", "é".repeat(MAX_SESSION_TITLE - 1)));
        // A cut after a space leaves none before the ellipsis.
        let spaced = format!("{} {}", "a".repeat(MAX_SESSION_TITLE - 2), "b".repeat(10));
        let kept = session_title(&spaced).expect("a title");
        assert_eq!(kept, format!("{}…", "a".repeat(MAX_SESSION_TITLE - 2)));
    }

    #[test]
    fn a_title_names_the_conversation_and_null_or_blank_clears_it() {
        let mut agent = Agent::ready();
        let out = agent.feed(&titled(json!("Hello.txt file creation")));
        assert_eq!(shape(&out.events), ["conversation"]);
        assert_eq!(title_of(&agent), Some("Hello.txt file creation"));
        assert_eq!(out.checkpoint, None, "no new checkpoint moment");
        // The same title again sends nothing, and so does a cleaned one
        // that comes out the same.
        assert_eq!(agent.feed(&titled(json!("Hello.txt file creation"))), Output::default());
        assert_eq!(
            agent.feed(&titled(json!(" Hello.txt\nfile  creation "))),
            Output::default()
        );
        // No title keeps it.
        let out = agent.feed(&update(
            json!({"sessionUpdate": "session_info_update", "_meta": {"codex": {}}}),
        ));
        assert_eq!(out, Output::default());
        assert_eq!(title_of(&agent), Some("Hello.txt file creation"));
        // A number is no title either.
        assert_eq!(agent.feed(&titled(json!(7))), Output::default());
        let out = agent.feed(&titled(Value::Null));
        assert_eq!(shape(&out.events), ["conversation"]);
        assert_eq!(title_of(&agent), None);
        assert_eq!(agent.feed(&titled(Value::Null)), Output::default());
        agent.feed(&titled(json!("Again")));
        agent.feed(&titled(json!("  \n ")));
        assert_eq!(title_of(&agent), None, "a blank title clears");
        // Everything else about the conversation stays.
        assert_eq!(
            agent.s.transcript().conversation().map(|c| (c.id.as_str(), c.cwd.as_str())),
            Some(("s1", "/home/u/src"))
        );
    }

    /// The captures' `agentInfo`, verbatim: the readable `title` names the
    /// agent when given, else `name`.
    #[test]
    fn the_agents_name_is_its_title_when_given_else_its_name() {
        let rows = [
            (
                json!({"name": "@agentclientprotocol/codex-acp", "title": "Codex", "version": "2.12.0"}),
                "Codex",
            ),
            (
                json!({"name": "@agentclientprotocol/claude-agent-acp", "title": "Claude Agent", "version": "0.112.0"}),
                "Claude Agent",
            ),
            (
                json!({"name": "@zed-industries/claude-code-acp", "title": "Claude Code", "version": "0.16.2"}),
                "Claude Code",
            ),
            (
                json!({"name": "Antigravity ACP Server", "version": "1.5.16"}),
                "Antigravity ACP Server",
            ),
            (
                json!({"name": "Copilot", "title": "Copilot", "version": "1.0.115"}),
                "Copilot",
            ),
            // A blank or null title is no title.
            (json!({"name": "pkg", "title": "  ", "version": "1"}), "pkg"),
            (json!({"name": "pkg", "title": null, "version": "1"}), "pkg"),
        ];
        for (info, name) in rows {
            let mut agent = Agent::new();
            agent.step(Input::Spawned);
            agent.feed(&result(1, json!({"protocolVersion": 1, "agentInfo": info})));
            agent.feed(&result(2, json!({"sessionId": "s1"})));
            assert!(
                matches!(agent.agent_state(), AgentState::Ready { name: Some(n), .. } if n == name),
                "{info}: {:?}",
                agent.agent_state()
            );
        }
    }

    #[test]
    fn a_title_mid_turn_adds_no_entry() {
        let mut agent = Agent::ready();
        let id = agent.turn("one");
        agent.feed(&said("Hel"));
        agent.feed(&titled(json!("Greeting")));
        agent.feed(&said("lo"));
        assert_eq!(agent.last_entry().1, Entry::Agent { text: "Hello".into() });
        agent.end(id, "end_turn");
        assert_eq!(title_of(&agent), Some("Greeting"));
    }

    #[test]
    fn a_title_before_the_session_is_given_names_nothing() {
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        agent.feed(&result(
            1,
            json!({"protocolVersion": 1, "agentInfo": {"name": "fake", "version": "1.0"}}),
        ));
        // `session/new` not answered yet: the session isn't known.
        assert_eq!(agent.feed(&titled(json!("Early"))), Output::default());
        agent.feed(&result(2, json!({"sessionId": "s1"})));
        assert_eq!(title_of(&agent), None);
        // Another session's title names nothing either.
        let line = json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "other", "update": {"sessionUpdate": "session_info_update", "title": "Other"}}});
        assert_eq!(agent.feed(&line.to_string()), Output::default());
        assert_eq!(title_of(&agent), None);
    }

    #[test]
    fn a_title_is_in_the_replay() {
        let mut agent = Agent::ready();
        agent.feed(&titled(json!("Hello.txt file creation")));
        let replay = agent.s.transcript().replay();
        assert!(replay.iter().any(|event| matches!(
            event,
            Event::Conversation { conversation } if conversation.title.as_deref() == Some("Hello.txt file creation")
        )));
        let mut fresh = Transcript::new();
        for event in &replay {
            fresh.apply(event).expect("a replay folds");
        }
        assert_eq!(fresh, *agent.s.transcript());
    }

    #[test]
    fn an_update_for_another_acp_session_is_ignored() {
        let mut agent = Agent::ready();
        agent.turn("one");
        let line = json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "other", "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "x"}}}});
        assert!(agent.feed(&line.to_string()).events.is_empty());
    }

    #[test]
    fn an_answer_to_no_request_the_core_awaits_is_ignored() {
        let mut agent = Agent::ready();
        assert_eq!(agent.feed(&result(99, json!({}))), Output::default());
        assert_eq!(agent.feed(&error(98, -1, "x")), Output::default());
    }
}
