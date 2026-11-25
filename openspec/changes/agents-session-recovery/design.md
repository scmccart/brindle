# Design

## Context

See proposal.md for why, and the specs in this change for what. Vocabulary as in `agents-sessions`: "the core" is the `serve` role, "the holder" the `hold` role, "the reducer" `remote-agent/src/acp/reduce.rs`'s `step`, "the zone" an agent zone (`src/agent/`). "Conversation" means the agent's ACP session (its `sessionId`), to keep it apart from the terminal server's session.

Facts from the code as built (`feat/agents-profiles`, 2026-11-25) that shape the design:

- **The reducer forgets the ACP session id at an exit.** `Phase::Ready { session }` holds it; `exited` sets `Phase::Exited`, which holds nothing, and the transcript never had it (`AgentState::Ready { name, version }`). The checkpoint keeps the `Phase`, so a running session's id survives a core restart today, but nothing outside the reducer can read it.
- **`initialize`'s capabilities are dropped.** `wire::initialize_result` keeps only `protocolVersion` and `agentInfo`. The schema crate's v1 types (1.22.0, no feature needed) have `agentCapabilities.loadSession: bool`, `sessionCapabilities.resume`, `LoadSessionRequest`/`Response` and `ResumeSessionRequest`/`Response`; `session/resume` is stable v1 in this pin.
- **What the captured agents offer** (`remote-agent/tests/fixtures/acp/`, `initialize` replies): `loadSession: true` in all of Antigravity 1.5.16, claude-agent-acp 0.112.0, claude-code-acp 0.16.2, Codex ACP 2.12.0 and Copilot 1.0.115; `sessionCapabilities.resume` in all but Copilot. Gemini's capture ends at its login. So resume is the common path, Copilot's is load-only, and every captured agent can be continued.
- **An exit ends the session.** `Core::session_exited` steps `Input::Exited`, then `policy::exit_fate`: `KeepForOwner` (owner away, `Life::ExitedAwaitingOwner`, which sends only `EXIT` when the owner returns) or `Report` (`EXIT` to the client, session removed). A lost holder (`session_lost`) steps `Exited { code: None }` and sends `EXIT { lost }`.
- **The holder forgets a pipe session once it has sent its `EXITED`** (`holder.rs` `remove_session`), and refuses a `SPAWN_PIPES` for an id it still holds (`policy::pipe_spawn_refusal`). Session ids are the core's (`next_session`, `free_id`); the holder is keyed by them. So the core can start a new program under an ended session's id with a plain `SPAWN_PIPES`, on any holder of its build: H1 needs nothing new.
- **The client keeps an exited zone's conversation.** `ChatModel` holds its own folded `Transcript`; `AgentPhase::Live(AgentState::Exited { .. })` with `SessionLife::Ended` after `EXIT`, `AgentPhase::Lost { reason }` after `EXIT { lost }` or a failed reattach. `queue_row`/`resend_of` enable send-again only on `Live(Ready)` with `SessionLife::Open`.
- **Client protocol 4 and holder protocol H1 3 are unreleased.** They come with `agents-sessions` and `agents-profiles` (PRs #122, #123), and every `agents-*` change ships in the same release (user decision, cross-cutting). `process R5`'s "a format that has shipped is frozen" therefore doesn't bind them yet.

## Goals / Non-Goals

**Goals:**
- A user-started "Continue conversation" for an agent that exited or was lost with its holder, on the same terminal-server session, keeping the log, entry ids and a held prompt.
- `session/resume` when it is offered; `session/load` with its replay muted when only load is offered; `session/load` folded into a fresh log when the server no longer has the session.
- No environment secrets stored by the server for recovery; a profile edit applies to the next continue.
- Everything testable against `brindle-fake-acp-agent` through the helper's protocols, and in named e2e cases.

**Non-Goals:**
- Automatic restarts (user decision 1). A pure backoff policy can be added later without changing the wire.
- Recording exited zones for session restore (user decision 4).
- `session/list` and picking a conversation Brindle never had: the proposal-only change `agents-session-list`.
- ACP v2's `session/resume { replayFrom }` (`agents-acp-v2`).
- Resubmitting a turn that was in flight at the exit, or offering to (user decision 5).
- In-zone login (`agents-login`), which reuses this change's restart (decision 9).
- Keeping an ended session across a core restart or upgrade (decision 6).

## Layer map

| Part | Kind | Crate and module |
| --- | --- | --- |
| `Event::Conversation`, `Conversation`, `Continuation`, `AgentState::Lost`, `Transcript::conversation`, their JSON forms and v4 fixtures | pure (types + fold) | `brindle-transcript` (`transcript/`) |
| `AGENT_RESTART` (21), `SpawnAgent.conversation` | codec | `brindle-remote-protocol` `client.rs` |
| `initialize`'s capabilities, `session/resume` and `session/load` requests and answers | pure | `brindle-remote-agent` `acp/wire.rs`, `acp/rpc.rs` (`Purpose::{Resume, Load}`) |
| The start choice, the muted replay, `Input::{Restart, Lost}`, `AgentSession::continuable` | pure, takes `now` | `acp/reduce.rs` |
| The checkpoint's new fields | pure | `acp/checkpoint.rs` |
| What an exit or a lost holder does to the session, what a detach does to an ended one, restart refusals | pure | `server/core/policy.rs` (`end_fate`, `restart_refusal`) |
| `Life::AgentEnded`, `SpawnRequest::Restart`, handling `AGENT_RESTART` | shell (loop handlers) | `server/core/agents.rs`, `core/sessions.rs`, `core/clients.rs` |
| The holder | unchanged | — |
| `Offer` and `continue_offer`, the phase after a continue, the prompt's reasons, dump words | pure (GPUI-free model) | `src/agent/model.rs` |
| The restart request's command, args and env from the profile | pure | `src/agent/spawn.rs` (`agent_spawn_request`, reused) |
| `AgentLink::restart`, a continuing `SPAWN_AGENT`, the zone's session swap | shell | `src/remote/` (the agent link), `src/agent/mod.rs` |
| The Continue conversation button, `agent_continue` | shell (view, action) | `src/agent/view.rs`, `src/actions.rs`, `src/zone_view.rs`/`src/workspace.rs` |
| Fake agent | test binary | `remote-agent/src/fake_acp/` (no new step, decision 11) |

References keep pointing one way (`ports R1`): the transcript's new types are shared through `brindle-transcript`; ACP stays in `remote-agent/src/acp/` (`only_the_acp_modules_name_the_schema_crate`); the app never sees `loadSession` or `resume`, only the transcript's `Continuation`.

## Decisions

### 1. The user starts it (user decision 1)

A zone whose agent exited or was lost shows a **Continue conversation** button in the prompt input's reason line, when the model's offer says it can (decision 8), and the palette action `agent_continue` ("Continue Agent Conversation") does the same for the active agent zone. Nothing restarts on its own: a crashing agent stays visible with its exit status and standard error, and there is no loop to bound. *Rejected:* automatic restarts with a limit (an agent that crashes on its first line restarts invisibly), and automatic once then the user (two behaviours to explain).

### 2. Resume, muted load, or load into a fresh log (user decision 2)

The choice is the reducer's, made **after the new agent's `initialize`**, from what that agent offers, not from what the old one did: the user may have updated the agent or edited the profile.

| The server has the log | The new agent offers | Sent | The replay |
| --- | --- | --- | --- |
| yes (`AGENT_RESTART`) | `session/resume` (with or without load) | `session/resume` | none |
| yes | `session/load` only (Copilot) | `session/load` | muted: counted, not applied; a notice after the answer |
| no (`SPAWN_AGENT` with `conversation`) | `session/load` (with or without resume) | `session/load` | folded into the new, empty log as ordinary updates |
| no | `session/resume` only | `session/resume` | none; a warning notice that the earlier messages aren't shown |
| either | neither | nothing | `Failed`, hang-up (decision 7) |

- **Entry ids continue from the kept log** for free: the `Transcript` keeps its next id, and nothing from a muted replay is added. A fresh log starts at 1 in a new terminal-server session, which the zone shows in place of its old copy (decision 8).
- **Muting** is the reducer's `Phase::Loading { replay: Replay::Muted { updates } }`: every `session/update` notification before `session/load`'s response increments `updates` and is otherwise dropped, the chunk run is untouched, and the response adds `Notice { Info, "conversation continued; the agent's replay of N updates wasn't shown" }`. Requests from the agent during the load (a permission, `fs/*`) are handled as at any time. `Replay::Shown` applies updates as `session/new`'s session would, and starts the log with `Notice { Info, "this continues an earlier conversation; what follows is the agent's replay of it" }`.
- **Turn state during a load** stays `Idle`: replayed chunks are entries, not a turn, and `session/load`'s response ends no turn.
- *Rejected:* always loading and reconciling duplicates by `messageId` (v1 doesn't guarantee message ids, and replays differ in chunking); resume into a kept log *and* load when both are offered (two requests for one continue).

### 3. What the core keeps, and where (user decision 3)

**The conversation becomes transcript state.** A new event `Event::Conversation { conversation: Conversation }` with

```rust
pub struct Conversation { pub id: String, pub cwd: String, pub continuation: Continuation }
pub enum Continuation { ResumeAndLoad, Resume, Load, Neither }   // what `initialize` offered
```

is emitted when `session/new`, `session/resume` or `session/load` succeeds (before the `Agent { Ready }` event), held by `Transcript` (`Transcript::conversation() -> Option<&Conversation>`), and replayed right before `Turn` and `Agent`. `Continuation` is one closed enum rather than two bools (`types R1`), so "offers load" and "offers resume" can't disagree with each other. It travels to clients because the zone needs it twice: to decide whether to offer a continue (decision 8), and to continue in a new session when the server lost the old one (the id and the directory). The core also needs it after an exit, which is why it can't stay in `Phase::Ready`.

**An ended agent session stays in the core's memory** as `Life::AgentEnded { code }` (decision 5 says when), with everything it had: the `AgentSession` (log, slot, conversation, `cwd`, negotiated version), its `meta` (key, profile, owner, policy, layout) and its ownership. Its holder has forgotten it. *Rejected:* also storing the spawn spec in the core (environment secrets in the core and its checkpoints; a profile edit wouldn't apply); a log file in the server directory (new IO, a new file format under `process R15`, and the restart case it covers is decision 6's).

**The client sends what to start.** `AGENT_RESTART { req, session, command, args, env }` (client → core, type 21, protocol 4), built by the zone from its agent profile through `agent_spawn_request`, the same function a new zone uses, so the environment rules (no `TERM` and friends) are one. The directory is not sent: the conversation's `cwd` is the session's, and ACP ties a conversation to its directory. The reply is `OK` once the core has queued the spawn, or `ERROR` naming the session (decision 7's refusals); the outcome arrives as transcript events. A profile that is gone means no recovery (decision 8), as `agents-profiles` decision 4 has it for splits.

**Retention** (`policy::end_fate`, pure): an ended session that can be continued (`AgentSession::continuable`: a conversation whose `continuation` isn't `Neither`, and an agent that didn't fail its start) is kept while its client has it attached and while its owner may return (`Lingering`, `AwaitingOwner`), as `ExitedAwaitingOwner` is today; a returning owner gets its replay instead of an `EXIT`. A `DETACH` of one, its owner's grace running out, a `CLOSE`, `KILL` or `STOP` end it (`EXIT` to the client if it has one, then removed). Everything else keeps today's path (`ExitFate`). So "until the zone closes" for an attached zone, and "as exits are kept for owners today" for one whose client is away. A quit detaches it, which ends it: Brindle doesn't restore it anyway (decision 10).

**A lost holder** (`session_lost`) steps the new `Input::Lost`, which ends the agent as an exit does but sets `AgentState::Lost { reason: "its holder ended" }`; then `end_fate` decides as for an exit. A continuable session stays (`Life::AgentEnded { code: None }`), so a holder crash, which loses the agent's process but not the core, is recoverable. The `EXIT { lost }` path stays for sessions that can't be continued, and for terminal sessions.

**Checkpoint fields, additive** (format 1, `process R15`; unreleased, so any reader meets them): the conversation (inside the replayed log, as an event), the new `Phase` variants with their data (`Spawning { start }`, `Resuming`, `Loading { replay }`), and `Rpc`'s new purposes. A format-1 checkpoint without them reads as before (`start: New`, no conversation).

### 4. The reducer's start (user decision 8: v1 only)

`Phase::Spawning` gains `start: Start`, with `enum Start { New, Continue { conversation: Conversation, log: Log } }` and `enum Log { Kept, Fresh }`. After `initialize` succeeds the reducer reads the new agent's `Continuation` (from `loadSession` and `sessionCapabilities.resume`, in `wire::initialize_result`) and, for `New`, sends `session/new` as today; for `Continue`, picks by decision 2's table and sends `session/resume` (`Phase::Resuming`) or `session/load` (`Phase::Loading { replay }`), each `{ sessionId, cwd, mcpServers: [] }`, through `Rpc::{resume, load}` with `Purpose::{Resume, Load}`. Their answers:
- success: `Conversation` (same id, the new agent's `Continuation`), the load notice when muted, then `Agent { Ready { name, version } }` from `initialize`'s `agentInfo`, `Phase::Ready { session }`, `Moment::Started`;
- `-32000`: `LoginRequired`, as for `session/new` (decision 17 of `agents-sessions`);
- any other error or an unreadable answer: `refuse("the agent couldn't continue the conversation: …")`.

`Input::Restart` (from `AGENT_RESTART`) is accepted only in `Phase::Exited` or `Phase::Lost` with a continuable conversation: it sets `Phase::Spawning { start: Continue { log: Kept } }`, adds `Notice { Info, "the agent was started again to continue the conversation" }`, and sets `Agent { Starting }`. It asks for no checkpoint: the session has no holder until the spawn is placed, and `Spawned` then checkpoints before `initialize` as for a new session (`Moment::BeforeRequest`). A continuing `SPAWN_AGENT` creates the session with `start: Continue { log: Fresh }`.

The `Ready` state gains no session id (user question 4): the conversation is its own event, which survives the agent's later states and every replay, so `Ready` stays `{ name, version }`.

**ACP v2.** This change calls `session/resume` and `session/load` directly (user decision 8). `agents-acp-v2` reshapes `Start`/`Log` and the `Loading` phase onto v2's single `session/resume { replayFrom }` with its `replay_complete` marker ending the mute, and drops `session/load`; the transcript's `Continuation` then maps from v2's capabilities, so no client-visible form changes.

### 5. The ended session in the core (`process R2`, `R3`)

`Life` gains `AgentEnded { code: Option<i32> }`: no pid, never `Starting`, listed with the agent's `error` activity and 0×0 as any agent session. `policy::end_fate(ownership, life, continuable) -> EndFate::{Keep, KeepForOwner, Report}` replaces `exit_fate` for agent sessions (terminal sessions keep `exit_fate`). `AGENT_RESTART` (`client_agent_restart`):
1. `policy::agent_request_refusal(protocol, AgentRequest::Restart)` (below 4), then `attached_session`, then `policy::restart_refusal(life, continuable, &command)`: "the agent hasn't ended" (any other `Life`), "the agent can't continue this conversation" (not continuable, unreachable while `end_fate` drops those), "AGENT_RESTART needs a command".
2. `reduce::step(Input::Restart)` under `guarded_step` (`process R3`); its events go to the client.
3. `Life::Starting { spawn: SpawnRequest::Restart { restart, cwd } }` with the same id, key and meta (its `command` updated for `list`, `SET_META` once placed); `placement` picks any holder of the core's build at H1 3, as for a new agent, starting one if needed.
4. `SPAWNED` steps `Input::Spawned` as today; `SPAWN_FAILED` steps `Input::Exited { code: None, stderr: message }` back to `AgentEnded`, so the user can fix the profile and try again.

### 6. What a core restart, a Brindle crash and an upgrade do (`process R2`, `R12`)

| Event | A running agent session | An ended one (`AgentEnded`) | A continue in progress |
| --- | --- | --- | --- |
| core crash or `SIGKILL` | adopted as today; the conversation is in the checkpoint, so a later exit can be continued | lost: it lived only in the core. The attached client reconnects, isn't given it back, and shows the zone lost; the zone offers to continue in a new session (decision 8), which is the "server lost the log" path | before `SPAWNED`: lost as an ended one. After it: adopted from the `BeforeRequest` checkpoint taken before `initialize`, `start` included, so the resume or load goes out from the new core |
| upgrade (`STEP_DOWN`) | as a core crash, untruncated | lost, as for a crash (the new core adopts only what holders hold) | as a crash |
| Brindle crash | its policy applies as today | kept while the owner may return (`Lingering` within its linger, `AwaitingOwner` within the grace), then ended | the agent starts; the returning owner gets the replay |
| holder crash | continuable: `AgentEnded` with `Lost`; else lost as today | unaffected (it has no holder) | before `SPAWNED`: re-placed as any `Starting` spawn; after: `Lost`, `AgentEnded` again |

Keeping ended sessions across core restarts would need the holder to keep a session after its program ended, or a file: both rejected in decision 3. The cost is that a core restart turns an exited zone's continue into a load into a fresh session, which every captured agent supports.

### 7. Expected failures and where they show (`railway R1`, `R6`)

| Failure | Value | Shown on |
| --- | --- | --- |
| The new agent offers neither load nor resume | `Failed { "the agent can't continue conversations: it offers neither session/load nor session/resume" }`, hang-up, session ends | the zone, which keeps its copy of the conversation |
| `session/resume` or `session/load` errors | `Failed { "the agent couldn't continue the conversation: MSG" }`, same | the zone |
| … with `-32000` | `LoginRequired { message }`, session kept, agent running | the zone (`agents-login` later turns this into a login) |
| The command can't be started | `SPAWN_FAILED` → `Exited { None, message }`, back to `AgentEnded` | the zone shows the cause as the standard error; Continue stays offered |
| A restart refused | `ERROR` naming the session | the zone's request reply, as other refused requests |
| The server no longer has the session | `AgentPhase::Lost` (client) | the zone, offering a continue in a new session |

No continue falls back to `session/new`: that would silently start a conversation that knows nothing of the log shown above it. A `Failed` continue ends the session as a version mismatch does, since the agent can do nothing; the zone keeps its copy, and the user can close it or open the profile again.

### 8. The zone (`fcis R1`, `types R1`)

`continue_offer(phase, life, conversation, profile) -> Offer` in `src/agent/model.rs`, pure, with

```rust
pub enum Offer {
    Restart,                    // AGENT_RESTART on this session: Live(Exited | Lost), SessionLife::Open, a continuable conversation, the profile found
    Respawn { conversation },   // SPAWN_AGENT continuing it: AgentPhase::Lost, a conversation with load or resume, the profile found
    None(NoOffer),
}
pub enum NoOffer { NotEnded, NoConversation, NotOffered, ProfileGone { name: String } }
```

The profile is found by the name the session was started with (the zone's profile, recorded in the session's `meta.profile`), and must still be an agent profile; a zone attached at launch under a substitute profile (`session-restore`'s "Agent profile gone") gets `ProfileGone`, so a continue never runs another profile's agent on a conversation. `DisabledWhy` gains the reasons' words ("the agent can't continue conversations", "the profile NAME is gone", "the agent never started a conversation"). After a `Restart` the phase goes back through `Live(Starting)` to `Live(Ready)` on the same `SessionLife::Open` session, so `resend_of` enables a held prompt's send-again with no new code (user decision 5). After a `Respawn` the zone starts a new link and resets its model to the new session's fold, as an attach does; `Requests` gains `restart(command, args, env)`.

- **Pending permissions at the exit** (user decision 6): unchanged; the exit records `Cancelled`, the rows show it, and a continued agent's new requests are new entries.
- **Dump:** the `--- agent` line gains `continue=restart|respawn|none:REASON` and the state word `holder-lost` for `AgentState::Lost`; the button is focus stop `agent stop=continue`. `--wait-for` matches these as other agent lines.
- **Close:** an ended zone closes without asking (its activity is `error`), and its `CLOSE` ends the session.

### 9. Protocol versions: client protocol 4 and H1 3 amended in place (user decision, cross-cutting; `process R4`–`R6`)

Every `agents-*` change ships in one release, so client protocol 4 and holder protocol H1 3, introduced by the unreleased `agents-sessions` and `agents-profiles`, are not frozen yet (`process R5` freezes a format once it has shipped). This change **amends protocol 4 in place** instead of adding protocol 5: `AGENT_RESTART` (21), the optional `SpawnAgent.conversation`, `Event::Conversation`, `AgentState::Lost` and the end-of-exit behaviour are protocol 4's from the first release that has protocol 4. There are no fallbacks for protocol-4 peers without them, because none will ever ship. H1 3 needs no amendment (decision 5's restart is a plain `SPAWN_PIPES`).

Pinned by: `remote-protocol/tests/transcripts.rs`'s `client-agent-session` fixture regenerated with an `AGENT_RESTART` and a continuing `SPAWN_AGENT` (the fixture is edited, which is allowed only because protocol 4 hasn't shipped; the design says so here so review doesn't read it as a breach of `process R5`), `every_message_variant_and_form_appears_in_a_transcript` with the new variant, `brindle-transcript`'s `v4/` fixtures (`conversation-*.json` for each `Continuation`, `agent-lost.json`), the fold's property test over the new event, and the fuzzers of both decoders. Once released, protocol 4 is frozen with these in it.

**For `agents-login`:** its post-login reconnect reuses `AGENT_RESTART` (user answer to login Q3; login lands after this change). That change widens `restart_refusal` to a `LoginRequired` session (which still runs: the restart hangs it up first) and its start to `Start::New` when no conversation exists yet; the message's form doesn't change. Message numbers: this change takes 21; `agents-login`'s `AGENT_LOGIN` and `agents-log-paging`'s `AGENT_FETCH` take the next free ones when they land.

### 10. Session restore (user decision 4)

Exited zones are not recorded, as today; the state file gains no field. Since an ended session now stays open on the server, the recording rule moves from `SessionLife::Ended` to the agent's phase: a zone whose agent is `Exited` or `Lost` is left out of the tab's record and its sibling takes its place, as for a zone with no running session. Quit detaches it as every session, and the core ends an ended session on `DETACH` (decision 3), so nothing is left for the next launch to reattach.

### 11. Tests and the fake agent (`testing`)

- **Unit, in the reducer:** every row of decision 2's table, the mute's count and notice, ids continuing, `Input::Restart` refused outside `Exited`/`Lost`, `Input::Lost`, a turn in flight failed and never re-sent, a held prompt resent after `Ready`, a permission cancelled at the exit staying cancelled, `-32000` and other errors to `session/resume`/`session/load`, and the captured `initialize` replies' `Continuation`s.
- **Unit, elsewhere:** `policy::end_fate` and `restart_refusal` over every `Ownership`/`Life` pair, `continue_offer` over every phase, life and profile case, the checkpoint round trip with each new phase.
- **Real-role tests** (`remote-agent/tests/agents.rs`, with `brindle-fake-acp-agent`): the existing steps suffice. `AGENT_RESTART` carries `args`, so a test restarts with a second script; `expect session/resume`, `record` and `respond` check what the new agent was sent; `send` plays a load's replay; `exit` and killing the holder end the first agent. *No new script step* is needed; if the e2e cases find rewriting a script between runs awkward, a `once <path>` step can be added then.
- **e2e** (`tests/e2e/cases/agent.rs`): the zone's profile runs a script the case rewrites before pressing continue (gated by `wait-file`, never a sleep). Cases in tasks.md.
- **Manual:** pointer presses on the button, and real agents (tasks.md).

### Rules relied on

`fcis R1`–`R3` (choices in the reducer, `policy` and the model), `types R1`–`R3` (closed `Continuation`, `Offer`, `Start`; the conversation's id kept as a string the agent gave; session identity id + key across a restart), `railway R1`, `R6` (failures as agent states shown in the zone), `process R2`, `R3`, `R4`–`R6`, `R12`, `R19` (the core is still the only ACP client), `ports R1`, `R3`. No exception is needed.

## Risks / Trade-offs

- [A core restart or upgrade loses ended sessions] → the zone offers a continue into a new session by `session/load`; the conversation is the agent's, so nothing is lost but the server's log, which the replay rebuilds.
- [A Copilot continue shows the conversation as it was, but not changes the agent made to its own history] → accepted: muting is what keeps the log free of duplicates, and the notice says the replay wasn't shown.
- [An agent claims `resume` but forgot the conversation] → its error becomes `Failed` with its message; the user can close the zone and start again.
- [A profile edited to another agent continues a conversation it can't know] → the new agent's `session/resume` or `session/load` fails, which is shown; Brindle can't tell agents apart beyond the profile name.
- [A continue right after a crash loop] → nothing restarts on its own; each continue is a click.
- [Message number collisions with the parallel `agents-login` and `agents-log-paging` designs] → this change takes 21; the others renumber when they land (decision 9).
- [Ended sessions pile up in the core] → each is bounded by the log's 16 MiB and lives only while its client has it or its owner may return.

## Migration Plan

Nothing to migrate: client protocol 4 and H1 3 have not shipped (decision 9), and no config or state file changes. A Brindle and a helper of this release meet only each other on protocol 4. Rollback is reverting the change before release; after release, protocol 4 is frozen with these messages in it.

## As built

- **2026-11-25, tasks 1.1–1.2: `AgentState::Lost { message }`, not `{ reason }`.** The transcript's fixture test (`an_unknown_kind_or_value_is_a_decode_error`) treats every string field named `reason` as a closed enum's tag (`StopReason`'s), so a free-text `reason` would read as an enum value. `message` also matches `Failed` and `LoginRequired`. The encoding is `{"agent":"lost","message":"its holder ended"}` (`v4/agent-lost.json`). Read decisions 3, 5, 7 and 8's `Lost { reason }` as `Lost { message }`.
- **2026-11-25, task 1.1: the app compiles against the new variants with stopgap arms** in `src/agent/model.rs` (a lost agent's prompt reason reuses `DisabledWhy::Exited`; its alert says "NAME was lost: MESSAGE"; the dump word is already `holder-lost`) and `src/agent/shared.rs` (`Event::Conversation` marks the turn-or-agent change). Task 4.1 replaces the stopgap reason with its own.
- **2026-11-25, task 1.3: the refusal below protocol 4 is the core's, not the codec's.** `client.rs` knows no connection's protocol, so it decodes `AGENT_RESTART` on any connection, as it does the other agent messages; the refusal below 4 is `policy::agent_request_refusal`'s (task 3.1's `AgentRequest::Restart`). Until task 3.3 the core answers every `AGENT_RESTART` with an `ERROR` naming its request and session (a stopgap arm in `core/clients.rs`), and ignores `SPAWN_AGENT`'s `conversation`. The payload's struct is `client::AgentRestart`, and the conversation's `client::AgentConversation { id, cwd }`.

## Open Questions

None. The proposal's eight questions are answered by the user and recorded in proposal.md; the choices this design adds beyond them are decisions 3 (`Event::Conversation` instead of a field on `Ready`), 3 and 5 (`AgentState::Lost`, `Life::AgentEnded`, the end of an ended session on `DETACH`), 5 (`SPAWN_FAILED` returning to `AgentEnded`), 6 (ended sessions lost on a core restart), 7 (no fallback to `session/new`; a failed continue ends the session), 8 (the offer by the session's profile name) and 11 (no new fake-agent step).
