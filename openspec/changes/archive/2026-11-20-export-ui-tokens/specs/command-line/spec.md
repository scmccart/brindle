## MODIFIED Requirements

### Requirement: Informational options
`--help`, `--version`, `--list-actions` (every bindable action name), `--list-themes` (the built-in themes), `--export-ui-tokens` (the built-in themes' UI tokens as CSS) and `--shell-integration <bash|zsh|fish|pwsh>` (that shell's integration script) SHALL print their output and exit without opening a window. `--shell-integration` with a missing or unsupported shell SHALL print an error naming the supported shells and exit with status 2.

`--export-ui-tokens` SHALL print, for each built-in theme, a CSS rule setting shadcn/ui's custom properties (`--background`, `--foreground`, `--card`, `--card-foreground`, `--popover`, `--popover-foreground`, `--primary`, `--primary-foreground`, `--secondary`, `--secondary-foreground`, `--muted`, `--muted-foreground`, `--accent`, `--accent-foreground`, `--destructive`, `--destructive-foreground`, `--border`, `--input`, `--ring`, `--radius`) to the tokens Brindle's chrome draws with in that theme, and the theme's terminal colours (`--terminal-background`, `--terminal-foreground`, `--terminal-cursor`, `--terminal-selection`, `--terminal-ansi-0` to `--terminal-ansi-15`). Each rule SHALL match `[data-theme="NAME"]`; the `brindle-light` rule SHALL also match `:root` and the `brindle-dark` rule `.dark`, and the `.dark` rule SHALL come after the `:root` rule.

#### Scenario: List actions
- **WHEN** the user runs `brindle --list-actions`
- **THEN** each bindable action name is printed, including `activate_tab_<N>` and `new_tab_with_profile_<N>`

#### Scenario: Print a shell script
- **WHEN** the user runs `brindle --shell-integration zsh`
- **THEN** the zsh integration script is printed and the process exits 0 without opening a window

#### Scenario: Print the PowerShell script
- **WHEN** the user runs `brindle --shell-integration pwsh`
- **THEN** the PowerShell integration script is printed and the process exits 0 without opening a window

#### Scenario: Export UI tokens
- **WHEN** the user runs `brindle --export-ui-tokens`
- **THEN** a CSS rule per built-in theme is printed, `brindle-dark`'s setting `--primary` to its accent `#e6a756`, and the process exits 0 without opening a window
