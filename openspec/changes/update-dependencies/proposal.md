# Proposal

## Why

GitHub lists 11 open Dependabot security alerts against `Cargo.lock`, and five Dependabot PRs (#1–#5) have sat unmerged since early October, because CI only runs pull requests from forks, so none of them was ever tested. Rust 1.99.1 came out on 2026-10-13 with a fix for a miscompilation of FFI `bool` return values on x86_64, which matters to a program built on many C libraries.

## What Changes

- **CI runs on every pull request**, including Dependabot's and those from branches in this repository. Pushes still run CI on every branch, except Dependabot's, which always have a PR.
- **Merge the five Dependabot branches**, so GitHub marks the PRs merged:
  - #1 `bytes` 1.10.1 → 1.12.1;
  - #2 `quinn-proto` 0.11.13 → 0.11.19, which also adds `rand` 0.10 and its dependencies to the lockfile;
  - #3 `rand` 0.8.5 → 0.8.8;
  - #4 `rustls-webpki` 0.103.7 → 0.103.15;
  - #5 `rustls` 0.23.33 → 0.23.45 (with the same `rustls-webpki` bump).
- **`rand` 0.9.2 → 0.9.3**, which fixes an alert that has no Dependabot PR (#7).
- **Rust 1.99.1:** `RUST_VERSION` in CI, and the desktop's toolchain.
- **Dependabot version updates for GitHub Actions**, in a new `.github/dependabot.yml`. Cargo stays on security updates only.
- **`grid` alert (#2) stays open:** the fix is `grid` 1.0.1, but `taffy` 0.9 (inside GPUI 0.2.2) requires `grid` 0.18. Brindle can't take it until GPUI moves.

Already current, so no change: GPUI 0.2.2, `alacritty_terminal` 0.26.0, tmux 3.8, and the action majors (`checkout@v7`, `cache@v6`, `upload-artifact@v7`, `rust-cache@v2`).

## Capabilities

### New Capabilities

None. This is build tooling and dependencies.

### Modified Capabilities

None (`skip_specs: true`): no user-visible behavior changes.

## Impact

- `.github/workflows/ci.yml` (triggers and job conditions), a new `.github/dependabot.yml`.
- `Cargo.lock`: only the packages above, through the Dependabot merges and one `cargo update --precise`. No broad `cargo update` and no `libc` bump, per CLAUDE.md.
- README.md's and CLAUDE.md's notes on what CI runs and on `RUST_VERSION`.
- The five Dependabot PRs close as merged when `main` is pushed.
