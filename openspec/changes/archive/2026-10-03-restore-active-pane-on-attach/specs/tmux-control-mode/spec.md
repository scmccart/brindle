# Spec Delta

## ADDED Requirements

### Requirement: Attaching keeps tmux's selection
Attaching to a session SHALL NOT change tmux's current window, its last window, or any window's active pane. The tab for tmux's current window SHALL become active, with keyboard focus in the pane tmux reports as active in that window. Each other tab SHALL show its window's active pane as active, so that commands aimed at the active pane act on it.

#### Scenario: Reattach to a pane that isn't first
- **WHEN** a window has three panes, the second is active, and the user attaches
- **THEN** keyboard focus is in the second pane, and tmux still reports the second pane as active

#### Scenario: Current and last window are kept
- **WHEN** a session has windows 0, 1 and 2, window 2 is current, window 0 is the last window, and the user attaches
- **THEN** the tab for window 2 is active, and tmux still reports window 2 as current and window 0 as the last window

#### Scenario: Commands act on tmux's active pane after attaching
- **WHEN** the user attaches to a window whose active pane is not the first, and splits right
- **THEN** the pane tmux had active is the one that is split

### Requirement: New windows come to the front only when tmux makes them current
A tab for a window that appears while attached SHALL become the active tab only when tmux makes that window its current window. Adding the tab SHALL NOT select the window in tmux.

#### Scenario: Window created in the foreground
- **WHEN** the user opens a new tmux window with ctrl-shift-t
- **THEN** its tab appears and becomes active

#### Scenario: Window created in the background
- **WHEN** another client runs `new-window -d` in the attached session
- **THEN** a tab for the new window appears without becoming active, and tmux's current window is unchanged
