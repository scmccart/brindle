# Proposal

## Why

`terminal-server-remote` moved remote tabs onto the host's terminal server. Implementing it, and running the new Docker e2e cases (`tests/e2e/cases/remote.rs`), left a list of rough edges:
- error rows and log lines that don't say why, or blame the wrong party;
- connections that never stop trying;
- a sessions palette that loses its place and waits for its slowest host.

None of them blocks normal use. Each one costs the user a wrong conclusion, a stray `ssh` every 30 s, or a wrong session attached. They are cheap to fix while the code is fresh.

## What Changes

Each item says what happens at HEAD (after `terminal-server-remote`) and what changes. Open questions are in design.md; the specs and tasks follow their recommendations.

### Saying why

1. **An offline endpoint's row says why.**
   - **Now:** a container launch that fails before the relay's frame marker produces a multi-line message. The first line is `the relay didn't start: the stream ended before the frame marker… (docker exited with 1)`, and docker's `Error response from daemon: container … is not running` comes on a later line (`local::relay::start`). The sessions palette shows only the first line of an offline endpoint's error (`picker::session_palette`), so a stopped container's row never says why. An SSH failure (exit 255) was already folded onto one line by `client::unlaunched`. The unit fixtures were all one line, which is why they passed.
   - **Change:** a launch failure's first line names its cause: the last line the ssh or engine process wrote to stderr, then its exit status. The rest follows on later lines. The tab, the log and the palette all use this message, so all three get the fix.
2. **A lost link says what ended it.**
   - **Now:** at the end of a relay link's stream, `Connection::read` returns the fixed text "the terminal server closed the connection". After a network cut, the log says the link "ended (the terminal server closed the connection)". In fact ssh ended it after its keepalives went unanswered (`ServerAliveInterval=5`, `ServerAliveCountMax=3`), or Brindle's own 30 s watchdog killed it. The relay process's exit status and stderr (`Wire::why`) are read only while a link is starting (`next_frame`). The same text reaches the palette's offline row ("connection lost: …").
   - **Change:** the reason names what ended the link:
     - the ssh or engine process exiting, with its status and what it wrote;
     - Brindle's dead-link limit;
     - or the relay closing the link itself.
3. **The relay warns about logins only where a login can end its server.**
   - **Now:** whenever the relay starts a core without a systemd scope, `core.log` says its sessions "may not outlive the login unless the user lingers" (`start_core`, `decide_scope`). A container without systemd has nothing that would end them, and they do outlive every `exec`.
   - **Change:** the warning is logged only when a user manager is reachable and the core still starts without a scope (the user doesn't linger, or `systemd-run` is missing). Otherwise the line says only that the core started without a scope, and why.

### When a remote connection stops trying

4. **A closed terminal's `CLOSE` keeps its connection trying for a bounded time.**
   - **Now:** a `CLOSE` queued while the link is down keeps the connection in use (`State::in_use` counts `closing`). After its first link, a relay connection retries without limit. So when the last tab on a host that is gone for good is closed, Brindle runs `ssh` every 30 s for the rest of its life, and the host's offline row never goes away. This is archived decision 9, pinned by `a_close_queued_while_disconnected_keeps_the_relay_until_it_is_written`.
   - **Change:** an unwritten `CLOSE` keeps the connection trying for at most 5 minutes after the close (decided question 1). Then the connection ends, and a log line names the sessions left running on the host.
5. **An endpoint stops being tried once its restored zones are closed.**
   - **Now:** a restoring launch waits for the first listing of each restored zone's endpoint, so it can sweep its detached sessions (`local::when_listed`). That listing hook keeps the connection trying, because `on_listing` counts in `State::in_use` and in `failed_to_connect`'s decision to keep going.
     - So an endpoint that was offline at launch is retried until it first answers, even after every zone on it has closed and their attaches were withdrawn.
     - HEAD's "One connection per host" already says the connection should end then.
   - **Change:** when the window's last tab on such an endpoint closes, and no profile connects to the endpoint at launch, the launch's wait for its listing is withdrawn. The connection then ends like any unused one, and that endpoint's sessions aren't swept in this launch. The closed tab's own session stays detached on the host, as today (decided question 5).
6. **An eager endpoint whose server refused Brindle is retried.**
   - **Now:** `run_relay` ends the connection on `Refused` (the endpoint's core answered `HELLO` with an error), even for an eager endpoint. `keep_connected` dials only endpoints that are new to the config. So the endpoint stays offline until a tab is opened on it, a reload drops it and adds it back, or Brindle restarts. "Connecting at launch" already says an eager endpoint is retried.
   - **Change:** an eager connection counts a refusal as a failed attempt and retries with the usual pauses, at most 60 s apart (decided question 2). A tab's own connection still ends on a refusal, which its tab shows.

### The sessions palette

7. **A refresh keeps the highlight.**
   - **Now:** any change in any endpoint's state, and every kill, rebuilds the rows through `Palette::set_groups`, and the command list highlights its first match again (`Command::set_groups` → `refilter`). A user who moved down to a session loses their place, and enter can then attach the wrong session.
   - **Change:** the highlighted session stays highlighted for as long as it is listed. When a session is killed, its highlight passes to the row that takes its place. `brindle-ui`'s command list gets a way for its owner to replace the items and name the one to keep highlighted.
8. **The palette opens without waiting for slow endpoints.**
   - **Now:** `refresh_sessions_palette` waits for every endpoint's listing (`join_all`) before it shows anything. A connected endpoint whose link has gone quiet takes `LIST_LIMIT` (2 s). While this machine's core restarts, its listing can take up to the local request limit.
   - **Change:** the palette opens once every endpoint has answered or a short grace has passed, whichever comes first (decided question 3).
     - An endpoint that hasn't answered yet gets a row saying its sessions are being listed. Its sessions replace that row when it answers, and the highlight stays where it was (item 7).
     - The palette doesn't say "No detached sessions" until every endpoint has answered.
9. **"No detached sessions" comes after the endpoints' rows.**
   - **Now:** when nothing is detached and an endpoint is connecting or offline, the disabled "No detached sessions" row comes first, above the endpoints' rows.
   - **Change:** it comes last.

### Restore

10. **The profile for a session on an endpoint no profile targets is specified.**
    - **Now:** this happens when a restored zone's profile now targets another host. Both the launch sweep (`Workspace::attach_swept`) and the palette's attach (`attach_detached_here`) then open the session on that endpoint, with the profile of the first tab already on it (`profile_on`), else the default profile. The specs ("Launch reattaches detached terminals" and "Sessions palette actions") say only "else the first profile that does", and here none does.
    - **Change:** both specs adopt the current rule, for both paths (decided question 4).

### Settings

11. **The profile editor's rows share one height.**
    - **Now:** each row is as tall as its control. A text field or choice is 36 px, while the "Connect at launch" switch and the "Default" checkbox are only as tall as their note, so those rows look squeezed beside the others. The labels sit in a fixed 120 px column, and on Windows (Segoe UI) "Connect at launch", the longest, doesn't wrap (the user checked on 2026-11-15).
    - **Change:** the editor lays its rows out on one height increment, half a text field's height (18 px). Every row's control line is two increments, whatever its control, and an error line beneath adds one. Linux's capture confirms that no label wraps there either. If one does, the label column is sized to the widest label as well.

### Tests

12. **Pure seams for the sessions palette and the launch sweep.**
    - **Now:** only the e2e cases cover these, with no unit tests:
      - `Workspace`'s `refresh_sessions_palette`, `show_sessions_palette`, `palette_kill`, `attach_swept`, `attach_detached_here` and `profile_on`;
      - `main.rs`'s `watch_hosts` and `sweep_launch_hosts`.
    - **Change:** move their decisions into pure functions with plain `#[test]`s. These cover:
      - which endpoints are listed;
      - how each answer becomes a listing;
      - the palette's state across answers, kills and refreshes;
      - which sessions a sweep opens, and with which profile;
      - how a batch of host reports is applied.

      The GPUI wiring stays thin.

Out of scope: `harden-terminal-server`'s items 2 and 3, which stay in that change, and README's wording about lingering.

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `remote-connections`:
  - "One connection per host": the bounded `CLOSE`, and the launch's wait for a listing no longer keeps an unused connection.
  - "Remote terminals persist": a close that can't be delivered in time.
  - "Lost connections reconnect": what ended the link.
  - "Connecting at launch": a refusing server is retried.
- `session-restore`: "Launch reattaches detached terminals", for the profile when no profile targets the endpoint, and for closed zones ending the wait.
- `command-palette`:
  - "Sessions palette": offline reasons, opening without waiting, the highlight kept, the note last;
  - "Sessions palette actions": the profile rule.
- `ui-components`: "Command filtering": an owner that replaces the items keeps the filter and the highlight it names.

Items 3, 11 and 12 change no requirement: they are a log line, a layout fix and tests.

## Impact

- `src/remote/local/relay.rs` and `src/remote/client.rs`: launch failure messages.
- `src/remote/local/connection.rs` and `link.rs`:
  - why a link ended;
  - the `CLOSE` limit;
  - retrying a refused eager connection;
  - withdrawing a listing hook.

  `src/remote/local.rs` gets the withdrawal's entry point.
- `remote-agent/src/server/relay.rs`: `decide_scope` and `start_core`'s log line.
- `ui/src/components/command.rs`: replacing the items while keeping a highlight. `src/picker.rs`: the sessions palette's state, and the highlight in the `--- palette` dump.
- `src/workspace.rs`, `src/main.rs` and `src/reattach.rs`: the palette and sweep wiring, and its seams. `src/remote/hosts.rs`: applying a batch of reports.
- `src/settings_dialog/profiles.rs`: the rows' height (and the label column, if a label wraps on Linux). `ui/`: the height increment, beside the `Input`'s height.
- Tests:
  - unit tests in the files above;
  - the e2e cases `remote-offline`, `remote-ssh-network`, `remote-container-exec`, `settings-editor-connect-at-launch` and the e2e harness's palette parser;
  - new e2e cases, listed in tasks.md.
- Docs: CLAUDE.md's "Brindle's client", "Relay", "Reattach" and `picker.rs` bullets, and its debug-hook description of the `--- palette` dump.
- Sibling: `session-layout-in-server` (outline only) would replace the sweep's profile rule if layouts move into the server. Nothing here blocks it.
