# Proposal

## Why

tmux up to 3.6 can segfault when a control client with format subscriptions loses its session. The subscription timer keeps running and `control_check_subs_timer` reads through a NULL session. `unsubscribe-before-tmux-detach` removes Brindle's subscriptions before every exit that Brindle starts. A session that ends from inside tmux is still exposed, because tmux clears the session before Brindle can react. Examples are the last shell exiting, or another client running `kill-session`. If that crash happens, it takes down every session on the user's server. tmux master has fixed it (`monitor_timer` returns without a session), but no release has the fix yet.

## What Changes

- Read tmux's version on attach (`display-message -p '#{version}'`).
- On tmux 3.6 and older, don't register format subscriptions. Brindle asks for the same values itself instead, so tmux has no subscription timer to crash:
  - a `list-panes -a -F` with the geometry, title, pane-style and border-style formats;
  - a `list-windows -F` with the border status and border styles;
  - roughly once a second while attached, and right after Brindle's own commands that may change them.
- Changes Brindle learns this way are handled as subscription values are today, including re-snapshotting panes whose size changed.
- On newer tmux, keep using subscriptions.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `tmux-control-mode`: "Subscriptions are removed before leaving a session" becomes conditional on Brindle using subscriptions. A requirement is added that on tmux versions with the subscription crash, Brindle uses no control-mode subscriptions.

## Impact

- **Code:** `src/tmux/mod.rs` gets the version check, a poll timer, and routing of poll replies into the existing handlers. Subscriptions report only changes; the poll gives full values, so unchanged values must be skipped.
- **Cost:** one or two small commands a second per attached session on old tmux.
- **Tests:**
  - an end-to-end case that ends a session from inside tmux (`exit` in the last shell) many times and checks the server survives;
  - the existing cases, run against both paths. The suite could force polling with an environment variable.
- **Open questions for design:**
  - which tmux release first contains the upstream fix, which sets the version cut-off;
  - whether polling should be the only path, for simplicity.
