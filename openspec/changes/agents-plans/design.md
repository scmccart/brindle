# Design

## Context

See proposal.md for why. Facts this design rests on, from the code on `feat/agents-profiles` (2026-11-25), the stacked designs, the pinned schema crate and the captured agent traffic:

- **The reducer drops plans today.** `wire.rs` maps `plan` (and every kind it doesn't name) to `Update::Ignored`; `Entry` has no plan kind; `initialize` advertises no plan capability.
- **No capture holds a plan.** `remote-agent/tests/fixtures/acp/` are short tasks of one edit or command, and none contains `plan`, `plan_update` or `plan_removed`. Task 1.1 captures a task of several steps from each agent we can run, before the mapping is written.
- **ACP v1** (schema crate 1.22.0, `v1/plan.rs`): `plan { entries }`, where "the Agent MUST send all entries with their current status" and "the Client MUST replace the entire checklist with each update"; an entry is `{ content, priority: high | medium | low, status: pending | in_progress | completed }`. Also `plan_update { plan }`, where the plan is `{ type: "items", planId, entries }` or `{ type: "markdown", planId, content, path? }` and replaces the whole plan with that id "even if its type changes", and `plan_removed { planId }`, which a client "MUST ignore" for an unknown id. An agent "MUST NOT send" the last two unless the client advertised `clientCapabilities.plan`.
- **ACP v2** (Preview): only `plan_update` and `plan_removed`; statuses add `cancelled`; priorities and statuses accept unknown values; display strings such as an entry's content may be Markdown.
- **The stacked changes:** `agents-tool-content` gives the `Dialect` seam, `acp/forms.rs`, and the `[agent]` config table with `tool_details`; `agents-commands` keeps commands and options as session state with whole-list events, which this change follows; `agents-markdown` gives `src/agent/markdown.rs::render` and `brindle-ui`'s `Response`, and says plans would need a new event kind.
- **From the stack's session (2026-11-25):** `agents-session-recovery` folds an agent's replayed updates into the log again, so kept plans must not double; `agents-log-paging` pages whole entries, so state outside entries must be sent on every attach.

## Goals / Non-Goals

**Goals:**
- A user sees, at a glance and without scrolling, how far a task of several steps has got.
- One form of Brindle's own for both ACP forms of a plan, filled inside each version's module.
- The server keeps plans, so a restored or reattached zone has them.
- A plan the agent has stopped keeping up can be put away by the user, and stays away.
- No new keyboard stop, and no change to a requirement another stacked change modifies.

**Non-Goals:** editing a plan or steering the agent through it; a plan as a conversation entry, and a history of its states; opening a Markdown plan's file; notifications for steps; plans in the sessions palette or a tab's title; a setting per profile; bringing a dismissed plan back by hand; clearing a plan automatically. No state file.

## Layer map

| Part | Kind | Crate and module |
| --- | --- | --- |
| `Plan`, `PlanBody`, `PlanItem`, `ItemStatus`, `Priority`; `Event::Plan`, `Event::PlanRemoved`; the transcript's state and replay | pure, shared contract | `brindle-transcript`, `transcript/src/lib.rs` |
| `AGENT_DISMISS_PLAN` | wire | `brindle-remote-protocol`, `remote-protocol/src/client.rs` |
| The forms: `PlanChange::{ Set(Plan), Removed { id } }` | pure, plain data | `brindle-remote-agent`, `acp/forms.rs` |
| ACP v1: `plan`, `plan_update`, `plan_removed`; the plan capability | pure | `acp/v1/{decode, encode}.rs` |
| Keeping, bounding and dismissing plans | pure | `acp/reduce.rs` |
| `plan_summary`, the block's parts, open state, dump lines | GPUI-free model | `brindle`, `src/agent/model.rs` |
| `[agent] plan_details` | pure (boundary parse) | `brindle`, `src/config.rs` |
| The block above the prompt input, the action | shell (entity) | `src/agent/view.rs`, `src/agent/mod.rs`, `src/actions.rs` |
| `Plan` | component | `brindle-ui`, `ui/src/components/plan.rs` (new) |

No crate reference and no closed dependency list changes (`ports R1`, `R3`).

## Decisions

### 1. Brindle's own form (`types R1`, `R2`, `R7`)

```rust
pub struct Plan { pub id: String, pub body: PlanBody }
pub enum PlanBody {
    Items { items: Vec<PlanItem> },
    Markdown { text: String, path: Option<String> },
}
pub struct PlanItem { pub text: String, pub status: ItemStatus, pub priority: Priority }
pub enum ItemStatus { Pending, InProgress, Completed, Cancelled, Other { name: String } }
pub enum Priority { High, Medium, Low, Other { name: String } }
```

- **A plan is a checklist or a text, never both**, as ACP's two types are. A plan with no path has `None`.
- **`Cancelled` and the two `Other`s** are values v1 never sends. They are in the form now because protocol 4 can still be amended (decision 3) and v2 will need them.
- **The unnamed plan's id is the empty string.** v1's `plan` has no id, and an id an agent gives is never empty (an empty `planId` is left out as unreadable), so the two can't collide.
- **Plans are session state, not log entries**, like the queue slot, commands and options: the transcript holds the current plans, each whole. A plan's text isn't history; what the agent did is in the tool rows.

### 2. The two ACP forms (`acp/forms.rs`, `acp/v1/`)

| ACP v1 | Form |
| --- | --- |
| `plan { entries }` | `PlanChange::Set` of the plan with id `""`, as `Items`; with no entries, `Removed { id: "" }` |
| `plan_update` of `type: "items"` | `Set` of that id, as `Items` |
| `plan_update` of `type: "markdown"` | `Set` of that id, as `Markdown` with its `path` |
| `plan_update` of another `type` | nothing, counted in one notice |
| `plan_removed { planId }` | `Removed { id }` |

- Entries are decoded one by one from the raw JSON, as tool content is, so an unreadable entry loses only itself.
- **The capability.** The v1 module's `initialize` advertises `clientCapabilities.plan: {}`, so a v1 agent may send the newer form. An agent that sends both forms simply has two plans, the unnamed one and its named ones; task 1.1 says whether any agent does.
- **ACP v2** (for `agents-acp-v2`): `plan_update` and `plan_removed` map as above; `cancelled` and unknown statuses and priorities fill the values decision 1 already has; there is no unnamed plan.

### 3. Events, the fold and replay (`brindle-transcript`; `process R4`–`R6`)

```rust
Event::Plan { plan: Plan }           // the plan with this id, whole: added, or replacing the one held
Event::PlanRemoved { id: String }    // nothing when no plan has the id
```

- Both are upserts or removals by id, so applying one twice gives the same state. That is what makes `agents-session-recovery`'s muted replay harmless: an agent replaying its conversation resends plans, which replace themselves.
- `Transcript` gains `plans`, in the order they were first set; `replay()` sends one `Plan` each after the agent's state, on every attach, whatever page of entries `agents-log-paging` sends with it; `fold(replay(t)) == t` gains them. No fold error is possible beyond decoding.
- The core sends an event only when a plan differs from the one it holds (agents resend them).
- **Bounds** (`railway R5`), applied by the reducer before the event: at most `MAX_PLANS` = 8 plans (a ninth id is refused with a notice); `MAX_PLAN_ITEMS` = 256 items a plan, later ones dropped and counted; an item's text bounded at 1 KiB; `PLAN_TEXT` = 64 KiB of Markdown, its end dropped with a mark; a path bounded as titles are; ids bounded as tool call ids are; every text cleaned. One plan therefore fits one event under `TEXT_CHUNK`'s frame.
- **Versions.** Client protocol 4 is unreleased and amended, as `agents-tool-content`'s decision 8 says: two event kinds, new `tests/fixtures/v4/` files and none edited. The checkpoint keeps format 1 with the plans in its state; one from before this change decodes with none.

### 4. When a plan goes (`acp/reduce.rs`; the user's decision, 2026-11-25)

Brindle never clears a plan on its own: not when a turn ends, not when every item is done, not when the next prompt is sent. A plan goes in two ways.

- **The agent removes it:** a `Removed`, or v1's `plan` with no entries.
- **The user dismisses it.** Agents often stop updating a plan, leaving steps unchecked that were done, and v1's unnamed plan can't be withdrawn at all, so the user needs a way to put one away.
  - **`AGENT_DISMISS_PLAN { req, session, plan }`**, a new client message of protocol 4 (amended; the next free number when the stack is assembled), `plan` being an id, or absent for every plan the session holds. Answered `OK`, or `ERROR` (`NoSuchPlan`, `NotAnAgentSession`). Fixtures pin it and it joins the protocol's fuzzing.
  - **The server removes the plan** from the session's state and tells attached clients (`PlanRemoved`), so it is gone for every client and after a restart. The agent isn't told: ACP has no message for it.
  - **Dismissed until the agent changes it.** The server remembers each dismissed plan beside the session's state (`dismissed: id → Plan`, at most `MAX_PLANS`, in the checkpoint). A plan the agent sends that equals the one dismissed under its id is ignored; one that differs is kept and shown again, and the record is dropped. So a stale plan an agent keeps resending stays away, and real progress brings it back. A removal by the agent drops the record too.
- This is also what keeps `agents-session-recovery`'s replay quiet: an agent replaying its conversation resends the plans it had, and one the user dismissed is equal to its record.

### 5. The chat model (`src/agent/model.rs`, GPUI-free; `fcis R1`, `R5`)

- **`plan_summary(plan) -> Summary`** (pure): for a checklist, how many items are done (`Completed`), how many there are, and the text of the first `InProgress` item, else of the first `Pending` one, else nothing; for Markdown, its first heading's text, else its first line.
- **Open state.** `plan_open: Shown` from `[agent] plan_details` (closed by default), flipped by hand for the zone as a whole and kept until the zone closes; `set_plan_policy` applies a reload to a zone the user hasn't flipped. Returns what the view must redraw; the plan block isn't part of the scrolled conversation.
- **Parts.** A checklist's items, each with its status and priority and its text rendered by `markdown::render` (under `catch_unwind`, as messages are); a Markdown plan's blocks, with its path as a muted line. The dropped-items count and the cut mark are lines of the block.
- **Several plans** are listed in the order they were set, each with its own summary line; one open state covers them all.

### 6. The zone and `brindle-ui` (`ports R2`)

- **Where.** Between the conversation and the prompt input, above the queue row, shown only while the session has a plan. It doesn't scroll away with the conversation.
- **`Plan`** (new, plain data in): a trigger line with a chevron, a label, a progress count (`3 of 7`) and a current step cut to one line; when open, its items below, each a status mark (an empty circle for pending, a spinner-style mark for in progress, a check for completed, a struck dash for cancelled), the text, and an accent mark before a high-priority item. A completed or cancelled item's text is muted. Status is never colour alone. The open list is at most 12 rem tall and scrolls within itself. A press on the trigger asks the owner to flip it. It takes no focus.
- **`toggle_agent_plan`** ("Toggle Agent Plan", no default key), handled only by `AgentView`, listed only while the session has a plan.
- **Dismissing.** Each plan's trigger line has a small dismiss mark at its end, pressed with the pointer, which asks the server to dismiss that plan. `agent_dismiss_plan` ("Dismiss Agent Plan", no default key) dismisses every plan the zone shows, and is listed only while the session has one. The plan goes from the zone when the server says it is removed, not before; a refusal shows on the window's notice line.
- **`[agent] plan_details`**: `"closed"` (the default) or `"open"`, decoded into `Shown` beside `tool_details`; another value is an invalid config.
- No keyboard stop is added: "Keyboard path through an agent zone" is unchanged.

### 7. The dump (`testing R2`)

```
--- plan id="" kind=items done=3 total=7 open=no current="Run the tests"
--- plan-item plan="" index=0 status=completed priority=medium "Read the failing test"
--- plan id="design" kind=markdown open=no path="/p/PLAN.md" "# Migration\n…"
```

`--- plan` lines, each followed by its `--- plan-item` lines whether open or closed, come after the `--- agent` line (and after `agents-commands`' option lines when stacked). Status is `pending`, `in-progress`, `completed`, `cancelled` or `other:NAME`; priority likewise. `wait_text` holds them, so `--wait-for 'done=7 total=7'` works.

### 8. Failures (`railway R6`, `R8`)

- An entry or a plan that can't be read, an unknown plan type, a ninth plan, items past the bound: left out, one notice in the conversation saying what and how many.
- A Markdown render panic: the plan's text as plain text, as for messages.
- A `PlanRemoved` for an id not held: nothing, as ACP requires.

### 9. Restarts, crashes and upgrades (`process R1`–`R3`, `R12`)

Plans are in the session's checkpoint, so a core restart and an upgrade keep them, and a restored zone shows them from the replay. A plan update lost in a restart shows the last plan checkpointed until the agent sends the next. A Brindle crash loses only the zone's open state.

### 10. Tests

- **Unit:** the fold and replay over both events, twice-applied events, and `fold(replay(t)) == t`; `v1::decode` over each form, an unreadable entry, an empty `planId`, an unknown type, and the captures of task 1.1; `v1::encode`'s capability; the reducer's bounds, equal plans emitting nothing, clearing finished plans when a prompt is sent and keeping unfinished and Markdown ones; `plan_summary` for each case; the dump lines; the config key.
- **Captures:** task 1.1, and the replay test asserting each agent's plans.
- **Real roles** (the fake ACP agent): a plan reaches an attached client and each update replaces it; a restart keeps it; a finished plan survives a new prompt; a dismissal reaches every client, stays after an unchanged resend and a restart, and ends when the agent changes the plan.
- **E2E:** `agent-plan-progress`, `agent-plan-markdown`, `agent-plan-dismiss`, `agent-plan-restore`, `agent-plan-settings`.
- **Manual:** a pointer press on the trigger; scrolling a long open plan; a real agent's plan on a task of several steps.

## Open decisions

Each has a recommendation, which the text above follows. They are the user's to settle before implementation.

Decided on 2026-11-25: the plan shows above the prompt input, staying in view (decision 6). It starts closed, as one line with the count and the current step, and `[agent] plan_details` opens it by default (decisions 5 and 6). A plan stays until the agent replaces or removes it, which the user chose over clearing a finished one at the next turn, and the user can dismiss one, since agents often forget to check items off (decision 4).

4. **Priority:** only high-priority items are marked (recommended), or every item shows its priority.
5. **Brindle tells v1 agents it takes the newer form**, so they may send named and Markdown plans (recommended), or only v1's one checklist is read until `agents-acp-v2`.
6. **The bounds:** 8 plans, 256 items a plan, 64 KiB of Markdown (recommended).

## Risks / Trade-offs

- No capture shows what agents really send → the first task captures it, and the mapping follows ACP's types, which are small and fixed.
- A plan above the prompt input takes room from the conversation → one line when closed, at most 12 rem when open.
- A plan stays until someone removes it, so a stale one sits above the prompt → it is one line when closed, and Dismiss Agent Plan puts it away for good unless the agent changes it.
- A dismissed plan can't be brought back by hand → the agent's next change to it brings it back, and the tool rows record what was done.
- The agent isn't told of a dismissal → it is only about what Brindle shows; the agent's own view of its plan is unchanged.
- An agent that never marks its last step done leaves its plan on view → it is closed by default, and the agent's next plan replaces it.
- One open state for all of a session's plans → several plans at once are rare, and one action stays enough.

## Migration Plan

Nothing to migrate. A session checkpointed before the change has no plan until its agent next sends one.
