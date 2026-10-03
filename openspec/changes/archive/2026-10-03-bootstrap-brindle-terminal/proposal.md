# Proposal

> **Backfilled record.** This change describes work that was already built and
> committed (commits `a87eb34` through `488ca95`, 2026-10-03) before the
> project used OpenSpec. It is written after the fact to give later changes a
> baseline and a record of why things are the way they are.

## Why

The goal was a GPU-accelerated Linux terminal emulator in Rust, built on GPUI
(the UI toolkit Zed uses), with tabs, profiles and tmux support good enough to
be the main way of using tmux. No existing terminal combined GPUI's rendering
stack with native tmux integration.

## What Changes

- New application `brindle`: one window per workspace, tabs drawn in a
  client-side-decorated title bar, and a GPU-rendered terminal grid per tab.
- Terminal emulation through `alacritty_terminal` on a local PTY, with
  xterm-compatible keyboard and mouse encoding, bracketed paste, focus
  reporting and OSC 52 clipboard.
- Profiles: named launch configurations (command, args, directory,
  environment, theme, font size, tmux mode) and a filterable profile picker.
- A TOML config file that is created on first run, validated, and reloaded
  live. It includes built-in and custom themes and keybinding overrides.
- tmux in two modes:
  - *Classic*: tmux runs inside a tab with full protocol support.
  - *Control mode* (`tmux -C`): each tmux window becomes a native tab, panes
    are drawn natively on tmux's layout grid, existing panes are restored on
    attach, and detaching leaves the session running.
- Command-line options for launching a profile or command, plus debug options
  (`--send`, `--action`, `--dump-screen-after`) for automated testing.

## Capabilities

### New Capabilities
- `terminal-emulation`: running programs on a PTY and translating keyboard,
  mouse, paste, focus and clipboard traffic between the user and the program.
- `terminal-rendering`: drawing a terminal grid: colors, text attributes,
  cursor, selection, wide and box-drawing characters, IME pre-edit and fonts.
- `tabs`: windows and tabs: opening, closing, switching, reordering, titles,
  bell indicators and the tab strip that doubles as the title bar.
- `profiles`: named launch configurations and how the user picks one.
- `configuration`: the config file, its defaults, validation, live reload,
  themes and keybinding overrides.
- `tmux-control-mode`: mirroring a tmux session through `tmux -C`, with
  windows as tabs and natively drawn panes.
- `command-line`: command-line launch options and the debug automation hooks.

### Modified Capabilities
<!-- None: this is the first change in the project. -->

## Impact

- New Rust crate (`src/`), built on `gpui` 0.2.2 and `alacritty_terminal`
  0.26. `Cargo.lock` starts from gpui's own lockfile.
- Linux only, on Wayland or X11. Rendering needs Vulkan.
- Build dependencies: system dev packages, or the rootless toolchain in
  `.toolchain/` (`bootstrap.sh`, `env.sh`).
- Writes `~/.config/brindle/config.toml` on first launch.
- Control mode needs `tmux` on `PATH` (tested with 3.6).
