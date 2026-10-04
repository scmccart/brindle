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

#### Scenario: Dividers meet cleanly
- **WHEN** dividers meet at a T-junction or cross, for example in a 2x2 tiled window
- **THEN** the lines join with no gap, and the active pane's highlight turns its corner on the lines it meets

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

### Requirement: Attaching keeps tmux's selection
Attaching to a session SHALL NOT change tmux's current window, its last window, or any window's active pane. The tab for tmux's current window SHALL become active, with keyboard focus in the pane tmux reports as active in that window. Each other tab SHALL show its window's active pane as active, so that commands aimed at the active pane act on it.

#### Scenario: Reattach to a pane that isn't first
- **WHEN** a window has three panes, the second is active, and the user attaches
- **THEN** keyboard focus is in the second pane, and tmux still reports the second pane as active

#### Scenario: Current and last window are kept
- **WHEN** a session has windows 0, 1 and 2, window 2 is current, window 0 is the last window, and the user attaches
- **THEN** the tab for window 2 is active, and tmux still reports window 2 as current and window 0 as the last window

#### Scenario: Commands act on tmux's active pane after attaching
- **WHEN** the user attaches to a window whose active pane is not the first, and splits right
- **THEN** the pane tmux had active is the one that is split

### Requirement: New windows come to the front only when tmux makes them current
A tab for a window that appears while attached SHALL become the active tab only when tmux makes that window its current window. Adding the tab SHALL NOT select the window in tmux.

#### Scenario: Window created in the foreground
- **WHEN** the user opens a new tmux window with ctrl-shift-t
- **THEN** its tab appears and becomes active

#### Scenario: Window created in the background
- **WHEN** another client runs `new-window -d` in the attached session
- **THEN** a tab for the new window appears without becoming active, and tmux's current window is unchanged

### Requirement: Color queries in panes get the theme's colors
A program in a control-mode pane that queries the default foreground (OSC 10) or background (OSC 11) color SHALL receive the foreground or background of the theme the pane is drawn with. This SHALL hold for panes present at attach and for panes created later. After a configuration reload, later queries SHALL receive the reloaded theme's colors.

#### Scenario: Background query after attach
- **WHEN** the user attaches with the `brindle-dark` theme, and a program in a pane sends `ESC ] 11 ; ? BEL`
- **THEN** the reply carries `brindle-dark`'s background color, not black

#### Scenario: Foreground query in a new pane
- **WHEN** the user splits a pane, and a program in the new pane sends `ESC ] 10 ; ? BEL`
- **THEN** the reply carries the theme's foreground color

#### Scenario: Theme change takes effect
- **WHEN** the user changes the profile's theme in the configuration file, the configuration reloads, and a program in an existing pane then queries OSC 11
- **THEN** the reply carries the new theme's background color

#### Scenario: tmux without color reports
- **WHEN** the tmux in use does not support reporting colors for a pane
- **THEN** attaching and using panes works as before, and color queries get whatever tmux answers on its own

### Requirement: Panes match tmux's pane geometry
Each pane SHALL be drawn at the position and size tmux gives the pane itself, not the layout cell around it, so that rows tmux reserves for border status lines (`pane-border-status top` or `bottom`) are not part of the pane's grid. When a pane's geometry changes without a layout change, Brindle SHALL resize the pane and show tmux's current contents for it within about a second.

#### Scenario: Attach to a window with border status lines
- **WHEN** a two-pane window has `pane-border-status top`, and the user attaches
- **THEN** each pane's grid has the height tmux reports for it, one row less than its layout cell, and its text lines up with `capture-pane`

#### Scenario: Border status turned on while attached
- **WHEN** the user runs `seq 1 60` in a pane, and `set -w pane-border-status top` is then run in the attached window
- **THEN** within about a second the pane shows the same lines as `capture-pane` for that pane, with a blank row above it

#### Scenario: Full-screen program after the change
- **WHEN** a full-screen program is running in a pane and border status lines are turned on
- **THEN** after the program redraws, Brindle's pane matches tmux's screen row for row, with no stale or doubled lines

#### Scenario: Panes rotated
- **WHEN** the user rotates a window's panes (Rotate Panes, or `rotate-window`), which tmux reports without a layout change
- **THEN** within about a second each pane is shown in its new place and size, matching `capture-pane`

#### Scenario: No border status lines
- **WHEN** `pane-border-status` is `off`
- **THEN** panes fill their layout cells exactly as before

### Requirement: Pane title lines
When a window's `pane-border-status` is `top` or `bottom`, each pane SHALL show its title line in the row tmux reserves for it, above or below the pane. The text SHALL be the pane's `pane-border-format` as tmux expands it, starting two columns in from the pane's left edge and clipped to the pane's width. Foreground and background colors, bold and reverse from the format's style directives SHALL be applied, and other directives SHALL NOT appear as text. Title lines SHALL update when tmux reports a change.

#### Scenario: Teammate pane names
- **WHEN** a pane's title is set with `select-pane -T @researcher` and its `pane-border-format` is `#[fg=blue,bold] #{pane_title} #[default]`, with `pane-border-status top`
- **THEN** the row above that pane shows ` @researcher ` in bold blue

#### Scenario: Default format
- **WHEN** `pane-border-status` is `bottom` and `pane-border-format` is tmux's default
- **THEN** each pane shows its index and quoted title in the row below it, and the active pane's index is shown in reverse video

#### Scenario: Title changes
- **WHEN** a program renames its pane with `select-pane -T` while the tab is open
- **THEN** the title line shows the new name within about a second

#### Scenario: Border status off
- **WHEN** `pane-border-status` is `off`
- **THEN** no title lines are drawn

### Requirement: Pane default colors follow tmux pane styles
A pane's default foreground and background SHALL follow the colors in its `window-style`, or in its `window-active-style` while it is the active pane, when those set a color other than `default`. For the active pane, a color that `window-active-style` leaves unset or `default` SHALL come from `window-style`, and a color neither sets SHALL be the theme's. Colors that a program sets explicitly SHALL NOT be changed. Programs that query the default colors (OSC 10/11) SHALL receive the colors the pane is shown with.

#### Scenario: Tinted teammate pane
- **WHEN** one pane of a window has `window-style fg=blue` and its program prints plain text
- **THEN** that text is blue, and text in the window's other panes keeps the theme foreground

#### Scenario: Active pane style
- **WHEN** a window has `window-active-style bg=#202020` and the user switches the active pane
- **THEN** the newly active pane's default background becomes `#202020`, and the previously active pane returns to its `window-style` or the theme background

#### Scenario: Explicit colors are kept
- **WHEN** a pane has `window-style fg=blue` and its program prints red text with SGR 31
- **THEN** the text is red

#### Scenario: Color queries see the style
- **WHEN** a pane has `window-style bg=#102030` and a program in it queries OSC 11
- **THEN** the reply carries `#102030`

### Requirement: Dividers follow tmux border styles
Dividers SHALL use the foreground color of the window's `pane-border-style`, and the active pane's highlight SHALL use the foreground of `pane-active-border-style`, when those set a color and differ from tmux's built-in defaults. Otherwise the theme's divider and accent colors SHALL be used. Title lines SHALL use the same colors as their base style.

#### Scenario: Colored borders
- **WHEN** a window has `pane-border-style fg=magenta` and `pane-active-border-style fg=yellow`
- **THEN** its dividers are drawn in the theme's magenta, and the active pane's highlight in the theme's yellow

#### Scenario: Default border styles
- **WHEN** both border styles have tmux's built-in values, for example the active border's default green
- **THEN** dividers and the highlight use the theme's colors as before

### Requirement: Subscriptions are removed before leaving a session
Before Brindle detaches from a session, quits, or ends the session itself by closing its last window or killing its last pane, it SHALL remove the control-mode format subscriptions it registered. tmux up to 3.6 can crash when a control client with subscriptions leaves or loses its session.

#### Scenario: Detach
- **WHEN** the user detaches a control-mode session with ctrl-shift-d
- **THEN** Brindle sends `refresh-client -B <name>` for each of its subscriptions before `detach-client`, and the tmux server keeps running

#### Scenario: Quit
- **WHEN** Brindle quits while attached to a control-mode session
- **THEN** it removes its subscriptions before its control connection closes, and the tmux server keeps running

#### Scenario: Closing the session's last window
- **WHEN** the user closes the tab of a session's only window
- **THEN** Brindle removes its subscriptions before sending `kill-window`
