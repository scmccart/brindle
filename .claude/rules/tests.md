---
paths:
  - "tests/**"
  - "remote-agent/tests/**"
  - "scripts/with-xvfb"
---
# Tests: the opt-in suites, the real-process tests and the architecture test

You are editing a test harness or its cases. Read `docs/architecture/testing-architecture.md` before adding a kind of test, fixing a flake, or changing a harness.

- Drive Brindle through its debug hooks (`--send`, `--action`, `--key`, `--wait-for`, `--dump-screen-after`), never synthetic OS input (`testing R2`).
- Every case is hermetic: its own config, state and server directories, artifacts kept, processes cleaned up; never the user's config, session or server (`testing R4`).
- Windows on the user's desktop need asking first; headless runs use `scripts/with-xvfb` (`testing R5`).
- No fixed sleeps or padding: wait for the condition, scaled by `BRINDLE_E2E_SLOW` (`testing R7`).
- Assert the rule, not a count that depends on timing; expected colours come from the `e2e` theme (`testing R6`, `R8`).
- A flake is fixed at its cause, with failures over N runs before and after. No retry, skip or longer wait on passing runs (`testing R9`, `R10`).
- A GUI check is a named case in `tests/e2e/cases/`, never a one-off script (`testing R13`). The perf suite reports numbers and never fails (`testing R14`).
- Test code may pair platforms with `cfg(unix)` and `cfg(windows)` (`testing`'s "Platform pairs"): `ports R4` binds production code only.
- A harness's pure helpers (its parsers, matchers and the like in `tests/e2e/`, `tests/perf/`) are tested in that harness's `--self-test`, since a `harness = false` target runs no `#[test]`: parsers MUST be, other pure helpers SHOULD be, and `fcis R8` binds production code only (`testing R3`).
- `tests/architecture.rs` encodes `docs/architecture` rules: change a list there only together with the rule in its doc, never to make a change compile.

If a rule can only be reached through an e2e case, it probably lives in a view; move it into a pure function or model and unit-test it there (`testing R1`).

Enforced by: the harness (stale or missing helper, leftover roles), both `--self-test`s in CI, and `the_opt_in_suites_are_harness_false_targets` and `each_opt_in_runner_has_its_variable_list_and_self_test` (each runner names its variable, `--list` and `--self-test`) in `tests/architecture.rs`.
