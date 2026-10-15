# Tasks

## 1. Headless runner and the suite's environment assumptions

- [x] 1.1 Write `scripts/with-xvfb` as in design.md (`Xvfb -displayfd`, openbox waited on via `_NET_SUPPORTING_WM_CHECK`, `DISPLAY`/`BRINDLE_E2E_DISPLAY`, `WAYLAND_DISPLAY` unset, `VK_DRIVER_FILES` set to lavapipe, `LC_ALL=C.UTF-8`, `dbus-run-session`, cleanup trap, the command's exit status). Verify in a throwaway `ubuntu:26.04` container with design.md's package list, the checkout mounted at its own path, and `--user $(id -u):$(id -g)`:
  - `scripts/with-xvfb xdpyinfo` succeeds, and no Xvfb or openbox is left running afterwards.
  - `scripts/with-xvfb false` exits 1.
  - Two concurrent runs get different displays.
- [x] 1.2 In `colors-palette-tty-client` (`tests/e2e/cases/colors.rs`), set `TERM=xterm-256color` on the `script` child, so the case doesn't depend on the caller's `TERM`. Verify: in the container from 1.1 with `TERM` unset, `scripts/with-xvfb cargo test --test e2e -- colors-palette-tty-client` passes. It still passes on the desktop's normal path.
- [x] 1.3 Add `--step-interval <S>` to Brindle (`src/main.rs`: parsed with the other debug options, a positive number of seconds, else a usage error; the steps wait S/2 then S between steps; listed in `--help`), and have the harness pass it when `BRINDLE_E2E_SLOW` isn't 1 (`Ctx::step_interval`, used by `Run` and `restore.rs`'s own launch). Update CLAUDE.md's note on `BRINDLE_E2E_SLOW`. Verify: `--step-interval 0` exits with a usage error; `session-end-new-tab`, `session-end-ask` and `search-input-selection` pass at `BRINDLE_E2E_SLOW=2` under `scripts/with-xvfb`.
- [x] 1.4 Run the whole suite in that container on both tmux versions: `scripts/with-xvfb` around the e2e harness, with `/opt/tmux-3.8/bin` first on `PATH` for the second run, and `BRINDLE_E2E_SLOW=2`. Verify: 100 of 100 pass on both. If `colors-styled-pane` or another timing case fails, rerun it alone. A case that fails alone too gets fixed to wait for state rather than a fixed time.

## 2. Workflow: unit tests and release build

- [x] 2.1 Add `.github/workflows/ci.yml` with the triggers from design.md (push to any branch, `pull_request` with a job-level `if` that runs only for fork PRs, `workflow_dispatch`), a per-ref `concurrency` group with `cancel-in-progress`, `RUST_VERSION: 1.99.0` in the workflow `env`, and the `test` job on `ubuntu-26.04`:
  - apt build packages;
  - `dtolnay/rust-toolchain` at `RUST_VERSION`;
  - `Swatinem/rust-cache` with `shared-key: dev`;
  - `cargo test --locked`, then the e2e and perf `--self-test` runs.

  Verify: pushing a branch starts one run (not two), `rustc -V` in the log is 1.99.0, and `test` passes with the self-tests' `ok` lines in its log.
- [x] 2.2 Add the `release` job (`cargo build --release --locked`, its own cache key). Verify: it passes in parallel with `test`, and a second push shows cache hits and a shorter build.
- [x] 2.3 Verify `--locked` bites: on a scratch commit, add an unused dependency to `Cargo.toml` without updating `Cargo.lock`, push, and check that `test` fails on the lockfile rather than updating it. Drop the commit.

## 3. Workflow: end-to-end jobs

- [x] 3.1 Add the `e2e` job with a `tmux: ["3.6a", "3.8"]` matrix (`fail-fast: false`):
  - apt build, display and test packages from design.md;
  - for 3.8: `actions/cache` for `~/tmux-3.8` keyed on version and runner OS, and on a miss, download, check the pinned sha256, build, and add `~/tmux-3.8/bin` to `GITHUB_PATH`;
  - a step that fails unless `tmux -V` prints `tmux 3.6` or `tmux 3.8`.

  Verify: each entry's log shows the right version, and a re-run restores 3.8 from cache without building.
- [x] 3.2 In the same job: Rust as in `test`, rust-cache with `shared-key: dev`, `timeout-minutes: 60`, and `BRINDLE_E2E=1 BRINDLE_E2E_SLOW=2 scripts/with-xvfb cargo test --locked --test e2e`. Verify: both entries run every case, with none skipped for "no X display" and `geometry-fullscreen-less` not skipped. Record each entry's suite time in the commit message.
- [x] 3.3 Add an `if: failure()` step uploading `target/tmp/e2e/` as `e2e-tmux-<v>` with 14-day retention. Verify: make one case fail on a scratch commit (for example a wrong expected color), push, and download the artifact (it holds that case's dump, log and capture). Drop the commit.
- [x] 3.4 Fix what the first runner runs turn up, mostly timing. For a flake, rerun the case alone; fix it to wait for state if it keeps failing. Tune `BRINDLE_E2E_SLOW` and `timeout-minutes` from the recorded times. Verify: three consecutive pushes pass on both tmux versions.

## 4. Docs

- [ ] 4.1 Document CI:
  - README.md's Testing section: what CI runs, the status badge, and `scripts/with-xvfb cargo test --test e2e` (after `apt install xvfb openbox`) as the way to run the suite headless, as CI does.
  - CLAUDE.md's "End-to-end suite" section:
    - the CI jobs, and that e2e is advisory;
    - how to move the pins: the matrix and checksum for tmux, `RUST_VERSION` for Rust;
    - that the suite needs a window manager and a UTF-8 locale, both of which `scripts/with-xvfb` provides;
    - that runs under `scripts/with-xvfb` open no windows on the desktop, so they don't need asking first, unlike runs on `:0`.

  Verify: the badge renders on GitHub, and every documented command runs as written.
- [x] 4.2 Record the locale bug as a follow-up OpenSpec change (proposal only): Brindle should pass `-u` to tmux, because without a UTF-8 locale tmux replaces the tabs in its control-mode replies with `_` and tmux tabs never appear. Verify: `openspec list` shows it.
- [ ] 4.3 Final check on `main` after the merge is pushed. Verify: the Actions page for the merge commit shows `test`, `release` and both `e2e` entries green.
