# Design

## Context

- **One view per terminal.** Every terminal on screen is a `TerminalView`: a local tab's single terminal and each control-mode pane alike. `TmuxWindowView` positions the pane views; tmux pane terminals are `Remote` terminals with their own alacritty grid and scrollback, fed from `%output`.
- **Key contexts.** The stack in a control-mode tab is `Workspace > TmuxWindow > Terminal`, and `Workspace > Terminal` in a local tab. User bindings are always loaded into `Terminal`. GPUI resolves bindings before `on_key_down`. `TerminalView::key_down` encodes any unbound special key and sends it to the program.
- **Focus.** `TerminalView` uses `on_focus_in`/`on_focus_out`, which fire when focus enters or leaves the handle *or its descendants*. They drive focus reports (mode 1004) and, in tmux, `select-pane`. The IME input handler is registered for the terminal's own focus handle only.
- **alacritty_terminal search.** `term::search` provides `RegexSearch` (lazy DFAs), `Term::search_next` (wraps around the buffer), `RegexIter` (ordered matches between two points) and `Term::scroll_to_point`. `RegexSearch::new` hard-codes smart case: case-insensitive unless the pattern has an uppercase letter. Matches never cross an unwrapped line break. Every search takes `&mut RegexSearch` (DFA caches).
- **Debug hooks.** `--send`/`--action` steps and `--dump-screen-after` drive the e2e suite. The dump prints overlays (palette, settings) after the screen text, and the e2e parser stops collecting screen lines at `--- settings`.

## Goals / Non-Goals

**Goals:**
- One implementation for local tabs and control-mode panes.
- No search work during `render`. Prepaint does only work bounded by the viewport, of the same order as the per-cell loop it already runs.
- Search logic in a pure module that unit tests can drive with a real `Term`.
- Checkable end to end without synthetic input.

**Non-Goals:**
- Tracking matches exactly while output streams in (see Risks).
- ~~IME composition in the query field.~~ *Amended after rebasing onto `main`, which had added `src/text_input.rs`:* the query is main's `TextInput`, which supports input methods.
- Searching more than one terminal at once.

## Decisions

### 1. The find bar belongs to `TerminalView`

Each `TerminalView` holds `find: Option<FindBar>` and renders the bar as an absolutely positioned child at its top-right corner, *inside* the div that tracks the terminal's focus handle. The bar has the key context `FindBar`. *Amended after rebasing onto `main`, which had added `src/text_input.rs`:* its focus is that of its query, a `TextInput` (decision 11), as the palette's is.

Consequences:
- Control-mode panes get a bar per pane for free, scoped to that pane's `Terminal`, as the spec requires.
- Moving focus between a terminal and its own bar stays inside the terminal's focus subtree. Neither `on_focus_in` nor `on_focus_out` fires, so no focus report reaches the program, and tmux is not sent `select-pane`, while the user types a query. The cursor draws hollow while the bar has focus, because the element checks the terminal handle exactly.
- Bar actions bubble from `FindBar` to the `Terminal` div, so all handlers live on `TerminalView`.

`TerminalView::key_down` returns early unless the terminal's own handle is focused. Without that, keys the bar doesn't consume, such as arrows, would bubble up and be encoded to the program.

The bar `occlude`s the mouse so clicks on it don't start a selection in the terminal underneath.

*Alternatives:*
- A `Workspace` overlay like the palette, aimed at the active terminal. Rejected: in a split tmux window it would sit over the whole tab rather than the pane it searches. The highlight state would also have to reach the `TerminalElement` of a view the workspace doesn't own.
- A separate `FindBar` entity. Rejected as unnecessary: the bar has no lifecycle apart from its terminal view. Keeping it in the view avoids a second subscription and event type.

### 2. Keys and actions

| Action (config name) | Default key | Context | Palette label |
|---|---|---|---|
| `Find` (`find`) | ctrl-shift-f | Terminal | "Find" |
| `FindNext` (`find_next`) | enter | FindBar | none |
| `FindPrevious` (`find_previous`) | shift-enter | FindBar | none |
| `FindClose` (`find_close`) | escape | FindBar | none |
| `FindToggleCase` (`find_toggle_case`) | alt-c | FindBar | none |
| `FindToggleRegex` (`find_toggle_regex`) | alt-r | FindBar | none |

- ctrl-shift-f follows the ctrl-shift-* convention for app shortcuts (GNOME Terminal, kitty and Konsole use it for find), and it is free.
- alt-c and alt-r are VS Code's find-widget toggles.
- *Amended after rebasing onto `main`, which had added `src/text_input.rs`:* paste needs no `FindBar` binding: the query's `TextInput` context binds ctrl-shift-v, shift-insert and ctrl-v to its own paste, and it is deeper than `Terminal`, so pasting never reaches the program.
- Only `Find` gets a palette label. The others mean nothing until a bar is open and act on the bar.
- All handlers are registered on the `Terminal` div, so `find_next`/`find_previous` bound by the user (in `Terminal`) also work with focus in the terminal. With no bar open they do nothing.
- Escape is bound only in `FindBar`, so with focus in the terminal it still reaches the program (vim, less).
- `Find` with the bar already open focuses it instead of toggling it, as browsers and VS Code do. Escape is the way to close.

### 3. Direction: "next" is up

Enter moves to the next match *up*, into older output, and shift-enter moves down. The first match is the nearest one at or above the bottom of the view. In a terminal the newest output is at the bottom and you search back through history, so this matches alacritty's backward search (its ctrl-shift-f), where enter continues upward.

Matches are numbered from the newest up (`1/11` is the bottom-most), so enter always increments the number.

*Alternative:* document order (top to bottom), as in editors. Rejected: the first enter after typing would jump away from the history you are reading, and the count would start somewhere in the middle.

### 4. Case and regex through the pattern

`search::pattern(query, options)` builds the string given to `RegexSearch::new`:
- Regex off: the query is escaped by a small hand-written function that backslashes the regex metacharacters `\.+*?()|[]{}^$#&-~`. This avoids adding `regex-syntax` as a direct dependency.
- The pattern is prefixed with `(?i)` or `(?-i)`. An inline flag overrides alacritty's smart-case syntax configuration, so match case is exactly the toggle and doesn't depend on whether the query has capitals.
- Empty query: no regex at all, because an empty pattern would only produce empty matches. The bar shows no count.

Defaults are match case off and regex off, as in browsers and VS Code. A unit test proves both directions of the case override, because that is where alacritty's smart case would leak through.

An invalid regex, including one too large to compile, gives `Err`. The bar shows "Invalid regex", there are no matches, and the current match is cleared. Keeping stale highlights for an expression that no longer exists would mislead.

### 5. State and the search origin

`FindBar` holds:
- the query field and options;
- the compiled `RegexSearch` (or `None`);
- the error;
- `focused: Option<Match>`;
- `origin: Point`;
- the display offset when the bar opened;
- the count;
- a debounce task.

The incremental-search origin is set to the bottom-right cell of the view when the bar opens. Every query edit searches from it with `search_next(Left, Side::Left)`, so refining the query never runs away from where the user was looking. That is the same rule alacritty uses.

Next and previous search from one cell beyond the current match (`start - 1` going up, `end + 1` going down, with `Boundary::None` so it wraps) and set `origin` to the new match's start. Typing after stepping then refines from the new match.

### 6. Scrolling to the current match

If the current match isn't fully on screen, the view is scrolled so that its first line sits at the middle row:
- target offset = `screen_lines / 2 - start.line`, clamped to `0..=history_size`;
- applied as `Scroll::Delta(target - current)`.

A pure function computes the target, and unit tests cover its clamping.

*Alternative:* `Term::scroll_to_point`, which scrolls minimally and leaves the match on the top or bottom row with no context, possibly under the bar. Rejected for that reason.

### 7. Highlights are computed in prepaint, bounded by the viewport

At the top of `TerminalElement::prepaint`, before it reads the view, `view.update` computes the visible matches. A `RegexIter` runs from the start of the wrapped line containing the top row to the end of the wrapped line containing the bottom row, at most 100 extra lines either way, as alacritty bounds it. Matches that end above the view or start below it are dropped.

The cell loop walks the sorted matches with an index while `display_iter` yields cells in order, so each cell costs O(1).

This is per frame, but bounded by the viewport, like the cell loop prepaint already runs. It is also always correct after output, scrolling or a resize. Caching per content generation would need a change counter the alacritty `Term` doesn't have.

### 8. Color precedence

From highest:
1. the cursor's own colors;
2. selection;
3. the current match;
4. other matches;
5. the cell's colors.

Selection wins because it is the user's most recent explicit act. Inverse and dim are applied before the override; hidden text stays hidden, by using the background as foreground after the override.

### 9. Theme colors and their defaults

`Theme` gains four `Option<Color>` fields:
- `search_match_background` / `search_match_foreground`;
- `search_focused_match_background` / `search_focused_match_foreground`.

Accessors derive the missing ones from the theme itself:

| Color | Default |
|---|---|
| match background | yellow (`ansi[3]`) mixed halfway towards the background |
| match foreground | the theme foreground |
| focused match background | yellow (`ansi[3]`) |
| focused match foreground | the theme background |

This is the familiar browser look: dim marks for every match and a solid one for the current match. It is derived per theme, so light themes get dark text on a light tan. Every built-in theme's yellow contrasts with both its foreground and its background. The built-in themes leave the fields `None`.

Persistence follows the precedent of `cursor_text` and `accent`:
- `with_effective_colors` fills all four in;
- `upsert_theme` writes them explicitly;
- the settings editor gets four non-optional slots after Accent, pre-filled with the effective colors.

As with `accent`, a saved custom theme then keeps its search colors when its yellow is edited later. That is accepted for consistency with how the other derived colors are saved.

*Alternative:* leave them out of the editor. Rejected: the settings-dialog spec promises every theme color.

### 10. Match count, debounced and capped

The count is computed with `RegexIter` going left from the bottom-right of the buffer and stops after 1000 matches (shown as `1000+`). The same pass finds the current match's number.

It runs:
- on the next tick after a query or option change, debounced by 100 ms, so typing quickly scans once;
- synchronously after next/previous, for one scan per keypress;
- debounced by 300 ms after the terminal reports new content while the bar is open.

The count pass also checks that the current match is still one of the matches. If output shifted the grid under it, the current match is cleared, and the bar shows `–/N` until the next step.

A query with no match scans the whole buffer, at most 10,000 lines by default. Measured during implementation, at 114 columns with alacritty at opt-level 2: about 60 ms for a full scan with no match, and about 1 ms for a capped count of a common letter.

60 ms on every keystroke is noticeable, so while the query is being edited, the search for the current match looks at most 1000 lines up from the origin (`MAX_LINES_WHILE_TYPING`, alacritty's own limit while typing). If that finds nothing, the debounced count pass runs one full search from the origin before counting. A distant match is therefore still found and scrolled to about 100 ms after the user stops typing. Enter and shift-enter always search the whole buffer.

### 11. Editing and the restored query

*Amended after rebasing onto `main`, which had added `src/text_input.rs`:* the query is main's `TextInput` entity, the same input as the palettes and the settings dialog, so cursor movement, selection, mouse positioning, clipboard and input methods behave as specified by `text-input`. The bar:
- subscribes to `TextInputEvent::Changed` and searches again on every change of text (not on cursor moves);
- uses `set_text` for `--find`, which emits nothing, and searches itself;
- calls `select_all` on a restored query, so it is drawn selected and the next edit replaces or removes it.

Enter, shift-enter, escape, alt-c and alt-r stay in the `FindBar` context around the input; the `TextInput` context doesn't bind them, so they reach the bar, as they reach the palette. Main's input binds ctrl-a to the start of the line (readline) and has no select-all key; double-click selects a word.

Before the rebase, the bar used `picker::TextField` with a `selected` flag. A follow-up change, `edit-text-fields`, had given that field a cursor and selection, but its highlight and click positioning failed hand-testing, and it was dropped in favour of main's input.

The last query and options are remembered per `TerminalView` when the bar closes.

Seeding the query from the terminal selection is left out. The restored-query behaviour covers the "search again" case, and seeding would clash with the restored query.

### 12. Closing restores the view

`find_close`, escape and the × button:
1. drop the bar state;
2. scroll back to the display offset saved when the bar opened, clamped to the current history;
3. focus the terminal, if the bar had focus.

The saved offset is restored as is. If the user was scrolled up and output arrived meanwhile, alacritty has already shifted their offset, so the restored position can be off by those lines. That is accepted as rare.

Clicking the terminal focuses it through the existing `mouse_down` and leaves the bar open, as in VS Code. Typing in the terminal goes through `Terminal::input`, which already snaps the view to the bottom.

### 13. Debug hook and dump line

- `--find <TEXT>` is a new step type. `Workspace::find_in_active` opens the bar on the active terminal (the active pane in tmux) and sets the query through the same path as typing.
- The dump prints, after the screen text: `--- find query="…" case=<bool> regex=<bool> count=<n|n+|none> index=<k|-> focused=<row>,<col>|offscreen|none error=<…>`, where row and col are the current match's start on screen.
- The e2e parser stops screen collection at `--- find`, as it does at `--- settings`.
- `Run` in the e2e harness gains a profile setter, so a case can use the local `Shell` profile.

## Risks / Trade-offs

- **[The current match drifts under live output]** alacritty grid points are relative to the bottom of the screen, so every new line shifts them. → Mitigation: all-match highlights are recomputed every frame and stay right. The current match is re-validated by the debounced count pass and cleared if it no longer matches. A match that still matches after a shift stays highlighted at the shifted text, which is harmless.
- **[Pathological regex]** For example `.`, which matches every cell. → Mitigation: the count stops at 1000. Visible matches are bounded by the viewport. alacritty's lazy DFA gives up on runaway state growth, which reads as no match.
- **[The bar covers the top-right of the terminal]** → Mitigation: matches scrolled into view land mid-screen (decision 6). The bar is narrow, and shrinks to the pane width in small tmux panes.
- ~~**[No IME in the query]**~~ *Amended after rebasing onto `main`, which had added `src/text_input.rs`:* resolved: main's `TextInput` supports input methods.
- **[Restored offset can be off after output while scrolled up]** → Accepted (decision 12).

## Migration Plan

Additive: new actions, keys and optional theme keys. ctrl-shift-f was unbound, so it used to reach programs as a ctrl-shift-f sequence. Users who relied on that can bind it to `none`. Roll back by reverting.
