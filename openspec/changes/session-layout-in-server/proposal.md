# Proposal

## Why

Local, Windows and remote tabs all run in a terminal server now (#22, #51, #56), but where a session sat (its tab, the split tree around it, the tab's manual title) lives only in the quitting Brindle's `session.toml`. Any launch that doesn't have that file's record of a detached session opens it as a flat tab of its own, and its splits are lost:
- a launch with another state directory, or another machine's Brindle sweeping a remote host's sessions;
- the sessions left by a `--no-restore` run, whose own quit replaced the file the earlier tabs were recorded in.

The outline this change started from asked for a full layout store in the core (windows, tab order, per-client layouts, a server-dir file with respawn records). Re-derived against today's code, most of that is unneeded:
- Brindle has one window (`session::merge_windows`), so there are no windows to store.
- Restore already drops zones whose session is gone (`session::drop_lost_zones`), so nothing respawns after a reboot. A server-dir file would have nothing to respawn into.
- `session.toml` already restores the instance's own tabs, crash included, and stays the source for them.

What is worth doing is the narrow part: **a detached session carries its tab with it**, so whichever Brindle sweeps it rebuilds the split.

## What Changes

- **Brindle sends the tab's layout with each `DETACH` it sends when quitting:** the tab's split tree with session keys as leaves, the active zone, the manual title and the tab's place in the window. This is an additive optional `layout` field on `DETACH`. No new message type and no protocol version bump: an older core ignores the field.
- **The core keeps the layout in the session's meta,** which goes to its holder through `SET_META` (H1 v2), so the layout survives core restarts and upgrades. The core lists it in `SESSIONS` (an additive `layout` field), and clears it when the session is attached again. It never reads the layout: it is client-owned JSON with a size cap.
- **A launch's sweep rebuilds tabs from the layouts:** this machine's detached sessions at launch, and each remote endpoint's on its first listing. Sessions that share a recorded tab open as one tab with that tab's split tree, pruned to the sessions still there, its active zone focused and its manual title restored. Sessions without a usable layout open as flat tabs, as today.
- **`session.toml` keeps everything it keeps today** and still wins: the zones it records take their sessions first, and only what it doesn't know is grouped by the server layouts.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `terminal-server`: a detached session stores the layout its client sent with `DETACH`, across core restarts and upgrades, and lists it. It is cleared on attach.
- `session-restore`: the launch's sweep of detached sessions (this machine's and remote endpoints') regroups them into their recorded tabs and splits instead of one flat tab per session.

## Impact

- `remote-protocol`: `client::Detach` (new struct for `DETACH`, `layout?`), `SessionInfo.layout?`, `holder::SessionMeta.layout`; transcript tests.
- `remote-agent`: `core/clients.rs` (`client_detach` stores the layout, `client_attach` clears it), `core/manage.rs` (`LIST`); tests in `remote-agent/tests/core.rs`.
- `src/layout.rs` (new): the layout's JSON schema, recording it from a zone tree, and grouping listed sessions into tabs.
- `src/workspace.rs`, `src/zone_view.rs`, `src/terminal/mod.rs`, `src/remote/local/{session,connection}.rs`: the quit path passes each tab's layout to its terminals' `DETACH`es; `attach_detached`/`attach_swept` open grouped tabs.
- `src/reattach.rs`: `swept_tabs` returns grouped tabs.
- e2e: a `local-server-*` case for splits coming back from the server layout.
