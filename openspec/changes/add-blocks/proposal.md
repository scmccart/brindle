# Proposal

## Why

Shell output runs together in one long stream, so a noisy command buries the commands around it and its own command line scrolls away while it runs. Brindle already tracks where each command and its output start and end (OSC 133), but only uses that for jumps, copying output and failure bars.

## What Changes

- Group each command and its output into a visual block: a tinted header with a status rail, followed by the output.
- Let finished blocks collapse to a header row with a summary (line count, duration, exit status).
- Give a tall block its own scroll position. A block grows until it would be cut off by a viewport edge, then scrolls internally with its command still showing, and a running block follows its output.
- Route the wheel with latching: a wheel run stays on the scroller it started on, so a huge block can't trap outer scrolling.
- Turn blocks off entirely on the alternate screen, and let the header step aside for inline programs that draw on the rows it covers.
- Add a `blocks` config key, on by default. It only affects shells with integration loaded, and `false` restores today's behaviour exactly.
- Replace the failed-command bar with the block's rail while blocks are on, and stop drawing failed-command marks over the alternate screen.
- Make search reveal a match inside a collapsed block or a block's scrolled-away output.

Out of scope: collapsing running blocks, keeping collapse state across restarts or tmux reattach, per-block actions such as sharing or rerunning, and changes to the shell scripts.

## Capabilities

### New Capabilities
- `blocks`: grouping commands and their output into collapsible, internally scrolling blocks, and how they behave with the wheel, keyboard, selection and full-screen programs.

### Modified Capabilities
- `shell-integration`: the failed-command mark becomes part of the block's rail while blocks are on, and is never drawn over the alternate screen.
- `scrollback-search`: bringing a match into view expands a collapsed block and moves a block's own scroll position.
- `terminal-emulation`: wheel scrolling routes through blocks when they are in play.
- `configuration`: adds the `blocks` behaviour setting.

## Impact

- **Rendering and input:** `src/terminal_element.rs` stops reading the viewport straight from alacritty when blocks are active, and `src/terminal_view.rs` (mouse, wheel, links, selection) maps through the same row table.
- **Terminal model:** `src/terminal/shell_integration.rs` gains per-command view state and command timing, `src/terminal/mod.rs` and `src/terminal/search.rs` gain a Brindle-owned view position, and `src/find_bar.rs` reveals matches through it.
- **Other code:** `src/actions.rs` (new actions), `src/config.rs`, `assets/default-config.toml`, the `--dump-screen-after` output, `tests/e2e/`, README.md and CLAUDE.md.
- **Local tabs and control-mode panes** both gain blocks. Classic tmux, shells without integration and alternate-screen programs are unchanged.
- **Dependencies:** none. The PTY size and grid are unchanged, so programs can't observe blocks.
