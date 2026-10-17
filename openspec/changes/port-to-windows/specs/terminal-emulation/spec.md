# Spec Delta

## MODIFIED Requirements

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

## ADDED Requirements

### Requirement: Pseudoconsole host on Windows
On Windows, Brindle SHALL host each local tab with the ConPTY build that ships with Brindle (`conpty.dll` and `OpenConsole.exe`). It SHALL load only that exact build, never a different `conpty.dll` found beside the executable, on `PATH` or in the current directory. When it can't, it SHALL use the ConPTY built into Windows and log a warning naming what was missing. `BRINDLE_CONPTY=inbox` SHALL force the built-in ConPTY, for tests.

#### Scenario: Shipped host
- **WHEN** a tab opens in Brindle installed from the MSI
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
