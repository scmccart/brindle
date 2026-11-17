# Design

## Context

See proposal.md, "Why". The audit's finding F1 (`docs/architecture/audits/2026-11-16.md`, "Findings" 1 and "Verified by hand" row 1; instance b27 #12) is the only finding this change closes.

How a write happens today:
- `SessionStore` (`src/session.rs:660-731`) is the process's recorded windows, a GPUI global. It holds the entries, a `touched` flag, `last_written: Option<String>` and the instance's `Lock` (an open file).
- `pending_write()` returns `None` when nothing was touched or this process doesn't hold the lock. Otherwise it serializes `state()` with `to_text()`, returns `None` when the text equals `last_written`, and else stores and returns the text.
- `record_session` (`src/main.rs:663-715`) calls it through its `pending` helper on a one-second tick, and hands any text to `write_atomic` on the background executor with a `next_generation()` ticket. `on_app_quit` calls the same `pending` once more after every window has quit (`Workspace::quit`), and writes synchronously.
- `to_text()` (`:267-272`) maps a `toml` error to a log line and `String::new()`. After the first record, `touched` stays true for the process's life, so a state that can't be serialized is serialized again, and fails again, every second. The first failure stores `""` in `last_written` and writes an empty file. Later ticks, and the quit, return `None`, because `""` equals `last_written`. So the log line repeats every second, and the empty file is written once.

The reachable trigger is a `PathBuf` that isn't valid UTF-8. serde serializes a `Path` only through `to_str()`, and fails with "path contains invalid UTF-8 characters" otherwise. Local zones' directories come from `/proc/<pid>/cwd` (`terminal/process.rs:165`), which can hold any bytes. `TabState::Local.cwd` and `ZoneNodeState.cwd` are `Option<PathBuf>`.

## Goals / Non-Goals

**Goals:**
- A serialization failure is a value (`railway R1`) that never reaches `write_atomic`, on the tick or on quit.
- `last_written` keeps naming the text the file last received, so the next state that serializes is written when it differs, and isn't rewritten when it matches.
- One log line per failure, not one per tick.

**Non-Goals:**
- **Recording the rest of the state when one field can't be serialized**, for example by dropping a non-UTF-8 directory from its zone. Until the shell leaves such a directory, nothing new is recorded. Dropping the field is a change to what is recorded (`SnapshotTab`, `collapse`, the "Local tabs restore their profile, zones and directory" requirement), and would hide other serialization failures. Left for a follow-up if it matters in practice. This change makes any serialization failure safe, whatever its cause.
- **Showing the failure in the window.** `railway R6` names sessions, tabs and endpoints as the units whose failures are drawn. The state file is none of them, and its write failures are already only logged (`main.rs:672`). The spec's "Bad state files are ignored" uses a log line too.
- **The other `railway R1` entries in `session.rs`** (b27 #13 `state_dir`'s `.` fallback, #14 the missing-profile fallback), which are change 12's. The write-generation static (b27 #23) is change 4's.
- **Any change to the file format, its location, `write_atomic` or the ordering ticket.**

## Decisions

### 1. `State::to_text` returns `Result<String, toml::ser::Error>`

The error stays `toml`'s own type: it is data (`Clone`, `PartialEq`, `Eq`, `Display`), and the only consumer logs it (`railway R4`, `R11`).

*Rejected:*
- **`Result<String, String>`:** decision 3 compares failures, and `railway R4` says never to compare error text.
- **`anyhow::Error`:** allowed in `brindle`, but it is neither `PartialEq` nor `Clone`, and the model needs both.

### 2. `pending_write` returns a closed outcome enum

```rust
/// What the state file needs ([`SessionStore::pending_write`]).
#[derive(Debug, PartialEq)]
pub enum PendingWrite {
    /// Nothing to write: this process doesn't own the file, nothing was
    /// recorded yet, or the state's text is what the file last received.
    Nothing,
    /// Replace the file with this text.
    Write(String),
    /// The state can't be serialized. The file stays as it was; report this.
    Unserializable(toml::ser::Error),
    /// The same failure as the last call, already reported.
    StillUnserializable,
}
```

- **States it rules out:** `Option<String>` let `Some("")` stand for both "write this empty text" and "serialization failed", and the shell couldn't tell them apart. A failure now can't reach `write_atomic`, because only `Write` carries text.
- **Where the log line goes:** the shell logs. `record_session`'s `pending` helper matches all four variants with no `_ =>` (`railway R3`), returns the text only for `Write`, and logs `Unserializable` as `not writing session state to <path>: it can't be serialized: <error>` (`railway R8`: the action, then the cause). The model stays free of side effects (`fcis R3`; b27 #11 flags a `log::warn!` inside pure code for the same reason), and the shell dispatches on a richer return value (`fcis R9`, `R10`). The tick and the quit path both go through `pending`, so neither can write a failed state.

*Rejected:*
- **`pending_write` logs and returns `None`**, as the remediation plan's scope line words it: that puts a side effect in the model, and it logs every second.
- **`Result<Option<String>, toml::ser::Error>`:** the shell would log every tick, or keep its own "already reported" flag. That flag is a decision in the shell (`fcis R1`).
- **Writing an empty but valid state (`version = 1`):** loses the saved windows the same way.

### 3. The decision is a pure function over plain values

```rust
fn next_write(
    last_written: &mut Option<String>,
    reported: &mut Option<toml::ser::Error>,
    text: Result<String, toml::ser::Error>,
) -> PendingWrite
```

- `Ok(text)` clears `reported`. It returns `Nothing` when `text` equals `last_written`; otherwise it stores `text` in `last_written` and returns `Write(text)`.
- `Err(error)` leaves `last_written` alone. It returns `StillUnserializable` when `reported` already holds an equal error; otherwise it stores the error in `reported` and returns `Unserializable(error)`.
- `SessionStore::pending_write` keeps its two early `Nothing` returns (not touched, no lock), then calls `next_write(&mut self.last_written, &mut self.unserializable, self.state().to_text())`. `SessionStore` gains the field `unserializable: Option<toml::ser::Error>` (the failure last reported, cleared by any state that serializes).
- **Why a free function:** `SessionStore` holds the instance `Lock`, an open file, so it isn't a plain-values model (`fcis R5`). It can be built in a test only after taking a real lock in a temporary directory. `next_write` takes plain values and can be tested on every platform with an error made by `serde::ser::Error::custom` (`fcis R7`, `R8`).
- **Field docs:** `last_written` gets the doc comment b27 #21 (`types R10`) asks for: the text the file last received from this process, set only by `next_write` on `Write` and kept through failures. `unserializable` gets one too.
- **A failure that comes back:** the shell leaves the directory, a state serializes (clearing `reported`), then it enters the directory again. That is reported again, once. An error with different text (another field) is reported too.

### 4. Tests

- **Unit, platform-neutral (`testing R1`, `fcis R8`), in `src/session.rs`'s `mod tests`:** `next_write`'s table, with errors from `serde::ser::Error::custom`. The cases:
  - a first failure is `Unserializable` and leaves `last_written` as it was;
  - the same failure again is `StillUnserializable`;
  - a different failure is `Unserializable`;
  - after a failure, the text last written gives `Nothing`, and other text gives `Write`;
  - a success between two equal failures makes the second one `Unserializable` again.
- **Unit, the real trigger:** a `SessionStore` holding a lock in its own temporary directory, recorded with a window whose tab `cwd` isn't valid UTF-8.
  - It returns `Unserializable` and then `StillUnserializable`, never `Write`.
  - Recording the window again with `cwd: Some("/tmp")` returns `Write` with that text.
  - Built through a test helper `unserializable_dir() -> PathBuf`: `OsStr::from_bytes(b"/tmp/\xff")` under `#[cfg(unix)]`, `OsString::from_wide(&[0xD800])` under `#[cfg(windows)]`. Both platforms are covered (`testing R17`). The `cfg` is in test code, which D1 of the remediation plan puts outside `ports R4` (written into the docs by change 5, `enforce-platform-seams`); this test module already has a `#[cfg(unix)]` block (`atomic_write_and_load`).
- **Existing tests:** `round_trip`, `atomic_write_and_load`, `old_tmux_entries_are_skipped`, `a_tabs_server_session_round_trips_and_old_files_still_load` and `zone_tree_round_trip_and_fallbacks` unwrap `to_text()` with `expect`. The last one's `assert!(!text.is_empty())` goes, since the `Result` now says it. `store_rules` and `forgetting_an_unrecorded_window_still_writes` compare with `PendingWrite::Nothing` and match `PendingWrite::Write(text)`.
- **No e2e case.** The shell's part is one exhaustive `match` in `record_session`, shared by the tick and the quit. The decisions the spec scenarios describe are in `next_write` and are held by the unit tests. The restore cases (`tests/e2e/cases/restore.rs`) keep covering the write path end to end. A hand check with a real non-UTF-8 directory is listed in tasks.md.

### 5. Layers, versions and the server

- **Layers:** pure code is `State::to_text`, `next_write` and `PendingWrite` in `src/session.rs`. The shell is `record_session` in `src/main.rs`. No seam, no shared crate, and no crate reference changes.
- **Versions (`process R15`):** the state file's format, keys and version are unchanged. A state that serializes is written byte for byte as before, so older and newer Brindles read each other's files as they do today.
- **Terminal server:** untouched. A core restart, a Brindle crash or an upgrade sees nothing new (`process R1`–`R3`, `R12`). A crash while the state can't be serialized leaves the last good file, which is the point.

## Risks / Trade-offs

- [The file goes stale while a shell sits in a non-UTF-8 directory: tabs opened meanwhile aren't recorded] → Writes resume within a second once any state serializes. Local sessions are detached on quit and swept back as tabs from the server's layouts anyway (see CLAUDE.md, "Reattach"). Recording around the bad field is a non-goal (above).
- [`toml::ser::Error`'s `PartialEq` compares its kind and message, so failures from two different fields read as different] → Each is logged once, which is the intent.
- [`update-dependencies` moves `toml` to 1.x] → If 1.x renames `toml::ser::Error`, whichever change lands second adjusts the path. If 1.x drops its `PartialEq`, `unserializable` becomes "a failure was reported since the last success", reporting once per run of failures. It never compares error text (`railway R4`).
