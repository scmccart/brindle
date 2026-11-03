# Design

## Context

- `ci.yml` has a `release` job (Linux release build plus the static musl helper, uploaded as `brindle-remote-linux-x86_64`) and a `windows` job that needs it. The Windows job builds, tests, checks the exe's resources and shortcut, then runs `scripts/package-windows.ps1` and `scripts/winget-manifest.ps1`, and uploads the zip. Both read `RUST_VERSION` and `BUILD_PACKAGES` from `ci.yml`'s top-level `env`.
- PRs are squash-merged (`… (#15)`). The `commits` job lints a PR's own commits, never its title.
- All three crates are at 0.2.0, each declaring its version separately. `remote/client.rs` matches the deployed helper, and names its cache directory, by Brindle's `CARGO_PKG_VERSION`, so the crates must move together.
- There are no tags. `scripts/winget-manifest.ps1` already assumes tags of the form `v<version>`.

## Goals / Non-Goals

**Goals:**
- One build definition per platform, shared by CI and releases.
- Releases need no manual version or notes editing; merging the release PR is the only manual step.
- No change to local dev builds: profiles, features and dependency versions stay as they are.

**Non-Goals:**
- Publishing to WinGet, or attaching the manifest (CI keeps validating it).
- `.deb`, AppImage, aarch64 or macOS artifacts.
- Signing binaries. Build-provenance attestations are a possible follow-up.

## Decisions

### release-please, as one root package
Use `googleapis/release-please-action` with `release-please-config.json` and `.release-please-manifest.json` (`{".": "0.2.0"}`). There is one package at `.`, with `release-type: rust`, the `cargo-workspace` plugin, `include-component-in-tag: false` (tags are `v0.3.0`), `bump-minor-pre-major: true`, `pull-request-title-pattern: "chore(release): ${version}"`, and `bootstrap-sha` set to `main`'s head when the change branches (`d33e156`, the 0.2.0 state), since a merge commit can't name itself. `changelog-sections` lists `feat`, `fix`, `perf` and `revert` and hides the rest. Breaking changes always get their own section.

Alternatives: tag-push plus git-cliff (manual versioning, which was rejected), and a `workflow_dispatch` that bumps and pushes (fights branch protection).

### One version through `[workspace.package]`
Root `Cargo.toml` gains `[workspace.package] version = "0.2.0"`, and all three crates use `version.workspace = true`. release-please then has one value to bump. The `cargo-workspace` plugin rewrites the members' entries in `Cargo.lock` and leaves third-party entries alone, which honours the lockfile rule.

Local build time is unaffected. Inheriting the version is resolved at manifest-load time, and a version bump recompiles only the workspace crates (`CARGO_PKG_VERSION` changes), exactly as a hand bump does today. Dependencies are not rebuilt.

**This needs a spike first** (task 1.1): confirm on a scratch branch that the rust strategy and `cargo-workspace` bump `[workspace.package].version` and the lockfile when crates inherit it. Fallback if they don't: `release-type: simple` with a `toml` extra-file at `$.workspace.package.version`, plus a release-PR job that runs `cargo update -w --offline` and pushes the lockfile change.

Alternative: three versioned packages linked by `linked-versions`. It gives three components and three changelogs for one product, and the tags collide.

### A GitHub App token for release-please
Pull requests opened with `GITHUB_TOKEN` trigger no `pull_request` workflows, so the release PR would get no CI and couldn't satisfy required checks. The release workflow mints a token with `actions/create-github-app-token` from `RELEASE_APP_ID` and `RELEASE_APP_PRIVATE_KEY`, for an app installed only on this repo with contents and pull-requests write. An App beats a PAT: it is short-lived, scoped to one repo and not tied to a person.

### Draft until complete
release-please creates the GitHub Release as a draft (`draft: true`, with `force-tag-creation: true` so the tag exists while it is a draft, since release-please finds the last release through its tag). The same run then builds, uploads the assets and `SHA256SUMS`, and publishes with `gh release edit --draft=false`. A failed build leaves a draft, which a rerun completes.

The spike (task 1.1) confirms `force-tag-creation` with drafts. Fallback: `skip-github-release` on the release-please step, and the workflow creates the tag and draft itself from release-please's notes output.

### Reusable build workflows
```
ci.yml ----------+---> build-linux.yml   (workflow_call: ref, package)
                 |        build --release, musl helper, [package-linux.sh]
release.yml -----+        artifacts: brindle-remote-linux-x86_64, [brindle-linux-x64]
                 |
                 +---> build-windows.yml (workflow_call: ref, winget-validate)
                          needs the helper artifact from the same run
                          build, test, exe checks, shortcut, package-windows.ps1
                          artifact: brindle-windows-x64
```
- Both take a `ref` (empty means the event's commit; the release passes the tag).
- `build-linux.yml` always packages, so CI exercises `scripts/package-linux.sh` on every PR. It is cheap, and it catches packaging breaks before a release does.
- `build-windows.yml` runs `winget-manifest.ps1` and `winget validate` only when `winget-validate` is true. CI passes true and the release passes false.
- Artifacts uploaded inside a called workflow belong to the caller's run, so `build-windows` downloads the helper that `build-linux` uploaded, as today.
- Called workflows don't inherit the caller's `env`, so `RUST_VERSION` moves to a root `rust-toolchain.toml` (`channel = "1.99.1"`, `profile = "minimal"`, no `targets`). That is one pin for local builds on both machines, CI, the build workflows and the e2e jobs, so release binaries come from the compiler that was tested. Jobs drop `dtolnay/rust-toolchain` and run `rustup toolchain install`, which reads the file. Jobs that need it add the musl target with `rustup target add`, which keeps it off Windows checkouts. Alternative: a CI-only `.github/rust-version` file. It was rejected because local builds could drift from the release compiler. `BUILD_PACKAGES` moves into `build-linux.yml`, and `ci.yml` keeps its own copy for `test` and `e2e`.

### Linux archive
`scripts/package-linux.sh` mirrors `package-windows.ps1`. It checks that the helper is a static ELF and that `brindle --version` matches `Cargo.toml`, then writes `target/dist/brindle-<version>-x86_64-unknown-linux-gnu.tar.gz` with a top-level directory of the same name. It is built on `ubuntu-26.04` like CI, so its glibc floor is that release's. The README states this.

### PR title check
`pr-title.yml` runs on `pull_request` (`opened`, `edited`, `reopened`, `synchronize`) with `contents: read`. It passes the title through `env` (never interpolated into the script), fails above 93 characters, writes the title to a file and runs `scripts/lint-commits --edit <file>`. That reuses the same commitlint config and pinned version as commits. In repository settings, the check becomes required and squash commits default to the PR title, so what is checked is what lands.

## Risks / Trade-offs

- [release-please doesn't handle inherited workspace versions] → Spike first; the fallback (simple strategy plus a lockfile job) is in place.
- [Drafts and tags interact badly] → Spike first; fallback is `skip-github-release` with the workflow creating the draft.
- [App secrets missing or expired] → The release-please step fails loudly; nothing is published.
- [`docs`/`chore` hidden from notes may drop something worth mentioning] → Use `feat`/`fix` for user-visible changes, as the commit rules already ask.
- [glibc floor of ubuntu-26.04 excludes older distros] → Documented. An older build runner can be a follow-up.
- [Release builds take as long as CI's Windows job (~tens of minutes)] → Acceptable; the release stays a draft until done.

## Migration Plan

1. Land the change. `bootstrap-sha` makes release-please ignore history up to 0.2.0.
2. Create the GitHub App, install it on the repo and add the two secrets. Set the title check as required and the squash default to PR title.
3. The next `feat`/`fix` on `main` opens `chore(release): 0.3.0` (or 0.2.1). Merging it produces the first release.

Rollback: delete `release.yml` and the release-please files. The build workflows and the title check stand on their own.
