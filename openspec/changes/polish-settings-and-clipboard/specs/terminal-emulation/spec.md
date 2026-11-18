## MODIFIED Requirements

### Requirement: Mouse reporting
When the program enables mouse tracking (modes 1000, 1002 or 1003), mouse presses, releases, drags, motion and wheel events SHALL be reported to the program.
- Coordinates SHALL be relative to the terminal's own grid.
- SGR encoding (1006) SHALL be used when enabled; otherwise UTF-8 (1005) or legacy X10 encoding.
- Holding shift SHALL bypass reporting: a shift-drag selects text instead, and a shift-right-click is handled as a right press that isn't reported (see "Right-click paste"), so it pastes only when right-click paste is on and no text is selected.
- A ctrl-click on a link SHALL bypass reporting and open the link instead, as defined by the `links` capability.

#### Scenario: SGR click
- **WHEN** SGR mouse mode is on and the user left-clicks column 5, row 10
- **THEN** the program receives `ESC [ < 0 ; 5 ; 10 M` and, on release, the same ending in `m`

#### Scenario: Shift overrides reporting
- **WHEN** mouse reporting is on and the user shift-drags
- **THEN** text is selected and nothing is sent to the program

#### Scenario: Right press reported
- **WHEN** SGR mouse mode is on, `right_click = "paste"`, and the user right-clicks column 6, row 2
- **THEN** the program receives `ESC [ < 2 ; 6 ; 2 M` and its release, and nothing is pasted

#### Scenario: Shift-right-click pastes over reporting
- **WHEN** SGR mouse mode is on, `right_click = "paste"`, no text is selected, and the user shift-right-clicks
- **THEN** the clipboard is pasted and no mouse report is sent

#### Scenario: Shift-right-click over a selection
- **WHEN** SGR mouse mode is on, `right_click = "paste"`, the user shift-drags to select text and then shift-right-clicks
- **THEN** nothing is pasted, no mouse report is sent, and the text stays selected

#### Scenario: Ctrl-click on a link overrides reporting
- **WHEN** mouse reporting is on and the user ctrl-clicks a URL
- **THEN** the URL opens and nothing is sent to the program

### Requirement: Selection and copy
The user SHALL be able to select text with the mouse:
- drag for a simple selection;
- double-click for a word;
- triple-click for a line;
- ctrl- or alt-drag for a block. A ctrl-drag that starts on a link SHALL NOT select; alt-drag SHALL block-select anywhere.

Finishing a selection SHALL copy it to the primary selection, and to the clipboard as well when `copy_on_select` is enabled. The setting in effect when the selection finishes SHALL decide, so a reload or the settings dialog changes it for open tabs too. A middle click SHALL paste the primary selection. Dragging past the top or bottom edge SHALL scroll.

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

#### Scenario: Copy on select after a reload
- **WHEN** a tab is open with `copy_on_select = false`, the config is changed to `copy_on_select = true` and reloaded, and the user drags over text in that tab
- **THEN** the clipboard contains the selected text

## ADDED Requirements

### Requirement: Right-click paste
A right press that isn't reported to the program (see "Mouse reporting") SHALL paste the clipboard, as the paste key does, when `right_click` is `"paste"` and no text is selected in the terminal. Otherwise it SHALL do nothing: with `right_click = "none"`, the default, or while text is selected, it neither pastes nor copies, and the selection stays. The setting and the selection at the press SHALL decide, so a reload changes it for open tabs. Its release SHALL NOT be reported either.

#### Scenario: Paste at a prompt
- **WHEN** `right_click = "paste"`, the clipboard holds `echo pasted`, no text is selected, and the user right-clicks in a shell's terminal and presses enter
- **THEN** the shell runs `echo pasted` and the screen shows `pasted`

#### Scenario: Off by default
- **WHEN** the config sets no `right_click`, the clipboard holds `echo pasted`, and the user right-clicks in a shell's terminal
- **THEN** nothing is typed into the shell

#### Scenario: Nothing happens over a selection
- **WHEN** `right_click = "paste"`, `copy_on_select = false`, the clipboard holds `echo pasted`, and the user selects text in a shell's terminal and right-clicks
- **THEN** nothing is typed into the shell, the text stays selected, and the clipboard still holds `echo pasted`

#### Scenario: Pastes once the selection is cleared
- **WHEN** `right_click = "paste"`, the clipboard holds `echo pasted`, and the user selects text in a shell's terminal, left-clicks in it to clear the selection, then right-clicks
- **THEN** `echo pasted` is typed into the shell

#### Scenario: Bracketed like the paste key
- **WHEN** `right_click = "paste"`, the program has enabled bracketed paste, no text is selected, and the user right-clicks
- **THEN** the program receives the clipboard text between `ESC [200~` and `ESC [201~`

#### Scenario: Turned on by a reload
- **WHEN** a tab is open with no `right_click` in the config, the config is changed to `right_click = "paste"` and reloaded, and the user right-clicks in that tab with no text selected
- **THEN** the clipboard is pasted

#### Scenario: Empty clipboard
- **WHEN** `right_click = "paste"`, the clipboard holds no text, and the user right-clicks
- **THEN** nothing is sent to the program
