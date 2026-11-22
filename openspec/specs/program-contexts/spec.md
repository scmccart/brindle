# program-contexts Specification

## Purpose
Track the nested contexts (elevation, containers, VMs, remote logins, commands) that systemd and other programs announce with OSC 3008, so Brindle can show where a terminal's output is coming from.

## Requirements

### Requirement: Context sequence recognition
Brindle SHALL recognise `ESC ] 3008 ; start=<id> [; <field>=<value>]…` and `ESC ] 3008 ; end=<id> [; <field>=<value>]…`, ended by BEL or ST, in the output of every terminal, however the output is split into reads. `start=` or `end=` SHALL come first; otherwise the sequence is ignored. A cancelled sequence or one longer than 4096 bytes SHALL be ignored. No sequence SHALL change the grid.

#### Scenario: run0
- **WHEN** `run0` starts a shell and prints `ESC ] 3008 ; start=bdb3ea70355040c9979877f40a12ac0a ; user=root ; hostname=A8 ; … ; targetuser=root ; sessionid=c6 ; type=session ESC \`, and never prints a matching `end`
- **THEN** the terminal's active context is `bdb3ea70355040c9979877f40a12ac0a`, of type `session`, with target user `root`, until its program exits or an outer context is started again

#### Scenario: Elevate
- **WHEN** a program prints `ESC ] 3008 ; start=a1 ; type=elevate ; user=me ; targetuser=root ESC \`
- **THEN** the terminal's active context is `a1`, of type `elevate`, with target user `root`

#### Scenario: Not first
- **WHEN** a program prints `ESC ] 3008 ; type=elevate ; start=a1 ESC \`
- **THEN** the terminal's contexts don't change

### Requirement: Context identifiers and fields
An id SHALL be 1 to 64 bytes in 0x20–0x7E without a raw `;` or `\`; a sequence with any other id SHALL be ignored. In ids and values, `\xHH` SHALL decode to that byte. Fields are `name=value`, in any order. An unknown field, a value over 255 bytes, a value with a control character or invalid UTF-8 after decoding, and a `type` outside UAPI.15's list SHALL be dropped, and the rest kept.

#### Scenario: Escaped separator
- **WHEN** a program prints `ESC ] 3008 ; start=c1 ; type=container ; container=web\x3bdb ESC \`
- **THEN** context `c1` has the container name `web;db`

#### Scenario: Bad type
- **WHEN** a program prints `ESC ] 3008 ; start=x ; type=spaceship ; hostname=h ESC \`
- **THEN** context `x` exists with no type and the hostname `h`

### Requirement: Context nesting
A start with a new id SHALL make it the active context, its parent being the context active before. A start with a known id SHALL replace that context's fields, end its subcontexts and make it active. An end SHALL end that context and its subcontexts, making its parent active; an end for an unknown id SHALL be ignored. Past 32 contexts, a new start SHALL be ignored and earlier contexts kept.

#### Scenario: Container inside an elevation
- **WHEN** contexts `a` (elevate) then `b` (container) start, and then `ESC ] 3008 ; end=a ESC \` arrives
- **THEN** the terminal has no contexts

#### Scenario: Unknown end
- **WHEN** context `a` is active and `ESC ] 3008 ; end=zzz ESC \` arrives
- **THEN** `a` is still active

#### Scenario: Too deep
- **WHEN** 32 contexts are open and a 33rd starts
- **THEN** the 33rd is not recorded, and an `end` for the 32nd makes the 31st active

### Requirement: Context lifetime
A terminal's contexts SHALL be dropped when its process exits. A full reset (`ESC c`), a soft reset, a new shell prompt and switching screens SHALL NOT affect them, and they SHALL NOT expire on their own.

#### Scenario: Reset inside run0
- **WHEN** a context of type `elevate` is active and the program prints `ESC c`
- **THEN** the context is still active

### Requirement: Notable context
A terminal's notable context SHALL be the innermost context of type `elevate`, `chpriv`, `remote`, `container` or `vm`, or of type `session` whose target user is exactly `root` (a root login, which is how `run0` reports itself), among the active context and its ancestors. A `session` with any other target user, or none, SHALL NOT be notable. Its label SHALL be the target user (else `elevated`) for `elevate` and `chpriv`, `root` for a `session`; `targetuser@targethost`, the target host alone, else `remote` for `remote`; and the container or VM name, else `container` or `vm`. Labels over 24 characters SHALL end in `…`.

#### Scenario: Shell inside a container
- **WHEN** a `container` context with `container=web` is active, and a `shell` context starts inside it
- **THEN** the terminal's notable context is the container, labelled `web`

#### Scenario: No notable context
- **WHEN** only `shell` contexts and a `session` with `targetuser=scmccart` are open
- **THEN** the terminal has no notable context

#### Scenario: Root session
- **WHEN** a `shell` and a `command` context are open, and a `session` context with `targetuser=root` starts inside them
- **THEN** the terminal's notable context is that session, privileged, labelled `root`

### Requirement: Detected elevation on Windows
A local Windows terminal SHALL be detected as elevated while its foreground program, the process its tab title names, runs elevated (high integrity or above), with no OSC 3008 needed. Its notable context SHALL then be privileged, labelled `Administrator`, whatever OSC 3008 context a program announces, which decides only while the terminal isn't elevated. Detection SHALL NOT block the UI thread. While a process's elevation can't be read, it SHALL count as not elevated.

#### Scenario: Elevated Brindle
- **WHEN** Brindle runs elevated and opens a local PowerShell tab
- **THEN** the terminal's notable context is privileged, labelled `Administrator`, and the tab face shows the warning badge with the tooltip "Running as Administrator"

#### Scenario: Elevated program from the tab's shell
- **WHEN** in an unelevated Brindle a user runs a program that stays the tab's descendant and runs elevated, and then it exits
- **THEN** the tab shows the privileged badge while that program runs, and none once it has exited

#### Scenario: sudo inline not detected
- **WHEN** in an unelevated Brindle a user runs `sudo pwsh` with Windows sudo in inline mode and approves the prompt
- **THEN** the badge may show for about a second while sudo's elevated helper runs, and none once the elevated shell's prompt shows, since that shell is no longer a descendant of the tab's program

#### Scenario: Detected elevation can't be hidden
- **WHEN** an elevated tab's program prints `ESC ] 3008 ; start=c1 ; type=container ; container=web ESC \`, or an `elevate` context with `targetuser=guest`
- **THEN** the terminal's notable context stays privileged, labelled `Administrator`

#### Scenario: Announced context in an unelevated tab
- **WHEN** an unelevated tab's program prints `ESC ] 3008 ; start=c1 ; type=container ; container=web ESC \`
- **THEN** the terminal's notable context is the container, labelled `web`

#### Scenario: Unelevated tab
- **WHEN** an unelevated Brindle opens a local tab running `cmd.exe`
- **THEN** the terminal has no notable context and its tab face shows no badge

#### Scenario: Remote and Linux tabs
- **WHEN** a tab runs on a remote host, or Brindle runs on Linux
- **THEN** its notable context comes from OSC 3008 alone
