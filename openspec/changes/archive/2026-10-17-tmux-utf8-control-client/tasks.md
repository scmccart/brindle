# Tasks

## 1. The failing case first

- [x] 1.1 Add `Run::env_remove(key)` in `tests/e2e/brindle.rs`, beside `env`, applied to Brindle's command after the harness's own variables. Verify with `cargo build --tests` and `BRINDLE_E2E=1 cargo test --test e2e -- --self-test`
- [x] 1.2 Add the display case `attach-without-locale` (in `tests/e2e/cases/tmux.rs`, registered in `cases/mod.rs`): rename the server's window to `café`, add a window `logs`, run Brindle with `LANG`, `LC_ALL`, `LC_CTYPE` and `TMUX` removed, and check that the dump's tabs are `café` and `logs`. Verify that it fails before the fix with `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- attach-without-locale` (no tabs, or mangled titles)

## 2. UTF-8 control clients

- [x] 2.1 In `src/tmux/mod.rs`, make `command_line` put `-u` before `tmux_args`, and update `command_line_for_profile` and `command_line_for_new_sessions` to expect it. Verify with `cargo test tmux::`, then that `attach-without-locale` passes under `scripts/with-xvfb`
- [x] 2.2 Pass `-u` in `ControlClient::attach` (`tests/e2e/tmux.rs`) and update its doc comment. Verify that the headless `tmux-` cases pass with the locale removed: `scripts/with-xvfb env -u LC_ALL -u LANG -u LC_CTYPE -u TMUX BRINDLE_E2E=1 cargo test --test e2e -- tmux-subscription-format tmux-border-status tmux-rotate tmux-color-report tmux-osc` (with-xvfb exports `LC_ALL`, so the `env -u` must come after it; `TMUX` would make the client UTF-8 anyway)
- [x] 2.3 Reword the reason for the locale in `scripts/with-xvfb`'s header comment and `CLAUDE.md` (the `scripts/with-xvfb` bullet): it keeps pane programs consistent, and tmux no longer needs it. Add to CLAUDE.md's `tmux/` section that control clients run with `-u`. Verify by reading each back and checking `grep -n "mangle\|replaces the tabs" scripts/with-xvfb CLAUDE.md` finds nothing stale

## 3. Integration

- [x] 3.1 Run `cargo test` and the whole e2e suite under `scripts/with-xvfb` (no windows on the desktop). Verify everything passes, and report any failures with their `target/tmp/e2e/<case>/` directory
- [x] 3.2 Run `openspec validate tmux-utf8-control-client --strict` and verify it reports the change as valid
