# Proposal

## Why

The tmux geometry, title and style changes were driven by Claude Code's `teammateMode: "tmux"`, but no test exercises that mode as a whole. Each tmux command Claude Code issues was tested separately. Claude Code's `TmuxBackend`, read from its binary (2.1.288), does the following when it creates teammate panes:
- `split-window -d -t <leader> -h|-v -P -F '#{pane_id}' -- <command>`;
- `set-option -p window-style 'bg=default,fg=<color>'`;
- `set-option -p pane-border-style` and `pane-active-border-style` with `fg=<color>`;
- `select-pane -T <name>`, and `pane-border-format '#[fg=<color>,bold] #{pane_title} #[default]'`;
- `set-option -w pane-border-status top`;
- `kill-pane` when a teammate ends.

The order and timing of these steps, several panes at once, and teammates ending have never been tested together.

## What Changes

- Add an end-to-end case, `claude-code-teammates`, that replays that sequence against a throwaway server while Brindle is attached. It creates three teammates with different colors, ends one, and checks:
  - geometry against `capture-pane` for every pane;
  - each teammate's title and tint in a window capture;
  - that the leader pane's screen is undisturbed.
- Add a short manual checklist in `CLAUDE.md` for trying the real thing: Claude Code with agent teams in a Brindle control-mode tab. It costs API usage and needs an account, so it can't be automated here.
- Record the Claude Code version the sequence was taken from, so it can be re-checked when Claude Code changes its backend.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. This tests behaviour already specified in `tmux-control-mode` (`skip_specs: true`).

## Impact

- `tests/e2e/cases/`: a new module with the replayed sequence.
- `CLAUDE.md`: the manual checklist.
- If the replay shows a bug, the fix goes in its own change.
