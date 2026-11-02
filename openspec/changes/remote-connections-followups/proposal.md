# Proposal

Outline only. Each item below still needs its own decision, and possibly its own change, before any design or tasks.

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

## What Changes

To be decided. Open questions for the full proposal:

- Is pinning a tab to its endpoint intended, and should tmux tabs match it?
- Should the helper push process info only when it changes?
- Is the idle wakeup cost worth removing, given the tab-close races that were just fixed in this code?
- Does a second endpoint kind, or another check, justify an `Endpoint` type?
- Does connection latency on slower links justify a one-connection SSH launch, or a per-container helper cache, given each changes the bootstrap or cleanup guarantees?

## Capabilities

To be decided. Likely `remote-connections`, and possibly `tmux-control-mode` and `profiles`.
