# Proposal

## Why

The 2026-11-26 agent code review (`docs/architecture/audits/2026-11-26-agents.md`; change 8 of `2026-11-26-agents-remediation.md`) found that a client's queue marks don't cover the client's own requests:

- **C-6** (`docs/architecture/audits/2026-11-26-agents/c.md`, plausible): a client's queue marks pause holder reads, but the core goes on reading and answering that client's own requests. Each `AGENT_FETCH` (about 40 bytes) is answered with up to a page of the session's log, so a client that sends `AGENT_FETCH`es without reading grows its queue without bound (`process R9`). `LIST` and `STATUS` have the same shape with small answers.

The finding was first planned in `agents-event-bounds` (change 3), with which it shares no code; the review of the written changes (2026-11-27) split it out into this change.

## What Changes

- **The core stops taking a client's requests while that client's queue is over its mark (C-6):** it stops reading the client's frames and handles no more of those already read until the queue is back under its low mark, so every request kind is bounded at once. A client whose write failed still has what it sent handled (`process R10`).
- The first task shows the fault with a real-role test. If it can't be shown, the fix is not made, the test stays as a pin, and the change is closed with that recorded.
- No wire change: no message, field or protocol version changes.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `terminal-server`: "Session I/O" gains that a client that doesn't read has its requests left unread until it does.

## Impact

- **Code:** `brindle-remote-agent` only: `server/core/policy.rs` (`takes_frames`), `server/core/clients.rs` (`process_client_frames`, `flush_client`), `server/core.rs` (`update_interests`).
- **Tests:** a unit test in `policy.rs`; unit tests of the buffered frames in the core; a real-role test in `remote-agent/tests/agents.rs`; an audit of the test support clients in `remote-agent/tests/support/`.
- **Docs:** `docs/code-map.md` (the server core entry), `docs/architecture/process-architecture.md` (`R9`'s list gains the request mark).
- **Order:** after `agents-module-structure` (change 5), which adds one protocol gate in `handle_client` and touches `server/core/clients.rs` and `policy.rs`. This change is written against the code at `ba18a14`, and its first task re-opens the sites on the `main` it is implemented against.
- **Overlap with the seven sibling changes:**
  - `agents-client-state-fixes` (1) fails a zone's queued fetches when its link is lost (D-5) and keeps one fetch in flight; Brindle's client still never floods fetches. No shared code.
  - `agents-zone-focus-and-scroll` (2): none.
  - `agents-event-bounds` (3) bounds a text entry at 1 MiB. With it, the largest answer to the one request that crosses the mark is `PAGE_BYTES` plus one entry, at most 2,097,280 bytes (that change's decision 2). Without it an entry has no bound of its own (A-1), so that one answer is bounded only by what a session can hold before its checkpoint fails (about 64 MiB). This change bounds the number of answers either way; change 3 bounds the size of each.
  - `agents-process-lifecycle` (4) lands before this change and touches `clients.rs` (`take_over`). Both changes modify `terminal-server` "Session I/O", each adding its own text (that change a pipe session's drain, this one the request mark): whichever is archived second carries the other's text in its delta. Change 4 is expected to be archived first, so this change's delta carries its text once it has been.
  - `agents-module-structure` (5) lands before this change: its protocol-4 gate in `handle_client` runs after this change's check, which is made before a frame is decoded, so the two don't meet. It moves no code this change edits.
  - `agents-checkpoint-deltas` (6) and `agents-absorb-cost` (7): none.
- **Overlap with open changes:**
  - `agents-prompt-context` adds `AGENT_ATTACH` and `AGENT_READ_ATTACHMENT`, and `agents-prompt-images` replaces the first with uploads in pieces and reads attachments back a piece at a time. Both are requests from a client, so both fall under the mark: an upload from a client over its mark waits until it reads, and a read-back's answers are bounded like a fetch's.
  - `agents-client-terminals` has the terminal server ask Brindle to run an agent's terminals, so Brindle's answers to those requests are frames the core reads. While Brindle is over its mark they wait until it reads, which Brindle's reader thread does without waiting on a write (design decision 2).
  - `add-pocket-client` speaks client protocol 1 over the relay and sends one request at a time; nothing changes for it.
