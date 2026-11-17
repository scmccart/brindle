# Design

## Context

See proposal.md, "Why". The audit is `docs/architecture/audits/2026-11-16.md`, finding 7 (`railway R1` 36, `R6` 5, `R8` 8). The plan is `2026-11-16-remediation.md`, change 12. "b25 #18" means item 18 under Violations in `2026-11-16/b25.md`. Batches whose violations are unnumbered bullets are cited by file and line (b05, b06, b15, b23). The code is unchanged since the audit (plan, "Re-checked"); line numbers below are the audited tree's.

### Rules

- **`railway R1`:** an expected failure is in the return type, never a log line with a default returned in its place.
- **`railway R6`:** a session's, tab's or endpoint's failure is that unit's visible state, not cached.
- **`railway R8`:** error text names the action, then the most specific cause, and keeps the cause the system had.
- Also used here: `railway R5` (text from outside is never trusted), `R7` ("no" is an answer), `R10` (config findings collected and shown); `fcis R1`, `R8` (decisions in pure code, tested in the same file); `process R4`–`R6`, `R13`, `R14`, `R16`; `ports R4`, `R8`, `R9`; `types R1`, `R2`, `R7`.

### Entries

Every `railway R1`, `R6` and `R8` violation in the batch files (`grep -n "railway R[168]"`), minus those another change takes. None of them is tagged under the `railway R4` exception, which covers `Result<_, String>` signatures, not these. The "unit" column says which unit owns the failure and how it is shown; "log only" says why no unit fits.

**Taken by other changes:**
- b27 #12 (`session.rs:267`, the empty-state write): change 1.
- b01 #21 and #22 (`running_sessions` drops its cause; `--attach` reads a failed listing as "no such session"): change 8. Its `reattach::Listing::Failed(why)` and `Listing::attach_target` carry the cause.
- Change 8 leaves "drawing a failed launch listing on the window" to this change (its design, Non-Goals). This change doesn't take it: the restored zones start their programs as for a session that is gone, and the sessions palette lists the sessions again on demand (decision 7 makes its own failure visible). A launch-wide notice would need a unit `railway R6` doesn't name. The launch log keeps the cause.

**This change, by group** (groups in "Groups and order"; decisions below):

| Entry | Site | Rule | Unit and how it is shown | Group |
|---|---|---|---|---|
| b20 #9 | `remote-protocol/src/holder.rs:670-694` `SessionMeta::from_value` | R1 | the session: kept, detached and listed; a `core.log` line names the fields (decision 11) | 1 |
| b11 #11 (+ b10 smell `clients.rs:404`) | `core/holders.rs:636-641`, `core/clients.rs:401-406` `queue` | R1 | the session's client: a failed spawn's error, or an `ERROR` for the session, which group 3 shows in its tab | 1 |
| b10 #13 | `server/core.rs:124-143`, `server/cli.rs:96, 202, 207, 259, 281, 308` | R8 | the CLI's stderr, or the starter's error: text names the action | 1 |
| b14 #8 | `platform/windows/pipe.rs:1064-1098` `input_waiting` | R1 | the holder connection: an `Err` drops it with the cause in `core.log` | 1 |
| b13 #3 | `platform/windows/daemon.rs:195` `ReadyPipe::read` | R1, R8 | the starter's "didn't start" error, now with the OS error | 1 |
| b13 #5 | `platform/windows/daemon.rs:553-558` `daemonize` | R1 | log only: the role's own log. The role still serves, and no tab is involved | 1 |
| b15 | `server/relay.rs:256` `build_id::current().ok()` | R1 | log only: one `core.log` line. The relay works, and only the upgrade offer is lost, as for a build with no id | 1 |
| b15 | `platform/windows/pty.rs:290-292` `end_its_programs` | R1 | log only: the holder's log. The hang-up ends a session no tab shows any more | 1 |
| b09 #4, #5 | `conpty/api.rs:50-109` `packaged`, `PACKAGED` | R1, R6 | log only: the holder's log, recomputed per spawn, with no failure cached. The session runs on the inbox ConPTY, a fallback `terminal-emulation` documents | 1 |
| b09 #6 | `conpty/console.rs:138-154` `reparent` | R1 | log only: the holder's log. `REPARENT` is advisory and has no reply (`terminal-server`) | 1 |
| b09 #7 | `conpty/api.rs:165-196` `missing` | R8 | the holder's log, naming the missing export | 1 |
| b21 smell (handed over by change 10) | `remote-protocol/src/win.rs:567-584` `prune_helper_copies` | R1 | log only: `core.log` at start. A copy in use is expected, and nothing is shown | 1 |
| b23 | `src/config.rs:423-426` unknown theme | R1 | the window's config line | 2 |
| b23 (`railway R10`, which the plan folds in) | `src/config.rs:394-395` clamps | R10 | the config line | 2 |
| b23 | `src/config.rs:314-316` `write_default` | R1 | the config line | 2 |
| b23 smell (folded) | `src/config.rs:337-346` removed keys logged in `parse` | R1 | the config line | 2 |
| b00 #1 | `src/actions.rs:441-457` `bind_keys` | R1 | the config line | 2 |
| b00 #8 | `src/config_edit.rs:134-135, 199-200` | R1 | the settings dialog's section | 2 |
| b04 #12 | `src/settings_dialog.rs:382-392` `reload_entries` | R1, R6 | the Theme section's line | 2 |
| b05 | `settings_dialog/profiles.rs:558` | R8 (+ R7, the same line) | the Profiles section's read-only line; a missing file reads as empty | 2 |
| b05 | `settings_dialog/profiles.rs:166, 170` `join_words`, `quote` | R1 | the profile editor's field, marked invalid | 2 |
| b06 | `src/win.rs:97` `has_font_family` | R1 | log only, once per process: the family stays listed. Font enumeration belongs to no tab, and hiding a family the user may have chosen is the worse fallback | 2 |
| b28 #10 | `src/terminal/links.rs:111-176` `open` | R1 | the clicked tab: a notice line | 3 |
| b25 #18 | `connection.rs:1459-1476` `reconnect_local` | R8 | the tab's lost line, with the last error | 3 |
| b25 #17 | `connection.rs:1625-1636` `write`'s encode failure | R6 | the session's tab: a notice line. An oversized layout is log only (decision 5) | 3 |
| change 9's handover | `connection.rs:1132-1137` session `ERROR` at `debug` | R6 | the session's tab: a notice line | 3 |
| b01 #20 (+ b01 #10, `fcis R10`, the same decision twice) | `src/main.rs:567-573`, `:792-797` | R1, R6 | the opened tab: a notice line | 3 |
| b07 #15 | `src/workspace.rs:655-658` `launch` | R6 | a message tab | 3 |
| b27 #14 (after change 7's `missing_profile`) | `src/session.rs:502-505` | R1, R6 | the restored tab: a notice line | 3 |
| b07 #13 | `src/workspace.rs:239-242` `listing_from` | R1 | this machine's palette row | 4 |
| b07 #14 | `src/workspace.rs:275-279` failed kill | R1 | the endpoint's palette row; the session's row stays | 4 |
| b07 #16 | `src/workspace.rs:237, 270` | R8 | palette rows name the action | 4 |
| b24 #11 | `src/remote/local.rs:218, 247` `shared` failures | R6 | the endpoint: `LinkState::Offline`, its palette row | 4 |
| b26 #6 | `src/remote/local/relay.rs:249-290` failed `wait` | R1, R8 | the endpoint's reason: its row, the lost line, the log | 4 |
| b23 | `src/remote/client.rs:50-55` stderr drain thread | R1 | the launch's error text: the endpoint's row and new tabs' messages | 4 |
| b23 | `src/remote/client.rs:61-77` stderr timeout | R8 | the same | 4 |
| b26 #7 | `src/remote/local/upgrade.rs:49-73` | R6 | log only: an `Offered` outcome that the caller logs. The running core keeps working by design, and `terminal-server` "A failed upgrade keeps the sessions" puts the reason in the log | 4 |
| b01 #19 | `src/main.rs:300-358` dropped option values | R1 | the command line: usage error, exit 2 | 5 |
| b01 #24 | `src/main.rs:925-945` bad debug steps | R1 | the command line: usage error at parse time | 5 |
| b01 #26 | `src/latency.rs:295-300` | R1 | the latency summary | 5 |
| b27 #13 | `src/session.rs:286` `state_dir`'s `.` | R1 | the window: the config line's state notice; the run stays on its own | 5 |
| b01 #23, #25 | `src/main.rs:643-649`, `:669-674` state file read and write | R1 | log only: the state file belongs to no tab or endpoint, and `session-restore` "Bad state files are ignored" logs it. This is change 1's call for its own write path. `session::load` already returns `Result`; the shell's log is the decision | — |
| b32 #4, #5, #6 | `tests/e2e/brindle.rs:559`, `:199-218`, `capture.rs:109` | R1 | the e2e case: it fails with the cause | 6 |
| b36 #3, #4, #5 | `tests/perf/main.rs:209`, `winproc.rs:28`, `:204-206` | R1 | the perf run or the e2e case: it fails with the cause | 6 |
| b41 #3 | `ui/src/icon.rs:242` | R1 | log only, once per icon: a paint has no unit, and a pure `Icon::path` is tested for every icon | 7 |

Not taken: the design smells that only touch these rules (b02's best-effort D-Bus send, b12 `holder.rs:441`, b26's `unix.rs:123` and `pipe.rs:241`, b43's paint, b27 #16, b34 and b35's harness strings) stay in `design-smells.md`, as the plan's "Not planned" says.

### What the plan got wrong or left out

1. **The config entries are b23's, not b26's.** The report's finding 7 cites b26 for `config.rs`, and the plan follows it. The font clamps are `railway R10` (SHOULD), not R1. They are in because the plan names them.
2. **`reconnect_local` is the local server, not a remote connection.** Its lost line is `terminal-emulation`'s "A session lost for good is reported in the tab", which changes 2 and 9 also modify. That delta copies change 9's text. The plan's `remote-connections` "Lost connections reconnect" delta is kept for the remote tab's own line, which today says "connection to A8 lost; reconnecting…" with no cause, although `relay_link_ended` has one (`railway R8`).
3. **The plan's `R8` list misses four entries:** b01 #21 (change 8's), b05 `profiles.rs:558`, b23 `client.rs:61` and b26 #6. The last three are taken here.
4. **More dependencies than 2 and 9.** Siblings hand this change work, or reshape the code it edits:
   - change 1 leaves b27 #13 and #14 here;
   - change 3's fuzz target for `SessionMeta::from_value` then accepts its refusal;
   - change 4 moves `winproc.rs` and adds a sibling of `shared`;
   - change 7 adds `RestoredTab::Local.missing_profile` (b27 #14) and edits `config.rs`;
   - change 8 reshapes the launch in `main.rs` and modifies `command-line` "Launch options";
   - change 10 moves the decisions of `packaged`, `kill_remaining` and `prune_helper_copies` and leaves their outcomes here.
5. **Size L, not M.** Every entry above is small, but they span every crate and seven areas. So the work is in seven groups that each land alone (next section).

## Groups and order

One PR per group, in this order; the change is archived after the last. A group may land as soon as the changes it names have.

| Group | Area | Needs, beyond 2 and 9 landing first |
|---|---|---|
| 1 | Terminal server, helper and protocol (the shared contracts) | change 3 (fuzz target), change 10 (moved seam decisions) |
| 2 | The config line and the settings dialog | change 7 (textual: `config.rs`), change 9 (`config::write_replacing`) |
| 3 | Tab notices | change 2 (`Shown`, `State::shows`), change 9 (event kinds), change 7 (`missing_profile`), change 8 (`Launch options` text; the launch code) |
| 4 | Endpoints and the sessions palette | change 4 (textual: `local.rs`) |
| 5 | Command line, latency and the state directory | change 8 (the launch code), change 4 (`StateFile`), change 1 (the state-file text), change 9 (`latency::Window`), group 2 (the config line) |
| 6 | Test harnesses | change 4 (`tests/common/winproc.rs`) |
| 7 | `brindle-ui` icon path | none |

## Goals / Non-Goals

**Goals:**
- Every entry in the table returns its failure as a value. The shell then shows it on the unit named there, or logs it for the stated reason (`railway R1`, `R6`).
- Every message names the action, then the cause it had (`railway R8`).
- Each new decision is a pure function or model with tests in its file (`fcis R1`, `R8`).

**Non-Goals:**
- **No wire, protocol-version, config-key or state-file format change** (decision 14).
- **No new modal, toast or status component.** The units are the ones `railway R6` names: a tab's line or message, an endpoint's row, the config line.
- **No change to what a fallback chooses.** An unknown theme is still `brindle-dark`, an unknown `-p` still the default profile, a clamped size still clamped. They are now said where the user looks.
- **Showing a failed launch listing on the window.** See "Entries".
- **Change 13's restructuring** of `Terminal`'s and `Pending`'s flags. Notices use the flags as they are.

## Layers

| Part | Layer | Crate and file |
|---|---|---|
| `tab_note::line`, `NoteGate` | pure | `brindle`, `src/terminal/tab_note.rs` (new) |
| `ConfigFinding`, `Loaded`, `config_line`, `Config::findings` | pure | `brindle`, `src/config.rs` |
| `actions::user_bindings` | pure (GPUI's keymap types, no `App`) | `brindle`, `src/actions.rs` |
| `config_edit::{custom_themes, profiles}` → `Result<_, Shape>`; `theme_entries`; `join_words` → `Result` | pure | `src/config_edit.rs`, `src/settings_dialog.rs`, `settings_dialog/profiles.rs` |
| `links::OpenError`, `links::failure_text` | pure | `src/terminal/links.rs` |
| `links::open` | shell (spawns the opener thread) | `src/terminal/links.rs` |
| `Terminal::note`, `SessionSink::note` | shell (entity; reader thread) | `src/terminal/mod.rs`, `server.rs`, `connection.rs` |
| `reconnect_gave_up`, `lost_line(name, reason)`, `layout_to_send` | pure | `src/remote/local/connection.rs` |
| `profile_for` (`Chosen`), `missing_profile_text` | pure | `src/workspace.rs` |
| `listing_from`, `SessionsModel::kill_failed`, `Listing::Unlisted` | pure | `src/workspace.rs`, `src/picker.rs` |
| `relay::Ended`, `client::StderrDrain`, `upgrade::Offered` | values returned by shell code | `src/remote/local/{relay,upgrade}.rs`, `src/remote/client.rs` |
| `parse_args_from` checks; `latency::LogState`; `session::state_dir` → `Option` | pure | `src/main.rs`, `src/latency.rs`, `src/session.rs` |
| `SessionMeta::from_value` → `Result`, `MetaFault` | codec, pure | `brindle-remote-protocol`, `holder.rs` |
| `ready::ReadyRead`, `ready_read` | pure, ungated | `brindle-remote-protocol`, `ready.rs` |
| `Pruned` | value from a seam | `brindle-remote-protocol`, `win.rs` |
| `adopt_meta`; `queue` → `io::Result` | pure; loop handler | `brindle-remote-agent`, `core/sessions.rs`, `core/{holders,clients}.rs` |
| `Conn::input_waiting` → `io::Result<bool>` | platform seam, both platforms | `server/conn.rs`, `platform/{unix,windows}/conn.rs`, `windows/pipe.rs` |
| `daemonize`, `end_its_programs`, `packaged`, `reparent` outcomes | platform seams | `platform/windows/{daemon,pty}.rs`, `conpty/{api,console}.rs` |
| `Icon::path` | pure | `brindle-ui`, `ui/src/icon.rs` |

- No decision is written in a view, a thread or a seam: each seam returns an outcome, and its caller logs or shows it (`ports R7`, `R9`; `railway R12`).
- No crate reference changes, and `tests/architecture.rs` holds as it is.
- No `cfg` is added outside a seam (`ports R4`). Decision 8 removes two from `relay.rs`.

## Decisions

### 1. The tab notice line

A failure that belongs to a running tab is one dim line in its terminal, as the "connection lost" line is today. The code says "note" (`tab_note`, `Terminal::note`), because `brindle_term_core::Notice` already names a program-status desktop notice.

**Pure (`src/terminal/tab_note.rs`):**
- `line(text: &str) -> String` gives `\r\n\x1b[2m{text}\x1b[22m\r\n`:
  - every C0 and C1 control in `text` is dropped, so text from a server, an opener or a relay can't move the cursor, retitle the tab or end the dim style (`railway R5`);
  - its lines are joined with `; `, so it stays one line.
- `NoteGate { last: Option<String> }`:
  - `admit(&text) -> bool` refuses the text that was shown last;
  - `output()` forgets it, so a repeat after the program printed anything is shown again.
  - This bounds a holder that fails on every message of one session (change 9's risk) to one line until the next output, with no clock (`fcis R2`).

**Shell:**
- **Reader thread.** `SessionSink::note(&mut self, text)` is a new trait method. `Sink::note` admits the text through its gate, then writes `line(text)` through `output`. `output` already appends to a held replay, so a notice that arrives during a replay lands after the replayed screen. `Sink::output` calls `gate.output()`.
- **UI thread.** `Terminal::note(&mut self, text, cx)` writes through parser → term → marks, as `feed` does, and notifies.
  - While the terminal is attaching (`attaching`, before its first `Replay(false)`), the notice is held in `held_notes: Vec<String>` and written after the replay, on `Replay(false)`.
  - On `Lost` or `Failed` it is written after that line, so a replay's `ESC c` never wipes it.
- **Who calls it:**
  - links (decision 3);
  - a server `ERROR` about a shown session, and an unsendable message (decision 5);
  - an unknown `-p` and a restored tab's missing profile (decision 6).
- **Event kind.** Notices from the UI thread don't go through the event channel. The reader-thread path writes through `output`, whose `Wakeup` change 9 already merges, so change 9's `EventGate` gets no new kind.

**Rejected:**
- *A toast or a status-bar component.* That is a new UI surface, and `railway R6` lists the line in the terminal as the unit's state.
- *A desktop notification.* It is a separate, rate-limited channel that the user may have turned off, and it doesn't say which tab.

### 2. The config line

Today `Settings.config_error: Option<String>` is drawn by `Workspace::render` as "Config error (using defaults): …".

**Pure (`src/config.rs`):**

```rust
pub enum ConfigFinding {
    UnknownTheme { name: String, profile: Option<String> },   // fell back to brindle-dark
    Clamped { key: &'static str, given: f32, used: f32 },     // font.size, font.line_height
    Removed { key: String, profile: Option<String>, hint: Option<&'static str> },
    DefaultNotWritten { path: PathBuf, error: String },
    UnknownAction { keys: String, action: String },
    InvalidKeys { keys: String, error: String },
}
pub struct Loaded { pub config: Config, pub findings: Vec<ConfigFinding> }
```

- **Producers:**
  - `Config::parse(text) -> anyhow::Result<Loaded>`. `normalized` returns its clamps as findings, and `parse` no longer logs.
  - `Config::findings(&self)` checks the global theme and each profile's theme against `themes` and the built-ins.
  - `load` returns `Loaded`.
  - `load_or_default() -> (Config, ConfigReport)`, with `ConfigReport { error: Option<String>, findings }`. A failed `write_default` is a finding.
- **The lookup.** `Config::theme` keeps returning a theme and stops logging. Every name it can be given came from the config, whose unknown names are findings, so its fallback is one the window reports (`railway R1`).
- **Keybindings.** `actions::user_bindings(&[(String, String)]) -> (Vec<KeyBinding>, Vec<ConfigFinding>)` is pure: an unknown action or a `KeyBinding::load` error is a finding. `bind_keys(cx) -> Vec<ConfigFinding>` binds the first list and returns the second. This closes b00 #1's `fcis R1` half too.
- **`config_line(error: Option<&str>, findings: &[ConfigFinding], notices: &[String]) -> Option<String>`** is pure:
  - an error gives today's text, with the findings after it;
  - otherwise it gives `Config: ` and the findings joined with `; `, such as `unknown theme "tokyonight" (profile Work), using brindle-dark` or `font.size 200 is out of range, using 96`;
  - `notices` are window notices that aren't config (decision 10), after them.
- **Shell:**
  - `Settings` keeps `config_findings` and a precomputed `config_line: Option<SharedString>`, rebuilt by `Settings::new` and by `reload_config` after `bind_keys`. `render` only reads it (`process R14`).
  - The banner's colour is unchanged.
  - `--dump-screen-after` prints `--- config <escaped text>` when there is one (`command-line`, "The dump shows the config line").
- **Rejected: a second banner for warnings.** One line, recomputed on every reload, is what "until the config is fixed" needs, and two would compete for the same room.

### 3. Links report what they couldn't open

```rust
pub enum OpenError {
    NoScheme,                                   // refused before anything ran
    NoThread(io::Error),                        // the opener thread couldn't start
    NotStarted { program: String, error: io::Error },
    Failed { program: String, status: ExitStatus },
    Shell { code: usize },                      // ShellExecuteW returned <= 32 (Windows)
}
```

- `links::open(uri) -> oneshot::Receiver<Result<(), OpenError>>`. `NoScheme` and `NoThread` are answered at once. The opener thread answers when the program has been started and has exited, or when ShellExecute has returned.
- `failure_text(uri, &OpenError) -> String` is pure: `couldn't open https://example.com: xdg-open exited with status 4`, or `… : it has no URI scheme`.
- **`TerminalView`'s ctrl-click** (`terminal_view.rs:511`) spawns a foreground task that awaits the receiver. On `Err`, it calls `terminal.note(failure_text(..))` on the terminal that was clicked, if the view still exists. Success is silent.
- The opener's wait stays on its own thread, so the UI never waits (`process R13`). Since `xdg-open` can outlive a browser start, a late failure still finds the tab.
- The `cfg(windows)` split of `open_default` is unchanged, and macOS (`port-to-macos`) gets the same outcome type.

### 4. Lost lines keep their cause

- **Local (b25 #18).** `reconnect_local` keeps `last: Option<String>`, the latest error from `try_connect`, `hello` or `start_core`, each still logged at `debug` or `warn` as today. At the limit it returns `reconnect_gave_up(RECONNECT_LIMIT, last.as_deref())`, a pure function: `no terminal server came back within 15 s: <last>`, or today's text when nothing failed (only timeouts). The tabs' lost line carries it through `ended`, as now.
- **Remote.** `lost_line(name, reason)`, also pure, gives `connection to A8 lost (<first line of reason>); reconnecting…` through `tab_note::line`'s control filter. `run_relay` already computes `relay_link_ended` before calling `lost_link` (`connection.rs:949-952`), so `lost_link(&name, &reason)` passes it on.

### 5. Errors about a session, and messages that can't be sent

- **A server `ERROR` with `session: Some(id)` and no request** (`connection.rs:1132-1137`, `debug` today). When the session is shown (change 2's `State::shows` by id; the core sends these only for sessions this client has attached), its sink gets `note("the terminal server: {message}")`. Otherwise it is logged at `warn`.
  - This carries change 9's holder fault reports and the holder's dropped-input refusal to the tab.
- **An encode failure in `Connection::write`** (b25 #17):
  - A request's failure is unchanged: its tab's message says `the request can't be sent`.
  - A message that names a shown session gets `note("Brindle couldn't send this session's {kind}: {error}")`, where `kind` is `input`, `resize`, `close`, `detach` or `window`. Those are `DATA`, `RESIZE`, `CLOSE`, `DETACH` and `REPARENT`.
  - `ToCore::session(&self) -> Option<SessionId>` names every variant (`railway R3`).
- **Layouts don't reach the encoder too large.** `layout_to_send(layout) -> Result<Option<Value>, LayoutTooLarge>` is pure. It refuses a layout whose JSON is over `client::MAX_LAYOUT`, which the core would drop anyway (`core/sessions.rs:207`).
  - `Running::set_layout` records `enum LayoutSent { Sent(Option<Value>), TooLarge(Option<Value>) }` instead of the plain value (b25 #17's "recorded as sent"). A layout equal to the recorded one isn't sent again, and a `TooLarge` one is logged once and retried only when the layout changes.
  - `Running::detach` sends `DETACH` with no layout when its layout is too large, so the session still leaves (`process R16`), and logs it.
  - This one is log only: the session runs on, and a session with no layout reopens as a tab of its own, `session-restore`'s fallback.

### 6. Profiles that aren't there

- **`profile_for(config, name: Option<&str>) -> Chosen`**, with `Chosen { index: usize, missing: Option<String> }`, is pure (in `workspace.rs`, beside `LaunchRequest`). It is the one rule for the first launch (`main.rs:792-797`) and `forwarded_launch` (`:567-573`). That closes b01 #10's duplicate too.
  - `LaunchRequest` gains `note: Option<String>`, built from `missing`: `no profile is named "wrk"; using Shell`.
  - `Workspace::launch` hands it to the new tab's terminal through `Terminal::note`.
  - A forwarded launch still answers `ok`, since its tab opened and says why.
- **A missing index (b07 #15).** `launch` with `request.profile` past the end opens a message tab of the default profile: `There is no profile 5: the config has 3 profiles.` (`open_message_tab`). That covers ctrl-alt-N, and a menu or picker choice made stale by a reload.
- **A restored tab (b27 #14).** Change 7 makes `resolve_tab` return `missing_profile`. `Workspace::restore` calls `note` on the tab's active zone's terminal: `this tab's profile "Work" is gone from the config; using Shell`. Decision 1's hold puts it after the zone's replay. Remote restored tabs resolve their profile by endpoint (`reattach::session_profile`) and have no such case.

### 7. The sessions palette

- **`Listing::Unlisted(String)`** is a new variant: the endpoint answered, but listing its sessions failed. `session_palette` draws it as a disabled row, `<name>  could not list sessions: <first line>`. It is not `Offline`, because this machine isn't offline, and the "No detached sessions" note is not added on its account. Every `match` on `Listing` names the variant (`railway R3`).
- **`listing_from(host, state, listed: Result<Vec<SessionInfo>, String>)`** no longer takes an `Option`. `listing_of` returns `Connecting` or `Offline` before listing. This machine's `Err` becomes `Unlisted`, and a remote endpoint's `Err` stays `Offline`, as `command-palette` "Slow host" specifies. The `"no listing was made"` text goes, since no path made it.
- **Texts (b07 #16)** name the action:
  - `listing sessions on this machine: …`;
  - `listing sessions on A8: the connection to A8 ended`.
- **Kills (b07 #14).** `list_sessions(host, kill)` returns `Listed { sessions: Result<Vec<SessionInfo>, String>, kill: Option<(u32, Result<(), String>)> }`.
  - `SessionsModel::answer` takes the kill's outcome. On `Err` it takes the session out of `killed`, so its row comes back, and keeps a `kill_failed: Option<(Host, String)>` row for that endpoint until the endpoint's next answer: `could not end session 4: <why>`.
  - The highlighted row's `RowKey` rules are unchanged.

### 8. Endpoints

- **`shared` failures (b24 #11).** `local::report_offline(host, &error)` sends `LinkState::Offline(format!("connecting to {name}: {error:#}"))` through `WATCHER`, with a fresh connection id from `NEXT_CONNECTION` (exposed as `next_connection_id()`), so `Hosts` orders it with the connection's own reports. The eager-connect thread and `when_listed` call it where they log today.
- **A relay process's exit (b26 #6):**
  - `Exit.status` becomes `Option<Ended>`, with `enum Ended { Status(ExitStatus), Unread(String) }`. A failed `wait()` is `Unread("couldn't read how ssh ended: <error>")`. A watcher thread that can't start kills the process and records `Unread("no thread could wait for ssh: <error>")`.
  - `why()` and `start()` word each case.
  - `failed_status()` and its `cfg(unix)` and `cfg(windows)` blocks go (b26 #4's `ports R4` sites). Change 5's exception list still names `relay.rs`, for its other `cfg!` uses.
- **A launch's stderr (b23 `client.rs:50-77`):**
  - `spawn_stderr_drain -> io::Result<StderrDrain>`. `StderrDrain` shares the kept bytes (`Arc<Mutex<Vec<u8>>>`).
  - `await_stderr` takes what has arrived when `STDERR_DRAIN_WAIT` passes, instead of nothing. A read error ends the drain with its text appended (`(reading stderr failed: …)`).
  - A drain that couldn't start says so in the `Ran` outcome's text, so `split_cause` still has a cause.
- **Upgrades (b26 #7).** `upgrade::offer` returns `Result<Offered>`, with `enum Offered { Upgraded(Connected), Current(Connected), Kept { connected: Connected, why: String } }`. `shared` logs `Kept`'s reason at `warn`, naming the endpoint. It is log only, as the entry says.

### 9. The settings dialog

- **The readers (b00 #8).** `config_edit::custom_themes` and `profiles` return `Result<Vec<…>, Shape>`, with `enum Shape { Unparsable(String), NotATable("themes"), NotAnArray("profiles") }`. A missing key is `Ok(vec![])`.
- **The Theme section (b04 #12).** `reload_entries` reads the file as now and then calls a pure `theme_entries(read: Result<DocumentMut, String>, fallback) -> (Vec<ThemeEntry>, Option<String>)`. The message is `reading <path>: <error>`, `<path> isn't valid TOML: …` or `themes in <path> is not a table`. The section shows it in its existing error line, with the fallback themes listed, as today.
- **The Profiles section (b05 `:558`):**
  - its read-only text is `reading <path>: <error>`;
  - `NotFound` reads as an empty document, the defaults, as `Config::load` treats it (`railway R7`).
- **`join_words` and `quote` return `Result<String, Unquotable>`.** The profile editor marks a field from the file that can't be quoted as invalid, with that reason, and blocks Save (`settings-dialog`, "Profile field validation"). Today the text it shows would change the profile when saved.
- **`has_font_family` (b06)** returns `io::Result<bool>`, converted at the seam (`ports R8`, `R9`). `platform_has_family` keeps the family on `Err` and logs once per process.

### 10. Command line, latency, state directory

- **`parse_args_from`** calls `usage_error` for:
  - every option missing its value: `-p`, `-d`, `--config`, `--send`, `--action`, `--find`, `--wait-for`, `--key`, `--hover`, `--click`, `-e` with no command;
  - `--dump-screen-after` without a number.
- **The steps are checked at parse time:**
  - `--action` through `actions::action_by_name`;
  - `--key` through `Keystroke::parse`;
  - `--hover` and `--click` through `PointerStep::parse`.
  - `Step::Pointer` then holds a `PointerStep`, not a `Result` (`types R7`). The run loop's `log::error!` arms go, and its lookups `expect("checked when parsed")` (`railway R9`).
- **Latency (b01 #26).** `latency.rs` keeps `enum LogState { Writing(File), Stopped { path, error } }` in place of `Option<File>`. The summary prints `--- latency-log stopped "<path>": <error>` after the samples header. The prefix differs from `--- latency `, so `tests/perf/report.rs::parse_latency` still finds the header first.
- **State directory (b27 #13).**
  - `session::state_dir() -> Option<PathBuf>`: `None` when `BRINDLE_STATE_DIR` is unset and neither the platform's directory nor a home directory exists.
  - `main` then runs as a debug launch beside an instance does: on its own, no `session.lock`, no restore, no reattach, no recording.
  - It adds the window notice `Brindle can't save its session: no state directory (set BRINDLE_STATE_DIR)` to the config line's `notices` (decision 2).
  - The callers change 4 and change 8 give `state_dir` (`StateFile`, `read_launch`) take the `Some` path.

### 11. Terminal server, helper and protocol

- **`SessionMeta::from_value(&Value) -> Result<SessionMeta, MetaFault>`** (b20 #9):
  - `MetaFault { malformed: Vec<&'static str>, read: SessionMeta }` names each field of the wrong type, or `"meta"` for a non-object.
  - `read` holds the well-formed fields, and the defaults only where a field was malformed.
  - A missing field is still its default (`Ok`): the additive rule for a meta an older core wrote (`process R5`). That is now the only defaulting path.
- **The core's adoption** calls a pure `adopt_meta(Result<SessionMeta, MetaFault>) -> (SessionMeta, Option<String>)`.
  - On a fault it keeps `read`, but with `owner` empty and `policy: Detach`. An unreadable owner can't come back, and `Kill` with no owner would end the program after `RECONNECT_GRACE`.
  - It also gets a fresh `key` if the key was malformed.
  - It returns the `core.log` line naming the session and the fields.
  - The session is then listed as detached and can be attached (`railway R6`: shown on its endpoint). The core sends `SET_META` with the repaired meta, so the next core reads it clean.
- **`queue` → `io::Result<()>`** in `core/holders.rs` and `core/clients.rs` (b11 #11, b10 smell). For the holder:
  - a failed `SPAWN` fails the waiting spawn with an `ERROR` to its client: `the terminal server couldn't pass the request to its holder: <error>`. Brindle shows that as "Brindle could not start…";
  - a failed `SET_META` leaves `storing` unset, so no replay waits on bytes that never went out;
  - any other message for a session sends that session's attached client an `ERROR` with the session.
  For the client, a reply that can't be encoded is replaced by an `ERROR` for its `req`, which is always small.
- **Texts (b10 #13):** `can't read the server directory: …`, `can't create the event loop: …`, `can't set a read limit on the core connection: …`, and so on. Each names its action, as `core.rs:137-141` already does.
- **`Conn::input_waiting() -> io::Result<bool>`** on both platforms (b14 #8):
  - Windows returns `port.pump`'s and `PeekNamedPipe`'s errors;
  - Unix returns a failed `FIONREAD`;
  - `holder_behind` reads `Err` as "behind", so the next read meets the error and `drop_holder` logs its cause.
  - The test helper calls in `conn.rs` take `.unwrap()` (test code).
- **The ready pipe (b13 #3).** `ready::ReadyRead { Pending, Listening, Ended, Failed(io::Error) }` and `ready_read(io::Result<usize>) -> ReadyRead` are pure, in `remote-protocol/src/ready.rs` beside the Linux rule, and tested on both platforms. `ReadyPipe::read` returns it, and the starter's "didn't start" error appends `reading the ready pipe failed: <error>`.
- **`daemonize` (b13 #5)** writes one line to the role's log for a failed `SetConsoleCtrlHandler` or `set_current_dir`, and carries on.
- **The relay's build id (b15 `relay.rs:256`).** `Server.build: Result<String, String>`. An `Err` writes one `core.log` line through `server.log`, and `RELAY_HOST` still sends `build: ""`, the wire's "unknown" (`process R5`).
- **`end_its_programs` → `io::Result<usize>`** (b15 `pty.rs:290`). `kill_remaining` passes it up, and the holder logs a failed snapshot with the session id.
- **ConPTY (b09 #4–#7):**
  - `packaged() -> Result<Api, String>`;
  - `PACKAGED` caches only `Ok`, so a failure is logged and tried again at the next spawn (`railway R6`'s "not cached");
  - `reparent -> io::Result<Reparented>`, with `enum Reparented { Done, Unsupported }`. The holder logs `Unsupported` once and an `Err` each time, at its `REPARENT` arm;
  - `missing(name)` names the export.
- **`prune_helper_copies` → `Pruned { removed: usize, kept: Vec<(PathBuf, String)> }`.** The core's `prune_copies` logs what it kept and why. Change 10's pure `pruned_builds` still decides what to try.

### 12. Test harnesses (b32, b36)

- `tests/e2e/brindle.rs:559`: a timeline thread that panicked is an error, `the timeline thread panicked: <payload>`.
- `list()` `ensure!`s `brindle-remote list` succeeded, with its stderr. The kill's status is checked, so `hang_up_detached` reports a session it couldn't end.
- `capture.rs::save_ppm` returns `io::Result`, and callers `?` it.
- `tests/perf/main.rs:209` prints the `summary.toml` write error and exits 1.
- `winproc::all` and `elevated` return `io::Result` (after change 4 moves them to `tests/common/winproc.rs`). The leftover-role check fails on `Err`.
- Each is held by its suite's `--self-test` where it is pure (`fcis R8`; change 7's D3), and otherwise by the cases that use it.

### 13. The icon path (b41 #3)

`Icon::path(&self, side, min_stroke) -> Result<Path<Pixels>, PathError>` is split out of `paint` and is pure. A unit test builds every `IconName` at the sizes the components use. `paint` logs an `Err` once per icon name (a `Mutex<HashSet>` in the crate) and paints nothing.

### 14. Wire, versions, files (`process R4`–`R6`, `R15`)

- **Wire:** none. No message, field or version changes in client protocols 1–3 or H1.
  - `SessionMeta`'s encoding is unchanged. Only its decoder's return type changes, and its transcripts (`holder-set-meta-layout` and the rest) decode to `Ok`.
  - Change 3's fuzz target for `from_value` asserts `Ok` or `Err(MetaFault)`, never a panic.
  - `RELAY_HOST.build` stays `""` for "unknown".
- **Versions that meet:**
  - an older core's meta is missing new fields, which is `Ok`;
  - an older Brindle gets the same `ERROR`s it handles today.
- **Config:** no key changes. The findings read the same file.
- **State file:** no format change. Without a state directory nothing is read or written.

### 15. Core restart, Brindle crash, upgrade (`process R1`–`R3`, `R12`)

- **A core restart** adopts with `adopt_meta`. A session whose meta was malformed comes back detached instead of with a default owner and policy, so a restart never ends a program because of a bad meta. The repaired meta is stored, so later restarts read it clean.
- **A Brindle crash** loses held notices and the palette's kill row, which are process memory. Nothing persisted changes.
- **An upgrade:** holders of older builds keep returning nothing from the seams they have. New behaviour in the holder (ConPTY retries, the reparent and snapshot logs) runs in holders of the new build, as change 9 says for its guards. The core's adoption rule takes effect at the next core start.

### 16. Tests, by layer (`testing`)

- **Unit, pure, in each file:**
  - `tab_note::line`'s control filtering and line joining; `NoteGate`;
  - `ConfigFinding`s from `parse` (a clamp, an unknown global and profile theme, a removed key), `user_bindings`, `config_line`;
  - `config_edit`'s `Shape` cases, `theme_entries`, `join_words`;
  - `links::failure_text`, `reconnect_gave_up`, `lost_line`, `layout_to_send`, `profile_for`;
  - `listing_from`, `session_palette` with `Unlisted`, `SessionsModel` with a failed kill;
  - `parse_args_from`'s usage errors (it already takes the argument list), `LogState`, `state_dir` with an injected environment (after change 7's parameter);
  - `SessionMeta::from_value` (malformed, non-object, missing fields), `adopt_meta`, `ready_read`, `Icon::path`.
- **Fake link** (`connection.rs`, `fake::link`):
  - a session `ERROR` reaches the shown sink as a notice, once for a repeat;
  - an `ERROR` for a session not shown is not delivered;
  - a too-large layout isn't sent and `DETACH` goes without it;
  - a remote loss's line carries the reason.
- **Reader-thread sink** (`terminal/server.rs`): a notice during a held replay is written after it.
- **Real processes** (`remote-agent/tests/core.rs`): a holder whose stored meta has `"owner": 7` is adopted with the session listed detached and its program running.
- **e2e:**
  - new cases:
    - `reload-config-findings`: `theme = "tokyonight"`, `font.size = 200` and a binding to `split_zone_rihgt`, all named on the `--- config` line;
    - `links-opener-fails`: `BRINDLE_LINK_OPENER` names a missing program; `--click ctrl-…` on a URL; the screen shows the notice;
    - `local-server-session-error`: change 9's `BRINDLE_TEST_HOLDER_PANIC` on input, the notice on screen, the session still taking input;
    - `profile-shortcut-missing`: `--key ctrl-alt-5` with one profile gives a message tab;
    - `launch-unknown-profile`: `-p wrk`;
    - `local-server-no-core-back`: holder and core killed, the private server directory made unreadable, and the lost line naming the error, with the mode restored by the case's cleanup;
    - `local-server-palette-unlisted`: `BRINDLE_SERVER_HELPER` names a missing file, so no core can start; the palette's row says this machine's sessions couldn't be listed, and there is no "No detached sessions" row;
  - extended: `remote-ssh-network` (the lost line names its cause) and `restore-fallbacks` (its restored tab of the removed profile `Removed` shows the notice).
- **Not end to end:** a failed kill, an unencodable message, a failed `wait()`, a failed ready-pipe read and the Windows seams can't be caused on purpose from outside. Their outcomes are held by the unit and fake-link tests above.
- **Manual:** a real ctrl-click on a link in a tab with no `xdg-open` handler, on Linux and on Windows. `--click` drives the same path, but only a real pointer goes through the platform's events.

## Risks / Trade-offs

- [A notice line lands in a full-screen program's screen] → The same as today's "connection lost" line and change 9's input-refusal line. It is one line, and the program's next redraw replaces it.
- [The config line now shows removed keys that were only logged] → They are keys the file still sets and Brindle ignores, which "Invalid config falls back to defaults" says are reported. Each names the key and how to remove it.
- [A server `ERROR` about a session could be noisy] → `NoteGate` shows a repeat once until the session prints again. These `ERROR`s are rare by construction: change 2's guard stops a lost tab from sending.
- [A malformed meta now leaves a program running that a default `Kill` would have ended] → Keeping a program is the server's job (`process R1`). The session is listed, so the user can attach it or end it.
- [`-e` with no command is now a usage error] → It did nothing before. Scripts that pass it get a message instead of a plain shell.
- [Usage errors for debug steps can break a test that passed a bad step and relied on it doing nothing] → That is the point. The e2e suite is run per group under `scripts/with-xvfb`, and any such case is fixed in the same group.
- [Seven PRs touch many files that sibling changes also touch] → Each group names the changes it waits for. The overlaps are textual except those listed under "What the plan got wrong" (4).

## Migration Plan

Nothing to migrate: no wire, config-key or state-file format change. Each group can be reverted alone. A revert of group 1 brings back `from_value`'s defaults, so change 3's fuzz target loosens with it.
