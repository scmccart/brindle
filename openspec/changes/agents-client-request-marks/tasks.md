# Tasks

Tests come first in each group, written as sentences (`testing R15`). No test sleeps (`testing R9`). Verify commands run from the repository root.

## 1. Re-open the sites and show the fault

- [ ] 1.1 Once `agents-module-structure` (change 5) is on `main`, rebase and re-open every site design.md's Context cites: `clients.rs` `read_client`, `process_client_frames`, `handle_client` (with change 5's protocol-4 gate), `send_client`, `flush_client`, `drop_failed_client`; `core.rs` `update_interests`, `CLIENT_QUEUE_HIGH`, `CLIENT_QUEUE_LOW`; `policy.rs` `queue_mark`; `agents.rs` `client_agent_fetch`; `src/remote/local/connection.rs` `Connection::queue`. Correct the line numbers in design.md, and record whether `agents-event-bounds` (change 3) has landed, which decides the bound design.md decision 1 states for the one answer that crosses the mark. Verify: design.md's Context names each site as it is on `main`.
- [ ] 1.2 (C-6, shown first) In `remote-agent/tests/agents.rs`, add a real-role test: "a client that sends `AGENT_FETCH` frames without reading has at most its high mark, one page answer and the socket buffers' worth of answers taken, and a second client's `LIST` is answered while the first is stalled". It starts an agent session whose log holds one small entry, writes fetches in batches on a nonblocking socket until a write would block or a cap the test names is reached, and gates on the second client's answer, not on time (`testing R9`). Neither `STATUS` (`ServerStatus`, `core/manage.rs`) nor the core's log counts answered fetches at `ba18a14`, so the test counts from its side: with the fault the core keeps reading and the client reaches the cap without blocking; with the fix its writes block once the high mark's worth of answers waits, plus what the socket buffers and one turn's `READ_BUDGET` hold. It then reads everything and checks that every fetch is answered, in the order sent. The test runs on both platforms through the support client's connection put in nonblocking mode. If the Windows named-pipe support client can't write without blocking, the real-role case is Unix-only with that reason in a comment, and task 3.1's in-process core test, written at this point instead, shows the fault on every platform. Record that it fails at the rebased commit. Verify: `cargo test -p brindle-remote-agent --test agents a_client_that_asks_without_reading`. If it can't be made to fail, make none of groups 2 and 3's code changes: keep the test as a pin, record what was observed in design.md decision 4, and close the change with that recorded.

## 2. The decision (pure)

- [ ] 2.1 Add `policy::takes_frames(over, failed)` in `remote-agent/src/server/core/policy.rs`. Test first, in `policy.rs`: "frames are taken under the mark, not over it, and always from a failed client" (one row per pair of `over` and `failed`). Verify: `cargo test -p brindle-remote-agent server::core::policy`.

## 3. The core's loop (shell)

- [ ] 3.1 Use `takes_frames` in `update_interests` (`server/core.rs`: `readable` only when it is true) and in `process_client_frames` (`server/core/clients.rs`: checked before each frame, stopping with the rest left in the connection's buffer), and handle the buffered frames on `flush_client`'s `Mark::Resume` after the sessions are resumed. Tests first, in `clients.rs`'s or `core.rs`'s unit tests: "frames buffered while over the mark are handled in order once the queue reaches the low mark"; "a failed client's frames are all handled whatever its mark". Verify: task 1.2's test and `cargo test -p brindle-remote-agent server::core`.
- [ ] 3.2 Update the docs this changes: `docs/code-map.md`'s core entry (a client over its high mark has its frames left unread, `takes_frames`, and they are handled at the low mark), and `docs/architecture/process-architecture.md`'s `R9` (its list of queues gains a client's requests under its queue's marks). Verify: `cargo test -p brindle --test architecture` passes, and the two docs name `takes_frames` and the request mark.
- [ ] 3.3 Audit the test support clients (`remote-agent/tests/support/`) and `remote-protocol`'s tests for one that writes many frames before reading, and move its reads to another thread (design.md decision 2). Verify: `cargo test -p brindle-remote-agent` passes in full, twice.

## 4. The whole

- [ ] 4.1 Run `cargo test --workspace --locked` and `cargo clippy --locked --workspace --all-targets -- -D warnings`. Verify: both pass, and `git diff --stat main -- remote-protocol/tests/fixtures/` shows no fixture changed.
- [ ] 4.2 Verify: `openspec validate agents-client-request-marks --strict` passes, and each task above is checked.

## Workflow follow-up

- Archive with `openspec archive agents-client-request-marks` in the implementation PR, syncing the delta spec. If `agents-process-lifecycle` has been archived by then, this change's "Session I/O" delta carries its text (the pipe session's drain and its two scenarios) before archiving.
