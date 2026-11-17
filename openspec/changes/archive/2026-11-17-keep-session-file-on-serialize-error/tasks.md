# Tasks

## 1. Pure code: the write decision (`src/session.rs`)

- [x] 1.1 Write the tests first, in `src/session.rs`'s `mod tests` (they fail to compile until 1.2 and 1.3 land). For `next_write`, build errors with `<toml::ser::Error as serde::ser::Error>::custom(..)`, with no platform code:
  - `an_unserializable_state_is_reported_once_and_never_written`: a first `Err` gives `PendingWrite::Unserializable(e)`, the same `Err` again gives `StillUnserializable`, and `last_written` is unchanged after both.
  - `a_different_failure_is_reported_again`: `Err(a)` then `Err(b)` gives `Unserializable(b)`.
  - `the_text_last_written_survives_a_failure`: after `Ok("a")` (`Write`) and an `Err`, `Ok("a")` gives `Nothing` and `Ok("b")` gives `Write("b")`.
  - `a_failure_after_a_success_is_reported_again`: `Err(e)`, `Ok("a")`, `Err(e)` gives `Unserializable(e)` the second time.
  - `a_directory_that_isnt_utf8_leaves_the_file_alone`: a `SessionStore` holding a `Lock` in its own temporary directory, recorded with `window(1)` whose tab `cwd` is `unserializable_dir()`. `pending_write()` gives `Unserializable` and then `StillUnserializable`. Recording that window again with `cwd: Some("/tmp".into())` gives `Write(text)`, and `State::parse(&text)` has that `cwd`.
  - The helper `unserializable_dir() -> PathBuf`: `std::os::unix::ffi::OsStrExt` on `b"/tmp/\xff"` under `#[cfg(unix)]`, `std::os::windows::ffi::OsStringExt::from_wide(&[0xD800])` under `#[cfg(windows)]`. This is test code, outside `ports R4` by decision D1.
- [x] 1.2 Change `State::to_text` to return `Result<String, toml::ser::Error>`, with no log line. Update its test callers to `.expect("serializes")`: `round_trip`, `atomic_write_and_load`, `old_tmux_entries_are_skipped`, `a_tabs_server_session_round_trips_and_old_files_still_load` and `zone_tree_round_trip_and_fallbacks`. Drop the last one's `assert!(!text.is_empty(), …)`. Verify with `cargo test -p brindle session::tests::round_trip`.
- [x] 1.3 Add the decision (design.md decisions 2 and 3):
  - `pub enum PendingWrite { Nothing, Write(String), Unserializable(toml::ser::Error), StillUnserializable }`, deriving `Debug` and `PartialEq`, with a doc comment per variant.
  - The private `fn next_write(last_written: &mut Option<String>, reported: &mut Option<toml::ser::Error>, text: Result<String, toml::ser::Error>) -> PendingWrite`.
  - The field `unserializable: Option<toml::ser::Error>` on `SessionStore`. Doc comments on it and on `last_written`: the text the file last received from this process, set only on `Write` and kept through failures (b27 #21, `types R10`).
  - `pending_write` returns `PendingWrite`: `Nothing` for the two early returns, then `next_write(&mut self.last_written, &mut self.unserializable, self.state().to_text())`.
  - Update `store_rules` and `forgetting_an_unrecorded_window_still_writes` to compare with `PendingWrite::Nothing` and match `PendingWrite::Write(text)`.
  - Verify that `cargo test -p brindle session::` passes, the 1.1 tests included.

## 2. Shell: `record_session` (`src/main.rs`)

- [x] 2.1 In `record_session`'s `pending` helper (`src/main.rs:664-668`), take the `PendingWrite` from the store and match all four variants, with no `_ =>` arm (`railway R3`). Return `Some(text)` only for `Write(text)`. Log `Unserializable(err)` once with `log::warn!("not writing session state to {}: it can't be serialized: {err}", session::state_path(&session::state_dir()).display())` (`railway R8`). Return `None` for `Nothing` and `StillUnserializable`. The tick (`:690-694`) and `on_app_quit` (`:709-711`) both use `pending` and keep their calls unchanged. Verify that `cargo build --locked` builds with no warnings and that `cargo test -p brindle` passes. This shell code has no unit test of its own: the decision it dispatches on is held by 1.1 (`testing R1`).

## 3. Docs

- [x] 3.1 In `README.md`'s session restore paragraph ("The state is written to `~/.local/state/brindle/session.toml` …"), add: a state that can't be saved, such as one with a tab in a directory whose name isn't valid UTF-8, leaves the file as it was, with a line in the log, until it can be saved again. Verify by reading it against the delta spec's "State file location and writes".

## 4. Checks

- [x] 4.1 Run `cargo test --workspace --locked` (it includes `tests/architecture.rs`) and `cargo build --release --locked`, and check that both pass with no new warnings.
- [ ] 4.2 Windows: CI's `windows` job (`build-windows.yml`, `cargo test`) runs `a_directory_that_isnt_utf8_leaves_the_file_alone` with its `#[cfg(windows)]` helper. Treat that run as the Windows arm's first run, not as already verified on Linux (`testing R17`). Verify that the job is green on the PR.
- [ ] 4.3 Manual check, handed to the user with `scripts/run-throwaway.sh --keep`, since it opens a window. Confirm each step:
  - In the tab, run `d=$(printf '/tmp/brindle-\377'); mkdir -p "$d"; cd "$d"`, then wait 5 s.
  - The terminal that ran the script shows one `not writing session state … it can't be serialized` line, not one per second, and the throwaway directory's `session.toml` is not empty.
  - Run `cd /tmp` and wait 2 s. `session.toml` now records `/tmp`.
  - `cd "$d"` again and quit. `session.toml` is still the `/tmp` version, and a second launch on that directory restores the tab.

## Workflow follow-up

- Land as one PR (title `fix(session): keep the state file when the state can't be serialized`, at most 93 characters), with the archive commit (`docs(openspec): archive keep-session-file-on-serialize-error`) on the same PR, syncing the `session-restore` delta into `openspec/specs/session-restore/spec.md`.
- List the manual check in 4.3 in the PR body (`testing R16`).
