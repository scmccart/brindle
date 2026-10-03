# Proposal

## Why

Brindle's only palette is the profile picker, and it sits on ctrl-shift-p, the key most users expect to open a command palette. Every other command is reachable only through its keybinding. In control mode that leaves tmux features that have no default key (layouts, renaming windows, arbitrary tmux commands) out of reach, because the tmux prefix doesn't work there.

## What Changes

- Add a **command palette** on ctrl-shift-p. It lists the commands available in the focused tab, each with a readable name and its current shortcut, filtered by fuzzy match. Commands that only apply to tmux control-mode tabs appear only when such a tab has focus.
- Let a command ask for text as a second step: after the user picks it, the palette switches to an input prompt, optionally pre-filled, and enter runs the command with that text.
- Separate the **new-tab palette** (today's profile picker) from the command palette. It keeps listing profiles and opening the `⌄` button, and moves to ctrl-shift-space. Profiles do not appear in the command palette. **BREAKING**: ctrl-shift-p no longer opens the profile list. The `open_profile_picker` action name keeps working in user keybindings.
- Add a bindable `open_command_palette` action.
- Add control-mode commands, available from the palette and as bindable actions:
  - new window;
  - layouts: even-horizontal, even-vertical, main-vertical, tiled, and next layout;
  - rotate panes;
  - swap the active pane with the previous or next pane;
  - break the active pane out into a new window (tab);
  - rename window (prompt, pre-filled with the current name);
  - run a tmux command (prompt, aimed at the active pane).
- Scope detach (ctrl-shift-d) to control-mode tabs, so it is neither offered nor bound on other tabs.

Out of scope: renaming or switching tmux sessions, which first need a decision on how a profile is matched to its session after the session's name changes. Also out of scope: killing a session, resizing panes by a typed amount, and confirming before closing a session's last window. Closing a control-mode tab still kills that tmux window.

## Capabilities

### New Capabilities
- `command-palette`: the searchable list of available commands, its shortcut display, the text-prompt step, and how it is opened, operated and dismissed.

### Modified Capabilities
- `profiles`: the profile picker becomes the new-tab palette and is opened by ctrl-shift-space (and the `⌄` button) instead of ctrl-shift-p.
- `tmux-control-mode`: the native command set gains the layout, rotate, swap, break-pane, rename-window and run-command commands, and detach applies only to control-mode tabs.

## Impact

- **Code:**
  - `src/picker.rs` becomes, or is joined by, a shared filterable list with a prompt mode.
  - `src/actions.rs` gains human-readable labels in `ACTIONS`, the new actions and the rebinding.
  - `src/workspace.rs`: palette state and opening, dispatch back to the tab.
  - `src/tmux_view.rs`: new handlers, and detach moves here.
  - `src/tmux/mod.rs`: new tmux commands.
- **Config:** new action names appear in `--list-actions`. Existing user keybindings keep working.
- **Docs:** the README keybinding tables.
- **Dependencies:** none.
