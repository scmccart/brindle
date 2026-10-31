# Tasks

## 1. Command state in Marks

- [ ] 1.1 Add `collapsed`, `body_top`, `started`/`ended` and `header` to `Command`, setting the timing from the `C` and `D` marks and capturing the header text at `C`; verify with `cargo test shell_integration` unit tests for timing and a header outliving its rows.
- [ ] 1.2 Add `body_top` to `positions_mut` and move `Marks::snapshot`/`restore` to five positions per command; verify with a unit test that a scrolled body keeps its top line through a reflow.
- [ ] 1.3 Add the finished-block rule (ended, or a later prompt) and an empty-body check; verify with unit tests for a running command, a command with no `D`, and an empty body.

## 2. Projection

- [ ] 2.1 Create `src/terminal/blocks.rs` with `Projection`, row kinds and `ViewAnchor`, walking commands only from the anchor to the bottom of the viewport; verify with `cargo test terminal::blocks` covering plain rows, expanded, collapsed and mixed documents.
- [ ] 2.2 Implement the height cap, tail following, scrolled bodies and tall headers; verify with unit tests for every `Block height`, `Tall headers`, `Following the tail` and `Inner scroll position` scenario.
- [ ] 2.3 Implement `Projection::active` (blocks off, alternate screen, no commands) so no mark is resolved to a grid line on the alternate screen; verify with a unit test that builds an alternate-screen `Term` with main-screen marks and gets the identity result.
- [ ] 2.4 Implement the inline-program rule (cursor on a covered row, or mouse reporting on) and the mapping from a screen row to a grid line and back; verify with unit tests for both triggers and for a row outside the live screen reporting nothing.

## 3. View position

- [ ] 3.1 Add the `blocks` config key to `src/config.rs` and `assets/default-config.toml`, on by default and applied on reload; verify with a config unit test and an update to the README's settings list.
- [ ] 3.2 Make `Terminal::scroll` and `Terminal::input` drive `ViewAnchor` while the projection is active, keeping `display_offset` at 0, and convert the position when entering and leaving the active state; verify with `cargo test` unit tests for line, page, top and bottom moves, and for typing resuming the running block's tail.
- [ ] 3.3 Make prompt jumps treat a block as one stop; verify with unit tests for the `Keyboard scrolling` scenario, and that `shell-prompt-jump` still passes under `scripts/with-xvfb`.
- [ ] 3.4 Clamp the anchor and `body_top` when lines leave the scrollback or it is cleared; verify with unit tests for pruning and `CSI 3 J`.

## 4. Paint

- [ ] 4.1 Add the projection to `GridLayout` and make `TerminalElement::prepaint` read rows through it when active, leaving the `display_iter` path for the identity case; verify that `cargo test` passes and the existing e2e suite under `scripts/with-xvfb` is unchanged with `blocks = false` and with no marks.
- [ ] 4.2 Move the failed-command bar behind the alternate-screen bypass and fix the existing overpaint; add an e2e case (`shell-failed-mark-alt-screen`) that runs `false`, opens a full-screen program and samples the bar's pixels.
- [ ] 4.3 Paint the header tint, the rail (replacing the bar while blocks are on), the chevron and the scroll thumb, with colors derived from the theme; add e2e pixel cases for the rail colors, for colored output matching `blocks = false`, and for a control-mode pane.
- [ ] 4.4 Paint the collapsed summary, leaving it out when it would touch text; verify with unit tests for the format and the fit check, and an e2e case on the dumped summary.
- [ ] 4.5 Add `--- block` lines and projected rows to `--dump-screen-after`, and document the format in CLAUDE.md; verify with an e2e case parsing them.

## 5. Input

- [ ] 5.1 Map `cell_at` consumers (selection, double- and triple-click, links, hover, mouse reports) through the projection; add an e2e case that hovers and clicks a link below a collapsed block with `--hover`/`--click`.
- [ ] 5.2 Add the `route` wheel function and `WheelLatch` to `TerminalView` with the 300 ms constant; verify with unit tests over injected instants for all four `Wheel latching` scenarios, and that mouse reporting and alternate scroll keep priority.
- [ ] 5.3 Add chevron hit-testing ahead of selection; verify with an e2e case that a `--click` on the chevron collapses the block and starts no selection.
- [ ] 5.4 Add `toggle_block`, `collapse_all_blocks` and `expand_all_blocks` to `ACTIONS`, with palette labels; verify with `--list-actions`, the e2e palette case, and an e2e case that a running block won't collapse.
- [ ] 5.5 Scroll the body, then the view, while drag-selecting past a body window's edge, and include hidden lines when copying across a collapsed block; add e2e cases for both to `tests/e2e/cases/selection.rs`.

## 6. Search

- [ ] 6.1 Reveal matches through the projection: expand a collapsed block, scroll the body, and move the view only if the block is off screen, updating `search.rs` and `find_bar.rs`; add e2e cases to `tests/e2e/cases/search.rs` for both new scenarios and keep `Match in history` passing.

## 7. Integration

- [ ] 7.1 Add a `tests/e2e/cases/blocks.rs` suite for full-screen programs (collapse state surviving `vim`, nothing covered), inline programs (an OSC 133 fixture moving the cursor to a covered row), tmux control-mode panes (independent panes, reattach with plain rows), and a Windows case in `tests/e2e/cases/win.rs`; verify under `scripts/with-xvfb` and through the Windows CI job.
- [ ] 7.2 Run `tests/perf` with blocks on and off against a saved baseline; verify `latency-shell` and `throughput-brindle` show no regression with no marks and record any change with marks in the PR.
- [ ] 7.3 Document blocks in README.md (behaviour, actions, `blocks` setting) and add the architecture note to CLAUDE.md and the wheel-latching check to its manual checks; verify the listed actions match `--list-actions`.

## Workflow follow-up

- Hand the manual wheel-latching check to the user, including tuning the 300 ms timeout and the header tint.
- Archive the change after review, syncing its delta specs.
