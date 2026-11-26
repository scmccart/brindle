## ADDED Requirements

### Requirement: Choosing agents are not restored
An agent zone whose agent is still choosing SHALL NOT be recorded for session restore, and quitting SHALL end its session; its tab's other zones are recorded as usual. An agent started only to list conversations SHALL be ended when its palette closes, when the window closes and when Brindle quits, and SHALL never be attached as a tab by a later launch.

#### Scenario: Quit while choosing
- **WHEN** a tab's one zone is choosing, and the user quits and relaunches
- **THEN** that tab isn't restored, and no agent session is listed for it

#### Scenario: Quit with a list open
- **WHEN** Brindle started an agent to list conversations, and the user quits with the palette still open
- **THEN** no agent session of that agent is left on the terminal server

#### Scenario: Chosen, then quit
- **WHEN** a zone chose a conversation, its agent is ready, and the user quits and relaunches
- **THEN** the zone comes back showing that conversation, as any agent zone does
