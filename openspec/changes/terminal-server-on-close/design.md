# Design

Phase 2 of the terminal-server series. Phase 1's design is in `openspec/changes/archive/2026-11-07-terminal-server/design.md`.

## Context

- **Protocol 1 already has** `SPAWN { policy }` with `kill` and `detach`, `ATTACH` with `REPLAY_START`/`REPLAY_END`, `DETACH`, `LIST` (`SESSIONS`), and reconnect by `client_token`, which returns sessions without a client to their owner.
- **Brindle's client** (`remote::local`) already applies replays quietly: bells, OSC 9/777 notices and status notices are dropped between the markers, while the state still updates.
- **The core keeps each session's owner and policy** in the `meta` blob the holder stores. H1 version 1 can't update `meta` after `SPAWN`.
- **Today every local tab spawns with `kill`**, and closing a zone sends `CLOSE` (`Terminal::shutdown` → `Session::close`). A quit or crash drops the connection, and the core hangs up the client's `kill` sessions (`client_gone`).
- **Each Brindle process has a fresh random `client_token`.** A relaunched Brindle is never the owner of the sessions its predecessor left, so getting them back always goes through `ATTACH` by id, never through reconnect-by-token.
- **Session ids restart at 1** in a core that starts with no holders (`next_session`), so an id alone doesn't identify a session across server lifetimes.
- **Only the process holding `session.lock`** reads and writes the restore state.

## Goals / Non-Goals

**Goals:**
- Local Linux terminals outlive Brindle by default, and come back on the next launch.
- One per-profile option decides what closing does, for every way a terminal can be closed.
- Sessions without a tab can be found and attached while Brindle runs.

**Non-Goals:**
- Windows (phase 3) and remote profiles (phase 4). Their tabs ignore `on_close`.
- tmux profiles: tmux already persists, and closing a tmux tab keeps killing that tmux window.
- Sessions surviving logout. Everything the user's session started ends with it.

## Decisions

### `on_close = "detach" | "ask"`, default `detach`; no kill mode

A per-profile key (also in the settings dialog's profile page). The user chose `detach` as the default, since persistence is the point of the series, and then dropped a per-profile `kill` mode altogether: a program ends when it exits, when the user picks "end" in an `ask` prompt, or when it is killed from the sessions palette or `brindle-remote kill`.

| Event | detach | ask |
|---|---|---|
| close a zone, tab or window | `DETACH` | prompt: keep running / end / cancel |
| quit Brindle | sessions keep running | one prompt for all asking sessions |
| crash, SIGTERM, a dropped connection that never comes back | keeps running | keeps running |
| the shell exits | tab closes | tab closes |

- **Spawn policy:** profile tabs always spawn with `Policy::Detach`, so anything that can't ask (a crash, SIGTERM, the session manager ending Brindle) keeps the session.
- **`-e` command tabs spawn with `Policy::Kill`** and close with `CLOSE`: they are never restored ("Commands and messages are not replayed"), so a command whose window closed must not come back as a reattached tab. The server's `kill` policy stays for them (and for phase 4).
- **Config compatibility:** `on_close = "kill"` is a config error, like any unknown value, naming the accepted values.
- **`ask` on close:** one palette prompt per close action, covering every asking zone it closes (a window or a split tab can hold several). Cancel closes nothing.
- **`ask` on quit:** `Quit`, and closing the last window, show one prompt covering every asking session in every window. "End" sends `CLOSE` for each and waits for the writer to flush before the app exits (as tmux's `release` does); "keep" lets the dropped connection detach them.
- **The window's caption ✕** goes through GPUI's should-close callback, so it asks too.
- **Profile changes** don't touch running terminals: whether a terminal asks is fixed when it starts.

### Every launch reattaches every detached session

The user chose this over a prompt or a palette-only model. There is no option to turn it off.

- **Every launch except `-e` and `--no-restore`**, which ask for one specific window.
- **Only the process holding `session.lock` reattaches.** A second Brindle running at the same time takes nothing.
- **A restored zone** whose record carries a session that is still detached is attached into that zone. A zone whose session is gone respawns in its recorded directory, as today.
- **Every other detached session** (closed tabs, sessions a `--no-restore` or `-p` launch left, sessions another Brindle left) opens as a new tab in the first window, in start order, with its profile looked up by the name in its meta (the default profile when gone). These come back flat; `session-layout-in-server` would give them their places.

### Restore is always on

Reattaching without the recorded layout flattens split tabs and merges windows, so the user made restore unconditional rather than opt-in.

- **`restore_session` is removed.** The config parser denies unknown fields, so the key stays accepted and ignored, with one warning, so existing configs (the user's own included) keep loading. It leaves `DEFAULT_CONFIG`, README and the main specs.
- **`--no-restore` stays** for one-off launches, and `BRINDLE_STATE_DIR` for tests.
- **The e2e and perf harnesses give every case its own `BRINDLE_STATE_DIR`,** not only the restore cases: otherwise a test run would restore, and overwrite, the user's real state. Cases that need a fresh window keep passing nothing; an empty state directory restores nothing.
- **Attaching replays quietly:** the existing replay handling drops bells and notices.
- **`--attach`, `-p` and the e2e steps** still act on the active tab, which stays the tab the launch would have made active without reattached sessions.

### Restore records a session key, not just an id

- `SPAWN` gets back a random 128-bit `key` the core generates and keeps in `meta`; `SESSIONS` reports it (additive field). Restore records `{ id, key }` per zone.
- A recorded session is attached only when `LIST` shows that id with that key and no client.

### `ATTACH` makes a terminal from an existing session

- `Connection::attach(id, size, sink)` sends `ATTACH` and resolves on `REPLAY_START` (with its `req`) or `ERROR`; `Session::attach` mirrors `Session::start`, so the UI never blocks.
- The sink is in the session map before the replay's first `DATA`.
- A terminal attached this way closes as its profile's `on_close` says (the default profile's when its profile is gone).

### The sessions palette

- A labelled action, `attach_session` ("Attach session…"), opens a palette of detached sessions from `LIST`.
- **Rows** show the session's title (the core's emulator keeps it), command, directory and age. `SessionInfo` gains `title` and `started` (unix seconds), additively.
- **Enter** attaches into a new tab of this window. A second binding, `picker_kill`, kills the selected session with `KILL` and refreshes the list.
- **Detached sessions only.** Ones attached to another Brindle aren't listed.

### `brindle --attach <id>`

Opens the launch's window with a tab attached to that session (still subject to the launch-time reattach above, so it isn't attached twice). A session that is gone or attached elsewhere gives a message tab naming why. `brindle-remote list` already prints ids.

### Server fixes

- **`DETACH` of a `kill` session switches it to `detach`.** An explicit detach means "keep it".
- **An explicitly detached session has no owner.** Otherwise a reconnect returns it to the client that detached it, which has no tab for it, and that client's "unknown replay → `CLOSE`" ends it. So `DETACH` (and a tab close under `detach`) clears the owner; `client_gone` keeps the owner, so a reconnect still gets back sessions it lost only to a dropped connection.
- **`ATTACH` makes the attaching client the owner.**
- **H1 version 2 adds `SET_META { session, meta }`.** The core sends it whenever owner or policy changes, so they survive a core restart. With a holder speaking only H1 v1, the core skips it and logs, and that holder's sessions keep phase 1's limitation until it drains.

### Server roles start in their own systemd scope

- **Why:** GNOME starts Brindle in `app-gnome-brindle-<pid>.scope` (`PartOf=graphical-session.target`, `KillMode=control-group`). A core and holders started from Brindle stay in that cgroup, so stopping the scope or systemd-oomd killing it ends every session. tmux 3.6 moves each pane into its own `tmux-spawn-*.scope` for the same reason.
- **What:** when a systemd user manager is reachable (`$XDG_RUNTIME_DIR/systemd/private` exists and `systemd-run` is on `PATH`), the starter runs `systemd-run --user --scope --collect --quiet --unit=brindle-<role>-<random> -- <helper> <role> --ready-fd N`. The ready fd is inherited through the scope, and the double fork stays in it.
- **Fallback:** started as today, with one log line. `BRINDLE_NO_SCOPE=1` forces the fallback (tests, and the e2e suite's private server directories).

## Risks / Trade-offs

- **Sessions accumulate:** every closed tab keeps a shell running. Reattaching all of them at launch makes them visible, `exit` or the palette's kill ends one, and `on_close = "ask"` offers "end" at close time.
- **Many detached sessions make a slow launch:** each attach replays scrollback. → Attaches run off the UI thread, and tabs appear as their replays arrive.
- **A new Brindle against an old core:** `SESSIONS` without `key` means restore can't match records; they respawn. An old holder without H1 v2 keeps the ownership limitation.
- **systemd-run differences:** on systems without a user manager the fallback is today's behaviour.

## Migration

- Existing profiles get `on_close = "detach"`, which changes what closing a tab does, and no profile can restore the old behaviour. The changelog marks it breaking (`feat!`).
- Restore files gain an optional session record per zone; version 1 files without one still load.
