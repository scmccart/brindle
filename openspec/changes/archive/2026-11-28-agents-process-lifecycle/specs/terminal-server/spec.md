## MODIFIED Requirements

### Requirement: Session I/O
For an attached session, the server SHALL carry the client's input and resizes to the PTY, stream its output to the client in order, and report the program's exit with its exit status. One session's slow or stalled client SHALL NOT delay other sessions. For a session whose program runs on pipes (an agent session), the output the program wrote before it exited SHALL be read and passed on before its exit is reported, even while the session's reads are paused for a slow client, up to a bound of 1 MiB read past the pause after the exit; output written after the exit by a process the program left running is read within the same bound and the existing two-second drain limit.

#### Scenario: Exit status
- **WHEN** the program in an attached session runs `exit 3`
- **THEN** the client receives the session's exit with status 3, and the session is no longer listed

#### Scenario: A stalled client
- **WHEN** one client stops reading while its program floods output
- **THEN** sessions of other clients keep streaming output without delay

#### Scenario: Last output of a paused pipe session
- **WHEN** a pipe session's reads are paused, and its program writes 48 KiB of lines to its standard output and exits
- **THEN** every line reaches the core, in order, before the session's exit

#### Scenario: A process left writing after the exit
- **WHEN** a paused pipe session's program exits, leaving a process it started that holds its standard output and writes without end
- **THEN** the session's exit is reported within the drain limit, and at most 1 MiB was read past the pause
