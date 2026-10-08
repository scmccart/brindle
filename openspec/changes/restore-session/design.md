# Design

## Context

- **Launch:** `main.rs` parses the arguments, builds one `LaunchRequest` (profile, optional command, optional cwd) and opens one window through `open_window`, which always uses a centred 1000×660 window. `Workspace::new` launches that request.
- **Tabs:** `Workspace` owns `Vec<Tab>`. A `Tab` has a `Profile` and either a local `TerminalView` or a tmux `(TmuxSession, TmuxWindowView)`. Local tabs come from `launch()`, which also covers command overrides (`-e`, the `ctrl-,` editor tab), and from `open_message_tab()` (attach failures, backed by a `Remote` terminal). Nothing on `Tab` records which of these it was.
- **Directories:** `Terminal::working_directory()` reads `/proc/<pid>/cwd` of the foreground process (falling back to the shell). New tabs inherit it.
- **tmux control mode:** `attach_tmux` creates a `TmuxSession` (`tmux -C new-session -A -s <name>`), so a missing session is created. The session announces its windows with `WindowAdded` (inserted inactive, after the session's last tab, else after the active tab) and then `WindowActivated` for tmux's current window. Activating a tmux tab sends `select-window` unless that window is already current. `session_name` follows `%session-renamed`.
- **Polling:** each `Workspace` already runs a 1 s task that refreshes tab titles from `/proc`.
- **Quit:** closing the last tab calls `window.remove_window()`. `on_window_closed` then quits when no windows remain. `cx.quit()` runs `on_app_quit` observers, which get a shared 100 ms budget (`TmuxSession` already uses up to 50 ms of it). By then the closed window is gone. `--dump-screen-after` quits through `cx.quit()` with its window still open.
- **Dependencies available:** `serde` (derive), `toml` 0.8 (serialize and parse), `dirs` 5 (`state_dir()`), `libc`.

## Goals / Non-Goals

**Goals:**
- Restore windows, tab order, active tab, profiles, directories and tmux attachments on a plain launch, with no change at all when the setting is off.
- Keep a snapshot on disk that is never more than a few seconds old, without file I/O on the UI thread's render path.
- Never violate "attaching keeps tmux's selection".

**Non-Goals:**
- Scrollback or screen contents, running programs, and `-e` commands (see proposal).
- Window positions: Wayland doesn't let clients place windows, and on X11 the window manager's placement is good enough.
- Per-pane state inside tmux, which tmux keeps itself.
- Several Brindle processes sharing or merging one session.

## Decisions

### 1. Setting name and default: `restore_session = false`

This is a top-level boolean beside `copy_on_select` and `osc52_paste`, and it is off by default as asked. The default config gets a commented `# restore_session = false` line explaining it. The name matches the feature's wording ("restore session") and follows the existing `snake_case` keys.
- *Alternative:* a `[session]` table. Rejected for now: there is only one key.

### 2. State file: versioned TOML at `<state dir>/session.toml`

```toml
version = 1

[[windows]]
width = 1000.0      # logical pixels of the restore size
height = 660.0
maximized = false
active = 1          # index into tabs

[[windows.tabs]]
kind = "local"
profile = "Shell"
cwd = "/home/me/src"

[[windows.tabs]]
kind = "tmux"
profile = "tmux"
session = "main"
```

- **Directory:** `$BRINDLE_STATE_DIR` if it is set and non-empty, else `dirs::state_dir()/brindle` (that is `$XDG_STATE_HOME`, default `~/.local/state`), else `~/.local/state/brindle`.
- **Format:** TOML, because `toml` and `serde` are already dependencies (`serde_json` is only in the lockfile transitively). It is also readable and editable by hand.
- **Version handling:** `version` is checked first, through a tiny `{ version: u32 }` parse. Anything other than `1`, or a parse failure, is logged (`ignoring session state <path>: <why>`) and treated as no state. The structs don't use `deny_unknown_fields`, so a later version that only adds optional fields could still be read by bumping the accepted range. Unknown `kind` values make the whole file invalid, which is acceptable for version 1.
- **Atomic write:** create the directory, write `session.toml.tmp`, `sync_all`, then rename it over `session.toml`.
- The file holds directory names, so it is created with mode 0600 (and the directory 0700), like other per-user state.

### 3. What is recorded per tab: an origin tag set when the tab is created

`Tab` gains `origin: TabOrigin`, with `Local` (any PTY tab, including command overrides), `Tmux`, or `Message`. It is set by `launch` / `on_tmux_event` / `open_message_tab`, so the snapshot doesn't have to guess from the terminal backend.
- **Local:** `{ profile, cwd }`. `cwd` comes from `Terminal::working_directory()` and is omitted when it can't be read.
- **Command overrides** come back as the profile's own program in the recorded directory. Re-running an editor on the config, or `htop`, on every launch would surprise more people than it helps, and other terminals with session restore behave the same way.
- **Message** tabs are skipped.
- **Classic tmux** tabs are ordinary local tabs: restoring reruns the profile, whose `new-session -A` reattaches.
- **Control-mode** tabs record `{ profile, session }` once per session per window, at the position of the session's first tab. `session` is the live `session_name`, so a renamed session reattaches by its new name. An empty window list (a session still attaching) also records the session, so a quit during an attach doesn't lose it.

### 4. Active tab

`active` is the index into the recorded tab list. For a tmux session entry it means "this session's tab was active".
- **If the active entry is local:** restoring activates that tab after all tabs are launched. Every restored tmux session is marked *quiet*: its first `WindowActivated` doesn't activate its tab. Tabs are added inactive anyway, so focus stays on the local tab and nothing is sent to tmux.
- **If the active entry is a tmux session:** that session is not quiet. When its `WindowActivated` arrives, tmux's current window becomes the active tab, which is what attaching does today. Any other restored session in the same window is quiet. The active window id is never saved or replayed: activating a window that is no longer tmux's current one would send `select-window` and break the `tmux-control-mode` requirement.
- Until the session's tabs arrive, the window shows the restored local tabs, the first one focused. This is the same as a `-p tmux` launch, which shows an empty window until then.

### 5. Placing a session's tabs: anchor by tab id

When a session is attached during restore, the workspace stores the id of the restored tab just before it, or "front" when nothing precedes it. Its first `WindowAdded` inserts at `position(anchor) + 1`, or at 0. Later windows already group after the session's last tab. If the anchor tab is gone by then, the default placement is used. Ids are used rather than indexes because local tabs are inserted synchronously while tmux tabs arrive later.

### 6. Fallbacks at restore time

- **Profile:** `Config::profile_index(name)`, else the default profile, with a warning. A recorded `tmux` entry whose profile is (now) not a control profile is launched as an ordinary tab of the resolved profile. A `local` entry whose profile is now a control profile attaches that profile's session (deduplicated, as usual, by `attach_tmux`).
- **Directory:** the recorded `cwd` if it is a directory, else the profile's `cwd` if that is a directory, else the home directory. It is always passed explicitly, so a restored tab never inherits the previous tab's directory by accident.
- **tmux session gone:** decided to **reattach, which recreates the session**, rather than skip it. `new-session -A -s <name>` creates the session, matching what opening the profile does, and the user asked for that session to be there. If tmux itself fails (not installed, bad args), the existing `Failed` path opens the explanation tab.
- **Empty restore:** if no window in the state has a tab after filtering, the launch is normal.

### 7. When the state is written: snapshot on the 1 s poll, write on change, and on quit

- **Store:** a GPUI global `SessionStore` holds a snapshot per window, keyed by `AnyWindowHandle::window_id().as_u64()`, plus a creation sequence number for ordering. It also holds the last serialized text that was written.
- **Snapshot:** each `Workspace`'s existing 1 s title poll also stores its window snapshot. This only reads `/proc` links, which it already does for titles, and window bounds. It is skipped when `restore_session` is off or this process doesn't own the state (Decision 9).
- **Write:** an app-level task ticks every second. It serializes the store and, only if the text differs from the last write, hands it to `background_executor().spawn` for the atomic write. A change therefore reaches disk within about 2 s, and an idle Brindle never writes. This covers tab changes without hooking every tab mutation.
- **Quit:** an `on_app_quit` observer refreshes the snapshots of windows still open (the ctrl-shift-q path), serializes, and writes **synchronously**. The file is tiny, well within GPUI's 100 ms quit budget.
- **Logout and crashes:** SIGTERM, a lost display or a crash skip the quit hook. The last periodic write, at most about 2 s old, is what remains. A signal handler is not added: GPUI owns the event loop, and the periodic write already bounds the loss.
- *Alternative:* write on every tab mutation. Rejected: there are many call sites, directories change without events anyway, and a poll is needed regardless.

### 8. Forgetting windows

- When a workspace removes its last tab, it removes its entry from the store before `remove_window()`. When the app then quits, the store holds no such window, so the next launch is fresh.
- On `on_window_closed`, entries whose window no longer exists are dropped, but only if some window is still open. When the last window closes with tabs (close button, compositor close), its entry stays and quit writes it.
- **Result:** closing a single window deliberately among several forgets it. Quitting keeps everything. Typing `exit` in the last tab leaves nothing to restore.

### 9. One owner per user: `flock` on `session.lock`

The first process that needs the state (at launch with the setting on, or on the first tick after it is turned on) takes `flock(LOCK_EX | LOCK_NB)` on `<state dir>/session.lock` and keeps the descriptor for its lifetime. A process that can't take the lock logs that another Brindle owns the session, and then neither restores nor records. Without this, opening Brindle from the launcher while it is already running would restore every tab, and attach every tmux session, a second time, and the two processes would overwrite each other's snapshot. The kernel releases the lock if the process crashes, so there's no stale-lock cleanup.

### 10. Plain launch and `--no-restore`

- A plain launch has none of `-p`, `-d`, `-e` or `--no-restore`. `--send`, `--action`, `--dump-screen-after` and `--config` don't count, so the e2e cases can drive restored windows. Debug steps and the dump act on the first restored window.
- Non-plain launches still **record** (if they own the state). The next plain launch therefore reopens whatever that run ended with, and `--no-restore` means "start fresh this time".
- `--no-restore` is a launch flag only. A command-palette entry was considered and not added, because there's nothing to do with restore once Brindle is running.

### 11. Window size

- **Recorded:** the size of `window.inner_window_bounds()` (the restore size, even when maximized), and `maximized` from the `WindowBounds::Maximized` variant. Fullscreen is recorded as not maximized. `window_bounds()` was tried first. Under Xwayland it includes the 6 px client-decoration inset on each side, so the window grew by 12×12 px on every restore (1000×660 → 1012×672 → 1024×684). `inner_window_bounds()` gives back exactly the requested 1000×660.
- **Restored:** `WindowBounds::Windowed(Bounds::centered(size))`, or `Maximized(...)`, which GPUI turns into `zoom()` when it opens the window. Sizes are clamped to at least the 360×200 minimum.
- The e2e round trip checks under X11 that the size doesn't drift across restores. With client-side decorations on Wayland, GPUI's bounds might include the shadow inset, which would grow the window on each restore. If the X11 check shows `inner_window_bounds` is the one that matches the requested size, that is used instead. The Wayland case is left for the user's hand test.

### 12. Code layout

- `src/session.rs`, with no GPUI types in the pure parts:
  - the state structs;
  - `state_dir()` / `state_path()`;
  - `parse(text) -> Result<State, String>` with version checks, and `to_text`;
  - `write_atomic`;
  - `Lock::acquire`;
  - `resolve_tab(entry, &Config) -> RestoredTab`, which applies the fallbacks;
  - the `SessionStore` global.

  The pure parts are unit-tested.
- `workspace.rs`: `Workspace::new` takes a `Launch` (`Request(LaunchRequest)` or `Restore(WindowState)`). It also gains `snapshot()` and the restore bookkeeping (quiet sessions, anchors).
- `main.rs`: argument parsing, `open_window(launch, bounds)`, the store and writer setup, and the quit hook.

## Risks / Trade-offs

- [The `/proc` cwd is the foreground process's, so a tab running `vim` in a subdirectory records that subdirectory.] → Accepted. That is the same directory a new tab would inherit, and the spec says so.
- [A restored tab whose profile now pins a different `cwd` uses the recorded directory, not the pinned one.] → Intentional, because the user moved there. The pinned directory is still the fallback.
- [Polling every second for snapshots.] → It reuses the existing title poll. The added work is one `readlink` per local tab and a string compare, and the file is only written when the text changes.
- [With two windows attached to the same tmux session, restoring attaches it once per window, as before the quit.] → Accepted. This mirrors what was open.
- [Quiet sessions suppress exactly one `WindowActivated`.] → `list-windows` always marks one window active, so the first window list emits it, and the flag is used up there. Even if it lingered, at most one later activation would be skipped.
- [The state file records directory paths.] → It is written 0600 under the user's state directory.
- [Snapshots are skipped while restore is off, so turning it on mid-run records from the next tick only.] → As specified ("within a few seconds").

## Migration Plan

None. The setting defaults to off, and existing configs parse unchanged. To roll back, remove the setting. A leftover `session.toml` is ignored when the setting is off and can be deleted.
