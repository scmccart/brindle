## ADDED Requirements

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

## MODIFIED Requirements

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

### Requirement: Keyboard path through an agent zone
An agent zone SHALL be operable from the keyboard alone. Tab and shift-tab SHALL move focus among the prompt input, the queue row's buttons while it shows, the login row's method buttons or its "Cancel login" button while it shows, the buttons of pending permission rows, and the stop button while a turn runs, in that order and wrapping, and SHALL NOT leave the zone. Keyboard focus SHALL go to the prompt input when the zone is focused.

#### Scenario: Back to the prompt
- **WHEN** focus is on a permission row's last button and the user presses tab with no turn's stop button shown
- **THEN** focus is on the prompt input

#### Scenario: To the login row
- **WHEN** the login row shows two methods, nothing is queued, and focus is on the prompt input
- **THEN** tab puts focus on the first method's button, and tab again on the second's

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
