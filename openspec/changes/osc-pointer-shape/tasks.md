# Tasks

## 1. Pure model (`brindle-term-core`)

- [ ] 1.1 Confirm kitty's semantics for set and push with several names and for the empty body against kitty's source, and update this change's spec if they differ
- [ ] 1.2 Add `term-core/src/pointer.rs` (`Shape`, `Op`, `Query`, `Screen`, `parse`, `Stacks`) and add it to `PURE_MODULES`; verify unit tests for every spec scenario, byte-exact replies and the name round trip with `cargo test -p brindle-term-core pointer`
- [ ] 1.3 Add `(22, 2)` to the scanner's wanted numbers returning `Found::Pointer`, make `Emulator::feed` ignore it, update the fuzz target, and verify `osc` unit tests for split reads and cancellation

## 2. Client (`src/terminal/`)

- [ ] 2.1 Switch `Sink::write` to `scan_positions` and split the parse at pointer finds to tag their `Screen`; verify a `server.rs` unit test for a push made on the alternate screen
- [ ] 2.2 Keep `Stacks` in `Terminal`, apply ops and write replies in `handle_found`, clear at exit, reset, `REPLAY_START` and the overload reset; verify `cargo test -p brindle terminal`
- [ ] 2.3 Add `src/terminal/pointer.rs` with the `Shape → CursorStyle` table and `choose`, and use it in `TerminalElement`'s prepaint and paint; verify unit tests for the table and the precedence

## 3. End to end

- [ ] 3.1 Print `--- pointer shape=… screen=…` from `--dump-screen-after` (documented in CLAUDE.md's debug-hooks paragraph), add `pointer-query`, `pointer-current` and `pointer-dump` in `tests/e2e/cases/pointer.rs`, and verify `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- pointer`
- [ ] 3.2 Manual check for the user (real pointer): a program setting `ew-resize` shows it over the grid only, and ctrl over a link still shows the hand

## 4. Docs

- [ ] 4.1 Update `docs/code-map.md` (`terminal/`, `terminal_element.rs`, `brindle-term-core`) and README.md (supported sequences and shapes), and verify `cargo test --workspace` and `cargo clippy --locked --workspace --all-targets -- -D warnings`

## Workflow follow-up

- Archive the change on the implementation PR once its checks pass.
