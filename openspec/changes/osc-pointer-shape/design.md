# Design

## Context

`TerminalElement::paint` picks the pointer from two frame flags: `pointing_hand` (a ctrl-hovered link) and `arrow_cursor` (mouse reporting), else `CursorStyle::IBeam`. It sets it with `window.set_cursor_style(…, &frame.hitbox)`, so it applies only over the grid. GPUI 0.2.2's `CursorStyle` offers Arrow, IBeam, Crosshair, ClosedHand, OpenHand, PointingHand, the eight directional and two-way resizes, ResizeColumn, ResizeRow, IBeamCursorForVerticalLayout, OperationNotAllowed, DragLink, DragCopy, ContextualMenu and None. There is no wait, progress, help, move, cell or zoom, which is why the spec's supported list is what it is. Alacritty drops OSC 22. The client's `Sink::write` (`src/terminal/server.rs`) scans the whole chunk with `osc::Scanner::scan` before parsing it, so a find doesn't know which screen was active when it arrived. The server core's `Emulator` already uses `scan_positions` to apply finds in stream order.

kitty's reference: <https://sw.kovidgoyal.net/kitty/pointer-shapes/>. The brief for this design came from a summary of it, so the implementer confirms the set-vs-push semantics of a multi-name list and the empty-body reset against kitty's source, and adjusts the spec in this change if they differ.

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
pub enum Op { Set(Shape), Push(Shape), Pop, Reset, Query(Query) }
pub enum Query { Current, Default, Grabbed, Support(Vec<bool>) }
pub enum Screen { Primary, Alternate }
pub fn parse(body: &[u8]) -> Option<Op>;               // name lists resolved to their first Shape here
pub struct Stacks { primary: Stack, alternate: Stack } // Stack: ArrayDeque-like Vec, cap 16
impl Stacks { pub fn apply(&mut self, screen: Screen, op: Op) -> Option<String> /* a reply */;
              pub fn top(&self, screen: Screen) -> Option<Shape>; pub fn clear(&mut self); }
```

`Shape` is closed, and the names map both ways through one table, so `parse`, `Support` and the `__current__` reply can't disagree (`types R1`). An unsupported name in a set or push list falls through to the next. An all-unsupported list resolves to `Shape::Default`. `Support` and `Query::Current` are resolved inside `apply`, which returns the reply text, so callers never build replies. The crate stays free of GPUI: the mapping to `gpui::CursorStyle` is a `match` in `src/terminal/pointer.rs`, exhaustive both ways (`railway R3`). The module joins `PURE_MODULES`.

### The screen travels with the find

`Sink::write` switches to `scan_positions`. When a chunk holds no `Found::Pointer` (the common case) it parses as today. When it holds one, it advances the parser up to that find's offset, reads `TermMode::ALT_SCREEN` under the term lock it already holds, and sends `ChannelEvent::Osc(Found::Pointer(op))` tagged with that `Screen`. That's the same splitting `Emulator::feed` does. The other finds keep their current path, so the order of events doesn't change. Alternative considered: reading the screen on the GPUI thread when the event is applied. Rejected: by then the program may have left the alternate screen, which is exactly the case the spec's scenario tests.

Pointer ops are side events (`EVENT_MARKS` drops them under overload). Losing one only leaves the pointer stale until the next op, and the overload reset clears the stacks with the records.

### Shell and paint

`Terminal` keeps `pointer: Stacks`. `handle_found` applies the op, writes any reply with `write` (dropped for read-only `Remote` terminals by `reply()`), and calls `cx.notify()` only when the active screen's top changed. Process exit, `Found::Reset` and `REPLAY_START` clear the stacks. The paint-time choice becomes a pure function, `pointer::choose(link_hover, selecting, program: Option<Shape>, mouse_mode) -> CursorShape`, tested by the precedence scenarios, and `prepaint` stores its result in the frame. The frame reads the top in `prepaint`, a field read with no lock.

### Testing

- `pointer.rs` unit tests: every spec scenario (push/pop/set/reset, cap 16, per-screen stacks, fallback lists, invalid characters, every query form byte-for-byte), and a test that every `Shape` round-trips through its name.
- `src/terminal/pointer.rs` unit test: the `Shape → CursorStyle` table, and `choose`'s precedence.
- `server.rs` unit test: a chunk that switches to the alternate screen, pushes, and switches back tags its find `Alternate`.
- e2e (`tests/e2e/cases/pointer.rs`): `pointer-query` and `pointer-current` (replies read back by the shell); `pointer-dump`, through a `--- pointer shape=NAME screen=primary|alternate` line in `--dump-screen-after`. GPUI's pointer can't be captured, so the dump line is what the cases check.
- Manual check: with a real mouse, a program that sets `ew-resize` shows it, and ctrl over a link still shows the hand.

## Risks / Trade-offs

- [kitty's exact semantics for multi-name pushes differ from the spec] → checked first while implementing, see Context.
- [A program floods OSC 22 while the mouse moves] → each op is a small event; `cx.notify()` only on a change of the visible shape.
