# Tasks

## 1. Spike: release-please behaviour

- [ ] 1.1 On a scratch fork or repo with this workspace's layout (`[workspace.package]` version, three members inheriting it), run release-please (`npx release-please release-pr --dry-run` or the action) with the planned config. Record in design.md whether (a) the rust strategy plus `cargo-workspace` bumps `[workspace.package].version` and only the members' `Cargo.lock` entries, and (b) `draft: true` with `force-tag-creation` creates the tag and the next run finds it. Switch design.md to the fallback for whichever fails. Verify: the dry-run output and the recorded result.

## 2. One workspace version

- [ ] 2.1 Move the version to `[workspace.package]` in the root `Cargo.toml`, and set `version.workspace = true` in all three crates. Verify: `cargo metadata --locked --no-deps` shows 0.2.0 for each, `git diff Cargo.lock` is empty, and `cargo build` of an already-built tree recompiles no dependency crates.
- [ ] 2.2 Check that `cargo test --workspace --locked` passes and that `brindle --version` and `brindle-remote --version` both print 0.2.0.

## 3. Reusable build workflows

- [ ] 3.1 Add `rust-toolchain.toml` (`channel = "1.99.1"`, `profile = "minimal"`, no targets). In `ci.yml`, replace `dtolnay/rust-toolchain` with `rustup toolchain install`, plus `rustup target add x86_64-unknown-linux-musl` where the helper is built, and remove the `RUST_VERSION` env. Update the README's build section to say rustup picks the toolchain from the file. Verify: `rustup show active-toolchain` in the checkout reports 1.99.1 from `rust-toolchain.toml`, a local `cargo build` recompiles no dependencies, `actionlint` is clean, and CI's logs show 1.99.1.
- [ ] 3.2 Add `scripts/package-linux.sh`, which checks that the helper is a static ELF and that `brindle --version` matches the crate version, then writes `target/dist/brindle-<version>-x86_64-unknown-linux-gnu.tar.gz` with its top-level directory, the two binaries and the licences. Verify: run it locally after `cargo build --release` and the musl helper build, then list the tarball's contents.
- [ ] 3.3 Extract `ci.yml`'s `release` job into `.github/workflows/build-linux.yml` (`workflow_call`, inputs `ref`), uploading the helper and the `brindle-linux-x64` tarball artifact. Verify: `actionlint`, and a CI run on the branch shows both artifacts.
- [ ] 3.4 Extract `ci.yml`'s `windows` job into `.github/workflows/build-windows.yml` (inputs `ref` and `winget-validate`, default true), with the manifest and `winget validate` step gated on the input. Verify: `actionlint`, and a CI run on the branch passes the exe checks and uploads `brindle-windows-x64`.
- [ ] 3.5 Replace the two jobs in `ci.yml` with `uses:` calls, keeping the job names so the e2e and needs wiring hold. Verify: the branch's CI run is green with the same set of checks as before, plus the Linux tarball.

## 4. PR title check

- [ ] 4.1 Add `.github/workflows/pr-title.yml` (`pull_request`: opened, edited, reopened, synchronize; `contents: read`). Pass the title through `env`, fail above 93 characters, then run `scripts/lint-commits --edit` on a file holding it. Verify: on a test PR, `Add release workflow` fails naming the rule, a 95-character valid title fails naming the 93 limit, and `ci: add a release workflow` passes after an edit.

## 5. Release workflow

- [ ] 5.1 Add `release-please-config.json` (one package `.`, rust type, `cargo-workspace` plugin, `include-component-in-tag: false`, `bump-minor-pre-major`, `pull-request-title-pattern: "chore(release): ${version}"`, `changelog-sections` showing only feat/fix/perf/revert, `draft` and `force-tag-creation` per the spike, `bootstrap-sha` `d33e156`), `.release-please-manifest.json` (`{".": "0.2.0"}`) and an empty `CHANGELOG.md`. Verify: `npx release-please release-pr --dry-run` against the branch proposes the expected version and notes and touches only the workspace version and the members' lockfile entries.
- [ ] 5.2 Add `.github/workflows/release.yml` (on push to `main`): mint the App token, run release-please, and on `release_created` call `build-linux.yml` and `build-windows.yml` (`winget-validate: false`) at the tag. Then download the two archives, write `SHA256SUMS`, `gh release upload` all three, and `gh release edit --draft=false`. Set `concurrency` so release runs don't overlap. Verify: `actionlint`, and a dry run on a fork produces a draft, attaches the three assets, then publishes.
- [ ] 5.3 Document in README.md the Linux tarball (contents, glibc floor, running `brindle --install-desktop` from it) and that the Windows zip comes from GitHub Releases. Verify: the README's commands match the archive layout from 3.2.

## 6. Docs and settings

- [ ] 6.1 Update CLAUDE.md's CI paragraph: the reusable build workflows, `rust-toolchain.toml` as the one Rust pin (replacing "`RUST_VERSION` matches the desktop's toolchain"), the Linux tarball in CI, `pr-title.yml` and its 93-character limit, `release.yml`, release-please's config files, and the App secrets. Verify: every file and pin the paragraph names exists.
- [ ] 6.2 List the manual repository settings in the PR description: create and install the GitHub App, add `RELEASE_APP_ID` and `RELEASE_APP_PRIVATE_KEY`, make `PR title` a required check, and set the squash commit default to the PR title. Verify: the user confirms each is done.

## 7. End-to-end check

- [ ] 7.1 After merging and setting up the settings, push a `fix:` commit and confirm the release PR opens with CI and the title check running on it. Merge it, then confirm that `v0.2.1` is tagged, the release stays a draft until its three assets are attached, and `sha256sum -c SHA256SUMS` passes on the downloads.

## Workflow follow-up

- Archive the change after 7.1, syncing the `release-process` spec.
