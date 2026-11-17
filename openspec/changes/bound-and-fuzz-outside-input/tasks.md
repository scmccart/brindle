# Tasks

Each group writes its tests first and watches them fail before the code they check. Test names are sentences (`testing R15`). Nothing sleeps or asserts on elapsed time (`testing R7`, `R8`), and every generator has a fixed seed.

## 1. H1 open-blob bound (shared contract, `remote-protocol`)

- [ ] 1.1 Add unit tests to `remote-protocol/src/holder.rs`'s `mod tests`, and verify they fail first (`cargo test -p brindle-remote-protocol holder::`):
  - `a_blob_past_the_open_limit_is_refused_and_the_others_stay_open`:
    - `MAX_OPEN_BLOBS` distinct sessions begin and `open_blobs()` equals the limit;
    - one more `begin` is `InvalidData` and inserts nothing;
    - each of the first blobs still chunks and ends to its bytes.
  - `ending_a_blob_makes_room_for_another`.
- [ ] 1.2 Add `pub const MAX_OPEN_BLOBS: usize = 16` beside `MAX_CHECKPOINT_LEN`, and `Assembler::open_blobs(&self) -> usize`. In `Assembler::begin`, after the "began a blob before ending the last one" and over-long checks, refuse a new session when `open.len() >= MAX_OPEN_BLOBS`, with `wire::invalid` ("session {s} began a blob while {n} are open; the limit is {MAX_OPEN_BLOBS}") (design §1). Verify that 1.1 and the existing `checkpoints_larger_than_a_frame_are_chunked` and assembler tests pass.
- [ ] 1.3 In `remote-protocol/src/fuzz.rs`, widen `the_assembler_never_panics_on_random_sequences` to session ids `rng.below(64)` and assert `open_blobs() <= holder::MAX_OPEN_BLOBS` after every step. Verify with `cargo test -p brindle-remote-protocol fuzz::`.
- [ ] 1.4 Document the bound:
  - **H1 module doc:** in `holder.rs`'s "Checkpoints and tails are chunked" paragraph, say that a receiver keeps at most `MAX_OPEN_BLOBS` blobs of one kind open per connection, refuses a `*_BEGIN` past it, and that every shipped sender has at most one open.
  - **`CLAUDE.md`:** in the "Protocols" bullet ("Large checkpoints/tails are chunked …"), add the same bound in a few words.

  Verify that `cargo doc -p brindle-remote-protocol --no-deps` builds with no broken intra-doc link.

## 2. Transcripts: shipped fixtures frozen, coverage per variant (shared contract)

- [ ] 2.1 In `remote-protocol/tests/transcripts.rs`, make regeneration and the module doc enforce the freeze (design §7):
  - `read_fixtures` writes a fixture under `BRINDLE_REGEN_FIXTURES=1` only when its file doesn't exist.
  - `encoding_the_messages_reproduces_the_fixtures` fails a differing fixture with a message saying shipped fixtures are frozen (`process R5`): add a new transcript, or delete a never-shipped fixture and regenerate.
  - The module doc says the file pins client protocols 1–3, the relay link and H1, and states the regeneration rule.

  Verify that `cargo test -p brindle-remote-protocol --test transcripts` passes, and that `BRINDLE_REGEN_FIXTURES=1 cargo test -p brindle-remote-protocol --test transcripts` leaves `git status remote-protocol/tests/fixtures` clean.
- [ ] 2.2 Replace `every_message_type_appears_in_a_transcript` with `every_message_variant_and_form_appears_in_a_transcript` (design §6):
  - `Step::forms()` matches exhaustively, with no `_ =>` (`railway R3`), over:
    - `client::ToCore`, `client::ToClient`, `holder::ToHolder`, `holder::ToCore` and `relay::RelayMessage`;
    - the nested `client::Policy`, `client::KillTarget` and `holder::Previous`;
    - the `Exit.lost` and `HeldSession.exit` → `ExitStatus.code` `Option` forms.
  - Each enum's match and its list of expected names come from one `macro_rules!` invocation.

  Verify that the test fails, naming `holder::Previous::Crash`, `client::Exit.lost=Some` and `holder::ExitStatus.code=None`, which are the audit's b22 #1–#3.
- [ ] 2.3 Add two transcripts to `transcripts()` (design §5), and leave `holder-adopt` and every other existing fixture alone:
  - `client-session-lost` (`Protocol::Client`): `HELLO`, `WELCOME`, `SPAWN`, `SPAWNED`, `DATA`, then `EXIT { code: None, lost: Some("its holder ended") }`.
  - `holder-adopt-after-crash` (`Protocol::Holder`):
    - a core's `HELLO`;
    - a `WELCOME` with `previous: Previous::Crash` and two held sessions, one running and one with `exit: Some(ExitStatus { code: None })`;
    - each session's checkpoint and tail, through `adopted`.

  Regenerate, then verify:
  - `git status remote-protocol/tests/fixtures` shows only the two new `.hex` files;
  - all three transcript tests pass, 2.2's included.
- [ ] 2.4 Fix the other stale protocol docs and the relay's collision check:
  - `remote-protocol/src/lib.rs` ("protocols 1 and 2" → 1–3) and `relay.rs`'s module doc ("protocol 1 and 2 frames");
  - in `relay_numbers_collide_with_no_other_protocol`, the client range becomes `MSG_DATA..=MSG_SET_LAYOUT`, and its comment "1-14 and 32-41".

  Verify with `cargo test -p brindle-remote-protocol relay::`.
- [ ] 2.5 In `docs/architecture/process-architecture.md`'s Enforcement section, the transcript bullet says transcripts are checked per message variant and nested form, and that regeneration never rewrites an existing fixture (R5, R6). This is a description of enforcement, not a rule change. Verify that the bullet names the test from 2.2.

## 3. Fuzz targets for the protocol crate's parsers (`remote-protocol/src/fuzz.rs`)

- [ ] 3.1 Add `skip_preamble_finds_the_first_marker_or_fails_within_its_bound` (design §4):
  - **Input:** noise with the marker whole, split, partial, repeated or missing, read through an in-memory `Read` that returns short reads and `ErrorKind::Interrupted`.
  - **On `Ok`:** the noise is the bytes before the first marker, and the reader is left just past it.
  - **Always:** it never consumes more than `MAX_PREAMBLE_NOISE + FRAME_STREAM_MARKER.len()` bytes, and an error is `UnexpectedEof` or `InvalidData`.

  Verify with `cargo test -p brindle-remote-protocol fuzz::skip_preamble`.
- [ ] 3.2 Add `session_meta_reads_any_value_and_round_trips_what_it_returns`:
  - **Input:** random JSON values, depth-limited, with keys drawn from `SessionMeta`'s field names and random ones, wrong-typed, huge and negative numbers, and mutated `holder::samples::meta()`.
  - **Assert:** `from_value(&m.to_value()) == m` for every `m` that `from_value` returns.

  Verify with `cargo test -p brindle-remote-protocol fuzz::session_meta`.
- [ ] 3.3 Add `versions_parse_without_panicking_and_newer_is_never_mutual`:
  - **`upgrade::semver`** gets random strings over digits, `.`, `-`, `+`, whitespace and random bytes. A `Some((a, b, c))` must re-parse from `"{a}.{b}.{c}"`.
  - **`upgrade::is_newer`** gets random `Build` pairs (mtimes from `SystemTime::UNIX_EPOCH` plus random seconds). It is never true both ways.

  Verify with `cargo test -p brindle-remote-protocol fuzz::versions`.
- [ ] 3.4 Add `holder_lock_names_parse_without_panicking_and_round_trip`:
  - **`server_dir::parse_holder_lock`** gets random names over `holder-.lock`, digits and random bytes, and must not panic.
  - **A built name** `format!("holder-{build}-{n}.lock")`, for any non-empty `build` and any `u32`, parses back to `(build, n)`.

  Verify with `cargo test -p brindle-remote-protocol fuzz::holder_lock`.
- [ ] 3.5 Run the crate's tests and the architecture test:
  - `cargo test -p brindle-remote-protocol` passes;
  - `cargo test -p brindle --test architecture the_wire_modules_do_no_io` still passes, so `fuzz.rs` names no `fs::`, `env::`, `process::` or clock;
  - each new test's run time in a debug build is recorded for the PR body.

## 4. term-core fuzz tests (`brindle-term-core`)

- [ ] 4.1 Create `term-core/src/fuzz.rs`, declared `#[cfg(test)] mod fuzz;` in `term-core/src/lib.rs`, holding:
  - a copy of the fixed-seed `XorShift` (`new`, `next`, `below`, `bytes`), whose doc comment names `remote-protocol/src/fuzz.rs` and says why it is a copy (`ports R3`);
  - a stream builder biased toward OSC 7 `file://` paths with `%XX` escapes, OSC 9;9, OSC 133 `A`/`B`/`C`/`D[;n]`, OSC 7501 bodies, `ESC c`, `CSI ? 1049 h|l`, `CSI 3 J` and `CSI J`, with every terminator (BEL, `ESC \`, CAN, SUB, a bare ESC), random cuts, splices and noise.

  Verify:
  - `cargo test -p brindle-term-core` builds;
  - `cargo test -p brindle --test architecture term_core_does_no_io_and_reads_no_clock` passes.
- [ ] 4.2 Add `the_emulator_never_panics_on_random_streams` to `fuzz.rs`:
  - small `Emulator`s are fed random streams in random-size pieces, with `resize` and `input_seen` in between;
  - after each piece, `records().iter().count() <= status::MAX_RECORDS`;
  - at the end, `render()` is fed to a fresh `Emulator` of the same size and scrollback.

  Verify with `cargo test -p brindle-term-core fuzz::`.
- [ ] 4.3 Add `the_osc_scanner_finds_the_same_sequences_however_the_stream_is_cut`. It checks `osc::Scanner::scan` on a stream fed whole against the same stream fed in random pieces. Verify with `cargo test -p brindle-term-core fuzz::`.
- [ ] 4.4 Add `status_reports_survive_encode_and_a_second_parse`. `status::parse` gets random bodies, and every `Message::Report { id, record }` re-parses from `record.encode(&id)` to the same message. Verify with `cargo test -p brindle-term-core fuzz::`.
- [ ] 4.5 In `term-core/src/shell_integration.rs`'s `mod tests`, add `reported_dirs_are_absolute_in_either_path_style_for_any_body`. It uses `crate::fuzz`, and:
  - feeds random OSC bodies to `reported_dir(body, false)` and `reported_dir(body, true)`;
  - asserts that a `Some` is absolute for its style;
  - also feeds the bodies to `parse_osc133`, which must not panic.

  If change 5 (`enforce-platform-seams`) has landed, use its path-style argument instead of the bool. Verify with `cargo test -p brindle-term-core shell_integration::`.
- [ ] 4.6 Handle any panic these tests find (design, Risks):
  - **In term-core's own code:** fix it, and add a unit test named for the input in the file that panicked.
  - **Inside `alacritty_terminal`:** stop and raise it, without narrowing the generator.

  Verify that `cargo test -p brindle-term-core` passes, and record each new test's run time for the PR body.
- [ ] 4.7 Update the docs that say where fuzz tests live:
  - `docs/architecture/testing-architecture.md`'s layer table: the fuzz-tests row names `remote-protocol/src/fuzz.rs` and `term-core/src/fuzz.rs`;
  - `railway-oriented-programming.md`: the term-core row of "How it applies here" says the fuzz tests check that no input panics, and the Enforcement bullet names both files;
  - the "Tests" paragraph of `CLAUDE.md`'s terminal-server section names the term-core fuzz tests beside `remote-protocol/src/fuzz.rs`.

  These describe where the tests live and change no rule, so `tests/architecture.rs` is unchanged. Verify by checking that each path named exists.

## 5. `TextInput` range ordering (`brindle-ui`, pure model)

- [ ] 5.1 Add unit tests to `ui/src/text_input.rs`'s `mod tests`, and verify they fail first (`cargo test -p brindle-ui text_input`):
  - `a_reversed_utf16_range_is_ordered`: `EditState::new("aé😀b").range_from_utf16(&(4..1)) == 1..7`.
  - `every_utf16_range_slices_the_text`: for every `a` and `b` in `0..=8`, past the end included, the range is ordered and `text().get(range)` is `Some`.
- [ ] 5.2 Make `EditState::range_from_utf16` return `min..max` of the two converted ends (design §2). Verify that 5.1, `utf16_conversions` and `composition` pass with `cargo test -p brindle-ui`.

## 6. Server roles keep their error paths (real-process)

- [ ] 6.1 In `remote-agent/tests/holder.rs`, add `a_core_past_the_open_blob_limit_gets_an_error_and_the_sessions_keep_running`. It is for both platforms, with a session started from `programs::`.
  - The test core starts a session, then sends `CHECKPOINT_BEGIN` for `MAX_OPEN_BLOBS + 1` distinct session ids.
  - It reads an H1 `ERROR` for the last one.
  - The session's input still echoes.
  - A reconnecting core still adopts the session with its previous checkpoint.

  It waits by reading frames, never by sleeping. The core's side, which drops a holder past the cap, is the existing `drop_holder` path for a malformed blob, held by 1.1's unit tests. Verify with `cargo test -p brindle-remote-agent --test holder open_blob`.

## 7. Integration

- [ ] 7.1 On Linux, `cargo test --workspace --locked` passes: unit, fuzz, transcript, architecture and real-process suites, run in parallel with the default `--test-threads`.
- [ ] 7.2 On Windows, push the branch and read `build-windows.yml`'s `cargo test` step. It runs 1.x, 3.x and 4.x and the holder test from 6.1. Report it as Windows' first run of these tests (`testing R17`), labelled as CI's result.
- [ ] 7.3 Manual check, since only real keys show it (`testing R16`): with `scripts/run-throwaway.sh`, compose `é` with IBus (ctrl-shift-u `e9` space) in the command palette's input and in a terminal. List it in the PR body for the user to run.
- [ ] 7.4 `openspec validate bound-and-fuzz-outside-input --strict` passes.

## Workflow follow-up

- The archive commit (`docs(openspec): …`) goes on the implementation PR. The change sets `skip_specs: true`, so archiving syncs no spec.
