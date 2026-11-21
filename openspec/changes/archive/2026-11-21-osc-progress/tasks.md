# Tasks

## 1. Pure progress model (`brindle-term-core`)

- [x] 1.1 Add `term-core/src/progress.rs` with `Percent`, `Progress`, `Report`, `parse`, `apply` and `most_pressing`, add it to `PURE_MODULES` in `tests/architecture.rs`, and verify unit tests for every `program-progress` scenario (states, clamping to 100, error/paused keeping the previous percent, unknown state ignored, aggregation order) pass with `cargo test -p brindle-term-core progress`
- [x] 1.2 Return `Found::Progress(Report)` from `osc::Scanner` for `9;4;…` before `parse_osc9`, update the module docs and the fuzz target, and verify `osc` unit tests for split reads, cancellation, `9;4` vs `9;9` vs `9;42` pass
- [x] 1.3 Keep `progress: Option<Progress>` in `Emulator` with the lifetime rules (report, `PromptStart`, `Reset`) and verify a unit test that feeds a report then an `A` ends with no progress
- [x] 1.4 Pass progress to `replay::render` and write one `9;4` report after the records, update the file's fidelity notes, and verify an emulator rebuilt from a render has the same progress, including when the replay contains OSC 133 `A` marks

## 2. Server core

- [x] 2.1 Drop the progress where the core ends a session's activity on exit, if the core does so for records, and verify with `cargo test -p brindle-remote-agent` that a core test attaching after a `9;4` report receives it in the replay

## 3. Client (`src/terminal/`)

- [x] 3.1 Handle `Found::Progress` in `Terminal::handle_found`, clear on `PromptStart`, `Reset`, process exit and the event-overload reset of `Records`, and emit `TerminalEvent::Status(None)` on change; verify with unit tests of the decision helpers and `cargo test -p brindle terminal`
- [x] 3.2 Print `--- progress …` lines from `--dump-screen-after` and document them in CLAUDE.md's debug-hooks paragraph

## 4. UI

- [x] 4.1 Cache `Tab::progress` (`progress::most_pressing` over the tab's terminals) where `Tab::status` is refreshed, and draw the 2-pixel bar on the tab face in `render_tab_bar` without changing the face's height; verify `cargo test -p brindle workspace`
- [x] 4.2 Draw the zone header's bar in `zone_view.rs` for multi-zone tabs
- [x] 4.3 Add e2e cases (`tests/e2e/cases/progress.rs`, registered in `cases/mod.rs`): `progress-report` (dump shows normal 30% after a split report, no notification logged), `progress-clears-at-prompt`, `progress-zones` (tab takes the most pressing zone), `progress-bar-colour` (capture samples the red bar for an error), and `local-server-progress-attach` in `local_server.rs`; verify `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- progress`

## 5. Docs and integration

- [x] 5.1 Update `docs/code-map.md` (`terminal/`, `brindle-term-core`, `notify/osc.rs` entries) and README.md (supported sequences, which programs emit `9;4` and how they detect support), and verify `cargo test --workspace` and `cargo clippy --locked --workspace --all-targets -- -D warnings` pass

## Workflow follow-up

- Archive the change on the implementation PR once its checks pass.
