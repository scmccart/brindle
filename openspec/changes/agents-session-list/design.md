# Design

## Context

See proposal.md for why. Facts this design rests on, from the code on `main` at 19368a9 (2026-11-26), the pinned schema crate and the captured agent traffic:

- **Continuing exists** (`agents-session-recovery`). `SPAWN_AGENT` takes an optional `conversation` (`Conversation { id, cwd, continuation, title }`); the reducer's `Phase::Spawning { start: Start::{New, Continue { conversation, log: Kept | Fresh }} }` sends, after `initialize`, `session/new`, or `session/resume` or `session/load` by `choose(log, offered)`; a zone's `continue_offer` gives `Restart` (the same session) or `Respawn` (a new session the zone resets onto).
- **A stopped agent's session is kept** while its zone is open (`Life::AgentEnded`), with a Continue conversation button on the prompt input's reason line (`agent stop=continue`), and isn't recorded for restore.
- **A login** (`agents-login`) can be needed by any start request (`-32000`), and returns to what it interrupted (`AfterLogin`).
- **ACP v1** (schema crate 1.22.0): `session/list { cwd?, cursor? }` answers `{ sessions: [{ sessionId, cwd, title?, updatedAt? }], nextCursor? }`, offered through `sessionCapabilities.list`. v2 keeps the method. Every captured agent's `initialize` offers it; no capture holds the exchange itself, so task 1.1 captures it.
- **`zones-mixed-tabs`** gives every zone its own profile and host, the split actions for an agent zone beside any pane, and the palette that asks which profile.
- **`brindle agent [PROFILE]`** takes `-d` and `-p`; a launch is handed to a running Brindle with its kind, profile, directory and prompt.
- **Message numbers** at this point: clients' through `AGENT_FETCH` 23, with 24 to 30 planned for `agents-commands`, `agents-prompt-context`, `agents-prompt-images` and `agents-plans`; the core's through `AGENT_PAGE` 43, with 44 planned.

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
| `Listed` (one conversation), `Request::List`, the list capability | pure, plain data | `brindle-remote-agent`, `acp/forms.rs` |
| ACP v1: `session/list` and its answer | pure | `acp/v1/{decode, encode}.rs` |
| `Phase::Choosing`, `Start::Choose`, the list and choose steps and their refusals | pure | `acp/reduce.rs` |
| The handlers | shell (loop handler) | `remote-agent/src/server/core/` |
| `lister_plan`, `list_dir`, `open_plan`, `latest_plan`, `ago`, the palette's rows, the choosing screen's parts, dump lines | GPUI-free model | `brindle`, `src/agent/model.rs`, `src/workspace/` |
| The palette, the question, the buttons, the two actions, `-c` and `-r` | shell (entities, launch) | `src/workspace.rs`, `src/agent/view.rs`, `src/actions.rs`, `src/main.rs`, `src/instance.rs` |

No crate reference and no closed dependency list changes (`ports R1`, `R3`).

## Decisions

### 1. An agent session can start by choosing (`types R1`)

`SPAWN_AGENT` gains an optional `start: "choose"`. Such a session is spawned and initialized as any, and then sends no `session/new`: it waits in `Phase::Choosing { greeting }`, its state `AgentState::Choosing`, until a client chooses.

```rust
pub enum Start { New, Continue { conversation, log }, Choose }
Phase::Choosing { greeting }          // initialized; no ACP session yet
AgentState::Choosing                  // JSON {"state":"choosing"}
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
- **Nothing is kept.** The server holds no list: the reply is built from the agent's answer and forgotten. The pending request is `Rpc` state, checkpointed like any; a core restart in the middle loses the reply, and the client's request fails with the connection's error.
- **Version seam** (`agents-tool-content`): `session/list` and its field names are named only in a version's module. Until that change lands the decoding sits where the other v1 decoding does, and moves with it.

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

The directory listed is the one a new agent zone of that profile would get from the pane the palette was opened from, by `zones-mixed-tabs`' split rule: an agent zone's own directory; a terminal's working directory when it runs on the profile's host; otherwise the profile's directory, else the home directory there. The palette's title names it ("Claude conversations in ~/src/brindle"), so what is listed is never a surprise.

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

- **`brindle agent [PROFILE] -c`** (`--continue`): the tab's zone starts a chooser in the launch's directory; when it is `Choosing`, the model asks for the first page and `latest_plan(conversations) -> Latest::{Continue(id), None}` takes the first entry. `Continue` chooses it; `None` leaves the zone on the choosing screen with "no earlier conversation in DIR" (the user's decision). An agent that can't list leaves it there with that reason.
- **`brindle agent [PROFILE] -r`** (`--resume`): the same start, and the palette opens on the zone once it is `Choosing`. A choice is taken `Here`; escape leaves the choosing screen.
- `-c` and `-r` together are a usage error (status 2). Either may come with `-d`, and with `-p`, whose prompt is sent on the first `Ready`, after the choice.
- **The hand-off** to a running Brindle carries the choice as an optional field. A running Brindle from before this change doesn't know it; the launch then prints that the running Brindle is older and can't continue a conversation, and exits with status 1, rather than opening a new conversation the user didn't ask for (`command-line`'s "The hand-off across versions").

### 9. Protocol versions (`process R4`–`R6`)

Client protocol 4 is unreleased and amended in place, as for the rest of the series: `AGENT_LIST` and `AGENT_CHOOSE` take the next two free client numbers when this lands (31 and 32 after `agents-plans`), `AGENT_CONVERSATIONS` the core's next (45 after `agents-prompt-context`); `SPAWN_AGENT` gains `start`; `AgentState` gains `Choosing`. Transcript fixtures pin each message, both forms of `AGENT_CHOOSE` and each refusal; a new `v4/` fixture pins the state; all join the fuzzing. The checkpoint (format 2) gains the phase. Holder protocol H1 3 is untouched.

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
- The newest conversation is "the first the agent lists" → every captured agent lists newest first; task 1.1 confirms it, and "As built" records any that doesn't, for which `latest_plan` then compares `updated`.
- A conversation continued while another tool has it open → that is the agent's to handle, as for any continue.

## Migration Plan

Nothing to migrate.
