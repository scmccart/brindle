# program-status Specification

## Purpose
TBD - created by archiving change program-status. Update Purpose after archive.

## Requirements

### Requirement: Report syntax
Brindle SHALL recognise the Program Status Protocol (OSC 7501, revision 0.1) in the output of every local tab and every tmux control-mode pane: `ESC ] 7501 ; <pairs>`, terminated by BEL or ST, the body being `key=value` pairs separated by `:`. Reports SHALL be recognised however the output is split into reads and SHALL NOT disturb the rest of the output. A cancelled sequence (CAN, SUB, or ESC not followed by `\`) SHALL be ignored.

#### Scenario: The specification's example
- **WHEN** a program prints `ESC ] 7501 ; state=blocked:kind=permission:app=terraform:msg=QXBwbHkgMyB0byBhZGQsIDEgdG8gY2hhbmdlLCAwIHRvIGRlc3Ryb3k/ ESC \`
- **THEN** the terminal's root record is `blocked`, kind `permission`, app `terraform`, with the message `Apply 3 to add, 1 to change, 0 to destroy?`

#### Scenario: Split across reads, in a tmux pane
- **WHEN** a program in a control-mode pane writes `ESC ] 7501 ; state=wor` and, in a later write, `king:app=cargo ESC \`
- **THEN** that pane's root record is `working` with app `cargo`

### Requirement: Pairs
Whitespace around keys and values SHALL be removed. A pair without `=`, with an empty key, or with a byte outside `[A-Za-z0-9_.,+/=-]` SHALL be skipped and the rest of the report processed. An unknown key SHALL be ignored, and the last value of a repeated key SHALL win.

#### Scenario: Unknown key, malformed pair and repeated key
- **WHEN** a program prints `ESC ] 7501 ; state=idle:colour=red:oops:state=working BEL`
- **THEN** the root record is `working`, and nothing else is recorded from the report

#### Scenario: Whitespace
- **WHEN** a program prints `ESC ] 7501 ; state = done : app = brew BEL`
- **THEN** the root record is `done` with app `brew`

### Requirement: Report keys
The keys SHALL be `state` (`idle`, `working`, `done`, `blocked`, `error` or `clear`; required), `id` (a path of `/`-separated segments of `[A-Za-z0-9_.+-]`), `kind` (`permission`, `question` or `auth`, kept with `blocked` only), `progress` (`0` to `100`, kept with `working` or `blocked` only; out of range counts as absent), `app`, `title` and `msg`. A report with a missing or unrecognised `state` SHALL be ignored whole.

#### Scenario: Unknown state
- **WHEN** a program prints `ESC ] 7501 ; state=paused:app=x BEL`
- **THEN** the terminal's records don't change

#### Scenario: Kind outside blocked
- **WHEN** a program prints `ESC ] 7501 ; state=working:kind=auth:progress=101 BEL`
- **THEN** the root record is `working` with no kind and no progress

### Requirement: Text keys
`app` SHALL match `[A-Za-z0-9_.+-]{1,32}`. `title` and `msg` SHALL be standard base64 of UTF-8 text, with padding optional; `msg` is one line saying what the record is doing, waiting for, or finished, and `title` a short label for the record.

#### Scenario: Unpadded message
- **WHEN** a program prints `ESC ] 7501 ; state=done:msg=SGk BEL`
- **THEN** the root record's message is `Hi`

### Requirement: Report limits
A report SHALL be discarded whole, changing no record, when `app` or `id` doesn't match its grammar, when a base64 value doesn't decode, when decoded text contains a control character (U+0000–U+001F, U+007F, U+0080–U+009F), or when it exceeds a limit: 4096 bytes for the sequence, 16 bytes per key, 2732 encoded and 2048 decoded bytes for `msg`, 256 and 192 for `title`, 32 for `app`, and for `id` 128 bytes, 32 per segment and 8 segments.

#### Scenario: Control character in the message
- **WHEN** a program prints a report whose `msg` decodes to text containing U+001B
- **THEN** the terminal's records don't change

#### Scenario: Message too long
- **WHEN** a program prints a report whose `msg` is 2736 base64 bytes
- **THEN** the terminal's records don't change

### Requirement: Records
Each terminal (a local tab, or one tmux pane) SHALL hold a set of records, one per id, where a report with no `id` addresses the root record. A report SHALL replace its record completely, so a key missing from the report is missing from the record. A record without `app` SHALL take the `app` of its nearest ancestor that has one (`build/test` is beneath `build`), when shown. Switching between the primary and alternate screen SHALL NOT affect records.

#### Scenario: Replaced whole
- **WHEN** a program reports `state=working:app=brew:msg=…` and then `state=done`
- **THEN** the root record is `done` with no app and no message

#### Scenario: App inherited
- **WHEN** the root record has `app=deploy` and a program reports `state=blocked:id=eu-west` without `app`
- **THEN** the `eu-west` record is shown with the app `deploy`

### Requirement: Clearing records
`state=clear` SHALL remove the addressed record and every record beneath it, and with no `id` SHALL remove every record of the terminal. A full reset (`ESC c`) SHALL remove every record; a soft reset SHALL NOT. A terminal SHALL hold at most 256 records; when a report would create one more, the record updated least recently SHALL be removed.

#### Scenario: Clearing a subtree
- **WHEN** records exist for the root, `build`, `build/test` and `deploy`, and a program reports `state=clear:id=build`
- **THEN** the root and `deploy` records remain and the other two are gone

#### Scenario: Past the cap
- **WHEN** a terminal holds 256 records and a program reports a 257th id
- **THEN** the new record exists and the record updated longest ago is gone

### Requirement: Record lifetimes
When a new shell prompt begins (OSC 133 `A`) or the terminal's process exits, the terminal SHALL drop its `working` and `blocked` records and keep `idle`, `done` and `error` ones. `done` and `error` records SHALL be dropped when the user types into that terminal. No record SHALL expire on its own, and none SHALL need re-sending to stay.

#### Scenario: A build that finishes and exits
- **WHEN** a program reports `state=working`, then `state=done:msg=…`, exits, and the shell prints its next prompt
- **THEN** the root record is still `done` after the prompt

#### Scenario: A program killed mid-way
- **WHEN** a program reports `state=blocked:kind=question`, is killed, and the shell prints its next prompt
- **THEN** the terminal has no root record

#### Scenario: Seen
- **WHEN** the root record is `error` and the user types a key into that terminal
- **THEN** the record is gone

### Requirement: Feature detection
When a program prints `ESC ] 7501 ; ? ST` (or BEL), Brindle SHALL write `ESC ] 7501 ; ? ESC \` back to that program: to the PTY of a local tab, or to the pane's input of a tmux control-mode pane. Nothing else from a record SHALL ever be written back.

#### Scenario: Query from a shell
- **WHEN** a local tab runs `printf '\e]7501;?\e\\'; read -r -t 2 -d '\\' r; printf '%s\n' "$r" | tr -d '\033'`
- **THEN** the screen shows a line `]7501;?`

#### Scenario: Query from a pane
- **WHEN** a program in a control-mode pane prints the query
- **THEN** the reply arrives on that pane's input and no other pane's
