# Proposal

## Why

Brindle's automatic tab titles follow program titles, foreground processes, tmux window names and profile names, so users cannot pin a human label to a tab whose process/title keeps changing. Users need a `Rename Tab` command that keeps their chosen label until they change or clear it, while preserving current automatic title behavior for tabs without a manual title.

## What Changes

- Add a general `Rename Tab` command for the active tab, available through the command palette and bindable as `rename_tab`.
- Prompt with the active tab's current displayed title selected, trim submitted text, and set a manual title when the trimmed value is non-empty.
- Treat an empty trimmed submission as clearing the manual title, returning the tab to the existing automatic title rules.
- Make manual titles override OSC title changes, foreground-process polling, profile-title fallback and tmux window-name changes while set.
- Persist manual titles for restored local tabs through session restore.
- Keep tmux control-mode manual titles Brindle-local and runtime-only; users who need a tmux-owned durable name continue to use `tmux: Rename Window`.
- No breaking changes: tabs without a manual title keep the existing title behavior.

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `tabs` — add manual tab titles above the existing automatic title precedence.
- `command-palette` — expose the prompted `Rename Tab` command like other command-palette actions.
- `session-restore` — persist manual titles for restored local tabs only.

## Out of Scope

- Adding a default keybinding for `Rename Tab`; the action is bindable and appears in the command palette.
- Persisting Brindle-local manual titles for tmux control-mode tabs across session restore.
- Changing tmux window names or tmux state from `Rename Tab`.
