## Why

Remote profiles (SSH and container) can only be created by editing `config.toml`; the settings dialog's profile editor knows nothing about `remote`. Separately, a remote tmux control session that loses its connection can leave the tmux client attached on the remote server, because the drop is only noticed by ssh's keepalive (SSH only) and nothing tells the remote tmux to let go.

## What Changes

- The profile editor in the settings dialog gains a "Remote" field (`none`, `ssh`, `container`) and the fields of the chosen kind: SSH host, user, port, identity file and extra ssh arguments; container engine, container and user. Saving writes the profile's `remote` table, and blank fields are written as absent keys.
- Invalid remote input (empty host or container, port outside 1-65535) blocks saving and is marked in the editor.
- A remote control-mode session detects a dead connection (a stalled client as well as a closed one), ends it with a distinct reason, and makes a bounded, best-effort attempt to detach its own tmux client on the remote server, so the session isn't left held by a ghost client.
- The dead session reports `Detached` with a "connection lost" reason, so its tabs close or show as ended as for any detach.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `settings-dialog`: the profile editor edits the remote target.
- `tmux-control-mode`: a dropped remote connection is detected and the remote client detached.

## Impact

- `src/settings_dialog/profiles.rs` (fields, values, validation, writing), `src/config_edit.rs` (write the `remote` table), `src/settings_dialog.rs` (rendering).
- `src/tmux/mod.rs` (heartbeat, deadline, drop reaction), `src/remote/mod.rs` (one-shot detach command).
- README settings section; no config format change, so nothing is **BREAKING**.
