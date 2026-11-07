# Proposal

**Status: proposal stage.** This is phase 2 of the terminal-server series. Phase 1 (`terminal-server`, PR #22, archived as `2026-11-07-terminal-server`) built the server and moved local Linux tabs onto it with the `kill` close policy. This change needs phase 1 merged first. It has no specs or tasks yet. Settle the open questions in `design.md` before writing them.

## Why

Phase 1 runs every local Linux terminal in a per-user server, but quitting Brindle still ends its programs, as before. The point of the series is that terminals outlive Brindle and come back when it opens again. The server already supports that: `detach` sessions, attach with a full screen replay, reconnect by client token, and checkpoints that survive core restarts. Brindle just never asks for them.

## What Changes

- **A per-profile close option,** `on_close = "kill" | "detach" | "ask"` (name to be settled). It applies uniformly to:
  - closing a zone, tab or window;
  - quitting Brindle, and a Brindle crash;
  - a dropped connection;
  - the shell exiting.
- **Restore reattaches by session id.** Session restore records each zone's session id beside its directory and target. On launch it attaches to the session, with a quiet replay. When the session is gone, it falls back to today's respawn in the recorded directory.
- **A "sessions on this host" palette** lists detached sessions, from the core's `LIST`. It can attach one into a new tab or zone, or kill it.
- **Brindle attaches to sessions it didn't spawn,** with the replay's bells and notifications suppressed, as reconnects already do.
- **The two gaps phase 1 left** for this change are settled (see `design.md`).

Not in this change: Windows (phase 3) and remote hosts (phase 4).

## Capabilities

### New Capabilities
- None expected. The server behaviour is already in `terminal-server`.

### Modified Capabilities
- `profiles` or `configuration`: the close option.
- `session-restore`: reattach by session id, with the respawn fallback.
- `tabs` and `zones`: close behaviour per policy, and `ask`.
- `terminal-server`: ownership updates and the detached-`kill` rule.
- `command-palette`: the sessions palette.
- `command-line`: any flag for attaching a session, if one is wanted.

## Impact

- `src/workspace.rs`, `zone_view.rs`, `session.rs` and settings: close handling, `ask` prompts, restore records.
- `src/remote/local/`: attach, and detach on close.
- `remote-protocol` and `remote-agent`: possibly an H1 message to update a session's `meta` (its owner and policy), which bumps H1's version.
- Tests: e2e cases for each close event under each policy, and restore reattaching after a quit and after a crash.
