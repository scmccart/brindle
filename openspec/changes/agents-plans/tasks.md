# Tasks

Before starting, check that the changes this one builds on are implemented and archived (the user's decisions are settled; design.md lists them):
- `agents-profiles` (and through it `agents-sessions`): the reducer, `brindle-transcript`, client protocol 4, `src/agent/model.rs`, the dump's agent lines, the fake ACP agent.
- `agents-markdown`: `src/agent/markdown.rs::render` and `brindle-ui`'s `Response`.
- `agents-tool-content`: `acp/forms.rs`, `acp/dialect.rs`, `acp/v1/`, and the `[agent]` config table with `Shown`.
- When stacked on `agents-commands`: plans follow its commands and options in the replay and in the dump.
- Client protocol 4 is still unreleased. If a release has shipped it, stop and raise design decision 3.

## 1. Captures and shared contracts

- [ ] 1.1 Capture each real ACP v1 agent we can run on a task of several steps (for example: read a file, change two functions, run the tests), once with today's `initialize` and once advertising `clientCapabilities.plan: {}`; add the logs to `remote-agent/tests/fixtures/acp/` with identifying values replaced as the existing ones are; write down in design.md's "As built" which agents send `plan`, which send `plan_update` or `plan_removed`, whether any sends both forms, whether an entry's text holds Markdown, and whether a finished plan is ever withdrawn.
- [ ] 1.2 Add to `brindle-transcript`: `Plan`, `PlanBody`, `PlanItem`, `ItemStatus`, `Priority`; `Event::Plan` and `Event::PlanRemoved`; `Transcript`'s `plans` in first-set order with their accessor; `replay` sending one `Plan` each after the agent's state; `PLANS_LIMIT` (4 MiB together, no limit on their number), `MAX_PLAN_ITEMS`, `PLAN_TEXT`; verify with unit tests that each form round-trips, a `Plan` replaces by id and a new id is appended, `PlanRemoved` for an unknown id changes nothing, an event applied twice gives the same state, `fold(replay(t)) == t` with plans, the forms in `fuzz.rs`, and that every existing `tests/fixtures/v4/` file still passes unedited.
- [ ] 1.3 Write the new encodings as new files in `transcript/tests/fixtures/v4/` (a checklist with each status and priority and an `other` of each, a Markdown plan with and without a path, a removal); verify they are written once and read back, and no existing fixture changed.
- [ ] 1.3a Add `AGENT_DISMISS_PLAN` (the next free client message number at the stack's tip) to `brindle-remote-protocol`, its `plan` optional, refused by name below protocol 4 as the other agent messages are; verify with fixtures for one plan and for all, decoding errors, and fuzzing.
- [ ] 1.4 Add `PlanChange` to `acp/forms.rs` and give `Dialect` its decoding; verify `cargo test -p brindle --test architecture` passes.

## 2. Pure code in the server

- [ ] 2.1 Decode in `acp/v1/decode.rs`: `plan` as the plan with the empty id (no entries as its removal), `plan_update` of each type, `plan_removed`, entries one by one from the raw JSON with a count of those unreadable, an empty `planId` and an unknown type left out; verify with unit tests for each and over the captures of 1.1.
- [ ] 2.2 Advertise `clientCapabilities.plan: {}` in `acp/v1/encode.rs`'s `initialize`; verify by pinning the line, and that the existing capture replays still pass.
- [ ] 2.3 Extend the reducer: keep plans by id; apply the bounds with their notices; emit `Plan` only when a plan differs and `PlanRemoved` only when one was held; never remove a plan on its own; the dismissal (`NoSuchPlan`, one plan or all, the `dismissed` record per id, an equal plan from the agent ignored, a differing one kept and the record dropped, a removal by the agent dropping the record); verify with unit tests for each, including a finished plan surviving a turn's end and a new prompt, a dismissed plan resent unchanged staying away, and one resent with a status changed coming back.
- [ ] 2.4 Extend `acp/checkpoint.rs` with the plans, format 1 amended; verify a session with a checklist and a Markdown plan round-trips, and a checkpoint from before this change decodes with none.
- [ ] 2.5 Extend the capture replay test to assert each agent's plans from the captures of 1.1.

## 3. Server roles

- [ ] 3.1 Add real-role tests through the fake ACP agent (its existing script steps send the updates): a plan reaches two attached clients and each update replaces it; a named plan is removed; a detached session replays its plans; a finished plan survives a new prompt; a dismissal reaches both clients and not the agent, stays after the agent resends the plan unchanged, and ends when the agent changes it.
- [ ] 3.2 Add the restart cases: a plan survives a core restart and an upgrade.
- [ ] 3.3 When stacked on `agents-session-recovery` and `agents-log-paging`: check that a muted `session/load` replay leaves one copy of each plan, that a load into a fresh log rebuilds them, and that an attach sends the plans whatever page of entries it sends.

## 4. Brindle's model

- [ ] 4.1 Add `[agent] plan_details` to `src/config.rs` (`"closed"` by default or `"open"`, decoded into `Shown`), documented in `assets/default-config.toml`; verify with unit tests for the default, each value, a bad value's message, and that the shipped default config parses.
- [ ] 4.2 Extend `src/agent/shared.rs` and `transcript.rs` for the two events; verify with unit tests folding each and a replay holding plans.
- [ ] 4.3 Add to `src/agent/model.rs` the pure `plan_summary`, the block's parts (items through `markdown::render` under `catch_unwind`, a Markdown plan's blocks and path, the dropped-items and cut lines), the zone's open state with `toggle_plan` and `set_plan_policy`; verify with unit tests for a checklist with an item in progress, with none in progress, all completed, and empty; a Markdown plan with and without a heading; a by-hand state outliving a plan update and a policy change; and an untouched zone following a policy change.
- [ ] 4.4 Extend the dump (`describe`, `wait_text`) with the `--- plan` and `--- plan-item` lines; verify with unit tests for each form, each status and priority word, and escaping.

## 5. brindle-ui

- [ ] 5.1 Add `Plan` (`ui/src/components/plan.rs`): the trigger with chevron, label, count and current step; items with a status mark of its own shape each, muted text for completed and cancelled, a high-priority mark; content no taller than the height given, scrolling within; no focus; verify with component tests for the press asking the owner, the four marks, the cut step, and the bounded height; add it to the gallery.

## 6. The zone

- [ ] 6.1 Draw the plan block in `src/agent/view.rs` between the conversation and the prompt input, above the queue row, only while the session has a plan; a press on its trigger calls `toggle_plan`. Nothing is computed in `render` beyond reading the model's parts (`process R14`).
- [ ] 6.2 Add `toggle_agent_plan` ("Toggle Agent Plan", no default key) to `ACTIONS`, handled only by `AgentView` and listed only while the session has a plan; verify with a unit test for the palette's listing with and without a plan and in a terminal zone.
- [ ] 6.2a Add `agent_dismiss_plan` ("Dismiss Agent Plan", no default key) to `ACTIONS`, handled only by `AgentView` and listed only while the session has a plan; wire it and the block's dismiss mark to the client's dismiss request, removing the plan only when the server says so and showing a refusal on the notice line; verify with a unit test for the palette's listing.
- [ ] 6.3 Hand the model its plan policy from the settings when the zone is built and on every config reload.

## 7. End-to-end cases (fake ACP agent)

- [ ] 7.1 `agent-plan-progress`: a plan's line and its `--- plan-item` lines, each update behind a `Gate` changing the count and the current step, and the line still in the dump after a long reply.
- [ ] 7.2 `agent-plan-markdown`: a named Markdown plan with a path, and its removal.
- [ ] 7.3 `agent-plan-dismiss`: a finished plan is still shown after the turn ends and after the next prompt; `agent_dismiss_plan` removes it; the fake agent resending it unchanged doesn't bring it back, and resending it with a status changed does; it stays dismissed across a quit and relaunch.
- [ ] 7.4 `agent-plan-restore`: quit and relaunch mid-plan, and the restored zone shows the same plan.
- [ ] 7.5 `agent-plan-settings`: `plan_details = "open"` shows the items at once; `toggle_agent_plan` closes it and it stays closed across an update; a bad value shows the config line.
- [ ] 7.6 Run the agent cases on Windows (one `windows-agent-*` case with a plan) and in the archive jobs.

## 8. Docs and checks

- [ ] 8.1 Update `docs/code-map.md`, `README.md`'s Agent tabs section (the plan, its action and setting), and `CLAUDE.md`'s dump description and action notes.
- [ ] 8.2 Tell `agents-acp-v2` what it inherits: add to its proposal the v2 mapping of design decision 2.
- [ ] 8.3 Run `cargo test --workspace`, the clippy command from `CLAUDE.md`, both harness self-tests and the e2e suite under `scripts/with-xvfb`; `openspec validate agents-plans --strict`.
- [ ] 8.4 Manual checks for the user, with `scripts/run-throwaway.sh` and a real agent profile: a pointer press on the plan's line opens and closes it; the wheel scrolls a long open plan without moving the conversation; a real agent's plan on a task of several steps moves from step to step and stays after it finishes; a pointer press on the dismiss mark puts it away, and it comes back when the agent next changes it.
