## ADDED Requirements

### Requirement: An agent message is one bubble
An agent message SHALL be drawn as one bubble across the conversation's width, however many Markdown blocks it holds and however long each block's lines are. Its left and right edges SHALL be straight from its first block to its last. A user message SHALL be as wide as its text, up to four fifths of the conversation's width.

#### Scenario: A short paragraph above a long one
- **WHEN** the agent's message is `Hi.`, a blank line, and a paragraph longer than the zone is wide
- **THEN** the message's bubble is one rectangle as wide as the conversation, with `Hi.` at its top and the long paragraph wrapped inside it

#### Scenario: A code block in a message
- **WHEN** the agent's message is a sentence followed by a fenced code block labelled `rust`
- **THEN** the sentence and the code block are in one bubble with straight edges, and the label `rust` shows on one line above the code

#### Scenario: A short user message
- **WHEN** the user sends `hi`
- **THEN** the user's bubble is as wide as `hi` and its padding, at the conversation's end

### Requirement: The prompt's reason line fits its zone
An agent zone's line under the prompt input (why the prompt is disabled, such as `connecting to the terminal server`) SHALL wrap at the zone's width, in a zone of any width the tab allows and from the first frame after a split.

#### Scenario: Connecting after a split
- **WHEN** an agent zone is split and the new agent zone shows `connecting to the terminal server` under its prompt input
- **THEN** the text reads across the zone on as few lines as the zone's width allows, not one letter or one word per line
