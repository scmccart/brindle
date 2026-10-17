# Proposal

## Why

Each CI run spends about 25 minutes in each e2e job: the harness runs the suite's 100 cases one after another, about 1450 s at `BRINDLE_E2E_SLOW=2`. The `test` and `release` jobs finish in 1–2 minutes with warm caches, so e2e sets the wall time of every run.

## What Changes

- **`--shard K/N` in the e2e harness:** after any name filters, run only the cases at positions K, K+N, K+2N, … (1-based) of the selected list. It prints which shard it ran, and rejects a malformed or out-of-range shard with a usage error. Without the option, the harness runs every selected case, as now.
- **CI matrix:** the `e2e` job runs once per tmux version and shard, with 4 shards, so 8 e2e jobs. Each runs `scripts/with-xvfb cargo test --locked --test e2e -- --shard K/4`. A failed shard uploads its output as `e2e-tmux-<v>-shard-<k>`.
- README.md and CLAUDE.md describe `--shard` and the sharded CI jobs.

## Capabilities

### New Capabilities

None. This is test tooling.

### Modified Capabilities

None (`skip_specs: true`): Brindle's behavior doesn't change.

## Impact

- `tests/e2e/main.rs` (argument parsing, case selection, self-test).
- `.github/workflows/ci.yml` (`e2e` matrix and artifact names).
- README.md and CLAUDE.md.
- CI runs 10 jobs instead of 4. A same-repo PR's push and `pull_request` runs together use all 20 concurrent jobs a free account gets.
