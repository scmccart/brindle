use std::time::Instant;

use brindle_transcript::{AgentState, Entry, EntryId, Event, NoticeLevel};

use crate::acp::reduce::{self, AgentSession, BAD_RUN_QUIET, Input, Output, Phase};

use super::decode;
use super::format::Unreadable;

/// What the log of a session whose checkpoint couldn't be read says, and
/// its agent's failure.
pub const NOT_RESTORED: &str = "the conversation could not be restored";

/// A session whose checkpoint couldn't be read (decision 11): a log of one
/// notice saying why, the agent `Failed` with [`NOT_RESTORED`], and
/// [`Phase::Broken`], so its lines are ignored, prompts are refused and the
/// agent runs until the session is closed. `cwd` and `version` are the
/// session's directory and the core's version, which it never sends.
pub fn not_restored(why: &Unreadable, cwd: &str, version: &str) -> AgentSession {
    let mut session = AgentSession::new(cwd.to_owned(), version.to_owned());
    session.phase = Phase::Broken;
    let events = [
        Event::Added {
            id: EntryId::FIRST,
            entry: Entry::Notice {
                level: NoticeLevel::Error,
                text: format!("{NOT_RESTORED}: {}", brindle_transcript::clean(&why.to_string())),
            },
        },
        Event::Agent {
            state: AgentState::Failed {
                message: NOT_RESTORED.to_owned(),
            },
        },
    ];
    for event in &events {
        // A fresh log takes both: the notice is its first entry.
        let _ = session.transcript.apply(event);
    }
    session
}

/// An agent session adopted from its holder.
#[derive(Debug, Clone, PartialEq)]
pub struct Adopted {
    pub session: AgentSession,
    /// What adopting did, applied as [`reduce::step`]'s is: a truncated
    /// tail's notice and failed turn, a waiting prompt held, a never
    /// checkpointed session's `initialize`, and when to tick for a bad-line
    /// run that was waiting for its notice.
    pub output: Output,
    /// Whether the checkpoint was read, and why not.
    pub restored: Result<(), Unreadable>,
}

/// Adopts an agent session from its holder's `checkpoint`, whose tail was
/// `truncated` (its front cut, decision 11). The core then feeds the tail
/// through [`reduce::step`] as `Stdout`.
///
/// - A checkpoint that reads: the session as it was. After a truncated
///   tail, [`Input::OutputLost`] is stepped: a prompt awaiting its response
///   is forgotten and its turn `Failed`, a waiting prompt is held
///   (`OutputLost`), a held one stays as it was. One still spawning (the
///   core stores a continue's state with its spawn, change
///   `agents-session-recovery`) is started ([`Input::Spawned`]), so its
///   agent continues the conversation rather than start another.
/// - An empty one: the core never checkpointed the session, which it does
///   before it first writes to the agent, so the agent was sent nothing.
///   It is adopted as a new session in `cwd` by `version`, and started
///   ([`Input::Spawned`]: `initialize` goes out), after `OutputLost` when
///   the tail was truncated.
/// - One that doesn't read: [`not_restored`]. Its lines are ignored, so a
///   truncation changes nothing but where the next line starts.
pub fn adopt(
    checkpoint: &[u8],
    truncated: bool,
    cwd: &str,
    version: &str,
    now: Instant,
) -> Adopted {
    if checkpoint.is_empty() {
        let mut session = AgentSession::new(cwd.to_owned(), version.to_owned());
        let mut output = Output::default();
        if truncated {
            output = reduce::step(&mut session, Input::OutputLost, now);
        }
        merge(&mut output, reduce::step(&mut session, Input::Spawned, now));
        return Adopted {
            session,
            output,
            restored: Ok(()),
        };
    }
    match decode(checkpoint, now) {
        Ok(mut session) => {
            let mut output = if truncated {
                reduce::step(&mut session, Input::OutputLost, now)
            } else {
                Output {
                    wake: session.bad.map(|bad| bad.last + BAD_RUN_QUIET),
                    ..Output::default()
                }
            };
            // Stored with a continue's spawn, before its `SPAWNED`: the
            // agent was sent nothing, so it starts now, continuing.
            if let Phase::Spawning { .. } = session.phase {
                merge(&mut output, reduce::step(&mut session, Input::Spawned, now));
            }
            Adopted {
                session,
                output,
                restored: Ok(()),
            }
        }
        Err(why) => {
            let mut session = not_restored(&why, cwd, version);
            if truncated {
                session.lines.cut_front();
            }
            Adopted {
                session,
                output: Output::default(),
                restored: Err(why),
            }
        }
    }
}

/// `later`'s output after `earlier`'s, as one.
fn merge(earlier: &mut Output, later: Output) {
    earlier.events.extend(later.events);
    earlier.stdin.extend(later.stdin);
    if later.reply.is_some() {
        earlier.reply = later.reply;
    }
    earlier.checkpoint = match (earlier.checkpoint, later.checkpoint) {
        (Some(reduce::Moment::BeforeRequest), _) | (_, Some(reduce::Moment::BeforeRequest)) => {
            Some(reduce::Moment::BeforeRequest)
        }
        (first, None) => first,
        (_, Some(moment)) => Some(moment),
    };
    earlier.hang_up |= later.hang_up;
    earlier.wake = later.wake;
}

#[cfg(test)]
mod tests {
    use brindle_remote_protocol::serde_json::json;
    use brindle_transcript::{Continuation, Conversation, HoldReason, Slot, TurnState};

    use crate::acp::lines::Partial;
    use crate::acp::reduce::{Start, step};
    use crate::acp::rpc::Purpose;

    use super::super::harness::*;
    use super::super::*;
    use super::*;

    // --- adoption -------------------------------------------------------

    #[test]
    fn an_untruncated_adoption_changes_nothing() {
        let agent = Agent::busy();
        let bytes = encode(&agent.s).expect("it fits");
        let adopted = adopt(&bytes, false, "/elsewhere", "9.9.9", agent.now);
        assert_eq!(adopted.restored, Ok(()));
        assert_eq!(adopted.session, agent.s);
        assert_eq!(adopted.output, Output::default());
    }

    #[test]
    fn a_truncated_adoption_fails_the_pending_prompt_and_holds_the_waiting_one() {
        let agent = Agent::busy();
        let bytes = encode(&agent.s).expect("it fits");
        let adopted = adopt(&bytes, true, "/home/u/src", "0.3.0", agent.now);
        assert_eq!(adopted.restored, Ok(()));
        let session = &adopted.session;
        assert_eq!(
            *session.transcript.slot(),
            Slot::Held {
                text: "second".into(),
                reason: HoldReason::OutputLost
            }
        );
        assert_eq!(
            *session.transcript.turn(),
            TurnState::Failed {
                message: "output was lost while the terminal server restarted".into()
            }
        );
        assert!(
            !session
                .rpc
                .outgoing()
                .values()
                .any(|p| *p == Purpose::Prompt)
        );
        assert!(
            adopted
                .output
                .events
                .iter()
                .any(|e| matches!(e, Event::Queue { .. }))
        );
        assert!(matches!(session.lines.partial(), Partial::Resyncing { .. }));
    }

    #[test]
    fn a_truncated_adoption_keeps_a_held_prompt_as_it_was() {
        let mut agent = Agent::ready();
        agent.prompt("first");
        agent.prompt("second");
        agent.feed(&result(3, json!({"stopReason": "refusal"})));
        let held = agent.s.transcript.slot().clone();
        assert!(matches!(held, Slot::Held { .. }));
        assert_eq!(agent.prompt("third").reply, Some(Ok(())));
        let bytes = encode(&agent.s).expect("it fits");
        let adopted = adopt(&bytes, true, "/home/u/src", "0.3.0", agent.now);
        assert_eq!(*adopted.session.transcript.slot(), held);
        assert!(matches!(
            adopted.session.transcript.turn(),
            TurnState::Failed { .. }
        ));
        assert!(
            !adopted
                .output
                .events
                .iter()
                .any(|e| matches!(e, Event::Queue { .. }))
        );
    }

    #[test]
    fn a_session_never_checkpointed_is_adopted_new_and_started() {
        let adopted = adopt(&[], false, "/home/u/src", "0.3.0", Instant::now());
        assert_eq!(adopted.restored, Ok(()));
        assert_eq!(adopted.session.phase, Phase::Initializing { start: Start::New });
        assert_eq!(sent(&adopted.output.stdin)[0]["method"], "initialize");
        assert_eq!(
            adopted.output.checkpoint,
            Some(reduce::Moment::BeforeRequest)
        );
        assert_eq!(adopted.session.cwd, "/home/u/src");
    }

    /// A continue's state, stored with its spawn before the core saw its
    /// `SPAWNED` (agents-session-recovery 3.3): the adopted agent is sent
    /// `initialize` and then continues the conversation, never
    /// `session/new`.
    #[test]
    fn a_continue_stored_before_its_spawned_is_started_and_continues() {
        let mut agent = Agent::ready();
        agent.prompt("first");
        agent.feed(&result(3, json!({"stopReason": "end_turn"})));
        agent.step(Input::Exited {
            code: Some(1),
            stderr: b"",
        });
        assert_eq!(agent.step(Input::Restart).reply, Some(Ok(())));
        let entries = agent.s.transcript.entries().count();
        let bytes = encode(&agent.s).expect("it fits");
        let mut adopted = adopt(&bytes, false, "/elsewhere", "0.3.0", agent.now);
        assert_eq!(adopted.restored, Ok(()));
        assert!(matches!(adopted.session.phase, Phase::Initializing { .. }));
        assert_eq!(sent(&adopted.output.stdin)[0]["method"], "initialize");
        assert_eq!(adopted.output.checkpoint, Some(reduce::Moment::BeforeRequest));
        assert_eq!(adopted.session.transcript.entries().count(), entries, "the log is kept");
        let out = step(
            &mut adopted.session,
            Input::Stdout(&result(
                4,
                json!({"protocolVersion": 1, "agentCapabilities": {
                    "loadSession": true, "sessionCapabilities": {"resume": {}}}}),
            )),
            agent.now,
        );
        assert_eq!(sent(&out.stdin)[0]["method"], "session/resume");

        // A fresh continue's likewise.
        let conversation = Conversation {
            id: "s1".into(),
            cwd: "/w".into(),
            continuation: Continuation::Neither,
            title: None,
        };
        let fresh = AgentSession::continuing(conversation, "0.3.0".into());
        let bytes = encode(&fresh).expect("it fits");
        let mut adopted = adopt(&bytes, false, "/elsewhere", "0.3.0", agent.now);
        assert_eq!(sent(&adopted.output.stdin)[0]["method"], "initialize");
        let out = step(
            &mut adopted.session,
            Input::Stdout(&result(1, json!({"protocolVersion": 1, "agentCapabilities": {"loadSession": true}}))),
            agent.now,
        );
        let load = &sent(&out.stdin)[0];
        assert_eq!((load["method"].as_str(), load["params"]["cwd"].as_str()), (Some("session/load"), Some("/w")));
    }

    #[test]
    fn a_session_never_checkpointed_with_a_truncated_tail_says_so_then_starts() {
        let adopted = adopt(&[], true, "/home/u/src", "0.3.0", Instant::now());
        let notices: Vec<&Event> = adopted
            .output
            .events
            .iter()
            .filter(|e| {
                matches!(
                    e,
                    Event::Added {
                        entry: Entry::Notice { .. },
                        ..
                    }
                )
            })
            .collect();
        assert_eq!(notices.len(), 1);
        assert_eq!(sent(&adopted.output.stdin)[0]["method"], "initialize");
        assert!(matches!(
            adopted.session.lines.partial(),
            Partial::Resyncing { .. }
        ));
    }

    #[test]
    fn a_truncated_unreadable_checkpoint_only_resyncs_its_lines() {
        let adopted = adopt(
            b"not a checkpoint",
            true,
            "/home/u/src",
            "0.3.0",
            Instant::now(),
        );
        assert_not_restored(&adopted);
        assert!(matches!(
            adopted.session.lines.partial(),
            Partial::Resyncing { .. }
        ));
    }
}
