# Tasks

## 1. CI on every pull request

- [x] 1.1 In `.github/workflows/ci.yml`, set `push` to `branches-ignore: ["dependabot/**"]`, keep `pull_request` and `workflow_dispatch`, and remove the three jobs' fork-only `if:` conditions. Update the comment above the triggers. Verify: actionlint passes, ignoring only the unknown `ubuntu-26.04` label.
- [x] 1.2 Add `.github/dependabot.yml` with monthly `github-actions` version updates and no Cargo entry, as in design.md. Verify: it parses as YAML, and after the merge, GitHub's Dependabot page (Insights → Dependency graph → Dependabot) lists the `github-actions` ecosystem without errors.
- [x] 1.3 Update README.md's and CLAUDE.md's CI notes: CI runs on every push except to Dependabot's branches, and on every PR. Dependabot opens monthly PRs for Actions; Cargo gets security PRs only. Verify: neither file still says PRs only run for forks.

## 2. Dependency updates

- [x] 2.1 Merge `origin/dependabot/cargo/{bytes-1.12.1,quinn-proto-0.11.19,rand-0.8.8,rustls-webpki-0.103.15,rustls-0.23.45}` in that order with `git merge --no-edit`. On a conflict, follow design.md: keep `main`'s `Cargo.lock` and apply the PR's `cargo update -p <pkg> --precise <version>`. Verify:
  - `git diff main -- Cargo.lock` changes only the packages listed in design.md, and `libc` is untouched;
  - `cargo build --locked` and `cargo test --locked` pass.
- [x] 2.2 Run `cargo update -p rand@0.9.2 --precise 0.9.3`. Verify: the lock diff for this step is only `rand` 0.9's version and checksum, and `cargo test --locked` passes.
- [x] 2.3 Set `RUST_VERSION: "1.99.1"` in the workflow. Ask the user before running `rustup update stable` on the desktop. Verify: `rustc -V` prints 1.99.1, and `cargo build --release --locked` and `cargo test --locked` pass with it.
- [x] 2.4 Run the e2e suite headless on the desktop: `BRINDLE_E2E=1 scripts/with-xvfb cargo test --locked --test e2e` (the desktop has Xvfb, openbox and lavapipe, and nothing opens on its display). Verify: 100 of 100 pass.

## 3. Verify on GitHub and land

- [x] 3.1 Push the branch, and open a PR from it to `main`. Verify:
  - the push starts one `push` run;
  - the PR starts one `pull_request` run with all four jobs (the PR's checks list them);
  - both runs pass, with `rustc 1.99.1` in their logs.
- [x] 3.2 Land the usual way: archive commit, rebase on `main`, `--no-ff` merge "Merge updating dependencies and running CI on every PR", push `main` (ask first). Verify:
  - PRs #1–#5 and the PR from 3.1 show as merged;
  - `gh api repos/scmccart/brindle/dependabot/alerts` shows only alert #2 (`grid`) open;
  - the run on `main` passes.
- [x] 3.3 Ask whether to dismiss the `grid` alert (#2) as "tolerable risk", with design.md's reasoning as the comment, or to leave it open. Do what the user decides. Delete the merged branches on GitHub. Verify: `git ls-remote --heads origin` lists only `main`.
- [x] 3.4 Offer to reinstall the dogfooding build (`scripts/install.sh`), now built with Rust 1.99.1 and the updated lock.
