# Process architecture

How Brindle splits into processes and threads, how they talk, and how they survive crashes, upgrades and version skew. It governs the terminal server (`remote-agent/`), its protocols (`remote-protocol/`), Brindle's client (`src/remote/`), the relay to remote endpoints, the single-instance lock (`src/instance.rs`), the event path into GPUI (`src/terminal/`), and the config and state files. Code review enforces the **Rules** section; reviewers cite rules by number (`process R5`).

## Why

A terminal holds work the user can't get back: running programs and their screens. A Windows pseudoconsole can't move to another process, so whatever owns it must outlive every upgrade above it. Brindle, the server core and the holders are also built and upgraded separately, on this machine and on remote hosts, so an old and a new version meet on every upgrade. Each rule here exists because the opposite once lost sessions, froze a window or broke a mixed-version pair.

## The idea

- **State lives where it survives; risk lives where it can restart.** The holder owns PTYs and keeps bytes it doesn't interpret. The server core holds the parser, the protocol and the policy, and rebuilds itself from the holder's bytes.
- **Every boundary between versions is an explicit, negotiated protocol**, pinned byte for byte.
- **One thread per role on a readiness loop**, with bounded queues, and a UI thread that never waits.

```rust
// The holder keeps, per session, what it doesn't interpret: the core's last
// checkpoint and the raw output since. A new core feeds a fresh emulator both.
struct Held { meta: serde_json::Value, checkpoint: Vec<u8>, tail: Vec<u8> }
```

## Idioms

**Additive fields** keep existing encodings byte for byte:

```rust
#[derive(Serialize, Deserialize)]
pub struct Exit {
    pub session: u64,
    pub code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lost: Option<LostReason>,
}
```

**Negotiation** picks the highest common version: `negotiate(offered, supported) -> Result<u32, NegotiationError>`. A new message type is a new version, sent only on connections that negotiated it (`PROCESS` on 2, `SET_LAYOUT` on 3).

**Chunking:** `CHECKPOINT_BEGIN`, 64 KiB `CHECKPOINT_CHUNK`s, `CHECKPOINT_END { covers }`. The receiver replaces its copy only on `END`, and drops only the `covers` tail bytes, since more output may be in flight.

**Ordering by counting, not acknowledging:** a replay waits until the session's last `SET_META` is in its holder's connection (`Session::storing`, a count of bytes written), instead of a new ack message that would need a version bump.

**Single instance:**

```rust
let Some(lock) = Lock::acquire(&dir.join("core.lock"))? else {
    return Ok(ExitCode::SUCCESS);       // lost the race: the starter connects to the winner
};
// …bind, then one ready byte; the starter counts the role ready only on exactly that byte.
```

**Graceful close:** flush the whole queue, shut down the write side, read to the peer's end within a bound, handling every frame already read.

**Threads to one UI task:** readers parse under the lock order and send events on a channel through the terminal's gate (`terminal/events.rs`), which merges wakeups and titles at the source and drops side events past its marks; the task paces bursts (`pace::step`) and drains for at most `pace::BUDGET`.

## How it applies here

| Process or thread | Owns | Rules that bind it most |
| --- | --- | --- |
| holder (`brindle-remote hold`) | PTYs and pseudoconsoles, programs, each session's meta, checkpoint and tail | R1, R9, R10 |
| server core (`brindle-remote serve`) | an `Emulator` per session, the client protocol, policy | R2, R3, R4–R8, R12, R19 |
| relay (`brindle-remote relay`; Linux only: built under `target_os = "linux"`, and refused with "relay runs only on Linux" on every other platform, macOS included) | one endpoint's link, copied by frame | R7, R18 |
| Brindle's connection threads (`src/remote/local/`) | one reader and one writer per endpoint | R13, R14, R18 |
| Brindle's UI thread | entities, painting | R13, R14, R16 |
| the instance (`src/instance.rs`) | `session.lock`, the launch socket | R11 |

Protocols: client protocol versions 1–3 and core↔holder H1 at version 2, both in `remote-protocol/src/{client,holder,wire,version}.rs`; the relay's frames in `relay.rs`. Files read across versions: `config.toml`, `session.toml`, and the layout blob stored in a session's meta.

## Rules

- **R1.** The holder MUST stay small and change rarely. It parses nothing and has no policy; what it stores (`meta`, checkpoint, tail) is opaque bytes the server core owns. Each feature added to it is one that can kill every session, and a holder is never replaced while it has sessions.
- **R2.** The server core MUST be restartable without losing a session. It checkpoints into the holders, sends `SET_META` on every owner or policy change, and a new core adopts what the holders have. A change that keeps session state only in the core's memory must say what a core restart does to it.
- **R3.** Faults MUST be confined to their unit. The core's per-session feed runs under `catch_unwind` and marks that emulator broken; the session keeps forwarding and the rest keep running. The holder handles each message from its core under `catch_unwind` too, and answers a failure for that message's unit: `SPAWN_FAILED` for a `SPAWN`, an `ERROR` for its session (dropping a checkpoint it was assembling) or for the holder, while the sessions and the connection carry on. A new per-session, per-client or per-endpoint call that handles outside data gets the same guard or a reason it can't panic.
- **R4.** Every message format between processes or versions MUST be versioned and negotiated per connection (`HELLO` offers a range, `WELCOME` picks). A process speaks every version up to its own, so mixed fleets work during an upgrade.
- **R5.** Within a version, changes MUST be additive: new JSON fields are optional with defaults and omitted when unset, so existing encodings stay byte for byte. A new message type or a changed meaning is a new version, sent only on connections that negotiated it. A format that has shipped is frozen.
- **R6.** Every encoding MUST be pinned by a transcript test and every decoder fuzzed (`fuzz.rs`). Encodings are deterministic: sort the keys of free-form maps. The message modules (`client`, `holder`, `wire`, `version`, `relay`) are codecs only, with no IO, clock or thread.
- **R7.** Frames MUST be bounded and self-describing (type, big-endian length, payload), with a limit per kind (`MAX_LAYOUT`, 16 MiB of unread frames from a dead peer). Large payloads are chunked. On a stream that can carry noise first (a shell's output over SSH), the sender writes the frame marker and the reader skips up to a bound until it sees it.
- **R8.** Each server role MUST run one thread on a level-triggered readiness loop, and handlers MUST read and write through a source's owner, never through the handle the `Source` wraps, so the Windows loop can do the IO itself. The role's logic names no platform type (`ports R5`).
- **R9.** Every queue between a producer and a slower consumer MUST have a high and a low mark (a client's queue: `PAUSE` past 1 MiB, `RESUME` under 256 KiB; the holder's queue to its core: no PTY read from `CORE_QUEUE_HIGH`, 8 MiB, until under `CORE_QUEUE_LOW`, 2 MiB; a pipe's writes: `WRITE_MARK`; a Brindle session's input: its `InputBudget`, 4 MiB / 1 MiB; a terminal's events to the UI task: its `EventGate`, 2048 / 512 waiting). A program blocks on output rather than losing it, and memory stays bounded. Input is the exception: the holder drops a session's input past its 4 MiB cap with an `ERROR`, lossy by design, because Brindle's own input budget in front of it is the same size and refuses first. That budget counts a session's input from when Brindle takes it (while the session starts, and while its link is down or slow) until the connection's writer has written or skipped it; a call is taken whole while less than 4 MiB waits, refused past it until less than 1 MiB waits, and the first refusal of each crossing puts one dim line in the tab. Resizes, closes, detaches and the other control messages are never refused. The event gate merges wakeups, titles, foreground reports and clipboard writes at the source, always queues life events, and drops side events (bells, notifications, status reports, query replies) past its high mark until under its low one, then says how many at `warn` and resets the status records if any were status reports.
- **R10.** Closes MUST be graceful: send the whole queue, shut down the write side, read to the peer's end within a bound. Everything already read is handled before a read or write error is acted on, so a `DETACH` sent just before a quit still counts.
- **R11.** A single instance MUST be an OS lock held for the process's life (`flock`, `LockFileEx`, taken without blocking), and a start race MUST leave both starters succeeding: the loser exits 0 and its starter connects to the winner. A role signals readiness with one byte once it is listening.
- **R12.** Upgrades MUST be decided by the client with the newer build. The running core only vets it (`check`) and steps down after checkpointing; holders of another build drain and exit when empty. A supervisor restarts a dead peer from its own image with backoff (3 s to 60 s). A wrong winner is tolerated when the next newer client steps it down again.
- **R13.** The UI thread MUST NOT block. `Terminal::spawn`, `Session::start` and `Session::attach` return at once and queue input; connecting, starting and upgrading happen on their own thread. Events reach GPUI on a channel, merged at the source and paced: on Windows a task that wakes without pause starves `WM_PAINT` and `WM_QUIT`.
- **R14.** Locks MUST be taken in the stated order, parser → term (`lock_unfair` while holding the parser) → marks. A terminal's event gate (`EventGate`) is a leaf: alacritty sends events mid-parse, so it is taken under the parser and term locks, and nothing is taken under it. `render` and `prepaint` MUST stay free of IO, heavy allocation and lock contention: GPUI 0.2.2 re-renders the whole window on any `cx.notify()`.
- **R15.** Files read by more than one version MUST load across them. Unknown config keys are rejected so typos fail loudly; a removed key stays as a field that is parsed, warned about and ignored; an old state-file variant maps to a skip variant; a bad config falls back to defaults and shows why. Writes go through a temporary file and a rename and change only the keys they concern.
- **R16.** What a quit must do MUST be done explicitly (`Workspace::quit` → `Terminal::quit_with`), never in a `Drop`. On Windows a dropped window's state outlives `App::shutdown` and its drops never run. Quit waits a bounded time (`QUIT_FLUSH`) for queued `DETACH`es and `CLOSE`s to be written.
- **R17.** There MUST be one path for a tab with a program: a session of a terminal server, local or remote. No in-process PTY fallback and no switch that turns the server off; a server problem is shown in the tab (`railway R6`), not hidden by a second backend.
- **R18.** Peer liveness MUST be stamped on the reader thread, which drops heartbeats itself. Never measure it where output is copied: a slow client blocks that path and healthy peers' frames go unread.
- **R19.** The server core MUST NOT answer a terminal's queries. Its emulator's listener drops every event but title changes; the attached client answers. Two answerers would reply twice.
- **R20.** A test-only switch MUST be an environment variable named `BRINDLE_TEST_…`, and one that injects a fault MUST exist only in debug builds (`BRINDLE_TEST_FEED_PANIC`), and each binary lists the ones it honours with `--list-fault-switches`, so a test tells a missing switch from a broken feature.
- **R21.** The five workspace crates MUST share one version: Brindle matches its helper by version. Versions are moved by release-please alone.

## Anti-patterns

- **A feature added to the holder because the data is already there.** Instead: the core interprets; the holder stores bytes (R1).
- **A new required JSON field, or a reordered one.** Instead: optional, defaulted, omitted when unset; older transcripts must still pass unchanged (R5).
- **A new message sent to every peer.** Instead: a new version, sent only where negotiated (R4, R5).
- **An ack message to prove an ordering the transport already gives.** Instead: count bytes written and gate on the count.
- **Blocking reader and writer threads per session in the server.** Instead: the role's one readiness loop (R8).
- **A raw ring buffer as the only replay state.** Instead: a checkpoint rendered from the model, plus the tail since.
- **Connecting, or reading `/proc`, inside `render`.** Instead: a cached value refreshed by an event or the 1 s poll (R13, R14).
- **Cleanup in `impl Drop` that the next launch depends on.** Instead: do it in the quit path (R16).
- **Dropping a peer on a write error with its frames unread.** Instead: handle what it sent, within a bound, then drop (R10).
- **A migration error for a config key that can only be ignored.** Instead: warn and drop (R15).

## Enforcement

- `tests/architecture.rs`: `the_wire_modules_do_no_io` (R6's codec half) and `the_server_core_and_holder_name_no_platform_type` (R8).
- Transcript tests in `remote-protocol/tests/transcripts.rs` pin every message in both directions (R5, R6). `every_message_variant_and_form_appears_in_a_transcript` checks coverage per message variant and nested form (each enum's variants, `Exit.lost`, `ExitStatus.code`), and `BRINDLE_REGEN_FIXTURES=1` writes only missing fixtures, never rewriting an existing one (R5). `fuzz.rs` covers the decoders (R6, R7).
- `remote-agent/tests/{holder,core,upgrade,manage,relay,launch}.rs` run the real roles: start races (R11), crash and adopt (R2), reconnect and replay, backpressure (R9), upgrade, drain and refused check (R12), fault injection (R3).
- CI's `test` job checks R21 (`cargo metadata`), and both platforms compile the platform module sets.
- R1, R13, R14 and R16–R20 rely on review; the e2e `local-server-*` cases catch some regressions of R13, R16 and R17.

## PR checklist

- [ ] Nothing new in the holder interprets bytes or holds policy.
- [ ] Session state added to the server core survives a core restart, or the change says what happens.
- [ ] A new call on outside data is confined per unit.
- [ ] Wire changes are additive within a version, or a new negotiated version; transcripts added, older ones untouched; the decoder is fuzzed.
- [ ] Frames and stored blobs have a bound.
- [ ] New queues have marks; new closes are graceful.
- [ ] Nothing on the UI thread waits; nothing in `render` or `prepaint` does IO or takes a contended lock.
- [ ] The lock order is kept.
- [ ] Config and state changes load old files and are loaded by old versions as far as they must be.
- [ ] Quit-time effects are explicit, not in `Drop`.

## References

- The architecture plugin's `process-architecture` skill (its examples are this repo's).
- `docs/code-map.md`, "Terminal server", "Brindle's client", "Relay"; `CLAUDE.md`, the "Windows" pitfalls.
- `openspec/changes/archive/` for `terminal-server`, `terminal-server-windows`, `terminal-server-on-close` and `session-layout-in-server`.
