## Context

- **The config line:** `Settings::config_line(error, findings, notices)` already builds one line from:
  - config errors;
  - config findings;
  - window notices that aren't about the config (`notices`).

  `Workspace::render` only reads it.
- **Link failures:** they go through `Terminal::note` (`terminal/tab_note.rs`), which writes a dim line into the grid.
- **The instance:** `instance.rs` keys it by the state directory. The directory holds `session.lock`, and the instance's pipe or socket is named from a hash of the directory's path.

## Goals / Non-Goals

**Goals:**
- Link failures leave the terminal's text alone.
- An elevated and an unelevated Brindle can both run at once.
- A launch that must give up says so where the user can see it.

**Non-Goals:**
- Moving the other `tab_note` users: connection lost, input not sent, server errors, an unknown `-p`. Each of those belongs to a session and stays in its terminal.
- Sharing a saved session between integrity levels.

## Decisions

### 1. Link failures become window notices

- **Pure model (`workspace/notices.rs`, new, GPUI-free):**
  - `Notices` holds the window's transient notices, each `Notice { id: NoticeId, text: String, until: Instant }`.
  - `push(text, now) -> NoticeId` replaces an entry with the same text (so a notice doesn't repeat), and keeps at most 3, dropping the oldest.
  - `expire(now)` drops those past `until` (`NOTICE_LIFETIME`, 10 s) and returns the next deadline.
  - `dismiss_all()` drops them all.
  - `texts()` gives what `config_line` joins.
  - Unit tests pass `now` in (`fcis R2`).
- **Shell:**
  - `TerminalView` emits `LinkFailed(String)` instead of calling `Terminal::note`. The text is the one `links::failure_text` already makes. This covers both a refused link and an opener's failure, which arrives later from its background thread.
  - `Workspace` subscribes, pushes the notice and keeps one timer task for the next deadline.
  - `Settings::config_line` gets these texts in `notices`, after the ones it has now. `config_line` stays pure.
- **View:**
  - The config line gets a dismiss button (ghost `IconXs`, tooltip "Dismiss") while it shows transient notices. It dismisses only those.
  - Config errors and findings stay until the file is fixed, as the `configuration` spec says.
  - The `dismiss_notices` action does the same for the keyboard and for `--action` runs.
- **Dump:** the format is unchanged. `--- config "…"` carries the notice text, so the e2e cases read it there.

**Rejected:** a toast or desktop notification. It leaves the window, and the config line is already where Brindle says it couldn't do what the user asked.

### 2. Elevated state directory on Windows

- **Pure:** `session::state_dir(env)` reads a new field, `StateEnv::elevated: bool`. On Windows, with `BRINDLE_STATE_DIR` unset and `elevated` true, the directory is `%LOCALAPPDATA%\brindle\state-elevated`. When `BRINDLE_STATE_DIR` is set, whoever set it chose the directory, as for the server directory.
- **Shell:** `main.rs` reads the elevation once through `win::is_elevated` (already in the `win.rs` seam), and Linux passes `false`.
- **Nothing else changes:** the instance's key, lock and pipe name all come from the state directory, so two levels are two instances. The integrity check before a hand-off stays. It is now reached only when `BRINDLE_STATE_DIR` names one directory for both levels on purpose.
- **Protection:** the elevated state directory is made like the elevated server directory, with `win::protected_dir(…, Contents::State)`. This stops an unelevated process from planting a `session.toml` for an elevated Brindle to restore. It uses `Contents::Executables`, not `State`: a directory found there without the elevated descriptor is set aside and made fresh rather than re-protected in place, since re-protecting keeps the files' contents, and nothing an unelevated process could have written may be restored. The Windows-only call sits in the `win.rs` seam, behind a function `main.rs` calls on every platform.

### 3. A failed hand-off is visible

- `instance::not_sent` and the 5 s timeout already make the text.
- On Windows, a launch with no console also shows that text in a `MessageBoxW` (`win.rs`) before it exits with status 1. "No console" means `GetConsoleWindow` returns null; the console seam already knows this.
- Linux keeps stderr only; a desktop launch there has the journal.

## Risks / Trade-offs

- **The elevated session saved before the upgrade is left behind.** It stays in the shared `session.toml`, where the unelevated Brindle restores it next time. Sessions still detached in `run-elevated` are reattached by the elevated Brindle's first listing, as tabs grouped by their layouts, so no running program is lost.
- **A notice can expire unread.** 10 s is enough to read one line after a click, and the failure is also logged at `info`.

## Architecture

- **`fcis R1`–`R3`:** `Notices` and `state_dir` are pure and unit-tested.
- **`ports R4`:** the elevation check and the message box stay in `win.rs`.
- **`railway R6`:** a failure names the action and its cause where the user acted, which is the window that took the click.
- No wire, protocol, config-key or server change.
