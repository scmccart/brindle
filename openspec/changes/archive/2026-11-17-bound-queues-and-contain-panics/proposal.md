# Proposal

## Why

The 2026-11-16 architecture audit (`docs/architecture/audits/2026-11-16.md`, finding 8; change 9 of `2026-11-16-remediation.md`) found queues between threads that grow without bound, per-session code on shared threads with no fault guard, refused connections dropped instead of closed, and a config write that a crash can truncate:
- **`process R9`, unbounded queues.**
  - The terminal event channel (`src/terminal/server.rs:58, 96, 107, 113`, `src/terminal/mod.rs:104, 112, 413, 443, 481`; b24 #2, b28 #9) takes a `Wakeup` per read plus every OSC, bell and reply, with no marks.
  - The connection's outgoing queue (`src/remote/local/connection.rs:442, 690-693`; b25 #19) holds all input typed while a relay is unreachable.
  - So does a starting session's own buffer (`src/remote/local/session.rs:33-34`; b26 smell).
  - The holder resumes reading PTYs the moment its queue to the core dips under `CORE_QUEUE_HIGH`, with no low mark (b12).
  - `--latency-log` keeps every sample for the life of the run (`src/latency.rs:232-239`; b01 smell).
- **`process R3`, faults not confined.**
  - The app parses each session's output on its endpoint's shared reader thread with no `catch_unwind` (`src/terminal/server.rs:72, 92`; b24 #3). A panic there ends that endpoint's reader, and with it every tab on the endpoint.
  - The holder has no guard around its message handling (b12 smell), so one bad `SPAWN` ends every session it holds.
- **`process R10`, refusals that lose their reason.** The core (`remote-agent/src/server/core/clients.rs:126-132, 154-156`; b10 #14) and the holder (`holder.rs:456, 477`; b12) flush a refusal and drop the connection. A peer whose unread bytes are still in the kernel gets a reset instead of the error.
- **`process R15`, a non-atomic write.** `src/config.rs:434` writes the default config with a plain `fs::write` (b23).
- **A Windows core reads directories as POSIX paths.** This was added at the user's request, from change 5's Open Questions.
  - The core's `Emulator::new` always parses with POSIX paths (`term-core/src/emulator.rs:80`, `Parser::posix()`).
  - So a Windows core ignores `ESC ] 9 ; 9 ; C:\… ST`, whose path isn't absolute in POSIX terms.
  - A Windows session's replay, after an attach, a reconnect or a core restart, then has no working directory until the shell reports one again.

## What Changes

- **Event channel.**
  - `Wakeup`, title changes, foreground reports and clipboard writes are merged at the source, so each has at most one entry waiting (clipboard writes, one per selection). The latest value wins.
  - Events the terminal ignores are no longer queued.
  - The rest are counted against a high mark of 2048 waiting events and a low mark of 512.
  - Past the high mark, until the count falls under the low one, a terminal drops its side events: bells, notifications, status reports and replies to the program's queries. Output, which goes straight into the grid, and life events (replay start and end, exit, loss, failure) are never dropped.
  - If a status report was dropped, the terminal clears its status records rather than show a stale dot.
- **Input.** Each session holds at most about 4 MiB of input not yet sent, while it starts and while its link is down or slow.
  - Once 4 MiB waits, further input is not sent until less than 1 MiB waits.
  - The first refusal after each crossing writes a dim line in the tab.
- **Holder.**
  - It stops reading PTYs at 8 MiB queued to the core, as today, and resumes only under a new low mark of 2 MiB. The two copies of that read policy become one pure function.
  - Each message from the core is handled under `catch_unwind`:
    - a failed `SPAWN` is answered with `SPAWN_FAILED`, naming the failure;
    - a failure on any other message is sent to the core as an `ERROR`, for the message's session when it has one. The session and the core connection carry on, and a checkpoint being assembled for that session is dropped.
- **Brindle's reader threads.** Every call into a session's sink runs under `catch_unwind`.
  - A panic detaches that session (`DETACH`), which keeps its program running.
  - Its tab replaces the screen with a line saying why, and that the session can be attached again.
  - The endpoint's other tabs keep working.
- **Refusals** in the core and the holder close the connection gracefully (`Conn::close_for`), so the peer reads the `ERROR` and then the end.
- **Latency samples** keep the last 10,000 of each list. The summary says how many were dropped.
- **Config writes.** The default config is written to a temporary file and renamed into place, through a helper that `config_edit::save` shares.
- **Test switches:** `BRINDLE_TEST_SINK_PANIC` (Brindle) and `BRINDLE_TEST_HOLDER_PANIC` (holder), both in debug builds only (`process R20`).
- **Replays keep a Windows session's directory.** `Emulator::new` takes the `PathStyle` (change 5) of the platform its program runs on. The core passes a `server::platform` constant: `Windows` on Windows, `Posix` on Linux. So a Windows core keeps OSC 9;9 directories, and its replays carry them. Linux is unchanged.
- **No wire change:** no protocol version, message, field, config key or state-file change.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `remote-connections`:
  - MODIFIED "Sessions come back after a reconnect": input typed while disconnected is delivered unless it was refused under the input bound. The text is the one change 2 leaves (see Impact).
  - ADDED "Input waiting to reach a session is bounded": the 4 MiB and 1 MiB marks, and the tab's line.
- `terminal-emulation`:
  - MODIFIED "A session lost for good is reported in the tab": a tab let go after an emulator failure doesn't act on a session that later got its id, like a lost one. The text is the one change 2 leaves.
  - ADDED "An emulator failure is confined to its tab": the session is detached and still running, the tab says why, and other tabs, on that endpoint too, keep working.
- `terminal-server`:
  - MODIFIED "Failures stay small": a failure while a holder handles one session's message ends neither the holder nor its other sessions.
  - MODIFIED "Attach replays the screen": the replay includes the working directory the program last reported, read as its platform's paths, so a Windows session keeps an OSC 9;9 directory across an attach or a core restart.

Each new behaviour is its own ADDED requirement rather than more text in the modified ones, since change 2's descriptions are already near the 500-character limit that `openspec validate --strict` checks.

## Impact

- **Depends on change 2 (`carry-session-identity-and-host`), which lands first.**
  - Both deltas here modify requirements that change 2 also modifies. Each is copied in full as change 2's delta leaves it, then changed.
  - The client code builds on change 2's `Shown` entries and on `State::forget(id, key)`.
- **Depends on change 5 (`enforce-platform-seams`), which lands first.** It adds term-core's `PathStyle` and `Parser::new(style)`, and has `Emulator::new` choose `PathStyle::Posix` explicitly. This change resolves the Open Question it leaves.
- **Code:**
  - Brindle:
    - `src/queue_marks.rs` (new, pure);
    - `src/terminal/events.rs` (new, pure);
    - `src/remote/budget.rs` (new, pure);
    - `src/terminal/mod.rs` and `src/terminal/server.rs`;
    - `src/remote/local/connection.rs` and `session.rs`;
    - `src/latency.rs`, `src/config.rs` and `src/config_edit.rs`.
  - Helper:
    - `remote-agent/src/server/fault.rs` (new);
    - `server/holder.rs`, `server/core/screen.rs` and `server/core/clients.rs`.
  - `remote-protocol/src/holder.rs`: `Assembler::abandon`, a codec helper. The encoding is unchanged.
  - The path style:
    - `term-core/src/emulator.rs`: `Emulator::new` takes a `PathStyle`;
    - `remote-agent/src/server/platform/{unix,windows}/pty.rs`: the `PATH_STYLE` constant;
    - `core/screen.rs` and `remote-agent/tests/support/mod.rs` pass it.
- **Tests:**
  - unit tests in each new module, `connection.rs` (a `fake::link` panicking sink and the input bound), `session.rs`, `latency.rs` and `config.rs`;
  - real-process tests in `remote-agent/tests/holder.rs` (a holder fault, a refused connection) and `core.rs` (refused clients);
  - the e2e case `local-server-sink-panic`.
- **Docs:**
  - `process-architecture.md` R9: the marks named here, and holder input being lossy past its cap;
  - R14: the event gate and input budget locks are leaves;
  - CLAUDE.md's architecture paragraphs on the event channel, Brindle's client and the holder.
- **Audit entries closed:**
  - `process R9`: b24 #2, b28 #9, b25 #19, b12's holder queue entry, and the b01 and b26 smells;
  - `process R3`: b24 #3 and the b12 smell;
  - `process R10`: b10 #14 and b12's refusal entry;
  - `process R15`: b23's `config.rs:434`;
  - `fcis R1`: b12's duplicated backpressure policy, since its two sites change anyway.
- **Not in scope:**
  - Showing a server `ERROR` for a session (a holder fault, or the holder's "dropped some" input refusal) in its tab. Today they are only logged. That is change 12 (`surface-swallowed-failures`, which depends on this change), with the other `railway R6` gaps.
  - The core's client-queue hysteresis (b10 #7) stays where it is, for change 13.
- **Overlap:**
  - Change 3 (`bound-and-fuzz-outside-input`) also edits `holder.rs`'s checkpoint arms (the `Assembler` cap). Whichever lands second rebases.
  - Change 13 (`extract-decisions-into-models`) folds `Terminal`'s lifecycle flags into one enum, which takes in the emulator failure added here, and may reuse the hysteresis rule for the core's client queue.
  - `add-blocks` edits `src/terminal/mod.rs` (view position), and `port-to-macos` edits `src/config.rs` (the config path). Both are textual overlaps only: neither touches the event channel or `write_default`.
  - `update-dependencies` and `add-pocket-client` don't overlap.
