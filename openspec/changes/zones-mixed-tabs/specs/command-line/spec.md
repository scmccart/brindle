## ADDED Requirements

### Requirement: Zone kinds and profiles in the dump
Each `--- zone` line of `--dump-screen-after` SHALL say, after the zone's rectangle, its kind (`kind=terminal` or `kind=agent`) and its profile's name (`profile="NAME"`, escaped onto the line), and `host="…"` when the zone runs on a remote host, before `active`. The palette that asks which profile a split runs SHALL print as any palette list does and be driven by the same list actions.

#### Scenario: A mixed tab's zone lines
- **WHEN** a tab holds a shell of profile `Shell` and a focused agent zone of profile `Claude`, and the dump is taken
- **THEN** it has a line `--- zone id=1 rect=… kind=terminal profile="Shell"` and a line `--- zone id=2 rect=… kind=agent profile="Claude" active`

#### Scenario: A remote zone
- **WHEN** one zone of the active tab runs on the SSH host `A8`
- **THEN** its zone line holds `host=` naming `A8`, and the local zones' lines hold no `host=`

#### Scenario: Driving the profile question
- **WHEN** a run uses `--action split_agent_right` with two agent profiles, then `--action list_down` and `--action list_confirm`
- **THEN** the dump shows a new agent zone of the second agent profile
