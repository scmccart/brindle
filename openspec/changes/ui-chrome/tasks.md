# Tasks

## 1. brindle-ui: icons, sizes, Separator, Kbd

- [x] 1.1 Add `ButtonSize::IconSm` (2rem) and `IconXs` (1.5rem) to `ui/src/style.rs` with the icon scaled to fit; verify `button_style` unit tests pin both sizes and the existing sizes are unchanged (`cargo test -p brindle-ui`)
- [x] 1.2 Add `Icon::{Plus, Minus, Square, Copy, AppWindow, ArrowUp, ArrowDown, Columns2, Rows2}` from lucide's paths (with a `Stroke::Rect` for the rectangles) in `ui/src/icon.rs`; verify each icon's strokes stay inside the unit square in a unit test
- [x] 1.3 Add `Separator` (horizontal and vertical) and `Kbd` (keycaps from a keymap string, a gap between strokes) in `ui/src/components`; verify a unit test of the key splitting (`ctrl-shift-t`, a two-stroke `ctrl-k ctrl-c`, a lone `enter`) passes
- [x] 1.4 Show the new sizes, icons, `Separator` and `Kbd` in the gallery, light and dark, and extend `-- --check` for the sizes; verify `scripts/with-xvfb cargo run -p brindle-ui --example gallery -- --check` prints `ok` for them
- [x] 1.5 Document the components in `ui/README.md`; verify the component list there names each new one

## 2. brindle-ui: Tooltip and Toggle

- [x] 2.1 Add `Tooltip` (label, optional shortcut `Kbd`, shadcn's inverted look) and a `.tooltip_for(label, shortcut)` extension over GPUI's `tooltip`, building text only when shown; verify a unit test of the style (colours from tokens) and that the extension compiles on a `Button` in the gallery
- [x] 2.2 Add `Toggle` (pressed state, default/outline variants, default/sm/lg sizes, activation as `Button`, no focus without a handle) reusing `interaction`/`activates`; verify unit tests of `toggle_style` (pressed uses accent) and the disabled and no-focus cases
- [x] 2.3 Show them in the gallery and add `-- --check` cases for Toggle (click and space flip it, disabled does not) and Tooltip (hover cases run on Windows only and are `SKIP` elsewhere, as for pointer checks); verify `scripts/with-xvfb cargo run -p brindle-ui --example gallery -- --check` has no `FAIL`
- [x] 2.4 Document both in `ui/README.md`, listing the differences from shadcn (no delay group, 500 ms hover delay from GPUI, no arrow); verify the Differences list names them

## 3. brindle-ui: menus (Menu, ContextMenu, DropdownMenu)

- [x] 3.1 Add the `Menu` view (`MenuEntry::{Item, Separator}`, items with label, optional description line, shortcut, disabled), its style, and `MenuEvent::Selected(index)`; verify unit tests of navigation (no wrap, skips disabled and separators, home/end, typeahead) built on `primitives::nav`
- [x] 3.2 Host it as an `Overlay` placed with `Anchored` against a zero-size rectangle at a point (context menu) or the trigger's bounds (dropdown), blocking the outside pointer, with a maximum height that scrolls; verify `placement` unit tests that a zero-size trigger near the right and bottom edges flips and clamps, and that a menu taller than the window is limited to it
- [x] 3.3 Implement `ContextMenu` (secondary press opens), `DropdownMenu` (primary press opens, a second press closes, enter/space/down open it from a focused trigger), selection (enter, space, click), the `Select` pointer gesture (opening release ignored within 10px, drag-release selects), Escape and outside-press dismissal, and focus return; verify `-- --check` cases for each scenario of the "Context menus", "Context menu keys and selection", "Context menu dismissal" and "Dropdown menus" requirements (pointer-only ones `SKIP` off Windows)
- [x] 3.4 Show both menus in the gallery, compare the pointer gesture and keys against Radix's menu source and document every difference in `ui/README.md`; verify the gallery builds and the README lists them

## 4. brindle-ui: Tabs

- [x] 4.1 Add `TabList` and `TabTrigger` (active state, `Default`, `Line` and `Attached` variants, per-trigger `surface` and `text_color` overrides, disabled, a close slot, adornment slots, min/max width with ellipsis) with token styles in `ui/src/style.rs`; verify unit tests of `tab_style` (attached active uses the surface override else the background token, line uses primary, hover uses accent) and the width clamp
- [x] 4.2 Implement the roving focus model (one focus handle on the list, arrows and home/end move focus and ask the owner to activate, wrap, skip disabled), activation on press without taking focus, middle-click close, close-slot press not activating, and drag-to-reorder with a drop bar; verify unit tests of the index navigation and `-- --check` cases for the "Tab lists", "Tab trigger additions" scenarios (pointer-only ones `SKIP` off Windows)
- [x] 4.3 Show tabs in the gallery in both variants and list in `ui/README.md` every difference from Radix (closable/reorderable triggers, middle-click, adornments, widths, no focus on press) with its reason; verify the gallery builds and the README lists them

## 5. Find bar

- [x] 5.1 Rebuild `render_find_bar` on `Input`, two `Toggle`s, ghost `IconXs` `Button`s (previous, next, close) and a `Separator`, in a popover-styled panel with token colours and tooltips (name and the bound key through the keymap); keep `occlude`, the `FindBar` key context, the click-to-focus handler and `describe_find`; verify `cargo build` is warning-free and `cargo test --workspace` passes
- [x] 5.2 Show the invalid expression as the `Input`'s invalid state plus a destructive message, and the count in the muted foreground; verify the existing `scrollback-search` e2e cases still pass (`scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- search`) and a case checks the destructive border colour by capture
- [x] 5.3 Remove the find bar's use of `Theme::accent`, `chrome_background`, `chrome_border`, `muted_foreground`; verify `grep` finds none in `src/find_bar.rs`

## 6. Tab strip

- [x] 6.1 Rebuild the strip on `TabList`/`TabTrigger` (`Attached` variant, the active trigger given the active terminal's background and foreground; the host's `on_activate` focuses the terminal when `by == Pointer`; titles truncate through the component's `trigger_label`, since gpui 0.2.2's `text_ellipsis` on a flex item cuts nothing): titles, tmux marker, bell and status dots as adornments, close slot with a tooltip, drag reorder through `drop_tab`, middle-click close through `close_tab_at`, `DraggedTab` and the window border from `cx.ui_theme()`, status dots in the global theme's ANSI colours; verify an e2e capture case with a profile theme differing from the global one samples the strip in token colours and the active tab in the tab's terminal background
- [x] 6.2 Make a press activate the tab and leave focus in its terminal (the host's activation focuses the terminal, never the list); add the hidden `FocusTabStrip` action (focus the list, Enter/Escape return focus to the terminal); verify e2e cases that run `--action focus_tab_strip` then `--key right`/`--key home`/`--key escape` and check the active tab and `--- focus` in each step
- [x] 6.3 Replace the new-tab and chevron buttons with ghost `IconXs` `Button`s in press-stopping wrappers (a `Button` without a focus handle still lets a press bubble to a focus-tracking ancestor, so the wrapper stops propagation and uses `inert_press`, as `Toggle` does), with tooltips ("New tab", "New tab from profile…" plus shortcuts); keep the caption buttons hand-built with no handlers on Windows, coloured from `button_style` and drawing the `Minus`, `Square`/`Copy` and `X` icons instead of text glyphs, and replace the tmux tab marker `⧉` with `AppWindow`; verify `cargo test --workspace` passes and the `win` caption hit-test case still passes on Windows
- [x] 6.4 Remove the strip's uses of the four chrome helpers; verify `grep` finds none in the strip code in `src/workspace.rs`

## 7. Profile menu

- [x] 7.1 Make `⌄` a `DropdownMenu` trigger listing the profiles in configuration order with the picker's description line and ctrl-alt-N shortcuts for the first nine, choosing an item through the picker's `open_profile` path (session shown here or by another Brindle included); add the hidden `OpenProfileMenu` action; verify unit tests of the item builder (names, shortcuts, more than nine profiles) and an e2e case that opens the menu with `--action open_profile_menu`, picks with `--key down`/`--key enter` and checks the new tab's profile
- [x] 7.2 Keep ctrl-shift-space, `open_profile_picker` and the palette unchanged; verify the existing `profiles`/picker e2e cases pass and a case checks the menu and the palette are never open together
- [x] 7.3 Update README.md (`⌄` row of the keybinding table, the removed-key note near the palette text, `focus_tab_strip`, the profile menu) and CLAUDE.md's manual-check list (the `⌄` check becomes "profile menu opens and toggles; ctrl-shift-space still toggles the palette"); verify the lines are present

## 8. Context menus on tabs and zone headers

- [x] 8.1 Add hidden actions `OpenTabMenu` and `OpenZoneMenu` to the `ACTIONS` table (config names, no palette label, no default key); verify `--list-actions` lists them and the actions test passes
- [x] 8.2 Add the `context_menu` overlay slot to `Workspace`, a right-press hook on `TabTrigger` (it has none yet; a right press now only bubbles) and the handler on it (activate, open at the pointer, stop propagation so the window menu doesn't open), the local and tmux item lists, item dispatch through `window.dispatch_action` after refocusing the tab, and Move Left/Right disabled at the ends; verify unit tests of the item builder (local, tmux, first and last tab)
- [x] 8.3 Add the zone header right-press handler and menu (Split Right/Down disabled via `can_split_zone` for the axis, Close Zone); verify a unit test of the item builder and e2e cases of the menu's items acting on the zone it was opened for (focus-zone-first on a real right-press has no harness here, since a secondary press can't be posted on Linux and gpui's test support isn't enabled; it is a manual check in 11.4)
- [x] 8.4 Print an open menu in `--dump-screen-after` (`--- menu items=… highlighted=N`, `--- focus menu`) and document it in CLAUDE.md's verification section; verify with e2e cases `tab-menu` and `zone-menu` that drive `--action open_tab_menu`, `--key down`, `--key enter` and check: items, disabled ends, Move Tab Left reorders, Rename Tab opens the prompt, Escape restores focus to the terminal, a tmux tab offers Rename Window and Close Tab ends the tmux window
- [x] 8.5 Add "right-click opens the tab and zone menus, a hover shows a tooltip, on Linux and on Windows" to CLAUDE.md's manual-check list and mention the tab menu in README.md; verify both files show the lines

## 9. Zone headers and dividers

- [x] 9.1 Rebuild the zone header on token colours, ghost `IconXs` `Button`s (`Columns2`, `Rows2`, `X`) with tooltips (presses stop at the button as in 6.3, since the header's own press focuses the zone; the title uses the same truncation helper as 6.1), keeping the 28px height, the 2px active line in `primary`, status dot and the focus-on-press handler; recolour the dividers from the `border` token; verify the `zones` e2e cases pass and a capture case samples header, active line and divider colours
- [x] 9.2 Remove the headers' uses of the four chrome helpers; verify `grep` finds none in `src/zone_view.rs`

## 10. tmux chrome and theme.rs

- [x] 10.1 Recolour the tmux zoomed badge with the `secondary` token pair and take the divider fallback from the tokens (`border` mixed with `foreground`), read in `render`; keep canvas-painted title lines and `theme.accent()` for the active divider; verify the `tmux-*` and title-line e2e cases pass under `scripts/with-xvfb`
- [x] 10.2 Make `chrome_background`, `chrome_border`, `muted_foreground` and `hover_background` private to `src/theme.rs` and delete the unused; verify `grep -rn "chrome_background\|chrome_border\|hover_background\|\.muted_foreground()" src --include=*.rs` outside `theme.rs` is empty, `cargo build` is warning-free and the contrast test passes
- [x] 10.3 Update CLAUDE.md's architecture bullets (`workspace.rs` strip on `Tabs`, `zone_view.rs` headers, the `brindle-ui` component list, tokens as the only chrome colour source); verify the text matches the code

## 11. Integration checks

- [x] 11.1 Run `cargo test --workspace`, `cargo build --release --locked` and `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e` (both tmux versions via the CI matrix if available); verify all pass
- [x] 11.2 Compare `latency-shell` and `latency-tmux` before and after (`BRINDLE_PERF_BASELINE`, asking before opening windows on `:0`); verify no regression beyond noise. Done under `scripts/with-xvfb` (lavapipe) on the final strip, alternating trees: latency p50 equal and p95 about 1.5 ms lower, no timeouts; `throughput-tmux` inside `main`'s spread except `unicode` (five runs per tree: `main` 303-373 ms, median 315; `ui-chrome` 335-540 ms, median 357, three of five inside `main`'s range), which stays inconclusive on a loaded machine and is noted in the PR. `throughput-brindle` fails on both trees (Brindle does not exit within 600 s), a harness problem outside this change.
- [ ] 11.3 On Windows, run the `win` e2e cases and check by hand that a tab and the new-tab button respond, empty strip space drags, the caption buttons and Snap Layouts work, and right-click on a tab opens the tab menu and not the system menu; verify each by observation
- [ ] 11.4 Hand the user the manual checks via `scripts/run-throwaway`: tab drag-reorder, the `⌄` profile menu and ctrl-shift-space palette, right-click menus on a tab and a zone header (a right-click on a background tab activates it, and on an unfocused zone's header focuses that zone first, before the menu acts on it; a right-click on a tab doesn't also open the window menu), tooltips on strip, header and find bar buttons, keyboard walk of the strip, a profile theme differing from the global one; verify with their report

## 12. Strip look: `line` variant, each tab in its own theme

Added after 11.1 passed; 11.2 to 11.4 run on the result, so redo 11.1 once this section is done.

- [x] 12.1 In `brindle-ui`, remove `TabVariant::Attached` (and `TAB_ATTACHED_HEIGHT`, `align_bottom`, `rounded_top_only` if nothing else uses them); give `TabColors` a `line` colour and apply a trigger's `surface`, `text` and `line` in the `Default` and `Line` variants whether it is active or not (inactive text at 60% opacity until hovered, then full; the active top line in `line` else `primary`); add `TabTrigger::line_color`; verify unit tests of `tab_style` (owner colours on an active and an inactive trigger, hover full opacity, no owner colours unchanged)
- [x] 12.2 Update the `ui-components` check cases and the gallery (`-- --check`) for the removed variant and the new colour rules, and `ui/README.md` (the `Attached` difference goes, the owner-colours addition is listed with its reason); verify `cargo run -p brindle-ui --example gallery -- --check` has no `FAIL` and the README no longer mentions `Attached`
- [x] 12.3 Move `render_tab_bar` to `TabVariant::Line` and give every trigger its own terminal's background, foreground and `accent()` (background tabs included, tmux tabs from their active pane's theme); the list stays on the global background token; verify an e2e capture case with a profile theme differing from the global one samples an inactive tab in its own theme's background and the strip in the token background, and that the active tab's top line is the tab's accent
- [x] 12.4 Update everything that assumed the 30px strip (`TAB_ATTACHED_HEIGHT` users, the caption hit-test region, e2e capture offsets, `--dump-screen-after` grid origin expectations, the zone and tmux layout offsets); verify `cargo test --workspace` and the full e2e under `scripts/with-xvfb` pass, and the `win` cases on Windows
- [x] 12.5 Update CLAUDE.md's `workspace.rs` bullet (the strip is a `brindle_ui::TabList` in the `Line` variant, with each tab's face in its terminal's colours) and the `brindle-ui` bullet (no `Attached` variant); verify `grep -n Attached CLAUDE.md` finds nothing about tabs

- [x] 12.6 Add the Split Right, Split Down and Find buttons to the strip between its empty space and the window controls (`Columns2`, `Rows2`, `Search`; press-stopping wrappers as in 6.3; tooltips with the bound key; tmux tabs dispatch the `tmux_split_*` actions; split buttons disabled through `can_split_zone`); verify e2e cases that use the tab's actions to check a split from the strip's action path and the disabled state, `--dump-screen-after` zone output after a split, and that the `win` caption hit-test still passes on Windows

## 13. After hand checking: pill tabs, no strip actions, legible icons

The line on the active tab ran past its rounded corners and the line variant did not look like shadcn's tabs; the strip's icons were too small to see; the split and find buttons are dropped from the strip (split moves to the zones, see the `zones-shadcn` change). This supersedes the top line of 12.1 to 12.3 and the buttons of 12.6. The perf comparison of 11.2 is not rerun.

- [ ] 13.1 In `brindle-ui`, remove the `Line` variant (and `TabVariant` with it, along with `TabList::variant`, if one value is left: the list is always the pill), `TabColors.line` and `TabTrigger::line_color`; make the pill honour a trigger's owner `surface` and `text` colours whether active or not (inactive text at 60% opacity until hovered, the active trigger raised with `shadow-sm`, icons and the close button in the text colour); update the `tab_style` unit tests, the gallery strip demo and its check, and `ui/README.md`; verify `cargo test -p brindle-ui` passes and `grep -rn line_color ui/ src/` finds nothing
- [ ] 13.2 Make the strip's icons legible: `IconSm` buttons for the new-tab, profile and window-control buttons, the icon at least 16px, and `Icon`'s stroke at least 2px at that size (set in pixels, not scaled down with the icon), also for the tab close button; keep the caption buttons handler-free on Windows; verify a unit test of the stroke width at 16px and `--dump-screen-after` or a capture case that samples the `+` glyph's stroke width
- [ ] 13.3 Move `render_tab_bar` to the default (pill) variant with each face in its terminal's background and foreground, no line; remove the Split Right, Split Down and Find buttons (`StripButton`, `press_strip_button`, `--strip-button`, the `--- strip` dump lines, `can_split_active` if nothing else uses it, the `tabs-strip-buttons` case) and update `tabs-strip-colors` and the `win` caption points for the pill's geometry and the missing buttons; verify `cargo test --workspace` and the full e2e under `scripts/with-xvfb` pass, and the `win` cases on Windows
- [ ] 13.4 Update CLAUDE.md (the strip bullet: pill variant, no actions, no `--strip-button` or `--- strip` lines); verify `grep -n "strip-button\|StripButton\|line_color" CLAUDE.md` finds nothing

## Workflow follow-up

- Open a PR from a branch (the usual): implementation commits, then an archive commit `docs(openspec): archive ui-chrome`; the user merges.
- Archive the change after review so `ui-components`, `ui-theme-tokens`, `tabs`, `profiles`, `zones` and `scrollback-search` take the deltas.
