# Design

## Context

- `tests/e2e/main.rs` takes name filters (substring matches) plus `--list` and `--self-test`, and runs the selected cases one after another. Each case has its own throwaway tmux server (`brindle-e2e-<pid>-<n>`) and its own directory in `target/tmp/e2e/<case>/`, so cases don't share state. Display cases share the one X display that `scripts/with-xvfb` provides.
- From run 38940957233 (`main`, tmux 3.6a, `BRINDLE_E2E_SLOW=2`): the 100 cases total 1448 s, from 0.7 s to 63.9 s each. Everything in an e2e job besides the suite (setup, apt packages, toolchain, cache restore and the cached build) takes about 55 s.
- Runners have 4 vCPUs, and lavapipe renders on the CPU. A free account runs up to 20 jobs at once.

## Goals / Non-Goals

**Goals:**
- Cut the e2e wall time to under 10 minutes, without changing what runs or how a case is timed.
- Keep shards deterministic, so a failed shard can be rerun locally exactly.

**Non-Goals:**
- Running cases in parallel within one job.
- Balancing shards by measured durations, or making e2e a required check.

## Decisions

### Split across jobs, not inside one

Each shard is its own matrix job with its own runner and its own Xvfb, so a case runs exactly as it does now, with the whole machine to itself.

*Alternative: several harness processes in one job,* each under its own `scripts/with-xvfb`. They would compete for 4 vCPUs and lavapipe, which brings back the timing flakes that `BRINDLE_E2E_SLOW` and the recent harness fixes dealt with, for a smaller saving.

### Round-robin by position, after filters

Shard K of N runs the selected cases at 0-based positions i with `i % N == K - 1`. The registration order in `cases::all()` groups cases by area, and dealing them out round-robin spreads each area, and its typical duration, across the shards. With the run above and N = 4, the shards total 368, 330, 382 and 367 s. A contiguous split would put whole areas, such as all the restore or all the geometry cases, in one shard.

Sharding after filters means `-- restore- --shard 2/2` splits just the restore cases. The harness prints `e2e: shard K/N, M of T cases` before running, so a log says which cases it covered. `K` must be 1 to N and `N` at least 1. Anything else is a usage error (exit 2), not a silently empty run.

*Alternative: balance by measured durations.* That's better in theory, but it needs a timing table to keep up to date as cases are added. Round-robin is already within about 15% of an even split.

### Four shards

| Shards | Longest shard (s) | e2e job (min, with setup) | Jobs per run |
|---|---|---|---|
| 1 (now) | 1448 | ~25 | 4 |
| 4 | 382 | ~7.5 | 10 |
| 6 | 269 | ~5.5 | 14 |

Four shards bring a run to about 7.5 minutes. A same-repo PR's two runs (push and `pull_request`) are then 20 jobs together, exactly the free account's concurrency limit, so nothing queues. Six shards would save two more minutes, but those two runs would be 28 jobs, and the extra jobs would wait for a free slot. The shard count is the matrix's `shard` list plus the `N` in the command, both in `ci.yml`.

### Matrix and artifacts

```yaml
matrix:
  tmux: [...]        # the existing include entries stay as they are
  shard: [1, 2, 3, 4]
```

The existing `include` entries (tmux version, `reports`, `sha256`) become the `tmux` axis, crossed with `shard`. Job names read `e2e (tmux 3.6, 2/4)`. Artifacts are `e2e-tmux-<v>-shard-<k>`, because artifact names must be unique within a run. The tmux 3.8 build cache is keyed by version, so the four 3.8 shards share one cache entry. The first run builds tmux in each shard, about 16 s, and the shards race to save the cache; one of them wins.

## Risks / Trade-offs

- [More runner minutes] → Each run spends about 3.5 extra minutes of setup in total (8 jobs instead of 2, about 55 s each), but less wall time. Minutes are free for a public repository.
- [The concurrency limit is reached] → A same-repo PR's two runs use all 20 slots. If a third run starts at the same time, its jobs wait for a free slot; that's slower, but nothing fails.
- [One slow case dominates a shard] → The longest case is 64 s, under a fifth of a shard. If shards drift apart as cases are added, balance by duration then.
- [A case depends on an earlier one] → Cases are independent by construction (own tmux server, own directory), and the suite already runs single cases by filter. The first sharded runs show whether that holds.
