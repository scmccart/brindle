# Spec Delta

## ADDED Requirements

### Requirement: Tab context badge
A tab face SHALL show, after its title, a badge for its active zone's notable context (see `program-contexts`) when it has one. The badge for `elevate` and `chpriv` SHALL be a warning icon alone, without the label, drawn in the face's theme's yellow (ANSI 3) made legible on the face's background (at least 3:1), with a tooltip naming the target user ("Running as root"). Other badges SHALL show the label in the face's muted foreground, and SHALL give way before the title is shortened below 8 characters. A badge SHALL NOT change the face's height. A context SHALL NOT change the terminal's colours or the tab's title: what a program such as `run0` prints for itself (a tinted background, a glyph in its title) is shown as printed.

#### Scenario: Elevated zone
- **WHEN** the active zone of a tab runs `run0`, which reports an `elevate` context with `targetuser=root`
- **THEN** that tab's face shows a yellow warning icon after its title, and no `root` text, until `run0` ends the context
- **AND** resting the pointer on the icon shows "Running as root"

#### Scenario: Background zone
- **WHEN** a tab's inactive zone is inside a `container` context and its active zone has no contexts
- **THEN** the tab face shows no badge
