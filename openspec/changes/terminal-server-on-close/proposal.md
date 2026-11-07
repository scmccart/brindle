# Proposal

Phase 2 of the terminal-server series. Phase 1 (`terminal-server`, PR #22, archived as `2026-11-07-terminal-server`) built the server and moved local Linux tabs onto it with the `kill` close policy.

## Why

Phase 1 runs every local Linux terminal in a per-user server, but quitting Brindle still ends its programs, as before. The point of the series is that terminals outlive Brindle and come back when it opens again. The server already supports that: `detach` sessions, attach with a full screen replay, reconnect by client token, and checkpoints that survive core restarts. Brindle just never asks for them.

## What Changes

- **BREAKING: closing no longer ends local programs.** Closing a zone, tab or window, quitting, a crash or SIGTERM leave the program running detached; the shell exiting still closes its tab. A per-profile `on_close = "detach" | "ask"` (default `detach`) can make Brindle ask first, once per close and once for all sessions on quit, with "end" as one answer. There is no per-profile kill mode. `-e` command tabs, which are never restored, still end with their window.
- **BREAKING: session restore is always on.** `restore_session` is removed (ignored with a warning if still set); `--no-restore` skips one launch.
- **Every launch reattaches every detached session,** with a quiet replay. Restored zones get their own sessions back (matched by session id and a random session key), and the rest open as new tabs. Only the process holding the restore lock does this. Moving the layout itself into the server is a follow-up, `session-layout-in-server`.
- **A sessions palette** (`attach_session`) lists detached sessions by title, command, directory and age, attaches one into a new tab, or kills it.
- **`brindle --attach <id>`** opens a tab attached to a session.
- **Server fixes:** `DETACH` switches a `kill` session to `detach` and clears its owner; `ATTACH` makes the client the owner. H1 version 2 adds `SET_META`, so owner and policy survive a core restart. `SESSIONS` reports each session's `key`, `title` and `started`.
- **Server roles start in their own systemd scope** (`systemd-run --user --scope`), so ending the desktop's app scope for Brindle doesn't end its sessions.

Not in this change: Windows (phase 3), remote hosts (phase 4) and tmux profiles.

## Capabilities

### New Capabilities
- None. The server behaviour is in `terminal-server`.

### Modified Capabilities
- `profiles`: the `on_close` option.
- `settings-dialog`: `on_close` on the profile page.
- `tabs` and `zones`: closing per policy, and the `ask` prompts.
- `session-restore`: always on, session records, and reattaching detached sessions at launch.
- `configuration`: `restore_session` removed.
- `tmux-control-mode`: a scenario that set `restore_session`.
- `terminal-server`: key, title and start time in `SESSIONS`; `DETACH` and `ATTACH` ownership rules; H1 v2 `SET_META`; systemd scopes.
- `command-palette`: the sessions palette.
- `command-line`: `--attach <id>`.

## Impact

- `src/config.rs`, `config_edit.rs`, `settings_dialog.rs`: the option.
- `src/workspace.rs`, `zone_view.rs`, `main.rs`: close handling, the `ask` prompts, quit, launch-time reattach.
- `src/session.rs`: session records in the restore file.
- `src/remote/local/`: attach, detach on close, ending sessions on quit.
- `src/terminal/`: a `Server` terminal made from an attached session.
- `remote-protocol`: `SessionInfo` fields, the session key in `SPAWNED`, H1 v2 `SET_META`.
- `remote-agent`: ownership rules, `SET_META`, holder storage of updated meta, systemd scopes.
- Tests: server integration tests for ownership and `SET_META`; e2e cases for each close event and policy, reattach after a quit and after a crash, the palette and `--attach`.
