## MODIFIED Requirements

### Requirement: Commands and messages are not restored
A tab that was opened with a command (`brindle terminal -e`, or the config editor tab) SHALL come back as its profile's program in the recorded directory, without the command. Tabs that only show a message, such as a failed connection, SHALL NOT be recorded.

#### Scenario: Command tab
- **WHEN** Brindle was started with `brindle terminal -e htop` in `~/src` and then quit
- **THEN** after a plain launch the tab runs the default profile's shell in `~/src`, not htop

#### Scenario: Message tab
- **WHEN** the only other tab showed a connection failure message when Brindle quit
- **THEN** that tab is not restored
