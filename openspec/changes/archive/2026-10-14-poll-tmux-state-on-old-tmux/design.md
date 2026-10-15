# Design

## Context

- `TmuxSession` (`src/tmux/mod.rs`) sends `list-windows` when it starts. On the first window list (`on_window_list`) it registers the five subscriptions in `SUBSCRIPTIONS`. Their changes arrive as `Notification::SubscriptionChanged { name, window, pane, value }` and are handled in one match in `on_notification`.
- tmux evaluates subscriptions once a second, and only sends values that changed since it last sent them. The first evaluation sends every value.
- Replies are matched FIFO against `Io.pending`. `Pending` says what each reply is for.
- `release`, `kill_window` and `kill_pane` call `unsubscribe`, which does nothing unless `subscribed` is set.
- The crash is fixed in tmux 3.7 (`control_check_subs_timer` returns when `c->session` is NULL). 3.8 moved the timer into `monitor.c` and kept the check. 3.6, 3.6a and 3.6b lack it.
- `#{version}` is `3.6`, `3.6a` and so on for releases. Builds from git print `next-3.9`. OpenBSD's base tmux prints `openbsd-7.x`.
- On tmux 3.6, `list-panes -s -F` in a control client lists the attached session's panes, with `#{T:…}` and option names such as `#{pane-border-status}` expanded per pane. This was checked against a throwaway server.

## Goals / Non-Goals

**Goals:**
- On tmux before 3.7, Brindle never registers a subscription, so tmux has no subscription timer for a client.
- The handlers for geometry, titles, pane styles, border status and border styles see the same stream of changes from either path.

**Non-Goals:**
- Telling the user that their tmux is old, or offering a setting. The fallback is automatic and invisible.
- `top-floating` and `bottom-floating` border status (new in 3.8). These are parsed as `off`, as they are today.
- Older tmux that has no `refresh-client -B` (before 3.2). Polling happens to help it too, but nothing here is tested there.

## Decisions

1. **Ask for the version first.** `attach` sends `display-message -p "#{version}"` (`Pending::Version`) before `list-windows`, so the reply is in hand by the first window list. A pure `subscriptions_safe(version) -> bool`, with unit tests, parses it:
   - `X.Y`, with an optional letter, counts as that release.
   - `next-X.Y` counts as a build just before X.Y, so `next-3.7` polls and `next-3.8` subscribes.
   - Anything else polls, including OpenBSD's own versions and an error reply.
   - Alternative rejected: trying to detect the crash, or checking whether `refresh-client -B` is supported. Neither tells us whether the timer is safe.
2. **`BRINDLE_TMUX_STATE=poll|subscribe` overrides the check.** It is a debug hook, like `BRINDLE_NOTIFY_LOG`. The e2e harness inherits the variable from its own environment, so `BRINDLE_TMUX_STATE=subscribe BRINDLE_E2E=1 cargo test --test e2e` runs the whole suite on the subscription path, even on 3.6. Cases that are about one path set it explicitly.
3. **One poll command, one line per pane.** `list-panes -s -F` with `#{window_id}`, `#{pane_id}` and `#{pane_active}`, then each subscription's format, tab-separated. The format comes from the `SUBSCRIPTIONS` table, so the two paths can't drift apart.
   - The title (`#{T:pane-border-format}`) is free text, so it goes last in the table and the line is split with `splitn`.
   - Window-scoped values (border status, border styles) are taken from the window's active pane's line (`#{pane_active}` = 1). That matches tmux's `@*` subscriptions, which expand window formats against the active pane.
   - Alternative rejected: a separate `list-windows` for window values. It means two commands and two replies to correlate, with no gain.
   - `-s`, not the proposal's `-a`: `-a` would list every session on the server.
4. **Poll replies become `SubscriptionChanged` notifications.** A pure `PollState` keeps the last value seen for each (subscription, window or pane). It turns a reply into the notifications whose value changed, and `on_notification` handles those exactly as it handles tmux's.
   - A value is recorded only once Brindle knows its pane or window, so a value for a pane that is still being created is sent again on the next poll and not lost.
   - Entries for panes and windows that are gone are dropped in `sync_panes`.
   - Alternative rejected: calling the handlers on every poll. `apply_geometry` lists the windows again on a mismatch, and title and style updates call `cx.notify()`, which re-renders the whole window in GPUI 0.2.2.
5. **The poll runs on a timer, one request at a time.** When the session attaches on the poll path, a `cx.spawn` loop held in the session asks for a poll every second.
   - A `polling` flag keeps at most one `list-panes` in flight. A request made while one is in flight sets `poll_again`, and the poll is sent once the reply arrives.
   - An immediate poll is requested after a user command's sentinel reply, because `set -w pane-border-status top` and similar are typed there. Other changes are caught by the next tick, which meets the specs' "about a second".
   - `release` and `on_detached` drop the task and clear `polling` and `poll_again`. `poll` does nothing once the session is released or detached, so nothing is sent to a session being left.
   - A `Pending::Poll` reply that is an error, or that arrives after `release` (a session ended by `kill-window` with a poll in flight), is dropped without the warning other failed commands log.
6. **Leaving is unchanged.** `subscribed` stays false on the poll path, so `unsubscribe` already does nothing. `release` still sends `detach-client`.

## Risks / Trade-offs

- [A session ended from inside tmux on 3.6 still crashes tmux when the override forces subscriptions] → The override is a debug hook. The suite sets `subscribe` only in cases that don't end the session from inside tmux.
- [Busy sessions cost more] → One reply a second, about 100 bytes per pane, all of it parsed and compared. tmux does the same format work for subscriptions.
- [A title containing a tab, or a pane format that expands to a newline] → The title is split last. A newline breaks a subscription value in the same way.
- [`next-X.Y` builds from before the fix was merged, between 3.6 and 3.7] → They count as below 3.7 and poll.
- [Distribution tmux with the fix backported] → It polls anyway, which works, just less efficiently.

## Migration Plan

None. The behaviour is picked per session when it attaches.
