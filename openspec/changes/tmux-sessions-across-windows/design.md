# Design

## Context

- Each `Workspace` (one per window) owns its control-mode sessions in `tmux_sessions`.
- `attach_tmux` already refuses to attach a session twice, but it only searches its own list and matches on profile name plus session name.
- `NewWindow` (`main.rs`) always launches the default profile.
- A session's name comes from the profile (`tmux_session`, default `main`) and is updated on `%session-renamed`. `%session-changed` triggers only a window refresh.
- An attach that tmux refuses (for example, `duplicate session: work`) arrives as a `%begin…%error` block with flags 0. The `Collector` reports it as `Response { ours: false }`, and `handle_event` ignores that. Probed with tmux 3.6:
  - `tmux -C new-session -s work` while `work` exists prints `%begin` / `duplicate session: work` / `%error` / `%exit` and exits 1.
  - `tmux -C new-session` without `-s` reports the chosen name in `%session-changed $1 1`.

See proposal.md for the motivation, and the delta specs for the required behaviour.

## Goals / Non-Goals

**Goals:**
- Within one Brindle process, each tmux session has at most one control client.
- Creating a session never reattaches by accident: it runs without `-A`.

**Non-Goals:**
- Coordinating between Brindle processes. A second process doesn't know about the first, and asking tmux with `list-clients` is a possible follow-up.
- Moving a session's tabs between windows.
- Grouped sessions (`new-session -t`).

## Decisions

### Session identity is the server plus the name

A session is identified by `SessionKey { program, tmux_args, name }`, built from the session's profile (`command`, else `tmux`, and `tmux_args`) and its *current* tmux name.
- **Why:** two profiles that point at the same server and session are the same tmux session, so matching on the profile name would still let them mirror each other.
- **Unknown names:** a session whose name isn't known yet (created with an empty name and still waiting for `%session-changed`) has no key and matches nothing. Otherwise it would match the profile's default `main`.
- **Detached sessions** match nothing either. This is the case where the ended session is still on screen behind the "ask" palette. Picking its profile from that palette must attach again, not focus the dead tabs.

Alternative considered: keep the match on profile name. It's simpler, but leaves the server-sharing case broken for no saving.

### Find open sessions by scanning windows, not with a registry

`find_session(key, cx)` returns where a session is open: in this window (a tab index), in another window (`WindowHandle<Workspace>` and the session entity), or nowhere.
- **How it searches:** it checks `self.tmux_sessions` first. It then walks `cx.windows()`, using `downcast::<Workspace>()`, and reads each other workspace. It skips its own window, whose root can't be read while it's being updated.
- **Why not a global map:** a map from `SessionKey` to window would have to be updated on every attach, rename, detach, failure and window close. Each missed update would be a stale entry that blocks an attach. There are only ever a few windows and sessions, so scanning costs nothing.

`attach_tmux`, the profile palette, and `open_profile` use it.

### Focusing another window

To focus a session open in another window, `cx.defer` a `handle.update(...)` that:
1. activates the session's tab: tmux's current window if it has a tab, else the first tab;
2. raises the window.

The deferred update runs after the current window's update has finished, the same reason `ReloadConfig` is deferred.

**Raising on Wayland** uses `notify::activation`. GPUI's `activate_window` requests a token for the *target* surface, which is unfocused, so GNOME refuses it and shows "Brindle is ready". `activation.rs` gets a second entry point that requests an `xdg_activation_v1` token on its own queue for the *focused* window's surface, which is the window where the user just pressed the key or clicked. It then activates the target surface with that token.

GNOME (Mutter) only honours a token whose request names the serial of the latest key or button press on the focused surface; a request without one gets a token that activates nothing, so GNOME shows "Brindle is ready" (seen in the manual check). GPUI tracks those serials but keeps them private. So `activation.rs` attaches to GPUI's display as soon as a Wayland window opens, binds its own `wl_seat`, and creates its own `wl_keyboard` and `wl_pointer` on its queue. The compositor sends input events, with their serials, to every keyboard and pointer object of the client, so ours see the same serials as GPUI's. It keeps the serial of the latest key or button press (a release read in the same batch as its press would not match), and `request_token` sends it with `set_serial`. A 1 s timer drains the queue, so pointer motion and keymap fds don't pile up between requests. A switch with no input behind it (e2e's `--action`) still gets the fallback.

If anything fails (X11, the protocol missing, the token refused), it falls back to `activate_window`. The tab is activated either way, so the worst case is the existing "is ready" notification. The spec says only that Brindle *asks* the desktop to raise the window.

### Asking when a session is open elsewhere

There is one entry point, `Workspace::open_profile(ix, target, window, cx)`, where `target` is `Tab` or `Window`. ctrl-alt-N, `new_tab` (non-tmux tab) and `NewWindow` all go through it:
- **Not a control-mode profile, or the session isn't open:** it launches as today, as a tab, or as a new window via `open_window`.
- **The session is open in this window and `target` is `Tab`:** it activates the session's tab, as today.
- **Otherwise:** it shows `PaletteKind::SessionOpenElsewhere { profile, location, target }`. A new window counts as "elsewhere" even from the window showing the session.

The new-tab palette doesn't ask: its row already says "in another window (switches)", so confirming that row calls `go_to_session` directly, and asking again was a redundant second prompt (found in the manual check). Other rows go through `open_profile` with `Target::Tab`.

How the palette works:
- It's an ordinary dismissible `Palette::list` (not `modal()`), so escape and a click outside both cancel through the normal `Dismissed` path with nothing to undo.
- `session_open_rows(config, picked, open_keys, target)` builds the rows. It's a pure function, unit-tested like `session_ended_rows`. It returns "Go to <session>", "New tmux session…", then `New tab:`/`New window:` rows for each profile other than the picked one, skipping control-mode profiles whose key is open anywhere. Each row maps to an enum, so confirming doesn't rely on row arithmetic.

`NewWindow` is now handled by each window's `Workspace` (registered whether or not the settings dialog is open), so the palette shows in the window where it was pressed. Looking up `cx.active_window()` from the global handler was considered, but it is `None` whenever no Brindle window has platform focus (for example, unfocused e2e runs). The global handler in `main.rs` stays for a dispatch no window takes.

**The session-ended palette:** `session_ended_rows` takes the open keys too, and labels such a profile "Go to <session>". Its confirm goes to the session's window and then removes the ended tabs, which closes this window.

**Alternative considered:** silently opening the first non-tmux profile, or silently switching windows (the first draft). Both were rejected because the user didn't ask for either one.

### "tmux: New Session"

The palette's "New tmux session…" row runs the same flow with the picked profile, and opens the session where the action would have: the current window, or a new one.

- **The action.** `TmuxNewSession` (`tmux_new_session`) is labelled, with no default binding. It's handled in the `Workspace` context. The handler is registered in `render` only while the config has a control-mode profile, so the command palette's availability check hides it otherwise. It opens the workspace's prompt (`open_prompt`), labelled "New tmux session name".
- **Choosing the profile:** the active tab's session profile, else the default profile if it's control mode, else the first control-mode profile.
- **Attach spec.** `TmuxSession::attach` takes an `AttachSpec { Named | New(Option<String>) }`. `command_line` emits `new-session -A -s <name>` for `Named` (unchanged), and `new-session [-s <name>]` for `New`, plus `-c <cwd>` from the active tab. A `New` session starts with no known name.
- **The submit result.** Submitting returns a oneshot receiver. The workspace keeps its sender in `creating: HashMap<EntityId, Sender>`. The first tab added for that session sends `Ok`, which closes the prompt. `Failed(reason)` sends `Err(reason)` and skips the attach-failure tab, so the prompt shows tmux's message. If the user dismisses the prompt while waiting, the receiver is dropped and the session still attaches. That matches how `TmuxCommand` prompts behave.
- **Session names.**
  - The session's name becomes `Option<String>`.
  - `%session-changed` sets it, as `%session-renamed` already does. This also corrects names tmux altered on attach (tmux replaces `.` and `:`).
  - `snapshot()` records only sessions with a known name. A session created with an empty name and quit before attaching is lost, which is harmless.
- **Attach errors.** Before attaching, `handle_event` keeps the body of a `Response { ours: false, ok: false }` as `attach_error`. `on_detached` uses it as the `Failed` reason when it's present. This also improves the attach-failure tab for profile attaches.

### Profile palette detail

`open_profile_picker` appends ` · open in this window` or ` · in another window (switches)` to a control-mode profile's description, using `find_session` with the profile's key. That's a snapshot taken when the palette opens. Classic tmux profiles get no state, because Brindle can't know it.

For e2e, `Palette::describe` prints a row's detail as `label  detail  [shortcut]`. Existing parsers split rows on two spaces and take the first field, so they keep working.

### Restore removes duplicates before opening windows

`session::dedupe_windows(windows, config) -> Vec<WindowState>` is a pure function and unit-tested:
- it resolves each tmux entry's key through the config, the same way `resolve_tab` does;
- it drops entries whose key an earlier window (or the same window) already had;
- it drops windows left with no tabs;
- it moves `active` to keep pointing at the same entry, or to 0 if that entry was dropped.

`main.rs` applies it to `restorable_windows` before opening anything. Doing this at runtime in `restore` instead would open the window and then need to close it again.

### Debug output for other windows

`--dump-screen-after` dumps only the first window. It gains one line per *other* window: `--- window N tabs=["Shell", …]`, in creation order. With that line, e2e can check what ctrl-shift-n opened.

## Risks / Trade-offs

- **Wayland may still refuse the raise** if the compositor wants a serial with the token request. Mitigation: the tab is activated anyway, and it falls back to `activate_window`. Raising needs a manual check on GNOME, because e2e can't observe focus.
- **Reading another window's `Workspace` during an update.** `WindowHandle::read` fails for a window that is mid-update. The only one is our own, which is skipped. A failure is treated as "not open there", which at worst attaches a duplicate, the old behaviour.
- **A taken name when the new session opens in a new window.** For `target = Window`, the name prompt runs in the current window and checks the name with `tmux [args] has-session -t =<name>` (an exact match) before opening the window, so a taken name is reported in the prompt. The window opens with `AttachSpec::New`. If tmux still refuses (a race), the new window shows the ordinary attach-failure tab. An empty name skips the check.
- **Losing a session named by tmux.** A session created with an empty name and quit before `%session-changed` arrives isn't recorded for restore. It still exists in tmux.

