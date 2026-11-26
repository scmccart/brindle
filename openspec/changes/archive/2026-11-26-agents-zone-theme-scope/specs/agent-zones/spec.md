## MODIFIED Requirements

### Requirement: Agent zone appearance
An agent zone SHALL be drawn as a card in its profile's theme, as a terminal zone is, and everything inside it (its messages, code blocks, tables, thoughts, tool, permission and login rows, alerts, the prompt input and its queue row, every button, and its header's buttons) SHALL take its colours from that theme's zone token set. Its focus ring, status dot, progress bar and context badge SHALL keep the window's colours. Its text SHALL be in the interface font and its code in the terminal's font family, at the profile's font size, else the terminal font size in effect. Its zone header, when the tab has two or more zones, SHALL show its status dot, its title and the split and close buttons.

#### Scenario: Profile font size
- **WHEN** an agent profile sets `font_size = 18`
- **THEN** its zone's messages are drawn at 18 points and other tabs keep their size

#### Scenario: Code in the terminal's font
- **WHEN** the config's terminal font family is `JetBrains Mono` and an agent message holds a code block and inline code
- **THEN** both are drawn in `JetBrains Mono`, and the message's other text in the interface font

#### Scenario: A profile theme unlike the window's
- **WHEN** an agent profile sets a dark theme while the global theme is light, and its zone shows a user message, an agent message with a code block, a pending permission and the prompt input
- **THEN** the bubbles, the code block, the permission row and its buttons, and the prompt input are drawn in the dark theme's zone tokens, and the tab strip in the light theme's tokens

#### Scenario: No profile theme
- **WHEN** an agent profile sets no theme
- **THEN** its zone is drawn in the global theme's zone token set, its card's surface and its bubbles' background being mixes of the terminal background the zone shows

#### Scenario: Tooltips and the header menu
- **WHEN** an agent zone with a dark profile theme in a light window shows the tooltip of a message's copy button, or opens its header menu
- **THEN** the tooltip and the menu are drawn in the dark theme's zone tokens, at the window's text size

#### Scenario: Window dialogs keep the window's colours
- **WHEN** closing a busy agent zone with a dark profile theme in a light window opens the close confirmation
- **THEN** the confirmation is drawn in the light window's tokens

#### Scenario: The ring and the status dot
- **WHEN** an agent zone with its own theme is the active zone of a tab of two zones, with the ring shown and a working status
- **THEN** its ring is the window's ring colour and its status dot the global theme's colour for working
