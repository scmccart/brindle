# Remediation plan for the 2026-11-16 audit

- **Report:** [`2026-11-16.md`](2026-11-16.md), of commit `ab6462b`. Instance lists in [`2026-11-16/`](2026-11-16/); "b25 #13" below means item 13 of `b25.md`'s Violations.
- **Planned against:** `main` at `39bb579`. `git diff ab6462b 39bb579` touches only the audit's own files, so every cited `file:line` is still current; the sites the plan acts on were opened again on `39bb579` (list under "Re-checked").
- **Output:** one OpenSpec change per group below, in this order, each sized for one PR.

## Decisions (answered 2026-11-16)

These are rule and doc changes, not fixes. Each was put to the user with a recommendation; all four recommendations were taken.

| # | Question | Decision | Lands in |
|---|---|---|---|
| D1 | `ports R4` against `testing`'s platform pairs: exempt test code, or name the test support files as seams? | **Exempt test code.** `ports R4` binds production code; `#[cfg(test)]` modules, `tests/**` and `remote-agent/tests/**` are out of its scope, and the new check skips them. | `enforce-platform-seams` |
| D2 | `process R13` against CLAUDE.md on the launch-time `running_sessions()` (`src/main.rs:863`). | **Move it off the UI thread**, as the instance listener (`main.rs:767`) already does; R13 stays as written; CLAUDE.md's Reattach section is rewritten. | `list-sessions-off-ui-thread` |
| D3 | Does `fcis R8` cover pure helpers in the e2e and perf harnesses? | **No, product code only.** Harness helpers are held by the harnesses' `--self-test`; `fcis` and `testing` say so. | `keep-pure-modules-pure` |
| D4 | Relay gate: `cfg(unix)` or `target_os = "linux"`? | **`target_os = "linux"`** in both `remote-agent/src/server/mod.rs` and `remote-agent/src/main.rs`; the docs name it. macOS refuses `relay` as Windows does. | `enforce-platform-seams` |

## Re-checked

- **Unchanged and acted on:** `src/session.rs:267`, `:719-730` (F1); `tests/perf/cases.rs:27-42`, `tests/e2e/cases/restore.rs:130`, `tests/e2e/cases/remote.rs:1957`, `tests/e2e/cases/instance.rs:37` (F2; the report cites it as `tests/e2e/instance.rs:39`); `src/remote/local/connection.rs:276-281`, `:1217-1233`, `:1279-1282`, `:1812-1849` (F3); `remote-protocol/src/holder.rs:536-600` (F4a); `src/workspace.rs:1748-1752` (F5); `src/main.rs:863` (F6/D2); `src/remote/local/connection.rs:2874` (`never_loop`); `term-core/src/shell_integration.rs:110`.
- **Already fixed:** none. No product code changed between the audit and this plan.
- **Dismissed in the report, still dismissed:** `remote-agent/src/server/core.rs:705`, `:781` (`ports R5`): test code.

### Finding 3 traced: reachable, narrower than stated

- A new core **keeps** adopted sessions' ids (`core/holders.rs:368-423`: "ids continuing after the largest"), and accepts clients only after adopting the startup holders. So an id is reused only when **no holder that held it survives**: the holder crashed or was killed, or the host rebooted (a relay endpoint), and then some client spawned new sessions in the fresh core before this Brindle's reconnect listed.
- The core gates `INPUT`, `RESIZE` and the rest on `session.attached == Some(client)` (`core/sessions.rs:801-815`), so keystrokes never reach another client's session.
- What can go wrong:
  1. **A lost tab isn't shown lost.** `kept` (`connection.rs:1220-1221`) takes every listed session that has *any* client (`SessionInfo.attached` is `!detached`, `core/manage.rs:45`). If another client holds the reused id, this tab's sink stays in `State.sessions`, frozen, with no "session lost" line.
  2. **A carried attach takes another session.** A non-recorded attach in flight when the link dropped is matched by bare id (`carried_attaches`, arm `(None, Some(_))` at `:1840-1844`): if the reused id is listed without a client, the `ATTACH` is sent again and the tab is fed the other session's replay.
- **Plan:** fix it (change 2). It needs a second client on the same endpoint and a lost holder, so it is rare, but the result is a tab showing the wrong program or a dead tab that looks alive.

### Finding 4b traced: not reachable; kept as hardening

- In GPUI 0.2.2, the Linux (X11, Wayland) and Windows backends never call `text_for_range`. Every range they pass to the input handler is `None`, the handler's own `marked_text_range()` or `selected_text_range()` (both built from `EditState`, ordered), or `pos..pos` (Windows composition caret, `platform/windows/events.rs:676-680`). The macOS backend (`port-to-macos`) calls it with an `NSRange` (location, length), which is ordered too.
- **Plan:** no defect fix. `railway R5` still asks that input from outside the process never panic, so ordering the range in `EditState::range_from_utf16` is a one-line hardening with a unit test, carried in change 3.

## The changes

Order: defects (1–4), what stops new breaches (5–6), what unblocks other work (7–10), the long tail (11–13). Sizes: S < ½ day, M ≈ 1–2 days, L more. "Spec delta" means the change alters behaviour and carries a delta against `openspec/specs/`.

### Overview

| # | Change | Findings / rules | Size | Depends on | Spec delta |
|---|---|---|---|---|---|
| 1 | `keep-session-file-on-serialize-error` | F1; `railway R1` | S | — | `session-restore` |
| 2 | `carry-session-identity-and-host` | F3, F5; `types R3`, `R7`, `R8` | S–M | — | `remote-connections`, `configuration` |
| 3 | `bound-and-fuzz-outside-input` | F4; `railway R5`, `process R6` | S–M | — | none |
| 4 | `isolate-test-runs` | F2, part of F11; `testing R4`, `ports R10` | S | — | none |
| 5 | `enforce-platform-seams` | `ports R4` exception, D1, D4, `fcis R7` (term-core) | M | — | none (rule + check) |
| 6 | `add-clippy-to-ci` | F7 (`never_loop`), enforcement gaps | S–M | — | none (check) |
| 7 | `keep-pure-modules-pure` | F13 (`fcis R2`/`R3`/`R5`), D3, doc drift | M | 5 (architecture-test layout) | none (refactor + check) |
| 8 | `list-sessions-off-ui-thread` | F8 (`process R13`/`R14`), D2 | M | — | `session-restore` |
| 9 | `bound-queues-and-contain-panics` | F8 (`process R9`, `R3`, `R10`, `R15`) | M | — | `remote-connections`, `terminal-emulation` |
| 10 | `test-platform-logic-everywhere` | F9; `fcis R7`, `ports R7` | M | 5 | none (refactor + tests) |
| 11 | `deterministic-tests` | F11 rest; `testing R7`, `R8`, `R5`, `R3`, `R9` | M–L | 4 | none (tests + check) |
| 12 | `surface-swallowed-failures` | F7; `railway R1`, `R6`, `R8` | M | 2 (key/listing types), 9 | `configuration`, `links`, `remote-connections` |
| 13 | `extract-decisions-into-models` | F6, F10, F12; `fcis R1`/`R10`/`R8`, `types R1`/`R7`/`R3` | L (lands in passes) | 7, 12 | none (refactor + tests) |

The overlap with changes in flight:
- **`port-to-macos`** is what ends the `ports R4` exception. Change 5 narrows that exception to an explicit list whose "Ends with" stays `port-to-macos`, and D4 means macOS refuses `relay`; change 5 adds a note to `port-to-macos`'s design rather than editing its tasks.
- **`add-blocks`** will rework `terminal_view.rs`/`terminal_element.rs`; change 13 leaves those two files' `fcis R1` entries to it (it must follow `fcis` anyway) and only adds the `fcis R8` tests they lack.
- `add-pocket-client` and `update-dependencies` don't overlap.

### 1. `keep-session-file-on-serialize-error`

- **Closes:** F1, `railway R1` (b27).
- **Scope:** `src/session.rs:265-270` `State::to_text` → `Result<String, toml::ser::Error>`; `src/session.rs:718-731` `pending_write` returns `None` and logs on `Err`, leaving `last_written` alone; the quit path's final write the same.
- **Behaviour:** yes. Delta on `session-restore` "State file location and writes": a state that can't be serialized leaves the file as it was.
- **Verify:** unit test in `session.rs` with a state that fails to serialize (or an injected serializer) asserting `pending_write` is `None` and `last_written` is unchanged; existing session tests.

### 2. `carry-session-identity-and-host`

- **Closes:** F3 (b25 #13; with b25 #14 and #15, the key and size sentinels the fix reads), F5 (b07 #24); `types R3`, `R7`, `R8`.
- **Scope:**
  - `src/remote/local/connection.rs`: `State.sessions` keeps each shown session's key beside its sink; the after-loss prune (`:1217-1233`) keeps a sink only when the listing has its id **with its key**; `carried_attaches` (`:1812-1849`) requires the key for carried attaches too; `attached_by_replay` (`:1276-1290`) stays as is: a reconnect's replay comes only for sessions the core gives back to this client's token, and `REPLAY_START` carries no key, so adding one would be a protocol change this plan doesn't need. Key as `Option<SessionKey>` decoded at the boundary (b25 #14, `:207`, `:1822`, `:1879`) and listed size as `Option<(u16, u16)>` (b25 #15, `:1826`). Wire structs unchanged (`process R4`).
  - `src/workspace.rs:1748-1752` `open_config`: launch the editor on `Host::Local` under a local profile whatever the default profile targets.
- **Behaviour:** yes. Delta on `remote-connections` "Sessions come back after a reconnect": a session whose id the endpoint now uses for another session (another key) is shown lost. Delta on `configuration` "Live reload" (ctrl-, opens the config): the editor always runs on this machine.
- **Verify:** table tests for the prune and `carried_attaches` as pure functions over `State` (they have none today, b25 #8); a `fake::link` test for the reused-id case; a unit test that `open_config`'s request is local with a remote default profile; e2e `remote-*` unchanged.

### 3. `bound-and-fuzz-outside-input`

- **Closes:** F4; `railway R5` (b20 #10, b43 #1, b31 #9), `process R6` (b20 #11, b22 #1–#3, and the coverage smell in b22).
- **Scope:**
  - `remote-protocol/src/holder.rs:536-600` `Assembler`: cap open blobs (a constant, refused past it with the same error path as an over-long blob).
  - `ui/src/text_input.rs:277` `range_from_utf16`: order the range (hardening; see "Finding 4b traced").
  - `remote-protocol/src/fuzz.rs`: targets for `skip_preamble` (`src/remote/local/link.rs`), `SessionMeta::from_value`, the `semver` parse used by upgrade, `parse_holder_lock`; a term-core fuzz test for the OSC 7, 133 and 7501 parsers.
  - `remote-protocol/tests/transcripts.rs`: transcripts for `Exit.lost = Some(..)`, `Previous::Crash`, `ExitStatus { code: None }`; coverage checked per variant, not per message type; shipped fixtures refused if regenerated differently (`process R5`); module doc says protocols 1–3.
- **Behaviour:** no user-visible change (a holder past the cap is a broken holder).
- **Verify:** the fuzz tests and transcripts themselves; a unit test for the cap and for a reversed range.

### 4. `isolate-test-runs`

- **Closes:** F2 (b36 #1, b34 #1–#2, the `Launcher` smell in b33), `ports R10` (b24 #4, the 8 `set_var`/`remove_var` sites in m2), `testing R7` shared state (b27 #23's write-generation static), the scopes test's `BRINDLE_NO_SCOPE=0` (b16 smell).
- **Scope:** `tests/perf/cases.rs:27-42` gets a private `/tmp` server directory, `BRINDLE_NO_SCOPE=1` and a stop-and-check step (moved into `tests/common/` so e2e and perf share it); `tests/e2e/cases/restore.rs:130` and `remote.rs:1957` go through `Run`; `tests/e2e/cases/instance.rs:37` sets `BRINDLE_NO_SCOPE=1`; `src/remote/local.rs` and `src/remote/client.rs` tests take the variable as a parameter instead of `set_var`; `src/session.rs`'s generation counter becomes per-store; `remote-agent/tests/core.rs:1746` runs systemd-run against a private user manager or is opt-in.
- **Behaviour:** no.
- **Verify:** `BRINDLE_PERF=1 cargo test --test perf -- --self-test`; e2e `restore`, `remote`, `instance` cases under `scripts/with-xvfb`; `cargo test` with `--test-threads` default.

### 5. `enforce-platform-seams`

- **Closes:** the stale `ports R4` exception; D1; D4; `fcis R7` b31 #2 (`term-core/src/shell_integration.rs:110`); doc drift "relay gate".
- **Scope:**
  - **Rule (D1):** `ports R4` and `testing` say R4 binds production code; test code (`#[cfg(test)]`, `tests/**`, `remote-agent/tests/**`) may pair platforms.
  - **Exception:** `docs/architecture/README.md`'s `ports R4` row becomes an explicit list of production files from the audit's Exceptions section and m1 (`src/remote/local.rs`, `src/notify/mod.rs`, `src/remote/local/relay.rs`, `src/terminal/mod.rs`, `src/main.rs`, `src/instance.rs`, `src/workspace.rs`, `src/settings.rs`, `src/session.rs`, `src/remote/mod.rs`, `src/remote/client.rs`, `src/desktop.rs`, `src/terminal_view.rs`, `src/config_edit.rs`, `remote-agent/src/main.rs`, `remote-agent/src/server/{core.rs,daemon.rs,conn.rs,cli.rs,mod.rs}`, `remote-agent/src/server/core/sessions.rs`, both `build.rs`), each re-checked against m1 for non-test sites; seam module declarations stay allowed. Ends with `port-to-macos`.
  - **Code:** term-core's `windows_paths` becomes a `Parser` constructor argument passed by the app from the session's `Host`; the relay's gate is `target_os = "linux"` in both places (D4).
  - **Check:** `tests/architecture.rs` rejects `cfg(unix|windows|target_os…)` and `cfg!(…)` in production code outside the seam set and the exception list, skipping `#[cfg(test)]` modules and test targets; a positive-direction test that it finds a planted site.
- **Behaviour:** no (macOS isn't a target yet).
- **Verify:** the new architecture test; term-core unit tests for both `windows_paths` values; `cargo test` on Linux and Windows CI.

### 6. `add-clippy-to-ci`

- **Closes:** report finding 7 (`never_loop`, `src/remote/local/connection.rs:2874`), the e1 clippy failure and its ~90 warnings, the three `too_many_arguments` allows without a `reason` (`src/zone_view.rs:432`, `src/terminal/mod.rs:333`, `:403`).
- **Scope:** fix `never_loop`; fix or `#[allow(..., reason = "…")]` each warning (by the e1 counts); a `clippy` job in `.github/workflows/ci.yml` (`cargo clippy --locked --workspace --all-targets -- -D warnings`) on Linux and in `build-windows.yml`; then, in this change, `clippy::wildcard_enum_match_arm` on Brindle's crates (allowing foreign-type matches with a reason) and `unwrap_used`/`expect_used` on non-test code of `remote-protocol` and `term-core`. Where the wildcard lint finds `_ =>` on Brindle's own enums inside the `railway R3` exception, the arm gets a reason-tagged allow and the exception row says the lint now lists them.
- **Behaviour:** no.
- **Verify:** the CI job passing on both platforms; `scripts/lint-commits`.

### 7. `keep-pure-modules-pure`

- **Closes:** F13's `fcis R2`/`R3`/`R5` entries: b01 #5–#6 (`src/layout.rs:69-91` `origin_id`), b03 #7 (`settings.rs:86-102`), b07 #8–#9 (`workspace.rs:2252-2280`, `:2491-2495`), b11 #3 (`core/sessions.rs:158`), b20 #1–#2 (`scope.rs:127-133`, `server_dir.rs:46`), b24 #12 (`remote/local.rs:281-291`), b25 #6–#7 (`connection.rs:331, 338`, `:1737-1755`), b27 #8, #9, #11 (`remote/mod.rs`, `session.rs:446-459`, `:503`), b28 #1 (`terminal/cmdline.rs:48`, the `Path::is_file()` the `fcis` doc's pure list contradicts). D3. The architecture-test gaps from the report's Enforcement section.
- **Scope:** each function takes `now`, the environment value or a probe as an argument; the caller gathers. `tests/architecture.rs` gains a search rejecting `Instant::now`, `SystemTime::now` and `std::env` in the modules `fcis` lists as pure; its needles learn grouped imports (`use a::{b, c}`) and full paths; its dependency checks read `[workspace.dependencies]`, `[patch]` and `[replace]`; its doc comment stops claiming a direct `railway R2` check; it checks each opt-in suite has `--self-test` (the perf suite's missing `--shard` goes to change 11). Docs: `fcis` R8 says it covers product code, harness helpers are held by `--self-test` (D3); the `cmdline` purity claim becomes true by the code change.
- **Behaviour:** no.
- **Verify:** unit tests for every function given a new argument (they become testable without a clock); the new architecture tests with positive-direction checks.

### 8. `list-sessions-off-ui-thread`

- **Closes:** `process R13` b01 #27 (`src/main.rs:863`), b01 #28 (`main.rs:495-515`), b02 #9 (`notify/mod.rs:231-235`), b02 #10 (`notify/activation.rs:98-99, 118`), b24 #1 (`remote/local.rs:379-401`); `process R14` b01 #29 (`find_bar.rs:255-261`); D2.
- **Scope:** the launch lists detached sessions on the background executor and opens the window when the listing answers or fails (within the same 2 s limit), applying `Detached` and the restore from that answer; the other sites move their blocking call off the UI thread the same way; CLAUDE.md's Reattach section is rewritten to match.
- **Behaviour:** yes, slightly. Delta on `session-restore` "Launch reattaches detached terminals": the listing happens before the restore is applied but no longer blocks the UI thread. Restored tabs are the same.
- **Verify:** a pure test of the launch decision given a listing (`Some`/`None`); e2e `local-server-*` and `restore` cases (they cover reattach on launch); a `--latency-log` look at time to first paint is a manual check.

### 9. `bound-queues-and-contain-panics`

- **Closes:** `process R9` b24 #2 (`terminal/server.rs:58, 96, 107, 113`), b28 #9 (`terminal/mod.rs:104, 112, 413, 443, 481`), b25 #19 (`connection.rs:442, 690-693`), `src/latency.rs:232-239` (b01 smell); `process R3` b24 #3 (`terminal/server.rs:72, 92`) and the holder's `handle`/`spawn` (b12 smell); `process R10` b10 #14 (`core/clients.rs:126-132, 154-156`); `process R15` default config write in `src/config.rs` (b26).
- **Scope:** the terminal event channel gets a high and low mark (events already coalesce, so it drops only redundant `Wakeup`s); the connection's outgoing queue counts bytes, and input past the high mark while the link is down is refused with a line in the tab; latency samples keep a bounded window; `SessionSink`'s `parser.advance` runs under `catch_unwind`, marking that terminal broken with a message; the holder's per-message handling the same, ending only that session; refused connections are shut down, not dropped; the default config is written with `write_atomic`.
- **Behaviour:** yes. Delta on `remote-connections` "Sessions come back after a reconnect" (input typed while disconnected is delivered up to a bound, past which the tab says it was dropped); delta on `terminal-emulation` "A session lost for good is reported in the tab" (a parse panic is reported in that tab, others continue).
- **Verify:** unit tests for the marks; a `BRINDLE_TEST_FEED_PANIC`-style injection test for `SessionSink` and the holder (`remote-agent/tests/holder.rs`); a core test that a refused client sees EOF.

### 10. `test-platform-logic-everywhere`

- **Closes:** F9; `fcis R7` and `ports R7`: b14 #1, #7 (`windows/listen.rs:104`, `windows/pipe.rs` decisions), b13 #1, #6 (`daemon.rs:155` `parse_ready_handle`, `dir.rs:38-67`), b19/b21 (`remote-protocol/src/{endpoint,win,access}.rs` `pipe_name`, `check`, `Dacl::allows_only`, b20 #3, b21 #2–#3), `prune_helper_copies` and the copy-race decisions (b14, b21), b09 #3 and b15 (`conpty/api.rs`, `program_name`, `environment`, the choice of processes to kill), b10 #3–#4 (`pty/unix.rs`), b01 #7 (`desktop.rs:399-406`), b02 #4 (`notify/dbus.rs:134`), b07 #10–#11 (`workspace.rs:2258`, `:2505-2508`), b28 #3 (`terminal/mod.rs:1214-1219`).
- **Scope:** each decision moves to a platform-neutral module beside its seam (as `conpty::cmdline::program_stem` is), the seam keeping only the call; unit tests run on Linux and Windows.
- **Behaviour:** no.
- **Verify:** the moved functions' unit tests run in Linux `cargo test`; Windows CI's `cargo test` and `windows-e2e`.

### 11. `deterministic-tests`

- **Closes:** the rest of F11: `testing R7` sleeps (b16 ≈14, b17 `holder.rs:1449`, `manage.rs:184`, b18 #5, b25 #20–#21, b26 #9–#11, #13, b27 #23, #25, b28 #12–#13, b00 #12, b09 #10, b10 #15, b13 #7, b14 #3–#4, b33 #2–#8), `testing R8` (b16 ≈6, b18 #4, b21 `pipe.rs` 20 s bound, b14 #5, b25 #22–#23, b26 #12, b34 #4, b35 #6, b13 #8), `testing R5` (b33 #1 `notify.rs:37`, b34 #3, b35 #2–#5), `testing R3` silent skips (b12, b16, b17 ConPTY test; b36 #2 perf `--shard`), `testing R9` (b35 #7 `tabs.rs` `face_top` flake; b27 #24).
- **Scope:** one `eventually(limit, condition)` helper per test crate; every e2e delay through `ctx.secs`; skips reported as skips; the `tabs.rs` flake reproduced and fixed with numbers (`testing R10`) or filed; `tests/architecture.rs` rejects `thread::sleep` in `remote-agent/tests/**` and `tests/e2e/cases/**` outside the named, scaled helper.
- **Behaviour:** no.
- **Verify:** `cargo test --workspace` looped in parallel (numbers in the PR); the e2e suite under `scripts/with-xvfb` with `BRINDLE_E2E_SLOW=2`; the new architecture test.

### 12. `surface-swallowed-failures`

- **Closes:** F7; `railway R1` (21 + entries in b07, b20, b23–b28), `R6` (b07 #15, b09 #5, b24 #11, b25 #17, b26 #7), `R8` (b01 #22, b07 #16, b09 #7, b10 #13, b13 #3, b25 #18).
- **Scope:** `src/config.rs` (unknown theme, clamped font size and line height) report to the settings error line and the reload notice; `src/terminal/links.rs` `links::open` failures reach the tab that clicked; `src/workspace.rs` `listing_from` shows the endpoint's failure row instead of an empty listing; `connection.rs` `reconnect_local` keeps the last cause in the lost line, and an encode failure un-records a layout as sent; `SessionMeta::from_value` returns a refusal for malformed fields instead of defaults; the rest by batch entry.
- **Behaviour:** yes. Deltas on `configuration` ("Themes", "Invalid config falls back to defaults": the fallback is named), `links` ("Opening links": a failed opener is reported), `remote-connections` ("Lost connections reconnect": the lost line names the cause).
- **Verify:** unit tests per function for the error value; e2e `reload`, `links` and `palette` (sessions) cases for what the user sees.

### 13. `extract-decisions-into-models`

- **Closes:** F6 (`fcis R1` 52, `R10` 17, `R9` 10, `R3` the rest), F10 (`types R1` 32, `R7` 37, `R5`, `R6`, `R12`, and the non-session `types R3` index identity: b04 #7, b39 #4, b40 #1–#2, b41 #1, b07 #19–#20, b01 #16–#17, b03 #16, b24 #6, b27 #18, b00 #11), F12 (`fcis R8` 33).
- **Scope:** scoped to the hotspots, one task group per area so it can land in passes (one PR per group, the change archived after the last); the rest stays on the README's "brought into line when next changed" policy and is listed in the change's design:
  1. `src/workspace.rs` (b07): a `Tabs` model for index arithmetic, the new tab's cwd policy and overlay exclusivity.
  2. Settings dialog (b04, b05): section models that return edits for the shell to write; selection keyed by item id.
  3. `src/remote/local/connection.rs` `State` (b25 #1–#5, #8–#12, #16): `connect_failed`, `idle_step`, `link_lost`, `retry_cap`, `route_*` as tested methods; `Sent` and `Link` enums; `Option<Negotiated>`.
  4. Server core handlers (b10–b12): `tick`'s exit and grace decisions and `should_exit` as pure functions with tests.
  5. `src/terminal/mod.rs` and `src/remote/local/session.rs` lifecycle flags (b23, b25) as one enum each.
  6. `brindle-ui` (b39–b42): `Select`, `Combobox`, `Menu`, `Tabs` keyed by item id; `Option<Bounds>` for an unmeasured trigger; `Menu::label` → `Option`; overlay depth → `Option`.
  7. `fcis R8` tests for pure code with none: `fit_grid`, `wants_links`, `Theme::indexed` (b06), the core's helpers (b10–b12), the rest by entry. `terminal_view.rs`/`terminal_element.rs` `fcis R1` entries go to `add-blocks`.
- **Behaviour:** no.
- **Verify:** unit tests in each new model's file; existing e2e suite unchanged.

## Not planned

- **SHOULD-only entries** (`types R10`/`R11`, `railway R9`, `ports R8`/`R11`, `fcis R9` beyond change 13): fixed when their files change, under the README's adoption policy.
- **Design smells** beyond those folded in above (holder `catch_unwind`, latency samples, transcript coverage, silent skips, the scopes test, the relay gate): the other ~425 stay in `design-smells.md` as a backlog.
