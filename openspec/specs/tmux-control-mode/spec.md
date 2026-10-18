# tmux-control-mode Specification

## Purpose
Attach to a tmux session in control mode and present it natively: each tmux window is a Brindle tab and panes are drawn by Brindle on tmux's layout grid. The session itself stays in tmux, so it survives detaching and closing Brindle.

## Requirements

### Requirement: Attach to a session
Opening a profile with `tmux = "control"` SHALL attach to the profile's session through `tmux -u [tmux_args] -C new-session -A -s <session>`, creating the session if needed. Each of the session's windows SHALL appear as a tab, in tmux's window order, with the tmux window name as the tab title. A session SHALL be shown by at most one Brindle window, also when several Brindle processes run (see "Session shown by another Brindle"). Opening a control profile whose session is already attached in the current window SHALL make one of its tabs active. When it is attached in another window, Brindle SHALL NOT attach it again. Picking the profile in the new-tab palette, which says beforehand that the session is open in another window, SHALL go to that session as "Go to <session>" does, without asking. The other ways of opening it SHALL ask what to do, as defined by "Choosing when a session is open elsewhere". Two profiles that run the same tmux program with the same `tmux_args` and session name SHALL count as the same session.

#### Scenario: Existing session with two windows
- **WHEN** the user opens a control profile whose session has windows 0 and 1
- **THEN** two tabs appear, named after the two windows, and tmux's current window is active

#### Scenario: Session open in this window
- **WHEN** the window shows a local tab (active) and the tabs of session `main`, and the user picks the profile for `main` in the new-tab palette
- **THEN** a tab of `main` becomes active, nothing is asked, and no second tmux client attaches

#### Scenario: Session open in another window
- **WHEN** window A shows session `main`, and the user picks the profile for `main` from the new-tab palette in window B
- **THEN** nothing is asked, window A's tab for tmux's current window of `main` becomes active, Brindle asks the desktop to raise window A, and `tmux list-clients` shows one Brindle client for `main`

#### Scenario: Session open in another window, opened by shortcut
- **WHEN** window A shows session `main`, and the user presses the ctrl-alt-N shortcut of the profile for `main` in window B
- **THEN** window B asks what to do, window A is not raised, and `tmux list-clients` shows one Brindle client for `main`

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
Detaching, or tmux exiting after the session had attached, SHALL close that session's tabs and leave the tmux session running. If those were the window's only tabs, what happens next SHALL follow the `tmux_session_end` setting, as defined by "A session leaving the window's only tabs".

#### Scenario: Detach beside other tabs
- **WHEN** the window shows a local shell tab and two tmux tabs, and the user presses ctrl-shift-d in a tmux tab
- **THEN** the two tmux tabs close, the shell tab stays, and `tmux ls` still lists the session

#### Scenario: Detach from the only tabs
- **WHEN** `tmux_session_end` is `new_tab`, the window shows only tmux tabs, and the user presses ctrl-shift-d
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
When Brindle uses control-mode format subscriptions, before it detaches from a session, quits, or ends the session itself by closing its last window or killing its last pane, it SHALL remove the subscriptions it registered. tmux up to 3.6 can crash when a control client with subscriptions leaves or loses its session.

#### Scenario: Detach
- **WHEN** the user detaches a control-mode session that uses subscriptions with ctrl-shift-d
- **THEN** Brindle sends `refresh-client -B <name>` for each of its subscriptions before `detach-client`, and the tmux server keeps running

#### Scenario: Quit
- **WHEN** Brindle quits while attached to a control-mode session that uses subscriptions
- **THEN** it removes its subscriptions before its control connection closes, and the tmux server keeps running

#### Scenario: Closing the session's last window
- **WHEN** the user closes the tab of the only window of a session that uses subscriptions
- **THEN** Brindle removes its subscriptions before sending `kill-window`

### Requirement: Bells and notifications from panes
Bells, OSC 9 and OSC 777 from a pane SHALL be handled as they are for a local tab, with the pane's tab as their tab, for panes in every window of the session, whether or not that window's tab is active. tmux sends no separate bell notification to control clients; these arrive in the pane's output.

#### Scenario: Notification from a background window
- **WHEN** a pane in a tmux window whose tab is not active prints `ESC ] 777 ; notify ; Build ; done BEL`
- **THEN** a desktop notification titled `Build — <window name>` with body `done` appears, and clicking it activates that window's tab

### Requirement: A session leaving the window's only tabs
When a control-mode session detaches or ends and its tabs were all of the window's tabs, Brindle SHALL act on `tmux_session_end`:
- `close_window` (the default) SHALL close the window, as closing its last tab does;
- `new_tab` SHALL open a tab with the default profile, or with the first profile that does not use tmux when the default is a control-mode profile, and then close the session's tabs; with no such profile, the window SHALL close;
- `ask` SHALL ask the user, as defined by "Asking when a session leaves".

#### Scenario: Last shell exits
- **WHEN** `tmux_session_end` is unset, the window shows only a tmux session's single tab, and the user types `exit` in its last pane
- **THEN** the window closes

#### Scenario: Last window closed
- **WHEN** `tmux_session_end` is `close_window`, another Brindle window is open, and the user closes the only tab of a session with ctrl-shift-w
- **THEN** the tmux window is killed and the window that showed it closes, while the other Brindle window stays open

#### Scenario: Detach closes the window
- **WHEN** `tmux_session_end` is `close_window`, the window shows only tmux tabs, and the user presses ctrl-shift-d
- **THEN** the window closes, and `tmux ls` still lists the session

#### Scenario: Session killed elsewhere, new tab
- **WHEN** `tmux_session_end` is `new_tab`, the default profile is a control-mode profile, the window shows only that session's tabs, and the session is killed with `tmux kill-session` from another terminal
- **THEN** a tab with the first non-tmux profile replaces the tmux tabs

#### Scenario: Window keeps other tabs
- **WHEN** `tmux_session_end` is `ask`, the window shows a local shell tab and a tmux tab, and the session ends
- **THEN** the tmux tab closes, the shell tab stays, and nothing is asked

### Requirement: Asking when a session leaves
With `tmux_session_end = "ask"`, the session's tabs SHALL stay in the window, with their panes ended, while a palette offers "Close window" first and selected, followed by "New tab: <profile>" for every profile in config order. A control-mode profile whose session is open in another window SHALL be listed as "Go to <session>" instead; choosing it SHALL make that session's tab active in its window, ask the desktop to raise that window, and close this one. Choosing any other profile SHALL open a tab with it and close the session's tabs. Choosing "Close window", or pressing escape, SHALL close the window; a click outside the palette SHALL NOT dismiss it. Opening this palette SHALL close any other open overlay.

#### Scenario: Choose a profile
- **WHEN** `tmux_session_end` is `ask`, the window shows only a tmux session's tabs, the session ends, and the user selects "New tab: Shell" and presses enter
- **THEN** a `Shell` tab opens and the tmux tabs close

#### Scenario: Escape closes the window
- **WHEN** the palette is asking after a session ended and the user presses escape
- **THEN** the window closes

#### Scenario: Stray click
- **WHEN** the palette is asking after a session ended and the user clicks on the ended tab behind it
- **THEN** the palette stays open and the window stays open

#### Scenario: Enter closes the window
- **WHEN** the palette is asking after a session ended and the user presses enter without moving the selection
- **THEN** the window closes

#### Scenario: Profile whose session is open elsewhere
- **WHEN** the palette is asking after session `work` ended, and another window shows session `main` of profile `tmux`
- **THEN** the palette lists "Go to main" in place of "New tab: tmux"

### Requirement: Quitting leaves windows as they were
When Brindle quits while attached to a control-mode session, the session leaving SHALL NOT close a window, open a tab, or ask, so that session restore records each window with its tmux tabs.

#### Scenario: Quit with only tmux tabs
- **WHEN** `restore_session = true`, `tmux_session_end` is `close_window`, the only window shows only tmux tabs, and the user quits Brindle
- **THEN** the next plain launch reattaches the session in a window, as it was

### Requirement: Choosing when a session is open elsewhere
When an action would attach a control-mode session that another window shows, Brindle SHALL show a palette in the current window instead, with the placeholder `tmux session "<session>" is open in another window` (`is open in this window` when ctrl-shift-n is pressed in the window that shows it). The actions are ctrl-alt-N, and opening the default profile with ctrl-shift-t, the `+` button or ctrl-shift-n (see `tabs`). The rows SHALL be, in order, with the first selected:
- "Go to <session>": make that session's tab active in its window and ask the desktop to raise that window;
- "New tmux session…": ask for a name, as "Create a new session" does, with the same profile;
- "New tab: <profile>" for each other profile in config order, or "New window: <profile>" when the action was ctrl-shift-n. The profile the action opened, and control-mode profiles whose session is open in any window, SHALL NOT be listed.

What a row opens SHALL go where the action would have opened the profile: a tab in the current window, or a new window for ctrl-shift-n. Escape or a click outside SHALL dismiss the palette without opening anything or raising any window.

#### Scenario: Go to the session
- **WHEN** window A shows session `main`, the user presses `tmux`'s ctrl-alt-N shortcut in window B, and presses enter in the palette that appears
- **THEN** window A's tab for tmux's current window of `main` becomes active, Brindle asks the desktop to raise window A, and window B has no new tab

#### Scenario: Another profile instead
- **WHEN** the palette is asking about `main` after ctrl-shift-t, and the user chooses "New tab: Shell"
- **THEN** a `Shell` tab opens in the current window

#### Scenario: New window with another profile
- **WHEN** the palette is asking about `main` after ctrl-shift-n, and the user chooses "New window: Shell"
- **THEN** a new window opens with one `Shell` tab

#### Scenario: New session from the palette
- **WHEN** the palette is asking about `main` after ctrl-shift-n, the user chooses "New tmux session…" and enters `scratch`
- **THEN** a new window opens showing session `scratch`, created on the same server as `main`

#### Scenario: Dismissed
- **WHEN** the palette is asking about `main` and the user presses escape
- **THEN** the palette closes, no tab or window opens, and no other window is raised

### Requirement: Create a new session
A "tmux: New Session" command SHALL prompt for a session name, then create that session and attach it in the current window, beside the window's other tabs. An empty name SHALL let tmux choose the name. The new session SHALL start in the active tab's directory. It SHALL use, in this order, the active tab's control-mode profile, the default profile if it is a control-mode profile, or the first control-mode profile. The command SHALL be available from every tab when the config has at least one control-mode profile, and SHALL NOT be listed otherwise. It has no default keybinding; its action name is `tmux_new_session`.

#### Scenario: Named session
- **WHEN** the user runs "tmux: New Session" from a local tab and enters `work`
- **THEN** `tmux ls` lists a session named `work`, its window appears as a tab in the current window, and the prompt closes

#### Scenario: Name left empty
- **WHEN** the user runs "tmux: New Session" and submits an empty name
- **THEN** a session with a name tmux chose is created and shown, and its tabs are recorded for session restore under that name

#### Scenario: From a tmux tab
- **WHEN** the active tab belongs to a session of profile `remote`, whose `tmux_args` select another server, and the user creates session `logs`
- **THEN** `logs` is created on the server `remote` uses

#### Scenario: Name already taken
- **WHEN** the user enters the name of a session that already exists
- **THEN** the prompt stays open showing tmux's error (`duplicate session: <name>`), no tab is added, and no attach-failure tab opens

#### Scenario: No control-mode profile
- **WHEN** no profile in the config uses `tmux = "control"`
- **THEN** "tmux: New Session" is not listed in the command palette

### Requirement: A new session's name is tracked
The name Brindle records for a control-mode session, for restore and for detecting a session that is already open, SHALL be the name tmux reports for the attached session. A session whose name is not yet known SHALL NOT be treated as matching any other session.

#### Scenario: tmux-chosen name
- **WHEN** a session created with an empty name was named `3` by tmux, and the user later picks a profile whose session is `3` on the same server
- **THEN** the existing tabs for `3` are focused instead of a second attach

### Requirement: No subscriptions on tmux with the subscription crash
On tmux older than 3.7, or when the version tmux reports can't be read, Brindle SHALL NOT register control-mode format subscriptions. It SHALL instead ask tmux for each pane's geometry, title line and pane styles, and each window's border status and border styles, about once a second while attached, so these keep updating as they do with subscriptions. tmux 3.7 and newer SHALL be followed with subscriptions.

#### Scenario: Session ended from inside tmux
- **WHEN** Brindle is attached to a session on tmux 3.6, another session exists on the same server, and `exit` is run in the attached session's last shell, repeated ten times
- **THEN** the tmux server keeps running and the other session still exists

#### Scenario: No subscription commands
- **WHEN** Brindle attaches to a session on tmux 3.6, then detaches
- **THEN** it sends no `refresh-client -B` command at any point

#### Scenario: Title changes on old tmux
- **WHEN** Brindle is attached on tmux 3.6, the window has `pane-border-status top`, and a program renames its pane with `select-pane -T`
- **THEN** the title line shows the new name within about a second

#### Scenario: Border status turned on while attached on old tmux
- **WHEN** Brindle is attached on tmux 3.6 and `set -w pane-border-status top` is run in the attached window
- **THEN** within about a second each pane is one row shorter, matches `capture-pane`, and shows its title line

#### Scenario: Newer tmux
- **WHEN** Brindle attaches to a session on tmux 3.7 or newer
- **THEN** it registers its subscriptions as before and does not ask for these values once a second

#### Scenario: Unreadable version
- **WHEN** tmux's reported version is not of the form `<major>.<minor>` with an optional letter or `next-` prefix (for example a distribution's own version string)
- **THEN** Brindle behaves as on tmux 3.6

### Requirement: Palette queries in panes get the theme's palette
When a program in a control-mode pane queries a palette color (OSC 4, indexes 0–255) and tmux does not answer it, Brindle SHALL reply with the theme's color for the index, on any tmux version. When tmux answers the query itself, Brindle SHALL NOT add a reply. tmux answers when the pane's palette has the index (a program set it, or the `pane-colours` option does) or when a terminal client is attached to the pane's session.

#### Scenario: ANSI color after attach
- **WHEN** the user attaches, and a program in a pane sends `ESC ] 4 ; 4 ; ? BEL`
- **THEN** it receives one reply, carrying the theme's blue

#### Scenario: Index beyond the ANSI colors
- **WHEN** a program in a pane queries index 196
- **THEN** it receives one reply, carrying the standard 256-color palette's `rgb:ffff/0000/0000`

#### Scenario: New pane
- **WHEN** the user splits a pane, and a program in the new pane queries index 1
- **THEN** it receives one reply, carrying the theme's red

#### Scenario: Theme change takes effect
- **WHEN** the user changes the profile's theme in the configuration file, the configuration reloads, and a program in an existing pane then queries index 4
- **THEN** the reply carries the new theme's blue

#### Scenario: Color a program set
- **WHEN** a program in a pane sets index 5 with `ESC ] 4 ; 5 ; rgb:ff/00/00 BEL` and then queries index 5
- **THEN** it receives one reply, carrying `rgb:ffff/0000/0000`

#### Scenario: Color from `pane-colours`
- **WHEN** the pane's `pane-colours[1]` is `#445566` and a program in it queries index 1
- **THEN** it receives one reply, carrying `rgb:4444/5555/6666`

#### Scenario: Terminal client attached
- **WHEN** a terminal client is attached to the same session as Brindle, and a program in a pane queries index 4
- **THEN** Brindle sends no reply, and the program gets only the reply tmux relays from that client's terminal, if any

### Requirement: Control clients are UTF-8 in any locale
Brindle's control-mode clients SHALL tell tmux they handle UTF-8, whatever locale Brindle runs in. Attaching, the tab titles and every reply Brindle parses SHALL be the same with no locale set (`LANG`, `LC_ALL` and `LC_CTYPE` all unset, or `C`) as under a UTF-8 locale.

#### Scenario: No locale
- **WHEN** Brindle runs with `LANG`, `LC_ALL`, `LC_CTYPE` and `TMUX` unset and opens a control profile whose session has windows named `café` and `logs`
- **THEN** two tabs appear, titled `café` and `logs`

#### Scenario: The user's own flags still apply
- **WHEN** a control profile has `tmux_args = ["-L", "work"]`
- **THEN** Brindle runs `tmux -u -L work -C new-session -A -s <session>`, attaching on the `work` server

### Requirement: Session shown by another Brindle
Before attaching a control-mode profile's session by name, Brindle SHALL ask tmux for the session's clients. When one of them is a control client of another running Brindle, Brindle SHALL NOT attach the session, and the window that would have shown it SHALL ask what to open instead. Clients of other programs, and control clients started from a shell, SHALL NOT count. When tmux can't list the session's clients, Brindle SHALL attach it as before.

#### Scenario: Second launch
- **WHEN** one Brindle shows session `main`, and a second Brindle is launched whose default profile attaches `main`
- **THEN** the second Brindle's window has no tabs and asks, and `tmux list-clients` shows one client for `main`

#### Scenario: Actions in the second Brindle
- **WHEN** one Brindle shows session `main`, a second Brindle was launched with `main`'s profile, and the user opens a new tmux window there
- **THEN** the first Brindle's tabs stay as they were and `main` keeps its windows

#### Scenario: Shortcut in the second Brindle
- **WHEN** one Brindle shows session `main`, and the user presses the ctrl-alt-N shortcut of `main`'s profile in a local tab of a second Brindle
- **THEN** the second Brindle asks in that window, its local tab stays, and `tmux list-clients` shows one client for `main`

#### Scenario: Restored in the second Brindle
- **WHEN** one Brindle shows session `main`, and a second Brindle restores a window whose tabs include `main`
- **THEN** that window asks about `main` and the window's other tabs open as recorded

#### Scenario: Another program's control client
- **WHEN** a `tmux -C attach -t main` started from a shell is attached, and the user opens `main`'s profile in Brindle
- **THEN** Brindle attaches `main` and shows its windows as tabs

#### Scenario: No server yet
- **WHEN** the profile's tmux server isn't running, and the user opens the profile
- **THEN** Brindle starts the session and attaches it

### Requirement: Asking about a session another Brindle shows
When another Brindle shows the session, Brindle SHALL show a palette with the placeholder `tmux session "<session>" is open in another Brindle`. Its rows SHALL be "New tmux session…", as in "Choosing when a session is open elsewhere", then "New tab: <profile>" for each other profile in config order, leaving out control-mode profiles whose session this Brindle shows. The first row SHALL be selected. Both SHALL open in the window that asked. There SHALL be no "Go to" row.

#### Scenario: Rows
- **WHEN** a second Brindle asks about `main`, and its profiles are `tmux` (session `main`) and `Shell`
- **THEN** the palette reads `tmux session "main" is open in another Brindle` and lists "New tmux session…" (selected), then "New tab: Shell"

#### Scenario: Another profile instead
- **WHEN** a second Brindle's window with no tabs asks about `main`, and the user chooses "New tab: Shell"
- **THEN** a `Shell` tab opens in that window and no tmux client attaches `main`

#### Scenario: New session instead
- **WHEN** a second Brindle asks about `main`, and the user chooses "New tmux session…" and enters `scratch`
- **THEN** session `scratch` is created on the same server and shown in that window

### Requirement: Dismissing the question about another Brindle's session
Escape or a click outside SHALL dismiss the palette without opening anything. When the window has no tabs, dismissing the palette, or the prompt its "New tmux session…" row opened, SHALL close the window. When the palette replaced a session that ended, the ended session's tabs SHALL close once what the user chose shows up, or when the palette or its prompt is dismissed.

#### Scenario: Dismissed at launch
- **WHEN** a second Brindle's only window has no tabs and asks about `main`, and the user presses escape
- **THEN** the window closes and the second Brindle quits

#### Scenario: Dismissed beside a local tab
- **WHEN** a second Brindle asks about `main` after ctrl-alt-N in a local tab, and the user presses escape
- **THEN** the palette closes and the local tab stays

#### Scenario: Asked after a session ended
- **WHEN** `tmux_session_end` is `ask`, session `work` ended, the user chooses "New tab: tmux" for session `main`, which another Brindle shows, and then chooses "New tab: Shell"
- **THEN** a `Shell` tab opens and `work`'s ended tabs close
