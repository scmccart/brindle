# Spec Delta

## Purpose

Show how far along a long-running program is, from the ConEmu `OSC 9 ; 4` progress reports that Windows Terminal, Ghostty and ConEmu understand.

## ADDED Requirements

### Requirement: Progress report recognition
Brindle SHALL recognise `ESC ] 9 ; 4 ; <state> [; <percent>]`, ended by BEL or ST, in the output of every terminal, local or remote, however the output is split into reads. A cancelled sequence (CAN, SUB, or ESC not followed by `\`) SHALL be ignored. A report SHALL NOT raise a notification and SHALL NOT change what is drawn in the grid.

#### Scenario: Split across reads
- **WHEN** a program writes `ESC ] 9 ; 4 ; 1 ; 3` and, in a later write, `0 BEL`
- **THEN** the terminal's progress is normal at 30%

#### Scenario: No notification
- **WHEN** a program in a background tab prints `ESC ] 9 ; 4 ; 1 ; 50 BEL`
- **THEN** no notification appears and the screen text is unchanged

### Requirement: Progress states
The state SHALL be `0` (clear: the terminal has no progress), `1` (normal), `2` (error), `3` (indeterminate) or `4` (paused). The percent SHALL be a whole number; above 100 counts as 100, and missing or non-numeric as absent. Normal with no percent SHALL be 0%. Error and paused SHALL keep a given percent, else the terminal's previous percent, if any. Indeterminate SHALL carry no percent. Any other state SHALL be ignored.

#### Scenario: Over 100
- **WHEN** a program prints `ESC ] 9 ; 4 ; 1 ; 250 BEL`
- **THEN** the terminal's progress is normal at 100%

#### Scenario: Error keeps the percent
- **WHEN** a program prints `ESC ] 9 ; 4 ; 1 ; 60 BEL` and then `ESC ] 9 ; 4 ; 2 BEL`
- **THEN** the terminal's progress is error at 60%

#### Scenario: Unknown state
- **WHEN** a terminal's progress is normal at 10% and a program prints `ESC ] 9 ; 4 ; 7 ; 50 BEL`
- **THEN** the terminal's progress is still normal at 10%

#### Scenario: Clear
- **WHEN** a program prints `ESC ] 9 ; 4 ; 1 ; 50 BEL` and then `ESC ] 9 ; 4 ; 0 BEL`
- **THEN** the terminal has no progress

### Requirement: Progress lifetime
Each terminal (a tab or zone) SHALL hold at most one progress value, which the latest report replaces. The value SHALL be dropped when a new shell prompt begins (OSC 133 `A`), when the terminal's process exits, and on a full reset (`ESC c`). Switching between the primary and alternate screen SHALL NOT affect it, and it SHALL NOT expire on its own.

#### Scenario: Killed mid-way
- **WHEN** a program reports `ESC ] 9 ; 4 ; 1 ; 40 BEL`, is killed with ctrl-c, and the shell prints its next prompt
- **THEN** the terminal has no progress

#### Scenario: Stays without re-sending
- **WHEN** a program reports normal at 40% and prints nothing more for a minute
- **THEN** the terminal's progress is still normal at 40%
