# Tasks

Before starting, check that `agents-profiles` and `agents-markdown` are archived, and read design.md's flags F1–F6: if the user has answered any differently, change the decision and its tests first. If `agents-markdown-layout-fixes` has landed, its `agent-markdown-layout` case's `muted` constant moves to the zone set's in task 4.1.

## 1. Pure code: zone tokens

- [x] 1.1 In `src/theme.rs`, move `ui_tokens`' derivation into a private `tokens_over(background)` and add `Theme::zone_tokens()` over the theme's own background (decision 2); verify with unit tests that `zone_tokens().background` is the theme's background and its primary and ring the accent, that `ui_tokens()` is unchanged for every built-in theme (compare against the values before the move), and that `brindle --export-ui-tokens`' output (`ui_tokens_css`) is byte for byte the same.
- [x] 1.2 Extend `ui_tokens_are_legible_for_every_builtin` to check the same pairs over `zone_tokens()`, naming the set in its message; verify with `cargo test -p brindle ui_tokens_are_legible_for_every_builtin`.

## 2. brindle-ui: the scope and what carries it

- [x] 2.1 In `ui/src/theme.rs`, give `UiTheme` its private `scope: Cell<Option<Tokens>>`, make `ActiveTheme::ui_theme` return `Tokens` by value (the scope's set, else the window's) and add `window_theme`; update every caller in `ui/` and `src/` (the `*cx.ui_theme()` and `&Tokens` sites); verify with `cargo build --workspace --locked`, `cargo test --workspace` and the clippy command, in a commit of its own.
- [x] 2.2 Add `ThemeScope` (`ui/src/primitives/theme_scope.rs`, exported from the crate root), swapping the cell around its child's three phases and restoring the outer value, its doc naming what it can't reach (decision 1); verify with `TestAppContext` tests of a probe element reading the active set in each phase inside a scope, outside it, after it, and in nested scopes, and of a `MessageScroller` item scrolled into view inside a scope.
- [x] 2.3 Audit every `ui_theme()` caller in `ui/` and `src/` for a read outside render, prepaint and paint (an event handler, an entity's `update`, a stored colour); fix any found inside an agent zone's subtree and list them in design.md's "As built"; verify by the list and the tests of 2.2.
- [x] 2.4 Give `Tooltip` its optional tokens, rendering its panel in a `ThemeScope` of them, and have `attach`, `Badge` and `Response`'s link tooltips take `cx.ui_theme()` at render only while a scope is active (decision 4); document on `TooltipFor` that its tooltips keep the window's set; verify with tests that a hovered `Button` inside a scope shows a tooltip holding the scope's tokens, one outside holds none, and a link tooltip inside a scope holds them.
- [x] 2.5 Add `tokens(Option<Tokens>)` to the overlay in `ui/src/primitives/overlay.rs`, putting the panel in a `ThemeScope` inside the `deferred` it may be lifted into (decision 5); verify with tests that a non-modal overlay given tokens renders its panel with them, both lifted and inline, and one without tokens renders with the window's.
- [x] 2.6 Add `ring: Option<Hsla>` to `CardColors`, defaulting to the ring token; verify with a test that a card inside a scope given the window's ring draws the ring in it, and that `CardColors`' existing defaults hold.
- [ ] 2.7 Add a scoped panel to the gallery beside an unscoped one, and the check `theme-scope-phases`; list `ThemeScope` in `ui/README.md` (a Brindle addition, not shadcn's); verify with `cargo build -p brindle-ui --example gallery`, and hand `-- --check` under `scripts/with-xvfb` to the run chore.

## 3. The app's views

- [ ] 3.1 In `src/zone_view.rs`, keep the zone tokens beside `theme` (computed in `new` and `set_profile`, the field's doc naming its refresher), read the window's set with `window_theme()` at the top of `render`, wrap an agent zone's `Card` in a `ThemeScope` of the zone tokens with the window's ring in its `CardColors`, and leave terminal zones unwrapped (decision 3); verify with fixture tests that an agent zone in a tab with a profile theme gets that theme's zone tokens, its card's ring is the window's, and a terminal zone gets no scope.
- [ ] 3.2 Give `ZoneMenuRequest` its `tokens` (the zone's for an agent zone, `None` for a terminal zone) and have `Workspace::open_zone_menu` keep them with the open menu and pass them to the overlay; verify with a fixture test of both requests, and that the tab menu still opens with none.
- [ ] 3.3 In `src/agent/view.rs`, draw the root's background and text from `cx.ui_theme()`, keeping the ANSI red and yellow for notices and the held row, and update `set_profile`'s doc; verify with `cargo test -p brindle agent::view` and that the view's tests still pass.
- [ ] 3.4 Update `docs/code-map.md`: the `agent/` entry's `set_profile` sentence (the zone is drawn in its theme's zone tokens), the `theme.rs` entry (`zone_tokens`), and the `brindle-ui` entry (`ThemeScope`, the by-value `ui_theme`, `window_theme`, tooltips and overlays carrying a set); verify each names the new items.

## 4. End to end and manual checks

- [ ] 4.1 Add the zone set's `card`, `muted` and `popover` for the `e2e-alt` colours and the window set's `card`, `muted` and `popover` for `e2e` to `tests/e2e/brindle.rs`'s constants, with their derivations; move `agent-markdown-layout`'s `muted` constant to the zone set's if that case exists; verify with `BRINDLE_E2E=1 cargo test --test e2e -- --self-test`.
- [ ] 4.2 Add the case `agent-zone-theme` (`needs_fake_agent`) to `tests/e2e/cases/agent.rs` and `cases/mod.rs`, as design.md's decision 6 describes: a case theme with `e2e-alt`'s colours and its own accent, a message with a code block and a pending permission, the zone set's colours and the case's accent present in the zone's rectangle and the window set's and the global accent absent, the strip unchanged; then a split, `open_zone_menu` with `--modality pointer`, and the window set's `popover` absent from the zone; verify with `--list` showing it, and hand the run under `scripts/with-xvfb` to the run chore.
- [ ] 4.3 Replace `CLAUDE.md`'s manual check "an agent zone with a profile theme unlike the chrome's (only the card takes the profile's background and foreground; the components keep the chrome's colours) …" with design.md's manual checks (every component in the profile's colours, tooltips with a real pointer, the header menu, the close confirmation in the chrome's; in brindle-light, brindle-dark and solarized-dark), keeping the large-font part.

## 5. Integration

- [ ] 5.1 Run the integration chore: `cargo test --workspace --locked`, `cargo clippy --locked --workspace --all-targets -- -D warnings`, the gallery's `--check` and `BRINDLE_E2E=1 cargo test --test e2e -- agent cards zone` under `scripts/with-xvfb` (through `systemd-run --user --scope` when run from inside Brindle); verify all pass, including the `cards` and `zone` cases whose colours this change must not move, and report the new checks' lines.

## Workflow follow-up

- Archive the change after review, syncing its delta specs.
