# Tasks

## 1. The bundle script

- [x] 1.1 Add `scripts/e2e-bundle` (POSIX sh) with two subcommands.
  - `pack --suite PATH OUT` writes design.md's file list for `RUNNER_OS` (Linux or Windows; anything else is a usage error, exit 2). On Linux it strips each binary through `objcopy --strip-debug`, puts the suite at `target/debug/brindle-e2e-suite[.exe]`, and writes the stamp `target/e2e-bundle.txt` (`commit=`, `workspace=`, `os=`).
  - `unpack FILE` checks the stamp against `git rev-parse HEAD`, `GITHUB_WORKSPACE` and `RUNNER_OS`, refusing with both values named on a mismatch. It then extracts with `tar -xzmf` into the workspace root and prints the stamp and the suite path.

  Add it to `scripts/lint-workflows`' shellcheck list. Verify, locally with no window:
  - `scripts/lint-workflows` passes;
  - after `cargo build -p brindle-remote-agent`, the musl fake agent build and `cargo test -p brindle --test e2e --no-run`, run `RUNNER_OS=Linux GITHUB_WORKSPACE=$PWD scripts/e2e-bundle pack --suite <exe> target/tmp/b.tar.gz`. The tarball lists exactly the design's Linux files and is about 20 MB;
  - `unpack` then succeeds, and leaves every listed file newer than `remote-agent/src/fake_acp/script.rs`;
  - `unpack` with `GITHUB_WORKSPACE=/elsewhere` refuses naming both paths, and so does a stamp edited to another commit;
  - `BRINDLE_E2E=1 target/debug/brindle-e2e-suite --self-test` and `--list` match `cargo test --test e2e -- --self-test`/`--list`.
- [x] 1.2 Run a few display cases from the unpacked bundle headless: `BRINDLE_E2E=1 scripts/with-xvfb target/debug/brindle-e2e-suite agent-acp agent-cancel local-server-close-zone` (run through `systemd-run --user --scope`, as the memory note on e2e scopes says). Verify: they pass with no helper or fake agent freshness message, as they do under `cargo test --test e2e`.

## 2. Linux jobs

- [x] 2.1 Add `e2e-build` to `ci.yml`:
  - the shards' current packages, toolchain, musl target, `rust-cache` (`shared-key: dev`) and build commands;
  - the suite path from `--message-format=json` through `jq`;
  - `scripts/e2e-bundle pack`;
  - `upload-artifact@v7` with `name: e2e-build-linux`, `compression-level: 0`, `retention-days: 1`, `overwrite: true`;
  - the `release-please--` condition and `timeout-minutes: 30`.

  Comment each choice with its README citation, as design.md gives it. Verify: `scripts/lint-workflows` passes.
- [x] 2.2 Change the `e2e` matrix and `e2e-archive` to need `e2e-build` (`e2e-archive`: `[release, e2e-build]`):
  - drop their Rust install, `rust-cache`, `musl-tools` and build steps;
  - add `download-artifact@v7` (`name: e2e-build-linux`, `path: ${{ runner.temp }}/e2e-build`) and `scripts/e2e-bundle unpack` after checkout;
  - run `scripts/with-xvfb target/debug/brindle-e2e-suite …` with the same arguments and `env:` as design.md's table.

  Keep the failure uploads unchanged. Verify: `scripts/lint-workflows` passes, and `git diff` shows every `env:` entry the jobs had.
- [x] 2.3 Update CLAUDE.md's CI paragraph and its re-run note, as design.md's "CLAUDE.md afterwards" lists for Linux. Also update README.md's CI paragraph and `docs/architecture/testing-architecture.md`'s CI line (the shards run one shared build). Verify: `rg -n 'cargo test --locked -p brindle --test e2e' .github CLAUDE.md` finds only the build jobs.

## 3. Windows jobs

- [x] 3.1 Add `windows-e2e-build`:
  - `windows-latest`;
  - Rust, `rust-cache` (`shared-key: windows`), `fetch-conpty.ps1`, `cargo build --locked -p brindle-remote-agent` and `cargo test --locked -p brindle --test e2e --no-run --message-format=json`, with the path through `ConvertFrom-Json` in `pwsh`;
  - `scripts/e2e-bundle pack` under `shell: bash`;
  - upload `e2e-build-windows` with the Linux job's settings.

  Verify: `scripts/lint-workflows` passes.
- [x] 3.2 Change `windows-e2e` (needs `windows-e2e-build`) and `windows-e2e-archive` (needs `[windows, windows-e2e-build]`):
  - drop Rust, `rust-cache`, `fetch-conpty.ps1` and the build steps;
  - download and `unpack` (`shell: bash`);
  - run `target/debug/brindle-e2e-suite.exe …` as design.md's table says;
  - point `windows-e2e-archive`'s `BRINDLE_FAKE_ACP_AGENT` at the bundle's debug agent.

  Update CLAUDE.md's Windows bullet ("`windows-e2e-build` on CI builds them") and the CI paragraph's Windows jobs. Verify: `scripts/lint-workflows` passes.

## 4. Verify on GitHub

- [ ] 4.1 Push the branch and open the PR. Verify, from the run:
  - `e2e-build` and `windows-e2e-build` pass and upload `e2e-build-linux` (about 20 MB) and `e2e-build-windows`, with their sizes recorded;
  - every consumer's log shows the stamp line and no `Compiling`;
  - the shard logs' `e2e: shard K/3, M of T cases` counts add up to the `--list` total;
  - every e2e job passes, or fails only as on `main`.

  Record the jobs' durations against design.md's estimate: removed runner-minutes, the run's wall time, and the shards' and `windows-e2e`'s finish times.
- [ ] 4.2 On a scratch branch, make one case fail. Verify: only its shard uploads `e2e-shard-<k>`, and "Re-run failed jobs" re-runs that shard alone, from the same artifact, with the stamp check passing. Delete the branch.
- [ ] 4.3 If 4.1 shows the Windows half saving under a minute of runner time and no wall time, record that in the PR, and ask the user whether to keep it or revert group 3 before merging.

## Workflow follow-up

- PR title: `ci(e2e): build the e2e suite once per platform and share it with every e2e job`. The PR body says that `build-linux.yml`, `build-windows.yml` and `release.yml` don't change, so the next release has nothing new to check.
- Archive the change in the implementation PR.
