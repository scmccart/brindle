# Spec Delta

## MODIFIED Requirements

### Requirement: Launch options
Brindle SHALL accept:
- `terminal [PROFILE]`, the subcommand that asks for a terminal tab, with PROFILE matched case-insensitively against the config's profile names, and under it:
  - `-d/--working-directory <DIR>`;
  - `-e/--command <CMD> [ARGS]...`, which runs CMD instead of the profile's program and takes every argument after it, so it comes last;
- `agent [PROFILE]`, the subcommand that asks for an agent tab, with PROFILE matched as under `terminal`, and under it:
  - `-d/--working-directory <DIR>`; without it the agent starts in the profile's `cwd`, else in the working directory of the `brindle agent` command;
  - `-p/--prompt <PROMPT>`, a first prompt, sent once the agent is ready (see `agent-zones`' "A launch's first prompt"); `-p -` reads it from standard input (see "A first prompt from standard input");
- `--config <FILE>`;
- `--no-restore`, which skips session restore for this launch;
- `--attach <ID>` (Linux and Windows), which opens a tab attached to the detached terminal server session `ID`, an agent tab for an agent session. When the terminal server's sessions can't be listed, that tab's message SHALL say so, and SHALL NOT say that session `ID` doesn't exist. It SHALL NOT be combined with a subcommand.

PROFILE, `-d` and `-p` MAY come in any order after their subcommand, and before `-e`. An unknown argument, an option given without the value it takes (`-e` without a command included), a value an option can't use, a blank PROMPT, a PROMPT over 256 KiB of UTF-8, `-p` under `terminal`, `-e` under `agent`, a second PROFILE or `-p`, and `--attach` given with a subcommand SHALL print usage, naming the option or argument, and exit with status 2.

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

### Requirement: A launch restores, then opens what it asks for
A launch that becomes the instance (see `session-restore`'s "One process owns the state") SHALL restore the recorded session and reattach detached sessions, as defined by `session-restore`, unless it has `--no-restore`. It SHALL then open what it asks for as the active tab: `brindle terminal` a new terminal tab with its profile, directory and command, `brindle agent` a new agent tab with its profile, directory and first prompt, `--attach` a tab attached to that session. A launch with none of them SHALL open no further tab, or one tab of the default profile, whatever its kind, when nothing was restored. The debug options (`--send`, `--action`, `--dump-screen-after`) act on the active tab after this.

#### Scenario: Profile launch
- **WHEN** Brindle quit with two tabs, and the user runs `brindle terminal Work`
- **THEN** the window shows the two restored tabs and an active tab with the `Work` profile

#### Scenario: Terminal without a profile
- **WHEN** Brindle quit with two tabs, and the user runs `brindle terminal`
- **THEN** the window shows the two restored tabs and an active tab with the default profile

#### Scenario: Agent launch
- **WHEN** Brindle quit with two tabs, and the user runs `brindle agent Agent`
- **THEN** the window shows the two restored tabs and an active agent tab of `Agent`

#### Scenario: Plain launch
- **WHEN** Brindle quit with two tabs, and the user runs `brindle`
- **THEN** the window shows the two restored tabs and no other tab

#### Scenario: Skip once
- **WHEN** the user runs `brindle --no-restore`
- **THEN** the window opens with a single default-profile tab

#### Scenario: Agent as the default profile
- **WHEN** the default profile is the agent profile `Agent` and the user runs `brindle --no-restore`
- **THEN** the window opens with a single agent tab of `Agent`

### Requirement: A launch goes to the running Brindle
A launch without debug options while an instance runs (one with the same state directory) SHALL send its request to the instance and exit with status 0 once the instance has accepted it. The instance SHALL raise its window and open what the launch asked for, as "A launch restores, then opens what it asks for" says, without restoring again; a `brindle terminal -e` command SHALL start in the launching process's working directory, as SHALL a `brindle agent` launch's agent when neither `-d` nor its profile sets a directory, and a `brindle agent -p` prompt, read by the launching process, SHALL be sent by the instance once that agent is ready. When the instance doesn't accept the request within 5 seconds, the launch SHALL print why and exit with status 1. The request channel SHALL be reachable only by the same user. Before sending anything, the launch SHALL check that the instance runs as the same user and, on Windows, at the same integrity level, and SHALL give it no right to act as the launching user; otherwise it SHALL send nothing, print why and exit with status 1. On Windows, a launch with no console that gives up (refused, or not accepted within 5 seconds) SHALL also show why in a message box. On Windows an elevated and an unelevated Brindle have different default state directories, so each is its own instance: a launch at either level, while a Brindle at the other runs, opens its own window, and the integrity check is reached only when `BRINDLE_STATE_DIR` names one directory for both.

#### Scenario: Second plain launch
- **WHEN** Brindle runs with two tabs and the user launches `brindle` from the desktop
- **THEN** the window is raised, it still has two tabs, and no second window opens

#### Scenario: New tab from a shell
- **WHEN** Brindle runs with two tabs and the user runs `brindle terminal` in a shell
- **THEN** the `brindle` command exits, and Brindle's window gains a third, active tab with the default profile

#### Scenario: Command from a shell
- **WHEN** Brindle runs and the user runs `brindle terminal -e htop` in a shell in `/srv`
- **THEN** the `brindle` command exits, and Brindle's window gains an active tab running htop in `/srv`

#### Scenario: Agent with a prompt from a shell
- **WHEN** Brindle runs and the user runs `brindle agent Agent -p 'run the tests'` in a shell in `~/src/brindle`, and `Agent` sets no `cwd`
- **THEN** the `brindle` command exits without waiting for the agent, and Brindle's window gains an active agent tab in `~/src/brindle` that sends `run the tests` once the agent is ready

#### Scenario: Attach from a shell
- **WHEN** Brindle runs, session 7 is detached, and the user runs `brindle --attach 7`
- **THEN** Brindle's window gains an active tab showing session 7

#### Scenario: Pipe served by someone else on Windows
- **WHEN** another account's process has created the instance pipe's name, and the user runs `brindle terminal -e pwsh`
- **THEN** the launch sends nothing to that process, prints that the pipe's server is not this user's Brindle, and exits with status 1

#### Scenario: Elevated launch beside an unelevated Brindle on Windows
- **WHEN** an unelevated Brindle runs, and the user starts `brindle` elevated without `BRINDLE_STATE_DIR`
- **THEN** a second, elevated window opens with the elevated Brindle's own session, and the unelevated window is unchanged

#### Scenario: Unelevated launch beside an elevated Brindle on Windows
- **WHEN** an elevated Brindle runs, and the user starts `brindle` unelevated from the Start Menu without `BRINDLE_STATE_DIR`
- **THEN** an unelevated window opens with the unelevated Brindle's own session, the elevated window is unchanged, and no message box shows

#### Scenario: Shared state directory across levels on Windows
- **WHEN** an unelevated Brindle runs with `BRINDLE_STATE_DIR=C:\s`, and the user starts `brindle` elevated with the same `BRINDLE_STATE_DIR`
- **THEN** the elevated launch sends nothing, says that the running Brindle is at another integrity level, and exits with status 1

#### Scenario: Refused launch from the Start Menu
- **WHEN** a launch with no console is refused by the check above
- **THEN** a message box says why before the launch exits with status 1

### Requirement: Notification log for tests
When the environment variable `BRINDLE_NOTIFY_LOG` names a file, Brindle SHALL append each notification it would show to that file instead of sending it to the desktop, one line per notification: the source (`bell`, `osc9`, `osc777`, `osc7501` or `agent`), the title and the body, separated by tabs. Lines SHALL be appended in the order the notifications were sent. Writing them SHALL NOT hold up the window, and every line SHALL be in the file by the time Brindle has exited, even if it quits right after the notification, for example from `--dump-screen-after`.

#### Scenario: Logged notification
- **WHEN** Brindle runs with `BRINDLE_NOTIFY_LOG=/tmp/n.log`, `notifications.when = "always"`, and `--send 'printf "\033]9;hi\a"\r'`
- **THEN** `/tmp/n.log` contains a line `osc9<TAB><tab title><TAB>hi` and no desktop notification appears

#### Scenario: Quit right after notifications
- **WHEN** Brindle runs as `brindle terminal -e sh -c 'printf "\033]9;one\a\033]777;notify;t;two\a"'` with `BRINDLE_NOTIFY_LOG=/tmp/n.log` and `notifications.when = "always"`, so its only tab's program sends two notifications and exits, which quits Brindle
- **THEN** once the process has exited, `/tmp/n.log` holds the `osc9` line for `one` followed by the `osc777` line for `two`

#### Scenario: Logged agent notification
- **WHEN** Brindle runs with `BRINDLE_NOTIFY_LOG=/tmp/n.log` and `notifications.when = "always"`, and an agent named `Fake Agent` ends a turn normally
- **THEN** `/tmp/n.log` contains a line `agent<TAB>Fake Agent — <tab title><TAB>Done`

### Requirement: A launch's profile has its subcommand's kind
A profile named under a subcommand SHALL be of that subcommand's kind: a terminal profile under `terminal`, an agent profile under `agent`. Otherwise the launch SHALL be refused as a usage error naming the profile, its kind and the subcommand that opens it, with status 2 and nothing opened, checked as an unknown name is (see "Launch options"). A subcommand that names no profile SHALL use the default profile when it is of the subcommand's kind, else the first profile of that kind in the config; when the config has none, the launch SHALL be refused the same way, saying no profile of that kind exists.

#### Scenario: Terminal profile under the terminal subcommand
- **WHEN** the config has profiles `Shell` and `Work` and the user runs `brindle terminal Work`
- **THEN** an active terminal tab of the `Work` profile opens

#### Scenario: Unknown name under the terminal subcommand
- **WHEN** no profile is named `wrk` and the user runs `brindle terminal wrk`
- **THEN** the launch is refused for the unknown name, with status 2, and no tab opens

#### Scenario: Agent profile under the terminal subcommand
- **WHEN** `Agent` is an agent profile and the user runs `brindle terminal Agent`
- **THEN** Brindle prints that `Agent` is an agent profile, opened with `brindle agent Agent`, and exits with status 2 without opening a window

#### Scenario: Terminal profile under the agent subcommand
- **WHEN** `Shell` is a terminal profile and the user runs `brindle agent Shell`
- **THEN** Brindle prints that `Shell` is a terminal profile, opened with `brindle terminal Shell`, and exits with status 2 without opening a window

#### Scenario: Wrong kind while Brindle runs
- **WHEN** Brindle runs and the user runs `brindle agent Shell` in a shell, and `Shell` is a terminal profile
- **THEN** the launch prints that `Shell` is a terminal profile and exits with status 2, and the window gains no tab

#### Scenario: Agent without a profile, default is a terminal
- **WHEN** the default profile is `Shell`, the config's first agent profile is `Agent`, and the user runs `brindle agent`
- **THEN** an active agent tab of `Agent` opens

#### Scenario: Terminal without a profile, default is an agent
- **WHEN** the default profile is the agent profile `Agent`, the config's first terminal profile is `Shell`, and the user runs `brindle terminal`
- **THEN** an active terminal tab of `Shell` opens

#### Scenario: No agent profile
- **WHEN** the config has no agent profile and the user runs `brindle agent`
- **THEN** Brindle prints that the config has no agent profile and exits with status 2 without opening a window

### Requirement: The hand-off across versions
A running Brindle SHALL take launches from older and newer versions: a request naming a profile, directory or command, with no kind or the terminal kind, SHALL open a terminal tab; a request of the agent kind SHALL open an agent tab with the profile, directory and prompt it carries in its own agent fields; and a request naming a kind of tab it can't open SHALL be refused with nothing opened, the launch printing why and exiting with status 1. An agent request SHALL carry nothing in the fields a terminal request uses. A refusal for an unknown or wrong-kind profile, or for a missing profile of the kind, SHALL make a launch of this version exit with status 2. A plain launch's request SHALL stay as older versions read it.

#### Scenario: Older launch, newer Brindle
- **WHEN** a Brindle of this version runs and an older `brindle -p Work` launch hands it its request
- **THEN** the window gains an active tab with the `Work` profile

#### Scenario: Newer launch, older Brindle
- **WHEN** a Brindle from before `agents-launch-commands` runs and the user runs `brindle terminal Work -e htop` with this version
- **THEN** the older window gains an active `Work` tab running htop

#### Scenario: A kind the running Brindle can't open
- **WHEN** a Brindle of this version runs and a launch hands it a request for a kind of tab it doesn't know
- **THEN** it opens nothing and answers why, and the launch prints that answer and exits with status 1

#### Scenario: Unknown name, older Brindle
- **WHEN** a Brindle from before `agents-launch-commands` runs and the user runs `brindle terminal wrk` with this version, and no profile is named `wrk`
- **THEN** the older window opens its default profile with its dim line naming `wrk`, as it always has, and the launch exits with status 0

#### Scenario: Agent launch, Brindle without agent tabs
- **WHEN** a Brindle of `agents-launch-commands` but not of this change runs, and the user runs `brindle agent Agent` with this version
- **THEN** the running Brindle opens nothing and answers that it can't open agent tabs, and the launch prints that and exits with status 1

#### Scenario: Agent launch, Brindle before the subcommands
- **WHEN** a Brindle from before `agents-launch-commands` runs, and the user runs `brindle agent Agent -p hi` with this version
- **THEN** the older window is raised and opens no tab

## REMOVED Requirements

### Requirement: The agent subcommand is reserved
**Reason**: `brindle agent` now opens agent tabs (see "Launch options").
**Migration**: Run `brindle agent [PROFILE] [-d DIR] [-p PROMPT]`; a script that relied on the refusal gets an agent tab, or a usage error when the config has no agent profile.

## ADDED Requirements

### Requirement: A first prompt from standard input
`brindle agent … -p -` SHALL read the first prompt from the launching process's standard input, to its end, before any hand-off or window. Standard input that is a terminal or absent, empty or blank, not UTF-8, or over 256 KiB SHALL be refused as a usage error (status 2) saying which, with nothing opened. A launch handed to a running Brindle whose request would then exceed that Brindle's 1 MiB request limit SHALL send nothing and exit with status 1, as any oversized launch does.

#### Scenario: Prompt from a file
- **WHEN** `task.md` holds 200 KiB of text and the user runs `brindle agent Agent -p - < task.md`
- **THEN** an agent tab opens and sends the text of `task.md` once the agent is ready

#### Scenario: Standard input is a terminal
- **WHEN** the user runs `brindle agent Agent -p -` in an interactive shell with nothing redirected
- **THEN** Brindle prints that `-p -` reads standard input, which is a terminal, and exits with status 2 without waiting for input or opening a window

#### Scenario: Too long
- **WHEN** the user pipes 300 KiB into `brindle agent Agent -p -`
- **THEN** Brindle prints that the prompt is over the 256 KiB limit and exits with status 2 without opening a window

#### Scenario: Not UTF-8
- **WHEN** the user redirects a file holding bytes that aren't UTF-8 into `brindle agent Agent -p -`
- **THEN** Brindle prints that the prompt isn't UTF-8 and exits with status 2 without opening a window

#### Scenario: Piped on Windows
- **WHEN** on Windows the user runs `Get-Content -Raw task.md | brindle agent Agent -p -` in PowerShell, or `brindle agent Agent -p - < task.md` in cmd
- **THEN** an agent tab opens and sends the text of `task.md` once the agent is ready

### Requirement: Debug hooks in an agent zone
When the active zone is an agent zone, `--dump-screen-after` SHALL print, in place of the terminal's lines, a `--- agent` line (zone, state, name, turn, activity, status), a `--- entry` line per entry in order (id, kind, fields, text on one line), a `--- queued` line while the slot holds a prompt (waiting, or held and why), a `--- conversation` line (entry count, at the end or away) and a `--- prompt` line (status, refusal). `--wait-for` SHALL match those lines.

#### Scenario: A turn in the dump
- **WHEN** Brindle runs `brindle agent Fake -p hi --wait-for 'Hello!' --dump-screen-after 2` against a fake ACP agent that answers `Hello!`
- **THEN** the dump has `--- entry 1 user "hi"` and `--- entry 2 agent "Hello!"`, and a `--- agent` line with `turn=ended:end_turn activity=done`

#### Scenario: A pending permission in the dump
- **WHEN** the fake ACP agent asks permission with options `allow-once` and `reject-once`, and the dump is taken
- **THEN** the permission's `--- entry` line names both options and says it is pending

#### Scenario: A held prompt in the dump
- **WHEN** `next` was queued, the user stopped the turn, and the dump is taken
- **THEN** the dump has `--- queued held reason=cancelled "next"`

### Requirement: Driving an agent zone
`--send` SHALL type into the agent zone's prompt input, as into any focused text input. `--key` SHALL press keys there (enter sends, shift-enter adds a line, escape stops a turn, tab moves among the zone's stops), and `--action` SHALL run the agent actions. The `--- focus` line SHALL report `agent stop=prompt`, `agent stop=queued:remove`, `agent stop=queued:send-again`, `agent stop=queued:discard`, `agent stop=permission:<entry>:<option>` or `agent stop=stop`.

#### Scenario: Focus on an option
- **WHEN** permission request entry 3 with options `allow-once` and `reject-once` is pending, nothing is queued, the prompt input has focus, and Brindle runs `--key tab --key tab --dump-screen-after 2`
- **THEN** the dump's focus line reads `--- focus agent stop=permission:3:reject-once modality=keyboard`

#### Scenario: Answer from the keyboard
- **WHEN** the same request is pending and Brindle runs `--key tab --key tab --key enter --dump-screen-after 2`
- **THEN** the dump's permission entry says `reject-once` was chosen

#### Scenario: Send a held prompt from the keyboard
- **WHEN** `next` is held, no turn runs, the prompt input has focus, and Brindle runs `--key tab --key enter --dump-screen-after 2`
- **THEN** the dump has no `--- queued` line and an entry `user "next"`

### Requirement: Driving the close confirmation
While the close confirmation is open, `--dump-screen-after` SHALL print a `--- confirm close` line naming the zones it would close and their agents' statuses, and the `--- focus` line SHALL report `confirm stop=cancel` or `confirm stop=close`. `--key` SHALL drive it as real keys do.

#### Scenario: Confirmation in the dump
- **WHEN** zone 1's agent is running a turn and Brindle runs `--action close_zone --dump-screen-after 2`
- **THEN** the dump has a `--- confirm close` line naming zone 1 as `working`, and `--- focus confirm stop=cancel modality=keyboard`
