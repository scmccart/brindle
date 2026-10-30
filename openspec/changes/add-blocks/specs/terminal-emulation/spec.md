# Spec Delta

## MODIFIED Requirements

### Requirement: Wheel scrolling
The mouse wheel SHALL:
- send wheel reports when the program has enabled mouse reporting;
- otherwise, in the alternate screen with alternate-scroll mode (1007) on, send arrow keys;
- otherwise scroll a block or the scrollback, as defined by the `blocks` capability when blocks are showing, and the scrollback when they are not.

#### Scenario: Scrolling a pager
- **WHEN** `less` is running (alternate screen, no mouse reporting) and the user scrolls up
- **THEN** `less` receives Up arrow sequences and scrolls its content

#### Scenario: Scrolling history
- **WHEN** a shell prompt is showing and the user scrolls up
- **THEN** earlier output from the scrollback becomes visible

#### Scenario: Scrolling inside a block
- **WHEN** the pointer is over a block taller than the cap, the wheel has been idle, and the user scrolls up
- **THEN** that block's output scrolls and the view does not move
