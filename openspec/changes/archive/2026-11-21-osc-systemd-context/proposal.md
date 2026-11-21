# Proposal

## Why

systemd 257 and later tell the terminal when its output moves into another context: `run0` (elevation), `systemd-nspawn` and `machinectl` (containers and VMs), ssh logins with systemd on the far side, and others. They do it with OSC 3008, the UAPI group's hierarchical context signalling (UAPI.15). Brindle ignores it, so nothing on screen says that a zone is now running as root or inside a container. That is exactly when a mistyped command costs the most. Few terminals act on OSC 3008 yet, so this makes the most of Brindle's Linux focus and its zones.

## What Changes

- Recognise `ESC ] 3008 ; start=<id> [; field=value …] ST` and `ESC ] 3008 ; end=<id> [; …] ST` in every terminal, local or remote.
- Keep each terminal's context stack: starting a new id nests it under the active context, starting a known id updates it and ends its subcontexts, and ending an id ends it and everything beneath it. Depth and field sizes are limited, and malformed input is skipped.
- Show a context badge on the tab face and on the zone header for the innermost notable context in the active chain: elevation and privilege changes (as the target user, in red), and remote hosts, containers and VMs (by name, muted).
- Keep the stack in the terminal server, so the badge survives detach, attach and core restarts.
- Print the stack from `--dump-screen-after` (`--- context …`), and add e2e cases.

Out of scope: acting on `command` contexts (failed-command marks, durations; that belongs with `add-blocks`), menus that kill a context's processes or open a shell in its directory, tinting output, and clearing contexts whose `end` never arrived because a connection dropped.

## Capabilities

### New Capabilities
- `program-contexts`: recognising OSC 3008, the per-terminal context stack, its limits and its lifetime.

### Modified Capabilities
- `tabs`: a tab face shows its active zone's context badge.
- `zones`: a zone header shows that zone's context badge in a multi-zone tab.
- `terminal-server`: a session's context stack survives detach, attach and core restarts.

## Impact

- A new pure `term-core/src/context.rs`, plus `term-core/src/osc.rs` (`Found::Context`), `emulator.rs` and `replay.rs`.
- `src/terminal/mod.rs` (the stack, its lifetime, clearing at replay start), `src/workspace.rs` and `src/zone_view.rs` (badges), the dump.
- `tests/e2e/cases/` (a new `context` case set), `docs/code-map.md`, README.md.
- No wire change and no new dependency. Old clients and old cores ignore OSC 3008.
