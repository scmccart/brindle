# Spec Delta

## MODIFIED Requirements

### Requirement: OSC 52 clipboard
Programs SHALL be able to set the clipboard and the primary selection through OSC 52. On platforms without a system primary selection, such as Windows, the primary selection is Brindle's own, as described under Selection and copy. Reading the clipboard through OSC 52 SHALL be refused unless the configuration enables it.

#### Scenario: Copy from tmux
- **WHEN** tmux, with `set-clipboard on`, copies a selection
- **THEN** the system clipboard contains that text

#### Scenario: Clipboard read refused by default
- **WHEN** a program requests the clipboard contents through OSC 52 and `osc52_paste` is false
- **THEN** no clipboard contents are sent to the program

### Requirement: Selection and copy
The user SHALL be able to select text with the mouse:
- drag for a simple selection;
- double-click for a word;
- triple-click for a line;
- ctrl- or alt-drag for a block.

Finishing a selection SHALL copy it to the primary selection, and to the clipboard as well when `copy_on_select` is enabled. A middle click SHALL paste the primary selection. Dragging past the top or bottom edge SHALL scroll.

On platforms without a system primary selection, such as Windows, the primary selection SHALL be a buffer shared by all of Brindle's windows. It is not visible to other applications.

#### Scenario: Copy with keybinding
- **WHEN** the user selects text and presses ctrl-shift-c
- **THEN** the clipboard contains the selected text

#### Scenario: Middle-click paste on Windows
- **WHEN** on Windows, with `copy_on_select` off, the user selects text in one tab and middle-clicks in another Brindle tab
- **THEN** the selected text is pasted there, and the system clipboard is unchanged
