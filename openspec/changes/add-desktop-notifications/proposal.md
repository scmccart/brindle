# Proposal

## Why

Long-running commands, builds and agents (Claude Code among them) finish in a tab the user isn't looking at, and today the only signal is a small bell dot on the tab strip, which is invisible when the window itself is in the background. Other terminals turn OSC 9 and OSC 777 into desktop notifications, and many tools already emit them. Brindle drops both sequences, and bells from tmux control-mode panes don't even mark their tab.

## What Changes

- **Desktop notifications from programs.** `OSC 9 ; message` (iTerm2/ConEmu style) and `OSC 777 ; notify ; title ; body` (rxvt/Ghostty/foot style) raise a freedesktop desktop notification. ConEmu's numbered `OSC 9 ; N ; …` sub-commands (progress `9;4`, working directory `9;9`, and the rest) are not messages and never notify. Sequences split across reads are recognised.
- **Bells notify too.** A bell in a background tab, or in any tab while its window is unfocused, raises a notification in addition to the existing bell dot.
- **Which tab.** The notification's title names the tab it came from. Clicking the notification raises the window and activates that tab, when the notification server supports actions.
- **Config.** A new `[notifications]` table turns each source on or off (`bell`, `osc9`, `osc777`) and picks `when`: `"unfocused"` (default; only when the tab isn't in front of the user) or `"always"`.
- **Rate limiting.** A bell storm or a program printing notifications in a loop cannot flood the desktop: bells notify at most once per tab every few seconds, program notifications have a small per-tab burst, and there is a global cap. A tab's newer bell notification replaces its older one instead of stacking.
- **tmux control mode.** Bells and OSC 9/777 from tmux panes, including panes in background tmux windows, behave exactly as in local tabs, and bells from tmux panes now set the tab's bell dot.
- **Test hook.** `BRINDLE_NOTIFY_LOG=<file>` writes notifications as lines to a file instead of sending them to the desktop, for the end-to-end suite.

## Capabilities

### New Capabilities

- `desktop-notifications`: which terminal events raise desktop notifications (OSC 9, OSC 777, bells), when they are suppressed (focus rule, per-source switches, rate limits), what a notification contains, what clicking it does, and the file sink used by tests.

### Modified Capabilities

- `tabs`: the Bell indicator requirement now covers bells from tmux control-mode panes, which until now never marked their tab.
- `tmux-control-mode`: adds a requirement that bells and notification sequences from panes reach Brindle as they would from a local program, for every window of the session.
- `command-line`: adds the `BRINDLE_NOTIFY_LOG` debug hook next to the other automation hooks.

## Impact

- **Code:**
  - a new `src/notify/` module: the streaming OSC scanner, the focus/rate-limit policy, the notification service (a GPUI global) with its D-Bus and file sinks, and window raising with an activation token;
  - `terminal/mod.rs`: a scanning wrapper around the PTY reader (alacritty's I/O thread), scanning of fed bytes for remote terminals, and a new `TerminalEvent::Notify`;
  - `tmux_view.rs`: forwards bells and notifications from its pane terminals;
  - `workspace.rs`: one handler for bells and notifications from local and tmux tabs, and focusing a tab by id;
  - `config.rs` and `assets/default-config.toml`: the `[notifications]` table;
  - `main.rs`: installs the notification service.
- **Dependencies:** `zbus` 5.12, `raw-window-handle` 0.6, `wayland-client` 0.31, `wayland-backend` 0.3 and `wayland-protocols` 0.31. All are already in `Cargo.lock` through gpui, with the needed features already enabled, so the lock file only gains entries in Brindle's own dependency list.
- **Specs:** a new `desktop-notifications` spec and deltas to `tabs`, `tmux-control-mode` and `command-line`.
- **Tests:** unit tests for OSC scanning (split reads, terminators, ConEmu sub-commands, OSC 777 forms), the focus rule and the rate limiter; end-to-end cases for OSC notifications in a local tab, background-tab bells and notifications in tmux control mode, and a headless check that tmux forwards these sequences in `%output`.
