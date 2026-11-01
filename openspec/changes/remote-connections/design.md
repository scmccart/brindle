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
|                                  └─── Process Info (/proc/<pid>/cwd, comm)        |
+───────────────────────────────────────────────────────────────────────────────────+
```

Remote tmux control mode does not go through the helper: Brindle runs `tmux -C` directly over the SSH/exec pipes.

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
To support binary transparency (terminal PTY data, out-of-band resize, and cwd/title queries), we use a lightweight, efficient framing protocol. One helper process serves one session, so frames carry no session id.

Each message is framed with a five-byte header and a payload:
- `msg_type: u8` (Data = 1, Resize = 2, Exit = 3, QueryProcess = 4, ProcessInfo = 5, Spawn = 6, SpawnResponse = 7, Error = 8)
- `length: u32` (Big-endian payload length)
- `payload: [u8; length]`

Both ends ship together (the cached helper's path contains its digest), so the protocol has no version negotiation.

In `--server` mode the agent writes the marker `\0BRINDLE-FRAMES1\n` before its first frame; the client discards whatever precedes it (up to 64 KiB, e.g. `~/.bashrc` output printed before the helper starts) and fails with that text in the error if the marker never arrives. The agent also lets a client stall for up to 120 s before ending a session, so a network blip or a laptop wake does not kill the shell.

For control and process info payloads, concise JSON or binary encoding is used:
- `SpawnRequest`:
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
2. It reads the `SpawnRequest` from the first frame.
3. It serves a PTY session:
   - Uses standard Linux `openpty()` / `forkpty()` (via `libc` or `nix`).
   - Sets raw mode, window size via `TIOCSWINSZ`.
   - Forward bytes received from the client to the PTY master fd.
   - Read from the PTY master fd and frame it to the client.
   - When requested, inspects `/proc` of the foreground process group (via `tcgetpgrp(fd)` and `/proc/<pid>/stat` / `/proc/<pid>/cwd` / `/proc/<pid>/comm`).

The helper serves only PTY sessions. Brindle's `TmuxSession` runs `tmux -u [tmux_args] -C ...` directly over the SSH/exec pipes through `remote::tmux_command`.

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
3. Run `<helper> --server` through `<engine> exec -i`, send `SpawnRequest` as the first frame, and remove the helper file/directory when the transport ends (the helper also self-unlinks when permitted).

## 5. Integration with Brindle Terminal & Workspace

1. **`Terminal` model integration:**
   - In `src/terminal/mod.rs`, add `Terminal::spawn_remote(...)`:
     Uses `Backend::Agent` with a `client::Handle` for user input, resize events, and process information (`Backend::Remote` remains the byte-feed backend for tmux panes).
   - When `terminal.resize(size)` is called: sends `Resize` frame to the remote agent.
   - When input is typed: `Handle::input(bytes)`, sent as `Data` frames of at most 32 KiB.
   - Remote output received: calls `terminal.feed(bytes, cx)`, which automatically passes through OSC 9/777, OSC 7501, and OSC 133 semantic marks without any special remote logic needed!
   - Process polling: `terminal.foreground_process_name()` and `terminal.working_directory()` query the helper's latest reported process info.

2. **`Workspace` & `ZoneTreeView`:**
   - When splitting a zone in a remote tab:
     - The split starts a new connection with its own helper process (one helper per session), inheriting the remote target and the remote CWD.
   - When closing or exiting:
     - The helper sends `Exit`, causing `TerminalEvent::Exited` to fire locally, closing the zone or tab as normal.

3. **Tmux Control Mode (`TmuxSession`):**
   - In `src/tmux/mod.rs`:
     - When `profile.remote` is `Some(_)`, `Starter` spawns the process built by `remote::tmux_command` (`sh -lc` over SSH or container exec) rather than a local `tmux`.
     - Output events from tmux control mode (`%begin`, `%output`, `%layout-change`, etc.) stream over the SSH/exec pipes directly to Brindle's `protocol.rs` collector.
     - tmux input (`send-keys -H`, commands) streams back over the same pipes.

## 6. Rendering Layer Boundary

Strictly untouched:
- `src/terminal_element.rs`
- `src/terminal_view.rs` (except standard event propagation)
- Shader/GPU code
All rendering remains 100% native local GPUI drawing.

## 7. Deferred Follow-ups

A quality review of this change left these alone on purpose. Each is a separate decision, not an oversight.

1. **Behaviour changes.**
   - `Workspace::apply_config` keeps an open tab on the endpoint it started on, even if the profile's `remote` changes. This is read as intended, but tmux tabs take the new profile, so the two paths differ. Holding the tab's launch identity apart from its settings would make that explicit.
   - Remote titles come from a `QueryProcess` round trip each second per tab, hidden and idle tabs included. Having the helper push `ProcessInfo` on a change would stop that traffic, but it changes how titles update.
2. **Lifecycle code that was only just stabilised.**
   - The `remote-cancel` watcher polls every 50 ms and the writer every 100 ms, about 30 idle wakeups a second per tab. A close message with a blocking `recv` could replace both, but the writer must still wake after a session ends on its own while the `Terminal` holds its sender.
   - The 20 ms `try_wait` loops (`wait_process`, `ChildGuard::wait_bounded`) could block on a thread with `recv_timeout`.
3. **Not worth it yet.**
   - An `Endpoint` type could hold the five `profile.remote.is_some()` checks, the WSL check and the `CONTROL_CLIENT_MARKER` skip. It is an abstraction over one-line checks; reconsider when another endpoint kind or check appears.
   - `block_on(events.send(..))` for each tmux control line or output chunk could try `try_send` first, if a measurement shows it matters.
4. **Connection cost.**
   - Each SSH tab or split makes two connections: one to check the cached helper and one to launch it. A launch of `test -x "$p" && exec "$p" --server`, uploading only when that reports a missing helper, needs one when the cache is warm. It changes the bootstrap flow, so judge it by latency on a real host.
   - Containers copy the helper for every tab and delete it on exit. One root-owned directory per container, keyed by digest, would skip that, but changes the cleanup guarantee.
