# Design

## Context

See proposal.md for why. The design was written against `main` at 124912e and refreshed on 2026-11-26 against 08c2a69 (`zones-mixed-tabs` implemented and archived, PR #149, on top of `agents-tool-content`, #146). Facts this design rests on, from that code, the pinned schema crate and the captured agent traffic:

- **The ACP seam** (`agents-tool-content`, `ports R14`). `remote-agent/src/acp/` is a version-free reducer (`reduce.rs`) over Brindle's own closed forms (`forms.rs`: `Initialized`, `NewSession`, `SessionUpdate`, `Request<'_>`, `Refusal`, …), with one module per ACP version (`v1/{decode,encode}.rs`) behind the closed `Dialect` enum (`dialect.rs`), whose every method is one exhaustive `match self`. `rpc.rs` keeps the outgoing requests by id as `Purpose::{Initialize, NewSession, Prompt, Resume, Load, Authenticate}` and writes each through the session's dialect; `envelope.rs` reads the JSON-RPC envelope. No ACP method or field name appears outside `v1/`.
- **Continuing exists** (`agents-session-recovery`). `SPAWN_AGENT` takes an optional `conversation` (`client::AgentConversation { id, cwd, … }`); the reducer's `Phase::Spawning { start }` and `Phase::Initializing { start }` carry `Start::{New, Continue { conversation, log: Log::{Kept, Fresh} }}`, and `Step::start` sends, after `initialize`, `session/new` (`Phase::Creating`), or `session/resume` (`Phase::Resuming`) or `session/load` (`Phase::Loading`) by `choose(log, offered) -> Choice`; `Step::continued` reads their answer, `Step::ready` emits `Event::Conversation` and `AgentState::Ready`. `AgentSession::new` and `AgentSession::continuing` build a session; `Greeting { name, version, continuation }` is what `initialize` said. On the client, `src/agent/model.rs`' `continue_offer(phase, life, conversation, profile) -> Offer::{Restart, Respawn { conversation }, None(NoOffer)}` gives what Continue conversation does, the dump's `continue=` word.
- **A stopped agent's session is kept** while its zone is open (`Life::AgentEnded` in the core), with a Continue conversation button on the prompt input's reason line (`agent stop=continue`), and isn't recorded for restore.
- **A login** (`agents-login`) can be needed by any start request (`-32000`, `forms::AUTH_REQUIRED`): `Phase::LoginRequired(Login { greeting, message, then: AfterLogin::{Start(Start), Ready { session }} })`, then `Phase::LoggingIn`; `Step::authenticated` returns to what it interrupted.
- **The core** (`remote-agent/src/server/core/agents.rs`): `client_spawn_agent` (refused by `policy::agent_request_refusal(protocol, AgentRequest::Spawn)` below protocol 4), `client_agent_restart`, `client_agent_command` (every other agent command: `reduce::Command::{Prompt, Unqueue, Resend, Cancel, Answer, Login}`, stepped by `agent_step`, answered `OK` or `ERROR` from `Output::reply: Option<Result<(), Refused>>`) and `client_agent_fetch` (`AGENT_PAGE` frames, answered without a step). `agent_apply` sends a step's events, checkpoints, writes stdin and hangs up, in that order.
- **The checkpoint** (`acp/checkpoint.rs`, format 2 with a base and segments, `agents-log-paging`): the JSON state's `phase` (`phase_value`, `with_start`, `greeting_value`), the `rpc` (`next_id`, the outgoing purposes, the permissions), the methods, the open outputs. Format 2 is unreleased and amended in place.
- **The transcript** (`brindle-transcript`): `AgentState::{Starting, Ready { name?, version? }, LoginRequired { message, methods }, LoggingIn { method, name }, Failed, Exited, Lost}`, tagged `agent` (`{"event":"agent","state":{"agent":"starting"}}`); the fixtures are `transcript/tests/fixtures/v4/*.json`, read by `tests/events.rs`' `fixtures()`, and `src/fuzz.rs` draws every state.
- **ACP v1** (schema crate 1.22.0): `session/list { cwd?, cursor? }` answers `{ sessions: [{ sessionId, cwd, title?, updatedAt? }], nextCursor? }`, offered through `agentCapabilities.sessionCapabilities.list`. v2 keeps the method. Codex's captured `initialize` (`remote-agent/tests/fixtures/acp/codex-acp-2.12.0-*.log`) offers it; no capture holds the exchange itself, so task 1.1 captures it.
- **`zones-mixed-tabs`** gives every zone its own profile and host (`ZoneSpec`), the split actions for an agent zone beside any pane (`zone_view.rs`' `split_choice(want, zone, config) -> SplitChoice::{Run, Ask, Refused}`, `SplitWith::{Kind, Profile}`, `split_cwd(split_dir, same_host)`), and the palette that asks which profile (`SplitAsk`, `Workspace::open_split_palette`, `PaletteKind::SplitProfile`). The dump's zone lines name each zone's `kind=`, `profile=` and `host=`.
- **The client's agent zone** (`src/agent/`): `Agent` (`mod.rs`) holds the model and its session through `Requests` (`prompt`, `resend`, `unqueue`, `answer`, `cancel`, `close`, `restart`, `login`, `fetch`, …), opened by `Connect` from `remote::AgentStart::{Spawn, Attach}`; `spawn.rs`' `AgentSpawnRequest { command, args, cwd, env, profile, conversation }` and `directory(profile, host, asked, opened, home)`; `model.rs` holds `AgentPhase`, `ChatModel`, the reason line and `describe` (the dump); `view.rs` draws the zone.
- **`brindle agent [PROFILE]`** (`src/main.rs`' `AgentLaunch { profile, cwd, prompt }`) takes `-d` and `-p`; a launch is handed to a running Brindle as `instance::Request { kind: Some("agent"), agent: Some(AgentRequest { profile, cwd, prompt }) }`. The request is never versioned: unknown fields are ignored, and `Request::asked` refuses a `kind` it doesn't know by name ("this Brindle (V) can't open KIND tabs; …").
- **Actions** (`src/actions.rs`' `ACTIONS`): `agent_continue` is listed as "Continue Agent Conversation".
- **Message numbers** (`remote-protocol/src/client.rs`): one number space for both directions, clients' messages below 32 and the core's from 32 (`WELCOME`). Clients' run through `AGENT_FETCH` 23, with 24 to 30 planned for `agents-commands` (24), `agents-prompt-context` (25, 26), `agents-prompt-images` (27 to 29) and `agents-plans` (30); the core's run through `AGENT_PAGE` 43, with 44 planned for `agents-prompt-context`. So 31 is the last client number below the core's block (decision 9).

The user's decisions (2026-11-26) are in "The user's decisions" below.

## Goals / Non-Goals

**Goals:**
- Any conversation an agent kept for a directory can be taken up again, from a pane, from a stopped zone and from the command line.
- Listing never costs an agent start that the continue then pays again.
- A choice never replaces a conversation the user can still see without their asking for that pane.
- One screen for "this pane has no conversation yet": after `-r`, after `-c` found nothing, and in a zone whose agent stopped.
- Nothing is kept on the server about the list: it is asked for when shown.

**Non-Goals:** conversations of other directories; deleting, archiving, renaming or forking; search beyond the palette's filter; a conversation id on the command line; previews; the list in the sessions palette; keeping the list between uses; listing from an agent that doesn't offer `session/list`.

## Layer map

| Part | Kind | Crate and module |
| --- | --- | --- |
| `AgentState::Choosing` | pure, shared contract | `brindle-transcript` |
| `AGENT_LIST`, `AGENT_CHOOSE`, `AGENT_CONVERSATIONS`; `SPAWN_AGENT`'s `start` | wire | `brindle-remote-protocol`, `remote-protocol/src/client.rs` |
| `Listed` (one conversation), `Listing` (a page), `Request::List`, the list capability on `Initialized` | pure, plain data | `brindle-remote-agent`, `acp/forms.rs` |
| ACP v1: `session/list` and its answer, `sessionCapabilities.list` | pure | `acp/v1/{decode, encode}.rs`, through `Dialect::list_result` |
| `Phase::Choosing`, `Start::Choose`, `Command::{List, Choose}`, `Purpose::List`, the list and choose steps and their refusals, `Output::listed` | pure | `acp/reduce.rs`, `acp/rpc.rs` |
| The phase and the list capability in the checkpoint | pure | `acp/checkpoint.rs` |
| The handlers, `AgentRequest::{List, Choose}` | shell (loop handler), pure policy | `remote-agent/src/server/core/{agents,clients,policy}.rs` |
| `lister_plan`, `list_dir`, `open_plan`, `latest_plan`, `ago`, the palette's rows, the choosing screen's parts, dump lines | GPUI-free model | `brindle`, `src/agent/model.rs`, `src/workspace/` |
| The palette, the question, the buttons, the two actions, `-c` and `-r` | shell (entities, launch) | `src/workspace.rs`, `src/agent/view.rs`, `src/actions.rs`, `src/main.rs`, `src/instance.rs` |

No crate reference and no closed dependency list changes (`ports R1`, `R3`).

## Decisions

### 1. An agent session can start by choosing (`types R1`)

`SPAWN_AGENT` gains an optional `start: "choose"`. Such a session is spawned and initialized as any, and then sends no `session/new`: it waits in `Phase::Choosing { greeting }`, its state `AgentState::Choosing`, until a client chooses.

```rust
pub enum Start { New, Continue { conversation, log }, Choose }
Phase::Choosing { greeting }          // initialized; no ACP session yet
AgentState::Choosing { message? }     // JSON {"agent":"choosing"}; a refused choice's message (flag 5)
```

- **`AGENT_CHOOSE { req, session, conversation? }`** ends the wait: with an id, the reducer continues that conversation in the session's directory as `Start::Continue { log: Fresh }` does (`session/load` shown, or `session/resume`, by `choose`); with none, it sends `session/new`. Answered `OK` once the request is written, or `ERROR` (`NotChoosing`, `NotAnAgentSession`). The outcome arrives as transcript events, as any start's does.
- **Why a phase and not a second kind of session.** The agent that lists is then the agent that continues, in the same process: one start. A listing-only session would be started, hung up, and its work paid again by the continue.
- A choosing session is otherwise an ordinary agent session: owned and attached by its client, placed on a holder, checkpointed (the phase is state), lost with its holder as any. It has no conversation, so nothing can be continued if it ends, and its zone, when it has one, shows the choosing screen (decision 6).
- A login needed by `session/list` or by the chosen start is `LoginRequired` as for any start, and returns to `Choosing` or to the start it interrupted (`AfterLogin`).
- `AGENT_PROMPT` in `Choosing` is refused `NotReady`. A launch's first prompt (`-p`) waits read-only for the first `Ready`, as it already does through a login.

### 2. Listing (`acp/forms.rs`, each version's module; `railway R5`)

- **`AGENT_LIST { req, session, cursor? }`** asks the session's agent for a page of its conversations in the session's directory. The reducer accepts it in `Ready` and in `Choosing`, and writes `Request::List { cwd, cursor }`, which the v1 module encodes as `session/list`.
- **`AGENT_CONVERSATIONS { req, session, conversations: [{ id, title?, updated? }], next? }`** is the reply, one frame. `ERROR` otherwise: `NotOffered` ("the agent can't list conversations"), `NotReady`, `Busy` (a list is already being answered), the agent's own error in words, or that a login is needed.
- **Only this directory.** The request carries the session's `cwd` as ACP's filter, and the reducer drops any answer entry whose `cwd` differs, so an agent that ignores the filter still lists one directory.
- **Bounds:** at most 256 conversations a reply (later ones dropped, with `next` kept so the client can ask on); an id bounded as tool call ids are; a title cleaned to one line and cut at 256 characters, as `agents-session-title` cleans one; `updated` kept as text of at most 64 characters when it is one, else left out. Entries are decoded one by one from the raw JSON, so an unreadable one loses only itself. `cursor` and `next` are opaque strings of at most 1 KiB.
- **Nothing is kept.** The server holds no list: the reply is built from the agent's answer and forgotten. The pending request is `Purpose::List { req }` in `Rpc`, holding the client's request id so the answer reaches it as `Output::listed`, which the core sends as `AGENT_CONVERSATIONS` or `ERROR` to the attached client. The checkpoint leaves a pending list out (flag 4): a core restart in the middle loses the reply, the client's request fails with the connection's error, and the agent's late answer is a response to no request, which the reducer ignores.
- **Whether the agent lists** is read from `initialize` (`Initialized::lists`, from `sessionCapabilities.list`) and kept on the session beside its login methods, and in its checkpoint (a checkpoint without it reads as not offered). It isn't in the transcript: a client learns it from `AGENT_LIST`'s refusal (flag 3).
- **Version seam** (`agents-tool-content`): `session/list` and its field names are named only in `acp/v1/` (`encode::encode` of `Request::List`, `decode::list_result`), behind `Dialect::list_result`; the reducer sees `forms::Listing`.

### 3. Who lists (`lister_plan`, pure)

```rust
pub enum Lister { Zone(ZoneId), Chooser }
pub fn lister_plan(profile, host, zones: &[ZoneFacts]) -> Lister
```

- **A zone of that profile on that host whose agent is `Ready`** lists through its own session: the zone the palette was opened from when it qualifies, else the most recently focused one. Its conversation's directory must be the directory wanted (decision 4); otherwise it can't answer for it.
- **Otherwise a chooser**: a `start: "choose"` session of the profile, started in the directory wanted, shown by no zone yet. The palette shows "starting AGENT…" until it is `Choosing`, then lists through it.
- **When the palette closes without a choice, the chooser is closed** (`CLOSE`), ending its agent. When a choice is made, the chooser becomes the conversation's session (decision 5).
- A chooser that fails to start, or needs a login, shows that in the palette's place: the failure's text, or "AGENT needs a login: open the profile to log in". Nothing is listed.

### 4. Which directory (`list_dir`, pure)

The directory listed is the one a new agent zone of that profile would get from the pane the palette was opened from, by `zones-mixed-tabs`' split rule (`split_cwd` over the pane's `zone_working_directory`, then `spawn::directory`): an agent zone's own directory; a terminal's working directory when it runs on the profile's host; otherwise the profile's directory, else the home directory there. `list_dir` is that rule as one pure function, which the split then also uses, so the two can't drift. The palette's title names it ("Claude conversations in ~/src/brindle"), so what is listed is never a surprise.

### 5. Where a choice opens (`open_plan`, pure; the user's rule)

```rust
pub enum Open { Show(ZoneId), Here(ZoneId), Ask }
pub fn open_plan(chosen: &Listed, from: PaneFacts, open_zones: &[ZoneFacts]) -> Open
```

1. **Already open:** a zone of the profile on that host shows that conversation (by its id) → `Show`: its tab becomes active and it takes focus. The row is marked "open". Nothing is started.
2. **The pane has no agent running** (an agent zone that is stopped, or still choosing) → `Here`: that zone takes the conversation. A stopped zone's old session is closed and the zone resets onto the new one, as a `Respawn` does; a choosing zone just chooses.
3. **Any other pane** (a running agent, a terminal) → `Ask`: the palette turns into three rows, Split Right, Split Down and New Tab, the first highlighted. Escape goes back to the list.

How the conversation's session comes to be:
- listed through a **chooser**: the new or reused zone attaches the chooser and sends `AGENT_CHOOSE` with the id. One agent start in all.
- listed through a **running zone's agent**: the new zone is started with `SPAWN_AGENT` and the conversation (`agents-session-recovery`'s path), since the lister is busy being that zone's agent.

A split uses `zones-mixed-tabs`' split with the profile given; a zone too small to split leaves Split Right and Split Down disabled.

### 6. The choosing screen and a stopped zone (`src/agent/model.rs`, `view.rs`)

One screen serves every pane that has no conversation running, drawn where the prompt input's reason line and its buttons are today:

| The zone | Reason shown | Buttons |
| --- | --- | --- |
| Agent exited or lost, can continue | as today | Continue conversation, Conversations…, New conversation |
| Agent exited or lost, can't continue | as today | Conversations…, New conversation |
| `Choosing` after `-r`, or after the list was dismissed | "choose a conversation, or start a new one" | Conversations…, New conversation |
| `Choosing` after `-c` found none | "no earlier conversation in DIR" | Conversations…, New conversation |
| `Choosing`, the agent can't list | "AGENT can't list conversations" | New conversation |

- **Conversations…** opens the palette for the zone's profile and directory. **New conversation** starts one in this zone: `AGENT_CHOOSE` with no id in a choosing zone; in a stopped zone, a new session the zone resets onto, the old one closed.
- The buttons are keyboard stops after the existing one: `agent stop=continue`, `stop=conversations`, `stop=new`. Their requests go off the UI thread as the zone's other requests do (`process R13`).
- A stopped zone whose profile is gone offers none of them, as it offers no continue today.
- A zone in `Choosing` shows no working row and raises no attention: nothing is running on the user's behalf.

### 7. The palette (`src/workspace.rs`, `src/actions.rs`)

- **`agent_conversations`** ("Agent Conversations…", no default key): handled by an agent zone, for its profile; listed only in an agent zone.
- **`agent_continue_conversation`** ("Continue Agent Conversation…", no default key): handled by the workspace, listed while the config has an agent profile. With one agent profile it goes straight to the list; with several it first asks which, in the profile palette `zones-mixed-tabs` adds, the active zone's profile first when that is an agent's.
- **Rows:** a conversation's title (else "Untitled" and the start of its id), then when it was last active (`ago(now, updated)`: "5 minutes ago", "yesterday", a date; nothing when the agent gave none or it doesn't parse), and "open" when a zone shows it. The agent's order is kept, which is newest first for every captured agent. The palette's filter narrows by title.
- **More:** when the reply has `next`, a last row "More…" asks for the next page and appends it. At most 1,024 conversations are held by the palette; past that the row says so.
- **States:** "starting AGENT…", "listing…", the rows, "no earlier conversation in DIR", or a failure's text. A chooser's start and the list request run off the UI thread; the palette stays responsive and escape cancels.
- The existing `agent_continue` ("Continue Agent Conversation", `agents-session-recovery`) is renamed in the palette to "Continue This Conversation", so the two can't be mistaken; its action name is unchanged.

### 8. The command line (`src/main.rs`, `src/instance.rs`)

- **`brindle agent [PROFILE] -c`** (`--continue`): the tab's zone starts a chooser in the launch's directory; when it is `Choosing`, the model asks for the first page and `latest_plan(conversations) -> Latest::{Continue(id), None}` takes the entry with the newest `updated`, else the first listed (Antigravity doesn't list newest first; decision 15). `Continue` chooses it; `None` leaves the zone on the choosing screen with "no earlier conversation in DIR" (the user's decision). An agent that can't list leaves it there with that reason.
- **`brindle agent [PROFILE] -r`** (`--resume`): the same start, and the palette opens on the zone once it is `Choosing`. A choice is taken `Here`; escape leaves the choosing screen.
- `-c` and `-r` together are a usage error (status 2). Either may come with `-d`, and with `-p`, whose prompt is sent on the first `Ready`, after the choice.
- **The hand-off** to a running Brindle carries the choice in the agent fields (`AgentRequest::choose: Some(Last | List)`) under a kind of its own, `agent-choose`, not `agent` (flag 2). The request is never versioned and an older Brindle ignores a field it doesn't know, so under `agent` it would open a new conversation the user didn't ask for; a kind it doesn't know it refuses by name (`Request::asked`), opening nothing. The launch then prints that the running Brindle is older and can't continue a conversation, with that answer, and exits with status 1 (`command-line`'s "The hand-off across versions"). This version reads `agent-choose` as `agent` with the choice.

### 9. Protocol versions (`process R4`–`R6`)

Client protocol 4 is unreleased and amended in place, as for the rest of the series. The numbers were settled with the design's author on 2026-11-26, with 24 to 30 and the core's 44 kept for `agents-commands`, `agents-prompt-context`, `agents-prompt-images` and `agents-plans`, which land later (protocol 4 being unreleased, the gap is harmless):

| Message | Number | Direction |
| --- | ---: | --- |
| `AGENT_LIST { req, session, cursor? }` | 31 | client → core |
| `AGENT_CHOOSE { req, session, conversation? }` | 64 | client → core |
| `AGENT_CONVERSATIONS { req, session, conversations, next? }` | 45 | core → client |

`AGENT_CHOOSE` was to be 32, but the two directions share one number space and 32 is `WELCOME`: 31 is the last client number below the core's block. It opens a second client block at 64, leaving the core 32 to 63 (flag 1). Both client messages join `policy::AgentRequest` (`List`, `Choose`) for the refusal by name below protocol 4. `SPAWN_AGENT` gains `start`; `AgentState` gains `Choosing`. Transcript fixtures pin each message, both forms of `AGENT_CHOOSE` and each refusal; a new `v4/` fixture pins the state; all join the fuzzing. The checkpoint (format 2) gains the phase. Holder protocol H1 3 is untouched.

### 10. The dump (`testing R2`)

```
--- agent zone=1 state=choosing turn=idle … continue=none:no-conversation
--- prompt status=disabled reason="no earlier conversation in ~/src/brindle"
--- palette conversations profile="Claude" dir="~/src/brindle" state=listed more=no
Fix the flaky upload test  2 hours ago
Untitled 7f3a9c  yesterday  open
--- palette open-in highlighted=0
```

The state word `choosing` joins the agent line's states. The conversations palette prints its profile, directory and state (`starting`, `listing`, `listed`, `empty`, `failed`), then its rows as palette rows print; the question prints as a palette list of its three rows. Both are driven with the list actions and `picker_confirm`. The buttons are the focus stops of decision 6. A row's time is as shown and so depends on when the dump is taken: the e2e cases give their conversations no `updatedAt`, or match a row by its title alone, and `ago` is unit-tested with `now` as an argument (`fcis R2`).

### 11. Failures (`railway R6`)

- The agent can't list: the palette says so; a choosing zone offers New conversation.
- The chooser can't start, or needs a login: the palette shows the cause and lists nothing.
- The agent answers `session/list` with an error: the palette shows its message; "More…" failing keeps the rows already shown.
- The chosen conversation can't be continued (the agent refuses, or offers neither load nor resume): the zone shows what `agents-session-recovery` shows for a refused continue; a choosing zone goes back to the choosing screen with the agent's message.
- A choice while the lister's session is gone: the palette says the list is stale and asks again.
- `-c` with no conversation: the choosing screen with the reason.

### 12. Restarts, crashes and session restore (`process R1`–`R3`, `R12`)

- A choosing session's phase is in its checkpoint, so a core restart or upgrade keeps it choosing, and its zone still shows the screen.
- A zone that is still choosing isn't recorded for session restore, as a stopped agent's isn't: there is no conversation to bring back. A quit ends its session.
- A chooser with no zone belongs to the palette: closing the palette, closing the window and Brindle quitting each close it. A Brindle crash leaves it to the owner-gone rules of any session, which end it.
- A list in flight at a core restart is lost; the palette shows the connection's error and can ask again.

### 13. With the changes around this one

- **`agents-commands`:** a choosing agent has no session, so no commands or options are listed until a conversation starts; `agents-commands`' "every start answers with options" covers the chosen start.
- **`agents-plans`:** a conversation continued here replays its plans as any continue does.
- **`agents-prompt-context`:** a kept attachment's file outlives its session by 14 days, so a conversation taken up from this list within that time still finds the files its links name.
- **`agents-session-info`** (proposal-only) keeps the title in the terminal server's session list; this list is the agent's own.
- **`agents-acp-v2`:** `session/list` is unchanged in v2, so its module writes the same request; the continue is that change's `session/resume { replayFrom }`.

### 14. Tests

- **Unit:** the reducer's `Choosing` phase (a choose of an id by each `Continuation`, a choose of none, a prompt refused, a login during a list returning to `Choosing`), the list step's refusals and bounds, an entry of another directory dropped; `v1` encode and decode over the captures of task 1.1; `lister_plan`, `list_dir`, `open_plan`, `latest_plan` and `ago`; the palette's rows and states; the choosing screen's table; the launch options and the hand-off's field; the dump lines.
- **Protocol:** fixtures and fuzzing for the three messages and `SPAWN_AGENT`'s `start`.
- **Real roles** (the fake ACP agent, answering `session/list` from its script): a chooser lists and then continues in one process; a list through a ready agent; a choose of none; a chooser closed without a choice leaves no session; paging; a core restart while choosing.
- **E2E:** `agent-conversations-list`, `agent-conversations-open` (here, split, new tab, already open), `agent-conversations-stopped`, `agent-conversations-cli` (`-c`, `-c` with none, `-r`), `agent-conversations-failures`.
- **Manual:** a real agent's list, `-c` and `-r` with each agent we can run; pointer presses on the choosing screen's buttons.

### 15. As built

Filled in as the tasks land.

- **The captures (task 1.1)**, `remote-agent/tests/fixtures/acp/*-list.log`, 2026-11-26. Each made two conversations in a new directory, then listed with that `cwd`, with none, and with an empty directory's. A list answer in the fixtures keeps only the conversations made in capture directories; the user's others are dropped (Claude's answer without `cwd` held 188).

  | Agent | Honours `cwd` | Order | `updatedAt` | Paging | Login to list |
  | --- | --- | --- | --- | --- | --- |
  | Codex (`codex-acp` 2.12.0) | yes | newest first | `2026-11-26T20:35:14.000Z` | none seen (5 in all) | not asked |
  | Antigravity (`agy` 1.5.16) | **no**: the same 8 for every `cwd` and for none | **not** newest first | `2026-11-26T20:35:37Z` | none seen | not asked |
  | Claude (`claude-agent-acp` 0.112.0) | yes | newest first | `2026-11-26T20:35:49.824Z` | **none**: 188 in one answer | not asked |
  | Copilot (`copilot` 1.0.116) | yes | newest first | `2026-11-25T02:42:30.315Z` | none seen | **no**: it listed while `session/new` failed on its GitHub credential |

  Every answer's entries have exactly `sessionId`, `cwd`, `title` and `updatedAt`; an empty directory is `sessions: []` (Antigravity's is everything, filtered to nothing by decision 2). So the reducer's directory filter is needed, the 256 bound can be reached, and `latest_plan` takes the entry with the newest `updated` (the first listed when none parses), not simply the first (decision 8). Copilot's capture is a list only: its `session/new` failed, as in `agents-tool-content`.

### 16. Flags

The user asked for open questions to be decided and flagged. Each is decided as recommended; the choices in "Choices made here, for review" below stand as written.

1. **`AGENT_CHOOSE` is 64, not 32** (decision 9). 32 is `WELCOME`, in the one number space both directions share, and 24 to 31 are planned or taken. Alternatives: renumber the planned messages (four other designs name theirs), or make choosing a form of another message (it is its own request). Recommend 64, opening a second client block and leaving the core 32 to 63.
2. **A continuing launch is handed off under its own kind, `agent-choose`** (decision 8). The hand-off is never versioned and an older Brindle ignores unknown fields, so a field alone would make it open a new conversation. Recommend the kind, which an older Brindle refuses by name.
3. **Whether an agent lists isn't in the transcript** (decision 2): the server keeps it from `initialize`, and a client learns it from `AGENT_LIST`'s `NotOffered` refusal, which the choosing screen and the palette then show. Alternative: a field on `AgentState::Choosing` or the conversation, a wire change for one bit the first list answers anyway.
4. **A list in flight isn't checkpointed** (decisions 2, 12): its `Purpose::List` holds a client's request id, which means nothing to the clients of a restarted core. The checkpoint leaves it out, and the agent's late answer is a response to no request, ignored.
5. **A refused choice's message rides on the state**: `AgentState::Choosing { message? }` (JSON `{"agent":"choosing"}` without one), so a zone that goes back to choosing (decision 11) shows the agent's message as its reason, attached late or not. A notice in the log would scroll away, and a choosing zone has no conversation to show it in.
6. **`updated` is the agent's text**, at most 64 bytes; `ago` reads RFC 3339 (with or without fractional seconds, `Z` or an offset), and shows nothing for anything else.
7. **`AGENT_CHOOSE` takes any bounded id**: the server keeps no list, so it can't check that the id was listed; an id the agent doesn't know is the agent's refusal, as for any continue.
8. **A list through a ready agent may run during its turn**: ACP allows concurrent requests, and `Busy` refuses only a second list.
9. **The reducer drops entries of other directories** (decision 2, `reduce::page`): Antigravity ignores `cwd` (decision 15). Directories compare without a trailing separator; nothing else is normalized (no symlinks resolved, no case folded), since the server can't know what the agent meant. An entry with an empty, control-bearing or over-1 KiB id is left out and counted in the core's log, as are the decoder's unreadable entries.
10. **A login a list needs changes the phase only while choosing** (decisions 1, 11): a choosing agent waits for the login and then chooses again (`AfterLogin::Start(Start::Choose)`); a ready agent keeps its conversation and the list is refused "the agent needs a login", since its next prompt finds the login needed as any prompt would. The alternative, a ready agent going to `LoginRequired` on a list's error, would stop a conversation for a side request.
11. **A list out when the agent stops being read is answered** with why (`the agent has exited`, `the agent was lost with its holder`, the start's failure, `the terminal server's ACP handling failed`), so a palette waiting on it never hangs.
12. **A `SPAWN_AGENT` naming both a `conversation` and `start: "choose"` is refused** ("SPAWN_AGENT can't both continue a conversation and choose one").
13. **A chosen conversation continues as a fresh log's continue does**, so an agent offering both loads it with its replay shown (decision 2 of `agents-session-recovery`), and a resume-only agent resumes it with the notice that its earlier turns aren't shown.
14. **A list's id is checkpointed, the list isn't** (flag 4, found by task 3.2's restart case): a list marks `BeforeRequest`, so the checkpoint the holder has before the agent's `session/list` already counts the id. Without it a restored session would send its next request under the same id, and the agent's late answer to the old list would answer it.

Flags from the client's build (sections 4 to 6), for the design's author:

15. **The client tells `NotOffered` by its text.** `brindle-remote-protocol` gains `client::LIST_NOT_OFFERED` ("the agent can't list conversations"), which the server's refusal writes and the client compares; no wire change (flag 3 stands). Alternative: an error code on `ERROR`, which nothing else has.
16. **`lister_plan` takes the pane itself when it is choosing** in the directory wanted, not only when it is ready: its agent is that profile's, waiting there, so `-r` and a choosing zone's Conversations… list through their own session and start nothing.
17. **The buttons follow the offer alone** (`ChatModel::zone_choices`): a stopped zone that never started a conversation still offers Conversations… and New conversation; a never-started agent (`NotStarted`) offers none and keeps its own reason; a gone profile offers none. The "open" mark and `Show` count only zones whose agent may run (ready or busy); a stopped zone showing the conversation is `Here` instead, which continues it in that zone.
18. **"Most recently focused" is a recency the workspace computes**: the zone's focus serial in its tab, the active tab's raised above the others'. Zones of two background tabs compare by their own serials, which aren't one clock.
19. **`ago` uses UTC calendar days** for "yesterday", and dates show as `YYYY-MM-DD`: there is no local time zone in the pure code (`fcis R2`), and the e2e cases give no `updatedAt`.
20. **`brindle-ui`'s `PromptInput` takes several keyed actions** (`set_actions`, `PromptAction { key, label }`, `PromptInputEvent::Action(key)`, `PromptFocus::Action(key)`), the key being the stop's name (`continue`, `conversations`, `new`); `set_action` keeps the one-button form. The dump's prompt line gains ` reason="…"` while choosing and ` actions=continue,conversations,new` while buttons show.
21. **A choosing zone's session is closed on quit** (`CLOSE`, not `DETACH`), as is one started to choose that hasn't chosen yet (still connecting or starting): neither is recorded, so a detached one would only wait for the owner-gone rules.
22. **The question has one `splits` bit**: Split Right and Split Down are both enabled when the pane fits either way, and a split that then doesn't fit says so on the config line ("the zone can't be split now; the conversation wasn't opened"), closing the chooser. Alternative: a bit per axis in `Stage::Ask`.
23. **A conversation chosen through a running zone's agent starts with `SPAWN_AGENT` and the conversation** in the listed directory, or, when this machine can't say it (another host's home), in the spawn's own directory.
24. **The profile question's title is "Continue a conversation of…"**, and with no agent profile `agent_continue_conversation` does nothing (it isn't listed then).

## The user's decisions

Settled on 2026-11-26, one at a time:
- Listing uses a running zone's agent when there is one, and otherwise an agent the server starts for it (decisions 1, 3).
- One palette, from two actions (decision 7).
- A choice opens in the pane itself when the pane has no agent running; from a pane with a running agent or a terminal, the palette asks whether to split or open a new tab. This is the user's own rule, replacing the recommended "in place if unused" (decision 5).
- Only this directory's conversations, chosen over a row for all directories (decisions 2, 4).
- No deleting.
- `-c`/`--continue` for the last conversation and `-r`/`--resume` to start from the list, the user's own choice of the agents' usual flags (decision 8).
- `-c` with no earlier conversation opens the choosing screen with the reason, "a variant of the screen that allows you to pick a session when an agent exits" (decisions 6, 8).
- Mixed tabs are their own change, defined first (`zones-mixed-tabs`).

Choices made here, for review:
- The agent that lists is the agent that continues (a choosing phase), so a list costs no extra start (decision 1).
- The question offers Split Right, Split Down and New Tab (decision 5).
- A conversation already open in a zone is shown, not opened twice (decision 5).
- A stopped zone also offers New conversation, and Continue Agent Conversation is renamed Continue This Conversation in the palette (decisions 6, 7).
- A running Brindle from before this change refuses a `-c` or `-r` hand-off (decision 8).

## Risks / Trade-offs

- No capture shows a real `session/list` answer → task 1.1 captures each agent first; the mapping follows ACP's small, fixed types.
- An agent may ignore the directory filter → the reducer filters the answer too.
- A chooser is a whole agent process waiting on a palette → it is closed when the palette is, and it is the process the conversation then uses.
- A list through a running agent, then a continue in a new zone, starts a second agent → that zone needs its own agent in any case.
- Two actions with similar names → the older one is renamed for this conversation, the new one says it lists.
- The newest conversation isn't always "the first the agent lists" → Antigravity lists another order (decision 15), so `latest_plan` compares `updated`, keeping the agent's first when no time parses. The palette keeps the agent's order, as decision 7 says.
- A conversation continued while another tool has it open → that is the agent's to handle, as for any continue.

## Migration Plan

Nothing to migrate.
