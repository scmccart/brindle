# Proposal

## Why

The 2026-11-16 architecture audit (`docs/architecture/audits/2026-11-16.md`, finding 7 and "Enforcement gaps"; change 6 of `2026-11-16-remediation.md`) found that clippy is in `rust-toolchain.toml` (`components = ["rustfmt", "clippy"]`) but runs nowhere:
- `cargo clippy --locked --workspace --all-targets` fails. `clippy::never_loop` is a deny-by-default error at `src/remote/local/connection.rs:2874` (a `loop` in a test whose body always breaks or panics), and it stops the `brindle` bin test target compiling under clippy. About 90 warnings sit beside it (e1; re-run on 2026-11-16, below).
- No step in `.github/workflows/ci.yml`, `build-linux.yml` or `build-windows.yml` runs clippy, so nothing keeps it green.
- Two MUST rules have no check but a lint could hold them: `railway R3`/`types R2` (no `_ =>` arm on Brindle's own enums; 173 `_ =>` arms in the audit's sweep, most on foreign types) and `railway R5`/`R9` (outside input never panics, and `unwrap`/`expect` stay out of non-test code; 1,550 uses in 85 files, mostly tests).
- Three `#[allow(clippy::too_many_arguments)]` (`src/zone_view.rs:432`, `src/terminal/mod.rs:333`, `:403`) carry no `reason`, and no doc mentions them. The audit asks that they get one when clippy enters CI.

No code here breaks a rule the lint would not also catch in review, so the change is a gate, not a fix: a rule Brindle already holds is checked by CI instead of by a reader.

## What Changes

- **Fix the build error and the warnings:** `never_loop` becomes a `let … else`. Each other warning is fixed, or silenced with `#[allow(…, reason = "…")]` where the code is right and the lint is wrong (for example `large_enum_variant` on enums that are built once). The three `too_many_arguments` allows get a `reason`.
- **A `clippy` job in `.github/workflows/ci.yml`:** `cargo clippy --locked --workspace --all-targets -- -D warnings` on `ubuntu-26.04`, with the same packages, toolchain and cache as `test`. It also runs on release pull requests, like `commits` and `test`, so `release-process`'s requirement that the release PR "trigger pull-request checks like any other" holds without a delta.
- **A clippy step in `.github/workflows/build-windows.yml`:** the same command on `windows-latest` after the `cargo test` step, because Linux clippy never sees `cfg(windows)` code. Releases call this workflow, so a release is built by the same checks as CI.
- **Restriction lints, switched on as the crate roots' own attributes** (`#![cfg_attr(not(test), warn(…))]`, so test code is out of scope and a local `cargo clippy` shows them as CI does):
  - `clippy::wildcard_enum_match_arm` on all five crates (`railway R3`, `types R2`);
  - `clippy::unwrap_used` and `clippy::expect_used` on `brindle-remote-protocol` and `brindle-term-core`, whose non-test code has none today (`railway R5`, `R9`).
- **Existing wildcard sites:** the lint can't tell Brindle's enums from foreign ones. A match on a foreign enum gets `#[allow(clippy::wildcard_enum_match_arm, reason = "foreign enum: …")]`. A match on one of Brindle's own enums is fixed by naming the variants where the change is small, and otherwise gets `reason = "railway R3 exception: …"`, so one search lists what the exception still covers.
- **Docs and the architecture test:**
  - the `railway R3`, `types R2` row in `docs/architecture/README.md`'s exceptions table now says the lint lists the remaining arms, and each is removed when its `match` is next changed;
  - the Enforcement sections of `railway-oriented-programming.md` (its "no clippy step in CI" line is false after this change), `illegal-states-unrepresentable.md` and `testing-architecture.md`, and the README's Enforcement paragraph;
  - a `tests/architecture.rs` check, with a positive-direction test, that the five crate roots carry the attributes, so deleting one fails `cargo test`;
  - `CLAUDE.md`: the build commands, the CI job list, and a note that a toolchain bump clears its new lints in the same change.

No user-visible behaviour changes. No rule is added or loosened: `railway R3`, `R5` and `R9` and `types R2` are applied as written, and the exception row narrows.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. No requirement in `openspec/specs/` changes, so `.openspec.yaml` sets `skip_specs: true`. `release-process` "Release builds pass CI's checks" says only that releases run the same build-and-check steps as CI, which the Windows step satisfies by being in the shared workflow; "Release pull requests run CI" lists the checks that run on a release PR as a floor, and the new job runs there too.

## Impact

- **Code:**
  - `src/remote/local/connection.rs` (`never_loop`), `src/zone_view.rs`, `src/terminal/mod.rs` (`reason`s);
  - the warnings' files, by the list in design.md: `term-core/src/shell_integration.rs`, `remote-agent/src/pty/unix.rs`, `remote-agent/tests/holder.rs`, `tests/e2e/{main,capture}.rs`, `tests/e2e/cases/{remote,tabs}.rs`, `tests/common/mod.rs`, and in `src/`: `actions.rs`, `config.rs`, `config_edit.rs`, `instance.rs`, `layout.rs`, `picker.rs`, `reattach.rs`, `session.rs`, `settings_dialog.rs`, `settings_dialog/profiles.rs`, `theme.rs`, `terminal/keys.rs`, `remote/{client,local,mod}.rs`, `remote/local/{connection,session}.rs`;
  - the five crate roots (`src/main.rs`, `remote-agent/src/main.rs`, `remote-protocol/src/lib.rs`, `term-core/src/lib.rs`, `ui/src/lib.rs`), and the wildcard sites the lint finds under them.
- **CI:** `.github/workflows/ci.yml` (a `clippy` job), `.github/workflows/build-windows.yml` (a step). `build-linux.yml` is unchanged: CI's `clippy` job covers Linux, and releases of the Linux build run `test` and `clippy` on the same commit through the pull request.
- **Docs:** `docs/architecture/README.md`, `railway-oriented-programming.md`, `illegal-states-unrepresentable.md`, `testing-architecture.md`, `CLAUDE.md`; `tests/architecture.rs`.
- **Dependencies:** none. `Cargo.lock` is untouched.
- **Overlap with other changes:**
  - **`update-dependencies`** (outline only): a toolchain or dependency bump can add lints, and `-D warnings` turns each into a red build. Whichever lands second clears its new lints in its own change. CLAUDE.md's pin note says so. If it moves `rust-toolchain.toml`, nothing here depends on the version.
  - **`port-to-macos`:** a macOS clippy run is not added; it joins the matrix when macOS joins CI.
  - **`add-blocks`, `add-pocket-client`:** new code lands under the lints. `add-pocket-client`'s `espidf` gating is outside what CI's targets build.
  - **Remediation siblings:** `bound-and-fuzz-outside-input` adds fuzz code under `#[cfg(test)]`, outside the lints. `enforce-platform-seams` and `keep-pure-modules-pure` also edit `tests/architecture.rs`; each adds its own test functions, so a rebase conflict is only adjacent lines. `surface-swallowed-failures` and `extract-decisions-into-models` remove `_ =>` arms and `railway R3 exception` allows as they touch each match.
