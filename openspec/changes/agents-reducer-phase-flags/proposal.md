# Proposal

**Depends on:** `agents-module-structure` (the reducer split into `acp/reduce/`, with `lifecycle.rs` holding the phase) and `agents-checkpoint-deltas` (the checkpoint's delta shape, which this changes again), both implemented and archived.
**Next:** nothing depends on it.

**Status: proposal stage.** This change has no design, specs or tasks yet. It records structural debt that the 2026-11-26 agent code review named (`docs/architecture/audits/2026-11-26-agents/a.md`, structural observation 5) and that no remediation change takes: `agents-module-structure` decision 19 left it out because it is not a move, and the remediation plan lists it under "Not planned".

## Why

Three fields of the reducer's `AgentSession` sit beside its `Phase` and are only meaningful in some phases:

- `chose: bool`: whether the continue that is out was chosen while the agent was `Phase::Choosing`. `continue_refused` and `start` read it to decide whether a refused continue returns to choosing, and `wait_for_choice` and `continued` must each remember to reset it.
- `lists: bool`: whether the agent said it can list conversations. It comes from the agent's `initialize` answer.
- `methods: Vec<LoginMethod>`: the login methods the agent offers, from the same answer.

A flag beside the state it qualifies can disagree with it (`types R1`): nothing stops `chose` being true in a phase with no continue out, and every new path through the start sequence has to know to reset it. The review's proposal is to make the type carry the fact: an origin on the continue itself, and the two capabilities in the `Greeting` they arrived with.

It was left out of the remediation because it changes what a checkpoint records, which a no-behaviour move may not do:

- `methods` and `lists` are written whenever they are set, including in `Phase::Exited` and `Phase::Spawning`, which carry no greeting. A greeting-owned field would be dropped there, so a core restart in those phases would lose the method list.
- `chose` survives an exit today, and a restart whose continue is refused returns to choosing with it still set. An origin on `Start::Continue` ends with the start.

## What Changes

- `chose` becomes part of the continue that is out (an origin on `Start::Continue`, carried through `Resuming`, `Loading` and `AfterLogin::Start`), so the guards in `start` and `continue_refused` match on the type.
- `lists` and `methods` move into the `Greeting`, or into a value that outlives the greeting where a phase without one still needs them.
- The checkpoint's state changes to match, in the unreleased format 2 if it is still unreleased, else in a new format with the earlier one still read (`process R4`–`R6`, `R15`).

Open questions, to answer before a design:
1. **Phases with no greeting.** `Exited` and `Spawning` need `methods` (a login row shown for an ended agent) and `lists` (whether Conversations… is offered) after the greeting is gone. Does the greeting outlive those phases, do the two move to a "last known capabilities" value, or do those phases stop needing them?
2. **`chose` across an exit.** Today a chosen continue that ends in an exit and a restart keeps `chose`. Is that behaviour wanted (the restart's refused continue returns to choosing), or an accident of the flag that the type should remove? The answer decides whether this change has a behaviour delta and a spec.
3. **The checkpoint.** If format 2 has shipped by then, this needs format 3 and a reader for format 2. If it hasn't, the plan's decision D2 applies: the earlier shape need not be read.
4. **Whether it is worth it.** The three fields are documented beside the phase after `agents-module-structure`. If questions 1 and 2 show the flags are the honest shape (state that really is independent of the phase), close this change with that recorded.

## What the full changes already say

- `agents-module-structure`, design decision 19: why the move isn't a move, with the two checkpoint differences above. It documents the fields' meaning beside them instead.
- `agents-checkpoint-deltas`, design decision 4: `methods` is written to a segment only when it changed, and stays a top-level record field.
- `agents-session-list` (archived): introduced `chose`, `lists` and `Phase::Choosing`.
- `agents-login` (archived), decision 8: what a restart for a login resets.

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `agent-sessions`: only if question 2 changes what a refused continue after a restart does. Otherwise none, and the change declares no spec, as `agents-module-structure` does.

## Impact

- `remote-agent/src/acp/reduce/lifecycle.rs` (the phase, the start sequence, `continue_refused`, `wait_for_choice`), `login.rs` and `list.rs` (the readers of `methods` and `lists`).
- `remote-agent/src/acp/checkpoint/state.rs`: the writers and readers of the three fields, with the round-trip and pinned-bytes tests.
- No client change: the transcript events that carry login methods and the conversation are unchanged.
