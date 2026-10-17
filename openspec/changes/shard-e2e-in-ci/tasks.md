# Tasks

## 1. Harness

- [x] 1.1 In `tests/e2e/main.rs`, parse `--shard K/N` (one argument after it), and after filtering keep the cases at 0-based positions i with `i % N == K - 1`. Print `e2e: shard K/N, M of T cases` before running. Exit 2 with a usage message for a missing or malformed value, `N = 0`, or `K` outside 1..=N. Put the selection in a function with a self-test (`--self-test`): 10 cases at 3 shards give 4, 3 and 3 cases, which together are all 10, each exactly once. Verify: the self-test passes; `--shard 0/4`, `--shard 5/4` and `--shard x` exit 2; `-- --list --shard 2/4` is unaffected (`--list` lists every case).
- [x] 1.2 Run all four shards headless (`BRINDLE_E2E=1 scripts/with-xvfb cargo test --test e2e -- --shard K/4` for K = 1–4). Verify: together they report 100 passed, every case exactly once (compare the `ok` lines with `--list`).

## 2. CI

- [x] 2.1 In `.github/workflows/ci.yml`, cross the `e2e` job's tmux entries with `shard: [1, 2, 3, 4]` as in design.md, pass `-- --shard ${{ matrix.shard }}/4` to the suite, name jobs `e2e (tmux <v>, <k>/4)`, and name the artifact `e2e-tmux-<v>-shard-<k>`. Verify: actionlint passes, ignoring only the unknown `ubuntu-26.04` label.
- [x] 2.2 Update README.md's and CLAUDE.md's CI and e2e notes: `--shard K/N`, CI's 4 shards per tmux version, where the shard count lives, and the artifact names.

## 3. Verify on GitHub and land

- [x] 3.1 Push the branch. Verify: the run has 8 e2e jobs plus `test` and `release`, all passing; each e2e job's log shows its `e2e: shard K/4` line, and the shards' counts add up to 100 per tmux version; the longest e2e job takes under 10 minutes. Record the shard times in the commit message.
- [x] 3.2 Verify artifacts on a scratch branch: make one case fail (as in run-tests-in-ci's 3.3), push, and check that only that case's shard uploads `e2e-tmux-<v>-shard-<k>`, for both tmux versions. Delete the scratch branch.
- [ ] 3.3 Land the usual way (archive, `--no-ff` merge "Merge sharding the e2e suite in CI"; rebase only if `main` moved), and ask before pushing `main`. Verify: the run on `main` passes.
