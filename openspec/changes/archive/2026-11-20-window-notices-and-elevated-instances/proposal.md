## Why

Checking the 2026-11-20 security fixes by hand on Windows turned up two problems.

- **Link failures write into the terminal.** When the user ctrl-clicks a link Brindle won't open (`ms-settings:`, a `file:` link to another host) or whose opener fails, Brindle writes a dim line into the terminal. That line is mixed in with the program's output, scrolls with it and stays in the scrollback, though it's about Brindle, not the program.
- **The second Brindle fails silently.**
  - With an elevated Brindle running, starting an unelevated one does nothing visible.
  - Since #104, a launch refuses to hand itself to an instance at another integrity level. The refusal is right, but it's printed only on stderr, which a launch from the Start Menu doesn't show.
  - The two Brindles also share one state directory, so only one of them can ever be the instance.

## What Changes

- **Link failures go to the config line.** A link Brindle doesn't open, or whose opener fails, is reported as a window notice on the config line instead of a line in the terminal. The config line already shows config errors and other window notices. The notice names the link and the cause. It can be dismissed, and it goes away by itself after 10 seconds.
- **Elevated Brindle gets its own state directory.** An elevated Brindle on Windows keeps its state in `%LOCALAPPDATA%\brindle\state-elevated`, as the terminal server already uses `run-elevated`. It is therefore a separate instance with its own saved session, so elevated and unelevated Brindles run side by side, and each launch goes to the instance at its own level.
  - **BREAKING** for elevated users: an elevated Brindle no longer restores the session it saved in the shared directory before this change. Its sessions still detached in the elevated server are reattached as tabs.
- **A failed hand-off is visible.** A launch that can't be handed to the running instance (another user serves the pipe, or no answer within 5 s) still prints why. On Windows, when the launch has no console, it also shows why in a message box.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `links`: where a refused or failed open is reported.
- `configuration`: the config line also carries transient window notices, which can be dismissed and expire.
- `command-line`: an elevated launch no longer meets an unelevated instance, and a failed hand-off is visible without a console.
- `session-restore`: the elevated state directory on Windows.

## Impact

- **Code:**
  - Link failures: `src/terminal_view.rs` reports a link failure to the workspace instead of calling `Terminal::note`. `src/workspace.rs` and the new `src/workspace/notices.rs` handle the notice, its dismissal and expiry, and `src/settings.rs` passes the notices to `config_line`.
  - Elevated state and hand-off: `src/session.rs` (`state_dir` takes the elevation), `src/main.rs`, `src/instance.rs`, and `src/win.rs` for the message box.
- **e2e:**
  - The `links` cases expect the refusal on the dump's `--- config` line, not in the screen text.
  - A new Windows case runs two instances at different levels. It needs an elevated runner and skips otherwise.
- **Unchanged:** no wire, protocol or server change.
