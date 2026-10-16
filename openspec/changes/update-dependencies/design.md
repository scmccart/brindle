# Design

## Context

- **CI triggers** (`.github/workflows/ci.yml`): `push` to any branch, `pull_request`, and `workflow_dispatch`. Every job has `if: github.event_name != 'pull_request' || <the PR comes from a fork>`. Same-repo PRs were skipped because their branch's push already ran. Dependabot's branches live in this repository, so their PRs never ran, and they were opened before the workflow existed, so they have no push runs either. Concurrency is grouped per workflow and ref, with `cancel-in-progress`.
- **Landing:** changes reach `main` by a local `--no-ff` merge and a push, not by merging PRs on GitHub. GitHub still marks a PR merged when its head commit lands in the base branch.
- **Dependabot:** security updates are on (alerts and PRs). There is no `.github/dependabot.yml`, so there are no version updates.
- **Lockfile rule (CLAUDE.md):** `Cargo.lock` came from GPUI's own lockfile. Never regenerate it or run a broad `cargo update`; a newer `libc` breaks `xattr`. `cargo update --dry-run` would touch 364 packages, `libc` 0.2.177 → 0.2.190 among them.
- **The alerts**, and where each package comes from (`cargo tree -i`):

  | Alert | Package | In the lock | Fixed in | Reached through | Covered by |
  |---|---|---|---|---|---|
  | 1 | `bytes` | 1.10.1 | 1.11.1 | `gpui_http_client`, `h2`, `hyper` | PR #1 |
  | 3, 10 | `quinn-proto` | 0.11.13 | 0.11.15 | `quinn`, an optional dependency of `zed-reqwest` that isn't enabled: locked, never built | PR #2 |
  | 8 | `rand` | 0.8.5 | 0.8.6 | `num-bigint-dig` ← `oo7` ← `gpui` | PR #3 |
  | 4, 5, 6, 9 | `rustls-webpki` | 0.103.7 | 0.103.13 | `rustls` ← `hyper-rustls`, `zed-reqwest` | PRs #4, #5 |
  | 11 | `rustls` | 0.23.33 | 0.23.45 | `hyper-rustls`, `tokio-rustls`, `zed-reqwest` | PR #5 |
  | 7 | `rand` | 0.9.2 | 0.9.3 | `ashpd` 0.11 (`gpui`), `ashpd` 0.12 (`oo7`) | none |
  | 2 | `grid` | 0.18.0 | 1.0.1 (affected: 0.17.0–1.0.0) | `taffy` 0.9 ← `gpui` | none |

- **Merging the branches:** `git merge-tree` merges all five Dependabot branches onto `main` without conflicts, in PR order. The combined diff touches only `Cargo.lock`: 85 lines added and 20 removed. #2 adds `rand` 0.10.3, `rand_core` 0.10.1, `rand_pcg` 0.10.2, `chacha20` 0.10.2, `cpufeatures` 0.3.1, `getrandom` 0.4.3 and `r-efi` 6.0.0, the new `quinn-proto`'s dependencies. They're locked but never built. `libc` is untouched.
- **Already current** (2026-10-16): `gpui` 0.2.2 and `alacritty_terminal` 0.26.0 are the latest on crates.io. tmux 3.8 is the latest release. `actions/checkout` v7.0.1, `actions/cache` v6.1.0, `actions/upload-artifact` v7.0.2 and `Swatinem/rust-cache` v2.9.2 are covered by the major tags the workflow uses.
- **Rust:** 1.99.1 was released on 2026-10-13. It fixes the miscompilation of FFI `bool` return values on x86_64 (rust-lang/rust#163911), plus two fixes for other platforms. The desktop and CI are on 1.99.0.

## Goals / Non-Goals

**Goals:**
- Every PR gets the whole CI, the same as a push.
- Every alert that a semver-compatible lockfile update can fix gets fixed, without touching anything else in the lock.

**Non-Goals:**
- Broad `cargo update`, or major-version bumps of direct dependencies. `toml` 0.8 → 1.x, `toml_edit` 0.22 → 0.25, `dirs` 5 → 7 and `zbus` 5 → 6 are all available. None of them fixes an alert. `zbus` and `polling` are pinned to what GPUI builds, and the others would add new copies to the build.
- Dependabot version updates for Cargo, which would open PRs for exactly the broad updates the lockfile rule avoids.
- Making e2e a required check, or setting up branch protection.

## Decisions

### Triggers: PRs always; pushes on every branch except Dependabot's

```yaml
on:
  push:
    branches-ignore: ["dependabot/**"]
  pull_request:
  workflow_dispatch:
```

The jobs lose their `if:` conditions.
- A same-repo branch with an open PR then runs twice per push, once for `push` and once for `pull_request`, in different concurrency groups (`refs/heads/x` and `refs/pull/N/merge`). That's acceptable: Actions minutes are free for a public repository, a warm run takes about 27 minutes, and the usual workflow lands changes without PRs, so those runs are rare.
- Dependabot's branches always have a PR, so pushes to them would only duplicate runs. They're excluded.

Alternatives:
- *One concurrency group per branch for both events* (`github.head_ref || github.ref_name`). That removes the duplicate, but the push and `synchronize` events arrive together and cancel each other. That can leave a PR's check cancelled instead of green.
- *Push only on `main`.* Branches pushed without a PR, the usual way of landing, would lose CI before the merge.

Fork PRs keep running as before. `pull_request` gives them a read-only token and no secrets, and the workflow uses none.

### Merge the Dependabot branches as they are, in PR order

`git merge --no-edit origin/dependabot/cargo/<name>` for #1 to #5 on the change branch. Merging, rather than reapplying the bumps with `cargo update -p --precise`, puts each PR's head commit into `main`, so GitHub marks the PRs merged and their alerts close when `main` is pushed. The merges are conflict-free (checked with `git merge-tree`). If a merge does conflict after all, keep `main`'s `Cargo.lock`, run that PR's `cargo update -p <pkg> --precise <version>`, and commit that as the merge.

- *Alternative: merge the PRs on GitHub.* That needs five rebases and five CI runs of about 27 minutes each, and it would merge onto `main` outside the usual workflow.
- *Alternative: skip #2,* since `quinn-proto` is never built. Merging it is cheap, it closes two high-severity alerts that would otherwise stay open, and the extra entries in the lock cost nothing to build.

### `rand` 0.9.3 with one precise update

`cargo update -p rand@0.9.2 --precise 0.9.3`, after the merges. Verify that the lock diff changes only `rand`'s version and checksum.

### Rust 1.99.1 everywhere

- CI: `RUST_VERSION: "1.99.1"`.
- The desktop: `rustup update stable`. That's a change on the user's machine, so ask first.

The workflow comment says `RUST_VERSION` is the desktop's toolchain, and the two should stay equal.

### Dependabot version updates for Actions only

`.github/dependabot.yml`:

```yaml
version: 2
updates:
  - package-ecosystem: github-actions
    directory: /
    schedule:
      interval: monthly
```

With CI running on PRs, those bumps get tested before anyone merges them. Cargo isn't listed: security updates keep working without an entry, and version updates would fight the lockfile rule.

### The `grid` alert stays open

`taffy` 0.9 requires `grid` 0.18 (`^0.18`), and the fix is in 1.0.1, which is semver-incompatible. Only a GPUI release that moves `taffy` can bring it in. The overflow needs a grid layout with row counts large enough to overflow `usize`, and Brindle builds no CSS grid layouts. Leave the alert open as a reminder. Dismissing it is the user's call; ask before doing it.

## Risks / Trade-offs

- [A dependency bump changes behavior] → `bytes`, `rustls` and `rustls-webpki` sit on GPUI's HTTP client path. Brindle makes no HTTP requests of its own (no `http_client` or remote `img` use), so the risk is in building, not at runtime. The full CI (unit tests, release build, e2e on both tmux versions) runs on the combined branch before the merge.
- [Rust 1.99.1 changes codegen] → It's a point release with three targeted fixes. CI runs everything on it before the merge, and the desktop dogfooding build gets rebuilt after it.
- [The PR trigger doubles runs for same-repo PRs] → Accepted (see Decisions). If it becomes a problem, add `if: github.event_name != 'push' || ...` once a cheap way to see an open PR exists.
- [Dependabot opens PRs with Actions bumps that need a workflow change] → They're monthly, and CI on the PR shows whether they work.

## Migration Plan

1. Land on a branch the usual way, and open a PR from it before merging locally. That PR's `pull_request` run is how to check that same-repo PRs now get CI.
2. The local `--no-ff` merge and push to `main` close the five Dependabot PRs and the PR from step 1 as merged.
3. Rollback: revert the merge commit. Dependabot would then reopen PRs for the alerts.
