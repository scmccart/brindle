# Spec Delta

## ADDED Requirements

### Requirement: Zone header context badge
While a local tab has two or more zones, each zone header SHALL show its zone's notable-context badge after the title, drawn as on the tab face (a yellow warning icon with the target user in its tooltip for `elevate` and `chpriv`, the muted label otherwise).

#### Scenario: Two zones, one elevated
- **WHEN** a tab has two zones and one of them runs `run0`
- **THEN** that zone's header shows the yellow warning icon, and the other zone's header shows none
