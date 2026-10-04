# Spec Delta

## ADDED Requirements

### Requirement: Pane title lines
When a window's `pane-border-status` is `top` or `bottom`, each pane SHALL show its title line in the row tmux reserves for it, above or below the pane. The text SHALL be the pane's `pane-border-format` as tmux expands it, starting two columns in from the pane's left edge and clipped to the pane's width. Foreground and background colors, bold and reverse from the format's style directives SHALL be applied, and other directives SHALL NOT appear as text. Title lines SHALL update when tmux reports a change.

#### Scenario: Teammate pane names
- **WHEN** a pane's title is set with `select-pane -T @researcher` and its `pane-border-format` is `#[fg=blue,bold] #{pane_title} #[default]`, with `pane-border-status top`
- **THEN** the row above that pane shows ` @researcher ` in bold blue

#### Scenario: Default format
- **WHEN** `pane-border-status` is `bottom` and `pane-border-format` is tmux's default
- **THEN** each pane shows its index and quoted title in the row below it, and the active pane's index is shown in reverse video

#### Scenario: Title changes
- **WHEN** a program renames its pane with `select-pane -T` while the tab is open
- **THEN** the title line shows the new name within about a second

#### Scenario: Border status off
- **WHEN** `pane-border-status` is `off`
- **THEN** no title lines are drawn
