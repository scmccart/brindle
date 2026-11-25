## MODIFIED Requirements

### Requirement: Launch options
Brindle SHALL accept:
- `terminal [PROFILE]`, the subcommand that asks for a terminal tab, with PROFILE matched case-insensitively against the config's profile names, and under it:
  - `-d/--working-directory <DIR>`;
  - `-e/--command <CMD> [ARGS]...`, which runs CMD instead of the profile's program and takes every argument after it, so it comes last;
- `--config <FILE>`;
- `--no-restore`, which skips session restore for this launch;
- `--attach <ID>` (Linux and Windows), which opens a tab attached to the detached terminal server session `ID`. When the terminal server's sessions can't be listed, that tab's message SHALL say so, and SHALL NOT say that session `ID` doesn't exist. It SHALL NOT be combined with a subcommand.

PROFILE and `-d` MAY come in either order after the subcommand, and before `-e`. An unknown argument, an option given without the value it takes (`-e` without a command included), a value an option can't use, a second PROFILE, and `--attach` given with a subcommand SHALL print usage, naming the option or argument, and exit with status 2.

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

#### Scenario: Attach a session
- **WHEN** `brindle-remote list` shows a detached session 7 running `htop`, and the user runs `brindle --attach 7`
- **THEN** an active tab shows that `htop`, and the session is no longer detached

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
- **WHEN** the user runs `brindle terminal -d`, or `brindle --dump-screen-after soon`
- **THEN** Brindle prints usage naming the option and exits with status 2, without opening a window

### Requirement: A launch restores, then opens what it asks for
A launch that becomes the instance (see `session-restore`'s "One process owns the state") SHALL restore the recorded session and reattach detached sessions, as defined by `session-restore`, unless it has `--no-restore`. It SHALL then open what it asks for as the active tab: `brindle terminal` a new terminal tab with its profile (the default one when it names none), directory and command, `--attach` a tab attached to that session. A launch with neither SHALL open no further tab, or one default-profile tab when nothing was restored. The debug options (`--send`, `--action`, `--dump-screen-after`) act on the active tab after this.

#### Scenario: Profile launch
- **WHEN** Brindle quit with two tabs, and the user runs `brindle terminal Work`
- **THEN** the window shows the two restored tabs and an active tab with the `Work` profile

#### Scenario: Terminal without a profile
- **WHEN** Brindle quit with two tabs, and the user runs `brindle terminal`
- **THEN** the window shows the two restored tabs and an active tab with the default profile

#### Scenario: Plain launch
- **WHEN** Brindle quit with two tabs, and the user runs `brindle`
- **THEN** the window shows the two restored tabs and no other tab

#### Scenario: Skip once
- **WHEN** the user runs `brindle --no-restore`
- **THEN** the window opens with a single default-profile tab

### Requirement: A launch goes to the running Brindle
A launch without debug options while an instance runs (one with the same state directory) SHALL send its request to the instance and exit with status 0 once the instance has accepted it. The instance SHALL raise its window and open what the launch asked for, as "A launch restores, then opens what it asks for" says, without restoring again; a `brindle terminal -e` command SHALL start in the launching process's working directory. When the instance doesn't accept the request within 5 seconds, the launch SHALL print why and exit with status 1. The request channel SHALL be reachable only by the same user. Before sending anything, the launch SHALL check that the instance runs as the same user and, on Windows, at the same integrity level, and SHALL give it no right to act as the launching user; otherwise it SHALL send nothing, print why and exit with status 1. On Windows, a launch with no console that gives up (refused, or not accepted within 5 seconds) SHALL also show why in a message box. On Windows an elevated and an unelevated Brindle have different default state directories, so each is its own instance: a launch at either level, while a Brindle at the other runs, opens its own window, and the integrity check is reached only when `BRINDLE_STATE_DIR` names one directory for both.

#### Scenario: Second plain launch
- **WHEN** Brindle runs with two tabs and the user launches `brindle` from the desktop
- **THEN** the window is raised, it still has two tabs, and no second window opens

#### Scenario: New tab from a shell
- **WHEN** Brindle runs with two tabs and the user runs `brindle terminal` in a shell
- **THEN** the `brindle` command exits, and Brindle's window gains a third, active tab with the default profile

#### Scenario: Command from a shell
- **WHEN** Brindle runs and the user runs `brindle terminal -e htop` in a shell in `/srv`
- **THEN** the `brindle` command exits, and Brindle's window gains an active tab running htop in `/srv`

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
When the environment variable `BRINDLE_NOTIFY_LOG` names a file, Brindle SHALL append each notification it would show to that file instead of sending it to the desktop, one line per notification: the source (`bell`, `osc9`, `osc777` or `osc7501`), the title and the body, separated by tabs. Lines SHALL be appended in the order the notifications were sent. Writing them SHALL NOT hold up the window, and every line SHALL be in the file by the time Brindle has exited, even if it quits right after the notification, for example from `--dump-screen-after`.

#### Scenario: Logged notification
- **WHEN** Brindle runs with `BRINDLE_NOTIFY_LOG=/tmp/n.log`, `notifications.when = "always"`, and `--send 'printf "\033]9;hi\a"\r'`
- **THEN** `/tmp/n.log` contains a line `osc9<TAB><tab title><TAB>hi` and no desktop notification appears

#### Scenario: Quit right after notifications
- **WHEN** Brindle runs as `brindle terminal -e sh -c 'printf "\033]9;one\a\033]777;notify;t;two\a"'` with `BRINDLE_NOTIFY_LOG=/tmp/n.log` and `notifications.when = "always"`, so its only tab's program sends two notifications and exits, which quits Brindle
- **THEN** once the process has exited, `/tmp/n.log` holds the `osc9` line for `one` followed by the `osc777` line for `two`

## ADDED Requirements

### Requirement: The flat launch options are removed
`-p/--profile`, and `-d/--working-directory` and `-e/--command` given without a subcommand, SHALL be refused as usage errors (status 2, before any window opens) that name the option and say what to type instead: `brindle terminal PROFILE`, `brindle terminal -d DIR` or `brindle terminal -e COMMAND`. `-p` and `--profile` after `terminal` SHALL be refused the same way, saying to name the profile directly.

#### Scenario: Old profile option
- **WHEN** the user runs `brindle -p work`
- **THEN** Brindle prints that `-p` was removed and to run `brindle terminal work`, and exits with status 2 without opening a window

#### Scenario: Old command option
- **WHEN** the user runs `brindle -e htop`
- **THEN** Brindle prints that `-e` now belongs to `brindle terminal` and to run `brindle terminal -e htop`, and exits with status 2 without opening a window

#### Scenario: Profile option under the subcommand
- **WHEN** the user runs `brindle terminal -p work`
- **THEN** Brindle prints that the profile is named directly, as `brindle terminal work`, and exits with status 2 without opening a window

#### Scenario: Old options while Brindle runs
- **WHEN** Brindle runs and the user runs `brindle -d /srv` in a shell
- **THEN** the launch prints the usage error and exits with status 2, and the running window gains no tab

### Requirement: Usage errors without a console on Windows
On Windows, a launch refused with a usage error that has no console to print to, such as one from a shortcut or the Run dialog, SHALL also show the error in a message box before it exits with status 2.

#### Scenario: Old option in a shortcut
- **WHEN** a Windows shortcut runs `brindle.exe -p work`
- **THEN** a message box says `-p` was removed and to use `brindle terminal work`, and no window opens

### Requirement: Options around the subcommand
The first argument that is neither an option nor an option's value SHALL be the subcommand, and any other such word in its place SHALL be refused as a usage error that names it and points at `brindle terminal`. `--config`, `--no-restore`, the debug, informational and desktop install options SHALL work before or after the subcommand, and with none.

#### Scenario: Debug options after the subcommand
- **WHEN** the user runs `brindle terminal Shell --no-restore --send 'echo hi\r' --dump-screen-after 3`
- **THEN** the dump shows a `Shell` tab with `hi` on its screen

#### Scenario: Options before the subcommand
- **WHEN** the user runs `brindle --no-restore --send 'echo hi\r' --dump-screen-after 3 terminal Shell`
- **THEN** the dump shows the same as with the options after the subcommand

#### Scenario: Debug options inside a command
- **WHEN** the user runs `brindle terminal -e printf '%s\n' --send`
- **THEN** the tab's program prints `--send`, and no step runs

#### Scenario: A profile name without the subcommand
- **WHEN** the user runs `brindle work`
- **THEN** Brindle prints usage naming `work` and suggesting `brindle terminal work`, and exits with status 2 without opening a window

#### Scenario: Informational option after the subcommand
- **WHEN** the user runs `brindle terminal --version`
- **THEN** the version is printed and the process exits 0 without opening a window

### Requirement: The agent subcommand is reserved
`brindle agent`, with or without further arguments, SHALL be refused as a usage error (status 2, before any window opens) saying that this Brindle has no agent tabs, and SHALL NOT be read as a profile name or as any other request.

#### Scenario: Agent launch before agent tabs exist
- **WHEN** the config has a profile named `agent` and the user runs `brindle agent`
- **THEN** Brindle prints that this Brindle has no agent tabs and exits with status 2, without opening a window or a tab of the `agent` profile

### Requirement: A launch's profile has its subcommand's kind
A profile named under a subcommand SHALL be of that subcommand's kind; otherwise the launch SHALL be refused as a usage error naming the profile, its kind and the subcommand that opens it, with status 2 and nothing opened, checked as an unknown name is (see "Launch options"). Every profile is a terminal profile.

#### Scenario: Terminal profile under the terminal subcommand
- **WHEN** the config has profiles `Shell` and `Work` and the user runs `brindle terminal Work`
- **THEN** an active terminal tab of the `Work` profile opens

#### Scenario: Unknown name under the terminal subcommand
- **WHEN** no profile is named `wrk` and the user runs `brindle terminal wrk`
- **THEN** the launch is refused for the unknown name, with status 2, and no tab opens

### Requirement: The hand-off across versions
A running Brindle SHALL take launches from older and newer versions: a request naming a profile, directory or command, with no kind or the terminal kind, SHALL open a terminal tab, and a request naming a kind of tab it can't open SHALL be refused with nothing opened, the launch printing why and exiting with status 1. A refusal for an unknown or wrong-kind profile SHALL make a launch of this version exit with status 2. A plain launch's request SHALL stay as older versions read it.

#### Scenario: Older launch, newer Brindle
- **WHEN** a Brindle of this version runs and an older `brindle -p Work` launch hands it its request
- **THEN** the window gains an active tab with the `Work` profile

#### Scenario: Newer launch, older Brindle
- **WHEN** a Brindle from before this change runs and the user runs `brindle terminal Work -e htop` with this version
- **THEN** the older window gains an active `Work` tab running htop

#### Scenario: A kind the running Brindle can't open
- **WHEN** a Brindle of this version runs and a launch hands it a request for a kind of tab it doesn't know
- **THEN** it opens nothing and answers why, and the launch prints that answer and exits with status 1

#### Scenario: Unknown name, older Brindle
- **WHEN** a Brindle from before this change runs and the user runs `brindle terminal wrk` with this version, and no profile is named `wrk`
- **THEN** the older window opens its default profile with its dim line naming `wrk`, as it always has, and the launch exits with status 0

### Requirement: The hand-off request's size
A running Brindle SHALL read a launch's request up to 1 MiB, its line ending included. A launch whose request would be larger SHALL send nothing, print that the launch is too large to hand to the running Brindle, naming the limit, and exit with status 1. A running Brindle that reads 1 MiB without the line's end SHALL open nothing and answer that the request is over the limit.

#### Scenario: Oversized launch
- **WHEN** Brindle runs and a launch's command and arguments encode to more than 1 MiB
- **THEN** the launch sends nothing, prints that it is too large for the running Brindle's 1 MiB limit, and exits with status 1, and the window gains no tab

#### Scenario: Oversized request from another program
- **WHEN** a process of the same user writes 2 MiB without a line end to the running Brindle's launch socket
- **THEN** Brindle opens nothing and answers that the request is over its 1 MiB limit
