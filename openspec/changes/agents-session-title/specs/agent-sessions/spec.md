## MODIFIED Requirements

### Requirement: The transcript names the agent's conversation
Once the agent has given an ACP session, the transcript SHALL hold that session's id, the absolute directory it works in, whether the agent offered `session/load`, `session/resume`, both or neither, and the conversation's title once the agent has given one, and every replay SHALL carry them.

#### Scenario: Conversation named after session/new
- **WHEN** the fake ACP agent offers `loadSession` and `sessionCapabilities.resume` in `initialize` and answers `session/new` with session id `s1`
- **THEN** the client receives the conversation `s1` with the session's directory, marked as offering both load and resume and with no title, before the agent's ready state

#### Scenario: Conversation in a replay
- **WHEN** a client attaches an agent session whose agent answered `session/new` with session id `s1`
- **THEN** the replay holds the conversation `s1`

#### Scenario: Titled conversation in a replay
- **WHEN** a client attaches an agent session whose agent titled its conversation `Hello.txt file creation`
- **THEN** the replay holds the conversation with that title

### Requirement: Transcript event forms are fixed for each protocol version
On a connection that negotiated client protocol 4, the server SHALL send only the transcript event kinds and enumerated values of protocol 4 as first released; once a release has shipped protocol 4, a new kind or value SHALL come with a new client protocol version. Within protocol 4 an event MAY gain optional fields, which a protocol 4 client SHALL be able to ignore.

#### Scenario: Every event decodes
- **WHEN** a protocol 4 client decodes every transcript event of an agent session's turn, queue, permission, cancel, login, trim and exit
- **THEN** each decodes as one of protocol 4's event forms

#### Scenario: An unknown field is ignored
- **WHEN** a protocol 4 client decodes a protocol 4 event that carries an extra field it doesn't know
- **THEN** it decodes to the same event as without the field

#### Scenario: A login demand without methods
- **WHEN** a protocol 4 client decodes the "login required" agent state with a message and no methods
- **THEN** its encoding is the same as before login methods existed

#### Scenario: A conversation without a title
- **WHEN** a protocol 4 client decodes a conversation event the agent gave no title
- **THEN** its encoding is the same as before conversation titles existed

## ADDED Requirements

### Requirement: The agent names the conversation
When the agent sends `session_info_update` with a string `title`, the server SHALL keep it as the conversation's title, its control characters dropped, each run of whitespace made one space, trimmed, and cut at 256 characters with `…`, and SHALL send the conversation with it to attached clients. A `null` or blank `title` SHALL clear the title; an update without `title`, or a title equal to the one kept, SHALL send nothing.

#### Scenario: A title
- **WHEN** the fake ACP agent sends `session_info_update` with title `Hello.txt file creation` during a turn
- **THEN** the client receives the conversation with title `Hello.txt file creation`

#### Scenario: No title in the update
- **WHEN** the agent sends `session_info_update` with only `_meta`
- **THEN** the client receives nothing for it, and the conversation keeps its title

#### Scenario: Cleared
- **WHEN** a titled conversation's agent sends `session_info_update` with title `null`
- **THEN** the client receives the conversation with no title

#### Scenario: A long title on several lines
- **WHEN** the agent sends a title of 400 characters with line breaks in it
- **THEN** the kept title is one line of at most 256 characters, ending `…`

#### Scenario: The same title again
- **WHEN** the agent sends `session_info_update` with the title already kept
- **THEN** the client receives nothing for it

### Requirement: A conversation keeps its title
The conversation's title SHALL survive a core restart and an upgrade, and an agent restart that continues the same ACP session, including a load whose replayed updates aren't shown. A new ACP session SHALL start with no title.

#### Scenario: Core killed after a title
- **WHEN** the agent has titled its conversation, the core is killed with SIGKILL, and a new core adopts the session
- **THEN** a client attaching afterwards receives the conversation with its title

#### Scenario: Restarted after a login
- **WHEN** a titled conversation's agent asks for a login on a prompt, and the session is restarted and resumes the same ACP session
- **THEN** the conversation keeps its title

#### Scenario: A new session
- **WHEN** an agent session continues a conversation the server no longer has, in a new server session
- **THEN** its conversation has no title until the agent sends one

### Requirement: The agent's readable name
The agent's name in its ready state, and in the session list, SHALL be `initialize`'s `agentInfo.title` when it is given and not blank, else its `agentInfo.name`.

#### Scenario: A title is given
- **WHEN** the agent answers `initialize` with `agentInfo` name `@agentclientprotocol/codex-acp` and title `Codex`
- **THEN** the client receives the ready state with the name `Codex`

#### Scenario: No title
- **WHEN** the agent answers `initialize` with `agentInfo` name `Antigravity ACP Server` and no title
- **THEN** the client receives the ready state with the name `Antigravity ACP Server`
