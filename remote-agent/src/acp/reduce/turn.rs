use brindle_remote_protocol::serde_json::Value;
use brindle_transcript::{Event, HoldReason, PermissionOutcome, Slot, StopReason, TurnState, clean};

use crate::acp::envelope::RpcError;
use crate::acp::forms::{Failure, Request};
use crate::acp::limits::MAX_PROMPT;

use super::lifecycle::{Ending, Phase};
use super::text::bound;
use super::{Moment, Refused, RunKind, Step};

impl Step<'_> {
    pub(super) fn prompted(&mut self, result: Result<Value, &RpcError>) {
        match result {
            Ok(result) => match self.s.rpc.dialect().prompt_result(&result) {
                Ok(StopReason::Other { name }) => self.end_turn(Ending::Stop(StopReason::Other {
                    name: bound(clean(&name)),
                })),
                Ok(stop) => self.end_turn(Ending::Stop(stop)),
                Err(_) => self.end_turn(Ending::Failed(
                    "the agent's answer to the prompt isn't one Brindle can read".to_owned(),
                )),
            },
            Err(error) if self.s.rpc.dialect().failure(error) == Failure::AuthRequired => {
                self.prompt_needs_login(error)
            }
            Err(error) => self.end_turn(Ending::Failed(bound(clean(&error.message)))),
        }
    }

    /// The prompt's answer ends the turn: a waiting prompt is sent after
    /// `end_turn` with the agent ready, and held after any other ending
    /// (decision 10).
    pub(super) fn end_turn(&mut self, ending: Ending) {
        self.s.process.run = None;
        let cancelling = *self.s.transcript.turn() == TurnState::Cancelling;
        let ready = self.readiness().is_ok();
        let waiting = match self.s.transcript.slot() {
            Slot::Waiting { text } => Some(text.clone()),
            Slot::Empty | Slot::Held { .. } => None,
        };
        let sends = ready && matches!(ending, Ending::Stop(StopReason::EndTurn));
        if let Some(text) = waiting {
            if sends {
                self.turn(ending.state());
                self.emit(Event::Queue { slot: Slot::Empty });
                self.mark(Moment::QueueChanged);
                self.send_prompt(text);
                return;
            }
            self.hold_waiting(hold_reason(&ending, cancelling));
        }
        self.turn(ending.state());
        self.mark(Moment::TurnEnded);
    }

    /// Sends `text` as a new turn: its user entry, the turn working, and
    /// the `session/prompt` line, checkpointed before it is written.
    fn send_prompt(&mut self, text: String) {
        let Phase::Ready { session } = &self.s.phase else {
            return;
        };
        let session = session.clone();
        self.add_text(RunKind::User, &text);
        // A prompt is no run the agent's chunks extend.
        self.s.process.run = None;
        self.turn(TurnState::Working);
        let line = self.s.rpc.prompt(&session, &text);
        self.out.stdin.extend(line);
        self.mark(Moment::BeforeRequest);
    }

    pub(super) fn prompt(&mut self, text: &str) -> Result<(), Refused> {
        self.readiness()?;
        let text = clean(text);
        if text.len() > MAX_PROMPT {
            return Err(Refused::TooLong { len: text.len() });
        }
        if !self.running() {
            // A held prompt stays held, untouched (decision 10).
            self.send_prompt(text);
            return Ok(());
        }
        match self.s.transcript.slot() {
            Slot::Empty => {
                self.emit(Event::Queue {
                    slot: Slot::Waiting { text },
                });
                self.mark(Moment::QueueChanged);
                Ok(())
            }
            Slot::Waiting { .. } | Slot::Held { .. } => Err(Refused::AlreadyQueued),
        }
    }

    pub(super) fn unqueue(&mut self) -> Result<(), Refused> {
        match self.s.transcript.slot() {
            Slot::Empty => Err(Refused::NothingQueued),
            Slot::Waiting { .. } | Slot::Held { .. } => {
                self.emit(Event::Queue { slot: Slot::Empty });
                self.mark(Moment::QueueChanged);
                Ok(())
            }
        }
    }

    pub(super) fn resend(&mut self) -> Result<(), Refused> {
        let Slot::Held { text, .. } = self.s.transcript.slot() else {
            return Err(Refused::NoneHeld);
        };
        let text = text.clone();
        self.readiness()?;
        if self.running() {
            self.emit(Event::Queue {
                slot: Slot::Waiting { text },
            });
            self.mark(Moment::QueueChanged);
        } else {
            self.emit(Event::Queue { slot: Slot::Empty });
            self.mark(Moment::QueueChanged);
            self.send_prompt(text);
        }
        Ok(())
    }

    /// Decision 8: a waiting prompt is held first, then `session/cancel`,
    /// then `cancelled` for each waiting permission request, and the turn
    /// is cancelling until the prompt's answer ends it. With no prompt out
    /// (decision 11's known gap) the turn ends here. While logging in, it
    /// cancels the login (`agents-login` decision 4).
    pub(super) fn cancel(&mut self) {
        if let Phase::LoggingIn { login, .. } = &self.s.phase {
            let login = login.clone();
            return self.cancel_login(login);
        }
        let Phase::Ready { session } = &self.s.phase else {
            return;
        };
        let session = session.clone();
        match self.s.transcript.turn() {
            TurnState::Working | TurnState::Blocked => {}
            TurnState::Idle
            | TurnState::Cancelling
            | TurnState::Ended { .. }
            | TurnState::Failed { .. } => return,
        }
        self.hold_waiting(HoldReason::Cancelled);
        self.out.stdin.extend(
            self.s
                .rpc
                .dialect()
                .encode(Request::Cancel { session: &session }),
        );
        for (entry, answer) in self.s.rpc.cancel_permissions() {
            self.out.stdin.extend(answer);
            self.emit(Event::Answered {
                id: entry,
                outcome: PermissionOutcome::Cancelled,
            });
        }
        if self.prompt_awaited() {
            self.turn(TurnState::Cancelling);
            self.mark(Moment::Permission);
        } else {
            self.s.process.run = None;
            self.turn(TurnState::Ended {
                stop: StopReason::Cancelled,
            });
            self.mark(Moment::TurnEnded);
        }
    }
}

/// Why a waiting prompt is held when its turn ends as `ending` (decision
/// 10). A `cancelled` stop the user didn't ask for (Codex ends a turn so
/// after a rejected permission) is a stop reason, not the user's cancel.
fn hold_reason(ending: &Ending, cancelling: bool) -> HoldReason {
    match ending {
        // Reached only when the agent isn't ready.
        Ending::Stop(StopReason::EndTurn) => HoldReason::AgentStopped,
        Ending::Stop(StopReason::Cancelled) if cancelling => HoldReason::Cancelled,
        Ending::Stop(stop) => HoldReason::Stopped { stop: stop.clone() },
        Ending::Failed(_) => HoldReason::Failed,
    }
}

#[cfg(test)]
mod tests {
    use brindle_remote_protocol::serde_json::json;

    use super::super::harness::*;
    use super::super::*;
    use super::*;

    // --- prompts --------------------------------------------------------

    #[test]
    fn a_prompt_when_idle_is_sent_as_a_turn() {
        let mut agent = Agent::ready();
        let out = agent.prompt("hi");
        assert_eq!(out.reply, Some(Ok(())));
        assert_eq!(shape(&out.events), ["added user", "turn working"]);
        assert_eq!(
            out.stdin,
            b"{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"session/prompt\",\"params\":{\"sessionId\":\"s1\",\"prompt\":[{\"type\":\"text\",\"text\":\"hi\"}]}}\n"
        );
        assert_eq!(out.checkpoint, Some(Moment::BeforeRequest));
    }

    #[test]
    fn a_turn_gives_the_user_message_the_agent_message_and_the_end_in_order() {
        let mut agent = Agent::ready();
        let id = agent.turn("hi");
        agent.feed(&said("hello"));
        let out = agent.end(id, "end_turn");
        assert_eq!(out.checkpoint, Some(Moment::TurnEnded));
        assert_eq!(
            agent.entries(),
            [
                Entry::User { text: "hi".into() },
                Entry::Agent {
                    text: "hello".into()
                }
            ]
        );
        assert_eq!(*agent.turn_state(), ended(StopReason::EndTurn));
    }

    #[test]
    fn chunks_of_one_kind_merge_into_one_entry() {
        let mut agent = Agent::ready();
        agent.turn("hi");
        agent.feed(&said("Hel"));
        agent.feed(&said("lo"));
        let out = agent.feed(&said("!"));
        assert_eq!(shape(&out.events), ["appended"]);
        assert_eq!(
            agent.last_entry().1,
            Entry::Agent {
                text: "Hello!".into()
            }
        );
    }

    #[test]
    fn a_prompt_during_a_turn_waits_in_the_slot() {
        let mut agent = Agent::ready();
        agent.turn("one");
        let out = agent.prompt("two");
        assert_eq!(out.reply, Some(Ok(())));
        assert_eq!(*agent.slot(), waiting("two"));
        assert_eq!(out.checkpoint, Some(Moment::QueueChanged));
        assert!(out.stdin.is_empty());
    }

    #[test]
    fn a_third_prompt_is_refused_as_already_queued() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.prompt("two");
        let out = agent.prompt("three");
        assert_eq!(out.reply, Some(Err(Refused::AlreadyQueued)));
        assert!(out.events.is_empty() && out.stdin.is_empty());
        assert_eq!(*agent.slot(), waiting("two"));
    }

    #[test]
    fn a_prompt_over_256_kib_is_refused_naming_the_limit() {
        let mut agent = Agent::ready();
        let out = agent.prompt(&"x".repeat(300 * 1024));
        assert_eq!(out.reply, Some(Err(Refused::TooLong { len: 300 * 1024 })));
        assert!(
            Refused::TooLong { len: 300 * 1024 }
                .to_string()
                .contains("256 KiB")
        );
        assert!(out.events.is_empty());
        assert!(agent.prompt(&"x".repeat(MAX_PROMPT)).reply == Some(Ok(())));
    }

    #[test]
    fn a_prompt_error_fails_the_turn_with_the_agents_message() {
        let mut agent = Agent::ready();
        let id = agent.turn("hi");
        agent.feed(&error(id, -32603, "rate limited"));
        assert_eq!(
            *agent.turn_state(),
            TurnState::Failed {
                message: "rate limited".into()
            }
        );
        assert_eq!(
            agent.prompt("again").reply,
            Some(Ok(())),
            "the agent is still ready"
        );
    }

    #[test]
    fn a_login_demanded_by_a_prompt_fails_the_turn_holds_the_waiting_prompt_then_needs_the_login() {
        let mut agent = Agent::ready();
        let id = agent.turn("one");
        assert_eq!(id, 3, "the fixture answers id 3");
        agent.prompt("two");
        let out = agent.feed(&fixture_line("prompt-auth-required", 0));
        assert_eq!(
            shape(&out.events),
            ["queue held", "turn failed", "agent login required"]
        );
        assert_eq!(*agent.slot(), held("two", HoldReason::LoginRequired));
        assert_eq!(
            *agent.turn_state(),
            TurnState::Failed {
                message: "Authentication required".into()
            }
        );
        assert_eq!(
            *agent.agent_state(),
            AgentState::LoginRequired {
                message: "Authentication required".into(),
                methods: Vec::new(),
            }
        );
        assert_eq!(
            agent
                .sent_methods()
                .iter()
                .filter(|m| **m == "session/prompt")
                .count(),
            1
        );
    }

    #[test]
    fn once_a_login_is_needed_prompts_and_resends_are_refused_and_unqueue_still_discards() {
        let mut agent = Agent::ready();
        let id = agent.turn("one");
        agent.prompt("two");
        agent.feed(&error(id, -32000, "session expired"));
        assert_eq!(
            agent.prompt("three").reply,
            Some(Err(Refused::LoginRequired))
        );
        assert_eq!(
            agent.cmd(Command::Resend).reply,
            Some(Err(Refused::LoginRequired))
        );
        assert_eq!(agent.cmd(Command::Unqueue).reply, Some(Ok(())));
        assert_eq!(*agent.slot(), Slot::Empty);
    }

    #[test]
    fn a_session_whose_handling_failed_refuses_prompts() {
        let mut agent = Agent::ready();
        agent.step(Input::Fault {
            reason: "boom".into(),
        });
        let refused = agent.prompt("hi").reply;
        assert_eq!(
            refused,
            Some(Err(Refused::Failed {
                message: "ACP handling failed: boom".into()
            }))
        );
    }

    #[test]
    fn a_prompt_after_the_agent_exited_is_refused() {
        let mut agent = Agent::ready();
        agent.step(Input::Exited {
            code: Some(0),
            stderr: b"",
        });
        assert_eq!(agent.prompt("hi").reply, Some(Err(Refused::Exited)));
    }

    // --- the slot (decision 10's table, row by row) ---------------------

    #[test]
    fn a_waiting_prompt_is_sent_after_end_turn_as_the_next_turn() {
        let mut agent = Agent::ready();
        let first = agent.turn("one");
        agent.prompt("two");
        let out = agent.end(first, "end_turn");
        assert_eq!(
            shape(&out.events),
            ["turn ended", "queue empty", "added user", "turn working"]
        );
        assert_eq!(agent.prompt_id(), first + 1);
        assert_eq!(
            agent.last("session/prompt")["params"]["prompt"][0]["text"],
            "two"
        );
        assert_eq!(out.checkpoint, Some(Moment::BeforeRequest));
    }

    #[test]
    fn a_cancel_holds_a_waiting_prompt_before_session_cancel_is_sent() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.prompt("two");
        let out = agent.cmd(Command::Cancel);
        assert_eq!(shape(&out.events), ["queue held", "turn cancelling"]);
        assert_eq!(*agent.slot(), held("two", HoldReason::Cancelled));
        assert_eq!(agent.sent_methods().last(), Some(&"session/cancel"));
    }

    #[test]
    fn a_failed_turn_holds_a_waiting_prompt() {
        let mut agent = Agent::ready();
        let id = agent.turn("one");
        agent.prompt("two");
        let out = agent.feed(&error(id, -32603, "rate limited"));
        assert_eq!(shape(&out.events), ["queue held", "turn failed"]);
        assert_eq!(*agent.slot(), held("two", HoldReason::Failed));
        assert_eq!(
            agent
                .sent_methods()
                .iter()
                .filter(|m| **m == "session/prompt")
                .count(),
            1
        );
    }

    #[test]
    fn every_other_stop_reason_holds_a_waiting_prompt_with_that_reason() {
        for (name, stop) in [
            ("max_tokens", StopReason::MaxTokens),
            ("max_turn_requests", StopReason::MaxTurnRequests),
            ("refusal", StopReason::Refusal),
            (
                "paused_for_review",
                StopReason::Other {
                    name: "paused_for_review".into(),
                },
            ),
        ] {
            let mut agent = Agent::ready();
            let id = agent.turn("one");
            agent.prompt("two");
            let out = agent.end(id, name);
            assert_eq!(shape(&out.events), ["queue held", "turn ended"], "{name}");
            assert_eq!(
                *agent.slot(),
                held("two", HoldReason::Stopped { stop: stop.clone() })
            );
            assert_eq!(*agent.turn_state(), ended(stop));
        }
    }

    #[test]
    fn a_cancelled_stop_nobody_asked_for_holds_as_a_stop_reason_not_as_a_cancel() {
        let mut agent = Agent::ready();
        let id = agent.turn("one");
        agent.prompt("two");
        agent.end(id, "cancelled");
        assert_eq!(
            *agent.slot(),
            held(
                "two",
                HoldReason::Stopped {
                    stop: StopReason::Cancelled
                }
            )
        );
    }

    #[test]
    fn an_agent_exit_holds_a_waiting_prompt_before_the_exit() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.prompt("two");
        let out = agent.step(Input::Exited {
            code: Some(1),
            stderr: b"",
        });
        assert_eq!(
            shape(&out.events),
            ["queue held", "turn failed", "agent exited"]
        );
        assert_eq!(*agent.slot(), held("two", HoldReason::AgentStopped));
    }

    #[test]
    fn a_handling_fault_holds_a_waiting_prompt_before_the_failure() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.prompt("two");
        let out = agent.step(Input::Fault {
            reason: "boom".into(),
        });
        assert_eq!(
            shape(&out.events),
            ["queue held", "turn failed", "agent failed"]
        );
        assert_eq!(*agent.slot(), held("two", HoldReason::AgentStopped));
    }

    #[test]
    fn lost_output_fails_the_turn_and_holds_a_waiting_prompt() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.prompt("two");
        let out = agent.step(Input::OutputLost);
        assert_eq!(
            shape(&out.events),
            ["added notice", "queue held", "turn failed"]
        );
        assert_eq!(*agent.slot(), held("two", HoldReason::OutputLost));
        assert_eq!(
            *agent.turn_state(),
            TurnState::Failed {
                message: "output was lost while the terminal server restarted".into()
            }
        );
        assert!(
            agent.s.rpc().outgoing().is_empty(),
            "the prompt is forgotten"
        );
        let next = agent.feed(&said("tail of a lost line"));
        assert!(
            next.events.is_empty(),
            "the rest of the cut line is dropped"
        );
        assert_eq!(
            agent.turn("three"),
            4,
            "the next prompt works, its id never reused"
        );
    }

    #[test]
    fn unqueue_discards_a_waiting_prompt_which_is_then_never_sent() {
        let mut agent = Agent::ready();
        let id = agent.turn("one");
        agent.prompt("two");
        let out = agent.cmd(Command::Unqueue);
        assert_eq!(out.reply, Some(Ok(())));
        assert_eq!(shape(&out.events), ["queue empty"]);
        agent.end(id, "end_turn");
        assert_eq!(
            agent
                .sent_methods()
                .iter()
                .filter(|m| **m == "session/prompt")
                .count(),
            1
        );
    }

    #[test]
    fn unqueue_discards_a_held_prompt() {
        let mut agent = Agent::ready();
        let id = agent.turn("one");
        agent.prompt("two");
        agent.end(id, "max_tokens");
        assert_eq!(agent.cmd(Command::Unqueue).reply, Some(Ok(())));
        assert_eq!(*agent.slot(), Slot::Empty);
    }

    #[test]
    fn resend_when_idle_sends_the_held_prompt_as_a_new_turn() {
        let mut agent = Agent::ready();
        let id = agent.turn("one");
        agent.prompt("two");
        agent.end(id, "refusal");
        let out = agent.cmd(Command::Resend);
        assert_eq!(out.reply, Some(Ok(())));
        assert_eq!(
            shape(&out.events),
            ["queue empty", "added user", "turn working"]
        );
        assert_eq!(
            agent.last("session/prompt")["params"]["prompt"][0]["text"],
            "two"
        );
    }

    #[test]
    fn resend_during_a_turn_queues_the_held_prompt_again() {
        let mut agent = Agent::ready();
        let id = agent.turn("one");
        agent.prompt("two");
        agent.end(id, "refusal");
        let third = agent.turn("three");
        let out = agent.cmd(Command::Resend);
        assert_eq!(out.reply, Some(Ok(())));
        assert_eq!(*agent.slot(), waiting("two"));
        agent.end(third, "end_turn");
        assert_eq!(
            agent.last("session/prompt")["params"]["prompt"][0]["text"],
            "two"
        );
    }

    #[test]
    fn resend_while_the_agent_isnt_ready_is_refused_and_keeps_it_held() {
        // A held prompt while starting comes only from a restored session.
        let mut agent = Agent::new();
        agent.step(Input::Spawned);
        agent
            .s
            .transcript
            .apply(&Event::Queue {
                slot: held("two", HoldReason::OutputLost),
            })
            .expect("a queue event");
        agent.client = agent.s.transcript.clone();
        let out = agent.cmd(Command::Resend);
        assert_eq!(out.reply, Some(Err(Refused::NotReady)));
        assert!(out.events.is_empty());
        assert_eq!(*agent.slot(), held("two", HoldReason::OutputLost));

        let mut broken = Agent::ready();
        let id = broken.turn("one");
        broken.prompt("two");
        broken.end(id, "max_tokens");
        broken.step(Input::Fault {
            reason: "boom".into(),
        });
        assert!(matches!(
            broken.cmd(Command::Resend).reply,
            Some(Err(Refused::Failed { .. }))
        ));
        assert!(matches!(broken.slot(), Slot::Held { .. }));
    }

    #[test]
    fn a_new_prompt_with_one_held_and_no_turn_is_sent_and_leaves_it_held() {
        let mut agent = Agent::ready();
        let id = agent.turn("one");
        agent.prompt("two");
        agent.end(id, "max_tokens");
        let out = agent.prompt("three");
        assert_eq!(out.reply, Some(Ok(())));
        assert_eq!(shape(&out.events), ["added user", "turn working"]);
        assert_eq!(
            *agent.slot(),
            held(
                "two",
                HoldReason::Stopped {
                    stop: StopReason::MaxTokens
                }
            )
        );
    }

    #[test]
    fn a_new_prompt_with_one_waiting_or_held_during_a_turn_is_refused() {
        let mut agent = Agent::ready();
        let id = agent.turn("one");
        agent.prompt("two");
        assert_eq!(agent.prompt("x").reply, Some(Err(Refused::AlreadyQueued)));
        agent.end(id, "max_tokens");
        agent.turn("three");
        assert_eq!(agent.prompt("x").reply, Some(Err(Refused::AlreadyQueued)));
        assert!(matches!(agent.slot(), Slot::Held { .. }));
    }

    #[test]
    fn unqueue_or_resend_with_an_empty_slot_is_refused() {
        let mut agent = Agent::ready();
        assert_eq!(
            agent.cmd(Command::Unqueue).reply,
            Some(Err(Refused::NothingQueued))
        );
        assert_eq!(
            agent.cmd(Command::Resend).reply,
            Some(Err(Refused::NoneHeld))
        );
        assert_eq!(Refused::NothingQueued.to_string(), "nothing is queued");
        assert_eq!(Refused::NoneHeld.to_string(), "no prompt is held");
    }

    #[test]
    fn resend_of_a_waiting_prompt_is_refused() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.prompt("two");
        assert_eq!(
            agent.cmd(Command::Resend).reply,
            Some(Err(Refused::NoneHeld))
        );
        assert_eq!(*agent.slot(), waiting("two"));
    }

    // --- cancel ---------------------------------------------------------

    #[test]
    fn cancel_sends_session_cancel_then_cancels_each_waiting_permission() {
        let mut agent = Agent::ready();
        agent.turn("one");
        let entry = agent.ask(json!(0), &ALLOW_REJECT);
        let out = agent.cmd(Command::Cancel);
        assert_eq!(out.reply, None);
        assert_eq!(
            out.stdin,
            b"{\"jsonrpc\":\"2.0\",\"method\":\"session/cancel\",\"params\":{\"sessionId\":\"s1\"}}\n{\"jsonrpc\":\"2.0\",\"id\":0,\"result\":{\"outcome\":{\"outcome\":\"cancelled\"}}}\n"
        );
        assert_eq!(shape(&out.events), ["answered", "turn cancelling"]);
        assert!(matches!(
            agent.s.transcript.entry(entry),
            Some(Entry::Permission {
                outcome: Some(PermissionOutcome::Cancelled),
                ..
            })
        ));
    }

    #[test]
    fn a_cancelled_turn_ends_when_the_agent_answers_the_prompt() {
        let mut agent = Agent::ready();
        let id = agent.turn("one");
        agent.cmd(Command::Cancel);
        assert_eq!(*agent.turn_state(), TurnState::Cancelling);
        agent.end(id, "cancelled");
        assert_eq!(*agent.turn_state(), ended(StopReason::Cancelled));
    }

    #[test]
    fn a_cancel_with_no_turn_running_produces_nothing() {
        let mut agent = Agent::ready();
        assert_eq!(agent.cmd(Command::Cancel), Output::default());
        let id = agent.turn("one");
        agent.end(id, "end_turn");
        assert_eq!(agent.cmd(Command::Cancel), Output::default());
    }

    #[test]
    fn a_cancel_with_no_prompt_out_ends_the_turn_here() {
        let mut agent = Agent::ready();
        agent.turn("one");
        agent.s.rpc.forget(Purpose::Prompt);
        let out = agent.cmd(Command::Cancel);
        assert_eq!(*agent.turn_state(), ended(StopReason::Cancelled));
        assert_eq!(out.checkpoint, Some(Moment::TurnEnded));
    }

}
