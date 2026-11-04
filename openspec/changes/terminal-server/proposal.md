# Proposal

**Status: exploration, not yet a settled proposal.** This change was split out of `remote-connections-followups` on 2026-11-04, where an exploration had added it as section 5 of a review-cleanup change. The user wants to reconsider it carefully before going further. `design.md` holds the exploration notes; the decisions that block any specs or tasks are listed there under "Unresolved decisions". The work is likely to become a series of changes (see "Phasing" in `design.md`), with this change keeping the first of them.

## Why

Today `brindle-remote` is one stdio child per tab, and it deliberately ends the session when the tab closes or the client goes quiet (`remote-connections`: "A vanished client ends the remote session"). Local terminals are PTYs owned by the Brindle process, so they end with it. Only tmux control-mode profiles persist, and only where tmux is available, which excludes native Windows.

The goal is a UX like herdr, where a server owns the terminals: they keep running when Brindle closes, and they are reattached when it opens again. Remote profiles are pre-registered, so Brindle can connect to them eagerly at launch and show their live sessions.

## What Changes

Decided in the exploration:

- **All terminals go through `brindle-remote`, local ones too, on Linux and on Windows.** It becomes the terminal server everywhere, with one instance per user per host. The GUI only renders. Persistence is the same wherever the session lives.
- **No tmux dependency.** This keeps Windows at parity and enables the eager display. tmux control mode stays as it is, as a separate persistence layer.
- **A virtual buffer in `brindle-remote`.** The server runs an emulator per session and uses it for the first draw when a client attaches.
- **Close behaviour is a per-profile option** for local and remote profiles alike, and it SHALL be respected in every case: closing a zone, tab or window, quitting, a crash, a dropped connection, and the shell exiting.
- **`brindle-remote` gets management commands** (`list`, `kill`, `status`, `stop`), so sessions can be managed on the host itself.
- **Session restore reattaches by session id,** and falls back to respawning in the recorded directory when the session is gone.
- **Container profiles may start their container** when Brindle connects to them eagerly.

### Relation to `remote-connections-followups`

This change would absorb or supersede three of that change's follow-ups:
- follow-up 2 (lifecycle polling): the server replaces the per-tab cancel watcher, writer and `try_wait` loops;
- follow-up 3 (an `Endpoint` type): a transport-agnostic server is the `Endpoint` type;
- follow-up 4 (connection cost): one connection per host instead of per tab, and a long-lived helper instead of per-tab container copies.

It would also replace the idle-limit requirement in `remote-connections`.

## Open questions

- The two decisions under "Unresolved decisions" in `design.md`: how an upgrade treats a running server that owns live sessions, and whether this change becomes the phased series. Both block phase 1.
- Which, if any, of follow-ups 2–4 in `remote-connections-followups` are worth doing first.

## Capabilities

Likely `remote-connections`, `terminal-emulation`, `tabs`, `zones`, `session-restore`, `command-line`, `configuration` and `profiles`, plus a new capability for the terminal server itself. `tmux-control-mode` is out of scope (see "Non-Goals" in `design.md`).

## Impact

- `remote-agent/` and `remote-protocol/`: the helper becomes a long-lived server with a versioned protocol, a vt buffer, management commands and a Windows build.
- `src/terminal/`: `Backend::Pty` goes away; the PTY side of `pty.rs`, `conpty.rs`, `cmdline.rs` and `process.rs` move into the helper.
- `src/workspace.rs`, session restore and settings: the close option, reattach by session id, and eager connection with an offline state.
- Tests: every e2e case needs an isolated server, and `tests/perf` baselines are taken before and after the extra IPC hop.
