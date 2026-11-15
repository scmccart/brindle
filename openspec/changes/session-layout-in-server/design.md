# Design

## Context

See proposal.md for why, and for how the original outline was cut down. Facts from the code that shape the design:

- **Session state the server already keeps per session.** `holder::SessionMeta` (`remote-protocol/src/holder.rs`) is an opaque JSON blob in the holder, owned by the core. It has `owner`, `policy`, `key`, `profile`, `started` and so on. The core re-sends it with `SET_META` (H1 v2) on every change (`Core::store_meta`), and a new core reads it back when it adopts the holder (`Session::adopted`). Fields are additive (`SessionMeta::from_value` reads only the fields it knows). A core upgrade or crash therefore keeps anything put in the meta, as long as it is the holder's (H1 v2). Adopted sessions keep their ids.
- **Quitting.** `Workspace::quit` → `ZoneTreeView::quit_all` → `Terminal::quit` → `Session::detach` → `Running::detach` → `ToCore::Detach(SessionRef)` for every `Closing::Keeps` terminal, local or remote. `DETACH` clears the owner (`client_detach`), so the next launch may sweep the session at once (`reattach::sweepable`).
- **Restoring.** `main` lists this machine's sessions into `reattach::Detached`. Zones from `session.toml` take theirs by id and key (`Detached::take`), and `Detached::take_rest` gives the rest, opened by `Workspace::attach_detached` as flat tabs at the end, in start order. A remote endpoint's first listing goes through `reattach::swept_tabs` → `Workspace::attach_swept`, also flat.
- **One window.** `session::merge_windows` folds the file's windows into one, and nothing opens a second one. Window size and maximized stay in `session.toml`.
- **No respawn.** A zone whose session is gone is dropped (`session::drop_lost_zones`). After a reboot nothing respawns: the file's role is matching live sessions, not recreating dead ones.
- **Zone tree.** `ZoneTree::Split { axis, first, second }` has no ratio: splits are always halves. A layout has no ratio to record.

## Goals / Non-Goals

**Goals:**
- A detached session carries the tab it sat in, so any launch that sweeps it rebuilds the tab's splits, active zone and manual title. This covers a launch with another state directory, a `--no-restore` run's leftovers, and another machine sweeping a remote host.
- No new message type, no protocol version bump. Older cores, clients and holders interoperate unchanged.

**Non-Goals:**
- Replacing `session.toml`. It still restores the instance's own tabs, crash included, and window geometry.
- Recording layouts while attached. A crashed client's sessions sweep flat into *other* Brindles, as today. Its own next launch still restores them from `session.toml`.
- Windows (plural), tab order across different quits, ratios (none exist), respawning after a reboot.
- Attaching a whole recorded tab from the sessions palette. The palette still attaches one session.

## Decisions

### 1. Where the layout lives: the session's holder meta, nowhere else

The layout is a new `SessionMeta.layout: Option<serde_json::Value>` field. It reaches the holder through the existing `SET_META` that `client_detach` already sends for the owner and policy change. So it survives core restarts and upgrades by the same path as the owner, at no extra cost, and it ends with its session.

*Rejected:*
- **Core memory only:** a core upgrade would drop it, and upgrades step down on every newer Brindle.
- **A file in the server directory:** it would survive a reboot, but then it describes sessions that no longer exist. Respawning them would be a new feature (restore already drops lost zones), and it would need its own locking and pruning, and a Windows path.
- **Both:** the costs of both, for a reboot case nothing uses.

With an H1 v1 holder, `store_meta` already logs and skips: the layout then lasts only in that core, like the owner.

### 2. How the client sets it: an additive field on `DETACH`

`DETACH` gets its own struct, `client::Detach { session, layout? }`. Today it shares `SessionRef` with `CLOSE`. `layout` is omitted when `None`, so a `DETACH` without one encodes byte for byte as before. An older core ignores the unknown field (JSON fields are additive), and an older client never sends it. `SessionInfo` gets `layout?` the same way.

*Rejected:*
- **A new `SET_LAYOUT` message,** sent on every layout change while attached. It needs protocol 3, the client's negotiation, and a stream of updates for drags and splits, and it buys only the crash case. `session.toml` already covers that case for the instance's own relaunch.
- **A field on `SPAWN`:** a layout changes after the spawn.

`DETACH` is the moment the layout starts to matter: a detached session is what a sweep picks up. While a session is attached, its client's window *is* its layout.

### 3. Granularity: each session carries its whole tab, sessions as leaves by key

Every session of a tab gets the same blob. It is Brindle's, and the core never parses it:

```json
{"v":1,"quit":"<per-quit id>","index":2,"title":"logs",
 "zones":{"split":"horizontal","first":{"key":"9f…","active":true},"second":{"key":"3c…"}}}
```

- `zones` is a leaf (`key`, `active`) or a split (`split` axis, `first`, `second`); a single-zone tab is a leaf.
- `quit` is a random id for one quit of one Brindle: the pid and the time in nanoseconds, hex. Together with `index` it identifies the tab.
- `title` is the manual title only.
- `v` is the format version. A layout with another `v`, or one that doesn't parse, counts as none.
- Leaves name sessions by `key`, not id. Keys are unique across server lifetimes, and the layout is only read against the same server's listing.

The blob holds no profile and no remote target. The tab's profile comes from its active zone's session by the existing rules (`reattach::session_profile`), and its host from the listing it came from. A remote target as the recording machine named it would mean nothing to another machine's config.

*Rejected:*
- **Per-session "neighbour" records** (each leaf knowing only its parent split): rebuilding them is harder than pruning a whole tree, for no saving that matters. A tab's blob is a few hundred bytes.
- **A tab id stored once in the core with sessions pointing at it:** that is a new core data structure, plus garbage collection.

### 4. One layout or one per client: neither — per session, last `DETACH` wins

Two Brindles showing sessions of one server each detach only their own sessions, so their layouts never collide. A session that moved between clients carries whichever tab detached it last.

The core clears `layout` on `ATTACH` (the `SET_META` the owner change sends carries it), so an attached session never advertises a stale tab. Grouping is still defensive: a session joins a tab only if its own blob names it (decision 6).

### 5. Size cap: 64 KiB, dropped not refused

`client_detach` keeps a layout only if its JSON encoding is at most 64 KiB (`MAX_LAYOUT`). A larger one is logged and dropped, and the `DETACH` still applies. Refusing the detach would leave a quitting client's session owned. 64 KiB is two orders of magnitude over a deep split tree, and small next to a checkpoint.

### 6. Grouping at restore (`layout::group`, pure)

Input: the sessions a sweep opens (already filtered: detached, sweepable, not taken by a `session.toml` zone, in start order).

1. Each session's blob is parsed. It belongs to the group `(quit, index)` only if its own key is a leaf of its own tree. Otherwise it is a group of its own (flat).
2. A group's tree is its first member's tree, pruned to the group's members: a split whose side has no members is replaced by the other side. Members that aren't leaves of that tree (impossible for one quit, handled anyway) become flat groups.
3. If the active leaf was pruned, the first leaf becomes active.
4. Order: every group of one `quit` sorts together, at the position of that quit's oldest `started` (ties by session id), and by `index` within it. A flat session sorts by its own `started`. This keeps today's "start order" rule for flat sessions, and keeps one quit's tabs in their recorded order.

The result is a list of `LayoutTab { title, zones: LayoutNode<SessionInfo> }`. One leaf is a plain tab, as today (plus its title).

### 7. Where grouping hooks in

- **This machine:** `Workspace::attach_detached(rest)` calls `layout::group(rest)`. For each tab it builds the terminals with `attached_terminal` (as today) and either `insert_terminal_tab` (one leaf) or a new `insert_zone_tab`. That one is `restore_zones` with infos already in hand: `ZoneTreeView::from_restored` and `insert_tab_inactive`. The active tab stays the same, as today.
- **Remote endpoints:** `reattach::swept_tabs` returns `Vec<(LayoutTab, profile index)>`, the profile taken from the tab's active session. `attach_swept` opens them the same way.
- **`session.toml` first:** the file's zones take their sessions before grouping (unchanged), so the file wins whenever both describe a session. A file-restored zone of a recorded tab removes that leaf from the group. The pruning gives the rest of the tab.

*Rejected:* turning layouts into `TabState`s and feeding them through `session::resolve_tab`/`Workspace::restore`. For remote endpoints, `restored_terminal` goes through the "wait for the listing" attach path, though the sweep already has the listing. It would also put the swept tabs ahead of the launch's request tab and change which tab is active.

### 8. Recording on quit

`Workspace::quit` makes one `quit` id and, for each local-origin tab at index `i`, asks its `ZoneTreeView` for a `Layout`. `layout::record(tree, active, key_of, …)` walks the `ZoneTree`. `key_of` is `zone_server_session(id)` (a running session's key), and zones without one are pruned as `session::record_zones` prunes message zones. `quit_all(layout)` then passes the blob, as a `serde_json::Value`, to each terminal's `Terminal::quit_with(layout)` → `Session::detach_with(layout)` → `Running::detach(layout)`. A session still starting (no key yet) detaches without one. The drop path (`Terminal::drop`) detaches without one: it runs only when `quit` didn't.

The index is the tab's position in the strip at quit, message tabs included. Only order matters, so gaps are harmless.

### 9. Session-less tabs, partial layouts, several hosts

- **Message tabs and failed attaches:** they have no session, so nothing records them (decision 8 prunes them). They don't come back, as today.
- **Ended sessions:** pruned at grouping (decision 6). A split whose zone ended shows the survivor alone. An empty tab doesn't exist, since a tab is only built from live members.
- **Several hosts:** one layout per host's server. A tab's zones are all on its host ("A tab stays on its host"), so a tab's layout lives entirely in one server. No cross-host merge: this machine's grouped tabs open at launch, and each endpoint's on its first listing at the end, as the flat ones do today.

### 10. What `session.toml` keeps

Everything it keeps today: window size and maximized, the active tab, every tab's profile, endpoint, title and zones with `{id, key}`, and the directories for tabs without a session (macOS, where no server runs). Dropping it would lose crash-safe restore of the instance's own tabs (decision 2's rejected alternative) and the window geometry.

### 11. Protocol rules check

- Client protocols stay 1–2. `DETACH.layout` and `SessionInfo.layout` are additive optional fields, omitted when empty, and pinned by the transcript tests (`client.rs` samples and the encoded-bytes transcript).
- H1 stays at 2. `SessionMeta.layout` is a meta field, and the meta is the core's opaque blob. The `SessionMeta` round-trip test pins it.
- Interop: a new client and an old core means the field is ignored and the sessions sweep flat. An old client and a new core means no layout. A new core adopting an old core's meta reads `layout: None`. An old core adopting a new core's meta parses only known fields, and its next `SET_META` drops the layout (a downgrade only loses layouts). An H1 v1 holder keeps the layout in core memory only (decision 1).
- `brindle-remote list --json` (if it serializes `SessionInfo`) shows the field. That is additive.

## Risks / Trade-offs

- [A crashed Brindle's sessions sweep flat into another Brindle] → Accepted. The same instance's relaunch still uses `session.toml`, and recording while attached would need a new protocol version (decision 2).
- [Layout clock/order across quits is by session start time, not quit time] → Accepted. It matches today's flat order, and one quit's tabs keep their own order.
- [An old core drops layouts after a downgrade] → Only a downgrade does this. Sessions then sweep flat as today.
- [A malicious or buggy client could store junk in another's sessions] → Only the session's attached client can `DETACH` it (`client_session` check), the core never interprets the blob, the 64 KiB cap bounds it, and Brindle ignores a blob that doesn't parse or doesn't name the session.

## Migration Plan

Nothing to migrate. Sessions detached by older Brindles have no layout and sweep flat. Rollback is reverting: older code ignores the fields.
