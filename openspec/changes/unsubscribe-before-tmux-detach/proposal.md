# Proposal

## Why

tmux 3.6 can crash its whole server, losing every session in it, when a control client with format subscriptions leaves. tmux clears the client's session when the client starts exiting (`MSG_EXITING`) or when its session is destroyed. The client's 1-second subscription timer stays armed until the connection closes, and `control_check_subs_all_panes` then reads through the NULL session. The end-to-end suite hit this once (`segfault at f0` in `tmux: server`). The core dump places the fault in `control_check_subs_timer` at a load of `c->session`. tmux master has fixed it (`monitor_timer` returns when there is no session), but no release has the fix.

Since `match-tmux-pane-geometry`, `show-tmux-pane-titles` and `apply-tmux-pane-styles`, Brindle subscribes on every attach. So every detach, every quit of Brindle, and every session Brindle ends now has a small chance of crashing the user's tmux server. Removing a client's last subscription deletes the timer in 3.6 (`control_remove_sub` calls `evtimer_del`), which closes that window.

## What Changes

- Before Brindle leaves a session, it removes its subscriptions with `refresh-client -B <name>`, which takes no format. This covers:
  - the user detaching (ctrl-shift-d);
  - a tmux tab's session being dropped, for example when its last tab closes;
  - Brindle quitting.
- On quit, Brindle registers an app-quit hook that does the same and gives the command writer a moment to send it before the process exits.
- Before Brindle ends a session itself, by closing the session's last window or killing its last pane, it removes its subscriptions first.
- A session that ends from inside tmux, such as the last shell exiting or another client killing it, can't be anticipated. Only tmux newer than 3.6 closes that window. This is recorded as a known limitation.
- A regression case in the end-to-end suite checks from Brindle's debug log that the unsubscribe commands go out before Brindle detaches, quits or ends a session. It also checks that the throwaway server survives repeated attach and quit cycles.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `tmux-control-mode`: a new requirement that Brindle removes its control-mode subscriptions before it detaches, quits or ends a session.

## Impact

- **Code:** `src/tmux/mod.rs`:
  - a table of the subscriptions, used both to subscribe and to unsubscribe;
  - `release()`, called by `detach`, `Drop` and an app-quit hook;
  - unsubscribing before killing a session's last window or pane.
- **Tests:**
  - unit tests for the subscribe and unsubscribe command lines;
  - the `tmux-subscriptions-removed` end-to-end case, which the `add-e2e-test-harness` change, still being applied, picks up.
- **Dependencies:** none.
