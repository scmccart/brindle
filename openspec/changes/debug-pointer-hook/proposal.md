# Proposal

**Status: proposal stage, blocked on GPUI.** This change has no design, specs or tasks yet. It was designed and partly built inside `agents-markdown` on 2026-11-25, then dropped at the user's request when the blocker below turned up. Take it further after a GPUI upgrade that makes the blocker go away, or once the user accepts another delivery path.

## Why

The debug pointer options (`--hover`, `--click`, `--right-click`, `--press`, `--release`) take a terminal cell (`COL,ROW`) and call `TerminalView`'s own mouse handlers. Nothing else in the window can be pressed from a run. That covers agent-zone buttons (permission options, stop, the queue row's buttons, a thought's trigger, a message's or code block's copy button, links), dialog buttons and backdrops, the settings dialog's list and the profile menu's split buttons. So every pointer check on those stays on `CLAUDE.md`'s manual list. The e2e suite can't post a press on Linux without synthetic input on the desktop, which is never used.

## What Changes (as designed)

- The pointer options, and a new `--double-click`, also take `[MODS-]@NAME[+FX,FY]`. This names an element painted in the last frame and presses it at fractions of its bounds (the centre by default). Names come from what the dump already numbers. They form a closed `HitName` enum: `tab:N`, `profile-menu`, `menu-item:N`, `zone:ID`, `zone-header:ID`, `zone-split-right:ID`, `zone-split-down:ID`; in agent zones `entry:N`, `copy-message:N`, `copy-code:N.P.K`, `link:N.P.K`, `thought:N`, `permission:N:OPTION`, `stop`, `prompt`, `queued:remove|send-again|discard` and `scroll-to-bottom`, each optionally qualified as `ZONE/NAME`; and `confirm:cancel|close|open`, `backdrop`, `dialog-close`, `settings-section:NAME`, `settings-row:NAME`, `settings-edit` and `settings-duplicate`.
- `brindle-ui` gains a window-keyed hit-target registry (`primitives/hit_targets.rs`, enabled only for runs with an `@` step). `.hit_target(name)` records an element's bounds in prepaint. The dump adds `--- hit` lines and lines for steps that missed.
- Presses are delivered in process, through GPUI's real dispatch and hit-testing, as a pure plan: frame, modifiers, move, frame, down, frame, up, frame. A frame must be drawn between the down and the up, because `InteractiveText` registers its mouse-up listener only in the frame after the down.
- The e2e harness gains `Run::click_element` and related helpers. The agent-zone, dialog, settings and profile-menu pointer checks become cases and leave the manual list.

## The blocker

In gpui 0.2.2, `Window::dispatch_event(PlatformInput, &mut App)` is `pub`, but it returns `DispatchEventResult`, which is `pub(crate)` (`window.rs:1272`). Rust's type privacy rejects any call to it from outside gpui ("type is private"). No other public path reaches the mouse listeners. The alternatives considered on 2026-11-25 were:
- patching gpui with a one-word `pub` through `[patch.crates-io]`, which changes `Cargo.lock`'s gpui source;
- sending X11 `SendEvent` (and `PostMessage` on Windows) to Brindle's own window;
- calling each element's app-side handler directly, which can't reach links and doesn't prove hit-testing;
- a transmute, rejected.

The user chose to drop the hook for now.

## Work kept

The partial implementation is saved outside the repository as `~/.cache/claude-tasks/brindle/agents-markdown/section-5a-wip.patch`. It applies cleanly to `feat/agents-markdown` at `1d90bb6`. It contains the pure `src/pointer_hook.rs` with tests, the CLI parsing, the `brindle-ui` registry with passing tests, the element names and the step runner. Only the dispatch call fails to compile. The full design (decision 16 and its tasks) is in commit `1d90bb6`'s `agents-markdown` design.md and tasks.md, reverted in the next commit. The design report is `~/.cache/claude-tasks/brindle/agents-markdown/pointer-hook-design-report.md`.

Open questions, to answer before a design:
1. **Delivery:** a GPUI upgrade (check whether a newer gpui exposes `DispatchEventResult` or another public dispatch), a patched gpui, or own-window platform events.
2. **The terminal's cell form:** whether `COL,ROW` steps move onto the same dispatch path.

## Capabilities

### Modified Capabilities
- `command-line`: the pointer options' element form and `--double-click`.
- `ui-components`: the hit-target registry.

## Impact

`src/main.rs` (step parsing and the runner), a new pure `src/pointer_hook.rs`, `ui/src/primitives/hit_targets.rs` with hit targets on components, `tests/e2e/` (harness helpers and backfilled cases), `CLAUDE.md`'s manual list and "Verifying GUI behaviour", and possibly `Cargo.lock` (depending on delivery).
