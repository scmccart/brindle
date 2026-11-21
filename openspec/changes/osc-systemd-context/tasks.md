# Tasks

## 1. Pure context model (`brindle-term-core`)

- [ ] 1.1 Add `term-core/src/context.rs` (`ContextId`, `Kind`, `Fields`, `Message`, `parse`, `Contexts::{apply, clear, notable}`, `Badge`), add it to `PURE_MODULES`, and verify unit tests for every `program-contexts` scenario pass with `cargo test -p brindle-term-core context`
- [ ] 1.2 Add `(3008, 4)` to the scanner's wanted numbers and return `Found::Context`, update the module docs and the fuzz target, and verify `osc` unit tests for split reads, cancellation and over-long payloads
- [ ] 1.3 Keep `Contexts` in `Emulator`, cleared only by process exit, and write the stack in `replay::render` with re-escaping; verify a render/feed round trip (escaped ids, nested contexts, an `ESC c` mid-stream not clearing)

## 2. Server core

- [ ] 2.1 Clear a session's contexts where the core handles its process's exit, and verify with a `remote-agent/tests/core.rs` test that an attach after `3008;start=…` replays it and one after exit doesn't

## 3. Client (`src/terminal/`)

- [ ] 3.1 Handle `Found::Context` in `Terminal::handle_found`, cache the badge, clear at process exit and at `REPLAY_START`, emit `TerminalEvent::Status(None)` on change; verify with `cargo test -p brindle terminal`
- [ ] 3.2 Print `--- context …` lines from `--dump-screen-after` and document them in CLAUDE.md's debug-hooks paragraph

## 4. UI

- [ ] 4.1 Cache `Tab::badge` from the active zone and draw the badge on the tab face (title truncating first, to 8 characters), and verify `cargo test -p brindle workspace`
- [ ] 4.2 Draw the badge on zone headers in `zone_view.rs`
- [ ] 4.3 Add e2e cases (`tests/e2e/cases/context.rs`): `context-badge` (dump and a red pixel sample), `context-end`, `context-zones`, and `local-server-context-attach` in `local_server.rs`; verify `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- context`
- [ ] 4.4 Manual check for the user: on a systemd ≥ 257 machine, `run0 bash` shows a red `root` badge that goes when it exits

## 5. Docs and integration

- [ ] 5.1 Update `docs/code-map.md` (`terminal/`, `brindle-term-core`, `notify/osc.rs`) and README.md (supported sequences; the badge is informational and can be spoofed), and verify `cargo test --workspace` and `cargo clippy --locked --workspace --all-targets -- -D warnings`

## Workflow follow-up

- Archive the change on the implementation PR once its checks pass.
