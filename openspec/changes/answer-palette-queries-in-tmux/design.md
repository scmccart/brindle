# Design

## Context

- A tmux pane's `Terminal` is `Remote`. alacritty raises `ColorRequest(index, format)` for OSC 4 (`index` 0–255) and for OSC 10/11/12 (256 and up). `handle_event` computes the color (the program's own setting, else `default_color`) and calls `reply()`, which drops it for remote terminals, because tmux answers queries itself.
- Checked against a throwaway tmux 3.6 server with a raw `tmux -C` client:
  - An OSC 4 query reaches control clients in `%output`, even when tmux answers it.
  - With no palette entry and no terminal client, tmux sends no reply.
  - `refresh-client -r "%0:\033]4;1;rgb:…\033\\"` is accepted and ignored: tmux reads only OSC 10/11 from reports.
  - tmux answers the index from the pane's palette when a program set it with OSC 4 (replying `rgb:ffff/0000/0000` for `rgb:ff/00/00`), or when `pane-colours[N]` sets it. `#{pane-colours[N]}` expands that index, including a value inherited from `set -g`. `#{pane-colours}` alone lists values without their indexes and misses inherited ones.
  - With a terminal client attached to the pane's session, tmux forwards the query to that client's terminal and relays its answer into the pane. A client attached to another session gets nothing. `list-clients -t %N -F '#{client_control_mode}'` lists the session's clients, `0` for a terminal client.

## Goals / Non-Goals

**Goals:**
- One reply per OSC 4 query in tmux panes, with Brindle's theme when tmux doesn't answer.
- No change to the server: no options set, nothing to undo on detach.

**Non-Goals:**
- OSC 10/11/12. OSC 10/11 stay with the color reports. OSC 12 (cursor color) is still not answered in tmux panes.
- Answering palette queries that arrive while a pane is being restored. Its output is dropped until the snapshot is in, as it is today.

## Decisions

1. **Brindle answers, not tmux.** The pane's `Terminal` already knows the color: the theme's palette, through `Theme::indexed`. The reply goes back as input, through `send-keys -H`, like a keystroke.
   - Alternative rejected: reporting the palette with `refresh-client -r`. tmux 3.6 ignores OSC 4 reports.
   - Alternative rejected: setting `pane-colours` on each pane. Terminal clients would render the pane with Brindle's palette, the option would have to be removed on detach, and a crash would leave it behind.
2. **Remote terminals hand palette replies to their owner.** `Backend::Remote` gains an optional palette handler, `Rc<dyn Fn(usize, Vec<u8>)>`, given the index and the formatted reply. `handle_event` calls it for `index < 256` when the program hasn't set the index itself (`term.colors()[index]` is `None`). Everything else still goes to `reply()` and is dropped. tmux panes pass a handler. Message tabs pass none.
   - Program-set indexes are skipped locally because tmux has them in the pane's palette and answers. alacritty tracks the same OSC 4 and OSC 104 sequences.
   - OSC 10/11 must stay dropped: tmux answers them from the reports, so a local answer would double them.
3. **Ask tmux before answering.** The handler sends `list-clients -t %N -F '#{client_control_mode}'` (`Pending::PaletteClients`). If every line is `1`, it sends `display-message -p -t %N '#{pane-colours[I]}'` (`Pending::PaletteOption`). If that is empty, it sends the reply with `send-keys -H`. An error reply at either step drops the reply.
   - Asking at query time is exact, and costs two local round trips (milliseconds) against programs' usual timeouts of 100 ms or more. Queries are rare, so nothing is cached or polled.
   - `client_control_mode` and the `-F` flag work on every tmux Brindle supports. `list-clients -f` would need a newer tmux.
   - Alternative rejected: following `pane-colours` through the subscription or poll table. Checking it properly means each of 256 indexes, and inherited values don't show in `#{pane-colours}`.
4. **Pure checks, unit-tested.** `palette_clients_allow(lines)` (no line is `0`) and the reply's `send-keys` commands are pure functions in `src/tmux/mod.rs`.

## Risks / Trade-offs

- [After a reattach, Brindle's new `Terminal` doesn't know which indexes a program set before the attach, but tmux still has them] → Brindle adds a second reply after tmux's, and it arrives as stray input. It needs a program that sets its palette, a detach, and a query of the same index afterwards. No format exposes the pane's palette, so this is recorded and accepted.
- [Two control clients that both answer, such as two Brindle processes attached to one session] → Each answers, and the program gets two replies. Brindle shows a session in one window, so this needs a second Brindle process. Accepted.
- [A future tmux answers OSC 4 for control clients, or stops passing the query through] → The headless `tmux-osc4-unanswered` case checks both on every e2e run, so the change shows up before users get two replies.
- [A program that sends a query and exits before the reply arrives] → The reply arrives as input to whatever runs next, as with any slow terminal. The round trips keep this window to milliseconds.

## Migration Plan

None. Brindle sets no tmux state, so dropping the change needs no cleanup.
