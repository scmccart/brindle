# Proposal

## Why

GUI behaviour is verified by hand-run end-to-end checks. Each check starts a throwaway tmux server, runs Brindle with its debug hooks (`--send`, `--action`, `--dump-screen-after`), and compares the result with tmux: screen text, colors in a window capture, or replies to color queries. The checks are described in prose in each archived change's `tasks.md`. The scripts that ran them, including a small x11rb window-capture tool, lived in a session's temporary directory and are gone, so nothing can be re-run when tmux, GPUI or Brindle's tmux code changes. The recent tmux work (pane geometry, titles, styles, color reports) also showed that some of tmux's behaviour isn't documented and may change between versions, for example that `rotate-window` sends no `%layout-change`.

## What Changes

- Add an opt-in end-to-end suite under `tests/e2e/`: a Cargo integration test target with its own runner (`harness = false`). It does nothing unless `BRINDLE_E2E=1` is set, so `cargo test` stays free of windows and displays. Run it with `BRINDLE_E2E=1 cargo test --test e2e -- [case ...]`.
- The suite owns its helpers:
  - throwaway tmux servers, killed afterwards;
  - launching Brindle with a test config and a timeline of external tmux commands;
  - parsing the debug dump;
  - an OSC 10/11 query helper;
  - the X11 window-capture tool (x11rb `GetImage`), as part of the suite rather than a separate example.
- Port the checks from this round of tmux work into named cases:
  - **Text:** pane geometry in each border-status configuration, with `less`, rotate, zoom and background windows; the dump must match `capture-pane`.
  - **Color queries:** color reports on attach, in a new pane, after a reload and in a styled pane.
  - **Pixels:** title lines and their styles; pane tints, the active-pane style and border colors.
  - **tmux behaviour:** what Brindle relies on, checked over a raw `tmux -C` connection without a display: subscriptions, `rotate-window` sending no layout change, and a control client's color report taking precedence.
- **BREAKING (debug output only):** `--dump-screen-after` also prints the active terminal's grid origin and cell size in window pixels, plus the window's scale factor, so pixel checks can find cells exactly.
- Document the suite in `CLAUDE.md`'s "Verifying GUI behaviour". Running it opens windows, so the existing rule to ask before GUI runs applies.

## Capabilities

### New Capabilities

None. The suite is development tooling, not product behaviour.

### Modified Capabilities

- `command-line`: "Debug automation hooks". The dump also prints the active terminal's grid geometry in window pixels.

## Impact

- **Code:**
  - `Cargo.toml`: a `[[test]]` target and an `x11rb = "=0.13.2"` dev-dependency. That version is already in `Cargo.lock` through GPUI, so the lockfile only gains the edge, with no version changes.
  - `tests/e2e/`: the runner, helpers, capture tool and cases.
  - `src/workspace.rs` / `src/main.rs`: the extra dump line.
- **Tests:** the suite itself; the dump change gets a unit-tested formatter.
- **Out of scope:**
  - Porting the end-to-end checks of earlier archived changes (command palette, restore-active-pane, dividers). The harness makes that easy as a follow-up.
  - Running the suite in CI, which would need a virtual display and software Vulkan.
