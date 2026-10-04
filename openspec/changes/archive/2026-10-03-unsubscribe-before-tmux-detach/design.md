# Design

## Context

- `TmuxSession` registers five subscriptions on its first window list: geometry, titles, border status, pane styles and border styles. Each is a `refresh-client -B name:scope:format` sent as `Pending::Ignore`.
- Commands go through `Io`, a channel to a writer thread that owns the `tmux -C` child's stdin.
- `detach()` sends `detach-client`. `Drop` sends `detach-client` and kills the child 500 ms later from another thread.
- On quit, GPUI's `App::shutdown` runs the quit observers, clears the windows (which releases `TmuxSession`, so `Drop` runs), and then waits up to `SHUTDOWN_TIMEOUT` (100 ms) for the observers' futures. Nothing else waits for the writer thread, so the process can exit before it writes. tmux then sees stdin close, which is the crash-prone exit path.
- In tmux 3.6, `refresh-client -B name` with no colon removes a subscription, and removing the last one deletes the timer.

## Goals / Non-Goals

**Goals:**
- No Brindle-initiated way of leaving a session leaves the subscription timer armed.

**Non-Goals:**
- Sessions ended from inside tmux (the last shell exiting, `kill-session` from another client). Brindle learns of these only after tmux has cleared the session. Only a tmux release with the upstream fix closes this.

## Decisions

1. **One table of subscriptions.** `SUBSCRIPTIONS: [(name, scope, format); 5]` drives both `subscribe_commands()` and `unsubscribe_commands()`. Pure functions, unit-tested, so a subscription can't be added without its removal.
2. **`release()` is idempotent.** It sends the unsubscribe commands, then `detach-client`, once; a flag stops repeats. It is called by `detach()`, by `Drop` (before the delayed kill), and by an app-quit hook. Sending is a no-op once the session has detached.
3. **The quit hook waits for the writer.** `cx.on_app_quit` (registered in `attach`, held in the session) calls `release()`. Its future then waits on a flush acknowledgement from the writer thread, with a 50 ms cap that keeps it under GPUI's 100 ms shutdown timeout. The acknowledgement is a sentinel the writer answers through a oneshot channel once everything before it is written. In normal shutdown the session is released after the observers run, so this hook is what guarantees the commands reach tmux before stdin closes.
   - Rejected alternative: a fixed sleep. It works, but the acknowledgement costs the same amount of code and doesn't guess.
4. **Ending a session.**
   - `kill_window` checks whether the window is the session's only one, and `kill_pane` whether it's the only pane of the only window.
   - If so, it sends the unsubscribe commands first; nothing re-subscribes, because the session is ending.
   - If the kill fails, for example because tmux refuses it, the session stays attached without subscriptions until the next attach. Titles, styles and geometry then stop updating, which is acceptable for a failure that shouldn't happen.

## Risks / Trade-offs

- [A session ended from inside tmux can still crash tmux 3.6] → The window is the short time between tmux clearing the session and the client connection closing. This is recorded in `CLAUDE.md`'s tmux notes, with the upstream fix as the remedy.
- [The quit hook adds up to 50 ms to quitting] → Only while a tmux session is attached.
