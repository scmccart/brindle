# links Specification

## Purpose
Make links in terminal output usable: find URLs and OSC 8 hyperlinks under the mouse, show them while ctrl is held, and open or copy them.

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
A left click with ctrl held (without alt) on a link SHALL open the link when the button is released over the same link. When the `BRINDLE_LINK_OPENER` environment variable is set, Brindle SHALL pass the link as a single argument to the program it names. Otherwise it SHALL pass the link as a single argument to `xdg-open` on Linux, and on Windows have the shell open it with the link's default handler. It SHALL do this without waiting for it to finish. Brindle SHALL open a link, detected or OSC 8, only when its scheme is one of those "URL detection" names, compared without case. It SHALL open a `file:` link only when its host is empty, `localhost` or this machine's name. On Windows, a `file:` link whose path isn't on a drive letter (`X:\…`, with no further `:`, no `"` and no control character) SHALL NOT be opened, a `file:` link to anything but a directory SHALL be shown selected in Explorer instead of opened, and double quotes and whitespace in a link SHALL be percent-encoded before it reaches the shell. When Brindle doesn't open a clicked link, or the opener can't be started or reports a failure, the window SHALL say so in a notice on its config line naming the link and the cause, and SHALL write nothing into the terminal.

#### Scenario: Ctrl-click
- **WHEN** the user ctrl-clicks `https://example.com` in a shell
- **THEN** `xdg-open https://example.com` runs and Brindle stays responsive while it does

#### Scenario: Release elsewhere
- **WHEN** the user ctrl-presses on a link, moves off it and releases
- **THEN** nothing is opened

#### Scenario: Opener override
- **WHEN** `BRINDLE_LINK_OPENER=/tmp/record` is set and the user ctrl-clicks a link
- **THEN** `/tmp/record` runs with the link as its only argument

#### Scenario: Ctrl-click on Windows
- **WHEN** the user ctrl-clicks `https://example.com/a?b=1&c=2` in a Windows tab
- **THEN** the default browser opens exactly that URL, including the `&c=2`, and no console window appears

#### Scenario: No opener
- **WHEN** `BRINDLE_LINK_OPENER` names a program that doesn't exist and the user ctrl-clicks `https://example.com`
- **THEN** the window's config line says `https://example.com` could not be opened, naming the program and the error, and Brindle stays responsive

#### Scenario: Opener fails
- **WHEN** `BRINDLE_LINK_OPENER` names a program that exits with status 4, and the user ctrl-clicks a link
- **THEN** once it has exited, the window's config line says the link could not be opened, with that exit status

#### Scenario: Link without a scheme
- **WHEN** a program prints an OSC 8 hyperlink whose URI is `example.com/x`, and the user ctrl-clicks it
- **THEN** nothing is opened, and the config line says the link was not opened because it has no URI scheme

#### Scenario: Scheme Brindle doesn't open
- **WHEN** a program prints an OSC 8 hyperlink whose URI is `ms-settings:privacy` or `C:\Windows\System32\calc.exe`, and the user ctrl-clicks it
- **THEN** nothing is opened, and the config line says the link was not opened because Brindle doesn't open that scheme, naming it

#### Scenario: File link to another host
- **WHEN** a program prints `file://host/share/x.exe` (as text or an OSC 8 URI), and the user ctrl-clicks it
- **THEN** nothing is opened, no connection is made to `host`, and the config line says the link was not opened because it names another host

#### Scenario: Local file link on Windows
- **WHEN** a program in a Windows tab prints an OSC 8 hyperlink to `file:///C:/Users/me/tool.exe`, and the user ctrl-clicks it
- **THEN** Explorer shows `tool.exe` selected in `C:\Users\me`, and the program doesn't run

#### Scenario: File link with no drive on Windows
- **WHEN** a program in a Windows tab prints an OSC 8 hyperlink whose URI is `file:shell:AppsFolder` or `file:///etc/hosts`, and the user ctrl-clicks it
- **THEN** nothing is opened, Explorer isn't started, and the config line says the link was not opened because its path isn't on a drive letter

#### Scenario: Quotes in a link on Windows
- **WHEN** a program in a Windows tab prints an OSC 8 hyperlink whose URI is `https://example.com/" --flag`, and the user ctrl-clicks it
- **THEN** the shell is given `https://example.com/%22%20--flag` as one link

#### Scenario: Refusal leaves the terminal alone
- **WHEN** the user ctrl-clicks an OSC 8 link to `ms-settings:` in a shell
- **THEN** the config line names the link and says Brindle doesn't open that scheme, and the terminal's text and scrollback are unchanged

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

### Requirement: Links in agent messages
Links in an agent message or thought (inline, reference, autolinks, bare URLs with a scheme from "URL detection", and images) SHALL be drawn in the link colour, underlined. A left click with ctrl held (without alt), released over the same link, SHALL open it as "Opening links" says, with its refusals, opener and config-line notices; the pointer SHALL be a pointing hand while ctrl is held over a link. Text in code SHALL NOT be a link.

#### Scenario: Ctrl-click a link
- **WHEN** the agent's message holds `[docs](https://example.com/docs)` and the user ctrl-clicks `docs`
- **THEN** the opener runs with `https://example.com/docs`, and nothing asks first

#### Scenario: Plain click
- **WHEN** the user clicks `docs` without ctrl
- **THEN** nothing is opened

#### Scenario: Bare URL
- **WHEN** the agent's message is `see https://example.com/a for more`
- **THEN** `https://example.com/a` is drawn as a link, and ` for more` is not part of it

#### Scenario: Refused scheme
- **WHEN** the agent's message holds `[run](javascript:alert(1))` and the user ctrl-clicks `run`
- **THEN** nothing is opened or asked, and the config line says the link was not opened because Brindle doesn't open that scheme

#### Scenario: URL in code
- **WHEN** the agent's message holds `https://example.com` inside backticks
- **THEN** it is drawn as code, not as a link

### Requirement: A link's target on hover
While the pointer is over a link in an agent message or thought whose shown text differs from its target, a tooltip SHALL show the whole target. A link whose text is its target SHALL show no tooltip.

#### Scenario: Text unlike its target
- **WHEN** the agent's message holds `[the docs](https://evil.example/login)` and the pointer rests on `the docs`
- **THEN** a tooltip shows `https://evil.example/login`

#### Scenario: Bare URL has no tooltip
- **WHEN** the pointer rests on the bare URL `https://example.com/a` in an agent message
- **THEN** no tooltip shows

### Requirement: Confirming links an agent wrote
A link from an agent message or thought whose scheme is one Brindle opens but is neither `http` nor `https` SHALL be opened only after the user confirms it in a dialog that shows the whole link, with Cancel focused when it opens and an Open button. Cancel, escape and a press on the backdrop SHALL open nothing. Links in terminals SHALL NOT ask.

#### Scenario: Mail link
- **WHEN** the agent's message holds `<mailto:dev@example.com>` and the user ctrl-clicks it
- **THEN** a dialog shows `mailto:dev@example.com` with Cancel focused, and the opener runs only after the user presses Open

#### Scenario: Enter by reflex
- **WHEN** that dialog has just opened and the user presses enter
- **THEN** the dialog closes and nothing is opened

#### Scenario: Refused file link
- **WHEN** the agent's message holds `file://otherhost/share/x` and the user ctrl-clicks it
- **THEN** no dialog opens, and the config line says the link was not opened because it names another host
