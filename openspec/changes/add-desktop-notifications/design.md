# Design

## Context

- **alacritty does not see OSC 9 or 777.** vte 0.15's `osc_dispatch` handles 0/2/4/8/10–12/50/52/104/110–112 and logs everything else as unhandled; `Handler` has no hook for unknown OSCs. So Brindle has to find these sequences in the byte stream itself.
- **Local tabs:** the PTY is read by alacritty's `EventLoop` on its own thread, straight into the `Term`. Brindle never sees the bytes. `EventLoop::new` is generic over `T: EventedPty + OnResize + Send`, and `EventedReadWrite::reader()` returns `&mut Self::Reader`, so the PTY can be wrapped.
- **Remote terminals** (tmux panes) get their bytes through `Terminal::feed`, on the UI thread.
- **tmux 3.6, checked with a raw `tmux -C` client:** `%output` for a pane in a *non-current* window carries `\033]777;notify;T;B\007`, `\033]9;Hi\007` and a bare `\007` as raw (octal-escaped) bytes. The control-mode notification list in `man tmux` has no `%bell`; tmux's bell handling (`bell-action`, `monitor-bell`) only drives alerts for terminal clients. Pane `Terminal`s already exist for every window of the session.
- **Bells today:** `Workspace::on_terminal_event` only handles local `TabContent::Terminal` tabs. Bells from tmux panes reach the pane `Terminal` and stop there.
- **Desktop:** the user's session is GNOME Shell 50.1 on Wayland, notification spec 1.2, capabilities `actions body body-markup icon-static persistence sound`. On Wayland, GPUI's `activate_window()` requests its own xdg-activation token without a recent input serial, which Mutter refuses (it shows "Brindle is ready" instead of raising). Spec 1.2 servers emit `ActivationToken(id, token)` just before `ActionInvoked`, and that token is the one a compositor accepts.
- **Dependencies already compiled through gpui** (`cargo tree -i`): `zbus` 5.12 with `async-io` (via ashpd/oo7), `wayland-backend` 0.3.11 with `client_system` and `dlopen`, `wayland-client` 0.31.11, `wayland-protocols` 0.31.2 with `client` and `staging` (gpui's own xdg-activation use), `raw-window-handle` 0.6.2 (gpui implements `HasWindowHandle`/`HasDisplayHandle` for `Window`), and `polling` 3.11 (alacritty's `EventedReadWrite` methods take its types, and alacritty doesn't re-export it).

## Goals / Non-Goals

**Goals:**
- One path for every source (bell, OSC 9, OSC 777) and every tab kind (local, tmux), from detection to the desktop.
- Nothing on the UI thread waits for D-Bus or the PTY thread.
- Clicking a notification really raises the window on GNOME Wayland, not just flashes it.
- Deterministic tests: pure scanner and policy units, and a file sink for e2e.

**Non-Goals:**
- OSC 99 (kitty's notification protocol), OSC 1337 (iTerm2), and ConEmu progress (`9;4`) display. They can build on the same scanner later.
- Notification sounds, icons per program, or a notification history UI.
- Classic tmux (`tmux = "plain"`): tmux swallows unknown OSCs from its panes. Programs must wrap them in tmux's DCS passthrough (`allow-passthrough on`), and tmux unwraps them before they reach Brindle. Bells are forwarded by tmux (`bell-action any`) as plain BEL, which Brindle already handles.
- A settings-dialog section for notifications (config file only for now).

## Decisions

### 1. Pre-parse OSC 9/777 with a streaming scanner beside vte

`notify::osc::Scanner` is a small byte state machine (`Ground → Esc → OscNum → OscBody → OscEsc`) that keeps its state between calls, so a sequence split across reads is found. It follows vte where that matters (other C0 bytes inside the payload are ignored; any ESC ends the OSC and starts a new escape) but is stricter about completion: only BEL or `ESC \` complete a notification. vte also dispatches an OSC ended by CAN, SUB or a stray ESC; Brindle treats those as cancelled, so a truncated or interrupted sequence never pops up a half message. It only collects OSCs whose number is `9` or `777`; every other OSC is skipped without buffering. Payloads over 4096 bytes are dropped. A BEL that terminates an OSC never counts as a bell, because bells only come from alacritty's own `Bell` event and alacritty makes the same distinction.

C1 introducers (0x9D) are not recognised: vte 0.15 runs in UTF-8 mode, where 0x9D is a continuation byte, so alacritty doesn't treat it as OSC either.

*Alternative:* fork or patch vte to surface unknown OSCs. Rejected: it means a vendored dependency for two sequences.

### 2. Where the scanner runs

- **Local tabs:** `ScanPty` wraps `tty::Pty`. Its `Reader` owns a `try_clone()` of the PTY file (the same open file description, so non-blocking mode and readiness are shared), and scans each chunk after reading it. Registration, writing, child events and resize delegate to the wrapped `Pty`. Notices go over the terminal's existing event channel, which now carries `enum Event { Alac(AlacEvent), Notice(Notice) }`, so they reach the UI thread through the task that already drains alacritty's events. The I/O thread does nothing but a channel send.
- **Remote terminals:** `Backend::Remote` holds a `Scanner` and `feed()` scans before advancing the processor, emitting `TerminalEvent::Notify` directly.

### 3. OSC 9 vs ConEmu sub-commands

If the payload's first `;`-separated field is an integer 1–12, it's a ConEmu command (`9;1` sleep, `9;2` message box, `9;3` tab title, `9;4` progress, `9;9` cwd, …) and is ignored. Shells emit `9;9` on every prompt and build tools emit `9;4` constantly, so both must never notify. `9;42` and `9;3 files copied` notify, because `42` is out of range and `3 files copied` isn't a bare number. This is the same rule Windows Terminal and Ghostty use. Recorded trade-off: a literal message `5` is swallowed.

### 4. OSC 777

Only `777;notify;<title>;<body>`. Everything after the second `;` is body, so bodies may contain `;`. Other sub-commands (`precmd`, `preexec`, `container`, …) are ignored. `777;notify;T` with no body is treated as a title-only notification with an empty body.

### 5. Text clean-up and notification layout

- Invalid UTF-8 is replaced lossily. Control characters are removed. Titles are cut to 200 characters, bodies to 1000, with `…`.
- **Title:** OSC 777 with a non-empty title is `"<title> — <tab title>"`; everything else is `"<tab title>"`. The program's own title comes first so it survives truncation in the popup.
- **Body:** the message; for bells, `Bell`.
- When the server advertises `body-markup` (GNOME does), `&`, `<` and `>` in the body are escaped so a program's text can't be read as markup.
- Hints: `desktop-entry = "brindle"` (matches the installed `brindle.desktop` and the window's app_id, so the server shows Brindle's name and icon) and `urgency = 1` (normal). `app_name = "Brindle"`, server default timeout.

### 6. Focus rule and settings

```toml
[notifications]
bell = true
osc9 = true
osc777 = true
when = "unfocused"   # or "always"
```

"Visible" means: the tab is its window's active tab **and** the window is focused (`window.is_window_active()`). This is the same test the existing bell indicator uses, so the dot and the notification agree. A window that is on screen but unfocused counts as not visible; the user is in another app, which is the main case notifications exist for. The default is `"unfocused"` as asked. Names: `bell`/`osc9`/`osc777` follow the sequences' common names (and what users will search for); `when` reads naturally as a sentence (`notify when unfocused`). Settings are read at the time of each event, so a reload applies at once.

### 7. Rate limiting

A token bucket per key, with an injected clock so tests are exact:

| bucket | capacity | refill |
|---|---|---|
| bells, per tab | 1 | 1 per 5 s |
| OSC 9/777, per tab | 3 | 1 per 2 s |
| everything, global | 10 | 1 per 2 s |

A notification is sent only when both its per-tab bucket and the global bucket have a token; neither is charged otherwise. Over-limit notifications are dropped (logged at debug level), not queued, because a queue would replay a storm later. Each tab's bell notification uses the D-Bus `replaces_id` of its previous one, so even permitted bells update a single popup instead of stacking. Buckets are keyed by tab id (globally unique already) and pruned when full. Numbers are constants, not settings: nobody tunes these, and fewer settings mean fewer ways to break it.

### 8. Transport: zbus, not `notify-send`/`gdbus`

`notify-send --action=default=Show --wait` can report a click, but needs one process per notification that stays alive until the notification is dismissed, can't replace a previous notification by our own id (`--replace-id` needs the id, which it prints only with `--print-id`), and never reports the activation token. `gdbus` can call `Notify` but can't wait for signals per notification without a long-lived `gdbus monitor` that we'd have to parse. zbus is already compiled into the binary, so a direct D-Bus client costs no new code downloads and gives all three: ids, `ActionInvoked`, and `ActivationToken`.

`zbus = { version = "5.12", default-features = false, features = ["async-io"] }` matches the locked version and the features already enabled. The service (`notify::Notifier`, a GPUI global) owns an unbounded command channel to one worker future on GPUI's background executor. The worker connects to the session bus lazily on the first notification (so a session without notifications never touches D-Bus), calls `GetCapabilities` once, and then serves `Notify`/`CloseNotification` calls while listening to `ActionInvoked`, `ActivationToken` and `NotificationClosed` through a `MatchRule` message stream. Results go back to the UI thread over another channel. If the bus or service is unavailable, the worker logs one warning, drops notifications, and retries the connection at most every 30 s.

### 9. Click-to-focus

The service remembers, for each notification id, the window handle and tab id it came from (pruned on `NotificationClosed` and capped at 256 entries). `actions = ["default", "Show"]` is sent only when the server has the `actions` capability. On `ActionInvoked(id, "default")`:

1. Activate the tab by id in that window (if it still exists).
2. Raise the window: on Wayland, with the token from the preceding `ActivationToken(id, token)`, by calling `xdg_activation_v1.activate(token, surface)` on GPUI's own `wl_surface`. Brindle gets the `wl_display` and `wl_surface` pointers through `raw-window-handle`, wraps the display with `wayland_backend::client::Backend::from_foreign_display` (its own event queue, so GPUI's queue is untouched), binds `xdg_activation_v1` from a fresh registry, and wraps the surface with `ObjectId::from_ptr`. This happens immediately on the UI thread inside the window update, because tokens are short-lived and the surface must not be destroyed mid-call; it costs one registry round trip to the local compositor.
3. Otherwise (X11, no token, or any failure in step 2) call `window.activate_window()`. Failures are logged and fall back; they never panic.

*Alternative:* omit the default action. GNOME then raises the app itself (via `desktop-entry`), but can't know which tab, and other servers do nothing. Rejected.

### 10. Not withdrawing notifications on focus

GNOME Console withdraws a tab's notifications when it gets focus. Brindle doesn't, for now: the notification server already removes a clicked notification, `replaces_id` keeps bells to one per tab, and leaving history in the tray is harmless. It can be added later through the same command channel.

### 11. tmux control mode

No tmux protocol work is needed: pane output already flows through `Terminal::feed`, which now scans for OSC 9/777, and BEL reaches alacritty's `Bell` event. `TmuxWindowView` subscribes to each pane terminal and re-emits `Bell` and `Notify` (it becomes an `EventEmitter<TerminalEvent>` alongside `PromptRequest`). The workspace subscribes to each tmux tab's view, so tmux tabs get the bell dot and notifications through the same handler as local tabs, keyed by the tab. A pane in a background tmux window is a background tab. A bell that arrives in the brief window while a pane is being restored (its `%output` is dropped until the capture lands) is lost; restores are sub-second and only happen on attach.

### 12. Test sink: `BRINDLE_NOTIFY_LOG`

When set, the service's worker is replaced by a file sink: each notification is one line `source\ttitle\tbody\n` (tabs and newlines inside fields become spaces). The line is appended **synchronously** on the UI thread, before the notification counts as sent, because `--dump-screen-after` quits the app right after printing and a background write could be lost. An `O_APPEND` write of one short line to a local file is not the kind of blocking the "never block the UI" rule is about (D-Bus round trips, process spawns); it only happens in tests. Ids are a counter, so the click-map and replacement logic run the same way. Rate limits and the focus rule apply before the sink, so e2e cases exercise them.

## Risks / Trade-offs

- [The `ScanPty` dup'd reader sees the same bytes as alacritty would] → it *is* the reader alacritty uses; the original file is only used for registration and writes, so there is no double read.
- [The scanner costs CPU on every PTY byte] → in `Ground` it jumps from ESC to ESC with `position()`, and it never copies output that isn't inside an OSC 9/777.
- [Wayland foreign-display activation is unusual] → it is confined to one function with a fallback to `activate_window()`, and it only runs on a click. If it fails on some compositor the window still gets GPUI's normal activation request.
- [A message `5` via OSC 9 is ignored as a ConEmu command] → documented; the same choice as other terminals.
- [Click map grows] → capped and pruned on `NotificationClosed`.
- [Notification servers without actions] → no click behaviour, which the spec allows; everything else works.
