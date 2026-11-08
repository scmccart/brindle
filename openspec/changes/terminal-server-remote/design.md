# Design

**Proposal-stage notes.** Phase 1's design (`openspec/changes/archive/2026-11-07-terminal-server/design.md` once PR #22 merges) sketched the transport:

```
   local Linux   : unix socket                -> server on this machine
   local Windows : named pipe                 -> server on this machine (phase 3)
   SSH Linux     : ssh relay -> unix socket   -> server on that host
   SSH Windows   : ssh relay -> named pipe    -> server on that host
   container     : exec relay -> unix socket  -> server in the container
```

## Points to settle

- **Liveness over a relay:** local sockets need no heartbeat, because EOF is reliable. Over SSH it isn't. The relay or protocol needs a heartbeat. Decide whether it ends sessions (`kill`) or only marks the connection lost and keeps the reconnect grace running.
- **Version skew:** the helper is cached by Brindle's version and digest, so an older Brindle meets a newer remote core, or the reverse. Phase 1's rules apply: the core speaks every older protocol, and the client offers an upgrade. Confirm that the deploy cache and the upgrade's `check` work together on remote hosts.
- **The upgrade decision on a remote host** compares the core binary's mtime through `/proc/<pid>/exe` locally. Remotely it has to go through the relay, or through `WELCOME` fields.
- **Eager connect** has to stay off the UI thread and must not slow startup. Unreachable hosts back off.
- **Remote tmux control mode** (`tmux -C` over pipes) stays as it is, outside the server.
