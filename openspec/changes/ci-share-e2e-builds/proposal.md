# Proposal

## Why

Every CI run compiles the same e2e build five times on Linux and twice on Windows.
- **Linux:** each of the 3 shards builds the debug app, `brindle-remote`, the static musl fake ACP agent and the e2e test binary. That took 9.1, 9.6 and 9.8 minutes on main's run for 725ed28, and 9.7, 8.9 and 8.1 on PR #133's last run. `e2e-archive` compiles the suite again: 8.0 and 11.3 minutes.
- **Windows:** `windows-e2e` and `windows-e2e-archive` each install Rust, restore a cache of about 50 s and compile the suite.

Building once and reusing the output saves about 27 runner-minutes a run, net, roughly 9% of a run's 290–307. Nearly all of it is on Linux. The user asked for this.

## What Changes

- **New job `e2e-build` (Linux).**
  - It builds what the shards build today: `cargo build -p brindle-remote-agent`, the static musl fake agent, and `cargo test -p brindle --test e2e --no-run`.
  - It packs the binaries and the compiled e2e test binary into a tarball, with debug info stripped and a stamp naming the commit and the workspace path.
  - It uploads the tarball as the artifact `e2e-build-linux`.
- **New job `windows-e2e-build` (Windows).** It does the same for the Windows cases: `fetch-conpty.ps1`, the debug helper, fake agent and suite, and the ConPTY files beside them. It uploads `e2e-build-windows`.
- **The shards, `e2e-archive`, `windows-e2e` and `windows-e2e-archive` no longer compile.** Each checks out the repository, downloads and unpacks its platform's artifact into `target/`, and runs the suite binary directly instead of through `cargo test`. They install no Rust toolchain and restore no cargo cache.
  - `windows-e2e-archive` runs the bundle's debug fake ACP agent instead of building a release one.
- **A new script, `scripts/e2e-bundle`, packs and unpacks for both platforms** (`pack`, and `unpack` with the stamp check). Git Bash runs it on Windows.
- **The harness doesn't change.** A local `BRINDLE_E2E=1 cargo test --test e2e` builds and runs exactly as now, and its helper and fake-agent freshness checks keep their rules.
- CLAUDE.md's CI paragraph and the Windows note about what `windows-e2e` builds, README.md's CI paragraph and `docs/architecture/testing-architecture.md`'s CI line describe the build jobs and the artifacts.

## Capabilities

### New Capabilities

None. This is CI plumbing.

### Modified Capabilities

None (`skip_specs: true`). The jobs still run every case they ran before, against the same kinds of binary. `release-process` says the end-to-end jobs MAY be skipped on release pull requests. The build jobs carry the same `release-please--` condition, and the jobs that need them skip with them.

## Impact

- `.github/workflows/ci.yml`: two new jobs, four changed. `build-linux.yml`, `build-windows.yml` and `release.yml` don't change, so releases are unaffected.
- A new `scripts/e2e-bundle`, added to `scripts/lint-workflows`' shellcheck list.
- CLAUDE.md, README.md, `docs/architecture/testing-architecture.md`.
- Artifact storage: about 20 MB per run on Linux (measured locally, stripped and gzipped), plus the Windows bundle. Each is kept 1 day.
- Wall time: the slowest job, `e2e-archive`, should finish about 4 minutes sooner. The Linux shards and `windows-e2e` should finish 1–3 minutes later, because they now wait for a build job instead of building in place (design.md, "Estimate").
