# Proposal

## Why

Programs ask the terminal for its default foreground and background colors with OSC 10 and OSC 11, and use the answer to choose light or dark palettes or to blend muted text. In a control-mode tmux tab, tmux answers these queries itself, and because a control client has no terminal for tmux to ask, it answers black for both (`rgb:0000/0000/0000`). Programs in tmux tabs therefore see colors that don't match the theme, while the same program in a local tab gets the theme's real colors. This came up while investigating washed-out colors in tmux tabs. Those turned out to be Claude Code's own 256-color fallback, but the wrong OSC 10/11 answers were found along the way.

## What Changes

- For every pane Brindle mirrors, Brindle reports the pane's theme foreground and background to tmux with `refresh-client -r '%<pane>:<OSC 10/11 report>'`. tmux then gives those colors to programs that query OSC 10/11. On tmux 3.6, testing showed a query that returned black returns the reported colors once they are reported.
- Reports are sent when Brindle first learns of a pane (on attach, and for panes created later), in the same place it starts restoring the pane.
- When the configuration reloads, the colors are reported again for every pane, so a theme change reaches programs that query afterwards.
- Out of scope: OSC 4 palette queries and OSC 12 (cursor color). tmux doesn't answer OSC 4 from a control client at all, and `refresh-client -r` is documented only for OSC 10-style reports.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `tmux-control-mode`: a new requirement that programs in a control-mode pane get the theme's foreground and background when they query OSC 10/11, kept current on configuration reload. The "Input and focus" requirement still holds: Brindle doesn't forward its own replies to panes, and tmux keeps answering, now with the right colors.

## Impact

- **Code:** `src/tmux/mod.rs`:
  - Send the two reports per pane alongside the restore commands in the pane-creation loop.
  - Re-send for all panes in `set_config`. It needs the pane's theme, which is `config.profile_theme(&profile)` as used for `Terminal::remote`.
  - Add a helper that formats an `rgb:rrrr/gggg/bbbb` report as a quoted tmux argument, with a unit test.
- **Compatibility:** older tmux versions without `refresh-client -r` reject the command. The reply is matched as `Pending::Ignore`, which logs a warning and changes nothing else.
- **Tests:** a unit test for the command text. An end-to-end check on a throwaway server: attach, query OSC 11 in a pane, and confirm that the answer is the theme background.
- **Dependencies:** none.
