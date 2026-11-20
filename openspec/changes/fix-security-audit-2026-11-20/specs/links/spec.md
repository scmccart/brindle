## MODIFIED Requirements

### Requirement: Opening links
A left click with ctrl held (without alt) on a link SHALL open the link when the button is released over the same link. When the `BRINDLE_LINK_OPENER` environment variable is set, Brindle SHALL pass the link as a single argument to the program it names. Otherwise it SHALL pass the link as a single argument to `xdg-open` on Linux, and on Windows have the shell open it with the link's default handler. It SHALL do this without waiting for it to finish. Brindle SHALL open a link, detected or OSC 8, only when its scheme is one of those "URL detection" names, compared without case. It SHALL open a `file:` link only when its host is empty, `localhost` or this machine's name. On Windows, a `file:` link whose path isn't on a drive letter (`X:\…`) SHALL NOT be opened, a `file:` link to anything but a directory SHALL be shown selected in Explorer instead of opened, and double quotes and whitespace in a link SHALL be percent-encoded before it reaches the shell. When Brindle doesn't open a clicked link, or the opener can't be started or reports a failure, the tab that was clicked SHALL say so in one dim line naming the link and the cause.

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
- **THEN** the tab shows a dim line saying `https://example.com` could not be opened, naming the program and the error, and Brindle stays responsive

#### Scenario: Opener fails
- **WHEN** `BRINDLE_LINK_OPENER` names a program that exits with status 4, and the user ctrl-clicks a link
- **THEN** once it has exited, the tab shows a dim line saying the link could not be opened, with that exit status

#### Scenario: Link without a scheme
- **WHEN** a program prints an OSC 8 hyperlink whose URI is `example.com/x`, and the user ctrl-clicks it
- **THEN** nothing is opened, and the tab says the link was not opened because it has no URI scheme

#### Scenario: Scheme Brindle doesn't open
- **WHEN** a program prints an OSC 8 hyperlink whose URI is `ms-settings:privacy` or `C:\Windows\System32\calc.exe`, and the user ctrl-clicks it
- **THEN** nothing is opened, and the tab says the link was not opened because Brindle doesn't open that scheme, naming it

#### Scenario: File link to another host
- **WHEN** a program prints `file://host/share/x.exe` (as text or an OSC 8 URI), and the user ctrl-clicks it
- **THEN** nothing is opened, no connection is made to `host`, and the tab says the link was not opened because it names another host

#### Scenario: Local file link on Windows
- **WHEN** a program in a Windows tab prints an OSC 8 hyperlink to `file:///C:/Users/me/tool.exe`, and the user ctrl-clicks it
- **THEN** Explorer shows `tool.exe` selected in `C:\Users\me`, and the program doesn't run

#### Scenario: File link with no drive on Windows
- **WHEN** a program in a Windows tab prints an OSC 8 hyperlink whose URI is `file:shell:AppsFolder` or `file:///etc/hosts`, and the user ctrl-clicks it
- **THEN** nothing is opened, Explorer isn't started, and the tab says the link was not opened because its path isn't on a drive letter

#### Scenario: Quotes in a link on Windows
- **WHEN** a program in a Windows tab prints an OSC 8 hyperlink whose URI is `https://example.com/" --flag`, and the user ctrl-clicks it
- **THEN** the shell is given `https://example.com/%22%20--flag` as one link
