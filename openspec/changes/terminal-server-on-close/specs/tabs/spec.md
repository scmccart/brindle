## MODIFIED Requirements

### Requirement: Window closes with its last tab
Closing the last tab of a window SHALL close the window. Closing the last window SHALL quit the application. Closing a window, by its close button, the window manager or `Close Window`, SHALL close its tabs as closing each tab would, with a single prompt for the window when any of its terminals asks (see the `zones` capability's "Asking before closing").

#### Scenario: Last tab closed
- **WHEN** a window's only tab is closed
- **THEN** the window closes

#### Scenario: Window close asks once
- **WHEN** a window has two tabs with `on_close = "ask"` and the user clicks its close button
- **THEN** one prompt is shown before the window closes, and cancelling it keeps the window open

## ADDED Requirements

### Requirement: Quitting keeps or ends programs
Quitting Brindle SHALL leave every open local Linux terminal's program running detached, except `-e` command terminals, which end. When any terminal's behaviour is `ask`, quitting SHALL first show one prompt covering every asking terminal in every window: keep them running, end them, or cancel the quit. When Brindle ends without asking (a crash, a signal, the session manager), every terminal but `-e` command terminals SHALL keep running.

#### Scenario: Quit with detach
- **WHEN** two windows hold local tabs with the default `on_close` and the user quits
- **THEN** Brindle exits without asking, and every tab's session is listed as detached

#### Scenario: Quit asks once
- **WHEN** three tabs in two windows have `on_close = "ask"` and the user quits
- **THEN** one prompt is shown, and picking "end" ends all three programs before Brindle exits

#### Scenario: Crash with ask
- **WHEN** Brindle is killed with SIGKILL while a tab with `on_close = "ask"` runs `sleep 1000`
- **THEN** the `sleep` keeps running, detached
