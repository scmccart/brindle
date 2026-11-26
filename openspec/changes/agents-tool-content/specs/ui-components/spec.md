## MODIFIED Requirements

### Requirement: Tool rows
A tool row SHALL be one line: an icon the owner chooses, a title cut with an ellipsis, an optional summary the owner gives, and a status badge reading pending, running, done or failed, the failed one in the destructive colour. When the owner says the row opens, it SHALL also show a chevron for whether it is open, and a primary press on the line SHALL ask the owner to flip it; the owner SHALL hold that state and draw the content. It SHALL take no focus.

#### Scenario: Long title
- **WHEN** a tool row's title is wider than the row
- **THEN** the title is cut with an ellipsis, and the summary and the status badge stay whole

#### Scenario: Asked to open
- **WHEN** a closed tool row that opens is pressed
- **THEN** the owner is asked to open it, and nothing is drawn below the line until the owner does

#### Scenario: A row that doesn't open
- **WHEN** the owner gives no open state
- **THEN** the row shows no chevron, and a press on it does nothing

## ADDED Requirements

### Requirement: Diff blocks
A diff block SHALL draw the file change it is given: a header with the operation's word, the path, the earlier path for a move or copy, and a small copy button that asks the owner to copy; then the lines it is given, with a border line between hunks. It SHALL take no focus and do no diffing of its own.

#### Scenario: Two hunks
- **WHEN** a diff block is given two hunks
- **THEN** a border line separates them

#### Scenario: Copy asked of the owner
- **WHEN** the copy button is clicked
- **THEN** the owner is asked to copy, and the block changes nothing itself

### Requirement: Diff block lines
Each line of a diff block SHALL show a sign column, the old and the new line number, and its text in the code family the owner names. Added lines SHALL be on the `diff_added` colour and removed lines on the `diff_removed` colour, and the sign column SHALL mark them too, so colour is never the only cue.

#### Scenario: Signs as well as colours
- **WHEN** a diff block draws one added and one removed line
- **THEN** the added line shows `+` on the added colour and the removed line shows `−` on the removed colour

#### Scenario: Line numbers
- **WHEN** a diff block draws a hunk starting at old line 10 and new line 12
- **THEN** its first context line shows 10 and 12
