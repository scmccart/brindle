## ADDED Requirements

### Requirement: Prompt input suggestions
Given suggestions, a prompt input SHALL show them as a list above its textarea: each row a label, an optional hint and an optional description cut to one line, with the row the owner names highlighted. The list SHALL take no focus, so typing goes on in the textarea. A primary press on a row SHALL ask the owner to accept it. The owner SHALL hold the rows and the highlight; the prompt input SHALL change neither, nor its text.

#### Scenario: Shown above the text
- **WHEN** the owner gives a prompt input three suggestions with the second highlighted
- **THEN** three rows show above the textarea, the second highlighted, and the textarea keeps focus

#### Scenario: A press asks the owner
- **WHEN** the user presses the third row
- **THEN** the owner is asked to accept the third suggestion, and the text is unchanged until the owner changes it

#### Scenario: Taken away
- **WHEN** the owner gives no suggestions
- **THEN** no list shows

### Requirement: Prompt input footer items
Given footer items, a prompt input SHALL show them under its textarea as small labels separated by a middle dot, each asking the owner, with its key, on a primary press. They SHALL NOT be keyboard stops, and a prompt input given none SHALL show no footer.

#### Scenario: Items
- **WHEN** the owner gives footer items `Auto`, `Opus` and `Medium`
- **THEN** the footer shows `Auto · Opus · Medium`, and pressing `Opus` asks the owner with that item's key

#### Scenario: Tab skips them
- **WHEN** the textarea has focus and the user presses tab
- **THEN** focus doesn't land on a footer item
