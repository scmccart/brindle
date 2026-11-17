# Proposal

## Why

The 2026-11-16 architecture audit (`docs/architecture/audits/2026-11-16.md`, finding 4; change 3 of `2026-11-16-remediation.md`) found input from outside the process that is unbounded, can panic or is never fuzzed, and wire encodings no transcript pins:
- `holder::Assembler` caps each blob's length but not how many blobs are open. A peer can open one per session id, and each costs up to 1 MiB of up-front allocation for a 9-byte frame (`railway R5`).
- `EditState::range_from_utf16` doesn't order a range, so `text_for_range` would panic on a reversed one. No GPUI 0.2.2 backend sends one ("Finding 4b traced"), so this is hardening.
- Several parsers of outside bytes have only fixed-case tests and no fuzz test: the relay's `skip_preamble`, `SessionMeta::from_value`, `upgrade::semver`, `server_dir::parse_holder_lock`, and term-core's OSC 7, 133 and 7501 parsers (`railway R5`, `process R6`).
- No transcript pins `Exit.lost = Some(..)`, `Previous::Crash` or `ExitStatus { code: None }`. The coverage test checks message types, not variants. `BRINDLE_REGEN_FIXTURES=1` can rewrite shipped fixtures (`process R5`, `R6`).

The fix is small and has no dependencies. Landing it before the larger changes means their decoders and messages join a harness that already checks variants.

## What Changes

- **Open-blob cap:** `holder::Assembler` refuses a `*_BEGIN` once `MAX_OPEN_BLOBS` blobs of its kind are open, on the same error path as an over-long blob. The H1 module doc states the bound.
  - Every shipped sender sends a blob as one contiguous run, so at most one is open at a time.
  - Neither protocol version moves, and no encoding changes.
- **Range ordering:** `EditState::range_from_utf16` returns an ordered range, so no range a platform passes can panic `text_for_range`.
- **New fuzz targets in `remote-protocol/src/fuzz.rs`:** `skip_preamble`, `SessionMeta::from_value`, `upgrade::semver` and `is_newer`, and `server_dir::parse_holder_lock`. Each asserts it never panics, plus a property:
  - the noise and stream position that `skip_preamble` leaves;
  - a meta round trip;
  - `is_newer` is never true both ways;
  - a holder lock name round trip.
- **term-core fuzz tests:** a new `term-core/src/fuzz.rs`, with its own copy of the seeded generator because shared crates never reference each other. It feeds random streams, biased toward OSC 7, 9;9, 133 and 7501 and the splits around them, to:
  - `Emulator` (`Parser`, `Marks`, `osc::Scanner`, `Records`) and its replay;
  - `osc::Scanner`, which must find the same sequences however the stream is cut;
  - `status::parse`, whose reports must survive `Record::encode` and a second parse unchanged.

  `shell_integration.rs` adds a fuzz test of `reported_dir` in both path styles and of `parse_osc133`.
- **Minimum grid width:** `alacritty_terminal` 0.26 panics (`grid/mod.rs:439`) when a 1-column grid is fed a double-width character, which the emulator fuzz test found. No grid Brindle creates or resizes is narrower than `MIN_COLS = 10` columns: `term-core`'s `emulator::MIN_COLS`, used by the server emulators' `size()`, the client's `Terminal::resize` (through `GridSize::clamped`) and replay size, `fit_grid`, and the split check (`can_split_zone`). The wire is unchanged: a listed or requested size is clamped where it is used. The fuzz generator is not narrowed.
- **Transcripts:** two new fixtures and three harness changes:
  - fixture `client-session-lost` (protocol 1: an `EXIT` with `lost`);
  - fixture `holder-adopt-after-crash` (H1: `previous: crash`, and a held session ended by a signal);
  - the coverage test names every variant of the five message enums and the nested `Previous`, `Policy`, `KillTarget` and `Exit.lost` and `ExitStatus.code` forms through exhaustive matches, instead of ranges of message numbers;
  - `BRINDLE_REGEN_FIXTURES=1` writes only fixtures that don't exist yet, so a changed shipped fixture fails instead of being rewritten;
  - the module doc says protocols 1–3.
- **Docs:**
  - the stale "protocols 1 and 2" in `remote-protocol/src/lib.rs` and `relay.rs`;
  - the relay's number-collision test, which skips `SET_LAYOUT` (14);
  - where fuzz tests live, in `testing-architecture.md`, the Enforcement sections of `railway-oriented-programming.md` and `process-architecture.md`, and `CLAUDE.md`.

No user-visible behaviour changes. A holder past the cap is a broken holder, and the core drops it as it drops one that sends an over-long blob. No rule changes: `railway R5` and `process R5`–`R7` are applied as written.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `zones`: "Minimum size for splitting" counts 10 columns instead of 2 (`MIN_COLS`), and gains the scenario that a pane narrower than 10 columns is laid out as 10 wide.
- The other specs hold as written: `terminal-server` "Failures stay small" and "Protocol versions", and `text-input` "Input methods" (no platform sends a reversed range).

## Impact

- **Code:**
  - `remote-protocol/src/holder.rs` (`Assembler`, `MAX_OPEN_BLOBS`, the H1 module doc);
  - `remote-protocol/src/fuzz.rs`;
  - `remote-protocol/src/lib.rs` and `relay.rs` (docs, the collision test);
  - `remote-protocol/tests/transcripts.rs` and two new fixtures in `remote-protocol/tests/fixtures/`;
  - `ui/src/text_input.rs` (`range_from_utf16`);
  - `term-core/src/lib.rs`, the new `term-core/src/fuzz.rs`, and the tests in `term-core/src/shell_integration.rs`.
- **Unchanged:**
  - Callers in `remote-agent/src/server/core/holders.rs` (`blob`) and `remote-agent/src/server/holder.rs` (`CheckpointBegin`) keep their error paths.
  - No wire encoding, protocol version, config key or state file changes.
- **Docs:**
  - `docs/architecture/testing-architecture.md` (the layer table);
  - `railway-oriented-programming.md` and `process-architecture.md` (Enforcement);
  - `CLAUDE.md` (the protocol and Tests paragraphs).
- **Dependencies:** none added. The term-core generator is a copy, because `ports R3` keeps the shared crates apart.
- **Overlap with other changes:**
  - **`port-to-macos`:** GPUI's macOS backend calls `text_for_range` with an `NSRange`, which is ordered. The hardening covers it, and its tasks need no edit.
  - **`add-blocks`:** it adds per-command state to `shell_integration.rs`'s `Command`, set from the `C` and `D` marks. That code then runs under the term-core fuzz test, and whichever change lands second keeps the fuzz test passing.
  - **`add-pocket-client`:** it may gate `remote-protocol`'s server-side modules (`server_dir` among them) and term-core behind features for `espidf`. The `parse_holder_lock` fuzz target then takes the same gate.
  - **`update-dependencies`:** no overlap. `alacritty_terminal` stays at 0.26.0, and `serde_json` and `base64` bumps run through the new fuzz tests.
  - **Remediation siblings:**
    - change 5 (`enforce-platform-seams`) turns term-core's `windows_paths` into a `Parser` argument, so the fuzz test builds the parser through whichever constructor exists when it lands;
    - change 12 (`surface-swallowed-failures`) makes `SessionMeta::from_value` return a refusal, so its fuzz target then asserts `Ok` or the refusal;
    - change 9 (`bound-queues-and-contain-panics`) adds `catch_unwind` to `SessionSink`, and these fuzz tests are the evidence that the guard is rarely needed.
