# links Specification

## Purpose
Make links in terminal output usable: find URLs and OSC 8 hyperlinks under the mouse, show them while ctrl is held, and open or copy them, in local tabs and tmux control-mode panes alike.

## Requirements

### Requirement: URL detection
Text under the mouse SHALL be recognised as a link when it is a URL with one of the schemes `http`, `https`, `ftp`, `file`, `ssh`, `git`, `gemini`, `gopher`, `ipfs`, `ipns` (each followed by `://`) or `mailto`, `news`, `magnet` (followed by `:`), with at least one character after the scheme. The scheme SHALL start at the beginning of a word. The URL SHALL end at whitespace, a control character, one of `<`, `>`, `"`, `` ` ``, `{`, `}`, `|`, `\`, `^`, or a box-drawing character.

#### Scenario: URL in a sentence
- **WHEN** the screen shows `see https://example.com/docs for details` and the mouse is over `example`
- **THEN** the link is `https://example.com/docs`

#### Scenario: Mail address
- **WHEN** the screen shows `mailto:dev@example.com` and the mouse is over it
- **THEN** the link is `mailto:dev@example.com`

#### Scenario: Unknown scheme
- **WHEN** the screen shows `foo://bar` or `https://` alone
- **THEN** no link is found there

#### Scenario: Scheme inside a word
- **WHEN** the screen shows `xhttps://example.com`
- **THEN** no link is found there

### Requirement: Link boundaries
Trailing characters that usually end a sentence (`.`, `,`, `:`, `;`, `!`, `?`, `'`, `*`) SHALL NOT be part of a detected URL. A closing `)` or `]` SHALL be part of the URL only when the URL contains its matching opening bracket. Brackets and quotes around a URL SHALL NOT be part of it.

#### Scenario: Sentence end
- **WHEN** the screen shows `Visit https://example.com/a.`
- **THEN** the link is `https://example.com/a`

#### Scenario: Parenthesised URL
- **WHEN** the screen shows `(see https://example.com/a)`
- **THEN** the link is `https://example.com/a`

#### Scenario: Balanced brackets kept
- **WHEN** the screen shows `https://en.wikipedia.org/wiki/Rust_(programming_language)`
- **THEN** the link includes the closing `)`

#### Scenario: Markdown link
- **WHEN** the screen shows `[docs](https://example.com/x)`
- **THEN** the link is `https://example.com/x`

### Requirement: Wrapped URLs
A URL that the terminal wrapped onto the next row because it reached the right edge SHALL be found as one link from any of its rows. Rows separated by a real line break SHALL NOT be joined.

#### Scenario: Long URL wraps
- **WHEN** a 120-character URL is printed in an 80-column terminal and the mouse is over its second row
- **THEN** the link is the whole URL

#### Scenario: Separate lines
- **WHEN** one line ends with `https://example.com/a` and the next line starts with `bc`
- **THEN** the link is `https://example.com/a`

### Requirement: OSC 8 hyperlinks
Text a program marks with an OSC 8 hyperlink SHALL be a link to that hyperlink's URI. An OSC 8 hyperlink SHALL take precedence over a URL detected in the same cell. Every visible cell carrying the same hyperlink (same URI and id) SHALL belong to the link, even when the cells are not adjacent.

#### Scenario: Labelled link
- **WHEN** a program prints `ESC ] 8 ; ; https://example.com ESC \ docs ESC ] 8 ; ; ESC \` and the mouse is over `docs`
- **THEN** the link is `https://example.com`, covering the four cells of `docs`

#### Scenario: Link text that looks like a URL
- **WHEN** an OSC 8 hyperlink to `https://a.example` has the text `https://b.example`
- **THEN** the link under the mouse is `https://a.example`

### Requirement: Hover feedback
While ctrl is held (without alt) and the mouse is over a link in a terminal, that link's cells SHALL be underlined and the mouse pointer SHALL be a pointing hand. The feedback SHALL go away when ctrl is released, the mouse leaves the link, or the window is deactivated. It SHALL follow the text under the mouse when the view scrolls or output changes it. There SHALL be none during a selection drag. Without ctrl, links SHALL look and behave like other text.

#### Scenario: Holding ctrl over a URL
- **WHEN** the mouse is over a URL and the user holds ctrl
- **THEN** the whole URL is underlined and the pointer is a pointing hand

#### Scenario: Releasing ctrl
- **WHEN** the user releases ctrl while the URL is underlined
- **THEN** the underline goes away and the pointer goes back to the text pointer

### Requirement: Opening links
A left click with ctrl held (without alt) on a link SHALL open the link when the button is released over the same link. Brindle SHALL pass the link as a single argument to `xdg-open`, or to the program named by the `BRINDLE_LINK_OPENER` environment variable when it is set, without waiting for it to finish. Brindle SHALL NOT open an OSC 8 URI that has no valid scheme. A failure to start the opener SHALL be logged.

#### Scenario: Ctrl-click
- **WHEN** the user ctrl-clicks `https://example.com` in a shell
- **THEN** `xdg-open https://example.com` runs and Brindle stays responsive while it does

#### Scenario: Release elsewhere
- **WHEN** the user ctrl-presses on a link, moves off it and releases
- **THEN** nothing is opened

#### Scenario: Opener override
- **WHEN** `BRINDLE_LINK_OPENER=/tmp/record` is set and the user ctrl-clicks a link
- **THEN** `/tmp/record` runs with the link as its only argument

### Requirement: Links take ctrl-clicks from mouse reporting
When the program has turned on mouse reporting, a ctrl-click on a link SHALL open it and SHALL NOT be reported to the program, neither the press nor the release. A ctrl-click anywhere else SHALL still be reported as before.

#### Scenario: Link in a mouse-aware program
- **WHEN** a program with SGR mouse reporting on shows a URL and the user ctrl-clicks it
- **THEN** the link opens and the program receives no mouse report

#### Scenario: Ctrl-click on plain text
- **WHEN** the same program is running and the user ctrl-clicks text that is not a link
- **THEN** the program receives the click with the ctrl modifier

### Requirement: Copying a link
A `copy_link` action, listed in the command palette as "Copy Link Under Mouse" and with no default key, SHALL copy the link under the mouse in the focused terminal to the clipboard, whether or not ctrl is held. When there is no link under the mouse, the clipboard SHALL be left unchanged.

#### Scenario: Copy from a keybinding
- **WHEN** the user binds a key to `copy_link`, points at a URL and presses the key
- **THEN** the clipboard contains the URL

### Requirement: Links in tmux control-mode panes
Detection, OSC 8 hyperlinks, hover feedback, opening and copying SHALL work in tmux control-mode panes as in local tabs. OSC 8 hyperlinks in a pane's existing content SHALL be kept when Brindle attaches and restores the pane. A ctrl-click on a link in an inactive pane SHALL select that pane and open the link.

#### Scenario: OSC 8 in a pane
- **WHEN** a program in a control-mode pane prints an OSC 8 hyperlink and the user ctrl-clicks it
- **THEN** the hyperlink's URI opens

#### Scenario: Link survives attach
- **WHEN** a pane already shows an OSC 8 hyperlink and Brindle attaches to the session
- **THEN** ctrl-clicking that text opens the hyperlink's URI

#### Scenario: Inactive pane
- **WHEN** a window has two panes and the user ctrl-clicks a URL in the inactive one
- **THEN** that pane becomes active and the URL opens
