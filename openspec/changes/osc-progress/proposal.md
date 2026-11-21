# Proposal

## Why

Long-running programs (winget, recent cargo, some systemd tools, scripts written for Windows Terminal) report progress with ConEmu's `OSC 9 ; 4`. Windows Terminal, Ghostty and ConEmu show it. Brindle currently drops it on purpose (`parse_osc9` treats `9;4` as a ConEmu command), so a build or a download in a background tab gives no sign of how far along it is.

## What Changes

- Recognise `ESC ] 9 ; 4 ; <state> [; <percent>] ST` in every terminal, local or remote: clear (0), normal (1), error (2), indeterminate (3) and paused (4).
- Keep one progress value per terminal (tab or zone), cleared by a `0` report, a new shell prompt (OSC 133 `A`), the program exiting or a full reset.
- Draw the progress as a thin bar along the bottom edge of the tab's pill, and along the bottom of a zone's header in a multi-zone tab, coloured by state.
- Keep the progress in the terminal server, so a detached or reconnected session shows it again after attach.
- Print it in `--dump-screen-after` (`--- progress …`), and add e2e cases.
- `OSC 9 ; 4` still raises no notification, and `9;1`–`9;12` other than 4 and 9 stay ignored.

Out of scope: the Windows taskbar button's progress (`ITaskbarList3`) and the Linux launcher progress (`com.canonical.Unity.LauncherEntry`), which can follow on top of the same per-terminal value; any change to OSC 7501's own `progress` key.

## Capabilities

### New Capabilities
- `program-progress`: recognising `OSC 9 ; 4` progress reports, the per-terminal progress value and its lifetime.

### Modified Capabilities
- `tabs`: a tab shows its terminals' progress as a bar on its pill.
- `zones`: a zone's header shows that zone's progress in a multi-zone tab.
- `terminal-server`: a session's progress survives detach, attach and core restarts.

## Impact

- `term-core/src/osc.rs` (a new `Found::Progress`), a new pure `term-core/src/progress.rs`, `term-core/src/emulator.rs` and `term-core/src/replay.rs`.
- `src/terminal/mod.rs` (the terminal's progress, its lifetimes, the event to `Workspace`), `src/workspace.rs` and `src/zone_view.rs` (drawing), the dump in `src/main.rs`'s debug hooks.
- `tests/e2e/cases/` (a new `progress` case set), `docs/code-map.md`, README.md.
- No wire change: the replay is a byte stream, and old clients already ignore `9;4`. No new dependency.
