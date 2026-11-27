## MODIFIED Requirements

### Requirement: Markdown in agent messages and thoughts
Agent messages and thoughts SHALL be rendered as CommonMark with GitHub's strikethrough, task lists, tables and autolinks: paragraphs, headings, emphasis, inline code, code blocks, lists, quotes, rules and links. Footnotes, math and smart punctuation SHALL NOT be interpreted; their text shows as written. A line break inside a paragraph SHALL read as a space, as CommonMark says. A list of any length SHALL read as one list: its items evenly spaced from its first to its last, and a numbered list's numbers going on without a break.

#### Scenario: Emphasis and a heading
- **WHEN** the agent's message is `## Plan` then a blank line and `Run **cargo test** and *read* the ~~old~~ log`
- **THEN** the message shows a level-two heading `Plan` and a paragraph with `cargo test` bold, `read` italic and `old` struck through, with no `#`, `*` or `~` shown

#### Scenario: Task list
- **WHEN** the agent's message is `- [x] build` and `- [ ] test` on two lines
- **THEN** the message shows a list of two items, `build` marked done and `test` marked open

#### Scenario: Footnote stays text
- **WHEN** the agent's message is `See [^1]` then a blank line and `[^1]: the note`
- **THEN** both lines show as written, as two paragraphs

#### Scenario: A long numbered list
- **WHEN** the agent's message is a numbered list of 120 items, `1. Step 1` to `120. Step 120`
- **THEN** the message shows one list numbered 1 to 120, with the same space between items 50 and 51 as between any other two

### Requirement: Tables in the conversation
A GitHub table in an agent message or thought SHALL be drawn as a grid: one column per header cell, the header row set apart from the body, and each column's text aligned as its delimiter row says. A cell's text SHALL wrap within its column. A table of any length SHALL read as one grid: its header once at its top, its columns the same width from its first row to its last, one frame round it, and a line between every two body rows.

#### Scenario: Aligned table
- **WHEN** the agent's message holds a table with header `| name | size |`, delimiter row `| :--- | ---: |` and two rows
- **THEN** the message shows a two-column grid with a set-apart header, names aligned to the start and sizes to the end, and no pipes

#### Scenario: A long table
- **WHEN** the agent's message holds a three-column table with 1,000 body rows
- **THEN** the message shows one grid with its header once, above the first row, every column as wide at row 1,000 as at row 1, and no gap, border or rounded corner between rows 50 and 51 or any others
