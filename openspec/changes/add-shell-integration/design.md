# Design

## Context

- **Parsing:** local tabs use alacritty's `EventLoop`. It reads the PTY on its own thread and calls `Processor::advance(&mut Term, bytes)` itself, so Brindle never sees the bytes. tmux panes (`Backend::Remote`) call `processor.advance` in `Terminal::feed`.
- **OSC 133 is dropped:** vte 0.15's `osc_dispatch` passes unknown OSC numbers to `unhandled()`, which only logs them. `Handler` has no hook for them, and alacritty 0.26 has no semantic-prompt support. Recognising the marks means scanning the byte stream ourselves, at a point where we also hold the `Term`.
- **Grid coordinates:** `Line(0)` is the top of the live screen and negative lines are history. Lines shift whenever output scrolls. When the history is full, the oldest row is recycled as the new bottom row and `history_size()` stops changing, so it can't tell you how far things scrolled. `Term::resize` reflows wrapped lines (`WRAPLINE` on a row's last cell) on the primary screen. On the alternate screen, the primary grid is `inactive_grid`, which is private, and it is reflowed out of sight.
- **tmux:** tmux 3.6 control mode puts the pane's raw output, OSC 133 included, in `%output`. This was checked against a throwaway server; the `tmux-osc133-passthrough` e2e case keeps checking it. Classic tmux interprets OSC 133 itself (3.4+ uses it for `next-prompt`/`previous-prompt` in copy mode) and does not pass it on.
- **Keys:** ctrl-shift-up/down are bound to `ScrollLineUp/Down` in the `Terminal` context. The other usual prompt-jump keys are taken: kitty and foot use ctrl-shift-z/x, which Brindle's `TmuxWindow` context uses for zoom pane and close pane; Ghostty uses ctrl-shift-pageup/down, which move tabs here.
- **Shells available here:** only bash 5.3 is installed. The zsh and fish scripts follow those shells' documented hook APIs but can't be run on this machine.

## Goals / Non-Goals

**Goals:**
- Exact mark positions in local tabs and control-mode panes, with no visible effect on rendering or throughput.
- Marks that stay on their text through scrolling, history overflow, clearing, resets, resizes and full-screen programs.
- Prompt navigation, output select/copy and failure marks, all reachable from the palette and keybindable.

**Non-Goals:**
- Automatic injection of the scripts into shells (see decision 8).
- Click-to-move-cursor (`click_events`), per-command durations or notifications, and "select output under the mouse". The mark store makes these possible later.
- Marks for output restored by `capture-pane` on attach, which carries no OSC sequences.
- 8-bit C1 introducers (`0x9d`). vte runs in UTF-8 mode, where those bytes are continuation bytes, and no shell integration emits them.

## Decisions

### 1. The PTY's reader does the parsing

`EventLoop<T: EventedPty>` only ever reads through `pty.reader()`. Brindle wraps alacritty's `tty::Pty` in `MarkedPty`, which forwards registration, writes, resize and child events to the inner `Pty`. Its `reader()` is a `ParsingReader` that:
1. reads up to 64 KiB from a `try_clone` of the PTY file (same open file description, so the same non-blocking flag);
2. takes the shared parser and the `Term` (`lock_unfair`, because the event loop already holds the `FairMutex` lease on this thread), runs the bytes through the OSC 133 scanner and the vte processor, and records marks;
3. sends `Wakeup`, and returns `Ok(0)`.

`pty_read` treats `Ok(0)` with nothing buffered as "done for now" and goes back to `poll`, which is level-triggered, so leftover data wakes it again at once. alacritty keeps everything else: its poller, the write queue, resize, child-exit detection and drain-on-exit. Its own `Processor` never sees a byte.

Synchronized updates (mode 2026) now buffer in Brindle's processor, so the timeout flush moves to the GPUI side: on `Wakeup`, if the processor is buffering, a timer calls `stop_sync` when it expires. This is the same code `Backend::Remote` already uses, now shared by both backends.

- *Alternative:* our own PTY event loop. Rejected: it re-implements alacritty's poller, write queue and SIGCHLD handling for no gain.
- *Alternative:* a reader that only tees bytes to a scanner. Rejected: `pty_read` may parse several reads in one go when the lock is busy, so a tee can't tell which `Term` state a mark belongs to.
- *Alternative:* rewriting OSC 133 into a sequence alacritty acts on (DSR 6 makes alacritty report the cursor position). Rejected: the report is relative to the screen, it would race with real DSR replies, and history size isn't available from inside the listener.
- *Alternative:* a delegating `Handler` wrapper around `Term`. Rejected: vte never passes unknown OSCs to a `Handler`, and wrapping 70-odd methods by hand is fragile.

**Lock order:** parser → `Term` (`lock_unfair` while holding the parser) → marks. Render and query code takes `Term` → marks, and never takes the parser.

### 2. A small scanner splits the stream at the points that matter

`Scanner` is a byte state machine mirroring vte's: ground, escape, CSI, OSC, and DCS/SOS/PM/APC strings. CAN/SUB abort, and an ESC inside a string ends it. Its state persists across reads. It reports four kinds of split point:
- **OSC 133 mark**: the split comes after the terminator, BEL or the ESC of `ESC \`; vte dispatches the OSC on that ESC. Only the first 64 bytes of an OSC are kept, which is enough for `133;D;<exit>;…`.
- **Alternate screen switch** (`CSI ? 1049|1047|47 h/l`): the split comes before the final byte.
- **Clear history** (`CSI 3 J`): the split comes before the final byte, and the final byte is then fed on its own.
- **Full reset** (`ESC c`): the split comes after it, and every mark is dropped.

Bytes between split points go to the processor in one call. A read without these sequences costs one pass of the scanner (a byte match per byte) on top of vte.

### 3. Marks are stored as absolute line numbers, with a scroll counter kept from row identity

A position is `(abs_line: i64, column)`, and `abs_line = base + Line`. `base` counts every line that has scrolled off the top of the live screen since the terminal started, including lines later dropped or cleared. Before each processor call, `Marks` notes the address of the cell buffer of the row at `Line(0)`, plus the cursor's row and line. alacritty moves rows by rotating its ring buffer and never reallocates a row's cells (it reuses them when recycling), so after the call the same address shows up at `Line(-k)` when `k` lines scrolled into history. A search upward from line 0 costs O(k). The address is only compared, never dereferenced.
- If the top row is not found (more than the whole history scrolled, or the history was wiped), the cursor's old row is tried, since it is the lowest row with content and lasts longest.
- If neither is found, everything older has gone, so all marks are dropped.
- Marks above the top of the history are pruned after every call.

Why this rather than counting `history_size()`: the count stops moving once the history is full (10,000 lines by default), and marks would drift by one line per line of output from then on. Raising the history limit temporarily to measure was also considered. Rejected: alacritty then allocates and frees up to 1,000 rows on every read.

Why not mark the cells themselves (a spare `Flags` bit, a zero-width character, an invisible underline color): the shell overwrites the cell where `A` lands with the first prompt character, and redrawn prompts overwrite the rest. Zero-width characters would also end up in copied text.

### 4. Resizing remaps through logical lines

Before `Term::resize`, each mark is converted to (logical lines above the cursor's logical line, offset in cells within its logical line), counting `WRAPLINE` continuations. After the resize, the same walk from the new cursor position converts them back, so the mark lands where alacritty's reflow put its text. This is O(rows between the oldest mark and the cursor), and it only runs when there are marks.

On the alternate screen the primary grid can't be read, so the logical form of every mark is captured when the screen switch is seen (the split before `?1049h`). A resize while on the alternate screen sets a flag, and on the way back the marks are rebuilt from that capture against the primary cursor. That cursor is the one in place at entry, after alacritty's reflow; `?1049l` restores the same saved cursor.

### 5. Marks during a synchronized update flush it early

While mode 2026 is active, vte buffers bytes without applying them, so the cursor position is stale. When a mark, a screen switch or a history clear arrives during a synchronized update, Brindle calls `stop_sync` first. The update is then drawn without synchronization from that point on. Shell prompts don't use mode 2026, so in practice this only affects TUIs that switch screens inside a synchronized update, and only for one frame. If a switch happens inside a buffered block that is flushed later (by timeout), the tracker sees the screen change across one processor call, assumes the primary screen didn't scroll, and leaves the marks alone.

### 6. Grouping and edge cases

- `A` on the same line as the latest command that has no `C` yet is a redraw, so it replaces the prompt mark. This also removes duplicates when fish 4 emits its own marks alongside the script.
- `D` counts only after a `C`. bash shows no PS0 for an empty line, so the script can send `D;$?` on every prompt without an empty enter inheriting a failure; ctrl-C at a prompt is also ignored.
- A missing or non-numeric exit status is "unknown", and unknown is never shown as a failure.
- Marks are ignored while the alternate screen is active: they belong to the primary screen's text.
- The store keeps at most 10,000 commands.

### 7. Keys, actions and the gutter mark

- **Keys:** ctrl-shift-up/down are rebound to `jump_to_previous_prompt`/`jump_to_next_prompt`. These are the keys the request named, and iTerm2 uses the same arrows with cmd. When a terminal has no prompt marks, the handlers fall back to scrolling one line, so nothing changes for users without integration. The reused alternatives (ctrl-shift-z/x, ctrl-shift-pageup/down) are rejected in Context, and plain ctrl/alt arrows belong to programs and GNOME. `scroll_line_up`/`scroll_line_down` lose their default keys but stay bindable and in the palette. The README documents how to restore them.
- **Jump targets:** the target line goes to the top of the view, as in iTerm2, WezTerm and kitty. The reference point is the top visible line, so repeated presses walk through prompts.
- **Last output:** select and copy have no default keys. Copy uses `Term::bounds_to_string` and leaves the selection alone. Select also sets the primary selection, as a mouse selection does. The output end is the `D` mark. A `D` at column 0 ends on the previous line.
- **Gutter mark:** a bar 2 px wide, or 3 px when the padding is 6 px or more, drawn 1 px left of the grid in the theme's `ansi[1]`. tmux panes (no padding) draw it over the left 2 px of column 0. It spans the prompt start row through the command start row, so a two-line prompt is covered. It is computed in `prepaint` under the `Term` lock that is already held: it walks the commands from newest to oldest and stops at the first one above the viewport.
- **Config:** `mark_failed_commands`, a flat boolean like `copy_on_select`, default `true`.

### 8. Scripts are opt-in and also embedded

`shell-integration/brindle.bash`, `.zsh` and `.fish` are compiled in with `include_str!`, and `brindle --shell-integration <shell>` prints one. `eval "$(brindle --shell-integration bash)"` then works for installed binaries with no checkout. Brindle doesn't inject them (no `--rcfile`, `ZDOTDIR` or `XDG_DATA_DIRS` rewriting):
- injection changes how the user's own startup files load, which is surprising and easy to break;
- it wouldn't reach shells inside tmux, ssh or containers anyway;
- WezTerm and Ghostty-style opt-in sourcing is the conventional answer, and the request allowed it.

The scripts don't check `TERM_PROGRAM`, because inside tmux it is `tmux`. Terminals that don't understand OSC 133 ignore the marks.

- **bash:**
  - `PROMPT_COMMAND` is wrapped with newline separators (safe whatever the existing value ends with). It becomes `__brindle_save` (stores `$?` and returns it again) + the user's commands + `__brindle_prompt`. Array-valued `PROMPT_COMMAND` (bash 5.1+) is extended in place.
  - `__brindle_prompt` prints `D;<status>` (except before the first prompt). It also adds `\[\e]133;A\a\]` and `\[\e]133;B\a\]` around `PS1`, and `\e]133;C\a` to `PS0`, re-checking on every prompt so tools that rebuild `PS1` (starship and the like) keep the marks.
  - bash older than 4.4 has no PS0, so it gets prompts but no output marks.
- **zsh:** a `precmd` hook first in `precmd_functions` saves `$?`. One last in the list prints `D` and wraps `PS1` in `%{…A…%}`/`%{…B…%}` when needed. `preexec` prints `C`. `A` lives in `PS1`, not printed from `precmd`, so it lands after zsh's `PROMPT_SP` line fix-up.
- **fish:** `fish_preexec` prints `C`, `fish_postexec` saves `$status`, and `fish_prompt` prints `D` and `A`. `B` isn't emitted: printing it inside `fish_prompt` would rely on fish leaving the escape out of its prompt width. Without `B`, the failure bar covers only the prompt's first row.
- All three guard against non-interactive shells and loading twice.

### 9. tmux

Control-mode panes need no extra work: `Terminal::feed` runs the same parser. Classic tmux swallows OSC 133. Passthrough (`allow-passthrough on` plus DCS-wrapped marks) is possible but needs changes to the user's tmux config and the scripts, so it is documented as unsupported for now. The headless `tmux-osc133-passthrough` e2e case watches for a tmux release changing what control mode forwards.

### 10. Dump selection line

`--dump-screen-after` prints `--- selection "<text>"` (Rust debug escaping) before `--- screen` when the active terminal has a selection. The e2e parser reads it. It comes before the screen block because everything after `--- screen` is screen text until `--- settings`.

## Risks / Trade-offs

- [The reader trick depends on `pty_read` treating `Ok(0)` as "no more data" and going back to `poll`, which is how alacritty 0.26 works.] → It's pinned to alacritty 0.26 and documented in the code. If an upgrade changes this, the symptom is a stalled or busy PTY, which every e2e case using a local tab would catch.
- [Row identity relies on alacritty never reallocating a row's cells outside resize.] → Resize is handled separately (decision 4). A wrong match would only misplace marks, never crash, because addresses are only compared. Unit tests cover scrolling, overflow, clears and resets against a real `Term`.
- [Rebinding ctrl-shift-up/down changes muscle memory for users who load the scripts.] → The fallback keeps the old behaviour without integration. The README shows the two-line rebind.
- [zsh and fish scripts weren't run on this machine.] → They follow the documented hook APIs. The hand-test checklist asks for a check where those shells are installed.
- [Marks inside a synchronized update end it early.] → Prompts never use mode 2026, so the visible cost is at most one unsynchronized frame.

## Migration Plan

None for config files: the new key is optional. Users who want line scrolling on ctrl-shift-up/down while using shell integration can add `"ctrl-shift-up" = "scroll_line_up"` and `"ctrl-shift-down" = "scroll_line_down"` under `[keybindings]`. Rollback means reverting the change. No data is stored.
