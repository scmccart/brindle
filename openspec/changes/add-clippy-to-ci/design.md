# Design

## Context

See proposal.md (Why). This change closes audit finding 7 (F7: the `never_loop` error, e1's clippy failure and its warnings), the "Enforcement gaps" rows for `railway R3`/`types R2` and `railway R9`/`R5`, and the "Suppressions" item (three `too_many_arguments` allows without a `reason`). `docs/architecture/audits/2026-11-16/e1.md` has the original run; it was repeated on 2026-11-16 for the lists below. The code is unchanged since the audit.

Facts that shape the approach:

**What clippy says today** (`cargo clippy --locked --workspace --all-targets`, toolchain 1.99.1 from `rust-toolchain.toml`). One error, then warnings by target (the bin test target repeats the bin's, so counts are of distinct sites):
- **Error:** `never_loop`, `src/remote/local/connection.rs:2874`, in a test: `let written = loop { match second.next() { ToCore::Close(close) => break (…), other => panic!("not CLOSE: {other:?}") } };`.
- **Production code (`brindle` bin):**
  - `very_complex_type`: `src/actions.rs:105`, `:227`; `src/remote/local/connection.rs:980`; `src/remote/client.rs:1952`; `src/session.rs:393`; `tests/e2e/main.rs:446`;
  - `large_enum_variant`: `src/picker.rs:329`, `src/session.rs:192`, `:612`, `src/settings_dialog/profiles.rs:513`, `src/settings_dialog.rs:975`;
  - `result_large_err`: `src/layout.rs:198`;
  - `collapsible_if`: `src/config.rs:313`, `src/terminal/keys.rs:54`, `tests/e2e/main.rs:138`;
  - single sites: `needless_borrows_for_generic_args` (`src/instance.rs:223`), `while_let_loop` (`src/remote/client.rs:68`), `wrong_self_convention` (`src/remote/client.rs:1664`), `empty_line_after_doc_comments` (`src/remote/local/session.rs:173`), `unnecessary_map_or` (`src/theme.rs:483`), `bool_assert_comparison` (`src/remote/mod.rs:317`), `bool_comparison` (`src/settings_dialog/profiles.rs:1425`), `manual_pattern_char_comparison` (`remote-agent/src/pty/unix.rs:345`);
  - `unit_arg`, `src/config_edit.rs:762`, `:773`, `:785`, `:800`;
  - `cloned_ref_to_slice_refs`: `src/actions.rs:551`, `src/reattach.rs:500-505` (4), `src/remote/local.rs:1115` (2), `remote-agent/tests/holder.rs:263`.
- **`brindle-term-core`:** `derivable_impls` (`shell_integration.rs:103`), `question_mark` (`:125`), `manual_range_patterns` (`:190`), and in tests `needless_update` (`:1026`, `:1060`).
- **e2e harness:** 22 sites (11 distinct): `tests/e2e/capture.rs:88`, `:307`, `:314`; `tests/common/mod.rs:21` (`neg_cmp_op_on_partial_ord`, 12 in all with the repeats); `tests/e2e/cases/remote.rs:671`, `:883`, `:893`; `tests/e2e/cases/tabs.rs:196` (`double_ended_iterator_last`); `tests/e2e/main.rs:138`, `:173`, `:446`.
- The lint names above are inferred from clippy's messages (a short-format run prints none), and the first task re-runs clippy to get the exact list. Fixing `never_loop` can bring more lints out of the bin test target, so the list is a start, not the whole.

**No clippy configuration exists.** No `clippy.toml`, no `[lints]` table in any manifest, and the only crate-root attribute is `src/main.rs:3` (`#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]`), which is the precedent for a `cfg_attr(not(test), …)` at a root.

**Suppressions.** Three `#[allow(clippy::too_many_arguments)]` have no `reason` (`src/zone_view.rs:432` `ZoneTreeView::from_restored`, `src/terminal/mod.rs:333` `Terminal::new`, `:403` `Terminal::start_session`). A fourth group exists that the audit's count (5 `#[allow(`) missed: 37 `#[cfg_attr(…, allow(dead_code))]` in `src/terminal/{mod,process}.rs`, which gate unused items by platform.

**Restriction lints today** (run with `-W` on each crate, all targets):
- `unwrap_used`/`expect_used` on `brindle-remote-protocol` and `brindle-term-core`: **no** hit in non-test code; the only hits are six helper functions in `remote-protocol/tests/transcripts.rs`, an integration test crate that a `lib.rs` attribute never reaches.
- `wildcard_enum_match_arm`: about 55 sites across the workspace including test targets. In production code: `remote-protocol/src/{access,holder}.rs` (1 each), `term-core/src/{emulator,osc}.rs`, `ui/src/{style.rs,components/tabs.rs,components/menu.rs}`, `remote-agent/src/server/{cli.rs (5),relay.rs (2)}` and `platform/unix/daemon.rs`, and about 35 in `src/` (`settings_dialog/profiles.rs` 10, `remote/local/session.rs` 9, `remote/local/connection.rs` 9, `terminal/mod.rs` 4, `settings_dialog.rs` 4, `remote/client.rs` 4, the rest one or two each). The lint can't tell whether the enum is Brindle's, so each site is classified by hand in task 3.

**CI today** (`ci.yml`): `commits`, `test`, `release` (`build-linux.yml`), `windows` (`build-windows.yml`, needs `release`), `windows-e2e`, `e2e` in three shards. Release pull requests (`release-please--…`) skip everything but `commits` and `test` (and the title workflow). `build-windows.yml` is also called by `release.yml`. The two jobs that compile in the dev profile on Linux share the cache key `dev`.

## Goals / Non-Goals

**Goals:**
- Clippy is green, and CI, on both platforms, keeps it green.
- `railway R3`/`types R2` and `railway R5`/`R9` have a mechanical check where one exists: a new `_ =>` on an enum, or a new `unwrap`/`expect` in the two decoder crates, fails CI.
- Each suppression says why.
- The docs stop saying "there is no clippy step in CI".

**Non-Goals:**
- Removing the existing `_ =>` arms on Brindle's own enums, or the `unwrap`/`expect` in the other three crates (`railway R9` is SHOULD; the exception row's "ends with" stays "next changed"). `surface-swallowed-failures` and `extract-decisions-into-models` do that as they touch each match.
- `clippy::pedantic`, `clippy::nursery`, and other restriction lints (`panic`, `indexing_slicing`, `allow_attributes_without_reason`: see decision 5).
- Clippy on macOS, the musl helper target, or `cargo fmt` in CI (`rustfmt` is installed but unchecked; a separate decision).
- Enabling the lints in test code, examples (`ui/examples/gallery.rs`) and the e2e and perf harnesses beyond the default clippy set.

## Layers

This change adds no program code of its own beyond fixes; it is CI configuration, lint attributes at crate roots, and docs.
- **Pure code, shell, seams:** unchanged. The fixes keep each function in its layer; none moves a decision (`fcis R1`), and none touches a seam (`ports R4`).
- **Crate graph:** no reference is added. The attributes sit in each crate's root (`src/main.rs`, `remote-agent/src/main.rs`, `remote-protocol/src/lib.rs`, `term-core/src/lib.rs`, `ui/src/lib.rs`), so `ports R3`'s closed lists are untouched.
- **Outcome and state types:** none added. `railway R3`/`types R2` are what the wildcard lint holds, which is why a match on Brindle's own enum that needs an allow must say so (decision 4).
- **Failure display:** a lint is a compile-time diagnostic. Locally it prints as a warning; in CI `-D warnings` fails the job, and the job's log names the file and line.
- **Wire, config, state file, terminal server:** untouched, so no protocol version, transcript, or core-restart/upgrade question arises (`process R4`–`R6`, `R1`–`R3`, `R12`).

## Decisions

### 1. Lint levels are crate-root attributes, not `[workspace.lints]` and not command-line flags

Each of the five roots gets:

- `#![cfg_attr(not(test), warn(clippy::wildcard_enum_match_arm))]`, and for `remote-protocol` and `term-core` also `#![cfg_attr(not(test), warn(clippy::unwrap_used, clippy::expect_used))]`.

Why this, and not the alternatives:
- **`[workspace.lints.clippy]` (rejected):** it applies to every target, including integration tests, so `remote-agent/tests/` (about 36 wildcard sites) and `remote-protocol/tests/transcripts.rs` (the six `expect`s) would need allows, and the lints would then govern test code, which `railway R9` and the audit leave out. `clippy.toml`'s `allow-unwrap-in-tests` covers `#[test]` functions and `#[cfg(test)]` modules but not helpers in a test crate, as the transcripts file shows.
- **Command-line flags in CI (rejected):** `cargo clippy -- -W clippy::…` is invisible to an editor and to a developer who runs plain `cargo clippy`, who then meets the failure only in CI. An attribute shows the same warning in both places, and a script or doc needn't repeat a flag list.
- **The attribute's reach:** `not(test)` drops it from a crate's own unit-test build, and integration-test and example crates are separate crates the root never reaches. So tests, `tests/e2e`, `tests/perf` and `ui/examples` stay out of scope with no per-file allow, which is the audit's "non-test code".
- **Level is `warn`, not `deny`:** a developer's iteration isn't blocked by a style lint, and CI's `-D warnings` turns every warning into a failure (decision 2). `deny` would also fail `cargo build`, which this change should not do.

### 2. One CI command, `-D warnings`, on both platforms

- **Command:** `cargo clippy --locked --workspace --all-targets -- -D warnings`. `--all-targets` matches the audit's run and covers tests, benches, examples and the e2e and perf harnesses under the default lint set.
- **Linux (`ci.yml`):** a new `clippy` job beside `test`: `runs-on: ubuntu-26.04`, the same `BUILD_PACKAGES` install, `rustup toolchain install`, `Swatinem/rust-cache@v2` with `shared-key: dev`. It has no `if:` on `release-please--` branches, like `commits` and `test`: a release PR changes versions only, and the check is the same as the test job's cost class (no GUI, no release profile), so it keeps `release-process`'s "like any other pull request" true with no delta (decision 7).
- **Windows (`build-windows.yml`):** one step after `cargo test --locked --workspace`, after `scripts/fetch-conpty.ps1` (`remote-agent/build.rs` embeds the ConPTY) and with the `windows` cache. It needs no `fxc.exe`, which only GPUI's release shader compile reads. The Linux run never sees `cfg(windows)` code (`src/win.rs`, `terminal/process.rs`, the helper's `conpty/` and `platform/windows/`), so without this step the Windows half of the code is unlinted.
- **Why in `build-windows.yml` and not `ci.yml`:** the workflow is already the Windows job's body, with the runner, Rust, cache and ConPTY steps; a separate job would repeat them and compile the dev profile twice on a slow runner. Releases call it too, so a release is also checked by clippy ("Release builds pass CI's checks").
- **Why not `build-linux.yml`:** it is the release profile and the musl helper, and its steps are builds; clippy is on Linux in `ci.yml`.
- **`-D warnings` and the future-incompat note:** the `proc-macro-error2`/`num-bigint-dig` message from cargo is a report on dependencies, not a rustc or clippy warning of a workspace crate, so `-D warnings` (which applies to the workspace's own crates, not dependencies) doesn't fail on it. Task 4 confirms it on the first run, and CLAUDE.md's "one build warning left on purpose" note stays true.
- **Windows can't be run here.** The Windows step's first result is the PR's own `windows` job. Task 5 fixes what it finds in the same change; the Linux machine's session can't check Windows code and a Windows session can't be assumed, per CLAUDE.md's Windows notes.

### 3. Fix each warning, or allow it with a reason; never a bare allow

Policy for the ~90 warnings (task 1 and 2):
- **Fix** (almost all): the mechanical ones (`collapsible_if`, `cloned_ref_to_slice_refs`, `unit_arg`, `unnecessary_map_or`, `question_mark`, `derivable_impls`, `manual_range_patterns`, `while_let_loop`, `bool_comparison`, `empty_line_after_doc_comments`, the harness's `chunks_exact_to_as_chunks`, `unnecessary_sort_by`, `manual_contains`, `useless_conversion`, `redundant_pattern_matching`, `double_ended_iterator_last`), each with no behaviour change; run the touched module's tests.
- **Fix by a type alias** for `very_complex_type` (six sites): a named `type` where the type has a meaning (a callback, a result of a lookup), which also helps `types R10`. If a name would not say anything, allow it.
- **Allow with a reason** where the lint is right in general and wrong here:
  - `large_enum_variant` (five enums, 288–824 bytes): these are values built once or a few at a time (`picker`, `session`, the settings dialogs' form states), not stored in bulk. Boxing the large variant would add an indirection and a construction site per variant for no measured gain. `#[allow(clippy::large_enum_variant, reason = "built once per dialog; boxing adds an allocation for no gain")]`, each saying its own reason. If measuring `size_of` shows a variant that is stored in a `Vec` of many, box it instead.
  - `result_large_err` (`src/layout.rs:198`): the `Err` is a 256-byte value; if it is the layout's own error with several fields, box nothing and allow with the reason; if it can be a smaller type cheaply (`railway R4` favours a typed error anyway), do that.
  - `wrong_self_convention` (`src/remote/client.rs:1664`): rename if the method name is private; otherwise allow.
- **The three `too_many_arguments` allows** get `reason = "…"`: the reason is written from each signature when the task reads it (what the caller already holds, and why a parameter struct would be a one-use type, if it would). Changing the signatures is `extract-decisions-into-models`'s ground (b-entries for `Terminal`), not this change's.
- **`#[allow]`, not `#[expect]`:** `#[expect]` fails the build when the lint doesn't fire, and a site can fire on one platform's `cfg` and not the other (decision 2 notes Windows code Linux never sees).

### 4. The wildcard lint and `railway R3`'s exception

The lint reports every `_ =>` (or binding wildcard) over an enum, and can't see whose enum it is. Each site in non-test code gets one of three treatments, decided by the enum:
1. **A foreign enum** (`alacritty_terminal`, `gpui`, `std`, `windows`, `x11rb`, `serde_json::Value`): `#[allow(clippy::wildcard_enum_match_arm, reason = "foreign enum: <type>")]` on the `match` (or the enclosing statement/function when the attribute can't sit on an expression). The rule's own text allows these ("Wildcards are for foreign types, bytes, strings and integers").
2. **One of Brindle's own enums, cheap to fix** (the match is short and each missing variant has an obvious arm): name the variants. This is the rule, and the arm that remains after it is typically one line.
3. **One of Brindle's own enums, not cheap** (a long match whose wildcard hides several distinct cases, or one where naming the variants forces a decision this change doesn't own): `#[allow(clippy::wildcard_enum_match_arm, reason = "railway R3 exception: <what the arm does>")]`. The exact text `railway R3 exception` is what ends the exception: `grep -rn "railway R3 exception" --include=*.rs` lists what is left, and the README's row says so.

**The exception row** in `docs/architecture/README.md` today reads "Existing `_ =>` arms have not been audited for matches on Brindle's own enums." That stops being true: after the change, the arms are audited by the lint, for non-test code. The row becomes: the arms on Brindle's own enums that an `allow(…, reason = "railway R3 exception: …")` marks; "Approved in" stays adoption, 2026-11-16; "Ends with": each allow is removed when its `match` is next changed, and the row goes when a search finds none. The row's scope is non-test code only, which the doc's text on R3 already assumes (tests `panic!` on an unexpected variant).

**Test code and the harnesses** keep their wildcards: `other => panic!(…)` is how a test fails on a wrong message, and the lint doesn't run there (decision 1).

### 5. No `allow_attributes_without_reason`

It would catch a future bare `#[allow]`, but it also flags the 37 `#[cfg_attr(…, allow(dead_code))]` in `src/terminal/{mod,process}.rs`, so enabling it means a reason on each, for attributes that say what they do. The three `too_many_arguments` allows are fixed by hand (decision 3), and review holds the rest. Revisit when those `dead_code` attributes move behind their seams (`port-to-macos`).

### 6. `never_loop` becomes a `let … else`

`src/remote/local/connection.rs:2874`: the `loop` runs once or panics. It becomes `let ToCore::Close(close) = second.next() else { panic!("not CLOSE: …") };` followed by `let written = (close.session, Instant::now());`, with the same panic message and no change in what the test waits on. Verified by the test that contains it (`connection::tests`, the closed-while-reconnecting case) still passing; no sleep or timing change (`testing R7`, `R8`).

### 7. Clippy runs on release pull requests

CI skips the builds and e2e jobs on `release-please--…` branches, because the code is unchanged and the release run builds again. Clippy is not in that set: a release PR changes versions and the changelog only, so the job passes the same way `test` does, and leaving it unskipped keeps `release-process` ("Release pull requests run CI") unchanged, with no delta. Cost: a dev-profile check per release PR, on a cache `test` has warmed. If that cost becomes a problem, the job gets the same `if:` and the spec gets a delta then.

### 8. `tests/architecture.rs` checks that the attributes are there

Without a check, deleting a crate-root attribute turns a rule's lint off silently. A new test `the_crate_roots_enable_the_restriction_lints` (name per `testing R15`) reads the five roots and asserts each contains `clippy::wildcard_enum_match_arm` inside a `cfg_attr(not(test), warn(…))`, and `remote-protocol/src/lib.rs` and `term-core/src/lib.rs` contain `clippy::unwrap_used` and `clippy::expect_used` too. It follows the file's pattern: a search over named files that asserts its selection isn't empty, beside a positive-direction test (`a_root_without_the_lint_is_reported`) that feeds the check a planted root. It is text search, as the file's other checks are, and says so in its comment; the lint itself, not this test, is what finds a violation.

It does not check the CI job exists: no test reads workflows today, and the audit's list of the architecture test's gaps is not widened here.

### 9. Docs

- **`railway-oriented-programming.md` Enforcement:** the bullet "R1, R3, R4 and R6–R12 rely on review. There is no clippy step in CI, so `unwrap_used` and wildcard arms are not linted." becomes: R3 is held by `clippy::wildcard_enum_match_arm` in CI on non-test code (foreign enums allowed with a reason, own-enum exceptions marked `railway R3 exception`); R5's "never panic on outside input" and R9 are held on `remote-protocol` and `term-core` by `clippy::unwrap_used`/`expect_used`; the rest (R1, R4, R6–R12 beyond those) rely on review.
- **`illegal-states-unrepresentable.md` Enforcement:** its bullet "The compiler holds R2 wherever the wildcard is absent" gains that the lint holds the wildcard in non-test code.
- **`testing-architecture.md` Enforcement:** the CI bullet adds that CI runs clippy with `-D warnings` on both platforms.
- **`docs/architecture/README.md`:** the Enforcement paragraph names clippy beside `tests/architecture.rs`, and the exceptions row changes (decision 4).
- **`CLAUDE.md`:** "Build and test" gets `cargo clippy --locked --workspace --all-targets -- -D warnings`; the CI paragraph names the `clippy` job and the Windows step; the `rust-toolchain.toml` pin note says a toolchain bump clears its new lints in the same change; the "Architecture rules" list's "rules broken most often" is unchanged.

## Risks / Trade-offs

- **[The lint list is longer after `never_loop` is fixed]** → Task 1 re-runs clippy and records the exact list in the PR before fixing; the design's list is a starting point.
- **[Windows-only code has warnings Linux never showed]** → The PR's `windows` job is the first run. Fix what it reports in this change; don't merge with that step failing or marked allowed-to-fail.
- **[A toolchain bump (`update-dependencies`) adds lints and turns CI red]** → Intended: the bumper clears them in the bump's own change. CLAUDE.md says so.
- **[The wildcard lint's allows hide real `railway R3` breaches]** → Only if a foreign-enum reason is false. Review checks the `foreign enum: <type>` text on each allow, and own-enum sites are the distinct `railway R3 exception` text, countable by a search.
- **[`warn` lets a local build pass what CI rejects]** → Intended (decision 1). The architecture test pins the attributes, CI pins `-D warnings`.
- **[`large_enum_variant` allows get stale]** → A reason that says "built once" is checkable at the construction sites; no lint can keep it true. Accepted; low stakes.
- **[Clippy compile time on the Windows runner]** → A dev-profile `check` of the workspace after the tests built it. The first run is cold; later runs use the `windows` cache. Measured and reported in the PR, not asserted (`testing R8`).

## Migration Plan

- **Deploy:** none; no shipped binary changes beyond the small fixes.
- **Order:** fix the build error and warnings first, so the job is green when added; add the lints and their allows next; the CI job and step last, in the same PR.
- **Rollback:** revert the PR. No config, state file or protocol is touched.
- **Branch protection:** the repository's required check is the PR title only; `clippy` is advisory until the user makes it required, which is a repository setting this change doesn't make.

## Open Questions

None blocking. Whether `cargo fmt --check` joins the same job is left to a separate change.
