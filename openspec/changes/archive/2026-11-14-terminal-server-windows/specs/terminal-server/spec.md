## MODIFIED Requirements

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

## ADDED Requirements

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
On Windows, no server role SHALL run from the installed `brindle-remote.exe`. Each SHALL run from a copy for its build under `%LOCALAPPDATA%\brindle\server\` (inside the server directory when `BRINDLE_SERVER_DIR` is set), so replacing or removing the installed files while sessions run succeeds. A copy SHALL be removed once no server role runs from it and a newer build has started.

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
