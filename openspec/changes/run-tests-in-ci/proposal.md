# Proposal

## Why

The repository has no CI. Unit tests, release builds and the end-to-end suite only run when someone remembers to run them locally, and the end-to-end suite needs a display, so it runs least often. The tmux 3.6 crash found by the suite shows the value of running it routinely.

## What Changes

- **`scripts/with-xvfb`:** runs a command against a private, headless X server with a window manager, software Vulkan (Mesa lavapipe), a UTF-8 locale and a private session bus. CI uses it to run the end-to-end suite. It works the same on a developer machine that has `xvfb` and `openbox` installed, so the suite can run there without opening windows on the desktop.
- **A GitHub Actions workflow**, `.github/workflows/ci.yml`, directly on `ubuntu-26.04` runners (no containers), with Rust pinned to 1.99.0. It runs for pushes to any branch, pull requests from forks, and by hand:
  - **`test`:** `cargo test --locked`, plus the e2e and perf harnesses' `--self-test`. A failure here is the signal that something is broken.
  - **`release`:** `cargo build --release --locked`, its own job, so thin LTO and stripping are checked without slowing the others.
  - **`e2e`:** `BRINDLE_E2E=1 cargo test --locked --test e2e` under `scripts/with-xvfb`, once per tmux version, uploading `target/tmp/e2e/` when it fails. It reports only at first: its jobs go red on failure but aren't required checks until the suite proves stable on runners.
- **tmux versions:** Ubuntu's 3.6a, the desktop's version, from apt. 3.8, the latest release, is built from a pinned, checksummed tarball and cached by version. The `tmux-*` behaviour cases then flag tmux changes when the pins move.
- **`--step-interval <S>`:** a debug option that spaces Brindle's `--send`/`--action` steps S seconds apart instead of one. The e2e harness passes it whenever `BRINDLE_E2E_SLOW` isn't 1, so the slow-down factor stretches the steps along with the harness's own timeline. Until now it didn't, so cases that interleave steps with timed tmux events failed deterministically at `BRINDLE_E2E_SLOW=2`.
- Cache Cargo's registry and dependency builds; GPUI builds are slow.
- Mention CI and `scripts/with-xvfb` in README.md and CLAUDE.md.

## Capabilities

### New Capabilities

None. This is build tooling.

### Modified Capabilities

- `command-line`: the debug automation hooks gain `--step-interval`.

## Impact

- **New:** `scripts/with-xvfb`, `.github/workflows/ci.yml`.
- **Changed:** `src/main.rs` (`--step-interval`); `tests/e2e/main.rs`, `tests/e2e/brindle.rs` and `tests/e2e/cases/restore.rs` pass it.
- **Possibly:** fixes in `tests/e2e/` for whatever a headless display turns up (see design.md's spike results).
- **Found by the spike, out of scope:** Brindle's tmux control mode breaks when the locale isn't UTF-8, because tmux then replaces the tab separators in its replies with `_`. CI sets a UTF-8 locale. The fix, passing `-u` to tmux, is a follow-up change.
