# Tasks

## 1. Action and prompt flow

- [x] 1.1 Add a `RenameTab` action, config name `rename_tab`, and command-palette label `Rename Tab` in `src/actions.rs`.
- [x] 1.2 Handle `RenameTab` in `Workspace` so it opens a `PromptRequest` for the active tab with the current displayed title pre-filled and selected.
- [x] 1.3 Trim submitted prompt text, set a manual title from non-empty trimmed text, and clear the manual title from empty trimmed text.
- [x] 1.4 Notify and refresh the active tab/window title after setting or clearing a manual title.

## 2. Manual title state

- [x] 2.1 Add manual-title state to `Tab` while keeping `Tab.title` as the displayed-title cache used by rendering, dumps, and window-title updates.
- [x] 2.2 Update `Tab::refresh_title` so manual titles override automatic local and tmux title sources without disrupting existing automatic behavior when unset.
- [x] 2.3 Ensure OSC title changes, foreground-process polling, profile fallback, and tmux window-name observations cannot replace a set manual title.
- [x] 2.4 Ensure clearing a manual title recomputes and displays the current automatic title.

## 3. Session restore

- [x] 3.1 Extend local session state with an optional `title` field that defaults to absent and is skipped when no manual title is set.
- [x] 3.2 Carry local manual titles through snapshot, collapse, restore resolution, and `Workspace::restore`.
- [x] 3.3 Leave control-mode tmux session restore unchanged so Brindle-local tmux manual titles are runtime-only.
- [x] 3.4 Add or update session-state tests for reading old state without titles and restoring/writing local manual titles.

## 4. Verification

- [x] 4.1 Add unit coverage for title refresh behavior: automatic title when unset, manual title overriding automatic changes, and automatic title resuming after clear.
- [x] 4.2 Add an e2e case or extend an existing no-synthetic-input case to run `rename_tab`, submit a trimmed title, and verify `--dump-screen-after` reports the manual title.
- [x] 4.3 Add e2e coverage that submitting whitespace clears the manual title and automatic title rules resume.
- [x] 4.4 Add e2e or unit coverage that `Rename Tab` on a tmux control-mode tab changes only Brindle's displayed tab title and not the tmux window name.
- [x] 4.5 Run targeted unit tests for session/title logic and the relevant e2e case(s) under the established debug hooks.
