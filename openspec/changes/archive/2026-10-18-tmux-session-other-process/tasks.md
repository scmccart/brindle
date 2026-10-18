# Tasks

## 1. The failing cases first

- [x] 1.1 `tests/e2e/cases/multi_window.rs` (already on the branch): `multi-window-local-tabs`, `multi-window-tmux-commands` and `multi-process-local` pass, and `multi-process-one-client` and `multi-process-independent` fail with two clients on the session. Verified with `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- multi-`
- [x] 1.2 Extend `multi-process-one-client` to check the second Brindle's dump: no tabs, and a palette whose placeholder is `tmux session "e2e" is open in another Brindle` and whose rows are "New tmux session…" then "New tab: Shell". Add `multi-process-dismiss` (escape in the second Brindle's tab-less window: it exits by itself, so `expect_exit`), `multi-process-shortcut` (a second Brindle on `Shell` presses `new_tab_with_profile_2`: it asks, keeps its local tab, and one client remains), `multi-process-other-client` (a raw `tmux -C` client attached first, via `ControlClient`, doesn't stop Brindle attaching), and `multi-process-new-window` (ctrl-shift-n in a second Brindle on `Shell`: the new window asks, and the dump's `--- window 2` line has no tabs). Verify that the new and extended cases fail before section 2's fix, apart from `multi-process-other-client`, which passes

## 2. Detecting another Brindle's client

- [x] 2.1 In `src/tmux/mod.rs`, spawn control clients with `BRINDLE_CONTROL_CLIENT=<own pid>`, and add `list_clients_command(profile, name)` (beside `has_session_command`) and the pure `other_brindle_client(pids, own_pid, read)`, with `read` giving a pid's parent pid and environment. Add `/proc` readers that parse `stat` after its last `)`. Unit tests cover a matching client, our own pid, a marker that doesn't match the parent (one inherited in a pane), a missing marker, unreadable entries, and the command line. Verify with `cargo test tmux::`
- [x] 2.2 Split `TmuxSession::attach` into the check and `start`: `AttachSpec::Named` runs `list-clients` on the background executor, then starts or emits `TmuxEvent::ShownByAnotherBrindle(name)`. `AttachSpec::New` starts at once. Check that `release`, `Drop` and the quit hook do nothing before `start`. Verify with `cargo test` and that `multi-window-*`, `session-*`, `new-window-*`, `restore-*` and `tmux-*` still pass under `scripts/with-xvfb`
- [x] 2.3 Note the marker, and its leak into the panes of a server it started, in the `tmux/` section of `CLAUDE.md`. Verify by reading it back

## 3. Asking in the window

- [x] 3.1 In `src/workspace.rs`, handle `ShownByAnotherBrindle` like `Failed` without the message tab, and ask with a new `SessionLocation::OtherBrindle(name)`: target `Tab`, placeholder `is open in another Brindle`, and `session_open_rows` without "Go to" (a flag). Add a unit test beside `session_open_rows_offer_going_there_first`. Verify with `cargo test workspace` and that `multi-process-one-client`, `multi-process-independent`, `multi-process-shortcut` and `multi-process-new-window` pass
- [x] 3.2 When the ask palette, or a prompt it opened, is dismissed, and after it opens a local tab, close the tabs of sessions that are no longer attached and that no `replacing` entry waits on. Then close the window if it has no tabs and no session attaching. "New tmux session…" hands such an ended session to the new session through `replacing`. Verify that `multi-process-dismiss` passes, and add `multi-process-session-ended` (`tmux_session_end = "ask"`, a session that ends in the second Brindle, "New tab: tmux" for `e2e`, which the first shows, then "New tab: Shell": one `Shell` tab remains) and check that it passes

## 4. Integration

- [x] 4.1 Run `cargo test` and the whole e2e suite under `scripts/with-xvfb` (no windows on the desktop). Verify everything passes, and report any failures with their `target/tmp/e2e/<case>/` directory
- [x] 4.2 Run `openspec validate tmux-session-other-process --strict` and verify it reports the change as valid
- [x] 4.3 Manual check for the user, on their desktop: launch Brindle twice from the launcher with the default `tmux` profile. The second window asks instead of mirroring the first, and escape closes it
