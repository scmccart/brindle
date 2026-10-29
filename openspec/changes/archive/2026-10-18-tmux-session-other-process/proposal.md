# Proposal

## Why

Launching Brindle a second time, from a launcher or a terminal, starts a second process. That process knows nothing about the first one's windows, so its default control-mode profile attaches the session the first one already shows. tmux keeps one current window per session for all its clients, so the two windows mirror each other: a new tmux window, a split or a window switch in one happens in the other too. Within one process "A session SHALL be shown by at most one Brindle window" already holds (`find_session`, "Choosing when a session is open elsewhere"); across processes nothing checks.

The new `multi-process-one-client` and `multi-process-independent` e2e cases reproduce it: two control clients on the session, and the first window gains the second's tmux window and switches to it.

## What Changes

- Brindle's control clients carry a marker in their environment that names the Brindle process that started them.
- Before attaching a session by name, Brindle asks tmux for the session's clients. A client counts as another Brindle when its marker names its parent process and that isn't this Brindle. Other programs' clients, and `tmux -C` started from a shell (which may have inherited the marker), don't count. When tmux can't list the clients (no server, no such session), Brindle attaches as before.
- When another Brindle shows the session, Brindle doesn't attach it. The window that would have shown it asks instead, with `tmux session "<session>" is open in another Brindle`: "New tmux session…", then "New tab: <profile>" for the other profiles. There is no "Go to" row, because Brindle can't bring up another process's window.
- This covers every way a session gets attached by name: at launch, ctrl-alt-N, ctrl-shift-t and the `+` button, ctrl-shift-n (the new window opens, then asks), the new-tab palette, session restore, and the palette shown after a session ended.
- A window with no tabs, such as the first window of a second launch, closes when that palette, or the prompt that its "New tmux session…" opens, is dismissed. After a session ended, its ended tabs close once the user picks a row or dismisses the palette, as they do on the session-ended palette.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `tmux-control-mode`: "Attach to a session" now holds across Brindle processes, and a new requirement, "Session shown by another Brindle", says how Brindle detects that and what it asks.
- `tabs`: "Multiple windows" says that ctrl-shift-n checks only this Brindle's windows before opening, and that a session another Brindle shows is asked about in the new window.

## Impact

- `src/tmux/mod.rs`: the client marker, the `list-clients` check before spawning a named attach (spawning moves after the check), a new `TmuxEvent`, and unit tests for picking out other Brindles' clients.
- `src/workspace.rs`: handling the new event, a third `SessionLocation` for another process, rows without "Go to", closing a window left without tabs, and ending a replaced session's tabs.
- `tests/e2e/cases/multi_window.rs`: the two failing cases also check the second Brindle's palette, plus cases for dismissing it and for a plain `tmux -C` client not counting.
- No new dependencies. Reading `/proc` is Linux-only, as Brindle is.
