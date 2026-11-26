## MODIFIED Requirements

### Requirement: Restored agent zones get their sessions back
An agent zone SHALL be recorded and restored as a terminal zone is ("Restored zones get their sessions back"), attaching its recorded agent session and showing the newest page of its conversation, earlier entries loading as the user scrolls up to them. When the listing failed, a recorded agent zone SHALL start a new agent session of its profile in its recorded directory. A recorded zone whose listed session is not of the tab's kind SHALL NOT be restored into it; that session is attached as its own tab.

#### Scenario: Agent survives a quit
- **WHEN** an agent zone has a finished turn, and the user quits and relaunches
- **THEN** the restored tab shows the same agent session, with the same pid and the whole conversation

#### Scenario: A long conversation comes back as a page
- **WHEN** an agent zone's conversation holds 2 MiB of entries, and the user quits and relaunches
- **THEN** the restored tab shows the same agent session with its newest entries and an "Earlier messages" item above them, and scrolling up shows the rest

#### Scenario: Agent split comes back
- **WHEN** an agent tab split into two agent zones is quit and relaunched
- **THEN** the tab comes back with both agent zones in their split, each showing its own conversation

#### Scenario: Kind changed under a recorded zone
- **WHEN** a recorded terminal tab's profile was changed to `type = "agent"` before the relaunch, and its recorded terminal session still runs
- **THEN** that terminal session is attached as a terminal tab of its own, and no agent zone shows it
