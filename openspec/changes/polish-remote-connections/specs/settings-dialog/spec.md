## ADDED Requirements

### Requirement: Profile editor rows share one height
The profile editor SHALL lay its field rows out on one height increment, half the height of a text field. Every row's control line SHALL be two increments tall with its control centred in it, whatever the control: a text field, a choice, a switch or a checkbox. A row showing a validation error SHALL add one increment beneath its control line for the message. No field's label SHALL wrap onto a second line.

#### Scenario: Switch row matches a text field row
- **WHEN** the user edits an ssh profile
- **THEN** the "Connect at launch" row is as tall as the "Host" row, and its switch is centred in that height

#### Scenario: An error adds one increment
- **WHEN** a field shows a validation error
- **THEN** that row is three increments tall, and the rows below it move down by one increment
