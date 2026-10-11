# Tasks

## 1. Setting and state model

- [x] 1.1 Add `restore_session: bool` (default `false`) to `Config`, plus a commented entry in `assets/default-config.toml`. Verify with `cargo test config`: `default_config_file_parses` still passes, and a new test parses `restore_session = true`.
- [x] 1.2 Create `src/session.rs` with the state structs (`State`, `WindowState`, `TabState` with `local`/`tmux` kinds), `parse` (version check first; corrupt, unknown-version and empty states are errors with a reason), `to_text`, the `state_dir()` resolution (`BRINDLE_STATE_DIR`, then `dirs::state_dir()/brindle`, then `~/.local/state/brindle`), and `write_atomic` (temp file, `sync_all`, rename, 0600). Verify with `cargo test session`: a round trip, version 2 rejected, garbage rejected, an unknown kind rejected, and an atomic write into a temp dir leaving no `.tmp` behind.
- [x] 1.3 Add `resolve_tab(entry, &Config)` with the fallbacks from design Decision 6 (missing profile → default; directory → recorded, else profile cwd, else home; tmux entry with a non-control profile → local launch). Verify with unit tests for each fallback.
- [x] 1.4 Add `Lock::acquire(dir)` (`flock` `LOCK_EX|LOCK_NB` on `session.lock`, held while the value lives). Verify with a unit test that a second acquire in the same process, through a separate open file description, fails while the first is held and succeeds after it is dropped.

## 2. Recording

- [x] 2.1 Add `TabOrigin` to `Tab`, set in `launch`, `on_tmux_event` and `open_message_tab`. Add `Workspace::snapshot(window, cx) -> WindowState`: local tabs with profile and `/proc` cwd, one tmux entry per session at its first tab, message tabs skipped, the active index mapped onto the recorded entries, and the window size and maximized state. Verify with `cargo build` and a unit test of the pure entry-collapsing helper (tabs of one session become one entry, and the active index follows).
- [x] 2.2 Add the `SessionStore` global (snapshots keyed by window id plus creation order, last written text, owner lock). Store snapshots from each workspace's 1 s poll when the setting is on and the lock is held. Forget the window when its last tab is removed. Prune closed windows in `on_window_closed` only while other windows remain. Verify by unit-testing the store's ordering and prune rules.
- [x] 2.3 In `main.rs`, add the 1 s writer task (serialize, compare, background atomic write) and an `on_app_quit` hook that re-snapshots live windows and writes synchronously. Take the lock lazily once the setting is on. Verify with the e2e round trip in 4.2.

## 3. Restoring

- [x] 3.1 Add `--no-restore` to `parse_args` and `HELP`. Compute "plain launch". At startup, with the setting on, the lock held and a usable state, open one window per recorded window with its size or maximized state; otherwise launch as today. Log and ignore a bad state. Verify that `brindle --help` lists `--no-restore` and that unknown-argument handling still exits 2.
- [x] 3.2 `Workspace::new` takes `Launch::Request | Launch::Restore`. Restoring launches local entries with explicit directories and attaches each tmux entry once, recording its anchor tab id and whether it's quiet. Then it activates the recorded local active tab. `WindowAdded` honours the anchor for a session's first tab, and `WindowActivated` skips once for quiet sessions. Verify with the e2e cases in 4.2–4.4.
- [x] 3.3 Document `restore_session`, `--no-restore`, the state file location and what is not restored in README.md (usage and a short "Session restore" section). Verify that the README's usage block matches `--help`.

## 4. End-to-end checks

- [x] 4.1 Extend the e2e `Run` builder: a plain launch (no `-p`), extra config lines, and a `BRINDLE_STATE_DIR` per run in the case directory. Keep the existing cases' behaviour unchanged. Verify that `--self-test` and `-- --list` still work.
- [x] 4.2 `restore-local`: run 1 is a plain launch with restore on. It `cd`s into a temp dir A, opens a new tab, `cd`s into dir B, goes back to the first tab, and the dump quits it. Check `session.toml` (two local tabs, cwd A and B, active 0). Run 2 is a plain launch that sends `pwd`. Check two tabs, the first active, A on screen, and an unchanged size. Verify with `flock /tmp/brindle-gui.lock env BRINDLE_E2E=1 cargo test --test e2e -- restore-`.
- [x] 4.3 `restore-fallbacks`: a hand-written state with a missing profile and a deleted directory restores the default profile in home (checked with `pwd`). A corrupt state file and a version 99 file each give a normal single-tab launch. `--no-restore` gives a single tab. Verify with the same command.
- [x] 4.4 `restore-tmux`: a throwaway server whose session has three windows, with window 2 current and window 0 last. A hand-written state has a local Shell tab followed by the tmux session, with the local tab active. Check:
  - after a plain launch: four tabs, in order Shell then three windows, the Shell tab active;
  - tmux's current and last windows are unchanged;
  - the rewritten state has exactly one tmux entry.

  A second run with the tmux entry active makes tmux's current window the active tab. Verify with the same command.
- [x] 4.5 `restore-forget-on-exit` (the only tab exits, so the state records no windows) and `restore-second-instance` (a second process neither restores nor records, and says so in its log). Verify with the same command.
- [x] 4.6 `restore-close-window`: `close_window` on the only window, which still has a tab, quits and keeps the tab and its directory for the next launch. `restore-forget-on-exit` also checks that the launch after an `exit` is a normal one and logs no "ignoring session state". Verify with the same command.

## 5. Integration

- [x] 5.1 Run `cargo build`, `cargo test`, `cargo build --release`, and the full e2e suite under `flock /tmp/brindle-gui.lock`. Verify that every command passes and `git diff Cargo.lock` is empty.
