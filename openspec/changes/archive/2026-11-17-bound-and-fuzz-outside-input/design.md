# Design

## Context

See proposal.md (Why). This change closes the audit's finding 4 (F4) and every instance list entry under it:
- **`railway R5`:** b20 #10 (`Assembler`), b43 #1 (`text_for_range`) and b31 #9 (term-core parsers with no fuzz test).
- **`process R6`:** b20 #11 (four parsers with no fuzz target) and b22 #1–#3 (transcript gaps).
- **b22's design smells:** coverage by message type, `BRINDLE_REGEN_FIXTURES` rewriting shipped fixtures, and the stale module doc.

The code is unchanged since the audit (`ab6462b`, re-checked on `39bb579`). Five facts shape the approach.

**Who uses `holder::Assembler`.** It is used on both ends of H1:
- **The core**, for checkpoints and tails from a holder (`remote-agent/src/server/core/holders.rs`, `Holder.checkpoints`/`tails`, through `blob`). An `Err` makes the core call `drop_holder(token, "bad {kind}: {error}")`, which drops the holder's connection, and with it the core's sessions on that holder.
- **The holder**, for checkpoints from its core (`remote-agent/src/server/holder.rs`, `Core.checkpoints`, the `ToHolder::Checkpoint*` arms). An `Err` there becomes an H1 `ERROR` for that session, and nothing is dropped.

**Every shipped sender sends a blob as one contiguous run.** That is `ToHolder::checkpoint` queued in a loop in `core/sessions.rs` `checkpoint`, and `ToCore::checkpoint` then `ToCore::tail` in `holder.rs`'s `WELCOME` path. So no shipped peer ever has more than one blob of a kind open on a connection. The H1 module doc nonetheless allows "chunks of one blob may interleave with other sessions' messages".

**The UTF-16 range conversion.** `EditState::range_from_utf16` (`ui/src/text_input.rs:277`) maps each end on its own through `from_utf16`, which always returns a char boundary. A reversed range therefore yields `start > end`, and `text_for_range`'s `text()[range]` panics. `replace` and `replace_and_mark` already survive one through `clamp(..).max(start)`.

**The existing fuzz tests and their architecture checks.** `remote-protocol/src/fuzz.rs` is a `#[cfg(test)]` module with a fixed-seed `XorShift`, and `tests/architecture.rs` checks it:
- `the_wire_modules_do_no_io` scans it for the `IO` and `CLOCK_AND_THREADS` needles;
- `term_core_does_no_io_and_reads_no_clock` scans every file under `term-core/src`, so a new term-core fuzz file is held to the same.

**Where the parsers live.**
- `skip_preamble` is `remote-protocol/src/lib.rs:66`, called from `src/remote/local/relay.rs:173`. The plan's `src/remote/local/link.rs` is wrong.
- `SessionMeta::from_value` is `holder.rs:670`, `semver` is `upgrade.rs:32` and `parse_holder_lock` is `server_dir.rs:127`.
- term-core's parsers:
  - `shell_integration.rs` holds `Scanner` (OSC 133, plus OSC 7 and 9;9 through the private `reported_dir`, `percent_decode` and `parse_osc133`);
  - `osc.rs` holds `Scanner` (OSC 9, 777, 7501, 133 `A`, `ESC c`);
  - `status.rs` holds `parse`;
  - `emulator.rs` puts them together as the core's per-session feed, and the client's `SessionSink` runs the same `Parser` and `osc::Scanner`.

## Goals / Non-Goals

**Goals:**
- No sequence of H1 blob frames can make an `Assembler` hold more than a fixed number of partial blobs.
- No UTF-16 range from a platform input handler can panic `TextInput`.
- Every parser of outside bytes named in F4 has a seeded fuzz test that asserts it never panics and asserts a property of its output.
- The variant-level coverage gaps get transcripts. Coverage is checked per variant and nested form. A shipped fixture can't be rewritten by regeneration.

**Non-Goals:**
- Changing what any decoder accepts beyond the open-blob bound. In particular, `SessionMeta::from_value` keeps its defaults; change 12 (`surface-swallowed-failures`) makes it return a refusal.
- `catch_unwind` around the client's `SessionSink` or the holder's handlers (change 9).
- Making `Parser`'s path style an argument (change 5). The fuzz test uses whatever constructor exists when it lands.
- Allocating blobs lazily, a cap on total announced bytes, or a `PathStyle` enum.
- Pinning `Process { name: None }` (b22's minor smell). Optional fields beyond the three gaps are left to the variant table's next user.
- Coverage-guided fuzzing (`cargo fuzz`), which would add a toolchain and a dependency. The seeded generators run in plain `cargo test`.

## Layers

| Part | Layer | Crate |
| --- | --- | --- |
| `Assembler`'s open-blob bound, `MAX_OPEN_BLOBS`, `open_blobs()` | pure codec state (no IO, clock or thread; `process R6`) | `brindle-remote-protocol` |
| `EditState::range_from_utf16` ordering | pure model (`fcis R1`; the entity's `EntityInputHandler` methods only call it) | `brindle-ui` |
| fuzz targets for `skip_preamble`, `SessionMeta`, `semver`/`is_newer`, `parse_holder_lock` | fuzz tests (`testing`'s layer table) | `brindle-remote-protocol` (`src/fuzz.rs`) |
| fuzz tests for the OSC 7, 9;9, 133 and 7501 parsers and `Emulator` | fuzz tests | `brindle-term-core` (new `src/fuzz.rs`; `reported_dir`'s in `shell_integration.rs`'s tests) |
| new fixtures, per-variant coverage, regeneration rule | transcript tests | `brindle-remote-protocol` (`tests/transcripts.rs`) |
| the core's `blob` and the holder's `CheckpointBegin` arms | shell, unchanged | `brindle-remote-agent` |

No decision moves into a shell or a seam, and no crate reference changes (`ports R1`, `R3`).

## Decisions

### 1. Cap open blobs by count at `MAX_OPEN_BLOBS = 16`, refused in `begin`

`Assembler::begin` checks three things in order, and only an accepted `begin` inserts:
1. A blob already open for that session is refused, as today, and dropped.
2. An over-long blob is refused, as today.
3. When `open.len() >= MAX_OPEN_BLOBS`, the frame is refused with `wire::invalid` ("session {s} began a blob while {n} are open; the limit is {MAX_OPEN_BLOBS}"). The other open blobs are left alone.

`MAX_OPEN_BLOBS` is a `pub const` beside `MAX_CHECKPOINT_LEN` and `TAIL_LIMIT`. `Assembler::open_blobs(&self) -> usize` lets the fuzz test check the bound. The H1 module doc's "Checkpoints and tails are chunked" paragraph says a receiver keeps at most `MAX_OPEN_BLOBS` blobs of one kind open per connection, and refuses a `*_BEGIN` past that. This is `process R7`'s "a limit per kind", written down.

- **Why a count:** the bound has to stop both the `HashMap` growth and the up-front allocation (`min(len, MAX_CHUNK_LEN)` per blob). A count stops both: at most 16 × `MAX_CHUNK_LEN` (about 16 MiB) is reserved per assembler before any chunk arrives. Bytes that do arrive are the sender's own, and each blob is still capped at its `limit`.
- **Why 16, not 1:** the H1 doc permits blobs of different sessions to interleave, though no sender does. A cap of 1 would make a sender the doc allows into a broken one. 16 leaves room and still bounds memory.
- **Alternatives:**
  - Lazy allocation (`Vec::new()`) removes the amplification but not the map's growth.
  - A cap on total announced bytes needs more state, and one legitimate 64 MiB checkpoint uses most of it.
  - Capping at the session count: the assembler doesn't know it, and on the core side the sessions are being adopted from the same untrusted `WELCOME`.

**What a refusal does** is unchanged per side (`railway R6`, `process R3`):
- **In the core:** a holder past the cap is dropped like any holder that sends a malformed blob. Its sessions end, and attached clients get the "session lost" `EXIT`.
- **In the holder:** a core past the cap gets an H1 `ERROR` for that session's checkpoint. The holder keeps its previous checkpoint and every session.

**Versions (`process R4`–`R6`):** no message, field or version changes. The bound narrows what a receiver accepts, and every shipped sender is already within it. In particular:
- a new core meeting an old holder, or an old core meeting a new holder, sees the same frames as today;
- no transcript changes, since the encoding doesn't;
- a sender that some day needs more than 16 open blobs must negotiate a new H1 version.

**Restart, crash and upgrade (`process R1`–`R3`, `R12`):** the bound is per connection and in memory only.
- **A core restart:** the new core starts with empty assemblers and adopts holders as before.
- **A Brindle crash:** no effect; Brindle has no `Assembler`.
- **An upgrade:** a mixed pair behaves as above.
- **The holder:** it gains no policy (`process R1`). The check sits in the codec type it already uses, and refuses through the error path it already has.

### 2. Order the range in `range_from_utf16`

`range_from_utf16` returns `min(a, b)..max(a, b)` of the two converted ends.
- It is the one place every platform range enters `EditState`: `text_for_range`, `bounds_for_range`, `replace_text_in_range` and both of `replace_and_mark_text_in_range`'s ranges. Ordering it there covers each call.
- Alternative: `text()[range]` → `text().get(range)?` in `text_for_range` alone. That fixes the panic, but leaves the other callers each to guard reversed ranges their own way.
- Side effect: a reversed `selected_utf16` given to `replace_and_mark_text_in_range` becomes a forward selection instead of a backward one. No GPUI 0.2.2 backend sends one ("Finding 4b traced"), and macOS's `NSRange` can't express one, so nothing observable changes and `text-input` needs no delta.

### 3. term-core gets its own `fuzz.rs`, with a copied generator

- **Why a copy:** term-core can't use `remote-protocol`'s `XorShift`. The shared crates never reference each other, and `crates_below_the_app_name_only_their_allowed_dependencies` checks dev-dependencies too (`ports R3`).
- **Rejected:** a sixth workspace crate for test support would join `process R21`'s version alignment and release-please, which is too much for about 25 lines.
- **What the file holds:**
  - the generator (`XorShift`, `below`, `bytes`);
  - a stream builder biased toward the sequences under test, with random cuts, splices and terminators (BEL, `ESC \`, CAN, SUB, a bare ESC):
    - `ESC ] 7;file://<host>/<path with %XX escapes>`;
    - `ESC ] 9;9;<path>`;
    - `ESC ] 133;A|B|C|D[;n]`;
    - `ESC ] 7501;<key=value:…>`;
    - `ESC c`;
    - `CSI ? 1049 h|l`;
    - `CSI 3 J`;
    - `CSI J`.
  - The stream-level tests, which use only public API.
- **`reported_dir`, `percent_decode` and `parse_osc133`:** they are private, so their fuzz test lives in `shell_integration.rs`'s own `mod tests`, which uses `crate::fuzz`. This fuzzes the Windows path style on Linux without widening visibility and without waiting for change 5.
- **The architecture checks:** `term_core_does_no_io_and_reads_no_clock` covers the new file. Seeds are constants, and nothing reads a clock (`fcis R2`, `R4`, `testing R7`).

### 4. Drive `Emulator`, and assert properties, not just "no panic"

`testing R8` says assert the rule. Each target asserts that nothing panics and also:

| Target | Property |
| --- | --- |
| `Emulator::feed` in random pieces, with `resize` and `input_seen` interleaved | records stay within `status::MAX_RECORDS`; `render()` fed to a fresh `Emulator` of the same size doesn't panic |
| `osc::Scanner::scan` | a stream scanned whole and scanned in random pieces finds the same `Found` list (the state carries across calls) |
| `status::parse` | every `Report { id, record }` re-parses from `record.encode(&id)` to the same message (`encode`'s documented contract) |
| `reported_dir(body, windows)` for both styles, and `parse_osc133` | a `Some` path is absolute for its style (`/…`, or a drive or UNC path) |
| `skip_preamble` over a reader that returns short reads and `Interrupted` | on `Ok`, the noise is the bytes before the first marker and the reader is left right after it; it never reads past `MAX_PREAMBLE_NOISE` + the marker; errors are `UnexpectedEof`, `InvalidData` or the reader's own |
| `SessionMeta::from_value` on random JSON values and mutated `samples::meta()` | `from_value(&m.to_value()) == m` for every `m` it returns |
| `upgrade::semver`, `is_newer` | a `Some((a, b, c))` re-parses from `"{a}.{b}.{c}"`; `is_newer(x, y) && is_newer(y, x)` never holds |
| `server_dir::parse_holder_lock` | `parse_holder_lock(&format!("holder-{build}-{n}.lock")) == Some((build, n))` for any non-empty `build` |
| `Assembler` (the existing test, session ids widened past the cap) | `open_blobs() <= MAX_OPEN_BLOBS` after every step |

- **Why `Emulator`:** it is the core's feed and the client's parse path in one: the same `Parser`, `Marks`, `osc::Scanner` and `Records`. The `MAX_COMMANDS` bound stays private, so the test doesn't assert it.
- **`remote-protocol/src/fuzz.rs` is scanned by `the_wire_modules_do_no_io`:** the new targets write no `fs::`, `env::` or `process::` text, even in comments. `skip_preamble` reads from an in-memory `Read` the test defines.

### 5. The transcript gaps go into new fixtures, never into shipped ones

b22 #3's suggested fix edits `holder-adopt`, but that fixture has shipped, so `process R5` freezes it. The gaps go into two new fixtures instead:
- **`client-session-lost`** (`Protocol::Client`): `HELLO`, `WELCOME`, a `SPAWN` and `SPAWNED`, output, then `EXIT { code: None, lost: Some("its holder ended") }`. That is the core's real text, from `core/sessions.rs:430`.
- **`holder-adopt-after-crash`** (`Protocol::Holder`): a core's `HELLO`, then a holder's `WELCOME` with `previous: Previous::Crash` and two held sessions, one running and one with `exit: Some(ExitStatus { code: None })`. Each session's checkpoint and tail follow, through the existing `adopted` helper.

Every existing fixture stays byte for byte.

### 6. Coverage by variant, from exhaustive matches

`every_message_type_appears_in_a_transcript` becomes `every_message_variant_and_form_appears_in_a_transcript`.
- **The forms:** `Step::forms()` returns the step's variant name, such as `client::ToClient::Data`, plus the nested forms it carries. It matches exhaustively, with no `_ =>` arm (`railway R3`), over:
  - `client::ToCore`, `client::ToClient`, `holder::ToHolder`, `holder::ToCore` and `relay::RelayMessage`;
  - `client::Policy` (in `Spawn` and `Sessions`' `SessionInfo`) and `client::KillTarget` (in `Kill`);
  - `holder::Previous` (in `Welcome`);
  - `Exit.lost` (`None`/`Some`) and `HeldSession.exit`'s `ExitStatus.code` (`Some`/`None`).
- **The expected names:** each enum's match and its list of names come from one small `macro_rules!` invocation. A new variant then fails to compile until it is named, and once named it is required in a transcript. The two `Option` forms are listed by hand beside their match.
- **What it fixes:** directions stay apart (`Data` both ways), and a message number added past a range no longer slips through.
- **Rejected:** `strum`'s `EnumIter` would add a dependency to a crate with a closed list (`ports R3`).

### 7. Regeneration writes only missing fixtures

With `BRINDLE_REGEN_FIXTURES=1`, `read_fixtures` writes a fixture only when its file doesn't exist. An existing fixture that differs from the rendered transcript fails `encoding_the_messages_reproduces_the_fixtures`. Its message says a shipped fixture is frozen (`process R5`): add a new transcript, or, if the fixture never shipped, delete it and regenerate. The module doc says this, and that the file pins client protocols 1–3, the relay link and H1.

The same staleness is fixed in two other places:
- the "protocols 1 and 2" in `remote-protocol/src/lib.rs` and `relay.rs`;
- `relay_numbers_collide_with_no_other_protocol`, which checks `MSG_DATA..=MSG_REPARENT` and so skips `SET_LAYOUT` (14). It moves to `..=MSG_SET_LAYOUT`.

### 8. Docs name where fuzz tests live, and no rule changes

- **No rule changes:** `railway R5` ("every decoder has a fuzz test") and `process R5`–`R7` are applied as written, so `tests/architecture.rs` gains no check and loses none.
- **Docs that say where fuzz tests live:**
  - `testing-architecture.md`'s layer table names `remote-protocol/src/fuzz.rs` and `term-core/src/fuzz.rs`;
  - `railway-oriented-programming.md`'s "How it applies here" row for term-core and its Enforcement bullet name the term-core fuzz tests;
  - `process-architecture.md`'s Enforcement bullet says transcripts are checked per variant, and that regeneration never rewrites an existing fixture.
- **`CLAUDE.md`:** its protocol paragraph gains the open-blob bound, and its Tests paragraph gains the term-core fuzz tests.

### 9. No grid is narrower than `MIN_COLS = 10`

- **Why:** the emulator fuzz test found a panic inside `alacritty_terminal` 0.26 (`grid/mod.rs:439`): a double-width character fed to a 1-column grid. Per the Risks rule the generator isn't narrowed. The fix is in the code that builds sizes, so no grid Brindle makes can be that narrow.
- **Where:** one constant, `brindle_term_core::emulator::MIN_COLS`, used by the server emulators' `size()` (`new` and `resize`), the client's `Terminal::resize` and replay size (`GridSize::clamped`), `fit_grid` and `can_split_zone`. Client and core clamp alike, so their grids and replays stay identical. Rows keep their own minimums.
- **Wire:** unchanged. A size from a listing or a request is clamped where it is used; the PTY gets the clamped size.
- **Spec:** the `zones` requirement "Minimum size for splitting" named 2 columns; its delta here says 10 and adds the scenario "a pane narrower than 10 columns is laid out as 10 columns wide".

## Risks / Trade-offs

- **[A fuzz test finds a real panic]** → If it is in Brindle's code, fix it in this change, since finding it is the point. If it is inside `alacritty_terminal`, stop and raise it, and don't narrow the generator to hide it (`testing R9`). The core already confines a feed panic per session (`screen.rs`), and change 9 adds the same to the client.
- **[The fuzz tests slow `cargo test`]** → Fixed iteration counts sized to about a second each in a debug build. The PR reports the measured times, and nothing asserts on them (`testing R8`).
- **[The two generator copies drift]** → Each is about 25 lines of a fixed algorithm and is exercised by its own crate's tests. Drift changes coverage, not correctness.
- **[A future sender needs more than 16 open blobs]** → The H1 doc states the bound, and such a sender needs a new negotiated H1 version anyway (`process R5`).
- **[Reworking a fixture that was never merged is now one step longer]** → Delete it and regenerate, as the module doc and the failure message say.
- **[`add-pocket-client` gates `server_dir` for `espidf`]** → The `parse_holder_lock` target takes the same gate. Default features keep it in `cargo test`.

## Migration Plan

- **Deploy:** none. The cap ships in the next helper build, and fleets mix safely, as Decision 1 shows.
- **Rollback:** revert the PR. The two new fixtures go with it, and no shipped fixture or encoding changed.
