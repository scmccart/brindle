# Railway-oriented programming

In Brindle an expected failure is a value in the return type, and it ends up as visible state on the thing that failed: a tab's message, an endpoint's offline row, a line in the terminal. `brindle-remote-protocol`, `brindle-term-core`, `brindle-ui` and `brindle-remote-agent` return their own error types or `io::Result`; `anyhow` is for the app alone, where errors are only reported. Code review enforces the **Rules** section; reviewers cite rules by number (`railway R5`).

## Why

Brindle reads bytes from other processes, other machines and other versions of itself, so malformed input is ordinary, and a panic in a long-lived role costs every session it holds. Its users also can't see a log: a failure that isn't drawn where it happened looks like a hang. Error text matters as well, because one message read as a different failure ("could not start" for a failed attach) sends the user after the wrong cause.

## The idea

- Every way a call can end is in its signature: `Result<T, E>` for one success shape, an outcome enum for several.
- Bytes from outside never cause a panic. A decoder returns `Err`, a scanner skips.
- The failure travels as a value to the unit it belongs to, which shows it and retries on the next attempt.

```rust
// remote-protocol/src/version.rs: one error type for one workflow, with data to branch on.
pub fn negotiate(offered: VersionRange, supported: VersionRange) -> Result<u32, NegotiationError>

// src/picker.rs: several shapes, none of them a failure of the palette.
pub enum Listing { Sessions(Vec<SessionInfo>), Connecting, Offline(String), Pending }
```

## Idioms

**`io::Result` with a meaningful kind** in the protocol crate and the helper: an oversize control line is `InvalidData`; a squatted pipe name is `AddrInUse`. Callers branch on the kind, never on the text.

**A workflow's own enum** where callers need the cases: `version::NegotiationError`, `access::Refusal`.

**`anyhow` at the app's edge**, with context that names the action:

```rust
// The terminal's sink keeps what it was doing (`start "bash"`, `attach session 3`),
// so the message names the action that failed, then its cause.
```

**The cause, in a fixed order.** `relay_link_ended` reports the watchdog's own reason first, then the relay process's account (`ssh exited with 255: <cause>`), then the generic one. `client::split_cause` puts a failed launch's last stderr line first.

**"No" as an answer.** `brindle-remote status --json` prints `{"running":false}` and exits 0 with no server. The loser of a start race exits 0. An endpoint that hasn't answered is `Listing::Pending`, a row, not an error.

**Panics are bugs, confined per unit** (`process R3`):

```rust
// remote-agent/src/server/core/screen.rs: a panic in one session's feed marks that
// emulator broken; the session keeps forwarding output and the server keeps serving.
```

## How it applies here

| Where | Error shape | Where the failure is shown |
| --- | --- | --- |
| `remote-protocol` decoders | `io::Result` (`InvalidData`), `NegotiationError`, `access::Refusal` | the peer closes the connection or sends `ERROR`; `fuzz.rs` checks no input panics |
| `term-core` parser and scanner | none: malformed sequences are skipped | nothing; the stream continues |
| `remote-agent` roles | `io::Result`, protocol `ERROR` frames, exit codes (0 for a lost start race, 97 for "deploy the helper") | the role's log; the client's tab |
| `src/remote/` (connecting, launching, upgrading) | `anyhow::Result`, `Refused`, `LinkState`, `HostState` | `Terminal::message`, the dim "reconnecting…" line, the red "session lost" line, a disabled palette row |
| config and settings | findings collected per field (`invalid=[…]`), defaults with a shown reason | the settings dialog; the config warning |

## Rules

- **R1.** An expected failure MUST be in the return type: `Result<T, E>`, or an outcome enum when success has several shapes. Not a panic, a sentinel value, or a log line with a default returned in its place.
- **R2.** `brindle-remote-protocol`, `brindle-term-core`, `brindle-ui` and `brindle-remote-agent` MUST return their own error types or `io::Result` with a meaningful `ErrorKind`. `anyhow` MUST stay in `brindle`, where errors are reported and nothing branches on them.
- **R3.** A `match` on one of Brindle's own enums MUST be exhaustive, with no `_ =>` arm, so a new variant fails to compile everywhere it matters. Wildcards are for foreign types, bytes, strings and integers. Don't use `#[non_exhaustive]`: no crate here is published.
- **R4.** Error cases SHOULD carry data that code can branch on. `Result<_, String>` is acceptable only where the single consumer shows the text (a tab's message, a CLI's stderr, a dump line). Once code needs to tell two failures apart, give them an enum; never compare error text.
- **R5.** Code that reads bytes from another process, a file, the user or the wire MUST NOT panic on any input: it returns `Err`, or skips and continues, and it bounds what it accepts (frame lengths, control lines, layout blobs). Every decoder has a fuzz test (`process R6`).
- **R6.** A failure of a session, tab or endpoint MUST become that unit's visible state (`Terminal::message`, `LinkState::Offline`, the lost line, a disabled row). Not a modal, and not a log line alone. It MUST NOT be cached: the next attempt retries.
- **R7.** When the question is "is there one?" or "is it valid?", "no" MUST be an ordinary answer, not an error: a listing with no server, a start race lost, an endpoint still pending, a validation finding.
- **R8.** Error text MUST name the action that failed and then its cause, most specific first. A message that reads as a different failure is a defect, as is one that drops the cause the system already had (a process's stderr, an exit status, a path).
- **R9.** `unwrap` and `expect` in non-test code MUST be limited to cases where failure is a bug in Brindle, and SHOULD be `expect("why this can't fail")`. Never on data from outside the process.
- **R10.** Findings a person will fix in one sitting (settings fields, config keys) SHOULD be collected and reported together. A bad config falls back to defaults and shows why; it never refuses to start (`process R15`).
- **R11.** Each workflow SHOULD have its own error type, widened with `map_err` or `From` where it joins a larger one. No crate-wide error enum, and no `Other(String)` case added for symmetry.
- **R12.** A seam module MUST turn the platform's failure into the caller's type at the seam: an OS error code becomes an `io::ErrorKind` or an outcome case there (`ports R9`), so no caller compares raw `GetLastError` or `errno` values.

## Anti-patterns

- **`let Ok(x) = decode(bytes) else { panic!() }` on wire data.** Instead: return the error; the connection is closed, not the process (R5).
- **`log::warn!` and carry on with a default, for a failure the user caused or must fix.** Instead: make it the unit's state (R6).
- **"could not start" for every failure in a tab.** Instead: name what it was doing (R8).
- **`if err.to_string().contains("refused")`.** Instead: an enum case (R4).
- **A `_ =>` arm added to a match on `Host` or `Closing` to make a new variant compile.** Instead: handle the variant in each match (R3).
- **A cached "the helper is missing" that survives the user installing it.** Instead: recompute per attempt (R6).
- **`anyhow` added to a shared crate for convenience.** Instead: `io::Result` or a small enum (R2).

## Enforcement

- `tests/architecture.rs` checks R2's dependency half: `crates_below_the_app_name_only_their_allowed_dependencies` fails if a shared crate or the helper gains `anyhow`.
- `remote-protocol/src/fuzz.rs` holds R5 for the decoders.
- R1, R3, R4 and R6–R12 rely on review. There is no clippy step in CI, so `unwrap_used` and wildcard arms are not linted.

## PR checklist

- [ ] Every expected failure is in a return type; nothing is logged and replaced by a default.
- [ ] No `anyhow` outside `brindle`.
- [ ] No `_ =>` arm on one of Brindle's own enums.
- [ ] Input from outside the process can't panic, and is bounded.
- [ ] The failure is drawn on the unit it belongs to and isn't cached.
- [ ] The message names the action, then the most specific cause.
- [ ] No `unwrap` or `expect` on outside data.

## References

- The architecture plugin's `railway-oriented-programming` skill.
- `CLAUDE.md`, "Brindle's client" (the link's reasons for ending) and "Terminal server" (exit codes, `ERROR` frames).
- Exceptions in force: [README.md](README.md#exceptions-in-force).
