## ADDED Requirements

### Requirement: The agent's login methods are offered with a login demand
The server SHALL keep the login methods the agent advertised in `initialize` (`authMethods`), in the agent's order, and SHALL send them with every "login required" agent state, each with its id, name, optional description and kind. A method with no `type`, or type `agent`, SHALL be of kind `agent`. A method of any other type the server doesn't run SHALL be left out. Text from the agent SHALL be cleaned and bounded as other transcript text.

#### Scenario: Methods sent with the demand
- **WHEN** the agent advertises methods `oauth-personal` ("Log in with Google") and `api-key` ("API Key", described "Use an API key") and answers `session/new` with error `-32000` and message `log in first`
- **THEN** the client receives the agent state "login required" with `log in first` and both methods in that order, each of kind `agent`, with `api-key`'s description

#### Scenario: No methods advertised
- **WHEN** the agent advertises no methods and answers `session/new` with `-32000`
- **THEN** the agent state is "login required" with the agent's message and no methods, and nothing else changes

#### Scenario: A method of an unknown type
- **WHEN** the agent advertises a method of type `env_var` and one with no type, and needs a login
- **THEN** the login-required state carries only the method with no type

### Requirement: A client logs the agent in through the server
A client attached to an agent session whose agent needs a login SHALL be able to choose one of its `agent` methods (`AGENT_LOGIN`). The server SHALL send `authenticate` with that method's id, make the agent state "logging in" with the method's id and name, and wait for the answer however long it takes. A login request for a method the agent doesn't offer, while the agent doesn't need a login, or while a login runs SHALL be refused with an error saying why, and change nothing.

#### Scenario: Logging in
- **WHEN** the agent needs a login and the client chooses `oauth-personal`
- **THEN** the agent receives `authenticate` with `methodId` `oauth-personal`, the client receives the agent state "logging in" with that method, and the session's activity is `working`

#### Scenario: No answer for an hour
- **WHEN** the agent takes an hour to answer `authenticate`
- **THEN** the session stays "logging in" for that hour, and nothing is sent to the agent meanwhile

#### Scenario: Method not offered
- **WHEN** the client asks to log in with `no-such-method`
- **THEN** it gets an error naming the method, the agent receives nothing, and the agent still needs a login

#### Scenario: Not needed
- **WHEN** the client asks to log in while the agent is ready
- **THEN** it gets an error saying the agent isn't waiting for a login, and the agent receives nothing

### Requirement: A successful login carries the session on
When the agent answers `authenticate` successfully, the agent state SHALL become "starting". If the login was demanded by `session/new`, the server SHALL send `session/new` again, and the session SHALL become ready as at a start. If it was demanded by a prompt, the session SHALL become ready again with the same ACP session, the agent's name and its version, and nothing SHALL be sent to the agent on its own.

#### Scenario: Login at the start
- **WHEN** the agent answered `session/new` with `-32000`, the client logs in, and the agent answers `authenticate` with `{}` and the second `session/new` with a session id
- **THEN** the client receives the agent state "starting", then "ready" with the agent's name, and prompts are taken

#### Scenario: Login demanded by a prompt
- **WHEN** a ready agent answered a prompt with `-32000`, the client logs in, and the agent answers `authenticate` with `{}`
- **THEN** the agent state becomes "starting", then "ready", the agent receives no `session/new` and no prompt, and the next prompt goes to the same ACP session

#### Scenario: Still not logged in
- **WHEN** the agent answers the `session/new` sent after a successful login with `-32000` again
- **THEN** the agent state is "login required" with the agent's new message and its methods

### Requirement: A failed login offers the methods again
When the agent answers `authenticate` with an error, the agent state SHALL become "login required" again, with the agent's error message and its methods, and the client MAY choose a method again.

#### Scenario: Login refused
- **WHEN** the client logs in with `chat-gpt` and the agent answers `authenticate` with error message `browser login timed out`
- **THEN** the agent state is "login required" with `browser login timed out` and the same methods, and a second login request is accepted

### Requirement: A login can be cancelled
While the agent is logging in, a client's cancel SHALL send the agent `$/cancel_request` naming the `authenticate` request, and make the agent state "login required" again with the message it had before the login and its methods. The server SHALL ignore a later answer to the cancelled request.

#### Scenario: Cancelled login
- **WHEN** the client logs in with `oauth-personal` and cancels before the agent answers
- **THEN** the agent receives `$/cancel_request` with the `authenticate` request's id, and the agent state is "login required" with its earlier message and methods

#### Scenario: Late answer
- **WHEN** a login was cancelled, and the agent then answers the cancelled `authenticate` with `{}`
- **THEN** the agent state stays "login required", and the agent receives no `session/new`

### Requirement: Prompts while a login is needed
While the agent needs a login or is logging in, prompts and resends SHALL be refused with an error saying so. A prompt whose turn failed with `-32000` SHALL end its turn as failed and SHALL never be sent again by the server. A prompt waiting at the demand SHALL be held because a login is needed, and once the agent is ready again the client SHALL be able to send it again or discard it.

#### Scenario: Held prompt after a login
- **WHEN** the agent answered a prompt with `-32000` while another prompt was waiting, and the client logs in successfully
- **THEN** the held prompt stays held, the agent has received neither prompt again, and a resend of the held prompt sends it as a new turn

#### Scenario: Prompt while a login is needed
- **WHEN** the agent answered `session/new` with `-32000` and message `run agent login`, and the client sends a prompt
- **THEN** it gets an error saying the agent needs a login, nothing is queued, and the session's activity is `error`

#### Scenario: Prompt while logging in
- **WHEN** the client sends a prompt while the agent is logging in
- **THEN** it gets an error saying the agent is logging in, and nothing is queued

### Requirement: A login survives core restarts and upgrades
A core restart or an upgrade SHALL keep an agent session's login state: its methods, its login-required message, the ACP session a prompt's demand left, and a running login with its `authenticate` request id, so that the agent's answer is applied by the next core.

#### Scenario: Core killed while logging in
- **WHEN** the core is killed with SIGKILL while the agent is logging in, a new core adopts the holder, and the agent then answers `authenticate` with `{}`
- **THEN** the client, after reconnecting, sees the agent state go from "logging in" to "starting" and "ready"

### Requirement: Terminal login methods
The server SHALL advertise `clientCapabilities.auth.terminal: true` in `initialize`, and SHALL offer a method of type `terminal` with kind `terminal`, its arguments and its environment. It SHALL refuse a login request for a `terminal` method, saying the client runs it.

#### Scenario: Advertised
- **WHEN** a client starts an agent session running the fake ACP agent
- **THEN** the agent receives `initialize` with `clientCapabilities.auth.terminal` `true`, and no `fs` and no `terminal` capability

#### Scenario: Terminal method offered
- **WHEN** the agent advertises a method `cli-login` of type `terminal` with args `["login"]` and env `{"LOGIN_MODE":"device"}`, and needs a login
- **THEN** the login-required state carries `cli-login` of kind `terminal` with those args and env

#### Scenario: Not run by the server
- **WHEN** the client asks the server to log in with the `terminal` method `cli-login`
- **THEN** it gets an error saying the client runs that method, and the agent receives no `authenticate`

### Requirement: An agent that needs a login can be restarted
A client's restart request while the agent needs a login SHALL hang up the running agent and start it again with the command, arguments and environment the request gives, keeping the log, the slot and any ACP session, then make the session ready as a recovery does: with `session/new` when no ACP session was made, else by taking up that session.

#### Scenario: Restarted after a terminal login
- **WHEN** a ready agent answered a prompt with `-32000`, and the client asks to restart it with its command, arguments and environment
- **THEN** the old agent is hung up, a new agent process is sent `initialize` with `auth.terminal`, the conversation and the held prompt are kept, and the session becomes ready in the same ACP session

## MODIFIED Requirements

### Requirement: Cancelling a turn
A client attached to an agent session SHALL be able to cancel the running turn. The server SHALL first hold a waiting prompt, then send `session/cancel`, answer each of the session's pending permission requests with the `cancelled` outcome and record that outcome, and end the turn when the agent answers the prompt. While the agent is logging in, a cancel SHALL cancel the login instead (see "A login can be cancelled"). A cancel with no turn running and no login running SHALL change nothing.

#### Scenario: Cancel while waiting for permission
- **WHEN** a turn is waiting on a permission request and the client cancels
- **THEN** the agent receives `session/cancel` and the `cancelled` outcome for that request, and the turn ends with stop reason `cancelled` once the agent answers

#### Scenario: Cancel holds the waiting prompt
- **WHEN** a turn is running with a prompt waiting, and the client cancels
- **THEN** the client is told the prompt is held because the turn was cancelled, with its text, before the turn ends, and the agent never receives it

#### Scenario: Cancel while logging in
- **WHEN** the agent is logging in and the client cancels
- **THEN** the agent receives `$/cancel_request` and no `session/cancel`, and the agent state is "login required"

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

### Requirement: The session list shows agent sessions
On protocol 4 the session list SHALL mark each agent session with its activity (`idle`, `working`, `blocked`, `done` or `error`, the names of program status) and the name the agent gave in `initialize`. `working` covers starting, logging in and a running turn, `blocked` a pending permission request, `done` a turn that ended with `end_turn`, `idle` a cancelled turn or a new session, and `error` everything that stopped the agent or a turn, a login demand included.

#### Scenario: Blocked and done
- **WHEN** one agent session waits on a permission request and another has finished a turn with `end_turn`
- **THEN** the list shows the first as `blocked` and the second as `done`

#### Scenario: Turn refused by the model
- **WHEN** an agent ends a turn with stop reason `refusal`
- **THEN** the session's activity is `error`

#### Scenario: Logging in
- **WHEN** an agent session's agent is logging in
- **THEN** the list shows it as `working`

## REMOVED Requirements

### Requirement: Login is left to the agent's own tools
**Reason**: The server now offers the agent's login methods and runs an `agent` login on the client's request ("The agent's login methods are offered with a login demand", "A client logs the agent in through the server").
**Migration**: The `-32000` handling it described stays: the session keeps running, a prompt's turn fails and is never sent again, a waiting prompt is held, prompts are refused and the activity is `error` ("Prompts while a login is needed"). Only the refusal to call `authenticate` goes.
