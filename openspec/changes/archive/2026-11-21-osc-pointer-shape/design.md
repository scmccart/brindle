# Design

## Context

`TerminalElement::paint` picks the pointer from two frame flags: `pointing_hand` (a ctrl-hovered link) and `arrow_cursor` (mouse reporting), else `CursorStyle::IBeam`. It sets it with `window.set_cursor_style(…, &frame.hitbox)`, so it applies only over the grid. GPUI 0.2.2's `CursorStyle` offers Arrow, IBeam, Crosshair, ClosedHand, OpenHand, PointingHand, the eight directional and two-way resizes, ResizeColumn, ResizeRow, IBeamCursorForVerticalLayout, OperationNotAllowed, DragLink, DragCopy, ContextualMenu and None. There is no wait, progress, help, move, cell or zoom, which is why the spec's supported list is what it is. Alacritty drops OSC 22. The client's `Sink::write` (`src/terminal/server.rs`) scans the whole chunk with `osc::Scanner::scan` before parsing it, so a find doesn't know which screen was active when it arrived. The server core's `Emulator` already uses `scan_positions` to apply finds in stream order.

kitty's reference: <https://sw.kovidgoyal.net/kitty/pointer-shapes/>. The brief for this design came from a summary of it, so the implementer checked it against kitty's source (`set_pointer_shape` in `kitty/window.py`, `change_pointer_shape` and `current_pointer_shape` in `kitty/screen.c`), which differs from the summary in four ways, and the spec now follows the source:

- A set (`=` or no first character) applies each name in turn, each replacing the top (or pushing onto an empty stack), so the last name kitty knows wins. It is not a fallback list read from the front.
- A push (`>`) pushes every name kitty knows, the last on top.
- An unknown name, whatever its characters, is skipped (kitty logs it); the rest of the list still applies. A list with no known name changes nothing. Brindle treats the names it can't show (wait, progress, help, cell, move, zoom-in, zoom-out and the four diagonal one-way resizes) as unknown, which keeps "accepted" and "answered `1` by a query" the same set.
- An empty body is a set of the empty name, which replaces the top with "no shape" (kitty's `INVALID_POINTER`, Brindle's own pointer here); it doesn't empty the stack, so a later pop shows the entry below. An empty name inside a set list does the same.

Queries answer each listed item in turn, so a list may mix shape names and the special names. kitty also accepts X11 cursor aliases (`hand`, `xterm`, `left_ptr`, …) in sets and pushes; Brindle takes the CSS names only, the set kitty's documentation defines.

## Goals / Non-Goals

**Goals:**
- Stack rules, name mapping and reply texts as pure code with unit tests.
- No cost on the output hot path for chunks without OSC 22.

**Non-Goals:**
- Server-side tracking and replay. The pointer is transient: programs that use it set it on every hover change, and a query after attach is answered from the client's empty stacks, consistently. The spec states this.
- Pointer shapes over chrome.

## Decisions

### Pure model: `term-core/src/pointer.rs`

```rust
pub enum Shape { Default, Text, VerticalText, Pointer, Crosshair, Grab, Grabbing, NotAllowed, NoDrop,
                 Alias, Copy, ContextMenu, EwResize, NsResize, EResize, WResize, NResize, SResize,
                 NwseResize, NeswResize, ColResize, RowResize }
pub enum Op { Set(Option<Shape>), Push(Vec<Shape>), Pop, Query(Vec<Ask>) } // None: Brindle's own
pub enum Ask { Supported(bool), Current, Default, Grabbed }
pub enum Screen { Primary, Alternate }
pub fn parse(body: &[u8]) -> Option<Op>;               // None: nothing to do (no known name)
pub struct Stacks { primary: Stack, alternate: Stack } // Stack: Vec<Option<Shape>>, cap 16
impl Stacks { pub fn apply(&mut self, screen: Screen, op: Op) -> Option<String> /* a reply */;
              pub fn top(&self, screen: Screen) -> Option<Shape>; pub fn clear(&mut self); }
```

`Shape` is closed, and the names map both ways through one table, so `parse`, the support answers and the `__current__` reply can't disagree (`types R1`). `parse` resolves a set list to its last known entry (applying sets in turn leaves only the last one on top) and a push list to its known names, at most the last 16 (more would only push the earlier ones off the bottom). A list with no known name parses to `None`, so it yields no find. A stack entry is `Option<Shape>`, `None` being the "no shape" an empty set leaves, which reads as Brindle's own. The current shape is resolved inside `apply`, which returns the whole reply sequence, so callers never build replies. The crate stays free of GPUI: the mapping to `gpui::CursorStyle` is a `match` in `src/terminal/pointer.rs`, exhaustive both ways (`railway R3`). The module joins `PURE_MODULES`.

### The screen travels with the find

`Sink::write` switches to `scan_positions`. When a chunk holds no `Found::Pointer` (the common case) it parses as today. When it holds one, it takes the parser and term locks once and walks the finds in order: each find is sent where it sits in the stream, and at a pointer find the parser is first advanced up to the find's offset, `TermMode::ALT_SCREEN` read, and `ChannelEvent::Pointer(screen, op)` sent. That's the same splitting `Emulator::feed` does. Every find, pointer or not, keeps its stream order (a push then `ESC c` in one chunk must leave the stacks empty), and only chunks with a pointer find interleave finds with the parse. `Found::Pointer` itself is never queued (the gate classes it `Ignored`); the tagged event is the only way an op reaches `Terminal`. A pointer find in the bytes of a held replay is dropped: the server keeps no pointer, so a replay holds none, and the screen it would act on isn't parsed yet. Alternative considered: reading the screen on the GPUI thread when the event is applied. Rejected: by then the program may have left the alternate screen, which is exactly the case the spec's scenario tests.

Pointer ops are side events (`EVENT_MARKS` drops them under overload). Losing one only leaves the pointer stale until the next op, and the overload reset clears the stacks with the records.

### Shell and paint

`Terminal` keeps `pointer: Stacks`. Applying `ChannelEvent::Pointer` applies the op and writes any reply with `write` (which sends nothing for read-only `Remote` terminals). The event task already notifies once per chunk of events, so no extra `cx.notify()` is needed. Process exit (and every other end of the terminal), `Found::Reset`, a replay's start and the overload reset clear the stacks. The paint-time choice becomes a pure function, `pointer::choose(link_hover, selecting, program: Option<Shape>, mouse_mode) -> CursorStyle`, tested by the precedence scenarios, and `prepaint` stores its result in the frame. `prepaint` reads the top of the screen it is drawing (from the mode it reads under the term lock it already holds), a field read.

### Testing

- `pointer.rs` unit tests: every spec scenario (push/pop/set/reset, cap 16, per-screen stacks, fallback lists, names with other characters, every query form byte-for-byte), and a test that every `Shape` round-trips through its name.
- `src/terminal/pointer.rs` unit test: the `Shape → CursorStyle` table, and `choose`'s precedence.
- `server.rs` unit test: a chunk that switches to the alternate screen, pushes, and switches back tags its find `Alternate`.
- e2e (`tests/e2e/cases/pointer.rs`): `pointer-query` and `pointer-current` (replies read back by the shell); `pointer-dump`, through a `--- pointer shape=NAME screen=primary|alternate` line in `--dump-screen-after`. GPUI's pointer can't be captured, so the dump line is what the cases check.
- Manual check: with a real mouse, a program that sets `ew-resize` shows it, and ctrl over a link still shows the hand.

## Risks / Trade-offs

- [kitty's exact semantics for multi-name pushes differ from the spec] → checked against kitty's source; the spec follows it (see Context).
- [A program floods OSC 22 while the mouse moves] → each op is a small event; `cx.notify()` only on a change of the visible shape.
