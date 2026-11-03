# Proposal

Outline only. Each item below still needs its own decision, and possibly its own change, before any design or tasks.

**Status: on ice (2026-11-03).** An exploration widened this change from review cleanups into a persistent terminal server: section 5 below, with the detail in `design.md`. The user wants to reconsider it carefully before going further. It is likely to be split out into its own change, or a series of changes, before any specs or tasks are written. Sections 1–4 are the original follow-ups. Several of them would be absorbed or superseded by section 5, as noted there.

## Why

The quality review of `remote-connections` left four groups of work alone on purpose, so as not to change bootstrap or close behaviour right before testing on a real host. Each is described in section 7 of `openspec/changes/archive/2026-11-02-remote-connections/design.md`.

1. **Behaviour that may need to change.**
   - `Workspace::apply_config` keeps an open tab on the endpoint it started on, even if the profile's `remote` changes, while tmux tabs take the new profile.
   - Remote tab titles cost one `QueryProcess` round trip per tab every second, hidden and idle tabs included.
2. **Lifecycle polling.**
   - The cancel watcher and the writer wake every 50 ms and 100 ms, about 30 idle wakeups a second per remote tab.
   - Child processes are awaited with 20 ms `try_wait` loops.
3. **Abstractions not yet earned.**
   - Five `profile.remote.is_some()` checks, the WSL check and the `CONTROL_CLIENT_MARKER` skip could become one `Endpoint` type.
   - Each tmux control line and output chunk goes through `block_on(send)`.
4. **Connection cost.**
   - Each SSH tab or split makes two connections: one to check the cached helper and one to launch it, about 0.3 s each on a fast link.
   - Containers copy the helper for every tab and delete it on exit.

### 5. Persistent sessions through a terminal server

Today `brindle-remote` is one stdio child per tab, and it deliberately ends the session when the tab closes or the client goes quiet (`remote-connections`: "A vanished client ends the remote session"). The goal is a UX like herdr, where a server owns the terminals: they keep running when Brindle closes, and they are reattached when it opens again. Remote profiles are pre-registered, so Brindle can connect to them eagerly at launch and show their live sessions.

Decided in the exploration:
- **All terminals go through `brindle-remote`, local ones too, on Linux and on Windows.** It becomes the terminal server everywhere, with one instance per user per host. The GUI only renders. Persistence is the same wherever the session lives.
- **No tmux dependency.** This keeps Windows at parity and enables the eager display. tmux control mode stays as it is, as a separate persistence layer.
- **A virtual buffer in `brindle-remote`.** The server runs an emulator per session and uses it for the first draw when a client attaches.
- **Close behaviour is a per-profile option** for local and remote profiles alike, and it SHALL be respected in every case: closing a zone, tab or window, quitting, a crash, a dropped connection, and the shell exiting.
- **`brindle-remote` gets management commands** (`list`, `kill`, `status`, `stop`), so sessions can be managed on the host itself.
- **Session restore reattaches by session id,** and falls back to respawning in the recorded directory when the session is gone.
- **Container profiles may start their container** when Brindle connects to them eagerly.

This would absorb or supersede follow-ups 2 (the server replaces the per-tab lifecycle loops), 3 (a transport-agnostic server is the `Endpoint` type) and 4 (one connection per host instead of per tab, and a long-lived helper instead of per-tab container copies). It would also replace the idle-limit requirement in `remote-connections`.

## What Changes

To be decided. Open questions for the full proposal:

- Is pinning a tab to its endpoint intended, and should tmux tabs match it?
- Should the helper push process info only when it changes?
- Is the idle wakeup cost worth removing, given the tab-close races that were just fixed in this code?
- Does a second endpoint kind, or another check, justify an `Endpoint` type?
- Does connection latency on slower links justify a one-connection SSH launch, or a per-container helper cache, given each changes the bootstrap or cleanup guarantees?
- Section 5: should the terminal server become its own change, or a series of changes (see the phasing in `design.md`), with this change keeping only the follow-ups the server would not absorb?
- Section 5: how should an upgrade treat a running server that owns live sessions? This blocks the first phase (see `design.md`).

## Capabilities

To be decided. Likely `remote-connections`, and possibly `tmux-control-mode` and `profiles`. Section 5 would also touch `terminal-emulation`, `tabs`, `zones`, `session-restore`, `command-line` and `configuration`, and likely needs a new capability for the terminal server itself.
