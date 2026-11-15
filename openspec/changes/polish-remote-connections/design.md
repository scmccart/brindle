# Design

## Context

See proposal.md for the items and what each one observes at HEAD. The parts of the current code that shape the approach:

- **A relay connection's keep-alives.** Three places decide whether a relay connection keeps trying, and they don't agree:
  - `State::in_use` counts sessions, requests, listing hooks (`on_listing`) and unwritten `CLOSE`s (`closing`). The idle thread ends the connection 5 s after this turns false, unless the connection is eager.
  - `failed_to_connect` keeps trying before the first link while it is eager, an attach waits, or a listing hook waits. After the first link it always keeps trying.
  - `run_relay` ends the connection on `Refused`, whatever the other two say.
- **Why a link ended.** `Connection::read` turns the end of the stream into fixed text. `Wire::why()` (a relay process's exit status and its kept stderr) is read only in `next_frame`, while a link is starting. The watchdog ends a dead link by killing its process, so `why()` after that reports the kill and not the silence.
- **The sessions palette.** `picker::session_palette` is pure and tested. `Workspace` wires it:
  - It lists each endpoint on a background task, and waits for all of them with `join_all`.
  - It drops a round that a later one replaced (`sessions_listing`).
  - It rebuilds the command list with `Palette::set_groups`, which highlights the first match again.

  `brindle-ui`'s `CommandItem` has no identity, and by the crate's boundary it knows nothing of sessions.
- **The profile for a remote session.** `reattach::remote_profile` returns `None` when no profile targets the endpoint. `attach_swept` and `attach_detached_here` then both fall back to `profile_on`: the first tab on the endpoint, whichever tab that is, else the default.
- **Tests and the GUI.** The repo's tests are plain `#[test]`, with no GPUI test context. GUI behaviour is checked through the debug hooks and the e2e harness, whose dump parser has its own self-tests. `render` must stay free of allocation-heavy work (CLAUDE.md).

## Goals / Non-Goals

**Goals:**
- Every failure a user reads names its cause on its first line, which is the only line a palette row shows.
- A relay connection keeps trying only while a terminal uses it or it is eager. The one exception, an undelivered `CLOSE`, is bounded.
- The sessions palette never loses the user's place, and never waits for its slowest endpoint.
- The decisions in the palette and sweep wiring are pure functions with unit tests.

**Non-Goals:**
- Changing the reconnect backoff, the dead-link limit, `LIST_LIMIT` or the owner grace.
- Recording undelivered closes across launches (decided question 1, alternative B).
- Specifying `core.log`'s wording.
- `harden-terminal-server`'s items 2 and 3, and README's wording about lingering.

## Decisions

### 1. A failure's first line names its cause

`local::relay::start` builds an `Unstarted` message. When the process wrote to stderr, the message becomes `the relay didn't start: <cause> (<program> exited with N)`.
- `<cause>` is the last non-empty line of what the process wrote.
- The remaining stderr lines follow, and then the stream problem (`the stream ended before the frame marker…`, or `no frame marker within 60 s`). So a tab still shows everything it shows today.
- With nothing on stderr, the first line stays as it is now.

`client::bootstrap_failure`, which covers ssh's 255 and the deploy steps, uses the same summary: `<what>: <cause>`, with the other lines below. The sessions palette keeps showing the first line.

Why the last line:
- docker and podman print one line;
- ssh prints its warnings (`Warning: Permanently added …`) before its error;
- a login script that prints before a failing `exec` ends with the `exec`'s error.

Alternatives considered:
- Joining every line into the row. Rejected: the generic text would still come first, and the row truncates.
- Recognising known engine messages. Rejected as brittle.

One pure helper picks the cause. Its tests use multi-line fixtures, which is what the current ones lack.

### 2. What ended a relay link

`Connection::read` returns how the stream ended (closed, or an I/O error) instead of text. `run_relay` then builds the reason, in this order:
1. **Watchdog:** if the watchdog ended this link, the reason is `nothing read from <name> for 30 s`. The watchdog marks the link's `Stamp` as expired before it calls `wire.end()`. That is how the reader tells a silent link from a process that died by itself.
2. **The process exited:** otherwise the reason is `wire.why()`, for example `ssh exited with 255: Timeout, server A8 not responding.`
3. **Clean close:** a process that exited 0 and wrote nothing gives `the relay closed the connection`. The relay exits when its core connection closes.

The local transport keeps "the terminal server closed the connection", which is accurate for the core's socket or pipe.

Where the reason goes:
- The log line keeps its prefix, `connection to <name> ended (`, which `remote-ssh-network` polls.
- `LinkState::Offline("connection lost: …")` gets the same reason, and so does `disconnected`'s "the terminal server went away (…)".

ssh logs its keepalive timeout at INFO (`logit`). With the e2e profiles' `LogLevel=ERROR`, only `ssh exited with 255` is left; with ssh's default logging, users also get "Timeout, server … not responding.". Which comes first depends on timing: ssh gives up after 15–20 s, the watchdog after 30 s. So the e2e check accepts either one, and never "the terminal server closed the connection".

Alternative considered: reading `why()` only for the log line. Rejected, because the palette row and the failed requests should give the same reason.

### 3. The relay's scope note

Two facts decide the warning:
- whether a user manager is reachable (`$XDG_RUNTIME_DIR/systemd/private`, which `scope::decide` already checks);
- whether the core gets a scope of its own.

The note:
- **Warning:** "its sessions may not outlive the login if the host sets `KillUserProcesses=yes`; `loginctl enable-linger` keeps them". Logged only when a user manager is reachable but the core starts without a scope, because the user doesn't linger or `systemd-run` is missing.
- **Plain line:** with no user manager (containers, hosts without systemd) or with `BRINDLE_NO_SCOPE=1`, the line says only that the core started without a scope, and why.

`scope::decide` returns `Plain` with its reason today. It gains a way to say whether a user manager was found, and `scope::current()` stays the same for local callers. `decide_scope` in the relay becomes a pure function of that and of lingering, returning the decision and the note together.

No spec change: `core.log`'s lines aren't specified, and "A server started by a relay outlives the connection" is unchanged.

### 4. What keeps a relay connection trying

One rule replaces the three places listed in Context. A relay connection keeps trying, and stays in use, while any of these holds:
- **A terminal uses it:** it shows a session, or a request waits on it (spawn, attach, list, kill). As now.
- **It is eager:** a profile connects to it at launch. As now.
  - A refusal no longer ends an eager connection. It reports `Offline(<refusal>)`, fails the new tabs' requests waiting on it with that message (as a failed first attempt does), and backs off up to the eager cap (decided question 2).
  - A connection that isn't eager still ends on a refusal, so its tab shows the refusal at once.
- **A `CLOSE` hasn't been written yet,** for at most `Timings::close_limit` (5 min) after it was queued (decided question 1).
  - `State` keeps the time the oldest unwritten `CLOSE` was queued. Once that passes with no link up, the connection stops counting them, and it logs `not ending session(s) <ids> on <name>: it couldn't be reached within 5 minutes of the close`.
  - The idle rule then ends it: the relay process is killed, and the writer's queue goes with the connection.
  - If a link comes up in time, the `CLOSE`s are written as now.
  - Only a running session gets a `CLOSE`. A tab closed while its spawn or attach is still waiting withdraws the request (`Session::leave`, `Abandon`), as now: a spawn starts nothing, and an attach leaves its session detached on the host. So closing a restored tab whose host can't be reached queues nothing, and only the listing hook below was keeping its connection (decided question 5).
- **The launch's listing hook,** until it is withdrawn.
  - When the window's last tab on an endpoint goes (a tab closed, or its last zone), and the endpoint isn't eager, `Workspace` calls a new `local::stop_listing(host)`. It drops that connection's hooks, which ends the sweep's wait for that endpoint.
  - The pure part, which endpoints just lost their last tab, is a function of the tabs' hosts before and after the close, and the eager hosts.

Alternative considered: hooks never keep a connection, and ride on the restored zones' attaches. Rejected because of a race at launch. The sweep's hook and the zones' attaches are registered from different threads. If the first attempt failed before an attach was registered, the connection would end and the sweep would skip that endpoint without a word. The UI thread knows exactly when the last tab on an endpoint goes, so withdrawing from there is deterministic.

Each case gets a test on `connection.rs`'s fake transport. The tests that pin today's behaviour (`a_close_queued_while_disconnected_keeps_the_relay_until_it_is_written`, `a_listing_hook_gets_the_first_listing_and_keeps_the_relay_connecting`, `an_eager_relay_is_offline_until_its_first_link_and_a_lazy_one_ends`) are updated, not removed.

### 5. The command list keeps a named highlight

`brindle-ui`:
- `CommandRows::build` takes an optional item index to highlight. It falls back to the first enabled match when that item is filtered out or disabled.
- `Command::set_groups` gains the index to keep. While the kept row stays in view, the list keeps its scroll offset instead of returning to the top.
- The crate still knows nothing of sessions: the owner names an index into its new items.

The owner maps identities:
- Each `picker::SessionRow` gets a key: endpoint, id and key for a session; the endpoint for an endpoint's row; a constant for the note.
- After an update, the palette finds the highlighted row's key among the new rows.
- When that row is gone (a kill), the highlight goes to the row now at its old position, else the last row, skipping disabled rows.

Alternative considered: an `id` on `CommandItem` that `brindle-ui` matches itself. Rejected for now, since it widens a public type for one owner. Worth revisiting if another palette needs it.

The `--- palette list` dump line gains `highlighted=N|none`: the highlighted row's index among the dumped rows, from 0. The e2e harness's parser and its self-test are updated with it.

### 6. The sessions palette's state is a pure model

A model in `picker.rs` holds the palette's state:
- each endpoint's listing, or that it is still pending;
- the round of listings it belongs to;
- the killed sessions;
- the highlighted row's key.

It has three operations: an answer arrives, a session is killed, and a view is built for the config and the time (groups, row targets, the empty message, the index to highlight). `Workspace` keeps the model in `PaletteKind::Sessions`, spawns the listings, and applies each view.

- **Answers:** they apply one at a time as they arrive, instead of after `join_all`. An answer from an earlier round is dropped, as `sessions_listing` does now.
- **Opening** (decided question 3): the palette opens when every endpoint in the round has answered, or after `OPEN_GRACE` (about 250 ms), whichever comes first.
  - An endpoint still out, this machine included, gets an inert row: `<name>  listing sessions…`.
  - The "No detached sessions" note shows only once nothing is pending, and it comes after every endpoint's row (item 9).
- **Seams** (item 12):
  - `palette_endpoints(states)`: this machine first, then the `Hosts` endpoints in their order;
  - `listing_from(host, state, result)`: what `listing_of` decides, without its I/O.

### 7. One rule for a session's profile on an endpoint

`reattach::session_profile(info, host, config, first_tab_profile)` returns a profile index:
- **This machine:** `local_profile`, as now.
- **A remote endpoint:** the first of these that exists:
  - the profile the session was started with, when it targets the endpoint;
  - the first profile that targets the endpoint;
  - `first_tab_profile`, by name;
  - the default profile.

`attach_swept` and `attach_detached_here` both call it. `Workspace` passes the profile name of the first tab on the endpoint, which is the only lookup that needs GPUI. `reattach::swept_tabs(listed, skip, host, config, first_tab_profile)` joins `sweep` with this choice, so `attach_swept` only inserts tabs. This is decided question 4, which keeps today's behaviour, now specified.

### 8. The profile editor's rows share one height increment

The user asked on 2026-11-15 for a standard height for the profile editor's fields. It replaces the earlier plan, which only made the label column fit. On Windows no label wraps (the user's check), so that plan was not needed there.

- `brindle-ui` gets one increment, `FIELD_STEP` (18 px), half of `Input`'s 36 px height (`h_9`). `Input` is defined in terms of it, so the two can't drift apart.
- Every profile editor row's control line is `2 × FIELD_STEP` tall, with its control centred: text fields, choices, the "Connect at launch" switch and the "Default" checkbox alike. An error line beneath a row takes one more `FIELD_STEP`, with its text centred in it, so the rows below it move by a whole increment.
- The gap between rows stays as it is (`gap_1`); the increment governs rows, not the space between them.
- Label wrap: Linux's capture (`settings-editor-connect-at-launch`) is still checked. If a label wraps there, the label column's width becomes the widest label's shaped width at the dialog's text size, measured when the editor opens, labels become `whitespace_nowrap`, and the error line's indent follows the column.

Alternatives considered:
- Padding only the switch and checkbox rows to 36 px: it fixes today's two rows but leaves the next control to drift again, and error lines would still move the rows below by odd amounts.
- An increment of a full 36 px: an error line would add a whole text field's height of mostly empty space.
- A height for every control in `brindle-ui` (checkbox, switch): they are drawn at their own sizes elsewhere (the theme list, the gallery), and only the editor's rows need to line up.

### 9. Pure seams for the wiring

Item 12's functions, each with `#[test]`s in its own module:
- `palette_endpoints` and `listing_from` (decision 6);
- the palette model (decisions 5 and 6);
- `session_profile` and `swept_tabs` (decision 7);
- the endpoints that lost their last tab (decision 4);
- `Hosts::apply_all(batch) -> bool`, the fold `main.rs`'s `watch_hosts` runs over a batch of reports.

`launch_hosts` and `eager_hosts` already have tests. What is left in `Workspace` and `main.rs` is spawning work and applying the results.

## Risks / Trade-offs

- [A `CLOSE` given up after 5 minutes leaves a `detach` program running on the host, still owned. A later launch's sweep reopens it after the 5-minute owner grace, though the user closed it.] → The log names the sessions, and the palette can kill them. Open question 1's alternative B fixes this if it turns out to matter.
- [Opening the palette before every endpoint answers moves rows under the user.] → The highlight is kept by identity (decision 5). A pending endpoint's row is disabled and sits where its sessions will appear. The grace opens the palette complete in the usual case, where every endpoint answers within milliseconds.
- [Existing e2e cases press keys one step after `attach_session`: 1 s, or 2 s under `BRINDLE_E2E_SLOW=2`.] → The grace is far below one step, and local listings answer in milliseconds.
- [An eager endpoint retried every 60 s against a refusing server adds one refused `HELLO` a minute to that server's log.] → It stops once a server accepts. Open question 2's alternative B lengthens the pause.
- [The stderr summary takes the last line, which is noise when a program prints after its error.] → The tab still shows the full stderr on the lines after the summary. The heuristic only affects the first line.
- [After the last tab on a restored endpoint closes, the launch's sweep of that endpoint is withdrawn, even if the user opens a new tab there right away.] → Its sessions stay in the sessions palette.

## Migration Plan

None. Nothing in the protocol, the config or the state file changes. The `--- palette list` dump line gains a field, and the e2e harness's parser is updated in the same change.

## Decided Questions

The user chose the recommendation for all five on 2026-11-15: 1A, 2A, 3B, 4A and 5A. The specs and tasks are written to these answers. Each question still names the text that would change under another answer, so a later move (to 1B and 5B together, say) knows what to touch.

1. **What becomes of a `CLOSE` that can't be delivered?** (item 4)
   - **A.** It keeps the connection trying for 5 minutes after the close, then is dropped with a log line naming the sessions.
   - **B.** As A, and the undelivered closes are also recorded in `session.toml`. The next launch that reaches the endpoint then ends those sessions, by id and key, instead of sweeping them back. This also covers quitting with a `CLOSE` still queued, which today is dropped silently. It needs a state file addition, those endpoints in `launch_hosts`, and `session-restore` text.
   - **C.** It keeps the connection trying for the process's life, as now.
   - **Recommendation: A.** It is the smallest change that ends the connection, as "One connection per host" says. The 5 minutes cover the short outages the keep-alive exists for: a Wi-Fi switch, a VPN reconnect, a suspend. Move to B if leftover sessions turn out to be a nuisance.
   - **If not A:** B adds a `session-restore` requirement, and changes "Remote terminals persist"'s close bullet to "…ends the next time a launch reaches the host". C drops the close-limit text and the "Host gone for good" and "Closed while the host is away too long" scenarios from `remote-connections`, and task 3.1.
2. **How does an eager endpoint retry after its server refused Brindle?** (item 6)
   - **A.** Like any failed attempt, at most 60 s apart.
   - **B.** With a longer pause, such as 10 minutes, since a refusal clears only when the host's server changes.
   - **C.** Only on an event: a config reload, or opening the sessions palette.
   - **Recommendation: A.** "Connecting at launch" already says an eager endpoint is retried with pauses at most 60 s apart. A refusal can clear at any moment: the refusing server exits 5 s after its last session ends, and the next relay starts this build's server.
   - **If not A:** B or C changes the "Server refuses" scenario's "within a minute of the refusing server exiting" and task 3.3's backoff, and C also changes "Connecting at launch"'s retry sentence.
3. **When does the sessions palette open?** (item 8)
   - **A.** At once, with a row for each endpoint that is still listing.
   - **B.** When every endpoint has answered, or after a grace of about 250 ms, whichever comes first. Endpoints still out get a "listing" row.
   - **C.** As now, after every endpoint has answered, with a shorter `LIST_LIMIT`.
   - **Recommendation: B.** In the usual case the palette opens complete, with no rows flickering in, and it is never more than a moment late.
   - **If not B:** A reads the same in the spec, and only decision 6's `OPEN_GRACE` goes. C drops "SHALL open without waiting…", the "being listed" row, the "Slow host" scenario and task 5.4.
4. **Which profile does a session on an endpoint that no profile targets get?** (item 10)
   - **A.** The profile of the first tab on that endpoint, else the default profile. The tab runs on the endpoint either way.
   - **B.** The launch doesn't sweep such sessions, which stay in the palette. The palette's attach uses A.
   - **C.** Always the default profile.
   - **Recommendation: A.** It is today's behaviour, it fits "A tab stays on its host", and it keeps the theme and font the user had on that endpoint.
   - **If not A:** B changes "Launch reattaches detached terminals" to leave out endpoints that no profile targets, and replaces its "Endpoint no profile targets" scenario. C changes the profile lists in both "Launch reattaches detached terminals" and "Sessions palette actions". Either changes task 6.1's rule and its tests.
5. **What does closing a tab do to a session it was still attaching?** (item 5, found while tracing it)
   - **A.** Nothing reaches the host: the attach is withdrawn and the session stays detached there, as now. A later launch that connects to the host sweeps it back, and the sessions palette lists it while Brindle is connected there.
   - **B.** Brindle queues a `KILL` for it under the same 5-minute limit as a `CLOSE`, so closing a tab always ends its program when the host comes back in time. `KILL` names a session by id alone, and ids restart in a fresh core, so this needs the session's key in `KILL` (an additive protocol field) to be safe.
   - **Recommendation: A.** The user never saw that session's screen in this run, a protocol change is out of proportion for this change, and the session can't be lost: it is listed and swept like any detached one.
   - **If not A:** B changes "Remote terminals persist"'s close bullet and the "Restored tab closed before its host answers" scenario in both `remote-connections` and `session-restore`, adds a `KillTarget` field to `remote-protocol` with transcript tests, and adds a task to group 3.
