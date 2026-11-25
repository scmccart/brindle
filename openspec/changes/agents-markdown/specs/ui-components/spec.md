## ADDED Requirements

### Requirement: Responses
A response SHALL draw a list of blocks the owner gives as plain data: paragraphs and headings of styled runs (strong, emphasis, strikethrough, inline code, muted, link), code blocks, bulleted, numbered and task lists (which may nest blocks), quotes, tables and rules. It SHALL interpret no markup of its own, and SHALL take the code font family from its owner.

#### Scenario: Styled runs
- **WHEN** the owner gives a paragraph whose runs are `plain`, `bold` (strong) and `code` (inline code)
- **THEN** `bold` is drawn in a bold weight and `code` in the owner's code font on the muted surface, all on one line

#### Scenario: Nested list
- **WHEN** the owner gives a numbered list starting at 3 whose second item holds a bulleted list
- **THEN** the items are numbered 3 and 4, and the bulleted list is indented under item 4

### Requirement: Response code blocks
A response's code block SHALL draw its text in the owner's code font on the muted surface inside a border, keeping line breaks and spaces, with its label (when it has one) and a copy button above the code. The copy button SHALL be pointer-only, taking no keyboard focus, and SHALL give the owner the block's text; the response SHALL NOT write the clipboard itself.

#### Scenario: Copy button
- **WHEN** the user presses a code block's copy button
- **THEN** the owner is given that block's text, and keyboard focus doesn't move

### Requirement: Response links
A response SHALL draw each link run in the link colour, underlined. A press released over the same link SHALL be reported to the owner with the link and the modifiers held; the response SHALL show a pointing hand over a link only while the modifiers the owner names are held, and a tooltip with the owner's text for a link that has one.

#### Scenario: Press on a link
- **WHEN** the user presses and releases on a link run with ctrl held
- **THEN** the owner is told which link, with ctrl among the modifiers

#### Scenario: Release elsewhere
- **WHEN** the user presses on a link, moves off it and releases
- **THEN** nothing is reported

### Requirement: Response tables
A response's table SHALL be drawn as a grid of equal-width columns, one per header cell, with the header row set apart by the border colour and each column aligned as the owner says. A cell's text SHALL wrap within its column, and a row with fewer cells than the header SHALL leave the rest empty.

#### Scenario: Short row
- **WHEN** the owner gives a three-column table whose second row has two cells
- **THEN** that row's third cell is empty and the grid keeps three columns
