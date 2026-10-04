# Proposal

## Why

The repository has no CI. Unit tests, release builds and the end-to-end suite only run when someone remembers to run them locally, and the end-to-end suite needs a display, so it runs least often. The tmux 3.6 crash found by the suite shows the value of running it routinely.

## What Changes

- Add a GitHub Actions workflow in `.github/workflows/`:
  - **On every push and pull request:** `cargo build --release` and `cargo test`, on Ubuntu with the build dependencies listed in the README.
  - **End-to-end suite** (`BRINDLE_E2E=1 cargo test --test e2e`): runs under a virtual X server (Xvfb, or Xwayland on a headless compositor) with software Vulkan (Mesa lavapipe), plus `tmux`, `bash` and `less`. It uploads `target/tmp/e2e/` as an artifact when it fails.
- Pin the tmux version used in CI, or test a small matrix (the distro's 3.x and the latest release), so the `tmux-*` behaviour cases flag tmux changes.
- Cache Cargo's registry and target directory; GPUI builds are slow.

## Capabilities

### New Capabilities

None. This is build tooling.

### Modified Capabilities

None (`skip_specs: true`).

## Impact

- **New:** `.github/workflows/ci.yml`.
- **Possibly:** `tests/e2e/` needs adjustments for a headless display, such as fonts. Pixel checks assume the default font list resolves to an installed monospace font, so CI must install one.
- **Open questions for design:**
  - Does GPUI's Blade renderer run on lavapipe under Xvfb?
  - How long does the suite take on a CI runner?
  - Should the end-to-end job block merges, or report only?
