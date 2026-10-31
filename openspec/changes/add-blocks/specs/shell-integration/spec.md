# Spec Delta

## MODIFIED Requirements

### Requirement: Failed command mark
When `mark_failed_commands` is on (the default), a terminal SHALL draw a thin vertical bar in the theme's red (ANSI color 1) at the left edge of the grid, beside the rows from a command's prompt start to its command start, for every visible command that finished with a non-zero exit status. The bar SHALL NOT cover text in a padded terminal, and it SHALL scroll with its prompt. While `blocks` is on, the block's rail SHALL show the failure instead, as defined by the `blocks` capability. No failed-command mark SHALL be drawn while the alternate screen is active.

#### Scenario: Failure marked
- **WHEN** the user runs `false` and then `true`
- **THEN** a red bar appears beside the `false` prompt and none beside the `true` prompt

#### Scenario: Turned off
- **WHEN** `mark_failed_commands = false`
- **THEN** no bars are drawn

#### Scenario: Rail with blocks on
- **WHEN** `blocks = true` and the user runs `false`
- **THEN** the block's rail is red for its full height and no separate bar is drawn

#### Scenario: Full-screen program over a failed command
- **WHEN** the user runs `false` and then opens `vim`
- **THEN** no red bar is drawn over vim
