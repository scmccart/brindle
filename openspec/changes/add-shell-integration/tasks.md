# Tasks

## 1. Mark parsing and anchoring

- [x] 1.1 Create `src/terminal/shell_integration.rs` with the `Scanner` state machine (ground, escape, CSI, OSC, strings; CAN/SUB; persistent across calls) reporting OSC 133 marks, alternate-screen switches, `CSI 3 J` and `ESC c` split points. Verify with unit tests run by `cargo test shell_integration`:
  - each kind, with BEL and `ESC \` terminators;
  - every split position of a mark sequence across two reads gives the same result;
  - OSC 133 inside a DCS string, non-133 OSCs and over-long OSCs are ignored;
  - `D` exit parsing (missing, non-numeric, negative).
- [x] 1.2 Add `Marks` (commands grouped from marks, absolute positions, `base`), row-identity scroll tracking around every processor call, pruning, `clear`, and the logical-line snapshot/restore used for resize and the alternate screen. Verify with unit tests against a real `Term` with a small grid: prompts survive scrolling, history overflow (scrollback 5 and 0), `CSI 2 J` + `CSI 3 J`, `ESC c`, narrower and wider resizes with wrapped lines, and a resize while `?1049h` is active.
- [x] 1.3 Add `Parser` (vte processor + scanner) with `advance(term, marks, bytes)` and tracked `stop_sync`, flushing a synchronized update before marks and screen switches. Verify with unit tests: a mark inside `CSI ? 2026 h` lands at the right position; grouping rules (redraw `A`, `D` without `C` ignored).

## 2. Backends

- [x] 2.1 Add `polling = "3.11"` to `Cargo.toml` without running `cargo update`. Verify that `git diff Cargo.lock` only adds the dependency edge to brindle's entry.
- [x] 2.2 Add `MarkedPty`/`ParsingReader` and use them in `Terminal::spawn_pty`. Route `Terminal::feed` through the same `Parser`, and move sync-timeout flushing into one path for both backends (scheduled on `Wakeup`). Wrap `Terminal::resize` and `clear_scrollback` with mark remapping and pruning. Verify that `cargo build` succeeds and that existing unit tests pass. Check by hand in a launch that typing in a local tab works and that `exit` closes the tab (drain on exit still works).
- [x] 2.3 Add `Terminal` queries: `jump_to_prompt(previous)`, `select_last_output`, `last_output_text`, and failed-row ranges for a viewport. Verify with unit tests on the pure parts (target selection and output range from a `Marks` and a `Term`).

## 3. Actions, rendering and config

- [ ] 3.1 Add `JumpToPreviousPrompt`, `JumpToNextPrompt`, `SelectLastOutput` and `CopyLastOutput` to `actions.rs` with labels, config names and default bindings ctrl-shift-up/down (`Terminal` context), replacing the `ScrollLine*` defaults. Handle them in `TerminalView`, with line-scroll fallback. Verify with `cargo test actions`, and check `--list-actions`.
- [ ] 3.2 Add `mark_failed_commands` (default `true`) to `Config` and `assets/default-config.toml`, and draw the gutter bar in `TerminalElement::prepaint`/`paint`. Verify that the `default_config_file_parses` test still passes and that `cargo build` succeeds.
- [ ] 3.3 Add the `--- selection` dump line and parse it in the e2e `Dump`. Verify with the harness self-test (`BRINDLE_E2E=1 cargo test --test e2e -- --self-test`).

## 4. Shell scripts and CLI

- [ ] 4.1 Write `shell-integration/brindle.bash`, `brindle.zsh` and `brindle.fish`, and add `--shell-integration <shell>` to `main.rs`, embedding them with `include_str!`. Verify by running `bash -n` on the bash script. Run an interactive bash in a pty (`script`) with the script loaded, and check that its output contains `133;A`, `133;B`, `133;C` and `133;D;1` after `false`, and that `$?` and an existing `PROMPT_COMMAND` still work. Check `brindle --shell-integration tcsh` exits 2.

## 5. Docs and end-to-end checks

- [ ] 5.1 Document shell integration in README.md (setup per shell, keys, palette commands, `mark_failed_commands`, tmux behaviour, how to restore line scrolling). Verify by reading it back against the specs.
- [ ] 5.2 Extend the e2e `Run` so it can open a local `Shell` tab (no tmux server needed). Add these cases:
  - `shell-prompt-jump`: bash with the script; `seq` output scrolls the prompts into history; two `jump_to_previous_prompt` steps; the dump's first line is the earlier prompt;
  - `shell-failed-mark`: `false`, then a capture; red gutter pixels beside the `false` prompt and none beside the `true` one;
  - `shell-select-output`: `printf 'one\ntwo\n'`, then `select_last_output`; the dump's selection line is `one\ntwo`;
  - `tmux-osc133-passthrough` (headless): `%output` from a control client contains OSC 133;
  - `tmux-prompt-jump`: the bash script in a control-mode pane, jump works.

  Verify by running them under `flock /tmp/brindle-gui.lock`.
- [ ] 5.3 Integration check: `cargo build`, `cargo test`, `cargo build --release`, and the e2e cases above, plus `dump-grid-geometry` and `geometry-plain` as regression checks for the PTY and feed paths.
