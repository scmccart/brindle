# Proposal

## Why

The 2026-11-20 security audit (`docs/security/audits/2026-11-20.md`) found one high and four medium vulnerabilities that survived refutation:
- **High:** a zero-click NTLM credential leak on Windows, through program-reported directories.
- **Medium:**
  - code planted unelevated gets run elevated;
  - links open any handler or program;
  - a remote endpoint can abort Brindle at launch;
  - the instance pipe trusts whoever serves it.

Each one is reachable from untrusted program output, a remote endpoint or another local principal. Each has a small, local fix.

## What Changes

- **Reported directories (high, finding 1):**
  - On Windows, a reported directory (OSC 7 or OSC 9;9) is accepted only when it is a drive-letter path. UNC, device (`\\?\`, `\\.\`) and WebDAV paths are ignored, like a path that isn't absolute.
  - Checking whether a reported directory still exists never runs on the UI thread, so a slow or unreachable path can't freeze the window.
- **Elevated server files (medium, finding 2; also the low `run-elevated` finding):**
  - On Windows, an elevated Brindle keeps its server directory, its helper copies and its extracted ConPTY files in directories an unelevated process can't write: owned by Administrators, with no access for the user's own SID, and a High integrity label.
  - It refuses or replaces such a directory when it finds one it didn't make, and never follows a reparse point inside it, including when pruning old copies.
  - Program names given without a path are resolved through `PATH` only, never from the helper's own directory.
- **Links (medium, finding 3):**
  - Ctrl-click opens only links whose scheme is one Brindle detects as a link (`http`, `https`, `ftp`, `file`, `ssh`, `git`, `gemini`, `gopher`, `ipfs`, `ipns`, `mailto`, `news`, `magnet`), for detected and OSC 8 links alike.
  - A `file:` link opens only when its host is empty, `localhost` or this machine's name.
  - On Windows, a `file:` link to anything but a directory is shown selected in Explorer, not opened. Double quotes and whitespace are percent-encoded before the link reaches the opener.
  - A link that isn't opened says why in the tab, as today.
- **Endpoint session sizes (medium, finding 4):** a session size an endpoint lists is used only within Brindle's grid limits. A larger one counts as no size, and every path that makes or resizes a grid clamps to the same limits.
- **Instance pipe (medium, finding 5):** on Windows, a launch that forwards to the running Brindle checks before writing anything that the pipe's server runs as the same user at the same integrity. It also connects so that the server can identify it but not impersonate it, and refuses with a message otherwise.

**BREAKING:**
- Ctrl-clicking a link with another scheme (for example `vscode:`, `ms-settings:`, or an OSC 8 URI like `C:\x`) no longer opens it.
- An elevated launch no longer forwards to an unelevated Brindle using the same state directory; it exits saying why.

Out of scope:
- **The lows:** the remaining low findings, which are left for later changes.
- **Showing a link's target before it opens.** That would be a separate UI change.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `shell-integration`: "Working directory reports" accepts only drive-letter paths on Windows, and checking a reported directory never blocks the window.
- `links`: "Opening links" opens only the detected schemes, limits `file:` links to this machine, and shows Windows files in Explorer instead of running them.
- `remote-connections`: a new requirement bounds the session sizes an endpoint lists.
- `command-line`: "A launch goes to the running Brindle" checks the instance's identity on Windows before sending the request.
- `terminal-server`: "Installed helper stays replaceable on Windows" names the elevated location for copies. Two new requirements:
  - every file an elevated server runs or loads stays out of an unelevated process's reach;
  - bare program names resolve through `PATH` only.

## Impact

- **`term-core/src/shell_integration.rs`:** `reported_dir`, and its tests.
- **`src/terminal/mod.rs`:** `working_directory`, and the directory check, which moves off the UI thread. Also its callers in `src/workspace.rs`, `src/zone_view.rs`, `src/workspace/launch.rs` and `src/session.rs`.
- **`src/terminal/links.rs`:** the scheme allowlist, the `file:` host rule and the Windows Explorer path.
- **`src/remote/identity.rs` and `term-core/src/emulator.rs`:** `ListedSession::of` and new `MAX_COLS`/`MAX_ROWS` limits beside `MIN_COLS`; the clamps in `src/terminal/mod.rs` and `src/terminal/server.rs`.
- **`src/instance.rs`:** the Windows `forward`, which gets SQOS identification and `brindle_remote_protocol::win::check_server`.
- **Elevated folders:**
  - `remote-protocol/src/{server_dir,access,win}.rs`: the elevated folders, their DACL and label, `ensure_helper_copy` and the prune;
  - `remote-agent/src/server/platform/windows/{dir,pty}.rs` and `remote-agent/src/conpty/{choice,api}.rs`;
  - `src/remote/local/windows.rs`.
- **No wire or protocol change.** The config is unchanged, and the state file is unchanged.
