## MODIFIED Requirements

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
