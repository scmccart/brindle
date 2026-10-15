# Design

## Context

The e2e harness already has what most of these checks need (see proposal.md - Why for the checks being ported):
- `Run` launches Brindle with `--action`/`--send` steps, a tmux timeline (`at`, `record_at`), a theme switch (`theme_at`) and one window capture.
- `Dump` parses tabs, the active terminal's size and `TermMode`, its grid, the palette lines and the other windows.
- `Cells` turns the grid into device-pixel rectangles, and `Image::count`/`expect` sample them.

Since the proposal was written, later changes added cases that already cover part of its list, and the `restore-*` prefix now belongs to session restore. This design settles what is new, what is already covered, and how each new case checks its behaviour.

## Goals / Non-Goals

**Goals:**
- Each behaviour verified by hand in the four archived changes is either a named case or named as already covered by one, or it is listed as manual in `CLAUDE.md`.
- Checks compare against tmux (its flags, listings and geometry) wherever tmux is the source of truth, as the existing cases do.

**Non-Goals:**
- No Brindle changes. A ported check that fails is reported, and fixed in its own change.
- No rendering checks for layouts beyond what `geometry-*` already does: the layout commands are checked by tmux state.
- Anything that needs a real keystroke (the ctrl-shift-d `Swallow` binding, IME pre-edit, wheel, classic-tmux prefix keys, drag-to-reorder).

## Decisions

### 1. What is already covered

| Proposal item | Covered by |
|---|---|
| Running an action through the palette (`palette-*`) | `input-palette-run` (Increase Font Size via filter and confirm) |
| The palette's dump description (`palette-*`) | the `--self-test` dump parser test, and every `input-*`/`session-end-ask`/`new-window-*` case that reads `Dump::palette` |
| Detaching from the only tabs (bootstrap 6.6) | `session-end-detach-close` and `session-end-new-tab` (the shell-tab fallback is now the `new_tab` setting) |
| Theme reload replies (bootstrap 5.4) | `colors-reload`, which checks OSC 10/11 after the reload but no pixels |
| Attach keeps the current window via session restore | `restore-tmux-session`, which uses restore and not a plain `-p` attach |

New cases cover only what these leave out.

### 2. Case names

`restore-*` is the session-restore group, and filtering on `restore` would mix them, so the pane-restore cases use `reattach-*`. The other groups keep the proposal's prefixes: `selection-*`, `palette-*`, `commands-*`, `dividers-*` and `reload-*`. Each group is a module in `tests/e2e/cases/` named after its prefix.

### 3. `reattach-*`: a scripted program, compared with tmux's flags

The original check reattached to vi. Which modes vi enables depends on its version, its config and `TERM`, so the cases run a `printf` that sets the modes and then waits in `cat`:
- `reattach-alt-screen`: 1049, DECCKM (`?1`) and DECKPAM (`ESC =`), with a marker line on the alternate screen;
- `reattach-mouse-modes`: 1002 and 1006.

Each case attaches, quits, and attaches again, so the second run is a real reattach to a pane whose modes were set before Brindle ever saw it. In both runs:
- the dump's `TermMode` tokens (split on ` | `) are compared with `display-message -p` flags: `alternate_on`→`ALT_SCREEN`, `keypad_cursor_flag`→`APP_CURSOR`, `keypad_flag`→`APP_KEYPAD`, `mouse_standard_flag`→`MOUSE_REPORT_CLICK`, `mouse_button_flag`→`MOUSE_DRAG`, `mouse_all_flag`→`MOUSE_MOTION`, `mouse_sgr_flag`→`SGR_MOUSE`;
- the screen is compared with `capture-pane` (`matches_tmux`).

For 1002+1006 this means `MOUSE_DRAG` and `SGR_MOUSE` are present and `MOUSE_MOTION` and `MOUSE_REPORT_CLICK` are absent. Alternative: hardcode the expected flags. Comparing with tmux catches a disagreement in either direction, and the case also asserts the expected values so that tmux and Brindle can't both be wrong.

### 4. `selection-*`: the old manual setup on a plain attach

The shared setup is the old one: windows 0–2 with window 2 current and window 0 last, and window 2 split into three panes with the second selected. Every case snapshots `list-windows` (`index:active last_flag`) and `list-panes -a` (`id:active`) before the run and compares after.
- `selection-current-window`: the dump's active tab is window 2's.
- `selection-active-pane`: the dumped active terminal matches the second pane (`matches_tmux` on it). The setup echoes each pane's id into it, so any other pane's text would fail the comparison.
- `selection-split-active-pane`: `--action tmux_split_right`. Afterwards there's one more pane, the other two panes' geometry is unchanged, the second pane is narrower, and the new pane has its top and height.
- `selection-background-window`: the timeline runs `new-window -d` at 2 s. The dump lists one more tab that isn't active, and tmux's current window is still 2. A second run with `--action tmux_new_window` shows the new tab active.

### 5. `palette-*`: tmux actions and per-tab listing

- `palette-tmux-action`: `open_command_palette`, filter `tmux: split right`, `picker_confirm` in a one-pane tmux tab. Afterwards the palette is closed, tmux has two panes, the new one (`%1`) is active, and the dumped terminal matches it, so focus followed.
- `palette-lists-for-tab`: two runs, the palette open with no filter.
  - In a tmux tab, the rows include `tmux: Split Right` and `tmux: Detach` with `[ctrl-shift-d]`.
  - In a local `Shell` tab, there is no `tmux: Split Right` and no `tmux: Detach`, while `tmux: New Session` (a Workspace action, listed wherever a control profile exists) and `Clear Scrollback` are present.

  Asserting on `tmux: ` as a prefix would be wrong because of New Session.

### 6. `commands-*`: assert tmux's state

Layout rendering is `geometry-*`'s job. These cases check what tmux did, read after each run. Each command gets its own short Brindle run, rather than several steps in one run with `record_at` listings between them, so the checks don't depend on step timing.
- `commands-break-pane`: two panes, `tmux_break_pane`. The dump has two tabs with the second active, `list-panes -a` shows window 0 with one pane, and the new window holds the pane that was active.
- `commands-layouts`: three panes. The steps are each of the four layout actions, then `tmux_next_layout`, with `list-panes` geometry recorded after each:
  - even-horizontal: every pane at top 0 with the window's full height;
  - even-vertical: every pane at left 0 with the full width;
  - main-vertical: the first pane at 0,0 with the full height, and the others to its right;
  - tiled: a 2x2 arrangement (two distinct tops and two distinct lefts);
  - next layout: a geometry different from the tiled one.
- `commands-rotate-swap`: three panes in a row, with the middle one active so that both directions have a neighbour.
  - After `tmux_rotate_panes`, the pane ids by position are the old order rotated by one.
  - After `tmux_swap_pane_next`, the active pane id is unchanged and has moved one position later.
  - After `tmux_swap_pane_prev`, it is back in the middle.
- `commands-detach-beside-local`: a `Shell` tab, then `new_tab_with_profile_2` on a session with two windows.
  - `activate_tab_1` (the local tab), then `tmux_detach`: all three tabs remain and `list-clients` still shows Brindle. This only shows that the action has no handler outside control-mode tabs. The ctrl-shift-d `Swallow` binding needs a real key and stays manual.
  - `activate_tab_2`, then `tmux_detach`: only the shell tab remains, it is live (a probe `echo` prints), and `has-session` succeeds.

### 7. `dividers-*`: known colors, junction cells

Both cases set `pane-border-style fg=magenta` and `pane-active-border-style fg=yellow`, so the line colors are known and distinct from the background. Lines are 1 logical pixel, drawn through the middle of tmux's one-cell gap, so each check works on a rectangle rather than on single pixels, which `Cells::rect` rounding would make fragile. The rectangle is the 3x3 cells centred on the junction cell: one cell gives arms only half a cell long, and the cells around it along the lines are divider cells, while the pane cells at its corners hold no line colors.

A helper, `junction`, splits that cell rectangle at the line positions found in it (the row and column with the most line-colored pixels) into four arms: up, down, left and right from the crossing point. For each arm it reports the color, or a gap where a run of background pixels separates the arm from the crossing.
- `dividers-tiled`: four panes, `select-layout tiled`, `%0` (top-left) active. At the cross:
  - the up and left arms are yellow, because they border `%0`;
  - the down and right arms are magenta;
  - none has a gap.

  The highlight therefore turns its corner on the lines.
- `dividers-t-junction`: `%0` left with `%1` and `%2` stacked on the right, and `%1` active. At the junction on the vertical divider:
  - up is yellow (it borders `%1`), and so is right (the horizontal divider under `%1`);
  - down is magenta;
  - left is empty background. Nothing crosses there, which checks that the helper doesn't invent lines.

### 8. `reload-*`: pixels in panes and in a local tab

`reload-theme-attached` makes two runs:
1. A split tmux window, with `theme_at(1.5, "e2e-alt")` and a capture at 4 s. Both panes' backgrounds are `ALT_BACKGROUND`, and a line of plain text (typed before the run) is `ALT_FOREGROUND`.
2. `-p Shell`, then `new_tab_with_profile_2`, the theme switch, and `activate_tab_1` before the capture. The local tab's background is `ALT_BACKGROUND`.

So both kinds of tab redraw while a session is open, as the spec scenario says. `theme_at` rewrites the config with `config()`, which drops `top` and `extra_config`. Neither run uses them, and both profiles exist in `config()`.

### 9. Manual checks in `CLAUDE.md`

A short "Manual checks" list under "Verifying GUI behaviour" names what the suite can't do:
- ctrl-shift-d in a local tab sends nothing to the shell;
- IME composition (the IBus `ctrl-shift-u e9 space` steps);
- wheel scrolling in panes;
- classic-tmux prefix keys;
- tab drag-to-reorder;
- toggling the palettes by key.

## Risks / Trade-offs

- [One run per command makes `commands-layouts` and `commands-rotate-swap` slower, about 10–16 s each] → accepted, since they then don't depend on when steps land relative to the tmux timeline.
- [The tiled layout with three panes is uneven] → the check asserts two tops and two lefts, not equal sizes.
- [The junction helper finds the line rows by color counts, so text in a pane next to the junction could add matching pixels] → panes are cleared before capture, and the theme's magenta and yellow are not used by the plain shell prompt.
- [A ported check fails because Brindle regressed] → per the proposal it stays in the suite and is reported, and the fix goes in a separate change.
