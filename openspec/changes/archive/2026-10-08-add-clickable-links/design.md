# Design

## Context

- **Mouse handling.** `TerminalElement::paint` registers mouse listeners that call into `TerminalView`. Down and wheel events count only over the element's hitbox. Move and up events are global while a drag is in progress, and otherwise move events count only over the hitbox. `TerminalView::mouse_down` sends a report when the program has mouse mode on, unless shift is held. Otherwise it starts a selection, a block selection when alt or ctrl is held.
- **Painting.** `prepaint` locks the `Term`, walks `renderable_content()` once and batches cells into same-style runs (`RunStyle`), which already carry an optional `UnderlineStyle`. GPUI re-renders the whole window on any notify, so `prepaint` must stay cheap.
- **OSC 8 in alacritty.** `alacritty_terminal` 0.26 parses OSC 8 and stores a `Hyperlink { id, uri }` on each cell it covers (`cell.hyperlink()`). A link without an explicit id gets a unique generated id, so two cells hold equal `Hyperlink`s exactly when they come from the same link.
- **Wrapping.** alacritty marks the last cell of a row that wrapped into the next row with `Flags::WRAPLINE`. Wide characters leave a `WIDE_CHAR_SPACER` cell after them.
- **tmux control mode.** Verified against tmux 3.6 with a raw `tmux -C` client: `%output` carries the bytes the program wrote, OSC 8 included (`\033]8;id=x;https://example.com/a\033\134LINK…`), so pane terminals parse OSC 8 themselves. `capture-pane -p -e`, which Brindle uses to restore panes on attach, re-emits OSC 8 around linked text, so links survive attaching. Restore does not use `-J`, so rows that were wrapped come back as separate lines.
- **tmux pane focus.** Clicking a pane calls `TerminalView::mouse_down`, which focuses the view. `TmuxWindowView`'s focus-in subscription then selects the pane in tmux. A ctrl-click on a link in an inactive pane therefore selects the pane and opens the link through the same code path.
- **Modifier events.** GPUI delivers `ModifiersChangedEvent` along the focus path only (`window.on_modifiers_changed`, registered during paint).

## Goals / Non-Goals

**Goals:**
- No new dependencies. Detection is a small hand-written scanner over one logical line.
- No per-frame grid scans. Link lookup runs in event handlers, only while ctrl is held over a terminal or when `copy_link` runs. `prepaint` only reads the already-computed spans.
- The same code path for local tabs and tmux panes.

**Non-Goals:**
- Regex-configurable hints, keyboard link hints, URL tooltips.
- Underlining OSC 8 text when ctrl isn't held.
- Joining wrapped rows in snapshots that tmux restores on attach.

## Decisions

### 1. A pure scanner over a logical line

`src/terminal/links.rs` holds `find_url(text: &[char], at: usize) -> Option<Range<usize>>`. It finds the URL covering index `at`, if any:

1. **Candidate run.** Expand left and right from `at` over URL characters. The run stops at whitespace, control characters, `<>"`` ` ``{}|\^`, and box-drawing or block characters (U+2500–U+259F), which TUIs draw around text. Non-ASCII letters are allowed (IRIs).
2. **Scheme.** Inside the run, find the last scheme start at or before `at`. A scheme start is a known scheme name, matched case-insensitively, followed by `://`, or by `:` for `mailto`, `news` and `magnet`. It is preceded by the start of the run or by a character that is not alphanumeric or one of `+-.`. Taking the last such start lets `https://a.com/https://b.com` resolve sensibly.
3. **Trimming.** Repeatedly drop the trailing character while it is one of `.,:;!?'*`, or a `)`/`]` that has no matching opener inside the URL. A URL that starts right after `(` or `[` therefore loses its unmatched closing bracket, and `Rust_(lang)` keeps its own. `"` and `>` never get in, because they end the run.
4. **Validity.** At least one character must follow the scheme separator after trimming, and `at` must still be inside the result.

**Scheme list:** `http`, `https`, `ftp`, `file`, `ssh`, `git`, `gemini`, `gopher`, `ipfs`, `ipns` (with `://`) and `mailto`, `news`, `magnet` (with `:`). These are the schemes Alacritty's default hint recognises. Bare `www.example.com` is not detected. *Rationale:* a scheme requirement keeps false positives (file paths, version strings) out.

### 2. Logical lines with a bounded join

`Terminal::link_at(point)` builds the logical line around a grid point:
- It walks up while the previous row's last cell has `WRAPLINE`, and down while the current row's has.
- It is bounded to 32 rows on each side, so the join is at most 65 rows (about 13,000 cells at 200 columns).
- It skips `WIDE_CHAR_SPACER` and `LEADING_WIDE_CHAR_SPACER` cells, and keeps a parallel `Vec<GridPoint>` so a char range maps back to cells.

*Rationale:* URLs longer than 32 wrapped rows don't occur in practice, and the bound keeps a pathological wrapped line from stalling the UI thread.

The result is a `Link { uri: String, spans: Vec<(Line, Column, Column)> }`, with inclusive column ranges per row in **grid coordinates** (absolute lines, independent of the scroll position), so a stored hover stays attached to its text when the view scrolls.

### 3. OSC 8 first, all visible cells of the hyperlink

`link_at` first checks the cell's `hyperlink()`. If it has one, the link's spans are every cell in the **visible viewport** with an equal `Hyperlink`, and the URI is used verbatim. This matches how programs use ids to join a link broken across rows or boxes.

The scan covers only the viewport (rows × cols cells), runs once per hover change, and never during painting.

**OSC 8 schemes:** any URI with a syntactically valid scheme (`ALPHA *( ALPHA / DIGIT / "+" / "-" / "." ) ":"`) and no control characters is opened. Detected URLs are limited to the list above because detection guesses, while an OSC 8 URI is what the program asked for: `vscode://`, `man:` or custom app schemes are legitimate there.

*Trade-off:* a program can make Brindle open any URI on a ctrl-click. This is the same trust model as kitty, WezTerm and Alacritty, and it always takes a deliberate ctrl-click.

Requiring a scheme also means the argument can never start with `-`, so it can't be taken as an `xdg-open` option.

### 4. Hover state lives in `TerminalView`, computed in handlers

`TerminalView` gains:
- `pointer: Option<Point<Pixels>>`, the last position over this terminal;
- `hovered_link: Option<Link>`.

`update_hover(modifiers)` recomputes the link when ctrl is held, alt is not, and the pointer is over the grid. Otherwise it clears it, and it notifies only when the result changes. It runs on:
- mouse move, when the pointer's cell or the modifiers change;
- `ModifiersChangedEvent`;
- terminal output (the existing `observe(&terminal)` callback), only while a link is hovered, so the underline follows text that changes underneath it;
- wheel scroll.

Hover is cleared when:
- the pointer leaves the element: the move listener also fires for a view with an active hover, so it can clear itself;
- the window is deactivated: a ctrl release in another window would otherwise leave a stuck underline.

*Rationale:* the link work happens only while ctrl is held over a terminal. Plain mouse motion without ctrl only compares a modifier flag.

**Prepaint** maps each span to viewport rows (`line + display_offset`). For cells inside a span it forces a single underline in the cell's foreground color, and sets `Frame::pointing_hand`. Paint uses `CursorStyle::PointingHand` over the hitbox in that case, ahead of the arrow and I-beam choice.

**Modifiers in tmux panes:** `ModifiersChangedEvent` reaches only the focused terminal's listener. `TmuxWindowView`'s root, which is always on the focus path while one of its panes has focus, also listens. It passes each change, with `window.mouse_position()`, to every pane view, and each pane shows or clears feedback depending on whether the pointer is over its grid. So pressing or releasing ctrl over an inactive pane updates it at once.

**No feedback while dragging:** while a selection drag or a reported drag is in progress, `hover` clears instead of looking links up. A ctrl block selection therefore doesn't flash underlines or switch to a pointing hand as it sweeps over URLs.

### 5. Click rule: ctrl takes a press on a link, open on release

`mouse_down` checks in this order:
1. **Link.** If the left button is pressed with ctrl and without alt, and `link_at` finds a link under the pointer (computed fresh, not taken from the hover), the press is consumed and stored as `pending_link`. Nothing is reported and no selection starts.
2. **Reporting.** Otherwise `reporting()` applies (mouse mode, shift bypass).
3. **Selection.** Otherwise selection proceeds as today.

`mouse_up` with a `pending_link` opens it if the link under the pointer has the same URI, and nothing else happens: no release report, no selection. `is_dragging()` includes `pending_link`, so the global up listener delivers the release even off the element.

**Decisions recorded:**
- **Ctrl overrides mouse reporting on links**, mirroring how shift bypasses it for selection. Ctrl-click on non-link text in a mouse-mode program is still reported with the ctrl bit. Programs that bind ctrl-click on link text lose that click, which is the trade every terminal with ctrl-click links makes.
- **Shift is allowed**: ctrl-shift-click opens too. Alt is excluded, so alt-drag stays a block selection everywhere.
- **Open on release, over the same link**, so a press that turns into a drag off the link is a cancel. This is the usual button semantics.
- **Ctrl-drag from a link doesn't block-select.** Users who want a block starting there can use alt-drag, which the spec already offers.

### 6. Opening: `xdg-open` on a background thread, overridable by env var

`links::open(uri)` reads `BRINDLE_LINK_OPENER` (default `xdg-open`) and spawns a thread. The thread runs the program with the URI as its only argument, stdin, stdout and stderr nulled and `process_group(0)`, then `wait()`s to reap it. Failures are logged with `log::warn!`.

*Rationale:*
- The UI thread never blocks on fork/exec or on a slow opener.
- No zombie processes are left behind.
- The opener doesn't share Brindle's process group.

**Override via env var, not config.** Tests need to replace the opener without a browser opening. An environment variable needs no config schema, settings dialog or reload plumbing, and the same mechanism is how `BRINDLE_CONFIG` and `BRINDLE_E2E_*` work. It is also usable by people who want a specific browser. A config key can be added later if users ask for one.

### 7. `copy_link` recomputes on demand

`CopyLink` (`copy_link`, label "Copy Link Under Mouse") is handled in the `Terminal` context. It computes `link_at` at the view's last pointer position (`pointer`), falling back to `window.mouse_position()`, without requiring ctrl. When the palette runs it, the pointer is wherever the user left it over the terminal, so a stored hover isn't needed. It has no default key: ctrl-shift-c is copy, and there's no conventional key for this.

### 8. Debug hooks: `--hover` and `--click`

`Step::Hover(pos)` and `Step::Click(pos)` parse `[mods-]COL,ROW`, for example `ctrl-3,0` or `ctrl-shift-10,2`. A position that doesn't parse is logged and skipped.

`Workspace::debug_mouse` resolves the active `TerminalView`: the local tab's view, or `TmuxWindowView::active_view()`. It computes the cell's centre from the view's `GridLayout`, and builds GPUI events with those modifiers:
- **hover:** a `MouseMoveEvent`, calling `mouse_move`;
- **click:** move, then a `MouseDownEvent` (`click_count: 1`), then a `MouseUpEvent`, all through the same handlers that real input uses.

No OS-level input is generated. The dump prints `--- link <uri>` after the grid line while a link is hovered.

## Risks / Trade-offs

- **Wrapped URLs in restored tmux snapshots aren't joined.** Joining them would mean `capture-pane -J`, which changes how restore replays trailing spaces. Live output wraps correctly, because the pane terminal wraps it itself.
- **Classic tmux (tmux inside a local tab):**
  - tmux re-emits OSC 8 to the outer terminal only when it knows the outer terminal supports hyperlinks (`set -as terminal-features ',xterm-256color:hyperlinks'`).
  - Wrapped URLs are joined only if tmux redraws them by writing past the margin; tmux often positions the cursor explicitly instead, which breaks the wrap flag.
  - Detection of single-row URLs always works.
- **False positives/negatives:**
  - URLs containing `'` near the end lose the trailing quote.
  - URLs ending in `.` or `:` are trimmed.
  - Both are deliberate: sentence punctuation is far more common.
- **Ctrl-press on a link is swallowed** in mouse-reporting programs. This is documented as the rule above.
