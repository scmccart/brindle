# Spec Delta

## ADDED Requirements

### Requirement: Tab context badge
A tab face SHALL show, after its title, a badge with the label of its active zone's notable context (see `program-contexts`) when it has one. The badge for `elevate` and `chpriv` SHALL be drawn in the theme's red (ANSI 1), and others in the face's muted foreground. The badge SHALL give way before the title is shortened below 8 characters, and SHALL NOT change the face's height.

#### Scenario: Elevated zone
- **WHEN** the active zone of a tab runs `run0`, which reports an `elevate` context with `targetuser=root`
- **THEN** that tab's face shows a red `root` badge after its title, until `run0` ends the context

#### Scenario: Background zone
- **WHEN** a tab's inactive zone is inside a `container` context and its active zone has no contexts
- **THEN** the tab face shows no badge
