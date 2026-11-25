## MODIFIED Requirements

### Requirement: The conversation
An agent zone SHALL show its session's entries in order: user messages and notices as plain text, agent messages and thoughts rendered as Markdown, tool calls and permission requests. Text streamed into an entry SHALL appear in that entry as it arrives. Control characters SHALL NOT be interpreted.

#### Scenario: A turn
- **WHEN** the user sends `hi` and the agent streams `Hel`, `lo` and `!` and ends the turn
- **THEN** the conversation shows a user message `hi` followed by one agent message `Hello!`

#### Scenario: Multi-line text
- **WHEN** the agent's message holds three paragraphs separated by blank lines, the second holding a tab
- **THEN** the message shows three paragraphs, with the tab as whitespace

#### Scenario: The user's text stays as typed
- **WHEN** the user sends `**not bold**`
- **THEN** the user's message shows `**not bold**` with its asterisks

### Requirement: Copying from the conversation
The `copy_last_agent_message` action SHALL copy the latest agent message's Markdown source to the clipboard. Each agent message SHALL show a copy button while the pointer is over it, which copies that message's Markdown source. Paste SHALL insert into the prompt input.

#### Scenario: Copy the reply
- **WHEN** the agent's last message is `Hello!` and the user runs `Copy Last Agent Message` from the command palette
- **THEN** the clipboard holds `Hello!`

#### Scenario: Copy keeps the Markdown
- **WHEN** the agent's last message is `Run **cargo test** first` and the user runs `Copy Last Agent Message`
- **THEN** the clipboard holds `Run **cargo test** first`, asterisks included

### Requirement: Agent zone appearance
An agent zone SHALL be drawn as a card in its profile's theme, as a terminal zone is. Its text SHALL be in the interface font and its code in the terminal's font family, at the profile's font size, else the terminal font size in effect. Its zone header, when the tab has two or more zones, SHALL show its status dot, its title and the split and close buttons.

#### Scenario: Profile font size
- **WHEN** an agent profile sets `font_size = 18`
- **THEN** its zone's messages are drawn at 18 points and other tabs keep their size

#### Scenario: Code in the terminal's font
- **WHEN** the config's terminal font family is `JetBrains Mono` and an agent message holds a code block and inline code
- **THEN** both are drawn in `JetBrains Mono`, and the message's other text in the interface font

### Requirement: Agent commands in the palette
While an agent zone has focus, the command palette SHALL list Stop Agent Turn, Remove Queued Prompt, Send Held Prompt Again, Copy Queued Prompt, Copy Last Agent Message, Copy Last Code Block, Toggle Agent Thoughts and Scroll Agent to Bottom, with the zone and tab commands, and SHALL NOT list commands that act on a terminal, such as Clear Scrollback or Find.

#### Scenario: Palette in an agent zone
- **WHEN** an agent zone has focus and the user opens the command palette
- **THEN** it lists "Stop Agent Turn", "Copy Last Code Block" and "Split Zone Right", and not "Clear Scrollback"

## ADDED Requirements

### Requirement: Markdown in agent messages and thoughts
Agent messages and thoughts SHALL be rendered as CommonMark with GitHub's strikethrough, task lists, tables and autolinks: paragraphs, headings, emphasis, inline code, code blocks, lists, quotes, rules and links. Footnotes, math and smart punctuation SHALL NOT be interpreted; their text shows as written. A line break inside a paragraph SHALL read as a space, as CommonMark says.

#### Scenario: Emphasis and a heading
- **WHEN** the agent's message is `## Plan` then a blank line and `Run **cargo test** and *read* the ~~old~~ log`
- **THEN** the message shows a level-two heading `Plan` and a paragraph with `cargo test` bold, `read` italic and `old` struck through, with no `#`, `*` or `~` shown

#### Scenario: Task list
- **WHEN** the agent's message is `- [x] build` and `- [ ] test` on two lines
- **THEN** the message shows a list of two items, `build` marked done and `test` marked open

#### Scenario: Footnote stays text
- **WHEN** the agent's message is `See [^1]` then a blank line and `[^1]: the note`
- **THEN** both lines show as written, as two paragraphs

### Requirement: Code blocks in the conversation
A fenced or indented code block in an agent message or thought SHALL be drawn in the terminal's font family on the muted surface, keeping its line breaks and indentation, with the first word of its fence's info string as a label. It SHALL have a copy button, reached with the pointer only, that copies the block's content exactly, without its fences. Code SHALL NOT be syntax highlighted.

#### Scenario: Fenced code
- **WHEN** the agent's message holds a fence opened with three backticks and `rust`, the line `fn main() {}`, and the closing fence
- **THEN** the message shows `fn main() {}` in the terminal's font family under the label `rust`, and no backticks

#### Scenario: Copying a code block
- **WHEN** the user presses the copy button of a code block holding two indented lines
- **THEN** the clipboard holds those two lines exactly, indentation and final line break included, and no fence

### Requirement: Copy Last Code Block
The `copy_last_code_block` action, listed in the command palette as "Copy Last Code Block" with no default key, SHALL copy the content of the last code block of the latest agent message that has one, exactly as the code block's copy button does. When no agent message has a code block, the clipboard SHALL be left unchanged.

#### Scenario: From the palette
- **WHEN** the agent's last message holds two code blocks, `a` and then `b`, and the user runs Copy Last Code Block
- **THEN** the clipboard holds `b` and its line break

#### Scenario: No code
- **WHEN** no agent message in the zone holds a code block and the user runs Copy Last Code Block
- **THEN** the clipboard is unchanged

### Requirement: Markdown while it streams
As an agent message or thought streams, it SHALL be rendered again from its whole text as the text arrives, so it always shows what its text so far means. A code fence not yet closed SHALL show as a code block running to the end of the text. The blocks of a message that the new text doesn't change SHALL stay where they are.

#### Scenario: Unclosed fence
- **WHEN** the agent has streamed a fence opened with three backticks and `rust` and two lines of code, and not yet the closing fence
- **THEN** the message shows those two lines as a code block labelled `rust`, and the paragraph above it unchanged

#### Scenario: A definition that comes later
- **WHEN** the agent streams `See [the docs][d].`, a blank line, and then `[d]: https://example.com`
- **THEN** once the definition has arrived, `the docs` shows as a link to `https://example.com`, and the definition line isn't shown

### Requirement: Tables in the conversation
A GitHub table in an agent message or thought SHALL be drawn as a grid: one column per header cell, the header row set apart from the body, and each column's text aligned as its delimiter row says. A cell's text SHALL wrap within its column.

#### Scenario: Aligned table
- **WHEN** the agent's message holds a table with header `| name | size |`, delimiter row `| :--- | ---: |` and two rows
- **THEN** the message shows a two-column grid with a set-apart header, names aligned to the start and sizes to the end, and no pipes

### Requirement: Images in the conversation are never fetched
An image in an agent message or thought SHALL NOT be fetched or drawn. It SHALL show as `[image: ALT]` in the muted colour, followed by its target (its first 120 characters and an ellipsis when longer), and SHALL act as a link to its whole target.

#### Scenario: Remote image
- **WHEN** the agent's message holds `![chart](https://example.com/c.png)`
- **THEN** the message shows `[image: chart]` and `https://example.com/c.png` muted, nothing is requested from `example.com`, and the text acts as a link to `https://example.com/c.png`

### Requirement: Raw HTML shows as text
HTML in an agent message or thought, as a block or inline, SHALL be shown as its literal text and SHALL NOT be interpreted.

#### Scenario: Inline HTML
- **WHEN** the agent's message is `a <b>bold</b> claim <script>x()</script>`
- **THEN** the message shows that text with its tags as written, and nothing is bold or run
