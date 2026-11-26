## MODIFIED Requirements

### Requirement: Continuing a conversation
A zone whose agent exited or was lost with a conversation it can continue SHALL show a Continue conversation button, and the `agent_continue` action (Continue This Conversation) SHALL do the same. Either SHALL start the zone's agent profile's command again with its current arguments and environment on the same session, or in a new session when the server lost it, and the zone SHALL say the agent is starting until it is ready. Nothing SHALL be continued without the user asking.

#### Scenario: Continue after a crash
- **WHEN** an agent zone's agent exited after one turn, and the user presses Continue conversation
- **THEN** the zone says the agent is starting, then shows the earlier turn, and the prompt input takes a prompt that reaches the agent

#### Scenario: From the palette
- **WHEN** an agent zone's agent exited and the user runs Continue This Conversation from the palette
- **THEN** the conversation is continued as from the button

#### Scenario: Never on its own
- **WHEN** an agent zone's agent exits and a minute passes
- **THEN** the agent hasn't been started again

#### Scenario: Replay not shown
- **WHEN** a zone is continued with an agent that offers only `session/load`
- **THEN** the conversation shows the earlier turns once, then a notice that the conversation continued and the agent's replay wasn't shown

#### Scenario: Continued in a new session
- **WHEN** a zone says its session was lost, and the user continues the conversation
- **THEN** the zone shows a new session's conversation, as the agent replayed it, after a notice that it continues an earlier conversation

## ADDED Requirements

### Requirement: The conversations palette
`Agent Conversations…`, in an agent zone, SHALL open a palette listing the earlier conversations of that zone's agent profile in that zone's directory. `Continue Agent Conversation…`, in any pane, SHALL do the same for an agent profile: the config's one agent profile, or one chosen first from a list when it has several. The palette's title SHALL name the profile and the directory.

#### Scenario: Listed from an agent zone
- **WHEN** an agent zone of profile `Claude` in `~/src/brindle` runs Agent Conversations…, and the agent lists `Fix the upload test` and `Add paging`
- **THEN** the palette is titled with `Claude` and `~/src/brindle`, and lists both

#### Scenario: From a shell
- **WHEN** a shell in `~/src/brindle` has focus, the config has one agent profile `Claude`, and the user runs Continue Agent Conversation…
- **THEN** the palette lists `Claude`'s conversations in `~/src/brindle`

#### Scenario: Several agent profiles
- **WHEN** the config has agent profiles `Claude` and `Codex` and the user runs Continue Agent Conversation… from a shell
- **THEN** a list asks which, and choosing `Codex` lists `Codex`'s conversations

### Requirement: Conversation rows
Each row of the conversations palette SHALL show the conversation's title, or that it is untitled with the start of its id, and when it was last active when the agent says, in the order the agent lists them. When the agent has more than it listed, a last row SHALL ask for more, and SHALL add them below those shown. When it has none, the palette SHALL say that there is no earlier conversation in the directory.

#### Scenario: Title and time
- **WHEN** the agent lists `Fix the upload test`, last active two hours ago
- **THEN** its row shows that title and `2 hours ago`

#### Scenario: Untitled
- **WHEN** the agent lists a conversation with no title
- **THEN** its row reads `Untitled` followed by the start of its id

#### Scenario: More conversations
- **WHEN** the agent has more conversations than one page, and the user chooses the last row, `More…`
- **THEN** the next page's conversations are added below those shown

#### Scenario: None
- **WHEN** the agent has no conversation in that directory
- **THEN** the palette says there is no earlier conversation in that directory

### Requirement: The directory a list is for
The conversations listed SHALL be those of one directory: an agent zone's own directory; for a terminal pane, its working directory when it runs on the agent profile's host; otherwise the profile's directory, else the home directory on its host. Conversations of other directories SHALL NOT be listed.

#### Scenario: The shell's directory
- **WHEN** a local shell is in `~/src/api` and the user lists the local agent profile `Claude`'s conversations
- **THEN** the conversations listed are those of `~/src/api`

#### Scenario: Another host
- **WHEN** a local shell is in `~/src/api` and the user lists the conversations of an agent profile that runs on the SSH host `A8`
- **THEN** the conversations listed are those of that profile's directory on `A8`, or the home directory there

### Requirement: Which agent lists
A list for a profile SHALL be answered by a zone of that profile on that host whose agent is ready in the directory wanted, when the window has one. Otherwise Brindle SHALL start that profile's agent to list, the palette saying that it is starting, and SHALL end it when the palette closes without a choice. An agent that can't be started, needs a login, or can't list SHALL be said in the palette, with nothing listed.

#### Scenario: A running agent answers
- **WHEN** an agent zone's agent is ready and the user runs Agent Conversations… there
- **THEN** the list comes from that agent, and no other agent is started

#### Scenario: Started to list
- **WHEN** no agent of profile `Claude` runs and the user runs Continue Agent Conversation… from a shell
- **THEN** the palette says `Claude` is starting, and then lists its conversations

#### Scenario: Closed without a choice
- **WHEN** that palette is closed with escape
- **THEN** the agent started to list is ended, and no agent session is left for it

#### Scenario: An agent that can't list
- **WHEN** the profile's agent doesn't offer a list of conversations
- **THEN** the palette says the agent can't list conversations

#### Scenario: A login is needed
- **WHEN** the agent started to list needs a login
- **THEN** the palette says so and that opening the profile lets the user log in, and lists nothing

### Requirement: Where a chosen conversation opens
A conversation chosen from the palette SHALL open in the pane the palette was opened from when that pane is an agent zone with no agent running: one whose agent stopped, or one still choosing. From any other pane, a shell or an agent zone whose agent runs, the palette SHALL ask whether to split the pane to the right, split it down, or open a new tab, and SHALL open the conversation there. Escape at that question SHALL return to the list.

#### Scenario: Into a stopped zone
- **WHEN** an agent zone's agent has exited, and the user chooses a conversation from its list
- **THEN** that zone shows the chosen conversation, its agent starting, and the stopped conversation is no longer shown

#### Scenario: From a shell, split
- **WHEN** a shell has focus and the user chooses a conversation, then Split Right
- **THEN** an agent zone showing that conversation opens to the shell's right, in the same tab

#### Scenario: From a shell, a new tab
- **WHEN** the user chooses a conversation from a shell, then New Tab
- **THEN** a new active tab shows that conversation

#### Scenario: From a running agent
- **WHEN** an agent zone's agent is working and the user chooses another conversation from its list
- **THEN** the palette asks where to open it, and the working zone is left as it is

#### Scenario: Back to the list
- **WHEN** the question shows and the user presses escape
- **THEN** the list shows again, with nothing opened

#### Scenario: Too small to split
- **WHEN** the pane is too small to split
- **THEN** Split Right and Split Down are disabled, and New Tab can be chosen

### Requirement: A conversation already open is shown
A listed conversation that a zone of the window already shows SHALL be marked as open, and choosing it SHALL make that zone's tab active and focus the zone, starting nothing.

#### Scenario: Already open
- **WHEN** a zone in another tab shows the conversation `Add paging`, and the user chooses it from the list
- **THEN** that tab becomes active with that zone focused, and no agent is started

### Requirement: One agent start for a list and its choice
When Brindle started an agent to list, the conversation chosen SHALL be continued by that same agent, without another being started.

#### Scenario: Listed and continued
- **WHEN** the user lists `Claude`'s conversations from a shell, with no `Claude` agent running, and chooses one into a new tab
- **THEN** the conversation is shown by the agent that listed it, and the profile's command was started once

### Requirement: The choosing screen
An agent zone whose agent is choosing SHALL say, where a prompt's reason is shown, that a conversation is to be chosen or a new one started, or the reason it holds instead, and SHALL offer Conversations…, which opens the list for the zone, and New conversation, which starts one in the zone. An agent that can't list SHALL be offered New conversation alone, with that reason.

#### Scenario: Choose from the screen
- **WHEN** a zone is choosing and the user presses Conversations… and chooses a conversation
- **THEN** the zone shows that conversation, and its prompt input takes a prompt once the agent is ready

#### Scenario: A new conversation
- **WHEN** a zone is choosing and the user presses New conversation
- **THEN** the agent starts a new conversation in the zone, as a newly opened agent zone has

#### Scenario: The list dismissed
- **WHEN** a choosing zone's list is closed with escape
- **THEN** the zone is still choosing, with both buttons

#### Scenario: Keyboard
- **WHEN** a zone is choosing and the user presses tab from its conversation
- **THEN** focus reaches Conversations… and then New conversation

### Requirement: A choosing zone waits quietly
A choosing zone's prompt input SHALL take no prompt until a conversation starts. The zone SHALL show no working row, no spinner and no status that asks for attention, and SHALL raise no notification.

#### Scenario: No prompt yet
- **WHEN** a zone is choosing and the user types into its prompt input and presses enter
- **THEN** nothing is sent, and the reason line says a conversation is to be chosen first

#### Scenario: Nothing is working
- **WHEN** a zone in a background tab is choosing
- **THEN** its tab shows no spinner and no status dot for it

### Requirement: A stopped zone's choices
A zone whose agent exited or was lost SHALL offer, beside Continue conversation where it can continue, Conversations…, which lists its profile's conversations in its directory, and New conversation, which starts a new conversation in the zone in place of the stopped one. A zone whose agent profile no longer exists SHALL offer neither.

#### Scenario: Another conversation
- **WHEN** an agent zone's agent exited, and the user presses Conversations… and chooses a conversation
- **THEN** the zone shows the chosen conversation

#### Scenario: Start over
- **WHEN** an agent zone's agent exited and the user presses New conversation
- **THEN** the zone shows a new, empty conversation with its agent starting

#### Scenario: Can't continue, can choose
- **WHEN** an agent that offered neither `session/load` nor `session/resume` exits
- **THEN** the zone has no Continue conversation button, and has Conversations… and New conversation

#### Scenario: Profile deleted
- **WHEN** an agent zone's agent exited and its profile has since been removed from the config
- **THEN** the zone offers none of the three

### Requirement: A choice that can't be continued
When the agent refuses to continue a chosen conversation, a zone that was choosing SHALL be choosing again, showing the agent's message, and a zone opened for the choice SHALL show that the agent can't be used, with the message.

#### Scenario: Refused in a choosing zone
- **WHEN** a choosing zone's chosen conversation is answered by the agent with the error `no such session`
- **THEN** the zone is choosing again and shows `no such session`
