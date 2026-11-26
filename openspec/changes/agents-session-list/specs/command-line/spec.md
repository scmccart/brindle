## ADDED Requirements

### Requirement: Continuing a conversation from the command line
Under `agent`, Brindle SHALL also accept `-c/--continue`, which opens the agent tab on the last conversation the profile's agent had in the launch's directory, and `-r/--resume`, which opens it on the list of that agent's conversations there. Either MAY be given with `-d` and with `-p`, whose prompt SHALL be sent once the chosen conversation's agent is ready. `-c` and `-r` together, either under `terminal`, and either given twice SHALL print usage and exit with status 2.

#### Scenario: Continue the last
- **WHEN** the agent has conversations in `~/src/brindle`, the newest titled `Add paging`, and the user runs `brindle agent Claude -c` in a shell there
- **THEN** an agent tab opens showing `Add paging`, with no list shown

#### Scenario: Choose from the list
- **WHEN** the user runs `brindle agent Claude -r` in `~/src/brindle`
- **THEN** an agent tab opens with the list of `Claude`'s conversations in `~/src/brindle`, and the one chosen is shown in that tab

#### Scenario: The list dismissed
- **WHEN** that list is closed with escape
- **THEN** the tab stays, choosing, and offers Conversations… and New conversation

#### Scenario: With a directory and a prompt
- **WHEN** the user runs `brindle agent Claude -c -d ~/src/api -p 'run the tests'`
- **THEN** the last conversation in `~/src/api` is continued, and receives `run the tests` once its agent is ready

#### Scenario: Both flags
- **WHEN** the user runs `brindle agent Claude -c -r`
- **THEN** Brindle prints usage naming the options and exits with status 2, opening nothing

### Requirement: Continue with nothing to continue
`brindle agent … -c` in a directory where the agent has no earlier conversation SHALL open the agent tab choosing, saying that there is no earlier conversation in that directory, with New conversation offered. It SHALL NOT start a new conversation on its own. The same SHALL hold, with its reason, when the agent can't list its conversations.

#### Scenario: Nothing to continue
- **WHEN** the agent has no conversation in `~/src/new` and the user runs `brindle agent Claude -c` there
- **THEN** the tab opens saying there is no earlier conversation in `~/src/new`, no conversation is started, and New conversation starts one

#### Scenario: An agent that can't list
- **WHEN** the profile's agent doesn't offer a list and the user runs `brindle agent Fake -c`
- **THEN** the tab opens saying the agent can't list conversations, with New conversation offered

### Requirement: A continuing launch handed to an older Brindle
A `brindle agent` launch with `-c` or `-r` that is handed to a running Brindle from before these options SHALL NOT open a new conversation in their place: the launch SHALL print that the running Brindle is older and can't continue a conversation, and exit with status 1.

#### Scenario: An older Brindle runs
- **WHEN** a Brindle from before this change runs, and the user runs `brindle agent Claude -c` with this version
- **THEN** no tab opens, the launch says the running Brindle can't continue a conversation, and exits with status 1

### Requirement: Conversations in the dump
The dump SHALL give a choosing agent's state as `choosing`. An open conversations palette SHALL print `--- palette conversations profile="…" dir="…" state=starting|listing|listed|empty|failed more=yes|no`, then its rows as palette rows print, a row of a conversation a zone shows ending in `open`; the question of where to open SHALL print as a palette list of its rows.

#### Scenario: A listed palette
- **WHEN** the conversations palette shows two conversations, one of them open in a zone, and the dump is taken
- **THEN** it holds the `--- palette conversations` line with `state=listed`, then the two rows, the second ending in `open`

#### Scenario: A choosing zone
- **WHEN** `brindle agent Fake -c --dump-screen-after 3` runs against a fake agent with no conversations
- **THEN** the dump's agent line has `state=choosing`, and its prompt line's reason says there is no earlier conversation in the directory

### Requirement: Driving the conversations palettes
The choosing screen's buttons SHALL be focus stops named `conversations` and `new` in the dump's focus line. `--wait-for` SHALL match the conversations palette's lines, and the list actions SHALL drive the conversations palette and the question of where to open.

#### Scenario: Driving a choice
- **WHEN** a run uses `--action agent_continue_conversation`, waits for `state=listed`, then `--action list_confirm` twice
- **THEN** the first conversation is opened where the question's first row says
