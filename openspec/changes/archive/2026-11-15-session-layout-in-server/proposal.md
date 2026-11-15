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

- **Brindle keeps each tab's layout with its sessions while it runs, and sends it once more as it quits:** the tab's split tree with session keys as leaves, the active zone, the manual title and the tab's place in the window.
  - While running, a change reaches the server within about a second through a new `SET_LAYOUT` message (client protocol 3), sent only for sessions whose layout changed. A Brindle that crashes, or whose last `DETACH` never gets written, still leaves its splits behind.
  - On quit, the layout goes on each `DETACH` as an additive optional field, which also reaches a core that only speaks protocols 1–2. Such a core gets layouts only on quit.
- **The core keeps the layout in the session's meta,** which goes to its holder through `SET_META` (H1 v2), so the layout survives core restarts and upgrades. The core lists it in `SESSIONS` (an additive `layout` field), and clears it when the session is attached again. It never reads the layout: it is client-owned JSON with a size cap.
- **A launch's sweep rebuilds tabs from the layouts:** this machine's detached sessions at launch, and each remote endpoint's on its first listing. Sessions that share a recorded tab open as one tab with that tab's split tree, pruned to the sessions still there, its active zone focused and its manual title restored. Sessions without a usable layout open as flat tabs, as today.
- **`session.toml` keeps everything it keeps today** and still wins: the zones it records take their sessions first, and only what it doesn't know is grouped by the server layouts.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `terminal-server`: a session stores the layout its client last sent (`SET_LAYOUT` while attached, protocol 3, or with `DETACH`), across core restarts and upgrades, and lists it while detached. It is cleared on attach. "Protocol versions" adds protocol 3.
- `session-restore`: the launch's sweep of detached sessions (this machine's and remote endpoints') regroups them into their recorded tabs and splits instead of one flat tab per session.

## Impact

- `remote-protocol`: client protocol 3 (`PROTOCOLS` 1–3) with `SET_LAYOUT`; `client::Detach` (new struct for `DETACH`, `layout?`), `SessionInfo.layout?`, `holder::SessionMeta.layout`; transcript tests.
- `remote-agent`: `core/clients.rs` (the client's protocol, `SET_LAYOUT` dispatch), `core/sessions.rs` (`client_detach` and `client_set_layout` store the layout, `client_attach` clears it), `core/manage.rs` (`LIST`); tests in `remote-agent/tests/core.rs`.
- `src/layout.rs` (new): the layout's JSON schema, recording it from a zone tree, and grouping listed sessions into tabs.
- `src/workspace.rs`, `src/zone_view.rs`, `src/terminal/mod.rs`, `src/remote/local/{session,connection}.rs`: the quit path passes each tab's layout to its terminals' `DETACH`es, and `main.rs`'s once-a-second session recorder sends changed layouts; `attach_detached`/`attach_swept` open grouped tabs.
- `src/reattach.rs`: `swept_tabs` returns grouped tabs.
- e2e: a `local-server-*` case for splits coming back from the server layout.
