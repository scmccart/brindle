# Proposal

## Why

Brindle doesn't know where a shell prompt starts, where a command's output begins or ends, or whether the command failed. Finding the start of a long build log means scrolling by hand, and copying one command's output means a careful mouse drag. Most current terminals (iTerm2, WezTerm, kitty, Ghostty, foot, VS Code) read the OSC 133 "semantic prompt" marks that shells can emit, and offer prompt navigation and output selection on top of them. Brindle should do the same, and it should also work in tmux control-mode panes, where tmux passes those marks through unchanged.

## What Changes

- Recognise OSC 133 `A` (prompt start), `B` (command start), `C` (output start) and `D[;exit]` (command finished) in a terminal's output, whether it comes from a local PTY or from a tmux control-mode pane. Each mark is anchored to the grid position where it landed. It moves with the text as lines scroll into history, is dropped when its line leaves the scrollback or the scrollback is cleared, and follows the reflow when the window is resized.
- **Jump to Previous Prompt** / **Jump to Next Prompt** scroll the viewport so the target prompt line is at the top. They are bound by default to ctrl-shift-up / ctrl-shift-down. Those keys used to scroll one line, and they still do in a terminal that has no prompt marks. **Behaviour change:** in a shell with integration loaded, ctrl-shift-up/down jump between prompts instead of scrolling a line. `scroll_line_up`/`scroll_line_down` stay bindable, and the README shows how to get the old keys back.
- **Select Last Command Output** and **Copy Last Command Output**: palette commands, with no default key but bindable. Select highlights the output of the most recent command that has an output mark. Copy puts that output on the clipboard and leaves the selection alone.
- A thin bar in the theme's red is drawn in the left gutter beside prompts whose command exited non-zero. A new `mark_failed_commands` setting (default `true`) turns it off.
- Ready-to-source integration scripts for bash, zsh and fish in `shell-integration/`, also embedded in the binary. `brindle --shell-integration <bash|zsh|fish>` prints one, so `eval "$(brindle --shell-integration bash)"` works without a checkout. Brindle does not inject them into shells automatically: you opt in by adding one line to your rc file.
- The `--dump-screen-after` output gains a `--- selection` line, so end-to-end cases can check output selection.

## Capabilities

### New Capabilities

- `shell-integration`: OSC 133 semantic prompt marks. Covers which sequences are recognised, how marks are anchored through scrolling, clearing, resizing and the alternate screen, prompt navigation, last-output selection and copy, failed-command marks, the shipped shell scripts, and behaviour in tmux control-mode panes and classic tmux.

### Modified Capabilities

- `configuration`: "Behaviour settings" adds `mark_failed_commands`.
- `command-line`: "Informational options" adds `--shell-integration <SHELL>`. "Debug automation hooks" adds the dump's selection line.
- `terminal-emulation`: "Scrollback" says that ctrl-shift-up/down scroll by line only when the terminal has no prompt marks.

## Impact

- **Code:**
  - a new `src/terminal/shell_integration.rs` (OSC 133 scanner, mark store, anchoring, queries);
  - `src/terminal/mod.rs`: the PTY backend parses output through a wrapper around alacritty's `Pty`, so marks are recorded at the right moment; `Terminal::feed` (tmux panes) uses the same parser; jump, select and copy methods;
  - `terminal_view.rs`: the new actions; `terminal_element.rs`: the gutter bar;
  - `actions.rs`, `config.rs`, `main.rs` (`--shell-integration`), `workspace.rs` (dump);
  - `shell-integration/brindle.{bash,zsh,fish}`.
- **Dependencies:** `polling = "=3.11.0"`, needed to name the types in alacritty's `EventedReadWrite` trait. Version 3.11.0 is already in `Cargo.lock` through `alacritty_terminal`, so only a dependency edge is added.
- **Specs:** a new `shell-integration` spec, and deltas to `configuration`, `command-line` and `terminal-emulation`.
- **Tests:**
  - unit tests for the scanner, including sequences split across reads, and for anchoring through scroll, history overflow, clear, alt screen and resize;
  - an end-to-end case per behaviour: prompt jumping, the failed-command mark's pixels, and selecting the last output in a local tab; prompt marks in a tmux control-mode pane; and a headless tmux check that `%output` carries OSC 133.
