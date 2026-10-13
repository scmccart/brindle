# Tasks

## 1. Setting

- [x] 1.1 Add `TmuxSessionEnd { CloseWindow (default), NewTab, Ask }` with `serde(rename_all = "snake_case")` and a `tmux_session_end` field on `Config`. Verify with `cargo test config`: the default is `CloseWindow`, each of `close_window`/`new_tab`/`ask` parses, `"shell"` is an error, and `default_config_file_parses` still passes.
- [x] 1.2 In `assets/default-config.toml`, add a commented `tmux_session_end = "close_window"` beside `restore_session` that lists the three values. Point the tmux profile's "Detaching (ctrl-shift-d) keeps the session running" comment at the setting. Verify that `cargo test config` still parses the default config.
- [x] 1.3 Add `config_edit::set_tmux_session_end(doc, value)` on `set_value`. Verify with `cargo test config_edit`:
  - on `DEFAULT_CONFIG`, only that key's line changes;
  - on a config with `[font]` and no key, the key lands before the first table;
  - a malformed file is refused through `save`.

## 2. Workspace behaviour

- [x] 2.1 Add a `quitting` flag to `TmuxSession`, set in its `on_app_quit` hook before `release()`. `on_detached` still marks panes exited but emits nothing while it is set. Verify with `cargo build`, and with the existing `restore-tmux-session` and `tmux-subscriptions-removed` e2e cases in 4.2.
- [x] 2.2 Rework the `Detached` handler in `workspace.rs` (design decision 2):
  - with other tabs present, remove only the session's tabs;
  - `CloseWindow` removes the tabs, so the window closes;
  - `NewTab` keeps today's profile rule, launched with `cwd: None`, then removes the tabs;
  - `Ask` opens the prompt from 2.3;
  - `Failed` is unchanged.
  - If `WindowClosed` is for the session's last window and the window has nothing else, leave the tab for `Detached` (design, Risks).

  Verify with `cargo build` and the e2e cases in 4.1.
- [x] 2.3 Add `PaletteKind::SessionEnded { session }` and its row builder: "Close window", then "New tab: <name>" for each profile, with the profile detail and the placeholder `tmux session "<name>" ended`. Handle it in `palette_confirmed` (row 0 closes the window; row n launches profile n−1 and removes the session's tabs) and treat `Dismissed` as closing the window. Unit-test the pure row builder (labels and order) with `cargo test workspace`.
- [x] 2.4 Add a palette construction option that ignores mouse-down outside, used only by `SessionEnded`. Verify that a unit or model test, or the dump in 4.1, shows the palette still open after an outside click is simulated through the model. If clicks can't be driven headlessly, record a manual check in 4.4.
- [x] 2.5 In the README, update the ctrl-shift-d row and the tmux control-mode text to say what happens when a session leaves a window's only tabs, and document `tmux_session_end` in the configuration section. Verify by reading the rendered README section against the spec.

## 3. Settings dialog

- [x] 3.1 Hoist the choice-field box (styling plus `SettingsChoice` context wiring) from `settings_dialog/profiles.rs` into a shared helper in `settings_dialog.rs`, and switch the profile editor to it. Verify that `cargo test settings_dialog` passes unchanged.
- [x] 3.2 Add `Section::General` and set `SECTIONS = [General, Profiles, Theme]`. Construct the dialog on General and give the `General` arm to every `match self.section`: never dirty, and confirm and cancel close the dialog. Verify with model unit tests: opening on General, the section order, and `next_section` wrapping from Theme to General.
- [x] 3.3 Add the General model: read the current `tmux_session_end` at open, and `step_session_end(forward)` saves through `set_tmux_session_end`. On success it updates the value and asks the view to reload the config. On failure it keeps the value and sets `error`. Add the `describe()` line `--- settings section=General tmux_session_end="…" error=…`. Verify with unit tests: stepping writes a temp config file, and a malformed file keeps the value and reports the error.
- [x] 3.4 Render the General pane: the label "When a tmux session ends and its tabs were the window's only tabs", a choice field ("Close window" / "Open a new tab" / "Ask"), focus on entering the section, left and right through `SettingsChoiceNext`/`Prev`, and clicks on the arrows. Call `reload_config` after a successful step. Verify with the e2e case in 4.3.
- [x] 3.5 Update the README's settings-dialog row and text to list the sections, General first. Verify by reading the README.

## 4. End-to-end checks

- [x] 4.1 Add e2e cases in `tests/e2e/cases/tmux.rs` and register them in `cases/mod.rs`, each on a throwaway server with the `tmux_session_end` value set in the generated config:
  - `session-end-close`: `--send 'exit\r'`; Brindle exits on its own with status 0 before its dump time.
  - `session-end-detach-close`: `--action tmux_detach`; Brindle exits, and `has-session -t e2e` succeeds.
  - `session-end-new-tab`: the session is killed from the test; the dump shows one local tab.
  - `session-end-ask`: the session is killed; the dump shows the `--- palette` line with "Close window" selected. A second run adds `--action picker_confirm` and Brindle exits.
  - `session-end-close-tab`: with `new_tab`, `--action close_tab` on the session's only window; the dump shows one local tab, so the setting applied rather than an early `%window-close`.

  If `Run` can't yet express "expect exit before the dump", add that to `tests/e2e/brindle.rs`. Verify with `BRINDLE_E2E=1 cargo test --test e2e -- session-end` (ask before running: it opens windows).
- [x] 4.2 Update `tmux-subscriptions-removed`: its `detach` and `close` runs relied on the fallback tab. Set `tmux_session_end = "new_tab"` for them, or expect the early exit, and keep the `removed_before` checks. Verify with `BRINDLE_E2E=1 cargo test --test e2e -- tmux-subscriptions-removed restore-`. `restore-tmux-session` quits through `--dump-screen-after`, which covers the "Quit with only tmux tabs" scenario once the default is `close_window`.
- [x] 4.3 Update the settings cases for the new order: Theme cases in `cases/settings.rs` and `input::settings_tab` (which opens the theme editor) add one `settings_section_prev` after `open_settings`, and Profiles cases keep one `settings_section_next`. The dump parser's self-test strings in `brindle.rs` need no change. Add `settings-general`: `open_settings`, `settings_choice_next`, then the dump shows `section=General tmux_session_end="new_tab"` and the config file contains `tmux_session_end = "new_tab"`. Verify with `BRINDLE_E2E=1 cargo test --test e2e -- settings-`.
- [x] 4.4 Full check: `cargo test`, `cargo build --release`, `openspec validate tmux-session-end-setting`, and the whole e2e suite (after asking the user). Hand the user a manual check: with `ask`, a click on the ended tab behind the palette leaves it open.
