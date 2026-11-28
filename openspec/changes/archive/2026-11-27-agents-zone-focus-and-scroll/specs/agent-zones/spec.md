# Spec Delta

## ADDED Requirements

### Requirement: A pointer press keeps focus in the zone
A pointer press of any button in an agent zone on anything that doesn't take focus itself (message text, a thought's label, a tool call row, a message's copy button, the "Earlier messages" item, the jump-to-end button, the space between them) SHALL put keyboard focus in the zone's prompt input. A press on a control that takes focus (the prompt input, a button of a permission row, the login row, the queue row, the stop button) SHALL give focus to that control.

#### Scenario: A press on a message
- **WHEN** the prompt input has focus and the user presses on the text of an agent message
- **THEN** the prompt input still has focus, and typed text goes into it

#### Scenario: A press on a thought's label
- **WHEN** the prompt input has focus and the user presses a closed thought's label
- **THEN** the thought opens and the prompt input still has focus

#### Scenario: A press on a tool call row
- **WHEN** the prompt input has focus and the user presses the line of a tool call row that opens
- **THEN** the row opens and the prompt input still has focus

#### Scenario: A press on a message's copy button
- **WHEN** the prompt input has focus and the user presses an agent message's copy button
- **THEN** the clipboard holds the message's Markdown source, and the prompt input still has focus

#### Scenario: From a permission button to the text
- **WHEN** a permission row's `Reject` button has focus and the user presses on a message's text
- **THEN** the prompt input has focus

#### Scenario: Escape after a press
- **WHEN** a turn is running, the user presses on a message's text, then presses escape
- **THEN** the turn ends as cancelled

#### Scenario: A press in an inactive zone
- **WHEN** a tab holds a terminal zone with focus and an agent zone, and the user presses on a message's text in the agent zone
- **THEN** the agent zone is the active zone and its prompt input has focus

#### Scenario: A press on a permission button
- **WHEN** the prompt input has focus and the user presses a pending permission row's `Reject` button
- **THEN** the agent receives `Reject`

### Requirement: A focused permission button is in view
When keyboard focus moves with tab or shift-tab onto a button of a pending permission row that isn't wholly in view, the conversation SHALL scroll the least distance that shows the row, its top when the row is taller than the view. When a permission button has focus and the user scrolls its row wholly out of view, focus SHALL go back to the prompt input.

#### Scenario: Tab to a request below the view
- **WHEN** the user has scrolled up, a permission request arrives below the view, and the user presses tab in the prompt input
- **THEN** the conversation scrolls to show the request's row with its `Reject` button focused, and enter answers `Reject`

#### Scenario: A request already in view
- **WHEN** a pending permission row is wholly in view and the user tabs onto its button
- **THEN** the conversation doesn't scroll

#### Scenario: Scrolled away from a focused button
- **WHEN** a permission row's button has focus and the user scrolls the row out of view with the wheel
- **THEN** the prompt input has focus, and the request stays pending

## MODIFIED Requirements

### Requirement: Scrolling the conversation
The conversation SHALL scroll, and SHALL stay at its end while new text arrives as long as the user is at the end. Scrolling away from the end SHALL keep the view where it is while text arrives or earlier entries are added above, and show a button that jumps to the end; reaching the end again SHALL make it follow again. The wheel, shift-pageup, shift-pagedown and the `agent_scroll_to_bottom` action SHALL scroll it.

#### Scenario: Follows the reply
- **WHEN** the conversation is at its end and the agent streams a reply longer than the zone
- **THEN** the end of the reply stays in view

#### Scenario: Reading back
- **WHEN** the user scrolls up while the agent streams a reply
- **THEN** the lines in view stay put, a jump-to-end button shows, and pressing it shows the end and follows again

#### Scenario: Earlier entries arrive above
- **WHEN** the user has scrolled up to the top of the entries the zone holds, and earlier entries are loaded above them
- **THEN** the lines in view stay put, and the earlier entries are above them

#### Scenario: Reading a code block while it streams
- **WHEN** the agent streams a code block longer than the zone, and the user scrolls up into its middle while it still grows
- **THEN** the lines of the code block in view stay put as more lines arrive below them

#### Scenario: Reading a thought while it streams
- **WHEN** the user has scrolled the middle of a streaming thought to the top of the view
- **THEN** the lines of the thought in view stay put as it grows

#### Scenario: Reading a command's output while it runs
- **WHEN** the user has scrolled the middle of an open tool call row's command output to the top of the view while the command runs
- **THEN** the lines of the output in view stay put as output arrives

#### Scenario: A config reload while reading back
- **WHEN** the user has scrolled up and the config is reloaded, or the settings dialog previews a theme
- **THEN** the lines in view stay put

#### Scenario: A zoom while reading back
- **WHEN** the user has scrolled up and zooms in or out
- **THEN** the entry that was at the top of the view is still at its top, at the same place in its text, and nothing jumps as other entries scroll into view

### Requirement: Earlier messages
When an agent session has entries before the oldest one its zone holds, the conversation SHALL start with an "Earlier messages" item, which takes no focus. When it scrolls into view, the zone SHALL fetch the entries before its oldest one, showing a spinner on the item until they arrive, then add them above. Fetched entries SHALL stay in the zone until it closes or its session is replayed again. When nothing earlier remains, the item SHALL go. A fetch SHALL belong to the connection it was asked on: a fetch not yet answered when the link drops SHALL fail, and SHALL NOT be sent on the next link or change what the zone shows after the reconnect. The spinner SHALL show only while a fetch is on its way.

#### Scenario: Scrolling to the top loads more
- **WHEN** an agent zone shows the newest page of a long conversation and the user scrolls to its top
- **THEN** the "Earlier messages" item shows a spinner, then the earlier entries appear above the ones the user was reading

#### Scenario: From the keyboard
- **WHEN** the prompt input has focus and the user presses shift-pageup until the zone's oldest entry is in view, then once more
- **THEN** the "Earlier messages" item comes into view and the earlier entries are fetched, and focus stays in the prompt input

#### Scenario: Everything loaded
- **WHEN** the user keeps scrolling up until the zone holds the session's whole log
- **THEN** the "Earlier messages" item is gone and the conversation starts with the first kept entry

#### Scenario: A short conversation
- **WHEN** an agent zone is opened on a session whose whole log fits in one page
- **THEN** the conversation shows no "Earlier messages" item

#### Scenario: A fetch that fails
- **WHEN** fetching earlier entries is refused, or the link to the endpoint goes down while it waits
- **THEN** the item says the earlier messages couldn't be loaded and why, with nothing else in the conversation changed, and the fetch is tried again the next time the item scrolls into view or is pressed

#### Scenario: A fetch not yet sent when the link drops
- **WHEN** a zone that holds two fetched pages asks for a third, and the link drops before the request is written, then comes back
- **THEN** the fetch fails, the zone shows the newest page again with the "Earlier messages" item, the session is still shown and not reported broken, and scrolling up fetches the earlier entries again

#### Scenario: A new session while a fetch is on its way
- **WHEN** an agent zone's agent has exited, a fetch of earlier entries is on its way, and the user continues the conversation in a new session that fails to start
- **THEN** the "Earlier messages" item shows no spinner, and the zone says why the new session didn't start

#### Scenario: Reattached
- **WHEN** a zone that had fetched earlier entries is restored after Brindle restarts
- **THEN** it shows the newest page again with the "Earlier messages" item, and scrolling up fetches the earlier entries again

#### Scenario: Titled from the first page
- **WHEN** a zone is restored on a long conversation the agent titled near its start
- **THEN** the zone's title and its tab's title are the conversation's title before any earlier entry is fetched

#### Scenario: A page that arrives in a hidden tab
- **WHEN** an agent zone showing the "Earlier messages" item has fetched a page, and the user switches to another tab before it arrives
- **THEN** the page is added and no further page is fetched until the zone is shown again and the item is still in view
