# Spec Delta

## ADDED Requirements

### Requirement: Palette queries in panes get the theme's palette
When a program in a control-mode pane queries a palette color (OSC 4, indexes 0–255) and tmux does not answer it, Brindle SHALL reply with the theme's color for the index, on any tmux version. When tmux answers the query itself, Brindle SHALL NOT add a reply. tmux answers when the pane's palette has the index (a program set it, or the `pane-colours` option does) or when a terminal client is attached to the pane's session.

#### Scenario: ANSI color after attach
- **WHEN** the user attaches, and a program in a pane sends `ESC ] 4 ; 4 ; ? BEL`
- **THEN** it receives one reply, carrying the theme's blue

#### Scenario: Index beyond the ANSI colors
- **WHEN** a program in a pane queries index 196
- **THEN** it receives one reply, carrying the standard 256-color palette's `rgb:ffff/0000/0000`

#### Scenario: New pane
- **WHEN** the user splits a pane, and a program in the new pane queries index 1
- **THEN** it receives one reply, carrying the theme's red

#### Scenario: Theme change takes effect
- **WHEN** the user changes the profile's theme in the configuration file, the configuration reloads, and a program in an existing pane then queries index 4
- **THEN** the reply carries the new theme's blue

#### Scenario: Color a program set
- **WHEN** a program in a pane sets index 5 with `ESC ] 4 ; 5 ; rgb:ff/00/00 BEL` and then queries index 5
- **THEN** it receives one reply, carrying `rgb:ffff/0000/0000`

#### Scenario: Color from `pane-colours`
- **WHEN** the pane's `pane-colours[1]` is `#445566` and a program in it queries index 1
- **THEN** it receives one reply, carrying `rgb:4444/5555/6666`

#### Scenario: Terminal client attached
- **WHEN** a terminal client is attached to the same session as Brindle, and a program in a pane queries index 4
- **THEN** Brindle sends no reply, and the program gets only the reply tmux relays from that client's terminal, if any
