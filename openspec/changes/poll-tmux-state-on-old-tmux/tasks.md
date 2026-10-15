# Tasks

## 1. Choosing the path

- [x] 1.1 Add the pure `subscriptions_safe(version)` in `src/tmux/mod.rs`, with unit tests for `3.6`, `3.6b`, `3.7`, `3.8`, `next-3.7`, `next-3.9`, `openbsd-7.6` and the empty string; verify with `cargo test tmux::`
- [x] 1.2 Send `display-message -p "#{version}"` as `Pending::Version` before the first `list-windows`, and record the path chosen (honouring `BRINDLE_TMUX_STATE=poll|subscribe`, which wins over the version). An error reply chooses polling. `on_window_list` subscribes only on the subscription path. Verify with `cargo test`, and with `RUST_LOG=brindle::tmux=debug` against a throwaway 3.6 server, where the log shows no `refresh-client -B`

## 2. Polling

- [x] 2.1 Move the title subscription to the end of `SUBSCRIPTIONS`, and build the `list-panes -s -F` poll format from the table (window id, pane id, pane active, then each format). Update `every_subscription_is_removed` and add a format unit test; verify with `cargo test tmux::`
- [x] 2.2 Add the pure `PollState`: it parses a poll reply into per-pane and per-window values (window values from the active pane's line), returns `Notification::SubscriptionChanged` for the values that changed, records only ids it is told are known, and forgets ids that are gone. Unit tests cover the first poll sending everything, an unchanged poll sending nothing, one title change, a window value change, a title containing a tab, and an unknown pane being sent again; verify with `cargo test tmux::`
- [x] 2.3 Wire up the poll: `Pending::Poll`, the one-second timer task started when the poll path attaches, the `polling`/`poll_again` flags, an immediate poll after a user command's sentinel, and stopping on `release` and `on_detached`. Prune `PollState` in `sync_panes`. Verify with `cargo test` and a `--dump-screen-after` run against a throwaway 3.6 server with `pane-border-status top`, whose dump shows the title row and the shortened pane grid
- [x] 2.4 Update CLAUDE.md's "Leaving a session" note (the fix is in 3.7; older tmux polls instead of subscribing, plus `BRINDLE_TMUX_STATE`), and add `BRINDLE_TMUX_STATE` to the debug hooks list; verify by reading the section back

## 3. End-to-end cases

- [x] 3.1 Set `BRINDLE_TMUX_STATE=subscribe` on every run in `tmux-subscriptions-removed`, the ten repeat runs included, since the case checks the subscription path; verify `BRINDLE_E2E=1 cargo test --test e2e -- tmux-subscriptions-removed` passes on tmux 3.6
- [x] 3.2 Add `tmux-poll-session-end-inside` (with `BRINDLE_TMUX_STATE=poll` and debug logging). Keep a second session on the server (`server.run(&["new-session", "-d", "-s", "other"])`). Ten times over, run Brindle with `.send("exit\r").expect_exit()` as `session_end::close` does, check `has-session -t other`, and recreate `e2e` with `new-session -d -s e2e`. Check that no run's log contains `refresh-client -B`. Register the case in `cases/mod.rs`; it takes about a minute. Verify it passes
- [x] 3.3 Run the geometry, titles and styles cases on both paths, `BRINDLE_TMUX_STATE=poll` and then `subscribe` with `-- geometry- titles- styles-`, and verify both pass (ask the user before opening windows)

## 4. Integration

- [x] 4.1 Run `cargo test`, then the whole e2e suite with the default path (polling on 3.6). Verify everything passes, and report any failures with their artifacts directory (ask the user before opening windows)
