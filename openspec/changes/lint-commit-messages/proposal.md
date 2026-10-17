# Proposal

## Why

On 2026-10-16 the whole history was rewritten to Conventional Commits, with scoped, lowercase subjects (`feat(tmux): draw pane title lines`). Nothing checks new commits, so a single `Fix the thing` or a Dependabot `Bump x from a to b` would break the convention again.

## What Changes

- **commitlint config (`commitlint.config.mjs`):** extends `@commitlint/config-conventional` and adds what this repo does on top:
  - Subjects start lowercase. A local `subject-lowercase-start` rule allows acronyms (README, CI, OSC).
  - Scopes are lower-case.
  - Headers are at most 100 characters.
  - Body and footer lines have no length limit, because Dependabot bodies carry long URLs.
  - Merge commits (`Merge …`) are skipped.
- **`scripts/lint-commits`:** one entry point for CI and for local use. It runs commitlint 21.2.3 through `npx`, with the versions pinned in the script.
  - `--edit FILE` checks one message, for the hook.
  - `[BASE]` checks `BASE..HEAD`, and defaults to the commits not on `origin/main`.
- **`scripts/hooks/commit-msg`:** calls `scripts/lint-commits --edit "$1"`. A clone enables it with `git config core.hooksPath scripts/hooks`.
- **CI `commits` job:** checks the commits a push or pull request adds:
  - `pull_request` checks from the PR's base.
  - `push` checks from `before`.
  - A new branch, a force-push and `workflow_dispatch` fall back to the commits not on `origin/main`.
- **Dependabot:** `commit-message` prefix `chore` with scope, so its subjects read `chore(deps): bump …`, matching the history.
  - The `github-actions` entry gets the setting.
  - A new `cargo` entry with `open-pull-requests-limit: 0` (no version updates, as now) carries the same setting for security PRs.
- **Docs:** README.md and CLAUDE.md describe the convention, the hook and the CI job.

## Capabilities

### New Capabilities

None. This is repository tooling.

### Modified Capabilities

None (`skip_specs: true`): Brindle's behavior doesn't change.

## Impact

- New files: `commitlint.config.mjs`, `scripts/lint-commits`, `scripts/hooks/commit-msg`.
- Changed files: `.github/workflows/ci.yml` (new `commits` job with `actions/setup-node`), `.github/dependabot.yml`, README.md, CLAUDE.md.
- **Node becomes a dev dependency:** Node ≥ 22.12 runs the hook and the CI job. The repo gets no `package.json` or lockfile. The commitlint version is pinned in `scripts/lint-commits` and moved by hand, like `RUST_VERSION`.
- The existing history (`7157fba..main`, 148 commits) passes the config with no errors or warnings. The root commit, which a range excludes, passes on its own.
