# release-process Specification

## Purpose
Defines how Brindle versions are derived from Conventional Commits, what a GitHub Release contains and when it is published, and the checks that keep the commit subjects on `main` in Conventional Commits form.

## Requirements

### Requirement: Release pull request
After each push to `main` that adds releasable commits since the last release, a release pull request titled `chore(release): <version>` SHALL exist and be kept current. It SHALL set the next version and add that version's `CHANGELOG.md` entry. With no releasable commits, no release pull request SHALL be opened.

#### Scenario: Feature commit lands
- **WHEN** a `feat: …` commit is pushed to `main` after release 0.3.0
- **THEN** an open pull request titled `chore(release): 0.4.0` sets the version to 0.4.0 and adds a 0.4.0 changelog entry listing the feature

#### Scenario: Further commits update the same pull request
- **WHEN** a `fix: …` commit is pushed to `main` while that release pull request is open
- **THEN** the same pull request is updated to include the fix, and no second release pull request is opened

#### Scenario: Only non-releasable commits
- **WHEN** only `docs`, `test`, `ci` or `chore` commits have landed since the last release
- **THEN** no release pull request is opened

### Requirement: Version from commits
The next version SHALL follow Semantic Versioning from the commit types since the last release: `fix` and `perf` bump the patch, `feat` bumps the minor, and a breaking change (`!` or a `BREAKING CHANGE` footer) bumps the major. While the major version is 0, a breaking change SHALL bump the minor instead.

#### Scenario: Breaking change before 1.0
- **WHEN** the last release is 0.3.2 and a `feat(settings)!: …` commit has landed since
- **THEN** the release pull request proposes 0.4.0

#### Scenario: Fixes only
- **WHEN** the last release is 0.3.2 and only `fix` commits have landed since
- **THEN** the release pull request proposes 0.3.3

### Requirement: One version for the workspace
The `brindle`, `brindle-remote-agent` and `brindle-remote-protocol` crates SHALL always carry the same version. The release pull request SHALL set it for all of them, and `Cargo.lock` SHALL change only in those crates' own entries.

#### Scenario: Version bump
- **WHEN** the release pull request for 0.4.0 is opened
- **THEN** `brindle --version` and `brindle-remote --version` built from it both report 0.4.0, and no third-party crate's entry in `Cargo.lock` has changed

### Requirement: Release notes from commits
Each release's notes and its `CHANGELOG.md` entry SHALL list the release's commits grouped by type, with sections for breaking changes, features, fixes and performance. Commits of type `docs`, `test`, `ci`, `build`, `style`, `refactor` and `chore` SHALL be left out.

#### Scenario: Mixed commits
- **WHEN** a release contains `feat(tmux): …`, `fix: …`, `docs(openspec): …` and `test(e2e): …` commits
- **THEN** its notes list the feature and the fix under their sections, and do not mention the docs or test commits

### Requirement: Release on merge
Merging the release pull request SHALL create the tag `v<version>` on the merge commit and a GitHub Release for that tag, carrying the version's notes.

#### Scenario: Release pull request merged
- **WHEN** the `chore(release): 0.4.0` pull request is merged
- **THEN** a tag `v0.4.0` exists on the merge commit, and a GitHub Release named for 0.4.0 carries the 0.4.0 notes

### Requirement: Release assets
Each release SHALL carry these assets, built from the tagged commit:
- `brindle-<version>-x86_64-unknown-linux-gnu.tar.gz`, holding `brindle`, `brindle-remote-linux-x86_64`, `LICENSE-MIT` and `LICENSE-APACHE` in a top-level `brindle-<version>-x86_64-unknown-linux-gnu/` directory;
- `brindle-<version>-x86_64-pc-windows-msvc.zip`, as the desktop-integration capability describes it;
- `SHA256SUMS`, with the SHA-256 of each other asset, in the format `sha256sum -c` reads.

#### Scenario: Verify a download
- **WHEN** a user downloads a release's archives and its `SHA256SUMS` and runs `sha256sum -c SHA256SUMS`
- **THEN** every archive checks out

#### Scenario: Run from the Linux archive
- **WHEN** a user unpacks the Linux archive and runs `brindle --version` from it
- **THEN** it prints the release's version, and a remote profile from that binary deploys the helper found beside it

#### Scenario: No WinGet manifest asset
- **WHEN** a release is published
- **THEN** none of its assets is a WinGet manifest

### Requirement: Published only when complete
A release SHALL become visible as published only after all of its assets are attached. If building any asset fails, the release SHALL stay unpublished, with the failure visible in the release run.

#### Scenario: Windows build fails
- **WHEN** the Windows build fails during a release run
- **THEN** no published release for that version exists, and the release run is marked failed

### Requirement: Release builds pass CI's checks
The release assets SHALL be built by the same build-and-check steps CI runs for the Linux release build and the Windows build, including the Windows exe's version, icon and DPI-manifest checks.

#### Scenario: Exe check fails at release time
- **WHEN** the Windows exe built for a release lacks its version resource
- **THEN** the release run fails before the zip is attached

### Requirement: Release pull requests run CI
The release pull request SHALL trigger pull-request checks like any other pull request: the commit message check, the unit tests with the workspace version check, and the title check. The release builds and the end-to-end suites MAY be skipped on it, since its code is unchanged and the release run builds both platforms before publishing.

#### Scenario: Release pull request opened
- **WHEN** the release pull request is opened or updated
- **THEN** the commit, test and title checks run on it, and it can satisfy required checks

#### Scenario: Heavy jobs skipped
- **WHEN** the release pull request is updated after a push to `main`
- **THEN** the release builds and end-to-end jobs report as skipped rather than running

### Requirement: Pull request title check
Every pull request's title SHALL be checked against the repository's Conventional Commits rules, the same rules as commit messages, whenever the pull request is opened, edited, reopened or updated. The title SHALL also be at most 93 characters, so the squash commit's subject with GitHub's ` (#NN)` suffix stays within the 100-character header limit.

#### Scenario: Out-of-format title
- **WHEN** a pull request is opened with the title `Add release workflow`
- **THEN** the title check fails and names the rule that the title breaks

#### Scenario: Title fixed
- **WHEN** the pull request's title is edited to `ci: add a release workflow`
- **THEN** the title check runs again and passes

#### Scenario: Title too long for the squash suffix
- **WHEN** a pull request title is 95 characters long and otherwise valid
- **THEN** the title check fails and says the limit is 93 characters
