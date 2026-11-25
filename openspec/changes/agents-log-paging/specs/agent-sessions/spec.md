## MODIFIED Requirements

### Requirement: Attach replays the transcript
A client on protocol 4 SHALL be able to attach a detached agent session. Before any live event the server SHALL send a delimited replay of a page of the log, the turn state and the agent's state as transcript events. The page SHALL hold the newest whole entries up to the page budget (256 KiB of encoded entries, or less when the client's `ATTACH` asks for less), and always at least the newest entry and every entry from the oldest permission request still waiting for an answer. When older entries exist, the replay SHALL say from which entry the page starts and how many entries before it can be fetched. After the replay the client's copy SHALL equal the server's log from the page's first entry on. Attaching follows the ownership and close policy rules of terminal sessions.

#### Scenario: Replay after a turn
- **WHEN** a session finished a turn while detached, and a client attaches it
- **THEN** the replay holds the user message, the agent message and the turn's end, and folding it gives the server's log

#### Scenario: Live after replay
- **WHEN** a client attaches a session in the middle of a turn
- **THEN** it receives the replay, then the turn's later chunks as live events, and none twice

#### Scenario: Attaching again replays from the start
- **WHEN** a client detaches an agent session after receiving part of a turn, and attaches it again
- **THEN** the new replay holds the newest page of the log, whatever the client had received or fetched before

#### Scenario: A long conversation sends a page
- **WHEN** a client attaches a session whose log holds 4 MiB of entries
- **THEN** the replay holds the newest entries up to 256 KiB, says that the entries before its first one can be fetched and how many there are, and ends with the slot, the turn and the agent's state

#### Scenario: A smaller budget
- **WHEN** a client attaches a long session asking for a 16 KiB page
- **THEN** the replay holds the newest entries up to 16 KiB, and at least the newest entry

#### Scenario: An entry larger than the page
- **WHEN** a client attaches a session whose newest entry is a 1 MiB agent message
- **THEN** the replay holds that whole entry, and no older one

#### Scenario: A waiting permission is always in the page
- **WHEN** a client attaches a session waiting on a permission request that has 2 MiB of entries after it
- **THEN** the replay starts at or before the permission request's entry, and the client can answer it

### Requirement: The log is bounded
An agent session's log SHALL stay within 16 MiB in its stored form. When it would pass that, the server SHALL drop the oldest entries whole until the log is within 12 MiB, record how many were dropped, and the session SHALL carry on. Entries that were dropped SHALL NOT be fetched again.

#### Scenario: Long conversation
- **WHEN** an agent session's log passes 16 MiB
- **THEN** its oldest entries are gone, the log is within 12 MiB, a replay starts with a notice saying how many were dropped, and the latest entries are intact

#### Scenario: Dropped entries can't be fetched
- **WHEN** a client fetches entries older than the oldest one the log still holds
- **THEN** it receives no entries and is told nothing earlier remains

## ADDED Requirements

### Requirement: Older entries are fetched on demand
A client that has an agent session attached SHALL be able to ask for the entries before a given entry, with a byte budget no larger than the page budget. The server SHALL answer with the newest whole entries before that entry, up to the budget and at least one when any remains, each in its current state, and SHALL say whether entries before them remain. Only the attached client SHALL be answered; any other request SHALL get an error naming the session.

#### Scenario: Fetching the page before
- **WHEN** a client whose replay started at entry 400 asks for the entries before entry 400 with a 256 KiB budget
- **THEN** it receives the newest whole entries before entry 400 up to 256 KiB, in order, and whether entries before them remain

#### Scenario: Fetching back to the start
- **WHEN** a client keeps fetching the entries before the oldest one it holds until it is told nothing earlier remains
- **THEN** its copy equals the server's whole log

#### Scenario: Not attached
- **WHEN** a client that doesn't have an agent session attached asks for its older entries
- **THEN** it gets an error naming the session

#### Scenario: A terminal session
- **WHEN** a client asks for older entries of a terminal session
- **THEN** it gets an error saying the session isn't an agent session

### Requirement: Entries a client hasn't fetched stay current
The server SHALL keep updating every entry of its log, whether or not the attached client has fetched it. A client SHALL ignore live changes to entries older than the oldest one it holds, and a later fetch SHALL return those entries with every change made to them.

#### Scenario: A tool call finishes outside the page
- **WHEN** a tool call entry older than the attached client's page is updated to `completed`, and the client then fetches the entries before its page
- **THEN** the fetched tool call entry has status `completed`

#### Scenario: Live after a fetch
- **WHEN** a client fetches an older page that holds a tool call still running, and the tool call is then updated
- **THEN** the client's copy of that entry shows the update

### Requirement: Checkpoints append what changed
The server SHALL store an agent session's checkpoint as a base and segments appended after it, each segment holding only what changed since the previous checkpoint. It SHALL write the whole checkpoint again when the session moves to another core, after entries were dropped, when the segments together would outgrow the base, and when the holder has no checkpoint for the session it can append to. A core restart or an upgrade SHALL rebuild the same session from the base and its segments.

#### Scenario: A message added to a long log
- **WHEN** an agent session whose log holds 8 MiB adds a 1 KiB agent message and is checkpointed
- **THEN** what the core writes to its holder for that checkpoint is about the size of the change, not of the log

#### Scenario: Core killed after appended checkpoints
- **WHEN** an agent session is checkpointed as a base and three segments, and the core is killed with SIGKILL
- **THEN** after the client reconnects, its copy, fetched back to the start, equals the log the killed core held, with the same entry ids

#### Scenario: Upgrade
- **WHEN** the core is upgraded while an agent session has appended segments
- **THEN** the old core writes the whole checkpoint before it steps down, and the new core carries the conversation on
