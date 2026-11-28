use brindle_remote_protocol::serde_json::Value;
use brindle_transcript::{
    Entry, EntryId, Event, NoticeLevel, PermissionOption, PermissionOutcome, ToolKind, ToolStatus,
    TurnState, clean,
};
#[cfg(test)]
use brindle_transcript::Transcript;

use crate::acp::envelope::Id;
use crate::acp::forms::{OptionKind, Patch, Request, ToolChange, ToolDetail};
use crate::acp::limits::{MAX_PERMISSION_NAME, MAX_PERMISSION_OPTIONS};
use crate::acp::rpc;

use super::lifecycle::Phase;
use super::text::{bound, cut_at, sendable_id};
use super::{Moment, Refused, Step};

impl Step<'_> {
    /// A permission request's `toolCall`: its content, locations and input
    /// go to the tool entry it names, added from the request's fields when
    /// the log has none. Its title, kind and status never change the
    /// entry, since the request's title may differ (Copilot's does), and
    /// its result and output are the update's to send.
    fn permission_tool(&mut self, change: ToolChange) {
        let id = bound(clean(&change.call));
        let entry = match self.tool(&id) {
            Some(entry) => entry,
            None => {
                let title = change
                    .title
                    .value()
                    .map_or_else(|| id.clone(), |title| bound(clean(title)));
                let kind = change.kind.value().copied().unwrap_or(ToolKind::Other);
                let status = change
                    .status
                    .value()
                    .copied()
                    .unwrap_or(ToolStatus::Pending);
                self.add_tool(id, title, kind, status)
            }
        };
        let detail = ToolDetail {
            result: Patch::Keep,
            outputs: Vec::new(),
            ..change.detail
        };
        self.tool_detail(entry, detail);
    }

    pub(super) fn permission(&mut self, id: Id, params: Value) {
        let Ok(request) = self.s.rpc.dialect().permission_request(params) else {
            // Answered by its envelope id, never left waiting (decision 5).
            self.out
                .stdin
                .extend(self.s.rpc.dialect().encode(Request::PermissionAnswer {
                    id: &id,
                    option: None,
                }));
            self.notice(
                NoticeLevel::Warning,
                "the agent asked for a permission Brindle couldn't read, which was answered cancelled"
                    .to_owned(),
            );
            return;
        };
        // At most `MAX_PERMISSION_OPTIONS`, each with an id an answer can
        // send back as it came (`agents-event-bounds` decision 3): one that
        // is empty, past `MAX_PERMISSION_ID` or holds a control character
        // is left out rather than cut, and counted.
        let mut options = Vec::new();
        let mut unknown = Vec::new();
        let mut left_out = 0usize;
        for option in request.options {
            match option.kind {
                OptionKind::Known(kind) => {
                    if options.len() >= MAX_PERMISSION_OPTIONS || !sendable_id(&option.id) {
                        left_out += 1;
                        continue;
                    }
                    options.push(PermissionOption {
                        id: option.id,
                        name: cut_at(clean(&option.name), MAX_PERMISSION_NAME).0,
                        kind,
                    });
                }
                OptionKind::Unknown { name } => {
                    if unknown.len() < MAX_PERMISSION_OPTIONS {
                        unknown.push(cut_at(clean(&name), MAX_PERMISSION_NAME).0);
                    }
                }
            }
        }
        // The request's tool call is applied first, so the entry its row
        // points at holds what the agent sent in either place
        // (`agents-tool-content` decision 7).
        // Its id and title are read first, so the call itself moves to the
        // entry with no copy of its content (`agents-checkpoint-deltas`
        // decision 11).
        let call = request.call.as_ref().map(|call| bound(clean(&call.call)));
        let call_title = request
            .call
            .as_ref()
            .and_then(|call| call.title.value())
            .map(|title| bound(clean(title)));
        if let Some(change) = request.call {
            self.permission_tool(change);
        }
        let title = call_title
            .or_else(|| {
                let entry = self.tool(call.as_deref()?)?;
                match self.s.transcript.entry(entry)? {
                    Entry::Tool { title, .. } => Some(title.clone()),
                    Entry::User { .. }
                    | Entry::Agent { .. }
                    | Entry::Thought { .. }
                    | Entry::Permission { .. }
                    | Entry::Notice { .. } => None,
                }
            })
            .unwrap_or_else(|| "the agent asks for permission".to_owned());
        let shown = !options.is_empty();
        let entry = self.add(Entry::Permission {
            call,
            title,
            options,
            outcome: None,
        });
        if !unknown.is_empty() {
            self.notice(
                NoticeLevel::Info,
                format!(
                    "permission request {entry} also offered options of a kind Brindle doesn't know ({}), which aren't shown",
                    unknown.join(", ")
                ),
            );
        }
        if left_out > 0 {
            self.notice(
                NoticeLevel::Info,
                format!(
                    "permission request {entry}: {left_out} of its options were left out, past {MAX_PERMISSION_OPTIONS} or with an id Brindle can't send back"
                ),
            );
        }
        if !shown {
            self.cancel_now(entry, &id);
            self.notice(
                NoticeLevel::Warning,
                format!(
                    "permission request {entry} offered no option Brindle can show, so it was answered cancelled"
                ),
            );
        } else if *self.s.transcript.turn() == TurnState::Cancelling {
            self.cancel_now(entry, &id);
        } else if self.s.rpc.hold_permission(entry, id.clone()).is_err() {
            self.cancel_now(entry, &id);
            self.notice(
                NoticeLevel::Warning,
                format!(
                    "more than {} permission requests were waiting, so request {entry} was answered cancelled",
                    rpc::MAX_PENDING_PERMISSIONS
                ),
            );
        } else if *self.s.transcript.turn() != TurnState::Blocked {
            self.turn(TurnState::Blocked);
        }
        self.mark(Moment::Permission);
    }

    /// Answers a permission request `cancelled` at once and records it.
    fn cancel_now(&mut self, entry: EntryId, id: &Id) {
        self.out.stdin.extend(
            self.s
                .rpc
                .dialect()
                .encode(Request::PermissionAnswer { id, option: None }),
        );
        self.emit(Event::Answered {
            id: entry,
            outcome: PermissionOutcome::Cancelled,
        });
    }

    /// `$/cancel_request` for one of the agent's permission requests.
    pub(super) fn withdraw(&mut self, id: &Id) {
        if let Some((entry, answer)) = self.s.rpc.withdraw(id) {
            self.out.stdin.extend(answer);
            self.emit(Event::Answered {
                id: entry,
                outcome: PermissionOutcome::Cancelled,
            });
            self.unblock();
            self.mark(Moment::Permission);
        }
    }

    /// With no permission request left waiting, a blocked turn works again
    /// (or, with no prompt out, is idle).
    pub(super) fn unblock(&mut self) {
        if self.s.rpc.permissions().is_empty() && *self.s.transcript.turn() == TurnState::Blocked {
            let state = if self.prompt_awaited() {
                TurnState::Working
            } else {
                TurnState::Idle
            };
            self.turn(state);
        }
    }

    pub(super) fn answer(&mut self, entry: EntryId, option: Option<String>) -> Result<(), Refused> {
        match self.s.phase {
            Phase::Restarting { .. }
            | Phase::Refused
            | Phase::Broken
            | Phase::Exited
            | Phase::Lost => {
                self.readiness()?;
            }
            Phase::Spawning { .. }
            | Phase::Initializing { .. }
            | Phase::Creating { .. }
            | Phase::Resuming { .. }
            | Phase::Loading { .. }
            | Phase::Ready { .. }
            | Phase::Choosing { .. }
            | Phase::LoginRequired(_)
            | Phase::LoggingIn { .. } => {}
        }
        let Some(Entry::Permission {
            options, outcome, ..
        }) = self.s.transcript.entry(entry)
        else {
            return Err(Refused::NoSuchRequest { entry });
        };
        if outcome.is_some() {
            return Err(Refused::AlreadyAnswered { entry });
        }
        if let Some(chosen) = &option
            && !options.iter().any(|offered| offered.id == *chosen)
        {
            return Err(Refused::NoSuchOption {
                entry,
                option: chosen.clone(),
            });
        }
        let Some(answer) = self.s.rpc.answer_permission(entry, option.as_deref()) else {
            return Err(Refused::AlreadyAnswered { entry });
        };
        self.out.stdin.extend(answer);
        let outcome = match option {
            Some(option) => PermissionOutcome::Selected { option },
            None => PermissionOutcome::Cancelled,
        };
        self.emit(Event::Answered { id: entry, outcome });
        self.unblock();
        self.mark(Moment::Permission);
        Ok(())
    }
}

/// The log's permission requests still waiting for an answer, in entry
/// order: the other side of the invariant `Rpc::waiting_permissions` is
/// checked against after every step (`agents-process-lifecycle` decision
/// 3).
#[cfg(test)]
pub(in crate::acp) fn pending_permissions(transcript: &Transcript) -> Vec<EntryId> {
    transcript
        .entries()
        .filter(|(_, entry)| matches!(entry, Entry::Permission { outcome: None, .. }))
        .map(|(id, _)| id)
        .collect()
}

#[cfg(test)]
mod tests {
    use brindle_remote_protocol::serde_json::json;
    use brindle_transcript::PermissionKind;

    use super::super::harness::*;
    use super::super::*;
    use super::*;

    // --- permissions ----------------------------------------------------

    #[test]
    fn a_permission_request_is_logged_with_its_options_and_blocks_the_turn() {
        let mut agent = Agent::ready();
        agent.turn("one");
        let out = agent.feed(&permission(json!(0), "call_1", &ALLOW_REJECT));
        assert_eq!(
            shape(&out.events),
            // The request names a call the log doesn't have: it is added first
            // (`agents-tool-content` decision 7).
            ["added tool", "added permission", "turn blocked"]
        );
        assert_eq!(out.checkpoint, Some(Moment::Permission));
        assert!(out.stdin.is_empty(), "never answered on its own");
        assert_eq!(
            agent.last_permission().1,
            Entry::Permission {
                call: Some("call_1".into()),
                title: "Write a file".into(),
                options: vec![
                    PermissionOption {
                        id: "allow-once".into(),
                        name: "ALLOW-ONCE".into(),
                        kind: PermissionKind::AllowOnce
                    },
                    PermissionOption {
                        id: "reject-once".into(),
                        name: "REJECT-ONCE".into(),
                        kind: PermissionKind::RejectOnce
                    },
                ],
                outcome: None,
            }
        );
    }

    #[test]
    fn a_tool_calls_id_is_cleaned_and_still_matches_its_updates() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.feed(&update(json!({"sessionUpdate": "tool_call_update", "toolCallId": "c\u{1b}]0;x\u{7}"})));
        assert!(matches!(
            agent.last_entry().1,
            Entry::Tool { call, title, .. } if call == "c\u{fffd}]0;x\u{fffd}" && title == "c\u{fffd}]0;x\u{fffd}"
        ));
        let entries = agent.entries().len();
        agent.feed(&update(
            json!({"sessionUpdate": "tool_call_update", "toolCallId": "c\u{1b}]0;x\u{7}", "status": "completed"}),
        ));
        assert_eq!(agent.entries().len(), entries);
    }

    #[test]
    fn a_permission_requests_title_falls_back_to_its_tool_calls() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.feed(&tool_call("call_1", "Edit main.rs", "pending"));
        agent.feed(&json!({"jsonrpc": "2.0", "id": 0, "method": "session/request_permission",
            "params": {"sessionId": "s1", "toolCall": {"toolCallId": "call_1"}, "options": [{"optionId": "a", "name": "A", "kind": "allow_once"}]}}).to_string());
        assert!(
            matches!(agent.last_permission().1, Entry::Permission { title, .. } if title == "Edit main.rs")
        );
    }

    #[test]
    fn the_attached_clients_choice_answers_it_and_the_turn_works_again() {
        let mut agent = Agent::ready();
        agent.turn("one");
        let entry = agent.ask(json!(0), &ALLOW_REJECT);
        let out = agent.cmd(Command::Answer {
            entry,
            option: Some("allow-once".into()),
        });
        assert_eq!(out.reply, Some(Ok(())));
        assert_eq!(
            out.stdin,
            b"{\"jsonrpc\":\"2.0\",\"id\":0,\"result\":{\"outcome\":{\"outcome\":\"selected\",\"optionId\":\"allow-once\"}}}\n"
        );
        assert_eq!(shape(&out.events), ["answered", "turn working"]);
        assert_eq!(out.checkpoint, Some(Moment::Permission));
    }

    #[test]
    fn an_answer_with_no_option_is_cancelled() {
        let mut agent = Agent::ready();
        agent.turn("one");
        let entry = agent.ask(json!(0), &ALLOW_REJECT);
        let out = agent.cmd(Command::Answer {
            entry,
            option: None,
        });
        assert_eq!(out.reply, Some(Ok(())));
        assert!(out.events.contains(&Event::Answered {
            id: entry,
            outcome: PermissionOutcome::Cancelled
        }));
    }

    #[test]
    fn a_second_answer_is_refused_naming_the_request() {
        let mut agent = Agent::ready();
        agent.turn("one");
        let entry = agent.ask(json!(0), &ALLOW_REJECT);
        agent.cmd(Command::Answer {
            entry,
            option: Some("allow-once".into()),
        });
        let out = agent.cmd(Command::Answer {
            entry,
            option: Some("reject-once".into()),
        });
        assert_eq!(out.reply, Some(Err(Refused::AlreadyAnswered { entry })));
        assert!(out.stdin.is_empty());
    }

    #[test]
    fn an_answer_naming_an_option_not_offered_or_an_entry_not_a_request_is_refused() {
        let mut agent = Agent::ready();
        let user = EntryId(1);
        agent.turn("one");
        let entry = agent.ask(json!(0), &ALLOW_REJECT);
        let out = agent.cmd(Command::Answer {
            entry,
            option: Some("deny".into()),
        });
        assert_eq!(
            out.reply,
            Some(Err(Refused::NoSuchOption {
                entry,
                option: "deny".into()
            }))
        );
        let out = agent.cmd(Command::Answer {
            entry: user,
            option: None,
        });
        assert_eq!(out.reply, Some(Err(Refused::NoSuchRequest { entry: user })));
        assert_eq!(*agent.turn_state(), TurnState::Blocked);
    }

    #[test]
    fn a_withdrawn_request_is_answered_cancelled() {
        let mut agent = Agent::ready();
        agent.turn("one");
        let entry = agent.ask(json!("7"), &ALLOW_REJECT);
        let out = agent
            .feed(r#"{"jsonrpc":"2.0","method":"$/cancel_request","params":{"requestId":"7"}}"#);
        assert_eq!(
            out.stdin,
            b"{\"jsonrpc\":\"2.0\",\"id\":\"7\",\"result\":{\"outcome\":{\"outcome\":\"cancelled\"}}}\n"
        );
        assert_eq!(shape(&out.events), ["answered", "turn working"]);
        assert_eq!(
            agent
                .cmd(Command::Answer {
                    entry,
                    option: None
                })
                .reply,
            Some(Err(Refused::AlreadyAnswered { entry }))
        );
    }

    #[test]
    fn a_request_past_sixteen_waiting_is_answered_cancelled_with_a_notice() {
        let mut agent = Agent::ready();
        agent.turn("one");
        for id in 0..16 {
            agent.ask(json!(id), &ALLOW_REJECT);
        }
        let out = agent.feed(&permission(json!(16), "call_1", &ALLOW_REJECT));
        assert_eq!(
            shape(&out.events),
            ["added permission", "answered", "added notice"]
        );
        assert_eq!(
            out.stdin,
            b"{\"jsonrpc\":\"2.0\",\"id\":16,\"result\":{\"outcome\":{\"outcome\":\"cancelled\"}}}\n"
        );
        assert_eq!(agent.s.rpc().permissions().len(), 16);
    }

    #[test]
    fn an_option_of_an_unknown_kind_is_left_out_with_a_notice() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.ask(
            json!(7),
            &[
                ("allow", "allow_once"),
                ("session", "allow_for_session"),
                ("reject", "reject_once"),
            ],
        );
        let (_, entry) = agent.last_permission();
        let Entry::Permission { options, .. } = entry else {
            panic!("not a permission");
        };
        let ids: Vec<&str> = options.iter().map(|o| o.id.as_str()).collect();
        assert_eq!(ids, ["allow", "reject"]);
        assert_eq!(agent.notices().len(), 1);
        assert!(agent.notices()[0].contains("allow_for_session"));
        assert_eq!(*agent.turn_state(), TurnState::Blocked);
    }

    #[test]
    fn a_request_with_no_option_brindle_knows_is_answered_cancelled() {
        let mut agent = Agent::ready();
        agent.turn("one");
        let out = agent.feed(&permission(
            json!(0),
            "call_1",
            &[("x", "allow_for_session")],
        ));
        assert_eq!(
            shape(&out.events),
            [
                "added tool",
                "added permission",
                "added notice",
                "answered",
                "added notice"
            ]
        );
        assert!(
            out.stdin
                .ends_with(b"\"outcome\":{\"outcome\":\"cancelled\"}}}\n")
        );
        assert_eq!(*agent.turn_state(), TurnState::Working);
    }

    #[test]
    fn a_permission_request_that_cant_be_read_is_answered_cancelled_by_its_id() {
        let mut agent = Agent::ready();
        agent.turn("one");
        let out = agent.feed(r#"{"jsonrpc":"2.0","id":"q","method":"session/request_permission","params":{"sessionId":"s1"}}"#);
        assert_eq!(
            out.stdin,
            b"{\"jsonrpc\":\"2.0\",\"id\":\"q\",\"result\":{\"outcome\":{\"outcome\":\"cancelled\"}}}\n"
        );
        assert_eq!(shape(&out.events), ["added notice"]);
    }

    #[test]
    fn a_permission_request_while_cancelling_is_answered_cancelled_at_once() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.cmd(Command::Cancel);
        let out = agent.feed(&permission(json!(0), "call_1", &ALLOW_REJECT));
        assert_eq!(
            shape(&out.events),
            ["added tool", "added permission", "answered"]
        );
        assert_eq!(*agent.turn_state(), TurnState::Cancelling);
    }
}
