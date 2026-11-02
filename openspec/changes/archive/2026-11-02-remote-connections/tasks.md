# Tasks

## 1. OpenSpec Specification & Configuration

- [x] 1.1 Create `openspec/changes/remote-connections/specs/remote-connections/spec.md` defining requirements and scenarios for remote connection configuration, bootstrapping, PTY I/O, resizing, process tracking, CWD tracking, and tmux control mode.
- [x] 1.2 Update `src/config.rs` to add `RemoteTarget` enum (`Ssh`, `Container`) and `remote: Option<RemoteTarget>` to `Profile`. Add unit tests in `src/config.rs` verifying TOML parsing and validation.

## 2. Remote Framing Protocol & Agent Implementation

- [x] 2.1 Implement the bounded framing protocol in `src/remote/protocol.rs` with encode/decode and unit tests.
- [x] 2.2 Implement the headless Linux agent in `src/remote/agent.rs` and the standalone `remote-agent` workspace package, supporting PTY creation, I/O, resize, `/proc` process/CWD inspection, tmux forwarding, bounded shutdown, and a non-Linux error stub.
- [x] 2.3 Implement supervised client transport in `src/remote/client.rs`, including SSH cache/upload, isolated container injection/cleanup, bounded event delivery, and child/thread cleanup.

## 3. Remote Client & Terminal Integration

- [x] 3.1 Implement per-session remote channels and lifecycle handling in `src/remote/client.rs`.
- [x] 3.2 Implement `Terminal::spawn_remote` with `Backend::Agent`, resize/input/reply forwarding, bounded output delivery, and POSIX process/CWD tracking on every desktop platform.
- [x] 3.3 Update `src/workspace.rs` and `src/zone_view.rs` to route remote profile launches, restore, and zone splitting through the remote client without crossing endpoint CWDs.

## 4. Remote Tmux Control Mode Integration

- [x] 4.1 Update `src/tmux/mod.rs` to run `tmux -u -C` through SSH/container pipes with bounded protocol collection and a secret-free environment prelude.
- [x] 4.2 Verify native tabs, pane input/output, status, and lifecycle over remote tmux control mode.

## 5. Verification & Testing

- [x] 5.1 Add unit tests for protocol parsing, serialization, bounds, command construction, profile persistence, and transport framing.
- [x] 5.2 Add Docker-based e2e cases for vanilla container exec and SSH bootstrap, covering PTY I/O, resize path, OSC 7 CWD inheritance, OSC 9/777, OSC 7501, helper isolation, and remote tmux control mode.
- [x] 5.3 Run `cargo check`, `cargo build`, and `cargo test` across the workspace with no source warnings and all tests passing; run Linux agent tests/static build and both Docker remote e2e cases.
