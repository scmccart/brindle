# Proposal

**Status: proposal stage.** This is phase 4 of the terminal-server series. It needs phase 1 (`terminal-server`, PR #22) merged first. Phase 2 (`terminal-server-on-close`) should come before it, so that remote sessions can persist. It has no specs or tasks yet.

## Why

Remote profiles (SSH and containers) still run one stdio `brindle-remote --server` per tab. That helper ends its session when the tab closes or the client goes quiet, because of the idle limit and heartbeat. So remote tabs don't persist, every tab is a new SSH or exec connection, and Brindle can't show a host's sessions before you open one. The server built in phase 1 can run on the remote host too.

## What Changes

- **A relay to the host's server.** `brindle-remote --attach` relays between ssh or `docker`/`podman exec` stdio and the host's core socket, starting the server there if none is running. Brindle talks protocol 1 over the relay, so remote tabs use the same backend as local ones.
- **One connection per host** instead of one per tab, multiplexed by session id.
- **Eager profiles:** profiles can opt in to being connected at launch. Brindle connects in the background, ensures the server is running, and lists its sessions. An unreachable host shows as offline and never blocks startup. A container profile may start its container. A container's server lives only as long as the container.
- **Windows SSH hosts:** a PowerShell deploy path for `brindle-remote.exe`, which needs phase 3.
- **Persistence and close behaviour** come from phase 2's `on_close`, applied to remote profiles too.

## Relation to `remote-connections-followups`

This change absorbs or supersedes three of that change's follow-ups:
- follow-up 2 (lifecycle polling): the server replaces the per-tab cancel watcher, writer and `try_wait` loops;
- follow-up 3 (an `Endpoint` type): the transport-agnostic server connection is the `Endpoint`;
- follow-up 4 (connection cost): one connection per host, and a long-lived helper instead of per-tab container copies.

It also replaces the `remote-connections` requirement "A vanished client ends the remote session". The reconnect grace and close policies take its place.

## Capabilities

### Modified Capabilities
- `remote-connections`: the transport, persistence, eager connection and offline state. The idle-limit requirement is removed.
- `profiles` and `configuration`: the eager option.
- `terminal-server`: the relay, and remote server lifetime.
- `session-restore`: remote session ids, with the existing remote target identity.

## Impact

- `src/remote/client.rs`: deployment stays (version-and-digest cache), and the per-tab stdio session becomes a per-host relay connection shared through `remote::local`'s client.
- `remote-agent/`: the `--attach` relay, remote server start, and the Windows deploy.
- Tests: the Docker-based `remote-*` e2e cases and `BRINDLE_REMOTE_DOCKER_TEST` grow to cover relay reconnects, eager listing and offline hosts.
