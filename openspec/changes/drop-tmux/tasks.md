# Tasks

Work on a branch off `main`, one commit per group, each building and passing `cargo test --workspace`. GUI and e2e runs open windows: use `scripts/with-xvfb`, or ask first. Linux checks of Windows-only code run through the push-and-ask routine in CLAUDE.md.

## 1. Tolerate the removed settings and state records

- [x] 1.1 Add `removed_*` fields for top-level `tmux_session_end` and profile `tmux`, `tmux_session`, `tmux_args` (typed `Option<toml::Value>`), warn once per key in `Config::parse` (profile keys name the profile), and never read them in `normalized()`. Verify with config tests: a config with all four keys and odd values loads and logs four warnings, and `tmux_sesion_end` still fails (`unknown_keys_are_errors`).
- [x] 1.2 In `config_edit`, make saving a profile remove the three legacy keys from its table and leave top-level `tmux_session_end` alone. Verify with `config_edit` tests: editing a profile with `tmux = "plain"` drops the keys and keeps comments; the top-level line survives a theme write.
- [x] 1.3 In `session.rs`, replace `TabState::Tmux` with a `#[serde(other)]` unit variant that restore filters out and snapshots never write. Verify with a session test that a state file with a `kind = "tmux"` entry between two local tabs restores the two tabs, and one holding only tmux entries gives the default-profile tab (the `session-restore` scenarios).
- [x] 1.4 Commit as `feat(tmux)!: ignore the removed tmux settings and state records`, then run `cargo test --workspace`.

## 2. Remove control mode and classic profiles

- [x] 2.1 Delete `src/tmux/`, `src/tmux_view.rs` and their `mod` lines; delete `TmuxMode`, `Profile::tmux`/`tmux_session`/`tmux_args`/`tmux_session_name`, `TmuxSessionEnd` and `Config::tmux_session_end` (keeping the `removed_*` fields), and the `tmux` arms of `Profile::description`. Verify `cargo build` reports only the expected fallout in step 2.2.
- [x] 2.2 Collapse `TabContent` in `workspace.rs` into the zone tree entity on `Tab` and delete every `Tmux` branch and helper listed in design decision 4, including the status/bell re-emission, `tab_menu_rows(is_tmux)` and `tab_shortcut`. Verify `cargo build` is clean and `tab_menu_line` and tab-menu unit tests pass without the tmux variant.
- [x] 2.3 In `zone_view.rs`, delete the `TmuxMode::Plain` command construction and the `Direction` import from `tmux`; move `Direction` into `zone_view.rs` (or `layout`) if zones still use it. Verify a zone tests run for split and focus.
- [x] 2.4 In `actions.rs`, delete the `Tmux*` actions, `ACTIONS` rows, the `TmuxWindow` context and bindings, `Swallow` and its ctrl-shift-d binding, and the `tmux: ` prefix handling; fix the tests that mention them. Verify `--list-actions` has no `tmux_` entry and `palette_commands` tests pass.
- [x] 2.5 In `picker.rs`, delete the session-ended and open-elsewhere kinds, `ignores_outside_press` question rules, the prompt-in-place path and the prompt failure path. Verify the palette unit tests pass and the `command-palette` delta scenarios hold (`Rename Tab` prompt, escape, click outside).
- [x] 2.6 In `settings_dialog.rs` and `settings_dialog/profiles.rs`, remove the General section (`SECTIONS` becomes Profiles then Theme, opening on Profiles), the `tmux_session_end` state and the tmux fields, validation and preview text; update the editor's "apply to new tabs" sentence. Verify dialog unit tests pass and `settings` e2e cases (updated in group 4) describe two sections.
- [x] 2.7 In `session.rs`, delete `SnapshotTab::Tmux`, `tmux_target`, the session dedupe and ordering code and the tmux branches of `merge_windows` and `drop_lost_zones`; in `main.rs`, delete the attach-wait comment and tmux profile examples in tests (use another profile name). Verify the session unit tests pass.
- [x] 2.8 Update the default config (`assets/default-config.toml`): drop `tmux_session_end` and the `tmux`, `tmux (classic)` and WSL profile blocks and their comments. Verify `Config::parse(DEFAULT_CONFIG)` tests and the number of default profiles asserted in `config.rs` tests.
- [x] 2.9 Commit as `refactor(tmux)!: remove control mode and classic tmux profiles`, then run `cargo test --workspace`.

## 3. Remove remote tmux and tmux-only terminal plumbing

- [x] 3.1 In `remote/mod.rs`, delete `tmux_command`, `TmuxProcess`, `detach_command`, the dropped-client detach and their tests; update the module comment. Verify `cargo test -p brindle remote::` passes.
- [x] 3.2 In `terminal/mod.rs`, delete `PaletteHandler`, tmux pane colour defaults, the tmux-specific `reply()` and `feed()` comments and code, and `fixed_size` if nothing else sets it (check `terminal_element.rs` and `cell_metrics()` callers). Verify `cargo test -p brindle terminal::` and that the message-tab path still builds.
- [x] 3.3 Remove tmux-only code from `win.rs`, `terminal/cmdline.rs` (the `wsl` detection only, keeping `program_stem`), `ui/src/filter.rs`, `ui/src/components/menu.rs`, `ui/src/text_input.rs` and `ui/examples/gallery.rs`: change tmux-only strings and fixtures in comments and tests to neutral ones, and remove what only tmux used. Verify `cargo test --workspace` and `cargo build -p brindle-ui --example gallery`.
- [x] 3.4 Remove `BRINDLE_TMUX_STATE` and the e2e passthrough of it, and scan for `BRINDLE_CONTROL_CLIENT` and `tmux` in `src/` with `grep -ri tmux src ui term-core remote-agent remote-protocol`; every remaining hit must be an ordinary program mention in a comment or a test of the scanner. Verify the grep output is reviewed and clean.
- [x] 3.5 Commit as `refactor(tmux)!: remove remote tmux and the tmux-only terminal plumbing`, then run `cargo test --workspace` and `cargo build --release --locked` (route the release build to a Haiku chore per CLAUDE.md).

## 4. Tests, perf and CI

- [x] 4.1 Delete `tests/e2e/tmux.rs`, `tests/e2e/cases/tmux.rs`, `tests/common/tmux.rs` and their registrations; delete the perf `latency-tmux` and `throughput-tmux` cases and `tests/perf/fixtures` tmux entries. Verify `cargo test --test e2e -- --list` and `cargo test --test perf -- --self-test` (with `BRINDLE_E2E=1` / `BRINDLE_PERF=1`) list no tmux case and compile.
- [x] 4.2 Audit every other file in `tests/` that mentions tmux (`grep -ril tmux tests`): rewrite cases that used tmux as a pane source against a shell or two-zone tab, delete cases that exist only for tmux behaviour (including `session_end`, `multi_window` tmux parts and the WSL and `remote-*` tmux cases), and list in the commit message which cases were deleted and which rewritten. Verify the e2e list under `scripts/with-xvfb` and that no case's expectation in `openspec/specs` loses coverage (the zone and card geometry scenarios in particular).
- [x] 4.3 Run the e2e suite under `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e` (headless, so no desktop windows open). Verify all remaining cases pass; investigate any failure rather than skipping.
- [x] 4.4 Update `.github/workflows/ci.yml`: remove the tmux matrix, the source build and cache steps, the version check, the `old-tmux` input and the header comment about it; rename artifacts to `e2e-shard-<k>`; keep the `tmux` apt package only if group 4.2 kept a case that runs tmux as a program; reconsider the shard count from the new case total. Verify with `actionlint` if available (else `gh workflow view` after pushing) and that the PR's CI jobs start.
- [x] 4.5 Update `tests/fixtures/remote/Dockerfile` to stop installing tmux unless a retained remote case needs it. Verify `docker build` of the fixture where Docker is available (skip otherwise, and say so in the PR).
- [ ] 4.6 Commit as `test(tmux)!: remove the tmux cases and CI matrix` (and a separate `ci:` commit if the workflow change reads better alone), then push and wait for the `commits`, `test`, `release`, `windows`, `windows-e2e` and `e2e` jobs.

## 5. Docs and packaging

- [x] 5.1 Update `README.md`: remove the tmux sections, keybinding tables rows, config keys and the tmux profile examples; add a short "Using tmux" note (run it as a profile command, with the two-line example) and a "Upgrading" note for the removed keys. Verify by searching the README for stale tmux features and by running its documented profile snippet in a throwaway config with `--dump-screen-after` under `scripts/with-xvfb`.
- [x] 5.2 Update `CLAUDE.md`: remove the tmux architecture, tmux-mode e2e and subscription/polling notes, the `BRINDLE_TMUX_STATE` line, the control-mode manual checks (`classic (plain) tmux profiles with the prefix keys`, `ctrl-shift-d does nothing…`) and the Windows WSL tmux case; fix the CI paragraph (matrix, `old-tmux`) and the e2e description. Verify `grep -n -i tmux CLAUDE.md` shows only deliberate mentions.
- [x] 5.3 Update `Cargo.toml`'s description and `packaging/winget/scmccart.brindle.locale.en-US.yaml`'s short description and tags to drop "first-class tmux support"/`tmux`; check `scripts/run-throwaway*` for tmux text. Verify `scripts/winget-manifest.ps1` output via the `windows` CI job's `winget validate`.
- [x] 5.4 Commit as `docs: drop tmux from the README, CLAUDE.md and package descriptions`.

## 6. In-flight changes and the teammate-mode change

- [x] 6.1 `git rm -r openspec/changes/test-claude-code-teammate-mode`. Verify `openspec list` no longer shows it.
- [x] 6.2 Edit `add-blocks` (proposal scope, design alternatives line, task 7.1's tmux case), `port-to-macos` (proposal PATH wording, design keybinding rows and tmux text, tasks 7.1 and the Homebrew-tmux check, `specs/macos-platform/spec.md` scenario), `session-layout-in-server` (the tmux placeholder bullet and open question), `remote-connections-followups` (tmux bullets and questions) and `terminal-server-remote` (the "remote tmux stays outside the server" line). Verify `grep -n -i tmux` over those changes shows no reference to removed behaviour, and `openspec validate --changes` still passes for each that has specs.
- [x] 6.3 Commit as `docs(openspec): drop tmux from the in-flight changes and remove test-claude-code-teammate-mode`.

## 7. Integration checks

- [x] 7.1 With a config containing the old default `tmux` and `tmux (classic)` profiles, `tmux_session_end = "ask"` and a state file with a `kind = "tmux"` entry, launch Brindle under `scripts/with-xvfb` with `--dump-screen-after 3` and verify: it starts without an error banner, the log shows the warnings, the old tmux tabs are not restored, and the `tmux (classic)` profile opens a shell.
- [x] 7.2 Run `scripts/lint-commits` and `scripts/lint-commits --title "feat(tmux)!: drop tmux support"`. Verify both pass.
- [ ] 7.3 Ask the user to hand-check, with `scripts/run-throwaway.sh`: a profile with `command = "tmux"` and `args = ["new-session", "-A", "-s", "main"]` opens and works with its prefix keys and mouse; ctrl-shift-d reaches the shell; the settings dialog has Profiles and Theme only; no tmux entry appears in the palette or the profile picker.
- [ ] 7.4 Open the PR with the title `feat(tmux)!: drop tmux support`, a body stating the breaking change and the migration lines, and the attribution lines; user merges.

## Workflow follow-up

- After merge, archive the change with `/opsx:archive` (sync delta specs without prompting): then edit the `Purpose` lines of `remote-connections`, `session-restore`, `scrollback-search`, `links`, `shell-integration`, `profiles` and `zones` and the tmux wording in `terminal-emulation`'s purpose, which deltas can't change; check that `openspec/specs/tmux-control-mode` is gone or empty and remove it if archive leaves it behind; verify `openspec validate --specs`.
- Update the user's saved notes that describe tmux behaviour (`claude-code-tmux-256color`, `tmux-followups`, `program-status-osc7501`'s tmux remarks) so they stop pointing at removed code.
