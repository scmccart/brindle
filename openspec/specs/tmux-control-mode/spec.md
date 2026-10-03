# tmux-control-mode Specification

## Purpose
Attach to a tmux session in control mode and present it natively: each tmux window is a Brindle tab and panes are drawn by Brindle on tmux's layout grid. The session itself stays in tmux, so it survives detaching and closing Brindle.

## Requirements

### Requirement: Attach to a session
Opening a profile with `tmux = "control"` SHALL attach to the profile's session through `tmux [tmux_args] -C new-session -A -s <session>`, creating the session if needed. Each of the session's windows SHALL appear as a tab, in tmux's window order, with the tmux window name as the tab title. Opening an already-attached control profile SHALL focus its tabs instead of attaching again.

#### Scenario: Existing session with two windows
- **WHEN** the user opens a control profile whose session has windows 0 and 1
- **THEN** two tabs appear, named after the two windows, and tmux's current window is active

### Requirement: Panes drawn natively
Each tab SHALL lay out its window's panes exactly on tmux's cell grid, with divider lines between panes. The part of a divider next to the active pane SHALL be highlighted. A zoomed window SHALL show only the zoomed pane, with a "zoomed" indicator.

#### Scenario: Split layout
- **WHEN** a window has one pane on the left and two stacked on the right
- **THEN** the tab shows the same three-pane arrangement with dividers between them

### Requirement: Client size follows the tab
The tmux client size SHALL be set to the number of whole cells that fit in the tab's content area, and updated whenever that changes, so tmux sizes the windows to fill the tab.

#### Scenario: Window resize reflows panes
- **WHEN** the user enlarges the Brindle window
- **THEN** tmux is told the new size and the panes grow to fill it

### Requirement: Input and focus
Keystrokes, pastes and mouse reports in a pane SHALL be delivered to that pane's program. Clicking a pane SHALL make it tmux's active pane. When tmux changes the active pane (for example after a split, or when a pane closes), keyboard focus SHALL follow while the tab has focus. The terminal's own replies to device queries SHALL NOT be forwarded to panes; tmux answers those itself.

#### Scenario: Typing after a split
- **WHEN** the user splits a pane and types `ls` and Enter
- **THEN** `ls` runs in the new pane

### Requirement: Existing pane contents are restored
On attach, and for every pane created afterwards, the pane SHALL show tmux's current screen and history. It SHALL also restore:
- the alternate screen, if one is active;
- the cursor position and visibility;
- application cursor and keypad modes;
- mouse tracking modes (1000/1002/1003, SGR and UTF-8).

Output produced before the snapshot SHALL NOT be shown twice.

#### Scenario: Reattach to a running editor
- **WHEN** the user detaches while vi is open in a pane, and then reattaches
- **THEN** the pane shows vi's screen, and vi still receives mouse events in the mode it requested

#### Scenario: Button-drag mode is restored exactly
- **WHEN** a pane's program enabled modes 1002 and 1006, and the user reattaches
- **THEN** the pane reports button-drag motion in SGR form, and not any-motion (1003)

### Requirement: Native window and pane commands
Because keys reach panes through tmux's `send-keys`, the tmux prefix SHALL NOT be relied upon. In control-mode tabs, Brindle SHALL provide bindings for:
- new tmux window as a tab (ctrl-shift-t), starting in the current pane's directory;
- close tab, which kills the tmux window (ctrl-shift-w);
- split right (ctrl-shift-e) and split down (ctrl-shift-o);
- close pane (ctrl-shift-x);
- toggle zoom (ctrl-shift-z);
- move between panes (alt-arrows);
- detach (ctrl-shift-d).

Changes made from other tmux clients, or from tmux commands inside a pane, SHALL be mirrored. These include renames, new or closed windows, splits and the current-window selection.

#### Scenario: Rename from inside a pane
- **WHEN** the user runs `tmux rename-window build` in a pane
- **THEN** that window's tab is titled `build`

#### Scenario: Close tab kills the tmux window
- **WHEN** the user closes a control-mode tab
- **THEN** that tmux window is killed and the tab disappears

### Requirement: Detach keeps the session
Detaching, or tmux exiting after the session had attached, SHALL close that session's tabs and leave the tmux session running. If those were the window's only tabs, a tab with a non-tmux profile SHALL be opened so the window stays open; if no such profile exists, the window SHALL close.

#### Scenario: Detach from the only tabs
- **WHEN** the window shows only tmux tabs and the user presses ctrl-shift-d
- **THEN** the tmux tabs close, a shell tab opens in their place, and `tmux ls` still lists the session

### Requirement: Attach failure is explained
If tmux exits before the session attaches (for example, tmux is not installed, the command is wrong, or the server refuses), Brindle SHALL open a tab explaining the failure and naming the profile settings to check.

#### Scenario: tmux missing
- **WHEN** a control profile's command does not exist
- **THEN** a tab opens saying tmux ended before attaching, with the OS error
