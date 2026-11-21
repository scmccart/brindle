# Proposal

## Why

Brindle picks the mouse pointer itself: a text beam, an arrow while a program has mouse reporting on, and a hand over a ctrl-hovered link. Full-screen programs that use the mouse can't say what a spot does. A split border in an editor or multiplexer can't show a resize pointer, a button can't show a hand, and a busy program can't show that it's busy. OSC 22, from xterm and extended by kitty with a stack and queries, lets them; kitty, foot, Ghostty and WezTerm support it.

## What Changes

- Recognise `ESC ] 22 ; [=|>|<|?] <names> ST`: set, push, pop, reset (empty) and query, with a stack of at least 16 entries per screen (primary and alternate).
- Map the CSS pointer names to the platform pointers GPUI offers. A list of names falls back to the first one Brindle can show, and names it can't show count as unsupported in query replies.
- Show the program's pointer over that terminal's grid, except while a link is ctrl-hovered (the hand) or a selection is being dragged (the text beam).
- Answer queries (`?__current__`, `?__default__`, `?__grabbed__`, `?name,…`) on the PTY of the terminal that asked.
- Empty both stacks on a full reset and when the process exits. After an attach or a reconnect the pointer is Brindle's own until the program sets one again.

Out of scope: pointer shapes over Brindle's chrome (tab strip, headers), custom pointer images, and keeping the stack in the terminal server.

## Capabilities

### New Capabilities
<!-- none -->

### Modified Capabilities
- `terminal-emulation`: adds the OSC 22 pointer shape requirements.

## Impact

- `term-core/src/osc.rs` (`22` joins the wanted numbers, `Found::Pointer`) and a new pure `term-core/src/pointer.rs` (names, stacks, replies). The server core's `Emulator` ignores it.
- `src/terminal/server.rs` (tag each pointer find with the screen it arrived on), `src/terminal/mod.rs` (the stacks, replies), `src/terminal_element.rs` (the pointer choice moves into a pure function), a mapping to `gpui::CursorStyle` in `src/terminal/`.
- `tests/e2e/cases/` (query replies, dump), `docs/code-map.md`, README.md. No wire change, no new dependency.
