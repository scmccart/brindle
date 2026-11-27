## MODIFIED Requirements

### Requirement: A launch's first prompt
A tab opened by `brindle agent … -p PROMPT` SHALL send PROMPT as the zone's first prompt once its agent is ready and its session runs, and SHALL send it only once. A zone whose agent is ready before Brindle has finished setting up its session SHALL wait for the session and then send PROMPT; it SHALL NOT refuse PROMPT or leave it as a draft for that reason. When the agent needs a login and offers a method Brindle can run, PROMPT SHALL be shown in the unavailable prompt input and SHALL be sent when the agent first becomes ready after a login in that zone, provided the input still holds exactly PROMPT. When the agent can't start, fails or exits before it is ready, or needs a login and offers no method Brindle can run, PROMPT SHALL be left in the prompt input as a draft, unsent. When the agent is first shown ready by a replay, PROMPT SHALL be left as a draft, unsent; an agent that becomes ready live in the same moment as a replay ends SHALL be sent PROMPT. A prompt not yet sent when Brindle quits SHALL be dropped: it is never recorded, and restoring or attaching the session SHALL NOT send it.

#### Scenario: Sent when ready
- **WHEN** the user runs `brindle agent Agent -p hi` and the agent takes a second to become ready
- **THEN** the conversation shows the user message `hi` once the agent is ready, and the agent receives it once

#### Scenario: Ready before the session is set up
- **WHEN** the user runs `brindle agent Agent -p hi`, and the agent is ready before Brindle has finished setting up the session
- **THEN** the prompt input says the zone is connecting until the session is set up, then the conversation shows the user message `hi`, no reason is shown beside the input, and the agent receives `hi` once

#### Scenario: Not sent again after a restore
- **WHEN** a tab opened with `-p hi` sent its prompt, and the user quits and relaunches
- **THEN** the restored conversation shows `hi` once, and the agent receives no second prompt

#### Scenario: Ready live right after a replay
- **WHEN** a tab opened with `-p hi` reconnects while its agent is starting, and the agent becomes ready live just after the reconnect's replay, before the zone has shown the replay
- **THEN** the agent receives `hi` once, and the input is empty

#### Scenario: Login demanded
- **WHEN** the user runs `brindle agent Agent -p hi` and the agent needs a login and offers no method
- **THEN** the zone says the agent needs a login, and `hi` is in the prompt input, unsent

#### Scenario: Sent after a login
- **WHEN** the user runs `brindle agent Agent -p hi`, the agent needs a login, and the user logs in with one of its methods
- **THEN** `hi` shows in the unavailable prompt input while the login runs, and once the agent is ready the conversation shows the user message `hi`, the input is empty, and the agent receives `hi` once

#### Scenario: Login cancelled, then done
- **WHEN** the user runs `brindle agent Agent -p hi`, cancels a first login, and completes a second
- **THEN** the agent receives `hi` once, after the second login

### Requirement: The prompt input
An agent zone SHALL have a multi-line prompt input under the conversation. Enter SHALL send its text, and shift-enter SHALL add a line. A sent prompt SHALL leave the input empty once the server accepts it; a refused one SHALL stay in the input, with the reason shown beside it. Blank text SHALL NOT be sent, and nothing SHALL be sent before the agent is ready and the zone's session is set up. Enter while the session is still being set up SHALL keep the input's text with no reason shown, and the prompt SHALL be sent by enter once the session is set up.

#### Scenario: Send
- **WHEN** the agent is ready, the user types `hi` and presses enter
- **THEN** the input is empty, and the conversation shows the user message `hi`

#### Scenario: Two lines
- **WHEN** the user types `one`, presses shift-enter, types `two` and presses enter
- **THEN** the conversation shows one user message of two lines, `one` and `two`

#### Scenario: Refused prompt
- **WHEN** the user sends a prompt the server refuses
- **THEN** the text stays in the input, and the reason is shown beside it

#### Scenario: Enter as an attached zone shows ready
- **WHEN** an attached zone's replay shows the agent ready before Brindle has finished setting up the session, and the user types `hi` and presses enter
- **THEN** `hi` stays in the input with no reason shown, the input says the zone is connecting, and once the session is set up enter sends `hi` and the agent receives it once

### Requirement: A disconnected endpoint
While the connection to an agent zone's endpoint is down, the zone SHALL say it is reconnecting and SHALL NOT send prompts, keeping the prompt input's text. A prompt sent just before the connection went down, whose answer was lost with it, SHALL NOT be shown as refused while the zone reconnects. Once the session is shown again, the zone SHALL decide from the session's conversation and queue slot: a prompt the session took SHALL leave the input empty; one it didn't take SHALL stay in the input with the reason shown. When the conversation shown can't tell, the input SHALL keep the prompt and say it may have been sent. When the session can't be got back, the zone SHALL say the session was lost, and stay open until the user closes it; when the zone knows the session's conversation, it SHALL offer to continue it in a new session.

#### Scenario: Reconnected
- **WHEN** the link to an SSH endpoint drops while its agent zone is idle, and comes back
- **THEN** the zone said it was reconnecting, then shows the conversation as before, and a prompt sent afterwards reaches the agent

#### Scenario: A prompt the session took before the link dropped
- **WHEN** the user sends `hi`, the session takes it, and the link drops before its answer arrives, then comes back
- **THEN** no refusal is shown while the zone reconnects, the conversation shows the user message `hi` once, the input is empty, and the agent receives `hi` once

#### Scenario: A prompt queued behind a turn when the link dropped
- **WHEN** a turn runs, the user sends `next`, the session queues it, and the link drops before its answer arrives, then comes back
- **THEN** `next` is shown as queued above the input, and the input is empty

#### Scenario: A prompt the session never got
- **WHEN** the user sends `hi`, the link drops before the session reads it, and the link comes back
- **THEN** `hi` stays in the input with the reason shown beside it, and the conversation has no user message `hi`

#### Scenario: Lost after the agent exited
- **WHEN** an agent zone's agent has exited, and the terminal server's core is killed and replaced before the user continues it
- **THEN** the zone says the session was lost, and still offers to continue the conversation

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

### Requirement: Replays raise nothing
Attaching an agent session, at launch or from the sessions palette, SHALL show the newest page of its conversation, with an "Earlier messages" item above it when older entries exist, and its queued or held prompt, and SHALL NOT raise notifications for what the replay holds or for earlier entries fetched afterwards. Live events that arrive after a replay ends SHALL raise their notifications as any live event does, even when the zone shows the replay and those events at once.

#### Scenario: Restored after a finished turn
- **WHEN** an agent finished a turn while Brindle wasn't running, and Brindle is launched
- **THEN** the restored zone shows the whole turn and its done status, and no notification appears

#### Scenario: Earlier entries raise nothing
- **WHEN** the user scrolls up a restored zone and the fetched entries hold a permission request answered long ago and turns that ended
- **THEN** no notification appears, and the zone's status is unchanged

#### Scenario: A permission asked live right after a reconnect's replay
- **WHEN** an agent zone in a background tab reconnects, and the agent asks for permission live just after the reconnect's replay ends, before the zone has shown the replay
- **THEN** the zone shows the permission request and its blocked status, and the permission's notification appears once

### Requirement: Opening a tool call row
A pointer press on the line of a tool call row that has a detail SHALL open or close it. A row SHALL stay as the user left it while its tool call changes. A detail SHALL add no keyboard stop. Which rows and thoughts the user opened or closed SHALL belong to one session's conversation: a zone that starts a new session (a continue in a new session, New conversation, or a listed conversation taken in a stopped zone) SHALL show the new conversation's rows and thoughts as they open on their own.

#### Scenario: Opened with the pointer
- **WHEN** a closed tool call row has a file change and the user presses its line
- **THEN** the file change shows below the line, and a second press hides it

#### Scenario: Open rows survive updates
- **WHEN** a tool call row is open and the agent appends to the call's command output
- **THEN** the row stays open and shows the new output

#### Scenario: Tab passes over a detail
- **WHEN** a tool call row is open, the prompt input has focus and the user presses tab
- **THEN** focus goes to the zone's next stop, never into the detail

#### Scenario: A new conversation forgets the old rows
- **WHEN** the user opened a finished tool call row and closed a thought in a stopped zone, then starts a new conversation in it, and the new conversation has a finished tool call and a streaming thought at the same places
- **THEN** the new tool call row is closed, as `[agent] tool_details` says, and the new thought is open while it streams

### Requirement: A choice that can't be continued
When the agent refuses to continue a chosen conversation, a zone that was choosing SHALL be choosing again, showing the agent's message, and a zone opened for the choice SHALL show that the agent can't be used, with the message. A choice that can't be sent to the choosing agent, because its link is down or another choice is on its way, SHALL leave the zone choosing and SHALL say why as its reason, never be dropped without a word.

#### Scenario: Refused in a choosing zone
- **WHEN** a choosing zone's chosen conversation is answered by the agent with the error `no such session`
- **THEN** the zone is choosing again and shows `no such session`

#### Scenario: Chosen while the link is down
- **WHEN** a choosing zone's endpoint is reconnecting, and the user chooses a conversation from the palette
- **THEN** the palette closes, the zone is still choosing, and its reason says the choice wasn't sent because the connection to the terminal server is down

### Requirement: Terminal logins are withheld from commands that re-parse arguments
A `terminal` method SHALL NOT be offered when the agent profile's command re-parses its arguments, or hands them to a shell: a `cmd /c` or `cmd /k` command, or a `.bat` or `.cmd` file; `pwsh` with `-Command` or `-EncodedCommand` (or a short form of either); `powershell` unless it runs a script with `-File`; `wsl` unless it runs its command with `--exec`; and a command whose program is `ssh`. The check SHALL be the same on every platform. Its button SHALL be shown unavailable, saying why.

#### Scenario: cmd /c profile
- **WHEN** an agent profile on Windows runs `cmd /c npx agent` and the agent offers a `terminal` method
- **THEN** that method's button is unavailable and says the login can't be run through `cmd /c`

#### Scenario: pwsh -Command profile
- **WHEN** an agent profile runs `pwsh -Command agy acp` and the agent offers a `terminal` method
- **THEN** that method's button is unavailable and says the login can't be run through `pwsh -Command`

#### Scenario: powershell without -File
- **WHEN** an agent profile runs `powershell agy acp` and the agent offers a `terminal` method
- **THEN** that method's button is unavailable and says the login can't be run through `powershell`, which reads its arguments as a command

#### Scenario: wsl profile
- **WHEN** an agent profile runs `wsl agy acp` and the agent offers a `terminal` method
- **THEN** that method's button is unavailable and says the login can't be run through `wsl` without `--exec`

#### Scenario: Commands that pass arguments through
- **WHEN** an agent profile runs `pwsh -File agent.ps1`, `powershell -File agent.ps1` or `wsl --exec agy acp`, and the agent offers a `terminal` method
- **THEN** that method's button is offered, showing the arguments it runs with
