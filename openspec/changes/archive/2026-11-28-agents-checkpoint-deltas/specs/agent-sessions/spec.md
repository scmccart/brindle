## MODIFIED Requirements

### Requirement: Checkpoints append what changed
The server SHALL store an agent session's checkpoint as a base and segments appended after it. Each segment SHALL hold only what changed since the previous checkpoint: the log's events, the bytes a partial line gained, and the session's other state only where it changed or is small. What one segment costs SHALL NOT grow with the length of the conversation or with the number of tool calls it holds. The server SHALL write the checkpoint whole again in these cases: the session moves to another core, entries were dropped, the segments would outgrow the base, its agent starts again in a new holder session, or the holder has nothing it can append to. A core restart or an upgrade SHALL rebuild the same session from the base and its segments. A checkpoint whose records are not in this form SHALL NOT be restored in part: its session SHALL be adopted as not restored, saying so in its conversation, with its agent left running.

#### Scenario: A message added to a long log
- **WHEN** an agent session whose log holds 8 MiB adds a 1 KiB agent message and is checkpointed
- **THEN** what the core writes to its holder for that checkpoint is about the size of the change, not of the log

#### Scenario: A tool-heavy conversation
- **WHEN** an agent session has made 5,000 tool calls, each asking a permission that is answered, and is checkpointed at each request and each answer
- **THEN** each segment the core writes is about the size of that step's events, the same at the 5,000th tool call as at the first
- **AND** over the conversation the bytes written stay within a small multiple of the bytes the session added, plus a fixed amount per checkpoint

#### Scenario: A long line arriving in pieces
- **WHEN** an agent writes one 8 MiB line that reaches the core in 64 KiB pieces, and the core checkpoints every 512 KiB of it
- **THEN** each segment carries only the line's bytes that came since the previous checkpoint, so its segments together hold the line about once

#### Scenario: Core killed while a long line arrives
- **WHEN** the core is killed with SIGKILL after it checkpointed part of an 8 MiB line as segments, and the agent finishes the line before a new core adopts the holder
- **THEN** the new core rebuilds the partial line from those segments, reads its rest from the tail, and applies the whole line once

#### Scenario: Core killed after appended checkpoints
- **WHEN** an agent session is checkpointed as a base and three segments, and the core is killed with SIGKILL
- **THEN** after the client reconnects, its copy, fetched back to the start, equals the log the killed core held, with the same entry ids

#### Scenario: Tool calls carried on after a core restart
- **WHEN** an agent session's checkpoint holds tool calls from an agent that was started again, and the new agent's tool calls, and the core is killed with SIGKILL
- **THEN** after the new core adopts it, an update for one of the new agent's tool calls changes that entry, and an update naming an earlier agent's call id adds a new entry, as before the restart

#### Scenario: Upgrade
- **WHEN** the core is upgraded while an agent session has appended segments
- **THEN** the old core writes the whole checkpoint before it steps down, and the new core carries the conversation on

#### Scenario: A checkpoint in another form
- **WHEN** a core adopts an agent session whose holder stores a checkpoint whose records carry the whole tool index and no tool epoch
- **THEN** the session's conversation says it could not be restored and why, its agent is left running, and no entry of the stored log is shown

#### Scenario: Continued in a new holder session
- **WHEN** an ended agent session is continued, and its new agent's first checkpoint is written
- **THEN** that checkpoint is whole, and a core killed afterwards rebuilds the session with its earlier entries and their ids
