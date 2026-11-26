# Design

## Context

See proposal.md for why. What this design rests on, from `main` at 725ed28 and the investigation of 2026-11-25 (`~/.cache/claude-tasks/brindle/manual-findings/investigation.md`, question 8):

- **Where an agent zone's colours come from today.** `AgentView::set_profile(theme, font_size)` (`src/agent/view.rs:386-397`) keeps the tab's terminal `Theme`. `render` uses only its `background` and `foreground` for the zone root (`view.rs:858, 883-886`), and its ANSI red and yellow for notices and the held row. The card and header come from `Theme::card_colors` in `zone_view.rs` (`render_leaf`, `src/zone_view.rs:1566-1700`). Every `brindle-ui` component inside reads `cx.ui_theme()`, the one `UiTheme` global (`ui/src/theme.rs:152-178`), which `Settings::set_ui_theme` fills from the global theme through `Theme::ui_tokens` (`src/theme.rs:304-340`).
- **`ui_tokens` is derived over the chrome background**, the theme's background mixed 35% towards black or white (`chrome_background`, `src/theme.rs:258-261`). Its `card` and `muted` are that background mixed 4% and 8% towards the foreground. A zone card's surface is the theme's own background (`card_colors`). So even with no profile theme, an agent message's bubble (`card`) is darker than the zone it sits on in a dark theme.
- **When components read tokens.** gpui renders `RenderOnce` components and entity views during their own `request_layout`. `gpui::list` (the `MessageScroller`) renders and lays out the items in view during **prepaint**. A few components read tokens at **paint** (`TextArea`'s cursor and selection). `RemScope` (`view.rs:777-845`) already wraps all three phases, with `Window::with_rem_size`, and shows that the timing allows a scope.
- **Globals and notification.** `cx.set_global` and gpui 0.2.2's `global_mut` both queue `NotifyGlobalObservers` (`gpui-0.2.2/src/app.rs:1474-1476`), so swapping the global around each phase would notify every observer of `UiTheme` twice a frame, and an observer that notifies a view would loop.
- **Tooltips** are built by a closure gpui runs on hover, then laid out and painted after the window's tree (`Window`'s tooltip prepaint), outside any element of it. `brindle-ui` builds them in three places: `attach` (a component's stored handler, run in the component's render), `TooltipFor` on a raw element (no `cx` where it is called), and `Response`'s link tooltips (in its render). Tooltips are inverted, `foreground` on `background` (`tooltip_style`, `ui/src/style.rs:687-700`).
- **The zone header's menu** is opened by the workspace: `ZoneTreeView::open_menu` emits `ZoneMenuRequest { rows, at, press }` (`zone_view.rs:1311-1328`), and `Workspace::open_zone_menu` (`src/workspace.rs:1444-1457`) opens an `ActionMenu` as a non-modal overlay, which `brindle-ui`'s overlay may lift into a `deferred` (`ui/src/primitives/overlay.rs:300-350`). A deferred element is prepainted and painted after the rest of the tree.
- **The window-level dialogs** (the close confirmation, the link confirmation) are modal overlays the workspace draws over the whole window.

## Goals / Non-Goals

**Goals:**
- Every component inside an agent zone, in every phase, takes the zone's tokens, with no change to the components' own code beyond how they read the active set.
- No notification, observer or redraw caused by entering or leaving a scope.
- Tokens that fit the surface the zone shows, checked for legibility like the window's.

**Non-Goals:** scopes for terminal zones; new tokens (warning, success); a per-zone font family; moving `RemScope` into `brindle-ui` (it could join `ThemeScope` later, and nothing here needs it).

## Layer map

| Part | Kind | Crate and module |
| --- | --- | --- |
| `Theme::zone_tokens`, and the derivation it shares with `ui_tokens` | pure | `brindle`, `src/theme.rs` |
| `UiTheme`'s scoped set, `ActiveTheme::{ui_theme, window_theme}` | component support (GPUI global) | `brindle-ui`, `ui/src/theme.rs` |
| `ThemeScope` | element | `brindle-ui`, `ui/src/primitives/theme_scope.rs` (new) |
| `Tooltip` keeping its tokens | component | `brindle-ui`, `ui/src/components/tooltip.rs`, `button.rs`, `badge.rs`, `response.rs` |
| An overlay's token set | primitive | `brindle-ui`, `ui/src/primitives/overlay.rs` |
| `CardColors::ring` | component | `brindle-ui`, `ui/src/components/card.rs` |
| The zone's tokens, kept per theme; the scope round an agent zone's card; the menu request's tokens | shell (entity) | `brindle`, `src/zone_view.rs` |
| The header menu drawn in the request's tokens | shell (entity) | `src/workspace.rs` |
| The zone root's background and text from the scoped set | shell (entity) | `src/agent/view.rs` |

`brindle-ui` learns nothing of profiles or zones: `ThemeScope` takes a `Tokens` value, and the app decides which (`ports R2`). The decision of which set a zone uses is the pure `Theme::zone_tokens` (`fcis R1`, `R5`); the entity only keeps and passes it. No crate reference changes (`ports R1`, `R3`).

## Decisions

### 1. The scoped set lives inside the global (`ports R2`; `process R14`)

```rust
pub struct UiTheme {
    pub tokens: Tokens,            // the window's set
    scope: Cell<Option<Tokens>>,   // the innermost scope's, while one is drawn
}
pub trait ActiveTheme {
    fn ui_theme(&self) -> Tokens;      // the scope's set, else the window's
    fn window_theme(&self) -> Tokens;  // the window's, ignoring any scope
}
```

- `ThemeScope { tokens, child }` swaps the cell around its child's `request_layout`, `prepaint` and `paint`, as `RemScope` swaps the rem size: `let outer = theme.scope.replace(Some(tokens)); child.phase(…); theme.scope.set(outer);`. Saving and restoring the outer value makes nested scopes work, the innermost winning.
- The cell is reached through `cx.global::<UiTheme>()`, an immutable borrow, so nothing is notified and nothing outside redraws. `UiTheme::set` (the window's set) still uses `set_global` and `refresh_windows`, as today.
- **`ui_theme()` returns `Tokens` by value** (it is `Copy`, about 370 bytes) instead of `&Tokens`, since a `Cell` can't lend a reference. The 57 callers change mechanically: `*cx.ui_theme()` loses its `*`, and a function taking `&Tokens` is given `&t`. `UiTheme` stops being `Copy`; no caller copies it.
- **Alternatives.** Swapping the global with `set_global` around each phase: rejected, since it notifies observers (see Context). A thread-local stack in `theme.rs`: works, but is shared by every `App` on the thread (gpui's tests run several), so a leak from one test could reach another; the cell is per `App`. Passing tokens down to each component as a prop: dozens of components and every nested one (a `Button` inside a `Confirmation` inside a `Message`) would need it, and a missed one is silently wrong.
- **What a scope can't reach.** A `deferred` child is prepainted and painted after the scope has ended, and a tooltip is drawn outside the tree. Decisions 4 and 5 handle the two that occur. `ThemeScope`'s doc says so, and a scope goes inside a deferred element, never round one.
- A panic inside a phase leaves the cell set, but a panic on the UI thread ends Brindle anyway; no guard is added.

### 2. Zone tokens (`fcis R1`, `R3`, `R8`; `ui-theme-tokens`)

- `Theme::zone_tokens()` derives the set as `ui_tokens()` does, over the theme's own `background` instead of `chrome_background()`. Both call one private `tokens_over(background)`, so the two can't drift. `background` is the theme's background; `card` and `popover` are it mixed 4% towards the foreground, `muted` and `secondary` 8%, `accent` 12%, `border` and `input` 12% (`card_border`); `primary`, `ring` and `link` come from the accent, `destructive` from the red, and every foreground is pushed to its pair's contrast (4.5:1, muted 3:1) as `ui_tokens` does.
- So an agent message's bubble is a faint raised surface on the background the zone actually shows, and a code block's `muted` surface is a step above it.
- **FLAG F1 (the user didn't choose this).** Recommendation: zone tokens over the theme's own background. The alternative is the profile theme's `ui_tokens()` as it is. It is simpler, but its `background` is the darker chrome mix, so any component drawn on `background` (the prompt's textarea, outline buttons) would sit darker than the card around it.
- **Contrast.** `ui_tokens_are_legible_for_every_builtin` (`src/theme.rs:802`) checks the same pairs over both derivations, one assertion message naming which set failed. A user's own theme gets the same pushes; nothing else checks it, as today.
- **As built (2026-11-25).** Over its own background, solarized-dark's zone set missed two pairs (foreground on muted 4.27:1, muted foreground on card 2.86:1), since the theme's own background is lighter than the chrome mix and its foreground low in contrast. `tokens_over` now also pushes the foreground to 4.5:1 on the muted surface and the muted foreground to 3:1 on the card, after the pushes it already made; the surfaces are still mixed from the unpushed foreground. Every window set already met both, so `ui_tokens` and `brindle --export-ui-tokens` are byte for byte unchanged (checked by a diff of the export before and after, and `ui_tokens_keep_their_values` pins the brindle themes' surfaces).

### 3. Which zones, and what keeps the window's colours

- **Every agent zone is drawn in a scope**, of its tab's theme's zone tokens: the profile's theme when it sets one, else the global theme. `ZoneTreeView` keeps the set beside its `theme`, computed in `set_profile` and `new` (`types R10`: the field's doc names `set_profile` as what refreshes it), never in `render` (`process R14`).
- **FLAG F2.** Recommendation: scope agent zones always, so a zone looks the same whether its theme comes from the profile or the global setting. The alternative scopes only when the profile names a theme, which leaves the darker bubbles of today in other agent zones.
- `render_leaf` wraps an agent zone's whole `Card` (header and content) in `ThemeScope`. The header's buttons, their hover, and the badge's radius follow it. What must keep the window's colours is passed explicitly from the window's set, read with `window_theme()` (not `ui_theme()`) at the top of `ZoneTreeView::render`:
  - the card's ring: `CardColors` gains `ring: Option<Hsla>`, defaulting to the ring token, set to the window's ring for agent zones (spec "Zone ring stays on the tokens");
  - the status dot, the progress bar and the context badge, already computed from `Settings::chrome_theme` and the window's `muted_foreground`;
  - the surround colour of the card's corners, already the window's background.
- **Focus rings of controls inside the zone** (the prompt's textarea, buttons) take the zone's `ring`, the profile theme's accent, as every other colour inside does.
- `AgentView` draws its root's `bg` and `text_color` from `cx.ui_theme()` (the scope's `background` and `foreground`) instead of `self.theme`, so there is one source. Its notices and held row keep the theme's ANSI red and yellow (there is no warning token).
- **Terminal zones are unchanged**, including a terminal login zone opened beside an agent zone (`agents-login`): their headers' buttons keep the window's tokens.
- **FLAG F3.** Recommendation: leave terminal zones as they are, since the user's decision named the agent zone. The alternative scopes every zone's header, which would also make a login zone's header match its agent zone's.

### 4. Tooltips (`ui-components` "Tooltips and overlays in a theme scope")

- `Tooltip` gains `tokens: Option<Tokens>`, and its panel renders inside a `ThemeScope` of them when set. The text size stays the window's, since the tooltip is drawn outside the zone's `RemScope`, as `agents-profiles` decided.
- The tokens are taken **when the control renders**, inside the scope: `attach` reads `cx.ui_theme()` in the component's render and gives it to the tooltip its handler builds. `Button`, `Badge` and `Response` (whose link tooltips are built in its render) all pass through it. Each stores the tokens only when a scope is active, so a tooltip outside one keeps following the window's set live.
- `TooltipFor` on a raw element has no `cx` where it is called, so its tooltip keeps the window's set. Nothing inside an agent zone uses it (every tooltip there is a `Button`'s, a `Badge`'s or a link's), and its doc says to use a component's tooltip inside a scope. No new form is added for a use that doesn't exist (`types R12`).
- **FLAG F4.** Recommendation: tooltips inside the zone take its tokens. The alternative leaves them in the window's tokens, which needs no change but shows light tooltips over a dark zone.

### 5. The header menu and window dialogs

- `ZoneMenuRequest` gains `tokens: Option<Tokens>`: the zone's set for an agent zone, `None` for a terminal zone. The workspace keeps it with the open menu and gives it to the overlay. `brindle-ui`'s overlay gains `tokens(Option<Tokens>)`, which puts its panel inside a `ThemeScope`, **inside** the `deferred` it may be lifted into, so the scope covers the menu's prepaint and paint (decision 1).
- **FLAG F5.** Recommendation: the zone header's menu takes the zone's tokens, as it opens from the zone and sits over it. The alternative keeps it in the window's tokens, like the tab menu, which needs no request change.
- **The close confirmation and the link confirmation keep the window's tokens.** They are modal dialogs over the whole window, with a backdrop over the strip as well, and the same dialogs serve terminal zones and tabs.
- **FLAG F6.** Recommendation: window dialogs stay in the window's tokens. The alternative would carry the zone's set into `ConfirmLink` and the close request.

### 6. Tests (`testing R1`, `R6`, `R13`, `R15`, `R16`)

- **Pure** (`src/theme.rs`): `zone_tokens`' background is the theme's own, its primary and ring the accent; the legibility test over both sets for every built-in theme; `ui_tokens` unchanged (its existing tests and `ui_tokens_css`' output byte for byte).
- **`brindle-ui`** (`TestAppContext`): a probe element reading `cx.ui_theme()` in each phase inside a scope, outside it, after it (restored) and in nested scopes; a `MessageScroller` inside a scope whose item, scrolled into view, renders with the scope's set; a `Button` with a tooltip inside a scope, hovered, whose `Tooltip` holds the scope's tokens, and one outside holding none; an overlay given tokens whose panel renders with them, lifted into `deferred` and not; a card in a scope with the owner's ring colour.
- **App views** (`src/zone_view.rs`'s fixture): an agent zone in a tab with a profile theme passes that theme's zone tokens to its scope, the card's ring is the window's, a terminal zone gets no scope, and a zone menu request from an agent zone carries its tokens and one from a terminal zone none.
- **Gallery**: a scoped panel beside an unscoped one, and the `-- --check` check `theme-scope-phases` with the probe.
- **e2e case `agent-zone-theme`** (`needs_fake_agent`, window captures). The case writes its own theme with `e2e-alt`'s foreground and background and an accent of its own (`e2e-alt` shares `e2e`'s accent, which would hide a wrong primary), and an agent profile using it; the global theme stays `e2e`. The fake agent answers with a message holding a code block and then asks a permission. In the capture, inside the zone's rectangle: the zone set's `card` and `muted` colours and the case's accent (the permission's allow button) are present, and the window set's `card`, `muted` and the global accent are absent; the strip is still `CHROME_BACKGROUND`. A second run splits the zone, opens the header menu (`open_zone_menu`, confirmed by the dump's `--- menu` line) with `--modality pointer`, and checks the window set's `popover` colour is absent from the zone's rectangle. The expected colours join `tests/e2e/brindle.rs`'s constants with their derivations.
- **Manual** (`CLAUDE.md`'s list, replacing "only the card takes the profile's background and foreground; the components keep the chrome's colours"): an agent zone with a profile theme unlike the chrome's, every component in the profile's colours; a hover over a copy button and a header button showing its tooltip in them; the header menu in them; the close confirmation in the chrome's; in brindle-light, brindle-dark and solarized-dark.

## Risks / Trade-offs

- [A component reads tokens outside the three phases (in an event handler or an entity's `update`) and gets the window's set] → The task audits every `ui_theme()` caller for that; today the only stored colour is `PromptInput::set_held_color`, which the app sets from the theme's ANSI yellow, not from tokens.
- [A future `deferred` or tooltip-like element inside a zone escapes the scope] → `ThemeScope`'s doc names both; the overlay and tooltip forms show how to carry the set.
- [Every agent zone's look changes, profile theme or not (F2)] → It's the fix for the darker bubbles too; the manual check covers the three themes the Markdown checks use.
- [`ui_theme()` by value touches 57 sites] → Mechanical, in one commit before anything else, with `cargo clippy` and the existing tests as the check.
- [e2e constants shared with `agents-markdown-layout-fixes`' case] → Whichever lands second updates the other's: that case's `muted` colour becomes the zone set's (its border colour is `card_border` in both sets, so it stays).

## Migration Plan

None: no config, state or wire change. A rollback is a revert.
