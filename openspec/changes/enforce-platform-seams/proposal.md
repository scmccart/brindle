# Proposal

## Why

`ports R4` says a platform concern has one seam and nothing outside it carries a `cfg` for it. The 2026-11-16 audit (`docs/architecture/audits/2026-11-16.md`, "Exceptions" and "Doc drift"; change 5 of `2026-11-16-remediation.md`) found four problems with it:
- **The exception is open-ended.** It names four files "among others", until `port-to-macos`. The m1 sweep found sites in 22 files, and nothing stops a new file from joining them.
- **It contradicts `testing`.** The "Platform pairs" idiom pairs `#[cfg(unix)]` and `#[cfg(windows)]` tests, and the test support code holds about 60 such pairs. R4 doesn't say whether that is allowed.
- **The pure crate depends on the platform.** `brindle-term-core` picks its OSC 7 path style with `cfg!(windows)` (`term-core/src/shell_integration.rs:110`; b31 #1 `ports R4`, b31 #2 `fcis R7`), so `Parser::new()` parses differently on Linux and Windows.
- **The relay's gate disagrees with itself.** It is `cfg(unix)` in `remote-agent/src/server/mod.rs` and `target_os = "linux"` in `remote-agent/src/main.rs`, which conflict on any Unix that isn't Linux. No doc names either one.

R4 has no check (audit, "Enforcement gaps": "do this once the exception is an explicit file list"). That makes it the rule most likely to be broken again, by `port-to-macos` and by the hotspots (`workspace.rs`, `main.rs`). The user answered D1 and D4 on 2026-11-16, so this change can now be written.

## What Changes

- **Rule (D1):** `ports R4` binds production code only: each crate's `src/` and its build script. Test code may pair platforms, as `testing`'s "Platform pairs" says: `#[cfg(test)]` items and the modules declared under them, `tests/**`, `remote-agent/tests/**`, the other crates' `tests/`, and examples. R4 also defines a platform `cfg`, and names the two forms that aren't platform differences:
  - a seam's module declaration (`#[cfg(windows)] mod win;`);
  - a `cfg_attr` that only sets lint levels (`allow(dead_code)` on a pure helper that only one platform's seam calls, as `fcis R7` asks).
- **Exception:** `docs/architecture/README.md`'s `ports R4` row becomes an explicit list of 17 production files. The list was re-derived from m1 and the current code (design §3), and `tests/architecture.rs` holds the same list. It still ends with `port-to-macos`.
- **Seams named in `ports-and-adapters.md`:**
  - the relay, which runs on Linux only;
  - the build scripts, as the home of the Windows resources;
  - the path style of a local program's directory reports, in `src/terminal/process.rs`.
- **Code, pure:** term-core's `Parser::new` takes a `PathStyle { Posix, Windows }`, and `Parser::posix()` and the `Default` impls that picked the style from the build platform are removed. The app chooses the style from the terminal's `Host`: `Host::Local` takes this machine's style, from the `process` seam, and a remote endpoint takes `Posix`. The server core's `Emulator` keeps `Posix`, as today.
- **Code, relay (D4):** `remote-agent/src/server/mod.rs` declares `relay` under `target_os = "linux"`, as `remote-agent/src/main.rs` already does. Every other platform, macOS included, refuses `relay`.
- **Check:** a new `tests/architecture.rs` test rejects any platform `cfg`, `cfg!` or `cfg_attr` in production code outside the seam set and the exception list. It skips test-only items, test-only modules and test targets. It also fails when:
  - a file on the exception list no longer has a platform `cfg`;
  - a listed path doesn't exist;
  - the list differs from the README's row.

  Unit tests show that its scanner finds planted sites.
- **Docs:** `ports-and-adapters.md` (R4, the seam table, Enforcement), `README.md` (the exception row and "Read by layer"), `testing-architecture.md` ("Platform pairs" and the layer table), `process-architecture.md` (the relay row), the Enforcement section of `functional-core-imperative-shell.md`, `CLAUDE.md`, and `.claude/rules/{app,remote-agent,tests}.md`. The docs change in the same commit as the check.

No user-visible behaviour changes. Every Linux and Windows tab parses directory reports as it does today. The relay's new gate differs from the old one only on non-Linux Unix, which no build targets yet.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. No requirement in `openspec/specs/` changes, so `.openspec.yaml` sets `skip_specs: true`:
- `shell-integration` "Working directory reports" holds as written: local tabs use the platform's path style, and remote tabs use POSIX;
- no spec names the relay's platforms.

## Impact

- **Code:**
  - `term-core/src/shell_integration.rs` (`PathStyle`, `Scanner::new`, `Parser::new`, `reported_dir`) and `term-core/src/emulator.rs`;
  - `src/remote/host.rs` (`Host::path_style`), `src/terminal/process.rs` (`LOCAL_PATH_STYLE`), and `src/terminal/mod.rs` (`parser_for` and `Terminal::message`);
  - the tests of `src/terminal/server.rs`;
  - `remote-agent/src/server/mod.rs`.
- **Check:** `tests/architecture.rs`.
- **Docs:** the six `docs/architecture/` files above, `CLAUDE.md` and three `.claude/rules/` files.
- **Unchanged:** wire messages, protocol versions, config keys, state files and dependencies.
- **Overlap with other changes:**
  - **`port-to-macos`:**
    - it ends the narrowed exception;
    - macOS refuses `relay` (D4);
    - the relay dispatch in `remote-agent/src/main.rs` is the one gate on the list that outlives the exception.

    The note is in this change's design (§8), and `port-to-macos`'s own files are not edited.
  - **`bound-and-fuzz-outside-input`:** its term-core fuzz tests build `Parser` and call `reported_dir`. Whichever change lands second uses `PathStyle`.
  - **`add-blocks`:** it reworks `src/terminal_view.rs`, whose one platform `cfg` (`parent_console_to`, line 180) is on the list. If that line moves, the file comes off the list and the README in the same commit, and the check says so.
  - **`add-pocket-client`:** a `not(target_os = "espidf")` gate on `remote-protocol`'s seam module declarations passes the check. A gate anywhere else needs a seam.
  - **Remediation siblings:**
    - `keep-session-file-on-serialize-error` adds a platform pair in `session.rs`'s tests, which D1 allows;
    - change 7 (`keep-pure-modules-pure`) and change 10 (`test-platform-logic-everywhere`) build on this check's scanner and seam list (both depend on change 5).
