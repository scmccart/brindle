---
paths:
  - "transcript/**"
---
# brindle-transcript: the agent transcript both sides fold

You are editing the crate that holds the agent transcript's events and their fold, which Brindle and the server core share. Client protocol 4 carries the events as JSON values that `brindle-remote-protocol` doesn't interpret; this crate gives them meaning on both sides. Read `docs/architecture/functional-core-imperative-shell.md` and `docs/architecture/illegal-states-unrepresentable.md` before non-trivial changes, then `railway-oriented-programming.md` and `process-architecture.md` (R4–R6).

- No filesystem, network, process, environment, thread or clock call anywhere in `src/`, its unit tests and `fuzz.rs` included (`fcis R4`). The only IO is in `tests/`, where the fixture tests read and, when missing, write `tests/fixtures/v4/`. Take values; return values. A time the fold needs is an argument.
- Dependencies are `serde` and `serde_json`, and nothing else (`ports R3`). No `gpui`, no `anyhow`, no other Brindle crate, and never `agent-client-protocol-schema`: ACP is decoded only in `remote-agent/src/acp/`.
- Event forms are frozen per client protocol version (`process R5`): the event kinds and the values of every closed enum inside an event are fixed for protocol 4. Within it an event may only gain optional fields, defaulted and omitted when unset, and `from_value` ignores fields it doesn't know. A new kind or enum value is a new client protocol version.
- The encoding fixtures (`tests/fixtures/v4/`) are written only when missing and never edited, as `remote-protocol`'s are. Never edit one to make a change pass.
- Decoding and folding return this crate's own errors (`DecodeError`, `FoldError`), never a panic, on any input: these values come over the wire (`railway R2`, `R5`). Every decoder and the fold are in `fuzz.rs`.
- No `_ =>` on the crate's own enums, and no `unwrap` or `expect` outside tests; the root's lints hold this (`railway R3`, `R9`).
- Every change ships with unit tests in the same file, with no sleep (`fcis R8`).

If the code needs IO, a clock, ACP's types or GPUI, it belongs in `src/` or `remote-agent/`, and this crate gets the values as arguments.

Enforced by: `tests/architecture.rs` (`transcript_does_no_io_and_reads_no_clock`, `crates_below_the_app_name_only_their_allowed_dependencies`, `only_the_acp_modules_name_the_schema_crate`, `the_crate_roots_enable_the_restriction_lints`), clippy's restriction lints, and the fixture tests.
