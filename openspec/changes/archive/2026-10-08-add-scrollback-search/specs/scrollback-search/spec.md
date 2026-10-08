# Spec Delta

## Purpose

Let users find text in a terminal's scrollback and screen from a find bar, in local tabs and in tmux control-mode panes alike, with matches highlighted and the view following the current match.

## ADDED Requirements

### Requirement: Opening the find bar
ctrl-shift-f, and "Find" in the command palette, SHALL open a find bar over the focused terminal and give it keyboard focus. If the bar is already open, they SHALL focus it. The bar SHALL start with the last query searched in that terminal, if any.

#### Scenario: Open from the keyboard
- **WHEN** a local shell tab has focus and the user presses ctrl-shift-f
- **THEN** a find bar appears at the top right of the terminal and typed text goes into it, not to the shell

#### Scenario: Open from the palette
- **WHEN** the user runs "Find" from the command palette
- **THEN** the find bar opens on the tab's focused terminal, as with ctrl-shift-f

### Requirement: Search scope
The find bar SHALL search one terminal: its scrollback and its screen. In a tmux control-mode tab, the bar SHALL belong to the focused pane and search only the output Brindle has received for that pane. Other panes, tabs and tmux's own history SHALL NOT be searched or highlighted.

#### Scenario: Control-mode pane
- **WHEN** two panes of a control-mode tab both show `needle`, and the user searches for `needle` in the left pane
- **THEN** only the left pane's `needle` is highlighted

### Requirement: Incremental search
The search SHALL rerun on every edit of the query. The first match SHALL be the nearest one at or above the bottom of the view when the bar opened. An empty query SHALL match nothing. Matches SHALL NOT span a line break unless the line was wrapped.

#### Scenario: Typing narrows the search
- **WHEN** the scrollback holds `apple` and `apricot` and the user types `apr`
- **THEN** only `apricot` is highlighted after the third letter

#### Scenario: Wrapped line
- **WHEN** a long line wraps so that `needle` is split across the wrap and the user searches for `needle`
- **THEN** the match is found and highlighted across both rows

### Requirement: Match highlighting
While the bar is open, every match in view SHALL be highlighted with the theme's search match colors. The current match SHALL use the theme's focused match colors instead, so it stands out from the others.

#### Scenario: Current match stands out
- **WHEN** three matches are on screen and the second from the bottom is current
- **THEN** that match is drawn in the focused match colors and the other two in the match colors

### Requirement: Current match is scrolled into view
When the current match changes and is not fully visible, the view SHALL scroll so that the match is near the middle of the terminal, as far as the scrollback allows.

#### Scenario: Match in history
- **WHEN** the only match is 500 lines up in the scrollback
- **THEN** the view scrolls up and the match is shown near the middle of the terminal

### Requirement: Next and previous
Enter in the bar, and the `find_next` action, SHALL make the next match up (older output) current. Shift-enter, and `find_previous`, SHALL make the next match down current. Moving past the oldest or newest match SHALL wrap around to the other end. Both actions SHALL also work with focus in the terminal while its bar is open.

#### Scenario: Step and wrap
- **WHEN** there are matches on lines 10, 20 and 30, line 30's is current, and the user presses enter three times
- **THEN** the current match moves to line 20, then 10, then back to 30

#### Scenario: Previous
- **WHEN** line 20's match is current and the user presses shift-enter
- **THEN** line 30's match becomes current

### Requirement: Match count
The bar SHALL show the current match's number and the total number of matches, numbered from the newest match up, or that there are none. Counting SHALL stop at 1000 matches, and the bar SHALL then show that there are more.

#### Scenario: Count shown
- **WHEN** the scrollback holds 11 matches and the newest is current
- **THEN** the bar shows `1/11`, and after enter it shows `2/11`

#### Scenario: No match
- **WHEN** the query matches nothing
- **THEN** the bar says there are no results and nothing is highlighted

### Requirement: Case and regex options
The bar SHALL offer a match-case option (alt-c, `find_toggle_case`) and a regular-expression option (alt-r, `find_toggle_regex`), both off when the bar first opens in a terminal. With match case off, letters SHALL match regardless of case. With regex off, the query SHALL match literally. Changing an option SHALL rerun the search.

#### Scenario: Match case
- **WHEN** the scrollback holds `Error` and `error` and the user searches `error` with match case on
- **THEN** only `error` matches, and with match case off both do

#### Scenario: Literal by default
- **WHEN** the user searches `a.c` with regex off
- **THEN** `a.c` matches but `abc` does not, and with regex on both match

### Requirement: Invalid regular expression
A query that is not a valid regular expression, while regex is on, SHALL be reported in the bar as invalid and SHALL match nothing. The search SHALL resume as soon as the query is valid again.

#### Scenario: Unclosed group
- **WHEN** regex is on and the user types `foo(`
- **THEN** the bar reports an invalid expression and nothing is highlighted

### Requirement: Editing the query
Typing SHALL append to the query and backspace SHALL delete its last character. ctrl-shift-v and shift-insert SHALL paste into the query, with line breaks replaced by spaces, and SHALL NOT paste into the terminal. A query restored when the bar opens SHALL be shown selected, so that typing replaces it.

#### Scenario: Paste goes to the bar
- **WHEN** the bar has focus and the user presses ctrl-shift-v
- **THEN** the clipboard text is added to the query and nothing is sent to the program

#### Scenario: Restored query is replaced
- **WHEN** the user reopens the bar after searching `foo` and types `b`
- **THEN** the query is `b`

### Requirement: Closing the find bar
Escape in the bar, the bar's close button, and `find_close` SHALL close the bar, remove every highlight, put the view back at the scroll position it had when the bar opened, and return focus to the terminal. Clicking the terminal SHALL move focus to it and leave the bar open with its highlights.

#### Scenario: Escape restores the view
- **WHEN** the view was at the bottom, the search scrolled it 500 lines up, and the user presses escape
- **THEN** the bar closes, the highlights disappear and the view is back at the bottom

#### Scenario: Click into the terminal
- **WHEN** the bar is open and the user clicks in the terminal and types `ls`
- **THEN** `ls` goes to the program and the bar stays open
