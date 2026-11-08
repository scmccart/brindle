# Proposal

Phase 2 of the terminal-server series. Phase 1 (`terminal-server`, PR #22, archived as `2026-11-07-terminal-server`) built the server and moved local Linux tabs onto it with the `kill` close policy.

## Why

Phase 1 runs every local Linux terminal in a per-user server, but quitting Brindle still ends its programs, as before. The point of the series is that terminals outlive Brindle and come back when it opens again. The server already supports that: `detach` sessions, attach with a full screen replay, reconnect by client token, and checkpoints that survive core restarts. Brindle just never asks for them.

Keeping terminals across restarts only makes sense when there is one Brindle to come back to. With several windows and several processes, "the terminals Brindle had" has no single answer: which window gets which session, and which process restores. So Brindle becomes one window in one process.

## What Changes

- **Closing ends, quitting keeps.** Closing a zone or tab ends its program, as today. Quitting Brindle, closing its window, a crash, SIGTERM or the desktop stopping Brindle leave every local Linux program running detached, and the next launch brings them back. `-e` command tabs end with Brindle. There is no setting and no prompt.
- **BREAKING: one window.** Brindle has a single window. New Window (ctrl-shift-n) and every "new window" path are removed; closing the window is quitting. tmux control-mode sessions open as tabs of that window, so "open in another window" and "Go to <session>" are gone. A state file with several windows restores into one.
- **BREAKING: one instance.** The Brindle process that holds the session-state lock is the instance. A later launch hands its request (`-p`, `-d`, `-e`, `--attach`, or nothing) to the instance, which raises its window and opens the requested tab, and exits. A launch with debug options (`--send`, `--action`, `--dump-screen-after`, latency) that finds an instance runs on its own without restoring, as a second process does today.
- **BREAKING: session restore is always on.** `restore_session` is removed (ignored with a warning if still set); `--no-restore` skips one launch. The instance always restores, and a launch's `-p`, `-d`, `-e` or `--attach` opens its tab on top of the restored ones.
- **Restored zones get their own sessions back** (matched by session id and a random session key). A local zone whose session is gone is not restored; a tab or window left with nothing is not either. Detached sessions no zone claims open as tabs of the window. Moving the layout itself into the server is a follow-up, `session-layout-in-server`.
- **A sessions palette** (`attach_session`) lists detached sessions by title, command, directory and age, attaches one into a new tab, or kills it.
- **`brindle --attach <id>`** opens a tab attached to a session.
- **Server fixes:** `DETACH` switches a `kill` session to `detach` and clears its owner; `ATTACH` makes the client the owner. H1 version 2 adds `SET_META`, so owner and policy survive a core restart. `SESSIONS` reports each session's `key`, `title` and `started`.
- **Server roles start in their own systemd scope** (`systemd-run --user --scope`), so ending the desktop's app scope for Brindle doesn't end its sessions.

Not in this change: Windows terminals in the server (phase 3), remote hosts (phase 4) and tmux profiles' persistence (tmux already persists). The one-window and one-instance rules apply on Windows too.

## Capabilities

### New Capabilities
- None. The server behaviour is in `terminal-server`.

### Modified Capabilities
- `tabs`: one window; quitting detaches; closing the window is quitting.
- `zones`: closing a zone ends its program.
- `profiles`: the default profile and the picker without other windows.
- `session-restore`: always on, one window, session records, dropped lost zones, reattaching detached sessions, and the instance.
- `configuration`: `restore_session` removed.
- `tmux-control-mode`: no other windows to go to.
- `terminal-server`: key, title and start time in `SESSIONS`; `DETACH` and `ATTACH` ownership rules; H1 v2 `SET_META`; systemd scopes.
- `command-palette`: the sessions palette.
- `command-line`: `--attach <id>`, a launch hands its request to the running instance, and every instance launch restores.

## Impact

- `src/main.rs`: the instance (lock, socket, forwarding), launch handling, quit.
- `src/instance.rs` (new): the request socket (a Unix socket beside the lock; a named pipe on Windows) and the request format.
- `src/workspace.rs`, `zone_view.rs`, `actions.rs`, `picker.rs`: close and quit, removing new-window paths and cross-window session lookups.
- `src/session.rs`: one window, session records, dropping lost zones.
- `src/remote/local/`: attach, detach at quit, close on tab close.
- `src/terminal/`: a `Server` terminal made from an attached session.
- `remote-protocol`: `SessionInfo` fields, the session key in `SPAWNED`, H1 v2 `SET_META`.
- `remote-agent`: ownership rules, `SET_META`, holder storage of updated meta, systemd scopes.
- Tests: server integration tests for ownership and `SET_META`; e2e cases for close and quit, reattach after a quit and after a crash, dropped zones, the instance handing over a launch, the palette and `--attach`; the multi-window cases go.
