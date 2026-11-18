# remote-connections Specification

## Purpose
Run profiles on a remote Linux host over SSH or inside an existing Docker or Podman container, with the same tabs, zones, titles, directories, and notifications as a local profile.

## Requirements

### Requirement: Remote target profile configuration
A profile SHALL be able to define an optional `remote` target specifying either an SSH host or a container.
For SSH targets, the configuration SHALL support:
- `host` (hostname or SSH config alias)
- optional `user`
- optional `port`
- optional `identity_file`
- optional `ssh_args`

For container targets, the configuration SHALL support:
- `engine` (`docker` or `podman`, defaulting to `docker`)
- `container` (container name or ID)
- optional `user`

Both kinds SHALL support an optional `connect_at_launch` (default `false`), defined by "Connecting at launch".

#### Scenario: SSH remote profile
- **WHEN** a profile is configured with `remote = { type = "ssh", host = "A8" }`
- **THEN** launching tabs with this profile connects to the remote host `A8` over SSH

#### Scenario: Container remote profile
- **WHEN** a profile is configured with `remote = { type = "container", engine = "docker", container = "my-box" }`
- **THEN** launching tabs with this profile executes inside the `my-box` container via `docker exec`

#### Scenario: Eager SSH profile
- **WHEN** a profile is configured with `remote = { type = "ssh", host = "A8", connect_at_launch = true }`
- **THEN** the config loads, and Brindle connects to `A8` at launch

### Requirement: Remote PTY lifecycle and I/O parity
A remote terminal session SHALL provide bidirectional PTY stream forwarding, resizing, and child exit notification with parity to local PTY sessions.
- Input typed into the terminal SHALL be transmitted to the remote PTY.
- Output from the remote PTY SHALL be streamed to the local terminal grid.
- Grid resizing SHALL be transmitted to the remote PTY via window size updates (`TIOCSWINSZ`).
- Termination of the remote shell or process SHALL trigger terminal exit locally.

#### Scenario: Interactive shell input and output
- **WHEN** the user types `echo hello` in a remote tab
- **THEN** the remote PTY receives the input and the terminal grid displays `hello`

#### Scenario: Window resizing
- **WHEN** the user resizes a remote terminal window to 120 columns and 40 rows
- **THEN** the remote PTY updates its terminal dimensions to 120x40

### Requirement: Process tracking and automatic titles
Remote Linux sessions SHALL report the remote foreground process name from `/proc` to Brindle's title refresh loop, matching local title behavior. The host SHALL report the name when it changes. Brindle SHALL NOT ask each remote tab's host for it every refresh.

#### Scenario: Remote command execution title
- **WHEN** the user executes `htop` in a remote tab
- **THEN** the tab title updates to display `htop`

#### Scenario: Idle tabs cost nothing
- **WHEN** ten remote tabs on one host sit idle, with no output and no input
- **THEN** Brindle sends that host nothing but the connection's heartbeat

### Requirement: Current working directory tracking
Remote sessions SHALL track the remote working directory through `/proc/<pid>/cwd`, reported by the host when it changes, and through OSC 7 reports. When splitting a zone or launching a new tab from an active remote tab, the new session SHALL inherit the remote directory.

#### Scenario: Remote zone split inherits remote CWD
- **WHEN** the user is in `/var/log` on a remote host and triggers `Split Zone Right`
- **THEN** the newly opened split zone launches in `/var/log` on the same remote host

### Requirement: Passthrough of notifications and program status
Remote sessions SHALL pass through OSC 9, OSC 777, and OSC 7501 sequences intact in the output stream so that desktop notifications, bell indicators, and program status dots function identically to local tabs.

#### Scenario: Remote desktop notification
- **WHEN** a remote command prints `ESC ] 9 ; build complete BEL`
- **THEN** Brindle triggers a desktop notification for that tab

### Requirement: Remote tabs run in the host's terminal server
A remote tab's program SHALL run in a session of the terminal server on its host or in its container, for the user the profile connects as, as defined by the `terminal-server` capability. Brindle SHALL reach that server through `brindle-remote relay` started over SSH or the container engine's `exec`, and SHALL start the server there when none runs.

#### Scenario: Server on the host
- **WHEN** no Brindle server runs for user `ubuntu` on host `A8`, and the user opens an SSH tab for `ubuntu@A8`
- **THEN** a core and a holder run on `A8` as `ubuntu`, the tab's shell is the holder's child, and `brindle-remote list` run on `A8` lists the tab's session

### Requirement: Remote programs get the endpoint's environment
A remote session's program SHALL get the environment of a fresh SSH or exec login on its endpoint, plus the profile's variables, and the user's login shell when the profile sets no command. A working directory that doesn't exist on the endpoint SHALL be replaced by the user's home directory there.

#### Scenario: Missing directory
- **WHEN** a restored remote zone recorded `/srv/old`, which no longer exists on its host
- **THEN** its new shell starts in the user's home directory on that host

#### Scenario: Host environment
- **WHEN** `ubuntu`'s environment over SSH on `A8` has `LANG=en_GB.UTF-8`, the profile sets `env = { FOO = "1" }`, and the user runs `env` in a new tab
- **THEN** the output has `LANG=en_GB.UTF-8` and `FOO=1`, and no variable that exists only in Brindle's own environment

### Requirement: One connection per host
Brindle SHALL keep at most one connection to each remote endpoint, shared by all of that endpoint's tabs and zones. An endpoint is identified by its SSH target (host, user, port, identity file and SSH arguments) or its container target (engine, container and user). Opening a tab or splitting a zone on an endpoint that is already connected SHALL NOT start another SSH or exec connection. A connection SHALL end once no terminal uses it, unless a profile of that endpoint connects at launch. A terminal whose program is running, closed while its endpoint can't be reached, SHALL keep using the connection until its program has been told to end, for at most 5 minutes after the close. A terminal closed before its session started or was attached SHALL NOT keep using it. A launch waiting for an endpoint's first listing (see "Launch reattaches detached terminals" in `session-restore`) SHALL NOT keep a connection that no terminal uses.

#### Scenario: Splits share the connection
- **WHEN** an SSH tab on `A8` is split twice
- **THEN** Brindle runs one `ssh` process for `A8`, and the three zones' sessions are listed on `A8`'s server

#### Scenario: Last tab closed
- **WHEN** the only tab on host `A8` is closed, and no profile for `A8` connects at launch
- **THEN** Brindle's `ssh` process for `A8` exits within a few seconds

#### Scenario: Closed while the host is unreachable
- **WHEN** the only tab on host `A8`, whose program is running, is closed while `A8` can't be reached, no profile for `A8` connects at launch, and `A8` can be reached again two minutes later
- **THEN** the tab's program on `A8` ends, and Brindle's `ssh` process for `A8` exits within a few seconds after that

#### Scenario: Host gone for good
- **WHEN** the only tab on host `A8`, whose program is running, is closed while `A8` can't be reached, no profile for `A8` connects at launch, and `A8` never comes back
- **THEN** Brindle stops trying to connect to `A8` about 5 minutes after the close, its log names the session it couldn't end, and "Attach Session…" no longer lists `A8`

#### Scenario: Restored tab closed before its host answers
- **WHEN** a restored tab on `A8` is connecting because `A8` can't be reached, no profile for `A8` connects at launch, and the user closes that tab
- **THEN** Brindle stops trying to connect to `A8` within a few seconds, and the session the tab was attaching stays detached on `A8`

### Requirement: The helper is deployed once per build
Brindle SHALL copy its Linux `brindle-remote` to an endpoint only when the endpoint doesn't already have that exact build, cached by version and digest (on an SSH host under the user's `~/.cache/brindle/`). Connecting to an endpoint whose cache is current SHALL take one SSH or exec connection: the relay's own.

#### Scenario: Warm SSH connect
- **WHEN** host `A8` already has this build's helper cached, and the user opens the first `A8` tab of a launch
- **THEN** Brindle starts exactly one `ssh` process for it

### Requirement: The container helper is shared and protected
In a container, the helper SHALL be cached in a root-owned directory that other container users can't write, and reused by every connection to that container for the same build. Brindle SHALL refuse to run a cached helper whose file or directory isn't root-owned, or is a symbolic link.

#### Scenario: Container helper kept
- **WHEN** the user opens a tab in container `devbox`, closes it, and opens another
- **THEN** the helper is copied into `devbox` once, and the second tab's relay runs the same copy

#### Scenario: Directory taken in a container
- **WHEN** another container user has already created the directory the helper would be cached in
- **THEN** the tab shows an error naming that directory, and Brindle runs nothing from it

### Requirement: Remote terminals persist
Remote terminals SHALL follow the same lifetime rules as local terminals:
- quitting Brindle, or Brindle ending without quitting, leaves them running detached on their host, except `-e` command terminals;
- closing a remote zone or tab ends its program. When its host can't be reached, the program ends once the host can be reached again within 5 minutes of the close. After that it is left as if Brindle had ended without quitting (see "One connection per host"). A zone or tab closed before its session was attached in this run, such as a restored one whose host can't be reached, leaves that session detached;
- detached remote sessions can be attached again with their screens.

#### Scenario: Quit with a remote tab
- **WHEN** an SSH tab on `A8` runs `htop` and the user quits Brindle
- **THEN** `htop` keeps running on `A8`, and `brindle-remote list` there shows its session as detached

#### Scenario: Close a remote zone
- **WHEN** a container zone runs `sleep 1000` and the user closes the zone
- **THEN** the `sleep` process in the container is gone within a few seconds

#### Scenario: Closed while the host is away too long
- **WHEN** an SSH tab on `A8` runs `sleep 1000`, the user closes it while `A8` can't be reached, and `A8` can be reached again only 10 minutes later
- **THEN** the `sleep` is still running on `A8`, and `brindle-remote list` there shows its session as detached

### Requirement: Lost connections reconnect
Brindle SHALL notice a remote connection that has gone silent, even when the transport never closes, within about half a minute. It SHALL then show that the connection was lost in each of that endpoint's terminals, and reconnect with growing pauses (at most 30 s apart) for as long as any terminal uses the endpoint. Where Brindle reports the loss (each terminal's line, its log, and the endpoint's row in the sessions palette), it SHALL say what ended the link:
- the SSH or container engine process exiting, with its exit status and what it wrote, or that its exit status couldn't be read, and why;
- nothing arriving within Brindle's own limit;
- or the far end closing the link.

#### Scenario: Network drop
- **WHEN** the network to `A8` is cut for two minutes while a tab there runs `htop`, and then restored
- **THEN** the tab says the connection was lost and why, and within 30 s of the restore it shows `htop` again, with the same pid

#### Scenario: ssh gives up
- **WHEN** the network to `A8` is cut, and `ssh` exits because its keepalives go unanswered
- **THEN** Brindle's log and the tab's line say the connection to `A8` ended because `ssh` exited, with its exit status and anything it wrote, and neither says that the terminal server closed the connection

#### Scenario: Silent link
- **WHEN** the link to `A8` delivers nothing for Brindle's limit, while `ssh` keeps running
- **THEN** Brindle's log and the tab's line say that nothing was read from `A8` in that time, and the `ssh` process is ended

### Requirement: Sessions come back after a reconnect
On reconnecting, every session Brindle still owns on that endpoint SHALL come back with its replay, then live output. Input typed while disconnected SHALL be delivered after the reconnect, unless refused under the input bound. A session SHALL be shown as lost only when the reconnected endpoint no longer has it. The endpoint SHALL count as having it only when it lists the session's id with the session's key. An attach cut off by the drop SHALL NOT be answered with a different session.

#### Scenario: Typing while disconnected
- **WHEN** the user types `ls` and enter in a remote tab while its connection is down
- **THEN** after the reconnect the shell runs `ls` once

#### Scenario: Session ended while disconnected
- **WHEN** a remote session's program is ended with `brindle-remote kill` on its host while Brindle is disconnected from that host
- **THEN** after the reconnect the tab says the session was lost, and stays open

#### Scenario: Id given to another session while disconnected
- **WHEN** `A8` reboots while Brindle is disconnected from it, and before Brindle reconnects another Brindle opens a tab on `A8` whose session gets the same id as a tab's session here
- **THEN** after the reconnect the tab here says the session was lost and stays open, and the other Brindle's tab keeps running, unaffected

#### Scenario: Attach cut off, id given to another session
- **WHEN** the user attaches a detached session on `A8` from "Attach Session…", the connection drops before its screen arrives, and when Brindle reconnects `A8` lists that id for a different session with no client
- **THEN** the new tab says `A8` no longer has the session, and shows nothing of the other session, which stays detached

### Requirement: The host notices a vanished client
A relay that hears nothing from Brindle for a bounded idle limit (60 s) SHALL end, so the host's server treats the client as gone. Brindle SHALL send a heartbeat while the connection is otherwise idle. A client that is merely slow to read output SHALL NOT be treated as vanished.

#### Scenario: Network cut without a clean close
- **WHEN** the connection to `A8` is cut so that `A8` never sees it close
- **THEN** the relay on `A8` exits within the idle limit plus a few seconds, and the client's `detach` sessions are listed as detached

#### Scenario: Idle connection stays
- **WHEN** a remote tab sits idle for longer than the idle limit with Brindle running
- **THEN** the session is still attached and accepts input

#### Scenario: Slow reader during heavy output
- **WHEN** Brindle stops reading a remote connection for longer than the idle limit while a remote program floods output
- **THEN** the relay doesn't end, and output resumes when Brindle reads again

### Requirement: Command sessions survive short drops
A remote `-e` command terminal's session SHALL NOT end when its connection drops, if Brindle reconnects within 5 minutes of the host noticing the drop. It SHALL end if Brindle doesn't reconnect in that time.

#### Scenario: Short drop
- **WHEN** a tab opened with `brindle -p A8 -e htop` loses its connection for one minute
- **THEN** after the reconnect the tab shows the same `htop` process

#### Scenario: Brindle crashed
- **WHEN** Brindle is killed with SIGKILL while that tab runs `htop`
- **THEN** `htop` on `A8` ends within about 6 minutes, and isn't listed after that

### Requirement: Connecting at launch
At launch, Brindle SHALL connect in the background to the endpoint of every profile with `connect_at_launch = true`, and keep those connections for as long as it runs. Connecting SHALL NOT delay the window or block input to other tabs. An endpoint that can't be reached, or whose terminal server refuses this Brindle, SHALL count as offline and be retried with growing pauses, at most 60 s apart. Brindle SHALL NOT start or create a container to connect to it.

#### Scenario: Host reachable
- **WHEN** profile `A8` connects at launch and `A8` has a detached session running `htop`
- **THEN** shortly after the window opens, a tab shows that `htop`

#### Scenario: Host unreachable
- **WHEN** profile `A8` connects at launch and `A8` can't be reached
- **THEN** the window opens as quickly as without the profile, and "Attach Session…" shows `A8` as offline

#### Scenario: Container stopped
- **WHEN** a container profile connects at launch and its container is stopped
- **THEN** the container stays stopped, and "Attach Session…" shows it as offline

#### Scenario: Server refuses
- **WHEN** profile `A8` connects at launch, and `A8`'s terminal server refuses this Brindle until that server exits
- **THEN** "Attach Session…" shows `A8` as offline with the refusal meanwhile, and within a minute of the refusing server exiting Brindle is connected to `A8`, without a tab being opened there

### Requirement: A tab stays on its host
A tab's zones SHALL keep running on the endpoint the tab was opened on when its profile's `remote` target changes. Splitting one of its zones SHALL start the new session on that same endpoint. New tabs of the profile SHALL use the new target.

#### Scenario: Profile moved to another host
- **WHEN** a tab of profile `Remote` runs on `old-host`, the profile's host is changed to `new-host`, and the user splits the tab's zone
- **THEN** the new zone runs on `old-host`, and a new tab of `Remote` runs on `new-host`

### Requirement: Input waiting to reach a session is bounded
Input waits in Brindle while its session starts and while its connection is down or slow. Input typed into a tab while 4 MiB or more of that tab's earlier input is waiting SHALL NOT be sent, then or later, until less than 1 MiB is waiting. The first input refused after input was last taken SHALL make the tab say that input was not sent and why. Input taken before the bound was reached SHALL still be delivered, in order.

#### Scenario: Input past the bound while disconnected
- **WHEN** a remote tab's connection is down, more than 4 MiB of input pasted into it is waiting to be sent, and the user then types `pwd` and enter
- **THEN** the tab says, once, that input was not sent, and after the reconnect the pasted input reaches the shell and `pwd` never does

#### Scenario: Input taken again once the backlog drains
- **WHEN** input was refused in a tab because 4 MiB was waiting, the connection comes back, and less than 1 MiB of that tab's input is still waiting when the user types `pwd` and enter
- **THEN** the shell runs `pwd` once

#### Scenario: A large paste while connected
- **WHEN** a tab's connection is up, less than 4 MiB of its input is waiting, and the user pastes 6 MiB
- **THEN** the whole paste reaches the program, and the tab says nothing about refused input
