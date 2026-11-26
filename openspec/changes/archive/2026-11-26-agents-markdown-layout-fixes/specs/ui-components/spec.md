## MODIFIED Requirements

### Requirement: Messages
A message SHALL show one turn's text in a bubble, aligned to the end for the person and to the start for the other side, the person's in the muted colours and the other's on the card. The other side's bubble SHALL fill the width of its row. The person's bubble SHALL be as wide as its text, up to four fifths of the row. A message drawn in parts SHALL give every part the same width, so its parts form one bubble with straight edges. A message SHALL keep its text's line breaks and wrap long lines, and SHALL be able to carry actions shown while the pointer is over it.

#### Scenario: Two sides
- **WHEN** a person's message and an agent's message are shown
- **THEN** the person's bubble sits at the end in the muted colours, and the agent's at the start

#### Scenario: The other side fills the row
- **WHEN** an agent's message holding only the word `ok` is shown in a row 600 pixels wide
- **THEN** its bubble is as wide as the row

#### Scenario: The person's bubble fits its text
- **WHEN** a person's message holding only the word `ok` is shown in a row 600 pixels wide
- **THEN** its bubble is narrower than half the row and sits at the row's end

#### Scenario: Parts of different lengths
- **WHEN** an agent's message is drawn in two parts, the first the word `short` and the second a paragraph wider than the row
- **THEN** both parts are as wide as the row, the paragraph wraps inside it, and the two parts' left and right edges line up

### Requirement: Response code blocks
A response's code block SHALL draw its text in the owner's code font on the muted surface inside a border, keeping line breaks and spaces, with its label (when it has one) and a copy button above the code. The label SHALL stay on one line: it SHALL NOT wrap, and when it is wider than the room beside the copy button it SHALL be cut with an ellipsis. The copy button SHALL be pointer-only, taking no keyboard focus, and SHALL give the owner the block's text; the response SHALL NOT write the clipboard itself.

#### Scenario: Copy button
- **WHEN** the user presses a code block's copy button
- **THEN** the owner is given that block's text, and keyboard focus doesn't move

#### Scenario: A short label inside a message
- **WHEN** a code block labelled `rust` is drawn inside an agent's message in a row 600 pixels wide
- **THEN** the label `rust` shows on one line, as wide as its text, above the code

#### Scenario: A label too long for its row
- **WHEN** a code block's label is wider than the room left beside its copy button
- **THEN** the label is one line cut with an ellipsis, and the copy button stays whole

## ADDED Requirements

### Requirement: Text wraps at its container's width
A component's text that may wrap SHALL wrap only at the width its container gives it, whatever the text measured before in the same frame and whatever width the container had in the frame before. A one-line label SHALL stay on one line, giving way with an ellipsis rather than wrapping.

#### Scenario: Reason line in a narrow input
- **WHEN** a prompt input 240 pixels wide shows the reason `connecting to the terminal server`
- **THEN** the reason wraps at the input's width, its first line holding at least the word `connecting`, and no line holds a single letter

#### Scenario: After the width changes
- **WHEN** a prompt input showing a reason is drawn 600 pixels wide and then, in the next frame, 240 pixels wide
- **THEN** the reason wraps at 240 pixels as it would have had it been drawn at that width first
