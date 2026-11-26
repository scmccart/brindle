# agent-zones Specification

## Purpose
Show an ACP agent's session in a zone of its own: its conversation, a prompt to talk to it, rows to answer its permission requests and follow its tool calls, its state and failures, with the lifetime terminal zones have.

## Requirements

### Requirement: Opening an agent zone
Opening an agent profile (from a shortcut, the picker, the profile menu, `brindle agent`, or as the default profile) SHALL open a tab whose zone starts an agent session of that profile in the terminal server of the profile's endpoint, running its command and arguments with its environment, in the profile's `cwd`, else the home directory (for `brindle agent`, see `command-line`). The zone SHALL show that the agent is starting until it is ready, and SHALL never wait on the window.

#### Scenario: Agent tab opens
- **WHEN** profile `Agent` sets `type = "agent"` and runs the fake ACP agent, and the user presses its ctrl-alt-N shortcut
- **THEN** a new active tab shows an agent zone that says the agent is starting, then shows an empty conversation and a prompt input with keyboard focus

#### Scenario: From the profile menu
- **WHEN** an agent profile sets no `cwd` and the user opens it from the profile menu
- **THEN** its agent session starts in the home directory

#### Scenario: Remote agent
- **WHEN** an agent profile has `remote = { type = "ssh", host = "A8" }` and the user opens it
- **THEN** the ACP agent runs on `A8` under its terminal server, and the zone shows its conversation

#### Scenario: Agent environment
- **WHEN** an agent profile sets `env = { MODEL = "small" }`
- **THEN** the agent runs with `MODEL=small` and without the variables Brindle sets only for terminals, such as `TERM`

### Requirement: A launch's first prompt
A tab opened by `brindle agent … -p PROMPT` SHALL send PROMPT as the zone's first prompt once its agent is ready, and SHALL send it only once. When the agent needs a login and offers a method Brindle can run, PROMPT SHALL be shown in the unavailable prompt input and SHALL be sent when the agent first becomes ready after a login in that zone, provided the input still holds exactly PROMPT. When the agent can't start, fails or exits before it is ready, or needs a login and offers no method Brindle can run, PROMPT SHALL be left in the prompt input as a draft, unsent. A prompt not yet sent when Brindle quits SHALL be dropped: it is never recorded, and restoring or attaching the session SHALL NOT send it.

#### Scenario: Sent when ready
- **WHEN** the user runs `brindle agent Agent -p hi` and the agent takes a second to become ready
- **THEN** the conversation shows the user message `hi` once the agent is ready, and the agent receives it once

#### Scenario: Not sent again after a restore
- **WHEN** a tab opened with `-p hi` sent its prompt, and the user quits and relaunches
- **THEN** the restored conversation shows `hi` once, and the agent receives no second prompt

#### Scenario: Login demanded
- **WHEN** the user runs `brindle agent Agent -p hi` and the agent needs a login and offers no method
- **THEN** the zone says the agent needs a login, and `hi` is in the prompt input, unsent

#### Scenario: Sent after a login
- **WHEN** the user runs `brindle agent Agent -p hi`, the agent needs a login, and the user logs in with one of its methods
- **THEN** `hi` shows in the unavailable prompt input while the login runs, and once the agent is ready the conversation shows the user message `hi`, the input is empty, and the agent receives `hi` once

#### Scenario: Login cancelled, then done
- **WHEN** the user runs `brindle agent Agent -p hi`, cancels a first login, and completes a second
- **THEN** the agent receives `hi` once, after the second login

### Requirement: The conversation
An agent zone SHALL show its session's entries in order: user messages and notices as plain text, agent messages and thoughts rendered as Markdown, tool calls and permission requests. Text streamed into an entry SHALL appear in that entry as it arrives. Control characters SHALL NOT be interpreted.

#### Scenario: A turn
- **WHEN** the user sends `hi` and the agent streams `Hel`, `lo` and `!` and ends the turn
- **THEN** the conversation shows a user message `hi` followed by one agent message `Hello!`

#### Scenario: Multi-line text
- **WHEN** the agent's message holds three paragraphs separated by blank lines, the second holding a tab
- **THEN** the message shows three paragraphs, with the tab as whitespace

#### Scenario: The user's text stays as typed
- **WHEN** the user sends `**not bold**`
- **THEN** the user's message shows `**not bold**` with its asterisks

### Requirement: Thoughts
A thought SHALL be shown as a collapsible block labelled as the agent's thinking, open while its text is still streaming and closed once another entry follows it. The user SHALL be able to open or close a thought with a pointer press on its label, and every thought of the zone with the `toggle_agent_thoughts` action.

#### Scenario: Closed after the reply
- **WHEN** the agent streams a thought, then a message
- **THEN** the thought shows only its label once the message starts, and pressing the label shows its text

### Requirement: Tool call rows
A tool call SHALL be shown as one line: an icon for its kind, its title cut to one line, and its status (pending, in progress, completed or failed). An update to the call SHALL change that line in place.

#### Scenario: Updated in place
- **WHEN** the agent reports tool call `call_1` titled `Read main.rs` as pending, then as completed
- **THEN** the conversation has one line for `call_1`, which shows completed

### Requirement: Permission rows
A permission request SHALL be shown as a row with its title and one button per option the agent offers, labelled with the option's name, allow options before reject options. Choosing an option SHALL send it as the answer. Once answered, the row SHALL show the chosen option, or that the request was cancelled, and offer no buttons.

#### Scenario: Allow from the keyboard
- **WHEN** the agent asks `Run cargo test?` with options `Allow once` and `Reject`, the prompt input has focus, nothing is queued, and the user presses tab and then enter
- **THEN** the agent receives `Allow once`, and the row says `Allow once` was chosen

#### Scenario: Reject with the pointer
- **WHEN** a permission row is pending and the user clicks `Reject`
- **THEN** the agent receives `Reject`, and the row says so

#### Scenario: Cancelled by stopping the turn
- **WHEN** a permission row is pending and the user stops the turn
- **THEN** the row says the request was cancelled and offers no buttons

### Requirement: Nothing is answered for the user
An agent zone SHALL NOT answer a permission request on its own, by a timeout, or by opening. A pending request SHALL NOT take keyboard focus from where it is when it arrives.

#### Scenario: Typing when a request arrives
- **WHEN** the user is typing in the prompt input and a permission request arrives
- **THEN** the typed text goes on into the prompt input, and the request stays pending

### Requirement: The prompt input
An agent zone SHALL have a multi-line prompt input under the conversation. Enter SHALL send its text, and shift-enter SHALL add a line. A sent prompt SHALL leave the input empty once the server accepts it; a refused one SHALL stay in the input, with the reason shown beside it. Blank text SHALL NOT be sent, and nothing SHALL be sent before the agent is ready.

#### Scenario: Send
- **WHEN** the agent is ready, the user types `hi` and presses enter
- **THEN** the input is empty, and the conversation shows the user message `hi`

#### Scenario: Two lines
- **WHEN** the user types `one`, presses shift-enter, types `two` and presses enter
- **THEN** the conversation shows one user message of two lines, `one` and `two`

#### Scenario: Refused prompt
- **WHEN** the user sends a prompt the server refuses
- **THEN** the text stays in the input, and the reason is shown beside it

### Requirement: A queue of one prompt
A prompt sent during a turn SHALL wait in the session's queue slot, shown in a row above the prompt input as queued, with a remove button, and SHALL be sent only when the turn ends normally (stop reason `end_turn`). While a turn runs and the slot holds a prompt, enter SHALL keep the input's text, with the reason beside it. Nothing from the slot SHALL ever go into the prompt input.

#### Scenario: Queued during a turn
- **WHEN** a turn is running and the user types `next` and presses enter
- **THEN** the input is empty, `next` is shown as queued above it, and when the turn ends normally the conversation shows the user message `next` and the row is gone

#### Scenario: Remove the queued prompt
- **WHEN** `next` is queued and the user runs `agent_unqueue`, or presses the row's remove button
- **THEN** the row is gone, and `next` is never sent

#### Scenario: A third prompt
- **WHEN** a turn is running, `next` is queued, and the user types `third` and presses enter
- **THEN** `third` stays in the input, the reason says a prompt is already queued, and `next` stays queued

### Requirement: A prompt that isn't sent is held
When the turn a queued prompt waits for ends any other way than normally (stopped, failed, or another stop reason such as a token limit), when the agent demands a login or stops being ready, or when a restart loses the turn's end, the row SHALL show the prompt as not sent, with why, a send-again button and a discard button, and the prompt input SHALL be left as it was. A held prompt SHALL NOT be sent on its own, and SHALL survive a quit and a restore.

#### Scenario: Stopping holds it
- **WHEN** `next` is queued, the input holds `draft`, and the user stops the turn
- **THEN** the row shows `next` as not sent because the turn was stopped, and the input still holds `draft`

#### Scenario: A token limit holds it
- **WHEN** `next` is queued and the turn ends with the stop reason `max_tokens`
- **THEN** the row shows `next` as not sent because the turn ended with `max_tokens`, and the agent never receives it

#### Scenario: A failed turn holds it
- **WHEN** `next` is queued and the agent answers the running prompt with an error
- **THEN** the row shows `next` as not sent because the turn failed

#### Scenario: Held across a quit
- **WHEN** `next` is held, and the user quits and relaunches
- **THEN** the restored zone shows `next` as not sent, with its buttons

### Requirement: Sending a held prompt again, or discarding it
The held row's send-again button and the `agent_resend` action SHALL send the held prompt: at once when the agent is ready and no turn runs, else queued again behind the running turn. Its discard button and `agent_unqueue` SHALL discard it. A new prompt sent while one is held and no turn runs SHALL be sent, leaving the held one held. When the agent isn't ready, send-again SHALL be unavailable; once the session has ended, discard SHALL only clear the row.

#### Scenario: Send again when idle
- **WHEN** `next` is held, no turn runs, and the user presses the row's send-again button
- **THEN** the row is gone, and the conversation shows the user message `next`

#### Scenario: Send again during a turn
- **WHEN** `next` is held, the user sends `other`, and while that turn runs presses send-again
- **THEN** the row shows `next` as queued, and it is sent when the `other` turn ends normally

#### Scenario: A new prompt beside a held one
- **WHEN** `next` is held, no turn runs, and the user types `other` and presses enter
- **THEN** `other` is sent, and `next` stays held

#### Scenario: Held when the agent exits
- **WHEN** `next` is queued and the agent exits
- **THEN** the zone says the agent exited, the row shows `next` as not sent because the agent stopped, its send-again button is unavailable, and discard clears the row

#### Scenario: Send again after continuing
- **WHEN** `next` was held because the agent stopped, and the user continues the conversation and the agent is ready
- **THEN** the row's send-again button is available, `next` has not been sent, and pressing the button sends it

### Requirement: Stopping a turn
While a turn runs, the prompt input's send button SHALL become a stop button. The stop button, escape in the prompt input and the `agent_cancel` action SHALL cancel the turn. With no turn running they SHALL do nothing.

#### Scenario: Stop
- **WHEN** a turn is running and the user presses escape in the prompt input
- **THEN** the turn ends as cancelled, and the send button comes back

### Requirement: Scrolling the conversation
The conversation SHALL scroll, and SHALL stay at its end while new text arrives as long as the user is at the end. Scrolling away from the end SHALL keep the view where it is while text arrives, and show a button that jumps to the end; reaching the end again SHALL make it follow again. The wheel, shift-pageup, shift-pagedown and the `agent_scroll_to_bottom` action SHALL scroll it.

#### Scenario: Follows the reply
- **WHEN** the conversation is at its end and the agent streams a reply longer than the zone
- **THEN** the end of the reply stays in view

#### Scenario: Reading back
- **WHEN** the user scrolls up while the agent streams a reply
- **THEN** the lines in view stay put, a jump-to-end button shows, and pressing it shows the end and follows again

### Requirement: Keyboard path through an agent zone
An agent zone SHALL be operable from the keyboard alone. Tab and shift-tab SHALL move focus among the prompt input, the queue row's buttons while it shows, the login row's method buttons or its "Cancel login" button while it shows, the buttons of pending permission rows, and the stop button while a turn runs, in that order and wrapping, and SHALL NOT leave the zone. Keyboard focus SHALL go to the prompt input when the zone is focused.

#### Scenario: Back to the prompt
- **WHEN** focus is on a permission row's last button and the user presses tab with no turn's stop button shown
- **THEN** focus is on the prompt input

#### Scenario: To the login row
- **WHEN** the login row shows two methods, nothing is queued, and focus is on the prompt input
- **THEN** tab puts focus on the first method's button, and tab again on the second's

### Requirement: Copying from the conversation
The `copy_last_agent_message` action SHALL copy the latest agent message's Markdown source to the clipboard. Each agent message SHALL show a copy button while the pointer is over it, which copies that message's Markdown source. Paste SHALL insert into the prompt input.

#### Scenario: Copy the reply
- **WHEN** the agent's last message is `Hello!` and the user runs `Copy Last Agent Message` from the command palette
- **THEN** the clipboard holds `Hello!`

#### Scenario: Copy keeps the Markdown
- **WHEN** the agent's last message is `Run **cargo test** first` and the user runs `Copy Last Agent Message`
- **THEN** the clipboard holds `Run **cargo test** first`, asterisks included

### Requirement: Agent zone appearance
An agent zone SHALL be drawn as a card in its profile's theme, as a terminal zone is. Its text SHALL be in the interface font and its code in the terminal's font family, at the profile's font size, else the terminal font size in effect. Its zone header, when the tab has two or more zones, SHALL show its status dot, its title and the split and close buttons.

#### Scenario: Profile font size
- **WHEN** an agent profile sets `font_size = 18`
- **THEN** its zone's messages are drawn at 18 points and other tabs keep their size

#### Scenario: Code in the terminal's font
- **WHEN** the config's terminal font family is `JetBrains Mono` and an agent message holds a code block and inline code
- **THEN** both are drawn in `JetBrains Mono`, and the message's other text in the interface font

### Requirement: Agent zone title
An agent zone's title SHALL be the name the agent gave, else its profile's name. A tab whose active zone is an agent zone SHALL take that title, unless it has a manual title.

#### Scenario: Agent's name
- **WHEN** the fake ACP agent names itself `Fake Agent` in its answer to `initialize`
- **THEN** the zone's title and its tab's title are `Fake Agent`

### Requirement: Agent zone status
An agent zone SHALL hold its session's activity as a status: `working` (starting, logging in, a turn running), `blocked` (a permission waiting), `done` (a turn ended normally), `error` (a turn ended for a refusal or a limit, or failed; the agent needs a login, failed or exited) or `idle` (a cancelled turn included). The zone header's and tab's dots SHALL show it as a program status record's state.

#### Scenario: Blocked in a background tab
- **WHEN** an agent in a background tab asks permission
- **THEN** that tab shows the yellow status dot

#### Scenario: Refusal is an error
- **WHEN** an agent ends a turn with the stop reason `refusal`
- **THEN** the zone's status is `error`

#### Scenario: Logging in is working
- **WHEN** the user chooses a login method and the agent hasn't answered yet
- **THEN** the zone's status is `working`

### Requirement: Seen statuses clear
An agent zone's `done` and `error` statuses SHALL be dropped once the user types into the zone's prompt input, until the activity changes again. The server's activity SHALL be unchanged by it.

#### Scenario: Done until seen
- **WHEN** an agent's turn ends normally, and later the user types a key into its prompt input
- **THEN** the tab shows the green dot until the key, and no dot after it

### Requirement: An agent that cannot start
When an agent session can't start, the zone SHALL say so in place of the conversation, naming the profile's command and the cause as the terminal server gave it, and the prompt input SHALL be unavailable. A terminal server too old for agent sessions SHALL be named as the cause, with its endpoint, saying it updates once its terminals have ended. A zone in this state SHALL NOT be recorded for session restore.

#### Scenario: No such command
- **WHEN** an agent profile's command is `no-such-agent`
- **THEN** the zone says Brindle could not start `no-such-agent` and why, and offers no prompt

#### Scenario: A batch file on Windows
- **WHEN** an agent profile on Windows has `command = "my-agent"` and `my-agent.cmd` is on `PATH`
- **THEN** the zone says `my-agent` could not be started and shows the form that runs it, `cmd /c my-agent`, as the terminal server gave it

#### Scenario: Server too old
- **WHEN** the user opens an agent profile on an endpoint whose terminal server speaks client protocols 1 to 3 only
- **THEN** the zone says the terminal server on that endpoint is too old for agent sessions and will update once its terminals have ended

### Requirement: An agent that needs a login
When the agent needs the user to log in, at start or in answer to a prompt after it was ready, the zone SHALL say that the agent needs a login, with the agent's own message and a login row offering its methods ("Choosing a login method"), and the prompt input SHALL be unavailable. When the agent offers no method Brindle can run, the zone SHALL say the login is done through the agent's own command line, then by opening the profile again. The conversation so far SHALL stay readable. A held prompt SHALL be discardable, and SHALL be sent again only once the agent is ready.

#### Scenario: Login required
- **WHEN** the fake ACP agent offers no login method and answers `session/new` with an authentication error whose message is `run agent login`
- **THEN** the zone says the agent needs a login through its own command line and shows `run agent login`, and the zone's status is `error`

#### Scenario: Login required with methods
- **WHEN** the fake ACP agent offers a method `Log in with Google` and answers `session/new` with an authentication error whose message is `run agent login`
- **THEN** the zone says the agent needs a login, shows `run agent login` and a button `Log in with Google`, and the zone's status is `error`

#### Scenario: Login demanded by a prompt
- **WHEN** an agent has answered one turn, the user sends `second` and, during that turn, queues `third`, and the agent answers `second` with an authentication error whose message is `session expired`
- **THEN** the first turn is still shown, the `second` turn shows as failed, the zone says the agent needs a login and shows `session expired`, the prompt input is unavailable, and the row shows `third` as not sent because the agent needs a login, with send-again unavailable and discard clearing the row

### Requirement: An agent that exits
When an agent exits, its zone SHALL stay open and say that the agent exited, with its exit status and the end of its standard error, and the prompt input SHALL be unavailable, unless the agent had already failed for its ACP version (see "An agent that doesn't speak ACP version 1"). When the conversation can be continued the zone SHALL offer to continue it (see "Continuing a conversation"). The zone SHALL close only when the user closes it, and SHALL NOT be recorded for session restore.

#### Scenario: Agent crashes
- **WHEN** an agent writes `fatal: no model` to its standard error and exits with status 1
- **THEN** its zone stays open, saying the agent exited with status 1 and showing `fatal: no model`

#### Scenario: Agent crashes with a conversation to continue
- **WHEN** an agent that offered `session/resume` and had a ready ACP session exits with status 1
- **THEN** its zone says the agent exited with status 1, and shows a Continue conversation button where the prompt input's reason is

#### Scenario: Agent lost with its holder
- **WHEN** the holder running an agent that offered `session/load` is killed
- **THEN** the zone says the agent was lost with its terminal server's holder, and offers to continue the conversation

### Requirement: An agent that doesn't speak ACP version 1
When the terminal server ends an agent because it doesn't speak ACP version 1, the zone SHALL stay open and keep showing, as its final state, that the agent can't be used, with the server's message naming the versions, and SHALL NOT show an exit status in its place. The agent's standard error SHALL show as a notice in the conversation, the prompt input SHALL be unavailable, and the zone SHALL NOT be recorded for session restore.

#### Scenario: Agent refuses version 1
- **WHEN** the fake ACP agent answers `initialize` with error `-32022`, naming version 2 as the one it supports, and writes `only ACP v2` to its standard error
- **THEN** the zone says the agent can't be used and that it supports ACP versions 2 while Brindle speaks 1, shows a notice with `only ACP v2`, and shows no exit status, also after the session has ended

#### Scenario: Agent answers another version
- **WHEN** the fake ACP agent answers `initialize` with protocol version 2
- **THEN** the zone says the agent can't be used because it speaks ACP version 2 and Brindle speaks 1, and shows no exit status

### Requirement: A disconnected endpoint
While the connection to an agent zone's endpoint is down, the zone SHALL say it is reconnecting and SHALL NOT send prompts, keeping the prompt input's text. When the session can't be got back, the zone SHALL say the session was lost, and stay open until the user closes it; when the zone knows the session's conversation, it SHALL offer to continue it in a new session.

#### Scenario: Reconnected
- **WHEN** the link to an SSH endpoint drops while its agent zone is idle, and comes back
- **THEN** the zone said it was reconnecting, then shows the conversation as before, and a prompt sent afterwards reaches the agent

#### Scenario: Lost after the agent exited
- **WHEN** an agent zone's agent has exited, and the terminal server's core is killed and replaced before the user continues it
- **THEN** the zone says the session was lost, and still offers to continue the conversation

### Requirement: A session Brindle can no longer show
When Brindle fails to follow an agent session's conversation, the zone SHALL say it can no longer show the session and that it can be attached again from Attach Session…, and the session SHALL be detached, still running.

#### Scenario: Broken fold
- **WHEN** the conversation events of an agent zone's session can't be decoded or applied to the zone's copy
- **THEN** the zone says it can't show the session any more, and the session is listed as detached

### Requirement: Closing an agent mid-turn asks first
Closing an agent zone, or a tab holding agent zones, whose agent's status is `working` or `blocked` SHALL first ask for confirmation in a dialog naming each such agent and what it is doing, with Cancel and Close buttons, Cancel focused. An agent that is logging in SHALL NOT count as working for this. Close SHALL close it; Cancel, escape and a press on the backdrop SHALL leave everything as it was. Otherwise closing SHALL end the agent at once, without asking. Quitting Brindle SHALL NOT ask.

#### Scenario: Close a working agent
- **WHEN** an agent zone's turn is running and the user presses ctrl-shift-x
- **THEN** a dialog asks whether to close the working agent, with Cancel focused, and the turn goes on

#### Scenario: Confirmed from the keyboard
- **WHEN** the dialog is open and the user presses tab and then enter
- **THEN** the zone closes, and its agent ends

#### Scenario: Cancelled
- **WHEN** the dialog is open and the user presses enter or escape
- **THEN** the dialog closes, the zone stays open with focus in its prompt input, and the turn goes on

#### Scenario: Close Tab with one busy agent
- **WHEN** a tab has two agent zones, one waiting on a permission request and one idle, and the user presses ctrl-shift-w
- **THEN** the dialog names the agent waiting for permission, and confirming closes the tab and ends both agents

#### Scenario: Idle agent
- **WHEN** an agent zone's last turn has ended and the user closes the zone
- **THEN** the zone closes at once, and its agent ends

#### Scenario: Quit never asks
- **WHEN** an agent's turn is running and the user presses ctrl-shift-q
- **THEN** Brindle quits without asking, and the agent session is listed as detached

#### Scenario: Logging in doesn't ask
- **WHEN** an agent zone's agent is logging in and the user closes the zone
- **THEN** the zone closes at once, and its agent ends

### Requirement: Closing an agent zone ends its agent
Closing an agent zone, or its tab, once confirmed where that is asked, SHALL end its agent session in the terminal server, and a pending permission request and a queued or held prompt SHALL go with it.

#### Scenario: Close
- **WHEN** an agent zone waits on a permission request, the user closes the zone and confirms
- **THEN** the agent and the processes it started are gone within a few seconds, and no session is listed for it

### Requirement: Quitting keeps agents running
Quitting Brindle, or Brindle ending without quitting, SHALL leave every agent session running, detached with its tab's layout, including one in the middle of a turn, with a queued or held prompt, or waiting on a permission request.

#### Scenario: Quit mid-turn
- **WHEN** an agent zone's turn is running and the user quits Brindle
- **THEN** the agent session is listed as detached with the same pid, and the turn goes on

#### Scenario: Permission waits across a quit
- **WHEN** an agent waits on a permission request, the user quits, and launches Brindle again
- **THEN** the restored agent zone shows the pending request with its buttons, and choosing one answers it

### Requirement: Replays raise nothing
Attaching an agent session, at launch or from the sessions palette, SHALL show its whole conversation and its queued or held prompt, and SHALL NOT raise notifications for what the replay holds.

#### Scenario: Restored after a finished turn
- **WHEN** an agent finished a turn while Brindle wasn't running, and Brindle is launched
- **THEN** the restored zone shows the whole turn and its done status, and no notification appears

### Requirement: Agent commands in the palette
While an agent zone has focus, the command palette SHALL list Stop Agent Turn, Remove Queued Prompt, Send Held Prompt Again, Copy Queued Prompt, Copy Last Agent Message, Copy Last Code Block, Toggle Agent Thoughts and Scroll Agent to Bottom, with the zone and tab commands, and SHALL NOT list commands that act on a terminal, such as Clear Scrollback or Find.

#### Scenario: Palette in an agent zone
- **WHEN** an agent zone has focus and the user opens the command palette
- **THEN** it lists "Stop Agent Turn", "Copy Last Code Block" and "Split Zone Right", and not "Clear Scrollback"

### Requirement: Markdown in agent messages and thoughts
Agent messages and thoughts SHALL be rendered as CommonMark with GitHub's strikethrough, task lists, tables and autolinks: paragraphs, headings, emphasis, inline code, code blocks, lists, quotes, rules and links. Footnotes, math and smart punctuation SHALL NOT be interpreted; their text shows as written. A line break inside a paragraph SHALL read as a space, as CommonMark says.

#### Scenario: Emphasis and a heading
- **WHEN** the agent's message is `## Plan` then a blank line and `Run **cargo test** and *read* the ~~old~~ log`
- **THEN** the message shows a level-two heading `Plan` and a paragraph with `cargo test` bold, `read` italic and `old` struck through, with no `#`, `*` or `~` shown

#### Scenario: Task list
- **WHEN** the agent's message is `- [x] build` and `- [ ] test` on two lines
- **THEN** the message shows a list of two items, `build` marked done and `test` marked open

#### Scenario: Footnote stays text
- **WHEN** the agent's message is `See [^1]` then a blank line and `[^1]: the note`
- **THEN** both lines show as written, as two paragraphs

### Requirement: Code blocks in the conversation
A fenced or indented code block in an agent message or thought SHALL be drawn in the terminal's font family on the muted surface, keeping its line breaks and indentation, with the first word of its fence's info string as a label. It SHALL have a copy button, reached with the pointer only, that copies the block's content exactly, without its fences. Code SHALL NOT be syntax highlighted.

#### Scenario: Fenced code
- **WHEN** the agent's message holds a fence opened with three backticks and `rust`, the line `fn main() {}`, and the closing fence
- **THEN** the message shows `fn main() {}` in the terminal's font family under the label `rust`, and no backticks

#### Scenario: Copying a code block
- **WHEN** the user presses the copy button of a code block holding two indented lines
- **THEN** the clipboard holds those two lines exactly, indentation and final line break included, and no fence

### Requirement: Copy Last Code Block
The `copy_last_code_block` action, listed in the command palette as "Copy Last Code Block" with no default key, SHALL copy the content of the last code block of the latest agent message that has one, exactly as the code block's copy button does. When no agent message has a code block, the clipboard SHALL be left unchanged.

#### Scenario: From the palette
- **WHEN** the agent's last message holds two code blocks, `a` and then `b`, and the user runs Copy Last Code Block
- **THEN** the clipboard holds `b` and its line break

#### Scenario: No code
- **WHEN** no agent message in the zone holds a code block and the user runs Copy Last Code Block
- **THEN** the clipboard is unchanged

### Requirement: Markdown while it streams
As an agent message or thought streams, it SHALL be rendered again from its whole text as the text arrives, so it always shows what its text so far means. A code fence not yet closed SHALL show as a code block running to the end of the text. The blocks of a message that the new text doesn't change SHALL stay where they are.

#### Scenario: Unclosed fence
- **WHEN** the agent has streamed a fence opened with three backticks and `rust` and two lines of code, and not yet the closing fence
- **THEN** the message shows those two lines as a code block labelled `rust`, and the paragraph above it unchanged

#### Scenario: A definition that comes later
- **WHEN** the agent streams `See [the docs][d].`, a blank line, and then `[d]: https://example.com`
- **THEN** once the definition has arrived, `the docs` shows as a link to `https://example.com`, and the definition line isn't shown

### Requirement: Tables in the conversation
A GitHub table in an agent message or thought SHALL be drawn as a grid: one column per header cell, the header row set apart from the body, and each column's text aligned as its delimiter row says. A cell's text SHALL wrap within its column.

#### Scenario: Aligned table
- **WHEN** the agent's message holds a table with header `| name | size |`, delimiter row `| :--- | ---: |` and two rows
- **THEN** the message shows a two-column grid with a set-apart header, names aligned to the start and sizes to the end, and no pipes

### Requirement: Images in the conversation are never fetched
An image in an agent message or thought SHALL NOT be fetched or drawn. It SHALL show as `[image: ALT]` in the muted colour, followed by its target (its first 120 characters and an ellipsis when longer), and SHALL act as a link to its whole target.

#### Scenario: Remote image
- **WHEN** the agent's message holds `![chart](https://example.com/c.png)`
- **THEN** the message shows `[image: chart]` and `https://example.com/c.png` muted, nothing is requested from `example.com`, and the text acts as a link to `https://example.com/c.png`

### Requirement: Raw HTML shows as text
HTML in an agent message or thought, as a block or inline, SHALL be shown as its literal text and SHALL NOT be interpreted.

#### Scenario: Inline HTML
- **WHEN** the agent's message is `a <b>bold</b> claim <script>x()</script>`
- **THEN** the message shows that text with its tags as written, and nothing is bold or run

### Requirement: Continuing a conversation
A zone whose agent exited or was lost with a conversation it can continue SHALL show a Continue conversation button, and the `agent_continue` action (Continue Agent Conversation) SHALL do the same. Either SHALL start the zone's agent profile's command again with its current arguments and environment on the same session, or in a new session when the server lost it, and the zone SHALL say the agent is starting until it is ready. Nothing SHALL be continued without the user asking.

#### Scenario: Continue after a crash
- **WHEN** an agent zone's agent exited after one turn, and the user presses Continue conversation
- **THEN** the zone says the agent is starting, then shows the earlier turn, and the prompt input takes a prompt that reaches the agent

#### Scenario: From the palette
- **WHEN** an agent zone's agent exited and the user runs Continue Agent Conversation from the palette
- **THEN** the conversation is continued as from the button

#### Scenario: Never on its own
- **WHEN** an agent zone's agent exits and a minute passes
- **THEN** the agent hasn't been started again

#### Scenario: Replay not shown
- **WHEN** a zone is continued with an agent that offers only `session/load`
- **THEN** the conversation shows the earlier turns once, then a notice that the conversation continued and the agent's replay wasn't shown

#### Scenario: Continued in a new session
- **WHEN** a zone says its session was lost, and the user continues the conversation
- **THEN** the zone shows a new session's conversation, as the agent replayed it, after a notice that it continues an earlier conversation

### Requirement: A conversation that can't be continued
A zone whose agent ended SHALL NOT offer to continue when the agent offered neither `session/load` nor `session/resume`, when it never gave an ACP session, or when the zone's agent profile no longer exists; the prompt input's reason SHALL say which. A continue that the agent refuses SHALL leave the zone showing that the agent can't be used, with the agent's message.

#### Scenario: Agent without load or resume
- **WHEN** an agent that offered neither `session/load` nor `session/resume` exits
- **THEN** the zone has no Continue conversation button, and the reason says the agent can't continue conversations

#### Scenario: Profile deleted
- **WHEN** an agent zone's agent exited and its profile has since been removed from the config
- **THEN** the zone has no Continue conversation button, and the reason names the missing profile

#### Scenario: Agent refuses
- **WHEN** the user continues a zone and the agent answers `session/resume` with an error `no such session`
- **THEN** the zone says the agent can't be used, shows `no such session`, and keeps the earlier conversation

### Requirement: Choosing a login method
When the agent needs a login and offers methods, the zone's login row SHALL show one button per method Brindle can run, in the agent's order, each labelled with the method's name and showing its description as plain text, with nothing chosen or focused for the user. Choosing a method SHALL start that login. The row SHALL NOT time out.

#### Scenario: Two methods
- **WHEN** the fake ACP agent offers `Log in with Google` and `API Key` (described `Use an API key`) and answers `session/new` with an authentication error
- **THEN** the zone shows a button `Log in with Google`, then a button `API Key` with `Use an API key`, neither chosen, and the prompt input keeps keyboard focus

#### Scenario: Chosen from the keyboard
- **WHEN** the login row shows and the user presses tab until `Log in with Google` has focus, then enter
- **THEN** the agent receives `authenticate` for that method, and the zone says it is logging in with `Log in with Google`

#### Scenario: Description with backticks
- **WHEN** a method's description is ``Run `claude /login` in the terminal``
- **THEN** the row shows that text with its backticks, unformatted

### Requirement: Logging in from the zone
While a login runs, the zone SHALL say the agent is logging in, naming the method, keep the conversation readable and the prompt input unavailable, and offer a "Cancel login" button. When the login succeeds the zone SHALL become usable again without being reopened; when it fails, the zone SHALL show the agent's error and the methods again.

#### Scenario: Login succeeds
- **WHEN** the user chooses a method and the fake ACP agent answers `authenticate` and `session/new` successfully
- **THEN** the login row is gone, the prompt input takes prompts, and the zone's status is no longer `error`

#### Scenario: Login fails
- **WHEN** the user chooses a method and the agent answers `authenticate` with an error whose message is `browser login timed out`
- **THEN** the zone says the agent needs a login, shows `browser login timed out`, and shows the method buttons again

### Requirement: Cancelling a login
The "Cancel login" button SHALL cancel the running login and bring back the method buttons with the message the zone showed before. Closing the zone SHALL always be possible while a login runs, and SHALL end the agent.

#### Scenario: Cancel
- **WHEN** a login is running and the user presses "Cancel login"
- **THEN** the zone shows the method buttons again, and a later answer from the agent to that login changes nothing

#### Scenario: Close while logging in
- **WHEN** a login is running and the user presses ctrl-shift-x
- **THEN** the zone closes without asking, and its agent ends

### Requirement: After a login
Once the agent is ready again after a login demanded by a prompt, the prompt whose turn failed SHALL stay failed in the conversation and SHALL NOT be offered to be sent again, and a prompt held because the agent needed a login SHALL get its send-again button back.

#### Scenario: Held prompt sent again after a login
- **WHEN** the agent demanded a login while answering `second`, `third` was held because the agent needed a login, and the user logs in
- **THEN** the `second` turn still shows as failed with no way to send it again, the row shows `third` as not sent with send-again available, and pressing it sends `third`

### Requirement: A terminal login runs beside the agent zone
Choosing a `terminal` method SHALL open a terminal zone beside the agent zone, on the agent profile's endpoint, running the profile's command and arguments followed by the method's arguments, with the profile's environment and the method's environment applied. When its program exits with status 0, the agent SHALL be started again in the same agent session, keeping the conversation; any other exit SHALL show the methods again with the exit status.

#### Scenario: Terminal login succeeds
- **WHEN** the fake ACP agent offers a `terminal` method with args `["login"]`, the user chooses it, and the program in the login zone exits with status 0
- **THEN** a terminal zone ran the profile's command with `login` appended beside the agent zone and closed when it exited, the agent was started again, and the agent zone takes prompts with its conversation kept

#### Scenario: Terminal login fails
- **WHEN** the program in the login zone exits with status 1
- **THEN** the agent zone says the login program exited with status 1 and shows the method buttons again

### Requirement: A terminal login zone is not kept
A terminal login's zone SHALL NOT be recorded for session restore or in a detached layout, and quitting Brindle, or Brindle ending, SHALL end its program rather than leave it detached.

#### Scenario: Quit mid-login
- **WHEN** a terminal login's zone is open beside its agent zone and the user quits and relaunches
- **THEN** the restored tab shows only the agent zone, needing a login, and the login program is no longer running

### Requirement: What a terminal login shows and applies
Before a `terminal` method is chosen, its button SHALL show the arguments it appends and the environment it sets. Environment variables that change how programs are found, loaded or trusted (`PATH`, `LD_*`, `DYLD_*`, `NODE_OPTIONS`, `NODE_PATH`, `PYTHON*`, `RUBYOPT`, `PERL5OPT`, `*_PROXY`, `SSL_CERT_*`, among others on a fixed list) SHALL NOT be applied, and SHALL be named on the button as not applied.

#### Scenario: Shown before launch
- **WHEN** a `terminal` method appends `login` and sets `LOGIN_MODE=device`
- **THEN** its button shows `login` and `LOGIN_MODE=device` before it is chosen

#### Scenario: PATH not applied
- **WHEN** a `terminal` method sets `PATH=/tmp/evil` and `LOGIN_MODE=device`
- **THEN** its button says `PATH` is not applied, and the login zone's program runs with `LOGIN_MODE=device` and the profile's own `PATH`

### Requirement: Terminal logins are withheld from commands that re-parse arguments
A `terminal` method SHALL NOT be offered when the agent profile's command re-parses its arguments: on Windows a `cmd /c` command or a `.bat` or `.cmd` file, and on any platform a command whose program is `ssh`. Its button SHALL be shown unavailable, saying why.

#### Scenario: cmd /c profile
- **WHEN** an agent profile on Windows runs `cmd /c npx agent` and the agent offers a `terminal` method
- **THEN** that method's button is unavailable and says the login can't be run through `cmd /c`
