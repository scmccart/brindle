# Design

## Context

See proposal.md for why. Current layout:

- Workspace `members = [".", "remote-agent"]`, no `[workspace.dependencies]`.
- `src/remote/protocol.rs` (529 lines, 10 tests) uses only `std`, `serde` and `serde_json`. `client.rs` imports it as `super::protocol::…`; the agent as `super::super::protocol::…` (it sits inside `mod platform`).
- `src/remote/agent.rs` (1190 lines, 5 tests) uses only `std`, `libc` and the protocol, and is Linux-only behind `cfg(target_os = "linux")`, with a stub that returns `Unsupported` elsewhere.
- `remote-agent/` is built for the host in dev and e2e (`cargo build -p brindle-remote-agent`) and for `x86_64-unknown-linux-musl` in CI's `release` job and the README's Docker command. The helper reports `env!("CARGO_PKG_VERSION")` of its own crate; the client compares it with `brindle`'s. Both are `0.2.0`.
- CI and scripts name the package `brindle-remote-agent` and the binary `brindle-remote`.
- `Cargo.lock` must not be regenerated or broadly updated (CLAUDE.md); CI builds with `--locked`.

## Goals / Non-Goals

**Goals:**
- Each source file is compiled by exactly one crate, and lives in that crate's directory.
- No `#[path]` includes, and no `dead_code` allowances that exist only to cover the other crate's use.
- Every existing command (`cargo build -p brindle-remote-agent`, the musl build, `scripts/install.sh`, `cargo test --workspace`) keeps working unchanged.

**Non-Goals:**
- Renaming crates or binaries, or moving to a `crates/` directory.
- Sharing the version through `[workspace.package]` or adding `[workspace.dependencies]`. Versions stay hand-matched, as today.
- Any edit to protocol or agent logic beyond import paths and the removed allowances, including the `remote-connections-followups` items.
- Splitting the agent into a library plus a thin binary.

## Decisions

**A protocol crate beside the agent crate, at the root.** `remote-protocol/` holds `brindle-remote-protocol`, next to `remote-agent/`, and `members` becomes `[".", "remote-agent", "remote-protocol"]`. A `crates/` directory would move `remote-agent/` too and touch every script, CI step and doc that names its path, for no gain with two small crates. Alternative considered: put the protocol inside `remote-agent` as a lib target that `brindle` depends on. That makes the app depend on the helper's package (and its `libc` dependency and binary target) just to get message types, and blurs which side owns what.

**The protocol file moves as is.** `src/remote/protocol.rs` becomes `remote-protocol/src/lib.rs` with `git mv`, so history follows it. Its items are already `pub`; in a library crate unused `pub` items aren't `dead_code`, so the five allowances go without other changes. The crate gets a short `//!` doc, `publish = false`, `edition = "2024"`, `version = "0.2.0"`, `license = "MIT OR Apache-2.0"`, and depends on `serde` (derive) and `serde_json` at the requirements the other crates already use, so the lockfile resolves to the same registry versions.

**`brindle` re-exports it as `remote::protocol`.** `src/remote/mod.rs` replaces `pub mod protocol;` with `pub use brindle_remote_protocol as protocol;`. `client.rs` and its tests keep `super::protocol::…`, which keeps this change's diff to moves and manifests. Alternative: rewrite each import to `brindle_remote_protocol::…`; more churn, no benefit.

**The agent becomes a module of the helper binary.** `src/remote/agent.rs` moves with `git mv` to `remote-agent/src/agent.rs`. `main.rs` declares `mod agent;` without `#[path]` and drops `mod protocol`. In `agent.rs`, `use super::super::protocol::{…}` becomes `use brindle_remote_protocol::{…}`. Its `#[cfg(test)]` tests run as the binary's unit tests under `cargo test -p brindle-remote-agent`, as they do now.

**Lockfile changes by hand-checkable addition only.** Adding a path crate adds one `[[package]]` entry for `brindle-remote-protocol` and a dependency line in `brindle` and `brindle-remote-agent`. The change is made by a normal `cargo build` (not `cargo update`), and the diff is checked to contain nothing else. `agent.rs` never names `serde` itself, so `remote-agent/Cargo.toml` replaces its `serde`/`serde_json` dependencies with the protocol crate and keeps `libc`.

## Risks / Trade-offs

- [Cargo rewrites unrelated lockfile entries] → build without `--locked` once, inspect `git diff Cargo.lock`, and revert anything beyond the new package and its dependency edges; then confirm `cargo build --locked` passes.
- [The musl build breaks because the new crate pulls something non-static] → it depends only on `serde`/`serde_json`, already in the helper. Verify with the README's Docker musl command, or leave it to CI's `release` job, which runs it on push.
- [Version skew: three crates now carry `0.2.0` by hand] → same as today for two; the client's version check catches a mismatched helper at runtime. Unifying versions is a separate decision (Non-Goals).
- [Windows: `brindle` now builds a separate crate] → it already compiled the same file; `cargo test --workspace` on Windows already builds `remote-agent`'s non-Linux stub. CI's `windows` job confirms.

## Migration Plan

No user-facing migration. Rollback is reverting the commit.
