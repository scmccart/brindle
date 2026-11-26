## MODIFIED Requirements

### Requirement: Agent commands in the palette
While an agent zone has focus, the command palette SHALL list Stop Agent Turn, Remove Queued Prompt, Send Held Prompt Again, Copy Queued Prompt, Copy Last Agent Message, Copy Last Code Block, Toggle Agent Thoughts, Toggle Tool Details and Scroll Agent to Bottom, with the zone and tab commands, and SHALL NOT list commands that act on a terminal, such as Clear Scrollback or Find.

#### Scenario: Palette in an agent zone
- **WHEN** an agent zone has focus and the user opens the command palette
- **THEN** it lists "Stop Agent Turn", "Copy Last Code Block", "Toggle Tool Details", "Split Agent Right" and "Split Terminal Right", and not "Clear Scrollback"
