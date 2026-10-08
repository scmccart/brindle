# Spec Delta

## MODIFIED Requirements

### Requirement: Scrollback
Each terminal SHALL keep `scrollback_lines` lines of history. The user SHALL be able to scroll by line, by page, and to the top or bottom, and to clear the scrollback. ctrl-shift-up and ctrl-shift-down SHALL scroll by one line in a terminal with no shell-integration prompt marks. In a terminal with marks, they jump between prompts as defined by the `shell-integration` capability. Line scrolling stays available as the bindable `scroll_line_up` and `scroll_line_down` actions.

#### Scenario: Page up
- **WHEN** the user presses shift-pageup
- **THEN** the view moves up one page of history

#### Scenario: Line scroll without shell integration
- **WHEN** the shell emits no prompt marks and the user presses ctrl-shift-up
- **THEN** the view moves up one line of history
