---
paths:
  - "remote-protocol/**"
---
# brindle-remote-protocol: the wire formats every version must agree on

You are editing the protocols between Brindle, the server core, the holders and the relay. Old and new builds meet on every upgrade. Read `docs/architecture/process-architecture.md` (R4–R7, R15) before any change, then `illegal-states-unrepresentable.md` and `railway-oriented-programming.md`.

- A shipped encoding is frozen. Within a version, only additive JSON fields: optional, defaulted, omitted when unset, so older transcripts pass byte for byte (`process R5`).
- A new message type or a changed meaning is a new protocol version, sent only on connections that negotiated it (`process R4`, `R5`).
- Every message has a transcript test in both directions, and every decoder is in `fuzz.rs` (`process R6`). Never edit an existing fixture to make a change pass.
- Decoders return `Err` on any input and never panic; frames and blobs have a bound (`railway R5`, `process R7`).
- `client.rs`, `holder.rs`, `wire.rs`, `version.rs` and `relay.rs` are codecs only: no IO, clock or thread (`process R6`).
- Errors are `io::Result` with a meaningful kind or a small enum (`NegotiationError`, `access::Refusal`); no `anyhow` (`railway R2`).
- Dependencies are `serde`, `serde_json` and `windows` only (`ports R3`). Pure helpers that read Windows names are tested on Linux too, so no `Path` on Windows paths (`fcis R7`).

If a change needs session logic or terminal state, it belongs in `remote-agent/` or `term-core/`; this crate only says how the bytes look and where the server's directory, pipes and access checks are.

Enforced by: `tests/architecture.rs` (`the_wire_modules_do_no_io`, `crates_below_the_app_name_only_their_allowed_dependencies`), the transcript tests and `fuzz.rs`.
