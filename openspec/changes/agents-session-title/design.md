# Design

## Context

See proposal.md for why. The code this design rests on, at `main` 725ed28 (manual-findings investigation, question 1):

- **The title today.** `ZoneContent::title` for an agent zone is `ChatModel::title()` (`zone_view.rs`), which is `agent_name.unwrap_or(profile)`. `agent_name` is set from every `AgentState::Ready { name: Some(..) }` and kept after an exit. A tab's `manual_title` wins over any zone's automatic title (`workspace.rs`, `refresh_displayed_title`). `title()` also fills `failure_text`'s `{agent}` and the close confirmation's busy lines; the dump prints `Ready`'s name as `name=`.
- **The name.** `reduce.rs`'s `initialized` builds `Greeting { name: bound(clean(agent.name)), version, continuation }`; `wire.rs` decodes `agentInfo` into `AgentInfo { name, title, version }` and nothing reads `title`. The captures: Claude `@agentclientprotocol/claude-agent-acp` / `Claude Agent`, Codex `@agentclientprotocol/codex-acp` / `Codex`, the older Claude adapter `@zed-industries/claude-code-acp` / `Claude Code`, Copilot `Copilot` / `Copilot`, Antigravity `Antigravity ACP Server` with no title. The session list's agent name is `Ready`'s.
- **`session_info_update`.** `wire::session_update` folds it into `Update::Ignored`, and the reducer drops it. In the captures: Claude sends `title` with `updatedAt` once per turn, Antigravity `title` alone, Copilot the whole first prompt as its title, Codex only `_meta` (`threadStatus`). ACP v1's `title` is "undefined, null or a string" (`MaybeUndefined<String>`): absent changes nothing, `null` clears.
- **The conversation.** `brindle_transcript::Conversation { id, cwd, continuation }` travels as `Event::Conversation`, sent by `reduce.rs`'s `ready()` before `Ready` on every start, resume, load and login; "a later one replaces it"; `Transcript::conversation` holds the last, and `replay()` and the checkpoint (the log as replay events) carry it. `ChatModel` keeps the last conversation it saw, through a respawn until the new session gives one. `SPAWN_AGENT`'s `conversation` is `brindle-remote-protocol`'s own `AgentConversation { id, cwd }`, a separate type.
- **Notifications.** `policy::layout(source, title, body, tab_title)` makes `<title> — <tab title>` for every source; for an agent tab both halves are `title()`.
- **Protocol 4** is merged and unreleased; the agent series amends it in place (decided 2026-11-25), and within protocol 4 an event may gain optional fields at any time.

## Goals / Non-Goals

**Goals:**
- A tab reads as the user's profile until the agent names the conversation, and as that name afterwards, unless the user renamed it.
- The title is the session's state on the server, so every client, restart and upgrade sees the same one.
- No new event kind and no new message; every existing fixture passes unchanged.

**Non-Goals:** the title in the session list or the sessions palette; telling the agent about a rename (`session/set_title`); usage and cost; a title for terminal zones; dropping a title that only repeats the first prompt.

## Layer map

| Part | Kind | Crate and module |
| --- | --- | --- |
| `Conversation.title: Option<String>` | pure, shared contract | `brindle-transcript`, `transcript/src/lib.rs` |
| `Update::SessionInfo { title: Patch<String> }`; `agentInfo.title` read into the greeting | pure | `brindle-remote-agent`, `acp/wire.rs` (later `acp/v1/decode.rs`, see decision 9) |
| `session_title(raw) -> Option<String>`, `MAX_SESSION_TITLE`; applying an update; carrying the title in `ready` | pure | `acp/reduce.rs` |
| `ChatModel::title()`, `agent_label()` | GPUI-free model | `brindle`, `src/agent/model.rs` |
| `TabTitle::{Automatic, Manual}` and the agent arm of `layout` | pure | `src/notify/policy.rs` |
| Passing the tab's title kind | shell | `src/workspace.rs` `on_attention` |

Crate references and closed dependency lists don't change (`ports R1`, `R3`). ACP is still decoded only in `acp/` (`ports R3`).

## Decisions

### 1. The title is a field of the conversation (`types R1`, `R10`; `process R4`, `R5`)

```rust
pub struct Conversation {
    pub id: String,
    pub cwd: String,
    pub continuation: Continuation,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}
```

- **Why here.** ACP names the conversation, not the agent process, and `Conversation` is already "the agent's conversation", replaced whole by a later one and replayed and checkpointed with the log. So a new ACP session can't inherit the last one's title by accident, and nothing new is needed in the fold, the replay or the checkpoint. "No title" is `None`, never an empty string (`types R10`): the reducer turns a blank title into `None`.
- **The encoding** gains `"title"` inside `conversation`, left out when `None`, so `conversation-*.json` still encode byte for byte; `conversation-titled.json` is added. `Event::from_value` ignores unknown fields, so a protocol 4 client from before this change reads a titled conversation as untitled.
- *Rejected:* a new `Event::SessionTitle { title }` kind with its own fold state: a new kind (`process R5`), a replay position to define, and a rule tying it to the conversation's id anyway. *Flagged*, decision 10.

### 2. Reading `session_info_update` (`acp/wire.rs`; `railway R5`)

- `session_update` decodes `"sessionUpdate": "session_info_update"` by hand into `Update::SessionInfo { title: Patch<String> }`: no `title` key is `Keep`, `null` is `Clear`, a string is `Set`, any other JSON type is `Keep` (logged, not an error: one odd field mustn't fail the update). `updatedAt` and `_meta` aren't read.
- `Patch { Keep, Clear, Set(T) }` is the type `agents-tool-content` plans in `acp/forms.rs`; whichever change lands first defines it, and the other uses it.
- The capture tests now decode Claude's, Antigravity's and Copilot's updates as `Set` and Codex's `_meta`-only ones as `Keep`.

### 3. The reducer (`acp/reduce.rs`; `fcis R1`–`R3`)

- **Clean-up** `session_title(raw: &str) -> Option<String>`: control characters dropped, each run of whitespace (line breaks included) made one space, trimmed; empty is `None`; longer than `MAX_SESSION_TITLE` = 256 characters, cut at a character boundary to 255 and ended with `…`. The bound is on characters, as a tab title is read, well under `TEXT_CHUNK`, so the event stays small however long the agent's title.
- **Applying it.** `Set(raw)` gives `session_title(raw)`, `Clear` gives `None`, `Keep` returns. When the session's transcript has a conversation and the result differs from its title, the reducer emits `Event::Conversation { conversation: Conversation { title, ..current } }`; otherwise nothing. With no conversation yet (an update before `session/new`'s answer, which carries the session id, can't name a session Brindle knows) it does nothing.
- **During a load's replay** (`Phase::Loading`, whose replayed updates are muted and counted), a title update is still applied and isn't counted: it is state, not log, and the agent's latest title is the one to keep.
- **Across restarts of the agent.** `ready(session, greeting, notice)` builds the conversation with `title` taken from the transcript's conversation when its `id` equals `session`, else `None`. So a restart after a login or a continue by resume or load of the same ACP session keeps the title, and a new `session/new` clears it.
- **No new checkpoint moment.** A title applied mid-turn reaches the checkpoint with the next moment. If the core dies first, the holder kept the agent's output past the checkpoint's `covers`, and adoption feeds it again, title update included (`process R2`).

### 4. The agent's readable name (`acp/reduce.rs`)

`initialized` builds the greeting's name from `agentInfo.title` when it is present and not blank after `clean`, else `agentInfo.name`, bounded as now. That is `Ready`'s `name`, so the session list, the dump's `name=` and `agent_label` (decision 5) show `Codex`, `Claude Agent`, `Copilot` and `Antigravity ACP Server`. `AgentState::Ready`'s form is unchanged; only which string fills it. *Flagged*, decision 10.

### 5. The zone (`src/agent/model.rs`; `fcis R1`, `R5`)

- `title()` is the conversation's title (`self.conversation`'s, the one the zone last saw) else the profile's name. `agent_name` no longer reaches it. `absorb` already compares `title()` before and after (`Before.title`) and emits `TitleChanged`, so the tab follows a new title, a cleared one and a profile rename.
- `agent_label()` keeps today's rule, the agent's name from the last `Ready` else the profile's, for `failure_text`'s `{agent}` (`Claude Agent needs you to log in.` reads better than a conversation's title there). The close confirmation's busy lines and the dump's tab line use `title()`, as the tab does. *Flagged*, decision 10.
- During a respawn the zone shows the old conversation's title until the new session's conversation arrives; that one has no title (decision 3), so the zone falls back to the profile's name until the agent names it again. *Flagged*, decision 10.

### 6. Notifications (`src/notify/policy.rs`)

- `layout(source, title, body, tab: TabTitle)` with `TabTitle::{Automatic(&str), Manual(&str)}`, which `on_attention` builds from the tab's `manual_title`. The programs' arms use either as today. The agent arm makes the summary the notice's title (the zone's title, decision 5), then ` — <tab title>` only for a `Manual` title that differs from it. So a default tab reads `Fake`, a named conversation `Hello.txt file creation`, a renamed tab `Fake — Build`.
- `agents-zone-attention` then puts the profile's name, the zone's title and what is asked into the title, on top of this arm.

### 7. Failures (`railway R5`, `R6`)

Nothing new fails visibly. A malformed `session_info_update` changes nothing; a title that cleans to nothing clears; the reducer never panics on it (fuzzed with the other updates). A `FoldError` on a `Conversation` can't arise: `Conversation` replaces and never fails.

### 8. Restarts, crashes and upgrades (`process R1`–`R3`, `R12`)

- **Core restart:** the title is in the checkpointed log (decision 3), or in the agent's output fed again on adoption.
- **Upgrade:** the new core adopts the checkpoint; format 1 amended, the field optional, so an older checkpoint decodes with no title.
- **Brindle crash or quit:** a reattach replays the conversation with its title; nothing is kept in Brindle's state file.
- **Versions that meet:** only unreleased builds carry protocol 4. A core from before this change sends no title and ignores `session_info_update`; a Brindle from before it ignores the field and titles the tab with the agent's name as now. Even after a release the field would be an allowed optional addition within protocol 4 (`process R5`).
- **Tests that pin it** (`process R6`): `transcript/tests/events.rs` with `conversation-titled.json` and the unchanged fixtures, the replay property test with titled conversations, the checkpoint round trip with a title, and a real-role test that kills the core after a title.

### 9. How it meets the other changes

- **`agents-session-info`** (proposal stage) had this title in its scope. It is narrowed (this change edits its proposal) to the title in `SessionInfo` and the sessions palette, `session/set_title` on a rename, and usage and cost; its questions 1, 2 and 7 are answered here.
- **`agents-tool-content`** moves `wire.rs`'s v1 decoding into `acp/v1/decode.rs` behind `Dialect`, in a commit at the stack's tip that moves whatever is there; if it lands first, this change writes the decoding in `acp/v1/decode.rs` and its form in `acp/forms.rs`. `Patch` is shared (decision 2). Both amend protocol 4 with optional fields only.
- **`agents-acp-v2`** maps v2's `session_info_update`, which has the same shape, onto the same form.
- **`agents-zone-attention`** archives after this change (decision 6).

### 10. Choices the user didn't make (flags)

1. The title is an optional field of `Conversation`, not a new event kind (decision 1). **Recommended:** no new kind, and a new session clears it by construction.
2. The agent's name is `agentInfo.title` when given (decision 4), so the session list and failure texts read `Codex`, not the package name. **Recommended.**
3. Failure texts keep the agent's name (`agent_label`), not the conversation's title (decision 5). **Recommended.**
4. Titles are made one line and cut at 256 characters; Copilot's whole-prompt title is kept, cut, with no rule that drops a title equal to the first prompt (decision 3). **Recommended:** such a heuristic would drop real titles.
5. A respawn into a new server session shows the profile's name until the agent names the conversation again (decision 5). **Recommended:** carrying it would need `SPAWN_AGENT` to carry a title, for a rare path.
6. Of `agents-session-info`'s scope this change takes the title in the zone, the tab and notifications; the session list's title, `session/set_title`, usage and cost stay there (decision 9).
7. The notification's summary is the zone's title, with the tab's title only when the user renamed it (decision 6). **Recommended.**

## Risks / Trade-offs

- Codex never sends a title, so its tabs stay the profile's name → that is what the user asked for until an agent names the session.
- An agent that retitles every turn retitles the tab each turn → one `TitleChanged` per change, and equal titles send nothing.
- Copilot's title is the prompt, cut at 256 characters → the tab strip cuts it further; a rename fixes it for good.
- Changing which string `Ready.name` holds changes the dump's `name=` for agents that give a title → only the fake agent's scripts appear in e2e, and they give no title.

## Migration Plan

Nothing to migrate. Sessions started before the change have conversations without titles and pick one up from the agent's next `session_info_update`.
