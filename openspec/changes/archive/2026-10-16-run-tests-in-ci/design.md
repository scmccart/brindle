# Design

## Context

- The repository is public on GitHub (`scmccart/brindle`), so Actions minutes on standard Linux runners are free. Changes land by merging branches locally with `--no-ff` and pushing, not through pull requests.
- `ubuntu-26.04` runners have been generally available since 2026-09-17 (`ubuntu-latest` moves to them in November 2026). Image 20260927 ships `xvfb`, `jq`, `bison`, `locales` and Rust 1.98.1, but no tmux, openbox or Mesa Vulkan drivers. The desktop is Ubuntu 26.04.1 with Rust 1.99.0 and Ubuntu's `tmux` 3.6a.
- `Cargo.lock` was seeded from gpui 0.2.2's lockfile and must not be regenerated (a newer `libc` breaks `xattr`).
- Building GPUI from scratch takes many minutes, and dependencies are built at opt-level 2 even in the dev profile.
- The e2e suite (`tests/e2e/`) is a `harness = false` target gated on `BRINDLE_E2E=1`.
  - It runs `CARGO_BIN_EXE_brindle`, the dev binary from the same `cargo test`.
  - It talks to X on `BRINDLE_E2E_DISPLAY` (default `:0`), with `WAYLAND_DISPLAY` removed. Cases that need a display are skipped when there's no X server, and `tmux-*` cases need none.
  - It captures windows with x11rb `GetImage` and compares cell colors with the `e2e` test theme, so it depends on a font being found, not on which one.
  - `BRINDLE_E2E_SLOW` stretches every delay. Artifacts land in `target/tmp/e2e/<case>/`.
- On Linux, GPUI renders with Blade over Vulkan. With no GPU, the only Vulkan driver is Mesa's lavapipe (`mesa-vulkan-drivers`).
- Brindle follows tmux pane state by polling on tmux < 3.7 and with subscriptions on 3.7 and later. Upstream's latest release is 3.8 (2026-09-09).

### Spike results (2026-10-14)

The spike ran in a throwaway `ubuntu:26.04` container standing in for a fresh runner, installing only the packages listed under Decisions. It ran the desktop's dev build and e2e harness against an Xvfb inside the container.

- **Blade runs on lavapipe under Xvfb.** The adapter is `llvmpipe (LLVM 21.1.8, 256 bits)`. A local-shell `--dump-screen-after` run shows the expected screen, and the pixel case `colors-attach` passes.
- **A window manager is required.** Without one, `dump-grid-geometry`, `styles-borders` and the palette and multi-window cases fail: palette actions don't land, and a dismissed prompt opens a window anyway. With `openbox` running, all of them pass.
- **A UTF-8 locale is required.** With no `LANG`, tmux replaces the tab separators in its control-mode replies with `_`. Brindle can't parse the window list and never shows the session, so the dump is empty. With `LANG=C.UTF-8` it works. `tmux -u` fixes it regardless of locale; that's a Brindle fix for a follow-up change, not CI's.
- **Ubuntu's 3.6a reports `tmux 3.6`** from `tmux -V`.
- **tmux 3.8 builds** from `tmux-3.8.tar.gz` (sha256 `e79c699c7e949dccd0a4a125e17b8d1261e311b16979dbc8eb34542f3966d82e`) with `libevent-dev libncurses-dev bison pkg-config` in a few seconds.
- **`Xvfb -displayfd`** picks a free display number and reports it, so the script can't collide with a desktop's `:0`.
- **GPUI logs `Found no xinput mouse pointers`** under Xvfb. It's harmless here, because the suite sends no real input.
- **Full suite**, with openbox and `LANG=C.UTF-8`, `BRINDLE_E2E_SLOW=1`: 98 of 100 cases pass on tmux 3.6a and 99 on 3.8, in about 730 s each, mostly the cases' own waits.
  - `colors-palette-tty-client` failed on both, because tmux won't attach its terminal client under `script` without `TERM`, and the container didn't set it (a runner may not either). With `TERM=xterm-256color` it passes 3 of 3. The case should set `TERM` for that child itself.
  - `colors-styled-pane` failed once on 3.6a: the background reply hadn't arrived by its fixed 3-second timeline. It passed 6 of 6 when run alone. It's a load-sensitive timing case, which `BRINDLE_E2E_SLOW=2` is for.
- **`BRINDLE_E2E_SLOW=2` didn't stretch everything.** With the `TERM` fix, the full suite at 2 passed 97 of 100 on both tmux versions (about 1425 s each). `session-end-new-tab`, `session-end-ask` and `search-input-selection` failed every time and pass at 1. The factor stretched the harness's tmux timeline, captures and dump time, but Brindle spaced its `--send`/`--action` steps a fixed second apart (after a fixed half second), so in cases that interleave the two, steps ran too early relative to tmux events: `session-end-new-tab` sent its probe before the session was killed.
- **With `--step-interval`**, the full suite at `BRINDLE_E2E_SLOW=2` passes 100 of 100 on both tmux 3.6a and 3.8, in about 1450 s each.
- **On runners**, the first runs turned up two more waits that didn't stretch, each failing one case once on tmux 3.6a. `search-input-selection` captured a window that hadn't presented its first frame (all zeros), so `capture::grab` retries an all-black image for up to 3 s. `colors-styled-pane` missed a reply by the query script's fixed 1 s `read -t`, so the query scripts' timeouts scale with `BRINDLE_E2E_SLOW`. After both fixes, three runs in a row passed every job. A warm-cache run takes about 1 minute for `test`, 1.5 for `release` and 25 for each e2e entry, so `timeout-minutes: 60` and `BRINDLE_E2E_SLOW=2` stay.

## Goals / Non-Goals

**Goals:**
- Every push gets the unit tests, the harness self-tests, a release build, and the e2e suite on two tmux versions.
- A red unit-test or release job means something is broken. A red e2e job is visible but advisory until the suite has a track record on runners.
- Failed e2e runs leave enough behind (dumps, logs, captures) to debug without re-running.
- The way CI provides a display is a script that also works on a developer machine, so a CI failure can be rerun headless locally.

**Non-Goals:**
- Containers. Runner packages are installed per run, and the explicit package list is the record of what CI depends on.
- The performance suite: shared runners and software rendering give meaningless numbers, and it fetches vtebench from the network.
- `cargo fmt --check` and `cargo clippy`: the tree isn't known to be clean, so they'd be their own change.
- macOS and Windows runners (`port-to-macos`, `port-to-windows` add their own).
- Real-input manual checks (IME, wheel, tab dragging).
- Branch protection and required checks: a repository setting, not something the workflow file controls.
- Fixing the locale bug the spike found (a follow-up change).

## Decisions

### Workflow shape: three kinds of jobs, on the runner

`.github/workflows/ci.yml` has:

1. **`test`**: `cargo test --locked`, then `BRINDLE_E2E=1 cargo test --locked --test e2e -- --self-test` and `BRINDLE_PERF=1 cargo test --locked --test perf -- --self-test`. The plain `cargo test` already compiles the e2e and perf targets, so the self-tests add no build time.
2. **`release`**: `cargo build --release --locked`, which only checks that the release profile (thin LTO, strip) builds.
3. **`e2e`**: a matrix over tmux versions (`3.6a`, `3.8`) running `scripts/with-xvfb cargo test --locked --test e2e`.

The jobs run in parallel and none depends on another.

- *Alternative: one job running everything in sequence.* That shares one build, but then e2e failures either fail the unit-test signal or have to be hidden with `continue-on-error`.
- *Alternative: containers* (explored and dropped). They make the environment pullable, but at the cost of an image pipeline. With the build outside, they add little: the runner and the desktop are the same Ubuntu release.

### Packages, installed per run

Every job runs `apt-get install --no-install-recommends` with the packages it needs, even ones the runner image has today, so a runner image change can't remove one silently:

- **Build (all jobs):** `build-essential pkg-config libxkbcommon-dev libxkbcommon-x11-dev libxcb1-dev libfreetype-dev`.
- **Display (`e2e`):** `xvfb x11-utils openbox mesa-vulkan-drivers libvulkan1 fonts-dejavu-core dbus`.
- **Tests (`e2e`):** `tmux less`. On the 3.8 entry, `libevent-dev libncurses-dev bison` too, in case the tmux build isn't cached.

This list is the one the spike proved sufficient on a bare `ubuntu:26.04`.

### `scripts/with-xvfb`: the display around a command

A POSIX shell script that:

1. Starts `Xvfb -displayfd <fd> -screen 0 1920x1080x24 -nolisten tcp` and reads the display number it picked, failing with a clear message if none arrives within 10 s.
2. Starts `openbox` on it and waits for `_NET_SUPPORTING_WM_CHECK` on the root window. The spike showed the palette and multi-window cases need a window manager.
3. Exports:
   - `DISPLAY` and `BRINDLE_E2E_DISPLAY` set to that display, with `WAYLAND_DISPLAY` unset;
   - `VK_DRIVER_FILES` set to lavapipe's ICD, so a machine with a GPU still renders the way CI does;
   - `LC_ALL=C.UTF-8`, so tmux keeps its separators (see the spike results), and runs match whatever the caller's locale is.

   It leaves `TERM` alone. The one case that needs a terminal type sets it for its own child (see Tasks).
4. Runs the command under `dbus-run-session`, so GPUI and zbus see a private bus. Nothing reaches the desktop's notification daemon.
5. Exits with the command's status. A trap kills openbox and Xvfb however it ends.

On a developer machine with `xvfb` and `openbox` installed, `scripts/with-xvfb cargo test --test e2e` runs the suite headless: no windows on the desktop and no focus stealing. That's the local rerun of a CI failure, and it also makes e2e runs safe while someone is using the desktop.

- *Alternative: the steps inline in `ci.yml`.* Then CI's setup can't be run locally, and it drifts from what people run.
- *Alternative: `xvfb-run`.* It doesn't start a window manager, force lavapipe or set up a session bus, so a wrapper would still be needed around it.

### Triggers: push to any branch, fork PRs, and by hand

`on: push` (all branches), `pull_request`, and `workflow_dispatch`. Pull-request runs are skipped (job-level `if`) unless the PR comes from a fork, because a same-repo branch already ran on push. A `concurrency` group per ref cancels superseded runs.

- *Alternative: `push` to `main` plus `pull_request`.* Changes here don't go through PRs, so branches would be checked only after they were merged.

### `--locked` everywhere, Rust pinned

Every cargo command passes `--locked`, so CI fails instead of quietly updating `Cargo.lock`. `dtolnay/rust-toolchain` installs `RUST_VERSION` (a workflow-level `env`, 1.99.0, the desktop's). The runner's own 1.98.1 lags the desktop, and a pin means a new stable release can't change CI under a commit.

- *Alternative: a `rust-toolchain.toml`.* That would also pin the desktop's builds and `scripts/install.sh`, which is a bigger decision than CI needs.

### "Report only" means not required, not `continue-on-error`

The e2e jobs fail red like any other job. They're advisory because they aren't listed as required checks, which is a repository setting. `continue-on-error: true` would turn a failed job green and hide exactly the failure we want to see. Making e2e required later is a settings change, not a workflow change.

### tmux: apt for 3.6a, a cached build for 3.8

- **3.6a:** Ubuntu's package, the desktop's exact version. Its `tmux -V` says `tmux 3.6`.
- **3.8:** downloaded from `https://github.com/tmux/tmux/releases/download/3.8/tmux-3.8.tar.gz`, checked against the sha256 above (pinned in the workflow, since tmux publishes no checksums), and built with `./configure --prefix=$HOME/tmux-3.8 && make install`. `actions/cache` keyed on the version and runner OS keeps the install, so the build only runs on a cache miss. Its `bin` goes first on `PATH` through `GITHUB_PATH`.

Each entry has a step that fails unless `tmux -V` prints the expected version (`tmux 3.6` or `tmux 3.8`), so the log always says which tmux ran. Moving a pin is a matrix and checksum edit.

- *Alternative: build 3.6b too.* The apt package is what the desktop runs, which is what matters.

### Caching Cargo

`Swatinem/rust-cache` caches `~/.cargo` and dependency builds in `target/`. The `test` and `e2e` jobs share a key (`shared-key: dev`), since both build the dev profile with the same features; only the first job to finish saves it. `release` has its own key. The workflow pins actions by major version.

### Runner and timeouts

Brindle gets a debug option, `--step-interval <S>`, that spaces its steps S seconds apart (and waits S/2 before the first, as it waits half a second now). The harness passes `--step-interval <BRINDLE_E2E_SLOW>` to every launch when the factor isn't 1, so the whole timeline stretches together. At 1 it passes nothing, so `BRINDLE_E2E_BIN` can still test a build from before the option.

- *Alternative: run CI at `BRINDLE_E2E_SLOW=1`.* It passes, but leaves no margin for slower runners, and the factor would stay broken for anyone who reaches for it locally.
- *Alternative: an environment variable instead of an option.* The other hooks are options and the spec lists them together; an option also shows up in `--help`.

`runs-on: ubuntu-26.04`, pinned rather than `ubuntu-latest`. The e2e jobs set `BRINDLE_E2E_SLOW=2`, because lavapipe paints slower than a GPU and runners are slower than the desktop, and `timeout-minutes: 60`. The spike's suite took about 12 minutes at `BRINDLE_E2E_SLOW=1`, mostly waiting, so about 25 at 2, plus the build on a cache miss. Lower `BRINDLE_E2E_SLOW` once the timing cases have a clean record.

### Failure artifacts

An `if: failure()` step in each e2e job uploads `target/tmp/e2e/` as `e2e-tmux-<v>`, kept 14 days. The directory has each case's config, dumps, `.log`s, `.ppm` captures and tmux transcripts. To rerun a case locally the same way: `scripts/with-xvfb cargo test --test e2e -- <case>`, with the matching tmux first on `PATH`.

## Risks / Trade-offs

- [The runner image changes under CI] → Every package is installed explicitly, Rust is pinned, and tmux 3.8 is built from a pinned tarball. The remaining drift is Ubuntu point releases of the same packages the desktop gets.
- [Timing-sensitive cases flake on slow shared runners] → `colors-styled-pane` already flaked once in the spike. `BRINDLE_E2E_SLOW=2` from the start. e2e stays advisory until it has a clean record. Failed runs keep their artifacts. A case that keeps flaking gets fixed to wait for state instead of a fixed time.
- [Two dev builds (`test`, each `e2e` entry) and a release build use runner time] → Free on a public repository. With a warm cache, only Brindle itself is rebuilt, and the shared key keeps one copy of the dev dependencies.
- [The cache outgrows the repository's 10 GB limit] → rust-cache drops workspace crates and stale artifacts before saving. There are two keys plus a small tmux install.
- [`LC_ALL=C.UTF-8` hides the locale bug in CI] → The bug is recorded as a follow-up change. Once Brindle passes `-u`, the variable stops mattering for tmux.
- [Pixel checks depend on the font] → `fonts-dejavu-core` is installed explicitly. It's in the default font list, and DejaVu Sans covers the UI font.

## Migration Plan

Additive. To roll back, delete `.github/workflows/ci.yml`. `scripts/with-xvfb` stays useful locally.

## Open Questions

- When to make e2e a required check: after a run of clean results, as a repository setting.
