# Tasks

The Windows steps run on a Windows machine with Docker Desktop: the user's, or a Windows session they start. Steps marked "no window" launch nothing. Steps that open Brindle are handed to the user, since a window takes focus (`testing R5`).

## 1. Shared Linux helper script

- [ ] 1.1 Add `scripts/linux-helper.ps1` with `-Root`, `-Bin`, `-Build Auto|Never|Always` and `-Check`.
  - It returns `@{ Path; State; Message }`, with `State` one of `Ready`, `Unchecked`, `Unavailable` or `Failed`.
  - It implements design.md's "right version" rules 1–3. The rule 2 file list (the four source directories, `remote-agent/Cargo.toml`, `Cargo.lock`) carries a comment naming `tests/common/server.rs` `check_helper`.
  - The Docker test, image tag, `target/linux-helper` target directory and `brindle-cargo-registry` volume are taken from `install.ps1` unchanged.
  - Verify (no window), on Windows with Docker:
    - `scripts/linux-helper.ps1 -Root . -Bin target\release\brindle.exe -Check` reports `missing` with no helper built;
    - after `-Build Auto` it reports `Ready`;
    - after touching `remote-protocol\src\lib.rs` it reports stale, naming that file;
    - with Docker Desktop stopped it reports `Unavailable` with `install.ps1`'s message.
- [ ] 1.2 Make `scripts/install.ps1` call `linux-helper.ps1 -Build Always`. It keeps its error when Docker is unusable and `-RemoteAgent` isn't given, and its `-RemoteAgent PATH` (which runs rules 1 and 3 only). Verify (no window): `scripts/install.ps1 -Prefix $env:TEMP\brindle-install-check` installs `brindle-remote-linux-x86_64`, which prints this version; then run `scripts/uninstall.ps1 -Prefix` on the same directory. With Docker stopped, it fails before building with the same message as before.

## 2. `run-throwaway.ps1`

- [ ] 2.1 Use `-Config` in place: resolve it to an absolute path, set `BRINDLE_CONFIG` to it, print `run-throwaway: config <path> (edits apply live; the settings dialog writes it)`, and drop the `Copy-Item`. Without `-Config`, keep `<dir>\config.toml`. Update the header paragraph and the `-Help` line count. Verify, handed to the user: `scripts/run-throwaway.ps1 -Config C:\temp\agents-check.toml`, then change a profile's `theme` in that file; the open tab's colours change within about a second. After exit the file is still there, and `%TEMP%` holds no `brindle-throwaway-*` directory (without `-Keep`).
- [ ] 2.2 Add the Linux helper step from design.md's decision table, through `linux-helper.ps1 -Build Auto`, before the launch:
  - `BRINDLE_REMOTE_AGENT` is set in the `try` and restored in the `finally`;
  - add `-NoLinuxHelper`;
  - update the header.

  Verify, handed to the user, after a helper source change (touch a file under `remote-agent\src`):
  - the run prints that it is building and why, builds, and opens;
  - an SSH or container profile connects;
  - a second run builds nothing.

  Verify (no window), with Docker Desktop stopped and the helper made stale (`-Check`, or the run up to its warning with `-BrindleArgs '--version'`): it warns that remote profiles won't work and leaves `BRINDLE_REMOTE_AGENT` unset.

## 3. `run-throwaway.sh`

- [ ] 3.1 Use `--config` in place, as in 2.1: an absolute path, the printed line, no `cp`, and an updated header and `--help`. Verify (no window): make a stub `target/tmp/throwaway-stub/brindle` that prints `$BRINDLE_CONFIG` and exits, with an executable `brindle-remote` beside it that accepts `stop`. Run `BRINDLE_BIN=target/tmp/throwaway-stub/brindle scripts/run-throwaway.sh --config ./some.toml`. It prints the file's absolute path, and the file survives the cleanup.
- [ ] 3.2 Add the helper freshness check (design.md, "The shell script"): for a binary under `<root>/target/`, stop with `run cargo build` when `brindle-remote` is older than any rule 2 file. Verify (no window) with the stub from 3.1:
  - after touching the stub helper the run proceeds;
  - after touching `term-core/src/lib.rs` it stops, naming that file;
  - a stub outside `target/` (under `$TMPDIR`) is not checked.
- [ ] 3.3 Add `scripts/run-throwaway.sh` to `scripts/lint-workflows`' shellcheck list. Verify: `scripts/lint-workflows` passes.

## 4. Docs

- [ ] 4.1 Update CLAUDE.md's "Manual checks" sentence about the two scripts:
  - the config option uses the file in place, so edits apply live and the dialog writes it;
  - what stays private;
  - the Windows script provides the Linux helper and has `-NoLinuxHelper`;
  - the shell script stops on a stale helper.

  Also update README.md's Windows source-build helper note (`run-throwaway.ps1` and `install.ps1` build it), and `docs/manual-checks/clipboard.md`'s script lines. Verify: `rg -n 'copy of FILE|copies a file in' scripts CLAUDE.md README.md docs` finds nothing.

## Workflow follow-up

- Land on its own branch and PR (title `chore(scripts): use the throwaway config in place and provide the linux helper`). It uses `chore`, with no `!`, because only developer scripts change and nothing belongs in the CHANGELOG. The PR lists the Windows checks from 1–2 as the user's to run.
- Archive the change in the implementation PR.
