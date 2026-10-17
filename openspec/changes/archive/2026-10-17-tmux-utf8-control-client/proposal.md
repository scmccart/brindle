# Proposal

## Why

When Brindle runs without a UTF-8 locale (no `LANG`, or `LANG=C`), tmux treats its control client as non-UTF-8. It then replaces control characters in command output with `_`, including the tabs Brindle uses to separate fields in its `-F` formats. Brindle can't parse the window list, so a tmux profile attaches but no tab ever appears.

`run-tests-in-ci`'s spike found this in a fresh `ubuntu:26.04` container. CI works around it with `LC_ALL=C.UTF-8`, but Brindle itself should work in any locale: launchers and service managers don't always set one.

## What Changes

- Control-mode clients start as `tmux -u [tmux_args] -C new-session …`. `-u` tells tmux the client handles UTF-8, so output keeps its tabs and non-ASCII text whatever the locale. With no `LANG` in a container, `tmux -u -C attach` keeps the tab in `list-windows -F "#{window_id}\t#{pane_id}"`, and without `-u` it prints `@0_%0`.
- The `-u` comes before `tmux_args`, so a user's own flags still apply.
- An e2e case runs a control profile with `LANG`, `LC_ALL` and `LC_CTYPE` removed from Brindle's environment and checks that the session's tabs appear.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `tmux-control-mode`: the attach command line gains `-u`, and attaching works regardless of the locale.

## Impact

- `src/tmux/mod.rs` (`command_line` and its unit tests).
- `tests/e2e/`: one new case, `Run::env_remove`, and `-u` for the suite's raw control client (`ControlClient`), so the `tmux-*` cases don't depend on the locale either.
- `scripts/with-xvfb` can keep `LC_ALL=C.UTF-8`, which still keeps runs consistent, but the suite no longer depends on it for tmux. Its comment and `CLAUDE.md` stop giving tmux as the reason.
