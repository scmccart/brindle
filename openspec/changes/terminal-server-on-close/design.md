# Design

Phase 2 of the terminal-server series. Phase 1's design is in `openspec/changes/archive/2026-11-07-terminal-server/design.md`.

## Context

- **Protocol 1 already has** `SPAWN { policy }` with `kill` and `detach`, `ATTACH` with `REPLAY_START`/`REPLAY_END`, `DETACH`, `LIST` (`SESSIONS`), and reconnect by `client_token`, which returns sessions without a client to their owner.
- **Brindle's client** (`remote::local`) already applies replays quietly: bells, OSC 9/777 notices and status notices are dropped between the markers, while the state still updates.
- **The core keeps each session's owner and policy** in the `meta` blob the holder stores. H1 version 1 can't update `meta` after `SPAWN`.
- **Phase 1 spawns every local tab with `kill`**, and closing a zone sends `CLOSE` (`Terminal::shutdown` → `Session::close`). A quit or crash drops the connection, and the core hangs up the client's `kill` sessions (`client_gone`).
- **Each Brindle process has a fresh random `client_token`.** A relaunched Brindle is never the owner of the sessions its predecessor left, so getting them back always goes through `ATTACH` by id, never through reconnect-by-token.
- **Session ids restart at 1** in a core that starts with no holders (`next_session`), so an id alone doesn't identify a session across server lifetimes.
- **Only the process holding `session.lock`** reads and writes the restore state. Today a second process opens its own windows beside the first.
- **Brindle has several windows today:** New Window (ctrl-shift-n), profiles opened in a new window, tmux sessions found and raised in another window ("Go to"), and restore recreating every recorded window.

## Goals / Non-Goals

**Goals:**
- Local Linux terminals outlive Brindle, and come back on the next launch.
- One rule for closing, with no setting and no prompt: closing a terminal ends it, Brindle going away keeps it.
- One window in one process, so "what comes back" has one answer.
- Sessions without a tab can be found and attached while Brindle runs.

**Non-Goals:**
- Windows terminals in the server (phase 3) and remote profiles (phase 4). Their tabs close and quit as today.
- tmux profiles: tmux already persists, and closing a tmux tab keeps killing that tmux window.
- Sessions surviving logout. Everything the user's session started ends with it.
- Several windows. A later change could bring them back with the layout in the server (`session-layout-in-server`).

## Decisions

### Closing ends, quitting keeps

The user's model, chosen after an earlier draft that detached on every close: a tab or zone is closed on purpose, so its program ends; Brindle going away is not a decision about the programs, so they stay.

| Event | local profile tab | `-e` command tab |
|---|---|---|
| close a zone or tab | `CLOSE`, the program ends | `CLOSE` |
| quit, close the window | sessions stay, detached | ends |
| crash, SIGTERM, the desktop stopping Brindle | stays, detached | ends |
| the program exits | the zone or tab closes | the tab closes |

- **Spawn policy:** profile tabs spawn with `Policy::Detach`, so anything that can't send `CLOSE` (a crash, SIGTERM, the session manager ending Brindle) keeps the session. Closing sends `CLOSE`, which ends a session whatever its policy.
- **`-e` command tabs spawn with `Policy::Kill`:** they are never restored ("Commands and messages are not replayed"), so a dropped connection ends them.
- **Quitting sends nothing.** The connection drops; `detach` sessions stay, `kill` sessions are hung up by `client_gone`. No prompt, no flush.
- **No `on_close` setting.** It existed only on this branch, so no released config has it.

### One window

- **New Window (`new_window`, ctrl-shift-n) is removed,** and with it opening a profile "in a new window" and the "New window: <profile>" rows.
- **Closing the window is quitting.** The caption ✕, the window manager's close and `Close Window` quit Brindle; `Close Window` stays as a command because it is what the ✕ does.
- **tmux control-mode sessions** keep opening as tabs of the window. A session this window shows is activated, as today; the "open in another window", "Go to <session>" and "switches" paths are removed, since there is no other window. "Session shown by another Brindle" stays: another process (a debug run, another `BRINDLE_STATE_DIR`, another machine's client) can still show it.
- **`tmux_session_end = "close_window"`** keeps its meaning: the window closes, which now quits Brindle.
- **Restore merges windows.** A state file with several windows (from earlier versions) restores as one window: the tabs of every recorded window in order, the first window's size and active tab, each control-mode session once.

### One instance

- **The instance is the process holding `session.lock`.** Nothing new decides it, and `BRINDLE_STATE_DIR` still separates test runs.
- **The request channel:** the instance listens on a Unix socket at `$XDG_RUNTIME_DIR/brindle-instance-<hash>.sock` (the hash of the state directory's path, so state directories in long paths still fit `sun_path`; `~/.cache/brindle/run` when `XDG_RUNTIME_DIR` is unset), mode 0600, peers checked with `SO_PEERCRED`. On Windows it is a named pipe, `\\.\pipe\brindle-instance-<hash>-<user>`, with a default ACL for the user.
- **A launch that can't take the lock** (and has no debug options) connects, sends one JSON request (the launch options, its working directory, and `XDG_ACTIVATION_TOKEN` or `DESKTOP_STARTUP_ID` when set), waits for `ok` and exits 0. The instance may be starting (the lock taken, the socket not yet bound), so the client retries for up to 5 seconds, then prints why and exits 1.
- **The instance** raises its window (on Wayland with the forwarded activation token through `activation.rs`; on X11 with GPUI's activation; on Windows the launcher first calls `AllowSetForegroundWindow` for the instance's pid) and opens the request as a launch in its window would: `-p`/`-d` a tab, `-e` a command tab in the sender's directory, `--attach` an attached tab, and nothing for a bare launch.
- **Debug options** (`--send`, `--action`, `--dump-screen-after`, `--latency-log`, `--latency-bench`) never forward. A launch with them that can't take the lock runs as a second process does today: its own window, no restore, no reattach, no state written. The e2e suite gives every run its own state directory, so its runs are instances.

### Every instance launch restores, then opens its request

With one window, a launch's options can't mean "a different window instead of the session". So the instance always restores (except `--no-restore`), reattaches, and then opens what the launch asked for as the active tab: `-p` and `-d` a new tab, `-e` a command tab, `--attach` the attached session. A bare launch opens nothing more, or one default-profile tab when nothing was restored.

- **`--no-restore`** skips restore and reattach for that launch, opens the request (or a default tab), and leaves the detached sessions alone. It still owns the state, which then records what it shows.
- **`-e`** used to skip restore; now its command opens as a tab on top of the restored ones. When it is the first launch, the restored tabs and reattached sessions come with it.

### Restore is always on

Reattaching without the recorded layout flattens split tabs, so restore is unconditional.

- **`restore_session` is removed.** The config parser denies unknown fields, so the key stays accepted and ignored, with one warning, so existing configs (the user's own included) keep loading. It leaves `DEFAULT_CONFIG`, README and the main specs.
- **`--no-restore` stays** for one-off launches, and `BRINDLE_STATE_DIR` for tests.
- **The e2e and perf harnesses give every case its own `BRINDLE_STATE_DIR`,** not only the restore cases: otherwise a test run would restore, and overwrite, the user's real state.
- **Attaching replays quietly:** the existing replay handling drops bells and notices.

### Restored zones get their sessions back, or are dropped

- `SPAWN` gets back a random 128-bit `key` the core generates and keeps in `meta`; `SESSIONS` reports it (additive field). Restore records `{ id, key }` per local Linux zone.
- **A recorded zone whose session is detached with that id and key** is attached into that zone.
- **A recorded zone whose session is gone,** attached elsewhere, or not the recorded one (a new server reused the id), is dropped: the user asked not to get empty shells back for terminals that ended. Its split's sibling takes its place; a tab with no zone left is not restored. After a reboot or `brindle-remote stop`, no local tab comes back.
- **A zone with no record** (a state file from before this change, Windows, remote profiles) restores as before: its profile in its recorded directory.
- **When nothing is left to restore,** the launch opens one default-profile tab.
- **Every other detached session** (sessions a `--no-restore` launch left, or a previous instance's tabs whose state write was lost) opens as a new tab, in start order, with its profile looked up by the name in its meta (the default profile when gone). These come back flat; `session-layout-in-server` would give them their places.
- **An attached terminal is fed its replay at the session's size** (the replay is rendered at it), then resized to its own: an inactive tab isn't laid out, so its terminal would otherwise take the replay at 80x24.

### `ATTACH` makes a terminal from an existing session

- `Connection::attach(id, size, sink)` sends `ATTACH` and resolves on `REPLAY_START` (with its `req`) or `ERROR`; `Session::attach` mirrors `Session::start`, so the UI never blocks.
- The sink is in the session map before the replay's first `DATA`.
- An attached terminal closes as any profile tab does: `CLOSE` on close, kept at quit.

### The sessions palette

- A labelled action, `attach_session` ("Attach Session…"), opens a palette of detached sessions from `LIST`.
- **Rows** show the session's title (the core's emulator keeps it), command, directory and age. `SessionInfo` gains `title` and `started` (unix seconds), additively.
- **Enter** attaches into a new tab. A second binding, `picker_kill`, kills the selected session with `KILL` and refreshes the list.
- **Detached sessions only.** Ones attached to another client aren't listed.
- Kept although launches reattach everything: sessions can still be detached while Brindle runs (`--no-restore` launches, and later, programs launched detached or through the server).

### `brindle --attach <id>`

Opens a tab attached to that session, in the instance (forwarded when one runs). A session that is gone or attached elsewhere gives a message tab naming why. `brindle-remote list` already prints ids.

### Server fixes

- **`DETACH` of a `kill` session switches it to `detach`.** An explicit detach means "keep it".
- **An explicitly detached session has no owner.** Otherwise a reconnect returns it to the client that detached it, which has no tab for it, and that client's "unknown replay → `CLOSE`" ends it. So `DETACH` clears the owner; `client_gone` keeps the owner, so a reconnect still gets back sessions it lost only to a dropped connection.
- **`ATTACH` makes the attaching client the owner.**
- **H1 version 2 adds `SET_META { session, meta }`.** The core sends it whenever owner or policy changes, so they survive a core restart. With a holder speaking only H1 v1, the core skips it and logs, and that holder's sessions keep phase 1's limitation until it drains.

### Server roles start in their own systemd scope

- **Why:** GNOME starts Brindle in `app-gnome-brindle-<pid>.scope` (`PartOf=graphical-session.target`, `KillMode=control-group`). A core and holders started from Brindle stay in that cgroup, so stopping the scope or systemd-oomd killing it ends every session. tmux 3.6 moves each pane into its own `tmux-spawn-*.scope` for the same reason.
- **What:** when a systemd user manager is reachable (`$XDG_RUNTIME_DIR/systemd/private` exists and `systemd-run` is on `PATH`), the starter runs `systemd-run --user --scope --collect --quiet --unit=brindle-<role>-<random> -- <helper> <role> --ready-fd N`. The ready fd is inherited through the scope, and the double fork stays in it.
- **Fallback:** started as today, with one log line. `BRINDLE_NO_SCOPE=1` forces the fallback (tests, and the e2e suite's private server directories).

## Risks / Trade-offs

- **Losing several windows** is a regression for anyone who used them. → Tabs and splits cover the same ground in one window; `session-layout-in-server` is where several windows could return.
- **A quit with many tabs leaves many programs running.** → That is the point; the next launch shows them all, and closing a tab ends its program.
- **Dropped zones after a reboot:** local tabs don't come back as fresh shells any more. → The user chose this; tmux and remote tabs still restore.
- **A hung instance** makes later launches wait. → The forwarding client gives up after 5 seconds and says so; `--no-restore` with another `BRINDLE_STATE_DIR` still starts a separate Brindle.
- **Many detached sessions make a slow launch:** each attach replays scrollback. → Attaches run off the UI thread, and tabs appear as their replays arrive.
- **A new Brindle against an old core:** `SESSIONS` without `key` means restore can't match records, so their zones are dropped. An old holder without H1 v2 keeps the ownership limitation.
- **systemd-run differences:** on systems without a user manager the fallback is today's behaviour.

## Migration

- Quitting no longer ends local programs; closing a tab still does. The changelog marks it breaking (`feat!`), with the one-window and one-instance changes.
- Restore files gain an optional session record per zone; version 1 files without one still load, and several recorded windows merge into one.
