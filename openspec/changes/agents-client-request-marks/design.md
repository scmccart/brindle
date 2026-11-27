# Design

## Context

See `proposal.md` for why. Every site below was opened at `ba18a14` (`agents-commands`, PR #151, merged). This change lands after `agents-module-structure` (change 5), which adds a protocol-4 gate in `handle_client`; the first task re-opens the sites on the `main` it is implemented against.

How the core treats a client today (`remote-agent/src/server/core/`):
- `read_client` (`clients.rs:279-293`) reads what the socket has, up to `READ_BUDGET` a turn (`server/conn.rs`'s `read_available`), then calls `process_client_frames` (`clients.rs:295-310`), which decodes and handles every whole frame read so far, whatever `entry.over` says.
- `update_interests` (`server/core.rs:420-435`) always asks a client for `readable`, and for `writable` while its queue holds bytes.
- `send_client` (`clients.rs:368-396`) queues an answer and, past `CLIENT_QUEUE_HIGH` (1 MiB, `core.rs:95`), sets `entry.over` and pauses the client's sessions in their holders. `flush_client` (`clients.rs:402-417`) writes what the socket takes and, under `CLIENT_QUEUE_LOW` (256 KiB, `core.rs:97`), clears `over` and resumes them. The mark's decision is `policy::queue_mark` (`policy.rs:224-231`), with its table test.
- `drop_failed_client` (`clients.rs:426-435`) sets `entry.failed`, reads up to `DEAD_CLIENT_READ_LIMIT` (16 MiB), handles those frames and drops the client (`process R10`). `send_client` sends a failed client nothing.
- `client_agent_fetch` (`agents.rs:712-748`) answers an `AGENT_FETCH` with a page of the session's log, packed by `pack_page` into `AGENT_PAGE` frames.

The readiness loop is level-triggered on both platforms (`server/event_loop.rs`): a source whose interest is off stays registered and silent, and is reported again once the interest returns if it still has input. On Linux (`PollLoop`) a paused socket's input stays in the kernel; on Windows a paused pipe holds at most the one read that was in flight, into its 64 KiB `READ_BUFFER` (`platform/windows/pipe.rs`).

### Finding, traced

**C-6, traced** (*plausible*). `process_client_frames` handles every frame whatever `entry.over`; `update_interests` always asks a client for `readable`; the marks (`send_client`, `flush_client`) only pause the client's sessions in their holders. The fault (no mark on a client's own requests, `process R9`) is confirmed by reading; what is plausible is a client that sends requests without reading. Brindle doesn't: its zone has one fetch out at a time. Task 1.2 shows the fault with a client that does; decision 4 says what happens if it can't be shown.

## Goals / Non-Goals

**Goals:**
- A client's own requests are bounded by its queue's marks, every request kind at once.
- Frames a client sent are handled in order and none is lost, across a pause and across a failed write.

**Non-Goals:**
- The size of one answer. A page is `PAGE_BYTES` plus at most one entry; how large an entry can be is `agents-event-bounds`' (change 3, A-1).
- Brindle's own fetch pacing: it already has one fetch in flight a zone.
- A liveness timeout for a client the core doesn't read (decision 3).
- Any wire change.

## Layers

| Part | Kind | Crate, file |
| --- | --- | --- |
| `policy::takes_frames` | pure | `brindle-remote-agent`, `remote-agent/src/server/core/policy.rs` |
| reading and handling a client's frames by it | shell (the core's loop) | `server/core/clients.rs` (`process_client_frames`, `flush_client`), `server/core.rs` (`update_interests`) |

No crate gains a dependency, and no platform `cfg` is added: both event loops already pause a source whose interest is off (`ports R4`, `R5`).

## Decisions

### 1. A client's requests stop while its queue is over its mark (C-6)

`policy::takes_frames(over: bool, failed: bool) -> bool` is the decision: true unless the client is over its high mark and its connection hasn't failed.
- `update_interests` asks for `readable` only when it is true.
- `process_client_frames` checks it before each frame and stops, leaving the frames already read in the connection's buffer.
- `flush_client`, on `Mark::Resume`, resumes the client's sessions as today and then calls `process_client_frames`, so the buffered frames are handled in order before anything newer is read.
- A client whose write failed (`drop_failed_client`) has every frame it sent handled whatever `over` says, as `process R10` requires (a `DETACH` sent just before a quit still counts). Their answers are dropped, as today.

This bounds every request kind at once (`AGENT_FETCH`, `LIST`, `STATUS`, `ATTACH`'s replay): one client's queue reaches at most `CLIENT_QUEUE_HIGH` plus the answer to the one frame that crossed it. The largest such answer is a page: `PAGE_BYTES` plus one entry, at most 2,097,280 bytes after `agents-event-bounds` (change 3) decision 2, which bounds a text entry at 1 MiB. If change 3 hasn't landed, an entry has no bound of its own (A-1): a text entry grows until the session's checkpoint passes `MAX_CHECKPOINT_LEN` and the session fails, so that one answer is bounded only by about 64 MiB. This change bounds how many answers wait either way; change 3 bounds the size of each.

An `ATTACH`'s replay can be larger than a fetch's page: `Transcript::page` reaches back from the newest page to every pending permission (at most `MAX_PENDING_PERMISSIONS`, 16) and its tool call, and the replay adds the slot, turn and agent state. That is still one answer, bounded by those limits.

The inputs are the client entry's existing `over` and `failed` fields; no new state is added.

### 2. Can it deadlock a client that must read to drain?

Only a client that won't read until a write completes, on one thread. None in the tree is one:
- **Brindle's `Connection`** reads on its reader thread and writes on its writer thread. The reader never blocks on a write: `Connection::queue` (`src/remote/local/connection.rs:1329`) sends to an unbounded channel. While the core reads nothing, the writer blocks, and the reader keeps draining the core's queue, which reopens the reads.
- **The relay** reads its stdin on one thread (`forward_input`) and copies the core's bytes on another (`forward_output`), as `server/relay.rs`'s module doc says.
- **The CLI** (`brindle-remote list`, `status`, `stop`) sends one small request and reads its answer.
- **The test support clients** may write many frames before reading. Task 3.3 audits them, and any that floods without reading moves its reads to another thread.

A user typing into a terminal whose output has filled the queue has their input taken once the queue drains. The client's sessions are paused meanwhile, so that is as soon as Brindle reads; Brindle's input budget refuses past 4 MiB, as it does for any slow link.

### 3. The rules it keeps (`process R9`, `R10`)

**`process R9`** asks for a high and a low mark on every queue from a producer to a slower consumer. Here the producer is the core's handling of the client's requests: the mark is `CLIENT_QUEUE_HIGH`/`LOW`, which already pause the sessions, and now also the requests. **`R10`** is kept as decision 1 describes.

The core has no client liveness timeout, so a client the core doesn't read isn't dropped for silence.

### 4. Shown first, or not made

C-6 is *plausible*. Task 1.2 writes the real-role test first and runs it against the unchanged core. If it fails there, the fix is made and the test passes after it. If it can't be made to fail (the core already stops taking the client's requests somewhere this design missed), the fix is not made: the test stays as a pin of the behaviour the spec states, this decision records what was observed, and the change is closed with no code change beyond the test.

### 5. Wire and versions

No message, field or protocol version changes. A client of any protocol version sees only its requests answered later while it doesn't read, which it can't tell from a slow link (`process R4`–`R6` not engaged). The protocol fixtures (`remote-protocol/tests/fixtures/`) are unchanged.

### 6. Restarts, crashes and upgrades (`process R1`–`R3`, `R12`)

- **A core restart** starts with no clients; each reconnects with `over` false, as today.
- **A Brindle crash** changes nothing here. The core's per-client `over` goes with the connection, and the next attach replays as before.
- **An upgrade** hands the core's sessions over by checkpoint; clients reconnect to the new core, which is the core-restart case.
- **The holder** is untouched (`process R1`).

## Alternatives rejected

- **Refuse `AGENT_FETCH` with "try again" while over** (C-6's other option): bounds one request kind, and shows the user a fetch error for the server's own backpressure.

## Risks / Trade-offs

- [This delays a flooding client's input] → only while its queue is over 1 MiB, with its sessions paused; Brindle's input budget refuses past 4 MiB, as on any slow link.
- [A test client that writes many frames before reading now blocks] → task 3.3 audits them before the change lands.
- [A paused client's frames wait in the core's buffer] → at most one turn's `READ_BUDGET` (1 MiB, `server/conn.rs`) was read before the pause; the rest stays in the socket (Linux) or in one 64 KiB read (Windows), so pausing costs the core no memory past that.
