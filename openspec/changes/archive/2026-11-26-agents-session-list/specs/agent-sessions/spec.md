## ADDED Requirements

### Requirement: An agent session can start by choosing
A client SHALL be able to start an agent session that, once its agent has answered `initialize`, starts no conversation and waits for a choice, its state saying it is choosing. A client attached to it SHALL then be able to choose one of the agent's conversations by its id, which the server SHALL continue in the session's directory as it continues any conversation, or to choose a new one, which the server SHALL start. A choice made when the session isn't choosing SHALL be refused with an error.

#### Scenario: Waiting for a choice
- **WHEN** a client starts an agent session that is to choose, and its agent answers `initialize`
- **THEN** the agent receives no `session/new`, and attached clients are told the agent is choosing

#### Scenario: A conversation chosen
- **WHEN** a client then chooses the conversation `s7`, and the agent offers `session/resume`
- **THEN** the agent receives `session/resume` for `s7` in the session's directory, and the session becomes ready with that conversation

#### Scenario: Chosen with a replay
- **WHEN** the agent offers only `session/load` and a client chooses `s7`
- **THEN** the session's log holds the conversation as the agent replays it, after a notice that it continues an earlier conversation

#### Scenario: A new conversation chosen
- **WHEN** a client chooses a new conversation
- **THEN** the agent receives `session/new`, and the session becomes ready as a newly started one does

#### Scenario: A prompt before the choice
- **WHEN** a client sends a prompt to a session that is choosing
- **THEN** it gets an error saying the agent isn't ready

#### Scenario: A choice out of turn
- **WHEN** a client chooses a conversation in a session whose agent is already ready
- **THEN** it gets an error saying the session isn't choosing

#### Scenario: Across a restart
- **WHEN** a session is choosing, the core restarts, and a client attaches the session
- **THEN** the replay says the agent is choosing, and a choice is carried out

### Requirement: Listing an agent's conversations
A client attached to an agent session whose agent is ready or choosing SHALL be able to ask for the agent's earlier conversations in the session's directory, a page at a time. The server SHALL answer with each conversation's id, its title and when it was last active, as the agent gives them, and with what to ask for the next page when there is one. Conversations of any other directory SHALL be left out, whether or not the agent leaves them out. The server SHALL keep no list.

#### Scenario: A page of conversations
- **WHEN** a client asks, and the agent answers `session/list` with two conversations of the session's directory, titled `Fix the upload test` and `Add paging`
- **THEN** the client receives both, each with its id, title and time

#### Scenario: Only this directory
- **WHEN** the agent's answer also holds a conversation whose directory is another
- **THEN** the client's reply leaves it out

#### Scenario: The next page
- **WHEN** the agent's answer has a cursor for more, and the client asks again with it
- **THEN** the agent receives `session/list` with that cursor, and the client receives the next conversations

#### Scenario: Listed by a choosing agent
- **WHEN** a client asks a session that is choosing
- **THEN** it receives the agent's conversations, and the session is still choosing

#### Scenario: None
- **WHEN** the agent has no conversation in the session's directory
- **THEN** the client receives an empty list, with nothing more to ask for

### Requirement: Refusing a list
A request for an agent's conversations SHALL be refused with an error saying why when the agent doesn't offer a list, when the agent is neither ready nor choosing, when an earlier list for the session hasn't been answered, and when the agent answers with an error, whose message SHALL be given. An agent that needs a login to list SHALL be shown as needing one, as for any request that does, and SHALL return to what it was doing after the login.

#### Scenario: An agent without a list
- **WHEN** a client asks for the conversations of an agent that doesn't offer `session/list`
- **THEN** it gets an error saying the agent can't list conversations, and the agent receives nothing

#### Scenario: One list at a time
- **WHEN** a client asks for a list, and asks again before the agent has answered
- **THEN** the second request gets an error saying a list is in progress

#### Scenario: The agent's error
- **WHEN** the agent answers `session/list` with an error whose message is `index unavailable`
- **THEN** the client gets an error holding `index unavailable`

#### Scenario: A login to list
- **WHEN** a choosing agent answers `session/list` with the error that a login is needed, and the user logs in
- **THEN** the session says a login is needed, and is choosing again once the login succeeds

### Requirement: Listed conversations are bounded
A reply SHALL hold at most 256 conversations, with titles made one line and cut at 256 characters, and ids and times bounded as other agent text is. A conversation the server can't read SHALL be left out without losing the others.

#### Scenario: A long page
- **WHEN** the agent answers with 400 conversations and a cursor
- **THEN** the client receives the first 256 and can ask for more

#### Scenario: One unreadable conversation
- **WHEN** the agent's answer holds three conversations, the second with no id
- **THEN** the client receives the first and the third

#### Scenario: A title of several lines
- **WHEN** a conversation's title is a whole prompt of several lines
- **THEN** the client receives it as one line of at most 256 characters
