# Proposal

Outline only. Each item below still needs its own decision before any design or tasks.

**Status (2026-11-04).** The persistent terminal server that an exploration had added here as section 5 is now its own change, `terminal-server`. That change would absorb or supersede follow-ups 2–4 below, as noted on each, so their decisions should wait on its fate, or be made knowing it. Follow-up 1 is independent of it.

## Why

The quality review of `remote-connections` left four groups of work alone on purpose, so as not to change bootstrap or close behaviour right before testing on a real host. Each is described in section 7 of `openspec/changes/archive/2026-11-02-remote-connections/design.md`.

1. **Behaviour that may need to change.**
   - `Workspace::apply_config` keeps an open tab on the endpoint it started on, even if the profile's `remote` changes.
   - Remote tab titles cost one `QueryProcess` round trip per tab every second, hidden and idle tabs included.
2. **Lifecycle polling.**
   - The cancel watcher and the writer wake every 50 ms and 100 ms, about 30 idle wakeups a second per remote tab.
   - Child processes are awaited with 20 ms `try_wait` loops.
   - `terminal-server` would absorb this: the server replaces the per-tab lifecycle loops.
3. **Abstractions not yet earned.**
   - Five `profile.remote.is_some()` checks could become one `Endpoint` type.
   - `terminal-server` would supersede the `Endpoint` idea: a transport-agnostic server is the `Endpoint` type.
4. **Connection cost.**
   - Each SSH tab or split makes two connections: one to check the cached helper and one to launch it, about 0.3 s each on a fast link.
   - Containers copy the helper for every tab and delete it on exit.
   - `terminal-server` would absorb this: one connection per host instead of per tab, and a long-lived helper instead of per-tab container copies.

## What Changes

To be decided. Open questions for the full proposal:

- Is pinning a tab to its endpoint intended?
- Should the helper push process info only when it changes?
- Is the idle wakeup cost worth removing, given the tab-close races that were just fixed in this code?
- Does a second endpoint kind, or another check, justify an `Endpoint` type?
- Does connection latency on slower links justify a one-connection SSH launch, or a per-container helper cache, given each changes the bootstrap or cleanup guarantees?
- Which of follow-ups 2–4 are worth doing before `terminal-server`, if it goes ahead, and which should wait for it?

## Capabilities

To be decided. Likely `remote-connections`, and possibly `profiles`.
