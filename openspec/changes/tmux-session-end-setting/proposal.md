# Proposal

## Why

Typing `exit` in the last shell of a control-mode tmux session leaves the user in a bash tab they never asked for. When a session's tabs were all the window had, Brindle opens a fallback shell so the window survives. That makes sense after a detach, but it isn't what most people expect when the session itself has ended, and closing the last tmux tab with ctrl-shift-w does the same thing. Local tabs already close the window when their shell exits. tmux tabs should be able to behave the same way, and users should choose what happens.

## What Changes

- New config key `tmux_session_end`, one of:
  - `close_window` (default): the window closes, like a local tab whose shell exits.
  - `new_tab`: today's behaviour. A tab with the default profile opens, or the first non-tmux profile if the default is a control-mode profile.
  - `ask`: a palette offers "Close window" and "New tab: <profile>" for every profile.
- The setting applies whenever a control-mode session goes away and its tabs were the window's only tabs. That covers the last shell exiting, closing the session's last tab, the session being killed, tmux exiting, **and detaching with ctrl-shift-d**. When the window has other tabs, the session's tabs just close, as today.
- **BREAKING (default behaviour):** with the default `close_window`, ctrl-shift-d, `exit` or ctrl-shift-w on a window that shows only one tmux session closes the window, and Brindle quits if it was the last window. `new_tab` restores the old behaviour. The tmux session is still left running after a detach.
- While `ask` is open, the ended session's tabs stay on screen with their panes ended, so the window is never empty. Escape closes the window.
- When Brindle itself is quitting, a session that leaves does not close the window or open a tab, so session restore still records that window.
- The settings dialog gets a **General** section, first in the list, holding this setting as a choice field. The sections are now General, Profiles, Theme, in that order, and the dialog opens on General. Changing the choice writes the config at once and applies it without waiting for the file watcher.
- Out of scope:
  - attach failures, which keep their explanatory message tab;
  - local tabs, which are unchanged;
  - moving other global settings into General;
  - tmux 3.6's crash when a session with subscriptions ends from inside (already documented).

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `tmux-control-mode`: "Detach keeps the session" becomes a requirement about what happens when a session's tabs were the window's only tabs. It is governed by `tmux_session_end` and covers detach, the session ending, and quitting.
- `settings-dialog`:
  - the sections become General, Profiles, Theme;
  - the dialog opens on General;
  - a new General section requirement;
  - keyboard operation covers the General choice field.
- `configuration`: "Behaviour settings" lists `tmux_session_end`.

## Impact

- **Code:**
  - `config.rs`: `TmuxSessionEnd` enum and `Config` field.
  - `assets/default-config.toml`: a commented `tmux_session_end` and a corrected comment on the tmux profile.
  - `config_edit.rs`: setting a top-level key.
  - `workspace.rs`: the `Detached` handler, plus a new `PaletteKind` for the prompt with its confirm and dismiss handling.
  - `tmux/mod.rs`: a quitting flag that suppresses the detach event.
  - `settings_dialog.rs` (and the shared choice field from `settings_dialog/profiles.rs`): the General section, section order, opening section and dump line.
- **Docs:** README rows for ctrl-shift-d and the settings dialog.
- **Tests:**
  - unit tests for config parsing, the config edit and the dialog model;
  - existing e2e settings cases updated for the new section order;
  - new e2e cases for each `tmux_session_end` value, and for changing it in the dialog.
