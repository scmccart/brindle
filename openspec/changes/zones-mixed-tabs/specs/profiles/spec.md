## MODIFIED Requirements

### Requirement: Per-profile appearance
A profile's theme and font size SHALL apply to its zones only, and SHALL update when the config is reloaded.

#### Scenario: Themed profile
- **WHEN** a profile sets `theme = "tokyo-night"`
- **THEN** its tabs use Tokyo Night colors and other tabs keep the default theme

#### Scenario: Themed profile in a mixed tab
- **WHEN** a tab holds a zone of a profile that sets `theme = "tokyo-night"` beside a zone of a profile that sets none
- **THEN** the first zone uses Tokyo Night colors and the second keeps the default theme

### Requirement: Renamed profiles keep their tabs
When a profile is renamed from the settings dialog, every open zone of that profile, in every tab and window, SHALL follow the new name. Later changes to the profile SHALL reach those zones on reload. A rename made by editing the config file directly SHALL leave such zones on their previous settings until they are closed.

#### Scenario: Edit after rename
- **WHEN** a tab uses profile `Work`, the user renames it to `Job` in the dialog, and then sets `Job`'s theme to `tokyo-night` and saves
- **THEN** the tab shows `tokyo-night`

#### Scenario: Rename with a mixed tab open
- **WHEN** a tab holds a zone of `Work` and a zone of `Shell`, and the user renames `Work` to `Job` in the dialog
- **THEN** the first zone follows `Job`, and the second stays `Shell`
