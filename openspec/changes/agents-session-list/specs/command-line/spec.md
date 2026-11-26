## MODIFIED Requirements

### Requirement: Launch options
Brindle SHALL accept:
- `terminal [PROFILE]`, the subcommand that asks for a terminal tab, with PROFILE matched case-insensitively against the config's profile names, and under it:
  - `-d/--working-directory <DIR>`;
  - `-e/--command <CMD> [ARGS]...`, which runs CMD instead of the profile's program and takes every argument after it, so it comes last;
- `agent [PROFILE]`, the subcommand that asks for an agent tab, with PROFILE matched as under `terminal`, and under it:
  - `-d/--working-directory <DIR>`; without it the agent starts in the profile's `cwd`, else in the working directory of the `brindle agent` command;
  - `-p/--prompt <PROMPT>`, a first prompt, sent once the agent is ready (see `agent-zones`' "A launch's first prompt"); `-p -` reads it from standard input (see "A first prompt from standard input");
  - `-c/--continue`, which continues the agent's last conversation in that directory, and `-r/--resume`, which opens the list of its conversations there (see "Continuing a conversation from the command line");
- `--config <FILE>`;
- `--no-restore`, which skips session restore for this launch;
- `--attach <ID>` (Linux and Windows), which opens a tab attached to the detached terminal server session `ID`, an agent tab for an agent session. When the terminal server's sessions can't be listed, that tab's message SHALL say so, and SHALL NOT say that session `ID` doesn't exist. It SHALL NOT be combined with a subcommand.

PROFILE, `-d`, `-p`, `-c` and `-r` MAY come in any order after their subcommand, and before `-e`. An unknown argument, an option given without the value it takes (`-e` without a command included), a value an option can't use, a blank PROMPT, a PROMPT over 256 KiB of UTF-8, `-p`, `-c` or `-r` under `terminal`, `-e` under `agent`, a second PROFILE, `-p`, `-c` or `-r`, `-c` with `-r`, and `--attach` given with a subcommand SHALL print usage, naming the option or argument, and exit with status 2.

A PROFILE that names no profile SHALL be refused, by the Brindle that would open the tab and against the config it runs with, with an error naming PROFILE and listing the profile names, and the launch SHALL exit with status 2 without opening a window or a tab, whether it would become the running Brindle or hand its request to one. When that config has an error and Brindle runs on the default config, the names listed SHALL be the default config's, and the error SHALL say the config has an error. No launch SHALL fall back to another profile for an unknown name.

#### Scenario: Run a command
- **WHEN** the user runs `brindle terminal -e htop`
- **THEN** a tab running htop opens and becomes active, and when htop exits its tab closes

#### Scenario: Profile, directory and command
- **WHEN** the user runs `brindle terminal -d /srv Work -e htop`
- **THEN** an active tab of the `Work` profile runs htop in `/srv`

#### Scenario: A profile named like a subcommand
- **WHEN** the config has a profile named `terminal` and the user runs `brindle terminal terminal`
- **THEN** an active tab of the `terminal` profile opens

#### Scenario: Agent with a directory and a prompt
- **WHEN** `Agent` is an agent profile and the user runs `brindle agent Agent -d ~/src -p 'run the tests'`
- **THEN** an active agent tab opens whose agent session starts in `~/src`, and `run the tests` is sent once the agent is ready

#### Scenario: Agent in the command's directory
- **WHEN** `Agent` sets no `cwd` and the user runs `brindle agent Agent` in a shell in `~/src/brindle`
- **THEN** the agent session starts in `~/src/brindle`

#### Scenario: A command for an agent
- **WHEN** the user runs `brindle agent Agent -e other-acp`
- **THEN** Brindle prints usage naming `-e`, saying an agent runs its profile's command, and exits with status 2 without opening a window

#### Scenario: Continue under the terminal subcommand
- **WHEN** the user runs `brindle terminal Work -c`
- **THEN** Brindle prints usage naming `-c` and exits with status 2, without opening a window

#### Scenario: Attach a session
- **WHEN** `brindle-remote list` shows a detached session 7 running `htop`, and the user runs `brindle --attach 7`
- **THEN** an active tab shows that `htop`, and the session is no longer detached

#### Scenario: Attach an agent session
- **WHEN** `brindle-remote list` shows a detached agent session 8, and the user runs `brindle --attach 8`
- **THEN** an active agent tab shows its conversation

#### Scenario: Attach a missing session
- **WHEN** the user runs `brindle --attach 99` and there is no session 99, or it is attached elsewhere
- **THEN** an active tab's message names session 99 and the reason

#### Scenario: Attach when the server can't be listed
- **WHEN** the user runs `brindle --attach 7` and Brindle can't list the terminal server's sessions
- **THEN** an active tab's message says the terminal server's sessions could not be listed

#### Scenario: Attach with a subcommand
- **WHEN** the user runs `brindle terminal Work --attach 7`
- **THEN** Brindle prints usage naming `--attach` and exits with status 2, without opening a window

#### Scenario: Unknown profile
- **WHEN** the config has profiles `Shell` and `Work`, and the user runs `brindle terminal wrk`
- **THEN** Brindle prints that no profile is named `wrk` and that the profiles are `Shell` and `Work`, and exits with status 2 without opening a window

#### Scenario: Unknown agent profile
- **WHEN** the config has profiles `Shell` and `Agent`, and the user runs `brindle agent agnt`
- **THEN** Brindle prints that no profile is named `agnt` and that the profiles are `Shell` and `Agent`, and exits with status 2 without opening a window

#### Scenario: Unknown profile while Brindle runs
- **WHEN** Brindle runs with two tabs and profiles `Shell` and `Work`, and the user runs `brindle terminal wrk` in a shell
- **THEN** the launch prints that no profile is named `wrk`, listing `Shell` and `Work`, and exits with status 2, and the window still has two tabs

#### Scenario: Profile added while Brindle runs
- **WHEN** Brindle runs, the user adds a profile `Logs` to the config, Brindle reloads it, and the user runs `brindle terminal Logs`
- **THEN** the window gains an active tab of the `Logs` profile

#### Scenario: Unknown profile with a broken config
- **WHEN** the config file has a syntax error, so Brindle runs on the default config, and the user runs `brindle terminal Work`
- **THEN** Brindle prints that no profile is named `Work`, that the config has an error, and the default config's profile names, and exits with status 2 without opening a window

#### Scenario: Matched without regard to case
- **WHEN** the config has a profile `Work` and the user runs `brindle terminal wORK`
- **THEN** an active tab of the `Work` profile opens

#### Scenario: Two profiles
- **WHEN** the user runs `brindle terminal Work Shell`
- **THEN** Brindle prints usage naming `Shell` and exits with status 2, without opening a window

#### Scenario: Option without its value
- **WHEN** the user runs `brindle terminal -d`, `brindle agent -p`, or `brindle --dump-screen-after soon`
- **THEN** Brindle prints usage naming the option and exits with status 2, without opening a window

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
