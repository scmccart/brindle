# Proposal

**Depends on:** `agents-profiles` (the agent zone, its card, `set_profile` and the rem scope) and `agents-markdown` (the `Response`, `CodeBlock` and link tooltips it draws), implemented and archived.
**Next:** nothing depends on it. It is independent of `agents-markdown-layout-fixes`; whichever lands second updates the other's e2e colour constants (tasks say where).

## Why

A profile can give an agent zone its own theme. Today only the zone's card takes it: the card's background and text and the zone header. Every component inside (bubbles, code blocks, the prompt input, buttons, permission rows, tool rows) reads the one window-wide token set, which comes from the global theme. So a dark profile theme in a light window shows light-chrome controls on a dark card. In the user's manual checks (2026-11-25) they didn't like this, and decided that a profile theme applies to the whole agent zone, while the tab strip and the window chrome keep the window's theme. That revises `agents-profiles`' decision 5.3 ("the `brindle-ui` components keep the chrome's tokens (GPUI has no scoped global)").

## What Changes

- **`brindle-ui` gains a theme scope:** an element that draws its subtree with another token set in layout, prepaint and paint. Components read the innermost scope's set, else the window's. Entering or leaving a scope notifies no one and redraws nothing outside it.
- **An agent zone is drawn in its theme's own token set**, the zone tokens, whether the theme comes from the profile or is the global one. That covers the card's surface, the bubbles, code blocks and tables, the prompt input and its queue row, buttons, tool rows, thoughts, permission and login rows, alerts, the scroll button, and the zone header's buttons. The zone's focus ring, its status dot, progress bar and context badge keep the window's colours, as they do on terminal zones.
- **Zone tokens are derived like the window's, over the zone's own background**, so the card and muted surfaces sit on the background the zone actually shows. The window's tokens are derived over the darker chrome background. Both are checked for legible pairs on every built-in theme.
- **Tooltips of controls inside the zone, and the zone header's menu, take the zone's tokens.** They keep the window's text size, as `agents-profiles` decided. Window-level dialogs opened from the zone (the close confirmation, the link confirmation) keep the window's tokens.
- **`Card` takes its ring colour from its owner**, so a scoped card can keep the window's ring.

Not in this change: theme scopes for terminal zones (their headers keep today's colours); a warning or success token; changing how a theme is chosen for a profile.

## Capabilities

### New Capabilities

None.

### Modified Capabilities

- `ui-components`: "Components draw from the active tokens" (the active set is the innermost scope's), "Card colours and ring" (the owner's ring colour), and new requirements for theme scopes and for tooltips and overlays inside one.
- `ui-theme-tokens`: "Chrome ignores per-tab themes" and "Window chrome uses the tokens" (agent zones are drawn in their own set), "Legible token pairs" (zone sets too), and a new requirement for zone token sets.
- `agent-zones`: "Agent zone appearance" (the whole zone in its theme's tokens, what keeps the window's, tooltips and the header menu).

## Impact

- **`brindle-ui`** (`ui/`): `theme.rs` (`UiTheme`'s scoped set, `ActiveTheme::ui_theme` returning the set by value, a way to read the window's set), a new `ThemeScope` element, `Tooltip` keeping the tokens in effect where its control was drawn, the overlay's optional token set, `CardColors`' ring; every `cx.ui_theme()` caller that dereferences it (57 sites, mechanical); tests, the gallery and `ui/README.md`.
- **The app:** `src/theme.rs` (`Theme::zone_tokens` beside `ui_tokens`, sharing their derivation, and its legibility test); `src/zone_view.rs` (agent zones wrapped in a scope with their tokens, the card's ring from the window's set, the zone menu request carrying the tokens); `src/workspace.rs` (the zone menu drawn in them); `src/agent/view.rs` (its root's background and text from the scoped set).
- **e2e:** a new case, `agent-zone-theme`, sampling a window capture.
- **Docs:** `docs/code-map.md`'s `agent/`, `theme.rs` and `brindle-ui` entries; `CLAUDE.md`'s manual check for profile themes in agent zones.
- No wire, config, state-file or dependency change.
