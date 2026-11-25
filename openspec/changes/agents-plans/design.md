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
- A plan from a finished task doesn't linger over the next one.
- No new keyboard stop, and no change to a requirement another stacked change modifies.

**Non-Goals:** editing a plan or steering the agent through it; a plan as a conversation entry, and a history of its states; opening a Markdown plan's file; notifications for steps; plans in the sessions palette or a tab's title; a setting per profile. No client message and no state file.

## Layer map

| Part | Kind | Crate and module |
| --- | --- | --- |
| `Plan`, `PlanBody`, `PlanItem`, `ItemStatus`, `Priority`; `Event::Plan`, `Event::PlanRemoved`; the transcript's state and replay | pure, shared contract | `brindle-transcript`, `transcript/src/lib.rs` |
| The forms: `PlanChange::{ Set(Plan), Removed { id } }` | pure, plain data | `brindle-remote-agent`, `acp/forms.rs` |
| ACP v1: `plan`, `plan_update`, `plan_removed`; the plan capability | pure | `acp/v1/{decode, encode}.rs` |
| Keeping, bounding and clearing plans | pure | `acp/reduce.rs` |
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

### 4. When a plan is cleared (`acp/reduce.rs`)

- **By the agent:** a `Removed`, or v1's `plan` with no entries.
- **By the next turn:** when a prompt is sent to the agent, every plan that is finished is removed first. A checklist is finished when it has items and each is `Completed` or `Cancelled`. A Markdown plan has no statuses and is never finished, so only the agent removes it. A plan with a step still pending or in progress stays: the agent may carry it into the next turn.
- **Not by a turn's end:** a plan stays on view after its task ends, finished or not, so the user can read what was done.
- Each clearing emits `PlanRemoved`.

### 5. The chat model (`src/agent/model.rs`, GPUI-free; `fcis R1`, `R5`)

- **`plan_summary(plan) -> Summary`** (pure): for a checklist, how many items are done (`Completed`), how many there are, and the text of the first `InProgress` item, else of the first `Pending` one, else nothing; for Markdown, its first heading's text, else its first line.
- **Open state.** `plan_open: Shown` from `[agent] plan_details` (closed by default), flipped by hand for the zone as a whole and kept until the zone closes; `set_plan_policy` applies a reload to a zone the user hasn't flipped. Returns what the view must redraw; the plan block isn't part of the scrolled conversation.
- **Parts.** A checklist's items, each with its status and priority and its text rendered by `markdown::render` (under `catch_unwind`, as messages are); a Markdown plan's blocks, with its path as a muted line. The dropped-items count and the cut mark are lines of the block.
- **Several plans** are listed in the order they were set, each with its own summary line; one open state covers them all.

### 6. The zone and `brindle-ui` (`ports R2`)

- **Where.** Between the conversation and the prompt input, above the queue row, shown only while the session has a plan. It doesn't scroll away with the conversation.
- **`Plan`** (new, plain data in): a trigger line with a chevron, a label, a progress count (`3 of 7`) and a current step cut to one line; when open, its items below, each a status mark (an empty circle for pending, a spinner-style mark for in progress, a check for completed, a struck dash for cancelled), the text, and an accent mark before a high-priority item. A completed or cancelled item's text is muted. Status is never colour alone. The open list is at most 12 rem tall and scrolls within itself. A press on the trigger asks the owner to flip it. It takes no focus.
- **`toggle_agent_plan`** ("Toggle Agent Plan", no default key), handled only by `AgentView`, listed only while the session has a plan.
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
- **Real roles** (the fake ACP agent): a plan reaches an attached client and each update replaces it; a restart keeps it; a new prompt clears a finished one.
- **E2E:** `agent-plan-progress`, `agent-plan-markdown`, `agent-plan-cleared`, `agent-plan-restore`, `agent-plan-settings`.
- **Manual:** a pointer press on the trigger; scrolling a long open plan; a real agent's plan on a task of several steps.

## Open decisions

Each has a recommendation, which the text above follows. They are the user's to settle before implementation.

1. **Where the plan shows:** above the prompt input, staying in view (recommended), or as an entry in the conversation where it first appeared, updated in place.
2. **How it starts:** closed, as one line with the count and the current step, with `[agent] plan_details` to open it by default (recommended), or always open.
3. **A finished plan is cleared when the next turn starts** (recommended), or stays until the agent replaces or removes it.
4. **Priority:** only high-priority items are marked (recommended), or every item shows its priority.
5. **Brindle tells v1 agents it takes the newer form**, so they may send named and Markdown plans (recommended), or only v1's one checklist is read until `agents-acp-v2`.
6. **The bounds:** 8 plans, 256 items a plan, 64 KiB of Markdown (recommended).

## Risks / Trade-offs

- No capture shows what agents really send → the first task captures it, and the mapping follows ACP's types, which are small and fixed.
- A plan above the prompt input takes room from the conversation → one line when closed, at most 12 rem when open.
- Clearing a finished plan at the next prompt removes something the user may still want to read → it stays through the whole pause between turns, and the tool rows record what was done.
- An agent that never marks its last step done leaves its plan on view → it is closed by default, and the agent's next plan replaces it.
- One open state for all of a session's plans → several plans at once are rare, and one action stays enough.

## Migration Plan

Nothing to migrate. A session checkpointed before the change has no plan until its agent next sends one.
