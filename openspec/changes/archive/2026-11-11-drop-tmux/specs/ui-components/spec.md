## MODIFIED Requirements

### Requirement: Menu item descriptions
A menu item MAY carry a description, drawn in the muted foreground on a second line under its label. Typeahead and the item's accessible name SHALL use the label only.

#### Scenario: Description shown
- **WHEN** a menu item has the label `Shell` and the description `default shell`
- **THEN** the item shows both lines, and typing `s` highlights it
