# Spec Delta

## ADDED Requirements

### Requirement: Color queries in panes get the theme's colors
A program in a control-mode pane that queries the default foreground (OSC 10) or background (OSC 11) color SHALL receive the foreground or background of the theme the pane is drawn with. This SHALL hold for panes present at attach and for panes created later. After a configuration reload, later queries SHALL receive the reloaded theme's colors.

#### Scenario: Background query after attach
- **WHEN** the user attaches with the `brindle-dark` theme, and a program in a pane sends `ESC ] 11 ; ? BEL`
- **THEN** the reply carries `brindle-dark`'s background color, not black

#### Scenario: Foreground query in a new pane
- **WHEN** the user splits a pane, and a program in the new pane sends `ESC ] 10 ; ? BEL`
- **THEN** the reply carries the theme's foreground color

#### Scenario: Theme change takes effect
- **WHEN** the user changes the profile's theme in the configuration file, the configuration reloads, and a program in an existing pane then queries OSC 11
- **THEN** the reply carries the new theme's background color

#### Scenario: tmux without color reports
- **WHEN** the tmux in use does not support reporting colors for a pane
- **THEN** attaching and using panes works as before, and color queries get whatever tmux answers on its own
