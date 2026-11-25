## ADDED Requirements

### Requirement: Prompts carry attachments
A client SHALL be able to send, with a prompt's text, a list of attachments of two kinds: a link to a file, by its path and name, and text with a name. The server SHALL send them to the agent after the prompt's text, in the client's order, and SHALL record them in the user message's log entry, where every attached client and every replay SHALL have them.

#### Scenario: A prompt with a link and a text
- **WHEN** a client sends the prompt `why does this fail?` with a text attachment named `cargo test` and a link to `src/main.rs`
- **THEN** the agent receives the prompt's text, then the attachment `cargo test`, then the link, and the log's user entry lists both attachments in that order

#### Scenario: Replayed
- **WHEN** that session is detached and a client attaches it
- **THEN** the replay holds the user message with its two attachments

#### Scenario: A prompt without attachments
- **WHEN** a client sends a prompt with no attachments
- **THEN** the agent receives the text alone, as before

### Requirement: Attached text is kept beside the log
The server SHALL keep a text attachment's text in a file of its own, in a directory private to the user on the agent's host, and SHALL NOT put the text in the log, the queue slot or a checkpoint: those SHALL hold its name, its size, whether it was cut, and the server's id for it. The file SHALL be named by that id alone, and SHALL NOT be writable once written.

#### Scenario: The log holds a reference
- **WHEN** a client sends a prompt with a 100 KiB text attachment
- **THEN** the log's user entry holds the attachment's name, size and id, and a replay of the session is no larger for the text

#### Scenario: A hostile name
- **WHEN** a client attaches text under the name `../../etc/passwd`
- **THEN** the text is kept in a file named by its id inside the session's attachment directory, and nothing is written elsewhere

#### Scenario: Private
- **WHEN** a text attachment has been kept on Linux
- **THEN** its directory is accessible to the user alone, and its file can't be written

### Requirement: Where attached text is kept
The directory that keeps attached text SHALL be under the user's cache directory on the agent's host: `$BRINDLE_CACHE_DIR` when set, else `$XDG_CACHE_HOME/brindle`, else `~/.cache/brindle`, and on Windows `%LOCALAPPDATA%\brindle\cache`. It SHALL NOT be the terminal server's own directory, so kept text is on disk and is still there after a reboot.

#### Scenario: The default place
- **WHEN** neither `BRINDLE_CACHE_DIR` nor `XDG_CACHE_HOME` is set and a text attachment is kept for a user whose home is `/home/u`
- **THEN** its file is under `/home/u/.cache/brindle/attachments/`

#### Scenario: Named by the environment
- **WHEN** the terminal server was started with `BRINDLE_CACHE_DIR=/tmp/case-7`
- **THEN** kept text is written under `/tmp/case-7/attachments/`, and nothing is written under the user's own cache directory

### Requirement: A client can read an attachment's text
A client attached to an agent session SHALL be able to ask for a kept attachment's text by its id, and SHALL receive that text. A request for an id the session doesn't hold, or whose text is no longer kept, SHALL be refused with an error saying so.

#### Scenario: Read back
- **WHEN** a client sent a text attachment holding `error: mismatched types`, and later asks for that attachment's text
- **THEN** it receives `error: mismatched types`

#### Scenario: After a reattach
- **WHEN** a client detaches, another attaches the session, and asks for the attachment listed in the replay
- **THEN** it receives the same text

#### Scenario: An unknown id
- **WHEN** a client asks for attachment 99 of a session that has kept three
- **THEN** it gets an error saying there is no such attachment

### Requirement: Prompt attachments are bounded
The server SHALL refuse, with an error saying why, a prompt with more than 8 attachments, a text attachment over 256 KiB, and a text attachment that would bring the session's kept text over 16 MiB; nothing of a refused prompt SHALL be added to the log, queued or kept. An attachment's text, name and path SHALL have their control characters replaced as prompt text has.

#### Scenario: Too many
- **WHEN** a client sends a prompt with 9 attachments
- **THEN** it gets an error naming the limit of 8, and the agent receives nothing

#### Scenario: A text that is too long
- **WHEN** a client attaches a text of 300 KiB
- **THEN** it gets an error naming the 256 KiB limit

#### Scenario: The store is full
- **WHEN** a session's kept text is at 16 MiB and a client attaches more
- **THEN** it gets an error saying how much the conversation's attachments hold

#### Scenario: A refused prompt keeps nothing
- **WHEN** a client attaches a text and its prompt is then refused because the agent isn't ready
- **THEN** no file for that text remains

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
The server SHALL send a text attachment of at most 64 KiB as embedded context, named, to an agent that says it accepts embedded context. A larger one, and any text attachment for an agent that doesn't accept embedded context, SHALL be sent as a link to the file that keeps it. An attachment whose text was cut SHALL say so to the agent, with which end was dropped.

#### Scenario: Embedded
- **WHEN** the agent accepts embedded context and a client sends a prompt with a text attachment `cargo test` holding `error: mismatched types`
- **THEN** the agent receives the prompt's text and a separate embedded text named `cargo test` holding `error: mismatched types`

#### Scenario: A large text
- **WHEN** that agent is sent a text attachment of 200 KiB
- **THEN** it receives a link to a file on its host that holds the 200 KiB, and the text isn't in the prompt

#### Scenario: An agent without embedded context
- **WHEN** the agent doesn't accept embedded context and a client sends a prompt with a 1 KiB text attachment
- **THEN** the agent receives a link to the file that keeps it

#### Scenario: A cut text
- **WHEN** a client sends a text attachment marked as having lost its start
- **THEN** what the agent receives for it says its beginning isn't included

### Requirement: Attachments stay with a queued or held prompt
A prompt that waits in the queue slot, or is held there, SHALL keep its attachments: in what attached clients are told, in every replay, across detaches, core restarts and upgrades, and when it is sent, by the queue or by a client sending it again.

#### Scenario: Queued with an attachment
- **WHEN** a client sends a prompt with a text attachment during a turn, and the turn ends with `end_turn`
- **THEN** the agent receives the queued prompt with its attachment

#### Scenario: Held across a restart
- **WHEN** a prompt with a text attachment is held, the core restarts, and a client attaches the session and sends the held prompt again
- **THEN** the agent receives the prompt with its attachment's text

### Requirement: Kept text is removed when nothing refers to it
While its session lives, the server SHALL remove a kept text when its queued or held prompt is discarded, when its log entry is dropped by the log's bound, and when a client that attached it detaches or disconnects before a prompt took it. A kept text that is missing when its prompt is sent SHALL be left out, with a notice naming it, and the prompt SHALL still be sent.

#### Scenario: Discarded
- **WHEN** a client discards a waiting prompt that has a text attachment
- **THEN** the slot is empty, the agent never receives the prompt, and the attachment's file is gone

#### Scenario: A client that never sent
- **WHEN** a client attaches a text and disconnects without sending a prompt
- **THEN** no file for that text remains

#### Scenario: A missing file
- **WHEN** a held prompt's kept text has been deleted from the directory, and a client sends the prompt again
- **THEN** the agent receives the prompt without that attachment, and the conversation gets a notice naming it

### Requirement: Kept text outlasts its session by a week
When an agent session ends, the text kept for it SHALL stay for 7 days, so that a link the agent was given still leads to its file if the conversation is taken up again, and SHALL then be removed. The server SHALL remove such directories when it starts and whenever a session ends, and SHALL NOT remove the directory of a session that is still live, however old its files.

#### Scenario: Still there after the session
- **WHEN** an agent session with a kept attachment is closed, and its directory is looked at an hour later
- **THEN** the attachment's file is still there

#### Scenario: Gone after a week
- **WHEN** a session ended 8 days ago, and the terminal server starts
- **THEN** that session's attachment directory is removed

#### Scenario: A long-lived session
- **WHEN** a live session's kept text is 30 days old and another session ends
- **THEN** the live session's text is still there

#### Scenario: After a reboot
- **WHEN** the machine is restarted a day after a session kept an attachment
- **THEN** the attachment's file is still at the path the agent was given
