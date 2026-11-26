# Design

## Context

- **What each e2e job compiles today** (`.github/workflows/ci.yml` at 725ed28):

  | Job | Compiles | Needs |
  |---|---|---|
  | `e2e` (3 shards) | `cargo build -p brindle-remote-agent` (debug `brindle-remote`, debug fake agent); the static musl fake agent (`--release --target x86_64-unknown-linux-musl --bin brindle-fake-acp-agent`); `cargo test -p brindle --test e2e --no-run` (debug `brindle` and the suite) | none |
  | `e2e-archive` | the suite (and with it the debug `brindle`), and the static musl fake agent | `release` |
  | `windows-e2e` | debug `brindle-remote.exe` and fake agent, the suite and debug `brindle.exe`, after `fetch-conpty.ps1` | none |
  | `windows-e2e-archive` | the suite, and a release fake agent | `windows` |

  Each then runs `cargo test --locked -p brindle --test e2e -- …`. That is a no-op compile followed by running `target/<profile>/deps/e2e-<hash>`.
- **Compiled-in paths.** The harness finds its files through compile-time `env!` paths:
  - `CARGO_BIN_EXE_brindle` (`tests/e2e/main.rs:183,192,200`);
  - `CARGO_MANIFEST_DIR`, for the helper's sources (`tests/common/server.rs:204`), the fake agent's (`tests/common/fake_agent.rs:112`), `STATIC_LINUX` (`tests/e2e/cases/remote.rs:476`) and `tests/fixtures/remote` (`remote.rs:1965`);
  - `CARGO_TARGET_TMPDIR`, for `target/tmp/e2e` (`main.rs:185`).

  None is read at run time, and the harness never runs `cargo`. On GitHub-hosted runners every job of a run checks out to the same `github.workspace` (`/home/runner/work/brindle/brindle` on Linux).
- **Freshness checks.**
  - `check_helper` with `Freshness::Sources` fails when a file under `remote-agent/src`, `remote-protocol/src`, `term-core/src` or `transcript/src` is newer than the helper.
  - `fake_agent::check` fails when a file under `remote-agent/src/fake_acp` is newer than the agent. The variable-named agents (`BRINDLE_FAKE_ACP_AGENT`, `STATIC_LINUX`) are checked the same way.
  - `actions/checkout` writes every file at checkout time, so in a consumer job the sources are newer than anything built in another job, unless the unpacked files' mtimes are later still.
- **Artifact actions.** The repository pins `actions/upload-artifact@v7` and `actions/download-artifact@v7`. Their READMEs say:
  - "File permissions are not maintained during zipped artifact upload". After a download, "the file is no longer guaranteed to be set as an executable". Both recommend tarring files to keep permissions.
  - Neither says whether modification times survive.
  - Within a run no token or permission is needed: "By default, the permissions are scoped so they can only download Artifacts within the current workflow run". `github-token` with `actions: read` is "required when downloading artifacts from a different repository or from a different workflow run".
  - `retention-days` is "Minimum 1 day", up to 90.
  - `compression-level` defaults to 6, and 0 is "recommended for significantly faster uploads" of data that doesn't compress.
  - Artifact names must be unique within a run, and `overwrite: true` deletes "an artifact with a matching name … before a new one is uploaded".
  - `archive: false` (upload v7.0.0) uploads one file unzipped, but the download README says that "only supports files uploaded with `actions/upload-artifact@v7` and `archive: false` set" with `download-artifact@v8`, and the repository is on v7.
- **`needs`.** It waits for the named jobs to "complete successfully", and "If a job fails or is skipped, all jobs that need it are skipped" (workflow syntax docs, `jobs.<job_id>.needs`).
- **`shell: bash` on Windows** is "the bash shell included with Git for Windows" (workflow syntax docs, `defaults.run.shell`).
- **Sizes measured locally** (Linux debug build at 725ed28):

  | Binary | As built | Debug info stripped |
  |---|---|---|
  | `target/debug/brindle` | 773 MB | 62.5 MB |
  | `brindle-remote` | 94 MB | 12.4 MB |
  | debug fake agent | 12 MB | 1.0 MB |
  | the suite | 55 MB | 5.7 MB |

  Stripped and gzipped (the static fake agent included), the whole bundle is about 20.5 MB. Unstripped, `brindle` and `brindle-remote` alone are 193 MB gzipped. Windows keeps debug info in `.pdb` files beside the exes. They aren't packed, and the exes' sizes weren't measured here.

## Goals / Non-Goals

**Goals:**
- Compile the e2e build once per platform per run, and run every e2e job from that build.
- Keep every case, filter, shard and environment variable each job uses today.
- Keep a local `cargo test --test e2e` and the harness's checks exactly as they are.

**Non-Goals:**
- Sharing the build with the `test` or `clippy` jobs, or with `build-linux.yml`/`build-windows.yml`. Those are release builds, and releases call them too.
- Changing the shard count or the cases.
- Moving to `download-artifact@v8` or `archive: false`.

## Decisions

### One build job per platform, which the e2e jobs need

`e2e-build` (`ubuntu-26.04`) and `windows-e2e-build` (`windows-latest`) run the existing build commands unchanged, with the same packages, toolchain, musl target, `fetch-conpty.ps1` and `Swatinem/rust-cache` keys (`dev`, `windows`).

| Consumer | `needs` |
|---|---|
| `e2e` (matrix) | `e2e-build` |
| `e2e-archive` | `[release, e2e-build]` |
| `windows-e2e` | `windows-e2e-build` |
| `windows-e2e-archive` | `[windows, windows-e2e-build]` |

- **Release PRs.** Every job keeps `if: ${{ !startsWith(github.head_ref, 'release-please--') }}`. On a release PR the build jobs skip, and by `needs` the e2e jobs skip with them, as `release-process` allows.
- **Build failures.** A failed build skips its e2e jobs. That is red, as a failed shard build is today, but it reports once instead of three times.

*Alternative: take the build from the `test` job.* `cargo test --workspace` already compiles the debug `brindle`, helper, fake agent and suite. But `needs` waits for the whole job, and `test` runs 12–15 minutes. That is 3–5 minutes later than a dedicated build, on the shards' wall time. It also ties e2e to unit-test failures, and the musl fake agent would still need building somewhere. Rejected.

*Alternative: take the Windows build from `windows / build`.* It also runs `cargo test --workspace`, but it is a reusable workflow that releases call, and it starts after `release` and ends about 28 minutes into the run. `windows-e2e` would become the run's slowest job. Rejected.

*Alternative: a compiler cache (sccache) per shard.* It still pays linking and the cache round trip in every shard, and it adds a service. Rejected.

### One artifact per platform: a tarball inside the zip

`scripts/e2e-bundle pack` writes `e2e-build-<os>.tar.gz`, holding paths relative to the workspace root:
- **Linux:** `target/debug/brindle`, `target/debug/brindle-remote`, `target/debug/brindle-fake-acp-agent`, `target/x86_64-unknown-linux-musl/release/brindle-fake-acp-agent`, `target/debug/brindle-e2e-suite` (the suite binary under a fixed name), and `target/e2e-bundle.txt` (the stamp).
- **Windows:** `target/debug/brindle.exe`, `brindle-remote.exe`, `conpty.dll` and `OpenConsole.exe` (which `remote-agent/build.rs` copies beside the exe), `brindle-fake-acp-agent.exe`, `brindle-e2e-suite.exe`, and the stamp.

The upload step uses:
- `upload-artifact@v7` with the artifact name `e2e-build-linux` or `e2e-build-windows`;
- `compression-level: 0`, since the tarball is already gzipped;
- `retention-days: 1`;
- `overwrite: true`, so a re-run of the build job replaces its artifact.

Why the tarball:
- The tar keeps the executable bit, which the zip loses, as both READMEs warn.
- It puts every file back at the path the suite was compiled to expect.
- It leaves mtimes to the unpack step (next decision), rather than to an action that doesn't document them.

A single artifact serves both of a platform's consumers. The archive jobs need only the suite and the fake agents, but at 20 MB the split isn't worth a second name.

The suite's path comes from `cargo test --no-run --message-format=json`: the `executable` of the message whose target is `e2e` (`jq` on Linux, `ConvertFrom-Json` on Windows). It is passed to `pack` as `--suite PATH`.

*Alternative: `archive: false`.* It would need `download-artifact@v8` and a new major version's checks. It uploads one file under its own name with `name` ignored, and the outer zip is cheap at compression 0. Rejected for now.

### Strip debug info on Linux

`pack` copies each Linux binary into the tarball through `objcopy --strip-debug`. The symbol table stays, so a backtrace still names functions. Panic messages keep their `file:line`, which comes from `Location`, not DWARF. Only a backtrace's line numbers are lost, and CI sets no `RUST_BACKTRACE`. This cuts the bundle from about 200 MB to 20 MB gzipped, and with it the upload and download time and the storage. On Windows nothing is stripped, and no `.pdb` is packed.

*Alternative: build with `CARGO_PROFILE_DEV_DEBUG=line-tables-only`.* It changes the build that `test` and `clippy` share through the `dev` cache key, and rebuilds what the cache holds. Rejected.

### Freshness: unpack with new mtimes, guarded by the stamp

`scripts/e2e-bundle unpack FILE` runs after `actions/checkout`. It:
1. reads the stamp: `commit=<git rev-parse HEAD at pack>`, `workspace=<GITHUB_WORKSPACE at pack>`, `os=<RUNNER_OS>`;
2. refuses, with a message naming both values, when the commit isn't this checkout's `HEAD`, the workspace isn't this job's `GITHUB_WORKSPACE`, or the OS differs;
3. extracts with `tar -xzmf` (`-m`/`--touch`: each file gets the extraction time), so every unpacked file is newer than the checkout's sources;
4. prints the suite's path and the stamp.

The harness's checks then pass, for the right reason. A test target doesn't rebuild the helper or the fake agent, so the checks exist to catch a binary built from other sources. In CI the stamp proves the bundle was built from this commit in this run, which is the same guarantee, checked before the run instead of by mtime. Re-runs use "the same `GITHUB_SHA`" (re-running docs), so the stamp also holds on a re-run.

The workspace check guards the compiled-in paths. If a consumer's runner ever checked out elsewhere, the suite would look for `brindle` and the sources at the build's path. The run then stops with that reason instead of failing in a case.

*Alternative: keep the archive's mtimes and touch the sources back.* That would hide a real stale binary. Rejected.

*Alternative: a harness variable that skips the freshness checks.* It changes the harness for CI's sake, and a skip is easy to leave set locally. The stamp check lives in the script that CI alone runs. Rejected.

### Consumers run the suite binary directly

Each consumer:
1. checks out;
2. installs its packages (as now, without `musl-tools`);
3. downloads `e2e-build-<os>` to `${{ runner.temp }}/e2e-build`;
4. runs `scripts/e2e-bundle unpack`;
5. runs the suite as `target/debug/brindle-e2e-suite ARGS`, with the same arguments `cargo test … --` passed and the same `env:`.

It has no `rustup`, no `rust-cache` and no `cargo`. The steps run with the workspace root as the working directory, as `cargo test` runs an integration test from its package root (here the workspace root).

| Job | Command after unpacking |
|---|---|
| `e2e` | `scripts/with-xvfb target/debug/brindle-e2e-suite --shard K/3` |
| `e2e-archive` | `scripts/with-xvfb target/debug/brindle-e2e-suite local-server remote- agent-`, with `BRINDLE_E2E_BIN` and `BRINDLE_FAKE_ACP_AGENT` as now (the static agent's path is unchanged) |
| `windows-e2e` | `target/debug/brindle-e2e-suite.exe` |
| `windows-e2e-archive` | `target/debug/brindle-e2e-suite.exe windows-conpty windows-pwsh-integration windows-agent-` |

- **`windows-e2e-archive`'s fake agent.** It uses the bundle's debug agent instead of a release one: `BRINDLE_FAKE_ACP_AGENT: ${{ github.workspace }}/target/debug/brindle-fake-acp-agent.exe`. The agent only speaks ACP from a script, and its profile doesn't affect what the archive cases check, which is the zip's `brindle.exe` and helper. A release build was chosen only because that job compiled one anyway.
- **What the archive jobs test is unchanged.** `BRINDLE_E2E_BIN` names the installed or unpacked release. `check_helper` takes the release's helper (`SameRelease`), and the debug `brindle` in the bundle is unused there.

### Scope of the script, and lint

`scripts/e2e-bundle` is POSIX sh. It runs under bash on both platforms, through Git Bash on Windows (`shell: bash`).
- It uses `tar`, `gzip`, `git` and, on Linux, `objcopy`. Each is present on both runner images through Git for Windows, or through the package list on Linux.
- It is added to `scripts/lint-workflows`' shellcheck list.
- `${{ }}` values reach it through `env:`, as `.claude/rules/workflows.md` requires.
- The new jobs need no `permissions:` beyond the workflow's `contents: read`, since same-run artifacts need none.

### Architecture

This is CI plumbing outside the crates. No layer in `docs/architecture/README.md` changes, and no wire message, config key or state file changes. The `testing` rules it keeps:
- **R3:** the suite stays a `harness = false` target with its own runner, `--shard K/N` and the shards across jobs. The anti-pattern "several harness processes in one CI job" isn't touched.
- **R4:** each case's isolation is the harness's and is unchanged.
- **R9:** no retry is added. A refused bundle fails the job.
- **R18:** the require variables (`BRINDLE_E2E_SWITCHES_REQUIRED`, `BRINDLE_REMOTE_E2E_REQUIRED`, `BRINDLE_E2E_CLIPBOARD_REQUIRED`) stay on the same jobs. The fault switches are still asked of the binaries, which are the same debug builds.

### Estimate

Measured from main's run for 725ed28 (45736436275) and PR #133's last run (45728494200). The two runs used 307 and 290 runner-minutes, 66 minutes wall each.

| Removed per run | main | PR #133 |
|---|---|---|
| Shards' build step (×3) | 28.5 | 26.7 |
| Shards' Rust install and cache restore/save (×3) | ~1.5 | ~1.0 |
| `e2e-archive` build and setup | 8.4 | 11.7 |
| `windows-e2e` build, Rust and cache | 6.6 | 2.1 |
| `windows-e2e-archive` build, Rust and cache | 2.9 | 2.8 |
| **Total removed** | **~47.9** | **~44.3** |

Added per run:
- `e2e-build`: about 11 minutes (the build step plus about 1 minute of setup, packing and upload);
- `windows-e2e-build`: about 3.5–8 minutes, following `windows-e2e`'s own build;
- about 0.3 minutes of download and unpack in each of the 6 consumers, about 2 minutes in all.

**Net: about 25–30 runner-minutes saved per run, roughly 9–10%.** Almost all of it is Linux, about 26 minutes. Windows nets about 1 minute. The new job costs nearly what it removes, because a Windows cache restore and its save take about as long as the warm build. What Windows gains is the 2–3 minutes cut from `windows-e2e-archive`'s wall time.

Wall time:
- **The run, about 66 → 62 minutes.** `e2e-archive`, the slowest job, now starts at max(`release`, `e2e-build`) ≈ 11 minutes in instead of 4–7, but skips an 8–11-minute build.
- **The shards finish about 1–2 minutes later**, since they wait for the build job instead of building in place.
- **`windows-e2e` finishes about 1–3 minutes later.** Neither is the slowest job.

The repository is public, so standard runners cost nothing (Actions billing docs). The saving is runner time and queue capacity, not money. These are estimates from two runs, and task 3.2 measures the real ones.

### CLAUDE.md afterwards

- **CI paragraph:**
  - Name `e2e-build` and `windows-e2e-build`: what each builds, and that it uploads `e2e-build-linux`/`e2e-build-windows` (a tarball made by `scripts/e2e-bundle`, Linux debug info stripped, kept 1 day).
  - Say that the e2e shards, `e2e-archive`, `windows-e2e` and `windows-e2e-archive` unpack it (`scripts/e2e-bundle unpack`, which checks the commit and workspace stamp and gives the files new mtimes) and run `target/debug/brindle-e2e-suite` directly, with no toolchain.
  - Say that `windows-e2e-archive` now uses the bundle's debug fake agent.
  - Say that a failed shard is still rerun locally with `cargo test --test e2e -- --shard K/3`.
- **The Windows bullet:** "`windows-e2e` on CI builds the helper, and with it the fake ACP agent, first" becomes "`windows-e2e-build` on CI builds them".
- **Re-runs:** a re-run of an e2e job after the artifact's 1-day retention needs "Re-run all jobs", which rebuilds.

### As built (2026-11-25)

- **The tarball goes through gzip on a pipe, not `tar -f`.** `pack` writes `tar -cf - … | gzip > OUT`, and `unpack` reads the stamp with `gzip -dc < FILE | tar -xOf - target/e2e-bundle.txt` and extracts with `gzip -dc < FILE | tar -xmf -`, instead of `tar -xzmf FILE`. GNU tar, which Git Bash runs, takes a path like `D:\a\_temp\…` as `host:file` on a remote host. `-m` still gives every file the extraction time.
- **Messages use `printf`, not `echo`.** A Linux `sh` (dash) `echo` reads a Windows workspace's backslashes as escapes: a local run printed `D:\a\brindle\brindle` as `D:rindlerindle`.
- **The shard number reaches its run step through `env:` (`SHARD`)**, as `.claude/rules/workflows.md` asks, where the old step had `${{ matrix.shard }}` inline.
- **Both uploads set `if-no-files-found: error`**, so a missing tarball fails the build job, not its consumers' download.
- **`windows-e2e-build` checks `$LASTEXITCODE` after each cargo in its `pwsh` step** and requires exactly one e2e executable. For `pwsh`, GitHub appends `if ((Test-Path -LiteralPath variable:\LASTEXITCODE)) { exit $LASTEXITCODE }` (workflow syntax docs, "Exit codes and error action preference"), which sees only the last native command's code.

## Risks / Trade-offs

- [A consumer's runner checks out to another path, e.g. during a runner-image rollout] → `unpack` refuses with both paths named. "Re-run all jobs" rebuilds on matching runners.
- [A re-run of only a failed e2e job after 1 day finds no artifact] → the download fails naming the artifact. Re-run all jobs. Flagged choice: retention could be 3–7 days at about 20 MB a run, if re-running a lone shard days later turns out to be common.
- [The stripped binaries differ from what a local run uses] → they differ only in DWARF. Fault switches and behaviour come from the code, and the dump's output is unaffected.
- [The touched mtimes would hide a stale bundle] → the commit stamp is the guard, and it is checked before anything runs.
- [Windows saves little] → the Windows half is its own task group (3), measured after landing. If it nets nothing, it can be reverted alone without touching the Linux half.
- [Shard wall time grows 1–2 minutes] → accepted for the runner time. The run as a whole gets shorter.

## Migration Plan

One PR. Rollback is reverting the `ci.yml` change. The script and docs have no other users.

## Open Questions

None that block the tasks. The user didn't make these choices; each is this design's recommendation:
- strip Linux debug info;
- 1-day retention;
- one artifact per platform;
- the debug fake agent for `windows-e2e-archive`;
- keep `download-artifact@v7` with a tarball rather than move to v8's `archive: false`;
- include Windows despite its small saving.
