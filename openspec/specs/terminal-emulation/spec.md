# terminal-emulation Specification

## Purpose
Run programs on a pseudo-terminal and carry keyboard, mouse, paste, focus and clipboard traffic between the user and the program using xterm-compatible encodings, so shells, editors and tmux behave as they do in mainstream terminals.

## Requirements

### Requirement: Program launch environment
The terminal SHALL start each program on its own PTY, which on Windows is a pseudoconsole, with `TERM=xterm-256color`, `COLORTERM=truecolor`, `TERM_PROGRAM=Brindle` and `TERM_PROGRAM_VERSION` set, plus any environment the profile defines. Profile entries SHALL override the defaults. On Windows, environment variable names SHALL match case-insensitively, so a profile entry replaces an inherited variable whatever its case.

#### Scenario: Default environment
- **WHEN** a tab starts the user's shell
- **THEN** `echo $TERM $COLORTERM $TERM_PROGRAM` prints `xterm-256color truecolor Brindle`

#### Scenario: Profile environment override
- **WHEN** a profile sets `env = { EDITOR = "nvim" }`
- **THEN** programs in tabs opened with that profile see `EDITOR=nvim`

#### Scenario: Case-insensitive override on Windows
- **WHEN** a Windows profile sets `env = { path = "C:\\tools" }`
- **THEN** programs in its tabs see a single `Path` variable whose value is `C:\tools`

### Requirement: Launch failure is reported in the tab
If the program cannot be started, the terminal SHALL show an error naming the program and the reason, instead of failing silently.

#### Scenario: Missing program
- **WHEN** a tab is opened with a command that does not exist
- **THEN** the tab shows text saying Brindle could not start that program, along with the OS error

### Requirement: Keyboard encoding
Special keys and modifier combinations SHALL be sent as xterm byte sequences. Plain text, with no modifiers or shift only, SHALL arrive through the platform text-input path so IME composition and dead keys work.
- Cursor and Home/End keys SHALL follow application-cursor mode when unmodified.
- Modified keys SHALL use the `CSI 1;m X` and `CSI n;m ~` forms.
- Ctrl+letter SHALL send the C0 control code.
- Alt SHALL act as Meta by prefixing ESC.

#### Scenario: Control codes reach the program
- **WHEN** the user presses ctrl-c or ctrl-b
- **THEN** the program receives byte 0x03 or 0x02 respectively

#### Scenario: Application cursor mode
- **WHEN** the program has enabled application cursor mode and the user presses Up
- **THEN** the program receives `ESC O A` (otherwise `ESC [ A`)

#### Scenario: Modified arrow
- **WHEN** the user presses ctrl-left
- **THEN** the program receives `ESC [ 1 ; 5 D`

#### Scenario: Alt as meta
- **WHEN** the user presses alt-f
- **THEN** the program receives `ESC f`

#### Scenario: Text is not double-sent
- **WHEN** the user types a plain letter
- **THEN** the program receives that character exactly once

### Requirement: Mouse reporting
When the program enables mouse tracking (modes 1000, 1002 or 1003), mouse presses, releases, drags, motion and wheel events SHALL be reported to the program.
- Coordinates SHALL be relative to the terminal's own grid.
- SGR encoding (1006) SHALL be used when enabled; otherwise UTF-8 (1005) or legacy X10 encoding.
- Holding shift SHALL bypass reporting and select text instead.
- A ctrl-click on a link SHALL bypass reporting and open the link instead, as defined by the `links` capability.

#### Scenario: SGR click
- **WHEN** SGR mouse mode is on and the user left-clicks column 5, row 10
- **THEN** the program receives `ESC [ < 0 ; 5 ; 10 M` and, on release, the same ending in `m`

#### Scenario: Shift overrides reporting
- **WHEN** mouse reporting is on and the user shift-drags
- **THEN** text is selected and nothing is sent to the program

#### Scenario: Ctrl-click on a link overrides reporting
- **WHEN** mouse reporting is on and the user ctrl-clicks a URL
- **THEN** the URL opens and nothing is sent to the program

### Requirement: Wheel scrolling
The mouse wheel SHALL:
- send wheel reports when the program has enabled mouse reporting;
- otherwise, in the alternate screen with alternate-scroll mode (1007) on, send arrow keys;
- otherwise scroll the scrollback.

#### Scenario: Scrolling a pager
- **WHEN** `less` is running (alternate screen, no mouse reporting) and the user scrolls up
- **THEN** `less` receives Up arrow sequences and scrolls its content

#### Scenario: Scrolling history
- **WHEN** a shell prompt is showing and the user scrolls up
- **THEN** earlier output from the scrollback becomes visible

### Requirement: Paste
Pasted text SHALL be wrapped in `ESC [200~` … `ESC [201~` when the program has enabled bracketed paste, with any ESC bytes removed from the pasted text. Otherwise newlines SHALL be converted to carriage returns. Pasting or typing SHALL scroll the view back to the bottom and clear any selection.

#### Scenario: Bracketed paste cannot end early
- **WHEN** bracketed paste is on and the clipboard contains `x ESC[201~ y`
- **THEN** the program receives `ESC[200~x[201~yESC[201~`

### Requirement: Focus reporting
When the program enables focus events (mode 1004), the terminal SHALL send `ESC [ I` when it gains focus and `ESC [ O` when it loses focus. Focus counts as lost when the window is deactivated.

#### Scenario: tmux focus-events
- **WHEN** tmux runs with `focus-events on` and the user switches to another window
- **THEN** tmux receives `ESC [ O`

### Requirement: OSC 52 clipboard
Programs SHALL be able to set the clipboard and the primary selection through OSC 52. On platforms without a system primary selection, such as Windows, the primary selection is Brindle's own, as described under "Selection and copy". Reading the clipboard through OSC 52 SHALL be refused unless the configuration enables it.

#### Scenario: Copy from tmux
- **WHEN** tmux, with `set-clipboard on`, copies a selection
- **THEN** the system clipboard contains that text

#### Scenario: Clipboard read refused by default
- **WHEN** a program requests the clipboard contents through OSC 52 and `osc52_paste` is false
- **THEN** no clipboard contents are sent to the program

#### Scenario: Primary selection through OSC 52 on Windows
- **WHEN** a program in a Windows tab sets the primary selection with `ESC ] 52 ; p ; aGk= BEL`
- **THEN** a middle click in any Brindle tab pastes `hi`, and the system clipboard is unchanged

### Requirement: Selection and copy
The user SHALL be able to select text with the mouse:
- drag for a simple selection;
- double-click for a word;
- triple-click for a line;
- ctrl- or alt-drag for a block. A ctrl-drag that starts on a link SHALL NOT select; alt-drag SHALL block-select anywhere.

Finishing a selection SHALL copy it to the primary selection, and to the clipboard as well when `copy_on_select` is enabled. A middle click SHALL paste the primary selection. Dragging past the top or bottom edge SHALL scroll.

On platforms without a system primary selection, such as Windows, the primary selection SHALL be a buffer shared by all of Brindle's windows. It is not visible to other applications.

#### Scenario: Copy with keybinding
- **WHEN** the user selects text and presses ctrl-shift-c
- **THEN** the clipboard contains the selected text

#### Scenario: Block selection from a link
- **WHEN** the user alt-drags starting on a URL
- **THEN** a block of text is selected and the URL is not opened

#### Scenario: Middle-click paste on Windows
- **WHEN** on Windows, with `copy_on_select` off, the user selects text in one tab and middle-clicks in another Brindle tab
- **THEN** the selected text is pasted there, and the system clipboard is unchanged

### Requirement: Scrollback
Each terminal SHALL keep `scrollback_lines` lines of history. The user SHALL be able to scroll by line, by page, and to the top or bottom, and to clear the scrollback. ctrl-shift-up and ctrl-shift-down SHALL scroll by one line in a terminal with no shell-integration prompt marks. In a terminal with marks, they jump between prompts as defined by the `shell-integration` capability. Line scrolling stays available as the bindable `scroll_line_up` and `scroll_line_down` actions.

#### Scenario: Page up
- **WHEN** the user presses shift-pageup
- **THEN** the view moves up one page of history

#### Scenario: Line scroll without shell integration
- **WHEN** the shell emits no prompt marks and the user presses ctrl-shift-up
- **THEN** the view moves up one line of history

### Requirement: Grid follows the view size
A terminal's grid SHALL be resized to the number of whole cells that fit in its area after padding, and the PTY SHALL be told the new size.

#### Scenario: Window resize
- **WHEN** the user makes the window wider
- **THEN** `tput cols` reports the larger column count

### Requirement: Pseudoconsole host on Windows
On Windows, Brindle SHALL host each local tab with the ConPTY build that ships with Brindle (`conpty.dll` and `OpenConsole.exe`). It SHALL load only that exact build, never a different `conpty.dll` found beside the executable, on `PATH` or in the current directory. When it can't, it SHALL use the ConPTY built into Windows and log a warning naming what was missing. `BRINDLE_CONPTY=inbox` SHALL force the built-in ConPTY, for tests.

#### Scenario: Shipped host
- **WHEN** a tab opens in Brindle run from the release zip or installed with WinGet
- **THEN** the tab's console host process is the shipped `OpenConsole.exe`, not `conhost.exe`

#### Scenario: Planted DLL is ignored
- **WHEN** a directory on `PATH`, the directory Brindle was started from, or the directory of a `cargo install`ed `brindle.exe` contains a `conpty.dll` that differs from the shipped one
- **THEN** Brindle doesn't load it

#### Scenario: Fallback
- **WHEN** the shipped ConPTY can't be loaded, or `BRINDLE_CONPTY=inbox` is set
- **THEN** tabs still open through the built-in ConPTY, and a warning is logged once

### Requirement: Output reaches Brindle as written on Windows
With the shipped ConPTY host, the escape sequences a Windows program writes SHALL reach Brindle unchanged and in order with its text. This covers mouse and focus modes, bracketed paste, synchronized updates, the alternate screen, and OSC 0/2, 7, 8, 9, 52, 133 and 777. Color queries (OSC 4, 10, 11) SHALL be answered by Brindle, not by the console host.

#### Scenario: Mouse in a Windows TUI
- **WHEN** a program in a Windows tab enables SGR mouse reporting and the user clicks column 5, row 10
- **THEN** the program receives `ESC [ < 0 ; 5 ; 10 M`

#### Scenario: Background query
- **WHEN** a program in a Windows tab sends `ESC ] 11 ; ? BEL`
- **THEN** it receives the active theme's background color

#### Scenario: Notification lands with its text
- **WHEN** a program prints `building`, then an OSC 9 notification, then `done`
- **THEN** the notification is raised after `building` is on screen, and before `done` is

### Requirement: Built-in ConPTY limits on Windows
When Brindle has fallen back to Windows' built-in ConPTY, "Output reaches Brindle as written on Windows" does not apply: that host can drop mouse modes and synchronized updates, and it repaints output. Every other requirement SHALL still hold. Typing, resizing, scrollback, titles and closing tabs SHALL keep working.

#### Scenario: Typing with the fallback
- **WHEN** a tab runs through the built-in ConPTY and the user types `echo hi` and enter in PowerShell
- **THEN** `hi` is printed

### Requirement: Glyph widths agree with the grid on Windows
On Windows, with the shipped ConPTY host, the console SHALL measure characters the way Brindle's grid does, one code point at a time, so that text written through console APIs lands in the columns Brindle draws it in. The built-in host has its own fixed measure, and this requirement doesn't apply to it.

#### Scenario: Emoji with a variation selector
- **WHEN** a Windows program writes `❤️` (U+2764 U+FE0F) at column 0 and then reads its cursor position from the console
- **THEN** the console reports column 1, the column where Brindle draws the next character

#### Scenario: Combining mark
- **WHEN** a Windows program writes `e` followed by U+0301 at column 0 and then reads its cursor position from the console
- **THEN** the console reports column 1

### Requirement: Closing a tab never blocks on Windows
Closing a Windows tab, or its program exiting, SHALL release its pseudoconsole without blocking Brindle's window. That holds even while the program is still writing output.

#### Scenario: Close during output
- **WHEN** a Windows tab is running a command that prints continuously and the user closes the tab
- **THEN** the tab closes at once and the window stays responsive

### Requirement: The window stays responsive during continuous output
While any terminal in a window (a local tab or zone, a remote terminal or a tmux pane) receives output continuously, the window SHALL keep repainting that terminal's latest output, SHALL keep handling input and actions, and Brindle SHALL still quit when asked. Updates during continuous output MAY be grouped, but output that arrives after a quiet spell SHALL be shown without added delay.

#### Scenario: Flooding tab keeps painting
- **WHEN** a Windows tab runs `for /L %i in (1,0,2) do @echo flood %i` in `cmd` for several seconds
- **THEN** the window repaints with new `flood` lines while the loop runs and Windows never reports it as not responding

#### Scenario: Quitting while a tab floods
- **WHEN** a tab is printing continuously and Brindle is asked to quit (for example by `--dump-screen-after`)
- **THEN** Brindle prints its dump and exits within a second of the deadline

#### Scenario: Echo after a quiet spell
- **WHEN** a tab has had no output for a while and the user types a character that the shell echoes
- **THEN** the echo is painted on the next frame, with no pacing delay added

### Requirement: Local Linux terminals run in the terminal server
On Linux, each local tab and zone SHALL run its program in a session of the terminal server, as defined by the `terminal-server` capability, with the `kill` close policy. Brindle SHALL start the server when none is running. Every other requirement of this capability, and titles, directories, shell integration, program status and notifications, SHALL behave as they do with a PTY owned by Brindle.

#### Scenario: Tab program runs under the server
- **WHEN** a local Linux tab starts the user's shell
- **THEN** the shell's parent process is the terminal server, not Brindle, and `brindle-remote list` lists its session as attached

#### Scenario: Closing a tab ends its program
- **WHEN** the user closes a local tab that is running `sleep 1000`
- **THEN** `sleep` is gone and the session is no longer listed

#### Scenario: Quitting ends every program
- **WHEN** the user quits Brindle with local tabs open
- **THEN** their programs are gone and their sessions are no longer listed

#### Scenario: A crash ends every program
- **WHEN** Brindle is killed with SIGKILL while local tabs are open
- **THEN** their programs are gone within a few seconds

#### Scenario: Titles and directories
- **WHEN** the user runs `cd /var/log` and then `htop` in a local tab, and splits the zone while `htop` runs
- **THEN** the tab title shows `htop`, and the new zone starts in `/var/log`

### Requirement: Sessions get the launching Brindle's environment
A local Linux tab's program SHALL see the environment of the Brindle process that opened it, with the defaults and profile entries of "Program launch environment" applied, never the environment of the Brindle that started the server.

#### Scenario: Second Brindle with a different environment
- **WHEN** a Brindle started with `FOO=1` is running, and a second Brindle started with `FOO=2` opens a tab
- **THEN** `echo $FOO` in the second Brindle's tab prints `2`

### Requirement: Brindle reconnects when the core restarts
When Brindle's connection to the terminal server's core ends, Brindle SHALL reconnect, starting a core if none is running, and get its sessions back. Each tab's screen SHALL then match its session's. The replay SHALL NOT ring the bell, raise notifications or change status dots beyond what the session's current state shows. Input typed while reconnecting SHALL be sent once reconnected.

#### Scenario: Core killed
- **WHEN** the core is killed with SIGKILL while a local tab is running `htop`
- **THEN** within a few seconds the tab shows `htop` again, responds to keys, and `htop` has the same pid

#### Scenario: No bell from the replay
- **WHEN** a session's output since the last screen save contains a bell and an OSC 9 notification, and the core restarts
- **THEN** reconnecting raises no bell dot and no notification

### Requirement: A session lost for good is reported in the tab
When Brindle can't get a session back, because its holder ended, the core no longer has it, or no core is reachable within a bounded time, the tab SHALL keep its last screen and show a line saying the session was lost and why, instead of closing.

#### Scenario: Holder killed
- **WHEN** the holder of a local tab's session is killed with SIGKILL
- **THEN** the tab stays open with its last screen and a line saying the session was lost

### Requirement: An unusable terminal server is reported in the tab
Local Linux tabs SHALL always run in the terminal server; Brindle SHALL NOT run their programs on PTYs of its own. When Brindle can't find the `brindle-remote` binary, can't start or reach the server, or shares no protocol version with the running server, each local tab it opens SHALL show an error naming the reason, as for a program that can't start. Brindle SHALL try the server again for the next tab.

#### Scenario: Missing helper
- **WHEN** no `brindle-remote` is found beside Brindle and `BRINDLE_REMOTE_AGENT` is unset, and the user opens a local tab
- **THEN** the tab shows an error saying the terminal server helper was not found, with how to point Brindle at it

#### Scenario: No common protocol version
- **WHEN** the running server speaks none of Brindle's protocol versions and refuses the upgrade
- **THEN** the tab shows an error naming both versions, and the running server and its sessions are untouched

#### Scenario: Recovery
- **WHEN** a tab showed a missing-helper error, the helper is then installed, and the user opens another local tab
- **THEN** the new tab runs in the server
