# Proposal

## Why

The remote helper's code and its crate are in different places, and its wire protocol is compiled into two crates by path include.

- `remote-agent/` holds the `brindle-remote-agent` crate, but its `src/main.rs` only parses `--version` and `--server`. The helper's implementation is `src/remote/agent.rs`, under the `brindle` package, which never compiles it: `src/remote/mod.rs` declares only `client` and `protocol`.
- `remote-agent/src/main.rs` pulls in `src/remote/agent.rs` and `src/remote/protocol.rs` with `#[path = "../../src/remote/…"]`, so `protocol.rs` is compiled twice, once by each crate, and its unit tests run twice.
- Each crate uses only part of `protocol.rs`. That is why the agent's `mod protocol` carries `#[allow(dead_code)]`, and why five items in `protocol.rs` (`MAX_IDLE_LIMIT_SECS`, `validate_idle_limit`, `Frame::spawn_response`, `Frame::process_info`, `Frame::error`) carry `#[allow(dead_code)] // Used by the agent only.` A genuinely unused item can hide behind those.
- Someone reading `src/` sees helper code the app never builds, and an edit to `protocol.rs` has to compile for two crates without either showing it.

## What Changes

- New workspace crate `brindle-remote-protocol` in `remote-protocol/` (`publish = false`), holding `src/remote/protocol.rs` unchanged as its `lib.rs`: frame header and limits, message types, heartbeat and idle limits, the frame-stream marker and preamble skipping, and the JSON message structs. Its tests move with it.
- The helper's implementation moves from `src/remote/agent.rs` to `remote-agent/src/agent.rs`, an ordinary module of the `brindle-remote` binary, and imports the protocol crate instead of `super::super::protocol`.
- `brindle` depends on `brindle-remote-protocol` by path and re-exports it as `remote::protocol`, so `client.rs` keeps its imports.
- The `#[path]` includes and every `#[allow(dead_code)]` that exists only because of them are removed.
- The crate name `brindle-remote-agent`, the binary name `brindle-remote`, its `--version`/`--server` interface, the wire format and the helper's behaviour do not change. `scripts/install.sh`, `scripts/package-windows.ps1`, CI, the README's musl build command and the e2e harness keep working as written.
- `CLAUDE.md`'s architecture notes say where the helper and protocol now live.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

None. This is a code-structure change with no behaviour change, so the change sets `skip_specs: true`.

## Impact

- Code: `src/remote/mod.rs`, `src/remote/protocol.rs` (moved), `src/remote/agent.rs` (moved), `remote-agent/src/main.rs`, new `remote-protocol/`.
- Build: root `Cargo.toml` (`[workspace] members`, a path dependency), `remote-agent/Cargo.toml`, and `Cargo.lock`, which gains one path package and no new registry packages.
- Docs: `CLAUDE.md` architecture section.
- No change to config, keybindings, the deployed helper's cache path or version check, or any spec.
