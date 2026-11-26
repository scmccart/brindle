## ADDED Requirements

### Requirement: Each zone's profile and host are recorded
The session state SHALL record, for every zone of a tab, the profile it runs and, when it runs on a remote host, that host. A restored zone SHALL run its own recorded profile and attach or start on its own recorded host, whatever the other zones of its tab use. Zones on different hosts SHALL be restored independently: one whose host can't be reached SHALL say so, as a restored remote tab does, and SHALL NOT delay or change the others.

#### Scenario: A mixed tab comes back
- **WHEN** a tab holds a shell of profile `Shell` and an agent zone of profile `Claude`, each with a running session, and the user quits and relaunches
- **THEN** the tab comes back with both zones in their split, the shell showing its screen and the agent zone its conversation

#### Scenario: Two hosts come back
- **WHEN** a tab holds a local shell and a shell on `A8`, and the user quits and relaunches
- **THEN** the tab comes back with both zones, each attached to its session where it ran

#### Scenario: One host is unreachable
- **WHEN** that tab is restored while `A8` can't be reached
- **THEN** the local zone shows its shell at once, and the `A8` zone says it can't reach `A8`

### Requirement: State files from before per-zone profiles
A state file written before zones recorded their own profiles SHALL restore as it did: every zone of a recorded tab SHALL take the tab's recorded profile and host. The state file's format version SHALL NOT change, and the tab's recorded profile and host SHALL be its active zone's, so that an earlier Brindle reading a newer file still restores the tab.

#### Scenario: An earlier state file
- **WHEN** the state file was written by a version before this one, recording a tab of profile `Work` split into two zones
- **THEN** after a launch the tab comes back with both zones of profile `Work`

#### Scenario: Read by an earlier version
- **WHEN** this version records a tab whose active zone is of profile `Claude`, and its file is read back
- **THEN** the tab's own recorded profile is `Claude`

### Requirement: Swept tabs hold both kinds
A tab opened from a recorded layout by a launch's sweep SHALL hold every session the layout names that the sweep attaches, terminal and agent alike. Each zone SHALL use the profile its own session gives it by the rules of "Launch reattaches detached terminals" and "Launch reattaches detached agent sessions"; where those rules name the profile of the first tab on an endpoint, it SHALL be the profile of the first zone on it.

#### Scenario: A mixed split comes back without the state file
- **WHEN** Brindle quit with a tab split into a shell and an agent zone on this machine, and Brindle is launched with another state directory against the same terminal server
- **THEN** the launch opens one tab with both sessions in the recorded split, the shell as a terminal zone and the agent as an agent zone

#### Scenario: Each zone's own profile
- **WHEN** a swept layout names a shell started with profile `Work` and an agent started with profile `Claude`
- **THEN** the tab's terminal zone is of `Work` and its agent zone of `Claude`

### Requirement: A swept layout groups one terminal server's sessions
A launch's sweep SHALL group into a tab only the sessions of the terminal server it is sweeping. A recorded layout whose zones ran on several hosts SHALL open as one tab for each host swept, each without the zones that ran elsewhere, a split's sibling taking a missing zone's place.

#### Scenario: A tab that spanned hosts
- **WHEN** Brindle quit with a tab holding a local shell and a shell on `A8`, and Brindle is launched with another state directory, with profile `A8` connecting at launch
- **THEN** the launch opens a tab with the local shell, and, once `A8` answers, a tab with the `A8` shell

#### Scenario: Restored whole from the state file
- **WHEN** that Brindle is launched with its own state directory instead
- **THEN** one tab comes back with both shells in their split

## MODIFIED Requirements

### Requirement: Local tabs restore their profile, zones and directory
A restored local tab SHALL restore its recorded zone split tree, with each zone running that zone's recorded profile in that zone's recorded working directory, and focus the tab's recorded active zone. Each zone's recorded directory SHALL be the working directory of that zone's foreground process, the same directory a new tab or split would inherit. A recorded local tab without a split tree SHALL restore as a single zone in the tab's recorded working directory. When a recorded local tab has a manual title, that title SHALL be restored with the tab. A zone with a recorded terminal server session SHALL instead be attached to that session, or not restored, as defined by "Restored zones get their sessions back".

#### Scenario: Directory comes back
- **WHEN** a zone without a session record (in a state file from an earlier version) was in `/srv/www` when Brindle quit
- **THEN** after a launch that zone's new shell starts in `/srv/www`

#### Scenario: Split zone tree comes back
- **WHEN** a local tab was split right, with the left zone in `/srv/api` and the focused right zone in `/srv/web`, when Brindle quit
- **THEN** after a launch the tab opens with two side-by-side zones in `/srv/api` and `/srv/web`, and the right zone is focused

#### Scenario: Manual title comes back
- **WHEN** a local tab has manual title `server` when Brindle quits
- **THEN** after a launch that tab is restored with title `server`

#### Scenario: Each zone's profile comes back
- **WHEN** a tab was split into a zone of profile `Shell` and a zone of profile `Work`, neither with a session record, when Brindle quit
- **THEN** after a launch the first zone runs `Shell`'s program and the second `Work`'s

### Requirement: Restoring falls back to defaults
Restoring SHALL succeed even when the recorded state no longer matches the system:
- a zone's missing profile SHALL be replaced by the default profile, and that zone SHALL say, in one dim line, that its recorded profile is gone and which profile it uses, after any replay of its screen;
- a missing directory for a tab or zone SHALL be replaced by the profile's own directory, else the home directory.

#### Scenario: Profile removed from the config
- **WHEN** a recorded tab's profile `Work` has been removed from the config
- **THEN** the tab is restored with the default profile in its recorded directory, and says that profile `Work` is gone and the default profile is used

#### Scenario: Directory deleted
- **WHEN** a recorded tab's or zone's directory has been deleted since Brindle quit
- **THEN** that terminal is restored in the home directory (for a profile without its own directory)

#### Scenario: One zone's profile removed
- **WHEN** a recorded tab holds a zone of profile `Shell` and a zone of profile `Work`, and `Work` has been removed from the config
- **THEN** only the second zone says that profile `Work` is gone, and the first is restored as `Shell`

### Requirement: Restored agent zones get their sessions back
An agent zone SHALL be recorded and restored as a terminal zone is ("Restored zones get their sessions back"), attaching its recorded agent session and showing its whole conversation. When the listing failed, a recorded agent zone SHALL start a new agent session of its profile in its recorded directory. A recorded zone whose listed session is of another kind than its recorded profile now is SHALL be restored as what the session is, a terminal zone for a terminal session and an agent zone for an agent session, under the profile a launch's sweep would give that session.

#### Scenario: Agent survives a quit
- **WHEN** an agent zone has a finished turn, and the user quits and relaunches
- **THEN** the restored tab shows the same agent session, with the same pid and the whole conversation

#### Scenario: Agent split comes back
- **WHEN** an agent tab split into two agent zones is quit and relaunched
- **THEN** the tab comes back with both agent zones in their split, each showing its own conversation

#### Scenario: Kind changed under a recorded zone
- **WHEN** a recorded shell zone's profile was changed to `type = "agent"` before the relaunch, and its recorded terminal session still runs
- **THEN** the zone comes back in its tab as a terminal zone showing that session, under a terminal profile, and no agent zone shows it

### Requirement: Launch reattaches detached agent sessions
A launch SHALL attach detached agent sessions as it attaches detached terminals ("Launch reattaches detached terminals"), each as an agent zone, grouped by their recorded layouts with the terminal sessions those layouts name. An agent session's zone SHALL use, of the profiles the terminal rules allow, the first that is an agent profile, else the profile those rules give.

#### Scenario: Agent left by a skipped launch
- **WHEN** a `--no-restore` launch left an agent session of profile `Agent` detached, and the user quits and launches `brindle`
- **THEN** the window shows the recorded tabs followed by an agent tab of profile `Agent` showing that session's conversation

#### Scenario: Agent profile gone
- **WHEN** a detached agent session's profile has been deleted, and the config has another agent profile `Other`
- **THEN** the launch attaches it as an agent tab of profile `Other`

#### Scenario: No agent profile at all
- **WHEN** a detached agent session is attached at launch and the config has no agent profile
- **THEN** it opens as an agent zone of the default profile, showing its conversation; an agent split of it does nothing, and a terminal split opens a shell beside it
