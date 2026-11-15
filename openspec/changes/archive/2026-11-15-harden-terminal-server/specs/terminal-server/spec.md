## MODIFIED Requirements

### Requirement: Programs start with default signal handling
A holder SHALL start each session's program with every signal's default disposition and an empty signal mask, whatever the holder inherited. Hanging up a session SHALL end a program that hasn't changed its own signal handling with SIGHUP, even when the server was started with SIGHUP ignored. On Linux the holder SHALL send SIGHUP before it closes the session's terminal, so a program blocked reading its terminal is ended by the signal, not by the failed read.

#### Scenario: Server started under nohup
- **WHEN** the server was started with SIGHUP ignored, a session runs `cat`, and the session is closed
- **THEN** `cat` is ended by SIGHUP, and is gone well before the holder's SIGKILL would be sent

#### Scenario: A program blocked reading its terminal
- **WHEN** a session runs `cat`, which is blocked reading its terminal, and the session is closed, any number of times
- **THEN** each time the session's exit reports `cat` ended by SIGHUP, never an exit status from a failed read
