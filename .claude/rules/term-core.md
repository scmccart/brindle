---
paths:
  - "term-core/**"
---
# brindle-term-core: the pure terminal state both sides share

You are editing the crate the app and the server core both build their emulators on. Read `docs/architecture/functional-core-imperative-shell.md` and `docs/architecture/illegal-states-unrepresentable.md` before non-trivial changes.

- No filesystem, network, process, environment, thread or clock call anywhere in this crate, tests included (`fcis R4`). Take bytes and values; return values.
- Dependencies are `alacritty_terminal`, `base64` and `parking_lot`, and nothing else (`ports R3`). No `gpui`, no `serde`, no other Brindle crate.
- Parsing, marks, status records and replay live here and only here, used by both sides; a rule the server core tracks differently from the client makes replays drift (`fcis R6`).
- Malformed sequences are skipped, never a panic: these bytes come from any program (`railway R5`).
- The emulator answers no queries; its listener drops every event but title changes (`process R19`).
- Marks are absolute lines (`base + Line`); keep line spaces apart (`types R5`).
- Every change ships with unit tests in the same file, with no sleep (`fcis R8`).

If the code needs IO, a clock or GPUI, it belongs in `src/` or `remote-agent/`, and this crate gets the values as arguments.

Enforced by: `tests/architecture.rs` (`term_core_does_no_io_and_reads_no_clock`, `crates_below_the_app_name_only_their_allowed_dependencies`).
