# Proposal

## Why

Local tabs on Linux and Windows run in the terminal server and survive a quit, a crash or an upgrade. Remote profiles (SSH and containers) still run one stdio `brindle-remote --server` per tab, which ends its session when the tab closes or the client goes quiet. So remote tabs don't persist, every tab or split costs its own SSH or exec connection, and Brindle can't show a host's sessions before you open a tab there. The server already runs on Linux, so it can run on the remote host too.

## What Changes

- **Remote tabs run in the host's terminal server.** `brindle-remote relay`, started over `ssh` or `docker`/`podman exec`, connects the stdio link to the host's core and starts the core when none runs. Brindle speaks protocol 1 over the relay, so remote tabs use the same server backend as local ones.
- **One connection per host** instead of one per tab, multiplexed by session id. On the warm path a connection costs one SSH or exec round trip, and containers keep one cached helper instead of a copy per tab.
- **Remote tabs persist like local ones.** Quitting or a crash leaves them running detached on the host, closing a tab ends its program, and the next launch reattaches them with their screens.
- **Lost connections reconnect.** A heartbeat on the relay link detects a dead link on both sides. Brindle keeps reconnecting with backoff while the host has open tabs, and gets each session back with its replay. A session is lost only when the reconnected host no longer has it.
- **Command (`-e`) sessions survive short drops.** A relayed client can ask the core to wait up to 5 minutes for it to come back before its `kill` sessions end.
- **A reconnecting client replaces its old connection.** A half-dead relay that still holds the client's sessions no longer blocks the reconnect.
- **Titles and directories are pushed.** The host's core reports each remote session's foreground process name and directory when they change (client protocol 2), replacing a round trip per tab every second.
- **Eager profiles:** `remote.connect_at_launch = true` connects to that host in the background at launch, attaches its detached sessions, and lists them in "Attach Session…". An unreachable host shows as offline there and never blocks startup. Containers are still never started.
- **Every connected host's detached sessions come back at launch,** whoever started them, as local detached sessions do now.
- **A tab stays on the host its sessions run on** when its profile's `remote` changes. New tabs follow the new target.
- **Hardening folded in from `harden-terminal-server`:**
  - session programs start with default signal dispositions and an empty mask (#44);
  - a holder drains a dead core's socket before dropping it;
  - a role's ready byte can't be mistaken for an exec error.
- **BREAKING (internal):** the stdio `--server` agent, its per-tab protocol and the idle-limit requirement are removed. Older Brindles deploy their own helper version, so they keep working.
- Windows SSH hosts are out of scope: remote hosts stay Linux.

## Capabilities

### New Capabilities
- None.

### Modified Capabilities
- `remote-connections`: the relay transport, one connection per host, persistence, reconnecting, eager connection and offline hosts, the pinned host and the cached container helper. The idle-limit requirement is removed.
- `terminal-server`: the relay, connection takeover, the linger for relayed clients, pushed process info, the relay's upgrade offer, servers that outlive the SSH login that started them, and default signal handling for programs.
- `session-restore`: remote zones record and reattach their sessions, restore without waiting for hosts, and launch reattaches detached sessions of the hosts it connects to.
- `tabs`: quitting keeps remote terminals running too.
- `zones`: closing ends remote programs too, and splits stay on the tab's host.
- `command-palette`: the sessions palette lists every connected host's sessions, grouped by host, with offline hosts shown.
- `profiles`: the remote target gains `connect_at_launch`.
- `settings-dialog`: the profile editor's remote fields gain "Connect at launch".

## Impact

- `remote-agent/`:
  - `relay` subcommand (new);
  - core: takeover, linger, process sampling;
  - holder: signal reset and draining a dead core;
  - daemon ready pipe;
  - `agent.rs` and `--server` removed.
- `remote-protocol/`:
  - relay link frames;
  - client protocol 2 (`PROCESS`) and additive `HELLO` fields;
  - the upgrade decision, shared by Brindle and the relay;
  - the stdio session messages removed.
- `src/remote/`:
  - `local` becomes per-host (a host key replaces the singleton `Endpoint`), with a child-process transport for relays;
  - `client.rs` keeps deployment only.
- `src/terminal/mod.rs`: `Backend::Agent` is removed, and remote tabs use `Backend::Server`.
- `src/session.rs`, `reattach.rs`, `main.rs`, `workspace.rs`: remote zone sessions, asynchronous remote restore and reattach, eager hosts.
- `src/picker.rs`: the sessions palette by host. `src/config.rs`, `config_edit.rs`, settings dialog: `connect_at_launch`.
- Tests:
  - `remote-agent/tests/{relay,core,holder}.rs`;
  - the `BRINDLE_REMOTE_DOCKER_TEST` transport test;
  - the `remote-*` e2e cases.
- Docs: README (remote profiles) and CLAUDE.md (architecture).
- Sibling changes: this absorbs `remote-connections-followups` entirely and items 1, 4 and 5 of `harden-terminal-server`.
- Issues: closes #44, and #46 items 2 and 3.
