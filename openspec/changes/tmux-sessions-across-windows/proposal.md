# Proposal

## Why

When the default profile is a control-mode tmux profile, pressing ctrl-shift-n opens a second window that attaches to the same session again. Each window then mirrors every tmux window of that session as tabs. That is never what a new window is for. The check that stops a session from being attached twice only looks at the current window. There is also no way to start a new tmux session on demand: picking a profile always reattaches its one named session.

## What Changes

- **One window per session.** A control-mode session that is open in any Brindle window is never attached again. If its profile is picked (profile palette, ctrl-alt-N) while the session is open in the current window, its tab becomes active, as today.
- **Ask instead of switching windows or swapping profiles.** Sometimes an action would attach a session that another window shows: picking the profile, or opening the default profile with ctrl-shift-t, `+` or ctrl-shift-n. The current window then shows a palette with these rows: "Go to <session>", "New tmux session…", and "New tab: <profile>" for the other profiles ("New window: …" for ctrl-shift-n). Escape cancels. Brindle never switches windows, or opens a profile the user didn't ask for, without asking. The ask palette for a session ending lists such a profile as "Go to <session>" too.
- **"tmux: New Session".** A new command prompts for a session name, then creates and attaches that session in the current window. Leaving the name empty lets tmux name the session. It uses the active tab's control-mode profile (socket and arguments), or else the default profile if that's control mode, or else the first control-mode profile. If tmux refuses, for example because the name is taken, the prompt stays open and shows tmux's error.
- **Live session state in the profile palette.** Each control-mode profile's detail line also says whether its session is open now: "open in this window" or "open in another window".
- **Restore attaches each session once.** If a state file records the same session in two windows (which the old mirroring could produce), only the first window attaches it. A window left with nothing to restore isn't opened.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `tmux-control-mode`:
  - "Attach to a session" covers every window, not just the current one.
  - Adds "Choosing when a session is open elsewhere" (the palette) and the "tmux: New Session" command.
  - "Asking when a session leaves" lists sessions open elsewhere as "Go to <session>".
- `tabs`: "Multiple windows" asks instead of attaching again when the default profile's session is already open. A new requirement makes ctrl-shift-t ask rather than switch windows.
- `profiles`: "Profile picker" shows whether each control-mode profile's session is open. "Default profile" points to the palette.
- `command-palette`: "tmux: New Session" is listed in every tab, not only control-mode tabs.
- `session-restore`: "Control-mode sessions are reattached once" applies across all windows.

## Impact

- `src/workspace.rs`:
  - `attach_tmux` searches every window's sessions.
  - `new_tab` and the profile palette rows change.
  - Adds the open-elsewhere palette, the new-session prompt, and focusing another window's tab.
- `src/main.rs`: the `NewWindow` handler asks the current window first when the default session is open. The restore path drops duplicate sessions before opening windows.
- `src/tmux/mod.rs`:
  - A session can be started without `-A`, and without `-s` for a name tmux chooses.
  - `%session-changed` sets the session's name.
  - An error before attaching becomes the failure reason.
- `src/session.rs`: a pure function that removes duplicate sessions from a restored state.
- `src/actions.rs`: the `tmux_new_session` action, labelled "tmux: New Session".
- README, unit tests, and e2e cases.
- Out of scope: a second Brindle *process* (for example, one started from the app launcher) still can't see the first process's windows and attaches as before.
