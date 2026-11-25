## ADDED Requirements

### Requirement: Prompts carry attachments
A client SHALL be able to send, with a prompt's text, a list of attachments of two kinds: a link to a file, by its path and name, and text with a name. The server SHALL send them to the agent after the prompt's text, in the client's order, and SHALL add them to the user message's log entry, where every attached client and every replay SHALL have them.

#### Scenario: A prompt with a link and a text
- **WHEN** a client sends the prompt `why does this fail?` with a text attachment named `cargo test` and a link to `src/main.rs`
- **THEN** the agent receives the prompt's text, then the attachment `cargo test`, then the link, and the log's user entry holds both attachments in that order

#### Scenario: Replayed
- **WHEN** that session is detached and a client attaches it
- **THEN** the replay holds the user message with its two attachments

#### Scenario: A prompt without attachments
- **WHEN** a client sends a prompt with no attachments
- **THEN** the agent receives the text alone, as before

### Requirement: Prompt attachments are bounded
The server SHALL refuse, with an error saying why, a prompt with more than 8 attachments, a text attachment over 64 KiB, or text attachments of more than 128 KiB together, and nothing SHALL be added to the log or queued. An attachment's text, name and path SHALL have their control characters replaced as prompt text has.

#### Scenario: Too many
- **WHEN** a client sends a prompt with 9 attachments
- **THEN** it gets an error naming the limit of 8, and the agent receives nothing

#### Scenario: A text that is too long
- **WHEN** a client sends a prompt with a text attachment of 100 KiB
- **THEN** it gets an error naming the 64 KiB limit

#### Scenario: Too much text together
- **WHEN** a client sends a prompt with three text attachments of 60 KiB each
- **THEN** it gets an error naming the 128 KiB limit

### Requirement: A linked file is named by its place on the agent's host
The server SHALL send a link to the agent as a file URI for the path resolved on the agent's host: a relative path against the session's working directory, and `~` as the session's home. The log SHALL keep the path as the client gave it. The server SHALL NOT read the file or check that it exists.

#### Scenario: A relative path
- **WHEN** a session's working directory is `/home/u/proj` and a client attaches a link to `src/main.rs`
- **THEN** the agent receives a link to `file:///home/u/proj/src/main.rs`, and the log entry's link reads `src/main.rs`

#### Scenario: A path with a space
- **WHEN** a client attaches a link to `/home/u/my notes.md`
- **THEN** the agent receives a link to `file:///home/u/my%20notes.md`

#### Scenario: A file that isn't there
- **WHEN** a client attaches a link to a path that doesn't exist
- **THEN** the prompt is sent with the link, and the session carries on

### Requirement: Attached text reaches every agent
The server SHALL send a text attachment as embedded context, named, to an agent that says it accepts embedded context. To an agent that doesn't, it SHALL send the text inside the prompt's own text, after the user's, under the attachment's name and marked off so its lines can't be taken for the user's words. An attachment whose text was cut SHALL say so to the agent, with which end was dropped.

#### Scenario: Embedded
- **WHEN** the agent accepts embedded context and a client sends a prompt with a text attachment `cargo test` holding `error: mismatched types`
- **THEN** the agent receives the prompt's text and a separate embedded text named `cargo test` holding `error: mismatched types`

#### Scenario: An agent without embedded context
- **WHEN** the agent doesn't accept embedded context and a client sends the same prompt
- **THEN** the agent receives one text holding the user's words, then `cargo test` and `error: mismatched types` in a marked block

#### Scenario: Text with fences in it
- **WHEN** that agent is sent an attachment whose text holds a line of three backticks
- **THEN** the block's own marks are longer than any run of backticks in the text

#### Scenario: A cut text
- **WHEN** a client sends a text attachment marked as having lost its start
- **THEN** what the agent receives for it says its beginning isn't included

### Requirement: Attachments stay with a queued or held prompt
A prompt that waits in the queue slot, or is held there, SHALL keep its attachments: in what attached clients are told, in every replay, across detaches, core restarts and upgrades, and when it is sent, by the queue or by a client sending it again. Discarding the prompt SHALL discard them.

#### Scenario: Queued with an attachment
- **WHEN** a client sends a prompt with a text attachment during a turn, and the turn ends with `end_turn`
- **THEN** the agent receives the queued prompt with its attachment

#### Scenario: Held across a restart
- **WHEN** a prompt with an attachment is held, the core restarts, and a client attaches the session and sends the held prompt again
- **THEN** the agent receives the prompt with its attachment

#### Scenario: Discarded
- **WHEN** a client discards a waiting prompt that has an attachment
- **THEN** the slot is empty, and the agent never receives the prompt or the attachment
