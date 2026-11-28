//! The zone's dump lines (`--dump-screen-after`).

use crate::agent::login::MethodOffer;
use crate::agent::markdown;
use crate::agent::tool;
use crate::agent::transcript::{
    AgentState, Attachment, DiffLine, Entry, EntryId, Exit, FileOp, NoticeLevel, OptionCategory, OptionControl, PermissionOutcome,
    ToolItem, ToolKind, ToolStatus, TurnState, status_of,
};
use crate::terminal::links::Plan;
use crate::terminal::status::State;

use super::ChatModel;
use super::attach::{Draft, TextState};
use super::blocks::BlockBody;
use super::login::LoginRow;
use super::paging::Earlier;
use super::phase::{AgentPhase, Link, SessionLife};
use super::queue::{QueueRow, Resend, hold_word, stop_word};
use super::recovery::Offer;

/// A tool entry's content in the dump (decision 12), open or not: a
/// `--- item` line per content item, a `--- hunk` line per hunk after its
/// diff's, a `--- location` line per location, and a `--- raw` line for
/// each raw value its detail shows. Nothing for the other entries.
fn tool_lines(id: EntryId, entry: &Entry) -> String {
    let Entry::Tool { content, locations, input, result, .. } = entry else { return String::new() };
    let mut out = String::new();
    for (index, item) in content.iter().enumerate() {
        out.push_str(&format!("--- item entry={id} index={index} kind="));
        match item {
            ToolItem::Text { text, cut } => {
                out.push_str(&format!("text cut={} {text:?}\n", if *cut { "yes" } else { "no" }));
            }
            ToolItem::Diff(change) => {
                let op = match &change.op {
                    FileOp::Add => "add".to_string(),
                    FileOp::Delete => "delete".to_string(),
                    FileOp::Modify => "modify".to_string(),
                    FileOp::Move { from } => format!("move from={from:?}"),
                    FileOp::Copy { from } => format!("copy from={from:?}"),
                    FileOp::Other { name } => format!("other:{name}"),
                };
                let lines = change.hunks.iter().flat_map(|hunk| &hunk.lines);
                let added = lines.clone().filter(|line| matches!(line, DiffLine::Added(_))).count();
                let removed = lines.filter(|line| matches!(line, DiffLine::Removed(_))).count();
                out.push_str(&format!(
                    "diff op={op} path={:?} added={added} removed={removed} omitted={}\n",
                    change.path, change.omitted
                ));
                for hunk in &change.hunks {
                    let text: Vec<String> = hunk
                        .lines
                        .iter()
                        .map(|line| match line {
                            DiffLine::Context(text) => format!(" {text}"),
                            DiffLine::Added(text) => format!("+{text}"),
                            DiffLine::Removed(text) => format!("-{text}"),
                        })
                        .collect();
                    out.push_str(&format!(
                        "--- hunk entry={id} index={index} old={} new={} {:?}\n",
                        hunk.old_start,
                        hunk.new_start,
                        text.join("\n")
                    ));
                }
            }
            ToolItem::Output(output) => {
                out.push_str("output");
                if let Some(command) = &output.command {
                    out.push_str(&format!(" command={command:?}"));
                }
                if let Some(cwd) = &output.cwd {
                    out.push_str(&format!(" cwd={cwd:?}"));
                }
                let exit = match &output.exit {
                    Some(Exit::Code { code }) => code.to_string(),
                    Some(Exit::Signal { name }) => format!("signal:{name}"),
                    None => "none".to_string(),
                };
                out.push_str(&format!(" exit={exit} omitted={} {:?}\n", output.omitted, output.text));
            }
        }
    }
    for location in locations {
        out.push_str(&format!("--- location entry={id} path={:?}", location.path));
        if let Some(line) = location.line {
            out.push_str(&format!(" line={line}"));
        }
        out.push('\n');
    }
    // Raw values only where the detail shows them: with no content items.
    if content.is_empty() {
        for (which, value) in [(tool::Raw::Input, input), (tool::Raw::Result, result)] {
            if let Some(text) = value {
                out.push_str(&format!("--- raw entry={id} which={} {text:?}\n", which.word()));
            }
        }
    }
    out
}

fn state_word(phase: &AgentPhase) -> String {
    match phase {
        AgentPhase::Connecting => "connecting".into(),
        AgentPhase::NotStarted(_) => "not-started".into(),
        AgentPhase::Live(AgentState::Starting) => "starting".into(),
        AgentPhase::Live(AgentState::Choosing { .. }) => "choosing".into(),
        AgentPhase::Live(AgentState::Ready { .. }) => "ready".into(),
        AgentPhase::Live(AgentState::LoginRequired { .. }) => "login-required".into(),
        AgentPhase::Live(AgentState::LoggingIn { .. }) => "logging-in".into(),
        AgentPhase::Live(AgentState::Failed { .. }) => "failed".into(),
        AgentPhase::Live(AgentState::Exited { code: Some(code), .. }) => format!("exited:{code}"),
        AgentPhase::Live(AgentState::Exited { code: None, .. }) => "exited:signal".into(),
        AgentPhase::Live(AgentState::Lost { .. }) => "holder-lost".into(),
        AgentPhase::Lost { .. } => "lost".into(),
        AgentPhase::Broken { .. } => "broken".into(),
    }
}

fn turn_word(turn: &TurnState) -> String {
    match turn {
        TurnState::Idle => "idle".into(),
        TurnState::Working => "working".into(),
        TurnState::Blocked => "blocked".into(),
        TurnState::Cancelling => "cancelling".into(),
        TurnState::Ended { stop } => format!("ended:{}", stop_word(stop)),
        TurnState::Failed { .. } => "failed".into(),
    }
}

fn tool_kind_word(kind: ToolKind) -> &'static str {
    match kind {
        ToolKind::Read => "read",
        ToolKind::Edit => "edit",
        ToolKind::Delete => "delete",
        ToolKind::Move => "move",
        ToolKind::Search => "search",
        ToolKind::Execute => "execute",
        ToolKind::Think => "think",
        ToolKind::Fetch => "fetch",
        ToolKind::SwitchMode => "switch_mode",
        ToolKind::Other => "other",
    }
}

fn tool_status_word(status: ToolStatus) -> &'static str {
    match status {
        ToolStatus::Pending => "pending",
        ToolStatus::InProgress => "in_progress",
        ToolStatus::Completed => "completed",
        ToolStatus::Failed => "failed",
    }
}

impl ChatModel {
    /// The dump's lines for this zone (decision 15), zone `zone`, with the
    /// conversation `at_end` or away from it and the continue `offer`
    /// ([`Self::offer`], which needs the config's profiles); each link's
    /// action from what `plan` says of its target (`links::plan_here` in
    /// Brindle).
    pub fn describe(&self, zone: u64, at_end: bool, offer: &Offer, plan: &dyn Fn(&str) -> Plan) -> String {
        self.lines(Some(zone), Some(at_end), offer, plan)
    }

    /// What `--wait-for` matches in an agent zone: the dump's lines,
    /// without the zone's id and the scroll position.
    pub fn wait_text(&self, offer: &Offer, plan: &dyn Fn(&str) -> Plan) -> String {
        self.lines(None, None, offer, plan)
    }

    /// The `--- block` line of each of entry `id`'s rendered parts, in
    /// order, each followed by a `--- link` line per link in it.
    fn part_lines(&self, id: EntryId, plan: &dyn Fn(&str) -> Plan) -> String {
        let mut out = String::new();
        for (index, part) in self.rendered.get(&id).into_iter().flatten().enumerate() {
            let (kind, text) = (markdown::kind_word(&part.block), markdown::shown_text(&part.block));
            out.push_str(&format!("--- block entry={id} part={index} kind={kind} {text:?}\n"));
            for (link, text) in markdown::block_links(&part.block) {
                let action = markdown::link_action(&link.target, plan(&link.target));
                out.push_str(&format!(
                    "--- link entry={id} part={index} target={:?} text={text:?} action={}\n",
                    link.target.as_ref(),
                    markdown::action_word(&action)
                ));
            }
        }
        out
    }

    fn lines(&self, zone: Option<u64>, at_end: Option<bool>, offer: &Offer, plan: &dyn Fn(&str) -> Plan) -> String {
        let mut out = String::from("--- agent");
        if let Some(zone) = zone {
            out.push_str(&format!(" zone={zone}"));
        }
        out.push_str(&format!(" state={}", state_word(&self.phase)));
        match &self.phase {
            AgentPhase::Live(
                AgentState::LoginRequired { message, .. }
                | AgentState::Failed { message }
                | AgentState::Choosing { message: Some(message) },
            ) => {
                out.push_str(&format!(" message={message:?}"));
            }
            // The failure as logged, the server's cause verbatim (on
            // Windows a batch file's `cmd /c` form included).
            AgentPhase::NotStarted(failure) => out.push_str(&format!(" message={:?}", failure.to_string())),
            AgentPhase::Live(
                AgentState::Starting
                | AgentState::Ready { .. }
                | AgentState::LoggingIn { .. }
                | AgentState::Exited { .. }
                | AgentState::Lost { .. }
                | AgentState::Choosing { message: None },
            )
            | AgentPhase::Connecting
            | AgentPhase::Lost { .. }
            | AgentPhase::Broken { .. } => {}
        }
        if let AgentPhase::Live(AgentState::Ready { name: Some(name), .. }) = &self.phase {
            out.push_str(&format!(" name={name:?}"));
        }
        let live = matches!(self.phase, AgentPhase::Live(_));
        let activity = if live { status_of(self.activity).name() } else { "none" };
        let status = self.shown_status().map_or("none", State::name);
        let link = match self.link {
            Link::Up => "up",
            Link::Reconnecting => "reconnecting",
        };
        let session = match self.life {
            SessionLife::Open => "open",
            SessionLife::Ended => "ended",
        };
        out.push_str(&format!(
            " turn={} activity={activity} status={status} link={link} session={session} continue={}\n",
            turn_word(&self.turn),
            offer.word()
        ));
        out.push_str(&self.login_lines());
        for note in self.notes() {
            out.push_str(&format!("--- note {note:?}\n"));
        }
        out.push_str(&self.list_lines());
        for (id, entry) in &self.entries {
            out.push_str(&format!("--- entry {id} {}\n", self.entry_line(*id, entry)));
            out.push_str(&self.part_lines(*id, plan));
            out.push_str(&self.attached_lines(*id, entry));
            out.push_str(&tool_lines(*id, entry));
        }
        if let Some(row) = self.working_row() {
            out.push_str(&format!("--- working {:?}\n", row.text().as_ref()));
        }
        // A queued prompt's attachments counted only when it has some, so
        // the line without them reads as before.
        let counted = |count: usize| if count > 0 { format!(" attachments={count}") } else { String::new() };
        match &self.queue {
            QueueRow::None => {}
            QueueRow::Waiting { text, attachments } => {
                out.push_str(&format!("--- queued waiting{} {:?}\n", counted(*attachments), text.as_ref()));
            }
            QueueRow::Held { text, attachments, reason, resend, .. } => {
                let resend = match resend {
                    Resend::Available => "yes",
                    Resend::Unavailable(_) => "no",
                };
                out.push_str(&format!(
                    "--- queued held reason={} resend={resend}{} {:?}\n",
                    hold_word(reason),
                    counted(*attachments),
                    text.as_ref()
                ));
            }
        }
        out.push_str(&format!("--- conversation entries={}", self.entries.len()));
        if let Some(at_end) = at_end {
            out.push_str(if at_end { " at=end" } else { " at=away" });
        }
        match &self.earlier() {
            Earlier::None => out.push_str(" earlier=none"),
            Earlier::Available { count } => out.push_str(&format!(" earlier={count}")),
            Earlier::Loading { .. } => out.push_str(" earlier=loading"),
            Earlier::Failed { count, why } => out.push_str(&format!(" earlier={count} fetch-error={:?}", why.as_ref())),
        }
        out.push('\n');
        out.push_str(&self.draft_lines());
        out.push_str(&format!("--- prompt status={}", self.prompt_status().word()));
        if let Some(first) = self.first_shown() {
            out.push_str(&format!(" first={first:?}"));
        }
        if let Some(refusal) = &self.refusal {
            out.push_str(&format!(" refusal={refusal:?}"));
        }
        if self.is_choosing() {
            out.push_str(&format!(" reason={:?}", self.choosing_reason()));
        }
        let choices = self.zone_choices(offer);
        if !choices.is_empty() {
            let words: Vec<&str> = choices.iter().map(|choice| choice.word()).collect();
            out.push_str(&format!(" actions={}", words.join(",")));
        }
        out.push('\n');
        if let Some(completion) = &self.completion {
            out.push_str(&completion.lines());
        }
        out
    }

    /// A user entry's `--- attached` line per attachment (OpenSpec change
    /// `agents-prompt-context`, decision 10): a kept text's id, size and
    /// row state, and its text only while shown; a link's path and name.
    fn attached_lines(&self, id: EntryId, entry: &Entry) -> String {
        let Entry::User { attachments, .. } = entry else { return String::new() };
        let mut out = String::new();
        for (index, attachment) in attachments.iter().enumerate() {
            out.push_str(&format!("--- attached entry={id} index={index} "));
            match attachment {
                Attachment::Text { id: kept, name, bytes } => {
                    let state = self.row_state(id, index);
                    out.push_str(&format!("kind=text id={} name={name:?} bytes={bytes} state={}", kept.0, state.name()));
                    match state {
                        TextState::Shown(text) => out.push_str(&format!(" {text:?}")),
                        TextState::Gone(why) => out.push_str(&format!(" why={why:?}")),
                        TextState::Closed | TextState::Loading => {}
                    }
                }
                Attachment::Link { path, name } => out.push_str(&format!("kind=link path={path:?} name={name:?}")),
            }
            out.push('\n');
        }
        out
    }

    /// A `--- draft` line per attachment the next prompt takes, each
    /// holding its text.
    fn draft_lines(&self) -> String {
        let mut out = String::new();
        for (index, draft) in self.drafts().iter().enumerate() {
            match draft {
                Draft::Text { name, text } => out.push_str(&format!(
                    "--- draft index={index} kind=text name={name:?} bytes={} {text:?}\n",
                    text.len()
                )),
                Draft::Link { path, name } => {
                    out.push_str(&format!("--- draft index={index} kind=link path={path:?} name={name:?}\n"));
                }
            }
        }
        out
    }

    /// The `--- commands` line and an `--- option` line per option, in the
    /// order the input shows them (OpenSpec change `agents-commands`,
    /// decision 8); nothing for an empty list.
    fn list_lines(&self) -> String {
        let mut out = String::new();
        if !self.commands.is_empty() {
            out.push_str(&format!("--- commands count={}\n", self.commands.len()));
        }
        for item in self.option_items() {
            let Some(option) = self.options.iter().find(|option| option.id == item.id) else { continue };
            let category = match &option.category {
                OptionCategory::Mode => "mode".to_string(),
                OptionCategory::Model => "model".to_string(),
                OptionCategory::ModelConfig => "model_config".to_string(),
                OptionCategory::ThoughtLevel => "thought_level".to_string(),
                OptionCategory::Other { name } => format!("other:{name}"),
            };
            let (kind, current, choices) = match &option.kind {
                OptionControl::Select { current, choices } => {
                    ("select", format!("{current:?}"), format!(" choices={}", choices.len()))
                }
                OptionControl::Toggle { on } => ("toggle", if *on { "on" } else { "off" }.to_string(), String::new()),
            };
            out.push_str(&format!(
                "--- option id={:?} category={category} kind={kind} current={current} name={:?}{choices}\n",
                option.id, option.name
            ));
        }
        out
    }

    /// The login row's dump lines (decision 11): one `--- login choice`
    /// per method, or `--- login running`; a note beside the row ends the
    /// last one as ` refusal="…"`.
    fn login_lines(&self) -> String {
        let mut lines: Vec<String> = match self.login_row() {
            LoginRow::None => return String::new(),
            LoginRow::Running { method, name, terminal: _ } => {
                vec![format!("--- login running method={:?} {:?}", method.as_ref(), name.as_ref())]
            }
            LoginRow::Choose { methods, .. } => methods
                .iter()
                .map(|choice| {
                    let kind = match choice.offer {
                        MethodOffer::Agent => "agent",
                        MethodOffer::Terminal(_) | MethodOffer::Withheld(_) => "terminal",
                    };
                    let mut line =
                        format!("--- login choice method={:?} kind={kind} {:?}", choice.id.as_ref(), choice.name.as_ref());
                    if let Some(description) = &choice.description {
                        line.push_str(&format!(" description={:?}", description.as_ref()));
                    }
                    match &choice.offer {
                        MethodOffer::Agent => {}
                        MethodOffer::Terminal(plan) => {
                            line.push_str(&format!(" args={:?} env={:?}", plan.appended.join(" "), plan.env_text()));
                            if !plan.dropped.is_empty() {
                                line.push_str(&format!(" dropped={:?}", plan.dropped.join(",")));
                            }
                        }
                        MethodOffer::Withheld(withheld) => line.push_str(&format!(" withheld={:?}", withheld.why)),
                    }
                    line
                })
                .collect(),
        };
        if let (Some(note), Some(last)) = (&self.login_note, lines.last_mut()) {
            last.push_str(&format!(" refusal={note:?}"));
        }
        lines.into_iter().map(|line| line + "\n").collect()
    }

    pub(super) fn entry_line(&self, id: EntryId, entry: &Entry) -> String {
        match entry {
            Entry::User { text, attachments } if attachments.is_empty() => format!("user {text:?}"),
            Entry::User { text, attachments } => format!("user attachments={} {text:?}", attachments.len()),
            Entry::Agent { text } => format!("agent {text:?}"),
            Entry::Thought { text } => format!("thought open={} {text:?}", self.thought_open(id)),
            Entry::Tool { call, title, kind, status, content, more, .. } => {
                // The items left out and the row's summary, each only when
                // there is one (flags 31 and 33), after `items=` so the
                // `open=… items=N` prefixes still match.
                let more = if *more > 0 { format!(" more={more}") } else { String::new() };
                let summary = tool::summary(entry).map(|s| format!(" summary={:?}", s.as_ref())).unwrap_or_default();
                format!(
                    "tool call={call:?} kind={} status={} open={} items={}{more}{summary} {title:?}",
                    tool_kind_word(*kind),
                    tool_status_word(*status),
                    self.tool_state(id, entry).word(),
                    content.len()
                )
            }
            Entry::Permission { call, title, options, outcome } => {
                let call = call.as_ref().map(|call| format!(" call={call:?}")).unwrap_or_default();
                // The agent's order, not the row's (the spec's "Debug hooks
                // in an agent zone").
                let options: Vec<String> = options.iter().map(|o| format!("{}:{:?}", o.id, o.name)).collect();
                let outcome = match outcome {
                    None => "pending".to_string(),
                    Some(PermissionOutcome::Selected { option }) => format!("selected:{option}"),
                    Some(PermissionOutcome::Cancelled) => "cancelled".to_string(),
                };
                // The parts of its tool call's detail shown with it, only
                // when there are some (decision 12).
                let shown = match self.blocks.get(self.entry_blocks(id).start).map(|block| &block.body) {
                    Some(BlockBody::Permission { detail, .. }) if !detail.is_empty() => format!(" detail={}", detail.len()),
                    Some(_) | None => String::new(),
                };
                format!("permission{call} options={} outcome={outcome}{shown} {title:?}", options.join("|"))
            }
            Entry::Notice { level, text } => {
                let level = match level {
                    NoticeLevel::Info => "info",
                    NoticeLevel::Warning => "warn",
                    NoticeLevel::Error => "error",
                };
                format!("notice level={level} {text:?}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::harness::*;
    use super::super::*;
    use super::*;

    /// Decision 15: the dump's lines, and what `--wait-for` matches.
    #[test]
    fn the_dump_describes_the_zone() {
        let mut zone = Zone::ready();
        zone.feed(&[
            added(1, user("hi")),
            turn(TurnState::Working),
            added(2, agent("Hello!")),
            added(3, thought("Considering")),
            added(4, Entry::tool("call_1".into(), "Read main.rs".into(), ToolKind::Read, ToolStatus::Pending)),
            tool_completed(4),
            added(5, permission("Run cargo test?")),
            added(6, Entry::Notice { level: NoticeLevel::Warning, text: "1 line was not an ACP message".into() }),
            queue(waiting()),
        ]);
        let expected = "\
--- agent zone=1 state=ready name=\"Fake Agent\" turn=working activity=working status=working link=up session=open continue=none:not-ended
--- entry 1 user \"hi\"
--- entry 2 agent \"Hello!\"
--- block entry=2 part=0 kind=paragraph \"Hello!\"
--- entry 3 thought open=false \"Considering\"
--- block entry=3 part=0 kind=paragraph \"Considering\"
--- entry 4 tool call=\"call_1\" kind=read status=completed open=none items=0 \"Read main.rs\"
--- entry 5 permission call=\"call_2\" options=reject-once:\"Reject\"|allow-once:\"Allow once\" outcome=pending \"Run cargo test?\"
--- entry 6 notice level=warn \"1 line was not an ACP message\"
--- working \"Working…\"
--- queued waiting \"next\"
--- conversation entries=6 at=end earlier=none
--- prompt status=streaming
";
        assert_eq!(zone.model.describe(1, true, &found(&zone.model), &linux), expected);
        let wait = zone.model.wait_text(&found(&zone.model), &linux);
        assert!(wait.starts_with("--- agent state=ready name=\"Fake Agent\" turn=working"), "{wait}");
        assert!(wait.contains("--- conversation entries=6 earlier=none\n"), "{wait}");

        let mut zone = Zone::new(None);
        zone.model.set_note("the profile \"Old\" is gone; this tab uses \"Fake\"");
        zone.model.not_started(StartFailure::TooOld { endpoint: "A8".into() });
        zone.model.submit("x", false);
        let dump = zone.model.describe(2, false, &found(&zone.model), &linux);
        assert_eq!(
            dump,
            "--- agent zone=2 state=not-started message=\"the terminal server on A8 is too old for agent sessions; \
             it will update once its terminals have ended\" turn=idle activity=none status=none link=up session=open \
             continue=none:no-conversation\n\
             --- note \"the profile \\\"Old\\\" is gone; this tab uses \\\"Fake\\\"\"\n\
             --- conversation entries=0 at=away earlier=none\n\
             --- prompt status=disabled\n"
        );
        assert_eq!(zone.model.note(), Some("the profile \"Old\" is gone; this tab uses \"Fake\""));
        let mut held = Zone::ready();
        held.feed(&[queue(held_slot())]);
        held.model.sent(1, held.model.sent_prompt("next", false));
        held.model.replied(1, Err("the agent isn't ready".into()));
        let dump = held.model.describe(1, true, &found(&held.model), &linux);
        assert!(dump.contains("--- queued held reason=stopped:max_tokens resend=yes \"next\"\n"), "{dump}");
        assert!(dump.ends_with("--- prompt status=ready refusal=\"the agent isn't ready\"\n"), "{dump}");
        assert_eq!(held.model.banner("A8"), None);
        held.model.set_link(Link::Reconnecting);
        assert_eq!(held.model.link(), Link::Reconnecting);
        assert_eq!(held.model.banner("A8").as_deref(), Some("reconnecting to A8…"));
        assert!(held.model.describe(1, true, &found(&held.model), &linux).contains("link=reconnecting"));
    }

    /// `remote-login-environment` 5.4: an endpoint note shows below a
    /// restore's, each its own dump line, and a later start's replaces it.
    #[test]
    fn an_endpoint_note_shows_below_a_restores_note() {
        const FELL_BACK: &str = "the login shell's environment on A8 wasn't read (it took longer than 10 s), \
                                 so this program has the SSH session's environment";
        let mut zone = Zone::ready();
        assert_eq!(zone.model.notes().count(), 0);
        zone.model.set_endpoint_note(FELL_BACK);
        assert_eq!(zone.model.notes().collect::<Vec<_>>(), [FELL_BACK]);
        assert_eq!(zone.model.note(), None);
        let dump = zone.model.describe(1, true, &found(&zone.model), &linux);
        assert!(dump.contains(&format!("--- note {FELL_BACK:?}\n")), "{dump}");

        zone.model.set_note("the profile \"Old\" is gone; this tab uses \"Fake\"");
        let again = "the login shell's environment on A8 wasn't read (no login shell is set), \
                     so this program has the SSH session's environment";
        zone.model.set_endpoint_note(again);
        assert_eq!(
            zone.model.notes().collect::<Vec<_>>(),
            ["the profile \"Old\" is gone; this tab uses \"Fake\"", again]
        );
        let dump = zone.model.describe(1, true, &found(&zone.model), &linux);
        let notes: Vec<&str> = dump.lines().filter(|line| line.starts_with("--- note ")).collect();
        assert_eq!(
            notes,
            [
                "--- note \"the profile \\\"Old\\\" is gone; this tab uses \\\"Fake\\\"\"".to_string(),
                format!("--- note {again:?}"),
            ]
        );
    }

    /// `agents-commands` 4.4: the lists' lines after the notes and before
    /// the entries, the completion's after the prompt line, names escaped;
    /// `--wait-for` sees them too; an agent with neither has no line.
    #[test]
    fn the_dump_shows_the_commands_the_options_and_the_completion() {
        assert!(!Zone::ready().model.describe(1, true, &found(&Zone::ready().model), &linux).contains("--- option"));
        let mut zone = with_lists();
        zone.feed(&[added(1, user("hi"))]);
        zone.model.typed("/co", 3, false);
        let dump = zone.model.describe(1, true, &found(&zone.model), &linux);
        let lines: Vec<&str> = dump.lines().collect();
        let at = |prefix: &str| lines.iter().position(|line| line.starts_with(prefix)).unwrap_or_else(|| panic!("{prefix}: {dump}"));
        assert_eq!(lines[at("--- commands")], "--- commands count=2");
        assert_eq!(
            lines[at("--- option")..at("--- option") + 3],
            [
                "--- option id=\"mode\" category=mode kind=select current=\"auto\" name=\"Mode\" choices=2",
                "--- option id=\"model\" category=model kind=select current=\"opus\" name=\"Model\" choices=2",
                "--- option id=\"fast\" category=other:speed kind=toggle current=off name=\"Fast \\\"mode\\\"\"",
            ]
        );
        assert!(at("--- commands") < at("--- entry"), "{dump}");
        assert_eq!(lines[at("--- completion")], "--- completion query=\"co\" highlighted=0");
        assert!(at("--- prompt") < at("--- completion"), "{dump}");
        assert_eq!(lines[at("--- completion") + 1], "/compact  <how>  about compact");
        let wait = zone.model.wait_text(&found(&zone.model), &linux);
        assert!(wait.contains("--- commands count=2\n") && wait.contains("--- completion query=\"co\""), "{wait}");
    }

    /// Decision 11: block and link lines follow each agent message's and
    /// thought's entry line; user messages have none.
    #[test]
    fn the_dump_shows_each_part_and_link() {
        let mut zone = Zone::ready();
        zone.feed(&[
            added(1, user("**hi**")),
            added(2, agent("Try:\n\n```rust\nfn main() {}\n```\nSee [docs](https://example.com) or <mailto:a@example.com> [x](javascript:void).")),
            added(3, thought("- [ ] check `x`\n- see https://t.example")),
        ]);
        let dump = zone.model.describe(1, true, &found(&zone.model), &linux);
        let expected = "\
--- entry 1 user \"**hi**\"
--- entry 2 agent \"Try:\\n\\n```rust\\nfn main() {}\\n```\\nSee [docs](https://example.com) or <mailto:a@example.com> [x](javascript:void).\"
--- block entry=2 part=0 kind=paragraph \"Try:\"
--- block entry=2 part=1 kind=code:rust \"fn main() {}\\n\"
--- block entry=2 part=2 kind=paragraph \"See docs or mailto:a@example.com x.\"
--- link entry=2 part=2 target=\"https://example.com\" text=\"docs\" action=open
--- link entry=2 part=2 target=\"mailto:a@example.com\" text=\"mailto:a@example.com\" action=confirm
--- link entry=2 part=2 target=\"javascript:void\" text=\"x\" action=refuse:scheme
--- entry 3 thought open=false \"- [ ] check `x`\\n- see https://t.example\"
--- block entry=3 part=0 kind=list \"- [ ] check x\\n- see https://t.example\"
--- link entry=3 part=0 target=\"https://t.example\" text=\"https://t.example\" action=open
--- conversation entries=3 at=end earlier=none
";
        assert!(dump.contains(expected), "{dump}");
        assert!(!dump.contains("--- block entry=1"), "{dump}");
        let wait = zone.model.wait_text(&found(&zone.model), &linux);
        assert!(wait.contains("--- block entry=2 part=1 kind=code:rust \"fn main() {}\\n\"\n"), "{wait}");
        assert!(wait.contains("--- link entry=2 part=2 target=\"https://example.com\" text=\"docs\" action=open\n"), "{wait}");
        // The plan is the caller's: Windows refuses a file link with no drive.
        zone.feed(&[added(4, agent("[f](file:///etc/hosts)"))]);
        let windows = |uri: &str| crate::terminal::links::plan(uri, "thishost", true);
        assert!(zone.model.wait_text(&found(&zone.model), &windows).contains("action=refuse:no-drive\n"));
    }

    /// `agents-prompt-context` 4.6 (decision 10): a user entry's count and
    /// `--- attached` lines in each row state, the draft's lines before the
    /// prompt's, the queued count, and escaping; `--wait-for` sees them all.
    #[test]
    fn the_dump_shows_attachments_the_draft_and_the_queued_count() {
        use crate::agent::transcript::{Attachment, AttachmentId, Slot};
        let mut zone = Zone::ready();
        let kept = Attachment::Text { id: AttachmentId(5), name: "cargo \"test\"".into(), bytes: 262_144 };
        let link = Attachment::Link { path: "src/main.rs".into(), name: "main.rs".into() };
        zone.feed(&[
            added(1, user_with("why does this fail?", vec![kept.clone(), link])),
            queue(Slot::Waiting { text: "next".into(), attachments: vec![kept] }),
        ]);
        zone.model.attach(Draft::text("output from zsh", "a\tb\n")).unwrap();
        zone.model.attach(Draft::link("C:\\w\\a.rs")).unwrap();
        let dump = zone.model.describe(1, true, &found(&zone.model), &linux);
        let expected = "\
--- entry 1 user attachments=2 \"why does this fail?\"
--- attached entry=1 index=0 kind=text id=5 name=\"cargo \\\"test\\\"\" bytes=262144 state=closed
--- attached entry=1 index=1 kind=link path=\"src/main.rs\" name=\"main.rs\"
--- queued waiting attachments=1 \"next\"
--- conversation entries=1 at=end earlier=none
--- draft index=0 kind=text name=\"output from zsh\" bytes=4 \"a\\tb\\n\"
--- draft index=1 kind=link path=\"C:\\\\w\\\\a.rs\" name=\"a.rs\"
--- prompt status=ready
";
        assert!(dump.ends_with(expected), "{dump}");
        let wait = zone.model.wait_text(&found(&zone.model), &linux);
        assert!(wait.contains("--- draft index=0 kind=text name=\"output from zsh\" bytes=4 \"a\\tb\\n\"\n"), "{wait}");

        let entry = EntryId(1);
        zone.model.press_row(entry, 0);
        zone.model.sent(7, Sent::Read { entry, index: 0 });
        assert!(zone.model.wait_text(&found(&zone.model), &linux).contains("state=loading\n"));
        zone.model.attachment_read(7, "ok\n".into());
        assert!(zone.model.wait_text(&found(&zone.model), &linux).contains("state=shown \"ok\\n\"\n"));
        zone.model.press_row(entry, 0);
        assert!(zone.model.wait_text(&found(&zone.model), &linux).contains("state=closed\n"), "closed, its text dropped");
        assert_eq!(zone.model.press_row(entry, 0), RowPress::Read(5));
        zone.model.sent(8, Sent::Read { entry, index: 0 });
        zone.model.replied(8, Err(ReplyError::Failed("attachment 5 is no longer kept".into())));
        let gone = zone.model.wait_text(&found(&zone.model), &linux);
        assert!(gone.contains("state=gone why=\"the text is no longer kept\"\n"), "{gone}");
    }
}
