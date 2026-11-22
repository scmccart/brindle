---
paths:
  - ".github/**"
  - "scripts/lint-workflows"
  - "scripts/package-*"
---
# GitHub Actions workflows

You are editing CI or release plumbing. CLAUDE.md's "End-to-end suite" section lists every job; keep it in step with the change.

- Run `scripts/lint-workflows` (actionlint and shellcheck, through Docker) before committing. Name the script in commands, not `.github/` paths: this repo's worktree guard refuses some commands that name `.github`.
- `build-linux.yml` and `build-windows.yml` are called by both `ci.yml` and `release.yml`. A called workflow gets no permission its caller didn't grant, and GitHub checks that when the workflow loads, even for a job that would be skipped. So anything that needs `id-token: write`, `attestations: write` or `contents: write` goes in `release.yml`'s own jobs (`attest`, `publish`), not in the reusable workflows.
- Give each job only the permissions it uses, at job level; the workflow's own stays `contents: read`.
- Pass `${{ }}` values a script reads through `env:`, never inline in `run:`.
- Cite GitHub's docs, or the action's README or `action.yml`, for any claim about permissions, triggers or an action's inputs. Don't rely on memory: actions move major versions often.
- `release.yml`'s build, attest and publish jobs run only when release-please creates a release. Say in the PR what that leaves untested, and what to check after the next release.
