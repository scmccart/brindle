## REMOVED Requirements

### Requirement: Restore is opt-in
**Reason**: Terminals now outlive Brindle, and reattaching them without the recorded layout flattens split tabs and merges windows. Restore is always on.
**Migration**: Remove `restore_session` from the config; it is ignored with a warning. Use `--no-restore` to skip restore for one launch.

## ADDED Requirements

### Requirement: Restore is always on
Brindle SHALL always record its windows and tabs and restore them on a plain launch, as defined by this capability. No config setting SHALL turn this off; `--no-restore` (see `command-line`) skips it for one launch, and `BRINDLE_STATE_DIR` points it elsewhere.

#### Scenario: Default config
- **WHEN** the config is empty and the user quits Brindle with three tabs, two of them split
- **THEN** the next plain launch opens one window with the same three tabs and splits, each zone showing its program as it was

#### Scenario: Skipped once
- **WHEN** the user quits with three tabs and runs `brindle --no-restore`
- **THEN** one window opens with a single default-profile tab, and the three tabs' sessions stay detached

## MODIFIED Requirements

### Requirement: Local tabs restore their profile and directory
A restored local tab SHALL restore its recorded zone split tree, with each zone running the tab's recorded profile in that zone's recorded working directory, and focus the tab's recorded active zone. Each zone's recorded directory SHALL be the working directory of that zone's foreground process, the same directory a new tab or split would inherit. A recorded local tab without a split tree SHALL restore as a single zone in the tab's recorded working directory. Classic tmux tabs SHALL rerun their profile, which reattaches to their session. When a recorded local tab has a manual title, that title SHALL be restored with the tab. A zone whose recorded terminal server session is still running detached SHALL be attached to that session instead of starting a new program (see "Restored zones get their sessions back").

#### Scenario: Directory comes back
- **WHEN** a tab's shell was in `/srv/www` when Brindle quit, and its session was ended with `brindle-remote kill` before the relaunch
- **THEN** after a plain launch that tab's new shell starts in `/srv/www`

#### Scenario: Split zone tree comes back
- **WHEN** a local tab was split right, with the left zone in `/srv/api` and the focused right zone in `/srv/web`, when Brindle quit
- **THEN** after a plain launch the tab opens with two side-by-side zones in `/srv/api` and `/srv/web`, and the right zone is focused

#### Scenario: Classic tmux tab
- **WHEN** a classic tmux tab for session `main` was open when Brindle quit
- **THEN** after a plain launch a tab runs `tmux new-session -A -s main` and shows that session

#### Scenario: Manual title comes back
- **WHEN** a local tab has manual title `server` when Brindle quits
- **THEN** after a plain launch that tab is restored with title `server`

## ADDED Requirements

### Requirement: Restored zones get their sessions back
Each recorded local Linux zone SHALL record which terminal server session it showed. On a restoring launch, a zone whose recorded session is still running and detached SHALL be attached to it, showing its screen and scrollback, without bells or notifications from the replayed output. A zone whose session is gone, attached elsewhere, or not the recorded one (the server was restarted in between and reused its id) SHALL start a new program in its recorded directory.

#### Scenario: Programs survive a quit
- **WHEN** a tab with the default `on_close` runs `htop`, and the user quits and relaunches
- **THEN** the restored tab shows the same `htop` process, with the same pid

#### Scenario: Programs survive a crash
- **WHEN** Brindle is killed with SIGKILL while a split tab's zones run `vim` and a shell with scrollback
- **THEN** the next plain launch restores the split with `vim` running and the shell's scrollback intact

#### Scenario: Session gone
- **WHEN** a recorded zone's program was ended with `brindle-remote kill` before the relaunch
- **THEN** that zone starts a new shell in its recorded directory, and the other zones are reattached

#### Scenario: Reused id
- **WHEN** the server stopped after Brindle quit, and a new server gave the recorded id to a different session
- **THEN** the restored zone starts a new shell and leaves that other session alone

### Requirement: Launch reattaches detached terminals
On every launch except `-e` and `--no-restore`, Brindle SHALL attach every detached local terminal server session it doesn't restore into a zone, each as a new tab in the first window, in the order the sessions were started, with the profile its session was started with (the default profile when that profile is gone). The active tab SHALL stay the one the launch would otherwise make active. Replayed output SHALL raise no bells or notifications.

#### Scenario: Closed tab comes back
- **WHEN** a window has tabs A and B, the user closes tab B while it runs `sleep 1000`, quits, and relaunches
- **THEN** the restored window has tab A, still active, followed by a tab showing that `sleep`

#### Scenario: No bell from the replay
- **WHEN** a detached session's output contained a bell before the launch
- **THEN** reattaching it marks no bell on its tab and sends no notification

### Requirement: Only the first Brindle reattaches
Only the Brindle process that owns the session state (see "One process owns the state") SHALL reattach detached sessions at launch. A second Brindle running at the same time SHALL NOT attach them.

#### Scenario: Second Brindle
- **WHEN** one Brindle is running and a detached session exists, and the user launches a second Brindle
- **THEN** the second Brindle's window has only its own tab, and the session stays detached
