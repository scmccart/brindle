# Proposal

## Why

Today, Brindle only supports remote Linux machines or containers through classic tmux (`plain` mode) or manual SSH commands inside a local PTY. This makes remote support second-class:
- No native tmux control-mode (`tmux -C`) integration across remote hosts or containers.
- No direct remote process/title polling (showing active foreground command like `vim` or `cargo`).
- Fragile or missing CWD tracking across remote sessions.
- Inability to seamlessly spawn, split, and manage native local zones on remote endpoints without full-screen terminal wrappers.
- Modern developer tools (such as VS Code Remote and Zed Remote Server) deliver first-class remote workflows by deploying a lightweight remote agent daemon or communicating over structured RPC channels via SSH or container exec.

Providing first-class remote Linux host and container connections turns remote support from incidental to first-class, with full parity with local tabs and tmux integration while keeping rendering completely native and untouched.

## What Changes

1. **Remote Target Configuration:**
   - Extend `Profile` configuration with an optional `remote` definition:
     ```toml
     [[profiles]]
     name = "A8 Ubuntu"
     remote = { type = "ssh", host = "A8" }
     # or
     # remote = { type = "container", engine = "docker", container = "devbox" }
     ```
   - Support SSH targets (`host`, optional `user`, `port`, `identity_file`, `ssh_args`) and container targets (`engine = "docker" | "podman"`, `container`, `user`).

2. **Headless Remote Agent (`brindle-remote`):**
   - Provide a lightweight, headless remote agent binary or embedded bootstrap script/executable for Linux targets.
   - Bootstrapping mechanism:
     - SSH: Connect via SSH, probe or upload/cache the version- and digest-matched agent in `$HOME/.cache/brindle/<version>-<digest>/`, and execute `--server` over stdio.
     - Container: Copy the helper to a unique root-owned `/tmp` path, execute it via Docker/Podman exec, and remove it when the connection ends.
   - Agent features:
     - One supervised session worker per helper process, supporting terminal PTY spawning, process inspection (`/proc`), and bounded shutdown.
     - A bounded binary framing protocol over stdio for PTY I/O, resizing, child exit, and status signals; JSON is confined to bounded control-frame payloads and never placed in process arguments.
     - CWD and title tracking via Linux `/proc` and OSC 7 notifications.
     - The helper can also run `tmux -C` itself, but Brindle does not use that: remote tmux control mode runs `tmux -u -C` directly over the SSH/exec pipes (no helper) and feeds Brindle's native `TmuxSession` and `TmuxWindowView`.

3. **Client-Side Remote Subsystem (`src/remote/`):**
   - `Transport`: manages SSH or container exec child process pipes, handshake, and message framing.
   - `RemotePty`: implements terminal session creation, feed/input pipes, resize commands, and process status callbacks.
   - Remote tmux: `remote::tmux_command` builds the `tmux -C` process over SSH/container-exec pipes (no helper), so Brindle's `TmuxSession` attaches natively.

4. **Full Parity with Local Terminal Tabs:**
   - Bidirectional PTY I/O with raw byte throughput.
   - Window/Grid resizing via remote `TIOCSWINSZ` / pseudoterminal resize.
   - Process tracking: queries remote foreground process name from remote `/proc` for automatic tab and zone titles.
   - CWD tracking: returns remote current working directory (via remote `/proc` and OSC 7) so new tabs or zone splits inherit the remote directory.
   - Desktop notifications (OSC 9 / OSC 777) and program status (OSC 7501) passed through unchanged to Brindle's `Scanner`.
   - Native tmux control mode (`TmuxMode::Control`) working over SSH/container-exec pipes.

5. **No Modifications to Rendering:**
   - Rendering layer (`src/terminal_element.rs`, GPUI shaders/rendering) remains untouched; remote terminal views use the existing `TerminalView` and GPUI elements.

## Capabilities

### New Capabilities
- `remote-connections`: Remote agent deployment, SSH and container exec bootstrapping, remote PTY lifecycle, remote process inspection, and remote tmux control mode bridging.

### Modified Capabilities
- `profiles`: Support `remote` table in profiles defining SSH or container endpoints.
- `tabs`: Tab titles, working directories, and process tracking support remote backends.
- `tmux-control-mode`: Control mode attaches to remote tmux servers over remote sessions.
- `zones`: Zone splitting in remote tabs spawns new remote sessions inheriting remote CWD.
- `session-restore`: Remote tabs record endpoint identity and reuse CWD/zone directories only on the same endpoint.

## Impact

- **Configuration:** `src/config.rs` adds `RemoteTarget` (`Ssh`, `Container`).
- **Remote Subsystem:** New `src/remote/` module containing client transport, protocol types, bootstrap logic, and remote agent implementation (`src/remote/agent/` or dedicated binary/module).
- **Terminal & Workspace:** `Terminal::spawn_remote(...)` creates `Backend::Agent` terminals wired to remote PTY sessions (one helper process per session); `Workspace` and `ZoneTreeView` handle remote tab and zone spawning.
- **Tmux:** `TmuxSession` supports remote stdio streaming for `tmux -C`.
- **Testing:** Unit tests for protocol framing and transport; containerized e2e test cases validating SSH bootstrapping and container exec bootstrapping.
