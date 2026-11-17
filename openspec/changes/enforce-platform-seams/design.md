# Design

## Context

See proposal.md (Why). The change closes these entries of the audit (`docs/architecture/audits/2026-11-16.md`), as change 5 of `2026-11-16-remediation.md`:
- **"Exceptions":** the `ports R4` exception is stale.
- **"Enforcement gaps":** `ports R4` has 42 entries plus the sweep, and no check.
- **b31 #1** (`ports R4`) and **b31 #2** (`fcis R7`): `term-core/src/shell_integration.rs:110`, `windows_paths: cfg!(windows)`.
- **"Doc drift":** the relay's platform gate, and `ports R4` against `testing`'s "Platform pairs".
- **Decisions D1 and D4** (answered 2026-11-16):
  - D1: R4 binds production code, and test code is exempt;
  - D4: the relay's gate is `target_os = "linux"` in both `remote-agent/src/server/mod.rs` and `remote-agent/src/main.rs`.

Six facts from the code, re-read on this branch (`67053fb`, whose product code is `ab6462b`'s), shape the approach.

**m1's sweep missed compound predicates.** Its regex was `cfg\((not\()?(unix|windows|target_os)`, so it never matched `cfg(any(target_os = "linux", windows))` or `cfg(all(…))`. These are common in `src/terminal/mod.rs`, `src/workspace.rs`, `src/main.rs`, `src/notify/mod.rs`, `src/remote/mod.rs` and `remote-agent/src/main.rs`, which gate the server for a future macOS build. They are the only site in `src/zone_view.rs` (`:594`, `:600`). m1 also counted sites inside `#[cfg(test)]` modules as production code.

**Some listed sites are test code or seams.**
- `src/config_edit.rs:792` is in that file's tests.
- `remote-agent/src/server/conn.rs:315` and `:349` are in its tests.
- `src/remote/client.rs`'s sites (`:858`–`:948`, `:1216`) are all in its tests, and `ports`' seam table already names the file.
- `src/desktop.rs` is a seam in that table ("launcher registration").
- `src/test_support.rs` is a module declared `#[cfg(all(test, unix))]` in `src/main.rs:24`.

**Seam declarations.** These module declarations carry the cfg of the seam they declare:
- `remote-protocol/src/lib.rs` (`scope`, `win`);
- `src/main.rs:27` (`win`);
- `src/notify/mod.rs:10`–`:16` (`activation`, `dbus`, `toast`);
- `src/remote/local.rs:34`–`:43` (`pipe`, and `sys` through `#[path = "local/{unix,windows}.rs"]`);
- `remote-agent/src/main.rs:2`–`:4` (`liveness`, `pty`);
- `remote-agent/src/server/platform/mod.rs`.

**Lint-only attributes.** `cfg_attr(not(windows), allow(dead_code))` marks pure helpers that are compiled and tested everywhere but called only from one platform's seam. Examples are `src/terminal/cmdline.rs:12`, `:41` and `src/remote/local/session.rs:175`, and more sit in seams and in files already listed. This is the result of following `fcis R7`, not a way around it.

**The term-core site.** Three types are involved:
- `Scanner` (private) takes `windows_paths: cfg!(windows)` in its `Default` impl.
- `Parser` derives `Default`, and `Parser::posix()` overrides the flag.
- `reported_dir(body, windows: bool)` reads OSC 7 and 9;9 bodies.

Today's callers:
- the app's `parser_for(host)` (`src/terminal/mod.rs:1207`): `Parser::new()` for `Host::Local`, `Parser::posix()` otherwise;
- `Terminal::message` (`:485`): `Parser::new()`;
- a test sink in `src/terminal/server.rs:163`;
- the server core's `Emulator::new` (`term-core/src/emulator.rs:80`): always `Parser::posix()`;
- term-core's tests, where `the_latest_report_is_kept` branches on `cfg!(windows)` (`:991`, `:994`).

**The relay's gate.**
- `remote-agent/src/server/mod.rs:17` declares `relay` under `cfg(unix)`.
- `remote-agent/src/main.rs` already declares `liveness` and dispatches `relay` under `target_os = "linux"`, with a `not(target_os = "linux")` refusal (`:62`–`:71`).
- `relay.rs` uses `crate::liveness`. Today the whole `server` module is gated `any(target_os = "linux", windows)` (`main.rs:6`), so no build sees the mismatch. Once `port-to-macos` opens `server` to macOS, a `cfg(unix)` relay would compile there without `liveness`, and the build would fail.
- `remote-agent/tests/relay.rs` is already `#![cfg(target_os = "linux")]`.

## Goals / Non-Goals

**Goals:**
- A platform `cfg` added to production code outside a seam and outside the listed files fails plain `cargo test`, on every platform CI builds.
- The exception is a closed list that can only shrink. The README row and the test's list are checked against each other.
- `brindle-term-core` has no platform `cfg` in production code, and parses the same on every platform for a given `PathStyle`.
- The relay's gate is one predicate, named in the docs.

**Non-Goals:**
- Moving any listed file's sites behind a seam. That is `port-to-macos`'s job, and change 10 (`test-platform-logic-everywhere`) moves pure Windows logic out of seams.
- Changing which path style the server core's `Emulator` uses (see Open Questions).
- `ports R5` and its existing check (`the_server_core_and_holder_name_no_platform_type`), which this change leaves alone.
- Parsing Rust properly. The check is a text scanner, like the rest of `tests/architecture.rs`, and it adds no dependency.
- `testing R3`'s `--self-test` and `--shard` gaps, the needles' grouped imports, and `[workspace.dependencies]` (change 7).

## Layers

| Part | Layer | Crate |
| --- | --- | --- |
| `PathStyle`, `Scanner::new`, `Parser::new(style)`, `reported_dir(body, style)` | pure code (`fcis R4`, `R7`) | `brindle-term-core` |
| `Emulator::new` choosing `PathStyle::Posix` | pure code, unchanged behaviour | `brindle-term-core` |
| `Host::path_style(local)` | pure code on a GPUI-free type (`fcis R1`) | `brindle` (`src/remote/host.rs`) |
| `LOCAL_PATH_STYLE` | platform seam: a local program's directory (`ports R4`) | `brindle` (`src/terminal/process.rs`) |
| `parser_for`, `Terminal::message` | shell: gathers the host and the seam's constant, then calls | `brindle` (`src/terminal/mod.rs`) |
| the relay's module declaration | platform seam declaration | `brindle-remote-agent` (`server/mod.rs`) |
| the scanner (`platform_cfgs`, `predicate`, `test_modules`) and the tree check | architecture tests (`testing`'s layer table); pure functions over text, tested on planted snippets | `tests/architecture.rs` |

No decision moves into a view, a thread or a seam. The only new seam item is a constant. No crate reference changes (`ports R1`, `R3`). `PathStyle` lives in term-core, which both sides use; the app gets it through the existing `terminal::shell_integration` re-export.

## Decisions

### 1. What R4 covers and what counts as a platform `cfg`

The rule text in `ports-and-adapters.md` R4 gains three definitions, and the check implements exactly these.

- **Production code** is each workspace crate's `src/` tree, plus `build.rs` and `remote-agent/build.rs`.
- **Test code is out of scope (D1):**
  - `#[cfg(test)]` items and the files of modules declared under one;
  - `tests/**`, `remote-agent/tests/**` and other crates' `tests/`;
  - examples (`ui/examples/gallery.rs`, whose `--check` drives scenarios on Windows).

  Examples aren't named in D1. They are ship-nothing developer tools, which is D1's reasoning, and the design names them so the scope is closed.
- **A platform `cfg`** is a `cfg(…)` or `cfg_attr(…)` attribute, outer or inner, or a `cfg!(…)` macro, whose predicate names `unix`, `windows`, `target_os` or `target_family`. For `cfg_attr`, only the first argument counts as the predicate.
- **Not a platform difference:**
  - **A seam's module declaration.** This is a platform `cfg` attribute on `mod NAME;`, after any other attributes, `#[path]` included, whose resolved file is in the seam set. It matches R4's own example (`#[cfg(unix)] mod unix;`).
  - **A lint-only `cfg_attr`.** Every attribute it sets is `allow`, `expect`, `warn` or `deny`. It changes no code, and it is how a helper `fcis R7` keeps ungated avoids a dead-code warning on the platforms whose seam doesn't call it.
- **A test-only predicate** is `test`, or `all(…)` with a test-only argument. `not(test)` and `any(test, …)` are not test-only. So `src/main.rs:3`'s `#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]` counts, and `#[cfg(all(test, unix))] mod test_support;` doesn't.

**Alternatives:**
- **Naming the test support files as seams.** This was D1's other option, and it was rejected by the user.
- **Counting lint-only `cfg_attr` as violations.** That would put `src/terminal/cmdline.rs` and `src/remote/local/session.rs` on an exception that `port-to-macos` can't end. Those files are already correct by `fcis R7`, so nothing in them could be moved.
- **Allowing any `cfg` on any `mod` declaration.** Then a second, parallel seam could start with one line. Requiring the declared file to be a named seam keeps R4's "never start a second seam".

### 2. The seam set

The check's `SEAMS` mirrors `ports-and-adapters.md`'s "How it applies here" table. Three rows are added and one is extended, in the same commit:

| Concern | Its one home | Change |
| --- | --- | --- |
| the terminal server's OS layer | `remote-agent/src/server/platform/` | existing |
| PTYs and pseudoconsoles | `remote-agent/src/pty.rs`, `remote-agent/src/pty/`, `remote-agent/src/conpty/` | existing |
| server directory, endpoint names, access checks, scopes | `remote-protocol/src/{server_dir,endpoint,access,scope,win}.rs` | existing |
| the app's connection to a server | `src/remote/local/{unix,windows,pipe}.rs` | existing |
| a remote endpoint's transport | `src/remote/client.rs` | existing |
| a local program's name and directory, **and the path style its directory reports use** | `src/terminal/process.rs` | extended (`LOCAL_PATH_STYLE`, §4) |
| the primary selection | `src/primary.rs` | existing |
| notifications and activation | `src/notify/{dbus,toast,activation}.rs` | existing |
| launcher registration | `src/desktop.rs` | existing |
| opening links | `src/terminal/links.rs` | existing |
| small Windows helpers | `src/win.rs` | existing |
| **the relay, which runs on Linux only** | `remote-agent/src/server/relay.rs`, `remote-agent/src/liveness.rs`, declared under `target_os = "linux"` | **new** (D4, §5) |
| **resources compiled into the Windows executables** (icon, version, manifest; `embed-resource` exists only on a Windows build host) | `build.rs`, `remote-agent/build.rs` | **new** |

- **The build scripts:** their `#[cfg(windows)]` is about the build host, not the target. It guards a build dependency that exists only on Windows hosts, behind a `CARGO_CFG_TARGET_OS` check. Each script is the one home of that concern, and `port-to-macos` wouldn't move them anywhere. So they are seams, not exceptions. The plan had listed them as exceptions.
- **The relay:** naming it as a seam makes `server/mod.rs`'s declaration a seam module declaration (§1). So that file needs no listing, although the plan listed it.

### 3. The exception: 17 production files

These are re-derived from m1 and from the current code. The grep was `grep -rn 'cfg(\(unix\|windows\|target_os\)\|cfg!(' src remote-agent/src remote-protocol/src term-core/src ui/src build.rs remote-agent/build.rs`, widened to `any(`/`all(`/`cfg_attr(`. Test code, seams, seam declarations and lint-only attributes were left out by §1 and §2. A read-only spike of §6's scanner gave the same 17 files and 124 sites, once term-core's site is gone.

| File | Platform `cfg` sites outside a seam (lines as of `67053fb`) |
| --- | --- |
| `src/instance.rs` | 4 (`:116`, `:185`, `:204`, `:241`) |
| `src/main.rs` | 13 (`:3`, `:309`, `:371`, `:443`, `:479`, `:583`, `:591`, `:602`, `:634`, `:786`, `:848`, `:856`, `:983`) |
| `src/notify/mod.rs` | 16 (the inline Windows and fallback `activation` modules at `:23`, `:53`; imports `:67`–`:73`; `:130`–`:136`, `:164`–`:180`, `:242`–`:258`, `:348`) |
| `src/remote/local.rs` | 8 (`:92`, `:95`, `:411`, `:416`, `:425`, `:427`, `:445`, `:447`) |
| `src/remote/local/relay.rs` | 2 (`failed_status`, `:282`, `:286`) |
| `src/remote/mod.rs` | 3 (`:15` the `local` module gate, `:35`, `:98`) |
| `src/session.rs` | 3 (`:281`, `:362`, `:372`) |
| `src/settings.rs` | 2 (`platform_has_family`, `:239`, `:248`) |
| `src/terminal/mod.rs` | 25 (the server gates from `:21` to `:1207`, `:890`–`:909`, `:1214`, `:1223`, `:1260`) |
| `src/terminal_view.rs` | 1 (`parent_console_to`, `:180`) |
| `src/workspace.rs` | 25 (server gates `:209`–`:1618`, window chrome `:1985`–`:2199`, `:2258`, `:2491`–`:2505`) |
| `src/zone_view.rs` | 2 (`zone_server_session`, `:594`, `:600`) |
| `remote-agent/src/main.rs` | 12 (the `server` module gate `:6`, `USAGE`, the `launch` arm, `serve`/`hold`/`manage` and their refusals, and the relay dispatch `:62`, `:68`) |
| `remote-agent/src/server/cli.rs` | 2 (`USAGE`, `:28`, `:43`) |
| `remote-agent/src/server/core.rs` | 2 (`prune_copies`, `:185`, `:200`) |
| `remote-agent/src/server/core/sessions.rs` | 2 (`client_reparent`, `:856`, `:872`) |
| `remote-agent/src/server/daemon.rs` | 2 (the `launch` and `start_role_as` re-exports, `:7`, `:10`) |

**Against the plan's list:**
- **Dropped:**
  - `src/config_edit.rs` and `remote-agent/src/server/conn.rs`: test code only;
  - `src/remote/client.rs` and `src/desktop.rs`: seams, and the first has only test sites;
  - `remote-agent/src/server/mod.rs`: a seam declaration once the relay is a seam;
  - `build.rs` and `remote-agent/build.rs`: seams (§2).
- **Added:** `src/zone_view.rs`, which m1's regex missed.
- **Not listed, by §1:**
  - `src/terminal/cmdline.rs` and `src/remote/local/session.rs`: lint-only;
  - `src/test_support.rs`: a test module.
- **Not listed because this change fixes it:** `term-core/src/shell_integration.rs` (§4).

The README row reads:

| Doc rule | Exception | Approved in | Ends with |
| --- | --- | --- | --- |
| `ports R4` | Platform `cfg`s outside a seam in these production files, and no others: `src/instance.rs`, `src/main.rs`, `src/notify/mod.rs`, `src/remote/local.rs`, `src/remote/local/relay.rs`, `src/remote/mod.rs`, `src/session.rs`, `src/settings.rs`, `src/terminal/mod.rs`, `src/terminal_view.rs`, `src/workspace.rs`, `src/zone_view.rs`, `remote-agent/src/main.rs`, `remote-agent/src/server/cli.rs`, `remote-agent/src/server/core.rs`, `remote-agent/src/server/core/sessions.rs`, `remote-agent/src/server/daemon.rs`. The architecture test `platform_cfgs_stay_in_their_seams` lists the same files and fails on any other (backlog: audit 2026-11-16, "Exceptions"). | adoption, 2026-11-16; narrowed by `enforce-platform-seams` | the `port-to-macos` change, which moves each behind its seam |

- R4's own exception bullet points at this row instead of naming files.
- The row's only backticked `.rs` paths are the listed files, so §6's README check can read them without special cases.

**Alternative:** listing sites (`file:line`) rather than files. That would be exact, but it breaks on every unrelated edit above a site. A file list that can only shrink is enough to stop new breaches, and that is the goal.

### 4. `PathStyle` replaces term-core's `cfg!(windows)`

In `term-core/src/shell_integration.rs`:

```rust
/// How a terminal's directory reports (OSC 7, 9;9) spell paths: the
/// platform its program runs on, which the caller knows and this crate doesn't.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathStyle { Posix, Windows }
```

- `Scanner::new(style)` replaces `impl Default for Scanner`, and `Parser::new(style)` replaces `Parser::new()`, `Parser::posix()` and `#[derive(Default)]`.
- `reported_dir(body, style)` matches `style` exhaustively, with no `_ =>` (`types R2`).

**What the types rule out (`types R1`):** a parser whose path style nobody chose. Today `Default` silently takes the build platform's style. With no `Default`, every constructor names its style, and a bool at a call site (`Parser::new(false)`) can't be misread.

**Who chooses:**
- **The app.** `parser_for(host)` becomes `Parser::new(host.path_style(LOCAL_PATH_STYLE))`. `Host::path_style(&self, local: PathStyle) -> PathStyle` is a pure method in `src/remote/host.rs`:
  - `Host::Local => local`;
  - `Host::Remote(_) => PathStyle::Posix`, because a remote endpoint is Linux.

  Assuming the program runs here sits in the `Host::Local` arm (`types R8`). The method takes the local style as an argument, so its tests cover both values on every platform (`fcis R7`).
- **The seam.** `src/terminal/process.rs` defines `pub const LOCAL_PATH_STYLE: PathStyle`: `Windows` under `cfg(windows)`, `Posix` under `cfg(not(windows))`. The directory a local program reports is the concern that file already owns ("a local program's name and directory").
- **`Terminal::message`.** It takes `LOCAL_PATH_STYLE`, as `Parser::new()` gave it before. It parses Brindle's own text, so the style never matters.
- **The server core's `Emulator::new`.** It takes `PathStyle::Posix` explicitly, which is today's `Parser::posix()`. No behaviour changes.

**Alternatives:**
- **A `windows_paths: bool` argument.** It is the smallest change, but it reads badly at call sites, and b31 #1 proposed the enum.
- **Leaving `Default` for tests.** Tests then never say which style they test, which is the bug class this change removes.
- **Passing the style into `Emulator::new` from a `server::platform` constant.** That would change a Windows core's replays (see Open Questions), which is a behaviour change outside this plan.

### 5. The relay's gate is `target_os = "linux"` (D4)

`remote-agent/src/server/mod.rs:17` becomes `#[cfg(target_os = "linux")] pub mod relay;`. `remote-agent/src/main.rs` keeps its `target_os = "linux"` `liveness` declaration and its `relay` dispatch with the `not(target_os = "linux")` refusal.

**What the docs say:**
- `ports-and-adapters.md` gets the seam row (§2).
- `process-architecture.md`'s table row for the relay says it runs on Linux only: built under `target_os = "linux"`, and refused with "relay runs only on Linux" on every other platform, macOS included.
- `CLAUDE.md`'s "Relay … Linux only, Windows refuses it" becomes "other platforms refuse it".

**What this changes, and doesn't:**
- On Linux and Windows nothing changes: both gates agree there.
- On a Unix other than Linux, nothing changes today either, because `server` isn't compiled there. Once `port-to-macos` compiles `server` on macOS, the helper builds and refuses `relay`, instead of failing to compile on `crate::liveness`.
- No protocol, process role or relay behaviour changes, and a core restart, a Brindle crash or an upgrade sees the same relay as before (`process R1`–`R3`, `R12`).

`remote-agent/src/server/daemon.rs:10` re-exports `start_role_as` under `cfg(unix)`, and only the relay uses it. It stays as it is, because it is listed and D4 names two places. §8 tells `port-to-macos` about it.

### 6. The check in `tests/architecture.rs`

These pure functions over text have their own tests (`testing R1`):
- `predicate(text) -> Predicate`, where `enum Predicate { TestOnly, Platform, Neither }` is matched without `_ =>`;
- `platform_cfgs(path, text, seams) -> Scan`, which returns the sites outside the allowances in §1 as `path:line: text`, plus the module files the text declares as test-only.

The tree check walks the production code, skips test-only files, and asserts as follows:
- **`platform_cfgs_stay_in_their_seams`:** every site is in `SEAMS` or `R4_EXCEPTION`. The failure lists each `path:line` and says to move the difference into the concern's seam, or to change README's row and `R4_EXCEPTION` together in the same commit.
- **`every_excepted_file_still_has_a_platform_cfg`:** a cleaned file has to leave the list, so the list only shrinks.
- **`every_seam_and_excepted_path_exists`.**
- **`the_ports_r4_exception_matches_the_readme`:** the `.rs` paths in backticks on README's `ports R4` row equal `R4_EXCEPTION`. The doc and the test can't drift.
- **The positive direction:** `a_platform_cfg_outside_a_seam_is_found` feeds planted snippets that must be found, and `what_is_not_a_platform_cfg_is_not_found` feeds snippets that must not be.
  - Planted sites: `#[cfg(windows)]`, `cfg!(target_os = "linux")`, `cfg(any(target_os = "linux", windows))`, `cfg(not(unix))`, `target_family`, a multi-line `cfg_attr(windows, path = …)`, and an inner `#![cfg_attr(all(windows, not(test)), …)]`.
  - Non-sites: `#[cfg(test)] mod tests { #[cfg(unix)] … }`, `#[cfg(all(test, unix))] mod helpers;`, a lint-only `cfg_attr`, `use windows::Win32;`, a commented-out `cfg`, `cfg(debug_assertions)`, `cfg(feature = "x")`, and a seam declaration with and without `#[path]`.

**How the scanner reads text:**
- An attribute is read until its brackets balance, so multi-line attributes count.
- `//` comment lines are skipped, as `offenders` already does.
- A test-only attribute skips the following item: to its `;`, or to the brace that closes its first `{`.
- A test-only `mod NAME;` adds NAME's file, resolved as rustc does (`#[path]` relative to the declaring file's directory, else `NAME.rs` or `NAME/mod.rs` beside a `mod.rs`/`lib.rs`/`main.rs`, or under the declaring file's own directory otherwise), to the files to skip.
- Paths are compared with `/` separators, so the check gives the same answer on Windows, where `strip_prefix` yields `\`.

**Alternatives:**
- **`syn`:** exact, but a new dev-dependency for the app, and against `CLAUDE.md`'s `Cargo.lock` rule. The text rules above cover every form in the tree.
- **A clippy lint (`disallowed_macros` or a custom lint):** it can't express "outside these files", and clippy isn't in CI yet (change 6).
- **Checking `SEAMS` against the ports table's text:** the table uses brace forms (`{unix,windows}`). It is reviewed together with `SEAMS` instead.

### 7. Docs change with the check, in one commit

The rule docs (`ports R4` and its exception, `testing`'s platform pairs, the README row, the relay gate in `ports` and `process`) change in the same commit as `tests/architecture.rs`, never the test alone.

| Doc | Edit |
| --- | --- |
| `ports-and-adapters.md` | R4 gains §1's definitions and points its exception at the README. The seam table gets §2's rows. Enforcement names the new tests and drops R4 from "rely on review". The anti-pattern list is unchanged. |
| `README.md` | The exception row from §3. The "Read by layer" row becomes "a `cfg(unix)`, `cfg(windows)` or `cfg(target_os)` in production code → ports". |
| `testing-architecture.md` | "Platform pairs" says test code is outside `ports R4` (D1). The architecture-tests row of the layer table also answers "do platform `cfg`s stay in their seams?". Enforcement names the check. |
| `process-architecture.md` | The relay row of "How it applies here" says Linux only (§5). |
| `functional-core-imperative-shell.md` | Enforcement: `platform_cfgs_stay_in_their_seams` holds R7's `cfg` half in production code (term-core has none). This describes the check and changes no rule. |
| `CLAUDE.md` | `Parser::posix()` → `PathStyle`, and the relay's "other platforms refuse it". "The rules broken most often" says `tests/architecture.rs` now checks `ports R4`. |
| `.claude/rules/app.md` | The R4 bullet names the seams exactly (`notify/{dbus,toast,activation}.rs`, not `notify/`), says it binds production code, and its "Enforced by" names the check. |
| `.claude/rules/remote-agent.md` | A bullet on the relay's `target_os = "linux"` gate and R4. "Enforced by" names the check. |
| `.claude/rules/tests.md` | Test code may pair platforms (D1). |

### 8. Note for `port-to-macos`

`port-to-macos`'s files are not edited here (plan, "The overlap with changes in flight"). Its design still says "`cfg(target_os = "macos")` in a few small modules". When it is next revised, it should take these points from this change:
- **Ending the exception means emptying the list.** Every site in §3's 17 files moves behind a seam, a new seam or a new item in an existing one, and the README row and `R4_EXCEPTION` are deleted with them. The check rejects a macOS `cfg` anywhere else. Its title bar and keybinding gates (its §3, §4) need a seam for window chrome and one for key mapping, not `cfg`s in `workspace.rs` or `actions.rs`.
- **macOS refuses `relay` (D4).** The gate is `target_os = "linux"`, so a macOS helper builds without the relay and refuses it.
  - The relay dispatch in `remote-agent/src/main.rs` (`:62`–`:71`) is the one listed gate that doesn't end with the exception. Move it into the relay's seam (for example, `server/mod.rs` declaring the refusal beside the module), or give it a ports row of its own.
  - `server/daemon.rs`'s `cfg(unix)` `start_role_as` re-export serves only the relay, and becomes unused on macOS.
- **Process info.** Its `process_name` and `process_cwd` for macOS go in `src/terminal/process.rs` (already a seam). `LOCAL_PATH_STYLE` is `Posix` there through `cfg(not(windows))`.

## Risks / Trade-offs

- **[The text scanner misreads a form it wasn't taught]**
  - A string literal containing `cfg!(windows)` would be a false positive. None exists, and the failure names the line.
  - A block comment hiding a `cfg` would be a false positive too.
  - A `cfg` produced by a macro would be a false negative. No macro in the tree emits one.

  The planted-snippet tests pin each form the scanner handles, and the failure message says how to read a false positive. Review still covers anything else (R4 isn't only the check).
- **[Brace counting is fooled by braces in strings or chars inside a skipped `#[cfg(test)]` item]**
  - The worst case is a test item skipped too early or too late. Too early means a test-only site is reported, which is loud. Too late means a production site is missed.
  - The tree check also asserts that each `R4_EXCEPTION` file still has a site, so a miss in a listed file shows up as a failure.
- **[The list and the README must change together, so every cleanup touches two files]** → That is the point (`tests/architecture.rs`'s own rule), and the failure says so.
- **[`add-blocks` or another change moves a listed site]** → If the site leaves the file, `every_excepted_file_still_has_a_platform_cfg` fails, and that change shortens the list. If it moves to an unlisted file, `platform_cfgs_stay_in_their_seams` fails, and the site goes into a seam.
- **[Line numbers in §3 go stale]** → They are a snapshot for reviewers. The README and the check list files, not lines.
- **[Removing `Parser::posix()` and `Default` breaks a sibling change in flight]** → `bound-and-fuzz-outside-input` already says its fuzz test uses whichever constructor exists when it lands, and the compiler finds every caller.

## Migration Plan

- **Deploy:** none. The relay's gate changes only non-Linux Unix builds, which aren't shipped. Parsing is unchanged on Linux and Windows.
- **Rollback:** revert the PR. Doc, list and check go together, since they are one commit for the rule part.

## Open Questions

- **A Windows core's replays drop a 9;9 directory.** `Emulator::new` uses `Posix`, so a Windows core ignores `9;9;C:\…` reports. Their absolute check is POSIX. A reattached Windows tab therefore has no reported directory until the shell reports again. OSC 7 reports survive, because `/C:/…` passes the POSIX check and the replay re-emits them.
  - Fixing it would need a `server::platform` path-style constant passed to `Emulator::new`, and it changes what a replay contains.
  - It is a behaviour change outside this plan, so it goes to the backlog as a follow-up. This change only makes the choice explicit.
