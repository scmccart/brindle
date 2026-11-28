## MODIFIED Requirements

### Requirement: Markdown while it streams
As an agent message or thought streams, it SHALL be rendered again from its whole text as the text arrives, so it always shows what its text so far means. A code fence not yet closed SHALL show as a code block running to the end of the text. The blocks of a message that the new text doesn't change SHALL stay where they are. Rendering a streaming message SHALL NOT hold up the output of other tabs and zones on the same endpoint.

#### Scenario: Unclosed fence
- **WHEN** the agent has streamed a fence opened with three backticks and `rust` and two lines of code, and not yet the closing fence
- **THEN** the message shows those two lines as a code block labelled `rust`, and the paragraph above it unchanged

#### Scenario: A definition that comes later
- **WHEN** the agent streams `See [the docs][d].`, a blank line, and then `[d]: https://example.com`
- **THEN** once the definition has arrived, `the docs` shows as a link to `https://example.com`, and the definition line isn't shown

#### Scenario: A shell beside a streaming reply
- **WHEN** an agent zone streams a long reply in small pieces and the user types in a local shell zone in the same window
- **THEN** each key's echo shows in the shell as promptly as it does beside an idle agent zone, give or take the time one frame takes to draw
