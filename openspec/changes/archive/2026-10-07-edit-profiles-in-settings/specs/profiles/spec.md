## ADDED Requirements

### Requirement: Renamed profiles keep their tabs
When a profile is renamed from the settings dialog, every open tab of that profile, in every window, SHALL follow the new name. Later changes to the profile SHALL reach those tabs on reload. A rename made by editing the config file directly SHALL leave such tabs on their previous settings until they are closed.

#### Scenario: Edit after rename
- **WHEN** a tab uses profile `Work`, the user renames it to `Job` in the dialog, and then sets `Job`'s theme to `tokyo-night` and saves
- **THEN** the tab shows `tokyo-night`
