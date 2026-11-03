# Tasks

## 1. Spike: release-please behaviour

- [x] 1.1 Read release-please 17.11.2's source to see (a) how the rust strategy bumps workspace members and `Cargo.lock`, and (b) whether `draft: true` with `force-tag-creation` creates the tag and the next run finds the release. Verify: the results are recorded in design.md, and design and tasks follow them.

## 2. One workspace version

- [x] 2.1 Add a step to `ci.yml`'s `test` job (which already has the toolchain) that fails, naming each crate and its version, when the `cargo metadata --no-deps` versions of the three workspace crates differ. Verify: it passes as-is, and fails on a local run with one crate's version edited.

## 3. Reusable build workflows

- [x] 3.1 Add `rust-toolchain.toml` (`channel = "1.99.1"`, `profile = "minimal"`, no targets). In `ci.yml`, replace `dtolnay/rust-toolchain` with `rustup toolchain install`, plus `rustup target add x86_64-unknown-linux-musl` where the helper is built, and remove the `RUST_VERSION` env. Update the README's build section to say rustup picks the toolchain from the file. Verify: `rustup show active-toolchain` in the checkout reports 1.99.1 from `rust-toolchain.toml`, a local `cargo build` recompiles no dependencies, `actionlint` is clean, and CI's logs show 1.99.1.
- [x] 3.2 Add `scripts/package-linux.sh`, which checks that the helper is a static ELF and that `brindle --version` matches the crate version, then writes `target/dist/brindle-<version>-x86_64-unknown-linux-gnu.tar.gz` with its top-level directory, the two binaries and the licences. Verify: run it locally after `cargo build --release` and the musl helper build, then list the tarball's contents.
- [x] 3.3 Extract `ci.yml`'s `release` job into `.github/workflows/build-linux.yml` (`workflow_call`, inputs `ref`), uploading the helper and the `brindle-linux-x64` tarball artifact. Verify: `actionlint`, and a CI run on the branch shows both artifacts.
- [x] 3.4 Extract `ci.yml`'s `windows` job into `.github/workflows/build-windows.yml` (inputs `ref` and `winget-validate`, default true), with the manifest and `winget validate` step gated on the input. Verify: `actionlint`, and a CI run on the branch passes the exe checks and uploads `brindle-windows-x64`.
- [x] 3.5 Replace the two jobs in `ci.yml` with `uses:` calls, keeping the job names so the e2e and needs wiring hold. Verify: the branch's CI run is green with the same set of checks as before, plus the Linux tarball.

## 4. PR title check

- [ ] 4.1 Add `.github/workflows/pr-title.yml` (`pull_request`: opened, edited, reopened, synchronize; `contents: read`). Pass the title through `env`, fail above 93 characters, then run `scripts/lint-commits --edit` on a file holding it. Verify: on a test PR, `Add release workflow` fails naming the rule, a 95-character valid title fails naming the 93 limit, and `ci: add a release workflow` passes after an edit.

## 5. Release workflow

- [x] 5.1 Add `release-please-config.json` (one package `.`, rust type, no plugins, `include-component-in-tag: false`, `bump-minor-pre-major`, `pull-request-title-pattern: "chore(release): ${version}"`, `changelog-sections` showing only feat/fix/perf/revert, `draft` and `force-tag-creation`, `bootstrap-sha` `2d6942e`, the 0.2.0 version bump) and `.release-please-manifest.json` (`{".": "0.2.0"}`). There is no `CHANGELOG.md` yet: release-please creates it, and an existing file without a version heading would be demoted below the first entry. Verify: the config validates against release-please 17.11.2's schema; its `CargoToml` and `CargoLock` updaters, run on this repo for 0.3.0, change only the three `version` lines and the three members' lockfile entries; and `cargo metadata --locked --offline` accepts the result. (Version choice and notes are checked end to end in 7.1.)
- [ ] 5.2 Add `.github/workflows/release.yml` (on push to `main`): mint the App token, run release-please, and on `release_created` call `build-linux.yml` and `build-windows.yml` (`winget-validate: false`) at the tag. Then download the two archives, write `SHA256SUMS`, `gh release upload` all three, and `gh release edit --draft=false`. Set `concurrency` so release runs don't overlap. Verify: `actionlint` is clean. The draft, upload and publish sequence is proven by the first real release in 7.1 rather than a fork (a failure there leaves only a draft).
- [x] 5.3 Document in README.md the Linux tarball (contents, glibc floor, running `brindle --install-desktop` from it) and that the Windows zip comes from GitHub Releases. Verify: the README's commands match the archive layout from 3.2.

## 6. Docs and settings

- [x] 6.1 Update CLAUDE.md's CI paragraph: the reusable build workflows, `rust-toolchain.toml` as the one Rust pin (replacing "`RUST_VERSION` matches the desktop's toolchain"), the Linux tarball in CI, `pr-title.yml` and its 93-character limit, `release.yml`, release-please's config files, and the App secrets. Verify: every file and pin the paragraph names exists.
- [ ] 6.2 List the manual repository settings in the PR description: create and install the GitHub App, add `RELEASE_APP_ID` and `RELEASE_APP_PRIVATE_KEY`, make `PR title` a required check, and set the squash commit default to the PR title. Verify: the user confirms each is done.

## 7. End-to-end check

- [ ] 7.1 After merging and setting up the settings, push a `fix:` commit and confirm the release PR opens with CI and the title check running on it. Merge it, then confirm that `v0.2.1` is tagged, the release stays a draft until its three assets are attached, and `sha256sum -c SHA256SUMS` passes on the downloads.

## Workflow follow-up

- Archive the change after 7.1, syncing the `release-process` spec.
