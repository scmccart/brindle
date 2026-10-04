# Tasks

## 1. Style runs

- [x] 1.1 Add `src/tmux/style.rs` with `TmuxColor`, `StyledRun` and `parse_format` (design.md decision 2). Unit-test:
  - Claude Code's format: `#[fg=blue,bold] @researcher #[default]`;
  - tmux's default format, active (`#[reverse]0#[default] "t"`) and inactive;
  - `##`;
  - unknown directives (`align=right`, `range=pane|%1`);
  - colors `brightred`, `colour208` and `#1a1b26`;
  - an unterminated `#[`.

  Verify with `cargo test tmux::style`.

## 2. Title state

- [x] 2.1 Send the `brindle-title` and `brindle-border-status` subscriptions with the geometry one, and store their values: parsed title runs per pane, and border status per window (design.md decision 1). Drop entries for closed panes and windows in `sync_panes` and on window close. Verify with `cargo test` and a clean `cargo build`.

## 3. Painting

- [x] 3.1 In `TmuxWindowView`, compute each visible pane's title row and column span from its pane rectangle and the window's border status, skipping rows outside the window (design.md decision 3). Put the computation in a pure helper and unit-test the top, bottom, off, zoomed and narrow-pane cases. Verify with `cargo test tmux_view`, or wherever the helper lives.
- [x] 3.2 Paint titles in the canvas after the dividers. Draw the divider line across reserved rows of edge panes, then a background quad and the shaped text for each run. Use the base colors from design.md decision 3, the theme palette for indexed colors, and bold through the font weight. Cache shaped runs (decision 4). Verify with a clean `cargo build`.
- [x] 3.3 Update `CLAUDE.md`'s tmux notes: title lines come from the `#{T:pane-border-format}` subscription and are drawn in the reserved rows. Verify the note matches the code.

## 4. End-to-end verification

Use a throwaway server (`-L brindle-e2e -f /dev/null`) and kill it afterwards. Titles are pixels, so check them with a window capture under Xwayland (`env -u WAYLAND_DISPLAY DISPLAY=:0` and an x11rb `GetImage` helper), not with `--dump-screen-after` alone.

- [x] 4.1 Split a window, set `pane-border-status top`, title one pane `@researcher` with Claude Code's format, attach, and capture the window. Confirm the title appears above that pane in bold blue, and the default-format title above the other.
- [x] 4.2 While attached, rename the pane with `select-pane -T renamed`, wait 2 s, capture again, and confirm the new title.
- [x] 4.3 Switch to `pane-border-status bottom`, then `off`, capture after each, and confirm the titles move below the panes and then disappear.
- [x] 4.4 Run `cargo test`, `cargo build --release` and `openspec validate show-tmux-pane-titles --strict`.
