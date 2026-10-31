# Spec Delta

## MODIFIED Requirements

### Requirement: Local tabs restore their profile and directory
A restored local tab SHALL restore its recorded zone split tree, with each zone running the tab's recorded profile in that zone's recorded working directory, and focus the tab's recorded active zone. Each zone's recorded directory SHALL be the working directory of that zone's foreground process, the same directory a new tab or split would inherit. A recorded local tab without a split tree SHALL restore as a single zone in the tab's recorded working directory. Classic tmux tabs SHALL rerun their profile, which reattaches to their session. When a recorded local tab has a manual title, that title SHALL be restored with the tab.

#### Scenario: Directory comes back
- **WHEN** a tab's shell was in `/srv/www` when Brindle quit
- **THEN** after a plain launch that tab's shell starts in `/srv/www`

#### Scenario: Split zone tree comes back
- **WHEN** a local tab was split right, with the left zone in `/srv/api` and the focused right zone in `/srv/web`, when Brindle quit
- **THEN** after a plain launch the tab opens with two side-by-side zones in `/srv/api` and `/srv/web`, and the right zone is focused

#### Scenario: Classic tmux tab
- **WHEN** a classic tmux tab for session `main` was open when Brindle quit
- **THEN** after a plain launch a tab runs `tmux new-session -A -s main` and shows that session

#### Scenario: Manual title comes back
- **WHEN** a local tab has manual title `server` when Brindle quits
- **THEN** after a plain launch that tab is restored with title `server`

### Requirement: Restoring falls back when things have changed
Restoring SHALL succeed even when the recorded state no longer matches the system:
- a missing profile SHALL be replaced by the default profile, with a log line;
- a missing directory for a tab or zone SHALL be replaced by the profile's own directory, else the home directory;
- a missing control-mode session SHALL be created again by the attach, as opening the profile does. If tmux can't attach, the attach-failure tab of `tmux-control-mode` SHALL be shown.

#### Scenario: Profile removed from the config
- **WHEN** a recorded tab's profile `Work` has been removed from the config
- **THEN** the tab is restored with the default profile in its recorded directory

#### Scenario: Directory deleted
- **WHEN** a recorded tab's or zone's directory has been deleted since Brindle quit
- **THEN** that terminal is restored in the home directory (for a profile without its own directory)

#### Scenario: Session killed
- **WHEN** a recorded control-mode session was killed after Brindle quit
- **THEN** after a plain launch a new session with the same name is created and attached
