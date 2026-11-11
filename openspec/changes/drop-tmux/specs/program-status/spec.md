## ADDED Requirements

### Requirement: Report recognition
Brindle SHALL recognise the Program Status Protocol (OSC 7501, revision 0.1) in the output of every local tab: `ESC ] 7501 ; <pairs>`, terminated by BEL or ST, the body being `key=value` pairs separated by `:`. Reports SHALL be recognised however the output is split into reads and SHALL NOT disturb the rest of the output. A cancelled sequence (CAN, SUB, or ESC not followed by `\`) SHALL be ignored.

#### Scenario: The specification's example
- **WHEN** a program prints `ESC ] 7501 ; state=blocked:kind=permission:app=terraform:msg=QXBwbHkgMyB0byBhZGQsIDEgdG8gY2hhbmdlLCAwIHRvIGRlc3Ryb3k/ ESC \`
- **THEN** the terminal's root record is `blocked`, kind `permission`, app `terraform`, with the message `Apply 3 to add, 1 to change, 0 to destroy?`

#### Scenario: Split across reads
- **WHEN** a program writes `ESC ] 7501 ; state=wor` and, in a later write, `king:app=cargo ESC \`
- **THEN** that terminal's root record is `working` with app `cargo`

## MODIFIED Requirements

### Requirement: Records
Each terminal (a local tab or zone) SHALL hold a set of records, one per id, where a report with no `id` addresses the root record. A report SHALL replace its record completely, so a key missing from the report is missing from the record. A record without `app` SHALL take the `app` of its nearest ancestor that has one (`build/test` is beneath `build`), when shown. Switching between the primary and alternate screen SHALL NOT affect records.

#### Scenario: Replaced whole
- **WHEN** a program reports `state=working:app=brew:msg=…` and then `state=done`
- **THEN** the root record is `done` with no app and no message

#### Scenario: App inherited
- **WHEN** the root record has `app=deploy` and a program reports `state=blocked:id=eu-west` without `app`
- **THEN** the `eu-west` record is shown with the app `deploy`

## ADDED Requirements

### Requirement: Feature query
When a program prints `ESC ] 7501 ; ? ST` (or BEL), Brindle SHALL write `ESC ] 7501 ; ? ESC \` back to that program, on the PTY of its terminal. Nothing else from a record SHALL ever be written back.

#### Scenario: Query from a shell
- **WHEN** a local tab runs `printf '\e]7501;?\e\\'; read -r -t 2 -d '\\' r; printf '%s\n' "$r" | tr -d '\033'`
- **THEN** the screen shows a line `]7501;?`

#### Scenario: Query from one zone
- **WHEN** a program in one zone of a split tab prints the query
- **THEN** the reply arrives on that zone's input and no other zone's

## REMOVED Requirements

### Requirement: Report syntax
**Reason**: Rewritten without tmux control-mode panes and the tmux scenario. Renamed because a modified requirement can't drop a scenario.
**Migration**: See "Report recognition".

### Requirement: Feature detection
**Reason**: Rewritten without tmux control-mode panes and the pane scenario. Renamed because a modified requirement can't drop a scenario.
**Migration**: See "Feature query".
