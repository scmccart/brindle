# Spec Delta

## ADDED Requirements

### Requirement: Restored agent zones get their sessions back
An agent zone SHALL be recorded and restored as a terminal zone is ("Restored zones get their sessions back"), attaching its recorded agent session and showing its whole conversation. When the listing failed, a recorded agent zone SHALL start a new agent session of its profile in its recorded directory. A recorded zone whose listed session is not of the tab's kind SHALL NOT be restored into it; that session is attached as its own tab.

#### Scenario: Agent survives a quit
- **WHEN** an agent zone has a finished turn, and the user quits and relaunches
- **THEN** the restored tab shows the same agent session, with the same pid and the whole conversation

#### Scenario: Agent split comes back
- **WHEN** an agent tab split into two agent zones is quit and relaunched
- **THEN** the tab comes back with both agent zones in their split, each showing its own conversation

#### Scenario: Kind changed under a recorded zone
- **WHEN** a recorded terminal tab's profile was changed to `type = "agent"` before the relaunch, and its recorded terminal session still runs
- **THEN** that terminal session is attached as a terminal tab of its own, and no agent zone shows it

### Requirement: Launch reattaches detached agent sessions
A launch SHALL attach detached agent sessions as it attaches detached terminals ("Launch reattaches detached terminals"), each as an agent tab, grouped by their recorded layouts. An agent session's tab SHALL use, of the profiles the terminal rules allow, the first that is an agent profile, else the profile those rules give. A recorded layout SHALL only group sessions of one kind; a session of the other kind opens as its own tab.

#### Scenario: Agent left by a skipped launch
- **WHEN** a `--no-restore` launch left an agent session of profile `Agent` detached, and the user quits and launches `brindle`
- **THEN** the window shows the recorded tabs followed by an agent tab of profile `Agent` showing that session's conversation

#### Scenario: Agent profile gone
- **WHEN** a detached agent session's profile has been deleted, and the config has another agent profile `Other`
- **THEN** the launch attaches it as an agent tab of profile `Other`

#### Scenario: No agent profile at all
- **WHEN** a detached agent session is attached at launch and the config has no agent profile
- **THEN** it opens as an agent tab of the default profile, showing its conversation, and splitting that tab does nothing
