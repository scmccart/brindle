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

### Requirement: Replays raise nothing
Attaching an agent session, at launch or from the sessions palette, SHALL show the newest page of its conversation, with an "Earlier messages" item above it when older entries exist, and its queued or held prompt, and SHALL NOT raise notifications for what the replay holds or for earlier entries fetched afterwards.

#### Scenario: Restored after a finished turn
- **WHEN** an agent finished a turn while Brindle wasn't running, and Brindle is launched
- **THEN** the restored zone shows the whole turn and its done status, and no notification appears

#### Scenario: Earlier entries raise nothing
- **WHEN** the user scrolls up a restored zone and the fetched entries hold a permission request answered long ago and turns that ended
- **THEN** no notification appears, and the zone's status is unchanged

### Requirement: Motion only while an agent works
Spinners and shimmering text SHALL advance in steps, at most 10 a second, from one clock for the whole window. The clock SHALL run only while some agent zone of the window is working or loading earlier messages, and SHALL NOT wake or redraw the window when none is.

#### Scenario: Stops at the end of the turn
- **WHEN** the only working agent zone's turn ends
- **THEN** the window's motion clock stops, and nothing in the window moves until an agent works again

#### Scenario: One clock for many zones
- **WHEN** two agent zones in different tabs are working
- **THEN** the window redraws for motion at most 10 times a second, not once per zone

#### Scenario: Loading earlier messages
- **WHEN** an idle agent zone's "Earlier messages" item is loading, and the page then arrives
- **THEN** the item's spinner turns while it loads, and the window's motion clock stops once the page is shown

## ADDED Requirements

### Requirement: Earlier messages
When an agent session has entries before the oldest one its zone holds, the conversation SHALL start with an "Earlier messages" item, which takes no focus. When it scrolls into view, the zone SHALL fetch the entries before its oldest one, showing a spinner on the item until they arrive, then add them above. Fetched entries SHALL stay in the zone until it closes or its session is replayed again. When nothing earlier remains, the item SHALL go.

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

#### Scenario: Reattached
- **WHEN** a zone that had fetched earlier entries is restored after Brindle restarts
- **THEN** it shows the newest page again with the "Earlier messages" item, and scrolling up fetches the earlier entries again

#### Scenario: Titled from the first page
- **WHEN** a zone is restored on a long conversation the agent titled near its start
- **THEN** the zone's title and its tab's title are the conversation's title before any earlier entry is fetched
