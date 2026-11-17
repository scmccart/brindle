# Proposal

## Why

The instance's launch lists this machine's terminal server's sessions on the UI thread, inside GPUI's launch closure, before the window opens (`src/main.rs:863` → `remote::local::running_sessions`, `src/remote/local.rs:379-401`). That call waits on another process: on Windows up to 2 s for a busy core's pipe, then up to 10 s (`HELLO_LIMIT`) for a core that is still adopting its holders to answer `HELLO`, then up to 2 s per read. `process R13` says the UI thread MUST NOT block. CLAUDE.md's Reattach section describes the wait as intended, so the audit read it as a conflict between the docs (audit 2026-11-16, "Verified by hand" row 6, finding 8, "Doc drift"). Decision D2 settled it: the listing moves off the UI thread, as the instance listener's already is (`main.rs:767`), R13 stays as written, and CLAUDE.md follows the code. The same finding has four more `process R13` entries and one `process R14` entry, and this change fixes them all:
- b24 #1 is the same launch listing, seen from `local.rs`;
- the other three are UI-thread waits: the config watch and reload, the notification log, and the Wayland activation round trip;
- the R14 entry is a regex scan in `prepaint`.

## What Changes

- **Launch listing.** The launch lists the sessions, and reads the state file, on a thread of their own, started in `main` before GPUI starts. The UI thread waits for the answer in a task and opens the window when the listing has answered or failed.
  - The restored tabs, the reattached sessions and the active tab are what they are today for the same answer.
  - The window opens no later than today: the listing now runs while GPUI starts up, where it used to start after.
  - What the launch does with the answer becomes one pure function with unit tests (`fcis R1`, `R8`). It replaces the decision written inline in GPUI's launch closure.
- **Failed listing.** A failure is a value, `reattach::Listing::Failed`, not an empty listing.
  - A restoring launch keeps every recorded zone, opens no detached session as a tab, and logs why. That is today's behaviour, now written into the spec.
  - `brindle --attach ID` now says the server couldn't be listed. Today it wrongly says the server has no session `ID`.
- **Config.** The config watch stats the file, and every reload reads and parses it, on the background executor. Results are applied on the UI thread in request order. The first read at startup moves into `main`, before GPUI starts.
- **Notification log.** The `BRINDLE_NOTIFY_LOG` sink appends on a writer thread of its own. Quitting waits for lines already queued, so a line is still in the file by the time Brindle exits.
- **Wayland activation.** The registry round trip that binds `xdg_activation_v1` runs on the background executor once the window is open. A raise that comes before it finishes waits for it in a task instead of blocking.
- **Find bar.** The matches in view are computed when the query, the options, the output or the scroll position changes, and cached. `prepaint` only reads the cache.
- **Listing tokens.** Each listing connection gets a client token of its own. Two listings at once (the launch's and a forwarded `--attach`) no longer share a token, which made the core close the first one.
- **Test switch.** `BRINDLE_TEST_LAUNCH_LISTING`, in debug builds only, makes the launch listing slow or fail, for two of the new e2e cases (`process R20`).
- **Docs.** CLAUDE.md's Reattach, settings and notifications paragraphs are rewritten to match, and so are the doc comments of `running_sessions` and `src/reattach.rs`. `process R13` is unchanged.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `session-restore`:
  - "Launch reattaches detached terminals": the window opens once the launch's listing has answered or failed, with the same tabs; a failed listing opens no detached session as a tab.
  - "Restored zones get their sessions back": what a local zone does when that listing failed.
- `command-line`:
  - "Notification log for tests": the line is in the file once Brindle has exited, without the write holding up the window.
  - "Launch options": `--attach` says when the server couldn't be listed.

## Impact

- **Code:**
  - `src/main.rs`: `main`, the `Application::run` closure, `running_sessions`, `restorable_window` (removed), `forwarded_launch`, `Forwarded`, `watch_config`, `reload_config`, `reload_config_renaming`, `take_launches`.
  - `src/reattach.rs`: the new `Listing` enum.
  - `src/remote/local.rs`: `running_sessions`' token and doc.
  - `src/notify/mod.rs`: the `Sink::Log` writer and the quit flush.
  - `src/notify/activation.rs`: `prepare`, `raise`, and the attachment state.
  - `src/find_bar.rs`: the visible-match cache.
  - `src/terminal_element.rs`: the `prepaint` call site.
  - `src/workspace.rs`: the settings dialog's `ConfigWritten` handler.
- **Unchanged:** no wire, protocol, config key or state-file format. The terminal server, `brindle-term-core`, `brindle-remote-protocol` and `brindle-ui` aren't touched.
- **Tests:** unit tests in `src/main.rs`, `src/reattach.rs`, `src/notify/activation.rs`, `src/find_bar.rs` and `src/remote/local.rs`. Three new e2e cases: `local-server-reattach-slow-listing`, `local-server-reattach-failed-listing` and `notify-log-at-exit`. The restore, reattach, instance, notify, search and reload cases run unchanged.
- **Overlap with the remediation plan's other changes:**
  - Change 4 (`isolate-test-runs`) splits `running_sessions` into `running_sessions_at(endpoint, limit)` (its task 3.3) and names this change as the caller that moves. Neither edits the other's lines, and the token change here goes into whichever function holds the body.
  - Change 1 (`keep-session-file-on-serialize-error`) changes `record_session` in `src/main.rs`. This change moves the call to `record_session` into the launch task but leaves its body alone.
  - Change 2 (`carry-session-identity-and-host`) changes `AttachRequest` and `Workspace::attached_terminal`, which the restore path calls. It doesn't touch `reattach::Detached` or the launch decision.
  - Change 9 (`bound-queues-and-contain-panics`) bounds other queues. The notification log's queue is bounded by the notification rate limits (`notify/policy.rs`).
  - Change 13 (`extract-decisions-into-models`) may later move `main.rs`'s pure launch helpers into a model of their own.
  - Changes 3, 5, 6, 7, 10, 11 and 12 touch none of these functions.
- **Overlap with changes in flight:**
  - `add-blocks` (task 6.1) reworks how `find_bar.rs` reveals matches. Whichever lands second keeps the cache refreshed wherever the view moves.
  - `update-dependencies` bumps `wayland-protocols` and lists `notify/activation.rs` to check. Whichever lands second adjusts the imports.
  - `port-to-macos` touches `src/main.rs`'s window options and menu bar, not the launch's listing. On macOS the listing stays `Listed(vec![])`, as today's `Some(Vec::new())`.
  - `add-pocket-client` doesn't overlap.
