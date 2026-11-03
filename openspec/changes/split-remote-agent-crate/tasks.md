# Tasks

## 1. Protocol crate

- [ ] 1.1 `git mv src/remote/protocol.rs remote-protocol/src/lib.rs`; add `remote-protocol/Cargo.toml` (`brindle-remote-protocol`, `0.2.0`, edition 2024, `MIT OR Apache-2.0`, `publish = false`, `serde` with `derive` and `serde_json = "1.0.145"`) and a `//!` crate doc; remove its five `#[allow(dead_code)] // Used by the agent only.` lines. Verify `cargo test -p brindle-remote-protocol` runs its 10 tests with no warnings.
- [ ] 1.2 Add `remote-protocol` to `[workspace] members` and `brindle-remote-protocol = { path = "remote-protocol" }` to `brindle`'s dependencies; replace `pub mod protocol;` in `src/remote/mod.rs` with `pub use brindle_remote_protocol as protocol;`. Verify `cargo build -p brindle` and `cargo test -p brindle remote::` pass with no new warnings.

## 2. Agent crate

- [ ] 2.1 `git mv src/remote/agent.rs remote-agent/src/agent.rs`; in `main.rs` drop both `#[path]` attributes, the `mod protocol` and its `#[allow(dead_code)]`; in `agent.rs` import from `brindle_remote_protocol` instead of `super::super::protocol`. Verify `grep -rn '#\[path' remote-agent src` finds nothing.
- [ ] 2.2 In `remote-agent/Cargo.toml` replace `serde`/`serde_json` with `brindle-remote-protocol = { path = "../remote-protocol" }`, keeping `libc`. Verify `cargo test -p brindle-remote-agent` runs the agent's 5 tests with no warnings, and `target/debug/brindle-remote --version` prints `0.2.0`.

## 3. Lockfile and docs

- [ ] 3.1 Check `git diff Cargo.lock`: only the new `brindle-remote-protocol` package and the dependency edges of `brindle` and `brindle-remote-agent` change, and no registry package is added, removed or moved. Verify `cargo build --locked --workspace` passes.
- [ ] 3.2 Update `CLAUDE.md`'s `remote/` / `brindle-remote` architecture entry to say the protocol is the `brindle-remote-protocol` crate (`remote-protocol/`, re-exported as `remote::protocol`) and the helper's implementation is `remote-agent/src/agent.rs`. Verify `grep -rn 'src/remote/agent.rs\|src/remote/protocol.rs' CLAUDE.md README.md scripts .github` finds nothing.

## 4. Integration checks

- [ ] 4.1 Run `cargo test --workspace --locked` and `cargo clippy --workspace --all-targets` (if clippy is installed) and confirm no new warnings, and that the protocol tests run once rather than twice.
- [ ] 4.2 Build the static helper with the README's Docker musl command (`-p brindle-remote-agent --target x86_64-unknown-linux-musl`) and run `BRINDLE_REMOTE_DOCKER_TEST=1 cargo test -p brindle --locked remote::client`; if Docker isn't available, record that CI's `release` job covers the musl build.
- [ ] 4.3 Run `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- remote-` (after `cargo build -p brindle-remote-agent`) and confirm the remote cases pass or skip for the same reasons as on `main`.

## Workflow follow-up

- Commit as `refactor(remote): …`, archive with `docs(openspec): …`, then land per the usual branch, rebase and `--no-ff` merge workflow.
