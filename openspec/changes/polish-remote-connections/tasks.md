# Tasks

The tasks follow design.md's recommended answer to each open question. Settle those questions with the user first; each one names the tasks that change if the answer differs.

## 1. Failure reasons (items 1 and 2)

- [ ] 1.1 Name the cause on a launch failure's first line (design decision 1):
  - add one pure helper that picks the last non-empty stderr line;
  - use it in `local::relay::start`'s `Unstarted` message (`the relay didn't start: <cause> (<program> exited with N)`, then the other stderr lines and the stream problem);
  - use it in `client::bootstrap_failure`.

  Verify:
  - unit tests in `src/remote/local/relay.rs` and `src/remote/client.rs` with multi-line stderr: docker's single line, ssh warnings before its error, and nothing on stderr (today's text unchanged);
  - a `picker.rs` test that an offline endpoint with a multi-line reason shows the cause;
  - `remote-offline` asserting that the stopped container's row contains `is not running` and the refused SSH endpoint's row contains `Connection refused` (`scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- remote-offline` on Linux).
- [ ] 1.2 Say what ended a relay link (design decision 2):
  - `Connection::read` returns how the stream ended;
  - the watchdog marks the link's `Stamp` expired before ending it;
  - `run_relay` builds the reason from the expiry, `Wire::why()`, or a clean exit, and uses it for the log line, `LinkState::Offline` and failed requests;
  - the local transport keeps its text.

  Verify:
  - `connection.rs` unit tests on the fake transport (whose wire gets a settable `why`): a process that exits with a status and stderr names them; a watchdog end says `nothing read from A8 for …`; a clean exit says `the relay closed the connection`;
  - `a_relay_reports_connecting_connected_offline_and_connected_again`'s pinned `Offline` text updated;
  - `remote-ssh-network` asserting that its `connection to … ended (` log line names `ssh exited with 255` or `nothing read from`, and never says `the terminal server closed the connection`.
- [ ] 1.3 Describe how a lost link's reason is built in `connection.rs`'s module doc and in CLAUDE.md's "Brindle's client" bullet. Verify that `grep -rn "the terminal server closed the connection" CLAUDE.md src/remote/local` finds it only for the local transport and in its tests.

## 2. The relay's scope note (item 3)

- [ ] 2.1 Make `scope::decide` say whether a user manager was found, without changing `scope::current()` for local callers. Make the relay's `decide_scope` a pure function that returns the decision and its note. Log the "may not outlive the login" warning only when a user manager is reachable and the core starts without a scope (design decision 3).

  Verify:
  - table tests in `remote-agent/src/server/relay.rs` for: scope; no linger with a user manager (warns); `systemd-run` missing with a user manager (warns); no user manager (no warning); `BRINDLE_NO_SCOPE=1` (no warning);
  - `remote-protocol/src/scope.rs`'s tests extended with the user-manager flag (`cargo test -p brindle-remote-agent relay`, `cargo test -p brindle-remote-protocol scope`);
  - `remote-container-exec` asserting that the container's `core.log` says the core started without a scope, and doesn't contain `may not outlive`.

  The Docker fixtures run no systemd, so the rows that warn are unit-tested only.
- [ ] 2.2 Update `relay.rs`'s module doc (step 2) and CLAUDE.md's "Relay" bullet on the scope line. Verify `cargo doc -p brindle-remote-agent --no-deps` builds without warnings.

## 3. What keeps a relay connection trying (items 4, 5 and 6)

- [ ] 3.1 Bound an unwritten `CLOSE` (design decision 4; open question 1):
  - add `Timings::close_limit` (5 min; tests shorten it);
  - keep the time the oldest unwritten `CLOSE` was queued;
  - stop counting the closes in `State::in_use` once the limit passes with no link up;
  - log the session ids left running.

  Verify in `connection.rs`:
  - `a_close_queued_while_disconnected_keeps_the_relay_until_it_is_written` still holds within the limit;
  - a new test: with the endpoint down past a short limit, the connection ends, its dialling stops, and the reason names the sessions.

  No e2e case: 5 minutes is too long for one, and nothing new shows in the GUI. `remote-ssh-split` keeps checking that closing a tab ends its programs and then its `ssh`.
- [ ] 3.2 Withdraw the launch's listing hook:
  - add `local::stop_listing(host)`, which drops a connection's listing hooks;
  - have `Workspace` call it when the window's last tab on an endpoint that isn't eager goes, a tab closed or its last zone;
  - add a pure `reattach` function for the endpoints that lost their last tab: the tabs' hosts before and after, and the eager hosts.

  Verify:
  - a `connection.rs` test: a hook alone keeps the connection trying, and once withdrawn, with nothing else in use, the connection ends and the hook is dropped. Update `a_listing_hook_gets_the_first_listing_and_keeps_the_relay_connecting` to match;
  - `reattach.rs` tests: an endpoint with another tab left, an eager endpoint and this machine are never named;
  - `session.rs`'s `a_tab_closed_while_its_attach_waits_for_a_relay_withdraws_it` and `a_tab_closed_while_attaching_leaves_the_session_detached` keep passing: the closed restored tab sends no `CLOSE`, and its session stays detached (open question 5);
  - a new e2e case `remote-ssh-restored-closed`:
    - a first run leaves a local tab and an SSH tab detached, and quits;
    - the SSH fixture is then stopped, so its port refuses;
    - a restoring second run shows the SSH zone connecting, then closes that tab;
    - within 15 s Brindle has no `ssh` child, and it starts none in the 10 s after that (`ssh_processes`).
- [ ] 3.3 Retry a refused eager connection (open question 2): `run_relay` ends on `Refused` only when the connection isn't eager. An eager one reports `Offline` with the refusal, fails the new tabs' waiting requests with it, and backs off up to the eager cap.

  Verify with a `connection.rs` test whose fake core answers `HELLO` with an error and then with `WELCOME`: the eager connection reports `Offline(<refusal>)`, fails a spawn with it, and is `Connected` after the next attempt. `an_eager_relay_is_offline_until_its_first_link_and_a_lazy_one_ends` keeps checking that a connection that isn't eager ends.

  No e2e case: the fixtures can't make a server refuse this build without a second, incompatible helper build.
- [ ] 3.4 Add `Hosts::apply_all(batch) -> bool` and use it in `main.rs`'s `watch_hosts`. Verify with a `src/remote/hosts.rs` test: a batch applies in order, and returns true when any report changed what the palette shows.
- [ ] 3.5 Describe the close limit, the withdrawn listing hook and eager refusals in `connection.rs`'s module doc ("A relay's first connect", "A relay's lifetime") and in CLAUDE.md's "Brindle's client" and "Reattach" bullets. Verify that `grep -n "close_limit" CLAUDE.md src/remote/local/connection.rs` finds the module doc, `State::in_use`'s comment (today's "until it is written") and the CLAUDE.md bullet.

## 4. The command list keeps a named highlight (item 7, `brindle-ui`)

- [ ] 4.1 Let `CommandRows::build` take an item to highlight, falling back to the first enabled match, and let `Command::set_groups` take it. Keep the list's scroll offset while the kept row is in view (design decision 5).

  Verify:
  - `cargo test -p brindle-ui command`: the kept item stays highlighted; a filtered-out or disabled one falls back to the first enabled match; the filter text is unchanged;
  - the gallery's `-- --check` gains the `ui-components` scenarios "Items replaced" and "Named item filtered out". It opens a window: run it under `scripts/with-xvfb` on Linux, or ask first.
- [ ] 4.2 Document the parameter in `ui/README.md`'s Command entry and on the method. Verify `cargo doc -p brindle-ui --no-deps` builds without warnings.

## 5. The sessions palette (items 7, 8 and 9)

- [ ] 5.1 Give each `SessionRow` a key, and move the palette's state into a pure model in `picker.rs` (design decision 6). The model holds:
  - each endpoint's listing or that it is pending, with its round;
  - the killed sessions;
  - the highlighted row's key;
  - pending endpoints' disabled `listing sessions…` rows;
  - the "No detached sessions" note after the endpoints' rows, shown only once nothing is pending.

  Verify with `picker.rs` tests:
  - an answer from an earlier round is dropped;
  - the highlighted session stays highlighted when an answer adds rows above it;
  - a killed row passes its highlight to the next row, else the last;
  - the note's position and when it shows. Update `connecting_and_offline_endpoints_are_rows_that_do_nothing` for the new order.

  Also verify `remote-offline` asserting that the "No detached sessions" row comes after both offline rows.
- [ ] 5.2 Add `highlighted=N|none` to the `--- palette list` dump line. Update the e2e harness's parser in `tests/e2e/brindle.rs` and its self-test, and CLAUDE.md's description of the dump. Verify `BRINDLE_E2E=1 cargo test --test e2e -- --self-test` and `cargo test -p brindle picker`.
- [ ] 5.3 Keep the model in `PaletteKind::Sessions`, apply each endpoint's answer as it arrives instead of after `join_all`, and keep the highlight through refreshes and kills. Add the pure seams `palette_endpoints` and `listing_from`, and have `listing_of` call `listing_from`.

  Verify:
  - unit tests for both seams in `workspace.rs`;
  - a new e2e case `local-server-palette-highlight`:
    - a first run leaves four local sessions detached, each showing its own marker, as `local-server-palette` does with two;
    - a second run (keeping them detached) runs `attach_session`, `--key down` and `--key ctrl-shift-delete`, and dumps: the killed session's row is gone, and `highlighted=1` is on the row that was listed third;
    - a third run does the same, then `--key enter`, and dumps: the active terminal shows the marker of the session that was listed third in that run;

    Each run takes one dump, since a dump ends the run.
  - `local-server-palette` keeps passing.
- [ ] 5.4 Open the palette when every endpoint has answered or after `OPEN_GRACE` (about 250 ms), whichever comes first (open question 3).

  Verify:
  - model tests: a palette whose endpoints all answer within the grace opens complete; otherwise it opens with pending rows; the note never shows while an endpoint is pending;
  - a new e2e case `remote-palette-slow-host`:
    - an SSH profile connects at launch to the fixture, and the container is paused (`docker pause`) once Brindle is connected;
    - `attach_session`, with a dump within 1 s of it;
    - the dump shows this machine's rows and the endpoint's `listing sessions…` row, before `LIST_LIMIT` (2 s) passes.
- [ ] 5.5 Update CLAUDE.md's `picker.rs` and "Reattach" bullets ("concurrently… within `LIST_LIMIT`") and the doc comments of `refresh_sessions_palette` and `session_palette`. Verify that `grep -rn "OPEN_GRACE" CLAUDE.md src/workspace.rs` finds the CLAUDE.md bullet and `refresh_sessions_palette`'s doc, and that `grep -n "inert first row above" src/picker.rs` finds nothing.

## 6. A session's profile on an endpoint (item 10)

- [ ] 6.1 Add `reattach::session_profile` and `reattach::swept_tabs` (design decision 7; open question 4). Make `attach_swept` and `attach_detached_here` use them, passing the profile name of the first tab on the endpoint; `profile_on` keeps only that lookup.

  Verify with `reattach.rs` tests:
  - the profile the session started with, when it targets the endpoint;
  - else the first profile that targets the endpoint;
  - else the first tab's profile when no profile targets the endpoint;
  - else the default, when that tab's profile is gone;
  - local sessions unchanged;
  - `swept_tabs` keeping the sweep's order with each session's profile.

  No new e2e case: the dump doesn't show a tab's profile, and the choice is the pure function's. `remote-ssh-connect-at-launch` keeps passing.
- [ ] 6.2 Update CLAUDE.md's "Reattach" bullet and the doc comments of `attach_swept`, `attach_detached_here` and `profile_on` to the one rule. Verify that `grep -rn "a restored one, for a launch" src` finds nothing.

## 7. The profile editor's label column (item 11)

- [ ] 7.1 Confirm the wrap.
  - **Linux:** extend `settings-editor-connect-at-launch` with a window capture of an ssh profile's editor, saved in the case directory, and run it with `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- settings-editor-connect-at-launch`.
  - **Windows:** the suite can't capture there. Hand the user `scripts/run-throwaway.ps1 -Config <file>` with an ssh profile, and ask whether "Connect at launch" wraps.

  Verify: record here, from both checks, whether any label wraps.
- [ ] 7.2 If a label wraps on either platform, size the label column to the widest label at the dialog's text size, make labels `whitespace_nowrap`, and indent errors to match (design decision 8).

  Verify:
  - `settings-editor-connect-at-launch` checks its capture: the label column's text in the "Connect at launch" row forms a single line;
  - the user's Windows check again;
  - `cargo test -p brindle settings_dialog`.

  If no label wraps, mark this task done as not needed, citing 7.1.

## 8. Platform checks

- [ ] 8.1 On Windows, run `cargo test --workspace --locked` and `cargo build --release --locked` and confirm both pass, along with the e2e and perf self-tests (`BRINDLE_E2E=1 cargo test --test e2e -- --self-test`, `BRINDLE_PERF=1 cargo test --test perf -- --self-test`) and `BRINDLE_E2E=1 cargo test --test e2e -- --list`. With the user's go-ahead (it opens windows on the desktop), also run `BRINDLE_E2E=1 cargo test --test e2e -- local-server-palette`, which includes `local-server-palette-highlight`. Record the results here.
- [ ] 8.2 Ask the Linux machine's session to run, at the pushed branch:
  - `cargo test --locked` and `cargo build --release --locked`;
  - `cargo build -p brindle-remote-agent`, then `scripts/with-xvfb env BRINDLE_E2E=1 BRINDLE_E2E_SLOW=2 BRINDLE_REMOTE_E2E_REQUIRED=1 cargo test --test e2e`, and `-- remote` three times. `BRINDLE_REMOTE_E2E_REQUIRED=1` stops a missing Docker daemon from passing as a skip;
  - `BRINDLE_REMOTE_DOCKER_TEST=1 cargo test -p brindle --locked remote::client`, after rebuilding the static helper with README's Docker command.

  Record the results here. The only warning allowed is the known `proc-macro-error2` / `num-bigint-dig` future-incompatibility note.

## Workflow follow-up

- Settle design.md's open questions with the user before implementing. If an answer differs from the recommendation, update the spec text and the tasks it names first (`openspec-update-change`).
- Run `openspec validate polish-remote-connections --strict` before opening the PR.
- Archive the change after review (`openspec archive polish-remote-connections`).
