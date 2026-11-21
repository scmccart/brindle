# Spec Delta

## ADDED Requirements

### Requirement: Zone header progress bar
While a local tab has two or more zones, each zone whose terminal has progress SHALL show it as a 2-pixel bar along the bottom edge of its header bar, drawn as the tab's bar is for that one terminal (length by percent, colour by state, indeterminate across the whole width without animation).

#### Scenario: Two builds side by side
- **WHEN** a tab has two zones, one reporting normal at 30% and the other paused at 70%
- **THEN** the first zone's header shows a muted bar 30% of its width long, the second a yellow bar 70% long, and the tab shows the yellow one
