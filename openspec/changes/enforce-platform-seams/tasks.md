# Tasks

Each group writes its tests first and watches them fail (or fail to compile) before the code they check. Test names are sentences (`testing R15`). Nothing sleeps or asserts on elapsed time (`testing R7`, `R8`). Line numbers are as of `67053fb`.

## 1. `PathStyle` in term-core (shared contract, pure code)

- [ ] 1.1 In `term-core/src/shell_integration.rs`'s `mod tests`, change the directory-report tests to name a `PathStyle` (design §4), and verify that `cargo test -p brindle-term-core shell_integration::` fails to compile:
  - `osc7_paths_are_percent_decoded`, `osc7_drive_paths_on_windows` and `conemu_osc9_9` pass `PathStyle::Posix` or `PathStyle::Windows` where they pass `false` or `true`.
  - New test `each_path_style_ignores_the_other_styles_reports`:
    - under `Posix`, `9;9;C:\src` is `None` and `7;file://pc/C:/x` is `/C:/x`;
    - under `Windows`, `7;file://pc/home/user` is `None`.
  - `the_latest_report_is_kept` becomes `the_latest_report_is_kept_in_either_path_style`. It has no `cfg!`, and it runs the same feed through a `Harness` for each style, with fixed expected strings:
    - `Posix`: `/one`, then `/two`;
    - `Windows`: `C:\one`, then `C:\two`.

    It compares `reported_dir().and_then(Path::to_str)` as strings, never `Path`s on another platform's paths (`fcis R7`).
  - `remote_parser_uses_posix_paths_on_every_platform` becomes `a_parser_keeps_the_path_style_it_was_given`, for both styles. `posix_parser_keeps_remote_osc7_paths_on_any_host` builds `Parser::new(PathStyle::Posix)`.
  - `Harness::new` takes a `PathStyle`, and `scan_all` uses `Scanner::new(PathStyle::Posix)`.
- [ ] 1.2 Add `pub enum PathStyle { Posix, Windows }` (`Debug, Clone, Copy, PartialEq, Eq`, no `Default`) with a doc comment saying the caller knows the platform and this crate doesn't (design §4).
  - Replace `impl Default for Scanner` with `Scanner::new(style)`, and the `windows_paths: bool` field with `style: PathStyle`.
  - Replace `#[derive(Default)]`, `Parser::new()` and `Parser::posix()` on `Parser` with `Parser::new(style: PathStyle)`.
  - Change `reported_dir(body, windows: bool)` to `reported_dir(body, style: PathStyle)`, matching `style` exhaustively with no `_ =>` (`types R2`).

  Verify:
  - 1.1's tests pass;
  - `grep -n 'cfg!' term-core/src/shell_integration.rs` prints nothing;
  - `cargo test -p brindle --test architecture term_core_does_no_io_and_reads_no_clock` passes.
- [ ] 1.3 Make the server core's choice explicit, with no behaviour change:
  - `Emulator::new` builds `Parser::new(PathStyle::Posix)`, and its doc comment says why (today's `Parser::posix()`; design, Open Questions);
  - `term-core/src/replay/tests.rs` builds the same parser.

  Add `the_emulator_reads_directory_reports_as_posix` to `emulator.rs`'s tests. It feeds `ESC ] 7;file://h/home/u ESC \` and asserts that `render()` carries `7;file:///home/u`, which pins today's behaviour.

  Verify that `cargo test -p brindle-term-core` passes.

## 2. The app chooses the path style from the `Host` (pure method, seam constant, shell)

- [ ] 2.1 In `src/remote/host.rs`'s `mod tests`, add these tests, and verify they fail to compile (`cargo test -p brindle remote::host`):
  - `a_local_host_takes_this_machines_path_style`: `Host::Local.path_style(s) == s` for both styles;
  - `a_remote_host_reports_posix_paths`: an SSH and a container `Host::Remote` give `PathStyle::Posix` for both local styles.
- [ ] 2.2 Add `Host::path_style(&self, local: PathStyle) -> PathStyle`, matching `Local` and `Remote(_)` with no `_ =>`. Its doc comment says a remote endpoint is Linux (`types R8`, design §4). Verify that 2.1 passes on Linux.
- [ ] 2.3 In `src/terminal/process.rs` (the seam for a local program's name and directory), add `pub const LOCAL_PATH_STYLE: PathStyle`: `Windows` under `#[cfg(windows)]`, `Posix` under `#[cfg(not(windows))]`. Extend the module doc's first line to name it. Verify that `cargo build --locked` passes on Linux; Windows is covered in 5.2.
- [ ] 2.4 Switch the callers:
  - `parser_for` (`src/terminal/mod.rs:1207`) builds `Parser::new(host.path_style(process::LOCAL_PATH_STYLE))`;
  - `Terminal::message` (`:485`) builds `Parser::new(process::LOCAL_PATH_STYLE)`;
  - the test sink in `src/terminal/server.rs:163` builds `Parser::new(PathStyle::Posix)`.

  Verify:
  - `grep -rn 'Parser::posix\|Parser::new()' src remote-agent/src term-core/src` prints nothing;
  - `cargo test -p brindle terminal::` passes.
- [ ] 2.5 In `CLAUDE.md`'s `Server` backend bullet, replace "a remote session's with `Parser::posix()`, since its OSC 7 paths are Linux ones" with the path style from `Host::path_style`: `PathStyle::Posix` for a remote endpoint, and `process::LOCAL_PATH_STYLE` here. Verify that `grep -n 'Parser::posix' CLAUDE.md` prints nothing.

## 3. The relay's gate is `target_os = "linux"` (D4)

- [ ] 3.1 In `remote-agent/src/server/mod.rs`, declare `relay` under `#[cfg(target_os = "linux")]` instead of `#[cfg(unix)]` (design §5). `remote-agent/src/main.rs`'s `liveness` declaration and `relay` dispatch already use it.

  Verify:
  - `grep -rn 'cfg(unix)' remote-agent/src/server/mod.rs` prints nothing;
  - on Linux, `cargo build -p brindle-remote-agent --locked` and `cargo test -p brindle-remote-agent --test relay` pass;
  - Windows is covered in 5.2.
- [ ] 3.2 In `CLAUDE.md`'s "Relay" bullet, "Linux only, Windows refuses it" becomes "Linux only (`target_os = "linux"`); other platforms, macOS included, refuse it". Verify by reading the bullet.

## 4. The `ports R4` rule and its check (one commit: `docs/architecture/` and `tests/architecture.rs` together)

- [ ] 4.1 In `tests/architecture.rs`, add the scanner's own tests (design §6), and verify they fail to compile (`cargo test -p brindle --test architecture`):
  - **`a_platform_cfg_outside_a_seam_is_found`.** These planted snippets are each reported with their line:
    - `#[cfg(windows)] fn f() {}`;
    - `if cfg!(target_os = "linux") {}`;
    - `#[cfg(any(target_os = "linux", windows))]`;
    - `#[cfg(not(unix))]`;
    - `#[cfg(target_family = "unix")]`;
    - a `#[cfg_attr(` / `windows,` / `path = "x.rs"` / `)]` attribute over four lines;
    - `#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]`;
    - `#[cfg(windows)] mod helpers;`, where `helpers.rs` is not a seam.
  - **`what_is_not_a_platform_cfg_is_not_found`.** None of these is reported:
    - `#[cfg(test)] mod tests { #[cfg(unix)] fn g() {} }`, and a `#[cfg(test)] fn` holding a `cfg!(windows)`;
    - `#[cfg(all(test, unix))] mod helpers;`, whose file is returned as test-only;
    - `#[cfg_attr(not(windows), allow(dead_code))]` and `#[cfg_attr(windows, allow(dead_code, reason = "…"))]`;
    - `use windows::Win32;` and `// #[cfg(windows)]`;
    - `#[cfg(debug_assertions)]` and `#[cfg(feature = "x")]`;
    - `#[cfg(windows)] mod win;` and `#[cfg(unix)] #[path = "local/unix.rs"] mod sys;`, whose files are seams.
  - **`a_test_only_predicate_needs_test_to_hold`:** `test` and `all(windows, test)` are test-only; `not(test)`, `any(test, unix)` and `unix` aren't.
  - **`a_declared_module_resolves_as_rustc_resolves_it`:**
    - `mod x;` in `a/lib.rs` → `a/x.rs`;
    - in `a/b.rs` → `a/b/x.rs`;
    - `#[path = "b/y.rs"]` in `a/c.rs` → `a/b/y.rs`;
    - `NAME/mod.rs` when `NAME.rs` doesn't exist.
- [ ] 4.2 Implement the scanner as pure functions over text (design §1, §6):
  - `enum Predicate { TestOnly, Platform, Neither }` and `predicate(text) -> Predicate`, matched with no `_ =>`;
  - `platform_cfgs(path, text, seams) -> Scan`, giving the sites as `path:line: text` plus the files of test-only modules. It reads attributes until their brackets balance, skips `//` lines, skips a test-only item to its `;` or to the brace that closes its first `{`, allows seam module declarations and lint-only `cfg_attr`, and treats only `cfg_attr`'s first argument as its predicate;
  - module resolution as in 4.1;
  - paths compared with `/` separators on every platform.

  Verify that 4.1's tests pass.
- [ ] 4.3 Add the tree check:
  - **`SEAMS`:** the paths of design §2, with a comment saying it mirrors `ports-and-adapters.md`'s "How it applies here" table and changes with it.
  - **`R4_EXCEPTION`:** exactly design §3's 17 files:
    - `src/instance.rs`, `src/main.rs`, `src/notify/mod.rs`, `src/remote/local.rs`, `src/remote/local/relay.rs`, `src/remote/mod.rs`;
    - `src/session.rs`, `src/settings.rs`, `src/terminal/mod.rs`, `src/terminal_view.rs`, `src/workspace.rs`, `src/zone_view.rs`;
    - `remote-agent/src/main.rs`, `remote-agent/src/server/cli.rs`, `remote-agent/src/server/core.rs`, `remote-agent/src/server/core/sessions.rs`, `remote-agent/src/server/daemon.rs`.
  - **The production walk:** the `src/` trees of the five crates, plus `build.rs` and `remote-agent/build.rs`, without the files the scan returned as test-only.
  - **Tests:**
    - `platform_cfgs_stay_in_their_seams`: its failure lists `path:line: text` and says to move the difference into the concern's seam, or to change README's `ports R4` row and `R4_EXCEPTION` in the same commit;
    - `every_excepted_file_still_has_a_platform_cfg`;
    - `every_seam_and_excepted_path_exists`;
    - `the_ports_r4_exception_matches_the_readme`, where the backticked `.rs` paths on README's `ports R4` row equal `R4_EXCEPTION`.
  - The module doc says these hold `ports R4`.

  Verify:
  - the four tests pass once groups 1–3 and 4.4 are in;
  - planting `#[cfg(windows)] const _: () = ();` in `src/theme.rs` makes `platform_cfgs_stay_in_their_seams` fail and name `src/theme.rs` with its line, and reverting it restores the pass. The plant is not committed; the PR body records it.
- [ ] 4.4 In the same commit as 4.1–4.3, edit the rule docs (design §7):
  - **`docs/architecture/ports-and-adapters.md`:**
    - R4 defines production code, a platform `cfg`, a seam module declaration and a lint-only `cfg_attr`, and says test code may pair platforms (D1, design §1);
    - R4's exception bullet points at README's row;
    - the "How it applies here" table gets the relay row, the build-script row and the `LOCAL_PATH_STYLE` extension of `src/terminal/process.rs` (design §2);
    - Enforcement names the four tree tests and the scanner tests, and "R4, R6–R11 and R13 rely on review" becomes "R6–R11 and R13".
  - **`docs/architecture/README.md`:**
    - the `ports R4` exceptions row is the exact row in design §3;
    - the "Read by layer" row reads "a `cfg(unix)`, `cfg(windows)` or `cfg(target_os)` in production code".
  - **`docs/architecture/testing-architecture.md`:**
    - "Platform pairs" says test code is outside `ports R4` (D1): `#[cfg(test)]` items, `tests/**`, `remote-agent/tests/**`;
    - the architecture-tests row of the layer table adds "do platform `cfg`s stay in their seams?";
    - Enforcement names `platform_cfgs_stay_in_their_seams`.
  - **`docs/architecture/process-architecture.md`:** the relay row of "How it applies here" says it runs on Linux only, built under `target_os = "linux"` and refused on every other platform (D4).
  - **`docs/architecture/functional-core-imperative-shell.md`:** Enforcement says `platform_cfgs_stay_in_their_seams` holds R7's `cfg` half in production code. This describes the check and changes no rule.

  Verify:
  - `cargo test -p brindle --test architecture` passes, `the_ports_r4_exception_matches_the_readme` included;
  - every path named in the seam table exists.
- [ ] 4.5 Update the agent-facing summaries:
  - **`.claude/rules/app.md`:** the R4 bullet says production code, and names `notify/{dbus,toast,activation}.rs` instead of `notify/`. "Enforced by" names `platform_cfgs_stay_in_their_seams`.
  - **`.claude/rules/remote-agent.md`:** a bullet says the relay is Linux only under `target_os = "linux"` and that platform `cfg`s stay in `server/platform/`, `pty`, `conpty` (`ports R4`). "Enforced by" names the check.
  - **`.claude/rules/tests.md`:** test code may pair platforms with `cfg(unix)`/`cfg(windows)` (D1).
  - **`CLAUDE.md`:** in "The rules broken most often", the `cfg` bullet says `tests/architecture.rs` now rejects it in production code outside the seams and the listed files.

  Verify by reading each edited line against design §7.

## 5. Integration

- [ ] 5.1 On Linux, verify that these pass with no new warnings:
  - `cargo test --workspace --locked`, run in parallel with the default `--test-threads`;
  - `cargo build --release --locked`.
- [ ] 5.2 On Windows, push the branch and read `build-windows.yml`'s `cargo test` step. Verify:
  - the architecture tests pass there, which checks the `/` path normalization;
  - the term-core and `remote::host` tests pass;
  - `src/terminal/process.rs`'s `cfg(windows)` constant builds.

  Report it as CI's result, and as Windows' first run of these tests (`testing R11`, `R17`).
- [ ] 5.3 Read CI's advisory e2e shards and `windows-e2e`. Every local tab runs through `parser_for`, so verify they are no worse than on `main`. Label the result as CI's; anything that couldn't run stays open in the PR body.
- [ ] 5.4 Verify that `openspec validate enforce-platform-seams --strict` passes.

## Workflow follow-up

- The archive commit (`docs(openspec): …`) goes on the implementation PR. `skip_specs: true`, so archiving syncs no spec.
- File the design's open question as a backlog item: a Windows core's replays drop `9;9` directories, because `Emulator` parses POSIX.
- `port-to-macos` takes design §8's note when its design is next revised.
