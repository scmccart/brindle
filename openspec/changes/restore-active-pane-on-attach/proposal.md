# Proposal

## Why

When Brindle attaches to an existing tmux session, it doesn't learn which pane is active in each window. It assumes the first pane in the layout is active, focuses that pane's view, and the focus handler then runs `select-pane` on it. So attaching quietly moves tmux's active pane: the user lands in a different pane than the one they left, and other clients see the change too. Commands aimed at the active pane (split, zoom, swap, break pane, Run Command) then act on that wrong pane. This showed up while verifying `add-command-palette`: swap and break-pane runs on a fresh attach acted on the layout's first pane.

## What Changes

- Read each window's active pane from tmux whenever Brindle fetches the window list (on attach, on new windows and on session changes), instead of guessing the layout's first pane. tmux 3.6 confirms that `list-windows -F '#{pane_id}'` gives each window's active pane.
- Attaching SHALL NOT change tmux's active pane in any window. Keyboard focus starts in the pane tmux reports as active.
- Fall back to the layout's first pane only while tmux hasn't reported an active pane, as today, but without selecting it in tmux.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `tmux-control-mode`: attaching preserves each window's active pane and focuses it. This adds a requirement alongside "Attach to a session" and "Input and focus".

## Impact

- **Code:** `src/tmux/mod.rs` gains the pane field in the `list_windows` format and in `on_window_list`. `src/tmux_view.rs` (`sync`, and the focus-in handler that calls `select_pane`) must not echo the initial focus back to tmux as a selection.
- **Tests:** a unit test for parsing the extended window-list line, and an end-to-end check on a throwaway server: set the active pane to one that isn't first, attach with `--dump-screen-after`, and confirm with `list-panes` that the active pane is unchanged.
- **Dependencies:** none.
