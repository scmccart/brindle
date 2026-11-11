# Design

## Context

See proposal.md for why. What the removal has to cut through:

- **Control mode is a second tab model.** `TabContent::Tmux { session, view }` sits beside `TabContent::Zones`. `Workspace` branches on it for titles, focus, theme, status, bells, menus, restore, the session-ended and "shown by another Brindle" flows, and quitting (about 190 mentions in `workspace.rs`). `session.rs` has a parallel `TabState::Tmux` record and merge/dedupe rules. `picker.rs` has two question kinds that exist only for it. `actions.rs` has 23 `Tmux*` actions and the `TmuxWindow` key context, plus `Swallow` on ctrl-shift-d, which exists only so `TmuxDetach` has a twin outside tmux tabs.
- **`Terminal` carries tmux-shaped plumbing.** The `Remote` backend (bytes pushed in with `feed()`, input through a closure) serves tmux panes and message tabs; `reply()` drops generated replies for it because tmux answers queries itself; pane `default_colors`; `PaletteHandler`.
- **Config rejects unknown keys.** Every config struct uses `deny_unknown_fields`, so deleting `tmux`, `tmux_session`, `tmux_args` and `tmux_session_end` outright would turn every existing config into an error banner. `restore_session` already set the precedent for a removed setting: a `removed_*` field that is parsed, warned about and dropped (`config.rs`).
- **The default config wrote tmux profiles into users' files.** First run copies `default-config.toml`, so many configs have `[[profiles]] name = "tmux"` and `"tmux (classic)"`.
- **tmux leaks into tests and CI.** `tests/common/tmux.rs` (`TmuxServer`), `tests/e2e/tmux.rs`, `cases/tmux.rs` and tmux halves of about 25 other e2e cases, the perf `latency-tmux` and `throughput-tmux` cases, and a CI matrix that builds tmux 3.8 from source and optionally tests 3.6a.

## Goals / Non-Goals

**Goals:**
- Remove all tmux-aware code, config, specs, tests, CI and docs in one change, leaving no dead `Tmux` names.
- Keep every existing config and `session.toml` loadable, with the removed settings reported.
- Leave local tabs, zones and remote (non-tmux) terminals behaving exactly as before.

**Non-Goals:**
- No migration tool and no translation of `tmux = "plain"` into a command. The user chose "ignored with a warning".
- No replacement for multi-client attach, remote persistence or Claude Code's tmux teammate mode. The first two are `terminal-server-remote` territory.
- No change to OSC handling. OSC 7501, 133, 8, 52, focus reports and colour queries stay as terminal features; only the tmux-specific delivery paths go.

## Decisions

### 1. Removed config keys are tolerated, warned about and dropped

Add `removed_*` fields (as `removed_restore_session`) for top-level `tmux_session_end` and, on `Profile`, `tmux`, `tmux_session` and `tmux_args`, typed `Option<toml::Value>` so any value loads. `Config::parse` logs one `log::warn!` per top-level key and one per profile and key naming the profile. `normalized()` never reads them. A misspelt key still fails, because `deny_unknown_fields` stays.

- *Alternative: leave the fields in place, unused.* Keeps code that looks live and still shows tmux in the settings dialog. Rejected.
- *Alternative: error with a migration message.* Breaks every user's launch for a setting that can only be ignored. Rejected.

`config_edit` writes a profile with `set_or_remove`; when saving a profile it also removes the three legacy keys from that profile's table, so editing a profile in the dialog quietly cleans it. It never touches the top-level `tmux_session_end` (a write changes only what it concerns, see `configuration`). The leftover `tmux_session_end` line is harmless and only warns.

### 2. A legacy tmux profile runs the default shell, and nothing rewrites it

`tmux = "plain"` with no `command` runs the default shell, with the warnings above. We do not translate it into `command = "tmux"`: the new `profiles` spec says tmux is an ordinary command and the user's decision was to ignore the keys. The consequence is a profile still named `tmux` that opens a shell; the release notes and the warning text say what to set (`command = "tmux"`, `args = ["new-session", "-A", "-s", "main"]`). The default config no longer ships tmux profiles, so new installs don't see this.

### 3. `session.toml`: keep a skip variant for old tmux records

`TabState` is an internally tagged enum on `kind`. Remove `TabState::Tmux`, and add a unit variant marked `#[serde(other)]` that restore filters out and `snapshot` never writes. A state file with `kind = "tmux"` then loads and its other entries restore, instead of falling under "Bad state files are ignored" and losing everything. `SnapshotTab::Tmux`, the dedupe and ordering code (`merge_windows`' tmux session rule) and `drop_lost_zones`' tmux exceptions go.

- *Alternative: treat the whole file as bad.* Loses every user's tabs on upgrade for no reason. Rejected.

### 4. Collapse `TabContent` instead of leaving a one-variant enum

With `Tmux` gone, `TabContent::Zones(Entity<ZoneTreeView>)` is the only content. Replace the enum with the zone tree entity directly on `Tab` and delete the matches. This is the bulk of the `workspace.rs` edit and removes `is_tmux`, `shows_tmux`, `find_session`, `show_session`, `attach_tmux`, `on_tmux_event`, `remove_session_tabs`, `ask_session_ended`, `finish_replacing`, `forget_restore`, `orphaned_sessions`, `tab_menu_rows(is_tmux, …)`, `tab_shortcut(…, is_tmux, …)` and the tmux halves of `apply_config`, the tab menu, bells and status.

- *Alternative: keep `TabContent` for a future tab kind.* Speculative; adding a variant later is a mechanical change. Rejected.

### 5. Actions, keys and palette

Delete the 23 `Tmux*` actions, their `ACTIONS` rows, the `TmuxWindow` context and its bindings, and the `tmux: ` prefix stripping in `menu_label`. Delete the `Swallow` action and its ctrl-shift-d binding: it only existed so ctrl-shift-d did nothing outside tmux tabs, and the key now reaches a program like any other unbound chord. Bindings in user configs to a `tmux_*` or `swallow` action are already "unknown action, logged and ignored" (`configuration`'s keybinding requirement), so they need no handling. In `picker.rs`, delete the session-ended and open-elsewhere question kinds, `ignores_outside_press`' question rules, the prompt-in-place conversion (`open_prompt` turning a palette into a prompt), and the prompt failure path (`Prompt failures are shown`). `Rename Tab` stays the only prompt.

### 6. `Terminal` backends

`Terminal::Remote` stays for message tabs (no input, no queries). Delete what only tmux panes needed: `PaletteHandler` and the OSC 4 reply hook, `default_colors`, the `reply()` carve-out comments and the pane-specific scanner wiring in `feed()` that exists for tmux's raw `%output`. `Terminal::Agent` is unaffected. `fixed_size` views were tmux panes only; if nothing else sets it, remove the flag and the "no padding" branch with `tmux_view.rs`. Check `terminal_element.rs` and `cell_metrics()` callers first, since `terminal-rendering` still specifies grid geometry for zones.

### 7. Remote and WSL

Delete `remote::tmux_command`, `TmuxProcess`, `detach_command` and the dropped-client detach, plus their unit tests. `shlex` stays (used for profile arguments and remote commands). The `wsl` program detection is tmux-only; `terminal::cmdline::program_stem` stays since the Windows title rules use it. Windows needs no new behaviour: a profile with `command = "wsl"` already runs in a ConPTY.

### 8. Tests and CI

- Delete `tests/e2e/tmux.rs`, `cases/tmux.rs`, `tests/common/tmux.rs` and the e2e/perf cases that only exist for tmux (`latency-tmux`, `throughput-tmux`, the `remote-*` tmux cases, `win` WSL tmux, the `profile_menu`, `tab_menu`, `palette`, `restore`, `session_end`, `multi_window`, `dividers`, `geometry`, `styles`, `colors` and `cards` cases' tmux variants). Each remaining case that mentions tmux is audited: where tmux was only a convenient pane-source it is rewritten against a shell zone; where the case is about tmux behaviour it is deleted.
- A case that runs tmux *as a program inside an ordinary tab* (focus events, OSC 52 copy, box-drawing borders) is kept only if the audit finds one that tests Brindle behaviour with no better stand-in; those use the distro `tmux` package and nothing else.
- CI drops the tmux version matrix, the source build and cache, the `old-tmux` dispatch input and the `e2e-tmux-<v>` artifact names. Shards stay at 4 unless the case count drops enough to justify fewer (decide when the audit is done, and record it in the change's tasks).
- `tests/fixtures/remote/Dockerfile` stops installing tmux unless a retained remote case needs it.

### 9. Order of work, one PR

One PR, commits ordered so each builds and passes tests: (1) tolerate the legacy config and state keys (new code, still compiles with tmux present); (2) delete control mode and classic profile handling; (3) delete remote/WSL tmux and tmux-only terminal plumbing; (4) tests, perf and CI; (5) docs, default config, package descriptions; (6) in-flight change edits and the `test-claude-code-teammate-mode` deletion. Commit types: `refactor(tmux)!:`, `test(tmux):`, `ci:`, `docs:`; the PR title is `feat(tmux)!: drop tmux support`, so release-please marks a breaking change (a minor bump before 1.0).

## Risks / Trade-offs

- **[Upgrading users with tmux profiles get a plain shell]** → One warning per key in the log; release notes show the two-line fix; the profile editor no longer shows tmux fields, so there is nothing hidden to be confused by.
- **[A deleted e2e case was the only guard for a shared behaviour]** (for example divider or card geometry reused by zones) → The audit in task group 4 re-covers each with a zone-based case before deleting its tmux twin, and reads the spec's scenarios for the capability it touches.
- **[Hidden coupling in `Workspace`]** → `TabContent` collapse is a compile-driven change; the compiler finds every match. Run the full unit suite and the e2e suite (under `scripts/with-xvfb`) before pushing.
- **[Windows is not built on this machine]** → The Windows-only cfg code that mentions tmux (`win.rs`, `cases/win.rs`, `run-throwaway.ps1`) is edited here but compiled by the Windows CI job, or by the Windows machine through the push-and-ask routine in CLAUDE.md. Do not merge before `windows` CI is green.
- **[Specs: renamed requirements]** `openspec validate` refuses a MODIFIED requirement that drops a scenario, so twenty-one requirements are removed and re-added under new names. At archive, main specs end with the new names; nothing else cites the old ones (checked with `grep`, apart from the `session-restore` cross-reference, which is modified here).
- **[Claude Code's tmux teammate mode]** stops working in native panes. Inside a real tmux in an ordinary tab it works as it does in any terminal.

## Migration Plan

For users: upgrade; the first launch logs one warning per leftover key; saved control-mode tabs are skipped and the other tabs restore; recreate a tmux workflow with `command = "tmux"` on a profile. No rollback path is needed beyond installing the previous release, since the state file only loses its tmux entries.

For the repo: the PR lands as one squash commit titled `feat(tmux)!: drop tmux support`. Archive this change with the usual `docs(openspec): archive drop-tmux` commit, syncing the delta specs (see the follow-up tasks for the Purpose lines and the emptied `tmux-control-mode` spec).
