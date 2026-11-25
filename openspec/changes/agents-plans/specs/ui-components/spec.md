## ADDED Requirements

### Requirement: Plan blocks
A plan block SHALL show a trigger line with a chevron, a label, a progress count and a current step cut to one line, over collapsible content the owner gives. The owner SHALL hold whether it is open; a primary press on the trigger SHALL ask the owner to flip it. While closed, only the trigger SHALL show. Open, the content SHALL be no taller than the height the owner gives and SHALL scroll within the block. It SHALL take no focus.

#### Scenario: Open on press
- **WHEN** a closed plan block's trigger is pressed
- **THEN** the owner is asked to open it, and once it does the content shows below the trigger

#### Scenario: A long current step
- **WHEN** the current step's text is wider than the block
- **THEN** it is cut with an ellipsis, and the progress count stays whole

### Requirement: Plan block dismiss mark
When the owner asks for one, a plan block's trigger line SHALL end with a small dismiss mark. A primary press on it SHALL ask the owner to dismiss the block and SHALL NOT flip it open or closed. The mark SHALL take no focus.

#### Scenario: Dismiss doesn't open
- **WHEN** the dismiss mark of a closed plan block is pressed
- **THEN** the owner is asked to dismiss, and isn't asked to open it

#### Scenario: No mark
- **WHEN** the owner asks for no dismiss mark
- **THEN** the trigger line shows none

### Requirement: Plan items
A plan block's item SHALL show a mark for its status (pending, in progress, completed or cancelled, each a different shape), then its content, with the text of a completed or cancelled item muted and a high-priority item marked before its text.

#### Scenario: Four statuses
- **WHEN** a plan block draws a pending, an in-progress, a completed and a cancelled item
- **THEN** the four marks differ in shape, not only in colour

#### Scenario: High priority
- **WHEN** an item is given as high priority
- **THEN** a mark shows before its text, and an item of another priority has none
