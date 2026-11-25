## ADDED Requirements

### Requirement: The agent's commands are kept
The server SHALL keep the latest list of slash commands the agent offers: each command's name, description and input hint. A new list from the agent SHALL replace the old one. The list SHALL be sent to attached clients when it changes, SHALL be in every replay, and SHALL survive detaches, core restarts and upgrades.

#### Scenario: Commands offered
- **WHEN** the agent sends its available commands `compact` and `review`, the second with the input hint `optional review instructions`
- **THEN** an attached client receives both commands, the second with its hint

#### Scenario: Replaced
- **WHEN** the agent sends commands `a` and `b`, then a list holding only `c`
- **THEN** the session's commands are `c` alone

#### Scenario: Replayed
- **WHEN** a session whose agent offered two commands is detached and a client attaches it
- **THEN** the replay holds both commands

#### Scenario: The same list again
- **WHEN** the agent sends the command list it already sent
- **THEN** attached clients receive nothing for it

### Requirement: Only commands that can be typed are kept
A command whose name is empty, or holds whitespace, a control character or `/`, SHALL be left out. At most 256 commands SHALL be kept, with texts bounded as tool call titles are; when the agent offers more, the log SHALL get one notice saying how many were offered and how many are kept.

#### Scenario: A name with a space
- **WHEN** the agent offers commands `review` and `do thing`
- **THEN** the session's commands are `review` alone

#### Scenario: Too many commands
- **WHEN** the agent offers 300 commands
- **THEN** the first 256 are kept, and a notice says 300 were offered and 256 are kept

### Requirement: The agent's session options are kept
The server SHALL keep the latest list of session options the agent offers: each option's name, description, category (mode, model, model configuration, thinking level, or the agent's own name for another), and either its choices with the current one or an on/off value. Choices the agent groups SHALL keep their group's name. The list SHALL be sent to attached clients when it changes, SHALL be in every replay, and SHALL survive detaches, core restarts and upgrades.

#### Scenario: Options at the start
- **WHEN** the agent answers `session/new` with a `model` option whose current value is `opus` and a `mode` option whose current value is `auto`
- **THEN** an attached client receives both options with their choices and current values

#### Scenario: The agent changes an option
- **WHEN** the agent then reports its options with the model `sonnet`
- **THEN** attached clients receive the options with `sonnet` current

#### Scenario: A toggle
- **WHEN** the agent offers a boolean option `Fast mode` that is off
- **THEN** the session's options hold it as an on/off option that is off

#### Scenario: Kept across a restart
- **WHEN** a session has options, the core restarts, and a client attaches the session
- **THEN** the replay holds the same options

### Requirement: Session options are bounded
At most 32 options SHALL be kept, each with at most 256 choices, with texts bounded as tool call titles are. An option the server can't read, or of a kind it doesn't know, SHALL be left out without losing the others. What is left out SHALL be said in one notice.

#### Scenario: One unreadable option
- **WHEN** the agent offers three options, the second with no name
- **THEN** the first and third are kept, and a notice says one option couldn't be read

#### Scenario: An option of an unknown kind
- **WHEN** the agent offers an option of type `slider`
- **THEN** it is left out, and the other options are kept

### Requirement: Session modes are an option
When an agent offers session modes and no option of category mode, the server SHALL present the modes as one option of category mode whose choices are the modes and whose current value is the current mode, and SHALL follow the agent's mode updates. When the agent offers an option of category mode, its session modes SHALL NOT add a second one.

#### Scenario: Modes only
- **WHEN** the agent answers `session/new` with modes `default` and `plan`, current `default`, and no config options
- **THEN** the session's options are one mode option with those two choices and `default` current

#### Scenario: The agent changes its mode
- **WHEN** that agent then reports its current mode as `plan`
- **THEN** attached clients receive the mode option with `plan` current

#### Scenario: Both offered
- **WHEN** the agent answers `session/new` with modes and with a config option of category mode
- **THEN** the session's options hold one mode option, the config option

### Requirement: A client sets a session option
A client attached to an agent session SHALL be able to ask for one option to be set to one value. The server SHALL ask the agent only when the agent is ready, the option is one the session holds, and the value is one of its choices or an on/off value for an on/off option; otherwise it SHALL refuse with an error saying why. The agent's answer SHALL become the session's options. A request naming the option's current value SHALL succeed and send nothing.

#### Scenario: Model changed
- **WHEN** a client sets the `model` option to `sonnet`, one of its choices, and the agent answers with its options, `sonnet` current
- **THEN** the agent received the request for `sonnet`, and every attached client receives the options with `sonnet` current

#### Scenario: A value that wasn't offered
- **WHEN** a client sets the `model` option to `gpt`, which isn't among its choices
- **THEN** it gets an error naming the option and the value, and the agent receives nothing

#### Scenario: Before the agent is ready
- **WHEN** a client sets an option before the agent has answered `session/new`
- **THEN** it gets an error saying the agent isn't ready

#### Scenario: A mode that came from session modes
- **WHEN** the session's mode option came from the agent's session modes and a client sets it to `plan`
- **THEN** the agent receives `session/set_mode` for `plan`, and attached clients receive the mode option with `plan` current

#### Scenario: During a turn
- **WHEN** a client sets the mode option while a turn is running
- **THEN** the agent receives the request, and the turn goes on

### Requirement: One option change at a time
While the agent hasn't answered a request to set an option, the server SHALL refuse another for that session with an error saying a change is in progress.

#### Scenario: A second change
- **WHEN** a client sets the model, and sets the thinking level before the agent has answered
- **THEN** the second request gets an error saying a change is in progress, and the agent receives only the first

### Requirement: An agent's refusal to change an option is shown
When the agent answers a request to set an option with an error, the session's options SHALL stay as they were, and the log SHALL get a notice naming the option, the value and the agent's message.

#### Scenario: Refused by the agent
- **WHEN** a client sets the model to `opus` and the agent answers with an error whose message is `not available on this plan`
- **THEN** the options are unchanged, and the conversation gets a notice saying the agent didn't change the model to `opus`: `not available on this plan`

### Requirement: A change of mode is recorded
When an option of category mode gets a different current value, the server SHALL add a notice to the log naming the new mode, whether a client or the agent changed it. The first options a session receives SHALL add none, and other categories SHALL add none.

#### Scenario: The user changes the mode
- **WHEN** a client sets the mode option to the choice named `Plan` and the agent confirms it
- **THEN** the conversation gets a notice `Mode: Plan`

#### Scenario: The agent changes its own mode
- **WHEN** the agent reports its options with the mode `Accept edits` current, with no client having asked
- **THEN** the conversation gets a notice `Mode: Accept edits`

#### Scenario: The model changes
- **WHEN** the model option's current value changes
- **THEN** no notice is added
