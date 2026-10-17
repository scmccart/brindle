# Tasks

## 1. Lint locally

- [ ] 1.1 Add `commitlint.config.mjs` as in design.md: extend config-conventional, ignore `Merge …`, use the local `subject-lowercase-start` rule instead of `subject-case`, `scope-case: lower-case`, `header-max-length: 100`, no body or footer line limits. Verify by piping messages through commitlint:
  - fail: `Fix the thing`, `feat: Draw pane titles`, `feat(Tmux): x`, `feature: x`, `feat: draw titles.`;
  - pass: `docs: README, licenses, smaller release build`, `feat(tmux)!: x`, `feat(e2e): x`, `feat(shell-integration): x`, `chore(deps): bump actions/checkout from 6 to 7`, `Merge sharding the e2e suite in CI`, `Revert "feat: x"`.
- [ ] 1.2 Add `scripts/lint-commits` (bash, `set -euo pipefail`, executable). It runs commitlint 21.2.3 through `npx --yes -p @commitlint/cli@… -p @commitlint/config-conventional@…`, with the version in one variable. Forms: `--edit FILE`, `BASE` (checks `BASE..HEAD`), and no argument (checks from `git merge-base origin/main HEAD`). It prints the range and its commit count before checking, and exits 2 with a usage message on bad arguments. Verify:
  - `scripts/lint-commits 7157fba` passes all 148 commits with no problems or warnings;
  - `git log -1 --format=%B 7157fba > f && scripts/lint-commits --edit f` passes;
  - on this branch, the no-argument form checks only the branch's commits;
  - `scripts/lint-commits --edit` and `scripts/lint-commits a b` exit 2.
- [ ] 1.3 Add `scripts/hooks/commit-msg` (executable, runs `scripts/lint-commits --edit "$1"` from the repo root), and run `git config core.hooksPath scripts/hooks` in this clone. Verify in a throwaway clone with the hook enabled: `git commit --allow-empty -m 'Bad message'` is refused and names `type-empty`, `git commit --allow-empty -m 'chore: ok'` succeeds, and `git merge --no-ff` with a `Merge …` message succeeds.
- [ ] 1.4 Document the convention and the hook. README.md: a short "Commit messages" section with the format, the hook setup line, and `scripts/lint-commits` before pushing. CLAUDE.md:
  - the convention (types, lower-case scopes such as `tmux`, `e2e` and `openspec`, a lowercase description, `!` for breaking, `docs(openspec)` for propose, archive and check-off commits, merges as `Merge …`);
  - the hook;
  - the commitlint pin among the hand-moved pins.

  Verify: both describe the same command and setup line as the scripts.

## 2. CI and Dependabot

- [ ] 2.1 Add a `commits` job to `.github/workflows/ci.yml`:
  - `actions/checkout@v7` with `fetch-depth: 0`, and `actions/setup-node@v7` with Node 24;
  - a step that picks `BASE` by the table in design.md and runs `scripts/lint-commits` with or without it;
  - a comment explaining the fallbacks.

  Update CLAUDE.md's CI paragraph (the jobs list) and README.md's CI notes. Verify: actionlint passes, ignoring only the unknown `ubuntu-26.04` label.
- [ ] 2.2 In `.github/dependabot.yml`:
  - give the `github-actions` entry `commit-message: {prefix: chore, include: scope}`;
  - add a `cargo` entry (`directory: /`, monthly) with `open-pull-requests-limit: 0` and the same `commit-message`;
  - rewrite the header comment: Cargo has an entry only to set the security PRs' subject, and gets no version updates.

  Update CLAUDE.md's Dependabot sentence. Verify: the file parses as YAML, and both entries have `commit-message`.

## 3. Verify on GitHub and land

- [ ] 3.1 Push the branch. Verify: the `commits` job passes; its log shows the fallback range (new branch) and the branch's commit count; the other jobs are unaffected. Record the run ID in the commit message.
- [ ] 3.2 Push a scratch branch from this one with a commit titled `Add a bad subject`. Verify: the `commits` job fails, and its log names `type-empty` and `subject-empty` for that commit only. Then push a second, well-formed commit to the scratch branch. Verify: that run's range is the first push's head `..HEAD`, so it checks only the new commit and passes. The bad commit stays visible to a PR's `base..head` range. Delete the scratch branch.
- [ ] 3.3 Land the usual way (archive, `--no-ff` merge "Merge linting commit messages"; rebase only if `main` moved), and ask before pushing `main`. Verify:
  - the run on `main` passes, with its `commits` job checking the pushed range;
  - Dependabot reports no config error for the new `dependabot.yml` (repo Insights → Dependency graph → Dependabot, or its "Dependabot Updates" run).
