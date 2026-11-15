## ADDED Requirements

### Requirement: Profile editor rows share one control height
The profile editor SHALL give every field row's control line at least the height of a text field, with its control centred in it, whatever the control: a text field, a choice, a switch or a checkbox. A validation error SHALL show beneath its row's control line at the message's natural height, wrapping when it is longer than the row is wide. No field's label SHALL wrap onto a second line.

#### Scenario: Switch row matches a text field row
- **WHEN** the user edits an ssh profile
- **THEN** the "Connect at launch" row is as tall as the "Host" row, and its switch is centred in that height

#### Scenario: A long error wraps
- **WHEN** a field shows a validation error longer than the row is wide
- **THEN** the message wraps onto further lines beneath the row, and the rows below it move down to make room
