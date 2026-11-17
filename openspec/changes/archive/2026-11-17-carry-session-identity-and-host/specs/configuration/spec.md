## MODIFIED Requirements

### Requirement: Live reload
Saving the config file SHALL reload it automatically, within about a second, and apply the result to open windows:
- theme changes;
- profile theme and font-size changes;
- keybindings;
- fonts, the terminal font (`[font]`) and the UI font (`[ui_font]`).

ctrl-shift-r SHALL force a reload, and ctrl-, SHALL open the config in `$VISUAL` or `$EDITOR` in a new tab. That tab SHALL run on this machine, where the config file is, whatever endpoint the default profile targets.

#### Scenario: Theme change while attached
- **WHEN** the user changes `theme` and saves while tabs are open
- **THEN** all of them redraw with the new theme without being restarted

#### Scenario: UI font change
- **WHEN** the user changes `ui_font.family` and saves
- **THEN** the chrome redraws in the new family without a restart, and the terminals are unchanged

#### Scenario: Default profile on a remote host
- **WHEN** the default profile targets SSH host `A8`, another profile is local, and the user presses ctrl-,
- **THEN** a new tab runs the editor on this machine with this machine's config file, and nothing is started on `A8`
