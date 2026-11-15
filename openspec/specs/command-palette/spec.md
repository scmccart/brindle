# command-palette Specification

## Purpose
Make every Brindle command discoverable and runnable by name from a searchable list over the active tab. A command that needs text, such as a new window name, asks for it as a second step.

## Requirements

### Requirement: Opening the command palette
ctrl-shift-p SHALL open the command palette over the active tab. Pressing it again while the palette is open SHALL close it. Only one palette SHALL be open at a time: opening the command palette or the new-tab palette SHALL close whichever one is already open.

#### Scenario: Toggle
- **WHEN** the user presses ctrl-shift-p twice
- **THEN** the palette opens and then closes, and keyboard focus returns to the tab

#### Scenario: Switching palettes
- **WHEN** the new-tab palette is open and the user presses ctrl-shift-p
- **THEN** the new-tab palette closes and the command palette opens

### Requirement: Shortcuts shown
Each listed command that has a keybinding in the focused tab SHALL show that binding, including the user's own overrides. A command without a binding SHALL show none.

#### Scenario: User rebinding
- **WHEN** the config binds `ctrl-alt-r` to `reload_config` and the user opens the palette
- **THEN** "Reload Config" is shown with `ctrl-alt-r`

### Requirement: Filtering and navigation
Typed text SHALL filter the list by a case-insensitive subsequence match on the command name, keeping the order. The text SHALL be edited with a text input, as defined by the `text-input` capability. The palette SHALL be operable with up/down (also ctrl-p / ctrl-n) to move the selection, wrapping from the last row to the first and back, enter to run the selected command, and escape or a click outside to dismiss. Home and end SHALL move the filter's caret, not the selection. Clicking a command SHALL run it.

#### Scenario: Filter and run
- **WHEN** the user opens the palette, types `clrsc` and presses enter
- **THEN** "Clear Scrollback" runs in the active tab

#### Scenario: No match
- **WHEN** the typed text matches no command
- **THEN** the palette says nothing matches, and enter does nothing

#### Scenario: Fix a typo in the filter
- **WHEN** the user types `splt`, presses left once and types `i`
- **THEN** the filter reads `split` and the list shows the commands matching it

#### Scenario: Wrapping
- **WHEN** the last listed command is selected and the user presses down
- **THEN** the first listed command is selected, and up from the first selects the last

#### Scenario: Home and end edit the filter
- **WHEN** the filter holds `split` and the user presses home and then types `x`
- **THEN** the filter reads `xsplit` and the selection is unchanged

### Requirement: Running a command
Running a command SHALL close the palette, return keyboard focus to the tab, and act exactly as the command's keybinding would in that tab.

#### Scenario: Same as the shortcut
- **WHEN** the user runs "Split Zone Right" from the palette in a tab
- **THEN** the active zone is split, as with its shortcut, and focus moves to the new zone

### Requirement: Sessions palette
The `Attach Session…` command (Linux and Windows) SHALL open a palette listing the detached sessions of the local terminal server and of every remote endpoint Brindle is connected to. The rows SHALL be grouped by endpoint, local first, then remote endpoints in profile order. Within a group they SHALL be newest first. Each row SHALL show the session's title (else its command), its working directory and how long ago it started. Each endpoint Brindle is connecting to, or that is offline, SHALL be listed as a row saying so. An offline endpoint's row SHALL say, on its one line, why its last attempt failed, with the SSH or container engine's own error when there was one. The palette SHALL open without waiting for an endpoint that is slow to answer. Until such an endpoint answers, it SHALL be listed as a row saying its sessions are being listed. Typed text SHALL filter the rows as in the command palette. Once every endpoint has answered, with no detached sessions anywhere the palette SHALL say so, after any endpoint's row. When the rows change while the palette is open, the filter text SHALL be kept, and the highlighted session SHALL stay highlighted while it is listed. Sessions attached to any client SHALL NOT be listed.

#### Scenario: Nothing detached
- **WHEN** every session is attached and the user runs "Attach Session…"
- **THEN** the palette says there are no detached sessions

#### Scenario: Detached Windows session
- **WHEN** on Windows a `--no-restore` launch left a PowerShell session detached, and the user runs "Attach Session…"
- **THEN** the palette lists that session with its title and directory

#### Scenario: Remote host's sessions
- **WHEN** Brindle is connected to `A8`, which has a detached session titled `build`, and the user runs "Attach Session…"
- **THEN** the palette lists `build` under `A8`, after the local sessions

#### Scenario: Offline host
- **WHEN** profile `A8` connects at launch, `A8` can't be reached, and the user runs "Attach Session…"
- **THEN** the palette has a row saying `A8` is offline, and choosing it opens nothing

#### Scenario: Stopped container says why
- **WHEN** a container profile connects at launch, its container is stopped, and the user runs "Attach Session…"
- **THEN** the container's row says it is offline because the container is not running

#### Scenario: Slow host
- **WHEN** Brindle is connected to `A8`, `A8`'s link has gone silent but Brindle hasn't noticed yet, and the user runs "Attach Session…"
- **THEN** the palette opens at once with this machine's sessions and a row saying `A8`'s sessions are being listed, and that row shows `A8` as offline once its listing gives up

#### Scenario: No sessions note comes last
- **WHEN** nothing is detached on this machine, profile `A8` connects at launch, `A8` can't be reached, and the user runs "Attach Session…"
- **THEN** the palette's first row says `A8` is offline, and the row after it says there are no detached sessions

#### Scenario: Highlight kept on a refresh
- **WHEN** the palette lists three local sessions, the user moves the highlight to the second, and then an endpoint's state changes
- **THEN** the second session is still highlighted, and the filter text is unchanged

### Requirement: Sessions palette actions
In the sessions palette, enter SHALL attach the selected session into a new tab after the active tab, and make it active. A remote session's tab SHALL run on the session's endpoint. It SHALL use the first of these profiles that exists:
- the profile it was started with, when that profile targets the endpoint;
- the first profile that targets the endpoint;
- the profile of the first tab on the endpoint;
- the default profile.

`picker_kill` (ctrl-shift-delete) SHALL end the selected session, local or remote, and refresh the list, with the highlight on the row that takes the ended session's place.

#### Scenario: Attach a detached session
- **WHEN** a `--no-restore` launch left a session running `top` detached, and the user runs "Attach Session…" and presses enter on its row
- **THEN** a new active tab shows that `top`, with its screen as it was

#### Scenario: Kill from the palette
- **WHEN** the user selects a session's row and presses ctrl-shift-delete
- **THEN** the session's program ends and its row disappears

#### Scenario: Kill keeps the place
- **WHEN** the palette lists three detached sessions, the user highlights the second and presses ctrl-shift-delete, then presses enter
- **THEN** the second session's program ends, and enter attaches the session that was listed third

#### Scenario: Attach a remote session
- **WHEN** the user presses enter on a row listed under `A8`
- **THEN** a new active tab shows that session, and splitting its zone opens the new zone on `A8`

#### Scenario: Endpoint no profile targets
- **WHEN** no profile targets `old-host`, a tab of profile `Remote` runs on `old-host`, and the user presses enter on a row listed under `old-host`
- **THEN** the new tab has profile `Remote`, and it and its splits run on `old-host`

### Requirement: Commands listed for the active tab
The palette SHALL list the commands that apply to the tab that had focus when it opened, each with a readable name. Commands that do not apply to that tab SHALL NOT be listed. Commands that only work inside a palette, and the "none" action, SHALL NOT be listed. The list SHALL be in a stable order. The `Rename Tab` command SHALL be listed for every tab.

#### Scenario: Shell tab
- **WHEN** a shell tab is active and the user opens the palette
- **THEN** commands such as "Reload Config", "Clear Scrollback", "Rename Tab" and "Split Zone Right" are listed, and no command whose name starts with `tmux:` is listed

### Requirement: Text prompt
A command that needs text SHALL turn the palette into a prompt showing what is asked and, when the command provides one, a pre-filled value. The prompt SHALL be a text input, as defined by the `text-input` capability, with the pre-filled value selected so typing replaces it. Enter SHALL submit the text and escape SHALL cancel without running the command. A command started from its keybinding SHALL open the same prompt directly.

#### Scenario: Rename tab prompt
- **WHEN** the user picks "Rename Tab" in the palette while the active tab is titled `bash`
- **THEN** the palette asks for the new tab title, pre-filled with `bash`

#### Scenario: Prompt cancelled
- **WHEN** the user presses escape in a prompt
- **THEN** the palette closes and the command does not run

#### Scenario: Edit the pre-filled value
- **WHEN** the rename prompt is pre-filled with `bash` and the user presses end and types `-2`
- **THEN** enter renames the tab to `bash-2`

### Requirement: Palette dismissal and focus
Keyboard focus SHALL stay inside an open palette: tab and shift-tab SHALL NOT move it to the tab behind. Escape SHALL dismiss the palette, and a click outside it SHALL dismiss it too. However a palette closes, keyboard focus SHALL return to the tab that had it, except when the closing makes another overlay open.

#### Scenario: Tab stays inside
- **WHEN** the command palette is open and the user presses tab
- **THEN** focus is still in the palette's filter

#### Scenario: Escape returns focus
- **WHEN** the user opens the command palette and presses escape
- **THEN** the palette closes and keyboard focus is on the tab

#### Scenario: Click outside a plain palette
- **WHEN** the new-tab palette is open and the user clicks on the terminal behind it
- **THEN** the palette closes and keyboard focus is on the tab
