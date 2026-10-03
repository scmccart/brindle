# Spec Delta

## MODIFIED Requirements

### Requirement: Native window and pane commands
Because keys reach panes through tmux's `send-keys`, the tmux prefix SHALL NOT be relied upon. In control-mode tabs, Brindle SHALL provide bindings for:
- new tmux window as a tab (ctrl-shift-t), starting in the current pane's directory;
- close tab, which kills the tmux window (ctrl-shift-w);
- split right (ctrl-shift-e) and split down (ctrl-shift-o);
- close pane (ctrl-shift-x);
- toggle zoom (ctrl-shift-z);
- move between panes (alt-arrows);
- detach (ctrl-shift-d).

Detach SHALL apply only in control-mode tabs. Elsewhere ctrl-shift-d SHALL do nothing, as before, and SHALL NOT be sent to the program, where it would arrive as ctrl-d.

Changes made from other tmux clients, or from tmux commands inside a pane, SHALL be mirrored. These include renames, new or closed windows, splits and the current-window selection.

#### Scenario: Rename from inside a pane
- **WHEN** the user runs `tmux rename-window build` in a pane
- **THEN** that window's tab is titled `build`

#### Scenario: Close tab kills the tmux window
- **WHEN** the user closes a control-mode tab
- **THEN** that tmux window is killed and the tab disappears

#### Scenario: Detach key in a local tab
- **WHEN** a local shell tab is active and the user presses ctrl-shift-d
- **THEN** nothing happens: the shell receives no input, and detach is not listed in the command palette

## ADDED Requirements

### Requirement: Layout and pane arrangement commands
In control-mode tabs, Brindle SHALL provide commands, runnable from the command palette and bindable by name but without default keys, that apply to the tab's tmux window:
- new window;
- apply the even-horizontal, even-vertical, main-vertical or tiled layout;
- move to the next layout;
- rotate the panes;
- swap the active pane with the previous or the next pane, keeping it active;
- break the active pane out into a new window, which appears as a new active tab.

#### Scenario: Tiled layout
- **WHEN** a window has three panes and the user runs "tmux: Layout Tiled"
- **THEN** tmux rearranges the panes into the tiled layout and the tab shows the new arrangement

#### Scenario: Break pane
- **WHEN** a window has two panes and the user runs "tmux: Break Pane to New Tab"
- **THEN** the active pane moves to a new tmux window, a new tab for it appears and becomes active, and the original tab shows the remaining pane

### Requirement: Rename window
In control-mode tabs, a "tmux: Rename Window" command SHALL prompt for a name, pre-filled with the window's current name, and rename the tab's tmux window to the submitted text. Submitting empty text SHALL leave the name unchanged.

#### Scenario: Rename
- **WHEN** the user runs "tmux: Rename Window", enters `build` and presses enter
- **THEN** the tmux window is renamed and its tab is titled `build`

### Requirement: Run a tmux command
In control-mode tabs, a "tmux: Run Command" command SHALL prompt for a tmux command line and send it to the session, where it applies to the tab's window and active pane unless it names its own target. A line holding several commands separated by `;` SHALL run them all. If tmux reports an error for any of them, the prompt SHALL show the error; otherwise the palette SHALL close. Output from successful commands SHALL be discarded. Running such a line SHALL NOT disturb Brindle's own tracking of the session.

#### Scenario: Command applies to the active pane
- **WHEN** the user runs `split-window -h` from the prompt in a control-mode tab
- **THEN** the tab's active pane is split

#### Scenario: Several commands on one line
- **WHEN** the user runs `rename-window a ; split-window` from the prompt
- **THEN** the window is renamed and split, and later window and pane updates are still shown correctly

#### Scenario: Unknown command
- **WHEN** the user runs `bogus-command` from the prompt
- **THEN** the prompt stays open and shows tmux's error message

### Requirement: Run Command starts in the current directory
When a line run from "tmux: Run Command" creates a window or pane with `new-window`, `split-window` or their aliases `neww` and `splitw`, the new window or pane SHALL start in the active pane's current directory. This applies to each such command at the top level of the line, separated by `;`. A `-c` directory given by the user SHALL take precedence. Commands nested inside braces (for example, in `if-shell` arguments) and user-defined command aliases are not adjusted, and keep tmux's default directory.

#### Scenario: Split starts where the user is
- **WHEN** the active pane's shell is in `/tmp/project` and the user runs `split-window -h` from the prompt
- **THEN** the new pane's shell starts in `/tmp/project`

#### Scenario: Explicit directory wins
- **WHEN** the user runs `new-window -c /var/log` from the prompt
- **THEN** the new window starts in `/var/log`

#### Scenario: Every top-level command is adjusted
- **WHEN** the active pane is in `/tmp/project` and the user runs `neww ; splitw -v`
- **THEN** both the new window and the pane split from it start in `/tmp/project`
