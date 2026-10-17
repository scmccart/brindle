# Spec Delta

## MODIFIED Requirements

### Requirement: Clicking a notification shows its tab
When the notification service supports actions, clicking a notification (its default action) SHALL raise the window it came from and activate the tab it came from. Windows toasts always support this. If that tab has closed, the window SHALL still be raised. If the window has closed, nothing SHALL happen.

#### Scenario: Click
- **WHEN** a notification from the third tab of a background window is clicked
- **THEN** that window comes to the front with the third tab active

#### Scenario: Click a toast on Windows
- **WHEN** on Windows, a toast from the second tab of a minimized Brindle window is clicked
- **THEN** that window is restored and comes to the front with the second tab active

### Requirement: Delivery never blocks the window
Notifications SHALL be sent without blocking the window: on Linux to the desktop's freedesktop notification service, and on Windows as toast notifications shown under Brindle's name and icon. If notifications can't be delivered, because there is no notification service or because the system or the user has turned Brindle's notifications off, they SHALL be dropped with a logged warning and the terminal SHALL keep working.

#### Scenario: No notification service
- **WHEN** Brindle runs in a session with no notification service and a background tab rings the bell
- **THEN** the tab shows its bell dot, a warning is logged, and Brindle stays responsive

#### Scenario: Toast on Windows
- **WHEN** on Windows a background tab titled `make` prints `ESC ] 9 ; done BEL`
- **THEN** a toast titled `make` with the body `done` appears, attributed to Brindle with Brindle's icon

#### Scenario: Toasts turned off on Windows
- **WHEN** the user has turned off notifications for Brindle in Windows Settings and a background tab rings the bell
- **THEN** no toast appears, the tab shows its bell dot, and Brindle stays responsive

#### Scenario: Toasts without the installer
- **WHEN** Brindle was installed with `cargo install`, with no Start Menu shortcut, and a background tab sends an OSC 9 notification
- **THEN** a toast attributed to Brindle appears
