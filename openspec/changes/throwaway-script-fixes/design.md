# Design

## Context

- **Config.** `Config::path()` is `BRINDLE_CONFIG` when set (`src/config.rs:530`). `watch_config` stats it once a second and reloads on any mtime change (`src/main.rs:1085-1107`). Brindle writes the file in two cases only: the default file when it is missing (`load_or_default_at`), and settings-dialog saves through `config_edit::save`. A save replaces the file in one step and writes through symlinks (`configuration` spec, "When Brindle writes the config file itself").
  - Both scripts copy `--config`/`-Config` into the throwaway directory (`run-throwaway.sh` `cp`, `run-throwaway.ps1:91`), so Brindle watches the copy.
  - A reload doesn't apply everything live. Profile themes, font sizes, UI fonts, the chrome theme and keybindings apply at once. A running agent keeps its `command`, `args` and `env` until its zone next starts (investigation question 2).
- **Linux helper lookup.** `remote::find_helper` (`src/remote/mod.rs:263-292`) takes `BRINDLE_REMOTE_AGENT`, then `brindle-remote-linux-x86_64` or `brindle-remote` beside the exe.
  - A Windows source build has neither.
  - Brindle labels an uploaded helper with its own version and the file's digest. The host refuses a helper of another version (exit 97, then `undeployed`), but accepts a same-version build of older sources (investigation question 3).
- **Existing Docker build.** `scripts/install.ps1` already builds the helper in Docker:
  - image `rust:<rust-toolchain.toml channel>`;
  - `CARGO_TARGET_DIR=target/linux-helper`, kept apart from the Windows build's host artifacts;
  - a `brindle-cargo-registry` volume;
  - output `target\linux-helper\x86_64-unknown-linux-musl\release\brindle-remote`.

  It then checks the ELF magic, and the `--version` through `docker run … --version` against `Cargo.toml`'s version. README.md's hand command is the same build into the default target directory.
- **The harness's rule** (`tests/common/server.rs` `check_helper`, `Freshness::Sources`): a helper is stale when any file under `remote-agent/src`, `remote-protocol/src`, `term-core/src` or `transcript/src` is newer than it. `same_release` compares `brindle X` with `X`.

## Goals / Non-Goals

**Goals:**
- Edits to the file passed with `-Config`/`--config` reach the running Brindle.
- A Windows run from a source build can open SSH and container profiles with a helper of this checkout's sources and this binary's release, without the user setting anything.
- The two scripts keep one set of options with one meaning, and the Docker build has one implementation.

**Non-Goals:**
- Making `command`, `args` or `env` changes reach a running agent. That is Brindle's reload behaviour, not the script's.
- Building `brindle.exe`, `brindle-remote.exe` or (on Linux) `brindle` itself. The scripts still run what `cargo build` made.
- Building a static musl helper on Linux (see the decision below).
- Changing Brindle's own helper lookup or version check.

## Decisions

### The config option uses the file in place

`BRINDLE_CONFIG` is set to the file's absolute path: `Resolve-Path` in PowerShell, `cd "$(dirname)" && pwd` plus the base name in sh. The path is absolute because Brindle's working directory, and the path's meaning, can differ from the shell's.
- **Stays private:** `BRINDLE_STATE_DIR` and `BRINDLE_SERVER_DIR` (and `BRINDLE_SERVER_HELPER` on Windows), in the throwaway directory as now.
  - So the launch still doesn't hand itself to a running Brindle. The single-instance lock is keyed by the state directory.
  - The saved session and terminal server are still the run's own.
- **Without the option:** `BRINDLE_CONFIG` is `<dir>/config.toml` as now. Brindle writes its default file there, and the cleanup removes it.
- **The cleanup never touches the given file.** It removes only the throwaway directory, and the file is outside it.
- **The file is the user's:** settings-dialog saves go to it. The help text says so, and says to pass a copy to keep an original.
- **The tmux warning stays.** A profile in the file that runs tmux attaches to the server it names.
- **The script prints the path:** `run-throwaway: config <path> (edits apply live; the settings dialog writes it)`, so the next "it didn't reload" has its answer on screen.

*Alternative: keep the copy and add a `-Live` switch.* That was rejected. The option exists to try a config the user wrote for the throwaway run, and live edits are what they expected. A switch keeps the surprising default.

*Alternative: copy, and also watch the original and copy it again on change.* That was rejected. It is a second watcher in a script, it races Brindle's own saves, and the settings dialog's writes would go to the copy and be lost.

### "The right version" of the Linux helper

The helper is right when all of these hold:
1. It is a Linux ELF (magic `7f 45 4c 46`), as `install.ps1` checks.
2. It is no older than any file under the harness's four source directories, `remote-agent/Cargo.toml` or the workspace `Cargo.lock`. The extra two files catch a version bump or a dependency change that touches no source. release-please bumps only manifests and the lockfile. The harness doesn't need them, because a test build rebuilds the helper's crates anyway.
3. Its `--version`, run in the `rust:<channel>` image with the file mounted read-only, is the binary's release (`brindle.exe --version` minus `brindle `, `same_release`'s rule). The binary's version is read from the binary, not from `Cargo.toml`, so a stale `brindle.exe` shows up as a mismatch instead of passing.

Rule 2 is cheap and needs no Docker. Rule 3 costs one container start, about a second with the image present, and needs Docker.

### When the script acts, and what it does

Decision table, checked in order. "Built here" means the binary is under `<root>\target\`.

| Situation | Action |
|---|---|
| `-NoLinuxHelper` | nothing; prints that remote profiles need `BRINDLE_REMOTE_AGENT` |
| `BRINDLE_REMOTE_AGENT` set and not empty | taken as is (Brindle's own rule); prints it |
| `brindle-remote-linux-x86_64` beside the binary (an unpacked zip) | nothing: Brindle finds it, and it is that release's |
| binary not built here | nothing; prints that remote profiles need the helper beside the binary or `BRINDLE_REMOTE_AGENT` |
| helper missing or stale by rule 2, Docker usable | builds it, then checks rules 1 and 3 |
| helper fresh by rule 2, Docker usable | checks rules 1 and 3 |
| helper missing or stale, no usable Docker | warns "remote profiles won't work: \<why\>; start Docker Desktop (Linux containers) or set BRINDLE_REMOTE_AGENT" and runs without setting the variable |
| helper fresh, no usable Docker | sets the variable and warns that its version couldn't be checked, as `install.ps1` does |
| build fails, or rule 1 or 3 fails after a build | stops (exit 1) with the build's or the check's message |
| rule 3 fails on a helper fresh by rule 2 | builds once, then checks again; a second failure stops |

- **Usable Docker:** `docker` is on `PATH`, `docker info` succeeds, and its `OSType` is `linux`. This is `install.ps1`'s test, with its messages.
- **The variable is restored afterwards.** `BRINDLE_REMOTE_AGENT` is set inside the `try` and restored in `finally`, as the other variables are.
- **A stale helper is not used when Docker is missing.** Uploading old code silently is the failure this change exists to stop. A missing variable gives Brindle's own clear "not found" error on the remote tab.
- **The build runs before the launch**, in the console, so its output is visible and a failure stops the run before a window opens.

*Alternative: the version check alone (rule 3).* It misses a same-version build of older sources, which is the case Brindle itself can't catch.

*Alternative: build on every run.* A warm incremental build still costs a container start, `apt-get` and cargo's checks, so tens of seconds per launch. That was rejected.

### One Docker implementation: `scripts/linux-helper.ps1`

The build and the checks move out of `install.ps1` into `scripts/linux-helper.ps1`, a script both callers run with `&`:
- **Parameters:** `-Root`, `-Bin` (whose `--version` is the release to match), `-Build` (`Auto`, `Never`, or `Always` for `install.ps1`).
- **Output:** one result object, `@{ Path; State; Message }`. `State` is one of `Ready`, `Unchecked`, `Unavailable` or `Failed`, a closed set that the callers `switch` on with no default arm.
- **Kept from `install.ps1`:** the target directory (`target/linux-helper`), the registry volume and the image tag, so the two callers share one build's output and cache.
- **`install.ps1` keeps its stricter policy.** No Docker and no `-RemoteAgent` is an error there, since an install without the helper is broken.
- **A standalone check:** `-Check` prints the decision for the current checkout without building or launching anything. It is the window-free way to verify the table.

*Alternative: copy the logic into `run-throwaway.ps1`.* That was rejected: two copies of a Docker command drift, which is the "keep the two scripts in step" problem again.

### The shell script: a freshness check, no build

On Linux the `brindle-remote` beside the binary is a native build. Brindle deploys it to remote hosts and runs it as the local terminal server. `cargo build` makes it with the binary, so the only failure is a stale one: `cargo build -p brindle` alone, or a checkout switched after a build. For a binary under `<root>/target/`, the script compares the helper's mtime with the same files as rule 2. A stale helper stops the run: `run-throwaway: <helper> is older than <file>: run cargo build`. Outside `target/` it checks nothing, as now.

*Alternative: build the static musl helper on Linux too.* It would let remote profiles reach hosts with an older glibc or musl (Alpine). But a source build on Linux already deploys its glibc helper today, and that is a separate question. A musl build needs the musl target, and the run never needed one before. It is left as a follow-up for the user to raise.

### Keeping the scripts in step

- **Options:** the same set with the same meanings: `--config`/`-Config` (in place), `--keep`/`-Keep`, and the arguments after `--`/`-BrindleArgs`. `-NoLinuxHelper` is Windows-only because only Windows lacks a helper. The sh header names it as the one difference.
- **Help text:** both headers carry the same paragraphs in the same order: what stays private, what the config option does, and how the helper is found. `--help`/`-Help` print them. The PowerShell `Select-Object -First N` count changes with the header.
- **Docs:** CLAUDE.md's "Manual checks" sentence describes both scripts at once, so a difference has to be written down.
- **Lint:** `scripts/lint-workflows` adds `scripts/run-throwaway.sh` to its shellcheck list. PowerShell has no linter in this repo. The new script follows `install.ps1`'s style (`$ErrorActionPreference = 'Stop'`, `$LASTEXITCODE` checks).

### Architecture

This is tooling outside the crates. No layer in `docs/architecture/README.md` changes, and no wire message, config key or state file changes.
- **`testing R4`** (hermetic runs): kept for state and the server, which stay private. The config is now the user's own file, by their choice.
- **`testing R16`** (manual checks handed over with `scripts/run-throwaway`): the scripts remain the tool for it.
- **`ports R4`:** the scripts' platform split is a file per platform, as now.

## Risks / Trade-offs

- [A user passes their real config and the settings dialog rewrites it] → the help text and the printed line say the file is used in place. Saves change only the keys they concern and keep comments (`configuration` spec).
- [mtimes on Windows after a branch switch: git sets checked-out files to the switch time, so a switch can mark the helper stale] → it costs one rebuild. Rule 2 errs toward building.
- [Rule 2's file list drifts from `check_helper`'s] → `linux-helper.ps1` names the four directories with a comment pointing at `tests/common/server.rs`, and `check_helper`'s comment points back. Both lists change together.
- [The Docker build is slow on first use (image pull, registry fetch, a cold musl build)] → it prints that it is building and why (missing, or the file it is older than). `-NoLinuxHelper` skips it for a local-only check.
- [Docker Desktop is in Windows-container mode] → treated as no usable Docker, with `install.ps1`'s message.

## Migration Plan

None beyond the help text. Anyone who relied on `-Config` copying passes a copy.

## Open Questions

None that block the tasks. The user didn't make these choices; each is this design's recommendation and can be reversed before apply:
- the config is used in place, with no copy switch;
- no musl build on Linux;
- a stale helper is not used when Docker is missing;
- the build is on by default, with a `-NoLinuxHelper` opt-out;
- `remote-agent/Cargo.toml` and `Cargo.lock` are freshness inputs.
