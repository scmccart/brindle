## ADDED Requirements

### Requirement: A tab follows its active zone
Where a tab takes something from a profile, it SHALL take it from its active zone's profile: the profile name its title falls back to, and the theme its face on the strip is drawn in. Both SHALL follow as the active zone changes. A tab SHALL have no profile, kind or host of its own.

#### Scenario: The face follows focus
- **WHEN** a tab holds a zone themed `gruvbox-dark` and a zone themed `brindle-light`, and focus moves from the first to the second
- **THEN** the tab's face is drawn in `gruvbox-dark` and then in `brindle-light`

#### Scenario: The title's fallback
- **WHEN** a tab's active zone is an agent zone of profile `Claude` with no conversation title, and its other zone is a shell of profile `Shell`
- **THEN** the tab's title is `Claude`

#### Scenario: One theme
- **WHEN** every zone of a tab is of one profile and focus moves between them
- **THEN** the tab's face doesn't change
