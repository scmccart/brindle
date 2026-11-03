# Proposal

## Why

Brindle has no releases. The README promises a Windows zip "with each release", and `scripts/winget-manifest.ps1` already points at `releases/download/v<version>/`, but nothing publishes them. Versions are bumped by hand. Separately, PRs are squash-merged, so the PR title becomes the commit on `main`, and CI's commit check only looks at the branch's own commits. Out-of-format titles are caught only after they land.

## What Changes

- Releases are driven by Conventional Commits through release-please. Each push to `main` keeps a release PR (`chore(release): <version>`) up to date with the next version and a `CHANGELOG.md` entry. Merging it tags `v<version>` and creates the GitHub Release with notes grouped by commit type. Before 1.0, breaking changes bump the minor version.
- The release run builds and attaches the assets: a Linux `tar.gz` (`brindle`, the static `brindle-remote-linux-x86_64` helper, licences), the existing Windows zip, and a `SHA256SUMS` file. The release is published only once every asset is attached. The WinGet manifest is not published.
- The Linux release build and the Windows build-and-package jobs move out of `ci.yml` into reusable workflows. CI and the release workflow both call them, so a release runs the same exe checks as CI.
- The workspace's three crates (`brindle`, `brindle-remote-agent`, `brindle-remote-protocol`) share one version from `[workspace.package]`. Brindle matches its helper by that version.
- A new PR-title workflow checks every pull request's title against the repo's commitlint rules, leaving room for GitHub's ` (#NN)` suffix.
- CLAUDE.md's CI paragraph and the README's install sections describe releases and the title check.

## Capabilities

### New Capabilities
- `release-process`: how versions are chosen from commits, what a release contains and when it is published, and the checks that keep commit subjects on `main` in Conventional Commits form.

### Modified Capabilities
<!-- None: desktop-integration's "Windows release archive" requirement is unchanged; the zip it describes becomes a release asset. -->

## Impact

- `.github/workflows/`: new `release.yml`, `pr-title.yml`, and reusable `build-linux.yml` and `build-windows.yml`. `ci.yml` calls the reusable ones.
- New `rust-toolchain.toml` pins Rust 1.99.1 for local builds and CI alike, replacing CI's `RUST_VERSION`.
- New `release-please-config.json`, `.release-please-manifest.json` and `CHANGELOG.md`.
- `Cargo.toml`, `remote-agent/Cargo.toml` and `remote-protocol/Cargo.toml` take their version from the workspace. `Cargo.lock` changes only in workspace members' versions.
- New repository secrets for a GitHub App (or fine-grained PAT) token, so release PRs trigger CI. The PR-title check becomes a required status check, and squash merges default to the PR title (repository settings, done by hand).
- `README.md` and `CLAUDE.md`.
