# Design

## Context

- `main` holds 149 commits in the convention: `type(scope)!: description`, with the types feat, fix, docs, test, ci, build, chore and refactor. Scopes are lower-case (`tmux`, `e2e`, `shell-integration`, `openspec`, `deps`). The description starts lowercase unless it starts with an acronym (`docs: README, licenses, …`). Merges keep git's or the landing workflow's `Merge …` subjects.
- Changes land as `--no-ff` merges of a branch with two or more commits. Dependabot PRs are merged with merge commits, so the PR's own commit lands on `main` as is.
- Dependabot opens monthly PRs for actions (`.github/dependabot.yml`) and security PRs for Cargo, which has no entry. Both use Dependabot's default subject, `Bump x from a to b`.
- CI (`ci.yml`) runs on every push except Dependabot's branches, on every pull request, and on `workflow_dispatch`. Checkouts are shallow. The repo has no rulesets or required checks.
- Most commits here are made from Claude Code sessions, so a local check catches a bad message before it's pushed.

## Goals / Non-Goals

**Goals:**
- Every commit a push or PR adds is checked, with an error that names the broken rule.
- The same rules run locally before a commit is created.
- Dependabot's commits pass without anyone editing them.

**Non-Goals:**
- Making the job a required check. There are no required checks today, and that's a GitHub setting, not a file.
- Generating changelogs or versions from the commits.
- Checking commits that are already on `main`.

## Decisions

### commitlint, not committed or cocogitto

Both commitlint 21.2.3 and `committed` 1.1.11 (crate-ci, Rust) were run over `7157fba..main`.
- **committed** flags all 20 merges ("Missing type in the commit summary"). It has no option to skip merge messages: `merge_commit = true` only allows merges to exist. It also can't require a lowercase description: `subject_capitalized` only requires a capital or checks nothing.
- **commitlint** with the config below passes all 148 commits with no warnings. It rejects `feat: Draw …` and `feat(Tmux): …`, and accepts `feat(tmux)!: …` and `chore(deps): bump …`.
- **cocogitto** checks the format and types, but has no case rules either. Most of it is release tooling this repo doesn't use.

The cost is Node, which this Rust repo doesn't use otherwise. The decisions below keep that cost to one pinned command.

### Config

`commitlint.config.mjs` at the repo root, where commitlint finds it on its own:
- `extends: ['@commitlint/config-conventional']`, which supplies the type list (with `revert`), the `type(scope)!: subject` grammar, and the empty-subject and trailing-period checks. git's own `Revert "…"` subject is among commitlint's default ignores.
- `ignores: [message => message.startsWith('Merge ')]`. commitlint's default ignores cover only git's own merge wordings (`Merge branch …`, `Merge pull request …`), not the landing workflow's `Merge sharding the e2e suite in CI`.
- `subject-case` is off and replaced by a local plugin rule, `subject-lowercase-start`, which fails when the description matches `^[A-Z][a-z]`. `subject-case`'s sentence-case check can't tell `Draw …` from `README, …`, and one commit in the history starts with an acronym.
- `scope-case: lower-case`. `kebab-case` rejects `e2e`, because of its digit.
- `header-max-length: 100`. The longest subject in the history is 92 characters, and config-conventional's 100 matches it. The 72-character body line limits are off: the history's bodies are wrapped by hand, but Dependabot's carry long URLs and YAML.

### No package.json: npx with versions pinned in one script

`scripts/lint-commits` runs `npx --yes -p @commitlint/cli@21.2.3 -p @commitlint/config-conventional@21.2.3 commitlint …`. This was tested from a directory holding only the config: `extends` resolves against npx's install, and the rule failures are reported.

*Alternative: a root `package.json` with devDependencies and a lockfile, run with `npm ci`.* Dependabot's `npm` ecosystem would then keep commitlint current. But the repo would gain a package manifest, a lockfile and `node_modules/`, and a monthly Dependabot PR stream, all for one tool. The pin goes in the script instead, next to the other hand-moved pins that CLAUDE.md lists (`RUST_VERSION`, the tmux matrix).

The first run downloads the packages, which takes a few seconds. Later runs use npx's cache, so the hook adds about a second per commit.

### One script, three ranges

```
scripts/lint-commits --edit FILE   # one message (the hook)
scripts/lint-commits BASE          # BASE..HEAD
scripts/lint-commits               # merge-base(origin/main, HEAD)..HEAD
```

The no-argument form is what you run before pushing a branch. In CI the job works out `BASE`:

| Event | BASE |
|---|---|
| `pull_request` | `github.event.pull_request.base.sha` |
| `push`, `before` is an ancestor of `HEAD` | `github.event.before` |
| `push` of a new branch (`before` is all zeros), a force-push (`before` isn't an ancestor), `workflow_dispatch` | none: the no-argument form |

On `main` itself, the fallback range is empty, so a force-push to `main` checks nothing. That's acceptable, because what lands on `main` comes from a branch that was already checked. The job checks out with `fetch-depth: 0`, so `origin/main` and every `BASE` are present.

The job runs `actions/setup-node@v7` with Node 24 (commitlint 21 needs ≥ 22.12). It doesn't depend on `test`, `release` or `e2e`, and it finishes in well under a minute.

### Hook through core.hooksPath

`scripts/hooks/commit-msg` runs `exec scripts/lint-commits --edit "$1"` from the repo root (`git rev-parse --show-toplevel`). Each clone enables it once with `git config core.hooksPath scripts/hooks`. That's a per-clone setting git won't take from the repo, so the README and CLAUDE.md say to run it. `.git/hooks` holds only samples in this clone, so nothing is lost by switching.

The hook runs for merges too, where commitlint's ignore applies. `git commit --no-verify` skips it, and CI still catches what it would have.

### Dependabot subjects

`commit-message: { prefix: chore, include: scope }` gives `chore(deps): bump x from a to b`. Dependabot adds the `(deps)` scope and the colon itself. `open-pull-requests-limit: 0` disables version updates for an ecosystem, while security PRs run under their own fixed limit. `commit-message` also applies to security PRs, unless `target-branch` is set, which it isn't. So the new `cargo` entry changes only the subject of Cargo security PRs. The file's comment saying Cargo "needs no entry" is rewritten to say why it has one.

## Risks / Trade-offs

- [Dependabot's documented `prefix` and `include: scope` behavior differs from what's described above] → Dependabot opens PRs on its own schedule, so the first real subject appears after landing, not during this change. A bad subject only fails that PR's `commits` job. The fix is the config plus `@dependabot recreate`.
- [npm is unreachable or slow when committing offline] → The hook fails, so the commit fails. `--no-verify` gets past it, and CI checks the commit later.
- [Someone never enables the hook] → CI's job catches the commit on push, before it reaches `main` through a merge.
- [commitlint's pin goes stale] → It's listed with the other hand-moved pins in CLAUDE.md. A new major version may change config-conventional's rules. To vet a bump, run `scripts/lint-commits 7157fba` over the history (task 1.2).
