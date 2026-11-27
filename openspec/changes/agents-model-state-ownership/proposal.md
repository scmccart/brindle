# Proposal

**Depends on:** `agents-module-structure` (the chat model split into `src/agent/model/`) and `agents-absorb-cost` (the two absorb halves, `draw`, and the kept facts, which give the conversation list its own state), both implemented and archived.
**Next:** nothing depends on it.

**Status: proposal stage.** This change has no design, specs or tasks yet. It records structural debt that the 2026-11-26 agent code review named (`docs/architecture/audits/2026-11-26-agents/d.md`, structural observation 1) and that the remediation takes only in part: `agents-module-structure` decision 20 moves the model's methods into modules and leaves every field on `ChatModel`.

## Why

After `agents-module-structure`, `src/agent/model/` holds a dozen files (`phase.rs`, `blocks.rs`, `ledger.rs`, `queue.rs`, `prompt.rs`, `login.rs`, `recovery.rs`, `choosing.rs`, `paging.rs`, `attention.rs`, `dump.rs`), each an `impl ChatModel` block. The files are smaller, but the state is not divided: `ChatModel` is still one struct of several dozen private fields, and every module can read and write all of them. So:

- Nothing says which fields a concern owns. A change to paging can touch the ledger's `pending` map or the blocks' caches, and the compiler has no opinion (`types R11`).
- A field's invariant has no home. `agents-client-state-fixes` fixes three cases of two fields that must agree and didn't (`Earlier::Loading`'s request id and the ledger, `first` and `first_shown`, the toggles and the log they belong to). Each is fixed where it was found; the shape that let them happen stays.
- A concern can't be tested alone. Every model test builds a whole `ChatModel` through the test `Zone`.

The review's proposal is that each concern owns its fields and returns its own part of `Absorbed`, with `ChatModel` the composition that calls them in `finish`. It was left out of the remediation because it is not a move: `phase`, `turn`, `slot`, `entries` and `pending` are read by most of the modules, so dividing them means deciding who owns each and what the others are handed.

## What Changes

- Each model module that has state of its own becomes a type holding it: the conversation list (`entries`, `rendered`, `texts`, `blocks`, the toggles, the facts and undrawn marks), the ledger (`pending`, the uncertain prompt), paging, the login row, choosing, and the first prompt.
- `ChatModel` holds those values and the few shared facts (the phase, the turn, the slot, the link, the session's life and handle), and passes the shared facts to a concern as arguments.
- `finish` composes each concern's result into `Absorbed`.
- No behaviour, dump, wire or state change is intended; the e2e agent cases and the model's tests pin that.

Open questions, to answer before a design:
1. **Which facts are shared.** The phase, turn and slot are read by the dump, attention, the queue row and the prompt. Are they one `Session` value passed by reference to each concern, or does one concern own each and expose readers?
2. **Where the cut pays.** The conversation list is the clear case: `agents-absorb-cost` already gives it its own state and its own two-step protocol. Which of the others have invariants worth a type (the ledger and paging, by `agents-client-state-fixes`' findings), and which are a few functions over shared facts that should stay methods (`attention.rs`, `recovery.rs`, `dump.rs`)?
3. **The dump.** `describe` reads everything. Does it take each concern's own `lines`, or stay a reader of the whole model?
4. **Tests.** Whether each owning type gets tests that build only it, and what the test `Zone` keeps.
5. **Order with the open agent changes.** `agents-prompt-context`, `agents-prompt-images`, `agents-plans` and `agents-chat-selection` each add model state. This is cheapest when none of them is in flight.

## What the full changes already say

- `agents-module-structure`, design decisions 5, 20 and 21: the module list, why the fields stay, and why the modules are children of `model/` (so they reach `ChatModel`'s private fields without widening them).
- `agents-absorb-cost`, design decisions 2 to 4 and 7: `ChangeCopy`, `absorb`, `draw`, `Undrawn` and `Facts`, the state the conversation list would own.
- `agents-client-state-fixes`, design decisions 3, 5 and 7: `FirstState`, the uncertain prompt and `Paging`, the three places where two facts became one.
- `docs/architecture/illegal-states-unrepresentable.md`, `types R1`, `R10` and `R11`: exclusive states as one enum, derived state named for what refreshes it, and private fields behind methods.

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- None: no behaviour changes. The change declares no spec, as `agents-module-structure` does.

## Impact

- `src/agent/model/`: every module, `mod.rs` most of all.
- `src/agent/mod.rs`, `src/agent/view.rs` and `src/agent/tests.rs`: only where they name a model type that moves.
- `tests/architecture.rs` and `docs/architecture/functional-core-imperative-shell.md`: unchanged unless a file is added to `model/`.
- `docs/code-map.md`: the agent entry's model paragraphs.
