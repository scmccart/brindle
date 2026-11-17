# Proposal

## Why

Across the app, the helper and the test harnesses, expected failures are logged and replaced by a default, or shown with their cause dropped (audit 2026-11-16, finding F7: `railway R1`, `R6`, `R8`; remediation plan, change 12). A user can't see a log, so each of these looks like nothing happened, or like a different failure:
- a misspelled theme quietly gives `brindle-dark`;
- a ctrl-click on a link does nothing when there is no opener;
- this machine's sessions palette says "No detached sessions" when the server couldn't be listed;
- ctrl-alt-5 with three profiles opens nothing;
- a local tab whose server never came back says only "no terminal server came back within 15 s", although Brindle had the real cause.

Changes 2 and 9 leave three of these to this change by name: change 9's server `ERROR`s about a session, change 7's restored tab whose profile is gone, and change 10's helper seams that return nothing.

## What Changes

- **The window's config line** (today the "Config error" banner) also lists what Brindle used in place of a config value: an unknown theme name, a clamped font size or line height, a keybinding to an unknown action or with invalid keys, a removed key, and a default config that couldn't be written. It shows them until the file is fixed, as it does a parse error. `--dump-screen-after` prints it.
- **A tab says what failed in it**, as one dim line:
  - a link the opener couldn't open, with the cause;
  - a terminal server `ERROR` about its session;
  - a message to its session that couldn't be sent;
  - a `-p` name that matched no profile;
  - a restored tab whose profile is gone, with the profile used instead.
  The line waits for a replay that is still coming, so it is never wiped by one.
- **Lost lines keep their cause.** A local tab whose server never came back names the last error. A remote tab's "connection lost" line names what ended the link.
- **ctrl-alt-N with no Nth profile** opens a message tab saying so, instead of nothing.
- **Sessions palette:** a failure to list this machine's sessions is a disabled row with the cause. A failed kill leaves the session's row and says why. Remote listing failures name the action.
- **Endpoints:** a remote endpoint that can't even start connecting is shown offline. A relay process whose exit couldn't be read says that, instead of "exited with 1". A launch whose stderr arrived late keeps what was read.
- **Settings dialog:** an unreadable config file, or a `themes`/`profiles` key of the wrong shape, is shown in its section with the action and the path. Arguments that can't be written back as shell words mark the field invalid. A missing config file reads as empty (`railway R7`, same line).
- **Command line:** an option missing its value, or with a value it can't use, is a usage error. So is an unknown `--action`, a bad `--key` and a bad `--hover`/`--click` position. A failed `--latency-log` write is in the summary.
- **No state directory** (no `$BRINDLE_STATE_DIR`, `$HOME` or platform directory): Brindle runs on its own, restoring and recording nothing, and the window says why. Today it writes `session.toml` in whatever directory it was started in.
- **Terminal server and helper:**
  - a held session's malformed meta is refused, field by field, and the core keeps the session detached rather than give it default owner and policy;
  - an unencodable message to a holder fails that session's request and tells its client;
  - error texts name the action;
  - the Windows seams return what they used to drop: the ready pipe, the console-ctrl flag, pipe peeks, the process snapshot, ConPTY loading and reparenting, helper-copy pruning. Each is logged by its caller, with no new cache of a failure.
- **Test harnesses:** e2e and perf stop turning a failed timeline thread, a failed `brindle-remote list`, an unwritten capture or summary, or a failed process snapshot into a pass.
- **`brindle-ui`:** an icon path that fails to build is a value, logged once.
- No wire, protocol-version, config-key or state-file format change.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `configuration`: "Config location and first run" (an unwritten default config is shown), "Invalid config falls back to defaults" (clamped values and other fallbacks are named on the config line), "Themes" (the unknown-theme warning is on the config line), "Keybinding overrides" (rejected bindings are shown, not only logged).
- `links`: "Opening links": a link that isn't opened, or an opener that fails, is reported in the tab.
- `remote-connections`: "Lost connections reconnect": the tab's line names what ended the link.
- `terminal-emulation`:
  - MODIFIED "A session lost for good is reported in the tab": the reason names the last error. The text is the one change 9 leaves, which is change 2's plus its own.
  - ADDED "Failures about a running session are shown in its tab".
- `command-palette`: ADDED "A failed listing is shown in the sessions palette" and "A failed kill is shown in the sessions palette".
- `command-line`:
  - MODIFIED "Launch options" (copied as change 8 leaves it): a missing or unusable value is a usage error, and an unknown `-p` is said in the tab;
  - MODIFIED "Latency log": a failed write is in the summary;
  - ADDED "Debug steps are checked before the run";
  - ADDED "The dump shows the config line".
- `profiles`: "Direct profile shortcuts": a missing Nth profile opens a message tab.
- `session-restore`:
  - MODIFIED "Restoring falls back to defaults": a replaced profile is said in the tab;
  - MODIFIED "State file location and writes" (copied as change 1 leaves it): no state directory means no restore and no recording, said in the window.
- `settings-dialog`: "Theme list" and "Profile list" (an unreadable file or a wrong-shaped key is shown with its cause), "Profile field validation" (words that can't be quoted again).

## Impact

- **Size: L, in seven task groups, one PR each.** The change is archived after the last. Each group lands alone, after the sibling changes it names (design, "Groups and order").
- **Code:**
  - Brindle:
    - `src/config.rs`, `src/actions.rs`, `src/settings.rs`, `src/win.rs`;
    - `src/config_edit.rs`, `src/settings_dialog.rs`, `src/settings_dialog/profiles.rs`;
    - `src/terminal/{mod,server,links}.rs`, a new pure `src/terminal/tab_note.rs`, `src/terminal_view.rs`;
    - `src/remote/local.rs`, `src/remote/local/{connection,relay,upgrade}.rs`, `src/remote/client.rs`;
    - `src/workspace.rs`, `src/picker.rs`, `src/main.rs`, `src/latency.rs`, `src/session.rs`.
  - Helper:
    - `remote-agent/src/server/{core.rs,cli.rs,relay.rs}`, `core/{holders,clients,sessions}.rs`;
    - `server/platform/{unix,windows}/{conn,daemon,pipe,pty}.rs`, `conpty/{api,console}.rs`.
  - `remote-protocol/src/{holder,ready,win}.rs`: `SessionMeta::from_value` and the ready-pipe and prune outcomes. Encodings unchanged.
  - `ui/src/icon.rs`.
  - Test harnesses: `tests/e2e/{brindle,capture}.rs`, `tests/common/winproc.rs` (after change 4 moves it), `tests/perf/main.rs`.
- **Tests:** unit tests for each new outcome; `fake::link` tests in `connection.rs`; core tests in `remote-agent/tests/core.rs`; new e2e cases `reload-config-findings`, `links-opener-fails`, `local-server-session-error`, `local-server-no-core-back`, `local-server-palette-unlisted`, `profile-shortcut-missing` and `launch-unknown-profile`; and checks added to `remote-ssh-network` and `restore-fallbacks`.
- **Docs:**
  - `railway-oriented-programming.md`: "How it applies here" names the tab notice line and the config line;
  - CLAUDE.md: the dump's `--- config` line and the notice line.
- **Audit entries closed:** every `railway R1`, `R6` and `R8` violation in the batch files that no sibling change takes; design, "Entries" lists each with its unit. Folded in: b10's `clients.rs:404` smell (the same encode shape as b11 #11), b23's removed-key smell (`Config::parse`), b05's `railway R7` on `profiles.rs:558` (the same line), and b21's `prune_helper_copies` smell, which change 10 hands over.
- **Depends on:**
  - change 2 (`Shown`, `State::shows`);
  - change 9 (the event kinds, `Outgoing`, the holder's `ERROR`s), both named by the plan;
  - per group, also changes 1, 3, 4, 7, 8 and 10 (design, "Groups and order").
- **Overlap with changes in flight:**
  - `add-blocks` edits `terminal_view.rs` near the link click;
  - `port-to-macos` edits `config.rs` (the path) and will need a macOS `open_default`;
  - both are textual overlaps only. `update-dependencies` and `add-pocket-client` don't overlap;
  - `extract-decisions-into-models` (change 13) builds on this change's error values and lands after it.
