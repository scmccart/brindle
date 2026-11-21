# Illegal states unrepresentable

Brindle's types are meant to make a wrong state fail to compile: exclusive states are one enum, an identity is the thing that stays stable, quantities in different spaces have different types or one named conversion, and raw input is parsed into a typed value once, at the boundary. This governs every crate. Code review enforces the **Rules** section; reviewers cite rules by number (`types R3`).

## Why

Brindle's bugs of this kind have a pattern. Session ids restart at 1 in a fresh server core, so an id alone named the wrong session after a restart. A font kept by list index pointed at another family once the list was regrouped. A replay fed at the wrong grid size misplaced the cursor. Code that assumed a program ran on this machine looked up a pid that belonged to a remote host. Each was a value that compiled but meant something else. The fix each time was a type or a key that could not be confused.

## The idea

- If two states can't hold together, they are variants of one enum, and every `match` names every variant.
- An identity is what survives a refresh, a reconnect and a restart: a key, a name, an id plus its endpoint. Never a position.
- Text and bytes are parsed once into a typed value. Code past the boundary never looks at the raw form again.

```rust
// src/remote/host.rs: where a session runs. Everything that assumes "here" matches on Local.
pub enum Host { Local, Remote(RemoteTarget) }

// src/terminal/mod.rs: what closing does. Not `keep_on_quit: bool` beside `ends_on_close: bool`.
pub enum Closing { Keeps, Ends }
```

## Idioms

**A stable key for a row.** The sessions palette keys rows by `RowKey` (a session's endpoint, id and key; an endpoint; or the note), so a refreshed listing keeps the highlighted row even when its index moves.

**Id plus key across servers.** A session is `{ id, key }`: the id is the core's counter, the key a random 128-bit value that tells sessions apart across cores. The state file records both, and a restored zone takes its session only when both match.

**Newtypes for in-process ids.** `ZoneId(u64)` in `zone_view.rs`, `LayerId` in `brindle-ui`.

**Normalised geometry as its own type.** `UnitRect` holds a zone's rectangle in `[0, 1] x [0, 1]`; pixel rectangles are gpui's `Bounds<Pixels>`. Cell sizes come only from `cell_metrics()` and `fit_grid()`.

**Old forms stay parseable at the boundary, and stop there.**

```rust
// src/session.rs: records an older version wrote map to a variant restore filters out.
#[serde(tag = "kind")]
pub enum TabState { Zones(ZonesState), #[serde(other)] Legacy }
```

A removed config key stays as a field that is parsed, warned about and ignored (`process R15`).

## How it applies here

| Concept | Type | What it rules out |
| --- | --- | --- |
| where a session runs | `Host` | a remote session treated as local (pid lookups, directories checked on disk, `parent_console_to`) |
| what closing a terminal does | `Closing` | a tab that both keeps and ends its program |
| what a terminal is backed by | `Backend` (`Server`, `Remote`) | input sent to a read-only message tab |
| an endpoint's link | `LinkState`, `HostState` | "connected" with a stale reason, "offline" with live sessions |
| an endpoint's listing | `Listing` | an empty list mistaken for "no answer yet" |
| a saved tab | `TabState` | an old record breaking the rest of the file |
| a program's status | `status::State` | `working` and `done` at once |
| a session's identity | id + key (+ endpoint) | the wrong session after a core restart |

## Rules

- **R1.** States that exclude each other MUST be one enum, with data on the variants that have it. Not two `bool`s, and not an `Option` whose meaning depends on another field.
- **R2.** A `match` on one of Brindle's own enums MUST name every variant, with no `_ =>` arm (`railway R3`). Adding a variant should produce a compile error at each place that must decide.
- **R3.** An identity MUST be the value that stays stable, never a position or a number that is reused. Sessions are named by endpoint, id and key; font choices by family name; palette rows by `RowKey`; zones by `ZoneId`. An index is valid only within the render or the call that computed it.
- **R4.** New identifiers that cross a module boundary SHOULD be newtypes (`ZoneId`, `LayerId`). Ids inside protocol structs stay the integers they are: their encodings are frozen (`process R5`).
- **R5.** Values in different spaces MUST NOT meet without a named conversion: cells and pixels (only through `cell_metrics()` and `fit_grid()`), normalised `UnitRect` and window pixels, logical and physical pixels, UTF-16 offsets and byte offsets (`EditState`), absolute lines and viewport lines (marks are `base + Line`). One function owns each conversion.
- **R6.** Raw input MUST be parsed once into a typed value at the boundary: config text into `Config`, a frame into its message struct, an OSC into a `Found` value, a state file into `TabState`. Code past the boundary takes the typed value and never re-checks the raw one.
- **R7.** Absence that means something MUST be a variant, or an `Option` handled at the boundary. No sentinels: no `-1`, no `0` id, no empty string for "none". The word `none` in a dump is display only.
- **R8.** Every code path that assumes the program runs on this machine MUST be inside a `Host::Local` arm. A tab records its own `Host`, and its splits stay on it whatever the profile targets later.
- **R9.** Persisted and wire representations MUST live at the serialization boundary: the `session.toml` types, `layout::Layout`'s JSON, the protocol structs. An old form maps to a skip variant or an ignored field there and doesn't enter the in-memory model.
- **R10.** A cached or stored derived value MUST name what refreshes it, in the code beside it. Tab titles are refreshed on `TitleChanged` and a 1 s poll; `Tab::status` is the maximum over the tab's terminals, recomputed from them when one reports a status. Prefer computing to storing when it is cheap.
- **R11.** A type with an invariant SHOULD have private fields and constructors that enforce it, and SHOULD NOT derive `Default` when its zero value is invalid.
- **R12.** A wrapper, variant or abstraction MUST exist for a meaning the code has today. Don't keep one "for a future kind": add the variant when the kind exists, and let R2 show every place that must handle it.

## Anti-patterns

- **`is_remote: bool` next to `target: Option<RemoteTarget>`.** Instead: `Host` (R1).
- **Storing the highlighted row's index across a refresh.** Instead: its `RowKey`, resolved to an index after the rows are rebuilt (R3).
- **Looking a session up by id alone after a reconnect.** Instead: id and key, on its endpoint (R3).
- **Multiplying a column by a cell width computed locally.** Instead: `cell_metrics()` (R5).
- **Reading `/proc/<pid>` for a terminal without matching on its `Host`.** Instead: inside `Host::Local`; a remote session's info comes from `PROCESS` (R8).
- **A `kind: String` field compared against literals in several readers.** Instead: an enum parsed once (R6).
- **Keeping a `Backend` variant nobody constructs.** Instead: delete it; add it back when it exists (R12).

## Enforcement

- The compiler holds R2 wherever the wildcard is absent, and R1 and R8 once the enum exists. In non-test code `clippy::wildcard_enum_match_arm` (CI, `-D warnings`) also reports every wildcard on an enum; one on a Brindle enum that stays carries `railway R3 exception` in its `reason` (`docs/architecture/README.md`).
- `openspec/specs/` scenarios and the e2e suite hold several instances (restore by id and key, `RowKey` across a kill).
- No rule here has a check in `tests/architecture.rs`. All of them rely on review.

## PR checklist

- [ ] No new `bool` or `Option` that only makes sense together with another field.
- [ ] No `_ =>` arm on one of Brindle's own enums.
- [ ] Nothing kept across a refresh, reconnect or restart is an index or a bare session id.
- [ ] No cell, pixel, offset or line arithmetic outside the function that owns the conversion.
- [ ] Raw text and bytes are parsed once; no sentinel values.
- [ ] Local-only assumptions sit inside `Host::Local`.
- [ ] No variant or wrapper without a present use.

## References

- The architecture plugin's `illegal-states-unrepresentable` skill.
- `docs/code-map.md`: `Host`, "Reattach", "Sessions palette state", "Grid geometry".
- Exceptions in force: [README.md](README.md#exceptions-in-force).
