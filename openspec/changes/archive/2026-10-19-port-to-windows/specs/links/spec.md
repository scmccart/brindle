# Spec Delta

## MODIFIED Requirements

### Requirement: Opening links
A left click with ctrl held (without alt) on a link SHALL open the link when the button is released over the same link. When the `BRINDLE_LINK_OPENER` environment variable is set, Brindle SHALL pass the link as a single argument to the program it names. Otherwise it SHALL pass the link as a single argument to `xdg-open` on Linux, and on Windows have the shell open it with the link's default handler. It SHALL do this without waiting for it to finish. Brindle SHALL NOT open an OSC 8 URI that has no valid scheme. A failure to start the opener SHALL be logged.

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
