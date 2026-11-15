## MODIFIED Requirements

### Requirement: One connection per host
Brindle SHALL keep at most one connection to each remote endpoint, shared by all of that endpoint's tabs and zones. An endpoint is identified by its SSH target (host, user, port, identity file and SSH arguments) or its container target (engine, container and user). Opening a tab or splitting a zone on an endpoint that is already connected SHALL NOT start another SSH or exec connection. A connection SHALL end once no terminal uses it, unless a profile of that endpoint connects at launch. A terminal whose program is running, closed while its endpoint can't be reached, SHALL keep using the connection until its program has been told to end, for at most 5 minutes after the close. A terminal closed before its session started or was attached SHALL NOT keep using it. A launch waiting for an endpoint's first listing (see "Launch reattaches detached terminals" in `session-restore`) SHALL NOT keep a connection that no terminal uses.

#### Scenario: Splits share the connection
- **WHEN** an SSH tab on `A8` is split twice
- **THEN** Brindle runs one `ssh` process for `A8`, and the three zones' sessions are listed on `A8`'s server

#### Scenario: Last tab closed
- **WHEN** the only tab on host `A8` is closed, and no profile for `A8` connects at launch
- **THEN** Brindle's `ssh` process for `A8` exits within a few seconds

#### Scenario: Closed while the host is unreachable
- **WHEN** the only tab on host `A8`, whose program is running, is closed while `A8` can't be reached, no profile for `A8` connects at launch, and `A8` can be reached again two minutes later
- **THEN** the tab's program on `A8` ends, and Brindle's `ssh` process for `A8` exits within a few seconds after that

#### Scenario: Host gone for good
- **WHEN** the only tab on host `A8`, whose program is running, is closed while `A8` can't be reached, no profile for `A8` connects at launch, and `A8` never comes back
- **THEN** Brindle stops trying to connect to `A8` about 5 minutes after the close, its log names the session it couldn't end, and "Attach Session…" no longer lists `A8`

#### Scenario: Restored tab closed before its host answers
- **WHEN** a restored tab on `A8` is connecting because `A8` can't be reached, no profile for `A8` connects at launch, and the user closes that tab
- **THEN** Brindle stops trying to connect to `A8` within a few seconds, and the session the tab was attaching stays detached on `A8`

### Requirement: Remote terminals persist
Remote terminals SHALL follow the same lifetime rules as local terminals:
- quitting Brindle, or Brindle ending without quitting, leaves them running detached on their host, except `-e` command terminals;
- closing a remote zone or tab ends its program. When its host can't be reached, the program ends once the host can be reached again within 5 minutes of the close. After that it is left as if Brindle had ended without quitting (see "One connection per host"). A zone or tab closed before its session was attached in this run, such as a restored one whose host can't be reached, leaves that session detached;
- detached remote sessions can be attached again with their screens.

#### Scenario: Quit with a remote tab
- **WHEN** an SSH tab on `A8` runs `htop` and the user quits Brindle
- **THEN** `htop` keeps running on `A8`, and `brindle-remote list` there shows its session as detached

#### Scenario: Close a remote zone
- **WHEN** a container zone runs `sleep 1000` and the user closes the zone
- **THEN** the `sleep` process in the container is gone within a few seconds

#### Scenario: Closed while the host is away too long
- **WHEN** an SSH tab on `A8` runs `sleep 1000`, the user closes it while `A8` can't be reached, and `A8` can be reached again only 10 minutes later
- **THEN** the `sleep` is still running on `A8`, and `brindle-remote list` there shows its session as detached

### Requirement: Lost connections reconnect
Brindle SHALL notice a remote connection that has gone silent, even when the transport never closes, within about half a minute. It SHALL then show that the connection was lost in each of that endpoint's terminals, and reconnect with growing pauses (at most 30 s apart) for as long as any terminal uses the endpoint. Where Brindle reports the loss (its log, and the endpoint's row in the sessions palette), it SHALL say what ended the link:
- the SSH or container engine process exiting, with its exit status and what it wrote;
- nothing arriving within Brindle's own limit;
- or the far end closing the link.

#### Scenario: Network drop
- **WHEN** the network to `A8` is cut for two minutes while a tab there runs `htop`, and then restored
- **THEN** the tab says the connection was lost, and within 30 s of the restore it shows `htop` again, with the same pid

#### Scenario: ssh gives up
- **WHEN** the network to `A8` is cut, and `ssh` exits because its keepalives go unanswered
- **THEN** Brindle's log says the connection to `A8` ended because `ssh` exited, with its exit status and anything it wrote, and doesn't say that the terminal server closed the connection

#### Scenario: Silent link
- **WHEN** the link to `A8` delivers nothing for Brindle's limit, while `ssh` keeps running
- **THEN** Brindle's log says that nothing was read from `A8` in that time, and the `ssh` process is ended

### Requirement: Connecting at launch
At launch, Brindle SHALL connect in the background to the endpoint of every profile with `connect_at_launch = true`, and keep those connections for as long as it runs. Connecting SHALL NOT delay the window or block input to other tabs. An endpoint that can't be reached, or whose terminal server refuses this Brindle, SHALL count as offline and be retried with growing pauses, at most 60 s apart. Brindle SHALL NOT start or create a container to connect to it.

#### Scenario: Host reachable
- **WHEN** profile `A8` connects at launch and `A8` has a detached session running `htop`
- **THEN** shortly after the window opens, a tab shows that `htop`

#### Scenario: Host unreachable
- **WHEN** profile `A8` connects at launch and `A8` can't be reached
- **THEN** the window opens as quickly as without the profile, and "Attach Session…" shows `A8` as offline

#### Scenario: Container stopped
- **WHEN** a container profile connects at launch and its container is stopped
- **THEN** the container stays stopped, and "Attach Session…" shows it as offline

#### Scenario: Server refuses
- **WHEN** profile `A8` connects at launch, and `A8`'s terminal server refuses this Brindle until that server exits
- **THEN** "Attach Session…" shows `A8` as offline with the refusal meanwhile, and within a minute of the refusing server exiting Brindle is connected to `A8`, without a tab being opened there
