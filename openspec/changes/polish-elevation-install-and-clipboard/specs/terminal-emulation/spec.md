# Spec Delta

## ADDED Requirements

### Requirement: Clipboard shared with other applications
Every copy Brindle makes (the copy keys, `copy_on_select`, Copy Link, the Copy Last commands and OSC 52) SHALL write the system clipboard, readable by other applications while Brindle runs, and paste SHALL read the plain text another application put there. A clipboard holding no text SHALL paste nothing. On Linux the primary selection SHALL be the desktop's, shared the same way both ways.

#### Scenario: Copy to another application
- **WHEN** the user selects `hello` in a tab and presses ctrl-shift-c, then pastes in a text editor
- **THEN** the editor receives `hello`

#### Scenario: Paste from another application
- **WHEN** the user copies `echo from-editor` in a text editor, then presses ctrl-shift-v in a tab at a shell prompt
- **THEN** the shell's line reads `echo from-editor`, not yet run

#### Scenario: Rich text pastes as plain text
- **WHEN** the user copies a bold link from a web page, then pastes in a tab
- **THEN** the tab receives the link's visible text, with no markup

#### Scenario: Image on the clipboard
- **WHEN** the clipboard holds only an image, and the user pastes in a tab
- **THEN** nothing is sent to the program

#### Scenario: OSC 52 reaches other applications
- **WHEN** a program prints `ESC ] 52 ; c ; aGk= BEL`, and the user then pastes in a text editor
- **THEN** the editor receives `hi`

#### Scenario: Primary selection from another application on Linux
- **WHEN** on Linux the user selects `abc` in a text editor and middle-clicks in a tab
- **THEN** the tab receives `abc`

#### Scenario: A copy outlives its tab
- **WHEN** the user copies text in a tab, closes that tab, and pastes in another application
- **THEN** the application receives the copied text

### Requirement: Copied line ends
Text Brindle copies from the grid SHALL join its lines with the platform's line end: `\r\n` on Windows, `\n` elsewhere. A line the program wrapped SHALL be copied as one line. Trailing blanks of a row SHALL NOT be copied. Text a program sets through OSC 52 SHALL be written as the program sent it.

#### Scenario: Two lines on Windows
- **WHEN** on Windows the user selects two printed lines `one` and `two` and copies them, then pastes in Notepad
- **THEN** the clipboard holds `one\r\ntwo`, and Notepad shows two lines

#### Scenario: Wrapped line
- **WHEN** a program prints a 100-character line in an 80-column tab, and the user triple-clicks it and copies
- **THEN** the clipboard holds the 100 characters with no line end inside

#### Scenario: Paste with Windows line ends
- **WHEN** the clipboard holds `a\r\nb` and bracketed paste is off
- **THEN** the program receives `a\rb`

### Requirement: Mouse clipboard feedback
A copy or paste made with the mouse SHALL show a small label at the pointer for about one second, then fade out: "Copied" when finishing a selection copies it to the clipboard (`copy_on_select`), "Pasted" when a right-click pastes, and "Nothing to paste" when a right-click finds no text on the clipboard. The label SHALL NOT take focus, catch the pointer or move the grid. Key and palette copies and pastes SHALL show none.

#### Scenario: Copy on select
- **WHEN** `copy_on_select = true` and the user drags over text and releases
- **THEN** "Copied" shows beside the pointer and is gone about a second later

#### Scenario: Right-click paste
- **WHEN** `right_click = "paste"`, the clipboard holds text, and the user right-clicks in a shell's terminal
- **THEN** the text is pasted and "Pasted" shows beside the pointer

#### Scenario: Primary only
- **WHEN** `copy_on_select = false` and the user finishes a selection
- **THEN** no label shows, since the clipboard didn't change

#### Scenario: Keys show nothing
- **WHEN** the user presses ctrl-shift-c over a selection, or ctrl-shift-v
- **THEN** no label shows

#### Scenario: Clicks pass through
- **WHEN** "Copied" is showing and the user clicks where it is drawn
- **THEN** the click reaches the grid as if no label were there

## MODIFIED Requirements

### Requirement: Right-click paste
A right press that isn't reported to the program (see "Mouse reporting") SHALL, when `right_click` is `"paste"`, clear any selection in the terminal and paste the clipboard, as the paste key does. When the clipboard holds no text it SHALL do neither. With `right_click = "none"`, the default, it SHALL do nothing and leave any selection. The setting at the press SHALL decide, so a reload changes it for open tabs. Its release SHALL NOT be reported either.

#### Scenario: Paste at a prompt
- **WHEN** `right_click = "paste"`, the clipboard holds `echo pasted`, no text is selected, and the user right-clicks in a shell's terminal and presses enter
- **THEN** the shell runs `echo pasted` and the screen shows `pasted`

#### Scenario: Off by default
- **WHEN** the config sets no `right_click`, the clipboard holds `echo pasted`, and the user right-clicks in a shell's terminal
- **THEN** nothing is typed into the shell

#### Scenario: Nothing happens over a selection
- **WHEN** the config sets no `right_click`, the clipboard holds `echo pasted`, and the user selects text in a shell's terminal and right-clicks
- **THEN** nothing is typed into the shell, and the text stays selected

#### Scenario: Over a selection, clears and pastes
- **WHEN** `right_click = "paste"`, `copy_on_select = false`, the clipboard holds `echo pasted`, and the user selects text in a shell's terminal and right-clicks
- **THEN** the selection is cleared, `echo pasted` is typed into the shell, and the clipboard still holds `echo pasted`

#### Scenario: Select, then right-click to paste it
- **WHEN** `right_click = "paste"` and `copy_on_select = true`, and the user selects `ls -la` in a shell's terminal and right-clicks
- **THEN** the selection is cleared and `ls -la` is typed into the shell

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
- **WHEN** `right_click = "paste"`, the clipboard holds no text, and the user right-clicks over a selection
- **THEN** nothing is sent to the program, and the selection stays

#### Scenario: A program that uses the mouse
- **WHEN** `right_click = "paste"`, htop reports the mouse, and the user right-clicks over a selection
- **THEN** htop receives the right press, and nothing is pasted or cleared; shift-right-click instead clears the selection and pastes
