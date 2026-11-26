# Proposal

**Depends on:** `agents-profiles` (the agent zone and its prompt input, and through it `agents-sessions`' reducer, transcript and client protocol 4), `agents-markdown` (the renderer that draws a plan's text) and `agents-tool-content` (the `Dialect` seam and the `[agent]` config table), implemented and archived.
**Next:** `agents-session-info`, proposal-only.

## Why

For a task of several steps an ACP agent tells its client its plan: a checklist of what it intends to do, each item pending, in progress or done, sent again whole whenever it changes. It is how a user sees, mid-turn, how far a long task has got and what is still to come. In step 1 the terminal server ignores every plan update, so an agent zone shows a stream of tool rows with nothing saying where they are heading.

ACP has two forms. Version 1's `plan` is one unnamed checklist. A newer form, `plan_update` and `plan_removed`, which v1 offers only to a client that says it can take them and which is all v2 has, gives each plan an id, lets an agent keep several, lets a plan be Markdown text in place of a checklist, and lets it be withdrawn. Brindle keeps both ACP versions, so both forms are read into one of Brindle's own.

## What Changes

- **The server keeps the agent's plans** as session state beside the conversation: each plan a checklist of items with a status and a priority, or Markdown text with an optional file path. A plan the agent sends again replaces the one with its id. Plans are in every replay and survive a detach, a quit, a core restart and an upgrade. They belong to the conversation: they stay when the agent stops and when the conversation is continued, and go when the session starts a new one.
- **Both ACP forms fill it.** The v1 module reads the unnamed `plan` as one plan and, since it now tells the agent it can take them, `plan_update` and `plan_removed` too.
- **Brindle never clears a plan on its own, and the user can dismiss one.** A plan stays until the agent replaces or removes it. Since agents often stop keeping a plan up to date, a dismiss mark on the plan and the `agent_dismiss_plan` action put it away, on the server, so it is gone for every client and after a restart. The same plan sent again unchanged doesn't bring it back; a changed one does.
- **The zone shows the plan above the prompt input**, where it stays in view while the conversation scrolls: one line with how many items are done and the step in progress, which opens to the whole checklist. Items' text is rendered as agent messages are.
- **`toggle_agent_plan`** opens and closes it from the keyboard and the palette; `[agent] plan_details` (`"closed"` by default, or `"open"`) sets how it starts.
- **Bounds on what an agent sends:** 256 items a plan and 64 KiB of Markdown. The number of plans isn't limited; a session's plans together may take 4 MiB, so the server's memory stays bounded.
- **Client protocol 4 is amended**, as for the rest of the series: two transcript event kinds and one client message, to dismiss a plan.

Not in this change: editing a plan, or telling the agent to skip or reorder a step; a plan as an entry in the conversation, or a history of its earlier states; opening a Markdown plan's file; notifications when a step finishes; plans in the sessions palette or a tab's title.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `agent-sessions`: new requirements for the agent's plans, their two forms, their bounds, and dismissing one.
- `agent-zones`: new requirements for the plan above the prompt input and its action.
- `ui-components`: a plan block.
- `command-line`: the dump's lines for plans.
- `configuration`: `[agent] plan_details`.

## Impact

- **`brindle-transcript`** (`transcript/`): `Plan`, `PlanBody`, `PlanItem`, `ItemStatus`, `Priority`; `Event::Plan` and `Event::PlanRemoved`; the transcript's state and replay; new `tests/fixtures/v4/` files (none edited).
- **`brindle-remote-agent`** (`remote-agent/src/acp/`): the forms and `acp/v1/decode.rs` for `plan`, `plan_update` and `plan_removed`; the plan capability in `initialize`; the reducer keeping, bounding and dismissing plans; a new client message in `brindle-remote-protocol`. The checkpoint's format 2 is amended (unreleased).
- **`brindle`** (`src/agent/`): the chat model's plans and their summary, the view's block, the dump; an action in `src/actions.rs`; `plan_details` in `src/config.rs`.
- **`brindle-ui`** (`ui/`): a `Plan` component. Its dependency list is unchanged.
- **Tests:** captures of real agents on a task of several steps, which no fixture holds yet; real-role tests through the fake ACP agent; new `agent-plan-*` e2e cases.
- **Docs:** `docs/code-map.md`, `README.md`, `CLAUDE.md`.
