# Proposal

## Why

Two things went wrong in the 2026-11-25 manual checks on Windows, both in `scripts/run-throwaway.ps1`:
- The user edited the config they had passed with `-Config C:\temp\agents-check.toml`, and nothing changed. Brindle reloads `BRINDLE_CONFIG` when its mtime changes (`watch_config`, `src/main.rs`), but the script copies the file into `%TEMP%\brindle-throwaway-…\config.toml` (`run-throwaway.ps1:91`). Brindle watches that copy, so edits to the file the user named are never seen. `run-throwaway.sh` copies the file the same way.
- Remote profiles need the static Linux helper. A Windows source build has none beside `brindle.exe`, so unless `BRINDLE_REMOTE_AGENT` is already set, an SSH or container tab fails with "Linux remote helper not found". Brindle can't run that ELF on Windows to check its version. So a same-version helper built from older sources is uploaded and runs old code without complaint. The user asked the script to build the helper when it is missing or isn't the right version.

## What Changes

- **`-Config FILE` / `--config FILE` uses the file in place.** Both scripts point `BRINDLE_CONFIG` at the file's absolute path, so edits apply live and the settings dialog writes to that file. With no option, the config is still a fresh private `config.toml` in the throwaway directory. The state directory and the terminal server directory stay private either way. **BREAKING** for the scripts only: anyone who relied on the copy to keep the original untouched now copies it themselves.
- **`run-throwaway.ps1` provides the Linux helper.** For a binary built in this checkout's `target\`, with `BRINDLE_REMOTE_AGENT` unset and no helper beside the binary, the script:
  - builds the static helper with README.md's Docker musl build when it is missing, or older than its sources, `remote-agent/Cargo.toml` or `Cargo.lock`;
  - then checks that its `--version`, run through Docker, is the binary's release;
  - sets `BRINDLE_REMOTE_AGENT` for the run and restores it afterwards.

  Without Docker, it warns that remote profiles won't work and runs anyway. `-NoLinuxHelper` skips all of this. The Docker build and the version check move from `scripts/install.ps1` into a shared `scripts/linux-helper.ps1`, which both scripts call.
- **`run-throwaway.sh` checks its helper's freshness.** On Linux, the `brindle-remote` beside the binary is the terminal server's helper and the helper remote profiles deploy. For a binary in this checkout's `target/`, the script stops with "run `cargo build`" when that helper is older than its sources, using the same rule as the e2e harness's `check_helper`. It builds nothing.
- The two scripts' headers, `--help` output, CLAUDE.md's "Manual checks" paragraph, README.md's Windows source-build note and `docs/manual-checks/clipboard.md` describe the new behaviour. `scripts/lint-workflows` adds `run-throwaway.sh` to its shellcheck list.

## Capabilities

### New Capabilities

None. These are developer scripts.

### Modified Capabilities

None (`skip_specs: true`). No spec covers `scripts/run-throwaway.*`, and Brindle's own behaviour doesn't change: the `configuration` spec already says Brindle reads and writes `$BRINDLE_CONFIG`, and `remote-connections` already says Brindle finds the helper through `BRINDLE_REMOTE_AGENT`.

## Impact

- `scripts/run-throwaway.ps1`, `scripts/run-throwaway.sh`, `scripts/install.ps1`, a new `scripts/linux-helper.ps1`, `scripts/lint-workflows`.
- CLAUDE.md ("Manual checks"), README.md (the Windows source-build helper note), `docs/manual-checks/clipboard.md`.
- No Rust code, wire, config key or state file changes.
- The first Windows run after a helper source change takes as long as the Docker build: several minutes cold, less with the shared `brindle-cargo-registry` volume.
