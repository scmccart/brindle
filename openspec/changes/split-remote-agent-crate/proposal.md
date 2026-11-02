# Proposal

Outline only, to expand into a full proposal after `remote-connections` lands.

## Why

The remote helper's code and its crate are in different places, and its protocol is compiled into two crates by path.

- `remote-agent/` holds the `brindle-remote-agent` crate, but its `src/main.rs` only parses `--version` and `--server`. The helper's implementation is in `src/remote/agent.rs`, under the `brindle` package. `brindle` never compiles it: `src/remote/mod.rs` declares only `client` and `protocol`.
- `remote-agent/src/main.rs` pulls in both `src/remote/agent.rs` and `src/remote/protocol.rs` with `#[path = "../../src/remote/…"]`, so `protocol.rs` is compiled twice, once by each crate.
- Each crate uses only part of `protocol.rs`, which is why it needs `#[allow(dead_code)]`, both on the agent's `mod protocol` and on items only one side uses. A real unused item can hide behind those.
- Someone reading `src/` sees helper code that the app never builds, and an edit to `protocol.rs` must compile for two crates without that being visible in either.

## What Changes

To be decided. A likely shape:

- A small shared crate for the wire protocol (frames, messages, limits, the preamble marker), used by both the app and the helper.
- The helper's implementation moved out of `src/` into its own crate, under `remote-agent/` or a new `crates/` directory.
- No more `#[path]` includes, and the `dead_code` allowances that only exist because of them removed.

Open questions:

- Where crates live: keep the root layout (`members = [".", "remote-agent"]`) or introduce `crates/`.
- Whether to rename `brindle-remote-agent`, and what that means for `scripts/install.sh`, `scripts/package-windows.ps1`, CI's helper build and the e2e harness, which use the crate and binary names.
- Whether the protocol crate stays private to the workspace (`publish = false`).

This must not change behaviour or the wire format: the helper binary, its `--version` and `--server` interface, and the protocol stay as they are.

## Capabilities

Probably none: this is a code-structure change with no behaviour change, so the full proposal may set `skip_specs: true`.
