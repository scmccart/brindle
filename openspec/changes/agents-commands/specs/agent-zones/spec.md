## ADDED Requirements

### Requirement: Slash command completion
When the prompt input's text starts with `/` and the cursor is in its first word, an agent zone SHALL list above the input the commands that match the word typed after `/`: the agent's commands and Brindle's option commands, each with its input hint and description, commands whose names start with the word first. The list SHALL follow the text as the user types, and SHALL NOT show when no command matches.

#### Scenario: Typing a slash
- **WHEN** the agent offers `compact`, `context` and `review`, and the user types `/`
- **THEN** the list shows all three with their descriptions, the first highlighted

#### Scenario: Narrowing
- **WHEN** the user then types `co`
- **THEN** the list shows `compact` and `context`, and not `review`

#### Scenario: No match
- **WHEN** the user types `/zzz`
- **THEN** no list shows

#### Scenario: Past the first word of an agent's command
- **WHEN** the user types `/compact keep the plan`
- **THEN** no list shows

#### Scenario: A slash later in the text
- **WHEN** the user types `see src/main.rs`
- **THEN** no list shows

### Requirement: Choosing a slash command
While the command list shows, up and down SHALL move its highlight, enter and tab SHALL replace the typed word with the highlighted command's name followed by a space, and escape SHALL close the list until the text stops being a command word; none of these SHALL send the prompt, move keyboard focus or stop a turn. A press on a row SHALL choose that command. A chosen command of the agent's SHALL be sent only when the user sends the prompt, as text.

#### Scenario: Enter takes the command
- **WHEN** the list shows `compact` highlighted and the user presses enter
- **THEN** the prompt input holds `/compact ` with the cursor at its end, nothing is sent, and the list is closed

#### Scenario: Then enter sends
- **WHEN** the user presses enter again
- **THEN** the agent receives the prompt `/compact`

#### Scenario: Escape closes the list
- **WHEN** the list shows while a turn is running and the user presses escape
- **THEN** the list closes, the turn goes on, and the text is unchanged

#### Scenario: Without a list, keys are as before
- **WHEN** no list shows and the user presses tab
- **THEN** focus moves to the zone's next stop

### Requirement: Option commands
An agent zone SHALL offer one slash command of Brindle's own for each session option, named from the option's name in lower case with each run of other characters as one `-` (`/mode`, `/model`, `/thinking-level`, `/fast-mode`). When the agent offers a command of the same name, the agent's SHALL be the one offered.

#### Scenario: Listed with the agent's commands
- **WHEN** the agent offers the command `compact` and options named `Mode` and `Model`, and the user types `/mo`
- **THEN** the list shows `/mode` and `/model`, each described by its option's current value

#### Scenario: The agent has its own
- **WHEN** the agent offers a command named `model` and an option named `Model`
- **THEN** `/model` in the list is the agent's command, and sending `/model x` sends that text to the agent

### Requirement: Option commands stay in Brindle
A prompt that is an option command SHALL be carried out by Brindle, and SHALL NOT be sent to the agent, queued or added to the conversation as a user message. Text that doesn't start with `/`, and text of more than one line, SHALL always be sent as it is.

#### Scenario: Not a user message
- **WHEN** the user sends `/mode plan` and the agent confirms the plan mode
- **THEN** the agent received a request to set the mode and no prompt, and the conversation has no user message `/mode plan`

#### Scenario: A leading space
- **WHEN** the user sends ` /mode plan`, with a space before the slash
- **THEN** the agent receives that text as a prompt

### Requirement: Setting an option with its command
After an option command and a space, the list SHALL show that option's choices that match the rest of the line, by name or value, without regard to case, all of them when nothing follows, with the current choice highlighted first. Enter SHALL ask the server to set the highlighted choice and empty the prompt input. An option command SHALL be carried out while a turn is running.

#### Scenario: Set the model by typing
- **WHEN** the model option's choices are `Opus` and `Sonnet`, and the user types `/model so` and presses enter
- **THEN** the agent is asked for `Sonnet`, the prompt input is empty, and the conversation has no user message for it

#### Scenario: All the choices
- **WHEN** the user types `/mode ` with a space
- **THEN** the list shows every choice of the mode option, the current one marked

#### Scenario: A choice with a space in its name
- **WHEN** the mode option has a choice named `Accept edits` and the user types `/mode accept e` and presses enter
- **THEN** the agent is asked for `Accept edits`

#### Scenario: During a turn
- **WHEN** a turn is running and the user sends `/mode plan`
- **THEN** the agent is asked for the plan mode at once, and nothing is queued

### Requirement: Option commands that name no choice
Text after an option command that matches none of its choices SHALL NOT be sent anywhere: the prompt input SHALL keep it and say the option has no such choice. An on/off option's command SHALL accept `on` and `off`, and SHALL flip the option when sent alone.

#### Scenario: No such choice
- **WHEN** the user types `/model gpt` and presses enter, and no choice matches
- **THEN** nothing is sent, the text stays, and the prompt input says the model has no choice `gpt`

#### Scenario: A toggle set by word
- **WHEN** the session has an on/off option `Fast mode` that is off, and the user sends `/fast-mode on`
- **THEN** the agent is asked to turn it on

#### Scenario: A toggle flipped
- **WHEN** `Fast mode` is on and the user sends `/fast-mode` alone
- **THEN** the agent is asked to turn it off

### Requirement: Session options under the prompt input
An agent zone SHALL show its session's options under the prompt input as their current values: for an option with choices the current choice's name, for an on/off option its name and `on` or `off`, in the order mode, model, thinking level, model configuration, then the agent's others. It SHALL follow the session's options as they change, and SHALL show nothing when the session has none. The items SHALL NOT be keyboard stops.

#### Scenario: Shown
- **WHEN** the session's options are a model `Opus`, a mode `Auto` and a thinking level `Medium`
- **THEN** the zone shows `Auto`, `Opus` and `Medium`, in that order, under the prompt input

#### Scenario: Follows the agent
- **WHEN** the agent reports the mode `Plan` current
- **THEN** the zone shows `Plan` in place of `Auto`

#### Scenario: No options
- **WHEN** the agent offers no options
- **THEN** nothing shows under the prompt input

### Requirement: Choosing a session option
A pointer press on an option under the prompt input SHALL list that option's choices, grouped as the agent grouped them, with the current one marked, and choosing one SHALL ask the server to set it; a press on an on/off option SHALL ask for the other value. The `agent_options` action SHALL list every option with its current value and lead to the same choices.

#### Scenario: Pick a model with the pointer
- **WHEN** the user presses `Opus` under the prompt input and chooses `Sonnet` from the list
- **THEN** the agent is asked for `Sonnet`, and once it confirms the zone shows `Sonnet`

#### Scenario: From the keyboard
- **WHEN** the user runs Agent Options… from the command palette, chooses the mode option, and chooses `Plan`
- **THEN** the agent is asked for `Plan`

#### Scenario: A toggle
- **WHEN** the user presses `Fast mode off` under the prompt input
- **THEN** the agent is asked to turn `Fast mode` on

### Requirement: A session option shows what the server reports
The value an agent zone shows for an option SHALL change only when the server reports the option changed, never when the user chooses. A request to set an option that the server refuses SHALL show its reason on the window's notice line and change nothing.

#### Scenario: Not changed until confirmed
- **WHEN** the user chooses `Sonnet` and the agent hasn't answered
- **THEN** the zone still shows `Opus`

#### Scenario: Refused
- **WHEN** the user chooses a model while an earlier change is still unanswered
- **THEN** the notice line says a change is in progress, and the options are unchanged

### Requirement: Stepping the mode
The `agent_next_mode` action SHALL ask the server to set the session's mode option to the choice after its current one, wrapping to the first. The command palette SHALL list it as Next Agent Mode, and Agent Options…, while an agent zone is active and its session has such an option, and neither in a terminal zone.

#### Scenario: Next mode
- **WHEN** the mode option's choices are `Manual`, `Accept edits` and `Plan` with `Accept edits` current, and the user runs Next Agent Mode
- **THEN** the agent is asked for `Plan`

#### Scenario: Wraps
- **WHEN** `Plan`, the last choice, is current and the user runs Next Agent Mode
- **THEN** the agent is asked for `Manual`

#### Scenario: No mode option
- **WHEN** the session has no option of category mode
- **THEN** the command palette doesn't list Next Agent Mode
