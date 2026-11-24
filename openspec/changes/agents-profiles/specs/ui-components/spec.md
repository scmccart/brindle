# Spec Delta

## ADDED Requirements

### Requirement: Textareas
A textarea SHALL frame a text area, as defined by `text-input`, as an input frames a text input: the input colour as its border, the ring when focused, the destructive colour when marked invalid, and a press anywhere in the frame focusing the text area.

#### Scenario: Press in the frame
- **WHEN** the user presses in a textarea's padding below its text
- **THEN** the text area has focus, with the cursor at the end of its text

### Requirement: Message scroller
A message scroller SHALL show a list of items of different heights, scrolled to its end when it first shows them. While it is at its end it SHALL stay there as items are added or grow; once scrolled away it SHALL keep the items in view where they are and show a scroll button, which scrolls to the end and follows again. It SHALL lay out only the items in view and near it.

#### Scenario: Growing last item
- **WHEN** a message scroller is at its end and its last item grows by ten lines
- **THEN** the end of that item is in view

#### Scenario: Scrolled away
- **WHEN** the user scrolls a message scroller up and an item is added at the end
- **THEN** the items in view don't move, and the scroll button shows

### Requirement: Messages
A message SHALL show one turn's text in a bubble, aligned to the end for the person and to the start for the other side, the person's in the muted colours and the other's on the card. A message SHALL keep its text's line breaks and wrap long lines, and SHALL be able to carry actions shown while the pointer is over it.

#### Scenario: Two sides
- **WHEN** a person's message and an agent's message are shown
- **THEN** the person's bubble sits at the end in the muted colours, and the agent's at the start

### Requirement: Prompt input
A prompt input SHALL hold a textarea and a toolbar with a submit button. The owner SHALL give it a status: ready (an arrow; disabled while the text is blank), submitted (a spinner, disabled), streaming (a stop square) or disabled. A press on the submit button, or enter in the textarea, SHALL ask the owner to submit when ready; in streaming, the button SHALL ask the owner to stop.

#### Scenario: Stop while streaming
- **WHEN** a prompt input's status is streaming and the user clicks its button
- **THEN** the owner is asked to stop, not to submit

#### Scenario: Blank text
- **WHEN** a prompt input is ready and its textarea is blank
- **THEN** its submit button is disabled, and enter asks nothing

### Requirement: Prompt input queue row
Given a queued text, a prompt input SHALL show it in a row above the textarea: as waiting, with a remove button, or as not sent, with the owner's reason and send-again and discard buttons. Each button SHALL be a keyboard stop and SHALL ask the owner; the row SHALL never change the textarea's text.

#### Scenario: Queued row
- **WHEN** the owner gives a prompt input queued text `next`
- **THEN** a row above the textarea shows `next` with a remove button, and pressing the button asks the owner to remove it

#### Scenario: Held row
- **WHEN** the owner gives a prompt input the text `next` as not sent, with the reason `the turn was stopped`
- **THEN** the row shows `next`, `not sent`, the reason, and send-again and discard buttons, each asking the owner, and the textarea's text is unchanged

### Requirement: Reasoning blocks
A reasoning block SHALL show a trigger with a label and a chevron over collapsible content. The owner SHALL hold whether it is open; a press on the trigger SHALL ask the owner to flip it. While closed, only the trigger SHALL show.

#### Scenario: Open on press
- **WHEN** a closed reasoning block's trigger is pressed
- **THEN** the owner is asked to open it, and once it does the content shows below the trigger

### Requirement: Tool rows
A tool row SHALL be one line: an icon the owner chooses, a title cut with an ellipsis, and a status badge reading pending, running, done or failed, the failed one in the destructive colour. It SHALL take no focus.

#### Scenario: Long title
- **WHEN** a tool row's title is wider than the row
- **THEN** the title is cut with an ellipsis, and the status badge stays whole

### Requirement: Confirmations
A confirmation SHALL show a title and, while requested, one small button per action the owner gives, each a keyboard stop activated by enter or space or a primary click. Once the owner marks it accepted, rejected or cancelled, it SHALL show that outcome with the chosen action's label and no buttons.

#### Scenario: Requested
- **WHEN** a confirmation is requested with actions `Allow once` and `Reject`
- **THEN** both buttons show, and pressing space on the focused `Reject` button asks the owner for `Reject`

#### Scenario: Answered
- **WHEN** the owner marks a confirmation accepted with `Allow once`
- **THEN** it shows `Allow once` as the outcome and no buttons

### Requirement: Alerts
An alert SHALL be a bordered block with an icon, a title and a description, in the default colours or, for the destructive variant, with its title and icon in the destructive colour. It SHALL take no focus.

#### Scenario: Destructive alert
- **WHEN** an alert of the destructive variant is shown
- **THEN** its title and icon are drawn in the destructive colour
