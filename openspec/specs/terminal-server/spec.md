# terminal-server Specification

## Purpose
A per-user terminal server, `brindle-remote` in server mode, owns terminal sessions on its host, so a session can outlive the client that started it, survive server upgrades and crashes, and be attached again with its screen intact.

## Requirements

### Requirement: A core and its holders
The server SHALL run as two kinds of process from the same binary. Holders own the sessions' PTYs: they are the programs' parent, carry their I/O, and parse nothing. The core runs the emulators, the client protocol and the management commands, and drives the holders. Clients SHALL talk only to the core.

#### Scenario: Programs belong to a holder
- **WHEN** a client starts a session
- **THEN** the program's parent is a holder process, not the core and not the client

#### Scenario: Holders serve many sessions
- **WHEN** a client starts three sessions
- **THEN** all three run under the same holder

### Requirement: One core per user per host
At most one core SHALL run per user per server directory. The directory is `$BRINDLE_SERVER_DIR` when set. Otherwise it is `$XDG_RUNTIME_DIR/brindle`, else `~/.cache/brindle/run` on Linux, and `%LOCALAPPDATA%\brindle\run` on Windows (`run-elevated` for an elevated process). It is created user-only: mode 0700 on Linux, a protected DACL for the user and SYSTEM on Windows. A core that finds another live core for its directory SHALL exit without disturbing it.

#### Scenario: Two launches race
- **WHEN** two Brindle processes start at the same moment with no server running
- **THEN** exactly one core runs afterwards, and both Brindles' tabs are served by it

#### Scenario: Isolated directory
- **WHEN** a server runs with `BRINDLE_SERVER_DIR=/tmp/a` and another client starts one with `BRINDLE_SERVER_DIR=/tmp/b`
- **THEN** two independent servers run, and neither lists the other's sessions

#### Scenario: Isolated directory on Windows
- **WHEN** a server runs with `BRINDLE_SERVER_DIR=C:\Temp\a` and another client starts one with `BRINDLE_SERVER_DIR=C:\Temp\b`
- **THEN** two independent servers run, and neither lists the other's sessions

### Requirement: Only the owning user can connect
The core's and the holders' endpoints (Unix sockets on Linux, named pipes on Windows) SHALL be reachable only by the user who runs them, and on Windows only from the same machine. Each SHALL check every connecting peer's user before reading a request, and close the connection when it differs from its own. On Windows, where another user could create a pipe name first, a client SHALL likewise refuse an endpoint whose server runs as another user, without sending it anything.

#### Scenario: Another user connects
- **WHEN** a process running as a different user connects to the core's or a holder's endpoint
- **THEN** the connection is closed and no session is listed, created or touched

#### Scenario: A pipe name taken by another user
- **WHEN** on Windows another user's process has already created the core's pipe name, and Brindle opens a local tab
- **THEN** Brindle sends that process nothing, and the tab shows an error saying the terminal server could not be reached

### Requirement: Server starts on demand and exits when unused
A client SHALL start the core when none is running, detached from the client, so the server survives the client: on Linux outside the client's process group and session, and on Windows with no console window and outside the client's console. The core starts holders as it needs them. A holder SHALL exit once it has no sessions, and the core SHALL exit once it has had neither sessions nor connected clients for a short grace period.

#### Scenario: First local tab starts the server
- **WHEN** no server is running and Brindle opens a local Linux tab
- **THEN** a core and a holder are running, and neither's parent is the Brindle process

#### Scenario: First local tab starts the server on Windows
- **WHEN** no server is running and Brindle opens a local Windows tab
- **THEN** a core and a holder are running with no console window, the tab's program is a child of the holder, and both keep running after Brindle is ended with `taskkill /F` while a `detach` session runs

#### Scenario: Unused server exits
- **WHEN** the last client disconnects and no sessions are left
- **THEN** the holders and the core exit within a few seconds and remove their sockets or pipes

### Requirement: Sessions
A client SHALL be able to start a session with a command, arguments, working directory, environment, grid size and scrollback limit. The server SHALL run the program on a new PTY with exactly that environment, not its own. It SHALL report the session's id and the program's pid, or the reason the program could not be started.

#### Scenario: Session uses the client's environment
- **WHEN** the server was started by a client whose environment had `FOO=1`, and a later client with `FOO=2` starts a session
- **THEN** the program in that session sees `FOO=2`

#### Scenario: Program cannot start
- **WHEN** a client starts a session whose command does not exist
- **THEN** the server replies with an error naming the program and the OS error, and no session is listed

### Requirement: Session I/O
For an attached session, the server SHALL carry the client's input and resizes to the PTY, stream its output to the client in order, and report the program's exit with its exit status. One session's slow or stalled client SHALL NOT delay other sessions.

#### Scenario: Exit status
- **WHEN** the program in an attached session runs `exit 3`
- **THEN** the client receives the session's exit with status 3, and the session is no longer listed

#### Scenario: A stalled client
- **WHEN** one client stops reading while its program floods output
- **THEN** sessions of other clients keep streaming output without delay

### Requirement: Close policies
Each session SHALL have a close policy, chosen when it is started. With `kill`, the session SHALL end, hanging up its program and the processes under it, when its client closes it, or when its client is gone: its connection ended and it didn't reconnect within the reconnect grace period after a core restart. With `detach`, the session SHALL keep running unattached when its client is gone. A client that explicitly detaches a session SHALL leave it running unattached whatever its policy, and a `kill` session detached that way SHALL become a `detach` session.

#### Scenario: Client crashes with kill
- **WHEN** a client that started a `kill` session is killed with SIGKILL
- **THEN** the session's program and the processes under it are gone within a few seconds

#### Scenario: Client leaves with detach
- **WHEN** a client that started a `detach` session disconnects
- **THEN** the session is listed as detached and its program keeps running

#### Scenario: Explicit detach of a kill session
- **WHEN** a client detaches a `kill` session and then disconnects
- **THEN** the session is listed as detached with policy `detach`, and its program keeps running

### Requirement: Clients reconnect after a core restart
When the core restarts, its clients' connections end. A client that reconnects within the reconnect grace period and identifies itself SHALL get back every session it was attached to when its connection ended, without the close policy applying, and SHALL receive each one's replay before live output resumes. Sessions the client had explicitly detached, or that another client attached since, SHALL NOT be given back to it. This SHALL hold across core restarts and upgrades: the server keeps each session's owner and policy as they last changed. An attach's change of owner and policy SHALL be kept from the moment the attaching client receives the start of its replay: a core restart after that, however soon, SHALL give the session back to that client and not to its previous owner.

#### Scenario: Reconnect keeps kill sessions
- **WHEN** the core restarts while a client has a `kill` session running `htop`, and the client reconnects within the grace period
- **THEN** the session is still running with the same pid, and the client receives its replay and then live output

#### Scenario: No reconnect ends kill sessions
- **WHEN** the core restarts and a client with `kill` sessions doesn't reconnect within the grace period
- **THEN** those sessions end

#### Scenario: Detached sessions stay detached
- **WHEN** a client detaches a session, then the core restarts and the client reconnects
- **THEN** the session is still listed as detached, and the client receives no replay of it

#### Scenario: Attached by another client
- **WHEN** client B attaches a session that client A started, and the core restarts
- **THEN** after both reconnect, B gets the session back and A doesn't

#### Scenario: Core killed right after an attach
- **WHEN** client B attaches a session that client A started, and the core is killed with SIGKILL as soon as B receives the start of its replay
- **THEN** after both reconnect to the next core, B gets the session back and A doesn't

### Requirement: Attach replays the screen
A client SHALL be able to attach to a detached session. Before any live output, the server SHALL send a replay that recreates the session's screen: a reset, the terminal modes, the scrollback within the session's limit, the visible grid with its colors and attributes, the cursor position and shape, the title, and the working directory the program last reported, read as the paths of the platform the program runs on. The replay SHALL be delimited, so the client can tell it from live output.

#### Scenario: Full-screen program
- **WHEN** a client attaches to a detached session that is running `htop`
- **THEN** after the replay the client's screen matches the session's screen, in the alternate screen with mouse reporting as `htop` set it

#### Scenario: Scrollback
- **WHEN** a detached session has printed 500 numbered lines with a scrollback limit of 1000, and a client attaches
- **THEN** the client's scrollback holds all 500 lines in order

#### Scenario: Attached sessions can't be taken
- **WHEN** a client asks to attach to a session another client is attached to
- **THEN** the server refuses with an error saying the session is attached

#### Scenario: Directory reported with OSC 9;9 on Windows
- **WHEN** on Windows a detached session's program reported `ESC ] 9 ; 9 ; C:\src ST`, and a client attaches
- **THEN** the replay reports `C:\src` as the session's working directory

#### Scenario: Directory after a core restart on Windows
- **WHEN** on Windows a session's program reported `ESC ] 9 ; 9 ; C:\src ST`, the core is killed and a new core takes over, and a client reconnects
- **THEN** the replay reports `C:\src` as the session's working directory

### Requirement: Program status and prompt marks survive attach
The server SHALL keep each session's program status records (OSC 7501) and shell-integration prompt marks (OSC 133), and send them to an attaching or reconnecting client after the replay, so the client's status dots and prompt navigation match the session.

#### Scenario: Status after attach
- **WHEN** a program in a detached session reported `working`, and a client attaches
- **THEN** the client shows the session's status as working

#### Scenario: Prompt marks after attach
- **WHEN** a detached session's shell emitted prompt marks for three commands, and a client attaches
- **THEN** the client can jump between those three prompts

### Requirement: The server never answers terminal queries
The server's per-session emulator SHALL NOT write anything to the PTY. Device attribute, cursor position and color queries SHALL be answered only by the attached client.

#### Scenario: Cursor position query
- **WHEN** a program in an attached session sends `ESC [ 6 n`
- **THEN** it receives exactly one reply, from the client

### Requirement: Protocol versions
A client and the core SHALL agree on a protocol version when they connect, and so SHALL the core and each holder. A core SHALL speak every client protocol version and every holder protocol version from the first terminal-server release's up to its own. Client protocol 2 adds process info. Client protocol 3 adds layout updates for attached sessions. A client SHALL refuse a core that speaks none of its versions, with an error naming both versions. A holder speaking an older holder protocol that can't store a session's changed owner or policy SHALL keep working; only those changes are lost if the core restarts before the holder drains.

#### Scenario: Older client, newer core
- **WHEN** a Brindle from an older release connects to a core from a newer release
- **THEN** its tabs open and work through that core

#### Scenario: Newer core, older holder
- **WHEN** a newer core adopts a holder started by an older release
- **THEN** that holder's sessions keep working: input, output, resize, exit and kill

#### Scenario: Newer client, older core
- **WHEN** a client that speaks protocols 1 and 2 connects to a core that speaks only protocol 1
- **THEN** they agree on protocol 1, the client's sessions work, and it receives no process info

#### Scenario: Layout updates need protocol 3
- **WHEN** a client that speaks protocols 1 to 3 connects to a core that speaks protocols 1 and 2
- **THEN** they agree on protocol 2, the client sends no layout updates, and its sessions still get their layouts when it detaches them

### Requirement: Upgrades keep every session
When a client whose server binary is newer than the running core connects, the core SHALL save each session's screen to its holder and exit, and the newer core SHALL take over. Every session SHALL keep running under its holder with the same program and pid. The new core SHALL rebuild each session's screen, scrollback, status and marks from what the holder kept.

#### Scenario: Upgrade with a running Brindle
- **WHEN** a Brindle of version N has local tabs open, and a Brindle of version N+1 starts
- **THEN** the core runs version N+1, and the version N Brindle's tabs reconnect and keep working with the same programs, pids and screens

#### Scenario: Detached session survives an upgrade
- **WHEN** a detached session is running `htop` and the core is upgraded
- **THEN** attaching afterwards replays `htop`'s screen, and `htop` still has the same pid

### Requirement: Older holders drain
A core SHALL start new sessions only on a holder of its own build. A holder of another build SHALL be told to drain: it takes no new sessions, keeps its sessions working, and exits when the last one ends.

#### Scenario: Drain after an upgrade
- **WHEN** the core is upgraded while an old-build holder has two sessions, and a client starts a third
- **THEN** the third runs under a new holder of the new build, the first two keep running under the old holder, and the old holder exits once both have ended

### Requirement: A failed upgrade keeps the sessions
Before the running core steps down, the client SHALL check that the new binary speaks the protocol versions of the running holders. If the check fails, the core SHALL keep running unchanged, and the client SHALL use it when they share a protocol version. If the new core fails to start after the old one stepped down, the holders SHALL start a core of their own build again, so the sessions are kept.

#### Scenario: Incompatible binary
- **WHEN** the offered binary doesn't speak a running holder's protocol version
- **THEN** the core keeps running with all its sessions, and its log says why the upgrade was refused

#### Scenario: New core won't start
- **WHEN** the old core has stepped down and the new binary exits at startup
- **THEN** within a few seconds a core of the holders' build is running, and every session is still listed

### Requirement: Failures stay small
A core crash SHALL NOT end any session: a holder whose core is gone SHALL start a core again after a short grace period, and that core rebuilds the screens as after an upgrade. A failure in one session's emulator SHALL NOT end the core or other sessions; that session's replay is then only a reset, and live output continues. A holder crash ends only that holder's sessions. A failure while a holder handles a message for one session SHALL NOT end the holder or its other sessions.

#### Scenario: Core killed
- **WHEN** the core is killed with SIGKILL while sessions are running
- **THEN** within a few seconds a new core is running, every session is still listed with the same pid, and attached clients reconnect

#### Scenario: Emulator fault
- **WHEN** the emulator of one session panics on its output
- **THEN** that session's program keeps running and its client keeps receiving output, and every other session is unaffected

#### Scenario: Holder fault while starting a session
- **WHEN** a holder fails while starting a session
- **THEN** the client is told the session could not start, with the cause, and the holder's other sessions keep running with the same pids

#### Scenario: Holder fault on a running session's message
- **WHEN** a holder fails while handling input for one of its two running sessions
- **THEN** the core is sent an error for that session, both programs keep running with the same pids, and the holder keeps carrying both sessions' input and output

### Requirement: Management commands
`brindle-remote` SHALL provide `list`, `kill <id>`, `kill --all`, `status` and `stop` for the server of the current server directory. `list` and `status` SHALL accept `--json`. When no server is running, `list` prints nothing and `status` says so, and both exit successfully. `kill` with an unknown id SHALL fail with an error naming the id.

#### Scenario: List
- **WHEN** a server has two sessions and the user runs `brindle-remote list`
- **THEN** one line per session shows its id, pid, attached or detached state, policy, age, title, command and working directory

#### Scenario: Kill one
- **WHEN** the user runs `brindle-remote kill <id>` for an attached session
- **THEN** that session's program is hung up and its client receives the session's exit

#### Scenario: Status
- **WHEN** the user runs `brindle-remote status --json`
- **THEN** it prints the core's version, protocol versions, pid, uptime and client count, and for each holder its pid, build, session count and whether it is draining

#### Scenario: Stop
- **WHEN** the user runs `brindle-remote stop`
- **THEN** every session is hung up, every client receives its sessions' exits, and the core and holders exit

### Requirement: Sessions are identified beyond their id
Each session SHALL have a random key, reported when it starts and in the session list, that no other session of any server lifetime shares. The session list SHALL also report each session's title, its start time and its policy. A client SHALL be able to tell a session it recorded from a different session that later got the same id.

#### Scenario: Same id, different key
- **WHEN** the server stops, a new server starts, and its first session gets id 1 as a session of the old server did
- **THEN** the two sessions' keys differ

#### Scenario: Title listed
- **WHEN** a session's program sets its title to `build` and a client lists sessions
- **THEN** that session's entry has title `build`

### Requirement: Attaching takes ownership
A client that attaches a session SHALL become its owner: it gets the session back on reconnect, as defined by "Clients reconnect after a core restart", and the session's policy applies when that client is gone.

#### Scenario: Attached kill session
- **WHEN** client B attaches a detached `kill` session and B is killed with SIGKILL
- **THEN** the session's program ends within a few seconds

### Requirement: Server outlives the desktop's app scope
On Linux with a systemd user manager, the core and the holders SHALL run outside the cgroup of the client that started them, so stopping that client's desktop app scope, or an out-of-memory kill of it, doesn't end the sessions. Without a user manager they SHALL start as before. Sessions SHALL NOT be required to survive the end of the user's login session.

#### Scenario: App scope stopped
- **WHEN** Brindle was launched from GNOME, has a `detach` session running `sleep 1000`, and its `app-gnome-brindle-*.scope` is stopped with `systemctl --user stop`
- **THEN** the `sleep` keeps running and is listed as detached

#### Scenario: No user manager
- **WHEN** no systemd user manager is reachable
- **THEN** the server starts and serves sessions as it did before this change

### Requirement: The server runs on Windows
Every requirement of this capability SHALL hold on Windows. Where a requirement names a Unix mechanism, the Windows form applies: a program is ended with `TerminateProcess` where SIGKILL is named, and a session is hung up by closing its pseudoconsole, which ends the processes attached to its console, then ending its program if it is still running after a grace period.

#### Scenario: Client killed with kill
- **WHEN** on Windows a client that started a `kill` session is ended with `taskkill /F`
- **THEN** the session's program and the console processes under it are gone within a few seconds

#### Scenario: Core killed on Windows
- **WHEN** on Windows the core is ended with `taskkill /F` while sessions are running
- **THEN** within a few seconds a new core is running, every session is still listed with the same pid, and attached clients reconnect

#### Scenario: Upgrade on Windows
- **WHEN** a Brindle of version N has local tabs open on Windows, and a Brindle of version N+1 starts
- **THEN** the core runs version N+1, and the version N Brindle's tabs reconnect and keep working with the same programs, pids and screens

### Requirement: Programs run on pseudoconsoles on Windows
On Windows, a holder SHALL run each session's program on a pseudoconsole it creates and keeps for the session's whole life, with the ConPTY host chosen as `terminal-emulation`'s "Pseudoconsole host on Windows" describes. The choice SHALL follow `BRINDLE_CONPTY` in the session's environment, not the holder's.

#### Scenario: Inbox host with a running holder
- **WHEN** a holder is already running and a client starts a session whose environment has `BRINDLE_CONPTY=inbox`
- **THEN** that session's console host is `conhost.exe`, and the holder's other sessions keep the shipped host

### Requirement: Server roles match the client's elevation on Windows
On Windows, an elevated client and an unelevated client SHALL use different servers. Each server's endpoints SHALL refuse a peer of lower integrity than its own, and a client SHALL refuse a server of a different integrity level, so an unelevated process can never start a program through an elevated server.

#### Scenario: Elevated and unelevated Brindle
- **WHEN** an unelevated Brindle and an elevated Brindle each open a local tab
- **THEN** two servers run, and `brindle-remote list` run unelevated lists only the unelevated tab's session

#### Scenario: Unelevated process tries the elevated server
- **WHEN** an unelevated process opens the elevated server's core pipe
- **THEN** the open fails or the connection is closed before any request is read

### Requirement: Server outlives the starter's job on Windows
On Windows, server roles SHALL start outside the job object of the process that starts them when the job allows that. When the job doesn't allow it, they SHALL start inside it, and the starter's log SHALL say once that sessions won't outlive that job.

#### Scenario: Started from the Start Menu
- **WHEN** Brindle was started from the Start Menu with a `detach` session running `ping -t localhost`, and Brindle is ended with `taskkill /F`
- **THEN** the `ping` keeps running and is listed as detached

#### Scenario: Process tree ended
- **WHEN** Brindle has a `detach` session running `ping -t localhost`, and Brindle's process tree is ended with `taskkill /T /F`
- **THEN** the `ping` keeps running and is listed as detached

#### Scenario: Job that forbids breakaway
- **WHEN** Brindle runs inside a job that forbids breakaway, and opens a local tab
- **THEN** the tab works, and Brindle's log says sessions won't outlive the job

### Requirement: Installed helper stays replaceable on Windows
On Windows, no server role SHALL run from the installed `brindle-remote.exe`. Each SHALL run from a copy for its build under `%LOCALAPPDATA%\brindle\server\` (inside the server directory when `BRINDLE_SERVER_DIR` is set; for an elevated process, under its elevated files directory, see "Elevated server files are out of an unelevated process's reach on Windows"), so replacing or removing the installed files while sessions run succeeds. A copy SHALL be removed once no server role runs from it and a newer build has started.

#### Scenario: Replace the helper while sessions run
- **WHEN** local tabs are running on Windows and `brindle-remote.exe` in Brindle's install directory is replaced with another build
- **THEN** the replacement succeeds, and the tabs keep working

### Requirement: Console windows follow Brindle's window on Windows
On Windows, a client SHALL be able to name the window that owns a session's console window. The server SHALL apply it asynchronously, ignore a window that no longer exists, and clear the owner when given none. A client SHALL name its window again after attaching, after reconnecting, and whenever the session moves to another window. A session whose owner window is gone SHALL keep its console window, without an owner. A Linux core SHALL refuse the request.

#### Scenario: Dialog owned by Brindle
- **WHEN** a program in a Windows tab opens a message box owned by its console window
- **THEN** the message box is owned by Brindle's window, and stays above it

#### Scenario: Owner after reattach
- **WHEN** a Windows session was detached by quitting Brindle and a new Brindle reattaches it, and the program then opens a message box owned by its console window
- **THEN** the message box is owned by the new Brindle's window

### Requirement: Relay to the server
`brindle-remote relay` SHALL connect its standard input and output to the core of the server directory it resolves, as a client of that core, starting the core when none runs, so that a client on another machine can reach the server over any byte stream (SSH, a container's `exec`). Before relaying, it SHALL report its version, build and environment, the user's login shell and home directory. It SHALL relay protocol frames unchanged in both directions, and end when either side closes.

#### Scenario: Relay starts the server
- **WHEN** no server runs for the user, and `brindle-remote relay` is started with its standard input and output connected to a client
- **THEN** a core runs, its parent is not the relay, and the client can start a session through the relay

#### Scenario: Relay ends with its client
- **WHEN** a relay's client closes the relay's standard input
- **THEN** the relay exits, and the core treats that client as gone

### Requirement: Relay liveness
The relay SHALL exchange heartbeats with its client on its standard input and output, separately from the protocol frames it relays: it sends one when it has written nothing for 10 s, and drops the client's without relaying them. It SHALL end when it has read nothing from its client for 60 s. The client's silence SHALL be measured from what the relay reads, independently of whether its writes to the client are blocked.

#### Scenario: Silent client
- **WHEN** a relay's client stays connected but sends nothing, not even heartbeats, for 60 s
- **THEN** the relay exits within a few seconds after that

#### Scenario: Heartbeats aren't relayed
- **WHEN** a client sends heartbeats through a relay to a core of an older release
- **THEN** the core receives none of them, and the connection keeps working

### Requirement: Relays offer upgrades
Before relaying, a relay whose binary is newer than the running core (by the rules a local client uses) SHALL check that it speaks the running holders' protocol versions, ask the core to step down, and start the next core from its own binary. A failed check or a refused step-down SHALL leave the running core in place, and the relay SHALL relay to it.

#### Scenario: Newer Brindle connects
- **WHEN** a host's core is version N with a session running `htop`, and a Brindle of version N+1 connects through its relay
- **THEN** the host's core runs version N+1, and `htop` keeps running with the same pid

#### Scenario: Older Brindle connects
- **WHEN** a host's core is version N+1, and a Brindle of version N connects through its relay
- **THEN** the core keeps running, and the version N Brindle's tabs work through it

### Requirement: A server started by a relay outlives the connection
A core or holder started by a relay SHALL keep running after the SSH login or `exec` that started it ends, on hosts where the user's processes are allowed to outlive their logins. It SHALL NOT be placed where the end of the user's last login would stop it. A server that ends with the container it runs in is expected.

#### Scenario: Last SSH login ends
- **WHEN** a relay started over SSH on a systemd host without user lingering started the server, a `detach` session runs `sleep 1000`, and the SSH connection ends while the user has no other login
- **THEN** the `sleep` keeps running and is listed as detached at the next connect

### Requirement: A reconnecting client replaces its old connection
When a client connects with the token of a client that is still connected, the core SHALL close the older connection and give its sessions to the new one, replaying each, without their close policies applying.

#### Scenario: Half-dead connection
- **WHEN** a client's connection stops delivering data without closing, and the client connects again with the same token while the core still holds the old connection
- **THEN** the old connection is closed, and the new one receives a replay of every session the old one was attached to, with the same pids

### Requirement: Clients may ask for a reconnect grace
A client SHALL be able to ask, when it connects, that its `kill` sessions wait for it for a stated time, at most 10 minutes, after its connection ends. Such a session SHALL end when that time passes without the client reconnecting with the same token. If the client reconnects in time, the session SHALL be given back to it as after a core restart. Without the request, `kill` sessions SHALL end as soon as the connection ends.

#### Scenario: Back within the grace
- **WHEN** a client that asked for a 300 s grace loses its connection while its `kill` session runs `htop`, and reconnects after 60 s
- **THEN** it receives `htop`'s replay, with the same pid

#### Scenario: Not back
- **WHEN** a client that asked for a 300 s grace loses its connection and doesn't reconnect
- **THEN** its `kill` sessions end about 300 s later

### Requirement: Process info for clients that ask
A client that negotiated client protocol 2 and asks for it SHALL receive, for each session it is attached to, the name and working directory of the session's foreground process: once after it attaches or starts the session, and again whenever either changes. The core SHALL look at most once a second per session, and only at sessions that have had output or input since its last look. A core on Windows MAY report no process info.

#### Scenario: Foreground process changes
- **WHEN** a client that asked for process info runs `htop` in its session's shell
- **THEN** within about a second it receives process info naming `htop`

#### Scenario: Idle session
- **WHEN** a session has had neither output nor input for a minute
- **THEN** its client receives no process info in that minute

### Requirement: The session list tells whether an owner may return
For each detached session, the session list SHALL report whether it still has an owner (its last client left without detaching it) and how long it has been detached.

#### Scenario: Owner dropped
- **WHEN** a client's connection to a `detach` session ends without a `DETACH`, and another client lists sessions 30 s later
- **THEN** the session is listed as detached, owned, and detached for about 30 s

#### Scenario: Explicit detach
- **WHEN** a client detaches a session and another client lists sessions
- **THEN** the session is listed as detached and not owned

### Requirement: Programs start with default signal handling
A holder SHALL start each session's program with every signal's default disposition and an empty signal mask, whatever the holder inherited. Hanging up a session SHALL end a program that hasn't changed its own signal handling with SIGHUP, even when the server was started with SIGHUP ignored. On Linux the holder SHALL send SIGHUP before it closes the session's terminal, so a program blocked reading its terminal is ended by the signal, not by the failed read.

#### Scenario: Server started under nohup
- **WHEN** the server was started with SIGHUP ignored, a session runs `cat`, and the session is closed
- **THEN** `cat` is ended by SIGHUP, and is gone well before the holder's SIGKILL would be sent

#### Scenario: A program blocked reading its terminal
- **WHEN** a session runs `cat`, which is blocked reading its terminal, and the session is closed, any number of times
- **THEN** each time the session's exit reports `cat` ended by SIGHUP, never an exit status from a failed read

### Requirement: Sessions keep their client's layout
A client SHALL be able to give a session a layout, data the server doesn't interpret describing where the session sits: with `DETACH`, and, on protocol 3, at any time while it has the session attached. The server SHALL keep the latest layout across core restarts and upgrades and when the client's connection ends, and report it in the session list while the session is detached. It SHALL drop it on attach, and SHALL NOT keep one over 64 KiB.

#### Scenario: Layout listed after a detach
- **WHEN** a client detaches a session with a layout and another client lists sessions
- **THEN** that session's entry has the same layout

#### Scenario: Layout updated while attached
- **WHEN** a client on protocol 3 sets a session's layout twice while attached, and is then killed with SIGKILL
- **THEN** listing sessions reports the second layout for it

#### Scenario: Layout survives a core restart
- **WHEN** a session was detached with a layout, its core is killed with SIGKILL, and a new core adopts the holder
- **THEN** listing sessions from the new core reports the same layout

#### Scenario: Attach clears the layout
- **WHEN** a session detached with a layout is attached by a client, which then detaches it without a layout
- **THEN** listing sessions reports no layout for it

#### Scenario: Only the attached client sets it
- **WHEN** a client sets the layout of a session another client has attached
- **THEN** it gets an error naming the session, and the session's layout is unchanged

#### Scenario: Oversized layout
- **WHEN** a client detaches a session with a 100 KiB layout
- **THEN** the session is detached and listed with no layout

#### Scenario: Older client
- **WHEN** a client that predates layouts detaches a session
- **THEN** the session is detached as before and listed with no layout

### Requirement: Elevated server files are out of an unelevated process's reach on Windows
On Windows, every directory an elevated Brindle or elevated server role runs, loads, writes or removes files in (its server directory, its helper copies and its ConPTY files) SHALL be one an unelevated process of the same user can't write or replace. One that is a reparse point, or that it didn't make that way, SHALL be refused or replaced before use, and nothing inside SHALL be followed through a reparse point.

#### Scenario: Planted helper copy
- **WHEN** an unelevated process writes a `brindle-remote.exe` where the elevated server's helper copy for the current build goes, and an elevated Brindle then opens a local tab
- **THEN** the planted file is never run, and the tab works

#### Scenario: Unelevated write into the elevated directories
- **WHEN** an elevated Brindle has a local tab open, and an unelevated process of the same user tries to create, replace or delete a file in the elevated server directory, helper copies or ConPTY files
- **THEN** each attempt fails with access denied

#### Scenario: Junction where the copies go
- **WHEN** an unelevated process makes the elevated copies directory a junction to another directory before an elevated Brindle starts
- **THEN** the elevated Brindle doesn't use or prune through the junction, and nothing in the junction's target is removed

### Requirement: Bare program names resolve through PATH on Windows
On Windows, a session's program named without a directory SHALL be found through `PATH`, never in the server role's own directory.

#### Scenario: Program planted beside the helper
- **WHEN** a `cmd.exe` is placed in the directory of the helper copy the server runs from, and a profile with `command = "cmd.exe"` opens a tab
- **THEN** the tab runs `cmd.exe` from `PATH`, not the placed file

### Requirement: Command lines survive attach
The server SHALL keep each command's reported line (`OSC 633 ; E`) with its prompt marks, and an attaching or reconnecting client SHALL receive it with the marks. A client attached to a server that predates command-line tracking SHALL have the marks without the reported lines.

#### Scenario: Copy after attach
- **WHEN** a detached session's shell reported `make test` with `633;E`, and a client attaches and runs Copy Last Command Line
- **THEN** the clipboard holds `make test`

### Requirement: Progress survives attach
The server SHALL keep each session's progress (`OSC 9 ; 4`) with the same lifetime rules as the client, and an attaching or reconnecting client SHALL receive it with the session's replay. A client attached to a server that predates progress tracking SHALL show no progress for that session until the program reports again.

#### Scenario: Progress after attach
- **WHEN** a program in a detached session reported normal at 60%, and a client attaches
- **THEN** the session's tab shows a bar 60% of its width long

#### Scenario: Core restart
- **WHEN** a session's program reported paused at 20% and the core is restarted
- **THEN** after the client reconnects, the tab shows a yellow bar 20% of its width long

### Requirement: Contexts survive attach
The server SHALL keep each session's OSC 3008 contexts with the same rules as the client, and an attaching or reconnecting client SHALL replace the session's contexts with those in its replay. A client attached to a server that predates context tracking SHALL show no contexts for that session until a program starts one.

#### Scenario: Attach inside run0
- **WHEN** a detached session's shell runs inside a `run0` elevation, and a client attaches
- **THEN** the session's tab shows the yellow warning icon for `root`

#### Scenario: Ended while disconnected
- **WHEN** the client loses its connection while a zone is inside a container context, the context ends meanwhile, and the client reconnects
- **THEN** the zone shows no badge
