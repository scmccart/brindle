# Brindle architecture

These are the normative architecture docs for Brindle, a GPU-accelerated terminal emulator whose tabs run in a persistent terminal server. "Normative" means code review enforces each doc's **Rules** section, and reviewers cite rule numbers from these files by the short names below (`fcis R2`, `process R5`).

`docs/code-map.md` describes how the code works today. These docs say what a change must keep true. Behaviour is specified separately, in `openspec/specs/`.

## Dependency rule

```
brindle (src/)  ─────────┬──▶ brindle-ui (ui/)                            ──▶ gpui
                         ├──▶ brindle-term-core (term-core/)              ──▶ alacritty_terminal
brindle-remote-agent ────┼──▶ brindle-remote-protocol (remote-protocol/)  ──▶ serde
(remote-agent/)          ├──▶ brindle-transcript (transcript/)            ──▶ serde
                         │
                         references point only rightward
```

- **`brindle`**, the app: GPUI entities, the server's client, config, the composition root (`main.rs`). It depends on all four shared crates and may add dependencies with care.
- **`brindle-remote-agent`**, the `brindle-remote` helper: the terminal server's roles, the relay, the management commands, and the ACP client its core runs agents with. It depends on `brindle-term-core`, `brindle-remote-protocol` and `brindle-transcript`, never on `gpui` or `brindle-ui`. It alone names `agent-client-protocol-schema`, in `src/acp/` only, so ACP's types never enter Brindle's build.
- **The two binaries never reference each other.** What both need goes in a shared crate.
- **`brindle-ui`**: components and tokens on `gpui` (and the `log` facade). It knows nothing of terminals, config or the `ACTIONS` table.
- **`brindle-term-core`**: the pure terminal state both sides share (parser, marks, status records, replay, emulator). No IO, no clock, no threads.
- **`brindle-remote-protocol`**: frames, messages and versions (pure codecs), plus the small platform helpers both sides must agree on (server directory, endpoint names, access checks, scopes).
- **`brindle-transcript`**: the agent transcript's events and their fold, which Brindle and the server core share; client protocol 4 carries the events as JSON values the protocol crate doesn't interpret. Pure as a crate (no IO, no clock, no threads), with `serde` and `serde_json` as its closed list.
- **The shared crates never reference each other**, and each has a closed list of dependencies (`ports R3`).

## Read by layer

| Touching | Read first |
| --- | --- |
| anything | this page, then [functional-core-imperative-shell.md](functional-core-imperative-shell.md) |
| `term-core/**` | fcis, types |
| `ui/**` | ports, fcis |
| `remote-protocol/**` | process, types, railway |
| `transcript/**` | fcis, types, railway, process |
| `remote-agent/**` | process, ports, railway |
| `remote-agent/src/acp/**` | fcis, railway |
| `src/remote/**`, `src/terminal/**`, `src/instance.rs`, `src/session.rs`, `src/config.rs` | process, railway, fcis |
| the rest of `src/**` (views, overlays, settings) | fcis, types, ports |
| `tests/**`, `remote-agent/tests/**`, any new kind of test, any flake | testing |
| a `cfg(unix)`, `cfg(windows)` or `cfg(target_os)` in production code | ports |

## The docs

| Doc | Cited as | One line |
| --- | --- | --- |
| [functional-core-imperative-shell.md](functional-core-imperative-shell.md) | `fcis` | Decisions are pure functions and GPUI-free models; entities, threads and loops only gather, call and apply. |
| [railway-oriented-programming.md](railway-oriented-programming.md) | `railway` | Expected failures are values, shown where they happened; decoders never panic. |
| [illegal-states-unrepresentable.md](illegal-states-unrepresentable.md) | `types` | Closed enums, stable identities, units kept apart, parsing at the boundary. |
| [ports-and-adapters.md](ports-and-adapters.md) | `ports` | The crate boundaries, and one seam per platform concern. |
| [process-architecture.md](process-architecture.md) | `process` | Holder and core, versioned protocols, readiness loops, upgrades, the UI thread. |
| [testing-architecture.md](testing-architecture.md) | `testing` | Which layer a test belongs in, hermetic runs, and how flakes are fixed. |

## Shared vocabulary

- **Pure code / shell:** pure code computes a value from its arguments and does nothing else. The shell is everything that touches the world: GPUI entities and their `render`, threads, event-loop handlers, `main.rs`. These docs say "pure code", not "core", because of the next entry.
- **Server core:** the terminal server's `serve` role (`remote-agent/src/server/core.rs`). In this repo "the core" always means that process, never the pure code.
- **Holder:** the `hold` role, which owns the PTYs and pseudoconsoles and interprets nothing.
- **Model:** a GPUI-free type that holds a view's state and decisions (`FontsState`, `SessionsModel`, `EditState`).
- **Seam:** the one module, or the one per-platform module set, that owns a platform concern. Brindle's seams are `cfg`-selected modules with the same item names, not traits. A seam is this repo's port, and each platform's module its adapter.
- **Outcome:** an enum a function returns when it can end in more than one way (`Listing`, `LinkState`, `access::Refusal`).
- **Unit:** the thing a fault is confined to: a session, a tab, an endpoint, a connection.

## Enforcement

`tests/architecture.rs` runs with plain `cargo test` on every platform CI builds. It checks the crate boundaries from the manifests and a few source rules by text search; each doc's Enforcement section says which of its rules that covers. CI also runs `cargo clippy --workspace --all-targets -- -D warnings` on Linux and Windows, and the crate roots switch on `clippy::wildcard_enum_match_arm` (all six crates) and `clippy::unwrap_used` and `clippy::expect_used` (`brindle-remote-protocol`, `brindle-term-core` and `brindle-transcript`) for non-test code; `tests/architecture.rs` fails if a root loses one. Everything else is held by review, through the `architecture-review` skill.

## Scope of adoption

These docs were adopted on 2026-11-16 by writing down what the code already does. The rules bind new and changed code. Code that predates them and falls short is brought into line when it is next changed, not in a sweep. The deviations known at adoption are below; none has an issue filed.

## Exceptions in force

| Doc rule | Exception | Approved in | Ends with |
| --- | --- | --- | --- |
| `railway R4` | About 90 existing functions return `Result<_, String>` (57 in `src/`, 36 in the other crates), such as `reattach::attach_target`. | adoption, 2026-11-16 | each is revisited when its error gains a second consumer or a branch on its text |
| `railway R3`, `types R2` | 13 `_ =>` or catch-all arms on Brindle's own enums (`ToClient`, `ToCore`, the settings dialog's `Field`), each under `#[allow(clippy::wildcard_enum_match_arm, reason = "railway R3 exception: …")]`: `remote-agent/src/server/cli.rs` 5, `remote-agent/src/server/relay.rs` 1, `src/remote/local.rs` 2, `src/remote/local/connection.rs` 1, `src/remote/local/upgrade.rs` 1, `src/settings_dialog/profiles.rs` 3. `grep -rn "railway R3 exception" --include=*.rs .` lists them. | adoption, 2026-11-16 | each allow is removed when its `match` is next changed, and the row goes when that search finds none |
| `ports R4` | Platform `cfg`s outside a seam in these production files, and no others: `src/instance.rs`, `src/main.rs`, `src/notify/mod.rs`, `src/remote/local.rs`, `src/remote/mod.rs`, `src/session.rs`, `src/settings.rs`, `src/terminal/mod.rs`, `src/terminal_view.rs`, `src/workspace.rs`, `src/zone_view.rs`, `remote-agent/src/main.rs`, `remote-agent/src/server/cli.rs`, `remote-agent/src/server/core/sessions.rs`, `remote-agent/src/server/daemon.rs`. The architecture test `platform_cfgs_stay_in_their_seams` lists the same files and fails on any other (backlog: audit 2026-11-16, "Exceptions"). | adoption, 2026-11-16; narrowed by `enforce-platform-seams` | the `port-to-macos` change, which moves each behind its seam |
| `process R4` | The instance request (`src/instance.rs`, a later launch handing its request to the running Brindle) is one unnegotiated JSON line with no version: fields are only added, each omitted when unset, unknown fields are ignored, and a `kind` the instance can't open is refused by name, so launches and instances of other versions meet without a greeting (an older instance speaks none a launch could wait for). It answers `ok`, `error: TEXT` or `usage: TEXT`, pinned by transcript tests. | `agents-launch-commands` | a change that needs a non-additive request, which brings a negotiated hand-off |
