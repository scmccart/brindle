# Tasks

## 1. Crate scaffold

- [x] 1.1 Create `ui/` (`brindle-ui`, lib `brindle_ui`, version equal to the other workspace crates, dependency `gpui = "0.2.2"` only, MIT OR Apache-2.0) with an empty `lib.rs`. Add it to workspace `members` and `default-members` and to `brindle`'s dependencies. Verify: `cargo build --locked` fails only because of the new path entry; then `cargo build` and the `git diff Cargo.lock` show only `brindle-ui` added (no third-party crate added or bumped).
- [x] 1.2 Update CLAUDE.md: "four workspace crates" becomes five (the CI `test` job description) and "bumps all four crates" becomes five (the `release.yml` paragraph), and the Architecture section gains a `brindle-ui` entry stating the boundary (depends on gpui only; tokens, primitives, components, `TextInput`). Verify: CI's version check command from `.github/workflows/ci.yml` (`cargo metadata --no-deps … | jq`) prints one version for all five crates, and `release-please-config.json`'s `rust` release type picks up the new member the way it does `term-core` (no per-crate entry exists for the others, so none is added).

## 2. Move TextInput and the filter

- [x] 2.1 Move `src/text_input.rs` to `ui/src/text_input.rs` and its 19 `TextInput*` actions to `ui/src/actions.rs` (`actions!(brindle_ui, …)`). Add `brindle_ui::default_bindings()` returning the `TextInput`-context bindings moved out of `src/actions.rs`. Verify: the moved `EditState` and `scroll_for_caret` tests pass with `cargo test -p brindle-ui`.
- [x] 2.2 In Brindle, chain `brindle_ui::default_bindings()` into `actions::default_bindings()`, point the `text_input_*` `ACTIONS` entries at `brindle_ui::TextInput*`, and repoint imports in `picker.rs`, `find_bar.rs`, `settings_dialog.rs`, `settings_dialog/profiles.rs`, `workspace.rs` and `tmux/mod.rs` (every file that names `text_input` or `TextInput`). Verify: `cargo test --workspace` passes (including `default_bindings_parse` and the ACTIONS round-trip test), and `brindle --list-actions` lists the same `text_input_*` names as before.
- [x] 2.3 Move `fuzzy_match`/`filter` from `picker.rs` to `ui/src/filter.rs` with their tests, and import them in `picker.rs`. Verify: `cargo test --workspace` passes.
- [x] 2.4 Verify the move changed no behaviour: on Linux (via the Remote Control session) `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e -- palette settings find` passes, and on Windows `BRINDLE_E2E=1 cargo test --test e2e` passes (ask before running on the desktop).

## 3. Tokens, styles and icons

- [x] 3.1 Add `Tokens` (`theme.rs`) with the fields in design.md, `neutral_light()`/`neutral_dark()` as hex constants converted from shadcn's neutral oklch values, radius steps, the `UiTheme` global and the `ActiveTheme` trait (`cx.ui_theme()`). Verify: a unit test pins `background`, `primary`, `border` and `ring` of both sets to the converted values, and one that `neutral_dark().border` is foreground-white at 10% opacity.
- [x] 3.2 Add `ButtonVariant`/`ButtonSize` and the pure `button_style()` (`style.rs`) using the rem scale (`h_9` default, `h_8` sm, `h_10` lg, `size_9` icon). Verify: a table test covers all 24 combinations against both token sets (colours come from the expected tokens, hover is the 90% opacity form where shadcn uses `/90`).
- [x] 3.3 Add `Icon` (`Check`, `ChevronDown`, `ChevronUp`, `X`, `Search`) painted with `PathBuilder` in the current text colour, sized in rems. Verify: unit test that each icon's path points stay inside its unit box; visual check in the gallery (task 6.1).

## 4. Primitives

- [x] 4.1 `LayerStack` (pure: push, remove, `is_top`, depth) and the per-window registry keyed by `WindowId`. Verify: unit tests for top-only dismissal, removal of a middle layer, and that dismiss decisions taken against a snapshot don't cascade (popover and dialog in one press → only the popover).
- [x] 4.2 `Overlay<V: ManagedView>`: open records the weak previously-focused handle, pushes the layer, focuses initial → first stop → own handle, subscribes to `DismissEvent`; close pops, drops and restores focus if the handle upgrades. Outside presses and Escape (`Dismiss` in `UiLayer`, applied via `window.defer`) dispatch `Dismiss` to the view, with a `dismiss_on_request()` default. Modal layers render inline with an `occlude`d full-window backdrop, never through `deferred` (the owner places them last in its window root); non-modal layers render through `deferred(...).with_priority(depth)` unless a non-modal layer is open below them, when they render inline (`wrap_in_deferred(stack, id)`, pure). Verify: unit tests for the pure focus-target choice (initial/first/own), the restore rule (upgrades or leaves focus alone) and the deferred-or-inline decision (modal never; non-modal alone, above a modal, above a non-modal).
- [x] 4.3 `FocusScope` element and `FocusStops` trait: `tab`/`shift-tab` bound to `FocusNext`/`FocusPrev` in `UiFocusScope` (added to `default_bindings()`), moving with pure `cycle()`. Add the debug assertion for a focused handle missing from the stops. Verify: unit tests for wrap forward/back, focus outside the stops, and an empty list.
- [x] 4.4 `place()` (pure: side, align, offset, flip, clamp with margin) and the `Anchored` element that lays out its child and positions it; trigger bounds recorded with `on_children_prepainted`. Verify: unit tests for each side and align, flip at each edge, and clamping when neither side fits.
- [x] 4.5 `ListNav` (next/prev/first/last over an enabled predicate, wrap flag) and `Typeahead` (`push(ch, now)`, 1 s accumulation, single-char cycling) with `UiList` bindings (`up`/`down`/`home`/`end`/`enter`, `ctrl-p`/`ctrl-n`) in `default_bindings()`. Verify: unit tests for disabled skipping, wrap vs stop, typeahead accumulation, timeout and cycling (the spec's `bash`/`fish`/`foot` scenario).
- [x] 4.6 Focus-visible modality: `init(cx)` registers `observe_keystrokes` (→ `Keyboard`); a crate helper used by every component's mouse-down sets `Pointer`. `init` binds no keys. Verify: unit test of the ring rule (`focused && Keyboard`, or any focused input); a reload-safety check that `init` registers no `KeyBinding`.

## 5. Components

- [x] 5.1 `Button` (`RenderOnce`): variant, size, `disabled`, `.focus(&handle)`, `on_click` (GPUI keyboard click covers enter/space), ring per 4.6, `Styled` passthrough for layout, optional leading `Icon`. Verify: unit test that a disabled button resolves to no click handler and no tracked focus; gallery check.
- [x] 5.2 `Input` (`RenderOnce` around `Entity<TextInput>`): input-colour border, ring when its `TextInput` is focused, destructive border when `invalid(true)`, press anywhere focuses the input. Verify: unit test of the border-colour choice (normal/focused/invalid); gallery check.
- [x] 5.3 `Checkbox` and `Switch` (`RenderOnce`, controlled): `checked`, `disabled`, `.focus(&handle)`, `on_click(|new: &bool, …|)`, `Icon::Check` for the tick, switch thumb position from state. Verify: unit test that the requested value is `!checked` and that disabled requests nothing; gallery check.
- [x] 5.4 `Dialog` chrome (`RenderOnce`): title, description, body, footer, close `X` button, used inside a `ManagedView` opened through a modal `Overlay`; entry animation keyed by open count. Verify: gallery dialog with an input, a select and two buttons; Tab cycles through all of them and wraps; Escape and a backdrop press close it and focus returns to the opening button; a dialog that handles `Dismiss` itself stays open.
- [ ] 5.5 `Popover`: trigger wrapper + content `ManagedView` in keyed state, non-modal `Overlay`, `Anchored` placement (default side bottom, align centre), trigger toggles. Verify: gallery popover opened near each window edge flips and stays inside; outside press and Escape close it; focus returns to the trigger.
- [ ] 5.6 `Select` (`Entity`, emits `SelectEvent::Changed(ix)` only on change): trigger shows label or placeholder and `ChevronDown`; enter/space/up/down open; menu is a popover layer highlighting the selection, no wrap, typeahead open and closed, scroll-into-view. Verify: unit tests for "changed only when different" and closed-menu typeahead; gallery checks of the spec's Selects scenarios, and a select inside the gallery dialog whose menu closes on Escape while the dialog stays open.
- [ ] 5.7 `Command` (`Entity`, emits `CommandEvent::Confirmed(item)` / `Cancelled`): `TextInput` filter with `Icon::Search`, grouped items in `gpui::list`, `brindle_ui::filter`, empty-group hiding, empty message, highlight reset to the first match on each filter change, wrapping `UiList` navigation. Verify: unit tests for the filtered row model (groups hidden, first match highlighted, the spec's `spr` scenario); gallery checks inline, in a dialog and in a popover.

## 6. Gallery and docs

- [ ] 6.1 `ui/examples/gallery.rs`: calls `init`, binds `default_bindings()`, shows every component in every variant and size with a light/dark toggle, and the dialog/popover/select/command cases from group 5. Verify: `cargo test --workspace` builds it; a manual run (ask first on a desktop; `scripts/with-xvfb cargo run -p brindle-ui --example gallery` on Linux) shows each component and passes the checks listed in 5.1–5.7.
- [ ] 6.2 Add `ui/README.md` (purpose, boundary rule, component kinds, how an app wires `init`/`default_bindings`/`UiTheme`/`Overlay`) and mention the gallery command in the root README's build section. Verify: the commands in both files run as written.

## 7. Integration

- [ ] 7.1 Full check: `cargo test --workspace --locked` and `cargo build --release --locked` on Windows and, via the Linux Remote Control session, on Linux with `scripts/with-xvfb env BRINDLE_E2E=1 cargo test --test e2e`. Verify: all pass, and `scripts/lint-commits` passes for the branch's commits (scope `ui`).

## Workflow follow-up

- Archive the change after review; `ui-components` becomes a main spec.
- Next in the sequence: `ui-tokens-from-theme`, then `ui-settings-dialog`, `ui-command-palette`, `ui-chrome`.
