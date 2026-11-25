## ADDED Requirements

### Requirement: Slash command completion
When the prompt input's text starts with `/` and the cursor is in its first word, an agent zone SHALL list above the input the agent's commands that match the word typed after `/`, each with its input hint and description, commands whose names start with the word first. The list SHALL follow the text as the user types, and SHALL NOT show when no command matches or the agent offers none.

#### Scenario: Typing a slash
- **WHEN** the agent offers `compact`, `context` and `review`, and the user types `/`
- **THEN** the list shows all three with their descriptions, the first highlighted

#### Scenario: Narrowing
- **WHEN** the user then types `co`
- **THEN** the list shows `compact` and `context`, and not `review`

#### Scenario: No match
- **WHEN** the user types `/zzz`
- **THEN** no list shows

#### Scenario: Past the first word
- **WHEN** the user types `/compact keep the plan`
- **THEN** no list shows

#### Scenario: A slash later in the text
- **WHEN** the user types `see src/main.rs`
- **THEN** no list shows

### Requirement: Choosing a slash command
While the command list shows, up and down SHALL move its highlight, enter and tab SHALL replace the typed word with the highlighted command's name followed by a space, and escape SHALL close the list until the text stops being a command word; none of these SHALL send the prompt, move keyboard focus or stop a turn. A press on a row SHALL choose that command. A chosen command SHALL be sent only when the user sends the prompt, as text.

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
