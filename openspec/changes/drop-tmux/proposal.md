# Proposal

## Why

Brindle's tmux support (control mode, classic profiles, remote and WSL attach) is about 5k lines of source, a 37-requirement spec and a share of every e2e run, and it exists mostly to give terminals persistence, splits and tabs that survive a quit. The terminal server now provides persistence for local tabs, zones provide splits, and `terminal-server-remote` already leaves remote tmux "outside the server" as the odd one out. Keeping a second, parallel model of tabs, panes, restore and remote attach costs every change to `workspace.rs`, session restore and the chrome. Dropping it now, before phases 3 and 4 of the terminal server, avoids carrying it through them.

## What Changes

- **BREAKING** Remove tmux control mode: the `tmux/` and `tmux_view.rs` modules, `TabContent::Tmux`, the `Tmux*` actions and their palette entries and keybindings, the tmux tab and zone menu variants, the "session shown by another Brindle", "session ended" and attach-failure flows, tmux pane styling, geometry, title lines, palette and colour queries, and the subscription/polling state machinery with `BRINDLE_TMUX_STATE`.
- **BREAKING** Remove classic tmux profiles (`tmux = "plain"`) and the whole profile-level tmux model: `Profile.tmux`, `tmux_session` and `tmux_args`, `TmuxMode`, the tmux fields of the settings dialog's profile editor, the `tmux` and `tmux (classic)` default profiles and the commented WSL example. A user who wants tmux sets `command = "tmux"` (and `args`) on an ordinary profile; it runs as any program does.
- **BREAKING** Remove the `tmux_session_end` setting which is the General section's only control, so the section goes and the dialog has Profiles and Theme.
- **BREAKING** Remove remote and WSL tmux control mode: `remote::tmux_command`, `detach_command`, the remote-tmux `remote-connections` requirement, and the WSL control profile with its `wsl` program detection (`program_stem` stays, used elsewhere).
- **BREAKING** Remove `kind = "tmux"` records from `session.toml`: an old file's tmux entries are skipped on restore.
- Old configs keep loading: `tmux`, `tmux_session` and `tmux_args` in a profile and `tmux_session_end` at the top level are accepted, ignored, and reported with one `log::warn!` each, as `restore_session` is today. The settings dialog and live reload keep rewriting a file without touching those lines.
- Remove tmux-only support code: `Terminal::Remote` pane wiring and `PaletteHandler` (the `Remote` backend stays for message tabs), the tmux parts of the `Notify`/`Bell`/status re-emission, tmux-specific e2e cases (`tests/e2e/tmux.rs`, `cases/tmux.rs`, `tests/common/tmux.rs` and the tmux halves of other cases), the perf `latency-tmux` and `throughput-tmux` cases, the e2e matrix's tmux versions and source build in `ci.yml`, and tmux mentions in README, `CLAUDE.md`, the default config and the package descriptions.
- Delete the `test-claude-code-teammate-mode` change; it only exists to test control mode.
- Keep everything that is not tmux-specific: OSC 7501 program status, OSC 133 marks, OSC 8 links, OSC 52, focus reporting and colour queries all keep working for programs in ordinary tabs, including a tmux running inside one.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `tmux-control-mode`: every requirement removed; the capability goes away.
- `profiles`: drop the tmux mode from the profile definition, the classic-profile requirement and the session wording in the picker, menu and default-profile requirements; `Remote target` loses its tmux settings.
- `remote-connections`: drop the remote tmux control mode requirement.
- `session-restore`: drop tmux records, the classic tmux scenario and the control-mode reattach requirement; fallbacks no longer mention tmux; old `kind = "tmux"` records are skipped.
- `tabs`: drop tmux from titles, manual rename, bell and status indicators, the single-window and new-tab rules and the tab menu.
- `settings-dialog`: drop the tmux profile fields, the tmux preview wording and the General section's `tmux_session_end` control.
- `command-palette`: drop the tmux commands, the control-mode listing and the tmux prompt scenarios.
- `command-line`: drop the tmux latency benchmark and the WSL control scenario; relabel examples that name a tmux profile.
- `configuration`: remove `tmux_session_end` and say that the old tmux keys are ignored with a warning; live reload no longer mentions tmux panes.
- `terminal-emulation`: the responsiveness requirement no longer lists tmux panes.
- `program-status`: report recognition, records and feature detection no longer cover tmux panes.
- `scrollback-search`: search scope and purpose no longer cover control-mode panes.
- `links`: remove the control-mode pane requirement and purpose wording.
- `shell-integration`: remove the tmux panes requirement and purpose wording.

## Impact

- **Code:** delete `src/tmux/`, `src/tmux_view.rs`, `tests/e2e/tmux.rs`, `tests/e2e/cases/tmux.rs`, `tests/common/tmux.rs`; large edits to `workspace.rs` (about 190 tmux mentions), `session.rs`, `actions.rs`, `picker.rs`, `config.rs`, `config_edit.rs`, `settings_dialog.rs`, `settings_dialog/profiles.rs`, `zone_view.rs`, `terminal/mod.rs`, `remote/mod.rs`, `main.rs`, `win.rs`; smaller edits across e2e cases, perf cases and `ui/examples/gallery.rs`. Dependencies stay (`shlex` is used elsewhere); `Cargo.lock` is not regenerated.
- **Behaviour:** an upgrading user's tmux profiles become ordinary profiles that no longer run tmux (the `tmux` fields are ignored), so a profile `tmux = "plain"` opens the default shell until they set `command = "tmux"`. Saved control-mode tabs are dropped on the first launch. Release notes carry the migration; the commit and PR title use `!`.
- **CI and packaging:** the e2e job drops the tmux matrix, source build and `old-tmux` input; the WinGet description and `Cargo.toml` description lose "first-class tmux support"; `tests/fixtures/remote/Dockerfile` stops installing tmux.
- **In-flight changes:** `add-blocks` loses its control-mode panes and classic-tmux scope and task 7.1's tmux case; `port-to-macos` loses task 7.1, the `TmuxWindow` keybinding rows and Homebrew-tmux PATH wording; `session-layout-in-server` loses its session-less tmux placeholders question; `remote-connections-followups` loses its tmux questions; `terminal-server-remote` drops the "remote tmux stays outside the server" note. These are edited when this change lands, not here.
