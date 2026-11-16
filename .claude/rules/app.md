---
paths:
  - "src/**"
---
# brindle: the app's shell, its models and its seams

You are editing the app crate: GPUI entities, the server's client, config, and the pure helpers and models they call. Read `docs/architecture/functional-core-imperative-shell.md` first; then `process-architecture.md` for `src/remote/`, `src/terminal/`, `instance.rs`, `session.rs` and `config.rs`, and `illegal-states-unrepresentable.md` and `ports-and-adapters.md` for views, overlays and settings.

- A decision goes in a pure function or a GPUI-free model (`SessionsModel`, `FontsState`, `layout::group`, `pace::step`), with unit tests in the same file. The entity or thread only gathers, calls and applies (`fcis R1`, `R8`).
- Pure code takes `now` and environment values as arguments and reads nothing ambient (`fcis R2`, `R3`).
- Nothing on the UI thread waits, and `render` and `prepaint` do no IO, heavy allocation or contended locking (`process R13`, `R14`). Lock order: parser → term → marks.
- No `cfg(windows)`, `cfg(unix)` or `cfg(target_os)` outside the module that owns that platform concern (`terminal/process.rs`, `primary.rs`, `notify/`, `remote/local/{unix,windows}.rs`, `win.rs`, `desktop.rs`) (`ports R4`).
- Exclusive states are one enum, matched without a `_ =>` arm; anything that assumes the program runs here sits inside `Host::Local` (`types R1`, `R2`, `R8`).
- Keep identities stable across a refresh or reconnect: endpoint + id + key for sessions, `RowKey` for rows, family names for fonts. Never an index (`types R3`).
- A failure becomes the tab's or endpoint's visible state, names the action and its cause, and isn't cached (`railway R6`, `R8`).
- What a quit must do is done in the quit path, never in a `Drop` (`process R16`).

If the code is terminal state both sides need, it belongs in `term-core/`; a wire type, in `remote-protocol/`; a reusable component, in `ui/`. The app never depends on `brindle-remote-agent`.

Enforced by: `tests/architecture.rs` for the crate references; the rest by review (`architecture-review` skill) and the e2e suite.
