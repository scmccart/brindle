# Design

## Architecture Overview

```
+-----------------------------------------------------------------------------------+
| Brindle Desktop (Windows / Linux)                                                 |
|                                                                                   |
|  Workspace / ZoneTreeView                                                         |
|         │                                                                         |
|         ▼                                                                         |
|    Terminal ◄────────────► RemoteSession ◄──────────────► Client Transport        |
|    (Backend::Agent)        (src/remote/client.rs)          (SSH / Container Exec) |
|         ▲                                                           │             |
|         │                                                           │ (stdio)     |
|   TmuxSession                                                       │             |
+─────────┼───────────────────────────────────────────────────────────┼─────────────+
          │ (Framed IPC / Stdio)                                      │
          ▼                                                           ▼
+───────────────────────────────────────────────────────────────────────────────────+
| Remote Host / Container (Linux)                                                   |
|                                                                                   |
|  brindle-remote (Headless Agent Daemon or Embedded Runner)                        |
|         │                                                                         |
|         ├── PtyWorker ────────► /dev/pts/X ──────► Remote Shell ($SHELL)          |
|         │                        │                                                |
|         │                        └─── Process Info (/proc/<pid>/cwd, comm)        |
|         │                                                                         |
|         └── TmuxWorker (supported, unused: Brindle runs remote tmux -C directly    |
|             over the SSH/exec pipes, without the helper)                          |
+───────────────────────────────────────────────────────────────────────────────────+
```

## 1. Remote Profile Configuration

In `src/config.rs`:
```rust
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum RemoteTarget {
    Ssh {
        host: String,
        user: Option<String>,
        port: Option<u16>,
        identity_file: Option<String>,
        ssh_args: Vec<String>,
    },
    Container {
        #[serde(default = "default_container_engine")]
        engine: String, // "docker" or "podman"
        container: String,
        user: Option<String>,
    },
}

#[derive(Debug, Clone, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Profile {
    pub name: String,
    pub remote: Option<RemoteTarget>,
    pub command: Option<String>,
    pub args: Vec<String>,
    pub cwd: Option<String>,
    pub env: BTreeMap<String, String>,
    pub theme: Option<String>,
    pub font_size: Option<f32>,
    pub tmux: TmuxMode,
    pub tmux_session: Option<String>,
    pub tmux_args: Vec<String>,
}
```

## 2. Remote Framing & Protocol

The client and remote agent communicate over `stdin`/`stdout` of the SSH or container exec process.
To support binary transparency and multiple concurrent channels (e.g. terminal PTY data, out-of-band resize, cwd/title queries, and tmux control mode streams), we use a lightweight, efficient framing protocol.

Each message is framed with:
- `channel_id: u32` (0 = control/agent, 1..N = session/PTY/tmux streams)
- `msg_type: u8` (Data = 1, Resize = 2, Exit = 3, QueryProcess = 4, ProcessInfo = 5, Spawn = 6, SpawnResponse = 7, Error = 8, Ping = 9, Pong = 10)
- `length: u32` (Big-endian payload length)
- `payload: [u8; length]`

In `--server` mode the agent writes the marker `\0BRINDLE-FRAMES1\n` before its first frame; the client discards whatever precedes it (up to 64 KiB, e.g. `~/.bashrc` output printed before the helper starts) and fails with that text in the error if the marker never arrives. The agent also lets a client stall for up to 120 s before ending a session, so a network blip or a laptop wake does not kill the shell.

For control and process info payloads, concise JSON or binary encoding is used:
- `SpawnRequest`:
  - `kind: "pty" | "tmux"`
  - `command: Option<String>`
  - `args: Vec<String>`
  - `cwd: Option<String>`
  - `env: BTreeMap<String, String>`
  - `cols: u16`, `rows: u16`, `cell_width: u16`, `cell_height: u16`
- `Resize`: `cols: u16`, `rows: u16`, `cell_width: u16`, `cell_height: u16`
- `ProcessInfo`:
  - `pid: u32`
  - `foreground_name: Option<String>`
  - `cwd: Option<String>`

## 3. Remote Agent Architecture (`brindle-remote`)

The agent is compiled for Linux (`x86_64-unknown-linux-musl` / `x86_64-unknown-linux-gnu` or self-contained Rust executable / Python/sh-based fallback for bootstrap).
Given Brindle is 100% Rust, compiling a lightweight standalone `brindle-remote` binary and embedding or deploying it provides maximum performance, reliability, and zero external runtime dependencies.

On the remote host:
1. `brindle-remote` runs in stdio mode (`brindle-remote --server`).
2. It listens for `SpawnRequest` on channel 0.
3. For PTY sessions:
   - Uses standard Linux `openpty()` / `forkpty()` (via `libc` or `nix`).
   - Sets raw mode, window size via `TIOCSWINSZ`.
   - Forward bytes received from client channel to PTY master fd.
   - Read from PTY master fd and frame onto the channel to client.
   - When requested or periodically, inspects `/proc` of the foreground process group (via `tcgetpgrp(fd)` and `/proc/<pid>/stat` / `/proc/<pid>/cwd` / `/proc/<pid>/comm`).
4. For tmux control sessions (supported by the helper, but unused by Brindle, whose `TmuxSession` runs `tmux -u [tmux_args] -C ...` directly over the SSH/exec pipes through `remote::tmux_command`):
   - Spawns `tmux -u [tmux_args] -C ...` with piped stdio.
   - Bridges stdin/stdout across the channel directly.

## 4. Bootstrapping Flow

### SSH Bootstrapping
When connecting to an SSH remote:
1. Client probes `$HOME/.cache/brindle/<version>-<digest>/brindle-remote` (the digest is the helper's SHA-256) with `--version` through the configured OpenSSH target.
2. If the user-owned helper matches the client version, Brindle executes it with `--server`.
3. Otherwise Brindle streams the packaged Linux helper to a per-shell temporary file under that directory, applies mode `0700`, and atomically renames it into place.
4. The `SpawnRequest` (including command, arguments and environment) is sent as the first framed stdin message, so credentials and profile data do not appear in process arguments.

### Container Exec Bootstrapping
When connecting to a Docker/Podman container:
1. A stopped or missing container fails at the first `<engine> exec`, and its error text is shown in the tab.
2. Inject an isolated helper:
   - Generate an OS-random directory name, create `/tmp/brindle-<nonce>` exclusively as container root with no write permission for other users, and copy the helper to its fixed leaf.
   - Verify the leaf is a regular root-owned file and mark it executable; the configured profile user runs it.
3. Run `<helper> --server` through `<engine> exec -i`, send `SpawnRequest` as the first bounded control frame, and remove the helper file/directory when the transport ends (the helper also self-unlinks when permitted).

## 5. Integration with Brindle Terminal & Workspace

1. **`Terminal` model integration:**
   - In `src/terminal/mod.rs`, add `Terminal::spawn_remote(...)`:
     Uses `Backend::Agent` with a `client::Handle` for user input, resize events, and process information (`Backend::Remote` remains the byte-feed backend for tmux panes).
   - When `terminal.resize(size)` is called: sends `Resize` frame to the remote agent.
   - When input is typed: `channel.send_data(bytes)`.
   - Remote output received: calls `terminal.feed(bytes, cx)`, which automatically passes through OSC 9/777, OSC 7501, and OSC 133 semantic marks without any special remote logic needed!
   - Process polling: `terminal.foreground_process_name()` and `terminal.working_directory()` query the remote channel's latest reported process info.

2. **`Workspace` & `ZoneTreeView`:**
   - When splitting a zone in a remote tab:
     - The split starts a new connection with its own helper process (one helper per session), inheriting the remote target and the remote CWD.
   - When closing or exiting:
     - Remote channel sends `Exit`, causing `TerminalEvent::Exited` to fire locally, closing the zone or tab as normal.

3. **Tmux Control Mode (`TmuxSession`):**
   - In `src/tmux/mod.rs`:
     - When `profile.remote` is `Some(_)`, `Starter` spawns the process built by `remote::tmux_command` (`sh -lc` over SSH or container exec) rather than a local `tmux`.
     - Output events from tmux control mode (`%begin`, `%output`, `%layout-change`, etc.) stream across the remote channel directly to Brindle's `protocol.rs` collector.
     - tmux input (`send-keys -H`, commands) streams back over the remote channel.

## 6. Rendering Layer Boundary

Strictly untouched:
- `src/terminal_element.rs`
- `src/terminal_view.rs` (except standard event propagation)
- Shader/GPU code
All rendering remains 100% native local GPUI drawing.
