# Tasks

## 1. Unsubscribing

- [ ] 1.1 Replace the inline subscription list with a `SUBSCRIPTIONS` table and pure `subscribe_commands()` / `unsubscribe_commands()` (design.md decision 1). Unit-test that every subscription gets a matching removal (`refresh-client -B "<name>"`, no colon). Verify with `cargo test tmux`.
- [ ] 1.2 Add `release()` (unsubscribe, then `detach-client`, once) and use it in `detach()` and `Drop` (decision 2). Verify with a clean `cargo build`.
- [ ] 1.3 Register an app-quit hook that calls `release()` and waits, with a 50 ms cap, for the writer thread to acknowledge a flush (decision 3). Verify with a clean `cargo build`.
- [ ] 1.4 Unsubscribe before `kill-window` of a session's only window and before `kill-pane` of its only pane (decision 4). Verify with a clean `cargo build`.
- [ ] 1.5 Note in `CLAUDE.md`'s tmux notes: subscriptions are removed before detaching, quitting or ending a session because tmux 3.6 can crash otherwise, and sessions ended from inside tmux remain a known gap. Verify the note matches the code.

## 2. Verification

- [ ] 2.1 Add the e2e case `tmux-subscriptions-removed`. Using Brindle's debug log (`RUST_LOG=brindle::tmux=debug`), check that every subscription's removal is sent after its subscription and before Brindle's last command, for three exits: a quit (`--dump-screen-after`), `--action tmux_detach`, and `--action close_tab` on the only window. For the quit, also check that the tmux server is still running afterwards, and repeat the quit 10 times. Run it with the user's go-ahead.
- [ ] 2.2 Run `cargo test`, then the whole e2e suite with the user's go-ahead, and `openspec validate unsubscribe-before-tmux-detach --strict`.
