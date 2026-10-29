# Tasks

## 1. Parsing and records

- [x] 1.1 Add `src/terminal/status.rs`: `State`, `Kind`, `Record`, `Message`, `parse`, with every limit and discard rule from specification sections 3, 5, 6 and 9, base64 with optional padding, and the control-character and bidi-control handling. Unit tests cover the spec's examples, each limit at its edge, a malformed pair, a repeated key, an unknown key, an unknown state, and whitespace. Verify with `cargo test terminal::status`
- [x] 1.2 Add `Records` to the same module: `apply`, `clear`, `prompt_start`, `process_exit`, `seen`, `reset`, `summary`, `effective_app`, the 256-record cap, and `describe` (the dump lines). Unit tests cover the lifetime table, clearing a subtree, eviction order, `app` inheritance and the notice transitions. Verify with `cargo test terminal::status`

## 2. Scanning and the terminal

- [x] 2.1 Make `notify::osc::Scanner` collect OSC 7501 and OSC 133 and detect `ESC c`, returning `Found` values; keep the existing OSC 9/777 tests green and add ones for a report, a query, a prompt start, a reset, and a 7501 report split across reads. Verify with `cargo test notify::osc`
- [x] 2.2 Thread `Found` through `ChannelEvent`, `ScanReader` and `Terminal::feed`; give `Terminal` its `Records`, the query reply, the `Status` event, and the `mark_exited` and `input` rules. Verify with `cargo test terminal::`

## 3. Presentation

- [x] 3.1 Add `Source::Osc7501`, `NotificationConfig::osc7501`, and the `policy::wanted` arm, with a config unit test. Verify with `cargo test config:: notify::`
- [x] 3.2 `TmuxWindowView` re-emits `Status` and exposes `status_summary`; `Workspace` keeps `Tab::status`, refreshes it from `on_attention`, notifies for `Status(Some(_))`, and draws the dot. Verify with `cargo build` and a `--dump-screen-after` run that prints `--- status` lines (ask before the GUI run)
- [x] 3.3 Print `--- status` lines from `dump_active_screen`, and parse them in the e2e `Dump` (`status: Vec<String>`), with a self-test. Verify with `BRINDLE_E2E=1 cargo test --test e2e -- --self-test`

## 4. Tests and docs

- [x] 4.1 Add e2e cases: `status-local` (a local tab: `working` then `blocked` reports, the dump's `--- status` line, the `osc7501` notification log line, the `?` reply echoed by the shell, and a new prompt dropping `working`) and `status-tmux-pane` (a `blocked` report from a pane in a background tmux window: the notification names the window). Verify with `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- status-`
- [x] 4.2 Document the protocol in README.md's "Desktop notifications" section (what the dot means, the setting, the shell one-liner) and in CLAUDE.md (the scanner's job, `terminal/status.rs`, the dump line). Verify by reading them back
- [x] 4.3 Run `cargo test`, `cargo build --release` and the whole e2e suite under `scripts/with-xvfb`, and `openspec validate program-status --strict`. Verify everything passes
