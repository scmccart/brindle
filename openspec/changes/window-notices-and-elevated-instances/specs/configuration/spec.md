# Spec Delta

## ADDED Requirements

### Requirement: Transient window notices
Besides config errors and findings, the window's config line SHALL show transient notices, such as a link Brindle didn't open. Each SHALL go away 10 seconds after it was shown, or when the user dismisses it with the config line's dismiss button or the `dismiss_notices` action, whichever comes first. Dismissing SHALL leave config errors and findings shown.

#### Scenario: Notice expires
- **WHEN** a link refusal is shown on the config line and the user does nothing
- **THEN** 10 seconds later the config line no longer shows it

#### Scenario: Dismiss keeps config errors
- **WHEN** the config names an unknown theme and a link refusal is shown, and the user runs `dismiss_notices`
- **THEN** the config line still names the unknown theme, and no longer shows the refusal

#### Scenario: Repeated and many notices
- **WHEN** the user ctrl-clicks the same refused link twice, then three other refused links
- **THEN** the first link's refusal shows once, and after the fourth different refusal only the three latest show
