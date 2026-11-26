## ADDED Requirements

### Requirement: Confirmation action descriptions
A confirmation's action MAY carry a description, shown as plain text under its button, and MAY be disabled. When any action has a description, the actions SHALL stack in the owner's order, each with its description under it. A disabled action SHALL be drawn muted, SHALL NOT be a keyboard stop and SHALL ask the owner for nothing.

#### Scenario: Action descriptions
- **WHEN** a confirmation is requested with actions `Log in with Google`, `API Key` described `Use an API key`, and a disabled `Log in from the terminal`
- **THEN** the three buttons show one under another in that order, `Use an API key` shows under `API Key`, tab from `Log in with Google` reaches `API Key`, and tab from `API Key` leaves the confirmation
