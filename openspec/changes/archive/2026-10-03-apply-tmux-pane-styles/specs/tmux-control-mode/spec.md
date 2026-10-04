# Spec Delta

## ADDED Requirements

### Requirement: Pane default colors follow tmux pane styles
A pane's default foreground and background SHALL follow the colors in its `window-style`, or in its `window-active-style` while it is the active pane, when those set a color other than `default`. For the active pane, a color that `window-active-style` leaves unset or `default` SHALL come from `window-style`, and a color neither sets SHALL be the theme's. Colors that a program sets explicitly SHALL NOT be changed. Programs that query the default colors (OSC 10/11) SHALL receive the colors the pane is shown with.

#### Scenario: Tinted teammate pane
- **WHEN** one pane of a window has `window-style fg=blue` and its program prints plain text
- **THEN** that text is blue, and text in the window's other panes keeps the theme foreground

#### Scenario: Active pane style
- **WHEN** a window has `window-active-style bg=#202020` and the user switches the active pane
- **THEN** the newly active pane's default background becomes `#202020`, and the previously active pane returns to its `window-style` or the theme background

#### Scenario: Explicit colors are kept
- **WHEN** a pane has `window-style fg=blue` and its program prints red text with SGR 31
- **THEN** the text is red

#### Scenario: Color queries see the style
- **WHEN** a pane has `window-style bg=#102030` and a program in it queries OSC 11
- **THEN** the reply carries `#102030`

### Requirement: Dividers follow tmux border styles
Dividers SHALL use the foreground color of the window's `pane-border-style`, and the active pane's highlight SHALL use the foreground of `pane-active-border-style`, when those set a color and differ from tmux's built-in defaults. Otherwise the theme's divider and accent colors SHALL be used. Title lines SHALL use the same colors as their base style.

#### Scenario: Colored borders
- **WHEN** a window has `pane-border-style fg=magenta` and `pane-active-border-style fg=yellow`
- **THEN** its dividers are drawn in the theme's magenta, and the active pane's highlight in the theme's yellow

#### Scenario: Default border styles
- **WHEN** both border styles have tmux's built-in values, for example the active border's default green
- **THEN** dividers and the highlight use the theme's colors as before
