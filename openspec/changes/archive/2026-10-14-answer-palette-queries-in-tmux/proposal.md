# Proposal

## Why

Programs in control-mode tmux panes get no reply when they query a palette color with OSC 4 (`ESC ] 4 ; <n> ; ? BEL`). This was seen while testing `report-pane-colors-to-tmux`. tmux 3.6 answers OSC 4 from the pane's palette, or else by asking an attached terminal client's terminal. A control client has no palette and no terminal to ask. Programs that adapt their colors to the terminal's palette time out waiting and fall back to guesses. In a local Brindle tab, they get the theme's colors.

## What Changes

- Brindle answers OSC 4 queries in tmux panes itself, with the theme's palette, by sending the reply into the pane as input (`send-keys -H`). The query reaches control clients in `%output`, even when tmux answers it.
- Brindle stays silent when tmux will answer, so a program never gets two replies:
  - the pane's palette has the index (a program set it with OSC 4, or the `pane-colours` option sets it);
  - a terminal client is attached to the pane's session, since tmux forwards the query to its terminal and relays the answer.
- OSC 10/11 queries are unchanged: tmux keeps answering them from Brindle's color reports.

Options considered and rejected (see design.md):
- **Report the palette to tmux.** tmux 3.6 ignores OSC 4 reports sent with `refresh-client -r`; it reads only OSC 10/11 from them.
- **Set the pane palette.** tmux answers OSC 4 from `pane-colours`, but it is state other clients see and render with, and it would have to be removed on detach.
- **Ask upstream.** Not needed for the answer itself. It remains an option if tmux ever lets a control client answer queries.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `tmux-control-mode`: programs in panes get the theme's palette colors from OSC 4 queries, with one reply.

## Impact

- **Code:** `src/terminal/mod.rs` (color requests in remote terminals go to a pane-supplied handler instead of being dropped), and `src/tmux/mod.rs` (the checks with tmux and the reply through `send-keys -H`).
- **Tests:** unit tests for the checks; end-to-end cases for the reply, for the cases where tmux answers, and a headless case pinning the tmux behaviour this relies on.
- **Risk:** a duplicate reply in the cases Brindle can't see. Those are listed in design.md.
